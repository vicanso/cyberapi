//! R12 (2/3): Yaak export JSON importer (`yaakSchema` / `yaakVersion` / object `resources`).
//!
//! Imports workspaces, folders (`sortPriority`), HTTP requests, GraphQL requests,
//! WebSocket requests (`websocketRequests`), gRPC requests (`grpcRequests`),
//! base and named environments, headers, query and `:param` path variables,
//! auth (Basic, Bearer, API key, Digest, AWS SigV4, JWT Bearer, OAuth 2.0),
//! and converts `${[ var ]}`, `${[ uuid.v4() ]}`, `${[ timestamp.unix() ]}`,
//! and `${[ timestamp.iso8601() ]}` template tags. Unconverted `${[ ... ]}`
//! tags and unsupported auth/body kinds are reported via [`lost`].

use super::lost::{self, Lost};
use super::{ImportBundle, ImportEnvironment, ImportItem, api, content_type_of, guess_content_type, kv};
use crate::auth::{Auth, KeyPlace};
use crate::aws_sigv4::AwsSigV4;
use crate::graphql::GraphQlBody;
use crate::grpc::GrpcBody;
use crate::jwt_bearer::{JwtAlgorithm, JwtBearer};
use crate::model::{ContentType, FolderSettings, HttpMethod, HttpRequest, KvParam};
use crate::oauth::{ClientAuth, Grant, OAuth2};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Whether `json` is a Yaak JSON export (`resources` is an object containing Yaak resource arrays).
pub(super) fn is_yaak(json: &Value) -> bool {
    let Some(map) = json.as_object() else {
        return false;
    };
    let Some(resources) = map.get("resources").and_then(Value::as_object) else {
        return false;
    };
    if map.contains_key("yaakVersion") || map.contains_key("yaakSchema") {
        return true;
    }
    [
        "workspaces",
        "httpRequests",
        "grpcRequests",
        "websocketRequests",
        "folders",
        "environments",
    ]
    .into_iter()
    .any(|key| resources.get(key).is_some_and(Value::is_array))
}

/// The project's own name when exporting a single workspace.
pub(super) fn name(json: &Value) -> Option<&str> {
    let workspaces = json
        .get("resources")
        .and_then(|r| r.get("workspaces"))
        .and_then(Value::as_array)?;
    if workspaces.len() == 1 {
        workspaces[0].get("name").and_then(Value::as_str)
    } else {
        None
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn value_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(num)) => num.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    }
}

fn bool_flag(value: &Value, key: &str) -> Option<bool> {
    match value.get(key) {
        Some(Value::Bool(flag)) => Some(*flag),
        Some(Value::String(text)) => match text.trim().to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn is_enabled(value: &Value) -> bool {
    bool_flag(value, "enabled") != Some(false)
}

fn sort_priority(value: &Value) -> f64 {
    value
        .get("sortPriority")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|n| n as f64)))
        .unwrap_or(0.0)
}

