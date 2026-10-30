//! What this build can do, and how it introduces itself — the places the
//! desktop and the browser differ in capability. Each is a pair of functions
//! with one signature, the desktop's first.

use crate::states::Installer;
use cyberapi_core::ItemId;
use cyberapi_core::template::FileReader;
use std::sync::Arc;

/// The `User-Agent` requests carry unless they name their own (F5.4).
#[cfg(not(target_family = "wasm"))]
pub fn user_agent() -> String {
    format!(
        "CyberAPI/{} ({}; {}; {})",
        crate::VERSION,
        std::env::consts::OS,
        desktop::os_version(),
        std::env::consts::ARCH
    )
}
#[cfg(target_family = "wasm")]
pub fn user_agent() -> String {
    format!("CyberAPI/{} (web)", crate::VERSION)
}

/// What the app runs on, for the about section (F1.7): the OS and the
/// architecture, or a page's browser.
#[cfg(not(target_family = "wasm"))]
pub fn platform() -> String {
    format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH)
}
#[cfg(target_family = "wasm")]
pub fn platform() -> String {
    web_sys::window()
        .and_then(|window| window.navigator().user_agent().ok())
        .unwrap_or_default()
}

/// The OS's version (F1.7); a page cannot know it.
#[cfg(not(target_family = "wasm"))]
pub fn os_version() -> Option<String> {
    Some(desktop::os_version().to_string())
}
#[cfg(target_family = "wasm")]
pub fn os_version() -> Option<String> {
    None
}

/// Whether a request can read local files: multipart file fields and the
/// `readFile` / `openFile` template functions. A page cannot.
#[cfg(not(target_family = "wasm"))]
pub fn reads_files() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn reads_files() -> bool {
    false
}

/// E1: whether a WebSocket can be opened: the desktop's engine opens
/// them; a page's bridge relays none.
#[cfg(not(target_family = "wasm"))]
pub fn opens_sockets() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn opens_sockets() -> bool {
    false
}

/// E2: whether a request body can stream while the answer comes back. The
/// desktop's engine sends one; a page's bridge relays none.
#[cfg(not(target_family = "wasm"))]
pub fn streams_bodies() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn streams_bodies() -> bool {
    false
}

/// G2: a project's mock server on this machine's loopback address: the
/// desktop starts one on the runtime its requests go out on; a page
/// cannot listen.
#[cfg(not(target_family = "wasm"))]
pub fn start_mock(
    port: u16,
    routes: cyberapi_core::mock::Routes,
    heard: futures::channel::mpsc::UnboundedSender<cyberapi_core::mock::Heard>,
    cx: &gpui_kit::App,
) -> gpui_kit::Task<Result<Box<dyn crate::states::MockHandle>, String>> {
    let Some(runtime) = cx.try_global::<crate::Runtime>().map(|runtime| runtime.0.clone()) else {
        return gpui_kit::Task::ready(Err("no runtime to listen on".into()));
    };
    gpui_kit::AppContext::background_spawn(cx, async move {
        cyberapi_http::MockServer::start(runtime, port, routes, heard)
            .await
            .map(|server| Box::new(server) as Box<dyn crate::states::MockHandle>)
    })
}
#[cfg(target_family = "wasm")]
pub fn start_mock(
    _port: u16,
    _routes: cyberapi_core::mock::Routes,
    _heard: futures::channel::mpsc::UnboundedSender<cyberapi_core::mock::Heard>,
    _cx: &gpui_kit::App,
) -> gpui_kit::Task<Result<Box<dyn crate::states::MockHandle>, String>> {
    gpui_kit::Task::ready(Err("A mock server runs in the desktop app only".into()))
}

/// C4: one loopback callback for an authorization-code sign-in. Dropping
/// the listen stops it. The browser is the view's to open.
pub struct OauthListen {
    port: u16,
    done: futures::channel::oneshot::Receiver<Result<String, String>>,
    /// Dropping stops the listener.
    _stop: Box<dyn Send>,
}

