use crate::error::CyberAPIError;
use chrono::Local;
use once_cell::sync::OnceCell;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbErr, Statement};
use std::fs::OpenOptions;
use std::io::Write;
use std::vec;
use std::{fs, fs::File, path::Path};
use tauri::api::path::download_dir;
use zip::write::FileOptions;

use crate::util;

use super::api_collection::{export_api_collection, get_api_collections_create_sql};
use super::api_folder::{export_api_folder, get_api_folders_create_sql};
use super::api_setting::{export_api_setting, get_api_settings_create_sql};
use super::variable::{export_variable, get_variables_create_sql};
use super::version::get_versions_table_create_sql;

static CREATE_DB_FILE: OnceCell<bool> = OnceCell::new();

pub struct ExportData {
    pub name: String,
    pub data: Vec<serde_json::Value>,
}

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

pub async fn export_tables() -> Result<String, CyberAPIError> {
    let download = download_dir().unwrap();

    let local = Local::now();

    let filename = format!("cyberapi-backup-{}.zip", local.format("%Y-%m-%d"));

    let file = File::create(&download.join(filename.clone()))?;
    let mut w = zip::ZipWriter::new(file);

    let table_data_list = vec![
        export_api_collection().await?,
        export_api_folder().await?,
        export_api_setting().await?,
        export_variable().await?,
    ];
    let options = FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);

    for table_data in table_data_list {
        let mut json = vec![];
        for ele in table_data.data {
            let str = serde_json::to_string(&ele)?;
            json.push(str);
        }
        let file_name = table_data.name + ".json";
        w.start_file(file_name, options)?;

        let file_data = format!("[{}]", json.join(","));
        w.write_all(file_data.as_bytes())?;
    }
    w.finish()?;

    Ok(filename)
}
