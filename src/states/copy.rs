//! A server project as a page edits it (`docs/server-sync.md` §3–4): a
//! working copy the workspace reads and writes in place of the server's
//! tree and variables. A change lands in the copy, marks it unsynced and is
//! kept as this browser's draft (`crate::drafts_local`); the server hears
//! of it only when the reader syncs, which pushes the copy whole.
//!
//! What a copy does about the server is `cyberapi_core::sync::step`'s to
//! decide. This keeps the copy and where it stands, and pushes.

use crate::drafts_local::{self, Draft};
use cyberapi_core::import::ImportItem;
use cyberapi_core::model::{ApiSetting, Collection, Folder, HttpRequest, Variable, VariableCategory};
use cyberapi_core::response::HttpResponse;
use cyberapi_core::sync::{ConfigAt, Moment, ProjectConfig, Step, step};
use cyberapi_core::tree::{Placement, Removal, TreeChange, TreeData};
use cyberapi_core::{CollectionId, ItemId, VariableId};
use cyberapi_store::{
    Entity, HistoryStore, Imported, Inserted, MemoryStore, NewVariable, Storage, StoreError, StoreFuture, TreeStore,
    VariableEdit, VariableStore,
};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Where a copy stands against the server.
#[derive(Clone, Copy, Debug)]
struct Marks {
    /// The server's revision the copy was last level with.
    base: Option<u64>,
    /// Changes the server has not had.
    unsynced: bool,
    /// Writes on their way into the copy.
    pending: usize,
    /// Writes that landed: one landing while a push is out leaves the copy
    /// unsynced.
    landed: u64,
    /// Whether this browser kept the last draft.
    kept: bool,
}

struct Shared {
    working: MemoryStore,
    collection: CollectionId,
    /// Whose drafts (`crate::pins_local::scope`).
    scope: String,
    marks: Mutex<Marks>,
    /// A write finished.
    news: UnboundedSender<()>,
}

impl Shared {
    fn marks(&self) -> MutexGuard<'_, Marks> {
        self.marks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn started(&self) {
        self.marks().pending += 1;
    }

    /// A write is done: when it changed the copy, the copy is unsynced and
    /// kept as the draft.
    fn finished(&self, changed: bool) {
        {
            let mut marks = self.marks();
            marks.pending = marks.pending.saturating_sub(1);
            if changed {
                marks.landed += 1;
                marks.unsynced = true;
                marks.kept = self.keep(marks.base);
            }
        }
        let _ = self.news.unbounded_send(());
    }

    fn keep(&self, base: Option<u64>) -> bool {
        match self.working.project(&self.collection) {
            Ok(config) => drafts_local::set(&self.scope, &self.collection, &Draft { base, config }),
            Err(error) => {
                tracing::warn!(%error, "the working copy could not be read");
                false
            }
        }
    }
}

/// A server project's working copy, and where it stands.
pub struct ServerCopy {
    shared: Arc<Shared>,
}

impl ServerCopy {
    /// An empty copy of `collection`, for `scope`'s drafts; the receiver
    /// hears each write that finishes.
    pub fn new(collection: &Collection, scope: String) -> (Self, UnboundedReceiver<()>) {
        let (news, heard) = unbounded();
        let shared = Arc::new(Shared {
            working: MemoryStore::new(),
            collection: collection.id.clone(),
            scope,
            marks: Mutex::new(Marks {
                base: None,
                unsynced: false,
                pending: 0,
                landed: 0,
                kept: true,
            }),
            news,
        });
        (Self { shared }, heard)
    }

    /// The server's stores, with the copy's tree and variables in place of
    /// its own. Sending, history, cookies and the reader's state still go
    /// to the server.
    pub fn storage(&self, server: &Storage) -> Storage {
        let copy = Arc::new(CopyStore(self.shared.clone()));
        server.with_project(copy.clone(), copy, Arc::new(CopyHistory(server.clone())))
    }

