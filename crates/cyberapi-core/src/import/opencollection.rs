//! B5: an OpenCollection document — what Bruno exports a collection as,
//! and writes each request of a YAML collection as — as folders and APIs
//! (F10.3).
//!
//! `items` nest: a `folder` has its own, an `http` or `graphql` item is an
//! API (gRPC and WebSocket ones are left out). A request's `params` of type
//! `path` fill the URL's `:name`s; a body with variants sends the selected
//! one. Auth — Basic, Bearer, API key, Digest, none, inherit — comes in as
//! this app's (#66) on requests and folders, the collection's as the
//! project's; a kind this app has no counterpart of inherits. The
//! collection's environments are named environments, its variables the
//! ones every environment shares; headers it sends keep a folder, the
//! project having none of its own. `{{process.env.NAME}}` is this app's
//! `{{env(NAME)}}` (D2).

use super::bruno::{fill_path, variables};
use super::{ImportBundle, ImportEnvironment, ImportItem, api, content_type_of, guess_content_type, kv};
use crate::auth::{Auth, KeyPlace};
use crate::graphql::GraphQlBody;
use crate::model::{ContentType, FolderSettings, KvParam};
use serde_json::Value;

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Whether `json` is a collection, or one of its requests alone.
pub(super) fn is_opencollection(json: &Value) -> bool {
    json.get("opencollection").is_some()
        || (json.get("info").is_some_and(Value::is_object)
            && (json.get("http").is_some() || json.get("graphql").is_some()))
}

fn list(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

fn enabled(value: &Value) -> bool {
    !value.get("disabled").and_then(Value::as_bool).unwrap_or(false)
}

/// `[{name, value, disabled}]`.
fn pairs(value: Option<&Value>) -> Vec<KvParam> {
    list(value)
        .iter()
        .map(|item| kv(text(item, "name"), &variables(text(item, "value")), enabled(item)))
        .collect()
}

/// A variable's value: text, or `{type, data}`.
fn value_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => variables(text),
        Some(object @ Value::Object(_)) => match object.get("data") {
            Some(Value::String(text)) => variables(text),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        },
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn variables_of(value: Option<&Value>) -> Vec<(String, String)> {
    list(value)
        .iter()
        .filter(|variable| enabled(variable) && !text(variable, "name").trim().is_empty())
        .map(|variable| {
            (
                text(variable, "name").trim().to_string(),
                value_of(variable.get("value")),
            )
        })
        .collect()
}

/// An `auth` as this app's: an object, or the words `inherit` / `none`;
/// `None` for a kind it has no counterpart of, which then inherits.
fn auth(value: &Value) -> Option<Auth> {
    let field = |key: &str| variables(text(value, key));
    let kind = value.as_str().unwrap_or_else(|| text(value, "type"));
    Some(match kind {
        "inherit" => Auth::Inherit,
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
            place: if text(value, "placement").contains("query") {
                KeyPlace::Query
            } else {
                KeyPlace::Header
            },
        },
        _ => return None,
    })
}

/// A folder's or the collection's `request`: what it sends and how it
/// signs in.
fn settings(value: &Value) -> FolderSettings {
    let request = value.get("request").unwrap_or(&Value::Null);
    FolderSettings {
        auth: request.get("auth").and_then(auth).unwrap_or_default(),
        headers: pairs(request.get("headers")),
    }
}

/// The body sent: the one, or the selected variant (the first when none
/// is).
fn body(value: &Value) -> &Value {
    match value {
        Value::Array(variants) => variants
            .iter()
            .find(|variant| variant.get("selected").and_then(Value::as_bool) == Some(true))
            .or(variants.first())
            .and_then(|variant| variant.get("body"))
            .unwrap_or(&Value::Null),
        one => one,
    }
}

