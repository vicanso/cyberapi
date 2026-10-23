//! The desktop's settings window, as zedis opens its own: one at a time,
//! on the monitor the app is on, with the app's title bar so it takes the
//! app's theme rather than the system's, and closed by Escape or ⌘W without
//! taking the app with it.

use crate::{APP_NAME, CloseWindow};
use gpui_kit::component::{ActiveTheme as _, TitleBar, h_flex, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Global, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, Styled as _, Window, WindowBounds, WindowOptions, div, px,
    size,
};
use rust_i18n::t;
use tracing::error;

/// The settings window while it is open.
#[derive(Default)]
struct Open(Option<AnyWindowHandle>);

impl Global for Open {}

/// Whether `window` is the settings window: it does not keep the app
/// running once the main window has gone. macOS keeps the app running with
/// no window at all.
#[cfg(not(target_os = "macos"))]
pub fn is_settings_window(window: AnyWindowHandle, cx: &App) -> bool {
    cx.try_global::<Open>().and_then(|open| open.0) == Some(window)
}

/// Opens the window `build` fills, or brings the open one forward.
pub(super) fn open<V: Render>(cx: &mut App, build: impl FnOnce(&mut Window, &mut App) -> Entity<V> + 'static) {
    if let Some(open) = cx.try_global::<Open>().and_then(|open| open.0)
        && open.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    // Centered on the monitor the main window is on, not always the
    // primary one.
    let display = cx
        .active_window()
        .and_then(|window| window.update(cx, |_, window, cx| window.display(cx)).ok().flatten())
        .map(|display| display.id());
    let mut titlebar = TitleBar::title_bar_options();
    titlebar.title = Some(format!("{APP_NAME} – {}", t!("settings.title")).into());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            display,
            size(px(760.), px(560.)),
            cx,
        ))),
        // Narrower and a row's text and its control no longer sit side by
        // side.
        window_min_size: Some(size(px(640.), px(420.))),
        titlebar: Some(titlebar),
        ..TitleBar::window_options()
    };
    match gpui_kit::open_window(options, cx, move |window, cx| {
        let content = build(window, cx);
        cx.new(|cx| Frame::new(content, window, cx))
    }) {
        Ok((handle, _)) => cx.set_global(Open(Some(handle))),
        Err(e) => error!(error = %e, "the settings window could not be opened"),
    }
}

/// The title bar over the settings, and the keys that close them.
struct Frame<V: Render> {
    focus_handle: FocusHandle,
    content: Entity<V>,
    /// What the system was last told the window is called.
    title: String,
}

impl<V: Render> Frame<V> {
    fn new(content: Entity<V>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            focus_handle,
            content,
            title: String::new(),
        }
    }
}

impl<V: Render> Render for Frame<V> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = t!("settings.title").to_string();
        // The name the system lists the window by follows a language chosen
        // here too.
        if title != self.title {
            window.set_window_title(&format!("{APP_NAME} – {title}"));
            self.title = title.clone();
        }
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, _| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
            // ⌘W closes this window, and only it: the app-wide answer hides
            // the app, which would leave the settings open on their own.
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .child(TitleBar::new().child(h_flex().flex_1().justify_center().text_sm().child(title)))
            .child(div().flex_1().min_h_0().w_full().child(self.content.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;

    struct Blank;

    impl Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    fn open_frame(cx: &mut TestAppContext) -> AnyWindowHandle {
        cx.open_window(size(px(400.), px(300.)), |window, cx| {
            let content = cx.new(|_| Blank);
            let frame = cx.new(|cx| Frame::new(content, window, cx));
            Root::new(frame, window, cx)
        })
        .into()
    }

    /// Escape and ⌘W close the settings, and only them.
    #[gpui_kit::test]
    async fn escape_or_close_window_closes_the_settings(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let escaped = open_frame(cx);
        cx.update_window(escaped, |_, window, cx| {
            window.render_frame(cx);
            window.press("escape", cx);
        })
        .expect("the window is open");
        cx.run_until_parked();
        assert!(cx.update_window(escaped, |_, _, _| ()).is_err(), "Escape closed it");

        let closed = open_frame(cx);
        cx.update_window(closed, |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_action(Box::new(CloseWindow), cx);
        })
        .expect("the window is open");
        cx.run_until_parked();
        assert!(cx.update_window(closed, |_, _, _| ()).is_err(), "⌘W closed it");
    }
}
