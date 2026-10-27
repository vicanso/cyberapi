//! B1: an OpenAPI 3.0 document, as YAML. An API's address is a server and
//! a path: the value its `{{NAME}}` prefix has in the reader's environment,
//! or the scheme and host it is written with; a server other than the first
//! is named on its path. A path segment that is a variable (`{{id}}`) is a
//! path parameter (`{id}`). Query and header parameters carry their values
//! as examples, a body is an example with a schema read off it, and the
//! folders an API is in are its tag. How the project, a folder or an API
//! signs in becomes a security scheme — what kind, never the values. A gRPC
//! call has no place in it.

use super::{Item, exported_auth, items};
use crate::ItemId;
use crate::auth::{Auth, KeyPlace};
use crate::graphql::GraphQlBody;
use crate::inherit;
use crate::model::{ApiSetting, ContentType, HttpRequest, split_env_prefix};
use crate::oauth::Grant;
use crate::send::form_fields;
use crate::tree::TreeData;
use serde_json::{Map, Value, json};

/// Headers OpenAPI describes in other ways, which a parameter may not name.
const UNNAMED_HEADERS: [&str; 3] = ["accept", "content-type", "authorization"];

/// The document of `folder`, or of the whole project, titled `title`;
/// `variables` give an address's `{{NAME}}` prefix its value.
pub fn openapi(data: &TreeData, folder: Option<&ItemId>, title: &str, variables: &[(String, String)]) -> String {
    let mut apis: Vec<(&ApiSetting, String)> = Vec::new();
    fn flatten<'a>(items: &[Item<'a>], tag: &str, out: &mut Vec<(&'a ApiSetting, String)>) {
        for item in items {
            match item {
                Item::Api(api) => out.push((api, tag.to_string())),
                Item::Folder(folder, children) => {
                    let tag = if tag.is_empty() {
                        folder.name.clone()
                    } else {
                        format!("{tag} / {}", folder.name)
                    };
                    flatten(children, &tag, out);
                }
            }
        }
    }
    flatten(&items(data, folder), "", &mut apis);

    let global = exported_auth(data, folder);
    let mut schemes = Map::new();
    let mut servers: Vec<String> = Vec::new();
    let mut paths = Map::new();
    let mut tags: Vec<String> = Vec::new();
    for (api, tag) in &apis {
        let request = &api.request;
        let (server, path) = address(&request.uri, variables);
        if let Some(server) = &server
            && !servers.contains(server)
        {
            servers.push(server.clone());
        }
        let (path, path_parameters) = templated_path(&path);
        let item = paths.entry(path.clone()).or_insert_with(|| json!({}));
        let method = request.method.as_str().to_ascii_lowercase();
        if item.get(&method).is_some() {
            continue;
        }
        if let Some(server) = &server
            && servers.first() != Some(server)
        {
            item["servers"] = json!([{"url": server}]);
        }
        let mut operation = Map::new();
        operation.insert("summary".into(), json!(api.name));
        // F3.
        if !request.description.trim().is_empty() {
            operation.insert("description".into(), json!(request.description.trim()));
        }
        if !tag.is_empty() {
            operation.insert("tags".into(), json!([tag]));
            if !tags.contains(tag) {
                tags.push(tag.clone());
            }
        }
        let parameters = parameters(request, &path_parameters);
        if !parameters.is_empty() {
            operation.insert("parameters".into(), Value::Array(parameters));
        }
        if let Some(body) = request_body(request) {
            operation.insert("requestBody".into(), body);
        }
        operation.insert("responses".into(), responses(request));
        let effective = inherit::inherited(data, &api.id).auth(&request.auth).clone();
        if effective != global {
            let security = match scheme(&effective) {
                Some((name, scheme)) => {
                    schemes.insert(name.clone(), scheme);
                    json!([{ name: scopes(&effective) }])
                }
                None => json!([]),
            };
            operation.insert("security".into(), security);
        }
        item[&method] = Value::Object(operation);
    }

    let mut document = Map::new();
    document.insert("openapi".into(), json!("3.0.3"));
    document.insert("info".into(), json!({"title": title, "version": "1.0.0"}));
    if !servers.is_empty() {
        document.insert(
            "servers".into(),
            Value::Array(servers.into_iter().map(|url| json!({ "url": url })).collect()),
        );
    }
    if !tags.is_empty() {
        document.insert(
            "tags".into(),
            Value::Array(tags.into_iter().map(|name| json!({ "name": name })).collect()),
        );
    }
    if let Some((name, scheme)) = scheme(&global) {
        schemes.insert(name.clone(), scheme);
        document.insert("security".into(), json!([{ name: scopes(&global) }]));
    }
    document.insert("paths".into(), Value::Object(paths));
    if !schemes.is_empty() {
        document.insert("components".into(), json!({ "securitySchemes": schemes }));
    }
    serde_saphyr::to_string(&Value::Object(document)).unwrap_or_default()
}

