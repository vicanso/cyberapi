use crate::entities::{api_collections, prelude::*};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, DbErr, EntityTrait, Set};
use serde::{Deserialize, Serialize};

use super::database::{get_database, ExportData};

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct APICollection {
    // id
    pub id: String,
    // 名称
    pub name: Option<String>,
    // 描述
    pub description: Option<String>,
    // 创建时间
    pub created_at: Option<String>,
    // 更新时间
    pub updated_at: Option<String>,
}

impl From<api_collections::Model> for APICollection {
    fn from(model: api_collections::Model) -> Self {
        APICollection {
            id: model.id,
            name: model.name,
            description: model.description,
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}

impl APICollection {
    fn into_active_model(self) -> api_collections::ActiveModel {
        let created_at = self.created_at.or_else(|| Some(Utc::now().to_rfc3339()));
        let updated_at = self.updated_at.or_else(|| Some(Utc::now().to_rfc3339()));
        api_collections::ActiveModel {
            id: Set(self.id),
            name: Set(self.name),
            description: Set(self.description),
            created_at: Set(created_at),
            updated_at: Set(updated_at),
        }
    }
}

pub fn get_api_collections_create_sql() -> String {
    "CREATE TABLE IF NOT EXISTS api_collections (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        name TEXT DEFAULT '',
        description TEXT DEFAULT '',
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )"
    .to_string()
}

pub async fn add_api_collection(collection: APICollection) -> Result<APICollection, DbErr> {
    let model: api_collections::ActiveModel = collection.into_active_model();
    let db = get_database().await?;

    let result = model.insert(&db).await?;
    Ok(result.into())
}
pub async fn update_api_collection(collection: APICollection) -> Result<APICollection, DbErr> {
    let model: api_collections::ActiveModel = collection.into_active_model();
    let db = get_database().await?;

    let result = model.update(&db).await?;
    Ok(result.into())
}

pub async fn list_api_collection() -> Result<Vec<APICollection>, DbErr> {
    let db = get_database().await?;
    let result = ApiCollections::find().all(&db).await?;

    Ok(result.into_iter().map(APICollection::from).collect())
}

pub async fn delete_api_collection(id: String) -> Result<u64, DbErr> {
    let db = get_database().await?;
    let result = ApiCollections::delete_by_id(id).exec(&db).await?;
    Ok(result.rows_affected)
}

pub async fn export_api_collection() -> Result<ExportData, DbErr> {
    let db = get_database().await?;
    let data = ApiCollections::find().into_json().all(&db).await?;
    Ok(ExportData {
        name: "api_collections".to_string(),
        data,
    })
}
