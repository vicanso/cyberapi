//! The storage seam (L1 in `docs/architecture.md` §4): the traits every
//! backend implements, so the layers above never know which one they hold.
//!
//! | Backend | Where |
//! |---|---|
//! | `TursoStore` (`cyberapi-store-turso`) | the desktop |
//! | `SqlStore` (`cyberapi-store-sql`) | the bridge, over PostgreSQL / MySQL |
//! | `BridgeStore` (`cyberapi-web`) | the browser, through the bridge |
//! | [`MemoryStore`] | tests |
//!
//! The rules every backend keeps are written once, as the `conformance`
//! suite (feature `conformance`), and each backend's tests run it.
//!
//! Every method takes owned arguments and returns a `'static` future, so a
//! call can be handed to gpui's background executor as it is, and every
//! argument and answer can cross HTTP. Every change to a project's content
//! names the project, and a backend refuses an item, variable or folder of
//! another project as if it did not exist.

#[cfg(feature = "conformance")]
pub mod conformance;
mod error;
#[cfg(not(target_family = "wasm"))]
mod memory;
pub mod rules;

pub use error::{Entity, Refusal, StoreError};
#[cfg(not(target_family = "wasm"))]
pub use memory::MemoryStore;

use cyberapi_core::access::{CollectionRole, Grantee, TeamRole};
use cyberapi_core::backup::Backup;
use cyberapi_core::cookie::Cookie;
use cyberapi_core::directory::{AccountInfo, Grant, Me, Member, Team};
use cyberapi_core::import::ImportItem;
use cyberapi_core::model::{ApiSetting, Collection, Folder, HttpRequest, Variable, VariableCategory};
use cyberapi_core::prefs::{AppPrefs, WorkspacePrefs};
use cyberapi_core::response::HttpResponse;
use cyberapi_core::tree::{Placement, Removal, TreeChange, TreeData};
use cyberapi_core::{AccountId, CollectionId, ItemId, TeamId, VariableId};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// What every store method returns. Boxed, because the stores are used as
/// `Arc<dyn …>` — chosen at run time — and an `async fn` in a trait cannot be.
pub type StoreFuture<T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'static>>;

/// The fields a new project is made from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewCollection {
    pub name: String,
    pub description: String,
    /// The team it belongs to in the browser build; `None` is the reader's
    /// own space there, and the only choice on the desktop.
    pub owner: Option<TeamId>,
}

/// The fields of a project its readers may change.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionEdit {
    pub name: String,
    pub description: String,
}

/// What an import wrote (M10): every folder and API with its new id, in the
/// order written, and the lists that changed to place them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub folders: Vec<Folder>,
    pub apis: Vec<ApiSetting>,
    pub change: TreeChange,
}

/// A new tree item, and where the store put it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inserted<T> {
    pub item: T,
    /// The lists that changed to place it ([`TreeData::plan_insert`]), for the
    /// caller to apply to its copy.
    pub change: TreeChange,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewVariable {
    pub collection: CollectionId,
    pub category: VariableCategory,
    pub name: String,
    pub value: String,
    pub enabled: bool,
}

/// The fields of a variable that change; its project and category do not.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableEdit {
    pub name: String,
    pub value: String,
    pub enabled: bool,
}

/// Projects (M2).
pub trait CollectionStore: Send + Sync {
    /// The projects the reader can see, in store order; each carries the
    /// reader's role.
    fn list(&self) -> StoreFuture<Vec<Collection>>;
    fn get(&self, id: CollectionId) -> StoreFuture<Collection>;
    /// A blank name is refused (F2.2).
    fn create(&self, input: NewCollection) -> StoreFuture<Collection>;
    fn update(&self, id: CollectionId, edit: CollectionEdit) -> StoreFuture<Collection>;
    /// Removes the project and everything in it: folders, APIs, variables
    /// and the readers' workspace state (#5 — the old app kept the variables).
    fn delete(&self, id: CollectionId) -> StoreFuture<()>;
}

