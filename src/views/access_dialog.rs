//! Who has a project (browser build, §9.1–9.3), for its managers: the team
//! it belongs to — moved to another of the reader's — and the accounts and
//! teams it is also shared with, each with a role.

use crate::i18n;
use crate::states::{AppStore, DirectoryDone, DirectoryEvent, DirectoryState};
use crate::views::inline_confirm::inline_confirm;
use crate::views::pick::pick;
use cyberapi_core::access::{CollectionRole, Grantee};
use cyberapi_core::directory::Grant;
use cyberapi_core::model::Collection;
use cyberapi_core::{CollectionId, TeamId};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;
use std::rc::Rc;

const ROLES: [CollectionRole; 3] = [CollectionRole::Viewer, CollectionRole::Editor, CollectionRole::Manager];

fn roles() -> Vec<(CollectionRole, String)> {
    ROLES
        .into_iter()
        .map(|role| (role, i18n::collection_role(role)))
        .collect()
}

/// `on_moved` runs once the project belongs to another team.
pub fn open_access_dialog(
    collection: Collection,
    on_moved: impl Fn(&mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(directory) = AppStore::global(cx).directory(cx) else {
        return;
    };
    let state = cx.new(|_| DirectoryState::new(directory));
    let title = t!("access.title", name = collection.name).to_string();
    let editor = cx.new(|cx| AccessEditor::new(state, collection, Rc::new(on_moved), window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(title.clone())
            .w(px(640.))
            .child(editor.clone())
            // Enter is a field's own, not the dialog's to close on.
            .on_ok(|_, _, _| false)
    });
}

struct AccessEditor {
    state: Entity<DirectoryState>,
    collection: Collection,
    add: Option<Grantee>,
    add_role: CollectionRole,
    /// The team the reader chose to move it to, not yet confirmed.
    moving: Option<(TeamId, String)>,
    _subscriptions: Vec<Subscription>,
}

impl AccessEditor {
    fn new(
        state: Entity<DirectoryState>,
        collection: Collection,
        on_moved: Rc<dyn Fn(&mut App)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let id = collection.id.clone();
        state.update(cx, |state, cx| {
            state.reload_me(cx);
            state.reload_accounts(cx);
            state.load_grants(id, cx);
        });
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe_in(&state, window, move |_, _, event, window, cx| match event {
                DirectoryEvent::Done(DirectoryDone::Transferred) => {
                    // The reader's own role may have changed with it.
                    window.close_dialog(cx);
                    window.push_notification(Notification::success(t!("access.moved").to_string()), cx);
                    on_moved(cx);
                }
                DirectoryEvent::Done(_) => {}
                DirectoryEvent::Failed(error) => {
                    window.push_notification(Notification::error(i18n::store_error(error)), cx);
                }
            }),
        ];
        Self {
            state,
            collection,
            add: None,
            add_role: CollectionRole::Viewer,
            moving: None,
            _subscriptions: subscriptions,
        }
    }

    fn id(&self) -> CollectionId {
        self.collection.id.clone()
    }

    /// The move to another team, asked about before it is made.
    fn render_moving(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (team, team_name) = self.moving.clone()?;
        let (cancelled, confirmed) = (cx.entity().downgrade(), cx.entity().downgrade());
        Some(
            inline_confirm(
                t!("access.move_title", project = self.collection.name, team = team_name).to_string(),
                t!("access.move_description", team = team_name).to_string(),
                None,
                t!("access.move").to_string(),
                move |_, cx| {
                    let _ = cancelled.update(cx, |this, cx| {
                        this.moving = None;
                        cx.notify();
                    });
                },
                move |_, cx| {
                    let team = team.clone();
                    let _ = confirmed.update(cx, |this, cx| {
                        this.moving = None;
                        let collection = this.id();
                        this.state.update(cx, |state, cx| state.transfer(collection, team, cx));
                    });
                },
                cx,
            )
            .into_any_element(),
        )
    }

    fn render_grant(&self, index: usize, grant: &Grant, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let kind = match grant.grantee {
            Grantee::Account(_) => t!("access.account"),
            Grantee::Team(_) => t!("access.team"),
        };
        let (state, collection, grantee) = (self.state.clone(), self.id(), grant.grantee.clone());
        let (removed_state, removed_collection, removed) = (state.clone(), collection.clone(), grantee.clone());
        h_flex()
            .id(("grant", index))
            .gap_2()
            .py_1()
            .items_center()
            .text_sm()
            .child(div().flex_1().min_w_0().truncate().child(grant.name.clone()))
            .child(div().w(px(60.)).text_xs().text_color(muted).child(kind.to_string()))
            .child(pick(
                ("grant-role", index),
                i18n::collection_role(grant.role),
                roles(),
                Some(grant.role),
                false,
                move |role, _, cx| {
                    let (collection, grantee) = (collection.clone(), grantee.clone());
                    state.update(cx, |state, cx| state.set_grant(collection, grantee, Some(role), cx));
                },
            ))
            .child(
                Button::new(("grant-remove", index))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(t!("common.delete").to_string())
                    .on_click(move |_, _, cx| {
                        let (collection, grantee) = (removed_collection.clone(), removed.clone());
                        removed_state.update(cx, |state, cx| state.set_grant(collection, grantee, None, cx));
                    }),
            )
    }
}

