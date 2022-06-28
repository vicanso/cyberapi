use rusqlite::params;
use serde::{Deserialize, Serialize};
use chrono::{Utc};


use super::database;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct APISetting {
    // id
    pub id: String,
    // 配置名称
    pub name: String,
    // 目录路径
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


fn create_if_not_exist() -> Result<usize, rusqlite::Error> {
    let conn = database::get_conn();
    let sql = "CREATE TABLE IF NOT EXISTS  api_settings (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        path TEXT,
        name TEXT,
        category TEXT,
        setting TEXT,
        created_at TEXT,
        updated_at TEXT
    )";
    conn.execute(sql, [])
}

pub fn api_setting_add(id: String) -> Result<usize, rusqlite::Error> {
    create_if_not_exist()?;
    let conn = database::get_conn();
    conn.execute(
        "INSERT INTO api_settings (id) VALUES (?1, ?2)",
        params![
            id,
            Utc::now().to_rfc3339(),
        ],
    )
}
