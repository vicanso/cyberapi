use crate::error::CyberAPIError;
use crate::schemas::{self, APIFolder, APISetting};
use crate::{cookies, http_request};
use tauri::Manager;
use tauri::{command, Window};

pub type CommandResult<T> = Result<T, CyberAPIError>;

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

// 新增API配置
#[command(async)]
pub fn add_api_setting(setting: APISetting) -> CommandResult<()> {
    schemas::add_or_update_api_setting(setting)?;
    Ok(())
}

// 更新API配置
#[command(async)]
pub fn update_api_setting(setting: APISetting) -> CommandResult<()> {
    schemas::add_or_update_api_setting(setting)?;
    Ok(())
}

// 获取所有API配置
#[command(async)]
pub fn list_api_setting() -> CommandResult<Vec<APISetting>> {
    let result = schemas::list_api_setting()?;
    Ok(result)
}

// 新增API目录
#[command(async)]
pub fn add_api_folder(folder: APIFolder) -> CommandResult<()> {
    schemas::add_or_update_api_folder(folder)?;
    Ok(())
}

// 更新API目录
#[command(async)]
pub fn update_api_folder(folder: APIFolder) -> CommandResult<()> {
    schemas::add_or_update_api_folder(folder)?;
    Ok(())
}

// 获取所有API目录
#[command(async)]
pub fn list_api_folder() -> CommandResult<Vec<APIFolder>> {
    let result = schemas::list_api_folder()?;
    Ok(result)
}

// 执行HTTP请求
#[command(async)]
pub async fn do_http_request(
    req: http_request::HTTPRequest,
) -> CommandResult<http_request::HTTPResponse> {
    http_request::request(req).await
}

// 获取所有cookie
#[command(async)]
pub fn list_cookie() -> CommandResult<Vec<cookies::Cookie>> {
    cookies::list_cookie()
}
