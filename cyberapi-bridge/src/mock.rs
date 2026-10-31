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

    let storage = bridge.inner.store.scoped(account.id);
    let tree = match storage.tree().load(CollectionId::from(project)).await {
        Ok(tree) => tree,
        Err(StoreError::NotFound { .. } | StoreError::Forbidden { .. }) => {
            return cors_json(StatusCode::NOT_FOUND, "project not found");
        }
        Err(error) => {
            tracing::warn!(%error, "a project's tree could not be loaded for mock");
            return cors_json(StatusCode::INTERNAL_SERVER_ERROR, "project could not be loaded");
        }
    };

    let wanted = header_str(EXAMPLE_HEADER);
    let answered = Routes::of(&tree).answer(&method, &subpath, wanted, preflight);
    into_response(answered)
}

fn into_response(answer: Answer) -> Response {
    let status = StatusCode::from_u16(answer.status).unwrap_or(StatusCode::OK);
    let mut response = Response::builder().status(status);
    if let Some(headers) = response.headers_mut() {
        for (name, value) in &answer.headers {
            if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value)) {
                headers.append(name, value);
            }
        }
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
