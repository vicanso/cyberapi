//! The desktop's backups (M13): everything the store holds as the old app's
//! zip — one deflated `{table}.json` per table (`cyberapi_core::backup`) —
//! and back. Either app restores the other's.

use chrono::{Datelike as _, Timelike as _};
use cyberapi_core::Timestamp;
use cyberapi_core::backup::Backup;
use cyberapi_store::{BackupStore, StoreError};
use snafu::{ResultExt as _, Snafu};
use std::io::{Cursor, Read as _, Write as _};
use std::sync::Arc;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

#[derive(Debug, Snafu)]
pub enum BackupFailure {
    #[snafu(display("{source}"))]
    Store { source: StoreError },
    /// The file is no backup: not a zip, or a table missing or unreadable.
    /// Nothing was restored.
    #[snafu(display("{message}"))]
    NotABackup { message: String },
    #[snafu(display("{message}"))]
    Write { message: String },
}

/// F13.1: the whole store as a backup file's bytes, its tables dated `at`.
pub async fn back_up(store: Arc<dyn BackupStore>, at: Timestamp) -> Result<Vec<u8>, BackupFailure> {
    let backup = store.snapshot().await.context(StoreSnafu)?;
    pack(&backup, &at).map_err(|error| BackupFailure::Write {
        message: error.to_string(),
    })
}

/// A zip's time is the wall clock's, with no zone; one it cannot hold
/// (before 1980) is its earliest.
fn zip_time(at: &Timestamp) -> zip::DateTime {
    let at = at.get();
    let small = |n: u32| u8::try_from(n).unwrap_or_default();
    zip::DateTime::from_date_and_time(
        u16::try_from(at.year()).unwrap_or_default(),
        small(at.month()),
        small(at.day()),
        small(at.hour()),
        small(at.minute()),
        small(at.second()),
    )
    .unwrap_or_default()
}

fn pack(backup: &Backup, at: &Timestamp) -> zip::result::ZipResult<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip_time(at));
    for (name, bytes) in backup.to_files() {
        zip.start_file(name, options)?;
        zip.write_all(&bytes)?;
    }
    Ok(zip.finish()?.into_inner())
}

/// F13.2: everything replaced by the backup in `zip`, at once (#8). A file
/// that is not a backup changes nothing.
pub async fn restore(store: Arc<dyn BackupStore>, zip: Vec<u8>) -> Result<(), BackupFailure> {
    let backup = read(&zip)?;
    store.restore(backup).await.context(StoreSnafu)
}

/// A table that cannot be read out of the zip counts as missing.
fn read(zip: &[u8]) -> Result<Backup, BackupFailure> {
    let not_a_backup = |message: String| BackupFailure::NotABackup { message };
    let mut archive = ZipArchive::new(Cursor::new(zip)).map_err(|error| not_a_backup(error.to_string()))?;
    Backup::from_files(|name| {
        let mut file = archive.by_name(name).ok()?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).ok()?;
        Some(bytes)
    })
    .map_err(|error| not_a_backup(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::backup::{APIS_FILE, COLLECTIONS_FILE, FOLDERS_FILE, VARIABLES_FILE};
    use cyberapi_store::{MemoryStore, NewCollection};
    use pretty_assertions::assert_eq;

    fn store_with(name: &str) -> MemoryStore {
        let store = MemoryStore::new();
        pollster::block_on(store.storage().collections().create(NewCollection {
            name: name.into(),
            ..Default::default()
        }))
        .expect("create");
        store
    }

    fn names(store: &MemoryStore) -> Vec<String> {
        pollster::block_on(store.storage().collections().list())
            .expect("list")
            .into_iter()
            .map(|collection| collection.name)
            .collect()
    }

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in files {
            zip.start_file(*name, SimpleFileOptions::default()).expect("start");
            zip.write_all(text.as_bytes()).expect("write");
        }
        zip.finish().expect("finish").into_inner()
    }

    #[test]
    fn a_backup_restores_what_it_held() {
        let store = store_with("shop");
        let at = Timestamp::parse_lenient("2026-10-21T14:15:32+08:00");
        let bytes = pollster::block_on(back_up(Arc::new(store.clone()), at)).expect("backed up");
        let mut archive = ZipArchive::new(Cursor::new(&bytes)).expect("a zip");
        let modified = archive.by_index(0).expect("a table").last_modified().expect("a time");
        assert_eq!(
            (modified.year(), modified.month(), modified.hour(), modified.second()),
            (2026, 10, 14, 32),
            "the wall clock's time"
        );
        let other = store_with("other");
        pollster::block_on(restore(Arc::new(other.clone()), bytes)).expect("restored");
        assert_eq!(names(&other), ["shop"]);
    }

    #[test]
    fn the_old_apps_backup_restores() {
        let bytes = zip_of(&[
            (
                COLLECTIONS_FILE,
                r#"[{"id":"c","name":"legacy","description":"","created_at":"2022-01-02T03:04:05+08:00","updated_at":"2022-01-02T03:04:05+08:00"}]"#,
            ),
            (FOLDERS_FILE, "[]"),
            (
                APIS_FILE,
                r#"[{"id":"a","collection":"c","name":"ping","category":"http","setting":"{\"method\":\"GET\",\"uri\":\"/ping\"}","created_at":"","updated_at":""}]"#,
            ),
            (VARIABLES_FILE, "[]"),
        ]);
        let store = store_with("mine");
        pollster::block_on(restore(Arc::new(store.clone()), bytes)).expect("restored");
        assert_eq!(names(&store), ["legacy"]);
    }

    #[test]
    fn what_is_not_a_backup_changes_nothing() {
        let store = store_with("mine");
        let not_zip = pollster::block_on(restore(Arc::new(store.clone()), b"not a zip".to_vec()));
        assert!(matches!(not_zip, Err(BackupFailure::NotABackup { .. })), "{not_zip:?}");
        let partial = zip_of(&[(COLLECTIONS_FILE, "[]"), (FOLDERS_FILE, "[]")]);
        let missing = pollster::block_on(restore(Arc::new(store.clone()), partial));
        assert!(
            matches!(&missing, Err(BackupFailure::NotABackup { message }) if message.contains(APIS_FILE)),
            "{missing:?}"
        );
        assert_eq!(names(&store), ["mine"]);
    }
}
