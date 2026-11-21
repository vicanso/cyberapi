//! C4: AWS Signature Version 4 request signing, implemented directly over
//! `hmac`, `sha2`, `percent-encoding`, `url`, and `jiff` (no AWS SDK crates).
//!
//! Signing runs as the final step of [`crate::send::prepare`], after template
//! evaluation and body serialization, so the payload hash matches the exact
//! bytes sent on the wire.

use hmac::{Hmac, KeyInit as _, Mac as _};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use url::Url;

type HmacSha256 = Hmac<Sha256>;

/// AWS SigV4 URI-encoding set: unreserved characters `A-Z a-z 0-9 - _ . ~` are
/// left unencoded; everything else is percent-encoded in uppercase hex.
const AWS_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

/// Stored AWS SigV4 credentials (template expressions allowed in every field).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwsSigV4 {
    #[serde(default)]
    pub access_key: String,
    #[serde(default)]
    pub secret_key: String,
    #[serde(default)]
    pub session_token: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub service: String,
}

impl AwsSigV4 {
    pub fn has_credentials(&self) -> bool {
        !self.access_key.trim().is_empty() && !self.secret_key.trim().is_empty()
    }

    pub fn templates(&self) -> Vec<&str> {
        vec![
            &self.access_key,
            &self.secret_key,
            &self.session_token,
            &self.region,
            &self.service,
        ]
    }

    pub fn login<E>(&self, render: impl Fn(&str) -> Result<String, E>) -> Result<AwsSigV4Login, E> {
        Ok(AwsSigV4Login {
            access_key: render(&self.access_key)?,
            secret_key: render(&self.secret_key)?,
            session_token: render(&self.session_token)?,
            region: render(&self.region)?,
            service: render(&self.service)?,
        })
    }
}

/// Rendered AWS SigV4 credentials, ready to sign a request or format cURL's
/// `--aws-sigv4` option.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AwsSigV4Login {
    pub access_key: String,
    pub secret_key: String,
    pub session_token: String,
    pub region: String,
    pub service: String,
}

impl AwsSigV4Login {
    pub fn effective_region(&self) -> &str {
        let region = self.region.trim();
        if region.is_empty() { "us-east-1" } else { region }
    }

    pub fn effective_service(&self) -> &str {
        self.service.trim()
    }
}

