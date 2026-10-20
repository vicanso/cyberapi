//! A dialog that asks for one name: a new API or folder (F3.4).

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{App, AppContext as _, Focusable as _, ParentElement as _, SharedString, Styled as _, Window, div, px};
use rust_i18n::t;
use std::cell::Cell;
use std::rc::Rc;

/// Asks for a name and hands it, trimmed and not blank, to `on_ok`.
pub fn open_name_dialog(
    window: &mut Window,
    cx: &mut App,
    title: SharedString,
    placeholder: SharedString,
    ok_label: SharedString,
    on_ok: impl Fn(String, &mut App) + 'static,
) {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
    let missing = Rc::new(Cell::new(false));
    let on_ok = Rc::new(on_ok);
    let field = input.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let submit = {
            let (input, missing, on_ok) = (input.clone(), missing.clone(), on_ok.clone());
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                let name = input.read(cx).value().trim().to_string();
                if name.is_empty() {
                    missing.set(true);
                    window.refresh();
                    return false;
                }
                on_ok(name, cx);
                true
            }
        };
        dialog
            .title(title.clone())
            .w(px(420.))
            .child(
                v_flex()
                    .gap_1p5()
                    .child(div().text_sm().font_medium().child(t!("common.name").to_string()))
                    .child(Input::new(&input))
                    .when(missing.get(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(t!("errors.name_required").to_string()),
                        )
                    }),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                    .child(DialogAction::new().child(Button::new("name-dialog-ok").primary().label(ok_label.clone()))),
            )
            .on_ok(submit)
    });
    // After the dialog took focus for itself, so typing starts in the field.
    field.focus_handle(cx).focus(window, cx);
}
