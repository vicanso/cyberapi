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

use super::copy::{self, Pushed, ServerCopy};
use super::{ApiState, AppStore, Load, run_serial, run_store};
use crate::drafts_local::Draft;
use crate::pacing;
use async_lock::Mutex;
use cyberapi_core::ItemId;
use cyberapi_core::import::{ImportBundle, ImportItem};
use cyberapi_core::model::{ApiSetting, Collection, Folder, Variable, VariableCategory};
use cyberapi_core::prefs::{RequestTab, WorkspacePrefs};
use cyberapi_core::sync::{self, ConfigAt, Moment, Step};
use cyberapi_core::tree::{Placement, Row, Tree, TreeData};
use cyberapi_service::Sender;
use cyberapi_store::{Storage, StoreError};
use futures::StreamExt as _;
use gpui_kit::{AppContext as _, Context, Entity, EventEmitter, Task};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

/// What the view should tell the reader about.
#[derive(Clone, Debug)]
pub enum WorkspaceEvent {
    /// A change the reader asked for did not happen.
    Failed(StoreError),
    /// An import was written: how many APIs and folders.
    Imported { apis: usize, folders: usize },
    /// F3.9: an API's cURL command, for the clipboard.
    Curl(String),
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
}

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
    /// A push waiting for the writes on their way: over the server's
    /// whatever it has, or only at the copy's base.
    push: Option<bool>,
    _news: Task<()>,
}

pub struct WorkspaceState {
    sender: Sender,
    collection: Collection,
    data: TreeData,
    tree: Tree,
    prefs: WorkspacePrefs,
    load: Load,
    keyword: String,
    /// The tree's changes, one at a time.
    gate: Arc<Mutex<()>>,
    /// The saves of the reader's state, one at a time in the order made:
    /// each writes the whole document.
    saves: Arc<Mutex<()>>,
    /// The selected API's editor, while one is selected.
    api: Option<Entity<ApiState>>,
    /// The API the reader just made, until its editor is up: its URI is
    /// what they type next.
    created: Option<ItemId>,
    /// The project's environment variables, for the URI's prefix (F4.4).
    environment: Vec<Variable>,
    /// The project's global request headers, for the file an `openFile`
    /// in one of them asks for before a send.
    global_headers: Vec<Variable>,
    /// A server project's working copy; `None` for a project of this
    /// machine's, whose changes are written at once.
    copy: Option<CopyState>,
    _poll: Option<Task<()>>,
}

impl EventEmitter<WorkspaceEvent> for WorkspaceState {}

impl WorkspaceState {
    /// F2.7: the tree and the reader's state of it. `sender` holds the
    /// stores and the engine the selected API sends with.
    pub fn new(sender: Sender, collection: Collection, cx: &mut Context<Self>) -> Self {
        Self::with_poll(sender, collection, None, pacing::REVISION_POLL, cx)
    }

    /// A server project, edited in a working copy that waits for the reader
    /// to sync it (server-sync §3–4); `scope` is whose drafts.
    pub fn server_copy(sender: Sender, collection: Collection, scope: String, cx: &mut Context<Self>) -> Self {
        Self::with_poll(sender, collection, Some(scope), pacing::REVISION_POLL, cx)
    }

