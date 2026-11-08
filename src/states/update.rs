//! The desktop's update check, after zedis's: the release's `latest.json`
//! (`scripts/checksums.sh`), or GitHub's API for a release made before
//! there was one; the package for this machine, downloaded and checked
//! against its SHA-256, then installed — on macOS in place, over the
//! running app, which a restart then opens; elsewhere handed to the system
//! (`this_build::install_update`). A page never makes one of these: its
//! bridge is updated by whoever runs it.
//!
//! The check runs by itself at start, at most once in
//! [`UpdatePrefs::INTERVAL`], unless the settings turn it off, and says
//! nothing unless it finds a version the reader has not skipped. Asked for,
//! it always says what it found.

use super::AppStore;
use cyberapi_core::prefs::UpdatePrefs;
use cyberapi_core::proxy::ProxyPrefs;
use cyberapi_core::response::{Exchange, StreamEnd};
use cyberapi_core::send::PreparedRequest;
use cyberapi_core::update::{self, Asset, Manifest, Release};
use cyberapi_http::{HttpEngine, Progress, Watch, stop_signal};
use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::{AppContext as _, Context, EventEmitter, Task};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tracing::{info, warn};

/// Always the newest release's, served by GitHub.
const MANIFEST_URL: &str = "https://github.com/vicanso/cyberapi/releases/latest/download/latest.json";
/// The newest release as GitHub's API says: its notes, and its version and
/// assets when the manifest cannot be reached.
const RELEASE_API: &str = "https://api.github.com/repos/vicanso/cyberapi/releases/latest";
/// Recent releases, newest first, so an upgrade across several versions shows
/// what changed in each of them.
const RELEASE_LIST_API: &str = "https://api.github.com/repos/vicanso/cyberapi/releases?per_page=15";
/// Where a release with no package for this machine is had.
const RELEASES_PAGE: &str = "https://github.com/vicanso/cyberapi/releases/latest";
/// A `latest.json` elsewhere, for trying the update on this machine: with
/// it, GitHub is not asked at all.
const MANIFEST_ENV: &str = "CYBERAPI_UPDATE_MANIFEST";
/// Redirects followed to a file: GitHub sends a download on to its CDN.
const REDIRECTS: usize = 5;
/// The check at start waits this long, so it does not compete with the
/// window opening.
const START_DELAY: Duration = Duration::from_secs(5);
/// Per-step read timeout while streaming an installer download (seconds).
const DOWNLOAD_READ_SECS: u32 = 300;

/// A newer release than the running build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    /// Without a leading `v`.
    pub version: String,
    /// The release's page.
    pub page: String,
    /// Its notes, Markdown; empty when they could not be read.
    pub notes: String,
    /// The package for this machine. `None` when the release has none (or
    /// no manifest): the page is the way to it.
    pub asset: Option<Asset>,
}

/// How a verified package was installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// Copied over the running app (macOS): a restart opens the new one.
    Replaced,
    /// Given to the system to install, from here.
    HandedToOs(PathBuf),
}

/// Saves a verified package (its name, its bytes) and installs it.
/// Blocking: run off the main thread.
pub type Installer = Arc<dyn Fn(&str, &[u8]) -> Result<Delivery, String> + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdatePhase {
    Idle,
    Checking {
        manual: bool,
    },
    Downloading,
    /// Installed in place, waiting for the restart.
    Installed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateEvent {
    /// A newer release; `manual` when the reader asked.
    Found { manual: bool },
    /// Asked, and there is none newer.
    UpToDate,
    /// Asked, and the check failed: why.
    CheckFailed(String),
    /// The package is on this machine, given to the system to install.
    HandedToOs(PathBuf),
    /// Installed in place: restart to finish.
    Installed,
    /// The download, its check or the install failed: why.
    InstallFailed(String),
}

pub struct UpdateState {
    engine: Arc<dyn HttpEngine>,
    install: Installer,
    /// The package kinds this machine installs, the first it finds.
    kinds: &'static [&'static str],
    phase: UpdatePhase,
    available: Option<UpdateInfo>,
    /// `(downloaded, total)` bytes while [`UpdatePhase::Downloading`].
    progress: Option<(u64, u64)>,
    _start: Option<Task<()>>,
    _task: Option<Task<()>>,
}

