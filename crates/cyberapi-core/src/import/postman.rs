//! A Postman collection (v2.0 / v2.1) as folders and APIs (F10.3).
//!
//! An item with a `request` is an API, one without is a folder; the
//! collection's `variable`s become environment variables. Unlike before,
//! headers, Basic auth and form bodies come along too (#28). Postman's
//! `{{name}}` is this app's prefix syntax already.
//!
//! B4: a request's, a folder's and the collection's `auth` — Basic,
//! Bearer, API key, Digest, none — come in as this app's (#66); a kind it
//! has no counterpart of (OAuth, AWS, Hawk…) inherits. A Postman
//! environment file is a named environment.

use super::{ImportBundle, ImportEnvironment, ImportItem, api, content_type_of, guess_content_type, kv};
use crate::auth::{Auth, KeyPlace};
use crate::graphql::GraphQlBody;
use crate::model::FolderSettings;
use crate::model::{ContentType, Example, KvParam};
use serde_json::Value;

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// `[{key, value, disabled}]`.
fn pairs(value: Option<&Value>, key: &str) -> Vec<KvParam> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    kv(
                        text(item, key),
                        text(item, "value"),
                        !item.get("disabled").and_then(Value::as_bool).unwrap_or(false),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A value as text: a variable's may be a number or a flag.
fn as_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// B4: an `auth` object as this app's; `None` for a kind it has no
/// counterpart of, which then inherits. v2.1 lists the fields
/// (`[{key, value}]`), v2.0 names them (`{username: …}`).
fn auth(value: &Value) -> Option<Auth> {
    let kind = text(value, "type");
    let fields = value.get(kind);
    let field = |key: &str| match fields {
        Some(Value::Array(items)) => as_text(
            items
                .iter()
                .find(|item| text(item, "key") == key)
                .and_then(|item| item.get("value")),
        ),
        Some(object) => as_text(object.get(key)),
        None => String::new(),
    };
    Some(match kind {
        "inherit" => Auth::Inherit,
        "noauth" => Auth::None,
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
            place: if field("in") == "query" {
                KeyPlace::Query
            } else {
                KeyPlace::Header
            },
        },
        _ => return None,
    })
}

