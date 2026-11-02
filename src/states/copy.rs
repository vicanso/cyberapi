//! A server project as this reader edits it (`docs/server-sync.md` §3–4,
//! §6): a working copy the workspace reads and writes in place of the
//! server's tree and variables. A change lands in the copy and marks it
//! unsynced; the server hears of it only when the reader syncs. A page
//! sends the changes it kept; the desktop still pushes its copy whole.
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
use cyberapi_core::model::{
    ApiSetting, Collection, Environment, Folder, FolderSettings, HttpRequest, ProjectSettings, Variable,
    VariableCategory,
};
use cyberapi_core::response::{HISTORY_LIMIT, HttpResponse};
use cyberapi_core::sync::{
    ConfigAt, CopyBinding, Moment, OutlineAt, ProjectChanges, ProjectConfig, ProjectOp, ProjectOutline, Step,
    apply_changes, step,
};
use cyberapi_core::tree::{Placement, Removal, TreeChange, TreeData};
use cyberapi_core::undo::{self, Deleted};
use cyberapi_core::{AccountId, CollectionId, EnvironmentId, ItemId, Timestamp, VariableId};
use cyberapi_store::{
    Entity, HistoryStore, Imported, Inserted, MemoryStore, NewVariable, ServerCopyStore, Storage, StoreError,
    StoreFuture, TreeStore, VariableEdit, VariableStore,
};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

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
    /// A page's draft is to be written: once the changes pause, the
    /// project closes or the page is hidden (PERF-13).
    due: bool,
}

/// PERF-13: how long after the last change a page writes the draft. The
/// reader's typing saves every pause; a page writes the changes, not the
/// project whole.
pub const KEEP_DRAFT_AFTER: Duration = Duration::from_secs(1);

thread_local! {
    /// PERF-13: the copies whose draft is due, for [`keep_due_drafts`]. A
    /// page has one thread.
    static DUE: RefCell<Vec<Weak<Shared>>> = const { RefCell::new(Vec::new()) };
}

/// PERF-13: writes every draft that is due, as the page is hidden — the
/// last a page is told before it may be closed.
pub fn keep_due_drafts() {
    let due = DUE.with(|due| std::mem::take(&mut *due.borrow_mut()));
    for shared in due.iter().filter_map(Weak::upgrade) {
        shared.keep_draft();
    }
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
    /// An older draft's whole configuration. A new page draft leaves this
    /// out and keeps [`Self::changes`].
    pub draft: Option<ProjectConfig>,
    /// What a page changed, in order.
    pub changes: ProjectChanges,
    /// The outline those changes sit on.
    pub outline: Option<ProjectOutline>,
}

struct Shared {
    keeper: Keeper,
    collection: CollectionId,
    /// Whose own items these are (`crate::local_items`).
    scope: String,
    marks: Mutex<Marks>,
    /// The reader's own items (server-sync §3.4).
    local: Mutex<LocalItems>,
    /// A page's answers to APIs the server does not know yet, newest first:
    /// kept while the project is open, and given to the history once a
    /// sync takes their API there (#52).
    waiting: Mutex<Vec<HttpResponse>>,
    /// D4: how many of an API's the reader keeps, as the last answer was
    /// kept: what the page keeps of the waiting ones, and what they go to
    /// the history with.
    keep: AtomicUsize,
    /// A write finished.
    news: UnboundedSender<()>,
    /// A page's changes, in the order they were made. The desktop pushes
    /// its database whole and keeps none.
    changes: Mutex<ProjectChanges>,
    /// An older draft is the project whole: a page holding one still
    /// replaces, and does not also record changes.
    legacy: Mutex<bool>,
    /// The outline a page's changes sit on: the server's, as last read,
    /// before those changes. A refresh offline replays the changes on it.
    base_outline: Mutex<Option<ProjectOutline>>,
}

