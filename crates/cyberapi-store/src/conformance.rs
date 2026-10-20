//! What every backend must do the same way (`docs/architecture.md` §4.2).
//!
//! A backend's tests hand [`run`] a [`Storage`] and a [`Probe`], and drive it
//! with their own executor. The checks share that storage: each makes its own
//! projects and looks only at those, so they can run against a database that
//! already holds data. A failure names the check through its file and line.

use crate::{
    CollectionEdit, Entity, Inserted, NewCollection, NewVariable, Storage, StoreError, StoreFuture, VariableEdit,
};
use cyberapi_core::access::CollectionRole;
use cyberapi_core::collection::SortOrder;
use cyberapi_core::cookie::Cookie;
use cyberapi_core::model::{
    ApiCategory, ApiSetting, Collection, ContentType, Folder, HttpMethod, HttpRequest, KvParam, VariableCategory,
};
use cyberapi_core::prefs::{AppPrefs, ThemeMode, WorkspacePrefs};
use cyberapi_core::response::{HISTORY_LIMIT, HttpResponse, HttpStats};
use cyberapi_core::tree::{Placement, Tree, TreeData, TreeError};
use cyberapi_core::{CollectionId, ItemId, VariableId};

/// What a backend shows the suite beyond the traits. Some rules cannot be
/// seen through them: once a project is deleted every method answers "not
/// found", whether its rows are gone or left behind — and the old app left its
/// variables behind (#5).
pub trait Probe: Send + Sync {
    /// How many stored rows still belong to the project, in any table.
    fn rows_of(&self, collection: CollectionId) -> StoreFuture<usize>;
}

/// Every check, in order.
pub async fn run(storage: &Storage, probe: &dyn Probe) {
    collections_round_trip(storage).await;
    collection_names_are_required(storage).await;
    unknown_collections_are_not_found(storage).await;
    deleting_a_collection_takes_its_content(storage, probe).await;
    new_items_are_placed_as_planned(storage).await;
    bad_parents_are_refused(storage).await;
    items_are_renamed(storage).await;
    requests_are_saved(storage).await;
    moves_follow_the_plan(storage).await;
    deleting_a_folder_takes_its_content(storage).await;
    projects_do_not_see_each_other(storage).await;
    content_changes_touch_the_collection(storage).await;
    shared_changes_move_the_revision(storage).await;
    variables_round_trip(storage).await;
    app_prefs_round_trip(storage).await;
    workspace_prefs_are_per_collection(storage).await;
    history_keeps_the_newest(storage, probe).await;
    history_is_only_of_the_projects_apis(storage).await;
    cookies_are_kept_by_key(storage).await;
}

const MISSING: &str = "conformance-missing";

async fn new_collection(storage: &Storage, name: &str) -> Collection {
    storage
        .collections()
        .create(NewCollection {
            name: name.into(),
            description: format!("{name}, made by the conformance suite"),
            owner: None,
        })
        .await
        .expect("create a collection")
}

async fn folder(storage: &Storage, collection: &Collection, parent: Option<&ItemId>, name: &str) -> Inserted<Folder> {
    storage
        .tree()
        .create_folder(collection.id.clone(), parent.cloned(), name.into())
        .await
        .expect("create a folder")
}

async fn api(storage: &Storage, collection: &Collection, parent: Option<&ItemId>, name: &str) -> Inserted<ApiSetting> {
    storage
        .tree()
        .create_api(collection.id.clone(), parent.cloned(), name.into())
        .await
        .expect("create an API")
}

async fn load(storage: &Storage, collection: &Collection) -> TreeData {
    storage.tree().load(collection.id.clone()).await.expect("load the tree")
}

fn not_found<T: std::fmt::Debug>(result: Result<T, StoreError>, what: Entity) {
    match result {
        Err(StoreError::NotFound { what: found, .. }) if found == what => {}
        other => panic!("expected {what:?} not found, got {other:?}"),
    }
}

