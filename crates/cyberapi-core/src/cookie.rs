//! Cookies (M9): what a `Set-Cookie` keeps, and which kept cookies go with a
//! request — the parts of RFC 6265 an API client needs. "Now" is the
//! caller's: this crate has no clock.
//!
//! As before (F9.2 / F9.3): a cookie is received from every response and
//! sent with every request it matches; an expired one is removed. Unlike
//! before, a session cookie (no `Expires` / `Max-Age`) is kept until it is
//! deleted, as an API client's cookie jar usually is, rather than until the
//! app restarts.

use crate::Timestamp;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// Lower case, without a leading dot.
    pub domain: String,
    pub path: String,
    /// `None`: a session cookie.
    pub expires: Option<Timestamp>,
    pub secure: bool,
    pub http_only: bool,
    /// Sent to `domain` only, not its subdomains: the response named no
    /// `Domain`.
    pub host_only: bool,
}

impl Cookie {
    /// What makes a cookie the same one: a new one with this key replaces it
    /// (F9.7).
    pub fn key(&self) -> (&str, &str, &str) {
        (&self.domain, &self.path, &self.name)
    }

    pub fn is_expired(&self, now: Timestamp) -> bool {
        self.expires.is_some_and(|expires| expires.get() <= now.get())
    }

    /// Whether it goes with a request to `url` (F9.3).
    pub fn matches(&self, url: &Url, now: Timestamp) -> bool {
        let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
            return false;
        };
        let domain_ok = if self.host_only {
            host == self.domain
        } else {
            domain_match(&host, &self.domain)
        };
        domain_ok
            && path_match(url.path(), &self.path)
            && (!self.secure || url.scheme() == "https")
            && !self.is_expired(now)
    }
}

/// `host` is `domain` or one of its subdomains.
fn domain_match(host: &str, domain: &str) -> bool {
    host == domain || host.strip_suffix(domain).is_some_and(|rest| rest.ends_with('.'))
}

/// RFC 6265 §5.1.4.
fn path_match(request: &str, cookie: &str) -> bool {
    request == cookie
        || (request.starts_with(cookie) && (cookie.ends_with('/') || request[cookie.len()..].starts_with('/')))
}

/// The directory of the request's path: where a cookie without `Path` holds.
fn default_path(url: &Url) -> String {
    let path = url.path();
    match path.rfind('/') {
        Some(0) | None => "/".into(),
        Some(end) => path[..end].into(),
    }
}

/// `Wed, 21 Oct 2015 07:28:00 GMT`, and the older `Wed, 21-Oct-2015 …`.
fn parse_expires(text: &str) -> Option<Timestamp> {
    let text = text.trim();
    if let Ok(at) = DateTime::parse_from_rfc2822(text) {
        return Some(Timestamp::new(at));
    }
    for format in [
        "%a, %d-%b-%Y %H:%M:%S GMT",
        "%a, %d-%b-%y %H:%M:%S GMT",
        "%A, %d-%b-%y %H:%M:%S GMT",
    ] {
        if let Ok(at) = NaiveDateTime::parse_from_str(text, format) {
            return Some(Timestamp::new(at.and_utc().fixed_offset()));
        }
    }
    None
}

