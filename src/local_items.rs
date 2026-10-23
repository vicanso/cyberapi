//! The items each reader keeps to themselves in a server project
//! (`docs/server-sync.md` §3.4, `cyberapi_core::local_only`), kept on this
//! machine: `local-items.json` beside the database on the desktop,
//! `localStorage` in the browser. They are never synced, so nothing else
//! has them: here is where a pull finds them to put back.

use cyberapi_core::CollectionId;
use cyberapi_core::local_only::LocalItems;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::warn;

/// What the reader keeps of `collection`; nothing yet is the default.
pub fn get(scope: &str, collection: &CollectionId) -> LocalItems {
    storage::read_document()
        .and_then(|mut file| file.readers.get_mut(scope)?.remove(collection.as_str()))
        .unwrap_or_default()
}

/// Keeps `items` in place of what there was. `false` when they could not
/// be kept.
pub fn set(scope: &str, collection: &CollectionId, items: &LocalItems) -> bool {
    let Some(mut file) = storage::read_document() else {
        return false;
    };
    let projects = file.readers.entry(scope.to_string()).or_default();
    if projects.get(collection.as_str()) == Some(items) {
        return true;
    }
    projects.insert(collection.as_str().to_string(), items.clone());
    storage::write_document(&file)
}

/// Forgets a project that is gone.
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct LocalFile {
    /// Reader, then project id.
    readers: BTreeMap<String, BTreeMap<String, LocalItems>>,
}

fn decode(text: &str) -> Option<LocalFile> {
    if text.trim().is_empty() {
        return Some(LocalFile::default());
    }
    match serde_json::from_str(text) {
        Ok(file) => Some(file),
        Err(error) => {
            warn!(%error, "the reader's own items could not be read");
            None
        }
    }
}

fn encode(file: &LocalFile) -> Option<String> {
    match serde_json::to_string(file) {
        Ok(text) => Some(text),
        Err(error) => {
            warn!(%error, "the reader's own items could not be saved");
            None
        }
    }
}

/// `None` only when a file is there and cannot be read: what it holds is
/// not replaced then. A missing file holds nothing yet.
#[cfg(not(target_family = "wasm"))]
fn read_path(path: &std::path::Path) -> Option<LocalFile> {
    match std::fs::read_to_string(path) {
        Ok(text) => decode(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(LocalFile::default()),
        Err(error) => {
            warn!(%error, "the reader's own items could not be read");
            None
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn write_path(path: &std::path::Path, file: &LocalFile) -> bool {
    let Some(text) = encode(file) else {
        return false;
    };
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
        && let Err(error) = std::fs::create_dir_all(dir)
    {
        warn!(%error, "the reader's own items could not be saved");
        return false;
    }
    match std::fs::write(path, text) {
        Ok(()) => true,
        Err(error) => {
            warn!(%error, "the reader's own items could not be saved");
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
        static FILE: RefCell<super::LocalFile> = RefCell::new(super::LocalFile::default());
    }

    #[cfg(test)]
    pub(super) fn read_document() -> Option<super::LocalFile> {
        Some(FILE.with(|file| file.borrow().clone()))
    }

    #[cfg(test)]
    pub(super) fn write_document(file: &super::LocalFile) -> bool {
        FILE.with(|slot| *slot.borrow_mut() = file.clone());
        true
    }

    #[cfg(not(test))]
    fn path() -> Option<std::path::PathBuf> {
        crate::paths::data_dir().map(|dir| dir.join("local-items.json"))
    }

    #[cfg(not(test))]
    pub(super) fn read_document() -> Option<super::LocalFile> {
        super::read_path(&path()?)
    }

    #[cfg(not(test))]
    pub(super) fn write_document(file: &super::LocalFile) -> bool {
        path().is_some_and(|path| super::write_path(&path, file))
    }
}

#[cfg(target_family = "wasm")]
mod storage {
    use super::{decode, encode};

    const STORAGE_KEY: &str = "cyberapi.localItems";

    fn local() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }

    pub(super) fn read_document() -> Option<super::LocalFile> {
        match local()?.get_item(STORAGE_KEY) {
            Ok(text) => decode(text.as_deref().unwrap_or_default()),
            Err(error) => {
                tracing::warn!(?error, "the reader's own items could not be read");
                None
            }
        }
    }

    pub(super) fn write_document(file: &super::LocalFile) -> bool {
        let (Some(text), Some(storage)) = (encode(file), local()) else {
            return false;
        };
        if let Err(error) = storage.set_item(STORAGE_KEY, &text) {
            tracing::warn!(?error, "the reader's own items could not be saved");
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::ItemId;

    #[test]
    fn a_projects_items_are_kept_until_it_goes() {
        let shop = CollectionId::new("shop");
        assert_eq!(get("", &shop), LocalItems::default());
        let mut items = LocalItems::default();
        items.mark(&ItemId::new("mine"));
        assert!(set("", &shop, &items));
        assert_eq!(get("", &shop), items);
        assert_eq!(get("alice", &shop), LocalItems::default(), "each reader's own");
        remove("", &shop);
        assert_eq!(get("", &shop), LocalItems::default());
    }

    #[test]
    fn a_file_round_trips_and_a_broken_one_is_left_alone() {
        let dir = tempfile::tempdir().expect("a directory");
        let path = dir.path().join("local-items.json");
        assert!(read_path(&path).is_some_and(|file| file.readers.is_empty()));
        let mut file = LocalFile::default();
        file.readers
            .entry(String::new())
            .or_default()
            .insert("shop".into(), LocalItems::default());
        assert!(write_path(&path, &file));
        assert_eq!(read_path(&path), Some(file));
        std::fs::write(&path, "{").expect("break it");
        assert_eq!(read_path(&path), None);
    }
}