async fn collections_round_trip(storage: &Storage) {
    let created = new_collection(storage, "  round trip  ").await;
    assert_eq!(created.name, "round trip", "names are trimmed");
    assert_eq!(created.role, CollectionRole::Manager, "the creator manages it");
    assert!(created.updated_at >= created.created_at);

    let listed = storage.collections().list().await.expect("list");
    assert!(listed.contains(&created), "a new collection is listed as created");
    assert_eq!(
        storage.collections().get(created.id.clone()).await.expect("get"),
        created
    );

    let updated = storage
        .collections()
        .update(
            created.id.clone(),
            CollectionEdit {
                name: "renamed".into(),
                description: "changed".into(),
            },
        )
        .await
        .expect("update");
    assert_eq!(
        (updated.name.as_str(), updated.description.as_str()),
        ("renamed", "changed")
    );
    assert_eq!(updated.created_at, created.created_at);
    assert!(updated.updated_at >= created.updated_at);
    assert_eq!(storage.collections().get(created.id).await.expect("get"), updated);
}

async fn collection_names_are_required(storage: &Storage) {
    let blank = storage
        .collections()
        .create(NewCollection {
            name: "  ".into(),
            ..Default::default()
        })
        .await;
    assert_eq!(blank, Err(StoreError::NameRequired));

    let collection = new_collection(storage, "names").await;
    let blank = storage
        .collections()
        .update(collection.id.clone(), CollectionEdit::default())
        .await;
    assert_eq!(blank, Err(StoreError::NameRequired));
    assert_eq!(
        storage.collections().get(collection.id).await.expect("get").name,
        "names"
    );
}

async fn unknown_collections_are_not_found(storage: &Storage) {
    let missing = || CollectionId::from(MISSING);
    not_found(storage.collections().get(missing()).await, Entity::Collection);
    not_found(
        storage
            .collections()
            .update(
                missing(),
                CollectionEdit {
                    name: "x".into(),
                    ..Default::default()
                },
            )
            .await,
        Entity::Collection,
    );
    not_found(storage.collections().delete(missing()).await, Entity::Collection);
    not_found(storage.tree().load(missing()).await, Entity::Collection);
    not_found(
        storage.tree().create_api(missing(), None, "x".into()).await,
        Entity::Collection,
    );
    not_found(
        storage.variables().list(missing(), VariableCategory::Environment).await,
        Entity::Collection,
    );
    not_found(storage.prefs().workspace(missing()).await, Entity::Collection);
}

async fn deleting_a_collection_takes_its_content(storage: &Storage, probe: &dyn Probe) {
    let doomed = new_collection(storage, "doomed").await;
    let kept = new_collection(storage, "kept").await;
    let users = folder(storage, &doomed, None, "users").await;
    let login = api(storage, &doomed, Some(&users.item.id), "login").await;
    let kept_api = api(storage, &kept, None, "health").await;
    storage
        .variables()
        .create(NewVariable {
            collection: doomed.id.clone(),
            category: VariableCategory::Environment,
            name: "HOST".into(),
            value: "http://localhost".into(),
            enabled: true,
        })
        .await
        .expect("create a variable");
    storage
        .prefs()
        .set_workspace(
            doomed.id.clone(),
            WorkspacePrefs {
                expanded: vec![users.item.id.clone()],
                ..Default::default()
            },
        )
        .await
        .expect("set workspace prefs");
    storage
        .history()
        .add(response(&doomed, &login.item.id, 0))
        .await
        .expect("add a response");
    let kept_rows = probe.rows_of(kept.id.clone()).await.expect("probe");
    // The project row, the folder, the API, the variable, the workspace
    // state, the response — and the top-level order wherever the backend
    // keeps one.
    assert!(
        probe.rows_of(doomed.id.clone()).await.expect("probe") >= 6,
        "the probe sees the project's rows"
    );

    storage.collections().delete(doomed.id.clone()).await.expect("delete");
    assert_eq!(
        probe.rows_of(doomed.id.clone()).await.expect("probe"),
        0,
        "nothing of a deleted project is left behind"
    );
    assert_eq!(probe.rows_of(kept.id.clone()).await.expect("probe"), kept_rows);

    not_found(storage.collections().get(doomed.id.clone()).await, Entity::Collection);
    not_found(storage.tree().load(doomed.id.clone()).await, Entity::Collection);
    not_found(
        storage
            .variables()
            .list(doomed.id.clone(), VariableCategory::Environment)
            .await,
        Entity::Collection,
    );
    not_found(storage.prefs().workspace(doomed.id.clone()).await, Entity::Collection);
    not_found(
        storage.history().list(doomed.id.clone(), login.item.id.clone()).await,
        Entity::Collection,
    );
    let listed = storage.collections().list().await.expect("list");
    assert!(!listed.iter().any(|collection| collection.id == doomed.id));
    assert_eq!(
        load(storage, &kept).await.apis,
        vec![kept_api.item],
        "other projects are untouched"
    );
}

