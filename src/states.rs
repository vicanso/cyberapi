//! The application's state (L3 in `docs/architecture.md`): gpui entities that
//! hold what is true and decide what changes. Views read them and forward
//! intent; only these call the stores.

mod app;
mod dashboard;

pub use app::{AppState, AppStore, Route};
pub use dashboard::{DashboardEvent, DashboardState, Load};

use cyberapi_store::{StoreError, StoreFuture};
use gpui_kit::{AppContext as _, Context};

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
