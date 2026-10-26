//! The store over the bridge (`docs/architecture.md` §2.2, §4.1): one wire
//! protocol, both ends of it.
//!
//! Every Store trait method is one [`Call`]. The browser's [`BridgeStore`]
//! turns a method into a call and posts it to [`STORE_PATH`]; the bridge
//! turns the call back into the method with [`dispatch`], on the store scoped
//! to the signed-in account, and answers with a [`Reply`]. A store refusal
//! (not found, forbidden, …) is an answer like any other, sent as it was
//! made; HTTP's own statuses are left for the transport — 401 when nobody is
//! signed in.
//!
//! The protocol grows with the traits and with nothing else: a new method is
//! a new variant here, matched in [`dispatch`] and made in [`BridgeStore`].
//! The directory of accounts and teams (`DirectoryStore`) rides the same
//! way, its calls prefixed `directory`.
//!
//! Sending a request is the other half (`docs/architecture.md` §5, D5): the
//! page prepares it with the shared pipeline and [`BridgeHttpEngine`] posts
//! it to [`SEND_PATH`], where the bridge's engine sends it and answers with a
//! [`SendReply`]. A watched send (`HttpEngine::send_watched`) goes to
//! [`SEND_STREAM_PATH`] instead, whose answer comes as it goes: one
//! [`StreamReply`] a line — an event stream's head and pieces, then the
//! exchange. The page stops a stream by leaving: what came by then is the
//! exchange.

use cyberapi_core::access::{CollectionRole, Grantee, TeamRole};
use cyberapi_core::cookie::Cookie;
use cyberapi_core::directory::{
    AccountInfo, Grant, Invite, IssuedMcpToken, McpToken, Me, Member, NewInvite, NewMcpToken, Team, TeamPlace,
};
use cyberapi_core::import::ImportItem;
use cyberapi_core::model::{
    ApiSetting, Collection, Environment, Folder, FolderSettings, HttpRequest, ProjectSettings, Variable,
    VariableCategory,
};
use cyberapi_core::prefs::{AppPrefs, WorkspacePrefs};
use cyberapi_core::response::{HISTORY_LIMIT, HISTORY_SIZES, HttpResponse, RECENT_LIMIT};
use cyberapi_core::response::{StreamEnd, StreamInfo};
use cyberapi_core::sync::{ConfigAt, ProjectConfig};
use cyberapi_core::tree::{Placement, Removal, TreeChange, TreeData};
use cyberapi_core::{AccountId, CollectionId, EnvironmentId, InviteId, ItemId, McpTokenId, TeamId, VariableId};
use cyberapi_http::{Exchange, HttpEngine, PreparedRequest, Progress, SendError, SendFuture, Watch};
use cyberapi_store::{
    AccountEdit, CollectionEdit, CollectionStore, CookieStore, DirectoryStore, HistoryStore, Imported, Inserted,
    NewCollection, NewVariable, PrefStore, Storage, StoreError, StoreFuture, TreeStore, VariableEdit, VariableStore,
};
use futures::{Stream, StreamExt as _};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

mod client;

pub use client::{
    BearerSession, EngineTransport, LOGIN_PATH, LOGOUT_PATH, LoginForm, SignInError, bridge_address, session_me,
    sign_in, sign_out,
};

/// Where calls are posted.
pub const STORE_PATH: &str = "/v1/store";

/// The header a client names its `cyberapi_core::version::MODEL` in, and the
/// bridge its own in its answers.
pub const MODEL_HEADER: &str = "x-cyberapi-model";

/// The header a client sends with each post: its model.
pub fn model_header() -> (&'static str, String) {
    (MODEL_HEADER, cyberapi_core::version::MODEL.to_string())
}

/// Whether the bridge, keeping projects as `bridge` (its model), refuses
/// `call` from a client that keeps them as `client` — `None` from one
/// that did not say, which keeps them as the first did: a change to a
/// project's content from an older client would drop what it cannot read.
/// Reading is never refused, nor what is the reader's own (prefs, history,
/// cookies).
pub fn refused_as_too_old(call: &Call, client: Option<u32>, bridge: u32) -> bool {
    call.changes_project() && client.unwrap_or(1) < bridge
}
/// Where requests to send are posted.
pub const SEND_PATH: &str = "/v1/send";

/// Where a watched send goes; its answer is NDJSON, one [`StreamReply`] a
/// line.
pub const SEND_STREAM_PATH: &str = "/v1/send/stream";

/// One line of a watched send's answer: what the engine told as the event
/// stream came, then how the send ended — the exchange's body left out
/// when it came in pieces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamReply {
    Progress(Progress),
    Done(SendReply),
}

impl StreamReply {
    /// The reply as its line.
    pub fn line(&self) -> Vec<u8> {
        // Strings, numbers and bytes as base64 cannot fail to serialize.
        let mut line = serde_json::to_vec(self).unwrap_or_default();
        line.push(b'\n');
        line
    }
}

/// What sending answers: the exchange, or why there was none. Boxed, the
/// exchange is as big as the reason; the JSON is the same.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SendReply {
    Ok(Box<Exchange>),
    Err(SendError),
}

fn history_limit() -> usize {
    HISTORY_LIMIT
}

