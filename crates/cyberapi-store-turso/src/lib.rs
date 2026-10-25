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
use cyberapi_core::backup::Backup;
use cyberapi_core::cookie::Cookie;
use cyberapi_core::environments::reordered;
use cyberapi_core::import::{ImportItem, assign_ids};
use cyberapi_core::model::{
    ApiCategory, ApiSetting, Collection, Environment, Folder, FolderSettings, HttpRequest, ProjectSettings, Variable,
    VariableCategory, legacy as columns,
};
use cyberapi_core::prefs::{AppPrefs, WorkspacePrefs};
use cyberapi_core::response::{HISTORY_LIMIT, HttpResponse};
use cyberapi_core::sync::{ConfigAt, CopyBinding, ProjectConfig};
use cyberapi_core::tree::{Placement, Removal, TreeChange, TreeData};
use cyberapi_core::{CollectionId, EnvironmentId, ItemId, Timestamp, VariableId};
use cyberapi_store::rules::{environment_name, known_environment, project_config, required_name};
use cyberapi_store::{
    BackupStore, CollectionEdit, CollectionStore, CookieStore, Entity, HistoryStore, Imported, Inserted, NewCollection,
    NewVariable, PrefStore, ServerCopyStore, Storage, StoreError, StoreFuture, TreeStore, VariableEdit, VariableStore,
    sort_cookies,
};
use rows::{API_COLUMNS, COLLECTION_COLUMNS, ENVIRONMENT_COLUMNS, FOLDER_COLUMNS, VARIABLE_COLUMNS};
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, HashMap};
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

/// How long a launch waits when the previous process still holds the file.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
const LOCK_PAUSE: std::time::Duration = std::time::Duration::from_millis(50);

/// The previous process has not let go of the file yet (#46).
fn file_is_locked(error: &StoreError) -> bool {
    matches!(error, StoreError::Backend { message } if message.to_ascii_lowercase().contains("locked"))
}

impl TursoStore {
    /// Opens (or creates) the file and brings it to the current schema. Times
    /// are written in local time with their offset, as the old app wrote them.
    /// The parent directory is created (#41). A file another process still
    /// holds is tried again for a short while (#46).
    pub async fn open(path: &Path) -> Result<Self, StoreError> {
        Self::open_with_clock(path, || Timestamp::new(Local::now().fixed_offset())).await
    }

