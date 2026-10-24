//! The desktop's update check, the rules of it: what a release's
//! `latest.json` says (`scripts/checksums.sh` writes it beside the
//! packages), whether its version is newer than the running build, which of
//! its files this machine installs, and the checks on what comes down. The
//! requests, the download and the install are the desktop's own.

use serde::Deserialize;
use sha2::{Digest as _, Sha256};

/// `latest.json`, as the release workflow publishes it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub tag: String,
    /// The release's page.
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// One package of a release: for which system and processor, of which
/// kind, where, and its SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub os: String,
    pub arch: String,
    /// `dmg`, `deb`, `tarball`, `zip`, `web`.
    pub kind: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
}

/// What GitHub's API says of a release, for a release without a manifest
/// (one made before there was one) and for the notes.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub html_url: String,
    /// The notes, Markdown.
    #[serde(default)]
    pub body: String,
}

/// `v1.2.3` or `1.2.3` as numbers, and whether it is a pre-release
/// (`1.2.3-beta.1`). `None` for anything else.
fn parse(version: &str) -> Option<([u64; 3], bool)> {
    let version = version.trim().trim_start_matches('v');
    let (core, pre) = match version.split_once(['-', '+']) {
        Some((core, rest)) => (core, version.as_bytes()[core.len()] == b'-' && !rest.is_empty()),
        None => (version, false),
    };
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let numbers = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some((numbers, pre))
}

/// Whether `latest` is a newer version than `current`. A pre-release comes
/// before its release; two pre-releases of one version are not told apart,
/// and nothing that does not read as a version is newer.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse(latest), parse(current)) {
        (Some((latest, latest_pre)), Some((current, current_pre))) => {
            latest > current || (latest == current && current_pre && !latest_pre)
        }
        _ => false,
    }
}

/// The package this machine installs: the first of `kinds`, in the order
/// given, that the release has for `os` and `arch` (the names Rust's
/// `std::env::consts` uses).
pub fn pick_asset<'a>(assets: &'a [Asset], os: &str, arch: &str, kinds: &[&str]) -> Option<&'a Asset> {
    kinds
        .iter()
        .find_map(|kind| assets.iter().find(|a| a.os == os && a.arch == arch && a.kind == *kind))
}

/// `bytes`' SHA-256, in lower-case hex, as `SHA256SUMS` writes it.
pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `bytes` are what the release published: their SHA-256 is the
/// one the manifest gives. An asset listed without one is not taken.
pub fn verified(bytes: &[u8], sha256: &str) -> bool {
    !sha256.is_empty() && self::sha256(bytes).eq_ignore_ascii_case(sha256.trim())
}

/// Where a redirect's `Location` leads from `url`: absolute, or resolved
/// against it. `None` for one that does not resolve.
pub fn redirected(url: &str, location: &str) -> Option<String> {
    let base = url::Url::parse(url).ok()?;
    base.join(location.trim()).ok().map(String::from)
}

/// A release's notes as the update dialog shows them: without their own
/// heading (`## [0.3.0](…) - date`, as a section of CHANGELOG.md starts),
/// which the dialog's title says already.
pub fn notes_body(notes: &str) -> &str {
    let notes = notes.trim_start();
    match notes.split_once('\n') {
        Some((first, rest)) if first.starts_with("## ") => rest.trim_start(),
        None if notes.starts_with("## ") => "",
        _ => notes,
    }
}