async fn new_items_are_placed_as_planned(storage: &Storage) {
    let collection = new_collection(storage, "placing").await;
    let mut expected = load(storage, &collection).await;
    assert_eq!(expected, TreeData::default(), "a new project's tree is empty");

    let users = folder(storage, &collection, None, " users ").await;
    assert_eq!(users.item.name, "users");
    assert_eq!(users.item.collection, collection.id);
    assert!(users.item.children.is_empty());
    expected.folders.push(users.item.clone());
    expected.apply(&users.change);

    let login = api(storage, &collection, Some(&users.item.id), "login").await;
    assert_eq!(login.item.category, ApiCategory::Http);
    assert_eq!(login.item.request, HttpRequest::default());
    expected.apis.push(login.item.clone());
    expected.apply(&login.change);

    let health = api(storage, &collection, None, "health").await;
    expected.apis.push(health.item.clone());
    expected.apply(&health.change);

    let loaded = load(storage, &collection).await;
    assert_eq!(loaded, expected, "the store wrote what it answered");
    let tree = Tree::build(&loaded);
    assert_eq!(
        tree.top_level_ids(),
        vec![health.item.id.clone(), users.item.id.clone()],
        "a new top-level item comes first"
    );
    assert_eq!(
        tree.get(&users.item.id).map(|node| node.children().to_vec()),
        Some(vec![login.item.id])
    );
}

async fn bad_parents_are_refused(storage: &Storage) {
    let collection = new_collection(storage, "parents").await;
    let health = api(storage, &collection, None, "health").await;
    not_found(
        storage
            .tree()
            .create_folder(collection.id.clone(), Some(ItemId::from(MISSING)), "x".into())
            .await,
        Entity::Folder,
    );
    let into_api = storage
        .tree()
        .create_api(collection.id.clone(), Some(health.item.id.clone()), "x".into())
        .await;
    assert_eq!(
        into_api,
        Err(StoreError::Tree {
            source: TreeError::NotAFolder { id: health.item.id }
        })
    );
    let blank = storage
        .tree()
        .create_folder(collection.id.clone(), None, " ".into())
        .await;
    assert_eq!(blank, Err(StoreError::NameRequired));
    assert_eq!(load(storage, &collection).await.apis.len(), 1, "nothing was half made");
}

async fn items_are_renamed(storage: &Storage) {
    let collection = new_collection(storage, "renames").await;
    let users = folder(storage, &collection, None, "users").await;
    let login = api(storage, &collection, Some(&users.item.id), "login").await;
    let tree = storage.tree();
    tree.rename(collection.id.clone(), users.item.id.clone(), " people ".into())
        .await
        .expect("rename a folder");
    tree.rename(collection.id.clone(), login.item.id.clone(), "sign in".into())
        .await
        .expect("rename an API");
    let loaded = load(storage, &collection).await;
    assert_eq!(loaded.folders[0].name, "people");
    assert_eq!(loaded.apis[0].name, "sign in");
    assert_eq!(
        loaded.folders[0].children,
        vec![login.item.id.clone()],
        "a rename moves nothing"
    );

    assert_eq!(
        tree.rename(collection.id.clone(), login.item.id, "".into()).await,
        Err(StoreError::NameRequired)
    );
    not_found(
        tree.rename(collection.id.clone(), ItemId::from(MISSING), "x".into())
            .await,
        Entity::Item,
    );
}

async fn requests_are_saved(storage: &Storage) {
    let collection = new_collection(storage, "requests").await;
    let users = folder(storage, &collection, None, "users").await;
    let login = api(storage, &collection, Some(&users.item.id), "login").await;
    let request = HttpRequest {
        method: HttpMethod::Post,
        uri: "{{HOST}}/login".into(),
        content_type: ContentType::Form,
        body: r#"[{"key":"user","value":"中文","enabled":true}]"#.into(),
        query: vec![KvParam {
            key: "lang".into(),
            value: "zh".into(),
            enabled: true,
        }],
        headers: vec![KvParam {
            key: "X-Trace".into(),
            value: "{{md5(a)}}".into(),
            enabled: false,
        }],
        auth: vec![],
    };
    let saved = storage
        .tree()
        .save_request(collection.id.clone(), login.item.id.clone(), request.clone())
        .await
        .expect("save");
    assert_eq!(saved.request, request);
    assert!(saved.updated_at >= login.item.updated_at);
    assert_eq!(saved.created_at, login.item.created_at);
    assert_eq!(load(storage, &collection).await.apis, vec![saved]);

    not_found(
        storage
            .tree()
            .save_request(collection.id.clone(), users.item.id, request)
            .await,
        Entity::Api,
    );
}

