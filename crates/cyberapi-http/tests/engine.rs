//! The native engine against a local server.

use bytes::Bytes;
use cyberapi_core::model::HttpMethod;
use cyberapi_core::prefs::{HttpVersion, Timeouts};
use cyberapi_core::response::StreamEnd;
use cyberapi_http::{HttpEngine, NativeEngine, PreparedRequest, Progress, SendError, TargetPolicy, Watch, stop_signal};
use http_body_util::{BodyExt as _, Full};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use pretty_assertions::assert_eq;
use std::io::Write as _;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;

/// A server whose paths answer: `/echo` with what it got, `/gzip` a gzipped
/// text, `/found` a redirect, `/slow` after three seconds, `/latin1` a
/// header that is not UTF-8.
async fn serve() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let service = service_fn(|request: hyper::Request<hyper::body::Incoming>| async move {
                    let path = request.uri().path().to_string();
                    let mut response = hyper::Response::builder();
                    let body: Vec<u8> = match path.as_str() {
                        "/echo" => {
                            let mut lines = vec![format!("{} {}", request.method(), request.uri())];
                            for (name, value) in request.headers() {
                                lines.push(format!("{name}: {}", value.to_str().unwrap_or("?")));
                            }
                            let body = request
                                .into_body()
                                .collect()
                                .await
                                .map(|b| b.to_bytes())
                                .unwrap_or_default();
                            lines.push(String::from_utf8_lossy(&body).into_owned());
                            lines.join("\n").into_bytes()
                        }
                        "/gzip" => {
                            response = response.header("content-encoding", "gzip");
                            gzip(&b"hello ".repeat(100))
                        }
                        "/zstd" => {
                            response = response.header("content-encoding", "zstd");
                            zstd(&b"hello ".repeat(100))
                        }
                        // Gzip, then zstd over that.
                        "/stacked" => {
                            response = response.header("content-encoding", "gzip, zstd");
                            zstd(&gzip(&b"hello ".repeat(100)))
                        }
                        "/found" => {
                            response = response.status(302).header("location", "/echo");
                            Vec::new()
                        }
                        "/slow" => {
                            tokio::time::sleep(Duration::from_secs(3)).await;
                            b"late".to_vec()
                        }
                        "/latin1" => {
                            response = response.header(
                                "x-name",
                                hyper::header::HeaderValue::from_bytes(b"caf\xe9").expect("a value"),
                            );
                            Vec::new()
                        }
                        _ => {
                            response = response.status(404);
                            Vec::new()
                        }
                    };
                    response.body(Full::new(Bytes::from(body)))
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    address
}

/// An event stream server: `/events` sends three events 50 ms apart and
/// ends, saying its encoding is `none` as GraphQL Yoga does; `/open` sends
/// one and stays open, quiet; `/grpc` sends the three as a gRPC server's
/// stream would, typed `application/grpc`.
async fn serve_events() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let service = service_fn(|request: hyper::Request<hyper::body::Incoming>| async move {
                    let open = request.uri().path() == "/open";
                    let grpc = request.uri().path() == "/grpc";
                    let count = if open { 1 } else { 3 };
                    let pieces = futures::stream::unfold(0, move |n| async move {
                        if n == count {
                            if open {
                                std::future::pending::<()>().await;
                            }
                            return None;
                        }
                        if n > 0 {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                        let piece = hyper::body::Frame::data(Bytes::from(format!("data: {n}\n\n")));
                        Some((Ok::<_, std::convert::Infallible>(piece), n + 1))
                    });
                    let kind = if grpc { "application/grpc" } else { "text/event-stream" };
                    let response = hyper::Response::builder().header("content-type", kind);
                    let response = if open || grpc {
                        response
                    } else {
                        response.header("content-encoding", "none")
                    };
                    response.body(http_body_util::StreamBody::new(pieces))
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    address
}

/// A watch that keeps what it is told, and fires its stop after `stop_after`
/// pieces of body, if given.
fn watching(stop_after: Option<usize>) -> (Watch, Arc<Mutex<Vec<Progress>>>) {
    let told = Arc::new(Mutex::new(Vec::new()));
    let (stop, stopped) = stop_signal();
    let stop = Mutex::new(Some(stop));
    let kept = told.clone();
    let progress = move |progress: Progress| {
        let mut told = kept.lock().expect("told");
        told.push(progress);
        let pieces = told.iter().filter(|p| matches!(p, Progress::Body(_))).count();
        if Some(pieces) == stop_after
            && let Some(stop) = stop.lock().expect("stop").take()
        {
            let _ = stop.send(());
        }
    };
    (
        Watch {
            progress: Arc::new(progress),
            stop: stopped,
        },
        told,
    )
}

/// A watched event stream is told as it comes and read to its end; one
/// the reader stops, or that goes quiet for the read timeout, is what came
/// by then. Unwatched, it is read as any body.
#[tokio::test]
async fn a_watched_event_stream_is_read_as_it_comes() {
    let address = serve_events().await;
    let (watch, told) = watching(None);
    let exchange = engine()
        .send_watched(get(address, "/events"), watch)
        .await
        .expect("an exchange");
    assert_eq!(exchange.body, b"data: 0\n\ndata: 1\n\ndata: 2\n\n");
    let stream = exchange.stream.expect("a stream");
    assert_eq!(stream.ended, StreamEnd::Closed);
    assert_eq!(
        stream.arrivals.iter().map(|(end, _)| *end).collect::<Vec<_>>(),
        [9, 18, 27]
    );
    // When each came, from the head as this end saw it: never earlier than
    // the one before. Not how far apart — a busy machine sees the head late,
    // and the pieces after it closer together than they were sent.
    assert!(
        stream.arrivals.windows(2).all(|pair| pair[0].1 <= pair[1].1),
        "{:?}",
        stream.arrivals
    );
    let told = told.lock().expect("told").clone();
    assert!(matches!(&told[0], Progress::Head { status: 200, .. }));
    assert_eq!(
        told[1..],
        [
            Progress::Body(b"data: 0\n\n".to_vec()),
            Progress::Body(b"data: 1\n\n".to_vec()),
            Progress::Body(b"data: 2\n\n".to_vec())
        ]
    );

    let (watch, _) = watching(Some(1));
    let stopped = engine()
        .send_watched(get(address, "/open"), watch)
        .await
        .expect("an exchange");
    assert_eq!(stopped.body, b"data: 0\n\n");
    assert_eq!(stopped.stream.map(|s| s.ended), Some(StreamEnd::Stopped));

    let (watch, _) = watching(None);
    let quiet = PreparedRequest {
        timeouts: Timeouts {
            read: 1,
            ..Timeouts::default()
        }
        .effective(),
        ..get(address, "/open")
    };
    let started = Instant::now();
    let quieted = engine().send_watched(quiet, watch).await.expect("an exchange");
    assert_eq!(quieted.stream.map(|s| s.ended), Some(StreamEnd::Quiet { seconds: 1 }));
    assert!(started.elapsed() < Duration::from_secs(3));

    let unwatched = engine().send(get(address, "/events")).await.expect("an exchange");
    assert_eq!((unwatched.body.len(), unwatched.stream), (27, None));

    // Any answer the request says streams.
    let (watch, told) = watching(None);
    let streamed = PreparedRequest {
        streams: true,
        ..get(address, "/grpc")
    };
    let exchange = engine().send_watched(streamed, watch).await.expect("an exchange");
    assert_eq!(exchange.stream.map(|s| s.arrivals.len()), Some(3));
    assert_eq!(told.lock().expect("told").len(), 4);
    let (watch, _) = watching(None);
    let plain = engine()
        .send_watched(get(address, "/grpc"), watch)
        .await
        .expect("an exchange");
    assert_eq!((plain.body.len(), plain.stream), (27, None), "not said to stream");
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).expect("gzip");
    encoder.finish().expect("gzip")
}

fn zstd(bytes: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(bytes, ruzstd::encoding::CompressionLevel::Fastest)
}

fn engine() -> NativeEngine {
    NativeEngine::new(tokio::runtime::Handle::current(), TargetPolicy::Anywhere)
}

fn get(address: SocketAddr, path: &str) -> PreparedRequest {
    PreparedRequest {
        method: HttpMethod::Get,
        url: format!("http://{address}{path}"),
        timeouts: Timeouts::default().effective(),
        ..Default::default()
    }
}

#[tokio::test]
async fn sends_what_was_prepared_and_times_it() {
    let address = serve().await;
    let request = PreparedRequest {
        method: HttpMethod::Post,
        headers: vec![
            ("Content-Type".into(), "text/plain".into()),
            ("X-Token".into(), "t".into()),
        ],
        body: b"hi there".to_vec(),
        ..get(address, "/echo?a=1&b=two")
    };
    let exchange = engine().send(request).await.expect("an exchange");
    assert_eq!(exchange.status, 200);
    let echo = String::from_utf8(exchange.body.clone()).expect("text");
    let lines: Vec<&str> = echo.lines().collect();
    assert_eq!(lines[0], "POST /echo?a=1&b=two");
    assert!(lines.contains(&format!("host: {address}").as_str()), "{echo}");
    assert!(lines.contains(&"accept-encoding: gzip, deflate, br, zstd"), "{echo}");
    assert!(lines.contains(&"x-token: t"), "{echo}");
    assert!(lines.contains(&"content-length: 8"), "{echo}");
    assert_eq!(lines.last(), Some(&"hi there"));

    let stats = exchange.stats;
    assert_eq!(stats.remote_addr, address.to_string());
    assert!(!stats.is_https);
    assert!(stats.total > Duration::ZERO);
    assert!(stats.total >= stats.tcp + stats.send + stats.server_processing + stats.content_transfer);
}

#[tokio::test]
async fn bodies_are_decompressed_with_their_size_as_received() {
    let address = serve().await;
    for path in ["/gzip", "/zstd", "/stacked"] {
        let exchange = engine().send(get(address, path)).await.expect("an exchange");
        assert_eq!(exchange.body, b"hello ".repeat(100), "{path}");
        assert!(
            exchange.body_size < 600,
            "{path}: the compressed size: {}",
            exchange.body_size
        );
    }
}

#[tokio::test]
async fn redirects_are_not_followed_and_odd_headers_still_read() {
    let address = serve().await;
    let found = engine().send(get(address, "/found")).await.expect("an exchange");
    assert_eq!(found.status, 302);
    assert!(found.headers.contains(&("location".into(), "/echo".into())));

    let latin1 = engine().send(get(address, "/latin1")).await.expect("an exchange");
    assert!(latin1.headers.contains(&("x-name".into(), "caf\u{fffd}".into())));
}

#[tokio::test]
async fn the_bridges_policy_refuses_inner_addresses() {
    let address = serve().await;
    // The default reaches the team's networks, not the bridge's own host.
    let networks = NativeEngine::new(
        tokio::runtime::Handle::current(),
        TargetPolicy::Networks { allowed: vec![] },
    );
    assert_eq!(
        networks.send(get(address, "/echo")).await,
        Err(SendError::Forbidden {
            target: "127.0.0.1".into()
        })
    );
    // A name is said with the address it led to.
    let by_name = PreparedRequest {
        url: format!("http://localhost:{}/echo", address.port()),
        ..get(address, "/echo")
    };
    let refused = networks.send(by_name).await.expect_err("refused");
    let SendError::Forbidden { target } = &refused else {
        panic!("{refused:?}");
    };
    assert!(target.starts_with("localhost (") && target.ends_with(')'), "{target}");
    assert!(refused.to_string().contains("--allow-target"), "{refused}");
    let public_only = NativeEngine::new(
        tokio::runtime::Handle::current(),
        TargetPolicy::PublicOnly { allowed: vec![] },
    );
    assert_eq!(
        public_only.send(get(address, "/echo")).await,
        Err(SendError::Forbidden {
            target: "127.0.0.1".into()
        })
    );
    let allowed = NativeEngine::new(
        tokio::runtime::Handle::current(),
        TargetPolicy::PublicOnly {
            allowed: vec!["127.0.0.0/8".parse().expect("a range")],
        },
    );
    assert_eq!(allowed.send(get(address, "/echo")).await.map(|e| e.status), Ok(200));
}

#[tokio::test]
async fn bad_requests_and_slow_servers_fail_with_their_reason() {
    let address = serve().await;
    let bad_header = PreparedRequest {
        headers: vec![("bad name".into(), "v".into())],
        ..get(address, "/echo")
    };
    assert!(matches!(
        engine().send(bad_header).await,
        Err(SendError::Invalid { .. })
    ));
    let ftp = PreparedRequest {
        url: "ftp://x/".into(),
        ..get(address, "/")
    };
    assert!(matches!(engine().send(ftp).await, Err(SendError::Invalid { .. })));

    let slow = PreparedRequest {
        timeouts: Timeouts {
            connect: 1,
            write: 1,
            read: 1,
        },
        ..get(address, "/slow")
    };
    let started = Instant::now();
    assert!(matches!(engine().send(slow).await, Err(SendError::Timeout { .. })));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn dropping_the_send_cancels_it() {
    let address = serve().await;
    let started = Instant::now();
    let sending = engine().send(get(address, "/slow"));
    // Gone before the answer: the request must not keep the caller waiting.
    let result = tokio::time::timeout(Duration::from_millis(200), sending).await;
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
}

/// A TLS acceptor for `localhost` with a self-signed certificate,
/// agreeing to the protocols in `alpn`.
fn self_signed(alpn: &[&[u8]]) -> tokio_rustls::TlsAcceptor {
    let made = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("a certificate");
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(made.signing_key.serialize_der().into());
    let mut config =
        rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("TLS versions")
            .with_no_client_auth()
            .with_single_cert(vec![made.cert.der().clone()], key)
            .expect("a server config");
    config.alpn_protocols = alpn.iter().map(|protocol| protocol.to_vec()).collect();
    tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config))
}

/// An HTTPS server for `localhost` with a self-signed certificate and no
/// ALPN, which answers every request over HTTP/1.1 with `secret`.
async fn serve_self_signed() -> SocketAddr {
    let acceptor = self_signed(&[]);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(stream) = acceptor.accept(stream).await else {
                    return;
                };
                let service = service_fn(|_: hyper::Request<hyper::body::Incoming>| async {
                    hyper::Response::builder().body(Full::new(Bytes::from_static(b"secret")))
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    address
}

/// A server's stream whose first write waits: over HTTP/2 that is its
/// settings, which reach a client far away a round trip after its request
/// went out — and the client answers them then.
struct SlowStart<S> {
    inner: S,
    wait: Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
}

impl<S: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for SlowStart<S> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for SlowStart<S> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if let Some(wait) = self.wait.as_mut() {
            std::task::ready!(wait.as_mut().poll(cx));
            self.wait = None;
        }
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// An HTTP/2-only server, over TLS (agreeing to `h2` alone) or plain (h2c).
/// Over TLS its first write waits 200 ms, as a far server's would arrive.
/// `/slow` answers after 300 ms, `/trailers` with a body and trailers after
/// it, as a gRPC server does; anything else answers with the request's
/// method, authority and headers.
async fn serve_h2(tls: bool) -> SocketAddr {
    let acceptor = tls.then(|| self_signed(&[b"h2"]));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let service = service_fn(|request: hyper::Request<hyper::body::Incoming>| async move {
                    if request.uri().path() == "/slow" {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                    if request.uri().path() == "/trailers" {
                        let mut trailers = hyper::HeaderMap::new();
                        trailers.insert("grpc-status", hyper::header::HeaderValue::from_static("5"));
                        trailers.insert("Grpc-Message", hyper::header::HeaderValue::from_static("not%20here"));
                        let body = Full::new(Bytes::from_static(b"reply"))
                            .with_trailers(std::future::ready(Some(Ok(trailers))))
                            .boxed();
                        return hyper::Response::builder().body(body);
                    }
                    let mut lines = vec![format!(
                        "{} {}",
                        request.method(),
                        request.uri().authority().map(|a| a.as_str()).unwrap_or("-")
                    )];
                    for (name, value) in request.headers() {
                        lines.push(format!("{name}: {}", value.to_str().unwrap_or("?")));
                    }
                    hyper::Response::builder().body(Full::new(Bytes::from(lines.join("\n"))).boxed())
                });
                let builder = hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new());
                let _ = match acceptor {
                    Some(acceptor) => match acceptor.accept(stream).await {
                        Ok(stream) => {
                            let slow = SlowStart {
                                inner: stream,
                                wait: Some(Box::pin(tokio::time::sleep(Duration::from_millis(200)))),
                            };
                            builder.serve_connection(TokioIo::new(slow), service).await
                        }
                        Err(_) => return,
                    },
                    None => builder.serve_connection(TokioIo::new(stream), service).await,
                };
            });
        }
    });
    address
}

