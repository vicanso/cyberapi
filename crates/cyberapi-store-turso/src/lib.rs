//! The desktop's store (`docs/architecture.md` §4.3): every Store trait over
//! one turso file in the SQLite format, the Tauri app's tables kept as they
//! were so its `my_db.db` opens unchanged.
//!
//! One connection serves every call, one at a time: a desktop has one reader,
//! and a change that touches several rows runs in a transaction that either
//! lands whole or not at all.

mod legacy;
mod rows;
mod schema;

pub use legacy::{LegacyCopy, TAURI_IDENTIFIER, copy_legacy_data, tauri_data_dir};
pub use schema::CURRENT_VERSION as SCHEMA_VERSION;

use async_lock::Mutex;
use chrono::Local;
use cyberapi_core::model::{
    ApiCategory, ApiSetting, Collection, Folder, HttpRequest, Variable, VariableCategory, legacy as columns,
};
use cyberapi_core::prefs::{AppPrefs, WorkspacePrefs};
use cyberapi_core::tree::{Placement, Removal, TreeChange, TreeData};
use cyberapi_core::{CollectionId, ItemId, Timestamp, VariableId};
use cyberapi_store::rules::required_name;
use cyberapi_store::{
    CollectionEdit, CollectionStore, Entity, Inserted, NewCollection, NewVariable, PrefStore, Storage, StoreError,
    StoreFuture, TreeStore, VariableEdit, VariableStore,
};
use rows::{API_COLUMNS, COLLECTION_COLUMNS, FOLDER_COLUMNS, VARIABLE_COLUMNS};
use serde::de::DeserializeOwned;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use turso::{Connection, IntoParams, Row};
use ulid::Ulid;

type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

#[derive(Clone)]
pub struct TursoStore {
    inner: Arc<Inner>,
}

struct Inner {
    // Held for the connection's sake: it is the file the connection is to.
    _database: turso::Database,
    conn: Mutex<Connection>,
    clock: Clock,
}

pub(crate) fn backend(error: impl std::fmt::Display) -> StoreError {
    StoreError::backend(error)
}

impl TursoStore {
    /// Opens (or creates) the file and brings it to the current schema. Times
    /// are written in local time with their offset, as the old app wrote them.
    pub async fn open(path: &Path) -> Result<Self, StoreError> {
        Self::open_with_clock(path, || Timestamp::new(Local::now().fixed_offset())).await
    }

    /// [`TursoStore::open`] with "now" read from `clock`, for tests that
    /// compare times.
    pub async fn open_with_clock(
        path: &Path,
        clock: impl Fn() -> Timestamp + Send + Sync + 'static,
    ) -> Result<Self, StoreError> {
        let path = path
            .to_str()
            .ok_or_else(|| backend(format!("the database path is not UTF-8: {}", path.display())))?;
        let database = turso::Builder::new_local(path).build().await.map_err(backend)?;
        let mut conn = database.connect().map_err(backend)?;
        schema::migrate(&mut conn).await?;
        Ok(Self {
            inner: Arc::new(Inner {
                _database: database,
                conn: Mutex::new(conn),
                clock: Arc::new(clock),
            }),
        })
    }

    /// This store as every trait the layers above use.
    pub fn storage(&self) -> Storage {
        Storage::from_backend(Arc::new(self.clone()))
    }