impl EventEmitter<UpdateEvent> for UpdateState {}

impl UpdateState {
    pub fn new(engine: Arc<dyn HttpEngine>, install: Installer, kinds: &'static [&'static str]) -> Self {
        Self {
            engine,
            install,
            kinds,
            phase: UpdatePhase::Idle,
            available: None,
            progress: None,
            _start: None,
            _task: None,
        }
    }

    pub fn phase(&self) -> UpdatePhase {
        self.phase
    }

    /// The newer release found, while it is not skipped.
    pub fn available(&self) -> Option<&UpdateInfo> {
        self.available.as_ref()
    }

    /// `(downloaded, total)` bytes while a package is coming down.
    pub fn download_progress(&self) -> Option<(u64, u64)> {
        self.progress
    }

    /// `0..=100` while a package of known size is coming down.
    pub fn download_percent(&self) -> Option<u8> {
        let (done, total) = self.progress?;
        (done * 100).checked_div(total).map(|pct| pct.min(100) as u8)
    }

    /// Hides the title-bar chip for the rest of this session without
    /// permanently skipping the version.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.phase == UpdatePhase::Idle && self.available.take().is_some() {
            cx.notify();
        }
    }

    /// The check at start, a moment after it, when it is on and due.
    pub fn check_at_start(&mut self, cx: &mut Context<Self>) {
        let prefs = AppStore::global(cx).state().read(cx).prefs().updates.clone();
        if !prefs.auto_check || !update::check_due(prefs.last_check, now(), UpdatePrefs::INTERVAL) {
            return;
        }
        self._start = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(START_DELAY).await;
            let _ = this.update(cx, |this, cx| this.check(false, cx));
        }));
    }

    /// Looks for a newer release. Asked for (`manual`), it says what it
    /// found, a skipped version too.
    pub fn check(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.phase != UpdatePhase::Idle {
            return;
        }
        let app = AppStore::global(cx).state().clone();
        let proxy = app.read(cx).prefs().sending.proxy.clone();
        app.update(cx, |app, cx| app.note_update_check(now(), cx));
        self.phase = UpdatePhase::Checking { manual };
        cx.notify();
        let (engine, kinds) = (self.engine.clone(), self.kinds);
        self._task = Some(cx.spawn(async move |this, cx| {
            let found = cx.background_spawn(latest(engine, kinds, proxy)).await;
            let _ = this.update(cx, |this, cx| {
                this.phase = UpdatePhase::Idle;
                match found {
                    Ok(Some(info)) => {
                        let skipped = app.read(cx).prefs().updates.skipped == info.version;
                        if manual || !skipped {
                            info!(version = %info.version, "update: a newer release");
                            this.available = Some(info);
                            cx.emit(UpdateEvent::Found { manual });
                        }
                    }
                    Ok(None) => {
                        this.available = None;
                        if manual {
                            cx.emit(UpdateEvent::UpToDate);
                        }
                    }
                    Err(error) => {
                        warn!(%error, "update: the check failed");
                        if manual {
                            cx.emit(UpdateEvent::CheckFailed(error));
                        }
                    }
                }
                cx.notify();
            });
        }));
    }

    /// Not to be told of the version found again, unless asked to look.
    pub fn skip(&mut self, cx: &mut Context<Self>) {
        if let Some(info) = self.available.take() {
            let app = AppStore::global(cx).state().clone();
            app.update(cx, |app, cx| app.skip_update(info.version, cx));
            cx.notify();
        }
    }

    /// Downloads the package for this machine, checks it against its sum,
    /// and installs it.
    pub fn download_and_install(&mut self, cx: &mut Context<Self>) {
        if self.phase != UpdatePhase::Idle {
            return;
        }
        let Some(asset) = self.available.as_ref().and_then(|info| info.asset.clone()) else {
            return;
        };
        let proxy = AppStore::global(cx).state().read(cx).prefs().sending.proxy.clone();
        // Publish 0% synchronously before any await so the dialog and the
        // title-bar chip switch to progress state on the click itself.
        self.phase = UpdatePhase::Downloading;
        self.progress = Some((0, asset.size));
        cx.notify();

        let (tx, mut rx) = mpsc::unbounded::<(u64, u64)>();
        cx.spawn(async move |this, cx| {
            while let Some(progress) = rx.next().await {
                let _ = this.update(cx, |this, cx| {
                    if this.phase == UpdatePhase::Downloading {
                        this.progress = Some(progress);
                        cx.notify();
                    }
                });
            }
        })
        .detach();

        let (engine, install) = (self.engine.clone(), self.install.clone());
        self._task = Some(cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move {
                    info!(name = %asset.name, size = asset.size, "update: downloading");
                    let exchange = fetch_watched(&engine, &asset.url, &proxy, &asset.name, asset.size, tx).await?;
                    if !update::verified(&exchange.body, &asset.sha256) {
                        return Err(format!("{} is not what the release published", asset.name));
                    }
                    install(&asset.name, &exchange.body)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.progress = None;
                match done {
                    Ok(Delivery::Replaced) => {
                        this.phase = UpdatePhase::Installed;
                        cx.emit(UpdateEvent::Installed);
                    }
                    Ok(Delivery::HandedToOs(path)) => {
                        this.phase = UpdatePhase::Idle;
                        cx.emit(UpdateEvent::HandedToOs(path));
                    }
                    Err(error) => {
                        warn!(%error, "update: not installed");
                        this.phase = UpdatePhase::Idle;
                        cx.emit(UpdateEvent::InstallFailed(error));
                    }
                }
                cx.notify();
            });
        }));
    }

    /// After an install in place: quits, and the new app opens.
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        if self.phase == UpdatePhase::Installed {
            crate::this_build::relaunch();
            cx.quit();
        }
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn request(url: &str, proxy: &ProxyPrefs) -> PreparedRequest {
    PreparedRequest {
        url: url.to_string(),
        headers: vec![
            ("User-Agent".into(), crate::this_build::user_agent()),
            (
                "Accept".into(),
                "application/vnd.github+json, application/json, */*".into(),
            ),
        ],
        timeouts: cyberapi_core::prefs::Timeouts::default().effective(),
        proxy: proxy.clone(),
        ..Default::default()
    }
}

