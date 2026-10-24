//! Installing an update that came down and matched its sum
//! (`states/update.rs`), after zedis's. On macOS it goes in place: the
//! image mounted, the bundle swapped once its identifier is ours, the
//! volume detached, and a restart opens the new app. Anywhere else — and on
//! macOS when the app does not run from a bundle (`cargo run`) or the swap
//! fails — the package is saved in the downloads folder and handed to the
//! system, which installs it as it would any other.

use crate::states::Delivery;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};

/// The packages of a release this machine takes, the first it has.
#[cfg(target_os = "macos")]
pub fn kinds() -> &'static [&'static str] {
    &["dmg"]
}
#[cfg(target_os = "windows")]
pub fn kinds() -> &'static [&'static str] {
    &["zip"]
}
/// Installed from the `.deb`, the app is under `/usr`, and the next one
/// comes the same way; unpacked from the tarball, it is anywhere else.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn kinds() -> &'static [&'static str] {
    if std::env::current_exe().is_ok_and(|exe| exe.starts_with("/usr")) {
        &["deb", "tarball"]
    } else {
        &["tarball"]
    }
}

/// Installs `bytes`, the package `name`. Blocking: `hdiutil` and `ditto`
/// take seconds.
pub fn install(name: &str, bytes: &[u8]) -> Result<Delivery, String> {
    match mac::in_place(name, bytes) {
        Some(Ok(())) => return Ok(Delivery::Replaced),
        Some(Err(error)) => warn!(%error, "update: not installed in place, handed to the system"),
        None => {}
    }
    let path = PathBuf::from(crate::this_build::save_to_downloads(name, bytes)?);
    hand_over(&path)?;
    info!(path = %path.display(), "update: handed to the system");
    Ok(Delivery::HandedToOs(path))
}

/// After [`Delivery::Replaced`]: opens the app again once this one has
/// quit, which the caller does next.
pub fn relaunch() {
    mac::relaunch();
}

/// The package opened as the system opens one: a disk image mounted with
/// its window, a `.deb` in the software installer.
#[cfg(target_os = "macos")]
fn hand_over(path: &Path) -> Result<(), String> {
    run(Command::new("open").arg(path))
}
/// The zip shown in Explorer, to unpack over the app.
#[cfg(target_os = "windows")]
fn hand_over(path: &Path) -> Result<(), String> {
    // Explorer's exit code says nothing either way.
    Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn()
        .map(drop)
        .map_err(|error| format!("explorer: {error}"))
}
/// A `.deb` given to the software installer; a tarball's folder shown. Not
/// waited for: `xdg-open` can wait on the program it starts.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn hand_over(path: &Path) -> Result<(), String> {
    let target = match path.extension().and_then(|ext| ext.to_str()) {
        Some("deb") => path,
        _ => path.parent().unwrap_or(path),
    };
    Command::new("xdg-open")
        .arg(target)
        .spawn()
        .map(drop)
        .map_err(|error| format!("xdg-open: {error}"))
}

/// Runs `command` to its end: an error says what it wrote.
#[cfg(target_os = "macos")]
fn run(command: &mut Command) -> Result<(), String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let out = command.output().map_err(|error| format!("{program}: {error}"))?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(format!(
        "{program}: {}",
        if said.is_empty() { out.status.to_string() } else { said }
    ))
}

#[cfg(target_os = "macos")]
mod mac {
    use super::run;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::Duration;
    use tracing::{info, warn};

    /// Must be `[package.metadata.bundle] identifier`: an image carrying
    /// anything else is not swapped in (a test holds the two together).
    pub(super) const BUNDLE_ID: &str = "com.bigtree.cyberapi";
    /// The bundle's name, on the image and on disk.
    pub(super) const BUNDLE_NAME: &str = "CyberAPI.app";

    /// `None` when the app does not run from a bundle.
    pub(super) fn in_place(name: &str, bytes: &[u8]) -> Option<Result<(), String>> {
        let target = running_bundle()?;
        Some(save_and_swap(&target, name, bytes))
    }

    fn save_and_swap(target: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
        let dir = std::env::temp_dir().join("cyberapi-update");
        std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        let dmg = dir.join(Path::new(name).file_name().unwrap_or(name.as_ref()));
        std::fs::write(&dmg, bytes).map_err(|error| format!("{}: {error}", dmg.display()))?;
        let swapped = install_over(target, &dmg);
        let _ = std::fs::remove_file(&dmg);
        swapped
    }

    /// The bundle this process runs from, or `None` under `cargo run`.
    /// `current_exe` is the path it was started by, so after a swap it
    /// names the new copy at the same place: what the restart opens.
    fn running_bundle() -> Option<PathBuf> {
        bundle_root_of(&std::env::current_exe().ok()?)
    }

    /// `…/Foo.app/Contents/MacOS/foo` → `…/Foo.app`.
    pub(super) fn bundle_root_of(exe: &Path) -> Option<PathBuf> {
        let root = exe.parent()?.parent()?.parent()?;
        root.extension()
            .is_some_and(|ext| ext == "app")
            .then(|| root.to_path_buf())
    }

    /// Mounts, swaps, detaches: the volume is detached however the swap
    /// went.
    fn install_over(target: &Path, dmg: &Path) -> Result<(), String> {
        let volume = attach(dmg)?;
        let swapped = replace_bundle(target, &volume);
        detach(&volume);
        swapped
    }

