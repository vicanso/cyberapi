//! A server project's configuration and its syncing (`docs/server-sync.md`):
//! the part of a project that travels whole between a reader's copy and the
//! server, and what a copy does when the two have moved apart.

use crate::model::{ApiSetting, Folder, Variable};
use crate::tree::TreeData;
use crate::{AccountId, CollectionId, ItemId};
use serde::{Deserialize, Serialize};

/// A project's configuration, whole (server-sync §2): its name and
/// description, its tree and its variables of every category. What its
/// readers keep for themselves — response history, cookies, prefs — is not
/// part of it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProjectConfig {
    pub name: String,
    pub description: String,
    pub folders: Vec<Folder>,
    pub apis: Vec<ApiSetting>,
    /// The top level's order.
    pub top_order: Vec<ItemId>,
    /// Environment variables, custom variables and global request headers.
    pub variables: Vec<Variable>,
}

impl ProjectConfig {
    /// Its tree, as `cyberapi_core::tree` builds it.
    pub fn tree(&self) -> TreeData {
        TreeData {
            folders: self.folders.clone(),
            apis: self.apis.clone(),
            top_order: self.top_order.clone(),
        }
    }
}

/// A configuration as a store read it, and the project's revision then: the
/// base a copy later syncs against.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConfigAt {
    pub revision: u64,
    pub config: ProjectConfig,
}

/// The desktop's copy of a server project (server-sync §6): whose it is,
/// the server's revision it was last level with, and whether it has changes
/// the server has not had. A project of the machine's own has none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyBinding {
    pub collection: CollectionId,
    pub account: AccountId,
    pub base: Option<u64>,
    pub unsynced: bool,
}

/// When a copy of a server project looks at the server's revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moment {
    /// Signing in, opening the project, or the poll while it is open (§3.1).
    Look,
    /// The reader asked to sync (§3.3).
    Push,
}

/// What a copy does about the server, decided in one place: every screen
/// that pulls, polls or pushes asks [`step`] and does what it answers.
/// Not being able to reach the server is not one of these: it is the
/// caller's, which then keeps the copy as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing unsynced, the server moved on (or there is no copy yet): its
    /// content replaces the copy and its revision becomes the base.
    Pull,
    /// Nothing unsynced and nothing new: the copy is the server's already.
    UpToDate,
    /// Unsynced changes are kept, and the reader told they are not on the
    /// server yet.
    KeepUnsynced,
    /// Unsynced changes are kept, and the reader told the server has moved
    /// on as well — nothing is overwritten either way.
    KeepUnsyncedBehind,
    /// A push at the base the copy has: the copy goes up whole.
    Write,
    /// A push, and the server moved on since the base: the reader chooses —
    /// theirs over the server's, the server's over theirs, or neither.
    Conflict,
}

/// The [`Step`] for a copy at `base` (`None`: none pulled yet), the server
/// at `server`, with or without `unsynced` changes, at `moment`.
pub fn step(moment: Moment, base: Option<u64>, server: u64, unsynced: bool) -> Step {
    let moved = base != Some(server);
    match moment {
        Moment::Look if unsynced && moved => Step::KeepUnsyncedBehind,
        Moment::Look if unsynced => Step::KeepUnsynced,
        Moment::Look if moved => Step::Pull,
        Moment::Look => Step::UpToDate,
        Moment::Push if moved => Step::Conflict,
        Moment::Push => Step::Write,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looking_pulls_only_over_nothing_unsynced() {
        // Nothing of the reader's to lose: the server's, when it moved.
        assert_eq!(step(Moment::Look, Some(3), 4, false), Step::Pull);
        assert_eq!(step(Moment::Look, None, 0, false), Step::Pull, "no copy yet");
        assert_eq!(step(Moment::Look, Some(4), 4, false), Step::UpToDate);
        // Unsynced changes are never overwritten by a look.
        assert_eq!(step(Moment::Look, Some(4), 4, true), Step::KeepUnsynced);
        assert_eq!(step(Moment::Look, Some(3), 4, true), Step::KeepUnsyncedBehind);
        assert_eq!(step(Moment::Look, None, 2, true), Step::KeepUnsyncedBehind);
    }

    #[test]
    fn pushing_writes_only_at_the_base() {
        assert_eq!(step(Moment::Push, Some(4), 4, true), Step::Write);
        assert_eq!(step(Moment::Push, Some(3), 4, true), Step::Conflict);
        assert_eq!(step(Moment::Push, None, 4, true), Step::Conflict, "no base to be at");
        // A push with nothing unsynced still writes what the copy holds.
        assert_eq!(step(Moment::Push, Some(4), 4, false), Step::Write);
    }
}
