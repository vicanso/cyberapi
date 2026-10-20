//! What a reader keeps for themselves: the application settings (4.7) and,
//! per project, the state of its workspace (4.6). Never shared with a team.
//!
//! Both are documents: every field defaults, so a field added later reads
//! from a document written before it, and one that is gone is ignored.

use crate::ItemId;
use crate::collection::SortOrder;
use crate::model::HttpMethod;
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
