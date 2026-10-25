//! Who may see and change what (`docs/architecture.md` §9), on each database.
//! The same environment as `conformance.rs`: a run without the databases
//! skips.

use cyberapi_core::access::{CollectionRole, Grantee, TeamRole};
use cyberapi_core::directory::{MCP_TOKEN_PREFIX, NewMcpToken, TeamPlace};
use cyberapi_core::model::{HttpRequest, ProjectSettings, VariableCategory};
use cyberapi_core::prefs::{AppPrefs, ThemeMode, WorkspacePrefs};
use cyberapi_core::response::HttpResponse;
use cyberapi_core::{AccountId, CollectionId};
use cyberapi_store::{AccountEdit, CollectionEdit, Entity, NewCollection, NewVariable, Refusal, Storage, StoreError};
use cyberapi_store_sql::{Acceptance, NewAccount, SqlStore};
use std::sync::Arc;

async fn account(store: &SqlStore, name: &str) -> AccountId {
    store
        .create_account(NewAccount {
            name: format!("{name}-{}", ulid::Ulid::generate()),
            display_name: name.into(),
            password_hash: "unused".into(),
            admin: false,
        })
        .await
        .expect("create an account")
}

fn not_found<T: std::fmt::Debug>(result: Result<T, StoreError>, what: Entity) {
    match result {
        Err(StoreError::NotFound { what: found, .. }) if found == what => {}
        other => panic!("expected {what:?} not found, got {other:?}"),
    }
}

fn forbidden<T: std::fmt::Debug>(result: Result<T, StoreError>, needed: CollectionRole) {
    match result {
        Err(StoreError::Forbidden { needed: found }) if found == needed => {}
        other => panic!("expected forbidden without {needed:?}, got {other:?}"),
    }
}

async fn role(storage: &Storage, id: &CollectionId) -> Option<CollectionRole> {
    let listed = storage.collections().list().await.expect("list");
    let role = listed.iter().find(|c| &c.id == id).map(|c| c.role);
    // `get` agrees with `list`.
    match storage.collections().get(id.clone()).await {
        Ok(collection) => assert_eq!(Some(collection.role), role),
        Err(_) => assert_eq!(role, None),
    }
    role
}

