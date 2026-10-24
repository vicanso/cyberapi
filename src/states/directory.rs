//! The browser build's accounts and teams (`docs/architecture.md` §9.4), as
//! their dialogs show and change them. Each dialog makes one and reads into
//! it what it shows: who is signed in, every account, a team's members, a
//! project's grants.
//!
//! A change to the reader's teams reads them again here and in
//! [`AppState`](super::AppState), whose copy the menu and the project list
//! show.

use super::{AppStore, run_store};
use crate::this_build;
use cyberapi_core::access::{CollectionRole, Grantee, TeamRole};
use cyberapi_core::directory::{AccountInfo, Grant, Invite, McpToken, Me, Member, NewMcpToken, TeamPlace};
use cyberapi_core::{AccountId, CollectionId, InviteId, McpTokenId, TeamId};
use cyberapi_store::{AccountEdit, DirectoryStore, StoreError};
use gpui_kit::{Context, EventEmitter};
use std::collections::HashMap;
use std::sync::Arc;

/// A change that went through, for the dialog to say so or move on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectoryDone {
    TeamCreated(TeamId),
    TeamDeleted,
    /// The reader left the team.
    TeamLeft,
    /// An invitation's link, to copy (`docs/server-sync.md` §5).
    InviteCreated(String),
    PasswordReset,
    PasswordChanged,
    Transferred,
    /// An MCP token, shown this once (`DirectoryState::issued`).
    McpTokenIssued,
}

#[derive(Clone, Debug)]
pub enum DirectoryEvent {
    Done(DirectoryDone),
    Failed(StoreError),
}

pub struct DirectoryState {
    directory: Arc<dyn DirectoryStore>,
    me: Option<Me>,
    accounts: Vec<AccountInfo>,
    members: HashMap<TeamId, Vec<Member>>,
    grants: Option<(CollectionId, Vec<Grant>)>,
    /// The reader's invitations that can still be used.
    invites: Vec<Invite>,
    /// The one just made, and its link: shown this once, as only now its
    /// token is known.
    made: Option<(InviteId, String)>,
    /// The reader's MCP tokens that still sign in.
    mcp_tokens: Vec<McpToken>,
    /// The token just issued, and its secret: shown this once.
    issued: Option<(McpTokenId, String)>,
}

impl EventEmitter<DirectoryEvent> for DirectoryState {}

impl DirectoryState {
    /// Reads nothing yet: each dialog loads what it shows.
    pub fn new(directory: Arc<dyn DirectoryStore>) -> Self {
        Self {
            directory,
            me: None,
            accounts: Vec::new(),
            members: HashMap::new(),
            grants: None,
            invites: Vec::new(),
            made: None,
            mcp_tokens: Vec::new(),
            issued: None,
        }
    }

    pub fn me(&self) -> Option<&Me> {
        self.me.as_ref()
    }

    pub fn accounts(&self) -> &[AccountInfo] {
        &self.accounts
    }

    /// A team's members, once read.
    pub fn members(&self, team: &TeamId) -> Option<&[Member]> {
        self.members.get(team).map(Vec::as_slice)
    }

    /// The project's grants, once read.
    pub fn grants(&self, collection: &CollectionId) -> Option<&[Grant]> {
        self.grants
            .as_ref()
            .filter(|(of, _)| of == collection)
            .map(|(_, grants)| grants.as_slice())
    }

