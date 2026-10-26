//! A GraphQL endpoint's schema to read (#70): its types on the left, the
//! operation types first, found by name; the type picked on the right —
//! its fields with their arguments and types, input fields, enum values,
//! members, and what each is for. A type named in a field goes to that
//! type; a field's name goes into the query, where its caret was.

use cyberapi_core::graphql_schema::{InputDef, Schema, TypeDef};
use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{EditorState, Input, InputEvent, InputState, RopeExt as _};
use gpui_kit::component::scroll::{ScrollableElement as _, Scrollbar, ScrollbarAxis};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, UniformListScrollHandle, Window,
    div, px, uniform_list,
};
use rust_i18n::t;
use std::sync::Arc;

const ROW_HEIGHT: f32 = 28.;

/// Opens the browser over `schema`; a field picked goes into `query`.
/// `fetch` asks the endpoint for its schema again.
pub fn open_schema_browser(
    schema: Arc<Schema>,
    query: Entity<EditorState>,
    fetch: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let browser = cx.new(|cx| SchemaBrowser::new(schema, query, window, cx));
    let fetch = std::rc::Rc::new(fetch);
    window.open_dialog(cx, move |dialog, _, _| {
        let fetch = fetch.clone();
        dialog
            .title(t!("editor.schema_title").to_string())
            .w(px(820.))
            .on_ok(|_, _, _| false)
            .child(browser.clone())
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("schema-fetch")
                            .outline()
                            .label(t!("editor.schema_fetch_again").to_string())
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                fetch(window, cx);
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("schema-close")
                            .label(t!("common.close").to_string())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    });
}

struct SchemaBrowser {
    schema: Arc<Schema>,
    query: Entity<EditorState>,
    search: Entity<InputState>,
    /// The types listed: those the search finds.
    names: Vec<String>,
    picked: Option<String>,
    list: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl SchemaBrowser {
    fn new(schema: Arc<Schema>, query: Entity<EditorState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("editor.schema_search").to_string())
                .clean_on_escape()
        });
        let searching = cx.subscribe(&search, |this: &mut Self, search, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                let filter = search.read(cx).value().to_string();
                this.names = this
                    .schema
                    .browsed_types(&filter)
                    .into_iter()
                    .map(str::to_string)
                    .collect();
                cx.notify();
            }
        });
        let names: Vec<String> = schema.browsed_types("").into_iter().map(str::to_string).collect();
        let picked = names.first().cloned();
        Self {
            schema,
            query,
            search,
            names,
            picked,
            list: UniformListScrollHandle::new(),
            _subscriptions: vec![searching],
        }
    }

    fn pick(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.schema.types.contains_key(name) {
            self.picked = Some(name.to_string());
            cx.notify();
        }
    }

    /// The field's name, into the query: at its caret in a selection set,
    /// or else last in the set around it, or in the operation (#75).
    fn insert(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |editor, cx| {
            let query = editor.value().to_string();
            let (at, text) = cyberapi_core::graphql_format::field_insertion(&query, editor.cursor(), name);
            let position = editor.text().offset_to_position(at);
            editor.set_cursor_position(position, window, cx);
            editor.insert(text, window, cx);
        });
        window.close_dialog(cx);
    }

    fn row(&self, index: usize, name: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let picked = self.picked.as_deref() == Some(name);
        let kind = self
            .schema
            .types
            .get(name)
            .map(|def| kind_label(&def.kind))
            .unwrap_or_default();
        let owned = name.to_string();
        h_flex()
            .id(("schema-type", index))
            .w_full()
            .h(px(ROW_HEIGHT))
            .px_2()
            .gap_2()
            .items_center()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(picked, |this| this.bg(theme.accent))
            .hover(|this| this.bg(theme.accent))
            .child(div().flex_1().min_w_0().truncate().text_sm().child(name.to_string()))
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(kind),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.pick(&owned, cx)))
            .into_any_element()
    }

    /// A type as written (`[User!]!`), the type under it a link.
    fn type_link(&self, id: SharedString, display: &str, named: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let known = self.schema.types.contains_key(named);
        let named_owned = named.to_string();
        div()
            .id(id)
            .text_color(theme.blue)
            .when(known, |this| {
                this.cursor_pointer()
                    .hover(|this| this.underline())
                    .on_click(cx.listener(move |this, _, _, cx| this.pick(&named_owned, cx)))
            })
            .child(display.to_string())
            .into_any_element()
    }

    fn arguments(&self, args: &[InputDef]) -> String {
        if args.is_empty() {
            return String::new();
        }
        let args: Vec<String> = args
            .iter()
            .map(|arg| format!("{}: {}", arg.name, arg.type_ref.display))
            .collect();
        format!("({})", args.join(", "))
    }

    fn render_type(&self, name: &str, def: &TypeDef, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, accent, mono) = (theme.muted_foreground, theme.link, theme.mono_font_family.clone());
        let description =
            |text: &str| (!text.is_empty()).then(|| div().text_xs().text_color(muted).child(text.to_string()));
        let section = |title: String| div().pt_2().text_xs().font_semibold().text_color(muted).child(title);
        let mut body = v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_baseline()
                    .child(div().text_lg().font_semibold().child(name.to_string()))
                    .child(div().text_xs().text_color(muted).child(kind_label(&def.kind))),
            )
            .children(description(&def.description));
        if !def.fields.is_empty() {
            body = body.child(section(t!("editor.schema_fields").to_string()));
            for (index, field) in def.fields.iter().enumerate() {
                let field_name = field.name.clone();
                let type_link = self.type_link(
                    format!("schema-field-type-{index}").into(),
                    &field.type_ref.display,
                    &field.type_ref.named,
                    cx,
                );
                body =
                    body.child(
                        v_flex()
                            .py_0p5()
                            .child(
                                h_flex()
                                    .flex_wrap()
                                    .items_baseline()
                                    .font_family(mono.clone())
                                    .text_sm()
                                    .child(
                                        div()
                                            .id(("schema-field", index))
                                            .text_color(accent)
                                            .cursor_pointer()
                                            .hover(|this| this.underline())
                                            .tooltip(|window, cx| {
                                                Tooltip::new(t!("editor.schema_insert").to_string()).build(window, cx)
                                            })
                                            .child(field.name.clone())
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.insert(&field_name, window, cx)
                                            })),
                                    )
                                    .child(div().text_color(muted).child(self.arguments(&field.args)))
                                    .child(div().text_color(muted).child(":\u{a0}"))
                                    .child(type_link),
                            )
                            .children(description(&field.description)),
                    );
            }
        }
        if !def.input_fields.is_empty() {
            body = body.child(section(t!("editor.schema_input_fields").to_string()));
            for (index, input) in def.input_fields.iter().enumerate() {
                let type_link = self.type_link(
                    format!("schema-input-type-{index}").into(),
                    &input.type_ref.display,
                    &input.type_ref.named,
                    cx,
                );
                body = body.child(
                    v_flex()
                        .py_0p5()
                        .child(
                            h_flex()
                                .items_baseline()
                                .font_family(mono.clone())
                                .text_sm()
                                .child(input.name.clone())
                                .child(div().text_color(muted).child(":\u{a0}"))
                                .child(type_link),
                        )
                        .children(description(&input.description)),
                );
            }
        }
        if !def.enum_values.is_empty() {
            body = body.child(section(t!("editor.schema_values").to_string()));
            for (value, about) in &def.enum_values {
                body = body.child(
                    v_flex()
                        .py_0p5()
                        .child(div().font_family(mono.clone()).text_sm().child(value.clone()))
                        .children(description(about)),
                );
            }
        }
        if !def.possible_types.is_empty() {
            body = body.child(section(t!("editor.schema_members").to_string()));
            for (index, member) in def.possible_types.iter().enumerate() {
                body = body.child(self.type_link(format!("schema-member-{index}").into(), member, member, cx));
            }
        }
        body.into_any_element()
    }
}