    /// [`TursoStore::open`] with "now" read from `clock`, for tests that
    /// compare times.
    pub async fn open_with_clock(
        path: &Path,
        clock: impl Fn() -> Timestamp + Send + Sync + 'static,
    ) -> Result<Self, StoreError> {
        let clock: Clock = Arc::new(clock);
        let started = std::time::Instant::now();
        loop {
            match Self::open_file(path, clock.clone()).await {
                Ok(store) => return Ok(store),
                Err(error) if file_is_locked(&error) && started.elapsed() < LOCK_WAIT => {
                    std::thread::sleep(LOCK_PAUSE);
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// This store as every trait the layers above use.
    pub fn storage(&self) -> Storage {
        Storage::from_backend(Arc::new(self.clone()))
    }

    /// Opens `path`, creating its directory. One try: the lock is retried by
    /// [`TursoStore::open_with_clock`].
    async fn open_file(path: &Path, clock: Clock) -> Result<TursoStore, StoreError> {
        if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .map_err(|error| backend(format!("could not create {}: {error}", parent.display())))?;
        }
        let path = path
            .to_str()
            .ok_or_else(|| backend(format!("the database path is not UTF-8: {}", path.display())))?;
        let database = turso::Builder::new_local(path).build().await.map_err(backend)?;
        let mut conn = database.connect().map_err(backend)?;
        schema::migrate(&mut conn).await?;
        Ok(TursoStore {
            inner: Arc::new(Inner {
                _database: database,
                conn: Mutex::new(conn),
                clock,
            }),
        })
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

/// A change to the project's content moves its `updated_at` (#21) and its
/// revision.
async fn touch(conn: &Connection, id: &CollectionId, now: Timestamp) -> Result<(), StoreError> {
    execute(
        conn,
        "UPDATE api_collections SET updated_at = ?1, revision = revision + 1 WHERE id = ?2",
        (now.to_rfc3339(), id.as_str()),
    )
    .await
    .map(|_| ())
}

/// The project's tree in store order: the order rows were inserted, which is
/// the order the old app read them in.
async fn tree_data(conn: &Connection, id: &CollectionId) -> Result<TreeData, StoreError> {
    let mut folders = query_all(
        conn,
        &format!("SELECT {FOLDER_COLUMNS} FROM api_folders WHERE collection = ?1 ORDER BY rowid"),
        (id.as_str(),),
        rows::folder,
    )
    .await?;
    let mut settings = folder_settings(conn, Some(id)).await?;
    for folder in &mut folders {
        if let Some(own) = settings.remove(folder.id.as_str()) {
            folder.settings = own;
        }
    }
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
    let settings = query_one(
        conn,
        "SELECT settings FROM project_settings WHERE collection = ?1",
        (id.as_str(),),
        |row| settings_of(&rows::text(row, 0)),
    )
    .await?
    .unwrap_or_default();
    Ok(TreeData {
        folders,
        apis,
        top_order,
        settings,
    })
}

/// A `settings` column (#66); one that cannot be read is none, as an
/// unreadable request is an empty one.
fn settings_of<T: DeserializeOwned + Default>(text: &str) -> T {
    serde_json::from_str(text).unwrap_or_default()
}

/// The folders' settings by folder id: one project's, or every one's.
async fn folder_settings(
    conn: &Connection,
    collection: Option<&CollectionId>,
) -> Result<HashMap<String, FolderSettings>, StoreError> {
    let read = |row: &Row| (rows::text(row, 0), settings_of(&rows::text(row, 1)));
    let found = match collection {
        Some(id) => {
            query_all(
                conn,
                "SELECT folder, settings FROM folder_settings WHERE collection = ?1",
                (id.as_str(),),
                read,
            )
            .await?
        }
        None => query_all(conn, "SELECT folder, settings FROM folder_settings", (), read).await?,
    };
    Ok(found.into_iter().collect())
}

/// What a folder sets (#66): a row while it sets something, none after.
async fn write_folder_settings(
    conn: &Connection,
    collection: &CollectionId,
    folder: &ItemId,
    settings: &FolderSettings,
) -> Result<(), StoreError> {
    if settings.is_empty() {
        execute(
            conn,
            "DELETE FROM folder_settings WHERE folder = ?1",
            (folder.as_str(),),
        )
        .await?;
    } else {
        let json = serde_json::to_string(settings).map_err(backend)?;
        execute(
            conn,
            "INSERT INTO folder_settings (folder, collection, settings) VALUES (?1, ?2, ?3)
             ON CONFLICT (folder) DO UPDATE SET collection = excluded.collection, settings = excluded.settings",
            (folder.as_str(), collection.as_str(), json),
        )
        .await?;
    }
    Ok(())
}

/// Which named environment each variable is in (#61), by variable id: one
/// project's, or every one's.
async fn variable_environments(
    conn: &Connection,
    collection: Option<&CollectionId>,
) -> Result<HashMap<String, EnvironmentId>, StoreError> {
    let read = |row: &Row| (rows::text(row, 0), EnvironmentId::new(rows::text(row, 1)));
    let found = match collection {
        Some(id) => {
            query_all(
                conn,
                "SELECT variable, environment FROM variable_environments WHERE collection = ?1",
                (id.as_str(),),
                read,
            )
            .await?
        }
        None => {
            query_all(
                conn,
                "SELECT variable, environment FROM variable_environments",
                (),
                read,
            )
            .await?
        }
    };
    Ok(found.into_iter().collect())
}

/// Variables as read from their table, each with its environment.
fn in_environments(variables: Vec<Option<Variable>>, environments: &HashMap<String, EnvironmentId>) -> Vec<Variable> {
    variables
        .into_iter()
        .flatten()
        .map(|variable| Variable {
            environment: environments.get(variable.id.as_str()).cloned(),
            ..variable
        })
        .collect()
}

/// Which environment a variable is in (#61): a row while it is in one.
async fn write_variable_environment(
    conn: &Connection,
    collection: &CollectionId,
    variable: &VariableId,
    environment: Option<&EnvironmentId>,
) -> Result<(), StoreError> {
    execute(
        conn,
        "DELETE FROM variable_environments WHERE variable = ?1",
        (variable.as_str(),),
    )
    .await?;
    if let Some(environment) = environment {
        execute(
            conn,
            "INSERT INTO variable_environments (variable, collection, environment) VALUES (?1, ?2, ?3)",
            (variable.as_str(), collection.as_str(), environment.as_str()),
        )
        .await?;
    }
    Ok(())
}

/// A project's named environments, in its order.
async fn environments_of(conn: &Connection, collection: &CollectionId) -> Result<Vec<Environment>, StoreError> {
    query_all(
        conn,
        &format!("SELECT {ENVIRONMENT_COLUMNS} FROM environments WHERE collection = ?1 ORDER BY position, rowid"),
        (collection.as_str(),),
        rows::environment,
    )
    .await
}

/// Writes `environments` as `collection`'s, in their order.
async fn insert_environments(
    conn: &Connection,
    collection: &CollectionId,
    environments: &[Environment],
) -> Result<(), StoreError> {
    for (position, environment) in environments.iter().enumerate() {
        execute(
            conn,
            "INSERT INTO environments (id, collection, name, position, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                environment.id.as_str(),
                collection.as_str(),
                environment.name.as_str(),
                i64::try_from(position).unwrap_or(i64::MAX),
                environment.created_at.to_rfc3339(),
                environment.updated_at.to_rfc3339(),
            ),
        )
        .await?;
    }
    Ok(())
}

/// `id`, if any, is one of `collection`'s environments.
async fn require_environment(
    conn: &Connection,
    collection: &CollectionId,
    id: Option<&EnvironmentId>,
) -> Result<(), StoreError> {
    match id {
        Some(id) => known_environment(&environments_of(conn, collection).await?, id),
        None => Ok(()),
    }
}

/// What a project sets (#66), as [`write_folder_settings`].
async fn write_project_settings(
    conn: &Connection,
    collection: &CollectionId,
    settings: &ProjectSettings,
) -> Result<(), StoreError> {
    if settings.is_empty() {
        execute(
            conn,
            "DELETE FROM project_settings WHERE collection = ?1",
            (collection.as_str(),),
        )
        .await?;
    } else {
        let json = serde_json::to_string(settings).map_err(backend)?;
        execute(
            conn,
            "INSERT INTO project_settings (collection, settings) VALUES (?1, ?2)
             ON CONFLICT (collection) DO UPDATE SET settings = excluded.settings",
            (collection.as_str(), json),
        )
        .await?;
    }
    Ok(())
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

/// A configuration in place of the project's (`TreeStore::replace`), on
/// `conn` inside the caller's transaction; the new revision.
async fn replace_project(
    conn: &Connection,
    collection: &CollectionId,
    config: ProjectConfig,
    base: Option<u64>,
    now: Timestamp,
) -> Result<u64, StoreError> {
    let revision = get_collection(conn, collection).await?.revision;
    let config = project_config(collection, config)?;
    if let Some(base) = base
        && base != revision
    {
        return Err(StoreError::Stale { base, revision });
    }
    let items = config.folders.iter().map(|folder| folder.id.as_str());
    let items = items.chain(config.apis.iter().map(|api| api.id.as_str()));
    for (id, tables) in items.map(|id| (id, ["api_folders", "api_settings"].as_slice())).chain(
        config
            .variables
            .iter()
            .map(|variable| (variable.id.as_str(), ["variables"].as_slice())),
    ) {
        for table in tables {
            let owner = query_one(
                conn,
                &format!("SELECT collection FROM {table} WHERE id = ?1"),
                (id,),
                |row| rows::text(row, 0),
            )
            .await?;
            if owner.is_some_and(|owner| owner != collection.as_str()) {
                return Err(StoreError::IdTaken { id: id.to_string() });
            }
        }
    }
    for environment in &config.environments {
        let owner = query_one(
            conn,
            "SELECT collection FROM environments WHERE id = ?1",
            (environment.id.as_str(),),
            |row| rows::text(row, 0),
        )
        .await?;
        if owner.is_some_and(|owner| owner != collection.as_str()) {
            return Err(StoreError::IdTaken {
                id: environment.id.to_string(),
            });
        }
    }
    let staying: Vec<&str> = config.apis.iter().map(|api| api.id.as_str()).collect();
    let kept_apis = query_all(
        conn,
        "SELECT id FROM api_settings WHERE collection = ?1",
        (collection.as_str(),),
        |row| rows::text(row, 0),
    )
    .await?;
    for gone in kept_apis.iter().filter(|id| !staying.contains(&id.as_str())) {
        execute(
            conn,
            "DELETE FROM responses WHERE api = ?1 AND collection = ?2",
            (gone.as_str(), collection.as_str()),
        )
        .await?;
    }
    for sql in [
        "DELETE FROM api_folders WHERE collection = ?1",
        "DELETE FROM folder_settings WHERE collection = ?1",
        "DELETE FROM api_settings WHERE collection = ?1",
        "DELETE FROM variables WHERE collection = ?1",
        "DELETE FROM variable_environments WHERE collection = ?1",
        "DELETE FROM environments WHERE collection = ?1",
    ] {
        execute(conn, sql, (collection.as_str(),)).await?;
    }
    insert_environments(conn, collection, &config.environments).await?;
    for folder in &config.folders {
        execute(
            conn,
            "INSERT INTO api_folders (id, collection, children, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                folder.id.as_str(),
                collection.as_str(),
                columns::join_id_list(&folder.children),
                folder.name.as_str(),
                folder.created_at.to_rfc3339(),
                folder.updated_at.to_rfc3339(),
            ),
        )
        .await?;
        write_folder_settings(conn, collection, &folder.id, &folder.settings).await?;
    }
    write_project_settings(conn, collection, &config.settings).await?;
    for api in &config.apis {
        execute(
            conn,
            "INSERT INTO api_settings (id, collection, name, category, setting, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'http', ?4, ?5, ?6)",
            (
                api.id.as_str(),
                collection.as_str(),
                api.name.as_str(),
                api.request.to_json(),
                api.created_at.to_rfc3339(),
                api.updated_at.to_rfc3339(),
            ),
        )
        .await?;
    }
    for variable in &config.variables {
        execute(
            conn,
            "INSERT INTO variables (id, category, collection, name, value, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                variable.id.as_str(),
                variable.category.as_str(),
                collection.as_str(),
                variable.name.as_str(),
                variable.value.as_str(),
                columns::enabled_text(variable.enabled),
                variable.created_at.to_rfc3339(),
                variable.updated_at.to_rfc3339(),
            ),
        )
        .await?;
        write_variable_environment(conn, collection, &variable.id, variable.environment.as_ref()).await?;
    }
    write_change(
        conn,
        collection,
        &TreeChange {
            top_order: Some(config.top_order.clone()),
            ..Default::default()
        },
    )
    .await?;
    execute(
        conn,
        "UPDATE api_collections
         SET name = ?1, description = ?2, updated_at = ?3, revision = revision + 1 WHERE id = ?4",
        (
            config.name.as_str(),
            config.description.as_str(),
            now.to_rfc3339(),
            collection.as_str(),
        ),
    )
    .await?;
    Ok(revision + 1)
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

    fn api_counts(&self) -> StoreFuture<BTreeMap<CollectionId, usize>> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            let counts = query_all(
                &conn,
                "SELECT collection, COUNT(*) FROM api_settings GROUP BY collection",
                (),
                rows::api_count,
            )
            .await?;
            Ok(counts.into_iter().collect())
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
                revision: 0,
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
                "UPDATE api_collections
                 SET name = ?1, description = ?2, updated_at = ?3, revision = revision + 1 WHERE id = ?4",
                (name.as_str(), edit.description.as_str(), now.to_rfc3339(), id.as_str()),
            )
            .await?;
            collection.name = name;
            collection.description = edit.description;
            collection.updated_at = now;
            collection.revision += 1;
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
                "DELETE FROM folder_settings WHERE collection = ?1",
                "DELETE FROM project_settings WHERE collection = ?1",
                "DELETE FROM variable_environments WHERE collection = ?1",
                "DELETE FROM environments WHERE collection = ?1",
                "DELETE FROM api_settings WHERE collection = ?1",
                "DELETE FROM variables WHERE collection = ?1",
                "DELETE FROM top_orders WHERE collection = ?1",
                "DELETE FROM responses WHERE collection = ?1",
                "DELETE FROM server_copies WHERE collection = ?1",
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
                settings: Default::default(),
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

    fn set_folder_settings(
        &self,
        collection: CollectionId,
        folder: ItemId,
        settings: FolderSettings,
    ) -> StoreFuture<Folder> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            let found = execute(
                &tx,
                "UPDATE api_folders SET updated_at = ?1 WHERE id = ?2 AND collection = ?3",
                (now.to_rfc3339(), folder.as_str(), collection.as_str()),
            )
            .await?;
            if found == 0 {
                return Err(StoreError::not_found(Entity::Item, &folder));
            }
            write_folder_settings(&tx, &collection, &folder, &settings).await?;
            let saved = query_one(
                &tx,
                &format!("SELECT {FOLDER_COLUMNS} FROM api_folders WHERE id = ?1"),
                (folder.as_str(),),
                rows::folder,
            )
            .await?
            .ok_or_else(|| StoreError::not_found(Entity::Item, &folder))?;
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(Folder { settings, ..saved })
        })
    }

    fn set_project_settings(&self, collection: CollectionId, settings: ProjectSettings) -> StoreFuture<()> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            write_project_settings(&tx, &collection, &settings).await?;
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)
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

    fn import(
        &self,
        collection: CollectionId,
        parent: Option<ItemId>,
        items: Vec<ImportItem>,
    ) -> StoreFuture<Imported> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            let assigned = assign_ids(&items, &mut || ItemId::new(new_id()));
            let change = tree_data(&tx, &collection)
                .await?
                .plan_import(&assigned.top, parent.as_ref())?;
            let mut imported = Imported {
                change,
                ..Default::default()
            };
            for (id, name, children, settings) in assigned.folders {
                let folder = Folder {
                    id,
                    collection: collection.clone(),
                    name,
                    children,
                    settings,
                    created_at: now,
                    updated_at: now,
                };
                write_folder_settings(&tx, &collection, &folder.id, &folder.settings).await?;
                execute(
                    &tx,
                    "INSERT INTO api_folders (id, collection, children, name, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    (
                        folder.id.as_str(),
                        collection.as_str(),
                        columns::join_id_list(&folder.children),
                        folder.name.as_str(),
                        now.to_rfc3339(),
                        now.to_rfc3339(),
                    ),
                )
                .await?;
                imported.folders.push(folder);
            }
            for (id, name, request) in assigned.apis {
                let api = ApiSetting {
                    id,
                    collection: collection.clone(),
                    name,
                    category: ApiCategory::Http,
                    request,
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
                imported.apis.push(api);
            }
            write_change(&tx, &collection, &imported.change).await?;
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(imported)
        })
    }

    fn delete(&self, collection: CollectionId, item: ItemId) -> StoreFuture<Removal> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            let removal = tree_data(&tx, &collection).await?.plan_remove(&item)?;
            for id in &removal.folders {
                for sql in [
                    "DELETE FROM api_folders WHERE id = ?1 AND collection = ?2",
                    "DELETE FROM folder_settings WHERE folder = ?1 AND collection = ?2",
                ] {
                    execute(&tx, sql, (id.as_str(), collection.as_str())).await?;
                }
            }
            for id in &removal.apis {
                for sql in [
                    "DELETE FROM api_settings WHERE id = ?1 AND collection = ?2",
                    "DELETE FROM responses WHERE api = ?1 AND collection = ?2",
                ] {
                    execute(&tx, sql, (id.as_str(), collection.as_str())).await?;
                }
            }
            write_change(&tx, &collection, &removal.change).await?;
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(removal)
        })
    }

    fn config(&self, collection: CollectionId) -> StoreFuture<ConfigAt> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            let found = get_collection(&conn, &collection).await?;
            let tree = tree_data(&conn, &collection).await?;
            let variables = query_all(
                &conn,
                &format!("SELECT {VARIABLE_COLUMNS} FROM variables WHERE collection = ?1 ORDER BY rowid"),
                (collection.as_str(),),
                rows::variable,
            )
            .await?;
            let variables = in_environments(variables, &variable_environments(&conn, Some(&collection)).await?);
            let environments = environments_of(&conn, &collection).await?;
            Ok(ConfigAt {
                revision: found.revision,
                config: ProjectConfig {
                    name: found.name,
                    description: found.description,
                    folders: tree.folders,
                    apis: tree.apis,
                    top_order: tree.top_order,
                    variables,
                    settings: tree.settings,
                    environments,
                },
            })
        })
    }

    /// The project's rows go and the configuration's are written in their
    /// place, in its order; the responses of the APIs that stay are left
    /// as they are (nothing here ties them to the API's row).
    fn replace(&self, collection: CollectionId, config: ProjectConfig, base: Option<u64>) -> StoreFuture<u64> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            let revision = replace_project(&tx, &collection, config, base, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(revision)
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
            Ok(in_environments(
                found,
                &variable_environments(&conn, Some(&collection)).await?,
            ))
        })
    }

    fn create(&self, input: NewVariable) -> StoreFuture<Variable> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &input.collection).await?;
            require_environment(&tx, &input.collection, input.environment.as_ref()).await?;
            let variable = Variable {
                id: VariableId::new(new_id()),
                collection: input.collection,
                category: input.category,
                name: input.name,
                value: input.value,
                enabled: input.enabled,
                environment: input.environment,
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
            write_variable_environment(&tx, &variable.collection, &variable.id, variable.environment.as_ref()).await?;
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
            require_environment(&tx, &collection, edit.environment.as_ref()).await?;
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
            write_variable_environment(&tx, &collection, &id, edit.environment.as_ref()).await?;
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(Variable {
                environment: edit.environment,
                ..variable
            })
        })
    }

    fn delete(&self, collection: CollectionId, ids: Vec<VariableId>) -> StoreFuture<()> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            for id in &ids {
                for sql in [
                    "DELETE FROM variables WHERE id = ?1 AND collection = ?2",
                    "DELETE FROM variable_environments WHERE variable = ?1 AND collection = ?2",
                ] {
                    execute(&tx, sql, (id.as_str(), collection.as_str())).await?;
                }
            }
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)
        })
    }

    fn environments(&self, collection: CollectionId) -> StoreFuture<Vec<Environment>> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            require_collection(&conn, &collection).await?;
            environments_of(&conn, &collection).await
        })
    }

    fn create_environment(&self, collection: CollectionId, name: String) -> StoreFuture<Environment> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            let existing = environments_of(&tx, &collection).await?;
            let environment = Environment {
                id: EnvironmentId::new(new_id()),
                collection: collection.clone(),
                name: environment_name(&existing, &name, None)?,
                created_at: now,
                updated_at: now,
            };
            execute(
                &tx,
                "INSERT INTO environments (id, collection, name, position, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    environment.id.as_str(),
                    collection.as_str(),
                    environment.name.as_str(),
                    i64::try_from(existing.len()).unwrap_or(i64::MAX),
                    now.to_rfc3339(),
                    now.to_rfc3339(),
                ),
            )
            .await?;
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(environment)
        })
    }

    fn rename_environment(
        &self,
        collection: CollectionId,
        id: EnvironmentId,
        name: String,
    ) -> StoreFuture<Environment> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            let existing = environments_of(&tx, &collection).await?;
            let name = environment_name(&existing, &name, Some(&id))?;
            let found = existing
                .into_iter()
                .find(|environment| environment.id == id)
                .ok_or_else(|| StoreError::not_found(Entity::Environment, &id))?;
            execute(
                &tx,
                "UPDATE environments SET name = ?1, updated_at = ?2 WHERE id = ?3 AND collection = ?4",
                (name.as_str(), now.to_rfc3339(), id.as_str(), collection.as_str()),
            )
            .await?;
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(Environment {
                name,
                updated_at: now,
                ..found
            })
        })
    }

    fn delete_environment(&self, collection: CollectionId, id: EnvironmentId) -> StoreFuture<()> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            require_environment(&tx, &collection, Some(&id)).await?;
            for sql in [
                "DELETE FROM variables WHERE collection = ?1 AND id IN
                 (SELECT variable FROM variable_environments WHERE environment = ?2)",
                "DELETE FROM variable_environments WHERE collection = ?1 AND environment = ?2",
                "DELETE FROM environments WHERE collection = ?1 AND id = ?2",
            ] {
                execute(&tx, sql, (collection.as_str(), id.as_str())).await?;
            }
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)
        })
    }

    fn reorder_environments(
        &self,
        collection: CollectionId,
        order: Vec<EnvironmentId>,
    ) -> StoreFuture<Vec<Environment>> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_collection(&tx, &collection).await?;
            let placed = reordered(&environments_of(&tx, &collection).await?, &order);
            for (position, environment) in placed.iter().enumerate() {
                execute(
                    &tx,
                    "UPDATE environments SET position = ?1 WHERE id = ?2 AND collection = ?3",
                    (
                        i64::try_from(position).unwrap_or(i64::MAX),
                        environment.id.as_str(),
                        collection.as_str(),
                    ),
                )
                .await?;
            }
            touch(&tx, &collection, now).await?;
            tx.commit().await.map_err(backend)?;
            Ok(placed)
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

    fn clear_workspaces(&self) -> StoreFuture<()> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            execute(&conn, "DELETE FROM prefs WHERE scope LIKE 'workspace/%'", ())
                .await
                .map(|_| ())
        })
    }
}

