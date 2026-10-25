//! The project list (M2).
//!
//! Projects may be a server's (`docs/server-sync.md`), and a reader's
//! unsynced changes to one wait: in a page, in a draft in this browser
//! (§3–4); on the desktop, in its copy in the database, bound to the
//! signed-in account (§6). A card shows the changes' name and says they are
//! unsynced, and syncs them or gives them up without opening the project.
//! A change of name or description is one of those changes.
//!
//! The desktop lists its own projects, and — signed in — the account's
//! server projects, each copied here when it is first listed and refreshed
//! while it has nothing unsynced. Another account's copies, or any while
//! signed out, are not listed; offline, the account's copies show as kept.

use super::copy::{self, Pushed};
use super::{Load, ServerState, ServerStatus, run_store};
use crate::drafts_local::{self, Draft};
use crate::{local_items, pins_local};
use cyberapi_core::model::Collection;
use cyberapi_core::sync::{self, CopyBinding, Moment, ProjectConfig, Step};
use cyberapi_core::{AccountId, CollectionId};
use cyberapi_store::{CollectionEdit, NewCollection, ServerCopyStore, Storage, StoreError, StoreFuture, rules};
use gpui_kit::{Context, Entity, EventEmitter, Subscription};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
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
    /// A change now waits here for the reader to sync it.
    Unsynced,
    /// This browser could not keep a change: a refresh loses it.
    NotKept,
    /// The server has the project's changes now.
    Synced(Collection),
    /// The server moved on since the changes' base: the reader chooses.
    Conflict(Collection),
}

/// The desktop's server, signed in, as the list needs it.
#[derive(Clone)]
struct Desktop {
    server: Storage,
    copies: Arc<dyn ServerCopyStore>,
    account: AccountId,
}

impl Desktop {
    /// Whose own items the copies hold (`crate::local_items`).
    fn scope(&self) -> String {
        pins_local::scope(Some(self.account.as_str()))
    }
}

pub struct DashboardState {
    storage: Storage,
    /// As the store lists them.
    listed: Vec<Collection>,
    /// As they show: a draft's name over the server's; the desktop's copies
    /// as the server has them, and only the signed-in account's.
    collections: Vec<Collection>,
    /// How many APIs each project holds, for its card; read beside the
    /// list, and `None` until it was.
    api_counts: Option<BTreeMap<CollectionId, usize>>,
    load: Load,
    /// Whose drafts (`crate::pins_local::scope`); a page's.
    scope: Option<String>,
    drafts: BTreeMap<CollectionId, Draft>,
    /// The desktop's server.
    server_state: Option<Entity<ServerState>>,
    /// Signed in there.
    desktop: Option<Desktop>,
    /// The server answered who is signed in: its projects can be listed.
    reached: bool,
    /// Every account's copies' bindings, as the server state has them.
    bound: Vec<CopyBinding>,
    /// The account's projects as its server lists them; `None` until read,
    /// or when it could not be.
    on_server: Option<Vec<Collection>>,
    /// Changes on their way to the server.
    syncing: BTreeSet<CollectionId>,
    _server: Option<Subscription>,
}

impl EventEmitter<DashboardEvent> for DashboardState {}

