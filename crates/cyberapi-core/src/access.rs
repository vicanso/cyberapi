//! Who may do what with a project (`docs/architecture.md` §9). Only the
//! browser build has more than one reader; on the desktop every project is
//! [`CollectionRole::Manager`].

use crate::{AccountId, TeamId};
use serde::{Deserialize, Serialize};

/// A reader's role on one project, weakest first, so roles compare with `<`
/// and the strongest of several grants is their `max`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CollectionRole {
    /// Sees everything and sends requests; the history is the reader's own.
    Viewer,
    /// Also changes the tree, the requests and the variables.
    Editor,
    /// Also renames or deletes the project and decides who has access.
    #[default]
    Manager,
}

impl CollectionRole {
    pub fn can_edit(&self) -> bool {
        *self >= Self::Editor
    }

    pub fn can_manage(&self) -> bool {
        *self >= Self::Manager
    }
}

/// An account's place in a team (§9.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TeamRole {
    Member,
    /// Manages the team's members and projects.
    Admin,
    /// The team's creator; the only member of a personal space.
    Owner,
}

impl TeamRole {
    /// The role a member has on the team's projects: owners and admins
    /// manage them, members edit them (D7).
    pub fn on_team_projects(&self) -> CollectionRole {
        match self {
            Self::Owner | Self::Admin => CollectionRole::Manager,
            Self::Member => CollectionRole::Editor,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Admin => "admin",
            Self::Owner => "owner",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "member" => Some(Self::Member),
            "admin" => Some(Self::Admin),
            "owner" => Some(Self::Owner),
            _ => None,
        }
    }
}

impl CollectionRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Editor => "editor",
            Self::Manager => "manager",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "viewer" => Some(Self::Viewer),
            "editor" => Some(Self::Editor),
            "manager" => Some(Self::Manager),
            _ => None,
        }
    }
}

/// Who a project is granted to (§9.1): one account, or every member of a
/// team.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "camelCase")]
pub enum Grantee {
    Account(AccountId),
    Team(TeamId),
}

/// An account's role on a project (§9.3): the strongest of what its place in
/// the owning team gives and of the grants that reach it — to the account
/// itself or to a team it is in. `None` when nothing does: the project is not
/// visible to it.
pub fn effective_role(
    owner_team_role: Option<TeamRole>,
    grants: impl IntoIterator<Item = CollectionRole>,
) -> Option<CollectionRole> {
    owner_team_role
        .map(|role| role.on_team_projects())
        .into_iter()
        .chain(grants)
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strongest_source_wins() {
        assert_eq!(effective_role(None, []), None);
        assert_eq!(effective_role(Some(TeamRole::Member), []), Some(CollectionRole::Editor));
        assert_eq!(effective_role(Some(TeamRole::Admin), []), Some(CollectionRole::Manager));
        assert_eq!(
            effective_role(None, [CollectionRole::Viewer]),
            Some(CollectionRole::Viewer)
        );
        assert_eq!(
            effective_role(
                Some(TeamRole::Member),
                [CollectionRole::Viewer, CollectionRole::Manager]
            ),
            Some(CollectionRole::Manager)
        );
        // A weaker grant never takes away what the team gives.
        assert_eq!(
            effective_role(Some(TeamRole::Member), [CollectionRole::Viewer]),
            Some(CollectionRole::Editor)
        );
    }

    #[test]
    fn stored_names_round_trip() {
        for role in [TeamRole::Member, TeamRole::Admin, TeamRole::Owner] {
            assert_eq!(TeamRole::parse(role.as_str()), Some(role));
        }
        for role in [CollectionRole::Viewer, CollectionRole::Editor, CollectionRole::Manager] {
            assert_eq!(CollectionRole::parse(role.as_str()), Some(role));
        }
    }

    #[test]
    fn roles_are_ordered_by_strength() {
        assert!(CollectionRole::Viewer < CollectionRole::Editor);
        assert_eq!(
            [CollectionRole::Editor, CollectionRole::Viewer].into_iter().max(),
            Some(CollectionRole::Editor)
        );
        assert!(!CollectionRole::Viewer.can_edit());
        assert!(CollectionRole::Editor.can_edit() && !CollectionRole::Editor.can_manage());
        assert!(CollectionRole::Manager.can_manage());
    }
}
