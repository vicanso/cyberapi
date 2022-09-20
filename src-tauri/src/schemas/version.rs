use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::vec;

use super::database::{add_or_update_record, get_conn, NewFromRow};

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    // id
    pub id: String,
    // 版本号
    pub version: String,
    // 创建时间
    pub created_at: String,
    // 更新时间
    pub updated_at: String,
}

impl Version {
    fn keys() -> Vec<String> {
        vec![
            "id".to_string(),
            "version".to_string(),
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
            self.version.clone(),
            created_at,
            updated_at,
        ]
    }
}

impl NewFromRow<Version> for Version {
    fn from_row(data: &rusqlite::Row) -> Result<Version, rusqlite::Error> {
        Ok(Version {
            id: data.get(0)?,
            version: data.get(1)?,
            created_at: data.get(2)?,
            updated_at: data.get(3)?,
        })
    }
}

static TABLE_NAME: &str = "versions";

fn create_versions_if_not_exists() -> Result<usize, rusqlite::Error> {
    let conn = get_conn();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS  {} (
        id TEXT PRIMARY KEY NOT NULL check (id != ''),
        version TEXT NOT NULL check (version != ''),
        created_at TEXT DEFAULT '',
        updated_at TEXT DEFAULT ''
    )",
        TABLE_NAME
    );
    conn.execute(&sql, [])
}

pub fn add_version(version: Version) -> Result<usize, rusqlite::Error> {
    create_versions_if_not_exists()?;
    add_or_update_record(TABLE_NAME, Version::keys(), version.values())
}

pub fn get_latest_version() -> Result<Version, rusqlite::Error> {
    create_versions_if_not_exists()?;
    let sql = format!(
        "SELECT {} FROM {} ORDER BY created_at DESC limit 1",
        Version::keys().join(", "),
        TABLE_NAME
    );
    let conn = get_conn();
    let mut statement = conn.prepare(&sql)?;
    let mut rows = statement.query(params![])?;
    let item = rows.next()?;

    match item {
        Some(data) => Ok(Version::from_row(data)?),
        None => Ok(Version {
            id: "".to_string(),
            version: "".to_string(),
            created_at: "".to_string(),
            updated_at: "".to_string(),
        }),
    }
}
