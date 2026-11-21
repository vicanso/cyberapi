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
    /// Whose session: the account whose server projects this machine shows
    /// and edits, offline too (`docs/server-sync.md` §6).
    pub account: Option<String>,
}

fn decode(text: &str) -> ServerFile {
    serde_json::from_str(text).unwrap_or_else(|error| {
        warn!(%error, "the server file could not be read");
        ServerFile::default()
    })
}

pub fn read() -> ServerFile {
    #[allow(unused_mut)]
    let mut file = storage::read().map(|text| decode(&text)).unwrap_or_default();
    #[cfg(not(target_family = "wasm"))]
    if let Some(raw) = file.token.take()
        && !raw.is_empty()
    {
        file.token = crate::master_key::decrypt(&raw);
    }
    file
}

/// `false` when it could not be written.
pub fn write(file: &ServerFile) -> bool {
    #[cfg(not(target_family = "wasm"))]
    let stored = {
        let mut copy = file.clone();
        if let Some(plain) = copy.token.as_ref()
            && !plain.is_empty()
        {
            copy.token = Some(crate::master_key::encrypt(plain));
        }
        copy
    };
    #[cfg(target_family = "wasm")]
    let stored = file;
    match serde_json::to_string_pretty(&stored) {
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

#[cfg(all(not(target_family = "wasm"), test))]
mod f18_tests {
    use super::*;

    #[test]
    fn token_is_encrypted_at_rest_and_legacy_plaintext_still_reads() {
        let file = ServerFile {
            address: "https://bridge.test".into(),
            token: Some("session-secret-xyz".into()),
            account: Some("acc-1".into()),
        };
        assert!(write(&file));
        let raw = storage::read().expect("written");
        assert!(
            !raw.contains("session-secret-xyz"),
            "plaintext session token must not appear in serialized server.json"
        );
        assert!(raw.contains("enc:v1:"), "stored token should carry enc:v1: prefix");
        assert_eq!(read(), file);

        // Legacy plaintext server.json still reads transparently.
        assert!(storage::write(
            r#"{"address":"https://bridge.test","token":"legacy-plain-tok","account":"acc-1"}"#
        ));
        assert_eq!(read().token.as_deref(), Some("legacy-plain-tok"));
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
            account: Some("a1".into()),
        };
        assert!(write(&file));
        assert_eq!(read(), file);
        assert_eq!(decode("{"), ServerFile::default(), "a broken file is no login");
    }
}
