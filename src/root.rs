//! The main window's content: the title bar and the page below it.

use crate::{APP_NAME, VERSION};
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, Theme, TitleBar, v_flex};
use gpui_kit::{Context, IntoElement, ParentElement as _, Render, Styled as _, Subscription, Window, div};

pub struct CyberApi {
    _appearance: Subscription,
}

impl CyberApi {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // F14.1: the default theme follows the system, and keeps following it.
        Theme::sync_system_appearance(Some(window), cx);
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
        });
        Self {
            _appearance: appearance,
        }
    }
}

impl Render for CyberApi {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(TitleBar::new().child(div().text_sm().child(APP_NAME)))
            .child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .child(div().text_2xl().font_semibold().child(APP_NAME))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(format!("v{VERSION}")),
                    ),
            )
    }
}