    /// Runs `work` with the connection to itself and "now", as a store future.
    fn run<T, F>(&self, work: impl FnOnce(Arc<Inner>, Timestamp) -> F + Send + 'static) -> StoreFuture<T>
    where
        F: Future<Output = Result<T, StoreError>> + Send + 'static,
        T: Send + 'static,
    {
        let inner = self.inner.clone();
        Box::pin(async move {
            // Through text and back: what is returned is exactly what a read
            // of the row will give, to the second.
            let now = Timestamp::parse_lenient(&(inner.clock)().to_rfc3339());
            work(inner, now).await
        })
    }
}

async fn query_all<T>(
    conn: &Connection,
    sql: &str,
    params: impl IntoParams,
    map: impl Fn(&Row) -> T,
) -> Result<Vec<T>, StoreError> {
    let mut rows = conn.query(sql, params).await.map_err(backend)?;
    let mut found = Vec::new();
    while let Some(row) = rows.next().await.map_err(backend)? {
        found.push(map(&row));
    }
    Ok(found)
}

async fn query_one<T>(
    conn: &Connection,
    sql: &str,
    params: impl IntoParams,
    map: impl Fn(&Row) -> T,
) -> Result<Option<T>, StoreError> {
    let mut rows = conn.query(sql, params).await.map_err(backend)?;
    Ok(rows.next().await.map_err(backend)?.map(|row| map(&row)))
}

async fn execute(conn: &Connection, sql: &str, params: impl IntoParams) -> Result<u64, StoreError> {
    conn.execute(sql, params).await.map_err(backend)
}

async fn get_collection(conn: &Connection, id: &CollectionId) -> Result<Collection, StoreError> {
    query_one(
        conn,
        &format!("SELECT {COLLECTION_COLUMNS} FROM api_collections WHERE id = ?1"),
        (id.as_str(),),
        rows::collection,
    )
    .await?
    .ok_or_else(|| StoreError::not_found(Entity::Collection, id))
}

async fn require_collection(conn: &Connection, id: &CollectionId) -> Result<(), StoreError> {
    get_collection(conn, id).await.map(|_| ())
}

/// A change to the project's content moves its `updated_at` (#21).
async fn touch(conn: &Connection, id: &CollectionId, now: Timestamp) -> Result<(), StoreError> {
    execute(
        conn,
        "UPDATE api_collections SET updated_at = ?1 WHERE id = ?2",
        (now.to_rfc3339(), id.as_str()),
    )
    .await
    .map(|_| ())
}

/// The project's tree in store order: the order rows were inserted, which is
/// the order the old app read them in.
async fn tree_data(conn: &Connection, id: &CollectionId) -> Result<TreeData, StoreError> {
    let folders = query_all(
        conn,
        &format!("SELECT {FOLDER_COLUMNS} FROM api_folders WHERE collection = ?1 ORDER BY rowid"),
        (id.as_str(),),
        rows::folder,
    )
    .await?;
    let apis = query_all(
        conn,
        &format!("SELECT {API_COLUMNS} FROM api_settings WHERE collection = ?1 ORDER BY rowid"),
        (id.as_str(),),
        rows::api,
    )
    .await?;
    let top_order = query_one(
        conn,
        "SELECT items FROM top_orders WHERE collection = ?1",
        (id.as_str(),),
        |row| columns::parse_id_list(&rows::text(row, 0)),
    )
    .await?
    .unwrap_or_default();
    Ok(TreeData {
        folders,
        apis,
        top_order,
    })
}

/// Writes the lists a plan changed.
async fn write_change(conn: &Connection, collection: &CollectionId, change: &TreeChange) -> Result<(), StoreError> {
    for (id, children) in &change.folders {
        execute(
            conn,
            "UPDATE api_folders SET children = ?1 WHERE id = ?2 AND collection = ?3",
            (columns::join_id_list(children), id.as_str(), collection.as_str()),
        )
        .await?;
    }
    if let Some(top_order) = &change.top_order {
        execute(
            conn,
            "INSERT INTO top_orders (collection, items) VALUES (?1, ?2)
             ON CONFLICT (collection) DO UPDATE SET items = excluded.items",
            (collection.as_str(), columns::join_id_list(top_order)),
        )
        .await?;
    }
    Ok(())
}

fn workspace_scope(collection: &CollectionId) -> String {
    format!("workspace/{collection}")
}

/// A stored document, or its defaults when there is none or it cannot be
/// read: a reader's preferences are not worth failing a launch over.
async fn read_doc<T: DeserializeOwned + Default>(conn: &Connection, scope: &str) -> Result<T, StoreError> {
    let doc = query_one(conn, "SELECT doc FROM prefs WHERE scope = ?1", (scope,), |row| {
        rows::text(row, 0)
    })
    .await?;
    Ok(doc.and_then(|doc| serde_json::from_str(&doc).ok()).unwrap_or_default())
}

async fn write_doc(conn: &Connection, scope: &str, doc: &impl serde::Serialize) -> Result<(), StoreError> {
    let doc = serde_json::to_string(doc).map_err(backend)?;
    execute(
        conn,
        "INSERT INTO prefs (scope, doc) VALUES (?1, ?2) ON CONFLICT (scope) DO UPDATE SET doc = excluded.doc",
        (scope, doc),
    )
    .await
    .map(|_| ())
}

fn new_id() -> String {
    Ulid::generate().to_string()
}

impl CollectionStore for TursoStore {
    fn list(&self) -> StoreFuture<Vec<Collection>> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            query_all(
                &conn,
                &format!("SELECT {COLLECTION_COLUMNS} FROM api_collections ORDER BY rowid"),
                (),
                rows::collection,
            )
            .await
        })
    }

    fn get(&self, id: CollectionId) -> StoreFuture<Collection> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            get_collection(&conn, &id).await
        })
    }

    fn create(&self, input: NewCollection) -> StoreFuture<Collection> {
        self.run(move |inner, now| async move {
            let collection = Collection {
                id: CollectionId::new(new_id()),
                name: required_name(&input.name)?,
                description: input.description,
                created_at: now,
                updated_at: now,
                owner: None,
                role: Default::default(),
            };
            let conn = inner.conn.lock().await;
            execute(
                &conn,
                "INSERT INTO api_collections (id, name, description, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (
                    collection.id.as_str(),
                    collection.name.as_str(),
                    collection.description.as_str(),
                    now.to_rfc3339(),
                    now.to_rfc3339(),
                ),
            )
            .await?;
            Ok(collection)
        })
    }

    fn update(&self, id: CollectionId, edit: CollectionEdit) -> StoreFuture<Collection> {
        self.run(move |inner, now| async move {
            let name = required_name(&edit.name)?;
            let conn = inner.conn.lock().await;
            let mut collection = get_collection(&conn, &id).await?;
            execute(
                &conn,
                "UPDATE api_collections SET name = ?1, description = ?2, updated_at = ?3 WHERE id = ?4",
                (name.as_str(), edit.description.as_str(), now.to_rfc3339(), id.as_str()),
            )
            .await?;
            collection.name = name;
            collection.description = edit.description;
            collection.updated_at = now;
            Ok(collection)
        })
    }

    fn delete(&self, id: CollectionId) -> StoreFuture<()> {
        self.run(move |inner, _| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &id).await?;
            for sql in [
                "DELETE FROM api_folders WHERE collection = ?1",
                "DELETE FROM api_settings WHERE collection = ?1",
                "DELETE FROM variables WHERE collection = ?1",
                "DELETE FROM top_orders WHERE collection = ?1",
                "DELETE FROM api_collections WHERE id = ?1",
            ] {
                execute(&tx, sql, (id.as_str(),)).await?;
            }
            execute(&tx, "DELETE FROM prefs WHERE scope = ?1", (workspace_scope(&id),)).await?;
            tx.commit().await.map_err(backend)
        })
    }
}

