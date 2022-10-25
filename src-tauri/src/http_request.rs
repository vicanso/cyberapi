use crate::cookies;
use crate::error::CyberAPIError;
use hyper::{
    body::{Buf, Bytes},
    client::connect::HttpInfo,
    client::HttpConnector,
    header::{HeaderName, HeaderValue},
    Body, Client, Method, Request, Uri,
};
use hyper_timeout::TimeoutConnector;
use hyper_tls::HttpsConnector;
use libflate::gzip::Decoder;
use once_cell::sync::OnceCell;
use tracing_subscriber::Layer;

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap, collections::HashMap, io::Read, sync::atomic::AtomicU64,
    sync::atomic::Ordering, time::Duration, vec,
};
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
pub struct RequestTimeout {
    pub connect: u64,
    pub write: u64,
    pub read: u64,
}

#[derive(Deserialize, Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct HTTPStats {
    pub remote_addr: String,
    pub dns_lookup: u32,
    pub connect: u32,
    pub send: u32,
    pub server_processing: u32,
    pub content_transfer: u32,
    pub total: u32,
}

impl HTTPStats {
    fn new() -> Self {
        HTTPStats {
            ..Default::default()
        }
    }
}

impl From<&HTTPTrace> for HTTPStats {
    fn from(trace: &HTTPTrace) -> Self {
        let mut stats = HTTPStats::new();
        stats.dns_lookup = trace.dns_consuming();
        stats.connect = trace.connect_consuming();
        stats.server_processing = trace.server_processing_consuming();
        stats.send = trace.send_consuming();
        stats.content_transfer = trace.content_transfer_consuming();
        stats.total = trace.consuming();
        stats
    }
}

#[derive(Deserialize, Serialize, Debug, Default)]
struct HTTPTrace {
    start_value: AtomicU64,
    get_conn_value: AtomicU64,
    dns_start_value: AtomicU64,
    dns_done_value: AtomicU64,
    connect_start_value: AtomicU64,
    connected_value: AtomicU64,
    written_value: AtomicU64,
    // tls_handshake_start: AtomicU64,
    // tls_handshake_done: AtomicU64,
    got_first_response_byte_value: AtomicU64,
    done_value: AtomicU64,
}

// get conn from pool
// http connect start
// dns start
// dns done
// conn start
// connected
// first byte
// done
impl HTTPTrace {
    fn now(&self) -> u64 {
        chrono::Utc::now().timestamp_millis() as u64
    }
    fn new() -> Self {
        HTTPTrace {
            ..Default::default()
        }
    }
    fn reset(&self) {
        self.start_value.store(0, Ordering::Relaxed);
        self.get_conn_value.store(0, Ordering::Relaxed);
        self.dns_start_value.store(0, Ordering::Relaxed);
        self.dns_done_value.store(0, Ordering::Relaxed);
        self.connect_start_value.store(0, Ordering::Relaxed);
        self.connected_value.store(0, Ordering::Relaxed);
        self.written_value.store(0, Ordering::Relaxed);
        self.got_first_response_byte_value
            .store(0, Ordering::Relaxed);
        self.done_value.store(0, Ordering::Relaxed);
    }
    fn get_conn_from_pool(&self) {
        self.start_value.store(self.now(), Ordering::Relaxed)
    }
    fn get_conn(&self) {
        self.get_conn_value.store(self.now(), Ordering::Relaxed)
    }
    fn dns_start(&self) {
        self.dns_start_value.store(self.now(), Ordering::Relaxed)
    }
    fn dns_done(&self) {
        self.dns_done_value.store(self.now(), Ordering::Relaxed);
    }
    fn connect_start(&self) {
        self.connect_start_value
            .store(self.now(), Ordering::Relaxed);
    }
    fn connected(&self) {
        self.connected_value.store(self.now(), Ordering::Relaxed);
    }
    fn got_first_response_byte(&self) {
        self.got_first_response_byte_value
            .store(self.now(), Ordering::Relaxed);
    }
    fn written(&self) {
        self.written_value.store(self.now(), Ordering::Relaxed);
    }
    fn done(&self) {
        self.done_value.store(self.now(), Ordering::Relaxed);
    }
    fn send_consuming(&self) -> u32 {
        let connected_value = self.connected_value.load(Ordering::Relaxed);
        let written_value = self.written_value.load(Ordering::Relaxed);
        if connected_value == 0 || written_value == 0 {
            return 0;
        }
        return (written_value - connected_value) as u32;
    }
    fn dns_consuming(&self) -> u32 {
        let dns_start_value = self.dns_start_value.load(Ordering::Relaxed);
        let dns_done_value = self.dns_done_value.load(Ordering::Relaxed);
        if dns_start_value == 0 || dns_done_value == 0 {
            return 0;
        }
        (dns_done_value - dns_start_value) as u32
    }
    fn connect_consuming(&self) -> u32 {
        let connect_start_value = self.connect_start_value.load(Ordering::Relaxed);
        let connected_value = self.connected_value.load(Ordering::Relaxed);
        if connect_start_value == 0 || connected_value == 0 {
            return 0;
        }

        (connected_value - connect_start_value) as u32
    }
    fn server_processing_consuming(&self) -> u32 {
        let connected_value = self.connected_value.load(Ordering::Relaxed);
        let got_first_response_byte_value =
            self.got_first_response_byte_value.load(Ordering::Relaxed);
        if connected_value == 0 || got_first_response_byte_value == 0 {
            return 0;
        }

        (got_first_response_byte_value - connected_value) as u32
    }
    fn content_transfer_consuming(&self) -> u32 {
        let got_first_response_byte_value =
            self.got_first_response_byte_value.load(Ordering::Relaxed);
        let done_value = self.done_value.load(Ordering::Relaxed);
        if got_first_response_byte_value == 0 || done_value == 0 {
            return 0;
        }
        (done_value - got_first_response_byte_value) as u32
    }
    fn consuming(&self) -> u32 {
        let start_value = self.start_value.load(Ordering::Relaxed);
        let done_value = self.done_value.load(Ordering::Relaxed);
        if start_value == 0 || done_value == 0 {
            return 0;
        }
        (done_value - start_value) as u32
    }
}

