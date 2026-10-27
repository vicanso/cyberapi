//! B5: a Hoppscotch collection — one, or the list "export all" writes — as
//! folders and APIs (F10.3), and a Hoppscotch environment as a named one.
//!
//! A collection with more than its requests (folders, or several
//! collections) is a folder; its `folders` nest, its `requests` are APIs,
//! REST ones (`endpoint`) and GraphQL ones (`url` and `query`). Hoppscotch
//! writes a variable as `<<name>>`; it comes in as this app's `{{name}}`.
//! Auth — Basic, Bearer, API key, Digest, none, inherit — comes in as this
//! app's (#66) at every level, one switched off as none; a kind this app
//! has no counterpart of (OAuth, AWS…) inherits. A collection's own
//! variables are the variables every environment shares.

use super::{ImportBundle, ImportEnvironment, ImportItem, api, content_type_of, guess_content_type, kv};
use crate::auth::{Auth, KeyPlace};
use crate::graphql::GraphQlBody;
use crate::model::{ContentType, FolderSettings, HttpMethod, KvParam};
use crate::oauth::{ClientAuth, Grant, OAuth2};
use serde_json::Value;

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Whether `json` is a collection, or a list of them.
pub(super) fn is_collection(json: &Value) -> bool {
    let one = |value: &Value| {
        value.get("folders").is_some_and(Value::is_array) && value.get("requests").is_some_and(Value::is_array)
    };
    match json {
        Value::Array(items) => !items.is_empty() && items.iter().all(one),
        other => one(other),
    }
}

/// Whether `json` is an environment, or a list of them: named variables,
/// which a collection has too, without its requests.
pub(super) fn is_environment(json: &Value) -> bool {
    let one = |value: &Value| {
        value.get("requests").is_none()
            && value.get("name").is_some_and(Value::is_string)
            && value
                .get("variables")
                .and_then(Value::as_array)
                .is_some_and(|variables| variables.iter().all(|variable| variable.get("key").is_some()))
    };
    match json {
        Value::Array(items) => !items.is_empty() && items.iter().all(one),
        other => one(other),
    }
}

