//! What the bridge's MCP tools (`cyberapi-bridge/src/mcp.rs`) show an AI
//! assistant of a project, the rules of it: the tree as paths, an API found
//! by its id, its path or its name, what reads like a secret masked, and
//! long text cut to fit a model's context. H1: what a token allowed to write
//! changes of a request ([`written`]), and the folder it names.

use crate::ItemId;
use crate::model::{ApiSetting, ContentType, HttpMethod, HttpRequest, KvParam};
use crate::tree::{Tree, TreeData};
use serde_json::Value;

/// What a masked value reads as.
pub const MASK: &str = "***";
/// Where a body in a result is cut, in characters.
pub const MAX_TEXT_CHARS: usize = 8 * 1024;
/// Where a whole result is cut, in bytes.
pub const MAX_RESULT_BYTES: usize = 200 * 1024;

/// Whether a name — a header's, a variable's, a form field's or a JSON
/// key's — reads like it holds a secret: `Authorization`, `X-Api-Key`,
/// `access_token`, `PASSWORD`, `Cookie`.
pub fn secret_name(name: &str) -> bool {
    let name: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    const WITHIN: [&str; 10] = [
        "token",
        "secret",
        "password",
        "passwd",
        "credential",
        "cookie",
        "session",
        "authorization",
        "signature",
        "privatekey",
    ];
    const ENDING: [&str; 4] = ["key", "pwd", "pass", "auth"];
    WITHIN.iter().any(|word| name.contains(word)) || ENDING.iter().any(|word| name.ends_with(word))
}

/// A value made with a template — `{{TOKEN}}`, `Bearer {{value(token)}}` —
/// names where the secret comes from rather than holding it, and stays: the
/// variable it reads is masked where variables are shown.
fn names_a_template(value: &str) -> bool {
    value.find("{{").is_some_and(|open| value[open..].contains("}}"))
}

/// `value`, or [`MASK`] when `name` reads like a secret and `value` holds
/// one.
pub fn masked(name: &str, value: &str) -> String {
    if secret_name(name) && !value.is_empty() && !names_a_template(value) {
        MASK.to_string()
    } else {
        value.to_string()
    }
}

/// A key / value list with its secrets masked.
pub fn masked_params(params: &[KvParam]) -> Vec<KvParam> {
    params
        .iter()
        .map(|param| KvParam {
            value: masked(&param.key, &param.value),
            ..param.clone()
        })
        .collect()
}

/// JSON with the values of keys that read like secrets masked, at any
/// depth. An object or an array under such a key is masked whole.
pub fn mask_json(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, field) in fields.iter_mut() {
                let template = field.as_str().is_some_and(names_a_template);
                if secret_name(key) && !field.is_null() && !template {
                    *field = Value::String(MASK.into());
                } else {
                    mask_json(field);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(mask_json),
        _ => {}
    }
}

/// A body as text with what reads like a secret masked: JSON by its keys,
/// `a=1&password=2` by its fields; any other text as it is.
pub fn masked_body(body: &str) -> String {
    let trimmed = body.trim_start();
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && let Ok(mut json) = serde_json::from_str::<Value>(body)
    {
        mask_json(&mut json);
        return crate::format::json(&json);
    }
    let form = !body.is_empty() && !body.contains(char::is_whitespace) && body.contains('=');
    if form {
        return body
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((key, value)) => format!("{key}={}", masked(key, value)),
                None => pair.to_string(),
            })
            .collect::<Vec<_>>()
            .join("&");
    }
    body.to_string()
}

/// `text` cut after `max` characters, with a note of how much there was.
pub fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_string(),
        Some((at, _)) => format!("{}… [cut: {} bytes in all]", &text[..at], text.len()),
    }
}

/// One line of a project's tree, as `list_apis` gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub id: ItemId,
    pub folder: bool,
    /// The names from the top, `/`-separated: `users/login`.
    pub path: String,
    /// An API's; `None` for a folder.
    pub method: Option<HttpMethod>,
    pub uri: Option<String>,
}

