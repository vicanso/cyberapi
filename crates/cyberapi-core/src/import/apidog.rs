//! R12 (3/3): Apidog / Apifox project JSON export importer.
//!
//! Imports project info, folders (`apiCollection`, unwrapping a single root
//! `"根目录"` / `"Root"` folder), endpoints (`api`), named environments with
//! `baseUrl`/`baseUrls` (`{{BASE_URL}}` injection for relative paths), query/path/
//! header/cookie parameters (including descriptions, `required`, `type`,
//! `format`, `enum`), `schemaCollection` `$ref` schemas (request schema + sample
//! body fallback + 2xx response schema assertions), `responseExamples` &
//! `responses` examples, auth (Basic, Bearer, API key, Digest, AWS SigV4, JWT
//! Bearer, OAuth 2.0), multipart/form/binary/GraphQL/raw bodies, declarative
//! `postProcessors` (`assertion`, `extractVariable`), and static `customScript`
//! conversion via [`super::script`]. Unconverted scripts/processors and
//! unsupported auth/body types are reported via [`lost`].

use super::lost::{self, Lost, ScriptPhase};
use super::script;
use super::{ImportBundle, ImportEnvironment, ImportItem, api, guess_content_type, kv};
use crate::assertions::{self, Assertion, Extraction, Op, Subject};
use crate::auth::{Auth, KeyPlace};
use crate::aws_sigv4::AwsSigV4;
use crate::graphql::GraphQlBody;
use crate::jwt_bearer::{JwtAlgorithm, JwtBearer};
use crate::model::{ContentType, Example, FolderSettings, HttpMethod, HttpRequest, KvParam};
use crate::oauth::{ClientAuth, Grant, OAuth2};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};

const SCHEMA_DEPTH_LIMIT: usize = 6;

fn project_root(json: &Value) -> &Value {
    for key in ["apidogProject", "apifoxProject"] {
        if let Some(inner) = json.get(key)
            && inner.is_object()
            && (inner.get("apiCollection").is_some() || inner.get("environments").is_some())
        {
            return inner;
        }
    }
    json
}

/// Whether `json` is an Apidog or Apifox project export JSON document.
pub(super) fn is_apidog(json: &Value) -> bool {
    let Some(map) = json.as_object() else {
        return false;
    };
    if map.contains_key("apidogProject") || map.contains_key("apifoxProject") {
        return true;
    }
    if json.pointer("/$schema/app").and_then(Value::as_str).is_some_and(|app| {
        let app = app.trim();
        app.eq_ignore_ascii_case("apidog") || app.eq_ignore_ascii_case("apifox")
    }) {
        return true;
    }
    map.get("apiCollection").is_some_and(Value::is_array)
        && (map.contains_key("environments")
            || map.contains_key("schemaCollection")
            || map.contains_key("projectSetting"))
}

