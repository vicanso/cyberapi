//! The project list (M2).
//!
//! In a page the projects are the server's, and a reader's unsynced
//! changes to one wait in a draft in this browser (`docs/server-sync.md`
//! §3–4): its card shows the draft's name and says so, and syncs it or
//! gives it up without opening the project. A change of name or
//! description is one of those changes.

use super::copy::{self, Pushed};
use super::{Load, run_store};
use crate::drafts_local::{self, Draft};
use cyberapi_core::CollectionId;
use cyberapi_core::model::Collection;
use cyberapi_core::sync::{self, Moment, ProjectConfig, Step};
use cyberapi_store::{CollectionEdit, NewCollection, Storage, StoreError, rules};
use gpui_kit::{Context, EventEmitter};
use std::collections::{BTreeMap, BTreeSet};
use tracing::warn;

/// What the view should tell the reader about.
#[derive(Clone, Debug)]
pub enum DashboardEvent {
    /// A change the reader asked for did not happen.
    Failed(StoreError),
    /// A project is gone, and with it the reader's state of it.
    Deleted(CollectionId),
    /// A project the reader made, to be opened.
    Created(Collection),
    /// A change now waits in this browser for the reader to sync it.
    Unsynced,
    /// This browser could not keep a change: a refresh loses it.
    NotKept,
    /// The server has the project's draft now.
    Synced(Collection),
    /// The server moved on since the draft's base: the reader chooses.
    Conflict(Collection),
}

pub struct DashboardState {
    storage: Storage,
    /// As the server lists them.
    listed: Vec<Collection>,
    /// As they show: with the name and description of their drafts.
    collections: Vec<Collection>,
    /// How many APIs each project holds, for its card; read beside the
    /// list, and `None` until it was.
    api_counts: Option<BTreeMap<CollectionId, usize>>,
    load: Load,
    /// Whose drafts (`crate::pins_local::scope`); `None` where changes are
    /// written at once.
    scope: Option<String>,
    drafts: BTreeMap<CollectionId, Draft>,
    /// Drafts on their way to the server.
    syncing: BTreeSet<CollectionId>,
}

impl EventEmitter<DashboardEvent> for DashboardState {}

impl DashboardState {
    /// `scope`: whose drafts the projects have, where they are the
    /// server's.
    pub fn new(storage: Storage, scope: Option<String>, cx: &mut Context<Self>) -> Self {
        let mut state = Self {
            storage,
            listed: Vec::new(),
            collections: Vec::new(),
            api_counts: None,
            load: Load::Loading,
            scope,
            drafts: BTreeMap::new(),
            syncing: BTreeSet::new(),
        };
        state.reload(cx);
        state
    }

    /// The reader is known now, or another: their drafts show.
    pub fn set_scope(&mut self, scope: Option<String>, cx: &mut Context<Self>) {
        if self.scope != scope {
            self.scope = scope;
            self.read_drafts();
            cx.notify();
        }
    }

    fn read_drafts(&mut self) {
        self.drafts = self.scope.as_deref().map(drafts_local::all).unwrap_or_default();
        self.show();
    }

    /// The list as it shows: a draft's name and description over the
    /// server's. A viewer changes nothing, so no draft counts for them.
    fn show(&mut self) {
        self.collections = self
            .listed
            .iter()
            .map(|collection| match self.draft(collection) {
                Some(draft) => Collection {
                    name: draft.config.name.clone(),
                    description: draft.config.description.clone(),
                    ..collection.clone()
                },
                None => collection.clone(),
            })
            .collect();
    }

    fn draft(&self, collection: &Collection) -> Option<&Draft> {
        if !collection.role.can_edit() {
            return None;
        }
        self.drafts.get(&collection.id)
    }

    /// Whether `id` has changes the server has not had, and then whether
    /// the server moved on since too (server-sync §3.1).
    pub fn unsynced(&self, id: &CollectionId) -> Option<bool> {
        let collection = self.listed.iter().find(|collection| &collection.id == id)?;
        let draft = self.draft(collection)?;
        let step = sync::step(Moment::Look, draft.base, collection.revision, true);
        Some(step == Step::KeepUnsyncedBehind)
    }

