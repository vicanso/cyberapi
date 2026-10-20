//! The cURL command of a request (F7.8).
//!
//! Made from the [`PreparedRequest`] — variables resolved, functions run,
//! headers merged — so the command sends what the app sends. As the old app
//! did: `-v`, the method, a JSON body on one line, a cookie header from the
//! cookies whose domain is part of the host. Unlike it, quotes inside values
//! are escaped for the shell, and a multipart body becomes `-F` fields.

use crate::model::{ContentType, HttpRequest};
use crate::send::{PreparedRequest, form_fields};

/// A stored cookie, as far as the command needs it.
pub struct CurlCookie<'a> {
    pub name: &'a str,
    pub value: &'a str,
    pub domain: &'a str,
}

/// `'text'` for a POSIX shell.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// `request` is what the API holds, for its multipart fields.
pub fn curl(prepared: &PreparedRequest, request: &HttpRequest, cookies: &[CurlCookie<'_>]) -> String {
    let mut parts = vec![
        "curl".to_string(),
        "-v".to_string(),
        format!("-X{}", prepared.method.as_str()),
    ];
    let multipart = prepared.method.has_body() && request.content_type == ContentType::Multipart;

    if multipart {
        for kv in form_fields(&request.body)
            .iter()
            .filter(|kv| kv.enabled && !kv.key.is_empty())
        {
            let field = match kv.value.strip_prefix("file://") {
                Some(path) => format!("{}=@{path}", kv.key),
                None => format!("{}={}", kv.key, kv.value),
            };
            parts.push(format!("-F {}", quote(&field)));
        }
    } else if !prepared.body.is_empty() {
        let text = String::from_utf8_lossy(&prepared.body);
        let body = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .filter(|_| request.content_type == ContentType::Json)
            .map(|json| json.to_string())
            .unwrap_or_else(|| text.into_owned());
        parts.push(format!("-d {}", quote(&body)));
    }

    let host = url::Url::parse(&prepared.url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_default();
    let cookie = cookies
        .iter()
        .filter(|cookie| !cookie.domain.is_empty() && host.contains(cookie.domain.trim_start_matches('.')))
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>()
        .join("; ");
    if !cookie.is_empty() {
        parts.push(format!("-H {}", quote(&format!("Cookie: {cookie}"))));
    }
    for (name, value) in &prepared.headers {
        // curl writes its own multipart type, with the boundary it picks.
        if multipart && name.eq_ignore_ascii_case("content-type") {
            continue;
        }
        parts.push(format!("-H {}", quote(&format!("{name}: {value}"))));
    }
    parts.push(quote(&prepared.url));
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HttpMethod, KvParam};
    use pretty_assertions::assert_eq;

    fn kv(key: &str, value: &str) -> KvParam {
        KvParam {
            key: key.into(),
            value: value.into(),
            enabled: true,
        }
    }

    #[test]
    fn json_goes_on_one_line_and_quotes_are_escaped() {
        let prepared = PreparedRequest {
            method: HttpMethod::Post,
            url: "https://api.test/v1/users?q=a".into(),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("X-Note".into(), "it's".into()),
            ],
            body: b"{\n  \"name\": \"o'neil\"\n}".to_vec(),
            ..Default::default()
        };
        let request = HttpRequest {
            method: HttpMethod::Post,
            ..Default::default()
        };
        let cookies = [
            CurlCookie {
                name: "sid",
                value: "1",
                domain: "api.test",
            },
            CurlCookie {
                name: "other",
                value: "2",
                domain: "elsewhere.test",
            },
        ];
        assert_eq!(
            curl(&prepared, &request, &cookies),
            r#"curl -v -XPOST -d '{"name":"o'\''neil"}' -H 'Cookie: sid=1' -H 'Content-Type: application/json' -H 'X-Note: it'\''s' 'https://api.test/v1/users?q=a'"#
        );
    }

    #[test]
    fn multipart_becomes_fields() {
        let request = HttpRequest {
            method: HttpMethod::Post,
            content_type: ContentType::Multipart,
            body: serde_json::to_string(&[kv("a", "1"), kv("f", "file:///tmp/x.png")]).expect("json"),
            ..Default::default()
        };
        let prepared = PreparedRequest {
            method: HttpMethod::Post,
            url: "http://h.test/".into(),
            headers: vec![("Content-Type".into(), "multipart/form-data; boundary=B".into())],
            body: b"--B...".to_vec(),
            ..Default::default()
        };
        assert_eq!(
            curl(&prepared, &request, &[]),
            "curl -v -XPOST -F 'a=1' -F 'f=@/tmp/x.png' 'http://h.test/'"
        );
    }

    #[test]
    fn a_get_has_no_data() {
        let prepared = PreparedRequest {
            url: "http://h.test/".into(),
            ..Default::default()
        };
        assert_eq!(
            curl(&prepared, &HttpRequest::default(), &[]),
            "curl -v -XGET 'http://h.test/'"
        );
    }
}
