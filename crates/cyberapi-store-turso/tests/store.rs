//! The desktop store: the shared rules, what survives a restart, and what the
//! Tauri app left on disk.

use cyberapi_core::model::{HttpMethod, HttpRequest, VariableCategory};
use cyberapi_core::prefs::{AppPrefs, ThemeMode};
use cyberapi_core::tree::Tree;
use cyberapi_core::{CollectionId, ItemId, Timestamp};
use cyberapi_store::{NewCollection, StoreError, conformance};
use cyberapi_store_turso::TursoStore;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

fn open(path: &Path) -> TursoStore {
    pollster::block_on(TursoStore::open(path)).expect("open the store")
}

/// Raw SQL against a file, as the old app (or a test) would write it.
fn raw(path: &Path, statements: &[&str]) {
    pollster::block_on(async {
        let database = turso::Builder::new_local(path.to_str().expect("utf-8 path"))
            .build()
            .await
            .expect("open");
        let conn = database.connect().expect("connect");
        for sql in statements {
            conn.execute(*sql, ()).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
        }
    });
}

fn raw_text(path: &Path, sql: &str) -> Vec<String> {
    pollster::block_on(async {
        let database = turso::Builder::new_local(path.to_str().expect("utf-8 path"))
            .build()
            .await
            .expect("open");
        let conn = database.connect().expect("connect");
        let mut rows = conn.query(sql, ()).await.expect("query");
        let mut found = Vec::new();
        while let Some(row) = rows.next().await.expect("row") {
            match row.get_value(0).expect("value") {
                turso::Value::Text(text) => found.push(text),
                other => found.push(format!("{other:?}")),
            }
        }
        found
    })
}

#[test]
fn turso_store_conforms() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open(&dir.path().join("cyberapi.db"));
    pollster::block_on(conformance::run(&store.storage(), &store));
}

#[test]
fn turso_store_backs_up() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open(&dir.path().join("cyberapi.db"));
    pollster::block_on(conformance::backup(&store.storage(), &store));
}

#[test]
fn turso_store_keeps_copies() {
    let dir = tempfile::tempdir().expect("a directory");
    let store = open(&dir.path().join("copies.db"));
    pollster::block_on(conformance::copies(&store.storage(), &store));
}

#[test]
fn content_changes_move_the_collection_forward() {
    let dir = tempfile::tempdir().expect("temp dir");
    let seconds = Arc::new(AtomicI64::new(1_700_000_000));
    let store = pollster::block_on(TursoStore::open_with_clock(&dir.path().join("t.db"), move || {
        let at = seconds.fetch_add(1, Ordering::SeqCst);
        Timestamp::new(
            chrono::DateTime::from_timestamp(at, 0)
                .unwrap_or_default()
                .fixed_offset(),
        )
    }))
    .expect("open");
    let storage = store.storage();
    pollster::block_on(async {
        let collection = storage
            .collections()
            .create(NewCollection {
                name: "p".into(),
                ..Default::default()
            })
            .await
            .expect("create");
        let api = storage
            .tree()
            .create_api(collection.id.clone(), None, "a".into())
            .await
            .expect("create an API");
        let after_create = storage.collections().get(collection.id.clone()).await.expect("get");
        assert!(after_create.updated_at > collection.updated_at);
        storage
            .tree()
            .save_request(collection.id.clone(), api.item.id, HttpRequest::default())
            .await
            .expect("save");
        let after_save = storage.collections().get(collection.id).await.expect("get");
        assert!(after_save.updated_at > after_create.updated_at);
        assert_eq!(after_save.created_at, collection.created_at);
    });
}

#[test]
fn data_survives_a_restart() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("cyberapi.db");
    let (collection, api) = {
        let storage = open(&path).storage();
        pollster::block_on(async {
            let collection = storage
                .collections()
                .create(NewCollection {
                    name: "kept".into(),
                    ..Default::default()
                })
                .await
                .expect("create");
            let api = storage
                .tree()
                .create_api(collection.id.clone(), None, "login".into())
                .await
                .expect("create an API");
            storage
                .prefs()
                .set_app(AppPrefs {
                    theme: ThemeMode::Dark,
                    ..Default::default()
                })
                .await
                .expect("prefs");
            // As stored, after the API moved it on.
            let collection = storage.collections().get(collection.id).await.expect("get");
            (collection, api.item)
        })
    };

    // Opening again runs the migrations again, which must change nothing.
    let storage = open(&path).storage();
    pollster::block_on(async {
        assert_eq!(
            storage.collections().get(collection.id.clone()).await.expect("get"),
            collection
        );
        assert_eq!(storage.tree().load(collection.id).await.expect("load").apis, vec![api]);
        assert_eq!(storage.prefs().app().await.expect("prefs").theme, ThemeMode::Dark);
    });
    assert_eq!(
        raw_text(&path, "SELECT value FROM cyberapi_meta WHERE key = 'schema_version'"),
        [cyberapi_store_turso::SCHEMA_VERSION.to_string()]
    );
}