/// The project's API `api`, as a history's owner.
async fn require_api(conn: &Connection, collection: &CollectionId, api: &ItemId) -> Result<(), StoreError> {
    require_collection(conn, collection).await?;
    query_one(
        conn,
        "SELECT id FROM api_settings WHERE id = ?1 AND collection = ?2",
        (api.as_str(), collection.as_str()),
        |row| rows::text(row, 0),
    )
    .await?
    .map(|_| ())
    .ok_or_else(|| StoreError::not_found(Entity::Api, api))
}

/// A kept response: the whole entry is its document.
fn response(row: &Row) -> Result<HttpResponse, StoreError> {
    serde_json::from_str(&rows::text(row, 0)).map_err(backend)
}

impl HistoryStore for TursoStore {
    fn add(&self, response: HttpResponse) -> StoreFuture<HttpResponse> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            require_api(&tx, &response.collection, &response.api).await?;
            let kept = HttpResponse {
                id: new_id(),
                created_at: now,
                ..response
            };
            let (collection, api) = (kept.collection.as_str(), kept.api.as_str());
            execute(
                &tx,
                "INSERT INTO responses (id, collection, api, doc, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                (
                    kept.id.as_str(),
                    collection,
                    api,
                    serde_json::to_string(&kept).map_err(backend)?,
                    now.to_rfc3339(),
                ),
            )
            .await?;
            // The oldest beyond the limit go.
            let oldest_kept = query_one(
                &tx,
                &format!(
                    "SELECT rowid FROM responses WHERE collection = ?1 AND api = ?2
                     ORDER BY rowid DESC LIMIT 1 OFFSET {}",
                    HISTORY_LIMIT - 1
                ),
                (collection, api),
                |row| rows::text(row, 0),
            )
            .await?;
            if let Some(rowid) = oldest_kept.and_then(|rowid| rowid.parse::<i64>().ok()) {
                execute(
                    &tx,
                    "DELETE FROM responses WHERE collection = ?1 AND api = ?2 AND rowid < ?3",
                    (collection, api, rowid),
                )
                .await?;
            }
            tx.commit().await.map_err(backend)?;
            Ok(kept)
        })
    }

    fn list(&self, collection: CollectionId, api: ItemId) -> StoreFuture<Vec<HttpResponse>> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            require_api(&conn, &collection, &api).await?;
            query_all(
                &conn,
                "SELECT doc FROM responses WHERE collection = ?1 AND api = ?2 ORDER BY rowid DESC",
                (collection.as_str(), api.as_str()),
                response,
            )
            .await?
            .into_iter()
            .collect()
        })
    }

    fn latest(&self, collection: CollectionId, apis: Vec<ItemId>) -> StoreFuture<Vec<HttpResponse>> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            require_collection(&conn, &collection).await?;
            let mut found = Vec::new();
            for api in &apis {
                if let Some(newest) = query_one(
                    &conn,
                    "SELECT doc FROM responses WHERE collection = ?1 AND api = ?2 ORDER BY rowid DESC LIMIT 1",
                    (collection.as_str(), api.as_str()),
                    response,
                )
                .await?
                {
                    found.push(newest?);
                }
            }
            Ok(found)
        })
    }

    fn clear(&self, collection: CollectionId, api: ItemId) -> StoreFuture<()> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            require_api(&conn, &collection, &api).await?;
            execute(
                &conn,
                "DELETE FROM responses WHERE collection = ?1 AND api = ?2",
                (collection.as_str(), api.as_str()),
            )
            .await
            .map(|_| ())
        })
    }

    fn clear_all(&self) -> StoreFuture<()> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            execute(&conn, "DELETE FROM responses", ()).await.map(|_| ())
        })
    }
}

