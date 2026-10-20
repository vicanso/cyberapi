//! The page's stores, until the bridge exists: every call answers that there
//! is no storage, so the page shows the failure states rather than pretending
//! to keep anything. Step 5 of `docs/architecture.md` replaces this with the
//! store that talks to the bridge.

use cyberapi_core::model::{ApiSetting, Collection, Folder, HttpRequest, Variable, VariableCategory};
use cyberapi_core::prefs::{AppPrefs, WorkspacePrefs};
use cyberapi_core::tree::{Placement, Removal, TreeChange, TreeData};
use cyberapi_core::{CollectionId, ItemId, VariableId};
use cyberapi_store::{
    CollectionEdit, CollectionStore, Inserted, NewCollection, NewVariable, PrefStore, StoreError, StoreFuture,
    TreeStore, VariableEdit, VariableStore,
};

pub struct UnavailableStore;

fn unavailable<T: Send + 'static>() -> StoreFuture<T> {
    Box::pin(async {
        Err(StoreError::backend(
            "the browser build cannot store anything yet: it needs the bridge",
        ))
    })
}

impl CollectionStore for UnavailableStore {
    fn list(&self) -> StoreFuture<Vec<Collection>> {
        unavailable()
    }
    fn get(&self, _: CollectionId) -> StoreFuture<Collection> {
        unavailable()
    }
    fn create(&self, _: NewCollection) -> StoreFuture<Collection> {
        unavailable()
    }
    fn update(&self, _: CollectionId, _: CollectionEdit) -> StoreFuture<Collection> {
        unavailable()
    }
    fn delete(&self, _: CollectionId) -> StoreFuture<()> {
        unavailable()
    }
}

impl TreeStore for UnavailableStore {
    fn load(&self, _: CollectionId) -> StoreFuture<TreeData> {
        unavailable()
    }
    fn create_folder(&self, _: CollectionId, _: Option<ItemId>, _: String) -> StoreFuture<Inserted<Folder>> {
        unavailable()
    }
    fn create_api(&self, _: CollectionId, _: Option<ItemId>, _: String) -> StoreFuture<Inserted<ApiSetting>> {
        unavailable()
    }
    fn rename(&self, _: CollectionId, _: ItemId, _: String) -> StoreFuture<()> {
        unavailable()
    }
    fn save_request(&self, _: CollectionId, _: ItemId, _: HttpRequest) -> StoreFuture<ApiSetting> {
        unavailable()
    }
    fn move_item(&self, _: CollectionId, _: ItemId, _: Placement) -> StoreFuture<TreeChange> {
        unavailable()
    }
    fn delete(&self, _: CollectionId, _: ItemId) -> StoreFuture<Removal> {
        unavailable()
    }
}

impl VariableStore for UnavailableStore {
    fn list(&self, _: CollectionId, _: VariableCategory) -> StoreFuture<Vec<Variable>> {
        unavailable()
    }
    fn create(&self, _: NewVariable) -> StoreFuture<Variable> {
        unavailable()
    }
    fn update(&self, _: CollectionId, _: VariableId, _: VariableEdit) -> StoreFuture<Variable> {
        unavailable()
    }
    fn delete(&self, _: CollectionId, _: Vec<VariableId>) -> StoreFuture<()> {
        unavailable()
    }
}

impl PrefStore for UnavailableStore {
    fn app(&self) -> StoreFuture<AppPrefs> {
        unavailable()
    }
    fn set_app(&self, _: AppPrefs) -> StoreFuture<()> {
        unavailable()
    }
    fn workspace(&self, _: CollectionId) -> StoreFuture<WorkspacePrefs> {
        unavailable()
    }
    fn set_workspace(&self, _: CollectionId, _: WorkspacePrefs) -> StoreFuture<()> {
        unavailable()
    }
}
