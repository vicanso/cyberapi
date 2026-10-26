//! F2: what the command palette finds besides its commands — the projects
//! the reader sees, and the APIs in them. The open project's are its own
//! as they are now; the others' are read when the palette opens, the most
//! recently changed first, up to [`OTHER_PROJECTS`] of them.

use super::AppStore;
use cyberapi_core::model::{Collection, HttpMethod};
use cyberapi_core::tree::{Tree, TreeData};
use cyberapi_core::{CollectionId, ItemId};
use gpui_kit::{AppContext as _, Context, Task};

/// How many projects besides the open one have their APIs read.
pub const OTHER_PROJECTS: usize = 50;

/// An API the palette finds.
#[derive(Clone, Debug)]
pub struct FoundApi {
    pub collection: Collection,
    pub api: ItemId,
    pub name: String,
    pub method: HttpMethod,
    /// Its folders' names, `/` between.
    pub folders: String,
}

pub struct PaletteState {
    projects: Vec<Collection>,
    apis: Vec<FoundApi>,
    /// Still reading the other projects.
    reading: bool,
    _reading: Option<Task<()>>,
}

/// The APIs of `data`, the project `collection`'s, in the tree's order.
fn found_in(collection: &Collection, data: &TreeData) -> Vec<FoundApi> {
    let tree = Tree::build(data);
    cyberapi_core::run::plan(data, None)
        .into_iter()
        .filter_map(|id| {
            let api = data.apis.iter().find(|api| api.id == id)?;
            let mut folders = Vec::new();
            let mut parent = tree.get(&id).and_then(|node| node.parent()).cloned();
            while let Some(folder) = parent {
                let node = tree.get(&folder);
                folders.push(node.map(|node| node.name().to_string()).unwrap_or_default());
                parent = node.and_then(|node| node.parent()).cloned();
            }
            folders.reverse();
            Some(FoundApi {
                collection: collection.clone(),
                api: id,
                name: api.name.clone(),
                method: api.request.method,
                folders: folders.join("/"),
            })
        })
        .collect()
}

impl PaletteState {
    /// Reads the projects and their APIs; `open` is the project open now,
    /// with its tree as it is.
    pub fn new(open: Option<(Collection, TreeData)>, cx: &mut Context<Self>) -> Self {
        let apis = open
            .as_ref()
            .map(|(collection, data)| found_in(collection, data))
            .unwrap_or_default();
        let open_id: Option<CollectionId> = open.map(|(collection, _)| collection.id);
        let store = AppStore::global(cx);
        let storage = store.storage().clone();
        let server = store.server().cloned();
        let task = cx.spawn(async move |this, cx| {
            let Ok(mut projects) = cx.background_spawn(storage.collections().list()).await else {
                let _ = this.update(cx, |this, cx| {
                    this.reading = false;
                    cx.notify();
                });
                return;
            };
            // Another account's copies are not this reader's (server-sync §1).
            let _ = this.update(cx, |_, cx| {
                if let Some(server) = &server {
                    projects.retain(|project| !server.read(cx).hides(&project.id));
                }
            });
            projects.sort_by_key(|project| std::cmp::Reverse(project.updated_at));
            let _ = this.update(cx, |this, cx| {
                this.projects = projects.clone();
                cx.notify();
            });
            let others: Vec<Collection> = projects
                .into_iter()
                .filter(|project| Some(&project.id) != open_id.as_ref())
                .take(OTHER_PROJECTS)
                .collect();
            for project in others {
                let read = cx.background_spawn(storage.tree().load(project.id.clone())).await;
                if let Ok(data) = read {
                    let found = found_in(&project, &data);
                    if this
                        .update(cx, |this, cx| {
                            this.apis.extend(found);
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.reading = false;
                cx.notify();
            });
        });
        Self {
            projects: Vec::new(),
            apis,
            reading: true,
            _reading: Some(task),
        }
    }

    pub fn projects(&self) -> &[Collection] {
        &self.projects
    }

    /// The open project's APIs first, then the others'.
    pub fn apis(&self) -> &[FoundApi] {
        &self.apis
    }

    pub fn reading(&self) -> bool {
        self.reading
    }
}