/// The cookie a `Set-Cookie` header from `url` sets (F9.2): `None` when it
/// is not one, or names a domain the response may not set. An expired one
/// is returned so it can remove its namesake.
pub fn parse_set_cookie(header: &str, url: &Url, now: Timestamp) -> Option<Cookie> {
    let host = url.host_str()?.to_ascii_lowercase();
    let mut parts = header.split(';');
    let (name, value) = parts.next()?.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut cookie = Cookie {
        name: name.to_string(),
        value: value.trim().trim_matches('"').to_string(),
        domain: host.clone(),
        path: default_path(url),
        host_only: true,
        ..Default::default()
    };
    let mut max_age = None;
    for attribute in parts {
        let (key, value) = match attribute.split_once('=') {
            Some((key, value)) => (key.trim(), value.trim()),
            None => (attribute.trim(), ""),
        };
        match key.to_ascii_lowercase().as_str() {
            "domain" => {
                let domain = value.trim_start_matches('.').to_ascii_lowercase();
                if domain.is_empty() {
                    continue;
                }
                // A response sets cookies for itself and its parents only.
                if !domain_match(&host, &domain) {
                    return None;
                }
                cookie.domain = domain;
                cookie.host_only = false;
            }
            "path" if value.starts_with('/') => cookie.path = value.to_string(),
            "expires" => {
                if let Some(at) = parse_expires(value) {
                    cookie.expires.get_or_insert(at);
                }
            }
            "max-age" => max_age = value.parse::<i64>().ok(),
            "secure" => cookie.secure = true,
            "httponly" => cookie.http_only = true,
            _ => {}
        }
    }
    // Max-Age wins over Expires.
    if let Some(seconds) = max_age {
        let at = if seconds <= 0 {
            DateTime::<Utc>::UNIX_EPOCH.fixed_offset()
        } else {
            now.get() + Duration::seconds(seconds)
        };
        cookie.expires = Some(Timestamp::new(at));
    }
    Some(cookie)
}

