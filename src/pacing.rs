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
/// (`docs/server-sync.md` §3.1). The desktop's own projects never ask. It
/// only shows others' changes sooner: a sync checks the server's revision
/// itself. Teammates editing in their pages see each other's sooner. The
/// projects page asks the server for its list as often, while it shows: a
/// project made or shared by others comes in without leaving it.
#[cfg(not(target_family = "wasm"))]
pub const SERVER_POLL: Option<Duration> = Some(Duration::from_secs(30));
#[cfg(target_family = "wasm")]
pub const SERVER_POLL: Option<Duration> = Some(Duration::from_secs(10));

/// After the server could not be reached, it is asked again this long
/// after, then twice as long after each failure, up to [`RETRY_LONGEST`]:
/// a network that stays down is not asked every beat.
#[cfg(not(target_family = "wasm"))]
pub const RETRY_FIRST: Duration = Duration::from_secs(10);
#[cfg(target_family = "wasm")]
pub const RETRY_FIRST: Duration = Duration::from_secs(10);

#[cfg(not(target_family = "wasm"))]
pub const RETRY_LONGEST: Duration = Duration::from_secs(5 * 60);
#[cfg(target_family = "wasm")]
pub const RETRY_LONGEST: Duration = Duration::from_secs(5 * 60);

/// How long after the `failures`th failure in a row (from 1) the server is
/// asked again: 10 s, 20 s, 40 s… up to five minutes.
pub fn retry_after(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    RETRY_FIRST.saturating_mul(1 << doublings).min(RETRY_LONGEST)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_wait_twice_as_long_each_time_up_to_five_minutes() {
        let waits: Vec<u64> = (1..=8).map(|failures| retry_after(failures).as_secs()).collect();
        assert_eq!(waits, [10, 20, 40, 80, 160, 300, 300, 300]);
        assert_eq!(retry_after(0).as_secs(), 10);
        assert_eq!(retry_after(u32::MAX).as_secs(), 300);
    }
}
