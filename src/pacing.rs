//! How often the app asks a store on its own, unprompted — the one place
//! the desktop and the browser differ in pace.
//!
//! Each value is two constants under their own gate, desktop first, so
//! changing one cannot move the other.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How often an open project asks whether another reader changed it
/// (`docs/architecture.md` §7). The desktop's store has one writer — this
/// app — so it never asks.
#[cfg(not(target_family = "wasm"))]
pub const REVISION_POLL: Option<Duration> = None;
#[cfg(target_family = "wasm")]
pub const REVISION_POLL: Option<Duration> = Some(Duration::from_secs(10));

/// How often an open server project — the page's, or the desktop's copy of
/// one — asks the server whether another reader changed it
/// (`docs/server-sync.md` §3.1). The desktop's own projects never ask.
#[cfg(not(target_family = "wasm"))]
pub const SERVER_POLL: Option<Duration> = Some(Duration::from_secs(10));
#[cfg(target_family = "wasm")]
pub const SERVER_POLL: Option<Duration> = Some(Duration::from_secs(10));

/// How long opening a server project waits for the server before working
/// on what this machine kept of it, the poll catching up after: the
/// desktop keeps a copy, and a bridge out of reach held the project shut
/// until the connection timed out. A page waits for its own server, which
/// it cannot work without.
#[cfg(not(target_family = "wasm"))]
pub const COPY_OPEN_WAIT: Option<Duration> = Some(Duration::from_secs(2));
#[cfg(target_family = "wasm")]
pub const COPY_OPEN_WAIT: Option<Duration> = None;

/// While the page is in a background tab, only one beat in this many asks:
/// a minute apart, and back to every beat as soon as the tab shows.
pub const HIDDEN_EVERY: u32 = 6;

static PAGE_HIDDEN: AtomicBool = AtomicBool::new(false);

/// Whether nobody can see the app: a browser page in a background tab. The
/// desktop never says so.
pub fn page_hidden() -> bool {
    PAGE_HIDDEN.load(Ordering::Relaxed)
}

/// The browser page's `visibilitychange`, handed in by `cyberapi-web`.
pub fn set_page_visible(visible: bool) {
    PAGE_HIDDEN.store(!visible, Ordering::Relaxed);
}
