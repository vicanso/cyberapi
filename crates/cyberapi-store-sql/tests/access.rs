//! Who may see and change what (`docs/architecture.md` §9), on each database.
//! The same environment as `conformance.rs`: a run without the databases
//! skips.

use cyberapi_core::access::{CollectionRole, Grantee, TeamRole};
use cyberapi_core::model::{HttpRequest, VariableCategory};
use cyberapi_core::prefs::{AppPrefs, ThemeMode, WorkspacePrefs};
use cyberapi_core::response::HttpResponse;
use cyberapi_core::{AccountId, CollectionId};
use cyberapi_store::{
    AccountEdit, AccountInvite, CollectionEdit, Entity, NewCollection, NewVariable, Refusal, Storage, StoreError,
};
use cyberapi_store_sql::{NewAccount, SqlStore};
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

    // Accounts are the administrators'.
    let invite = |name: &str, password: &str| AccountInvite {
        name: name.into(),
        display_name: String::new(),
        password: password.into(),
        admin: false,
    };
    let dave_name = format!("dave-{}", ulid::Ulid::generate());
    refused(
        alice.create_account(invite(&dave_name, "long enough")).await,
        Refusal::AdminOnly,
    );
    refused(
        root.create_account(invite("a b", "long enough")).await,
        Refusal::BadAccountName,
    );
    refused(
        root.create_account(invite(&dave_name, "short")).await,
        Refusal::PasswordTooShort,
    );
    let dave = root
        .create_account(invite(&dave_name, "long enough"))
        .await
        .expect("dave");
    refused(
        root.create_account(invite(&dave_name, "long enough")).await,
        Refusal::NameTaken,
    );
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
