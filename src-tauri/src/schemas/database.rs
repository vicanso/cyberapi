use crate::error::CyberAPIError;
use chrono::Local;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbErr, Statement};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::time::Duration;
use std::vec;
use std::{fs, fs::File, path::Path};
use tauri::api::path::download_dir;
use tokio::sync::OnceCell;
use zip::write::FileOptions;

use crate::util;

use super::api_collection::{
    delete_all_api_collection, export_api_collection, get_api_collections_create_sql,
    get_table_name_api_collection, import_api_collection,
};
use super::api_folder::{
    delete_all_api_folder, export_api_folder, get_api_folders_create_sql,
    get_table_name_api_folder, import_api_folder,
};
use super::api_setting::{
    delete_all_api_setting, export_api_setting, get_api_settings_create_sql,
    get_table_name_api_setting, import_api_setting,
};
use super::variable::{
    delete_all_variable, export_variable, get_table_name_variable, get_variables_create_sql,
    import_variable,
};
use super::version::get_versions_table_create_sql;

static DB: OnceCell<DatabaseConnection> = OnceCell::const_new();

pub struct ExportData {
    pub name: String,
    pub data: Vec<serde_json::Value>,
}

async fn get_conn() -> DatabaseConnection {
    let dir = Path::new(util::get_app_dir());
    let file = dir.join("my_db.db");
    fs::create_dir_all(dir).unwrap();
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(file.clone())
        .unwrap();

    let conn_uri = format!("sqlite://{}", file.into_os_string().into_string().unwrap());

    let mut opt = ConnectOptions::new(conn_uri);
    opt.max_connections(10)
        .min_connections(2)
        .connect_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(60));

    let result = Database::connect(opt).await;
    result.unwrap()
}

pub async fn get_database() -> DatabaseConnection {
    let db = DB.get_or_init(get_conn).await;
    db.to_owned()
}

pub async fn init_tables() -> Result<(), DbErr> {
    let db = get_database().await;
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

pub async fn import_tables(filename: String) -> Result<(), CyberAPIError> {
    let mut r = zip::ZipArchive::new(File::open(filename)?)?;

    delete_all_api_collection().await?;
    delete_all_api_folder().await?;
    delete_all_api_setting().await?;
    delete_all_variable().await?;

    let names = vec![
        get_table_name_api_collection(),
        get_table_name_api_folder(),
        get_table_name_api_setting(),
        get_table_name_variable(),
    ];
    for i in 0..names.len() {
        let name = names.get(i).unwrap();
        let mut buf = Vec::new();
        {
            let mut file = r.by_name((name.to_owned() + ".json").as_str())?;
            file.read_to_end(&mut buf)?;
        }
        let data: Vec<serde_json::Value> = serde_json::from_slice(&buf)?;
        match i {
            0 => import_api_collection(data).await?,
            1 => import_api_folder(data).await?,
            2 => import_api_setting(data).await?,
            3 => import_variable(data).await?,
            _ => (),
        }
    }

    Ok(())
}