/// One Store trait method and its arguments.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "call", content = "args", rename_all = "camelCase")]
pub enum Call {
    CollectionsList,
    CollectionsGet {
        id: CollectionId,
    },
    CollectionsCreate {
        input: NewCollection,
    },
    CollectionsUpdate {
        id: CollectionId,
        edit: CollectionEdit,
    },
    CollectionsDelete {
        id: CollectionId,
    },
    CollectionsApiCounts,
    TreeLoad {
        collection: CollectionId,
    },
    TreeCreateFolder {
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    },
    TreeCreateApi {
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    },
    TreeRename {
        collection: CollectionId,
        item: ItemId,
        name: String,
    },
    TreeSaveRequest {
        collection: CollectionId,
        api: ItemId,
        request: HttpRequest,
    },
    TreeSetFolderSettings {
        collection: CollectionId,
        folder: ItemId,
        settings: FolderSettings,
    },
    TreeSetProjectSettings {
        collection: CollectionId,
        settings: ProjectSettings,
    },
    TreeMove {
        collection: CollectionId,
        item: ItemId,
        to: Placement,
    },
    TreeImport {
        collection: CollectionId,
        parent: Option<ItemId>,
        items: Vec<ImportItem>,
    },
    TreeDelete {
        collection: CollectionId,
        item: ItemId,
    },
    TreeConfig {
        collection: CollectionId,
    },
    TreeReplace {
        collection: CollectionId,
        config: ProjectConfig,
        base: Option<u64>,
    },
    VariablesList {
        collection: CollectionId,
        category: VariableCategory,
    },
    VariablesCreate {
        input: NewVariable,
    },
    VariablesUpdate {
        collection: CollectionId,
        id: VariableId,
        edit: VariableEdit,
    },
    VariablesDelete {
        collection: CollectionId,
        ids: Vec<VariableId>,
    },
    EnvironmentsList {
        collection: CollectionId,
    },
    EnvironmentsCreate {
        collection: CollectionId,
        name: String,
    },
    EnvironmentsRename {
        collection: CollectionId,
        id: EnvironmentId,
        name: String,
    },
    EnvironmentsDelete {
        collection: CollectionId,
        id: EnvironmentId,
    },
    EnvironmentsReorder {
        collection: CollectionId,
        order: Vec<EnvironmentId>,
    },
    PrefsApp,
    PrefsSetApp {
        prefs: AppPrefs,
    },
    PrefsWorkspace {
        collection: CollectionId,
    },
    PrefsSetWorkspace {
        collection: CollectionId,
        prefs: WorkspacePrefs,
    },
    PrefsClearWorkspaces,
    HistoryAdd {
        // Boxed: a response is far bigger than any other call.
        response: Box<HttpResponse>,
        /// D4: how many the API keeps. A page from before sends none: the
        /// default.
        #[serde(default = "history_limit")]
        keep: usize,
    },
    HistoryList {
        collection: CollectionId,
        api: ItemId,
    },
    HistoryLatest {
        collection: CollectionId,
        apis: Vec<ItemId>,
    },
    HistoryRecent {
        collection: CollectionId,
        limit: usize,
    },
    HistoryClear {
        collection: CollectionId,
        api: ItemId,
    },
    HistoryClearAll,
    CookiesList,
    CookiesPut {
        cookies: Vec<Cookie>,
    },
    CookiesDelete {
        domain: String,
        path: String,
        name: String,
    },
    CookiesClear,
    DirectoryMe,
    DirectoryAccounts,
    DirectoryCreateTeam {
        name: String,
    },
    DirectoryRenameTeam {
        team: TeamId,
        name: String,
    },
    DirectoryDeleteTeam {
        team: TeamId,
    },
    DirectoryMembers {
        team: TeamId,
    },
    DirectorySetMember {
        team: TeamId,
        account: AccountId,
        role: Option<TeamRole>,
    },
    DirectoryGrants {
        collection: CollectionId,
    },
    DirectorySetGrant {
        collection: CollectionId,
        grantee: Grantee,
        role: Option<CollectionRole>,
    },
    DirectoryTransfer {
        collection: CollectionId,
        team: TeamId,
    },
    DirectoryInvites,
    DirectoryCreateInvite {
        place: Option<TeamPlace>,
    },
    DirectoryRevokeInvite {
        id: InviteId,
    },
    DirectoryUpdateAccount {
        account: AccountId,
        edit: AccountEdit,
    },
    DirectoryResetPassword {
        account: AccountId,
        password: String,
    },
    DirectoryChangePassword {
        current: String,
        password: String,
    },
    DirectoryMcpTokens,
    DirectoryIssueMcpToken {
        input: NewMcpToken,
    },
    DirectoryRevokeMcpToken {
        id: McpTokenId,
    },
}

