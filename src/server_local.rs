//! The desktop's bridge and its session (`docs/server-sync.md` §1, step 5):
//! `server.json` beside the database, never in it, so a backup holds no
//! login. Readable by its owner only. The page signs in by its own cookie
//! and keeps nothing here.

use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerFile {
    /// The one bridge this machine signs in to, as typed and checked.
    pub address: String,
    /// The session's token while signed in.
    pub token: Option<String>,
}

fn decode(text: &str) -> ServerFile {
    serde_json::from_str(text).unwrap_or_else(|error| {
        warn!(%error, "the server file could not be read");
        ServerFile::default()
    })
}

pub fn read() -> ServerFile {
    storage::read().map(|text| decode(&text)).unwrap_or_default()
}

/// `false` when it could not be written.
pub fn write(file: &ServerFile) -> bool {
    match serde_json::to_string_pretty(file) {
        Ok(text) => storage::write(&text),
        Err(error) => {
            warn!(%error, "the server file could not be saved");
            false
        }
    }
}

#[cfg(all(not(target_family = "wasm"), not(test)))]
mod storage {
    use std::path::PathBuf;

    fn path() -> Option<PathBuf> {
        crate::paths::data_dir().map(|dir| dir.join("server.json"))
    }

    pub(super) fn read() -> Option<String> {
        match std::fs::read_to_string(path()?) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                tracing::warn!(%error, "the server file could not be read");
                None
            }
        }
    }

    pub(super) fn write(text: &str) -> bool {
        let Some(path) = path() else { return false };
        if let Some(dir) = path.parent()
            && let Err(error) = std::fs::create_dir_all(dir)
        {
            tracing::warn!(%error, "the server file could not be saved");
            return false;
        }
        if let Err(error) = std::fs::write(&path, text) {
            tracing::warn!(%error, "the server file could not be saved");
            return false;
        }
        owner_only(&path);
        true
    }

    /// The token is a login: no one else on the machine reads it.
    #[cfg(unix)]
    fn owner_only(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt as _;
        if let Err(error) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(%error, "the server file is readable by others");
        }
    }
    #[cfg(not(unix))]
    fn owner_only(_: &std::path::Path) {}
}

/// The tests' file is the thread's.
#[cfg(all(not(target_family = "wasm"), test))]
mod storage {
    use std::cell::RefCell;

    thread_local! {
        static TEXT: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    pub(super) fn read() -> Option<String> {
        TEXT.with(|text| text.borrow().clone())
    }

    pub(super) fn write(text: &str) -> bool {
        TEXT.with(|slot| *slot.borrow_mut() = Some(text.to_string()));
        true
    }
}

#[cfg(target_family = "wasm")]
mod storage {
    pub(super) fn read() -> Option<String> {
        None
    }

    pub(super) fn write(_: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_round_trips() {
        assert_eq!(read(), ServerFile::default());
        let file = ServerFile {
            address: "https://api.test".into(),
            token: Some("tok".into()),
        };
        assert!(write(&file));
        assert_eq!(read(), file);
        assert_eq!(decode("{"), ServerFile::default(), "a broken file is no login");
    }
}
