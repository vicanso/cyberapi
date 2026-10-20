//! The kit's icons, fetched, and a repaint when one lands.
//!
//! On the desktop the kit embeds its icons. In a page it fetches each from
//! `{origin}/assets/icons/…` the first time it is asked for, answering
//! "not yet" until the download lands — and nothing tells the window when it
//! does, so an icon on a quiet screen stayed missing until something else
//! repainted (step 4 drew the empty state without its icon). This source
//! remembers what it could not answer, and [`repaint_when_icons_land`]
//! refreshes the windows once one of those starts answering. Ported from
//! zedis-web, where the same hole was found first.
//!
//! The watcher sleeps on a channel while nothing is awaited: a timer that
//! never stopped would keep the page from ever resting.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui_kit::{App, AssetSource, Result, SharedString};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Paths asked for and not yet answered, with how many times each has been
/// looked at since.
type Awaited = Arc<Mutex<HashMap<String, u32>>>;

/// How often the watcher looks while something is awaited.
const WATCH_BUSY: Duration = Duration::from_millis(60);

/// After this many looks an icon is given up on: asking the kit for a path
/// it does not have starts another fetch, so a 404 would be fetched forever.
const WATCH_LOOKS: u32 = 150;

pub struct IconWatch {
    awaited: Awaited,
    wake: UnboundedReceiver<()>,
}

pub struct WebAssets {
    kit: gpui_kit::assets::Assets,
    awaited: Awaited,
    /// Pokes the watcher when a path is listed. Unbounded and never awaited:
    /// `load` runs inside a paint.
    wake: UnboundedSender<()>,
    watch: Mutex<Option<UnboundedReceiver<()>>>,
}

impl WebAssets {
    /// `origin` is where the kit fetches from.
    pub fn new(origin: &str) -> Self {
        let (wake, watch) = unbounded();
        Self {
            kit: gpui_kit::assets::Assets::new(origin.to_string()),
            awaited: Awaited::default(),
            wake,
            watch: Mutex::new(Some(watch)),
        }
    }

    /// The watcher's half; `None` the second time — there is one watcher.
    pub fn take_watch(&self) -> Option<IconWatch> {
        let wake = self.watch.lock().ok()?.take()?;
        Some(IconWatch {
            awaited: self.awaited.clone(),
            wake,
        })
    }
}

/// Refreshes every window when an icon that was asked for arrives.
pub fn repaint_when_icons_land(watch: IconWatch, cx: &mut App) {
    let IconWatch { awaited, mut wake } = watch;
    cx.spawn(async move |cx| {
        loop {
            let busy = awaited.lock().map(|list| !list.is_empty()).unwrap_or(false);
            if !busy && wake.next().await.is_none() {
                // The asset source is gone, and everything that could ask.
                return;
            }
            cx.background_executor().timer(WATCH_BUSY).await;
            let paths: Vec<String> = match awaited.lock() {
                Ok(list) => list.keys().cloned().collect(),
                Err(_) => return,
            };
            if paths.is_empty() {
                continue;
            }
            let awaited = awaited.clone();
            cx.update(move |cx| {
                let source = cx.asset_source().clone();
                // Asking again is what finds out. It also lists a path still
                // missing again, which is why the list is edited after.
                let landed: Vec<&String> = paths
                    .iter()
                    .filter(|path| matches!(source.load(path), Ok(Some(_))))
                    .collect();
                if let Ok(mut list) = awaited.lock() {
                    for path in &landed {
                        list.remove(*path);
                    }
                    list.retain(|_, looks| {
                        *looks += 1;
                        *looks < WATCH_LOOKS
                    });
                }
                if !landed.is_empty() {
                    cx.refresh_windows();
                }
            });
        }
    })
    .detach();
}

impl AssetSource for WebAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let answer = self.kit.load(path);
        if !matches!(answer, Ok(Some(_)))
            && let Ok(mut list) = self.awaited.lock()
            && !list.contains_key(path)
        {
            list.insert(path.to_string(), 0);
            let _ = self.wake.unbounded_send(());
        }
        // The kit answers an icon still on its way with an error, which GPUI
        // logs on every frame. "Not there yet" is `Ok(None)`: GPUI paints
        // nothing and asks again on the next paint, which the watcher makes.
        answer.or(Ok(None))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        self.kit.list(path)
    }
}
