//! CyberAPI in a browser tab.
//!
//! The same application the desktop runs, compiled to wasm:
//! `cyberapi_gui::launch` opens the same window the desktop opens. What this
//! crate adds is only the browser's way in — the platform, the UI font, the
//! fetched icons, and the stores, which are the bridge's (`BridgeStore`
//! through [`transport::FetchTransport`]).
//!
//! The page signs in before any of this runs (`www/index.html`), and hands in
//! the account's settings with the call.
//!
//! Build with `make web-bundle` (from the repo root; it runs from this
//! directory so rustup reads the `rust-toolchain.toml` here and selects the
//! nightly the browser backend needs), then serve it with `make bridge`.

#[cfg(target_family = "wasm")]
mod assets;
#[cfg(target_family = "wasm")]
mod transport;

#[cfg(target_family = "wasm")]
use cyberapi_core::prefs::AppPrefs;
#[cfg(target_family = "wasm")]
use cyberapi_store_bridge::BridgeStore;
#[cfg(target_family = "wasm")]
use gpui_kit::web::{CanvasFontFallback, WebBackendPreference, WebPlatform};
#[cfg(target_family = "wasm")]
use gpui_kit::{Application, ApplicationHandle, TextRenderingMode};
#[cfg(target_family = "wasm")]
use std::borrow::Cow;
#[cfg(target_family = "wasm")]
use std::cell::RefCell;
#[cfg(target_family = "wasm")]
use std::rc::Rc;
#[cfg(target_family = "wasm")]
use std::sync::Arc;
#[cfg(target_family = "wasm")]
use tracing::error;
#[cfg(target_family = "wasm")]
use tracing_subscriber::prelude::*;

/// The browser entry point, called by `www/index.html`.
///
/// `origin` is where the page was served from, without a trailing slash: the
/// kit's icons are fetched from `{origin}/assets/icons/…`. `ui_font` is the
/// bytes of the one family the web text system resolves the UI font to (IBM
/// Plex Sans): that system starts with no fonts at all, so the page fetches
/// the file and hands it in, and it is registered before the first frame
/// measures any text. `prefs` is the account's settings as the bridge answered
/// them (JSON; anything unreadable is the defaults): the first frame needs the
/// language and the theme, and asking from here would put a frame before them.
#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn run(origin: String, ui_font: Vec<u8>, prefs: String) {
    // Panics and the platform's `log` output go to the console.
    gpui_kit::platform::web_init();
    // Straight to `set_global_default`, not `init()`: `init` would install
    // the `log` → `tracing` bridge too, and `web_init` has already claimed
    // the `log` logger.
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .without_time()
            .with_writer(tracing_web::MakeWebConsoleWriter::new()),
    );
    if tracing::subscriber::set_global_default(subscriber).is_err() {
        error!("a tracing subscriber was already installed");
    }

    // `gpui_kit::platform::single_threaded_web()`, spelled out for the one
    // thing it does not take: the canvas font fallback, which is fixed at
    // construction. IBM Plex Sans has no CJK glyphs and a page cannot read
    // the system's fonts, so `EmojiAndCjk` lets the browser draw what the
    // loaded font lacks with its own `sans-serif`.
    //
    // Single-threaded (`false`): browser threads need cross-origin isolation
    // (`SharedArrayBuffer`), which a plain static server does not give.
    let platform = Rc::new(WebPlatform::new_with_backend_and_font_fallback(
        false,
        WebBackendPreference::Auto,
        CanvasFontFallback::EmojiAndCjk,
    ));
    let http_client = Arc::new(platform.fetch_http_client());
    let storage = BridgeStore::new(Arc::new(transport::FetchTransport::new(
        http_client.clone(),
        origin.clone(),
    )))
    .storage();
    let prefs: AppPrefs = serde_json::from_str(&prefs).unwrap_or_default();
    let web_assets = assets::WebAssets::new(&origin);
    let icon_watch = web_assets.take_watch();
    let app = Application::with_platform(platform)
        .with_http_client(http_client)
        .with_assets(web_assets);
    // `run_embedded`, not `run`: the browser's run loop belongs to the
    // browser, so `run` would schedule the launch and return, dropping the
    // app with it. The handle it returns owns the app for the page's life.
    let handle = app.run_embedded(move |cx| {
        if let Err(e) = cx.text_system().add_fonts(vec![Cow::Owned(ui_font)]) {
            error!(error = %e, "the UI font could not be registered");
        }
        gpui_kit::init(cx);
        // The web text system asks for subpixel antialiasing for the fonts
        // it was handed and grayscale for what the browser draws, so Latin
        // text in a CJK line would get colour fringes the characters beside
        // it do not. A page does not know the screen's subpixel order anyway.
        cx.set_text_rendering_mode(TextRenderingMode::Grayscale);
        if let Some(icon_watch) = icon_watch {
            assets::repaint_when_icons_land(icon_watch, cx);
        }
        cyberapi_gui::launch(cx, storage, prefs);
    });
    APP.with(|slot| *slot.borrow_mut() = Some(handle));
}

/// The page's `visibilitychange`: a background tab polls less often.
#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn set_page_visible(visible: bool) {
    cyberapi_gui::set_page_visible(visible);
}

// The page is the process: it lives until the tab closes, and so does this.
#[cfg(target_family = "wasm")]
thread_local! {
    static APP: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
}