/// Whether the automatic check is due: never made, or made `interval`
/// seconds ago or more (a clock set back counts as due too).
pub fn check_due(last: i64, now: i64, interval: i64) -> bool {
    last <= 0 || now < last || now - last >= interval
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(os: &str, arch: &str, kind: &str) -> Asset {
        Asset {
            os: os.into(),
            arch: arch.into(),
            kind: kind.into(),
            name: format!("{os}-{arch}.{kind}"),
            url: format!("https://example.test/{os}-{arch}.{kind}"),
            sha256: String::new(),
            size: 0,
        }
    }

    #[test]
    fn a_version_is_newer_by_its_numbers() {
        assert!(is_newer("0.3.0", "0.2.0"));
        assert!(is_newer("v0.2.1", "0.2.0"), "a tag reads as its version");
        assert!(is_newer("1.0.0", "0.10.9"));
        assert!(is_newer("0.10.0", "0.9.9"), "numbers, not text");
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.21", "0.2.0"));
        assert!(is_newer("0.3.0", "0.3.0-beta.1"), "the release after its pre-release");
        assert!(!is_newer("0.3.0-beta.1", "0.3.0"));
        assert!(is_newer("0.3.0-beta.1", "0.2.0"));
        assert!(!is_newer("nightly", "0.2.0"));
        assert!(!is_newer("0.3", "0.2.0"));
        assert!(!is_newer("0.3.0.1", "0.2.0"));
    }

    #[test]
    fn this_machine_takes_its_own_package() {
        let assets = vec![
            asset("macos", "aarch64", "dmg"),
            asset("macos", "x86_64", "dmg"),
            asset("linux", "x86_64", "tarball"),
            asset("linux", "x86_64", "deb"),
            asset("linux", "x86_64", "web"),
            asset("windows", "x86_64", "zip"),
        ];
        let picked = |os, arch, kinds: &[&str]| pick_asset(&assets, os, arch, kinds).map(|a| a.name.clone());
        assert_eq!(picked("macos", "aarch64", &["dmg"]), Some("macos-aarch64.dmg".into()));
        assert_eq!(
            picked("linux", "x86_64", &["deb", "tarball"]),
            Some("linux-x86_64.deb".into()),
            "the kinds in the order asked"
        );
        assert_eq!(
            picked("linux", "x86_64", &["tarball"]),
            Some("linux-x86_64.tarball".into())
        );
        assert_eq!(
            picked("linux", "aarch64", &["deb", "tarball"]),
            None,
            "none built for it"
        );
        assert_eq!(picked("windows", "x86_64", &["zip"]), Some("windows-x86_64.zip".into()));
    }

    #[test]
    fn the_manifest_reads_as_the_release_writes_it() {
        let json = r#"{"version":"0.3.0","tag":"v0.3.0","notes":"https://github.com/vicanso/cyberapi/releases/tag/v0.3.0","assets":[{"os":"macos","arch":"aarch64","kind":"dmg","name":"cyberapi-0.3.0-aarch64-apple-darwin.dmg","url":"https://github.com/vicanso/cyberapi/releases/download/v0.3.0/cyberapi-0.3.0-aarch64-apple-darwin.dmg","sha256":"ab","size":17}]}"#;
        let manifest: Manifest = serde_json::from_str(json).expect("parses");
        assert_eq!(manifest.version, "0.3.0");
        assert_eq!(manifest.assets[0].kind, "dmg");
        assert_eq!(manifest.assets[0].size, 17);
    }

    #[test]
    fn what_came_down_is_checked_against_its_sum() {
        let sum = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verified(b"abc", sum));
        assert!(verified(b"abc", &sum.to_uppercase()));
        assert!(!verified(b"abd", sum));
        assert!(!verified(b"abc", ""), "no sum, no install");
    }

    #[test]
    fn a_redirect_leads_on_from_where_it_was() {
        assert_eq!(
            redirected("https://github.com/a/b", "https://objects.example/x?y=1").as_deref(),
            Some("https://objects.example/x?y=1")
        );
        assert_eq!(
            redirected("https://github.com/a/b/c", "/d").as_deref(),
            Some("https://github.com/d")
        );
        assert_eq!(redirected("not a url", "/d"), None);
    }

    #[test]
    fn the_notes_lose_the_heading_the_title_repeats() {
        let section = "## [0.3.0](https://example.test/compare) - 2026-10-24\r\n\r\n### Features\r\n\n- one";
        assert_eq!(notes_body(section), "### Features\r\n\n- one");
        assert_eq!(notes_body("- one\n- two"), "- one\n- two");
        assert_eq!(notes_body("## [0.3.0]"), "");
        assert_eq!(
            notes_body("### Features\n- one"),
            "### Features\n- one",
            "only the version's own"
        );
    }

    #[test]
    fn the_check_is_due_once_the_interval_has_passed() {
        let day = 24 * 60 * 60;
        assert!(check_due(0, 1_000_000, day), "never checked");
        assert!(!check_due(1_000_000, 1_000_000 + day - 1, day));
        assert!(check_due(1_000_000, 1_000_000 + day, day));
        assert!(check_due(1_000_000, 999_000, day), "a clock set back");
    }
}