fn to(url: String, http_version: HttpVersion) -> PreparedRequest {
    PreparedRequest {
        url,
        timeouts: Timeouts::default().effective(),
        accept_invalid_certs: true,
        http_version,
        ..Default::default()
    }
}

/// F14.10: HTTPS speaks HTTP/2 where the server agrees, and the time to
/// its answer is the server's, not the connection's own frames that come
/// first.
#[tokio::test]
async fn https_speaks_http2_where_the_server_agrees() {
    let address = serve_h2(true).await;
    let url = format!("https://localhost:{}/slow", address.port());
    let exchange = engine()
        .send(PreparedRequest {
            headers: vec![
                ("Connection".into(), "keep-alive".into()),
                ("X-Note".into(), "kept".into()),
            ],
            ..to(url.clone(), HttpVersion::Auto)
        })
        .await
        .expect("an exchange");
    assert_eq!((exchange.status, exchange.stats.version.as_str()), (200, "HTTP/2"));
    let echoed = String::from_utf8_lossy(&exchange.body).into_owned();
    assert!(echoed.contains("x-note: kept"), "{echoed}");
    assert!(
        !echoed.contains("connection"),
        "HTTP/2 has no Connection header: {echoed}"
    );
    // The client answers the server's settings after its request went out;
    // that answer is no part of sending it.
    let stats = &exchange.stats;
    assert!(stats.server_processing >= Duration::from_millis(300), "{stats:?}");
    assert!(stats.send < Duration::from_millis(100), "{stats:?}");
    // A body too: it is taken in frames, not with the headers.
    let posted = engine()
        .send(PreparedRequest {
            method: HttpMethod::Post,
            body: vec![b'x'; 64 * 1024],
            ..to(url.clone(), HttpVersion::Auto)
        })
        .await
        .expect("an exchange");
    let stats = &posted.stats;
    assert!(stats.server_processing >= Duration::from_millis(300), "{stats:?}");
    assert!(stats.send < Duration::from_millis(100), "{stats:?}");

    let forced = engine().send(to(url.clone(), HttpVersion::Http2)).await;
    assert_eq!(forced.map(|e| e.stats.version), Ok("HTTP/2".to_string()));
    let refused = engine().send(to(url, HttpVersion::Http1)).await;
    assert!(refused.is_err(), "an HTTP/2-only server takes no HTTP/1.1: {refused:?}");
}

