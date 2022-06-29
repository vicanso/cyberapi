use chrono::Utc;
use rusqlite::params_from_iter;
use serde::{Deserialize, Serialize};

use super::database::{get_conn};

#[derive(Deserialize, Serialize)]
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

pub fn add_api_setting(id: String) -> Result<usize, rusqlite::Error> {
    create_api_settings_if_not_exist()?;
    let conn = get_conn();

    let p = params_from_iter(vec![id, Utc::now().to_rfc3339(), Utc::now().to_rfc3339()]);
    let keys = vec!["id", "created_at", "updated_at"];

    let mut values = Vec::new();
    for n in 0..keys.len() {
        values.push(format!("?{}", n + 1));
    }
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        TABLE_NAME,
        keys.join(", "),
        values.join(", ")
    );
    conn.execute(&sql, p)
}

pub fn list_api_setting() -> Result<Vec<APISetting>, rusqlite::Error> {
    create_api_settings_if_not_exist()?;
    let conn = get_conn();

    let keys = vec![
        "id",
        "name",
        "path",
        "category",
        "setting",
        "created_at",
        "updated_at",
    ];
    let sql = format!("SELECT {} FROM {}", keys.join(", "), TABLE_NAME);
    let mut statement = conn.prepare(&sql)?;
    let mut rows = statement.query([])?;

    let mut result = Vec::new();
    let mut done = false;
    while !done {
        let item = rows.next()?;
        match item {
            Some(data) => result.push(APISetting {
                id: data.get(0)?,
                name: data.get(1)?,
                path: data.get(2)?,
                category: data.get(3)?,
                setting: data.get(4)?,
                created_at: data.get(5)?,
                updated_at: data.get(6)?,
            }),
            None => done = true,
        }
    }
    Ok(result)
}
