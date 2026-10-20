//! Who may see and change what (`docs/architecture.md` §9), on each database.
//! The same environment as `conformance.rs`: a run without the databases
//! skips.

use cyberapi_core::access::{CollectionRole, Grantee, TeamRole};
use cyberapi_core::model::{HttpRequest, VariableCategory};
use cyberapi_core::prefs::{AppPrefs, ThemeMode, WorkspacePrefs};
use cyberapi_core::{AccountId, CollectionId};
use cyberapi_store::{CollectionEdit, Entity, NewCollection, NewVariable, Storage, StoreError};
use cyberapi_store_sql::{NewAccount, SqlStore};

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
