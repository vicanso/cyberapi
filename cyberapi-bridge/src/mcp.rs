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
//! least an editor of the project. What reads like a secret is masked
//! (`cyberapi_core::mcp`), long bodies are cut, an account makes at most
//! [`CALLS_PER_MINUTE`] calls a minute, and every call is a log line.

use crate::Bridge;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use cyberapi_core::EnvironmentId;
use cyberapi_core::access::CollectionRole;
use cyberapi_core::auth::{Auth, KeyPlace};
use cyberapi_core::directory::McpToken;
use cyberapi_core::graphql::GraphQlBody;
use cyberapi_core::grpc::GrpcBody;
use cyberapi_core::inherit::{AuthSource, Inherited, inherited};
use cyberapi_core::mcp::{
    MAX_RESULT_BYTES, MAX_TEXT_CHARS, cut, find_api, listing, masked, masked_body, masked_params,
};
use cyberapi_core::model::{ApiSetting, Collection, ContentType, Environment, HttpMethod, KvParam, VariableCategory};
use cyberapi_core::response::{BodyKind, HttpResponse};
use cyberapi_core::template::{Answers, NoEnv, NoFiles};
use cyberapi_core::version;
use cyberapi_service::Sender;
use cyberapi_store::Storage;
use cyberapi_store_sql::Account;
use serde_json::{Value as Json, json};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The protocol revisions this server answers. A client that asks for one
/// of them is answered in kind; any other gets the newest.
const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const LATEST_PROTOCOL: &str = "2025-06-18";

/// How many tool calls one account may make in a minute: generous for a
/// conversation, mean for a model in a loop.
pub const CALLS_PER_MINUTE: usize = 120;
const WINDOW: Duration = Duration::from_secs(60);
/// The responses `get_history` gives when not told, and the most.
const DEFAULT_HISTORY: u64 = 3;
const MAX_HISTORY: u64 = 10;

const INSTRUCTIONS: &str = "Access to the HTTP API projects this CyberAPI bridge keeps, as the signed-in \
account sees them. Start with list_projects. list_apis gives a project's folders and APIs, each with its \
path and id; the other tools take a project by its name or id and an API by its id, its path or a name \
only one API has. get_api shows an API's saved request, get_variables the project's environments, custom \
variables and global request headers, get_history an API's latest responses. A token allowed to send also \
has send_api, which sends a saved API from the bridge with the project's variables and keeps the response \
in the account's history. Values that read like secrets (passwords, tokens, keys, cookies) are masked as \
***, a value made with a template such as {{TOKEN}} is shown as written, long bodies are cut, and every \
call is logged.";

// JSON-RPC error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

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
    let result = match rpc.method.as_str() {
        "initialize" => Ok(initialize(&rpc.params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_list(&token) })),
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

fn rpc_error(id: Json, code: i64, message: String) -> Json {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// One request, taken apart. `id` is `None` for a notification.
struct Rpc {
    id: Option<Json>,
    method: String,
    params: Json,
}

impl Rpc {
    /// The request, or why it is not one, with the id to answer under: its
    /// own where it named one, else null.
    fn parse(request: Json) -> Result<Self, (Json, i64, String)> {
        let Json::Object(mut fields) = request else {
            let why = if request.is_array() {
                "one request at a time: JSON-RPC batches are not supported"
            } else {
                "a request is a JSON object"
            };
            return Err((Json::Null, INVALID_REQUEST, why.into()));
        };
        let id = fields.remove("id").filter(|id| !id.is_null());
        let Some(Json::String(method)) = fields.remove("method") else {
            let answer_to = id.filter(|id| id.is_string() || id.is_number()).unwrap_or(Json::Null);
            return Err((answer_to, INVALID_REQUEST, "a request names a method".into()));
        };
        let params = fields.remove("params").unwrap_or_else(|| json!({}));
        Ok(Self { id, method, params })
    }
}

fn initialize(params: &Json) -> Json {
    let asked = params.get("protocolVersion").and_then(Json::as_str);
    let protocol = asked
        .filter(|asked| PROTOCOLS.contains(asked))
        .unwrap_or(LATEST_PROTOCOL);
    json!({
        "protocolVersion": protocol,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "cyberapi-bridge", "version": version::VERSION },
        "instructions": format!(
            "{INSTRUCTIONS} Each account may make {CALLS_PER_MINUTE} calls a minute; past that a call answers with an error saying to wait."
        ),
    })
}