impl ServerCopyStore for TursoStore {
    fn copies(&self) -> StoreFuture<Vec<CopyBinding>> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            query_all(
                &conn,
                "SELECT collection, account, base, unsynced FROM server_copies ORDER BY rowid",
                (),
                rows::copy_binding,
            )
            .await
        })
    }

    fn bind(&self, binding: CopyBinding) -> StoreFuture<()> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            execute(
                &conn,
                "INSERT INTO server_copies (collection, account, base, unsynced) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (collection) DO UPDATE
                 SET account = excluded.account, base = excluded.base, unsynced = excluded.unsynced",
                (
                    binding.collection.as_str(),
                    binding.account.as_str(),
                    binding.base.and_then(|base| i64::try_from(base).ok()),
                    i64::from(binding.unsynced),
                ),
            )
            .await?;
            Ok(())
        })
    }

    fn unbind(&self, collection: CollectionId) -> StoreFuture<()> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            execute(
                &conn,
                "DELETE FROM server_copies WHERE collection = ?1",
                (collection.as_str(),),
            )
            .await?;
            Ok(())
        })
    }

    fn keep_copy(&self, collection: Collection, config: ProjectConfig) -> StoreFuture<()> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            execute(
                &tx,
                "INSERT INTO api_collections (id, name, description, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (id) DO NOTHING",
                (
                    collection.id.as_str(),
                    collection.name.as_str(),
                    collection.description.as_str(),
                    collection.created_at.to_rfc3339(),
                    collection.updated_at.to_rfc3339(),
                ),
            )
            .await?;
            replace_project(&tx, &collection.id, config, None, now).await?;
            tx.commit().await.map_err(backend)
        })
    }
}

