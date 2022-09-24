use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::vec;

use super::database::{
    add_or_update_record, delete_by_ids, get_conn, list_condition_records, NewFromRow,
};

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    // id
    pub id: String,
    // 分类
    pub category: String,
    // collection ID
    pub collection: String,
    // 变量名称
    pub name: String,
    // 变量值
    pub value: String,
    // 是否启用(0:禁用 1:启用)
    pub enabled: String,
    // 创建时间
    pub created_at: String,
    // 更新时间
    pub updated_at: String,
}

impl Variable {
    fn keys() -> Vec<String> {
        vec![
            "id".to_string(),
            "category".to_string(),
            "collection".to_string(),
            "name".to_string(),
            "value".to_string(),
            "enabled".to_string(),
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
            self.category.clone(),
            self.collection.clone(),
            self.name.clone(),
            self.value.clone(),
            self.enabled.clone(),
            created_at,
            updated_at,
        ]
    }
}

impl NewFromRow<Variable> for Variable {
    fn from_row(data: &rusqlite::Row) -> Result<Variable, rusqlite::Error> {
        Ok(Variable {
            id: data.get(0)?,
            category: data.get(1)?,
            collection: data.get(2)?,
            name: data.get(3)?,
            value: data.get(4)?,
            enabled: data.get(5)?,
            created_at: data.get(6)?,
            updated_at: data.get(7)?,
        })
    }
}

static TABLE_NAME: &str = "variables";

fn create_value_if_not_exist() -> Result<usize, rusqlite::Error> {
    let conn = get_conn();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS  {} (
            id TEXT PRIMARY KEY NOT NULL check (id != ''),
            category TEXT NOT NULL check (category != ''),
            collection TEXT NOT NULL check (collection != ''),
            name TEXT DEFAULT '',
            value TEXT DEFAULT '',
            enabled TEXT DEFAULT '',
            created_at TEXT DEFAULT '',
            updated_at TEXT DEFAULT ''
        )",
        TABLE_NAME
    );
    conn.execute(&sql, [])
}

pub fn add_or_update_variable(value: Variable) -> Result<usize, rusqlite::Error> {
    create_value_if_not_exist()?;
    add_or_update_record(TABLE_NAME, Variable::keys(), value.values())
}

pub fn list_variable(
    collection: String,
    category: String,
) -> Result<Vec<Variable>, rusqlite::Error> {
    create_value_if_not_exist()?;
    let sql = format!(
        "SELECT {} FROM {} WHERE collection = ?1 and category = ?2",
        Variable::keys().join(", "),
        TABLE_NAME
    );
    list_condition_records::<Variable>(&sql, vec![collection, category])
}

pub fn delete_variable(ids: Vec<String>) -> Result<usize, rusqlite::Error> {
    delete_by_ids(TABLE_NAME, ids)
}
