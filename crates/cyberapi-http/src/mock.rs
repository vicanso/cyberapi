//! G2: a project's mock server on this machine's loopback address: each
//! request is answered by `cyberapi_core::mock::Routes`, which the app
//! replaces as the project changes, and told to the app for its log. It
//! stops when dropped.

use crate::native::AbortOnDrop;
use bytes::Bytes;
use cyberapi_core::mock::{EXAMPLE_HEADER, Heard, Routes};
use futures::channel::mpsc::UnboundedSender;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, RwLock};
use tokio::net::TcpListener;
use tokio::runtime::Handle;

/// A running mock server.
pub struct MockServer {
    address: SocketAddr,
    routes: Arc<RwLock<Routes>>,
    _accepting: AbortOnDrop,
}

impl MockServer {
    /// Listens on `127.0.0.1:port` (any free port for `0`), on `runtime`,
    /// telling each request answered to `heard`. `Err` says why not: the
    /// port taken, say.
    pub async fn start(
        runtime: Handle,
        port: u16,
        routes: Routes,
        heard: UnboundedSender<Heard>,
    ) -> Result<Self, String> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let listener = runtime
            .spawn(async move { TcpListener::bind(address).await })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| format!("{address}: {error}"))?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let routes = Arc::new(RwLock::new(routes));
        let shared = routes.clone();
        let accepting = runtime.spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (routes, heard) = (shared.clone(), heard.clone());
                tokio::spawn(async move {
                    let service = service_fn(move |request| {
                        let answered = answer(&routes, &request, &heard);
                        async move { Ok::<_, Infallible>(answered) }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Ok(Self {
            address,
            routes,
            _accepting: AbortOnDrop(accepting.abort_handle()),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// What answers from now on.
    pub fn set_routes(&self, routes: Routes) {
        if let Ok(mut current) = self.routes.write() {
            *current = routes;
        }
    }
}

fn answer(
    routes: &RwLock<Routes>,
    request: &hyper::Request<Incoming>,
    heard: &UnboundedSender<Heard>,
) -> hyper::Response<Full<Bytes>> {
    let header = |name: &str| request.headers().get(name).and_then(|value| value.to_str().ok());
    let method = request.method().as_str().to_string();
    let path = request
        .uri()
        .path_and_query()
        .map(|path| path.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    let preflight = header("access-control-request-method")
        .is_some()
        .then(|| header("access-control-request-headers").unwrap_or_default());
    let answered = routes
        .read()
        .map(|routes| routes.answer(&method, &path, header(EXAMPLE_HEADER), preflight))
        .unwrap_or_else(|_| cyberapi_core::mock::Answer {
            status: 500,
            headers: Vec::new(),
            body: Vec::new(),
            api: None,
        });
    let _ = heard.unbounded_send(Heard {
        at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis() as i64)
            .unwrap_or_default(),
        method,
        path,
        status: answered.status,
        api: answered.api.clone(),
    });
    let mut response = hyper::Response::builder().status(answered.status);
    for (name, value) in &answered.headers {
        response = response.header(name.as_str(), value.as_str());
    }
    response
        .body(Full::new(Bytes::from(answered.body)))
        .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new())))
}