/// Converts simple Yaak `${[ ... ]}` tags to CyberAPI `{{...}}` expressions;
/// complex template expressions remain untouched so `lost::scan_yaak_tags` reports them.
fn convert_templates(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${[") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 3..];
        let Some(end) = after.find("]}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after[..end];
        if inner.contains(['\n', '\r']) {
            out.push_str("${[");
            rest = after;
            continue;
        }
        let trimmed = inner.trim();
        let converted = match trimmed {
            "uuid.v4()" => Some("{{uuid()}}".to_string()),
            "timestamp.unix()" => Some("{{timestamp()}}".to_string()),
            "timestamp.iso8601()" => Some("{{$isoTimestamp}}".to_string()),
            ident
                if !ident.is_empty()
                    && ident
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') =>
            {
                Some(format!("{{{{{ident}}}}}"))
            }
            _ => None,
        };
        if let Some(expr) = converted {
            out.push_str(&expr);
        } else {
            out.push_str(&rest[start..start + 3 + end + 2]);
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

fn kv_list(value: Option<&Value>) -> Vec<KvParam> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let key = text(item, "name").trim();
                    if key.is_empty() {
                        return None;
                    }
                    Some(kv(
                        &convert_templates(key),
                        &convert_templates(&value_text(item.get("value"))),
                        is_enabled(item),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn has_header(headers: &[KvParam], key: &str) -> bool {
    headers.iter().any(|header| header.key.trim().eq_ignore_ascii_case(key))
}

fn normalize_file_value(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "file://".to_string()
    } else if trimmed.starts_with("file://") {
        trimmed.to_string()
    } else {
        format!("file://{trimmed}")
    }
}

fn resource_list<'a>(resources: Option<&'a serde_json::Map<String, Value>>, key: &str) -> &'a [Value] {
    resources
        .and_then(|map| map.get(key))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

pub(super) fn parse(json: &Value) -> ImportBundle {
    let resources = json.get("resources").and_then(Value::as_object);
    let workspaces = resource_list(resources, "workspaces");
    let environments_raw = resource_list(resources, "environments");
    let folders_raw = resource_list(resources, "folders");
    let http_raw = resource_list(resources, "httpRequests");
    let ws_raw = resource_list(resources, "websocketRequests");
    let grpc_raw = resource_list(resources, "grpcRequests");

    let (shared_env, named_envs) = parse_environments(environments_raw);

    let mut lost = Vec::new();
    let workspace_ids: HashSet<&str> = workspaces
        .iter()
        .map(|ws| text(ws, "id").trim())
        .filter(|id| !id.is_empty())
        .collect();
    let folder_ids_by_ws: HashMap<&str, HashSet<&str>> = {
        let mut by_ws: HashMap<&str, HashSet<&str>> = HashMap::new();
        for f in folders_raw {
            let id = text(f, "id").trim();
            if id.is_empty() {
                continue;
            }
            let raw_ws = text(f, "workspaceId").trim();
            let ws_key = if workspace_ids.contains(raw_ws) { raw_ws } else { "" };
            by_ws.entry(ws_key).or_default().insert(id);
        }
        by_ws
    };

    #[derive(Clone, Copy)]
    enum Kind<'a> {
        Folder(usize, &'a Value),
        Http(&'a Value),
        WebSocket(&'a Value),
        Grpc(&'a Value),
    }

    type ChildEntry<'a> = (f64, u8, usize, Kind<'a>);
    type ChildrenMap<'a> = HashMap<(&'a str, &'a str), Vec<ChildEntry<'a>>>;

    // Key: (ws_key, parent_folder_id) -> list of (sortPriority, folder_before_req_rank, orig_idx, Kind)
    let mut children_by_parent: ChildrenMap<'_> = HashMap::new();
    let mut ws_folders: HashMap<&str, Vec<(usize, &Value)>> = HashMap::new();

    for (idx, f) in folders_raw.iter().enumerate() {
        let id = text(f, "id").trim();
        let raw_ws = text(f, "workspaceId").trim();
        let ws_key = if workspace_ids.contains(raw_ws) { raw_ws } else { "" };
        let valid_folders = folder_ids_by_ws.get(ws_key);
        let raw_parent = text(f, "folderId").trim();
        let parent = if !raw_parent.is_empty()
            && raw_parent != id
            && valid_folders.is_some_and(|set| set.contains(raw_parent))
        {
            raw_parent
        } else {
            ""
        };
        children_by_parent
            .entry((ws_key, parent))
            .or_default()
            .push((sort_priority(f), 0, idx, Kind::Folder(idx, f)));
        ws_folders.entry(ws_key).or_default().push((idx, f));
    }

    for (base_offset, kind_tag, raw_list) in [
        (0usize, 0u8, http_raw),
        (http_raw.len(), 1u8, ws_raw),
        (http_raw.len() + ws_raw.len(), 2u8, grpc_raw),
    ] {
        for (idx, r) in raw_list.iter().enumerate() {
            let raw_ws = text(r, "workspaceId").trim();
            let ws_key = if workspace_ids.contains(raw_ws) { raw_ws } else { "" };
            let valid_folders = folder_ids_by_ws.get(ws_key);
            let raw_parent = text(r, "folderId").trim();
            let parent = if !raw_parent.is_empty() && valid_folders.is_some_and(|set| set.contains(raw_parent)) {
                raw_parent
            } else {
                ""
            };
            let kind = match kind_tag {
                0 => Kind::Http(r),
                1 => Kind::WebSocket(r),
                _ => Kind::Grpc(r),
            };
            children_by_parent.entry((ws_key, parent)).or_default().push((
                sort_priority(r),
                1,
                base_offset + idx,
                kind,
            ));
        }
    }

    for entries in children_by_parent.values_mut() {
        entries.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
    }

    struct WalkCx<'a> {
        children_by_parent: &'a ChildrenMap<'a>,
        visiting: HashSet<&'a str>,
        visited_folders: Vec<bool>,
        lost: &'a mut Vec<Lost>,
    }

    fn build_level<'a>(ws_key: &'a str, parent_id: &str, prefix: &str, cx: &mut WalkCx<'a>) -> Vec<ImportItem> {
        let Some(entries) = cx.children_by_parent.get(&(ws_key, parent_id)) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(entries.len());
        for (_, _, _, kind) in entries {
            match *kind {
                Kind::Folder(folder_idx, folder_val) => {
                    if let Some(item) = visit_folder(ws_key, folder_idx, folder_val, prefix, cx) {
                        out.push(item);
                    }
                }
                Kind::Http(req) => out.push(http_request(req, prefix, cx.lost)),
                Kind::WebSocket(req) => out.push(websocket_request(req, prefix, cx.lost)),
                Kind::Grpc(req) => out.push(grpc_request(req, prefix, cx.lost)),
            }
        }
        out
    }

    fn visit_folder<'a>(
        ws_key: &'a str,
        folder_idx: usize,
        folder_val: &'a Value,
        prefix: &str,
        cx: &mut WalkCx<'a>,
    ) -> Option<ImportItem> {
        if cx.visited_folders[folder_idx] {
            return None;
        }
        let id = text(folder_val, "id").trim();
        if !id.is_empty() && !cx.visiting.insert(id) {
            return None;
        }
        cx.visited_folders[folder_idx] = true;

        let raw_name = convert_templates(text(folder_val, "name").trim());
        let folder_name = if raw_name.trim().is_empty() {
            "Folder".to_string()
        } else {
            raw_name.trim().to_string()
        };
        let target = lost::join_path(prefix, &folder_name);
        let settings = container_settings(folder_val, &target, cx.lost);
        let children = if id.is_empty() {
            Vec::new()
        } else {
            build_level(ws_key, id, &target, cx)
        };

        if !id.is_empty() {
            cx.visiting.remove(id);
        }
        Some(ImportItem::Folder {
            name: folder_name,
            children,
            settings,
        })
    }

    fn build_ws_items<'a>(
        ws_key: &'a str,
        prefix: &str,
        ws_folders: &HashMap<&'a str, Vec<(usize, &'a Value)>>,
        cx: &mut WalkCx<'a>,
    ) -> Vec<ImportItem> {
        let mut items = build_level(ws_key, "", prefix, cx);
        if let Some(list) = ws_folders.get(ws_key) {
            let mut sorted_leftover = list.clone();
            sorted_leftover.sort_by(|a, b| {
                sort_priority(a.1)
                    .total_cmp(&sort_priority(b.1))
                    .then_with(|| a.0.cmp(&b.0))
            });
            for (folder_idx, folder_val) in sorted_leftover {
                if !cx.visited_folders[folder_idx]
                    && let Some(item) = visit_folder(ws_key, folder_idx, folder_val, prefix, cx)
                {
                    items.push(item);
                }
            }
        }
        items
    }

    let mut cx = WalkCx {
        children_by_parent: &children_by_parent,
        visiting: HashSet::new(),
        visited_folders: vec![false; folders_raw.len()],
        lost: &mut lost,
    };

    let mut bundle_auth = Auth::Inherit;
    let mut items = Vec::new();

    if workspaces.len() > 1 {
        for ws in workspaces {
            let ws_id = text(ws, "id").trim();
            let raw_name = convert_templates(text(ws, "name").trim());
            let ws_name = if raw_name.trim().is_empty() {
                "Workspace".to_string()
            } else {
                raw_name.trim().to_string()
            };
            let settings = container_settings(ws, &ws_name, cx.lost);
            let children = if ws_id.is_empty() {
                Vec::new()
            } else {
                build_ws_items(ws_id, &ws_name, &ws_folders, &mut cx)
            };
            items.push(ImportItem::Folder {
                name: ws_name,
                children,
                settings,
            });
        }
        // Append any requests/folders not assigned to one of the listed workspaces.
        items.extend(build_ws_items("", "", &ws_folders, &mut cx));
    } else if let Some(ws) = workspaces.first() {
        let ws_id = text(ws, "id").trim();
        let ws_settings = container_settings(ws, "", cx.lost);
        bundle_auth = ws_settings.auth;
        let wraps_headers = !ws_settings.headers.is_empty();
        let raw_name = convert_templates(text(ws, "name").trim());
        let ws_name = if raw_name.trim().is_empty() {
            "Workspace".to_string()
        } else {
            raw_name.trim().to_string()
        };
        let root_prefix = if wraps_headers { ws_name.as_str() } else { "" };
        let mut root_items = if ws_id.is_empty() {
            Vec::new()
        } else {
            build_ws_items(ws_id, root_prefix, &ws_folders, &mut cx)
        };
        if !ws_id.is_empty() {
            root_items.extend(build_ws_items("", root_prefix, &ws_folders, &mut cx));
        }
        if wraps_headers {
            items = vec![ImportItem::Folder {
                name: ws_name,
                children: root_items,
                settings: FolderSettings {
                    auth: Auth::Inherit,
                    headers: ws_settings.headers,
                    ..Default::default()
                },
            }];
        } else {
            items = root_items;
        }
    } else {
        items = build_ws_items("", "", &ws_folders, &mut cx);
    }

    let mut bundle = ImportBundle {
        items,
        environment: shared_env,
        auth: bundle_auth,
        environments: named_envs,
        lost,
        ..Default::default()
    };
    lost::scan_yaak_tags(&mut bundle);
    bundle
}

fn parse_environments(environments: &[Value]) -> (Vec<(String, String)>, Vec<ImportEnvironment>) {
    let parent_ids: HashSet<&str> = environments
        .iter()
        .flat_map(|env| {
            ["environmentId", "parentId"]
                .into_iter()
                .map(|key| text(env, key).trim())
                .filter(|id| !id.is_empty())
        })
        .collect();

    let mut shared = Vec::new();
    let mut named = Vec::new();

    for env in environments {
        let id = text(env, "id").trim();
        let parent_model = text(env, "parentModel").trim();
        let is_base = bool_flag(env, "base") == Some(true)
            || parent_model.eq_ignore_ascii_case("workspace")
            || (!id.is_empty() && parent_ids.contains(id));
        let vars: Vec<(String, String)> = env
            .get("variables")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter(|v| is_enabled(v))
                    .filter_map(|v| {
                        let key = text(v, "name").trim();
                        if key.is_empty() {
                            return None;
                        }
                        let val = convert_templates(&value_text(v.get("value")));
                        Some((key.to_string(), val))
                    })
                    .collect()
            })
            .unwrap_or_default();

        if is_base {
            shared.extend(vars);
        } else {
            let env_name = convert_templates(text(env, "name").trim()).trim().to_string();
            if !env_name.is_empty() {
                named.push(ImportEnvironment {
                    name: env_name,
                    variables: vars,
                });
            }
        }
    }
    (shared, named)
}

