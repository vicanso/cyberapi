//! F3 / F11: an API's Docs tab — what it is for, its input and output fields
//! (from its parameters, its request JSON Schema, and its response JSON
//! Schema assertion, both editable right here), and the answers kept as
//! examples of what it returns.

use crate::states::ApiState;
use crate::views::name_dialog::ok_button;
use cyberapi_core::assertions::Subject;
use cyberapi_core::model::{ContentType, HttpRequest, KvParam};
use cyberapi_core::response::{StatusLevel, format_size, status_level};
use cyberapi_core::schema_docs::{self, SchemaField};
use cyberapi_core::send::{self, form_fields};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

/// The most of an example's body shown.
const SHOWN: usize = 64 * 1024;

/// A starter JSON Schema when there is no JSON body or response to infer from.
const SCHEMA_EXAMPLE: &str = "{\n    \"type\": \"object\",\n    \"required\": [\"id\"],\n    \"properties\": {\n        \"id\": {\n            \"type\": \"integer\",\n            \"description\": \"\"\n        }\n    }\n}\n";

pub struct DocsEditor {
    api: Entity<ApiState>,
    description: Entity<EditorState>,
    /// The example opened to read, by its place.
    open: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

impl DocsEditor {
    pub fn new(api: Entity<ApiState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = api.read(cx).request().description.clone();
        let description = cx.new(|cx| {
            let mut editor = EditorState::new(window, cx)
                .line_number(false)
                .soft_wrap(true)
                .placeholder(t!("docs.placeholder").to_string());
            editor.set_value(text, window, cx);
            editor
        });
        let mut subscriptions = vec![
            cx.observe(&api, |_, _, cx| cx.notify()),
            cx.subscribe(&description, |this, editor, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = editor.read(cx).value().to_string();
                    this.api.update(cx, |api, cx| api.set_description(text, cx));
                }
            }),
        ];
        if let Some(workspace) = api.read(cx).workspace().upgrade() {
            subscriptions.push(cx.observe(&workspace, |_, _, cx| cx.notify()));
        }
        Self {
            api,
            description,
            open: None,
            _subscriptions: subscriptions,
        }
    }

    /// F11: opens a JSON editor dialog for either the request body schema
    /// (`is_request = true`) or the response body schema (`Subject::Schema`).
    fn edit_schema(&mut self, is_request: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (existing, imported, sample) = {
            let api = self.api.read(cx);
            let request = api.request();
            if is_request {
                let sample = (request.content_type == ContentType::Json)
                    .then(|| serde_json::from_str::<serde_json::Value>(&request.body).ok())
                    .flatten();
                (request.request_schema.clone(), false, sample)
            } else {
                let (schema, imported) = match api.response_schema() {
                    Some((text, imported)) => (Some(text.to_string()), imported),
                    None => (None, false),
                };
                let sample = api
                    .shown_response()
                    .and_then(|response| response.body_json())
                    .or_else(|| {
                        request
                            .examples
                            .iter()
                            .find_map(|example| serde_json::from_str::<serde_json::Value>(&example.body).ok())
                    });
                (schema, imported, sample)
            }
        };
        let has_existing = existing.as_ref().is_some_and(|text| !text.trim().is_empty());
        let initial = match &existing {
            Some(text) if !text.trim().is_empty() => text.clone(),
            _ => match &sample {
                Some(value) => cyberapi_core::format::json(&schema_docs::infer_schema(value)),
                None => SCHEMA_EXAMPLE.to_string(),
            },
        };
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .soft_wrap(false)
                .tab_size(crate::views::json_tab())
                .default_value(initial)
        });
        let api = self.api.downgrade();
        let title = if is_request {
            t!("docs.request_schema_title").to_string()
        } else {
            t!("docs.response_schema_title").to_string()
        };
        let infer_label = if is_request {
            t!("docs.infer_from_body").to_string()
        } else {
            t!("docs.infer_from_response").to_string()
        };

        window.open_dialog(cx, move |dialog, _, cx| {
            let save = {
                let (editor, api) = (editor.clone(), api.clone());
                move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                    let written = editor.read(cx).value().to_string();
                    let next = (!written.trim().is_empty()).then_some(written);
                    let _ = api.update(cx, |api, cx| {
                        if is_request {
                            api.set_request_schema(next, cx);
                        } else {
                            api.set_response_schema(next, cx);
                        }
                    });
                    true
                }
            };
            let muted = cx.theme().muted_foreground;
            let sample_for_btn = sample.clone();
            let editor_for_infer = editor.clone();
            let editor_for_fmt = editor.clone();
            let api_for_clear = api.clone();

            dialog
                .title(title.clone())
                .w(px(640.))
                .child(crate::views::dialog_body(
                    v_flex()
                        .gap_2()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(t!("docs.schema_hint").to_string()),
                                )
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .flex_none()
                                        .when_some(sample_for_btn, |this, sample_val| {
                                            let label = infer_label.clone();
                                            this.child(
                                                Button::new("docs-schema-infer")
                                                    .ghost()
                                                    .xsmall()
                                                    .label(label)
                                                    .on_click(move |_, window, cx| {
                                                        let text = cyberapi_core::format::json(
                                                            &schema_docs::infer_schema(&sample_val),
                                                        );
                                                        editor_for_infer.update(cx, |state, cx| {
                                                            state.set_value(text, window, cx);
                                                        });
                                                    }),
                                            )
                                        })
                                        .child(
                                            Button::new("docs-schema-format")
                                                .ghost()
                                                .xsmall()
                                                .label(t!("editor.format").to_string())
                                                .on_click(move |_, window, cx| {
                                                    let raw = editor_for_fmt.read(cx).value().to_string();
                                                    match send::format_json(&raw) {
                                                        Ok(pretty) => {
                                                            editor_for_fmt.update(cx, |state, cx| {
                                                                state.set_value(pretty, window, cx);
                                                            });
                                                        }
                                                        Err(problem) => {
                                                            window.push_notification(Notification::error(problem), cx);
                                                        }
                                                    }
                                                }),
                                        ),
                                ),
                        )
                        .when(imported, |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(t!("assertions.schema_imported").to_string()),
                            )
                        })
                        .child(
                            div()
                                .h(px(360.))
                                .border_1()
                                .border_color(cx.theme().border)
                                .rounded_md()
                                .child(Editor::new(&editor).h_full().bordered(false)),
                        ),
                ))
                .footer(
                    DialogFooter::new()
                        .when(has_existing, |footer| {
                            footer.child(
                                Button::new("docs-schema-clear")
                                    .ghost()
                                    .label(t!("docs.clear_schema").to_string())
                                    .on_click(move |_, window, cx| {
                                        let _ = api_for_clear.update(cx, |api, cx| {
                                            if is_request {
                                                api.set_request_schema(None, cx);
                                            } else {
                                                api.set_response_schema(None, cx);
                                            }
                                        });
                                        window.close_dialog(cx);
                                    }),
                            )
                        })
                        .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                        .child(ok_button("docs-schema-save", t!("common.save").to_string(), save)),
                )
                .on_ok(|_, _, _| false)
        });
    }

    fn render_flat_param(
        index: usize,
        place: &'static str,
        param: &KvParam,
        fallback_desc: Option<&str>,
        from_source_row: bool,
        cx: &App,
    ) -> AnyElement {
        let theme = cx.theme();
        let (border, fg, muted, mono) = (
            theme.border,
            theme.foreground,
            theme.muted_foreground,
            theme.mono_font_family.clone(),
        );
        let own = param.description.trim();
        let (desc, from_source) = if !own.is_empty() {
            (own.to_string(), from_source_row)
        } else if let Some(matched) = fallback_desc.map(str::trim).filter(|text| !text.is_empty()) {
            (matched.to_string(), true)
        } else if place == "file" {
            (param.value.clone(), from_source_row)
        } else {
            (String::new(), from_source_row)
        };
        h_flex()
            .id(("docs-param", index))
            .gap_3()
            .items_center()
            .py_1()
            .border_t_1()
            .border_color(border)
            .when(!param.enabled, |this| this.opacity(0.5))
            .child(
                div()
                    .w(px(190.))
                    .flex_none()
                    .truncate()
                    .text_xs()
                    .font_family(mono.clone())
                    .text_color(fg)
                    .child(param.key.trim().to_string()),
            )
            .child(
                div()
                    .w(px(110.))
                    .flex_none()
                    .truncate()
                    .text_xs()
                    .font_family(mono)
                    .text_color(muted)
                    .child(place),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1p5()
                    .items_center()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(if desc.is_empty() { muted } else { fg })
                            .child(desc),
                    )
                    .when(from_source, |this| {
                        this.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(muted)
                                .child(format!("({})", t!("docs.from_source"))),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_schema_field(id_prefix: &'static str, index: usize, field: &SchemaField, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (border, fg, muted, warning, mono) = (
            theme.border,
            theme.foreground,
            theme.muted_foreground,
            theme.warning,
            theme.mono_font_family.clone(),
        );
        let indent = px((field.depth.min(8) as f32) * 14.0);
        h_flex()
            .id((id_prefix, index))
            .gap_3()
            .items_center()
            .py_1()
            .border_t_1()
            .border_color(border)
            .child(
                h_flex()
                    .w(px(190.))
                    .flex_none()
                    .gap_1p5()
                    .items_center()
                    .pl(indent)
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .font_family(mono.clone())
                            .text_color(fg)
                            .child(field.name.clone()),
                    )
                    .when(field.required, |this| {
                        this.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(warning)
                                .child(t!("docs.required").to_string()),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(110.))
                    .flex_none()
                    .truncate()
                    .text_xs()
                    .font_family(mono)
                    .text_color(muted)
                    .child(field.kind.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(if field.description.is_empty() { muted } else { fg })
                    .child(field.description.clone()),
            )
            .into_any_element()
    }

    fn matched_source(&self, cx: &App) -> Option<HttpRequest> {
        let api = self.api.read(cx);
        let workspace = api.workspace().upgrade()?;
        workspace
            .read(cx)
            .matched_source_request(api.api(), api.request())
            .cloned()
    }

    fn render_inputs(&self, matched: Option<&HttpRequest>, cx: &mut Context<Self>) -> AnyElement {
        fn find_source_desc<'a>(list: &'a [KvParam], key: &str, ignore_case: bool) -> Option<&'a str> {
            list.iter()
                .find(|kv| {
                    !kv.description.trim().is_empty()
                        && if ignore_case {
                            kv.key.trim().eq_ignore_ascii_case(key.trim())
                        } else {
                            kv.key.trim() == key.trim()
                        }
                })
                .map(|kv| kv.description.as_str())
        }

        let theme = cx.theme();
        let (border, muted, warning) = (theme.border, theme.muted_foreground, theme.warning);
        let request = self.api.read(cx).request().clone();

        let mut rows: Vec<AnyElement> = Vec::new();
        let mut flat_idx = 0usize;
        for kv in request.variables.iter().filter(|kv| !kv.key.trim().is_empty()) {
            rows.push(Self::render_flat_param(flat_idx, "var", kv, None, false, cx));
            flat_idx += 1;
        }
        for kv in request.query.iter().filter(|kv| !kv.key.trim().is_empty()) {
            let fallback = matched.and_then(|m| find_source_desc(&m.query, &kv.key, false));
            rows.push(Self::render_flat_param(flat_idx, "query", kv, fallback, false, cx));
            flat_idx += 1;
        }
        if let Some(m) = matched {
            for kv in m
                .query
                .iter()
                .filter(|kv| !kv.key.trim().is_empty() && !kv.description.trim().is_empty())
            {
                if !request.query.iter().any(|own| own.key.trim() == kv.key.trim()) {
                    rows.push(Self::render_flat_param(flat_idx, "query", kv, None, true, cx));
                    flat_idx += 1;
                }
            }
        }
        for kv in request.headers.iter().filter(|kv| !kv.key.trim().is_empty()) {
            let fallback = matched.and_then(|m| find_source_desc(&m.headers, &kv.key, true));
            rows.push(Self::render_flat_param(flat_idx, "header", kv, fallback, false, cx));
            flat_idx += 1;
        }
        if let Some(m) = matched {
            for kv in m
                .headers
                .iter()
                .filter(|kv| !kv.key.trim().is_empty() && !kv.description.trim().is_empty())
            {
                if !request
                    .headers
                    .iter()
                    .any(|own| own.key.trim().eq_ignore_ascii_case(kv.key.trim()))
                {
                    rows.push(Self::render_flat_param(flat_idx, "header", kv, None, true, cx));
                    flat_idx += 1;
                }
            }
        }
        let matched_form = matched
            .filter(|m| m.content_type.is_key_value())
            .map(|m| form_fields(&m.body))
            .unwrap_or_default();
        if request.content_type.is_key_value() {
            let multipart = request.content_type == ContentType::Multipart;
            let own_form = form_fields(&request.body);
            for kv in own_form.iter().filter(|kv| !kv.key.trim().is_empty()) {
                let place = if multipart && kv.value.starts_with("file://") {
                    "file"
                } else {
                    "form"
                };
                let fallback = find_source_desc(&matched_form, &kv.key, false);
                rows.push(Self::render_flat_param(flat_idx, place, kv, fallback, false, cx));
                flat_idx += 1;
            }
            for kv in matched_form
                .iter()
                .filter(|kv| !kv.key.trim().is_empty() && !kv.description.trim().is_empty())
            {
                if !own_form.iter().any(|own| own.key.trim() == kv.key.trim()) {
                    let place = if multipart && kv.value.starts_with("file://") {
                        "file"
                    } else {
                        "form"
                    };
                    rows.push(Self::render_flat_param(flat_idx, place, kv, None, true, cx));
                    flat_idx += 1;
                }
            }
        }

        let (active_schema, schema_from_source) = match &request.request_schema {
            Some(schema) => (Some(schema.as_str()), false),
            None => (matched.and_then(|m| m.request_schema.as_deref()), true),
        };
        let mut schema_problem: Option<String> = None;
        if let Some(schema) = active_schema {
            match schema_docs::fields_of_schema(schema) {
                Ok(fields) => {
                    for (index, field) in fields.iter().enumerate() {
                        rows.push(Self::render_schema_field("docs-in-schema", index, field, cx));
                    }
                }
                Err(why) => {
                    schema_problem = Some(t!("docs.bad_schema", why = why).to_string());
                }
            }
        }

        let empty = rows.is_empty() && schema_problem.is_none();
        v_flex()
            .pt_2()
            .gap_1p5()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().text_sm().font_semibold().child(t!("docs.inputs").to_string()))
                            .when(schema_from_source && active_schema.is_some(), |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(format!("({})", t!("docs.from_source"))),
                                )
                            }),
                    )
                    .child(
                        Button::new("docs-edit-request-schema")
                            .ghost()
                            .xsmall()
                            .label(t!("docs.edit_request_schema").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.edit_schema(true, window, cx);
                            })),
                    ),
            )
            .when_some(schema_problem, |this, problem| {
                this.child(div().text_xs().text_color(warning).child(problem))
            })
            .when(empty, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("docs.no_inputs").to_string()),
                )
            })
            .when(!rows.is_empty(), |this| {
                this.child(v_flex().border_b_1().border_color(border).children(rows))
            })
            .into_any_element()
    }

    fn render_outputs(&self, matched: Option<&HttpRequest>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, warning) = (theme.border, theme.muted_foreground, theme.warning);
        let own_schema = self
            .api
            .read(cx)
            .response_schema()
            .map(|(schema, _)| schema.to_string());
        let (response_schema, from_source) = match own_schema {
            Some(schema) => (Some(schema), false),
            None => (
                matched.and_then(|m| {
                    m.assertions
                        .iter()
                        .filter(|assertion| assertion.subject == Subject::Schema)
                        .max_by_key(|assertion| assertion.enabled)
                        .map(|assertion| assertion.expected.clone())
                }),
                true,
            ),
        };

        let mut rows: Vec<AnyElement> = Vec::new();
        let mut schema_problem: Option<String> = None;
        if let Some(schema) = &response_schema {
            match schema_docs::fields_of_schema(schema) {
                Ok(fields) => {
                    for (index, field) in fields.iter().enumerate() {
                        rows.push(Self::render_schema_field("docs-out-schema", index, field, cx));
                    }
                }
                Err(why) => {
                    schema_problem = Some(t!("docs.bad_schema", why = why).to_string());
                }
            }
        }

        let empty = rows.is_empty() && schema_problem.is_none();
        v_flex()
            .pt_2()
            .gap_1p5()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().text_sm().font_semibold().child(t!("docs.outputs").to_string()))
                            .when(from_source && response_schema.is_some(), |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(format!("({})", t!("docs.from_source"))),
                                )
                            }),
                    )
                    .child(
                        Button::new("docs-edit-response-schema")
                            .ghost()
                            .xsmall()
                            .label(t!("docs.edit_response_schema").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.edit_schema(false, window, cx);
                            })),
                    ),
            )
            .when_some(schema_problem, |this, problem| {
                this.child(div().text_xs().text_color(warning).child(problem))
            })
            .when(empty, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("docs.no_outputs").to_string()),
                )
            })
            .when(!rows.is_empty(), |this| {
                this.child(v_flex().border_b_1().border_color(border).children(rows))
            })
            .into_any_element()
    }
}

