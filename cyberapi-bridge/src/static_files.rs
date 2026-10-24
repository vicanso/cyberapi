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
//! Every answer carries its content hash as an ETag and asks to be
//! revalidated: the module keeps its name from build to build. A gzip a
//! release bundle stores beside a file is sent instead of it to a browser
//! that accepts gzip.
//!
//! Under `--base-path` the page lives at `/prefix/` and nowhere else
//! ([`mounted`]): the page addresses everything relative to where it was
//! loaded from, so it needs no telling.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

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

/// Whether `Accept-Encoding` takes gzip (and does not refuse it with
/// `q=0`).
pub fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|part| {
            let mut pieces = part.split(';');
            let name = pieces.next().unwrap_or_default().trim();
            let refused = pieces.any(|p| matches!(p.trim().strip_prefix("q="), Some(q) if q.parse::<f32>() == Ok(0.0)));
            (name.eq_ignore_ascii_case("gzip") || name == "*") && !refused
        })
}

pub fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

/// A GET of the page's files: `path` below the mount.
pub fn serve(headers: &HeaderMap, path: &str) -> Response {
    let Some(key) = key_for(path) else {
        return not_found();
    };
    let gzip = format!("{key}.gz");
    let (file, encoding) = match WebBuild::get(&gzip).filter(|_| accepts_gzip(headers)) {
        Some(file) => (file, Some("gzip")),
        None => match WebBuild::get(&key) {
            Some(file) => (file, None),
            None => return not_found(),
        },
    };
    let etag = format!("\"{}\"", hex::encode(file.metadata.sha256_hash()));
    let fresh = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|tags| tags.split(',').any(|tag| tag.trim() == etag));
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
    set(header::CACHE_CONTROL, "no-cache");
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

    /// The page from the directory a debug build reads: its type, its tag,
    /// and nothing sent again for a tag the browser has.
    #[test]
    fn the_page_is_served_and_revalidated() {
        let page = serve(&HeaderMap::new(), "/");
        assert_eq!(page.status(), StatusCode::OK);
        assert_eq!(page.headers()[header::CONTENT_TYPE], "text/html");
        let etag = page.headers()[header::ETAG].clone();

        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, etag);
        let again = serve(&headers, "/index.html");
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);

        let missing = serve(&HeaderMap::new(), "/no-such-file");
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        let climbing = serve(&HeaderMap::new(), "/../Cargo.toml");
        assert_eq!(climbing.status(), StatusCode::NOT_FOUND);
    }
}
