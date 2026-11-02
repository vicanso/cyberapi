//! A server project's configuration and its syncing (`docs/server-sync.md`):
//! the part of a project that travels whole between a reader's copy and the
//! server, and what a copy does when the two have moved apart.

use crate::model::{ApiCategory, ApiSetting, Environment, Folder, HttpMethod, HttpRequest, ProjectSettings, Variable};
use crate::tree::{Placement, TreeData, TreeError};
use crate::{AccountId, CollectionId, ItemId, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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
    /// What the project sets for its APIs (#66).
    #[serde(skip_serializing_if = "ProjectSettings::is_empty")]
    pub settings: ProjectSettings,
    /// Its named environments, in order (#61); the variables say which is
    /// theirs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub environments: Vec<Environment>,
}

impl ProjectConfig {
    /// Its tree, as `cyberapi_core::tree` builds it.
    pub fn tree(&self) -> TreeData {
        TreeData {
            folders: self.folders.clone(),
            apis: self.apis.clone(),
            top_order: self.top_order.clone(),
            settings: self.settings.clone(),
        }
    }
}

/// G4: how many versions a server keeps of each project.
pub const KEPT_VERSIONS: usize = 50;

/// G4: a project as it was before it was replaced whole — a sync, an
/// upload, a version put back — as a list of them shows it; its
/// configuration is read apart.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProjectVersion {
    pub id: String,
    /// The project's revision it was at.
    pub revision: u64,
    /// When it was replaced, and by whom (an account's name).
    pub saved_at: crate::Timestamp,
    pub by: String,
    pub folders: u32,
    pub apis: u32,
}

impl ProjectConfig {
    /// Whether it holds nothing: no item, no variable, no environment.
    pub fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.apis.is_empty() && self.variables.is_empty() && self.environments.is_empty()
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

/// One API as an outline lists it: enough to draw the tree and to search,
/// and not its request. The request is read when the API is opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiHead {
    pub id: ItemId,
    pub name: String,
    pub method: HttpMethod,
    pub uri: String,
    pub updated_at: Timestamp,
}

impl ApiHead {
    /// The API as the outline knows it: method and URI, and an empty request
    /// beside them. A stored request that is empty looks the same; the caller
    /// remembers which APIs it has actually read.
    pub fn stub(&self, collection: &CollectionId) -> ApiSetting {
        ApiSetting {
            id: self.id.clone(),
            collection: collection.clone(),
            name: self.name.clone(),
            category: ApiCategory::Http,
            request: HttpRequest {
                method: self.method,
                uri: self.uri.clone(),
                ..HttpRequest::default()
            },
            created_at: self.updated_at,
            updated_at: self.updated_at,
        }
    }

    pub fn from_api(api: &ApiSetting) -> Self {
        Self {
            id: api.id.clone(),
            name: api.name.clone(),
            method: api.request.method,
            uri: api.request.uri.clone(),
            updated_at: api.updated_at,
        }
    }
}

/// A project as opening it reads it: the tree's shape, and everything small
/// that travels with the project. Request documents are heads, not bodies.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProjectOutline {
    pub name: String,
    pub description: String,
    pub folders: Vec<Folder>,
    pub apis: Vec<ApiHead>,
    pub top_order: Vec<ItemId>,
    pub variables: Vec<Variable>,
    #[serde(skip_serializing_if = "ProjectSettings::is_empty")]
    pub settings: ProjectSettings,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub environments: Vec<Environment>,
}

impl ProjectOutline {
    /// A configuration whose APIs are stubs, so a working copy can hold the
    /// outline without the request documents.
    pub fn config(&self, collection: &CollectionId) -> ProjectConfig {
        ProjectConfig {
            name: self.name.clone(),
            description: self.description.clone(),
            folders: self.folders.clone(),
            apis: self.apis.iter().map(|api| api.stub(collection)).collect(),
            top_order: self.top_order.clone(),
            variables: self.variables.clone(),
            settings: self.settings.clone(),
            environments: self.environments.clone(),
        }
    }
}

/// An outline as a store read it, and the project's revision then.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OutlineAt {
    pub revision: u64,
    pub outline: ProjectOutline,
}

