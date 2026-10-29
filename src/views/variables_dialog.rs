//! A project's variables of one category, in a dialog (M8), a layer at a
//! time (#61): the shared ones, or a named environment's, which stand in
//! for the shared ones of the same name. The environments are made,
//! renamed and deleted here, in the dialog itself (a dialog over a dialog
//! does not show in the page).

use crate::i18n;
use crate::states::{Load, VariablesEvent, VariablesState, WorkspaceState};
use crate::views::inline_confirm::inline_confirm;
use cyberapi_core::model::VariableCategory;
use cyberapi_core::{EnvironmentId, VariableId};
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
    let (storage, collection, editable, layer) = {
        let workspace = workspace.read(cx);
        (
            workspace.storage().clone(),
            workspace.collection().id.clone(),
            workspace.editable(),
            // The reader's environment's layer first (#61).
            workspace
                .current_environment()
                .map(|environment| environment.id.clone()),
        )
    };
    let weak = workspace.downgrade();
    let state = cx.new(|cx| VariablesState::new(storage, weak, collection, category, layer, cx));
    let workspace = workspace.clone();
    let editor = cx.new(|cx| VariablesEditor::new(state, workspace, editable, window, cx));
    let (title, _) = describe(category);
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(title.clone())
            .w(px(680.))
            .child(crate::views::dialog_body(editor.clone()))
            // No button to confirm: Enter is a field's — a new environment's
            // name (#61) — not the dialog's to close.
            .on_ok(|_, _, _| false)
    });
}

struct Row {
    name: Entity<InputState>,
    value: Entity<InputState>,
    /// D3: made for a secret variable: masked, the reader's own value.
    secret: bool,
    _subscriptions: [Subscription; 2],
}

/// An environment's name being typed: a new one's, or `renaming`'s.
struct Naming {
    input: Entity<InputState>,
    renaming: Option<EnvironmentId>,
    _subscription: Subscription,
}

struct VariablesEditor {
    state: Entity<VariablesState>,
    /// D6: whose settings say which environments are production.
    workspace: Entity<WorkspaceState>,
    /// A viewer reads them only (§9.2).
    editable: bool,
    rows: HashMap<VariableId, Row>,
    naming: Option<Naming>,
    /// The environment asked about before it goes, with its variables.
    deleting: Option<EnvironmentId>,
    _subscriptions: Vec<Subscription>,
}

