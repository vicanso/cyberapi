//! The records a project is made of (section 4 of
//! `docs/gpui-refactor-feature-analysis.md`).
//!
//! These are the domain's shapes, not the old database's: `enabled` is a
//! `bool` rather than `"1"` / `"0"`, times are [`Timestamp`]s, a folder's
//! children are a list rather than a comma-separated string, and an API's
//! request is parsed rather than kept as JSON text. The `legacy` helpers at
//! the bottom convert the columns that need it; each store maps the rest.

use crate::access::CollectionRole;
use crate::{CollectionId, ItemId, TeamId, Timestamp, VariableId};
use serde::{Deserialize, Serialize};

/// A project: the top-level container. Variables are scoped to it too.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: CollectionId,
    pub name: String,
    pub description: String,
    pub created_at: Timestamp,
    /// Also moved by any change to the project's tree, requests or variables,
    /// so "last modified" means the project, not only its name (#21).
    pub updated_at: Timestamp,
    /// Counts the changes to what the project shares: its name, tree,
    /// requests and variables. The browser build polls it to see other
    /// readers' changes (`docs/architecture.md` §7).
    #[serde(default)]
    pub revision: u64,
    /// The team the project belongs to in the browser build; `None` on the
    /// desktop.
    #[serde(default)]
    pub owner: Option<TeamId>,
    /// What the reader may do with it — not stored, worked out per reader. The
    /// desktop's only user manages everything.
    #[serde(default)]
    pub role: CollectionRole,
}

/// A folder of a project's tree. Its `children` — folders and APIs mixed — is
/// the only record of who is inside whom, and their order is the display
/// order (4.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: ItemId,
    pub collection: CollectionId,
    pub name: String,
    pub children: Vec<ItemId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// What kind of request an API holds. Only HTTP exists; the column was left
/// room for GraphQL.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApiCategory {
    #[default]
    Http,
}

/// An API: a named, saved request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiSetting {
    pub id: ItemId,
    pub collection: CollectionId,
    pub name: String,
    pub category: ApiCategory,
    pub request: HttpRequest,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// The request an API saves (4.3). Every field may be missing from what the
/// old app wrote — it saved only what had been touched — so each defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HttpRequest {
    pub method: HttpMethod,
    /// May start with an environment variable: `{{HOST}}/users/v1/me`.
    pub uri: String,
    pub content_type: ContentType,
    /// Text for JSON / XML / plain bodies; the JSON of a `KvParam` list for
    /// form and multipart bodies (4.3).
    pub body: String,
    pub query: Vec<KvParam>,
    pub headers: Vec<KvParam>,
    /// Only the first enabled entry is used, as Basic auth (F4.9).
    pub auth: Vec<KvParam>,
}

impl HttpRequest {
    /// Whether the editor has a Body tab for it (F4.12): a method that sends
    /// one, a GraphQL query, which a GET sends as parameters, or a gRPC
    /// call's message.
    pub fn has_body_tab(&self) -> bool {
        self.method.has_body() || self.content_type == ContentType::GraphQl || self.method == HttpMethod::Grpc
    }

    /// Reads `api_settings.setting`. What cannot be read is an empty request
    /// rather than an error (F4.1): one damaged row must not hide a project.
    pub fn from_json_lenient(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        // A struct of strings, enums and vectors of them cannot fail to
        // serialize.
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// One row of a key / value list: query, headers, auth, form fields (F4.10).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KvParam {
    pub key: String,
    pub value: String,
    /// Missing means disabled: the old app tested `!kv.enabled`.
    pub enabled: bool,
}

/// A key / value list as text, one `key: value` per line and a disabled
/// row behind `//`: the lists' bulk edit, as other clients write it, and
/// the way headers are copied out of a browser.
pub fn kv_text(list: &[KvParam]) -> String {
    list.iter()
        .map(|kv| {
            let off = if kv.enabled { "" } else { "//" };
            format!("{off}{}: {}", kv.key, kv.value)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// [`kv_text`] read back. A line without `:` is a key alone; blank lines
/// are skipped. A value keeps any `:` after the first (`Host: a:8080`).
pub fn kv_from_text(text: &str) -> Vec<KvParam> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (enabled, line) = match line.strip_prefix("//") {
                Some(rest) => (false, rest.trim_start()),
                None => (true, line),
            };
            let (key, value) = line.split_once(':').unwrap_or((line, ""));
            KvParam {
                key: key.trim().to_string(),
                value: value.trim().to_string(),
                enabled,
            }
        })
        .filter(|kv| !kv.key.is_empty() || !kv.value.is_empty())
        .collect()
}

/// The request method. The old app stored the text and sent anything it did
/// not know as GET (F5.5); this keeps that, so an odd value reads as GET.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "&'static str")]
pub enum HttpMethod {
    #[default]
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Options,
    Head,
    Connect,
    Trace,
    /// A gRPC unary call (`crate::grpc`): sent as HTTP/2 POSTs are, to a
    /// service's method, with a message its definitions encode.
    Grpc,
}

impl HttpMethod {
    /// The methods the editor offers (F4.2), in its order.
    pub const EDITABLE: [Self; 8] = [
        Self::Get,
        Self::Post,
        Self::Put,
        Self::Patch,
        Self::Delete,
        Self::Options,
        Self::Head,
        Self::Grpc,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Options => "OPTIONS",
            Self::Head => "HEAD",
            Self::Connect => "CONNECT",
            Self::Trace => "TRACE",
            Self::Grpc => "gRPC",
        }
    }