impl Render for DocsEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, mono) = (theme.border, theme.muted_foreground, theme.mono_font_family.clone());
        let matched = self.matched_source(cx);
        let inputs = self.render_inputs(matched.as_ref(), cx);
        let outputs = self.render_outputs(matched.as_ref(), cx);
        let examples = self.api.read(cx).request().examples.clone();
        let rows: Vec<AnyElement> = examples
            .iter()
            .enumerate()
            .map(|(at, example)| {
                let theme = cx.theme();
                let color = match status_level(example.status) {
                    StatusLevel::Success => theme.success,
                    StatusLevel::Redirect => theme.info,
                    StatusLevel::ClientError => theme.warning,
                    StatusLevel::ServerError => theme.danger,
                };
                let opened = self.open == Some(at);
                let note = [
                    example.content_type().map(str::to_string),
                    Some(format_size(example.body.len() as u64)),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                let shown = &example.body[..example.body.floor_char_boundary(SHOWN)];
                v_flex()
                    .border_t_1()
                    .border_color(border)
                    .py_1()
                    .child(
                        h_flex()
                            .id(("example", at))
                            .gap_3()
                            .items_center()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open = if this.open == Some(at) { None } else { Some(at) };
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .w(px(36.))
                                    .flex_none()
                                    .font_semibold()
                                    .text_color(color)
                                    .child(example.status.to_string()),
                            )
                            .child(div().flex_1().min_w_0().truncate().child(example.name.clone()))
                            .child(div().text_xs().text_color(muted).child(note))
                            .child(
                                Button::new(("example-remove", at))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .tooltip(SharedString::from(t!("common.delete").to_string()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open = None;
                                        this.api.update(cx, |api, cx| api.remove_example(at, cx));
                                    })),
                            ),
                    )
                    .when(opened, |this| {
                        this.child(
                            div()
                                .id(("example-body", at))
                                .mt_1()
                                .max_h(px(260.))
                                .overflow_y_scrollbar()
                                .p_2()
                                .rounded_md()
                                .bg(cx.theme().muted)
                                .text_xs()
                                .font_family(mono.clone())
                                .child(shown.to_string()),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        v_flex()
            .id("request-docs")
            .size_full()
            .overflow_y_scrollbar()
            .px_3()
            .py_2()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(t!("docs.description").to_string()),
            )
            .child(
                div()
                    .h(px(140.))
                    .border_1()
                    .border_color(border)
                    .rounded_md()
                    .child(Editor::new(&self.description).w_full().h_full().bordered(false)),
            )
            .child(inputs)
            .child(outputs)
            .child(
                div()
                    .pt_2()
                    .text_sm()
                    .font_semibold()
                    .child(t!("docs.examples").to_string()),
            )
            .when(empty, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("docs.no_examples").to_string()),
                )
            })
            .when(!empty, |this| {
                this.child(v_flex().border_b_1().border_color(border).children(rows))
            })
    }
}
