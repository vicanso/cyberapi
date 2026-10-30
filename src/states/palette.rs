//! F2: what the command palette finds besides its commands — the projects
//! the reader sees, and the APIs in them. The open project's are its own
//! as they are now; the others' are read once the reader searches, the
//! most recently changed first, up to [`OTHER_PROJECTS`] of them.

use super::AppStore;
use cyberapi_core::model::{ApiSetting, Collection, HttpMethod, split_env_prefix};
use cyberapi_core::search::matches_text_or_pinyin;
use cyberapi_core::tree::{Tree, TreeData};
use cyberapi_core::{CollectionId, ItemId};
use gpui_kit::{AppContext as _, Context, Task};
use std::collections::HashMap;

/// How many projects besides the open one have their APIs read.
pub const OTHER_PROJECTS: usize = 50;

/// An API the palette finds.
#[derive(Clone, Debug)]
pub struct FoundApi {
    pub collection: Collection,
    pub api: ItemId,
    pub name: String,
    pub method: HttpMethod,
    /// F8: its URI without an `{{ENV}}` prefix, as the tree searches it.
    pub path: String,
    /// Its folders' names, `/` between.
    pub folders: String,
}

impl FoundApi {
    /// F8: whether `query` matches this API's folders, name, method or path
    /// (and, for another project's API, that project's name), whole or word by
    /// word.
    pub fn matches(&self, query: &str, with_project: bool) -> bool {
        let query = query.trim();
        if query.is_empty() {
            return true;
        }
        let text = if with_project {
            format!(
                "{} {} {} {} {}",
                self.collection.name,
                self.folders,
                self.name,
                self.method.as_str(),
                self.path
            )
        } else {
            format!("{} {} {} {}", self.folders, self.name, self.method.as_str(), self.path)
        };
        matches_text_or_pinyin(&text, query)
            || (query.contains(char::is_whitespace)
                && query.split_whitespace().all(|part| matches_text_or_pinyin(&text, part)))
    }
}

/// Whether the other projects' APIs have been read.
enum Others {
    /// Not asked for: they show only for a search.
    NotAsked,
    /// Asked for before the projects were listed.
    Wanted,
    Reading(#[allow(dead_code)] Task<()>),
    Read,
}

pub struct PaletteState {
    projects: Vec<Collection>,
    apis: Vec<FoundApi>,
    /// The open project, whose APIs are there from the start.
    open: Option<CollectionId>,
    /// Still listing the projects.
    listing: bool,
    others: Others,
    _listing: Option<Task<()>>,
}

/// The APIs of `data`, the project `collection`'s, in the tree's order.
pub fn found_in(collection: &Collection, data: &TreeData) -> Vec<FoundApi> {
    let tree = Tree::build(data);
    // PERF-11: by id, not searched for each.
    let apis: HashMap<&ItemId, &ApiSetting> = data.apis.iter().map(|api| (&api.id, api)).collect();
    cyberapi_core::run::plan(data, None)
        .into_iter()
        .filter_map(|id| {
            let api = apis.get(&id)?;
            let mut folders = Vec::new();
            let mut parent = tree.get(&id).and_then(|node| node.parent()).cloned();
            while let Some(folder) = parent {
                let node = tree.get(&folder);
                folders.push(node.map(|node| node.name().to_string()).unwrap_or_default());
                parent = node.and_then(|node| node.parent()).cloned();
            }
            folders.reverse();
            let (_, path) = split_env_prefix(&api.request.uri);
            Some(FoundApi {
                collection: collection.clone(),
                api: id,
                name: api.name.clone(),
                method: api.request.method,
                path: path.to_string(),
                folders: folders.join("/"),
            })
        })
        .collect()
}

impl PaletteState {
    /// Lists the projects; `open` is the project open now, with its APIs
    /// as they are. The others' APIs are read once [`Self::read_others`]
    /// asks for them.
    pub fn new(open: Option<(Collection, Vec<FoundApi>)>, cx: &mut Context<Self>) -> Self {
        let (open, apis) = match open {
            Some((collection, apis)) => (Some(collection.id), apis),
            None => (None, Vec::new()),
        };
        let store = AppStore::global(cx);
        let storage = store.storage().clone();
        let server = store.server().cloned();
        let task = cx.spawn(async move |this, cx| {
            let listed = cx.background_spawn(storage.collections().list()).await;
            let _ = this.update(cx, |this, cx| {
                this.listing = false;
                if let Ok(mut projects) = listed {
                    // Another account's copies are not this reader's
                    // (server-sync §1).
                    if let Some(server) = &server {
                        projects.retain(|project| !server.read(cx).hides(&project.id));
                    }
                    projects.sort_by_key(|project| std::cmp::Reverse(project.updated_at));
                    this.projects = projects;
                }
                if matches!(this.others, Others::Wanted) {
                    this.read(cx);
                }
                cx.notify();
            });
        });
        Self {
            projects: Vec::new(),
            apis,
            open,
            listing: true,
            others: Others::NotAsked,
            _listing: Some(task),
        }
    }