async fn moves_follow_the_plan(storage: &Storage) {
    let collection = new_collection(storage, "moves").await;
    let a = folder(storage, &collection, None, "a").await;
    let b = folder(storage, &collection, None, "b").await;
    let x = api(storage, &collection, Some(&a.item.id), "x").await;
    let y = api(storage, &collection, None, "y").await;
    let mut expected = load(storage, &collection).await;
    let tree = storage.tree();

    let moves = [
        (x.item.id.clone(), None, Some(y.item.id.clone())),
        (b.item.id.clone(), Some(a.item.id.clone()), None),
        (y.item.id.clone(), Some(b.item.id.clone()), None),
        (x.item.id.clone(), Some(a.item.id.clone()), Some(b.item.id.clone())),
    ];
    for (item, parent, before) in moves {
        let change = tree
            .move_item(collection.id.clone(), item, Placement { parent, before })
            .await
            .expect("move");
        expected.apply(&change);
        assert_eq!(
            load(storage, &collection).await,
            expected,
            "the store wrote what it answered"
        );
    }
    let built = Tree::build(&expected);
    assert_eq!(built.top_level_ids(), vec![a.item.id.clone()]);
    assert_eq!(
        built.get(&a.item.id).map(|node| node.children().to_vec()),
        Some(vec![x.item.id.clone(), b.item.id.clone()])
    );

    let cycle = tree
        .move_item(
            collection.id.clone(),
            a.item.id.clone(),
            Placement {
                parent: Some(b.item.id.clone()),
                before: None,
            },
        )
        .await;
    assert_eq!(
        cycle,
        Err(StoreError::Tree {
            source: TreeError::Cycle { id: a.item.id }
        })
    );
    not_found(
        tree.move_item(
            collection.id.clone(),
            x.item.id,
            Placement {
                parent: Some(ItemId::from(MISSING)),
                before: None,
            },
        )
        .await,
        Entity::Folder,
    );
    assert_eq!(
        load(storage, &collection).await,
        expected,
        "a refused move changes nothing"
    );
}

async fn deleting_a_folder_takes_its_content(storage: &Storage) {
    let collection = new_collection(storage, "deletes").await;
    let a = folder(storage, &collection, None, "a").await;
    let b = folder(storage, &collection, Some(&a.item.id), "b").await;
    let x = api(storage, &collection, Some(&b.item.id), "x").await;
    let y = api(storage, &collection, None, "y").await;

    let removal = storage
        .tree()
        .delete(collection.id.clone(), a.item.id.clone())
        .await
        .expect("delete");
    assert_eq!(removal.folders, vec![a.item.id.clone(), b.item.id]);
    assert_eq!(removal.apis, vec![x.item.id]);
    let loaded = load(storage, &collection).await;
    assert!(loaded.folders.is_empty());
    assert_eq!(loaded.apis, vec![y.item.clone()]);
    assert!(!loaded.top_order.contains(&a.item.id), "the top-level order forgets it");

    not_found(
        storage.tree().delete(collection.id.clone(), a.item.id).await,
        Entity::Item,
    );
    let removal = storage
        .tree()
        .delete(collection.id.clone(), y.item.id)
        .await
        .expect("delete an API");
    assert!(removal.folders.is_empty());
    assert_eq!(load(storage, &collection).await, TreeData::default());
}