impl Shared {
    fn marks(&self) -> MutexGuard<'_, Marks> {
        self.marks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn local(&self) -> MutexGuard<'_, LocalItems> {
        self.local.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn waiting(&self) -> MutexGuard<'_, Vec<HttpResponse>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn changes(&self) -> MutexGuard<'_, ProjectChanges> {
        self.changes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn legacy(&self) -> MutexGuard<'_, bool> {
        self.legacy.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn base_outline(&self) -> MutexGuard<'_, Option<ProjectOutline>> {
        self.base_outline.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A page records each write, unless it is still holding an older draft
    /// of the whole project.
    fn logs_ops(&self) -> bool {
        matches!(self.keeper, Keeper::Page { .. }) && !*self.legacy()
    }

    fn is_page(&self) -> bool {
        matches!(self.keeper, Keeper::Page { .. })
    }

    /// Records `op`, collapsing it into the previous one when that was the
    /// same whole document (one API's request, the variables, the name).
    fn note(&self, op: ProjectOp) {
        if !self.logs_ops() {
            return;
        }
        let mut changes = self.changes();
        let replace = changes.ops.last().is_some_and(|last| same_document(last, &op));
        if replace {
            *changes.ops.last_mut().expect("the op just found") = op;
        } else {
            changes.ops.push(op);
        }
    }

    /// The variables and environments, both whole, after one of them changed.
    /// The changes lock is not held across the reads.
    async fn note_variables(self: &Arc<Self>) {
        if !self.logs_ops() {
            return;
        }
        let mut variables = Vec::new();
        for category in VariableCategory::ALL {
            match self.variables().list(self.collection.clone(), category).await {
                Ok(found) => variables.extend(found),
                Err(error) => {
                    tracing::warn!(%error, "the variables could not be read");
                    return;
                }
            }
        }
        let environments = match self.variables().environments(self.collection.clone()).await {
            Ok(found) => found,
            Err(error) => {
                tracing::warn!(%error, "the environments could not be read");
                return;
            }
        };
        self.note(ProjectOp::Variables {
            variables,
            environments,
        });
    }

    /// `item` is the reader's own no longer: the subtree, as the copy has
    /// it, becomes changes a sync can send. Skipped when this session
    /// already recorded creating it.
    fn note_unmarked(&self, item: &ItemId) {
        let Keeper::Page { working, .. } = &self.keeper else {
            return;
        };
        let Ok(config) = working.project(&self.collection) else {
            return;
        };
        if !self.local().members(&config.folders).contains(item) {
            return;
        }
        if self.changes().ops.iter().any(|op| op_creates(op, item)) {
            return;
        }
        let mut inside = HashSet::new();
        collect_subtree(&config, item, &mut inside);
        let parent = config
            .folders
            .iter()
            .find(|folder| folder.children.contains(item) && !inside.contains(&folder.id))
            .map(|folder| folder.id.clone());
        let folders: Vec<Folder> = config
            .folders
            .iter()
            .filter(|folder| inside.contains(&folder.id))
            .cloned()
            .collect();
        let apis: Vec<ApiSetting> = config
            .apis
            .iter()
            .filter(|api| inside.contains(&api.id))
            .cloned()
            .collect();
        if folders.is_empty() && apis.is_empty() {
            return;
        }
        self.note(ProjectOp::Import { parent, folders, apis });
    }

    fn keep_local(&self, local: &LocalItems) {
        if !local_items::set(&self.scope, &self.collection, local) {
            tracing::warn!("the reader's own items could not be kept");
        }
    }

    /// PERF-13: writes a page's draft, when one is due.
    fn keep_draft(&self) {
        let Keeper::Page { working, scope } = &self.keeper else {
            return;
        };
        let marks = {
            let mut marks = self.marks();
            if !marks.due {
                return;
            }
            marks.due = false;
            *marks
        };
        let draft = if *self.legacy() {
            match working.project(&self.collection) {
                Ok(config) => Some(Draft::whole(marks.base, config)),
                Err(error) => {
                    tracing::warn!(%error, "the working copy could not be read");
                    None
                }
            }
        } else {
            Some(Draft::edits(
                marks.base,
                self.changes().clone(),
                self.base_outline().clone(),
            ))
        };
        let kept = draft.is_some_and(|draft| drafts_local::set(scope, &self.collection, &draft));
        self.marks().kept = kept;
    }

    /// After a change: the reader's own items as the copy has them now, and
    /// whether what syncs of the copy differs from the server's. `None`
    /// when the reader has none, or that is not known: every change counts
    /// then.
    async fn recount(self: Arc<Self>) -> Option<bool> {
        // A page's unsynced changes are the ones a sync would send. A
        // fingerprint of the working copy would see stubs and call every
        // untouched API changed.
        if self.logs_ops() {
            if !self.local().roots.is_empty()
                && let Ok(at) = self.tree().config(self.collection.clone()).await
            {
                let mut local = self.local();
                local.keep(&at.config);
                self.keep_local(&local);
            }
            let folders = match self.tree().outline(self.collection.clone()).await {
                Ok(at) => at.outline.folders,
                Err(error) => {
                    tracing::warn!(%error, "the working copy could not be read");
                    return None;
                }
            };
            let members = self.local().members(&folders);
            let left = !filter_changes(&self.changes(), &members).is_empty();
            return Some(left);
        }
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

    /// Drops the changes a sync already sent, or that a pull replaced.
    /// What the reader keeps to themselves stays, so making it sync later
    /// still has it.
    fn retain_local_ops(&self, folders: &[Folder]) {
        if !self.is_page() {
            return;
        }
        let members = self.local().members(folders);
        self.changes().ops.retain(|op| op_is_local(op, &members));
        *self.legacy() = false;
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
    /// desktop's binding. A page's draft with changes is only marked due:
    /// it is written by [`Self::keep_draft`] (PERF-13).
    fn persist(self: &Arc<Self>) -> StoreFuture<()> {
        let marks = *self.marks();
        match &self.keeper {
            Keeper::Page { scope, .. } => {
                let mut marks = self.marks();
                if !marks.unsynced {
                    drafts_local::remove(scope, &self.collection);
                    marks.kept = true;
                    marks.due = false;
                } else if !marks.due {
                    marks.due = true;
                    DUE.with(|due| {
                        let mut due = due.borrow_mut();
                        due.retain(|shared| shared.strong_count() > 0 && !Weak::ptr_eq(shared, &Arc::downgrade(self)));
                        due.push(Arc::downgrade(self));
                    });
                }
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
#[derive(Clone)]
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
            waiting: Mutex::default(),
            keep: AtomicUsize::new(HISTORY_LIMIT),
            marks: Mutex::new(Marks {
                base: None,
                unsynced: false,
                pending: 0,
                landed: 0,
                kept: true,
                due: false,
            }),
            news,
            changes: Mutex::new(ProjectChanges::default()),
            legacy: Mutex::new(false),
            base_outline: Mutex::new(None),
        });
        (Self { shared }, heard)
    }

    /// The stores the workspace works with: the copy's tree and variables,
    /// and for the rest the server's (a page) or the machine's own (the
    /// desktop: sending, history and cookies stay there).
    pub fn storage(&self, server: &Storage) -> Storage {
        let copy = Arc::new(CopyStore(self.shared.clone()));
        match &self.shared.keeper {
            Keeper::Page { .. } => server.with_project(
                copy.clone(),
                copy,
                Arc::new(CopyHistory {
                    server: server.clone(),
                    shared: self.shared.clone(),
                }),
            ),
            Keeper::Desktop { local, .. } => local.with_content(copy.clone(), copy),
        }
    }

    /// After a sync: the answers the page kept for APIs the server did not
    /// know go into its history, oldest first, as each API is there now (an
    /// API kept local never is). Answers whether any went (#52, #53).
    pub fn keep_waiting(&self, server: &Storage) -> StoreFuture<bool> {
        let shared = self.shared.clone();
        let server = server.clone();
        Box::pin(async move {
            let waiting: Vec<HttpResponse> = shared.waiting().iter().rev().cloned().collect();
            let mut moved = false;
            let keep = shared.keep.load(Ordering::Relaxed);
            for response in waiting {
                match server.history().add(response.clone(), keep).await {
                    Ok(_) => {
                        shared.waiting().retain(|kept| kept.id != response.id);
                        moved = true;
                    }
                    Err(error) if unknown_api(&error) => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(moved)
        })
    }

    /// What was kept of the copy: a page's draft, the desktop's binding.
    pub fn kept(&self) -> StoreFuture<Option<Kept>> {
        let collection = self.shared.collection.clone();
        match &self.shared.keeper {
            Keeper::Page { scope, .. } => {
                let kept = drafts_local::get(scope, &collection).map(|draft| Kept {
                    base: draft.base,
                    unsynced: true,
                    draft: draft.config,
                    changes: draft.changes,
                    outline: draft.outline,
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
                            changes: ProjectChanges::default(),
                            outline: None,
                        })
                    }))
                })
            }
        }
    }

