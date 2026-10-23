//! What the window shows when the data could not be opened — a database from a
//! newer build, a file the disk will not give up — instead of an app that
//! would fail at every click.

use crate::this_build;
use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, TitleBar, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    ClipboardItem, Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div,
};
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
                    )
                    // What to do about it: the details to pass on (the text
                    // cannot be selected), and the folder the data is in.
                    .child(
                        h_flex()
                            .gap_2()
                            .child({
                                let message = self.message.clone();
                                Button::new("startup-copy")
                                    .outline()
                                    .small()
                                    .label(t!("common.copy").to_string())
                                    .on_click(move |_, window, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(message.to_string()));
                                        window.push_notification(
                                            Notification::success(t!("common.copied").to_string()),
                                            cx,
                                        );
                                    })
                            })
                            .children(this_build::shows_data().then(|| {
                                Button::new("startup-show-data")
                                    .outline()
                                    .small()
                                    .label(t!("settings.show_data").to_string())
                                    .on_click(|_, _, cx| this_build::show_data(cx))
                            })),
                    ),
            )
    }
}
