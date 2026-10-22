//! Server projects' unsynced changes, kept in this browser
//! (`docs/server-sync.md` §4).
//!
//! A draft is a project's whole configuration as the reader left it, and
//! the server's revision it was made over. It is there while something is
//! unsynced: syncing, or giving the changes up, removes it. In a page each
//! draft is a `localStorage` entry of its own, under the account and the
//! project, so a refresh finds it and the tabs of one browser share it —
//! the later save wins. The desktop keeps none (its server projects are
//! copies in its database), so natively the entries are this thread's, for
//! the tests.

use cyberapi_core::CollectionId;
use cyberapi_core::sync::ProjectConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::warn;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Draft {
    /// The server's revision the changes were made over; `None` when the
    /// project was never pulled here.
    pub base: Option<u64>,
    pub config: ProjectConfig,
}

const PREFIX: &str = "cyberapi.draft.";

fn prefix(scope: &str) -> String {
    format!("{PREFIX}{scope}.")
}

fn key(scope: &str, collection: &CollectionId) -> String {
    format!("{}{}", prefix(scope), collection.as_str())
}

fn decode(text: &str) -> Option<Draft> {
    match serde_json::from_str(text) {
        Ok(draft) => Some(draft),
        Err(error) => {
            warn!(%error, "a draft could not be read");
            None
        }
    }
}

/// The reader's draft of `collection`, if there is one that reads.
pub fn get(scope: &str, collection: &CollectionId) -> Option<Draft> {
    decode(&storage::read(&key(scope, collection))?)
}

/// Keeps `draft` in place of the one there was. `false` when this browser
/// could not keep it — out of room, or storage turned off.
pub fn set(scope: &str, collection: &CollectionId, draft: &Draft) -> bool {
    match serde_json::to_string(draft) {
        Ok(text) => storage::write(&key(scope, collection), &text),
        Err(error) => {
            warn!(%error, "a draft could not be saved");
            false
        }
    }
}

pub fn remove(scope: &str, collection: &CollectionId) {
    storage::delete(&key(scope, collection));
}

/// Every draft of the reader's, by project.
pub fn all(scope: &str) -> BTreeMap<CollectionId, Draft> {
    let prefix = prefix(scope);
    storage::keys()
        .into_iter()
        .filter_map(|key| {
            let id = key.strip_prefix(&prefix)?;
            // Another reader whose id starts as this one's does.
            if id.contains('.') {
                return None;
            }
            Some((CollectionId::new(id), decode(&storage::read(&key)?)?))
        })
        .collect()
}

#[cfg(not(target_family = "wasm"))]
mod storage {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    thread_local! {
        static ENTRIES: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
    }

    pub(super) fn read(key: &str) -> Option<String> {
        ENTRIES.with(|entries| entries.borrow().get(key).cloned())
    }

    pub(super) fn write(key: &str, text: &str) -> bool {
        ENTRIES.with(|entries| entries.borrow_mut().insert(key.to_string(), text.to_string()));
        true
    }

    pub(super) fn delete(key: &str) {
        ENTRIES.with(|entries| entries.borrow_mut().remove(key));
    }

    pub(super) fn keys() -> Vec<String> {
        ENTRIES.with(|entries| entries.borrow().keys().cloned().collect())
    }
}

/// The page runs gpui without threads (`cyberapi-web`), so a store's future
/// that keeps a draft runs on the page's thread, where `localStorage` is.
#[cfg(target_family = "wasm")]
mod storage {
    fn local() -> Option<web_sys::Storage> {
        let storage = web_sys::window()?.local_storage().ok().flatten();
        if storage.is_none() {
            tracing::warn!("this browser keeps no drafts");
        }
        storage
    }

    pub(super) fn read(key: &str) -> Option<String> {
        match local()?.get_item(key) {
            Ok(text) => text,
            Err(error) => {
                tracing::warn!(?error, "a draft could not be read");
                None
            }
        }
    }

    pub(super) fn write(key: &str, text: &str) -> bool {
        let Some(storage) = local() else {
            return false;
        };
        if let Err(error) = storage.set_item(key, text) {
            tracing::warn!(?error, "a draft could not be saved");
            return false;
        }
        true
    }

    pub(super) fn delete(key: &str) {
        if let Some(storage) = local()
            && let Err(error) = storage.remove_item(key)
        {
            tracing::warn!(?error, "a draft could not be removed");
        }
    }

    pub(super) fn keys() -> Vec<String> {
        let Some(storage) = local() else {
            return Vec::new();
        };
        let count = storage.length().unwrap_or_default();
        (0..count)
            .filter_map(|index| storage.key(index).ok().flatten())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(name: &str, base: u64) -> Draft {
        Draft {
            base: Some(base),
            config: ProjectConfig {
                name: name.into(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn drafts_are_kept_by_reader_and_project() {
        let (shop, blog) = (CollectionId::new("shop"), CollectionId::new("blog"));
        assert_eq!(get("alice", &shop), None);

        assert!(set("alice", &shop, &draft("shop", 3)));
        assert!(set("alice", &blog, &draft("blog", 1)));
        assert!(set("alice.b", &shop, &draft("not alice's", 9)));
        assert_eq!(get("alice", &shop), Some(draft("shop", 3)));
        assert!(set("alice", &shop, &draft("shop 2", 4)));
        assert_eq!(get("alice", &shop), Some(draft("shop 2", 4)), "the later save wins");

        let all = all("alice");
        assert_eq!(all.keys().cloned().collect::<Vec<_>>(), [blog.clone(), shop.clone()]);
        remove("alice", &shop);
        assert_eq!(get("alice", &shop), None);
        assert_eq!(get("alice.b", &shop), Some(draft("not alice's", 9)));
        assert_eq!(super::all("alice").len(), 1);
    }
}
