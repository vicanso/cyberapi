//! H2: the desktop's local loopback MCP server configuration (`mcp.json`
//! beside the database, never in it, so a backup holds no token). Readable
//! by its owner only.

use cyberapi_service::mcp::generate_local_token;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// Default port for the desktop's local MCP server (`127.0.0.1:4020`).
pub const DEFAULT_PORT: u16 = 4020;

fn default_port() -> u16 {
    DEFAULT_PORT
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpFile {
    /// Port on `127.0.0.1` (`1..=65535`).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Bearer token (`cat_local_…`) required in `Authorization: Bearer`.
    pub token: String,
    /// Whether `send_api` is enabled.
    pub can_send: bool,
    /// Whether `create_api` and `update_api` are enabled.
    pub can_write: bool,
    /// Whether the server starts automatically on launch.
    pub enabled: bool,
}

impl Default for McpFile {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            token: generate_local_token(),
            can_send: false,
            can_write: false,
            enabled: false,
        }
    }
}

impl McpFile {
    fn normalized(mut self) -> Self {
        if self.port == 0 {
            self.port = DEFAULT_PORT;
        }
        if self.token.trim().is_empty() {
            self.token = generate_local_token();
        }
        self
    }
}

fn decode(text: &str) -> McpFile {
    serde_json::from_str::<McpFile>(text)
        .unwrap_or_else(|error| {
            warn!(%error, "the local MCP file could not be read");
            McpFile::default()
        })
        .normalized()
}

pub fn read() -> McpFile {
    match storage::read() {
        Some(text) => decode(&text),
        None => {
            let fresh = McpFile::default();
            let _ = write(&fresh);
            fresh
        }
    }
}

pub fn write(file: &McpFile) -> bool {
    match serde_json::to_string_pretty(file) {
        Ok(text) => storage::write(&text),
        Err(error) => {
            warn!(%error, "the local MCP file could not be saved");
            false
        }
    }
}

#[cfg(all(not(target_family = "wasm"), not(test)))]
mod storage {
    use std::path::PathBuf;

    fn path() -> Option<PathBuf> {
        crate::paths::data_dir().map(|dir| dir.join("mcp.json"))
    }

    pub(super) fn read() -> Option<String> {
        match std::fs::read_to_string(path()?) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                tracing::warn!(%error, "the local MCP file could not be read");
                None
            }
        }
    }

    pub(super) fn write(text: &str) -> bool {
        let Some(path) = path() else { return false };
        if let Some(dir) = path.parent()
            && let Err(error) = std::fs::create_dir_all(dir)
        {
            tracing::warn!(%error, "the local MCP file could not be saved");
            return false;
        }
        if let Err(error) = std::fs::write(&path, text) {
            tracing::warn!(%error, "the local MCP file could not be saved");
            return false;
        }
        owner_only(&path);
        true
    }

    #[cfg(unix)]
    fn owner_only(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt as _;
        if let Err(error) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(%error, "the local MCP file is readable by others");
        }
    }
    #[cfg(not(unix))]
    fn owner_only(_: &std::path::Path) {}
}

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
    fn local_mcp_file_round_trips_and_generates_token() {
        let initial = read();
        assert_eq!(initial.port, DEFAULT_PORT);
        assert!(initial.token.starts_with("cat_local_"));
        let updated = McpFile {
            port: 4099,
            token: "cat_local_fixed".into(),
            can_send: true,
            can_write: true,
            enabled: true,
        };
        assert!(write(&updated));
        assert_eq!(read(), updated);
    }
}
