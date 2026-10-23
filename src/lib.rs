//! CyberAPI, as a library.
//!
//! The desktop binary next to this file only calls [`run`]. The split exists
//! so the views can be compiled into the browser build as well: `cyberapi-web`
//! sets up the page's platform and its stores itself and enters through
//! [`launch`], which is everything both builds share.

// The app's strings. The current locale is rust-i18n's, shared with
// gpui-component, and set by `i18n::apply`.
rust_i18n::i18n!("locales", fallback = "en");

#[cfg(not(target_family = "wasm"))]
mod assets;
mod drafts_local;
mod i18n;
mod json_view_local;
mod local_items;
mod pacing;
#[cfg(not(target_family = "wasm"))]
mod paths;
mod pins_local;
mod root;
mod server_local;
pub mod states;
mod this_build;
mod views;

pub use crate::pacing::set_page_visible;
use crate::root::CyberApi;
use crate::states::AppStore;
use crate::views::StartupError;
use cyberapi_core::prefs::{AppPrefs, MonitorPlacement, WindowPrefs, opening_monitor};
use cyberapi_http::HttpEngine;
use cyberapi_store::{BackupStore, DirectoryStore, ServerCopyStore, Storage};
use gpui_kit::component::TitleBar;
use gpui_kit::{
    App, AppContext as _, Bounds, DisplayId, Entity, Pixels, Render, Size, Window, WindowBounds, WindowOptions, point,
    px, size,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::error;

pub const APP_NAME: &str = "Cyber API";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The identifier the original Tauri build used; Linux groups windows by it.
const APP_ID: &str = "com.bigtree.cyberapi";

gpui_kit::actions!(
    cyberapi,
    [
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
        ToggleFullScreen,
        CloseWindow
    ]
);

/// What only one of the builds has; the other leaves it `None`.
#[derive(Clone, Default)]
pub struct Extras {
    /// The desktop's backups (M13).
    pub backups: Option<Arc<dyn BackupStore>>,
    /// The desktop's copies of server projects (`docs/server-sync.md` §6).
    pub copies: Option<Arc<dyn ServerCopyStore>>,
    /// The browser's accounts and teams (`docs/architecture.md` §9.4).
    pub directory: Option<Arc<dyn DirectoryStore>>,
    /// Ends the browser's login; the page then asks for one again.
    pub sign_out: Option<SignOut>,
}

/// Ending a login: the work, as a future to run.
pub type SignOut = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Everything the desktop `main` does.
#[cfg(not(target_family = "wasm"))]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_logger();
    tracing::info!(version = VERSION, "cyberapi launch");
    // Before the platform starts: the first window needs the settings.
    let opened = open_desktop_storage();
    // Requests go out on a runtime of their own, alive as long as the app.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("cyberapi-http")
        .enable_all()
        .build()?;
    let engine: Arc<dyn HttpEngine> = Arc::new(cyberapi_http::NativeEngine::new(
        runtime.handle().clone(),
        cyberapi_http::TargetPolicy::Anywhere,
    ));
    gpui_kit::application().with_assets(assets::Assets).run(move |cx| {
        // Must come before anything that uses a component.
        gpui_kit::init(cx);
        match opened {
            Ok((storage, store, prefs)) => {
                let extras = Extras {
                    backups: Some(store.clone()),
                    copies: Some(store),
                    ..Default::default()
                };
                launch(cx, storage, engine, prefs, extras)
            }
            Err(message) => launch_failed(cx, message),
        }
    });
    drop(runtime);
    Ok(())
}

/// `RUST_LOG` picks the level; `info` without it.
#[cfg(not(target_family = "wasm"))]
fn init_logger() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// The data directory's database, with the Tauri app's data copied in on the
/// first launch (D2): as the stores, as itself — for backups (M13) and the
/// copies of server projects — and the reader's settings.
#[cfg(not(target_family = "wasm"))]
fn open_desktop_storage() -> Result<(Storage, Arc<cyberapi_store_turso::TursoStore>, AppPrefs), String> {
    use cyberapi_store_turso::{TursoStore, copy_legacy_data};
    use tracing::{info, warn};

    let dir = paths::data_dir().ok_or("this system names no data directory")?;
    let database = dir.join(paths::DATABASE_FILE);
    if let Some(legacy) = paths::legacy_dir() {
        match copy_legacy_data(&legacy, &database) {
            Ok(copied) if copied.database => info!(from = %legacy.display(), "copied the Tauri app's data"),
            Ok(_) => {}
            // Starting empty beats not starting: the old data stays where it
            // is, and the next launch tries again.
            Err(e) => warn!(error = %e, from = %legacy.display(), "the Tauri app's data could not be copied"),
        }
    }
    info!(database = %database.display(), "opening the store");
    let store = pollster::block_on(TursoStore::open(&database))
        .map_err(|e| format!("{}: {}", database.display(), i18n::store_error(&e)))?;
    let storage = store.storage();
    import_legacy_cookies(&storage, &dir);
    let prefs = pollster::block_on(storage.prefs().app()).unwrap_or_else(|e| {
        warn!(error = %e, "the settings could not be read; using the defaults");
        AppPrefs::default()
    });
    Ok((storage, Arc::new(store), prefs))
}