/// Signs a finalized HTTP request with AWS Signature Version 4 at `now_unix_secs`
/// (UTC epoch seconds). Returns the headers to attach (`X-Amz-Date`, optional
/// `X-Amz-Security-Token`, optional `X-Amz-Content-Sha256`, and `Authorization`).
pub fn sign(
    method: &str,
    url: &Url,
    headers: &[(String, String)],
    body: &[u8],
    login: &AwsSigV4Login,
    now_unix_secs: i64,
) -> Vec<(String, String)> {
    let access_key = login.access_key.trim();
    let secret_key = login.secret_key.trim();
    if access_key.is_empty() || secret_key.is_empty() {
        return Vec::new();
    }
    let region = login.effective_region();
    let service = login.effective_service();
    let session_token = login.session_token.trim();

    let (amz_date, date_stamp) = format_amz_timestamps(now_unix_secs);
    let is_s3 = service.eq_ignore_ascii_case("s3");

    // Existing x-amz-content-sha256 (e.g. UNSIGNED-PAYLOAD) wins if provided.
    let existing_payload_hash = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("x-amz-content-sha256"))
        .map(|(_, v)| v.trim().to_string());
    let payload_hash = existing_payload_hash
        .clone()
        .unwrap_or_else(|| hex_lower(&Sha256::digest(body)));

    let mut added: Vec<(String, String)> = Vec::new();
    let has_header = |name: &str| headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));

    let effective_amz_date = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("x-amz-date"))
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_else(|| {
            added.push(("X-Amz-Date".into(), amz_date.clone()));
            amz_date
        });

    // If the caller already set X-Amz-Date, derive date_stamp from its first 8 chars if valid.
    let effective_date_stamp =
        if effective_amz_date.len() >= 8 && effective_amz_date.as_bytes()[..8].iter().all(u8::is_ascii_digit) {
            effective_amz_date[..8].to_string()
        } else {
            date_stamp
        };

    if !session_token.is_empty() && !has_header("x-amz-security-token") {
        added.push(("X-Amz-Security-Token".into(), session_token.to_string()));
    }
    if is_s3 && existing_payload_hash.is_none() {
        added.push(("X-Amz-Content-Sha256".into(), payload_hash.clone()));
    }

    // Build canonical headers map (lowercase name -> list of trimmed/collapsed values).
    let mut canon_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, value) in headers.iter().chain(added.iter()) {
        let lower = name.trim().to_ascii_lowercase();
        if lower.is_empty() || lower == "authorization" {
            continue;
        }
        canon_map.entry(lower).or_default().push(collapse_ws(value));
    }
    if !canon_map.contains_key("host")
        && let Some(host) = host_header(url)
    {
        canon_map.insert("host".into(), vec![host]);
    }

    let mut canonical_headers = String::new();
    let mut signed_names: Vec<&str> = Vec::with_capacity(canon_map.len());
    for (name, values) in &canon_map {
        signed_names.push(name.as_str());
        canonical_headers.push_str(name);
        canonical_headers.push(':');
        canonical_headers.push_str(&values.join(","));
        canonical_headers.push('\n');
    }
    let signed_headers = signed_names.join(";");

    let canonical_uri = canonical_uri(url.path(), is_s3);
    let canonical_query = canonical_query(url.query());
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method.to_ascii_uppercase(),
        canonical_uri,
        canonical_query,
        canonical_headers,
        signed_headers,
        payload_hash
    );

    let credential_scope = format!("{effective_date_stamp}/{region}/{service}/aws4_request");
    let canonical_request_hash = hex_lower(&Sha256::digest(canonical_request.as_bytes()));
    let string_to_sign =
        format!("AWS4-HMAC-SHA256\n{effective_amz_date}\n{credential_scope}\n{canonical_request_hash}");

    let k_date = hmac_sha256(format!("AWS4{secret_key}").as_bytes(), effective_date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex_lower(&hmac_sha256(&k_signing, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );
    added.push(("Authorization".into(), authorization));
    added
}

fn format_amz_timestamps(now_unix_secs: i64) -> (String, String) {
    let ts = jiff::Timestamp::from_second(now_unix_secs).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    let dt = ts.to_zoned_datetime(jiff::tz::TimeZone::UTC).datetime();
    let date_stamp = format!("{:04}{:02}{:02}", dt.year(), dt.month(), dt.day());
    let amz_date = format!("{date_stamp}T{:02}{:02}{:02}Z", dt.hour(), dt.minute(), dt.second());
    (amz_date, date_stamp)
}

fn host_header(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    match url.port() {
        Some(port) => Some(format!("{host}:{port}")),
        None => Some(host.to_string()),
    }
}

fn aws_encode(input: &str) -> String {
    utf8_percent_encode(input, AWS_ENCODE_SET).to_string()
}

fn canonical_uri(path: &str, is_s3: bool) -> String {
    if path.is_empty() || path == "/" {
        return "/".to_string();
    }
    let encode_segment = |seg: &str| -> String {
        let decoded = percent_decode_str(seg).decode_utf8_lossy();
        let once = aws_encode(&decoded);
        if is_s3 { once } else { aws_encode(&once) }
    };
    let trailing_slash = path.ends_with('/');
    let mut out = String::new();
    for seg in path.trim_start_matches('/').split('/') {
        out.push('/');
        if !seg.is_empty() {
            out.push_str(&encode_segment(seg));
        }
    }
    if trailing_slash && !out.ends_with('/') {
        out.push('/');
    }
    if out.is_empty() { "/".to_string() } else { out }
}

fn canonical_query(query: Option<&str>) -> String {
    let Some(query) = query else {
        return String::new();
    };
    if query.is_empty() {
        return String::new();
    }
    let mut pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (raw_k, raw_v) = part.split_once('=').unwrap_or((part, ""));
            // In query strings, `+` represents a space in form encoding, while `%XX` is percent-encoded.
            let raw_k = raw_k.replace('+', " ");
            let raw_v = raw_v.replace('+', " ");
            let k = percent_decode_str(&raw_k).decode_utf8_lossy();
            let v = percent_decode_str(&raw_v).decode_utf8_lossy();
            (aws_encode(&k), aws_encode(&v))
        })
        .collect();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn collapse_ws(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut prev_space = false;
    for ch in value.trim().chars() {
        if ch == ' ' || ch == '\t' {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// 2015-08-30T12:36:00Z — the fixed timestamp used across the official
    /// AWS Signature Version 4 Test Suite.
    const TEST_NOW: i64 = 1_440_938_160;

    fn test_login(service: &str) -> AwsSigV4Login {
        AwsSigV4Login {
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: String::new(),
            region: "us-east-1".into(),
            service: service.into(),
        }
    }

    fn auth_header(signed: &[(String, String)]) -> &str {
        signed
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .map(|(_, v)| v.as_str())
            .expect("Authorization header")
    }

    /// Official AWS SigV4 Test Suite: `get-vanilla`
    #[test]
    fn aws_test_suite_get_vanilla() {
        let url = Url::parse("https://example.amazonaws.com/").expect("url");
        let signed = sign("GET", &url, &[], b"", &test_login("service"), TEST_NOW);
        assert_eq!(
            auth_header(&signed),
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    /// Official AWS SigV4 Test Suite: `get-vanilla-query-order-key-case`
    #[test]
    fn aws_test_suite_get_vanilla_query_order_key_case() {
        let url = Url::parse("https://example.amazonaws.com/?Param2=value2&Param1=value1").expect("url");
        let signed = sign("GET", &url, &[], b"", &test_login("service"), TEST_NOW);
        assert_eq!(
            auth_header(&signed),
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500"
        );
    }

    /// Official AWS SigV4 Test Suite: `post-vanilla`
    #[test]
    fn aws_test_suite_post_vanilla() {
        let url = Url::parse("https://example.amazonaws.com/").expect("url");
        let signed = sign("POST", &url, &[], b"", &test_login("service"), TEST_NOW);
        assert_eq!(
            auth_header(&signed),
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5da7c1a2acd57cee7505fc6676e4e544621c30862966e37dddb68e92efbe5d6b"
        );
    }

    /// Official AWS SigV4 Test Suite: `post-x-www-form-urlencoded` (body `foo=bar`)
    #[test]
    fn aws_test_suite_post_form_urlencoded() {
        let url = Url::parse("https://example.amazonaws.com/").expect("url");
        let headers = vec![("Content-Type".into(), "application/x-www-form-urlencoded".into())];
        let signed = sign("POST", &url, &headers, b"foo=bar", &test_login("service"), TEST_NOW);
        assert_eq!(
            auth_header(&signed),
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=e8f0b28e87a3da970c2bafa968b2f115f97e98c86fc0274df59b91056ad85b02"
        );
    }

    /// S3 requests include `X-Amz-Content-Sha256` and single-encode the URI path;
    /// temporary credentials include `X-Amz-Security-Token`.
    #[test]
    fn s3_adds_content_sha256_and_session_token_is_signed() {
        let url = Url::parse("https://my-bucket.s3.us-east-1.amazonaws.com/path/to/my%20file.txt").expect("url");
        let mut login = test_login("s3");
        login.session_token = "AQoDYXdzEJr///token".into();
        let signed = sign("PUT", &url, &[], b"hello", &login, TEST_NOW);

        let sha = signed
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("x-amz-content-sha256"))
            .map(|(_, v)| v.as_str())
            .expect("x-amz-content-sha256");
        assert_eq!(sha, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");

        let tok = signed
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("x-amz-security-token"))
            .map(|(_, v)| v.as_str())
            .expect("x-amz-security-token");
        assert_eq!(tok, "AQoDYXdzEJr///token");

        let auth = auth_header(&signed);
        assert!(auth.contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token"));
        assert!(auth.contains("Credential=AKIDEXAMPLE/20150830/us-east-1/s3/aws4_request"));
    }

    #[test]
    fn send_prepare_curl_and_code_handle_aws_sigv4() {
        use crate::auth::Auth;
        use crate::code::{CodeTarget, code};
        use crate::curl::curl;
        use crate::model::{HttpMethod, HttpRequest};
        use crate::send::{Context, prepare};
        use crate::template::Inputs;

        let request = HttpRequest {
            method: HttpMethod::Post,
            uri: "https://example.amazonaws.com/".into(),
            body: r#"{"ok":true}"#.into(),
            auth: Auth::from(AwsSigV4 {
                access_key: "AKIDEXAMPLE".into(),
                secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
                session_token: "tok-123".into(),
                region: "us-east-1".into(),
                service: "s3".into(),
            }),
            ..Default::default()
        };
        let inputs = Inputs {
            now: TEST_NOW,
            ..Default::default()
        };
        let proxy = crate::proxy::ProxyPrefs::default();
        let inherited = crate::inherit::Inherited::default();
        let cx = Context {
            variables: &[],
            template: &inputs,
            files: &crate::template::NoFiles("test"),
            user_agent: "cyberapi-test",
            timeouts: crate::prefs::Timeouts::default(),
            accept_invalid_certs: false,
            http_version: crate::prefs::HttpVersion::Auto,
            proxy: &proxy,
            boundary: "boundary",
            inherited: &inherited,
            environment: None,
        };
        let prepared = prepare(&request, &cx).expect("prepared");
        assert!(
            prepared
                .header("authorization")
                .is_some_and(|v| v.starts_with("AWS4-HMAC-SHA256 "))
        );
        assert_eq!(prepared.header("x-amz-date"), Some("20150830T123600Z"));
        assert_eq!(prepared.header("x-amz-security-token"), Some("tok-123"));
        assert!(prepared.header("x-amz-content-sha256").is_some());

        let cmd = curl(&prepared, &request, &[], false);
        assert!(cmd.contains("--aws-sigv4 'aws:amz:us-east-1:s3'"));
        assert!(cmd.contains("--user 'AKIDEXAMPLE:wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY'"));
        assert!(cmd.contains("X-Amz-Security-Token: tok-123"));
        assert!(!cmd.contains("Authorization:"));
        assert!(!cmd.contains("X-Amz-Date:"));

        let py = code(CodeTarget::Python, &prepared, &request, false);
        assert!(py.starts_with("# Note: AWS SigV4 signature headers below are time-limited and will expire.\n"));
        assert!(py.contains("AWS4-HMAC-SHA256 "));

        let php = code(CodeTarget::Php, &prepared, &request, false);
        assert!(
            php.starts_with("<?php\n// Note: AWS SigV4 signature headers below are time-limited and will expire.\n")
        );

        let sent = crate::send::sent_request(&prepared, &request, &inherited, &[], false, "HTTP/1.1");
        for name in [
            "Authorization",
            "X-Amz-Date",
            "X-Amz-Security-Token",
            "X-Amz-Content-Sha256",
        ] {
            let found = sent
                .headers
                .iter()
                .find(|h| h.name.eq_ignore_ascii_case(name))
                .expect("signed header present");
            assert_eq!(found.from, crate::response::HeaderSource::Auth, "{name} should be Auth");
        }
    }

    #[test]
    fn postman_insomnia_and_bruno_import_export_aws_sigv4() {
        use crate::auth::Auth;
        use crate::export::postman as export_postman;
        use crate::import::ImportItem;
        use crate::model::{ApiSetting, HttpRequest};
        use crate::tree::TreeData;

        let expected = Auth::from(AwsSigV4 {
            access_key: "AKIA123".into(),
            secret_key: "SECRET456".into(),
            session_token: "TOK789".into(),
            region: "us-west-2".into(),
            service: "execute-api".into(),
        });

        // Postman export -> re-import round-trip.
        let data = TreeData {
            apis: vec![ApiSetting {
                id: "01JAWS".into(),
                name: "Invoke".into(),
                request: HttpRequest {
                    uri: "https://api.example.com/invoke".into(),
                    auth: expected.clone(),
                    ..Default::default()
                },
                ..Default::default()
            }],
            top_order: vec!["01JAWS".into()],
            ..Default::default()
        };
        let exported = export_postman(&data, None, "AWS", &[]);
        let pm_bundle = crate::import::parse(&exported).expect("postman parsed");
        let ImportItem::Api { request, .. } = &pm_bundle.items[0] else {
            panic!("api");
        };
        assert_eq!(request.auth, expected);

        // Insomnia import.
        let insomnia = r#"{"_type":"export","__export_format":4,"resources":[
            {"_id":"wrk_1","_type":"workspace","name":"AWS"},
            {"_id":"req_1","_type":"request","parentId":"wrk_1","name":"Invoke","method":"GET","url":"https://api.example.com/invoke",
             "authentication":{"type":"aws","accessKeyId":"AKIA123","secretAccessKey":"SECRET456","sessionToken":"TOK789","region":"us-west-2","service":"execute-api"}}
        ]}"#;
        let ins_bundle = crate::import::parse(insomnia).expect("insomnia parsed");
        let ImportItem::Api { request, .. } = &ins_bundle.items[0] else {
            panic!("api");
        };
        assert_eq!(request.auth, expected);

        // Bruno import.
        let bruno = "meta {\n  name: Invoke\n}\n\nget {\n  url: https://api.example.com/invoke\n  auth: awsv4\n}\n\nauth:awsv4 {\n  accessKeyId: AKIA123\n  secretAccessKey: SECRET456\n  sessionToken: TOK789\n  region: us-west-2\n  service: execute-api\n}\n";
        let bru_bundle = crate::import::parse(bruno).expect("bruno parsed");
        let ImportItem::Api { request, .. } = &bru_bundle.items[0] else {
            panic!("api");
        };
        assert_eq!(request.auth, expected);
    }
}