async fn rules(variable: &str) {
    let Ok(url) = std::env::var(variable) else {
        eprintln!("skipped: {variable} is not set");
        return;
    };
    let store = SqlStore::connect(&url).await.expect("connect");
    let (alice_id, bob_id, carol_id, dave_id) = (
        account(&store, "alice").await,
        account(&store, "bob").await,
        account(&store, "carol").await,
        account(&store, "dave").await,
    );
    let (alice, bob, carol, dave) = (
        store.scoped(alice_id.clone()),
        store.scoped(bob_id.clone()),
        store.scoped(carol_id.clone()),
        store.scoped(dave_id.clone()),
    );

    // Alice's team, with Bob in it as a member; Dave is in another team.
    let backend = store.create_team("backend", &alice_id).await.expect("team");
    store
        .set_member(&backend, &bob_id, Some(TeamRole::Member))
        .await
        .expect("member");
    let qa = store.create_team("qa", &alice_id).await.expect("team");
    store
        .set_member(&qa, &dave_id, Some(TeamRole::Member))
        .await
        .expect("member");

    let project = alice
        .collections()
        .create(NewCollection {
            name: "orders".into(),
            description: String::new(),
            owner: Some(backend.clone()),
        })
        .await
        .expect("create in the team");
    let id = project.id.clone();
    assert_eq!(project.owner, Some(backend.clone()));
    assert_eq!(
        project.role,
        CollectionRole::Manager,
        "the team's owner manages its projects"
    );

    // A member edits the team's projects (D7) but does not manage them.
    assert_eq!(role(&bob, &id).await, Some(CollectionRole::Editor));
    let login = bob
        .tree()
        .create_api(id.clone(), None, "login".into())
        .await
        .expect("an editor adds an API");
    bob.tree()
        .save_request(id.clone(), login.item.id.clone(), HttpRequest::default())
        .await
        .expect("an editor saves a request");
    // A project's API count goes to those who see the project only.
    let count = |counts: std::collections::BTreeMap<CollectionId, usize>| counts.get(&id).copied();
    assert_eq!(count(alice.collections().api_counts().await.expect("counts")), Some(1));
    assert_eq!(count(bob.collections().api_counts().await.expect("counts")), Some(1));
    assert_eq!(
        count(carol.collections().api_counts().await.expect("counts")),
        None,
        "an outsider is not told the project's size"
    );
    forbidden(
        bob.collections()
            .update(
                id.clone(),
                CollectionEdit {
                    name: "mine now".into(),
                    ..Default::default()
                },
            )
            .await,
        CollectionRole::Manager,
    );
    forbidden(bob.collections().delete(id.clone()).await, CollectionRole::Manager);

    // Someone with no way in does not learn the project exists.
    assert_eq!(role(&carol, &id).await, None);
    not_found(carol.tree().load(id.clone()).await, Entity::Collection);
    not_found(
        carol
            .tree()
            .save_request(id.clone(), login.item.id.clone(), HttpRequest::default())
            .await,
        Entity::Collection,
    );
    not_found(
        carol.variables().list(id.clone(), VariableCategory::Environment).await,
        Entity::Collection,
    );

    // Granted as a viewer: reads, keeps her own state, changes nothing.
    store
        .set_grant(&id, &Grantee::Account(carol_id.clone()), Some(CollectionRole::Viewer))
        .await
        .expect("grant");
    assert_eq!(role(&carol, &id).await, Some(CollectionRole::Viewer));
    assert_eq!(carol.tree().load(id.clone()).await.expect("load").apis.len(), 1);
    carol
        .prefs()
        .set_workspace(
            id.clone(),
            WorkspacePrefs {
                pinned: vec![login.item.id.clone()],
                ..Default::default()
            },
        )
        .await
        .expect("a viewer keeps her own workspace state");
    forbidden(
        carol.tree().create_api(id.clone(), None, "x".into()).await,
        CollectionRole::Editor,
    );
    // Nor what the project sets for its APIs (#66).
    forbidden(
        carol
            .tree()
            .set_project_settings(id.clone(), ProjectSettings::default())
            .await,
        CollectionRole::Editor,
    );
    forbidden(
        carol
            .variables()
            .create(NewVariable {
                collection: id.clone(),
                category: VariableCategory::Environment,
                name: "HOST".into(),
                value: "x".into(),
                enabled: true,
            })
            .await,
        CollectionRole::Editor,
    );
    // Her workspace state is hers alone.
    assert_eq!(
        bob.prefs().workspace(id.clone()).await.expect("prefs"),
        WorkspacePrefs::default()
    );
    // So are the responses she got: a viewer sends and keeps them, and
    // nobody else sees them.
    let answered = HttpResponse {
        api: login.item.id.clone(),
        collection: id.clone(),
        status: Some(200),
        ..Default::default()
    };
    let kept = carol
        .history()
        .add(answered.clone())
        .await
        .expect("a viewer keeps her responses");
    bob.history().add(answered).await.expect("so does an editor");
    assert_eq!(
        carol
            .history()
            .list(id.clone(), login.item.id.clone())
            .await
            .expect("list"),
        std::slice::from_ref(&kept)
    );
    assert_eq!(
        alice
            .history()
            .latest(id.clone(), vec![login.item.id.clone()])
            .await
            .expect("latest"),
        []
    );
    bob.history()
        .clear(id.clone(), login.item.id.clone())
        .await
        .expect("clear his own");
    assert_eq!(
        carol
            .history()
            .list(id.clone(), login.item.id.clone())
            .await
            .expect("list"),
        std::slice::from_ref(&kept),
        "clearing his history leaves hers"
    );
    // A whole configuration (server-sync §3): a viewer reads it but cannot
    // put one in its place; an editor can, though the name and description
    // stay the managers'.
    let pulled = carol
        .tree()
        .config(id.clone())
        .await
        .expect("a viewer reads the configuration");
    forbidden(
        carol
            .tree()
            .replace(id.clone(), pulled.config.clone(), Some(pulled.revision))
            .await,
        CollectionRole::Editor,
    );
    let mut edited = pulled.config.clone();
    edited.name = "renamed by an editor".into();
    edited.apis[0].name = "sign in".into();
    let synced = bob
        .tree()
        .replace(id.clone(), edited, Some(pulled.revision))
        .await
        .expect("an editor syncs");
    let now = alice.tree().config(id.clone()).await.expect("config");
    assert_eq!(now.revision, synced);
    assert_eq!(now.config.name, "orders", "the name stays the managers'");
    assert_eq!(now.config.apis[0].name, "sign in");
    assert_eq!(
        carol
            .history()
            .list(id.clone(), login.item.id.clone())
            .await
            .expect("list"),
        std::slice::from_ref(&kept),
        "her history of the API that stayed is still there"
    );
    // Clearing all of it, and every workspace state, is one account's too.
    alice
        .prefs()
        .set_workspace(
            id.clone(),
            WorkspacePrefs {
                selected: Some(login.item.id.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("her state");
    bob.history().clear_all().await.expect("clear all of his");
    alice.prefs().clear_workspaces().await.expect("forget hers");
    assert_eq!(
        carol
            .history()
            .list(id.clone(), login.item.id.clone())
            .await
            .expect("list"),
        [kept]
    );
    assert_eq!(
        carol.prefs().workspace(id.clone()).await.expect("prefs").pinned,
        std::slice::from_ref(&login.item.id),
        "her pins stay"
    );
    assert_eq!(
        alice.prefs().workspace(id.clone()).await.expect("prefs"),
        WorkspacePrefs::default()
    );

    // A grant to a team reaches its members.
    store
        .set_grant(&id, &Grantee::Team(qa.clone()), Some(CollectionRole::Editor))
        .await
        .expect("grant");
    assert_eq!(role(&dave, &id).await, Some(CollectionRole::Editor));
    dave.tree()
        .create_folder(id.clone(), None, "from qa".into())
        .await
        .expect("a team grant lets its members edit");

    // The strongest way in wins; a weaker grant takes nothing away.
    store
        .set_grant(&id, &Grantee::Account(bob_id.clone()), Some(CollectionRole::Viewer))
        .await
        .expect("grant");
    assert_eq!(role(&bob, &id).await, Some(CollectionRole::Editor));
    store
        .set_grant(&id, &Grantee::Account(carol_id.clone()), Some(CollectionRole::Manager))
        .await
        .expect("grant");
    carol
        .collections()
        .update(
            id.clone(),
            CollectionEdit {
                name: "orders v2".into(),
                ..Default::default()
            },
        )
        .await
        .expect("a manager grant lets her rename it");

    // Taking access back.
    store
        .set_grant(&id, &Grantee::Account(carol_id.clone()), None)
        .await
        .expect("revoke");
    assert_eq!(role(&carol, &id).await, None);
    store.set_member(&qa, &dave_id, None).await.expect("leave");
    assert_eq!(role(&dave, &id).await, None);
    not_found(
        carol.history().list(id.clone(), login.item.id.clone()).await,
        Entity::Collection,
    );

    // A personal space is one account's.
    let private = alice
        .collections()
        .create(NewCollection {
            name: "scratch".into(),
            ..Default::default()
        })
        .await
        .expect("create in her own space");
    assert_ne!(private.owner, Some(backend.clone()));
    assert_eq!(role(&bob, &private.id).await, None);

    // Creating in a team takes being in it.
    not_found(
        carol
            .collections()
            .create(NewCollection {
                name: "intruder".into(),
                description: String::new(),
                owner: Some(backend.clone()),
            })
            .await,
        Entity::Team,
    );

    // Settings are per account.
    alice
        .prefs()
        .set_app(AppPrefs {
            theme: ThemeMode::Dark,
            ..Default::default()
        })
        .await
        .expect("prefs");
    assert_eq!(bob.prefs().app().await.expect("prefs").theme, ThemeMode::System);

    // So are cookie jars.
    alice
        .cookies()
        .put(vec![cyberapi_core::cookie::Cookie {
            name: "sid".into(),
            value: "alice".into(),
            domain: "api.test".into(),
            path: "/".into(),
            ..Default::default()
        }])
        .await
        .expect("cookies");
    assert_eq!(alice.cookies().list().await.expect("cookies").len(), 1);
    assert!(bob.cookies().list().await.expect("cookies").is_empty());
    bob.cookies().clear().await.expect("clear his own");
    assert_eq!(alice.cookies().list().await.expect("cookies").len(), 1);

    // Deleting the project takes every account's state of it.
    alice.collections().delete(id.clone()).await.expect("delete");
    not_found(carol.prefs().workspace(id.clone()).await, Entity::Collection);
    assert_eq!(role(&dave, &id).await, None);
}

#[tokio::test]
async fn postgres_access_rules() {
    rules("CYBERAPI_TEST_POSTGRES").await;
}

#[tokio::test]
async fn mysql_access_rules() {
    rules("CYBERAPI_TEST_MYSQL").await;
}

/// Setup makes no administrator once there are accounts — that it makes
/// exactly one on an empty store is the bridge's test, on a database of its
/// own — and the logins that ran out are forgotten, the others kept.
async fn sign_ins(variable: &str) {
    let Ok(url) = std::env::var(variable) else {
        eprintln!("skipped: {variable} is not set");
        return;
    };
    let store = SqlStore::connect(&url).await.expect("connect");
    let erin = account(&store, "erin").await;
    let late = format!("late-{}", ulid::Ulid::generate());
    let made = store
        .create_first_admin(NewAccount {
            name: late.clone(),
            password_hash: "unused".into(),
            ..Default::default()
        })
        .await
        .expect("setup");
    assert_eq!(made, None);
    assert_eq!(store.find_login(&late).await.expect("find"), None);

    let (ran_out, kept) = (
        format!("ran-out-{}", ulid::Ulid::generate()),
        format!("kept-{}", ulid::Ulid::generate()),
    );
    store.create_session(&ran_out, &erin, 1).await.expect("a session");
    store.create_session(&kept, &erin, i64::MAX).await.expect("a session");
    assert!(store.delete_expired_sessions(2).await.expect("forget") >= 1);
    let erin_now = store.session_account(&kept, 2).await.expect("a session");
    assert_eq!(erin_now.map(|account| account.id), Some(erin));
    assert_eq!(store.session_account(&ran_out, 0).await.expect("a session"), None);
}

#[tokio::test]
async fn postgres_sign_ins() {
    sign_ins("CYBERAPI_TEST_POSTGRES").await;
}

#[tokio::test]
async fn mysql_sign_ins() {
    sign_ins("CYBERAPI_TEST_MYSQL").await;
}

/// Passwords kept as they are: what is checked is the rules, not Argon2.
struct Plain;

impl cyberapi_store_sql::Passwords for Plain {
    fn hash(&self, password: &str) -> Result<String, String> {
        Ok(format!("plain:{password}"))
    }

    fn verify(&self, hash: &str, password: &str) -> bool {
        hash == format!("plain:{password}")
    }
}

fn refused<T: std::fmt::Debug>(result: Result<T, StoreError>, reason: Refusal) {
    match result {
        Err(StoreError::Refused { reason: found }) if found == reason => {}
        other => panic!("expected {reason:?}, got {other:?}"),
    }
}

/// Teams, members, grants and accounts (§9.4): who may change what.
async fn directory(variable: &str) {
    let Ok(url) = std::env::var(variable) else {
        eprintln!("skipped: {variable} is not set");
        return;
    };
    let store = SqlStore::connect(&url)
        .await
        .expect("connect")
        .with_passwords(Arc::new(Plain));
    let root_id = store
        .create_account(NewAccount {
            name: format!("root-{}", ulid::Ulid::generate()),
            display_name: "Root".into(),
            password_hash: "plain:rootpass".into(),
            admin: true,
        })
        .await
        .expect("an administrator");
    let (alice_id, bob_id, carol_id) = (
        account(&store, "alice").await,
        account(&store, "bob").await,
        account(&store, "carol").await,
    );
    let (root, alice, bob, carol) = (
        store.directory(root_id.clone()),
        store.directory(alice_id.clone()),
        store.directory(bob_id.clone()),
        store.directory(carol_id.clone()),
    );

    // Everyone starts with their own space.
    let me = alice.me().await.expect("me");
    assert_eq!((me.account.label(), me.teams.len()), ("alice", 1));
    assert!(me.teams[0].personal && me.teams[0].role == TeamRole::Owner);
    let space = me.teams[0].id.clone();
    refused(
        alice.rename_team(space.clone(), "mine".into()).await,
        Refusal::PersonalSpace,
    );
    refused(
        alice
            .set_member(space.clone(), bob_id.clone(), Some(TeamRole::Member))
            .await,
        Refusal::PersonalSpace,
    );

    // A team: its owner adds; an admin adds members but not owners.
    let team = alice.create_team("backend".into()).await.expect("team");
    assert_eq!(alice.me().await.expect("me").teams.len(), 2);
    not_found(bob.members(team.id.clone()).await, Entity::Team);
    alice
        .set_member(team.id.clone(), bob_id.clone(), Some(TeamRole::Admin))
        .await
        .expect("bob as admin");
    bob.set_member(team.id.clone(), carol_id.clone(), Some(TeamRole::Member))
        .await
        .expect("carol as member");
    refused(
        bob.set_member(team.id.clone(), carol_id.clone(), Some(TeamRole::Owner))
            .await,
        Refusal::TeamRole {
            needed: TeamRole::Owner,
        },
    );
    refused(
        bob.set_member(team.id.clone(), alice_id.clone(), None).await,
        Refusal::TeamRole {
            needed: TeamRole::Owner,
        },
    );
    refused(
        carol.rename_team(team.id.clone(), "ours".into()).await,
        Refusal::TeamRole {
            needed: TeamRole::Admin,
        },
    );
    bob.rename_team(team.id.clone(), "platform".into())
        .await
        .expect("rename");
    let members = carol.members(team.id.clone()).await.expect("members");
    let roles: Vec<TeamRole> = members.iter().map(|member| member.role).collect();
    assert_eq!(roles, [TeamRole::Owner, TeamRole::Admin, TeamRole::Member]);
    not_found(
        alice
            .set_member(team.id.clone(), AccountId::new("nobody"), Some(TeamRole::Member))
            .await,
        Entity::Account,
    );

    // Anyone may leave, but the last owner stays.
    carol
        .set_member(team.id.clone(), carol_id.clone(), None)
        .await
        .expect("carol leaves");
    refused(
        alice.set_member(team.id.clone(), alice_id.clone(), None).await,
        Refusal::LastOwner,
    );

    // Grants are for a project's managers; a grant gives what it says.
    let project = store
        .scoped(alice_id.clone())
        .collections()
        .create(NewCollection {
            name: "shop".into(),
            owner: Some(team.id.clone()),
            ..Default::default()
        })
        .await
        .expect("a project in the team");
    not_found(carol.grants(project.id.clone()).await, Entity::Collection);
    alice
        .set_grant(
            project.id.clone(),
            Grantee::Account(carol_id.clone()),
            Some(CollectionRole::Viewer),
        )
        .await
        .expect("grant");
    let grants = bob.grants(project.id.clone()).await.expect("grants");
    assert_eq!(grants.len(), 1);
    assert_eq!(
        (grants[0].name.as_str(), grants[0].role),
        ("carol", CollectionRole::Viewer)
    );
    forbidden(carol.grants(project.id.clone()).await, CollectionRole::Manager);
    not_found(
        alice
            .set_grant(
                project.id.clone(),
                Grantee::Team(cyberapi_core::TeamId::new("nowhere")),
                Some(CollectionRole::Viewer),
            )
            .await,
        Entity::Team,
    );

    // A team with projects stays; moved away, it can go.
    refused(alice.delete_team(team.id.clone()).await, Refusal::TeamHasProjects);
    refused(
        bob.delete_team(team.id.clone()).await,
        Refusal::TeamRole {
            needed: TeamRole::Owner,
        },
    );
    not_found(bob.transfer(project.id.clone(), space.clone()).await, Entity::Team);
    alice
        .transfer(project.id.clone(), space.clone())
        .await
        .expect("transfer");
    alice.delete_team(team.id.clone()).await.expect("delete");
    assert_eq!(alice.me().await.expect("me").teams.len(), 1);
    // Bob was a member of the team that owned it: no longer.
    assert_eq!(role(&store.scoped(bob_id.clone()), &project.id).await, None);

    // Accounts come from administrators' invitations.
    let dave_name = format!("dave-{}", ulid::Ulid::generate());
    refused(alice.create_invite(None).await, Refusal::AdminOnly);
    let made = root.create_invite(None).await.expect("an invitation");
    let dave = store
        .accept_invite(
            &made.token,
            Acceptance {
                name: dave_name.clone(),
                display_name: String::new(),
                password: "long enough".into(),
            },
        )
        .await
        .expect("dave");
    let (_, hash) = store
        .find_login(&dave_name)
        .await
        .expect("find")
        .expect("dave signs in");
    assert_eq!(hash, "plain:long enough");
    assert!(
        root.accounts()
            .await
            .expect("accounts")
            .iter()
            .any(|account| account.id == dave.id)
    );

    store
        .create_session("dave-session", &dave.id, i64::MAX)
        .await
        .expect("a session");
    let edit = |disabled: bool| AccountEdit {
        display_name: "Dave".into(),
        admin: false,
        disabled,
    };
    refused(
        alice.update_account(dave.id.clone(), edit(true)).await,
        Refusal::AdminOnly,
    );
    refused(
        root.update_account(
            root_id.clone(),
            AccountEdit {
                admin: true,
                disabled: true,
                ..Default::default()
            },
        )
        .await,
        Refusal::NotYourself,
    );
    let disabled = root.update_account(dave.id.clone(), edit(true)).await.expect("disable");
    assert!(disabled.disabled && disabled.display_name == "Dave");
    assert_eq!(store.session_account("dave-session", 0).await.expect("session"), None);
    assert_eq!(
        store.find_login(&dave_name).await.expect("find"),
        None,
        "no sign-in while disabled"
    );
    root.update_account(dave.id.clone(), edit(false)).await.expect("enable");

    root.reset_password(dave.id.clone(), "brand new pass".into())
        .await
        .expect("reset");
    let dave_directory = store.directory(dave.id.clone());
    refused(
        dave_directory
            .change_password("long enough".into(), "another pass".into())
            .await,
        Refusal::WrongPassword,
    );
    dave_directory
        .change_password("brand new pass".into(), "another pass".into())
        .await
        .expect("change");
    let (_, hash) = store.find_login(&dave_name).await.expect("find").expect("dave");
    assert_eq!(hash, "plain:another pass");
}

#[tokio::test]
async fn postgres_directory() {
    directory("CYBERAPI_TEST_POSTGRES").await;
}

#[tokio::test]
async fn mysql_directory() {
    directory("CYBERAPI_TEST_MYSQL").await;
}

/// Invitations (`docs/server-sync.md` §5): who may send which, and that an
/// account comes from one only while it can be used.
async fn invitations(variable: &str) {
    let Ok(url) = std::env::var(variable) else {
        eprintln!("skipped: {variable} is not set");
        return;
    };
    let store = SqlStore::connect(&url)
        .await
        .expect("connect")
        .with_passwords(Arc::new(Plain));
    let root_id = store
        .create_account(NewAccount {
            name: format!("root-{}", ulid::Ulid::generate()),
            display_name: "Root".into(),
            password_hash: "plain:rootpass".into(),
            admin: true,
        })
        .await
        .expect("an administrator");
    let (alice_id, bob_id, carol_id) = (
        account(&store, "alice").await,
        account(&store, "bob").await,
        account(&store, "carol").await,
    );
    let (root, alice, bob, carol) = (
        store.directory(root_id.clone()),
        store.directory(alice_id.clone()),
        store.directory(bob_id.clone()),
        store.directory(carol_id.clone()),
    );
    let fresh = |prefix: &str| format!("{prefix}-{}", ulid::Ulid::generate());
    let accept = |name: &str, password: &str| Acceptance {
        name: name.into(),
        display_name: "New".into(),
        password: password.into(),
    };

    // An administrator's: an account with its own space only.
    let made = root.create_invite(None).await.expect("an invitation");
    assert_eq!(made.invite.place, None);
    assert_eq!(
        root.invites().await.expect("invites"),
        std::slice::from_ref(&made.invite)
    );
    assert!(alice.invites().await.expect("invites").is_empty(), "the sender's own");
    let info = store.invite_info(&made.token).await.expect("info");
    assert_eq!((info.inviter.as_str(), info.team), ("Root", None));
    // A refused name or password leaves the invitation as it was.
    refused(
        store.accept_invite(&made.token, accept("a b", "long enough")).await,
        Refusal::BadAccountName,
    );
    refused(
        store.accept_invite(&made.token, accept(&fresh("erin"), "short")).await,
        Refusal::PasswordTooShort,
    );
    let alice_name = alice.me().await.expect("me").account.name;
    refused(
        store
            .accept_invite(&made.token, accept(&alice_name, "long enough"))
            .await,
        Refusal::NameTaken,
    );
    let erin_name = fresh("erin");
    let erin = store
        .accept_invite(&made.token, accept(&erin_name, "long enough"))
        .await
        .expect("erin");
    let erin_me = store.directory(erin.id.clone()).me().await.expect("me");
    assert!(!erin_me.account.admin && erin_me.teams.len() == 1 && erin_me.teams[0].personal);
    // Used once.
    refused(
        store
            .accept_invite(&made.token, accept(&fresh("frank"), "long enough"))
            .await,
        Refusal::InviteGone,
    );
    refused(store.invite_info(&made.token).await, Refusal::InviteGone);
    refused(store.invite_info("no such token").await, Refusal::InviteGone);
    assert!(root.invites().await.expect("invites").is_empty());

    // Taken back by its sender only, and then it opens nothing.
    let revoked = root.create_invite(None).await.expect("an invitation");
    match alice.revoke_invite(revoked.invite.id.clone()).await {
        Err(StoreError::NotFound {
            what: Entity::Invite, ..
        }) => {}
        other => panic!("not alice's to take back: {other:?}"),
    }
    root.revoke_invite(revoked.invite.id.clone()).await.expect("revoke");
    refused(
        store
            .accept_invite(&revoked.token, accept(&fresh("gina"), "long enough"))
            .await,
        Refusal::InviteGone,
    );
    assert!(root.revoke_invite(revoked.invite.id.clone()).await.is_err());

    // Run out.
    let expired = root.create_invite(None).await.expect("an invitation");
    sqlx::any::install_default_drivers();
    let pool = sqlx::AnyPool::connect(&url).await.expect("a pool");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE invites SET expires_at = 1 WHERE id = '{}'",
        expired.invite.id
    )))
    .execute(&pool)
    .await
    .expect("age it");
    refused(
        store
            .accept_invite(&expired.token, accept(&fresh("hank"), "long enough"))
            .await,
        Refusal::InviteGone,
    );
    assert!(root.invites().await.expect("invites").is_empty());

    // A team's: its owners and admins invite, only an owner invites an owner,
    // and no one joins a personal space.
    let team = alice.create_team("backend".into()).await.expect("team");
    alice
        .set_member(team.id.clone(), bob_id.clone(), Some(TeamRole::Admin))
        .await
        .expect("bob admin");
    alice
        .set_member(team.id.clone(), carol_id.clone(), Some(TeamRole::Member))
        .await
        .expect("carol member");
    let place = |role| {
        Some(TeamPlace {
            team: team.id.clone(),
            role,
        })
    };
    refused(
        bob.create_invite(place(TeamRole::Owner)).await,
        Refusal::TeamRole {
            needed: TeamRole::Owner,
        },
    );
    refused(
        carol.create_invite(place(TeamRole::Member)).await,
        Refusal::TeamRole {
            needed: TeamRole::Admin,
        },
    );
    match root.create_invite(place(TeamRole::Member)).await {
        Err(StoreError::NotFound { what: Entity::Team, .. }) => {}
        other => panic!("not root's team: {other:?}"),
    }
    let space = alice.me().await.expect("me").teams[0].id.clone();
    refused(
        alice
            .create_invite(Some(TeamPlace {
                team: space,
                role: TeamRole::Member,
            }))
            .await,
        Refusal::PersonalSpace,
    );
    let to_team = bob.create_invite(place(TeamRole::Admin)).await.expect("bob invites");
    assert_eq!(to_team.invite.team_name.as_deref(), Some("backend"));
    let info = store.invite_info(&to_team.token).await.expect("info");
    assert_eq!(info.team.as_deref(), Some("backend"));
    let ivy = store
        .accept_invite(&to_team.token, accept(&fresh("ivy"), "long enough"))
        .await
        .expect("ivy");
    let members = alice.members(team.id.clone()).await.expect("members");
    assert!(
        members
            .iter()
            .any(|member| member.account.id == ivy.id && member.role == TeamRole::Admin)
    );

    // An invitation is only as good as its sender's place when it is used.
    let owner_invite = alice
        .create_invite(place(TeamRole::Owner))
        .await
        .expect("alice invites");
    alice
        .set_member(team.id.clone(), bob_id.clone(), Some(TeamRole::Owner))
        .await
        .expect("bob owner");
    bob.set_member(team.id.clone(), alice_id.clone(), Some(TeamRole::Admin))
        .await
        .expect("alice steps down");
    refused(
        store
            .accept_invite(&owner_invite.token, accept(&fresh("jack"), "long enough"))
            .await,
        Refusal::InviteGone,
    );
}

