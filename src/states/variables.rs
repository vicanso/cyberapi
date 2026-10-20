//! One category of a project's variables, as its dialog edits them (M8):
//! environment variables, custom variables or global request headers.
//!
//! Each row is one stored variable (F8.2): adding makes one at once (blank),
//! typing updates it 300 ms after the last key, turning it on or off and
//! removing it are immediate. The list is the store's order — by name — as
//! it was when the dialog opened; a row does not jump while it is renamed.

use super::{Load, WorkspaceState, run_store};
use cyberapi_core::model::{Variable, VariableCategory};
use cyberapi_core::{CollectionId, VariableId};
use cyberapi_store::{NewVariable, Storage, StoreError, VariableEdit};
use gpui_kit::{Context, EventEmitter, Task, WeakEntity};
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
    variables: Vec<Variable>,
    load: Load,
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
        cx: &mut Context<Self>,
    ) -> Self {
        // Typing still waiting when the dialog goes is saved then.
        cx.on_release(|this, cx| {
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
                        },
                    )
                })
                .collect();
            if calls.is_empty() {
                return;
            }
            let (workspace, environment) = (this.workspace.clone(), this.category == VariableCategory::Environment);
            cx.spawn(async move |cx| {
                for call in calls {
                    if let Err(error) = call.await {
                        tracing::warn!(%error, "a variable could not be saved");
                    }
                }
                if environment {
                    let _ = workspace.update(cx, |workspace, cx| workspace.refresh_environment(cx));
                }
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
        Self {
            storage,
            workspace,
            collection,
            category,
            variables: Vec::new(),
            load: Load::Loading,
            saving: HashMap::new(),
        }
    }

    pub fn category(&self) -> VariableCategory {
        self.category
    }

    pub fn variables(&self) -> &[Variable] {
        &self.variables
    }

    pub fn load(&self) -> &Load {
        &self.load
    }

    /// The workspace keeps the environment for the URI's prefix (F4.4).
    fn changed(&self, cx: &mut Context<Self>) {
        if self.category == VariableCategory::Environment {
            let _ = self
                .workspace
                .update(cx, |workspace, cx| workspace.refresh_environment(cx));
        }
        cx.notify();
    }

    /// A blank, enabled variable at the end.
    pub fn add(&mut self, cx: &mut Context<Self>) {
        let call = self.storage.variables().create(NewVariable {
            collection: self.collection.clone(),
            category: self.category,
            name: String::new(),
            value: String::new(),
            enabled: true,
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
        self.edit(id, |variable| variable.value = value, false, cx);
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
    use gpui_kit::{AppContext as _, TestAppContext};

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
}
