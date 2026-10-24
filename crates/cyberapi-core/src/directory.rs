//! The browser build's accounts and teams (`docs/architecture.md` §9.1), as
//! a reader sees them, and the rules for new ones. The desktop has none.

use crate::access::{CollectionRole, Grantee, TeamRole};
use crate::{AccountId, InviteId, McpTokenId, TeamId, Timestamp};
use serde::{Deserialize, Serialize};

pub const MIN_PASSWORD_CHARS: usize = 8;
pub const MAX_ACCOUNT_NAME_CHARS: usize = 64;
/// How long an invitation can be used (`docs/server-sync.md` §5).
pub const INVITE_DAYS: i64 = 7;
/// The longest an MCP token's name is.
pub const MAX_TOKEN_NAME_CHARS: usize = 64;
/// How every MCP token starts, so one pasted somewhere is recognised.
pub const MCP_TOKEN_PREFIX: &str = "cat_";
/// The lifetimes a new MCP token may be given, in days, the default first;
/// or none, for one that lasts until it is revoked.
pub const MCP_TOKEN_DAYS: [u32; 3] = [90, 30, 365];

/// An account, as every signed-in reader may see it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub id: AccountId,
    /// What it signs in with.
    pub name: String,
    pub display_name: String,
    /// A system administrator: manages accounts, not everyone's projects.
    pub admin: bool,
    /// Cannot sign in.
    pub disabled: bool,
}

impl AccountInfo {
    /// What to call it: the display name, or the sign-in name without one.
    pub fn label(&self) -> &str {
        if self.display_name.trim().is_empty() {
            &self.name
        } else {
            &self.display_name
        }
    }
}

/// A team the reader is in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub id: TeamId,
    pub name: String,
    /// The reader's own space, which no one else can join.
    pub personal: bool,
    /// The reader's place in it.
    pub role: TeamRole,
}

/// Someone in a team.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub account: AccountInfo,
    pub role: TeamRole,
}

/// A project given to someone outside its team (§9.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub grantee: Grantee,
    /// The account's label or the team's name.
    pub name: String,
    pub role: CollectionRole,
}

/// The signed-in reader and their teams, their own space first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    pub account: AccountInfo,
    pub teams: Vec<Team>,
}

/// A team, and the place in it an invitation gives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamPlace {
    pub team: TeamId,
    pub role: TeamRole,
}

/// An invitation to make an account (`docs/server-sync.md` §5), as the one
/// who sent it lists it while it can still be used.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    pub id: InviteId,
    /// The team the new account joins, and as what; `None`: an
    /// administrator's, for an account with only its own space.
    pub place: Option<TeamPlace>,
    /// The team's name, with `place`.
    pub team_name: Option<String>,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

/// An invitation just made, and what opens it: shown this once, never kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewInvite {
    pub invite: Invite,
    pub token: String,
}

/// An MCP token of the reader's: how an AI assistant signs in as them at the
/// bridge's `/v1/mcp`, and nowhere else. It reads what the account reads;
/// `can_send` lets it send the account's saved requests too.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToken {
    pub id: McpTokenId,
    /// The machine or the client it is for, unique among the account's.
    pub name: String,
    pub can_send: bool,
    pub created_at: Timestamp,
    /// `None`: until it is revoked.
    pub expires_at: Option<Timestamp>,
}

/// A token to issue: its name, whether it may send, and how many days it
/// lasts (one of [`MCP_TOKEN_DAYS`], or `None` for no end).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMcpToken {
    pub name: String,
    pub can_send: bool,
    pub days: Option<u32>,
}

/// A token just issued, and the secret itself: shown this once, never kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuedMcpToken {
    pub token: McpToken,
    pub secret: String,
}

/// A token's name: 1 to 64 characters, not all blank.
pub fn valid_token_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name.chars().count() <= MAX_TOKEN_NAME_CHARS
}

/// What an invitation's link says before an account is made with it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteInfo {
    /// Who sent it.
    pub inviter: String,
    /// The team it joins, if one.
    pub team: Option<String>,
}

/// The place in a team its member needs to invite someone in as `role`
/// (server-sync §5): an admin invites members and admins; only an owner
/// invites another owner.
pub fn invite_needs(role: TeamRole) -> TeamRole {
    match role {
        TeamRole::Owner => TeamRole::Owner,
        TeamRole::Admin | TeamRole::Member => TeamRole::Admin,
    }
}

/// A sign-in name: 1 to 64 characters, none of them blank.
pub fn valid_account_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= MAX_ACCOUNT_NAME_CHARS && !name.chars().any(char::is_whitespace)
}

pub fn valid_password(password: &str) -> bool {
    password.chars().count() >= MIN_PASSWORD_CHARS
}

/// Teams in the order a reader lists them: their own space, then by name.
pub fn sort_teams(teams: &mut [Team]) {
    teams.sort_by(|a, b| {
        b.personal
            .cmp(&a.personal)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_passwords() {
        assert!(valid_account_name("alice") && valid_account_name("张三"));
        assert!(!valid_account_name("") && !valid_account_name("a b") && !valid_account_name(&"x".repeat(65)));
        assert!(valid_password("12345678") && !valid_password("1234567"));
        let mut info = AccountInfo {
            name: "alice".into(),
            ..Default::default()
        };
        assert_eq!(info.label(), "alice");
        info.display_name = "Alice".into();
        assert_eq!(info.label(), "Alice");
    }

    #[test]
    fn only_an_owner_invites_an_owner() {
        assert_eq!(invite_needs(TeamRole::Member), TeamRole::Admin);
        assert_eq!(invite_needs(TeamRole::Admin), TeamRole::Admin);
        assert_eq!(invite_needs(TeamRole::Owner), TeamRole::Owner);
    }

    #[test]
    fn the_own_space_comes_first() {
        let team = |name: &str, personal: bool| Team {
            id: TeamId::new(name),
            name: name.into(),
            personal,
            role: TeamRole::Owner,
        };
        let mut teams = vec![team("qa", false), team("Backend", false), team("me", true)];
        sort_teams(&mut teams);
        let names: Vec<&str> = teams.iter().map(|team| team.name.as_str()).collect();
        assert_eq!(names, ["me", "Backend", "qa"]);
    }
}
