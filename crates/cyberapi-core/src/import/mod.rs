//! Importing APIs (M10): text in one of five formats becomes an
//! [`ImportBundle`] — a forest of folders and APIs, and environment
//! variables — which a store writes with ids of its own.
//!
//! The formats are told apart as before (F10.2): text starting with `curl`
//! is a cURL command; JSON with `swagger` is Swagger 2.0, with `openapi`
//! OpenAPI 3, with `item` a Postman collection, with `resources` an Insomnia
//! export; any other JSON is CyberAPI's own export.
//!
//! The bundle carries no ids: the old app kept the ids of its own format,
//! so importing a file twice clashed, and retried APIs left their folders
//! pointing at the old ids (#7). A store gives every item a new id.

mod curl;
mod insomnia;
mod openapi;
mod own;
mod postman;

pub use own::{api_config, export, parse_own};

use crate::ItemId;
use crate::model::{ContentType, HttpMethod, HttpRequest, KvParam};
use serde::{Deserialize, Serialize};
use snafu::Snafu;

/// One folder or API to add, with what it contains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ImportItem {
    Folder { name: String, children: Vec<ImportItem> },
    Api { name: String, request: HttpRequest },
}

/// What an import adds to a project.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportBundle {
    /// The top level of what is imported, in order.
    pub items: Vec<ImportItem>,
    /// Environment variables, name and value.
    pub environment: Vec<(String, String)>,
}

/// Ids given to a bundle's items, depth first, and the records to write.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assigned {
    /// Each folder with its children's new ids, parents before children.
    pub folders: Vec<(ItemId, String, Vec<ItemId>)>,
    pub apis: Vec<(ItemId, String, HttpRequest)>,
    /// The new ids of the bundle's top level, in order.
    pub top: Vec<ItemId>,
}

/// Gives every item a new id from `new_id` (#7).
pub fn assign_ids(items: &[ImportItem], new_id: &mut dyn FnMut() -> ItemId) -> Assigned {
    fn walk(items: &[ImportItem], new_id: &mut dyn FnMut() -> ItemId, out: &mut Assigned) -> Vec<ItemId> {
        let mut ids = Vec::new();
        for item in items {
            let id = new_id();
            match item {
                ImportItem::Api { name, request } => out.apis.push((id.clone(), name.clone(), request.clone())),
                ImportItem::Folder { name, children } => {
                    let at = out.folders.len();
                    out.folders.push((id.clone(), name.clone(), Vec::new()));
                    let children = walk(children, new_id, out);
                    out.folders[at].2 = children;
                }
            }
            ids.push(id);
        }
        ids
    }
    let mut out = Assigned::default();
    out.top = walk(items, new_id, &mut out);
    out
}

