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

#[cfg(not(target_family = "wasm"))]
mod native;
mod policy;

pub use cyberapi_core::response::Exchange;
pub use cyberapi_core::send::PreparedRequest;
#[cfg(not(target_family = "wasm"))]
pub use native::NativeEngine;
pub use policy::TargetPolicy;

use serde::{Deserialize, Serialize};
use snafu::Snafu;
use std::future::Future;
use std::pin::Pin;

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
    #[snafu(display("no answer within {seconds}s ({phase})"))]
    Timeout { phase: String, seconds: u32 },
    #[snafu(display("{message}"))]
    Http { message: String },
    /// The bridge will not reach this address (`TargetPolicy`).
    #[snafu(display("the server does not send requests to {target}"))]
    Forbidden { target: String },
    /// The browser build could not reach the bridge.
    #[snafu(display("the bridge did not answer: {message}"))]
    Bridge { message: String },
}

pub type SendFuture = Pin<Box<dyn Future<Output = Result<Exchange, SendError>> + Send + 'static>>;

/// Sends prepared requests. Dropping the future cancels its request.
pub trait HttpEngine: Send + Sync {
    fn send(&self, request: PreparedRequest) -> SendFuture;
}
