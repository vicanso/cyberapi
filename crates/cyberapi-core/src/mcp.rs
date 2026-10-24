//! What the bridge's MCP tools (`cyberapi-bridge/src/mcp.rs`) show an AI
//! assistant of a project, the rules of it: the tree as paths, an API found
//! by its id, its path or its name, what reads like a secret masked, and
//! long text cut to fit a model's context.

use crate::ItemId;
use crate::model::{ApiSetting, HttpMethod, KvParam};
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
                created_at: at,
                updated_at: at,
            }],
            apis: vec![
                api("a1", "health", "{{HOST}}/health"),
                api("a2", "login", "{{HOST}}/login"),
                api("a3", "login", "{{HOST}}/users/login2"),
            ],
            top_order: vec![ItemId::new("f"), ItemId::new("a1")],
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
}