    /// The reader's invitations into `team`, or (`None`) into no team.
    pub fn invites<'a>(&'a self, team: Option<&'a TeamId>) -> impl Iterator<Item = &'a Invite> + 'a {
        self.invites
            .iter()
            .filter(move |invite| invite.place.as_ref().map(|place| &place.team) == team)
    }

    /// The invitation just made, and its link.
    pub fn made(&self) -> Option<&(InviteId, String)> {
        self.made.as_ref()
    }

    /// The reader's MCP tokens, once read, oldest first.
    pub fn mcp_tokens(&self) -> &[McpToken] {
        &self.mcp_tokens
    }

    /// The MCP token just issued, and its secret.
    pub fn issued(&self) -> Option<&(McpTokenId, String)> {
        self.issued.as_ref()
    }

    fn failed(cx: &mut Context<Self>, error: StoreError) {
        cx.emit(DirectoryEvent::Failed(error));
    }

    pub fn reload_me(&mut self, cx: &mut Context<Self>) {
        run_store(cx, self.directory.me(), |this, result, cx| match result {
            Ok(me) => {
                this.me = Some(me);
                cx.notify();
            }
            Err(error) => Self::failed(cx, error),
        });
    }

    /// The reader's teams changed: here, and for the rest of the window.
    fn teams_changed(&mut self, cx: &mut Context<Self>) {
        self.reload_me(cx);
        let app = AppStore::global(cx).state().clone();
        app.update(cx, |app, cx| app.reload_me(cx));
    }

    pub fn reload_accounts(&mut self, cx: &mut Context<Self>) {
        run_store(cx, self.directory.accounts(), |this, result, cx| match result {
            Ok(accounts) => {
                this.accounts = accounts;
                cx.notify();
            }
            Err(error) => Self::failed(cx, error),
        });
    }

    pub fn load_members(&mut self, team: TeamId, cx: &mut Context<Self>) {
        let of = team.clone();
        run_store(cx, self.directory.members(team), move |this, result, cx| match result {
            Ok(members) => {
                this.members.insert(of, members);
                cx.notify();
            }
            Err(error) => Self::failed(cx, error),
        });
    }

    pub fn load_grants(&mut self, collection: CollectionId, cx: &mut Context<Self>) {
        let of = collection.clone();
        run_store(
            cx,
            self.directory.grants(collection),
            move |this, result, cx| match result {
                Ok(grants) => {
                    this.grants = Some((of, grants));
                    cx.notify();
                }
                Err(error) => Self::failed(cx, error),
            },
        );
    }

    pub fn create_team(&mut self, name: String, cx: &mut Context<Self>) {
        run_store(cx, self.directory.create_team(name), |this, result, cx| match result {
            Ok(team) => {
                this.teams_changed(cx);
                cx.emit(DirectoryEvent::Done(DirectoryDone::TeamCreated(team.id)));
            }
            Err(error) => Self::failed(cx, error),
        });
    }

    pub fn rename_team(&mut self, team: TeamId, name: String, cx: &mut Context<Self>) {
        run_store(
            cx,
            self.directory.rename_team(team, name),
            |this, result, cx| match result {
                Ok(()) => this.teams_changed(cx),
                Err(error) => {
                    // What the field shows goes back to the stored name.
                    cx.notify();
                    Self::failed(cx, error);
                }
            },
        );
    }

    pub fn delete_team(&mut self, team: TeamId, cx: &mut Context<Self>) {
        let deleted = team.clone();
        run_store(
            cx,
            self.directory.delete_team(team),
            move |this, result, cx| match result {
                Ok(()) => {
                    this.members.remove(&deleted);
                    this.teams_changed(cx);
                    cx.emit(DirectoryEvent::Done(DirectoryDone::TeamDeleted));
                }
                Err(error) => Self::failed(cx, error),
            },
        );
    }

    /// Adds, changes or (`None`) removes a member; the reader removing
    /// themselves leaves the team.
    pub fn set_member(&mut self, team: TeamId, account: AccountId, role: Option<TeamRole>, cx: &mut Context<Self>) {
        let leaving = role.is_none() && self.me.as_ref().is_some_and(|me| me.account.id == account);
        let of = team.clone();
        run_store(
            cx,
            self.directory.set_member(team, account, role),
            move |this, result, cx| match result {
                Ok(()) if leaving => {
                    this.members.remove(&of);
                    this.teams_changed(cx);
                    cx.emit(DirectoryEvent::Done(DirectoryDone::TeamLeft));
                }
                Ok(()) => this.load_members(of, cx),
                Err(error) => Self::failed(cx, error),
            },
        );
    }

    pub fn set_grant(
        &mut self,
        collection: CollectionId,
        grantee: Grantee,
        role: Option<CollectionRole>,
        cx: &mut Context<Self>,
    ) {
        let of = collection.clone();
        run_store(
            cx,
            self.directory.set_grant(collection, grantee, role),
            move |this, result, cx| match result {
                Ok(()) => this.load_grants(of, cx),
                Err(error) => Self::failed(cx, error),
            },
        );
    }

    pub fn transfer(&mut self, collection: CollectionId, team: TeamId, cx: &mut Context<Self>) {
        run_store(
            cx,
            self.directory.transfer(collection, team),
            |_, result, cx| match result {
                Ok(()) => cx.emit(DirectoryEvent::Done(DirectoryDone::Transferred)),
                Err(error) => Self::failed(cx, error),
            },
        );
    }

    pub fn load_invites(&mut self, cx: &mut Context<Self>) {
        run_store(cx, self.directory.invites(), |this, result, cx| match result {
            Ok(invites) => {
                this.invites = invites;
                cx.notify();
            }
            Err(error) => Self::failed(cx, error),
        });
    }

    /// An invitation to `place`, or to an account of its own; its link is
    /// handed out once, in [`DirectoryDone::InviteCreated`].
    pub fn create_invite(&mut self, place: Option<TeamPlace>, cx: &mut Context<Self>) {
        run_store(
            cx,
            self.directory.create_invite(place),
            |this, result, cx| match result {
                Ok(made) => {
                    let server = cx.try_global::<AppStore>().and_then(|store| store.server_address(cx));
                    let link = this_build::invite_link(server.as_deref(), &made.token);
                    this.made = Some((made.invite.id.clone(), link.clone()));
                    this.invites.push(made.invite);
                    cx.emit(DirectoryEvent::Done(DirectoryDone::InviteCreated(link)));
                    cx.notify();
                }
                Err(error) => Self::failed(cx, error),
            },
        );
    }

    pub fn revoke_invite(&mut self, id: InviteId, cx: &mut Context<Self>) {
        let revoked = id.clone();
        run_store(
            cx,
            self.directory.revoke_invite(id),
            move |this, result, cx| match result {
                Ok(()) => {
                    this.invites.retain(|invite| invite.id != revoked);
                    if this.made.as_ref().is_some_and(|(made, _)| made == &revoked) {
                        this.made = None;
                    }
                    cx.notify();
                }
                Err(error) => Self::failed(cx, error),
            },
        );
    }

    pub fn update_account(&mut self, account: AccountId, edit: AccountEdit, cx: &mut Context<Self>) {
        run_store(
            cx,
            self.directory.update_account(account, edit),
            |this, result, cx| match result {
                Ok(updated) => {
                    if let Some(found) = this.accounts.iter_mut().find(|found| found.id == updated.id) {
                        *found = updated;
                    }
                    cx.notify();
                }
                Err(error) => {
                    cx.notify();
                    Self::failed(cx, error);
                }
            },
        );
    }

    pub fn reset_password(&mut self, account: AccountId, password: String, cx: &mut Context<Self>) {
        run_store(
            cx,
            self.directory.reset_password(account, password),
            |_, result, cx| match result {
                Ok(()) => cx.emit(DirectoryEvent::Done(DirectoryDone::PasswordReset)),
                Err(error) => Self::failed(cx, error),
            },
        );
    }

    pub fn reload_mcp_tokens(&mut self, cx: &mut Context<Self>) {
        run_store(cx, self.directory.mcp_tokens(), |this, result, cx| match result {
            Ok(tokens) => {
                this.mcp_tokens = tokens;
                cx.notify();
            }
            Err(error) => Self::failed(cx, error),
        });
    }

    pub fn issue_mcp_token(&mut self, input: NewMcpToken, cx: &mut Context<Self>) {
        run_store(
            cx,
            self.directory.issue_mcp_token(input),
            |this, result, cx| match result {
                Ok(issued) => {
                    this.issued = Some((issued.token.id.clone(), issued.secret));
                    this.mcp_tokens.push(issued.token);
                    cx.emit(DirectoryEvent::Done(DirectoryDone::McpTokenIssued));
                    cx.notify();
                }
                Err(error) => Self::failed(cx, error),
            },
        );
    }

    pub fn revoke_mcp_token(&mut self, id: McpTokenId, cx: &mut Context<Self>) {
        let revoked = id.clone();
        run_store(
            cx,
            self.directory.revoke_mcp_token(id),
            move |this, result, cx| match result {
                Ok(()) => {
                    this.mcp_tokens.retain(|token| token.id != revoked);
                    if this.issued.as_ref().is_some_and(|(issued, _)| issued == &revoked) {
                        this.issued = None;
                    }
                    cx.notify();
                }
                Err(error) => Self::failed(cx, error),
            },
        );
    }

    pub fn change_password(&mut self, current: String, password: String, cx: &mut Context<Self>) {
        run_store(
            cx,
            self.directory.change_password(current, password),
            |_, result, cx| match result {
                Ok(()) => cx.emit(DirectoryEvent::Done(DirectoryDone::PasswordChanged)),
                Err(error) => Self::failed(cx, error),
            },
        );
    }
}
