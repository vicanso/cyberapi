//! Which build this is (#54): the release's version, or a nightly's own,
//! and the commit it was built from, as the build was told them. CI sets
//! `CYBERAPI_VERSION` for a nightly and `CYBERAPI_COMMIT` for every build
//! (`.github/workflows/package.yml`, the Dockerfile); a local build has
//! neither, and is the version in Cargo.toml.

/// `0.2.1`, or a nightly's `0.2.2-nightly.20261024.137a858`: a
/// pre-release of the version after the last, so that the release it leads
/// to is newer and the one before it is not (`update::is_newer`).
pub const VERSION: &str = match option_env!("CYBERAPI_VERSION") {
    Some(version) if !version.is_empty() => version,
    _ => env!("CARGO_PKG_VERSION"),
};

/// The version of the shape a project's content is kept in (requests,
/// folders, variables, what a project syncs). It goes up by one with each
/// change to that shape (`docs/batch4-design.md` §2.1), whatever the app's
/// version, so a nightly is told apart from the release before it. A
/// client that has an older one reads a server's projects but may not
/// change them: it would drop what it cannot read. Clients that do not say
/// which they have (0.2.1 and before) have the first.
///
/// 2: an API's own sending settings (#71, `HttpRequest.settings`).
/// 3: an API signs in as one of several kinds (#66, `HttpRequest.auth`,
///    kept as `authentication`; the old Basic list is read, not written).
/// 4: what folders and projects set for their APIs (#66, `Folder.settings`,
///    `ProjectConfig.settings`).
/// 5: named environments and the variables in them (#61,
///    `ProjectConfig.environments`, `Variable.environment`).
pub const MODEL: u32 = 5;

/// The commit's short hash, when the build was told it.
pub const COMMIT: Option<&str> = match option_env!("CYBERAPI_COMMIT") {
    Some(commit) if !commit.is_empty() => Some(commit),
    _ => None,
};

/// The version with its commit, `0.2.1 (abc1234)`, for `--version`, the
/// logs and the about page; a nightly's version names it already.
pub fn describe() -> String {
    described(VERSION, COMMIT)
}

fn described(version: &str, commit: Option<&str>) -> String {
    match commit {
        Some(commit) if !version.contains(commit) => format!("{version} ({commit})"),
        _ => version.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commit_is_said_once() {
        assert_eq!(described("0.2.1", Some("abc1234")), "0.2.1 (abc1234)");
        assert_eq!(described("0.2.1", None), "0.2.1");
        assert_eq!(
            described("0.2.2-nightly.20261024.137a858", Some("137a858")),
            "0.2.2-nightly.20261024.137a858"
        );
    }

    #[test]
    fn a_nightly_is_newer_than_the_release_before_it_and_older_than_the_next() {
        let nightly = "0.2.2-nightly.20261024.137a858";
        assert!(crate::update::is_newer(nightly, "0.2.1"));
        assert!(!crate::update::is_newer("0.2.1", nightly));
        assert!(crate::update::is_newer("0.2.2", nightly));
    }
}
