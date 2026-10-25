//! A request through a proxy (#60): which one — the settings', or the
//! environment's `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` — and whether a
//! host goes around it (`NO_PROXY`, or the settings' list). An HTTP proxy
//! takes a plain request whole and tunnels HTTPS with `CONNECT`; a SOCKS5
//! proxy (`socks5://`, `socks5h://` to have the proxy find the host) is
//! asked to connect. A loopback address never goes through one: the
//! proxy's own machine is not this one.
//!
//! The rules are here; reading the environment and talking to the proxy are
//! the engine's (`cyberapi-http`), on the desktop only.

use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Where a request's proxy comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProxyMode {
    /// `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY`, as curl
    /// reads them; none set is none.
    #[default]
    Environment,
    /// The address and the list kept in the settings.
    Manual,
    /// None: every request goes to its server.
    Off,
}

impl ProxyMode {
    pub const ALL: [Self; 3] = [Self::Environment, Self::Manual, Self::Off];
}

/// The settings' proxy (F14).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProxyPrefs {
    pub mode: ProxyMode,
    /// `http://host:port`, `socks5://user:password@host:port`, or
    /// `host:port` (HTTP).
    pub url: String,
    /// Hosts that go to their server: `example.com` (and below it),
    /// `.example.com`, `*.example.com`, an address, `*` for every one;
    /// separated by commas or spaces.
    pub bypass: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyKind {
    Http,
    /// `remote_dns`: the proxy finds the host (`socks5h://`); otherwise it
    /// is found here and its address sent.
    Socks5 {
        remote_dns: bool,
    },
}

/// A proxy to go through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub auth: Option<(String, String)>,
}

impl Proxy {
    /// `http://host:port`, `socks5h://host:port`, or `host:port` for HTTP.
    /// A port left out is 1080, curl's.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let written = if text.contains("://") {
            text.to_string()
        } else {
            format!("http://{text}")
        };
        let url = url::Url::parse(&written).map_err(|error| format!("{text}: {error}"))?;
        let kind = match url.scheme() {
            "http" => ProxyKind::Http,
            "socks5" => ProxyKind::Socks5 { remote_dns: false },
            "socks5h" => ProxyKind::Socks5 { remote_dns: true },
            other => {
                return Err(format!(
                    "{text}: {other} proxies are not supported (http, socks5, socks5h are)"
                ));
            }
        };
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or(format!("{text}: no host"))?;
        let decoded = |text: &str| percent_decode_str(text).decode_utf8_lossy().into_owned();
        let auth = (!url.username().is_empty())
            .then(|| (decoded(url.username()), decoded(url.password().unwrap_or_default())));
        Ok(Self {
            kind,
            host: host.trim_start_matches('[').trim_end_matches(']').to_string(),
            port: url.port().unwrap_or(1080),
            auth,
        })
    }

    /// Where it is, without its password: what the response says it went
    /// through.
    pub fn display(&self) -> String {
        let scheme = match self.kind {
            ProxyKind::Http => "http",
            ProxyKind::Socks5 { remote_dns: false } => "socks5",
            ProxyKind::Socks5 { remote_dns: true } => "socks5h",
        };
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("{scheme}://{host}:{}", self.port)
    }
}

/// Whether `host` goes around the proxy: a loopback address always; one
/// `list` names otherwise.
pub fn bypassed(host: &str, list: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    let loopback = host == "localhost"
        || host.ends_with(".localhost")
        || host.parse::<IpAddr>().is_ok_and(|address| address.is_loopback());
    loopback
        || list
            .split([',', ' ', '\t', '\n'])
            .map(|entry| entry.trim().to_ascii_lowercase())
            .filter(|entry| !entry.is_empty())
            .any(|entry| {
                if entry == "*" {
                    return true;
                }
                // A port named in the entry is not told apart.
                let entry = match entry.rsplit_once(':') {
                    Some((name, port)) if !name.contains(':') && port.parse::<u16>().is_ok() => name.to_string(),
                    _ => entry,
                };
                let name = entry.trim_start_matches("*.").trim_start_matches('.');
                host == name || host.ends_with(&format!(".{name}"))
            })
}

