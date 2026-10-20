//! CyberAPI, as a library.
//!
//! The desktop binary next to this file only calls [`run`]. The split exists
//! so the views can be compiled into the browser build as well: `cyberapi-web`
//! sets up the page's platform and its stores itself and enters through
//! [`launch`], which is everything both builds share.

// The app's strings. The current locale is rust-i18n's, shared with
// gpui-component, and set by `i18n::apply`.
rust_i18n::i18n!("locales", fallback = "en");

mod i18n;
mod pacing;
#[cfg(not(target_family = "wasm"))]
mod paths;
mod root;
pub mod states;
mod views;

pub use crate::pacing::set_page_visible;
use crate::root::CyberApi;
use crate::states::AppStore;
use crate::views::StartupError;
use cyberapi_core::prefs::AppPrefs;
use cyberapi_store::Storage;
use gpui_kit::component::TitleBar;
use gpui_kit::{App, AppContext as _, Bounds, Entity, Render, Window, WindowBounds, WindowOptions, px, size};
use tracing::error;

pub const APP_NAME: &str = "Cyber API";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The identifier the original Tauri build used; Linux groups windows by it.
const APP_ID: &str = "com.bigtree.cyberapi";

gpui_kit::actions!(cyberapi, [Quit, Hide, HideOthers, ShowAll]);

/// Everything the desktop `main` does.
#[cfg(not(target_family = "wasm"))]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_logger();
    tracing::info!(version = VERSION, "cyberapi launch");
    // Before the platform starts: the first window needs the settings.
    let opened = open_desktop_storage();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            // Must come before anything that uses a component.
            gpui_kit::init(cx);
            match opened {
                Ok((storage, prefs)) => launch(cx, storage, prefs),
                Err(message) => launch_failed(cx, message),
            }
        });
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
/// first launch (D2), and the reader's settings.
#[cfg(not(target_family = "wasm"))]
fn open_desktop_storage() -> Result<(Storage, AppPrefs), String> {
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
    let prefs = pollster::block_on(storage.prefs().app()).unwrap_or_else(|e| {
        warn!(error = %e, "the settings could not be read; using the defaults");
        AppPrefs::default()
    });
    Ok((storage, prefs))
}

/// Opens the main window on `storage`. Both builds call this once the
/// platform is up and `gpui_kit::init` has run: the desktop from [`run`],
/// the browser from `cyberapi-web`.
pub fn launch(cx: &mut App, storage: Storage, prefs: AppPrefs) {
    i18n::apply(prefs.language.as_deref());
    AppStore::init(storage, prefs, cx);
    views::bind_workspace_keys(cx);
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
    // window closing is the app quitting.
    #[cfg(not(any(target_os = "macos", target_family = "wasm")))]
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
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
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    let mut titlebar = TitleBar::title_bar_options();
    // The title bar is drawn by the app, but the OS still lists the window
    // by this name (task switcher, window menu).
    titlebar.title = Some(APP_NAME.into());
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        // F14.2: the smallest size the settings allow.
        window_min_size: Some(size(px(900.), px(600.))),
        titlebar: Some(titlebar),
        app_id: Some(APP_ID.to_string()),
        // Only macOS paints a hidden window. Windows drives frames with
        // WM_PAINT, which a hidden window never receives, and Wayland cannot
        // reliably reveal a window that was never mapped.
        show: cfg!(not(target_os = "macos")),
        ..TitleBar::window_options()
    }
}

/// The application menu. Closing the window only hides the app on macOS, so
/// this is where Quit lives.
#[cfg(target_os = "macos")]
fn install_mac_menus(cx: &mut App) {
    use gpui_kit::{KeyBinding, Menu, MenuItem};

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
    ]);
    cx.set_menus([Menu::new(APP_NAME).items([
        MenuItem::action(format!("Hide {APP_NAME}"), Hide),
        MenuItem::action("Hide Others", HideOthers),
        MenuItem::action("Show All", ShowAll),
        MenuItem::separator(),
        MenuItem::action(format!("Quit {APP_NAME}"), Quit),
    ])]);
}
