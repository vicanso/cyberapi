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
            .w(px(780.))
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

/// D8: one cell in the multi-environment matrix view (`(environment, name)`).
struct MatrixCell {
    input: Entity<InputState>,
    variable_id: Option<VariableId>,
    placeholder: String,
    secret: bool,
    creating: bool,
    _subscription: Subscription,
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
    /// D8: side-by-side matrix mode across Shared and all named environments.
    matrix: bool,
    matrix_cells: HashMap<(Option<EnvironmentId>, String), MatrixCell>,
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
            matrix: false,
            matrix_cells: HashMap::new(),
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
        let has_environments = !state.environments().is_empty();
        let matrix = self.matrix && has_environments;
        let duplicates = state.duplicate_count();
        let muted = cx.theme().muted_foreground;
        let button = |id: gpui_kit::ElementId, label: String, chosen: Option<EnvironmentId>| {
            let selected = !matrix && layer == chosen;
            Button::new(id)
                .small()
                .when(selected, |button| button.primary())
                .when(!selected, |button| button.ghost())
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.matrix = false;
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
        let shown = (!matrix)
            .then(|| {
                layer
                    .as_ref()
                    .and_then(|id| state.environments().iter().find(|environment| &environment.id == id))
                    .cloned()
            })
            .flatten();
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
            .when(has_environments, |bar| {
                bar.child(
                    Button::new("variables-matrix")
                        .small()
                        .when(matrix, |button| button.primary())
                        .when(!matrix, |button| button.ghost())
                        .icon(gpui_kit::assets::IconName::Table)
                        .label(t!("variables.matrix").to_string())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.matrix = !this.matrix;
                            this.naming = None;
                            this.deleting = None;
                            this.sync_rows(window, cx);
                        })),
                )
            })
            .when(editable && self.naming.is_none(), |bar| {
                bar.child(
                    Button::new("environment-new")
                        .ghost()
                        .small()
                        .icon(IconName::Plus)
                        .label(t!("variables.new_environment").to_string())
                        .on_click(cx.listener(|this, _, window, cx| this.start_naming(None, window, cx))),
                )
            })
            .when(editable && duplicates > 0, |bar| {
                bar.child(
                    Button::new("variables-deduplicate")
                        .ghost()
                        .small()
                        .icon(gpui_kit::assets::IconName::WandSparkles)
                        .label(t!("variables.deduplicate", count = duplicates).to_string())
                        .tooltip(t!("variables.deduplicate_about").to_string())
                        .on_click(cx.listener(|this, _, window, cx| {
                            let removed = this.state.update(cx, |state, cx| state.deduplicate(cx));
                            if removed > 0 {
                                window.push_notification(
                                    Notification::success(t!("variables.deduplicated", count = removed).to_string()),
                                    cx,
                                );
                            }
                        })),
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
        self.sync_matrix_cells(window, cx);
        cx.notify();
    }

    /// D8: keeps input fields for `(layer, variable_name)` cells in the matrix
    /// view without recreating active inputs while typing.
    fn sync_matrix_cells(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (category, variables, environments) = {
            let state = self.state.read(cx);
            (
                state.category(),
                state.variables().to_vec(),
                state.environments().to_vec(),
            )
        };
        if environments.is_empty() {
            self.matrix = false;
            self.matrix_cells.clear();
            return;
        }
        let names = cyberapi_core::environments::matrix_names(&variables, category);
        let layers: Vec<Option<EnvironmentId>> = std::iter::once(None)
            .chain(environments.iter().map(|env| Some(env.id.clone())))
            .collect();
        self.matrix_cells
            .retain(|(env, key), _| names.iter().any(|name| name == key) && layers.iter().any(|layer| layer == env));
        for name in &names {
            let shared_var = cyberapi_core::environments::layer_variable(&variables, category, None, name);
            let shared_shown = match shared_var.filter(|v| v.enabled) {
                Some(v) if self.state.read(cx).is_secret(&v.id, cx) => cyberapi_core::secrets::MASK.to_string(),
                Some(v) => v.value.clone(),
                None => String::new(),
            };
            for layer in &layers {
                let var = cyberapi_core::environments::layer_variable(&variables, category, layer.as_ref(), name);
                let var_id = var.map(|v| v.id.clone());
                let secret = var.is_some_and(|v| self.state.read(cx).is_secret(&v.id, cx));
                let shown = var.map(|v| self.state.read(cx).shown_value(v, cx)).unwrap_or_default();
                let placeholder = if secret {
                    t!("variables.secret_value").to_string()
                } else if layer.is_some() && !shared_shown.is_empty() {
                    format!("← {shared_shown}")
                } else {
                    t!("variables.matrix_missing").to_string()
                };
                let cell_key = (layer.clone(), name.clone());
                if let Some(cell) = self.matrix_cells.get_mut(&cell_key)
                    && cell.secret == secret
                {
                    if cell.creating
                        && let Some(new_id) = var_id.clone()
                    {
                        cell.creating = false;
                        cell.variable_id = Some(new_id.clone());
                        let typed = cell.input.read(cx).value().to_string();
                        if typed != shown {
                            self.state.update(cx, |state, cx| state.set_value(&new_id, typed, cx));
                        }
                    } else if cell.variable_id != var_id {
                        cell.variable_id = var_id;
                        cell.input
                            .update(cx, |input, cx| input.set_value(shown.clone(), window, cx));
                    }
                    if cell.placeholder != placeholder {
                        cell.placeholder = placeholder.clone();
                        cell.input
                            .update(cx, |input, cx| input.set_placeholder(placeholder, window, cx));
                    }
                    continue;
                }
                let input_placeholder = placeholder.clone();
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(input_placeholder)
                        .masked(secret)
                        .default_value(shown)
                });
                let (cell_layer, cell_name) = (layer.clone(), name.clone());
                let subscription = cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let text = input.read(cx).value().to_string();
                    let existing = {
                        let state = this.state.read(cx);
                        cyberapi_core::environments::layer_variable(
                            state.variables(),
                            category,
                            cell_layer.as_ref(),
                            &cell_name,
                        )
                        .map(|v| (v.id.clone(), v.enabled))
                    };
                    if let Some((id, enabled)) = existing {
                        this.state.update(cx, |state, cx| {
                            if !enabled && !text.is_empty() {
                                state.toggle(&id, cx);
                            }
                            state.set_value(&id, text, cx);
                        });
                    } else if !text.is_empty()
                        && let Some(cell) = this.matrix_cells.get_mut(&(cell_layer.clone(), cell_name.clone()))
                        && !cell.creating
                    {
                        cell.creating = true;
                        let (env, key) = (cell_layer.clone(), cell_name.clone());
                        this.state.update(cx, |state, cx| state.create_at(env, key, text, cx));
                    }
                });
                self.matrix_cells.insert(
                    cell_key,
                    MatrixCell {
                        input,
                        variable_id: var_id,
                        placeholder,
                        secret,
                        creating: false,
                        _subscription: subscription,
                    },
                );
            }
        }
    }

    /// D8: renders the side-by-side matrix table across Shared and all named environments.
    fn render_matrix(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let state = self.state.read(cx);
        let category = state.category();
        let variables = state.variables();
        let environments = state.environments();
        let names = cyberapi_core::environments::matrix_names(variables, category);
        let theme = cx.theme();
        let (border, muted, warning) = (theme.border, theme.muted_foreground, theme.warning);
        if names.is_empty() {
            return div()
                .py_4()
                .text_sm()
                .text_color(muted)
                .child(t!("variables.empty").to_string())
                .into_any_element();
        }
        let editable = self.editable;
        let cell = move || {
            div()
                .h_full()
                .flex_1()
                .min_w(px(140.))
                .flex()
                .items_center()
                .border_l_1()
                .border_color(border)
        };
        let mut header = h_flex()
            .h(px(28.))
            .bg(theme.muted)
            .text_xs()
            .text_color(muted)
            .child(
                div()
                    .w(px(160.))
                    .flex_none()
                    .h_full()
                    .flex()
                    .items_center()
                    .px_2()
                    .child(t!("variables.name").to_string()),
            )
            .child(cell().px_2().child(t!("variables.shared").to_string()));
        for environment in environments {
            let mut label = environment.name.clone();
            if self.workspace.read(cx).protects(&environment.id) {
                label += &t!("environments.production_mark");
            }
            header = header.child(cell().px_2().child(div().truncate().child(label)));
        }

        let layers: Vec<Option<EnvironmentId>> = std::iter::once(None)
            .chain(environments.iter().map(|env| Some(env.id.clone())))
            .collect();
        let rows: Vec<gpui_kit::AnyElement> = names
            .iter()
            .enumerate()
            .map(|(row_idx, name)| {
                let shared_var = cyberapi_core::environments::layer_variable(variables, category, None, name);
                let shared_has_value =
                    shared_var.is_some_and(|v| v.enabled && (!v.value.is_empty() || state.is_secret(&v.id, cx)));
                let mut row = h_flex()
                    .id(("matrix-row", row_idx))
                    .h(px(30.))
                    .items_center()
                    .border_t_1()
                    .border_color(border)
                    .child(
                        div()
                            .w(px(160.))
                            .flex_none()
                            .h_full()
                            .flex()
                            .items_center()
                            .px_2()
                            .text_xs()
                            .font_family(theme.mono_font_family.clone())
                            .child(div().truncate().child(name.clone())),
                    );
                for layer in &layers {
                    let var = cyberapi_core::environments::layer_variable(variables, category, layer.as_ref(), name);
                    let own_has_value =
                        var.is_some_and(|v| v.enabled && (!v.value.is_empty() || state.is_secret(&v.id, cx)));
                    let missing = !own_has_value && (layer.is_none() || !shared_has_value);
                    let cell_elem = if let Some(matrix_cell) = self.matrix_cells.get(&(layer.clone(), name.clone())) {
                        let secret = matrix_cell.secret;
                        cell().when(missing, |this| this.bg(warning.opacity(0.08))).child(
                            Input::new(&matrix_cell.input)
                                .small()
                                .appearance(false)
                                .disabled(!editable && !secret)
                                .when(secret, |input| input.mask_toggle()),
                        )
                    } else {
                        cell()
                    };
                    row = row.child(cell_elem);
                }
                row.into_any_element()
            })
            .collect();

        div()
            .id("variables-matrix-scroll")
            .w_full()
            .overflow_x_scrollbar()
            .rounded(theme.radius)
            .border_1()
            .border_color(border)
            .child(v_flex().min_w_full().child(header).children(rows))
            .into_any_element()
    }
}

