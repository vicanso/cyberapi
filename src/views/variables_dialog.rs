//! A project's variables of one category, in a dialog (M8).

use crate::i18n;
use crate::states::{Load, VariablesEvent, VariablesState, WorkspaceState};
use cyberapi_core::VariableId;
use cyberapi_core::model::VariableCategory;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;
use std::collections::HashMap;

/// The title and what the category is for.
fn describe(category: VariableCategory) -> (String, String) {
    match category {
        VariableCategory::Environment => (t!("variables.env_title").into(), t!("variables.env_hint").into()),
        VariableCategory::Custom => (t!("variables.custom_title").into(), t!("variables.custom_hint").into()),
        VariableCategory::GlobalRequestHeader => (
            t!("variables.headers_title").into(),
            t!("variables.headers_hint").into(),
        ),
    }
}

pub fn open_variables_dialog(
    workspace: &Entity<WorkspaceState>,
    category: VariableCategory,
    window: &mut Window,
    cx: &mut App,
) {
    let (storage, collection, editable) = {
        let workspace = workspace.read(cx);
        (
            workspace.storage().clone(),
            workspace.collection().id.clone(),
            workspace.editable(),
        )
    };
    let weak = workspace.downgrade();
    let state = cx.new(|cx| VariablesState::new(storage, weak, collection, category, cx));
    let editor = cx.new(|cx| VariablesEditor::new(state, editable, window, cx));
    let (title, _) = describe(category);
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title(title.clone()).w(px(680.)).child(editor.clone())
    });
}

struct Row {
    name: Entity<InputState>,
    value: Entity<InputState>,
    _subscriptions: [Subscription; 2],
}

struct VariablesEditor {
    state: Entity<VariablesState>,
    /// A viewer reads them only (§9.2).
    editable: bool,
    rows: HashMap<VariableId, Row>,
    _subscriptions: Vec<Subscription>,
}

impl VariablesEditor {
    fn new(state: Entity<VariablesState>, editable: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| this.sync_rows(window, cx)),
            cx.subscribe_in(&state, window, |_, _, event, window, cx| match event {
                VariablesEvent::Failed(error) => {
                    window.push_notification(Notification::error(i18n::store_error(error)), cx);
                }
            }),
        ];
        Self {
            state,
            editable,
            rows: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    /// One click removes the row (#47). The request lists still ask twice
    /// (F4.10); waiting out that pause here left the row in place.
    fn remove(&mut self, id: VariableId, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.delete(&id, cx));
    }

    /// Inputs for variables that have none yet; a row keeps its inputs while
    /// it is typed in.
    fn sync_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let variables = self.state.read(cx).variables().to_vec();
        self.rows
            .retain(|id, _| variables.iter().any(|variable| &variable.id == id));
        for variable in variables {
            if self.rows.contains_key(&variable.id) {
                continue;
            }
            let name = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(t!("variables.name").to_string())
                    .default_value(variable.name.clone())
            });
            let value = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(t!("variables.value").to_string())
                    .default_value(variable.value.clone())
            });
            let (for_name, for_value) = (variable.id.clone(), variable.id.clone());
            let subscriptions = [
                cx.subscribe(&name, move |this, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let text = input.read(cx).value().to_string();
                        this.state.update(cx, |state, cx| state.set_name(&for_name, text, cx));
                    }
                }),
                cx.subscribe(&value, move |this, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let text = input.read(cx).value().to_string();
                        this.state.update(cx, |state, cx| state.set_value(&for_value, text, cx));
                    }
                }),
            ];
            self.rows.insert(
                variable.id.clone(),
                Row {
                    name,
                    value,
                    _subscriptions: subscriptions,
                },
            );
        }
        cx.notify();
    }
}

impl Render for VariablesEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let (_, hint) = describe(state.category());
        let editable = self.editable;
        let muted = cx.theme().muted_foreground;
        let body: gpui_kit::AnyElement =
            match state.load() {
                Load::Loading => h_flex()
                    .gap_2()
                    .text_sm()
                    .text_color(muted)
                    .child(Spinner::new().small())
                    .into_any_element(),
                _ if state.variables().is_empty() => div()
                    .py_4()
                    .text_sm()
                    .text_color(muted)
                    .child(t!("variables.empty").to_string())
                    .into_any_element(),
                _ => {
                    let theme = cx.theme();
                    let border = theme.border;
                    // The request's key / value table (`KvEditor`): fields in
                    // its cells without frames, the lines drawing the grid.
                    let cell = move || div().h_full().flex().items_center().border_l_1().border_color(border);
                    let actions = move || h_flex().w(px(32.)).flex_none().justify_end().pr_0p5();
                    let header = h_flex()
                        .h(px(26.))
                        .bg(theme.muted)
                        .text_xs()
                        .text_color(muted)
                        .child(div().w_8().flex_none())
                        .child(
                            cell()
                                .w(px(200.))
                                .flex_none()
                                .px_2()
                                .child(t!("variables.name").to_string()),
                        )
                        .child(
                            cell()
                                .flex_1()
                                .min_w_0()
                                .px_2()
                                .child(t!("variables.value").to_string()),
                        )
                        .when(editable, |this| this.child(actions()));
                    let rows =
                        state.variables().iter().enumerate().filter_map(|(index, variable)| {
                            let row = self.rows.get(&variable.id)?;
                            let (toggle_id, delete_id) = (variable.id.clone(), variable.id.clone());
                            Some(
                                h_flex()
                                    .id(("variable", index))
                                    .h(px(30.))
                                    .items_center()
                                    .border_t_1()
                                    .border_color(border)
                                    .child(
                                        div().w_8().flex_none().flex().justify_center().child(
                                            Checkbox::new(("variable-enabled", index))
                                                .checked(variable.enabled)
                                                .disabled(!editable)
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.state.update(cx, |state, cx| state.toggle(&toggle_id, cx));
                                                })),
                                        ),
                                    )
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .h_full()
                                            .when(!variable.enabled, |this| this.opacity(0.45))
                                            .child(cell().w(px(200.)).flex_none().child(
                                                Input::new(&row.name).small().appearance(false).disabled(!editable),
                                            ))
                                            .child(cell().flex_1().min_w_0().child(
                                                Input::new(&row.value).small().appearance(false).disabled(!editable),
                                            )),
                                    )
                                    .when(editable, |this| {
                                        this.child(
                                            actions().child(
                                                Button::new(("variable-delete", index))
                                                    .xsmall()
                                                    .ghost()
                                                    .icon(IconName::Close)
                                                    .tooltip(t!("common.delete").to_string())
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.remove(delete_id.clone(), cx);
                                                    })),
                                            ),
                                        )
                                    }),
                            )
                        });
                    v_flex()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(border)
                        .overflow_hidden()
                        .child(header)
                        .children(rows.collect::<Vec<_>>())
                        .into_any_element()
                }
            };
        v_flex()
            .gap_3()
            .child(div().text_sm().text_color(muted).child(hint))
            .child(div().id("variables").max_h(px(420.)).overflow_y_scrollbar().child(body))
            .child(if editable {
                h_flex()
                    .child(
                        Button::new("variable-add")
                            .outline()
                            .small()
                            .icon(IconName::Plus)
                            .label(t!("variables.add").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.state.update(cx, |state, cx| state.add(cx)))),
                    )
                    .into_any_element()
            } else {
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("variables.read_only").to_string())
                    .into_any_element()
            })
    }
}