/// A type's kind, as GraphQL's SDL writes it.
fn kind_label(kind: &str) -> String {
    match kind {
        "OBJECT" => "type",
        "INTERFACE" => "interface",
        "UNION" => "union",
        "ENUM" => "enum",
        "INPUT_OBJECT" => "input",
        "SCALAR" => "scalar",
        other => other,
    }
    .to_string()
}

impl Render for SchemaBrowser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let border = theme.border;
        let muted = theme.muted_foreground;
        let count = self.names.len();
        let list = uniform_list(
            "schema-types",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let names: Vec<String> = this.names[range.clone()].to_vec();
                names
                    .iter()
                    .zip(range)
                    .map(|(name, index)| this.row(index, name, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.list)
        .size_full();
        let detail = match self
            .picked
            .clone()
            .and_then(|name| Some((self.schema.types.get(&name)?.clone(), name)))
        {
            Some((def, name)) => self.render_type(&name, &def, cx),
            None => div()
                .text_sm()
                .text_color(muted)
                .child(t!("editor.schema_none").to_string())
                .into_any_element(),
        };
        h_flex()
            .h(px(480.))
            .gap_3()
            .child(
                v_flex()
                    .w(px(240.))
                    .flex_none()
                    .h_full()
                    .gap_2()
                    .pr_3()
                    .border_r_1()
                    .border_color(border)
                    .child(
                        Input::new(&self.search)
                            .small()
                            .cleanable(true)
                            .prefix(Icon::new(IconName::Search).small().text_color(muted)),
                    )
                    .child(
                        div().flex_1().min_h_0().relative().child(list).child(
                            div()
                                .absolute()
                                .inset_0()
                                .child(Scrollbar::new(&self.list).axis(ScrollbarAxis::Vertical)),
                        ),
                    ),
            )
            .child(
                div()
                    .id("schema-detail")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scrollbar()
                    .child(detail),
            )
    }
}
