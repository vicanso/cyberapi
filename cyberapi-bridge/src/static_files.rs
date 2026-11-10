//! The page, served by the bridge itself: one origin for the page and the
//! API, so the login cookie can stay `SameSite=Strict`, and one file to
//! deploy.
//!
//! A release bridge carries `cyberapi-web/www` compiled in ([`WebBuild`]);
//! `build.rs` refuses one whose module is missing. A debug bridge reads the
//! directory from disk on every request (rust-embed without `debug-embed`),
//! so `make web-bundle` refreshes the page with no rebuild. `--www <dir>`
//! serves a directory instead, in either.
//!
//! Every answer carries its content hash as an ETag. The page and the wasm
//! ask to be revalidated, because the module keeps its name from build to
//! build. Fonts and icons stay for a day and still revalidate when the
//! hash changes. A gzip a release bundle stores beside a file is sent
//! instead of it to a browser that accepts gzip.
//!
//! Under `--base-path` the page lives at `/prefix/` and nowhere else
//! ([`mounted`]): the page addresses everything relative to where it was
//! loaded from, so it needs no telling.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

#[derive(RustEmbed)]
#[folder = "../cyberapi-web/www"]
// wasm-pack's typings and package manifest are for an npm consumer, which
// the page is not.
#[exclude = "wasm/*.d.ts"]
#[exclude = "wasm/package.json"]
#[exclude = "wasm/.gitignore"]
pub struct WebBuild;

/// `--base-path` as typed, made `/a/b`: no slash at the end, one at the
/// start; empty for the root. Each segment plain — letters, digits and
/// `- _ . ~` — so it can stand in a route and a cookie's path as it is.
pub fn normalize_base_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~');
    for segment in trimmed.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." || !segment.chars().all(plain) {
            return Err(format!(
                "base path {raw:?}: each segment may hold letters, digits and - _ . ~ only"
            ));
        }
    }
    Ok(format!("/{trimmed}"))
}

/// Where a request's path falls for a bridge mounted at `base` (as
/// [`normalize_base_path`] leaves it).
#[derive(Debug, PartialEq, Eq)]
pub enum Mounted<'a> {
    /// Under the mount: the path below it, from its `/`.
    File(&'a str),
    /// The mount without its slash: the page's relative addresses would
    /// resolve against the parent, so it is sent to `base/`.
    NeedsSlash,
    /// Not this bridge's.
    Outside,
}

pub fn mounted<'a>(base: &str, path: &'a str) -> Mounted<'a> {
    if base.is_empty() {
        return Mounted::File(path);
    }
    match path.strip_prefix(base) {
        Some("") => Mounted::NeedsSlash,
        Some(rest) if rest.starts_with('/') => Mounted::File(rest),
        _ => Mounted::Outside,
    }
}

/// How long a browser may keep `key` without asking again. The page and
/// the wasm change under a stable name, so they are revalidated. Fonts
/// and icons are named for what they are and change rarely; the ETag
/// still catches a replacement.
fn cache_control(key: &str) -> &'static str {
    if key == "index.html" || key.starts_with("wasm/") {
        "no-cache"
    } else if key.starts_with("fonts/") || key.starts_with("assets/icons/") {
        "public, max-age=86400"
    } else {
        "no-cache"
    }
}

/// The stored file a request path names: `index.html` for the root, and
/// nothing for a path that climbs out or is not plain.
pub fn key_for(path: &str) -> Option<String> {
    let key = path.trim_start_matches('/');
    let key = if key.is_empty() { "index.html" } else { key };
    let plain = key
        .split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
        && !key.contains('\\');
    plain.then(|| key.to_string())
}

/// Quality weight for `target` in `Accept-Encoding` (`0.0` when absent or
/// refused with `q=0`). An explicit match overrides `*`.
fn encoding_quality(headers: &HeaderMap, target: &str) -> f32 {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|part| {
            let mut pieces = part.split(';');
            let name = pieces.next().unwrap_or_default().trim();
            if !name.eq_ignore_ascii_case(target) && name != "*" {
                return None;
            }
            let q = pieces
                .find_map(|p| p.trim().strip_prefix("q="))
                .and_then(|q| q.parse::<f32>().ok())
                .unwrap_or(1.0);
            Some((name.eq_ignore_ascii_case(target), q))
        })
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)))
        .map(|(_, q)| q)
        .unwrap_or(0.0)
}

/// Whether `Accept-Encoding` accepts `zstd` and does not prefer `gzip` over it.
pub fn prefers_zstd(headers: &HeaderMap) -> bool {
    let zstd = encoding_quality(headers, "zstd");
    zstd > 0.0 && zstd >= encoding_quality(headers, "gzip")
}

/// Whether `Accept-Encoding` takes gzip (and does not refuse it with
/// `q=0`).
pub fn accepts_gzip(headers: &HeaderMap) -> bool {
    encoding_quality(headers, "gzip") > 0.0
}

pub fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

const PAGE: &str = "index.html";
const MAP_START: &str = "/*ASSET_REV*/";
const MAP_END: &str = "/*/ASSET_REV*/";