fn container_settings(value: &Value, target: &str, lost: &mut Vec<Lost>) -> FolderSettings {
    FolderSettings {
        auth: container_auth(value, target, lost).unwrap_or_default(),
        headers: kv_list(value.get("headers")),
        description: convert_templates(text(value, "description").trim()),
        variables: Vec::new(),
        source: None,
    }
}

fn apply_url_parameters(url_params: Option<&Value>, built: &mut HttpRequest) {
    let mut param_query = Vec::new();
    let mut seen_keys: HashSet<String> = HashSet::new();
    if let Some(params) = url_params.and_then(Value::as_array) {
        for param in params {
            let raw_key = text(param, "name").trim();
            if raw_key.is_empty() {
                continue;
            }
            let enabled = is_enabled(param);
            let val = convert_templates(&value_text(param.get("value")));
            if let Some(path_name) = raw_key.strip_prefix(':') {
                let path_name = path_name.trim();
                if !path_name.is_empty() {
                    built.variables.push(kv(path_name, &val, enabled));
                }
            } else {
                let key = convert_templates(raw_key);
                seen_keys.insert(key.clone());
                param_query.push(kv(&key, &val, enabled));
            }
        }
    }
    for url_param in built.query.drain(..) {
        if !seen_keys.contains(&url_param.key) {
            param_query.push(url_param);
        }
    }
    built.query = param_query;
}