fn item(value: &Value) -> Option<ImportItem> {
    let name = text(value, "name");
    let Some(request) = value.get("request") else {
        let children = value
            .get("item")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(item).collect())
            .unwrap_or_default();
        return Some(ImportItem::Folder {
            name: name.to_string(),
            children,
            settings: FolderSettings {
                auth: value.get("auth").and_then(auth).unwrap_or_default(),
                headers: Vec::new(),
                description: description(value.get("description")),
            },
        });
    };
    // A request may be just its URL.
    if let Some(url) = request.as_str() {
        let (name, request) = api(name, "GET", url);
        return Some(ImportItem::Api {
            name,
            request: Box::new(request),
        });
    }
    let url = match request.get("url") {
        Some(Value::String(url)) => url.clone(),
        Some(url) => text(url, "raw").to_string(),
        None => String::new(),
    };
    let (name, mut built) = api(name, text(request, "method"), &url);
    // The query list, where the URL object has one, says which are off.
    let listed = pairs(request.get("url").and_then(|url| url.get("query")), "key");
    if !listed.is_empty() {
        built.query = listed;
    }
    built.headers = pairs(request.get("header"), "key");
    if let Some(at) = built
        .headers
        .iter()
        .position(|header| header.key.eq_ignore_ascii_case("content-type") && content_type_of(&header.value).is_some())
    {
        let header = built.headers.remove(at);
        built.content_type = content_type_of(&header.value).unwrap_or_default();
    }
    if let Some(body) = request.get("body") {
        match text(body, "mode") {
            "raw" => {
                built.body = text(body, "raw").to_string();
                let language = body
                    .get("options")
                    .and_then(|options| options.get("raw"))
                    .map(|raw| text(raw, "language"))
                    .unwrap_or_default();
                built.content_type = match language {
                    "json" => ContentType::Json,
                    "xml" => ContentType::Xml,
                    "text" => ContentType::Plain,
                    _ => guess_content_type(&built.body),
                };
            }
            // Its own mode: the query and the variables as text.
            "graphql" => {
                let graphql = body.get("graphql").unwrap_or(&Value::Null);
                built.content_type = ContentType::GraphQl;
                built.body = GraphQlBody {
                    query: text(graphql, "query").to_string(),
                    variables: text(graphql, "variables").to_string(),
                    operation_name: String::new(),
                }
                .to_body();
            }
            "urlencoded" => {
                built.content_type = ContentType::Form;
                built.body = serde_json::to_string(&pairs(body.get("urlencoded"), "key")).unwrap_or_default();
            }
            "formdata" => {
                built.content_type = ContentType::Multipart;
                let fields: Vec<KvParam> = body
                    .get("formdata")
                    .and_then(Value::as_array)
                    .map(|fields| {
                        fields
                            .iter()
                            .map(|field| {
                                let value = if text(field, "type") == "file" {
                                    format!("file://{}", text(field, "src"))
                                } else {
                                    text(field, "value").to_string()
                                };
                                kv(
                                    text(field, "key"),
                                    &value,
                                    !field.get("disabled").and_then(Value::as_bool).unwrap_or(false),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                built.body = serde_json::to_string(&fields).unwrap_or_default();
            }
            _ => {}
        }
    }
    if let Some(own) = request.get("auth").and_then(auth) {
        built.auth = own;
    }
    // F3: what it is for, and its saved responses as examples.
    built.description = description(request.get("description"));
    built.examples = value
        .get("response")
        .and_then(Value::as_array)
        .map(|responses| responses.iter().filter_map(example).collect())
        .unwrap_or_default();
    Some(ImportItem::Api {
        name,
        request: Box::new(built),
    })
}

/// F3: a description: text, or `{content}`.
fn description(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(object @ Value::Object(_)) => text(object, "content").trim().to_string(),
        _ => String::new(),
    }
}

/// F3: a saved response as an example; one with no status code is none.
fn example(response: &Value) -> Option<Example> {
    let status = response.get("code").and_then(Value::as_u64)?;
    Some(Example {
        name: text(response, "name").trim().to_string(),
        status: u16::try_from(status).ok()?,
        headers: response
            .get("header")
            .and_then(Value::as_array)
            .map(|headers| {
                headers
                    .iter()
                    .map(|header| (text(header, "key").to_string(), text(header, "value").to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        body: text(response, "body").to_string(),
    })
}

pub(super) fn parse(json: &Value) -> ImportBundle {
    ImportBundle {
        items: json
            .get("item")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(item).collect())
            .unwrap_or_default(),
        environment: json
            .get("variable")
            .and_then(Value::as_array)
            .map(|variables| {
                variables
                    .iter()
                    .map(|variable| (text(variable, "key").to_string(), text(variable, "value").to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        auth: json.get("auth").and_then(auth).unwrap_or_default(),
        ..Default::default()
    }
}

/// B4: a Postman environment file as a named environment: its enabled
/// values.
pub(super) fn parse_environment(json: &Value) -> ImportBundle {
    let name = text(json, "name").trim();
    let variables = json
        .get("values")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter(|value| value.get("enabled").and_then(Value::as_bool).unwrap_or(true))
                .map(|value| (text(value, "key").to_string(), as_text(value.get("value"))))
                .filter(|(key, _)| !key.trim().is_empty())
                .collect()
        })
        .unwrap_or_default();
    ImportBundle {
        environments: vec![ImportEnvironment {
            name: if name.is_empty() {
                "Postman".into()
            } else {
                name.to_string()
            },
            variables,
        }],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HttpMethod, HttpRequest};
    use pretty_assertions::assert_eq;

    #[test]
    fn folders_requests_and_variables() {
        let json: Value = serde_json::from_str(
            r#"{
            "info": {"name": "Shop", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
            "item": [
                {"name": "users", "item": [
                    {"name": "login", "request": {
                        "method": "POST",
                        "header": [{"key": "Content-Type", "value": "application/json"}, {"key": "X-Off", "value": "1", "disabled": true}],
                        "url": {"raw": "{{host}}/login?debug=1", "query": [{"key": "debug", "value": "1", "disabled": true}]},
                        "body": {"mode": "raw", "raw": "{\"a\":1}", "options": {"raw": {"language": "json"}}},
                        "auth": {"type": "basic", "basic": [{"key": "password", "value": "p"}, {"key": "username", "value": "u"}]}
                    }}
                ]},
                {"name": "form", "request": {"method": "PUT", "url": "https://a.test/f",
                    "body": {"mode": "urlencoded", "urlencoded": [{"key": "k", "value": "v"}]}}},
                {"name": "me", "request": {"method": "POST", "url": "{{host}}/graphql",
                    "body": {"mode": "graphql", "graphql": {"query": "query { me { id } }", "variables": "{\"a\": 1}"}}}},
                {"name": "bare", "request": "https://a.test/bare"}
            ],
            "variable": [{"key": "host", "value": "https://shop.test"}]
        }"#,
        )
        .expect("JSON");
        let bundle = parse(&json);
        assert_eq!(
            bundle.environment,
            [("host".to_string(), "https://shop.test".to_string())]
        );
        let ImportItem::Folder { name, children, .. } = &bundle.items[0] else {
            panic!("a folder");
        };
        assert_eq!(name, "users");
        assert_eq!(
            children[0],
            ImportItem::Api {
                name: "login".into(),
                request: Box::new(HttpRequest {
                    method: HttpMethod::Post,
                    uri: "{{host}}/login".into(),
                    content_type: ContentType::Json,
                    body: "{\"a\":1}".into(),
                    query: vec![kv("debug", "1", false)],
                    headers: vec![kv("X-Off", "1", false)],
                    auth: Auth::Basic {
                        username: "u".into(),
                        password: "p".into()
                    },
                    ..Default::default()
                }),
            }
        );
        let ImportItem::Api { request, .. } = &bundle.items[1] else {
            panic!("an API");
        };
        assert_eq!(request.content_type, ContentType::Form);
        assert_eq!(request.body, r#"[{"key":"k","value":"v","enabled":true}]"#);
        let ImportItem::Api { request, .. } = &bundle.items[2] else {
            panic!("an API");
        };
        assert_eq!(request.content_type, ContentType::GraphQl);
        let graphql = GraphQlBody::read(&request.body);
        assert_eq!(
            (graphql.query.as_str(), graphql.variables.as_str()),
            ("query { me { id } }", "{\"a\": 1}")
        );
        let ImportItem::Api { request, .. } = &bundle.items[3] else {
            panic!("an API");
        };
        assert_eq!(
            (request.method, request.uri.as_str()),
            (HttpMethod::Get, "https://a.test/bare")
        );
    }

    /// B4: the collection's, a folder's and a request's auth, in v2.1's
    /// list and v2.0's named form; a kind with no counterpart inherits.
    #[test]
    fn auth_comes_in_at_every_level() {
        let json: Value = serde_json::from_str(
            r#"{"info": {"name": "Shop"},
                "auth": {"type": "bearer", "bearer": [{"key": "token", "value": "{{token}}", "type": "string"}]},
                "item": [
                    {"name": "admin", "auth": {"type": "apikey", "apikey": [
                        {"key": "key", "value": "X-Key"}, {"key": "value", "value": "k"}, {"key": "in", "value": "query"}]},
                     "item": [{"name": "stats", "request": {"method": "GET", "url": "{{host}}/stats",
                        "auth": {"type": "noauth"}}}]},
                    {"name": "legacy", "request": {"method": "GET", "url": "{{host}}/old",
                        "auth": {"type": "digest", "digest": {"username": "u", "password": "p"}}}},
                    {"name": "oauth", "request": {"method": "GET", "url": "{{host}}/o",
                        "auth": {"type": "oauth2", "oauth2": []}}}
                ]}"#,
        )
        .expect("JSON");
        let bundle = parse(&json);
        assert_eq!(
            bundle.auth,
            Auth::Bearer {
                token: "{{token}}".into()
            }
        );
        let ImportItem::Folder { settings, children, .. } = &bundle.items[0] else {
            panic!("a folder");
        };
        assert_eq!(
            settings.auth,
            Auth::ApiKey {
                name: "X-Key".into(),
                value: "k".into(),
                place: KeyPlace::Query
            }
        );
        let request_auth = |item: &ImportItem| match item {
            ImportItem::Api { request, .. } => request.auth.clone(),
            ImportItem::Folder { .. } => panic!("an API"),
        };
        assert_eq!(request_auth(&children[0]), Auth::None);
        assert_eq!(
            request_auth(&bundle.items[1]),
            Auth::Digest {
                username: "u".into(),
                password: "p".into()
            }
        );
        assert_eq!(
            request_auth(&bundle.items[2]),
            Auth::Inherit,
            "OAuth has no counterpart yet"
        );
    }

    /// B4: a Postman environment file is a named environment of its
    /// enabled values.
    #[test]
    fn an_environment_file_is_a_named_environment() {
        let text = r#"{"id": "e1", "name": "staging", "_postman_variable_scope": "environment",
            "values": [{"key": "host", "value": "https://staging.test", "enabled": true},
                       {"key": "port", "value": 8443, "enabled": true},
                       {"key": "old", "value": "x", "enabled": false}]}"#;
        assert_eq!(
            super::super::detect(text).map(|(format, _)| format),
            Ok(super::super::Format::PostmanEnvironment)
        );
        let bundle = super::super::parse(text).expect("a bundle");
        assert_eq!(
            bundle.environments,
            [ImportEnvironment {
                name: "staging".into(),
                variables: vec![
                    ("host".into(), "https://staging.test".into()),
                    ("port".into(), "8443".into())
                ],
            }]
        );
        assert!(bundle.items.is_empty() && bundle.name.is_none());
    }
}
