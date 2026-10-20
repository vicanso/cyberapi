//! One project's workspace (M3, M12): its tree, and what the reader keeps of
//! it — open folders, the selected API, pins.
//!
//! The tree's rules are `cyberapi_core::tree`'s. A change is the store's
//! answer applied to the copy here — the same plan the store wrote — so the
//! copy never guesses. Changes go one at a time ([`run_serial`]) so their
//! answers apply in the order the store made them.
//!
//! In the browser build other readers change the project too: the state
//! polls its revision and loads the tree again when it moved (§7).

use super::{ApiState, Load, run_serial, run_store};
use crate::pacing;
use async_lock::Mutex;
use cyberapi_core::ItemId;
use cyberapi_core::model::{ApiSetting, Collection, Folder, Variable, VariableCategory};
use cyberapi_core::prefs::{RequestTab, WorkspacePrefs};
use cyberapi_core::tree::{Placement, Row, Tree, TreeData};
use cyberapi_service::Sender;
use cyberapi_store::{Storage, StoreError};
use gpui_kit::{AppContext as _, Context, Entity, EventEmitter, Task};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

/// What the view should tell the reader about.
#[derive(Clone, Debug)]
pub enum WorkspaceEvent {
    /// A change the reader asked for did not happen.
    Failed(StoreError),
}

pub struct WorkspaceState {
    sender: Sender,
    collection: Collection,
    data: TreeData,
    tree: Tree,
    prefs: WorkspacePrefs,
    load: Load,
    keyword: String,
    /// The tree's changes, one at a time.
    gate: Arc<Mutex<()>>,
    /// The selected API's editor, while one is selected.
    api: Option<Entity<ApiState>>,
    /// The project's environment variables, for the URI's prefix (F4.4).
    environment: Vec<Variable>,
    _poll: Option<Task<()>>,
}

impl EventEmitter<WorkspaceEvent> for WorkspaceState {}

impl WorkspaceState {
    /// F2.7: the tree and the reader's state of it. `sender` holds the
    /// stores and the engine the selected API sends with.
    pub fn new(sender: Sender, collection: Collection, cx: &mut Context<Self>) -> Self {
        Self::with_poll(sender, collection, pacing::REVISION_POLL, cx)
    }

    /// Asks every `poll` whether another reader changed the project.
    fn with_poll(sender: Sender, collection: Collection, poll: Option<Duration>, cx: &mut Context<Self>) -> Self {
        let mut state = Self {
            sender,
            collection,
            data: TreeData::default(),
            tree: Tree::default(),
            prefs: WorkspacePrefs::default(),
            load: Load::Loading,
            keyword: String::new(),
            gate: Arc::default(),
            api: None,
            environment: Vec::new(),
            _poll: poll.map(|every| Self::poll(every, cx)),
        };
        state.reload(cx);
        state
    }

