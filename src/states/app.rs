//! What the whole application shares: the stores, where the reader is, and
//! their settings.

use super::run_store;
use crate::{Extras, this_build};
use cyberapi_core::Timestamp;
use cyberapi_core::collection::SortOrder;
use cyberapi_core::directory::Me;
use cyberapi_core::model::Collection;
use cyberapi_core::prefs::{AppPrefs, ColumnWidths, ThemeMode, Timeouts, WindowPrefs};
use cyberapi_http::HttpEngine;
use cyberapi_service::Sender;
use cyberapi_service::backup::{self, BackupFailure};
use cyberapi_store::{BackupStore, DirectoryStore, Storage};
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
}

impl Global for AppStore {}

impl AppStore {
    pub fn init(storage: Storage, engine: Arc<dyn HttpEngine>, prefs: AppPrefs, extras: Extras, cx: &mut App) {
        let state = cx.new(|_| AppState {
            route: Route::Dashboard,
            prefs,
            me: None,
            save_task: None,
        });
        cx.set_global(Self {
            storage,
            engine,
            extras,
            state: state.clone(),
        });
        state.update(cx, |state, cx| state.reload_me(cx));
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

    /// The browser's accounts and teams (§9.4); the desktop has none.
    pub fn directory(&self) -> Option<&Arc<dyn DirectoryStore>> {
        self.extras.directory.as_ref()
    }

    /// Whether this build signs in (the browser).
    pub fn signs_in(&self) -> bool {
        self.extras.sign_out.is_some()
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
}

impl EventEmitter<AppEvent> for AppState {}

pub struct AppState {
    route: Route,
    prefs: AppPrefs,
    /// Who is signed in, and their teams: the browser's, once read.
    me: Option<Me>,
    /// A save waiting for a drag to settle.
    save_task: Option<Task<()>>,
}

impl AppState {
    pub fn route(&self) -> &Route {
        &self.route
    }

    pub fn prefs(&self) -> &AppPrefs {
        &self.prefs
    }

    pub fn me(&self) -> Option<&Me> {
        self.me.as_ref()
    }

    /// Reads who is signed in again: their teams change as they make,
    /// leave and are added to them.
    pub fn reload_me(&mut self, cx: &mut Context<Self>) {
        let Some(directory) = AppStore::global(cx).directory().cloned() else {
            return;
        };
        run_store(cx, directory.me(), |this, result, cx| match result {
            Ok(me) => {
                this.me = Some(me);
                cx.notify();
            }
            Err(error) => warn!(%error, "who is signed in could not be read"),
        });
    }

    /// Ends the browser's login; the page signs in again.
    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        if let Some(sign_out) = AppStore::global(cx).extras.sign_out.clone() {
            cx.background_spawn(sign_out()).detach();
        }
    }

    /// F2.7.
    pub fn open_collection(&mut self, collection: Collection, cx: &mut Context<Self>) {
        self.route = Route::Workspace(collection);
        cx.notify();
    }

    /// F2.8.
    pub fn go_home(&mut self, cx: &mut Context<Self>) {
        self.route = Route::Dashboard;
        cx.notify();
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
        self.prefs = AppPrefs::default();
        self.save_prefs(cx);
        cx.notify();
    }

    /// F14.5: the workspace's columns, saved once a drag settles.
    pub fn set_columns(&mut self, columns: ColumnWidths, cx: &mut Context<Self>) {
        if self.prefs.columns == columns {
            return;
        }
        self.prefs.columns = columns;
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
        cx.background_spawn(async move {
            if let Err(error) = call.await {
                warn!(%error, "the settings could not be saved");
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                });
            })
            .detach();
            state
        });
        (state, events)
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
