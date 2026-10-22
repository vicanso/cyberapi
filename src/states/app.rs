//! What the whole application shares: the stores, where the reader is, and
//! their settings.

use super::{ServerState, ServerStatus, run_serial, run_store};
use crate::json_view_local::JsonView;
use crate::{Extras, this_build};
use async_lock::Mutex;
use cyberapi_core::collection::SortOrder;
use cyberapi_core::directory::Me;
use cyberapi_core::model::Collection;
use cyberapi_core::prefs::{AppPrefs, ColumnWidths, ThemeMode, Timeouts, WindowPrefs};
#[cfg(not(target_family = "wasm"))]
use cyberapi_core::prefs::{MonitorPlacement, remember_monitor};
use cyberapi_core::{CollectionId, Timestamp};
use cyberapi_http::HttpEngine;
use cyberapi_service::Sender;
use cyberapi_service::backup::{self, BackupFailure};
use cyberapi_store::{BackupStore, DirectoryStore, Storage, StoreError, StoreFuture};
use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

/// The stores, the engine and the application state, installed once by
/// `launch`.
#[derive(Clone)]
pub struct AppStore {
    storage: Storage,
    engine: Arc<dyn HttpEngine>,
    extras: Extras,
    state: Entity<AppState>,
    /// The bridge the desktop signs in to (`docs/server-sync.md` step 5);
    /// a page is signed in already, by its own.
    server: Option<Entity<ServerState>>,
}

impl Global for AppStore {}

impl AppStore {
    pub fn init(storage: Storage, engine: Arc<dyn HttpEngine>, prefs: AppPrefs, extras: Extras, cx: &mut App) {
        let state = cx.new(|_| AppState {
            route: Route::Dashboard,
            prefs,
            // The signed-in account is read next, and replaces this on a page.
            json_view: crate::json_view_local::get(&crate::pins_local::scope(None)),
            me: None,
            opening: None,
            save_task: None,
            saves: Arc::default(),
        });
        let server = extras.sign_out.is_none().then(|| {
            let engine = engine.clone();
            let copies = extras.copies.clone();
            cx.new(|cx| ServerState::new(engine, copies, cx))
        });
        cx.set_global(Self {
            storage,
            engine,
            extras,
            state: state.clone(),
            server,
        });
        state.update(cx, |state, cx| {
            state.reload_me(cx);
            state.reopen_last_page(cx);
        });
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// The desktop's backups (M13); a page has none.
    pub fn backups(&self) -> Option<&Arc<dyn BackupStore>> {
        self.extras.backups.as_ref()
    }

    /// The accounts and teams (§9.4): the page's, or — the desktop signed
    /// in to a bridge — that bridge's (`docs/server-sync.md` §1).
    pub fn directory(&self, cx: &App) -> Option<Arc<dyn DirectoryStore>> {
        if let Some(directory) = &self.extras.directory {
            return Some(directory.clone());
        }
        self.server.as_ref()?.read(cx).directory()
    }

    /// Whether an account is signed in: always in a page, and on the
    /// desktop while its bridge knows the session.
    pub fn has_account(&self, cx: &App) -> bool {
        self.signs_in()
            || self
                .server
                .as_ref()
                .is_some_and(|server| matches!(server.read(cx).status(), ServerStatus::SignedIn(_)))
    }

    /// The desktop's bridge, while signed in to it.
    pub fn server_address(&self, cx: &App) -> Option<String> {
        let server = self.server.as_ref()?.read(cx);
        server.account()?;
        Some(server.address().to_string())
    }

    /// Whether this build signs in (the browser).
    pub fn signs_in(&self) -> bool {
        self.extras.sign_out.is_some()
    }

    /// Whose drafts the projects' unsynced changes are, where the projects
    /// are the server's and edited in a working copy (`docs/server-sync.md`
    /// §4): the signed-in reader's, in a page. `None` where changes are
    /// written at once.
    pub fn draft_scope(&self, cx: &App) -> Option<String> {
        self.signs_in().then(|| self.state.read(cx).pin_scope())
    }

    /// What sending needs, with the reader's settings as they are now.
    pub fn sender(&self, cx: &App) -> Sender {
        Sender {
            storage: self.storage.clone(),
            engine: self.engine.clone(),
            files: this_build::files(),
            user_agent: this_build::user_agent(),
            timeouts: self.state.read(cx).prefs().timeouts,
        }
    }

    pub fn state(&self) -> &Entity<AppState> {
        &self.state
    }

    /// The desktop's bridge and who is signed in there.
    pub fn server(&self) -> Option<&Entity<ServerState>> {
        self.server.as_ref()
    }
}

/// The page the main window shows (F1.9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// The project list.
    Dashboard,
    /// A project's workspace. The project as it was when it was opened, for
    /// its name in the title bar.
    Workspace(Collection),
}