/// The project's own name from `info.name`, `info.title`, or top-level `name`.
pub(super) fn name(json: &Value) -> Option<&str> {
    let root = project_root(json);
    root.pointer("/info/name")
        .or_else(|| root.pointer("/info/title"))
        .or_else(|| root.get("name"))
        .and_then(Value::as_str)
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
    bool_flag(value, "enable") != Some(false)
        && bool_flag(value, "enabled") != Some(false)
        && bool_flag(value, "disabled") != Some(true)
        && bool_flag(value, "isDisabled") != Some(true)
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

fn parse_variable_list(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| is_enabled(item))
                .filter_map(|item| {
                    let key = ["name", "key"]
                        .into_iter()
                        .map(|k| text(item, k).trim())
                        .find(|k| !k.is_empty())?;
                    Some((key.to_string(), value_text(item.get("value"))))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_environments(root: &Value) -> (Vec<(String, String)>, Vec<ImportEnvironment>, bool) {
    let shared = parse_variable_list(root.get("globalVariables").or_else(|| root.get("variables")));
    let mut has_base_url = shared.iter().any(|(k, v)| k == "BASE_URL" && !v.trim().is_empty());

    let named = root
        .get("environments")
        .and_then(Value::as_array)
        .map(|envs| {
            envs.iter()
                .filter_map(|env| {
                    let env_name = text(env, "name").trim().to_string();
                    if env_name.is_empty() {
                        return None;
                    }
                    let mut vars = Vec::new();
                    let single_base = text(env, "baseUrl").trim();
                    let mut env_has_base = false;
                    if !single_base.is_empty() {
                        vars.push(("BASE_URL".to_string(), single_base.to_string()));
                        env_has_base = true;
                        has_base_url = true;
                    }
                    if let Some(base_urls) = env.get("baseUrls").and_then(Value::as_array) {
                        for entry in base_urls {
                            match entry {
                                Value::String(url) if !url.trim().is_empty() => {
                                    if !env_has_base {
                                        vars.push(("BASE_URL".to_string(), url.trim().to_string()));
                                        env_has_base = true;
                                        has_base_url = true;
                                    }
                                }
                                Value::Object(_) => {
                                    let url = text(entry, "url").trim();
                                    if url.is_empty() {
                                        continue;
                                    }
                                    if !env_has_base {
                                        vars.push(("BASE_URL".to_string(), url.to_string()));
                                        env_has_base = true;
                                        has_base_url = true;
                                    } else {
                                        let label = ["name", "id"]
                                            .into_iter()
                                            .map(|k| value_text(entry.get(k)))
                                            .find(|s| !s.trim().is_empty());
                                        if let Some(label) = label {
                                            let key = label.trim().to_string();
                                            if !vars.iter().any(|(k, _)| k == &key) {
                                                vars.push((key, url.to_string()));
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    for (k, v) in parse_variable_list(env.get("parameters").or_else(|| env.get("variables"))) {
                        if k == "BASE_URL" && !v.trim().is_empty() {
                            has_base_url = true;
                        }
                        if !vars.iter().any(|(existing, _)| existing == &k) {
                            vars.push((k, v));
                        }
                    }
                    Some(ImportEnvironment {
                        name: env_name,
                        variables: vars,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    (shared, named, has_base_url)
}

fn collect_schemas_tree(root: &Value) -> HashMap<String, &Value> {
    fn walk<'a>(items: &'a [Value], out: &mut HashMap<String, &'a Value>) {
        for item in items {
            if let Some(children) = item.get("items").and_then(Value::as_array) {
                walk(children, out);
            }
            let schema = item
                .get("jsonSchema")
                .or_else(|| item.pointer("/schema/jsonSchema"))
                .or_else(|| item.get("schema"))
                .filter(|v| v.is_object());
            if let Some(schema) = schema {
                let id = match item.get("id") {
                    Some(Value::String(s)) => s.trim().to_string(),
                    Some(Value::Number(n)) => n.to_string(),
                    _ => String::new(),
                };
                let name = text(item, "name").trim();
                if !id.is_empty() {
                    out.insert(id.clone(), schema);
                    out.insert(format!("#/definitions/{id}"), schema);
                    out.insert(format!("#/components/schemas/{id}"), schema);
                }
                if !name.is_empty() {
                    out.entry(name.to_string()).or_insert(schema);
                    out.entry(format!("#/definitions/{name}")).or_insert(schema);
                    out.entry(format!("#/components/schemas/{name}")).or_insert(schema);
                }
            }
        }
    }
    let mut out = HashMap::new();
    if let Some(list) = root.get("schemaCollection").and_then(Value::as_array) {
        walk(list, &mut out);
    }
    if let Some(schemas) = root.pointer("/components/schemas").and_then(Value::as_object) {
        for (name, schema) in schemas {
            if schema.is_object() {
                out.entry(name.clone()).or_insert(schema);
                out.entry(format!("#/components/schemas/{name}")).or_insert(schema);
                out.entry(format!("#/definitions/{name}")).or_insert(schema);
            }
        }
    }
    out
}

fn lookup_schema<'a>(reference: &str, schemas: &HashMap<String, &'a Value>) -> Option<&'a Value> {
    if let Some(found) = schemas.get(reference) {
        return Some(*found);
    }
    let tail = reference.rsplit('/').next().unwrap_or(reference).trim();
    (!tail.is_empty()).then(|| schemas.get(tail).copied()).flatten()
}

fn resolve_schema(
    schema: &Value,
    schemas: &HashMap<String, &Value>,
    depth: usize,
    visiting: &mut HashSet<String>,
) -> Value {
    match schema {
        Value::Object(map) => {
            if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                let ref_key = reference.trim().to_string();
                if depth >= SCHEMA_DEPTH_LIMIT || visiting.contains(&ref_key) {
                    return json!({});
                }
                let Some(target) = lookup_schema(&ref_key, schemas) else {
                    return json!({});
                };
                visiting.insert(ref_key.clone());
                let resolved = resolve_schema(target, schemas, depth + 1, visiting);
                visiting.remove(&ref_key);
                return resolved;
            }
            let mut out = Map::with_capacity(map.len());
            for (key, val) in map {
                out.insert(key.clone(), resolve_schema(val, schemas, depth, visiting));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| resolve_schema(item, schemas, depth, visiting))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn resolve_standalone(schema: &Value, schemas: &HashMap<String, &Value>) -> Value {
    let mut visiting = HashSet::new();
    resolve_schema(schema, schemas, 0, &mut visiting)
}

pub(super) fn parse(json: &Value) -> ImportBundle {
    let root = project_root(json);
    let schemas = collect_schemas_tree(root);
    let (shared_env, named_envs, has_base_url) = parse_environments(root);
    let mut lost = Vec::new();

    let collection = root
        .get("apiCollection")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    let is_synthetic_root = |node: &Value| -> bool {
        if node.get("api").is_some() || (node.get("method").is_some() && node.get("path").is_some()) {
            return false;
        }
        if !node.get("items").is_some_and(Value::is_array) {
            return false;
        }
        let n = text(node, "name").trim();
        let id_zero = node.get("id").and_then(Value::as_i64) == Some(0)
            || node.get("parentId").and_then(Value::as_i64) == Some(0);
        n.is_empty() || n == "根目录" || n.eq_ignore_ascii_case("root") || id_zero
    };

    let (bundle_auth, items) = if collection.len() == 1 && is_synthetic_root(&collection[0]) {
        let root_node = &collection[0];
        let root_auth = root_node
            .get("auth")
            .and_then(|v| auth(v, "", &mut lost))
            .unwrap_or_default();
        let root_headers = common_headers(root_node);
        let root_children = root_node
            .get("items")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();

        report_container_processors(root_node, "", &mut lost);

        let folder_name = {
            let candidate = text(root_node, "name").trim();
            if candidate.is_empty() {
                name(json).map(str::trim).filter(|s| !s.is_empty()).unwrap_or("Root")
            } else {
                candidate
            }
        };
        let wraps = !root_headers.is_empty();
        let child_prefix = if wraps { folder_name } else { "" };
        let built_children = convert_items(root_children, child_prefix, has_base_url, &schemas, &mut lost);
        let items = if wraps {
            vec![ImportItem::Folder {
                name: folder_name.to_string(),
                children: built_children,
                settings: FolderSettings {
                    auth: Auth::Inherit,
                    headers: root_headers,
                    ..Default::default()
                },
            }]
        } else {
            built_children
        };
        (root_auth, items)
    } else {
        let root_auth = root
            .get("auth")
            .and_then(|v| auth(v, "", &mut lost))
            .unwrap_or_default();
        let items = convert_items(collection, "", has_base_url, &schemas, &mut lost);
        (root_auth, items)
    };

    ImportBundle {
        items,
        environment: shared_env,
        auth: bundle_auth,
        environments: named_envs,
        lost,
        ..Default::default()
    }
}

fn convert_items(
    items: &[Value],
    prefix: &str,
    has_base_url: bool,
    schemas: &HashMap<String, &Value>,
    lost: &mut Vec<Lost>,
) -> Vec<ImportItem> {
    let mut out = Vec::with_capacity(items.len());
    for entry in items {
        let is_api = entry.get("api").is_some_and(Value::is_object)
            || (entry.get("method").is_some() && (entry.get("path").is_some() || entry.get("url").is_some()));
        if is_api {
            out.push(convert_api(entry, prefix, has_base_url, schemas, lost));
        } else if let Some(children) = entry.get("items").and_then(Value::as_array) {
            out.push(convert_folder(entry, children, prefix, has_base_url, schemas, lost));
        }
    }
    out
}

fn common_headers(node: &Value) -> Vec<KvParam> {
    let list = node
        .pointer("/commonParameters/header")
        .or_else(|| node.get("headers"))
        .and_then(Value::as_array);
    list.map(|items| items.iter().filter_map(param_row).collect())
        .unwrap_or_default()
}

fn report_container_processors(node: &Value, target: &str, lost: &mut Vec<Lost>) {
    for (key, phase) in [
        ("preProcessors", ScriptPhase::PreRequest),
        ("postProcessors", ScriptPhase::PostResponse),
    ] {
        let Some(list) = node.get(key).and_then(Value::as_array) else {
            continue;
        };
        for proc in list {
            if !is_enabled(proc) {
                continue;
            }
            if text(proc, "type").trim() == "customScript" {
                let script_text = proc
                    .pointer("/data/script")
                    .or_else(|| proc.get("data"))
                    .or_else(|| proc.get("script"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                lost::push_script(lost, target, phase, script_text);
            } else if let Ok(raw_json) = serde_json::to_string(proc) {
                lost::push_script(lost, target, phase, &raw_json);
            }
        }
    }
}

fn convert_folder(
    node: &Value,
    children: &[Value],
    prefix: &str,
    has_base_url: bool,
    schemas: &HashMap<String, &Value>,
    lost: &mut Vec<Lost>,
) -> ImportItem {
    let raw_name = text(node, "name").trim();
    let folder_name = if raw_name.is_empty() {
        "Folder".to_string()
    } else {
        raw_name.to_string()
    };
    let target = lost::join_path(prefix, &folder_name);
    let folder_auth = node
        .get("auth")
        .and_then(|v| auth(v, &target, lost))
        .unwrap_or_default();
    let settings = FolderSettings {
        auth: folder_auth,
        headers: common_headers(node),
        description: text(node, "description").trim().to_string(),
        variables: Vec::new(),
        source: None,
    };
    report_container_processors(node, &target, lost);
    ImportItem::Folder {
        name: folder_name,
        children: convert_items(children, &target, has_base_url, schemas, lost),
        settings,
    }
}

fn basic_param_row(item: &Value) -> Option<KvParam> {
    let key = ["name", "key"]
        .into_iter()
        .map(|k| text(item, k).trim())
        .find(|k| !k.is_empty())?;
    let val = ["example", "value", "default"]
        .into_iter()
        .find_map(|k| item.get(k).filter(|v| !v.is_null()).map(|v| value_text(Some(v))))
        .unwrap_or_default();
    let enabled = is_enabled(item);
    let description = text(item, "description").trim();
    let mut row = kv(key, &val, enabled);
    if !description.is_empty() {
        row = row.with_description(description);
    }
    Some(row)
}

fn param_row(item: &Value) -> Option<KvParam> {
    let mut row = basic_param_row(item)?;
    let schema_type = {
        let t = text(item, "type").trim();
        if !t.is_empty() {
            t
        } else {
            item.pointer("/schema/type").and_then(Value::as_str).unwrap_or_default()
        }
    };
    let required = bool_flag(item, "required");
    let format = {
        let f = text(item, "format").trim();
        if !f.is_empty() {
            f
        } else {
            item.pointer("/schema/format")
                .and_then(Value::as_str)
                .unwrap_or_default()
        }
    };
    let enum_vals: Vec<String> = item
        .get("enum")
        .or_else(|| item.pointer("/schema/enum"))
        .and_then(Value::as_array)
        .map(|vals| {
            vals.iter()
                .map(|v| value_text(Some(v)))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    if !schema_type.is_empty() || required.is_some() || !format.is_empty() || !enum_vals.is_empty() {
        row = row.with_param_schema(schema_type, required, format, enum_vals);
    }
    Some(row)
}

fn convert_api(
    entry: &Value,
    prefix: &str,
    has_base_url: bool,
    schemas: &HashMap<String, &Value>,
    lost: &mut Vec<Lost>,
) -> ImportItem {
    let api_obj = entry.get("api").filter(|v| v.is_object()).unwrap_or(entry);
    let raw_name = [text(entry, "name"), text(api_obj, "name"), text(api_obj, "summary")]
        .into_iter()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or_default();
    let method = {
        let m = text(api_obj, "method").trim();
        if m.is_empty() { "GET" } else { m }
    };
    let raw_path = {
        let p = text(api_obj, "path").trim();
        if !p.is_empty() { p } else { text(api_obj, "url").trim() }
    };
    let effective_url = if has_base_url && raw_path.starts_with('/') && !raw_path.contains("://") {
        format!("{{{{BASE_URL}}}}{raw_path}")
    } else {
        raw_path.to_string()
    };

    let (name, mut built) = api(raw_name, method, &effective_url);
    let target = lost::join_path(prefix, &name);

    built.description = [text(api_obj, "description"), text(entry, "description")]
        .into_iter()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .to_string();

    if let Some(params) = api_obj.get("parameters") {
        let mut query_rows: Vec<KvParam> = Vec::new();
        let mut seen_keys: HashSet<String> = HashSet::new();
        if let Some(qs) = params.get("query").and_then(Value::as_array) {
            for item in qs {
                if let Some(row) = param_row(item) {
                    seen_keys.insert(row.key.clone());
                    query_rows.push(row);
                }
            }
        }
        for url_param in built.query.drain(..) {
            if !seen_keys.contains(&url_param.key) {
                query_rows.push(url_param);
            }
        }
        built.query = query_rows;

        if let Some(hs) = params.get("header").and_then(Value::as_array) {
            built.headers.extend(hs.iter().filter_map(param_row));
        }
        if let Some(ps) = params.get("path").and_then(Value::as_array) {
            built.variables.extend(ps.iter().filter_map(param_row));
        }
        if let Some(cs) = params.get("cookie").and_then(Value::as_array)
            && !has_header(&built.headers, "cookie")
        {
            let pairs: Vec<String> = cs
                .iter()
                .filter_map(param_row)
                .filter(|row| row.enabled && !row.key.is_empty())
                .map(|row| format!("{}={}", row.key, row.value))
                .collect();
            if !pairs.is_empty() {
                built.headers.push(kv("Cookie", &pairs.join("; "), true));
            }
        }
    }

    if let Some(rb) = api_obj.get("requestBody") {
        apply_request_body(rb, schemas, &target, &mut built, lost);
    }

    if !built.method.has_body() && built.method != HttpMethod::Get {
        built.content_type = ContentType::default();
    }

    if let Some(own_auth) = api_obj
        .get("auth")
        .or_else(|| entry.get("auth"))
        .and_then(|v| auth(v, &target, lost))
    {
        built.auth = own_auth;
    }

    apply_examples_and_responses(api_obj, schemas, &mut built);
    apply_processors(entry, api_obj, &target, &mut built, lost);

    ImportItem::Api {
        name,
        request: Box::new(built),
    }
}

fn body_value_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => crate::format::json(other),
    }
}

fn first_body_example(rb: &Value) -> Option<String> {
    if let Some(ex) = rb.get("example").filter(|v| !v.is_null()) {
        let s = body_value_text(ex);
        if !s.trim().is_empty() {
            return Some(s);
        }
    }
    match rb.get("examples") {
        Some(Value::Array(list)) => {
            for item in list {
                let candidate = item.get("value").or_else(|| item.get("data")).unwrap_or(item);
                if !candidate.is_null() {
                    let s = body_value_text(candidate);
                    if !s.trim().is_empty() {
                        return Some(s);
                    }
                }
            }
        }
        Some(Value::Object(map)) => {
            for (_, item) in map {
                let candidate = item.get("value").or_else(|| item.get("data")).unwrap_or(item);
                if !candidate.is_null() {
                    let s = body_value_text(candidate);
                    if !s.trim().is_empty() {
                        return Some(s);
                    }
                }
            }
        }
        _ => {}
    }
    None
}

fn apply_request_body(
    rb: &Value,
    schemas: &HashMap<String, &Value>,
    target: &str,
    built: &mut HttpRequest,
    lost: &mut Vec<Lost>,
) {
    let body_type_raw = text(rb, "type").trim();
    let body_type = body_type_raw.to_ascii_lowercase();
    let raw_text = first_body_example(rb)
        .or_else(|| rb.get("raw").map(body_value_text))
        .unwrap_or_default();

    match body_type.as_str() {
        "" | "none" => {}
        "application/json" | "json" => {
            built.content_type = ContentType::Json;
            if let Some(schema) = rb.get("jsonSchema").filter(|v| v.is_object()) {
                let standalone = resolve_standalone(schema, schemas);
                let schema_str = crate::format::json(&standalone);
                built.request_schema = Some(schema_str);
            }
            if !raw_text.trim().is_empty() {
                built.body = raw_text;
            } else if let Some(schema_str) = &built.request_schema
                && let Ok(sample) = crate::schema_docs::sample_of_schema(schema_str)
            {
                built.body = crate::format::json(&sample);
            }
        }
        "application/xml" | "xml" | "text/xml" => {
            built.content_type = ContentType::Xml;
            built.body = raw_text;
        }
        "text/plain" | "text" | "raw" => {
            built.content_type = ContentType::Plain;
            built.body = raw_text;
        }
        "text/html" | "html" => {
            built.content_type = ContentType::Plain;
            built.body = raw_text;
            if !has_header(&built.headers, "content-type") {
                built.headers.push(kv("Content-Type", "text/html", true));
            }
        }
        "application/x-www-form-urlencoded" | "x-www-form-urlencoded" | "form" => {
            built.content_type = ContentType::Form;
            let fields: Vec<KvParam> = rb
                .get("parameters")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(basic_param_row).collect())
                .unwrap_or_default();
            built.body = serde_json::to_string(&fields).unwrap_or_default();
        }
        "multipart/form-data" | "form-data" => {
            built.content_type = ContentType::Multipart;
            let fields: Vec<KvParam> = rb
                .get("parameters")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let mut row = basic_param_row(item)?;
                            if text(item, "type").trim().eq_ignore_ascii_case("file") {
                                row.value = normalize_file_value(&row.value);
                            }
                            Some(row)
                        })
                        .collect()
                })
                .unwrap_or_default();
            built.body = serde_json::to_string(&fields).unwrap_or_default();
        }
        "application/octet-stream" | "binary" => {
            built.content_type = ContentType::Binary;
            let raw_path = ["example", "binary", "file", "filePath"]
                .into_iter()
                .map(|k| value_text(rb.get(k)))
                .find(|s| !s.trim().is_empty())
                .unwrap_or_default();
            built.body = normalize_file_value(&raw_path);
        }
        "graphql" | "application/graphql" => {
            built.content_type = ContentType::GraphQl;
            let (query, variables, operation_name) = if let Some(gql) = rb.get("graphql").filter(|v| v.is_object()) {
                let vars = match gql.get("variables") {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) if !other.is_null() => crate::format::json(other),
                    _ => String::new(),
                };
                (
                    text(gql, "query").to_string(),
                    vars,
                    text(gql, "operationName").to_string(),
                )
            } else if let Some(Value::Object(ex)) = rb.get("example") {
                let vars = match ex.get("variables") {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) if !other.is_null() => crate::format::json(other),
                    _ => String::new(),
                };
                (
                    ex.get("query").and_then(Value::as_str).unwrap_or_default().to_string(),
                    vars,
                    ex.get("operationName")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                )
            } else {
                (raw_text, String::new(), String::new())
            };
            built.body = GraphQlBody {
                query,
                variables,
                operation_name,
            }
            .to_body();
        }
        _ => {
            if !raw_text.trim().is_empty() && body_type_raw.contains('/') {
                built.body = raw_text;
                built.content_type = guess_content_type(&built.body);
                if !has_header(&built.headers, "content-type") {
                    built.headers.push(kv("Content-Type", body_type_raw, true));
                }
            } else {
                lost::push_body(lost, target, body_type_raw);
            }
        }
    }
}

fn example_headers(value: &Value, default_ct: &str) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    if let Some(list) = value.get("headers").and_then(Value::as_array) {
        for item in list {
            let key = ["name", "key"]
                .into_iter()
                .map(|k| text(item, k).trim())
                .find(|k| !k.is_empty());
            if let Some(key) = key {
                headers.push((key.to_string(), value_text(item.get("value"))));
            }
        }
    }
    let ct = {
        let own = text(value, "contentType").trim();
        if own.is_empty() { default_ct } else { own }
    };
    if !ct.is_empty() && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
        headers.push(("Content-Type".to_string(), ct.to_string()));
    }
    headers
}

fn apply_examples_and_responses(api_obj: &Value, schemas: &HashMap<String, &Value>, built: &mut HttpRequest) {
    let responses = api_obj
        .get("responses")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    struct RespInfo<'a> {
        status: u16,
        content_type: &'a str,
        name: &'a str,
    }
    let mut resp_by_id: HashMap<String, RespInfo<'_>> = HashMap::new();
    for resp in responses {
        let id = match resp.get("id") {
            Some(Value::String(s)) => s.trim().to_string(),
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        };
        let status = resp
            .get("code")
            .or_else(|| resp.get("status"))
            .and_then(Value::as_u64)
            .and_then(|n| u16::try_from(n).ok())
            .unwrap_or(200);
        if !id.is_empty() {
            resp_by_id.insert(
                id,
                RespInfo {
                    status,
                    content_type: text(resp, "contentType").trim(),
                    name: text(resp, "name").trim(),
                },
            );
        }
    }

    if let Some(examples) = api_obj.get("responseExamples").and_then(Value::as_array) {
        for ex in examples {
            let linked = match ex.get("responseId") {
                Some(Value::String(s)) => resp_by_id.get(s.trim()),
                Some(Value::Number(n)) => resp_by_id.get(&n.to_string()),
                _ => None,
            };
            let status = ex
                .get("code")
                .or_else(|| ex.get("status"))
                .and_then(Value::as_u64)
                .and_then(|n| u16::try_from(n).ok())
                .or_else(|| linked.map(|info| info.status))
                .unwrap_or(200);
            let default_ct = linked.map(|info| info.content_type).unwrap_or_default();
            let raw_body = ex
                .get("data")
                .or_else(|| ex.get("value"))
                .filter(|v| !v.is_null())
                .map(body_value_text)
                .unwrap_or_default();
            if raw_body.len() > Example::BODY_LIMIT {
                continue;
            }
            let ex_name = {
                let n = text(ex, "name").trim();
                if !n.is_empty() {
                    n.to_string()
                } else if let Some(info) = linked
                    && !info.name.is_empty()
                {
                    info.name.to_string()
                } else {
                    status.to_string()
                }
            };
            built.examples.push(Example {
                name: ex_name,
                status,
                headers: example_headers(ex, default_ct),
                body: raw_body,
            });
        }
    }

    let mut added_response_schema = false;
    for resp in responses {
        let status = resp
            .get("code")
            .or_else(|| resp.get("status"))
            .and_then(Value::as_u64)
            .and_then(|n| u16::try_from(n).ok())
            .unwrap_or(200);
        let ct = text(resp, "contentType").trim();
        let resp_name = {
            let n = text(resp, "name").trim();
            if n.is_empty() {
                status.to_string()
            } else {
                n.to_string()
            }
        };
        if let Some(ex) = resp.get("example").filter(|v| !v.is_null()) {
            let body = body_value_text(ex);
            if !body.trim().is_empty()
                && body.len() <= Example::BODY_LIMIT
                && !built
                    .examples
                    .iter()
                    .any(|existing| existing.status == status && existing.name == resp_name)
            {
                built.examples.push(Example {
                    name: resp_name,
                    status,
                    headers: example_headers(resp, ct),
                    body,
                });
            }
        }
        if !added_response_schema
            && (200..300).contains(&status)
            && (ct.is_empty() || ct.to_ascii_lowercase().contains("json"))
            && let Some(schema) = resp.get("jsonSchema").filter(|v| v.is_object())
            && schema.as_object().is_some_and(|m| !m.is_empty())
        {
            let standalone = resolve_standalone(schema, schemas);
            built.assertions.push(Assertion::imported_schema(&standalone));
            added_response_schema = true;
        }
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
                            && is_enabled(entry)
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
    if !is_enabled(value) {
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
            let place = if field("apikey", &["in", "addTo", "location"])
                .to_ascii_lowercase()
                .contains("query")
            {
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
        "awsv4" | "aws" => Auth::from(AwsSigV4 {
            access_key: field("awsv4", &["accessKey", "accessKeyId"])
                .if_empty_then(|| field("aws", &["accessKey", "accessKeyId"])),
            secret_key: field("awsv4", &["secretKey", "secretAccessKey"])
                .if_empty_then(|| field("aws", &["secretKey", "secretAccessKey"])),
            session_token: field("awsv4", &["sessionToken"]).if_empty_then(|| field("aws", &["sessionToken"])),
            region: field("awsv4", &["region"]).if_empty_then(|| field("aws", &["region"])),
            service: field("awsv4", &["service"]).if_empty_then(|| field("aws", &["service"])),
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
            let place = if field("jwt", &["addTokenTo", "place", "in", "location"])
                .to_ascii_lowercase()
                .contains("query")
            {
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
            let redirect_port = ["redirectUri", "redirect_uri", "callbackUrl", "callback"]
                .into_iter()
                .filter_map(|k| opt("oauth2", &[k]))
                .find_map(|uri| {
                    let port = crate::oauth::redirect_port_of(&uri);
                    (port != 0).then_some(port)
                })
                .unwrap_or(0);
            let client_auth = if field(
                "oauth2",
                &["clientAuthentication", "client_authentication", "clientAuth"],
            )
            .to_ascii_lowercase()
            .contains("body")
            {
                ClientAuth::Body
            } else {
                ClientAuth::Header
            };
            Auth::from(OAuth2 {
                grant,
                token_url: field("oauth2", &["accessTokenUrl", "tokenUrl"]),
                authorization_url: field("oauth2", &["authUrl", "authorizationUrl"]),
                redirect_port,
                client_id: field("oauth2", &["clientId"]),
                client_secret: field("oauth2", &["clientSecret"]),
                username: field("oauth2", &["username"]),
                password: field("oauth2", &["password"]),
                scope: field("oauth2", &["scope"]),
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

trait EmptyFallback {
    fn if_empty_then(self, f: impl FnOnce() -> String) -> String;
}

impl EmptyFallback for String {
    fn if_empty_then(self, f: impl FnOnce() -> String) -> String {
        if self.is_empty() { f() } else { self }
    }
}

fn normalize_json_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with('$') {
        trimmed.to_string()
    } else if trimmed.starts_with('[') {
        format!("${trimmed}")
    } else {
        format!("$.{}", trimmed.trim_start_matches('.'))
    }
}

fn map_subject(raw: &str) -> Option<Subject> {
    match raw.trim().to_ascii_lowercase().replace(['_', '-'], "").as_str() {
        "responsejson" | "json" | "bodyjson" => Some(Subject::Json),
        "responseheader" | "header" => Some(Subject::Header),
        "responsetext" | "responsebody" | "body" | "text" => Some(Subject::Body),
        "statuscode" | "status" | "responsecode" => Some(Subject::Status),
        "responsetime" | "latency" => Some(Subject::Latency),
        "responsesize" | "size" => Some(Subject::Size),
        _ => None,
    }
}

fn map_op(raw: &str) -> Option<Op> {
    match raw.trim().to_ascii_lowercase().replace(['_', '-', ' '], "").as_str() {
        "equal" | "equals" | "equalto" | "eq" | "==" | "=" => Some(Op::Equals),
        "notequal" | "notequals" | "ne" | "!=" => Some(Op::NotEquals),
        "contains" | "contain" => Some(Op::Contains),
        "notcontains" | "notcontain" | "doesnotcontain" => Some(Op::NotContains),
        "matches" | "match" | "regex" | "regular" => Some(Op::Matches),
        "exists" | "exist" | "notnull" | "isnotnull" | "isnotempty" | "notempty" => Some(Op::Exists),
        "notexists" | "notexist" | "isnull" | "isempty" | "empty" | "null" => Some(Op::NotExists),
        "<" | "lt" | "less" | "lessthan" => Some(Op::Less),
        ">" | "gt" | "greater" | "greaterthan" => Some(Op::Greater),
        "istype" | "type" => Some(Op::IsType),
        _ => None,
    }
}

fn proc_field(proc: &Value, keys: &[&str]) -> String {
    let data = proc.get("data");
    for key in keys {
        if let Some(d) = data
            && let Some(val) = d.get(*key)
            && !val.is_null()
        {
            return value_text(Some(val));
        }
        if let Some(val) = proc.get(*key)
            && !val.is_null()
        {
            return value_text(Some(val));
        }
    }
    String::new()
}

fn apply_processors(entry: &Value, api_obj: &Value, target: &str, built: &mut HttpRequest, lost: &mut Vec<Lost>) {
    let pre_list = entry
        .get("preProcessors")
        .or_else(|| api_obj.get("preProcessors"))
        .and_then(Value::as_array);
    if let Some(list) = pre_list {
        for proc in list {
            if !is_enabled(proc) {
                continue;
            }
            if text(proc, "type").trim() == "customScript" {
                let src = proc_field(proc, &["script", "data"]);
                script::apply_script(built, lost, target, ScriptPhase::PreRequest, &src);
            } else if let Ok(raw_json) = serde_json::to_string(proc) {
                lost::push_script(lost, target, ScriptPhase::PreRequest, &raw_json);
            }
        }
    }

    let post_list = entry
        .get("postProcessors")
        .or_else(|| api_obj.get("postProcessors"))
        .and_then(Value::as_array);
    if let Some(list) = post_list {
        for proc in list {
            if !is_enabled(proc) {
                continue;
            }
            let kind = text(proc, "type").trim();
            match kind {
                "customScript" => {
                    let src = proc_field(proc, &["script", "data"]);
                    script::apply_script(built, lost, target, ScriptPhase::PostResponse, &src);
                }
                "assertion" => {
                    let subj_raw = proc_field(proc, &["subject"]);
                    let expr_raw = proc_field(proc, &["expression", "target", "path"]);
                    let op_raw = proc_field(proc, &["assertion", "op", "operator", "condition"]);
                    let expected_raw = proc_field(proc, &["value", "expected"]);
                    if let Some(subject) = map_subject(&subj_raw)
                        && let Some(op) = map_op(&op_raw)
                        && subject.ops().contains(&op)
                    {
                        let ass_target = if subject == Subject::Json {
                            normalize_json_path(&expr_raw)
                        } else if subject == Subject::Header {
                            expr_raw.trim().to_string()
                        } else {
                            String::new()
                        };
                        let expected = if op == Op::IsType {
                            expected_raw.trim().to_ascii_lowercase()
                        } else {
                            expected_raw
                        };
                        let valid_target = !subject.has_target() || !ass_target.trim().is_empty();
                        let valid_type = op != Op::IsType || assertions::TYPES.contains(&expected.as_str());
                        if valid_target && valid_type {
                            built.assertions.push(Assertion {
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
                    if let Ok(raw_json) = serde_json::to_string(proc) {
                        lost::push_script(lost, target, ScriptPhase::PostResponse, &raw_json);
                    }
                }
                "extractVariable" => {
                    let var_name = proc_field(proc, &["variableName", "name", "varName"])
                        .trim()
                        .trim_start_matches("{{")
                        .trim_end_matches("}}")
                        .trim()
                        .to_string();
                    let subj_raw = proc_field(proc, &["subject"]);
                    let expr_raw = proc_field(proc, &["expression", "target", "path"]);
                    if !var_name.is_empty()
                        && let Some(subject) = map_subject(&subj_raw)
                    {
                        let ext_target = if subject == Subject::Json {
                            normalize_json_path(&expr_raw)
                        } else if subject == Subject::Header {
                            expr_raw.trim().to_string()
                        } else {
                            String::new()
                        };
                        let valid_target = !subject.has_target() || !ext_target.trim().is_empty();
                        if valid_target {
                            built.extractions.push(Extraction {
                                enabled: true,
                                name: var_name,
                                subject,
                                target: ext_target,
                            });
                            continue;
                        }
                    }
                    if let Ok(raw_json) = serde_json::to_string(proc) {
                        lost::push_script(lost, target, ScriptPhase::PostResponse, &raw_json);
                    }
                }
                _ => {
                    if let Ok(raw_json) = serde_json::to_string(proc) {
                        lost::push_script(lost, target, ScriptPhase::PostResponse, &raw_json);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn project_environments_folders_params_and_examples() {
        let doc = json!({
            "apifoxProject": "1.0.0",
            "$schema": { "app": "apifox", "type": "project" },
            "info": { "name": "Orders Service" },
            "globalVariables": [{ "name": "API_VER", "value": "v1", "enable": true }],
            "environments": [
                {
                    "id": 1,
                    "name": "Dev",
                    "baseUrl": "https://dev.api.test",
                    "parameters": [
                        { "name": "tenant", "value": "acme", "enable": true },
                        { "name": "disabledVar", "value": "x", "enable": false }
                    ]
                },
                {
                    "id": 2,
                    "name": "Prod",
                    "baseUrls": [
                        { "id": "default", "name": "Default", "url": "https://api.test" },
                        { "id": "cdn", "name": "CDN_URL", "url": "https://cdn.test" }
                    ]
                }
            ],
            "schemaCollection": [
                {
                    "name": "Models",
                    "items": [
                        {
                            "id": 100,
                            "name": "CreateOrder",
                            "schema": {
                                "jsonSchema": {
                                    "type": "object",
                                    "required": ["sku"],
                                    "properties": {
                                        "sku": { "type": "string", "example": "SKU-1" },
                                        "qty": { "type": "integer", "default": 2 }
                                    }
                                }
                            }
                        },
                        {
                            "id": "resp-order",
                            "name": "OrderResponse",
                            "jsonSchema": {
                                "type": "object",
                                "required": ["id"],
                                "properties": {
                                    "id": { "type": "string" }
                                }
                            }
                        }
                    ]
                }
            ],
            "apiCollection": [
                {
                    "id": 0,
                    "name": "根目录",
                    "auth": { "type": "bearer", "bearer": [{ "key": "token", "value": "{{tok}}" }] },
                    "items": [
                        {
                            "name": "Orders",
                            "description": "Order APIs",
                            "commonParameters": { "header": [{ "name": "X-Group", "value": "orders" }] },
                            "items": [
                                {
                                    "name": "Create Order",
                                    "api": {
                                        "method": "post",
                                        "path": "/orders/{orgId}",
                                        "description": "Creates an order",
                                        "parameters": {
                                            "path": [
                                                { "name": "orgId", "example": "org-1", "required": true, "type": "string", "description": "Org identifier" }
                                            ],
                                            "query": [
                                                { "name": "dryRun", "value": "true", "required": false, "type": "boolean", "enum": ["true", "false"] }
                                            ],
                                            "header": [
                                                { "name": "X-Trace", "example": "t1", "type": "string", "format": "uuid" }
                                            ],
                                            "cookie": [
                                                { "name": "sid", "value": "s1" }
                                            ]
                                        },
                                        "requestBody": {
                                            "type": "application/json",
                                            "jsonSchema": { "$ref": "#/definitions/100" }
                                        },
                                        "responses": [
                                            {
                                                "id": 201,
                                                "name": "Created",
                                                "code": 201,
                                                "contentType": "application/json",
                                                "jsonSchema": { "$ref": "#/components/schemas/OrderResponse" }
                                            }
                                        ],
                                        "responseExamples": [
                                            {
                                                "name": "Created sample",
                                                "responseId": 201,
                                                "data": { "id": "ord-99" }
                                            }
                                        ]
                                    }
                                }
                            ]
                        }
                    ]
                }
            ]
        });

        assert!(is_apidog(&doc));
        assert_eq!(name(&doc), Some("Orders Service"));

        let bundle = parse(&doc);
        assert_eq!(bundle.environment, vec![("API_VER".into(), "v1".into())]);
        assert_eq!(
            bundle.environments,
            vec![
                ImportEnvironment {
                    name: "Dev".into(),
                    variables: vec![
                        ("BASE_URL".into(), "https://dev.api.test".into()),
                        ("tenant".into(), "acme".into()),
                    ],
                },
                ImportEnvironment {
                    name: "Prod".into(),
                    variables: vec![
                        ("BASE_URL".into(), "https://api.test".into()),
                        ("CDN_URL".into(), "https://cdn.test".into()),
                    ],
                },
            ]
        );
        assert_eq!(
            bundle.auth,
            Auth::Bearer {
                token: "{{tok}}".into()
            }
        );

        let [
            ImportItem::Folder {
                name: orders_name,
                settings: orders_settings,
                children,
            },
        ] = &bundle.items[..]
        else {
            panic!("expected unwrapped Orders folder: {:?}", bundle.items);
        };
        assert_eq!(orders_name, "Orders");
        assert_eq!(orders_settings.description, "Order APIs");
        assert_eq!(orders_settings.headers, [kv("X-Group", "orders", true)]);

        let [
            ImportItem::Api {
                name: create_name,
                request,
            },
        ] = &children[..]
        else {
            panic!("expected Create Order api");
        };
        assert_eq!(create_name, "Create Order");
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.uri, "{{BASE_URL}}/orders/{orgId}");
        assert_eq!(request.description, "Creates an order");
        assert_eq!(
            request.variables,
            [kv("orgId", "org-1", true)
                .with_description("Org identifier")
                .with_param_schema("string", Some(true), "", Vec::<String>::new())]
        );
        assert_eq!(
            request.query,
            [kv("dryRun", "true", true).with_param_schema("boolean", Some(false), "", ["true", "false"])]
        );
        assert_eq!(
            request.headers,
            [
                kv("X-Trace", "t1", true).with_param_schema("string", None, "uuid", Vec::<String>::new()),
                kv("Cookie", "sid=s1", true),
            ]
        );

        let schema_str = request.request_schema.as_deref().expect("resolved request schema");
        let parsed_schema: Value = serde_json::from_str(schema_str).expect("schema json");
        assert_eq!(parsed_schema["required"], json!(["sku"]));
        let sample_body: Value = serde_json::from_str(&request.body).expect("generated sample body");
        assert_eq!(sample_body, json!({ "sku": "SKU-1", "qty": 2 }));

        assert_eq!(request.examples.len(), 1);
        assert_eq!(request.examples[0].status, 201);
        assert_eq!(request.examples[0].name, "Created sample");
        assert!(request.examples[0].body.contains("\"ord-99\""));

        assert_eq!(request.assertions.len(), 1);
        assert!(request.assertions[0].imported);
        assert_eq!(request.assertions[0].subject, Subject::Schema);
    }

    #[test]
    fn bodies_auth_and_processors() {
        let doc = json!({
            "apidogProject": "1.0.0",
            "apiCollection": [
                {
                    "name": "Multipart + Aws",
                    "method": "POST",
                    "path": "https://a.test/upload",
                    "requestBody": {
                        "type": "multipart/form-data",
                        "parameters": [
                            { "name": "title", "type": "string", "example": "doc" },
                            { "name": "file", "type": "file", "example": "/tmp/a.png" }
                        ]
                    },
                    "auth": {
                        "type": "awsv4",
                        "awsv4": {
                            "accessKey": "AKIA1",
                            "secretKey": "SEC1",
                            "sessionToken": "TOK1",
                            "region": "us-east-1",
                            "service": "s3"
                        }
                    }
                },
                {
                    "name": "Binary + Jwt",
                    "method": "PUT",
                    "path": "https://a.test/bin",
                    "requestBody": { "type": "application/octet-stream", "example": "/tmp/blob.bin" },
                    "auth": {
                        "type": "jwt",
                        "jwt": { "algorithm": "ES256", "secret": "PEM", "headerPrefix": "" }
                    }
                },
                {
                    "name": "GraphQL + Implicit",
                    "method": "POST",
                    "path": "https://a.test/gql",
                    "requestBody": {
                        "type": "graphql",
                        "graphql": { "query": "query { me { id } }", "variables": "{\"id\":1}" }
                    },
                    "auth": {
                        "type": "oauth2",
                        "oauth2": {
                            "grantType": "implicit",
                            "authUrl": "https://id.test/auth",
                            "redirectUri": "http://127.0.0.1:8899/callback",
                            "clientId": "app"
                        }
                    }
                },
                {
                    "name": "Login",
                    "method": "POST",
                    "path": "https://a.test/login",
                    "preProcessors": [
                        { "type": "customScript", "data": { "script": "pm.variables.set('nonce', '1');" } }
                    ],
                    "postProcessors": [
                        {
                            "type": "assertion",
                            "data": { "subject": "responseJson", "expression": "$.ok", "assertion": "equal", "value": "true" }
                        },
                        {
                            "type": "extractVariable",
                            "data": { "variableName": "{{refresh}}", "subject": "responseHeader", "expression": "X-Refresh" }
                        },
                        {
                            "type": "customScript",
                            "data": "pm.response.to.have.status(200);\npm.environment.set('tok', pm.response.json().token);\nconsole.log('keep me');"
                        }
                    ]
                },
                {
                    "name": "Unsupported",
                    "method": "POST",
                    "path": "https://a.test/x",
                    "requestBody": { "type": "msgpack" },
                    "auth": { "type": "hawk" }
                }
            ]
        });

        let bundle = parse(&doc);
        assert_eq!(bundle.items.len(), 5);

        let ImportItem::Api { request: r0, .. } = &bundle.items[0] else {
            panic!("expected r0");
        };
        assert_eq!(r0.content_type, ContentType::Multipart);
        let fields: Vec<KvParam> = serde_json::from_str(&r0.body).expect("multipart json");
        assert_eq!(
            fields,
            [kv("title", "doc", true), kv("file", "file:///tmp/a.png", true)]
        );
        assert_eq!(
            r0.auth,
            Auth::from(AwsSigV4 {
                access_key: "AKIA1".into(),
                secret_key: "SEC1".into(),
                session_token: "TOK1".into(),
                region: "us-east-1".into(),
                service: "s3".into(),
            })
        );

        let ImportItem::Api { request: r1, .. } = &bundle.items[1] else {
            panic!("expected r1");
        };
        assert_eq!(r1.content_type, ContentType::Binary);
        assert_eq!(r1.body, "file:///tmp/blob.bin");
        assert_eq!(
            r1.auth,
            Auth::from(JwtBearer {
                algorithm: JwtAlgorithm::ES256,
                secret: "PEM".into(),
                secret_base64: false,
                payload: "{}".into(),
                header: String::new(),
                place: KeyPlace::Header,
                prefix: String::new(),
                query_param: "token".into(),
            })
        );

        let ImportItem::Api { request: r2, .. } = &bundle.items[2] else {
            panic!("expected r2");
        };
        assert_eq!(r2.content_type, ContentType::GraphQl);
        assert_eq!(
            r2.auth,
            Auth::from(OAuth2 {
                grant: Grant::Implicit,
                authorization_url: "https://id.test/auth".into(),
                redirect_port: 8899,
                client_id: "app".into(),
                client_auth: ClientAuth::Header,
                ..Default::default()
            })
        );

        let ImportItem::Api { request: login, .. } = &bundle.items[3] else {
            panic!("expected login");
        };
        assert_eq!(
            login.assertions,
            vec![
                Assertion {
                    enabled: true,
                    subject: Subject::Json,
                    target: "$.ok".into(),
                    op: Op::Equals,
                    expected: "true".into(),
                    imported: false,
                },
                Assertion {
                    enabled: true,
                    subject: Subject::Status,
                    target: String::new(),
                    op: Op::Equals,
                    expected: "200".into(),
                    imported: false,
                },
            ]
        );
        assert_eq!(
            login.extractions,
            vec![
                Extraction {
                    enabled: true,
                    name: "refresh".into(),
                    subject: Subject::Header,
                    target: "X-Refresh".into(),
                },
                Extraction {
                    enabled: true,
                    name: "tok".into(),
                    subject: Subject::Json,
                    target: "$.token".into(),
                },
            ]
        );
        assert!(login.description.contains("pm.variables.set('nonce', '1');"));
        assert!(login.description.contains("console.log('keep me');"));

        assert_eq!(bundle.lost.len(), 4);
    }

    #[test]
    fn wrapped_apidog_project_object_variant() {
        let doc = json!({
            "apidogProject": {
                "version": "2.7.0",
                "info": { "title": "Wrapped Apidog" },
                "environments": [{ "name": "Prod", "baseUrl": "https://prod.test" }],
                "apiCollection": [
                    {
                        "name": "Users",
                        "items": [
                            {
                                "method": "GET",
                                "path": "/users",
                                "summary": "List Users",
                                "responseExamples": [
                                    { "code": 200, "name": "OK", "contentType": "application/json", "value": [{ "id": 1 }] }
                                ]
                            }
                        ]
                    }
                ]
            }
        });
        assert!(is_apidog(&doc));
        assert_eq!(name(&doc), Some("Wrapped Apidog"));
        let bundle = parse(&doc);
        let [
            ImportItem::Folder {
                name: folder_name,
                children,
                ..
            },
        ] = &bundle.items[..]
        else {
            panic!("expected Users folder");
        };
        assert_eq!(folder_name, "Users");
        let [
            ImportItem::Api {
                name: api_name,
                request,
            },
        ] = &children[..]
        else {
            panic!("expected List Users");
        };
        assert_eq!(api_name, "List Users");
        assert_eq!(request.uri, "{{BASE_URL}}/users");
        assert_eq!(request.examples.len(), 1);
    }
}