/// The server and the path of an address: its `{{NAME}}` prefix's value,
/// or its scheme and host. One that is neither is a path alone.
fn address(uri: &str, variables: &[(String, String)]) -> (Option<String>, String) {
    let uri = uri.split(['?', '#']).next().unwrap_or_default().trim();
    let rooted = |path: &str| {
        if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        }
    };
    if let (Some(name), rest) = split_env_prefix(uri) {
        let server = variables
            .iter()
            .find(|(variable, _)| variable == name)
            .map(|(_, value)| value.trim_end_matches('/').to_string())
            .unwrap_or_else(|| format!("{{{{{name}}}}}"));
        return (Some(server), rooted(rest));
    }
    match url::Url::parse(uri) {
        Ok(url) if url.has_host() => {
            let host = url.host_str().unwrap_or_default();
            let server = match url.port() {
                Some(port) => format!("{}://{host}:{port}", url.scheme()),
                None => format!("{}://{host}", url.scheme()),
            };
            (Some(server), rooted(url.path()))
        }
        _ => (None, rooted(uri)),
    }
}

/// `/users/{{id}}` as `/users/{id}`, and the names so made.
fn templated_path(path: &str) -> (String, Vec<String>) {
    let mut names = Vec::new();
    let segments: Vec<String> = path
        .split('/')
        .map(|segment| {
            let name = segment
                .strip_prefix("{{")
                .and_then(|rest| rest.strip_suffix("}}"))
                .map(str::trim)
                .filter(|name| !name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_'));
            match name {
                Some(name) => {
                    names.push(name.to_string());
                    format!("{{{name}}}")
                }
                None => segment.to_string(),
            }
        })
        .collect();
    (segments.join("/"), names)
}

/// A value as an example: text, unless it reads as a number or a flag.
fn example(value: &str) -> Value {
    match serde_json::from_str::<Value>(value.trim()) {
        Ok(found @ (Value::Number(_) | Value::Bool(_))) => found,
        _ => Value::String(value.to_string()),
    }
}

fn parameters(request: &HttpRequest, path: &[String]) -> Vec<Value> {
    let mut out: Vec<Value> = path
        .iter()
        .map(|name| json!({"name": name, "in": "path", "required": true, "schema": {"type": "string"}}))
        .collect();
    for (place, list) in [("query", &request.query), ("header", &request.headers)] {
        for kv in list.iter().filter(|kv| !kv.key.trim().is_empty()) {
            let name = kv.key.trim();
            if place == "header" && UNNAMED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                continue;
            }
            let example = example(&kv.value);
            out.push(json!({
                "name": name,
                "in": place,
                "required": false,
                "schema": schema_of(&example),
                "example": example,
            }));
        }
    }
    out
}

