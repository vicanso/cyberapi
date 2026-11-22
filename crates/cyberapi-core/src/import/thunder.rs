//! R12 (1/3): Thunder Client collection (v1.1 / v1.2+) and environment importer.
//!
//! Imports folders (`containerId`, `sortNum`), requests (method, URL, `baseUrl`,
//! query & path params, headers, raw/form/multipart/GraphQL/binary bodies, auth
//! including Basic/Bearer/API key/Digest/OAuth 2.0/AWS SigV4/JWT Bearer),
//! declarative `tests` assertions, `set-env-var` extractions, and environments.
//! Inline scripts and unsupported test rules are reported via [`lost`].

use super::lost::{self, Lost, ScriptPhase};
use super::{ImportBundle, ImportEnvironment, ImportItem, api, content_type_of, guess_content_type, kv};
use crate::assertions::{self, Assertion, Extraction, Op, Subject};
use crate::auth::{Auth, KeyPlace};
use crate::aws_sigv4::AwsSigV4;
use crate::graphql::GraphQlBody;
use crate::jwt_bearer::{JwtAlgorithm, JwtBearer};
use crate::model::{ContentType, FolderSettings, HttpMethod, KvParam};
use crate::oauth::{ClientAuth, Grant, OAuth2};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

fn is_thunder_client(value: &Value) -> bool {
    ["clientName", "client"].into_iter().any(|key| {
        value
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|text| text.trim().eq_ignore_ascii_case("Thunder Client"))
    })
}

fn col_name(value: &Value) -> Option<&str> {
    ["collectionName", "colName"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
}

/// Whether `json` is a Thunder Client collection export (single object or array of them).
pub(super) fn is_collection(json: &Value) -> bool {
    let one = |value: &Value| {
        let Some(map) = value.as_object() else {
            return false;
        };
        let has_name = col_name(value).is_some();
        let has_requests = map.get("requests").is_some_and(Value::is_array);
        (is_thunder_client(value) && (has_name || has_requests)) || (has_name && has_requests)
    };
    match json {
        Value::Array(items) => !items.is_empty() && items.iter().all(one),
        other => one(other),
    }
}

/// Whether `json` is a Thunder Client environment export (single object or array of them).
pub(super) fn is_environment(json: &Value) -> bool {
    let one = |value: &Value| {
        let Some(map) = value.as_object() else {
            return false;
        };
        !map.contains_key("requests")
            && !map.contains_key("collectionName")
            && !map.contains_key("colName")
            && map.get("environmentName").and_then(Value::as_str).is_some()
            && (is_thunder_client(value)
                || map.get("data").is_some_and(Value::is_array)
                || map.get("variables").is_some_and(Value::is_array))
    };
    match json {
        Value::Array(items) => !items.is_empty() && items.iter().all(one),
        other => one(other),
    }
}

/// The collection's own name, if importing a single Thunder Client collection.
pub(super) fn name(json: &Value) -> Option<&str> {
    match json {
        Value::Array(collections) if collections.len() == 1 => collections.first().and_then(col_name),
        Value::Array(_) => None,
        collection => col_name(collection),
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

fn is_disabled(value: &Value) -> bool {
    bool_flag(value, "isDisabled") == Some(true)
        || bool_flag(value, "disabled") == Some(true)
        || bool_flag(value, "enabled") == Some(false)
}

fn sort_num(value: &Value) -> f64 {
    value
        .get("sortNum")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|n| n as f64)))
        .unwrap_or(f64::MAX)
}

