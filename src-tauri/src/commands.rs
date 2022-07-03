use crate::schemas::{self, APIFolder, APISetting};

use tauri::Manager;
use tauri::{command, Window};

#[derive(Debug, Clone, serde::Serialize)]
pub struct CommandError {
    message: String,
    category: String,
}
#[derive(strum_macros::Display)]
enum CommandErrorCategory {
    Database,
}

pub type CommandResult<T> = Result<T, CommandError>;

impl Default for CommandError {
    fn default() -> CommandError {
        CommandError {
            message: "".to_string(),
            category: "".to_string(),
        }
    }
}

// 关闭启动视窗切换至主视窗
#[command]
pub fn close_splashscreen(window: Window) {
    // 关闭启动视图
    if let Some(splashscreen) = window.get_window("splashscreen") {
        splashscreen.close().unwrap();
    }
    // 展示主视图
    window.get_window("main").unwrap().show().unwrap();
}

fn convert_sql_error(error: rusqlite::Error) -> CommandError {
    CommandError {
        category: CommandErrorCategory::Database.to_string(),
        message: error.to_string(),
    }
}

// 新增API配置
#[command(async)]
pub fn add_api_setting(setting: APISetting) -> CommandResult<()> {
    match schemas::add_or_update_api_setting(setting) {
        Ok(_) => Ok(()),
        Err(error) => Err(convert_sql_error(error)),
    }
}

// 更新API配置
#[command(async)]
pub fn update_api_setting(setting: APISetting) -> CommandResult<()> {
    match schemas::add_or_update_api_setting(setting) {
        Ok(_) => Ok(()),
        Err(error) => Err(convert_sql_error(error)),
    }
}

// 获取所有API配置
#[command(async)]
pub fn list_api_setting() -> CommandResult<Vec<APISetting>> {
    match schemas::list_api_setting() {
        Ok(result) => Ok(result),
        Err(error) => Err(convert_sql_error(error)),
    }
}

// 新增API目录
#[command(async)]
pub fn add_api_folder(folder: APIFolder) -> CommandResult<()> {
    match schemas::add_or_update_api_folder(folder) {
        Ok(_) => Ok(()),
        Err(error) => Err(convert_sql_error(error)),
    }
}

// 更新API目录
#[command(async)]
pub fn update_api_folder(folder: APIFolder) -> CommandResult<()> {
    match schemas::add_or_update_api_folder(folder) {
        Ok(_) => Ok(()),
        Err(error) => Err(convert_sql_error(error)),
    }
}

// 获取所有API目录
#[command(async)]
pub fn list_api_folder() -> CommandResult<Vec<APIFolder>> {
    match schemas::list_api_folder() {
        Ok(result) => Ok(result),
        Err(error) => Err(convert_sql_error(error)),
    }
}