fn request(item: &Value) -> Option<ImportItem> {
    let name = text(item.get("info").unwrap_or(&Value::Null), "name");
    let (graphql, details) = match (item.get("http"), item.get("graphql")) {
        (Some(http), _) => (false, http),
        (None, Some(graphql)) => (true, graphql),
        (None, None) => return None,
    };
    let params = list(details.get("params"));
    let of_type = |kind: &str| -> Vec<KvParam> {
        params
            .iter()
            .filter(|param| (text(param, "type").is_empty() && kind == "query") || text(param, "type") == kind)
            .map(|param| kv(text(param, "name"), &variables(text(param, "value")), enabled(param)))
            .collect()
    };
    let url = fill_path(&variables(text(details, "url")), &of_type("path"));
    let method = match text(details, "method") {
        "" if graphql => "POST",
        method => method,
    };
    let (name, mut built) = api(name, method, &url);
    let listed = of_type("query");
    if !listed.is_empty() {
        built.query = listed;
    }
    built.headers = pairs(details.get("headers"));
    if let Some(at) = built
        .headers
        .iter()
        .position(|header| header.key.eq_ignore_ascii_case("content-type") && content_type_of(&header.value).is_some())
    {
        let header = built.headers.remove(at);
        built.content_type = content_type_of(&header.value).unwrap_or_default();
    }
    let sent = body(details.get("body").unwrap_or(&Value::Null));
    if graphql {
        built.content_type = ContentType::GraphQl;
        built.body = GraphQlBody {
            query: variables(text(sent, "query")),
            variables: variables(text(sent, "variables")),
            operation_name: String::new(),
        }
        .to_body();
    } else {
        let data = sent.get("data");
        match text(sent, "type") {
            "json" => built.content_type = ContentType::Json,
            "xml" => built.content_type = ContentType::Xml,
            "text" | "sparql" => built.content_type = ContentType::Plain,
            "form-urlencoded" => built.content_type = ContentType::Form,
            "multipart-form" => built.content_type = ContentType::Multipart,
            _ => {}
        }
        match (text(sent, "type"), data) {
            ("form-urlencoded", _) => built.body = serde_json::to_string(&pairs(data)).unwrap_or_default(),
            ("multipart-form", _) => {
                let fields: Vec<KvParam> = list(data)
                    .iter()
                    .map(|field| {
                        let value = if text(field, "type") == "file" {
                            let path = match field.get("value") {
                                Some(Value::Array(paths)) => paths.first().and_then(Value::as_str).unwrap_or_default(),
                                Some(Value::String(path)) => path,
                                _ => "",
                            };
                            format!("file://{path}")
                        } else {
                            variables(text(field, "value"))
                        };
                        kv(text(field, "name"), &value, enabled(field))
                    })
                    .collect();
                built.body = serde_json::to_string(&fields).unwrap_or_default();
            }
            (kind, Some(Value::String(data))) => {
                built.body = variables(data);
                if kind.is_empty() && !built.body.is_empty() {
                    built.content_type = guess_content_type(&built.body);
                }
            }
            _ => {}
        }
    }
    if let Some(own) = details.get("auth").and_then(auth) {
        built.auth = own;
    }
    Some(ImportItem::Api {
        name,
        request: Box::new(built),
    })
}

fn items(value: Option<&Value>) -> Vec<ImportItem> {
    let mut items: Vec<&Value> = list(value).iter().collect();
    // As Bruno shows them: by `seq`, where they have one.
    items.sort_by_key(|item| {
        item.pointer("/info/seq")
            .and_then(Value::as_f64)
            .map_or(i64::MAX, |seq| seq as i64)
    });
    items
        .into_iter()
        .filter_map(|item| match text(item.get("info").unwrap_or(&Value::Null), "type") {
            "folder" => Some(ImportItem::Folder {
                name: text(item.get("info").unwrap_or(&Value::Null), "name")
                    .trim()
                    .to_string(),
                children: self::items(item.get("items")),
                settings: settings(item),
            }),
            "http" | "graphql" | "" => request(item),
            _ => None,
        })
        .collect()
}

