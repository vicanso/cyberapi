//! The desktop's backup (M13): every project, folder, API and variable as
//! the old app's backup held them — one JSON file per table, rows as its
//! columns — so either app restores the other's. The top-level orders ride
//! along in a file the old app does not know (and ignores), and a folder's
//! or a project's settings (#66) in a `settings` column of its row, which
//! the old app ignores too; so do the named environments (#61), a file of
//! their own, and a variable's in an `environment` column.

use crate::model::{
    ApiCategory, ApiSetting, Collection, Environment, Folder, HttpRequest, ProjectSettings, Variable, VariableCategory,
    legacy,
};
use crate::{CollectionId, EnvironmentId, ItemId, Timestamp, VariableId};
use serde_json::{Value, json};
use snafu::Snafu;

pub const COLLECTIONS_FILE: &str = "api_collections.json";
pub const FOLDERS_FILE: &str = "api_folders.json";
pub const APIS_FILE: &str = "api_settings.json";
pub const VARIABLES_FILE: &str = "variables.json";
pub const TOP_ORDERS_FILE: &str = "top_orders.json";
pub const ENVIRONMENTS_FILE: &str = "environments.json";

/// Everything a restore replaces.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Backup {
    pub collections: Vec<Collection>,
    pub folders: Vec<Folder>,
    pub apis: Vec<ApiSetting>,
    pub variables: Vec<Variable>,
    pub top_orders: Vec<(CollectionId, Vec<ItemId>)>,
    /// The projects that set something for their APIs (#66).
    pub project_settings: Vec<(CollectionId, ProjectSettings)>,
    /// Every project's named environments, each project's in its order
    /// (#61).
    pub environments: Vec<Environment>,
}

#[derive(Debug, Snafu, PartialEq, Eq)]
pub enum BackupError {
    #[snafu(display("{file} is missing from the backup"))]
    Missing { file: String },
    #[snafu(display("{file} is not a list of rows: {message}"))]
    Unreadable { file: String, message: String },
}

/// F13.1: what a backup made `at` is called — to the second, so a second
/// backup the same day is a file of its own (#8).
pub fn file_name(at: &Timestamp) -> String {
    format!("cyberapi-backup-{}.zip", at.get().format("%Y-%m-%d-%H%M%S"))
}

fn text(row: &Value, key: &str) -> String {
    match row.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    }
}

fn time(row: &Value, key: &str) -> Timestamp {
    Timestamp::parse_lenient(&text(row, key))
}

/// A `settings` column: JSON text, none when it is missing or unreadable.
fn settings<T: serde::de::DeserializeOwned + Default>(row: &Value) -> T {
    serde_json::from_str(&text(row, "settings")).unwrap_or_default()
}

/// A row with a `settings` column when there is something to keep.
fn with_settings(mut row: Value, settings: Option<String>) -> Value {
    if let (Some(settings), Value::Object(fields)) = (settings, &mut row) {
        fields.insert("settings".into(), Value::String(settings));
    }
    row
}

fn rows(file: &str, bytes: Option<Vec<u8>>) -> Result<Vec<Value>, BackupError> {
    let bytes = bytes.ok_or_else(|| BackupError::Missing { file: file.into() })?;
    serde_json::from_slice(&bytes).map_err(|error| BackupError::Unreadable {
        file: file.into(),
        message: error.to_string(),
    })
}

