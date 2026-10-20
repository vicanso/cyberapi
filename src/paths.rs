//! Where the desktop keeps its data.

use std::path::PathBuf;

/// The database file inside [`data_dir`].
pub const DATABASE_FILE: &str = "cyberapi.db";

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
