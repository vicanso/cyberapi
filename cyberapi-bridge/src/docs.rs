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
use cyberapi_store_sql::{PublishedDocs, PublishedFetch};
use std::sync::{Arc, Mutex};

const MAX_CACHED_PAGES: usize = 64;

/// Rendered documentation for one `(project, revision, html_docs::RENDER)`.
/// A new process starts empty, so a renderer change cannot serve the old HTML.
#[derive(Default)]
pub struct HtmlCache {
    entries: Mutex<Vec<(CollectionId, u64, Arc<str>)>>,
}

impl HtmlCache {
    pub fn get(&self, project: &CollectionId, revision: u64) -> Option<Arc<str>> {
        let mut guard = self.entries.lock().ok()?;
        let pos = guard
            .iter()
            .position(|(id, rev, _)| id == project && *rev == revision)?;
        let entry = guard.remove(pos);
        let html = Arc::clone(&entry.2);
        guard.push(entry);
        Some(html)
    }

    pub fn put(&self, project: CollectionId, revision: u64, html: Arc<str>) {
        let Ok(mut guard) = self.entries.lock() else {
            return;
        };
        guard.retain(|(id, _, _)| id != &project);
        if guard.len() >= MAX_CACHED_PAGES {
            guard.remove(0);
        }
        guard.push((project, revision, html));
    }
}

/// Authenticated page: the browser must revalidate, and a shared cache must
/// not keep it. `font-src 'self'` lets the page load the bridge's fonts.
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; font-src 'self'; img-src data:; base-uri 'none'; frame-ancestors 'none'";

pub async fn get(State(bridge): State<Bridge>, Path(project): Path<String>, request: Request) -> Response {
    let id = CollectionId::from(project);
    let headers = request.headers().clone();
    let cache = Arc::clone(&bridge.inner.docs_html);
    // Published first, so a stale `?token=` does not turn a public page
    // into a 401, and a private id is not distinguished from a missing one.
    // The tree is read only when the validator missed and the page is not
    // already rendered for this revision.
    let fetched = bridge
        .inner
        .store
        .published_docs(&id, |collection| {
            page_missing(&cache, &id, collection.revision, &headers)
        })
        .await;
    match fetched {
        Ok(PublishedFetch::Ready(published)) => return serve_loaded(&cache, &id, &published),
        Ok(PublishedFetch::Revision(revision)) => {
            if let Some(response) = kept_answer(&cache, &id, revision, &headers) {
                return response;
            }
            match bridge.inner.store.published_docs(&id, |_| true).await {
                Ok(PublishedFetch::Ready(published)) => return serve_loaded(&cache, &id, &published),
                Ok(PublishedFetch::NotPublished) => {}
                Ok(PublishedFetch::Revision(_)) => {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                Err(error) => {
                    tracing::warn!(%error, "published documentation could not be read");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
        }
        Ok(PublishedFetch::NotPublished) => {}
        Err(error) => {
            tracing::warn!(%error, "published documentation could not be read");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    let query = request.uri().query();
    let account = match signed_in_or_token(&bridge.inner.store, &headers, query).await {
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
    // Signed in, and allowed to see it: the validator can answer before
    // the tree is read. A page that was public and is not any more never
    // reaches here without a session, so its old validator is not a 304.
    if let Some(response) = kept_answer(&cache, &id, collection.revision, &headers) {
        return response;
    }
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
        .list(id.clone(), VariableCategory::Environment)
        .await
        .unwrap_or_default();

    serve_loaded(
        &cache,
        &id,
        &PublishedDocs {
            collection,
            tree,
            environments,
            variables,
        },
    )
}

/// The tree is needed when neither the validator nor a rendered copy of
/// this revision can answer. Checked again after the project is known to
/// be public, or after the caller has signed in and may see it.
fn page_missing(cache: &HtmlCache, id: &CollectionId, revision: u64, headers: &HeaderMap) -> bool {
    kept_answer(cache, id, revision, headers).is_none()
}

/// `304` when the validator matches, or the rendered page when this
/// process still has it. `None` when the tree has to be read.
fn kept_answer(cache: &HtmlCache, id: &CollectionId, revision: u64, headers: &HeaderMap) -> Option<Response> {
    let etag = docs_etag(revision);
    let if_none_match = headers.get(header::IF_NONE_MATCH).and_then(|value| value.to_str().ok());
    if if_none_match.is_some_and(|header| etag_matches(header, &etag)) {
        return Some(not_modified(&etag));
    }
    cache.get(id, revision).map(|html| page(&etag, html.to_string()))
}

fn serve_loaded(cache: &HtmlCache, id: &CollectionId, published: &PublishedDocs) -> Response {
    let etag = docs_etag(published.collection.revision);
    let html = render_page(published);
    cache.put(id.clone(), published.collection.revision, Arc::from(html.as_str()));
    page(&etag, html)
}

fn render_page(published: &PublishedDocs) -> String {
    html_docs::render(
        &published.collection,
        &published.tree,
        &published.environments,
        &published.variables,
    )
}

fn docs_etag(revision: u64) -> String {
    format!("W/\"docs-{revision}-{}\"", html_docs::RENDER)
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
    use super::{HtmlCache, browser_navigation, etag_matches, sign_in_location};
    use axum::http::{HeaderMap, HeaderValue, header};
    use cyberapi_core::CollectionId;
    use std::sync::Arc;

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
    fn a_rendered_page_is_kept_for_its_revision_only() {
        let cache = HtmlCache::default();
        let project = CollectionId::from("proj");
        assert!(cache.get(&project, 3).is_none());
        cache.put(project.clone(), 3, Arc::from("page"));
        assert_eq!(cache.get(&project, 3).as_deref(), Some("page"));
        assert!(cache.get(&project, 4).is_none());
        cache.put(project.clone(), 4, Arc::from("next"));
        assert!(cache.get(&project, 3).is_none(), "one revision of a project is kept");
        assert_eq!(cache.get(&project, 4).as_deref(), Some("next"));
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