impl TreeStore for TursoStore {
    fn load(&self, collection: CollectionId) -> StoreFuture<TreeData> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            require_collection(&conn, &collection).await?;
            tree_data(&conn, &collection).await
        })
    }

    fn create_folder(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<Folder>> {
        self.run(move |inner, now| async move {
            let name = required_name(&name)?;
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            let id = ItemId::new(new_id());
            let change = tree_data(&tx, &collection).await?.plan_insert(&id, parent.as_ref())?;
            let folder = Folder {
                id,
                collection: collection.clone(),
                name,
                children: Vec::new(),
                created_at: now,
                updated_at: now,
            };
            execute(
                &tx,
                "INSERT INTO api_folders (id, collection, children, name, created_at, updated_at)
                 VALUES (?1, ?2, '', ?3, ?4, ?5)",
                (
                    folder.id.as_str(),
                    collection.as_str(),
                    folder.name.as_str(),
                    now.to_rfc3339(),
                    now.to_rfc3339(),
                ),
            )
            .await?;
            write_change(&tx, &collection, &change).await?;
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(Inserted { item: folder, change })
        })
    }

    fn create_api(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        name: String,
    ) -> StoreFuture<Inserted<ApiSetting>> {
        self.run(move |inner, now| async move {
            let name = required_name(&name)?;
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            let id = ItemId::new(new_id());
            let change = tree_data(&tx, &collection).await?.plan_insert(&id, parent.as_ref())?;
            let api = ApiSetting {
                id,
                collection: collection.clone(),
                name,
                category: ApiCategory::Http,
                request: HttpRequest::default(),
                created_at: now,
                updated_at: now,
            };
            execute(
                &tx,
                "INSERT INTO api_settings (id, collection, name, category, setting, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'http', ?4, ?5, ?6)",
                (
                    api.id.as_str(),
                    collection.as_str(),
                    api.name.as_str(),
                    api.request.to_json(),
                    now.to_rfc3339(),
                    now.to_rfc3339(),
                ),
            )
            .await?;
            write_change(&tx, &collection, &change).await?;
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(Inserted { item: api, change })
        })
    }

    fn rename(&self, collection: CollectionId, item: ItemId, name: String) -> StoreFuture<()> {
        self.run(move |inner, now| async move {
            let name = required_name(&name)?;
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            let params = (name.as_str(), now.to_rfc3339(), item.as_str(), collection.as_str());
            let renamed = execute(
                &tx,
                "UPDATE api_folders SET name = ?1, updated_at = ?2 WHERE id = ?3 AND collection = ?4",
                params.clone(),
            )
            .await?
                + execute(
                    &tx,
                    "UPDATE api_settings SET name = ?1, updated_at = ?2 WHERE id = ?3 AND collection = ?4",
                    params,
                )
                .await?;
            if renamed == 0 {
                return Err(StoreError::not_found(Entity::Item, &item));
            }
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)
        })
    }

    fn save_request(&self, collection: CollectionId, api: ItemId, request: HttpRequest) -> StoreFuture<ApiSetting> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            let saved = execute(
                &tx,
                "UPDATE api_settings SET setting = ?1, updated_at = ?2 WHERE id = ?3 AND collection = ?4",
                (request.to_json(), now.to_rfc3339(), api.as_str(), collection.as_str()),
            )
            .await?;
            if saved == 0 {
                return Err(StoreError::not_found(Entity::Api, &api));
            }
            let found = query_one(
                &tx,
                &format!("SELECT {API_COLUMNS} FROM api_settings WHERE id = ?1"),
                (api.as_str(),),
                rows::api,
            )
            .await?
            .ok_or_else(|| StoreError::not_found(Entity::Api, &api))?;
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(found)
        })
    }

    fn move_item(&self, collection: CollectionId, item: ItemId, to: Placement) -> StoreFuture<TreeChange> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            let change = tree_data(&tx, &collection).await?.plan_move(&item, &to)?;
            write_change(&tx, &collection, &change).await?;
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(change)
        })
    }

    fn delete(&self, collection: CollectionId, item: ItemId) -> StoreFuture<Removal> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            let removal = tree_data(&tx, &collection).await?.plan_remove(&item)?;
            for id in &removal.folders {
                execute(
                    &tx,
                    "DELETE FROM api_folders WHERE id = ?1 AND collection = ?2",
                    (id.as_str(), collection.as_str()),
                )
                .await?;
            }
            for id in &removal.apis {
                execute(
                    &tx,
                    "DELETE FROM api_settings WHERE id = ?1 AND collection = ?2",
                    (id.as_str(), collection.as_str()),
                )
                .await?;
            }
            write_change(&tx, &collection, &removal.change).await?;
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(removal)
        })
    }
}

