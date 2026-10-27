//! E1: a WebSocket on a connection of its own, made as any request's is
//! (`native::dial`: timed, checked against the address rules, through the
//! proxy as a tunnel, with the client certificate the host takes), then
//! upgraded over HTTP/1.1 (RFC 6455 §4): the request's headers with the
//! handshake's, answered by a 101 whose `Sec-WebSocket-Accept` answers the
//! key. A task of its own then carries what the reader sends and what
//! comes, until either side closes it or the connection fails.

use crate::native::{Desktop, dial, pairs, read_body, seconds, timed_out};
use crate::{Exchange, Frame, Opened, PreparedRequest, SendError, Socket, SocketIn, SocketOut, TargetPolicy};
use bytes::Bytes;
use cyberapi_core::response::RESPONSE_LIMIT;
use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures::{SinkExt as _, StreamExt as _};
use http_body_util::Full;
use hyper_util::rt::TokioIo;
use std::time::{Duration, Instant};
use tokio::time::timeout;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use url::Url;

/// How long the server has to answer the reader's close before the
/// connection is dropped anyway.
const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// The handshake's own headers: the request's of these names give way.
const HANDSHAKE: [&str; 6] = [
    "connection",
    "upgrade",
    "sec-websocket-key",
    "sec-websocket-version",
    "content-length",
    "transfer-encoding",
];

pub(crate) async fn open(
    request: PreparedRequest,
    policy: TargetPolicy,
    desktop: Desktop,
) -> Result<Opened, SendError> {
    let started = Instant::now();
    let invalid = |message: String| SendError::Invalid { message };
    let address = cyberapi_core::ws::http_url(&request.url)
        .ok_or_else(|| invalid(format!("{}: only ws and wss", request.url)))?;
    let url = Url::parse(&address).map_err(|error| invalid(format!("{}: {error}", request.url)))?;
    let dialed = dial(&request, &url, &policy, &desktop, true).await?;
    let mut stats = dialed.stats;
    let key = generate_key();
    let handshake = handshake(&request, &url, &key)?;
    let timeouts = request.timeouts;
    let limit = seconds(timeouts.write) + seconds(timeouts.read);
    let http_error = |error: hyper::Error| SendError::Http {
        message: error.to_string(),
    };
    let answer = async {
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(dialed.stream))
            .await
            .map_err(http_error)?;
        // It hands the connection over once upgraded.
        tokio::spawn(connection.with_upgrades());
        let sent = Instant::now();
        let response = sender.send_request(handshake).await.map_err(http_error)?;
        Ok::<_, SendError>((response, sent.elapsed()))
    };
    let late = || timed_out("response", timeouts.write + timeouts.read);
    let (response, waited) = timeout(limit, answer).await.map_err(|_| late())??;
    stats.server_processing = waited;
    stats.version = "HTTP/1.1".into();
    let status = response.status().as_u16();
    let headers = pairs(response.headers());
    if status != 101 {
        let left = limit.saturating_sub(started.elapsed());
        let (body, truncated, trailers) = timeout(left, read_body(response.into_body(), RESPONSE_LIMIT))
            .await
            .map_err(|_| late())?
            .map_err(http_error)?;
        stats.total = started.elapsed();
        return Ok(Opened::Refused(Exchange {
            status,
            headers,
            body_size: body.len() as u64,
            body,
            stats,
            truncated,
            trailers,
            stream: None,
        }));
    }
    let accepted = headers
        .iter()
        .find(|(name, _)| name == "sec-websocket-accept")
        .map(|(_, value)| value.trim());
    if accepted != Some(derive_accept_key(key.as_bytes()).as_str()) {
        return Err(SendError::Http {
            message: "the server's Sec-WebSocket-Accept does not answer the key".into(),
        });
    }
    let upgraded = hyper::upgrade::on(response).await.map_err(http_error)?;
    stats.total = started.elapsed();
    let socket = WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Client, None).await;
    let (outgoing, sending) = mpsc::unbounded();
    let (telling, incoming) = mpsc::unbounded();
    tokio::spawn(carry(socket, sending, telling, Instant::now()));
    Ok(Opened::Socket(Socket {
        status,
        headers,
        stats,
        outgoing,
        incoming,
    }))
}