/// What the application tells its window.
pub enum AppEvent {
    /// F13.1: the backup is saved, there.
    BackedUp(String),
    /// F13.2: everything is the backup's now.
    Restored,
    BackupFailed(BackupFailure),
    /// The backup file could not be written or read: where, and why.
    FileFailed(String),
    /// F15.3: the history or the state of the projects is forgotten.
    Cleared,
    Failed(StoreError),
}

impl EventEmitter<AppEvent> for AppState {}

pub struct AppState {
    route: Route,
    prefs: AppPrefs,
    /// How JSON opens. On this machine, not in the settings document.
    json_view: JsonView,
    /// Who is signed in, and their teams: the browser's, once read.
    me: Option<Me>,
    /// A project to open once it is known who is signed in: their drafts
    /// are what it opens over.
    opening: Option<Collection>,
    /// A save waiting for a drag or a resize to settle.
    save_task: Option<Task<()>>,
    /// The saves of the settings, one at a time in the order made: each
    /// writes the whole document, so a later one landing first would be
    /// overwritten by an older.
    saves: Arc<Mutex<()>>,
}

impl AppState {
    pub fn route(&self) -> &Route {
        &self.route
    }

    pub fn prefs(&self) -> &AppPrefs {
        &self.prefs
    }

    /// How JSON opens, until a panel's Text / Tree switch says otherwise
    /// for the document on screen.
    pub fn json_view(&self) -> JsonView {
        self.json_view
    }

    pub fn me(&self) -> Option<&Me> {
        self.me.as_ref()
    }

    /// Reads who is signed in again: their teams change as they make,
    /// leave and are added to them.
    pub fn reload_me(&mut self, cx: &mut Context<Self>) {
        let Some(directory) = AppStore::global(cx).directory(cx) else {
            return;
        };
        run_store(cx, directory.me(), |this, result, cx| match result {
            Ok(me) => {
                this.me = Some(me);
                this.json_view = crate::json_view_local::get(&this.pin_scope());
                this.open_waiting(cx);
                cx.notify();
            }
            Err(error) => warn!(%error, "who is signed in could not be read"),
        });
    }