impl OauthListen {
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The authorization code, or why the callback did not give one.
    pub async fn code(self) -> Result<String, String> {
        match self.done.await {
            Ok(result) => result,
            Err(_) => Err("OAuth 2.0: the authorization was cancelled".into()),
        }
    }
}

/// C4: listen on `127.0.0.1:port` (`0` for any free port) for a callback
/// whose `state` matches. The desktop's runtime; a page cannot listen.
#[cfg(not(target_family = "wasm"))]
pub fn listen_oauth(port: u16, state: String, cx: &gpui_kit::App) -> gpui_kit::Task<Result<OauthListen, String>> {
    let Some(runtime) = cx.try_global::<crate::Runtime>().map(|runtime| runtime.0.clone()) else {
        return gpui_kit::Task::ready(Err("no runtime to listen on".into()));
    };
    gpui_kit::AppContext::background_spawn(cx, async move {
        let listener = cyberapi_http::CallbackListener::start(runtime, port, state).await?;
        let (port, done, stop) = listener.into_wait();
        Ok(OauthListen {
            port,
            done,
            _stop: stop,
        })
    })
}
#[cfg(target_family = "wasm")]
pub fn listen_oauth(_port: u16, _state: String, _cx: &gpui_kit::App) -> gpui_kit::Task<Result<OauthListen, String>> {
    gpui_kit::Task::ready(Err("Authorization code runs in the desktop app only".into()))
}

/// C4: whether this build can open the system browser and catch the
/// callback. A page can keep the setup; it cannot authorize.
#[cfg(not(target_family = "wasm"))]
pub fn opens_oauth_browser() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn opens_oauth_browser() -> bool {
    false
}

/// F7: whether an open tab goes into a window of its own: a page has one.
#[cfg(not(target_family = "wasm"))]
pub fn opens_windows() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn opens_windows() -> bool {
    false
}

/// F7: `id`'s tab of `workspace` in a window of its own, or that window
/// brought forward; in a page, it stays where it is.
#[cfg(not(target_family = "wasm"))]
pub fn open_apart(workspace: gpui_kit::Entity<crate::states::WorkspaceState>, id: ItemId, cx: &mut gpui_kit::App) {
    crate::views::open_apart(workspace, id, cx);
}
#[cfg(target_family = "wasm")]
pub fn open_apart(workspace: gpui_kit::Entity<crate::states::WorkspaceState>, id: ItemId, cx: &mut gpui_kit::App) {
    workspace.update(cx, |workspace, cx| workspace.join(&id, cx));
}

/// F7: `id`'s own window closed, its tab back in the main window.
#[cfg(not(target_family = "wasm"))]
pub fn close_apart(id: &ItemId, cx: &mut gpui_kit::App) {
    crate::views::close_apart(id, cx);
}
#[cfg(target_family = "wasm")]
pub fn close_apart(_: &ItemId, _: &mut gpui_kit::App) {}

/// A7: whether this build load tests an API: a page's sends go through
/// the bridge, which lets an account have only so many at once.
#[cfg(not(target_family = "wasm"))]
pub fn load_tests() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn load_tests() -> bool {
    false
}

/// G2: whether this build serves mocks.
#[cfg(not(target_family = "wasm"))]
pub fn serves_mocks() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn serves_mocks() -> bool {
    false
}

#[cfg(not(target_family = "wasm"))]
impl crate::states::MockHandle for cyberapi_http::MockServer {
    fn address(&self) -> String {
        format!("http://{}", cyberapi_http::MockServer::address(self))
    }

    fn set_routes(&self, routes: cyberapi_core::mock::Routes) {
        cyberapi_http::MockServer::set_routes(self, routes);
    }
}

/// Whether requests may go through a proxy (#60): the desktop's do; a
/// page's go from the bridge, straight to their servers.
#[cfg(not(target_family = "wasm"))]
pub fn proxies() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn proxies() -> bool {
    false
}

/// Whether the app sizes its own window (F14.2); a page's is the tab's.
#[cfg(not(target_family = "wasm"))]
pub fn sizes_window() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn sizes_window() -> bool {
    false
}

