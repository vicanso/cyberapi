//! The Model Context Protocol (MCP) JSON-RPC tools shared by `cyberapi-bridge`
//! (`/v1/mcp`) and the desktop's local loopback MCP server (H2).
//!
//! Speaks MCP's JSON-RPC 2.0 over plain HTTP (`initialize`, `ping`,
//! `tools/list`, `tools/call`). Secrets in headers, parameters, variables,
//! auth and bodies are masked (`cyberapi_core::mcp`), and long results are
//! cut to fit a model's context window.

use crate::Sender;
use cyberapi_core::EnvironmentId;
use cyberapi_core::access::CollectionRole;
use cyberapi_core::auth::{Auth, KeyPlace};
use cyberapi_core::directory::Team;
use cyberapi_core::graphql::GraphQlBody;
use cyberapi_core::grpc::GrpcBody;
use cyberapi_core::inherit::{AuthSource, Inherited, inherited};
use cyberapi_core::mcp::{
    MAX_RESULT_BYTES, MAX_TEXT_CHARS, cut, find_api, listing, masked, masked_body, masked_params,
};
use cyberapi_core::model::{
    ApiSetting, Collection, ContentType, Environment, HttpMethod, KvParam, ProjectSettings, VariableCategory,
};
use cyberapi_core::oauth::Grant;
use cyberapi_core::response::{BodyKind, HttpResponse};
use cyberapi_core::template::{Answers, NoEnv, NoFiles};
use cyberapi_core::version;
use cyberapi_http::HttpEngine;
use cyberapi_store::Storage;
use serde_json::{Value as Json, json};
use std::sync::Arc;

/// The protocol revisions this server answers. A client that asks for one
/// of them is answered in kind; any other gets the newest.
const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const LATEST_PROTOCOL: &str = "2025-06-18";

/// The responses `get_history` gives when not told, and the most.
const DEFAULT_HISTORY: u64 = 3;
const MAX_HISTORY: u64 = 10;

pub const INSTRUCTIONS: &str = "Access to the HTTP API projects this CyberAPI instance keeps. \
Start with list_projects. list_apis gives a project's folders and APIs, each with its \
path and id; the other tools take a project by its name or id and an API by its id, its path or a name \
only one API has. get_api shows an API's saved request, get_variables the project's environments, custom \
variables and global request headers, get_history an API's latest responses. A token allowed to send also \
has send_api, which sends a saved API with the project's variables and keeps the response \
in the history. When the API has assertions enabled, that answer also lists each one's result \
(whether it held, and when it did not, the value that was there); those results are not kept in the history. \
A token allowed to change also has create_api and update_api, which make an API and \
change one's request where the reader edits; nothing deletes. Values that read like secrets (passwords, tokens, keys, cookies) are masked as \
***, a value made with a template such as {{TOKEN}} is shown as written, long bodies are cut, and every \
call is logged.";

// JSON-RPC error codes.
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