/// A server that does not agree to HTTP/2 gets HTTP/1.1, unless the request
/// asks for HTTP/2 only.
#[tokio::test]
async fn http2_only_fails_where_the_server_does_not_agree() {
    let address = serve_self_signed().await;
    let url = format!("https://localhost:{}/", address.port());
    let exchange = engine()
        .send(to(url.clone(), HttpVersion::Auto))
        .await
        .expect("an exchange");
    assert_eq!(exchange.stats.version, "HTTP/1.1");
    let refused = engine().send(to(url, HttpVersion::Http2)).await;
    assert_eq!(
        refused.map(|e| e.status),
        Err(SendError::Http {
            message: "the server did not agree to HTTP/2".into()
        })
    );
}

/// Plain HTTP starts in HTTP/2 when asked (h2c), a `Host` the request names
/// becoming the authority.
#[tokio::test]
async fn plain_http_speaks_http2_when_asked() {
    let address = serve_h2(false).await;
    let exchange = engine()
        .send(PreparedRequest {
            headers: vec![("Host".into(), "api.test".into())],
            ..to(format!("http://{address}/"), HttpVersion::Http2)
        })
        .await
        .expect("an exchange");
    assert_eq!(exchange.stats.version, "HTTP/2");
    let echoed = String::from_utf8_lossy(&exchange.body).into_owned();
    assert!(echoed.starts_with("GET api.test"), "{echoed}");
}

