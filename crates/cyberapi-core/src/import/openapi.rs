//! Swagger 2.0 and OpenAPI 3 as folders and APIs (F10.3).
//!
//! As before for Swagger: every path and method is an API named by its
//! `summary` (or `operationId`), in a folder per first tag; its URI starts
//! with an environment variable named after the title, whose value is the
//! server; query parameters take their examples; a JSON body is an example
//! made from its schema. OpenAPI 3 is read the same way (#28), with
//! `servers` and `requestBody`. `$ref`s are followed — not just one level —
//! up to a depth that stops loops.

use super::{ImportBundle, ImportItem, kv};
use crate::model::{ContentType, HttpMethod, HttpRequest, KvParam};
use serde_json::{Map, Value, json};

const METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "options", "head"];
const MAX_DEPTH: usize = 8;

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// What `$ref` points to in this document: `#/definitions/User`.
fn resolve<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    match schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|path| path.strip_prefix('#'))
    {
        Some(pointer) => root.pointer(pointer).unwrap_or(schema),
        None => schema,
    }
}

/// An example value of a schema: its own example, or one made from its type.
fn example(root: &Value, schema: &Value, depth: usize) -> Value {
    if depth > MAX_DEPTH {
        return Value::Null;
    }
    let schema = resolve(root, schema);
    if let Some(example) = schema.get("example") {
        return example.clone();
    }
    if let Some(first) = schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
    {
        return first.clone();
    }
    match text(schema, "type") {
        "string" => json!(""),
        "integer" | "number" => json!(0),
        "boolean" => json!(false),
        "array" => json!([example(root, schema.get("items").unwrap_or(&Value::Null), depth + 1)]),
        _ => match schema.get("properties").and_then(Value::as_object) {
            Some(properties) => Value::Object(
                properties
                    .iter()
                    .map(|(name, property)| (name.clone(), example(root, property, depth + 1)))
                    .collect::<Map<_, _>>(),
            ),
            None if schema.get("allOf").is_some() => {
                let mut merged = Map::new();
                for part in schema.get("allOf").and_then(Value::as_array).into_iter().flatten() {
                    if let Value::Object(fields) = example(root, part, depth + 1) {
                        merged.extend(fields);
                    }
                }
                Value::Object(merged)
            }
            None => json!({}),
        },
    }
}

/// A parameter's value for the query or header list.
fn parameter_value(root: &Value, parameter: &Value) -> String {
    let value = parameter
        .get("example")
        .cloned()
        .or_else(|| parameter.get("default").cloned())
        .or_else(|| parameter.get("schema").map(|schema| example(root, schema, 0)))
        .unwrap_or(Value::Null);
    match value {
        Value::String(text) => text,
        Value::Null => String::new(),
        Value::Object(map) if map.is_empty() => String::new(),
        other => other.to_string(),
    }
}

/// The environment variable for the server: the title without spaces.
fn server(json: &Value) -> Option<(String, String)> {
    let title: String = json
        .get("info")
        .map(|info| text(info, "title"))
        .unwrap_or_default()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect();
    let name = if title.is_empty() { "HOST".to_string() } else { title };
    let value = if json.get("swagger").is_some() {
        let scheme = json
            .get("schemes")
            .and_then(Value::as_array)
            .and_then(|schemes| schemes.first())
            .and_then(Value::as_str)
            .unwrap_or("https");
        let host = text(json, "host");
        if host.is_empty() {
            return None;
        }
        format!("{scheme}://{host}{}", text(json, "basePath").trim_end_matches('/'))
    } else {
        let url = json
            .get("servers")
            .and_then(Value::as_array)
            .and_then(|servers| servers.first())
            .map(|server| text(server, "url"))
            .unwrap_or_default();
        if url.is_empty() {
            return None;
        }
        url.trim_end_matches('/').to_string()
    };
    Some((name, value))
}

