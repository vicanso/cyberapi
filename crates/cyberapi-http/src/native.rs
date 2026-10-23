//! The engine of the desktop and the bridge: one new connection per request,
//! as the old app made, on hyper's connection-level client so every phase is
//! timed on this request's own connection (#1) — resolve, connect, TLS, then
//! the stream itself records when the request was written and when the
//! answer began.
//!
//! As before: HTTP/1.1 only, redirects are not followed (the send pipeline
//! follows them when the reader asks, one request per hop), `gzip` / `br`
//! (and now `deflate`) bodies are decompressed with their size as received
//! kept. A request may take any certificate (F14.9); the handshake is still
//! verified to be with the holder of the one presented.
//! Header values that are not UTF-8 are read lossily instead of failing the
//! request (#11). The connect timeout bounds resolving, connecting and TLS;
//! the write and read timeouts together bound the exchange after that.

use crate::{Exchange, HttpEngine, PreparedRequest, SendError, SendFuture, TargetPolicy};
use bytes::Bytes;
use cyberapi_core::response::HttpStats;
use http_body_util::{BodyExt as _, Full};
use hyper_util::rt::TokioIo;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::io::Read as _;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::task::AbortHandle;
use tokio::time::timeout;
use url::{Host, Url};

/// Sends on a tokio runtime: the bridge's own, or one the desktop keeps for
/// it (gpui's executor is not tokio).
pub struct NativeEngine {
    runtime: Handle,
    policy: TargetPolicy,
}

impl NativeEngine {
    pub fn new(runtime: Handle, policy: TargetPolicy) -> Self {
        Self { runtime, policy }
    }
}

impl HttpEngine for NativeEngine {
    fn send(&self, request: PreparedRequest) -> SendFuture {
        let task = self.runtime.spawn(exchange(request, self.policy.clone()));
        let guard = AbortOnDrop(task.abort_handle());
        Box::pin(async move {
            let _guard = guard;
            task.await.unwrap_or_else(|error| {
                Err(SendError::Http {
                    message: error.to_string(),
                })
            })
        })
    }
}

/// Cancels a spawned task when its owner goes: a dropped send stops its
/// request, and a finished one closes its connection.
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn tls_config(accept_invalid_certs: bool) -> Arc<rustls::ClientConfig> {
    static CHECKED: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    static ANY: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    let config = if accept_invalid_certs { &ANY } else { &CHECKED };
    config
        .get_or_init(|| Arc::new(client_config(!accept_invalid_certs)))
        .clone()
}

