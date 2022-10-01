use crate::entities::{api_settings, prelude::*};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, DbErr, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};

use super::database::get_database;
#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct APISetting {
    // id
    pub id: String,
    // collection ID
    pub collection: String,
    // 配置名称
    pub name: Option<String>,
    // 类型(http, graphQL)
    pub category: Option<String>,
    // 配置信息
    pub setting: Option<String>,
    // 创建时间
    pub created_at: Option<String>,
    // 更新时间
    pub updated_at: Option<String>,
}

impl From<api_settings::Model> for APISetting {
    fn from(model: api_settings::Model) -> Self {
        APISetting {
            id: model.id,
            collection: model.collection,
            name: model.name,
            category: model.category,
            setting: model.setting,
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}
impl APISetting {
    fn into_active_model(self) -> api_settings::ActiveModel {
        let created_at = self.created_at.or_else(|| Some(Utc::now().to_rfc3339()));
        let updated_at = self.updated_at.or_else(|| Some(Utc::now().to_rfc3339()));
        api_settings::ActiveModel {
            id: Set(self.id),
            collection: Set(self.collection),
            name: Set(self.name),
            category: Set(self.category),
            setting: Set(self.setting),
            created_at: Set(created_at),
            updated_at: Set(updated_at),
        }
    }
}

pub fn get_api_settings_create_sql() -> String {
    "CREATE TABLE IF NOT EXISTS api_settings (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        collection TEXT NOT NULL check (collection != ''),
        name TEXT DEFAULT '',
        category TEXT DEFAULT '',
        setting TEXT DEFAULT '',
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )"
    .to_string()
}

pub async fn add_api_setting(setting: APISetting) -> Result<APISetting, DbErr> {
    let model = setting.into_active_model();
    let db = get_database().await?;
    let result = model.insert(&db).await?;
    Ok(result.into())
}
pub async fn update_api_setting(setting: APISetting) -> Result<APISetting, DbErr> {
    let model = setting.into_active_model();
    let db = get_database().await?;
    let result = model.update(&db).await?;
    Ok(result.into())
}

pub async fn list_api_setting(collection: String) -> Result<Vec<APISetting>, DbErr> {
    let db = get_database().await?;
    let result = ApiSettings::find()
        .filter(api_settings::Column::Collection.eq(collection))
        .all(&db)
        .await?;

    Ok(result.into_iter().map(APISetting::from).collect())
}

pub async fn delete_api_setting_by_collection(collection: String) -> Result<u64, DbErr> {
    let db = get_database().await?;
    let result = ApiSettings::delete_many()
        .filter(api_settings::Column::Collection.eq(collection))
        .exec(&db)
        .await?;
    Ok(result.rows_affected)
}

pub async fn delete_api_settings(ids: Vec<String>) -> Result<u64, DbErr> {
    let db = get_database().await?;

    let result = ApiSettings::delete_many()
        .filter(api_settings::Column::Id.is_in(ids))
        .exec(&db)
        .await?;
    Ok(result.rows_affected)
}