    /// The draft this browser kept of the project.
    pub fn draft(&self) -> Option<Draft> {
        drafts_local::get(&self.shared.scope, &self.shared.collection)
    }

    /// The copy becomes `config`, level with the server at `base`, or with
    /// changes the server has not had. Level, it keeps no draft.
    pub fn hold(
        &self,
        collection: Collection,
        config: ProjectConfig,
        base: Option<u64>,
        unsynced: bool,
    ) -> Result<(), StoreError> {
        self.shared.working.hold(collection, config)?;
        let mut marks = self.shared.marks();
        marks.base = base;
        marks.unsynced = unsynced;
        marks.kept = true;
        if !unsynced {
            drafts_local::remove(&self.shared.scope, &self.shared.collection);
        }
        Ok(())
    }

    /// Whether the copy has changes the server has not had, landed or on
    /// their way: nothing pulled may replace it then.
    pub fn unsynced(&self) -> bool {
        let marks = self.shared.marks();
        marks.unsynced || marks.pending > 0
    }

    /// No write is on its way: what the copy holds is all the reader did.
    pub fn settled(&self) -> bool {
        self.shared.marks().pending == 0
    }

    /// Whether this browser kept the last change.
    pub fn kept(&self) -> bool {
        self.shared.marks().kept
    }

    /// What to do about the server at `revision` (server-sync §3.1).
    pub fn look(&self, revision: u64) -> Step {
        let base = self.shared.marks().base;
        step(Moment::Look, base, revision, self.unsynced())
    }

    /// The copy as it would go up, its base, and how many writes it holds.
    pub fn snapshot(&self) -> Result<(ProjectConfig, Option<u64>, u64), StoreError> {
        let (base, landed) = {
            let marks = self.shared.marks();
            (marks.base, marks.landed)
        };
        let config = self.shared.working.project(&self.shared.collection)?;
        Ok((config, base, landed))
    }

    /// The snapshot taken at `landed` writes is the server's now, at
    /// `revision`. A change that landed meanwhile stays unsynced, over the
    /// new base. Answers whether the copy is level.
    pub fn pushed(&self, revision: u64, landed: u64) -> bool {
        let mut marks = self.shared.marks();
        marks.base = Some(revision);
        if marks.landed == landed {
            marks.unsynced = false;
            marks.kept = true;
            drafts_local::remove(&self.shared.scope, &self.shared.collection);
            return true;
        }
        marks.kept = self.shared.keep(marks.base);
        false
    }
}

/// How a push went (server-sync §3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pushed {
    /// The server has it, at this revision.
    Written(u64),
    /// The server moved on since the base: nothing was written.
    Conflict,
}

/// Pushes `config` over the server's copy of `collection`, if the server is
/// still at `base`.
pub fn push(
    server: Storage,
    collection: CollectionId,
    config: ProjectConfig,
    base: Option<u64>,
) -> StoreFuture<Pushed> {
    Box::pin(async move {
        let revision = server.collections().get(collection.clone()).await?.revision;
        match step(Moment::Push, base, revision, true) {
            Step::Write => match server.tree().replace(collection, config, base).await {
                Ok(revision) => Ok(Pushed::Written(revision)),
                // Someone else pushed between the two calls.
                Err(StoreError::Stale { .. }) => Ok(Pushed::Conflict),
                Err(error) => Err(error),
            },
            // A push is written or in conflict.
            _ => Ok(Pushed::Conflict),
        }
    })
}

/// Pushes `config` over whatever the server has ("mine over the server's").
pub fn push_over(server: Storage, collection: CollectionId, config: ProjectConfig) -> StoreFuture<Pushed> {
    Box::pin(async move { Ok(Pushed::Written(server.tree().replace(collection, config, None).await?)) })
}

/// The copy as the workspace's tree and variables: every change goes
/// through [`CopyStore::write`].
#[derive(Clone)]
struct CopyStore(Arc<Shared>);

impl CopyStore {
    fn working(&self) -> &MemoryStore {
        &self.0.working
    }