    /// Ends the browser's login; the page signs in again.
    /// The page's login ends and it signs in again; the desktop's ends
    /// at its bridge.
    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        let store = AppStore::global(cx);
        if let Some(sign_out) = store.extras.sign_out.clone() {
            return cx.background_spawn(sign_out()).detach();
        }
        if let Some(server) = store.server().cloned() {
            server.update(cx, |server, cx| server.sign_out(cx));
        }
    }

    /// Who is signed in, as the desktop's bridge says. A copy of a server
    /// project open that is no longer theirs closes.
    pub fn set_me(&mut self, me: Option<Me>, cx: &mut Context<Self>) {
        if self.me != me {
            self.me = me;
            cx.notify();
        }
        if let Route::Workspace(collection) = &self.route
            && AppStore::global(cx)
                .server()
                .is_some_and(|server| server.read(cx).hides(&collection.id))
        {
            self.go_home(cx);
        }
    }

    /// F2.7.
    pub fn open_collection(&mut self, collection: Collection, cx: &mut Context<Self>) {
        let store = AppStore::global(cx);
        let server = store.server().map(|server| server.read(cx));
        if (self.me.is_none() && store.signs_in()) || server.is_some_and(|server| !server.ready()) {
            self.opening = Some(collection);
            return;
        }
        // Another account's copy of a server project (`docs/server-sync.md`
        // §1) is not this reader's to open.
        if server.is_some_and(|server| server.hides(&collection.id)) {
            return self.go_home(cx);
        }
        self.remember_page(Some(collection.id.clone()), cx);
        self.route = Route::Workspace(collection);
        cx.notify();
    }

    /// The project asked for before who is signed in was known, now that
    /// it is.
    pub fn open_waiting(&mut self, cx: &mut Context<Self>) {
        if let Some(collection) = self.opening.take() {
            self.open_collection(collection, cx);
        }
    }

    /// F2.8.
    pub fn go_home(&mut self, cx: &mut Context<Self>) {
        self.remember_page(None, cx);
        self.route = Route::Dashboard;
        cx.notify();
    }

    /// F1.9: the page is kept, to open again at the next start.
    fn remember_page(&mut self, collection: Option<CollectionId>, cx: &mut Context<Self>) {
        if self.prefs.last_collection != collection {
            self.prefs.last_collection = collection;
            self.save_prefs(cx);
        }
    }

    /// F1.1: back to the project open when the app last left it, if it is
    /// still there to open.
    fn reopen_last_page(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.prefs.last_collection.clone() else {
            return;
        };
        let call = AppStore::global(cx).storage().collections().get(id);
        run_store(cx, call, |this, result, cx| match result {
            // Not if the reader has gone somewhere meanwhile.
            Ok(collection) if this.route == Route::Dashboard => this.open_collection(collection, cx),
            Ok(_) => {}
            Err(_) => this.remember_page(None, cx),
        });
    }

    /// F2.6: the choice is kept.
    pub fn set_collection_sort(&mut self, order: SortOrder, cx: &mut Context<Self>) {
        if self.prefs.collection_sort == order {
            return;
        }
        self.prefs.collection_sort = order;
        self.save_prefs(cx);
        cx.notify();
    }

    /// The reader's groups of the project list: `project` into `group`, or
    /// into none for a blank one.
    pub fn set_project_group(&mut self, project: CollectionId, group: &str, cx: &mut Context<Self>) {
        let group = group.trim();
        let changed = if group.is_empty() {
            self.prefs.project_groups.remove(&project).is_some()
        } else {
            self.prefs.project_groups.insert(project, group.to_string()).as_deref() != Some(group)
        };
        if changed {
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// The group `from` renamed `to`, or every project of it in none for a
    /// blank name; folded or not as it was.
    pub fn rename_group(&mut self, from: &str, to: &str, cx: &mut Context<Self>) {
        let to = to.trim();
        if from == to {
            return;
        }
        let groups = std::mem::take(&mut self.prefs.project_groups);
        self.prefs.project_groups = groups
            .into_iter()
            .filter_map(|(project, group)| {
                if group.trim() != from {
                    Some((project, group))
                } else {
                    (!to.is_empty()).then(|| (project, to.to_string()))
                }
            })
            .collect();
        let folded = self.prefs.folded_groups.iter().any(|group| group == from);
        self.prefs.folded_groups.retain(|group| group != from && group != to);
        if folded && !to.is_empty() {
            self.prefs.folded_groups.push(to.to_string());
        }
        self.save_prefs(cx);
        cx.notify();
    }

    /// A group of the project list folded or unfolded; `None` is the
    /// projects in none.
    pub fn toggle_group(&mut self, group: Option<&str>, cx: &mut Context<Self>) {
        let key = group.unwrap_or_default();
        match self.prefs.folded_groups.iter().position(|folded| folded == key) {
            Some(at) => {
                self.prefs.folded_groups.remove(at);
            }
            None => self.prefs.folded_groups.push(key.to_string()),
        }
        self.save_prefs(cx);
        cx.notify();
    }

    pub fn is_group_folded(&self, group: Option<&str>) -> bool {
        let key = group.unwrap_or_default();
        self.prefs.folded_groups.iter().any(|folded| folded == key)
    }

    /// A deleted project leaves its group, its pins and its draft.
    pub fn forget_project(&mut self, project: &CollectionId, cx: &mut Context<Self>) {
        crate::pins_local::remove(&self.pin_scope(), project);
        crate::drafts_local::remove(&self.pin_scope(), project);
        if self.prefs.project_groups.remove(project).is_some() {
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// The default for every JSON document. A panel switch does not call
    /// this: that choice lasts only while that document is on screen.
    pub fn set_json_view(&mut self, view: JsonView, cx: &mut Context<Self>) {
        if self.json_view == view {
            return;
        }
        if !crate::json_view_local::set(&self.pin_scope(), view) {
            warn!("the json view could not be saved");
            return;
        }
        self.json_view = view;
        cx.notify();
    }

    /// F14.1.
    pub fn set_theme(&mut self, theme: ThemeMode, cx: &mut Context<Self>) {
        if self.prefs.theme != theme {
            self.prefs.theme = theme;
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// F15.2: `None` follows the system. The window follows at once; there
    /// is no restart.
    pub fn set_language(&mut self, language: Option<String>, cx: &mut Context<Self>) {
        if self.prefs.language != language {
            self.prefs.language = language;
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// F14.3: seconds, `0` for the default.
    pub fn set_timeouts(&mut self, timeouts: Timeouts, cx: &mut Context<Self>) {
        if self.prefs.timeouts != timeouts {
            self.prefs.timeouts = timeouts;
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// F14.2: the window's size, applied by the view and at the next launch.
    pub fn set_window(&mut self, window: WindowPrefs, cx: &mut Context<Self>) {
        if self.prefs.window != window {
            self.prefs.window = window;
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// F15.3: every setting back to its default.
    pub fn reset_prefs(&mut self, cx: &mut Context<Self>) {
        // Where the reader is, and how they grouped their projects, are not
        // settings.
        self.prefs = AppPrefs {
            last_collection: self.prefs.last_collection.clone(),
            project_groups: std::mem::take(&mut self.prefs.project_groups),
            folded_groups: std::mem::take(&mut self.prefs.folded_groups),
            ..AppPrefs::default()
        };
        self.json_view = JsonView::Text;
        if !crate::json_view_local::set(&self.pin_scope(), JsonView::Text) {
            warn!("the json view could not be saved");
        }
        self.save_prefs(cx);
        cx.notify();
    }

    /// F15.3: every response the reader keeps, of every project.
    pub fn clear_history(&mut self, cx: &mut Context<Self>) {
        let call = AppStore::global(cx).storage().history().clear_all();
        self.clear(call, false, cx);
    }

    /// F15.3: the reader's state of every project — pins, open folders,
    /// the selection, tabs. Pins are on this machine, so they go with the
    /// stored state, once that clear has landed.
    pub fn clear_workspaces(&mut self, cx: &mut Context<Self>) {
        let call = AppStore::global(cx).storage().prefs().clear_workspaces();
        self.clear(call, true, cx);
    }

    /// Then the project list again, built anew: an open project would show
    /// what was cleared, and keep it.
    fn clear(&mut self, call: StoreFuture<()>, forget_pins: bool, cx: &mut Context<Self>) {
        run_store(cx, call, move |this, result, cx| match result {
            Ok(()) => {
                if forget_pins {
                    crate::pins_local::clear_scope(&this.pin_scope());
                }
                this.go_home(cx);
                cx.emit(AppEvent::Cleared);
            }
            Err(error) => cx.emit(AppEvent::Failed(error)),
        });
    }

    /// Whose pins, JSON view and drafts are kept on this machine.
    pub(crate) fn pin_scope(&self) -> String {
        crate::pins_local::scope(self.me().map(|me| me.account.id.as_str()))
    }

    /// The window moved or settled on a monitor. A zoom remembers the monitor
    /// and leaves the size from before it. Nothing is notified: a settings
    /// field is a snapshot and must not be rewritten while it is typed in.
    #[cfg(not(target_family = "wasm"))]
    pub fn note_monitor(&mut self, id: &str, placement: MonitorPlacement, zoomed: bool, cx: &mut Context<Self>) {
        let Some(next) = remember_monitor(&self.prefs.monitors, &self.prefs.last_monitor, id, placement, zoomed) else {
            return;
        };
        self.prefs.monitors = next.monitors;
        self.prefs.last_monitor = next.last_monitor;
        self.schedule_save(cx);
    }

    /// The document to write before the process exits, and the gate the
    /// other saves wait on. A save still waiting is dropped: this write is
    /// the last one.
    #[cfg(not(target_family = "wasm"))]
    pub fn prefs_to_flush(&mut self) -> (AppPrefs, Arc<Mutex<()>>) {
        self.save_task = None;
        (self.prefs.clone(), self.saves.clone())
    }

    /// F14.5: the workspace's columns, saved once a drag settles.
    pub fn set_columns(&mut self, columns: ColumnWidths, cx: &mut Context<Self>) {
        if self.prefs.columns == columns {
            return;
        }
        self.prefs.columns = columns;
        self.schedule_save(cx);
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            let _ = this.update(cx, |this, cx| this.save_prefs(cx));
        }));
    }

    /// F13.1: everything in a backup file in the downloads folder, named
    /// for now.
    pub fn back_up(&mut self, cx: &mut Context<Self>) {
        let Some(backups) = AppStore::global(cx).backups().cloned() else {
            return;
        };
        let at = Timestamp::new(chrono::Local::now().fixed_offset());
        let name = cyberapi_core::backup::file_name(&at);
        let saving = cx.background_spawn(async move {
            let bytes = backup::back_up(backups, at).await.map_err(AppEvent::BackupFailed)?;
            this_build::save_to_downloads(&name, &bytes)
                .map(AppEvent::BackedUp)
                .map_err(AppEvent::FileFailed)
        });
        cx.spawn(async move |this, cx| {
            let event = saving.await.unwrap_or_else(|failed| failed);
            let _ = this.update(cx, |_, cx| cx.emit(event));
        })
        .detach();
    }

    /// F13.2: everything replaced by the backup in `path`, then the project
    /// list again. Offered on that list only, so no open project has edits
    /// still to save over the backup's.
    pub fn restore(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(backups) = AppStore::global(cx).backups().cloned() else {
            return;
        };
        let restoring = cx.background_spawn(async move {
            let bytes = this_build::read_bytes(&path).map_err(AppEvent::FileFailed)?;
            backup::restore(backups, bytes)
                .await
                .map(|()| AppEvent::Restored)
                .map_err(AppEvent::BackupFailed)
        });
        cx.spawn(async move |this, cx| {
            let event = restoring.await.unwrap_or_else(|failed| failed);
            let _ = this.update(cx, |this, cx| {
                if matches!(event, AppEvent::Restored) {
                    this.go_home(cx);
                }
                cx.emit(event);
            });
        })
        .detach();
    }

    /// Settings are saved in the background; a failure costs the reader the
    /// choice next launch, not now, so it is only logged.
    fn save_prefs(&self, cx: &mut Context<Self>) {
        let call = AppStore::global(cx).storage().prefs().set_app(self.prefs.clone());
        run_serial(cx, &self.saves, call, |_, result, _| {
            if let Err(error) = result {
                warn!(%error, "the settings could not be saved");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::prefs::WorkspacePrefs;
    use cyberapi_core::response::HttpResponse;
    use cyberapi_http::{PreparedRequest, SendFuture};
    use cyberapi_store::{MemoryStore, NewCollection};
    use gpui_kit::TestAppContext;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct NoEngine;

    impl HttpEngine for NoEngine {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            Box::pin(std::future::pending())
        }
    }

    fn create(store: &MemoryStore, name: &str) -> Collection {
        pollster::block_on(store.storage().collections().create(NewCollection {
            name: name.into(),
            ..Default::default()
        }))
        .expect("create")
    }

    fn names(store: &MemoryStore) -> Vec<String> {
        pollster::block_on(store.storage().collections().list())
            .expect("list")
            .into_iter()
            .map(|collection| collection.name)
            .collect()
    }

    /// The app on `store`, and what its state has told the window.
    fn launch(store: &MemoryStore, cx: &mut TestAppContext) -> (Entity<AppState>, Rc<RefCell<Vec<String>>>) {
        let events: Rc<RefCell<Vec<String>>> = Rc::default();
        let seen = events.clone();
        let state = cx.update(|cx| {
            let extras = Extras {
                backups: Some(Arc::new(store.clone())),
                ..Default::default()
            };
            AppStore::init(store.storage(), Arc::new(NoEngine), AppPrefs::default(), extras, cx);
            let state = AppStore::global(cx).state().clone();
            cx.subscribe(&state, move |_, event, _| {
                seen.borrow_mut().push(match event {
                    AppEvent::BackedUp(_) => "backed up".into(),
                    AppEvent::Restored => "restored".into(),
                    AppEvent::BackupFailed(failure) => format!("failed: {failure}"),
                    AppEvent::FileFailed(message) => format!("file: {message}"),
                    AppEvent::Cleared => "cleared".into(),
                    AppEvent::Failed(error) => format!("store: {error}"),
                });
            })
            .detach();
            state
        });
        (state, events)
    }

    /// F1.1 / F1.9: the project open at the last exit opens again; one that
    /// is gone leaves the project list, and is forgotten.
    #[gpui_kit::test]
    async fn the_last_project_opens_again(cx: &mut TestAppContext) {
        let store = MemoryStore::new();
        let shop = create(&store, "shop");
        let start = |last: Option<CollectionId>, cx: &mut TestAppContext| {
            let state = cx.update(|cx| {
                let prefs = AppPrefs {
                    last_collection: last,
                    ..Default::default()
                };
                AppStore::init(store.storage(), Arc::new(NoEngine), prefs, Extras::default(), cx);
                AppStore::global(cx).state().clone()
            });
            cx.run_until_parked();
            state
        };
        let state = start(Some(shop.id.clone()), cx);
        assert_eq!(
            state.read_with(cx, |state, _| state.route().clone()),
            Route::Workspace(shop.clone())
        );
        state.update(cx, |state, cx| state.go_home(cx));
        cx.run_until_parked();
        let kept = pollster::block_on(store.storage().prefs().app()).expect("prefs");
        assert_eq!(kept.last_collection, None, "the list is where the reader left");

        let state = start(Some(CollectionId::new("gone")), cx);
        assert_eq!(state.read_with(cx, |state, _| state.route().clone()), Route::Dashboard);
        assert_eq!(
            state.read_with(cx, |state, _| state.prefs().last_collection.clone()),
            None
        );
    }

    /// The way JSON opens lives on this machine. Resetting settings puts it
    /// back, and never writes it into the settings document.
    #[gpui_kit::test]
    async fn the_json_view_stays_on_this_machine(cx: &mut TestAppContext) {
        let store = MemoryStore::new();
        let (state, _) = launch(&store, cx);
        state.update(cx, |state, cx| {
            state.set_json_view(crate::json_view_local::JsonView::Tree, cx)
        });
        cx.run_until_parked();
        let kept = pollster::block_on(store.storage().prefs().app()).expect("prefs");
        assert_eq!(kept, AppPrefs::default(), "the server's settings do not gain it");
        assert_eq!(crate::json_view_local::get(""), crate::json_view_local::JsonView::Tree);
        state.update(cx, |state, cx| state.reset_prefs(cx));
        cx.run_until_parked();
        assert_eq!(
            state.read_with(cx, |state, _| state.json_view()),
            crate::json_view_local::JsonView::Text
        );
        assert_eq!(crate::json_view_local::get(""), crate::json_view_local::JsonView::Text);
    }

    /// The reader's groups: set, renamed, folded, kept by a reset, and left
    /// by a deleted project.
    #[gpui_kit::test]
    async fn projects_go_in_the_readers_groups(cx: &mut TestAppContext) {
        let store = MemoryStore::new();
        let (shop, blog) = (create(&store, "shop"), create(&store, "blog"));
        let state = cx.update(|cx| {
            AppStore::init(
                store.storage(),
                Arc::new(NoEngine),
                AppPrefs::default(),
                Extras::default(),
                cx,
            );
            AppStore::global(cx).state().clone()
        });
        cx.run_until_parked();
        state.update(cx, |state, cx| {
            state.set_project_group(shop.id.clone(), " work ", cx);
            state.set_project_group(blog.id.clone(), "work", cx);
            state.toggle_group(Some("work"), cx);
            state.rename_group("work", "jobs", cx);
            state.set_project_group(blog.id.clone(), "", cx);
            state.reset_prefs(cx);
        });
        cx.run_until_parked();
        let kept = pollster::block_on(store.storage().prefs().app()).expect("prefs");
        assert_eq!(
            kept.project_groups.into_iter().collect::<Vec<_>>(),
            [(shop.id.clone(), "jobs".to_string())]
        );
        assert_eq!(kept.folded_groups, ["jobs"], "folded as it was, under its new name");

        assert!(crate::pins_local::set(
            "",
            &shop.id,
            &[cyberapi_core::ItemId::new("login")]
        ));
        state.update(cx, |state, cx| state.forget_project(&shop.id, cx));
        cx.run_until_parked();
        let kept = pollster::block_on(store.storage().prefs().app()).expect("prefs");
        assert!(kept.project_groups.is_empty());
        assert_eq!(
            crate::pins_local::get("", &shop.id),
            None,
            "a deleted project leaves no pins"
        );
    }

    /// F15.3: the history of every project goes, and the state of every
    /// project, each on its own; the window is back on the project list.
    #[gpui_kit::test]
    async fn the_readers_history_and_state_clear(cx: &mut TestAppContext) {
        let store = MemoryStore::new();
        let shop = create(&store, "shop");
        let storage = store.storage();
        let api = pollster::block_on(storage.tree().create_api(shop.id.clone(), None, "ping".into()))
            .expect("an API")
            .item
            .id;
        pollster::block_on(storage.history().add(HttpResponse {
            collection: shop.id.clone(),
            api: api.clone(),
            status: Some(200),
            ..Default::default()
        }))
        .expect("a response");
        let pinned = WorkspacePrefs {
            pinned: vec![api.clone()],
            ..Default::default()
        };
        pollster::block_on(storage.prefs().set_workspace(shop.id.clone(), pinned.clone())).expect("pins");
        assert!(crate::pins_local::set("", &shop.id, std::slice::from_ref(&api)));

        let (state, events) = launch(&store, cx);
        state.update(cx, |state, cx| state.open_collection(shop.clone(), cx));
        state.update(cx, |state, cx| state.clear_history(cx));
        cx.run_until_parked();
        assert_eq!(*events.borrow(), ["cleared"]);
        assert_eq!(state.read_with(cx, |state, _| state.route().clone()), Route::Dashboard);
        let history = pollster::block_on(storage.history().list(shop.id.clone(), api.clone())).expect("list");
        assert!(history.is_empty());
        let kept = pollster::block_on(storage.prefs().workspace(shop.id.clone())).expect("prefs");
        assert_eq!(kept, pinned, "the pins are another thing to clear");
        assert_eq!(
            crate::pins_local::get("", &shop.id),
            Some(vec![api.clone()]),
            "clearing history leaves the pins on this machine"
        );

        state.update(cx, |state, cx| state.clear_workspaces(cx));
        cx.run_until_parked();
        let kept = pollster::block_on(storage.prefs().workspace(shop.id.clone())).expect("prefs");
        assert_eq!(kept, WorkspacePrefs::default());
        assert_eq!(crate::pins_local::get("", &shop.id), None);
    }

    #[gpui_kit::test]
    async fn a_restore_replaces_everything_and_goes_home(cx: &mut TestAppContext) {
        let backed_up = MemoryStore::new();
        create(&backed_up, "shop");
        let at = Timestamp::new(chrono::Local::now().fixed_offset());
        let bytes = pollster::block_on(backup::back_up(Arc::new(backed_up), at)).expect("backed up");
        let dir = tempfile::tempdir().expect("a directory");
        let file = dir.path().join("backup.zip");
        std::fs::write(&file, bytes).expect("write");

        let store = MemoryStore::new();
        let mine = create(&store, "mine");
        let (state, events) = launch(&store, cx);
        state.update(cx, |state, cx| state.open_collection(mine, cx));
        state.update(cx, |state, cx| state.restore(file.clone(), cx));
        cx.run_until_parked();
        assert_eq!(*events.borrow(), ["restored"]);
        assert_eq!(state.read_with(cx, |state, _| state.route().clone()), Route::Dashboard);
        assert_eq!(names(&store), ["shop"]);

        // Not a backup: nothing changes, and the reader is told why.
        std::fs::write(&file, b"not a zip").expect("write");
        state.update(cx, |state, cx| state.restore(file.clone(), cx));
        cx.run_until_parked();
        assert!(events.borrow()[1].starts_with("failed: "), "{:?}", events.borrow());
        assert_eq!(names(&store), ["shop"]);
    }
}