impl Render for AccessEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let state = self.state.read(cx);
        let teams = state.me().map(|me| me.teams.clone()).unwrap_or_default();
        let me = state.me().map(|me| me.account.id.clone());
        let grants: Vec<Grant> = state
            .grants(&self.collection.id)
            .map(<[Grant]>::to_vec)
            .unwrap_or_default();
        let owner = self.collection.owner.clone();
        let owner_name = teams
            .iter()
            .find(|team| Some(&team.id) == owner.as_ref())
            .map(|team| team.name.clone())
            .unwrap_or_else(|| t!("access.other_team").to_string());
        let team_options: Vec<(TeamId, String)> =
            teams.iter().map(|team| (team.id.clone(), team.name.clone())).collect();
        // Whom it can still be shared with: not its own team, not the
        // reader, not who has it already.
        let shared = |grantee: &Grantee| grants.iter().any(|grant| &grant.grantee == grantee);
        let mut candidates: Vec<(Grantee, String)> = teams
            .iter()
            .filter(|team| Some(&team.id) != owner.as_ref() && !team.personal)
            .map(|team| {
                (
                    Grantee::Team(team.id.clone()),
                    t!("access.team_named", name = team.name).to_string(),
                )
            })
            .filter(|(grantee, _)| !shared(grantee))
            .collect();
        candidates.extend(
            state
                .accounts()
                .iter()
                .filter(|account| !account.disabled && Some(&account.id) != me.as_ref())
                .map(|account| {
                    (
                        Grantee::Account(account.id.clone()),
                        format!("{} ({})", account.label(), account.name),
                    )
                })
                .filter(|(grantee, _)| !shared(grantee)),
        );
        let chosen = self
            .add
            .as_ref()
            .and_then(|add| candidates.iter().find(|(grantee, _)| grantee == add))
            .map(|(_, label)| label.clone());
        let view = cx.entity().downgrade();
        let names: Vec<(TeamId, String)> = team_options.clone();
        let current_owner = owner.clone();
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_sm().w(px(110.)).child(t!("access.belongs_to").to_string()))
                    .child(pick("access-owner", owner_name, team_options, owner.clone(), false, {
                        let view = view.clone();
                        move |team: TeamId, _, cx| {
                            if Some(&team) == current_owner.as_ref() {
                                return;
                            }
                            let name = names
                                .iter()
                                .find(|(id, _)| id == &team)
                                .map(|(_, name)| name.clone())
                                .unwrap_or_default();
                            let _ = view.update(cx, |this, cx| {
                                this.moving = Some((team, name));
                                cx.notify();
                            });
                        }
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("access.owner_hint").to_string()),
            )
            .children(self.render_moving(cx))
            .child(
                div()
                    .pt_2()
                    .border_t_1()
                    .border_color(border)
                    .text_xs()
                    .text_color(muted)
                    .child(t!("access.shared_with").to_string()),
            )
            .child(
                v_flex()
                    .id("grants")
                    .max_h(px(240.))
                    .overflow_y_scrollbar()
                    .when(grants.is_empty(), |this| {
                        this.child(
                            div()
                                .py_2()
                                .text_sm()
                                .text_color(muted)
                                .child(t!("access.none").to_string()),
                        )
                    })
                    .children(
                        grants
                            .iter()
                            .enumerate()
                            .map(|(index, grant)| self.render_grant(index, grant, cx).into_any_element()),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(pick(
                        "grant-add-who",
                        chosen.unwrap_or_else(|| t!("access.pick").to_string()),
                        candidates,
                        self.add.clone(),
                        false,
                        {
                            let view = view.clone();
                            move |grantee, _, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.add = Some(grantee);
                                    cx.notify();
                                });
                            }
                        },
                    ))
                    .child(pick(
                        "grant-add-role",
                        i18n::collection_role(self.add_role),
                        roles(),
                        Some(self.add_role),
                        false,
                        move |role, _, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.add_role = role;
                                cx.notify();
                            });
                        },
                    ))
                    .child(div().flex_1())
                    .child(
                        Button::new("grant-add")
                            .primary()
                            .small()
                            .label(t!("access.share").to_string())
                            .disabled(self.add.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(grantee) = this.add.take() {
                                    let (collection, role) = (this.id(), this.add_role);
                                    this.state
                                        .update(cx, |state, cx| state.set_grant(collection, grantee, Some(role), cx));
                                }
                            })),
                    ),
            )
    }
}