/// The proxy a request to `url` goes through, if any. `env` reads an
/// environment variable (the engine's; a test's map).
pub fn choose(
    prefs: &ProxyPrefs,
    url: &url::Url,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Option<Proxy>, String> {
    let host = url.host_str().unwrap_or_default();
    match prefs.mode {
        ProxyMode::Off => Ok(None),
        ProxyMode::Manual if prefs.url.trim().is_empty() || bypassed(host, &prefs.bypass) => Ok(None),
        ProxyMode::Manual => Proxy::parse(&prefs.url).map(Some),
        ProxyMode::Environment => {
            let read = |names: &[&str]| {
                names
                    .iter()
                    .filter_map(|name| env(name))
                    .find(|value| !value.trim().is_empty())
            };
            let names: &[&str] = if url.scheme() == "https" {
                &["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"]
            } else {
                &["http_proxy", "HTTP_PROXY", "all_proxy", "ALL_PROXY"]
            };
            let Some(proxy) = read(names) else {
                return Ok(None);
            };
            let bypass = read(&["no_proxy", "NO_PROXY"]).unwrap_or_default();
            if bypassed(host, &bypass) {
                return Ok(None);
            }
            Proxy::parse(&proxy).map(Some)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;

    fn url(text: &str) -> url::Url {
        url::Url::parse(text).expect("a URL")
    }

    #[test]
    fn a_proxy_address_reads_as_curl_reads_it() {
        assert_eq!(
            Proxy::parse("http://u%40x:p%3Aw@proxy.test:3128"),
            Ok(Proxy {
                kind: ProxyKind::Http,
                host: "proxy.test".into(),
                port: 3128,
                auth: Some(("u@x".into(), "p:w".into())),
            })
        );
        let bare = Proxy::parse(" 10.0.0.1:8080 ").expect("host:port");
        assert_eq!((bare.kind, bare.port, bare.auth), (ProxyKind::Http, 8080, None));
        assert_eq!(
            Proxy::parse("socks5h://[::1]").expect("ipv6").display(),
            "socks5h://[::1]:1080"
        );
        assert_eq!(
            Proxy::parse("socks5://p.test:1080").expect("socks").kind,
            ProxyKind::Socks5 { remote_dns: false }
        );
        assert!(
            Proxy::parse("https://p.test:443").is_err(),
            "a TLS proxy is not supported"
        );
        assert!(Proxy::parse("http://").is_err());
    }

    #[test]
    fn a_host_named_or_below_one_named_goes_around() {
        let list = "internal.test, .corp.test *.dev.test 10.1.2.3 api.test:8443";
        for host in [
            "internal.test",
            "a.internal.test",
            "x.corp.test",
            "b.dev.test",
            "10.1.2.3",
            "api.test",
        ] {
            assert!(bypassed(host, list), "{host}");
        }
        for host in ["notinternal.test", "corp.test.example", "10.1.2.4"] {
            assert!(!bypassed(host, list), "{host}");
        }
        assert!(bypassed("anything.test", "*"));
        for loopback in ["localhost", "app.localhost", "127.0.0.1", "127.9.9.9", "[::1]"] {
            assert!(bypassed(loopback, ""), "{loopback} is this machine");
        }
    }

    #[test]
    fn the_environment_names_a_proxy_for_each_scheme() {
        let env: HashMap<&str, &str> = HashMap::from([
            ("HTTPS_PROXY", "http://secure.proxy:8443"),
            ("http_proxy", "http://plain.proxy:3128"),
            ("ALL_PROXY", "socks5h://all.proxy:1080"),
            ("NO_PROXY", "skip.test"),
        ]);
        let read = |name: &str| env.get(name).map(|value| value.to_string());
        let prefs = ProxyPrefs::default();
        let host = |found: Result<Option<Proxy>, String>| found.expect("read").map(|proxy| proxy.host);
        assert_eq!(
            host(choose(&prefs, &url("https://a.test/"), read)),
            Some("secure.proxy".into())
        );
        assert_eq!(
            host(choose(&prefs, &url("http://a.test/"), read)),
            Some("plain.proxy".into())
        );
        assert_eq!(host(choose(&prefs, &url("https://skip.test/"), read)), None);
        assert_eq!(host(choose(&prefs, &url("http://127.0.0.1:8080/"), read)), None);
        // ALL_PROXY when the scheme has none.
        let all_only = |name: &str| (name == "ALL_PROXY").then(|| "socks5h://all.proxy:1080".to_string());
        assert_eq!(
            host(choose(&prefs, &url("https://a.test/"), all_only)),
            Some("all.proxy".into())
        );
        assert_eq!(host(choose(&prefs, &url("https://a.test/"), |_| None)), None);
    }

    #[test]
    fn the_settings_name_one_or_none() {
        let manual = ProxyPrefs {
            mode: ProxyMode::Manual,
            url: "http://p.test:8080".into(),
            bypass: "intra.test".into(),
        };
        let env = |_: &str| Some("http://env.proxy:1".to_string());
        let host = |prefs: &ProxyPrefs, to: &str| choose(prefs, &url(to), env).expect("read").map(|proxy| proxy.host);
        assert_eq!(host(&manual, "https://a.test/"), Some("p.test".into()));
        assert_eq!(host(&manual, "https://x.intra.test/"), None);
        let blank = ProxyPrefs {
            url: " ".into(),
            ..manual.clone()
        };
        assert_eq!(host(&blank, "https://a.test/"), None);
        let off = ProxyPrefs {
            mode: ProxyMode::Off,
            ..manual
        };
        assert_eq!(host(&off, "https://a.test/"), None, "not the environment's either");
        let broken = ProxyPrefs {
            mode: ProxyMode::Manual,
            url: "ftp://p.test".into(),
            bypass: String::new(),
        };
        assert!(choose(&broken, &url("https://a.test/"), env).is_err());
    }
}