impl Call {
    /// Whether it changes a project's content — its tree, requests or
    /// variables, what a client of an older model would write without what
    /// it cannot read. A project's name and its being there are not that
    /// (`docs/batch4-design.md` §2.1).
    pub fn changes_project(&self) -> bool {
        match self {
            Self::TreeCreateFolder { .. }
            | Self::TreeCreateApi { .. }
            | Self::TreeRename { .. }
            | Self::TreeSaveRequest { .. }
            | Self::TreeSetFolderSettings { .. }
            | Self::TreeSetProjectSettings { .. }
            | Self::TreeMove { .. }
            | Self::TreeImport { .. }
            | Self::TreeDelete { .. }
            | Self::TreeReplace { .. }
            | Self::VariablesCreate { .. }
            | Self::VariablesUpdate { .. }
            | Self::VariablesDelete { .. }
            | Self::EnvironmentsCreate { .. }
            | Self::EnvironmentsRename { .. }
            | Self::EnvironmentsDelete { .. }
            | Self::EnvironmentsReorder { .. } => true,
            Self::EnvironmentsList { .. } => false,
            Self::CollectionsList
            | Self::CollectionsGet { .. }
            | Self::CollectionsCreate { .. }
            | Self::CollectionsUpdate { .. }
            | Self::CollectionsDelete { .. }
            | Self::CollectionsApiCounts
            | Self::TreeLoad { .. }
            | Self::TreeConfig { .. }
            | Self::VariablesList { .. }
            | Self::PrefsApp
            | Self::PrefsSetApp { .. }
            | Self::PrefsWorkspace { .. }
            | Self::PrefsSetWorkspace { .. }
            | Self::PrefsClearWorkspaces
            | Self::HistoryAdd { .. }
            | Self::HistoryList { .. }
            | Self::HistoryLatest { .. }
            | Self::HistoryRecent { .. }
            | Self::HistoryClear { .. }
            | Self::HistoryClearAll
            | Self::CookiesList
            | Self::CookiesPut { .. }
            | Self::CookiesDelete { .. }
            | Self::CookiesClear
            | Self::DirectoryMe
            | Self::DirectoryAccounts
            | Self::DirectoryCreateTeam { .. }
            | Self::DirectoryRenameTeam { .. }
            | Self::DirectoryDeleteTeam { .. }
            | Self::DirectoryMembers { .. }
            | Self::DirectorySetMember { .. }
            | Self::DirectoryGrants { .. }
            | Self::DirectorySetGrant { .. }
            | Self::DirectoryTransfer { .. }
            | Self::DirectoryInvites
            | Self::DirectoryCreateInvite { .. }
            | Self::DirectoryRevokeInvite { .. }
            | Self::DirectoryUpdateAccount { .. }
            | Self::DirectoryResetPassword { .. }
            | Self::DirectoryChangePassword { .. }
            | Self::DirectoryMcpTokens
            | Self::DirectoryIssueMcpToken { .. }
            | Self::DirectoryRevokeMcpToken { .. } => false,
        }
    }
}

/// What a call answers: the method's value, or the store's refusal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reply {
    Ok(serde_json::Value),
    Err(StoreError),
}

/// Runs `call` on `storage`, or on `directory` for the directory's calls —
/// the bridge's side.
pub async fn dispatch(storage: &Storage, directory: &dyn DirectoryStore, call: Call) -> Reply {
    fn reply<T: Serialize>(result: Result<T, StoreError>) -> Reply {
        match result.and_then(|value| serde_json::to_value(value).map_err(StoreError::backend)) {
            Ok(value) => Reply::Ok(value),
            Err(error) => Reply::Err(error),
        }
    }
    let (collections, tree, variables, prefs, history, cookies) = (
        storage.collections(),
        storage.tree(),
        storage.variables(),
        storage.prefs(),
        storage.history(),
        storage.cookies(),
    );
    match call {
        Call::CollectionsList => reply(collections.list().await),
        Call::CollectionsGet { id } => reply(collections.get(id).await),
        Call::CollectionsCreate { input } => reply(collections.create(input).await),
        Call::CollectionsUpdate { id, edit } => reply(collections.update(id, edit).await),
        Call::CollectionsDelete { id } => reply(collections.delete(id).await),
        Call::CollectionsApiCounts => reply(collections.api_counts().await),
        Call::TreeLoad { collection } => reply(tree.load(collection).await),
        Call::TreeCreateFolder {
            collection,
            parent,
            name,
        } => reply(tree.create_folder(collection, parent, name).await),
        Call::TreeCreateApi {
            collection,
            parent,
            name,
        } => reply(tree.create_api(collection, parent, name).await),
        Call::TreeRename { collection, item, name } => reply(tree.rename(collection, item, name).await),
        Call::TreeSaveRequest {
            collection,
            api,
            request,
        } => reply(tree.save_request(collection, api, request).await),
        Call::TreeSetFolderSettings {
            collection,
            folder,
            settings,
        } => reply(tree.set_folder_settings(collection, folder, settings).await),
        Call::TreeSetProjectSettings { collection, settings } => {
            reply(tree.set_project_settings(collection, settings).await)
        }
        Call::TreeMove { collection, item, to } => reply(tree.move_item(collection, item, to).await),
        Call::TreeImport {
            collection,
            parent,
            items,
        } => reply(tree.import(collection, parent, items).await),
        Call::TreeDelete { collection, item } => reply(tree.delete(collection, item).await),
        Call::TreeConfig { collection } => reply(tree.config(collection).await),
        Call::TreeReplace {
            collection,
            config,
            base,
        } => reply(tree.replace(collection, config, base).await),
        Call::VariablesList { collection, category } => reply(variables.list(collection, category).await),
        Call::VariablesCreate { input } => reply(variables.create(input).await),
        Call::VariablesUpdate { collection, id, edit } => reply(variables.update(collection, id, edit).await),
        Call::VariablesDelete { collection, ids } => reply(variables.delete(collection, ids).await),
        Call::EnvironmentsList { collection } => reply(variables.environments(collection).await),
        Call::EnvironmentsCreate { collection, name } => reply(variables.create_environment(collection, name).await),
        Call::EnvironmentsRename { collection, id, name } => {
            reply(variables.rename_environment(collection, id, name).await)
        }
        Call::EnvironmentsDelete { collection, id } => reply(variables.delete_environment(collection, id).await),
        Call::EnvironmentsReorder { collection, order } => {
            reply(variables.reorder_environments(collection, order).await)
        }
        Call::PrefsApp => reply(prefs.app().await),
        Call::PrefsSetApp { prefs: value } => reply(prefs.set_app(value).await),
        Call::PrefsWorkspace { collection } => reply(prefs.workspace(collection).await),
        Call::PrefsSetWorkspace {
            collection,
            prefs: value,
        } => reply(prefs.set_workspace(collection, value).await),
        Call::PrefsClearWorkspaces => reply(prefs.clear_workspaces().await),
        // D4: an account keeps no more than the settings offer.
        Call::HistoryAdd { response, keep } => {
            let keep = keep.clamp(*HISTORY_SIZES.start(), *HISTORY_SIZES.end());
            reply(history.add(*response, keep).await)
        }
        Call::HistoryList { collection, api } => reply(history.list(collection, api).await),
        Call::HistoryLatest { collection, apis } => reply(history.latest(collection, apis).await),
        Call::HistoryRecent { collection, limit } => reply(history.recent(collection, limit.min(RECENT_LIMIT)).await),
        Call::HistoryClear { collection, api } => reply(history.clear(collection, api).await),
        Call::HistoryClearAll => reply(history.clear_all().await),
        Call::CookiesList => reply(cookies.list().await),
        Call::CookiesPut { cookies: put } => reply(cookies.put(put).await),
        Call::CookiesDelete { domain, path, name } => reply(cookies.delete(domain, path, name).await),
        Call::CookiesClear => reply(cookies.clear().await),
        Call::DirectoryMe => reply(directory.me().await),
        Call::DirectoryAccounts => reply(directory.accounts().await),
        Call::DirectoryCreateTeam { name } => reply(directory.create_team(name).await),
        Call::DirectoryRenameTeam { team, name } => reply(directory.rename_team(team, name).await),
        Call::DirectoryDeleteTeam { team } => reply(directory.delete_team(team).await),
        Call::DirectoryMembers { team } => reply(directory.members(team).await),
        Call::DirectorySetMember { team, account, role } => reply(directory.set_member(team, account, role).await),
        Call::DirectoryGrants { collection } => reply(directory.grants(collection).await),
        Call::DirectorySetGrant {
            collection,
            grantee,
            role,
        } => reply(directory.set_grant(collection, grantee, role).await),
        Call::DirectoryTransfer { collection, team } => reply(directory.transfer(collection, team).await),
        Call::DirectoryInvites => reply(directory.invites().await),
        Call::DirectoryCreateInvite { place } => reply(directory.create_invite(place).await),
        Call::DirectoryRevokeInvite { id } => reply(directory.revoke_invite(id).await),
        Call::DirectoryUpdateAccount { account, edit } => reply(directory.update_account(account, edit).await),
        Call::DirectoryResetPassword { account, password } => reply(directory.reset_password(account, password).await),
        Call::DirectoryChangePassword { current, password } => {
            reply(directory.change_password(current, password).await)
        }
        Call::DirectoryMcpTokens => reply(directory.mcp_tokens().await),
        Call::DirectoryIssueMcpToken { input } => reply(directory.issue_mcp_token(input).await),
        Call::DirectoryRevokeMcpToken { id } => reply(directory.revoke_mcp_token(id).await),
    }
}

