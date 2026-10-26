//! Importing APIs (M10): text in one of five formats becomes an
//! [`ImportBundle`] — a forest of folders and APIs, and environment
//! variables — which a store writes with ids of its own.
//!
//! The formats are told apart as before (F10.2): text starting with `curl`
//! is a cURL command; JSON with `swagger` is Swagger 2.0, with `openapi`
//! OpenAPI 3, with `item` a Postman collection, with `values` a Postman
//! environment, with `resources` an Insomnia export; any other JSON is
//! CyberAPI's own export.
//!
//! B5: also a HAR archive (`log.entries`), a Hoppscotch collection
//! (`folders` and `requests`, or a list of them) or environment (`name` and
//! `variables`), a Bruno `.bru` file (text whose first block is `meta {`, a
//! method's or `vars {`) and an OpenCollection document (`opencollection`,
//! or one request's `info` with its `http` or `graphql`), Bruno's YAML.
//!
//! B4: besides its items, an import brings how the project signs in (a
//! Postman collection's auth, an OpenAPI document's security, its values
//! left empty) and named environments (a Postman environment, Insomnia's
//! sub-environments); the project keeps an auth of its own, and an
//! environment it has already only gets the variables it lacks.
//!
//! The bundle carries no ids: the old app kept the ids of its own format,
//! so importing a file twice clashed, and retried APIs left their folders
//! pointing at the old ids (#7). A store gives every item a new id.

mod bruno;
mod curl;
mod har;
mod hoppscotch;
mod insomnia;
pub mod merge;
mod openapi;
mod opencollection;
mod own;
mod postman;

pub use own::{api_config, export, parse_own};

use crate::ItemId;
use crate::auth::Auth;
use crate::model::{ContentType, FolderSettings, HttpMethod, HttpRequest, KvParam};
use serde::{Deserialize, Serialize};
use snafu::Snafu;

/// One folder or API to add, with what it contains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ImportItem {
    Folder {
        name: String,
        children: Vec<ImportItem>,
        /// What it sets for the APIs in it (#66): only this app's own
        /// exports say.
        #[serde(default, skip_serializing_if = "FolderSettings::is_empty")]
        settings: FolderSettings,
    },
    /// Boxed: a request is several times the size of a folder.
    Api { name: String, request: Box<HttpRequest> },
}

/// What an import adds to a project.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportBundle {
    /// The top level of what is imported, in order.
    pub items: Vec<ImportItem>,
    /// Environment variables, name and value.
    pub environment: Vec<(String, String)>,
    /// What the document calls itself, for a project made from it (#65).
    pub name: Option<String>,
    /// B4: how the project signs in, as the document says; `Inherit` when
    /// it says nothing.
    #[serde(skip_serializing_if = "Auth::is_inherit")]
    pub auth: Auth,
    /// B4: named environments.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub environments: Vec<ImportEnvironment>,
}

/// A named environment an import brings (B4), with its variables, name and
/// value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportEnvironment {
    pub name: String,
    pub variables: Vec<(String, String)>,
}

/// Ids given to a bundle's items, depth first, and the records to write.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assigned {
    /// Each folder with its children's new ids and its settings, parents
    /// before children.
    pub folders: Vec<(ItemId, String, Vec<ItemId>, FolderSettings)>,
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
                ImportItem::Api { name, request } => out.apis.push((id.clone(), name.clone(), (**request).clone())),
                ImportItem::Folder {
                    name,
                    children,
                    settings,
                } => {
                    let at = out.folders.len();
                    out.folders
                        .push((id.clone(), name.clone(), Vec::new(), settings.clone()));
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
    /// Whether it brings nothing at all.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.environment.is_empty() && self.environments.is_empty() && self.auth.is_inherit()
    }

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
    /// B4: a Postman environment file.
    PostmanEnvironment,
    Insomnia,
    Swagger,
    OpenApi,
    /// B5.
    Har,
    Hoppscotch,
    HoppscotchEnvironment,
    Bruno,
    OpenCollection,
    CyberApi,
}

#[derive(Debug, Snafu, PartialEq, Eq)]
pub enum ImportError {
    #[snafu(display("not a cURL command, JSON or YAML: {message}"))]
    Unreadable { message: String },
    #[snafu(display("the cURL command has no URL"))]
    NoUrl,
    #[snafu(display("nothing to import"))]
    Empty,
}

