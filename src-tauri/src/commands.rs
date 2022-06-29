use crate::schemas::{self, APISetting};

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
pub fn add_api_setting(id: String) -> CommandResult<()> {
    match schemas::add_api_setting(id) {
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

// 保存API配置
#[command(async)]
pub fn save_api(setting: APISetting) -> CommandResult<String> {
    println!("received person struct with name: {}", setting.name);
    // future::ready(Ok("done".to_string()))
    Err(CommandError {
        message: "error".to_string(),
        ..Default::default()
    })
    // future::ready(Err(eprintln!("error")))
}
