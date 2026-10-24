//! What a reader keeps for themselves: the application settings (4.7) and,
//! per project, the state of its workspace (4.6). Never shared with a team.
//!
//! Both are documents: every field defaults, so a field added later reads
//! from a document written before it, and one that is gone is ignored.

use crate::collection::SortOrder;
use crate::model::HttpMethod;
use crate::{CollectionId, ItemId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::RangeInclusive;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppPrefs {
    pub theme: ThemeMode,
    /// `None` follows the system language (F15.2).
    pub language: Option<String>,
    /// Text sizes and wrapping (F14.6, F14.7).
    pub text: TextPrefs,
    pub collection_sort: SortOrder,
    pub columns: ColumnWidths,
    pub window: WindowPrefs,
    /// Where the window last sat on each monitor, keyed by that monitor's
    /// stable id, and whether it was zoomed there. Desktop only; a page
    /// leaves this empty. The size here is the one before a zoom, never the
    /// frame that fills the monitor: a zoomed window opens zoomed and
    /// returns to it.
    pub monitors: BTreeMap<String, MonitorPlacement>,
    /// The monitor the window was on last. Empty when none was recorded.
    /// The next launch opens there while it is connected, else on another
    /// connected monitor in `monitors`, else as [`WindowPrefs`] says (see
    /// [`opening_monitor`]).
    pub last_monitor: String,
    pub timeouts: Timeouts,
    /// How a request goes out besides its timeouts (F14.8, F14.9).
    pub sending: SendPrefs,
    /// The project open when the app last left it, opened again at the
    /// next start (F1.1, F1.9); `None` was the project list.
    pub last_collection: Option<CollectionId>,
    /// The reader's groups of the project list: a project's group by its
    /// id. A project not here is in none.
    pub project_groups: BTreeMap<CollectionId, String>,
    /// The groups folded on the project list; `""` is the projects in none.
    pub folded_groups: Vec<String>,
    /// The desktop's update check. A page leaves it as it is.
    pub updates: UpdatePrefs,
}

/// How the desktop looks for a newer release ([`crate::update`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdatePrefs {
    /// Look at start, at most once in [`UpdatePrefs::INTERVAL`].
    pub auto_check: bool,
    /// When it last looked, by itself or asked, in Unix seconds; 0 for
    /// never.
    pub last_check: i64,
    /// A version the reader asked not to be told of again. Asked to look,
    /// the app still says it is there.
    pub skipped: String,
}

impl UpdatePrefs {
    /// Two days between the automatic checks.
    pub const INTERVAL: i64 = 2 * 24 * 60 * 60;
}

impl Default for UpdatePrefs {
    fn default() -> Self {
        Self {
            auto_check: true,
            last_check: 0,
            skipped: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemeMode {
    /// Follows the system, and keeps following it (F14.1).
    #[default]
    System,
    Light,
    Dark,
}

/// How text shows: sizes in pixels, `0` the default (F14.6), and whether
/// long lines in the code editors wrap (F14.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TextPrefs {
    /// The interface's: the rem everything is measured in.
    pub ui_size: u32,
    /// The code editors': a request's body and a response's.
    pub editor_size: u32,
    pub wrap: bool,
}

impl Default for TextPrefs {
    fn default() -> Self {
        Self {
            ui_size: 0,
            editor_size: 0,
            wrap: true,
        }
    }
}

impl TextPrefs {
    /// The kit's 16 px is a document's; a tool's panels read better
    /// denser, as Zed's do.
    pub const DEFAULT_UI: u32 = 14;
    /// The kit's code size.
    pub const DEFAULT_EDITOR: u32 = 13;
    /// The sizes offered. The interface stops at 18: rows of a fixed height
    /// hold no larger text.
    pub const UI_SIZES: RangeInclusive<u32> = 12..=18;
    pub const EDITOR_SIZES: RangeInclusive<u32> = 11..=20;

    /// The interface's size to use: the setting, kept to the sizes offered,
    /// or the default.
    pub fn ui(&self) -> u32 {
        within(self.ui_size, Self::DEFAULT_UI, Self::UI_SIZES)
    }

    /// The code editors' size to use, as [`Self::ui`].
    pub fn editor(&self) -> u32 {
        within(self.editor_size, Self::DEFAULT_EDITOR, Self::EDITOR_SIZES)
    }
}

/// `value` within `sizes`; `0` is `default`.
fn within(value: u32, default: u32, sizes: RangeInclusive<u32>) -> u32 {
    if value == 0 {
        default
    } else {
        value.clamp(*sizes.start(), *sizes.end())
    }
}

/// How a request goes out besides its timeouts. The switches off is how the
/// old app sent, and what a document from before them reads as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SendPrefs {
    /// A redirect is followed to where it points (F14.8); off, the
    /// redirect is the answer.
    pub follow_redirects: bool,
    /// HTTPS goes to a server whose certificate does not verify:
    /// self-signed, expired, or for another name (F14.9).
    pub accept_invalid_certs: bool,
    /// F14.10.
    pub http_version: HttpVersion,
}

/// Which HTTP a request speaks (F14.10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HttpVersion {
    /// HTTP/2 where the TLS handshake agrees on it, HTTP/1.1 otherwise and
    /// over plain HTTP: what browsers and curl do.
    #[default]
    Auto,
    /// HTTP/1.1 only, as the old app sent.
    Http1,
    /// HTTP/2 only: a server that does not agree to it over TLS fails the
    /// request, and plain HTTP starts in HTTP/2 straight away (h2c).
    Http2,
}