impl BackupStore for TursoStore {
    fn snapshot(&self) -> StoreFuture<Backup> {
        self.run(|inner, _| async move {
            let conn = inner.conn.lock().await;
            let collections = query_all(
                &conn,
                &format!("SELECT {COLLECTION_COLUMNS} FROM api_collections ORDER BY rowid"),
                (),
                rows::collection,
            )
            .await?;
            let mut folders = query_all(
                &conn,
                &format!("SELECT {FOLDER_COLUMNS} FROM api_folders ORDER BY rowid"),
                (),
                rows::folder,
            )
            .await?;
            let mut settings = folder_settings(&conn, None).await?;
            for folder in &mut folders {
                if let Some(own) = settings.remove(folder.id.as_str()) {
                    folder.settings = own;
                }
            }
            let project_settings = query_all(
                &conn,
                "SELECT collection, settings FROM project_settings ORDER BY rowid",
                (),
                |row| (CollectionId::new(rows::text(row, 0)), settings_of(&rows::text(row, 1))),
            )
            .await?;
            let apis = query_all(
                &conn,
                &format!("SELECT {API_COLUMNS} FROM api_settings ORDER BY rowid"),
                (),
                rows::api,
            )
            .await?;
            let variables = query_all(
                &conn,
                &format!("SELECT {VARIABLE_COLUMNS} FROM variables ORDER BY rowid"),
                (),
                rows::variable,
            )
            .await?;
            let variables = in_environments(variables, &variable_environments(&conn, None).await?);
            let environments = query_all(
                &conn,
                &format!("SELECT {ENVIRONMENT_COLUMNS} FROM environments ORDER BY collection, position, rowid"),
                (),
                rows::environment,
            )
            .await?;
            let top_orders = query_all(
                &conn,
                "SELECT collection, items FROM top_orders ORDER BY rowid",
                (),
                |row| {
                    (
                        CollectionId::new(rows::text(row, 0)),
                        columns::parse_id_list(&rows::text(row, 1)),
                    )
                },
            )
            .await?;
            Ok(Backup {
                collections,
                folders,
                apis,
                variables,
                top_orders,
                project_settings,
                environments,
            })
        })
    }