/// Where the data is kept, for the settings' Data section (F1.7).
#[cfg(not(target_family = "wasm"))]
pub fn data_location() -> String {
    crate::paths::data_dir()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default()
}
#[cfg(target_family = "wasm")]
pub fn data_location() -> String {
    rust_i18n::t!("settings.on_the_server").to_string()
}

/// Whether the data folder can be shown (the settings' Data section).
#[cfg(not(target_family = "wasm"))]
pub fn shows_data() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn shows_data() -> bool {
    false
}

/// Opens the data folder in the system's file manager.
#[cfg(not(target_family = "wasm"))]
pub fn show_data(cx: &mut gpui_kit::App) {
    if let Some(dir) = crate::paths::data_dir() {
        cx.open_with_system(&dir);
    }
}
#[cfg(target_family = "wasm")]
pub fn show_data(_: &mut gpui_kit::App) {}

/// How many days of log files are kept; a page has no log files.
#[cfg(not(target_family = "wasm"))]
pub fn log_days() -> Option<usize> {
    Some(crate::paths::LOG_DAYS)
}
#[cfg(target_family = "wasm")]
pub fn log_days() -> Option<usize> {
    None
}

/// F6: whether this build backs its data up by itself; a page's data is
/// the server's.
#[cfg(not(target_family = "wasm"))]
pub fn backs_up_by_itself() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn backs_up_by_itself() -> bool {
    false
}

/// F6: whether another of the desktop's own backups is due `now` (the wall
/// clock), by those in its folder.
#[cfg(not(target_family = "wasm"))]
pub fn backup_due(now: chrono::NaiveDateTime) -> bool {
    let Some(dir) = crate::paths::backups_dir() else {
        return false;
    };
    cyberapi_core::backup::due(&file_names(&dir), now)
}
#[cfg(target_family = "wasm")]
pub fn backup_due(_: chrono::NaiveDateTime) -> bool {
    false
}

/// F6: `bytes` kept as the backup `name` in the backups' folder — written
/// apart first, so a half-written one never takes the name — and the
/// oldest removed past `cyberapi_core::backup::KEPT`; answers where.
#[cfg(not(target_family = "wasm"))]
pub fn keep_backup(name: &str, bytes: &[u8]) -> Result<std::path::PathBuf, String> {
    let dir = crate::paths::backups_dir().ok_or("no data directory")?;
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(name);
    let writing = dir.join(format!(".{name}.part"));
    std::fs::write(&writing, bytes)
        .and_then(|()| std::fs::rename(&writing, &path))
        .map_err(|error| {
            let _ = std::fs::remove_file(&writing);
            error.to_string()
        })?;
    let now = chrono::Local::now().naive_local();
    for old in cyberapi_core::backup::pruned(&file_names(&dir), now) {
        if let Err(error) = std::fs::remove_file(dir.join(&old)) {
            tracing::warn!(%error, old, "an old backup could not be removed");
        }
    }
    Ok(path)
}
#[cfg(target_family = "wasm")]
pub fn keep_backup(_: &str, _: &[u8]) -> Result<std::path::PathBuf, String> {
    Err("a page keeps no files".into())
}