fn operation(
    json: &Value,
    prefix: &str,
    path: &str,
    method: &str,
    op: &Value,
    shared: &[Value],
) -> (String, ImportItem) {
    let name = [text(op, "summary"), text(op, "operationId")]
        .into_iter()
        .find(|name| !name.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{} {path}", method.to_ascii_uppercase()));
    let mut request = HttpRequest {
        method: HttpMethod::parse_lenient(method),
        uri: format!("{prefix}{path}"),
        ..Default::default()
    };
    let parameters: Vec<&Value> = shared
        .iter()
        .chain(op.get("parameters").and_then(Value::as_array).into_iter().flatten())
        .map(|parameter| resolve(json, parameter))
        .collect();
    let mut form: Vec<KvParam> = Vec::new();
    for parameter in parameters {
        let name = text(parameter, "name");
        match text(parameter, "in") {
            "query" => request.query.push(kv(name, &parameter_value(json, parameter), true)),
            "header" => request.headers.push(kv(name, &parameter_value(json, parameter), true)),
            "formData" => form.push(kv(name, &parameter_value(json, parameter), true)),
            // Swagger 2's body.
            "body" => {
                if let Some(schema) = parameter.get("schema") {
                    request.content_type = ContentType::Json;
                    request.body = crate::format::json(&example(json, schema, 0));
                }
            }
            _ => {}
        }
    }
    if !form.is_empty() {
        let multipart = op
            .get("consumes")
            .and_then(Value::as_array)
            .is_some_and(|types| types.iter().any(|kind| kind.as_str() == Some("multipart/form-data")));
        request.content_type = if multipart {
            ContentType::Multipart
        } else {
            ContentType::Form
        };
        request.body = serde_json::to_string(&form).unwrap_or_default();
    }
    // OpenAPI 3's body.
    if let Some(content) = op
        .get("requestBody")
        .map(|body| resolve(json, body))
        .and_then(|body| body.get("content"))
        .and_then(Value::as_object)
    {
        if let Some(media) = content
            .iter()
            .find(|(kind, _)| kind.contains("json"))
            .map(|(_, media)| media)
        {
            let value = media
                .get("example")
                .cloned()
                .or_else(|| media.get("schema").map(|schema| example(json, schema, 0)))
                .unwrap_or_else(|| json!({}));
            request.content_type = ContentType::Json;
            request.body = crate::format::json(&value);
        } else if let Some((kind, media)) = content
            .iter()
            .find(|(kind, _)| kind.contains("form-urlencoded") || kind.contains("multipart"))
        {
            let fields: Vec<KvParam> = media
                .get("schema")
                .map(|schema| resolve(json, schema))
                .and_then(|schema| schema.get("properties"))
                .and_then(Value::as_object)
                .map(|properties| {
                    properties
                        .iter()
                        .map(|(name, property)| {
                            let value = match example(json, property, 1) {
                                Value::String(text) => text,
                                other => other.to_string(),
                            };
                            kv(name, &value, true)
                        })
                        .collect()
                })
                .unwrap_or_default();
            request.content_type = if kind.contains("multipart") {
                ContentType::Multipart
            } else {
                ContentType::Form
            };
            request.body = serde_json::to_string(&fields).unwrap_or_default();
        }
    }
    let tag = op
        .get("tags")
        .and_then(Value::as_array)
        .and_then(|tags| tags.first())
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    (tag, ImportItem::Api { name, request })
}

pub(super) fn parse(json: &Value) -> ImportBundle {
    let server = server(json);
    let prefix = server
        .as_ref()
        .map(|(name, _)| format!("{{{{{name}}}}}"))
        .unwrap_or_default();
    let mut untagged: Vec<ImportItem> = Vec::new();
    let mut folders: Vec<(String, Vec<ImportItem>)> = Vec::new();
    for (path, item) in json.get("paths").and_then(Value::as_object).into_iter().flatten() {
        let shared: Vec<Value> = item
            .get("parameters")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for method in METHODS {
            let Some(op) = item.get(method) else { continue };
            let (tag, api) = operation(json, &prefix, path, method, op, &shared);
            if tag.is_empty() {
                untagged.push(api);
            } else if let Some((_, children)) = folders.iter_mut().find(|(name, _)| *name == tag) {
                children.push(api);
            } else {
                folders.push((tag, vec![api]));
            }
        }
    }
    let items = folders
        .into_iter()
        .map(|(name, children)| ImportItem::Folder { name, children })
        .chain(untagged)
        .collect();
    ImportBundle {
        items,
        environment: server.into_iter().collect(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn first_api(items: &[ImportItem]) -> &HttpRequest {
        match &items[0] {
            ImportItem::Api { request, .. } => request,
            ImportItem::Folder { children, .. } => first_api(children),
        }
    }

    #[test]
    fn swagger_paths_become_apis_by_tag() {
        let json: Value = serde_json::from_str(
            r##"{
            "swagger": "2.0", "info": {"title": "Pet Store"}, "host": "pets.test", "basePath": "/v2", "schemes": ["https"],
            "paths": {"/pet": {"post": {"tags": ["pet"], "summary": "Add a pet",
                "parameters": [{"in": "body", "name": "body", "schema": {"$ref": "#/definitions/Pet"}},
                               {"in": "query", "name": "dry", "type": "boolean", "default": false}]},
                "get": {"operationId": "listPets", "parameters": [{"in": "header", "name": "X-Key", "type": "string"}]}}},
            "definitions": {
                "Pet": {"type": "object", "properties": {"name": {"type": "string", "example": "rex"},
                        "tags": {"type": "array", "items": {"$ref": "#/definitions/Tag"}}, "age": {"type": "integer"}}},
                "Tag": {"type": "object", "properties": {"id": {"type": "integer"}}}
            }
        }"##,
        )
        .expect("JSON");
        let bundle = parse(&json);
        assert_eq!(
            bundle.environment,
            [("PetStore".to_string(), "https://pets.test/v2".to_string())]
        );
        let ImportItem::Folder { name, children } = &bundle.items[0] else {
            panic!("a folder per tag");
        };
        assert_eq!(name, "pet");
        let ImportItem::Api { name, request } = &children[0] else {
            panic!("an API");
        };
        assert_eq!(name, "Add a pet");
        assert_eq!(
            (request.method, request.uri.as_str()),
            (HttpMethod::Post, "{{PetStore}}/pet")
        );
        assert_eq!(request.content_type, ContentType::Json);
        let body: Value = serde_json::from_str(&request.body).expect("JSON");
        assert_eq!(body, json!({"name": "rex", "tags": [{"id": 0}], "age": 0}));
        assert_eq!(request.query, [kv("dry", "false", true)]);
        let ImportItem::Api { name, request } = &bundle.items[1] else {
            panic!("the untagged one after");
        };
        assert_eq!(
            (name.as_str(), request.headers.clone()),
            ("listPets", vec![kv("X-Key", "", true)])
        );
    }

    #[test]
    fn openapi_3_bodies_and_servers() {
        let json: Value = serde_json::from_str(
            r##"{
            "openapi": "3.0.1", "info": {"title": "Orders"}, "servers": [{"url": "https://orders.test/api/"}],
            "paths": {"/orders/{id}": {"put": {"summary": "Update",
                "requestBody": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Order"}}}}}}},
            "components": {"schemas": {"Order": {"type": "object", "properties": {"state": {"type": "string", "enum": ["open", "shut"]}}}}}
        }"##,
        )
        .expect("JSON");
        let bundle = parse(&json);
        assert_eq!(
            bundle.environment,
            [("Orders".to_string(), "https://orders.test/api".to_string())]
        );
        let request = first_api(&bundle.items);
        assert_eq!(request.uri, "{{Orders}}/orders/{id}");
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).expect("JSON"),
            json!({"state": "open"})
        );
    }

    #[test]
    fn loops_in_schemas_end() {
        let json: Value = serde_json::from_str(
            r##"{"swagger": "2.0", "paths": {"/n": {"post": {"parameters": [{"in": "body", "schema": {"$ref": "#/definitions/Node"}}]}}},
                "definitions": {"Node": {"type": "object", "properties": {"next": {"$ref": "#/definitions/Node"}}}}}"##,
        )
        .expect("JSON");
        let request = first_api(&parse(&json).items).clone();
        assert!(request.body.contains("next"));
    }
}
