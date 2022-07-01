use std::vec;

use chrono::Utc;
use rusqlite::params_from_iter;
use serde::{Deserialize, Serialize};

use super::database::get_conn;

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct APISetting {
    // id
    pub id: String,
    // 配置名称
    pub name: String,
    // 目录ID
    pub path: String,
    // 类型(http, graphQL)
    pub category: String,
    // 配置信息
    pub setting: String,
    // 创建时间
    pub created_at: String,
    // 更新时间
    pub updated_at: String,
}
impl Default for APISetting {
    fn default() -> APISetting {
        APISetting {
            id: "".to_string(),
            name: "".to_string(),
            path: "".to_string(),
            category: "".to_string(),
            setting: "".to_string(),
            created_at:Utc::now().to_rfc3339(),    
            updated_at:Utc::now().to_rfc3339(),    
        }
    }
}

impl APISetting {
    fn keys() -> Vec<String> {
        return vec![
            "id".to_string(),
            "name".to_string(),
            "path".to_string(),
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
            self.path.clone(),
            self.category.clone(),
            self.setting.clone(),
            created_at,
            updated_at,
        ];
    }
    fn new(data: &rusqlite::Row) -> Result<APISetting, rusqlite::Error> {
        Ok(APISetting {
            id: data.get(0)?,
            name: data.get(1)?,
            path: data.get(2)?,
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
        name TEXT DEFAULT '',
        path TEXT DEFAULT '',
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
    let conn = get_conn();

    let p = params_from_iter(setting.values());

    let keys = APISetting::keys();
    let mut values = Vec::new();
    for n in 0..keys.len() {
        values.push(format!("?{}", n + 1));
    }
    let sql = format!(
        "INSERT OR REPLACE INTO {} ({}) VALUES ({})",
        TABLE_NAME,
        keys.join(", "),
        values.join(", "),
    );
    conn.execute(&sql, p)
}

pub fn list_api_setting() -> Result<Vec<APISetting>, rusqlite::Error> {
    create_api_settings_if_not_exist()?;
    let conn = get_conn();

    let keys = APISetting::keys();
    let sql = format!("SELECT {} FROM {}", keys.join(", "), TABLE_NAME);
    let mut statement = conn.prepare(&sql)?;
    let mut rows = statement.query([])?;

    let mut result = Vec::new();
    let mut done = false;
    while !done {
        let item = rows.next()?;
        match item {
            Some(data) => result.push(APISetting::new(data)?),
            None => done = true,
        }
    }
    Ok(result)
}