/// One change a web draft keeps, and a server applies onto the project as it
/// is. A change names what it touches. An API it never names stays, body and
/// all: applying a draft does not delete what was never opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum ProjectOp {
    CreateFolder {
        parent: Option<ItemId>,
        folder: Folder,
    },
    CreateApi {
        parent: Option<ItemId>,
        api: ApiSetting,
    },
    /// Folders and APIs brought in together (an import), placed at the end of
    /// `parent`. Their own children lists say what is inside them.
    Import {
        parent: Option<ItemId>,
        folders: Vec<Folder>,
        apis: Vec<ApiSetting>,
    },
    Rename {
        item: ItemId,
        name: String,
    },
    Move {
        item: ItemId,
        to: Placement,
    },
    Delete {
        item: ItemId,
    },
    SaveRequest {
        api: ItemId,
        request: HttpRequest,
    },
    FolderSettings {
        folder: ItemId,
        settings: crate::model::FolderSettings,
    },
    ProjectSettings {
        settings: ProjectSettings,
    },
    /// Variables and named environments, both whole, whenever either changed.
    Variables {
        variables: Vec<Variable>,
        environments: Vec<Environment>,
    },
    Describe {
        name: String,
        description: String,
    },
}

/// The changes a draft has saved up, in the order they were made.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProjectChanges {
    pub ops: Vec<ProjectOp>,
}

impl ProjectChanges {
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// Why [`apply_changes`] could not. A missing item on a delete is not one:
/// deleting what is already gone changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyError {
    Tree(TreeError),
    NameRequired,
    MissingItem { id: ItemId },
    MissingApi { id: ItemId },
    MissingFolder { id: ItemId },
}

impl From<TreeError> for ApplyError {
    fn from(error: TreeError) -> Self {
        match error {
            TreeError::UnknownItem { id } => Self::MissingItem { id },
            TreeError::UnknownFolder { id } => Self::MissingFolder { id },
            other => Self::Tree(other),
        }
    }
}

/// Applies `changes` onto `config`, which is the project whole — every
/// request the server has. An API no change names is left as it was.
pub fn apply_changes(config: &mut ProjectConfig, changes: &ProjectChanges) -> Result<(), ApplyError> {
    for op in &changes.ops {
        apply_op(config, op)?;
    }
    Ok(())
}

/// Gives "now" to the items a change wrote, so a later read can see that
/// those requests moved. Items a change does not name keep their time.
pub fn touch_changed(config: &mut ProjectConfig, changes: &ProjectChanges, now: Timestamp) {
    for op in &changes.ops {
        match op {
            ProjectOp::CreateFolder { folder, .. } => stamp_folder(config, &folder.id, now),
            ProjectOp::FolderSettings { folder, .. } => stamp_folder(config, folder, now),
            ProjectOp::CreateApi { api, .. } => stamp_api(config, &api.id, now),
            ProjectOp::SaveRequest { api, .. } => stamp_api(config, api, now),
            ProjectOp::Rename { item, .. } => {
                stamp_folder(config, item, now);
                stamp_api(config, item, now);
            }
            ProjectOp::Import { folders, apis, .. } => {
                for folder in folders {
                    stamp_folder(config, &folder.id, now);
                }
                for api in apis {
                    stamp_api(config, &api.id, now);
                }
            }
            ProjectOp::Move { .. }
            | ProjectOp::Delete { .. }
            | ProjectOp::ProjectSettings { .. }
            | ProjectOp::Variables { .. }
            | ProjectOp::Describe { .. } => {}
        }
    }
}

fn stamp_folder(config: &mut ProjectConfig, id: &ItemId, now: Timestamp) {
    if let Some(folder) = config.folders.iter_mut().find(|folder| &folder.id == id) {
        folder.updated_at = now;
    }
}

fn stamp_api(config: &mut ProjectConfig, id: &ItemId, now: Timestamp) {
    if let Some(api) = config.apis.iter_mut().find(|api| &api.id == id) {
        api.updated_at = now;
    }
}

