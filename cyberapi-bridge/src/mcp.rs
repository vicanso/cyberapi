//! The MCP entry point, after zedis-bridge's: an AI assistant signs in with
//! one of an account's MCP tokens and reads that account's projects.
//!
//! `POST /v1/mcp` speaks the Model Context Protocol's JSON-RPC over plain
//! HTTP — one request, one JSON reply, no server-initiated stream, no
//! session, no batches — the subset a tools-only server needs. Hand-rolled:
//! it is five methods, and what the tools may do is the point.
//!
//! Only a token signs in here (`Authorization: Bearer cat_…`), and a token
//! signs in nowhere else: the page's cookie and the desktop's session are
//! refused, and a token is no session to the other routes. The tools run on
//! the account's scoped store, so a token sees what its account sees. They
//! read; a token issued with `can_send` also has `send_api`, which sends a
//! saved API from the bridge, as the page does, where the account is at
//! least an editor of the project; one issued with `can_write` (H1) has
//! `create_api` and `update_api` there — nothing deletes. What reads like a
//! secret is masked
//! (`cyberapi_core::mcp`), long bodies are cut, an account makes at most
//! [`CALLS_PER_MINUTE`] calls a minute, and every call is a log line.

use crate::Bridge;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use cyberapi_core::directory::McpToken;
use cyberapi_service::mcp::{
    INSTRUCTIONS, INVALID_PARAMS, METHOD_NOT_FOUND, PARSE_ERROR, Rpc, create_api, execute_send_api, finish, get_api,
    get_history, get_variables, initialize, list_apis, list_projects, prepare_send_api, rpc_error, tool_list,
    update_api,
};
use cyberapi_store::Storage;
use cyberapi_store_sql::Account;
use serde_json::{Value as Json, json};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How many tool calls one account may make in a minute: generous for a
/// conversation, mean for a model in a loop.
pub const CALLS_PER_MINUTE: usize = 120;
const WINDOW: Duration = Duration::from_secs(60);

/// Calls per account in the last minute: a sliding window, swept as it is
/// asked.
#[derive(Default)]
pub struct Limiter(Mutex<HashMap<String, VecDeque<Instant>>>);

impl Limiter {
    /// Whether `account` may make a call now; counted when it may.
    fn allow(&self, account: &str, now: Instant) -> bool {
        let mut calls = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let recent = calls.entry(account.to_string()).or_default();
        while recent.front().is_some_and(|at| now.duration_since(*at) >= WINDOW) {
            recent.pop_front();
        }
        if recent.len() >= CALLS_PER_MINUTE {
            return false;
        }
        recent.push_back(now);
        true
    }
}

/// `GET` and `DELETE`: the stream and the session this server does not
/// have, answered as the protocol allows.
pub async fn not_allowed() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response()
}

/// One JSON-RPC request in, one reply out.
pub async fn post(State(bridge): State<Bridge>, headers: HeaderMap, body: Bytes) -> Response {
    let Some((account, token)) = signed_in(&bridge, &headers).await else {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            axum::Json(json!({ "error": "an MCP token is needed: Authorization: Bearer cat_…" })),
        )
            .into_response();
    };
    let request: Json = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            let error = rpc_error(Json::Null, PARSE_ERROR, format!("the body is not JSON: {error}"));
            return reply(StatusCode::BAD_REQUEST, error);
        }
    };
    let rpc = match Rpc::parse(request) {
        Ok(rpc) => rpc,
        Err((id, code, message)) => return reply(StatusCode::OK, rpc_error(id, code, message)),
    };
    // A notification has no id and gets no reply.
    let Some(id) = rpc.id else {
        return StatusCode::ACCEPTED.into_response();
    };
    let instructions = format!(
        "{INSTRUCTIONS} Each account may make {CALLS_PER_MINUTE} calls a minute; past that a call answers with an error saying to wait."
    );
    let result = match rpc.method.as_str() {
        "initialize" => Ok(initialize(&rpc.params, "cyberapi-bridge", &instructions)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_list(token.can_send, token.can_write) })),
        "tools/call" => call(&bridge, &account, &token, rpc.params).await,
        other => Err((METHOD_NOT_FOUND, format!("unknown method {other:?}"))),
    };
    reply(
        StatusCode::OK,
        match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => rpc_error(id, code, message),
        },
    )
}

