//! Sending a request (F5.5, `docs/architecture.md` §5).
//!
//! [`HttpEngine`] is the seam: the desktop and the bridge send with
//! [`NativeEngine`]; the browser build cannot (CORS, forbidden headers, no
//! timings), so it hands the request to the bridge, which sends it with the
//! same engine. What goes in is a [`PreparedRequest`] — variables, functions
//! and headers already settled by `cyberapi_core::send` — and what comes back
//! is an [`Exchange`] or a [`SendError`].
//!
//! Dropping the future cancels the request (#2: the old app's "abort" only
//! dropped the answer while the request went on).
//!
//! An event stream (`text/event-stream`), or an answer the request says is
//! a stream (`PreparedRequest::streams`, a gRPC call's), can be watched instead
//! ([`HttpEngine::send_watched`]): its head and each piece of its body are
//! told as they come, and it is read until the server ends it, it goes
//! quiet for the read timeout, it reaches the body limit or the reader
//! stops it — what came by then is the exchange, with how it ended.
//!
//! E1: a WebSocket is opened ([`HttpEngine::open_socket`]) rather than
//! sent: the desktop's engine upgrades an HTTP/1.1 connection and hands
//! back a [`Socket`] to send on and hear from; the others cannot.

#[cfg(not(target_family = "wasm"))]
mod client_tls;
#[cfg(not(target_family = "wasm"))]
mod mock;
#[cfg(not(target_family = "wasm"))]
mod native;
mod policy;
#[cfg(not(target_family = "wasm"))]
mod proxy;
#[cfg(not(target_family = "wasm"))]
mod socket;

pub use cyberapi_core::response::Exchange;
pub use cyberapi_core::send::PreparedRequest;
pub use cyberapi_core::ws::Frame;
#[cfg(not(target_family = "wasm"))]
pub use mock::MockServer;
#[cfg(not(target_family = "wasm"))]
pub use native::NativeEngine;
pub use policy::TargetPolicy;

use cyberapi_core::response::HttpStats;
use futures::FutureExt as _;
use futures::channel::{mpsc, oneshot};
use futures::future::Shared;
use serde::{Deserialize, Serialize};
use snafu::Snafu;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Why there is no exchange. Each carries the underlying message; the kind
/// says which step failed.
#[derive(Clone, Debug, PartialEq, Eq, Snafu, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SendError {
    /// The request itself cannot be sent: not an HTTP URL, a header that is
    /// not one.
    #[snafu(display("invalid request: {message}"))]
    Invalid { message: String },
    #[snafu(display("could not resolve {host}: {message}"))]
    Dns { host: String, message: String },
    #[snafu(display("could not connect: {message}"))]
    Connect { message: String },
    #[snafu(display("TLS failed: {message}"))]
    Tls { message: String },
    /// The server's certificate did not verify: self-signed, expired, for
    /// another name. The settings can take it anyway (F14.9).
    #[snafu(display("untrusted certificate: {message}"))]
    Certificate { message: String },
    #[snafu(display("no answer within {seconds}s ({phase})"))]
    Timeout { phase: String, seconds: u32 },
    #[snafu(display("{message}"))]
    Http { message: String },
    /// The bridge will not reach this address (`TargetPolicy`). `target`
    /// is the host as written, with the address it resolved to when it is
    /// a name: `api.internal (10.1.2.3)`.
    #[snafu(display(
        "the server does not send requests to {target}; its administrator can allow that address with --allow-target"
    ))]
    Forbidden { target: String },
    /// The browser build could not reach the bridge.
    #[snafu(display("the bridge did not answer: {message}"))]
    Bridge { message: String },
    /// Redirects were followed and led to another, past the limit: a loop,
    /// most likely (F14.8).
    #[snafu(display("stopped after {count} redirects"))]
    Redirects { count: usize },
    /// The proxy (#60) could not be used: a setting that does not read, or
    /// a proxy that would not connect.
    #[snafu(display("proxy: {message}"))]
    Proxy { message: String },
    /// The bridge already has as many of this reader's requests on their
    /// way as it takes from one.
    #[snafu(display("{limit} requests are already on their way; send this one when one of them is back"))]
    Busy { limit: usize },
}