/// The upgrade request: `GET` of the path and query, the request's headers
/// but the handshake's own, then those.
fn handshake(request: &PreparedRequest, url: &Url, key: &str) -> Result<http::Request<Full<Bytes>>, SendError> {
    let invalid = |message: String| SendError::Invalid { message };
    let mut target = url.path().to_string();
    if let Some(query) = url.query() {
        target.push('?');
        target.push_str(query);
    }
    let mut builder = http::Request::builder().method(http::Method::GET).uri(target);
    if request.header("host").is_none() {
        let host = url.host_str().unwrap_or_default();
        let authority = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };
        builder = builder.header("Host", authority);
    }
    for (name, value) in &request.headers {
        if HANDSHAKE.contains(&name.trim().to_ascii_lowercase().as_str()) {
            continue;
        }
        let name = http::HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| invalid(format!("{name:?} is not a header name")))?;
        let value = http::HeaderValue::from_bytes(value.as_bytes())
            .map_err(|_| invalid(format!("the value of {name} is not a header value")))?;
        builder = builder.header(name, value);
    }
    builder
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", key)
        .body(Full::new(Bytes::new()))
        .map_err(|error| invalid(error.to_string()))
}

/// What the reader sends goes out, and what comes is told, each with when,
/// till it ends: closed by either side — the reader's close waits
/// `CLOSE_WAIT` for the server's answer — or failed.
async fn carry<S>(
    socket: WebSocketStream<S>,
    mut sending: UnboundedReceiver<SocketOut>,
    telling: UnboundedSender<(SocketIn, u64)>,
    opened: Instant,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut sink, mut stream) = socket.split();
    let tell = |what: SocketIn| {
        let _ = telling.unbounded_send((what, opened.elapsed().as_millis() as u64));
    };
    let closed = |code: Option<u16>, reason: String| SocketIn::Closed { code, reason };
    // When the reader closed it.
    let mut closing: Option<Instant> = None;
    loop {
        let wait = closing.map_or(CLOSE_WAIT, |since| CLOSE_WAIT.saturating_sub(since.elapsed()));
        tokio::select! {
            out = sending.next(), if closing.is_none() => match out {
                Some(SocketOut::Send(frame)) => {
                    let message = match frame {
                        Frame::Text(text) => Message::Text(text.into()),
                        Frame::Binary(bytes) => Message::Binary(bytes.into()),
                    };
                    if let Err(error) = sink.send(message).await {
                        tell(SocketIn::Failed(error.to_string()));
                        return;
                    }
                }
                // Closed, or the reader gone.
                Some(SocketOut::Close) | None => {
                    let _ = sink.send(Message::Close(None)).await;
                    closing = Some(Instant::now());
                }
            },
            came = stream.next() => match came {
                Some(Ok(Message::Text(text))) => tell(SocketIn::Message(Frame::Text(text.to_string()))),
                Some(Ok(Message::Binary(bytes))) => tell(SocketIn::Message(Frame::Binary(bytes.to_vec()))),
                Some(Ok(Message::Close(frame))) => {
                    tell(match frame {
                        Some(frame) => closed(Some(u16::from(frame.code)), frame.reason.to_string()),
                        None => closed(None, String::new()),
                    });
                    return;
                }
                // Pings are answered as they are read.
                Some(Ok(_)) => {}
                Some(Err(_)) | None if closing.is_some() => {
                    tell(closed(None, String::new()));
                    return;
                }
                Some(Err(error)) => {
                    tell(SocketIn::Failed(error.to_string()));
                    return;
                }
                None => {
                    tell(closed(None, String::new()));
                    return;
                }
            },
            _ = tokio::time::sleep(wait), if closing.is_some() => {
                tell(closed(None, String::new()));
                return;
            }
        }
    }
}