fn versioned(name: &str) -> bool {
    name != PAGE
        && !name.starts_with("assets/icons/")
        && !name.ends_with(".d.ts")
        && !name.ends_with("package.json")
        && !name.ends_with(".gitignore")
}

/// Content hashes for the files the page fetches by name, so `index.html`
/// appends `?v=<hash>` and a browser or proxy never pairs `cyberapi_web.js`
/// from one build with `cyberapi_web_bg.wasm` from another.
pub fn embedded_asset_map() -> BTreeMap<String, String> {
    let mut own = BTreeMap::new();
    let mut compressed = BTreeMap::new();
    for key in WebBuild::iter() {
        let Some(file) = WebBuild::get(&key) else {
            continue;
        };
        let hash = file.metadata.sha256_hash();
        let hex: String = hash[..6].iter().map(|b| format!("{b:02x}")).collect();
        let key_str = key.as_ref();
        if let Some(name) = key_str.strip_suffix(".gz").or_else(|| key_str.strip_suffix(".zst")) {
            compressed.insert(name.to_string(), hex);
        } else {
            own.insert(key_str.to_string(), hex);
        }
    }
    for (name, hex) in compressed {
        own.entry(name).or_insert(hex);
    }
    own.retain(|name, _| versioned(name));
    own
}

pub fn with_asset_map(html: &[u8], map: &BTreeMap<String, String>) -> Vec<u8> {
    let text = String::from_utf8_lossy(html);
    let Some(start) = text.find(MAP_START).map(|at| at + MAP_START.len()) else {
        return html.to_vec();
    };
    let Some(end) = text[start..].find(MAP_END).map(|at| start + at) else {
        return html.to_vec();
    };
    let json = serde_json::to_string(map).unwrap_or_else(|_| "{}".to_string());
    format!("{}{json}{}", &text[..start], &text[end..]).into_bytes()
}

/// Whether `If-None-Match` matches `etag`, including when `CompressionLayer`
/// turned `etag` into a weak validator (`W/"…"`).
fn matches_etag(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|tags| {
            tags.split(',')
                .map(str::trim)
                .any(|tag| tag == "*" || tag.trim_start_matches("W/") == etag)
        })
}

