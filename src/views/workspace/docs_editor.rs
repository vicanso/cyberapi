//! F3 / F11: an API's Docs tab — what it is for, its input and output fields
//! (from its parameters, its request JSON Schema, and its response JSON
//! Schema assertion, both editable right here), and the answers kept as
//! examples of what it returns.

use crate::states::{ApiEvent, ApiState};
use crate::views::name_dialog::ok_button;
use cyberapi_core::assertions::Subject;
use cyberapi_core::model::{ContentType, Example, HttpRequest, KvParam};
use cyberapi_core::response::{StatusLevel, format_size, status_level};
use cyberapi_core::schema_docs::{self, SchemaField};
use cyberapi_core::send::{self, form_fields};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::text::TextView;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Window, div, px, relative,
};
use rust_i18n::t;

/// The most of an example's body shown.
const SHOWN: usize = 64 * 1024;

/// A starter JSON Schema when there is no JSON body or response to infer from.
const SCHEMA_EXAMPLE: &str = "{\n    \"type\": \"object\",\n    \"required\": [\"id\"],\n    \"properties\": {\n        \"id\": {\n            \"type\": \"integer\",\n            \"description\": \"\"\n        }\n    }\n}\n";

/// A page-picked file shows its name. The stored marker stays on the request.
fn shown_file(kv: &KvParam) -> KvParam {
    match send::page_file(&kv.value) {
        Some((_, name)) => KvParam {
            value: name.to_string(),
            ..kv.clone()
        },
        None => kv.clone(),
    }
}

fn example_shown(body: &str) -> &str {
    &body[..body.floor_char_boundary(SHOWN)]
}

fn example_language(example: &Example) -> &'static str {
    let content_type = example.content_type().map(str::to_ascii_lowercase).unwrap_or_default();
    if content_type.contains("json") {
        "json"
    } else if content_type.contains("html") || content_type.contains("xml") {
        "html"
    } else if example.body.len() <= SHOWN
        && matches!(example.body.trim_start().as_bytes().first(), Some(b'{' | b'['))
        && serde_json::from_str::<serde_json::Value>(&example.body).is_ok()
    {
        "json"
    } else {
        "text"
    }
}

fn example_height(shown: &str, mono_font_size: Pixels) -> Pixels {
    let mut lines: usize = 0;
    for line in shown.lines() {
        lines += line.len().div_ceil(72).max(1);
        if lines >= 20 {
            break;
        }
    }
    px((lines.max(1) as f32 * (f32::from(mono_font_size) * 1.5) + 20.).clamp(60., 260.))
}

fn copy_example(body: String, window: &mut Window, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(body));
    window.push_notification(Notification::success(t!("common.copied").to_string()), cx);
}

struct OpenExample {
    index: usize,
    editor: Entity<EditorState>,
    body: String,
    language: &'static str,
}

pub struct DocsEditor {
    api: Entity<ApiState>,
    description: Entity<EditorState>,
    /// F16: whether the description is shown as an editor (`true`) or as
    /// rendered Markdown (`false`) when non-empty.
    editing_description: bool,
    /// The example opened to read, with its read-only code editor.
    open: Option<OpenExample>,
    _subscriptions: Vec<Subscription>,
}

impl DocsEditor {
    pub fn new(api: Entity<ApiState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = api.read(cx).request().description.clone();
        let editable = api
            .read(cx)
            .workspace()
            .upgrade()
            .is_none_or(|workspace| workspace.read(cx).editable());
        let editing_description = editable && text.trim().is_empty();
        let description = cx.new(|cx| {
            let mut editor = EditorState::new(window, cx)
                .line_number(false)
                .soft_wrap(true)
                .placeholder(t!("docs.placeholder").to_string());
            editor.set_value(text, window, cx);
            crate::views::fix_editor_search(cx);
            editor
        });
        let mut subscriptions = vec![
            cx.observe_in(&api, window, |this, _, window, cx| {
                this.sync_open_example(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(&api, window, |this, api, event: &ApiEvent, window, cx| {
                if matches!(event, ApiEvent::RequestReplaced) {
                    let text = api.read(cx).request().description.clone();
                    this.sync_open_example(window, cx);
                    if this.description.read(cx).value().as_ref() != text {
                        this.description
                            .update(cx, |editor, cx| editor.set_value(text.clone(), window, cx));
                    }
                    this.editing_description = this.editable(cx) && text.trim().is_empty();
                    cx.notify();
                }
            }),
            cx.subscribe(&description, |this, editor, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) && this.editable(cx) {
                    let text = editor.read(cx).value().to_string();
                    this.api.update(cx, |api, cx| api.set_description(text, cx));
                }
            }),
        ];
        if let Some(workspace) = api.read(cx).workspace().upgrade() {
            subscriptions.push(cx.observe(&workspace, |this, workspace, cx| {
                if !workspace.read(cx).editable() {
                    this.editing_description = false;
                }
                cx.notify();
            }));
        }
        Self {
            api,
            description,
            editing_description,
            open: None,
            _subscriptions: subscriptions,
        }
    }

    fn build_example_editor(
        shown: &str,
        language: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<EditorState> {
        cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .line_number(false)
                .tab_size(crate::views::json_tab())
                .soft_wrap(true)
                .searchable(false)
                .default_value(shown.to_string())
        })
    }