/// F10.2. A document is JSON or, when it is not, YAML (#59): an OpenAPI
/// document is as often written in one as in the other.
pub fn detect(text: &str) -> Result<(Format, Option<serde_json::Value>), ImportError> {
    let trimmed = text.trim_start();
    // `get`, not a slice: four bytes may end inside a character.
    if trimmed.get(..4).is_some_and(|start| start.eq_ignore_ascii_case("curl")) {
        return Ok((Format::Curl, None));
    }
    if bruno::is_bru(text) {
        return Ok((Format::Bruno, None));
    }
    let json = document(text)?;
    let format = match &json {
        serde_json::Value::Object(map) if map.contains_key("swagger") => Format::Swagger,
        serde_json::Value::Object(map) if map.contains_key("openapi") => Format::OpenApi,
        serde_json::Value::Object(map) if map.contains_key("item") => Format::Postman,
        serde_json::Value::Object(map)
            if map.get("values").is_some_and(serde_json::Value::is_array)
                && (map.contains_key("_postman_variable_scope") || map.contains_key("name")) =>
        {
            Format::PostmanEnvironment
        }
        serde_json::Value::Object(map) if map.contains_key("resources") => Format::Insomnia,
        serde_json::Value::Object(map) if map.get("log").is_some_and(|log| log.get("entries").is_some()) => Format::Har,
        json if opencollection::is_opencollection(json) => Format::OpenCollection,
        json if hoppscotch::is_collection(json) => Format::Hoppscotch,
        json if hoppscotch::is_environment(json) => Format::HoppscotchEnvironment,
        _ => Format::CyberApi,
    };
    Ok((format, Some(json)))
}

/// JSON, or YAML that holds a mapping or a list: plain text reads as YAML
/// too, a single string, which is nothing to import. What looks like JSON
/// says why it is not JSON.
fn document(text: &str) -> Result<serde_json::Value, ImportError> {
    let json_error = match serde_json::from_str(text) {
        Ok(json) => return Ok(json),
        Err(error) => error,
    };
    let unreadable = |message: String| ImportError::Unreadable { message };
    if text.trim_start().starts_with(['{', '[']) {
        return Err(unreadable(json_error.to_string()));
    }
    match serde_saphyr::from_str::<serde_json::Value>(text) {
        Ok(yaml @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) => Ok(yaml),
        Ok(_) => Err(unreadable(json_error.to_string())),
        Err(error) => Err(unreadable(error.to_string())),
    }
}

/// F10.1–F10.3: whatever was pasted or picked.
pub fn parse(text: &str) -> Result<ImportBundle, ImportError> {
    let (format, json) = detect(text)?;
    let name = json.as_ref().and_then(|json| json_name(format, json));
    let bundle = match (format, json) {
        (Format::Curl, _) => curl::parse(text)?,
        (Format::Bruno, _) => bruno::parse(text),
        (Format::Postman, Some(json)) => postman::parse(&json),
        (Format::PostmanEnvironment, Some(json)) => postman::parse_environment(&json),
        (Format::Insomnia, Some(json)) => insomnia::parse(&json),
        (Format::Swagger | Format::OpenApi, Some(json)) => openapi::parse(&json),
        (Format::Har, Some(json)) => har::parse(&json),
        (Format::Hoppscotch, Some(json)) => hoppscotch::parse(&json),
        (Format::HoppscotchEnvironment, Some(json)) => hoppscotch::parse_environment(&json),
        (Format::OpenCollection, Some(json)) => opencollection::parse(&json),
        (_, Some(json)) => own::parse_value(&json),
        (_, None) => ImportBundle::default(),
    };
    if bundle.is_empty() {
        return EmptySnafu.fail();
    }
    Ok(ImportBundle { name, ..bundle })
}

/// A collection's own name: Postman's and OpenCollection's `info.name`,
/// an OpenAPI or Swagger document's `info.title`, an Insomnia export's
/// workspace's name, a Hoppscotch collection's `name`.
fn json_name(format: Format, json: &serde_json::Value) -> Option<String> {
    fn text(value: Option<&serde_json::Value>) -> Option<&str> {
        value.and_then(serde_json::Value::as_str).map(str::trim)
    }
    let name = match format {
        Format::Postman => text(json.pointer("/info/name")),
        Format::OpenCollection if json.get("opencollection").is_some() => text(json.pointer("/info/name")),
        Format::Hoppscotch => hoppscotch::name(json).map(str::trim),
        Format::Swagger | Format::OpenApi => text(json.pointer("/info/title")),
        Format::Insomnia => json
            .get("resources")
            .and_then(serde_json::Value::as_array)?
            .iter()
            .find(|resource| text(resource.get("_type")) == Some("workspace"))
            .and_then(|workspace| text(workspace.get("name"))),
        Format::Curl
        | Format::PostmanEnvironment
        | Format::Har
        | Format::HoppscotchEnvironment
        | Format::Bruno
        | Format::OpenCollection
        | Format::CyberApi => None,
    };
    name.filter(|name| !name.is_empty()).map(str::to_string)
}