/// How the browser reaches the bridge: the page's HTTP client, handed in.
pub trait Transport: Send + Sync {
    /// POSTs `body` (JSON) to `path` on the bridge; answers the status and
    /// the body, or why nothing came back.
    fn post(&self, path: &'static str, body: Vec<u8>)
    -> Pin<Box<dyn Future<Output = Result<Response, String>> + Send>>;

    /// As [`Self::post`], the body answered as it comes. One that cannot
    /// read it so answers it whole, as one piece.
    fn post_stream(
        &self,
        path: &'static str,
        body: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<Streamed, String>> + Send>> {
        let posted = self.post(path, body);
        Box::pin(async move {
            let response = posted.await?;
            let body = response.body;
            Ok(Streamed {
                status: response.status,
                pieces: Box::pin(futures::stream::once(async move { Ok(body) })),
            })
        })
    }
}

/// What came back from the bridge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

/// What came back from the bridge, its body as it comes.
pub struct Streamed {
    pub status: u16,
    pub pieces: Pin<Box<dyn Stream<Item = Result<Vec<u8>, String>> + Send>>,
}

/// Every Store trait, through the bridge — the browser's side.
#[derive(Clone)]
pub struct BridgeStore {
    transport: Arc<dyn Transport>,
}

impl BridgeStore {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { transport }
    }

    pub fn storage(&self) -> Storage {
        Storage::from_backend(Arc::new(self.clone()))
    }

    /// The accounts and teams, through the same transport.
    pub fn directory(&self) -> Arc<dyn DirectoryStore> {
        Arc::new(self.clone())
    }

    fn call<T: DeserializeOwned + Send + 'static>(&self, call: Call) -> StoreFuture<T> {
        let transport = self.transport.clone();
        Box::pin(async move {
            let body = serde_json::to_vec(&call).map_err(StoreError::backend)?;
            let response = transport.post(STORE_PATH, body).await.map_err(StoreError::backend)?;
            match response.status {
                200 => {}
                401 => return Err(StoreError::backend("not signed in")),
                status => {
                    return Err(StoreError::backend(format!(
                        "the bridge answered {status}: {}",
                        String::from_utf8_lossy(&response.body)
                    )));
                }
            }
            match serde_json::from_slice(&response.body).map_err(StoreError::backend)? {
                Reply::Ok(value) => serde_json::from_value(value).map_err(StoreError::backend),
                Reply::Err(error) => Err(error),
            }
        })
    }
}