/// The names of the files in `dir`; none when it cannot be read.
#[cfg(not(target_family = "wasm"))]
fn file_names(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// F6: opens the backups' folder, made first if none was written yet.
#[cfg(not(target_family = "wasm"))]
pub fn show_backups(cx: &mut gpui_kit::App) {
    if let Some(dir) = crate::paths::backups_dir() {
        let _ = std::fs::create_dir_all(&dir);
        cx.open_with_system(&dir);
    }
}
#[cfg(target_family = "wasm")]
pub fn show_backups(_: &mut gpui_kit::App) {}

/// Opens the logs' folder (`paths::logs_dir`), made first if no log was
/// written yet. A page logs to the browser's console, and has none.
#[cfg(not(target_family = "wasm"))]
pub fn show_logs(cx: &mut gpui_kit::App) {
    if let Some(dir) = crate::paths::logs_dir() {
        let _ = std::fs::create_dir_all(&dir);
        cx.open_with_system(&dir);
    }
}
#[cfg(target_family = "wasm")]
pub fn show_logs(_: &mut gpui_kit::App) {}

/// Whether an export can be saved as a file (M11); a page copies it to the
/// clipboard instead.
#[cfg(not(target_family = "wasm"))]
pub fn saves_files() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn saves_files() -> bool {
    false
}

/// Saves `bytes` in the downloads folder as `name`, or `name-1` … `name-9`
/// when it is taken (F15.4); answers where. A page has the browser download
/// them, under `name` as far as the browser keeps it.
#[cfg(not(target_family = "wasm"))]
pub fn save_to_downloads(name: &str, bytes: &[u8]) -> Result<String, String> {
    desktop::save_to_downloads(name, bytes)
}
#[cfg(target_family = "wasm")]
pub fn save_to_downloads(name: &str, bytes: &[u8]) -> Result<String, String> {
    web::download(name, bytes)
}

/// Whether Cmd/Ctrl-N and Shift-Cmd/Ctrl-N reach the app (F3.12), so they
/// may be shown: a page on Windows or Linux has the browser take Ctrl-N
/// for a window first. A Mac's browser leaves Ctrl-N to the page.
#[cfg(not(target_family = "wasm"))]
pub fn takes_new_keys() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn takes_new_keys() -> bool {
    web::on_mac()
}

/// Whether Cmd/Ctrl and the keys a browser keeps for itself reach the app,
/// so they may be shown: a page on Windows or Linux has the browser take
/// Ctrl-1 to Ctrl-9 for its tabs (a pin's, F12.2), Ctrl-L for its address
/// bar and Ctrl-D for a bookmark (#64) first. A Mac's browser does those
/// with Cmd and leaves Ctrl to the page.
#[cfg(not(target_family = "wasm"))]
pub fn takes_browser_keys() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn takes_browser_keys() -> bool {
    web::on_mac()
}

/// Whether this is a page in a Mac's browser, which binds the Cmd
/// shortcuts the kit gives only its macOS build
/// (`views::bind_mac_page_keys`). The desktop has them already.
#[cfg(not(target_family = "wasm"))]
pub fn mac_page() -> bool {
    false
}
#[cfg(target_family = "wasm")]
pub fn mac_page() -> bool {
    web::on_mac()
}

/// The editors' monospace face, JetBrains Mono (as zedis has it): the
/// desktop carries it; a page is handed it after its first frame
/// (`cyberapi-web`), and text drawn before then uses the fallback.
#[cfg(not(target_family = "wasm"))]
pub fn add_fonts(cx: &mut gpui_kit::App) {
    let faces = [
        include_bytes!("../resources/fonts/JetBrainsMono-Regular.ttf").as_slice(),
        include_bytes!("../resources/fonts/JetBrainsMono-Bold.ttf").as_slice(),
    ];
    if let Err(error) = cx.text_system().add_fonts(faces.map(Into::into).to_vec()) {
        tracing::error!(%error, "the monospace font could not be registered");
    }
}
#[cfg(target_family = "wasm")]
pub fn add_fonts(_: &mut gpui_kit::App) {}

/// F7.4: a body the app cannot draw (a PDF) shown by what can: the
/// system's viewer for it, or a new tab of the browser.
#[cfg(not(target_family = "wasm"))]
pub fn view_outside(name: &str, _content_type: &str, bytes: &[u8], cx: &gpui_kit::App) -> Result<(), String> {
    desktop::view_outside(name, bytes, cx)
}
#[cfg(target_family = "wasm")]
pub fn view_outside(_name: &str, content_type: &str, bytes: &[u8], cx: &gpui_kit::App) -> Result<(), String> {
    cx.open_url(&web::blob_url(content_type, bytes)?);
    Ok(())
}

/// Whether an HTML body may be opened as a page (F7.4): the desktop hands a
/// file to the system's browser. A page does not: a tab of its own `blob:`
/// would run the answer's scripts as the bridge's page, signed in.
#[cfg(not(target_family = "wasm"))]
pub fn opens_html() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn opens_html() -> bool {
    false
}

/// The link that opens an invitation (`docs/server-sync.md` §5): the
/// bridge's page, with the token after `#`, which no server sees or logs.
/// The desktop names its bridge (`server`); a page is on it.
#[cfg(not(target_family = "wasm"))]
pub fn invite_link(server: Option<&str>, token: &str) -> String {
    format!("{}/#invite={token}", server.unwrap_or_default())
}
#[cfg(target_family = "wasm")]
pub fn invite_link(_server: Option<&str>, token: &str) -> String {
    let page = web_sys::window()
        .and_then(|window| window.location().href().ok())
        .unwrap_or_default();
    let page = page.split('#').next().unwrap_or_default();
    format!("{page}#invite={token}")
}

/// This machine's half of the update check (`states/update.rs`): which
/// packages of a release it takes, and what installs one. A page has none:
/// its bridge is updated by whoever runs it.
#[cfg(not(target_family = "wasm"))]
pub fn updates() -> Option<(&'static [&'static str], Installer)> {
    Some((crate::update_install::kinds(), Arc::new(crate::update_install::install)))
}
#[cfg(target_family = "wasm")]
pub fn updates() -> Option<(&'static [&'static str], Installer)> {
    None
}

