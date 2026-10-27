//! Everything in memory, in insertion order: the reference backend the
//! conformance suite is checked against, and the store tests of the layers
//! above use.

use crate::rules::{environment_name, known_environment, project_config, required_name};
use crate::{
    BackupStore, CollectionEdit, CollectionStore, CookieStore, Entity, HistoryStore, Imported, Inserted, NewCollection,
    NewVariable, PrefStore, ServerCopyStore, Storage, StoreError, StoreFuture, TreeStore, VariableEdit, VariableStore,
    sort_cookies,
};
use chrono::Utc;
use cyberapi_core::backup::Backup;
use cyberapi_core::cookie::Cookie;
use cyberapi_core::environments::reordered;
use cyberapi_core::import::{ImportItem, assign_ids};
use cyberapi_core::model::{
    ApiCategory, ApiSetting, Collection, Environment, Folder, FolderSettings, HttpRequest, ProjectSettings, Variable,
    VariableCategory,
};
use cyberapi_core::oauth::KeptTokens;
use cyberapi_core::prefs::{AppPrefs, WorkspacePrefs};
use cyberapi_core::response::HttpResponse;
use cyberapi_core::sync::{ConfigAt, CopyBinding, ProjectConfig};
use cyberapi_core::tree::{Placement, Removal, TreeChange, TreeData};
use cyberapi_core::{CollectionId, EnvironmentId, ItemId, Timestamp, VariableId};
use std::collections::{BTreeMap, HashMap, HashSet};
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
    /// What each project sets for its APIs (#66); none is no entry.
    project_settings: HashMap<CollectionId, ProjectSettings>,
    variables: Vec<Variable>,
    /// Every project's named environments (#61), each project's in its
    /// order.
    environments: Vec<Environment>,
    app: AppPrefs,
    workspaces: HashMap<CollectionId, WorkspacePrefs>,
    /// C4.
    oauth_tokens: KeptTokens,
    /// Oldest first.
    responses: Vec<HttpResponse>,
    cookies: Vec<Cookie>,
    /// The desktop's copies of server projects.
    copies: Vec<CopyBinding>,
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

    /// Holds `collection` with `config` as all it has, in place of whatever
    /// the store had of it: a page's working copy of a server project
    /// (`docs/server-sync.md` §4), filled from the server or from a draft.
    /// Ids and times stay as they come, and the project's responses go.
    pub fn hold(&self, collection: Collection, config: ProjectConfig) -> Result<(), StoreError> {
        let mut state = self.lock()?;
        let id = collection.id.clone();
        let owned = |found: &CollectionId| found == &id;
        state.collections.retain(|found| !owned(&found.id));
        state.folders.retain(|folder| !owned(&folder.collection));
        state.apis.retain(|api| !owned(&api.collection));
        state.variables.retain(|variable| !owned(&variable.collection));
        state.environments.retain(|environment| !owned(&environment.collection));
        state.responses.retain(|response| !owned(&response.collection));
        state.folders.extend(config.folders.into_iter().map(|folder| Folder {
            collection: id.clone(),
            ..folder
        }));
        state.apis.extend(config.apis.into_iter().map(|api| ApiSetting {
            collection: id.clone(),
            ..api
        }));
        state
            .variables
            .extend(config.variables.into_iter().map(|variable| Variable {
                collection: id.clone(),
                ..variable
            }));
        state.top_orders.insert(id.clone(), config.top_order);
        state.project_settings.insert(id.clone(), config.settings);
        state
            .environments
            .extend(config.environments.into_iter().map(|environment| Environment {
                collection: id.clone(),
                ..environment
            }));
        state.collections.push(Collection {
            name: config.name,
            description: config.description,
            ..collection
        });
        Ok(())
    }

    /// What the store has of `collection` now, whole: a working copy as it
    /// is kept and pushed.
    pub fn project(&self, collection: &CollectionId) -> Result<ProjectConfig, StoreError> {
        Ok(self.lock()?.config_of(collection)?.config)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>, StoreError> {
        self.inner
            .state
            .lock()
            .map_err(|_| StoreError::backend("the memory store's lock was poisoned"))
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

    fn config_of(&self, collection: &CollectionId) -> Result<ConfigAt, StoreError> {
        let found = self.collection(collection)?;
        let tree = self.tree_data(collection);
        Ok(ConfigAt {
            revision: found.revision,
            config: ProjectConfig {
                name: found.name.clone(),
                description: found.description.clone(),
                folders: tree.folders,
                apis: tree.apis,
                top_order: tree.top_order,
                settings: tree.settings,
                environments: self.environments_of(collection),
                variables: self
                    .variables
                    .iter()
                    .filter(|variable| &variable.collection == collection)
                    .cloned()
                    .collect(),
            },
        })
    }

    /// A configuration in place of the project's (`TreeStore::replace`).
    fn replace_project(
        &mut self,
        collection: CollectionId,
        config: ProjectConfig,
        base: Option<u64>,
        now: Timestamp,
    ) -> Result<u64, StoreError> {
        let revision = self.collection(&collection)?.revision;
        let config = project_config(&collection, config)?;
        if let Some(base) = base
            && base != revision
        {
            return Err(StoreError::Stale { base, revision });
        }
        let elsewhere = |id: &ItemId| {
            self.folders
                .iter()
                .any(|folder| &folder.id == id && folder.collection != collection)
                || self
                    .apis
                    .iter()
                    .any(|api| &api.id == id && api.collection != collection)
        };
        let items = config.folders.iter().map(|folder| &folder.id);
        if let Some(id) = items
            .chain(config.apis.iter().map(|api| &api.id))
            .find(|id| elsewhere(id))
        {
            return Err(StoreError::IdTaken { id: id.to_string() });
        }
        if let Some(environment) = config.environments.iter().find(|environment| {
            self.environments
                .iter()
                .any(|found| found.id == environment.id && found.collection != collection)
        }) {
            return Err(StoreError::IdTaken {
                id: environment.id.to_string(),
            });
        }
        if let Some(variable) = config.variables.iter().find(|variable| {
            self.variables
                .iter()
                .any(|found| found.id == variable.id && found.collection != collection)
        }) {
            return Err(StoreError::IdTaken {
                id: variable.id.to_string(),
            });
        }
        // The history of an API that stays stays; of one that goes, goes.
        let staying: HashSet<&ItemId> = config.apis.iter().map(|api| &api.id).collect();
        self.responses
            .retain(|response| response.collection != collection || staying.contains(&response.api));
        self.folders.retain(|folder| folder.collection != collection);
        self.folders.extend(config.folders);
        self.apis.retain(|api| api.collection != collection);
        self.apis.extend(config.apis);
        self.variables.retain(|variable| variable.collection != collection);
        self.variables.extend(config.variables);
        self.environments
            .retain(|environment| environment.collection != collection);
        self.environments.extend(config.environments);
        self.top_orders.insert(collection.clone(), config.top_order);
        self.project_settings.insert(collection.clone(), config.settings);
        let found = self.collection_mut(&collection)?;
        found.name = config.name;
        found.description = config.description;
        found.updated_at = now;
        found.revision += 1;
        Ok(found.revision)
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
            settings: self.project_settings.get(id).cloned().unwrap_or_default(),
        }
    }

    /// A project's named environments, in its order.
    fn environments_of(&self, collection: &CollectionId) -> Vec<Environment> {
        self.environments
            .iter()
            .filter(|environment| &environment.collection == collection)
            .cloned()
            .collect()
    }

    /// `id` is one of `collection`'s environments.
    fn known_environment(&self, collection: &CollectionId, id: Option<&EnvironmentId>) -> Result<(), StoreError> {
        match id {
            Some(id) => known_environment(&self.environments_of(collection), id),
            None => Ok(()),
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

    fn api_counts(&self) -> StoreFuture<BTreeMap<CollectionId, usize>> {
        self.run(|state, _| {
            let mut counts = BTreeMap::new();
            for api in &state.apis {
                *counts.entry(api.collection.clone()).or_insert(0) += 1;
            }
            Ok(counts)
        })
    }

    fn create(&self, input: NewCollection) -> StoreFuture<Collection> {
        self.run(move |state, now| {
            let name = required_name(&input.name)?;
            if let Some(id) = &input.id
                && state.collections.iter().any(|collection| &collection.id == id)
            {
                return Err(StoreError::IdTaken { id: id.to_string() });
            }
            let collection = Collection {
                id: input.id.unwrap_or_else(|| CollectionId::new(new_id())),
                name,
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
            state.environments.retain(|environment| environment.collection != id);
            state.top_orders.remove(&id);
            state.project_settings.remove(&id);
            state.workspaces.remove(&id);
            state.responses.retain(|response| response.collection != id);
            state.copies.retain(|binding| binding.collection != id);
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
                settings: Default::default(),
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

    fn set_folder_settings(
        &self,
        collection: CollectionId,
        folder: ItemId,
        settings: FolderSettings,
    ) -> StoreFuture<Folder> {
        self.run(move |state, now| {
            state.collection(&collection)?;
            let found = state
                .folders
                .iter_mut()
                .find(|found| found.collection == collection && found.id == folder)
                .ok_or_else(|| StoreError::not_found(Entity::Item, &folder))?;
            found.settings = settings;
            found.updated_at = now;
            let saved = found.clone();
            state.touch(&collection, now)?;
            Ok(saved)
        })
    }

    fn set_project_settings(&self, collection: CollectionId, settings: ProjectSettings) -> StoreFuture<()> {
        self.run(move |state, now| {
            state.collection(&collection)?;
            state.project_settings.insert(collection.clone(), settings);
            state.touch(&collection, now)
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

    fn import(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        items: Vec<ImportItem>,
    ) -> StoreFuture<Imported> {
        self.run(move |state, now| {
            state.collection(&collection)?;
            let assigned = assign_ids(&items, &mut || ItemId::new(new_id()));
            let change = state
                .tree_data(&collection)
                .plan_import(&assigned.top, parent.as_ref())?;
            let folders: Vec<Folder> = assigned
                .folders
                .into_iter()
                .map(|(id, name, children, settings)| Folder {
                    id,
                    collection: collection.clone(),
                    name,
                    children,
                    settings,
                    created_at: now,
                    updated_at: now,
                })
                .collect();
            let apis: Vec<ApiSetting> = assigned
                .apis
                .into_iter()
                .map(|(id, name, request)| ApiSetting {
                    id,
                    collection: collection.clone(),
                    name,
                    category: ApiCategory::Http,
                    request,
                    created_at: now,
                    updated_at: now,
                })
                .collect();
            state.folders.extend(folders.clone());
            state.apis.extend(apis.clone());
            state.apply(&collection, &change);
            state.touch(&collection, now)?;
            Ok(Imported { folders, apis, change })
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
            state
                .responses
                .retain(|response| response.collection != collection || !removal.apis.contains(&response.api));
            state.apply(&collection, &removal.change);
            state.touch(&collection, now)?;
            Ok(removal)
        })
    }

    fn config(&self, collection: CollectionId) -> StoreFuture<ConfigAt> {
        self.run(move |state, _| state.config_of(&collection))
    }

    fn replace(&self, collection: CollectionId, config: ProjectConfig, base: Option<u64>) -> StoreFuture<u64> {
        self.run(move |state, now| state.replace_project(collection, config, base, now))
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
            state.known_environment(&input.collection, input.environment.as_ref())?;
            let variable = Variable {
                id: VariableId::new(new_id()),
                collection: input.collection.clone(),
                category: input.category,
                name: input.name,
                value: input.value,
                enabled: input.enabled,
                environment: input.environment,
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
            state.known_environment(&collection, edit.environment.as_ref())?;
            let variable = state
                .variables
                .iter_mut()
                .find(|variable| variable.collection == collection && variable.id == id)
                .ok_or_else(|| StoreError::not_found(Entity::Variable, &id))?;
            variable.name = edit.name;
            variable.value = edit.value;
            variable.enabled = edit.enabled;
            variable.environment = edit.environment;
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

    fn environments(&self, collection: CollectionId) -> StoreFuture<Vec<Environment>> {
        self.run(move |state, _| {
            state.collection(&collection)?;
            Ok(state.environments_of(&collection))
        })
    }

    fn create_environment(&self, collection: CollectionId, name: String) -> StoreFuture<Environment> {
        self.run(move |state, now| {
            state.collection(&collection)?;
            let name = environment_name(&state.environments_of(&collection), &name, None)?;
            let environment = Environment {
                id: EnvironmentId::new(new_id()),
                collection: collection.clone(),
                name,
                created_at: now,
                updated_at: now,
            };
            state.environments.push(environment.clone());
            state.touch(&collection, now)?;
            Ok(environment)
        })
    }

    fn rename_environment(
        &self,
        collection: CollectionId,
        id: EnvironmentId,
        name: String,
    ) -> StoreFuture<Environment> {
        self.run(move |state, now| {
            state.collection(&collection)?;
            let name = environment_name(&state.environments_of(&collection), &name, Some(&id))?;
            let found = state
                .environments
                .iter_mut()
                .find(|environment| environment.collection == collection && environment.id == id)
                .ok_or_else(|| StoreError::not_found(Entity::Environment, &id))?;
            found.name = name;
            found.updated_at = now;
            let renamed = found.clone();
            state.touch(&collection, now)?;
            Ok(renamed)
        })
    }

    fn delete_environment(&self, collection: CollectionId, id: EnvironmentId) -> StoreFuture<()> {
        self.run(move |state, now| {
            state.collection(&collection)?;
            state.known_environment(&collection, Some(&id))?;
            state
                .environments
                .retain(|environment| environment.collection != collection || environment.id != id);
            state
                .variables
                .retain(|variable| variable.collection != collection || variable.environment.as_ref() != Some(&id));
            state.touch(&collection, now)
        })
    }

    fn reorder_environments(
        &self,
        collection: CollectionId,
        order: Vec<EnvironmentId>,
    ) -> StoreFuture<Vec<Environment>> {
        self.run(move |state, now| {
            state.collection(&collection)?;
            let placed = reordered(&state.environments_of(&collection), &order);
            state
                .environments
                .retain(|environment| environment.collection != collection);
            state.environments.extend(placed.clone());
            state.touch(&collection, now)?;
            Ok(placed)
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

    fn clear_workspaces(&self) -> StoreFuture<()> {
        self.run(|state, _| {
            state.workspaces.clear();
            Ok(())
        })
    }

    fn oauth_tokens(&self) -> StoreFuture<KeptTokens> {
        self.run(|state, _| Ok(state.oauth_tokens.clone()))
    }

    fn set_oauth_tokens(&self, tokens: KeptTokens) -> StoreFuture<()> {
        self.run(move |state, _| {
            state.oauth_tokens = tokens;
            Ok(())
        })
    }
}

impl State {
    /// The project's API `api`, as a history's owner.
    fn require_api(&self, collection: &CollectionId, api: &ItemId) -> Result<(), StoreError> {
        self.collection(collection)?;
        if self
            .apis
            .iter()
            .any(|found| &found.collection == collection && &found.id == api)
        {
            Ok(())
        } else {
            Err(StoreError::not_found(Entity::Api, api))
        }
    }
}

impl HistoryStore for MemoryStore {
    fn add(&self, response: HttpResponse, keep: usize) -> StoreFuture<HttpResponse> {
        self.run(move |state, now| {
            state.require_api(&response.collection, &response.api)?;
            let kept = HttpResponse {
                id: new_id(),
                created_at: now,
                ..response
            };
            state.responses.push(kept.clone());
            let of_api = |response: &HttpResponse| response.collection == kept.collection && response.api == kept.api;
            let count = state.responses.iter().filter(|response| of_api(response)).count();
            let mut extra = count.saturating_sub(keep.max(1));
            state.responses.retain(|response| {
                if extra > 0 && of_api(response) {
                    extra -= 1;
                    false
                } else {
                    true
                }
            });
            Ok(kept)
        })
    }

    fn list(&self, collection: CollectionId, api: ItemId) -> StoreFuture<Vec<HttpResponse>> {
        self.run(move |state, _| {
            state.require_api(&collection, &api)?;
            Ok(state
                .responses
                .iter()
                .rev()
                .filter(|response| response.collection == collection && response.api == api)
                .cloned()
                .collect())
        })
    }

    fn recent(&self, collection: CollectionId, limit: usize) -> StoreFuture<Vec<HttpResponse>> {
        self.run(move |state, _| {
            state.collection(&collection)?;
            Ok(state
                .responses
                .iter()
                .rev()
                .filter(|response| response.collection == collection)
                .take(limit)
                .cloned()
                .map(HttpResponse::summary)
                .collect())
        })
    }

    fn latest(&self, collection: CollectionId, apis: Vec<ItemId>) -> StoreFuture<Vec<HttpResponse>> {
        self.run(move |state, _| {
            state.collection(&collection)?;
            Ok(apis
                .iter()
                .filter_map(|api| {
                    state
                        .responses
                        .iter()
                        .rev()
                        .find(|response| response.collection == collection && &response.api == api)
                        .cloned()
                })
                .collect())
        })
    }

    fn clear(&self, collection: CollectionId, api: ItemId) -> StoreFuture<()> {
        self.run(move |state, _| {
            state.require_api(&collection, &api)?;
            state
                .responses
                .retain(|response| response.collection != collection || response.api != api);
            Ok(())
        })
    }

    fn clear_all(&self) -> StoreFuture<()> {
        self.run(|state, _| {
            state.responses.clear();
            Ok(())
        })
    }
}

impl ServerCopyStore for MemoryStore {
    fn copies(&self) -> StoreFuture<Vec<CopyBinding>> {
        self.run(|state, _| Ok(state.copies.clone()))
    }

    fn bind(&self, binding: CopyBinding) -> StoreFuture<()> {
        self.run(move |state, _| {
            state.copies.retain(|found| found.collection != binding.collection);
            state.copies.push(binding);
            Ok(())
        })
    }

    fn unbind(&self, collection: CollectionId) -> StoreFuture<()> {
        self.run(move |state, _| {
            state.copies.retain(|found| found.collection != collection);
            Ok(())
        })
    }

    fn keep_copy(&self, collection: Collection, config: ProjectConfig) -> StoreFuture<()> {
        self.run(move |state, now| {
            let id = collection.id.clone();
            if state.collection(&id).is_err() {
                state.collections.push(collection);
            }
            state.replace_project(id, config, None, now).map(|_| ())
        })
    }
}

impl BackupStore for MemoryStore {
    fn snapshot(&self) -> StoreFuture<Backup> {
        self.run(|state, _| {
            Ok(Backup {
                collections: state.collections.clone(),
                folders: state.folders.clone(),
                apis: state.apis.clone(),
                variables: state.variables.clone(),
                environments: state.environments.clone(),
                top_orders: state
                    .collections
                    .iter()
                    .filter_map(|collection| {
                        state
                            .top_orders
                            .get(&collection.id)
                            .map(|order| (collection.id.clone(), order.clone()))
                    })
                    .collect(),
                project_settings: state
                    .collections
                    .iter()
                    .filter_map(|collection| {
                        state
                            .project_settings
                            .get(&collection.id)
                            .filter(|settings| !settings.is_empty())
                            .map(|settings| (collection.id.clone(), settings.clone()))
                    })
                    .collect(),
            })
        })
    }

    fn restore(&self, backup: Backup) -> StoreFuture<()> {
        self.run(move |state, _| {
            state.collections = backup.collections;
            state.folders = backup.folders;
            state.apis = backup.apis;
            state.variables = backup.variables;
            state.environments = backup.environments;
            state.top_orders = backup.top_orders.into_iter().collect();
            state.project_settings = backup.project_settings.into_iter().collect();
            state.responses.clear();
            Ok(())
        })
    }
}

impl CookieStore for MemoryStore {
    fn list(&self) -> StoreFuture<Vec<Cookie>> {
        self.run(|state, _| {
            let mut cookies = state.cookies.clone();
            sort_cookies(&mut cookies);
            Ok(cookies)
        })
    }

    fn put(&self, cookies: Vec<Cookie>) -> StoreFuture<()> {
        self.run(move |state, now| {
            for cookie in cookies {
                state.cookies.retain(|kept| kept.key() != cookie.key());
                if !cookie.is_expired(now) {
                    state.cookies.push(cookie);
                }
            }
            Ok(())
        })
    }

    fn delete(&self, domain: String, path: String, name: String) -> StoreFuture<()> {
        self.run(move |state, _| {
            state
                .cookies
                .retain(|kept| kept.key() != (domain.as_str(), path.as_str(), name.as_str()));
            Ok(())
        })
    }

    fn clear(&self) -> StoreFuture<()> {
        self.run(|state, _| {
            state.cookies.clear();
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
                    + usize::from(state.project_settings.contains_key(&collection))
                    + state
                        .environments
                        .iter()
                        .filter(|environment| environment.collection == collection)
                        .count()
                    + usize::from(state.workspaces.contains_key(&collection))
                    + state
                        .responses
                        .iter()
                        .filter(|response| response.collection == collection)
                        .count(),
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

    #[test]
    fn a_held_project_is_what_it_was_given() {
        let server = ticking();
        let storage = server.storage();
        let (collection, config) = pollster::block_on(async {
            let collection = storage
                .collections()
                .create(NewCollection {
                    name: "shop".into(),
                    ..Default::default()
                })
                .await
                .expect("create");
            let api = storage
                .tree()
                .create_api(collection.id.clone(), None, "login".into())
                .await
                .expect("api");
            storage
                .history()
                .add(
                    HttpResponse {
                        collection: collection.id.clone(),
                        api: api.item.id,
                        ..Default::default()
                    },
                    cyberapi_core::response::HISTORY_LIMIT,
                )
                .await
                .expect("history");
            let at = storage.tree().config(collection.id.clone()).await.expect("config");
            (collection, at.config)
        });

        let copy = ticking();
        copy.hold(collection.clone(), config.clone()).expect("hold");
        assert_eq!(copy.project(&collection.id).expect("held"), config);
        let held = pollster::block_on(copy.storage().collections().get(collection.id.clone())).expect("get");
        assert_eq!(held.revision, collection.revision, "as it came");

        // Held again: in place of what it had, and none of its history.
        copy.hold(
            collection.clone(),
            ProjectConfig {
                name: "shop 2".into(),
                ..ProjectConfig::default()
            },
        )
        .expect("hold again");
        let again = copy.project(&collection.id).expect("held");
        assert_eq!((again.name.as_str(), again.apis.len()), ("shop 2", 0));
        let projects = pollster::block_on(copy.storage().collections().list()).expect("list");
        assert_eq!(projects.len(), 1);
        assert!(copy.project(&CollectionId::new("other")).is_err());
    }
}
