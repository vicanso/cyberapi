//! CyberAPI, as a library.
//!
//! The desktop binary next to this file only calls [`run`]. The split exists
//! so the views can be compiled into the browser build as well: `cyberapi-web`
//! sets up the page's platform itself and enters through [`launch`], which is
//! everything both builds share.

mod root;

use crate::root::CyberApi;
use gpui_kit::component::TitleBar;
use gpui_kit::{App, AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};
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
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(|cx| {
        // Must come before anything that uses a component.
        gpui_kit::init(cx);
        launch(cx);
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

/// Open the main window. Both builds call this once the platform is up and
/// `gpui_kit::init` has run: the desktop from [`run`], the browser from
/// `cyberapi-web`.
pub fn launch(cx: &mut App) {
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
        cx.new(|cx| CyberApi::new(window, cx))
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
