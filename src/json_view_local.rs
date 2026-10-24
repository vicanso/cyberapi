//! How JSON opens, kept on this machine.
//!
//! The panel's Raw / Pretty / Tree switch changes only the document on
//! screen. This is the default every JSON opens as, and it is not part of
//! the settings document the server keeps. Desktop: `json-view.json` beside the database.
//! Browser: `localStorage` key `cyberapi.json_view`, one value per account.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::warn;

/// How a JSON document opens. A request's body, which is the reader's own
/// text, has no raw and pretty forms: it is text unless this is the tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JsonView {
    /// As the server sent it (#51).
    #[default]
    Raw,
    /// Indented ([`cyberapi_core::format::json`]).
    Pretty,
    Tree,
}

impl JsonView {
    /// In the order the switch and the settings offer them.
    pub const ALL: [Self; 3] = [Self::Raw, Self::Pretty, Self::Tree];

    fn name(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Pretty => "pretty",
            Self::Tree => "tree",
        }
    }

    /// `text` is what the indented view was called before there was a raw
    /// one: whoever chose it keeps it.
    fn parse(name: &str) -> Option<Self> {
        match name {
            "raw" => Some(Self::Raw),
            "pretty" | "text" => Some(Self::Pretty),
            "tree" => Some(Self::Tree),
            _ => None,
        }
    }
}

/// The default for this reader. A missing entry, or a file that cannot be
/// read, is [`JsonView::Raw`].
pub fn get(scope: &str) -> JsonView {
    let Some(file) = storage::read_document() else {
        return JsonView::Raw;
    };
    file.readers
        .get(scope)
        .and_then(|name| JsonView::parse(name))
        .unwrap_or_default()
}

/// Remembers `view` for this reader. `false` when the file could not be
/// read or written, so a broken file is left as it is.
pub fn set(scope: &str, view: JsonView) -> bool {
    let Some(mut file) = storage::read_document() else {
        return false;
    };
    let name = view.name();
    if file.readers.get(scope).is_some_and(|had| had == name) {
        return true;
    }
    file.readers.insert(scope.to_string(), name.to_string());
    storage::write_document(&file)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct ViewFile {
    /// Reader scope, then `text` or `tree`.
    readers: BTreeMap<String, String>,
}

fn decode(text: &str) -> Option<ViewFile> {
    if text.trim().is_empty() {
        return Some(ViewFile::default());
    }
    match serde_json::from_str(text) {
        Ok(file) => Some(file),
        Err(error) => {
            warn!(%error, "the json view could not be read");
            None
        }
    }
}

fn encode(file: &ViewFile) -> Option<String> {
    match serde_json::to_string(file) {
        Ok(text) => Some(text),
        Err(error) => {
            warn!(%error, "the json view could not be saved");
            None
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn read_path(path: &std::path::Path) -> Option<ViewFile> {
    match std::fs::read_to_string(path) {
        Ok(text) => decode(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(ViewFile::default()),
        Err(error) => {
            warn!(%error, "the json view could not be read");
            None
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn write_path(path: &std::path::Path, file: &ViewFile) -> bool {
    let Some(text) = encode(file) else {
        return false;
    };
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
        && let Err(error) = std::fs::create_dir_all(dir)
    {
        warn!(%error, "the json view could not be saved");
        return false;
    }
    match std::fs::write(path, text) {
        Ok(()) => true,
        Err(error) => {
            warn!(%error, "the json view could not be saved");
            false
        }
    }
}

#[cfg(not(target_family = "wasm"))]
mod storage {
    #[cfg(test)]
    use std::cell::RefCell;

    #[cfg(test)]
    thread_local! {
        static FILE: RefCell<super::ViewFile> = RefCell::new(super::ViewFile::default());
    }

    #[cfg(test)]
    pub(super) fn read_document() -> Option<super::ViewFile> {
        Some(FILE.with(|file| file.borrow().clone()))
    }

    #[cfg(test)]
    pub(super) fn write_document(file: &super::ViewFile) -> bool {
        FILE.with(|slot| *slot.borrow_mut() = file.clone());
        true
    }

    #[cfg(not(test))]
    pub(super) fn read_document() -> Option<super::ViewFile> {
        let Some(path) = crate::paths::data_dir().map(|dir| dir.join("json-view.json")) else {
            tracing::warn!("the json view could not be read");
            return None;
        };
        super::read_path(&path)
    }

    #[cfg(not(test))]
    pub(super) fn write_document(file: &super::ViewFile) -> bool {
        let Some(path) = crate::paths::data_dir().map(|dir| dir.join("json-view.json")) else {
            tracing::warn!("the json view could not be saved");
            return false;
        };
        super::write_path(&path, file)
    }
}

#[cfg(target_family = "wasm")]
mod storage {
    use super::{decode, encode};

    const STORAGE_KEY: &str = "cyberapi.json_view";

    pub(super) fn read_document() -> Option<super::ViewFile> {
        let storage = web_sys::window()?.local_storage().ok().flatten()?;
        match storage.get_item(STORAGE_KEY) {
            Ok(text) => decode(text.as_deref().unwrap_or_default()),
            Err(error) => {
                tracing::warn!(?error, "the json view could not be read");
                None
            }
        }
    }

    pub(super) fn write_document(file: &super::ViewFile) -> bool {
        let Some(text) = encode(file) else {
            return false;
        };
        let Some(window) = web_sys::window() else {
            tracing::warn!("the json view could not be saved");
            return false;
        };
        let Ok(Some(storage)) = window.local_storage() else {
            tracing::warn!("the json view could not be saved");
            return false;
        };
        if let Err(error) = storage.set_item(STORAGE_KEY, &text) {
            tracing::warn!(?error, "the json view could not be saved");
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_is_kept_for_its_reader() {
        assert_eq!(get("alice"), JsonView::Raw, "raw until chosen otherwise");
        assert!(set("alice", JsonView::Tree));
        assert_eq!(get("alice"), JsonView::Tree);
        assert_eq!(get("bob"), JsonView::Raw, "another reader keeps their own");
        assert!(set("alice", JsonView::Pretty));
        assert_eq!(get("alice"), JsonView::Pretty);
    }

    #[test]
    fn the_indented_view_chosen_before_raw_was_one_is_kept() {
        assert_eq!(JsonView::parse("text"), Some(JsonView::Pretty));
        for view in JsonView::ALL {
            assert_eq!(JsonView::parse(view.name()), Some(view));
        }
        assert_eq!(JsonView::parse("hex"), None);
    }

    #[test]
    fn a_file_round_trips_and_a_broken_one_is_left_alone() {
        let dir = tempfile::tempdir().expect("a directory");
        let path = dir.path().join("json-view.json");
        assert!(read_path(&path).is_some_and(|file| file.readers.is_empty()));

        let mut file = ViewFile::default();
        file.readers.insert(String::new(), "tree".to_string());
        assert!(write_path(&path, &file));
        assert_eq!(read_path(&path), Some(file));

        std::fs::write(&path, "{").expect("break it");
        assert_eq!(read_path(&path), None);
        let broken = std::fs::read_to_string(&path).expect("still there");
        assert_eq!(broken, "{", "a choice that cannot be read is not replaced");
    }
}
