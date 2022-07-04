use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::vec;

use super::database::{add_or_update_record, get_conn, list_records, NewFromRow};

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct APIFolder {
    // id(如果为_表示根目录，不展示)
    pub id: String,
    // 子目录ID或API ID，以,分割
    pub children: String,
    // 目录名称
    pub name: String,
    // 创建时间
    pub created_at: String,
    // 更新时间
    pub updated_at: String,
}

impl APIFolder {
    fn keys() -> Vec<String> {
        return vec![
            "id".to_string(),
            "children".to_string(),
            "name".to_string(),
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
        return vec![self.id.clone(), self.children.clone(), self.name.clone(), created_at, updated_at];
    }
}

impl NewFromRow<APIFolder> for APIFolder {
    fn from_row(data: &rusqlite::Row) -> Result<APIFolder, rusqlite::Error> {
        Ok(APIFolder {
            id: data.get(0)?,
            children: data.get(1)?,
            name: data.get(2)?,
            created_at: data.get(3)?,
            updated_at: data.get(4)?,
        })
    }
}

static TABLE_NAME: &str = "api_folder";

fn create_api_folders_if_not_exist() -> Result<usize, rusqlite::Error> {
    let conn = get_conn();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS  {} (
            id TEXT PRIMARY KEY NOT NULL check (id != ''),
            children TEXT DEFAULT '',
            name TEXT DEFAULT '',
            created_at TEXT DEFAULT '',
            updated_at TEXT DEFAULT ''
        )",
        TABLE_NAME
    );
    conn.execute(&sql, [])
}

pub fn add_or_update_api_folder(folder: APIFolder) -> Result<usize, rusqlite::Error> {
    create_api_folders_if_not_exist()?;

    add_or_update_record(TABLE_NAME, APIFolder::keys(), folder.values())
}

pub fn list_api_folder() -> Result<Vec<APIFolder>, rusqlite::Error> {
    create_api_folders_if_not_exist()?;
    list_records::<APIFolder>(TABLE_NAME, APIFolder::keys())
}