/// The trailers after a body are kept: a gRPC call's status is there.
#[tokio::test]
async fn the_trailers_after_a_body_are_kept() {
    let address = serve_h2(false).await;
    let exchange = engine()
        .send(to(format!("http://{address}/trailers"), HttpVersion::Http2))
        .await
        .expect("an exchange");
    assert_eq!(exchange.body, b"reply");
    assert_eq!(
        exchange.trailers,
        [
            ("grpc-status".to_string(), "5".to_string()),
            ("grpc-message".to_string(), "not%20here".to_string())
        ]
    );
    assert!(!exchange.headers.iter().any(|(name, _)| name == "grpc-status"));
}

/// At most the limit of a body is read, as it comes and decompressed.
#[tokio::test]
async fn a_body_is_read_up_to_the_limit() {
    let address = serve().await;
    let small = NativeEngine::new(tokio::runtime::Handle::current(), TargetPolicy::Anywhere).body_limit(100);
    let exchange = small
        .send(PreparedRequest {
            body: vec![b'x'; 500],
            method: HttpMethod::Post,
            ..get(address, "/echo")
        })
        .await
        .expect("an exchange");
    assert_eq!(
        (exchange.body.len(), exchange.body_size, exchange.truncated),
        (100, 100, true)
    );

    // 600 bytes of text, a few dozen gzipped: cut once inflated.
    let exchange = small.send(get(address, "/gzip")).await.expect("an exchange");
    assert_eq!(exchange.body, b"hello ".repeat(100)[..100].to_vec());
    assert!(exchange.truncated);
    assert!(exchange.body_size < 100);

    let whole = engine().send(get(address, "/gzip")).await.expect("an exchange");
    assert_eq!((whole.body.len(), whole.truncated), (600, false));
}

