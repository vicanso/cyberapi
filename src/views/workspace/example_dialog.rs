//! M0: an API's example written by hand or edited — its name, status,
//! headers and body — and what the mock does with it (M1–M3): the
//! conditions it answers on, how long it waits, and the pause between the
//! events of a stream.

use crate::states::ApiState;
use crate::views::name_dialog::ok_button;
use cyberapi_core::mock::{conditions_from_text, conditions_text};
use cyberapi_core::model::{Example, KvParam, kv_from_text, kv_text};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Editor, EditorState, Input, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Entity, InteractiveElement as _, ParentElement as _, Styled as _, Window, div, px,
};
use rust_i18n::t;
use std::cell::RefCell;
use std::rc::Rc;

/// The fields, as typed.
struct Fields {
    name: Entity<InputState>,
    status: Entity<InputState>,
    delay: Entity<InputState>,
    interval: Entity<InputState>,
    headers: Entity<EditorState>,
    conditions: Entity<EditorState>,
    body: Entity<EditorState>,
}

impl Fields {
    /// The example they say, or what is wrong with them.
    fn read(&self, cx: &App) -> Result<Example, String> {
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let name = text(&self.name);
        if name.is_empty() {
            return Err(t!("errors.name_required").to_string());
        }
        let status = text(&self.status)
            .parse::<u16>()
            .ok()
            .filter(|status| (100..=599).contains(status))
            .ok_or_else(|| t!("example.status_invalid").to_string())?;
        let number =
            |input: &Entity<InputState>, field: &str, max: u32| {
                let typed = text(input);
                match typed.is_empty() {
                    true => Ok(0),
                    false => typed.parse::<u32>().ok().filter(|ms| *ms <= max).ok_or_else(|| {
                        t!("example.number_invalid", field = t!(field).to_string(), max = max).to_string()
                    }),
                }
            };
        let delay_ms = number(&self.delay, "example.delay", Example::MAX_DELAY_MS)?;
        let stream_interval_ms = number(&self.interval, "example.interval", Example::MAX_STREAM_INTERVAL_MS)?;
        let when = conditions_from_text(&self.conditions.read(cx).value())
            .map_err(|line| t!("example.condition_invalid", line = line).to_string())?;
        let body = self.body.read(cx).value().to_string();
        if body.len() > Example::BODY_LIMIT {
            return Err(t!("example.body_too_long").to_string());
        }
        let headers = kv_from_text(&self.headers.read(cx).value())
            .into_iter()
            .filter(|kv| kv.enabled && !kv.key.is_empty())
            .map(|kv| (kv.key, kv.value))
            .collect();
        Ok(Example {
            name,
            status,
            headers,
            body,
            when,
            delay_ms,
            stream_interval_ms,
        })
    }
}

/// Opens the dialog for `api`'s example at `at`, or a new one for `None`.
pub fn open_example_dialog(api: Entity<ApiState>, at: Option<usize>, window: &mut Window, cx: &mut App) {
    let example = at
        .and_then(|at| api.read(cx).request().examples.get(at).cloned())
        .unwrap_or_else(|| Example {
            status: 200,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: "{}".into(),
            ..Default::default()
        });
    let input = |value: String, window: &mut Window, cx: &mut App| {
        cx.new(|cx| InputState::new(window, cx).default_value(value))
    };
    let number = |ms: u32| if ms == 0 { String::new() } else { ms.to_string() };
    let editor = |value: String, json: bool, placeholder: Option<String>, window: &mut Window, cx: &mut App| {
        cx.new(|cx| {
            crate::views::fix_editor_search(cx);
            let editor = EditorState::new(window, cx).line_number(false).soft_wrap(false);
            let editor = match json {
                true => editor.language("json").tab_size(crate::views::json_tab()),
                false => editor,
            };
            let editor = match placeholder {
                Some(placeholder) => editor.placeholder(placeholder),
                None => editor,
            };
            editor.default_value(value)
        })
    };
    let headers = example
        .headers
        .iter()
        .map(|(key, value)| KvParam {
            key: key.clone(),
            value: value.clone(),
            enabled: true,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let json = example.content_type().is_none_or(|kind| kind.contains("json"));
    let fields = Rc::new(Fields {
        name: input(example.name.clone(), window, cx),
        status: input(example.status.to_string(), window, cx),
        delay: input(number(example.delay_ms), window, cx),
        interval: input(number(example.stream_interval_ms), window, cx),
        headers: editor(kv_text(&headers), false, None, window, cx),
        conditions: editor(
            conditions_text(&example.when),
            false,
            Some(t!("example.conditions_placeholder").to_string()),
            window,
            cx,
        ),
        body: editor(example.body.clone(), json, None, window, cx),
    });
    let problem: Rc<RefCell<Option<String>>> = Rc::default();
    let title = match at {
        Some(_) => t!("example.edit").to_string(),
        None => t!("example.new").to_string(),
    };
    window.open_dialog(cx, move |dialog, window, cx| {
        let submit = {
            let (fields, problem, api) = (fields.clone(), problem.clone(), api.downgrade());
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| match fields.read(cx) {
                Ok(example) => {
                    let _ = api.update(cx, |api, cx| api.set_example(at, example, cx));
                    true
                }
                Err(why) => {
                    *problem.borrow_mut() = Some(why);
                    window.refresh();
                    false
                }
            }
        };
        let theme = cx.theme();
        let (border, muted, danger) = (theme.border, theme.muted_foreground, theme.danger);
        let label = |key: &str| div().text_sm().font_medium().child(t!(key).to_string());
        let hint = |key: &str| div().text_xs().text_color(muted).child(t!(key).to_string());
        let boxed = |editor: &Entity<EditorState>, height: f32| {
            div()
                .h(px(height))
                .border_1()
                .border_color(border)
                .rounded_md()
                .child(Editor::new(editor).h_full().bordered(false))
        };
        let field = |key: &str, input: &Entity<InputState>| {
            v_flex().flex_1().gap_1().child(label(key)).child(Input::new(input))
        };
        // Within a short page the fields scroll; the footer and what is
        // wrong stay in sight.
        let height = (window.viewport_size().height - px(200.)).max(px(240.));
        dialog
            .title(title.clone())
            .w(px(640.))
            .on_ok(|_, _, _| false)
            .child(crate::views::dialog_body(
                v_flex()
                    .gap_2()
                    .child(
                        div().id("example-fields").max_h(height).overflow_y_scrollbar().child(
                            v_flex()
                                .gap_3()
                                .pr_4()
                                .child(
                                    h_flex()
                                        .gap_3()
                                        .child(field("common.name", &fields.name))
                                        .child(div().w(px(120.)).child(field("example.status", &fields.status))),
                                )
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .child(label("example.headers"))
                                                .child(hint("example.headers_hint")),
                                        )
                                        .child(boxed(&fields.headers, 72.)),
                                )
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(label("example.body"))
                                        .child(boxed(&fields.body, 180.)),
                                )
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(label("example.conditions"))
                                        .child(hint("example.conditions_hint"))
                                        .child(boxed(&fields.conditions, 72.)),
                                )
                                .child(
                                    h_flex()
                                        .gap_3()
                                        .child(field("example.delay", &fields.delay))
                                        .child(field("example.interval", &fields.interval)),
                                )
                                .child(hint("example.interval_hint")),
                        ),
                    )
                    .when_some(problem.borrow().clone(), |this, why| {
                        this.child(div().text_xs().text_color(danger).child(why))
                    }),
            ))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                    .child(ok_button("example-dialog-ok", t!("common.save").to_string(), submit)),
            )
    });
}