static HTTP_TRACE: OnceCell<HTTPTrace> = OnceCell::new();

fn get_http_trace() -> &'static HTTPTrace {
    HTTP_TRACE.get_or_init(HTTPTrace::new)
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

struct JsonVisitor<'a>(&'a mut BTreeMap<String, String>);

impl<'a> tracing::field::Visit for JsonVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{:?}", value));
    }
}

pub struct HTTPTraceLayer;
impl<S> Layer<S> for HTTPTraceLayer
where
    S: tracing::Subscriber,
    // Scary! But there's no need to even understand it. We just need it.
    S: for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let target = event.metadata().target();
        if !target.starts_with("hyper::") {
            return;
        }

        let mut fields = BTreeMap::new();
        let mut visitor = JsonVisitor(&mut fields);

        event.record(&mut visitor);
        let message = fields.get("message");
        if message.is_none() {
            return;
        }
        let message = message.unwrap();
        // 暂时不会使用tracing span，使用比较简陋的处理方法
        let trace = get_http_trace();
        match target {
            "hyper::client::pool" => {
                // 从连接池中获取
                if message.contains("checkout waiting for idle connection") {
                    trace.get_conn_from_pool();
                }
            }
            "hyper::client::connect::http" => {
                // HTTP 开始连接
                if message.starts_with("Http::connect;") {
                    trace.get_conn();
                } else if message.starts_with("connecting to") {
                    trace.dns_done();
                }
            }
            "hyper::client::connect::dns" => {
                if message.starts_with("resolving host") {
                    trace.dns_start();
                }
            }
            "hyper::client::conn" => {
                // 开始连接
                if message.starts_with("client handshake") {
                    trace.connect_start();
                }
            }
            "hyper::client::client" => {
                // 如果是https，包括tls
                if message.starts_with("handshake complete") {
                    trace.connected();
                }
            }
            "hyper::proto::h1::io" => {
                // 获取首字节
                if message.starts_with("received ") {
                    if trace.got_first_response_byte_value.load(Ordering::Relaxed) == 0 {
                        trace.got_first_response_byte();
                    }
                } else if message.starts_with("flushed ") {
                    trace.written();
                }
            }
            _ => {}
        }
    }
}

pub async fn request(
    api: String,
    http_request: HTTPRequest,
    timeout: RequestTimeout,
) -> Result<HTTPResponse, CyberAPIError> {
    // 暂时使用单个实例，后续调整
    // 如果多个请求并发，有可能数据不精确，暂时忽略
    let trace = get_http_trace();
    trace.reset();

    let body = if http_request.content_type.starts_with("multipart/form-data") {
        // 数据为base64
        let buf = base64::decode(http_request.body)?;
        Body::from(buf)
    } else {
        Body::from(http_request.body)
    };

    let mut req = Request::new(body);

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
    let connect_timeout = Duration::from_secs(timeout.connect);
    let write_timeout = Duration::from_secs(timeout.write);
    let read_timeout = Duration::from_secs(timeout.read);

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

    let status = resp.status().as_u16();
    let mut headers = HashMap::new();
    // let mut cookie_updated = false;
    let mut set_cookies = Vec::new();
    // 对响应的header处理
    // 对于set-cookie记录至cookie store
    let mut is_gzip = false;
    let mut is_br = false;
    let content_encoding_key = "content-encoding";
    for (name, value) in resp.headers() {
        let mut key = name.to_string();
        key = key.to_lowercase();

        let value = value.to_str()?.to_string();
        if key == "set-cookie" {
            set_cookies.push(value.clone());
        }
        if key == content_encoding_key {
            if value == "gzip" {
                is_gzip = true;
            }
            if value == "br" {
                is_br = true;
            }
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

    let mut remote_addr = "".to_string();
    if let Some(info) = resp.extensions().get::<HttpInfo>() {
        remote_addr = info.remote_addr().to_string();
    }
    let mut buf = hyper::body::to_bytes(resp).await?;
    // 主动触发done，不计算解压数据耗时
    trace.done();
    let body_size = buf.len();
    // 解压gzip
    if is_gzip {
        let mut decoder = Decoder::new(&buf[..])?;
        let mut decode_data = Vec::new();
        let _ = decoder.read_to_end(&mut decode_data)?;
        buf = Bytes::copy_from_slice(&decode_data);
    }
    // 解压br
    if is_br {
        let mut decode_data = Vec::new();
        let mut r = buf.reader();
        brotli_decompressor::BrotliDecompress(&mut r, &mut decode_data)?;
        buf = Bytes::copy_from_slice(&decode_data);
    }

    let mut stats: HTTPStats = trace.into();
    stats.remote_addr = remote_addr;

    let resp = HTTPResponse {
        api,
        body_size: body_size as u32,
        latency: stats.total,
        status,
        headers,
        body: base64::encode(buf),
        stats,
    };

    Ok(resp)
}
