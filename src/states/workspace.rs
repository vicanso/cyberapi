//! One project's workspace (M3, M12): its tree, and what the reader keeps of
//! it — open folders, the selected API. Pins are kept on this machine
//! (`crate::pins_local`); the document stored for the project leaves them out.
//!
//! The tree's rules are `cyberapi_core::tree`'s. A change is the store's
//! answer applied to the copy here — the same plan the store wrote — so the
//! copy never guesses. Changes go one at a time ([`run_serial`]) so their
//! answers apply in the order the store made them.
//!
//! In the browser build other readers change the project too: the state
//! polls its revision and loads the tree again when it moved (§7).
//!
//! There the project is the server's, and the reader edits a working copy
//! of it (`docs/server-sync.md` §3–4, `super::copy`): their changes wait in
//! this browser until they sync. Opening, the poll and syncing do what
//! `cyberapi_core::sync::step` says, with the server's content or the
//! draft's.

use super::copy::{self, Kept, Pushed, ServerCopy};
use super::{ApiState, AppStore, GrpcDefinitions, Load, MockState, ServerStatus, run_serial, run_store};
use crate::pacing;
use async_lock::Mutex;
use cyberapi_core::auth::Auth;
use cyberapi_core::code::CodeTarget;
use cyberapi_core::environments;
use cyberapi_core::export::ExportFormat;
use cyberapi_core::graphql_schema::Schema;
use cyberapi_core::import::{ImportBundle, ImportEnvironment, ImportItem};
use cyberapi_core::inherit::{self, Inherited};
use cyberapi_core::model::{
    ApiSetting, Collection, Environment, Folder, FolderSettings, HttpMethod, HttpRequest, ImportSource,
    ProjectSettings, Variable, VariableCategory,
};
use cyberapi_core::prefs::{RequestTab, Scratch, WorkspacePrefs};
use cyberapi_core::response::{HttpResponse, RECENT_LIMIT};
use cyberapi_core::secrets::{self, SecretValues};
use cyberapi_core::sync::{self, ConfigAt, Moment, ProjectConfig, ProjectVersion, Step};
use cyberapi_core::template::Answers;
use cyberapi_core::template_hints;
use cyberapi_core::tree::{Placement, Row, Tree, TreeData};
use cyberapi_core::undo;
use cyberapi_core::{EnvironmentId, ItemId, VariableId};
use cyberapi_service::Sender;
use cyberapi_store::{Storage, StoreError, StoreFuture};
use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Task};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::warn;

/// F1: the most tabs open at once; opening one more closes the one shown
/// longest ago. Each open tab's editor is kept, with its answers.
pub const MAX_TABS: usize = 16;

/// F1: one open tab, a project's API or a scratch request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenTab {
    pub id: ItemId,
    pub method: HttpMethod,
    pub name: String,
    pub scratch: bool,
}

/// F1: a scratch request's id. Not a tree id, so a reload can tell them apart.
fn scratch_id() -> ItemId {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let millis = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    ItemId::new(format!("scratch-{millis}-{n}"))
}

/// What the view should tell the reader about.
#[derive(Clone, Debug)]
pub enum WorkspaceEvent {
    /// A change the reader asked for did not happen.
    Failed(StoreError),
    /// An import was written: how many APIs and folders. Imported again
    /// (B3), also how many APIs the project had were updated, and how many
    /// were left as they were.
    Imported {
        apis: usize,
        folders: usize,
        again: Option<(usize, usize)>,
    },
    /// F3.9 / #69: an API as code, for the clipboard: what it is written
    /// as, and the code.
    Code(CodeTarget, String),
    /// The command could not be made: why.
    CurlFailed(String),
    /// The reader's first change this opening: it waits in this browser
    /// until they sync (server-sync §3.2).
    FirstChange,
    /// This browser could not keep the changes: a refresh loses them (§4).
    NotKept,
    /// Opened over a draft the server has not had; the server moved on too
    /// when `behind` (§3.1).
    KeptUnsynced { behind: bool },
    /// The server's content could not be read: the draft shows (§3.1).
    NotPulled(StoreError),
    /// The server has the reader's changes (§3.3).
    Synced,
    /// The server moved on since the copy's base: the reader chooses.
    Conflict,
    /// The server's content is the copy's now, the reader's changes gone.
    TookServers,
    /// A move would have put the server's items into the reader's own
    /// folder, which a sync leaves out (§3.4): it did not happen.
    NotIntoLocal,
    /// F6: `name` was deleted; delete `count` can be undone a while.
    Deleted { name: String, apis: usize, count: u64 },
    /// F5: `name` was copied, or moved, into `project`; the variables it
    /// reads that `project` has none of.
    Transferred {
        name: String,
        project: String,
        moved: bool,
        missing: Vec<String>,
    },
    /// F7: the open tab `0` is to be shown in a window of its own, or that
    /// window brought forward.
    Parted(ItemId),
}

/// F6: a delete that can be undone, and what of the reader's own state of
/// the tree it took: the pins and the open folders among what went.
struct Undo {
    /// Which delete it is: the view's notice undoes this one only.
    count: u64,
    deleted: undo::Deleted,
    pinned: Vec<ItemId>,
    expanded: Vec<ItemId>,
    /// What of it was the reader's own (server-sync §3.4), to be again.
    local: Vec<ItemId>,
}

/// What the rows were made of (PERF-1).
struct RowsOf {
    tree: u64,
    keyword: String,
    expanded: Vec<ItemId>,
}

/// What opening a copy found: what this machine kept, and the server's
/// project and content — or why not.
type Opened = (Option<Kept>, Result<(Collection, ConfigAt), StoreError>);

/// A server project's working copy, as the workspace keeps it.
struct CopyState {
    copy: ServerCopy,
    /// The server's stores, for what the copy does not answer.
    server: Storage,
    /// The server moved on while the copy has unsynced changes.
    behind: bool,
    /// The reader was told, this opening, that their changes wait here.
    told: bool,
    told_not_kept: bool,
    /// A pull, a push or giving changes up on its way.
    busy: bool,
    /// Why the server could not be reached, the last it was asked: the
    /// network or the bridge failed. Any answer from it clears this.
    unreachable: Option<String>,
    /// How many times in a row it could not be: the poll waits longer
    /// after each (`pacing::retry_after`).
    failures: u32,
    /// A push waiting for the writes on their way: over the server's
    /// whatever it has, or only at the copy's base.
    push: Option<bool>,
    /// PERF-13: writes a page's draft once the changes pause.
    draft: Task<()>,
    _news: Task<()>,
}

pub struct WorkspaceState {
    sender: Sender,
    collection: Collection,
    data: TreeData,
    tree: Tree,
    /// Which `tree` it is: one more each time it is built again.
    tree_built: u64,
    /// PERF-1: the rows as last made, and of what — made again only when
    /// the tree, the keyword or the open folders change, not each frame.
    rows: RefCell<Option<(RowsOf, Rc<[Row]>)>>,
    prefs: WorkspacePrefs,
    load: Load,
    /// What to import once the tree has loaded: a project made from an
    /// import (#65).
    waiting_import: Option<Arc<ImportBundle>>,
    /// F2: the API to select once the tree and the reader's state of it
    /// are read — the state names a selection of its own.
    waiting_select: Option<ItemId>,
    /// The reader's state of the tree has been read, or could not be.
    prefs_read: bool,
    keyword: String,
    /// The tree's changes, one at a time.
    gate: Arc<Mutex<()>>,
    /// The saves of the reader's state, one at a time in the order made:
    /// each writes the whole document.
    saves: Arc<Mutex<()>>,
    /// The opened API's editor, while one is open.
    api: Option<Entity<ApiState>>,
    /// F1: the editors of the open tabs shown this session, the opened one
    /// too, kept while their tab is: a tab shown again has its edits, its
    /// answers and the request on its way as they were.
    kept: HashMap<ItemId, Entity<ApiState>>,
    /// F7: the open tabs shown in a window of their own, whose editor is
    /// that window's alone.
    apart: HashSet<ItemId>,
    /// F1: the open tabs in the order they were shown, the latest last:
    /// past `MAX_TABS`, the one shown longest ago closes.
    shown: Vec<ItemId>,
    /// A pin opened over the tree's selection (F12.2): the editor shows it
    /// while the tree keeps its row. This session's only.
    pin_open: Option<ItemId>,
    /// The API the reader just made, until its editor is up: its URI is
    /// what they type next.
    created: Option<ItemId>,
    /// The project's environment variables, for the URI's prefix (F4.4).
    environment: Vec<Variable>,
    /// The project's global request headers, for the file an `openFile`
    /// in one of them asks for before a send.
    global_headers: Vec<Variable>,
    /// The project's custom variables, for what `{{value(` offers (M6).
    custom: Vec<Variable>,
    /// The project's named environments (#61), in order; the reader's is
    /// `prefs.environment`. The variables above are every layer's, read
    /// through it (`environments::effective`).
    named: Vec<Environment>,
    /// The JSON of APIs' last answers, read for the fields a `get` path
    /// offers (M6): `None` for one with no JSON answer. Kept until the API
    /// answers again.
    answers: HashMap<ItemId, Option<Arc<serde_json::Value>>>,
    /// GraphQL endpoints' schemas, by the URI as the APIs write it: asked
    /// for by one API, offered to every API sent there, until the project
    /// is left.
    schemas: HashMap<String, Arc<Schema>>,
    /// gRPC definitions by the address as the APIs write it, the same way.
    grpc: HashMap<String, Arc<GrpcDefinitions>>,
    /// D4: the project's newest responses, as last read for its recent
    /// list; `None` while they are read.
    recent: Option<Result<Vec<Arc<HttpResponse>>, StoreError>>,
    /// G4: the server project's versions, as last read; `None` while read.
    versions: Option<Result<Vec<ProjectVersion>, StoreError>>,
    /// F6: the last delete, while it can be undone.
    undo: Option<Undo>,
    /// F6: how many deletes the reader was told of: each one's number,
    /// never another's (BUG-9).
    deletes: u64,
    /// A server project's working copy; `None` for a project of this
    /// machine's, whose changes are written at once.
    copy: Option<CopyState>,
    _poll: Option<Task<()>>,
    /// A response filter's save, waiting for the typing to settle.
    _filter_save: Option<Task<()>>,
    /// D3: the reader's values of the project's secret variables, as read
    /// and as typed since.
    secrets: SecretValues,
    /// Their save, waiting for the typing to settle.
    secrets_save: Option<Task<()>>,
    /// G2: the project's mock server, once its dialog opened; it stops
    /// with the project.
    mock: Option<Entity<MockState>>,
    /// A server project's beat, its poll started once the opening heard
    /// from the server or failed to: a first ask after a failed opening
    /// waits as one after any failure.
    poll_every: Option<Duration>,
}

impl EventEmitter<WorkspaceEvent> for WorkspaceState {}

impl WorkspaceState {
    /// F2.7: the tree and the reader's state of it. `sender` holds the
    /// stores and the engine the selected API sends with.
    pub fn new(sender: Sender, collection: Collection, cx: &mut Context<Self>) -> Self {
        Self::with_poll(sender, collection, None, pacing::REVISION_POLL, cx)
    }

    /// A server project, edited in a working copy that waits for the reader
    /// to sync it (server-sync §3–4, §6); `server` is where it is synced to,
    /// `news` hears the copy's writes.
    pub fn server_copy(
        sender: Sender,
        collection: Collection,
        server: Storage,
        copy: (ServerCopy, UnboundedReceiver<()>),
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_poll(sender, collection, Some((server, copy)), pacing::SERVER_POLL, cx)
    }

    /// Asks every `poll` whether another reader changed the project.
    fn with_poll(
        mut sender: Sender,
        collection: Collection,
        copy: Option<(Storage, (ServerCopy, UnboundedReceiver<()>))>,
        poll: Option<Duration>,
        cx: &mut Context<Self>,
    ) -> Self {
        let is_copy = copy.is_some();
        // PERF-13: a draft still waiting is written as the project closes.
        cx.on_release(|this, _| {
            if let Some(copy) = &this.copy {
                copy.copy.keep_draft();
            }
        })
        .detach();
        let copy = copy.map(|(server, (copy, mut news))| {
            sender.storage = copy.storage(&server);
            let news = cx.spawn(async move |this, cx| {
                while news.next().await.is_some() {
                    if this.update(cx, |this, cx| this.copy_written(cx)).is_err() {
                        return;
                    }
                }
            });
            CopyState {
                copy,
                server,
                behind: false,
                told: false,
                told_not_kept: false,
                busy: false,
                unreachable: None,
                failures: 0,
                push: None,
                draft: Task::ready(()),
                _news: news,
            }
        });
        let mut state = Self {
            sender,
            collection,
            data: TreeData::default(),
            tree: Tree::default(),
            tree_built: 0,
            rows: RefCell::new(None),
            prefs: WorkspacePrefs::default(),
            load: Load::Loading,
            waiting_import: None,
            waiting_select: None,
            prefs_read: false,
            keyword: String::new(),
            gate: Arc::default(),
            saves: Arc::default(),
            api: None,
            kept: HashMap::new(),
            apart: HashSet::new(),
            shown: Vec::new(),
            created: None,
            pin_open: None,
            environment: Vec::new(),
            global_headers: Vec::new(),
            custom: Vec::new(),
            named: Vec::new(),
            answers: HashMap::new(),
            schemas: HashMap::new(),
            grpc: HashMap::new(),
            recent: None,
            versions: None,
            undo: None,
            deletes: 0,
            copy,
            _poll: poll.filter(|_| !is_copy).map(|every| Self::poll(every, cx)),
            _filter_save: None,
            secrets: SecretValues::default(),
            secrets_save: None,
            mock: None,
            poll_every: poll,
        };
        state.reload(cx);
        state
    }