/// Generates a random local MCP bearer token (`cat_local_<32 hex chars>`).
pub fn generate_local_token() -> String {
    let mut bytes = [0u8; 16];
    let _ = getrandom::fill(&mut bytes);
    let mut hex = String::with_capacity(42);
    hex.push_str("cat_local_");
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

pub fn rpc_error(id: Json, code: i64, message: String) -> Json {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// One request, taken apart. `id` is `None` for a notification.
pub struct Rpc {
    pub id: Option<Json>,
    pub method: String,
    pub params: Json,
}

impl Rpc {
    /// The request, or why it is not one, with the id to answer under: its
    /// own where it named one, else null.
    pub fn parse(request: Json) -> Result<Self, (Json, i64, String)> {
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

pub fn initialize(params: &Json, server_name: &str, instructions: &str) -> Json {
    let asked = params.get("protocolVersion").and_then(Json::as_str);
    let protocol = asked
        .filter(|asked| PROTOCOLS.contains(asked))
        .unwrap_or(LATEST_PROTOCOL);
    json!({
        "protocolVersion": protocol,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": server_name, "version": version::VERSION },
        "instructions": instructions,
    })
}

/// The `tools/list` reply: every tool this token has, with the schema of
/// its arguments.
pub fn tool_list(can_send: bool, can_write: bool) -> Vec<Json> {
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
            "description": "An API's saved request: method, URL, query, headers, the headers of the folders it is in, how it signs in (its own or inherited, and from where), body, the assertions its answers are checked against (subject: status, header, json (a JSONPath), body, latency in ms or size in bytes; op; expected), what it is for (description) and answers kept as examples of what it returns. A URL or value may use the project's variables, written {{NAME}}.",
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
    if can_send {
        tools.push(json!({
            "name": "send_api",
            "description": "Sends an API's saved request, with the project's variables and global headers, and answers its response, which also goes into the history. When the API has assertions enabled, the answer also lists each one's result: passed, and when it did not hold the actual value; a check that could not be made says why. Those results are not kept in the history. The request really reaches its server: a POST or DELETE does what it says. Needs an editor's role in the project.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": project,
                    "api": api,
                    "environment": { "type": "string", "description": "The named environment to send in, as get_variables lists them; an empty string for none. When not given, the one last chosen in the project." },
                },
                "required": ["project", "api"],
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": true, "openWorldHint": true },
        }));
    }
    // H1: making and changing, never deleting.
    if can_write {
        let written = json!({
            "method": { "type": "string", "description": "GET, POST, PUT, PATCH, DELETE, HEAD or OPTIONS." },
            "url": { "type": "string", "description": "The URL; may start with an environment variable, {{HOST}}/users. Its query goes to the query list." },
            "query": { "description": "The query, replacing the list: an object of names and values, or a list of {name, value, enabled}." },
            "headers": { "description": "The headers, replacing the list, as query is given." },
            "contentType": { "type": "string", "enum": ["json", "form", "multipart", "xml", "text", "graphql"] },
            "body": { "description": "The body as text; a form's may be an object of fields, a JSON one any JSON." },
            "requestSchema": { "description": "A JSON Schema object (or its JSON text) describing the request body's fields; an empty string clears it." },
            "description": { "type": "string", "description": "What the API is for, as plain text." },
        });
        let mut create = written.clone();
        if let Some(properties) = create.as_object_mut() {
            properties.insert("project".into(), project.clone());
            properties.insert(
                "name".into(),
                json!({ "type": "string", "description": "The new API's name." }),
            );
            properties.insert("folder".into(), json!({ "type": "string", "description": "The folder to put it in, by its path (users/admin) or id, as list_apis names them; the top level when not given." }));
        }
        let mut update = written;
        if let Some(properties) = update.as_object_mut() {
            properties.insert("project".into(), project.clone());
            properties.insert("api".into(), api.clone());
            properties.insert(
                "name".into(),
                json!({ "type": "string", "description": "A new name for it." }),
            );
        }
        tools.push(json!({
            "name": "create_api",
            "description": "Makes an API in a project — in a folder, or at the top level — with the request given; what is not given is left empty (a GET). Answers its id and path. Needs an editor's role in the project.",
            "inputSchema": { "type": "object", "properties": create, "required": ["project", "name"] },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false },
        }));
        tools.push(json!({
            "name": "update_api",
            "description": "Changes an API's saved request: what is given replaces what it had, what is not stays; a new name renames it. Answers it as get_api does. Needs an editor's role in the project.",
            "inputSchema": { "type": "object", "properties": update, "required": ["project", "api"] },
            "annotations": { "readOnlyHint": false, "destructiveHint": true, "openWorldHint": false },
        }));
    }
    tools
}

/// A tool's answer as the protocol carries it: one text block of JSON, cut
/// to fit a context, or the failure, marked as one.
pub fn finish(answer: Result<Json, String>) -> Json {
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
pub async fn project(storage: &Storage, arguments: &Json) -> Result<Collection, String> {
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

/// The API `arguments` name: its project, itself, its path in the tree,
/// what it takes from above, and the project's secret variables (D3).
pub async fn api(
    storage: &Storage,
    arguments: &Json,
) -> Result<(Collection, ApiSetting, String, Inherited, ProjectSettings), String> {
    let collection = project(storage, arguments).await?;
    let data = storage.tree().load(collection.id.clone()).await.map_err(failed)?;
    let api = find_api(&data, argument(arguments, "api")?)?.clone();
    let path = listing(&data)
        .into_iter()
        .find(|line| line.id == api.id)
        .map(|line| line.path)
        .unwrap_or_else(|| api.name.clone());
    let inherited = inherited(&data, &api.id);
    Ok((collection, api, path, inherited, data.settings))
}

/// H1: an API made where `arguments` say, with the request they give.
pub async fn create_api(storage: &Storage, arguments: &Json) -> Result<Json, String> {
    let collection = project(storage, arguments).await?;
    if collection.role < CollectionRole::Editor {
        return Err(format!(
            "changing APIs needs an editor's role in {:?}; this account views it",
            collection.name
        ));
    }
    let name = argument(arguments, "name")?.to_string();
    let request = cyberapi_core::mcp::written(&Default::default(), arguments)?;
    let data = storage.tree().load(collection.id.clone()).await.map_err(failed)?;
    let folder = match arguments
        .get("folder")
        .and_then(Json::as_str)
        .filter(|spec| !spec.trim().is_empty())
    {
        Some(spec) => Some(cyberapi_core::mcp::find_folder(&data, spec)?),
        None => None,
    };
    let made = storage
        .tree()
        .create_api(collection.id.clone(), folder, name)
        .await
        .map_err(failed)?;
    let id = made.item.id.clone();
    storage
        .tree()
        .save_request(collection.id.clone(), id.clone(), request)
        .await
        .map_err(failed)?;
    let mut arguments = arguments.clone();
    arguments["api"] = json!(id.as_str());
    arguments["project"] = json!(collection.id.as_str());
    get_api(storage, &arguments).await
}

/// H1: an API's request with what `arguments` give in place, renamed when
/// they name it anew.
pub async fn update_api(storage: &Storage, arguments: &Json) -> Result<Json, String> {
    let (collection, api, ..) = self::api(storage, arguments).await?;
    if collection.role < CollectionRole::Editor {
        return Err(format!(
            "changing APIs needs an editor's role in {:?}; this account views it",
            collection.name
        ));
    }
    let request = cyberapi_core::mcp::written(&api.request, arguments)?;
    if let Some(name) = arguments.get("name").and_then(Json::as_str).map(str::trim)
        && !name.is_empty()
        && name != api.name
    {
        storage
            .tree()
            .rename(collection.id.clone(), api.id.clone(), name.to_string())
            .await
            .map_err(failed)?;
    }
    if request != api.request {
        storage
            .tree()
            .save_request(collection.id.clone(), api.id.clone(), request)
            .await
            .map_err(failed)?;
    }
    let mut arguments = arguments.clone();
    arguments["api"] = json!(api.id.as_str());
    arguments["project"] = json!(collection.id.as_str());
    get_api(storage, &arguments).await
}

pub async fn list_projects(storage: &Storage, teams: &[Team]) -> Result<Json, String> {
    let projects = storage.collections().list().await.map_err(failed)?;
    let counts = storage.collections().api_counts().await.map_err(failed)?;
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

pub async fn list_apis(storage: &Storage, arguments: &Json) -> Result<Json, String> {
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
            .map(|param| {
                let mut out = json!({ "name": param.key, "value": param.value, "enabled": param.enabled });
                if !param.description.is_empty() {
                    out["description"] = json!(param.description);
                }
                out
            })
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
        // C4: where its token is had, the secrets masked.
        Auth::OAuth2(oauth) => json!({
            "type": "oauth2",
            "grant": match oauth.grant {
                Grant::ClientCredentials => "client_credentials",
                Grant::Password => "password",
                Grant::AuthorizationCode => "authorization_code",
            },
            "tokenUrl": oauth.token_url,
            "authorizationUrl": oauth.authorization_url,
            "clientId": oauth.client_id,
            "clientSecret": masked("secret", &oauth.client_secret),
            "username": oauth.username,
            "password": masked("password", &oauth.password),
            "scope": oauth.scope,
        }),
    }
}

pub async fn get_api(storage: &Storage, arguments: &Json) -> Result<Json, String> {
    let (_, api, path, inherited, _) = api(storage, arguments).await?;
    let request = &api.request;
    let auth = auth_json(inherited.auth(&request.auth));
    let auth_from = match (&auth, request.auth.is_inherit(), &inherited.from) {
        (Json::Null, ..) => Json::Null,
        (_, false, _) => json!("api"),
        (_, true, AuthSource::Folder { name, .. }) => json!(format!("folder {name}")),
        (_, true, AuthSource::Project) => json!("project"),
    };
    let body = match request.content_type {
        _ if request.method == HttpMethod::Grpc => {
            let grpc = GrpcBody::read(&request.body);
            json!({
                "service": grpc.service,
                "method": grpc.method,
                "message": cut(&masked_body(&grpc.message), MAX_TEXT_CHARS),
            })
        }
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
        "variables": params_json(&request.variables),
        "folderVariables": params_json(&inherited.variables),
        "auth": auth,
        "authFrom": auth_from,
        "contentType": request.content_type.as_str(),
        "body": body,
        "requestSchema": request.request_schema.as_deref().map(|schema| cut(schema, MAX_TEXT_CHARS)),
        "assertions": request.assertions,
        "description": request.description,
        "examples": request
            .examples
            .iter()
            .map(|example| json!({
                "name": example.name,
                "status": example.status,
                "body": cut(&masked_body(&example.body), MAX_TEXT_CHARS),
            }))
            .collect::<Vec<_>>(),
    }))
}

/// The named environment a send is in (#61): the one `arguments` name (an
/// empty name is none), or else the one the account last chose in the
/// project.
pub async fn chosen_environment(
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

pub async fn get_variables(storage: &Storage, arguments: &Json) -> Result<Json, String> {
    let collection = project(storage, arguments).await?;
    let secret = storage
        .tree()
        .load(collection.id.clone())
        .await
        .map_err(failed)?
        .settings
        .secret_variables;
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
            .map(|variable| match secret.contains(&variable.id) {
                true => json!({
                    "name": variable.name,
                    "secret": true,
                    "enabled": variable.enabled,
                    "environment": named(&variable.environment),
                }),
                false => json!({
                    "name": variable.name,
                    "value": masked(&variable.name, &variable.value),
                    "enabled": variable.enabled,
                    "environment": named(&variable.environment),
                }),
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
pub fn response_json(response: &HttpResponse) -> Json {
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

pub async fn get_history(storage: &Storage, arguments: &Json) -> Result<Json, String> {
    let (collection, api, ..) = api(storage, arguments).await?;
    let limit = arguments
        .get("limit")
        .and_then(Json::as_u64)
        .unwrap_or(DEFAULT_HISTORY)
        .clamp(1, MAX_HISTORY);
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut responses = storage
        .history()
        .summaries(collection.id.clone(), api.id.clone())
        .await
        .map_err(failed)?;
    responses.truncate(limit);
    if let Some(first) = responses.first_mut()
        && !first.id.is_empty()
        && let Ok(full) = storage.history().get(collection.id, api.id, first.id.clone()).await
    {
        *first = full;
    }
    Ok(Json::Array(responses.iter().map(response_json).collect()))
}

pub struct PreparedSend {
    pub collection: Collection,
    pub api: ApiSetting,
    pub inherited: Inherited,
    pub settings: ProjectSettings,
    pub environment: Option<Environment>,
}

/// Validates and loads everything `send_api` needs before acquiring any
/// in-flight slot.
pub async fn prepare_send_api(storage: &Storage, arguments: &Json) -> Result<PreparedSend, String> {
    let (collection, api, _, inherited, settings) = api(storage, arguments).await?;
    let environment = chosen_environment(storage, &collection, arguments).await?;
    if collection.role < CollectionRole::Editor {
        return Err(format!(
            "sending needs an editor's role in {:?}; this account views it",
            collection.name
        ));
    }
    // D6: in production, what may change something waits for a person to
    // say it goes; an assistant cannot.
    if let Some(environment) = &environment
        && settings.protects(Some(&environment.id))
        && cyberapi_core::environments::asks_first(&api.request)
    {
        return Err(format!(
            "{:?} is marked as production, and a {} request may change something there: send it from the app, \
             or choose another environment",
            environment.name,
            api.request.method.as_str()
        ));
    }
    Ok(PreparedSend {
        collection,
        api,
        inherited,
        settings,
        environment,
    })
}

/// Sends a prepared `send_api` call and formats its response and assertion
/// report.
pub async fn execute_send_api(
    storage: &Storage,
    engine: Arc<dyn HttpEngine>,
    prepared: PreparedSend,
) -> Result<Json, String> {
    let PreparedSend {
        collection,
        api,
        inherited,
        settings,
        environment,
    } = prepared;
    let prefs = storage.prefs().app().await.unwrap_or_default();
    let sender = Sender {
        storage: storage.clone(),
        engine,
        files: Arc::new(NoFiles("a request an AI assistant sends cannot read files")),
        env: Arc::new(NoEnv(
            "a request an AI assistant sends cannot read environment variables",
        )),
        user_agent: format!("CyberAPI/{} (mcp)", version::VERSION),
        timeouts: prefs.timeouts,
        sending: cyberapi_core::prefs::SendPrefs {
            certificates: Default::default(),
            ..prefs.sending
        },
        inherited,
        environment,
        tokens: Default::default(),
        secrets: storage
            .prefs()
            .secret_values(collection.id.clone())
            .await
            .unwrap_or_default()
            .of(&settings.secret_variables),
    };
    let assertions = api.request.assertions.clone();
    let response = crate::send(sender, collection.id, api.id, api.request, Answers::default())
        .await
        .map_err(failed)?;
    let mut answer = response_json(&response);
    let reported = cyberapi_core::assertions::report(&assertions, &response);
    if !reported.is_empty()
        && let Some(object) = answer.as_object_mut()
    {
        object.insert("assertions".into(), json!(reported));
    }
    Ok(answer)
}

/// H2 / Q-2: desktop-specific context for the local loopback MCP server so it
/// hides copies belonging to other/signed-out accounts, applies known server
/// roles, refuses `create_api` / `update_api` on server-synced project copies,
/// and notifies open workspaces when a local project is modified.
#[derive(Clone, Default)]
pub struct LocalMcpContext {
    pub copies: Option<Arc<dyn cyberapi_store::ServerCopyStore>>,
    pub account: Arc<std::sync::RwLock<Option<cyberapi_core::AccountId>>>,
    pub roles: Arc<std::sync::RwLock<std::collections::HashMap<cyberapi_core::CollectionId, CollectionRole>>>,
    pub on_write: Option<Arc<dyn Fn(cyberapi_core::CollectionId) + Send + Sync>>,
}

async fn visible_local_projects(
    storage: &Storage,
    context: &LocalMcpContext,
) -> Result<(Vec<Collection>, std::collections::HashSet<cyberapi_core::CollectionId>), String> {
    let projects = storage.collections().list().await.map_err(failed)?;
    let mut bound_ids = std::collections::HashSet::new();
    let Some(copies) = &context.copies else {
        return Ok((projects, bound_ids));
    };
    let bindings = copies.copies().await.unwrap_or_default();
    let current_account = context.account.read().ok().and_then(|g| g.clone());
    let roles = context.roles.read().ok().map(|g| g.clone()).unwrap_or_default();
    let mut visible = Vec::with_capacity(projects.len());
    for mut project in projects {
        if let Some(binding) = bindings.iter().find(|b| b.collection == project.id) {
            if current_account.as_ref() != Some(&binding.account) {
                continue;
            }
            bound_ids.insert(project.id.clone());
            if let Some(&role) = roles.get(&project.id) {
                project.role = role;
            }
        }
        visible.push(project);
    }
    Ok((visible, bound_ids))
}

async fn resolve_local_project(
    storage: &Storage,
    context: &LocalMcpContext,
    arguments: &Json,
) -> Result<(Collection, bool), String> {
    let spec = argument(arguments, "project")?;
    let (projects, bound_ids) = visible_local_projects(storage, context).await?;
    if let Some(found) = projects.iter().find(|project| project.id.as_str() == spec) {
        let is_copy = bound_ids.contains(&found.id);
        return Ok((found.clone(), is_copy));
    }
    let named: Vec<&Collection> = projects.iter().filter(|project| project.name == spec).collect();
    match named.as_slice() {
        [found] => {
            let is_copy = bound_ids.contains(&found.id);
            Ok(((*found).clone(), is_copy))
        }
        [] => Err(format!("no project is {spec:?}: list_projects names them")),
        many => Err(format!("{} projects are named {spec:?}: give one's id", many.len())),
    }
}

/// H2: handles one JSON-RPC request for the desktop's local loopback MCP
/// server (`127.0.0.1`), returning the HTTP status code and optional JSON
/// body (`None` for a JSON-RPC notification).
pub async fn handle_local(
    storage: &Storage,
    engine: &Arc<dyn HttpEngine>,
    can_send: bool,
    can_write: bool,
    body: &[u8],
) -> (u16, Option<String>) {
    handle_local_with_context(storage, engine, &LocalMcpContext::default(), can_send, can_write, body).await
}

pub async fn handle_local_with_context(
    storage: &Storage,
    engine: &Arc<dyn HttpEngine>,
    context: &LocalMcpContext,
    can_send: bool,
    can_write: bool,
    body: &[u8],
) -> (u16, Option<String>) {
    let request: Json = match serde_json::from_slice(body) {
        Ok(request) => request,
        Err(error) => {
            let error = rpc_error(Json::Null, PARSE_ERROR, format!("the body is not JSON: {error}"));
            return (400, Some(error.to_string()));
        }
    };
    let rpc = match Rpc::parse(request) {
        Ok(rpc) => rpc,
        Err((id, code, message)) => return (200, Some(rpc_error(id, code, message).to_string())),
    };
    let Some(id) = rpc.id else {
        return (202, None);
    };
    let result = match rpc.method.as_str() {
        "initialize" => Ok(initialize(&rpc.params, "cyberapi-desktop", INSTRUCTIONS)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_list(can_send, can_write) })),
        "tools/call" => call_local(storage, engine, context, can_send, can_write, rpc.params).await,
        other => Err((METHOD_NOT_FOUND, format!("unknown method {other:?}"))),
    };
    let reply = match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => rpc_error(id, code, message),
    };
    (200, Some(reply.to_string()))
}

async fn call_local(
    storage: &Storage,
    engine: &Arc<dyn HttpEngine>,
    context: &LocalMcpContext,
    can_send: bool,
    can_write: bool,
    params: Json,
) -> Result<Json, (i64, String)> {
    let Some(tool) = params.get("name").and_then(Json::as_str).map(str::to_string) else {
        return Err((INVALID_PARAMS, "tools/call names a tool".into()));
    };
    let mut arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let answer = match tool.as_str() {
        "list_projects" => match visible_local_projects(storage, context).await {
            Ok((projects, _)) => match storage.collections().api_counts().await.map_err(failed) {
                Ok(counts) => Ok(Json::Array(
                    projects
                        .iter()
                        .map(|project| {
                            json!({
                                "id": project.id,
                                "name": project.name,
                                "description": project.description,
                                "team": Json::Null,
                                "role": project.role.as_str(),
                                "apis": counts.get(&project.id).copied().unwrap_or_default(),
                            })
                        })
                        .collect(),
                )),
                Err(why) => Err(why),
            },
            Err(why) => Err(why),
        },
        "list_apis" => match resolve_local_project(storage, context, &arguments).await {
            Ok((collection, _)) => {
                arguments["project"] = json!(collection.id.as_str());
                list_apis(storage, &arguments).await
            }
            Err(why) => Err(why),
        },
        "get_api" => match resolve_local_project(storage, context, &arguments).await {
            Ok((collection, _)) => {
                arguments["project"] = json!(collection.id.as_str());
                get_api(storage, &arguments).await
            }
            Err(why) => Err(why),
        },
        "get_variables" => match resolve_local_project(storage, context, &arguments).await {
            Ok((collection, _)) => {
                arguments["project"] = json!(collection.id.as_str());
                get_variables(storage, &arguments).await
            }
            Err(why) => Err(why),
        },
        "get_history" => match resolve_local_project(storage, context, &arguments).await {
            Ok((collection, _)) => {
                arguments["project"] = json!(collection.id.as_str());
                get_history(storage, &arguments).await
            }
            Err(why) => Err(why),
        },
        "send_api" if can_send => match resolve_local_project(storage, context, &arguments).await {
            Ok((collection, _)) => {
                if collection.role < CollectionRole::Editor {
                    Err(format!(
                        "sending needs an editor's role in {:?}; this account views it",
                        collection.name
                    ))
                } else {
                    arguments["project"] = json!(collection.id.as_str());
                    match prepare_send_api(storage, &arguments).await {
                        Ok(prepared) => execute_send_api(storage, engine.clone(), prepared).await,
                        Err(why) => Err(why),
                    }
                }
            }
            Err(why) => Err(why),
        },
        "send_api" => Err(
            "this local MCP server is not allowed to send requests: enable 'Allow sending requests' in CyberAPI"
                .into(),
        ),
        "create_api" if can_write => match resolve_local_project(storage, context, &arguments).await {
            Ok((collection, true)) => Err(format!(
                "{:?} is a server-synced project: edit it in the CyberAPI app or use the bridge's MCP endpoint",
                collection.name
            )),
            Ok((collection, false)) => {
                if collection.role < CollectionRole::Editor {
                    Err(format!(
                        "changing APIs needs an editor's role in {:?}; this account views it",
                        collection.name
                    ))
                } else {
                    arguments["project"] = json!(collection.id.as_str());
                    let res = create_api(storage, &arguments).await;
                    if res.is_ok()
                        && let Some(on_write) = &context.on_write
                    {
                        on_write(collection.id);
                    }
                    res
                }
            }
            Err(why) => Err(why),
        },
        "update_api" if can_write => match resolve_local_project(storage, context, &arguments).await {
            Ok((collection, true)) => Err(format!(
                "{:?} is a server-synced project: edit it in the CyberAPI app or use the bridge's MCP endpoint",
                collection.name
            )),
            Ok((collection, false)) => {
                if collection.role < CollectionRole::Editor {
                    Err(format!(
                        "changing APIs needs an editor's role in {:?}; this account views it",
                        collection.name
                    ))
                } else {
                    arguments["project"] = json!(collection.id.as_str());
                    let res = update_api(storage, &arguments).await;
                    if res.is_ok()
                        && let Some(on_write) = &context.on_write
                    {
                        on_write(collection.id);
                    }
                    res
                }
            }
            Err(why) => Err(why),
        },
        "create_api" | "update_api" => Err(
            "this local MCP server is not allowed to change projects: enable 'Allow creating and updating APIs' in CyberAPI"
                .into(),
        ),
        other => return Err((INVALID_PARAMS, format!("unknown tool {other:?}"))),
    };
    tracing::info!(tool, ok = answer.is_ok(), "local mcp call");
    Ok(finish(answer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::AccountId;
    use cyberapi_core::sync::CopyBinding;
    use cyberapi_http::{HttpEngine, PreparedRequest, SendError, SendFuture, Watch};
    use cyberapi_store::{MemoryStore, NewCollection, ServerCopyStore};

    struct DummyEngine;

    impl HttpEngine for DummyEngine {
        fn send(&self, _request: PreparedRequest) -> SendFuture {
            Box::pin(async {
                Err(SendError::Http {
                    message: "not sent in unit test".into(),
                })
            })
        }

        fn send_watched(&self, request: PreparedRequest, _watch: Watch) -> SendFuture {
            self.send(request)
        }
    }

    #[test]
    fn handle_local_runs_initialize_list_create_and_update() {
        pollster::block_on(async {
            let store = Arc::new(MemoryStore::new());
            let storage = Storage::from_backend(store);
            let engine: Arc<dyn HttpEngine> = Arc::new(DummyEngine);
            let created = storage
                .collections()
                .create(NewCollection {
                    id: None,
                    name: "Shop".into(),
                    description: "Orders".into(),
                    owner: None,
                })
                .await
                .expect("collection created");

            let (status, init) = handle_local(
                &storage,
                &engine,
                false,
                false,
                br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
            )
            .await;
            assert_eq!(status, 200);
            assert!(init.unwrap_or_default().contains("cyberapi-desktop"));

            // Without can_write, create_api is refused.
            let (_, denied) = handle_local(
                &storage,
                &engine,
                false,
                false,
                br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_api","arguments":{"project":"Shop","name":"Get Order"}}}"#,
            )
            .await;
            assert!(denied.unwrap_or_default().contains("\"isError\":true"));

            // With can_write, create_api creates the API and get_api reads it back.
            let (_, created_reply) = handle_local(
                &storage,
                &engine,
                false,
                true,
                br#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_api","arguments":{"project":"Shop","name":"Get Order","method":"GET","url":"https://api.example.com/orders/1"}}}"#,
            )
            .await;
            let text = created_reply.unwrap_or_default();
            assert!(text.contains("\"isError\":false"));
            assert!(text.contains("Get Order"));

            let tree = storage.tree().load(created.id).await.expect("tree loaded");
            assert_eq!(tree.apis.len(), 1);
        });
    }

    #[test]
    fn handle_local_hides_other_account_copies_and_protects_server_copies_and_viewer_roles() {
        pollster::block_on(async {
            let store = Arc::new(MemoryStore::new());
            let storage = Storage::from_backend(store.clone());
            let engine: Arc<dyn HttpEngine> = Arc::new(DummyEngine);

            let local_proj = storage
                .collections()
                .create(NewCollection {
                    id: None,
                    name: "LocalProj".into(),
                    description: String::new(),
                    owner: None,
                })
                .await
                .expect("local_proj");
            let my_copy = storage
                .collections()
                .create(NewCollection {
                    id: None,
                    name: "MyServerCopy".into(),
                    description: String::new(),
                    owner: None,
                })
                .await
                .expect("my_copy");
            let other_copy = storage
                .collections()
                .create(NewCollection {
                    id: None,
                    name: "OtherAccountCopy".into(),
                    description: String::new(),
                    owner: None,
                })
                .await
                .expect("other_copy");

            let copies: Arc<dyn ServerCopyStore> = store.clone();
            copies
                .bind(CopyBinding {
                    collection: my_copy.id.clone(),
                    account: AccountId::new("acct-me"),
                    base: Some(1),
                    unsynced: false,
                })
                .await
                .expect("bind my_copy");
            copies
                .bind(CopyBinding {
                    collection: other_copy.id.clone(),
                    account: AccountId::new("acct-other"),
                    base: Some(1),
                    unsynced: false,
                })
                .await
                .expect("bind other_copy");

            let touched = Arc::new(std::sync::Mutex::new(Vec::new()));
            let touched_cl = touched.clone();
            let mut roles_map = std::collections::HashMap::new();
            roles_map.insert(my_copy.id.clone(), CollectionRole::Viewer);
            let ctx = LocalMcpContext {
                copies: Some(copies),
                account: Arc::new(std::sync::RwLock::new(Some(AccountId::new("acct-me")))),
                roles: Arc::new(std::sync::RwLock::new(roles_map)),
                on_write: Some(Arc::new(move |id| {
                    touched_cl.lock().expect("lock").push(id);
                })),
            };

            // 1. list_projects hides OtherAccountCopy and shows MyServerCopy as viewer.
            let (_, list_reply) = handle_local_with_context(
                &storage,
                &engine,
                &ctx,
                true,
                true,
                br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_projects","arguments":{}}}"#,
            )
            .await;
            let list_text = list_reply.unwrap_or_default();
            assert!(list_text.contains("LocalProj"));
            assert!(list_text.contains("MyServerCopy"));
            assert!(
                !list_text.contains("OtherAccountCopy"),
                "other account's server copy must be hidden: {list_text}"
            );
            assert!(list_text.contains("\\\"role\\\": \\\"viewer\\\""));

            // 2. Accessing OtherAccountCopy by name is refused as not found.
            let (_, hidden_reply) = handle_local_with_context(
                &storage,
                &engine,
                &ctx,
                true,
                true,
                br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_apis","arguments":{"project":"OtherAccountCopy"}}}"#,
            )
            .await;
            let hidden_text = hidden_reply.unwrap_or_default();
            assert!(hidden_text.contains("\"isError\":true"));
            assert!(hidden_text.contains("no project is"));

            // 3. create_api on MyServerCopy is refused because it is a server-synced project.
            let (_, copy_write_reply) = handle_local_with_context(
                &storage,
                &engine,
                &ctx,
                true,
                true,
                br#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_api","arguments":{"project":"MyServerCopy","name":"NewApi"}}}"#,
            )
            .await;
            let copy_write_text = copy_write_reply.unwrap_or_default();
            assert!(copy_write_text.contains("\"isError\":true"));
            assert!(copy_write_text.contains("server-synced project"));

            // 4. create_api on LocalProj succeeds and triggers on_write.
            let (_, local_write_reply) = handle_local_with_context(
                &storage,
                &engine,
                &ctx,
                true,
                true,
                br#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"create_api","arguments":{"project":"LocalProj","name":"Ping","url":"https://example.com/ping"}}}"#,
            )
            .await;
            let local_write_text = local_write_reply.unwrap_or_default();
            assert!(local_write_text.contains("\"isError\":false"), "{local_write_text}");
            assert_eq!(touched.lock().expect("lock").as_slice(), &[local_proj.id]);
        });
    }
}