/// The workspace's columns (F14.5): the tree in pixels, then the request
/// editor's share of what is left; the response takes the rest.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ColumnWidths {
    pub tree: f32,
    pub editor_ratio: f32,
}

impl Default for ColumnWidths {
    fn default() -> Self {
        Self {
            tree: 300.,
            editor_ratio: 0.5,
        }
    }
}

/// How the main window opens (F1.3). Desktop only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WindowPrefs {
    pub maximized: bool,
    /// The size when not maximized; `0` lets the app choose. Never below
    /// 900 × 600 (F14.2).
    pub width: u32,
    pub height: u32,
}

impl WindowPrefs {
    /// The size the app chooses.
    pub const DEFAULT_WIDTH: u32 = 1280;
    pub const DEFAULT_HEIGHT: u32 = 800;
    /// The smallest the window is made (F14.2).
    pub const MIN_WIDTH: u32 = 900;
    pub const MIN_HEIGHT: u32 = 600;
}

/// Position and content size of the window on one monitor, and whether it
/// was zoomed there. The origin is that monitor's top left, in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MonitorPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Maximized, full screen or filling the monitor when the window last
    /// left it: it opens there maximized, and the frame above is the one
    /// it returns to. Not written while false, as before there was one.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub zoomed: bool,
}

impl MonitorPlacement {
    /// Large enough to open. A zoom animation passes through smaller frames.
    pub fn usable(self) -> bool {
        self.width >= WindowPrefs::MIN_WIDTH && self.height >= WindowPrefs::MIN_HEIGHT
    }

    /// Says something about how the window opens: a frame to open at, or
    /// that it opens zoomed.
    pub fn kept(self) -> bool {
        self.usable() || self.zoomed
    }

    /// Moved, and shrunk when larger, into `area`: the monitor's visible
    /// part, in the same coordinates. Never below the smallest window. A
    /// monitor whose resolution or arrangement changed since still shows
    /// the whole window; an empty area changes nothing.
    pub fn fitted(self, area: MonitorPlacement) -> Self {
        if area.width == 0 || area.height == 0 {
            return self;
        }
        let width = self.width.min(area.width).max(WindowPrefs::MIN_WIDTH);
        let height = self.height.min(area.height).max(WindowPrefs::MIN_HEIGHT);
        let within = |at: i32, start: i32, room: u32, length: u32| {
            let last = i64::from(start) + i64::from(room) - i64::from(length);
            i64::from(at).min(last).max(i64::from(start)) as i32
        };
        Self {
            x: within(self.x, area.x, area.width, width),
            y: within(self.y, area.y, area.height, height),
            width,
            height,
            zoomed: self.zoomed,
        }
    }
}

/// The placement kept for `monitor`, when it says how to open there (see
/// [`MonitorPlacement::kept`]).
pub fn placement_for<'a>(
    monitors: &'a BTreeMap<String, MonitorPlacement>,
    monitor: &str,
) -> Option<&'a MonitorPlacement> {
    let placement = monitors.get(monitor)?;
    placement.kept().then_some(placement)
}

