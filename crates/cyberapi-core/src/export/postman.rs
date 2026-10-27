//! B2: a Postman collection (v2.1). Folders keep their hierarchy and their
//! auth; a request its query and headers (those switched off too), its
//! body in Postman's mode for it, and its auth. The collection signs in as
//! the project does, or as the exported folder does; the variables every
//! environment shares are its variables. This app's `{{NAME}}` is
//! Postman's syntax already.

use super::{Item, exported_auth, items};
use crate::ItemId;
use crate::auth::{Auth, KeyPlace};
use crate::graphql::GraphQlBody;
use crate::model::{ContentType, HttpRequest, KvParam};
use crate::oauth::{ClientAuth, Grant};
use crate::send::form_fields;
use crate::tree::TreeData;
use serde_json::{Map, Value, json};

const SCHEMA: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

/// The collection of `folder`, or of the whole project, named `name`, with
/// `variables` as its own.
pub fn postman(data: &TreeData, folder: Option<&ItemId>, name: &str, variables: &[(String, String)]) -> String {
    let mut collection = Map::new();
    collection.insert("info".into(), json!({"name": name, "schema": SCHEMA}));
    collection.insert(
        "item".into(),
        Value::Array(items(data, folder).iter().map(item).collect()),
    );
    if let Some(auth) = auth(&exported_auth(data, folder)) {
        collection.insert("auth".into(), auth);
    }
    if !variables.is_empty() {
        collection.insert(
            "variable".into(),
            variables
                .iter()
                .map(|(key, value)| json!({"key": key, "value": value, "type": "string"}))
                .collect(),
        );
    }
    crate::format::json(&Value::Object(collection))
}

/// Postman's `auth`; nothing for inheriting, Postman's default.
fn auth(auth: &Auth) -> Option<Value> {
    let fields = |kind: &str, fields: &[(&str, &str)]| {
        let listed: Vec<Value> = fields
            .iter()
            .map(|(key, value)| json!({"key": key, "value": value, "type": "string"}))
            .collect();
        json!({"type": kind, kind: listed})
    };
    Some(match auth {
        Auth::Inherit => return None,
        Auth::None => json!({"type": "noauth"}),
        Auth::Basic { username, password } => fields("basic", &[("username", username), ("password", password)]),
        Auth::Digest { username, password } => fields("digest", &[("username", username), ("password", password)]),
        Auth::Bearer { token } => fields("bearer", &[("token", token)]),
        // C4: the grant Postman asks with, unless it asks in a browser.
        Auth::OAuth2(oauth) => fields(
            "oauth2",
            &[
                (
                    "grant_type",
                    match oauth.grant {
                        Grant::ClientCredentials => "client_credentials",
                        Grant::Password => "password_credentials",
                    },
                ),
                ("accessTokenUrl", &oauth.token_url),
                ("clientId", &oauth.client_id),
                ("clientSecret", &oauth.client_secret),
                ("username", &oauth.username),
                ("password", &oauth.password),
                ("scope", &oauth.scope),
                ("audience", &oauth.audience),
                ("resource", &oauth.resource),
                (
                    "client_authentication",
                    match oauth.client_auth {
                        ClientAuth::Header => "header",
                        ClientAuth::Body => "body",
                    },
                ),
                ("addTokenTo", "header"),
            ],
        ),
        Auth::ApiKey { name, value, place } => fields(
            "apikey",
            &[
                ("key", name),
                ("value", value),
                (
                    "in",
                    match place {
                        KeyPlace::Header => "header",
                        KeyPlace::Query => "query",
                    },
                ),
            ],
        ),
    })
}

fn item(item: &Item<'_>) -> Value {
    match item {
        Item::Folder(folder, children) => {
            let mut out = json!({"name": folder.name, "item": children.iter().map(self::item).collect::<Vec<_>>()});
            if let Some(auth) = auth(&folder.settings.auth) {
                out["auth"] = auth;
            }
            // F3.
            if !folder.settings.description.trim().is_empty() {
                out["description"] = json!(folder.settings.description.trim());
            }
            out
        }
        Item::Api(api) => {
            let mut out = json!({"name": api.name, "request": request(&api.request)});
            // F3: its examples, as Postman's saved responses.
            if !api.request.examples.is_empty() {
                out["response"] = api
                    .request
                    .examples
                    .iter()
                    .map(|example| {
                        json!({
                            "name": example.name,
                            "code": example.status,
                            "status": crate::response::status_text(example.status),
                            "header": example.headers.iter().map(|(key, value)| json!({"key": key, "value": value})).collect::<Vec<_>>(),
                            "body": example.body,
                        })
                    })
                    .collect();
            }
            out
        }
    }
}

