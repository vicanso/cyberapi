use serde::Deserialize;
use tauri::Manager;
use tauri::{command, Window};

use crate::database;
#[derive(Debug, Clone, serde::Serialize)]
pub struct CommandError {
    message: String,
    category: String,
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
    println!("{:?}", database::init_conn());
}

#[derive(Deserialize)]
pub struct APISetting {
    name: String,
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