    /// The old bundle is moved aside into the temporary folder, not
    /// deleted: the running app may still read its files, and the system
    /// clears that folder itself. The move is also the permission check —
    /// an `/Applications` this user cannot write fails it before anything
    /// has moved. A failed copy moves the old bundle straight back.
    fn replace_bundle(target: &Path, volume: &Path) -> Result<(), String> {
        let fresh = volume.join(BUNDLE_NAME);
        if bundle_plist_value(&fresh, "CFBundleIdentifier").as_deref() != Some(BUNDLE_ID) {
            return Err(format!("{BUNDLE_NAME} on the image is not this app"));
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let aside = std::env::temp_dir().join(format!("cyberapi-previous-{}-{stamp}.app", std::process::id()));
        std::fs::rename(target, &aside).map_err(|error| format!("{}: {error}", target.display()))?;
        match run(Command::new("/usr/bin/ditto").arg(&fresh).arg(target)) {
            Ok(()) => {
                info!(target = %target.display(), "update: installed in place");
                Ok(())
            }
            Err(error) => {
                if let Err(back) = std::fs::rename(&aside, target) {
                    warn!(aside = %aside.display(), error = %back, "update: the old bundle could not be put back");
                }
                Err(error)
            }
        }
    }

    /// Mounted without a Finder window; answers where. `-noverify`: the
    /// bytes already matched the release's sum, and hdiutil's own check
    /// reads the whole image again.
    fn attach(dmg: &Path) -> Result<PathBuf, String> {
        let out = Command::new("hdiutil")
            .args(["attach", "-nobrowse", "-noverify", "-plist"])
            .arg(dmg)
            .output()
            .map_err(|error| format!("hdiutil: {error}"))?;
        if !out.status.success() {
            return Err(format!(
                "hdiutil attach: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        mount_point_from_plist(&String::from_utf8_lossy(&out.stdout))
            .ok_or_else(|| "hdiutil attach: no mount point".to_string())
    }

    /// The `mount-point` of `hdiutil attach -plist`, read without a plist
    /// parser: the volume's name is ours and plain ASCII.
    pub(super) fn mount_point_from_plist(xml: &str) -> Option<PathBuf> {
        let after = xml.split("<key>mount-point</key>").nth(1)?;
        let start = after.find("<string>")? + "<string>".len();
        let end = start + after[start..].find("</string>")?;
        Some(PathBuf::from(&after[start..end]))
    }

    /// Tried twice, a second apart: hdiutil says "resource busy" while
    /// Spotlight still reads a fresh volume. One that stays is left, to be
    /// ejected in Finder.
    fn detach(volume: &Path) {
        for attempt in 0..2 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_secs(1));
            }
            if run(Command::new("hdiutil").arg("detach").arg(volume)).is_ok() {
                return;
            }
        }
        warn!(volume = %volume.display(), "update: the image is left mounted");
    }

    /// A string of a bundle's `Info.plist` (XML or binary), by `defaults
    /// read`; `None` for no bundle, no key, or no `defaults`.
    fn bundle_plist_value(app: &Path, key: &str) -> Option<String> {
        // Without the extension: `defaults` adds `.plist`.
        let out = Command::new("defaults")
            .arg("read")
            .arg(app.join("Contents/Info"))
            .arg(key)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let value = String::from_utf8(out.stdout).ok()?;
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    }

    /// A shell, left behind, waits for this process to end and opens the
    /// bundle; its path rides in `$0`, so nothing is quoted in the script.
    /// The shell outlives the app as launchd's, never a zombie of ours.
    pub(super) fn relaunch() {
        let Some(bundle) = running_bundle() else {
            return;
        };
        let script = format!(
            "while /bin/kill -0 {pid} 2>/dev/null; do /bin/sleep 0.1; done; /usr/bin/open \"$0\"",
            pid = std::process::id()
        );
        match Command::new("/bin/sh").arg("-c").arg(script).arg(&bundle).spawn() {
            Ok(_) => info!(bundle = %bundle.display(), "update: opens again once this quits"),
            Err(error) => warn!(%error, "update: the restart could not be arranged"),
        }
    }
}

/// Nowhere but macOS is a package installed in place.
#[cfg(not(target_os = "macos"))]
mod mac {
    pub(super) fn in_place(_name: &str, _bytes: &[u8]) -> Option<Result<(), String>> {
        None
    }

    pub(super) fn relaunch() {}
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::mac::*;
    use std::path::{Path, PathBuf};

    /// The bundle identifier is checked before a swap: it must move with
    /// Cargo.toml's, or every update falls back to the image's window.
    #[test]
    fn the_swap_looks_for_this_apps_identifier() {
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains(&format!("identifier = \"{BUNDLE_ID}\"")));
        assert!(manifest.contains(&format!("name = \"{}\"", BUNDLE_NAME.trim_end_matches(".app"))));
    }

    #[test]
    fn the_mount_point_comes_out_of_hdiutils_answer() {
        // The disk has no mount point; its file system has.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>system-entities</key><array>
  <dict>
    <key>content-hint</key><string>GUID_partition_scheme</string>
    <key>dev-entry</key><string>/dev/disk5</string>
  </dict>
  <dict>
    <key>content-hint</key><string>Apple_HFS</string>
    <key>dev-entry</key><string>/dev/disk5s1</string>
    <key>mount-point</key>
    <string>/Volumes/CyberAPI</string>
  </dict>
</array></dict></plist>"#;
        assert_eq!(mount_point_from_plist(xml), Some(PathBuf::from("/Volumes/CyberAPI")));
        assert_eq!(mount_point_from_plist("<plist></plist>"), None);
    }

    #[test]
    fn the_bundle_is_the_app_folder_or_nothing() {
        assert_eq!(
            bundle_root_of(Path::new("/Applications/CyberAPI.app/Contents/MacOS/cyberapi")),
            Some(PathBuf::from("/Applications/CyberAPI.app"))
        );
        assert_eq!(
            bundle_root_of(Path::new("/Users/x/cyberapi/target/debug/cyberapi")),
            None
        );
    }
}
