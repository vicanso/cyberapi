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
use cyberapi_core::response::Exchange;
use cyberapi_core::send::PreparedRequest;
use cyberapi_core::update::{self, Asset, Manifest, Release};
use cyberapi_http::HttpEngine;
use gpui_kit::{AppContext as _, Context, EventEmitter, Task};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Always the newest release's, served by GitHub.
const MANIFEST_URL: &str = "https://github.com/vicanso/cyberapi/releases/latest/download/latest.json";
/// The newest release as GitHub's API says: its notes, and its version when
/// it has no manifest.
const RELEASE_API: &str = "https://api.github.com/repos/vicanso/cyberapi/releases/latest";
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
        app.update(cx, |app, cx| app.note_update_check(now(), cx));
        self.phase = UpdatePhase::Checking { manual };
        cx.notify();
        let (engine, kinds) = (self.engine.clone(), self.kinds);
        self._task = Some(cx.spawn(async move |this, cx| {
            let found = cx.background_spawn(latest(engine, kinds)).await;
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
        self.phase = UpdatePhase::Downloading;
        cx.notify();
        let (engine, install) = (self.engine.clone(), self.install.clone());
        self._task = Some(cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move {
                    info!(name = %asset.name, size = asset.size, "update: downloading");
                    let exchange = fetch(&engine, &asset.url).await?;
                    if !update::verified(&exchange.body, &asset.sha256) {
                        return Err(format!("{} is not what the release published", asset.name));
                    }
                    install(&asset.name, &exchange.body)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
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

fn request(url: &str) -> PreparedRequest {
    PreparedRequest {
        url: url.to_string(),
        headers: vec![
            ("User-Agent".into(), crate::this_build::user_agent()),
            ("Accept".into(), "*/*".into()),
        ],
        timeouts: cyberapi_core::prefs::Timeouts::default().effective(),
        ..Default::default()
    }
}

/// `url`'s answer, following redirects: a 2xx, or why there is none.
async fn fetch(engine: &Arc<dyn HttpEngine>, url: &str) -> Result<Exchange, String> {
    let mut url = url.to_string();
    for _ in 0..=REDIRECTS {
        let exchange = engine.send(request(&url)).await.map_err(|error| error.to_string())?;
        if (300..400).contains(&exchange.status) {
            let location = exchange
                .headers
                .iter()
                .find(|(name, _)| name == "location")
                .map(|(_, value)| value.clone())
                .ok_or_else(|| format!("{url} redirects nowhere"))?;
            url = update::redirected(&url, &location).ok_or_else(|| format!("{url} redirects to {location}"))?;
            continue;
        }
        if !(200..300).contains(&exchange.status) {
            return Err(format!("{url} answered {}", exchange.status));
        }
        if exchange.truncated {
            return Err(format!("{url} is too large"));
        }
        return Ok(exchange);
    }
    Err(format!("{url}: more than {REDIRECTS} redirects"))
}

async fn json<T: serde::de::DeserializeOwned>(engine: &Arc<dyn HttpEngine>, url: &str) -> Result<T, String> {
    let exchange = fetch(engine, url).await?;
    serde_json::from_slice(&exchange.body).map_err(|error| format!("{url}: {error}"))
}

/// The newest release when it is newer than this build, from its manifest,
/// or from the API for a release without one.
async fn latest(engine: Arc<dyn HttpEngine>, kinds: &'static [&'static str]) -> Result<Option<UpdateInfo>, String> {
    let custom = std::env::var(MANIFEST_ENV).ok().filter(|url| !url.trim().is_empty());
    let manifest = json::<Manifest>(&engine, custom.as_deref().unwrap_or(MANIFEST_URL)).await;
    let manifest = match (manifest, &custom) {
        (Ok(manifest), _) => manifest,
        (Err(error), Some(_)) => return Err(error),
        // A release made before there was a manifest: the version and the
        // page only, nothing to download here.
        (Err(error), None) => {
            info!(%error, "update: no manifest, asking the API");
            let release = json::<Release>(&engine, RELEASE_API).await?;
            let version = release.tag_name.trim_start_matches('v').to_string();
            if !update::is_newer(&version, crate::VERSION) {
                return Ok(None);
            }
            return Ok(Some(UpdateInfo {
                version,
                page: release.html_url,
                notes: update::notes_body(&release.body).to_string(),
                asset: None,
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
        None => json::<Release>(&engine, RELEASE_API)
            .await
            .ok()
            .filter(|release| release.tag_name.trim_start_matches('v') == manifest.version)
            .map(|release| update::notes_body(&release.body).to_string())
            .unwrap_or_default(),
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