impl DirectoryStore for BridgeStore {
    fn me(&self) -> StoreFuture<Me> {
        self.call(Call::DirectoryMe)
    }

    fn accounts(&self) -> StoreFuture<Vec<AccountInfo>> {
        self.call(Call::DirectoryAccounts)
    }

    fn create_team(&self, name: String) -> StoreFuture<Team> {
        self.call(Call::DirectoryCreateTeam { name })
    }

    fn rename_team(&self, team: TeamId, name: String) -> StoreFuture<()> {
        self.call(Call::DirectoryRenameTeam { team, name })
    }

    fn delete_team(&self, team: TeamId) -> StoreFuture<()> {
        self.call(Call::DirectoryDeleteTeam { team })
    }

    fn members(&self, team: TeamId) -> StoreFuture<Vec<Member>> {
        self.call(Call::DirectoryMembers { team })
    }

    fn set_member(&self, team: TeamId, account: AccountId, role: Option<TeamRole>) -> StoreFuture<()> {
        self.call(Call::DirectorySetMember { team, account, role })
    }

    fn grants(&self, collection: CollectionId) -> StoreFuture<Vec<Grant>> {
        self.call(Call::DirectoryGrants { collection })
    }

    fn set_grant(&self, collection: CollectionId, grantee: Grantee, role: Option<CollectionRole>) -> StoreFuture<()> {
        self.call(Call::DirectorySetGrant {
            collection,
            grantee,
            role,
        })
    }

    fn transfer(&self, collection: CollectionId, team: TeamId) -> StoreFuture<()> {
        self.call(Call::DirectoryTransfer { collection, team })
    }

    fn invites(&self) -> StoreFuture<Vec<Invite>> {
        self.call(Call::DirectoryInvites)
    }

    fn create_invite(&self, place: Option<TeamPlace>) -> StoreFuture<NewInvite> {
        self.call(Call::DirectoryCreateInvite { place })
    }

    fn revoke_invite(&self, id: InviteId) -> StoreFuture<()> {
        self.call(Call::DirectoryRevokeInvite { id })
    }

    fn update_account(&self, account: AccountId, edit: AccountEdit) -> StoreFuture<AccountInfo> {
        self.call(Call::DirectoryUpdateAccount { account, edit })
    }

    fn reset_password(&self, account: AccountId, password: String) -> StoreFuture<()> {
        self.call(Call::DirectoryResetPassword { account, password })
    }

    fn change_password(&self, current: String, password: String) -> StoreFuture<()> {
        self.call(Call::DirectoryChangePassword { current, password })
    }

    fn mcp_tokens(&self) -> StoreFuture<Vec<McpToken>> {
        self.call(Call::DirectoryMcpTokens)
    }

    fn issue_mcp_token(&self, input: NewMcpToken) -> StoreFuture<IssuedMcpToken> {
        self.call(Call::DirectoryIssueMcpToken { input })
    }

    fn revoke_mcp_token(&self, id: McpTokenId) -> StoreFuture<()> {
        self.call(Call::DirectoryRevokeMcpToken { id })
    }
}

impl CollectionStore for BridgeStore {
    fn list(&self) -> StoreFuture<Vec<Collection>> {
        self.call(Call::CollectionsList)
    }
    fn get(&self, id: CollectionId) -> StoreFuture<Collection> {
        self.call(Call::CollectionsGet { id })
    }
    fn create(&self, input: NewCollection) -> StoreFuture<Collection> {
        self.call(Call::CollectionsCreate { input })
    }
    fn update(&self, id: CollectionId, edit: CollectionEdit) -> StoreFuture<Collection> {
        self.call(Call::CollectionsUpdate { id, edit })
    }
    fn delete(&self, id: CollectionId) -> StoreFuture<()> {
        self.call(Call::CollectionsDelete { id })
    }
    fn api_counts(&self) -> StoreFuture<BTreeMap<CollectionId, usize>> {
        self.call(Call::CollectionsApiCounts)
    }
}