impl Backup {
    /// The files, by name, in the order the old app wrote them.
    pub fn to_files(&self) -> Vec<(&'static str, Vec<u8>)> {
        let collections: Vec<Value> = self
            .collections
            .iter()
            .map(|c| {
                let settings = self
                    .project_settings
                    .iter()
                    .find(|(id, settings)| id == &c.id && !settings.is_empty())
                    .and_then(|(_, settings)| serde_json::to_string(settings).ok());
                let row = json!({
                    "id": c.id, "name": c.name, "description": c.description,
                    "created_at": c.created_at.to_rfc3339(), "updated_at": c.updated_at.to_rfc3339(),
                });
                with_settings(row, settings)
            })
            .collect();
        let folders: Vec<Value> = self
            .folders
            .iter()
            .map(|f| {
                let row = json!({
                    "id": f.id, "collection": f.collection, "children": legacy::join_id_list(&f.children),
                    "name": f.name, "created_at": f.created_at.to_rfc3339(), "updated_at": f.updated_at.to_rfc3339(),
                });
                let settings = (!f.settings.is_empty()).then(|| serde_json::to_string(&f.settings).ok());
                with_settings(row, settings.flatten())
            })
            .collect();
        let apis: Vec<Value> = self
            .apis
            .iter()
            .map(|a| {
                json!({
                    "id": a.id, "collection": a.collection, "name": a.name, "category": "http",
                    "setting": a.request.to_json(),
                    "created_at": a.created_at.to_rfc3339(), "updated_at": a.updated_at.to_rfc3339(),
                })
            })
            .collect();
        let variables: Vec<Value> = self
            .variables
            .iter()
            .map(|v| {
                let mut row = json!({
                    "id": v.id, "category": v.category.as_str(), "collection": v.collection,
                    "name": v.name, "value": v.value, "enabled": if v.enabled { "1" } else { "0" },
                    "created_at": v.created_at.to_rfc3339(), "updated_at": v.updated_at.to_rfc3339(),
                });
                if let (Some(environment), Value::Object(fields)) = (&v.environment, &mut row) {
                    fields.insert("environment".into(), json!(environment));
                }
                row
            })
            .collect();
        let environments: Vec<Value> = self
            .environments
            .iter()
            .map(|e| {
                json!({
                    "id": e.id, "collection": e.collection, "name": e.name,
                    "created_at": e.created_at.to_rfc3339(), "updated_at": e.updated_at.to_rfc3339(),
                })
            })
            .collect();
        let top_orders: Vec<Value> = self
            .top_orders
            .iter()
            .map(|(collection, items)| json!({"collection": collection, "items": legacy::join_id_list(items)}))
            .collect();
        [
            (COLLECTIONS_FILE, collections),
            (FOLDERS_FILE, folders),
            (APIS_FILE, apis),
            (VARIABLES_FILE, variables),
            (TOP_ORDERS_FILE, top_orders),
            (ENVIRONMENTS_FILE, environments),
        ]
        .into_iter()
        .map(|(name, rows)| (name, serde_json::to_vec(&rows).unwrap_or_default()))
        .collect()
    }

