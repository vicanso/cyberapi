//! E2: a gRPC call whose request body streams, on a connection of its own.
//! It is dialed as any request is (`native::dial`) and then spoken as
//! HTTP/2, prior knowledge on plain HTTP and ALPN on TLS. The body is a
//! stream: the caller sends pieces while the answer's headers, body and
//! trailers come back, which a client-streaming server may withhold until
//! the request is finished. Dropping the outgoing side gives the call up.

use crate::native::{Desktop, dial, pairs, seconds, timed_out};
use crate::{CallIn, CallOut, LiveCall, PreparedRequest, SendError, TargetPolicy};
use bytes::Bytes;
use cyberapi_core::prefs::HttpVersion;
use futures::StreamExt as _;
use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use http_body_util::{BodyExt as _, StreamBody};
use hyper::body::Frame;
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::convert::Infallible;
use tokio::task::AbortHandle;
use url::Url;

/// The body of a streaming call: frames the caller sends, one at a time.
type CallBody = StreamBody<UnboundedReceiver<Result<Frame<Bytes>, Infallible>>>;

/// Headers HTTP/2 does not carry, plus a length: the body is not one
/// length, it is a stream.
const SKIPPED: [&str; 7] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
    "content-length",
    "host",
];

pub(crate) async fn open(
    mut request: PreparedRequest,
    policy: TargetPolicy,
    desktop: Desktop,
) -> Result<LiveCall, SendError> {
    // A streaming gRPC body is HTTP/2, even when the settings say otherwise.
    request.http_version = HttpVersion::Http2;
    let invalid = |message: String| SendError::Invalid { message };
    let url = Url::parse(&request.url).map_err(|error| invalid(format!("{}: {error}", request.url)))?;
    let dialed = dial(&request, &url, &policy, &desktop, false).await?;
    if !dialed.h2 {
        return Err(SendError::Http {
            message: "the server did not agree to HTTP/2".into(),
        });
    }
    let mut stats = dialed.stats;
    stats.version = "HTTP/2".into();
    let http_error = |error: hyper::Error| SendError::Http {
        message: error.to_string(),
    };
    let io = TokioIo::new(dialed.stream);
    let (sender, connection) = tokio::time::timeout(
        seconds(request.timeouts.write),
        hyper::client::conn::http2::handshake::<_, _, CallBody>(TokioExecutor::new(), io),
    )
    .await
    .map_err(|_| timed_out("HTTP/2", request.timeouts.write))?
    .map_err(http_error)?;

    let (outgoing, out_rx) = mpsc::unbounded();
    let (incoming_tx, incoming) = mpsc::unbounded();
    let (frame_tx, frame_rx) = mpsc::unbounded();
    let request = streaming_request(&request, &url, frame_rx)?;
    // The connection and the sender both stay until the body has been read:
    // dropping the sender before that drops the stream.
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    let reading = tokio::spawn(read_answer(sender, request, incoming_tx));
    tokio::spawn(write_body(
        out_rx,
        frame_tx,
        connection.abort_handle(),
        reading.abort_handle(),
    ));
    // The handshake's own frames are not this call's.
    stats.total = stats.dns + stats.tcp + stats.tls;
    Ok(LiveCall {
        stats,
        outgoing,
        incoming,
    })
}

/// Forwards what the caller sends. [`CallOut::Finish`] ends the body and
/// leaves the answer to come; anything else, including the caller going
/// away, gives the call up.
async fn write_body(
    mut outgoing: UnboundedReceiver<CallOut>,
    frames: UnboundedSender<Result<Frame<Bytes>, Infallible>>,
    connection: AbortHandle,
    reading: AbortHandle,
) {
    let mut finish = false;
    while let Some(out) = outgoing.next().await {
        match out {
            CallOut::Data(bytes) => {
                if frames.unbounded_send(Ok(Frame::data(Bytes::from(bytes)))).is_err() {
                    break;
                }
            }
            CallOut::Finish => {
                finish = true;
                break;
            }
            CallOut::Cancel => break,
        }
    }
    drop(frames);
    if !finish {
        connection.abort();
        reading.abort();
    }
}

/// The answer, as it comes, until it ends or the call fails. `sender` is
/// held until then: it owns the stream the body is read from.
async fn read_answer(
    mut sender: hyper::client::conn::http2::SendRequest<CallBody>,
    request: http::Request<CallBody>,
    incoming: UnboundedSender<CallIn>,
) {
    let response = match sender.send_request(request).await {
        Ok(response) => response,
        Err(error) => {
            let _ = incoming.unbounded_send(CallIn::Failed(error.to_string()));
            return;
        }
    };
    let status = response.status().as_u16();
    let headers = pairs(response.headers());
    if incoming.unbounded_send(CallIn::Head { status, headers }).is_err() {
        return;
    }
    let mut body = response.into_body();
    loop {
        match body.frame().await {
            None => {
                let _ = incoming.unbounded_send(CallIn::Ended);
                break;
            }
            Some(Err(error)) => {
                let _ = incoming.unbounded_send(CallIn::Failed(error.to_string()));
                break;
            }
            Some(Ok(frame)) => match frame.into_data() {
                Ok(data) => {
                    if !data.is_empty() && incoming.unbounded_send(CallIn::Data(data.to_vec())).is_err() {
                        break;
                    }
                }
                Err(frame) => {
                    if let Ok(trailers) = frame.into_trailers()
                        && incoming.unbounded_send(CallIn::Trailers(pairs(&trailers))).is_err()
                    {
                        break;
                    }
                }
            },
        }
    }
    drop(sender);
}

/// The request over HTTP/2, its body the stream `frames`. No
/// `Content-Length`: the body ends when the stream does. No
/// `Accept-Encoding` either, unless the request names one — a gRPC body
/// is framed messages, not a compressed HTTP body.
fn streaming_request(
    request: &PreparedRequest,
    url: &Url,
    frames: UnboundedReceiver<Result<Frame<Bytes>, Infallible>>,
) -> Result<http::Request<CallBody>, SendError> {
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
    let mut builder = http::Request::builder()
        .method(
            http::Method::from_bytes(request.method.as_str().as_bytes()).map_err(|error| invalid(error.to_string()))?,
        )
        .version(http::Version::HTTP_2)
        .uri(format!("{}://{authority}{target}", url.scheme()));
    for (name, value) in &request.headers {
        let lower = name.trim().to_ascii_lowercase();
        if SKIPPED.contains(&lower.as_str()) || (lower == "te" && !value.trim().eq_ignore_ascii_case("trailers")) {
            continue;
        }
        let name = http::HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| invalid(format!("{name:?} is not a header name")))?;
        let value = http::HeaderValue::from_bytes(value.as_bytes())
            .map_err(|_| invalid(format!("the value of {name} is not a header value")))?;
        builder = builder.header(name, value);
    }
    builder
        .body(StreamBody::new(frames))
        .map_err(|error| invalid(error.to_string()))
}