async fn projects_do_not_see_each_other(storage: &Storage) {
    let mine = new_collection(storage, "mine").await;
    let theirs = new_collection(storage, "theirs").await;
    let my_folder = folder(storage, &mine, None, "folder").await;
    let my_api = api(storage, &mine, Some(&my_folder.item.id), "api").await;
    let my_variable = storage
        .variables()
        .create(NewVariable {
            collection: mine.id.clone(),
            category: VariableCategory::Custom,
            name: "token".into(),
            value: "secret".into(),
            enabled: true,
        })
        .await
        .expect("create a variable");
    let other = || theirs.id.clone();
    let tree = storage.tree();

    not_found(
        tree.rename(other(), my_api.item.id.clone(), "x".into()).await,
        Entity::Item,
    );
    not_found(
        tree.save_request(other(), my_api.item.id.clone(), HttpRequest::default())
            .await,
        Entity::Api,
    );
    not_found(
        tree.move_item(other(), my_api.item.id.clone(), Placement::default())
            .await,
        Entity::Item,
    );
    not_found(tree.delete(other(), my_api.item.id.clone()).await, Entity::Item);
    not_found(
        tree.create_api(other(), Some(my_folder.item.id.clone()), "x".into())
            .await,
        Entity::Folder,
    );
    not_found(
        storage
            .variables()
            .update(
                other(),
                my_variable.id.clone(),
                VariableEdit {
                    name: "stolen".into(),
                    ..Default::default()
                },
            )
            .await,
        Entity::Variable,
    );
    storage
        .variables()
        .delete(other(), vec![my_variable.id.clone()])
        .await
        .expect("deleting another project's ids does nothing");

    assert_eq!(load(storage, &theirs).await, TreeData::default());
    let loaded = load(storage, &mine).await;
    assert_eq!(loaded.apis, vec![my_api.item]);
    let variables = storage
        .variables()
        .list(mine.id.clone(), VariableCategory::Custom)
        .await
        .expect("list");
    assert_eq!(variables, vec![my_variable]);
}

/// Only that time does not go backwards: stored times have whole seconds and
/// the suite does not own the backend's clock, so "moved forward" cannot be
/// told from "untouched" here. A backend proves that with its own clock, as
/// `MemoryStore`'s tests do.
async fn content_changes_touch_the_collection(storage: &Storage) {
    let collection = new_collection(storage, "touch").await;
    let login = api(storage, &collection, None, "login").await;
    let before = storage.collections().get(collection.id.clone()).await.expect("get");
    storage
        .tree()
        .save_request(collection.id.clone(), login.item.id, HttpRequest::default())
        .await
        .expect("save");
    let after = storage.collections().get(collection.id).await.expect("get");
    assert!(after.updated_at >= before.updated_at);
    assert_eq!(after.created_at, before.created_at);
}

async fn revision(storage: &Storage, id: &CollectionId) -> u64 {
    storage.collections().get(id.clone()).await.expect("get").revision
}

/// The revision now, which `what` must have moved past `last`.
async fn moved(storage: &Storage, id: &CollectionId, last: u64, what: &str) -> u64 {
    let now = revision(storage, id).await;
    assert!(now > last, "{what} must move the revision past {last}, it is {now}");
    now
}

/// Every change the project's other readers should see moves its revision
/// on (`docs/architecture.md` §7): the browser build polls it to find them.
/// Reading, and a reader's own state, do not.
async fn shared_changes_move_the_revision(storage: &Storage) {
    let collection = new_collection(storage, "revision").await;
    let id = &collection.id;
    let tree = storage.tree();
    let variables = storage.variables();
    let mut last = collection.revision;
    assert_eq!(revision(storage, id).await, last, "created as answered");

    let users = folder(storage, &collection, None, "users").await;
    last = moved(storage, id, last, "a new folder").await;
    let login = api(storage, &collection, None, "login").await;
    last = moved(storage, id, last, "a new API").await;
    tree.rename(id.clone(), login.item.id.clone(), "sign in".into())
        .await
        .expect("rename");
    last = moved(storage, id, last, "a rename").await;
    tree.save_request(id.clone(), login.item.id.clone(), HttpRequest::default())
        .await
        .expect("save");
    last = moved(storage, id, last, "a saved request").await;
    tree.move_item(
        id.clone(),
        login.item.id.clone(),
        Placement {
            parent: Some(users.item.id.clone()),
            before: None,
        },
    )
    .await
    .expect("move");
    last = moved(storage, id, last, "a move").await;
    let variable = variables
        .create(NewVariable {
            collection: id.clone(),
            category: VariableCategory::Environment,
            name: "host".into(),
            value: "a".into(),
            enabled: true,
        })
        .await
        .expect("create a variable");
    last = moved(storage, id, last, "a new variable").await;
    variables
        .update(
            id.clone(),
            variable.id.clone(),
            VariableEdit {
                name: "host".into(),
                value: "b".into(),
                enabled: true,
            },
        )
        .await
        .expect("update a variable");
    last = moved(storage, id, last, "an edited variable").await;
    variables
        .delete(id.clone(), vec![variable.id])
        .await
        .expect("delete a variable");
    last = moved(storage, id, last, "a deleted variable").await;
    let edited = storage
        .collections()
        .update(
            id.clone(),
            CollectionEdit {
                name: "revision, renamed".into(),
                description: String::new(),
            },
        )
        .await
        .expect("update");
    assert!(
        edited.revision > last,
        "a renamed project answers with its new revision"
    );
    last = moved(storage, id, last, "a renamed project").await;
    tree.delete(id.clone(), users.item.id).await.expect("delete");
    last = moved(storage, id, last, "a deletion").await;

    load(storage, &collection).await;
    storage
        .prefs()
        .set_workspace(id.clone(), WorkspacePrefs::default())
        .await
        .expect("set prefs");
    assert_eq!(
        revision(storage, id).await,
        last,
        "reading and a reader's own state change nothing"
    );
}