/// The tree in display order, depth first, each item with its path.
pub fn listing(data: &TreeData) -> Vec<Listed> {
    let tree = Tree::build(data);
    let mut out = Vec::new();
    let mut stack: Vec<(ItemId, String)> = tree
        .roots()
        .map(|node| (node.id().clone(), String::new()))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    while let Some((id, above)) = stack.pop() {
        let Some(node) = tree.get(&id) else { continue };
        let path = if above.is_empty() {
            node.name().to_string()
        } else {
            format!("{above}/{}", node.name())
        };
        let api = data.apis.iter().find(|api| &api.id == node.id());
        out.push(Listed {
            id: node.id().clone(),
            folder: node.is_folder(),
            path: path.clone(),
            method: api.map(|api| api.request.method),
            uri: api.map(|api| api.request.uri.clone()),
        });
        for child in node.children().iter().rev() {
            stack.push((child.clone(), path.clone()));
        }
    }
    out
}

/// The API `spec` names: its id, its path (`users/login`), or a name only
/// one API has. Otherwise why not, for the model to read.
pub fn find_api<'a>(data: &'a TreeData, spec: &str) -> Result<&'a ApiSetting, String> {
    let spec = spec.trim();
    if let Some(api) = data.apis.iter().find(|api| api.id.as_str() == spec) {
        return Ok(api);
    }
    let ambiguous = |count: usize| format!("{count} APIs are {spec:?}: give one's id (list_apis names them)");
    let by_path: Vec<ItemId> = listing(data)
        .into_iter()
        .filter(|line| !line.folder && line.path == spec)
        .map(|line| line.id)
        .collect();
    match by_path.as_slice() {
        [id] => {
            if let Some(api) = data.apis.iter().find(|api| &api.id == id) {
                return Ok(api);
            }
        }
        [] => {}
        many => return Err(ambiguous(many.len())),
    }
    let named: Vec<&ApiSetting> = data.apis.iter().filter(|api| api.name == spec).collect();
    match named.as_slice() {
        [api] => Ok(api),
        [] => Err(format!(
            "no API is {spec:?}: list_apis names them, with their paths and ids"
        )),
        many => Err(ambiguous(many.len())),
    }
}

/// H1: the folder `spec` names — its id or its path (`users/admin`) — or
/// why not.
pub fn find_folder(data: &TreeData, spec: &str) -> Result<ItemId, String> {
    let spec = spec.trim().trim_matches('/');
    if data.folders.iter().any(|folder| folder.id.as_str() == spec) {
        return Ok(ItemId::new(spec));
    }
    let found: Vec<ItemId> = listing(data)
        .into_iter()
        .filter(|line| line.folder && line.path == spec)
        .map(|line| line.id)
        .collect();
    match found.as_slice() {
        [id] => Ok(id.clone()),
        [] => Err(format!("no folder is {spec:?}: list_apis names them")),
        many => Err(format!("{} folders are {spec:?}: give one's id", many.len())),
    }
}

/// `[{name, value, enabled?}]` or `{name: value}`, as a list.
fn params(value: &Value, what: &str) -> Result<Vec<KvParam>, String> {
    let text = |value: &Value| match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    match value {
        Value::Object(map) => Ok(map
            .iter()
            .map(|(key, value)| KvParam {
                key: key.clone(),
                value: text(value),
                enabled: true,
            })
            .collect()),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                let name = item.get("name").or_else(|| item.get("key")).and_then(Value::as_str);
                let Some(name) = name.filter(|name| !name.trim().is_empty()) else {
                    return Err(format!("each of {what} has a name"));
                };
                Ok(KvParam {
                    key: name.trim().to_string(),
                    value: item.get("value").map(text).unwrap_or_default(),
                    enabled: item.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                })
            })
            .collect(),
        _ => Err(format!(
            "{what} is an object of names and values, or a list of {{name, value}}"
        )),
    }
}

/// H1: the content types an assistant names, as it names them.
const CONTENT_TYPES: [(&str, ContentType); 6] = [
    ("json", ContentType::Json),
    ("form", ContentType::Form),
    ("multipart", ContentType::Multipart),
    ("xml", ContentType::Xml),
    ("text", ContentType::Plain),
    ("graphql", ContentType::GraphQl),
];