    fn poll(every: Duration, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut beats = 0u32;
            loop {
                cx.background_executor().timer(every).await;
                beats = beats.wrapping_add(1);
                if pacing::page_hidden() && !beats.is_multiple_of(pacing::HIDDEN_EVERY) {
                    continue;
                }
                let Ok(call) = this.update(cx, |this, _| {
                    this.sender.storage.collections().get(this.collection.id.clone())
                }) else {
                    return;
                };
                let result = cx.background_spawn(call).await;
                if this.update(cx, |this, cx| this.revision_read(result, cx)).is_err() {
                    return;
                }
            }
        })
    }

    /// A moved revision means someone changed the project: its tree loads
    /// again, behind any change of this reader's still on its way. Their own
    /// changes move it too, which costs one load.
    fn revision_read(&mut self, result: Result<Collection, StoreError>, cx: &mut Context<Self>) {
        let collection = match result {
            Ok(collection) => collection,
            // Asked again next beat.
            Err(error) => return warn!(%error, "the project's revision could not be read"),
        };
        if collection.revision == self.collection.revision {
            return;
        }
        self.collection = collection;
        self.load_environment(cx);
        let call = self.sender.storage.tree().load(self.collection.id.clone());
        run_serial(cx, &self.gate, call, |this, result, cx| match result {
            Ok(data) => {
                this.set_data(data, cx);
                cx.notify();
            }
            Err(error) => warn!(%error, "the project's tree could not be read again"),
        });
    }

    pub fn collection(&self) -> &Collection {
        &self.collection
    }

    pub fn storage(&self) -> &Storage {
        &self.sender.storage
    }

    /// The selected API's editor.
    pub fn api(&self) -> Option<&Entity<ApiState>> {
        self.api.as_ref()
    }

    /// F4.4: the enabled environment variables, by name.
    pub fn environments(&self) -> impl Iterator<Item = &Variable> {
        self.environment.iter().filter(|variable| variable.enabled)
    }

    fn load_environment(&mut self, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .variables()
            .list(self.collection.id.clone(), VariableCategory::Environment);
        run_store(cx, call, |this, result, cx| match result {
            Ok(variables) => {
                this.environment = variables;
                cx.notify();
            }
            Err(error) => warn!(%error, "the environment variables could not be read"),
        });
    }

    /// F4.12: the tab the selected API's editor shows.
    pub fn tab(&self, cx: &gpui_kit::App) -> Option<RequestTab> {
        let api = self.api.as_ref()?.read(cx);
        Some(self.prefs.tab(api.api(), api.request().method))
    }

    pub fn set_tab(&mut self, tab: RequestTab, cx: &mut Context<Self>) {
        let Some(api) = self.api.as_ref() else { return };
        let (id, method) = {
            let api = api.read(cx);
            (api.api().clone(), api.request().method)
        };
        if self.prefs.tab(&id, method) != tab {
            self.prefs.set_tab(&id, method, tab);
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// Opens the editor of the selected API, or closes it when none is
    /// selected (or the selected one is gone). The editor of an API that
    /// stays selected is kept, with what it holds.
    fn sync_api(&mut self, cx: &mut Context<Self>) {
        let selected = self.selected_api().cloned();
        let current = self.api.as_ref().map(|api| api.read(cx).api().clone());
        match selected {
            Some(api) if current.as_ref() != Some(&api.id) => {
                let (sender, workspace) = (self.sender.clone(), cx.entity().downgrade());
                self.api = Some(cx.new(|cx| ApiState::new(sender, workspace, &api, cx)));
            }
            Some(_) => {}
            None => self.api = None,
        }
    }

    /// An API's request was saved: the tree's copy follows (its method label).
    pub fn api_saved(&mut self, setting: ApiSetting, cx: &mut Context<Self>) {
        if let Some(api) = self.data.apis.iter_mut().find(|api| api.id == setting.id) {
            *api = setting;
            self.tree = Tree::build(&self.data);
            cx.notify();
        }
    }

    pub fn load(&self) -> &Load {
        &self.load
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    pub fn keyword(&self) -> &str {
        &self.keyword
    }

    pub fn selected(&self) -> Option<&ItemId> {
        self.prefs.selected.as_ref()
    }

    /// The selected API's record, when it still exists.
    pub fn selected_api(&self) -> Option<&ApiSetting> {
        let selected = self.prefs.selected.as_ref()?;
        self.data.apis.iter().find(|api| &api.id == selected)
    }

    /// Pinned APIs that still exist, in pin order (F12.2).
    pub fn pinned(&self) -> Vec<&ApiSetting> {
        self.prefs
            .pinned
            .iter()
            .filter_map(|id| self.data.apis.iter().find(|api| &api.id == id))
            .collect()
    }

    pub fn is_pinned(&self, id: &ItemId) -> bool {
        self.prefs.pinned.contains(id)
    }

    /// The rows to draw under the current keyword and open folders.
    pub fn rows(&self) -> Vec<Row> {
        let expanded: HashSet<ItemId> = self.prefs.expanded.iter().cloned().collect();
        self.tree.rows(&expanded, &self.keyword)
    }

    /// Loads (or loads again) the tree and the reader's state of it.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load_environment(cx);
        let id = self.collection.id.clone();
        run_store(cx, self.sender.storage.tree().load(id.clone()), |this, result, cx| {
            match result {
                Ok(data) => {
                    this.set_data(data, cx);
                    this.load = Load::Loaded;
                }
                Err(error) => this.load = Load::Failed(error),
            }
            cx.notify();
        });
        run_store(cx, self.sender.storage.prefs().workspace(id), |this, result, cx| {
            match result {
                Ok(prefs) => {
                    this.prefs = prefs;
                    this.sync_api(cx);
                }
                // The tree still works without them; the reader starts from
                // a closed tree.
                Err(error) => warn!(%error, "the workspace state could not be read"),
            }
            cx.notify();
        });
    }

    fn set_data(&mut self, data: TreeData, cx: &mut Context<Self>) {
        self.data = data;
        self.tree = Tree::build(&self.data);
        self.sync_api(cx);
    }

    /// F3.2.
    pub fn set_keyword(&mut self, keyword: String, cx: &mut Context<Self>) {
        if self.keyword != keyword {
            self.keyword = keyword;
            cx.notify();
        }
    }

    /// F3.3.
    pub fn toggle_folder(&mut self, id: &ItemId, cx: &mut Context<Self>) {
        if let Some(at) = self.prefs.expanded.iter().position(|open| open == id) {
            self.prefs.expanded.remove(at);
        } else {
            self.prefs.expanded.push(id.clone());
        }
        self.save_prefs(cx);
        cx.notify();
    }

    /// F3.3: "collapse all" closes every folder of the project.
    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        if !self.prefs.expanded.is_empty() {
            self.prefs.expanded.clear();
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// F3.8: an API is selected; a folder is opened or closed instead. The
    /// folders it sits in open, so a pin (F12.2) shows its row.
    pub fn select(&mut self, id: &ItemId, cx: &mut Context<Self>) {
        let mut changed = self.prefs.selected.as_ref() != Some(id);
        self.prefs.selected = Some(id.clone());
        let mut parent = self.tree.get(id).and_then(|node| node.parent()).cloned();
        while let Some(folder) = parent {
            parent = self.tree.get(&folder).and_then(|node| node.parent()).cloned();
            if !self.prefs.expanded.contains(&folder) {
                self.prefs.expanded.push(folder);
                changed = true;
            }
        }
        if changed {
            self.sync_api(cx);
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// F12.1 / F12.3. Only this project's APIs, once each.
    pub fn set_pinned(&mut self, id: &ItemId, pinned: bool, cx: &mut Context<Self>) {
        let present = self.prefs.pinned.contains(id);
        if pinned && !present && self.data.apis.iter().any(|api| &api.id == id) {
            self.prefs.pinned.push(id.clone());
        } else if !pinned && present {
            self.prefs.pinned.retain(|pin| pin != id);
        } else {
            return;
        }
        self.save_prefs(cx);
        cx.notify();
    }

    /// F3.4: a new API is selected, and the folder it went into opens.
    pub fn create_api(&mut self, parent: Option<ItemId>, name: String, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .tree()
            .create_api(self.collection.id.clone(), parent.clone(), name);
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(inserted) => {
                this.data.apis.push(inserted.item.clone());
                this.data.apply(&inserted.change);
                this.tree = Tree::build(&this.data);
                this.open_folder(parent.as_ref());
                this.prefs.selected = Some(inserted.item.id);
                this.sync_api(cx);
                this.save_prefs(cx);
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// F3.4.
    pub fn create_folder(&mut self, parent: Option<ItemId>, name: String, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .tree()
            .create_folder(self.collection.id.clone(), parent.clone(), name);
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(inserted) => {
                let folder: Folder = inserted.item;
                this.data.folders.push(folder);
                this.data.apply(&inserted.change);
                this.tree = Tree::build(&this.data);
                this.open_folder(parent.as_ref());
                this.save_prefs(cx);
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    fn open_folder(&mut self, folder: Option<&ItemId>) {
        if let Some(folder) = folder
            && !self.prefs.expanded.contains(folder)
        {
            self.prefs.expanded.push(folder.clone());
        }
    }

    /// F3.5.
    pub fn rename(&mut self, id: ItemId, name: String, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .tree()
            .rename(self.collection.id.clone(), id.clone(), name.clone());
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(()) => {
                // The store keeps the name trimmed.
                let name = name.trim().to_string();
                if let Some(folder) = this.data.folders.iter_mut().find(|folder| folder.id == id) {
                    folder.name = name;
                } else if let Some(api) = this.data.apis.iter_mut().find(|api| api.id == id) {
                    api.name = name;
                }
                this.tree = Tree::build(&this.data);
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// F3.7.
    pub fn move_item(&mut self, id: ItemId, to: Placement, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .tree()
            .move_item(self.collection.id.clone(), id, to.clone());
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(change) => {
                this.data.apply(&change);
                this.tree = Tree::build(&this.data);
                this.open_folder(to.parent.as_ref());
                this.save_prefs(cx);
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// F3.6. The view asks first. What was removed leaves the reader's state
    /// too: selection, pins, open folders.
    pub fn delete(&mut self, id: ItemId, cx: &mut Context<Self>) {
        let call = self.sender.storage.tree().delete(self.collection.id.clone(), id);
        run_serial(cx, &self.gate, call, move |this, result, cx| match result {
            Ok(removal) => {
                this.data.apply_removal(&removal);
                this.tree = Tree::build(&this.data);
                let gone = |id: &ItemId| removal.folders.contains(id) || removal.apis.contains(id);
                if this.prefs.selected.as_ref().is_some_and(gone) {
                    this.prefs.selected = None;
                }
                this.prefs.pinned.retain(|id| !gone(id));
                this.prefs.expanded.retain(|id| !gone(id));
                this.sync_api(cx);
                this.save_prefs(cx);
                cx.notify();
            }
            Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
        });
    }

    /// The reader's state is saved in the background; a failure costs them
    /// the state next visit, not now, so it is only logged.
    fn save_prefs(&self, cx: &mut Context<Self>) {
        let call = self
            .sender
            .storage
            .prefs()
            .set_workspace(self.collection.id.clone(), self.prefs.clone());
        cx.background_spawn(async move {
            if let Err(error) = call.await {
                warn!(%error, "the workspace state could not be saved");
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::states::ApiEvent;
    use crate::states::api::SAVE_DELAY;
    use cyberapi_core::model::{HttpMethod, HttpRequest, KvParam};
    use cyberapi_core::prefs::Timeouts;
    use cyberapi_core::response::Exchange;
    use cyberapi_core::template::NoFiles;
    use cyberapi_core::tree::TreeError;
    use cyberapi_http::{HttpEngine, PreparedRequest, SendFuture};
    use cyberapi_store::{MemoryStore, NewCollection};
    use gpui_kit::{Entity, TestAppContext};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Answers every request with `200 ok`, or never.
    struct Engine {
        answers: bool,
    }

    impl HttpEngine for Engine {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            if self.answers {
                Box::pin(async {
                    Ok(Exchange {
                        status: 200,
                        body: b"ok".to_vec(),
                        ..Default::default()
                    })
                })
            } else {
                Box::pin(std::future::pending())
            }
        }
    }

    fn sender(storage: &Storage, answers: bool) -> Sender {
        Sender {
            storage: storage.clone(),
            engine: Arc::new(Engine { answers }),
            files: Arc::new(NoFiles("no files in tests")),
            user_agent: "CyberAPI/test".into(),
            timeouts: Timeouts::default(),
        }
    }

    async fn project(storage: &Storage) -> Collection {
        storage
            .collections()
            .create(NewCollection {
                name: "p".into(),
                ..Default::default()
            })
            .await
            .expect("create")
    }

    fn open(storage: &Storage, collection: Collection, cx: &mut TestAppContext) -> Entity<WorkspaceState> {
        let state = cx.new(|cx| WorkspaceState::new(sender(storage, true), collection, cx));
        cx.run_until_parked();
        state
    }

    fn row_names(state: &Entity<WorkspaceState>, cx: &mut TestAppContext) -> Vec<String> {
        state.read_with(cx, |state, _| {
            state
                .rows()
                .iter()
                .map(|row| {
                    let name = state
                        .tree()
                        .get(&row.id)
                        .map(|node| node.name().to_string())
                        .unwrap_or_default();
                    format!("{}{name}", "  ".repeat(row.depth))
                })
                .collect()
        })
    }

    fn find(state: &Entity<WorkspaceState>, name: &str, cx: &mut TestAppContext) -> ItemId {
        state.read_with(cx, |state, _| {
            state
                .tree()
                .roots()
                .flat_map(|root| std::iter::once(root.id().clone()).chain(state.tree().descendants(root.id())))
                .find(|id| state.tree().get(id).is_some_and(|node| node.name() == name))
                .unwrap_or_else(|| panic!("no item named {name}"))
        })
    }

    #[gpui_kit::test]
    async fn a_new_api_is_selected_inside_its_opened_folder(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection, cx);
        state.update(cx, |state, cx| state.create_folder(None, "users".into(), cx));
        cx.run_until_parked();
        let users = find(&state, "users", cx);
        state.update(cx, |state, cx| {
            state.create_api(Some(users.clone()), "login".into(), cx)
        });
        cx.run_until_parked();

        assert_eq!(row_names(&state, cx), ["users", "  login"]);
        let login = find(&state, "login", cx);
        state.read_with(cx, |state, _| {
            assert_eq!(state.selected(), Some(&login));
            assert_eq!(state.selected_api().map(|api| api.name.as_str()), Some("login"));
        });
    }

    #[gpui_kit::test]
    async fn the_readers_state_survives_a_reopening(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_folder(None, "users".into(), cx));
        cx.run_until_parked();
        let users = find(&state, "users", cx);
        state.update(cx, |state, cx| {
            state.create_api(Some(users.clone()), "login".into(), cx)
        });
        cx.run_until_parked();
        let login = find(&state, "login", cx);
        state.update(cx, |state, cx| state.set_pinned(&login, true, cx));
        cx.run_until_parked();

        let again = open(&storage, collection, cx);
        assert_eq!(row_names(&again, cx), ["users", "  login"], "the folder stays open");
        again.read_with(cx, |state, _| {
            assert_eq!(state.selected(), Some(&login));
            assert_eq!(state.pinned().len(), 1);
        });

        again.update(cx, |state, cx| state.collapse_all(cx));
        cx.run_until_parked();
        assert_eq!(row_names(&again, cx), ["users"]);

        // Chosen from its pin, the API shows again.
        again.update(cx, |state, cx| state.select(&login, cx));
        assert_eq!(row_names(&again, cx), ["users", "  login"]);
    }

    #[gpui_kit::test]
    async fn moves_and_keywords_follow_the_core_rules(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection, cx);
        for name in ["a", "b"] {
            state.update(cx, |state, cx| state.create_folder(None, name.into(), cx));
        }
        state.update(cx, |state, cx| state.create_api(None, "登录".into(), cx));
        cx.run_until_parked();
        // New items go first on the top level.
        assert_eq!(row_names(&state, cx), ["登录", "b", "a"]);

        let (a, login) = (find(&state, "a", cx), find(&state, "登录", cx));
        state.update(cx, |state, cx| {
            state.move_item(
                login.clone(),
                Placement {
                    parent: Some(a.clone()),
                    before: None,
                },
                cx,
            )
        });
        cx.run_until_parked();
        assert_eq!(row_names(&state, cx), ["b", "a", "  登录"], "the target folder opens");

        state.update(cx, |state, cx| state.set_keyword("dl".into(), cx));
        assert_eq!(row_names(&state, cx), ["a", "  登录"]);

        // A refused move changes nothing and says why.
        let failures: Rc<RefCell<Vec<StoreError>>> = Rc::default();
        let seen = failures.clone();
        cx.update(|cx| {
            cx.subscribe(&state, move |_, event, _| match event {
                WorkspaceEvent::Failed(error) => seen.borrow_mut().push(error.clone()),
            })
            .detach();
        });
        state.update(cx, |state, cx| state.set_keyword(String::new(), cx));
        state.update(cx, |state, cx| {
            state.move_item(
                a.clone(),
                Placement {
                    parent: Some(a.clone()),
                    before: None,
                },
                cx,
            )
        });
        cx.run_until_parked();
        assert_eq!(row_names(&state, cx), ["b", "a", "  登录"]);
        assert_eq!(
            failures.borrow().as_slice(),
            [StoreError::Tree {
                source: TreeError::Cycle { id: a }
            }]
        );
    }

    #[gpui_kit::test]
    async fn other_readers_changes_show_on_the_next_beat(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let every = Duration::from_secs(10);
        let state = cx.new(|cx| WorkspaceState::with_poll(sender(&storage, true), collection.clone(), Some(every), cx));
        cx.run_until_parked();
        assert!(row_names(&state, cx).is_empty());

        // Someone else, straight through the store.
        storage
            .tree()
            .create_api(collection.id.clone(), None, "theirs".into())
            .await
            .expect("create");
        cx.run_until_parked();
        assert!(row_names(&state, cx).is_empty(), "not before the beat");

        cx.executor().advance_clock(every);
        cx.run_until_parked();
        assert_eq!(row_names(&state, cx), ["theirs"]);
    }

    fn editor(state: &Entity<WorkspaceState>, cx: &mut TestAppContext) -> Entity<ApiState> {
        state.read_with(cx, |state, _| state.api().cloned().expect("an editor"))
    }

    fn stored(storage: &Storage, collection: &Collection, api: &ItemId) -> HttpRequest {
        let data = pollster::block_on(storage.tree().load(collection.id.clone())).expect("load");
        data.apis
            .into_iter()
            .find(|found| &found.id == api)
            .expect("the API")
            .request
    }

    fn header(key: &str, value: &str) -> KvParam {
        KvParam {
            key: key.into(),
            value: value.into(),
            enabled: true,
        }
    }

    #[gpui_kit::test]
    async fn the_editor_saves_at_once_or_after_a_pause(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_api(None, "login".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        let id = api.read_with(cx, |api, _| api.api().clone());

        // F4.2: the method at once, and the tree's label follows.
        api.update(cx, |api, cx| api.set_method(HttpMethod::Post, cx));
        cx.run_until_parked();
        assert_eq!(stored(&storage, &collection, &id).method, HttpMethod::Post);
        state.read_with(cx, |state, _| {
            assert_eq!(state.tree().get(&id).map(|node| node.method()), Some(HttpMethod::Post));
        });

        // F4.11: typing waits; an empty row is not kept (F4.10).
        api.update(cx, |api, cx| {
            api.set_headers(vec![header("X-A", "1"), header(" ", "")], cx)
        });
        cx.run_until_parked();
        assert!(stored(&storage, &collection, &id).headers.is_empty(), "not yet");
        cx.executor().advance_clock(SAVE_DELAY);
        cx.run_until_parked();
        assert_eq!(stored(&storage, &collection, &id).headers, [header("X-A", "1")]);

        // Leaving the API saves what is still waiting.
        api.update(cx, |api, cx| api.set_query(vec![header("q", "x")], cx));
        drop(api);
        state.update(cx, |state, cx| state.create_api(None, "other".into(), cx));
        cx.run_until_parked();
        assert_eq!(stored(&storage, &collection, &id).query, [header("q", "x")]);
        let other = editor(&state, cx);
        assert_ne!(other.read_with(cx, |api, _| api.api().clone()), id);
    }

    #[gpui_kit::test]
    async fn sending_keeps_the_answer_and_a_second_click_cancels(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = cx.new(|cx| WorkspaceState::new(sender(&storage, true), collection.clone(), cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.create_api(None, "ping".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        api.update(cx, |api, cx| api.set_uri(" http://ping.test ".into(), cx));
        api.update(cx, |api, cx| api.send_or_cancel(None, cx));
        cx.run_until_parked();
        api.read_with(cx, |api, _| {
            assert_eq!(api.request().uri, "http://ping.test");
            assert!(api.sending_since().is_none());
            let shown = api.shown_response().expect("an answer");
            assert_eq!(shown.status, Some(200));
            assert_eq!(api.responses().len(), 1);
        });

        // A request that never answers is cancelled by sending again.
        let stuck = cx.new(|cx| WorkspaceState::new(sender(&storage, false), collection.clone(), cx));
        cx.run_until_parked();
        let api = editor(&stuck, cx);
        api.update(cx, |api, cx| api.send_or_cancel(None, cx));
        cx.run_until_parked();
        assert!(api.read_with(cx, |api, _| api.sending_since().is_some()));
        api.update(cx, |api, cx| api.send_or_cancel(None, cx));
        assert!(
            api.read_with(cx, |api, _| api.sending_since().is_some()),
            "a double click is one click"
        );
        std::thread::sleep(Duration::from_millis(210));
        api.update(cx, |api, cx| api.send_or_cancel(None, cx));
        api.read_with(cx, |api, _| {
            assert!(api.sending_since().is_none(), "cancelled");
            assert_eq!(api.responses().len(), 1, "a cancelled request leaves nothing");
        });
    }

    #[gpui_kit::test]
    async fn choosing_a_response_puts_its_request_back(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_api(None, "ping".into(), cx));
        cx.run_until_parked();
        let api = editor(&state, cx);
        api.update(cx, |api, cx| api.set_uri("http://first.test".into(), cx));
        api.update(cx, |api, cx| api.send_or_cancel(None, cx));
        cx.run_until_parked();
        let first = api.read_with(cx, |api, _| api.responses()[0].id.clone());
        api.update(cx, |api, cx| api.set_uri("http://second.test".into(), cx));
        cx.run_until_parked();

        let replaced = Rc::new(RefCell::new(0));
        let seen = replaced.clone();
        cx.update(|cx| {
            cx.subscribe(&api, move |_, event, _| {
                if matches!(event, ApiEvent::RequestReplaced) {
                    *seen.borrow_mut() += 1;
                }
            })
            .detach();
        });
        api.update(cx, |api, cx| api.choose_response(&first, cx));
        api.read_with(cx, |api, _| {
            assert_eq!(api.request().uri, "http://first.test");
            assert_eq!(api.shown_response().map(|r| r.id.clone()), Some(first.clone()));
        });
        assert_eq!(*replaced.borrow(), 1);
        let id = api.read_with(cx, |api, _| api.api().clone());
        // Not saved until it is changed (F7.10).
        cx.run_until_parked();
        assert_eq!(stored(&storage, &collection, &id).uri, "http://second.test");

        api.update(cx, |api, cx| api.clear_history(cx));
        cx.run_until_parked();
        assert!(api.read_with(cx, |api, _| api.shown_response().is_none()));
    }

    #[gpui_kit::test]
    async fn the_tab_is_kept_per_api(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_api(None, "a".into(), cx));
        cx.run_until_parked();
        assert_eq!(state.read_with(cx, |state, cx| state.tab(cx)), Some(RequestTab::Query));
        state.update(cx, |state, cx| state.set_tab(RequestTab::Auth, cx));
        let api = editor(&state, cx);
        api.update(cx, |api, cx| api.set_method(HttpMethod::Post, cx));
        cx.run_until_parked();
        assert_eq!(state.read_with(cx, |state, cx| state.tab(cx)), Some(RequestTab::Auth));

        let again = open(&storage, collection, cx);
        assert_eq!(again.read_with(cx, |state, cx| state.tab(cx)), Some(RequestTab::Auth));
    }

    #[gpui_kit::test]
    async fn deleting_clears_what_pointed_at_it(cx: &mut TestAppContext) {
        let storage = MemoryStore::new().storage();
        let collection = project(&storage).await;
        let state = open(&storage, collection.clone(), cx);
        state.update(cx, |state, cx| state.create_folder(None, "users".into(), cx));
        cx.run_until_parked();
        let users = find(&state, "users", cx);
        state.update(cx, |state, cx| {
            state.create_api(Some(users.clone()), "login".into(), cx)
        });
        cx.run_until_parked();
        let login = find(&state, "login", cx);
        state.update(cx, |state, cx| state.set_pinned(&login, true, cx));
        state.update(cx, |state, cx| state.delete(users.clone(), cx));
        cx.run_until_parked();

        state.read_with(cx, |state, _| {
            assert!(state.rows().is_empty());
            assert_eq!(state.selected(), None);
            assert!(state.pinned().is_empty());
            assert!(state.tree().is_empty());
        });
        // And from the store: a reopening finds nothing either.
        let again = open(&storage, collection, cx);
        assert!(row_names(&again, cx).is_empty());
    }
}