/// Opens the app again once it has quit, after an update installed in
/// place.
#[cfg(not(target_family = "wasm"))]
pub fn relaunch() {
    crate::update_install::relaunch();
}
#[cfg(target_family = "wasm")]
pub fn relaunch() {}

/// Where an MCP client reaches the bridge (`/v1/mcp`): the desktop's
/// server, or the page's own address, below its base path.
#[cfg(not(target_family = "wasm"))]
pub fn mcp_endpoint(server: Option<&str>) -> String {
    format!("{}/v1/mcp", server.unwrap_or_default().trim_end_matches('/'))
}
#[cfg(target_family = "wasm")]
pub fn mcp_endpoint(_server: Option<&str>) -> String {
    let page = web_sys::window()
        .and_then(|window| window.location().href().ok())
        .unwrap_or_default();
    let page = page.split(['#', '?']).next().unwrap_or_default();
    // The page is the mount's directory: `/` or `/prefix/`.
    let directory = page.rfind('/').map_or(page, |at| &page[..=at]);
    format!("{directory}v1/mcp")
}

/// A text file the reader picked.
#[cfg(not(target_family = "wasm"))]
pub fn read_text(path: &std::path::Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}
#[cfg(target_family = "wasm")]
pub fn read_text(_: &std::path::Path) -> Result<String, String> {
    Err("a page cannot read files".into())
}

/// A file the reader picked, as it is.
#[cfg(not(target_family = "wasm"))]
pub fn read_bytes(path: &std::path::Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))
}
#[cfg(target_family = "wasm")]
pub fn read_bytes(_: &std::path::Path) -> Result<Vec<u8>, String> {
    Err("a page cannot read files".into())
}

/// What reads those files.
#[cfg(not(target_family = "wasm"))]
pub fn files() -> Arc<dyn FileReader + Send + Sync> {
    Arc::new(desktop::Files)
}
#[cfg(target_family = "wasm")]
pub fn files() -> Arc<dyn FileReader + Send + Sync> {
    Arc::new(cyberapi_core::template::NoFiles(
        "the browser build cannot read local files",
    ))
}

/// What reads this machine's environment variables for `env` (D2): the
/// app's own and the login shell's (`shell_env`). A page has none.
#[cfg(not(target_family = "wasm"))]
pub fn env() -> Arc<dyn cyberapi_core::template::EnvReader + Send + Sync> {
    Arc::new(crate::shell_env::ShellEnv)
}
#[cfg(target_family = "wasm")]
pub fn env() -> Arc<dyn cyberapi_core::template::EnvReader + Send + Sync> {
    Arc::new(cyberapi_core::template::NoEnv(
        "the browser build cannot read environment variables",
    ))
}

