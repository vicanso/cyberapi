//! One category of a project's variables, as its dialog edits them (M8):
//! environment variables, custom variables or global request headers.
//!
//! Each row is one stored variable (F8.2): adding makes one at once (blank),
//! typing updates it 300 ms after the last key, turning it on or off and
//! removing it are immediate. The list is the store's order — by name — as
//! it was when the dialog opened; a row does not jump while it is renamed.
//!
//! It shows one layer at a time (#61): the variables every environment
//! shares, or one named environment's. What is added goes into the layer
//! shown. The environments themselves are made, renamed and deleted here.
//!
//! A secret variable's value (D3) is the reader's own, kept by the
//! workspace: its row keeps none, and a viewer types theirs too.

use super::{Load, WorkspaceState, run_store};
use cyberapi_core::model::{Environment, Variable, VariableCategory};
use cyberapi_core::{CollectionId, EnvironmentId, VariableId};
use cyberapi_store::{NewVariable, Storage, StoreError, StoreFuture, VariableEdit};
use gpui_kit::{AppContext as _, Context, EventEmitter, Task, WeakEntity};
use std::collections::HashMap;
use std::time::Duration;

const SAVE_DELAY: Duration = Duration::from_millis(300);

#[derive(Clone, Debug)]
pub enum VariablesEvent {
    Failed(StoreError),
}

pub struct VariablesState {
    storage: Storage,
    workspace: WeakEntity<WorkspaceState>,
    collection: CollectionId,
    category: VariableCategory,
    /// Every layer's.
    variables: Vec<Variable>,
    load: Load,
    /// The project's named environments, in order.
    environments: Vec<Environment>,
    /// The layer shown: `None` is the shared one.
    layer: Option<EnvironmentId>,
    /// An update waiting for typing to pause, per variable.
    saving: HashMap<VariableId, Task<()>>,
}

impl EventEmitter<VariablesEvent> for VariablesState {}

impl VariablesState {
    pub fn new(
        storage: Storage,
        workspace: WeakEntity<WorkspaceState>,
        collection: CollectionId,
        category: VariableCategory,
        layer: Option<EnvironmentId>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Typing still waiting when the dialog goes is saved then.
        cx.on_release(|this, cx| {
            let _ = this.workspace.update(cx, |workspace, cx| workspace.flush_secrets(cx));
            let calls: Vec<_> = this
                .saving
                .keys()
                .filter_map(|id| this.variables.iter().find(|variable| &variable.id == id))
                .map(|variable| {
                    this.storage.variables().update(
                        this.collection.clone(),
                        variable.id.clone(),
                        VariableEdit {
                            name: variable.name.clone(),
                            value: variable.value.clone(),
                            enabled: variable.enabled,
                            environment: variable.environment.clone(),
                        },
                    )
                })
                .collect();
            if calls.is_empty() {
                return;
            }
            let (workspace, category) = (this.workspace.clone(), this.category);
            cx.spawn(async move |cx| {
                for call in calls {
                    if let Err(error) = call.await {
                        tracing::warn!(%error, "a variable could not be saved");
                    }
                }
                let _ = workspace.update(cx, |workspace, cx| workspace.refresh_variables(category, cx));
            })
            .detach();
        })
        .detach();
        let call = storage.variables().list(collection.clone(), category);
        run_store(cx, call, |this, result, cx| {
            match result {
                Ok(variables) => {
                    this.variables = variables;
                    this.load = Load::Loaded;
                }
                Err(error) => {
                    this.load = Load::Failed(error.clone());
                    cx.emit(VariablesEvent::Failed(error));
                }
            }
            cx.notify();
        });
        let call = storage.variables().environments(collection.clone());
        run_store(cx, call, |this, result, cx| match result {
            Ok(environments) => {
                // A layer that is not there any more is the shared one.
                if this
                    .layer
                    .as_ref()
                    .is_some_and(|id| !environments.iter().any(|environment| &environment.id == id))
                {
                    this.layer = None;
                }
                this.environments = environments;
                cx.notify();
            }
            Err(error) => cx.emit(VariablesEvent::Failed(error)),
        });
        Self {
            storage,
            workspace,
            collection,
            category,
            variables: Vec::new(),
            load: Load::Loading,
            environments: Vec::new(),
            layer,
            saving: HashMap::new(),
        }
    }

