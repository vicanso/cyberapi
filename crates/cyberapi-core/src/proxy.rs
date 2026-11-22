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
    /// reads them; when unset, falls back to the OS proxy / PAC on macOS
    /// and Windows if enabled.
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

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/// The settings' proxy (F14 / R2).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// R2: when `mode == ProxyMode::Environment` and no proxy environment
    /// variable matches, consult the OS system proxy / PAC (macOS / Windows).
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub system: bool,
}

impl Default for ProxyPrefs {
    fn default() -> Self {
        Self {
            mode: ProxyMode::Environment,
            url: String::new(),
            bypass: String::new(),
            system: true,
        }
    }
}

/// R2: how a request should route based on settings and environment variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProxyDecision {
    Direct,
    Use(Proxy),
    System,
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

/// R2: decides the proxy policy for `url` using manual settings first, then
/// environment variables (`NO_PROXY`, `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`),
/// and finally `ProxyDecision::System` when `prefs.system` is enabled.
pub fn decide(
    prefs: &ProxyPrefs,
    url: &url::Url,
    env: impl Fn(&str) -> Option<String>,
) -> Result<ProxyDecision, String> {
    let host = url.host_str().unwrap_or_default();
    match prefs.mode {
        ProxyMode::Off => Ok(ProxyDecision::Direct),
        ProxyMode::Manual if prefs.url.trim().is_empty() || bypassed(host, &prefs.bypass) => Ok(ProxyDecision::Direct),
        ProxyMode::Manual => Proxy::parse(&prefs.url).map(ProxyDecision::Use),
        ProxyMode::Environment => {
            if bypassed(host, "") {
                return Ok(ProxyDecision::Direct);
            }
            let read = |names: &[&str]| {
                names
                    .iter()
                    .filter_map(|name| env(name))
                    .find(|value| !value.trim().is_empty())
            };
            let bypass = read(&["no_proxy", "NO_PROXY"]).unwrap_or_default();
            if !bypass.is_empty() && bypassed(host, &bypass) {
                return Ok(ProxyDecision::Direct);
            }
            let names: &[&str] = if matches!(url.scheme(), "https" | "wss") {
                &["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"]
            } else {
                &["http_proxy", "HTTP_PROXY", "all_proxy", "ALL_PROXY"]
            };
            if let Some(proxy) = read(names) {
                return Proxy::parse(&proxy).map(ProxyDecision::Use);
            }
            if prefs.system {
                Ok(ProxyDecision::System)
            } else {
                Ok(ProxyDecision::Direct)
            }
        }
    }
}