/// The `tools/list` reply: every tool this token has, with the schema of
/// its arguments.
fn tool_list(token: &McpToken) -> Vec<Json> {
    let project = json!({ "type": "string", "description": "The project's name or id, as list_projects shows it." });
    let api = json!({ "type": "string", "description": "The API's id, its path (folder/name) or a name only one API has, as list_apis shows them." });
    let read = json!({ "readOnlyHint": true, "openWorldHint": false });
    let mut tools = vec![
        json!({
            "name": "list_projects",
            "description": "The projects this account can see: id, name, description, team, the account's role and how many APIs each has.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": read,
        }),
        json!({
            "name": "list_apis",
            "description": "A project's folders and APIs in display order, each with its path and id, and an API's method and URL.",
            "inputSchema": { "type": "object", "properties": { "project": project }, "required": ["project"] },
            "annotations": read,
        }),
        json!({
            "name": "get_api",
            "description": "An API's saved request: method, URL, query, headers, the headers of the folders it is in, how it signs in (its own or inherited, and from where), body, and the assertions its answers are checked against (subject: status, header, json (a JSONPath), body, latency in ms or size in bytes; op; expected). A URL or value may use the project's variables, written {{NAME}}.",
            "inputSchema": { "type": "object", "properties": { "project": project, "api": api }, "required": ["project", "api"] },
            "annotations": read,
        }),
        json!({
            "name": "get_variables",
            "description": "A project's environment variables ({{NAME}} in a URL), custom variables (value(name)) and global request headers, with whether each is enabled and the named environment it is in (none: every environment's); the project's named environments, and the one this account sends in.",
            "inputSchema": { "type": "object", "properties": { "project": project }, "required": ["project"] },
            "annotations": read,
        }),
        json!({
            "name": "get_history",
            "description": "An API's latest responses in this account's history, newest first: status, time, size, headers and the body, cut.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": project,
                    "api": api,
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_HISTORY, "description": format!("How many, {DEFAULT_HISTORY} when not given.") },
                },
                "required": ["project", "api"],
            },
            "annotations": read,
        }),
    ];
    if token.can_send {
        tools.push(json!({
            "name": "send_api",
            "description": "Sends an API's saved request from the bridge, with the project's variables and global headers, and answers its response, which also goes into this account's history. The request really reaches its server: a POST or DELETE does what it says. Needs an editor's role in the project.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": project,
                    "api": api,
                    "environment": { "type": "string", "description": "The named environment to send in, as get_variables lists them; an empty string for none. When not given, the one this account last chose in the project." },
                },
                "required": ["project", "api"],
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": true, "openWorldHint": true },
        }));
    }
    tools
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
        "list_projects" => list_projects(bridge, account, &storage).await,
        "list_apis" => list_apis(&storage, &arguments).await,
        "get_api" => get_api(&storage, &arguments).await,
        "get_variables" => get_variables(&storage, &arguments).await,
        "get_history" => get_history(&storage, &arguments).await,
        "send_api" if token.can_send => send_api(bridge, account, &storage, &arguments).await,
        "send_api" => Err("this token may not send: issue one that may, or read with the other tools".into()),
        other => return Err((INVALID_PARAMS, format!("unknown tool {other:?}"))),
    };
    tracing::info!(account = %account.name, token = %token.name, tool, ok = answer.is_ok(), "mcp call");
    Ok(finish(answer))
}

/// A tool's answer as the protocol carries it: one text block of JSON, cut
/// to fit a context, or the failure, marked as one.
fn finish(answer: Result<Json, String>) -> Json {
    match answer {
        Ok(value) => {
            let text = serde_json::to_string_pretty(&value).unwrap_or_default();
            let text = if text.len() > MAX_RESULT_BYTES {
                let mut at = MAX_RESULT_BYTES;
                while !text.is_char_boundary(at) {
                    at -= 1;
                }
                format!("{}… [cut: {} bytes in all]", &text[..at], text.len())
            } else {
                text
            };
            json!({ "content": [{ "type": "text", "text": text }], "isError": false })
        }
        Err(why) => json!({ "content": [{ "type": "text", "text": why }], "isError": true }),
    }
}