async fn variables_round_trip(storage: &Storage) {
    let collection = new_collection(storage, "variables").await;
    let variables = storage.variables();
    let create = |category, name: &str, value: &str| NewVariable {
        collection: collection.id.clone(),
        category,
        name: name.into(),
        value: value.into(),
        enabled: true,
    };
    let b = variables
        .create(create(VariableCategory::Environment, "b", "2"))
        .await
        .expect("create");
    let a = variables
        .create(create(VariableCategory::Environment, "a", "1"))
        .await
        .expect("create");
    let c = variables
        .create(create(VariableCategory::Environment, "c", "3"))
        .await
        .expect("create");
    let header = variables
        .create(create(VariableCategory::GlobalRequestHeader, "X-Token", "t"))
        .await
        .expect("create");
    assert_eq!(
        (b.collection.clone(), b.category),
        (collection.id.clone(), VariableCategory::Environment)
    );
    assert!(b.enabled);

    let names = |list: Vec<cyberapi_core::model::Variable>| list.into_iter().map(|v| v.name).collect::<Vec<_>>();
    let env = variables
        .list(collection.id.clone(), VariableCategory::Environment)
        .await
        .expect("list");
    assert_eq!(names(env), ["a", "b", "c"], "listed by name");
    let headers = variables
        .list(collection.id.clone(), VariableCategory::GlobalRequestHeader)
        .await
        .expect("list");
    assert_eq!(headers, vec![header], "categories are kept apart");
    assert!(
        variables
            .list(collection.id.clone(), VariableCategory::Custom)
            .await
            .expect("list")
            .is_empty()
    );

    let updated = variables
        .update(
            collection.id.clone(),
            b.id.clone(),
            VariableEdit {
                name: "b2".into(),
                value: "two".into(),
                enabled: false,
            },
        )
        .await
        .expect("update");
    assert_eq!(
        (updated.name.as_str(), updated.value.as_str(), updated.enabled),
        ("b2", "two", false)
    );
    assert_eq!(updated.category, VariableCategory::Environment);
    assert_eq!(updated.created_at, b.created_at);
    assert!(updated.updated_at >= b.updated_at);
    not_found(
        variables
            .update(
                collection.id.clone(),
                VariableId::from(MISSING),
                VariableEdit::default(),
            )
            .await,
        Entity::Variable,
    );

    variables
        .delete(collection.id.clone(), vec![a.id, c.id])
        .await
        .expect("delete");
    let env = variables
        .list(collection.id.clone(), VariableCategory::Environment)
        .await
        .expect("list");
    assert_eq!(env, vec![updated]);
}

async fn app_prefs_round_trip(storage: &Storage) {
    let mut prefs = AppPrefs {
        theme: ThemeMode::Dark,
        language: Some("zh".into()),
        collection_sort: SortOrder::NameAsc,
        ..Default::default()
    };
    prefs.columns.tree = 280.;
    prefs.timeouts.read = 60;
    storage.prefs().set_app(prefs.clone()).await.expect("set app prefs");
    assert_eq!(storage.prefs().app().await.expect("app prefs"), prefs);
}