/// The account and the token an `Authorization: Bearer …` header signs in
/// as, while the token lasts and the account is enabled.
async fn signed_in(bridge: &Bridge, headers: &HeaderMap) -> Option<(Account, McpToken)> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?
        .trim();
    let hash = crate::auth::hash_token(token);
    match bridge
        .inner
        .store
        .mcp_token_account(&hash, crate::auth::unix_now())
        .await
    {
        Ok(found) => found,
        Err(error) => {
            tracing::warn!(%error, "an MCP token could not be looked up");
            None
        }
    }
}

fn reply(status: StatusCode, body: Json) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

/// `tools/call`: a protocol error for a call that is not one, otherwise
/// the tool's answer — its failure too, which the model reads.
async fn call(bridge: &Bridge, account: &Account, token: &McpToken, params: Json) -> Result<Json, (i64, String)> {
    let Some(tool) = params.get("name").and_then(Json::as_str).map(str::to_string) else {
        return Err((INVALID_PARAMS, "tools/call names a tool".into()));
    };
    let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    if !bridge.inner.mcp_calls.allow(account.id.as_str(), Instant::now()) {
        let why = format!("more than {CALLS_PER_MINUTE} calls in a minute: wait a little and try again");
        return Ok(finish(Err(why)));
    }
    let storage = bridge.inner.store.scoped(account.id.clone());
    let answer = match tool.as_str() {
        "list_projects" => {
            let teams = bridge
                .inner
                .store
                .directory(account.id.clone())
                .me()
                .await
                .map_err(|error| error.to_string())
                .map(|me| me.teams);
            match teams {
                Ok(teams) => list_projects(&storage, &teams).await,
                Err(why) => Err(why),
            }
        }
        "list_apis" => list_apis(&storage, &arguments).await,
        "get_api" => get_api(&storage, &arguments).await,
        "get_variables" => get_variables(&storage, &arguments).await,
        "get_history" => get_history(&storage, &arguments).await,
        "send_api" if token.can_send => send_api(bridge, account, &storage, &arguments).await,
        "send_api" => Err("this token may not send: issue one that may, or read with the other tools".into()),
        "create_api" if token.can_write => create_api(&storage, &arguments).await,
        "update_api" if token.can_write => update_api(&storage, &arguments).await,
        "create_api" | "update_api" => {
            Err("this token may not change projects: issue one that may, or read with the other tools".into())
        }
        other => return Err((INVALID_PARAMS, format!("unknown tool {other:?}"))),
    };
    tracing::info!(account = %account.name, token = %token.name, tool, ok = answer.is_ok(), "mcp call");
    Ok(finish(answer))
}

async fn send_api(bridge: &Bridge, account: &Account, storage: &Storage, arguments: &Json) -> Result<Json, String> {
    let prepared = prepare_send_api(storage, arguments).await?;
    let _sending = match bridge.inner.in_flight.start(&account.id) {
        Ok(sending) => sending,
        Err(busy) => return Err(busy.to_string()),
    };
    execute_send_api(storage, bridge.inner.engine.clone(), prepared).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::mcp::MAX_RESULT_BYTES;
    use cyberapi_service::mcp::INVALID_REQUEST;

    #[test]
    fn an_account_makes_so_many_calls_a_minute() {
        let limiter = Limiter::default();
        let start = Instant::now();
        for _ in 0..CALLS_PER_MINUTE {
            assert!(limiter.allow("a", start));
        }
        assert!(!limiter.allow("a", start), "one too many");
        assert!(limiter.allow("b", start), "each account its own");
        assert!(limiter.allow("a", start + WINDOW), "a minute on");
    }

    #[test]
    fn a_request_is_taken_apart_or_refused_with_its_id() {
        let rpc = Rpc::parse(json!({"jsonrpc": "2.0", "id": 7, "method": "ping"})).expect("a request");
        assert_eq!((rpc.id, rpc.method.as_str()), (Some(json!(7)), "ping"));
        let notification =
            Rpc::parse(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).expect("a notification");
        assert_eq!(notification.id, None);
        assert!(matches!(Rpc::parse(json!([1])), Err((Json::Null, INVALID_REQUEST, _))));
        assert!(matches!(Rpc::parse(json!({"id": "x"})), Err((id, INVALID_REQUEST, _)) if id == json!("x")));
    }

    #[test]
    fn a_long_answer_is_cut_and_a_failure_is_marked() {
        let long = finish(Ok(json!("x".repeat(MAX_RESULT_BYTES * 2))));
        let text = long["content"][0]["text"].as_str().unwrap_or_default();
        assert!(text.len() < MAX_RESULT_BYTES + 100 && text.contains("[cut:"));
        assert_eq!(finish(Err("no".into()))["isError"], json!(true));
    }
}
