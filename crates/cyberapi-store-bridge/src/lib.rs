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
//!
//! Sending a request is the other half (`docs/architecture.md` §5, D5): the
//! page prepares it with the shared pipeline and [`BridgeHttpEngine`] posts
//! it to [`SEND_PATH`], where the bridge's engine sends it and answers with a
//! [`SendReply`].

use cyberapi_core::cookie::Cookie;
use cyberapi_core::import::ImportItem;
use cyberapi_core::model::{ApiSetting, Collection, Folder, HttpRequest, Variable, VariableCategory};
use cyberapi_core::prefs::{AppPrefs, WorkspacePrefs};
use cyberapi_core::response::HttpResponse;
use cyberapi_core::tree::{Placement, Removal, TreeChange, TreeData};
use cyberapi_core::{CollectionId, ItemId, VariableId};
use cyberapi_http::{Exchange, HttpEngine, PreparedRequest, SendError, SendFuture};
use cyberapi_store::{
    CollectionEdit, CollectionStore, CookieStore, HistoryStore, Imported, Inserted, NewCollection, NewVariable,
    PrefStore, Storage, StoreError, StoreFuture, TreeStore, VariableEdit, VariableStore,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Where calls are posted.
pub const STORE_PATH: &str = "/v1/store";
/// Where requests to send are posted.
pub const SEND_PATH: &str = "/v1/send";

/// What sending answers: the exchange, or why there was none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SendReply {
    Ok(Exchange),
    Err(SendError),
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
    HistoryAdd {
        // Boxed: a response is far bigger than any other call.
        response: Box<HttpResponse>,
    },
    HistoryList {
        collection: CollectionId,
        api: ItemId,
    },
    HistoryLatest {
        collection: CollectionId,
        apis: Vec<ItemId>,
    },
    HistoryClear {
        collection: CollectionId,
        api: ItemId,
    },
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
}

/// What a call answers: the method's value, or the store's refusal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reply {
    Ok(serde_json::Value),
    Err(StoreError),
}

/// Runs `call` on `storage` — the bridge's side.
pub async fn dispatch(storage: &Storage, call: Call) -> Reply {
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
        Call::TreeMove { collection, item, to } => reply(tree.move_item(collection, item, to).await),
        Call::TreeImport {
            collection,
            parent,
            items,
        } => reply(tree.import(collection, parent, items).await),
        Call::TreeDelete { collection, item } => reply(tree.delete(collection, item).await),
        Call::VariablesList { collection, category } => reply(variables.list(collection, category).await),
        Call::VariablesCreate { input } => reply(variables.create(input).await),
        Call::VariablesUpdate { collection, id, edit } => reply(variables.update(collection, id, edit).await),
        Call::VariablesDelete { collection, ids } => reply(variables.delete(collection, ids).await),
        Call::PrefsApp => reply(prefs.app().await),
        Call::PrefsSetApp { prefs: value } => reply(prefs.set_app(value).await),
        Call::PrefsWorkspace { collection } => reply(prefs.workspace(collection).await),
        Call::PrefsSetWorkspace {
            collection,
            prefs: value,
        } => reply(prefs.set_workspace(collection, value).await),
        Call::HistoryAdd { response } => reply(history.add(*response).await),
        Call::HistoryList { collection, api } => reply(history.list(collection, api).await),
        Call::HistoryLatest { collection, apis } => reply(history.latest(collection, apis).await),
        Call::HistoryClear { collection, api } => reply(history.clear(collection, api).await),
        Call::CookiesList => reply(cookies.list().await),
        Call::CookiesPut { cookies: put } => reply(cookies.put(put).await),
        Call::CookiesDelete { domain, path, name } => reply(cookies.delete(domain, path, name).await),
        Call::CookiesClear => reply(cookies.clear().await),
    }
}

/// How the browser reaches the bridge: the page's HTTP client, handed in.
pub trait Transport: Send + Sync {
    /// POSTs `body` (JSON) to `path` on the bridge; answers the status and
    /// the body, or why nothing came back.
    fn post(&self, path: &'static str, body: Vec<u8>)
    -> Pin<Box<dyn Future<Output = Result<Response, String>> + Send>>;
}

/// What came back from the bridge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
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
}

impl HistoryStore for BridgeStore {
    fn add(&self, response: HttpResponse) -> StoreFuture<HttpResponse> {
        self.call(Call::HistoryAdd {
            response: Box::new(response),
        })
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
                SendReply::Ok(exchange) => Ok(exchange),
                SendReply::Err(error) => Err(error),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn refusals_cross_as_they_were_made() {
        let reply = Reply::Err(StoreError::NameRequired);
        let json = serde_json::to_string(&reply).expect("serialize");
        assert_eq!(serde_json::from_str::<Reply>(&json).expect("parse"), reply);
    }
}