#[tokio::test]
async fn a_certificate_no_root_vouches_for_is_taken_only_when_asked() {
    let address = serve_self_signed().await;
    let request = PreparedRequest {
        url: format!("https://localhost:{}/", address.port()),
        timeouts: Timeouts::default().effective(),
        ..Default::default()
    };
    let refused = engine().send(request.clone()).await;
    assert!(matches!(refused, Err(SendError::Certificate { .. })), "{refused:?}");

    let exchange = engine()
        .send(PreparedRequest {
            accept_invalid_certs: true,
            ..request
        })
        .await
        .expect("an exchange");
    assert_eq!((exchange.status, exchange.body), (200, b"secret".to_vec()));
    assert!(exchange.stats.is_https);
    assert!(!exchange.stats.cipher.is_empty());
}

#[tokio::test]
#[ignore = "reaches the internet"]
async fn https_with_the_systems_roots() {
    let request = PreparedRequest {
        url: "https://example.com/".into(),
        timeouts: Timeouts::default().effective(),
        ..Default::default()
    };
    let exchange = engine().send(request).await.expect("an exchange");
    assert_eq!(exchange.status, 200);
    assert_eq!(exchange.stats.version, "HTTP/2", "example.com agrees to HTTP/2");
    assert!(exchange.stats.is_https);
    assert!(!exchange.stats.cipher.is_empty());
    assert!(exchange.stats.tls > Duration::ZERO);
}

