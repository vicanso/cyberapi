//! What every backend must do the same way (`docs/architecture.md` §4.2).
//!
//! A backend's tests hand [`run`] a [`Storage`] and a [`Probe`], and drive it
//! with their own executor. The checks share that storage: each makes its own
//! projects and looks only at those, so they can run against a database that
//! already holds data. A failure names the check through its file and line.

use crate::{
    CollectionEdit, Entity, Imported, Inserted, NewCollection, NewVariable, Refusal, Storage, StoreError, StoreFuture,
    VariableEdit,
};
use cyberapi_core::access::CollectionRole;
use cyberapi_core::auth::{Auth, KeyPlace};
use cyberapi_core::collection::SortOrder;
use cyberapi_core::cookie::Cookie;
use cyberapi_core::import::ImportItem;
use cyberapi_core::model::{
    ApiCategory, ApiSetting, Collection, ContentType, Folder, FolderSettings, HttpMethod, HttpRequest, KvParam,
    ProjectSettings, VariableCategory,
};
use cyberapi_core::prefs::{AppPrefs, HttpVersion, RequestSettings, ThemeMode, Timeouts, WorkspacePrefs};
use cyberapi_core::response::{HISTORY_LIMIT, HttpResponse, HttpStats};
use cyberapi_core::sync::ProjectConfig;
use cyberapi_core::tree::{Placement, Tree, TreeData, TreeError};
use cyberapi_core::{CollectionId, EnvironmentId, ItemId, VariableId};

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
    api_counts_are_per_project(storage).await;
    new_items_are_placed_as_planned(storage).await;
    bad_parents_are_refused(storage).await;
    items_are_renamed(storage).await;
    requests_are_saved(storage).await;
    settings_are_kept(storage).await;
    deleting_a_folder_takes_its_settings(storage, probe).await;
    moves_follow_the_plan(storage).await;
    deleting_a_folder_takes_its_content(storage).await;
    projects_do_not_see_each_other(storage).await;
    content_changes_touch_the_collection(storage).await;
    shared_changes_move_the_revision(storage).await;
    variables_round_trip(storage).await;
    environments_round_trip(storage, probe).await;
    app_prefs_round_trip(storage).await;
    workspace_prefs_are_per_collection(storage).await;
    history_keeps_the_newest(storage, probe).await;
    history_is_only_of_the_projects_apis(storage).await;
    cookies_are_kept_by_key(storage).await;
    imports_get_new_ids_at_the_end(storage).await;
    the_readers_state_clears(storage).await;
    configs_are_read_whole(storage).await;
    configs_replace_at_their_base(storage).await;
    configs_keep_their_ids_to_themselves(storage).await;
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