fn variable_list(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| !is_disabled(item))
                .filter_map(|item| {
                    let key = ["name", "key"]
                        .into_iter()
                        .map(|k| text(item, k).trim())
                        .find(|k| !k.is_empty())?
                        .to_string();
                    let val = value_text(item.get("value"));
                    Some((key, val))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parses one or an array of Thunder Client environments.
pub(super) fn parse_environment(json: &Value) -> ImportBundle {
    let list: Vec<&Value> = match json {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    let environments = list
        .into_iter()
        .filter_map(|env| {
            let env_name = text(env, "environmentName").trim().to_string();
            if env_name.is_empty() {
                return None;
            }
            let vars = variable_list(env.get("data").or_else(|| env.get("variables")));
            Some(ImportEnvironment {
                name: env_name,
                variables: vars,
            })
        })
        .collect();
    ImportBundle {
        environments,
        ..Default::default()
    }
}

pub(super) fn parse(json: &Value) -> ImportBundle {
    let mut lost = Vec::new();
    match json {
        Value::Array(collections) if collections.len() > 1 => {
            let mut items = Vec::with_capacity(collections.len());
            let mut environment = Vec::new();
            for collection in collections {
                let col_label = col_name(collection).unwrap_or("Collection").trim().to_string();
                let col_label = if col_label.is_empty() {
                    "Collection".to_string()
                } else {
                    col_label
                };
                let (folder_settings, children, col_env) = parse_collection(collection, &col_label, &mut lost);
                items.push(ImportItem::Folder {
                    name: col_label,
                    children,
                    settings: folder_settings,
                });
                environment.extend(col_env);
            }
            ImportBundle {
                items,
                environment,
                lost,
                ..Default::default()
            }
        }
        Value::Array(collections) => collections
            .first()
            .map(|first| parse_single(first, &mut lost))
            .unwrap_or_default(),
        collection => parse_single(collection, &mut lost),
    }
}

fn parse_single(collection: &Value, lost: &mut Vec<Lost>) -> ImportBundle {
    let (settings, mut items, environment) = parse_collection(collection, "", lost);
    if !settings.headers.is_empty() {
        let name = col_name(collection)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or("Collection")
            .to_string();
        items = vec![ImportItem::Folder {
            name,
            children: items,
            settings: FolderSettings {
                auth: Auth::Inherit,
                headers: settings.headers,
                ..Default::default()
            },
        }];
    }
    ImportBundle {
        items,
        environment,
        auth: settings.auth,
        lost: std::mem::take(lost),
        ..Default::default()
    }
}

fn kv_headers(value: Option<&Value>) -> Vec<KvParam> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let key = ["name", "key"]
                        .into_iter()
                        .map(|k| text(item, k).trim())
                        .find(|k| !k.is_empty())?;
                    Some(kv(key, &value_text(item.get("value")), !is_disabled(item)))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_base_url(value: &Value) -> String {
    value
        .pointer("/settings/options/baseUrl")
        .or_else(|| value.pointer("/settings/baseUrl"))
        .or_else(|| value.pointer("/options/baseUrl"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn folder_settings(folder_val: &Value, target: &str, lost: &mut Vec<Lost>) -> (FolderSettings, Option<String>) {
    let settings_val = folder_val.get("settings");
    let auth = settings_val
        .and_then(|s| s.get("auth"))
        .or_else(|| folder_val.get("auth"))
        .and_then(|v| auth(v, target, lost))
        .unwrap_or_default();
    let headers = kv_headers(
        settings_val
            .and_then(|s| s.get("headers"))
            .or_else(|| folder_val.get("headers")),
    );
    let description = ["docs", "description"]
        .into_iter()
        .map(|k| text(folder_val, k).trim())
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .to_string();

    if let Some(settings_obj) = settings_val {
        collect_script_blocks(settings_obj, target, lost);
        if let Some(tests_val) = settings_obj.get("tests") {
            let mut dummy_assertions = Vec::new();
            let mut dummy_extractions = Vec::new();
            parse_tests(tests_val, target, &mut dummy_assertions, &mut dummy_extractions, lost);
        }
    }
    collect_script_blocks(folder_val, target, lost);

    let base_url = extract_base_url(folder_val);
    let own_base = (!base_url.is_empty()).then_some(base_url);
    (
        FolderSettings {
            auth,
            headers,
            description,
            variables: Vec::new(),
            source: None,
        },
        own_base,
    )
}

fn parse_collection(
    collection: &Value,
    root_prefix: &str,
    lost: &mut Vec<Lost>,
) -> (FolderSettings, Vec<ImportItem>, Vec<(String, String)>) {
    let col_settings_val = collection.get("settings");
    let root_auth = col_settings_val
        .and_then(|s| s.get("auth"))
        .or_else(|| collection.get("auth"))
        .and_then(|v| auth(v, root_prefix, lost))
        .unwrap_or_default();
    let root_headers = kv_headers(
        col_settings_val
            .and_then(|s| s.get("headers"))
            .or_else(|| collection.get("headers")),
    );
    if let Some(settings_obj) = col_settings_val {
        collect_script_blocks(settings_obj, root_prefix, lost);
        if let Some(tests_val) = settings_obj.get("tests") {
            let mut dummy_assertions = Vec::new();
            let mut dummy_extractions = Vec::new();
            parse_tests(
                tests_val,
                root_prefix,
                &mut dummy_assertions,
                &mut dummy_extractions,
                lost,
            );
        }
    }
    collect_script_blocks(collection, root_prefix, lost);

    let root_base_url = extract_base_url(collection);

    let raw_folders = collection
        .get("folders")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut folders: Vec<(usize, &Value)> = raw_folders.iter().enumerate().collect();
    folders.sort_by(|(idx_a, a), (idx_b, b)| sort_num(a).total_cmp(&sort_num(b)).then_with(|| idx_a.cmp(idx_b)));

    let raw_requests = collection
        .get("requests")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut requests: Vec<(usize, &Value)> = raw_requests.iter().enumerate().collect();
    requests.sort_by(|(idx_a, a), (idx_b, b)| sort_num(a).total_cmp(&sort_num(b)).then_with(|| idx_a.cmp(idx_b)));

    let folder_id_set: HashSet<&str> = folders
        .iter()
        .map(|(_, f)| text(f, "_id").trim())
        .filter(|id| !id.is_empty())
        .collect();

    #[derive(Clone, Copy)]
    enum Entry<'a> {
        Folder(usize, &'a Value),
        Request(&'a Value),
    }

    let mut children_by_parent: HashMap<&str, Vec<(f64, u8, usize, Entry<'_>)>> = HashMap::new();
    for (sorted_idx, (_, f)) in folders.iter().enumerate() {
        let id = text(f, "_id").trim();
        let container = text(f, "containerId").trim();
        let parent = if !container.is_empty() && container != id && folder_id_set.contains(container) {
            container
        } else {
            ""
        };
        children_by_parent
            .entry(parent)
            .or_default()
            .push((sort_num(f), 0, sorted_idx, Entry::Folder(sorted_idx, f)));
    }
    for (sorted_idx, (_, req)) in requests.iter().enumerate() {
        let container = text(req, "containerId").trim();
        let parent = if !container.is_empty() && folder_id_set.contains(container) {
            container
        } else {
            ""
        };
        children_by_parent
            .entry(parent)
            .or_default()
            .push((sort_num(req), 1, sorted_idx, Entry::Request(req)));
    }
    for entries in children_by_parent.values_mut() {
        entries.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
    }

    struct WalkCx<'a> {
        children_by_parent: &'a HashMap<&'a str, Vec<(f64, u8, usize, Entry<'a>)>>,
        visiting: HashSet<&'a str>,
        visited_folders: Vec<bool>,
        lost: &'a mut Vec<Lost>,
    }

    fn build_list<'a>(parent_id: &str, prefix: &str, base_url: &str, cx: &mut WalkCx<'a>) -> Vec<ImportItem> {
        let Some(entries) = cx.children_by_parent.get(parent_id) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(entries.len());
        for (_, _, _, entry) in entries {
            match *entry {
                Entry::Folder(folder_idx, folder_val) => {
                    if let Some(item) = visit_folder(folder_idx, folder_val, prefix, base_url, cx) {
                        out.push(item);
                    }
                }
                Entry::Request(req_val) => {
                    out.push(request(req_val, prefix, base_url, cx.lost));
                }
            }
        }
        out
    }

    fn visit_folder<'a>(
        folder_idx: usize,
        folder_val: &'a Value,
        prefix: &str,
        parent_base_url: &str,
        cx: &mut WalkCx<'a>,
    ) -> Option<ImportItem> {
        if cx.visited_folders[folder_idx] {
            return None;
        }
        let id = text(folder_val, "_id").trim();
        if !id.is_empty() && !cx.visiting.insert(id) {
            return None;
        }
        cx.visited_folders[folder_idx] = true;

        let folder_name = text(folder_val, "name").trim();
        let name = if folder_name.is_empty() {
            "Folder".to_string()
        } else {
            folder_name.to_string()
        };
        let target = lost::join_path(prefix, &name);
        let (settings, own_base) = folder_settings(folder_val, &target, cx.lost);
        let effective_base = own_base.as_deref().unwrap_or(parent_base_url);
        let children = if id.is_empty() {
            Vec::new()
        } else {
            build_list(id, &target, effective_base, cx)
        };

        if !id.is_empty() {
            cx.visiting.remove(id);
        }
        Some(ImportItem::Folder {
            name,
            children,
            settings,
        })
    }

    let mut cx = WalkCx {
        children_by_parent: &children_by_parent,
        visiting: HashSet::new(),
        visited_folders: vec![false; folders.len()],
        lost,
    };

    let mut items = build_list("", root_prefix, &root_base_url, &mut cx);
    // Any disconnected folder cycle still gets emitted safely at root without hanging.
    for (sorted_idx, (_, folder_val)) in folders.iter().enumerate() {
        if !cx.visited_folders[sorted_idx]
            && let Some(item) = visit_folder(sorted_idx, folder_val, root_prefix, &root_base_url, &mut cx)
        {
            items.push(item);
        }
    }

    let collection_env = variable_list(collection.get("variables"));
    (
        FolderSettings {
            auth: root_auth,
            headers: root_headers,
            description: String::new(),
            variables: Vec::new(),
            source: None,
        },
        items,
        collection_env,
    )
}

fn join_base_url(base_url: &str, url: &str) -> String {
    let url = url.trim();
    let base = base_url.trim();
    if base.is_empty() || url.is_empty() || url.contains("://") || url.starts_with("{{") {
        return url.to_string();
    }
    if url.starts_with('?') {
        format!("{}{url}", base.trim_end_matches('/'))
    } else {
        format!("{}/{}", base.trim_end_matches('/'), url.trim_start_matches('/'))
    }
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

fn request(value: &Value, prefix: &str, base_url: &str, lost: &mut Vec<Lost>) -> ImportItem {
    let effective_url = join_base_url(base_url, text(value, "url"));
    let (name, mut built) = api(text(value, "name"), text(value, "method"), &effective_url);
    let target = lost::join_path(prefix, &name);

    built.description = ["docs", "description"]
        .into_iter()
        .map(|k| text(value, k).trim())
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .to_string();

    built.headers = kv_headers(value.get("headers"));

    // Split path parameters (`isPath: true`) from query params; deduplicate query rows
    // against query parameters parsed out of the URL.
    let mut param_query = Vec::new();
    let mut param_keys: HashSet<String> = HashSet::new();
    if let Some(params) = value.get("params").and_then(Value::as_array) {
        for param in params {
            let key = ["name", "key"]
                .into_iter()
                .map(|k| text(param, k).trim())
                .find(|k| !k.is_empty());
            let Some(key) = key else {
                continue;
            };
            let val = value_text(param.get("value"));
            let enabled = !is_disabled(param);
            if bool_flag(param, "isPath") == Some(true) {
                built.variables.push(kv(key, &val, enabled));
            } else {
                param_keys.insert(key.to_string());
                param_query.push(kv(key, &val, enabled));
            }
        }
    }
    for url_param in built.query {
        if !param_keys.contains(&url_param.key) {
            param_query.push(url_param);
        }
    }
    built.query = param_query;

    if let Some(body_val) = value.get("body") {
        let body_type = text(body_val, "type").trim().to_ascii_lowercase();
        let raw = text(body_val, "raw");
        match body_type.as_str() {
            "" | "none" => {
                if !raw.trim().is_empty() {
                    built.body = raw.to_string();
                    built.content_type = guess_content_type(&built.body);
                }
            }
            "json" => {
                built.content_type = ContentType::Json;
                built.body = raw.to_string();
            }
            "xml" => {
                built.content_type = ContentType::Xml;
                built.body = raw.to_string();
            }
            "text" => {
                built.content_type = ContentType::Plain;
                built.body = raw.to_string();
            }
            "html" => {
                built.content_type = ContentType::Plain;
                built.body = raw.to_string();
                if !has_header(&built.headers, "content-type") {
                    built.headers.push(kv("Content-Type", "text/html", true));
                }
            }
            "formencoded" | "urlencoded" => {
                built.content_type = ContentType::Form;
                let fields = kv_headers(body_val.get("form"));
                built.body = serde_json::to_string(&fields).unwrap_or_default();
            }
            "formdata" => {
                built.content_type = ContentType::Multipart;
                let mut fields = kv_headers(body_val.get("form"));
                if let Some(files) = body_val.get("files").and_then(Value::as_array) {
                    for file in files {
                        let key = ["name", "key"]
                            .into_iter()
                            .map(|k| text(file, k).trim())
                            .find(|k| !k.is_empty());
                        if let Some(key) = key {
                            let file_val = normalize_file_value(&value_text(file.get("value")));
                            fields.push(kv(key, &file_val, !is_disabled(file)));
                        }
                    }
                }
                built.body = serde_json::to_string(&fields).unwrap_or_default();
            }
            "graphql" => {
                built.content_type = ContentType::GraphQl;
                let gql = body_val.get("graphql").unwrap_or(&Value::Null);
                let variables = match gql.get("variables") {
                    Some(Value::String(text)) => text.clone(),
                    Some(other) if !other.is_null() => crate::format::json(other),
                    _ => String::new(),
                };
                built.body = GraphQlBody {
                    query: text(gql, "query").to_string(),
                    variables,
                    operation_name: String::new(),
                }
                .to_body();
            }
            "binary" => {
                built.content_type = ContentType::Binary;
                let binary_path = match body_val.get("binary") {
                    Some(Value::String(path)) if !path.trim().is_empty() => path.clone(),
                    Some(Value::Object(map)) => ["file", "path", "src", "value"]
                        .into_iter()
                        .filter_map(|k| map.get(k).and_then(Value::as_str))
                        .find(|s| !s.trim().is_empty())
                        .unwrap_or_default()
                        .to_string(),
                    _ => body_val
                        .get("files")
                        .and_then(Value::as_array)
                        .and_then(|files| {
                            files
                                .iter()
                                .filter(|file| !is_disabled(file))
                                .filter_map(|file| file.get("value").and_then(Value::as_str))
                                .find(|s| !s.trim().is_empty())
                        })
                        .unwrap_or_default()
                        .to_string(),
                };
                built.body = normalize_file_value(&binary_path);
            }
            _ => {
                if let Some(mime) = content_type_of(&body_type) {
                    built.content_type = mime;
                    built.body = raw.to_string();
                } else {
                    lost::push_body(lost, &target, &body_type);
                }
            }
        }
    }

    if !built.method.has_body() && built.method != HttpMethod::Get {
        built.content_type = ContentType::default();
    }

    if let Some(own_auth) = value.get("auth").and_then(|v| auth(v, &target, lost)) {
        built.auth = own_auth;
    }

    collect_script_blocks(value, &target, lost);
    if let Some(tests_val) = value.get("tests") {
        parse_tests(tests_val, &target, &mut built.assertions, &mut built.extractions, lost);
    }

    ImportItem::Api {
        name,
        request: Box::new(built),
    }
}

fn auth_field(value: &Value, kind_key: &str, keys: &[&str]) -> Option<String> {
    for candidate in keys {
        if let Some(raw @ (Value::String(_) | Value::Number(_) | Value::Bool(_))) = value.get(*candidate) {
            return Some(value_text(Some(raw)));
        }
    }
    if let Some(nested) = value.get(kind_key) {
        match nested {
            Value::Object(_) => {
                for candidate in keys {
                    if let Some(raw @ (Value::String(_) | Value::Number(_) | Value::Bool(_))) = nested.get(*candidate) {
                        return Some(value_text(Some(raw)));
                    }
                }
            }
            Value::Array(entries) => {
                for candidate in keys {
                    if let Some(found) = entries.iter().find(|entry| {
                        let key = text(entry, "key");
                        let name = text(entry, "name");
                        (key.eq_ignore_ascii_case(candidate) || name.eq_ignore_ascii_case(candidate))
                            && !is_disabled(entry)
                    }) {
                        return Some(value_text(found.get("value")));
                    }
                }
            }
            Value::String(text) if keys.contains(&"token") || keys.contains(&kind_key) => {
                return Some(text.clone());
            }
            _ => {}
        }
    }
    None
}

fn auth(value: &Value, target: &str, lost: &mut Vec<Lost>) -> Option<Auth> {
    if is_disabled(value) {
        return Some(Auth::None);
    }
    let kind_raw = text(value, "type").trim();
    let kind = kind_raw.to_ascii_lowercase();
    let opt = |kind_key: &str, keys: &[&str]| auth_field(value, kind_key, keys);
    let field = |kind_key: &str, keys: &[&str]| opt(kind_key, keys).unwrap_or_default();

    Some(match kind.as_str() {
        "" | "inherit" => Auth::Inherit,
        "none" | "noauth" => Auth::None,
        "basic" => Auth::Basic {
            username: field("basic", &["username", "user"]),
            password: field("basic", &["password", "pass"]),
        },
        "bearer" => Auth::Bearer {
            token: field("bearer", &["token", "bearer"]),
        },
        "apikey" | "api-key" => {
            let place_raw = field("apikey", &["in", "addTo", "place"]).to_ascii_lowercase();
            let place = if place_raw.contains("query") {
                KeyPlace::Query
            } else {
                KeyPlace::Header
            };
            Auth::ApiKey {
                name: field("apikey", &["key", "name"]),
                value: field("apikey", &["value", "token"]),
                place,
            }
        }
        "digest" => Auth::Digest {
            username: field("digest", &["username", "user"]),
            password: field("digest", &["password", "pass"]),
        },
        "aws" | "awsv4" => Auth::from(AwsSigV4 {
            access_key: field("aws", &["accessKey", "accessKeyId"]),
            secret_key: field("aws", &["secretKey", "secretAccessKey"]),
            session_token: field("aws", &["sessionToken"]),
            region: field("aws", &["region"]),
            service: field("aws", &["service"]),
        }),
        "jwt" => {
            let raw_alg = field("jwt", &["algorithm", "alg"]);
            let algorithm = if raw_alg.trim().is_empty() {
                JwtAlgorithm::HS256
            } else if let Some(alg) = JwtAlgorithm::parse(&raw_alg) {
                alg
            } else {
                lost::push_auth(lost, target, &format!("jwt ({})", raw_alg.trim()));
                return None;
            };
            let payload = {
                let raw = field("jwt", &["payload"]);
                if raw.trim().is_empty() { "{}".into() } else { raw }
            };
            let header = opt("jwt", &["header", "headers"]).unwrap_or_default();
            let place_raw = field("jwt", &["addTokenTo", "place", "in"]).to_ascii_lowercase();
            let place = if place_raw.contains("query") {
                KeyPlace::Query
            } else {
                KeyPlace::Header
            };
            let prefix = opt("jwt", &["headerPrefix", "prefix"]).unwrap_or_else(|| "Bearer".into());
            let query_param = {
                let q = field("jwt", &["queryParamKey", "queryParam", "query_param"]);
                if q.trim().is_empty() { "token".into() } else { q }
            };
            let secret_base64 = field("jwt", &["isSecretBase64Encoded", "secretBase64"])
                .trim()
                .eq_ignore_ascii_case("true");
            Auth::from(JwtBearer {
                algorithm,
                secret: field("jwt", &["secret", "privateKey"]),
                secret_base64,
                payload,
                header,
                place,
                prefix,
                query_param,
            })
        }
        "oauth2" => {
            let grant_raw = field("oauth2", &["grantType", "grant_type"]);
            let grant_norm = grant_raw.trim().to_ascii_lowercase().replace(['_', '-', ' '], "");
            let grant = match grant_norm.as_str() {
                "" | "authorizationcode" | "authorizationcodewithpkce" | "pkce" => Grant::AuthorizationCode,
                "clientcredentials" => Grant::ClientCredentials,
                "password" | "passwordcredentials" => Grant::Password,
                "implicit" => Grant::Implicit,
                _ => {
                    let kind = format!("oauth2 ({})", grant_raw.trim());
                    lost::push_auth(lost, target, &kind);
                    return None;
                }
            };
            let redirect_port = ["callbackUrl", "redirectUri", "redirect_uri", "callback"]
                .into_iter()
                .filter_map(|k| opt("oauth2", &[k]))
                .find_map(|uri| {
                    let port = crate::oauth::redirect_port_of(&uri);
                    (port != 0).then_some(port)
                })
                .unwrap_or(0);
            let client_auth_raw =
                field("oauth2", &["clientAuth", "credentialsIn", "client_authentication"]).to_ascii_lowercase();
            let client_auth = if client_auth_raw.contains("body") {
                ClientAuth::Body
            } else {
                ClientAuth::Header
            };
            Auth::from(OAuth2 {
                grant,
                token_url: field("oauth2", &["tokenUrl", "accessTokenUrl"]),
                authorization_url: field("oauth2", &["authUrl", "authorizationUrl"]),
                redirect_port,
                client_id: field("oauth2", &["clientId"]),
                client_secret: field("oauth2", &["clientSecret"]),
                username: field("oauth2", &["username"]),
                password: field("oauth2", &["password"]),
                scope: field("oauth2", &["scope", "scopes"]),
                audience: field("oauth2", &["audience"]),
                resource: field("oauth2", &["resource"]),
                client_auth,
            })
        }
        _ => {
            lost::push_auth(lost, target, kind_raw);
            return None;
        }
    })
}

fn collect_script_value(value: &Value, target: &str, phase: ScriptPhase, lost: &mut Vec<Lost>) {
    match value {
        Value::String(script) => lost::push_script(lost, target, phase, script),
        Value::Object(map) => {
            if let Some(script) = map.get("script").and_then(Value::as_str) {
                lost::push_script(lost, target, phase, script);
            }
            if let Some(inline) = map.get("inlineScripts") {
                collect_inline_scripts(inline, target, phase, lost);
            }
            if let Some(run_reqs) = map.get("runRequests").and_then(Value::as_array)
                && !run_reqs.is_empty()
                && let Ok(pretty) = serde_json::to_string_pretty(&Value::Array(run_reqs.clone()))
            {
                lost::push_script(lost, target, phase, &format!("// runRequests\n{pretty}"));
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_script_value(item, target, phase, lost);
            }
        }
        _ => {}
    }
}

fn collect_inline_scripts(value: &Value, target: &str, phase: ScriptPhase, lost: &mut Vec<Lost>) {
    let Some(items) = value.as_array() else {
        return;
    };
    for item in items {
        if is_disabled(item) {
            continue;
        }
        match item {
            Value::String(script) => lost::push_script(lost, target, phase, script),
            Value::Object(_) => {
                let script = text(item, "script");
                if !script.trim().is_empty() {
                    lost::push_script(lost, target, phase, script);
                } else if let Some(val) = item.get("value").and_then(Value::as_str) {
                    lost::push_script(lost, target, phase, val);
                } else if let Ok(json_text) = serde_json::to_string(item) {
                    lost::push_script(lost, target, phase, &json_text);
                }
            }
            _ => {}
        }
    }
}

fn collect_script_blocks(container: &Value, target: &str, lost: &mut Vec<Lost>) {
    for key in ["preReq", "preRequestScript"] {
        if let Some(val) = container.get(key) {
            collect_script_value(val, target, ScriptPhase::PreRequest, lost);
        }
    }
    if let Some(val) = container.get("postReq") {
        collect_script_value(val, target, ScriptPhase::PostResponse, lost);
    }
}

fn normalize_json_query_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with('$') {
        return trimmed.to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("json") {
        if rest.is_empty() {
            return "$".to_string();
        }
        if rest.starts_with(['.', '[']) {
            return format!("${rest}");
        }
    }
    if trimmed.starts_with('[') {
        format!("${trimmed}")
    } else {
        format!("$.{}", trimmed.trim_start_matches('.'))
    }
}

fn map_op(raw_action: &str) -> Option<Op> {
    match raw_action
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-', ' '], "")
        .as_str()
    {
        "equal" | "equals" | "eq" | "==" | "=" => Some(Op::Equals),
        "notequal" | "notequals" | "ne" | "!=" => Some(Op::NotEquals),
        "contains" | "contain" => Some(Op::Contains),
        "notcontains" | "doesnotcontain" => Some(Op::NotContains),
        "matches" | "match" | "regex" => Some(Op::Matches),
        "<" | "lt" | "less" | "lessthan" => Some(Op::Less),
        ">" | "gt" | "greater" | "greaterthan" => Some(Op::Greater),
        "istype" | "type" => Some(Op::IsType),
        "exists" | "exist" | "notnull" => Some(Op::Exists),
        "notexists" | "notexist" | "isnull" => Some(Op::NotExists),
        _ => None,
    }
}

fn extract_source(raw: &str) -> Option<(Subject, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if trimmed == "json" || trimmed.starts_with("json.") || trimmed.starts_with("json[") || trimmed.starts_with('$') {
        return Some((Subject::Json, normalize_json_query_path(trimmed)));
    }
    if let Some(header) = trimmed
        .strip_prefix("header.")
        .or_else(|| trimmed.strip_prefix("headers."))
    {
        let header = header.trim();
        if !header.is_empty() {
            return Some((Subject::Header, header.to_string()));
        }
    }
    if matches!(lower.as_str(), "text" | "res-body" | "body") {
        return Some((Subject::Body, String::new()));
    }
    if matches!(lower.as_str(), "res-code" | "status") {
        return Some((Subject::Status, String::new()));
    }
    if matches!(lower.as_str(), "res-time" | "latency") {
        return Some((Subject::Latency, String::new()));
    }
    if matches!(lower.as_str(), "res-size" | "size") {
        return Some((Subject::Size, String::new()));
    }
    None
}

fn parse_tests(
    value: &Value,
    target: &str,
    out_assertions: &mut Vec<Assertion>,
    out_extractions: &mut Vec<Extraction>,
    lost: &mut Vec<Lost>,
) {
    match value {
        Value::String(script) => lost::push_script(lost, target, ScriptPhase::Test, script),
        Value::Object(_) => collect_script_value(value, target, ScriptPhase::Test, lost),
        Value::Array(rows) => {
            for row in rows {
                if is_disabled(row) {
                    continue;
                }
                let raw_type = text(row, "type").trim();
                let kind = raw_type.to_ascii_lowercase();
                let row_target = {
                    let custom = text(row, "custom").trim();
                    if !custom.is_empty() {
                        custom
                    } else {
                        text(row, "target").trim()
                    }
                };
                let action = text(row, "action").trim();
                let val_str = {
                    let v = value_text(row.get("value"));
                    if v.is_empty() { value_text(row.get("pval")) } else { v }
                };

                if kind == "set-env-var" {
                    let action_lower = action.to_ascii_lowercase();
                    let var_name = val_str
                        .trim()
                        .trim_start_matches("{{")
                        .trim_end_matches("}}")
                        .trim()
                        .to_string();
                    let source = extract_source(row_target);
                    if (action_lower.is_empty() || action_lower == "setto")
                        && !var_name.is_empty()
                        && let Some((subject, ext_target)) = source
                    {
                        out_extractions.push(Extraction {
                            enabled: true,
                            name: var_name,
                            subject,
                            target: ext_target,
                        });
                    } else if let Ok(raw_json) = serde_json::to_string(row) {
                        lost::push_script(lost, target, ScriptPhase::Test, &raw_json);
                    }
                    continue;
                }

                let mapped_subject = match kind.as_str() {
                    "res-code" => Some((Subject::Status, String::new())),
                    "res-body" => Some((Subject::Body, String::new())),
                    "res-time" => Some((Subject::Latency, String::new())),
                    "res-size" => Some((Subject::Size, String::new())),
                    "res-headers" => Some((Subject::Header, row_target.to_string())),
                    "content-type" => Some((Subject::Header, "Content-Type".to_string())),
                    "content-encoding" => Some((Subject::Header, "Content-Encoding".to_string())),
                    "json-query" => Some((Subject::Json, normalize_json_query_path(row_target))),
                    _ => None,
                };

                if let Some((subject, ass_target)) = mapped_subject
                    && let Some(op) = map_op(action)
                    && subject.ops().contains(&op)
                    && (!subject.has_target() || !ass_target.trim().is_empty())
                {
                    let expected = if op == Op::IsType {
                        val_str.trim().to_ascii_lowercase()
                    } else {
                        val_str
                    };
                    if op != Op::IsType || assertions::TYPES.contains(&expected.as_str()) {
                        out_assertions.push(Assertion {
                            enabled: true,
                            subject,
                            target: ass_target,
                            op,
                            expected,
                            ..Default::default()
                        });
                        continue;
                    }
                }

                // Unsupported or custom test rules (custom scripts, unsupported operators, etc.)
                if let Some(script) = row.get("script").and_then(Value::as_str)
                    && !script.trim().is_empty()
                {
                    lost::push_script(lost, target, ScriptPhase::Test, script);
                } else if let Ok(raw_json) = serde_json::to_string(row) {
                    lost::push_script(lost, target, ScriptPhase::Test, &raw_json);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HttpRequest;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn collection_folders_ordering_params_and_base_url() {
        let doc = json!({
            "clientName": "Thunder Client",
            "collectionName": "Shop API",
            "settings": {
                "options": { "baseUrl": "https://api.example.com/v1/" },
                "auth": {
                    "type": "bearer",
                    "bearer": [{ "key": "token", "value": "{{apiToken}}" }]
                },
                "headers": [
                    { "name": "X-Client", "value": "thunder" },
                    { "name": "X-Off", "value": "no", "isDisabled": true }
                ]
            },
            "folders": [
                {
                    "_id": "f-sub",
                    "name": "Admin",
                    "containerId": "f-users",
                    "sortNum": 20000,
                    "docs": "Admin operations",
                    "settings": {
                        "options": { "baseUrl": "https://admin.example.com" },
                        "headers": [{ "name": "X-Role", "value": "admin" }],
                        "auth": { "type": "basic", "basic": { "username": "u", "password": "p" } }
                    }
                },
                {
                    "_id": "f-users",
                    "name": "Users",
                    "containerId": "",
                    "sortNum": 10000
                }
            ],
            "requests": [
                {
                    "_id": "r-health",
                    "name": "Health",
                    "containerId": "",
                    "sortNum": 5000,
                    "method": "GET",
                    "url": "/health?verbose=1"
                },
                {
                    "_id": "r-user",
                    "name": "Get User",
                    "containerId": "f-users",
                    "sortNum": 10000,
                    "method": "GET",
                    "url": "users/{id}?include=profile&verbose=url",
                    "params": [
                        { "name": "id", "value": "42", "isPath": true },
                        { "name": "verbose", "value": "row", "isDisabled": true },
                        { "name": "page", "value": "2" }
                    ],
                    "docs": "Fetch a single user"
                },
                {
                    "_id": "r-admin",
                    "name": "Ban User",
                    "containerId": "f-sub",
                    "sortNum": 1000,
                    "method": "POST",
                    "url": "/ban"
                }
            ]
        });

        assert!(is_collection(&doc));
        assert_eq!(name(&doc), Some("Shop API"));

        let bundle = parse(&doc);
        assert_eq!(
            bundle.auth,
            Auth::Bearer {
                token: "{{apiToken}}".into()
            }
        );
        // Root headers wrap top-level items into a collection folder, preserving sortNum order:
        // `Health` (sortNum 5000) precedes `Users` folder (sortNum 10000).
        let [
            ImportItem::Folder {
                name: root_name,
                settings: root_settings,
                children: root_children,
            },
        ] = &bundle.items[..]
        else {
            panic!("expected wrapped root folder: {:?}", bundle.items);
        };
        assert_eq!(root_name, "Shop API");
        assert_eq!(root_settings.auth, Auth::Inherit);
        assert_eq!(
            root_settings.headers,
            [kv("X-Client", "thunder", true), kv("X-Off", "no", false)]
        );

        let [
            ImportItem::Api {
                name: health_name,
                request: health_req,
            },
            ImportItem::Folder {
                name: users_name,
                children: users_children,
                ..
            },
        ] = &root_children[..]
        else {
            panic!("unexpected root children: {root_children:?}");
        };
        assert_eq!(health_name, "Health");
        assert_eq!(health_req.uri, "https://api.example.com/v1/health");
        assert_eq!(health_req.query, [kv("verbose", "1", true)]);

        assert_eq!(users_name, "Users");
        let [
            ImportItem::Api {
                name: user_name,
                request: user_req,
            },
            ImportItem::Folder {
                name: admin_name,
                settings: admin_settings,
                children: admin_children,
            },
        ] = &users_children[..]
        else {
            panic!("unexpected users children: {users_children:?}");
        };
        assert_eq!(user_name, "Get User");
        assert_eq!(user_req.uri, "https://api.example.com/v1/users/{id}");
        assert_eq!(user_req.description, "Fetch a single user");
        assert_eq!(user_req.variables, [kv("id", "42", true)]);
        assert_eq!(
            user_req.query,
            [
                kv("verbose", "row", false),
                kv("page", "2", true),
                kv("include", "profile", true),
            ]
        );

        assert_eq!(admin_name, "Admin");
        assert_eq!(admin_settings.description, "Admin operations");
        assert_eq!(
            admin_settings.auth,
            Auth::Basic {
                username: "u".into(),
                password: "p".into()
            }
        );
        let [
            ImportItem::Api {
                name: ban_name,
                request: ban_req,
            },
        ] = &admin_children[..]
        else {
            panic!("unexpected admin children: {admin_children:?}");
        };
        assert_eq!(ban_name, "Ban User");
        assert_eq!(ban_req.uri, "https://admin.example.com/ban");
        assert_eq!(ban_req.method, HttpMethod::Post);
    }

    #[test]
    fn bodies_and_auth_variants() {
        let doc = json!({
            "client": "Thunder Client",
            "collectionName": "Bodies & Auth",
            "requests": [
                {
                    "name": "Multipart",
                    "method": "POST",
                    "url": "https://a.test/upload",
                    "body": {
                        "type": "formdata",
                        "form": [{ "name": "caption", "value": "hi" }],
                        "files": [{ "name": "avatar", "value": "/tmp/a.png" }]
                    },
                    "auth": {
                        "type": "apikey",
                        "apikey": { "key": "x-api-key", "value": "secret", "in": "query" }
                    }
                },
                {
                    "name": "Binary",
                    "method": "PUT",
                    "url": "https://a.test/bin",
                    "body": { "type": "binary", "binary": "/tmp/blob.bin" },
                    "auth": {
                        "type": "digest",
                        "digest": [{ "key": "username", "value": "du" }, { "key": "password", "value": "dp" }]
                    }
                },
                {
                    "name": "GraphQL",
                    "method": "POST",
                    "url": "https://a.test/graphql",
                    "body": {
                        "type": "graphql",
                        "graphql": { "query": "query { me { id } }", "variables": "{\"id\":1}" }
                    },
                    "auth": {
                        "type": "aws",
                        "aws": {
                            "accessKey": "AKIA123",
                            "secretKey": "SEC",
                            "sessionToken": "TOK",
                            "region": "us-east-1",
                            "service": "execute-api"
                        }
                    }
                },
                {
                    "name": "JwtAndOAuth",
                    "method": "POST",
                    "url": "https://a.test/jwt",
                    "body": { "type": "formencoded", "form": [{ "name": "k", "value": "v", "isDisabled": true }] },
                    "auth": {
                        "type": "jwt",
                        "algorithm": "RS256",
                        "secret": "PEM",
                        "payload": "{\"sub\":\"1\"}",
                        "header": "{\"kid\":\"k\"}",
                        "addTokenTo": "header",
                        "headerPrefix": "",
                        "queryParamKey": "tok"
                    }
                },
                {
                    "name": "OAuthImplicit",
                    "method": "GET",
                    "url": "https://a.test/me",
                    "auth": {
                        "type": "oauth2",
                        "grantType": "implicit",
                        "authUrl": "https://id.test/authorize",
                        "callbackUrl": "http://127.0.0.1:9988/callback",
                        "clientId": "app",
                        "scope": "read"
                    }
                },
                {
                    "name": "UnsupportedStuff",
                    "method": "POST",
                    "url": "https://a.test/custom",
                    "body": { "type": "bson" },
                    "auth": { "type": "ntlm" }
                }
            ]
        });

        let bundle = parse(&doc);
        let reqs: Vec<(&str, &HttpRequest)> = bundle
            .items
            .iter()
            .filter_map(|item| match item {
                ImportItem::Api { name, request } => Some((name.as_str(), request.as_ref())),
                _ => None,
            })
            .collect();
        assert_eq!(reqs.len(), 6);

        assert_eq!(reqs[0].1.content_type, ContentType::Multipart);
        let multipart_fields: Vec<KvParam> = serde_json::from_str(&reqs[0].1.body).expect("multipart json");
        assert_eq!(
            multipart_fields,
            [kv("caption", "hi", true), kv("avatar", "file:///tmp/a.png", true)]
        );
        assert_eq!(
            reqs[0].1.auth,
            Auth::ApiKey {
                name: "x-api-key".into(),
                value: "secret".into(),
                place: KeyPlace::Query
            }
        );

        assert_eq!(reqs[1].1.content_type, ContentType::Binary);
        assert_eq!(reqs[1].1.body, "file:///tmp/blob.bin");
        assert_eq!(
            reqs[1].1.auth,
            Auth::Digest {
                username: "du".into(),
                password: "dp".into()
            }
        );

        assert_eq!(reqs[2].1.content_type, ContentType::GraphQl);
        assert_eq!(GraphQlBody::read(&reqs[2].1.body).query, "query { me { id } }");
        assert_eq!(
            reqs[2].1.auth,
            Auth::from(AwsSigV4 {
                access_key: "AKIA123".into(),
                secret_key: "SEC".into(),
                session_token: "TOK".into(),
                region: "us-east-1".into(),
                service: "execute-api".into(),
            })
        );

        assert_eq!(reqs[3].1.content_type, ContentType::Form);
        assert_eq!(
            reqs[3].1.auth,
            Auth::from(JwtBearer {
                algorithm: JwtAlgorithm::RS256,
                secret: "PEM".into(),
                secret_base64: false,
                payload: r#"{"sub":"1"}"#.into(),
                header: r#"{"kid":"k"}"#.into(),
                place: KeyPlace::Header,
                prefix: String::new(),
                query_param: "tok".into(),
            })
        );

        assert_eq!(
            reqs[4].1.auth,
            Auth::from(OAuth2 {
                grant: Grant::Implicit,
                authorization_url: "https://id.test/authorize".into(),
                redirect_port: 9988,
                client_id: "app".into(),
                scope: "read".into(),
                client_auth: ClientAuth::Header,
                ..Default::default()
            })
        );

        assert_eq!(reqs[5].1.auth, Auth::Inherit);
        assert_eq!(
            bundle.lost,
            vec![
                Lost {
                    target: "UnsupportedStuff".into(),
                    kind: lost::LostKind::Body { kind: "bson".into() },
                },
                Lost {
                    target: "UnsupportedStuff".into(),
                    kind: lost::LostKind::Auth { kind: "ntlm".into() },
                },
            ]
        );
    }

    #[test]
    fn tests_extractions_and_scripts_report() {
        let doc = json!({
            "clientName": "Thunder Client",
            "collectionName": "Checks",
            "settings": {
                "preReq": { "inlineScripts": [{ "script": "tc.setVar('col', '1');" }] }
            },
            "requests": [
                {
                    "name": "Login",
                    "method": "POST",
                    "url": "https://a.test/login",
                    "preReq": { "script": "console.log('pre');" },
                    "postReq": { "script": "console.log('post');" },
                    "tests": [
                        { "type": "res-code", "custom": "", "action": "equal", "value": "200" },
                        { "type": "res-time", "custom": "", "action": "<", "value": "500" },
                        { "type": "json-query", "custom": "json.user.role", "action": "equal", "value": "admin" },
                        { "type": "res-headers", "custom": "X-Request-Id", "action": "exists", "value": "" },
                        { "type": "content-type", "custom": "", "action": "contains", "value": "application/json" },
                        { "type": "set-env-var", "custom": "json.data.token", "action": "setto", "value": "{{authToken}}" },
                        { "type": "set-env-var", "custom": "header.X-Session", "action": "setto", "value": "sessionId" },
                        { "type": "custom", "custom": "js", "action": "custom", "value": "tc.test()" }
                    ]
                }
            ]
        });

        let bundle = parse(&doc);
        let [ImportItem::Api { request, .. }] = &bundle.items[..] else {
            panic!("expected single api");
        };
        assert_eq!(
            request.assertions,
            vec![
                Assertion {
                    enabled: true,
                    subject: Subject::Status,
                    target: String::new(),
                    op: Op::Equals,
                    expected: "200".into(),
                    imported: false,
                },
                Assertion {
                    enabled: true,
                    subject: Subject::Latency,
                    target: String::new(),
                    op: Op::Less,
                    expected: "500".into(),
                    imported: false,
                },
                Assertion {
                    enabled: true,
                    subject: Subject::Json,
                    target: "$.user.role".into(),
                    op: Op::Equals,
                    expected: "admin".into(),
                    imported: false,
                },
                Assertion {
                    enabled: true,
                    subject: Subject::Header,
                    target: "X-Request-Id".into(),
                    op: Op::Exists,
                    expected: String::new(),
                    imported: false,
                },
                Assertion {
                    enabled: true,
                    subject: Subject::Header,
                    target: "Content-Type".into(),
                    op: Op::Contains,
                    expected: "application/json".into(),
                    imported: false,
                },
            ]
        );
        assert_eq!(
            request.extractions,
            vec![
                Extraction {
                    enabled: true,
                    name: "authToken".into(),
                    subject: Subject::Json,
                    target: "$.data.token".into(),
                },
                Extraction {
                    enabled: true,
                    name: "sessionId".into(),
                    subject: Subject::Header,
                    target: "X-Session".into(),
                },
            ]
        );
        assert_eq!(bundle.lost.len(), 4);
        assert_eq!(bundle.lost[0].target, "");
        assert_eq!(bundle.lost[1].target, "Login");
    }

    #[test]
    fn environments_and_multi_collection() {
        let env_doc = json!([
            {
                "clientName": "Thunder Client",
                "environmentName": "Staging",
                "data": [
                    { "name": "host", "value": "https://staging.example.com" },
                    { "name": "retries", "value": 3 },
                    { "name": "off", "value": "ignored", "isDisabled": true }
                ]
            },
            {
                "client": "Thunder Client",
                "environmentName": "Production",
                "variables": [{ "name": "host", "value": "https://prod.example.com" }]
            }
        ]);
        assert!(is_environment(&env_doc));
        let env_bundle = parse_environment(&env_doc);
        assert_eq!(
            env_bundle.environments,
            vec![
                ImportEnvironment {
                    name: "Staging".into(),
                    variables: vec![
                        ("host".into(), "https://staging.example.com".into()),
                        ("retries".into(), "3".into()),
                    ],
                },
                ImportEnvironment {
                    name: "Production".into(),
                    variables: vec![("host".into(), "https://prod.example.com".into())],
                },
            ]
        );

        let multi = json!([
            {
                "collectionName": "One",
                "requests": [{ "name": "Ping", "method": "GET", "url": "https://a.test/1" }]
            },
            {
                "collectionName": "Two",
                "requests": [{ "name": "Pong", "method": "GET", "url": "https://a.test/2" }]
            }
        ]);
        assert!(is_collection(&multi));
        assert_eq!(name(&multi), None);
        let multi_bundle = parse(&multi);
        assert_eq!(multi_bundle.items.len(), 2);
    }
}
