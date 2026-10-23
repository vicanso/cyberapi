//! The desktop's copies of server projects end to end (`docs/server-sync.md`
//! §6): a bridge in memory — the wire protocol over a `MemoryStore`, signed
//! in as one account — the desktop's own store beside it, and the server,
//! dashboard and workspace states between them.

use super::copy::ServerCopy;
use super::{DashboardState, ServerState, WorkspaceEvent, WorkspaceState};
use crate::server_local::{self, ServerFile};
use cyberapi_core::access::{CollectionRole, Grantee, TeamRole};
use cyberapi_core::directory::{AccountInfo, Grant, Invite, Me, Member, NewInvite, Team, TeamPlace};
use cyberapi_core::model::Collection;
use cyberapi_core::prefs::{SendPrefs, Timeouts};
use cyberapi_core::response::Exchange;
use cyberapi_core::template::NoFiles;
use cyberapi_core::{AccountId, CollectionId, InviteId, TeamId};
use cyberapi_http::{HttpEngine, PreparedRequest, SendError, SendFuture};
use cyberapi_service::Sender;
use cyberapi_store::{
    AccountEdit, DirectoryStore, MemoryStore, NewCollection, ServerCopyStore, Storage, StoreError, StoreFuture,
};
use cyberapi_store_bridge::{Call, Reply, dispatch};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const ADDRESS: &str = "http://bridge.test";
const ACCOUNT: &str = "alice-id";

/// The server, behind the protocol: every call is the signed-in account's.
struct Bridge {
    server: MemoryStore,
    online: AtomicBool,
    calls: AtomicUsize,
}

impl Bridge {
    fn me() -> Me {
        Me {
            account: AccountInfo {
                id: AccountId::new(ACCOUNT),
                name: "alice".into(),
                display_name: "Alice".into(),
                admin: false,
                disabled: false,
            },
            teams: vec![Team {
                id: TeamId::new("team"),
                name: "team".into(),
                personal: false,
                role: TeamRole::Owner,
            }],
        }
    }
}

impl HttpEngine for Bridge {
    fn send(&self, request: PreparedRequest) -> SendFuture {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if !self.online.load(Ordering::SeqCst) {
            return Box::pin(async {
                Err(SendError::Connect {
                    message: "offline".into(),
                })
            });
        }
        let storage = self.server.storage();
        Box::pin(async move {
            let reply = if request.url.ends_with("/v1/store") {
                match serde_json::from_slice::<Call>(&request.body) {
                    Ok(Call::DirectoryMe) => Reply::Ok(serde_json::to_value(Bridge::me()).unwrap_or_default()),
                    Ok(call) => dispatch(&storage, &NoDirectory, call).await,
                    Err(error) => Reply::Err(StoreError::backend(error)),
                }
            } else {
                Reply::Err(StoreError::backend("not this bridge's"))
            };
            Ok(Exchange {
                status: 200,
                body: serde_json::to_vec(&reply).unwrap_or_default(),
                ..Default::default()
            })
        })
    }
}

/// Nothing of the directory but who is signed in, which [`Bridge`] answers.
struct NoDirectory;

fn none<T: Send + 'static>() -> StoreFuture<T> {
    Box::pin(async { Err(StoreError::backend("no directory here")) })
}

