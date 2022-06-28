use crate::schemas::{APISetting, api_setting_add};

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

#[command(async)]
pub fn add_api_setting(id: String) -> Result<(), CommandError> {
    match api_setting_add(id) {
        Ok(_) => Ok(()),
        Err(error) => Err(CommandError{
            category: CommandErrorCategory::Database.into(),
            message: error.to_string(),
        })
    }
}

// 保存API配置
#[command(async)]
pub fn save_api(setting: APISetting) -> Result<String, CommandError> {
    println!("received person struct with name: {}", setting.name);
    // future::ready(Ok("done".to_string()))
    Err(CommandError {
        message: "error".to_string(),
        ..Default::default()
    })
    // future::ready(Err(eprintln!("error")))
}
