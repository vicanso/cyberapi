//! What the whole application shares: the stores, where the reader is, and
//! their settings.

use crate::this_build;
use cyberapi_core::collection::SortOrder;
use cyberapi_core::model::Collection;
use cyberapi_core::prefs::{AppPrefs, ColumnWidths};
use cyberapi_http::HttpEngine;
use cyberapi_service::Sender;
use cyberapi_store::Storage;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

/// The stores, the engine and the application state, installed once by
/// `launch`.
#[derive(Clone)]
pub struct AppStore {
    storage: Storage,
    engine: Arc<dyn HttpEngine>,
    state: Entity<AppState>,
}

impl Global for AppStore {}

impl AppStore {
    pub fn init(storage: Storage, engine: Arc<dyn HttpEngine>, prefs: AppPrefs, cx: &mut App) {
        let state = cx.new(|_| AppState {
            route: Route::Dashboard,
            prefs,
            save_task: None,
        });
        cx.set_global(Self { storage, engine, state });
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub fn storage(&self) -> &Storage {
        &self.storage
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

pub struct AppState {
    route: Route,
    prefs: AppPrefs,
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
