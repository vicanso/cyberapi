//! An Insomnia export (format 4) as folders and APIs (F10.3).
//!
//! `request_group`s are folders, nested by `parentId`; `request`s are APIs;
//! the `data` of `environment`s are environment variables. Insomnia writes a
//! variable as `{{ _.name }}`; it comes in as this app's `{{name}}`.
//!
//! B4: a request's and a folder's `authentication` — Basic, Bearer, API
//! key, Digest — come in as this app's (#66); the base environment's data
//! are the variables every environment shares, and each sub-environment is
//! a named environment.

use super::{ImportBundle, ImportEnvironment, ImportItem, api, content_type_of, guess_content_type, kv};
use crate::auth::{Auth, KeyPlace};
use crate::graphql::GraphQlBody;
use crate::model::FolderSettings;
use crate::model::{ContentType, KvParam};
use serde_json::Value;

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// `{{ _.host }}` → `{{host}}`.
fn variables(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let name = after[..end].trim();
                let name = name.strip_prefix("_.").unwrap_or(name);
                out.push_str(&format!("{{{{{name}}}}}"));
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn pairs(value: Option<&Value>) -> Vec<KvParam> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    kv(
                        text(item, "name"),
                        &variables(text(item, "value")),
                        !item.get("disabled").and_then(Value::as_bool).unwrap_or(false),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn request(resource: &Value) -> ImportItem {
    let (name, mut built) = api(
        text(resource, "name"),
        text(resource, "method"),
        &variables(text(resource, "url")),
    );
    built.query.extend(pairs(resource.get("parameters")));
    built.headers = pairs(resource.get("headers"));
    built.headers.retain(|header| {
        if header.key.eq_ignore_ascii_case("content-type")
            && let Some(known) = content_type_of(&header.value)
        {
            built.content_type = known;
            return false;
        }
        true
    });
    if let Some(body) = resource.get("body") {
        let mime = text(body, "mimeType");
        if let Some(known) = content_type_of(mime) {
            built.content_type = known;
        }
        match built.content_type {
            ContentType::Form | ContentType::Multipart => {
                built.body = serde_json::to_string(&pairs(body.get("params"))).unwrap_or_default();
            }
            // `{"query": …, "variables": {…}}` as text.
            ContentType::GraphQl => {
                built.body = GraphQlBody::read(&variables(text(body, "text"))).to_body();
            }
            _ => {
                built.body = variables(text(body, "text"));
                if mime.is_empty() && !built.body.is_empty() {
                    built.content_type = guess_content_type(&built.body);
                }
            }
        }
    }
    if let Some(own) = resource.get("authentication").and_then(auth) {
        built.auth = own;
    }
    // F3.
    built.description = text(resource, "description").trim().to_string();
    ImportItem::Api {
        name,
        request: Box::new(built),
    }
}

/// B4: an `authentication` object as this app's; `None` where there is
/// none (`{}`) or a kind this app has no counterpart of (OAuth, AWS…),
/// which then inherits. One switched off signs in with nothing.
fn auth(value: &Value) -> Option<Auth> {
    let field = |key: &str| variables(text(value, key));
    if value.get("disabled").and_then(Value::as_bool) == Some(true) {
        return Some(Auth::None);
    }
    Some(match text(value, "type") {
        "none" => Auth::None,
        "basic" => Auth::Basic {
            username: field("username"),
            password: field("password"),
        },
        "digest" => Auth::Digest {
            username: field("username"),
            password: field("password"),
        },
        "bearer" => Auth::Bearer { token: field("token") },
        "apikey" => Auth::ApiKey {
            name: field("key"),
            value: field("value"),
            place: if text(value, "addTo") == "queryParams" {
                KeyPlace::Query
            } else {
                KeyPlace::Header
            },
        },
        _ => return None,
    })
}

/// An environment's data, name and value as text.
fn data(resource: &Value) -> Vec<(String, String)> {
    resource
        .get("data")
        .and_then(Value::as_object)
        .map(|data| {
            data.iter()
                .map(|(key, value)| {
                    let value = match value {
                        Value::String(text) => variables(text),
                        other => other.to_string(),
                    };
                    (key.clone(), value)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn children(resources: &[Value], parent: &str) -> Vec<ImportItem> {
    resources
        .iter()
        .filter(|resource| text(resource, "parentId") == parent)
        .filter_map(|resource| match text(resource, "_type") {
            "request" => Some(request(resource)),
            "request_group" => Some(ImportItem::Folder {
                name: text(resource, "name").to_string(),
                children: children(resources, text(resource, "_id")),
                settings: FolderSettings {
                    auth: resource.get("authentication").and_then(auth).unwrap_or_default(),
                    headers: Vec::new(),
                    description: text(resource, "description").trim().to_string(),
                },
            }),
            _ => None,
        })
        .collect()
}

pub(super) fn parse(json: &Value) -> ImportBundle {
    let resources: Vec<Value> = json
        .get("resources")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // Whatever hangs on a workspace (or on nothing this file has) is on top.
    let ids: Vec<&str> = resources
        .iter()
        .filter(|resource| matches!(text(resource, "_type"), "request" | "request_group"))
        .map(|resource| text(resource, "_id"))
        .collect();
    let mut tops: Vec<&str> = Vec::new();
    for resource in &resources {
        let parent = text(resource, "parentId");
        if matches!(text(resource, "_type"), "request" | "request_group")
            && !ids.contains(&parent)
            && !tops.contains(&parent)
        {
            tops.push(parent);
        }
    }
    let items = tops.into_iter().flat_map(|top| children(&resources, top)).collect();
    // A sub-environment hangs on another environment; the base one on the
    // workspace.
    let environments: Vec<&Value> = resources
        .iter()
        .filter(|resource| text(resource, "_type") == "environment")
        .collect();
    let ids: Vec<&str> = environments.iter().map(|resource| text(resource, "_id")).collect();
    let (subs, bases): (Vec<&Value>, Vec<&Value>) = environments
        .into_iter()
        .partition(|resource| ids.contains(&text(resource, "parentId")));
    ImportBundle {
        items,
        environment: bases.into_iter().flat_map(data).collect(),
        environments: subs
            .into_iter()
            .map(|resource| ImportEnvironment {
                name: text(resource, "name").trim().to_string(),
                variables: data(resource),
            })
            .filter(|environment| !environment.name.is_empty())
            .collect(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HttpMethod;
    use pretty_assertions::assert_eq;

    #[test]
    fn groups_nest_by_parent() {
        let json: Value = serde_json::from_str(
            r#"{"_type": "export", "__export_format": 4, "resources": [
                {"_id": "req_2", "_type": "request", "parentId": "fld_1", "name": "login", "method": "POST",
                 "url": "{{ _.host }}/login", "body": {"mimeType": "application/json", "text": "{\"a\": \"{{ _.user }}\"}"},
                 "parameters": [{"name": "v", "value": "1"}], "headers": [{"name": "X-A", "value": "a", "disabled": true}],
                 "authentication": {"type": "basic", "username": "u", "password": "p"}},
                {"_id": "fld_1", "_type": "request_group", "parentId": "wrk_1", "name": "users"},
                {"_id": "req_1", "_type": "request", "parentId": "wrk_1", "name": "health", "method": "GET", "url": "https://a.test/h"},
                {"_id": "req_3", "_type": "request", "parentId": "wrk_1", "name": "me", "method": "POST", "url": "{{ _.host }}/graphql",
                 "body": {"mimeType": "application/graphql", "text": "{\"query\": \"{ me(id: $id) { id } }\", \"variables\": {\"id\": \"{{ _.user }}\"}}"}},
                {"_id": "env_1", "_type": "environment", "parentId": "wrk_1", "data": {"host": "https://a.test", "port": 8080}},
                {"_id": "wrk_1", "_type": "workspace", "name": "Shop"}
            ]}"#,
        )
        .expect("JSON");
        let bundle = parse(&json);
        assert_eq!(
            bundle.environment,
            [
                ("host".to_string(), "https://a.test".to_string()),
                ("port".to_string(), "8080".to_string())
            ]
        );
        let ImportItem::Folder { name, children, .. } = &bundle.items[0] else {
            panic!("a folder first: {:?}", bundle.items);
        };
        assert_eq!(name, "users");
        let ImportItem::Api { request, .. } = &children[0] else {
            panic!("an API");
        };
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.uri, "{{host}}/login");
        assert_eq!(request.body, "{\"a\": \"{{user}}\"}");
        assert_eq!(request.content_type, ContentType::Json);
        assert_eq!(request.query, [kv("v", "1", true)]);
        assert_eq!(request.headers, [kv("X-A", "a", false)]);
        assert_eq!(
            request.auth,
            Auth::Basic {
                username: "u".into(),
                password: "p".into()
            }
        );
        assert!(matches!(&bundle.items[1], ImportItem::Api { name, .. } if name == "health"));
        let ImportItem::Api { request, .. } = &bundle.items[2] else {
            panic!("an API");
        };
        assert_eq!(request.content_type, ContentType::GraphQl);
        let graphql = GraphQlBody::read(&request.body);
        assert_eq!(graphql.query, "{ me(id: $id) { id } }");
        assert_eq!(
            graphql.variables, "{\n    \"id\": \"{{user}}\"\n}",
            "its templates as this app writes them"
        );
    }

    /// B4: requests' and folders' auth; the base environment shared, each
    /// sub-environment named.
    #[test]
    fn auth_and_sub_environments() {
        let json: Value = serde_json::from_str(
            r#"{"_type": "export", "__export_format": 4, "resources": [
                {"_id": "wrk_1", "_type": "workspace", "name": "Shop"},
                {"_id": "env_base", "_type": "environment", "parentId": "wrk_1", "name": "Base", "data": {"host": "https://shop.test"}},
                {"_id": "env_dev", "_type": "environment", "parentId": "env_base", "name": "dev", "data": {"host": "http://localhost:8080", "token": "{{ _.secret }}"}},
                {"_id": "fld_1", "_type": "request_group", "parentId": "wrk_1", "name": "admin",
                 "authentication": {"type": "bearer", "token": "{{ _.token }}"}},
                {"_id": "req_1", "_type": "request", "parentId": "fld_1", "name": "stats", "method": "GET",
                 "url": "{{ _.host }}/stats", "authentication": {}},
                {"_id": "req_2", "_type": "request", "parentId": "wrk_1", "name": "key", "method": "GET",
                 "url": "{{ _.host }}/k", "authentication": {"type": "apikey", "key": "X-Key", "value": "v", "addTo": "queryParams"}},
                {"_id": "req_3", "_type": "request", "parentId": "wrk_1", "name": "off", "method": "GET",
                 "url": "{{ _.host }}/off", "authentication": {"type": "basic", "username": "u", "password": "p", "disabled": true}}
            ]}"#,
        )
        .expect("JSON");
        let bundle = parse(&json);
        assert_eq!(
            bundle.environment,
            [("host".to_string(), "https://shop.test".to_string())]
        );
        assert_eq!(
            bundle.environments,
            [ImportEnvironment {
                name: "dev".into(),
                variables: vec![
                    ("host".into(), "http://localhost:8080".into()),
                    ("token".into(), "{{secret}}".into())
                ],
            }]
        );
        let ImportItem::Folder { settings, children, .. } = &bundle.items[0] else {
            panic!("a folder");
        };
        assert_eq!(
            settings.auth,
            Auth::Bearer {
                token: "{{token}}".into()
            }
        );
        let auth = |item: &ImportItem| match item {
            ImportItem::Api { request, .. } => request.auth.clone(),
            ImportItem::Folder { .. } => panic!("an API"),
        };
        assert_eq!(auth(&children[0]), Auth::Inherit, "an empty one inherits");
        assert_eq!(
            auth(&bundle.items[1]),
            Auth::ApiKey {
                name: "X-Key".into(),
                value: "v".into(),
                place: KeyPlace::Query
            }
        );
        assert_eq!(auth(&bundle.items[2]), Auth::None, "switched off");
    }
}
