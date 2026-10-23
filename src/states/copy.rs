//! A server project as this reader edits it (`docs/server-sync.md` §3–4,
//! §6): a working copy the workspace reads and writes in place of the
//! server's tree and variables. A change lands in the copy and marks it
//! unsynced; the server hears of it only when the reader syncs, which
//! pushes the copy whole.
//!
//! Where the copy lives is its keeper's. A page's is in memory and kept as
//! this browser's draft (`crate::drafts_local`); the desktop's is the
//! project in its database, bound to the account (`ServerCopyStore`), so
//! sending, history and cookies there are the machine's own.
//!
//! What a copy does about the server is `cyberapi_core::sync::step`'s to
//! decide. This keeps the copy and where it stands, and pushes.
//!
//! The reader's own items (server-sync §3.4, `cyberapi_core::local_only`)
//! are in the copy too, and kept on this machine (`crate::local_items`): a
//! push leaves them out, a pull puts them back, and while there are some,
//! whether the copy is unsynced is whether what syncs of it changed.

use crate::drafts_local::{self, Draft};
use crate::{local_items, pins_local};
use cyberapi_core::import::ImportItem;
use cyberapi_core::local_only::{LocalItems, fingerprint, ids};
use cyberapi_core::model::{ApiSetting, Collection, Folder, HttpRequest, Variable, VariableCategory};
use cyberapi_core::response::HttpResponse;
use cyberapi_core::sync::{ConfigAt, CopyBinding, Moment, ProjectConfig, Step, step};
use cyberapi_core::tree::{Placement, Removal, TreeChange, TreeData};
use cyberapi_core::{AccountId, CollectionId, ItemId, VariableId};
use cyberapi_store::{
    Entity, HistoryStore, Imported, Inserted, MemoryStore, NewVariable, ServerCopyStore, Storage, StoreError,
    StoreFuture, TreeStore, VariableEdit, VariableStore,
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
    /// Whether the last change was kept (a page's draft can run out of
    /// room).
    kept: bool,
}

/// Where the copy lives.
enum Keeper {
    /// A page's: in memory, kept as this browser's draft of `scope`'s.
    Page { working: MemoryStore, scope: String },
    /// The desktop's: the project in its database, bound to `account`.
    Desktop {
        local: Storage,
        copies: Arc<dyn ServerCopyStore>,
        account: AccountId,
    },
}

/// What was kept of a copy before the server is asked: its base, whether
/// it has unsynced changes, and — a page's draft — those changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub base: Option<u64>,
    pub unsynced: bool,
    pub draft: Option<ProjectConfig>,
}

struct Shared {
    keeper: Keeper,
    collection: CollectionId,
    /// Whose own items these are (`crate::local_items`).
    scope: String,
    marks: Mutex<Marks>,
    /// The reader's own items (server-sync §3.4).
    local: Mutex<LocalItems>,
    /// A write finished.
    news: UnboundedSender<()>,
}

impl Shared {
    fn marks(&self) -> MutexGuard<'_, Marks> {
        self.marks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn local(&self) -> MutexGuard<'_, LocalItems> {
        self.local.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn keep_local(&self, local: &LocalItems) {
        if !local_items::set(&self.scope, &self.collection, local) {
            tracing::warn!("the reader's own items could not be kept");
        }
    }

    /// After a change: the reader's own items as the copy has them now, and
    /// whether what syncs of the copy differs from the server's. `None`
    /// when the reader has none, or that is not known: every change counts
    /// then.
    async fn recount(self: Arc<Self>) -> Option<bool> {
        if self.local().roots.is_empty() {
            return None;
        }
        let config = match self.tree().config(self.collection.clone()).await {
            Ok(at) => at.config,
            Err(error) => {
                tracing::warn!(%error, "the working copy could not be read");
                return None;
            }
        };
        let mut local = self.local();
        local.keep(&config);
        self.keep_local(&local);
        local.unsynced(&config)
    }

    fn tree(&self) -> &dyn TreeStore {
        match &self.keeper {
            Keeper::Page { working, .. } => working,
            Keeper::Desktop { local, .. } => local.tree(),
        }
    }

    fn variables(&self) -> &dyn VariableStore {
        match &self.keeper {
            Keeper::Page { working, .. } => working,
            Keeper::Desktop { local, .. } => local.variables(),
        }
    }

