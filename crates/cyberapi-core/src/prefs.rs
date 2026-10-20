//! What a reader keeps for themselves: the application settings (4.7) and,
//! per project, the state of its workspace (4.6). Never shared with a team.
//!
//! Both are documents: every field defaults, so a field added later reads
//! from a document written before it, and one that is gone is ignored.

use crate::ItemId;
use crate::collection::SortOrder;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppPrefs {
    pub theme: ThemeMode,
    /// `None` follows the system language (F15.2).
    pub language: Option<String>,
    pub collection_sort: SortOrder,
    pub columns: ColumnWidths,
    pub window: WindowPrefs,
    pub timeouts: Timeouts,
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
    /// Pinned APIs, in pin order (M12).
    pub pinned: Vec<ItemId>,
    /// The request editor's tab per API when it is not the default (F4.12).
    pub tabs: BTreeMap<ItemId, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_document_is_the_defaults() {
        let prefs: AppPrefs = serde_json::from_str("{}").expect("defaults");
        assert_eq!(prefs, AppPrefs::default());
        assert_eq!(prefs.columns.tree, 300.);
        let workspace: WorkspacePrefs = serde_json::from_str(r#"{"pinned":["a"],"gone":1}"#).expect("defaults");
        assert_eq!(workspace.pinned, vec![ItemId::from("a")]);
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
