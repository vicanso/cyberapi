//! The engine of the desktop and the bridge: one new connection per request,
//! as the old app made, on hyper's connection-level client so every phase is
//! timed on this request's own connection (#1) — resolve, connect, TLS, then
//! the stream itself records when the request was written, and the answer's
//! start is when its first byte came over HTTP/1.1 and its headers over
//! HTTP/2, whose connection exchanges frames of its own before that.
//!
//! HTTP/2 where TLS agrees on it, or straight away over plain HTTP when the
//! request asks for it (F14.10); HTTP/1.1 otherwise. Redirects are not
//! followed (the send pipeline follows them when the reader asks, one
//! request per hop). `gzip` / `br` (and now `deflate`) bodies are
//! decompressed, away from the async threads, with their size as received
//! kept; at most [`RESPONSE_LIMIT`] of a body is read either way. A request
//! may take any certificate (F14.9); the handshake is still verified to be
//! with the holder of the one presented. Header values that are not UTF-8
//! are read lossily instead of failing the request (#11). The connect
//! timeout bounds resolving, connecting and TLS; the write and read timeouts
//! together bound the exchange after that.

use crate::{Exchange, HttpEngine, PreparedRequest, SendError, SendFuture, TargetPolicy};
use bytes::Bytes;
use cyberapi_core::prefs::HttpVersion;
use cyberapi_core::response::{HttpStats, RESPONSE_LIMIT};
use http_body_util::{BodyExt as _, Full};
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::any::Any;
use std::io::Read;
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
    body_limit: usize,
}

impl NativeEngine {
    pub fn new(runtime: Handle, policy: TargetPolicy) -> Self {
        Self {
            runtime,
            policy,
            body_limit: RESPONSE_LIMIT,
        }
    }

    /// Reads at most `limit` bytes of a body, as it comes and decompressed;
    /// [`RESPONSE_LIMIT`] unless set.
    pub fn body_limit(mut self, limit: usize) -> Self {
        self.body_limit = limit;
        self
    }
}