    /// Goes on from what was kept: its base, and its changes when it has
    /// some. A page's draft becomes the copy; the desktop's copy is in its
    /// database already. `fresh` is the server's outline when it answered:
    /// the changes sit on that. Offline, they sit on the outline the draft
    /// kept.
    ///
    /// [`Resume::Behind`]: a change names an API the outline no longer has.
    /// The outline is what shows, the changes stay, and nothing is written.
    pub fn resume(
        &self,
        collection: Collection,
        kept: Kept,
        fresh: Option<ProjectOutline>,
    ) -> Result<Resume, StoreError> {
        if let Keeper::Page { working, .. } = &self.shared.keeper {
            if let Some(draft) = kept.draft {
                *self.shared.legacy() = true;
                *self.shared.changes() = ProjectChanges::default();
                let draft = self.shared.local().restore(draft);
                working.hold(collection, draft)?;
            } else {
                *self.shared.legacy() = false;
                *self.shared.changes() = kept.changes.clone();
                let outline = fresh.or(kept.outline);
                *self.shared.base_outline() = outline.clone();
                let mut config = match &outline {
                    Some(outline) => outline.config(&collection.id),
                    None => ProjectConfig {
                        name: collection.name.clone(),
                        description: collection.description.clone(),
                        ..Default::default()
                    },
                };
                match apply_changes(&mut config, &kept.changes) {
                    Ok(()) => {
                        let config = self.shared.local().restore(config);
                        working.hold(collection, config)?;
                    }
                    Err(error) => {
                        let error = StoreError::from(error);
                        tracing::info!(%error, "a draft's changes do not sit on the server's outline");
                        let bare = outline
                            .as_ref()
                            .map(|outline| outline.config(&collection.id))
                            .unwrap_or_default();
                        working.hold(collection, self.shared.local().restore(bare))?;
                        let mut marks = self.shared.marks();
                        marks.base = kept.base;
                        marks.unsynced = true;
                        marks.kept = true;
                        return Ok(Resume::Behind);
                    }
                }
            }
        }
        let mut marks = self.shared.marks();
        marks.base = kept.base;
        marks.unsynced = kept.unsynced;
        marks.kept = true;
        Ok(Resume::Held)
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
            // A page's outline has no request documents. Its ids say what
            // the server has; a fingerprint of the stubs would not.
            if shared.logs_ops() {
                if shared.local().server.is_some() {
                    return Ok(());
                }
                let at = shared.tree().outline(shared.collection.clone()).await?;
                let config = at.outline.config(&shared.collection);
                let mut local = shared.local();
                if local.server.is_none() {
                    local.server = Some(ids(&local.syncable(&config)));
                }
                shared.keep_local(&local);
                return Ok(());
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
                if shared.is_page() {
                    // The stubs are not a level to compare later. What the
                    // server has is every id that is not the reader's own.
                    local.server = Some(ids(&syncable));
                } else {
                    local.level_with(&syncable);
                }
                shared.keep_local(&local);
            }
            shared.retain_local_ops(&held.folders);
            shared.persist().await?;
            Ok(held)
        })
    }

    /// The server's outline becomes the copy, level with it at `base`.
    /// A page keeps that outline and the reader's own items, not every
    /// request document.
    pub fn take_outline(
        &self,
        collection: Collection,
        outline: ProjectOutline,
        base: u64,
    ) -> StoreFuture<ProjectConfig> {
        *self.shared.base_outline() = Some(outline.clone());
        let config = outline.config(&collection.id);
        self.take(collection, config, base)
    }

    /// Whether this copy is a page's: its tree is an outline until a tab,
    /// or a sync, asks for a request.
    pub fn is_page(&self) -> bool {
        self.shared.is_page()
    }

    /// The page's working copy as it is now. A desktop copy is read from
    /// its database instead.
    pub fn held_config(&self) -> Result<ProjectConfig, StoreError> {
        match &self.shared.keeper {
            Keeper::Page { working, .. } => working.project(&self.shared.collection),
            Keeper::Desktop { .. } => Err(StoreError::backend("a desktop copy is read from its database")),
        }
    }

    /// Replaces one API the page already has, and does not mark the copy
    /// unsynced: the request was read, not edited.
    pub fn fill_api(&self, api: ApiSetting) -> Result<(), StoreError> {
        match &self.shared.keeper {
            Keeper::Page { working, .. } => working.fill_api(api),
            Keeper::Desktop { .. } => Ok(()),
        }
    }

    /// The request `api`, from the working copy when it already has the
    /// document, otherwise from `server` — which a page then keeps, still
    /// without a change to sync.
    pub fn load_body(&self, server: &Storage, api: ItemId) -> StoreFuture<ApiSetting> {
        let collection = self.shared.collection.clone();
        if !self.is_page() || self.holds_body(&api) {
            return self.shared.tree().api(collection, api);
        }
        let server = server.clone();
        let copy = self.clone();
        Box::pin(async move {
            let setting = server.tree().api(collection, api).await?;
            copy.fill_api(setting.clone())?;
            Ok(setting)
        })
    }

    /// Whether the working copy's request for `api` is the document, not a
    /// stub: the reader edited or created it, it is theirs alone, or the
    /// draft is still the whole project.
    pub fn holds_body(&self, api: &ItemId) -> bool {
        let Keeper::Page { working, .. } = &self.shared.keeper else {
            return true;
        };
        if *self.shared.legacy() {
            return true;
        }
        if self.shared.changes().ops.iter().any(|op| op_carries_body(op, api)) {
            return true;
        }
        let Ok(config) = working.project(&self.shared.collection) else {
            return false;
        };
        self.shared.local().members(&config.folders).contains(api)
    }

    /// APIs whose request in the working copy is the document.
    pub fn body_ids(&self) -> HashSet<ItemId> {
        let mut ids = HashSet::new();
        let Keeper::Page { working, .. } = &self.shared.keeper else {
            return ids;
        };
        let Ok(config) = working.project(&self.shared.collection) else {
            return ids;
        };
        if *self.shared.legacy() {
            ids.extend(config.apis.iter().map(|api| api.id.clone()));
            return ids;
        }
        for op in &self.shared.changes().ops {
            match op {
                ProjectOp::SaveRequest { api, .. } => {
                    ids.insert(api.clone());
                }
                ProjectOp::CreateApi { api, .. } => {
                    ids.insert(api.id.clone());
                }
                ProjectOp::Import { apis, .. } => ids.extend(apis.iter().map(|api| api.id.clone())),
                _ => {}
            }
        }
        let members = self.shared.local().members(&config.folders);
        for api in &config.apis {
            if members.contains(&api.id) {
                ids.insert(api.id.clone());
            }
        }
        ids
    }

    /// Puts `deleted` back into the page's working copy and drops the
    /// delete that took it. The copy's other changes stay. Not the desktop:
    /// its database still has every request, and undoing there replaces.
    pub fn restore_deleted(&self, deleted: &Deleted) -> StoreFuture<ProjectConfig> {
        let Keeper::Page { working, .. } = &self.shared.keeper else {
            return Box::pin(async { Err(StoreError::backend("only a page restores a delete this way")) });
        };
        let working = working.clone();
        let shared = self.shared.clone();
        let deleted = deleted.clone();
        Box::pin(async move {
            let mut config = working.project(&shared.collection)?;
            undo::put_back(&mut config, &deleted);
            TreeStore::replace(&working, shared.collection.clone(), config, None).await?;
            shared
                .changes()
                .ops
                .retain(|op| !matches!(op, ProjectOp::Delete { item } if deleted.holds(item)));
            let counted = shared.clone().recount().await;
            {
                let mut marks = shared.marks();
                marks.landed += 1;
                marks.unsynced = counted.unwrap_or(false);
            }
            shared.persist().await?;
            working.project(&shared.collection)
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
        if !local {
            // Before the mark goes: the subtree is still the reader's, and
            // this must not run while that lock is held.
            self.shared.note_unmarked(item);
        }
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
        local_settled(shared, local)
    }

    /// F6: `items`, the reader's own when they were deleted, theirs again
    /// once an undo has put them back (BUG-3). Marked as this runs, after
    /// that write: a change landing before it would drop the marks again.
    pub fn put_back_local(&self, items: Vec<ItemId>) -> StoreFuture<()> {
        let shared = self.shared.clone();
        Box::pin(async move {
            shared.marks().pending += 1;
            {
                let mut local = shared.local();
                for item in &items {
                    local.mark(item);
                }
                shared.keep_local(&local);
            }
            local_settled(shared, true).await
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

    /// PERF-13: a page's draft written now, when one is due.
    pub fn keep_draft(&self) {
        self.shared.keep_draft();
    }

    /// What to do about the server at `revision` (server-sync §3.1).
    pub fn look(&self, revision: u64) -> Step {
        let base = self.shared.marks().base;
        step(Moment::Look, base, revision, self.unsynced())
    }

    /// What a sync sends. A page sends the changes a merge can apply; the
    /// desktop, and a page still holding an older whole draft, sends the
    /// configuration with the reader's own items left out.
    pub fn snapshot(&self) -> StoreFuture<PushPlan> {
        let (base, landed) = {
            let marks = self.shared.marks();
            (marks.base, marks.landed)
        };
        let shared = self.shared.clone();
        if shared.logs_ops() {
            return Box::pin(async move {
                let at = shared.tree().outline(shared.collection.clone()).await?;
                let members = shared.local().members(&at.outline.folders);
                let changes = filter_changes(&shared.changes(), &members);
                Ok(PushPlan::Merge { changes, base, landed })
            });
        }
        let call = shared.tree().config(shared.collection.clone());
        Box::pin(async move {
            let config = call.await?.config;
            let syncable = shared.local().syncable(&config);
            Ok(PushPlan::Replace {
                config: Box::new(syncable),
                base,
                landed,
            })
        })
    }

    /// `sent`, taken at `landed` writes, is on the server at `revision`.
    /// A change that landed meanwhile stays unsynced. What was sent leaves
    /// the log; what the reader keeps to themselves stays in it.
    pub fn merged(&self, revision: u64, landed: u64, sent: ProjectChanges) -> StoreFuture<()> {
        {
            let mut changes = self.shared.changes();
            drain_sent(&mut changes, &sent);
        }
        let shared = self.shared.clone();
        Box::pin(async move {
            let config = shared.tree().config(shared.collection.clone()).await?.config;
            let left = {
                let mut local = shared.local();
                local.keep(&config);
                let members = local.members(&config.folders);
                let mut server_ids = ids(&config);
                server_ids.retain(|id| !members.contains(id));
                local.server = Some(server_ids);
                shared.keep_local(&local);
                !filter_changes(&shared.changes(), &members).is_empty()
            };
            {
                let mut marks = shared.marks();
                marks.base = Some(revision);
                marks.unsynced = left || marks.landed != landed;
            }
            shared.persist().await
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

/// How [`ServerCopy::resume`] left a page's draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resume {
    /// The changes sit on the outline, or there were none.
    Held,
    /// A change names something the outline no longer has. The outline
    /// shows, and the changes are still kept.
    Behind,
}

/// What a sync sends (server-sync §3.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushPlan {
    /// The desktop, or a page still holding an older whole draft.
    Replace {
        config: Box<ProjectConfig>,
        base: Option<u64>,
        landed: u64,
    },
    /// A page's changes, the reader's own items left out.
    Merge {
        changes: ProjectChanges,
        base: Option<u64>,
        landed: u64,
    },
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

/// After the reader's own items changed (`local`: marked, or made to sync):
/// they are kept as the copy has them, and the copy is unsynced by what
/// syncs of it. The change is counted pending already.
fn local_settled(shared: Arc<Shared>, local: bool) -> StoreFuture<()> {
    Box::pin(async move {
        let read = shared.tree().config(shared.collection.clone()).await;
        if let Ok(at) = &read {
            let mut items = shared.local();
            items.keep(&at.config);
            shared.keep_local(&items);
            if shared.logs_ops() {
                let members = items.members(&at.config.folders);
                drop(items);
                let left = !filter_changes(&shared.changes(), &members).is_empty();
                let mut marks = shared.marks();
                marks.landed += 1;
                marks.unsynced = left;
            } else {
                let unsynced = items.unsynced(&at.config);
                drop(items);
                let mut marks = shared.marks();
                marks.landed += 1;
                // Not known: one made to sync is something to sync.
                marks.unsynced = unsynced.unwrap_or(marks.unsynced || !local);
            }
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

/// Pushes `config` over whatever the server has ("mine over the server's").
pub fn push_over(server: Storage, collection: CollectionId, config: ProjectConfig) -> StoreFuture<Pushed> {
    Box::pin(async move { Ok(Pushed::Written(server.tree().replace(collection, config, None).await?)) })
}

/// The copy as the workspace's tree and variables: every change goes
/// through [`CopyStore::write`].
#[derive(Clone)]
struct CopyStore(Arc<Shared>);

/// What a tree write records, once it has landed. `None` records nothing.
type TreeNote<T> = Box<dyn FnOnce(&T) -> Option<ProjectOp> + Send>;

/// What a write records, once it has landed.
enum Note<T> {
    /// A tree write. `None` records nothing (a whole replace).
    Tree(TreeNote<T>),
    /// Variables or environments changed: record both lists, whole.
    Variables,
    /// Marks the copy unsynced the way a tree write does, and records
    /// nothing. A page's undo does not come through here.
    Silent,
}

impl CopyStore {
    /// A write into the copy: when it lands, the copy is unsynced and kept
    /// so — unless, with the reader's own items in the tree, what syncs of
    /// it is what the server has. A page also records the change.
    fn write<T: Send + 'static>(&self, call: StoreFuture<T>, note: Note<T>) -> StoreFuture<T> {
        let shared = self.0.clone();
        shared.marks().pending += 1;
        // Variables are always something to sync; the op says what.
        let variables = matches!(note, Note::Variables);
        Box::pin(async move {
            let answer = call.await;
            if let Ok(value) = &answer {
                match note {
                    Note::Tree(make) => {
                        if let Some(op) = make(value) {
                            shared.note(op);
                        }
                    }
                    Note::Variables => shared.note_variables().await,
                    Note::Silent => {}
                }
                let counted = if variables {
                    None
                } else {
                    shared.clone().recount().await
                };
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
        let noted = parent.clone();
        self.write(
            self.0.tree().create_folder(collection, parent, name),
            Note::Tree(Box::new(move |inserted: &Inserted<Folder>| {
                Some(ProjectOp::CreateFolder {
                    parent: noted,
                    folder: inserted.item.clone(),
                })
            })),
        )
    }

    fn create_api(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<ApiSetting>> {
        let noted = parent.clone();
        self.write(
            self.0.tree().create_api(collection, parent, name),
            Note::Tree(Box::new(move |inserted: &Inserted<ApiSetting>| {
                Some(ProjectOp::CreateApi {
                    parent: noted,
                    api: inserted.item.clone(),
                })
            })),
        )
    }

    fn rename(&self, collection: CollectionId, item: ItemId, name: String) -> StoreFuture<()> {
        let noted = name.clone();
        self.write(
            self.0.tree().rename(collection, item.clone(), name),
            Note::Tree(Box::new(move |_| Some(ProjectOp::Rename { item, name: noted }))),
        )
    }

    fn save_request(&self, collection: CollectionId, api: ItemId, request: HttpRequest) -> StoreFuture<ApiSetting> {
        self.write(
            self.0.tree().save_request(collection, api, request),
            Note::Tree(Box::new(|setting: &ApiSetting| {
                Some(ProjectOp::SaveRequest {
                    api: setting.id.clone(),
                    request: setting.request.clone(),
                })
            })),
        )
    }

    fn set_folder_settings(
        &self,
        collection: CollectionId,
        folder: ItemId,
        settings: FolderSettings,
    ) -> StoreFuture<Folder> {
        self.write(
            self.0.tree().set_folder_settings(collection, folder, settings),
            Note::Tree(Box::new(|folder: &Folder| {
                Some(ProjectOp::FolderSettings {
                    folder: folder.id.clone(),
                    settings: folder.settings.clone(),
                })
            })),
        )
    }

    fn set_project_settings(&self, collection: CollectionId, settings: ProjectSettings) -> StoreFuture<()> {
        let noted = settings.clone();
        self.write(
            self.0.tree().set_project_settings(collection, settings),
            Note::Tree(Box::new(move |_| Some(ProjectOp::ProjectSettings { settings: noted }))),
        )
    }

    fn move_item(&self, collection: CollectionId, item: ItemId, to: Placement) -> StoreFuture<TreeChange> {
        let noted = to.clone();
        self.write(
            self.0.tree().move_item(collection, item.clone(), to),
            Note::Tree(Box::new(move |_| Some(ProjectOp::Move { item, to: noted }))),
        )
    }

    fn import(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        items: Vec<ImportItem>,
    ) -> StoreFuture<Imported> {
        let noted = parent.clone();
        self.write(
            self.0.tree().import(collection, parent, items),
            Note::Tree(Box::new(move |imported: &Imported| {
                Some(ProjectOp::Import {
                    parent: noted,
                    folders: imported.folders.clone(),
                    apis: imported.apis.clone(),
                })
            })),
        )
    }

    fn delete(&self, collection: CollectionId, item: ItemId) -> StoreFuture<Removal> {
        self.write(
            self.0.tree().delete(collection, item.clone()),
            Note::Tree(Box::new(move |_| Some(ProjectOp::Delete { item }))),
        )
    }

    fn config(&self, collection: CollectionId) -> StoreFuture<ConfigAt> {
        self.0.tree().config(collection)
    }

    fn replace(&self, collection: CollectionId, config: ProjectConfig, base: Option<u64>) -> StoreFuture<u64> {
        self.write(self.0.tree().replace(collection, config, base), Note::Silent)
    }

    fn outline(&self, collection: CollectionId) -> StoreFuture<OutlineAt> {
        self.0.tree().outline(collection)
    }

    fn api(&self, collection: CollectionId, api: ItemId) -> StoreFuture<ApiSetting> {
        self.0.tree().api(collection, api)
    }

    fn merge(&self, collection: CollectionId, changes: ProjectChanges, base: Option<u64>) -> StoreFuture<u64> {
        self.0.tree().merge(collection, changes, base)
    }
}

impl VariableStore for CopyStore {
    fn list(&self, collection: CollectionId, category: VariableCategory) -> StoreFuture<Vec<Variable>> {
        self.0.variables().list(collection, category)
    }

    fn create(&self, input: NewVariable) -> StoreFuture<Variable> {
        self.write(self.0.variables().create(input), Note::Variables)
    }

    fn update(&self, collection: CollectionId, id: VariableId, edit: VariableEdit) -> StoreFuture<Variable> {
        self.write(self.0.variables().update(collection, id, edit), Note::Variables)
    }

    fn delete(&self, collection: CollectionId, ids: Vec<VariableId>) -> StoreFuture<()> {
        self.write(self.0.variables().delete(collection, ids), Note::Variables)
    }

    fn environments(&self, collection: CollectionId) -> StoreFuture<Vec<Environment>> {
        self.0.variables().environments(collection)
    }

    fn create_environment(&self, collection: CollectionId, name: String) -> StoreFuture<Environment> {
        self.write(self.0.variables().create_environment(collection, name), Note::Variables)
    }

    fn rename_environment(
        &self,
        collection: CollectionId,
        id: EnvironmentId,
        name: String,
    ) -> StoreFuture<Environment> {
        self.write(
            self.0.variables().rename_environment(collection, id, name),
            Note::Variables,
        )
    }

    fn delete_environment(&self, collection: CollectionId, id: EnvironmentId) -> StoreFuture<()> {
        self.write(self.0.variables().delete_environment(collection, id), Note::Variables)
    }

    fn reorder_environments(
        &self,
        collection: CollectionId,
        order: Vec<EnvironmentId>,
    ) -> StoreFuture<Vec<Environment>> {
        self.write(
            self.0.variables().reorder_environments(collection, order),
            Note::Variables,
        )
    }
}

/// A page's history: the server's, of which an API made here and not synced
/// yet has none — the server does not know it. Such an API's answers are
/// kept in the page instead, while the project is open (#52), and go to the
/// server's history with the sync that takes the API there
/// ([`ServerCopy::keep_waiting`]).
struct CopyHistory {
    server: Storage,
    shared: Arc<Shared>,
}

/// Changes a sync sends: the reader's own items left out.
pub fn filter_changes(changes: &ProjectChanges, members: &HashSet<ItemId>) -> ProjectChanges {
    ProjectChanges {
        ops: changes.ops.iter().filter_map(|op| filter_op(op, members)).collect(),
    }
}

fn filter_op(op: &ProjectOp, members: &HashSet<ItemId>) -> Option<ProjectOp> {
    match op {
        ProjectOp::CreateFolder { folder, .. } if members.contains(&folder.id) => None,
        ProjectOp::CreateApi { api, .. } if members.contains(&api.id) => None,
        ProjectOp::Rename { item, .. } if members.contains(item) => None,
        ProjectOp::Move { item, .. } if members.contains(item) => None,
        ProjectOp::Delete { item } if members.contains(item) => None,
        ProjectOp::SaveRequest { api, .. } if members.contains(api) => None,
        ProjectOp::FolderSettings { folder, .. } if members.contains(folder) => None,
        ProjectOp::Import { parent, folders, apis } => {
            let folders: Vec<Folder> = folders
                .iter()
                .filter(|folder| !members.contains(&folder.id))
                .cloned()
                .map(|mut folder| {
                    folder.children.retain(|child| !members.contains(child));
                    folder
                })
                .collect();
            let apis: Vec<ApiSetting> = apis.iter().filter(|api| !members.contains(&api.id)).cloned().collect();
            if folders.is_empty() && apis.is_empty() {
                None
            } else {
                Some(ProjectOp::Import {
                    parent: parent.clone(),
                    folders,
                    apis,
                })
            }
        }
        other => Some(other.clone()),
    }
}

fn op_is_local(op: &ProjectOp, members: &HashSet<ItemId>) -> bool {
    match op {
        ProjectOp::CreateFolder { folder, .. } => members.contains(&folder.id),
        ProjectOp::CreateApi { api, .. } => members.contains(&api.id),
        ProjectOp::Rename { item, .. } | ProjectOp::Move { item, .. } | ProjectOp::Delete { item } => {
            members.contains(item)
        }
        ProjectOp::SaveRequest { api, .. } => members.contains(api),
        ProjectOp::FolderSettings { folder, .. } => members.contains(folder),
        ProjectOp::Import { folders, apis, .. } => {
            !folders.is_empty()
                && folders.iter().all(|folder| members.contains(&folder.id))
                && apis.iter().all(|api| members.contains(&api.id))
                || !apis.is_empty()
                    && folders.iter().all(|folder| members.contains(&folder.id))
                    && apis.iter().all(|api| members.contains(&api.id))
        }
        ProjectOp::ProjectSettings { .. } | ProjectOp::Variables { .. } | ProjectOp::Describe { .. } => false,
    }
}

fn op_creates(op: &ProjectOp, item: &ItemId) -> bool {
    match op {
        ProjectOp::CreateFolder { folder, .. } => &folder.id == item,
        ProjectOp::CreateApi { api, .. } => &api.id == item,
        ProjectOp::Import { folders, apis, .. } => {
            folders.iter().any(|folder| &folder.id == item) || apis.iter().any(|api| &api.id == item)
        }
        _ => false,
    }
}

fn op_carries_body(op: &ProjectOp, api: &ItemId) -> bool {
    match op {
        ProjectOp::SaveRequest { api: id, .. } => id == api,
        ProjectOp::CreateApi { api: setting, .. } => &setting.id == api,
        ProjectOp::Import { apis, .. } => apis.iter().any(|setting| &setting.id == api),
        _ => false,
    }
}

fn same_document(last: &ProjectOp, op: &ProjectOp) -> bool {
    match (last, op) {
        (ProjectOp::Variables { .. }, ProjectOp::Variables { .. }) => true,
        (ProjectOp::Describe { .. }, ProjectOp::Describe { .. }) => true,
        (ProjectOp::ProjectSettings { .. }, ProjectOp::ProjectSettings { .. }) => true,
        (ProjectOp::SaveRequest { api: left, .. }, ProjectOp::SaveRequest { api: right, .. }) => left == right,
        (ProjectOp::FolderSettings { folder: left, .. }, ProjectOp::FolderSettings { folder: right, .. }) => {
            left == right
        }
        (ProjectOp::Rename { item: left, .. }, ProjectOp::Rename { item: right, .. }) => left == right,
        _ => false,
    }
}

fn collect_subtree(config: &ProjectConfig, item: &ItemId, into: &mut HashSet<ItemId>) {
    if !into.insert(item.clone()) {
        return;
    }
    if let Some(folder) = config.folders.iter().find(|folder| &folder.id == item) {
        for child in &folder.children {
            collect_subtree(config, child, into);
        }
    }
}

/// Drops each sent op the first time it appears. A later write of the same
/// op, appended while the push was out, stays.
pub fn drain_sent(changes: &mut ProjectChanges, sent: &ProjectChanges) {
    let mut sent_iter = sent.ops.iter();
    let mut next = sent_iter.next();
    changes.ops.retain(|op| {
        if next.is_some_and(|want| want == op) {
            next = sent_iter.next();
            false
        } else {
            true
        }
    });
}

fn unknown_api(error: &StoreError) -> bool {
    matches!(error, StoreError::NotFound { what: Entity::Api, .. })
}

/// An id for an answer the page keeps, different for each.
fn waiting_id() -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    HttpResponse::waiting_id(COUNT.fetch_add(1, Ordering::Relaxed))
}

/// The page's answers of `api` among `waiting`, newest first.
fn waiting_of(waiting: &[HttpResponse], collection: &CollectionId, api: &ItemId) -> Vec<HttpResponse> {
    waiting
        .iter()
        .filter(|response| &response.collection == collection && &response.api == api)
        .cloned()
        .collect()
}

impl HistoryStore for CopyHistory {
    fn add(&self, response: HttpResponse, keep: usize) -> StoreFuture<HttpResponse> {
        let call = self.server.history().add(response.clone(), keep);
        let shared = self.shared.clone();
        shared.keep.store(keep, Ordering::Relaxed);
        Box::pin(async move {
            match call.await {
                Err(error) if unknown_api(&error) => {
                    let kept = HttpResponse {
                        id: waiting_id(),
                        created_at: Timestamp::from_unix(chrono::Utc::now().timestamp()),
                        ..response
                    };
                    let mut waiting = shared.waiting();
                    waiting.insert(0, kept.clone());
                    // As many of each API as the history keeps.
                    let mut count = 0;
                    waiting.retain(|other| {
                        let of_api = other.collection == kept.collection && other.api == kept.api;
                        count += usize::from(of_api);
                        !of_api || count <= keep
                    });
                    Ok(kept)
                }
                answer => answer,
            }
        })
    }

    fn list(&self, collection: CollectionId, api: ItemId) -> StoreFuture<Vec<HttpResponse>> {
        let call = self.server.history().list(collection.clone(), api.clone());
        let shared = self.shared.clone();
        Box::pin(async move {
            let kept = match call.await {
                Err(error) if unknown_api(&error) => Vec::new(),
                answer => answer?,
            };
            let waiting = waiting_of(&shared.waiting(), &collection, &api);
            if waiting.is_empty() {
                return Ok(kept);
            }
            // Synced, and not given to the history yet: both, newest first.
            let mut all: Vec<HttpResponse> = waiting.into_iter().chain(kept).collect();
            all.sort_by_key(|response| std::cmp::Reverse(response.created_at));
            all.truncate(shared.keep.load(Ordering::Relaxed));
            Ok(all)
        })
    }

    /// PERF-9: as [`Self::list`], the server's sent without their bodies.
    fn summaries(&self, collection: CollectionId, api: ItemId) -> StoreFuture<Vec<HttpResponse>> {
        let call = self.server.history().summaries(collection.clone(), api.clone());
        let shared = self.shared.clone();
        Box::pin(async move {
            let kept = match call.await {
                Err(error) if unknown_api(&error) => Vec::new(),
                answer => answer?,
            };
            let waiting = waiting_of(&shared.waiting(), &collection, &api);
            if waiting.is_empty() {
                return Ok(kept);
            }
            let waiting = waiting.into_iter().map(HttpResponse::summary);
            let mut all: Vec<HttpResponse> = waiting.chain(kept).collect();
            all.sort_by_key(|response| std::cmp::Reverse(response.created_at));
            all.truncate(shared.keep.load(Ordering::Relaxed));
            Ok(all)
        })
    }

    /// PERF-9: one the page keeps, or the server's.
    fn get(&self, collection: CollectionId, api: ItemId, id: String) -> StoreFuture<HttpResponse> {
        let waiting = self
            .shared
            .waiting()
            .iter()
            .find(|response| response.id == id && response.collection == collection && response.api == api)
            .cloned();
        if let Some(waiting) = waiting {
            return Box::pin(async move { Ok(waiting) });
        }
        self.server.history().get(collection, api, id)
    }

    /// D4: the server's and the page's, newest first.
    fn recent(&self, collection: CollectionId, limit: usize) -> StoreFuture<Vec<HttpResponse>> {
        let call = self.server.history().recent(collection.clone(), limit);
        let shared = self.shared.clone();
        Box::pin(async move {
            let kept = call.await?;
            let waiting: Vec<HttpResponse> = shared
                .waiting()
                .iter()
                .filter(|response| response.collection == collection)
                .cloned()
                .map(HttpResponse::summary)
                .collect();
            if waiting.is_empty() {
                return Ok(kept);
            }
            let mut all: Vec<HttpResponse> = waiting.into_iter().chain(kept).collect();
            all.sort_by_key(|response| std::cmp::Reverse(response.created_at));
            all.truncate(limit);
            Ok(all)
        })
    }

    fn latest(&self, collection: CollectionId, apis: Vec<ItemId>) -> StoreFuture<Vec<HttpResponse>> {
        let call = self.server.history().latest(collection.clone(), apis.clone());
        let shared = self.shared.clone();
        Box::pin(async move {
            let mut latest = call.await?;
            // An API only the page knows has its newest here (`get`, M6).
            let waiting = shared.waiting();
            for api in apis {
                if latest.iter().any(|response| response.api == api) {
                    continue;
                }
                if let Some(newest) = waiting_of(&waiting, &collection, &api).into_iter().next() {
                    latest.push(newest);
                }
            }
            Ok(latest)
        })
    }

    fn clear(&self, collection: CollectionId, api: ItemId) -> StoreFuture<()> {
        let call = self.server.history().clear(collection.clone(), api.clone());
        let shared = self.shared.clone();
        Box::pin(async move {
            shared
                .waiting()
                .retain(|response| response.collection != collection || response.api != api);
            match call.await {
                Err(error) if unknown_api(&error) => Ok(()),
                answer => answer,
            }
        })
    }

    fn clear_all(&self) -> StoreFuture<()> {
        let call = self.server.history().clear_all();
        let shared = self.shared.clone();
        Box::pin(async move {
            shared.waiting().clear();
            call.await
        })
    }
}
