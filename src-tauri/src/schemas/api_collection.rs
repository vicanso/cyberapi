use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::database::{add_or_update_record, get_conn, list_records, NewFromRow};

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct APICollection {
    // collection id
    pub id: String,
    // collection 名称
    pub name: String,
    // 描述
    pub description: String,
    // 创建时间
    pub created_at: String,
    // 更新时间
    pub updated_at: String,
}

impl APICollection {
    fn keys() -> Vec<String> {
        vec![
            "id".to_string(),
            "name".to_string(),
            "description".to_string(),
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
        vec![self.id.clone(), self.name.clone(), self.description.clone(), created_at, updated_at]
    }
}

impl NewFromRow<APICollection> for APICollection {
    fn from_row(data: &rusqlite::Row) -> Result<APICollection, rusqlite::Error> {
        Ok(APICollection {
            id: data.get(0)?,
            name: data.get(1)?,
            description: data.get(2)?,
            created_at: data.get(3)?,
            updated_at: data.get(4)?,
        })
    }
}

static TABLE_NAME: &str = "api_collections";

fn create_api_collections_if_not_exist() -> Result<usize, rusqlite::Error> {
    let conn = get_conn();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS  {} (
            id TEXT PRIMARY KEY NOT NULL check (id != ''),
            name TEXT DEFAULT '',
            description TEXT DEFAULT '',
            created_at TEXT DEFAULT '',
            updated_at TEXT DEFAULT ''
        )",
        TABLE_NAME
    );
    conn.execute(&sql, [])
}

pub fn add_or_update_api_collection(collection: APICollection) -> Result<usize, rusqlite::Error> {
    create_api_collections_if_not_exist()?;
    add_or_update_record(TABLE_NAME, APICollection::keys(), collection.values())
}

pub fn list_api_collection() -> Result<Vec<APICollection>, rusqlite::Error> {
    create_api_collections_if_not_exist()?;
    list_records::<APICollection>(TABLE_NAME, APICollection::keys())
}