impl TreeStore for BridgeStore {
    fn load(&self, collection: CollectionId) -> StoreFuture<TreeData> {
        self.call(Call::TreeLoad { collection })
    }
    fn create_folder(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<Folder>> {
        self.call(Call::TreeCreateFolder {
            collection,
            parent,
            name,
        })
    }
    fn create_api(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<ApiSetting>> {
        self.call(Call::TreeCreateApi {
            collection,
            parent,
            name,
        })
    }
    fn rename(&self, collection: CollectionId, item: ItemId, name: String) -> StoreFuture<()> {
        self.call(Call::TreeRename { collection, item, name })
    }
    fn save_request(&self, collection: CollectionId, api: ItemId, request: HttpRequest) -> StoreFuture<ApiSetting> {
        self.call(Call::TreeSaveRequest {
            collection,
            api,
            request,
        })
    }
    fn set_folder_settings(
        &self,
        collection: CollectionId,
        folder: ItemId,
        settings: FolderSettings,
    ) -> StoreFuture<Folder> {
        self.call(Call::TreeSetFolderSettings {
            collection,
            folder,
            settings,
        })
    }
    fn set_project_settings(&self, collection: CollectionId, settings: ProjectSettings) -> StoreFuture<()> {
        self.call(Call::TreeSetProjectSettings { collection, settings })
    }
    fn move_item(&self, collection: CollectionId, item: ItemId, to: Placement) -> StoreFuture<TreeChange> {
        self.call(Call::TreeMove { collection, item, to })
    }
    fn import(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        items: Vec<ImportItem>,
    ) -> StoreFuture<Imported> {
        self.call(Call::TreeImport {
            collection,
            parent,
            items,
        })
    }
    fn delete(&self, collection: CollectionId, item: ItemId) -> StoreFuture<Removal> {
        self.call(Call::TreeDelete { collection, item })
    }
    fn config(&self, collection: CollectionId) -> StoreFuture<ConfigAt> {
        self.call(Call::TreeConfig { collection })
    }
    fn replace(&self, collection: CollectionId, config: ProjectConfig, base: Option<u64>) -> StoreFuture<u64> {
        self.call(Call::TreeReplace {
            collection,
            config,
            base,
        })
    }
}

impl VariableStore for BridgeStore {
    fn list(&self, collection: CollectionId, category: VariableCategory) -> StoreFuture<Vec<Variable>> {
        self.call(Call::VariablesList { collection, category })
    }
    fn create(&self, input: NewVariable) -> StoreFuture<Variable> {
        self.call(Call::VariablesCreate { input })
    }
    fn update(&self, collection: CollectionId, id: VariableId, edit: VariableEdit) -> StoreFuture<Variable> {
        self.call(Call::VariablesUpdate { collection, id, edit })
    }
    fn delete(&self, collection: CollectionId, ids: Vec<VariableId>) -> StoreFuture<()> {
        self.call(Call::VariablesDelete { collection, ids })
    }
    fn environments(&self, collection: CollectionId) -> StoreFuture<Vec<Environment>> {
        self.call(Call::EnvironmentsList { collection })
    }
    fn create_environment(&self, collection: CollectionId, name: String) -> StoreFuture<Environment> {
        self.call(Call::EnvironmentsCreate { collection, name })
    }
    fn rename_environment(
        &self,
        collection: CollectionId,
        id: EnvironmentId,
        name: String,
    ) -> StoreFuture<Environment> {
        self.call(Call::EnvironmentsRename { collection, id, name })
    }
    fn delete_environment(&self, collection: CollectionId, id: EnvironmentId) -> StoreFuture<()> {
        self.call(Call::EnvironmentsDelete { collection, id })
    }
    fn reorder_environments(
        &self,
        collection: CollectionId,
        order: Vec<EnvironmentId>,
    ) -> StoreFuture<Vec<Environment>> {
        self.call(Call::EnvironmentsReorder { collection, order })
    }
}

impl PrefStore for BridgeStore {
    fn app(&self) -> StoreFuture<AppPrefs> {
        self.call(Call::PrefsApp)
    }
    fn set_app(&self, prefs: AppPrefs) -> StoreFuture<()> {
        self.call(Call::PrefsSetApp { prefs })
    }
    fn workspace(&self, collection: CollectionId) -> StoreFuture<WorkspacePrefs> {
        self.call(Call::PrefsWorkspace { collection })
    }
    fn set_workspace(&self, collection: CollectionId, prefs: WorkspacePrefs) -> StoreFuture<()> {
        self.call(Call::PrefsSetWorkspace { collection, prefs })
    }

    fn clear_workspaces(&self) -> StoreFuture<()> {
        self.call(Call::PrefsClearWorkspaces)
    }
}

impl HistoryStore for BridgeStore {
    fn add(&self, response: HttpResponse, keep: usize) -> StoreFuture<HttpResponse> {
        self.call(Call::HistoryAdd {
            response: Box::new(response),
            keep,
        })
    }
    fn recent(&self, collection: CollectionId, limit: usize) -> StoreFuture<Vec<HttpResponse>> {
        self.call(Call::HistoryRecent { collection, limit })
    }
    fn list(&self, collection: CollectionId, api: ItemId) -> StoreFuture<Vec<HttpResponse>> {
        self.call(Call::HistoryList { collection, api })
    }
    fn latest(&self, collection: CollectionId, apis: Vec<ItemId>) -> StoreFuture<Vec<HttpResponse>> {
        self.call(Call::HistoryLatest { collection, apis })
    }
    fn clear(&self, collection: CollectionId, api: ItemId) -> StoreFuture<()> {
        self.call(Call::HistoryClear { collection, api })
    }

    fn clear_all(&self) -> StoreFuture<()> {
        self.call(Call::HistoryClearAll)
    }
}

impl CookieStore for BridgeStore {
    fn list(&self) -> StoreFuture<Vec<Cookie>> {
        self.call(Call::CookiesList)
    }
    fn put(&self, cookies: Vec<Cookie>) -> StoreFuture<()> {
        self.call(Call::CookiesPut { cookies })
    }
    fn delete(&self, domain: String, path: String, name: String) -> StoreFuture<()> {
        self.call(Call::CookiesDelete { domain, path, name })
    }
    fn clear(&self) -> StoreFuture<()> {
        self.call(Call::CookiesClear)
    }
}

/// Sending through the bridge — the browser's engine. Dropping the future
/// stops waiting; the bridge finishes the request on its own (it is bounded by
/// its timeouts).
#[derive(Clone)]
pub struct BridgeHttpEngine {
    transport: Arc<dyn Transport>,
}

impl BridgeHttpEngine {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { transport }
    }
}

impl HttpEngine for BridgeHttpEngine {
    fn send(&self, request: PreparedRequest) -> SendFuture {
        let transport = self.transport.clone();
        Box::pin(async move {
            let bridge = |message: String| SendError::Bridge { message };
            let body = serde_json::to_vec(&request).map_err(|e| bridge(e.to_string()))?;
            let response = transport.post(SEND_PATH, body).await.map_err(bridge)?;
            match response.status {
                200 => {}
                401 => return Err(bridge("not signed in".into())),
                status => {
                    return Err(bridge(format!(
                        "answered {status}: {}",
                        String::from_utf8_lossy(&response.body)
                    )));
                }
            }
            match serde_json::from_slice(&response.body).map_err(|e| bridge(e.to_string()))? {
                SendReply::Ok(exchange) => Ok(*exchange),
                SendReply::Err(error) => Err(error),
            }
        })
    }

