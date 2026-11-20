//! G2: a project's mock server on this machine's loopback address: each
//! request is answered by `cyberapi_core::mock::Routes`, which the app
//! replaces as the project changes, and told to the app for its log. It
//! waits as the example says (M2) and sends an event stream one event at a
//! time (M3). It stops when dropped.

use crate::native::AbortOnDrop;
use bytes::Bytes;
use cyberapi_core::mock::{Answer, Asked, BODY_READ_LIMIT, Heard, Routes};
use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedSender;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt as _, Full, Limited, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::Duration;
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
                        let (routes, heard) = (routes.clone(), heard.clone());
                        async move { Ok::<_, Infallible>(answer(&routes, request, &heard).await) }
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

type Body = BoxBody<Bytes, Infallible>;

async fn answer(
    routes: &RwLock<Routes>,
    request: hyper::Request<Incoming>,
    heard: &UnboundedSender<Heard>,
) -> hyper::Response<Body> {
    let (parts, body) = request.into_parts();
    let method = parts.method.as_str().to_string();
    let path = parts
        .uri
        .path_and_query()
        .map(|path| path.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    let headers: Vec<(String, String)> = parts
        .headers
        .iter()
        .filter_map(|(name, value)| Some((name.as_str().to_string(), value.to_str().ok()?.to_string())))
        .collect();
    // M1: a body past the limit is not read; its conditions do not hold.
    let body = Limited::new(body, BODY_READ_LIMIT)
        .collect()
        .await
        .map(|collected| collected.to_bytes())
        .unwrap_or_default();
    let asked = Asked {
        method: &method,
        path: &path,
        headers: &headers,
        body: &body,
    };
    let answered = routes
        .read()
        .map(|routes| routes.answer(&asked))
        .unwrap_or_else(|_| Answer {
            status: 500,
            ..Default::default()
        });
    let _ = heard.unbounded_send(Heard::of(
        &asked,
        &answered,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis() as i64)
            .unwrap_or_default(),
    ));
    if answered.delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(answered.delay_ms.into())).await;
    }
    let mut response = hyper::Response::builder().status(answered.status);
    for (name, value) in &answered.headers {
        response = response.header(name.as_str(), value.as_str());
    }
    let body = if answered.stream_interval_ms > 0 {
        let pause = Duration::from_millis(answered.stream_interval_ms.into());
        let events = answered.events().into_iter().enumerate();
        let frames = futures::stream::iter(events).then(move |(at, event)| async move {
            if at > 0 {
                tokio::time::sleep(pause).await;
            }
            Ok::<_, Infallible>(Frame::data(Bytes::from(event)))
        });
        BoxBody::new(StreamBody::new(frames))
    } else {
        Full::new(Bytes::from(answered.body)).boxed()
    };
    response
        .body(body)
        .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new()).boxed()))
}