/// `.proto` files compiled for a gRPC call's definitions: what they import
/// is looked for beside each, then in the folders above it (an import
/// names a file from the root of its tree); Google's own are built in. A
/// page reads no files.
#[cfg(not(target_family = "wasm"))]
pub fn compile_protos(paths: &[std::path::PathBuf]) -> Result<cyberapi_core::grpc::Definitions, String> {
    desktop::compile_protos(paths)
}
#[cfg(target_family = "wasm")]
pub fn compile_protos(_: &[std::path::PathBuf]) -> Result<cyberapi_core::grpc::Definitions, String> {
    Err("the browser build cannot read .proto files".into())
}

#[cfg(not(target_family = "wasm"))]
mod desktop {
    use cyberapi_core::template::{BaseDir, FileReader};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    /// How many folders above a `.proto` file its imports are looked for in.
    const IMPORT_LEVELS: usize = 5;

    pub(super) fn compile_protos(paths: &[PathBuf]) -> Result<cyberapi_core::grpc::Definitions, String> {
        let mut includes: Vec<&Path> = Vec::new();
        for path in paths {
            for folder in path.ancestors().skip(1).take(IMPORT_LEVELS) {
                if !folder.as_os_str().is_empty() && !includes.contains(&folder) {
                    includes.push(folder);
                }
            }
        }
        let mut compiler = protox::Compiler::new(includes).map_err(|error| error.to_string())?;
        compiler.include_imports(true);
        compiler.open_files(paths).map_err(|error| error.to_string())?;
        cyberapi_core::grpc::Definitions::decode(&compiler.encode_file_descriptor_set())
    }

    /// Read once: finding it out may start a process, and every request's
    /// User-Agent names it.
    pub(super) fn os_version() -> &'static str {
        static VERSION: OnceLock<String> = OnceLock::new();
        VERSION.get_or_init(|| os_info::get().version().to_string())
    }

    /// Files from this machine: a name relative to the downloads, documents
    /// or desktop folder (M6), or an absolute path.
    pub struct Files;

    impl FileReader for Files {
        fn read(&self, file: &str, dir: Option<BaseDir>) -> Result<Vec<u8>, String> {
            let path = resolve(file, dir);
            std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))
        }
    }

    /// The downloads folder, or `~/Downloads` when the system names none
    /// (`XDG_DOWNLOAD_DIR` unset, #49). The directory is created. Home is
    /// the last place a file can land.
    fn downloads_folder() -> Result<PathBuf, String> {
        let dirs = directories::UserDirs::new().ok_or("this system names no home directory")?;
        let home = dirs.home_dir().to_path_buf();
        let preferred = dirs
            .download_dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join("Downloads"));
        if std::fs::create_dir_all(&preferred).is_ok() {
            return Ok(preferred);
        }
        std::fs::create_dir_all(&home).map_err(|error| format!("{}: {error}", home.display()))?;
        Ok(home)
    }

    pub(super) fn save_to_downloads(name: &str, bytes: &[u8]) -> Result<String, String> {
        let dir = downloads_folder()?;
        save_unique(&dir, name, bytes)
    }

    /// A fresh folder of the temporary one for each body, so the viewer
    /// shows the body's own name.
    pub(super) fn view_outside(name: &str, bytes: &[u8], cx: &gpui_kit::App) -> Result<(), String> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join("cyberapi-view").join(stamp.to_string());
        std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        let path = dir.join(file_name(name));
        std::fs::write(&path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
        cx.open_with_system(&path);
        Ok(())
    }

    /// `name` with what no file name may hold made `_`.
    fn file_name(name: &str) -> String {
        name.chars()
            .map(|ch| {
                if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                    '_'
                } else {
                    ch
                }
            })
            .collect()
    }

    /// F15.4: `name`, then `stem-1.ext` … `stem-9.ext`; never over a file.
    pub(super) fn save_unique(dir: &Path, name: &str, bytes: &[u8]) -> Result<String, String> {
        let name = file_name(name);
        let (stem, extension) = match name.rsplit_once('.') {
            Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
            None => (name.clone(), String::new()),
        };
        for attempt in 0..10 {
            let candidate = if attempt == 0 {
                dir.join(&name)
            } else {
                dir.join(format!("{stem}-{attempt}{extension}"))
            };
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(mut file) => {
                    use std::io::Write as _;
                    file.write_all(bytes).map_err(|error| error.to_string())?;
                    return Ok(candidate.display().to_string());
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("{}: {error}", candidate.display())),
            }
        }
        Err(format!("{name} and nine more like it exist already"))
    }

    pub(super) fn resolve(file: &str, dir: Option<BaseDir>) -> PathBuf {
        let path = Path::new(file);
        let Some(dir) = dir.filter(|_| path.is_relative()) else {
            return path.to_path_buf();
        };
        let base = directories::UserDirs::new().and_then(|dirs| {
            match dir {
                BaseDir::Download => dirs.download_dir(),
                BaseDir::Document => dirs.document_dir(),
                BaseDir::Desktop => dirs.desktop_dir(),
            }
            .map(Path::to_path_buf)
        });
        match base {
            Some(base) => base.join(path),
            None => path.to_path_buf(),
        }
    }
}

