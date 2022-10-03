use crate::entities::{prelude::*, versions};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, DbErr, EntityTrait, QueryOrder, Set};
use serde::{Deserialize, Serialize};

use super::database::get_database;

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    // id
    pub id: String,
    // 版本号
    pub version: String,
    // 创建时间
    pub created_at: Option<String>,
    // 更新时间
    pub updated_at: Option<String>,
}

impl From<versions::Model> for Version {
    fn from(model: versions::Model) -> Self {
        Version {
            id: model.id,
            version: model.version,
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}

pub fn get_versions_table_create_sql() -> String {
    "CREATE TABLE IF NOT EXISTS versions (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        version TEXT NOT NULL check (version != ''),
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )"
    .to_string()
}

pub async fn add_version(version: Version) -> Result<Version, DbErr> {
    let created_at = version.created_at.or_else(|| Some(Utc::now().to_rfc3339()));
    let updated_at = version.updated_at.or_else(|| Some(Utc::now().to_rfc3339()));
    let model = versions::ActiveModel {
        id: Set(version.id),
        version: Set(version.version),
        created_at: Set(created_at),
        updated_at: Set(updated_at),
    };
    let db = get_database().await;
    let result = model.insert(&db).await?;
    Ok(result.into())
}

pub async fn get_latest_version() -> Result<Version, DbErr> {
    let db = get_database().await;
    let result = Versions::find()
        .order_by_desc(versions::Column::CreatedAt)
        .one(&db)
        .await?;

    match result {
        Some(data) => Ok(Version::from(data)),
        None => Ok(Version {
            id: "".to_string(),
            version: "".to_string(),
            created_at: Some("".to_string()),
            updated_at: Some("".to_string()),
        }),
    }
}