pub(super) fn parse(json: &Value) -> ImportBundle {
    if json.get("opencollection").is_none() {
        return ImportBundle {
            items: request(json).into_iter().collect(),
            ..Default::default()
        };
    }
    let own = settings(json);
    let mut items = items(json.get("items"));
    if !own.headers.is_empty() {
        items = vec![ImportItem::Folder {
            name: text(json.get("info").unwrap_or(&Value::Null), "name")
                .trim()
                .to_string(),
            children: items,
            settings: FolderSettings {
                auth: Auth::Inherit,
                headers: own.headers,
            },
        }];
    }
    ImportBundle {
        items,
        environment: variables_of(json.pointer("/request/variables")),
        auth: own.auth,
        environments: list(json.pointer("/config/environments"))
            .iter()
            .map(|environment| ImportEnvironment {
                name: text(environment, "name").trim().to_string(),
                variables: variables_of(environment.get("variables")),
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

    fn request(item: &ImportItem) -> &crate::model::HttpRequest {
        match item {
            ImportItem::Api { request, .. } => request,
            ImportItem::Folder { .. } => panic!("an API"),
        }
    }

    fn yaml(text: &str) -> Value {
        serde_saphyr::from_str(text).expect("YAML")
    }

    #[test]
    fn a_collection_brings_its_items_auth_and_environments() {
        let json = yaml(
            r#"
opencollection: 1.0.0
info:
  name: Shop
config:
  environments:
    - name: dev
      variables:
        - name: host
          value: http://localhost:3000
        - name: token
          value:
            type: string
            data: "{{process.env.TOKEN}}"
        - name: old
          value: x
          disabled: true
request:
  auth:
    type: bearer
    token: "{{token}}"
  variables:
    - name: host
      value: https://shop.test
items:
  - info:
      name: health
      type: http
      seq: 2
    http:
      method: GET
      url: "{{host}}/health"
      auth: none
  - info:
      name: orders
      type: folder
      seq: 1
    request:
      auth:
        type: apikey
        key: X-Key
        value: k
        placement: query
      headers:
        - name: X-Shop
          value: "1"
    items:
      - info:
          name: create
          type: http
        http:
          method: POST
          url: "{{host}}/shops/:shop/orders"
          params:
            - name: shop
              value: "42"
              type: path
            - name: draft
              value: "1"
              type: query
            - name: lang
              value: en
              type: query
              disabled: true
          headers:
            - name: Content-Type
              value: application/json
          body:
            - title: small
              body:
                type: json
                data: '{"sku": "a"}'
            - title: big
              selected: true
              body:
                type: json
                data: '{"sku": "b", "count": 9}'
          auth: inherit
      - info:
          name: upload
          type: http
        http:
          method: PUT
          url: "{{host}}/files"
          body:
            type: multipart-form
            data:
              - name: note
                type: text
                value: hi
              - name: file
                type: file
                value: [/tmp/a.png]
      - info:
          name: socket
          type: websocket
  - info:
      name: me
      type: graphql
    graphql:
      url: "{{host}}/graphql"
      body:
        query: "{ me { id } }"
        variables: '{"a": 1}'
"#,
        );
        assert!(is_opencollection(&json));
        let bundle = parse(&json);
        assert_eq!(
            bundle.auth,
            Auth::Bearer {
                token: "{{token}}".into()
            }
        );
        assert_eq!(
            bundle.environment,
            [("host".to_string(), "https://shop.test".to_string())]
        );
        assert_eq!(
            bundle.environments,
            [ImportEnvironment {
                name: "dev".into(),
                variables: vec![
                    ("host".into(), "http://localhost:3000".into()),
                    ("token".into(), "{{env(TOKEN)}}".into())
                ],
            }]
        );
        let ImportItem::Folder {
            name,
            children,
            settings,
        } = &bundle.items[0]
        else {
            panic!("the folder first, by seq: {:?}", bundle.items);
        };
        assert_eq!(name, "orders");
        assert_eq!(settings.headers, [kv("X-Shop", "1", true)]);
        assert_eq!(
            settings.auth,
            Auth::ApiKey {
                name: "X-Key".into(),
                value: "k".into(),
                place: KeyPlace::Query
            }
        );
        assert_eq!(children.len(), 2, "the WebSocket left out");
        let create = request(&children[0]);
        assert_eq!(create.method, HttpMethod::Post);
        assert_eq!(create.uri, "{{host}}/shops/42/orders");
        assert_eq!(create.query, [kv("draft", "1", true), kv("lang", "en", false)]);
        assert!(create.headers.is_empty());
        assert_eq!(create.content_type, ContentType::Json);
        assert_eq!(create.body, r#"{"sku": "b", "count": 9}"#, "the selected variant");
        assert_eq!(create.auth, Auth::Inherit);
        let upload = request(&children[1]);
        assert_eq!(upload.content_type, ContentType::Multipart);
        assert_eq!(
            crate::send::form_fields(&upload.body),
            [kv("note", "hi", true), kv("file", "file:///tmp/a.png", true)]
        );
        assert_eq!(request(&bundle.items[1]).auth, Auth::None);
        let me = request(&bundle.items[2]);
        assert_eq!(me.method, HttpMethod::Post);
        assert_eq!(me.content_type, ContentType::GraphQl);
        assert_eq!(GraphQlBody::read(&me.body).query, "{ me { id } }");
    }

    #[test]
    fn one_request_file_alone() {
        let json = yaml(
            r#"
info:
  name: login
  type: http
http:
  method: POST
  url: https://a.test/login
  body:
    type: form-urlencoded
    data:
      - name: user
        value: me
      - name: debug
        value: "1"
        disabled: true
  auth:
    type: basic
    username: u
    password: "{{process.env.PW}}"
"#,
        );
        assert!(is_opencollection(&json));
        let bundle = parse(&json);
        let [ImportItem::Api { name, request }] = &bundle.items[..] else {
            panic!("one API: {:?}", bundle.items);
        };
        assert_eq!(name, "login");
        assert_eq!(request.content_type, ContentType::Form);
        assert_eq!(
            crate::send::form_fields(&request.body),
            [kv("user", "me", true), kv("debug", "1", false)]
        );
        assert_eq!(
            request.auth,
            Auth::Basic {
                username: "u".into(),
                password: "{{env(PW)}}".into()
            }
        );
    }
}