impl VariablesEditor {
    fn new(
        state: Entity<VariablesState>,
        workspace: Entity<WorkspaceState>,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| this.sync_rows(window, cx)),
            // D3: which are secret is the project's settings.
            cx.observe_in(&workspace, window, |this, _, window, cx| this.sync_rows(window, cx)),
            cx.subscribe_in(&state, window, |_, _, event, window, cx| match event {
                VariablesEvent::Failed(error) => {
                    window.push_notification(Notification::error(i18n::store_error(error)), cx);
                }
            }),
        ];
        Self {
            state,
            workspace,
            editable,
            rows: HashMap::new(),
            naming: None,
            deleting: None,
            _subscriptions: subscriptions,
        }
    }

    /// #61: a name for a new environment, or for `renaming`.
    fn start_naming(&mut self, renaming: Option<EnvironmentId>, window: &mut Window, cx: &mut Context<Self>) {
        let current = renaming.as_ref().and_then(|id| {
            self.state
                .read(cx)
                .environments()
                .iter()
                .find(|environment| &environment.id == id)
                .map(|environment| environment.name.clone())
        });
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("variables.environment_name").to_string())
                .default_value(current.unwrap_or_default())
        });
        let subscription = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.finish_naming(window, cx);
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.deleting = None;
        self.naming = Some(Naming {
            input,
            renaming,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn finish_naming(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(naming) = self.naming.take() else { return };
        let name = naming.input.read(cx).value().trim().to_string();
        if !name.is_empty() {
            self.state.update(cx, |state, cx| match naming.renaming {
                Some(id) => state.rename_environment(id, name, cx),
                None => state.create_environment(name, cx),
            });
        }
        cx.notify();
    }

    /// The layers: the shared one, each environment, and — for an editor —
    /// a new one; the one shown, renamed or deleted under them.
    fn render_layers(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let state = self.state.read(cx);
        let layer = state.layer().cloned();
        let muted = cx.theme().muted_foreground;
        let button = |id: gpui_kit::ElementId, label: String, chosen: Option<EnvironmentId>| {
            let selected = layer == chosen;
            Button::new(id)
                .small()
                .when(selected, |button| button.primary())
                .when(!selected, |button| button.ghost())
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.naming = None;
                    this.deleting = None;
                    let chosen = chosen.clone();
                    this.state.update(cx, |state, cx| state.set_layer(chosen, cx));
                }))
        };
        let mut buttons = vec![button(
            "variables-layer-shared".into(),
            t!("variables.shared").to_string(),
            None,
        )];
        for (index, environment) in state.environments().iter().enumerate() {
            let mut name = environment.name.clone();
            if self.workspace.read(cx).protects(&environment.id) {
                name += &t!("environments.production_mark");
            }
            buttons.push(button(
                ("variables-layer", index).into(),
                name,
                Some(environment.id.clone()),
            ));
        }
        let shown = layer
            .as_ref()
            .and_then(|id| state.environments().iter().find(|environment| &environment.id == id))
            .cloned();
        let editable = self.editable;
        let bar = h_flex()
            .gap_1()
            .flex_wrap()
            .items_center()
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .mr_1()
                    .child(t!("variables.layer").to_string()),
            )
            .children(buttons)
            .when(editable && self.naming.is_none(), |bar| {
                bar.child(
                    Button::new("environment-new")
                        .ghost()
                        .small()
                        .icon(IconName::Plus)
                        .label(t!("variables.new_environment").to_string())
                        .on_click(cx.listener(|this, _, window, cx| this.start_naming(None, window, cx))),
                )
            });
        let below: Option<gpui_kit::AnyElement> = if let Some(naming) = &self.naming {
            Some(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().w(px(240.)).child(Input::new(&naming.input).small()))
                    .child(
                        Button::new("environment-name-ok")
                            .primary()
                            .small()
                            .label(if naming.renaming.is_some() {
                                t!("variables.rename_environment").to_string()
                            } else {
                                t!("common.create").to_string()
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.finish_naming(window, cx))),
                    )
                    .child(
                        Button::new("environment-name-cancel")
                            .ghost()
                            .small()
                            .label(t!("common.cancel").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.naming = None;
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            )
        } else if let (Some(environment), Some(deleting)) = (&shown, &self.deleting) {
            let (view, keep) = (cx.entity().downgrade(), cx.entity().downgrade());
            let id = deleting.clone();
            Some(
                inline_confirm(
                    t!("variables.delete_environment_title", name = environment.name).to_string(),
                    t!("variables.delete_environment_hint").to_string(),
                    None,
                    t!("common.delete").to_string(),
                    move |_, cx| {
                        let _ = keep.update(cx, |this, cx| {
                            this.deleting = None;
                            cx.notify();
                        });
                    },
                    move |_, cx| {
                        let id = id.clone();
                        let _ = view.update(cx, |this, cx| {
                            this.deleting = None;
                            this.state.update(cx, |state, cx| state.delete_environment(id, cx));
                        });
                    },
                    cx,
                )
                .into_any_element(),
            )
        } else if let (Some(environment), true) = (&shown, editable) {
            let (rename, delete) = (environment.id.clone(), environment.id.clone());
            let (production, marked) = (
                environment.id.clone(),
                self.workspace.read(cx).protects(&environment.id),
            );
            Some(
                h_flex()
                    .gap_1()
                    .items_center()
                    // D6.
                    .child(
                        Checkbox::new("environment-production")
                            .checked(marked)
                            .label(t!("environments.production").to_string())
                            .mr_2()
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                let (id, on) = (production.clone(), *checked);
                                this.workspace
                                    .update(cx, |workspace, cx| workspace.set_production(id, on, cx));
                            })),
                    )
                    .child(
                        Button::new("environment-rename")
                            .ghost()
                            .xsmall()
                            .icon(gpui_kit::assets::IconName::Pencil)
                            .label(t!("variables.rename_environment").to_string())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_naming(Some(rename.clone()), window, cx)
                            })),
                    )
                    .child(
                        Button::new("environment-delete")
                            .ghost()
                            .xsmall()
                            .icon(gpui_kit::assets::IconName::Trash)
                            .label(t!("variables.delete_environment").to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.deleting = Some(delete.clone());
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            )
        } else {
            None
        };
        v_flex().gap_2().child(bar).children(below).into_any_element()
    }

    /// One click removes the row (#47). The request lists still ask twice
    /// (F4.10); waiting out that pause here left the row in place.
    fn remove(&mut self, id: VariableId, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.delete(&id, cx));
    }

    /// Inputs for variables that have none yet; a row keeps its inputs while
    /// it is typed in, and is made again when it turns secret or back (D3).
    fn sync_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let variables = self.state.read(cx).variables().to_vec();
        self.rows
            .retain(|id, _| variables.iter().any(|variable| &variable.id == id));
        for variable in variables {
            let secret = self.state.read(cx).is_secret(&variable.id, cx);
            if self.rows.get(&variable.id).is_some_and(|row| row.secret == secret) {
                continue;
            }
            let shown = self.state.read(cx).shown_value(&variable, cx);
            let name = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(t!("variables.name").to_string())
                    .default_value(variable.name.clone())
            });
            let value = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(match secret {
                        true => t!("variables.secret_value").to_string(),
                        false => t!("variables.value").to_string(),
                    })
                    .masked(secret)
                    .default_value(shown)
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
                    secret,
                    _subscriptions: subscriptions,
                },
            );
        }
        cx.notify();
    }
}

