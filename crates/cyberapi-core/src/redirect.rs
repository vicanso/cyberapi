//! Following a redirect (F14.8): the request a 3xx answer leads to, made as
//! a browser makes it. The engine sends one request; the send pipeline asks
//! this for the next, so every hop goes out as its own request, through the
//! bridge's address check too, with the cookie jar consulted again.

use crate::model::HttpMethod;
use crate::send::PreparedRequest;
use url::Url;

/// The redirects one send follows before giving up on a loop, as Postman's
/// default.
pub const MAX_REDIRECTS: usize = 10;

/// What describes a body, dropped with it.
const BODY_HEADERS: [&str; 5] = [
    "content-type",
    "content-length",
    "content-encoding",
    "content-language",
    "content-location",
];

/// What was written for one origin only. The jar's cookies for the next
/// origin are the send pipeline's to add.
const ORIGIN_HEADERS: [&str; 4] = ["authorization", "proxy-authorization", "cookie", "host"];

/// Where an answer of `status` with `headers`, to a request for `from`,
/// sends it: the `Location`, against `from`. `None` when it is not a
/// redirect to follow: not 301, 302, 303, 307 or 308, without a `Location`,
/// or to somewhere other than http(s).
pub fn target(from: &str, status: u16, headers: &[(String, String)]) -> Option<Url> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    let location = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("location"))
        .map(|(_, value)| value.trim())?;
    let to = Url::parse(from).ok()?.join(location).ok()?;
    matches!(to.scheme(), "http" | "https").then_some(to)
}

/// The request an answer of `status` with `headers` points to; `None` when
/// [`target`] finds nowhere to go.
///
/// 303 asks for a GET, and 301 and 302 turn a POST into one, as browsers
/// and curl do; the body goes with the method. 307 and 308 keep both. To
/// another origin, the credentials and the `Host` written for this one stay
/// behind.
pub fn next(request: &PreparedRequest, status: u16, headers: &[(String, String)]) -> Option<PreparedRequest> {
    let to = target(&request.url, status, headers)?;
    let from = Url::parse(&request.url).ok()?;
    let mut next = request.clone();
    let to_get = match status {
        303 => request.method != HttpMethod::Head,
        301 | 302 => request.method == HttpMethod::Post,
        _ => false,
    };
    if to_get {
        next.method = HttpMethod::Get;
        next.body.clear();
        drop_headers(&mut next, &BODY_HEADERS);
    }
    if origin(&from) != origin(&to) {
        drop_headers(&mut next, &ORIGIN_HEADERS);
        // A Digest login, like the Authorization it would make.
        next.digest = None;
    }
    next.url = to.to_string();
    Some(next)
}

fn drop_headers(request: &mut PreparedRequest, names: &[&str]) {
    request
        .headers
        .retain(|(name, _)| !names.iter().any(|dropped| name.eq_ignore_ascii_case(dropped)));
}

fn origin(url: &Url) -> (&str, Option<&str>, Option<u16>) {
    (url.scheme(), url.host_str(), url.port_or_known_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::{HttpVersion, Timeouts};
    use pretty_assertions::assert_eq;

    fn post() -> PreparedRequest {
        PreparedRequest {
            method: HttpMethod::Post,
            url: "https://api.test/v1/login?x=1".into(),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("Authorization".into(), "Basic YTpi".into()),
                ("Cookie".into(), "sid=1".into()),
                ("X-Trace".into(), "t".into()),
            ],
            body: b"{}".to_vec(),
            timeouts: Timeouts::default().effective(),
            accept_invalid_certs: true,
            http_version: HttpVersion::Http2,
            streams: false,
            proxy: Default::default(),
            digest: Some(crate::digest::DigestLogin {
                username: "a".into(),
                password: "b".into(),
            }),
        }
    }

    fn location(to: &str) -> Vec<(String, String)> {
        vec![("location".into(), to.into())]
    }

    fn names(request: &PreparedRequest) -> Vec<&str> {
        request.headers.iter().map(|(name, _)| name.as_str()).collect()
    }

    #[test]
    fn a_post_redirected_by_302_or_303_becomes_a_get_without_its_body() {
        for status in [301, 302, 303] {
            let next = next(&post(), status, &location("/v1/me")).expect("followed");
            assert_eq!(next.method, HttpMethod::Get, "{status}");
            assert_eq!(next.url, "https://api.test/v1/me");
            assert!(next.body.is_empty());
            assert_eq!(names(&next), vec!["Authorization", "Cookie", "X-Trace"], "same origin");
            assert!(next.accept_invalid_certs, "the settings go along");
            assert_eq!(next.http_version, HttpVersion::Http2);
        }
    }

    #[test]
    fn a_307_or_308_keeps_the_method_and_body() {
        for status in [307, 308] {
            let next = next(&post(), status, &location("other")).expect("followed");
            assert_eq!(next.method, HttpMethod::Post);
            assert_eq!(next.body, b"{}".to_vec());
            assert_eq!(next.url, "https://api.test/v1/other", "relative to the request");
            assert_eq!(names(&next), names(&post()));
        }
        // 301 and 302 change a POST only; 303 any method but HEAD.
        let put = PreparedRequest {
            method: HttpMethod::Put,
            ..post()
        };
        assert_eq!(
            next(&put, 302, &location("/a")).map(|n| n.method),
            Some(HttpMethod::Put)
        );
        assert_eq!(
            next(&put, 303, &location("/a")).map(|n| n.method),
            Some(HttpMethod::Get)
        );
        let head = PreparedRequest {
            method: HttpMethod::Head,
            body: Vec::new(),
            ..post()
        };
        assert_eq!(
            next(&head, 303, &location("/a")).map(|n| n.method),
            Some(HttpMethod::Head)
        );
    }

    #[test]
    fn credentials_stay_with_their_origin() {
        let request = PreparedRequest {
            headers: vec![
                ("Host".into(), "api.test".into()),
                ("authorization".into(), "Bearer t".into()),
                ("X-Trace".into(), "t".into()),
            ],
            ..post()
        };
        for elsewhere in [
            "https://cdn.test/file",
            "http://api.test/v1/me",
            "https://api.test:8443/v1/me",
        ] {
            let next = next(&request, 307, &location(elsewhere)).expect("followed");
            assert_eq!(names(&next), vec!["X-Trace"], "{elsewhere}");
            assert_eq!(next.digest, None, "a Digest login too: {elsewhere}");
        }
        let same = next(&request, 307, &location("https://api.test:443/v1/me")).expect("followed");
        assert!(same.digest.is_some());
        assert_eq!(
            names(&same),
            vec!["Host", "authorization", "X-Trace"],
            "the default port is the origin's"
        );
    }

    #[test]
    fn only_a_redirect_with_somewhere_to_go_is_followed() {
        assert_eq!(next(&post(), 200, &location("/a")), None);
        assert_eq!(next(&post(), 304, &location("/a")), None, "not modified is an answer");
        assert_eq!(next(&post(), 302, &[]), None, "nowhere to go");
        assert_eq!(next(&post(), 302, &location("ftp://files.test/a")), None);
        assert_eq!(next(&post(), 302, &location("mailto:a@b.test")), None);
        let mixed_case = vec![("Location".to_string(), " /b ".to_string())];
        assert_eq!(
            next(&post(), 302, &mixed_case).map(|n| n.url),
            Some("https://api.test/b".to_string())
        );
    }
}