impl VariableStore for TursoStore {
    fn list(&self, collection: CollectionId, category: VariableCategory) -> StoreFuture<Vec<Variable>> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            require_collection(&conn, &collection).await?;
            let found = query_all(
                &conn,
                &format!(
                    "SELECT {VARIABLE_COLUMNS} FROM variables
                     WHERE collection = ?1 AND category = ?2 ORDER BY name, rowid"
                ),
                (collection.as_str(), category.as_str()),
                rows::variable,
            )
            .await?;
            Ok(found.into_iter().flatten().collect())
        })
    }

    fn create(&self, input: NewVariable) -> StoreFuture<Variable> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &input.collection).await?;
            let variable = Variable {
                id: VariableId::new(new_id()),
                collection: input.collection,
                category: input.category,
                name: input.name,
                value: input.value,
                enabled: input.enabled,
                created_at: now,
                updated_at: now,
            };
            execute(
                &tx,
                "INSERT INTO variables (id, category, collection, name, value, enabled, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                (
                    variable.id.as_str(),
                    variable.category.as_str(),
                    variable.collection.as_str(),
                    variable.name.as_str(),
                    variable.value.as_str(),
                    columns::enabled_text(variable.enabled),
                    now.to_rfc3339(),
                    now.to_rfc3339(),
                ),
            )
            .await?;
            touch(&tx, &variable.collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(variable)
        })
    }

    fn update(&self, collection: CollectionId, id: VariableId, edit: VariableEdit) -> StoreFuture<Variable> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            let updated = execute(
                &tx,
                "UPDATE variables SET name = ?1, value = ?2, enabled = ?3, updated_at = ?4
                 WHERE id = ?5 AND collection = ?6",
                (
                    edit.name.as_str(),
                    edit.value.as_str(),
                    columns::enabled_text(edit.enabled),
                    now.to_rfc3339(),
                    id.as_str(),
                    collection.as_str(),
                ),
            )
            .await?;
            if updated == 0 {
                return Err(StoreError::not_found(Entity::Variable, &id));
            }
            let variable = query_one(
                &tx,
                &format!("SELECT {VARIABLE_COLUMNS} FROM variables WHERE id = ?1"),
                (id.as_str(),),
                rows::variable,
            )
            .await?
            .flatten()
            .ok_or_else(|| StoreError::not_found(Entity::Variable, &id))?;
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(variable)
        })
    }

    fn delete(&self, collection: CollectionId, ids: Vec<VariableId>) -> StoreFuture<()> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            for id in &ids {
                execute(
                    &tx,
                    "DELETE FROM variables WHERE id = ?1 AND collection = ?2",
                    (id.as_str(), collection.as_str()),
                )
                .await?;
            }
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)
        })
    }
}

