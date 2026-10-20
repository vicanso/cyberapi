//! An Insomnia export (format 4) as folders and APIs (F10.3).
//!
//! `request_group`s are folders, nested by `parentId`; `request`s are APIs;
//! the `data` of `environment`s are environment variables. Insomnia writes a
//! variable as `{{ _.name }}`; it comes in as this app's `{{name}}`.

use super::{ImportBundle, ImportItem, api, content_type_of, guess_content_type, kv};
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
            _ => {
                built.body = variables(text(body, "text"));
                if mime.is_empty() && !built.body.is_empty() {
                    built.content_type = guess_content_type(&built.body);
                }
            }
        }
    }
    if let Some(auth) = resource.get("authentication")
        && text(auth, "type") == "basic"
    {
        built.auth = vec![kv(text(auth, "username"), text(auth, "password"), true)];
    }
    ImportItem::Api { name, request: built }
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
    let environment = resources
        .iter()
        .filter(|resource| text(resource, "_type") == "environment")
        .filter_map(|resource| resource.get("data").and_then(Value::as_object))
        .flat_map(|data| {
            data.iter().map(|(key, value)| {
                let value = match value {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                (key.clone(), value)
            })
        })
        .collect();
    ImportBundle { items, environment }
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
        let ImportItem::Folder { name, children } = &bundle.items[0] else {
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
        assert_eq!(request.auth, [kv("u", "p", true)]);
        assert!(matches!(&bundle.items[1], ImportItem::Api { name, .. } if name == "health"));
    }
}
