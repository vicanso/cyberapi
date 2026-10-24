//! The storage seam (L1 in `docs/architecture.md` §4): the traits every
//! backend implements, so the layers above never know which one they hold.
//!
//! | Backend | Where |
//! |---|---|
//! | `TursoStore` (`cyberapi-store-turso`) | the desktop |
//! | `SqlStore` (`cyberapi-store-sql`) | the bridge, over PostgreSQL / MySQL |
//! | `BridgeStore` (`cyberapi-web`) | the browser, through the bridge |
//! | [`MemoryStore`] | tests; a page's working copy of a server project |
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
mod memory;
pub mod rules;

pub use error::{Entity, Refusal, StoreError};
pub use memory::MemoryStore;

use cyberapi_core::access::{CollectionRole, Grantee, TeamRole};
use cyberapi_core::backup::Backup;
use cyberapi_core::cookie::Cookie;
use cyberapi_core::directory::{
    AccountInfo, Grant, Invite, IssuedMcpToken, McpToken, Me, Member, NewInvite, NewMcpToken, Team, TeamPlace,
};
use cyberapi_core::import::ImportItem;
use cyberapi_core::model::{ApiSetting, Collection, Folder, HttpRequest, Variable, VariableCategory};
use cyberapi_core::prefs::{AppPrefs, WorkspacePrefs};
use cyberapi_core::response::HttpResponse;
use cyberapi_core::sync::{ConfigAt, CopyBinding, ProjectConfig};
use cyberapi_core::tree::{Placement, Removal, TreeChange, TreeData};
use cyberapi_core::{AccountId, CollectionId, InviteId, ItemId, McpTokenId, TeamId, VariableId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    /// How many APIs each project the reader can see holds, in its folders
    /// too, for the project list; a project with none is not there.
    fn api_counts(&self) -> StoreFuture<BTreeMap<CollectionId, usize>>;
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
    /// The project's configuration, whole, and the revision it is at: what a
    /// copy of a server project pulls (`docs/server-sync.md` §3.1).
    fn config(&self, collection: CollectionId) -> StoreFuture<ConfigAt>;
    /// Puts `config` in place of the project's at once (server-sync §3.3).
    /// Its folders, APIs and variables keep the ids they bring — an id
    /// another project has is [`StoreError::IdTaken`] — and what the project
    /// has and `config` does not goes, with the history of the APIs that go;
    /// what stays keeps its history. Only when the project is still at
    /// revision `base`: otherwise nothing is written and the answer is
    /// [`StoreError::Stale`]. `None` replaces whatever it is at ("mine
    /// over the server's"). The revision moves on once; the answer is the
    /// new one. Editors and up; the name and description are the managers'
    /// to change, and stay as they are for anyone else.
    fn replace(&self, collection: CollectionId, config: ProjectConfig, base: Option<u64>) -> StoreFuture<u64>;
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
    /// Every response the reader keeps, of every project (F15.3).
    fn clear_all(&self) -> StoreFuture<()>;
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
    /// Forgets the reader's state of every project — pins, open folders,
    /// the selection, tabs (F15.3). The app's settings stay.
    fn clear_workspaces(&self) -> StoreFuture<()>;
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

/// The desktop's copies of server projects (`docs/server-sync.md` §6):
/// which of its projects are whose server's, and keeping them as the server
/// has them. A copy is a project like any other in the other stores — sent
/// from, with its history and cookies here. Not part of [`Storage`]: only
/// the desktop keeps copies, beside its own projects.
pub trait ServerCopyStore: Send + Sync {
    /// Every copy's binding, of every account.
    fn copies(&self) -> StoreFuture<Vec<CopyBinding>>;
    /// In place of the project's binding, if it had one.
    fn bind(&self, binding: CopyBinding) -> StoreFuture<()>;
    /// The project is no copy any more; one that was not is no error.
    fn unbind(&self, collection: CollectionId) -> StoreFuture<()>;
    /// The server's `collection`, with `config` as all it has: made here,
    /// with the server's id, when it is not. As [`TreeStore::replace`]
    /// without a base: ids are kept, another project's are refused, and
    /// what stays keeps its history. The copy's revision is its own.
    fn keep_copy(&self, collection: Collection, config: ProjectConfig) -> StoreFuture<()>;
}

/// The order cookies are listed in (F9.4).
pub fn sort_cookies(cookies: &mut [Cookie]) {
    cookies.sort_by(|a, b| (&a.name, &a.domain, &a.path).cmp(&(&b.name, &b.domain, &b.path)));
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
    /// The invitations the reader sent that can still be used
    /// (`docs/server-sync.md` §5), oldest first.
    fn invites(&self) -> StoreFuture<Vec<Invite>>;
    /// An invitation for one new account, used once within
    /// `cyberapi_core::directory::INVITE_DAYS`: to a team the reader may
    /// invite into (`invite_needs`), or — `None`, administrators only — to
    /// an account with only its own space. No one joins a personal space.
    fn create_invite(&self, place: Option<TeamPlace>) -> StoreFuture<NewInvite>;
    /// Its sender takes it back before it is used.
    fn revoke_invite(&self, id: InviteId) -> StoreFuture<()>;
    /// Administrators. Disabling signs the account out.
    fn update_account(&self, account: AccountId, edit: AccountEdit) -> StoreFuture<AccountInfo>;
    /// Administrators; signs the account out.
    fn reset_password(&self, account: AccountId, password: String) -> StoreFuture<()>;
    /// The reader's own, given the one it has now.
    fn change_password(&self, current: String, password: String) -> StoreFuture<()>;
    /// The reader's MCP tokens (an AI assistant's way in at the bridge's
    /// `/v1/mcp`) that still sign in, oldest first.
    fn mcp_tokens(&self) -> StoreFuture<Vec<McpToken>>;
    /// A new one for the reader; its secret is answered this once, and the
    /// store keeps only its hash.
    fn issue_mcp_token(&self, input: NewMcpToken) -> StoreFuture<IssuedMcpToken>;
    /// One of the reader's, which signs in no more.
    fn revoke_mcp_token(&self, id: McpTokenId) -> StoreFuture<()>;
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

    /// These stores with a project's tree, variables and history answered
    /// elsewhere: a page's working copy of a server project
    /// (`docs/server-sync.md` §4).
    pub fn with_project(
        &self,
        tree: Arc<dyn TreeStore>,
        variables: Arc<dyn VariableStore>,
        history: Arc<dyn HistoryStore>,
    ) -> Self {
        Self {
            tree,
            variables,
            history,
            ..self.clone()
        }
    }

    /// These stores with a project's tree and variables answered
    /// elsewhere: the desktop's copy of a server project, whose changes are
    /// marked unsynced as they land (`docs/server-sync.md` §6).
    pub fn with_content(&self, tree: Arc<dyn TreeStore>, variables: Arc<dyn VariableStore>) -> Self {
        Self {
            tree,
            variables,
            ..self.clone()
        }
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
