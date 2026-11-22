//! A project, or one folder and what is in it, written for another client
//! (B1, B2, R11): a Postman collection (v2.1), an OpenAPI 3.0 document, an
//! HTTP Archive (HAR 1.2), or a `.http` file. This app's own export, which
//! brings everything back, is [`crate::import::export`]. What the other formats
//! have no place for is left out — a gRPC call — or kept as written — a
//! template function.

mod har;
mod http_file;
mod openapi;
mod postman;
mod schema_30;

pub use har::har;
pub use http_file::http_file;
pub use openapi::openapi;
pub use postman::postman;

use crate::ItemId;
use crate::auth::{Auth, Signed};
use crate::inherit;
use crate::model::{ApiSetting, Folder, HttpMethod};
use crate::tree::{Tree, TreeData};

/// What a project or a folder is exported as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExportFormat {
    /// This app's own: everything comes back on import.
    CyberApi,
    /// B2.
    Postman,
    /// B1.
    OpenApi,
    /// R11: HTTP Archive (HAR 1.2).
    Har,
    /// R11: `.http` request file (REST Client / JetBrains HTTP Client).
    Http,
    /// F17: standalone single-file HTML documentation.
    Html,
}

impl ExportFormat {
    pub const ALL: [Self; 6] = [
        Self::CyberApi,
        Self::Postman,
        Self::OpenApi,
        Self::Har,
        Self::Http,
        Self::Html,
    ];
}

/// An item to export, in the tree's order.
enum Item<'a> {
    Folder(&'a Folder, Vec<Item<'a>>),
    Api(&'a ApiSetting),
}

/// What is in `folder`, or the whole project, in the tree's order; gRPC calls
/// are always left out, and WebSocket calls are included only when requested.
fn collect_items<'a>(data: &'a TreeData, folder: Option<&ItemId>, include_websocket: bool) -> Vec<Item<'a>> {
    fn walk<'a>(data: &'a TreeData, tree: &Tree, ids: &[ItemId], include_websocket: bool) -> Vec<Item<'a>> {
        ids.iter()
            .filter_map(|id| {
                if let Some(folder) = data.folders.iter().find(|folder| &folder.id == id) {
                    let children = tree.get(id).map(|node| node.children().to_vec()).unwrap_or_default();
                    return Some(Item::Folder(folder, walk(data, tree, &children, include_websocket)));
                }
                data.apis
                    .iter()
                    .find(|api| {
                        &api.id == id
                            && match api.request.method {
                                HttpMethod::Grpc => false,
                                HttpMethod::WebSocket => include_websocket,
                                _ => true,
                            }
                    })
                    .map(Item::Api)
            })
            .collect()
    }
    let tree = Tree::build(data);
    let top = match folder {
        Some(id) => tree.get(id).map(|node| node.children().to_vec()).unwrap_or_default(),
        None => tree.top_level_ids(),
    };
    walk(data, &tree, &top, include_websocket)
}

/// What is in `folder`, or the whole project, in the tree's order; gRPC
/// and WebSocket calls left out.
fn items<'a>(data: &'a TreeData, folder: Option<&ItemId>) -> Vec<Item<'a>> {
    collect_items(data, folder, false)
}

/// What is in `folder`, or the whole project, in the tree's order; WebSocket
/// calls included, gRPC calls left out.
fn items_with_websocket<'a>(data: &'a TreeData, folder: Option<&ItemId>) -> Vec<Item<'a>> {
    collect_items(data, folder, true)
}

/// Flattens hierarchical items depth-first into `(api, folder_path)` pairs.
fn flatten_apis<'a>(items: &[Item<'a>]) -> Vec<(&'a ApiSetting, String)> {
    fn walk<'a>(items: &[Item<'a>], path: &str, out: &mut Vec<(&'a ApiSetting, String)>) {
        for item in items {
            match item {
                Item::Api(api) => out.push((api, path.to_string())),
                Item::Folder(folder, children) => {
                    let next = if path.is_empty() {
                        folder.name.clone()
                    } else {
                        format!("{path} / {}", folder.name)
                    };
                    walk(children, &next, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(items, "", &mut out);
    out
}

/// Inserts or replaces a header case-insensitively while preserving order.
fn upsert_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return;
    }
    if let Some(existing) = headers.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(trimmed)) {
        existing.0 = trimmed.to_string();
        existing.1 = value.to_string();
    } else {
        headers.push((trimmed.to_string(), value.to_string()));
    }
}

type Pairs = Vec<(String, String)>;

/// Computes effective enabled `(headers, query)` for `api`, merging global
/// headers, inherited folder headers, static auth (`Basic`, `Bearer`, `ApiKey`),
/// and the request's own headers/query.
fn effective_headers_and_query(
    data: &TreeData,
    api: &ApiSetting,
    global_headers: &[(String, String)],
) -> (Pairs, Pairs) {
    let mut headers = Vec::new();
    for (name, value) in global_headers {
        upsert_header(&mut headers, name, value);
    }

    let inherited = inherit::inherited(data, &api.id);
    for kv in inherited.sent_headers() {
        upsert_header(&mut headers, &kv.key, &kv.value);
    }

    let auth = inherited.auth(&api.request.auth);
    let has_static_auth = match auth {
        Auth::Basic { .. } => auth.has_credentials(),
        Auth::Bearer { token } => !token.trim().is_empty(),
        Auth::ApiKey { name, .. } => !name.trim().is_empty(),
        _ => false,
    };

    let mut auth_query = None;
    if has_static_auth
        && let Ok(Some(signed)) = auth.sign::<std::convert::Infallible>(true, |value| Ok(value.to_string()))
    {
        match signed {
            Signed::Header { name, value } => {
                // Request's own enabled header with the same name wins below.
                upsert_header(&mut headers, &name, &value);
            }
            Signed::Query { name, value } => {
                auth_query = Some((name, value));
            }
        }
    }

    for kv in &api.request.headers {
        if kv.enabled {
            upsert_header(&mut headers, &kv.key, &kv.value);
        }
    }

    let mut query: Vec<(String, String)> = api
        .request
        .query
        .iter()
        .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
        .map(|kv| (kv.key.trim().to_string(), kv.value.clone()))
        .collect();

    if let Some((name, value)) = auth_query
        && !query.iter().any(|(existing, _)| existing == &name)
    {
        query.push((name, value));
    }

    (headers, query)
}

/// How what is exported signs in as a whole: the project's, or what the
/// exported folder sets or takes from above it.
fn exported_auth(data: &TreeData, folder: Option<&ItemId>) -> Auth {
    let Some(id) = folder else {
        return data.settings.auth.clone();
    };
    let own = data
        .folders
        .iter()
        .find(|found| &found.id == id)
        .map(|found| found.settings.auth.clone())
        .unwrap_or_default();
    inherit::inherited(data, id).auth(&own).clone()
}