    /// Keeps where the copy stands: a page's draft (none once level), the
    /// desktop's binding.
    fn persist(&self) -> StoreFuture<()> {
        let marks = *self.marks();
        match &self.keeper {
            Keeper::Page { working, scope } => {
                let kept = if !marks.unsynced {
                    drafts_local::remove(scope, &self.collection);
                    true
                } else {
                    match working.project(&self.collection) {
                        Ok(config) => drafts_local::set(
                            scope,
                            &self.collection,
                            &Draft {
                                base: marks.base,
                                config,
                            },
                        ),
                        Err(error) => {
                            tracing::warn!(%error, "the working copy could not be read");
                            false
                        }
                    }
                };
                self.marks().kept = kept;
                Box::pin(async { Ok(()) })
            }
            Keeper::Desktop { copies, account, .. } => copies.bind(CopyBinding {
                collection: self.collection.clone(),
                account: account.clone(),
                base: marks.base,
                unsynced: marks.unsynced,
            }),
        }
    }
}

/// A server project's working copy, and where it stands.
pub struct ServerCopy {
    shared: Arc<Shared>,
}

impl ServerCopy {
    /// A page's copy of `collection`, for `scope`'s drafts; the receiver
    /// hears each write that finishes.
    pub fn page(collection: &Collection, scope: String) -> (Self, UnboundedReceiver<()>) {
        Self::with(
            collection,
            scope.clone(),
            Keeper::Page {
                working: MemoryStore::new(),
                scope,
            },
        )
    }

    /// The desktop's copy of `collection` in `local`, `account`'s.
    pub fn desktop(
        collection: &Collection,
        local: Storage,
        copies: Arc<dyn ServerCopyStore>,
        account: AccountId,
    ) -> (Self, UnboundedReceiver<()>) {
        let scope = pins_local::scope(Some(account.as_str()));
        Self::with(collection, scope, Keeper::Desktop { local, copies, account })
    }

