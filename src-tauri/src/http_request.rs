use crate::cookies;
use crate::error::CyberAPIError;
use hyper::{
    header::{HeaderName, HeaderValue},
    Body, Client, Method, Request, Uri,
};
use hyper_tls::HttpsConnector;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HTTPRequest {
    pub method: String,
    pub uri: String,
    pub body: String,
    pub headers: HashMap<String, Vec<String>>,
    pub query: HashMap<String, Vec<String>>,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HTTPResponse {
    pub status: u16,
    pub headers: HashMap<String, Vec<String>>,
    pub body: String,
}

pub async fn request(http_request: HTTPRequest) -> Result<HTTPResponse, CyberAPIError> {
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
    for (key, values) in http_request.query {
        for value in values {
            current_url
                .query_pairs_mut()
                .append_pair(key.as_str(), value.as_str());
        }
    }

    let request_uri = current_url.as_str().parse::<Uri>()?;
    *req.uri_mut() = request_uri;

    // 设置header
    let header = req.headers_mut();
    for (key, values) in http_request.headers {
        for value in values {
            header.insert(
                key.parse::<HeaderName>()?,
                HeaderValue::from_str(value.as_str())?,
            );
        }
    }

    // 设置Cookie
    let mut cookie_store = cookies::get_cookie_store()?;
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
    let mut cookie_updated = false;
    // 对响应的header处理
    // 对于set-cookie记录至cookie store
    for (name, value) in resp.headers() {
        let key = name.to_string();

        let mut value = value.to_str()?.to_string();
        if key.to_lowercase() == "set-cookie" {
            // 对于session有效期的cookie单独处理
            if !value.contains("Expires=") {
                value.push_str(cookies::get_session_cookie_expires().as_str());
            }
            cookie_store.parse(value.as_str(), &current_url)?;
            cookie_updated = true;
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
    if cookie_updated {
        cookies::save_cookie_store(cookie_store)?;
    }
    let buf = hyper::body::to_bytes(resp).await?;

    Ok(HTTPResponse {
        status,
        headers,
        body: base64::encode(buf),
    })
}