impl DirectoryStore for NoDirectory {
    fn me(&self) -> StoreFuture<Me> {
        none()
    }
    fn accounts(&self) -> StoreFuture<Vec<AccountInfo>> {
        none()
    }
    fn create_team(&self, _: String) -> StoreFuture<Team> {
        none()
    }
    fn rename_team(&self, _: TeamId, _: String) -> StoreFuture<()> {
        none()
    }
    fn delete_team(&self, _: TeamId) -> StoreFuture<()> {
        none()
    }
    fn members(&self, _: TeamId) -> StoreFuture<Vec<Member>> {
        none()
    }
    fn set_member(&self, _: TeamId, _: AccountId, _: Option<TeamRole>) -> StoreFuture<()> {
        none()
    }
    fn grants(&self, _: CollectionId) -> StoreFuture<Vec<Grant>> {
        none()
    }
    fn set_grant(&self, _: CollectionId, _: Grantee, _: Option<CollectionRole>) -> StoreFuture<()> {
        none()
    }
    fn transfer(&self, _: CollectionId, _: TeamId) -> StoreFuture<()> {
        none()
    }
    fn invites(&self) -> StoreFuture<Vec<Invite>> {
        none()
    }
    fn create_invite(&self, _: Option<TeamPlace>) -> StoreFuture<NewInvite> {
        none()
    }
    fn revoke_invite(&self, _: InviteId) -> StoreFuture<()> {
        none()
    }
    fn update_account(&self, _: AccountId, _: AccountEdit) -> StoreFuture<AccountInfo> {
        none()
    }
    fn reset_password(&self, _: AccountId, _: String) -> StoreFuture<()> {
        none()
    }
    fn change_password(&self, _: String, _: String) -> StoreFuture<()> {
        none()
    }
}

struct Machine {
    bridge: Arc<Bridge>,
    local: MemoryStore,
    server: Entity<ServerState>,
}

impl Machine {
    /// Signed in to the bridge as alice, its session kept from before.
    fn signed_in(cx: &mut TestAppContext) -> Self {
        server_local::write(&ServerFile {
            address: ADDRESS.into(),
            token: Some("tok".into()),
            account: Some(ACCOUNT.into()),
        });
        let bridge = Arc::new(Bridge {
            server: MemoryStore::new(),
            online: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
        });
        let local = MemoryStore::new();
        let engine: Arc<dyn HttpEngine> = bridge.clone();
        let copies: Arc<dyn ServerCopyStore> = Arc::new(local.clone());
        let server = cx.new(|cx| ServerState::new(engine, Some(copies), cx));
        cx.run_until_parked();
        Self { bridge, local, server }
    }

    fn dashboard(&self, cx: &mut TestAppContext) -> Entity<DashboardState> {
        let (local, server) = (self.local.storage(), self.server.clone());
        let dashboard = cx.new(|cx| DashboardState::new(local, None, Some(server), cx));
        cx.run_until_parked();
        dashboard
    }

    fn sender(&self) -> Sender {
        Sender {
            storage: self.local.storage(),
            engine: Arc::new(Answers),
            files: Arc::new(NoFiles("no files in tests")),
            user_agent: "CyberAPI/test".into(),
            timeouts: Timeouts::default(),
            sending: SendPrefs::default(),
        }
    }

    fn open(
        &self,
        collection: Collection,
        cx: &mut TestAppContext,
    ) -> (Entity<WorkspaceState>, Rc<RefCell<Vec<String>>>) {
        let (sender, local) = (self.sender(), self.local.storage());
        let server = self
            .server
            .read_with(cx, |server, _| server.storage().expect("signed in"));
        let copies: Arc<dyn ServerCopyStore> = Arc::new(self.local.clone());
        let copy = ServerCopy::desktop(&collection, local, copies, AccountId::new(ACCOUNT));
        let state = cx.new(|cx| WorkspaceState::server_copy(sender, collection, server, copy, cx));
        let told: Rc<RefCell<Vec<String>>> = Rc::default();
        let heard = told.clone();
        cx.update(|cx| {
            cx.subscribe(&state, move |_, event: &WorkspaceEvent, _| {
                heard.borrow_mut().push(format!("{event:?}"));
            })
            .detach();
        });
        cx.run_until_parked();
        (state, told)
    }

    async fn on_server(&self, name: &str) -> Collection {
        self.bridge
            .server
            .storage()
            .collections()
            .create(NewCollection {
                name: name.into(),
                ..Default::default()
            })
            .await
            .expect("a server project")
    }

