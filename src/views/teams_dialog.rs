//! The reader's teams (`docs/architecture.md` §9.1, browser build): make
//! one, rename, delete or leave it, and choose who is in it and as what —
//! an account there is already, or someone invited to make one
//! (`docs/server-sync.md` §5). What the reader may not change is shown, not
//! offered.

use crate::i18n;
use crate::states::{AppStore, DirectoryDone, DirectoryEvent, DirectoryState};
use crate::views::inline_confirm::inline_confirm;
use crate::views::invites::{copy_link, invites_section};
use crate::views::pick::pick;
use cyberapi_core::access::TeamRole;
use cyberapi_core::directory::{Member, Team, TeamPlace};
use cyberapi_core::{AccountId, TeamId};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;
use std::rc::Rc;

const ROLES: [TeamRole; 3] = [TeamRole::Member, TeamRole::Admin, TeamRole::Owner];

pub fn open_teams_dialog(window: &mut Window, cx: &mut App) {
    let Some(directory) = AppStore::global(cx).directory(cx) else {
        return;
    };
    let state = cx.new(|_| DirectoryState::new(directory));
    let editor = cx.new(|cx| TeamsEditor::new(state, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(t!("teams.title").to_string())
            .w(px(820.))
            .child(editor.clone())
            // Enter is a field's own, not the dialog's to close on.
            .on_ok(|_, _, _| false)
    });
}

/// The roles `mine` may give: an owner gives any, an admin not owner.
fn roles_given_by(mine: TeamRole) -> Vec<(TeamRole, String)> {
    ROLES
        .into_iter()
        .filter(|role| mine == TeamRole::Owner || *role != TeamRole::Owner)
        .map(|role| (role, i18n::team_role(role)))
        .collect()
}

/// Whether `mine` may change or remove a member who is `theirs`.
fn may_change(mine: TeamRole, theirs: TeamRole) -> bool {
    mine >= TeamRole::Admin && (theirs != TeamRole::Owner || mine == TeamRole::Owner)
}

/// What a confirmed change does.
type TeamChange = Rc<dyn Fn(&mut DirectoryState, &mut Context<DirectoryState>)>;

/// A change to a team waiting for the reader to confirm it.
struct Pending {
    team: TeamId,
    title: String,
    description: String,
    ok: String,
    then: TeamChange,
}

struct TeamsEditor {
    state: Entity<DirectoryState>,
    selected: Option<TeamId>,
    pending: Option<Pending>,
    new_team: Entity<InputState>,
    /// The selected team's name, where the reader may change it, and
    /// which team it is the name of.
    name: Option<Entity<InputState>>,
    name_for: Option<TeamId>,
    add_account: Option<AccountId>,
    add_role: TeamRole,
    _name_subscription: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl TeamsEditor {
    fn new(state: Entity<DirectoryState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        state.update(cx, |state, cx| {
            state.reload_me(cx);
            state.reload_accounts(cx);
            state.load_invites(cx);
        });
        let new_team = cx.new(|cx| InputState::new(window, cx).placeholder(t!("teams.new_placeholder").to_string()));
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, state, window, cx| {
                // The first team once they are read, or again when the
                // selected one is gone.
                let teams: Vec<Team> = state.read(cx).me().map(|me| me.teams.clone()).unwrap_or_default();
                let gone = this
                    .selected
                    .as_ref()
                    .is_none_or(|selected| !teams.iter().any(|team| &team.id == selected));
                if gone && let Some(first) = teams.first() {
                    this.select(first.id.clone(), window, cx);
                }
                // A team just made is in the list only now.
                this.sync_name(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(&state, window, |this, _, event, window, cx| match event {
                DirectoryEvent::Done(DirectoryDone::TeamCreated(team)) => {
                    this.new_team.update(cx, |input, cx| input.set_value("", window, cx));
                    this.select(team.clone(), window, cx);
                }
                DirectoryEvent::Done(DirectoryDone::InviteCreated(link)) => copy_link(link, window, cx),
                DirectoryEvent::Done(_) => {}
                DirectoryEvent::Failed(error) => {
                    window.push_notification(Notification::error(i18n::store_error(error)), cx);
                }
            }),
            cx.subscribe_in(&new_team, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.create(window, cx);
                }
            }),
        ];
        Self {
            state,
            selected: None,
            pending: None,
            new_team,
            name: None,
            name_for: None,
            add_account: None,
            add_role: TeamRole::Member,
            _name_subscription: None,
            _subscriptions: subscriptions,
        }
    }

