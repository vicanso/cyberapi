//! G3: read-only HTML project documentation served at `GET /v1/docs/{project}`.
//!
//! Accessible with a signed-in session (cookie or bearer token) or an MCP
//! token (`?token=cat_…`, `Authorization: Bearer cat_…`, or
//! `X-CyberAPI-Token: cat_…`) for any project the account can view.

use crate::Bridge;
use crate::auth::signed_in_or_token;
use axum::extract::{Path, Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use cyberapi_core::CollectionId;
use cyberapi_core::html_docs;
use cyberapi_core::model::VariableCategory;
use cyberapi_store::StoreError;

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
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; base-uri 'none'; frame-ancestors 'none'",
            ),
        ],
        html,
    )
        .into_response()
}