    fn restore(&self, backup: Backup) -> StoreFuture<()> {
        self.run(move |inner, _| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            for table in [
                "api_collections",
                "api_folders",
                "folder_settings",
                "project_settings",
                "environments",
                "variable_environments",
                "api_settings",
                "variables",
                "top_orders",
                "responses",
            ] {
                execute(&tx, &format!("DELETE FROM {table}"), ()).await?;
            }
            for c in &backup.collections {
                execute(
                    &tx,
                    "INSERT INTO api_collections (id, name, description, created_at, updated_at, revision)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    (
                        c.id.as_str(),
                        c.name.as_str(),
                        c.description.as_str(),
                        c.created_at.to_rfc3339(),
                        c.updated_at.to_rfc3339(),
                        i64::try_from(c.revision).unwrap_or_default(),
                    ),
                )
                .await?;
            }
            for f in &backup.folders {
                execute(
                    &tx,
                    "INSERT INTO api_folders (id, collection, children, name, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    (
                        f.id.as_str(),
                        f.collection.as_str(),
                        columns::join_id_list(&f.children),
                        f.name.as_str(),
                        f.created_at.to_rfc3339(),
                        f.updated_at.to_rfc3339(),
                    ),
                )
                .await?;
                write_folder_settings(&tx, &f.collection, &f.id, &f.settings).await?;
            }
            for (collection, settings) in &backup.project_settings {
                write_project_settings(&tx, collection, settings).await?;
            }
            for a in &backup.apis {
                execute(
                    &tx,
                    "INSERT INTO api_settings (id, collection, name, category, setting, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 'http', ?4, ?5, ?6)",
                    (
                        a.id.as_str(),
                        a.collection.as_str(),
                        a.name.as_str(),
                        a.request.to_json(),
                        a.created_at.to_rfc3339(),
                        a.updated_at.to_rfc3339(),
                    ),
                )
                .await?;
            }
            for v in &backup.variables {
                execute(
                    &tx,
                    "INSERT INTO variables (id, category, collection, name, value, enabled, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    (
                        v.id.as_str(),
                        v.category.as_str(),
                        v.collection.as_str(),
                        v.name.as_str(),
                        v.value.as_str(),
                        if v.enabled { "1" } else { "0" },
                        v.created_at.to_rfc3339(),
                        v.updated_at.to_rfc3339(),
                    ),
                )
                .await?;
                write_variable_environment(&tx, &v.collection, &v.id, v.environment.as_ref()).await?;
            }
            let mut by_project: BTreeMap<&CollectionId, Vec<Environment>> = BTreeMap::new();
            for environment in &backup.environments {
                by_project
                    .entry(&environment.collection)
                    .or_default()
                    .push(environment.clone());
            }
            for (collection, environments) in by_project {
                insert_environments(&tx, collection, &environments).await?;
            }
            for (collection, items) in &backup.top_orders {
                execute(
                    &tx,
                    "INSERT INTO top_orders (collection, items) VALUES (?1, ?2)",
                    (collection.as_str(), columns::join_id_list(items)),
                )
                .await?;
            }
            tx.commit().await.map_err(backend)
        })
    }
}

