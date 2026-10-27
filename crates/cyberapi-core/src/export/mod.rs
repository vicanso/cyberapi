//! A project, or one folder and what is in it, written for another client
//! (B1, B2): a Postman collection (v2.1) or an OpenAPI 3.0 document. This
//! app's own export, which brings everything back, is
//! [`crate::import::export`]. What the other formats have no place for is
//! left out — a gRPC call — or kept as written — a template function.

mod openapi;
mod postman;

pub use openapi::openapi;
pub use postman::postman;

use crate::ItemId;
use crate::auth::Auth;
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
}

impl ExportFormat {
    pub const ALL: [Self; 3] = [Self::CyberApi, Self::Postman, Self::OpenApi];
}

/// An item to export, in the tree's order.
enum Item<'a> {
    Folder(&'a Folder, Vec<Item<'a>>),
    Api(&'a ApiSetting),
}

/// What is in `folder`, or the whole project, in the tree's order; gRPC
/// calls left out.
fn items<'a>(data: &'a TreeData, folder: Option<&ItemId>) -> Vec<Item<'a>> {
    fn walk<'a>(data: &'a TreeData, tree: &Tree, ids: &[ItemId]) -> Vec<Item<'a>> {
        ids.iter()
            .filter_map(|id| {
                if let Some(folder) = data.folders.iter().find(|folder| &folder.id == id) {
                    let children = tree.get(id).map(|node| node.children().to_vec()).unwrap_or_default();
                    return Some(Item::Folder(folder, walk(data, tree, &children)));
                }
                data.apis
                    .iter()
                    // Neither has a counterpart in the formats exported.
                    .find(|api| {
                        &api.id == id && !matches!(api.request.method, HttpMethod::Grpc | HttpMethod::WebSocket)
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
    walk(data, &tree, &top)
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