    /// Whether `id`'s draft is on its way to the server.
    pub fn syncing(&self, id: &CollectionId) -> bool {
        self.syncing.contains(id)
    }

    /// In store order; the view filters and sorts (F2.5 / F2.6).
    pub fn collections(&self) -> &[Collection] {
        &self.collections
    }

    pub fn load(&self) -> &Load {
        &self.load
    }

    /// How many APIs `id` holds, when that was read.
    pub fn api_count(&self, id: &CollectionId) -> Option<usize> {
        if let Some(collection) = self.listed.iter().find(|collection| &collection.id == id)
            && let Some(draft) = self.draft(collection)
        {
            return Some(draft.config.apis.len());
        }
        let counts = self.api_counts.as_ref()?;
        Some(counts.get(id).copied().unwrap_or_default())
    }

    /// F2.1.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load = Load::Loading;
        cx.notify();
        run_store(cx, self.storage.collections().list(), |this, result, cx| {
            match result {
                Ok(collections) => {
                    this.listed = collections;
                    this.read_drafts();
                    this.load = Load::Loaded;
                }
                Err(error) => this.load = Load::Failed(error),
            }
            cx.notify();
        });
        run_store(cx, self.storage.collections().api_counts(), |this, result, cx| {
            match result {
                Ok(counts) => this.api_counts = Some(counts),
                // The cards go without their counts.
                Err(error) => warn!(%error, "the projects' API counts could not be read"),
            }
            cx.notify();
        });
    }

    /// F2.2.
    pub fn create(&mut self, input: NewCollection, cx: &mut Context<Self>) {
        run_store(
            cx,
            self.storage.collections().create(input),
            |this, result, cx| match result {
                Ok(collection) => {
                    this.listed.push(collection.clone());
                    this.show();
                    cx.emit(DashboardEvent::Created(collection));
                    cx.notify();
                }
                Err(error) => cx.emit(DashboardEvent::Failed(error)),
            },
        );
    }

    /// F2.3. A server project's name and description are part of what
    /// syncs (server-sync §2): the change goes into its draft, made from the
    /// server's content when there is none yet.
    pub fn update(&mut self, id: CollectionId, edit: CollectionEdit, cx: &mut Context<Self>) {
        if self.scope.is_some() {
            return self.update_draft(id, edit, cx);
        }
        run_store(
            cx,
            self.storage.collections().update(id, edit),
            |this, result, cx| match result {
                Ok(updated) => {
                    if let Some(found) = this.listed.iter_mut().find(|found| found.id == updated.id) {
                        *found = updated;
                    }
                    this.show();
                    cx.notify();
                }
                Err(error) => cx.emit(DashboardEvent::Failed(error)),
            },
        );
    }

    /// F2.4. The view asks first.
    pub fn delete(&mut self, id: CollectionId, cx: &mut Context<Self>) {
        let deleted = id.clone();
        run_store(
            cx,
            self.storage.collections().delete(id),
            move |this, result, cx| match result {
                Ok(()) => {
                    this.listed.retain(|found| found.id != deleted);
                    this.drafts.remove(&deleted);
                    this.show();
                    cx.emit(DashboardEvent::Deleted(deleted));
                    cx.notify();
                }
                Err(error) => cx.emit(DashboardEvent::Failed(error)),
            },
        );
    }

    fn update_draft(&mut self, id: CollectionId, edit: CollectionEdit, cx: &mut Context<Self>) {
        let name = match rules::required_name(&edit.name) {
            Ok(name) => name,
            Err(error) => return cx.emit(DashboardEvent::Failed(error)),
        };
        let description = edit.description;
        if let Some(mut draft) = self.drafts.get(&id).cloned() {
            draft.config.name = name;
            draft.config.description = description;
            return self.keep_draft(id, draft, cx);
        }
        let call = self.storage.tree().config(id.clone());
        run_store(cx, call, move |this, result, cx| match result {
            Ok(at) => {
                let draft = Draft {
                    base: Some(at.revision),
                    config: ProjectConfig {
                        name,
                        description,
                        ..at.config
                    },
                };
                this.keep_draft(id, draft, cx);
            }
            Err(error) => cx.emit(DashboardEvent::Failed(error)),
        });
    }

    /// Kept in this browser; when it cannot be, it still shows until the
    /// page goes (server-sync §4).
    fn keep_draft(&mut self, id: CollectionId, draft: Draft, cx: &mut Context<Self>) {
        let Some(scope) = self.scope.as_deref() else { return };
        let kept = drafts_local::set(scope, &id, &draft);
        self.drafts.insert(id, draft);
        self.show();
        cx.emit(if kept {
            DashboardEvent::Unsynced
        } else {
            DashboardEvent::NotKept
        });
        cx.notify();
    }

    /// Pushes `id`'s draft (server-sync §3.3): only where the server is
    /// still at its base, or `over` whatever it has.
    pub fn sync(&mut self, id: &CollectionId, over: bool, cx: &mut Context<Self>) {
        let Some(collection) = self.listed.iter().find(|collection| &collection.id == id).cloned() else {
            return;
        };
        let Some(draft) = self.draft(&collection).cloned() else {
            return;
        };
        if !self.syncing.insert(id.clone()) {
            return;
        }
        let (server, id) = (self.storage.clone(), id.clone());
        let call = if over {
            copy::push_over(server, id.clone(), draft.config.clone())
        } else {
            copy::push(server, id.clone(), draft.config.clone(), draft.base)
        };
        run_store(cx, call, move |this, result, cx| {
            this.syncing.remove(&id);
            match result {
                Ok(Pushed::Written(_)) => {
                    // Unless it changed in another tab meanwhile.
                    if let Some(scope) = this.scope.as_deref()
                        && drafts_local::get(scope, &id).as_ref() == Some(&draft)
                    {
                        drafts_local::remove(scope, &id);
                    }
                    cx.emit(DashboardEvent::Synced(collection));
                    this.reload(cx);
                }
                Ok(Pushed::Conflict) => cx.emit(DashboardEvent::Conflict(collection)),
                Err(error) => cx.emit(DashboardEvent::Failed(error)),
            }
            cx.notify();
        });
        cx.notify();
    }

    /// Gives `id`'s unsynced changes up: it opens as the server has it.
    pub fn discard(&mut self, id: &CollectionId, cx: &mut Context<Self>) {
        if let Some(scope) = self.scope.as_deref() {
            drafts_local::remove(scope, id);
        }
        self.drafts.remove(id);
        self.show();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::access::CollectionRole;
    use cyberapi_store::MemoryStore;
    use gpui_kit::{AppContext as _, Entity, TestAppContext};

    const READER: &str = "bob";

    async fn project(storage: &Storage, name: &str) -> Collection {
        storage
            .collections()
            .create(NewCollection {
                name: name.into(),
                ..Default::default()
            })
            .await
            .expect("create")
    }

    fn names(state: &Entity<DashboardState>, cx: &mut TestAppContext) -> Vec<String> {
        state.read_with(cx, |state, _| {
            state.collections().iter().map(|found| found.name.clone()).collect()
        })
    }

    fn edit(name: &str) -> CollectionEdit {
        CollectionEdit {
            name: name.into(),
            description: "d".into(),
        }
    }

    /// Server-sync §2–4: a server project's name is part of what syncs.
    #[gpui_kit::test]
    async fn a_server_projects_name_waits_for_its_sync(cx: &mut TestAppContext) {
        let server = MemoryStore::new().storage();
        let shop = project(&server, "shop").await;
        let state = cx.new(|cx| DashboardState::new(server.clone(), Some(READER.into()), cx));
        cx.run_until_parked();

        state.update(cx, |state, cx| state.update(shop.id.clone(), edit(" store "), cx));
        cx.run_until_parked();
        assert_eq!(names(&state, cx), ["store"]);
        assert_eq!(state.read_with(cx, |state, _| state.unsynced(&shop.id)), Some(false));
        let on_server = server.collections().get(shop.id.clone()).await.expect("get");
        assert_eq!(on_server.name, "shop", "not on the server yet");
        // A blank one is refused, as the store refuses it.
        state.update(cx, |state, cx| state.update(shop.id.clone(), edit(" "), cx));
        cx.run_until_parked();
        assert_eq!(names(&state, cx), ["store"]);

        // Someone else moves the server on: the card says so.
        server
            .tree()
            .create_api(shop.id.clone(), None, "theirs".into())
            .await
            .expect("create");
        state.update(cx, |state, cx| state.reload(cx));
        cx.run_until_parked();
        assert_eq!(state.read_with(cx, |state, _| state.unsynced(&shop.id)), Some(true));
        assert_eq!(
            state.read_with(cx, |state, _| state.api_count(&shop.id)),
            Some(0),
            "the draft's"
        );

        // A push at the stale base is a conflict; over it, it is written.
        state.update(cx, |state, cx| state.sync(&shop.id, false, cx));
        cx.run_until_parked();
        assert_eq!(
            server.collections().get(shop.id.clone()).await.expect("get").name,
            "shop"
        );
        state.update(cx, |state, cx| state.sync(&shop.id, true, cx));
        cx.run_until_parked();
        let on_server = server.collections().get(shop.id.clone()).await.expect("get");
        assert_eq!(
            (on_server.name.as_str(), on_server.description.as_str()),
            ("store", "d")
        );
        assert_eq!(state.read_with(cx, |state, _| state.unsynced(&shop.id)), None);
        assert_eq!(drafts_local::get(READER, &shop.id), None);
    }

    #[gpui_kit::test]
    async fn a_draft_given_up_shows_the_servers(cx: &mut TestAppContext) {
        let server = MemoryStore::new().storage();
        let shop = project(&server, "shop").await;
        let state = cx.new(|cx| DashboardState::new(server.clone(), Some(READER.into()), cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.update(shop.id.clone(), edit("store"), cx));
        cx.run_until_parked();

        state.update(cx, |state, cx| state.discard(&shop.id, cx));
        assert_eq!(names(&state, cx), ["shop"]);
        assert_eq!(drafts_local::get(READER, &shop.id), None);
    }

    #[gpui_kit::test]
    async fn a_viewers_card_has_no_draft(cx: &mut TestAppContext) {
        let server = MemoryStore::new().storage();
        let shop = project(&server, "shop").await;
        drafts_local::set(
            READER,
            &shop.id,
            &Draft {
                base: Some(shop.revision),
                config: ProjectConfig {
                    name: "mine".into(),
                    ..Default::default()
                },
            },
        );
        let state = cx.new(|cx| DashboardState::new(server.clone(), Some(READER.into()), cx));
        cx.run_until_parked();
        assert_eq!(names(&state, cx), ["mine"]);

        state.update(cx, |state, _| {
            state.listed[0].role = CollectionRole::Viewer;
            state.show();
        });
        assert_eq!(names(&state, cx), ["shop"]);
        assert_eq!(state.read_with(cx, |state, _| state.unsynced(&shop.id)), None);
    }

    /// The desktop writes a project's name at once.
    #[gpui_kit::test]
    async fn without_drafts_a_name_is_written_at_once(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let shop = project(&storage, "shop").await;
        let state = cx.new(|cx| DashboardState::new(storage.clone(), None, cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.update(shop.id.clone(), edit("store"), cx));
        cx.run_until_parked();
        assert_eq!(
            storage.collections().get(shop.id.clone()).await.expect("get").name,
            "store"
        );
        assert_eq!(state.read_with(cx, |state, _| state.unsynced(&shop.id)), None);
    }
}