fn cookie_key(domain: &str, path: &str, name: &str) -> String {
    format!("{domain}\t{path}\t{name}")
}

impl CookieStore for TursoStore {
    fn list(&self) -> StoreFuture<Vec<Cookie>> {
        self.run(|inner, _| async move {
            let conn = inner.conn.lock().await;
            let docs = query_all(&conn, "SELECT doc FROM cookies", (), |row| rows::text(row, 0)).await?;
            let mut cookies: Vec<Cookie> = docs
                .iter()
                .map(|doc| serde_json::from_str(doc).map_err(backend))
                .collect::<Result<_, _>>()?;
            sort_cookies(&mut cookies);
            Ok(cookies)
        })
    }

    fn put(&self, cookies: Vec<Cookie>) -> StoreFuture<()> {
        self.run(move |inner, now| async move {
            let mut conn = inner.conn.lock().await;
            let tx = conn.transaction().await.map_err(backend)?;
            for cookie in cookies {
                let key = cookie_key(&cookie.domain, &cookie.path, &cookie.name);
                if cookie.is_expired(now) {
                    execute(&tx, "DELETE FROM cookies WHERE key = ?1", (key,)).await?;
                } else {
                    execute(
                        &tx,
                        "INSERT INTO cookies (key, doc) VALUES (?1, ?2)
                         ON CONFLICT (key) DO UPDATE SET doc = excluded.doc",
                        (key, serde_json::to_string(&cookie).map_err(backend)?),
                    )
                    .await?;
                }
            }
            tx.commit().await.map_err(backend)
        })
    }