impl SendError {
    /// Whether `error`, the text a response keeps, is a
    /// [`SendError::Certificate`].
    pub fn is_untrusted_certificate(error: &str) -> bool {
        error.starts_with("untrusted certificate: ")
    }
}

pub type SendFuture = Pin<Box<dyn Future<Output = Result<Exchange, SendError>> + Send + 'static>>;

/// What a watched send tells as an event stream comes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Progress {
    /// The answer is an event stream: its status and headers, before any
    /// of its body.
    Head {
        status: u16,
        headers: Vec<(String, String)>,
    },
    /// A piece of its body, as it came.
    Body(#[serde(with = "cyberapi_core::send::base64_bytes")] Vec<u8>),
}

/// Fires when the reader wants an event stream ended: sent, or its sender
/// dropped.
pub type Stop = Shared<oneshot::Receiver<()>>;

/// A [`Stop`], and what fires it.
pub fn stop_signal() -> (oneshot::Sender<()>, Stop) {
    let (sender, receiver) = oneshot::channel();
    (sender, receiver.shared())
}

/// How a send is watched: told of an event stream as it comes, and asked
/// to end it.
#[derive(Clone)]
pub struct Watch {
    pub progress: Arc<dyn Fn(Progress) + Send + Sync>,
    pub stop: Stop,
}

/// E1: what goes out on a WebSocket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocketOut {
    Send(Frame),
    /// The reader closes it: the server is told, and its answer awaited.
    Close,
}

/// E1: what comes in on a WebSocket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocketIn {
    Message(Frame),
    /// It closed — the server closed it, or answered the reader's close —
    /// with the code and reason its close said, if it said any.
    Closed {
        code: Option<u16>,
        reason: String,
    },
    /// The connection failed.
    Failed(String),
}

/// E1: an open WebSocket: the handshake's answer, and the two ways of it.
/// Each thing that comes in comes with when, in milliseconds after it
/// opened. Dropping `outgoing` closes it; it ends with a `Closed` or a
/// `Failed`.
pub struct Socket {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub stats: HttpStats,
    pub outgoing: mpsc::UnboundedSender<SocketOut>,
    pub incoming: mpsc::UnboundedReceiver<(SocketIn, u64)>,
}

/// What opening a WebSocket came to: the connection, or the server's
/// answer when it would not upgrade.
pub enum Opened {
    Socket(Socket),
    Refused(Exchange),
}

pub type OpenFuture = Pin<Box<dyn Future<Output = Result<Opened, SendError>> + Send + 'static>>;

/// Sends prepared requests. Dropping the future cancels its request.
pub trait HttpEngine: Send + Sync {
    fn send(&self, request: PreparedRequest) -> SendFuture;

    /// As [`Self::send`]; an answer that is an event stream is told to
    /// `watch` as it comes and read until it ends, goes quiet or is
    /// stopped. An engine that cannot watch sends as `send` does.
    fn send_watched(&self, request: PreparedRequest, watch: Watch) -> SendFuture {
        let _ = watch;
        self.send(request)
    }

    /// E1: a WebSocket opened with `request`. Only the desktop's engine
    /// opens one; a page's bridge does not relay them (yet).
    fn open_socket(&self, request: PreparedRequest) -> OpenFuture {
        let _ = request;
        Box::pin(async {
            Err(SendError::Invalid {
                message: "WebSocket connections are made by the desktop app only".into(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_certificate_failure_is_told_by_its_text() {
        let failure = SendError::Certificate {
            message: "invalid peer certificate: UnknownIssuer".into(),
        };
        assert!(SendError::is_untrusted_certificate(&failure.to_string()));
        let other = SendError::Tls {
            message: "handshake".into(),
        };
        assert!(!SendError::is_untrusted_certificate(&other.to_string()));
    }
}