fn client_config(check_certs: bool) -> rustls::ClientConfig {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions");
    let mut config = if check_certs {
        // The system's roots, as the old app used; Mozilla's when the
        // system offers none.
        let mut roots = rustls::RootCertStore::empty();
        for cert in rustls_native_certs::load_native_certs().certs {
            let _ = roots.add(cert);
        }
        if roots.is_empty() {
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
        builder.with_root_certificates(roots).with_no_client_auth()
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
            .with_no_client_auth()
    };
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}

/// Takes the certificate a server presents, whoever issued it and whatever
/// name it is for, and still checks the server signs with its key.
#[derive(Debug)]
struct AnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn seconds(value: u32) -> Duration {
    Duration::from_secs(u64::from(value))
}

fn timed_out(phase: &str, seconds: u32) -> SendError {
    SendError::Timeout {
        phase: phase.to_string(),
        seconds,
    }
}

async fn exchange(request: PreparedRequest, policy: TargetPolicy) -> Result<Exchange, SendError> {
    let started = Instant::now();
    let timeouts = request.timeouts;
    let invalid = |message: String| SendError::Invalid { message };
    let url = Url::parse(&request.url).map_err(|error| invalid(format!("{}: {error}", request.url)))?;
    let https = match url.scheme() {
        "http" => false,
        "https" => true,
        other => return Err(invalid(format!("{other}: only http and https"))),
    };
    let host = url.host().ok_or_else(|| invalid(format!("{}: no host", request.url)))?;
    let port = url.port_or_known_default().unwrap_or(if https { 443 } else { 80 });
    let connect_limit = seconds(timeouts.connect);

    let mut stats = HttpStats {
        is_https: https,
        ..Default::default()
    };
    let connecting = async {
        let addresses: Vec<SocketAddr> = match &host {
            Host::Ipv4(ip) => vec![SocketAddr::new((*ip).into(), port)],
            Host::Ipv6(ip) => vec![SocketAddr::new((*ip).into(), port)],
            Host::Domain(name) => {
                let resolving = Instant::now();
                let found = tokio::net::lookup_host((*name, port))
                    .await
                    .map_err(|error| SendError::Dns {
                        host: name.to_string(),
                        message: error.to_string(),
                    })?
                    .collect();
                stats.dns = resolving.elapsed();
                found
            }
        };
        let permitted: Vec<SocketAddr> = addresses.into_iter().filter(|a| policy.permits(a.ip())).collect();
        if permitted.is_empty() {
            return Err(SendError::Forbidden {
                target: host.to_string(),
            });
        }
        let connecting = Instant::now();
        let mut last_error = None;
        // Each address gets its share of the time, so one that never answers
        // (an unreachable IPv6 route) cannot keep the next from being tried.
        let share = (connect_limit / permitted.len().max(1) as u32).max(Duration::from_secs(1));
        for address in permitted {
            let attempt = timeout(share, TcpStream::connect(address))
                .await
                .unwrap_or_else(|_| Err(std::io::Error::from(std::io::ErrorKind::TimedOut)));
            match attempt {
                Ok(stream) => {
                    stats.tcp = connecting.elapsed();
                    stats.remote_addr = address.to_string();
                    let _ = stream.set_nodelay(true);
                    return Ok(stream);
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(SendError::Connect {
            message: last_error.map(|e| e.to_string()).unwrap_or_default(),
        })
    };
    let stream = timeout(connect_limit, connecting)
        .await
        .map_err(|_| timed_out("connect", timeouts.connect))??;

    if https {
        let shaking = Instant::now();
        let name = match &host {
            Host::Domain(name) => ServerName::try_from(name.to_string()),
            Host::Ipv4(ip) => Ok(ServerName::IpAddress(std::net::IpAddr::V4(*ip).into())),
            Host::Ipv6(ip) => Ok(ServerName::IpAddress(std::net::IpAddr::V6(*ip).into())),
        }
        .map_err(|error| invalid(error.to_string()))?;
        let tls = timeout(
            connect_limit,
            tokio_rustls::TlsConnector::from(tls_config(request.accept_invalid_certs)).connect(name, stream),
        )
        .await
        .map_err(|_| timed_out("TLS", timeouts.connect))?
        .map_err(|error| {
            let message = error.to_string();
            match error.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()) {
                Some(rustls::Error::InvalidCertificate(_)) => SendError::Certificate { message },
                _ => SendError::Tls { message },
            }
        })?;
        stats.tls = shaking.elapsed();
        stats.cipher = tls
            .get_ref()
            .1
            .negotiated_cipher_suite()
            .map(|suite| format!("{:?}", suite.suite()))
            .unwrap_or_default();
        over(tls, &request, &url, stats, started).await
    } else {
        over(stream, &request, &url, stats, started).await
    }
}

/// The exchange on a connected stream.
async fn over<S>(
    stream: S,
    request: &PreparedRequest,
    url: &Url,
    mut stats: HttpStats,
    started: Instant,
) -> Result<Exchange, SendError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let timeouts = request.timeouts;
    let limit = seconds(timeouts.write) + seconds(timeouts.read);
    let http_error = |error: hyper::Error| SendError::Http {
        message: error.to_string(),
    };
    let timeline = Arc::new(Mutex::new(Timeline::default()));
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(Timed {
        inner: stream,
        timeline: timeline.clone(),
    }))
    .await
    .map_err(http_error)?;
    let _connection = AbortOnDrop(tokio::spawn(connection).abort_handle());

    let outgoing = build(request, url)?;
    let answer = async {
        let response = sender.send_request(outgoing).await.map_err(http_error)?;
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let raw = response.into_body().collect().await.map_err(http_error)?.to_bytes();
        Ok::<_, SendError>((status, headers, raw))
    };
    let (status, headers, raw) = timeout(limit, answer)
        .await
        .map_err(|_| timed_out("response", timeouts.write + timeouts.read))??;
    let ended = Instant::now();

    let line = timeline.lock().map(|line| line.clone()).unwrap_or_default();
    if let (Some(first_write), Some(last_write)) = (line.first_write, line.last_write) {
        stats.send = last_write.duration_since(first_write);
        if let Some(first_read) = line.first_read {
            stats.server_processing = first_read.saturating_duration_since(last_write);
            stats.content_transfer = ended.saturating_duration_since(first_read);
        }
    }
    stats.total = ended.duration_since(started);

    let encoding = headers
        .iter()
        .find(|(name, _)| name == "content-encoding")
        .map(|(_, value)| value.clone());
    let body = encoding
        .and_then(|encoding| decompress(&encoding, &raw))
        .unwrap_or_else(|| raw.to_vec());
    Ok(Exchange {
        status,
        headers,
        body,
        body_size: raw.len() as u64,
        stats,
    })
}

/// The request in origin form, with `Host` and `Accept-Encoding` unless it
/// names its own.
fn build(request: &PreparedRequest, url: &Url) -> Result<http::Request<Full<Bytes>>, SendError> {
    let invalid = |message: String| SendError::Invalid { message };
    let mut target = url.path().to_string();
    if let Some(query) = url.query() {
        target.push('?');
        target.push_str(query);
    }
    let mut builder = http::Request::builder()
        .method(
            http::Method::from_bytes(request.method.as_str().as_bytes()).map_err(|error| invalid(error.to_string()))?,
        )
        .uri(target);
    let named = |name: &str| request.headers.iter().any(|(key, _)| key.eq_ignore_ascii_case(name));
    if !named("host") {
        let host = url.host_str().unwrap_or_default();
        builder = builder.header(
            "Host",
            match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_string(),
            },
        );
    }
    if !named("accept-encoding") {
        builder = builder.header("Accept-Encoding", "gzip, deflate, br");
    }
    for (name, value) in &request.headers {
        let name = http::HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| invalid(format!("{name:?} is not a header name")))?;
        let value = http::HeaderValue::from_bytes(value.as_bytes())
            .map_err(|_| invalid(format!("the value of {name} is not a header value")))?;
        builder = builder.header(name, value);
    }
    builder
        .body(Full::new(Bytes::from(request.body.clone())))
        .map_err(|error| invalid(error.to_string()))
}

