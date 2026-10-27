//! C4: the loopback listener an authorization-code sign-in redirects to
//! (RFC 8252). One `GET /callback` on `127.0.0.1`, then the code — or why
//! the authorization server refused. Dropping it stops listening. The
//! desktop opens the browser; this only answers the redirect.

use crate::native::AbortOnDrop;
use bytes::Bytes;
use futures::channel::oneshot;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::runtime::Handle;

/// A listener waiting for one callback.
pub struct CallbackListener {
    port: u16,
    done: oneshot::Receiver<Result<String, String>>,
    _accepting: AbortOnDrop,
}

impl CallbackListener {
    /// Listens on `127.0.0.1:port` (`0` for any free port) and completes
    /// when `/callback` arrives with `state`. `Err` is why it could not
    /// listen.
    pub async fn start(runtime: Handle, port: u16, state: String) -> Result<Self, String> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let listener = runtime
            .spawn(async move { TcpListener::bind(address).await })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| format!("{address}: {error}"))?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let (tx, done) = oneshot::channel();
        let slot = Arc::new(Mutex::new(Some(tx)));
        let accepting = runtime.spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (slot, state) = (slot.clone(), state.clone());
                tokio::spawn(async move {
                    let service = service_fn(move |request| {
                        let (slot, state) = (slot.clone(), state.clone());
                        async move { Ok::<_, Infallible>(answer(request, &slot, &state)) }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Ok(Self {
            port: address.port(),
            done,
            _accepting: AbortOnDrop(accepting.abort_handle()),
        })
    }

    /// The port that was bound.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The bound port, the code when it arrives, and a guard whose drop
    /// stops the listener.
    pub fn into_wait(self) -> (u16, oneshot::Receiver<Result<String, String>>, Box<dyn Send>) {
        (self.port, self.done, Box::new(self._accepting))
    }
}

fn answer(
    request: hyper::Request<Incoming>,
    slot: &Mutex<Option<oneshot::Sender<Result<String, String>>>>,
    state: &str,
) -> hyper::Response<Full<Bytes>> {
    if request.uri().path() != "/callback" {
        return page(404, "Not found.");
    }
    if request.method() != hyper::Method::GET {
        return page(405, "Method not allowed.");
    }
    let query = query_of(request.uri().query().unwrap_or(""));
    let result = match (query.get("error"), query.get("code"), query.get("state")) {
        (Some(error), _, _) => Err(match query.get("error_description") {
            Some(description) => format!("OAuth 2.0: {error}: {description}"),
            None => format!("OAuth 2.0: {error}"),
        }),
        (_, Some(code), Some(got)) if got == state && !code.is_empty() => Ok(code.clone()),
        (_, _, Some(_)) => Err("OAuth 2.0: the callback's state did not match".into()),
        _ => Err("OAuth 2.0: the callback had no code".into()),
    };
    let message = match &result {
        Ok(_) => "You can close this window. 可以关掉这个窗口了。".to_string(),
        Err(why) => why.clone(),
    };
    if let Some(tx) = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
        let _ = tx.send(result);
    }
    page(200, &message)
}

fn query_of(query: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes()).into_owned().collect()
}

fn page(status: u16, message: &str) -> hyper::Response<Full<Bytes>> {
    let body = format!(
        "<!DOCTYPE html><meta charset=\"utf-8\"><title>Cyber API</title><p>{}</p>",
        escape(message)
    );
    hyper::Response::builder()
        .status(status)
        .header("content-type", "text/html; charset=utf-8")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new())))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt as _;

    async fn get(port: u16, path: &str) {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.expect("write");
    }

    #[tokio::test]
    async fn the_callback_returns_the_code_when_the_state_matches() {
        let listener = CallbackListener::start(Handle::current(), 0, "s1".into())
            .await
            .expect("listen");
        let (port, done, stop) = listener.into_wait();
        get(port, "/callback?code=abc&state=s1").await;
        assert_eq!(done.await.expect("sent").expect("code"), "abc");
        drop(stop);
    }

    #[tokio::test]
    async fn a_callback_that_does_not_match_is_refused() {
        let listener = CallbackListener::start(Handle::current(), 0, "s1".into())
            .await
            .expect("listen");
        let (port, done, stop) = listener.into_wait();
        get(
            port,
            "/callback?error=access_denied&error_description=no%20thanks&state=s1",
        )
        .await;
        assert_eq!(
            done.await.expect("sent").expect_err("refused"),
            "OAuth 2.0: access_denied: no thanks"
        );
        drop(stop);
    }
}
