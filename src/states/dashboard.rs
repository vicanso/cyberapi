//! The project list (M2).

use super::{Load, run_store};
use cyberapi_core::CollectionId;
use cyberapi_core::model::Collection;
use cyberapi_store::{CollectionEdit, NewCollection, Storage, StoreError};
use gpui_kit::{Context, EventEmitter};

/// What the view should tell the reader about.
#[derive(Clone, Debug)]
pub enum DashboardEvent {
    /// A change the reader asked for did not happen.
    Failed(StoreError),
}

pub struct DashboardState {
    storage: Storage,
    collections: Vec<Collection>,
    load: Load,
}

impl EventEmitter<DashboardEvent> for DashboardState {}

impl DashboardState {
    pub fn new(storage: Storage, cx: &mut Context<Self>) -> Self {
        let mut state = Self {
            storage,
            collections: Vec::new(),
            load: Load::Loading,
        };
        state.reload(cx);
        state
    }

    /// In store order; the view filters and sorts (F2.5 / F2.6).
    pub fn collections(&self) -> &[Collection] {
        &self.collections
    }

    pub fn load(&self) -> &Load {
        &self.load
    }

    /// F2.1.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load = Load::Loading;
        cx.notify();
        run_store(cx, self.storage.collections().list(), |this, result, cx| {
            match result {
                Ok(collections) => {
                    this.collections = collections;
                    this.load = Load::Loaded;
                }
                Err(error) => this.load = Load::Failed(error),
            }
            cx.notify();
        });
    }

    /// F2.2.
    pub fn create(&mut self, input: NewCollection, cx: &mut Context<Self>) {
        run_store(
            cx,
            self.storage.collections().create(input),
            |this, result, cx| match result {
                Ok(collection) => {
                    this.collections.push(collection);
                    cx.notify();
                }
                Err(error) => cx.emit(DashboardEvent::Failed(error)),
            },
        );
    }

    /// F2.3.
    pub fn update(&mut self, id: CollectionId, edit: CollectionEdit, cx: &mut Context<Self>) {
        run_store(
            cx,
            self.storage.collections().update(id, edit),
            |this, result, cx| match result {
                Ok(updated) => {
                    if let Some(found) = this.collections.iter_mut().find(|found| found.id == updated.id) {
                        *found = updated;
                    }
                    cx.notify();
                }
                Err(error) => cx.emit(DashboardEvent::Failed(error)),
            },
        );
    }

    /// F2.4. The view asks first.
    pub fn delete(&mut self, id: CollectionId, cx: &mut Context<Self>) {
        let deleted = id.clone();
        run_store(
            cx,
            self.storage.collections().delete(id),
            move |this, result, cx| match result {
                Ok(()) => {
                    this.collections.retain(|found| found.id != deleted);
                    cx.notify();
                }
                Err(error) => cx.emit(DashboardEvent::Failed(error)),
            },
        );
    }
}