/// The address a failure may name. The query and the fragment stay out: a
/// download redirect signs them, and they are not the address. Headers are
/// not part of it either.
fn logged_address(url: &str) -> &str {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    &url[..end]
}

/// `url`'s answer, following redirects: a 2xx, or why there is none.
async fn fetch(engine: &Arc<dyn HttpEngine>, url: &str, proxy: &ProxyPrefs) -> Result<Exchange, String> {
    let mut url = url.to_string();
    for _ in 0..=REDIRECTS {
        let exchange = engine
            .send(request(&url, proxy))
            .await
            .map_err(|error| format!("{}: {error}", logged_address(&url)))?;
        if (300..400).contains(&exchange.status) {
            let location = exchange
                .headers
                .iter()
                .find(|(name, _)| name == "location")
                .map(|(_, value)| value.clone())
                .ok_or_else(|| format!("{} redirects nowhere", logged_address(&url)))?;
            url = update::redirected(&url, &location)
                .ok_or_else(|| format!("{} redirects to {}", logged_address(&url), logged_address(&location)))?;
            continue;
        }
        if !(200..300).contains(&exchange.status) {
            return Err(format!("{} answered {}", logged_address(&url), exchange.status));
        }
        if exchange.truncated {
            return Err(format!("{} is too large", logged_address(&url)));
        }
        return Ok(exchange);
    }
    Err(format!("{}: more than {REDIRECTS} redirects", logged_address(&url)))
}