/// A proxy for the tests (#60): an HTTP one answers a plain request itself,
/// with the head it was sent, and tunnels a `CONNECT` to `to` (one to a
/// host named `blocked…` it refuses); a SOCKS5 one connects every request
/// to `to`. What each was asked is kept, a line a request.
async fn serve_proxy(socks: bool, to: SocketAddr) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let asked: Arc<Mutex<Vec<String>>> = Arc::default();
    let kept = asked.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut client, _)) = listener.accept().await else {
                return;
            };
            let asked = asked.clone();
            tokio::spawn(async move {
                let note = |line: String| asked.lock().expect("asked").push(line);
                if socks {
                    let mut head = [0u8; 2];
                    client.read_exact(&mut head).await.expect("greeting");
                    let mut methods = vec![0u8; usize::from(head[1])];
                    client.read_exact(&mut methods).await.expect("methods");
                    if methods.contains(&0x02) {
                        client.write_all(&[0x05, 0x02]).await.expect("method");
                        let mut length = [0u8; 2];
                        client.read_exact(&mut length).await.expect("user length");
                        let mut user = vec![0u8; usize::from(length[1])];
                        client.read_exact(&mut user).await.expect("user");
                        let mut password_length = [0u8];
                        client.read_exact(&mut password_length).await.expect("password length");
                        let mut password = vec![0u8; usize::from(password_length[0])];
                        client.read_exact(&mut password).await.expect("password");
                        note(format!(
                            "user {}:{}",
                            String::from_utf8_lossy(&user),
                            String::from_utf8_lossy(&password)
                        ));
                        client.write_all(&[0x01, 0x00]).await.expect("signed in");
                    } else {
                        client.write_all(&[0x05, 0x00]).await.expect("method");
                    }
                    let mut request = [0u8; 4];
                    client.read_exact(&mut request).await.expect("request");
                    let host = match request[3] {
                        0x01 => {
                            let mut octets = [0u8; 4];
                            client.read_exact(&mut octets).await.expect("ipv4");
                            std::net::Ipv4Addr::from(octets).to_string()
                        }
                        _ => {
                            let mut length = [0u8];
                            client.read_exact(&mut length).await.expect("name length");
                            let mut name = vec![0u8; usize::from(length[0])];
                            client.read_exact(&mut name).await.expect("name");
                            String::from_utf8_lossy(&name).into_owned()
                        }
                    };
                    let mut port = [0u8; 2];
                    client.read_exact(&mut port).await.expect("port");
                    note(format!("connect {host}:{}", u16::from_be_bytes(port)));
                    client
                        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                        .await
                        .expect("reply");
                } else {
                    let mut head = Vec::new();
                    while !head.ends_with(b"\r\n\r\n") {
                        let mut byte = [0u8];
                        if client.read(&mut byte).await.unwrap_or(0) == 0 {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    note(head.trim_end().replace("\r\n", " | "));
                    if !head.starts_with("CONNECT ") {
                        let answer = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{head}", head.len());
                        client.write_all(answer.as_bytes()).await.expect("answer");
                        return;
                    }
                    if head.starts_with("CONNECT blocked") {
                        client
                            .write_all(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\n\r\n")
                            .await
                            .expect("refusal");
                        return;
                    }
                    client
                        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                        .await
                        .expect("tunnel");
                }
                let mut server = tokio::net::TcpStream::connect(to).await.expect("the server");
                let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
            });
        }
    });
    (address, kept)
}