fn apply_op(config: &mut ProjectConfig, op: &ProjectOp) -> Result<(), ApplyError> {
    match op {
        ProjectOp::CreateFolder { parent, folder } => {
            if item_exists(config, &folder.id) {
                return Ok(());
            }
            let mut folder = folder.clone();
            folder.children.clear();
            let id = folder.id.clone();
            config.folders.push(folder);
            place_new(config, &id, parent.as_ref())
        }
        ProjectOp::CreateApi { parent, api } => {
            if item_exists(config, &api.id) {
                return Ok(());
            }
            let id = api.id.clone();
            config.apis.push(api.clone());
            place_new(config, &id, parent.as_ref())
        }
        ProjectOp::Import { parent, folders, apis } => {
            let mut nested = HashSet::new();
            for folder in folders {
                nested.extend(folder.children.iter().cloned());
            }
            let mut tops = Vec::new();
            for folder in folders {
                if !item_exists(config, &folder.id) {
                    if !nested.contains(&folder.id) {
                        tops.push(folder.id.clone());
                    }
                    config.folders.push(folder.clone());
                }
            }
            for api in apis {
                if !item_exists(config, &api.id) {
                    if !nested.contains(&api.id) {
                        tops.push(api.id.clone());
                    }
                    config.apis.push(api.clone());
                }
            }
            if tops.is_empty() {
                return Ok(());
            }
            let mut tree = config.tree();
            let change = tree.plan_import(&tops, parent.as_ref())?;
            tree.apply(&change);
            write_tree(config, tree);
            Ok(())
        }
        ProjectOp::Rename { item, name } => {
            let name = name.trim();
            if name.is_empty() {
                return Err(ApplyError::NameRequired);
            }
            let name = name.to_string();
            if let Some(folder) = config.folders.iter_mut().find(|folder| &folder.id == item) {
                folder.name = name;
                return Ok(());
            }
            if let Some(api) = config.apis.iter_mut().find(|api| &api.id == item) {
                api.name = name;
                return Ok(());
            }
            Err(ApplyError::MissingItem { id: item.clone() })
        }
        ProjectOp::Move { item, to } => {
            let mut tree = config.tree();
            let change = tree.plan_move(item, to)?;
            tree.apply(&change);
            write_tree(config, tree);
            Ok(())
        }
        ProjectOp::Delete { item } => {
            let mut tree = config.tree();
            match tree.plan_remove(item) {
                Ok(removal) => {
                    tree.apply_removal(&removal);
                    write_tree(config, tree);
                    Ok(())
                }
                Err(TreeError::UnknownItem { .. }) => Ok(()),
                Err(error) => Err(error.into()),
            }
        }
        ProjectOp::SaveRequest { api, request } => {
            let found = config
                .apis
                .iter_mut()
                .find(|found| &found.id == api)
                .ok_or_else(|| ApplyError::MissingApi { id: api.clone() })?;
            found.request = request.clone();
            Ok(())
        }
        ProjectOp::FolderSettings { folder, settings } => {
            let found = config
                .folders
                .iter_mut()
                .find(|found| &found.id == folder)
                .ok_or_else(|| ApplyError::MissingFolder { id: folder.clone() })?;
            found.settings = settings.clone();
            Ok(())
        }
        ProjectOp::ProjectSettings { settings } => {
            config.settings = settings.clone();
            Ok(())
        }
        ProjectOp::Variables {
            variables,
            environments,
        } => {
            config.variables = variables.clone();
            config.environments = environments.clone();
            Ok(())
        }
        ProjectOp::Describe { name, description } => {
            config.name = name.clone();
            config.description = description.clone();
            Ok(())
        }
    }
}

fn item_exists(config: &ProjectConfig, id: &ItemId) -> bool {
    config.folders.iter().any(|folder| &folder.id == id) || config.apis.iter().any(|api| &api.id == id)
}

fn place_new(config: &mut ProjectConfig, item: &ItemId, parent: Option<&ItemId>) -> Result<(), ApplyError> {
    let mut tree = config.tree();
    let change = tree.plan_insert(item, parent)?;
    tree.apply(&change);
    write_tree(config, tree);
    Ok(())
}

