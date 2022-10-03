use crate::{
    entities::{api_folders, prelude::*},
    error::CyberAPIError,
};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, DbErr, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::database::{get_database, ExportData};

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct APIFolder {
    // 目录ID
    pub id: String,
    // collection ID
    pub collection: String,
    // 子目录ID或API ID，以,分割
    pub children: Option<String>,
    // 目录名称
    pub name: Option<String>,
    // 创建时间
    pub created_at: Option<String>,
    // 更新时间
    pub updated_at: Option<String>,
}

impl From<api_folders::Model> for APIFolder {
    fn from(model: api_folders::Model) -> Self {
        APIFolder {
            id: model.id,
            collection: model.collection,
            children: model.children,
            name: model.name,
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}
impl APIFolder {
    fn into_active_model(self) -> api_folders::ActiveModel {
        let created_at = self.created_at.or_else(|| Some(Utc::now().to_rfc3339()));
        let updated_at = self.updated_at.or_else(|| Some(Utc::now().to_rfc3339()));
        api_folders::ActiveModel {
            id: Set(self.id),
            collection: Set(self.collection),
            children: Set(self.children),
            name: Set(self.name),
            created_at: Set(created_at),
            updated_at: Set(updated_at),
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct APIFolderChildren {
    pub folders: Vec<String>,
    pub settings: Vec<String>,
}

pub fn get_api_folders_create_sql() -> String {
    "CREATE TABLE IF NOT EXISTS api_folders (
            id TEXT PRIMARY KEY NOT NULL check (id != ''),
            collection TEXT NOT NULL check (collection != ''),
            children TEXT DEFAULT '',
            name TEXT DEFAULT '',
            created_at TEXT DEFAULT '',
            updated_at TEXT DEFAULT ''
        )"
    .to_string()
}

pub async fn add_api_folder(folder: APIFolder) -> Result<APIFolder, DbErr> {
    let model = folder.into_active_model();
    let db = get_database().await;
    let result = model.insert(&db).await?;
    Ok(result.into())
}

pub async fn update_api_folder(folder: APIFolder) -> Result<APIFolder, DbErr> {
    let model = folder.into_active_model();
    let db = get_database().await;
    let result = model.update(&db).await?;
    Ok(result.into())
}

pub async fn list_api_folder(collection: String) -> Result<Vec<APIFolder>, DbErr> {
    let db = get_database().await;
    let result = ApiFolders::find()
        .filter(api_folders::Column::Collection.eq(collection))
        .all(&db)
        .await?;
    Ok(result.into_iter().map(APIFolder::from).collect())
}

pub async fn delete_api_folder_by_collection(collection: String) -> Result<u64, DbErr> {
    let db = get_database().await;
    let result = ApiFolders::delete_many()
        .filter(api_folders::Column::Collection.eq(collection))
        .exec(&db)
        .await?;

    Ok(result.rows_affected)
}

pub async fn delete_api_folders(ids: Vec<String>) -> Result<u64, DbErr> {
    let db = get_database().await;

    let result = ApiFolders::delete_many()
        .filter(api_folders::Column::Id.is_in(ids))
        .exec(&db)
        .await?;

    Ok(result.rows_affected)
}

// 获取该目录的所有子元素（包括子元素以及子目录、子目录的子元素）
pub async fn list_api_folder_all_children(id: String) -> Result<APIFolderChildren, DbErr> {
    // 使用偷懒的方式，直接查询所有api folder再过滤
    let mut folder_children = HashMap::new();
    let mut folders = Vec::new();
    let mut settings = Vec::new();
    let mut children = "".to_string();

    let db = get_database().await;
    let current_folder = ApiFolders::find()
        .filter(api_folders::Column::Id.eq(id.clone()))
        .one(&db)
        .await?;
    if let Some(folder) = current_folder {
        // 记录所有folder与它的子目录
        for ele in list_api_folder(folder.collection).await? {
            if ele.id == id {
                if let Some(data) = ele.children.clone() {
                    children = data
                }
            }
            folder_children.insert(ele.id, ele.children.clone());
        }
    }
    while !children.is_empty() {
        let arr = children.split(',');
        let mut current_children = Vec::new();

        for ele in arr {
            let id = ele.trim();
            // 是folder
            match folder_children.get(id) {
                // 目录
                Some(str) => {
                    folders.push(id.to_string());
                    if let Some(value) = str {
                        // 记录子元素
                        if !value.is_empty() {
                            current_children.push(value.as_str());
                        }
                    }
                }
                None => {
                    // api settings
                    settings.push(id.to_string())
                }
            }
        }
        // 记录新的children
        children = current_children.join(",")
    }
    Ok(APIFolderChildren { folders, settings })
}

pub fn get_table_name_api_folder() -> String {
    "api_folders".to_string()
}

pub async fn delete_all_api_folder() -> Result<(), CyberAPIError> {
    let db = get_database().await;
    ApiFolders::delete_many().exec(&db).await?;
    Ok(())
}

pub async fn export_api_folder() -> Result<ExportData, DbErr> {
    let db = get_database().await;
    let data = ApiFolders::find().into_json().all(&db).await?;
    Ok(ExportData {
        name: get_table_name_api_folder(),
        data,
    })
}

pub async fn import_api_folder(data: Vec<serde_json::Value>) -> Result<(), CyberAPIError> {
    let db = get_database().await;

    let mut records = Vec::new();
    for ele in data {
        let model = api_folders::ActiveModel::from_json(ele)?;
        records.push(model);
    }
    ApiFolders::insert_many(records).exec(&db).await?;
    Ok(())
}
