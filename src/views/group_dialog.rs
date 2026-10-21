//! The reader's group of a project, or a group's new name: typed, or one of
//! the groups there are picked with a click. Empty is no group.

use crate::views::name_dialog::ok_button;
use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{App, AppContext as _, Focusable as _, ParentElement as _, SharedString, Styled as _, Window, div, px};
use rust_i18n::t;
use std::rc::Rc;

/// Asks for a group: `on_ok` takes the name, trimmed, or `""` for none.
pub fn open_group_dialog(
    window: &mut Window,
    cx: &mut App,
    title: SharedString,
    placeholder: SharedString,
    current: String,
    groups: Vec<String>,
    on_ok: impl Fn(String, &mut App) + 'static,
) {
    let input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(current)
    });
    let on_ok = Rc::new(on_ok);
    let field = input.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let submit = {
            let (input, on_ok) = (input.clone(), on_ok.clone());
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                let name = input.read(cx).value().trim().to_string();
                on_ok(name, cx);
                true
            }
        };
        let picks = groups.iter().enumerate().map(|(index, name)| {
            let (name, on_ok) = (name.clone(), on_ok.clone());
            Button::new(("group-pick", index))
                .outline()
                .xsmall()
                .label(name.clone())
                .on_click(move |_, window, cx| {
                    on_ok(name.clone(), cx);
                    window.close_dialog(cx);
                })
        });
        dialog
            .title(title.clone())
            .w(px(420.))
            .child(
                v_flex()
                    .gap_3()
                    .child(Input::new(&input))
                    .when(!groups.is_empty(), |this| {
                        this.child(
                            v_flex()
                                .gap_1p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(t!("dashboard.existing_groups").to_string()),
                                )
                                .child(h_flex().flex_wrap().gap_1p5().children(picks)),
                        )
                    }),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                    .child(ok_button(
                        "group-dialog-ok",
                        t!("common.save").to_string(),
                        submit.clone(),
                    )),
            )
            .on_ok(submit)
    });
    // After the dialog took focus for itself, so typing starts in the field.
    field.focus_handle(cx).focus(window, cx);
}