async fn api_counts_are_per_project(storage: &Storage) {
    let counted = new_collection(storage, "counted").await;
    let empty = new_collection(storage, "no APIs").await;
    let group = folder(storage, &counted, None, "group").await;
    api(storage, &counted, None, "top").await;
    let nested = api(storage, &counted, Some(&group.item.id), "nested").await;
    folder(storage, &empty, None, "only a folder").await;
    let counts = storage.collections().api_counts().await.expect("counts");
    assert_eq!(counts.get(&counted.id), Some(&2), "an API in a folder counts");
    assert_eq!(counts.get(&empty.id), None, "a project with no API is not there");

    storage
        .tree()
        .delete(counted.id.clone(), nested.item.id)
        .await
        .expect("delete an API");
    let counts = storage.collections().api_counts().await.expect("counts");
    assert_eq!(counts.get(&counted.id), Some(&1), "a deleted API does not count");
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
            environment: None,
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
    let tree = storage.tree();
    let none = || FolderSettings {
        auth: Auth::None,
        ..Default::default()
    };
    tree.set_folder_settings(doomed.id.clone(), users.item.id.clone(), none())
        .await
        .expect("a folder's settings");
    tree.set_project_settings(doomed.id.clone(), ProjectSettings { auth: Auth::None })
        .await
        .expect("the project's settings");
    let dev = storage
        .variables()
        .create_environment(doomed.id.clone(), "dev".into())
        .await
        .expect("an environment");
    storage
        .variables()
        .create(NewVariable {
            collection: doomed.id.clone(),
            category: VariableCategory::Custom,
            name: "token".into(),
            value: "dev".into(),
            enabled: true,
            environment: Some(dev.id),
        })
        .await
        .expect("a variable in it");
    let kept_rows = probe.rows_of(kept.id.clone()).await.expect("probe");
    // The project row, the folder, the API, the variable, the workspace
    // state, the response, the project's settings — and the top-level order
    // and the folder's settings wherever the backend keeps them apart.
    assert!(
        probe.rows_of(doomed.id.clone()).await.expect("probe") >= 7,
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
        // How it signs in (#66).
        auth: Auth::ApiKey {
            name: "X-Key".into(),
            value: "{{value(key)}}".into(),
            place: KeyPlace::Query,
        },
        // An API's own sending settings (#71).
        settings: RequestSettings {
            timeouts: Timeouts {
                connect: 3,
                ..Timeouts::default()
            },
            follow_redirects: Some(false),
            accept_invalid_certs: None,
            http_version: Some(HttpVersion::Http2),
        },
        // What its answers are checked against (A1).
        assertions: vec![cyberapi_core::assertions::Assertion {
            subject: cyberapi_core::assertions::Subject::Json,
            target: "$.token".into(),
            op: cyberapi_core::assertions::Op::Exists,
            ..Default::default()
        }],
        // What a newer version keeps in a request comes back as it went
        // (`docs/batch4-design.md` §2.1).
        extra: HttpRequest::from_json_lenient(r#"{"laterField": {"kept": [1, "two"]}}"#).extra,
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

/// #66: what a folder and the project set for their APIs, read back with
/// the tree and the configuration; none again is none.
async fn settings_are_kept(storage: &Storage) {
    let collection = new_collection(storage, "settings").await;
    let users = folder(storage, &collection, None, "users").await;
    let admin = folder(storage, &collection, Some(&users.item.id), "admin").await;
    let tree = storage.tree();
    let settings = FolderSettings {
        auth: Auth::Bearer {
            token: "{{value(token)}}".into(),
        },
        headers: vec![KvParam {
            key: "X-Team".into(),
            value: "users".into(),
            enabled: true,
        }],
    };
    let set = tree
        .set_folder_settings(collection.id.clone(), users.item.id.clone(), settings.clone())
        .await
        .expect("set a folder's");
    assert_eq!(set.settings, settings);
    assert_eq!((set.id.clone(), set.name.as_str()), (users.item.id.clone(), "users"));
    assert!(set.updated_at >= users.item.updated_at);
    let project = ProjectSettings {
        auth: Auth::Basic {
            username: "me".into(),
            password: "pw".into(),
        },
    };
    tree.set_project_settings(collection.id.clone(), project.clone())
        .await
        .expect("set the project's");
    let loaded = load(storage, &collection).await;
    let found = |data: &TreeData, id: &ItemId| {
        data.folders
            .iter()
            .find(|folder| &folder.id == id)
            .cloned()
            .expect("the folder")
    };
    assert_eq!(found(&loaded, &users.item.id), set, "the folder as answered");
    assert!(found(&loaded, &admin.item.id).settings.is_empty(), "only that folder's");
    assert_eq!(loaded.settings, project);
    let read = tree.config(collection.id.clone()).await.expect("config");
    assert_eq!(read.config.tree(), loaded, "the configuration has them too");

    tree.set_folder_settings(collection.id.clone(), users.item.id.clone(), FolderSettings::default())
        .await
        .expect("none again");
    tree.set_project_settings(collection.id.clone(), ProjectSettings::default())
        .await
        .expect("none again");
    let loaded = load(storage, &collection).await;
    assert!(found(&loaded, &users.item.id).settings.is_empty());
    assert!(loaded.settings.is_empty());

    let api = api(storage, &collection, None, "an API").await;
    not_found(
        tree.set_folder_settings(collection.id.clone(), api.item.id, settings.clone())
            .await,
        Entity::Item,
    );
    not_found(
        tree.set_folder_settings(collection.id.clone(), ItemId::from(MISSING), settings.clone())
            .await,
        Entity::Item,
    );
    not_found(
        tree.set_folder_settings(CollectionId::new(MISSING), users.item.id, settings)
            .await,
        Entity::Collection,
    );
    not_found(
        tree.set_project_settings(CollectionId::new(MISSING), project).await,
        Entity::Collection,
    );
}

/// #66: a folder's settings go with it, wherever a backend keeps them.
async fn deleting_a_folder_takes_its_settings(storage: &Storage, probe: &dyn Probe) {
    let collection = new_collection(storage, "settings go").await;
    let users = folder(storage, &collection, None, "users").await;
    let admin = folder(storage, &collection, Some(&users.item.id), "admin").await;
    let before = probe.rows_of(collection.id.clone()).await.expect("probe");
    for id in [&users.item.id, &admin.item.id] {
        storage
            .tree()
            .set_folder_settings(
                collection.id.clone(),
                id.clone(),
                FolderSettings {
                    auth: Auth::None,
                    ..Default::default()
                },
            )
            .await
            .expect("settings");
    }
    storage
        .tree()
        .delete(collection.id.clone(), users.item.id)
        .await
        .expect("delete");
    assert_eq!(
        probe.rows_of(collection.id.clone()).await.expect("probe"),
        before - 2,
        "the two folders go, and nothing of their settings stays"
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
            environment: None,
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
            environment: None,
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
                environment: None,
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
    tree.import(
        id.clone(),
        None,
        vec![ImportItem::Api {
            name: "imported".into(),
            request: Box::default(),
        }],
    )
    .await
    .expect("import");
    last = moved(storage, id, last, "an import").await;
    tree.set_folder_settings(
        id.clone(),
        users.item.id.clone(),
        FolderSettings {
            auth: Auth::None,
            ..Default::default()
        },
    )
    .await
    .expect("a folder's settings");
    last = moved(storage, id, last, "a folder's settings").await;
    tree.set_project_settings(id.clone(), ProjectSettings { auth: Auth::None })
        .await
        .expect("the project's settings");
    last = moved(storage, id, last, "the project's settings").await;
    let environment = variables
        .create_environment(id.clone(), "dev".into())
        .await
        .expect("an environment");
    last = moved(storage, id, last, "a new environment").await;
    variables
        .rename_environment(id.clone(), environment.id.clone(), "qa".into())
        .await
        .expect("renamed");
    last = moved(storage, id, last, "a renamed environment").await;
    variables
        .reorder_environments(id.clone(), vec![environment.id.clone()])
        .await
        .expect("reordered");
    last = moved(storage, id, last, "environments reordered").await;
    variables
        .delete_environment(id.clone(), environment.id)
        .await
        .expect("deleted");
    last = moved(storage, id, last, "a deleted environment").await;
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
        environment: None,
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
                environment: None,
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

/// #61: a project's named environments — named once each, in an order of
/// the readers' making — and the variables in them, which go with them.
async fn environments_round_trip(storage: &Storage, probe: &dyn Probe) {
    let collection = new_collection(storage, "environments").await;
    let id = collection.id.clone();
    let variables = storage.variables();
    assert!(variables.environments(id.clone()).await.expect("none yet").is_empty());
    let dev = variables
        .create_environment(id.clone(), " dev ".into())
        .await
        .expect("dev");
    assert_eq!((dev.name.as_str(), &dev.collection), ("dev", &id));
    let prod = variables
        .create_environment(id.clone(), "prod".into())
        .await
        .expect("prod");
    assert_eq!(
        variables.environments(id.clone()).await.expect("list"),
        vec![dev.clone(), prod.clone()],
        "in the order made"
    );
    assert_eq!(
        variables.create_environment(id.clone(), "  ".into()).await,
        Err(StoreError::NameRequired)
    );
    let taken = Err(StoreError::Refused {
        reason: Refusal::EnvironmentNameTaken,
    });
    assert_eq!(variables.create_environment(id.clone(), "DEV".into()).await, taken);

    let staging = variables
        .rename_environment(id.clone(), prod.id.clone(), "staging".into())
        .await
        .expect("rename");
    assert_eq!(
        (staging.name.as_str(), &staging.created_at),
        ("staging", &prod.created_at)
    );
    assert_eq!(
        variables
            .rename_environment(id.clone(), prod.id.clone(), "Dev".into())
            .await,
        taken
    );
    variables
        .rename_environment(id.clone(), prod.id.clone(), "Staging".into())
        .await
        .expect("its own name, cased otherwise");
    let reordered = variables
        .reorder_environments(id.clone(), vec![prod.id.clone(), EnvironmentId::from(MISSING)])
        .await
        .expect("reorder");
    assert_eq!(
        reordered.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["Staging", "dev"]
    );
    assert_eq!(variables.environments(id.clone()).await.expect("list"), reordered);

    // Variables in one, and shared.
    let new = |name: &str, environment: Option<&EnvironmentId>| NewVariable {
        collection: id.clone(),
        category: VariableCategory::Environment,
        name: name.into(),
        value: format!("{name} value"),
        enabled: true,
        environment: environment.cloned(),
    };
    let shared = variables.create(new("HOST", None)).await.expect("shared");
    let mine = variables.create(new("HOST", Some(&dev.id))).await.expect("dev's");
    assert_eq!(mine.environment.as_ref(), Some(&dev.id));
    not_found(
        variables.create(new("HOST", Some(&EnvironmentId::from(MISSING)))).await,
        Entity::Environment,
    );
    let listed = variables
        .list(id.clone(), VariableCategory::Environment)
        .await
        .expect("list");
    assert_eq!(listed.len(), 2);
    assert!(
        listed.contains(&mine) && listed.contains(&shared),
        "each with its environment"
    );
    // One moves into the other environment, and back to shared.
    let moved = variables
        .update(
            id.clone(),
            mine.id.clone(),
            VariableEdit {
                name: mine.name.clone(),
                value: mine.value.clone(),
                enabled: true,
                environment: Some(prod.id.clone()),
            },
        )
        .await
        .expect("move");
    assert_eq!(moved.environment.as_ref(), Some(&prod.id));
    let back = variables
        .update(
            id.clone(),
            mine.id.clone(),
            VariableEdit {
                name: mine.name.clone(),
                value: mine.value.clone(),
                enabled: true,
                environment: None,
            },
        )
        .await
        .expect("shared again");
    assert_eq!(back.environment, None);
    not_found(
        variables
            .update(
                id.clone(),
                mine.id.clone(),
                VariableEdit {
                    environment: Some(EnvironmentId::from(MISSING)),
                    ..VariableEdit::default()
                },
            )
            .await,
        Entity::Environment,
    );
    // The configuration has them.
    let read = storage.tree().config(id.clone()).await.expect("config");
    assert_eq!(read.config.environments, reordered);

    // A variable deleted leaves nothing of where it was.
    let before = probe.rows_of(id.clone()).await.expect("probe");
    let gone = variables.create(new("TOKEN", Some(&dev.id))).await.expect("dev's");
    variables.delete(id.clone(), vec![gone.id]).await.expect("delete");
    assert_eq!(probe.rows_of(id.clone()).await.expect("probe"), before);

    // An environment deleted takes its variables; the shared ones stay.
    variables
        .update(
            id.clone(),
            mine.id.clone(),
            VariableEdit {
                name: mine.name.clone(),
                value: mine.value.clone(),
                enabled: true,
                environment: Some(dev.id.clone()),
            },
        )
        .await
        .expect("into dev again");
    let before = probe.rows_of(id.clone()).await.expect("probe");
    variables
        .delete_environment(id.clone(), dev.id.clone())
        .await
        .expect("delete dev");
    let left = variables
        .list(id.clone(), VariableCategory::Environment)
        .await
        .expect("list");
    assert_eq!(left, vec![shared], "dev's went with it");
    assert!(
        probe.rows_of(id.clone()).await.expect("probe") < before - 1,
        "the environment's row and its variable's are gone"
    );
    assert_eq!(
        variables
            .environments(id.clone())
            .await
            .expect("list")
            .into_iter()
            .map(|environment| environment.id)
            .collect::<Vec<_>>(),
        vec![prod.id.clone()]
    );

    for missing in [
        variables
            .rename_environment(id.clone(), dev.id.clone(), "again".into())
            .await
            .map(|_| ()),
        variables.delete_environment(id.clone(), dev.id.clone()).await,
    ] {
        not_found(missing, Entity::Environment);
    }
    let nowhere = CollectionId::new(MISSING);
    not_found(variables.environments(nowhere.clone()).await, Entity::Collection);
    not_found(
        variables.create_environment(nowhere.clone(), "x".into()).await,
        Entity::Collection,
    );
    not_found(
        variables.reorder_environments(nowhere, vec![]).await,
        Entity::Collection,
    );
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
        filters: [(login.item.id.clone(), "$.data[*].name".to_string())]
            .into_iter()
            .collect(),
        tabs: [(login.item.id, "headers".to_string())].into_iter().collect(),
        // The reader's environment (#61).
        environment: Some(EnvironmentId::from("e1")),
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
            version: "HTTP/2".into(),
            cipher: "TLS13_AES_128_GCM_SHA256".into(),
            dns: std::time::Duration::from_micros(10),
            tcp: std::time::Duration::from_micros(20),
            tls: std::time::Duration::from_micros(30),
            send: std::time::Duration::from_micros(40),
            server_processing: std::time::Duration::from_micros(50),
            content_transfer: std::time::Duration::from_micros(60),
            total: std::time::Duration::from_micros(1500 + n as u64),
            proxy: if n.is_multiple_of(2) {
                "http://proxy.test:3128".into()
            } else {
                String::new()
            },
        },
        truncated: n.is_multiple_of(4),
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

fn bundle() -> Vec<ImportItem> {
    vec![
        ImportItem::Folder {
            name: "orders".into(),
            settings: FolderSettings {
                auth: Auth::None,
                ..Default::default()
            },
            children: vec![
                ImportItem::Api {
                    name: "create".into(),
                    request: Box::new(HttpRequest {
                        method: HttpMethod::Post,
                        uri: "{{HOST}}/orders".into(),
                        body: "{\"a\": 1}".into(),
                        ..Default::default()
                    }),
                },
                ImportItem::Folder {
                    name: "admin".into(),
                    settings: FolderSettings::default(),
                    children: vec![ImportItem::Api {
                        name: "purge".into(),
                        request: Box::new(HttpRequest {
                            method: HttpMethod::Delete,
                            ..Default::default()
                        }),
                    }],
                },
            ],
        },
        ImportItem::Api {
            name: "health".into(),
            request: Box::default(),
        },
    ]
}

/// F10.4: every item gets a new id, so the same bundle imports twice (#7);
/// what is imported goes at the end of where it goes (#9); the answer is
/// what was written.
async fn imports_get_new_ids_at_the_end(storage: &Storage) {
    let collection = new_collection(storage, "imports").await;
    let tree = storage.tree();
    let first = api(storage, &collection, None, "first").await;
    let users = folder(storage, &collection, None, "users").await;
    let before = Tree::build(&load(storage, &collection).await).top_level_ids();

    let mut expected = load(storage, &collection).await;
    let imported: Imported = tree
        .import(collection.id.clone(), None, bundle())
        .await
        .expect("import");
    expected.folders.extend(imported.folders.clone());
    expected.apis.extend(imported.apis.clone());
    expected.apply(&imported.change);
    let loaded = load(storage, &collection).await;
    assert_eq!(loaded, expected, "the answer is what was written");

    let names = |ids: &[ItemId], data: &TreeData| -> Vec<String> {
        ids.iter()
            .map(|id| {
                data.folders
                    .iter()
                    .find(|folder| &folder.id == id)
                    .map(|folder| folder.name.clone())
                    .or_else(|| data.apis.iter().find(|api| &api.id == id).map(|api| api.name.clone()))
                    .unwrap_or_default()
            })
            .collect()
    };
    let top = Tree::build(&loaded).top_level_ids();
    assert_eq!(&top[..before.len()], &before[..], "what was there stays first");
    assert_eq!(
        names(&top[before.len()..], &loaded),
        ["orders", "health"],
        "the import at the end"
    );
    let orders = loaded
        .folders
        .iter()
        .find(|folder| folder.name == "orders")
        .expect("orders");
    assert_eq!(names(&orders.children, &loaded), ["create", "admin"]);
    assert_eq!(orders.settings.auth, Auth::None, "a folder brings its settings (#66)");
    let create = loaded.apis.iter().find(|api| api.name == "create").expect("create");
    assert_eq!(
        (create.request.method, create.request.uri.as_str()),
        (HttpMethod::Post, "{{HOST}}/orders")
    );
    assert_eq!(create.collection, collection.id);

    // Again, into a folder: new ids, the folder's end.
    let again = tree
        .import(collection.id.clone(), Some(users.item.id.clone()), bundle())
        .await
        .expect("import again");
    let first_ids: Vec<&ItemId> = imported
        .folders
        .iter()
        .map(|folder| &folder.id)
        .chain(imported.apis.iter().map(|api| &api.id))
        .collect();
    assert!(
        again
            .folders
            .iter()
            .map(|folder| &folder.id)
            .chain(again.apis.iter().map(|api| &api.id))
            .all(|id| !first_ids.contains(&id) && id != &first.item.id),
        "every item has a new id"
    );
    let loaded = load(storage, &collection).await;
    let users_now = loaded
        .folders
        .iter()
        .find(|folder| folder.id == users.item.id)
        .expect("users");
    assert_eq!(names(&users_now.children, &loaded), ["orders", "health"]);
    assert_eq!(loaded.apis.len(), 7);

    not_found(
        tree.import(collection.id.clone(), Some(ItemId::from(MISSING)), bundle())
            .await,
        Entity::Folder,
    );
    not_found(
        tree.import(CollectionId::from(MISSING), None, bundle()).await,
        Entity::Collection,
    );
    assert_eq!(load(storage, &collection).await.apis.len(), 7, "nothing was half made");
}

/// M13, for a store that backs up: a restore brings back what the snapshot
/// held and only that.
/// F15.3: the reader forgets its history of every project at once, and its
/// state of every project (pins, open folders, the selection); the app's
/// settings stay.
async fn the_readers_state_clears(storage: &Storage) {
    let first = new_collection(storage, "clear one").await;
    let second = new_collection(storage, "clear two").await;
    let (a, b) = (
        api(storage, &first, None, "a").await,
        api(storage, &second, None, "b").await,
    );
    let history = storage.history();
    history.add(response(&first, &a.item.id, 1)).await.expect("add");
    history.add(response(&second, &b.item.id, 2)).await.expect("add");
    let pinned = WorkspacePrefs {
        pinned: vec![a.item.id.clone()],
        selected: Some(a.item.id.clone()),
        ..Default::default()
    };
    storage
        .prefs()
        .set_workspace(first.id.clone(), pinned)
        .await
        .expect("set");
    let app = AppPrefs {
        theme: ThemeMode::Dark,
        ..Default::default()
    };
    storage.prefs().set_app(app.clone()).await.expect("set app prefs");

    history.clear_all().await.expect("clear the history");
    for (collection, api) in [(&first, &a), (&second, &b)] {
        let left = history
            .list(collection.id.clone(), api.item.id.clone())
            .await
            .expect("list");
        assert!(left.is_empty(), "no project keeps a response");
    }
    storage.prefs().clear_workspaces().await.expect("clear the state");
    assert_eq!(
        storage.prefs().workspace(first.id.clone()).await.expect("prefs"),
        WorkspacePrefs::default()
    );
    assert_eq!(
        storage.prefs().app().await.expect("app prefs"),
        app,
        "the settings stay"
    );
    // The projects themselves stay too.
    assert_eq!(load(storage, &first).await.apis.len(), 1);
}

pub async fn backup(storage: &Storage, backups: &dyn crate::BackupStore) {
    let kept = new_collection(storage, "backed up").await;
    let users = folder(storage, &kept, None, "users").await;
    let login = api(storage, &kept, Some(&users.item.id), "login").await;
    storage
        .tree()
        .save_request(
            kept.id.clone(),
            login.item.id.clone(),
            HttpRequest {
                method: HttpMethod::Post,
                uri: "/login".into(),
                ..Default::default()
            },
        )
        .await
        .expect("save");
    storage
        .variables()
        .create(NewVariable {
            collection: kept.id.clone(),
            category: VariableCategory::Environment,
            name: "HOST".into(),
            value: "https://a.test".into(),
            enabled: true,
            environment: None,
        })
        .await
        .expect("a variable");
    storage
        .history()
        .add(response(&kept, &login.item.id, 1))
        .await
        .expect("a response");
    // What the folder and the project set (#66) is backed up with them.
    storage
        .tree()
        .set_folder_settings(
            kept.id.clone(),
            users.item.id.clone(),
            FolderSettings {
                auth: Auth::Bearer { token: "t".into() },
                ..Default::default()
            },
        )
        .await
        .expect("a folder's settings");
    storage
        .tree()
        .set_project_settings(kept.id.clone(), ProjectSettings { auth: Auth::None })
        .await
        .expect("the project's settings");
    // So are its environments and the variables in them (#61).
    let dev = storage
        .variables()
        .create_environment(kept.id.clone(), "dev".into())
        .await
        .expect("an environment");
    storage
        .variables()
        .create(NewVariable {
            collection: kept.id.clone(),
            category: VariableCategory::Environment,
            name: "HOST".into(),
            value: "https://dev.test".into(),
            enabled: true,
            environment: Some(dev.id.clone()),
        })
        .await
        .expect("a variable in it");
    let snapshot = backups.snapshot().await.expect("snapshot");
    let before_list = storage.collections().list().await.expect("list");
    let before_tree = load(storage, &kept).await;
    let before_variables = storage
        .variables()
        .list(kept.id.clone(), VariableCategory::Environment)
        .await
        .expect("variables");
    assert!(snapshot.collections.iter().any(|collection| collection.id == kept.id));
    assert!(snapshot.apis.iter().any(|found| found.id == login.item.id));

    // Afterwards: the project goes, another comes.
    storage.collections().delete(kept.id.clone()).await.expect("delete");
    let later = new_collection(storage, "made later").await;

    backups.restore(snapshot.clone()).await.expect("restore");
    assert_eq!(storage.collections().list().await.expect("list"), before_list);
    assert_eq!(load(storage, &kept).await, before_tree);
    assert!(
        !before_tree.settings.is_empty() && before_tree.folders.iter().all(|folder| !folder.settings.is_empty()),
        "the settings came back"
    );
    assert_eq!(
        storage.variables().environments(kept.id.clone()).await.expect("list"),
        vec![dev.clone()]
    );
    assert!(
        before_variables
            .iter()
            .any(|variable| variable.environment.as_ref() == Some(&dev.id)),
        "the variable in it came back in it"
    );
    assert_eq!(
        storage
            .variables()
            .list(kept.id.clone(), VariableCategory::Environment)
            .await
            .expect("variables"),
        before_variables
    );
    not_found(storage.collections().get(later.id).await, Entity::Collection);
    assert!(
        storage
            .history()
            .list(kept.id.clone(), login.item.id.clone())
            .await
            .expect("history")
            .is_empty(),
        "kept responses go with a restore"
    );
    assert_eq!(
        backups.snapshot().await.expect("snapshot"),
        snapshot,
        "a restore is the snapshot"
    );
}

/// A configuration with its records in id order: a store keeps them in an
/// order of its own (the one rows were written in), which the tree and the
/// variables' lists do not read.
fn by_id(mut config: ProjectConfig) -> ProjectConfig {
    config.folders.sort_by(|a, b| a.id.cmp(&b.id));
    config.apis.sort_by(|a, b| a.id.cmp(&b.id));
    config.variables.sort_by(|a, b| a.id.cmp(&b.id));
    config
}

/// A project's configuration is everything its readers share, read at once
/// with its revision (server-sync §2, §3.1).
async fn configs_are_read_whole(storage: &Storage) {
    let collection = new_collection(storage, "config read").await;
    let users = folder(storage, &collection, None, "users").await;
    api(storage, &collection, Some(&users.item.id), "login").await;
    api(storage, &collection, None, "health").await;
    for category in [
        VariableCategory::Environment,
        VariableCategory::Custom,
        VariableCategory::GlobalRequestHeader,
    ] {
        storage
            .variables()
            .create(NewVariable {
                collection: collection.id.clone(),
                category,
                name: format!("{category:?}"),
                value: "v".into(),
                enabled: true,
                environment: None,
            })
            .await
            .expect("create a variable");
    }

    let read = storage.tree().config(collection.id.clone()).await.expect("config");
    let now = storage.collections().get(collection.id.clone()).await.expect("get");
    assert_eq!(read.revision, now.revision, "read with the revision it is at");
    assert_eq!(
        (read.config.name.as_str(), read.config.description.as_str()),
        (now.name.as_str(), now.description.as_str())
    );
    let tree = load(storage, &collection).await;
    assert_eq!(read.config.tree(), tree, "the tree as load reads it");
    assert_eq!(read.config.variables.len(), 3, "variables of every category");
    not_found(
        storage.tree().config(CollectionId::new(MISSING)).await,
        Entity::Collection,
    );
}

/// A configuration replaces the project's whole, only at the base it was
/// read at, keeping the ids it brings and the history of the APIs that stay
/// (server-sync §3.3).
async fn configs_replace_at_their_base(storage: &Storage) {
    let collection = new_collection(storage, "config replace").await;
    let users = folder(storage, &collection, None, "users").await;
    let login = api(storage, &collection, Some(&users.item.id), "login").await;
    let gone = api(storage, &collection, None, "gone").await;
    let token = storage
        .variables()
        .create(NewVariable {
            collection: collection.id.clone(),
            category: VariableCategory::Environment,
            name: "HOST".into(),
            value: "https://a.test".into(),
            enabled: true,
            environment: None,
        })
        .await
        .expect("create a variable");
    let history = storage.history();
    history
        .add(response(&collection, &login.item.id, 1))
        .await
        .expect("history");
    history
        .add(response(&collection, &gone.item.id, 2))
        .await
        .expect("history");
    let read = storage.tree().config(collection.id.clone()).await.expect("config");

    // Renamed, rewritten and reordered; one API gone, one new with an id of
    // the copy's making; the variable changed, one more beside it.
    let mut wanted = read.config.clone();
    wanted.description = "synced".into();
    let at = login.item.created_at;
    let made = ItemId::new(format!("{}-made-here", collection.id));
    wanted.apis.retain(|found| found.id != gone.item.id);
    for found in &mut wanted.apis {
        found.request.uri = "{{HOST}}/login".into();
    }
    wanted.apis.push(ApiSetting {
        id: made.clone(),
        collection: collection.id.clone(),
        name: "made here".into(),
        category: ApiCategory::Http,
        request: HttpRequest {
            method: HttpMethod::Post,
            ..Default::default()
        },
        created_at: at,
        updated_at: at,
    });
    wanted.folders[0].name = "people".into();
    wanted.folders[0].children = vec![made.clone(), login.item.id.clone()];
    // What the folder and the project set (#66) comes as given too.
    wanted.folders[0].settings.auth = Auth::Bearer { token: "t".into() };
    wanted.settings.auth = Auth::None;
    // Named environments (#61), and a variable in one, likewise.
    let environment = cyberapi_core::model::Environment {
        id: EnvironmentId::new(format!("{}-environment", collection.id)),
        collection: collection.id.clone(),
        name: "dev".into(),
        created_at: at,
        updated_at: at,
    };
    wanted.environments = vec![environment.clone()];
    wanted.top_order = vec![users.item.id.clone()];
    wanted.variables[0].value = "https://b.test".into();
    let mut second = wanted.variables[0].clone();
    second.id = VariableId::new(format!("{}-variable", collection.id));
    second.name = "TOKEN".into();
    second.environment = Some(environment.id.clone());
    wanted.variables.push(second);

    // Stale: nothing written.
    match storage
        .tree()
        .replace(collection.id.clone(), wanted.clone(), Some(read.revision + 1))
        .await
    {
        Err(StoreError::Stale { base, revision }) => {
            assert_eq!((base, revision), (read.revision + 1, read.revision));
        }
        other => panic!("a stale base is refused, got {other:?}"),
    }
    assert_eq!(
        storage.tree().config(collection.id.clone()).await.expect("config"),
        read,
        "a refused replace changes nothing"
    );

    // At its base: written whole, the revision once on.
    let revision = storage
        .tree()
        .replace(collection.id.clone(), wanted.clone(), Some(read.revision))
        .await
        .expect("replace at the base");
    assert_eq!(revision, read.revision + 1, "the revision moves on once");
    let now = storage.tree().config(collection.id.clone()).await.expect("config");
    assert_eq!(now.revision, revision);
    assert_eq!(
        by_id(now.config.clone()),
        by_id(wanted.clone()),
        "the configuration as given"
    );
    assert!(
        now.config.variables.iter().any(|variable| variable.id == token.id),
        "an id given is kept"
    );
    assert_eq!(
        history
            .list(collection.id.clone(), login.item.id.clone())
            .await
            .expect("history")
            .len(),
        1,
        "an API that stays keeps its history"
    );
    not_found(
        history.list(collection.id.clone(), gone.item.id.clone()).await,
        Entity::Api,
    );

    // Forced: whatever revision it is at.
    let mut mine = wanted.clone();
    mine.apis.retain(|found| found.id == made);
    mine.folders[0].children = vec![made.clone()];
    mine.folders[0].settings = FolderSettings::default();
    mine.settings = ProjectSettings::default();
    mine.environments.clear();
    mine.variables.retain(|variable| variable.environment.is_none());
    let forced = storage
        .tree()
        .replace(collection.id.clone(), mine.clone(), None)
        .await
        .expect("a forced replace");
    assert_eq!(forced, revision + 1);
    let now = storage.tree().config(collection.id.clone()).await.expect("config");
    assert_eq!(by_id(now.config), by_id(mine.clone()));

    // A nameless item is refused, and nothing written.
    let mut nameless = mine.clone();
    nameless.apis[0].name = "  ".into();
    assert_eq!(
        storage.tree().replace(collection.id.clone(), nameless, None).await,
        Err(StoreError::NameRequired)
    );
    assert_eq!(revision_of_config(storage, &collection).await, forced);
    not_found(
        storage.tree().replace(CollectionId::new(MISSING), mine, None).await,
        Entity::Collection,
    );
}

async fn revision_of_config(storage: &Storage, collection: &Collection) -> u64 {
    storage
        .tree()
        .config(collection.id.clone())
        .await
        .expect("config")
        .revision
}

/// An id another project's item or variable has cannot be brought in (the
/// ids are the server's, one space for every project).
async fn configs_keep_their_ids_to_themselves(storage: &Storage) {
    let theirs = new_collection(storage, "config theirs").await;
    let their_api = api(storage, &theirs, None, "theirs").await;
    let their_variable = storage
        .variables()
        .create(NewVariable {
            collection: theirs.id.clone(),
            category: VariableCategory::Custom,
            name: "theirs".into(),
            value: String::new(),
            enabled: true,
            environment: None,
        })
        .await
        .expect("create a variable");
    let mine = new_collection(storage, "config mine").await;
    let read = storage.tree().config(mine.id.clone()).await.expect("config");

    let mut taking_an_api = read.config.clone();
    taking_an_api.apis.push(ApiSetting {
        name: "mine now".into(),
        ..their_api.item.clone()
    });
    taking_an_api.top_order = vec![their_api.item.id.clone()];
    assert_eq!(
        storage.tree().replace(mine.id.clone(), taking_an_api, None).await,
        Err(StoreError::IdTaken {
            id: their_api.item.id.to_string()
        })
    );
    let mut taking_a_variable = read.config.clone();
    taking_a_variable.variables.push(their_variable.clone());
    assert_eq!(
        storage.tree().replace(mine.id.clone(), taking_a_variable, None).await,
        Err(StoreError::IdTaken {
            id: their_variable.id.to_string()
        })
    );
    assert_eq!(
        storage.tree().config(mine.id.clone()).await.expect("config"),
        read,
        "a refused replace changes nothing"
    );
    assert_eq!(
        load(storage, &theirs).await.apis,
        vec![their_api.item],
        "the other project keeps its API"
    );
}

/// The desktop's copies of server projects (`docs/server-sync.md` §6):
/// kept whole with the server's ids, and bound to an account.
pub async fn copies(storage: &Storage, copies: &dyn crate::ServerCopyStore) {
    use cyberapi_core::AccountId;
    use cyberapi_core::sync::CopyBinding;

    // What a server has: a project made here and gone again, so its ids are
    // free for the copy.
    let server = new_collection(storage, "on the server").await;
    let users = folder(storage, &server, None, "users").await;
    let login = api(storage, &server, Some(&users.item.id), "login").await;
    let logout = api(storage, &server, None, "logout").await;
    storage
        .variables()
        .create(NewVariable {
            collection: server.id.clone(),
            category: VariableCategory::Environment,
            name: "HOST".into(),
            value: "https://a.test".into(),
            enabled: true,
            environment: None,
        })
        .await
        .expect("a variable");
    let mut config = storage
        .tree()
        .config(server.id.clone())
        .await
        .expect("its configuration")
        .config;
    storage.collections().delete(server.id.clone()).await.expect("gone");
    config.name = "the server's".into();
    config.folders[0].settings.headers = vec![KvParam {
        key: "X-Team".into(),
        value: "users".into(),
        enabled: true,
    }];
    config.settings.auth = Auth::Bearer { token: "t".into() };
    config.environments.push(cyberapi_core::model::Environment {
        id: EnvironmentId::new(format!("{}-dev", server.id)),
        collection: server.id.clone(),
        name: "dev".into(),
        created_at: config.variables[0].created_at,
        updated_at: config.variables[0].created_at,
    });
    config.variables[0].environment = Some(config.environments[0].id.clone());
    config.description = "theirs".into();
    let copy = Collection {
        name: config.name.clone(),
        description: config.description.clone(),
        revision: 7,
        ..server.clone()
    };

    // A project here like any other, with the server's id.
    copies.keep_copy(copy.clone(), config.clone()).await.expect("kept");
    let here = storage.collections().get(copy.id.clone()).await.expect("here");
    assert_eq!(
        (here.name.as_str(), here.description.as_str()),
        ("the server's", "theirs")
    );
    let held = storage.tree().config(copy.id.clone()).await.expect("held").config;
    assert_eq!(held, config, "as the server has it");

    // Kept again: what goes, goes with its history; what stays keeps it.
    for id in [&login.item.id, &logout.item.id] {
        storage.history().add(response(&copy, id, 1)).await.expect("a response");
    }
    let mut fewer = config.clone();
    fewer.apis.retain(|api| api.id != logout.item.id);
    fewer.top_order.retain(|id| id != &logout.item.id);
    copies.keep_copy(copy.clone(), fewer.clone()).await.expect("kept again");
    assert_eq!(
        storage.tree().config(copy.id.clone()).await.expect("held").config,
        fewer
    );
    let kept = storage.history().list(copy.id.clone(), login.item.id.clone()).await;
    assert_eq!(kept.expect("history").len(), 1, "an API that stays keeps its history");
    not_found(
        storage.history().list(copy.id.clone(), logout.item.id.clone()).await,
        Entity::Api,
    );

    // Another project's id is not the copy's.
    let other = new_collection(storage, "another").await;
    let theirs = api(storage, &other, None, "theirs").await;
    let mut taken = fewer.clone();
    taken.apis.push(ApiSetting {
        collection: copy.id.clone(),
        ..theirs.item.clone()
    });
    taken.top_order.push(theirs.item.id.clone());
    match copies.keep_copy(copy.clone(), taken).await {
        Err(StoreError::IdTaken { .. }) => {}
        other => panic!("expected the id refused, got {other:?}"),
    }
    assert_eq!(
        storage.tree().config(copy.id.clone()).await.expect("held").config,
        fewer
    );

    // Bound to an account, at a base; in place of the binding it had.
    let bound = |all: Vec<CopyBinding>| -> Vec<CopyBinding> {
        all.into_iter()
            .filter(|binding| binding.collection == copy.id)
            .collect()
    };
    let binding = CopyBinding {
        collection: copy.id.clone(),
        account: AccountId::new("conformance-account"),
        base: Some(7),
        unsynced: false,
    };
    copies.bind(binding.clone()).await.expect("bound");
    assert_eq!(
        bound(copies.copies().await.expect("copies")),
        std::slice::from_ref(&binding)
    );
    let moved = CopyBinding {
        base: Some(8),
        unsynced: true,
        ..binding.clone()
    };
    copies.bind(moved.clone()).await.expect("bound again");
    assert_eq!(bound(copies.copies().await.expect("copies")), [moved]);
    copies.unbind(copy.id.clone()).await.expect("unbound");
    assert!(bound(copies.copies().await.expect("copies")).is_empty());
    copies.unbind(copy.id.clone()).await.expect("no binding is no error");

    // A copy deleted takes its binding.
    copies.bind(binding).await.expect("bound");
    storage.collections().delete(copy.id.clone()).await.expect("deleted");
    assert!(bound(copies.copies().await.expect("copies")).is_empty());
}