/// Streams `url` following redirects and reports `(downloaded, total)` bytes
/// to `tx` on integer percentage ticks.
async fn fetch_watched(
    engine: &Arc<dyn HttpEngine>,
    url: &str,
    proxy: &ProxyPrefs,
    name: &str,
    expected_size: u64,
    tx: mpsc::UnboundedSender<(u64, u64)>,
) -> Result<Exchange, String> {
    let mut url = url.to_string();
    for _ in 0..=REDIRECTS {
        let mut req = request(&url, proxy);
        req.streams = true;
        req.timeouts.read = DOWNLOAD_READ_SECS;
        let state = Arc::new(Mutex::new((0u64, expected_size, u8::MAX, u8::MAX)));
        let step_state = state.clone();
        let step_tx = tx.clone();
        let log_name = name.to_string();
        let (_stop_tx, stop) = stop_signal();
        let watch = Watch {
            progress: Arc::new(move |event| {
                let Ok(mut guard) = step_state.lock() else {
                    return;
                };
                match event {
                    Progress::Head { status, headers } if (200..300).contains(&status) => {
                        guard.0 = 0;
                        if let Some(len) = headers
                            .iter()
                            .find(|(k, _)| k == "content-length")
                            .and_then(|(_, v)| v.trim().parse::<u64>().ok())
                            .filter(|&n| n > 0)
                        {
                            guard.1 = len;
                        }
                        let _ = step_tx.unbounded_send((0, guard.1));
                    }
                    Progress::Head { .. } => {}
                    Progress::Body(piece) => {
                        guard.0 += piece.len() as u64;
                        let (done, total) = (guard.0, guard.1);
                        if total == 0 {
                            let _ = step_tx.unbounded_send((done, 0));
                            return;
                        }
                        let pct = ((done * 100 / total).min(100)) as u8;
                        if pct == guard.2 {
                            return;
                        }
                        guard.2 = pct;
                        let decile = pct / 10;
                        if decile != guard.3 {
                            guard.3 = decile;
                            info!(asset = %log_name, pct, done, total, "update: download progress");
                        }
                        let _ = step_tx.unbounded_send((done, total));
                    }
                }
            }),
            stop,
        };
        let exchange = engine
            .send_watched(req, watch)
            .await
            .map_err(|error| format!("{}: {error}", logged_address(&url)))?;
        if (300..400).contains(&exchange.status) {
            let location = exchange
                .headers
                .iter()
                .find(|(name, _)| name == "location")
                .map(|(_, value)| value.clone())
                .ok_or_else(|| format!("{} redirects nowhere", logged_address(&url)))?;
            url = update::redirected(&url, &location)
                .ok_or_else(|| format!("{} redirects to {}", logged_address(&url), logged_address(&location)))?;
            continue;
        }
        if !(200..300).contains(&exchange.status) {
            return Err(format!("{} answered {}", logged_address(&url), exchange.status));
        }
        if exchange.truncated {
            return Err(format!("{} is too large", logged_address(&url)));
        }
        if let Some(stream) = &exchange.stream {
            match &stream.ended {
                StreamEnd::Closed => {}
                StreamEnd::Quiet { seconds } => {
                    return Err(format!("{}: no data within {seconds}s", logged_address(&url)));
                }
                StreamEnd::Failed { message } => {
                    return Err(format!("{}: {message}", logged_address(&url)));
                }
                StreamEnd::Cut | StreamEnd::Stopped => {
                    return Err(format!("{}: download interrupted", logged_address(&url)));
                }
            }
        }
        return Ok(exchange);
    }
    Err(format!("{}: more than {REDIRECTS} redirects", logged_address(&url)))
}

async fn json<T: serde::de::DeserializeOwned>(
    engine: &Arc<dyn HttpEngine>,
    url: &str,
    proxy: &ProxyPrefs,
) -> Result<T, String> {
    let exchange = fetch(engine, url, proxy).await?;
    serde_json::from_slice(&exchange.body).map_err(|error| format!("{}: {error}", logged_address(url)))
}

/// Best-effort changelog between the running build and `version`: asks the
/// release list first so an upgrade across several versions shows what each
/// of them changed, and falls back to `RELEASE_API` when the list is not
/// available.
async fn release_notes(engine: &Arc<dyn HttpEngine>, version: &str, proxy: &ProxyPrefs) -> String {
    if let Ok(releases) = json::<Vec<Release>>(engine, RELEASE_LIST_API, proxy).await {
        let combined = update::notes_between(&releases, crate::VERSION, version);
        if !combined.is_empty() {
            return combined;
        }
    }
    json::<Release>(engine, RELEASE_API, proxy)
        .await
        .ok()
        .filter(|release| release.tag_name.trim_start_matches('v') == version)
        .map(|release| update::notes_body(&release.body).to_string())
        .unwrap_or_default()
}

