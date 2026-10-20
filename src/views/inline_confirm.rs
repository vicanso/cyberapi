//! A question asked inside a dialog, not in one stacked on it: the browser
//! build opens a second dialog over a first but does not draw it
//! (gpui-kit 0.8), so every dialog asks this way.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{AnyElement, App, IntoElement, ParentElement as _, Styled as _, Window, div};
use rust_i18n::t;

/// `title` and `description`, with what to ask in `content` (or nothing),
/// a cancel button and one that does `ok`.
pub fn inline_confirm(
    title: String,
    description: String,
    content: Option<AnyElement>,
    ok: String,
    on_cancel: impl Fn(&mut Window, &mut App) + 'static,
    on_ok: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .gap_2()
        .p_3()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.danger)
        .child(div().text_sm().font_semibold().child(title))
        .child(div().text_sm().text_color(theme.muted_foreground).child(description))
        .children(content)
        .child(
            h_flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new("inline-cancel")
                        .outline()
                        .small()
                        .label(t!("common.cancel").to_string())
                        .on_click(move |_, window, cx| on_cancel(window, cx)),
                )
                .child(
                    Button::new("inline-ok")
                        .small()
                        .with_variant(ButtonVariant::Danger)
                        .label(ok)
                        .on_click(move |_, window, cx| on_ok(window, cx)),
                ),
        )
}