/// The request of a cURL command pasted where a URI goes: the editor fills
/// itself in from it. The body stays as pasted (#43): indenting it would
/// change the bytes that are sent. `None` for anything else.
pub fn request_from_curl(text: &str) -> Option<HttpRequest> {
    let ImportItem::Api { request, .. } = curl_command(text)?.items.into_iter().next()? else {
        return None;
    };
    Some(*request)
}

/// A cURL command, as the API it imports, named after its path (F10.5):
/// what pasting one into a project adds. `None` for anything else.
pub fn curl_command(text: &str) -> Option<ImportBundle> {
    let text = text.trim_start();
    let command = text.get(..4).is_some_and(|start| start.eq_ignore_ascii_case("curl"))
        && text[4..].starts_with(char::is_whitespace);
    if !command {
        return None;
    }
    curl::parse(text).ok()
}

/// A request's URL with its query moved to the query list, as every importer
/// does (F10.3), and as the URI field does with a query typed into it
/// (#50). A URL that does not parse — `{{HOST}}/a?b=1` — is split by hand;
/// a fragment stays with the URL, where it was.
pub fn split_url(url: &str) -> (String, Vec<KvParam>) {
    let (url, fragment) = match url.split_once('#') {
        Some((url, fragment)) => (url, format!("#{fragment}")),
        None => (url, String::new()),
    };
    let Some((base, query)) = url.split_once('?') else {
        return (format!("{url}{fragment}"), Vec::new());
    };
    let pairs = url::form_urlencoded::parse(query.as_bytes())
        .map(|(key, value)| KvParam {
            key: key.into_owned(),
            value: value.into_owned(),
            enabled: true,
        })
        .collect();
    (format!("{base}{fragment}"), pairs)
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
        "application/graphql" => Some(ContentType::GraphQl),
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
        let bundle = curl_command("curl 'https://a.test/v1/users/login' -d 'a=1'").expect("a command");
        assert!(matches!(&bundle.items[..], [ImportItem::Api { name, .. }] if name == "login"));
        assert!(bundle.environment.is_empty());
        assert!(curl_command("  {\"item\": []}").is_none(), "only a cURL command");
    }

    #[test]
    fn formats_are_told_apart() {
        let format = |text: &str| detect(text).map(|(format, _)| format);
        assert_eq!(format("  curl https://a.test"), Ok(Format::Curl));
        assert_eq!(format(r#"{"swagger": "2.0"}"#), Ok(Format::Swagger));
        assert_eq!(format(r#"{"openapi": "3.0.1"}"#), Ok(Format::OpenApi));
        assert_eq!(format(r#"{"info": {}, "item": []}"#), Ok(Format::Postman));
        assert_eq!(format(r#"{"resources": []}"#), Ok(Format::Insomnia));
        assert_eq!(format(r#"{"log": {"entries": []}}"#), Ok(Format::Har));
        assert_eq!(
            format(r#"{"name": "a", "folders": [], "requests": []}"#),
            Ok(Format::Hoppscotch)
        );
        assert_eq!(
            format(r#"[{"name": "a", "folders": [], "requests": []}]"#),
            Ok(Format::Hoppscotch)
        );
        assert_eq!(
            format(r#"{"name": "dev", "variables": [{"key": "a", "value": "1"}]}"#),
            Ok(Format::HoppscotchEnvironment)
        );
        assert_eq!(format("meta {\n  name: a\n}\nget {\n  url: /a\n}\n"), Ok(Format::Bruno));
        assert_eq!(
            format("opencollection: 1.0.0\ninfo:\n  name: a\n"),
            Ok(Format::OpenCollection)
        );
        assert_eq!(
            format("info:\n  name: a\n  type: http\nhttp:\n  url: /a\n"),
            Ok(Format::OpenCollection)
        );
        assert_eq!(format("[]"), Ok(Format::CyberApi));
        assert!(matches!(format("hello"), Err(ImportError::Unreadable { .. })));
        // A character of several bytes across the fourth is text, not a panic.
        assert!(matches!(format("新建接口"), Err(ImportError::Unreadable { .. })));
        assert!(matches!(format("cu新"), Err(ImportError::Unreadable { .. })));
        assert_eq!(parse("[]"), Err(ImportError::Empty));
    }

    #[test]
    fn an_openapi_document_in_yaml_imports_as_its_json_would() {
        let yaml = r#"
openapi: 3.0.1
info:
  title: Pets
paths:
  /pets/{id}:
    get:
      summary: one pet
      tags: [pets]
      responses:
        200:
          description: ok
  /stores:
    post:
      summary: a store
"#;
        let json = r#"{"openapi": "3.0.1", "info": {"title": "Pets"}, "paths": {
            "/pets/{id}": {"get": {"summary": "one pet", "tags": ["pets"], "responses": {"200": {"description": "ok"}}}},
            "/stores": {"post": {"summary": "a store"}}
        }}"#;
        assert_eq!(detect(yaml).map(|(format, _)| format), Ok(Format::OpenApi));
        assert_eq!(parse(yaml), parse(json));
        assert_eq!(parse(yaml).expect("parse").name.as_deref(), Some("Pets"));
        // Plain text is a YAML string, which is nothing to import; what
        // looks like JSON says why it is not.
        assert!(matches!(detect("hello"), Err(ImportError::Unreadable { .. })));
        let Err(ImportError::Unreadable { message }) = detect(r#"{"a": "#) else {
            panic!("not JSON")
        };
        assert!(message.contains("EOF"), "{message}");
        // YAML that does not read says why.
        assert!(matches!(detect("a: [1, 2"), Err(ImportError::Unreadable { .. })));
    }

    #[test]
    fn a_collection_names_the_project_made_from_it() {
        let name = |text: &str| parse(text).expect("parse").name;
        let api = r#"{"name": "a", "request": {"method": "GET", "url": "http://a.test"}}"#;
        assert_eq!(
            name(&format!(r#"{{"info": {{"name": " Shop "}}, "item": [{api}]}}"#)),
            Some("Shop".into())
        );
        let paths = r#""paths": {"/a": {"get": {"summary": "a"}}}"#;
        assert_eq!(
            name(&format!(
                r#"{{"openapi": "3.0.1", "info": {{"title": "Pets"}}, {paths}}}"#
            )),
            Some("Pets".into())
        );
        assert_eq!(
            name(&format!(r#"{{"swagger": "2.0", "info": {{"title": ""}}, {paths}}}"#)),
            None,
            "an empty name is none"
        );
        let insomnia = r#"{"resources": [
            {"_type": "workspace", "_id": "wrk_1", "name": "Billing"},
            {"_type": "request", "_id": "req_1", "parentId": "wrk_1", "name": "a", "method": "GET", "url": "http://a.test"}
        ]}"#;
        assert_eq!(name(insomnia), Some("Billing".into()));
        assert_eq!(name("curl http://a.test/x"), None);
    }

    #[test]
    fn every_item_gets_a_new_id() {
        let items = vec![
            ImportItem::Folder {
                name: "a".into(),
                settings: Default::default(),
                children: vec![
                    ImportItem::Api {
                        name: "x".into(),
                        request: Box::default(),
                    },
                    ImportItem::Folder {
                        name: "b".into(),
                        children: vec![],
                        settings: Default::default(),
                    },
                ],
            },
            ImportItem::Api {
                name: "y".into(),
                request: Box::default(),
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
                    vec![ItemId::from("n2"), ItemId::from("n3")],
                    FolderSettings::default()
                ),
                ("n3".into(), "b".to_string(), vec![], FolderSettings::default()),
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
        // A key alone, an empty pair, a template, a fragment.
        assert_eq!(
            split_url("/a?flag&&t={{value(token)}}#top"),
            (
                "/a#top".to_string(),
                vec![kv("flag", "", true), kv("t", "{{value(token)}}", true)]
            )
        );
        assert_eq!(split_url("/a"), ("/a".to_string(), vec![]));
        assert_eq!(split_url("/a?"), ("/a".to_string(), vec![]));
        assert_eq!(split_url("/a#x"), ("/a#x".to_string(), vec![]));
        assert_eq!(
            content_type_of("application/json; charset=utf-8"),
            Some(ContentType::Json)
        );
        assert_eq!(content_type_of("application/vnd.api+json"), None);
        assert_eq!(guess_content_type(" [1] "), ContentType::Json);
    }
}