/// D4: the Tauri app's cookie jar, copied beside the database as
/// `legacy-cookies.json`, goes into the store once; the file is renamed so
/// the next launch leaves it be. A jar that cannot be read stays for a later
/// launch to try again.
#[cfg(not(target_family = "wasm"))]
fn import_legacy_cookies(storage: &Storage, dir: &std::path::Path) {
    use tracing::{info, warn};

    let file = dir.join("legacy-cookies.json");
    let Ok(text) = std::fs::read_to_string(&file) else {
        return;
    };
    let cookies = cyberapi_core::cookie::parse_legacy_jar(&text);
    let count = cookies.len();
    match pollster::block_on(storage.cookies().put(cookies)) {
        Ok(()) => {
            info!(count, "imported the Tauri app's cookies");
            if let Err(e) = std::fs::rename(&file, dir.join("legacy-cookies.imported.json")) {
                warn!(error = %e, "the imported cookie jar could not be renamed");
            }
        }
        Err(e) => warn!(error = %e, "the Tauri app's cookies could not be imported"),
    }
}

/// Opens the main window on `storage`, sending with `engine`, with what
/// only this build has in `extras`. Both builds call this once the platform
/// is up and `gpui_kit::init` has run: the desktop from [`run`], the browser
/// from `cyberapi-web`.
pub fn launch(cx: &mut App, storage: Storage, engine: Arc<dyn HttpEngine>, prefs: AppPrefs, extras: Extras) {
    i18n::apply(prefs.language.as_deref());
    this_build::add_fonts(cx);
    AppStore::init(storage, engine, prefs, extras, cx);
    // A Mac page's Cmd keys first: both sets work, and the shortcuts shown
    // are the last bound. The kit writes Cmd in a page as "Win", so a hint
    // reads Ctrl+F, which works there too, rather than a wrong Win+F.
    if this_build::mac_page() {
        views::bind_mac_page_keys(cx);
        views::bind_dashboard_mac_page_keys(cx);
    }
    views::bind_workspace_keys(cx);
    views::bind_dashboard_keys(cx);
    open_main_window(cx, |window, cx| cx.new(|cx| CyberApi::new(window, cx)));
}

/// A window that says why the data could not be opened, in place of the app.
pub fn launch_failed(cx: &mut App, message: String) {
    i18n::apply(None);
    open_main_window(cx, move |_, cx| cx.new(|_| StartupError::new(message)));
}

fn open_main_window<V: Render>(cx: &mut App, build: impl FnOnce(&mut Window, &mut App) -> Entity<V>) {
    #[cfg(target_os = "macos")]
    install_mac_menus(cx);
    // Closing the window hides the app on macOS (F1.4); elsewhere the last
    // window closing is the app quitting. The settings window alone is not
    // the app.
    #[cfg(not(any(target_os = "macos", target_family = "wasm")))]
    cx.on_window_closed(|cx, _| {
        if cx
            .windows()
            .into_iter()
            .all(|window| views::is_settings_window(window, cx))
        {
            cx.quit();
        }
    })
    .detach();

    cx.activate(true);
    let opened = gpui_kit::open_window(main_window_options(cx), cx, |window, cx| {
        #[cfg(target_os = "macos")]
        {
            window.on_window_should_close(cx, |_window, cx| {
                cx.hide();
                false
            });
            // Created hidden (see `show`) and revealed once the first themed
            // frame has painted, so the default white backing never shows.
            window.on_next_frame(|window, _cx| window.activate_window());
        }
        build(window, cx)
    });
    if let Err(e) = opened {
        error!(error = %e, "the main window could not be opened");
    }
}

