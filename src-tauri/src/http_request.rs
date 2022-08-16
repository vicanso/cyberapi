use crate::cookies;
use crate::error::CyberAPIError;
use hyper::{
    header::{HeaderName, HeaderValue},
    Body, Client, Method, Request, Uri,
};
use hyper_tls::HttpsConnector;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, vec};
use time::Instant;
use url::Url;

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HTTPRequestKVParam {
    pub key: String,
    pub value: String,
    pub enabled: bool,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HTTPRequest {
    pub method: String,
    pub uri: String,
    pub body: String,
    pub content_type: String,
    pub headers: Vec<HTTPRequestKVParam>,
    pub query: Vec<HTTPRequestKVParam>,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HTTPResponse {
    pub api: String,
    pub latency: u32,
    pub status: u16,
    pub headers: HashMap<String, Vec<String>>,
    pub body: String,
}

pub async fn request(
    api: String,
    http_request: HTTPRequest,
) -> Result<HTTPResponse, CyberAPIError> {
    let now = Instant::now();

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

    // 设置query
    let mut current_url = Url::parse(http_request.uri.as_str())?;
    for q in http_request.query {
        if !q.enabled {
            continue;
        }
        current_url.query_pairs_mut().append_pair(&q.key, &q.value);
    }

    let request_uri = current_url.as_str().parse::<Uri>()?;
    *req.uri_mut() = request_uri;

    // 设置header
    let mut set_content_type = false;
    let content_type = "content-type";
    let header = req.headers_mut();
    for h in http_request.headers {
        if !h.enabled {
            continue;
        }
        if h.key.to_lowercase() == content_type {
            set_content_type = true;
        }
        header.insert(
            h.key.parse::<HeaderName>()?,
            HeaderValue::from_str(h.value.as_str())?,
        );
    }
    // 如果未设置content type
    // 设置content type
    if !set_content_type && !http_request.content_type.is_empty() {
        header.insert(
            content_type.parse::<HeaderName>()?,
            HeaderValue::from_str(http_request.content_type.as_str())?,
        );
    }

    {
        // cookie store未实现send，避免与下面的await冲突
        // 设置Cookie
        let cookie_store = cookies::get_cookie_store();
        let cookie_header = cookie_store
            .get_request_values(&current_url)
            .map(|(name, value)| format!("{}={}", name, value))
            .collect::<Vec<_>>()
            .join("; ");
        if !cookie_header.is_empty() {
            header.insert(
                "Cookie".parse::<HeaderName>()?,
                HeaderValue::from_str(cookie_header.as_str())?,
            );
        }
    }
    // TODO 设置超时

    // http 与 https使用不同的connector
    let resp = if current_url.scheme() == "https" {
        let https = HttpsConnector::new();
        Client::builder()
            .build::<_, hyper::Body>(https)
            .request(req)
            .await?
    } else {
        Client::new().request(req).await?
    };

    let status = resp.status().as_u16();
    let mut headers = HashMap::new();
    // let mut cookie_updated = false;
    let mut set_cookies = Vec::new();
    // 对响应的header处理
    // 对于set-cookie记录至cookie store
    for (name, value) in resp.headers() {
        let key = name.to_string();

        let value = value.to_str()?.to_string();
        if key.to_lowercase() == "set-cookie" {
            set_cookies.push(value.clone());
        }
        // 响应的Header value处理
        let values: Option<&Vec<String>> = headers.get(&key);
        match values {
            Some(values) => {
                let mut values = values.to_vec();
                values.push(value);
                headers.insert(key, values);
            }
            None => {
                headers.insert(key, vec![value]);
            }
        }
    }
    // 如果有更新cookie，则写入
    if !set_cookies.is_empty() {
        cookies::save_cookie_store(set_cookies, &current_url)?;
    }
    let buf = hyper::body::to_bytes(resp).await?;

    let d = Instant::now() - now;

    Ok(HTTPResponse {
        api,
        latency: d.whole_milliseconds() as u32,
        status,
        headers,
        body: base64::encode(buf),
    })
}