    fn toggle_example(&mut self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.as_ref().is_some_and(|open| open.index == at) {
            self.open = None;
            cx.notify();
            return;
        }
        let Some(example) = self.api.read(cx).request().examples.get(at).cloned() else {
            return;
        };
        let shown = example_shown(&example.body);
        let language = example_language(&example);
        let editor = Self::build_example_editor(shown, language, window, cx);
        self.open = Some(OpenExample {
            index: at,
            editor,
            body: shown.to_string(),
            language,
        });
        cx.notify();
    }

    fn sync_open_example(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.open.as_ref().map(|open| open.index) else {
            return;
        };
        let Some(example) = self.api.read(cx).request().examples.get(index).cloned() else {
            self.open = None;
            return;
        };
        let shown = example_shown(&example.body);
        let language = example_language(&example);
        let needs_rebuild = self.open.as_ref().is_some_and(|open| open.language != language);
        if needs_rebuild {
            let editor = Self::build_example_editor(shown, language, window, cx);
            if let Some(open) = &mut self.open {
                open.editor = editor;
                open.language = language;
                open.body = shown.to_string();
            }
            return;
        }
        let editor = self
            .open
            .as_ref()
            .filter(|open| open.body != shown)
            .map(|open| open.editor.clone());
        if let Some(editor) = editor {
            editor.update(cx, |editor, cx| editor.set_value(shown.to_string(), window, cx));
            if let Some(open) = &mut self.open {
                open.body = shown.to_string();
            }
        }
    }

    fn editable(&self, cx: &App) -> bool {
        self.api
            .read(cx)
            .workspace()
            .upgrade()
            .is_none_or(|workspace| workspace.read(cx).editable())
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
            crate::views::fix_editor_search(cx);
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
        let (border, fg, muted, warning, mono) = (
            theme.border,
            theme.foreground,
            theme.muted_foreground,
            theme.warning,
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
                    .w(px(72.))
                    .flex_none()
                    .truncate()
                    .text_xs()
                    .font_family(mono.clone())
                    .text_color(muted)
                    .child(place),
            )
            .when(matches!(place, "path" | "query" | "header"), |row| {
                let fact = |slot: u8, text: String, hint: String, color: gpui_kit::Hsla| {
                    div()
                        .id(("docs-fact", index * 4 + usize::from(slot)))
                        .w(px(0.))
                        .flex_1()
                        .max_w(px(160.))
                        .min_w_0()
                        .overflow_hidden()
                        .truncate()
                        .text_xs()
                        .font_family(mono.clone())
                        .text_color(color)
                        .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                        .child(text)
                };
                let mut row = row;
                if !param.schema_type.is_empty() {
                    row = row.child(fact(0, param.schema_type.clone(), t!("docs.type").to_string(), muted));
                }
                if let Some(required) = param.required {
                    let (text, color) = if required {
                        (t!("docs.required").to_string(), warning)
                    } else {
                        (t!("docs.optional").to_string(), muted)
                    };
                    row = row.child(fact(1, text, t!("docs.required").to_string(), color));
                }
                if !param.format.is_empty() {
                    row = row.child(fact(2, param.format.clone(), t!("docs.format").to_string(), muted));
                }
                if !param.schema_enum.is_empty() {
                    row = row.child(fact(3, param.enum_text(), t!("docs.enum").to_string(), muted));
                }
                row
            })
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

    fn render_inputs(&self, matched: Option<&HttpRequest>, editable: bool, cx: &mut Context<Self>) -> AnyElement {
        fn find_source_param<'a>(list: &'a [KvParam], key: &str, ignore_case: bool) -> Option<&'a KvParam> {
            list.iter().find(|kv| {
                if ignore_case {
                    kv.key.trim().eq_ignore_ascii_case(key.trim())
                } else {
                    kv.key.trim() == key.trim()
                }
            })
        }

