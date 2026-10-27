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
            this.environments.retain(|environment| environment.id != id);
            this.variables
                .retain(|variable| variable.environment.as_ref() != Some(&id));
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
        let call = self.storage.variables().create(NewVariable {
            collection: self.collection.clone(),
            category: self.category,
            name: String::new(),
            value: String::new(),
            enabled: true,
            environment: self.layer.clone(),
        });
        run_store(cx, call, |this, result, cx| match result {
            Ok(variable) => {
                this.variables.push(variable);
                this.changed(cx);
            }
            Err(error) => cx.emit(VariablesEvent::Failed(error)),
        });
    }

    pub fn set_name(&mut self, id: &VariableId, name: String, cx: &mut Context<Self>) {
        self.edit(id, |variable| variable.name = name, false, cx);
    }

    pub fn set_value(&mut self, id: &VariableId, value: String, cx: &mut Context<Self>) {
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

    /// D3: `id` made secret, its value moved to the reader's own and the
    /// row keeping none; or not, the reader's value the row's again.
    pub fn set_secret(&mut self, id: &VariableId, secret: bool, cx: &mut Context<Self>) {
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
    }

    pub fn delete(&mut self, id: &VariableId, cx: &mut Context<Self>) {
        self.saving.remove(id);
        self.variables.retain(|variable| &variable.id != id);
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
}