/// The JSON Schema of an example, as far as the example tells it.
/// F3: an operation's responses — its examples, each under its status, a
/// JSON one with a schema read off it; a plain 200 when it keeps none.
fn responses(request: &crate::model::HttpRequest) -> Value {
    let mut out = Map::new();
    for example in &request.examples {
        let status = example.status.to_string();
        let entry = out.entry(status.clone()).or_insert_with(|| {
            json!({"description": if example.name.is_empty() { crate::response::status_text(example.status).to_string() } else { example.name.clone() }})
        });
        let media = example
            .content_type()
            .map(|value| value.split(';').next().unwrap_or_default().trim().to_string())
            .filter(|media| !media.is_empty())
            .unwrap_or_else(|| "text/plain".into());
        let content = entry
            .as_object_mut()
            .map(|entry| entry.entry("content").or_insert_with(|| json!({})));
        let Some(Value::Object(content)) = content else {
            continue;
        };
        if content.contains_key(&media) {
            continue;
        }
        let value = serde_json::from_str::<Value>(&example.body)
            .ok()
            .filter(|_| media.contains("json"))
            .unwrap_or_else(|| json!(example.body));
        let mut body = Map::new();
        if value.is_object() || value.is_array() {
            body.insert("schema".into(), schema_of(&value));
        }
        body.insert("example".into(), value);
        content.insert(media, Value::Object(body));
    }
    if out.is_empty() {
        out.insert("200".into(), json!({"description": "OK"}));
    }
    Value::Object(out)
}

fn schema_of(value: &Value) -> Value {
    match value {
        Value::Null => json!({"nullable": true}),
        Value::Bool(_) => json!({"type": "boolean"}),
        Value::Number(number) if number.is_i64() || number.is_u64() => json!({"type": "integer"}),
        Value::Number(_) => json!({"type": "number"}),
        Value::String(_) => json!({"type": "string"}),
        Value::Array(items) => json!({
            "type": "array",
            "items": items.first().map(schema_of).unwrap_or_else(|| json!({})),
        }),
        Value::Object(fields) => {
            let properties: Map<String, Value> = fields
                .iter()
                .map(|(name, value)| (name.clone(), schema_of(value)))
                .collect();
            json!({"type": "object", "properties": properties})
        }
    }
}

fn request_body(request: &HttpRequest) -> Option<Value> {
    if !request.has_body_tab() {
        return None;
    }
    let content = match request.content_type {
        _ if request.content_type != ContentType::GraphQl
            && !request.content_type.is_key_value()
            && request.body.trim().is_empty() =>
        {
            return None;
        }
        ContentType::Json => match serde_json::from_str::<Value>(&request.body) {
            Ok(example) => json!({"application/json": {"schema": schema_of(&example), "example": example}}),
            // Templates in it: as written.
            Err(_) => json!({"application/json": {"schema": {"type": "object"}, "example": request.body}}),
        },
        ContentType::Xml => json!({"application/xml": {"schema": {"type": "string"}, "example": request.body}}),
        ContentType::Plain => json!({"text/plain": {"schema": {"type": "string"}, "example": request.body}}),
        ContentType::GraphQl => {
            let graphql = GraphQlBody::read(&request.body);
            let variables = serde_json::from_str::<Value>(&graphql.variables).unwrap_or_else(|_| json!({}));
            let example = json!({"query": graphql.query, "variables": variables});
            json!({"application/json": {"schema": schema_of(&example), "example": example}})
        }
        ContentType::Form | ContentType::Multipart => {
            let multipart = request.content_type == ContentType::Multipart;
            let mut properties = Map::new();
            for kv in form_fields(&request.body).iter().filter(|kv| !kv.key.trim().is_empty()) {
                let property = match kv.value.strip_prefix("file://") {
                    Some(_) if multipart => json!({"type": "string", "format": "binary"}),
                    _ => {
                        let example = example(&kv.value);
                        let mut schema = schema_of(&example);
                        schema["example"] = example;
                        schema
                    }
                };
                properties.insert(kv.key.trim().to_string(), property);
            }
            let kind = if multipart {
                "multipart/form-data"
            } else {
                "application/x-www-form-urlencoded"
            };
            json!({ kind: {"schema": {"type": "object", "properties": properties}} })
        }
    };
    Some(json!({ "content": content }))
}

/// The scopes a requirement asks for: OAuth's (C4), else none.
fn scopes(auth: &Auth) -> Vec<&str> {
    match auth {
        Auth::OAuth2(oauth) => oauth.scope.split_whitespace().collect(),
        _ => Vec::new(),
    }
}

