//! The cURL command of a request (F7.8).
//!
//! Made from the [`PreparedRequest`] — variables resolved, functions run,
//! headers merged — so the command sends what the app sends. One option a
//! line, as Postman and Insomnia write it: `--request`, `--url`, the send
//! settings, `--header`s, then `--data` (a JSON body indented as the app
//! shows it) or `--form` fields; a cookie header from the cookies whose
//! domain is part of the host. Quotes inside values are escaped for the
//! shell. A send that follows redirects is `--location`, one that takes any
//! certificate `--insecure` (F14.8, F14.9), and an HTTP version other than
//! curl's own choice is named (F14.10).

use crate::certificates::CertificateFormat;
use crate::model::{ContentType, HttpMethod, HttpRequest};
use crate::prefs::HttpVersion;
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

/// `text` as it is when the shell reads it so, quoted otherwise: a URL
/// with `?`, `&` or a space must be.
fn word(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._~:/@%+,=-".contains(c));
    if plain { text.to_string() } else { quote(text) }
}

/// Where one option ends and the next begins.
const NEXT: &str = " \\\n  ";

/// `request` is what the API holds, for its multipart fields.
pub fn curl(
    prepared: &PreparedRequest,
    request: &HttpRequest,
    cookies: &[CurlCookie<'_>],
    follow_redirects: bool,
) -> String {
    // A gRPC call is no cURL command: grpcurl makes it; websocat a
    // WebSocket's (E1).
    if request.method == HttpMethod::Grpc {
        return crate::grpc::grpcurl(prepared, &crate::grpc::GrpcBody::read(&request.body));
    }
    if request.method == HttpMethod::WebSocket {
        return crate::ws::websocat(prepared);
    }
    let mut parts = vec![
        format!("curl --request {}", prepared.method.as_str()),
        format!("--url {}", word(&prepared.url)),
    ];
    if follow_redirects {
        parts.push("--location".to_string());
    }
    if prepared.accept_invalid_certs {
        parts.push("--insecure".to_string());
    }
    if let Some(login) = &prepared.digest {
        parts.push("--digest".to_string());
        parts.push(format!(
            "--user {}",
            quote(&format!("{}:{}", login.username, login.password))
        ));
    }
    // C5: the client certificate, and the authority when it is one file
    // (curl takes one bundle).
    if let Some(client) = &prepared.tls.client {
        match client.format {
            CertificateFormat::Pem => {
                parts.push(format!("--cert {}", quote(&client.certificate)));
                parts.push(format!("--key {}", quote(&client.key)));
            }
            CertificateFormat::Pkcs12 => {
                parts.push("--cert-type P12".to_string());
                parts.push(format!(
                    "--cert {}",
                    quote(&format!("{}:{}", client.certificate, client.password))
                ));
            }
        }
    }
    if let [authority] = prepared.tls.authorities.as_slice()
        && !prepared.accept_invalid_certs
    {
        parts.push(format!("--cacert {}", quote(authority)));
    }
    // curl, too, prefers HTTP/2 over TLS and speaks HTTP/1.1 otherwise.
    match prepared.http_version {
        HttpVersion::Auto => {}
        HttpVersion::Http1 => parts.push("--http1.1".to_string()),
        HttpVersion::Http2 if prepared.url.starts_with("https:") => parts.push("--http2".to_string()),
        HttpVersion::Http2 => parts.push("--http2-prior-knowledge".to_string()),
    }
    let multipart = prepared.method.has_body() && request.content_type == ContentType::Multipart;

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
        parts.push(format!("--header {}", quote(&format!("Cookie: {cookie}"))));
    }
    for (name, value) in &prepared.headers {
        // curl writes its own multipart type, with the boundary it picks.
        if multipart && name.eq_ignore_ascii_case("content-type") {
            continue;
        }
        parts.push(format!("--header {}", quote(&format!("{name}: {value}"))));
    }

    if multipart {
        for kv in form_fields(&request.body)
            .iter()
            .filter(|kv| kv.enabled && !kv.key.is_empty())
        {
            let field = match kv.value.strip_prefix("file://") {
                Some(path) => format!("{}=@{path}", kv.key),
                None => format!("{}={}", kv.key, kv.value),
            };
            parts.push(format!("--form {}", quote(&field)));
        }
    } else if !prepared.body.is_empty() {
        let text = String::from_utf8_lossy(&prepared.body);
        let body = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .filter(|_| matches!(request.content_type, ContentType::Json | ContentType::GraphQl))
            .map(|json| crate::format::json(&json))
            .unwrap_or_else(|| text.into_owned());
        parts.push(format!("--data {}", quote(&body)));
    }
    parts.join(NEXT)
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
            ..Default::default()
        }
    }

    #[test]
    fn one_option_a_line_json_indented_and_quotes_escaped() {
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
            curl(&prepared, &request, &cookies, false),
            r#"curl --request POST \
  --url 'https://api.test/v1/users?q=a' \
  --header 'Cookie: sid=1' \
  --header 'Content-Type: application/json' \
  --header 'X-Note: it'\''s' \
  --data '{
    "name": "o'\''neil"
}'"#
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
            curl(&prepared, &request, &[], false),
            "curl --request POST \\\n  --url http://h.test/ \\\n  --form 'a=1' \\\n  --form 'f=@/tmp/x.png'"
        );
    }

    #[test]
    fn a_get_has_no_data() {
        let prepared = PreparedRequest {
            url: "http://h.test/".into(),
            ..Default::default()
        };
        assert_eq!(
            curl(&prepared, &HttpRequest::default(), &[], false),
            "curl --request GET \\\n  --url http://h.test/"
        );
    }

    #[test]
    fn the_send_settings_are_flags() {
        let prepared = PreparedRequest {
            url: "https://h.test/".into(),
            accept_invalid_certs: true,
            ..Default::default()
        };
        assert_eq!(
            curl(&prepared, &HttpRequest::default(), &[], true),
            "curl --request GET \\\n  --url https://h.test/ \\\n  --location \\\n  --insecure"
        );
        let versions = [
            (HttpVersion::Http1, "https://h.test/", "--http1.1"),
            (HttpVersion::Http2, "https://h.test/", "--http2"),
            (HttpVersion::Http2, "http://h.test/", "--http2-prior-knowledge"),
        ];
        for (http_version, url, flag) in versions {
            let prepared = PreparedRequest {
                url: url.into(),
                http_version,
                ..Default::default()
            };
            assert_eq!(
                curl(&prepared, &HttpRequest::default(), &[], false),
                format!("curl --request GET \\\n  --url {url} \\\n  {flag}")
            );
        }
    }

    /// As Postman and Insomnia write it, and as it pastes back in.
    #[test]
    fn it_reads_as_it_is_written_and_pastes_back() {
        let prepared = PreparedRequest {
            method: HttpMethod::Post,
            url: "http://ai.example/ner/single".into(),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("app-token".into(), "t-1".into()),
            ],
            body: r#"{"docid":"test_001","text":"慧科讯业发布新品"}"#.as_bytes().to_vec(),
            ..Default::default()
        };
        let request = HttpRequest {
            method: HttpMethod::Post,
            content_type: ContentType::Json,
            ..Default::default()
        };
        let command = curl(&prepared, &request, &[], false);
        assert_eq!(
            command,
            "curl --request POST \\
  --url http://ai.example/ner/single \\
  --header 'Content-Type: application/json' \\
  --header 'app-token: t-1' \\
  --data '{
    \"docid\": \"test_001\",
    \"text\": \"慧科讯业发布新品\"
}'"
        );
        let pasted = crate::import::curl_command(&command).expect("a cURL command");
        let Some(crate::import::ImportItem::Api { request: api, .. }) = pasted.items.first() else {
            panic!("an API")
        };
        assert_eq!(
            (api.method, api.uri.as_str()),
            (HttpMethod::Post, "http://ai.example/ner/single")
        );
        assert!(
            api.headers
                .iter()
                .any(|header| header.key == "app-token" && header.value == "t-1")
        );
        let body: serde_json::Value = serde_json::from_str(&api.body).expect("JSON");
        assert_eq!(body["text"], "慧科讯业发布新品");
    }
}