    fn binding(&self, id: &CollectionId) -> Option<cyberapi_core::sync::CopyBinding> {
        let bound = pollster::block_on(self.local.copies()).expect("bindings");
        bound.into_iter().find(|binding| &binding.collection == id)
    }
}

/// Every request answered `200 ok`.
struct Answers;

impl HttpEngine for Answers {
    fn send(&self, _: PreparedRequest) -> SendFuture {
        Box::pin(async {
            Ok(Exchange {
                status: 200,
                body: b"ok".to_vec(),
                ..Default::default()
            })
        })
    }
}

fn api_names(storage: &Storage, id: &CollectionId) -> Vec<String> {
    let data = pollster::block_on(storage.tree().load(id.clone())).expect("load");
    let mut names: Vec<String> = data.apis.into_iter().map(|api| api.name).collect();
    names.sort();
    names
}

fn shown(dashboard: &Entity<DashboardState>, cx: &mut TestAppContext) -> Vec<String> {
    dashboard.read_with(cx, |state, _| {
        state
            .collections()
            .iter()
            .map(|collection| collection.name.clone())
            .collect()
    })
}

#[gpui_kit::test]
async fn the_accounts_server_projects_are_copied_and_synced(cx: &mut TestAppContext) {
    let machine = Machine::signed_in(cx);
    let theirs = machine.on_server("orders").await;
    let mine = machine
        .local
        .storage()
        .collections()
        .create(NewCollection {
            name: "scratch".into(),
            ..Default::default()
        })
        .await
        .expect("a project of this machine's");

    // Listed beside this machine's own, and copied here level with it.
    let dashboard = machine.dashboard(cx);
    assert_eq!(shown(&dashboard, cx), ["scratch", "orders"]);
    let binding = machine.binding(&theirs.id).expect("a copy");
    assert_eq!((binding.base, binding.unsynced), (Some(theirs.revision), false));
    assert_eq!(machine.binding(&mine.id), None, "this machine's own is no copy");

    // Edits go to the copy here, not to the server.
    let (workspace, told) = machine.open(theirs.clone(), cx);
    workspace.update(cx, |state, cx| state.create_api(None, "login".into(), cx));
    cx.run_until_parked();
    assert_eq!(api_names(&machine.local.storage(), &theirs.id), ["login"]);
    assert!(api_names(&machine.bridge.server.storage(), &theirs.id).is_empty());
    assert!(machine.binding(&theirs.id).expect("a copy").unsynced);
    assert!(told.borrow().iter().any(|event| event == "FirstChange"));

    // Sent from here, its history is this machine's.
    let api = workspace.read_with(cx, |state, _| state.api().cloned().expect("an editor"));
    api.update(cx, |api, cx| {
        api.set_uri("http://api.test/login".into(), cx);
        api.send_or_cancel(None, cx);
    });
    cx.run_until_parked();
    let login = api.read_with(cx, |api, _| api.api().clone());
    let history = pollster::block_on(machine.local.storage().history().list(theirs.id.clone(), login));
    assert_eq!(history.expect("kept here").len(), 1);

    // Synced: the server has it, and the copy is level again.
    workspace.update(cx, |state, cx| state.sync(false, cx));
    cx.run_until_parked();
    assert_eq!(api_names(&machine.bridge.server.storage(), &theirs.id), ["login"]);
    let binding = machine.binding(&theirs.id).expect("a copy");
    assert!(!binding.unsynced);
    let revision = pollster::block_on(machine.bridge.server.storage().collections().get(theirs.id.clone()));
    assert_eq!(binding.base, Some(revision.expect("on the server").revision));
}