/// The newest release when it is newer than this build, from its manifest,
/// or from the API when the manifest cannot be reached.
async fn latest(
    engine: Arc<dyn HttpEngine>,
    kinds: &'static [&'static str],
    proxy: ProxyPrefs,
) -> Result<Option<UpdateInfo>, String> {
    let custom = std::env::var(MANIFEST_ENV).ok().filter(|url| !url.trim().is_empty());
    let manifest = json::<Manifest>(&engine, custom.as_deref().unwrap_or(MANIFEST_URL), &proxy).await;
    let manifest = match (manifest, &custom) {
        (Ok(manifest), _) => manifest,
        (Err(error), Some(_)) => return Err(error),
        // Manifest unreachable or absent: fall back to GitHub's release API,
        // which also carries the release's uploaded assets and their SHA-256
        // digests.
        (Err(error), None) => {
            info!(%error, "update: no manifest, asking the API");
            let release = json::<Release>(&engine, RELEASE_API, &proxy).await?;
            let version = release.tag_name.trim_start_matches('v').to_string();
            if !update::is_newer(&version, crate::VERSION) {
                return Ok(None);
            }
            let api_assets = update::assets_from_release(&release);
            let asset = update::pick_asset(&api_assets, std::env::consts::OS, std::env::consts::ARCH, kinds)
                .cloned()
                .filter(|a| !a.sha256.is_empty());
            let notes = json::<Vec<Release>>(&engine, RELEASE_LIST_API, &proxy)
                .await
                .ok()
                .map(|releases| update::notes_between(&releases, crate::VERSION, &version))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| update::notes_body(&release.body).to_string());
            let page = if release.html_url.trim().is_empty() {
                RELEASES_PAGE.to_string()
            } else {
                release.html_url
            };
            return Ok(Some(UpdateInfo {
                version,
                page,
                notes,
                asset,
            }));
        }
    };
    if !update::is_newer(&manifest.version, crate::VERSION) {
        return Ok(None);
    }
    let asset = update::pick_asset(&manifest.assets, std::env::consts::OS, std::env::consts::ARCH, kinds).cloned();
    // The notes are the API's, when it speaks of the same release; without
    // them the page still has them.
    let notes = match custom {
        Some(_) => String::new(),
        None => release_notes(&engine, &manifest.version, &proxy).await,
    };
    let page = match manifest.notes.trim() {
        "" => RELEASES_PAGE.to_string(),
        page => page.to_string(),
    };
    Ok(Some(UpdateInfo {
        version: manifest.version,
        page,
        notes,
        asset,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::states::AppStore;
    use cyberapi_http::{SendError, SendFuture};
    use cyberapi_store::MemoryStore;
    use gpui_kit::{Entity, TestAppContext};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Mutex;

    /// GitHub as the check sees it: the manifest behind a redirect, the
    /// API's notes, and the package.
    struct Releases {
        version: String,
        payload: Vec<u8>,
        /// The sum the manifest gives the package.
        sum: String,
        /// Whether the release has a manifest.
        manifest: bool,
        /// The newest release the API speaks of.
        api_version: String,
        asked: Mutex<Vec<String>>,
    }

    impl Releases {
        fn new(version: &str, manifest: bool) -> Self {
            let payload = b"the new app".to_vec();
            let sum = update::sha256(&payload);
            Self {
                version: version.into(),
                payload,
                sum,
                manifest,
                api_version: version.into(),
                asked: Mutex::default(),
            }
        }
    }

    impl HttpEngine for Releases {
        fn send(&self, request: PreparedRequest) -> SendFuture {
            if let Ok(mut asked) = self.asked.lock() {
                asked.push(request.url.clone());
            }
            let answer = |status: u16, headers: Vec<(String, String)>, body: Vec<u8>| {
                Ok(Exchange {
                    status,
                    headers,
                    body,
                    ..Default::default()
                })
            };
            let manifest = format!(
                r#"{{"version":"{v}","tag":"v{v}","notes":"https://releases.test/v{v}","assets":[{{"os":"{os}","arch":"{arch}","kind":"pkg","name":"cyberapi.pkg","url":"https://github.com/download/cyberapi.pkg","sha256":"{sum}","size":11}}]}}"#,
                v = self.version,
                os = std::env::consts::OS,
                arch = std::env::consts::ARCH,
                sum = self.sum,
            );
            let reply: Result<Exchange, SendError> = match request.url.as_str() {
                MANIFEST_URL if self.manifest => answer(
                    302,
                    vec![("location".into(), "https://objects.test/latest.json".into())],
                    vec![],
                ),
                MANIFEST_URL => answer(404, vec![], vec![]),
                "https://objects.test/latest.json" => answer(200, vec![], manifest.into_bytes()),
                RELEASE_API => answer(
                    200,
                    vec![],
                    format!(
                        r###"{{"tag_name":"v{v}","html_url":"https://releases.test/v{v}","body":"## [{v}](https://releases.test/compare) - 2026-10-24\r\n\r\n- what changed"}}"###,
                        v = self.api_version
                    )
                    .into_bytes(),
                ),
                "https://github.com/download/cyberapi.pkg" => {
                    answer(302, vec![("location".into(), "/cdn/cyberapi.pkg".into())], vec![])
                }
                "https://github.com/cdn/cyberapi.pkg" => answer(200, vec![], self.payload.clone()),
                _ => answer(404, vec![], vec![]),
            };
            Box::pin(async move { reply })
        }
    }

    fn launch(cx: &mut TestAppContext) {
        let store = MemoryStore::new();
        cx.update(|cx| {
            gpui_kit::init(cx);
            AppStore::init(
                store.storage(),
                Arc::new(Releases::new("0.0.1", true)),
                Default::default(),
                Default::default(),
                cx,
            );
        });
    }

    /// The packages the installer was handed: their names and bytes.
    type Handed = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

    /// A check of `releases`, whose installer answers `delivery`.
    fn state(releases: Arc<Releases>, delivery: Delivery, cx: &mut TestAppContext) -> (Entity<UpdateState>, Handed) {
        let installed = Arc::new(Mutex::new(Vec::new()));
        let seen = installed.clone();
        let install: Installer = Arc::new(move |name: &str, bytes: &[u8]| {
            if let Ok(mut seen) = seen.lock() {
                seen.push((name.to_string(), bytes.to_vec()));
            }
            Ok(delivery.clone())
        });
        let state = cx.new(|_| UpdateState::new(releases, install, &["pkg"]));
        (state, installed)
    }

    fn heard(state: &Entity<UpdateState>, cx: &mut TestAppContext) -> Rc<RefCell<Vec<UpdateEvent>>> {
        let heard = Rc::new(RefCell::new(Vec::new()));
        let into = heard.clone();
        cx.update(|cx| {
            cx.subscribe(state, move |_, event: &UpdateEvent, _| {
                into.borrow_mut().push(event.clone())
            })
            .detach();
        });
        heard
    }

    #[gpui_kit::test]
    async fn a_newer_release_is_found_downloaded_checked_and_installed(cx: &mut TestAppContext) {
        launch(cx);
        let releases = Arc::new(Releases::new("99.0.0", true));
        let (state, installed) = state(releases.clone(), Delivery::Replaced, cx);
        let heard = heard(&state, cx);

        state.update(cx, |state, cx| state.check(false, cx));
        cx.run_until_parked();
        let found = state
            .read_with(cx, |state, _| state.available().cloned())
            .expect("found");
        assert_eq!(found.version, "99.0.0");
        assert_eq!(found.page, "https://releases.test/v99.0.0");
        assert_eq!(found.notes, "- what changed", "the API's notes, without their heading");
        assert_eq!(
            found.asset.as_ref().map(|asset| asset.name.as_str()),
            Some("cyberapi.pkg")
        );
        assert!(
            cx.read(|cx| AppStore::global(cx).state().read(cx).prefs().updates.last_check) > 0,
            "the check is noted"
        );

        state.update(cx, |state, cx| state.download_and_install(cx));
        cx.run_until_parked();
        let installed = installed.lock().map(|seen| seen.clone()).unwrap_or_default();
        assert_eq!(installed, [("cyberapi.pkg".to_string(), b"the new app".to_vec())]);
        assert_eq!(state.read_with(cx, |state, _| state.phase()), UpdatePhase::Installed);
        assert_eq!(
            heard.borrow().as_slice(),
            [UpdateEvent::Found { manual: false }, UpdateEvent::Installed]
        );
    }

    #[gpui_kit::test]
    async fn a_package_that_is_not_what_was_published_is_not_installed(cx: &mut TestAppContext) {
        launch(cx);
        let mut releases = Releases::new("99.0.0", true);
        releases.sum = "00".repeat(32);
        let (state, installed) = state(Arc::new(releases), Delivery::Replaced, cx);
        let heard = heard(&state, cx);
        state.update(cx, |state, cx| state.check(true, cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.download_and_install(cx));
        cx.run_until_parked();
        assert!(installed.lock().map(|seen| seen.is_empty()).unwrap_or(false));
        assert!(matches!(
            heard.borrow().last(),
            Some(UpdateEvent::InstallFailed(why)) if why.contains("not what the release published")
        ));
        assert_eq!(state.read_with(cx, |state, _| state.phase()), UpdatePhase::Idle);
    }

    #[gpui_kit::test]
    async fn the_same_version_is_up_to_date_and_said_so_when_asked(cx: &mut TestAppContext) {
        launch(cx);
        let (state, _) = state(Arc::new(Releases::new(crate::VERSION, true)), Delivery::Replaced, cx);
        let heard = heard(&state, cx);
        state.update(cx, |state, cx| state.check(false, cx));
        cx.run_until_parked();
        assert!(heard.borrow().is_empty(), "the check at start says nothing");
        state.update(cx, |state, cx| state.check(true, cx));
        cx.run_until_parked();
        assert_eq!(heard.borrow().as_slice(), [UpdateEvent::UpToDate]);
        assert!(state.read_with(cx, |state, _| state.available().is_none()));
    }

    #[gpui_kit::test]
    async fn a_skipped_version_is_told_only_when_asked(cx: &mut TestAppContext) {
        launch(cx);
        let (state, _) = state(Arc::new(Releases::new("99.0.0", true)), Delivery::Replaced, cx);
        let heard = heard(&state, cx);
        state.update(cx, |state, cx| state.check(false, cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.skip(cx));
        assert_eq!(
            cx.read(|cx| AppStore::global(cx).state().read(cx).prefs().updates.skipped.clone()),
            "99.0.0"
        );
        state.update(cx, |state, cx| state.check(false, cx));
        cx.run_until_parked();
        assert!(state.read_with(cx, |state, _| state.available().is_none()));
        state.update(cx, |state, cx| state.check(true, cx));
        cx.run_until_parked();
        assert!(state.read_with(cx, |state, _| state.available().is_some()));
        assert_eq!(
            heard.borrow().as_slice(),
            [
                UpdateEvent::Found { manual: false },
                UpdateEvent::Found { manual: true }
            ]
        );
    }

    #[gpui_kit::test]
    async fn the_check_at_start_waits_and_keeps_to_the_settings(cx: &mut TestAppContext) {
        launch(cx);
        let releases = Arc::new(Releases::new("99.0.0", true));
        let found = |state: &Entity<UpdateState>, cx: &mut TestAppContext| {
            state.read_with(cx, |state, _| state.available().is_some())
        };
        let (first, _) = state(releases.clone(), Delivery::Replaced, cx);
        first.update(cx, |state, cx| state.check_at_start(cx));
        cx.run_until_parked();
        assert!(!found(&first, cx), "not while the window opens");
        cx.executor().advance_clock(START_DELAY);
        cx.run_until_parked();
        assert!(found(&first, cx));

        // Checked just now: not due again for a while.
        let (again, _) = state(releases.clone(), Delivery::Replaced, cx);
        again.update(cx, |state, cx| state.check_at_start(cx));
        cx.executor().advance_clock(START_DELAY);
        cx.run_until_parked();
        assert!(!found(&again, cx));

        // Due, and turned off.
        let app = cx.read(|cx| AppStore::global(cx).state().clone());
        app.update(cx, |app, cx| {
            app.note_update_check(0, cx);
            app.set_auto_update_check(false, cx);
        });
        let (off, _) = state(releases, Delivery::Replaced, cx);
        off.update(cx, |state, cx| state.check_at_start(cx));
        cx.executor().advance_clock(START_DELAY);
        cx.run_until_parked();
        assert!(!found(&off, cx));
    }

    /// The API's notes are another release's while it lags behind the
    /// manifest (or runs ahead): none are better than those.
    #[gpui_kit::test]
    async fn the_notes_are_only_the_releases_own(cx: &mut TestAppContext) {
        launch(cx);
        let mut releases = Releases::new("99.0.0", true);
        releases.api_version = "98.0.0".into();
        let (state, _) = state(Arc::new(releases), Delivery::Replaced, cx);
        state.update(cx, |state, cx| state.check(true, cx));
        cx.run_until_parked();
        let found = state
            .read_with(cx, |state, _| state.available().cloned())
            .expect("found");
        assert_eq!((found.version.as_str(), found.notes.as_str()), ("99.0.0", ""));
    }

    /// A failed hop is named by its address. A redirect's signed query, and
    /// the request's headers, stay out of the text the log records.
    #[gpui_kit::test]
    async fn a_failed_fetch_names_the_address_without_its_query(_cx: &mut TestAppContext) {
        struct FailAfterRedirect;
        impl HttpEngine for FailAfterRedirect {
            fn send(&self, request: PreparedRequest) -> SendFuture {
                let url = request.url;
                Box::pin(async move {
                    if url.contains('?') {
                        return Err(SendError::Tls {
                            message: "tls handshake eof".into(),
                        });
                    }
                    Ok(Exchange {
                        status: 302,
                        headers: vec![(
                            "location".into(),
                            "https://objects.example/latest.json?X-Amz-Signature=secret&x=1".into(),
                        )],
                        ..Default::default()
                    })
                })
            }
        }

        let engine: Arc<dyn HttpEngine> = Arc::new(FailAfterRedirect);
        let error = fetch(
            &engine,
            "https://github.com/vicanso/cyberapi/releases/latest/download/latest.json",
            &ProxyPrefs::default(),
        )
        .await
        .expect_err("the signed hop fails");
        assert_eq!(
            error,
            "https://objects.example/latest.json: TLS failed: tls handshake eof"
        );
        assert!(
            !error.contains("X-Amz-Signature") && !error.contains("secret"),
            "{error}"
        );
        assert!(!error.contains("User-Agent") && !error.contains("Accept"), "{error}");
    }

    #[test]
    fn a_logged_address_drops_the_query_and_the_fragment() {
        assert_eq!(
            logged_address("https://api.github.com/repos/vicanso/cyberapi/releases/latest?token=s#top"),
            "https://api.github.com/repos/vicanso/cyberapi/releases/latest"
        );
        assert_eq!(
            logged_address("https://github.com/vicanso/cyberapi/releases/latest/download/latest.json"),
            "https://github.com/vicanso/cyberapi/releases/latest/download/latest.json"
        );
    }

    /// A release made before there was a manifest is found by the API, with
    /// its page and nothing to download here.
    #[gpui_kit::test]
    async fn a_release_without_a_manifest_is_found_by_the_api(cx: &mut TestAppContext) {
        launch(cx);
        let releases = Arc::new(Releases::new("99.0.0", false));
        let (state, _) = state(releases.clone(), Delivery::Replaced, cx);
        state.update(cx, |state, cx| state.check(true, cx));
        cx.run_until_parked();
        let found = state
            .read_with(cx, |state, _| state.available().cloned())
            .expect("found");
        assert_eq!((found.version.as_str(), found.asset), ("99.0.0", None));
        assert_eq!(found.page, "https://releases.test/v99.0.0");
    }
}