    fn team(&self, cx: &App) -> Option<Team> {
        let selected = self.selected.as_ref()?;
        let me = self.state.read(cx).me()?;
        me.teams.iter().find(|team| &team.id == selected).cloned()
    }

    fn select(&mut self, id: TeamId, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(id.clone());
        self.pending = None;
        self.add_account = None;
        self.add_role = TeamRole::Member;
        self.sync_name(window, cx);
        self.state.update(cx, |state, cx| state.load_members(id, cx));
        cx.notify();
    }

    /// The name is a field only where the reader may change it.
    fn sync_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let team = self
            .team(cx)
            .filter(|team| !team.personal && team.role >= TeamRole::Admin);
        let wanted = team.as_ref().map(|team| team.id.clone());
        if self.name_for == wanted {
            return;
        }
        self.name_for = wanted;
        self.name = None;
        self._name_subscription = None;
        if let Some(team) = team {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(team.name.clone()));
            let renamed = team.id.clone();
            self._name_subscription = Some(cx.subscribe_in(&input, window, move |this, input, event, _, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    let name = input.read(cx).value().trim().to_string();
                    let current = this.team(cx).map(|team| team.name).unwrap_or_default();
                    if !name.is_empty() && name != current {
                        let team = renamed.clone();
                        this.state.update(cx, |state, cx| state.rename_team(team, name, cx));
                    }
                }
            }));
            self.name = Some(input);
        }
    }

    fn create(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let name = self.new_team.read(cx).value().trim().to_string();
        if !name.is_empty() {
            self.state.update(cx, |state, cx| state.create_team(name, cx));
        }
    }

    /// Asks in the right column before `then` changes `team`.
    fn confirm(
        &mut self,
        team: TeamId,
        title: String,
        description: String,
        ok: String,
        cx: &mut Context<Self>,
        then: impl Fn(&mut DirectoryState, &mut Context<DirectoryState>) + 'static,
    ) {
        self.pending = Some(Pending {
            team,
            title,
            description,
            ok,
            then: Rc::new(then),
        });
        cx.notify();
    }

    fn render_pending(&self, team: &TeamId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pending = self.pending.as_ref().filter(|pending| &pending.team == team)?;
        let (cancelled, confirmed) = (cx.entity().downgrade(), cx.entity().downgrade());
        Some(
            inline_confirm(
                pending.title.clone(),
                pending.description.clone(),
                None,
                pending.ok.clone(),
                move |_, cx| {
                    let _ = cancelled.update(cx, |this, cx| {
                        this.pending = None;
                        cx.notify();
                    });
                },
                move |_, cx| {
                    let _ = confirmed.update(cx, |this, cx| {
                        if let Some(pending) = this.pending.take() {
                            this.state.update(cx, |state, cx| (pending.then)(state, cx));
                        }
                        cx.notify();
                    });
                },
                cx,
            )
            .into_any_element(),
        )
    }

    fn render_teams(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, active) = (theme.muted_foreground, theme.accent);
        let teams: Vec<Team> = self.state.read(cx).me().map(|me| me.teams.clone()).unwrap_or_default();
        let selected = self.selected.clone();
        v_flex()
            .w(px(240.))
            .flex_none()
            .gap_2()
            .child(
                v_flex()
                    .id("teams")
                    .h(px(340.))
                    .overflow_y_scrollbar()
                    .gap_0p5()
                    .children(teams.into_iter().enumerate().map(|(index, team)| {
                        let id = team.id.clone();
                        let role = if team.personal {
                            t!("teams.personal").to_string()
                        } else {
                            i18n::team_role(team.role)
                        };
                        h_flex()
                            .id(("team", index))
                            .px_2()
                            .py_1p5()
                            .gap_2()
                            .rounded(cx.theme().radius)
                            .cursor_pointer()
                            .when(selected.as_ref() == Some(&team.id), |this| this.bg(active))
                            .hover(|style| style.bg(active))
                            .on_click(cx.listener(move |this, _, window, cx| this.select(id.clone(), window, cx)))
                            .child(div().flex_1().min_w_0().truncate().text_sm().child(team.name.clone()))
                            .child(div().text_xs().text_color(muted).child(role))
                    })),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(div().flex_1().child(Input::new(&self.new_team).small()))
                    .child(
                        Button::new("team-create")
                            .primary()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip(t!("teams.create").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.create(window, cx))),
                    ),
            )
    }

    fn render_member(&self, index: usize, member: &Member, team: &Team, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let me = self.state.read(cx).me().map(|me| me.account.id.clone());
        let mine = member.account.id == me.clone().unwrap_or_default();
        let changeable = !team.personal && !mine && may_change(team.role, member.role);
        let (state, team_id, account) = (self.state.clone(), team.id.clone(), member.account.id.clone());
        let (removed_state, removed_team, removed_account) = (state.clone(), team_id.clone(), account.clone());
        h_flex()
            .id(("member", index))
            .gap_2()
            .py_1()
            .items_center()
            .text_sm()
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1p5()
                    .child(div().truncate().child(member.account.label().to_string()))
                    .child(div().text_xs().text_color(muted).child(member.account.name.clone()))
                    .when(mine, |this| {
                        this.child(div().text_xs().text_color(muted).child(t!("teams.you").to_string()))
                    }),
            )
            .child(pick(
                ("member-role", index),
                i18n::team_role(member.role),
                roles_given_by(team.role),
                Some(member.role),
                !changeable,
                move |role, _, cx| {
                    let (team, account) = (team_id.clone(), account.clone());
                    state.update(cx, |state, cx| state.set_member(team, account, Some(role), cx));
                },
            ))
            .child(
                Button::new(("member-remove", index))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(t!("teams.remove").to_string())
                    .disabled(!changeable)
                    .on_click(move |_, _, cx| {
                        let (team, account) = (removed_team.clone(), removed_account.clone());
                        removed_state.update(cx, |state, cx| state.set_member(team, account, None, cx));
                    }),
            )
            .into_any_element()
    }

    fn render_team(&self, team: Team, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let state = self.state.read(cx);
        let members: Vec<Member> = state.members(&team.id).map(<[Member]>::to_vec).unwrap_or_default();
        let candidates: Vec<(AccountId, String)> = state
            .accounts()
            .iter()
            .filter(|account| !account.disabled && !members.iter().any(|member| member.account.id == account.id))
            .map(|account| (account.id.clone(), format!("{} ({})", account.label(), account.name)))
            .collect();
        let chosen = self
            .add_account
            .as_ref()
            .and_then(|id| candidates.iter().find(|(candidate, _)| candidate == id))
            .map(|(_, label)| label.clone());
        let adds = !team.personal && team.role >= TeamRole::Admin;
        let title: AnyElement = match &self.name {
            Some(input) => div().flex_1().child(Input::new(input).small()).into_any_element(),
            None => div()
                .flex_1()
                .font_semibold()
                .truncate()
                .child(team.name.clone())
                .into_any_element(),
        };
        let (leave_name, delete_name) = (team.name.clone(), team.name.clone());
        let (leave_team, delete_team) = (team.id.clone(), team.id.clone());
        let me = state.me().map(|me| me.account.id.clone()).unwrap_or_default();
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(title)
                    .when(!team.personal, |this| {
                        this.child(
                            Button::new("team-leave")
                                .outline()
                                .xsmall()
                                .label(t!("teams.leave").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let (team, me) = (leave_team.clone(), me.clone());
                                    this.confirm(
                                        team.clone(),
                                        t!("teams.leave_title", name = leave_name).to_string(),
                                        t!("teams.leave_description").to_string(),
                                        t!("teams.leave").to_string(),
                                        cx,
                                        move |state, cx| state.set_member(team.clone(), me.clone(), None, cx),
                                    );
                                })),
                        )
                    })
                    .when(!team.personal && team.role == TeamRole::Owner, |this| {
                        this.child(
                            Button::new("team-delete")
                                .outline()
                                .xsmall()
                                .label(t!("teams.delete").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let team = delete_team.clone();
                                    this.confirm(
                                        team.clone(),
                                        t!("teams.delete_title", name = delete_name).to_string(),
                                        t!("teams.delete_description").to_string(),
                                        t!("common.delete").to_string(),
                                        cx,
                                        move |state, cx| state.delete_team(team.clone(), cx),
                                    );
                                })),
                        )
                    }),
            )
            .children(self.render_pending(&team.id, cx))
            .when(team.personal, |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(t!("teams.personal_hint").to_string()),
                )
            })
            .child(div().text_xs().text_color(muted).child(t!("teams.members").to_string()))
            .child(
                v_flex().id("members").h(px(250.)).overflow_y_scrollbar().children(
                    members
                        .iter()
                        .enumerate()
                        .map(|(index, member)| self.render_member(index, member, &team, cx)),
                ),
            )
            .when(adds, |this| {
                let add_team = team.id.clone();
                this.child(
                    h_flex()
                        .gap_2()
                        .pt_2()
                        .border_t_1()
                        .border_color(border)
                        .items_center()
                        .child(pick(
                            "member-add-account",
                            chosen.unwrap_or_else(|| t!("teams.pick_account").to_string()),
                            candidates,
                            self.add_account.clone(),
                            false,
                            {
                                let view = cx.entity().downgrade();
                                move |account, _, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.add_account = Some(account);
                                        cx.notify();
                                    });
                                }
                            },
                        ))
                        .child(pick(
                            "member-add-role",
                            i18n::team_role(self.add_role),
                            roles_given_by(team.role),
                            Some(self.add_role),
                            false,
                            {
                                let view = cx.entity().downgrade();
                                move |role, _, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.add_role = role;
                                        cx.notify();
                                    });
                                }
                            },
                        ))
                        .child(div().flex_1())
                        .child({
                            let invite_team = team.id.clone();
                            Button::new("member-invite")
                                .outline()
                                .small()
                                .label(t!("teams.invite").to_string())
                                .tooltip(t!("teams.invite_hint").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let place = TeamPlace {
                                        team: invite_team.clone(),
                                        role: this.add_role,
                                    };
                                    this.state.update(cx, |state, cx| state.create_invite(Some(place), cx));
                                }))
                        })
                        .child(
                            Button::new("member-add")
                                .primary()
                                .small()
                                .label(t!("teams.add_member").to_string())
                                .disabled(self.add_account.is_none())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(account) = this.add_account.take() {
                                        let (team, role) = (add_team.clone(), this.add_role);
                                        this.state
                                            .update(cx, |state, cx| state.set_member(team, account, Some(role), cx));
                                    }
                                })),
                        ),
                )
            })
            .when_some(invites_section(&self.state, Some(&team.id), cx), |this, invites| {
                this.child(invites)
            })
            .into_any_element()
    }
}

impl Render for TeamsEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let team = self.team(cx);
        let right: AnyElement = match team {
            Some(team) => self.render_team(team, cx),
            None => div()
                .flex_1()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(SharedString::from(t!("teams.none").to_string()))
                .into_any_element(),
        };
        h_flex()
            .gap_4()
            .items_start()
            .child(self.render_teams(cx))
            .child(div().w(px(1.)).h(px(380.)).bg(border))
            .child(right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admins_do_not_touch_owners() {
        assert!(may_change(TeamRole::Owner, TeamRole::Owner));
        assert!(may_change(TeamRole::Admin, TeamRole::Member));
        assert!(!may_change(TeamRole::Admin, TeamRole::Owner));
        assert!(!may_change(TeamRole::Member, TeamRole::Member));
        assert_eq!(roles_given_by(TeamRole::Admin).len(), 2);
        assert_eq!(roles_given_by(TeamRole::Owner).len(), 3);
    }
}