#[tokio::test]
async fn postgres_invitations() {
    invitations("CYBERAPI_TEST_POSTGRES").await;
}

#[tokio::test]
async fn mysql_invitations() {
    invitations("CYBERAPI_TEST_MYSQL").await;
}

/// An MCP token's hash, as the bridge looks one up.
fn token_hash(token: &str) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// MCP tokens: an account's own, shown once, signing in while they last and
/// the account is enabled.
async fn mcp_tokens(variable: &str) {
    let Ok(url) = std::env::var(variable) else {
        eprintln!("skipped: {variable} is not set");
        return;
    };
    let store = SqlStore::connect(&url).await.expect("connect");
    let (alice_id, bob_id) = (account(&store, "alice").await, account(&store, "bob").await);
    let (alice, bob) = (store.directory(alice_id.clone()), store.directory(bob_id.clone()));
    let input = |name: &str, can_send, days| NewMcpToken {
        name: name.into(),
        can_send,
        days,
    };
    let now = || {
        let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
        i64::try_from(since.map(|since| since.as_secs()).unwrap_or_default()).unwrap_or_default()
    };

    let laptop = alice
        .issue_mcp_token(input(" laptop ", false, Some(90)))
        .await
        .expect("issue");
    assert!(laptop.secret.starts_with(MCP_TOKEN_PREFIX) && laptop.secret.len() == MCP_TOKEN_PREFIX.len() + 64);
    assert_eq!(laptop.token.name, "laptop", "trimmed");
    let lasts = laptop.token.expires_at.expect("an end").get().timestamp() - now();
    assert!((89 * 24 * 3600..=90 * 24 * 3600).contains(&lasts));
    let ci = alice.issue_mcp_token(input("ci", true, None)).await.expect("issue");
    assert_eq!(ci.token.expires_at, None);
    assert_eq!(
        alice.mcp_tokens().await.expect("list"),
        [laptop.token.clone(), ci.token.clone()]
    );
    assert!(bob.mcp_tokens().await.expect("list").is_empty(), "each account its own");

    refused(
        alice.issue_mcp_token(input("laptop", true, None)).await,
        Refusal::TokenNameTaken,
    );
    refused(
        alice.issue_mcp_token(input("  ", false, None)).await,
        Refusal::BadTokenName,
    );
    refused(
        alice.issue_mcp_token(input("x", false, Some(7))).await,
        Refusal::BadTokenDays,
    );
    bob.issue_mcp_token(input("laptop", false, None))
        .await
        .expect("another account's name");

    // It signs in as its account, and only with the token itself.
    let (who, token) = store
        .mcp_token_account(&token_hash(&ci.secret), now())
        .await
        .expect("look up")
        .expect("signs in");
    assert_eq!((who.id, token), (alice_id.clone(), ci.token.clone()));
    assert!(
        store
            .mcp_token_account(&token_hash("cat_0"), now())
            .await
            .expect("look up")
            .is_none()
    );
    assert!(
        store
            .mcp_token_account(&ci.secret, now())
            .await
            .expect("look up")
            .is_none(),
        "not the token"
    );

    // Past its end it is gone; a disabled account's sign nobody in.
    let pool = sqlx::AnyPool::connect(&url).await.expect("a pool");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE mcp_tokens SET expires_at = 1 WHERE id = '{}'",
        laptop.token.id
    )))
    .execute(&pool)
    .await
    .expect("age it");
    assert!(
        store
            .mcp_token_account(&token_hash(&laptop.secret), now())
            .await
            .expect("look up")
            .is_none()
    );
    assert_eq!(alice.mcp_tokens().await.expect("list"), std::slice::from_ref(&ci.token));
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE accounts SET disabled = 1 WHERE id = '{alice_id}'"
    )))
    .execute(&pool)
    .await
    .expect("disable");
    assert!(
        store
            .mcp_token_account(&token_hash(&ci.secret), now())
            .await
            .expect("look up")
            .is_none()
    );
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE accounts SET disabled = 0 WHERE id = '{alice_id}'"
    )))
    .execute(&pool)
    .await
    .expect("enable");

    // Revoked by its owner only.
    not_found(bob.revoke_mcp_token(ci.token.id.clone()).await, Entity::McpToken);
    alice.revoke_mcp_token(ci.token.id.clone()).await.expect("revoke");
    assert!(
        store
            .mcp_token_account(&token_hash(&ci.secret), now())
            .await
            .expect("look up")
            .is_none()
    );
    not_found(alice.revoke_mcp_token(ci.token.id.clone()).await, Entity::McpToken);
}

#[tokio::test]
async fn postgres_mcp_tokens() {
    mcp_tokens("CYBERAPI_TEST_POSTGRES").await;
}

#[tokio::test]
async fn mysql_mcp_tokens() {
    mcp_tokens("CYBERAPI_TEST_MYSQL").await;
}
