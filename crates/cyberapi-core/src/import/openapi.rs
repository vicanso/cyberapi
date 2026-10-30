//! Swagger 2.0 and OpenAPI 3 as folders and APIs (F10.3).
//!
//! As before for Swagger: every path and method is an API named by its
//! `summary` (or `operationId`), in a folder per first tag; its URI starts
//! with an environment variable named after the title, whose value is the
//! server; query parameters take their examples; a JSON body is an example
//! made from its schema. OpenAPI 3 is read the same way (#28), with
//! `servers` and `requestBody`. `$ref`s are followed — not just one level —
//! up to a depth that stops loops.
//!
//! B4: the scheme the document's `security` names — OpenAPI 3's
//! `components.securitySchemes`, Swagger's `securityDefinitions` — is how
//! the project signs in: HTTP Basic, Bearer or Digest, or an API key in a
//! header or the query, its values left for the reader to fill in. An
//! operation that asks for another, or for none (`security: []`), signs in
//! so itself. OAuth and OpenID Connect have no counterpart here yet.
//!
//! A6: the schema of an operation's first 2xx JSON response is an enabled
//! assertion that the answer conforms to it, marked as the import's: made
//! to stand alone (its `$ref`s written in) and read as JSON Schema
//! (OpenAPI 3.0's `nullable` and boolean exclusive bounds put its way).
//! One too big to keep with every API is left out.

use super::{ImportBundle, ImportItem, kv};
use crate::auth::{Auth, KeyPlace};
use crate::model::{ContentType, HttpMethod, HttpRequest, KvParam};
use crate::oauth::{Grant, OAuth2};
use serde_json::{Map, Value, json};

const METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "options", "head"];
const MAX_DEPTH: usize = 8;
/// A6: the most parts of a response's schema written out, `$ref`s in; one
/// with more is no assertion.
const SCHEMA_PARTS: usize = 4_000;

/// A6: the schema of `op`'s first 2xx response with JSON in it: OpenAPI 3's
/// `content`, or Swagger's own `schema`.
fn response_schema<'a>(root: &'a Value, op: &'a Value) -> Option<&'a Value> {
    let (_, response) = op
        .get("responses")?
        .as_object()?
        .iter()
        .find(|(status, _)| status.starts_with('2'))?;
    let response = resolve(root, response);
    if let Some(content) = response.get("content").and_then(Value::as_object) {
        return content
            .iter()
            .find(|(kind, _)| kind.contains("json"))
            .and_then(|(_, media)| media.get("schema"));
    }
    response.get("schema")
}

/// A6: `schema` standing alone as JSON Schema: its `$ref`s written in, to
/// [`MAX_DEPTH`] (past it, anything goes), and OpenAPI's ways put JSON
/// Schema's. `None` past [`SCHEMA_PARTS`].
fn standalone(root: &Value, schema: &Value, depth: usize, parts: &mut usize) -> Option<Value> {
    *parts += 1;
    if *parts > SCHEMA_PARTS {
        return None;
    }
    let Value::Object(fields) = schema else {
        return Some(schema.clone());
    };
    // OpenAPI 3.0 and Swagger: a null allowed besides what it says — beside
    // a `$ref` or an `allOf` too, where 3.0 specs often put it (BUG-12).
    let nullable = ["nullable", "x-nullable"]
        .iter()
        .any(|flag| fields.get(*flag) == Some(&Value::Bool(true)));
    let out = if fields.contains_key("$ref") {
        let target = resolve(root, schema);
        // Too deep, or a `$ref` this document does not hold: anything goes.
        if depth >= MAX_DEPTH || std::ptr::eq(target, schema) {
            json!({})
        } else {
            standalone(root, target, depth + 1, parts)?
        }
    } else {
        let mut out = Map::new();
        for (key, value) in fields {
            let value = match key.as_str() {
                "nullable" | "x-nullable" => continue,
                // What is data, not a schema.
                "example" | "examples" | "enum" | "default" | "const" => value.clone(),
                // Names, each to a schema: a property may be called
                // `nullable` or `example` (BUG-12).
                "properties" | "patternProperties" | "definitions" | "$defs" | "dependentSchemas" => match value {
                    Value::Object(named) => Value::Object(
                        named
                            .iter()
                            .map(|(name, schema)| Some((name.clone(), standalone(root, schema, depth, parts)?)))
                            .collect::<Option<Map<_, _>>>()?,
                    ),
                    other => other.clone(),
                },
                _ => match value {
                    Value::Object(_) => standalone(root, value, depth, parts)?,
                    Value::Array(items) => Value::Array(
                        items
                            .iter()
                            .map(|item| standalone(root, item, depth, parts))
                            .collect::<Option<Vec<_>>>()?,
                    ),
                    other => other.clone(),
                },
            };
            out.insert(key.clone(), value);
        }
        // OpenAPI 3.0: an exclusive bound is a flag on the bound.
        for (flag, bound) in [("exclusiveMinimum", "minimum"), ("exclusiveMaximum", "maximum")] {
            if let Some(Value::Bool(exclusive)) = out.get(flag).cloned() {
                out.remove(flag);
                if exclusive && let Some(value) = out.remove(bound) {
                    out.insert(flag.into(), value);
                }
            }
        }
        Value::Object(out)
    };
    Some(if nullable { or_null(out) } else { out })
}