/// A GET of the page's files: `path` below the mount.
pub fn serve(headers: &HeaderMap, path: &str) -> Response {
    let Some(key) = key_for(path) else {
        return not_found();
    };
    if key == PAGE {
        let Some(file) = WebBuild::get(&key) else {
            return not_found();
        };
        let body = with_asset_map(&file.data, &embedded_asset_map());
        let hash: [u8; 32] = Sha256::digest(&body).into();
        let etag = format!("\"{}\"", hex::encode(hash));
        let fresh = matches_etag(headers, &etag);
        let mut response = if fresh {
            StatusCode::NOT_MODIFIED.into_response()
        } else {
            Response::new(Body::from(body))
        };
        let response_headers = response.headers_mut();
        let mut set = |name, value: &str| {
            if let Ok(value) = HeaderValue::from_str(value) {
                response_headers.insert(name, value);
            }
        };
        set(header::ETAG, &etag);
        set(header::CACHE_CONTROL, cache_control(&key));
        set(header::VARY, "Accept-Encoding");
        if !fresh {
            set(header::CONTENT_TYPE, "text/html");
        }
        return response;
    }
    let zstd = format!("{key}.zst");
    let gzip = format!("{key}.gz");
    let (file, encoding) = if prefers_zstd(headers) {
        match WebBuild::get(&zstd) {
            Some(file) => (file, Some("zstd")),
            None => match WebBuild::get(&key) {
                Some(file) => (file, None),
                None => return not_found(),
            },
        }
    } else if accepts_gzip(headers) {
        match WebBuild::get(&gzip) {
            Some(file) => (file, Some("gzip")),
            None => match WebBuild::get(&key) {
                Some(file) => (file, None),
                None => return not_found(),
            },
        }
    } else {
        match WebBuild::get(&key) {
            Some(file) => (file, None),
            None => return not_found(),
        }
    };
    let etag = format!("\"{}\"", hex::encode(file.metadata.sha256_hash()));
    let fresh = matches_etag(headers, &etag);
    let content_type = mime_guess::from_path(&key).first_or_octet_stream().to_string();
    let mut response = if fresh {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Response::new(Body::from(file.data.into_owned()))
    };
    let response_headers = response.headers_mut();
    let mut set = |name, value: &str| {
        if let Ok(value) = HeaderValue::from_str(value) {
            response_headers.insert(name, value);
        }
    };
    set(header::ETAG, &etag);
    set(header::CACHE_CONTROL, cache_control(&key));
    set(header::VARY, "Accept-Encoding");
    if !fresh {
        set(header::CONTENT_TYPE, &content_type);
        if let Some(encoding) = encoding {
            set(header::CONTENT_ENCODING, encoding);
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_name_plain_files_only() {
        assert_eq!(key_for("/"), Some("index.html".into()));
        assert_eq!(key_for("/wasm/cyberapi_web.js"), Some("wasm/cyberapi_web.js".into()));
        assert_eq!(key_for("/../Cargo.toml"), None);
        assert_eq!(key_for("/assets//x.svg"), None);
        assert_eq!(key_for("/a\\b"), None);
    }

    #[test]
    fn fonts_and_icons_are_kept_and_the_page_is_revalidated() {
        assert_eq!(cache_control("index.html"), "no-cache");
        assert_eq!(cache_control("wasm/cyberapi_web_bg.wasm"), "no-cache");
        assert_eq!(cache_control("fonts/IBMPlexSans.ttf"), "public, max-age=86400");
        assert_eq!(cache_control("assets/icons/globe.svg"), "public, max-age=86400");
    }

    #[test]
    fn a_base_path_is_one_plain_prefix() {
        assert_eq!(normalize_base_path(""), Ok(String::new()));
        assert_eq!(normalize_base_path(" / "), Ok(String::new()));
        assert_eq!(normalize_base_path("cyberapi"), Ok("/cyberapi".into()));
        assert_eq!(normalize_base_path("/tools/cyberapi/"), Ok("/tools/cyberapi".into()));
        assert!(normalize_base_path("/a//b").is_err());
        assert!(normalize_base_path("/../x").is_err());
        assert!(normalize_base_path("/a b").is_err());
        assert!(normalize_base_path("/a?b").is_err());
    }

    #[test]
    fn only_paths_under_the_mount_are_the_page() {
        assert_eq!(mounted("", "/"), Mounted::File("/"));
        assert_eq!(mounted("", "/wasm/x.js"), Mounted::File("/wasm/x.js"));
        assert_eq!(mounted("/cyberapi", "/cyberapi/"), Mounted::File("/"));
        assert_eq!(
            mounted("/cyberapi", "/cyberapi/fonts/a.ttf"),
            Mounted::File("/fonts/a.ttf")
        );
        assert_eq!(mounted("/cyberapi", "/cyberapi"), Mounted::NeedsSlash);
        assert_eq!(mounted("/cyberapi", "/"), Mounted::Outside);
        assert_eq!(
            mounted("/cyberapi", "/cyberapix/"),
            Mounted::Outside,
            "a longer name is another"
        );
        assert_eq!(mounted("/cyberapi", "/other/cyberapi/"), Mounted::Outside);
    }

    #[test]
    fn gzip_is_taken_unless_refused() {
        let with = |value: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::ACCEPT_ENCODING, HeaderValue::from_static(value));
            headers
        };
        assert!(accepts_gzip(&with("gzip, deflate, br")));
        assert!(accepts_gzip(&with("br;q=1.0, GZIP;q=0.5")));
        assert!(!accepts_gzip(&with("gzip;q=0, br")));
        assert!(!accepts_gzip(&with("identity")));
        assert!(!accepts_gzip(&HeaderMap::new()));
    }

    #[test]
    fn zstd_is_preferred_over_gzip_when_accepted() {
        let with = |value: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::ACCEPT_ENCODING, HeaderValue::from_static(value));
            headers
        };
        assert!(prefers_zstd(&with("gzip, deflate, br, zstd")));
        assert!(prefers_zstd(&with("zstd")));
        assert!(prefers_zstd(&with("*")));
        assert!(!prefers_zstd(&with("gzip, deflate, br")));
        assert!(!prefers_zstd(&with("zstd;q=0, gzip")));
        assert!(!prefers_zstd(&with("gzip;q=1.0, zstd;q=0.5")));
        assert!(!prefers_zstd(&HeaderMap::new()));
    }

    /// The page from the directory a debug build reads: its type, its tag,
    /// and nothing sent again for a tag the browser has.
    #[test]
    fn the_page_is_served_and_revalidated() {
        let page = serve(&HeaderMap::new(), "/");
        assert_eq!(page.status(), StatusCode::OK);
        assert_eq!(page.headers()[header::CONTENT_TYPE], "text/html");
        let etag = page.headers()[header::ETAG].clone();

        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, etag.clone());
        let again = serve(&headers, "/index.html");
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);

        let weak = HeaderValue::from_str(&format!("W/{}", etag.to_str().expect("ascii"))).expect("header");
        headers.insert(header::IF_NONE_MATCH, weak);
        let weak_again = serve(&headers, "/index.html");
        assert_eq!(weak_again.status(), StatusCode::NOT_MODIFIED);

        let mapped = String::from_utf8(with_asset_map(
            b"const ASSET_REV = /*ASSET_REV*/{}/*/ASSET_REV*/;",
            &BTreeMap::from([("wasm/cyberapi_web.js".to_string(), "1a2b3c4d5e6f".to_string())]),
        ))
        .expect("utf8");
        assert_eq!(
            mapped,
            r#"const ASSET_REV = /*ASSET_REV*/{"wasm/cyberapi_web.js":"1a2b3c4d5e6f"}/*/ASSET_REV*/;"#
        );

        let missing = serve(&HeaderMap::new(), "/no-such-file");
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        let climbing = serve(&HeaderMap::new(), "/../Cargo.toml");
        assert_eq!(climbing.status(), StatusCode::NOT_FOUND);
    }
}