    fn with(collection: &Collection, scope: String, keeper: Keeper) -> (Self, UnboundedReceiver<()>) {
        let (news, heard) = unbounded();
        let local = local_items::get(&scope, &collection.id);
        let shared = Arc::new(Shared {
            keeper,
            collection: collection.id.clone(),
            scope,
            local: Mutex::new(local),
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

    /// The stores the workspace works with: the copy's tree and variables,
    /// and for the rest the server's (a page) or the machine's own (the
    /// desktop: sending, history and cookies stay there).
    pub fn storage(&self, server: &Storage) -> Storage {
        let copy = Arc::new(CopyStore(self.shared.clone()));
        match &self.shared.keeper {
            Keeper::Page { .. } => server.with_project(copy.clone(), copy, Arc::new(CopyHistory(server.clone()))),
            Keeper::Desktop { local, .. } => local.with_content(copy.clone(), copy),
        }
    }

    /// What was kept of the copy: a page's draft, the desktop's binding.
    pub fn kept(&self) -> StoreFuture<Option<Kept>> {
        let collection = self.shared.collection.clone();
        match &self.shared.keeper {
            Keeper::Page { scope, .. } => {
                let kept = drafts_local::get(scope, &collection).map(|draft| Kept {
                    base: draft.base,
                    unsynced: true,
                    draft: Some(draft.config),
                });
                Box::pin(async { Ok(kept) })
            }
            Keeper::Desktop { copies, account, .. } => {
                let (call, account) = (copies.copies(), account.clone());
                Box::pin(async move {
                    Ok(call.await?.into_iter().find_map(|binding| {
                        (binding.collection == collection && binding.account == account).then_some(Kept {
                            base: binding.base,
                            unsynced: binding.unsynced,
                            draft: None,
                        })
                    }))
                })
            }
        }
    }

    /// Goes on from what was kept: its base, and its changes when it has
    /// some. A page's draft becomes the copy; the desktop's copy is in its
    /// database already.
    pub fn resume(&self, collection: Collection, kept: Kept) -> Result<(), StoreError> {
        if let (Keeper::Page { working, .. }, Some(draft)) = (&self.shared.keeper, kept.draft) {
            // A draft made on the project list has none of the reader's own.
            let draft = self.shared.local().restore(draft);
            working.hold(collection, draft)?;
        }
        let mut marks = self.shared.marks();
        marks.base = kept.base;
        marks.unsynced = kept.unsynced;
        marks.kept = true;
        Ok(())
    }

    /// What is not known yet of the reader's own items, from the copy as
    /// it is: what the server has — all of it, until a pull says — and,
    /// when the copy is level, its level.
    pub fn adopt(&self) -> StoreFuture<()> {
        let shared = self.shared.clone();
        Box::pin(async move {
            {
                let local = shared.local();
                if local.server.is_some() && local.level.is_some() {
                    return Ok(());
                }
            }
            let config = shared.tree().config(shared.collection.clone()).await?.config;
            let level = !shared.marks().unsynced;
            let mut local = shared.local();
            let syncable = local.syncable(&config);
            if local.server.is_none() {
                local.server = Some(ids(&syncable));
            }
            if local.level.is_none() && level {
                local.level = Some(fingerprint(&syncable));
            }
            shared.keep_local(&local);
            Ok(())
        })
    }

    /// The server's content becomes the copy, level with it at `base`, the
    /// reader's own items put back: what the copy holds now.
    pub fn take(&self, collection: Collection, config: ProjectConfig, base: u64) -> StoreFuture<ProjectConfig> {
        {
            let mut marks = self.shared.marks();
            marks.base = Some(base);
            marks.unsynced = false;
            marks.kept = true;
        }
        let shared = self.shared.clone();
        let id = collection.id.clone();
        let config = shared.local().restore(config);
        let kept: StoreFuture<()> = match &self.shared.keeper {
            Keeper::Page { working, .. } => {
                let held = working.hold(collection, config);
                Box::pin(async move { held })
            }
            Keeper::Desktop { copies, .. } => copies.keep_copy(collection, config),
        };
        Box::pin(async move {
            kept.await?;
            let held = shared.tree().config(id).await?.config;
            {
                let mut local = shared.local();
                local.keep(&held);
                let syncable = local.syncable(&held);
                local.level_with(&syncable);
                shared.keep_local(&local);
            }
            shared.persist().await?;
            Ok(held)
        })
    }

    /// The reader's own items (server-sync §3.4).
    pub fn local_items(&self) -> LocalItems {
        self.shared.local().clone()
    }

    /// Before `item` goes into `parent` (`folders` as they are): one that
    /// leaves the reader's own folder stays theirs.
    pub fn moving(&self, folders: &[Folder], item: &ItemId, parent: Option<&ItemId>) {
        let mut local = self.shared.local();
        local.moving(folders, item, parent);
        self.shared.keep_local(&local);
    }

    /// Marks `item` the reader's own, or (`false`) makes it sync again. The
    /// copy is unsynced or not by what syncs of it then.
    pub fn set_local(&self, item: &ItemId, local: bool) -> StoreFuture<()> {
        {
            let mut items = self.shared.local();
            if local {
                items.mark(item);
            } else {
                items.unmark(item);
            }
            self.shared.keep_local(&items);
        }
        let shared = self.shared.clone();
        shared.marks().pending += 1;
        Box::pin(async move {
            let read = shared.tree().config(shared.collection.clone()).await;
            if let Ok(at) = &read {
                let mut items = shared.local();
                items.keep(&at.config);
                shared.keep_local(&items);
                let unsynced = items.unsynced(&at.config);
                drop(items);
                let mut marks = shared.marks();
                marks.landed += 1;
                // Not known: one made to sync is something to sync.
                marks.unsynced = unsynced.unwrap_or(marks.unsynced || !local);
            }
            let kept = shared.persist().await;
            {
                let mut marks = shared.marks();
                marks.pending = marks.pending.saturating_sub(1);
            }
            let _ = shared.news.unbounded_send(());
            read?;
            kept
        })
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

    /// Whether the last change was kept.
    pub fn kept_here(&self) -> bool {
        self.shared.marks().kept
    }

    /// What to do about the server at `revision` (server-sync §3.1).
    pub fn look(&self, revision: u64) -> Step {
        let base = self.shared.marks().base;
        step(Moment::Look, base, revision, self.unsynced())
    }

    /// The copy as it would go up — the reader's own items left out —, its
    /// base, and how many writes it holds.
    pub fn snapshot(&self) -> StoreFuture<(ProjectConfig, Option<u64>, u64)> {
        let (base, landed) = {
            let marks = self.shared.marks();
            (marks.base, marks.landed)
        };
        let call = self.shared.tree().config(self.shared.collection.clone());
        let shared = self.shared.clone();
        Box::pin(async move {
            let config = call.await?.config;
            let syncable = shared.local().syncable(&config);
            Ok((syncable, base, landed))
        })
    }

    /// `pushed`, the snapshot taken at `landed` writes, is the server's now,
    /// at `revision`. A change that landed meanwhile stays unsynced, over
    /// the new base.
    pub fn pushed(&self, revision: u64, landed: u64, pushed: ProjectConfig) -> StoreFuture<()> {
        let shared = self.shared.clone();
        Box::pin(async move {
            {
                let mut local = shared.local();
                local.level_with(&pushed);
                shared.keep_local(&local);
            }
            let counted = shared.clone().recount().await;
            {
                let mut marks = shared.marks();
                marks.base = Some(revision);
                marks.unsynced = counted.unwrap_or(marks.landed != landed);
            }
            shared.persist().await
        })
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
    /// A write into the copy: when it lands, the copy is unsynced and kept
    /// so — unless, with the reader's own items in the tree (`tree`), what
    /// syncs of it is what the server has.
    fn write<T: Send + 'static>(&self, call: StoreFuture<T>, tree: bool) -> StoreFuture<T> {
        let shared = self.0.clone();
        shared.marks().pending += 1;
        Box::pin(async move {
            let answer = call.await;
            if answer.is_ok() {
                let counted = if tree { shared.clone().recount().await } else { None };
                {
                    let mut marks = shared.marks();
                    marks.landed += 1;
                    marks.unsynced = counted.unwrap_or(true);
                }
                if let Err(error) = shared.persist().await {
                    tracing::warn!(%error, "an unsynced change could not be marked");
                }
            }
            {
                let mut marks = shared.marks();
                marks.pending = marks.pending.saturating_sub(1);
            }
            let _ = shared.news.unbounded_send(());
            answer
        })
    }
}

impl TreeStore for CopyStore {
    fn load(&self, collection: CollectionId) -> StoreFuture<TreeData> {
        self.0.tree().load(collection)
    }

    fn create_folder(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<Folder>> {
        self.write(self.0.tree().create_folder(collection, parent, name), true)
    }

    fn create_api(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<ApiSetting>> {
        self.write(self.0.tree().create_api(collection, parent, name), true)
    }

    fn rename(&self, collection: CollectionId, item: ItemId, name: String) -> StoreFuture<()> {
        self.write(self.0.tree().rename(collection, item, name), true)
    }

    fn save_request(&self, collection: CollectionId, api: ItemId, request: HttpRequest) -> StoreFuture<ApiSetting> {
        self.write(self.0.tree().save_request(collection, api, request), true)
    }

    fn move_item(&self, collection: CollectionId, item: ItemId, to: Placement) -> StoreFuture<TreeChange> {
        self.write(self.0.tree().move_item(collection, item, to), true)
    }

    fn import(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        items: Vec<ImportItem>,
    ) -> StoreFuture<Imported> {
        self.write(self.0.tree().import(collection, parent, items), true)
    }

    fn delete(&self, collection: CollectionId, item: ItemId) -> StoreFuture<Removal> {
        self.write(self.0.tree().delete(collection, item), true)
    }

    fn config(&self, collection: CollectionId) -> StoreFuture<ConfigAt> {
        self.0.tree().config(collection)
    }

    fn replace(&self, collection: CollectionId, config: ProjectConfig, base: Option<u64>) -> StoreFuture<u64> {
        self.write(self.0.tree().replace(collection, config, base), true)
    }
}

impl VariableStore for CopyStore {
    fn list(&self, collection: CollectionId, category: VariableCategory) -> StoreFuture<Vec<Variable>> {
        self.0.variables().list(collection, category)
    }

    fn create(&self, input: NewVariable) -> StoreFuture<Variable> {
        self.write(self.0.variables().create(input), false)
    }

    fn update(&self, collection: CollectionId, id: VariableId, edit: VariableEdit) -> StoreFuture<Variable> {
        self.write(self.0.variables().update(collection, id, edit), false)
    }

    fn delete(&self, collection: CollectionId, ids: Vec<VariableId>) -> StoreFuture<()> {
        self.write(self.0.variables().delete(collection, ids), false)
    }
}

/// A page's history: the server's, of which an API made here and not synced
/// yet has none — the server does not know it. Its responses still show;
/// they are not kept until it is synced.
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