        fn with_source_facts<'a>(own: &KvParam, source: Option<&'a KvParam>) -> (KvParam, Option<&'a str>) {
            let mut kv = own.clone();
            if let Some(src) = source {
                if kv.schema_type.is_empty() {
                    kv.schema_type = src.schema_type.clone();
                }
                if kv.required.is_none() {
                    kv.required = src.required;
                }
                if kv.format.is_empty() {
                    kv.format = src.format.clone();
                }
                if kv.schema_enum.is_empty() {
                    kv.schema_enum = src.schema_enum.clone();
                }
            }
            let fallback = source
                .map(|src| src.description.as_str())
                .filter(|desc| !desc.trim().is_empty());
            (kv, fallback)
        }

        let theme = cx.theme();
        let (border, muted, warning) = (theme.border, theme.muted_foreground, theme.warning);
        let request = self.api.read(cx).request().clone();

        let mut rows: Vec<AnyElement> = Vec::new();
        let mut flat_idx = 0usize;
        let path_names = cyberapi_core::model::path_param_names(&request.uri);
        for name in &path_names {
            let kv = request
                .variables
                .iter()
                .find(|kv| kv.key.trim() == name)
                .cloned()
                .unwrap_or_else(|| KvParam {
                    key: name.clone(),
                    enabled: true,
                    ..Default::default()
                });
            let source = matched.and_then(|m| find_source_param(&m.variables, name, false));
            let (kv, fallback) = with_source_facts(&kv, source);
            rows.push(Self::render_flat_param(flat_idx, "path", &kv, fallback, false, cx));
            flat_idx += 1;
        }
        for kv in request.query.iter().filter(|kv| !kv.key.trim().is_empty()) {
            let source = matched.and_then(|m| find_source_param(&m.query, &kv.key, false));
            let (kv, fallback) = with_source_facts(kv, source);
            rows.push(Self::render_flat_param(flat_idx, "query", &kv, fallback, false, cx));
            flat_idx += 1;
        }
        if let Some(m) = matched {
            for kv in m
                .query
                .iter()
                .filter(|kv| !kv.key.trim().is_empty() && (!kv.description.trim().is_empty() || kv.schema_recorded()))
            {
                if !request.query.iter().any(|own| own.key.trim() == kv.key.trim()) {
                    rows.push(Self::render_flat_param(flat_idx, "query", kv, None, true, cx));
                    flat_idx += 1;
                }
            }
        }
        for kv in request.headers.iter().filter(|kv| !kv.key.trim().is_empty()) {
            let source = matched.and_then(|m| find_source_param(&m.headers, &kv.key, true));
            let (kv, fallback) = with_source_facts(kv, source);
            rows.push(Self::render_flat_param(flat_idx, "header", &kv, fallback, false, cx));
            flat_idx += 1;
        }
        if let Some(m) = matched {
            for kv in m
                .headers
                .iter()
                .filter(|kv| !kv.key.trim().is_empty() && (!kv.description.trim().is_empty() || kv.schema_recorded()))
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
                let place = if multipart && send::is_file_value(&kv.value) {
                    "file"
                } else {
                    "form"
                };
                let fallback = find_source_param(&matched_form, &kv.key, false)
                    .map(|src| src.description.as_str())
                    .filter(|desc| !desc.trim().is_empty());
                let shown = shown_file(kv);
                rows.push(Self::render_flat_param(flat_idx, place, &shown, fallback, false, cx));
                flat_idx += 1;
            }
            for kv in matched_form
                .iter()
                .filter(|kv| !kv.key.trim().is_empty() && !kv.description.trim().is_empty())
            {
                if !own_form.iter().any(|own| own.key.trim() == kv.key.trim()) {
                    let place = if multipart && send::is_file_value(&kv.value) {
                        "file"
                    } else {
                        "form"
                    };
                    let shown = shown_file(kv);
                    rows.push(Self::render_flat_param(flat_idx, place, &shown, None, true, cx));
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
                    .when(editable, |this| {
                        this.child(
                            Button::new("docs-edit-request-schema")
                                .ghost()
                                .xsmall()
                                .label(t!("docs.edit_request_schema").to_string())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.edit_schema(true, window, cx);
                                })),
                        )
                    }),
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

    fn render_outputs(&self, matched: Option<&HttpRequest>, editable: bool, cx: &mut Context<Self>) -> AnyElement {
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
                    .when(editable, |this| {
                        this.child(
                            Button::new("docs-edit-response-schema")
                                .ghost()
                                .xsmall()
                                .label(t!("docs.edit_response_schema").to_string())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.edit_schema(false, window, cx);
                                })),
                        )
                    }),
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

    fn render_example_row(&self, at: usize, example: &Example, editable: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, mono_size) = (theme.border, theme.muted_foreground, theme.mono_font_size);
        let color = match status_level(example.status) {
            StatusLevel::Success => theme.success,
            StatusLevel::Redirect => theme.info,
            StatusLevel::ClientError => theme.warning,
            StatusLevel::ServerError => theme.danger,
        };
        let note = [
            example.content_type().map(str::to_string),
            Some(format_size(example.body.len() as u64)),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let shown = example_shown(&example.body);
        let clipped = example.body.len() > SHOWN;
        let full_body = example.body.clone();
        let body_empty = full_body.is_empty();
        let open_editor = self
            .open
            .as_ref()
            .filter(|open| open.index == at)
            .map(|open| open.editor.clone());
        let clipped_note = clipped.then(|| {
            div()
                .mt_1()
                .text_xs()
                .text_color(muted)
                .child(t!("docs.example_clipped", size = format_size(shown.len() as u64)).to_string())
        });
        let height = example_height(shown, mono_size);

        v_flex()
            .border_t_1()
            .border_color(border)
            .py_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        h_flex()
                            .id(("example", at))
                            .flex_1()
                            .flex_basis(px(0.))
                            .min_w_0()
                            .overflow_hidden()
                            .gap_3()
                            .items_center()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| this.toggle_example(at, window, cx)))
                            .child(
                                div()
                                    .w(px(36.))
                                    .flex_none()
                                    .font_semibold()
                                    .text_color(color)
                                    .child(example.status.to_string()),
                            )
                            .child(div().flex_1().min_w_0().truncate().child(example.name.clone()))
                            .child(
                                div()
                                    .max_w(relative(0.55))
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(note),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_none()
                            .items_center()
                            .child(
                                Button::new(("example-copy", at))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Copy)
                                    .tooltip(SharedString::from(t!("common.copy").to_string()))
                                    .disabled(body_empty)
                                    .on_click(move |_, window, cx| copy_example(full_body.clone(), window, cx)),
                            )
                            .when(editable, |this| {
                                this.child(
                                    Button::new(("example-remove", at))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Close)
                                        .tooltip(SharedString::from(t!("common.delete").to_string()))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.open = None;
                                            this.api.update(cx, |api, cx| api.remove_example(at, cx));
                                        })),
                                )
                            }),
                    ),
            )
            .when_some(open_editor, |this, editor| {
                this.children(clipped_note).child(
                    div()
                        .id(("example-body", at))
                        .mt_1()
                        .h(height)
                        .border_1()
                        .border_color(border)
                        .rounded_md()
                        .overflow_hidden()
                        .child(Editor::new(&editor).w_full().h_full().readonly(true).bordered(false)),
                )
            })
            .into_any_element()
    }

    fn render_examples(&self, editable: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let examples = self.api.read(cx).request().examples.clone();
        let rows: Vec<AnyElement> = examples
            .iter()
            .enumerate()
            .map(|(at, example)| self.render_example_row(at, example, editable, cx))
            .collect();
        let empty = rows.is_empty();
        v_flex()
            .pt_2()
            .gap_1p5()
            .child(div().text_sm().font_semibold().child(t!("docs.examples").to_string()))
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
            .into_any_element()
    }
}