/// The monitor the window opens on, among those `connected` now (the one a
/// window opens on by default first), and the placement kept for it. The
/// monitor it was on last while that one is connected, with its placement
/// when one was kept (`None`: centered there); else the first connected
/// monitor that has one. `None` when neither: the default monitor, centered.
/// A zoomed placement opens maximized, over its frame when it has a usable
/// one and over the centered size when not.
pub fn opening_monitor<'a>(
    monitors: &BTreeMap<String, MonitorPlacement>,
    last: &str,
    connected: &[&'a str],
) -> Option<(&'a str, Option<MonitorPlacement>)> {
    if let Some(id) = connected.iter().find(|id| !last.is_empty() && **id == last) {
        return Some((id, placement_for(monitors, id).copied()));
    }
    connected
        .iter()
        .find_map(|id| Some((*id, Some(*placement_for(monitors, id)?))))
}

/// A double-click Fill sits a few pixels inside the monitor (the system's
/// margins for tiled windows). That frame is the monitor, a zoom, not a size
/// to restore. Only a few pixels: on a laptop's screen a window made large
/// by hand is well within a wider margin, and is a size to keep.
pub fn fills_monitor(width: u32, height: u32, monitor_width: u32, monitor_height: u32) -> bool {
    const MARGIN: u32 = 16;
    if monitor_width == 0 || monitor_height == 0 {
        return false;
    }
    width + MARGIN >= monitor_width
        && height + MARGIN >= monitor_height
        && width <= monitor_width.saturating_add(MARGIN)
        && height <= monitor_height.saturating_add(MARGIN)
}

/// What keeping a sample changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorRecord {
    pub monitors: BTreeMap<String, MonitorPlacement>,
    pub last_monitor: String,
}

/// `zoomed` marks the monitor the window is on as zoomed and leaves the
/// frame from before the zoom, to return to. A windowed sample replaces that
/// monitor's position and size, and the mark.
pub fn remember_monitor(
    monitors: &BTreeMap<String, MonitorPlacement>,
    last: &str,
    id: &str,
    placement: MonitorPlacement,
    zoomed: bool,
) -> Option<MonitorRecord> {
    if id.is_empty() {
        return None;
    }
    let next = if zoomed {
        MonitorPlacement {
            zoomed: true,
            ..monitors.get(id).copied().unwrap_or_default()
        }
    } else if placement.usable() {
        MonitorPlacement {
            zoomed: false,
            ..placement
        }
    } else {
        return None;
    };
    if last == id && monitors.get(id) == Some(&next) {
        return None;
    }
    let mut monitors = monitors.clone();
    monitors.insert(id.to_string(), next);
    Some(MonitorRecord {
        monitors,
        last_monitor: id.to_string(),
    })
}

/// Request timeouts in seconds; `0` is the default (F5.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Timeouts {
    pub connect: u32,
    pub write: u32,
    pub read: u32,
}

impl Timeouts {
    pub const DEFAULT_CONNECT: u32 = 10;
    pub const DEFAULT_WRITE: u32 = 120;
    pub const DEFAULT_READ: u32 = 300;

    /// The timeouts to use: each setting above `0`, the default otherwise.
    pub fn effective(&self) -> Self {
        let pick = |value: u32, default: u32| if value > 0 { value } else { default };
        Self {
            connect: pick(self.connect, Self::DEFAULT_CONNECT),
            write: pick(self.write, Self::DEFAULT_WRITE),
            read: pick(self.read, Self::DEFAULT_READ),
        }
    }
}

/// One reader's state of one project's workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkspacePrefs {
    /// Open folders (F3.3).
    pub expanded: Vec<ItemId>,
    /// The API last selected, restored on the next visit (F3.8). Kept per
    /// project; the old app kept one for all of them (#20).
    pub selected: Option<ItemId>,
    /// Pinned APIs, in pin order (M12). The application keeps the list on
    /// the machine and writes this empty; the field stays so a document
    /// from before that still parses, and is copied across once.
    pub pinned: Vec<ItemId>,
    /// The request editor's tab per API when it is not the default (F4.12).
    pub tabs: BTreeMap<ItemId, String>,
}