fn http_request(value: &Value, prefix: &str, lost: &mut Vec<Lost>) -> ImportItem {
    let url = convert_templates(text(value, "url"));
    let (name, mut built) = api(&convert_templates(text(value, "name")), text(value, "method"), &url);
    let target = lost::join_path(prefix, &name);

    built.description = convert_templates(text(value, "description").trim());
    built.headers = kv_list(value.get("headers"));
    apply_url_parameters(value.get("urlParameters"), &mut built);

    let body_type_raw = text(value, "bodyType").trim();
    let body_type = body_type_raw.to_ascii_lowercase();
    let body_obj = value.get("body").unwrap_or(&Value::Null);
    let body_text = text(body_obj, "text");

    match body_type.as_str() {
        "" | "none" => {
            if !body_text.trim().is_empty() {
                built.body = convert_templates(body_text);
                built.content_type = guess_content_type(&built.body);
            }
        }
        "application/json" | "json" => {
            built.content_type = ContentType::Json;
            built.body = convert_templates(body_text);
        }
        "text/xml" | "application/xml" | "xml" => {
            built.content_type = ContentType::Xml;
            built.body = convert_templates(body_text);
        }
        "text/plain" | "other" | "text" => {
            built.content_type = ContentType::Plain;
            built.body = convert_templates(body_text);
        }
        "text/html" => {
            built.content_type = ContentType::Plain;
            built.body = convert_templates(body_text);
            if !has_header(&built.headers, "content-type") {
                built.headers.push(kv("Content-Type", "text/html", true));
            }
        }
        "application/x-www-form-urlencoded" | "form_urlencoded" => {
            built.content_type = ContentType::Form;
            let fields = kv_list(body_obj.get("form"));
            built.body = serde_json::to_string(&fields).unwrap_or_default();
        }
        "multipart/form-data" | "multipart" => {
            built.content_type = ContentType::Multipart;
            let fields: Vec<KvParam> = body_obj
                .get("form")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let key = text(item, "name").trim();
                            if key.is_empty() {
                                return None;
                            }
                            let enabled = is_enabled(item);
                            let file_field = item.get("file").and_then(Value::as_str);
                            let is_file = bool_flag(item, "isFile") == Some(true)
                                || text(item, "type").trim().eq_ignore_ascii_case("file")
                                || file_field.is_some();
                            let val = match file_field {
                                Some(path) => normalize_file_value(&convert_templates(path)),
                                None if is_file => "file://".to_string(),
                                None => convert_templates(&value_text(item.get("value"))),
                            };
                            Some(kv(&convert_templates(key), &val, enabled))
                        })
                        .collect()
                })
                .unwrap_or_default();
            built.body = serde_json::to_string(&fields).unwrap_or_default();
        }
        "graphql" => {
            built.content_type = ContentType::GraphQl;
            let (query, variables, operation_name) = if !body_text.trim().is_empty()
                && let Ok(Value::Object(parsed)) = serde_json::from_str::<Value>(body_text)
                && (parsed.contains_key("query") || parsed.contains_key("variables"))
            {
                let q = parsed.get("query").and_then(Value::as_str).unwrap_or_default();
                let vars = match parsed.get("variables") {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) if !other.is_null() => crate::format::json(other),
                    _ => String::new(),
                };
                let op = parsed.get("operationName").and_then(Value::as_str).unwrap_or_default();
                (q.to_string(), vars, op.to_string())
            } else {
                let q = text(body_obj, "query");
                let vars = match body_obj.get("variables") {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) if !other.is_null() => crate::format::json(other),
                    _ => String::new(),
                };
                let op = text(body_obj, "operationName");
                (
                    if q.is_empty() {
                        body_text.to_string()
                    } else {
                        q.to_string()
                    },
                    vars,
                    op.to_string(),
                )
            };
            built.body = GraphQlBody {
                query: convert_templates(&query),
                variables: convert_templates(&variables),
                operation_name: convert_templates(&operation_name),
            }
            .to_body();
        }
        "binary" => {
            built.content_type = ContentType::Binary;
            let raw_path = ["filePath", "file", "path"]
                .into_iter()
                .map(|k| text(body_obj, k).trim())
                .find(|s| !s.is_empty())
                .unwrap_or_default();
            built.body = normalize_file_value(&convert_templates(raw_path));
        }
        _ => {
            if let Some(known) = content_type_of(&body_type) {
                built.content_type = known;
                built.body = convert_templates(body_text);
            } else if !body_text.trim().is_empty() && body_type_raw.contains('/') {
                built.body = convert_templates(body_text);
                built.content_type = guess_content_type(&built.body);
                if !has_header(&built.headers, "content-type") {
                    built.headers.push(kv("Content-Type", body_type_raw, true));
                }
            } else {
                lost::push_body(lost, &target, body_type_raw);
            }
        }
    }

    if !built.method.has_body() && built.method != HttpMethod::Get {
        built.content_type = ContentType::default();
    }

    if let Some(own_auth) = container_auth(value, &target, lost) {
        built.auth = own_auth;
    }

    ImportItem::Api {
        name,
        request: Box::new(built),
    }
}

