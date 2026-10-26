//! D1: what a request's `prompt`s ask for, asked before it is sent — all of
//! them in one dialog, each field starting with what was typed for it last.

use crate::views::name_dialog::ok_button;
use cyberapi_core::template::Prompt;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, v_flex};
use gpui_kit::{
    App, AppContext as _, Entity, Focusable as _, IntoElement as _, ParentElement as _, Styled as _, Window, div, px,
};
use rust_i18n::t;
use std::collections::HashMap;
use std::rc::Rc;

/// Asks for each of `prompts`, its field starting with `typed`'s answer for
/// its label, or else its start, and hands what was typed, by label, to
/// `on_ok`. Cancelling sends nothing.
pub fn open_prompt_dialog(
    window: &mut Window,
    cx: &mut App,
    prompts: Vec<Prompt>,
    typed: &HashMap<String, String>,
    on_ok: impl Fn(HashMap<String, String>, &mut Window, &mut App) + 'static,
) {
    let fields: Rc<Vec<(String, Entity<InputState>)>> = Rc::new(
        prompts
            .into_iter()
            .map(|prompt| {
                let start = typed.get(&prompt.label).cloned().or(prompt.start).unwrap_or_default();
                let input = cx.new(|cx| InputState::new(window, cx).default_value(start));
                (prompt.label, input)
            })
            .collect(),
    );
    let on_ok = Rc::new(on_ok);
    let first = fields.first().map(|(_, input)| input.clone());
    window.open_dialog(cx, move |dialog, _, cx| {
        let submit = {
            let (fields, on_ok) = (fields.clone(), on_ok.clone());
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                let answers = fields
                    .iter()
                    .map(|(label, input)| (label.clone(), input.read(cx).value().to_string()))
                    .collect();
                on_ok(answers, window, cx);
                true
            }
        };
        let rows = fields.iter().map(|(label, input)| {
            v_flex()
                .gap_1p5()
                .child(div().text_sm().font_medium().child(label.clone()))
                .child(Input::new(input))
                .into_any_element()
        });
        dialog
            .title(t!("editor.prompt_title").to_string())
            .w(px(420.))
            .child(crate::views::dialog_body(
                v_flex()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("editor.prompt_hint").to_string()),
                    )
                    .children(rows),
            ))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                    .child(ok_button(
                        "prompt-dialog-ok",
                        t!("editor.send").to_string(),
                        submit.clone(),
                    )),
            )
            .on_ok(submit)
    });
    // After the dialog took focus for itself, so typing starts in the field.
    if let Some(first) = first {
        first.focus_handle(cx).focus(window, cx);
    }
}