fn through(url: &str, proxy: &str) -> PreparedRequest {
    PreparedRequest {
        method: HttpMethod::Get,
        url: url.into(),
        timeouts: Timeouts::default().effective(),
        accept_invalid_certs: true,
        proxy: cyberapi_core::proxy::ProxyPrefs {
            mode: cyberapi_core::proxy::ProxyMode::Manual,
            url: proxy.into(),
            bypass: String::new(),
        },
        ..Default::default()
    }
}

#[tokio::test]
async fn a_plain_request_goes_to_an_http_proxy_whole() {
    let (proxy, asked) = serve_proxy(false, serve().await).await;
    let engine = engine().with_proxies();
    let exchange = engine
        .send(through(
            "http://origin.test:8080/x?y=1#part",
            &format!("http://u:p@{proxy}"),
        ))
        .await
        .expect("an exchange");
    let head = String::from_utf8(exchange.body).expect("text");
    assert!(
        head.starts_with("GET http://origin.test:8080/x?y=1 HTTP/1.1\r\n"),
        "{head}"
    );
    assert!(
        head.to_ascii_lowercase().contains("proxy-authorization: basic dtpw"),
        "{head}"
    );
    assert!(head.to_ascii_lowercase().contains("host: origin.test:8080"), "{head}");
    assert_eq!(exchange.stats.proxy, format!("http://{proxy}"));
    assert_eq!(exchange.stats.remote_addr, proxy.to_string());
    assert_eq!(asked.lock().expect("asked").len(), 1);
}