fn main_window_options(cx: &App) -> WindowOptions {
    // F1.3: the size the settings name, at least 900 × 600. A connected
    // monitor with a recorded position uses that instead (see
    // `remembered_bounds`).
    let window = cx
        .try_global::<AppStore>()
        .map(|store| store.state().read(cx).prefs().window)
        .unwrap_or_default();
    let (width, height) = if window.width > 0 && window.height > 0 {
        (
            window.width.max(WindowPrefs::MIN_WIDTH),
            window.height.max(WindowPrefs::MIN_HEIGHT),
        )
    } else {
        (WindowPrefs::DEFAULT_WIDTH, WindowPrefs::DEFAULT_HEIGHT)
    };
    let chosen = size(px(width as f32), px(height as f32));
    let (display_id, bounds) =
        remembered_bounds(chosen, cx).unwrap_or_else(|| (None, Bounds::centered(None, chosen, cx)));
    let window_bounds = if window.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    };
    let mut titlebar = TitleBar::title_bar_options();
    // The title bar is drawn by the app, but the OS still lists the window
    // by this name (task switcher, window menu).
    titlebar.title = Some(APP_NAME.into());
    WindowOptions {
        window_bounds: Some(window_bounds),
        display_id,
        // F14.2: the smallest size the settings allow.
        window_min_size: Some(size(
            px(WindowPrefs::MIN_WIDTH as f32),
            px(WindowPrefs::MIN_HEIGHT as f32),
        )),
        titlebar: Some(titlebar),
        app_id: Some(APP_ID.to_string()),
        // Only macOS paints a hidden window. Windows drives frames with
        // WM_PAINT, which a hidden window never receives, and Wayland cannot
        // reliably reveal a window that was never mapped.
        show: cfg!(not(target_os = "macos")),
        ..TitleBar::window_options()
    }
}

/// The monitor the window opens on, by the displays' stable ids: the one it
/// was on last while that is connected, else a connected one with a recorded
/// position, the default display first. A recorded position is kept inside
/// that monitor's visible area; without one the window is centered there at
/// `chosen`, the size the settings name.
fn remembered_bounds(chosen: Size<Pixels>, cx: &App) -> Option<(Option<DisplayId>, Bounds<Pixels>)> {
    let prefs = cx.try_global::<AppStore>()?.state().read(cx).prefs();
    let primary = cx.primary_display().map(|display| display.id());
    let mut displays: Vec<_> = cx
        .displays()
        .into_iter()
        .filter_map(|display| Some((display.uuid().ok()?.to_string(), display)))
        .collect();
    displays.sort_by_key(|(_, display)| Some(display.id()) != primary);
    let ids: Vec<&str> = displays.iter().map(|(id, _)| id.as_str()).collect();
    let (wanted, placement) = opening_monitor(&prefs.monitors, &prefs.last_monitor, &ids)?;
    let display = &displays.iter().find(|(id, _)| id == wanted)?.1;
    let Some(placement) = placement else {
        return Some((Some(display.id()), Bounds::centered(Some(display.id()), chosen, cx)));
    };
    let visible = display.visible_bounds();
    let whole = |value: Pixels| f32::from(value).round() as i32;
    let placement = placement.fitted(MonitorPlacement {
        x: whole(visible.origin.x),
        y: whole(visible.origin.y),
        width: whole(visible.size.width).max(0) as u32,
        height: whole(visible.size.height).max(0) as u32,
    });
    Some((
        Some(display.id()),
        Bounds {
            origin: point(px(placement.x as f32), px(placement.y as f32)),
            size: size(px(placement.width as f32), px(placement.height as f32)),
        },
    ))
}

/// The menu bar. Closing the window only hides the app on macOS, so the
/// application menu is where Quit lives; Edit's items act on the focused
/// field, Window's on the main window.
#[cfg(target_os = "macos")]
fn install_mac_menus(cx: &mut App) {
    use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
    use gpui_kit::{KeyBinding, Menu, MenuItem, OsAction};

    // The bundle's name (`[package.metadata.bundle]`), which macOS shows as
    // the application menu's title; its items say the same.
    const BUNDLE_NAME: &str = "CyberAPI";

    fn on_window(cx: &mut App, act: impl FnOnce(&mut Window)) {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| act(window));
        }
    }

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| on_window(cx, |window| window.minimize_window()));
    cx.on_action(|_: &Zoom, cx| on_window(cx, |window| window.zoom_window()));
    cx.on_action(|_: &ToggleFullScreen, cx| on_window(cx, |window| window.toggle_fullscreen()));
    // As the close button does (F1.4).
    cx.on_action(|_: &CloseWindow, cx| cx.hide());
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
    ]);
    cx.set_menus([
        Menu::new(BUNDLE_NAME).items([
            MenuItem::action(format!("Hide {BUNDLE_NAME}"), Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action(format!("Quit {BUNDLE_NAME}"), Quit),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        // gpui makes the menu named "Window" the system's window menu.
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
            MenuItem::action("Enter Full Screen", ToggleFullScreen),
            MenuItem::separator(),
            MenuItem::action("Close Window", CloseWindow),
        ]),
    ]);
}