/// The request editor's tabs (F4.12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RequestTab {
    Body,
    Query,
    Headers,
    Auth,
}

impl RequestTab {
    /// In the editor's order; Body only for the methods that send one.
    pub fn for_method(method: HttpMethod) -> &'static [Self] {
        if method.has_body() {
            &[Self::Body, Self::Query, Self::Headers, Self::Auth]
        } else {
            &[Self::Query, Self::Headers, Self::Auth]
        }
    }

    /// Body for the methods that send one, Query otherwise.
    pub fn default_for(method: HttpMethod) -> Self {
        if method.has_body() { Self::Body } else { Self::Query }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Query => "query",
            Self::Headers => "header",
            Self::Auth => "auth",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "body" => Some(Self::Body),
            "query" => Some(Self::Query),
            "header" => Some(Self::Headers),
            "auth" => Some(Self::Auth),
            _ => None,
        }
    }
}

impl WorkspacePrefs {
    /// The tab `api` shows: the one kept, if the method has it, or the
    /// method's default.
    pub fn tab(&self, api: &ItemId, method: HttpMethod) -> RequestTab {
        self.tabs
            .get(api)
            .and_then(|tab| RequestTab::parse(tab))
            .filter(|tab| RequestTab::for_method(method).contains(tab))
            .unwrap_or(RequestTab::default_for(method))
    }

