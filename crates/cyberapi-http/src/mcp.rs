//! H2: the desktop's local loopback MCP HTTP server on `127.0.0.1:<port>`.
//!
//! Answers `POST /v1/mcp` (and `/mcp` or `/`) when signed in with the local
//! bearer token (`Authorization: Bearer cat_local_…`), delegating JSON-RPC
//! calls to the callback the desktop supplies. Stops when dropped.

use crate::native::AbortOnDrop;
use bytes::Bytes;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use tokio::net::TcpListener;
use tokio::runtime::Handle;

/// Maximum JSON-RPC request body size accepted by the local MCP server (1 MiB).
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// What a local MCP token is allowed to do right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpAccess {
    pub token: String,
    pub can_send: bool,
    pub can_write: bool,
}

/// Async callback that executes one authenticated MCP JSON-RPC request body
/// (`(can_send, can_write, body) -> (status_code, optional_json_body)`).
pub type McpHandler =
    Arc<dyn Fn(bool, bool, Vec<u8>) -> Pin<Box<dyn Future<Output = (u16, Option<String>)> + Send>> + Send + Sync>;

/// A running local loopback MCP server.
pub struct McpServer {
    address: SocketAddr,
    access: Arc<RwLock<McpAccess>>,
    _accepting: AbortOnDrop,
}

impl McpServer {
    /// Listens on `127.0.0.1:port` (`0` for any free port) on `runtime`.
    pub async fn start(runtime: Handle, port: u16, access: McpAccess, handler: McpHandler) -> Result<Self, String> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let listener = runtime
            .spawn(async move { TcpListener::bind(address).await })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| format!("{address}: {error}"))?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let access = Arc::new(RwLock::new(access));
        let shared = access.clone();
        let accepting = runtime.spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (access, handler) = (shared.clone(), handler.clone());
                tokio::spawn(async move {
                    let service = service_fn(move |request| {
                        let (access, handler) = (access.clone(), handler.clone());
                        async move { Ok::<_, Infallible>(answer(access, handler, request).await) }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Ok(Self {
            address,
            access,
            _accepting: AbortOnDrop(accepting.abort_handle()),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Updates the bearer token and permissions without restarting the
    /// listener.
    pub fn set_access(&self, access: McpAccess) {
        if let Ok(mut current) = self.access.write() {
            *current = access;
        }
    }
}

fn token_matches(expected: &str, given: &str) -> bool {
    if expected.is_empty() || expected.len() != given.len() {
        return false;
    }
    expected
        .as_bytes()
        .iter()
        .zip(given.as_bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

async fn answer(
    access: Arc<RwLock<McpAccess>>,
    handler: McpHandler,
    request: hyper::Request<Incoming>,
) -> hyper::Response<Full<Bytes>> {
    let path = request.uri().path();
    if !matches!(path, "/v1/mcp" | "/v1/mcp/" | "/mcp" | "/mcp/" | "/") {
        return hyper::Response::builder()
            .status(404)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from_static(b"{\"error\":\"not found\"}")))
            .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new())));
    }
    if request.method() != hyper::Method::POST {
        return hyper::Response::builder()
            .status(405)
            .header("allow", "POST")
            .body(Full::new(Bytes::new()))
            .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new())));
    }
    let current = access.read().ok().map(|guard| guard.clone());
    let bearer = request
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer ")))
        .map(str::trim);
    let Some((current, true)) = current.zip(bearer).map(|(c, b)| {
        let ok = token_matches(&c.token, b);
        (c, ok)
    }) else {
        return hyper::Response::builder()
            .status(401)
            .header("www-authenticate", "Bearer")
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from_static(
                b"{\"error\":\"an MCP token is needed: Authorization: Bearer cat_local_...\"}",
            )))
            .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new())));
    };
    let Ok(collected) = Limited::new(request.into_body(), MAX_BODY_BYTES).collect().await else {
        return hyper::Response::builder()
            .status(413)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from_static(b"{\"error\":\"request body too large\"}")))
            .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new())));
    };
    let (status, body) = (handler)(current.can_send, current.can_write, collected.to_bytes().to_vec()).await;
    match body {
        Some(json) => hyper::Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(json)))
            .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new()))),
        None => hyper::Response::builder()
            .status(status)
            .body(Full::new(Bytes::new()))
            .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new()))),
    }
}
