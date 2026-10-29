//! Where the desktop keeps its data.

use std::path::PathBuf;

/// The database file inside [`data_dir`]. A debug build (`make dev`) uses
/// its own file, so it does not open the installed app's `cyberapi.db`.
#[cfg(not(debug_assertions))]
pub const DATABASE_FILE: &str = "cyberapi.db";

/// See [`DATABASE_FILE`].
#[cfg(debug_assertions)]
pub const DATABASE_FILE: &str = "cyberapi-dev.db";

/// The log files' name in [`logs_dir`], each with its day after it
/// (`cyberapi.log.2026-10-27`); a debug build's are its own, as its
/// database is.
#[cfg(not(debug_assertions))]
pub const LOG_PREFIX: &str = "cyberapi.log";

/// See [`LOG_PREFIX`].
#[cfg(debug_assertions)]
pub const LOG_PREFIX: &str = "cyberapi-dev.log";

/// How many days of logs are kept: the oldest go as new ones come.
pub const LOG_DAYS: usize = 7;

/// F6: the desktop's own backups' folder in [`data_dir`]; a debug build's
/// is its own, as its database is, so it never prunes the installed app's.
#[cfg(not(debug_assertions))]
const BACKUPS_DIR: &str = "backups";

/// See [`BACKUPS_DIR`].
#[cfg(debug_assertions)]
const BACKUPS_DIR: &str = "backups-dev";

/// F6: where the desktop keeps its own backups.
pub fn backups_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join(BACKUPS_DIR))
}

/// Where the logs go: `logs` in the data directory.
pub fn logs_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("logs"))
}

/// This app's data directory: the platform's data directory plus `CyberAPI`
/// (`cyberapi` on Linux, where directories are lower case). Never the Tauri
/// app's `com.bigtree.cyberapi`, which a case-insensitive file system would
/// take for the same name with any other spelling of it.
///
/// `CYBERAPI_DATA_DIR` overrides it — for development, and for trying a build
/// without touching the real data.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CYBERAPI_DATA_DIR") {
        return Some(PathBuf::from(dir));
    }
    let name = if cfg!(any(target_os = "linux", target_os = "freebsd")) {
        "cyberapi"
    } else {
        "CyberAPI"
    };
    directories::BaseDirs::new().map(|dirs| dirs.data_dir().join(name))
}

/// Where the Tauri app kept its data, to copy it from on the first launch.
/// `CYBERAPI_LEGACY_DIR` overrides it.
pub fn legacy_dir() -> Option<PathBuf> {
    std::env::var_os("CYBERAPI_LEGACY_DIR")
        .map(PathBuf::from)
        .or_else(cyberapi_store_turso::tauri_data_dir)
}