    fn write<T: Send + 'static>(&self, call: StoreFuture<T>) -> StoreFuture<T> {
        let shared = self.0.clone();
        shared.started();
        Box::pin(async move {
            let answer = call.await;
            shared.finished(answer.is_ok());
            answer
        })
    }
}

impl TreeStore for CopyStore {
    fn load(&self, collection: CollectionId) -> StoreFuture<TreeData> {
        self.working().load(collection)
    }

    fn create_folder(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<Folder>> {
        self.write(self.working().create_folder(collection, parent, name))
    }

    fn create_api(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<ApiSetting>> {
        self.write(self.working().create_api(collection, parent, name))
    }

    fn rename(&self, collection: CollectionId, item: ItemId, name: String) -> StoreFuture<()> {
        self.write(TreeStore::rename(self.working(), collection, item, name))
    }

    fn save_request(&self, collection: CollectionId, api: ItemId, request: HttpRequest) -> StoreFuture<ApiSetting> {
        self.write(self.working().save_request(collection, api, request))
    }

    fn move_item(&self, collection: CollectionId, item: ItemId, to: Placement) -> StoreFuture<TreeChange> {
        self.write(self.working().move_item(collection, item, to))
    }

    fn import(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        items: Vec<ImportItem>,
    ) -> StoreFuture<Imported> {
        self.write(self.working().import(collection, parent, items))
    }

    fn delete(&self, collection: CollectionId, item: ItemId) -> StoreFuture<Removal> {
        self.write(TreeStore::delete(self.working(), collection, item))
    }

    fn config(&self, collection: CollectionId) -> StoreFuture<ConfigAt> {
        self.working().config(collection)
    }

    fn replace(&self, collection: CollectionId, config: ProjectConfig, base: Option<u64>) -> StoreFuture<u64> {
        self.write(self.working().replace(collection, config, base))
    }
}

impl VariableStore for CopyStore {
    fn list(&self, collection: CollectionId, category: VariableCategory) -> StoreFuture<Vec<Variable>> {
        VariableStore::list(self.working(), collection, category)
    }

    fn create(&self, input: NewVariable) -> StoreFuture<Variable> {
        self.write(VariableStore::create(self.working(), input))
    }

    fn update(&self, collection: CollectionId, id: VariableId, edit: VariableEdit) -> StoreFuture<Variable> {
        self.write(VariableStore::update(self.working(), collection, id, edit))
    }

    fn delete(&self, collection: CollectionId, ids: Vec<VariableId>) -> StoreFuture<()> {
        self.write(VariableStore::delete(self.working(), collection, ids))
    }
}

/// The server's history, of which an API made here and not synced yet has
/// none: the server does not know it. Its responses still show; they are
/// not kept until it is synced.
struct CopyHistory(Storage);

fn unknown_api(error: &StoreError) -> bool {
    matches!(error, StoreError::NotFound { what: Entity::Api, .. })
}

impl HistoryStore for CopyHistory {
    fn add(&self, response: HttpResponse) -> StoreFuture<HttpResponse> {
        self.0.history().add(response)
    }

    fn list(&self, collection: CollectionId, api: ItemId) -> StoreFuture<Vec<HttpResponse>> {
        let call = self.0.history().list(collection, api);
        Box::pin(async move {
            match call.await {
                Err(error) if unknown_api(&error) => Ok(Vec::new()),
                answer => answer,
            }
        })
    }

    fn latest(&self, collection: CollectionId, apis: Vec<ItemId>) -> StoreFuture<Vec<HttpResponse>> {
        self.0.history().latest(collection, apis)
    }

    fn clear(&self, collection: CollectionId, api: ItemId) -> StoreFuture<()> {
        let call = self.0.history().clear(collection, api);
        Box::pin(async move {
            match call.await {
                Err(error) if unknown_api(&error) => Ok(()),
                answer => answer,
            }
        })
    }

    fn clear_all(&self) -> StoreFuture<()> {
        self.0.history().clear_all()
    }
}
