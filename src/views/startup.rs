//! What the window shows when the data could not be opened — a database from a
//! newer build, a file the disk will not give up — instead of an app that
//! would fail at every click.

use gpui_kit::component::{ActiveTheme as _, StyledExt as _, TitleBar, v_flex};
use gpui_kit::{Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div};
use rust_i18n::t;

pub struct StartupError {
    message: SharedString,
}

impl StartupError {
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Render for StartupError {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(TitleBar::new())
            .child(
                v_flex()
                    .flex_1()
                    .justify_center()
                    .items_center()
                    .gap_3()
                    .px_8()
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .child(t!("startup.failed_title").to_string()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(t!("startup.failed_hint").to_string()),
                    )
                    .child(
                        div()
                            .max_w(gpui_kit::px(640.))
                            .p_3()
                            .rounded(theme.radius)
                            .bg(theme.muted)
                            .text_sm()
                            .child(self.message.clone()),
                    ),
            )
    }
}
