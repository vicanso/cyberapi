//! Embeds the build timestamp (`CYBERAPI_BUILD_TIME`) and falls back to git for
//! `CYBERAPI_COMMIT` when the environment did not supply one.
//!
//! Narrow `rerun-if-env-changed` guards prevent incremental rebuild thrashing
//! while editing unrelated workspace files.

use std::process::Command;
use std::time::SystemTime;

fn main() {
    println!("cargo:rerun-if-env-changed=CYBERAPI_BUILD_TIME");
    println!("cargo:rerun-if-env-changed=CYBERAPI_COMMIT");
    println!("cargo:rerun-if-env-changed=CYBERAPI_VERSION");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    if std::env::var("CYBERAPI_COMMIT")
        .ok()
        .is_none_or(|v| v.trim().is_empty())
        && let Some(commit) = git_short_commit()
    {
        println!("cargo:rustc-env=CYBERAPI_COMMIT={commit}");
    }

    let build_time = std::env::var("CYBERAPI_BUILD_TIME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(current_utc_timestamp);
    println!("cargo:rustc-env=CYBERAPI_BUILD_TIME={build_time}");
}

fn git_short_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let hash = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!hash.is_empty()).then_some(hash)
}

fn current_utc_timestamp() -> String {
    if let Ok(epoch) = std::env::var("SOURCE_DATE_EPOCH")
        && let Ok(secs) = epoch.trim().parse::<i64>()
        && let Ok(ts) = jiff::Timestamp::from_second(secs)
    {
        return ts.strftime("%Y-%m-%d %H:%M:%S UTC").to_string();
    }
    let now = jiff::Timestamp::try_from(SystemTime::now()).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    now.strftime("%Y-%m-%d %H:%M:%S UTC").to_string()
}