/// `schema`, with `null` allowed besides: added to its type, or to its
/// `enum`, or else one of the two.
fn or_null(schema: Value) -> Value {
    let Value::Object(mut fields) = schema else {
        return schema;
    };
    if fields.is_empty() {
        return Value::Object(fields);
    }
    if let Some(Value::Array(values)) = fields.get_mut("enum")
        && !values.contains(&Value::Null)
    {
        values.push(Value::Null);
    }
    match fields.get("type").cloned() {
        Some(Value::String(kind)) => {
            fields.insert("type".into(), json!([kind, "null"]));
            Value::Object(fields)
        }
        Some(Value::Array(mut kinds)) => {
            if !kinds.contains(&json!("null")) {
                kinds.push(json!("null"));
            }
            fields.insert("type".into(), Value::Array(kinds));
            Value::Object(fields)
        }
        _ => json!({ "anyOf": [Value::Object(fields), { "type": "null" }] }),
    }
}

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

/// B4: the auth a `security` list asks for, its values empty: the first
/// scheme of its first requirement this app has a counterpart of. `[]` is
/// none; `None` when it names nothing this app knows.
fn security(json: &Value, requirements: &Value) -> Option<Auth> {
    let requirements = requirements.as_array()?;
    if requirements.is_empty() {
        return Some(Auth::None);
    }
    let schemes = json
        .pointer("/components/securitySchemes")
        .or_else(|| json.get("securityDefinitions"))?;
    requirements
        .iter()
        .filter_map(Value::as_object)
        .flat_map(|requirement| requirement.iter())
        .filter_map(|(name, scopes)| schemes.get(name).map(|scheme| (resolve(json, scheme), scopes)))
        .find_map(|(scheme, scopes)| {
            let place = |scheme: &Value| match text(scheme, "in") {
                "header" => Some(KeyPlace::Header),
                "query" => Some(KeyPlace::Query),
                _ => None,
            };
            match (
                text(scheme, "type"),
                text(scheme, "scheme").to_ascii_lowercase().as_str(),
            ) {
                ("http", "bearer") => Some(Auth::Bearer { token: String::new() }),
                ("http", "basic") | ("basic", _) => Some(Auth::Basic {
                    username: String::new(),
                    password: String::new(),
                }),
                ("http", "digest") => Some(Auth::Digest {
                    username: String::new(),
                    password: String::new(),
                }),
                ("apiKey", _) => Some(Auth::ApiKey {
                    name: text(scheme, "name").to_string(),
                    value: String::new(),
                    place: place(scheme)?,
                }),
                ("oauth2", _) => oauth(scheme, scopes),
                _ => None,
            }
        })
}