    /// Keeps `tab` for `api`; the method's default is kept as nothing.
    pub fn set_tab(&mut self, api: &ItemId, method: HttpMethod, tab: RequestTab) {
        if tab == RequestTab::default_for(method) {
            self.tabs.remove(api);
        } else {
            self.tabs.insert(api.clone(), tab.as_str().to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_document_is_the_defaults() {
        let prefs: AppPrefs = serde_json::from_str("{}").expect("defaults");
        assert_eq!(prefs, AppPrefs::default());
        assert_eq!(prefs.columns.tree, 300.);
        assert!(prefs.text.wrap, "long lines wrap, as they did before the setting");
        assert_eq!(
            prefs.sending,
            SendPrefs::default(),
            "redirects stay answers, certificates are checked"
        );
        let text: TextPrefs = serde_json::from_str(r#"{"uiSize":16}"#).expect("defaults");
        assert_eq!(
            text,
            TextPrefs {
                ui_size: 16,
                ..TextPrefs::default()
            }
        );
        let workspace: WorkspacePrefs = serde_json::from_str(r#"{"pinned":["a"],"gone":1}"#).expect("defaults");
        assert_eq!(workspace.pinned, vec![ItemId::from("a")]);
    }

    #[test]
    fn tabs_default_by_method_and_keep_only_the_others() {
        let api = ItemId::from("a");
        let mut prefs = WorkspacePrefs::default();
        assert_eq!(prefs.tab(&api, HttpMethod::Get), RequestTab::Query);
        assert_eq!(prefs.tab(&api, HttpMethod::Post), RequestTab::Body);
        prefs.set_tab(&api, HttpMethod::Post, RequestTab::Headers);
        assert_eq!(prefs.tabs.get(&api).map(String::as_str), Some("header"));
        assert_eq!(prefs.tab(&api, HttpMethod::Post), RequestTab::Headers);
        prefs.set_tab(&api, HttpMethod::Post, RequestTab::Body);
        assert!(prefs.tabs.is_empty(), "the default is kept as nothing");
        // A kept Body tab on a method without one falls back to Query.
        prefs.tabs.insert(api.clone(), "body".into());
        assert_eq!(prefs.tab(&api, HttpMethod::Get), RequestTab::Query);
    }

    /// A frame on a monitor, not zoomed.
    fn frame(x: i32, y: i32, width: u32, height: u32) -> MonitorPlacement {
        MonitorPlacement {
            x,
            y,
            width,
            height,
            zoomed: false,
        }
    }

    #[test]
    fn a_zoom_is_remembered_over_the_size_from_before_it() {
        let placed = frame(40, 24, 1100, 720);
        let mut monitors = BTreeMap::new();
        monitors.insert("a".into(), placed);

        let zoomed = remember_monitor(&monitors, "a", "a", MonitorPlacement::default(), true).expect("zoom");
        assert_eq!(
            zoomed.monitors.get("a"),
            Some(&MonitorPlacement { zoomed: true, ..placed }),
            "the frame from before the zoom is kept under the mark"
        );
        assert_eq!(
            remember_monitor(&zoomed.monitors, "a", "a", MonitorPlacement::default(), true),
            None,
            "zooming again writes nothing"
        );
        let unzoomed = remember_monitor(&zoomed.monitors, "a", "a", placed, false).expect("back to a window");
        assert_eq!(unzoomed.monitors.get("a"), Some(&placed));

        let elsewhere =
            remember_monitor(&monitors, "a", "b", MonitorPlacement::default(), true).expect("other monitor");
        assert_eq!(elsewhere.last_monitor, "b");
        assert_eq!(elsewhere.monitors.get("a"), Some(&placed));
        assert_eq!(
            elsewhere.monitors.get("b"),
            Some(&MonitorPlacement {
                zoomed: true,
                ..MonitorPlacement::default()
            }),
            "zoomed on a monitor it has no frame for yet"
        );
        assert!(
            placement_for(&elsewhere.monitors, "b").is_some(),
            "a zoom alone says how to open"
        );

        let next = frame(80, 10, 1000, 700);
        let recorded = remember_monitor(&monitors, "a", "a", next, false).expect("resize");
        assert_eq!(recorded.monitors.get("a"), Some(&next));
        assert_eq!(remember_monitor(&recorded.monitors, "a", "a", next, false), None);
        let sampled = remember_monitor(&monitors, "a", "a", MonitorPlacement { zoomed: true, ..next }, false)
            .expect("a windowed sample");
        assert_eq!(sampled.monitors.get("a"), Some(&next), "a window is not zoomed");
        assert_eq!(
            remember_monitor(&monitors, "a", "a", frame(40, 24, 800, 700), false),
            None,
            "below the minimum is not a size"
        );
        assert_eq!(remember_monitor(&monitors, "a", "", next, false), None);
        assert_eq!(placement_for(&monitors, "missing"), None);
        assert_eq!(placement_for(&recorded.monitors, "a").copied(), Some(next));
    }

    #[test]
    fn a_placement_kept_before_the_zoom_mark_reads_as_it_was() {
        let written = r#"{"x":153,"y":108,"width":1281,"height":801}"#;
        let old: MonitorPlacement = serde_json::from_str(written).expect("parses");
        assert_eq!(old, frame(153, 108, 1281, 801));
        assert_eq!(
            serde_json::to_string(&old).expect("writes"),
            written,
            "no mark while not zoomed"
        );
        let zoomed = MonitorPlacement { zoomed: true, ..old };
        let again: MonitorPlacement =
            serde_json::from_str(&serde_json::to_string(&zoomed).expect("writes")).expect("parses");
        assert_eq!(again, zoomed);
    }

    #[test]
    fn the_window_opens_on_a_connected_monitor_it_was_placed_on() {
        let on = |x| frame(x, 40, 1100, 720);
        let zoomed_only = MonitorPlacement {
            zoomed: true,
            ..MonitorPlacement::default()
        };
        let mut monitors = BTreeMap::new();
        monitors.insert("a".to_string(), on(10));
        monitors.insert("b".to_string(), on(20));
        monitors.insert("small".to_string(), MonitorPlacement { width: 400, ..on(30) });
        monitors.insert("z".to_string(), zoomed_only);

        assert_eq!(
            opening_monitor(&monitors, "b", &["a", "b"]),
            Some(("b", Some(on(20)))),
            "the last monitor while it is connected"
        );
        assert_eq!(
            opening_monitor(&monitors, "b", &["c", "a"]),
            Some(("a", Some(on(10)))),
            "the last one is gone: a connected one with a position"
        );
        assert_eq!(
            opening_monitor(&monitors, "c", &["c", "a"]),
            Some(("c", None)),
            "the last one, without a position, is centered on"
        );
        assert_eq!(
            opening_monitor(&monitors, "z", &["a", "z"]),
            Some(("z", Some(zoomed_only))),
            "zoomed there with no frame: maximized over the centered size"
        );
        assert_eq!(opening_monitor(&monitors, "gone", &["small", "c"]), None);
        assert_eq!(
            opening_monitor(&monitors, "", &["", "c"]),
            None,
            "no monitor was recorded"
        );
    }

    #[test]
    fn a_placement_is_kept_inside_the_monitor() {
        let area = frame(75, 30, 1845, 1050);
        let placed = frame(300, 120, 1100, 720);
        assert_eq!(placed.fitted(area), placed, "already inside");
        assert_eq!(placed.fitted(MonitorPlacement::default()), placed, "no area known");

        let past_the_edge = MonitorPlacement {
            x: 1500,
            y: 900,
            ..placed
        };
        assert_eq!(
            past_the_edge.fitted(area),
            MonitorPlacement {
                x: 75 + 1845 - 1100,
                y: 30 + 1050 - 720,
                ..placed
            }
        );
        let above = MonitorPlacement { x: -40, y: 0, ..placed };
        assert_eq!(above.fitted(area), MonitorPlacement { x: 75, y: 30, ..placed });
        let zoomed = MonitorPlacement {
            zoomed: true,
            ..past_the_edge
        };
        assert!(zoomed.fitted(area).zoomed, "fitting keeps the mark");

        let larger = frame(0, 0, 2400, 1400);
        assert_eq!(larger.fitted(area), area, "shrunk to the visible part");
        let tiny = frame(100, 100, 800, 500);
        assert_eq!(
            placed.fitted(tiny),
            frame(100, 100, WindowPrefs::MIN_WIDTH, WindowPrefs::MIN_HEIGHT),
            "never below the smallest window"
        );
    }

    #[test]
    fn a_frame_that_fills_the_monitor_is_not_a_window_size() {
        // A laptop's visible area, below the menu bar and beside the Dock.
        assert!(fills_monitor(1441, 949, 1441, 949), "the monitor itself");
        assert!(
            fills_monitor(1425, 933, 1441, 949),
            "a Fill, inside the system's margins"
        );
        assert!(!fills_monitor(1380, 890, 1441, 949), "a window made large by hand");
        assert!(!fills_monitor(1100, 720, 1441, 949));
        assert!(!fills_monitor(1441, 949, 0, 0));
    }

    /// A document written before the update check looks for updates.
    #[test]
    fn the_update_check_is_on_unless_turned_off() {
        let old: AppPrefs = serde_json::from_str("{}").expect("parses");
        assert!(old.updates.auto_check);
        let empty: AppPrefs = serde_json::from_str(r#"{"updates":{}}"#).expect("parses");
        assert_eq!(empty.updates, UpdatePrefs::default());
        let off: AppPrefs =
            serde_json::from_str(r#"{"updates":{"autoCheck":false,"skipped":"0.3.0"}}"#).expect("parses");
        assert!(!off.updates.auto_check);
        assert_eq!(off.updates.skipped, "0.3.0");
    }

    #[test]
    fn text_sizes_default_and_stay_within_those_offered() {
        let text = TextPrefs::default();
        assert_eq!((text.ui(), text.editor()), (14, 13));
        let chosen = TextPrefs {
            ui_size: 16,
            editor_size: 20,
            wrap: false,
        };
        assert_eq!((chosen.ui(), chosen.editor()), (16, 20));
        // A document edited by hand cannot make the window unreadable.
        let odd = TextPrefs {
            ui_size: 3,
            editor_size: 90,
            wrap: true,
        };
        assert_eq!((odd.ui(), odd.editor()), (12, 20));
        assert!(TextPrefs::UI_SIZES.contains(&TextPrefs::DEFAULT_UI));
        assert!(TextPrefs::EDITOR_SIZES.contains(&TextPrefs::DEFAULT_EDITOR));
        let inside = TextPrefs {
            ui_size: 17,
            editor_size: 19,
            wrap: true,
        };
        assert_eq!((inside.ui(), inside.editor()), (17, 19), "any size in the range");
    }

    #[test]
    fn zero_timeouts_are_the_defaults() {
        let timeouts = Timeouts {
            connect: 0,
            write: 5,
            read: 0,
        };
        assert_eq!(
            timeouts.effective(),
            Timeouts {
                connect: 10,
                write: 5,
                read: 300
            }
        );
    }
}
