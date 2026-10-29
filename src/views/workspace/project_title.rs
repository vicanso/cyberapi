//! The project's name in the title bar, as the workspace has it — a server
//! project's draft may name it otherwise than the server — and its sync
//! (`docs/server-sync.md` §3.2, §3.3): while the reader has changes the
//! server has not, it says so, and that the server moved on too when it
//! did, beside the button that syncs them. No sync for a project of this
//! machine's, or for a viewer, who is told instead that changes are not
//! kept. While the server cannot be reached, that is said where the button
//! is, whether or not there are changes to sync.
//!
//! Beside it, the reader's environment (#61), to switch.

use crate::states::WorkspaceState;
use crate::views::open_variables_dialog;
use cyberapi_core::model::VariableCategory;
use gpui_kit::assets::IconName as Icons;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Sizable as _, StyledExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

pub struct ProjectTitle {
    state: Entity<WorkspaceState>,
    _subscription: Subscription,
}

impl ProjectTitle {
    pub fn new(state: Entity<WorkspaceState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            _subscription: subscription,
        }
    }
}

impl Render for ProjectTitle {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_none()
            .items_center()
            .gap_2()
            .child(self.title(cx))
            .child(self.environment_switch(cx))
    }
}

impl ProjectTitle {
    /// #61: the reader's environment, and the others to switch to; the
    /// variables' dialog, where they are made, from the same menu.
    fn environment_switch(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let current = state.current_environment().map(|environment| environment.id.clone());
        // D6: production stands out, and says so in the menu.
        let production = state.in_production();
        let named: Vec<(cyberapi_core::model::Environment, bool)> = state
            .named_environments()
            .iter()
            .map(|environment| (environment.clone(), state.protects(&environment.id)))
            .collect();
        let mut label = state
            .current_environment()
            .map(|environment| environment.name.clone())
            .unwrap_or_else(|| t!("environments.none").to_string());
        if production {
            label += &t!("environments.production_mark");
        }
        let workspace = self.state.downgrade();
        Button::new("environment-switch")
            .when(production, |button| button.danger())
            .when(!production, |button| button.ghost())
            .xsmall()
            .icon(Icons::Globe)
            .label(label)
            .tooltip(if production {
                t!("environments.production_tooltip").to_string()
            } else {
                t!("environments.switch_tooltip").to_string()
            })
            .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                let pick = |id| {
                    let workspace = workspace.clone();
                    move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                        let _ = workspace.update(cx, |state, cx| state.set_environment(Option::clone(&id), cx));
                    }
                };
                let menu = menu.item(
                    PopupMenuItem::new(t!("environments.none").to_string())
                        .checked(current.is_none())
                        .on_click(pick(None)),
                );
                let menu = if named.is_empty() {
                    menu
                } else {
                    named.iter().fold(menu.separator(), |menu, (environment, production)| {
                        let mut name = environment.name.clone();
                        if *production {
                            name += &t!("environments.production_mark");
                        }
                        menu.item(
                            PopupMenuItem::new(name)
                                .checked(current.as_ref() == Some(&environment.id))
                                .on_click(pick(Some(environment.id.clone()))),
                        )
                    })
                };
                let manage = workspace.clone();
                menu.separator().item(
                    PopupMenuItem::new(t!("environments.manage").to_string())
                        .icon(Icons::Globe)
                        .on_click(move |_, window, cx| {
                            if let Some(workspace) = manage.upgrade() {
                                open_variables_dialog(&workspace, VariableCategory::Environment, window, cx);
                            }
                        }),
                )
            })
            .into_any_element()
    }

    fn title(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let unreachable = state
            .unreachable()
            .map(|reason| crate::views::unreachable_note("sync-unreachable".into(), reason, cx));
        let name = div()
            .flex_none()
            .max_w(px(240.))
            .truncate()
            .text_sm()
            .font_medium()
            .child(state.collection().name.clone());
        // A viewer may change a request to send it, but nothing is kept:
        // said beside the name, where it stays in sight.
        if !state.editable() {
            let hint = t!("workspace.viewer_hint").to_string();
            return h_flex()
                .flex_none()
                .items_center()
                .child(name.mr_2())
                .child(
                    div()
                        .id("viewer-status")
                        .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                        .child(
                            Tag::secondary()
                                .small()
                                .rounded_full()
                                .child(t!("roles.viewer").to_string()),
                        ),
                )
                .children(unreachable.map(|note| div().ml_2().child(note)))
                .into_any_element();
        }
        // A long name gives way to the pins beside it.
        if !state.unsynced() {
            return match unreachable {
                Some(note) => h_flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .child(name)
                    .child(note)
                    .into_any_element(),
                None => name.into_any_element(),
            };
        }
        let (label, hint) = if state.server_moved() {
            (t!("sync.unsynced_behind"), t!("sync.unsynced_behind_hint"))
        } else {
            (t!("sync.unsynced"), t!("sync.unsynced_hint"))
        };
        let hint = hint.to_string();
        let syncing = state.syncing();
        let state = self.state.clone();
        h_flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(name.mr_1())
            .child(
                div()
                    .id("sync-status")
                    .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                    .child(crate::views::unsynced_tag().child(label.to_string())),
            )
            // Out of reach, there is nothing to sync with.
            .child(match unreachable {
                Some(note) => div().ml_1().child(note).into_any_element(),
                None => Button::new("sync-push")
                    .ghost()
                    .xsmall()
                    .icon(gpui_kit::assets::IconName::CloudUpload)
                    .label(t!("sync.sync").to_string())
                    .loading(syncing)
                    .on_click(move |_, _, cx| state.update(cx, |state, cx| state.sync(false, cx)))
                    .into_any_element(),
            })
            .into_any_element()
    }
}