fn argument<'a>(arguments: &'a Json, name: &str) -> Result<&'a str, String> {
    arguments
        .get(name)
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{name} is missing"))
}

fn failed(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// The project `arguments` name: its id, or a name only one project has.
async fn project(storage: &Storage, arguments: &Json) -> Result<Collection, String> {
    let spec = argument(arguments, "project")?;
    let projects = storage.collections().list().await.map_err(failed)?;
    if let Some(found) = projects.iter().find(|project| project.id.as_str() == spec) {
        return Ok(found.clone());
    }
    let named: Vec<&Collection> = projects.iter().filter(|project| project.name == spec).collect();
    match named.as_slice() {
        [found] => Ok((*found).clone()),
        [] => Err(format!("no project is {spec:?}: list_projects names them")),
        many => Err(format!("{} projects are named {spec:?}: give one's id", many.len())),
    }
}

/// The project and the API `arguments` name.
/// The API an assistant names, where it is, and what it takes from its
/// folders and project (#66).
async fn api(storage: &Storage, arguments: &Json) -> Result<(Collection, ApiSetting, String, Inherited), String> {
    let collection = project(storage, arguments).await?;
    let data = storage.tree().load(collection.id.clone()).await.map_err(failed)?;
    let api = find_api(&data, argument(arguments, "api")?)?.clone();
    let path = listing(&data)
        .into_iter()
        .find(|line| line.id == api.id)
        .map(|line| line.path)
        .unwrap_or_else(|| api.name.clone());
    let inherited = inherited(&data, &api.id);
    Ok((collection, api, path, inherited))
}

async fn list_projects(bridge: &Bridge, account: &Account, storage: &Storage) -> Result<Json, String> {
    let projects = storage.collections().list().await.map_err(failed)?;
    let counts = storage.collections().api_counts().await.map_err(failed)?;
    let teams = bridge
        .inner
        .store
        .directory(account.id.clone())
        .me()
        .await
        .map_err(failed)?
        .teams;
    Ok(Json::Array(
        projects
            .iter()
            .map(|project| {
                let team = project
                    .owner
                    .as_ref()
                    .and_then(|owner| teams.iter().find(|team| &team.id == owner))
                    .map(|team| team.name.clone());
                json!({
                    "id": project.id,
                    "name": project.name,
                    "description": project.description,
                    "team": team,
                    "role": project.role.as_str(),
                    "apis": counts.get(&project.id).copied().unwrap_or_default(),
                })
            })
            .collect(),
    ))
}

async fn list_apis(storage: &Storage, arguments: &Json) -> Result<Json, String> {
    let collection = project(storage, arguments).await?;
    let data = storage.tree().load(collection.id.clone()).await.map_err(failed)?;
    Ok(Json::Array(
        listing(&data)
            .into_iter()
            .map(|line| match line.method {
                None => json!({ "kind": "folder", "path": line.path, "id": line.id }),
                Some(method) => json!({
                    "kind": "api",
                    "path": line.path,
                    "id": line.id,
                    "method": method.as_str(),
                    "url": line.uri,
                }),
            })
            .collect(),
    ))
}

fn params_json(params: &[KvParam]) -> Json {
    Json::Array(
        masked_params(params)
            .into_iter()
            .map(|param| json!({ "name": param.key, "value": param.value, "enabled": param.enabled }))
            .collect(),
    )
}

/// How an API signs in (#66), its secrets masked; null when it does not
/// (none, or inheriting where nothing above sets one).
fn auth_json(auth: &Auth) -> Json {
    match auth {
        Auth::Inherit | Auth::None => Json::Null,
        Auth::Basic { username, password } => {
            json!({ "type": "basic", "username": username, "password": masked("password", password) })
        }
        Auth::Bearer { token } => json!({ "type": "bearer", "token": masked("token", token) }),
        Auth::Digest { username, password } => {
            json!({ "type": "digest", "username": username, "password": masked("password", password) })
        }
        Auth::ApiKey { name, value, place } => json!({
            "type": "apiKey",
            "name": name,
            "in": match place {
                KeyPlace::Header => "header",
                KeyPlace::Query => "query",
            },
            "value": masked("token", value),
        }),
    }
}

async fn get_api(storage: &Storage, arguments: &Json) -> Result<Json, String> {
    let (_, api, path, inherited) = api(storage, arguments).await?;
    let request = &api.request;
    // How it signs in as sent: its own, or what it inherits, and from where.
    let auth = auth_json(inherited.auth(&request.auth));
    let auth_from = match (&auth, request.auth.is_inherit(), &inherited.from) {
        (Json::Null, ..) => Json::Null,
        (_, false, _) => json!("api"),
        (_, true, AuthSource::Folder { name, .. }) => json!(format!("folder {name}")),
        (_, true, AuthSource::Project) => json!("project"),
    };
    let body = match request.content_type {
        // A gRPC call: its method and message; its definitions (base64
        // descriptors) say nothing an assistant reads.
        _ if request.method == HttpMethod::Grpc => {
            let grpc = GrpcBody::read(&request.body);
            json!({
                "service": grpc.service,
                "method": grpc.method,
                "message": cut(&masked_body(&grpc.message), MAX_TEXT_CHARS),
            })
        }
        // A form's fields are kept as the JSON of a list.
        ContentType::Form | ContentType::Multipart => {
            let fields: Vec<KvParam> = serde_json::from_str(&request.body).unwrap_or_default();
            params_json(&fields)
        }
        ContentType::GraphQl => {
            let graphql = GraphQlBody::read(&request.body);
            json!({
                "query": cut(&graphql.query, MAX_TEXT_CHARS),
                "variables": cut(&masked_body(&graphql.variables), MAX_TEXT_CHARS),
                "operationName": graphql.operation_name,
            })
        }
        _ => Json::String(cut(&masked_body(&request.body), MAX_TEXT_CHARS)),
    };
    Ok(json!({
        "id": api.id,
        "name": api.name,
        "path": path,
        "method": request.method.as_str(),
        "url": request.uri,
        "query": params_json(&request.query),
        "headers": params_json(&request.headers),
        "folderHeaders": params_json(&inherited.headers),
        "auth": auth,
        "authFrom": auth_from,
        "contentType": request.content_type.as_str(),
        "body": body,
        // A1: what its answers are checked against.
        "assertions": request.assertions,
    }))
}

/// The named environment a send is in (#61): the one `arguments` name (an
/// empty name is none), or else the one the account last chose in the
/// project.
async fn chosen_environment(
    storage: &Storage,
    collection: &Collection,
    arguments: &Json,
) -> Result<Option<Environment>, String> {
    let environments = storage
        .variables()
        .environments(collection.id.clone())
        .await
        .map_err(failed)?;
    match arguments.get("environment").and_then(Json::as_str).map(str::trim) {
        Some("") => Ok(None),
        Some(name) => environments
            .iter()
            .find(|environment| environment.name.eq_ignore_ascii_case(name))
            .cloned()
            .map(Some)
            .ok_or_else(|| {
                let names: Vec<&str> = environments
                    .iter()
                    .map(|environment| environment.name.as_str())
                    .collect();
                format!("{name:?} is none of the project's environments ({})", names.join(", "))
            }),
        None => {
            let prefs = storage
                .prefs()
                .workspace(collection.id.clone())
                .await
                .unwrap_or_default();
            Ok(prefs
                .environment
                .and_then(|id| environments.into_iter().find(|environment| environment.id == id)))
        }
    }
}

async fn get_variables(storage: &Storage, arguments: &Json) -> Result<Json, String> {
    let collection = project(storage, arguments).await?;
    let environments = storage
        .variables()
        .environments(collection.id.clone())
        .await
        .map_err(failed)?;
    let named = |id: &Option<EnvironmentId>| {
        id.as_ref()
            .and_then(|id| environments.iter().find(|environment| &environment.id == id))
            .map(|environment| environment.name.clone())
    };
    let mut answer = serde_json::Map::new();
    for (key, category) in [
        ("environments", VariableCategory::Environment),
        ("custom", VariableCategory::Custom),
        ("globalHeaders", VariableCategory::GlobalRequestHeader),
    ] {
        let variables = storage
            .variables()
            .list(collection.id.clone(), category)
            .await
            .map_err(failed)?;
        let listed = variables
            .iter()
            .map(|variable| {
                json!({
                    "name": variable.name,
                    "value": masked(&variable.name, &variable.value),
                    "enabled": variable.enabled,
                    "environment": named(&variable.environment),
                })
            })
            .collect();
        answer.insert(key.into(), Json::Array(listed));
    }
    answer.insert(
        "namedEnvironments".into(),
        json!(
            environments
                .iter()
                .map(|environment| environment.name.clone())
                .collect::<Vec<_>>()
        ),
    );
    let selected = chosen_environment(storage, &collection, &json!({})).await?;
    answer.insert("selected".into(), json!(selected.map(|environment| environment.name)));
    Ok(Json::Object(answer))
}

/// A response as a tool shows it: secrets masked, the body cut, a body that
/// is not text described rather than shown.
fn response_json(response: &HttpResponse) -> Json {
    let body = match response.body_omitted {
        Some(size) => format!("[not kept in the history: {size} bytes]"),
        None => match response.body_kind() {
            BodyKind::Json | BodyKind::Text => {
                let text = response.raw_body_text().unwrap_or_default();
                cut(&masked_body(&text), MAX_TEXT_CHARS)
            }
            _ => format!("[{}, {} bytes: not text]", response.media_type(), response.body.len()),
        },
    };
    let headers: Vec<Json> = response
        .headers
        .iter()
        .map(|(name, value)| json!([name, masked(name, value)]))
        .collect();
    json!({
        "id": response.id,
        "at": response.created_at.to_rfc3339(),
        "url": response.url,
        "status": response.status,
        "error": response.error,
        "latencyMs": u64::try_from(response.latency.as_millis()).unwrap_or(u64::MAX),
        "size": response.size(),
        "truncated": response.truncated,
        "headers": headers,
        "body": body,
    })
}

async fn get_history(storage: &Storage, arguments: &Json) -> Result<Json, String> {
    let (collection, api, ..) = api(storage, arguments).await?;
    let limit = arguments
        .get("limit")
        .and_then(Json::as_u64)
        .unwrap_or(DEFAULT_HISTORY)
        .clamp(1, MAX_HISTORY);
    let responses = storage.history().list(collection.id, api.id).await.map_err(failed)?;
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    Ok(Json::Array(responses.iter().take(limit).map(response_json).collect()))
}

async fn send_api(bridge: &Bridge, account: &Account, storage: &Storage, arguments: &Json) -> Result<Json, String> {
    let (collection, api, _, inherited) = api(storage, arguments).await?;
    let environment = chosen_environment(storage, &collection, arguments).await?;
    if collection.role < CollectionRole::Editor {
        return Err(format!(
            "sending needs an editor's role in {:?}; this account views it",
            collection.name
        ));
    }
    let Some(_sending) = bridge.inner.in_flight.start(&account.id) else {
        return Err(format!(
            "this account has {} requests on their way already: wait for them",
            crate::in_flight::PER_ACCOUNT
        ));
    };
    let prefs = storage.prefs().app().await.unwrap_or_default();
    let sender = Sender {
        storage: storage.clone(),
        engine: bridge.inner.engine.clone(),
        files: Arc::new(NoFiles("a request an AI assistant sends cannot read files")),
        // Never the bridge's own: any editor of a project would read the
        // server's secrets.
        env: Arc::new(NoEnv(
            "a request an AI assistant sends cannot read environment variables",
        )),
        user_agent: format!("CyberAPI/{} (mcp)", version::VERSION),
        timeouts: prefs.timeouts,
        sending: prefs.sending,
        inherited,
        environment,
    };
    let response = cyberapi_service::send(sender, collection.id, api.id, api.request, Answers::default())
        .await
        .map_err(failed)?;
    Ok(response_json(&response))
}

#[cfg(test)]
mod tests {
    use super::*;

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