impl ImportBundle {
    /// How many APIs and folders it holds, at every depth.
    pub fn counts(&self) -> (usize, usize) {
        fn walk(items: &[ImportItem], apis: &mut usize, folders: &mut usize) {
            for item in items {
                match item {
                    ImportItem::Api { .. } => *apis += 1,
                    ImportItem::Folder { children, .. } => {
                        *folders += 1;
                        walk(children, apis, folders);
                    }
                }
            }
        }
        let (mut apis, mut folders) = (0, 0);
        walk(&self.items, &mut apis, &mut folders);
        (apis, folders)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Curl,
    Postman,
    Insomnia,
    Swagger,
    OpenApi,
    CyberApi,
}

#[derive(Debug, Snafu, PartialEq, Eq)]
pub enum ImportError {
    #[snafu(display("not a cURL command or JSON: {message}"))]
    Unreadable { message: String },
    #[snafu(display("the cURL command has no URL"))]
    NoUrl,
    #[snafu(display("nothing to import"))]
    Empty,
}

/// F10.2.
pub fn detect(text: &str) -> Result<(Format, Option<serde_json::Value>), ImportError> {
    let trimmed = text.trim_start();
    if trimmed.len() >= 4 && trimmed[..4].eq_ignore_ascii_case("curl") {
        return Ok((Format::Curl, None));
    }
    let json: serde_json::Value = serde_json::from_str(text).map_err(|error| ImportError::Unreadable {
        message: error.to_string(),
    })?;
    let format = match &json {
        serde_json::Value::Object(map) if map.contains_key("swagger") => Format::Swagger,
        serde_json::Value::Object(map) if map.contains_key("openapi") => Format::OpenApi,
        serde_json::Value::Object(map) if map.contains_key("item") => Format::Postman,
        serde_json::Value::Object(map) if map.contains_key("resources") => Format::Insomnia,
        _ => Format::CyberApi,
    };
    Ok((format, Some(json)))
}

/// F10.1–F10.3: whatever was pasted or picked.
pub fn parse(text: &str) -> Result<ImportBundle, ImportError> {
    let (format, json) = detect(text)?;
    let bundle = match (format, json) {
        (Format::Curl, _) => curl::parse(text)?,
        (Format::Postman, Some(json)) => postman::parse(&json),
        (Format::Insomnia, Some(json)) => insomnia::parse(&json),
        (Format::Swagger | Format::OpenApi, Some(json)) => openapi::parse(&json),
        (_, Some(json)) => own::parse_value(&json),
        (_, None) => ImportBundle::default(),
    };
    if bundle.items.is_empty() && bundle.environment.is_empty() {
        return EmptySnafu.fail();
    }
    Ok(bundle)
}

/// The request of a cURL command pasted where a URI goes: the editor fills
/// itself in from it. The body stays as pasted (#43): indenting it would
/// change the bytes that are sent. `None` for anything else.
pub fn request_from_curl(text: &str) -> Option<HttpRequest> {
    let text = text.trim_start();
    let command = text.get(..4).is_some_and(|start| start.eq_ignore_ascii_case("curl"))
        && text[4..].starts_with(char::is_whitespace);
    if !command {
        return None;
    }
    let ImportItem::Api { request, .. } = curl::parse(text).ok()?.items.into_iter().next()? else {
        return None;
    };
    Some(request)
}

/// A request's URL with its query moved to the query list, as every importer
/// does (F10.3). A URL that does not parse — `{{HOST}}/a?b=1` — is split by
/// hand.
fn split_url(url: &str) -> (String, Vec<KvParam>) {
    let Some((base, query)) = url.split_once('?') else {
        return (url.to_string(), Vec::new());
    };
    let pairs = url::form_urlencoded::parse(query.as_bytes())
        .map(|(key, value)| KvParam {
            key: key.into_owned(),
            value: value.into_owned(),
            enabled: true,
        })
        .collect();
    (base.to_string(), pairs)
}

/// A body's type when nothing says it: JSON when it looks like JSON.
fn guess_content_type(body: &str) -> ContentType {
    let trimmed = body.trim();
    if (trimmed.starts_with('{') && trimmed.ends_with('}')) || (trimmed.starts_with('[') && trimmed.ends_with(']')) {
        ContentType::Json
    } else {
        ContentType::Plain
    }
}

/// A `Content-Type` value as the editor's type, if it is one of them.
fn content_type_of(value: &str) -> Option<ContentType> {
    let mime = value.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    match mime.as_str() {
        "application/json" => Some(ContentType::Json),
        "application/x-www-form-urlencoded" => Some(ContentType::Form),
        "multipart/form-data" => Some(ContentType::Multipart),
        "application/xml" | "text/xml" => Some(ContentType::Xml),
        "text/plain" => Some(ContentType::Plain),
        _ => None,
    }
}

/// An API from the parts every format has.
fn api(name: &str, method: &str, url: &str) -> (String, HttpRequest) {
    let (uri, query) = split_url(url.trim());
    (
        if name.trim().is_empty() {
            uri.clone()
        } else {
            name.trim().to_string()
        },
        HttpRequest {
            method: HttpMethod::parse_lenient(method),
            uri,
            query,
            ..Default::default()
        },
    )
}

fn kv(key: &str, value: &str, enabled: bool) -> KvParam {
    KvParam {
        key: key.to_string(),
        value: value.to_string(),
        enabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn a_pasted_curl_command_is_a_request() {
        let request = request_from_curl(" curl -X PUT 'https://a.test/x?y=1' -H 'X-A: 1'").expect("a request");
        assert_eq!(request.method, HttpMethod::Put);
        assert_eq!(request.uri, "https://a.test/x");
        assert_eq!(request.query, [kv("y", "1", true)]);
        assert_eq!(request.headers, [kv("X-A", "1", true)]);
        // A JSON body stays as pasted, so sending does not change its bytes.
        let request = request_from_curl(r#"curl https://a.test -H 'Content-Type: application/json' -d '{"a":[1]}'"#)
            .expect("a request");
        assert_eq!(request.body, r#"{"a":[1]}"#);
        // A URI, a word that only starts like one, a command without a URL.
        assert_eq!(request_from_curl("https://a.test/curl"), None);
        assert_eq!(request_from_curl("curly.test/a"), None);
        assert_eq!(request_from_curl("curl -v"), None);
        assert_eq!(request_from_curl("网厅"), None);
    }

    #[test]
    fn formats_are_told_apart() {
        let format = |text: &str| detect(text).map(|(format, _)| format);
        assert_eq!(format("  curl https://a.test"), Ok(Format::Curl));
        assert_eq!(format(r#"{"swagger": "2.0"}"#), Ok(Format::Swagger));
        assert_eq!(format(r#"{"openapi": "3.0.1"}"#), Ok(Format::OpenApi));
        assert_eq!(format(r#"{"info": {}, "item": []}"#), Ok(Format::Postman));
        assert_eq!(format(r#"{"resources": []}"#), Ok(Format::Insomnia));
        assert_eq!(format("[]"), Ok(Format::CyberApi));
        assert!(matches!(format("hello"), Err(ImportError::Unreadable { .. })));
        assert_eq!(parse("[]"), Err(ImportError::Empty));
    }

    #[test]
    fn every_item_gets_a_new_id() {
        let items = vec![
            ImportItem::Folder {
                name: "a".into(),
                children: vec![
                    ImportItem::Api {
                        name: "x".into(),
                        request: HttpRequest::default(),
                    },
                    ImportItem::Folder {
                        name: "b".into(),
                        children: vec![],
                    },
                ],
            },
            ImportItem::Api {
                name: "y".into(),
                request: HttpRequest::default(),
            },
        ];
        let mut next = 0;
        let assigned = assign_ids(&items, &mut || {
            next += 1;
            ItemId::new(format!("n{next}"))
        });
        assert_eq!(assigned.top, [ItemId::from("n1"), ItemId::from("n4")]);
        assert_eq!(
            assigned.folders,
            [
                (
                    "n1".into(),
                    "a".to_string(),
                    vec![ItemId::from("n2"), ItemId::from("n3")]
                ),
                ("n3".into(), "b".to_string(), vec![]),
            ]
        );
        assert_eq!(
            assigned
                .apis
                .iter()
                .map(|(id, name, _)| (id.as_str(), name.as_str()))
                .collect::<Vec<_>>(),
            [("n2", "x"), ("n4", "y")]
        );
    }

    #[test]
    fn query_strings_become_lists() {
        assert_eq!(
            split_url("{{HOST}}/a?b=1&c=x%20y"),
            ("{{HOST}}/a".to_string(), vec![kv("b", "1", true), kv("c", "x y", true)])
        );
        assert_eq!(
            content_type_of("application/json; charset=utf-8"),
            Some(ContentType::Json)
        );
        assert_eq!(content_type_of("application/vnd.api+json"), None);
        assert_eq!(guess_content_type(" [1] "), ContentType::Json);
    }
}
