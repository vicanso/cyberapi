//! E2: a request whose body streams, against a local HTTP/2 server.
//! A client-streaming handler answers only once the body is finished, so
//! the call must be usable before its headers arrive. A bidirectional one
//! answers at once and echoes each piece.

use cyberapi_core::model::HttpMethod;
use cyberapi_core::prefs::{HttpVersion, Timeouts};
use cyberapi_http::{CallIn, CallOut, HttpEngine, LiveCall, NativeEngine, PreparedRequest, TargetPolicy};
use futures::StreamExt as _;
use futures::channel::mpsc;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::Frame;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;

/// What `/Upload` was asked.
struct Seen {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// `/Upload` collects the body, then echoes it with a gRPC status trailer.
/// `/Chat` sends its headers at once and echoes each data frame.
async fn serve() -> (SocketAddr, Arc<Mutex<Seen>>) {
    let seen = Arc::new(Mutex::new(Seen {
        headers: Vec::new(),
        body: Vec::new(),
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let kept = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let kept = kept.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                    let kept = kept.clone();
                    async move {
                        let path = request.uri().path().to_string();
                        let headers = request
                            .headers()
                            .iter()
                            .map(|(name, value)| (name.as_str().to_string(), value.to_str().unwrap_or("?").to_string()))
                            .collect::<Vec<_>>();
                        if path == "/Upload" {
                            let collected = request.into_body().collect().await;
                            let bytes = collected.map(|body| body.to_bytes()).unwrap_or_default();
                            if let Ok(mut seen) = kept.lock() {
                                seen.headers = headers;
                                seen.body = bytes.to_vec();
                            }
                            let mut trailers = hyper::HeaderMap::new();
                            trailers.insert("grpc-status", hyper::header::HeaderValue::from_static("0"));
                            let body = Full::new(bytes)
                                .with_trailers(std::future::ready(Some(Ok(trailers))))
                                .boxed();
                            return hyper::Response::builder()
                                .header("content-type", "application/grpc")
                                .body(body);
                        }
                        let (tx, rx) = mpsc::unbounded();
                        tokio::spawn(async move {
                            let mut body = request.into_body();
                            while let Some(frame) = body.frame().await {
                                let Ok(frame) = frame else { break };
                                if let Ok(data) = frame.into_data()
                                    && !data.is_empty()
                                {
                                    let _ = tx.unbounded_send(Ok::<_, Infallible>(Frame::data(data)));
                                }
                            }
                            let mut trailers = hyper::HeaderMap::new();
                            trailers.insert("grpc-status", hyper::header::HeaderValue::from_static("0"));
                            let _ = tx.unbounded_send(Ok(Frame::trailers(trailers)));
                        });
                        hyper::Response::builder()
                            .header("content-type", "application/grpc")
                            .body(BodyExt::boxed(StreamBody::new(rx)))
                    }
                });
                let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    (address, seen)
}

fn engine() -> NativeEngine {
    NativeEngine::new(tokio::runtime::Handle::current(), TargetPolicy::Anywhere)
}

fn request(address: SocketAddr, path: &str) -> PreparedRequest {
    PreparedRequest {
        method: HttpMethod::Post,
        url: format!("http://{address}{path}"),
        headers: vec![
            ("content-type".into(), "application/grpc".into()),
            ("te".into(), "trailers".into()),
            // Named, and stripped: a streaming body has no one length.
            ("content-length".into(), "5".into()),
        ],
        timeouts: Timeouts::default().effective(),
        http_version: HttpVersion::Http2,
        ..Default::default()
    }
}

async fn until_end(call: &mut LiveCall) -> Vec<CallIn> {
    let mut got = Vec::new();
    while let Some(item) = call.incoming.next().await {
        let ended = matches!(item, CallIn::Ended | CallIn::Failed(_));
        got.push(item);
        if ended {
            break;
        }
    }
    got
}

/// The body goes out before any header comes back. Finishing it is what
/// lets the server answer; dropping the call would reset it instead.
#[tokio::test]
async fn a_client_stream_is_answered_only_once_its_body_ends() {
    let (address, seen) = serve().await;
    let mut call = engine().open_call(request(address, "/Upload")).await.expect("open");
    call.outgoing
        .unbounded_send(CallOut::Data(b"one".to_vec()))
        .expect("one");
    call.outgoing
        .unbounded_send(CallOut::Data(b"two".to_vec()))
        .expect("two");
    call.outgoing.unbounded_send(CallOut::Finish).expect("finish");
    let got = tokio::time::timeout(Duration::from_secs(2), until_end(&mut call))
        .await
        .expect("the answer came; waiting for headers first would deadlock");
    assert!(got.iter().any(|item| matches!(item, CallIn::Head { status: 200, .. })));
    let data = got.iter().fold(Vec::<u8>::new(), |mut data, item| {
        if let CallIn::Data(bytes) = item {
            data.extend(bytes);
        }
        data
    });
    assert_eq!(data, b"onetwo");
    let trailers = got.iter().find_map(|item| match item {
        CallIn::Trailers(fields) => Some(fields.clone()),
        _ => None,
    });
    assert_eq!(
        trailers.as_ref().and_then(|fields| {
            fields
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("grpc-status"))
                .map(|(_, value)| value.as_str())
        }),
        Some("0")
    );
    assert!(got.iter().any(|item| matches!(item, CallIn::Ended)));
    let seen = seen.lock().expect("seen");
    assert_eq!(seen.body, b"onetwo");
    assert!(
        seen.headers
            .iter()
            .any(|(name, value)| name == "content-type" && value == "application/grpc"),
        "{:?}",
        seen.headers
    );
    assert!(
        seen.headers
            .iter()
            .any(|(name, value)| name == "te" && value == "trailers"),
        "{:?}",
        seen.headers
    );
    assert!(
        seen.headers.iter().all(|(name, _)| name != "content-length"),
        "{:?}",
        seen.headers
    );
}

/// Headers come before the body is finished, and each piece is echoed.
#[tokio::test]
async fn a_bidi_stream_answers_before_the_body_ends() {
    let (address, _) = serve().await;
    let mut call = engine().open_call(request(address, "/Chat")).await.expect("open");
    let head = tokio::time::timeout(Duration::from_secs(2), call.incoming.next())
        .await
        .expect("headers before the body ends")
        .expect("a head");
    assert!(matches!(head, CallIn::Head { status: 200, .. }), "{head:?}");
    call.outgoing.unbounded_send(CallOut::Data(b"hi".to_vec())).expect("hi");
    let echo = tokio::time::timeout(Duration::from_secs(2), call.incoming.next())
        .await
        .expect("the echo")
        .expect("data");
    assert!(matches!(echo, CallIn::Data(ref bytes) if bytes == b"hi"), "{echo:?}");
    call.outgoing.unbounded_send(CallOut::Finish).expect("finish");
    let rest = tokio::time::timeout(Duration::from_secs(2), until_end(&mut call))
        .await
        .expect("trailers");
    assert!(rest.iter().any(|item| matches!(item, CallIn::Trailers(_))));
    assert!(rest.iter().any(|item| matches!(item, CallIn::Ended)));
}

/// Giving the call up ends it, rather than half-closing the body.
#[tokio::test]
async fn cancelling_ends_the_call() {
    let (address, _) = serve().await;
    let mut call = engine().open_call(request(address, "/Chat")).await.expect("open");
    let _ = tokio::time::timeout(Duration::from_secs(2), call.incoming.next())
        .await
        .expect("open");
    call.outgoing.unbounded_send(CallOut::Cancel).expect("cancel");
    tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(item) = call.incoming.next().await {
            if matches!(item, CallIn::Ended | CallIn::Failed(_)) {
                return;
            }
        }
    })
    .await
    .expect("cancel ended the call");
}