#[cfg(target_family = "wasm")]
mod web {
    use wasm_bindgen::JsCast as _;

    fn reason(error: wasm_bindgen::JsValue) -> String {
        error.as_string().unwrap_or_else(|| format!("{error:?}"))
    }

    pub(super) fn on_mac() -> bool {
        let Some(navigator) = web_sys::window().map(|window| window.navigator()) else {
            return false;
        };
        navigator.platform().is_ok_and(|platform| platform.starts_with("Mac"))
            || navigator.user_agent().is_ok_and(|agent| agent.contains("Macintosh"))
    }

    /// The bytes as a URL of this page, typed so the browser can show them.
    /// It lives as long as the page: the tab it opens reads it later.
    pub(super) fn blob_url(content_type: &str, bytes: &[u8]) -> Result<String, String> {
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
        let options = web_sys::BlobPropertyBag::new();
        options.set_type(content_type);
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(reason)?;
        web_sys::Url::create_object_url_with_blob(&blob).map_err(reason)
    }

    /// A link to the bytes, clicked: the browser saves them as `name`.
    pub(super) fn download(name: &str, bytes: &[u8]) -> Result<String, String> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or("this page has no document")?;
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
        let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).map_err(reason)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(reason)?;
        let anchor: web_sys::HtmlAnchorElement = document
            .create_element("a")
            .map_err(reason)?
            .dyn_into()
            .map_err(|_| "not a link".to_string())?;
        anchor.set_href(&url);
        anchor.set_download(name);
        anchor.click();
        // The download has the blob already; the URL is no longer needed.
        let _ = web_sys::Url::revoke_object_url(&url);
        Ok(name.to_string())
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;
    use cyberapi_core::template::BaseDir;

    #[test]
    fn the_desktop_reads_files() {
        assert!(reads_files());
        assert!(user_agent().starts_with(&format!("CyberAPI/{} (", crate::VERSION)));
        let dir = tempfile::tempdir().expect("a directory");
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"hello").expect("write");
        let path = file.to_str().expect("UTF-8");
        assert_eq!(files().read(path, Some(BaseDir::Desktop)), Ok(b"hello".to_vec()));
        assert!(files().read("/no/such/file", None).is_err());
        // Saving never overwrites (F15.4).
        let first = desktop::save_unique(dir.path(), "a/b.json", b"1").expect("saved");
        let second = desktop::save_unique(dir.path(), "a/b.json", b"2").expect("saved");
        assert!(
            first.ends_with("a_b.json") && second.ends_with("a_b-1.json"),
            "{first} {second}"
        );
        // A relative name is in the folder it names, where the machine has
        // one: a Linux without XDG user directories (CI's) has none.
        let relative = desktop::resolve("x.json", Some(BaseDir::Download));
        match directories::UserDirs::new().and_then(|dirs| dirs.download_dir().map(|dir| dir.join("x.json"))) {
            Some(named) => assert!(named.is_absolute() && relative == named, "{}", relative.display()),
            None => assert_eq!(relative, std::path::Path::new("x.json")),
        }
    }
}
