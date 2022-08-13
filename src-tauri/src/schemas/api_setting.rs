use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::vec;

use super::database::{
    add_or_update_record, delete_by_ids, get_conn, list_condition_records, NewFromRow,
};

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct APISetting {
    // id
    pub id: String,
    // collection ID
    pub collection: String,
    // 配置名称
    pub name: String,
    // 类型(http, graphQL)
    pub category: String,
    // 配置信息
    pub setting: String,
    // 创建时间
    pub created_at: String,
    // 更新时间
    pub updated_at: String,
}

impl APISetting {
    fn keys() -> Vec<String> {
        vec![
            "id".to_string(),
            "collection".to_string(),
            "name".to_string(),
            "category".to_string(),
            "setting".to_string(),
            "created_at".to_string(),
            "updated_at".to_string(),
        ]
    }
    fn values(&self) -> Vec<String> {
        let mut created_at = self.created_at.clone();
        if created_at.is_empty() {
            created_at = Utc::now().to_rfc3339();
        }
        let mut updated_at = self.updated_at.clone();
        if updated_at.is_empty() {
            updated_at = Utc::now().to_rfc3339();
        }
        vec![
            self.id.clone(),
            self.collection.clone(),
            self.name.clone(),
            self.category.clone(),
            self.setting.clone(),
            created_at,
            updated_at,
        ]
    }
}
impl NewFromRow<APISetting> for APISetting {
    fn from_row(data: &rusqlite::Row) -> Result<APISetting, rusqlite::Error> {
        Ok(APISetting {
            id: data.get(0)?,
            collection: data.get(1)?,
            name: data.get(2)?,
            category: data.get(3)?,
            setting: data.get(4)?,
            created_at: data.get(5)?,
            updated_at: data.get(6)?,
        })
    }
}

static TABLE_NAME: &str = "api_settings";

fn create_api_settings_if_not_exist() -> Result<usize, rusqlite::Error> {
    let conn = get_conn();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS  {} (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        collection TEXT NOT NULL check (collection != ''),
        name TEXT DEFAULT '',
        category TEXT DEFAULT '',
        setting TEXT DEFAULT '',
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )",
        TABLE_NAME
    );
    conn.execute(&sql, [])
}

pub fn add_or_update_api_setting(setting: APISetting) -> Result<usize, rusqlite::Error> {
    create_api_settings_if_not_exist()?;
    add_or_update_record(TABLE_NAME, APISetting::keys(), setting.values())
}

pub fn list_api_setting(collection: String) -> Result<Vec<APISetting>, rusqlite::Error> {
    // 有可能未有table，先创建
    create_api_settings_if_not_exist()?;
    let sql = format!(
        "SELECT {} FROM {} WHERE collection = ?1",
        APISetting::keys().join(", "),
        TABLE_NAME
    );
    list_condition_records::<APISetting>(&sql, vec![collection])
}

pub fn delete_api_setting_by_collection(collection: String) -> Result<usize, rusqlite::Error> {
    // 有可能未有table，先创建
    create_api_settings_if_not_exist()?;
    let sql = format!("DELETE FROM {} WHERE collection = ?1", TABLE_NAME);
    get_conn().execute(&sql, params![collection])
}

pub fn delete_api_settings(ids: Vec<String>) -> Result<usize, rusqlite::Error> {
    delete_by_ids(TABLE_NAME, ids)
}
