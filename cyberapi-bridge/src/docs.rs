//! G3: read-only HTML project documentation served at `GET /v1/docs/{project}`.
//!
//! A project an editor has published (`ProjectSettings::public_docs`) is
//! readable with no session. Otherwise the page needs a signed-in session
//! (cookie or bearer token) or an MCP token (`?token=cat_…`,
//! `Authorization: Bearer cat_…`, or `X-CyberAPI-Token: cat_…`) for any
//! project the account can view. A browser that opens a private address
//! while signed out is sent to the sign-in page and, after signing in,
//! back here. Any other client still gets 401.

use crate::Bridge;
use crate::auth::signed_in_or_token;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use cyberapi_core::CollectionId;
use cyberapi_core::html_docs;
use cyberapi_core::model::VariableCategory;
use cyberapi_store::StoreError;
use cyberapi_store_sql::PublishedDocs;

/// Authenticated page: the browser must revalidate, and a shared cache must
/// not keep it. `font-src 'self'` lets the page load the bridge's fonts.
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; font-src 'self'; img-src data:; base-uri 'none'; frame-ancestors 'none'";

pub async fn get(State(bridge): State<Bridge>, Path(project): Path<String>, request: Request) -> Response {
    let id = CollectionId::from(project);
    // Published first, so a stale `?token=` does not turn a public page
    // into a 401, and a private id is not distinguished from a missing one.
    match bridge.inner.store.published_docs(&id).await {
        Ok(Some(published)) => return served(&published, request.headers()),
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(%error, "published documentation could not be read");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    let query = request.uri().query();
    let account = match signed_in_or_token(&bridge.inner.store, request.headers(), query).await {
        Ok(account) => account,
        Err(status) => {
            // The path only: a `?token=` that did not sign in must not be
            // copied onto the sign-in address.
            if status == StatusCode::UNAUTHORIZED && browser_navigation(request.headers()) {
                return sign_in_redirect(&bridge.inner.base_path, request.uri().path());
            }
            return (
                status,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                "Sign in or append ?token=cat_… to view this project's documentation.",
            )
                .into_response();
        }
    };

    let storage = bridge.inner.store.scoped(account.id);
    let collection = match storage.collections().get(id.clone()).await {
        Ok(collection) => collection,
        Err(StoreError::NotFound { .. } | StoreError::Forbidden { .. }) => {
            return (StatusCode::NOT_FOUND, "Project not found").into_response();
        }
        Err(error) => {
            tracing::warn!(%error, "project could not be loaded for docs");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let tree = match storage.tree().load(id.clone()).await {
        Ok(tree) => tree,
        Err(error) => {
            tracing::warn!(%error, "project tree could not be loaded for docs");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let environments = storage.variables().environments(id.clone()).await.unwrap_or_default();
    let variables = storage
        .variables()
        .list(id, VariableCategory::Environment)
        .await
        .unwrap_or_default();

    served(
        &PublishedDocs {
            collection,
            tree,
            environments,
            variables,
        },
        request.headers(),
    )
}

fn served(published: &PublishedDocs, headers: &HeaderMap) -> Response {
    // Revision already moves when the tree, requests, or variables change, so
    // it is the validator. The renderer token moves when the HTML shape does,
    // or a deploy would keep answering 304 with the previous page. `private`
    // so a shared cache does not keep the page after the project is made
    // private again.
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let etag = format!("W/\"docs-{}-{}\"", published.collection.revision, html_docs::RENDER);
    if if_none_match
        .as_deref()
        .is_some_and(|header| etag_matches(header, &etag))
    {
        return not_modified(&etag);
    }
    let html = html_docs::render(
        &published.collection,
        &published.tree,
        &published.environments,
        &published.variables,
    );
    page(&etag, html)
}

/// `If-None-Match` lists validators. A weak tag matches the same opaque tag.
fn etag_matches(header: &str, etag: &str) -> bool {
    let want = etag.trim().trim_start_matches("W/").trim();
    header
        .split(',')
        .any(|part| part.trim().trim_start_matches("W/").trim() == want)
}

fn page(etag: &str, body: String) -> Response {
    let mut response = (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "private, no-cache"),
            (header::CONTENT_SECURITY_POLICY, CSP),
        ],
        body,
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

/// A top-level document navigation, as opposed to `fetch` from the page.
/// The page's own requests send `sec-fetch-mode` and stay a plain 401.
/// An address bar without that header is recognized by `Accept: text/html`.
fn browser_navigation(headers: &HeaderMap) -> bool {
    let mode = header_text(headers, "sec-fetch-mode");
    if mode == Some("navigate") || header_text(headers, "sec-fetch-dest") == Some("document") {
        return true;
    }
    if mode.is_some() {
        return false;
    }
    header_text(headers, header::ACCEPT.as_str()).is_some_and(accepts_html)
}

fn header_text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn accepts_html(accept: &str) -> bool {
    accept.split(',').any(|part| {
        let media = part.split(';').next().unwrap_or("").trim();
        media.eq_ignore_ascii_case("text/html") || media.eq_ignore_ascii_case("application/xhtml+xml")
    })
}

/// `{mount}/?next={path}`. `next` is the docs path alone, percent-encoded,
/// so the sign-in page can allow only that one address.
fn sign_in_redirect(base_path: &str, docs_path: &str) -> Response {
    let mut response = Redirect::to(&sign_in_location(base_path, docs_path)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn sign_in_location(base_path: &str, docs_path: &str) -> String {
    format!("{base_path}/?next={}", query_encode(docs_path))
}

fn query_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn not_modified(etag: &str) -> Response {
    let mut response = (StatusCode::NOT_MODIFIED, [(header::CACHE_CONTROL, "private, no-cache")]).into_response();
    if let Ok(value) = HeaderValue::from_str(etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::{browser_navigation, etag_matches, sign_in_location};
    use axum::http::{HeaderMap, HeaderValue, header};

    #[test]
    fn a_weak_validator_matches_with_or_without_the_weak_mark() {
        let etag = "W/\"docs-4-2\"";
        assert!(etag_matches(etag, etag));
        assert!(etag_matches("\"docs-4-2\"", etag));
        assert!(etag_matches("W/\"docs-1-2\", \"docs-4-2\"", etag));
        assert!(!etag_matches("W/\"docs-4-1\"", etag));
    }

    #[test]
    fn a_browser_navigation_is_told_from_a_fetch() {
        let mut headers = HeaderMap::new();
        assert!(!browser_navigation(&headers));
        headers.insert(header::ACCEPT, HeaderValue::from_static("*/*"));
        assert!(!browser_navigation(&headers), "curl's accept is not a page");
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/html,application/xhtml+xml;q=0.9,*/*;q=0.8"),
        );
        assert!(browser_navigation(&headers));
        headers.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
        assert!(!browser_navigation(&headers), "the page's own fetch stays a 401");
        headers.insert("sec-fetch-mode", HeaderValue::from_static("navigate"));
        assert!(browser_navigation(&headers));
    }

    #[test]
    fn the_sign_in_link_names_the_docs_path_under_the_mount() {
        assert_eq!(sign_in_location("", "/v1/docs/proj"), "/?next=%2Fv1%2Fdocs%2Fproj");
        assert_eq!(
            sign_in_location("/tools", "/tools/v1/docs/proj"),
            "/tools/?next=%2Ftools%2Fv1%2Fdocs%2Fproj"
        );
    }
}
