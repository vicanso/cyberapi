//! The application's state (L3 in `docs/architecture.md`): gpui entities that
//! hold what is true and decide what changes. Views read them and forward
//! intent; only these call the stores.

mod api;
mod app;
mod cookies;
mod dashboard;
mod directory;
mod variables;
mod workspace;

pub use api::{ApiEvent, ApiState};
pub use app::{AppEvent, AppState, AppStore, Route};
pub use cookies::{CookieKey, CookiesEvent, CookiesState, cookie_key};
pub use dashboard::{DashboardEvent, DashboardState};
pub use directory::{DirectoryDone, DirectoryEvent, DirectoryState};
pub use variables::{VariablesEvent, VariablesState};
pub use workspace::{WorkspaceEvent, WorkspaceState};

use async_lock::Mutex;
use cyberapi_store::{StoreError, StoreFuture};
use gpui_kit::{AppContext as _, Context};
use std::sync::Arc;

/// Where loading a state's data stands. A reload keeps what is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Load {
    Loading,
    Loaded,
    Failed(StoreError),
}

/// Runs a store call off the main thread and hands its result back to the
/// entity that asked, if it still exists.
pub(crate) fn run_store<T, R>(
    cx: &mut Context<T>,
    call: StoreFuture<R>,
    done: impl FnOnce(&mut T, Result<R, StoreError>, &mut Context<T>) + 'static,
) where
    T: 'static,
    R: Send + 'static,
{
    cx.spawn(async move |this, cx| {
        let result = cx.background_spawn(call).await;
        // The entity is gone when its view closed meanwhile: nothing to tell.
        let _ = this.update(cx, |this, cx| done(this, result, cx));
    })
    .detach();
}

/// Like [`run_store`], but one call at a time through `gate`, in the order
/// they were made, each applied before the next starts.
///
/// For changes whose answers are absolute ("this folder's children are now
/// …"): applied out of order — two requests racing over HTTP — they would
/// leave the copy disagreeing with the store. The gate is fair, so the order
/// of the calls is the order of the changes.
pub(crate) fn run_serial<T, R>(
    cx: &mut Context<T>,
    gate: &Arc<Mutex<()>>,
    call: StoreFuture<R>,
    done: impl FnOnce(&mut T, Result<R, StoreError>, &mut Context<T>) + 'static,
) where
    T: 'static,
    R: Send + 'static,
{
    let gate = gate.clone();
    let turn = gate.lock_arc();
    cx.spawn(async move |this, cx| {
        let _turn = turn.await;
        let result = cx.background_spawn(call).await;
        let _ = this.update(cx, |this, cx| done(this, result, cx));
    })
    .detach();
}
