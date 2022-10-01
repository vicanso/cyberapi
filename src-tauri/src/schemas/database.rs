// use tauri::{Env};
use once_cell::sync::OnceCell;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbErr, Statement};
use std::fs::OpenOptions;
use std::{fs, path::Path};

use crate::util;

use super::api_collection::get_api_collections_create_sql;
use super::api_folder::get_api_folders_create_sql;
use super::api_setting::get_api_settings_create_sql;
use super::variable::get_variables_create_sql;
use super::version::get_versions_table_create_sql;

static CREATE_DB_FILE: OnceCell<bool> = OnceCell::new();

pub async fn get_database() -> Result<DatabaseConnection, DbErr> {
    let dir = Path::new(util::get_app_dir());
    let file = dir.join("my_db.db");
    CREATE_DB_FILE.get_or_init(|| {
        fs::create_dir_all(dir).unwrap();
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(file.clone())
            .unwrap();
        true
    });

    let opt = format!("sqlite://{}", file.into_os_string().into_string().unwrap());
    Database::connect(opt).await
}

pub async fn init_tables() -> Result<(), DbErr> {
    let db = get_database().await?;
    let init_sql_list = vec![
        get_versions_table_create_sql(),
        get_api_collections_create_sql(),
        get_api_folders_create_sql(),
        get_api_settings_create_sql(),
        get_variables_create_sql(),
    ];
    for sql in init_sql_list {
        db.execute(Statement::from_string(db.get_database_backend(), sql))
            .await?;
    }
    Ok(())
}