/// `[{key, value, disabled}]`.
fn pairs(list: &[KvParam]) -> Vec<Value> {
    list.iter()
        .filter(|kv| !kv.key.trim().is_empty())
        .map(|kv| {
            let mut pair = json!({"key": kv.key, "value": kv.value});
            if !kv.enabled {
                pair["disabled"] = Value::Bool(true);
            }
            pair
        })
        .collect()
}

fn request(request: &HttpRequest) -> Value {
    let mut headers = pairs(&request.headers);
    let query: Vec<&KvParam> = request
        .query
        .iter()
        .filter(|kv| kv.enabled && !kv.key.is_empty())
        .collect();
    let mut raw = request.uri.clone();
    for (index, kv) in query.iter().enumerate() {
        let joint = if index == 0 && !raw.contains('?') { '?' } else { '&' };
        raw.push(joint);
        raw.push_str(&kv.key);
        raw.push('=');
        raw.push_str(&kv.value);
    }
    let mut url = json!({"raw": raw});
    if !request.query.is_empty() {
        url["query"] = Value::Array(pairs(&request.query));
    }
    let named = |name: &str| {
        request
            .headers
            .iter()
            .any(|kv| kv.enabled && kv.key.trim().eq_ignore_ascii_case(name))
    };
    let body = if request.has_body_tab() {
        match request.content_type {
            ContentType::Json | ContentType::Xml | ContentType::Plain if !request.body.trim().is_empty() => {
                let language = match request.content_type {
                    ContentType::Json => "json",
                    ContentType::Xml => "xml",
                    _ => "text",
                };
                // Postman sends what its headers say, so the type goes there.
                if !named("content-type") {
                    headers.push(json!({"key": "Content-Type", "value": request.content_type.as_str()}));
                }
                Some(json!({"mode": "raw", "raw": request.body, "options": {"raw": {"language": language}}}))
            }
            ContentType::Form => Some(json!({"mode": "urlencoded", "urlencoded": pairs(&form_fields(&request.body))})),
            ContentType::Multipart => {
                let fields: Vec<Value> = form_fields(&request.body)
                    .iter()
                    .filter(|kv| !kv.key.trim().is_empty())
                    .map(|kv| {
                        let mut field = match kv.value.strip_prefix("file://") {
                            Some(path) => json!({"key": kv.key, "type": "file", "src": path}),
                            None => json!({"key": kv.key, "type": "text", "value": kv.value}),
                        };
                        if !kv.enabled {
                            field["disabled"] = Value::Bool(true);
                        }
                        field
                    })
                    .collect();
                Some(json!({"mode": "formdata", "formdata": fields}))
            }
            ContentType::GraphQl => {
                let graphql = GraphQlBody::read(&request.body);
                Some(json!({"mode": "graphql", "graphql": {"query": graphql.query, "variables": graphql.variables}}))
            }
            _ => None,
        }
    } else {
        None
    };
    let mut out = json!({"method": request.method.as_str(), "header": headers, "url": url});
    // F3.
    if !request.description.trim().is_empty() {
        out["description"] = json!(request.description.trim());
    }
    if let Some(body) = body {
        out["body"] = body;
    }
    if let Some(auth) = auth(&request.auth) {
        out["auth"] = auth;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ApiCategory, ApiSetting, Folder, FolderSettings, HttpMethod, ProjectSettings};
    use crate::{CollectionId, ItemId};
    use pretty_assertions::assert_eq;

    fn api(id: &str, name: &str, request: HttpRequest) -> ApiSetting {
        ApiSetting {
            id: ItemId::new(id),
            collection: CollectionId::new("c"),
            name: name.into(),
            category: ApiCategory::Http,
            request,
            created_at: Default::default(),
            updated_at: Default::default(),
        }
    }

    fn kv(key: &str, value: &str, enabled: bool) -> KvParam {
        KvParam {
            key: key.into(),
            value: value.into(),
            enabled,
        }
    }

    fn shop() -> TreeData {
        TreeData {
            folders: vec![Folder {
                id: ItemId::new("f1"),
                collection: CollectionId::new("c"),
                name: "orders".into(),
                children: vec![ItemId::new("a1"), ItemId::new("a2")],
                settings: FolderSettings {
                    auth: Auth::ApiKey {
                        name: "X-Key".into(),
                        value: "{{key}}".into(),
                        place: KeyPlace::Header,
                    },
                    headers: Vec::new(),
                    description: "Orders, made and listed.".into(),
                },
                created_at: Default::default(),
                updated_at: Default::default(),
            }],
            apis: vec![
                api(
                    "a1",
                    "list",
                    HttpRequest {
                        method: HttpMethod::Get,
                        uri: "{{HOST}}/orders".into(),
                        query: vec![kv("page", "1", true), kv("debug", "1", false)],
                        headers: vec![kv("Accept", "application/json", true)],
                        ..Default::default()
                    },
                ),
                api(
                    "a2",
                    "create",
                    HttpRequest {
                        method: HttpMethod::Post,
                        uri: "{{HOST}}/orders".into(),
                        content_type: ContentType::Json,
                        body: "{\"sku\": \"a\"}".into(),
                        auth: Auth::Basic {
                            username: "u".into(),
                            password: "p".into(),
                        },
                        description: "Makes an order.".into(),
                        examples: vec![crate::model::Example {
                            name: "made".into(),
                            status: 201,
                            headers: vec![("Content-Type".into(), "application/json".into())],
                            body: "{\"id\": 7}".into(),
                        }],
                        ..Default::default()
                    },
                ),
                api(
                    "a3",
                    "grpc",
                    HttpRequest {
                        method: HttpMethod::Grpc,
                        uri: "localhost:50051".into(),
                        ..Default::default()
                    },
                ),
            ],
            top_order: vec![ItemId::new("f1"), ItemId::new("a3")],
            settings: ProjectSettings {
                auth: Auth::Bearer {
                    token: "{{token}}".into(),
                },
            },
        }
    }

    /// What this app writes, its own Postman import reads back the same.
    #[test]
    fn a_collection_reads_back_as_it_was() {
        let text = postman(&shop(), None, "Shop", &[("HOST".into(), "https://shop.test".into())]);
        let bundle = crate::import::parse(&text).expect("a collection");
        assert_eq!(bundle.name.as_deref(), Some("Shop"));
        assert_eq!(
            bundle.auth,
            Auth::Bearer {
                token: "{{token}}".into()
            }
        );
        assert_eq!(
            bundle.environment,
            [("HOST".to_string(), "https://shop.test".to_string())]
        );
        assert_eq!(bundle.items.len(), 1, "the gRPC call is left out");
        let crate::import::ImportItem::Folder {
            name,
            children,
            settings,
        } = &bundle.items[0]
        else {
            panic!("a folder");
        };
        assert_eq!(name, "orders");
        assert_eq!(settings.auth, shop().folders[0].settings.auth);
        let requests: Vec<&HttpRequest> = children
            .iter()
            .map(|child| match child {
                crate::import::ImportItem::Api { request, .. } => &**request,
                crate::import::ImportItem::Folder { .. } => panic!("an API"),
            })
            .collect();
        assert_eq!(requests[0].uri, "{{HOST}}/orders");
        assert_eq!(requests[0].query, [kv("page", "1", true), kv("debug", "1", false)]);
        assert_eq!(requests[0].headers, [kv("Accept", "application/json", true)]);
        assert_eq!(requests[1].method, HttpMethod::Post);
        assert_eq!(requests[1].content_type, ContentType::Json);
        assert_eq!(requests[1].body, "{\"sku\": \"a\"}");
        assert_eq!(requests[1].auth, shop().apis[1].request.auth);
        // F3: what it is for, and its examples.
        assert_eq!(requests[1].description, "Makes an order.");
        assert_eq!(requests[1].examples, shop().apis[1].request.examples);
        assert_eq!(settings.description, "Orders, made and listed.");
    }

    /// C4: OAuth goes as Postman's, and this app's import reads it back.
    #[test]
    fn oauth_comes_back() {
        let oauth = Auth::from(crate::oauth::OAuth2 {
            grant: Grant::Password,
            token_url: "{{ID}}/token".into(),
            client_id: "app".into(),
            client_secret: "{{value(secret)}}".into(),
            username: "me".into(),
            password: "pw".into(),
            scope: "read".into(),
            audience: "https://api.test".into(),
            resource: String::new(),
            client_auth: ClientAuth::Body,
        });
        let mut data = shop();
        data.settings.auth = oauth.clone();
        let text = postman(&data, None, "shop", &[]);
        let bundle = crate::import::parse(&text).expect("a collection");
        assert_eq!(bundle.auth, oauth);
    }

    /// A folder alone signs in as it does, its own or from above.
    #[test]
    fn a_folder_takes_its_auth_along() {
        let text = postman(&shop(), Some(&ItemId::new("f1")), "orders", &[]);
        let json: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(json["auth"]["type"], "apikey");
        assert_eq!(json["item"].as_array().map(Vec::len), Some(2));
        assert!(json.get("variable").is_none());
    }
}
