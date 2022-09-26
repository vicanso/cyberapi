use crate::cookies;
use crate::error::CyberAPIError;
use hyper::{
    body::Bytes,
    client::connect::HttpInfo,
    client::HttpConnector,
    header::{HeaderName, HeaderValue},
    Body, Client, Method, Request, Uri,
};
use hyper_timeout::TimeoutConnector;
use hyper_tls::HttpsConnector;
use libflate::gzip::Decoder;

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::Read, time::Duration, vec};
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

#[derive(Deserialize, Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct HTTPStats {
    pub remote_addr: String,
    pub got_first_response_byte: u32,
    pub done: u32,
}
#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HTTPResponse {
    pub api: String,
    pub latency: u32,
    pub status: u16,
    pub headers: HashMap<String, Vec<String>>,
    pub body: String,
    pub stats: HTTPStats,
    pub body_size: u32,
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
    let connect_timeout = Duration::from_secs(10);
    let write_timeout = Duration::from_secs(5);
    // TODO 设置超时由参数指定
    let read_timeout = Duration::from_secs(60);
    let mut stats = HTTPStats {
        ..Default::default()
    };

    // TODO 后续增加各阶段耗时
    // https://docs.rs/tower-http/latest/tower_http/

    // http 与 https使用不同的connector
    let resp = if current_url.scheme() == "https" {
        let h = HttpsConnector::new();
        let mut connector = TimeoutConnector::new(h);
        connector.set_connect_timeout(Some(connect_timeout));
        connector.set_read_timeout(Some(read_timeout));
        connector.set_write_timeout(Some(write_timeout));
        Client::builder()
            .build::<_, hyper::Body>(connector)
            .request(req)
            .await?
    } else {
        let h = HttpConnector::new();
        let mut connector = TimeoutConnector::new(h);
        connector.set_connect_timeout(Some(connect_timeout));
        connector.set_read_timeout(Some(read_timeout));
        connector.set_write_timeout(Some(write_timeout));
        Client::builder()
            .build::<_, hyper::Body>(connector)
            .request(req)
            .await?
    };

    stats.got_first_response_byte = (Instant::now() - now).whole_milliseconds() as u32;

    let status = resp.status().as_u16();
    let mut headers = HashMap::new();
    // let mut cookie_updated = false;
    let mut set_cookies = Vec::new();
    // 对响应的header处理
    // 对于set-cookie记录至cookie store
    let mut is_gzip = false;
    let content_encoding_key = "content-encoding";
    for (name, value) in resp.headers() {
        let mut key = name.to_string();
        key = key.to_lowercase();

        let value = value.to_str()?.to_string();
        if key == "set-cookie" {
            set_cookies.push(value.clone());
        }
        if key == content_encoding_key && value == "gzip" {
            is_gzip = true;
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

    if let Some(info) = resp.extensions().get::<HttpInfo>() {
        stats.remote_addr = info.remote_addr().to_string();
    }
    let mut buf = hyper::body::to_bytes(resp).await?;
    let body_size = buf.len();
    // 解压gzip
    if is_gzip {
        let mut decoder = Decoder::new(&buf[..])?;
        let mut decode_data = Vec::new();
        let _ = decoder.read_to_end(&mut decode_data)?;
        buf = Bytes::copy_from_slice(&decode_data);
        let _ = headers.remove(content_encoding_key);
    }

    stats.done = (Instant::now() - now).whole_milliseconds() as u32;

    let resp = HTTPResponse {
        api,
        body_size: body_size as u32,
        latency: stats.done,
        status,
        headers,
        body: base64::encode(buf),
        stats,
    };

    Ok(resp)
}