/// C4: an OAuth 2.0 scheme's flow asked without a browser — client
/// credentials, else password — with the scopes the requirement names.
/// Swagger 2 names its one flow `application` or `password`.
fn oauth(scheme: &Value, scopes: &Value) -> Option<Auth> {
    let flows = scheme.get("flows");
    let (grant, flow) = match flows {
        Some(flows) => [
            ("clientCredentials", Grant::ClientCredentials),
            ("password", Grant::Password),
            ("authorizationCode", Grant::AuthorizationCode),
        ]
        .into_iter()
        .find_map(|(name, grant)| Some((grant, flows.get(name)?)))?,
        None => match text(scheme, "flow") {
            "application" => (Grant::ClientCredentials, scheme),
            "password" => (Grant::Password, scheme),
            "accessCode" => (Grant::AuthorizationCode, scheme),
            _ => return None,
        },
    };
    let scope: Vec<&str> = scopes
        .as_array()
        .map(|scopes| scopes.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    Some(Auth::from(OAuth2 {
        grant,
        token_url: text(flow, "tokenUrl").to_string(),
        authorization_url: text(flow, "authorizationUrl").to_string(),
        scope: scope.join(" "),
        ..Default::default()
    }))
}

/// F9: whether a multipart schema property is a binary file upload (OpenAPI
/// 3.0's `format: binary`, 3.1's `contentMediaType`, or an array of files).
fn is_binary_property(root: &Value, property: &Value) -> bool {
    let property = resolve(root, property);
    text(property, "format") == "binary"
        || property.get("contentMediaType").is_some()
        || (text(property, "type") == "array"
            && property
                .get("items")
                .is_some_and(|items| is_binary_property(root, items)))
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
        // F3.
        description: text(op, "description").trim().to_string(),
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
        let desc = text(parameter, "description").trim();
        match text(parameter, "in") {
            "query" => request
                .query
                .push(kv(name, &parameter_value(json, parameter), true).with_description(desc)),
            "header" => request
                .headers
                .push(kv(name, &parameter_value(json, parameter), true).with_description(desc)),
            "formData" => {
                let value = if text(parameter, "type") == "file" {
                    "file://".to_string()
                } else {
                    parameter_value(json, parameter)
                };
                form.push(kv(name, &value, true).with_description(desc));
            }
            // Swagger 2's body.
            "body" => {
                if let Some(schema) = parameter.get("schema") {
                    request.content_type = ContentType::Json;
                    request.body = crate::format::json(&example(json, schema, 0));
                    // F11: keep its JSON Schema standalone for Docs.
                    request.request_schema =
                        standalone(json, schema, 0, &mut 0).map(|schema| crate::format::json(&schema));
                }
            }
            _ => {}
        }
    }
    if !form.is_empty() {
        let has_file = form.iter().any(|field| field.value.starts_with("file://"));
        let multipart = has_file
            || op
                .get("consumes")
                .or_else(|| json.get("consumes"))
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
            // F11: keep its JSON Schema standalone for Docs.
            request.request_schema = media
                .get("schema")
                .and_then(|schema| standalone(json, schema, 0, &mut 0))
                .map(|schema| crate::format::json(&schema));
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
                            let resolved = resolve(json, property);
                            let desc = text(resolved, "description").trim();
                            let value = if kind.contains("multipart") && is_binary_property(json, property) {
                                "file://".to_string()
                            } else {
                                match example(json, property, 1) {
                                    Value::String(text) => text,
                                    other => other.to_string(),
                                }
                            };
                            kv(name, &value, true).with_description(desc)
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
    // A6: what its answer is to look like.
    if let Some(schema) = response_schema(json, op).and_then(|schema| standalone(json, schema, 0, &mut 0)) {
        request.assertions = vec![crate::assertions::Assertion::imported_schema(&schema)];
    }
    // B4: an operation's own security, where it is not the document's.
    if let Some(own) = op.get("security").and_then(|requirements| security(json, requirements))
        && json
            .get("security")
            .and_then(|requirements| security(json, requirements))
            != Some(own.clone())
    {
        request.auth = own;
    }
    let tag = op
        .get("tags")
        .and_then(Value::as_array)
        .and_then(|tags| tags.first())
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    (
        tag,
        ImportItem::Api {
            name,
            request: Box::new(request),
        },
    )
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
        .map(|(name, children)| ImportItem::Folder {
            name,
            children,
            settings: Default::default(),
        })
        .chain(untagged)
        .collect();
    ImportBundle {
        items,
        environment: server.into_iter().collect(),
        auth: json
            .get("security")
            .and_then(|requirements| security(json, requirements))
            .unwrap_or_default(),
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
        let ImportItem::Folder { name, children, .. } = &bundle.items[0] else {
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

    /// A6: the first 2xx JSON response's schema, standing alone, checks
    /// the answer; OpenAPI 3.0's ways read as JSON Schema's.
    #[test]
    fn a_responses_schema_becomes_an_assertion() {
        use crate::assertions::{Subject, check};
        let json: Value = serde_json::from_str(
            r##"{"openapi": "3.0.1", "paths": {
                    "/orders/{id}": {"get": {"summary": "order", "responses": {
                        "404": {"description": "none"},
                        "200": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Order"}}}}}}},
                    "/ping": {"get": {"summary": "ping", "responses": {"204": {"description": "nothing"}}}}},
                "components": {"schemas": {
                    "Order": {"type": "object", "required": ["id"], "additionalProperties": false, "properties": {
                        "id": {"type": "integer", "minimum": 0, "exclusiveMinimum": true},
                        "note": {"type": "string", "nullable": true},
                        "next": {"$ref": "#/components/schemas/Order"},
                        "owner": {"nullable": true, "allOf": [{"$ref": "#/components/schemas/Owner"}]},
                        "status": {"$ref": "#/components/schemas/Status", "nullable": true},
                        "nullable": {"type": "boolean"},
                        "example": {"$ref": "#/components/schemas/Owner"}}},
                    "Owner": {"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}}},
                    "Status": {"type": "string", "enum": ["open", "shut"]}}}}"##,
        )
        .expect("JSON");
        let bundle = parse(&json);
        let order = first_api(&bundle.items);
        let [assertion] = &order.assertions[..] else {
            panic!("one assertion: {:?}", order.assertions);
        };
        assert!(assertion.imported && assertion.enabled && assertion.subject == Subject::Schema);
        assert!(!assertion.expected.contains("$ref"), "{}", assertion.expected);
        let answer = |body: &str| crate::response::HttpResponse {
            status: Some(200),
            body: body.as_bytes().to_vec(),
            ..Default::default()
        };
        let holds = |body: &str| {
            let response = answer(body);
            check(assertion, &response, response.body_json().as_ref())
                .expect("checked")
                .passed
        };
        assert!(holds(r#"{"id": 1, "note": null, "next": {"id": 2}}"#));
        assert!(!holds(r#"{"id": 0}"#), "the bound is exclusive");
        assert!(!holds(r#"{"note": "x"}"#), "id is required");
        // BUG-12: `nullable` beside an `allOf` or a `$ref` holds; properties
        // called `nullable` and `example` are properties.
        assert!(holds(r#"{"id": 1, "owner": null, "status": null}"#));
        assert!(holds(r#"{"id": 1, "owner": {"name": "a"}, "status": "open"}"#));
        assert!(!holds(r#"{"id": 1, "owner": {}}"#), "the owner's name is required");
        assert!(!holds(r#"{"id": 1, "status": "lost"}"#));
        assert!(holds(r#"{"id": 1, "nullable": true, "example": {"name": "a"}}"#));
        assert!(!holds(r#"{"id": 1, "example": {}}"#), "its $ref written in");
        let ImportItem::Api { request: ping, .. } = &bundle.items[1] else {
            panic!("an API");
        };
        assert!(ping.assertions.is_empty(), "no JSON answered");

        // Swagger's own `schema`.
        let swagger: Value = serde_json::from_str(
            r##"{"swagger": "2.0", "paths": {"/n": {"get": {"responses": {"200": {"schema": {"type": "array", "items": {"type": "string"}}}}}}}}"##,
        )
        .expect("JSON");
        let listed = first_api(&parse(&swagger).items).clone();
        assert_eq!(listed.assertions.len(), 1);
    }

    /// B4: the document's security is the project's auth, values empty; an
    /// operation's own, or none, is its API's.
    #[test]
    fn security_schemes_become_auth() {
        let json: Value = serde_json::from_str(
            r#"{"openapi": "3.0.1", "info": {"title": "Shop"},
                "components": {"securitySchemes": {
                    "bearer": {"type": "http", "scheme": "bearer", "bearerFormat": "JWT"},
                    "key": {"type": "apiKey", "in": "header", "name": "X-API-Key"},
                    "oauth": {"type": "oauth2", "flows": {}}}},
                "security": [{"oauth": []}, {"bearer": []}],
                "paths": {
                    "/me": {"get": {"summary": "me"}},
                    "/health": {"get": {"summary": "health", "security": []}},
                    "/admin": {"get": {"summary": "admin", "security": [{"key": []}]}}
                }}"#,
        )
        .expect("JSON");
        let bundle = parse(&json);
        assert_eq!(bundle.auth, Auth::Bearer { token: String::new() }, "OAuth passed over");
        let auth = |name: &str| {
            bundle
                .items
                .iter()
                .find_map(|item| match item {
                    ImportItem::Api { name: own, request } if own == name => Some(request.auth.clone()),
                    _ => None,
                })
                .expect("there")
        };
        assert_eq!(auth("me"), Auth::Inherit);
        assert_eq!(auth("health"), Auth::None);
        assert_eq!(
            auth("admin"),
            Auth::ApiKey {
                name: "X-API-Key".into(),
                value: String::new(),
                place: KeyPlace::Header
            }
        );
        // Swagger 2's definitions.
        let swagger: Value = serde_json::from_str(
            r#"{"swagger": "2.0", "info": {"title": "Old"}, "securityDefinitions": {"b": {"type": "basic"}},
                "security": [{"b": []}], "paths": {}}"#,
        )
        .expect("JSON");
        assert_eq!(
            parse(&swagger).auth,
            Auth::Basic {
                username: String::new(),
                password: String::new()
            }
        );
    }

    #[test]
    fn multipart_file_fields_import_as_file_placeholders() {
        let swagger: Value = serde_json::from_str(
            r#"{"swagger": "2.0", "paths": {"/upload": {"post": {
                "parameters": [
                    {"in": "formData", "name": "avatar", "type": "file"},
                    {"in": "formData", "name": "note", "type": "string", "default": "hi"}
                ]
            }}}}"#,
        )
        .expect("JSON");
        let sw_req = first_api(&parse(&swagger).items).clone();
        assert_eq!(sw_req.content_type, ContentType::Multipart);
        let sw_fields: Vec<KvParam> = serde_json::from_str(&sw_req.body).expect("fields");
        assert_eq!(sw_fields, [kv("avatar", "file://", true), kv("note", "hi", true)]);

        let oas3: Value = serde_json::from_str(
            r#"{"openapi": "3.0.3", "paths": {"/upload": {"post": {
                "requestBody": {"content": {"multipart/form-data": {"schema": {"type": "object", "properties": {
                    "doc": {"type": "string", "format": "binary"},
                    "title": {"type": "string", "example": "report"}
                }}}}}
            }}}}"#,
        )
        .expect("JSON");
        let oas_req = first_api(&parse(&oas3).items).clone();
        assert_eq!(oas_req.content_type, ContentType::Multipart);
        let oas_fields: Vec<KvParam> = serde_json::from_str(&oas_req.body).expect("fields");
        assert_eq!(oas_fields, [kv("doc", "file://", true), kv("title", "report", true)]);
    }

    #[test]
    fn a_request_bodys_schema_is_kept_standalone() {
        let oas3: Value = serde_json::from_str(
            r##"{"openapi": "3.0.3",
                "components": {"schemas": {"NewUser": {
                    "type": "object",
                    "required": ["email"],
                    "properties": {"email": {"type": "string", "description": "Sign-in email"}}
                }}},
                "paths": {"/users": {"post": {
                    "requestBody": {"content": {"application/json": {
                        "schema": {"$ref": "#/components/schemas/NewUser"}
                    }}}
                }}}}"##,
        )
        .expect("JSON");
        let bundle = parse(&oas3);
        let req = first_api(&bundle.items);
        let schema = req.request_schema.as_deref().expect("request schema");
        let fields = crate::schema_docs::fields_of_schema(schema).expect("fields");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "email");
        assert!(fields[0].required);
        assert_eq!(fields[0].description, "Sign-in email");
    }
}