    fn delete(&self, domain: String, path: String, name: String) -> StoreFuture<()> {
        self.run(move |inner, _| async move {
            let conn = inner.conn.lock().await;
            execute(
                &conn,
                "DELETE FROM cookies WHERE key = ?1",
                (cookie_key(&domain, &path, &name),),
            )
            .await
            .map(|_| ())
        })
    }

    fn clear(&self) -> StoreFuture<()> {
        self.run(|inner, _| async move {
            let conn = inner.conn.lock().await;
            execute(&conn, "DELETE FROM cookies", ()).await.map(|_| ())
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
                    "SELECT count(*) FROM folder_settings WHERE collection = ?1",
                    id.to_string(),
                ),
                (
                    "SELECT count(*) FROM project_settings WHERE collection = ?1",
                    id.to_string(),
                ),
                (
                    "SELECT count(*) FROM environments WHERE collection = ?1",
                    id.to_string(),
                ),
                (
                    "SELECT count(*) FROM variable_environments WHERE collection = ?1",
                    id.to_string(),
                ),
                ("SELECT count(*) FROM responses WHERE collection = ?1", id.to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locked_database_is_recognized() {
        assert!(file_is_locked(&StoreError::backend(
            "File is locked by another process"
        )));
        assert!(file_is_locked(&StoreError::backend("database is locked")));
        assert!(!file_is_locked(&StoreError::backend(
            "I/O error (open): entity not found"
        )));
        assert!(!file_is_locked(&StoreError::NameRequired));
    }
}