impl Render for VariablesEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layers = self.render_layers(cx);
        let matrix = self.matrix && !self.state.read(cx).environments().is_empty();
        let (hint, layer_hint, stood_in_for, any_secret) = {
            let state = self.state.read(cx);
            let (_, hint) = describe(state.category());
            // #61: what the layer shown is for.
            let layer_hint = if matrix {
                t!("variables.matrix_hint").to_string()
            } else {
                state
                    .layer()
                    .and_then(|id| state.environments().iter().find(|environment| &environment.id == id))
                    .map(|environment| t!("variables.environment_hint", name = environment.name).to_string())
                    .unwrap_or_else(|| t!("variables.shared_hint").to_string())
            };
            let stood_in_for: Vec<(String, String)> = if matrix {
                Vec::new()
            } else {
                state
                    .stood_in_for()
                    .into_iter()
                    .map(|variable| {
                        let value = match state.is_secret(&variable.id, cx) {
                            true => cyberapi_core::secrets::MASK.to_string(),
                            false => variable.value.clone(),
                        };
                        (variable.name.clone(), value)
                    })
                    .collect()
            };
            let any_secret = if matrix {
                state
                    .variables()
                    .iter()
                    .any(|variable| state.is_secret(&variable.id, cx))
            } else {
                state.shown().any(|variable| state.is_secret(&variable.id, cx))
            };
            (hint, layer_hint, stood_in_for, any_secret)
        };
        let editable = self.editable;
        let muted = cx.theme().muted_foreground;
        let body: gpui_kit::AnyElement =
            match self.state.read(cx).load() {
                Load::Loading => h_flex()
                    .gap_2()
                    .text_sm()
                    .text_color(muted)
                    .child(Spinner::new().small())
                    .into_any_element(),
                _ if matrix => self.render_matrix(cx),
                _ if self.state.read(cx).shown().next().is_none() => div()
                    .py_4()
                    .text_sm()
                    .text_color(muted)
                    .child(t!("variables.empty").to_string())
                    .into_any_element(),
                _ => {
                    let state = self.state.read(cx);
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
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.matrix {
                                    this.matrix = false;
                                    this.state.update(cx, |state, cx| {
                                        state.set_layer(None, cx);
                                        state.add(cx);
                                    });
                                } else {
                                    this.state.update(cx, |state, cx| state.add(cx));
                                }
                            })),
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
