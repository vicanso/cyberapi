//! A Postman collection (v2.0 / v2.1) as folders and APIs (F10.3).
//!
//! An item with a `request` is an API, one without is a folder; the
//! collection's `variable`s become environment variables. Unlike before,
//! headers, Basic auth and form bodies come along too (#28). Postman's
//! `{{name}}` is this app's prefix syntax already.

use super::{ImportBundle, ImportItem, api, content_type_of, guess_content_type, kv};
use crate::graphql::GraphQlBody;
use crate::model::{ContentType, KvParam};
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
        });
    };
    // A request may be just its URL.
    if let Some(url) = request.as_str() {
        let (name, request) = api(name, "GET", url);
        return Some(ImportItem::Api { name, request });
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
    if let Some(auth) = request.get("auth")
        && text(auth, "type") == "basic"
    {
        let basic = pairs(auth.get("basic"), "key");
        let find = |key: &str| {
            basic
                .iter()
                .find(|pair| pair.key == key)
                .map(|pair| pair.value.clone())
                .unwrap_or_default()
        };
        built.auth = vec![kv(&find("username"), &find("password"), true)];
    }
    Some(ImportItem::Api { name, request: built })
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
        let ImportItem::Folder { name, children } = &bundle.items[0] else {
            panic!("a folder");
        };
        assert_eq!(name, "users");
        assert_eq!(
            children[0],
            ImportItem::Api {
                name: "login".into(),
                request: HttpRequest {
                    method: HttpMethod::Post,
                    uri: "{{host}}/login".into(),
                    content_type: ContentType::Json,
                    body: "{\"a\":1}".into(),
                    query: vec![kv("debug", "1", false)],
                    headers: vec![kv("X-Off", "1", false)],
                    auth: vec![kv("u", "p", true)],
                    ..Default::default()
                },
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
}