    /// Case-insensitive; empty or unknown is GET.
    pub fn parse_lenient(text: &str) -> Self {
        match text.trim().to_ascii_uppercase().as_str() {
            "POST" => Self::Post,
            "PUT" => Self::Put,
            "PATCH" => Self::Patch,
            "DELETE" => Self::Delete,
            "OPTIONS" => Self::Options,
            "HEAD" => Self::Head,
            "CONNECT" => Self::Connect,
            "TRACE" => Self::Trace,
            "GRPC" => Self::Grpc,
            _ => Self::Get,
        }
    }

    /// Only these show a Body tab and send a body (F4.5); a DELETE too, a
    /// batch delete's ids, say, though only when it has one (#62).
    pub fn has_body(&self) -> bool {
        matches!(self, Self::Post | Self::Put | Self::Patch | Self::Delete)
    }
}

impl From<String> for HttpMethod {
    fn from(text: String) -> Self {
        Self::parse_lenient(&text)
    }
}

impl From<HttpMethod> for &'static str {
    fn from(method: HttpMethod) -> Self {
        method.as_str()
    }
}

/// The body's content type (F4.6). JSON is the default, and what an empty or
/// unknown stored value reads as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "&'static str")]
pub enum ContentType {
    #[default]
    Json,
    Form,
    Multipart,
    Xml,
    Plain,
    /// A GraphQL request: the body keeps a [`crate::graphql::GraphQlBody`],
    /// sent as JSON (a POST) or as query parameters (a GET).
    GraphQl,
}

impl ContentType {
    /// The choices the editor offers, in its order.
    pub const ALL: [Self; 6] = [
        Self::Json,
        Self::Form,
        Self::Multipart,
        Self::Xml,
        Self::Plain,
        Self::GraphQl,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Form => "application/x-www-form-urlencoded",
            Self::Multipart => "multipart/form-data",
            Self::Xml => "application/xml",
            Self::Plain => "text/plain",
            // What is stored; the request goes as `application/json`.
            Self::GraphQl => "application/graphql",
        }
    }

    pub fn parse_lenient(text: &str) -> Self {
        match text.trim() {
            "application/x-www-form-urlencoded" => Self::Form,
            "multipart/form-data" => Self::Multipart,
            "application/xml" => Self::Xml,
            "text/plain" => Self::Plain,
            "application/graphql" => Self::GraphQl,
            _ => Self::Json,
        }
    }

    /// Form and multipart bodies are edited and stored as a `KvParam` list;
    /// the others are text (4.3).
    pub fn is_key_value(&self) -> bool {
        matches!(self, Self::Form | Self::Multipart)
    }
}

impl From<String> for ContentType {
    fn from(text: String) -> Self {
        Self::parse_lenient(&text)
    }
}

impl From<ContentType> for &'static str {
    fn from(content_type: ContentType) -> Self {
        content_type.as_str()
    }
}

/// The three kinds of variable share one table and one set of operations,
/// kept apart by `(collection, category)` (M8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VariableCategory {
    /// Picked as a URI prefix: `{{HOST}}/path` (F5.2).
    #[serde(rename = "env")]
    Environment,
    /// Read by the `value(name)` template function.
    #[serde(rename = "customize")]
    Custom,
    /// Appended to every request of the project.
    #[serde(rename = "globalReqHeaders")]
    GlobalRequestHeader,
}

impl VariableCategory {
    /// The `variables.category` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Environment => "env",
            Self::Custom => "customize",
            Self::GlobalRequestHeader => "globalReqHeaders",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "env" => Some(Self::Environment),
            "customize" => Some(Self::Custom),
            "globalReqHeaders" => Some(Self::GlobalRequestHeader),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    pub id: VariableId,
    pub collection: CollectionId,
    pub category: VariableCategory,
    pub name: String,
    pub value: String,
    pub enabled: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Splits a URI into its environment variable and the rest (F4.3): the
/// editor shows `{{HOST}}` as a picker and the path beside it.
///
/// Only a leading `{{…}}` is a prefix, and it ends at the first `}}`. The old
/// regex was unanchored and greedy, so `{{HOST}}/a/{{id}}` read `HOST}}/a/{{id`
/// as the variable's name.
pub fn split_env_prefix(uri: &str) -> (Option<&str>, &str) {
    let Some(rest) = uri.strip_prefix("{{") else {
        return (None, uri);
    };
    match rest.find("}}") {
        Some(end) => {
            let name = rest[..end].trim();
            if name.is_empty() {
                (None, uri)
            } else {
                (Some(name), &rest[end + 2..])
            }
        }
        None => (None, uri),
    }
}

/// Conversions for the old database's columns.
pub mod legacy {
    use crate::ItemId;