impl PrefStore for TursoStore {
    fn app(&self) -> StoreFuture<AppPrefs> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            read_doc(&conn, "app").await
        })
    }

    fn set_app(&self, prefs: AppPrefs) -> StoreFuture<()> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            write_doc(&conn, "app", &prefs).await
        })
    }

    fn workspace(&self, collection: CollectionId) -> StoreFuture<WorkspacePrefs> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            require_collection(&conn, &collection).await?;
            read_doc(&conn, &workspace_scope(&collection)).await
        })
    }

    fn set_workspace(&self, collection: CollectionId, prefs: WorkspacePrefs) -> StoreFuture<()> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            require_collection(&conn, &collection).await?;
            write_doc(&conn, &workspace_scope(&collection), &prefs).await
        })
    }
}

#[cfg(feature = "conformance")]
impl cyberapi_store::conformance::Probe for TursoStore {
    fn rows_of(&self, collection: CollectionId) -> StoreFuture<usize> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            let id = collection.as_str();
            let mut total = 0;
            for (sql, key) in [
                ("SELECT count(*) FROM api_collections WHERE id = ?1", id.to_string()),
                ("SELECT count(*) FROM api_folders WHERE collection = ?1", id.to_string()),
                (
                    "SELECT count(*) FROM api_settings WHERE collection = ?1",
                    id.to_string(),
                ),
                ("SELECT count(*) FROM variables WHERE collection = ?1", id.to_string()),
                ("SELECT count(*) FROM top_orders WHERE collection = ?1", id.to_string()),
                (
                    "SELECT count(*) FROM prefs WHERE scope = ?1",
                    workspace_scope(&collection),
                ),
            ] {
                let count = query_one(&conn, sql, (key,), |row| rows::text(row, 0)).await?;
                total += count.and_then(|count| count.parse::<usize>().ok()).unwrap_or(0);
            }
            Ok(total)
        })
    }
}
