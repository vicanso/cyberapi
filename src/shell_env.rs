//! D2: this machine's environment variables, for `env(NAME)`. The app's
//! own first; an app opened from the Dock or Finder has few of the reader's
//! — what `~/.zshrc` or `~/.profile` exports reaches only what a shell
//! starts — so the rest come from the reader's login shell, run once (as
//! VS Code does) and kept. A name it does not have runs it again: a
//! variable just added there is found without restarting the app.
//!
//! Windows has no login shell to ask: the app's own are all.

use cyberapi_core::template::{Env, EnvReader};
use std::collections::HashMap;
use std::sync::Mutex;

/// What the login shell exported, once read: for every send that reads one.
static SHELL: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

pub struct ShellEnv;

impl EnvReader for ShellEnv {
    fn read(&self, names: &[String]) -> Env {
        let mut read: HashMap<String, Option<String>> = names
            .iter()
            .map(|name| (name.clone(), std::env::var(name).ok()))
            .collect();
        if read.values().all(Option::is_some) {
            return Env::Read(read);
        }
        let mut shell = SHELL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let lacks = |shell: &HashMap<String, String>| {
            read.iter()
                .any(|(name, value)| value.is_none() && !shell.contains_key(name))
        };
        if shell.as_ref().is_none_or(lacks) {
            match login_shell() {
                Ok(exported) => *shell = Some(exported),
                Err(why) => tracing::warn!("the login shell's environment could not be read: {why}"),
            }
        }
        if let Some(exported) = shell.as_ref() {
            for (name, value) in &mut read {
                if value.is_none() {
                    *value = exported.get(name).cloned();
                }
            }
        }
        Env::Read(read)
    }
}

/// What the login shell exports, as an interactive one sets it up.
#[cfg(unix)]
fn login_shell() -> Result<HashMap<String, String>, String> {
    use std::io::Read as _;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    /// How long a shell's startup files may take.
    const WAIT: Duration = Duration::from_secs(5);
    /// Where the variables start: a startup file may print first.
    const MARK: &str = "__CYBERAPI_ENV__";

    let shell = std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    let mut child = Command::new(&shell)
        .args(["-l", "-i", "-c", &format!("printf '%s' {MARK}; env -0")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("{shell}: {error}"))?;
    let mut stdout = child.stdout.take().expect("piped");
    let (done, out) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = done.send(bytes);
    });
    let bytes = out.recv_timeout(WAIT);
    // Ended already, or stopped: what it started is not waited for.
    let _ = child.kill();
    let _ = child.wait();
    let bytes = bytes.map_err(|_| format!("{shell} did not finish in {}s", WAIT.as_secs()))?;
    let text = String::from_utf8_lossy(&bytes);
    let (_, exported) = text
        .split_once(MARK)
        .ok_or_else(|| format!("{shell} printed no variables"))?;
    Ok(parse(exported))
}

#[cfg(not(unix))]
fn login_shell() -> Result<HashMap<String, String>, String> {
    Ok(HashMap::new())
}

/// `env -0`'s output: `NAME=value`, each ended by a NUL.
#[cfg_attr(not(unix), allow(dead_code))]
fn parse(exported: &str) -> HashMap<String, String> {
    exported
        .split('\0')
        .filter_map(|entry| entry.split_once('='))
        .filter(|(name, _)| !name.is_empty())
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_0_is_split_on_nuls_and_the_first_equals() {
        let exported = parse("A=1\0B=x=y\0MULTI=line one\nline two\0\0=nameless\0");
        assert_eq!(exported.get("A").map(String::as_str), Some("1"));
        assert_eq!(exported.get("B").map(String::as_str), Some("x=y"));
        assert_eq!(exported.get("MULTI").map(String::as_str), Some("line one\nline two"));
        assert_eq!(exported.len(), 3);
    }

    #[test]
    fn the_apps_own_come_first_and_an_unset_one_is_none() {
        let Env::Read(read) = ShellEnv.read(&["PATH".into(), "CYBERAPI_SURELY_UNSET_9F2C".into()]) else {
            panic!("the desktop reads");
        };
        assert_eq!(read["PATH"], std::env::var("PATH").ok());
        assert_eq!(read["CYBERAPI_SURELY_UNSET_9F2C"], None);
    }
}