fn write_tree(config: &mut ProjectConfig, tree: TreeData) {
    config.folders = tree.folders;
    config.apis = tree.apis;
    config.top_order = tree.top_order;
    config.settings = tree.settings;
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

    fn api(id: &str, body: &str) -> ApiSetting {
        ApiSetting {
            id: ItemId::from(id),
            name: id.into(),
            request: HttpRequest {
                method: HttpMethod::Post,
                uri: format!("/{id}"),
                body: body.into(),
                ..HttpRequest::default()
            },
            ..ApiSetting::default()
        }
    }

    fn project() -> ProjectConfig {
        ProjectConfig {
            name: "shop".into(),
            apis: vec![api("login", "secret-body"), api("health", "ok-body")],
            ..ProjectConfig::default()
        }
    }

    fn body(config: &ProjectConfig, id: &str) -> String {
        config
            .apis
            .iter()
            .find(|api| api.id.as_str() == id)
            .expect(id)
            .request
            .body
            .clone()
    }

    #[test]
    fn a_saved_request_leaves_every_other_body() {
        let mut config = project();
        let mut changed = api("login", "secret-body").request;
        changed.body = "edited".into();
        apply_changes(
            &mut config,
            &ProjectChanges {
                ops: vec![ProjectOp::SaveRequest {
                    api: ItemId::from("login"),
                    request: changed,
                }],
            },
        )
        .expect("apply");
        assert_eq!(body(&config, "login"), "edited");
        assert_eq!(body(&config, "health"), "ok-body", "never opened, so it stays");
    }

    #[test]
    fn creating_and_deleting_do_not_drop_an_unmentioned_api() {
        let mut config = project();
        apply_changes(
            &mut config,
            &ProjectChanges {
                ops: vec![
                    ProjectOp::CreateApi {
                        parent: None,
                        api: api("extra", "new-body"),
                    },
                    ProjectOp::Delete {
                        item: ItemId::from("login"),
                    },
                ],
            },
        )
        .expect("apply");
        assert!(config.apis.iter().all(|api| api.id.as_str() != "login"));
        assert_eq!(body(&config, "health"), "ok-body");
        assert_eq!(body(&config, "extra"), "new-body");
        assert_eq!(config.top_order.first().map(|id| id.as_str()), Some("extra"));
    }

    #[test]
    fn deleting_what_is_already_gone_changes_nothing() {
        let mut config = project();
        apply_changes(
            &mut config,
            &ProjectChanges {
                ops: vec![ProjectOp::Delete {
                    item: ItemId::from("missing"),
                }],
            },
        )
        .expect("a delete of nothing");
        assert_eq!(config.apis.len(), 2);
    }

    #[test]
    fn saving_a_missing_api_writes_nothing() {
        let mut config = project();
        let error = apply_changes(
            &mut config,
            &ProjectChanges {
                ops: vec![ProjectOp::SaveRequest {
                    api: ItemId::from("missing"),
                    request: HttpRequest::default(),
                }],
            },
        )
        .expect_err("missing");
        assert!(matches!(error, ApplyError::MissingApi { .. }));
        assert_eq!(body(&config, "login"), "secret-body", "the failure is before any write");
    }

    #[test]
    fn a_blank_rename_is_refused() {
        let mut config = project();
        let error = apply_changes(
            &mut config,
            &ProjectChanges {
                ops: vec![ProjectOp::Rename {
                    item: ItemId::from("login"),
                    name: "  ".into(),
                }],
            },
        )
        .expect_err("blank");
        assert!(matches!(error, ApplyError::NameRequired));
    }

    #[test]
    fn an_outline_stub_keeps_the_method_and_drops_the_body() {
        let head = ApiHead::from_api(&api("login", "secret-body"));
        let stub = head.stub(&CollectionId::from("shop"));
        assert_eq!(stub.request.method, HttpMethod::Post);
        assert_eq!(stub.request.uri, "/login");
        assert!(stub.request.body.is_empty());
        let outline = ProjectOutline {
            name: "shop".into(),
            apis: vec![head],
            ..ProjectOutline::default()
        };
        let config = outline.config(&CollectionId::from("shop"));
        assert_eq!(config.apis.len(), 1);
        assert!(config.apis[0].request.body.is_empty());
    }
}
