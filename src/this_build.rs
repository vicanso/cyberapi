//! What this build can do, and how it introduces itself — the places the
//! desktop and the browser differ in capability. Each is a pair of functions
//! with one signature, the desktop's first.

use cyberapi_core::template::FileReader;
use std::sync::Arc;

/// The `User-Agent` requests carry unless they name their own (F5.4).
#[cfg(not(target_family = "wasm"))]
pub fn user_agent() -> String {
    format!(
        "CyberAPI/{} ({}; {})",
        crate::VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}
#[cfg(target_family = "wasm")]
pub fn user_agent() -> String {
    format!("CyberAPI/{} (web)", crate::VERSION)
}

/// Whether a request can read local files: multipart file fields and the
/// `readFile` / `openFile` template functions. A page cannot.
#[cfg(not(target_family = "wasm"))]
pub fn reads_files() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn reads_files() -> bool {
    false
}

/// Whether the app sizes its own window (F14.2); a page's is the tab's.
#[cfg(not(target_family = "wasm"))]
pub fn sizes_window() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn sizes_window() -> bool {
    false
}

/// Where the data is kept, for the about section (F1.7).
#[cfg(not(target_family = "wasm"))]
pub fn data_location() -> String {
    crate::paths::data_dir()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default()
}
#[cfg(target_family = "wasm")]
pub fn data_location() -> String {
    rust_i18n::t!("settings.on_the_server").to_string()
}

/// Whether an export can be saved as a file (M11); a page copies it to the
/// clipboard instead.
#[cfg(not(target_family = "wasm"))]
pub fn saves_files() -> bool {
    true
}
#[cfg(target_family = "wasm")]
pub fn saves_files() -> bool {
    false
}

/// Saves `bytes` in the downloads folder as `name`, or `name-1` … `name-9`
/// when it is taken (F15.4); answers where.
#[cfg(not(target_family = "wasm"))]
pub fn save_to_downloads(name: &str, bytes: &[u8]) -> Result<String, String> {
    desktop::save_to_downloads(name, bytes)
}
#[cfg(target_family = "wasm")]
pub fn save_to_downloads(_: &str, _: &[u8]) -> Result<String, String> {
    Err("a page cannot save files".into())
}

/// A text file the reader picked.
#[cfg(not(target_family = "wasm"))]
pub fn read_text(path: &std::path::Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}
#[cfg(target_family = "wasm")]
pub fn read_text(_: &std::path::Path) -> Result<String, String> {
    Err("a page cannot read files".into())
}

/// A file the reader picked, as it is.
#[cfg(not(target_family = "wasm"))]
pub fn read_bytes(path: &std::path::Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))
}
#[cfg(target_family = "wasm")]
pub fn read_bytes(_: &std::path::Path) -> Result<Vec<u8>, String> {
    Err("a page cannot read files".into())
}

/// What reads those files.
#[cfg(not(target_family = "wasm"))]
pub fn files() -> Arc<dyn FileReader + Send + Sync> {
    Arc::new(desktop::Files)
}
#[cfg(target_family = "wasm")]
pub fn files() -> Arc<dyn FileReader + Send + Sync> {
    Arc::new(cyberapi_core::template::NoFiles(
        "the browser build cannot read local files",
    ))
}

#[cfg(not(target_family = "wasm"))]
mod desktop {
    use cyberapi_core::template::{BaseDir, FileReader};
    use std::path::{Path, PathBuf};

    /// Files from this machine: a name relative to the downloads, documents
    /// or desktop folder (M6), or an absolute path.
    pub struct Files;

    impl FileReader for Files {
        fn read(&self, file: &str, dir: Option<BaseDir>) -> Result<Vec<u8>, String> {
            let path = resolve(file, dir);
            std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))
        }
    }

    pub(super) fn save_to_downloads(name: &str, bytes: &[u8]) -> Result<String, String> {
        let dir = directories::UserDirs::new()
            .and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
            .ok_or("this system names no downloads folder")?;
        save_unique(&dir, name, bytes)
    }

    /// F15.4: `name`, then `stem-1.ext` … `stem-9.ext`; never over a file.
    pub(super) fn save_unique(dir: &Path, name: &str, bytes: &[u8]) -> Result<String, String> {
        let name: String = name
            .chars()
            .map(|ch| {
                if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                    '_'
                } else {
                    ch
                }
            })
            .collect();
        let (stem, extension) = match name.rsplit_once('.') {
            Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
            None => (name.clone(), String::new()),
        };
        for attempt in 0..10 {
            let candidate = if attempt == 0 {
                dir.join(&name)
            } else {
                dir.join(format!("{stem}-{attempt}{extension}"))
            };
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(mut file) => {
                    use std::io::Write as _;
                    file.write_all(bytes).map_err(|error| error.to_string())?;
                    return Ok(candidate.display().to_string());
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("{}: {error}", candidate.display())),
            }
        }
        Err(format!("{name} and nine more like it exist already"))
    }

    pub(super) fn resolve(file: &str, dir: Option<BaseDir>) -> PathBuf {
        let path = Path::new(file);
        let Some(dir) = dir.filter(|_| path.is_relative()) else {
            return path.to_path_buf();
        };
        let base = directories::UserDirs::new().and_then(|dirs| {
            match dir {
                BaseDir::Download => dirs.download_dir(),
                BaseDir::Document => dirs.document_dir(),
                BaseDir::Desktop => dirs.desktop_dir(),
            }
            .map(Path::to_path_buf)
        });
        match base {
            Some(base) => base.join(path),
            None => path.to_path_buf(),
        }
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;
    use cyberapi_core::template::BaseDir;

    #[test]
    fn the_desktop_reads_files() {
        assert!(reads_files());
        assert!(user_agent().starts_with(&format!("CyberAPI/{} (", crate::VERSION)));
        let dir = tempfile::tempdir().expect("a directory");
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"hello").expect("write");
        let path = file.to_str().expect("UTF-8");
        assert_eq!(files().read(path, Some(BaseDir::Desktop)), Ok(b"hello".to_vec()));
        assert!(files().read("/no/such/file", None).is_err());
        // Saving never overwrites (F15.4).
        let first = desktop::save_unique(dir.path(), "a/b.json", b"1").expect("saved");
        let second = desktop::save_unique(dir.path(), "a/b.json", b"2").expect("saved");
        assert!(
            first.ends_with("a_b.json") && second.ends_with("a_b-1.json"),
            "{first} {second}"
        );
        // A relative name is in the folder it names.
        let relative = desktop::resolve("x.json", Some(BaseDir::Download));
        assert!(
            relative.ends_with("x.json") && relative.is_absolute(),
            "{}",
            relative.display()
        );
    }
}