#[tokio::test]
async fn https_goes_through_a_connect_tunnel() {
    let (proxy, asked) = serve_proxy(false, serve_self_signed().await).await;
    let engine = engine().with_proxies();
    let exchange = engine
        .send(through("https://origin.test/", &format!("{proxy}")))
        .await
        .expect("an exchange");
    assert_eq!(exchange.body, b"secret");
    assert!(exchange.stats.is_https);
    assert_eq!(
        asked.lock().expect("asked").as_slice(),
        ["CONNECT origin.test:443 HTTP/1.1 | Host: origin.test:443"]
    );
    // A proxy that will not connect says so.
    let refused = engine.send(through("https://blocked.test/", &format!("{proxy}"))).await;
    match refused {
        Err(SendError::Proxy { message }) => assert!(message.contains("403"), "{message}"),
        other => panic!("not refused: {other:?}"),
    }
}

#[tokio::test]
async fn socks5_connects_by_name_or_by_address() {
    let (proxy, asked) = serve_proxy(true, serve().await).await;
    let engine = engine().with_proxies();
    let exchange = engine
        .send(through("http://origin.test/echo", &format!("socks5h://u:p@{proxy}")))
        .await
        .expect("an exchange");
    let echo = String::from_utf8(exchange.body).expect("text");
    assert!(echo.starts_with("GET /echo"), "{echo}");
    assert_eq!(exchange.stats.proxy, format!("socks5h://{proxy}"));
    // socks5: the address found here; an address as it is.
    engine
        .send(through("http://10.9.8.7:81/echo", &format!("socks5://{proxy}")))
        .await
        .expect("an exchange");
    assert_eq!(
        asked.lock().expect("asked").as_slice(),
        ["user u:p", "connect origin.test:80", "connect 10.9.8.7:81"]
    );
}

#[tokio::test]
async fn this_machine_and_the_bridge_go_straight_to_the_server() {
    let address = serve().await;
    // Nothing listens there: going through it would fail.
    let dead = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind")
        .local_addr()
        .expect("address");
    let to_loopback = through(&format!("http://{address}/echo"), &format!("http://{dead}"));
    let exchange = engine().with_proxies().send(to_loopback).await.expect("straight there");
    assert!(exchange.stats.proxy.is_empty());
    // The bridge's engine takes no proxy from a request (a page's): this
    // one would answer for any address; straight there, there is none.
    let (proxy, asked) = serve_proxy(false, address).await;
    let elsewhere = through("http://10.9.8.7:1/", &format!("http://{proxy}"));
    let direct = engine().send(PreparedRequest {
        timeouts: Timeouts {
            connect: 1,
            ..Timeouts::default()
        }
        .effective(),
        ..elsewhere
    });
    assert!(direct.await.is_err(), "not through the proxy");
    assert!(asked.lock().expect("asked").is_empty());
}
