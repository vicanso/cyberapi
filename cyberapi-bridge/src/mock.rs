//! G2b: shared project mock routes on the bridge (`/v1/mock/{project}` and
//! `/v1/mock/{project}/{*path}`).
//!
//! A signed-in session (cookie or bearer token) or an account's MCP token
//! (`Authorization: Bearer cat_…`, `X-CyberAPI-Token: cat_…`, or
//! `?token=cat_…`) may call any project that account can view, and the
//! request is answered by [`cyberapi_core::mock::Routes`] from the project's
//! saved response examples. Browser CORS preflight (`OPTIONS`) requests are
//! answered directly so frontend applications on another origin can call
//! the mock endpoint with a bearer/header token.

use crate::Bridge;
use crate::auth::signed_in_or_token;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use cyberapi_core::CollectionId;
use cyberapi_core::mock::{Answer, EXAMPLE_HEADER, Routes};
use cyberapi_store::StoreError;
use serde_json::json;
use std::sync::{Arc, Mutex};

const MAX_CACHED_PROJECTS: usize = 64;

#[derive(Default)]
pub struct RouteCache {
    entries: Mutex<Vec<(CollectionId, u64, Arc<Routes>)>>,
}

impl RouteCache {
    pub fn get(&self, project: &CollectionId, revision: u64) -> Option<Arc<Routes>> {
        let mut guard = self.entries.lock().ok()?;
        let pos = guard
            .iter()
            .position(|(id, rev, _)| id == project && *rev == revision)?;
        let entry = guard.remove(pos);
        let routes = entry.2.clone();
        guard.push(entry);
        Some(routes)
    }

    pub fn put(&self, project: CollectionId, revision: u64, routes: Arc<Routes>) {
        let Ok(mut guard) = self.entries.lock() else {
            return;
        };
        guard.retain(|(id, _, _)| id != &project);
        if guard.len() >= MAX_CACHED_PROJECTS {
            guard.remove(0);
        }
        guard.push((project, revision, routes));
    }
}

pub async fn handle_root(State(bridge): State<Bridge>, Path(project): Path<String>, request: Request) -> Response {
    handle(bridge, project, "/".to_string(), request).await
}

pub async fn handle_path(
    State(bridge): State<Bridge>,
    Path((project, rest)): Path<(String, String)>,
    request: Request,
) -> Response {
    let subpath = if rest.starts_with('/') {
        rest
    } else {
        format!("/{rest}")
    };
    handle(bridge, project, subpath, request).await
}

async fn handle(bridge: Bridge, project: String, subpath: String, request: Request) -> Response {
    let method = request.method().as_str().to_string();
    let headers = request.headers();
    let header_str = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    let preflight = header_str("access-control-request-method")
        .is_some()
        .then(|| header_str("access-control-request-headers").unwrap_or_default());

    if method.eq_ignore_ascii_case("OPTIONS") && preflight.is_some() {
        return into_response(Routes::default().answer(&method, &subpath, None, preflight));
    }

    let query = request.uri().query();
    let account = match signed_in_or_token(&bridge.inner.store, headers, query).await {
        Ok(account) => account,
        Err(status) => {
            return cors_json(
                status,
                "sign in or pass an MCP token (Authorization: Bearer cat_… or ?token=cat_…)",
            );
        }
    };

    let project_id = CollectionId::from(project);
    let storage = bridge.inner.store.scoped(account.id);
    let collection = match storage.collections().get(project_id.clone()).await {
        Ok(collection) => collection,
        Err(StoreError::NotFound { .. } | StoreError::Forbidden { .. }) => {
            return cors_json(StatusCode::NOT_FOUND, "project not found");
        }
        Err(error) => {
            tracing::warn!(%error, "a project could not be checked for mock");
            return cors_json(StatusCode::INTERNAL_SERVER_ERROR, "project could not be loaded");
        }
    };

    let routes = match bridge.inner.mock_routes.get(&project_id, collection.revision) {
        Some(cached) => cached,
        None => {
            let tree = match storage.tree().load(project_id.clone()).await {
                Ok(tree) => tree,
                Err(StoreError::NotFound { .. } | StoreError::Forbidden { .. }) => {
                    return cors_json(StatusCode::NOT_FOUND, "project not found");
                }
                Err(error) => {
                    tracing::warn!(%error, "a project's tree could not be loaded for mock");
                    return cors_json(StatusCode::INTERNAL_SERVER_ERROR, "project could not be loaded");
                }
            };
            let built = Arc::new(Routes::of(&tree));
            bridge
                .inner
                .mock_routes
                .put(project_id, collection.revision, built.clone());
            built
        }
    };

    let wanted = header_str(EXAMPLE_HEADER);
    let answered = routes.answer(&method, &subpath, wanted, preflight);
    into_response(answered)
}

fn is_forbidden_mock_header(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "set-cookie"
            | "set-cookie2"
            | "content-security-policy"
            | "content-security-policy-report-only"
            | "x-content-type-options"
            | "x-frame-options"
            | "strict-transport-security"
            | "clear-site-data"
    ) || lower.starts_with("cross-origin-")
}

fn into_response(answer: Answer) -> Response {
    let status = StatusCode::from_u16(answer.status).unwrap_or(StatusCode::OK);
    let mut response = Response::builder().status(status);
    if let Some(headers) = response.headers_mut() {
        for (name, value) in &answer.headers {
            if is_forbidden_mock_header(name) {
                continue;
            }
            if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value)) {
                headers.append(name, value);
            }
        }
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("sandbox; default-src 'none'"),
        );
        headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    response
        .body(Body::from(answer.body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn cors_json(status: StatusCode, message: &str) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            (header::ACCESS_CONTROL_EXPOSE_HEADERS, "*"),
        ],
        json!({ "error": message }).to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_responses_strip_set_cookie_and_enforce_sandbox_csp() {
        let answer = Answer {
            api: None,
            status: 200,
            headers: vec![
                ("Content-Type".into(), "text/html".into()),
                ("Set-Cookie".into(), "cyberapi_session=forged; Path=/".into()),
                ("Content-Security-Policy".into(), "default-src *".into()),
                ("X-Custom".into(), "ok".into()),
            ],
            body: b"<script>alert(1)</script>".to_vec(),
        };
        let response = into_response(answer);
        let headers = response.headers();
        assert!(headers.get(header::SET_COOKIE).is_none());
        assert_eq!(
            headers
                .get(header::CONTENT_SECURITY_POLICY)
                .and_then(|v| v.to_str().ok()),
            Some("sandbox; default-src 'none'")
        );
        assert_eq!(
            headers
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .and_then(|v| v.to_str().ok()),
            Some("nosniff")
        );
        assert_eq!(
            headers.get(header::X_FRAME_OPTIONS).and_then(|v| v.to_str().ok()),
            Some("DENY")
        );
        assert_eq!(headers.get("x-custom").and_then(|v| v.to_str().ok()), Some("ok"));
    }

    #[test]
    fn route_cache_matches_revision_and_invalidates_on_new_revision() {
        let cache = RouteCache::default();
        let pid: CollectionId = "proj-1".into();
        let routes = Arc::new(Routes::default());
        cache.put(pid.clone(), 1, routes.clone());
        assert!(cache.get(&pid, 1).is_some());
        assert!(cache.get(&pid, 2).is_none());
    }
}
