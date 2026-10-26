//! F3: an API's Docs tab — what it is for, as plain text (Markdown shows
//! as written), and the answers kept as examples of what it returns, each
//! opened to read its body. Both go with the project; an example is kept
//! from the response panel's history menu.

use crate::states::ApiState;
use cyberapi_core::response::{StatusLevel, format_size, status_level};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

/// The most of an example's body shown.
const SHOWN: usize = 64 * 1024;

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
        let subscriptions = vec![
            cx.observe(&api, |_, _, cx| cx.notify()),
            cx.subscribe(&description, |this, editor, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = editor.read(cx).value().to_string();
                    this.api.update(cx, |api, cx| api.set_description(text, cx));
                }
            }),
        ];
        Self {
            api,
            description,
            open: None,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for DocsEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, mono) = (theme.border, theme.muted_foreground, theme.mono_font_family.clone());
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
                    .h(px(160.))
                    .border_1()
                    .border_color(border)
                    .rounded_md()
                    .child(Editor::new(&self.description).w_full().h_full().bordered(false)),
            )
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