fn websocket_request(value: &Value, prefix: &str, lost: &mut Vec<Lost>) -> ImportItem {
    let url = convert_templates(text(value, "url"));
    let (name, mut built) = api(&convert_templates(text(value, "name")), "WS", &url);
    built.method = HttpMethod::WebSocket;
    let target = lost::join_path(prefix, &name);

    built.description = convert_templates(text(value, "description").trim());
    built.headers = kv_list(value.get("headers"));
    apply_url_parameters(value.get("urlParameters"), &mut built);
    built.body = convert_templates(text(value, "message"));
    if let Some(own_auth) = container_auth(value, &target, lost) {
        built.auth = own_auth;
    }

    ImportItem::Api {
        name,
        request: Box::new(built),
    }
}

fn grpc_request(value: &Value, prefix: &str, lost: &mut Vec<Lost>) -> ImportItem {
    let url = convert_templates(text(value, "url"));
    let (uri, _) = super::split_url(url.trim());
    let raw_name = convert_templates(text(value, "name"));
    let name = if raw_name.trim().is_empty() {
        if uri.is_empty() {
            "gRPC".to_string()
        } else {
            uri.clone()
        }
    } else {
        raw_name.trim().to_string()
    };
    let target = lost::join_path(prefix, &name);

    let mut built = HttpRequest {
        method: HttpMethod::Grpc,
        uri,
        headers: kv_list(value.get("metadata")),
        body: GrpcBody {
            service: text(value, "service").trim().to_string(),
            method: text(value, "method").trim().to_string(),
            message: convert_templates(text(value, "message")),
            descriptors: String::new(),
            source: String::new(),
        }
        .to_body(),
        description: convert_templates(text(value, "description").trim()),
        ..Default::default()
    };
    if let Some(own_auth) = container_auth(value, &target, lost) {
        built.auth = own_auth;
    }

    ImportItem::Api {
        name,
        request: Box::new(built),
    }
}