    fn send_watched(&self, request: PreparedRequest, watch: Watch) -> SendFuture {
        let transport = self.transport.clone();
        Box::pin(async move {
            let bridge = |message: String| SendError::Bridge { message };
            let body = serde_json::to_vec(&request).map_err(|e| bridge(e.to_string()))?;
            let mut streamed = transport.post_stream(SEND_STREAM_PATH, body).await.map_err(bridge)?;
            if streamed.status != 200 {
                let mut answer = Vec::new();
                while let Some(Ok(piece)) = streamed.pieces.next().await {
                    answer.extend(piece);
                }
                return Err(bridge(match streamed.status {
                    401 => "not signed in".into(),
                    status => format!("answered {status}: {}", String::from_utf8_lossy(&answer)),
                }));
            }
            let mut told = Told::new();
            let mut line = Vec::new();
            let mut stop = watch.stop.clone();
            loop {
                let next = match futures::future::select(&mut stop, streamed.pieces.next()).await {
                    futures::future::Either::Left(_) => return told.ended(StreamEnd::Stopped),
                    futures::future::Either::Right((next, _)) => next,
                };
                let piece = match next {
                    Some(Ok(piece)) => piece,
                    Some(Err(message)) => return told.ended(StreamEnd::Failed { message }),
                    None => {
                        let message = "the bridge ended the answer early".to_string();
                        return told.ended(StreamEnd::Failed { message });
                    }
                };
                for byte in piece {
                    if byte != b'\n' {
                        line.push(byte);
                        continue;
                    }
                    let reply: StreamReply =
                        serde_json::from_slice(&std::mem::take(&mut line)).map_err(|e| bridge(e.to_string()))?;
                    match reply {
                        StreamReply::Progress(progress) => {
                            told.add(&progress);
                            (watch.progress)(progress);
                        }
                        StreamReply::Done(SendReply::Ok(mut exchange)) => {
                            if exchange.stream.is_some() {
                                exchange.body = told.body;
                            }
                            return Ok(*exchange);
                        }
                        StreamReply::Done(SendReply::Err(error)) => return Err(error),
                    }
                }
            }
        })
    }
}

/// What a watched send was told so far: what the page keeps of an event
/// stream it stops, or that breaks off, before the bridge's own account of
/// it.
struct Told {
    /// When the send began: how long a stream it ends took.
    started: web_time::Instant,
    status: u16,
    headers: Vec<(String, String)>,
    /// When the head came; `None` before it did.
    head: Option<web_time::Instant>,
    body: Vec<u8>,
    arrivals: Vec<(u64, u64)>,
}

impl Told {
    fn new() -> Self {
        Self {
            started: web_time::Instant::now(),
            status: 0,
            headers: Vec::new(),
            head: None,
            body: Vec::new(),
            arrivals: Vec::new(),
        }
    }

    fn add(&mut self, progress: &Progress) {
        match progress {
            Progress::Head { status, headers } => {
                self.status = *status;
                self.headers = headers.clone();
                self.head = Some(web_time::Instant::now());
            }
            Progress::Body(piece) => {
                self.body.extend_from_slice(piece);
                let at = self.head.map_or(0, |head| head.elapsed().as_millis() as u64);
                self.arrivals.push((self.body.len() as u64, at));
            }
        }
    }