/// `<<host>>` → `{{host}}`.
fn variables(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<<") {
        let after = &rest[start + 2..];
        match after.find(">>") {
            Some(end) if is_name(&after[..end]) => {
                out.push_str(&rest[..start]);
                out.push_str(&format!("{{{{{}}}}}", &after[..end]));
                rest = &after[end + 2..];
            }
            _ => {
                out.push_str(&rest[..start + 2]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// What Hoppscotch takes for a variable's name: no space, nothing that
/// closes or opens another.
fn is_name(text: &str) -> bool {
    !text.is_empty() && !text.contains(|c: char| c.is_whitespace() || c == '<' || c == '>')
}

/// `[{key, value, active}]`.
fn pairs(value: Option<&Value>) -> Vec<KvParam> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    kv(
                        &variables(text(item, "key")),
                        &variables(text(item, "value")),
                        item.get("active").and_then(Value::as_bool).unwrap_or(true),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// An urlencoded body: a line a field, `key: value`, one switched off
/// behind `#`; a key or value with `:` or `#` in it in JSON quotes.
fn raw_pairs(text: &str) -> Vec<KvParam> {
    fn part(text: &str) -> String {
        let text = text.trim();
        if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
            return serde_json::from_str(text).unwrap_or_else(|_| text.to_string());
        }
        text.to_string()
    }
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (enabled, line) = match line.strip_prefix('#') {
                Some(rest) => (false, rest),
                None => (true, line),
            };
            // A quoted key may hold a colon: split after its closing quote.
            let split = if line.trim_start().starts_with('"') {
                let start = line.find('"').unwrap_or_default();
                line[start + 1..]
                    .match_indices('"')
                    .find(|(at, _)| !line[start + 1..start + 1 + at].ends_with('\\'))
                    .and_then(|(at, _)| {
                        let after = start + 1 + at + 1;
                        line[after..].find(':').map(|colon| after + colon)
                    })
            } else {
                line.find(':')
            };
            let (key, value) = match split {
                Some(at) => (&line[..at], &line[at + 1..]),
                None => (line, ""),
            };
            let key = part(key);
            (!key.is_empty()).then(|| kv(&variables(&key), &variables(&part(value)), enabled))
        })
        .collect()
}

/// An `auth` object as this app's; `None` for a kind it has no counterpart
/// of, which then inherits.
fn auth(value: &Value) -> Option<Auth> {
    let field = |key: &str| variables(text(value, key));
    if value.get("authActive").and_then(Value::as_bool) == Some(false) {
        return Some(Auth::None);
    }
    Some(match text(value, "authType") {
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
        "api-key" => Auth::ApiKey {
            name: field("key"),
            value: field("value"),
            place: if text(value, "addTo").to_ascii_lowercase().contains("query") {
                KeyPlace::Query
            } else {
                KeyPlace::Header
            },
        },
        // C4: the grants asked without a browser. Their token endpoint is
        // `authEndpoint`.
        "oauth-2" => {
            let info = value.get("grantTypeInfo").unwrap_or(&Value::Null);
            let field = |key: &str| variables(text(info, key));
            let grant = match text(info, "grantType") {
                "CLIENT_CREDENTIALS" => Grant::ClientCredentials,
                "PASSWORD" => Grant::Password,
                _ => return None,
            };
            Auth::from(OAuth2 {
                grant,
                token_url: [field("tokenEndpoint"), field("authEndpoint")]
                    .into_iter()
                    .find(|url| !url.is_empty())
                    .unwrap_or_default(),
                client_id: field("clientID"),
                client_secret: field("clientSecret"),
                username: field("username"),
                password: field("password"),
                scope: field("scopes"),
                audience: String::new(),
                resource: String::new(),
                client_auth: match text(info, "clientAuthentication") {
                    "IN_BODY" => ClientAuth::Body,
                    _ => ClientAuth::Header,
                },
            })
        }
        _ => return None,
    })
}

fn request(value: &Value) -> ImportItem {
    // GraphQL's requests have a `url` and a `query`, REST's an `endpoint`.
    if value.get("endpoint").is_none() && value.get("query").is_some_and(Value::is_string) {
        let (name, mut built) = api(text(value, "name"), "POST", &variables(text(value, "url")));
        built.headers = pairs(value.get("headers"));
        built.content_type = ContentType::GraphQl;
        built.body = GraphQlBody {
            query: variables(text(value, "query")),
            variables: variables(text(value, "variables")),
            operation_name: String::new(),
        }
        .to_body();
        if let Some(own) = value.get("auth").and_then(auth) {
            built.auth = own;
        }
        return ImportItem::Api {
            name,
            request: Box::new(built),
        };
    }
    let (name, mut built) = api(
        text(value, "name"),
        text(value, "method"),
        &variables(text(value, "endpoint")),
    );
    built.query.extend(pairs(value.get("params")));
    built.headers = pairs(value.get("headers"));
    if let Some(body) = value.get("body") {
        let mime = text(body, "contentType");
        if let Some(known) = content_type_of(mime) {
            built.content_type = known;
        }
        match (mime, body.get("body")) {
            ("", _) | (_, None | Some(Value::Null)) => {}
            ("application/x-www-form-urlencoded", Some(Value::String(raw))) => {
                built.body = serde_json::to_string(&raw_pairs(raw)).unwrap_or_default();
            }
            ("multipart/form-data", Some(Value::Array(fields))) => {
                // A file's content stays behind in Hoppscotch: the reader
                // picks it again.
                let fields: Vec<KvParam> = fields
                    .iter()
                    .map(|field| {
                        let value = if field.get("isFile").and_then(Value::as_bool) == Some(true) {
                            "file://".to_string()
                        } else {
                            variables(text(field, "value"))
                        };
                        kv(
                            &variables(text(field, "key")),
                            &value,
                            field.get("active").and_then(Value::as_bool).unwrap_or(true),
                        )
                    })
                    .collect();
                built.body = serde_json::to_string(&fields).unwrap_or_default();
            }
            (_, Some(Value::String(raw))) => {
                built.body = variables(raw);
                if content_type_of(mime).is_none() {
                    // Its JSON kinds (`application/ld+json`…) and HTML: the
                    // header says which, the editor what it looks like.
                    built.content_type = guess_content_type(&built.body);
                    if !built
                        .headers
                        .iter()
                        .any(|header| header.key.eq_ignore_ascii_case("content-type"))
                    {
                        built.headers.push(kv("Content-Type", mime, true));
                    }
                }
            }
            _ => {}
        }
    }
    if !built.method.has_body() && built.method != HttpMethod::Get {
        built.content_type = ContentType::default();
    }
    if let Some(own) = value.get("auth").and_then(auth) {
        built.auth = own;
    }
    ImportItem::Api {
        name,
        request: Box::new(built),
    }
}

/// A collection or folder as a folder.
fn folder(value: &Value) -> ImportItem {
    ImportItem::Folder {
        name: text(value, "name").trim().to_string(),
        children: children(value),
        settings: settings(value),
    }
}

fn settings(value: &Value) -> FolderSettings {
    FolderSettings {
        auth: value.get("auth").and_then(auth).unwrap_or_default(),
        headers: pairs(value.get("headers")),
        // F3.
        description: text(value, "description").trim().to_string(),
    }
}

/// A collection's or a folder's folders, then its requests, as Hoppscotch
/// lists them.
fn children(value: &Value) -> Vec<ImportItem> {
    let list = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
    };
    list("folders")
        .iter()
        .map(folder)
        .chain(list("requests").iter().map(request))
        .collect()
}

/// A collection's variables: what it starts with, which is what an export
/// has.
fn collection_variables(value: &Value) -> Vec<(String, String)> {
    value
        .get("variables")
        .and_then(Value::as_array)
        .map(|variables| {
            variables
                .iter()
                .filter(|variable| !text(variable, "key").trim().is_empty())
                .map(|variable| {
                    let value = ["initialValue", "value", "currentValue"]
                        .iter()
                        .map(|key| text(variable, key))
                        .find(|value| !value.is_empty())
                        .unwrap_or_default();
                    (text(variable, "key").trim().to_string(), self::variables(value))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn parse(json: &Value) -> ImportBundle {
    match json {
        // "Export all": each collection a folder.
        Value::Array(collections) if collections.len() > 1 => ImportBundle {
            items: collections.iter().map(folder).collect(),
            environment: collections.iter().flat_map(collection_variables).collect(),
            ..Default::default()
        },
        Value::Array(collections) => collections.first().map(parse).unwrap_or_default(),
        // One: its contents, signing in and sending headers as the project.
        collection => {
            let own = settings(collection);
            let mut items = children(collection);
            // The project has no headers of its own to put them in: a
            // collection that sends some keeps them on a folder.
            if !own.headers.is_empty() {
                items = vec![ImportItem::Folder {
                    name: text(collection, "name").trim().to_string(),
                    children: items,
                    settings: FolderSettings {
                        auth: Auth::Inherit,
                        headers: own.headers,
                        description: String::new(),
                    },
                }];
            }
            ImportBundle {
                items,
                environment: collection_variables(collection),
                auth: own.auth,
                ..Default::default()
            }
        }
    }
}

/// An environment, or a list of them, each a named environment.
pub(super) fn parse_environment(json: &Value) -> ImportBundle {
    let list: Vec<&Value> = match json {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    ImportBundle {
        environments: list
            .into_iter()
            .map(|environment| ImportEnvironment {
                name: text(environment, "name").trim().to_string(),
                variables: collection_variables(environment),
            })
            .filter(|environment| !environment.name.is_empty())
            .collect(),
        ..Default::default()
    }
}

/// A collection's name, for a project made from it: the one collection's.
pub(super) fn name(json: &Value) -> Option<&str> {
    match json {
        Value::Array(collections) if collections.len() == 1 => collections.first().map(|first| text(first, "name")),
        Value::Array(_) => None,
        collection => Some(text(collection, "name")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn request(item: &ImportItem) -> &crate::model::HttpRequest {
        match item {
            ImportItem::Api { request, .. } => request,
            ImportItem::Folder { .. } => panic!("an API"),
        }
    }

    #[test]
    fn a_collection_brings_its_folders_requests_and_auth() {
        let json: Value = serde_json::from_str(
            r#"{"v": 12, "name": "Shop", "folders": [
                {"v": 12, "name": "orders", "folders": [], "headers": [],
                 "auth": {"authType": "api-key", "authActive": true, "key": "X-Key", "value": "<<key>>", "addTo": "QUERY_PARAMS"},
                 "requests": [
                    {"v": "17", "name": "create", "method": "POST", "endpoint": "<<host>>/orders?draft=1",
                     "params": [{"key": "lang", "value": "en", "active": false}],
                     "headers": [{"key": "X-Trace", "value": "<<trace>>", "active": true}],
                     "auth": {"authType": "inherit", "authActive": true},
                     "body": {"contentType": "application/json", "body": "{\"sku\": \"<<sku>>\"}"}},
                    {"v": "17", "name": "login", "method": "POST", "endpoint": "<<host>>/login",
                     "auth": {"authType": "basic", "authActive": false, "username": "u", "password": "p"},
                     "body": {"contentType": "application/x-www-form-urlencoded",
                              "body": "user: <<user>>\n# debug: 1\n\"a:b\": c: d\n"}},
                    {"v": "17", "name": "upload", "method": "PUT", "endpoint": "<<host>>/files",
                     "body": {"contentType": "multipart/form-data", "body": [
                        {"key": "note", "value": "hi", "active": true, "isFile": false},
                        {"key": "file", "value": [{}], "active": true, "isFile": true}]}},
                    {"v": "17", "name": "doc", "method": "POST", "endpoint": "<<host>>/doc",
                     "body": {"contentType": "application/ld+json", "body": "{\"@id\": 1}"}}
                 ]}
             ],
             "requests": [
                {"v": "17", "name": "health", "method": "GET", "endpoint": "https://shop.test/h <<x",
                 "auth": {"authType": "digest", "authActive": true, "username": "d", "password": "<<pw>>"},
                 "body": {"contentType": null, "body": null}},
                {"v": 8, "name": "me", "url": "<<host>>/graphql", "headers": [],
                 "query": "query { me { id } }", "variables": "{\"id\": \"<<user>>\"}",
                 "auth": {"authType": "bearer", "authActive": true, "token": "<<token>>"}}
             ],
             "auth": {"authType": "bearer", "authActive": true, "token": "<<token>>"},
             "headers": [],
             "variables": [{"key": "host", "initialValue": "https://shop.test", "currentValue": "", "secret": false}]
            }"#,
        )
        .expect("JSON");
        assert!(is_collection(&json));
        assert!(!is_environment(&json));
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
        let ImportItem::Folder {
            name,
            children,
            settings,
        } = &bundle.items[0]
        else {
            panic!("a folder first: {:?}", bundle.items);
        };
        assert_eq!(name, "orders");
        assert_eq!(
            settings.auth,
            Auth::ApiKey {
                name: "X-Key".into(),
                value: "{{key}}".into(),
                place: KeyPlace::Query
            }
        );
        let create = request(&children[0]);
        assert_eq!(create.uri, "{{host}}/orders");
        assert_eq!(create.query, [kv("draft", "1", true), kv("lang", "en", false)]);
        assert_eq!(create.headers, [kv("X-Trace", "{{trace}}", true)]);
        assert_eq!(create.content_type, ContentType::Json);
        assert_eq!(create.body, "{\"sku\": \"{{sku}}\"}");
        assert_eq!(create.auth, Auth::Inherit);
        let login = request(&children[1]);
        assert_eq!(login.auth, Auth::None, "switched off");
        assert_eq!(login.content_type, ContentType::Form);
        assert_eq!(
            crate::send::form_fields(&login.body),
            [
                kv("user", "{{user}}", true),
                kv("debug", "1", false),
                kv("a:b", "c: d", true)
            ]
        );
        let upload = request(&children[2]);
        assert_eq!(
            crate::send::form_fields(&upload.body),
            [kv("note", "hi", true), kv("file", "file://", true)]
        );
        let doc = request(&children[3]);
        assert_eq!(doc.content_type, ContentType::Json);
        assert_eq!(doc.headers, [kv("Content-Type", "application/ld+json", true)]);
        let health = request(&bundle.items[1]);
        assert_eq!(health.uri, "https://shop.test/h <<x", "not a variable: left as written");
        assert_eq!(
            health.auth,
            Auth::Digest {
                username: "d".into(),
                password: "{{pw}}".into()
            }
        );
        let me = request(&bundle.items[2]);
        assert_eq!(me.method, HttpMethod::Post);
        assert_eq!(me.content_type, ContentType::GraphQl);
        let graphql = GraphQlBody::read(&me.body);
        assert_eq!(graphql.query, "query { me { id } }");
        assert!(graphql.variables.contains("{{user}}"), "{}", graphql.variables);
        assert_eq!(super::name(&json), Some("Shop"));
    }

    #[test]
    fn export_all_is_a_folder_each_and_headers_keep_a_folder() {
        let json: Value = serde_json::from_str(
            r#"[{"name": "A", "folders": [], "requests": [], "headers": [{"key": "X-A", "value": "1", "active": true}]},
                {"name": "B", "folders": [], "requests": []}]"#,
        )
        .expect("JSON");
        assert!(is_collection(&json));
        let bundle = parse(&json);
        let names: Vec<&str> = bundle
            .items
            .iter()
            .map(|item| match item {
                ImportItem::Folder { name, .. } => name.as_str(),
                ImportItem::Api { .. } => panic!("folders"),
            })
            .collect();
        assert_eq!(names, ["A", "B"]);
        assert_eq!(name(&json), None);
        let one: Value = serde_json::from_str(
            r#"{"name": "A", "folders": [], "requests": [{"name": "x", "method": "GET", "endpoint": "/x"}],
                "headers": [{"key": "X-A", "value": "1", "active": true}]}"#,
        )
        .expect("JSON");
        let bundle = parse(&one);
        let [ImportItem::Folder { settings, children, .. }] = &bundle.items[..] else {
            panic!("its headers on a folder: {:?}", bundle.items);
        };
        assert_eq!(settings.headers, [kv("X-A", "1", true)]);
        assert_eq!(children.len(), 1);
    }

    #[test]
    fn an_environment_is_a_named_one() {
        let json: Value = serde_json::from_str(
            r#"[{"v": 2, "id": "1", "name": "dev", "variables": [
                    {"key": "host", "initialValue": "http://localhost", "currentValue": "x", "secret": false},
                    {"key": "token", "value": "<<secret>>", "secret": true}]},
                {"name": "prod", "variables": []}]"#,
        )
        .expect("JSON");
        assert!(is_environment(&json));
        assert!(!is_collection(&json));
        assert_eq!(
            parse_environment(&json).environments,
            [
                ImportEnvironment {
                    name: "dev".into(),
                    variables: vec![
                        ("host".into(), "http://localhost".into()),
                        ("token".into(), "{{secret}}".into())
                    ],
                },
                ImportEnvironment {
                    name: "prod".into(),
                    variables: vec![],
                },
            ]
        );
    }
}