fn container_auth(value: &Value, target: &str, lost: &mut Vec<Lost>) -> Option<Auth> {
    let cfg = value.get("authentication").unwrap_or(&Value::Null);
    if bool_flag(cfg, "disabled") == Some(true) {
        return Some(Auth::None);
    }
    let kind_raw = text(value, "authenticationType").trim();
    let kind = kind_raw.to_ascii_lowercase();
    let opt = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|k| {
            let raw = cfg.get(*k)?;
            if raw.is_null() {
                None
            } else {
                Some(convert_templates(&value_text(Some(raw))))
            }
        })
    };
    let field = |keys: &[&str]| opt(keys).unwrap_or_default();

    Some(match kind.as_str() {
        "" | "inherit" => Auth::Inherit,
        "none" | "noauth" => Auth::None,
        "basic" => Auth::Basic {
            username: field(&["username", "user"]),
            password: field(&["password", "pass"]),
        },
        "bearer" => Auth::Bearer {
            token: field(&["token", "bearer"]),
        },
        "apikey" | "api-key" => {
            let place = if field(&["location", "in", "addTo"])
                .to_ascii_lowercase()
                .contains("query")
            {
                KeyPlace::Query
            } else {
                KeyPlace::Header
            };
            Auth::ApiKey {
                name: field(&["key", "name"]),
                value: field(&["value", "token"]),
                place,
            }
        }
        "digest" => Auth::Digest {
            username: field(&["username", "user"]),
            password: field(&["password", "pass"]),
        },
        "awsv4" | "aws" => Auth::from(AwsSigV4 {
            access_key: field(&["accessKeyId", "accessKey"]),
            secret_key: field(&["secretAccessKey", "secretKey"]),
            session_token: field(&["sessionToken"]),
            region: field(&["region"]),
            service: field(&["service"]),
        }),
        "jwt" => {
            let raw_alg = field(&["algorithm", "alg"]);
            let algorithm = if raw_alg.trim().is_empty() {
                JwtAlgorithm::HS256
            } else if let Some(alg) = JwtAlgorithm::parse(&raw_alg) {
                alg
            } else {
                lost::push_auth(lost, target, &format!("jwt ({})", raw_alg.trim()));
                return None;
            };
            let payload = {
                let p = field(&["payload"]);
                if p.trim().is_empty() { "{}".into() } else { p }
            };
            let header = opt(&["header", "headers"]).unwrap_or_default();
            let place = if field(&["location", "addTokenTo", "place"])
                .to_ascii_lowercase()
                .contains("query")
            {
                KeyPlace::Query
            } else {
                KeyPlace::Header
            };
            let prefix = opt(&["locationPrefix", "headerPrefix", "prefix"]).unwrap_or_else(|| "Bearer".into());
            let query_param = ["locationName", "queryParamKey", "queryParam"]
                .into_iter()
                .filter_map(|k| opt(&[k]))
                .map(|s| s.trim().to_string())
                .find(|s| !s.is_empty())
                .unwrap_or_else(|| "token".into());
            let secret_base64 = field(&["secretBase64", "isSecretBase64Encoded"])
                .trim()
                .eq_ignore_ascii_case("true");
            Auth::from(JwtBearer {
                algorithm,
                secret: field(&["secret", "privateKey"]),
                secret_base64,
                payload,
                header,
                place,
                prefix,
                query_param,
            })
        }
        "oauth2" => {
            let grant_raw = field(&["grantType", "grant_type"]);
            let grant_norm = grant_raw.trim().to_ascii_lowercase().replace(['_', '-', ' '], "");
            let grant = match grant_norm.as_str() {
                "clientcredentials" => Grant::ClientCredentials,
                "password" | "passwordcredentials" => Grant::Password,
                "authorizationcode" | "authorizationcodewithpkce" | "pkce" => Grant::AuthorizationCode,
                "implicit" => Grant::Implicit,
                _ => {
                    let kind = if grant_raw.trim().is_empty() {
                        "oauth2".to_string()
                    } else {
                        format!("oauth2 ({})", grant_raw.trim())
                    };
                    lost::push_auth(lost, target, &kind);
                    return None;
                }
            };
            let redirect_port = ["redirectUri", "callbackUrl", "redirect_uri", "callback"]
                .into_iter()
                .filter_map(|k| opt(&[k]))
                .find_map(|uri| {
                    let port = crate::oauth::redirect_port_of(&uri);
                    (port != 0).then_some(port)
                })
                .unwrap_or(0);
            let client_auth = if field(&["credentials", "clientAuth", "client_authentication"])
                .to_ascii_lowercase()
                .contains("body")
            {
                ClientAuth::Body
            } else {
                ClientAuth::Header
            };
            Auth::from(OAuth2 {
                grant,
                token_url: field(&["accessTokenUrl", "tokenUrl"]),
                authorization_url: field(&["authorizationUrl", "authUrl"]),
                redirect_port,
                client_id: field(&["clientId"]),
                client_secret: field(&["clientSecret"]),
                username: field(&["username"]),
                password: field(&["password"]),
                scope: field(&["scope"]),
                audience: field(&["audience"]),
                resource: field(&["resource"]),
                client_auth,
            })
        }
        _ => {
            lost::push_auth(lost, target, kind_raw);
            return None;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::lost::LostKind;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn workspace_folders_environments_and_templates() {
        let doc = json!({
            "yaakVersion": "2025.4.0",
            "yaakSchema": 4,
            "resources": {
                "workspaces": [
                    {
                        "id": "wk_1",
                        "name": "Shop Workspace",
                        "description": "Main API",
                        "authenticationType": "bearer",
                        "authentication": { "token": "${[ api_token ]}" },
                        "headers": [{ "name": "X-Workspace", "value": "${[ uuid.v4() ]}", "enabled": true }]
                    }
                ],
                "environments": [
                    {
                        "id": "ev_base",
                        "workspaceId": "wk_1",
                        "parentModel": "workspace",
                        "base": true,
                        "name": "Global",
                        "variables": [
                            { "name": "BASE_URL", "value": "https://api.example.com", "enabled": true },
                            { "name": "OFF", "value": "skip", "enabled": false }
                        ]
                    },
                    {
                        "id": "ev_dev",
                        "workspaceId": "wk_1",
                        "parentModel": "environment",
                        "base": false,
                        "environmentId": "ev_base",
                        "name": "Dev",
                        "variables": [
                            { "name": "TAG", "value": "${[ timestamp.iso8601() ]}", "enabled": true }
                        ]
                    }
                ],
                "folders": [
                    {
                        "id": "fd_sub",
                        "workspaceId": "wk_1",
                        "folderId": "fd_users",
                        "name": "Admin",
                        "sortPriority": 20.0,
                        "authenticationType": "none",
                        "authentication": { "disabled": true }
                    },
                    {
                        "id": "fd_users",
                        "workspaceId": "wk_1",
                        "folderId": null,
                        "name": "Users",
                        "description": "User group",
                        "sortPriority": 10.0,
                        "authenticationType": "basic",
                        "authentication": { "username": "u", "password": "${[ pw ]}" }
                    }
                ],
                "httpRequests": [
                    {
                        "id": "rq_1",
                        "workspaceId": "wk_1",
                        "folderId": "fd_users",
                        "name": "Get User",
                        "method": "GET",
                        "url": "${[ BASE_URL ]}/users/:id?expand=profile",
                        "urlParameters": [
                            { "name": ":id", "value": "${[ user_id ]}", "enabled": true },
                            { "name": "expand", "value": "roles", "enabled": false },
                            { "name": "ts", "value": "${[ timestamp.unix() ]}", "enabled": true }
                        ],
                        "headers": [
                            { "name": "X-Dyn", "value": "${[ response.body.path('$.id') ]}", "enabled": true }
                        ],
                        "description": "Fetch by ${[ user_id ]}",
                        "sortPriority": 5.0
                    },
                    {
                        "id": "rq_health",
                        "workspaceId": "wk_1",
                        "folderId": null,
                        "name": "Health",
                        "method": "GET",
                        "url": "${[ BASE_URL ]}/health",
                        "sortPriority": 1.0
                    }
                ]
            }
        });

        assert!(is_yaak(&doc));
        assert_eq!(name(&doc), Some("Shop Workspace"));

        let bundle = parse(&doc);
        assert_eq!(
            bundle.auth,
            Auth::Bearer {
                token: "{{api_token}}".into()
            }
        );
        assert_eq!(
            bundle.environment,
            vec![("BASE_URL".into(), "https://api.example.com".into())]
        );
        assert_eq!(
            bundle.environments,
            vec![ImportEnvironment {
                name: "Dev".into(),
                variables: vec![("TAG".into(), "{{$isoTimestamp}}".into())],
            }]
        );

        let [
            ImportItem::Folder {
                name: ws_folder,
                settings: ws_settings,
                children,
            },
        ] = &bundle.items[..]
        else {
            panic!("expected workspace header wrapper folder: {:?}", bundle.items);
        };
        assert_eq!(ws_folder, "Shop Workspace");
        assert_eq!(ws_settings.headers, [kv("X-Workspace", "{{uuid()}}", true)]);

        let [
            ImportItem::Api { name: health_name, .. },
            ImportItem::Folder {
                name: users_name,
                settings: users_settings,
                children: users_children,
            },
        ] = &children[..]
        else {
            panic!("unexpected workspace children: {children:?}");
        };
        assert_eq!(health_name, "Health");
        assert_eq!(users_name, "Users");
        assert_eq!(users_settings.description, "User group");
        assert_eq!(
            users_settings.auth,
            Auth::Basic {
                username: "u".into(),
                password: "{{pw}}".into()
            }
        );

        let [
            ImportItem::Api {
                name: get_name,
                request: get_req,
            },
            ImportItem::Folder {
                name: admin_name,
                settings: admin_settings,
                ..
            },
        ] = &users_children[..]
        else {
            panic!("unexpected users children: {users_children:?}");
        };
        assert_eq!(get_name, "Get User");
        assert_eq!(get_req.uri, "{{BASE_URL}}/users/:id");
        assert_eq!(get_req.variables, [kv("id", "{{user_id}}", true)]);
        assert_eq!(
            get_req.query,
            [kv("expand", "roles", false), kv("ts", "{{timestamp()}}", true),]
        );
        assert_eq!(get_req.description, "Fetch by {{user_id}}");
        assert_eq!(admin_name, "Admin");
        assert_eq!(admin_settings.auth, Auth::None);

        assert_eq!(
            bundle.lost,
            vec![Lost {
                target: "Shop Workspace / Users / Get User".into(),
                kind: LostKind::Template {
                    expr: "${[ response.body.path('$.id') ]}".into(),
                },
            }]
        );
    }

    #[test]
    fn bodies_auth_websocket_and_grpc() {
        let doc = json!({
            "yaakSchema": 4,
            "resources": {
                "workspaces": [{ "id": "w1", "name": "All Types" }],
                "httpRequests": [
                    {
                        "id": "r1",
                        "workspaceId": "w1",
                        "name": "Multipart + ApiKey",
                        "method": "POST",
                        "url": "https://a.test/upload",
                        "bodyType": "multipart/form-data",
                        "body": {
                            "form": [
                                { "name": "note", "value": "${[ title ]}", "enabled": true },
                                { "name": "doc", "file": "/tmp/a.pdf", "enabled": true },
                                { "name": "blank_file", "type": "file", "file": "", "enabled": true }
                            ]
                        },
                        "authenticationType": "apikey",
                        "authentication": { "name": "x-key", "value": "${[ k ]}", "location": "query" }
                    },
                    {
                        "id": "r2",
                        "workspaceId": "w1",
                        "name": "Binary + Aws",
                        "method": "PUT",
                        "url": "https://s3.test/obj",
                        "bodyType": "binary",
                        "body": { "filePath": "/tmp/blob.bin" },
                        "authenticationType": "awsv4",
                        "authentication": {
                            "accessKeyId": "AKIA1",
                            "secretKey": "SEC1",
                            "sessionToken": "TOK1",
                            "region": "us-east-1",
                            "service": "s3"
                        }
                    },
                    {
                        "id": "r3",
                        "workspaceId": "w1",
                        "name": "GraphQL + Jwt",
                        "method": "POST",
                        "url": "https://a.test/gql",
                        "bodyType": "graphql",
                        "body": { "text": "{\"query\":\"query { me { id } }\",\"variables\":{\"id\":1}}" },
                        "authenticationType": "jwt",
                        "authentication": {
                            "algorithm": "HS512",
                            "secret": "${[ jwt_key ]}",
                            "payload": "{\"sub\":\"1\"}",
                            "locationPrefix": ""
                        }
                    },
                    {
                        "id": "r4",
                        "workspaceId": "w1",
                        "name": "Form + OAuthImplicit",
                        "method": "POST",
                        "url": "https://a.test/form",
                        "bodyType": "application/x-www-form-urlencoded",
                        "body": { "form": [{ "name": "a", "value": "1", "enabled": true }] },
                        "authenticationType": "oauth2",
                        "authentication": {
                            "grantType": "implicit",
                            "authorizationUrl": "https://id.test/auth",
                            "redirectUri": "http://127.0.0.1:8787/cb",
                            "clientId": "app"
                        }
                    },
                    {
                        "id": "r5",
                        "workspaceId": "w1",
                        "name": "Unsupported",
                        "method": "POST",
                        "url": "https://a.test/bad",
                        "bodyType": "avro",
                        "authenticationType": "ntlm"
                    }
                ],
                "websocketRequests": [
                    {
                        "id": "ws1",
                        "workspaceId": "w1",
                        "name": "Stream",
                        "url": "wss://a.test/ws",
                        "message": "{\"hello\":\"${[ name ]}\"}",
                        "headers": [{ "name": "X-Ws", "value": "1", "enabled": true }]
                    }
                ],
                "grpcRequests": [
                    {
                        "id": "g1",
                        "workspaceId": "w1",
                        "name": "SayHello",
                        "url": "https://a.test:50051",
                        "service": "greet.Greeter",
                        "method": "SayHello",
                        "message": "{\"name\":\"${[ person ]}\"}",
                        "metadata": [{ "name": "x-meta", "value": "val", "enabled": true }]
                    }
                ]
            }
        });

        let bundle = parse(&doc);
        assert_eq!(bundle.items.len(), 7);

        let ImportItem::Api { request: r1, .. } = &bundle.items[0] else {
            panic!("expected r1");
        };
        assert_eq!(r1.content_type, ContentType::Multipart);
        let fields: Vec<KvParam> = serde_json::from_str(&r1.body).expect("multipart json");
        assert_eq!(
            fields,
            [
                kv("note", "{{title}}", true),
                kv("doc", "file:///tmp/a.pdf", true),
                kv("blank_file", "file://", true),
            ]
        );
        assert_eq!(
            r1.auth,
            Auth::ApiKey {
                name: "x-key".into(),
                value: "{{k}}".into(),
                place: KeyPlace::Query
            }
        );

        let ImportItem::Api { request: r2, .. } = &bundle.items[1] else {
            panic!("expected r2");
        };
        assert_eq!(r2.content_type, ContentType::Binary);
        assert_eq!(r2.body, "file:///tmp/blob.bin");
        assert_eq!(
            r2.auth,
            Auth::from(AwsSigV4 {
                access_key: "AKIA1".into(),
                secret_key: "SEC1".into(),
                session_token: "TOK1".into(),
                region: "us-east-1".into(),
                service: "s3".into(),
            })
        );

        let ImportItem::Api { request: r3, .. } = &bundle.items[2] else {
            panic!("expected r3");
        };
        assert_eq!(r3.content_type, ContentType::GraphQl);
        assert_eq!(GraphQlBody::read(&r3.body).query, "query { me { id } }");
        assert_eq!(
            r3.auth,
            Auth::from(JwtBearer {
                algorithm: JwtAlgorithm::HS512,
                secret: "{{jwt_key}}".into(),
                secret_base64: false,
                payload: r#"{"sub":"1"}"#.into(),
                header: String::new(),
                place: KeyPlace::Header,
                prefix: String::new(),
                query_param: "token".into(),
            })
        );

        let ImportItem::Api { request: r4, .. } = &bundle.items[3] else {
            panic!("expected r4");
        };
        assert_eq!(r4.content_type, ContentType::Form);
        assert_eq!(
            r4.auth,
            Auth::from(OAuth2 {
                grant: Grant::Implicit,
                authorization_url: "https://id.test/auth".into(),
                redirect_port: 8787,
                client_id: "app".into(),
                client_auth: ClientAuth::Header,
                ..Default::default()
            })
        );

        let ImportItem::Api { request: ws, .. } = &bundle.items[5] else {
            panic!("expected ws");
        };
        assert_eq!(ws.method, HttpMethod::WebSocket);
        assert_eq!(ws.uri, "wss://a.test/ws");
        assert_eq!(ws.body, r#"{"hello":"{{name}}"}"#);

        let ImportItem::Api { request: grpc, .. } = &bundle.items[6] else {
            panic!("expected grpc");
        };
        assert_eq!(grpc.method, HttpMethod::Grpc);
        assert_eq!(grpc.headers, [kv("x-meta", "val", true)]);
        let grpc_body = GrpcBody::read(&grpc.body);
        assert_eq!(grpc_body.service, "greet.Greeter");
        assert_eq!(grpc_body.method, "SayHello");
        assert_eq!(grpc_body.message, r#"{"name":"{{person}}"}"#);

        assert_eq!(
            bundle.lost,
            vec![
                Lost {
                    target: "Unsupported".into(),
                    kind: LostKind::Body { kind: "avro".into() },
                },
                Lost {
                    target: "Unsupported".into(),
                    kind: LostKind::Auth { kind: "ntlm".into() },
                },
            ]
        );
    }

    #[test]
    fn multi_workspace_export() {
        let doc = json!({
            "yaakVersion": "2025.4.0",
            "yaakSchema": 4,
            "resources": {
                "workspaces": [
                    { "id": "w1", "name": "Alpha", "authenticationType": "bearer", "authentication": { "token": "t1" } },
                    { "id": "w2", "name": "Beta" }
                ],
                "httpRequests": [
                    { "id": "r1", "workspaceId": "w1", "name": "One", "method": "GET", "url": "https://a.test/1" },
                    { "id": "r2", "workspaceId": "w2", "name": "Two", "method": "POST", "url": "https://b.test/2" }
                ]
            }
        });
        assert_eq!(name(&doc), None);
        let bundle = parse(&doc);
        assert_eq!(bundle.items.len(), 2);
        let ImportItem::Folder {
            name: first_name,
            settings: first_settings,
            children: first_children,
        } = &bundle.items[0]
        else {
            panic!("expected Alpha folder");
        };
        assert_eq!(first_name, "Alpha");
        assert_eq!(first_settings.auth, Auth::Bearer { token: "t1".into() });
        assert_eq!(first_children.len(), 1);
    }
}