    /// Asks every `poll` whether another reader changed the project.
    fn with_poll(
        mut sender: Sender,
        collection: Collection,
        scope: Option<String>,
        poll: Option<Duration>,
        cx: &mut Context<Self>,
    ) -> Self {
        let copy = scope.map(|scope| {
            let (copy, mut news) = ServerCopy::new(&collection, scope);
            let server = sender.storage.clone();
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
                push: None,
                _news: news,
            }
        });
        let mut state = Self {
            sender,
            collection,
            data: TreeData::default(),
            tree: Tree::default(),
            prefs: WorkspacePrefs::default(),
            load: Load::Loading,
            keyword: String::new(),
            gate: Arc::default(),
            saves: Arc::default(),
            api: None,
            created: None,
            environment: Vec::new(),
            global_headers: Vec::new(),
            copy,
            _poll: poll.map(|every| Self::poll(every, cx)),
        };
        state.reload(cx);
        state
    }

    fn poll(every: Duration, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut beats = 0u32;
            loop {
                cx.background_executor().timer(every).await;
                beats = beats.wrapping_add(1);
                if pacing::page_hidden() && !beats.is_multiple_of(pacing::HIDDEN_EVERY) {
                    continue;
                }
                let Ok(call) = this.update(cx, |this, _| {
                    this.sender.storage.collections().get(this.collection.id.clone())
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

    /// Opening a server project (server-sync §3.1): the server's content,
    /// or the draft of changes it has not had. A viewer changes nothing,
    /// so no draft counts for them.
    fn open_copy(&mut self, cx: &mut Context<Self>) {
        let Some(copy) = &self.copy else { return };
        let draft = if self.editable() { copy.copy.draft() } else { None };
        let call = copy.server.tree().config(self.collection.id.clone());
        run_serial(cx, &self.gate, call, move |this, result, cx| {
            this.copy_opened(result, draft, cx)
        });
    }

    fn copy_opened(&mut self, result: Result<ConfigAt, StoreError>, draft: Option<Draft>, cx: &mut Context<Self>) {
        let collection = self.collection.clone();
        let Some(copy) = self.copy.as_mut() else { return };
        let held = match (result, draft) {
            (Ok(at), Some(draft)) => {
                let step = sync::step(Moment::Look, draft.base, at.revision, true);
                copy.behind = step == Step::KeepUnsyncedBehind;
                copy.told = true;
                cx.emit(WorkspaceEvent::KeptUnsynced { behind: copy.behind });
                Ok((draft, true))
            }
            (Ok(at), None) => Ok((
                Draft {
                    base: Some(at.revision),
                    config: at.config,
                },
                false,
            )),
            // Not reachable, or not the reader's any more: what this
            // browser kept shows, as long as there is that.
            (Err(error), Some(draft)) => {
                copy.told = true;
                cx.emit(WorkspaceEvent::NotPulled(error));
                Ok((draft, true))
            }
            (Err(error), None) => Err(error),
        };
        let held = held.and_then(|(draft, unsynced)| {
            let (name, description) = (draft.config.name.clone(), draft.config.description.clone());
            copy.copy.hold(collection, draft.config, draft.base, unsynced)?;
            Ok((name, description))
        });
        match held {
            Ok((name, description)) => {
                self.collection.name = name;
                self.collection.description = description;
                self.load_all(cx);
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
                Err(error) => warn!(%error, "the server's project could not be pulled"),
            }
        });
    }

    fn keeps_changes(&self, cx: &gpui_kit::App) -> bool {
        self.copy.as_ref().is_some_and(|copy| copy.copy.unsynced())
            || self.api.as_ref().is_some_and(|api| api.read(cx).has_unsaved())
    }

    /// The server's content becomes the copy, level with it. The editor
    /// starts again from it when its request changed — or always, when the
    /// reader gave their changes up (`forget`).
    fn take(&mut self, at: ConfigAt, forget: bool, cx: &mut Context<Self>) {
        let Some(copy) = self.copy.as_mut() else { return };
        let data = at.config.tree();
        let (name, description) = (at.config.name.clone(), at.config.description.clone());
        let collection = Collection {
            name: name.clone(),
            description: description.clone(),
            ..self.collection.clone()
        };
        if let Err(error) = copy.copy.hold(collection, at.config, Some(at.revision), false) {
            return cx.emit(WorkspaceEvent::Failed(error));
        }
        copy.behind = false;
        self.collection.name = name;
        self.collection.description = description;
        let editor = self.api.as_ref().map(|api| api.read(cx).api().clone());
        let request = |data: &TreeData, id: &ItemId| {
            data.apis
                .iter()
                .find(|api| &api.id == id)
                .map(|api| api.request.clone())
        };
        if let Some(id) = editor
            && (forget || request(&self.data, &id) != request(&data, &id))
            && let Some(api) = self.api.take()
        {
            api.update(cx, |api, _| api.forget_unsaved());
        }
        if self.load == Load::Loaded {
            self.set_data(data, cx);
            self.refresh_kept_variables(cx);
        } else {
            self.load_all(cx);
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
        if !copy.copy.kept() && !copy.told_not_kept {
            copy.told_not_kept = true;
            cx.emit(WorkspaceEvent::NotKept);
        }
        self.push_when_settled(cx);
        cx.notify();
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
        if let Some(api) = self.api.clone() {
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
        let (config, base, landed) = match copy.copy.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                copy.busy = false;
                return cx.emit(WorkspaceEvent::Failed(error));
            }
        };
        let server = copy.server.clone();
        let call = if over {
            copy::push_over(server, id, config)
        } else {
            copy::push(server, id, config, base)
        };
        run_store(cx, call, move |this, result, cx| this.pushed(result, landed, cx));
    }

    fn pushed(&mut self, result: Result<Pushed, StoreError>, landed: u64, cx: &mut Context<Self>) {
        let Some(copy) = self.copy.as_mut() else { return };
        copy.busy = false;
        match result {
            Ok(Pushed::Written(revision)) => {
                copy.copy.pushed(revision, landed);
                copy.behind = false;
                cx.emit(WorkspaceEvent::Synced);
            }
            Ok(Pushed::Conflict) => cx.emit(WorkspaceEvent::Conflict),
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
            match result {
                Ok(at) => {
                    this.take(at, true, cx);
                    cx.emit(WorkspaceEvent::TookServers);
                }
                Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
            }
            cx.notify();
        });
        cx.notify();
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

    /// F4.4: the enabled environment variables, by name.
    pub fn environments(&self) -> impl Iterator<Item = &Variable> {
        self.environment.iter().filter(|variable| variable.enabled)
    }

    /// The global request headers, enabled or not.
    pub fn global_headers(&self) -> &[Variable] {
        &self.global_headers
    }

    /// Reads the variables kept here again: the project changed.
    fn refresh_kept_variables(&mut self, cx: &mut Context<Self>) {
        self.refresh_variables(VariableCategory::Environment, cx);
        self.refresh_variables(VariableCategory::GlobalRequestHeader, cx);
    }

    /// Reads a category's variables again, when it is one kept here: they
    /// changed.
    pub fn refresh_variables(&mut self, category: VariableCategory, cx: &mut Context<Self>) {
        if !matches!(
            category,
            VariableCategory::Environment | VariableCategory::GlobalRequestHeader
        ) {
            return;
        }
        let call = self
            .sender
            .storage
            .variables()
            .list(self.collection.id.clone(), category);
        run_store(cx, call, move |this, result, cx| match result {
            Ok(variables) => {
                match category {
                    VariableCategory::Environment => this.environment = variables,
                    _ => this.global_headers = variables,
                }
                cx.notify();
            }
            Err(error) => warn!(%error, ?category, "variables could not be read"),
        });
    }

    /// F4.12: the tab the selected API's editor shows.
    pub fn tab(&self, cx: &gpui_kit::App) -> Option<RequestTab> {
        let api = self.api.as_ref()?.read(cx);
        Some(self.prefs.tab(api.api(), api.request().method))
    }

    pub fn set_tab(&mut self, tab: RequestTab, cx: &mut Context<Self>) {
        let Some(api) = self.api.as_ref() else { return };
        let (id, method) = {
            let api = api.read(cx);
            (api.api().clone(), api.request().method)
        };
        if self.prefs.tab(&id, method) != tab {
            self.prefs.set_tab(&id, method, tab);
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// F4.12: a new method opens on its default tab — the kept one goes.
    pub fn method_changed(&mut self, api: &ItemId, cx: &mut Context<Self>) {
        if self.prefs.tabs.remove(api).is_some() {
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// Opens the editor of the selected API, or closes it when none is
    /// selected (or the selected one is gone). The editor of an API that
    /// stays selected is kept, with what it holds.
    fn sync_api(&mut self, cx: &mut Context<Self>) {
        let selected = self.selected_api().cloned();
        let current = self.api.as_ref().map(|api| api.read(cx).api().clone());
        match selected {
            Some(api) if current.as_ref() != Some(&api.id) => {
                let (sender, workspace) = (self.sender.clone(), cx.entity().downgrade());
                let editable = self.editable();
                self.api = Some(cx.new(|cx| ApiState::new(sender, workspace, &api, editable, cx)));
            }
            Some(_) => {}
            None => self.api = None,
        }
    }

    /// An API's request was saved: the tree's copy follows (its method label).
    pub fn api_saved(&mut self, setting: ApiSetting, cx: &mut Context<Self>) {
        if let Some(api) = self.data.apis.iter_mut().find(|api| api.id == setting.id) {
            *api = setting;
            self.tree = Tree::build(&self.data);
            cx.notify();
        }
    }

    pub fn load(&self) -> &Load {
        &self.load
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    pub fn keyword(&self) -> &str {
        &self.keyword
    }

    pub fn selected(&self) -> Option<&ItemId> {
        self.prefs.selected.as_ref()
    }

    /// The selected API's record, when it still exists.
    pub fn selected_api(&self) -> Option<&ApiSetting> {
        let selected = self.prefs.selected.as_ref()?;
        self.data.apis.iter().find(|api| &api.id == selected)
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
    pub fn rows(&self) -> Vec<Row> {
        let expanded: HashSet<ItemId> = self.prefs.expanded.iter().cloned().collect();
        self.tree.rows(&expanded, &self.keyword)
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
                    this.set_data(data, cx);
                    this.load = Load::Loaded;
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
            cx.notify();
        });
    }

    fn set_data(&mut self, data: TreeData, cx: &mut Context<Self>) {
        self.data = data;
        self.tree = Tree::build(&self.data);
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

    /// F3.8: an API is selected; a folder is opened or closed instead. The
    /// folders it sits in open, so a pin (F12.2) shows its row.
    pub fn select(&mut self, id: &ItemId, cx: &mut Context<Self>) {
        let mut changed = self.prefs.selected.as_ref() != Some(id);
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

    /// F12.1 / F12.3. Only this project's APIs, once each.
    pub fn set_pinned(&mut self, id: &ItemId, pinned: bool, cx: &mut Context<Self>) {
        let present = self.prefs.pinned.contains(id);
        if pinned && !present && self.data.apis.iter().any(|api| &api.id == id) {
            self.prefs.pinned.push(id.clone());
        } else if !pinned && present {
            self.prefs.pinned.retain(|pin| pin != id);
        } else {
            return;
        }
        self.save_prefs(cx);
        cx.notify();
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
                this.tree = Tree::build(&this.data);
                this.open_folder(parent.as_ref());
                this.created = Some(inserted.item.id.clone());
                this.prefs.selected = Some(inserted.item.id);
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
                this.tree = Tree::build(&this.data);
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
                this.tree = Tree::build(&this.data);
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// F3.7.
    pub fn move_item(&mut self, id: ItemId, to: Placement, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .tree()
            .move_item(self.collection.id.clone(), id, to.clone());
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(change) => {
                this.data.apply(&change);
                this.tree = Tree::build(&this.data);
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
        let request = match &self.api {
            Some(open) if open.read(cx).api() == id => open.read(cx).request().clone(),
            _ => stored.request.clone(),
        };
        let items = vec![ImportItem::Api { name, request }];
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
                this.tree = Tree::build(&this.data);
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
    /// environment variables — those with a name or a value.
    pub fn import(&mut self, parent: Option<ItemId>, bundle: ImportBundle, cx: &mut Context<Self>) {
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
                this.data.folders.extend(imported.folders);
                this.data.apis.extend(imported.apis);
                this.data.apply(&imported.change);
                this.tree = Tree::build(&this.data);
                this.open_folder(parent.as_ref());
                this.save_prefs(cx);
                this.add_environment(environment, cx);
                cx.emit(WorkspaceEvent::Imported { apis, folders });
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
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

    /// F3.9 / F7.8: the API's cURL command, prepared as sending would
    /// prepare its stored request.
    pub fn copy_curl(&mut self, id: &ItemId, cx: &mut Context<Self>) {
        let Some(api) = self.data.apis.iter().find(|api| &api.id == id) else {
            return;
        };
        let call = cyberapi_service::curl(self.sender.clone(), self.collection.id.clone(), api.request.clone());
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(call).await;
            let _ = this.update(cx, |_, cx| match result {
                Ok(command) => cx.emit(WorkspaceEvent::Curl(command)),
                Err(failure) => cx.emit(WorkspaceEvent::CurlFailed(failure.to_string())),
            });
        })
        .detach();
    }

    pub fn export(&self, folder: Option<&ItemId>) -> (String, String) {
        let name = folder
            .and_then(|id| self.tree.get(id))
            .map(|node| node.name().to_string())
            .unwrap_or_else(|| self.collection.name.clone());
        (
            format!("cyberapi-{name}.json"),
            cyberapi_core::import::export(&self.data, folder),
        )
    }

    /// F3.6. The view asks first. What was removed leaves the reader's state
    /// too: selection, pins, open folders.
    pub fn delete(&mut self, id: ItemId, cx: &mut Context<Self>) {
        let call = self.sender.storage.tree().delete(self.collection.id.clone(), id);
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(removal) => {
                this.data.apply_removal(&removal);
                this.tree = Tree::build(&this.data);
                let gone = |id: &ItemId| removal.folders.contains(id) || removal.apis.contains(id);
                if this.prefs.selected.as_ref().is_some_and(gone) {
                    this.prefs.selected = None;
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
    use cyberapi_core::model::{HttpMethod, HttpRequest, KvParam};
    use cyberapi_core::prefs::Timeouts;
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
            user_agent: "CyberAPI/test".into(),
            timeouts: Timeouts::default(),
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
                let environment: Vec<Variable> = state.environments().cloned().collect();
                (names(&environment), names(state.global_headers()))
            })
        };
        assert_eq!(names(&state, cx), (vec!["dev".into()], vec!["X-Sign".into()]));
        let needs = state.read_with(cx, |state, _| {
            cyberapi_core::send::needs(&HttpRequest::default(), state.global_headers())
        });
        assert!(needs.picks_file);

        let new = variable(VariableCategory::GlobalRequestHeader, "X-Trace", "1");
        storage.variables().create(new).await.expect("create");
        state.update(cx, |state, cx| {
            state.refresh_variables(VariableCategory::GlobalRequestHeader, cx)
        });
        cx.run_until_parked();
        assert_eq!(names(&state, cx).1, ["X-Sign", "X-Trace"]);
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
            assert_eq!(state.selected_api().map(|api| api.name.as_str()), Some("login"));
        });
        // Its editor asks once whether it is new (the URI takes the keys).
        state.update(cx, |state, _| {
            assert!(state.take_created(&login));
            assert!(!state.take_created(&login));
        });
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

        // Chosen from its pin, the API shows again.
        again.update(cx, |state, cx| state.select(&login, cx));
        assert_eq!(row_names(&again, cx), ["users", "  login"]);
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
        api.update(cx, |api, cx| api.send_or_cancel(None, cx));
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
        api.update(cx, |api, cx| api.send_or_cancel(None, cx));
        cx.run_until_parked();
        assert!(api.read_with(cx, |api, _| api.sending_since().is_some()));
        api.update(cx, |api, cx| api.send_or_cancel(None, cx));
        assert!(
            api.read_with(cx, |api, _| api.sending_since().is_some()),
            "a double click is one click"
        );
        std::thread::sleep(Duration::from_millis(210));
        api.update(cx, |api, cx| api.send_or_cancel(None, cx));
        api.read_with(cx, |api, _| {
            assert!(api.sending_since().is_none(), "cancelled");
            assert_eq!(api.responses().len(), 1, "a cancelled request leaves nothing");
        });
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
        api.update(cx, |api, cx| api.send_or_cancel(None, cx));
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
        api.update(cx, |api, cx| api.send_or_cancel(None, cx));
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

    #[gpui_kit::test]
    async fn the_tab_is_kept_per_api(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_api(None, "a".into(), cx));
        cx.run_until_parked();
        assert_eq!(state.read_with(cx, |state, cx| state.tab(cx)), Some(RequestTab::Query));
        state.update(cx, |state, cx| state.set_tab(RequestTab::Auth, cx));
        cx.run_until_parked();
        let again = open(&storage, collection.clone(), cx);
        assert_eq!(again.read_with(cx, |state, cx| state.tab(cx)), Some(RequestTab::Auth));

        // F4.12: another method opens on its default tab.
        let api = editor(&state, cx);
        api.update(cx, |api, cx| api.set_method(HttpMethod::Post, cx));
        cx.run_until_parked();
        assert_eq!(state.read_with(cx, |state, cx| state.tab(cx)), Some(RequestTab::Body));
        let again = open(&storage, collection, cx);
        assert_eq!(again.read_with(cx, |state, cx| state.tab(cx)), Some(RequestTab::Body));
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

        /// A server project opened as a page opens it, and what it said.
        fn open_copy(
            server: &Storage,
            collection: Collection,
            cx: &mut TestAppContext,
        ) -> (Entity<WorkspaceState>, Rc<RefCell<Vec<String>>>) {
            let state = cx.new(|cx| {
                WorkspaceState::with_poll(sender(server, true), collection, Some(READER.into()), Some(BEAT), cx)
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
            let draft = drafts_local::get(READER, &collection.id).expect("a draft");
            assert_eq!(draft.base, Some(collection.revision));
            assert_eq!(draft.config.apis.len(), 1);
            // A beat keeps it: the server has not moved.
            beat(cx);
            assert_eq!(row_names(&state, cx), ["users", "mine"]);

            // A refresh finds it.
            drop(state);
            let (again, told) = open_copy(&server, collection.clone(), cx);
            assert_eq!(row_names(&again, cx), ["users", "mine"]);
            assert_eq!(told.borrow().as_slice(), ["KeptUnsynced { behind: false }"]);

            // The editor's typing goes too, without its pause.
            let mine = find(&again, "mine", cx);
            again.update(cx, |state, cx| state.select(&mine, cx));
            cx.run_until_parked();
            editor(&again, cx).update(cx, |api, cx| api.set_headers(vec![header("X-A", "1")], cx));
            again.update(cx, |state, cx| state.sync(false, cx));
            cx.run_until_parked();
            assert_eq!(server_rows(&server, &collection), ["mine"]);
            assert_eq!(stored(&server, &collection, &mine).headers, [header("X-A", "1")]);
            assert!(!unsynced(&again, cx));
            assert_eq!(drafts_local::get(READER, &collection.id), None, "the draft goes");
            assert!(told.borrow().iter().any(|event| event == "Synced"));
            // Its own push is no news on the next beat.
            beat(cx);
            assert!(!again.read_with(cx, |state, _| state.server_moved()));
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

        #[gpui_kit::test]
        async fn a_viewer_has_nothing_to_sync(cx: &mut TestAppContext) {
            let server = MemoryStore::new().storage();
            let collection = project(&server).await;
            let (editor_state, _) = open_copy(&server, collection.clone(), cx);
            editor_state.update(cx, |state, cx| state.create_api(None, "mine".into(), cx));
            cx.run_until_parked();
            drop(editor_state);

            let viewer = Collection {
                role: CollectionRole::Viewer,
                ..collection.clone()
            };
            let (state, told) = open_copy(&server, viewer, cx);
            assert!(row_names(&state, cx).is_empty(), "the server's, not the draft");
            assert!(!state.read_with(cx, |state, _| state.syncs()));
            assert!(!unsynced(&state, cx));
            assert!(told.borrow().is_empty());
        }
    }
}
