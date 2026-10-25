//! The tables, and how a file reaches the current schema.
//!
//! The five tables the Tauri app made are created exactly as it made them,
//! so a copied `my_db.db` and a new file look the same (4.1). Everything this
//! app adds is a numbered migration on top, recorded in `cyberapi_meta`; the
//! old app's `versions` table is left as it is.

use crate::backend;
use cyberapi_store::StoreError;
use turso::Connection;

/// The old app's tables, as its `get_*_create_sql` wrote them.
const LEGACY_TABLES: [&str; 5] = [
    "CREATE TABLE IF NOT EXISTS api_collections (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        name TEXT DEFAULT '',
        description TEXT DEFAULT '',
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )",
    "CREATE TABLE IF NOT EXISTS api_folders (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        collection TEXT NOT NULL check (collection != ''),
        children TEXT DEFAULT '',
        name TEXT DEFAULT '',
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )",
    "CREATE TABLE IF NOT EXISTS api_settings (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        collection TEXT NOT NULL check (collection != ''),
        name TEXT DEFAULT '',
        category TEXT DEFAULT '',
        setting TEXT DEFAULT '',
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )",
    "CREATE TABLE IF NOT EXISTS variables (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        category TEXT NOT NULL check (category != ''),
        collection TEXT NOT NULL check (collection != ''),
        name TEXT DEFAULT '',
        value TEXT DEFAULT '',
        enabled TEXT DEFAULT '',
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )",
    "CREATE TABLE IF NOT EXISTS versions (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        version TEXT NOT NULL check (version != ''),
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )",
];

const META_TABLE: &str = "CREATE TABLE IF NOT EXISTS cyberapi_meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL DEFAULT ''
)";

const SCHEMA_VERSION_KEY: &str = "schema_version";

/// This app's migrations, in order; migration `n` (1-based) brings a file to
/// schema version `n`. Append only: a released step is never edited, since
/// files out there have already run it.
const MIGRATIONS: &[&[&str]] = &[
    // 1: what used to live in the WebView's localforage (4.6), and indexes
    // for the per-project reads the old app scanned for.
    &[
        // A project's top-level order: comma-separated ids, like `children`.
        "CREATE TABLE IF NOT EXISTS top_orders (
            collection TEXT PRIMARY KEY NOT NULL,
            items TEXT NOT NULL DEFAULT ''
        )",
        // The reader's documents: `app`, and `workspace/<collection id>`.
        "CREATE TABLE IF NOT EXISTS prefs (
            scope TEXT PRIMARY KEY NOT NULL,
            doc TEXT NOT NULL DEFAULT '{}'
        )",
        "CREATE INDEX IF NOT EXISTS api_folders_collection ON api_folders (collection)",
        "CREATE INDEX IF NOT EXISTS api_settings_collection ON api_settings (collection)",
        "CREATE INDEX IF NOT EXISTS variables_collection ON variables (collection, category)",
    ],
    // 2: a project's revision, which every shared change moves on
    // (`docs/architecture.md` §7). The desktop has no other reader to tell,
    // but every store keeps the same rules.
    &["ALTER TABLE api_collections ADD COLUMN revision INTEGER NOT NULL DEFAULT 0"],
    // 3: the responses each API keeps (F7.9), newest by rowid. The old app
    // kept them in the WebView's storage, which is not carried over.
    &[
        "CREATE TABLE IF NOT EXISTS responses (
            id TEXT NOT NULL,
            collection TEXT NOT NULL,
            api TEXT NOT NULL,
            doc TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
        "CREATE INDEX IF NOT EXISTS responses_api ON responses (collection, api)",
    ],
    // 4: the cookie jar (M9), which the old app kept in `cookies.json`. A
    // cookie's document, by its domain, path and name.
    &["CREATE TABLE IF NOT EXISTS cookies (
            key TEXT PRIMARY KEY NOT NULL,
            doc TEXT NOT NULL
        )"],
    // 5: which projects are copies of server projects, and whose
    // (`docs/server-sync.md` §6). A project of the machine's own has no row.
    &["CREATE TABLE IF NOT EXISTS server_copies (
            collection TEXT PRIMARY KEY NOT NULL,
            account TEXT NOT NULL,
            base INTEGER,
            unsynced INTEGER NOT NULL DEFAULT 0
        )"],
    // 6: what folders and projects set for their APIs (#66,
    // `docs/batch4-design.md` §2.2), beside the old app's tables rather
    // than in them. Only those that set something have a row.
    &[
        "CREATE TABLE IF NOT EXISTS folder_settings (
            folder TEXT PRIMARY KEY NOT NULL,
            collection TEXT NOT NULL,
            settings TEXT NOT NULL
        )",
        "CREATE INDEX IF NOT EXISTS folder_settings_collection ON folder_settings (collection)",
        "CREATE TABLE IF NOT EXISTS project_settings (
            collection TEXT PRIMARY KEY NOT NULL,
            settings TEXT NOT NULL
        )",
    ],
];

/// The schema version this build writes.
pub const CURRENT_VERSION: usize = MIGRATIONS.len();

/// Brings a new or older file to [`CURRENT_VERSION`]. A file from a newer
/// build is refused rather than written with rules it does not know.
pub async fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    for sql in LEGACY_TABLES.iter().chain([&META_TABLE]) {
        conn.execute(sql, ()).await.map_err(backend)?;
    }
    let version = schema_version(conn).await?;
    if version > CURRENT_VERSION {
        return Err(StoreError::backend(format!(
            "this database was written by a newer CyberAPI (schema {version}; this one knows {CURRENT_VERSION})"
        )));
    }
    for (index, steps) in MIGRATIONS.iter().enumerate().skip(version) {
        let tx = conn.transaction().await.map_err(backend)?;
        for sql in steps.iter() {
            tx.execute(sql, ()).await.map_err(backend)?;
        }
        tx.execute(
            "INSERT INTO cyberapi_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            (SCHEMA_VERSION_KEY, (index + 1).to_string()),
        )
        .await
        .map_err(backend)?;
        tx.commit().await.map_err(backend)?;
    }
    Ok(())
}

pub async fn schema_version(conn: &Connection) -> Result<usize, StoreError> {
    let mut rows = conn
        .query("SELECT value FROM cyberapi_meta WHERE key = ?1", (SCHEMA_VERSION_KEY,))
        .await
        .map_err(backend)?;
    let Some(row) = rows.next().await.map_err(backend)? else {
        return Ok(0);
    };
    let text = crate::rows::text(&row, 0);
    text.parse()
        .map_err(|_| StoreError::backend(format!("unreadable schema version {text:?}")))
}
