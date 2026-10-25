//! Rows to records.

use crate::db;
use cyberapi_core::model::{
    ApiCategory, ApiSetting, Collection, Environment, Folder, HttpRequest, Variable, VariableCategory,
};
use cyberapi_core::{CollectionId, EnvironmentId, ItemId, TeamId, Timestamp, VariableId};
use cyberapi_store::StoreError;
use sqlx::Row;
use sqlx::any::AnyRow;

pub fn text(row: &AnyRow, index: usize) -> Result<String, StoreError> {
    row.try_get::<String, _>(index).map_err(db)
}

fn time(row: &AnyRow, index: usize) -> Result<Timestamp, StoreError> {
    Ok(Timestamp::parse_lenient(&text(row, index)?))
}

/// `id, name, description, created_at, updated_at, owner_team, revision`
/// from `start`. The role is the reader's, worked out apart from the row.
pub fn collection(row: &AnyRow, start: usize) -> Result<Collection, StoreError> {
    Ok(Collection {
        id: CollectionId::new(text(row, start)?),
        name: text(row, start + 1)?,
        description: text(row, start + 2)?,
        created_at: time(row, start + 3)?,
        updated_at: time(row, start + 4)?,
        owner: Some(TeamId::new(text(row, start + 5)?)),
        revision: u64::try_from(row.try_get::<i64, _>(start + 6).map_err(db)?).unwrap_or_default(),
        role: Default::default(),
    })
}

/// A folder's `children` is a JSON array of ids.
pub fn folder(row: &AnyRow) -> Result<Folder, StoreError> {
    Ok(Folder {
        id: ItemId::new(text(row, 0)?),
        collection: CollectionId::new(text(row, 1)?),
        children: serde_json::from_str(&text(row, 2)?).unwrap_or_default(),
        name: text(row, 3)?,
        // Kept apart (#66): the tree reads them.
        settings: Default::default(),
        created_at: time(row, 4)?,
        updated_at: time(row, 5)?,
    })
}

pub fn api(row: &AnyRow) -> Result<ApiSetting, StoreError> {
    Ok(ApiSetting {
        id: ItemId::new(text(row, 0)?),
        collection: CollectionId::new(text(row, 1)?),
        name: text(row, 2)?,
        category: ApiCategory::Http,
        request: HttpRequest::from_json_lenient(&text(row, 4)?),
        created_at: time(row, 5)?,
        updated_at: time(row, 6)?,
    })
}

/// `None` for a category this app does not know.
pub fn variable(row: &AnyRow) -> Result<Option<Variable>, StoreError> {
    let Some(category) = VariableCategory::parse(&text(row, 2)?) else {
        return Ok(None);
    };
    Ok(Some(Variable {
        id: VariableId::new(text(row, 0)?),
        collection: CollectionId::new(text(row, 1)?),
        category,
        name: text(row, 3)?,
        value: text(row, 4)?,
        enabled: row.try_get::<i32, _>(5).map_err(db)? != 0,
        // Kept apart (#61): the store reads which.
        environment: None,
        created_at: time(row, 6)?,
        updated_at: time(row, 7)?,
    }))
}

pub fn environment(row: &AnyRow) -> Result<Environment, StoreError> {
    Ok(Environment {
        id: EnvironmentId::new(text(row, 0)?),
        collection: CollectionId::new(text(row, 1)?),
        name: text(row, 2)?,
        created_at: time(row, 3)?,
        updated_at: time(row, 4)?,
    })
}
