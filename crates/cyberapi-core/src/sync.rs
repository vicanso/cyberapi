//! A server project's configuration and its syncing (`docs/server-sync.md`):
//! the part of a project that travels whole between a reader's copy and the
//! server, and what a copy does when the two have moved apart.

use crate::ItemId;
use crate::model::{ApiSetting, Folder, Variable};
use crate::tree::TreeData;
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