/// A security scheme for how a request signs in, and its name; none for
/// none.
fn scheme(auth: &Auth) -> Option<(String, Value)> {
    Some(match auth {
        Auth::Inherit | Auth::None => return None,
        Auth::Basic { .. } => ("basicAuth".into(), json!({"type": "http", "scheme": "basic"})),
        Auth::Bearer { .. } => ("bearerAuth".into(), json!({"type": "http", "scheme": "bearer"})),
        Auth::Digest { .. } => ("digestAuth".into(), json!({"type": "http", "scheme": "digest"})),
        // C4: the flow, where its token is had and the scopes it asks.
        Auth::OAuth2(oauth) => {
            let flow = match oauth.grant {
                Grant::ClientCredentials => "clientCredentials",
                Grant::Password => "password",
            };
            let scopes: Map<String, Value> = oauth
                .scope
                .split_whitespace()
                .map(|scope| (scope.to_string(), json!("")))
                .collect();
            (
                "oauth2".into(),
                json!({"type": "oauth2", "flows": {flow: {"tokenUrl": oauth.token_url.trim(), "scopes": scopes}}}),
            )
        }
        Auth::ApiKey { name, place, .. } => {
            let place = match place {
                KeyPlace::Header => "header",
                KeyPlace::Query => "query",
            };
            let safe: String = name
                .trim()
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
                .collect();
            (
                format!("apiKey_{place}_{safe}"),
                json!({"type": "apiKey", "in": place, "name": name.trim()}),
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ApiCategory, Folder, FolderSettings, HttpMethod, KvParam, ProjectSettings};
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

    fn kv(key: &str, value: &str) -> KvParam {
        KvParam {
            key: key.into(),
            value: value.into(),
            enabled: true,
        }
    }

    fn shop() -> TreeData {
        TreeData {
            folders: vec![Folder {
                id: ItemId::new("f1"),
                collection: CollectionId::new("c"),
                name: "users".into(),
                children: vec![ItemId::new("a1"), ItemId::new("a2")],
                settings: FolderSettings::default(),
                created_at: Default::default(),
                updated_at: Default::default(),
            }],
            apis: vec![
                api(
                    "a1",
                    "get user",
                    HttpRequest {
                        method: HttpMethod::Get,
                        uri: "{{HOST}}/users/{{id}}".into(),
                        query: vec![kv("expand", "true"), kv("page", "2")],
                        headers: vec![kv("X-Trace", "t"), kv("Accept", "application/json")],
                        ..Default::default()
                    },
                ),
                api(
                    "a2",
                    "create user",
                    HttpRequest {
                        method: HttpMethod::Post,
                        uri: "{{HOST}}/users".into(),
                        content_type: ContentType::Json,
                        body: r#"{"name": "ann", "age": 3, "tags": ["a"]}"#.into(),
                        auth: Auth::ApiKey {
                            name: "X-Key".into(),
                            value: "secret".into(),
                            place: KeyPlace::Header,
                        },
                        ..Default::default()
                    },
                ),
                api(
                    "a3",
                    "health",
                    HttpRequest {
                        method: HttpMethod::Get,
                        uri: "https://status.test:8443/health".into(),
                        auth: Auth::None,
                        ..Default::default()
                    },
                ),
            ],
            top_order: vec![ItemId::new("f1"), ItemId::new("a3")],
            settings: ProjectSettings {
                auth: Auth::Bearer {
                    token: "{{token}}".into(),
                },
                ..Default::default()
            },
        }
    }

    fn document() -> Value {
        let yaml = openapi(
            &shop(),
            None,
            "Shop",
            &[("HOST".into(), "https://shop.test/api/".into())],
        );
        assert!(
            !yaml.contains("secret") && !yaml.contains("{{token}}"),
            "no values of auth: {yaml}"
        );
        serde_saphyr::from_str(&yaml).expect("YAML")
    }

    #[test]
    fn servers_paths_and_parameters() {
        let doc = document();
        assert_eq!(doc["openapi"], "3.0.3");
        assert_eq!(
            doc["servers"],
            json!([{"url": "https://shop.test/api"}, {"url": "https://status.test:8443"}])
        );
        let get = &doc["paths"]["/users/{id}"]["get"];
        assert_eq!(get["summary"], "get user");
        assert_eq!(get["tags"], json!(["users"]));
        let parameters = get["parameters"].as_array().expect("parameters");
        let named: Vec<(&str, &str)> = parameters
            .iter()
            .map(|parameter| {
                (
                    parameter["in"].as_str().unwrap_or(""),
                    parameter["name"].as_str().unwrap_or(""),
                )
            })
            .collect();
        assert_eq!(
            named,
            [
                ("path", "id"),
                ("query", "expand"),
                ("query", "page"),
                ("header", "X-Trace")
            ]
        );
        assert_eq!(parameters[1]["example"], json!(true));
        assert_eq!(parameters[2]["schema"], json!({"type": "integer"}));
        assert_eq!(
            doc["paths"]["/health"]["servers"],
            json!([{"url": "https://status.test:8443"}]),
            "another server on its path"
        );
        assert!(doc["paths"]["/users/{id}"]["get"]["responses"].get("200").is_some());
    }

    #[test]
    fn bodies_carry_an_example_and_its_schema() {
        let doc = document();
        let content = &doc["paths"]["/users"]["post"]["requestBody"]["content"]["application/json"];
        assert_eq!(content["example"], json!({"name": "ann", "age": 3, "tags": ["a"]}));
        assert_eq!(
            content["schema"],
            json!({"type": "object", "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"},
                "tags": {"type": "array", "items": {"type": "string"}}
            }})
        );
    }

    #[test]
    fn auth_is_security_without_values() {
        let doc = document();
        assert_eq!(doc["security"], json!([{"bearerAuth": []}]));
        assert_eq!(
            doc["components"]["securitySchemes"],
            json!({
                "apiKey_header_X_Key": {"type": "apiKey", "in": "header", "name": "X-Key"},
                "bearerAuth": {"type": "http", "scheme": "bearer"}
            })
        );
        assert_eq!(
            doc["paths"]["/users"]["post"]["security"],
            json!([{"apiKey_header_X_Key": []}])
        );
        assert_eq!(doc["paths"]["/health"]["get"]["security"], json!([]), "none");
        assert!(
            doc["paths"]["/users/{id}"]["get"].get("security").is_none(),
            "the project's"
        );
    }

    /// C4: OAuth is its flow, its token URL and the scopes asked; its
    /// client is the reader's to fill again.
    #[test]
    fn oauth_is_its_flow_and_comes_back() {
        let oauth = Auth::from(crate::oauth::OAuth2 {
            grant: Grant::Password,
            token_url: "https://id.test/token".into(),
            client_id: "app".into(),
            client_secret: "s".into(),
            username: "me".into(),
            password: "pw".into(),
            scope: "read write".into(),
            ..Default::default()
        });
        let mut data = shop();
        data.settings.auth = oauth;
        let yaml = openapi(&data, None, "Shop", &[]);
        let doc: Value = serde_saphyr::from_str(&yaml).expect("YAML");
        assert_eq!(doc["security"], json!([{"oauth2": ["read", "write"]}]));
        assert_eq!(
            doc["components"]["securitySchemes"]["oauth2"],
            json!({"type": "oauth2", "flows": {"password": {"tokenUrl": "https://id.test/token", "scopes": {"read": "", "write": ""}}}})
        );
        let bundle = crate::import::parse(&yaml).expect("a document");
        assert_eq!(
            bundle.auth,
            Auth::from(crate::oauth::OAuth2 {
                grant: Grant::Password,
                token_url: "https://id.test/token".into(),
                scope: "read write".into(),
                ..Default::default()
            })
        );
    }

    /// This app's own import reads the document back.
    #[test]
    fn it_imports_back() {
        let yaml = openapi(&shop(), None, "Shop", &[("HOST".into(), "https://shop.test".into())]);
        let bundle = crate::import::parse(&yaml).expect("a document");
        assert_eq!(bundle.counts(), (3, 1));
        assert_eq!(bundle.auth, Auth::Bearer { token: String::new() });
    }
}
