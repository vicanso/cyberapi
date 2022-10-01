use crate::error::CyberAPIError;
use crate::schemas::{self, APICollection, APIFolder, APISetting, Variable};
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
pub async fn add_api_setting(setting: APISetting) -> CommandResult<APISetting> {
    let result = schemas::add_api_setting(setting).await?;
    Ok(result)
}

// 更新API配置
#[command(async)]
pub async fn update_api_setting(setting: APISetting) -> CommandResult<APISetting> {
    let result = schemas::update_api_setting(setting).await?;
    Ok(result)
}

// 初始化数据库
#[command(async)]
pub async fn init_tables() -> CommandResult<()> {
    schemas::init_tables().await?;
    Ok(())
}

// 获取所有API配置
#[command(async)]
pub async fn list_api_setting(collection: String) -> CommandResult<Vec<APISetting>> {
    let result = schemas::list_api_setting(collection).await?;
    Ok(result)
}

// 删除API配置
#[command(async)]
pub async fn delete_api_settings(ids: Vec<String>) -> CommandResult<()> {
    schemas::delete_api_settings(ids).await?;
    Ok(())
}

// 新增collection
#[command(async)]
pub async fn add_api_collection(collection: APICollection) -> CommandResult<APICollection> {
    let result = schemas::add_api_collection(collection).await?;
    Ok(result)
}

// 更新collection
#[command(async)]
pub async fn update_api_collection(collection: APICollection) -> CommandResult<APICollection> {
    let result = schemas::update_api_collection(collection).await?;
    Ok(result)
}

// 获取所有collection
#[command(async)]
pub async fn list_api_collection() -> CommandResult<Vec<APICollection>> {
    let result = schemas::list_api_collection().await?;
    Ok(result)
}

#[command(async)]
pub async fn delete_api_collection(id: String) -> CommandResult<u64> {
    schemas::delete_api_setting_by_collection(id.clone()).await?;
    schemas::delete_api_folder_by_collection(id.clone()).await?;
    let count = schemas::delete_api_collection(id).await?;
    Ok(count)
}

// 新增API目录
#[command(async)]
pub async fn add_api_folder(folder: APIFolder) -> CommandResult<APIFolder> {
    let result = schemas::add_api_folder(folder).await?;
    Ok(result)
}

// 更新API目录
#[command(async)]
pub async fn update_api_folder(folder: APIFolder) -> CommandResult<APIFolder> {
    let result = schemas::update_api_folder(folder).await?;
    Ok(result)
}

// 获取所有API目录
#[command(async)]
pub async fn list_api_folder(collection: String) -> CommandResult<Vec<APIFolder>> {
    let result = schemas::list_api_folder(collection).await?;
    Ok(result)
}

// 删除API目录对应的所有子目录
#[command(async)]
pub async fn delete_api_folder(id: String) -> CommandResult<schemas::APIFolderChildren> {
    let mut result = schemas::list_api_folder_all_children(id.clone()).await?;
    result.folders.push(id);
    schemas::delete_api_folders(result.folders.clone()).await?;
    schemas::delete_api_settings(result.settings.clone()).await?;
    Ok(result)
}

// 新增变量
#[command(async)]
pub async fn add_variable(value: Variable) -> CommandResult<Variable> {
    let result = schemas::add_variable(value).await?;
    Ok(result)
}

// 更新变量
#[command(async)]
pub async fn update_variable(value: Variable) -> CommandResult<Variable> {
    let result = schemas::update_variable(value).await?;
    Ok(result)
}

// 删除变量
#[command(async)]
pub async fn delete_variable(ids: Vec<String>) -> CommandResult<u64> {
    let count = schemas::delete_variable(ids).await?;
    Ok(count)
}
// 获取所有变量
#[command(async)]
pub async fn list_variable(collection: String, category: String) -> CommandResult<Vec<Variable>> {
    let result = schemas::list_variable(collection, category).await?;
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
pub async fn get_latest_version() -> CommandResult<schemas::Version> {
    let result = schemas::get_latest_version().await?;
    Ok(result)
}

// 添加版本记录
#[command(async)]
pub async fn add_version(version: schemas::Version) -> CommandResult<()> {
    schemas::add_version(version).await?;
    Ok(())
}