/// The Tauri app's tables and rows, as it wrote them: everything text, empty
/// times, `children` with blanks and repeats, settings with only the fields
/// that were touched, `enabled` as `"1"` / `"0"`.
const LEGACY: &[&str] = &[
    "CREATE TABLE api_collections (id TEXT PRIMARY KEY NOT NULL check (id != ''), name TEXT DEFAULT '',
        description TEXT DEFAULT '', created_at TEXT DEFAULT '', updated_at TEXT DEFAULT '')",
    "CREATE TABLE api_folders (id TEXT PRIMARY KEY NOT NULL check (id != ''), collection TEXT NOT NULL check (collection != ''),
        children TEXT DEFAULT '', name TEXT DEFAULT '', created_at TEXT DEFAULT '', updated_at TEXT DEFAULT '')",
    "CREATE TABLE api_settings (id TEXT PRIMARY KEY NOT NULL check (id != ''), collection TEXT NOT NULL check (collection != ''),
        name TEXT DEFAULT '', category TEXT DEFAULT '', setting TEXT DEFAULT '', created_at TEXT DEFAULT '', updated_at TEXT DEFAULT '')",
    "CREATE TABLE variables (id TEXT PRIMARY KEY NOT NULL check (id != ''), category TEXT NOT NULL check (category != ''),
        collection TEXT NOT NULL check (collection != ''), name TEXT DEFAULT '', value TEXT DEFAULT '', enabled TEXT DEFAULT '',
        created_at TEXT DEFAULT '', updated_at TEXT DEFAULT '')",
    "CREATE TABLE versions (id TEXT PRIMARY KEY NOT NULL check (id != ''), version TEXT NOT NULL check (version != ''),
        created_at TEXT DEFAULT '', updated_at TEXT DEFAULT '')",
    "INSERT INTO versions VALUES ('v', '0.1.21', '2022-09-01T00:00:00+00:00', '2022-09-01T00:00:00+00:00')",
    "INSERT INTO api_collections VALUES ('c1', 'Orders', 'the shop', '2022-09-05T10:03:34+08:00', '2022-09-08T15:49:46+08:00')",
    "INSERT INTO api_folders VALUES ('f1', 'c1', 'a1,,a2,a1', '用户', '', '')",
    "INSERT INTO api_settings VALUES ('a1', 'c1', '登录', 'http',
        '{\"uri\":\"{{HOST}}/login\",\"method\":\"POST\",\"contentType\":\"application/json\",\"body\":\"{}\"}', '', '')",
    "INSERT INTO api_settings VALUES ('a2', 'c1', 'me', 'http', '', '', '')",
    "INSERT INTO api_settings VALUES ('a3', 'c1', 'health', 'http', '{\"uri\":\"/health\"}', '', '')",
    "INSERT INTO variables VALUES ('v1', 'env', 'c1', 'HOST', 'http://localhost', '1', '', '')",
    "INSERT INTO variables VALUES ('v2', 'env', 'c1', 'BACKUP', 'http://backup', '0', '', '')",
    "INSERT INTO variables VALUES ('v3', 'graphql', 'c1', 'odd', 'x', '1', '', '')",
];