    /// The exchange of a stream that ended here, before the bridge said
    /// so; a send that had not begun one has none.
    fn ended(self, ended: StreamEnd) -> Result<Exchange, SendError> {
        if self.head.is_none() {
            return Err(SendError::Bridge {
                message: match ended {
                    StreamEnd::Failed { message } => message,
                    _ => "stopped".into(),
                },
            });
        }
        Ok(Exchange {
            status: self.status,
            headers: self.headers,
            stats: cyberapi_core::response::HttpStats {
                total: self.started.elapsed(),
                ..Default::default()
            },
            body_size: self.body.len() as u64,
            body: self.body,
            stream: Some(StreamInfo {
                arrivals: self.arrivals,
                ended,
            }),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_client_reads_projects_but_does_not_change_them() {
        let rename = Call::TreeRename {
            collection: CollectionId::from("c"),
            item: ItemId::from("i"),
            name: "n".into(),
        };
        let load = Call::TreeLoad {
            collection: CollectionId::from("c"),
        };
        let prefs = Call::PrefsApp;
        // A bridge two shapes on: a client that does not say is the first.
        assert!(refused_as_too_old(&rename, None, 2));
        assert!(refused_as_too_old(&rename, Some(1), 2));
        assert!(!refused_as_too_old(&rename, Some(2), 2));
        assert!(!refused_as_too_old(&rename, Some(3), 2), "a newer client may write");
        assert!(!refused_as_too_old(&load, None, 2), "reading is never refused");
        assert!(
            !refused_as_too_old(&prefs, None, 2),
            "the reader's own is not a project's"
        );
        // The first shape refuses no one.
        assert!(!refused_as_too_old(&rename, None, 1));
    }

    #[test]
    fn calls_read_as_named_methods() {
        let call = Call::TreeRename {
            collection: CollectionId::from("c"),
            item: ItemId::from("i"),
            name: "n".into(),
        };
        let json = serde_json::to_string(&call).expect("serialize");
        assert_eq!(
            json,
            r#"{"call":"treeRename","args":{"collection":"c","item":"i","name":"n"}}"#
        );
        assert_eq!(serde_json::from_str::<Call>(&json).expect("parse"), call);
        assert_eq!(
            serde_json::to_string(&Call::CollectionsList).expect("serialize"),
            r#"{"call":"collectionsList"}"#
        );
    }

    /// D4: a page from before says nothing of how many to keep: the default.
    #[test]
    fn a_response_kept_without_a_count_keeps_the_default() {
        let call: Call = serde_json::from_str(r#"{"call":"historyAdd","args":{"response":{"id":""}}}"#).expect("parse");
        assert!(matches!(call, Call::HistoryAdd { keep, .. } if keep == HISTORY_LIMIT));
    }

    #[test]
    fn refusals_cross_as_they_were_made() {
        let reply = Reply::Err(StoreError::NameRequired);
        let json = serde_json::to_string(&reply).expect("serialize");
        assert_eq!(serde_json::from_str::<Reply>(&json).expect("parse"), reply);
    }

    /// A bridge answering a watched send with these lines, then — when
    /// `open` — nothing more, its stream left open.
    struct Streaming {
        lines: Vec<StreamReply>,
        open: bool,
    }

    impl Transport for Streaming {
        fn post(&self, _: &'static str, _: Vec<u8>) -> Pin<Box<dyn Future<Output = Result<Response, String>> + Send>> {
            Box::pin(async { Err("not posted".to_string()) })
        }

        fn post_stream(
            &self,
            path: &'static str,
            _: Vec<u8>,
        ) -> Pin<Box<dyn Future<Output = Result<Streamed, String>> + Send>> {
            assert_eq!(path, SEND_STREAM_PATH);
            // A line split across pieces too.
            let mut pieces: Vec<Result<Vec<u8>, String>> = Vec::new();
            for line in &self.lines {
                let line = line.line();
                let (a, b) = line.split_at(line.len() / 2);
                pieces.push(Ok(a.to_vec()));
                pieces.push(Ok(b.to_vec()));
            }
            let open = self.open;
            let pieces = futures::stream::iter(pieces).chain(futures::stream::once(async move {
                if open {
                    std::future::pending::<()>().await;
                }
                Err("closed".to_string())
            }));
            Box::pin(async move {
                Ok(Streamed {
                    status: 200,
                    pieces: Box::pin(pieces),
                })
            })
        }
    }

    fn watch() -> (
        Watch,
        Arc<std::sync::Mutex<Vec<Progress>>>,
        futures::channel::oneshot::Sender<()>,
    ) {
        let told = Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = told.clone();
        let (stop, stopped) = cyberapi_http::stop_signal();
        let watch = Watch {
            progress: Arc::new(move |progress| kept.lock().expect("told").push(progress)),
            stop: stopped,
        };
        (watch, told, stop)
    }

    /// The bridge's lines are told as they come; its exchange takes the
    /// body the pieces made. Stopped before the bridge's last line, the
    /// exchange is what came.
    #[test]
    fn a_watched_send_is_told_as_the_bridge_streams_it() {
        let head = Progress::Head {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
        };
        let pieces = [
            Progress::Body(b"data: 0\n\n".to_vec()),
            Progress::Body(b"data: 1\n\n".to_vec()),
        ];
        let stream = StreamInfo {
            arrivals: vec![(9, 5), (18, 50)],
            ended: StreamEnd::Closed,
        };
        let done = Exchange {
            status: 200,
            stream: Some(stream.clone()),
            ..Default::default()
        };
        let mut lines = vec![StreamReply::Progress(head.clone())];
        lines.extend(pieces.iter().cloned().map(StreamReply::Progress));
        lines.push(StreamReply::Done(SendReply::Ok(Box::new(done))));
        let engine = BridgeHttpEngine::new(Arc::new(Streaming { lines, open: false }));
        let (watched, told, _stop) = watch();
        let exchange =
            futures::executor::block_on(engine.send_watched(PreparedRequest::default(), watched)).expect("an exchange");
        assert_eq!(exchange.body, b"data: 0\n\ndata: 1\n\n");
        assert_eq!(exchange.stream, Some(stream));
        assert_eq!(told.lock().expect("told").len(), 3);

        let lines = vec![StreamReply::Progress(head), StreamReply::Progress(pieces[0].clone())];
        let engine = BridgeHttpEngine::new(Arc::new(Streaming { lines, open: true }));
        let (watched, told, stop) = watch();
        let sending = engine.send_watched(PreparedRequest::default(), watched);
        let waiting = std::thread::spawn(move || futures::executor::block_on(sending));
        while told.lock().expect("told").len() < 2 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _ = stop.send(());
        let stopped = waiting.join().expect("joined").expect("an exchange");
        assert_eq!((stopped.status, stopped.body.as_slice()), (200, &b"data: 0\n\n"[..]));
        assert_eq!(stopped.stream.map(|stream| stream.ended), Some(StreamEnd::Stopped));
        assert!(stopped.stats.total > std::time::Duration::ZERO, "how long it took");
    }
}
