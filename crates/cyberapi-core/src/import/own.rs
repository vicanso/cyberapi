//! CyberAPI's own export (F10.3 / M11): a JSON array of folder and API
//! records as the old app held them — a folder's `children` comma-separated,
//! an API's request a JSON string in `setting`. Both apps read what the
//! other writes.

use super::{ImportBundle, ImportItem};
use crate::ItemId;
use crate::model::{ApiSetting, Folder, HttpRequest, legacy};
use crate::tree::TreeData;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    id: String,
    #[serde(default)]
    collection: String,
    #[serde(default)]
    name: String,
    /// `http` for an API; a folder has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    setting: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    children: Option<String>,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
}

/// The text form, for a file or the clipboard.
pub fn parse_own(text: &str) -> Option<ImportBundle> {
    serde_json::from_str::<Value>(text).ok().map(|json| parse_value(&json))
}

/// One record or an array of them. The top level is what no folder lists;
/// a folder's children keep its order; what lists itself, or loops, is cut.
pub(super) fn parse_value(json: &Value) -> ImportBundle {
    let values: Vec<&Value> = match json {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    let records: Vec<Record> = values
        .into_iter()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .collect();
    let by_id: HashMap<&str, &Record> = records.iter().map(|record| (record.id.as_str(), record)).collect();
    let children_of = |record: &Record| -> Vec<ItemId> {
        record
            .children
            .as_deref()
            .map(legacy::parse_id_list)
            .unwrap_or_default()
    };
    let listed: HashSet<ItemId> = records
        .iter()
        .filter(|record| record.category.as_deref() != Some("http"))
        .flat_map(children_of)
        .collect();

    fn build(
        record: &Record,
        by_id: &HashMap<&str, &Record>,
        children_of: &dyn Fn(&Record) -> Vec<ItemId>,
        visiting: &mut HashSet<String>,
    ) -> Option<ImportItem> {
        if record.category.as_deref() == Some("http") {
            return Some(ImportItem::Api {
                name: record.name.clone(),
                request: HttpRequest::from_json_lenient(record.setting.as_deref().unwrap_or("{}")),
            });
        }
        if !visiting.insert(record.id.clone()) {
            return None;
        }
        let children = children_of(record)
            .iter()
            .filter_map(|id| by_id.get(id.as_str()))
            .filter_map(|child| build(child, by_id, children_of, visiting))
            .collect();
        visiting.remove(&record.id);
        Some(ImportItem::Folder {
            name: record.name.clone(),
            children,
        })
    }

    let mut visiting = HashSet::new();
    let items = records
        .iter()
        .filter(|record| !listed.contains(&ItemId::new(record.id.as_str())))
        .filter_map(|record| build(record, &by_id, &children_of, &mut visiting))
        .collect();
    ImportBundle {
        items,
        environment: Vec::new(),
    }
}

fn folder_record(folder: &Folder) -> Record {
    Record {
        id: folder.id.to_string(),
        collection: folder.collection.to_string(),
        name: folder.name.clone(),
        category: None,
        setting: None,
        children: Some(folder.children.iter().map(ItemId::as_str).collect::<Vec<_>>().join(",")),
        created_at: folder.created_at.to_rfc3339(),
        updated_at: folder.updated_at.to_rfc3339(),
    }
}

fn api_record(api: &ApiSetting) -> Record {
    Record {
        id: api.id.to_string(),
        collection: api.collection.to_string(),
        name: api.name.clone(),
        category: Some("http".into()),
        setting: Some(api.request.to_json()),
        children: None,
        created_at: api.created_at.to_rfc3339(),
        updated_at: api.updated_at.to_rfc3339(),
    }
}

fn pretty(records: &impl Serialize) -> String {
    // Plain records serialize.
    crate::format::json(records)
}

/// F3.9: one API as its record, for the clipboard; importing the text adds
/// the API back.
pub fn api_config(api: &ApiSetting) -> String {
    pretty(&api_record(api))
}

/// F11.1 / F11.2: the whole project, or one folder with everything under it
/// — its folders too, so the hierarchy comes back on import (#6). Folders
/// first, then APIs, indented by four spaces as before.
pub fn export(data: &TreeData, folder: Option<&ItemId>) -> String {
    let folders: Vec<&Folder> = match folder {
        None => data.folders.iter().collect(),
        Some(root) => {
            let mut inside: Vec<&ItemId> = vec![root];
            let mut found = Vec::new();
            while let Some(id) = inside.pop() {
                if let Some(folder) = data.folders.iter().find(|folder| &folder.id == id)
                    && !found.iter().any(|seen: &&Folder| seen.id == folder.id)
                {
                    found.push(folder);
                    inside.extend(folder.children.iter());
                }
            }
            // In store order, like the whole project.
            data.folders
                .iter()
                .filter(|folder| found.iter().any(|seen| seen.id == folder.id))
                .collect()
        }
    };
    let apis: Vec<&ApiSetting> = match folder {
        None => data.apis.iter().collect(),
        Some(_) => data
            .apis
            .iter()
            .filter(|api| folders.iter().any(|folder| folder.children.contains(&api.id)))
            .collect(),
    };
    let records: Vec<Record> = folders
        .into_iter()
        .map(folder_record)
        .chain(apis.into_iter().map(api_record))
        .collect();
    pretty(&records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HttpMethod;
    use crate::{CollectionId, Timestamp};
    use pretty_assertions::assert_eq;

    fn folder(id: &str, name: &str, children: &[&str]) -> Folder {
        Folder {
            id: ItemId::from(id),
            collection: CollectionId::from("c"),
            name: name.into(),
            children: children.iter().map(|id| ItemId::from(*id)).collect(),
            created_at: Timestamp::parse_lenient("2024-01-01T00:00:00Z"),
            updated_at: Timestamp::parse_lenient("2024-01-01T00:00:00Z"),
        }
    }

    fn api(id: &str, name: &str, uri: &str) -> ApiSetting {
        ApiSetting {
            id: ItemId::from(id),
            collection: CollectionId::from("c"),
            name: name.into(),
            request: HttpRequest {
                method: HttpMethod::Post,
                uri: uri.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn sample() -> TreeData {
        TreeData {
            folders: vec![folder("f1", "users", &["a1", "f2"]), folder("f2", "admin", &["a2"])],
            apis: vec![
                api("a1", "login", "/login"),
                api("a2", "ban", "/ban"),
                api("a3", "health", "/health"),
            ],
            top_order: vec![],
        }
    }

    fn names(items: &[ImportItem]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                ImportItem::Api { name, .. } => name.clone(),
                ImportItem::Folder { name, children } => format!("{name}[{}]", names(children).join(",")),
            })
            .collect()
    }

    #[test]
    fn an_export_reads_back_as_the_same_tree() {
        let text = export(&sample(), None);
        assert!(text.contains("\n    {\n        \"id\": \"f1\""), "four spaces: {text}");
        let bundle = parse_own(&text).expect("JSON");
        assert_eq!(names(&bundle.items), ["users[login,admin[ban]]", "health"]);
        let ImportItem::Folder { children, .. } = &bundle.items[0] else {
            panic!("a folder first");
        };
        let ImportItem::Api { request, .. } = &children[0] else {
            panic!("an API first");
        };
        assert_eq!((request.method, request.uri.as_str()), (HttpMethod::Post, "/login"));
    }

    #[test]
    fn a_folder_exports_with_its_folders() {
        let bundle = parse_own(&export(&sample(), Some(&ItemId::from("f1")))).expect("JSON");
        assert_eq!(names(&bundle.items), ["users[login,admin[ban]]"]);
        let inner = parse_own(&export(&sample(), Some(&ItemId::from("f2")))).expect("JSON");
        assert_eq!(names(&inner.items), ["admin[ban]"]);
    }

    #[test]
    fn an_apis_config_reads_back_as_the_api() {
        let text = api_config(&api("a1", "login", "/login"));
        let bundle = parse_own(&text).expect("JSON");
        assert_eq!(names(&bundle.items), ["login"]);
        let ImportItem::Api { request, .. } = &bundle.items[0] else {
            panic!("an API");
        };
        assert_eq!((request.method, request.uri.as_str()), (HttpMethod::Post, "/login"));
    }

    #[test]
    fn the_old_apps_records_and_loops_read() {
        let text = r#"[
            {"id": "x", "name": "loop", "children": "y"},
            {"id": "y", "name": "back", "children": "x,z"},
            {"id": "z", "category": "http", "name": "inner", "setting": "{\"method\":\"PUT\"}"},
            {"id": "solo", "category": "http", "name": "solo", "setting": "not json"}
        ]"#;
        let bundle = parse_own(text).expect("JSON");
        // x and y list each other: neither is on top, so only solo is.
        assert_eq!(names(&bundle.items), ["solo"]);
        let single = parse_own(r#"{"id": "z", "category": "http", "name": "one", "setting": "{}"}"#).expect("JSON");
        assert_eq!(names(&single.items), ["one"]);
    }
}