/// The proxy a request to `url` goes through, if any. `env` reads an
/// environment variable (the engine's; a test's map).
pub fn choose(
    prefs: &ProxyPrefs,
    url: &url::Url,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Option<Proxy>, String> {
    match decide(prefs, url, env)? {
        ProxyDecision::Use(proxy) => Ok(Some(proxy)),
        ProxyDecision::Direct | ProxyDecision::System => Ok(None),
    }
}

/// R2: parses a semicolon-separated PAC `FindProxyForURL` result string
/// (`DIRECT`, `PROXY host:port`, `HTTP host:port`, `SOCKS host:port`,
/// `SOCKS5 host:port`), returning `None` on the first `DIRECT` directive or
/// `Some(Proxy)` on the first valid proxy entry.
pub fn parse_pac_result(list: &str) -> Option<Proxy> {
    for raw_entry in list.split(';') {
        let entry = raw_entry.trim();
        if entry.is_empty() {
            continue;
        }
        let mut parts = entry.split_whitespace();
        let Some(kind) = parts.next() else {
            continue;
        };
        let rest = parts.collect::<Vec<_>>().join(" ");
        let upper = kind.to_ascii_uppercase();
        if upper == "DIRECT" {
            return None;
        }
        let target = rest.trim();
        if target.is_empty() {
            continue;
        }
        let candidate = match upper.as_str() {
            "PROXY" | "HTTP" => target.to_string(),
            "SOCKS" | "SOCKS5" => {
                if target.contains("://") {
                    target.to_string()
                } else {
                    format!("socks5h://{target}")
                }
            }
            _ => continue,
        };
        if let Ok(proxy) = Proxy::parse(&candidate) {
            return Some(proxy);
        }
    }
    None
}

/// R2: parses a WinHTTP proxy list (`http=h:p;https=h:p`, `socks=h:p`, or bare
/// `h:p` / `http://h:p`), selecting the best match for `scheme`.
pub fn parse_winhttp_proxy_list(proxy_list: &str, scheme: &str) -> Option<Proxy> {
    let mut http_proxy: Option<Proxy> = None;
    let mut https_proxy: Option<Proxy> = None;
    let mut socks_proxy: Option<Proxy> = None;
    let mut fallback: Option<Proxy> = None;

    for raw_token in proxy_list.split([';', ' ', '\t', '\r', '\n']) {
        let token = raw_token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some((key, val)) = token.split_once('=') {
            let key = key.trim().to_ascii_lowercase();
            let val = val.trim();
            if val.is_empty() {
                continue;
            }
            match key.as_str() {
                "http" if http_proxy.is_none() => {
                    http_proxy = Proxy::parse(val).ok();
                }
                "https" if https_proxy.is_none() => {
                    https_proxy = Proxy::parse(val).ok();
                }
                "socks" | "socks5" if socks_proxy.is_none() => {
                    let wrapped = if val.contains("://") {
                        val.to_string()
                    } else {
                        format!("socks5h://{val}")
                    };
                    socks_proxy = Proxy::parse(&wrapped).ok();
                }
                _ => {}
            }
        } else if fallback.is_none() {
            fallback = Proxy::parse(token).ok();
        }
    }

    let is_https = matches!(scheme.to_ascii_lowercase().as_str(), "https" | "wss");
    if is_https {
        https_proxy.or(http_proxy).or(socks_proxy).or(fallback)
    } else {
        http_proxy.or(https_proxy).or(socks_proxy).or(fallback)
    }
}

/// R2: checks Windows WinHTTP semicolon-separated bypass lists including `<local>`.
pub fn bypassed_windows(host: &str, bypass: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    let has_local = bypass
        .split([';', ',', ' ', '\t', '\r', '\n'])
        .any(|entry| entry.trim().eq_ignore_ascii_case("<local>"));
    if has_local && !bare.contains('.') {
        return true;
    }
    let normalized = bypass.replace(';', ",");
    bypassed(host, &normalized)
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
            ..Default::default()
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
            ..Default::default()
        };
        assert!(choose(&broken, &url("https://a.test/"), env).is_err());
    }

    #[test]
    fn decide_precedence_and_defaults_work() {
        // Omitting `system` in serialized JSON defaults it to `true`.
        let legacy: ProxyPrefs = serde_json::from_str(r#"{"mode":"environment"}"#).expect("legacy prefs");
        assert!(legacy.system);

        let no_env = |_: &str| None;
        assert_eq!(
            decide(&ProxyPrefs::default(), &url("https://api.test/"), no_env),
            Ok(ProxyDecision::System)
        );
        assert_eq!(
            decide(&ProxyPrefs::default(), &url("http://127.0.0.1:8080/"), no_env),
            Ok(ProxyDecision::Direct)
        );
        let no_proxy_env = |name: &str| (name == "NO_PROXY").then(|| "internal.test".to_string());
        assert_eq!(
            decide(&ProxyPrefs::default(), &url("https://internal.test/"), no_proxy_env),
            Ok(ProxyDecision::Direct)
        );

        let sys_off = ProxyPrefs {
            system: false,
            ..Default::default()
        };
        assert_eq!(
            decide(&sys_off, &url("https://api.test/"), no_env),
            Ok(ProxyDecision::Direct)
        );
        let manual = ProxyPrefs {
            mode: ProxyMode::Manual,
            url: "http://manual.proxy:8888".into(),
            ..Default::default()
        };
        assert_eq!(
            decide(&manual, &url("https://api.test/"), |_| Some(
                "http://env.proxy:1".into()
            )),
            Ok(ProxyDecision::Use(Proxy {
                kind: ProxyKind::Http,
                host: "manual.proxy".into(),
                port: 8888,
                auth: None,
            }))
        );
    }

    #[test]
    fn pac_and_winhttp_parsers_handle_rules_and_bypass() {
        assert_eq!(parse_pac_result("DIRECT"), None);
        assert_eq!(parse_pac_result("DIRECT; PROXY p.test:8080"), None);
        assert_eq!(
            parse_pac_result("HTTPS unsupported; PROXY p.test:8080; DIRECT"),
            Some(Proxy {
                kind: ProxyKind::Http,
                host: "p.test".into(),
                port: 8080,
                auth: None,
            })
        );
        assert_eq!(
            parse_pac_result("SOCKS5 10.0.0.2:1080"),
            Some(Proxy {
                kind: ProxyKind::Socks5 { remote_dns: true },
                host: "10.0.0.2".into(),
                port: 1080,
                auth: None,
            })
        );

        assert_eq!(
            parse_winhttp_proxy_list("http=127.0.0.1:8080;https=127.0.0.1:8443", "https")
                .expect("https")
                .port,
            8443
        );
        assert_eq!(
            parse_winhttp_proxy_list("http=127.0.0.1:8080;https=127.0.0.1:8443", "http")
                .expect("http")
                .port,
            8080
        );
        assert_eq!(
            parse_winhttp_proxy_list("socks=10.0.0.3:1080", "https")
                .expect("socks")
                .kind,
            ProxyKind::Socks5 { remote_dns: true }
        );
        assert_eq!(
            parse_winhttp_proxy_list("10.0.0.4:3128", "http").expect("bare").port,
            3128
        );

        assert!(bypassed_windows("intranet", "<local>;*.corp"));
        assert!(bypassed_windows("api.corp", "<local>;*.corp"));
        assert!(!bypassed_windows("example.com", "<local>;*.corp"));
    }
}
