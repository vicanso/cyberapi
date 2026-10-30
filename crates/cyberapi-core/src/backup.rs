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
    format!("cyberapi-backup-{}.zip", at.get().format(NAMED_AT))
}

/// How a backup's file name says when it was made, on the wall clock.
const NAMED_AT: &str = "%Y-%m-%d-%H%M%S";

/// F6: how many of its own backups the desktop keeps.
pub const KEPT: usize = 7;

/// F6: how long the desktop waits after one of its own backups before it
/// makes the next.
pub const EVERY: chrono::TimeDelta = chrono::TimeDelta::hours(24);

/// F6: when the backup named `name` ([`file_name`]) was made; `None` for a
/// file that is not one.
pub fn made_at(name: &str) -> Option<chrono::NaiveDateTime> {
    let at = name.strip_prefix("cyberapi-backup-")?.strip_suffix(".zip")?;
    chrono::NaiveDateTime::parse_from_str(at, NAMED_AT).ok()
}

/// F6: how far ahead of the wall clock a backup's name may be and still
/// count as made a moment ago: the reader crossed time zones westward,
/// which a name — the wall clock's — does not say. Further ahead, the
/// clock was wrong when it was made (BUG-6).
const AHEAD: chrono::TimeDelta = chrono::TimeDelta::hours(26);

/// Whether a backup named for `made` was named by a clock that was wrong.
fn misdated(made: chrono::NaiveDateTime, now: chrono::NaiveDateTime) -> bool {
    now.checked_add_signed(AHEAD).is_some_and(|ahead| made > ahead)
}

/// F6: whether, with the files `names` in the backups' folder, another is
/// due `now` (the wall clock): there is none, or the newest is [`EVERY`]
/// old. One a little ahead of the clock was made a moment ago, elsewhere;
/// one far ahead counts for nothing — else every check would find it, and
/// a backup would be due each time (BUG-6).
pub fn due<S: AsRef<str>>(names: &[S], now: chrono::NaiveDateTime) -> bool {
    let newest = names
        .iter()
        .filter_map(|name| made_at(name.as_ref()))
        .filter(|made| !misdated(*made, now))
        .max();
    newest.is_none_or(|newest| now.signed_duration_since(newest) >= EVERY)
}

/// F6: those of `names` to remove so that only the newest [`KEPT`] backups
/// stay, `now`. One named far ahead of the clock is the oldest — made before
/// the clock was put right — and goes first; else it would outlast every
/// new one. What is not a backup is never among them.
pub fn pruned<S: AsRef<str>>(names: &[S], now: chrono::NaiveDateTime) -> Vec<String> {
    let mut backups: Vec<(bool, chrono::NaiveDateTime, &str)> = names
        .iter()
        .filter_map(|name| {
            let made = made_at(name.as_ref())?;
            Some((!misdated(made, now), made, name.as_ref()))
        })
        .collect();
    backups.sort();
    let extra = backups.len().saturating_sub(KEPT);
    backups[..extra].iter().map(|(_, _, name)| name.to_string()).collect()
}

impl Backup {
    /// No project: nothing worth keeping.
    pub fn is_empty(&self) -> bool {
        self.collections.is_empty()
    }
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
    /// F6: a day between the desktop's own; seven kept; nothing else
    /// touched.
    #[test]
    fn a_backup_is_due_once_a_day_and_seven_are_kept() {
        let at = |text: &str| chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").expect("a time");
        let names: Vec<String> = (1..=9)
            .map(|day| format!("cyberapi-backup-2026-10-{day:02}-093000.zip"))
            .chain(["notes.txt".to_string(), "cyberapi-backup-x.zip".to_string()])
            .collect();
        assert_eq!(made_at(&names[0]), Some(at("2026-10-01 09:30")), "the name's own time");
        let now = at("2026-10-10 09:00");
        assert!(!due(&names, now));
        assert!(due(&names, at("2026-10-10 09:30")));
        assert!(due(&["notes.txt"], now));
        assert_eq!(
            pruned(&names, now),
            [
                "cyberapi-backup-2026-10-01-093000.zip",
                "cyberapi-backup-2026-10-02-093000.zip"
            ]
        );
        assert!(pruned(&names[..7], now).is_empty());
    }

    /// BUG-6: a backup named ahead of the clock does not make one due on
    /// every check, nor outlast every new one.
    #[test]
    fn a_backup_ahead_of_the_clock_keeps_the_day() {
        let at = |text: &str| chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").expect("a time");
        let name = |at: &str| format!("cyberapi-backup-{at}.zip");
        // Westward across seven time zones: the newest is seven hours
        // ahead, and was made a moment ago.
        let newest = [name("2026-10-09-160000")];
        assert!(!due(&newest, at("2026-10-09 09:00")));
        assert!(!due(&newest, at("2026-10-10 09:00")));
        assert!(due(&newest, at("2026-10-10 16:00")), "a day after it");

        // Made while the clock was a year ahead: they count for nothing.
        let mut names: Vec<String> = (1..=7).map(|day| name(&format!("2027-10-{day:02}-093000"))).collect();
        let now = at("2026-10-10 09:00");
        assert!(due(&names, now));
        names.push(name("2026-10-10-090000"));
        assert!(!due(&names, at("2026-10-10 10:00")), "an hour later, not again");
        assert_eq!(
            pruned(&names, at("2026-10-10 10:00")),
            [name("2027-10-01-093000")],
            "the misdated go first, not the one just made"
        );
    }

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
                    variables: vec![crate::model::KvParam {
                        key: "page".into(),
                        value: "1".into(),
                        enabled: true,
                        ..Default::default()
                    }],
                    source: Some("https://api.test/users.yaml".into()),
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
                    request_schema: Some(r#"{"type":"object"}"#.into()),
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
                    // B7: and where it syncs from.
                    source: Some(crate::model::ImportSource {
                        url: "https://api.test/openapi.json".into(),
                        folder: Some("f".into()),
                    }),
                    // D6: and which environments are production.
                    protected_environments: vec!["e".into()],
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