    /// A backup from its files: the four tables must be there; the
    /// top-level orders may not (an old app's backup).
    pub fn from_files(mut read: impl FnMut(&str) -> Option<Vec<u8>>) -> Result<Self, BackupError> {
        let collection_rows = rows(COLLECTIONS_FILE, read(COLLECTIONS_FILE))?;
        let collections = collection_rows
            .iter()
            .map(|row| Collection {
                id: CollectionId::new(text(row, "id")),
                name: text(row, "name"),
                description: text(row, "description"),
                created_at: time(row, "created_at"),
                updated_at: time(row, "updated_at"),
                ..Default::default()
            })
            .collect();
        let project_settings = collection_rows
            .iter()
            .map(|row| (CollectionId::new(text(row, "id")), settings::<ProjectSettings>(row)))
            .filter(|(_, settings)| !settings.is_empty())
            .collect();
        let folders = rows(FOLDERS_FILE, read(FOLDERS_FILE))?
            .iter()
            .map(|row| Folder {
                id: ItemId::new(text(row, "id")),
                collection: CollectionId::new(text(row, "collection")),
                name: text(row, "name"),
                children: legacy::parse_id_list(&text(row, "children")),
                settings: settings(row),
                created_at: time(row, "created_at"),
                updated_at: time(row, "updated_at"),
            })
            .collect();
        let apis = rows(APIS_FILE, read(APIS_FILE))?
            .iter()
            .map(|row| ApiSetting {
                id: ItemId::new(text(row, "id")),
                collection: CollectionId::new(text(row, "collection")),
                name: text(row, "name"),
                category: ApiCategory::Http,
                request: HttpRequest::from_json_lenient(&text(row, "setting")),
                created_at: time(row, "created_at"),
                updated_at: time(row, "updated_at"),
            })
            .collect();
        let variables = rows(VARIABLES_FILE, read(VARIABLES_FILE))?
            .iter()
            .filter_map(|row| {
                Some(Variable {
                    id: VariableId::new(text(row, "id")),
                    collection: CollectionId::new(text(row, "collection")),
                    category: VariableCategory::parse(&text(row, "category"))?,
                    name: text(row, "name"),
                    value: text(row, "value"),
                    enabled: text(row, "enabled") == "1",
                    environment: Some(text(row, "environment"))
                        .filter(|id| !id.is_empty())
                        .map(EnvironmentId::new),
                    created_at: time(row, "created_at"),
                    updated_at: time(row, "updated_at"),
                })
            })
            .collect();
        let top_orders = match read(TOP_ORDERS_FILE) {
            Some(bytes) => rows(TOP_ORDERS_FILE, Some(bytes))?
                .iter()
                .map(|row| {
                    (
                        CollectionId::new(text(row, "collection")),
                        legacy::parse_id_list(&text(row, "items")),
                    )
                })
                .collect(),
            None => Vec::new(),
        };
        // An older backup has none.
        let environments = match read(ENVIRONMENTS_FILE) {
            Some(bytes) => rows(ENVIRONMENTS_FILE, Some(bytes))?
                .iter()
                .map(|row| Environment {
                    id: EnvironmentId::new(text(row, "id")),
                    collection: CollectionId::new(text(row, "collection")),
                    name: text(row, "name"),
                    created_at: time(row, "created_at"),
                    updated_at: time(row, "updated_at"),
                })
                .collect(),
            None => Vec::new(),
        };
        Ok(Self {
            collections,
            folders,
            apis,
            variables,
            top_orders,
            project_settings,
            environments,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Auth;
    use crate::model::{FolderSettings, HttpMethod};
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;

    fn at() -> Timestamp {
        Timestamp::parse_lenient("2024-05-06T07:08:09+08:00")
    }

    fn sample() -> Backup {
        Backup {
            collections: vec![Collection {
                id: "c".into(),
                name: "shop".into(),
                description: "d".into(),
                created_at: at(),
                updated_at: at(),
                ..Default::default()
            }],
            folders: vec![Folder {
                id: "f".into(),
                collection: "c".into(),
                name: "users".into(),
                children: vec!["a".into()],
                settings: FolderSettings {
                    auth: Auth::Bearer { token: "t".into() },
                    headers: vec![],
                    description: String::new(),
                },
                created_at: at(),
                updated_at: at(),
            }],
            apis: vec![ApiSetting {
                id: "a".into(),
                collection: "c".into(),
                name: "login".into(),
                category: ApiCategory::Http,
                request: HttpRequest {
                    method: HttpMethod::Post,
                    uri: "/login".into(),
                    ..Default::default()
                },
                created_at: at(),
                updated_at: at(),
            }],
            variables: vec![Variable {
                id: "v".into(),
                collection: "c".into(),
                category: VariableCategory::GlobalRequestHeader,
                name: "X".into(),
                value: "1".into(),
                enabled: false,
                environment: Some("e".into()),
                created_at: at(),
                updated_at: at(),
            }],
            top_orders: vec![("c".into(), vec!["f".into()])],
            environments: vec![Environment {
                id: "e".into(),
                collection: "c".into(),
                name: "dev".into(),
                created_at: at(),
                updated_at: at(),
            }],
            project_settings: vec![(
                "c".into(),
                ProjectSettings {
                    auth: Auth::Basic {
                        username: "u".into(),
                        password: "p".into(),
                    },
                    // D3: which are secret goes along; their values do not.
                    secret_variables: vec!["v1".into()],
                },
            )],
        }
    }

    #[test]
    fn a_backup_reads_back_whole() {
        let files: HashMap<&str, Vec<u8>> = sample().to_files().into_iter().collect();
        let read = Backup::from_files(|name| files.get(name).cloned()).expect("a backup");
        assert_eq!(read, sample());
        let variables: Value = serde_json::from_slice(&files[VARIABLES_FILE]).expect("JSON");
        assert_eq!(variables[0]["enabled"], "0", "the old columns' form");
        assert_eq!(variables[0]["category"], "globalReqHeaders");
    }

    #[test]
    fn a_backup_is_named_for_its_second() {
        assert_eq!(file_name(&at()), "cyberapi-backup-2024-05-06-070809.zip");
    }

    #[test]
    fn an_old_backup_has_no_orders_but_needs_its_tables() {
        let mut files: HashMap<&str, Vec<u8>> = sample().to_files().into_iter().collect();
        files.remove(TOP_ORDERS_FILE);
        let read = Backup::from_files(|name| files.get(name).cloned()).expect("a backup");
        assert!(read.top_orders.is_empty());
        files.remove(APIS_FILE);
        assert_eq!(
            Backup::from_files(|name| files.get(name).cloned()),
            Err(BackupError::Missing { file: APIS_FILE.into() })
        );
    }
}