/// A project's folders and APIs (M3, F4.1). Each change follows the plan
/// `cyberapi_core::tree` makes for it and moves the project's `updated_at`.
pub trait TreeStore: Send + Sync {
    fn load(&self, collection: CollectionId) -> StoreFuture<TreeData>;
    /// A folder at the end of `parent`, or first on the top level (F3.4).
    fn create_folder(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<Folder>>;
    /// An HTTP API with an empty request, placed like a folder (F3.4).
    fn create_api(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<ApiSetting>>;
    /// Renames a folder or an API (F3.5). A blank name is refused.
    fn rename(&self, collection: CollectionId, item: ItemId, name: String) -> StoreFuture<()>;
    /// Replaces an API's request (F4.11).
    fn save_request(&self, collection: CollectionId, api: ItemId, request: HttpRequest) -> StoreFuture<ApiSetting>;
    /// Moves an item (F3.7); answers what changed.
    fn move_item(&self, collection: CollectionId, item: ItemId, to: Placement) -> StoreFuture<TreeChange>;
    /// Adds imported folders and APIs (F10.4) at the end of `parent`, or of
    /// the top level (#9); every one gets a new id (#7).
    fn import(&self, collection: CollectionId, parent: Option<ItemId>, items: Vec<ImportItem>)
    -> StoreFuture<Imported>;
    /// Deletes an item, and for a folder all it contains (F3.6), with the
    /// response history of every API that goes.
    fn delete(&self, collection: CollectionId, item: ItemId) -> StoreFuture<Removal>;
}

/// The responses each API keeps (F7.9 – F7.11): the newest
/// `cyberapi_core::response::HISTORY_LIMIT`, newest first. A reader's own,
/// like their prefs — in the browser build every account keeps its own — and
/// not a change to the project: it moves neither `updated_at` nor the
/// revision.
pub trait HistoryStore: Send + Sync {
    /// Keeps a response of `response.api` in `response.collection`, giving it
    /// an id and the time; the oldest beyond the limit go.
    fn add(&self, response: HttpResponse) -> StoreFuture<HttpResponse>;
    fn list(&self, collection: CollectionId, api: ItemId) -> StoreFuture<Vec<HttpResponse>>;
    /// The newest response of each of `apis` that has one, in no particular
    /// order; an id that is not one of the project's APIs is skipped. For the
    /// `get` template function (M6).
    fn latest(&self, collection: CollectionId, apis: Vec<ItemId>) -> StoreFuture<Vec<HttpResponse>>;
    fn clear(&self, collection: CollectionId, api: ItemId) -> StoreFuture<()>;
}

/// Environment variables, custom variables and global request headers (M8).
pub trait VariableStore: Send + Sync {
    /// Sorted by name; equal names keep store order.
    fn list(&self, collection: CollectionId, category: VariableCategory) -> StoreFuture<Vec<Variable>>;
    fn create(&self, input: NewVariable) -> StoreFuture<Variable>;
    fn update(&self, collection: CollectionId, id: VariableId, edit: VariableEdit) -> StoreFuture<Variable>;
    /// Ids that are not the project's are ignored.
    fn delete(&self, collection: CollectionId, ids: Vec<VariableId>) -> StoreFuture<()>;
}

/// What the reader keeps for themselves (4.6 / 4.7). A document never
/// written reads as its defaults.
pub trait PrefStore: Send + Sync {
    fn app(&self) -> StoreFuture<AppPrefs>;
    fn set_app(&self, prefs: AppPrefs) -> StoreFuture<()>;
    fn workspace(&self, collection: CollectionId) -> StoreFuture<WorkspacePrefs>;
    fn set_workspace(&self, collection: CollectionId, prefs: WorkspacePrefs) -> StoreFuture<()>;
}

/// The reader's cookies (M9): every project's requests share them (F9.1);
/// in the browser build each account has its own. A cookie is known by its
/// domain, path and name.
pub trait CookieStore: Send + Sync {
    /// By name, then domain, then path (F9.4); expired ones included until
    /// something removes them.
    fn list(&self) -> StoreFuture<Vec<Cookie>>;
    /// Keeps each cookie in place of the one with its key; an expired one
    /// removes that one instead (F9.2).
    fn put(&self, cookies: Vec<Cookie>) -> StoreFuture<()>;
    /// F9.7. A cookie that is not kept is no error.
    fn delete(&self, domain: String, path: String, name: String) -> StoreFuture<()>;
    fn clear(&self) -> StoreFuture<()>;
}

/// Every project at once (M13): the desktop's backup and restore. Not part
/// of [`Storage`] — the browser build has none — so the desktop holds it
/// beside.
pub trait BackupStore: Send + Sync {
    /// Every project, folder, API and variable, and the top-level orders,
    /// in store order.
    fn snapshot(&self) -> StoreFuture<Backup>;
    /// Replaces them all with `backup` at once: a restore that fails leaves
    /// everything as it was (#8). The responses kept for APIs go too.
    fn restore(&self, backup: Backup) -> StoreFuture<()>;
}

/// The order cookies are listed in (F9.4).
pub fn sort_cookies(cookies: &mut [Cookie]) {
    cookies.sort_by(|a, b| (&a.name, &a.domain, &a.path).cmp(&(&b.name, &b.domain, &b.path)));
}

/// An account an administrator makes (§9.5). The password is sent once, to
/// be hashed by the store; it is never read back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInvite {
    pub name: String,
    pub display_name: String,
    pub password: String,
    pub admin: bool,
}

/// What an administrator may change about an account.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountEdit {
    pub display_name: String,
    pub admin: bool,
    pub disabled: bool,
}

