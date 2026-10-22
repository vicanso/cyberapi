//! Rows to records. The old app wrote every column as text and left many
//! empty, so each read is lenient: a value that cannot be read is the field's
//! default, never an error that hides a whole project.

use cyberapi_core::model::{
    ApiCategory, ApiSetting, Collection, Folder, HttpRequest, Variable, VariableCategory, legacy,
};
use cyberapi_core::sync::CopyBinding;
use cyberapi_core::{AccountId, CollectionId, ItemId, Timestamp, VariableId};
use turso::{Row, Value};

/// A column as text: `NULL` is empty, numbers are spelled out.
pub fn text(row: &Row, index: usize) -> String {
    match row.get_value(index) {
        Ok(Value::Text(text)) => text,
        Ok(Value::Integer(number)) => number.to_string(),
        Ok(Value::Real(number)) => number.to_string(),
        Ok(Value::Blob(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        Ok(Value::Null) | Err(_) => String::new(),
    }
}

fn time(row: &Row, index: usize) -> Timestamp {
    Timestamp::parse_lenient(&text(row, index))
}

/// A count this app wrote: anything else is zero.
fn count(row: &Row, index: usize) -> u64 {
    match row.get_value(index) {
        Ok(Value::Integer(number)) => u64::try_from(number).unwrap_or_default(),
        _ => 0,
    }
}

/// A project and how many APIs it holds.
pub fn api_count(row: &Row) -> (CollectionId, usize) {
    (
        CollectionId::new(text(row, 0)),
        usize::try_from(count(row, 1)).unwrap_or_default(),
    )
}

pub const COLLECTION_COLUMNS: &str = "id, name, description, created_at, updated_at, revision";

pub fn collection(row: &Row) -> Collection {
    Collection {
        id: CollectionId::new(text(row, 0)),
        name: text(row, 1),
        description: text(row, 2),
        created_at: time(row, 3),
        updated_at: time(row, 4),
        revision: count(row, 5),
        // No teams on the desktop, and its one reader manages everything.
        owner: None,
        role: Default::default(),
    }
}

/// `collection, account, base, unsynced` of `server_copies`.
pub fn copy_binding(row: &Row) -> CopyBinding {
    CopyBinding {
        collection: CollectionId::new(text(row, 0)),
        account: AccountId::new(text(row, 1)),
        base: match row.get_value(2) {
            Ok(Value::Integer(base)) => u64::try_from(base).ok(),
            _ => None,
        },
        unsynced: count(row, 3) != 0,
    }
}

pub const FOLDER_COLUMNS: &str = "id, collection, children, name, created_at, updated_at";

pub fn folder(row: &Row) -> Folder {
    Folder {
        id: ItemId::new(text(row, 0)),
        collection: CollectionId::new(text(row, 1)),
        children: legacy::parse_id_list(&text(row, 2)),
        name: text(row, 3),
        created_at: time(row, 4),
        updated_at: time(row, 5),
    }
}

pub const API_COLUMNS: &str = "id, collection, name, category, setting, created_at, updated_at";

/// `category` has only ever been `"http"`.
pub fn api(row: &Row) -> ApiSetting {
    ApiSetting {
        id: ItemId::new(text(row, 0)),
        collection: CollectionId::new(text(row, 1)),
        name: text(row, 2),
        category: ApiCategory::Http,
        request: HttpRequest::from_json_lenient(&text(row, 4)),
        created_at: time(row, 5),
        updated_at: time(row, 6),
    }
}

pub const VARIABLE_COLUMNS: &str = "id, collection, category, name, value, enabled, created_at, updated_at";

/// `None` for a category this app does not know, which no version wrote.
pub fn variable(row: &Row) -> Option<Variable> {
    Some(Variable {
        id: VariableId::new(text(row, 0)),
        collection: CollectionId::new(text(row, 1)),
        category: VariableCategory::parse(&text(row, 2))?,
        name: text(row, 3),
        value: text(row, 4),
        enabled: legacy::parse_enabled(&text(row, 5)),
        created_at: time(row, 6),
        updated_at: time(row, 7),
    })
}
