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
        // A relative name is in the folder it names.
        let relative = desktop::resolve("x.json", Some(BaseDir::Download));
        assert!(
            relative.ends_with("x.json") && relative.is_absolute(),
            "{}",
            relative.display()
        );
    }
}
