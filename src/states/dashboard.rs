//! The project list (M2).

use super::{Load, run_store};
use cyberapi_core::CollectionId;
use cyberapi_core::model::Collection;
use cyberapi_store::{CollectionEdit, NewCollection, Storage, StoreError};
use gpui_kit::{Context, EventEmitter};
use std::collections::BTreeMap;
use tracing::warn;

/// What the view should tell the reader about.
#[derive(Clone, Debug)]
pub enum DashboardEvent {
    /// A change the reader asked for did not happen.
    Failed(StoreError),
    /// A project is gone, and with it the reader's state of it.
    Deleted(CollectionId),
    /// A project the reader made, to be opened.
    Created(Collection),
}

pub struct DashboardState {
    storage: Storage,
    collections: Vec<Collection>,
    /// How many APIs each project holds, for its card; read beside the
    /// list, and `None` until it was.
    api_counts: Option<BTreeMap<CollectionId, usize>>,
    load: Load,
}

impl EventEmitter<DashboardEvent> for DashboardState {}

impl DashboardState {
    pub fn new(storage: Storage, cx: &mut Context<Self>) -> Self {
        let mut state = Self {
            storage,
            collections: Vec::new(),
            api_counts: None,
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

    /// How many APIs `id` holds, when that was read.
    pub fn api_count(&self, id: &CollectionId) -> Option<usize> {
        let counts = self.api_counts.as_ref()?;
        Some(counts.get(id).copied().unwrap_or_default())
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
        run_store(cx, self.storage.collections().api_counts(), |this, result, cx| {
            match result {
                Ok(counts) => this.api_counts = Some(counts),
                // The cards go without their counts.
                Err(error) => warn!(%error, "the projects' API counts could not be read"),
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
                    this.collections.push(collection.clone());
                    cx.emit(DashboardEvent::Created(collection));
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
                    cx.emit(DashboardEvent::Deleted(deleted));
                    cx.notify();
                }
                Err(error) => cx.emit(DashboardEvent::Failed(error)),
            },
        );
    }
}
