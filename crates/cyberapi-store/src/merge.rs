//! Applying a draft's changes onto a project that still has every request.
//!
//! The read and the replace are the store's own. A page must not run this
//! on a working copy that only holds heads: the replace would delete every
//! request the copy never loaded. The bridge runs it on the server, where
//! the configuration is whole.

use crate::{StoreError, TreeStore};
use cyberapi_core::sync::{ProjectChanges, apply_changes, touch_changed};
use cyberapi_core::{CollectionId, Timestamp};

/// How many times a merge with no base retries when the project moves
/// between the read and the write.
const TRIES: usize = 8;

/// Reads the project, applies `changes`, and replaces it at the revision it
/// was read at. See [`TreeStore::merge`](crate::TreeStore::merge).
pub async fn merge_changes(
    tree: &impl TreeStore,
    collection: CollectionId,
    changes: ProjectChanges,
    base: Option<u64>,
    now: Timestamp,
) -> Result<u64, StoreError> {
    for _ in 0..TRIES {
        let at = tree.config(collection.clone()).await?;
        if let Some(base) = base
            && at.revision != base
        {
            return Err(StoreError::Stale {
                base,
                revision: at.revision,
            });
        }
        if changes.is_empty() {
            return Ok(at.revision);
        }
        let mut config = at.config;
        apply_changes(&mut config, &changes)?;
        touch_changed(&mut config, &changes, now);
        match tree.replace(collection.clone(), config, Some(at.revision)).await {
            Ok(revision) => return Ok(revision),
            Err(StoreError::Stale { .. }) if base.is_none() => continue,
            Err(error) => return Err(error),
        }
    }
    Err(StoreError::backend(
        "the project kept moving while its changes were merged",
    ))
}