impl DashboardState {
    /// `scope`: whose drafts a page's projects have. `server`: the
    /// desktop's server, whose projects it lists beside its own.
    pub fn new(
        storage: Storage,
        scope: Option<String>,
        server: Option<Entity<ServerState>>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Signed in or out, copies made, synced or edited.
        let watch = server
            .as_ref()
            .map(|server| cx.observe(server, |this, _, cx| this.server_changed(cx)));
        let mut state = Self {
            storage,
            listed: Vec::new(),
            collections: Vec::new(),
            api_counts: None,
            load: Load::Loading,
            scope,
            drafts: BTreeMap::new(),
            server_state: server,
            desktop: None,
            reached: false,
            bound: Vec::new(),
            on_server: None,
            syncing: BTreeSet::new(),
            _server: watch,
        };
        state.read_server(cx);
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

    /// Why the server of `id`, a copy of a server project, cannot be
    /// reached; `None` for one it reaches, and for a project of this
    /// machine's.
    pub fn unreachable(&self, id: &CollectionId, cx: &gpui_kit::App) -> Option<String> {
        let server = self.server_state.as_ref()?.read(cx);
        server.mine(id)?;
        match server.status() {
            ServerStatus::Unreachable(reason) => Some(reason.clone()),
            _ => None,
        }
    }

    /// What a sync said of reaching the desktop's server, for its status.
    fn heard_from_server(&self, error: Option<&StoreError>, cx: &mut Context<Self>) {
        let Some(server) = self.server_state.clone() else {
            return;
        };
        let unreachable = match error {
            Some(StoreError::Backend { message }) => Some(message.clone()),
            _ => None,
        };
        server.update(cx, |server, cx| server.heard(unreachable.as_deref(), cx));
    }

    /// The desktop's server changed: signed in or out, or its copies'
    /// bindings. Another account lists again.
    pub fn server_changed(&mut self, cx: &mut Context<Self>) {
        let before = self.desktop.as_ref().map(|desktop| desktop.account.clone());
        let unreached = !self.reached;
        self.read_server(cx);
        let after = self.desktop.as_ref().map(|desktop| desktop.account.clone());
        // Another account lists again, and so does one whose server came
        // back within reach.
        if before != after || (unreached && self.reached) {
            self.on_server = None;
            self.reload(cx);
        } else {
            self.show();
            cx.notify();
        }
    }

    fn read_server(&mut self, cx: &mut Context<Self>) {
        let Some(server) = &self.server_state else { return };
        let server = server.read(cx);
        self.bound = server.bound().to_vec();
        self.reached = matches!(server.status(), ServerStatus::SignedIn(_));
        self.desktop = (|| {
            Some(Desktop {
                server: server.storage()?,
                copies: server.copies()?.clone(),
                account: server.account()?,
            })
        })();
    }

    fn read_drafts(&mut self) {
        self.drafts = self.scope.as_deref().map(drafts_local::all).unwrap_or_default();
        self.show();
    }

    /// The signed-in account's copy `id`, on the desktop.
    fn mine(&self, id: &CollectionId) -> Option<&CopyBinding> {
        let account = &self.desktop.as_ref()?.account;
        self.bound
            .iter()
            .find(|binding| &binding.collection == id && &binding.account == account)
    }

    /// The list as it shows.
    fn show(&mut self) {
        self.collections = self
            .listed
            .iter()
            .filter_map(|collection| {
                if let Some(draft) = self.draft(collection) {
                    return Some(Collection {
                        name: draft.config.name.clone(),
                        description: draft.config.description.clone(),
                        ..collection.clone()
                    });
                }
                let bound = self.bound.iter().any(|binding| binding.collection == collection.id);
                match self.mine(&collection.id) {
                    // This machine's own.
                    None if !bound => Some(collection.clone()),
                    // Another account's, or any while signed out.
                    None => None,
                    Some(binding) => match &self.on_server {
                        // Offline: as kept.
                        None => Some(collection.clone()),
                        // Gone from the server, or from the account's
                        // reach: not listed.
                        Some(projects) => {
                            let theirs = projects.iter().find(|project| project.id == collection.id)?;
                            let mine = if binding.unsynced { collection } else { theirs };
                            Some(Collection {
                                name: mine.name.clone(),
                                description: mine.description.clone(),
                                ..theirs.clone()
                            })
                        }
                    },
                }
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
    /// the server moved on since too (server-sync §3.1). A viewer has none.
    pub fn unsynced(&self, id: &CollectionId) -> Option<bool> {
        let collection = self.collections.iter().find(|collection| &collection.id == id)?;
        if !collection.role.can_edit() {
            return None;
        }
        let (base, revision) = match self.drafts.get(id) {
            Some(draft) => (draft.base, collection.revision),
            None => {
                let binding = self.mine(id).filter(|binding| binding.unsynced)?;
                let on_server = self
                    .on_server
                    .as_ref()
                    .and_then(|projects| projects.iter().find(|project| &project.id == id));
                match on_server {
                    Some(project) => (binding.base, project.revision),
                    // Offline: unsynced, and nothing known of the server.
                    None => return Some(false),
                }
            }
        };
        Some(sync::step(Moment::Look, base, revision, true) == Step::KeepUnsyncedBehind)
    }

    /// Whether `id`'s changes are on their way to the server.
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

    /// F2.1; on the desktop signed in, the server's projects after this
    /// machine's.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load = Load::Loading;
        cx.notify();
        if let Some(server) = &self.server_state {
            server.update(cx, |server, cx| server.reload_copies(cx));
        }
        self.reload_listed(true, cx);
    }

    fn reload_listed(&mut self, then_server: bool, cx: &mut Context<Self>) {
        run_store(cx, self.storage.collections().list(), move |this, result, cx| {
            match result {
                Ok(collections) => {
                    this.listed = collections;
                    this.read_drafts();
                    this.load = Load::Loaded;
                    if then_server {
                        this.reload_server(cx);
                    }
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

    /// The account's server projects (server-sync §3.1, §6): each copied
    /// here when it is not yet, and refreshed when the server moved on and
    /// the copy has nothing unsynced.
    fn reload_server(&mut self, cx: &mut Context<Self>) {
        let Some(desktop) = self.desktop.clone() else {
            self.on_server = None;
            return;
        };
        run_store(cx, desktop.server.collections().list(), move |this, result, cx| {
            match result {
                Ok(projects) => {
                    let stale: Vec<Collection> =
                        projects.iter().filter(|project| this.stale(project)).cloned().collect();
                    this.on_server = Some(projects);
                    if !stale.is_empty() {
                        this.pull(desktop, stale, None, cx);
                    }
                }
                Err(error) => {
                    warn!(%error, "the server's projects could not be listed");
                    this.on_server = None;
                }
            }
            this.show();
            cx.notify();
        });
    }

    /// Whose own items this list's server projects hold: the page's reader,
    /// or the desktop's account.
    fn local_scope(&self) -> Option<String> {
        self.scope.clone().or_else(|| self.desktop.as_ref().map(Desktop::scope))
    }

    fn stale(&self, project: &Collection) -> bool {
        let here = self.listed.iter().any(|collection| collection.id == project.id);
        match self.mine(&project.id) {
            None => true,
            Some(_) if !here => true,
            Some(binding) => !binding.unsynced && binding.base != Some(project.revision),
        }
    }

    /// Copies `projects` here as the server has them; `then` hears of the
    /// first, made.
    fn pull(&mut self, desktop: Desktop, projects: Vec<Collection>, then: Option<Collection>, cx: &mut Context<Self>) {
        let local = self.storage.clone();
        run_store(cx, pull_copies(desktop, local, projects), move |this, result, cx| {
            match result {
                Ok(bound) => {
                    this.note_bound(bound, cx);
                    if let Some(collection) = then {
                        cx.emit(DashboardEvent::Created(collection));
                    }
                }
                Err(error) => cx.emit(DashboardEvent::Failed(error)),
            }
            this.reload_listed(false, cx);
        });
    }

    /// Bindings the database has now: here and in the server state, so a
    /// copy opens as one at once.
    fn note_bound(&mut self, bound: Vec<CopyBinding>, cx: &mut Context<Self>) {
        for binding in &bound {
            self.bound.retain(|found| found.collection != binding.collection);
            self.bound.push(binding.clone());
        }
        if let Some(server) = &self.server_state {
            server.update(cx, |server, cx| server.note_bound(bound, cx));
        }
        self.show();
        cx.notify();
    }

    /// F2.2: on the desktop signed in, a team's project is made on the
    /// server and copied here (server-sync §2); without a team, it is this
    /// machine's.
    pub fn create(&mut self, input: NewCollection, cx: &mut Context<Self>) {
        if let Some(desktop) = self.desktop.clone()
            && input.owner.is_some()
        {
            let call = desktop.server.collections().create(input);
            return run_store(cx, call, move |this, result, cx| match result {
                Ok(made) => this.pull(desktop, vec![made.clone()], Some(made), cx),
                Err(error) => cx.emit(DashboardEvent::Failed(error)),
            });
        }
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
    /// syncs (server-sync §2): the change goes into its draft (made from the
    /// server's content when there is none yet), or the desktop's copy.
    pub fn update(&mut self, id: CollectionId, edit: CollectionEdit, cx: &mut Context<Self>) {
        if self.scope.is_some() {
            return self.update_draft(id, edit, cx);
        }
        let copy = self.desktop.clone().zip(self.mine(&id).cloned());
        let call = self.storage.collections().update(id, edit);
        run_store(cx, call, move |this, result, cx| match result {
            Ok(updated) => {
                if let Some(found) = this.listed.iter_mut().find(|found| found.id == updated.id) {
                    *found = updated;
                }
                if let Some((desktop, binding)) = copy {
                    this.mark_unsynced(desktop, binding, cx);
                }
                this.show();
                cx.notify();
            }
            Err(error) => cx.emit(DashboardEvent::Failed(error)),
        });
    }

    fn mark_unsynced(&mut self, desktop: Desktop, binding: CopyBinding, cx: &mut Context<Self>) {
        let binding = CopyBinding {
            unsynced: true,
            ..binding
        };
        let call = desktop.copies.bind(binding.clone());
        run_store(cx, call, move |this, result, cx| match result {
            Ok(()) => {
                this.note_bound(vec![binding], cx);
                cx.emit(DashboardEvent::Unsynced);
            }
            Err(error) => cx.emit(DashboardEvent::Failed(error)),
        });
    }

    /// F2.4. The view asks first. The desktop's copy of a server project
    /// goes from the server first, then from here with its binding.
    pub fn delete(&mut self, id: CollectionId, cx: &mut Context<Self>) {
        let deleted = id.clone();
        let local = self.storage.clone();
        let call: StoreFuture<()> = match self.desktop.clone().filter(|_| self.mine(&id).is_some()) {
            Some(desktop) => Box::pin(async move {
                desktop.server.collections().delete(id.clone()).await?;
                local.collections().delete(id).await
            }),
            None => local.collections().delete(id),
        };
        run_store(cx, call, move |this, result, cx| match result {
            Ok(()) => {
                this.listed.retain(|found| found.id != deleted);
                this.drafts.remove(&deleted);
                if let Some(scope) = this.local_scope() {
                    local_items::remove(&scope, &deleted);
                }
                this.bound.retain(|found| found.collection != deleted);
                if let Some(server) = &this.server_state {
                    server.update(cx, |server, cx| server.forget_bound(&deleted, cx));
                }
                this.show();
                cx.emit(DashboardEvent::Deleted(deleted));
                cx.notify();
            }
            Err(error) => cx.emit(DashboardEvent::Failed(error)),
        });
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

    /// Pushes `id`'s changes (server-sync §3.3): only where the server is
    /// still at their base, or `over` whatever it has.
    pub fn sync(&mut self, id: &CollectionId, over: bool, cx: &mut Context<Self>) {
        let Some(collection) = self.collections.iter().find(|collection| &collection.id == id).cloned() else {
            return;
        };
        if self.syncing.contains(id) {
            return;
        }
        if let Some(listed) = self.listed.iter().find(|listed| &listed.id == id).cloned()
            && let Some(draft) = self.draft(&listed).cloned()
        {
            return self.sync_draft(collection, draft, over, cx);
        }
        let (Some(desktop), Some(binding)) = (self.desktop.clone(), self.mine(id).cloned()) else {
            return;
        };
        self.syncing.insert(id.clone());
        let local = self.storage.clone();
        let call: StoreFuture<(Pushed, Option<CopyBinding>)> = Box::pin(async move {
            let config = local.tree().config(binding.collection.clone()).await?.config;
            let (server, id) = (desktop.server.clone(), binding.collection.clone());
            // The reader's own items stay here (server-sync §3.4).
            let scope = desktop.scope();
            let mut own = local_items::get(&scope, &id);
            let config = own.syncable(&config);
            let pushed = if over {
                copy::push_over(server, id.clone(), config.clone()).await?
            } else {
                copy::push(server, id.clone(), config.clone(), binding.base).await?
            };
            let Pushed::Written(revision) = pushed else {
                return Ok((pushed, None));
            };
            own.level_with(&config);
            local_items::set(&scope, &id, &own);
            let level = CopyBinding {
                base: Some(revision),
                unsynced: false,
                ..binding
            };
            desktop.copies.bind(level.clone()).await?;
            Ok((pushed, Some(level)))
        });
        let id = id.clone();
        run_store(cx, call, move |this, result, cx| {
            this.syncing.remove(&id);
            this.heard_from_server(result.as_ref().err(), cx);
            match result {
                Ok((Pushed::Written(_), level)) => {
                    this.note_bound(level.into_iter().collect(), cx);
                    cx.emit(DashboardEvent::Synced(collection));
                    this.reload(cx);
                }
                Ok((Pushed::Conflict, _)) => cx.emit(DashboardEvent::Conflict(collection)),
                Err(error) => cx.emit(DashboardEvent::Failed(error)),
            }
            cx.notify();
        });
        cx.notify();
    }

    fn sync_draft(&mut self, collection: Collection, draft: Draft, over: bool, cx: &mut Context<Self>) {
        let (server, id) = (self.storage.clone(), collection.id.clone());
        self.syncing.insert(id.clone());
        // The reader's own items stay in this browser (server-sync §3.4).
        let scope = self.scope.clone().unwrap_or_default();
        let mut own = local_items::get(&scope, &id);
        let config = own.syncable(&draft.config);
        let call = if over {
            copy::push_over(server, id.clone(), config.clone())
        } else {
            copy::push(server, id.clone(), config.clone(), draft.base)
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
                    own.level_with(&config);
                    local_items::set(&scope, &id, &own);
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

    /// Gives `id`'s unsynced changes up: a page's draft goes, and it opens
    /// as the server has it; the desktop's copy is pulled again — when the
    /// server can be reached, else it stays.
    pub fn discard(&mut self, id: &CollectionId, cx: &mut Context<Self>) {
        if let Some(scope) = self.scope.as_deref() {
            drafts_local::remove(scope, id);
            self.drafts.remove(id);
            self.show();
            return cx.notify();
        }
        let (Some(desktop), Some(collection)) = (
            self.desktop.clone(),
            self.collections.iter().find(|collection| &collection.id == id).cloned(),
        ) else {
            return;
        };
        self.pull(desktop, vec![collection], None, cx);
    }
}

/// The server's `projects` as copies here in `local`, level with it, the
/// reader's own items put back (server-sync §3.4): what they bind.
fn pull_copies(desktop: Desktop, local: Storage, projects: Vec<Collection>) -> StoreFuture<Vec<CopyBinding>> {
    Box::pin(async move {
        let mut bound = Vec::new();
        let scope = desktop.scope();
        for project in projects {
            let at = desktop.server.tree().config(project.id.clone()).await?;
            let mut own = local_items::get(&scope, &project.id);
            desktop
                .copies
                .keep_copy(project.clone(), own.restore(at.config))
                .await?;
            let held = local.tree().config(project.id.clone()).await?.config;
            own.keep(&held);
            own.level_with(&own.syncable(&held));
            local_items::set(&scope, &project.id, &own);
            let binding = CopyBinding {
                collection: project.id,
                account: desktop.account.clone(),
                base: Some(at.revision),
                unsynced: false,
            };
            desktop.copies.bind(binding.clone()).await?;
            bound.push(binding);
        }
        Ok(bound)
    })
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
        let state = cx.new(|cx| DashboardState::new(server.clone(), Some(READER.into()), None, cx));
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

    /// Server-sync §3.4: a draft synced from its card leaves the reader's
    /// own items in this browser.
    #[gpui_kit::test]
    async fn a_cards_sync_leaves_the_readers_own_out(cx: &mut TestAppContext) {
        use cyberapi_core::local_only::LocalItems;
        use cyberapi_core::model::{ApiSetting, Folder};

        let server = MemoryStore::new().storage();
        let shop = project(&server, "shop").await;
        let at = server.tree().config(shop.id.clone()).await.expect("config");
        let mut draft = at.config.clone();
        draft.folders.push(Folder {
            id: "mine".into(),
            collection: shop.id.clone(),
            name: "mine".into(),
            ..Default::default()
        });
        draft.apis.push(ApiSetting {
            id: "shared".into(),
            collection: shop.id.clone(),
            name: "shared".into(),
            ..Default::default()
        });
        draft.top_order = vec!["mine".into(), "shared".into()];
        drafts_local::set(
            READER,
            &shop.id,
            &Draft {
                base: Some(at.revision),
                config: draft.clone(),
            },
        );
        let mut own = LocalItems::default();
        own.level_with(&at.config);
        own.mark(&"mine".into());
        own.keep(&draft);
        local_items::set(READER, &shop.id, &own);

        let state = cx.new(|cx| DashboardState::new(server.clone(), Some(READER.into()), None, cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.sync(&shop.id, false, cx));
        cx.run_until_parked();
        let synced = server.tree().config(shop.id.clone()).await.expect("config").config;
        assert!(synced.folders.is_empty(), "the reader's folder stays here");
        assert_eq!(synced.top_order, vec![cyberapi_core::ItemId::from("shared")]);
        assert_eq!(
            local_items::get(READER, &shop.id).kept.folders.len(),
            1,
            "kept for the next pull"
        );
    }

    #[gpui_kit::test]
    async fn a_draft_given_up_shows_the_servers(cx: &mut TestAppContext) {
        let server = MemoryStore::new().storage();
        let shop = project(&server, "shop").await;
        let state = cx.new(|cx| DashboardState::new(server.clone(), Some(READER.into()), None, cx));
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
        let state = cx.new(|cx| DashboardState::new(server.clone(), Some(READER.into()), None, cx));
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
        let state = cx.new(|cx| DashboardState::new(storage.clone(), None, None, cx));
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