impl HttpEngine for NativeEngine {
    fn send(&self, request: PreparedRequest) -> SendFuture {
        let task = self
            .runtime
            .spawn(exchange(request, self.policy.clone(), self.body_limit));
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

/// The client's TLS: the roots checked, or any certificate taken, offering
/// the protocols `version` allows.
fn tls_config(accept_invalid_certs: bool, version: HttpVersion) -> Arc<rustls::ClientConfig> {
    static CHECKED: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    static ANY: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    let base = if accept_invalid_certs { &ANY } else { &CHECKED };
    let mut config = (**base.get_or_init(|| Arc::new(client_config(!accept_invalid_certs)))).clone();
    config.alpn_protocols = match version {
        HttpVersion::Auto => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
        HttpVersion::Http1 => vec![b"http/1.1".to_vec()],
        HttpVersion::Http2 => vec![b"h2".to_vec()],
    };
    Arc::new(config)
}

fn client_config(check_certs: bool) -> rustls::ClientConfig {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions");
    if check_certs {
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
    }
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

async fn exchange(request: PreparedRequest, policy: TargetPolicy, body_limit: usize) -> Result<Exchange, SendError> {
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
            tokio_rustls::TlsConnector::from(tls_config(request.accept_invalid_certs, request.http_version))
                .connect(name, stream),
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
        let session = tls.get_ref().1;
        stats.cipher = session
            .negotiated_cipher_suite()
            .map(|suite| format!("{:?}", suite.suite()))
            .unwrap_or_default();
        let h2 = session.alpn_protocol() == Some(b"h2".as_slice());
        if request.http_version == HttpVersion::Http2 && !h2 {
            return Err(SendError::Http {
                message: "the server did not agree to HTTP/2".to_string(),
            });
        }
        over(tls, &request, &url, stats, started, h2, body_limit).await
    } else {
        let h2 = request.http_version == HttpVersion::Http2;
        over(stream, &request, &url, stats, started, h2, body_limit).await
    }
}

/// The exchange on a connected stream, in HTTP/2 when `h2`.
async fn over<S>(
    stream: S,
    request: &PreparedRequest,
    url: &Url,
    mut stats: HttpStats,
    started: Instant,
    h2: bool,
    body_limit: usize,
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
    let io = TokioIo::new(Timed {
        inner: stream,
        timeline: timeline.clone(),
    });
    let outgoing = build(request, url, h2)?;
    let answer = async {
        // The connection runs on a task of its own, and both it and the
        // sender stay until the body is read.
        let (response, _kept): (_, (AbortOnDrop, Box<dyn Any + Send>)) = if h2 {
            let (mut sender, connection) = hyper::client::conn::http2::handshake(TokioExecutor::new(), io)
                .await
                .map_err(http_error)?;
            let connection = AbortOnDrop(tokio::spawn(connection).abort_handle());
            // The preface and the settings are the connection's: the
            // request starts after them.
            if let Ok(mut line) = timeline.lock() {
                *line = Timeline::default();
            }
            let response = sender.send_request(outgoing).await.map_err(http_error)?;
            (response, (connection, Box::new(sender)))
        } else {
            let (mut sender, connection) = hyper::client::conn::http1::handshake(io).await.map_err(http_error)?;
            let connection = AbortOnDrop(tokio::spawn(connection).abort_handle());
            let response = sender.send_request(outgoing).await.map_err(http_error)?;
            (response, (connection, Box::new(sender)))
        };
        if let Ok(mut line) = timeline.lock() {
            line.headers = Some(Instant::now());
        }
        let version = match response.version() {
            http::Version::HTTP_2 => "HTTP/2".to_string(),
            http::Version::HTTP_11 => "HTTP/1.1".to_string(),
            http::Version::HTTP_10 => "HTTP/1.0".to_string(),
            other => format!("{other:?}"),
        };
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
        let (raw, truncated) = read_body(response.into_body(), body_limit).await.map_err(http_error)?;
        Ok::<_, SendError>((version, status, headers, raw, truncated))
    };
    let (version, status, headers, raw, truncated) = timeout(limit, answer)
        .await
        .map_err(|_| timed_out("response", timeouts.write + timeouts.read))??;
    let ended = Instant::now();

    let line = timeline.lock().map(|line| line.clone()).unwrap_or_default();
    if let (Some(first_write), Some(last_write)) = (line.first_write, line.last_write) {
        stats.send = last_write.duration_since(first_write);
        // Over HTTP/2 the server's settings are read before any answer.
        let began = if h2 {
            line.headers
        } else {
            line.first_read.or(line.headers)
        };
        if let Some(began) = began {
            stats.server_processing = began.saturating_duration_since(last_write);
            stats.content_transfer = ended.saturating_duration_since(began);
        }
    }
    stats.total = ended.duration_since(started);
    stats.version = version;

    let body_size = raw.len() as u64;
    let encoding = headers
        .iter()
        .find(|(name, _)| name == "content-encoding")
        .map(|(_, value)| value.clone());
    let (body, truncated) = match encoding {
        // Inflating is work for a thread of its own: on an async thread it
        // holds up every other request there.
        Some(encoding) => tokio::task::spawn_blocking(move || {
            decompress(&encoding, &raw, truncated, body_limit).unwrap_or((raw, truncated))
        })
        .await
        .map_err(|error| SendError::Http {
            message: error.to_string(),
        })?,
        None => (raw, truncated),
    };
    Ok(Exchange {
        status,
        headers,
        body,
        body_size,
        stats,
        truncated,
    })
}

/// The body, up to `limit` bytes, and whether there was more.
async fn read_body(mut body: Incoming, limit: usize) -> Result<(Vec<u8>, bool), hyper::Error> {
    let mut raw = Vec::new();
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame?.into_data() {
            let room = limit - raw.len();
            if data.len() > room {
                raw.extend_from_slice(&data[..room]);
                return Ok((raw, true));
            }
            raw.extend_from_slice(&data);
        }
    }
    Ok((raw, false))
}

/// What HTTP/2 forbids: a connection's own headers. `Host` is its
/// authority there, and `TE` may only ask for trailers.
const HTTP1_ONLY: [&str; 5] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
];