impl Render for DocsEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let editable = self.editable(cx);
        let matched = self.matched_source(cx);
        let inputs = self.render_inputs(matched.as_ref(), editable, cx);
        let outputs = self.render_outputs(matched.as_ref(), editable, cx);
        let examples = self.render_examples(editable, cx);
        let desc_text = self.description.read(cx).value().to_string();
        let has_desc = !desc_text.trim().is_empty();
        let show_markdown = has_desc && (!self.editing_description || !editable);
        v_flex()
            .id("request-docs")
            .size_full()
            .overflow_y_scrollbar()
            .px_3()
            .py_2()
            .gap_2()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(t!("docs.description").to_string()),
                    )
                    .when(editable && has_desc, |this| {
                        let label = if show_markdown {
                            t!("common.edit").to_string()
                        } else {
                            t!("docs.preview").to_string()
                        };
                        this.child(
                            Button::new("docs-toggle-markdown")
                                .ghost()
                                .xsmall()
                                .label(label)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.editing_description = !this.editing_description;
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(if show_markdown {
                div()
                    .id("docs-markdown-scroll")
                    .max_h(px(240.))
                    .overflow_y_scrollbar()
                    .p_2()
                    .border_1()
                    .border_color(border)
                    .rounded_md()
                    .child(TextView::markdown("docs-markdown", desc_text))
                    .into_any_element()
            } else if editable {
                div()
                    .h(px(140.))
                    .border_1()
                    .border_color(border)
                    .rounded_md()
                    .child(Editor::new(&self.description).w_full().h_full().bordered(false))
                    .into_any_element()
            } else {
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("docs.placeholder").to_string())
                    .into_any_element()
            })
            .child(inputs)
            .child(outputs)
            .child(examples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Extras;
    use crate::states::{AppStore, WorkspaceState};
    use cyberapi_core::model::HttpMethod;
    use cyberapi_core::prefs::{AppPrefs, SendPrefs, Timeouts};
    use cyberapi_core::template::NoFiles;
    use cyberapi_http::{HttpEngine, PreparedRequest, SendFuture};
    use cyberapi_service::Sender;
    use cyberapi_store::{MemoryStore, NewCollection};
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AnyWindowHandle, TestAppContext, size};
    use std::sync::Arc;

    struct NoEngine;

    impl HttpEngine for NoEngine {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            Box::pin(std::future::pending())
        }
    }

    fn example_with(content_type: Option<&str>, body: &str) -> Example {
        Example {
            name: "sample".into(),
            status: 200,
            headers: content_type
                .map(|ct| vec![("Content-Type".into(), ct.into())])
                .unwrap_or_default(),
            body: body.into(),
        }
    }

    fn setup_docs_editor(
        request: HttpRequest,
        cx: &mut TestAppContext,
    ) -> (Entity<ApiState>, Entity<DocsEditor>, AnyWindowHandle) {
        cx.update(gpui_kit::init);
        let store = MemoryStore::new();
        let storage = store.storage();
        cx.update(|cx| {
            AppStore::init(
                storage.clone(),
                Arc::new(NoEngine),
                AppPrefs::default(),
                Extras::default(),
                cx,
            );
        });
        let collection = pollster::block_on(storage.collections().create(NewCollection {
            name: "p".into(),
            ..Default::default()
        }))
        .expect("create collection");
        let inserted = pollster::block_on(storage.tree().create_api(collection.id.clone(), None, "orders".into()))
            .expect("create api");
        let api_id = inserted.item.id;
        pollster::block_on(
            storage
                .tree()
                .save_request(collection.id.clone(), api_id.clone(), request),
        )
        .expect("save request");
        let sender = Sender {
            storage,
            engine: Arc::new(NoEngine),
            files: Arc::new(NoFiles("no files in tests")),
            env: Arc::new(cyberapi_core::template::NoEnv("no environment in tests")),
            user_agent: "CyberAPI/test".into(),
            timeouts: Timeouts::default(),
            sending: SendPrefs::default(),
            inherited: Default::default(),
            environment: None,
            tokens: Default::default(),
            secrets: Default::default(),
            runtime: Vec::new(),
        };
        let workspace = cx.new(|cx| WorkspaceState::new(sender, collection, cx));
        cx.run_until_parked();
        workspace.update(cx, |ws, cx| ws.select(&api_id, cx));
        cx.run_until_parked();
        let api = workspace.read_with(cx, |ws, _| ws.api().expect("selected api").clone());
        let mut docs = None;
        let window: AnyWindowHandle = cx
            .open_window(size(px(800.), px(600.)), |window, cx| {
                let view = cx.new(|cx| DocsEditor::new(api.clone(), window, cx));
                docs = Some(view.clone());
                Root::new(view, window, cx)
            })
            .into();
        cx.run_until_parked();
        (api, docs.expect("docs editor"), window)
    }

    #[test]
    fn example_language_detects_json_html_and_plain() {
        assert_eq!(
            example_language(&example_with(Some("application/json; charset=utf-8"), "{}")),
            "json"
        );
        assert_eq!(example_language(&example_with(Some("text/html"), "<p>hi</p>")), "html");
        assert_eq!(example_language(&example_with(Some("application/xml"), "<a/>")), "html");
        assert_eq!(example_language(&example_with(None, "{\"ok\":true}")), "json");
        assert_eq!(example_language(&example_with(None, " [1, 2]")), "json");

        let oversized_json = format!("{{\"data\":\"{}\"}}", "x".repeat(SHOWN + 10));
        assert_eq!(
            example_language(&example_with(Some("application/json"), &oversized_json)),
            "json"
        );
        assert_eq!(example_language(&example_with(None, &oversized_json)), "text");
        assert_eq!(example_language(&example_with(None, "pong")), "text");
    }

    #[test]
    fn example_height_scales_and_clamps() {
        let size = px(13.);
        assert_eq!(example_height("", size), px(60.));

        let five_lines = "a\nb\nc\nd\ne";
        let mid = example_height(five_lines, size);
        assert!(mid > px(60.) && mid < px(260.));

        let wrapped = "a".repeat(200);
        assert!(example_height(&wrapped, size) > px(60.));

        let fifty_lines = "line\n".repeat(50);
        assert_eq!(example_height(&fifty_lines, size), px(260.));
    }

    #[gpui_kit::test]
    async fn docs_editor_opens_syncs_and_copies_full_example(cx: &mut TestAppContext) {
        let json_body = "{\n    \"ok\": true\n}".to_string();
        let large_body = "x".repeat(SHOWN + 500);
        let request = HttpRequest {
            method: HttpMethod::Get,
            uri: "https://example.test/orders".into(),
            examples: vec![
                Example {
                    name: "ok".into(),
                    status: 200,
                    headers: vec![("content-type".into(), "application/json".into())],
                    body: json_body.clone(),
                },
                Example {
                    name: "large".into(),
                    status: 200,
                    headers: vec![("content-type".into(), "text/plain".into())],
                    body: large_body.clone(),
                },
            ],
            ..Default::default()
        };
        let (api, docs, window) = setup_docs_editor(request, cx);

        cx.update_window(window, |_, window, cx| {
            docs.update(cx, |docs, cx| docs.toggle_example(0, window, cx));
            window.render_frame(cx);
            let open = docs.read(cx).open.as_ref().expect("first example opened");
            assert_eq!(open.index, 0);
            assert_eq!(open.language, "json");
            assert_eq!(open.editor.read(cx).value().as_ref(), json_body);
        })
        .expect("open json example");

        cx.update_window(window, |_, window, cx| {
            docs.update(cx, |docs, cx| docs.toggle_example(1, window, cx));
            window.render_frame(cx);
            let open = docs.read(cx).open.as_ref().expect("large example opened");
            assert_eq!(open.index, 1);
            assert_eq!(open.language, "text");
            assert_eq!(open.editor.read(cx).value().len(), SHOWN);

            copy_example(large_body.clone(), window, cx);
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some(large_body.clone()),
            );

            docs.update(cx, |docs, cx| docs.toggle_example(1, window, cx));
            assert!(docs.read(cx).open.is_none());

            docs.update(cx, |docs, cx| docs.toggle_example(1, window, cx));
            assert!(docs.read(cx).open.is_some());
        })
        .expect("open and copy large example");

        api.update(cx, |api, cx| api.remove_example(1, cx));
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(docs.read(cx).open.is_none());

            docs.update(cx, |docs, cx| docs.toggle_example(0, window, cx));
            window.render_frame(cx);
            assert_eq!(docs.read(cx).open.as_ref().map(|open| open.index), Some(0));

            docs.update(cx, |docs, cx| docs.toggle_example(0, window, cx));
            assert!(docs.read(cx).open.is_none());
        })
        .expect("sync clears removed example");
    }
}