/// The body as it was before `content-encoding`; `None` for an encoding
/// this does not read, or a body that does not decode, which then stays as
/// it came.
fn decompress(encoding: &str, raw: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let read = match encoding.trim().to_ascii_lowercase().as_str() {
        "gzip" | "x-gzip" => flate2::read::MultiGzDecoder::new(raw).read_to_end(&mut out),
        "deflate" => flate2::read::ZlibDecoder::new(raw).read_to_end(&mut out).or_else(|_| {
            out.clear();
            flate2::read::DeflateDecoder::new(raw).read_to_end(&mut out)
        }),
        "br" => brotli_decompressor::Decompressor::new(raw, 4096).read_to_end(&mut out),
        _ => return None,
    };
    read.ok().map(|_| out)
}

/// When the request went out and the answer began, as the stream saw it.
#[derive(Clone, Debug, Default)]
struct Timeline {
    first_write: Option<Instant>,
    /// The last write before the first read: the request's end.
    last_write: Option<Instant>,
    first_read: Option<Instant>,
}

struct Timed<S> {
    inner: S,
    timeline: Arc<Mutex<Timeline>>,
}

impl<S> Timed<S> {
    fn wrote(&self) {
        if let Ok(mut line) = self.timeline.lock()
            && line.first_read.is_none()
        {
            let now = Instant::now();
            line.first_write.get_or_insert(now);
            line.last_write = Some(now);
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Timed<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(poll, Poll::Ready(Ok(())))
            && buf.filled().len() > before
            && let Ok(mut line) = self.timeline.lock()
        {
            line.first_read.get_or_insert_with(Instant::now);
        }
        poll
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Timed<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write(cx, buf);
        if matches!(poll, Poll::Ready(Ok(written)) if written > 0) {
            self.wrote();
        }
        poll
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write_vectored(cx, bufs);
        if matches!(poll, Poll::Ready(Ok(written)) if written > 0) {
            self.wrote();
        }
        poll
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