/// The `Cookie` header for a request to `url` (F9.3): longer paths first.
pub fn cookie_header(cookies: &[Cookie], url: &Url, now: Timestamp) -> Option<String> {
    let mut sent: Vec<&Cookie> = cookies.iter().filter(|cookie| cookie.matches(url, now)).collect();
    if sent.is_empty() {
        return None;
    }
    sent.sort_by_key(|cookie| std::cmp::Reverse(cookie.path.len()));
    Some(
        sent.iter()
            .map(|cookie| format!("{}={}", cookie.name, cookie.value))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// The old app's `cookies.json` (cookie_store's format, one cookie per
/// line): `raw_cookie` is the `Set-Cookie` it came from, with the domain,
/// path and expiry beside it as cookie_store worked them out. Unreadable
/// lines are skipped.
pub fn parse_legacy_jar(text: &str) -> Vec<Cookie> {
    #[derive(Deserialize)]
    struct Line {
        raw_cookie: String,
        path: (String, bool),
        domain: serde_json::Value,
        expires: serde_json::Value,
    }
    text.lines()
        .filter_map(|line| serde_json::from_str::<Line>(line.trim()).ok())
        .filter_map(|line| {
            let mut parts = line.raw_cookie.split(';');
            let (name, value) = parts.next()?.split_once('=')?;
            let attributes: Vec<String> = parts.map(|part| part.trim().to_ascii_lowercase()).collect();
            let (domain, host_only) = match &line.domain {
                serde_json::Value::Object(map) => match (map.get("HostOnly"), map.get("Suffix")) {
                    (Some(host), _) => (host.as_str()?.to_string(), true),
                    (_, Some(suffix)) => (suffix.as_str()?.trim_start_matches('.').to_string(), false),
                    _ => return None,
                },
                _ => return None,
            };
            let expires = line
                .expires
                .get("AtUtc")
                .and_then(|at| at.as_str())
                .map(Timestamp::parse_lenient);
            Some(Cookie {
                name: name.trim().to_string(),
                value: value.trim().to_string(),
                domain: domain.to_ascii_lowercase(),
                path: line.path.0,
                expires,
                secure: attributes.iter().any(|attribute| attribute == "secure"),
                http_only: attributes.iter().any(|attribute| attribute == "httponly"),
                host_only,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn now() -> Timestamp {
        Timestamp::parse_lenient("2026-01-01T00:00:00Z")
    }

    fn url(text: &str) -> Url {
        Url::parse(text).expect("a URL")
    }

    #[test]
    fn set_cookie_reads_its_attributes() {
        let from = url("https://api.example.com/v1/users/me");
        let cookie = parse_set_cookie(
            "sid=abc; Domain=.Example.com; Path=/v1; Secure; HttpOnly; Expires=Wed, 21 Oct 2026 07:28:00 GMT",
            &from,
            now(),
        )
        .expect("a cookie");
        assert_eq!(
            cookie,
            Cookie {
                name: "sid".into(),
                value: "abc".into(),
                domain: "example.com".into(),
                path: "/v1".into(),
                expires: Some(Timestamp::parse_lenient("2026-10-21T07:28:00Z")),
                secure: true,
                http_only: true,
                host_only: false,
            }
        );

        // Without Domain and Path: this host, this directory, a session.
        let plain = parse_set_cookie("a=\"1\"", &from, now()).expect("a cookie");
        assert_eq!(
            (plain.domain.as_str(), plain.path.as_str()),
            ("api.example.com", "/v1/users")
        );
        assert!(plain.host_only && plain.expires.is_none());
        assert_eq!(plain.value, "1");

        // Max-Age counts from now and wins over Expires; 0 expires it.
        let aged =
            parse_set_cookie("a=1; Expires=Wed, 21 Oct 2099 07:28:00 GMT; Max-Age=60", &from, now()).expect("a cookie");
        assert_eq!(aged.expires, Some(Timestamp::parse_lenient("2026-01-01T00:01:00Z")));
        assert!(
            parse_set_cookie("a=1; Max-Age=0", &from, now())
                .expect("a cookie")
                .is_expired(now())
        );
        let old_style = parse_set_cookie("a=1; expires=Thu, 01-Jan-1970 00:00:01 GMT", &from, now());
        assert!(old_style.expect("a cookie").is_expired(now()));

        // A response cannot set another site's cookie.
        assert_eq!(parse_set_cookie("a=1; Domain=evil.test", &from, now()), None);
        assert_eq!(parse_set_cookie("novalue", &from, now()), None);
    }

    #[test]
    fn requests_get_the_cookies_that_match() {
        let jar = [
            Cookie {
                name: "root".into(),
                value: "r".into(),
                domain: "example.com".into(),
                path: "/".into(),
                ..Default::default()
            },
            Cookie {
                name: "deep".into(),
                value: "d".into(),
                domain: "example.com".into(),
                path: "/v1".into(),
                ..Default::default()
            },
            Cookie {
                name: "host".into(),
                value: "h".into(),
                domain: "example.com".into(),
                path: "/".into(),
                host_only: true,
                ..Default::default()
            },
            Cookie {
                name: "secure".into(),
                value: "s".into(),
                domain: "example.com".into(),
                path: "/".into(),
                secure: true,
                ..Default::default()
            },
            Cookie {
                name: "gone".into(),
                value: "g".into(),
                domain: "example.com".into(),
                path: "/".into(),
                expires: Some(Timestamp::parse_lenient("2020-01-01T00:00:00Z")),
                ..Default::default()
            },
        ];
        assert_eq!(
            cookie_header(&jar, &url("http://api.example.com/v1/x"), now()).as_deref(),
            Some("deep=d; root=r")
        );
        assert_eq!(
            cookie_header(&jar, &url("https://example.com/v10"), now()).as_deref(),
            Some("root=r; host=h; secure=s")
        );
        assert_eq!(cookie_header(&jar, &url("http://other.test/"), now()), None);
        assert_eq!(cookie_header(&jar, &url("http://notexample.com/"), now()), None);
    }

    #[test]
    fn the_old_apps_jar_reads() {
        let text = r#"{"raw_cookie":"T=abc; HttpOnly; Path=/; Max-Age=31536000","path":["/",true],"domain":{"HostOnly":"me.gf.com.cn"},"expires":{"AtUtc":"2027-08-10T08:21:30Z"}}
{"raw_cookie":"S=1; Secure","path":["/a",false],"domain":{"Suffix":".gf.com.cn"},"expires":"SessionEnd"}
not json
"#;
        assert_eq!(
            parse_legacy_jar(text),
            [
                Cookie {
                    name: "T".into(),
                    value: "abc".into(),
                    domain: "me.gf.com.cn".into(),
                    path: "/".into(),
                    expires: Some(Timestamp::parse_lenient("2027-08-10T08:21:30Z")),
                    secure: false,
                    http_only: true,
                    host_only: true,
                },
                Cookie {
                    name: "S".into(),
                    value: "1".into(),
                    domain: "gf.com.cn".into(),
                    path: "/a".into(),
                    expires: None,
                    secure: true,
                    http_only: false,
                    host_only: false,
                },
            ]
        );
    }
}