    /// PERF-11: reads the other projects' APIs, the most recently changed
    /// first, once: they show only for a search, and each is read whole.
    pub fn read_others(&mut self, cx: &mut Context<Self>) {
        match self.others {
            Others::NotAsked if self.listing => self.others = Others::Wanted,
            Others::NotAsked => self.read(cx),
            _ => {}
        }
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let storage = AppStore::global(cx).storage().clone();
        let others: Vec<Collection> = self
            .projects
            .iter()
            .filter(|project| Some(&project.id) != self.open.as_ref())
            .take(OTHER_PROJECTS)
            .cloned()
            .collect();
        let task = cx.spawn(async move |this, cx| {
            for project in others {
                let read = cx.background_spawn(storage.tree().load(project.id.clone())).await;
                if let Ok(data) = read {
                    let found = cx.background_spawn(async move { found_in(&project, &data) }).await;
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
                this.others = Others::Read;
                cx.notify();
            });
        });
        self.others = Others::Reading(task);
        cx.notify();
    }

    pub fn projects(&self) -> &[Collection] {
        &self.projects
    }

    /// The open project's APIs first, then the others' read so far.
    pub fn apis(&self) -> &[FoundApi] {
        &self.apis
    }

    /// Still listing the projects, or reading the APIs asked for.
    pub fn reading(&self) -> bool {
        self.listing || matches!(self.others, Others::Wanted | Others::Reading(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Extras;
    use cyberapi_core::prefs::AppPrefs;
    use cyberapi_http::{HttpEngine, PreparedRequest, SendFuture};
    use cyberapi_store::{MemoryStore, NewCollection, Storage};
    use gpui_kit::{Entity, TestAppContext};
    use std::sync::Arc;

    struct NoEngine;

    impl HttpEngine for NoEngine {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            Box::pin(std::future::pending())
        }
    }

    async fn project(storage: &Storage, name: &str, api: &str) -> Collection {
        let collection = storage
            .collections()
            .create(NewCollection {
                name: name.into(),
                ..Default::default()
            })
            .await
            .expect("create");
        storage
            .tree()
            .create_api(collection.id.clone(), None, api.into())
            .await
            .expect("an API");
        collection
    }

    fn names(palette: &Entity<PaletteState>, cx: &mut TestAppContext) -> Vec<String> {
        palette.read_with(cx, |palette, _| {
            let mut names: Vec<String> = palette.apis().iter().map(|api| api.name.clone()).collect();
            names.sort();
            names
        })
    }

    /// PERF-11: the other projects' APIs are read once asked for — by a
    /// search — not as the palette opens; asked for before the projects
    /// are listed, once they are.
    #[gpui_kit::test]
    async fn the_other_projects_are_read_once_asked_for(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        cx.update(|cx| {
            gpui_kit::init(cx);
            AppStore::init(
                storage.clone(),
                Arc::new(NoEngine),
                AppPrefs::default(),
                Extras::default(),
                cx,
            );
        });
        let open = project(&storage, "open", "mine").await;
        project(&storage, "other", "theirs").await;
        let data = storage.tree().load(open.id.clone()).await.expect("load");
        let mine = found_in(&open, &data);
        let palette = cx.new(|cx| PaletteState::new(Some((open, mine)), cx));
        cx.run_until_parked();
        assert_eq!(names(&palette, cx), ["mine"]);
        palette.read_with(cx, |palette, _| {
            assert_eq!(palette.projects().len(), 2);
            assert!(!palette.reading());
        });
        palette.update(cx, |palette, cx| palette.read_others(cx));
        cx.run_until_parked();
        assert_eq!(names(&palette, cx), ["mine", "theirs"]);

        let early = cx.new(|cx| {
            let mut palette = PaletteState::new(None, cx);
            palette.read_others(cx);
            assert!(palette.reading());
            palette
        });
        cx.run_until_parked();
        assert_eq!(names(&early, cx), ["mine", "theirs"]);
        assert!(!early.read_with(cx, |palette, _| palette.reading()));
    }

    #[test]
    fn finds_an_api_by_its_path_without_its_env_prefix() {
        let collection = Collection {
            id: "c1".into(),
            name: "Shop".into(),
            ..Default::default()
        };
        let mut api = ApiSetting {
            id: "a1".into(),
            name: "创建订单".into(),
            ..Default::default()
        };
        api.request.method = HttpMethod::Post;
        api.request.uri = "{{API_HOST}}/v1/orders/{id}".into();
        let data = TreeData {
            apis: vec![api],
            folders: Vec::new(),
            top_order: vec!["a1".into()],
            settings: Default::default(),
        };

        let found = found_in(&collection, &data);
        assert_eq!(found.len(), 1);
        let entry = &found[0];
        assert_eq!(entry.path, "/v1/orders/{id}");
        assert!(entry.matches("/v1/orders", false));
        assert!(entry.matches("orders/{id}", false));
        assert!(entry.matches("POST /v1/orders", false));
        assert!(entry.matches("cjdd /v1", false));
        assert!(!entry.matches("API_HOST", false));
        assert!(!entry.matches("Shop /v1", false));
        assert!(entry.matches("Shop /v1", true));
    }
}