async fn workspace_prefs_are_per_collection(storage: &Storage) {
    let first = new_collection(storage, "workspace one").await;
    let second = new_collection(storage, "workspace two").await;
    let login = api(storage, &first, None, "login").await;
    assert_eq!(
        storage.prefs().workspace(first.id.clone()).await.expect("prefs"),
        WorkspacePrefs::default()
    );

    let prefs = WorkspacePrefs {
        expanded: vec![ItemId::from("f1"), ItemId::from("f2")],
        selected: Some(login.item.id.clone()),
        pinned: vec![login.item.id.clone()],
        tabs: [(login.item.id, "headers".to_string())].into_iter().collect(),
    };
    storage
        .prefs()
        .set_workspace(first.id.clone(), prefs.clone())
        .await
        .expect("set");
    assert_eq!(storage.prefs().workspace(first.id).await.expect("prefs"), prefs);
    assert_eq!(
        storage.prefs().workspace(second.id).await.expect("prefs"),
        WorkspacePrefs::default()
    );
}

/// A response of `api`, told apart by `n`, with every field set: what comes
/// back must be what went in.
fn response(collection: &Collection, api: &ItemId, n: usize) -> HttpResponse {
    HttpResponse {
        api: api.clone(),
        collection: collection.id.clone(),
        request: HttpRequest {
            method: HttpMethod::Post,
            uri: format!("{{{{HOST}}}}/n/{n}"),
            body: "{\"a\": 1}".into(),
            ..Default::default()
        },
        status: Some(200 + n as u16),
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("set-cookie".into(), format!("n={n}")),
            ("set-cookie".into(), "b=2".into()),
        ],
        body: vec![b'{', 0, 159, 146, 150, n as u8, b'}'],
        error: n.is_multiple_of(3).then(|| format!("failure {n}")),
        body_size: 7 + n as u64,
        latency: std::time::Duration::from_micros(1500 + n as u64),
        stats: HttpStats {
            remote_addr: "127.0.0.1:80".into(),
            is_https: n.is_multiple_of(2),
            cipher: "TLS13_AES_128_GCM_SHA256".into(),
            dns: std::time::Duration::from_micros(10),
            tcp: std::time::Duration::from_micros(20),
            tls: std::time::Duration::from_micros(30),
            send: std::time::Duration::from_micros(40),
            server_processing: std::time::Duration::from_micros(50),
            content_transfer: std::time::Duration::from_micros(60),
            total: std::time::Duration::from_micros(1500 + n as u64),
        },
        ..Default::default()
    }
}

/// F7.9 – F7.11: ten per API, newest first, kept as they were added; the
/// newest of each for `get`; clearing one API leaves the others; the
/// history goes with its API. Keeping a response is not a change to the
/// project.
async fn history_keeps_the_newest(storage: &Storage, probe: &dyn Probe) {
    let collection = new_collection(storage, "history").await;
    let login = api(storage, &collection, None, "login").await;
    let other = api(storage, &collection, None, "other").await;
    let history = storage.history();
    let before = storage.collections().get(collection.id.clone()).await.expect("get");

    let mut added = Vec::new();
    for n in 0..HISTORY_LIMIT + 2 {
        let kept = history
            .add(response(&collection, &login.item.id, n))
            .await
            .expect("add");
        assert!(!kept.id.is_empty(), "the store gives the id");
        assert_eq!(
            HttpResponse {
                id: String::new(),
                created_at: Default::default(),
                ..kept.clone()
            },
            response(&collection, &login.item.id, n),
            "kept as given, but for the id and the time"
        );
        added.push(kept);
    }
    let mut ids: Vec<&str> = added.iter().map(|kept| kept.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), added.len(), "every response has its own id");
    let other_kept = history
        .add(response(&collection, &other.item.id, 99))
        .await
        .expect("add");

    let listed = history
        .list(collection.id.clone(), login.item.id.clone())
        .await
        .expect("list");
    let newest: Vec<HttpResponse> = added.iter().rev().take(HISTORY_LIMIT).cloned().collect();
    assert_eq!(listed, newest, "the newest, newest first, as they were answered");

    let mut latest = history
        .latest(
            collection.id.clone(),
            vec![login.item.id.clone(), other.item.id.clone(), ItemId::new(MISSING)],
        )
        .await
        .expect("latest");
    latest.sort_by(|a, b| a.api.cmp(&b.api));
    let mut expected = vec![newest[0].clone(), other_kept.clone()];
    expected.sort_by(|a, b| a.api.cmp(&b.api));
    assert_eq!(latest, expected);

    let after = storage.collections().get(collection.id.clone()).await.expect("get");
    assert_eq!(
        (after.revision, after.updated_at),
        (before.revision, before.updated_at),
        "a reader's history is not a change to the project"
    );

    history
        .clear(collection.id.clone(), login.item.id.clone())
        .await
        .expect("clear");
    assert!(
        history
            .list(collection.id.clone(), login.item.id.clone())
            .await
            .expect("list")
            .is_empty()
    );
    assert_eq!(
        history
            .list(collection.id.clone(), other.item.id.clone())
            .await
            .expect("list"),
        [other_kept]
    );

    let rows = probe.rows_of(collection.id.clone()).await.expect("probe");
    storage
        .tree()
        .delete(collection.id.clone(), other.item.id.clone())
        .await
        .expect("delete");
    assert_eq!(
        rows - probe.rows_of(collection.id.clone()).await.expect("probe"),
        2,
        "the API and its one response go"
    );
    not_found(history.list(collection.id, other.item.id).await, Entity::Api);
}

