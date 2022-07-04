use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::vec;

use super::database::{add_or_update_record, get_conn, list_records, NewFromRow};

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct APISetting {
    // id
    pub id: String,
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
        return vec![
            "id".to_string(),
            "name".to_string(),
            "category".to_string(),
            "setting".to_string(),
            "created_at".to_string(),
            "updated_at".to_string(),
        ];
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
        return vec![
            self.id.clone(),
            self.name.clone(),
            self.category.clone(),
            self.setting.clone(),
            created_at,
            updated_at,
        ];
    }
}
impl NewFromRow<APISetting> for APISetting {
    fn from_row(data: &rusqlite::Row) -> Result<APISetting, rusqlite::Error> {
        Ok(APISetting {
            id: data.get(0)?,
            name: data.get(1)?,
            category: data.get(2)?,
            setting: data.get(3)?,
            created_at: data.get(4)?,
            updated_at: data.get(5)?,
        })
    }
}

static TABLE_NAME: &str = "api_settings";

fn create_api_settings_if_not_exist() -> Result<usize, rusqlite::Error> {
    let conn = get_conn();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS  {} (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        name TEXT DEFAULT '',
        folder TEXT DEFAULT '',
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

pub fn list_api_setting() -> Result<Vec<APISetting>, rusqlite::Error> {
    create_api_settings_if_not_exist()?;
    list_records::<APISetting>(TABLE_NAME, APISetting::keys())
}
