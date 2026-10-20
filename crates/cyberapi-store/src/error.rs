use cyberapi_core::access::{CollectionRole, TeamRole};
use cyberapi_core::tree::TreeError;
use serde::{Deserialize, Serialize};
use snafu::Snafu;

/// What a store refuses or fails with. Serializable, because the browser
/// receives it from the bridge exactly as the bridge's store answered it; the
/// variants a reader can cause are distinct so the UI can say what happened.
#[derive(Clone, Debug, PartialEq, Eq, Snafu, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[snafu(visibility(pub))]
pub enum StoreError {
    /// Not there — or, in the browser build, not visible to this reader,
    /// which is answered the same way so it does not tell them it exists.
    #[snafu(display("{what:?} {id} was not found"))]
    NotFound { what: Entity, id: String },
    /// Visible, but the reader's role is not enough for the change.
    #[snafu(display("this needs the {needed:?} role"))]
    Forbidden { needed: CollectionRole },
    #[snafu(display("a name is required"))]
    NameRequired,
    /// A change to accounts or teams the rules do not allow (§9).
    #[snafu(display("refused: {reason:?}"))]
    Refused { reason: Refusal },
    /// A tree change the rules refuse: a folder into itself, an API used as a
    /// folder. A missing item or folder is [`StoreError::NotFound`] instead.
    #[snafu(display("{source}"))]
    Tree { source: TreeError },
    /// The backend itself failed: the database, the disk, the network.
    #[snafu(display("storage failed: {message}"))]
    Backend { message: String },
}

impl StoreError {
    pub fn not_found(what: Entity, id: impl ToString) -> Self {
        Self::NotFound {
            what,
            id: id.to_string(),
        }
    }

    pub fn backend(error: impl std::fmt::Display) -> Self {
        Self::Backend {
            message: error.to_string(),
        }
    }
}

/// A plan's "not in this tree" is the store's "not found", so a missing id
/// reads the same whichever method met it.
impl From<TreeError> for StoreError {
    fn from(error: TreeError) -> Self {
        match error {
            TreeError::UnknownItem { id } => Self::not_found(Entity::Item, id),
            TreeError::UnknownFolder { id } => Self::not_found(Entity::Folder, id),
            source => Self::Tree { source },
        }
    }
}

/// What a [`StoreError::NotFound`] was looking for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Entity {
    Collection,
    Folder,
    Api,
    /// A folder or an API.
    Item,
    Variable,
    /// A team the reader is not in (the browser build's).
    Team,
    Account,
}

/// Why a change to accounts or teams was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "why", rename_all = "camelCase")]
pub enum Refusal {
    /// Only a system administrator manages accounts.
    AdminOnly,
    /// The reader's place in the team is not enough.
    TeamRole {
        needed: TeamRole,
    },
    /// A personal space has its owner and no one else, and keeps its name.
    PersonalSpace,
    /// A team keeps at least one owner.
    LastOwner,
    /// A team that owns projects cannot go; move them first.
    TeamHasProjects,
    /// Another account signs in with that name.
    NameTaken,
    /// A sign-in name is 1 to 64 characters, none of them blank.
    BadAccountName,
    PasswordTooShort,
    WrongPassword,
    /// An administrator cannot disable or demote themselves.
    NotYourself,
}