#[test]
fn reads_what_the_tauri_app_wrote() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("my_db.db");
    raw(&path, LEGACY);

    let storage = open(&path).storage();
    let c1 = CollectionId::from("c1");
    pollster::block_on(async {
        let collections = storage.collections().list().await.expect("list");
        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0].name, "Orders");
        assert_eq!(collections[0].created_at.to_rfc3339(), "2022-09-05T10:03:34+08:00");

        let data = storage.tree().load(c1.clone()).await.expect("load");
        assert_eq!(data.folders[0].children, [ItemId::from("a1"), ItemId::from("a2")]);
        assert_eq!(
            data.folders[0].created_at,
            Timestamp::default(),
            "an empty time reads as the epoch"
        );
        assert_eq!(data.apis[0].request.method, HttpMethod::Post);
        assert_eq!(data.apis[0].request.uri, "{{HOST}}/login");
        assert_eq!(
            data.apis[1].request,
            HttpRequest::default(),
            "an empty setting is an empty request"
        );
        // With no stored top-level order, the old app's: APIs, then folders.
        let tree = Tree::build(&data);
        assert_eq!(tree.top_level_ids(), [ItemId::from("a3"), ItemId::from("f1")]);

        let env = storage
            .variables()
            .list(c1.clone(), VariableCategory::Environment)
            .await
            .expect("variables");
        let summary: Vec<(&str, bool)> = env.iter().map(|v| (v.name.as_str(), v.enabled)).collect();
        assert_eq!(summary, [("BACKUP", false), ("HOST", true)]);

        // Written back in the old columns' own forms.
        storage
            .tree()
            .move_item(
                c1.clone(),
                ItemId::from("a3"),
                cyberapi_core::tree::Placement {
                    parent: Some(ItemId::from("f1")),
                    before: None,
                },
            )
            .await
            .expect("move");
        storage
            .variables()
            .update(
                c1.clone(),
                env[1].id.clone(),
                cyberapi_store::VariableEdit {
                    name: "HOST".into(),
                    value: "http://localhost".into(),
                    enabled: false,
                },
            )
            .await
            .expect("update");
    });
    assert_eq!(
        raw_text(&path, "SELECT children FROM api_folders WHERE id = 'f1'"),
        ["a1,a2,a3"]
    );
    assert_eq!(raw_text(&path, "SELECT enabled FROM variables WHERE id = 'v1'"), ["0"]);
    assert_eq!(
        raw_text(&path, "SELECT version FROM versions"),
        ["0.1.21"],
        "the old app's version table is left as it is"
    );
}

#[test]
fn a_missing_data_directory_is_created() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("fresh").join("cyberapi.db");
    drop(open(&path));
    assert!(path.is_file(), "the database exists");
}

#[test]
fn a_newer_schema_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("cyberapi.db");
    drop(open(&path));
    raw(
        &path,
        &["UPDATE cyberapi_meta SET value = '999' WHERE key = 'schema_version'"],
    );
    match pollster::block_on(TursoStore::open(&path)) {
        Err(StoreError::Backend { message }) => assert!(message.contains("newer"), "{message}"),
        Err(other) => panic!("expected a backend error, got {other:?}"),
        Ok(_) => panic!("a file from a newer build was opened"),
    }
}

/// Run by hand against a copy of a real `my_db.db`:
/// `CYBERAPI_LEGACY_DB=/path/to/copy.db cargo test -p cyberapi-store-turso -- --ignored --nocapture`
/// The file is opened, so it is migrated: never point this at the original.
#[test]
#[ignore = "needs a copy of a real Tauri database"]
fn opens_a_real_tauri_database() {
    let path = std::env::var("CYBERAPI_LEGACY_DB").expect("CYBERAPI_LEGACY_DB");
    let storage = open(Path::new(&path)).storage();
    pollster::block_on(async {
        let collections = storage.collections().list().await.expect("list");
        let (mut folders, mut apis, mut variables) = (0, 0, 0);
        for collection in &collections {
            let data = storage.tree().load(collection.id.clone()).await.expect("load");
            let tree = Tree::build(&data);
            let shown = tree.top_level_ids().len()
                + tree
                    .roots()
                    .map(|root| tree.descendants(root.id()).len())
                    .sum::<usize>();
            assert_eq!(
                shown,
                data.folders.len() + data.apis.len(),
                "every item of {} is on its tree",
                collection.name
            );
            folders += data.folders.len();
            apis += data.apis.len();
            for category in [
                VariableCategory::Environment,
                VariableCategory::Custom,
                VariableCategory::GlobalRequestHeader,
            ] {
                variables += storage
                    .variables()
                    .list(collection.id.clone(), category)
                    .await
                    .expect("variables")
                    .len();
            }
        }
        println!(
            "{} collections, {folders} folders, {apis} APIs, {variables} variables",
            collections.len()
        );
    });
}