impl Render for VariablesEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layers = self.render_layers(cx);
        let state = self.state.read(cx);
        let (_, hint) = describe(state.category());
        // #61: what the layer shown is for.
        let layer_hint = state
            .layer()
            .and_then(|id| state.environments().iter().find(|environment| &environment.id == id))
            .map(|environment| t!("variables.environment_hint", name = environment.name).to_string())
            .unwrap_or_else(|| t!("variables.shared_hint").to_string());
        let stood_in_for: Vec<(String, String)> = state
            .stood_in_for()
            .into_iter()
            .map(|variable| {
                let value = match state.is_secret(&variable.id, cx) {
                    true => cyberapi_core::secrets::MASK.to_string(),
                    false => variable.value.clone(),
                };
                (variable.name.clone(), value)
            })
            .collect();
        let any_secret = state.shown().any(|variable| state.is_secret(&variable.id, cx));
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
                _ if state.shown().next().is_none() => div()
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
                    let actions = move || h_flex().w(px(56.)).flex_none().justify_end().pr_0p5();
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
                        state.shown().enumerate().filter_map(|(index, variable)| {
                            let row = self.rows.get(&variable.id)?;
                            let (toggle_id, delete_id) = (variable.id.clone(), variable.id.clone());
                            let (secret, secret_id) = (row.secret, variable.id.clone());
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
                                            // D3: a viewer types their own secret value too.
                                            .child(
                                                cell().flex_1().min_w_0().child(
                                                    Input::new(&row.value)
                                                        .small()
                                                        .appearance(false)
                                                        .disabled(!editable && !secret)
                                                        .when(secret, |input| input.mask_toggle()),
                                                ),
                                            ),
                                    )
                                    .when(editable, |this| {
                                        this.child(
                                            actions()
                                                .child(
                                                    Button::new(("variable-secret", index))
                                                        .xsmall()
                                                        .ghost()
                                                        .icon(match secret {
                                                            true => gpui_kit::assets::IconName::Lock,
                                                            false => gpui_kit::assets::IconName::LockOpen,
                                                        })
                                                        .tooltip(match secret {
                                                            true => t!("variables.unmake_secret").to_string(),
                                                            false => t!("variables.make_secret").to_string(),
                                                        })
                                                        .on_click(cx.listener(move |this, _, _, cx| {
                                                            this.state.update(cx, |state, cx| {
                                                                state.set_secret(&secret_id, !secret, cx)
                                                            });
                                                        })),
                                                )
                                                .child(
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
        // The shared ones this environment stands in for, to compare.
        let compared = (!stood_in_for.is_empty()).then(|| {
            v_flex()
                .gap_1()
                .text_xs()
                .text_color(muted)
                .child(t!("variables.stood_in_for").to_string())
                .children(stood_in_for.into_iter().map(|(name, value)| {
                    // In the table's columns, past its checkbox.
                    h_flex()
                        .pl_8()
                        .opacity(0.7)
                        .child(div().w(px(200.)).flex_none().px_2().truncate().child(name))
                        .child(div().flex_1().min_w_0().px_2().truncate().child(value))
                }))
        });
        v_flex()
            .gap_3()
            .child(layers)
            .child(
                v_flex()
                    .gap_0p5()
                    .text_sm()
                    .text_color(muted)
                    .child(hint)
                    .child(div().text_xs().child(layer_hint)),
            )
            .child(div().id("variables").max_h(px(420.)).overflow_y_scrollbar().child(body))
            // D3: where a secret's value is kept.
            .when(any_secret, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("variables.secret_hint").to_string()),
                )
            })
            .children(compared)
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
