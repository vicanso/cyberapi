//! Everything in memory, in insertion order: the reference backend the
//! conformance suite is checked against, and the store tests of the layers
//! above use.

use crate::rules::required_name;
use crate::{
    CollectionEdit, CollectionStore, Entity, Inserted, NewCollection, NewVariable, PrefStore, Storage, StoreError,
    StoreFuture, TreeStore, VariableEdit, VariableStore,
};
use chrono::Utc;
use cyberapi_core::model::{ApiCategory, ApiSetting, Collection, Folder, HttpRequest, Variable, VariableCategory};
use cyberapi_core::prefs::{AppPrefs, WorkspacePrefs};
use cyberapi_core::tree::{Placement, Removal, TreeChange, TreeData};
use cyberapi_core::{CollectionId, ItemId, Timestamp, VariableId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ulid::Ulid;

type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

#[derive(Clone)]
pub struct MemoryStore {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    clock: Clock,
}

#[derive(Default)]
struct State {
    collections: Vec<Collection>,
    folders: Vec<Folder>,
    apis: Vec<ApiSetting>,
    top_orders: HashMap<CollectionId, Vec<ItemId>>,
    variables: Vec<Variable>,
    app: AppPrefs,
    workspaces: HashMap<CollectionId, WorkspacePrefs>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::with_clock(|| Timestamp::new(Utc::now().fixed_offset()))
    }

    /// A store whose "now" is `clock`, for tests that compare times.
    pub fn with_clock(clock: impl Fn() -> Timestamp + Send + Sync + 'static) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                clock: Arc::new(clock),
            }),
        }
    }

    /// This store as every trait the layers above use.
    pub fn storage(&self) -> Storage {
        Storage::from_backend(Arc::new(self.clone()))
    }

    fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut State, Timestamp) -> Result<T, StoreError> + Send + 'static,
    ) -> StoreFuture<T> {
        let inner = self.inner.clone();
        Box::pin(async move {
            let now = (inner.clock)();
            let mut state = inner
                .state
                .lock()
                .map_err(|_| StoreError::backend("the memory store's lock was poisoned"))?;
            work(&mut state, now)
        })
    }
}

fn new_id() -> String {
    Ulid::generate().to_string()
}

impl State {
    fn collection(&self, id: &CollectionId) -> Result<&Collection, StoreError> {
        self.collections
            .iter()
            .find(|collection| &collection.id == id)
            .ok_or_else(|| StoreError::not_found(Entity::Collection, id))
    }

    fn collection_mut(&mut self, id: &CollectionId) -> Result<&mut Collection, StoreError> {
        self.collections
            .iter_mut()
            .find(|collection| &collection.id == id)
            .ok_or_else(|| StoreError::not_found(Entity::Collection, id))
    }

    /// A change to the project's content moves its `updated_at` (#21) and
    /// its revision.
    fn touch(&mut self, id: &CollectionId, now: Timestamp) -> Result<(), StoreError> {
        let collection = self.collection_mut(id)?;
        collection.updated_at = now;
        collection.revision += 1;
        Ok(())
    }

    fn tree_data(&self, id: &CollectionId) -> TreeData {
        TreeData {
            folders: self
                .folders
                .iter()
                .filter(|folder| &folder.collection == id)
                .cloned()
                .collect(),
            apis: self.apis.iter().filter(|api| &api.collection == id).cloned().collect(),
            top_order: self.top_orders.get(id).cloned().unwrap_or_default(),
        }
    }

    fn apply(&mut self, collection: &CollectionId, change: &TreeChange) {
        for (id, children) in &change.folders {
            for folder in self
                .folders
                .iter_mut()
                .filter(|folder| &folder.collection == collection && &folder.id == id)
            {
                folder.children = children.clone();
            }
        }
        if let Some(top_order) = &change.top_order {
            self.top_orders.insert(collection.clone(), top_order.clone());
        }
    }

    /// Where a new item goes, after checking the project and the parent.
    fn plan_insert(
        &self,
        collection: &CollectionId,
        parent: Option<&ItemId>,
        item: &ItemId,
    ) -> Result<TreeChange, StoreError> {
        self.collection(collection)?;
        Ok(self.tree_data(collection).plan_insert(item, parent)?)
    }
}

impl CollectionStore for MemoryStore {
    fn list(&self) -> StoreFuture<Vec<Collection>> {
        self.run(|state, _| Ok(state.collections.clone()))
    }

    fn get(&self, id: CollectionId) -> StoreFuture<Collection> {
        self.run(move |state, _| state.collection(&id).cloned())
    }

    fn create(&self, input: NewCollection) -> StoreFuture<Collection> {
        self.run(move |state, now| {
            let collection = Collection {
                id: CollectionId::new(new_id()),
                name: required_name(&input.name)?,
                description: input.description,
                created_at: now,
                updated_at: now,
                revision: 0,
                // No teams here: every project is the one reader's.
                owner: None,
                role: Default::default(),
            };
            state.collections.push(collection.clone());
            Ok(collection)
        })
    }