/// The request, with `Accept-Encoding` unless it names its own. Over
/// HTTP/1.1 in origin form with `Host`; over HTTP/2 with the address in
/// full, a `Host` the request names as its authority, and the headers
/// HTTP/2 forbids left out.
fn build(request: &PreparedRequest, url: &Url, h2: bool) -> Result<http::Request<Full<Bytes>>, SendError> {
    let invalid = |message: String| SendError::Invalid { message };
    let mut target = url.path().to_string();
    if let Some(query) = url.query() {
        target.push('?');
        target.push_str(query);
    }
    let authority = match request.header("host") {
        Some(host) => host.trim().to_string(),
        None => {
            let host = url.host_str().unwrap_or_default();
            match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_string(),
            }
        }
    };
    let mut builder = http::Request::builder().method(
        http::Method::from_bytes(request.method.as_str().as_bytes()).map_err(|error| invalid(error.to_string()))?,
    );
    builder = if h2 {
        builder
            .version(http::Version::HTTP_2)
            .uri(format!("{}://{authority}{target}", url.scheme()))
    } else {
        let builder = builder.uri(target);
        if request.header("host").is_none() {
            builder.header("Host", authority)
        } else {
            builder
        }
    };
    if request.header("accept-encoding").is_none() {
        builder = builder.header("Accept-Encoding", "gzip, deflate, br");
    }
    for (name, value) in &request.headers {
        let lower = name.trim().to_ascii_lowercase();
        if h2
            && (HTTP1_ONLY.contains(&lower.as_str())
                || lower == "host"
                || (lower == "te" && !value.trim().eq_ignore_ascii_case("trailers")))
        {
            continue;
        }
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

/// The body as it was before `content-encoding`, at most `limit` bytes of
/// it, and whether there was more; `None` for an encoding this does not
/// read, or a body that does not decode, which then stays as it came. A
/// body cut short (`cut`) gives what decodes of its start.
fn decompress(encoding: &str, raw: &[u8], cut: bool, limit: usize) -> Option<(Vec<u8>, bool)> {
    let inflate = |reader: &mut dyn Read, out: &mut Vec<u8>| reader.take(limit as u64 + 1).read_to_end(out);
    let mut out = Vec::new();
    let read = match encoding.trim().to_ascii_lowercase().as_str() {
        "gzip" | "x-gzip" => inflate(&mut flate2::read::MultiGzDecoder::new(raw), &mut out),
        "deflate" => inflate(&mut flate2::read::ZlibDecoder::new(raw), &mut out).or_else(|_| {
            out.clear();
            inflate(&mut flate2::read::DeflateDecoder::new(raw), &mut out)
        }),
        "br" => inflate(&mut brotli_decompressor::Decompressor::new(raw, 4096), &mut out),
        _ => return None,
    };
    if out.len() > limit {
        out.truncate(limit);
        return Some((out, true));
    }
    match read {
        Ok(_) => Some((out, cut)),
        Err(_) if cut && !out.is_empty() => Some((out, true)),
        Err(_) => None,
    }
}

/// When the request went out and the answer began, as the stream saw it.
#[derive(Clone, Debug, Default)]
struct Timeline {
    first_write: Option<Instant>,
    /// The last write before the answer: the request's end.
    last_write: Option<Instant>,
    first_read: Option<Instant>,
    /// When the answer's headers were in.
    headers: Option<Instant>,
}

struct Timed<S> {
    inner: S,
    timeline: Arc<Mutex<Timeline>>,
}

impl<S> Timed<S> {
    fn wrote(&self) {
        // Until the answer's headers: over HTTP/2 the request's frames go
        // out after the server's settings were read.
        if let Ok(mut line) = self.timeline.lock()
            && line.headers.is_none()
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