    pub fn category(&self) -> VariableCategory {
        self.category
    }

    /// Every layer's.
    pub fn variables(&self) -> &[Variable] {
        &self.variables
    }

    /// The layer shown's.
    pub fn shown(&self) -> impl Iterator<Item = &Variable> {
        self.variables
            .iter()
            .filter(|variable| variable.environment == self.layer)
    }

    /// #61: the shared variables the environment shown stands in for — a
    /// name it has enabled — to compare with; none on the shared layer.
    pub fn stood_in_for(&self) -> Vec<&Variable> {
        let Some(layer) = &self.layer else {
            return Vec::new();
        };
        let effective = cyberapi_core::environments::effective(&self.variables, Some(layer));
        self.variables
            .iter()
            .filter(|variable| variable.environment.is_none())
            .filter(|variable| !effective.iter().any(|found| found.id == variable.id))
            .collect()
    }

    pub fn environments(&self) -> &[Environment] {
        &self.environments
    }

    pub fn layer(&self) -> Option<&EnvironmentId> {
        self.layer.as_ref()
    }

    pub fn set_layer(&mut self, layer: Option<EnvironmentId>, cx: &mut Context<Self>) {
        self.layer = layer;
        cx.notify();
    }

    /// A change to the environments: once it is in, the workspace reads
    /// them again, and every category — an environment that goes takes the
    /// variables in it — whether or not the dialog is still open; `done`
    /// applies it here while it is.
    fn change_environments<R: Send + 'static>(
        &self,
        call: StoreFuture<R>,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, R, &mut Context<Self>) + 'static,
    ) {
        let workspace = self.workspace.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(call).await;
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.refresh_environments(cx);
                for category in VariableCategory::ALL {
                    workspace.refresh_variables(category, cx);
                }
            });
            let _ = this.update(cx, |this, cx| match result {
                Ok(answer) => done(this, answer, cx),
                Err(error) => cx.emit(VariablesEvent::Failed(error)),
            });
        })
        .detach();
    }

    /// #61: a new environment, shown once made.
    pub fn create_environment(&mut self, name: String, cx: &mut Context<Self>) {
        let call = self
            .storage
            .variables()
            .create_environment(self.collection.clone(), name);
        self.change_environments(call, cx, |this, environment, cx| {
            this.layer = Some(environment.id.clone());
            this.environments.push(environment);
            cx.notify();
        });
    }

    pub fn rename_environment(&mut self, id: EnvironmentId, name: String, cx: &mut Context<Self>) {
        let call = self
            .storage
            .variables()
            .rename_environment(self.collection.clone(), id, name);
        self.change_environments(call, cx, |this, renamed, cx| {
            if let Some(found) = this.environments.iter_mut().find(|found| found.id == renamed.id) {
                *found = renamed;
            }
            cx.notify();
        });
    }

    /// #61: an environment and the variables in it, of every category.
    pub fn delete_environment(&mut self, id: EnvironmentId, cx: &mut Context<Self>) {
        let call = self
            .storage
            .variables()
            .delete_environment(self.collection.clone(), id.clone());
        self.change_environments(call, cx, move |this, (), cx| {
            let removed_ids: Vec<VariableId> = this
                .variables
                .iter()
                .filter(|variable| variable.environment.as_ref() == Some(&id))
                .map(|variable| variable.id.clone())
                .collect();
            this.environments.retain(|environment| environment.id != id);
            this.variables
                .retain(|variable| variable.environment.as_ref() != Some(&id));
            if !removed_ids.is_empty() {
                let _ = this
                    .workspace
                    .update(cx, |workspace, cx| workspace.remove_secret_variables(&removed_ids, cx));
            }
            if this.layer.as_ref() == Some(&id) {
                this.layer = None;
            }
            cx.notify();
        });
    }

    pub fn load(&self) -> &Load {
        &self.load
    }

    /// The workspace keeps the environment for the URI's prefix (F4.4) and
    /// the global request headers for the file they may ask for.
    fn changed(&self, cx: &mut Context<Self>) {
        let category = self.category;
        let _ = self
            .workspace
            .update(cx, |workspace, cx| workspace.refresh_variables(category, cx));
        cx.notify();
    }

    /// A blank, enabled variable at the end of the layer shown.
    pub fn add(&mut self, cx: &mut Context<Self>) {
        self.create_at(self.layer.clone(), String::new(), String::new(), cx);
    }

    /// D8: creates an enabled variable in `environment` with `name` and `value`.
    /// When another layer already marks `name` as secret (BUG-4), the new row
    /// is created with an empty stored value, marked secret, and its value is
    /// stored in the reader's local `SecretValues`.
    pub fn create_at(
        &mut self,
        environment: Option<EnvironmentId>,
        name: String,
        value: String,
        cx: &mut Context<Self>,
    ) {
        let secret = self.is_name_secret(&name, cx);
        let stored_value = if secret { String::new() } else { value.clone() };
        let call = self.storage.variables().create(NewVariable {
            collection: self.collection.clone(),
            category: self.category,
            name,
            value: stored_value,
            enabled: true,
            environment,
        });
        run_store(cx, call, move |this, result, cx| match result {
            Ok(mut variable) => {
                if secret {
                    variable.value = value;
                    let _ = this
                        .workspace
                        .update(cx, |workspace, cx| workspace.set_secret(&variable, true, cx));
                    variable.value = String::new();
                }
                this.variables.push(variable);
                this.changed(cx);
            }
            Err(error) => cx.emit(VariablesEvent::Failed(error)),
        });
    }

    /// D8: how many redundant duplicate variables exist within the same layer.
    pub fn duplicate_count(&self) -> usize {
        cyberapi_core::environments::duplicate_ids(&self.variables, self.category).len()
    }

    /// D8: removes redundant duplicate variables within the same layer, keeping
    /// the primary entry for each name and layer and preserving any secret
    /// flag/value from duplicate entries (BUG-4).
    pub fn deduplicate(&mut self, cx: &mut Context<Self>) -> usize {
        let ids = cyberapi_core::environments::duplicate_ids(&self.variables, self.category);
        let count = ids.len();
        if count == 0 {
            return 0;
        }
        // If a duplicate row was marked secret, transfer its secret status and
        // value (if the kept primary row has none) to the primary row before
        // removing the duplicate IDs.
        let mut promote: Vec<(VariableId, Option<String>)> = Vec::new();
        for dup in self.variables.iter().filter(|v| ids.contains(&v.id)) {
            if self.is_secret(&dup.id, cx)
                && let Some(primary) = self.variables.iter().find(|v| {
                    !ids.contains(&v.id)
                        && v.category == dup.category
                        && v.environment == dup.environment
                        && v.name.trim() == dup.name.trim()
                })
            {
                let dup_val = self.shown_value(dup, cx);
                let primary_val = self.shown_value(primary, cx);
                let migrated = (primary_val.is_empty() && !dup_val.is_empty()).then_some(dup_val);
                promote.push((primary.id.clone(), migrated));
            }
        }
        for (primary_id, migrated_val) in promote {
            if !self.is_secret(&primary_id, cx) {
                self.set_secret_one(&primary_id, true, cx);
            }
            if let Some(val) = migrated_val {
                self.set_value(&primary_id, val, cx);
            }
        }
        for id in &ids {
            self.saving.remove(id);
        }
        self.variables.retain(|variable| !ids.contains(&variable.id));
        let clean_ids = ids.clone();
        let _ = self
            .workspace
            .update(cx, |workspace, cx| workspace.remove_secret_variables(&clean_ids, cx));
        let call = self.storage.variables().delete(self.collection.clone(), ids);
        run_store(cx, call, |this, result, cx| match result {
            Ok(()) => this.changed(cx),
            Err(error) => cx.emit(VariablesEvent::Failed(error)),
        });
        cx.notify();
        count
    }

    pub fn set_name(&mut self, id: &VariableId, name: String, cx: &mut Context<Self>) {
        let should_be_secret = !self.is_secret(id, cx) && self.is_name_secret(&name, cx);
        self.edit(id, |variable| variable.name = name, false, cx);
        if should_be_secret {
            self.set_secret_one(id, true, cx);
        }
    }

    pub fn set_value(&mut self, id: &VariableId, value: String, cx: &mut Context<Self>) {
        if !self.is_secret(id, cx) {
            let name_secret = self
                .variables
                .iter()
                .find(|v| &v.id == id)
                .map(|v| v.name.clone())
                .is_some_and(|name| self.is_name_secret(&name, cx));
            if name_secret {
                self.set_secret_one(id, true, cx);
            }
        }
        if self.is_secret(id, cx) {
            let id = id.clone();
            let _ = self
                .workspace
                .update(cx, |workspace, cx| workspace.set_secret_value(id, value, cx));
            return;
        }
        self.edit(id, |variable| variable.value = value, false, cx);
    }

    /// D3: whether `id` is secret: its value is the reader's own.
    pub fn is_secret(&self, id: &VariableId, cx: &gpui_kit::App) -> bool {
        self.workspace
            .upgrade()
            .is_some_and(|workspace| workspace.read(cx).is_secret(id))
    }

    /// BUG-4: whether any variable named `name` in this category is marked
    /// secret across any layer (Shared or named environment).
    pub fn is_name_secret(&self, name: &str, cx: &gpui_kit::App) -> bool {
        let name = name.trim();
        !name.is_empty()
            && self.variables.iter().any(|variable| {
                variable.category == self.category && variable.name.trim() == name && self.is_secret(&variable.id, cx)
            })
    }

    /// The value a row shows: the reader's own, for a secret one.
    pub fn shown_value(&self, variable: &Variable, cx: &gpui_kit::App) -> String {
        match self.workspace.upgrade() {
            Some(workspace) if workspace.read(cx).is_secret(&variable.id) => workspace
                .read(cx)
                .secret_value(&variable.id)
                .unwrap_or_default()
                .to_string(),
            _ => variable.value.clone(),
        }
    }

    fn set_secret_one(&mut self, id: &VariableId, secret: bool, cx: &mut Context<Self>) {
        let Some(variable) = self.variables.iter().find(|variable| &variable.id == id).cloned() else {
            return;
        };
        let kept = self
            .workspace
            .update(cx, |workspace, cx| workspace.set_secret(&variable, secret, cx))
            .ok()
            .flatten();
        if let Some(kept) = kept {
            self.edit(id, |variable| variable.value = kept, true, cx);
            cx.notify();
        }
    }

    /// D3 / BUG-4: `id` (and any same-named variable in this category across
    /// environments) made secret, its value moved to the reader's own and the
    /// row keeping none; or not, the reader's value the row's again.
    pub fn set_secret(&mut self, id: &VariableId, secret: bool, cx: &mut Context<Self>) {
        let Some(target) = self.variables.iter().find(|variable| &variable.id == id).cloned() else {
            return;
        };
        let trimmed = target.name.trim();
        let target_ids: Vec<VariableId> = if trimmed.is_empty() {
            vec![id.clone()]
        } else {
            self.variables
                .iter()
                .filter(|v| v.category == self.category && v.name.trim() == trimmed)
                .map(|v| v.id.clone())
                .collect()
        };
        for tid in target_ids {
            self.set_secret_one(&tid, secret, cx);
        }
    }

    /// F8.3.
    pub fn toggle(&mut self, id: &VariableId, cx: &mut Context<Self>) {
        self.edit(id, |variable| variable.enabled = !variable.enabled, true, cx);
    }

    fn edit(&mut self, id: &VariableId, change: impl FnOnce(&mut Variable), now: bool, cx: &mut Context<Self>) {
        let Some(variable) = self.variables.iter_mut().find(|variable| &variable.id == id) else {
            return;
        };
        let before = variable.clone();
        change(variable);
        if *variable == before {
            return;
        }
        let id = id.clone();
        if now {
            self.save(id, cx);
        } else {
            let task = cx.spawn({
                let id = id.clone();
                async move |this, cx| {
                    cx.background_executor().timer(SAVE_DELAY).await;
                    let _ = this.update(cx, |this, cx| this.save(id, cx));
                }
            });
            self.saving.insert(id, task);
        }
        cx.notify();
    }

    fn save(&mut self, id: VariableId, cx: &mut Context<Self>) {
        self.saving.remove(&id);
        let Some(variable) = self.variables.iter().find(|variable| variable.id == id) else {
            return;
        };
        let call = self.storage.variables().update(
            self.collection.clone(),
            id,
            VariableEdit {
                name: variable.name.clone(),
                value: variable.value.clone(),
                enabled: variable.enabled,
                // An edit leaves it in its environment (#61).
                environment: variable.environment.clone(),
            },
        );
        run_store(cx, call, |this, result, cx| match result {
            Ok(_) => this.changed(cx),
            Err(error) => cx.emit(VariablesEvent::Failed(error)),
        });
    }

    /// Saves what is still waiting: the dialog is closing.
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        let waiting: Vec<VariableId> = self.saving.keys().cloned().collect();
        for id in waiting {
            self.save(id, cx);
        }
        let _ = self.workspace.update(cx, |workspace, cx| workspace.flush_secrets(cx));
    }

    pub fn delete(&mut self, id: &VariableId, cx: &mut Context<Self>) {
        self.saving.remove(id);
        self.variables.retain(|variable| &variable.id != id);
        let clean_id = id.clone();
        let _ = self.workspace.update(cx, |workspace, cx| {
            workspace.remove_secret_variables(std::slice::from_ref(&clean_id), cx)
        });
        let call = self
            .storage
            .variables()
            .delete(self.collection.clone(), vec![id.clone()]);
        run_store(cx, call, |this, result, cx| match result {
            Ok(()) => this.changed(cx),
            Err(error) => cx.emit(VariablesEvent::Failed(error)),
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_store::{MemoryStore, NewCollection};
    use gpui_kit::TestAppContext;

    /// D3: a variable made secret keeps no value in the project — the
    /// reader's is kept as theirs, typed there from then on — and made
    /// public again takes the reader's value back.
    #[gpui_kit::test]
    async fn a_secret_value_is_the_readers_own(cx: &mut TestAppContext) {
        use crate::states::WorkspaceState;
        let storage = MemoryStore::new().storage();
        let collection = storage
            .collections()
            .create(NewCollection {
                name: "p".into(),
                ..Default::default()
            })
            .await
            .expect("create");
        let id = collection.id.clone();
        storage
            .variables()
            .create(NewVariable {
                collection: id.clone(),
                category: VariableCategory::Custom,
                name: "key".into(),
                value: "abc".into(),
                enabled: true,
                environment: None,
            })
            .await
            .expect("a variable");
        struct NoEngine;
        impl cyberapi_http::HttpEngine for NoEngine {
            fn send(&self, _: cyberapi_http::PreparedRequest) -> cyberapi_http::SendFuture {
                Box::pin(std::future::pending())
            }
        }
        let sender = cyberapi_service::Sender {
            storage: storage.clone(),
            engine: std::sync::Arc::new(NoEngine),
            files: std::sync::Arc::new(cyberapi_core::template::NoFiles("no files")),
            env: std::sync::Arc::new(cyberapi_core::template::NoEnv("no environment")),
            user_agent: "CyberAPI/test".into(),
            timeouts: Default::default(),
            sending: Default::default(),
            inherited: Default::default(),
            environment: None,
            tokens: Default::default(),
            secrets: Default::default(),
        };
        let workspace = cx.new(|cx| WorkspaceState::new(sender, collection, cx));
        cx.run_until_parked();
        let state = cx.new(|cx| {
            VariablesState::new(
                storage.clone(),
                workspace.downgrade(),
                id.clone(),
                VariableCategory::Custom,
                None,
                cx,
            )
        });
        cx.run_until_parked();
        let variable = state.read_with(cx, |state, _| state.variables()[0].clone());
        let stored = || {
            pollster::block_on(storage.variables().list(id.clone(), VariableCategory::Custom)).expect("list")[0]
                .value
                .clone()
        };
        let kept = || pollster::block_on(storage.prefs().secret_values(id.clone())).expect("values");

        state.update(cx, |state, cx| state.set_secret(&variable.id, true, cx));
        cx.run_until_parked();
        assert_eq!(stored(), "", "the project keeps none");
        assert_eq!(kept().get(&variable.id), Some("abc"), "the reader's");
        workspace.read_with(cx, |workspace, _| {
            assert!(workspace.is_secret(&variable.id));
            assert_eq!(workspace.secrets().get(&variable.id), Some("abc"));
        });
        assert!(state.read_with(cx, |state, cx| state.is_secret(&variable.id, cx)));
        assert_eq!(state.read_with(cx, |state, cx| state.shown_value(&variable, cx)), "abc");

        state.update(cx, |state, cx| state.set_value(&variable.id, "xyz".into(), cx));
        cx.executor().advance_clock(SAVE_DELAY);
        cx.run_until_parked();
        assert_eq!((stored().as_str(), kept().get(&variable.id)), ("", Some("xyz")));

        state.update(cx, |state, cx| state.set_secret(&variable.id, false, cx));
        cx.run_until_parked();
        assert_eq!(stored(), "xyz", "the reader's value is the project's again");
        assert_eq!(kept().get(&variable.id), None);
        assert!(!workspace.read_with(cx, |workspace, _| workspace.is_secret(&variable.id)));
    }

    #[gpui_kit::test]
    async fn rows_are_variables_saved_as_they_change(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = storage
            .collections()
            .create(NewCollection {
                name: "p".into(),
                ..Default::default()
            })
            .await
            .expect("create")
            .id;
        let state = cx.new(|cx| {
            VariablesState::new(
                storage.clone(),
                WeakEntity::new_invalid(),
                collection.clone(),
                VariableCategory::Environment,
                None,
                cx,
            )
        });
        cx.run_until_parked();
        state.update(cx, |state, cx| state.add(cx));
        cx.run_until_parked();
        let id = state.read_with(cx, |state, _| state.variables()[0].id.clone());

        state.update(cx, |state, cx| {
            state.set_name(&id, "HOST".into(), cx);
            state.set_value(&id, "https://a.test".into(), cx);
        });
        let stored = |cx: &mut TestAppContext| {
            let _ = cx;
            pollster::block_on(
                storage
                    .variables()
                    .list(collection.clone(), VariableCategory::Environment),
            )
            .expect("list")
        };
        assert_eq!(stored(cx)[0].name, "", "typing waits");
        cx.executor().advance_clock(SAVE_DELAY);
        cx.run_until_parked();
        assert_eq!(
            (stored(cx)[0].name.as_str(), stored(cx)[0].value.as_str()),
            ("HOST", "https://a.test")
        );

        state.update(cx, |state, cx| state.toggle(&id, cx));
        cx.run_until_parked();
        assert!(!stored(cx)[0].enabled, "at once");

        state.update(cx, |state, cx| state.set_value(&id, "x".into(), cx));
        state.update(cx, |state, cx| state.flush(cx));
        cx.run_until_parked();
        assert_eq!(stored(cx)[0].value, "x", "flushed as the dialog closes");

        state.update(cx, |state, cx| state.delete(&id, cx));
        cx.run_until_parked();
        assert!(stored(cx).is_empty());
    }

    /// #61: a layer at a time; what is added goes into the layer shown, and
    /// an environment deleted takes its variables.
    #[gpui_kit::test]
    async fn environments_are_layers_of_their_own(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = storage
            .collections()
            .create(NewCollection {
                name: "p".into(),
                ..Default::default()
            })
            .await
            .expect("create")
            .id;
        let state = cx.new(|cx| {
            VariablesState::new(
                storage.clone(),
                WeakEntity::new_invalid(),
                collection.clone(),
                VariableCategory::Custom,
                None,
                cx,
            )
        });
        cx.run_until_parked();
        state.update(cx, |state, cx| state.add(cx));
        state.update(cx, |state, cx| state.create_environment("dev".into(), cx));
        cx.run_until_parked();
        let dev = state.read_with(cx, |state, _| {
            assert_eq!(state.environments().len(), 1);
            state.layer().cloned().expect("the new one shown")
        });
        state.update(cx, |state, cx| state.add(cx));
        cx.run_until_parked();
        state.read_with(cx, |state, _| {
            let shown: Vec<_> = state.shown().map(|variable| variable.environment.clone()).collect();
            assert_eq!(shown, [Some(dev.clone())], "dev's layer has dev's alone");
            assert_eq!(state.variables().len(), 2);
        });
        state.update(cx, |state, cx| state.delete_environment(dev.clone(), cx));
        cx.run_until_parked();
        state.read_with(cx, |state, _| {
            assert!(state.environments().is_empty());
            assert_eq!(state.layer(), None, "back to the shared layer");
            assert_eq!(state.variables().len(), 1);
        });
        let stored = pollster::block_on(storage.variables().list(collection, VariableCategory::Custom)).expect("list");
        assert_eq!(stored.len(), 1, "dev's went with it");
    }

    #[gpui_kit::test]
    async fn matrix_create_at_and_deduplicate_keep_secrets_out_of_variable_store(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = storage
            .collections()
            .create(NewCollection {
                name: "p".into(),
                ..Default::default()
            })
            .await
            .expect("create");
        let id = collection.id.clone();

        struct NoEngine;
        impl cyberapi_http::HttpEngine for NoEngine {
            fn send(&self, _: cyberapi_http::PreparedRequest) -> cyberapi_http::SendFuture {
                Box::pin(std::future::pending())
            }
        }
        let sender = cyberapi_service::Sender {
            storage: storage.clone(),
            engine: std::sync::Arc::new(NoEngine),
            files: std::sync::Arc::new(cyberapi_core::template::NoFiles("no files")),
            env: std::sync::Arc::new(cyberapi_core::template::NoEnv("no environment")),
            user_agent: "CyberAPI/test".into(),
            timeouts: Default::default(),
            sending: Default::default(),
            inherited: Default::default(),
            environment: None,
            tokens: Default::default(),
            secrets: Default::default(),
        };
        let workspace = cx.new(|cx| WorkspaceState::new(sender, collection, cx));
        cx.run_until_parked();
        let state = cx.new(|cx| {
            VariablesState::new(
                storage.clone(),
                workspace.downgrade(),
                id.clone(),
                VariableCategory::Environment,
                None,
                cx,
            )
        });
        cx.run_until_parked();

        // Create two environments (dev, prod)
        state.update(cx, |state, cx| state.create_environment("dev".into(), cx));
        state.update(cx, |state, cx| state.create_environment("prod".into(), cx));
        cx.run_until_parked();
        let (dev_id, prod_id) = state.read_with(cx, |state, _| {
            (state.environments()[0].id.clone(), state.environments()[1].id.clone())
        });

        // Create a variable in dev and mark it secret
        state.update(cx, |state, cx| {
            state.create_at(Some(dev_id.clone()), "API_KEY".into(), "secret-val".into(), cx);
        });
        cx.run_until_parked();
        let dev_var_id = state.read_with(cx, |state, _| state.variables()[0].id.clone());
        state.update(cx, |state, cx| state.set_secret(&dev_var_id, true, cx));
        cx.run_until_parked();

        // Now create_at in prod for the same secret name "API_KEY" with identical value "secret-val"
        state.update(cx, |state, cx| {
            state.create_at(Some(prod_id.clone()), "API_KEY".into(), "secret-val".into(), cx);
        });
        cx.run_until_parked();
        let prod_var_id = state.read_with(cx, |state, _| {
            state
                .variables()
                .iter()
                .find(|v| v.environment.as_ref() == Some(&prod_id))
                .expect("prod var")
                .id
                .clone()
        });

        // Verify prod_var_id was automatically marked secret and its plaintext is NOT in VariableStore
        let stored_vars =
            pollster::block_on(storage.variables().list(id.clone(), VariableCategory::Environment)).expect("list");
        assert!(
            stored_vars.iter().all(|v| v.value.is_empty()),
            "no secret plaintext stored in VariableStore: {stored_vars:?}"
        );
        assert!(state.read_with(cx, |state, cx| state.is_secret(&prod_var_id, cx)));
        let secret_vals = pollster::block_on(storage.prefs().secret_values(id.clone())).expect("secrets");
        assert_eq!(secret_vals.get(&dev_var_id), Some("secret-val"));
        assert_eq!(secret_vals.get(&prod_var_id), Some("secret-val"));

        // Now test deduplicate within the same layer (Shared): first row plain empty, second row secret
        state.update(cx, |state, cx| {
            state.create_at(None, "TOKEN".into(), String::new(), cx);
            state.create_at(None, "TOKEN".into(), "tok-secret".into(), cx);
        });
        cx.run_until_parked();
        let (first_tok_id, dup_tok_id) = state.read_with(cx, |state, _| {
            let toks: Vec<_> = state.variables().iter().filter(|v| v.name == "TOKEN").collect();
            assert_eq!(toks.len(), 2);
            (toks[0].id.clone(), toks[1].id.clone())
        });
        state.update(cx, |state, cx| state.set_secret(&dup_tok_id, true, cx));
        cx.run_until_parked();

        assert_eq!(state.read_with(cx, |state, _| state.duplicate_count()), 1);
        let removed = state.update(cx, |state, cx| state.deduplicate(cx));
        cx.run_until_parked();
        assert_eq!(removed, 1);

        // Verify the kept primary TOKEN variable inherited the secret flag and value,
        // and the duplicate ID was cleaned up from secret preferences.
        assert!(state.read_with(cx, |state, cx| state.is_secret(&first_tok_id, cx)));
        let secret_vals_after = pollster::block_on(storage.prefs().secret_values(id.clone())).expect("secrets");
        assert_eq!(secret_vals_after.get(&first_tok_id), Some("tok-secret"));
        assert_eq!(secret_vals_after.get(&dup_tok_id), None);

        // Also verify that if a variable already exists in both dev and prod before one is marked secret,
        // marking one secret synchronizes all same-named rows and typing into prod via set_value keeps it out of VariableStore.
        state.update(cx, |state, cx| {
            state.create_at(Some(dev_id.clone()), "DB_PASS".into(), "dev-pass".into(), cx);
            state.create_at(Some(prod_id.clone()), "DB_PASS".into(), String::new(), cx);
        });
        cx.run_until_parked();
        let (dev_db_id, prod_db_id) = state.read_with(cx, |state, _| {
            let dev_v = state
                .variables()
                .iter()
                .find(|v| v.name == "DB_PASS" && v.environment.as_ref() == Some(&dev_id))
                .expect("dev DB_PASS")
                .id
                .clone();
            let prod_v = state
                .variables()
                .iter()
                .find(|v| v.name == "DB_PASS" && v.environment.as_ref() == Some(&prod_id))
                .expect("prod DB_PASS")
                .id
                .clone();
            (dev_v, prod_v)
        });
        state.update(cx, |state, cx| state.set_secret(&dev_db_id, true, cx));
        cx.run_until_parked();
        assert!(state.read_with(cx, |state, cx| state.is_secret(&prod_db_id, cx)));

        state.update(cx, |state, cx| {
            state.set_value(&prod_db_id, "prod-super-secret".into(), cx);
            state.flush(cx);
        });
        cx.run_until_parked();
        let stored_after =
            pollster::block_on(storage.variables().list(id.clone(), VariableCategory::Environment)).expect("list");
        assert!(
            stored_after.iter().all(|v| v.value.is_empty()),
            "existing row in prod must not write secret plaintext to VariableStore: {stored_after:?}"
        );
        let secret_vals_final = pollster::block_on(storage.prefs().secret_values(id)).expect("secrets");
        assert_eq!(secret_vals_final.get(&prod_db_id), Some("prod-super-secret"));
    }
}
