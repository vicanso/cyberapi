//! Pinned APIs, kept on this machine (M12).
//!
//! The workspace document still has a `pinned` field, so an older copy
//! parses. A save writes that field empty. The list lives here: `pins.json`
//! beside the database on the desktop, `localStorage` in the browser. A
//! reader who already pinned before that keeps those pins the first time
//! this build opens the project, and the stored copy is then cleared.

use cyberapi_core::{CollectionId, ItemId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::warn;

/// Desktop is one reader. The browser separates accounts that share a profile.
#[cfg(not(target_family = "wasm"))]
pub fn scope(_account: Option<&str>) -> String {
    String::new()
}
#[cfg(target_family = "wasm")]
pub fn scope(account: Option<&str>) -> String {
    account.unwrap_or_default().to_string()
}

/// `None` when this project has no list here yet, so a copy still in the
/// workspace document may be adopted. `Some`, including an empty list, is
/// the reader's list: an empty one means they unpinned everything.
pub fn get(scope: &str, collection: &CollectionId) -> Option<Vec<ItemId>> {
    let file = storage::read_document()?;
    file.readers.get(scope)?.get(collection.as_str()).cloned()
}

/// Writes `pins` for this project. `false` when the list could not be
/// read or written, so the caller can leave the old copy where it was.
pub fn set(scope: &str, collection: &CollectionId, pins: &[ItemId]) -> bool {
    let Some(mut file) = storage::read_document() else {
        return false;
    };
    let projects = file.readers.entry(scope.to_string()).or_default();
    if projects.get(collection.as_str()).is_some_and(|had| had == pins) {
        return true;
    }
    projects.insert(collection.as_str().to_string(), pins.to_vec());
    storage::write_document(&file)
}

/// Drops one project's list. A project that was never pinned is left as it is.
pub fn remove(scope: &str, collection: &CollectionId) {
    let Some(mut file) = storage::read_document() else {
        return;
    };
    let Some(projects) = file.readers.get_mut(scope) else {
        return;
    };
    if projects.remove(collection.as_str()).is_none() {
        return;
    }
    if projects.is_empty() {
        file.readers.remove(scope);
    }
    let _ = storage::write_document(&file);
}

/// Drops every project's list for this reader (F15.3).
pub fn clear_scope(scope: &str) {
    let Some(mut file) = storage::read_document() else {
        return;
    };
    if file.readers.remove(scope).is_none() {
        return;
    }
    let _ = storage::write_document(&file);
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct PinsFile {
    /// Reader, then project id, then API ids in pin order.
    readers: BTreeMap<String, BTreeMap<String, Vec<ItemId>>>,
}

fn decode(text: &str) -> Option<PinsFile> {
    if text.trim().is_empty() {
        return Some(PinsFile::default());
    }
    match serde_json::from_str(text) {
        Ok(file) => Some(file),
        Err(error) => {
            warn!(%error, "pins could not be read");
            None
        }
    }
}

fn encode(file: &PinsFile) -> Option<String> {
    match serde_json::to_string(file) {
        Ok(text) => Some(text),
        Err(error) => {
            warn!(%error, "pins could not be saved");
            None
        }
    }
}

/// `None` only when a file is there and cannot be parsed, or cannot be
/// read. A missing file is an empty list, so the first pin can create it.
#[cfg(not(target_family = "wasm"))]
fn read_path(path: &std::path::Path) -> Option<PinsFile> {
    match std::fs::read_to_string(path) {
        Ok(text) => decode(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(PinsFile::default()),
        Err(error) => {
            warn!(%error, "pins could not be read");
            None
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn write_path(path: &std::path::Path, file: &PinsFile) -> bool {
    let Some(text) = encode(file) else {
        return false;
    };
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
        && let Err(error) = std::fs::create_dir_all(dir)
    {
        warn!(%error, "pins could not be saved");
        return false;
    }
    match std::fs::write(path, text) {
        Ok(()) => true,
        Err(error) => {
            warn!(%error, "pins could not be saved");
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
        static FILE: RefCell<super::PinsFile> = RefCell::new(super::PinsFile::default());
    }

    #[cfg(test)]
    pub(super) fn read_document() -> Option<super::PinsFile> {
        Some(FILE.with(|file| file.borrow().clone()))
    }

    #[cfg(test)]
    pub(super) fn write_document(file: &super::PinsFile) -> bool {
        FILE.with(|slot| *slot.borrow_mut() = file.clone());
        true
    }

    #[cfg(not(test))]
    pub(super) fn read_document() -> Option<super::PinsFile> {
        let Some(path) = crate::paths::data_dir().map(|dir| dir.join("pins.json")) else {
            tracing::warn!("pins could not be read");
            return None;
        };
        super::read_path(&path)
    }

    #[cfg(not(test))]
    pub(super) fn write_document(file: &super::PinsFile) -> bool {
        let Some(path) = crate::paths::data_dir().map(|dir| dir.join("pins.json")) else {
            tracing::warn!("pins could not be saved");
            return false;
        };
        super::write_path(&path, file)
    }
}

#[cfg(target_family = "wasm")]
mod storage {
    use super::{decode, encode};

    const STORAGE_KEY: &str = "cyberapi.pins";

    pub(super) fn read_document() -> Option<super::PinsFile> {
        let storage = web_sys::window()?.local_storage().ok().flatten()?;
        match storage.get_item(STORAGE_KEY) {
            Ok(text) => decode(text.as_deref().unwrap_or_default()),
            Err(error) => {
                tracing::warn!(?error, "pins could not be read");
                None
            }
        }
    }

    pub(super) fn write_document(file: &super::PinsFile) -> bool {
        let Some(text) = encode(file) else {
            return false;
        };
        let Some(window) = web_sys::window() else {
            tracing::warn!("pins could not be saved");
            return false;
        };
        let Ok(Some(storage)) = window.local_storage() else {
            tracing::warn!("pins could not be saved");
            return false;
        };
        if let Err(error) = storage.set_item(STORAGE_KEY, &text) {
            tracing::warn!(?error, "pins could not be saved");
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_is_kept_until_it_is_removed() {
        let project = CollectionId::new("shop");
        let other = CollectionId::new("blog");
        let api = ItemId::new("login");
        assert_eq!(get("", &project), None, "no list is not an empty one");

        assert!(set("", &project, std::slice::from_ref(&api)));
        assert_eq!(get("", &project), Some(vec![api.clone()]));
        assert!(set("", &project, &[]));
        assert_eq!(get("", &project), Some(vec![]), "unpinning everything is remembered");

        assert!(set("alice", &other, std::slice::from_ref(&api)));
        remove("", &project);
        assert_eq!(get("", &project), None);
        assert_eq!(get("alice", &other), Some(vec![api]));

        clear_scope("alice");
        assert_eq!(get("alice", &other), None);
    }

    #[test]
    fn the_desktop_is_one_reader() {
        assert_eq!(scope(Some("alice")), "");
        assert_eq!(scope(None), "");
    }

    #[test]
    fn a_file_round_trips_and_a_broken_one_is_left_alone() {
        let dir = tempfile::tempdir().expect("a directory");
        let path = dir.path().join("pins.json");
        assert!(read_path(&path).is_some_and(|file| file.readers.is_empty()));

        let mut file = PinsFile::default();
        file.readers
            .entry(String::new())
            .or_default()
            .insert("shop".into(), vec![ItemId::new("login")]);
        assert!(write_path(&path, &file));
        assert_eq!(read_path(&path), Some(file));

        std::fs::write(&path, "{").expect("break it");
        assert_eq!(read_path(&path), None);
        let broken = std::fs::read_to_string(&path).expect("still there");
        assert_eq!(broken, "{", "a list that cannot be read is not replaced");
    }
}