    fn update(&self, id: CollectionId, edit: CollectionEdit) -> StoreFuture<Collection> {
        self.run(move |state, now| {
            let name = required_name(&edit.name)?;
            let collection = state.collection_mut(&id)?;
            collection.name = name;
            collection.description = edit.description;
            collection.updated_at = now;
            collection.revision += 1;
            Ok(collection.clone())
        })
    }

    fn delete(&self, id: CollectionId) -> StoreFuture<()> {
        self.run(move |state, _| {
            state.collection(&id)?;
            state.collections.retain(|collection| collection.id != id);
            state.folders.retain(|folder| folder.collection != id);
            state.apis.retain(|api| api.collection != id);
            state.variables.retain(|variable| variable.collection != id);
            state.top_orders.remove(&id);
            state.workspaces.remove(&id);
            Ok(())
        })
    }
}

impl TreeStore for MemoryStore {
    fn load(&self, collection: CollectionId) -> StoreFuture<TreeData> {
        self.run(move |state, _| {
            state.collection(&collection)?;
            Ok(state.tree_data(&collection))
        })
    }

    fn create_folder(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<Folder>> {
        self.run(move |state, now| {
            let name = required_name(&name)?;
            let id = ItemId::new(new_id());
            let change = state.plan_insert(&collection, parent.as_ref(), &id)?;
            let folder = Folder {
                id,
                collection: collection.clone(),
                name,
                children: Vec::new(),
                created_at: now,
                updated_at: now,
            };
            state.folders.push(folder.clone());
            state.apply(&collection, &change);
            state.touch(&collection, now)?;
            Ok(Inserted { item: folder, change })
        })
    }

    fn create_api(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<ApiSetting>> {
        self.run(move |state, now| {
            let name = required_name(&name)?;
            let id = ItemId::new(new_id());
            let change = state.plan_insert(&collection, parent.as_ref(), &id)?;
            let api = ApiSetting {
                id,
                collection: collection.clone(),
                name,
                category: ApiCategory::Http,
                request: HttpRequest::default(),
                created_at: now,
                updated_at: now,
            };
            state.apis.push(api.clone());
            state.apply(&collection, &change);
            state.touch(&collection, now)?;
            Ok(Inserted { item: api, change })
        })
    }

    fn rename(&self, collection: CollectionId, item: ItemId, name: String) -> StoreFuture<()> {
        self.run(move |state, now| {
            let name = required_name(&name)?;
            state.collection(&collection)?;
            if let Some(folder) = state
                .folders
                .iter_mut()
                .find(|folder| folder.collection == collection && folder.id == item)
            {
                folder.name = name;
                folder.updated_at = now;
            } else if let Some(api) = state
                .apis
                .iter_mut()
                .find(|api| api.collection == collection && api.id == item)
            {
                api.name = name;
                api.updated_at = now;
            } else {
                return Err(StoreError::not_found(Entity::Item, &item));
            }
            state.touch(&collection, now)
        })
    }

    fn save_request(&self, collection: CollectionId, api: ItemId, request: HttpRequest) -> StoreFuture<ApiSetting> {
        self.run(move |state, now| {
            state.collection(&collection)?;
            let found = state
                .apis
                .iter_mut()
                .find(|found| found.collection == collection && found.id == api)
                .ok_or_else(|| StoreError::not_found(Entity::Api, &api))?;
            found.request = request;
            found.updated_at = now;
            let saved = found.clone();
            state.touch(&collection, now)?;
            Ok(saved)
        })
    }

    fn move_item(&self, collection: CollectionId, item: ItemId, to: Placement) -> StoreFuture<TreeChange> {
        self.run(move |state, now| {
            state.collection(&collection)?;
            let change = state.tree_data(&collection).plan_move(&item, &to)?;
            state.apply(&collection, &change);
            state.touch(&collection, now)?;
            Ok(change)
        })
    }

    fn delete(&self, collection: CollectionId, item: ItemId) -> StoreFuture<Removal> {
        self.run(move |state, now| {
            state.collection(&collection)?;
            let removal = state.tree_data(&collection).plan_remove(&item)?;
            state
                .folders
                .retain(|folder| folder.collection != collection || !removal.folders.contains(&folder.id));
            state
                .apis
                .retain(|api| api.collection != collection || !removal.apis.contains(&api.id));
            state.apply(&collection, &removal.change);
            state.touch(&collection, now)?;
            Ok(removal)
        })
    }
}

impl VariableStore for MemoryStore {
    fn list(&self, collection: CollectionId, category: VariableCategory) -> StoreFuture<Vec<Variable>> {
        self.run(move |state, _| {
            state.collection(&collection)?;
            let mut found: Vec<Variable> = state
                .variables
                .iter()
                .filter(|variable| variable.collection == collection && variable.category == category)
                .cloned()
                .collect();
            found.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(found)
        })
    }

    fn create(&self, input: NewVariable) -> StoreFuture<Variable> {
        self.run(move |state, now| {
            state.collection(&input.collection)?;
            let variable = Variable {
                id: VariableId::new(new_id()),
                collection: input.collection.clone(),
                category: input.category,
                name: input.name,
                value: input.value,
                enabled: input.enabled,
                created_at: now,
                updated_at: now,
            };
            state.variables.push(variable.clone());
            state.touch(&input.collection, now)?;
            Ok(variable)
        })
    }

    fn update(&self, collection: CollectionId, id: VariableId, edit: VariableEdit) -> StoreFuture<Variable> {
        self.run(move |state, now| {
            state.collection(&collection)?;
            let variable = state
                .variables
                .iter_mut()
                .find(|variable| variable.collection == collection && variable.id == id)
                .ok_or_else(|| StoreError::not_found(Entity::Variable, &id))?;
            variable.name = edit.name;
            variable.value = edit.value;
            variable.enabled = edit.enabled;
            variable.updated_at = now;
            let updated = variable.clone();
            state.touch(&collection, now)?;
            Ok(updated)
        })
    }

    fn delete(&self, collection: CollectionId, ids: Vec<VariableId>) -> StoreFuture<()> {
        self.run(move |state, now| {
            state.collection(&collection)?;
            state
                .variables
                .retain(|variable| variable.collection != collection || !ids.contains(&variable.id));
            state.touch(&collection, now)
        })
    }
}

impl PrefStore for MemoryStore {
    fn app(&self) -> StoreFuture<AppPrefs> {
        self.run(|state, _| Ok(state.app.clone()))
    }

    fn set_app(&self, prefs: AppPrefs) -> StoreFuture<()> {
        self.run(move |state, _| {
            state.app = prefs;
            Ok(())
        })
    }

    fn workspace(&self, collection: CollectionId) -> StoreFuture<WorkspacePrefs> {
        self.run(move |state, _| {
            state.collection(&collection)?;
            Ok(state.workspaces.get(&collection).cloned().unwrap_or_default())
        })
    }

    fn set_workspace(&self, collection: CollectionId, prefs: WorkspacePrefs) -> StoreFuture<()> {
        self.run(move |state, _| {
            state.collection(&collection)?;
            state.workspaces.insert(collection, prefs);
            Ok(())
        })
    }
}

#[cfg(feature = "conformance")]
impl crate::conformance::Probe for MemoryStore {
    fn rows_of(&self, collection: CollectionId) -> StoreFuture<usize> {
        self.run(move |state, _| {
            Ok(
                usize::from(state.collections.iter().any(|found| found.id == collection))
                    + state
                        .folders
                        .iter()
                        .filter(|folder| folder.collection == collection)
                        .count()
                    + state.apis.iter().filter(|api| api.collection == collection).count()
                    + state
                        .variables
                        .iter()
                        .filter(|variable| variable.collection == collection)
                        .count()
                    + usize::from(state.top_orders.contains_key(&collection))
                    + usize::from(state.workspaces.contains_key(&collection)),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    /// A clock that moves a second each time it is read.
    fn ticking() -> MemoryStore {
        let seconds = Arc::new(AtomicI64::new(1_700_000_000));
        MemoryStore::with_clock(move || {
            let at = seconds.fetch_add(1, Ordering::SeqCst);
            Timestamp::new(
                chrono::DateTime::from_timestamp(at, 0)
                    .unwrap_or_default()
                    .fixed_offset(),
            )
        })
    }

    #[test]
    fn content_changes_move_the_collection_forward() {
        let storage = ticking().storage();
        pollster::block_on(async {
            let collection = storage
                .collections()
                .create(NewCollection {
                    name: "p".into(),
                    ..Default::default()
                })
                .await
                .expect("create");
            let api = storage
                .tree()
                .create_api(collection.id.clone(), None, "a".into())
                .await
                .expect("create api");
            let after_create = storage.collections().get(collection.id.clone()).await.expect("get");
            assert!(after_create.updated_at > collection.updated_at);
            assert_eq!(after_create.created_at, collection.created_at);

            storage
                .tree()
                .save_request(collection.id.clone(), api.item.id, HttpRequest::default())
                .await
                .expect("save");
            let after_save = storage.collections().get(collection.id.clone()).await.expect("get");
            assert!(after_save.updated_at > after_create.updated_at);

            // A reader's own state is not the project's content.
            storage
                .prefs()
                .set_workspace(collection.id.clone(), WorkspacePrefs::default())
                .await
                .expect("prefs");
            let after_prefs = storage.collections().get(collection.id).await.expect("get");
            assert_eq!(after_prefs.updated_at, after_save.updated_at);
        });
    }
}