    /// Reads `api_folders.children`: comma-separated, possibly with empty
    /// entries and repeats (the old app de-duplicated on every fetch). The
    /// first occurrence of an id keeps its place.
    pub fn parse_id_list(text: &str) -> Vec<ItemId> {
        let mut ids: Vec<ItemId> = Vec::new();
        for id in text.split(',').map(str::trim).filter(|id| !id.is_empty()) {
            if !ids.iter().any(|seen| seen.as_str() == id) {
                ids.push(ItemId::from(id));
            }
        }
        ids
    }

    pub fn join_id_list(ids: &[ItemId]) -> String {
        ids.iter().map(ItemId::as_str).collect::<Vec<_>>().join(",")
    }

    /// `variables.enabled` is `"1"` or `"0"`.
    pub fn parse_enabled(text: &str) -> bool {
        text.trim() == "1"
    }

    pub fn enabled_text(enabled: bool) -> &'static str {
        if enabled { "1" } else { "0" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn key_value_lists_go_to_text_and_back() {
        let list = vec![
            KvParam {
                key: "Host".into(),
                value: "a.test:8080".into(),
                enabled: true,
            },
            KvParam {
                key: "X-Off".into(),
                value: "1".into(),
                enabled: false,
            },
        ];
        let text = kv_text(&list);
        assert_eq!(text, "Host: a.test:8080\n//X-Off: 1");
        assert_eq!(kv_from_text(&text), list);
        // As copied from a browser: spaces, blank lines, a key alone.
        assert_eq!(
            kv_from_text("  accept:  */*  \n\n// x-a :b\nflag"),
            [
                KvParam {
                    key: "accept".into(),
                    value: "*/*".into(),
                    enabled: true,
                },
                KvParam {
                    key: "x-a".into(),
                    value: "b".into(),
                    enabled: false,
                },
                KvParam {
                    key: "flag".into(),
                    value: String::new(),
                    enabled: true,
                },
            ]
        );
    }

    #[test]
    fn a_partial_request_reads_with_defaults() {
        // The shapes the old app saved: only the fields that were touched.
        let request = HttpRequest::from_json_lenient(
            r#"{"uri":"{{HOST}}/users","query":[{"key":"a","value":"1","enabled":true}]}"#,
        );
        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(request.content_type, ContentType::Json);
        assert_eq!(request.uri, "{{HOST}}/users");
        assert_eq!(request.query.len(), 1);
        assert!(request.headers.is_empty());
    }

    #[test]
    fn a_damaged_request_is_empty() {
        assert_eq!(HttpRequest::from_json_lenient("{not json"), HttpRequest::default());
        assert_eq!(HttpRequest::from_json_lenient(""), HttpRequest::default());
    }

    #[test]
    fn a_request_round_trips_in_the_old_field_names() {
        let request = HttpRequest {
            method: HttpMethod::Post,
            uri: "/login".into(),
            content_type: ContentType::Form,
            body: "[]".into(),
            ..Default::default()
        };
        let json = request.to_json();
        assert!(json.contains(r#""method":"POST""#), "{json}");
        assert!(
            json.contains(r#""contentType":"application/x-www-form-urlencoded""#),
            "{json}"
        );
        assert_eq!(HttpRequest::from_json_lenient(&json), request);
    }

    #[test]
    fn a_missing_enabled_flag_is_disabled() {
        let request = HttpRequest::from_json_lenient(r#"{"headers":[{"key":"a","value":"b"}]}"#);
        assert!(!request.headers[0].enabled);
    }

    #[test]
    fn unknown_methods_are_get() {
        assert_eq!(HttpMethod::parse_lenient("post"), HttpMethod::Post);
        assert_eq!(HttpMethod::parse_lenient(""), HttpMethod::Get);
        assert_eq!(HttpMethod::parse_lenient("PURGE"), HttpMethod::Get);
    }

    #[test]
    fn env_prefix_is_split_at_the_first_close() {
        assert_eq!(split_env_prefix("{{HOST}}/users"), (Some("HOST"), "/users"));
        assert_eq!(split_env_prefix("{{ HOST }}/a/{{id}}"), (Some("HOST"), "/a/{{id}}"));
        assert_eq!(split_env_prefix("https://x/{{id}}"), (None, "https://x/{{id}}"));
        assert_eq!(split_env_prefix("{{HOST"), (None, "{{HOST"));
        assert_eq!(split_env_prefix("{{}}/a"), (None, "{{}}/a"));
    }

    #[test]
    fn child_lists_skip_blanks_and_repeats() {
        let ids = legacy::parse_id_list("a,,b,a, c ,");
        assert_eq!(legacy::join_id_list(&ids), "a,b,c");
        assert!(legacy::parse_id_list("").is_empty());
    }
}
