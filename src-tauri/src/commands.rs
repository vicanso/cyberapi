use crate::error::CyberAPIError;
use crate::schemas::{self, APICollection, APIFolder, APISetting, Environment};
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
pub fn list_api_setting(collection: String) -> CommandResult<Vec<APISetting>> {
    let result = schemas::list_api_setting(collection)?;
    Ok(result)
}

// 删除API配置
#[command(async)]
pub fn delete_api_settings(ids: Vec<String>) -> CommandResult<()> {
    schemas::delete_api_settings(ids)?;
    Ok(())
}

// 新增collection
#[command(async)]
pub fn add_api_collection(collection: APICollection) -> CommandResult<()> {
    schemas::add_or_update_api_collection(collection)?;
    Ok(())
}

// 更新collection
#[command(async)]
pub fn update_api_collection(collection: APICollection) -> CommandResult<()> {
    schemas::add_or_update_api_collection(collection)?;
    Ok(())
}

// 获取所有collection
#[command(async)]
pub fn list_api_collection() -> CommandResult<Vec<APICollection>> {
    let result = schemas::list_api_collection()?;
    Ok(result)
}

#[command(async)]
pub fn delete_api_collection(id: String) -> CommandResult<()> {
    schemas::delete_api_setting_by_collection(id.clone())?;
    schemas::delete_api_folder_by_collection(id.clone())?;
    schemas::delete_api_collection(id)?;
    Ok(())
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
pub fn list_api_folder(collection: String) -> CommandResult<Vec<APIFolder>> {
    let result = schemas::list_api_folder(collection)?;
    Ok(result)
}

// 删除API目录对应的所有子目录
#[command(async)]
pub fn delete_api_folder(id: String) -> CommandResult<schemas::APIFolderChildren> {
    let mut result = schemas::list_api_folder_all_children(id.clone())?;
    result.folders.push(id);
    schemas::delete_api_folders(result.folders.clone())?;
    schemas::delete_api_settings(result.settings.clone())?;
    Ok(result)
}

// 新增环境变量
#[command(async)]
pub fn add_environment(env: Environment) -> CommandResult<()> {
    schemas::add_or_update_environment(env)?;
    Ok(())
}

// 更新环境变量
#[command(async)]
pub fn update_environment(env: Environment) -> CommandResult<()> {
    schemas::add_or_update_environment(env)?;
    Ok(())
}

// 更新环境变量
#[command(async)]
pub fn delete_environment(ids: Vec<String>) -> CommandResult<()> {
    schemas::delete_environment(ids)?;
    Ok(())
}

// 获取所有环境变量
#[command(async)]
pub fn list_environment(collection: String) -> CommandResult<Vec<Environment>> {
    let result = schemas::list_environment(collection)?;
    Ok(result)
}

// 执行HTTP请求
#[command(async)]
pub async fn do_http_request(
    api: String,
    req: http_request::HTTPRequest,
) -> CommandResult<http_request::HTTPResponse> {
    http_request::request(api, req).await
}

// 获取所有cookie
#[command(async)]
pub fn list_cookie() -> CommandResult<Vec<String>> {
    cookies::list_cookie()
}

// 删除cookie
#[command(async)]
pub fn delete_cookie(c: cookies::Cookie) -> CommandResult<()> {
    cookies::delete_cookie_from_store(c)?;
    Ok(())
}

// 添加cookie
#[command(async)]
pub fn add_cookie(c: cookies::Cookie) -> CommandResult<()> {
    cookies::add_cookie(c)?;
    Ok(())
}

// 获取最新版本
#[command(async)]
pub fn get_latest_version() -> CommandResult<schemas::Version> {
    let result = schemas::get_latest_version()?;
    Ok(result)
}

// 添加版本记录
#[command(async)]
pub fn add_version(version: schemas::Version) -> CommandResult<()> {
    _ = schemas::add_version(version)?;
    Ok(())
}