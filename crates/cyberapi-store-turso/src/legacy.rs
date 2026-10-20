//! Bringing over the Tauri app's data (D2 / D4 in `docs/architecture.md`).
//!
//! The new app keeps its data in a directory of its own and copies the old
//! app's files in once, on the first launch that finds them. The originals
//! are only read: the old app keeps working on them, and turso — which turns
//! a file it opens into WAL mode — never touches them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The Tauri app's bundle identifier, which names its data directory.
pub const TAURI_IDENTIFIER: &str = "com.bigtree.cyberapi";

const LEGACY_DATABASE: &str = "my_db.db";
const LEGACY_COOKIES: &str = "cookies.json";

/// Where the Tauri app kept its data: its `app_data_dir`, which Tauri 1 puts
/// under the platform's data directory (`~/Library/Application Support` on
/// macOS, `%APPDATA%` on Windows, `~/.local/share` on Linux).
pub fn tauri_data_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.data_dir().join(TAURI_IDENTIFIER))
}

/// What [`copy_legacy_data`] brought over.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegacyCopy {
    /// The database, with its `-wal` / `-shm` files when there were any.
    pub database: bool,
    /// `cookies.json`, copied beside the database as `legacy-cookies.json`
    /// for the cookie store to import.
    pub cookies: bool,
}

/// Copies the old app's database from `legacy_dir` to `database` — once:
/// when `database` does not exist yet and the old one does. Its cookies come
/// along beside it.
///
/// The database is copied under a temporary name and renamed last, so a copy
/// cut short leaves no file that would be taken for the database next time.
pub fn copy_legacy_data(legacy_dir: &Path, database: &Path) -> io::Result<LegacyCopy> {
    let mut copied = LegacyCopy::default();
    let source = legacy_dir.join(LEGACY_DATABASE);
    if database.exists() || !source.is_file() {
        return Ok(copied);
    }
    if let Some(parent) = database.parent() {
        fs::create_dir_all(parent)?;
    }
    for suffix in ["-wal", "-shm"] {
        let side = with_suffix(&source, suffix);
        if side.is_file() {
            fs::copy(&side, with_suffix(database, suffix))?;
        }
    }
    let partial = with_suffix(database, ".copying");
    fs::copy(&source, &partial)?;
    fs::rename(&partial, database)?;
    copied.database = true;

    let cookies = legacy_dir.join(LEGACY_COOKIES);
    if cookies.is_file()
        && let Some(parent) = database.parent()
    {
        fs::copy(&cookies, parent.join("legacy-cookies.json"))?;
        copied.cookies = true;
    }
    Ok(copied)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_once_and_leaves_the_originals() -> io::Result<()> {
        let legacy = tempfile::tempdir()?;
        let data = tempfile::tempdir()?;
        fs::write(legacy.path().join(LEGACY_DATABASE), b"old database")?;
        fs::write(legacy.path().join("my_db.db-wal"), b"old wal")?;
        fs::write(legacy.path().join(LEGACY_COOKIES), b"{}")?;
        let database = data.path().join("nested").join("cyberapi.db");

        let copied = copy_legacy_data(legacy.path(), &database)?;
        assert_eq!(
            copied,
            LegacyCopy {
                database: true,
                cookies: true
            }
        );
        assert_eq!(fs::read(&database)?, b"old database");
        assert_eq!(fs::read(with_suffix(&database, "-wal"))?, b"old wal");
        assert!(!with_suffix(&database, "-shm").exists());
        assert!(!with_suffix(&database, ".copying").exists());
        assert_eq!(fs::read(database.with_file_name("legacy-cookies.json"))?, b"{}");
        assert_eq!(fs::read(legacy.path().join(LEGACY_DATABASE))?, b"old database");

        // The new database is the one in use from now on.
        fs::write(&database, b"new database")?;
        assert_eq!(copy_legacy_data(legacy.path(), &database)?, LegacyCopy::default());
        assert_eq!(fs::read(&database)?, b"new database");
        Ok(())
    }

    #[test]
    fn nothing_to_copy() -> io::Result<()> {
        let legacy = tempfile::tempdir()?;
        let data = tempfile::tempdir()?;
        let database = data.path().join("cyberapi.db");
        assert_eq!(copy_legacy_data(legacy.path(), &database)?, LegacyCopy::default());
        assert!(!database.exists());
        Ok(())
    }
}
