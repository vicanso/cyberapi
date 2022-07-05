use crate::schemas::{self, APIFolder, APISetting};

use hyper::{
    header::{HeaderName, HeaderValue},
    Body, Client, Method, Request, Uri, body::Bytes,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::Manager;
use tauri::{command, Window};
use base64;

#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    message: String,
    category: String,
}
impl From<rusqlite::Error> for CommandError {
    fn from(error: rusqlite::Error) -> Self {
        CommandError {
            message: error.to_string(),
            category: "database".to_string(),
        }
    }
}
impl From<hyper::http::Error> for CommandError {
    fn from(error: hyper::http::Error) -> Self {
        CommandError {
            message: error.to_string(),
            category: "http".to_string(),
        }
    }
}
impl From<hyper::Error> for CommandError {
    fn from(error: hyper::Error) -> Self {
        CommandError {
            message: error.to_string(),
            category: "http".to_string(),
        }
    }
}
impl From<tauri::http::InvalidUri> for CommandError {
    fn from(error: tauri::http::InvalidUri) -> Self {
        CommandError {
            message: error.to_string(),
            category: "invalidUri".to_string(),
        }
    }
}
impl From<hyper::header::InvalidHeaderValue> for CommandError {
    fn from(error: hyper::header::InvalidHeaderValue) -> Self {
        CommandError {
            message: error.to_string(),
            category: "invalidHeader".to_string(),
        }
    }
}

impl From<hyper::header::InvalidHeaderName> for CommandError {
    fn from(error: hyper::header::InvalidHeaderName) -> Self {
        CommandError {
            message: error.to_string(),
            category: "invalidHeaderName".to_string(),
        }
    }
}

pub type CommandResult<T> = Result<T, CommandError>;

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

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HTTPRequest {
    pub method: String,
    pub uri: String,
    pub body: String,
    pub headers: HashMap<String, Vec<String>>,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HTTPResponse {
    pub status: u16,
    pub headers: HashMap<String, Vec<String>>,
    pub body: String,
}

// 执行HTTP请求
#[command(async)]
pub async fn do_http_request(http_request: HTTPRequest) -> CommandResult<HTTPResponse> {
    let mut req = Request::new(Body::from(http_request.body));

    match http_request.method.to_uppercase().as_str() {
        "POST" => *req.method_mut() = Method::POST,
        "PUT" => *req.method_mut() = Method::PUT,
        "DELETE" => *req.method_mut() = Method::DELETE,
        "HEAD" => *req.method_mut() = Method::HEAD,
        "OPTIONS" => *req.method_mut() = Method::OPTIONS,
        "CONNECT" => *req.method_mut() = Method::CONNECT,
        "PATCH" => *req.method_mut() = Method::PATCH,
        "TRACE" => *req.method_mut() = Method::TRACE,
        _ => *req.method_mut() = Method::GET,
    };
    *req.uri_mut() = http_request.uri.parse::<Uri>()?;

    let header = req.headers_mut();

    for (key, values) in http_request.headers {
        for value in values {
            header.insert(
                key.parse::<HeaderName>()?,
                HeaderValue::from_str(value.as_str())?,
            );
        }
    }
    // TODO 添加cookie

    let resp = Client::new().request(req).await?;
    let status = resp.status().as_u16();
    let headers = HashMap::new();
    for (name, value) in resp.headers() {
    }
    let buf = hyper::body::to_bytes(resp).await?;
   
    Ok(HTTPResponse{
        status: status,
        headers: headers,
        body:  base64::encode(buf),
    })
}
