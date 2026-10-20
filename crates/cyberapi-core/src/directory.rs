//! The browser build's accounts and teams (`docs/architecture.md` §9.1), as
//! a reader sees them, and the rules for new ones. The desktop has none.

use crate::access::{CollectionRole, Grantee, TeamRole};
use crate::{AccountId, TeamId};
use serde::{Deserialize, Serialize};

pub const MIN_PASSWORD_CHARS: usize = 8;
pub const MAX_ACCOUNT_NAME_CHARS: usize = 64;

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