/// A response belongs to one of the project's APIs: not another project's,
/// not a folder.
async fn history_is_only_of_the_projects_apis(storage: &Storage) {
    let collection = new_collection(storage, "history of apis").await;
    let elsewhere = new_collection(storage, "history elsewhere").await;
    let users = folder(storage, &collection, None, "users").await;
    let theirs = api(storage, &elsewhere, None, "theirs").await;
    let history = storage.history();

    not_found(
        history
            .add(HttpResponse {
                collection: CollectionId::new(MISSING),
                ..response(&collection, &theirs.item.id, 1)
            })
            .await,
        Entity::Collection,
    );
    not_found(
        history.add(response(&collection, &theirs.item.id, 1)).await,
        Entity::Api,
    );
    not_found(history.add(response(&collection, &users.item.id, 1)).await, Entity::Api);
    not_found(
        history.list(collection.id.clone(), theirs.item.id.clone()).await,
        Entity::Api,
    );
    not_found(
        history.clear(collection.id.clone(), theirs.item.id.clone()).await,
        Entity::Api,
    );
    assert!(
        history
            .latest(collection.id.clone(), vec![theirs.item.id.clone()])
            .await
            .expect("latest")
            .is_empty(),
        "another project's API has no history here"
    );
    assert!(
        history
            .list(elsewhere.id.clone(), theirs.item.id.clone())
            .await
            .expect("list")
            .is_empty()
    );
}

fn cookie(name: &str, value: &str, domain: &str, path: &str) -> Cookie {
    Cookie {
        name: name.into(),
        value: value.into(),
        domain: domain.into(),
        path: path.into(),
        ..Default::default()
    }
}

/// M9: kept by domain, path and name, listed by name; a newer cookie
/// replaces its namesake, an expired one removes it.
async fn cookies_are_kept_by_key(storage: &Storage) {
    let cookies = storage.cookies();
    cookies.clear().await.expect("clear");
    let lasting = Cookie {
        expires: Some(cyberapi_core::Timestamp::parse_lenient("2099-01-02T03:04:05Z")),
        secure: true,
        http_only: true,
        host_only: true,
        ..cookie("x", "3", "other.test", "/")
    };
    cookies
        .put(vec![
            cookie("sid", "2", "example.com", "/v1"),
            lasting.clone(),
            cookie("sid", "1", "example.com", "/"),
        ])
        .await
        .expect("put");
    assert_eq!(
        cookies.list().await.expect("list"),
        [
            cookie("sid", "1", "example.com", "/"),
            cookie("sid", "2", "example.com", "/v1"),
            lasting.clone(),
        ],
        "by name, then domain and path, as they were put"
    );

    let expired = Cookie {
        expires: Some(cyberapi_core::Timestamp::parse_lenient("1970-01-01T00:00:01Z")),
        ..cookie("sid", "", "example.com", "/v1")
    };
    cookies
        .put(vec![cookie("sid", "9", "example.com", "/"), expired])
        .await
        .expect("put");
    assert_eq!(
        cookies.list().await.expect("list"),
        [cookie("sid", "9", "example.com", "/"), lasting.clone()],
        "replaced, and removed by an expired one"
    );

    cookies
        .delete("other.test".into(), "/".into(), "x".into())
        .await
        .expect("delete");
    cookies
        .delete("nowhere.test".into(), "/".into(), "x".into())
        .await
        .expect("deleting what is not kept");
    assert_eq!(
        cookies.list().await.expect("list"),
        [cookie("sid", "9", "example.com", "/")]
    );
    cookies.clear().await.expect("clear");
    assert!(cookies.list().await.expect("list").is_empty());
}