#[gpui_kit::test]
async fn offline_the_copy_is_still_worked_on(cx: &mut TestAppContext) {
    let machine = Machine::signed_in(cx);
    let theirs = machine.on_server("orders").await;
    machine.dashboard(cx);

    machine.bridge.online.store(false, Ordering::SeqCst);
    let (workspace, told) = machine.open(theirs.clone(), cx);
    assert!(told.borrow().iter().any(|event| event.starts_with("NotPulled")));
    workspace.update(cx, |state, cx| state.create_api(None, "offline".into(), cx));
    cx.run_until_parked();
    assert_eq!(api_names(&machine.local.storage(), &theirs.id), ["offline"]);
    assert!(workspace.read_with(cx, |state, _| state.unsynced()));

    // Listed as kept while the server cannot be asked.
    let dashboard = machine.dashboard(cx);
    assert_eq!(shown(&dashboard, cx), ["orders"]);
    assert_eq!(
        dashboard.read_with(cx, |state, _| state.unsynced(&theirs.id)),
        Some(false)
    );
}

#[gpui_kit::test]
async fn this_machines_projects_ask_the_server_nothing(cx: &mut TestAppContext) {
    let machine = Machine::signed_in(cx);
    let mine = machine
        .local
        .storage()
        .collections()
        .create(NewCollection {
            name: "scratch".into(),
            ..Default::default()
        })
        .await
        .expect("a project of this machine's");
    machine.dashboard(cx);
    let before = machine.bridge.calls.load(Ordering::SeqCst);
    let workspace = cx.new(|cx| WorkspaceState::new(machine.sender(), mine.clone(), cx));
    cx.run_until_parked();
    workspace.update(cx, |state, cx| state.create_api(None, "local".into(), cx));
    cx.executor().advance_clock(std::time::Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(machine.bridge.calls.load(Ordering::SeqCst), before, "not one request");
    assert_eq!(api_names(&machine.local.storage(), &mine.id), ["local"]);
}

#[gpui_kit::test]
async fn signed_out_the_copies_are_not_listed(cx: &mut TestAppContext) {
    let machine = Machine::signed_in(cx);
    let theirs = machine.on_server("orders").await;
    let dashboard = machine.dashboard(cx);
    assert_eq!(shown(&dashboard, cx), ["orders"]);

    machine.server.update(cx, |server, cx| server.sign_out(cx));
    cx.run_until_parked();
    dashboard.update(cx, |state, cx| state.server_changed(cx));
    cx.run_until_parked();
    assert!(shown(&dashboard, cx).is_empty());
    assert!(machine.binding(&theirs.id).is_some(), "the copy stays here");
}

#[gpui_kit::test]
async fn a_level_copy_follows_the_server_and_an_unsynced_one_stays(cx: &mut TestAppContext) {
    let machine = Machine::signed_in(cx);
    let (level, edited) = (machine.on_server("level").await, machine.on_server("edited").await);
    let dashboard = machine.dashboard(cx);

    // Changes of this reader's to one of them, here.
    let (workspace, _) = machine.open(edited.clone(), cx);
    workspace.update(cx, |state, cx| state.create_api(None, "mine".into(), cx));
    cx.run_until_parked();
    drop(workspace);

    // Someone else changes both on the server.
    for project in [&level, &edited] {
        machine
            .bridge
            .server
            .storage()
            .tree()
            .create_api(project.id.clone(), None, "theirs".into())
            .await
            .expect("their change");
    }
    dashboard.update(cx, |state, cx| state.reload(cx));
    cx.run_until_parked();
    assert_eq!(api_names(&machine.local.storage(), &level.id), ["theirs"], "pulled");
    assert_eq!(api_names(&machine.local.storage(), &edited.id), ["mine"], "kept");
    assert_eq!(
        dashboard.read_with(cx, |state, _| state.unsynced(&edited.id)),
        Some(true)
    );
    assert_eq!(dashboard.read_with(cx, |state, _| state.unsynced(&level.id)), None);
}

/// The project open when the app last left reopens once it is known who is
/// signed in and which projects are copies — a copy as one, and another
/// account's copy not at all.
#[gpui_kit::test]
async fn the_last_copy_reopens_once_the_server_is_known(cx: &mut TestAppContext) {
    use crate::Extras;
    use crate::states::{AppStore, Route};
    use cyberapi_core::prefs::AppPrefs;

    let machine = Machine::signed_in(cx);
    let theirs = machine.on_server("orders").await;
    machine.dashboard(cx);
    let (local, bridge) = (machine.local.clone(), machine.bridge.clone());
    let state = cx.update(|cx| {
        gpui_kit::init(cx);
        let extras = Extras {
            copies: Some(Arc::new(local.clone())),
            ..Default::default()
        };
        let prefs = AppPrefs {
            last_collection: Some(theirs.id.clone()),
            ..Default::default()
        };
        AppStore::init(local.storage(), bridge, prefs, extras, cx);
        AppStore::global(cx).state().clone()
    });
    cx.run_until_parked();
    match state.read_with(cx, |state, _| state.route().clone()) {
        Route::Workspace(collection) => assert_eq!(collection.id, theirs.id),
        other => panic!("expected the copy open, got {other:?}"),
    }
    assert_eq!(
        state.read_with(cx, |state, _| state.me().map(|me| me.account.name.clone())),
        Some("alice".to_string()),
        "who is signed in, as the bridge says"
    );

    // Signed out, the copy is no longer the reader's to have open.
    let server = cx.update(|cx| AppStore::global(cx).server().cloned().expect("the desktop's server"));
    server.update(cx, |server, cx| server.sign_out(cx));
    cx.run_until_parked();
    assert_eq!(state.read_with(cx, |state, _| state.route().clone()), Route::Dashboard);
    assert_eq!(state.read_with(cx, |state, _| state.me().cloned()), None);
}

/// Server-sync §3.4 on the desktop: the reader's own items stay in the copy
/// when the project list syncs it, and when it pulls it again.
#[gpui_kit::test]
async fn the_readers_own_items_stay_in_the_copy(cx: &mut TestAppContext) {
    let machine = Machine::signed_in(cx);
    let theirs = machine.on_server("orders").await;
    let dashboard = machine.dashboard(cx);
    let (workspace, _) = machine.open(theirs.clone(), cx);
    workspace.update(cx, |state, cx| state.create_api(None, "scratch".into(), cx));
    cx.run_until_parked();
    let scratch = workspace.read_with(cx, |state, _| state.selected().cloned().expect("made and opened"));
    workspace.update(cx, |state, cx| state.set_local(&scratch, true, cx));
    cx.run_until_parked();
    assert!(!workspace.read_with(cx, |state, _| state.unsynced()));
    workspace.update(cx, |state, cx| state.create_api(None, "shared".into(), cx));
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |state, _| state.unsynced()));
    drop(workspace);

    // Synced from the project list: the server has the shared one alone.
    dashboard.update(cx, |state, cx| state.sync(&theirs.id, false, cx));
    cx.run_until_parked();
    assert_eq!(api_names(&machine.bridge.server.storage(), &theirs.id), ["shared"]);
    assert!(!machine.binding(&theirs.id).expect("a copy").unsynced);

    // Someone else changes it; the list pulls it again, the reader's kept.
    machine
        .bridge
        .server
        .storage()
        .tree()
        .create_api(theirs.id.clone(), None, "theirs".into())
        .await
        .expect("their change");
    dashboard.update(cx, |state, cx| state.reload(cx));
    cx.run_until_parked();
    assert_eq!(
        api_names(&machine.local.storage(), &theirs.id),
        ["scratch", "shared", "theirs"]
    );

    // Opened again, it is still theirs alone, and nothing is unsynced.
    let (again, _) = machine.open(theirs.clone(), cx);
    again.read_with(cx, |state, _| {
        assert!(state.local_members().contains(&scratch));
        assert!(!state.unsynced());
    });
}