/// H1: `request` with what `arguments` gives of it in place: `method`,
/// `url` (its query moves to the list, as the URL field does), `query` and
/// `headers` (each replacing the list), `contentType`, `body` (text; a
/// form's an object of fields) and `description`. What it does not give
/// stays. A gRPC call or a WebSocket is not written here.
pub fn written(request: &HttpRequest, arguments: &Value) -> Result<HttpRequest, String> {
    let mut out = request.clone();
    if out.method == HttpMethod::Grpc {
        return Err("a gRPC call is not written by an assistant".into());
    }
    if out.method == HttpMethod::WebSocket {
        return Err("a WebSocket is not written by an assistant".into());
    }
    let given = |name: &str| arguments.get(name).filter(|value| !value.is_null());
    if let Some(method) = given("method") {
        let method = method.as_str().ok_or("method is text: GET, POST…")?;
        let parsed = HttpMethod::parse_lenient(method);
        if matches!(parsed, HttpMethod::Grpc | HttpMethod::WebSocket)
            || !parsed.as_str().eq_ignore_ascii_case(method.trim())
        {
            return Err(format!(
                "{method:?} is no method: GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS"
            ));
        }
        out.method = parsed;
    }
    if let Some(url) = given("url") {
        let url = url.as_str().ok_or("url is text")?;
        let (uri, query) = crate::import::split_url(url.trim());
        out.uri = uri;
        if !query.is_empty() {
            out.query = query;
        }
    }
    if let Some(query) = given("query") {
        out.query = params(query, "query")?;
    }
    if let Some(headers) = given("headers") {
        out.headers = params(headers, "headers")?;
    }
    if let Some(kind) = given("contentType") {
        let kind = kind.as_str().unwrap_or_default().trim().to_ascii_lowercase();
        out.content_type = CONTENT_TYPES
            .iter()
            .find(|(name, _)| *name == kind)
            .map(|(_, content_type)| *content_type)
            .ok_or_else(|| {
                let names: Vec<&str> = CONTENT_TYPES.iter().map(|(name, _)| *name).collect();
                format!("contentType is one of {}", names.join(", "))
            })?;
    }
    if let Some(body) = given("body") {
        out.body = match (out.content_type.is_key_value(), body) {
            (true, fields @ (Value::Object(_) | Value::Array(_))) => {
                serde_json::to_string(&params(fields, "body")?).unwrap_or_default()
            }
            (_, Value::String(text)) => text.clone(),
            (false, json @ (Value::Object(_) | Value::Array(_))) => crate::format::json(json),
            _ => return Err("body is text, or a form's fields".into()),
        };
    }
    if let Some(description) = given("description") {
        out.description = description.as_str().ok_or("description is text")?.to_string();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Folder, HttpRequest};
    use crate::{CollectionId, Timestamp};
    use serde_json::json;

    #[test]
    fn names_that_hold_secrets_are_told() {
        for name in [
            "Authorization",
            "X-Api-Key",
            "api_key",
            "APP_KEY",
            "access_token",
            "PASSWORD",
            "Cookie",
            "Set-Cookie",
            "client_secret",
            "X-Auth",
            "pwd",
        ] {
            assert!(secret_name(name), "{name}");
        }
        for name in ["Content-Type", "HOST", "user", "page", "Accept", "author_name"] {
            assert!(!secret_name(name), "{name}");
        }
    }

    #[test]
    fn a_secret_is_masked_and_a_template_naming_one_is_not() {
        assert_eq!(masked("Authorization", "Bearer abc"), MASK);
        assert_eq!(masked("Authorization", "Bearer {{token}}"), "Bearer {{token}}");
        assert_eq!(masked("X-Token", "{{value(token)}}"), "{{value(token)}}");
        assert_eq!(masked("X-Token", "}}abc{{"), MASK, "not a template");
        assert_eq!(masked("token", ""), "");
        assert_eq!(masked("host", "api.example.test"), "api.example.test");
    }

    #[test]
    fn json_and_form_bodies_lose_their_secrets() {
        let mut json = json!({"user": "ann", "password": "p", "nested": [{"token": 1, "id": 2}], "auth": {"a": 1}});
        mask_json(&mut json);
        assert_eq!(
            json,
            json!({"user": "ann", "password": MASK, "nested": [{"token": MASK, "id": 2}], "auth": MASK})
        );
        assert!(masked_body(r#"{"password":"p","name":"x"}"#).contains(r#""password": "***""#));
        assert_eq!(masked_body("user=ann&password=p"), "user=ann&password=***");
        assert_eq!(masked_body("plain text = with spaces"), "plain text = with spaces");
    }

    #[test]
    fn long_text_is_cut_with_a_note() {
        assert_eq!(cut("abc", 5), "abc");
        assert_eq!(cut("中文字符", 2), "中文… [cut: 12 bytes in all]");
    }

    fn data() -> TreeData {
        let at = Timestamp::from_unix(0);
        let api = |id: &str, name: &str, uri: &str| ApiSetting {
            id: ItemId::new(id),
            collection: CollectionId::new("c"),
            name: name.into(),
            category: Default::default(),
            request: HttpRequest {
                uri: uri.into(),
                ..Default::default()
            },
            created_at: at,
            updated_at: at,
        };
        TreeData {
            folders: vec![Folder {
                id: ItemId::new("f"),
                collection: CollectionId::new("c"),
                name: "users".into(),
                children: vec![ItemId::new("a2"), ItemId::new("a3")],
                settings: Default::default(),
                created_at: at,
                updated_at: at,
            }],
            apis: vec![
                api("a1", "health", "{{HOST}}/health"),
                api("a2", "login", "{{HOST}}/login"),
                api("a3", "login", "{{HOST}}/users/login2"),
            ],
            top_order: vec![ItemId::new("f"), ItemId::new("a1")],
            ..Default::default()
        }
    }

    #[test]
    fn the_tree_lists_as_paths_in_order() {
        let paths: Vec<String> = listing(&data()).into_iter().map(|line| line.path).collect();
        assert_eq!(paths, ["users", "users/login", "users/login", "health"]);
    }

    #[test]
    fn an_api_is_found_by_id_path_or_a_name_only_it_has() {
        let data = data();
        assert_eq!(find_api(&data, "a3").map(|api| api.id.as_str()), Ok("a3"));
        assert_eq!(
            find_api(&data, "health").map(|api| api.id.as_str()),
            Ok("a1"),
            "its path"
        );
        assert!(find_api(&data, "users/login").is_err_and(|why| why.contains("2 APIs")));
        assert!(find_api(&data, "login").is_err_and(|why| why.contains("2 APIs")));
        assert!(find_api(&data, "nope").is_err_and(|why| why.contains("list_apis")));
    }

    /// H1: a folder by its id or its path.
    #[test]
    fn a_folder_is_found_by_id_or_path() {
        let data = data();
        assert_eq!(find_folder(&data, "f"), Ok(ItemId::new("f")));
        assert_eq!(find_folder(&data, "/users/"), Ok(ItemId::new("f")));
        assert!(find_folder(&data, "health").is_err(), "an API is no folder");
        assert!(find_folder(&data, "admins").is_err_and(|why| why.contains("list_apis")));
    }

    /// H1: what an assistant gives is written; what it does not, stays.
    #[test]
    fn an_assistant_writes_what_it_gives() {
        let kept = HttpRequest {
            method: HttpMethod::Post,
            uri: "{{HOST}}/login".into(),
            headers: vec![KvParam {
                key: "X-A".into(),
                value: "1".into(),
                enabled: true,
            }],
            body: "{}".into(),
            ..Default::default()
        };
        let changed = written(
            &kept,
            &json!({"url": "{{HOST}}/users?page=2", "headers": {"Accept": "application/json"}, "description": "Lists users."}),
        )
        .expect("written");
        assert_eq!(changed.method, HttpMethod::Post, "not given, kept");
        assert_eq!(changed.uri, "{{HOST}}/users");
        assert_eq!(changed.query[0].key, "page");
        assert_eq!(changed.headers.len(), 1);
        assert_eq!(changed.headers[0].key, "Accept");
        assert_eq!(changed.body, "{}");
        assert_eq!(changed.description, "Lists users.");
        let form = written_form();
        assert_eq!(form.content_type, ContentType::Form);
        assert_eq!(crate::send::form_fields(&form.body).len(), 2);
        let json_body = written(&kept, &json!({"body": {"a": 1}})).expect("a JSON body");
        assert_eq!(json_body.body, "{\n    \"a\": 1\n}");
        assert!(written(&kept, &json!({"method": "FETCH"})).is_err());
        assert!(written(&kept, &json!({"contentType": "yaml"})).is_err_and(|why| why.contains("json")));
        assert!(written(&kept, &json!({"headers": [{"value": "x"}]})).is_err());
        let grpc = HttpRequest {
            method: HttpMethod::Grpc,
            ..Default::default()
        };
        assert!(written(&grpc, &json!({})).is_err());
    }

    fn written_form() -> HttpRequest {
        written(
            &HttpRequest::default(),
            &json!({"method": "post", "contentType": "form", "body": [{"name": "user", "value": "me"}, {"name": "pass", "value": "p", "enabled": false}]}),
        )
        .expect("a form")
    }
}