    fn poll(every: Duration, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut beats = 0u32;
            loop {
                let Ok(wait) = this.update(cx, |this, _| this.poll_wait(every)) else {
                    return;
                };
                cx.background_executor().timer(wait).await;
                beats = beats.wrapping_add(1);
                if pacing::page_hidden() && !beats.is_multiple_of(pacing::HIDDEN_EVERY) {
                    continue;
                }
                let Ok(call) = this.update(cx, |this, _| {
                    // A copy asks its server; the desktop's own projects
                    // are asked here only in tests.
                    let stores = this.copy.as_ref().map_or(&this.sender.storage, |copy| &copy.server);
                    stores.collections().get(this.collection.id.clone())
                }) else {
                    return;
                };
                let result = cx.background_spawn(call).await;
                if this.update(cx, |this, cx| this.revision_read(result, cx)).is_err() {
                    return;
                }
            }
        })
    }

    /// A moved revision means someone changed the project: its tree loads
    /// again, behind any change of this reader's still on its way. Their own
    /// changes move it too, which costs one load.
    fn revision_read(&mut self, result: Result<Collection, StoreError>, cx: &mut Context<Self>) {
        self.heard_from_server(result.as_ref().err(), cx);
        let collection = match result {
            Ok(collection) => collection,
            // Asked again next beat.
            Err(error) => return warn!(%error, "the project's revision could not be read"),
        };
        if self.copy.is_some() {
            return self.copy_looked(collection, cx);
        }
        if collection.revision == self.collection.revision {
            return;
        }
        self.collection = collection;
        self.refresh_kept_variables(cx);
        let call = self.sender.storage.tree().load(self.collection.id.clone());
        run_serial(cx, &self.gate, call, |this, result, cx| match result {
            Ok(data) => {
                this.set_data(data, cx);
                cx.notify();
            }
            Err(error) => warn!(%error, "the project's tree could not be read again"),
        });
    }

    /// Opening a server project (server-sync §3.1): what this machine kept
    /// of it, and the server's — the project as the reader has it there, and
    /// its content.
    fn open_copy(&mut self, cx: &mut Context<Self>) {
        let Some(copy) = &self.copy else { return };
        let (server, id, kept) = (copy.server.clone(), self.collection.id.clone(), copy.copy.kept());
        // A kept copy opens without the server once it has been waited for
        // long enough — at once, the server known to be out of reach — as
        // when the server could not be reached.
        let patience = pacing::COPY_OPEN_WAIT.map(|wait| {
            let unreachable = cx
                .try_global::<AppStore>()
                .and_then(|store| store.server())
                .is_some_and(|server| matches!(server.read(cx).status(), ServerStatus::Unreachable(_)));
            let wait = if unreachable { Duration::ZERO } else { wait };
            (wait, cx.background_executor().timer(wait))
        });
        let call: StoreFuture<Opened> = Box::pin(async move {
            let kept = kept.await?;
            let on_server = Box::pin(async {
                let found = server.collections().get(id.clone()).await?;
                Ok((found, server.tree().config(id).await?))
            });
            let on_server = match patience {
                Some((wait, timer)) if kept.is_some() => match futures::future::select(on_server, timer).await {
                    futures::future::Either::Left((answer, _)) => answer,
                    futures::future::Either::Right(_) => Err(StoreError::backend(format!(
                        "the server did not answer within {} s",
                        wait.as_secs()
                    ))),
                },
                _ => on_server.await,
            };
            Ok((kept, on_server))
        });
        run_serial(cx, &self.gate, call, |this, result, cx| match result {
            Ok((kept, on_server)) => this.copy_opened(kept, on_server, cx),
            Err(error) => {
                this.load = Load::Failed(error);
                cx.notify();
            }
        });
    }

    fn copy_opened(
        &mut self,
        kept: Option<Kept>,
        on_server: Result<(Collection, ConfigAt), StoreError>,
        cx: &mut Context<Self>,
    ) {
        self.heard_from_server(on_server.as_ref().err(), cx);
        if self._poll.is_none()
            && let Some(every) = self.poll_every
        {
            self._poll = Some(Self::poll(every, cx));
        }
        // The reader's place in it is the server's to say.
        if let Ok((found, _)) = &on_server {
            self.collection.role = found.role;
            self.collection.owner = found.owner.clone();
        }
        // A viewer changes nothing: no changes kept count for them.
        let editable = self.editable();
        let kept = kept.filter(|kept| editable || !kept.unsynced);
        let collection = self.collection.clone();
        let Some(copy) = self.copy.as_mut() else { return };
        let kept = match (on_server, kept) {
            (Ok((_, at)), Some(kept)) => {
                let step = sync::step(Moment::Look, kept.base, at.revision, kept.unsynced);
                if step == Step::Pull {
                    return self.take(at, false, cx);
                }
                if kept.unsynced {
                    copy.behind = step == Step::KeepUnsyncedBehind;
                    copy.told = true;
                    cx.emit(WorkspaceEvent::KeptUnsynced { behind: copy.behind });
                }
                kept
            }
            (Ok((_, at)), None) => return self.take(at, false, cx),
            // Not reachable, or not the reader's any more: what this
            // machine kept is worked on, as long as there is that.
            (Err(error), Some(kept)) => {
                copy.told = kept.unsynced;
                cx.emit(WorkspaceEvent::NotPulled(error));
                kept
            }
            (Err(error), None) => {
                self.load = Load::Failed(error);
                return cx.notify();
            }
        };
        if let Some(draft) = &kept.draft {
            self.collection.name = draft.name.clone();
            self.collection.description = draft.description.clone();
        }
        match copy.copy.resume(collection, kept) {
            Ok(()) => {
                let adopt = copy.copy.adopt();
                run_serial(cx, &self.gate, adopt, |_, result, _| {
                    if let Err(error) = result {
                        warn!(%error, "the reader's own items could not be settled");
                    }
                });
                self.load_all(cx)
            }
            Err(error) => {
                self.load = Load::Failed(error);
                cx.notify();
            }
        }
    }

    /// The poll of a server project: what [`sync::step`] says about the
    /// copy and the server's revision. The copy's name stays its own.
    fn copy_looked(&mut self, collection: Collection, cx: &mut Context<Self>) {
        let (name, description) = (
            std::mem::take(&mut self.collection.name),
            std::mem::take(&mut self.collection.description),
        );
        self.collection = Collection {
            name,
            description,
            ..collection
        };
        let Some(copy) = self.copy.as_mut() else { return };
        let step = copy.copy.look(self.collection.revision);
        let behind = match step {
            Step::Pull => return self.pull(cx),
            Step::KeepUnsyncedBehind => true,
            _ => false,
        };
        if copy.behind != behind {
            copy.behind = behind;
            cx.notify();
        }
    }

    /// The server's content replaces a copy with nothing unsynced. A change
    /// the reader made meanwhile, or one still in the editor, keeps the copy
    /// as it is: it is theirs, and the next look tells them the server moved.
    fn pull(&mut self, cx: &mut Context<Self>) {
        let id = self.collection.id.clone();
        let Some(copy) = self.copy.as_mut().filter(|copy| !copy.busy) else {
            return;
        };
        copy.busy = true;
        let call = copy.server.tree().config(id);
        run_serial(cx, &self.gate, call, |this, result, cx| {
            if let Some(copy) = this.copy.as_mut() {
                copy.busy = false;
            }
            match result {
                Ok(at) if !this.keeps_changes(cx) => this.take(at, false, cx),
                Ok(_) => {}
                Err(error) => {
                    this.heard_from_server(Some(&error), cx);
                    warn!(%error, "the server's project could not be pulled");
                }
            }
        });
    }

    fn keeps_changes(&self, cx: &gpui_kit::App) -> bool {
        self.copy.as_ref().is_some_and(|copy| copy.copy.unsynced())
            || self.kept.values().any(|api| api.read(cx).has_unsaved())
    }

    /// The server's content becomes the copy, level with it.
    fn take(&mut self, at: ConfigAt, forget: bool, cx: &mut Context<Self>) {
        let Some(copy) = self.copy.as_mut() else { return };
        let collection = Collection {
            name: at.config.name.clone(),
            description: at.config.description.clone(),
            ..self.collection.clone()
        };
        let call = copy.copy.take(collection, at.config.clone(), at.revision);
        copy.behind = false;
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            // With the reader's own items back in it.
            Ok(config) => this.took(
                ConfigAt {
                    revision: at.revision,
                    config,
                },
                forget,
                cx,
            ),
            Err(error) => {
                if this.load != Load::Loaded {
                    this.load = Load::Failed(error.clone());
                }
                cx.emit(WorkspaceEvent::Failed(error));
                cx.notify();
            }
        });
    }

    /// The copy is the server's content now. An open tab's editor starts
    /// again from it when its request changed — or always, when the reader
    /// gave their changes up (`forget`).
    fn took(&mut self, at: ConfigAt, forget: bool, cx: &mut Context<Self>) {
        let data = at.config.tree();
        self.collection.name = at.config.name;
        self.collection.description = at.config.description;
        let request = |data: &TreeData, id: &ItemId| {
            data.apis
                .iter()
                .find(|api| &api.id == id)
                .map(|api| api.request.clone())
        };
        let stale: Vec<ItemId> = self
            .kept
            .keys()
            .filter(|id| forget || request(&self.data, id) != request(&data, id))
            .cloned()
            .collect();
        for id in &stale {
            self.start_again(id, cx);
        }
        if self.load == Load::Loaded {
            self.set_data(data, cx);
            self.refresh_kept_variables(cx);
        } else {
            self.load_all(cx);
        }
        if forget {
            cx.emit(WorkspaceEvent::TookServers);
        }
        cx.notify();
    }

    /// A write into the copy finished.
    fn copy_written(&mut self, cx: &mut Context<Self>) {
        let Some(copy) = self.copy.as_mut() else { return };
        if copy.copy.unsynced() && !copy.told {
            copy.told = true;
            cx.emit(WorkspaceEvent::FirstChange);
        }
        self.tell_not_kept(cx);
        // PERF-13: a page's draft is written once the changes pause.
        if let Some(copy) = self.copy.as_mut() {
            copy.draft = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(copy::KEEP_DRAFT_AFTER).await;
                let _ = this.update(cx, |this, cx| {
                    if let Some(copy) = &this.copy {
                        copy.copy.keep_draft();
                    }
                    this.tell_not_kept(cx);
                });
            });
        }
        self.push_when_settled(cx);
        cx.notify();
    }

    /// Tells, once an opening, that a change could not be kept here.
    fn tell_not_kept(&mut self, cx: &mut Context<Self>) {
        let Some(copy) = self.copy.as_mut() else { return };
        if !copy.copy.kept_here() && !copy.told_not_kept {
            copy.told_not_kept = true;
            cx.emit(WorkspaceEvent::NotKept);
        }
    }

    /// Whether the reader syncs this project: a server project they may
    /// change (server-sync §3.2). A viewer has nothing to sync.
    pub fn syncs(&self) -> bool {
        self.copy.is_some() && self.editable()
    }

    /// Changes the server has not had.
    pub fn unsynced(&self) -> bool {
        self.syncs() && self.copy.as_ref().is_some_and(|copy| copy.copy.unsynced())
    }

    /// Unsynced changes, and the server moved on too.
    pub fn server_moved(&self) -> bool {
        self.unsynced() && self.copy.as_ref().is_some_and(|copy| copy.behind)
    }

    /// The reader's own items and everything in them (server-sync §3.4):
    /// none outside a server project.
    pub fn local_members(&self) -> HashSet<ItemId> {
        self.copy
            .as_ref()
            .map(|copy| copy.copy.local_items().members(&self.data.folders))
            .unwrap_or_default()
    }

    /// Whether `item` may become the reader's own: made here, never synced.
    pub fn can_make_local(&self, item: &ItemId) -> bool {
        self.syncs()
            && self
                .copy
                .as_ref()
                .is_some_and(|copy| copy.copy.local_items().can_mark(&self.data.folders, item))
    }

    /// Whether `item` is the reader's own and not inside another of theirs:
    /// what may be made to sync again.
    pub fn is_local_root(&self, item: &ItemId) -> bool {
        self.syncs()
            && self
                .copy
                .as_ref()
                .is_some_and(|copy| copy.copy.local_items().is_outer(&self.data.folders, item))
    }

    /// Marks `item` the reader's own, never synced, or (`false`) makes it
    /// sync again.
    pub fn set_local(&mut self, item: &ItemId, local: bool, cx: &mut Context<Self>) {
        let Some(copy) = &self.copy else { return };
        let allowed = if local {
            self.can_make_local(item)
        } else {
            self.is_local_root(item)
        };
        if !allowed {
            return;
        }
        let call = copy.copy.set_local(item, local);
        run_serial(cx, &self.gate, call, |_, result, cx| match result {
            Ok(()) => cx.notify(),
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
        cx.notify();
    }

    /// A push, a pull or giving changes up is on its way.
    pub fn syncing(&self) -> bool {
        self.copy.as_ref().is_some_and(|copy| copy.busy)
    }

    /// Pushes the copy whole (server-sync §3.3): only where the server is
    /// still at the copy's base, or `over` whatever it has ("mine over the
    /// server's"). What the editor holds goes in first.
    pub fn sync(&mut self, over: bool, cx: &mut Context<Self>) {
        if !self.syncs() {
            return;
        }
        let Some(copy) = self.copy.as_mut().filter(|copy| !copy.busy) else {
            return;
        };
        copy.busy = true;
        copy.push = Some(over);
        for api in self.kept.values() {
            api.update(cx, |api, cx| api.save_now(cx));
        }
        self.push_when_settled(cx);
        cx.notify();
    }

    /// The waiting push goes once no write is on its way: then the copy
    /// holds all the reader did. Each write that finishes asks again.
    fn push_when_settled(&mut self, cx: &mut Context<Self>) {
        let id = self.collection.id.clone();
        let Some(copy) = self.copy.as_mut().filter(|copy| copy.copy.settled()) else {
            return;
        };
        let Some(over) = copy.push.take() else { return };
        let (server, snapshot) = (copy.server.clone(), copy.copy.snapshot());
        let call: StoreFuture<(Pushed, u64, ProjectConfig)> = Box::pin(async move {
            let (config, base, landed) = snapshot.await?;
            let pushed = if over {
                copy::push_over(server, id, config.clone()).await?
            } else {
                copy::push(server, id, config.clone(), base).await?
            };
            Ok((pushed, landed, config))
        });
        run_store(cx, call, |this, result, cx| this.pushed(result, cx));
    }

    fn pushed(&mut self, result: Result<(Pushed, u64, ProjectConfig), StoreError>, cx: &mut Context<Self>) {
        self.heard_from_server(result.as_ref().err(), cx);
        let Some(copy) = self.copy.as_mut() else { return };
        copy.busy = false;
        match result {
            Ok((Pushed::Written(revision), landed, config)) => {
                let keep = copy.copy.pushed(revision, landed, config);
                cx.background_spawn(async move {
                    if let Err(error) = keep.await {
                        warn!(%error, "the synced copy could not be marked");
                    }
                })
                .detach();
                copy.behind = false;
                // What the page kept of APIs now on the server goes into
                // its history, and the open tabs read their history again.
                let keep = copy.copy.keep_waiting(&copy.server);
                run_store(cx, keep, |this, result, cx| match result {
                    Ok(true) => {
                        for api in this.kept.values() {
                            api.update(cx, |api, cx| api.reload_history(cx));
                        }
                    }
                    Ok(false) => {}
                    Err(error) => warn!(%error, "the page's answers could not go into the history"),
                });
                cx.emit(WorkspaceEvent::Synced);
            }
            Ok((Pushed::Conflict, ..)) => cx.emit(WorkspaceEvent::Conflict),
            // Not the reader's to change any more, or not reachable: the
            // changes stay, unsynced.
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        }
        cx.notify();
    }

    /// The server's content in place of the reader's unsynced changes
    /// (server-sync §3.3: "use the server's", "discard my changes"). When it
    /// cannot be read, the copy stays as it is.
    pub fn take_server(&mut self, cx: &mut Context<Self>) {
        let id = self.collection.id.clone();
        let Some(copy) = self.copy.as_mut().filter(|copy| !copy.busy) else {
            return;
        };
        copy.busy = true;
        let call = copy.server.tree().config(id);
        run_serial(cx, &self.gate, call, |this, result, cx| {
            if let Some(copy) = this.copy.as_mut() {
                copy.busy = false;
            }
            this.heard_from_server(result.as_ref().err(), cx);
            match result {
                Ok(at) => this.take(at, true, cx),
                Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
            }
            cx.notify();
        });
        cx.notify();
    }

    /// What a call to a server project's server says of reaching it: a
    /// failure of the backend — the network, the bridge — is the server out
    /// of reach; any answer, a refusal too, is it there.
    fn heard_from_server(&mut self, error: Option<&StoreError>, cx: &mut Context<Self>) {
        let Some(copy) = self.copy.as_mut() else { return };
        let unreachable = match error {
            Some(StoreError::Backend { message }) => Some(message.clone()),
            _ => None,
        };
        // The desktop's server is told too: its projects' cards say it.
        if let Some(server) = cx.try_global::<AppStore>().and_then(|store| store.server().cloned()) {
            let reason = unreachable.clone();
            server.update(cx, |server, cx| server.heard(reason.as_deref(), cx));
        }
        copy.failures = match unreachable {
            Some(_) => copy.failures.saturating_add(1),
            None => 0,
        };
        if copy.unreachable != unreachable {
            copy.unreachable = unreachable;
            cx.notify();
        }
    }

    /// How long the poll waits before it asks: a beat, or — the server out
    /// of reach — longer after each failure, up to five minutes.
    fn poll_wait(&self, every: Duration) -> Duration {
        match self.copy.as_ref().map_or(0, |copy| copy.failures) {
            0 => every,
            failures => pacing::retry_after(failures),
        }
    }

    /// Why a server project's server could not be reached, the last it
    /// was asked; `None` while it answers, and for a project of this
    /// machine's.
    pub fn unreachable(&self) -> Option<&str> {
        self.copy.as_ref()?.unreachable.as_deref()
    }

    pub fn collection(&self) -> &Collection {
        &self.collection
    }

    /// Whether the reader may change the tree, the requests and the
    /// variables (§9.2): a viewer only reads and sends.
    pub fn editable(&self) -> bool {
        self.collection.role.can_edit()
    }

    pub fn storage(&self) -> &Storage {
        &self.sender.storage
    }

    /// The selected API's editor.
    pub fn api(&self) -> Option<&Entity<ApiState>> {
        self.api.as_ref()
    }

    /// G4: whether the project's server keeps its versions: a server
    /// project, with the server to ask.
    pub fn keeps_versions(&self, cx: &App) -> bool {
        self.copy.is_some()
            && cx
                .try_global::<AppStore>()
                .is_some_and(|store| store.versions(cx).is_some())
    }

    /// G4: reads the project's versions again.
    pub fn load_versions(&mut self, cx: &mut Context<Self>) {
        let Some(store) = cx.try_global::<AppStore>().and_then(|store| store.versions(cx)) else {
            return;
        };
        self.versions = None;
        run_store(cx, store.versions(self.collection.id.clone()), |this, result, cx| {
            this.versions = Some(result);
            cx.notify();
        });
        cx.notify();
    }

    /// G4: the project's versions, newest first, as last read; `None`
    /// while they are read.
    pub fn versions(&self) -> Option<&Result<Vec<ProjectVersion>, StoreError>> {
        self.versions.as_ref()
    }

    /// G4: version `id` put back on the server — which keeps the project
    /// as it is now as a version too — and the copy then takes it. Not
    /// over changes the server has not had, nor by a viewer.
    pub fn restore_version(&mut self, id: String, cx: &mut Context<Self>) {
        if self.unsynced() || !self.editable() {
            return;
        }
        let store = cx.try_global::<AppStore>().and_then(|store| store.versions(cx));
        let (Some(store), Some(copy)) = (store, self.copy.as_ref()) else {
            return;
        };
        let server = copy.server.clone();
        let collection = self.collection.id.clone();
        let call: StoreFuture<u64> = Box::pin(async move {
            let config = store.version(collection.clone(), id).await?;
            let at = server.tree().config(collection.clone()).await?;
            server.tree().replace(collection, config, Some(at.revision)).await
        });
        run_serial(cx, &self.gate, call, |this, result, cx| match result {
            Ok(_) => {
                this.take_server(cx);
                this.load_versions(cx);
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// G8: version `id` and what it is compared with: version `after`, or
    /// the project as the server has it now — changes not synced yet are
    /// not in it. `None` when the project keeps no versions.
    pub fn compared_versions(
        &self,
        id: String,
        after: Option<String>,
        cx: &App,
    ) -> Option<StoreFuture<(ProjectConfig, ProjectConfig)>> {
        let store = cx.try_global::<AppStore>().and_then(|store| store.versions(cx))?;
        let server = self.copy.as_ref()?.server.clone();
        let collection = self.collection.id.clone();
        Some(Box::pin(async move {
            let old = store.version(collection.clone(), id).await?;
            let new = match after {
                Some(after) => store.version(collection, after).await?,
                None => server.tree().config(collection).await?.config,
            };
            Ok((old, new))
        }))
    }

    /// D4: reads the project's newest responses again, for its recent list.
    pub fn load_recent(&mut self, cx: &mut Context<Self>) {
        self.recent = None;
        let call = self
            .sender
            .storage
            .history()
            .recent(self.collection.id.clone(), RECENT_LIMIT);
        run_store(cx, call, |this, result, cx| {
            this.recent = Some(result.map(|responses| responses.into_iter().map(Arc::new).collect()));
            cx.notify();
        });
        cx.notify();
    }

    /// D4: the project's newest responses, newest first, as last read;
    /// `None` while they are read.
    pub fn recent(&self) -> Option<&Result<Vec<Arc<HttpResponse>>, StoreError>> {
        self.recent.as_ref()
    }

    /// D4: a response of the recent list: its API selected, the response
    /// shown and its request put back, as choosing it in the API's history
    /// does (F7.10). One of an API that is gone does nothing.
    pub fn open_response(&mut self, api: &ItemId, response: &str, cx: &mut Context<Self>) {
        if !self.data.apis.iter().any(|found| &found.id == api) {
            return;
        }
        self.select(api, cx);
        if let Some(state) = &self.api {
            state.update(cx, |state, cx| state.show_response(response.to_string(), cx));
        }
    }

    /// An API's name, for a list of responses.
    pub fn api_name(&self, api: &ItemId) -> Option<&str> {
        self.data
            .apis
            .iter()
            .find(|found| &found.id == api)
            .map(|found| found.name.as_str())
    }

    /// F4.4: the enabled environment variables, by name, as the reader's
    /// environment reads them (#61).
    pub fn environments(&self) -> Vec<Variable> {
        self.in_environment(&self.environment)
            .into_iter()
            .filter(|variable| variable.enabled)
            .map(|variable| self.shown_secret(variable))
            .collect()
    }

    /// D3: a secret variable as shown: masked where the reader has a value.
    fn shown_secret(&self, variable: Variable) -> Variable {
        match self.is_secret(&variable.id) {
            true => Variable {
                value: match self.secrets.get(&variable.id) {
                    Some(value) if !value.is_empty() => secrets::MASK.to_string(),
                    _ => String::new(),
                },
                ..variable
            },
            false => variable,
        }
    }

    /// The global request headers, enabled or not, as the reader's
    /// environment reads them (#61).
    pub fn global_headers(&self) -> Vec<Variable> {
        self.in_environment(&self.global_headers)
    }

    /// `variables` as a send in the reader's environment reads them.
    fn in_environment(&self, variables: &[Variable]) -> Vec<Variable> {
        environments::effective(variables, self.current_environment().map(|found| &found.id))
    }

    /// #61: the project's named environments, in order.
    pub fn named_environments(&self) -> &[Environment] {
        &self.named
    }

    /// #61: the named environment the reader sends in, while it is one of
    /// the project's.
    pub fn current_environment(&self) -> Option<&Environment> {
        let id = self.prefs.environment.as_ref()?;
        self.named.iter().find(|environment| &environment.id == id)
    }

    /// D6: whether the reader's environment is marked as production.
    pub fn in_production(&self) -> bool {
        self.data
            .settings
            .protects(self.current_environment().map(|environment| &environment.id))
    }

    /// D6: whether `environment` is marked as production.
    pub fn protects(&self, environment: &EnvironmentId) -> bool {
        self.data.settings.protects(Some(environment))
    }

    /// D6: `environment` marked as production, or not; an editor's, as the
    /// rest of the project's settings are.
    pub fn set_production(&mut self, environment: EnvironmentId, on: bool, cx: &mut Context<Self>) {
        if self.protects(&environment) == on {
            return;
        }
        let mut settings = self.data.settings.clone();
        settings.protected_environments.retain(|kept| kept != &environment);
        if on {
            settings.protected_environments.push(environment);
        }
        self.set_project_settings(settings, cx);
    }

    /// D6: whether sending `request` now is asked about first: the reader's
    /// environment is production, the request may change something there,
    /// and the reader has not said to stop asking in this project.
    pub fn asks_before(&self, request: &HttpRequest, cx: &gpui_kit::App) -> bool {
        self.in_production()
            && cyberapi_core::environments::asks_first(request)
            && !cx
                .try_global::<Unasked>()
                .is_some_and(|unasked| unasked.0.contains(&self.collection.id))
    }

    /// D6: no more asking before sends in production in this project, until
    /// the app quits.
    pub fn stop_asking(&self, cx: &mut gpui_kit::App) {
        cx.default_global::<Unasked>().0.insert(self.collection.id.clone());
    }

    /// #61: the reader's environment, their own (kept with their state of
    /// the project, not the project's).
    pub fn set_environment(&mut self, environment: Option<EnvironmentId>, cx: &mut Context<Self>) {
        if self.prefs.environment != environment {
            self.prefs.environment = environment;
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// #61: reads the project's environments again: they changed.
    pub fn refresh_environments(&mut self, cx: &mut Context<Self>) {
        let call = self.sender.storage.variables().environments(self.collection.id.clone());
        run_store(cx, call, |this, result, cx| match result {
            Ok(named) => {
                this.named = named;
                cx.notify();
            }
            Err(error) => warn!(%error, "environments could not be read"),
        });
    }

    /// #66: what an API or a folder takes from the folders above it and the
    /// project.
    pub fn inherited(&self, item: &ItemId) -> Inherited {
        inherit::inherited(&self.data, item)
    }

    /// #66: a folder, with what it sets.
    pub fn folder(&self, id: &ItemId) -> Option<&Folder> {
        self.data.folders.iter().find(|folder| &folder.id == id)
    }

    /// #66: what the project sets for its APIs.
    pub fn project_settings(&self) -> &ProjectSettings {
        &self.data.settings
    }

    /// M6: what a template's `{{` offers: the enabled custom variables and
    /// the APIs, the ones near the opened API first. `machine`: whether
    /// this build reads this machine's files and environment variables.
    pub fn hint_sources(&self, machine: bool) -> template_hints::Sources {
        let mut custom: Vec<(String, String)> = self
            .in_environment(&self.custom)
            .into_iter()
            .filter(|variable| variable.enabled)
            .map(|variable| self.shown_secret(variable))
            .map(|variable| (variable.name, variable.value))
            .collect();
        // D5: the opened API's folders' variables and its own, in place of
        // the project's of the same names.
        if let Some(api) = self.opened() {
            let own = self
                .data
                .apis
                .iter()
                .find(|found| &found.id == api)
                .map(|found| found.request.variables.clone())
                .unwrap_or_default();
            for (name, value) in self.inherited(api).values(&own, &[]) {
                custom.retain(|(given, _)| *given != name);
                custom.push((name, value));
            }
        }
        template_hints::Sources {
            custom,
            apis: template_hints::apis(&self.tree, self.opened()),
            machine,
            response: None,
        }
    }

    /// M6: the JSON of `api`'s last answer, which a `get` path's fields
    /// come from: as read before, or read now. One that cannot be read is
    /// not kept, so the next asks again.
    pub fn answer(&mut self, api: ItemId, cx: &mut Context<Self>) -> Task<Option<Arc<serde_json::Value>>> {
        if let Some(known) = self.answers.get(&api) {
            return Task::ready(known.clone());
        }
        let call = self
            .sender
            .storage
            .history()
            .latest(self.collection.id.clone(), vec![api.clone()]);
        cx.spawn(async move |this, cx| {
            let found = match cx.background_spawn(call).await {
                Ok(responses) => responses
                    .into_iter()
                    .find(|response| response.api == api)
                    .and_then(|response| response.body_json())
                    .map(Arc::new),
                Err(error) => {
                    warn!(%error, "an API's last answer could not be read");
                    return None;
                }
            };
            let _ = this.update(cx, |this, _| this.answers.insert(api, found.clone()));
            found
        })
    }

    /// `api`'s last answer as read before, if it was: `Some(None)` for one
    /// with no JSON answer.
    pub fn known_answer(&self, api: &ItemId) -> Option<Option<Arc<serde_json::Value>>> {
        self.answers.get(api).cloned()
    }

    /// `api` answered: the answer its fields were read from is not the last.
    pub fn forget_answer(&mut self, api: &ItemId) {
        self.answers.remove(api);
    }

    /// A3: what a run sends with: the stores and engine of this project,
    /// the settings now, and the reader's environment. Each API's own
    /// inherited part is the run's to fill.
    pub fn run_sender(&self, cx: &App) -> Sender {
        let mut sender = self.sender.clone();
        if let Some(store) = cx.try_global::<AppStore>() {
            let prefs = store.state().read(cx).prefs();
            sender.timeouts = prefs.timeouts;
            sender.sending = prefs.sending.clone();
        }
        sender.environment = self.current_environment().cloned();
        sender.secrets = self.secrets();
        sender
    }

    /// A3: a run sent `apis`: their last answers are new, and those open
    /// read their history again.
    pub fn ran(&mut self, apis: &[ItemId], cx: &mut Context<Self>) {
        for api in apis {
            self.answers.remove(api);
            if let Some(state) = self.kept.get(api) {
                state.update(cx, |state, cx| state.reload_history(cx));
            }
        }
    }

    /// The schema of the GraphQL endpoint at `uri`, if an API asked for it.
    pub fn schema(&self, uri: &str) -> Option<Arc<Schema>> {
        self.schemas.get(uri.trim()).cloned()
    }

    pub fn keep_schema(&mut self, uri: &str, schema: Arc<Schema>, cx: &mut Context<Self>) {
        self.schemas.insert(uri.trim().to_string(), schema);
        cx.notify();
    }

    /// The gRPC definitions loaded for the address `uri`, if an API loaded
    /// any.
    pub fn grpc_definitions(&self, uri: &str) -> Option<Arc<GrpcDefinitions>> {
        self.grpc.get(uri.trim()).cloned()
    }

    pub fn keep_grpc_definitions(&mut self, uri: &str, definitions: Arc<GrpcDefinitions>, cx: &mut Context<Self>) {
        self.grpc.insert(uri.trim().to_string(), definitions);
        cx.notify();
    }

    /// Reads the variables kept here again: the project changed.
    fn refresh_kept_variables(&mut self, cx: &mut Context<Self>) {
        self.refresh_secrets(cx);
        self.refresh_environments(cx);
        self.refresh_variables(VariableCategory::Environment, cx);
        self.refresh_variables(VariableCategory::GlobalRequestHeader, cx);
        self.refresh_variables(VariableCategory::Custom, cx);
    }

    /// G2: the project's mock server, made the first time it is asked for.
    pub fn mock(&mut self, cx: &mut Context<Self>) -> Entity<MockState> {
        if let Some(mock) = &self.mock {
            return mock.clone();
        }
        let workspace = cx.entity().downgrade();
        let mock = cx.new(|cx| MockState::new(workspace, cx));
        self.mock = Some(mock.clone());
        mock
    }

    /// G2: where the mock server listens, while it runs.
    pub fn mock_address(&self, cx: &App) -> Option<String> {
        self.mock.as_ref()?.read(cx).address()
    }

    /// D3: reads the reader's secret values again — unless some typed are
    /// still to be saved, which are newer.
    fn refresh_secrets(&mut self, cx: &mut Context<Self>) {
        let call = self.sender.storage.prefs().secret_values(self.collection.id.clone());
        run_store(cx, call, |this, result, cx| match result {
            Ok(values) if this.secrets_save.is_none() => {
                this.secrets = values;
                cx.notify();
            }
            Ok(_) => {}
            Err(error) => warn!(%error, "the secret values could not be read"),
        });
    }

    /// D3: the project's secret variables: their values are each reader's
    /// own, and the project keeps none.
    pub fn secret_variables(&self) -> &[VariableId] {
        &self.data.settings.secret_variables
    }

    pub fn is_secret(&self, id: &VariableId) -> bool {
        self.secret_variables().contains(id)
    }

    /// D3: the reader's own value of a secret variable.
    pub fn secret_value(&self, id: &VariableId) -> Option<&str> {
        self.secrets.get(id)
    }

    /// D3: the reader's values of the secret variables, for a send.
    pub fn secrets(&self) -> SecretValues {
        self.secrets.of(self.secret_variables())
    }

    /// D3: the reader's own value of a secret variable, saved once the
    /// typing settles.
    pub fn set_secret_value(&mut self, id: VariableId, value: String, cx: &mut Context<Self>) {
        if self.secrets.get(&id) == Some(value.as_str()) {
            return;
        }
        self.secrets.0.insert(id, value);
        self.secrets_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(crate::states::api::SAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| this.save_secrets(cx));
        }));
        cx.notify();
    }

    /// D3: the reader's secret values saved now: each write is the whole
    /// set, one at a time.
    pub fn save_secrets(&mut self, cx: &mut Context<Self>) {
        self.secrets_save = None;
        let call = self
            .sender
            .storage
            .prefs()
            .set_secret_values(self.collection.id.clone(), self.secrets.clone());
        run_serial(cx, &self.saves, call, |_, result, cx| {
            if let Err(error) = result {
                cx.emit(WorkspaceEvent::Failed(error));
            }
        });
    }

    /// D3: the secret values typed and still waiting, saved now.
    pub fn flush_secrets(&mut self, cx: &mut Context<Self>) {
        if self.secrets_save.is_some() {
            self.save_secrets(cx);
        }
    }

    /// D3: `variable` made secret — its value becomes the reader's own, and
    /// the project keeps none — or not, the reader's value its own again.
    /// The value its row keeps now; an editor's alone.
    pub fn set_secret(&mut self, variable: &Variable, secret: bool, cx: &mut Context<Self>) -> Option<String> {
        if !self.editable() || self.is_secret(&variable.id) == secret {
            return None;
        }
        let id = variable.id.clone();
        let mut settings = self.data.settings.clone();
        let kept = if secret {
            self.secrets.0.insert(id.clone(), variable.value.clone());
            settings.secret_variables.push(id);
            String::new()
        } else {
            settings.secret_variables.retain(|secret| secret != &id);
            self.secrets.0.remove(&id).unwrap_or_default()
        };
        self.save_secrets(cx);
        self.set_project_settings(settings, cx);
        Some(kept)
    }

    /// Reads a category's variables again, when it is one kept here: they
    /// changed.
    pub fn refresh_variables(&mut self, category: VariableCategory, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .variables()
            .list(self.collection.id.clone(), category);
        run_store(cx, call, move |this, result, cx| match result {
            Ok(variables) => {
                match category {
                    VariableCategory::Environment => this.environment = variables,
                    VariableCategory::GlobalRequestHeader => this.global_headers = variables,
                    VariableCategory::Custom => this.custom = variables,
                }
                cx.notify();
            }
            Err(error) => warn!(%error, ?category, "variables could not be read"),
        });
    }

    /// F4.12: the tab the selected API's editor shows.
    /// The tab `api`'s editor shows — `api`'s own, not the opened one's:
    /// an editor in a window of its own shows another (F7, BUG-4).
    pub fn tab(&self, api: &Entity<ApiState>, cx: &gpui_kit::App) -> RequestTab {
        let api = api.read(cx);
        self.prefs.tab(api.api(), api.request())
    }

    pub fn set_tab(&mut self, api: &Entity<ApiState>, tab: RequestTab, cx: &mut Context<Self>) {
        let (id, request) = {
            let api = api.read(cx);
            (api.api().clone(), api.request().clone())
        };
        if self.prefs.tab(&id, &request) != tab {
            self.prefs.set_tab(&id, &request, tab);
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// #63: the JSONPath `api`'s responses are narrowed by.
    pub fn response_filter(&self, api: &ItemId) -> &str {
        self.prefs.filter(api)
    }

    /// Keeps `filter` for `api`, saved once the typing settles.
    pub fn set_response_filter(&mut self, api: &ItemId, filter: &str, cx: &mut Context<Self>) {
        if !self.prefs.set_filter(api, filter) {
            return;
        }
        self._filter_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(crate::states::api::SAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| this.save_prefs(cx));
        }));
    }

    /// F4.12: a new method opens on its default tab — the kept one goes.
    pub fn method_changed(&mut self, api: &ItemId, cx: &mut Context<Self>) {
        if self.prefs.tabs.remove(api).is_some() {
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// The opened API's editor, and its tab (F1): one kept from before,
    /// or a new one beside the tab shown till now. Tabs of APIs that are
    /// gone close; a scratch request stays, it is not in the tree. Past
    /// `MAX_TABS`, the one shown longest ago does.
    fn sync_api(&mut self, cx: &mut Context<Self>) {
        if self.load == Load::Loaded {
            let before = self.prefs.open.len();
            // Collected first: `retain` borrows each list while `tab_lives`
            // reads the tree and the scratches.
            let open: Vec<ItemId> = self
                .prefs
                .open
                .iter()
                .filter(|id| self.tab_lives(id))
                .cloned()
                .collect();
            let kept: HashSet<ItemId> = self.kept.keys().filter(|id| self.tab_lives(id)).cloned().collect();
            let shown: Vec<ItemId> = self.shown.iter().filter(|id| self.tab_lives(id)).cloned().collect();
            self.prefs.open = open;
            self.kept.retain(|id, _| kept.contains(id));
            self.shown = shown;
            if self.prefs.open.len() != before {
                self.save_prefs(cx);
            }
        }
        let selected = self.opened_setting();
        let current = self.api.as_ref().map(|api| api.read(cx).api().clone());
        let Some(api) = selected else {
            self.api = None;
            return;
        };
        if !self.prefs.open.contains(&api.id) {
            let beside = current
                .as_ref()
                .and_then(|current| self.prefs.open.iter().position(|open| open == current))
                .map_or(self.prefs.open.len(), |at| at + 1);
            self.prefs.open.insert(beside, api.id.clone());
            self.save_prefs(cx);
        }
        self.shown.retain(|shown| shown != &api.id);
        self.shown.push(api.id.clone());
        if current.as_ref() != Some(&api.id) {
            // Leaving a tab saves what its editor still waits to.
            if let Some(left) = &self.api {
                left.update(cx, |left, cx| left.save_now(cx));
            }
            let state = match self.kept.get(&api.id) {
                Some(kept) => kept.clone(),
                None => {
                    let (sender, workspace) = (self.sender.clone(), cx.entity().downgrade());
                    let editable = self.editable();
                    let scratch = self.is_scratch(&api.id);
                    let saves = self.saves.clone();
                    let state = cx.new(|cx| ApiState::new(sender, workspace, &api, editable, scratch, saves, cx));
                    self.kept.insert(api.id.clone(), state.clone());
                    state
                }
            };
            self.api = Some(state);
        }
        self.limit_tabs(&api.id, cx);
    }

    /// An open tab's id still names an API, or a scratch request.
    fn tab_lives(&self, id: &ItemId) -> bool {
        self.data.apis.iter().any(|api| &api.id == id) || self.is_scratch(id)
    }

    fn is_scratch(&self, id: &ItemId) -> bool {
        self.prefs.scratches.iter().any(|scratch| &scratch.id == id)
    }

    /// The API in the editor as a record: a tree API, or a scratch request
    /// worn as one so the editor can open it.
    fn opened_setting(&self) -> Option<ApiSetting> {
        let id = self.opened()?.clone();
        if let Some(api) = self.data.apis.iter().find(|api| api.id == id) {
            return Some(api.clone());
        }
        let scratch = self.prefs.scratches.iter().find(|scratch| scratch.id == id)?;
        Some(ApiSetting {
            id: scratch.id.clone(),
            collection: self.collection.id.clone(),
            name: scratch.name.clone(),
            request: scratch.request.clone(),
            ..Default::default()
        })
    }

    /// F1: past `MAX_TABS`, the tab shown longest ago closes — one never
    /// shown this session first — but not `opened`, nor one whose request
    /// is on its way.
    fn limit_tabs(&mut self, opened: &ItemId, cx: &mut Context<Self>) {
        while self.prefs.open.len() > MAX_TABS {
            let shown_at = |id: &ItemId| self.shown.iter().position(|shown| shown == id);
            let sending = |id: &ItemId| {
                self.kept
                    .get(id)
                    .is_some_and(|api| api.read(cx).sending_since().is_some())
            };
            let oldest = self
                .prefs
                .open
                .iter()
                // F7: nor one in a window of its own.
                .filter(|id| *id != opened && !sending(id) && !self.apart.contains(*id))
                .min_by_key(|id| shown_at(id).map_or(0, |at| at + 1))
                .cloned();
            let Some(oldest) = oldest else { return };
            self.forget_tab(&oldest, cx);
            self.save_prefs(cx);
        }
    }

    /// F1: the tab and its editor gone. A project's API saves what it still
    /// held; a scratch request is dropped with the tab.
    fn forget_tab(&mut self, id: &ItemId, cx: &mut Context<Self>) {
        let scratch = self.is_scratch(id);
        self.apart.remove(id);
        self.prefs.open.retain(|open| open != id);
        self.shown.retain(|shown| shown != id);
        self.prefs.scratches.retain(|scratch| &scratch.id != id);
        if let Some(api) = self.kept.remove(id) {
            // A scratch's release would write it back. Closing drops it.
            if scratch {
                api.update(cx, |api, _| api.forget_unsaved());
            }
        }
    }

    /// An open tab's editor made again from the API as kept: its edits not
    /// saved yet are given up.
    fn start_again(&mut self, id: &ItemId, cx: &mut Context<Self>) {
        self.apart.remove(id);
        if let Some(api) = self.kept.remove(id) {
            api.update(cx, |api, _| api.forget_unsaved());
        }
        if self.api.as_ref().is_some_and(|api| api.read(cx).api() == id) {
            self.api = None;
        }
    }

    /// F1: the open tabs, in order, that still exist. A scratch request is
    /// one of them and is not in the tree.
    pub fn tabs(&self) -> Vec<OpenTab> {
        self.prefs
            .open
            .iter()
            .filter_map(|id| {
                if let Some(api) = self.data.apis.iter().find(|api| &api.id == id) {
                    return Some(OpenTab {
                        id: id.clone(),
                        method: api.request.method,
                        name: api.name.clone(),
                        scratch: false,
                    });
                }
                self.prefs
                    .scratches
                    .iter()
                    .find(|scratch| &scratch.id == id)
                    .map(|scratch| OpenTab {
                        id: id.clone(),
                        method: scratch.request.method,
                        name: scratch.name.clone(),
                        scratch: true,
                    })
            })
            .collect()
    }

    /// F1: the scratch request `id`, when it is one.
    pub fn scratch(&self, id: &ItemId) -> Option<&Scratch> {
        self.prefs.scratches.iter().find(|scratch| &scratch.id == id)
    }

    /// F1: whether `id`'s tab has a request on its way.
    pub fn tab_sending(&self, id: &ItemId, cx: &gpui_kit::App) -> bool {
        self.kept
            .get(id)
            .is_some_and(|api| api.read(cx).sending_since().is_some())
    }

    /// F1: `id`'s tab closed. Closing the one opened opens the one after
    /// it, else the one before; with none left, nothing is.
    /// F7: the open tab `id` in a window of its own, its editor that
    /// window's alone — two editors of one request would each write back
    /// what they hold. The view opens the window (`Parted`).
    pub fn part(&mut self, id: &ItemId, cx: &mut Context<Self>) {
        if !self.prefs.open.contains(id) {
            return;
        }
        if !self.kept.contains_key(id) {
            self.select(id, cx);
        }
        if !self.kept.contains_key(id) {
            return;
        }
        self.apart.insert(id.clone());
        cx.emit(WorkspaceEvent::Parted(id.clone()));
        cx.notify();
    }

    /// F7: `id`'s window closed: its tab is edited here again.
    pub fn join(&mut self, id: &ItemId, cx: &mut Context<Self>) {
        if self.apart.remove(id) {
            cx.notify();
        }
    }

    /// F7: whether the open tab `id` is in a window of its own.
    pub fn is_apart(&self, id: &ItemId) -> bool {
        self.apart.contains(id) && self.prefs.open.contains(id)
    }

    /// F7: the editor state of the open tab `id`, once it has been shown.
    pub fn kept_api(&self, id: &ItemId) -> Option<&Entity<ApiState>> {
        self.kept.get(id)
    }

    pub fn close_tab(&mut self, id: &ItemId, cx: &mut Context<Self>) {
        let Some(at) = self.prefs.open.iter().position(|open| open == id) else {
            return;
        };
        let opened = self.opened() == Some(id);
        let next = self.tab_beside(at, &|other| other == id);
        self.forget_tab(id, cx);
        if opened {
            self.pin_open = None;
            self.api = None;
            match next {
                Some(next) => self.prefs.selected = Some(next),
                None => self.prefs.selected = None,
            }
        }
        self.sync_api(cx);
        self.save_prefs(cx);
        cx.notify();
    }

    /// F1: the tab after the one at `at`, else the one before it, of those
    /// not `going`.
    fn tab_beside(&self, at: usize, going: &dyn Fn(&ItemId) -> bool) -> Option<ItemId> {
        let open = &self.prefs.open;
        open.iter()
            .skip(at + 1)
            .chain(open.iter().take(at).rev())
            .find(|id| !going(id))
            .cloned()
    }

    /// F1: every tab closed but `id`'s, which is opened.
    pub fn close_other_tabs(&mut self, id: &ItemId, cx: &mut Context<Self>) {
        let others: Vec<ItemId> = self.prefs.open.iter().filter(|open| *open != id).cloned().collect();
        for other in &others {
            self.forget_tab(other, cx);
        }
        if self.opened() != Some(id) {
            self.select(id, cx);
        }
        self.save_prefs(cx);
        cx.notify();
    }

    /// F1: every tab closed; nothing is open.
    pub fn close_all_tabs(&mut self, cx: &mut Context<Self>) {
        for id in self.prefs.open.clone() {
            self.forget_tab(&id, cx);
        }
        self.pin_open = None;
        self.prefs.selected = None;
        self.sync_api(cx);
        self.save_prefs(cx);
        cx.notify();
    }

    /// F1: the tab `by` places after the opened one (before it, below
    /// zero), round the ends.
    pub fn step_tab(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = self.prefs.open.len() as isize;
        let Some(at) = self
            .opened()
            .and_then(|opened| self.prefs.open.iter().position(|open| open == opened))
        else {
            return;
        };
        if count < 2 {
            return;
        }
        let next = (at as isize + by).rem_euclid(count) as usize;
        let id = self.prefs.open[next].clone();
        self.select(&id, cx);
    }

    /// An API's request was saved: the tree's copy follows (its method label).
    pub fn api_saved(&mut self, setting: ApiSetting, cx: &mut Context<Self>) {
        if let Some(api) = self.data.apis.iter_mut().find(|api| api.id == setting.id) {
            *api = setting;
            self.build_tree();
            cx.notify();
        }
    }

    pub fn load(&self) -> &Load {
        &self.load
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// The project's folders and APIs as kept.
    pub fn tree_data(&self) -> &TreeData {
        &self.data
    }

    pub fn keyword(&self) -> &str {
        &self.keyword
    }

    /// The tree's selected row.
    pub fn selected(&self) -> Option<&ItemId> {
        self.prefs.selected.as_ref()
    }

    /// The API in the editor: a pin opened over the tree's selection, else
    /// that selection.
    pub fn opened(&self) -> Option<&ItemId> {
        self.pin_open
            .as_ref()
            .filter(|pin| self.data.apis.iter().any(|api| &api.id == *pin))
            .or(self.prefs.selected.as_ref())
    }

    /// The opened API's record, when it still exists.
    pub fn opened_api(&self) -> Option<&ApiSetting> {
        let opened = self.opened()?;
        self.data.apis.iter().find(|api| &api.id == opened)
    }

    /// Pinned APIs that still exist, in pin order (F12.2).
    pub fn pinned(&self) -> Vec<&ApiSetting> {
        self.prefs
            .pinned
            .iter()
            .filter_map(|id| self.data.apis.iter().find(|api| &api.id == id))
            .collect()
    }

    pub fn is_pinned(&self, id: &ItemId) -> bool {
        self.prefs.pinned.contains(id)
    }

    /// The rows to draw under the current keyword and open folders.
    /// The folder the selected API is in: where an API added beside it
    /// goes. `None` on the top level, or with nothing selected.
    pub fn selected_folder(&self) -> Option<ItemId> {
        let selected = self.prefs.selected.as_ref()?;
        self.tree.get(selected).and_then(|node| node.parent()).cloned()
    }

    pub fn rows(&self) -> Rc<[Row]> {
        if let Some((of, rows)) = &*self.rows.borrow()
            && of.tree == self.tree_built
            && of.keyword == self.keyword
            && of.expanded == self.prefs.expanded
        {
            return rows.clone();
        }
        let expanded: HashSet<ItemId> = self.prefs.expanded.iter().cloned().collect();
        let rows: Rc<[Row]> = self.tree.rows(&expanded, &self.keyword).into();
        let of = RowsOf {
            tree: self.tree_built,
            keyword: self.keyword.clone(),
            expanded: self.prefs.expanded.clone(),
        };
        *self.rows.borrow_mut() = Some((of, rows.clone()));
        rows
    }

    /// The tree made again from the data.
    fn build_tree(&mut self) {
        self.tree = Tree::build(&self.data);
        self.tree_built += 1;
    }

    /// Loads (or loads again) the tree and the reader's state of it. A
    /// server project's copy that holds nothing yet is opened first.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if self.copy.is_some() && self.load != Load::Loaded {
            return self.open_copy(cx);
        }
        self.load_all(cx);
    }

    fn load_all(&mut self, cx: &mut Context<Self>) {
        self.refresh_kept_variables(cx);
        let id = self.collection.id.clone();
        run_store(cx, self.sender.storage.tree().load(id.clone()), |this, result, cx| {
            match result {
                Ok(data) => {
                    // Loaded first: the open tabs are checked against it.
                    this.load = Load::Loaded;
                    this.set_data(data, cx);
                    if let Some(import) = this.waiting_import.take() {
                        this.import(None, Arc::unwrap_or_clone(import), cx);
                    }
                    this.select_waiting(cx);
                }
                Err(error) => this.load = Load::Failed(error),
            }
            cx.notify();
        });
        run_store(cx, self.sender.storage.prefs().workspace(id), |this, result, cx| {
            match result {
                Ok(mut prefs) => {
                    // Pins live on this machine. A list already here wins,
                    // including an empty one. No list yet, and the document
                    // still has the old pins: adopt them, then store the
                    // document without that field.
                    let on_server = !prefs.pinned.is_empty();
                    if let Some(pinned) = crate::pins_local::get(&pin_scope(cx), &this.collection.id) {
                        prefs.pinned = pinned;
                    }
                    this.prefs = prefs;
                    this.sync_api(cx);
                    if on_server {
                        this.save_prefs(cx);
                    }
                }
                // The tree still works without them; the reader starts from
                // a closed tree.
                Err(error) => warn!(%error, "the workspace state could not be read"),
            }
            this.prefs_read = true;
            this.select_waiting(cx);
            cx.notify();
        });
    }

    /// F2: `api` selected as soon as the project has loaded.
    pub fn select_when_loaded(&mut self, api: ItemId, cx: &mut Context<Self>) {
        self.waiting_select = Some(api);
        self.select_waiting(cx);
    }

    fn select_waiting(&mut self, cx: &mut Context<Self>) {
        if self.load != Load::Loaded || !self.prefs_read {
            return;
        }
        if let Some(api) = self.waiting_select.take()
            && self.data.apis.iter().any(|found| found.id == api)
        {
            self.select(&api, cx);
        }
    }

    fn set_data(&mut self, data: TreeData, cx: &mut Context<Self>) {
        self.data = data;
        self.build_tree();
        self.sync_api(cx);
    }

    /// F3.2.
    pub fn set_keyword(&mut self, keyword: String, cx: &mut Context<Self>) {
        if self.keyword != keyword {
            self.keyword = keyword;
            cx.notify();
        }
    }

    /// F3.3.
    pub fn toggle_folder(&mut self, id: &ItemId, cx: &mut Context<Self>) {
        if let Some(at) = self.prefs.expanded.iter().position(|open| open == id) {
            self.prefs.expanded.remove(at);
        } else {
            self.prefs.expanded.push(id.clone());
        }
        self.save_prefs(cx);
        cx.notify();
    }

    /// F3.3: "collapse all" closes every folder of the project.
    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        if !self.prefs.expanded.is_empty() {
            self.prefs.expanded.clear();
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// F3.8: an API is selected, and opened in the editor over any pin; a
    /// folder is opened or closed instead. The folders it sits in open, so
    /// its row shows.
    pub fn select(&mut self, id: &ItemId, cx: &mut Context<Self>) {
        // Close the pin before the `||`. When the tree row changes, the
        // left side is already true and a `take` on the right never runs:
        // the row looks selected while the editor keeps the pin, until the
        // same row is chosen again.
        let closed_pin = self.pin_open.take();
        let mut changed = self.prefs.selected.as_ref() != Some(id) || closed_pin.is_some();
        self.prefs.selected = Some(id.clone());
        let mut parent = self.tree.get(id).and_then(|node| node.parent()).cloned();
        while let Some(folder) = parent {
            parent = self.tree.get(&folder).and_then(|node| node.parent()).cloned();
            if !self.prefs.expanded.contains(&folder) {
                self.prefs.expanded.push(folder);
                changed = true;
            }
        }
        if changed {
            self.sync_api(cx);
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// F12.2: a pin's API in the editor. The tree keeps its selection and
    /// its folders: a pin is for the APIs used beside the one at hand.
    pub fn open_pin(&mut self, id: &ItemId, cx: &mut Context<Self>) {
        let pin = Some(id.clone()).filter(|id| self.prefs.selected.as_ref() != Some(id));
        if pin != self.pin_open {
            self.pin_open = pin;
            self.sync_api(cx);
            cx.notify();
        }
    }

    /// F12.1 / F12.3. Only this project's APIs, once each. Closing the pin
    /// that is open takes the editor back to the tree's selection.
    pub fn set_pinned(&mut self, id: &ItemId, pinned: bool, cx: &mut Context<Self>) {
        let present = self.prefs.pinned.contains(id);
        if pinned && !present && self.data.apis.iter().any(|api| &api.id == id) {
            self.prefs.pinned.push(id.clone());
        } else if !pinned && present {
            self.prefs.pinned.retain(|pin| pin != id);
            if self.pin_open.as_ref() == Some(id) {
                self.pin_open = None;
                self.sync_api(cx);
            }
        } else {
            return;
        }
        self.save_prefs(cx);
        cx.notify();
    }

    /// F1: a scratch request, not in the tree, opened as a tab. It uses this
    /// project's variables and environment. A viewer may make one: it is
    /// theirs, not the project's. `name` is what the tab shows.
    pub fn new_scratch(&mut self, name: String, cx: &mut Context<Self>) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let scratch = Scratch {
            id: scratch_id(),
            name,
            request: HttpRequest::default(),
        };
        let id = scratch.id.clone();
        self.prefs.scratches.push(scratch);
        self.select(&id, cx);
    }

    /// F1: the editor's request, kept on the scratch. Nothing happens once
    /// the tab was closed: closing drops it, and a late save must not put
    /// it back.
    pub fn keep_scratch(&mut self, id: &ItemId, request: HttpRequest, cx: &mut Context<Self>) {
        let Some(scratch) = self.prefs.scratches.iter_mut().find(|scratch| &scratch.id == id) else {
            return;
        };
        if scratch.request == request {
            return;
        }
        scratch.request = request;
        self.save_prefs(cx);
        cx.notify();
    }

    /// F1: the scratch becomes an API in `parent` (the top level when none).
    /// The editor's request is written before the new API is opened. Only an
    /// editor of the project can; a viewer keeps the scratch.
    pub fn save_scratch_into(&mut self, id: ItemId, parent: Option<ItemId>, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let Some(scratch) = self.prefs.scratches.iter().find(|scratch| scratch.id == id).cloned() else {
            return;
        };
        let request = self
            .kept
            .get(&id)
            .map(|api| api.read(cx).request().clone())
            .unwrap_or(scratch.request);
        let call = self
            .sender
            .storage
            .tree()
            .create_api(self.collection.id.clone(), parent.clone(), scratch.name);
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(inserted) => {
                let new_id = inserted.item.id.clone();
                let change = inserted.change;
                let call = this
                    .sender
                    .storage
                    .tree()
                    .save_request(this.collection.id.clone(), new_id.clone(), request);
                run_serial(cx, &this.gate, call, move |this, result, cx| match result {
                    Ok(setting) => {
                        this.data.apis.push(setting);
                        this.data.apply(&change);
                        this.build_tree();
                        this.open_folder(parent.as_ref());
                        for open in &mut this.prefs.open {
                            if *open == id {
                                *open = new_id.clone();
                            }
                        }
                        for shown in &mut this.shown {
                            if *shown == id {
                                *shown = new_id.clone();
                            }
                        }
                        if this.prefs.selected.as_ref() == Some(&id) {
                            this.prefs.selected = Some(new_id.clone());
                        }
                        this.prefs.scratches.retain(|scratch| scratch.id != id);
                        if let Some(old) = this.kept.remove(&id) {
                            old.update(cx, |api, _| api.forget_unsaved());
                        }
                        if this.api.as_ref().is_some_and(|api| api.read(cx).api() == &id) {
                            this.api = None;
                        }
                        this.created = Some(new_id);
                        this.pin_open = None;
                        this.sync_api(cx);
                        this.save_prefs(cx);
                        cx.notify();
                    }
                    Err(error) => {
                        // The empty API was written. Take it back; the scratch stays.
                        let created = new_id.clone();
                        let call = this.sender.storage.tree().delete(this.collection.id.clone(), created);
                        run_serial(cx, &this.gate, call, move |_, _, cx| {
                            cx.emit(WorkspaceEvent::Failed(error));
                        });
                    }
                });
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// F3.4: a new API is selected, and the folder it went into opens.
    pub fn create_api(&mut self, parent: Option<ItemId>, name: String, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .tree()
            .create_api(self.collection.id.clone(), parent.clone(), name);
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(inserted) => {
                this.data.apis.push(inserted.item.clone());
                this.data.apply(&inserted.change);
                this.build_tree();
                this.open_folder(parent.as_ref());
                this.created = Some(inserted.item.id.clone());
                this.prefs.selected = Some(inserted.item.id);
                this.pin_open = None;
                this.sync_api(cx);
                this.save_prefs(cx);
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// Whether `id` is the API just made, asked once by its editor.
    pub fn take_created(&mut self, id: &ItemId) -> bool {
        if self.created.as_ref() == Some(id) {
            self.created = None;
            return true;
        }
        false
    }

    /// F3.4.
    pub fn create_folder(&mut self, parent: Option<ItemId>, name: String, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .tree()
            .create_folder(self.collection.id.clone(), parent.clone(), name);
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(inserted) => {
                let folder: Folder = inserted.item;
                this.data.folders.push(folder);
                this.data.apply(&inserted.change);
                this.build_tree();
                this.open_folder(parent.as_ref());
                this.save_prefs(cx);
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    fn open_folder(&mut self, folder: Option<&ItemId>) {
        if let Some(folder) = folder
            && !self.prefs.expanded.contains(folder)
        {
            self.prefs.expanded.push(folder.clone());
        }
    }

    /// F3.5.
    pub fn rename(&mut self, id: ItemId, name: String, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .tree()
            .rename(self.collection.id.clone(), id.clone(), name.clone());
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(()) => {
                // The store keeps the name trimmed.
                let name = name.trim().to_string();
                if let Some(folder) = this.data.folders.iter_mut().find(|folder| folder.id == id) {
                    folder.name = name;
                } else if let Some(api) = this.data.apis.iter_mut().find(|api| api.id == id) {
                    api.name = name;
                }
                this.build_tree();
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// #66: what a folder sets for the APIs in it.
    pub fn set_folder_settings(&mut self, id: ItemId, settings: FolderSettings, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .tree()
            .set_folder_settings(self.collection.id.clone(), id, settings);
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(folder) => {
                if let Some(found) = this.data.folders.iter_mut().find(|found| found.id == folder.id) {
                    *found = folder;
                }
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// #66: what the project sets for its APIs.
    pub fn set_project_settings(&mut self, settings: ProjectSettings, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .tree()
            .set_project_settings(self.collection.id.clone(), settings.clone());
        // Held at once: a change made before this one is written starts
        // from it, not from what it replaces (BUG-19).
        let before = std::mem::replace(&mut self.data.settings, settings.clone());
        cx.notify();
        run_serial(cx, &self.gate, call, move |this, result, cx| {
            if let Err(error) = result {
                // Not written: as it was, unless another change came since.
                if this.data.settings == settings {
                    this.data.settings = before;
                    cx.notify();
                }
                cx.emit(WorkspaceEvent::Failed(error));
            }
        });
    }

    /// F3.7. The server's items stay out of the reader's own folders
    /// (server-sync §3.4), where a sync would leave them out.
    pub fn move_item(&mut self, id: ItemId, to: Placement, cx: &mut Context<Self>) {
        if let Some(copy) = &self.copy {
            if !copy
                .copy
                .local_items()
                .may_move(&self.data.folders, &id, to.parent.as_ref())
            {
                return cx.emit(WorkspaceEvent::NotIntoLocal);
            }
            copy.copy.moving(&self.data.folders, &id, to.parent.as_ref());
        }
        let call = self
            .sender
            .storage
            .tree()
            .move_item(self.collection.id.clone(), id, to.clone());
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(change) => {
                this.data.apply(&change);
                this.build_tree();
                this.open_folder(to.parent.as_ref());
                this.save_prefs(cx);
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// A copy of the API named `name`, right after it in its folder, and
    /// selected. Its request is the one in the editor, when the API is the
    /// one open — edits not saved yet go too; its history starts empty.
    pub fn duplicate(&mut self, id: &ItemId, name: String, cx: &mut Context<Self>) {
        let Some(stored) = self.data.apis.iter().find(|api| &api.id == id) else {
            return;
        };
        let request = match self.kept.get(id) {
            Some(open) => open.read(cx).request().clone(),
            None => stored.request.clone(),
        };
        let items = vec![ImportItem::Api {
            name,
            request: Box::new(request),
        }];
        let parent = self.tree.get(id).and_then(|node| node.parent()).cloned();
        let before = self.tree.next_sibling(id).cloned();
        let call = self
            .sender
            .storage
            .tree()
            .import(self.collection.id.clone(), parent.clone(), items);
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(imported) => {
                let copy = imported.apis.first().map(|api| api.id.clone());
                this.data.apis.extend(imported.apis);
                this.data.apply(&imported.change);
                this.build_tree();
                let Some(copy) = copy else { return };
                // Imported last in the folder: moved up beside the original.
                if before.is_some() {
                    this.move_item(copy.clone(), Placement { parent, before }, cx);
                }
                this.select(&copy, cx);
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// F10.4: the bundle under `parent` (the top level when `None`), and its
    /// environment variables — those with a name or a value. One API alone
    /// (a cURL command, F10.5) is opened.
    /// #65: `import` on the top level, as soon as the tree has loaded — a
    /// server project's copy loads only once it is open.
    pub fn import_when_loaded(&mut self, import: Arc<ImportBundle>, cx: &mut Context<Self>) {
        if self.load == Load::Loaded {
            self.import(None, Arc::unwrap_or_clone(import), cx);
        } else {
            self.waiting_import = Some(import);
        }
    }

    pub fn import(&mut self, parent: Option<ItemId>, bundle: ImportBundle, cx: &mut Context<Self>) {
        let source = source_of(bundle.source, &parent);
        self.import_settings(bundle.auth, bundle.environments, source, cx);
        let call = self
            .sender
            .storage
            .tree()
            .import(self.collection.id.clone(), parent.clone(), bundle.items);
        let environment: Vec<(String, String)> = bundle
            .environment
            .into_iter()
            .filter(|(name, value)| !name.trim().is_empty() || !value.trim().is_empty())
            .collect();
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(imported) => {
                let (apis, folders) = (imported.apis.len(), imported.folders.len());
                let only = match (&imported.apis[..], folders) {
                    ([api], 0) => Some(api.id.clone()),
                    _ => None,
                };
                this.data.folders.extend(imported.folders);
                this.data.apis.extend(imported.apis);
                this.data.apply(&imported.change);
                this.build_tree();
                this.open_folder(parent.as_ref());
                if let Some(api) = only {
                    this.select(&api, cx);
                }
                this.save_prefs(cx);
                this.add_environment(environment, cx);
                cx.emit(WorkspaceEvent::Imported {
                    apis,
                    folders,
                    again: None,
                });
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// B3: `bundle` imported into `parent` again: an API the project has —
    /// the same method to the same path — is updated (`update`) or left, not
    /// added a second time; what is new goes into the folders of the same
    /// name (`import::merge`); an environment variable it has is not added
    /// twice.
    pub fn import_again(&mut self, parent: Option<ItemId>, bundle: ImportBundle, update: bool, cx: &mut Context<Self>) {
        // The open tabs as their editors hold them: an edit not saved yet is
        // what updating starts from.
        let open: Vec<(ItemId, HttpRequest)> = self
            .kept
            .values()
            .map(|api| {
                let api = api.read(cx);
                (api.api().clone(), api.request().clone())
            })
            .collect();
        let mut data = std::borrow::Cow::Borrowed(&self.data);
        for (id, request) in &open {
            if data.apis.iter().any(|api| &api.id == id && &api.request != request)
                && let Some(api) = data.to_mut().apis.iter_mut().find(|api| &api.id == id)
            {
                api.request = request.clone();
            }
        }
        let merge = cyberapi_core::import::merge::merge(&data, parent.as_ref(), &bundle.items, update);
        let source = source_of(bundle.source, &parent);
        self.import_settings(bundle.auth, bundle.environments, source, cx);
        let (apis, folders) = merge.added();
        let again = Some((merge.updates.len(), merge.matched - merge.updates.len()));
        let known: Vec<&str> = self.environment.iter().map(|variable| variable.name.as_str()).collect();
        let environment: Vec<(String, String)> = bundle
            .environment
            .into_iter()
            .filter(|(name, value)| !name.trim().is_empty() || !value.trim().is_empty())
            .filter(|(name, _)| !known.contains(&name.as_str()))
            .collect();
        self.add_environment(environment, cx);
        let calls = merge.updates.len() + merge.additions.len();
        if calls == 0 {
            cx.emit(WorkspaceEvent::Imported { apis, folders, again });
            return;
        }
        let tree = self.sender.storage.tree();
        let collection = self.collection.id.clone();
        // Said once, when the last call is answered: they are answered in
        // the order they are made.
        let mut left = calls;
        let mut said = move || {
            left -= 1;
            left == 0
        };
        for (id, request) in merge.updates {
            let done = said();
            let call = tree.save_request(collection.clone(), id.clone(), request);
            run_serial(cx, &self.gate, call, move |this, result, cx| {
                match result {
                    Ok(setting) => {
                        this.api_saved(setting, cx);
                        // Its editor starts again from what the import made.
                        if this.kept.contains_key(&id) {
                            this.start_again(&id, cx);
                            this.sync_api(cx);
                        }
                    }
                    Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
                }
                if done {
                    cx.emit(WorkspaceEvent::Imported { apis, folders, again });
                }
                cx.notify();
            });
        }
        for (place, items) in merge.additions {
            let done = said();
            let call = tree.import(collection.clone(), place.clone(), items);
            run_serial(cx, &self.gate, call, move |this, result, cx| {
                match result {
                    Ok(imported) => {
                        this.data.folders.extend(imported.folders);
                        this.data.apis.extend(imported.apis);
                        this.data.apply(&imported.change);
                        this.build_tree();
                        this.open_folder(place.as_ref());
                        this.save_prefs(cx);
                    }
                    Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
                }
                if done {
                    cx.emit(WorkspaceEvent::Imported { apis, folders, again });
                }
                cx.notify();
            });
        }
    }

    /// B7: where the project syncs from, when it has a source.
    pub fn source(&self) -> Option<&ImportSource> {
        self.data.settings.source.as_ref()
    }

    /// B7: the folder a sync goes into: the source's, while the project
    /// still has it; the top level otherwise.
    pub fn source_folder(&self) -> Option<ItemId> {
        let folder = self.source()?.folder.as_ref()?;
        self.data
            .folders
            .iter()
            .any(|known| &known.id == folder)
            .then(|| folder.clone())
    }

    /// B7: what was fetched from `url` again: the APIs the project has are
    /// updated, new ones added where the source's went, none removed; `url`
    /// stays the source.
    pub fn sync_from_source(&mut self, url: String, mut bundle: ImportBundle, cx: &mut Context<Self>) {
        bundle.source = Some(url);
        let folder = self.source_folder();
        self.import_again(folder, bundle, true, cx);
    }

    /// B7: the project keeps no source.
    pub fn forget_source(&mut self, cx: &mut Context<Self>) {
        if self.data.settings.source.is_some() {
            let settings = ProjectSettings {
                source: None,
                ..self.data.settings.clone()
            };
            self.set_project_settings(settings, cx);
        }
    }

    /// B4: what an import brings besides its items: how the project signs
    /// in, unless it does already; named environments, made where the
    /// project has none of the name (in any case), each given the variables
    /// it lacks. B7: the source it was fetched from, when the reader keeps
    /// it.
    fn import_settings(
        &mut self,
        auth: Auth,
        environments: Vec<ImportEnvironment>,
        source: Option<ImportSource>,
        cx: &mut Context<Self>,
    ) {
        let mut settings = self.data.settings.clone();
        if auth.signs() && !settings.auth.signs() {
            settings.auth = auth;
        }
        if let Some(mut source) = source {
            // Q-3: the same address imported again from elsewhere — the
            // project menu's import is into the top level — keeps the
            // folder it went into, while there is one.
            if let Some(kept) = &settings.source
                && kept.url == source.url
                && let Some(folder) = &kept.folder
                && self.data.folders.iter().any(|found| &found.id == folder)
            {
                source.folder = Some(folder.clone());
            }
            settings.source = Some(source);
        }
        if settings != self.data.settings {
            self.set_project_settings(settings, cx);
        }
        let environments: Vec<ImportEnvironment> = environments
            .into_iter()
            .filter(|environment| !environment.name.trim().is_empty())
            .collect();
        if environments.is_empty() {
            return;
        }
        let (storage, collection) = (self.sender.storage.clone(), self.collection.id.clone());
        let named = self.named.clone();
        let known: Vec<(Option<EnvironmentId>, String)> = self
            .environment
            .iter()
            .map(|variable| (variable.environment.clone(), variable.name.clone()))
            .collect();
        cx.spawn(async move |this, cx| {
            let failed = |error, cx: &mut gpui_kit::AsyncApp| {
                let _ = this.update(cx, |_, cx| cx.emit(WorkspaceEvent::Failed(error)));
            };
            for imported in environments {
                let name = imported.name.trim().to_string();
                let id = match named
                    .iter()
                    .find(|environment| environment.name.eq_ignore_ascii_case(&name))
                {
                    Some(environment) => environment.id.clone(),
                    None => {
                        let call = storage.variables().create_environment(collection.clone(), name);
                        match cx.background_spawn(call).await {
                            Ok(made) => made.id,
                            Err(error) => {
                                failed(error, cx);
                                continue;
                            }
                        }
                    }
                };
                for (name, value) in imported.variables {
                    let has = known
                        .iter()
                        .any(|(environment, known)| environment.as_ref() == Some(&id) && *known == name);
                    if has || name.trim().is_empty() {
                        continue;
                    }
                    let call = storage.variables().create(cyberapi_store::NewVariable {
                        collection: collection.clone(),
                        category: VariableCategory::Environment,
                        name,
                        value,
                        enabled: true,
                        environment: Some(id.clone()),
                    });
                    if let Err(error) = cx.background_spawn(call).await {
                        failed(error, cx);
                    }
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.refresh_environments(cx);
                this.refresh_variables(VariableCategory::Environment, cx);
            });
        })
        .detach();
    }

    fn add_environment(&mut self, variables: Vec<(String, String)>, cx: &mut Context<Self>) {
        if variables.is_empty() {
            return;
        }
        let (storage, collection) = (self.sender.storage.clone(), self.collection.id.clone());
        cx.spawn(async move |this, cx| {
            for (name, value) in variables {
                let call = storage.variables().create(cyberapi_store::NewVariable {
                    collection: collection.clone(),
                    category: VariableCategory::Environment,
                    name,
                    value,
                    enabled: true,
                    environment: None,
                });
                if let Err(error) = cx.background_spawn(call).await {
                    let _ = this.update(cx, |_, cx| cx.emit(WorkspaceEvent::Failed(error)));
                }
            }
            let _ = this.update(cx, |this, cx| this.refresh_variables(VariableCategory::Environment, cx));
        })
        .detach();
    }

    /// F11.1 / F11.2: the project, or a folder with what is under it, in
    /// CyberAPI's format, and the file name it is saved as.
    /// F3.9: the API's record as JSON, which an import reads back.
    pub fn api_config(&self, id: &ItemId) -> Option<String> {
        let api = self.data.apis.iter().find(|api| &api.id == id)?;
        Some(cyberapi_core::import::api_config(api))
    }

    /// The method of the API `id`.
    pub fn api_method(&self, id: &ItemId) -> Option<HttpMethod> {
        self.data
            .apis
            .iter()
            .find(|api| &api.id == id)
            .map(|api| api.request.method)
    }

    /// F3.9 / F7.8 / #69: the API as code, prepared as sending would
    /// prepare its stored request.
    pub fn copy_code(&mut self, id: &ItemId, target: CodeTarget, cx: &mut Context<Self>) {
        let Some(api) = self.data.apis.iter().find(|api| &api.id == id) else {
            return;
        };
        let sender = Sender {
            inherited: self.inherited(id),
            environment: self.current_environment().cloned(),
            secrets: self.secrets(),
            ..self.sender.clone()
        };
        let answers = Answers {
            file: None,
            prompts: super::api::typed_prompts(&self.collection.id, cx),
        };
        let call = cyberapi_service::code(sender, self.collection.id.clone(), api.request.clone(), answers, target);
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(call).await;
            let _ = this.update(cx, |_, cx| match result {
                Ok(code) => cx.emit(WorkspaceEvent::Code(target, code)),
                Err(failure) => cx.emit(WorkspaceEvent::CurlFailed(failure.to_string())),
            });
        })
        .detach();
    }

    /// F11.1 / F11.2, B1, B2: the project, or a folder with what is in it,
    /// in `format`, and the file name it is saved as. A Postman collection
    /// takes the variables every environment shares; an OpenAPI document
    /// reads its servers in the reader's environment.
    pub fn export(&self, folder: Option<&ItemId>, format: ExportFormat) -> (String, String) {
        let name = folder
            .and_then(|id| self.tree.get(id))
            .map(|node| node.name().to_string())
            .unwrap_or_else(|| self.collection.name.clone());
        let pairs = |variables: Vec<&Variable>| -> Vec<(String, String)> {
            variables
                .into_iter()
                .map(|variable| (variable.name.clone(), variable.value.clone()))
                .collect()
        };
        match format {
            ExportFormat::CyberApi => (
                format!("cyberapi-{name}.json"),
                cyberapi_core::import::export(&self.data, folder),
            ),
            ExportFormat::Postman => {
                let shared = pairs(
                    self.environment
                        .iter()
                        .filter(|variable| variable.enabled && variable.environment.is_none())
                        .collect(),
                );
                (
                    format!("{name}.postman_collection.json"),
                    cyberapi_core::export::postman(&self.data, folder, &name, &shared),
                )
            }
            ExportFormat::OpenApi => {
                // As the project keeps them: a secret one's value is no one's
                // to export (D3).
                let effective: Vec<Variable> = self
                    .in_environment(&self.environment)
                    .into_iter()
                    .filter(|variable| variable.enabled)
                    .collect();
                (
                    format!("{name}.openapi.yaml"),
                    cyberapi_core::export::openapi(&self.data, folder, &name, &pairs(effective.iter().collect())),
                )
            }
        }
    }

    /// F3.6. The view asks first. What was removed leaves the reader's state
    /// too: selection, pins, open folders.
    pub fn delete(&mut self, id: ItemId, cx: &mut Context<Self>) {
        self.delete_told(id, true, cx);
    }

    /// Deletes `id`; `told`: the reader is told, and offered to undo it.
    fn delete_told(&mut self, id: ItemId, told: bool, cx: &mut Context<Self>) {
        // F6: what goes, kept to be put back.
        let deleted = undo::deleted(&self.data, &id).filter(|_| told);
        // BUG-3: the delete drops the marks of the reader's own items in it.
        let local: Vec<ItemId> = match (&deleted, &self.copy) {
            (Some(deleted), Some(copy)) => copy
                .copy
                .local_items()
                .roots
                .into_iter()
                .filter(|root| deleted.holds(root))
                .collect(),
            _ => Vec::new(),
        };
        let call = self.sender.storage.tree().delete(self.collection.id.clone(), id);
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(removal) => {
                let gone = |id: &ItemId| removal.folders.contains(id) || removal.apis.contains(id);
                if let Some(deleted) = deleted {
                    this.deletes += 1;
                    let count = this.deletes;
                    let (name, apis) = (deleted.name.clone(), deleted.apis.len());
                    this.undo = Some(Undo {
                        count,
                        pinned: this.prefs.pinned.iter().filter(|id| gone(id)).cloned().collect(),
                        expanded: this.prefs.expanded.iter().filter(|id| gone(id)).cloned().collect(),
                        deleted,
                        local,
                    });
                    cx.emit(WorkspaceEvent::Deleted { name, apis, count });
                }
                // F1: the open tab beside the opened API takes its place.
                let beside = this
                    .opened()
                    .filter(|opened| gone(opened))
                    .and_then(|opened| this.prefs.open.iter().position(|open| open == opened))
                    .and_then(|at| this.tab_beside(at, &gone));
                this.data.apply_removal(&removal);
                this.build_tree();
                if this.prefs.selected.as_ref().is_some_and(gone) || beside.is_some() {
                    this.prefs.selected = beside;
                }
                if this.pin_open.as_ref().is_some_and(gone) {
                    this.pin_open = None;
                }
                this.prefs.pinned.retain(|id| !gone(id));
                this.prefs.expanded.retain(|id| !gone(id));
                this.sync_api(cx);
                this.save_prefs(cx);
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// F5: `id` copied into project `target` — at its top level, or into
    /// its `folder` — with new ids, then, `moving`, deleted here. Written
    /// into the app's store: on the desktop a server project's copy is
    /// there, and is marked unsynced; a page writes to the server. The
    /// history does not go with it.
    pub fn transfer(
        &mut self,
        id: ItemId,
        target: Collection,
        folder: Option<ItemId>,
        moving: bool,
        cx: &mut Context<Self>,
    ) {
        if moving && !self.editable() || target.id == self.collection.id {
            return;
        }
        let Some(item) = cyberapi_core::transfer::item(&self.data, &id) else {
            return;
        };
        let name = match &item {
            ImportItem::Api { name, .. } | ImportItem::Folder { name, .. } => name.clone(),
        };
        let read = cyberapi_core::transfer::read_variables(&item);
        let store = AppStore::global(cx);
        let storage = store.storage().clone();
        let server = store.server().cloned();
        // The account's copy, as the server state knows; its base as the
        // copy store has it now — a sync in a workspace moves it on without
        // telling the server state (BUG-23).
        let copies = server.as_ref().and_then(|server| {
            let server = server.read(cx);
            server.mine(&target.id)?;
            server.copies().cloned()
        });
        let to = target.id.clone();
        let call: StoreFuture<(Vec<String>, Option<sync::CopyBinding>)> = Box::pin(async move {
            storage.tree().import(to.clone(), folder, vec![item]).await?;
            let mut unsynced = None;
            if let Some(copies) = copies {
                let bound = copies
                    .copies()
                    .await?
                    .into_iter()
                    .find(|binding| binding.collection == to);
                if let Some(binding) = bound {
                    let binding = sync::CopyBinding {
                        unsynced: true,
                        ..binding
                    };
                    copies.bind(binding.clone()).await?;
                    unsynced = Some(binding);
                }
            }
            // What they read and the project lacks, when that can be told:
            // the copy is made either way.
            let mut names = Vec::new();
            for category in [VariableCategory::Environment, VariableCategory::Custom] {
                match storage.variables().list(to.clone(), category).await {
                    Ok(variables) => names.extend(variables.into_iter().map(|variable| variable.name)),
                    Err(error) => {
                        warn!(%error, "the variables of a project copied into were not read");
                        return Ok((Vec::new(), unsynced));
                    }
                }
            }
            Ok((cyberapi_core::transfer::missing(&read, &names), unsynced))
        });
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok((missing, unsynced)) => {
                if let (Some(server), Some(binding)) = (&server, unsynced) {
                    server.update(cx, |server, cx| server.note_bound(vec![binding], cx));
                }
                if moving {
                    this.delete_told(id.clone(), false, cx);
                }
                cx.emit(WorkspaceEvent::Transferred {
                    name,
                    project: target.name.clone(),
                    moved: moving,
                    missing,
                });
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// F6: delete `count` undone: what it took put back as it was, into the
    /// project as it is now, written whole (`undo::put_back`). Only while
    /// it is the last delete, and not after [`Self::forget_undo`].
    pub fn undo_delete(&mut self, count: u64, cx: &mut Context<Self>) {
        if self.undo.as_ref().is_none_or(|undo| undo.count != count) || !self.editable() {
            return;
        }
        let Some(undo) = self.undo.take() else { return };
        // What the open APIs hold goes in first: the project is written
        // whole as it is read.
        for api in self.kept.values() {
            api.update(cx, |api, cx| api.save_now(cx));
        }
        let storage = self.sender.storage.clone();
        let collection = self.collection.id.clone();
        let deleted = undo.deleted.clone();
        // BUG-3: the reader's own again, or the next sync takes them up.
        let local = self
            .copy
            .as_ref()
            .filter(|_| !undo.local.is_empty())
            .map(|copy| copy.copy.put_back_local(undo.local.clone()));
        let call: StoreFuture<TreeData> = Box::pin(async move {
            let tree = storage.tree();
            // An open API's save landing between the read and the write
            // makes the write stale: read again, once (BUG-10).
            let mut tries = 2;
            loop {
                let at = tree.config(collection.clone()).await?;
                let mut config = at.config;
                // All there again — the server's taken, say: nothing to
                // write, and nothing to sync (BUG-24).
                if !undo::put_back(&mut config, &deleted) {
                    break;
                }
                match tree.replace(collection.clone(), config, Some(at.revision)).await {
                    Ok(_) => break,
                    Err(StoreError::Stale { .. }) if tries > 1 => tries -= 1,
                    Err(error) => return Err(error),
                }
            }
            if let Some(local) = local {
                local.await?;
            }
            tree.load(collection).await
        });
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(data) => {
                this.set_data(data, cx);
                for id in undo.pinned.iter().chain(&undo.expanded) {
                    let list = if undo.pinned.contains(id) {
                        &mut this.prefs.pinned
                    } else {
                        &mut this.prefs.expanded
                    };
                    if !list.contains(id) {
                        list.push(id.clone());
                    }
                }
                this.save_prefs(cx);
                cx.notify();
            }
            Err(error) => {
                // Not undone: still to be, while no delete came after it,
                // its notice back (BUG-10).
                if this.undo.is_none() {
                    let (name, apis, count) = (undo.deleted.name.clone(), undo.deleted.apis.len(), undo.count);
                    this.undo = Some(undo);
                    cx.emit(WorkspaceEvent::Deleted { name, apis, count });
                }
                cx.emit(WorkspaceEvent::Failed(error));
            }
        });
    }

    /// F6: delete `count` can no longer be undone.
    pub fn forget_undo(&mut self, count: u64) {
        if self.undo.as_ref().is_some_and(|undo| undo.count == count) {
            self.undo = None;
        }
    }

    /// The reader's state is saved in the background; a failure costs them
    /// the state next visit, not now, so it is only logged. Pins are written
    /// on this machine first. The document then goes out without them, so
    /// the browser build does not leave them on the server. When they could
    /// not be written here, they stay in the document rather than be lost.
    fn save_prefs(&self, cx: &mut Context<Self>) {
        let mut stored = self.prefs.clone();
        if crate::pins_local::set(&pin_scope(cx), &self.collection.id, &stored.pinned) {
            stored.pinned.clear();
        }
        let call = self
            .sender
            .storage
            .prefs()
            .set_workspace(self.collection.id.clone(), stored);
        run_serial(cx, &self.saves, call, |_, result, _| {
            if let Err(error) = result {
                warn!(%error, "the workspace state could not be saved");
            }
        });
    }
}

/// Whose pins. Tests build a workspace with no application installed; that
/// reader is the desktop's one reader.
/// D6: the projects whose sends in production the reader said not to be
/// asked about again, while the app runs.
#[derive(Default)]
struct Unasked(HashSet<cyberapi_core::CollectionId>);

impl gpui_kit::Global for Unasked {}

/// B7: the source an import keeps: the address it was fetched from, and
/// the folder it went into.
fn source_of(url: Option<String>, parent: &Option<ItemId>) -> Option<ImportSource> {
    url.map(|url| ImportSource {
        url,
        folder: parent.clone(),
    })
}

fn pin_scope(cx: &gpui_kit::App) -> String {
    let account = cx
        .try_global::<AppStore>()
        .and_then(|store| store.state().read(cx).me().map(|me| me.account.id.as_str().to_string()));
    crate::pins_local::scope(account.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::states::ApiEvent;
    use crate::states::api::SAVE_DELAY;
    use cyberapi_core::access::CollectionRole;
    use cyberapi_core::grpc::{Definitions, GrpcBody};
    use cyberapi_core::model::{HttpMethod, HttpRequest, KvParam};
    use cyberapi_core::prefs::{SendPrefs, Timeouts};
    use cyberapi_core::response::Exchange;
    use cyberapi_core::template::NoFiles;
    use cyberapi_core::tree::TreeError;
    use cyberapi_http::{HttpEngine, PreparedRequest, SendFuture};
    use cyberapi_store::{MemoryStore, NewCollection, NewVariable};
    use gpui_kit::{Entity, TestAppContext};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Answers every request with `200 ok`, or never.
    struct Engine {
        answers: bool,
    }

    impl HttpEngine for Engine {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            if self.answers {
                Box::pin(async {
                    Ok(Exchange {
                        status: 200,
                        body: b"ok".to_vec(),
                        ..Default::default()
                    })
                })
            } else {
                Box::pin(std::future::pending())
            }
        }
    }

    fn sender(storage: &Storage, answers: bool) -> Sender {
        Sender {
            storage: storage.clone(),
            engine: Arc::new(Engine { answers }),
            files: Arc::new(NoFiles("no files in tests")),
            env: Arc::new(cyberapi_core::template::NoEnv("no environment in tests")),
            user_agent: "CyberAPI/test".into(),
            timeouts: Timeouts::default(),
            sending: SendPrefs::default(),
            inherited: Default::default(),
            environment: None,
            tokens: Default::default(),
            secrets: Default::default(),
        }
    }

    async fn project(storage: &Storage) -> Collection {
        storage
            .collections()
            .create(NewCollection {
                name: "p".into(),
                ..Default::default()
            })
            .await
            .expect("create")
    }

    fn open(storage: &Storage, collection: Collection, cx: &mut TestAppContext) -> Entity<WorkspaceState> {
        let state = cx.new(|cx| WorkspaceState::new(sender(storage, true), collection, cx));
        cx.run_until_parked();
        state
    }

    fn row_names(state: &Entity<WorkspaceState>, cx: &mut TestAppContext) -> Vec<String> {
        state.read_with(cx, |state, _| {
            state
                .rows()
                .iter()
                .map(|row| {
                    let name = state
                        .tree()
                        .get(&row.id)
                        .map(|node| node.name().to_string())
                        .unwrap_or_default();
                    format!("{}{name}", "  ".repeat(row.depth))
                })
                .collect()
        })
    }

    fn find(state: &Entity<WorkspaceState>, name: &str, cx: &mut TestAppContext) -> ItemId {
        state.read_with(cx, |state, _| {
            state
                .tree()
                .roots()
                .flat_map(|root| std::iter::once(root.id().clone()).chain(state.tree().descendants(root.id())))
                .find(|id| state.tree().get(id).is_some_and(|node| node.name() == name))
                .unwrap_or_else(|| panic!("no item named {name}"))
        })
    }

    /// The global request headers are kept beside the environment, so a
    /// send knows the file an `openFile` in one asks for.
    #[gpui_kit::test]
    async fn the_global_headers_are_kept(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let variable = |category, name: &str, value: &str| NewVariable {
            collection: collection.id.clone(),
            category,
            name: name.into(),
            value: value.into(),
            enabled: true,
            environment: None,
        };
        for new in [
            variable(VariableCategory::Environment, "dev", "http://dev"),
            variable(VariableCategory::GlobalRequestHeader, "X-Sign", "{{openFile()}}"),
        ] {
            storage.variables().create(new).await.expect("create");
        }
        let state = open(&storage, collection.clone(), cx);
        let names = |state: &Entity<WorkspaceState>, cx: &mut TestAppContext| {
            state.read_with(cx, |state, _| {
                let names = |list: &[Variable]| list.iter().map(|v| v.name.clone()).collect::<Vec<_>>();
                let environment: Vec<Variable> = state.environments();
                (names(&environment), names(&state.global_headers()))
            })
        };
        assert_eq!(names(&state, cx), (vec!["dev".into()], vec!["X-Sign".into()]));
        let needs = state.read_with(cx, |state, _| {
            cyberapi_core::send::needs(&HttpRequest::default(), &state.global_headers(), &Default::default())
        });
        assert!(needs.picks_file);

        let new = variable(VariableCategory::GlobalRequestHeader, "X-Trace", "1");
        storage.variables().create(new).await.expect("create");
        state.update(cx, |state, cx| {
            state.refresh_variables(VariableCategory::GlobalRequestHeader, cx)
        });
        cx.run_until_parked();
        assert_eq!(names(&state, cx).1, ["X-Sign", "X-Trace"]);

        // #61: in a named environment its variables stand in for the shared
        // ones; the choice is the reader's, kept with their state.
        let staging = storage
            .variables()
            .create_environment(collection.id.clone(), "staging".into())
            .await
            .expect("an environment");
        let mut theirs = variable(VariableCategory::GlobalRequestHeader, "X-Trace", "staging");
        theirs.environment = Some(staging.id.clone());
        storage.variables().create(theirs).await.expect("create");
        state.update(cx, |state, cx| {
            state.refresh_variables(VariableCategory::GlobalRequestHeader, cx)
        });
        state.update(cx, |state, cx| state.refresh_environments(cx));
        cx.run_until_parked();
        let values = |state: &Entity<WorkspaceState>, cx: &mut TestAppContext| {
            state.read_with(cx, |state, _| {
                state
                    .global_headers()
                    .into_iter()
                    .map(|header| header.value)
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(
            values(&state, cx),
            ["{{openFile()}}", "1"],
            "none chosen: the shared ones"
        );
        state.update(cx, |state, cx| state.set_environment(Some(staging.id.clone()), cx));
        cx.run_until_parked();
        assert_eq!(values(&state, cx), ["{{openFile()}}", "staging"]);
        let kept = storage.prefs().workspace(collection.id.clone()).await.expect("prefs");
        assert_eq!(kept.environment, Some(staging.id));
    }

    /// M6: what `{{` offers, the custom variables kept up to date.
    #[gpui_kit::test]
    async fn the_template_hints_know_the_variables_and_the_apis(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let variable = |name: &str, enabled: bool| NewVariable {
            collection: collection.id.clone(),
            category: VariableCategory::Custom,
            name: name.into(),
            value: format!("{name}-value"),
            enabled,
            environment: None,
        };
        storage
            .variables()
            .create(variable("token", true))
            .await
            .expect("create");
        storage
            .variables()
            .create(variable("off", false))
            .await
            .expect("create");
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_folder(None, "users".into(), cx));
        cx.run_until_parked();
        let users = find(&state, "users", cx);
        state.update(cx, |state, cx| state.create_api(Some(users), "login".into(), cx));
        state.update(cx, |state, cx| state.create_api(None, "ping".into(), cx));
        cx.run_until_parked();
        let login = find(&state, "login", cx);

        let sources = state.read_with(cx, |state, _| state.hint_sources(true));
        assert_eq!(
            sources.custom,
            [("token".to_string(), "token-value".to_string())],
            "the enabled ones"
        );
        let apis: Vec<(&str, &str)> = sources
            .apis
            .iter()
            .map(|api| (api.name.as_str(), api.place.as_str()))
            .collect();
        assert_eq!(apis, [("login", "users"), ("ping", "")], "the one being edited last");
        assert_eq!(sources.apis[0].id, login.as_str());
        assert!(sources.machine);

        storage
            .variables()
            .create(variable("tenant", true))
            .await
            .expect("create");
        state.update(cx, |state, cx| state.refresh_variables(VariableCategory::Custom, cx));
        cx.run_until_parked();
        let names = state.read_with(cx, |state, _| state.hint_sources(false).custom.len());
        assert_eq!(names, 2);
    }

    /// M6: the last answer a `get` path's fields come from, read once and
    /// again only after the API answers anew.
    #[gpui_kit::test]
    async fn an_apis_last_answer_is_read_for_its_fields_until_it_answers_again(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_api(None, "login".into(), cx));
        cx.run_until_parked();
        let login = find(&state, "login", cx);
        let answer = |body: &str| cyberapi_core::response::HttpResponse {
            api: login.clone(),
            collection: collection.id.clone(),
            status: Some(200),
            body: body.as_bytes().to_vec(),
            ..Default::default()
        };
        storage
            .history()
            .add(answer(r#"{"token": "t-1"}"#), cyberapi_core::response::HISTORY_LIMIT)
            .await
            .expect("kept");

        let read = |state: &Entity<WorkspaceState>, cx: &mut TestAppContext| {
            state.update(cx, |state, cx| state.answer(login.clone(), cx))
        };
        let token = |value: &str| Some(Arc::new(serde_json::json!({ "token": value })));
        assert_eq!(read(&state, cx).await, token("t-1"));

        // Kept: a newer answer is not read until the API is said to have one.
        storage
            .history()
            .add(answer(r#"{"token": "t-2"}"#), cyberapi_core::response::HISTORY_LIMIT)
            .await
            .expect("kept");
        assert_eq!(read(&state, cx).await, token("t-1"));
        state.update(cx, |state, _| state.forget_answer(&login));
        assert_eq!(read(&state, cx).await, token("t-2"));

        // Not JSON: nothing to offer.
        storage
            .history()
            .add(answer("plain text"), cyberapi_core::response::HISTORY_LIMIT)
            .await
            .expect("kept");
        state.update(cx, |state, _| state.forget_answer(&login));
        assert_eq!(read(&state, cx).await, None);
    }

    #[gpui_kit::test]
    async fn a_new_api_is_selected_inside_its_opened_folder(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection, cx);
        state.update(cx, |state, cx| state.create_folder(None, "users".into(), cx));
        cx.run_until_parked();
        let users = find(&state, "users", cx);
        state.update(cx, |state, cx| {
            state.create_api(Some(users.clone()), "login".into(), cx)
        });
        cx.run_until_parked();

        assert_eq!(row_names(&state, cx), ["users", "  login"]);
        let login = find(&state, "login", cx);
        state.read_with(cx, |state, _| {
            assert_eq!(state.selected(), Some(&login));
            assert_eq!(state.opened_api().map(|api| api.name.as_str()), Some("login"));
        });
        // Its editor asks once whether it is new (the URI takes the keys).
        state.update(cx, |state, _| {
            assert!(state.take_created(&login));
            assert!(!state.take_created(&login));
        });
    }

    /// F10.5: a pasted cURL command is an API in its folder, opened.
    #[gpui_kit::test]
    async fn a_pasted_curl_command_is_opened_in_its_folder(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection, cx);
        state.update(cx, |state, cx| state.create_folder(None, "users".into(), cx));
        cx.run_until_parked();
        let users = find(&state, "users", cx);
        let bundle =
            cyberapi_core::import::curl_command("curl -X PUT 'https://a.test/v1/users/login?x=1'").expect("a command");
        state.update(cx, |state, cx| state.import(Some(users.clone()), bundle, cx));
        cx.run_until_parked();

        assert_eq!(row_names(&state, cx), ["users", "  login"]);
        let login = find(&state, "login", cx);
        state.read_with(cx, |state, _| {
            assert_eq!(state.selected(), Some(&login));
            assert_eq!(state.selected_folder(), Some(users.clone()), "the next goes beside it");
            let request = &state.opened_api().expect("opened").request;
            assert_eq!(
                (request.method, request.uri.as_str()),
                (HttpMethod::Put, "https://a.test/v1/users/login")
            );
        });
    }

    /// #65: a project made from an import takes it in once its tree has
    /// loaded — given before, it waits; given after, it goes in at once.
    #[gpui_kit::test]
    async fn an_import_given_while_loading_waits_for_the_tree(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let bundle = cyberapi_core::import::parse(
            r#"{"info": {"name": "Shop"}, "item": [{"name": "orders", "item": [
                {"name": "list", "request": {"method": "GET", "url": "https://a.test/orders"}}
            ]}]}"#,
        )
        .expect("a collection");
        let state = cx.new(|cx| WorkspaceState::new(sender(&storage, true), collection.clone(), cx));
        state.update(cx, |state, cx| {
            assert_ne!(state.load, Load::Loaded, "still loading");
            state.import_when_loaded(Arc::new(bundle.clone()), cx);
        });
        cx.run_until_parked();
        assert_eq!(row_names(&state, cx), ["orders"]);
        find(&state, "list", cx);

        let other = open(&storage, project(&storage).await, cx);
        other.update(cx, |state, cx| state.import_when_loaded(Arc::new(bundle), cx));
        cx.run_until_parked();
        assert_eq!(row_names(&other, cx), ["orders"]);
        find(&other, "list", cx);
        // Once: loading again reads what was written, not a second import.
        state.update(cx, |state, cx| state.reload(cx));
        cx.run_until_parked();
        assert_eq!(row_names(&state, cx), ["orders"]);
    }

    /// B4: an import's auth becomes the project's when it has none, and its
    /// named environments are made, or given what they lack.
    #[gpui_kit::test]
    async fn an_import_brings_its_auth_and_named_environments(cx: &mut TestAppContext) {
        use cyberapi_core::import::ImportEnvironment;
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let staging = storage
            .variables()
            .create_environment(collection.id.clone(), "Staging".into())
            .await
            .expect("an environment");
        storage
            .variables()
            .create(NewVariable {
                collection: collection.id.clone(),
                category: VariableCategory::Environment,
                name: "host".into(),
                value: "https://mine.test".into(),
                enabled: true,
                environment: Some(staging.id.clone()),
            })
            .await
            .expect("a variable");
        let state = open(&storage, collection.clone(), cx);
        cx.run_until_parked();
        let bundle = |auth| ImportBundle {
            auth,
            environments: vec![
                ImportEnvironment {
                    name: "staging".into(),
                    variables: vec![
                        ("host".into(), "https://theirs.test".into()),
                        ("token".into(), "t".into()),
                    ],
                },
                ImportEnvironment {
                    name: "dev".into(),
                    variables: vec![("host".into(), "http://localhost".into())],
                },
            ],
            ..Default::default()
        };
        state.update(cx, |state, cx| {
            state.import(
                None,
                bundle(Auth::Bearer {
                    token: "{{token}}".into(),
                }),
                cx,
            )
        });
        cx.run_until_parked();
        state.read_with(cx, |state, _| {
            assert_eq!(
                state.project_settings().auth,
                Auth::Bearer {
                    token: "{{token}}".into()
                }
            );
            let names: Vec<&str> = state.named_environments().iter().map(|e| e.name.as_str()).collect();
            assert_eq!(
                names,
                ["Staging", "dev"],
                "the one of the same name, in any case, is the project's"
            );
        });
        let variables = storage
            .variables()
            .list(collection.id.clone(), VariableCategory::Environment)
            .await
            .expect("variables");
        let in_staging: Vec<(&str, &str)> = variables
            .iter()
            .filter(|variable| variable.environment.as_ref() == Some(&staging.id))
            .map(|variable| (variable.name.as_str(), variable.value.as_str()))
            .collect();
        assert_eq!(
            in_staging,
            [("host", "https://mine.test"), ("token", "t")],
            "only what it lacks"
        );

        // The project's own auth stays.
        state.update(cx, |state, cx| {
            state.import(None, bundle(Auth::Bearer { token: "other".into() }), cx)
        });
        cx.run_until_parked();
        state.read_with(cx, |state, _| {
            assert_eq!(
                state.project_settings().auth,
                Auth::Bearer {
                    token: "{{token}}".into()
                }
            );
            assert_eq!(state.named_environments().len(), 2, "made once");
        });
    }

    /// B3: imported again, an API the project has is left or updated, not
    /// added twice; what is new goes into the folder of the same name; an
    /// environment variable it has is not added twice.
    #[gpui_kit::test]
    async fn an_import_again_adds_only_what_is_new(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        let parse = |text: &str| cyberapi_core::import::parse(text).expect("a collection");
        let first = parse(
            r#"{"info": {"name": "Shop"}, "variable": [{"key": "host", "value": "https://a.test"}],
                "item": [{"name": "orders", "item": [
                    {"name": "list", "request": {"method": "GET", "url": "{{host}}/orders"}}
                ]}]}"#,
        );
        let second = parse(
            r#"{"info": {"name": "Shop"}, "variable": [{"key": "host", "value": "https://a.test"}],
                "item": [{"name": "orders", "item": [
                    {"name": "list", "request": {"method": "GET", "url": "{{host}}/orders?page=1",
                        "header": [{"key": "X-New", "value": "1"}]}},
                    {"name": "make", "request": {"method": "POST", "url": "{{host}}/orders"}}
                ]}]}"#,
        );
        state.update(cx, |state, cx| state.import(None, first, cx));
        cx.run_until_parked();
        let apis = |cx: &mut TestAppContext| -> Vec<(String, String)> {
            state.read_with(cx, |state, _| {
                let mut apis: Vec<(String, String)> = state
                    .tree_data()
                    .apis
                    .iter()
                    .map(|api| (api.name.clone(), api.request.uri.clone()))
                    .collect();
                apis.sort();
                apis
            })
        };

        state.update(cx, |state, cx| state.import_again(None, second.clone(), false, cx));
        cx.run_until_parked();
        assert_eq!(
            apis(cx),
            [
                ("list".to_string(), "{{host}}/orders".to_string()),
                ("make".to_string(), "{{host}}/orders".to_string())
            ],
            "the one it had left as it was"
        );
        let orders = find(&state, "orders", cx);
        let in_orders = state.read_with(cx, |state, _| {
            state
                .tree_data()
                .folders
                .iter()
                .filter(|folder| folder.name == "orders")
                .map(|folder| folder.children.len())
                .collect::<Vec<_>>()
        });
        assert_eq!(in_orders, [2], "into the folder it had, not a second one");
        let hosts = storage
            .variables()
            .list(collection.id.clone(), VariableCategory::Environment)
            .await
            .expect("variables");
        assert_eq!(hosts.iter().filter(|variable| variable.name == "host").count(), 1);

        state.update(cx, |state, cx| state.import_again(Some(orders), second, true, cx));
        cx.run_until_parked();
        assert_eq!(apis(cx).len(), 2, "nothing added");
        // Updated, as written: the query the import splits off the address,
        // its header.
        let kept = storage.tree().load(collection.id.clone()).await.expect("the tree");
        let list = kept.apis.iter().find(|api| api.name == "list").expect("list");
        assert_eq!(list.request.query[0].key, "page");
        assert_eq!(list.request.headers[0].key, "X-New");
    }

    /// B7: an import fetched from an address keeps it as the project's
    /// source, with the folder it went into; syncing from it updates what
    /// the project has, adds what is new there, removes nothing; forgotten,
    /// the project keeps no source.
    #[gpui_kit::test]
    async fn a_project_syncs_from_the_address_it_was_imported_from(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_folder(None, "spec".into(), cx));
        cx.run_until_parked();
        let spec = find(&state, "spec", cx);
        let parse = |text: &str| cyberapi_core::import::parse(text).expect("a collection");
        let url = "https://api.test/openapi.json".to_string();
        let mut first = parse(
            r#"{"info": {"name": "Shop"}, "item": [
                {"name": "list", "request": {"method": "GET", "url": "{{host}}/orders"}},
                {"name": "drop", "request": {"method": "DELETE", "url": "{{host}}/orders/:id"}}
            ]}"#,
        );
        first.source = Some(url.clone());
        state.update(cx, |state, cx| state.import(Some(spec.clone()), first, cx));
        cx.run_until_parked();
        let kept = storage.tree().load(collection.id.clone()).await.expect("the tree");
        assert_eq!(
            kept.settings.source,
            Some(ImportSource {
                url: url.clone(),
                folder: Some(spec.clone()),
            }),
            "kept, with the folder it went into"
        );

        let second = parse(
            r#"{"info": {"name": "Shop"}, "item": [
                {"name": "list", "request": {"method": "GET", "url": "{{host}}/orders?page=1"}},
                {"name": "make", "request": {"method": "POST", "url": "{{host}}/orders"}}
            ]}"#,
        );
        state.update(cx, |state, cx| state.sync_from_source(url.clone(), second, cx));
        cx.run_until_parked();
        let kept = storage.tree().load(collection.id.clone()).await.expect("the tree");
        let mut apis: Vec<(&str, &str)> = kept
            .apis
            .iter()
            .map(|api| (api.name.as_str(), api.request.uri.as_str()))
            .collect();
        apis.sort();
        assert_eq!(
            apis,
            [
                ("drop", "{{host}}/orders/:id"),
                ("list", "{{host}}/orders"),
                ("make", "{{host}}/orders"),
            ],
            "updated (the query split off), added, the one the source lacks left"
        );
        let in_spec = kept.folders.iter().find(|folder| folder.id == spec).expect("spec");
        assert_eq!(in_spec.children.len(), 3, "what is new goes where the source's went");

        // Q-3: the same address imported again, into the top level: the
        // source keeps its folder.
        let mut again = parse(
            r#"{"info": {"name": "Shop"}, "item": [{"name": "list", "request": {"method": "GET", "url": "{{host}}/orders"}}]}"#,
        );
        again.source = Some(url.clone());
        state.update(cx, |state, cx| state.import(None, again, cx));
        cx.run_until_parked();
        let kept = storage.tree().load(collection.id.clone()).await.expect("the tree");
        assert_eq!(
            kept.settings.source.and_then(|source| source.folder),
            Some(spec.clone())
        );
        // Another address goes where it was imported.
        let mut other = parse(
            r#"{"info": {"name": "Shop"}, "item": [{"name": "list", "request": {"method": "GET", "url": "{{host}}/orders"}}]}"#,
        );
        other.source = Some("https://api.test/v2.json".into());
        state.update(cx, |state, cx| state.import(None, other, cx));
        cx.run_until_parked();
        let kept = storage.tree().load(collection.id.clone()).await.expect("the tree");
        assert_eq!(
            kept.settings.source,
            Some(ImportSource {
                url: "https://api.test/v2.json".into(),
                folder: None,
            })
        );

        state.update(cx, |state, cx| state.forget_source(cx));
        cx.run_until_parked();
        let kept = storage.tree().load(collection.id.clone()).await.expect("the tree");
        assert_eq!(kept.settings.source, None);
    }

    /// F6: a deleted folder comes back with its ids, its place, what its
    /// APIs held and the reader's pins; the history of what stayed stays.
    #[gpui_kit::test]
    async fn a_delete_is_undone(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let tree = storage.tree();
        let users = tree
            .create_folder(collection.id.clone(), None, "users".into())
            .await
            .expect("a folder")
            .item;
        let login = tree
            .create_api(collection.id.clone(), Some(users.id.clone()), "login".into())
            .await
            .expect("an API")
            .item;
        let request = HttpRequest {
            method: HttpMethod::Post,
            uri: "https://a.test/login".into(),
            description: "signs in".into(),
            ..Default::default()
        };
        tree.save_request(collection.id.clone(), login.id.clone(), request.clone())
            .await
            .expect("its request");
        let health = tree
            .create_api(collection.id.clone(), None, "health".into())
            .await
            .expect("an API")
            .item;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.set_pinned(&login.id, true, cx));
        cx.run_until_parked();
        let before = storage.tree().load(collection.id.clone()).await.expect("the tree");

        let counts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = counts.clone();
        cx.update(|cx| {
            cx.subscribe(&state, move |_, event: &WorkspaceEvent, _| {
                if let WorkspaceEvent::Deleted { count, apis, .. } = event {
                    seen.lock().expect("seen").push((*count, *apis));
                }
            })
            .detach()
        });
        state.update(cx, |state, cx| state.delete(users.id.clone(), cx));
        cx.run_until_parked();
        assert_eq!(*counts.lock().expect("seen"), [(1, 1)]);
        assert!(state.read_with(cx, |state, _| state.pinned().is_empty()));

        // Not one that is not the last.
        state.update(cx, |state, cx| state.undo_delete(7, cx));
        cx.run_until_parked();
        assert_eq!(
            storage
                .tree()
                .load(collection.id.clone())
                .await
                .expect("tree")
                .apis
                .len(),
            1
        );

        state.update(cx, |state, cx| state.undo_delete(1, cx));
        cx.run_until_parked();
        let after = storage.tree().load(collection.id.clone()).await.expect("the tree");
        assert_eq!(after.top_order, before.top_order);
        assert_eq!(after.folders.iter().map(|f| &f.id).collect::<Vec<_>>(), [&users.id]);
        let back = after.apis.iter().find(|api| api.id == login.id).expect("login is back");
        assert_eq!(back.request, request);
        assert!(after.apis.iter().any(|api| api.id == health.id));
        assert_eq!(
            state.read_with(cx, |state, _| state
                .pinned()
                .iter()
                .map(|api| api.id.clone())
                .collect::<Vec<_>>()),
            vec![login.id.clone()],
            "pinned again"
        );

        // Once only, and not after it is forgotten.
        state.update(cx, |state, cx| state.delete(health.id.clone(), cx));
        cx.run_until_parked();
        state.update(cx, |state, _| state.forget_undo(2));
        state.update(cx, |state, cx| state.undo_delete(2, cx));
        cx.run_until_parked();
        let last = storage.tree().load(collection.id.clone()).await.expect("the tree");
        assert!(!last.apis.iter().any(|api| api.id == health.id));
        assert_eq!(
            counts.lock().expect("seen").last(),
            Some(&(2, 1)),
            "a number of its own"
        );

        // BUG-9: an earlier delete's time running out leaves the last one
        // to undo.
        state.update(cx, |state, cx| state.delete(users.id.clone(), cx));
        cx.run_until_parked();
        assert_eq!(counts.lock().expect("seen").last(), Some(&(3, 1)));
        state.update(cx, |state, _| {
            state.forget_undo(1);
            state.forget_undo(2);
        });
        state.update(cx, |state, cx| state.undo_delete(3, cx));
        cx.run_until_parked();
        let last = storage.tree().load(collection.id.clone()).await.expect("the tree");
        assert!(last.folders.iter().any(|folder| folder.id == users.id), "back");

        // BUG-24: one back already, as the server's was taken, is not
        // written again.
        let whole = storage.tree().config(collection.id.clone()).await.expect("config");
        state.update(cx, |state, cx| state.delete(users.id.clone(), cx));
        cx.run_until_parked();
        let deleted = storage.tree().config(collection.id.clone()).await.expect("config");
        let revision = storage
            .tree()
            .replace(collection.id.clone(), whole.config.clone(), Some(deleted.revision))
            .await
            .expect("back as it was");
        state.update(cx, |state, cx| state.undo_delete(4, cx));
        cx.run_until_parked();
        let now = storage.tree().config(collection.id.clone()).await.expect("config");
        assert_eq!((now.revision, now.config), (revision, whole.config), "not written");
    }

    use cyberapi_core::CollectionId;
    use cyberapi_core::tree::TreeChange;
    use cyberapi_store::TreeStore;

    /// PERF-1: the rows are made again only when what they show changes —
    /// the tree, the keyword, the open folders — not each time asked.
    #[gpui_kit::test]
    async fn the_rows_are_made_once_for_what_they_show(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection, cx);
        state.update(cx, |state, cx| state.create_folder(None, "users".into(), cx));
        cx.run_until_parked();
        let users = find(&state, "users", cx);
        let rows = |cx: &mut TestAppContext| state.read_with(cx, |state, _| state.rows());
        let names = |rows: &[Row], cx: &mut TestAppContext| -> Vec<String> {
            state.read_with(cx, |state, _| {
                rows.iter()
                    .filter_map(|row| state.tree().get(&row.id).map(|node| node.name().to_string()))
                    .collect()
            })
        };
        let first = rows(cx);
        assert!(Rc::ptr_eq(&first, &rows(cx)), "the same rows, not made again");

        state.update(cx, |state, cx| {
            state.create_api(Some(users.clone()), "login".into(), cx)
        });
        cx.run_until_parked();
        let built = rows(cx);
        assert!(!Rc::ptr_eq(&first, &built), "the tree changed");
        assert!(state.read_with(cx, |state, _| state.prefs.expanded.contains(&users)));
        assert_eq!(names(&built, cx), ["users", "login"]);
        // The tree alone: the open folders are as they were.
        state.update(cx, |state, cx| state.create_api(None, "health".into(), cx));
        cx.run_until_parked();
        assert_eq!(names(&rows(cx), cx), ["health", "users", "login"]);

        state.update(cx, |state, cx| state.toggle_folder(&users, cx));
        let folded = rows(cx);
        assert_eq!(names(&folded, cx), ["health", "users"], "the folder closed");

        state.update(cx, |state, cx| state.set_keyword("log".into(), cx));
        let found = rows(cx);
        assert_eq!(names(&found, cx), ["users", "login"], "found inside");
        assert!(Rc::ptr_eq(&found, &rows(cx)));
    }

    /// A project's tree where, while `races` lasts, someone else's write
    /// lands right after each read of it whole.
    struct Racing {
        inner: MemoryStore,
        races: Arc<std::sync::atomic::AtomicU32>,
    }

    impl TreeStore for Racing {
        fn load(&self, collection: CollectionId) -> StoreFuture<TreeData> {
            self.inner.load(collection)
        }
        fn create_folder(
            &self,
            collection: CollectionId,
            parent: Option<ItemId>,
            name: String,
        ) -> StoreFuture<cyberapi_store::Inserted<Folder>> {
            self.inner.create_folder(collection, parent, name)
        }
        fn create_api(
            &self,
            collection: CollectionId,
            parent: Option<ItemId>,
            name: String,
        ) -> StoreFuture<cyberapi_store::Inserted<ApiSetting>> {
            self.inner.create_api(collection, parent, name)
        }
        fn rename(&self, collection: CollectionId, item: ItemId, name: String) -> StoreFuture<()> {
            self.inner.rename(collection, item, name)
        }
        fn save_request(&self, collection: CollectionId, api: ItemId, request: HttpRequest) -> StoreFuture<ApiSetting> {
            self.inner.save_request(collection, api, request)
        }
        fn set_folder_settings(
            &self,
            collection: CollectionId,
            folder: ItemId,
            settings: FolderSettings,
        ) -> StoreFuture<Folder> {
            self.inner.set_folder_settings(collection, folder, settings)
        }
        fn set_project_settings(&self, collection: CollectionId, settings: ProjectSettings) -> StoreFuture<()> {
            self.inner.set_project_settings(collection, settings)
        }
        fn move_item(&self, collection: CollectionId, item: ItemId, to: Placement) -> StoreFuture<TreeChange> {
            self.inner.move_item(collection, item, to)
        }
        fn import(
            &self,
            collection: CollectionId,
            parent: Option<ItemId>,
            items: Vec<ImportItem>,
        ) -> StoreFuture<cyberapi_store::Imported> {
            self.inner.import(collection, parent, items)
        }
        fn delete(&self, collection: CollectionId, item: ItemId) -> StoreFuture<cyberapi_core::tree::Removal> {
            self.inner.delete(collection, item)
        }
        fn config(&self, collection: CollectionId) -> StoreFuture<ConfigAt> {
            let (inner, races) = (self.inner.clone(), self.races.clone());
            Box::pin(async move {
                let at = inner.config(collection.clone()).await?;
                let raced = races
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(1))
                    .is_ok();
                if raced {
                    inner.create_folder(collection, None, "theirs".into()).await?;
                }
                Ok(at)
            })
        }
        fn replace(&self, collection: CollectionId, config: ProjectConfig, base: Option<u64>) -> StoreFuture<u64> {
            self.inner.replace(collection, config, base)
        }
    }

    /// BUG-10: an undo whose write a save beats is read again and made;
    /// one that cannot be made stays to undo, and says so again.
    #[gpui_kit::test]
    async fn an_undo_a_save_beats_is_tried_again(cx: &mut TestAppContext) {
        let inner = MemoryStore::new();
        let races = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let backend = Arc::new(inner.clone());
        let storage = Storage::new(
            backend.clone(),
            Arc::new(Racing {
                inner,
                races: races.clone(),
            }),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend,
        );
        let collection = project(&storage).await;
        let users = storage
            .tree()
            .create_folder(collection.id.clone(), None, "users".into())
            .await
            .expect("a folder")
            .item
            .id;
        let state = open(&storage, collection.clone(), cx);
        let told = Rc::new(RefCell::new(Vec::new()));
        let seen = told.clone();
        cx.update(|cx| {
            cx.subscribe(&state, move |_, event: &WorkspaceEvent, _| match event {
                WorkspaceEvent::Deleted { count, .. } => seen.borrow_mut().push(format!("deleted {count}")),
                WorkspaceEvent::Failed(_) => seen.borrow_mut().push("failed".into()),
                _ => {}
            })
            .detach()
        });
        let has_users = |cx: &mut TestAppContext| {
            state.read_with(cx, |state, _| {
                state.data.folders.iter().any(|folder| folder.id == users)
            })
        };

        state.update(cx, |state, cx| state.delete(users.clone(), cx));
        cx.run_until_parked();
        races.store(1, Ordering::Relaxed);
        state.update(cx, |state, cx| state.undo_delete(1, cx));
        cx.run_until_parked();
        assert!(has_users(cx), "read again, and undone");
        assert_eq!(*told.borrow(), ["deleted 1"]);

        // Beaten every time: not undone, and it can still be.
        state.update(cx, |state, cx| state.delete(users.clone(), cx));
        cx.run_until_parked();
        races.store(2, Ordering::Relaxed);
        state.update(cx, |state, cx| state.undo_delete(2, cx));
        cx.run_until_parked();
        assert!(!has_users(cx));
        assert_eq!(*told.borrow(), ["deleted 1", "deleted 2", "deleted 2", "failed"]);
        state.update(cx, |state, cx| state.undo_delete(2, cx));
        cx.run_until_parked();
        assert!(has_users(cx), "undone the second time asked");
    }

    /// D6: an environment marked as production is the project's to keep;
    /// in it, what may change something is asked about, until the reader
    /// says to stop asking in this project.
    #[gpui_kit::test]
    async fn sends_in_production_are_asked_about(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        let prod = storage
            .variables()
            .create_environment(collection.id.clone(), "prod".into())
            .await
            .expect("an environment");
        state.update(cx, |state, cx| state.refresh_environments(cx));
        cx.run_until_parked();
        let post = HttpRequest {
            method: HttpMethod::Post,
            ..Default::default()
        };
        let get = HttpRequest::default();
        let asks = |request: &HttpRequest, cx: &mut TestAppContext| {
            state.read_with(cx, |state, cx| state.asks_before(request, cx))
        };
        state.update(cx, |state, cx| state.set_production(prod.id.clone(), true, cx));
        cx.run_until_parked();
        assert!(!asks(&post, cx), "not while in no environment");

        // BUG-19: a change made before the last is written keeps it.
        let token = storage
            .variables()
            .create(NewVariable {
                collection: collection.id.clone(),
                category: VariableCategory::Environment,
                name: "TOKEN".into(),
                value: "t".into(),
                enabled: true,
                environment: None,
            })
            .await
            .expect("a variable");
        let qa = storage
            .variables()
            .create_environment(collection.id.clone(), "qa".into())
            .await
            .expect("an environment");
        state.update(cx, |state, cx| {
            state.set_production(qa.id.clone(), true, cx);
            state.set_secret(&token, true, cx);
        });
        cx.run_until_parked();
        let kept = storage.tree().load(collection.id.clone()).await.expect("load").settings;
        assert_eq!(kept.protected_environments, [prod.id.clone(), qa.id.clone()]);
        assert_eq!(kept.secret_variables, std::slice::from_ref(&token.id));
        state.update(cx, |state, cx| state.set_production(qa.id.clone(), false, cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.set_environment(Some(prod.id.clone()), cx));
        cx.run_until_parked();
        assert!(state.read_with(cx, |state, _| state.in_production()));
        assert!(asks(&post, cx));
        assert!(!asks(&get, cx), "a GET goes");
        let kept = storage.tree().load(collection.id.clone()).await.expect("the tree");
        assert_eq!(
            kept.settings.protected_environments,
            vec![prod.id.clone()],
            "the project's"
        );

        state.update(cx, |state, cx| state.stop_asking(cx));
        assert!(!asks(&post, cx), "not again while the app runs");

        state.update(cx, |state, cx| state.set_production(prod.id.clone(), false, cx));
        cx.run_until_parked();
        let kept = storage.tree().load(collection.id.clone()).await.expect("the tree");
        assert!(kept.settings.protected_environments.is_empty());
    }

    /// Each change saves the whole state; the last one made is the one kept,
    /// however the saves are scheduled.
    #[gpui_kit::test]
    async fn the_last_state_is_the_one_saved(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_folder(None, "users".into(), cx));
        cx.run_until_parked();
        let users = find(&state, "users", cx);
        state.update(cx, |state, cx| {
            for _ in 0..9 {
                state.toggle_folder(&users, cx);
            }
        });
        cx.run_until_parked();
        let kept = storage.prefs().workspace(collection.id.clone()).await.expect("prefs");
        assert_eq!(kept, state.read_with(cx, |state, _| state.prefs.clone()));
    }

    #[gpui_kit::test]
    async fn the_readers_state_survives_a_reopening(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_folder(None, "users".into(), cx));
        cx.run_until_parked();
        let users = find(&state, "users", cx);
        state.update(cx, |state, cx| {
            state.create_api(Some(users.clone()), "login".into(), cx)
        });
        cx.run_until_parked();
        let login = find(&state, "login", cx);
        state.update(cx, |state, cx| state.set_pinned(&login, true, cx));
        cx.run_until_parked();
        let kept = storage.prefs().workspace(collection.id.clone()).await.expect("prefs");
        assert!(kept.pinned.is_empty(), "pins stay on this machine");
        assert_eq!(crate::pins_local::get("", &collection.id), Some(vec![login.clone()]));

        let again = open(&storage, collection, cx);
        assert_eq!(row_names(&again, cx), ["users", "  login"], "the folder stays open");
        again.read_with(cx, |state, _| {
            assert_eq!(state.selected(), Some(&login));
            assert_eq!(state.pinned().len(), 1);
        });

        again.update(cx, |state, cx| state.collapse_all(cx));
        cx.run_until_parked();
        assert_eq!(row_names(&again, cx), ["users"]);

        // Chosen in the tree, the API shows again.
        again.update(cx, |state, cx| state.select(&login, cx));
        assert_eq!(row_names(&again, cx), ["users", "  login"]);
    }

    /// F12.2: a pin opens its API in the editor and leaves the tree as it
    /// was, its selection and its folders. The tree's row, chosen again,
    /// takes the editor back; so does closing the pin.
    #[gpui_kit::test]
    async fn a_pin_opens_its_api_and_leaves_the_tree_alone(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection, cx);
        state.update(cx, |state, cx| state.create_folder(None, "users".into(), cx));
        cx.run_until_parked();
        let users = find(&state, "users", cx);
        state.update(cx, |state, cx| state.create_api(Some(users), "login".into(), cx));
        cx.run_until_parked();
        let login = find(&state, "login", cx);
        state.update(cx, |state, cx| state.create_api(None, "ping".into(), cx));
        cx.run_until_parked();
        let ping = find(&state, "ping", cx);
        state.update(cx, |state, cx| {
            state.set_pinned(&login, true, cx);
            state.collapse_all(cx);
        });
        cx.run_until_parked();
        let editing = |state: &Entity<WorkspaceState>, cx: &mut TestAppContext| {
            state.read_with(cx, |state, cx| state.api().map(|api| api.read(cx).api().clone()))
        };

        state.update(cx, |state, cx| state.open_pin(&login, cx));
        state.read_with(cx, |state, _| {
            assert_eq!(state.selected(), Some(&ping), "the tree keeps its row");
            assert_eq!(state.opened(), Some(&login));
        });
        assert_eq!(editing(&state, cx), Some(login.clone()));
        assert_eq!(row_names(&state, cx), ["ping", "users"], "its folder stays shut");

        // The tree's row takes the editor back.
        state.update(cx, |state, cx| state.select(&ping, cx));
        assert_eq!(editing(&state, cx), Some(ping.clone()));

        // So does closing the pin that is open.
        state.update(cx, |state, cx| state.open_pin(&login, cx));
        assert_eq!(editing(&state, cx), Some(login.clone()));
        state.update(cx, |state, cx| state.set_pinned(&login, false, cx));
        assert_eq!(editing(&state, cx), Some(ping));
    }

    /// F1: each API opened is a tab beside the one before; a tab shown
    /// again has its editor as it was; closing opens the one beside it; the
    /// tabs come back with the project, and those of APIs deleted go.
    #[gpui_kit::test]
    async fn opened_apis_stay_open_as_tabs(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        for name in ["a", "b", "c"] {
            state.update(cx, |state, cx| state.create_api(None, name.into(), cx));
            cx.run_until_parked();
        }
        let (a, b, c) = (find(&state, "a", cx), find(&state, "b", cx), find(&state, "c", cx));
        let tabs = |state: &Entity<WorkspaceState>, cx: &mut TestAppContext| -> Vec<String> {
            state.read_with(cx, |state, _| state.tabs().iter().map(|api| api.name.clone()).collect())
        };
        let opened = |state: &Entity<WorkspaceState>, cx: &mut TestAppContext| {
            state.read_with(cx, |state, _| state.opened().cloned())
        };
        assert_eq!(tabs(&state, cx), ["a", "b", "c"]);

        // Shown again: the same editor, its edits and all.
        state.update(cx, |state, cx| state.select(&a, cx));
        let editor_a = editor(&state, cx);
        editor_a.update(cx, |api, cx| api.set_uri("http://a.test/".into(), cx));
        state.update(cx, |state, cx| state.select(&b, cx));
        state.update(cx, |state, cx| state.select(&a, cx));
        assert_eq!(editor(&state, cx).entity_id(), editor_a.entity_id());

        // A new one goes beside the one open.
        state.update(cx, |state, cx| state.create_api(None, "d".into(), cx));
        cx.run_until_parked();
        let d = find(&state, "d", cx);
        assert_eq!(tabs(&state, cx), ["a", "d", "b", "c"]);

        // Next and previous, round the ends.
        state.update(cx, |state, cx| state.step_tab(2, cx));
        assert_eq!(opened(&state, cx), Some(c.clone()));
        state.update(cx, |state, cx| state.step_tab(1, cx));
        assert_eq!(opened(&state, cx), Some(a.clone()));

        // Closing the open one opens the one after it, else the one before.
        state.update(cx, |state, cx| state.close_tab(&a, cx));
        assert_eq!(
            (tabs(&state, cx), opened(&state, cx)),
            (vec!["d".into(), "b".into(), "c".into()], Some(d.clone()))
        );
        state.update(cx, |state, cx| state.close_tab(&c, cx));
        assert_eq!(opened(&state, cx), Some(d.clone()), "not the open one");
        state.update(cx, |state, cx| state.select(&b, cx));
        state.update(cx, |state, cx| state.close_tab(&b, cx));
        assert_eq!(
            (tabs(&state, cx), opened(&state, cx)),
            (vec!["d".to_string()], Some(d.clone()))
        );
        // What its editor held is saved as it closes.
        cx.run_until_parked();
        assert_eq!(stored(&storage, &collection, &a).uri, "http://a.test/");

        // They come back with the project.
        let again = open(&storage, collection.clone(), cx);
        assert_eq!(tabs(&again, cx), ["d"]);
        assert_eq!(opened(&again, cx), Some(d.clone()));

        // The others, or all.
        state.update(cx, |state, cx| state.select(&a, cx));
        state.update(cx, |state, cx| state.select(&c, cx));
        assert_eq!(tabs(&state, cx), ["d", "a", "c"]);
        state.update(cx, |state, cx| state.close_other_tabs(&a, cx));
        assert_eq!(
            (tabs(&state, cx), opened(&state, cx)),
            (vec!["a".to_string()], Some(a.clone()))
        );
        state.update(cx, |state, cx| state.close_all_tabs(cx));
        assert!(tabs(&state, cx).is_empty());
        assert!(state.read_with(cx, |state, _| state.api().is_none()), "nothing open");

        // A deleted API's tab goes, and the one beside it opens.
        state.update(cx, |state, cx| state.select(&b, cx));
        state.update(cx, |state, cx| state.select(&c, cx));
        state.update(cx, |state, cx| state.delete(c.clone(), cx));
        cx.run_until_parked();
        assert_eq!((tabs(&state, cx), opened(&state, cx)), (vec!["b".to_string()], Some(b)));
    }

    /// F7: a tab goes into a window of its own with its editor as it was,
    /// is told so, and comes back; closing it ends that.
    #[gpui_kit::test]
    async fn a_tab_goes_apart_and_comes_back(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        for name in ["a", "b"] {
            state.update(cx, |state, cx| state.create_api(None, name.into(), cx));
            cx.run_until_parked();
        }
        let (a, b) = (find(&state, "a", cx), find(&state, "b", cx));
        let parted = Rc::new(RefCell::new(Vec::new()));
        let told = parted.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&state, move |_, event: &WorkspaceEvent, _| {
                if let WorkspaceEvent::Parted(id) = event {
                    told.borrow_mut().push(id.clone());
                }
            })
        });
        state.update(cx, |state, cx| state.select(&a, cx));
        let editor_a = editor(&state, cx);
        // B is open but not shown yet: shown so, it has an editor to take.
        state.update(cx, |state, cx| state.part(&b, cx));
        state.update(cx, |state, cx| state.part(&a, cx));
        cx.run_until_parked();
        assert_eq!(*parted.borrow(), [b.clone(), a.clone()]);
        state.read_with(cx, |state, _| {
            assert!(state.is_apart(&a) && state.is_apart(&b));
            assert_eq!(
                state.kept_api(&a).map(|api| api.entity_id()),
                Some(editor_a.entity_id()),
                "the editor it had"
            );
        });
        state.update(cx, |state, cx| state.join(&a, cx));
        state.update(cx, |state, cx| state.close_tab(&b, cx));
        state.read_with(cx, |state, _| {
            assert!(!state.is_apart(&a), "back");
            assert!(!state.is_apart(&b) && state.kept_api(&b).is_none(), "closed");
        });
        // Not an open tab: nothing.
        state.update(cx, |state, cx| state.part(&b, cx));
        assert_eq!(parted.borrow().len(), 2);
    }

    /// F1: past `MAX_TABS`, the tab shown longest ago closes.
    #[gpui_kit::test]
    async fn past_the_limit_the_tab_shown_longest_ago_closes(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection, cx);
        for at in 0..MAX_TABS {
            state.update(cx, |state, cx| state.create_api(None, format!("api {at}"), cx));
            cx.run_until_parked();
        }
        // The first shown again: the second is the one shown longest ago.
        let first = find(&state, "api 0", cx);
        state.update(cx, |state, cx| state.select(&first, cx));
        state.update(cx, |state, cx| state.create_api(None, "one more".into(), cx));
        cx.run_until_parked();
        let names: Vec<String> =
            state.read_with(cx, |state, _| state.tabs().iter().map(|api| api.name.clone()).collect());
        assert_eq!(names.len(), MAX_TABS);
        assert!(!names.contains(&"api 1".to_string()), "{names:?}");
        assert!(names.contains(&"api 0".to_string()) && names.contains(&"one more".to_string()));
    }

    /// A document that still carries pins (the old place) is copied onto
    /// this machine and then stored without them. An empty list here is the
    /// reader's, and is not filled back in from that document.
    #[gpui_kit::test]
    async fn pins_in_the_document_move_onto_this_machine(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let api = storage
            .tree()
            .create_api(collection.id.clone(), None, "login".into())
            .await
            .expect("an API")
            .item
            .id;
        storage
            .prefs()
            .set_workspace(
                collection.id.clone(),
                WorkspacePrefs {
                    pinned: vec![api.clone()],
                    ..Default::default()
                },
            )
            .await
            .expect("prefs");

        let state = open(&storage, collection.clone(), cx);
        state.read_with(cx, |state, _| assert_eq!(state.pinned().len(), 1));
        let kept = storage.prefs().workspace(collection.id.clone()).await.expect("prefs");
        assert!(kept.pinned.is_empty());
        assert_eq!(crate::pins_local::get("", &collection.id), Some(vec![api.clone()]));

        state.update(cx, |state, cx| state.set_pinned(&api, false, cx));
        cx.run_until_parked();
        storage
            .prefs()
            .set_workspace(
                collection.id.clone(),
                WorkspacePrefs {
                    pinned: vec![api.clone()],
                    ..Default::default()
                },
            )
            .await
            .expect("the old pins");
        let again = open(&storage, collection, cx);
        again.read_with(cx, |state, _| assert!(state.pinned().is_empty()));
    }

    #[gpui_kit::test]
    async fn moves_and_keywords_follow_the_core_rules(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection, cx);
        for name in ["a", "b"] {
            state.update(cx, |state, cx| state.create_folder(None, name.into(), cx));
        }
        state.update(cx, |state, cx| state.create_api(None, "登录".into(), cx));
        cx.run_until_parked();
        // New items go first on the top level.
        assert_eq!(row_names(&state, cx), ["登录", "b", "a"]);

        let (a, login) = (find(&state, "a", cx), find(&state, "登录", cx));
        state.update(cx, |state, cx| {
            state.move_item(
                login.clone(),
                Placement {
                    parent: Some(a.clone()),
                    before: None,
                },
                cx,
            )
        });
        cx.run_until_parked();
        assert_eq!(row_names(&state, cx), ["b", "a", "  登录"], "the target folder opens");

        state.update(cx, |state, cx| state.set_keyword("dl".into(), cx));
        assert_eq!(row_names(&state, cx), ["a", "  登录"]);

        // A refused move changes nothing and says why.
        let failures: Rc<RefCell<Vec<StoreError>>> = Rc::default();
        let seen = failures.clone();
        cx.update(|cx| {
            cx.subscribe(&state, move |_, event, _| {
                if let WorkspaceEvent::Failed(error) = event {
                    seen.borrow_mut().push(error.clone());
                }
            })
            .detach();
        });
        state.update(cx, |state, cx| state.set_keyword(String::new(), cx));
        state.update(cx, |state, cx| {
            state.move_item(
                a.clone(),
                Placement {
                    parent: Some(a.clone()),
                    before: None,
                },
                cx,
            )
        });
        cx.run_until_parked();
        assert_eq!(row_names(&state, cx), ["b", "a", "  登录"]);
        assert_eq!(
            failures.borrow().as_slice(),
            [StoreError::Tree {
                source: TreeError::Cycle { id: a }
            }]
        );
    }

    #[gpui_kit::test]
    async fn other_readers_changes_show_on_the_next_beat(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let every = Duration::from_secs(10);
        let state =
            cx.new(|cx| WorkspaceState::with_poll(sender(&storage, true), collection.clone(), None, Some(every), cx));
        cx.run_until_parked();
        assert!(row_names(&state, cx).is_empty());

        // Someone else, straight through the store.
        storage
            .tree()
            .create_api(collection.id.clone(), None, "theirs".into())
            .await
            .expect("create");
        cx.run_until_parked();
        assert!(row_names(&state, cx).is_empty(), "not before the beat");

        cx.executor().advance_clock(every);
        cx.run_until_parked();
        assert_eq!(row_names(&state, cx), ["theirs"]);
    }

    fn editor(state: &Entity<WorkspaceState>, cx: &mut TestAppContext) -> Entity<ApiState> {
        state.read_with(cx, |state, _| state.api().cloned().expect("an editor"))
    }

    fn stored(storage: &Storage, collection: &Collection, api: &ItemId) -> HttpRequest {
        let data = pollster::block_on(storage.tree().load(collection.id.clone())).expect("load");
        data.apis
            .into_iter()
            .find(|found| &found.id == api)
            .expect("the API")
            .request
    }

    fn header(key: &str, value: &str) -> KvParam {
        KvParam {
            key: key.into(),
            value: value.into(),
            enabled: true,
            ..Default::default()
        }
    }

    #[gpui_kit::test]
    async fn the_editor_saves_at_once_or_after_a_pause(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_api(None, "login".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        let id = api.read_with(cx, |api, _| api.api().clone());

        // F4.2: the method at once, and the tree's label follows.
        api.update(cx, |api, cx| api.set_method(HttpMethod::Post, cx));
        cx.run_until_parked();
        assert_eq!(stored(&storage, &collection, &id).method, HttpMethod::Post);
        state.read_with(cx, |state, _| {
            assert_eq!(state.tree().get(&id).map(|node| node.method()), Some(HttpMethod::Post));
        });

        // F4.11: typing waits; an empty row is not kept (F4.10).
        api.update(cx, |api, cx| {
            api.set_headers(vec![header("X-A", "1"), header(" ", "")], cx)
        });
        cx.run_until_parked();
        assert!(stored(&storage, &collection, &id).headers.is_empty(), "not yet");
        cx.executor().advance_clock(SAVE_DELAY);
        cx.run_until_parked();
        assert_eq!(stored(&storage, &collection, &id).headers, [header("X-A", "1")]);

        // Leaving the API saves what is still waiting.
        api.update(cx, |api, cx| api.set_query(vec![header("q", "x")], cx));
        drop(api);
        state.update(cx, |state, cx| state.create_api(None, "other".into(), cx));
        cx.run_until_parked();
        assert_eq!(stored(&storage, &collection, &id).query, [header("q", "x")]);
        let other = editor(&state, cx);
        assert_ne!(other.read_with(cx, |api, _| api.api().clone()), id);
    }

    /// F1: a scratch request is a tab and not a row of the tree. It uses the
    /// reader's prefs, so it is still there on the next visit, and closing
    /// its tab drops it. Saving it into a folder makes it an API.
    #[gpui_kit::test]
    async fn a_scratch_request_stays_until_it_is_saved_or_closed(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);

        state.update(cx, |state, cx| state.new_scratch("Scratch".into(), cx));
        cx.run_until_parked();
        let id = state.read_with(cx, |state, _| {
            let tabs = state.tabs();
            assert_eq!(tabs.len(), 1);
            assert!(tabs[0].scratch);
            assert!(state.tree().get(&tabs[0].id).is_none());
            tabs[0].id.clone()
        });
        assert!(row_names(&state, cx).is_empty(), "a scratch is not in the tree");

        let api = editor(&state, cx);
        api.update(cx, |api, cx| api.set_uri("http://scratch.test/one".into(), cx));
        // Still waiting: leaving the tab is what keeps it.
        api.update(cx, |api, cx| api.set_body("noted".into(), cx));
        drop(api);
        state.update(cx, |state, cx| state.create_api(None, "ping".into(), cx));
        cx.run_until_parked();
        state.read_with(cx, |state, _| {
            let scratch = state.scratch(&id).expect("the tab is still open");
            assert_eq!(scratch.request.uri, "http://scratch.test/one");
            assert_eq!(scratch.request.body, "noted");
            assert!(state.tabs().iter().any(|tab| tab.scratch && tab.id == id));
        });
        assert_eq!(row_names(&state, cx), ["ping"]);

        drop(state);
        let again = open(&storage, collection.clone(), cx);
        again.read_with(cx, |state, _| {
            let scratch = state.scratch(&id).expect("it survives the visit");
            assert_eq!(scratch.request.uri, "http://scratch.test/one");
            assert_eq!(scratch.request.body, "noted");
            assert!(state.tabs().iter().any(|tab| tab.id == id && tab.scratch));
        });
        again.update(cx, |state, cx| state.close_tab(&id, cx));
        cx.run_until_parked();
        drop(again);
        let closed = open(&storage, collection.clone(), cx);
        closed.read_with(cx, |state, _| {
            assert!(state.scratch(&id).is_none(), "closing the tab drops it");
            assert!(state.tabs().iter().all(|tab| tab.id != id));
        });

        closed.update(cx, |state, cx| state.create_folder(None, "users".into(), cx));
        cx.run_until_parked();
        let folder = find(&closed, "users", cx);
        closed.update(cx, |state, cx| state.new_scratch("Scratch".into(), cx));
        cx.run_until_parked();
        let scratch = editor(&closed, cx);
        let scratch_id = scratch.read_with(cx, |api, _| api.api().clone());
        scratch.update(cx, |api, cx| api.set_uri("http://scratch.test/two".into(), cx));
        drop(scratch);
        closed.update(cx, |state, cx| {
            state.save_scratch_into(scratch_id.clone(), Some(folder.clone()), cx)
        });
        cx.run_until_parked();

        let saved = find(&closed, "Scratch", cx);
        assert_eq!(stored(&storage, &collection, &saved).uri, "http://scratch.test/two");
        closed.read_with(cx, |state, _| {
            assert_eq!(state.tree().get(&saved).and_then(|node| node.parent()), Some(&folder));
            assert!(state.scratch(&scratch_id).is_none());
            assert!(state.tabs().iter().any(|tab| tab.id == saved && !tab.scratch));
            assert!(state.tabs().iter().all(|tab| tab.id != scratch_id));
        });
    }

    /// A pasted cURL command replaces the request whole, saved at once, and
    /// the tree's label follows its method.
    #[gpui_kit::test]
    async fn a_replaced_request_is_saved_at_once(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_api(None, "login".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        let id = api.read_with(cx, |api, _| api.api().clone());
        let replaced = Rc::new(RefCell::new(0));
        let counted = replaced.clone();
        let _events = cx.update(|cx| {
            cx.subscribe(&api, move |_, event, _| {
                if matches!(event, ApiEvent::RequestReplaced) {
                    *counted.borrow_mut() += 1;
                }
            })
        });

        let request = HttpRequest {
            method: HttpMethod::Delete,
            uri: "https://a.test/x".into(),
            headers: vec![header("X-A", "1")],
            ..Default::default()
        };
        api.update(cx, |api, cx| api.replace_request(request.clone(), cx));
        cx.run_until_parked();
        assert_eq!(stored(&storage, &collection, &id), request);
        assert_eq!(*replaced.borrow(), 1);
        state.read_with(cx, |state, _| {
            assert_eq!(
                state.tree().get(&id).map(|node| node.method()),
                Some(HttpMethod::Delete)
            );
        });

        // The same request again changes nothing.
        api.update(cx, |api, cx| api.replace_request(request, cx));
        cx.run_until_parked();
        assert_eq!(*replaced.borrow(), 1);
    }

    /// A copy sits right after its API, with its request, selected.
    #[gpui_kit::test]
    async fn a_copy_goes_beside_its_api(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        for name in ["a", "b"] {
            state.update(cx, |state, cx| state.create_api(None, name.into(), cx));
            cx.run_until_parked();
        }
        let a = find(&state, "a", cx);
        state.update(cx, |state, cx| state.select(&a, cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        api.update(cx, |api, cx| api.set_method(HttpMethod::Post, cx));
        cx.run_until_parked();
        // Typed, and not saved yet: the copy has it all the same.
        api.update(cx, |api, cx| api.set_headers(vec![header("X-A", "1")], cx));

        // A new API goes first on the top level: "b" is above "a".
        assert_eq!(row_names(&state, cx), ["b", "a"]);

        // The last item's copy is imported where it belongs, last.
        state.update(cx, |state, cx| state.duplicate(&a, "a copy".into(), cx));
        cx.run_until_parked();
        assert_eq!(row_names(&state, cx), ["b", "a", "a copy"]);
        let copy = find(&state, "a copy", cx);
        assert_eq!(stored(&storage, &collection, &copy).method, HttpMethod::Post);
        assert_eq!(stored(&storage, &collection, &copy).headers, [header("X-A", "1")]);
        state.read_with(cx, |state, _| assert_eq!(state.selected(), Some(&copy)));

        // Any other's is moved up beside it.
        let b = find(&state, "b", cx);
        state.update(cx, |state, cx| state.duplicate(&b, "b copy".into(), cx));
        cx.run_until_parked();
        assert_eq!(row_names(&state, cx), ["b", "b copy", "a", "a copy"]);
    }

    /// §9.2: a viewer edits and sends, but nothing it changes is saved.
    #[gpui_kit::test]
    async fn a_viewers_edits_stay_its_own(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let manager = open(&storage, collection.clone(), cx);
        manager.update(cx, |state, cx| state.create_api(None, "login".into(), cx));
        cx.run_until_parked();
        let id = editor(&manager, cx).read_with(cx, |api, _| api.api().clone());

        let viewer = open(
            &storage,
            Collection {
                role: CollectionRole::Viewer,
                ..collection.clone()
            },
            cx,
        );
        assert!(!viewer.read_with(cx, |state, _| state.editable()));
        viewer.update(cx, |state, cx| state.select(&id, cx));
        cx.run_until_parked();
        let api = editor(&viewer, cx);
        api.update(cx, |api, cx| {
            api.set_method(HttpMethod::Post, cx);
            api.set_uri("http://viewer.test".into(), cx);
        });
        cx.executor().advance_clock(SAVE_DELAY);
        cx.run_until_parked();
        drop(api);
        drop(viewer);
        cx.run_until_parked();
        let request = stored(&storage, &collection, &id);
        assert_eq!((request.method, request.uri.as_str()), (HttpMethod::Get, ""));
    }

    /// #50: a URI that starts with an environment variable nobody enabled
    /// is said as that, by its name, not as the URL it would leave.
    #[gpui_kit::test]
    async fn an_unknown_environment_variable_is_named(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = cx.new(|cx| WorkspaceState::new(sender(&storage, true), collection.clone(), cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.create_api(None, "ping".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        let heard = Rc::new(RefCell::new(Vec::new()));
        let into = heard.clone();
        cx.update(|cx| {
            cx.subscribe(&api, move |_, event, _| match event {
                ApiEvent::NoEnvironment { name, .. } => into.borrow_mut().push(format!("no {name}")),
                ApiEvent::NotSent(message) => into.borrow_mut().push(message.clone()),
                _ => {}
            })
            .detach();
        });
        api.update(cx, |api, cx| api.set_uri("{{host}}/echo".into(), cx));
        api.update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
        cx.run_until_parked();
        assert_eq!(heard.borrow().as_slice(), ["no host"]);
        assert!(api.read_with(cx, |api, _| api.responses().is_empty()), "nothing sent");
    }

    /// D1: what was typed for a prompt is kept while the app runs, for its
    /// project alone, and the request's code takes it.
    #[gpui_kit::test]
    async fn what_was_typed_for_a_prompt_is_kept_for_its_project(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = cx.new(|cx| WorkspaceState::new(sender(&storage, true), collection.clone(), cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.create_api(None, "verify".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        api.update(cx, |api, cx| api.set_uri("http://example.test/".into(), cx));
        let query = vec![KvParam {
            key: "code".into(),
            value: "{{prompt(code)}}".into(),
            enabled: true,
            ..Default::default()
        }];
        api.update(cx, |api, cx| api.set_query(query, cx));
        let answers = Answers {
            file: None,
            prompts: HashMap::from([("code".to_string(), "123456".to_string())]),
        };
        api.update(cx, |api, cx| api.send_or_cancel(answers, cx));
        cx.run_until_parked();
        assert_eq!(
            api.read_with(cx, |api, cx| api.typed(cx).prompts),
            HashMap::from([("code".to_string(), "123456".to_string())])
        );
        let elsewhere = cyberapi_core::CollectionId::new("another");
        assert!(
            cx.read(|cx| super::super::api::typed_prompts(&elsewhere, cx))
                .is_empty()
        );

        let heard = Rc::new(RefCell::new(Vec::new()));
        let into = heard.clone();
        cx.update(|cx| {
            cx.subscribe(&state, move |_, event, _| {
                if let WorkspaceEvent::Code(_, code) = event {
                    into.borrow_mut().push(code.clone());
                }
            })
            .detach();
        });
        let id = state
            .read_with(cx, |state, _| state.selected().cloned())
            .expect("selected");
        state.update(cx, |state, cx| state.copy_code(&id, CodeTarget::Curl, cx));
        cx.run_until_parked();
        assert!(heard.borrow()[0].contains("code=123456"), "{:?}", heard.borrow());
    }

    /// A GraphQL endpoint's schema, asked for by one API, is offered to
    /// every API of the project sent to the same URI, and to no other.
    #[gpui_kit::test]
    async fn a_schema_one_api_asked_for_serves_every_api_sent_there(cx: &mut TestAppContext) {
        struct Introspected;
        impl HttpEngine for Introspected {
            fn send(&self, _: PreparedRequest) -> SendFuture {
                let body = r#"{"data": {"__schema": {"queryType": {"name": "Query"},
                    "types": [{"kind": "OBJECT", "name": "Query", "fields": []}]}}}"#;
                Box::pin(async move {
                    Ok(Exchange {
                        status: 200,
                        body: body.as_bytes().to_vec(),
                        ..Default::default()
                    })
                })
            }
        }
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let introspected = Sender {
            engine: Arc::new(Introspected),
            ..sender(&storage, true)
        };
        let state = cx.new(|cx| WorkspaceState::new(introspected, collection.clone(), cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.create_api(None, "me".into(), cx));
        cx.run_until_parked();
        let me = editor(&state, cx);
        me.update(cx, |api, cx| api.set_uri("http://gql.test/graphql".into(), cx));
        me.update(cx, |api, cx| api.ask_schema(cx));
        assert!(me.read_with(cx, |api, _| api.asking_schema()));
        cx.run_until_parked();
        me.read_with(cx, |api, cx| {
            assert!(!api.asking_schema());
            assert_eq!(api.schema(cx).map(|schema| schema.type_count()), Some(1));
            assert!(api.responses().is_empty(), "not in the history");
        });

        state.update(cx, |state, cx| state.create_api(None, "users".into(), cx));
        cx.run_until_parked();
        let users = editor(&state, cx);
        assert!(users.read_with(cx, |api, cx| api.schema(cx).is_none()));
        users.update(cx, |api, cx| api.set_uri(" http://gql.test/graphql ".into(), cx));
        assert!(
            users.read_with(cx, |api, cx| api.schema(cx).is_some()),
            "the same endpoint"
        );

        // An endpoint that answers with no schema says why.
        let plain = cx.new(|cx| WorkspaceState::new(sender(&storage, true), collection.clone(), cx));
        cx.run_until_parked();
        let api = editor(&plain, cx);
        let heard = Rc::new(RefCell::new(Vec::new()));
        let into = heard.clone();
        cx.update(|cx| {
            cx.subscribe(&api, move |_, event, _| {
                if let ApiEvent::NoSchema(why) = event {
                    into.borrow_mut().push(why.clone());
                }
            })
            .detach();
        });
        api.update(cx, |api, cx| api.ask_schema(cx));
        cx.run_until_parked();
        assert!(heard.borrow()[0].starts_with("not JSON"), "{:?}", heard.borrow());
        assert!(api.read_with(cx, |api, cx| api.schema(cx).is_none()));
    }

    /// A gRPC call: the body becomes its message; `.proto` files give the
    /// definitions, every API at the address is offered them, and a call
    /// with no method takes the first it can make, with its example.
    #[gpui_kit::test]
    async fn a_grpc_call_takes_its_definitions_from_proto_files(cx: &mut TestAppContext) {
        let folder = tempfile::tempdir().expect("a folder");
        let proto = folder.path().join("shop.proto");
        std::fs::write(
            &proto,
            "syntax = \"proto3\";\npackage shop;\nimport \"google/protobuf/timestamp.proto\";\n\
             message Ask { string id = 1; google.protobuf.Timestamp at = 2; }\n\
             service Store { rpc Watch(Ask) returns (stream Ask); rpc Get(Ask) returns (Ask); }\n",
        )
        .expect("written");
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_api(None, "get".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        let heard = Rc::new(RefCell::new(Vec::new()));
        let into = heard.clone();
        cx.update(|cx| {
            cx.subscribe(&api, move |_, event, _| match event {
                ApiEvent::RequestReplaced => into.borrow_mut().push("replaced".to_string()),
                ApiEvent::GrpcDefinitions { methods, example } => into
                    .borrow_mut()
                    .push(format!("{methods} {}", example.as_deref().unwrap_or("-"))),
                ApiEvent::NoGrpcDefinitions(why) => into.borrow_mut().push(why.clone()),
                _ => {}
            })
            .detach();
        });
        api.update(cx, |api, cx| {
            api.set_method(HttpMethod::Post, cx);
            api.set_body(r#"{"id": "7"}"#.into(), cx);
            api.set_uri("localhost:50051".into(), cx);
            api.set_method(HttpMethod::Grpc, cx);
        });
        let message = |api: &Entity<ApiState>, cx: &mut TestAppContext| {
            api.read_with(cx, |api, _| GrpcBody::read(&api.request().body))
        };
        assert_eq!(message(&api, cx).message, r#"{"id": "7"}"#);
        assert_eq!(heard.borrow().as_slice(), ["replaced"]);

        api.update(cx, |api, cx| api.import_protos(vec![proto], cx));
        assert!(api.read_with(cx, |api, _| api.loading_grpc()));
        cx.run_until_parked();
        assert_eq!(
            heard.borrow()[1],
            "2 {\n    \"id\": \"\",\n    \"at\": \"1970-01-01T00:00:00Z\"\n}",
            "Watch streams: Get is taken"
        );
        let body = message(&api, cx);
        assert_eq!((body.service.as_str(), body.method.as_str()), ("shop.Store", "Get"));
        assert_eq!(body.source, "shop.proto");
        assert_eq!(body.message, r#"{"id": "7"}"#, "what the reader wrote stays");
        let kept = Definitions::from_base64(&body.descriptors).expect("kept");
        assert_eq!(kept.methods().len(), 2);

        // Another API at the address is offered them.
        state.update(cx, |state, cx| state.create_api(None, "other".into(), cx));
        cx.run_until_parked();
        let other = editor(&state, cx);
        other.update(cx, |api, cx| {
            api.set_method(HttpMethod::Grpc, cx);
            api.set_uri(" localhost:50051 ".into(), cx);
        });
        let offered = other.read_with(cx, |api, cx| api.grpc_definitions(cx).map(|known| known.source.clone()));
        assert_eq!(offered.as_deref(), Some("shop.proto"));

        // Back to HTTP, the message is the body again.
        api.update(cx, |api, cx| api.set_method(HttpMethod::Post, cx));
        assert_eq!(api.read_with(cx, |api, _| api.request().body.clone()), r#"{"id": "7"}"#);

        // A file that does not compile says why; what was loaded stays.
        let refused = Rc::new(RefCell::new(Vec::new()));
        let into = refused.clone();
        cx.update(|cx| {
            cx.subscribe(&other, move |_, event, _| {
                if let ApiEvent::NoGrpcDefinitions(why) = event {
                    into.borrow_mut().push(why.clone());
                }
            })
            .detach();
        });
        let broken = folder.path().join("broken.proto");
        std::fs::write(&broken, "syntax = \"proto3\";\nmessage {").expect("written");
        other.update(cx, |api, cx| api.import_protos(vec![broken], cx));
        cx.run_until_parked();
        assert_eq!(refused.borrow().len(), 1, "{:?}", refused.borrow());
        assert!(other.read_with(cx, |api, cx| api.grpc_definitions(cx).is_some()));
    }

    #[gpui_kit::test]
    async fn sending_keeps_the_answer_and_a_second_click_cancels(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = cx.new(|cx| WorkspaceState::new(sender(&storage, true), collection.clone(), cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.create_api(None, "ping".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        api.update(cx, |api, cx| api.set_uri(" http://ping.test ".into(), cx));
        api.update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
        cx.run_until_parked();
        api.read_with(cx, |api, _| {
            assert_eq!(api.request().uri, "http://ping.test");
            assert!(api.sending_since().is_none());
            let shown = api.shown_response().expect("an answer");
            assert_eq!(shown.status, Some(200));
            assert_eq!(api.responses().len(), 1);
        });

        // A request that never answers is cancelled by sending again.
        let stuck = cx.new(|cx| WorkspaceState::new(sender(&storage, false), collection.clone(), cx));
        cx.run_until_parked();
        let api = editor(&stuck, cx);
        api.update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
        cx.run_until_parked();
        assert!(api.read_with(cx, |api, _| api.sending_since().is_some()));
        api.update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
        assert!(
            api.read_with(cx, |api, _| api.sending_since().is_some()),
            "a double click is one click"
        );
        std::thread::sleep(Duration::from_millis(210));
        api.update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
        api.read_with(cx, |api, _| {
            assert!(api.sending_since().is_none(), "cancelled");
            assert_eq!(api.responses().len(), 1, "a cancelled request leaves nothing");
        });
    }

    /// PERF-2: a GraphQL answer's errors are read once for the response,
    /// however often the panel and the editor ask.
    #[gpui_kit::test]
    async fn a_graphql_answers_errors_are_read_once(cx: &mut TestAppContext) {
        struct Failing;
        impl HttpEngine for Failing {
            fn send(&self, _: PreparedRequest) -> SendFuture {
                Box::pin(async {
                    Ok(Exchange {
                        status: 200,
                        body: br#"{"data": null, "errors": [{"message": "no such field",
                            "locations": [{"line": 2, "column": 3}], "path": ["me"]}]}"#
                            .to_vec(),
                        ..Default::default()
                    })
                })
            }
        }
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let sender = Sender {
            engine: Arc::new(Failing),
            ..sender(&storage, true)
        };
        let state = cx.new(|cx| WorkspaceState::new(sender, collection, cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.create_api(None, "me".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        api.update(cx, |api, cx| {
            api.set_method(HttpMethod::Post, cx);
            api.set_uri("https://a.test/graphql".into(), cx);
            api.set_content_type(cyberapi_core::model::ContentType::GraphQl, cx);
            api.send_or_cancel(Answers::default(), cx);
        });
        cx.run_until_parked();
        api.read_with(cx, |api, _| {
            let response = api.shown_response().expect("answered");
            let errors = api.graphql_errors(response);
            assert_eq!(errors.messages, ["no such field (me)"]);
            assert_eq!(
                errors
                    .places
                    .iter()
                    .map(|place| (place.line, place.column))
                    .collect::<Vec<_>>(),
                [(2, 3)]
            );
            assert!(Rc::ptr_eq(&errors, &api.graphql_errors(response)), "read once");
        });
    }

    type SocketServer = (
        futures::channel::mpsc::UnboundedReceiver<cyberapi_http::SocketOut>,
        futures::channel::mpsc::UnboundedSender<(cyberapi_http::SocketIn, u64)>,
    );

    /// E1: an engine whose WebSockets the test is the server of.
    struct Sockets(Arc<std::sync::Mutex<Option<SocketServer>>>);

    impl HttpEngine for Sockets {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            Box::pin(std::future::pending())
        }
        fn open_socket(&self, _: PreparedRequest) -> cyberapi_http::OpenFuture {
            let (outgoing, sent) = futures::channel::mpsc::unbounded();
            let (told, incoming) = futures::channel::mpsc::unbounded();
            *self.0.lock().expect("server") = Some((sent, told));
            Box::pin(async move {
                Ok(cyberapi_http::Opened::Socket(cyberapi_http::Socket {
                    status: 101,
                    headers: Vec::new(),
                    stats: Default::default(),
                    outgoing,
                    incoming,
                }))
            })
        }
    }

    /// E1: a WebSocket API opened on a [`Sockets`] engine, its workspace,
    /// and its server.
    async fn open_socket(
        cx: &mut TestAppContext,
    ) -> (
        Storage,
        Collection,
        Entity<WorkspaceState>,
        Entity<ApiState>,
        SocketServer,
    ) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let server = Arc::new(std::sync::Mutex::new(None));
        let sender = Sender {
            engine: Arc::new(Sockets(server.clone())),
            ..sender(&storage, true)
        };
        let state = cx.new(|cx| WorkspaceState::new(sender, collection.clone(), cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.create_api(None, "chat".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        api.update(cx, |api, cx| {
            api.set_method(HttpMethod::WebSocket, cx);
            api.set_uri("ws://chat.test".into(), cx);
            api.set_body("hi".into(), cx);
        });
        api.update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
        cx.run_until_parked();
        assert!(api.read_with(cx, |api, _| api.socket_open()), "open");
        let server = server.lock().expect("server").take().expect("opened");
        (storage, collection, state, api, server)
    }

    /// E1: a WebSocket API connects, hears and sends messages as they go,
    /// and once closed keeps what went each way as its answer.
    #[gpui_kit::test]
    async fn a_websocket_connects_and_keeps_its_messages(cx: &mut TestAppContext) {
        use cyberapi_core::ws::{self, Frame};
        use cyberapi_http::{SocketIn, SocketOut};
        let (storage, collection, _state, api, (mut sent, told)) = open_socket(cx).await;

        told.unbounded_send((SocketIn::Message(Frame::Text("welcome".into())), 5))
            .expect("open");
        cx.run_until_parked();
        api.update(cx, |api, cx| api.send_message(Answers::default(), cx));
        cx.run_until_parked();
        assert_eq!(sent.try_recv().ok(), Some(SocketOut::Send(Frame::Text("hi".into()))));
        let live: Vec<(String, String)> = api.read_with(cx, |api, _| {
            let live = api.live().expect("live");
            assert_eq!(live.status, 101);
            live.events
                .iter()
                .map(|event| (event.event.clone(), event.data.clone()))
                .collect()
        });
        assert_eq!(
            live,
            [
                (ws::RECEIVED.to_string(), "welcome".to_string()),
                (ws::SENT.to_string(), "hi".to_string())
            ]
        );

        // Closed: the server is told, and once it answers it is kept.
        std::thread::sleep(Duration::from_millis(210));
        api.update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
        cx.run_until_parked();
        assert_eq!(sent.try_recv().ok(), Some(SocketOut::Close));
        assert!(api.read_with(cx, |api, _| api.live().is_some_and(|live| live.stopping)));
        told.unbounded_send((
            SocketIn::Closed {
                code: Some(1000),
                reason: String::new(),
            },
            9,
        ))
        .expect("open");
        // What comes is taken in a frame at a time.
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        api.read_with(cx, |api, _| {
            assert!(!api.socket_open() && api.sending_since().is_none() && api.live().is_none());
            let shown = api.shown_response().expect("kept");
            assert_eq!(shown.status, Some(101));
            assert_eq!(
                shown
                    .stream_events()
                    .iter()
                    .map(|event| event.data.as_str())
                    .collect::<Vec<_>>(),
                ["welcome", "hi"]
            );
        });
        let api_id = api.read_with(cx, |api, _| api.api().clone());
        assert_eq!(
            storage
                .history()
                .list(collection.id, api_id)
                .await
                .expect("history")
                .len(),
            1
        );
    }

    /// PERF-6: once what went each way is as much as is kept, the socket
    /// is closed, nothing more is kept, and what is says it was cut.
    #[gpui_kit::test]
    async fn a_websocket_is_closed_once_it_has_kept_the_most(cx: &mut TestAppContext) {
        use cyberapi_core::response::StreamEnd;
        use cyberapi_core::ws::Frame;
        use cyberapi_http::{SocketIn, SocketOut};
        let (_storage, _collection, _state, api, (mut sent, told)) = open_socket(cx).await;
        api.update(cx, |api, _| api.limit_live(64));
        let message = |text: &str| (SocketIn::Message(Frame::Text(text.into())), 5);
        told.unbounded_send(message("one")).expect("open");
        cx.run_until_parked();
        assert!(sent.try_recv().is_err(), "room for more");
        cx.executor().advance_clock(Duration::from_millis(100));
        for _ in 0..3 {
            told.unbounded_send(message("a longer message")).expect("open");
        }
        cx.run_until_parked();
        assert_eq!(sent.try_recv().ok(), Some(SocketOut::Close));
        assert!(api.read_with(cx, |api, _| api.live().is_some_and(|live| live.stopping)));
        // What comes while it closes is not kept, nor is what is sent.
        cx.executor().advance_clock(Duration::from_millis(100));
        told.unbounded_send(message("after")).expect("open");
        cx.run_until_parked();
        api.update(cx, |api, cx| api.send_message(Answers::default(), cx));
        cx.run_until_parked();
        assert!(sent.try_recv().is_err(), "nothing sent once closing");
        told.unbounded_send((
            SocketIn::Closed {
                code: Some(1000),
                reason: String::new(),
            },
            9,
        ))
        .expect("open");
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        api.read_with(cx, |api, _| {
            let shown = api.shown_response().expect("kept");
            assert_eq!(
                shown.stream.as_ref().map(|stream| stream.ended.clone()),
                Some(StreamEnd::Cut)
            );
            let kept: Vec<String> = shown.stream_events().into_iter().map(|event| event.data).collect();
            // The second reached the most: the third came after.
            assert_eq!(kept, ["one", "a longer message"]);
        });
    }

    /// PERF-8: only the newest answer is held whole; the one before it is
    /// as the history keeps it, its big body left out.
    #[gpui_kit::test]
    async fn only_the_newest_answer_is_held_whole(cx: &mut TestAppContext) {
        use cyberapi_core::response::HISTORY_BODY_LIMIT;
        struct Big;
        impl HttpEngine for Big {
            fn send(&self, _: PreparedRequest) -> SendFuture {
                Box::pin(async {
                    Ok(Exchange {
                        status: 200,
                        body: vec![b'x'; HISTORY_BODY_LIMIT + 1],
                        ..Default::default()
                    })
                })
            }
        }
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let sender = Sender {
            engine: Arc::new(Big),
            ..sender(&storage, true)
        };
        let state = cx.new(|cx| WorkspaceState::new(sender, collection, cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.create_api(None, "big".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        api.update(cx, |api, cx| api.set_uri("https://a.test".into(), cx));
        for _ in 0..2 {
            // Not a double click.
            std::thread::sleep(Duration::from_millis(210));
            api.update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
            cx.run_until_parked();
        }
        api.read_with(cx, |api, _| {
            let held: Vec<(usize, Option<u64>)> = api
                .responses()
                .iter()
                .map(|response| (response.body.len(), response.body_omitted))
                .collect();
            let whole = HISTORY_BODY_LIMIT + 1;
            assert_eq!(held, [(whole, None), (0, Some(whole as u64))]);
        });
    }

    /// PERF-10: a gRPC call's body is read once for each body it has: the
    /// editor asks for it in every frame.
    #[gpui_kit::test]
    async fn a_grpc_body_is_read_once_for_each_body(cx: &mut TestAppContext) {
        use cyberapi_core::grpc::GrpcBody;
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection, cx);
        state.update(cx, |state, cx| state.create_api(None, "call".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        let body = |method: &str| {
            GrpcBody {
                service: "shop.Store".into(),
                method: method.into(),
                ..Default::default()
            }
            .to_body()
        };
        api.update(cx, |api, cx| {
            api.set_method(HttpMethod::Grpc, cx);
            api.set_body(body("Get"), cx);
        });
        api.update(cx, |api, cx| {
            let first = api.grpc_body();
            assert_eq!(first.method, "Get");
            assert!(Rc::ptr_eq(&first, &api.grpc_body()), "read once");
            api.set_body(body("List"), cx);
            let changed = api.grpc_body();
            assert_eq!(changed.method, "List", "read again once it changed");
        });
    }

    /// PERF-9: an opened API holds its history as a list shows it, its
    /// newest whole; one chosen is read whole before it is shown, and the
    /// one shown before goes back to its summary.
    #[gpui_kit::test]
    async fn the_history_is_read_whole_only_where_shown(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let api = storage
            .tree()
            .create_api(collection.id.clone(), None, "a".into())
            .await
            .expect("create")
            .item;
        let mut ids = Vec::new();
        for n in 0..3 {
            let kept = storage
                .history()
                .add(
                    HttpResponse {
                        collection: collection.id.clone(),
                        api: api.id.clone(),
                        status: Some(200),
                        body: format!("body {n}").into_bytes(),
                        ..Default::default()
                    },
                    10,
                )
                .await
                .expect("add");
            ids.push(kept.id);
        }
        let state = open(&storage, collection, cx);
        state.update(cx, |state, cx| state.select(&api.id, cx));
        cx.run_until_parked();
        let editor = editor(&state, cx);
        let bodies = |cx: &mut TestAppContext| -> (Vec<String>, String) {
            editor.read_with(cx, |api, _| {
                let held = api
                    .responses()
                    .iter()
                    .map(|response| String::from_utf8_lossy(&response.body).into_owned())
                    .collect();
                let shown = api.shown_response().expect("shown");
                (held, String::from_utf8_lossy(&shown.body).into_owned())
            })
        };
        assert_eq!(
            bodies(cx),
            (vec!["body 2".into(), String::new(), String::new()], "body 2".into())
        );
        editor.update(cx, |api, cx| api.choose_response(&ids[0], cx));
        cx.run_until_parked();
        assert_eq!(
            bodies(cx),
            (vec!["body 2".into(), String::new(), "body 0".into()], "body 0".into())
        );
        editor.update(cx, |api, cx| api.choose_response(&ids[1], cx));
        cx.run_until_parked();
        assert_eq!(
            bodies(cx),
            (vec!["body 2".into(), "body 1".into(), String::new()], "body 1".into()),
            "the one shown before is a summary again"
        );
        let whole = editor.read_with(cx, |api, _| api.whole_response(&ids[0]));
        assert_eq!(whole.await.expect("whole").body, b"body 0");
    }

    #[gpui_kit::test]
    async fn choosing_a_response_puts_its_request_back(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_api(None, "ping".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        api.update(cx, |api, cx| api.set_uri("http://first.test".into(), cx));
        api.update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
        cx.run_until_parked();
        let first = api.read_with(cx, |api, _| api.responses()[0].id.clone());
        api.update(cx, |api, cx| api.set_uri("http://second.test".into(), cx));
        cx.run_until_parked();

        let replaced = Rc::new(RefCell::new(0));
        let seen = replaced.clone();
        cx.update(|cx| {
            cx.subscribe(&api, move |_, event, _| {
                if matches!(event, ApiEvent::RequestReplaced) {
                    *seen.borrow_mut() += 1;
                }
            })
            .detach();
        });
        api.update(cx, |api, cx| api.choose_response(&first, cx));
        api.read_with(cx, |api, _| {
            assert_eq!(api.request().uri, "http://first.test");
            assert_eq!(api.shown_response().map(|r| r.id.clone()), Some(first.clone()));
        });
        assert_eq!(*replaced.borrow(), 1);
        let id = api.read_with(cx, |api, _| api.api().clone());
        // Not saved until it is changed (F7.10): not by sending it,
        cx.run_until_parked();
        assert_eq!(stored(&storage, &collection, &id).uri, "http://second.test");
        std::thread::sleep(Duration::from_millis(210));
        api.update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
        cx.run_until_parked();
        assert_eq!(stored(&storage, &collection, &id).uri, "http://second.test");
        // nor by leaving the API.
        drop(api);
        state.update(cx, |state, cx| state.create_api(None, "other".into(), cx));
        cx.run_until_parked();
        assert_eq!(stored(&storage, &collection, &id).uri, "http://second.test");

        // Changed, it is saved whole.
        state.update(cx, |state, cx| state.select(&id, cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        api.update(cx, |api, cx| api.choose_response(&first, cx));
        api.update(cx, |api, cx| api.set_method(HttpMethod::Put, cx));
        cx.run_until_parked();
        let saved = stored(&storage, &collection, &id);
        assert_eq!(
            (saved.method, saved.uri.as_str()),
            (HttpMethod::Put, "http://first.test")
        );

        api.update(cx, |api, cx| api.clear_history(cx));
        cx.run_until_parked();
        assert!(api.read_with(cx, |api, _| api.shown_response().is_none()));
    }

    /// F2: an API asked for while the project loads is selected once it
    /// and the reader's state of it are read, over the selection kept.
    #[gpui_kit::test]
    async fn an_api_asked_for_while_loading_is_selected(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_api(None, "a".into(), cx));
        state.update(cx, |state, cx| state.create_api(None, "b".into(), cx));
        cx.run_until_parked();
        let (a, b) = (find(&state, "a", cx), find(&state, "b", cx));
        assert_eq!(
            state.read_with(cx, |state, _| state.opened_api().map(|api| api.id.clone())),
            Some(b)
        );
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        drop(state);
        let state = cx.new(|cx| WorkspaceState::new(sender(&storage, true), collection.clone(), cx));
        state.update(cx, |state, cx| state.select_when_loaded(a.clone(), cx));
        cx.run_until_parked();
        assert_eq!(
            state.read_with(cx, |state, _| state.opened_api().map(|api| api.id.clone())),
            Some(a),
            "the one asked for, not the one kept"
        );
    }

    /// A3: a run sends the APIs in the tree's order, once for each row,
    /// and says how each went; what it sent is in the histories.
    #[gpui_kit::test]
    async fn a_run_sends_in_order_and_says_how_each_went(cx: &mut TestAppContext) {
        use cyberapi_core::assertions::{Assertion, Op, Subject};
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        for (name, uri) in [("a", "http://a.test/1"), ("b", "http://b.test/2")] {
            state.update(cx, |state, cx| state.create_api(None, name.into(), cx));
            cx.run_until_parked();
            let api = editor(&state, cx);
            api.update(cx, |api, cx| api.set_uri(uri.into(), cx));
            if name == "b" {
                api.update(cx, |api, cx| {
                    api.set_assertions(
                        vec![Assertion {
                            subject: Subject::Status,
                            op: Op::Equals,
                            expected: "201".into(),
                            ..Default::default()
                        }],
                        cx,
                    )
                });
            }
            cx.executor().advance_clock(SAVE_DELAY);
            cx.run_until_parked();
        }
        let run = cx.new(|cx| super::super::RunState::new(state.clone(), None, cx));
        let names: Vec<String> = run.read_with(cx, |run, _| run.planned().iter().map(|p| p.name.clone()).collect());
        assert_eq!(names, ["b", "a"], "the tree's order: a new API goes first");
        run.update(cx, |run, cx| run.start("n\n1\n2", HashMap::new(), cx))
            .expect("the data reads");
        assert!(run.read_with(cx, |run, _| run.running()));
        for _ in 0..10 {
            cx.executor().advance_clock(Duration::from_millis(60));
            cx.run_until_parked();
        }
        run.read_with(cx, |run, _| {
            assert!(!run.running(), "over");
            assert_eq!(run.passes(), 2);
            let places: Vec<(usize, usize)> = run.results().iter().map(|r| (r.pass, r.step)).collect();
            assert_eq!(places, [(0, 0), (0, 1), (1, 0), (1, 1)]);
            assert_eq!(run.tally(), (2, 2), "b's 200 is not the 201 it asks for");
        });
        let a = find(&state, "a", cx);
        let kept = storage.history().list(collection.id.clone(), a).await.expect("list");
        assert_eq!(kept.len(), 2, "each send kept");
        // Data that does not read starts nothing.
        let started = run.update(cx, |run, cx| run.start("[1", HashMap::new(), cx));
        assert!(started.is_err());
        assert_eq!(
            run.read_with(cx, |run, _| run.results().len()),
            4,
            "the last run still shown"
        );
    }

    /// A7: a load test sends the API so many times and counts them; none
    /// is kept in its history.
    #[gpui_kit::test]
    async fn a_load_test_counts_what_it_sent(cx: &mut TestAppContext) {
        use cyberapi_core::load::{Limit, Plan};
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_api(None, "a".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        api.update(cx, |api, cx| api.set_uri("http://a.test/1".into(), cx));
        cx.executor().advance_clock(SAVE_DELAY);
        cx.run_until_parked();
        let a = find(&state, "a", cx);
        let load = cx.new(|cx| super::super::LoadState::new(state.clone(), a.clone(), cx));
        let plan = Plan {
            senders: 3,
            limit: Limit::Count(7),
        };
        load.update(cx, |load, cx| load.start(plan, HashMap::new(), cx));
        assert!(load.read_with(cx, |load, _| load.running()));
        for _ in 0..10 {
            cx.executor().advance_clock(Duration::from_millis(400));
            cx.run_until_parked();
        }
        load.read_with(cx, |load, _| {
            assert!(!load.running(), "over");
            assert_eq!(load.failure(), None);
            assert_eq!((load.summary().sent, load.summary().failed), (7, 0));
            assert_eq!(load.plan(), Some(plan));
        });
        let kept = storage.history().list(collection.id.clone(), a).await.expect("list");
        assert!(kept.is_empty(), "nothing kept");
    }

    /// D4: the project's newest responses, any API's; one picked opens its
    /// API and is shown there, its request put back.
    #[gpui_kit::test]
    async fn the_recent_list_opens_a_response_where_it_was(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_api(None, "a".into(), cx));
        state.update(cx, |state, cx| state.create_api(None, "b".into(), cx));
        cx.run_until_parked();
        let (a, b) = (find(&state, "a", cx), find(&state, "b", cx));
        let mut kept = Vec::new();
        for (api, uri) in [
            (&a, "http://a.test/1"),
            (&a, "http://a.test/2"),
            (&b, "http://b.test/1"),
        ] {
            let response = HttpResponse {
                collection: collection.id.clone(),
                api: api.clone(),
                request: HttpRequest {
                    uri: uri.into(),
                    ..Default::default()
                },
                status: Some(200),
                body: b"{}".to_vec(),
                ..Default::default()
            };
            kept.push(
                storage
                    .history()
                    .add(response, cyberapi_core::response::HISTORY_LIMIT)
                    .await
                    .expect("kept"),
            );
        }
        state.update(cx, |state, cx| state.load_recent(cx));
        assert!(state.read_with(cx, |state, _| state.recent().is_none()), "being read");
        cx.run_until_parked();
        let listed: Vec<String> = state.read_with(cx, |state, _| match state.recent() {
            Some(Ok(responses)) => responses.iter().map(|response| response.id.clone()).collect(),
            other => panic!("read: {other:?}"),
        });
        assert_eq!(listed, [kept[2].id.as_str(), kept[1].id.as_str(), kept[0].id.as_str()]);
        assert_eq!(
            state.read_with(cx, |state, _| state.api_name(&a).map(str::to_string)),
            Some("a".into())
        );

        // `b` is open; the oldest of `a` is picked.
        state.update(cx, |state, cx| state.open_response(&a, &kept[0].id, cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        api.read_with(cx, |api, _| {
            assert_eq!(api.api(), &a);
            assert_eq!(
                api.shown_response().map(|shown| shown.id.clone()),
                Some(kept[0].id.clone())
            );
            assert_eq!(api.request().uri, "http://a.test/1");
        });
    }

    #[gpui_kit::test]
    async fn the_tab_is_kept_per_api(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_api(None, "a".into(), cx));
        cx.run_until_parked();
        let tab = |state: &Entity<WorkspaceState>, cx: &mut TestAppContext| {
            let api = editor(state, cx);
            state.read_with(cx, |state, cx| state.tab(&api, cx))
        };
        assert_eq!(tab(&state, cx), RequestTab::Query);
        let a = editor(&state, cx);
        state.update(cx, |state, cx| state.set_tab(&a, RequestTab::Auth, cx));
        cx.run_until_parked();
        let again = open(&storage, collection.clone(), cx);
        assert_eq!(tab(&again, cx), RequestTab::Auth);

        // F4.12: another method opens on its default tab.
        a.update(cx, |api, cx| api.set_method(HttpMethod::Post, cx));
        cx.run_until_parked();
        assert_eq!(tab(&state, cx), RequestTab::Body);
        let again = open(&storage, collection, cx);
        assert_eq!(tab(&again, cx), RequestTab::Body);

        // BUG-4: with `b` opened, `a`'s own tab is still `a`'s — as its
        // editor in a window of its own shows it — and picking one for `a`
        // leaves `b`'s alone.
        state.update(cx, |state, cx| state.create_api(None, "b".into(), cx));
        cx.run_until_parked();
        let b = editor(&state, cx);
        assert_ne!(a.entity_id(), b.entity_id());
        state.update(cx, |state, cx| state.set_tab(&a, RequestTab::Headers, cx));
        assert_eq!(state.read_with(cx, |state, cx| state.tab(&a, cx)), RequestTab::Headers);
        assert_eq!(state.read_with(cx, |state, cx| state.tab(&b, cx)), RequestTab::Query);
    }

    #[gpui_kit::test]
    async fn deleting_clears_what_pointed_at_it(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_folder(None, "users".into(), cx));
        cx.run_until_parked();
        let users = find(&state, "users", cx);
        state.update(cx, |state, cx| {
            state.create_api(Some(users.clone()), "login".into(), cx)
        });
        cx.run_until_parked();
        let login = find(&state, "login", cx);
        state.update(cx, |state, cx| state.set_pinned(&login, true, cx));
        state.update(cx, |state, cx| state.delete(users.clone(), cx));
        cx.run_until_parked();

        state.read_with(cx, |state, _| {
            assert!(state.rows().is_empty());
            assert_eq!(state.selected(), None);
            assert!(state.pinned().is_empty());
            assert!(state.tree().is_empty());
        });
        // And from the store: a reopening finds nothing either.
        let again = open(&storage, collection, cx);
        assert!(row_names(&again, cx).is_empty());
    }

    /// Server-sync §3–4: a server project in a page.
    mod copies {
        use super::*;
        use crate::drafts_local;

        const BEAT: Duration = Duration::from_secs(10);
        const READER: &str = "alice";

        /// Projects the reader only views.
        struct Viewing(MemoryStore);

        impl cyberapi_store::CollectionStore for Viewing {
            fn list(&self) -> cyberapi_store::StoreFuture<Vec<Collection>> {
                let call = self.0.list();
                Box::pin(async move {
                    let mut found = call.await?;
                    for collection in &mut found {
                        collection.role = CollectionRole::Viewer;
                    }
                    Ok(found)
                })
            }

            fn get(&self, id: cyberapi_core::CollectionId) -> cyberapi_store::StoreFuture<Collection> {
                let call = self.0.get(id);
                Box::pin(async move {
                    Ok(Collection {
                        role: CollectionRole::Viewer,
                        ..call.await?
                    })
                })
            }

            fn create(&self, input: NewCollection) -> cyberapi_store::StoreFuture<Collection> {
                self.0.create(input)
            }

            fn update(
                &self,
                id: cyberapi_core::CollectionId,
                edit: cyberapi_store::CollectionEdit,
            ) -> cyberapi_store::StoreFuture<Collection> {
                self.0.update(id, edit)
            }

            fn delete(&self, id: cyberapi_core::CollectionId) -> cyberapi_store::StoreFuture<()> {
                cyberapi_store::CollectionStore::delete(&self.0, id)
            }

            fn api_counts(
                &self,
            ) -> cyberapi_store::StoreFuture<std::collections::BTreeMap<cyberapi_core::CollectionId, usize>>
            {
                self.0.api_counts()
            }
        }

        /// A server project opened as a page opens it, and what it said.
        fn open_copy(
            server: &Storage,
            collection: Collection,
            cx: &mut TestAppContext,
        ) -> (Entity<WorkspaceState>, Rc<RefCell<Vec<String>>>) {
            let state = cx.new(|cx| {
                let copy = ServerCopy::page(&collection, READER.into());
                WorkspaceState::with_poll(
                    sender(server, true),
                    collection,
                    Some((server.clone(), copy)),
                    Some(BEAT),
                    cx,
                )
            });
            let told: Rc<RefCell<Vec<String>>> = Rc::default();
            let heard = told.clone();
            cx.update(|cx| {
                cx.subscribe(&state, move |_, event: &WorkspaceEvent, _| {
                    heard.borrow_mut().push(format!("{event:?}"));
                })
                .detach();
            });
            cx.run_until_parked();
            (state, told)
        }

        fn server_rows(server: &Storage, collection: &Collection) -> Vec<String> {
            let data = pollster::block_on(server.tree().load(collection.id.clone())).expect("load");
            let mut names: Vec<String> = data.apis.into_iter().map(|api| api.name).collect();
            names.sort();
            names
        }

        fn unsynced(state: &Entity<WorkspaceState>, cx: &mut TestAppContext) -> bool {
            state.read_with(cx, |state, _| state.unsynced())
        }

        fn beat(cx: &mut TestAppContext) {
            cx.executor().advance_clock(BEAT);
            cx.run_until_parked();
        }

        #[gpui_kit::test]
        async fn changes_wait_in_the_draft_until_synced(cx: &mut TestAppContext) {
            let server = MemoryStore::new().storage();
            let collection = project(&server).await;
            let (state, told) = open_copy(&server, collection.clone(), cx);
            assert!(state.read_with(cx, |state, _| state.syncs()));
            assert!(!unsynced(&state, cx));

            state.update(cx, |state, cx| state.create_api(None, "mine".into(), cx));
            state.update(cx, |state, cx| state.create_folder(None, "users".into(), cx));
            cx.run_until_parked();
            assert_eq!(row_names(&state, cx), ["users", "mine"]);
            assert!(server_rows(&server, &collection).is_empty(), "not on the server");
            assert!(unsynced(&state, cx));
            assert_eq!(told.borrow().as_slice(), ["FirstChange"], "told once");
            // PERF-13: written once the changes pause.
            assert_eq!(drafts_local::get(READER, &collection.id), None, "not at each change");
            cx.executor().advance_clock(copy::KEEP_DRAFT_AFTER);
            cx.run_until_parked();
            let draft = drafts_local::get(READER, &collection.id).expect("a draft");
            assert_eq!(draft.base, Some(collection.revision));
            assert_eq!(draft.config.apis.len(), 1);
            // The page hidden, what waits is written at once.
            state.update(cx, |state, cx| state.create_api(None, "hidden".into(), cx));
            cx.run_until_parked();
            crate::set_page_visible(false);
            crate::set_page_visible(true);
            let draft = drafts_local::get(READER, &collection.id).expect("a draft");
            assert_eq!(draft.config.apis.len(), 2);
            // A beat keeps it: the server has not moved.
            beat(cx);
            assert_eq!(row_names(&state, cx), ["hidden", "users", "mine"]);

            // A refresh finds it, and what waited as the project closed.
            state.update(cx, |state, cx| state.create_api(None, "closing".into(), cx));
            cx.run_until_parked();
            drop(state);
            // Released as the app next flushes what happened.
            cx.update(|_| {});
            let draft = drafts_local::get(READER, &collection.id).expect("a draft");
            assert_eq!(draft.config.apis.len(), 3);
            let (again, told) = open_copy(&server, collection.clone(), cx);
            assert_eq!(row_names(&again, cx), ["closing", "hidden", "users", "mine"]);
            assert_eq!(told.borrow().as_slice(), ["KeptUnsynced { behind: false }"]);

            // The editor's typing goes too, without its pause.
            let mine = find(&again, "mine", cx);
            again.update(cx, |state, cx| state.select(&mine, cx));
            cx.run_until_parked();
            editor(&again, cx).update(cx, |api, cx| api.set_headers(vec![header("X-A", "1")], cx));
            again.update(cx, |state, cx| state.sync(false, cx));
            cx.run_until_parked();
            assert_eq!(server_rows(&server, &collection), ["closing", "hidden", "mine"]);
            assert_eq!(stored(&server, &collection, &mine).headers, [header("X-A", "1")]);
            assert!(!unsynced(&again, cx));
            assert_eq!(drafts_local::get(READER, &collection.id), None, "the draft goes");
            assert!(told.borrow().iter().any(|event| event == "Synced"));
            // Its own push is no news on the next beat.
            beat(cx);
            assert!(!again.read_with(cx, |state, _| state.server_moved()));
        }

        /// The children of the folder `id`, as the copy has them.
        fn children(state: &Entity<WorkspaceState>, id: &ItemId, cx: &mut TestAppContext) -> Vec<ItemId> {
            state.read_with(cx, |state, _| {
                state
                    .data
                    .folders
                    .iter()
                    .find(|folder| &folder.id == id)
                    .map(|folder| folder.children.clone())
                    .unwrap_or_default()
            })
        }

        /// Server-sync §3.4: what the reader keeps to themselves stays here
        /// — out of every sync, back after every pull — and changing it is
        /// nothing to sync.
        #[gpui_kit::test]
        async fn the_readers_own_items_stay_here(cx: &mut TestAppContext) {
            let server = MemoryStore::new().storage();
            let collection = project(&server).await;
            let users = server
                .tree()
                .create_folder(collection.id.clone(), None, "users".into())
                .await
                .expect("create")
                .item
                .id;
            let login = server
                .tree()
                .create_api(collection.id.clone(), Some(users.clone()), "login".into())
                .await
                .expect("create")
                .item
                .id;
            let collection = server.collections().get(collection.id.clone()).await.expect("get");
            let (state, told) = open_copy(&server, collection.clone(), cx);
            let can = |state: &Entity<WorkspaceState>, id: &ItemId, cx: &mut TestAppContext| {
                state.read_with(cx, |state, _| state.can_make_local(id))
            };
            assert!(!can(&state, &users, cx), "the server's stay the server's");

            state.update(cx, |state, cx| {
                state.create_folder(Some(users.clone()), "mine".into(), cx)
            });
            cx.run_until_parked();
            let mine = find(&state, "mine", cx);
            assert!(unsynced(&state, cx), "new, and so far to sync");
            assert!(can(&state, &mine, cx));
            state.update(cx, |state, cx| state.set_local(&mine, true, cx));
            cx.run_until_parked();
            assert!(!unsynced(&state, cx), "what is new is the reader's own");
            assert!(state.read_with(cx, |state, _| state.is_local_root(&mine)));

            // Made inside it, the reader's own too: nothing to sync.
            state.update(cx, |state, cx| {
                state.create_api(Some(mine.clone()), "scratch".into(), cx)
            });
            cx.run_until_parked();
            let scratch = find(&state, "scratch", cx);
            assert!(state.read_with(cx, |state, _| state.local_members().contains(&scratch)));
            assert!(!unsynced(&state, cx));

            // BUG-3: deleted and put back, still the reader's own.
            state.update(cx, |state, cx| state.delete(mine.clone(), cx));
            cx.run_until_parked();
            let count = state.read_with(cx, |state, _| state.undo.as_ref().map(|undo| undo.count));
            state.update(cx, |state, cx| state.undo_delete(count.expect("undoable"), cx));
            cx.run_until_parked();
            assert_eq!(find(&state, "scratch", cx), scratch, "back");
            assert!(state.read_with(cx, |state, _| state.is_local_root(&mine)));
            assert!(!unsynced(&state, cx), "nothing of it to sync");

            // The server's do not go into it.
            let into_mine = Placement {
                parent: Some(mine.clone()),
                before: None,
            };
            state.update(cx, |state, cx| state.move_item(login.clone(), into_mine, cx));
            cx.run_until_parked();
            assert!(told.borrow().iter().any(|event| event == "NotIntoLocal"));
            assert_eq!(children(&state, &users, cx), [login.clone(), mine.clone()]);

            // A change to the server's syncs; the sync leaves the reader's out.
            state.update(cx, |state, cx| state.rename(login.clone(), "sign in".into(), cx));
            cx.run_until_parked();
            assert!(unsynced(&state, cx));
            state.update(cx, |state, cx| state.sync(false, cx));
            cx.run_until_parked();
            assert_eq!(server_rows(&server, &collection), ["sign in"]);
            let on_server = server.tree().load(collection.id.clone()).await.expect("load");
            assert_eq!(on_server.folders.len(), 1, "no folder of the reader's");
            assert!(!unsynced(&state, cx));

            // Someone adds theirs: the pull puts the reader's back in place.
            let logout = server
                .tree()
                .create_api(collection.id.clone(), Some(users.clone()), "logout".into())
                .await
                .expect("create")
                .item
                .id;
            beat(cx);
            assert_eq!(children(&state, &users, cx), [login.clone(), mine.clone(), logout]);
            assert_eq!(children(&state, &mine, cx), std::slice::from_ref(&scratch));
            assert!(!unsynced(&state, cx));

            // A refresh still has it.
            drop(state);
            let (again, _) = open_copy(&server, collection.clone(), cx);
            assert_eq!(find(&again, "scratch", cx), scratch);
            assert!(!unsynced(&again, cx));

            // Made to sync after all, it goes up with the next sync.
            again.update(cx, |state, cx| state.set_local(&mine, false, cx));
            cx.run_until_parked();
            assert!(unsynced(&again, cx));
            again.update(cx, |state, cx| state.sync(false, cx));
            cx.run_until_parked();
            assert_eq!(server_rows(&server, &collection), ["logout", "scratch", "sign in"]);
        }

        #[gpui_kit::test]
        async fn nothing_unsynced_follows_the_server(cx: &mut TestAppContext) {
            let server = MemoryStore::new().storage();
            let collection = project(&server).await;
            let (state, _) = open_copy(&server, collection.clone(), cx);
            server
                .tree()
                .create_api(collection.id.clone(), None, "theirs".into())
                .await
                .expect("create");
            beat(cx);
            assert_eq!(row_names(&state, cx), ["theirs"]);
            assert!(!unsynced(&state, cx));
        }

        /// An edit made while a pull is out is the reader's: the pull does
        /// not land over it.
        #[gpui_kit::test]
        async fn a_pull_does_not_land_over_an_edit(cx: &mut TestAppContext) {
            let server = MemoryStore::new().storage();
            let collection = project(&server).await;
            let api = server
                .tree()
                .create_api(collection.id.clone(), None, "login".into())
                .await
                .expect("create");
            let (state, _) = open_copy(&server, collection.clone(), cx);
            state.update(cx, |state, cx| state.select(&api.item.id, cx));
            cx.run_until_parked();
            state.update(cx, |state, cx| state.pull(cx));
            editor(&state, cx).update(cx, |api, cx| api.set_method(HttpMethod::Post, cx));
            cx.run_until_parked();
            assert!(unsynced(&state, cx));
            cx.executor().advance_clock(copy::KEEP_DRAFT_AFTER);
            cx.run_until_parked();
            let draft = drafts_local::get(READER, &collection.id).expect("a draft");
            assert_eq!(draft.config.apis[0].request.method, HttpMethod::Post);
            let copy = state.read_with(cx, |state, _| state.data.apis[0].request.method);
            assert_eq!(copy, HttpMethod::Post);
        }

        #[gpui_kit::test]
        async fn a_conflict_waits_for_the_readers_choice(cx: &mut TestAppContext) {
            let server = MemoryStore::new().storage();
            let collection = project(&server).await;
            let (state, told) = open_copy(&server, collection.clone(), cx);
            state.update(cx, |state, cx| state.create_api(None, "mine".into(), cx));
            cx.run_until_parked();
            server
                .tree()
                .create_api(collection.id.clone(), None, "theirs".into())
                .await
                .expect("create");

            // The look keeps the reader's and says the server moved.
            beat(cx);
            assert_eq!(row_names(&state, cx), ["mine"]);
            assert!(state.read_with(cx, |state, _| state.server_moved()));

            state.update(cx, |state, cx| state.sync(false, cx));
            cx.run_until_parked();
            assert_eq!(told.borrow().last().map(String::as_str), Some("Conflict"));
            assert_eq!(server_rows(&server, &collection), ["theirs"], "nothing written");
            assert!(unsynced(&state, cx));

            // Mine over the server's.
            state.update(cx, |state, cx| state.sync(true, cx));
            cx.run_until_parked();
            assert_eq!(server_rows(&server, &collection), ["mine"]);
            assert!(!unsynced(&state, cx));
            assert!(!state.read_with(cx, |state, _| state.server_moved()));
        }

        #[gpui_kit::test]
        async fn taking_the_servers_drops_the_readers_changes(cx: &mut TestAppContext) {
            let server = MemoryStore::new().storage();
            let collection = project(&server).await;
            let api = server
                .tree()
                .create_api(collection.id.clone(), None, "login".into())
                .await
                .expect("create");
            let collection = server.collections().get(collection.id.clone()).await.expect("get");
            let (state, told) = open_copy(&server, collection.clone(), cx);
            state.update(cx, |state, cx| state.select(&api.item.id, cx));
            cx.run_until_parked();
            editor(&state, cx).update(cx, |api, cx| api.set_method(HttpMethod::Post, cx));
            cx.run_until_parked();
            // Typing that waits for its pause goes too.
            editor(&state, cx).update(cx, |api, cx| api.set_query(vec![header("q", "x")], cx));
            assert!(unsynced(&state, cx));

            state.update(cx, |state, cx| state.take_server(cx));
            cx.executor().advance_clock(SAVE_DELAY);
            cx.run_until_parked();
            assert!(!unsynced(&state, cx));
            assert_eq!(drafts_local::get(READER, &collection.id), None);
            let request = editor(&state, cx).read_with(cx, |api, _| api.request().clone());
            assert_eq!((request.method, request.query.len()), (HttpMethod::Get, 0));
            assert!(told.borrow().iter().any(|event| event == "TookServers"));
        }

        #[gpui_kit::test]
        async fn an_api_made_here_has_no_history_yet(cx: &mut TestAppContext) {
            let server = MemoryStore::new().storage();
            let collection = project(&server).await;
            let (state, _) = open_copy(&server, collection.clone(), cx);
            state.update(cx, |state, cx| state.create_api(None, "mine".into(), cx));
            cx.run_until_parked();
            let history = editor(&state, cx).read_with(cx, |api, _| api.history().clone());
            assert_eq!(history, Load::Loaded, "not a failure: the server has none");
        }

        /// #52, #53: an answer of an API only the page knows stays while
        /// the project is open, and goes into the history with the sync
        /// that takes the API to the server.
        #[gpui_kit::test]
        async fn an_answer_of_an_api_made_here_waits_in_the_page_until_synced(cx: &mut TestAppContext) {
            let server = MemoryStore::new().storage();
            let collection = project(&server).await;
            let (state, _) = open_copy(&server, collection.clone(), cx);
            state.update(cx, |state, cx| state.create_api(None, "first".into(), cx));
            cx.run_until_parked();
            let first = state
                .read_with(cx, |state, _| state.selected().cloned())
                .expect("selected");
            let api = editor(&state, cx);
            api.update(cx, |api, cx| api.set_uri("http://example.test/".into(), cx));
            api.update(cx, |api, cx| api.send_or_cancel(Answers::default(), cx));
            cx.run_until_parked();
            let shown = api
                .read_with(cx, |api, _| api.shown_response().cloned())
                .expect("answered");
            assert!(shown.waits_for_sync() && !shown.kept());

            // Another API, and back: the answer is still there.
            state.update(cx, |state, cx| state.create_api(None, "second".into(), cx));
            cx.run_until_parked();
            state.update(cx, |state, cx| state.select(&first, cx));
            cx.run_until_parked();
            let back = editor(&state, cx).read_with(cx, |api, _| api.responses().to_vec());
            assert_eq!(
                back.iter().map(|response| response.id.clone()).collect::<Vec<_>>(),
                std::slice::from_ref(&shown.id)
            );

            // Synced: the server's history has it, and so does the panel.
            state.update(cx, |state, cx| state.sync(false, cx));
            cx.run_until_parked();
            let kept = server
                .history()
                .list(collection.id.clone(), first.clone())
                .await
                .expect("the API is on the server");
            assert_eq!(kept.len(), 1);
            let shown = editor(&state, cx)
                .read_with(cx, |api, _| api.shown_response().cloned())
                .expect("shown");
            assert!(shown.kept(), "the note goes");
            assert_eq!(shown.id, kept[0].id);
        }

        #[gpui_kit::test]
        async fn a_viewer_has_nothing_to_sync(cx: &mut TestAppContext) {
            let memory = MemoryStore::new();
            let server = memory.storage();
            let collection = project(&server).await;
            let (editor_state, _) = open_copy(&server, collection.clone(), cx);
            editor_state.update(cx, |state, cx| state.create_api(None, "mine".into(), cx));
            cx.run_until_parked();
            drop(editor_state);

            // The same server, which now says the reader only views it.
            let viewing = Storage::new(
                Arc::new(Viewing(memory.clone())),
                Arc::new(memory.clone()),
                Arc::new(memory.clone()),
                Arc::new(memory.clone()),
                Arc::new(memory.clone()),
                Arc::new(memory),
            );
            let (state, told) = open_copy(&viewing, collection, cx);
            assert!(row_names(&state, cx).is_empty(), "the server's, not the draft");
            assert!(!state.read_with(cx, |state, _| state.syncs()));
            assert!(!unsynced(&state, cx));
            assert!(told.borrow().is_empty());
        }
    }
}
