//! G3: read-only HTML project documentation served at `GET /v1/docs/{project}`.
//!
//! Accessible with a signed-in session (cookie or bearer token) or an MCP
//! token (`?token=cat_…`, `Authorization: Bearer cat_…`, or
//! `X-CyberAPI-Token: cat_…`) for any project the account can view.

use crate::Bridge;
use crate::auth::signed_in_or_token;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use cyberapi_core::CollectionId;
use cyberapi_core::html_docs;
use cyberapi_core::model::VariableCategory;
use cyberapi_store::StoreError;

/// Authenticated page: the browser must revalidate, and a shared cache must
/// not keep it. `font-src 'self'` lets the page load the bridge's fonts.
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; font-src 'self'; img-src data:; base-uri 'none'; frame-ancestors 'none'";

pub async fn get(State(bridge): State<Bridge>, Path(project): Path<String>, request: Request) -> Response {
    let query = request.uri().query();
    let account = match signed_in_or_token(&bridge.inner.store, request.headers(), query).await {
        Ok(account) => account,
        Err(status) => {
            return (
                status,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                "Sign in or append ?token=cat_… to view this project's documentation.",
            )
                .into_response();
        }
    };

    let if_none_match = request
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let id = CollectionId::from(project);
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
    // Revision already moves when the tree, requests, or variables change, so
    // it is the validator. The renderer token moves when the HTML shape does,
    // or a deploy would keep answering 304 with the previous page.
    let etag = format!("W/\"docs-{}-{}\"", collection.revision, html_docs::RENDER);
    if if_none_match
        .as_deref()
        .is_some_and(|header| etag_matches(header, &etag))
    {
        return not_modified(&etag);
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
        .list(id, VariableCategory::Environment)
        .await
        .unwrap_or_default();

    let html = html_docs::render(&collection, &tree, &environments, &variables);
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

fn not_modified(etag: &str) -> Response {
    let mut response = (StatusCode::NOT_MODIFIED, [(header::CACHE_CONTROL, "private, no-cache")]).into_response();
    if let Ok(value) = HeaderValue::from_str(etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::etag_matches;

    #[test]
    fn a_weak_validator_matches_with_or_without_the_weak_mark() {
        let etag = "W/\"docs-4-2\"";
        assert!(etag_matches(etag, etag));
        assert!(etag_matches("\"docs-4-2\"", etag));
        assert!(etag_matches("W/\"docs-1-2\", \"docs-4-2\"", etag));
        assert!(!etag_matches("W/\"docs-4-1\"", etag));
    }
}