/// Accounts, teams and who has which project (`docs/architecture.md` §9.4):
/// the browser build's only — the bridge's `SqlStore` answers it, for the
/// signed-in reader, and the page's `BridgeStore` asks. Not in [`Storage`]:
/// the desktop has no accounts.
///
/// A team the reader is not in is "not found", as a project is; a change
/// the reader's place does not allow is [`StoreError::Refused`].
pub trait DirectoryStore: Send + Sync {
    fn me(&self) -> StoreFuture<Me>;
    /// Every account, by name: whom a team or a project can be given to.
    fn accounts(&self) -> StoreFuture<Vec<AccountInfo>>;
    /// A team, with the reader as its owner.
    fn create_team(&self, name: String) -> StoreFuture<Team>;
    /// Its owners and admins; not a personal space.
    fn rename_team(&self, team: TeamId, name: String) -> StoreFuture<()>;
    /// Its owners, once it owns no project.
    fn delete_team(&self, team: TeamId) -> StoreFuture<()>;
    /// The reader must be in it.
    fn members(&self, team: TeamId) -> StoreFuture<Vec<Member>>;
    /// Adds, changes or (`None`) removes a member: the team's owners and
    /// admins, though only an owner makes or touches an owner, and anyone
    /// may leave. The last owner stays.
    fn set_member(&self, team: TeamId, account: AccountId, role: Option<TeamRole>) -> StoreFuture<()>;
    /// The project's grants, for its managers.
    fn grants(&self, collection: CollectionId) -> StoreFuture<Vec<Grant>>;
    /// Gives the project to an account or a team, or (`None`) takes it back;
    /// its managers.
    fn set_grant(&self, collection: CollectionId, grantee: Grantee, role: Option<CollectionRole>) -> StoreFuture<()>;
    /// Moves the project to another team the reader is in; its managers.
    fn transfer(&self, collection: CollectionId, team: TeamId) -> StoreFuture<()>;
    /// Administrators: a new account, with its own space.
    fn create_account(&self, invite: AccountInvite) -> StoreFuture<AccountInfo>;
    /// Administrators. Disabling signs the account out.
    fn update_account(&self, account: AccountId, edit: AccountEdit) -> StoreFuture<AccountInfo>;
    /// Administrators; signs the account out.
    fn reset_password(&self, account: AccountId, password: String) -> StoreFuture<()>;
    /// The reader's own, given the one it has now.
    fn change_password(&self, current: String, password: String) -> StoreFuture<()>;
}

/// One backend's stores, as the layers above hold them.
#[derive(Clone)]
pub struct Storage {
    collections: Arc<dyn CollectionStore>,
    tree: Arc<dyn TreeStore>,
    variables: Arc<dyn VariableStore>,
    prefs: Arc<dyn PrefStore>,
    history: Arc<dyn HistoryStore>,
    cookies: Arc<dyn CookieStore>,
}

impl Storage {
    pub fn new(
        collections: Arc<dyn CollectionStore>,
        tree: Arc<dyn TreeStore>,
        variables: Arc<dyn VariableStore>,
        prefs: Arc<dyn PrefStore>,
        history: Arc<dyn HistoryStore>,
        cookies: Arc<dyn CookieStore>,
    ) -> Self {
        Self {
            collections,
            tree,
            variables,
            prefs,
            history,
            cookies,
        }
    }

    /// Every store from one backend that implements them all.
    pub fn from_backend<B>(backend: Arc<B>) -> Self
    where
        B: CollectionStore + TreeStore + VariableStore + PrefStore + HistoryStore + CookieStore + 'static,
    {
        Self::new(
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend,
        )
    }

    pub fn collections(&self) -> &dyn CollectionStore {
        self.collections.as_ref()
    }

    pub fn tree(&self) -> &dyn TreeStore {
        self.tree.as_ref()
    }

    pub fn variables(&self) -> &dyn VariableStore {
        self.variables.as_ref()
    }

    pub fn prefs(&self) -> &dyn PrefStore {
        self.prefs.as_ref()
    }

    pub fn history(&self) -> &dyn HistoryStore {
        self.history.as_ref()
    }

    pub fn cookies(&self) -> &dyn CookieStore {
        self.cookies.as_ref()
    }
}
