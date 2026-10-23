//! The native engine against a local server.

use bytes::Bytes;
use cyberapi_core::model::HttpMethod;
use cyberapi_core::prefs::Timeouts;
use cyberapi_http::{HttpEngine, NativeEngine, PreparedRequest, SendError, TargetPolicy};
use http_body_util::{BodyExt as _, Full};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use pretty_assertions::assert_eq;
use std::io::Write as _;
use std::net::SocketAddr;
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
                            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                            encoder.write_all(&b"hello ".repeat(100)).expect("gzip");
                            encoder.finish().expect("gzip")
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
    assert!(lines.contains(&"accept-encoding: gzip, deflate, br"), "{echo}");
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
    let exchange = engine().send(get(address, "/gzip")).await.expect("an exchange");
    assert_eq!(exchange.body, b"hello ".repeat(100));
    assert!(exchange.body_size < 600, "the compressed size: {}", exchange.body_size);
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

/// An HTTPS server for `localhost` with a self-signed certificate, which
/// answers every request with `secret`.
async fn serve_self_signed() -> SocketAddr {
    let made = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("a certificate");
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(made.signing_key.serialize_der().into());
    let config =
        rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("TLS versions")
            .with_no_client_auth()
            .with_single_cert(vec![made.cert.der().clone()], key)
            .expect("a server config");
    let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
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
    assert!(exchange.stats.is_https);
    assert!(!exchange.stats.cipher.is_empty());
    assert!(exchange.stats.tls > Duration::ZERO);
}
