//! C4 / R6: the loopback listener an authorization-code or implicit OAuth
//! sign-in redirects to (RFC 8252). Drops stop listening. The desktop opens the
//! browser; this answers the redirect and hands back either the authorization
//! `code` or the implicit `#fragment` token.

use crate::native::AbortOnDrop;
use bytes::Bytes;
use futures::channel::oneshot;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::runtime::Handle;

const FRAGMENT_LIMIT: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackKind {
    Code,
    Implicit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallbackOutcome {
    Code(String),
    Token(cyberapi_core::oauth::Token),
}

type OutcomeSlot = Arc<Mutex<Option<oneshot::Sender<Result<CallbackOutcome, String>>>>>;

/// A listener waiting for one callback.
pub struct CallbackListener {
    port: u16,
    done: oneshot::Receiver<Result<CallbackOutcome, String>>,
    _accepting: AbortOnDrop,
}

impl CallbackListener {
    /// Listens on `127.0.0.1:port` (`0` for any free port) and completes
    /// when `/callback` (or `/callback/fragment` for [`CallbackKind::Implicit`])
    /// arrives with matching `state`. `Err` is why it could not listen.
    pub async fn start(runtime: Handle, port: u16, state: String, kind: CallbackKind) -> Result<Self, String> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let listener = runtime
            .spawn(async move { TcpListener::bind(address).await })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| format!("{address}: {error}"))?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let (tx, done) = oneshot::channel();
        let slot: OutcomeSlot = Arc::new(Mutex::new(Some(tx)));
        let accepting = runtime.spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (slot, state) = (slot.clone(), state.clone());
                tokio::spawn(async move {
                    let service = service_fn(move |request| {
                        let (slot, state) = (slot.clone(), state.clone());
                        async move { Ok::<_, Infallible>(answer(request, slot, state, kind).await) }
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

    /// The bound port, the outcome when it arrives, and a guard whose drop
    /// stops the listener.
    pub fn into_wait(self) -> (u16, oneshot::Receiver<Result<CallbackOutcome, String>>, Box<dyn Send>) {
        (self.port, self.done, Box::new(self._accepting))
    }
}

async fn answer(
    request: hyper::Request<Incoming>,
    slot: OutcomeSlot,
    state: String,
    kind: CallbackKind,
) -> hyper::Response<Full<Bytes>> {
    let path = request.uri().path().to_string();
    match (kind, request.method(), path.as_str()) {
        (CallbackKind::Code, &hyper::Method::GET, "/callback") => {
            let query = query_of(request.uri().query().unwrap_or(""));
            let result = match (query.get("error"), query.get("code"), query.get("state")) {
                (Some(error), _, _) => Err(match query.get("error_description") {
                    Some(description) => format!("OAuth 2.0: {error}: {description}"),
                    None => format!("OAuth 2.0: {error}"),
                }),
                (_, Some(code), Some(got)) if got == &state && !code.is_empty() => {
                    Ok(CallbackOutcome::Code(code.clone()))
                }
                (_, _, Some(_)) => Err("OAuth 2.0: the callback's state did not match".into()),
                _ => Err("OAuth 2.0: the callback had no code".into()),
            };
            let message = match &result {
                Ok(_) => "You can close this window. 可以关掉这个窗口了。".to_string(),
                Err(why) => why.clone(),
            };
            deliver(&slot, result);
            page(200, &message)
        }
        (CallbackKind::Implicit, &hyper::Method::GET, "/callback") => {
            let query = query_of(request.uri().query().unwrap_or(""));
            if let Some(error) = query.get("error") {
                let message = match query.get("error_description") {
                    Some(description) => format!("OAuth 2.0: {error}: {description}"),
                    None => format!("OAuth 2.0: {error}"),
                };
                deliver(&slot, Err(message.clone()));
                return page(200, &message);
            }
            implicit_bridge_page()
        }
        (CallbackKind::Implicit, &hyper::Method::POST, "/callback/fragment") => {
            let body = match Limited::new(request.into_body(), FRAGMENT_LIMIT).collect().await {
                Ok(collected) => collected.to_bytes(),
                Err(error) => {
                    let message = format!("OAuth 2.0: could not read callback fragment: {error}");
                    deliver(&slot, Err(message.clone()));
                    return plain(400, &message);
                }
            };
            let fragment = String::from_utf8_lossy(&body);
            let result = cyberapi_core::oauth::read_implicit_fragment(&fragment, &state).map(CallbackOutcome::Token);
            let message = match &result {
                Ok(_) => "You can close this window. 可以关掉这个窗口了。".to_string(),
                Err(why) => why.clone(),
            };
            let status = if result.is_ok() { 200 } else { 400 };
            deliver(&slot, result);
            plain(status, &message)
        }
        (_, method, "/callback" | "/callback/fragment") if method != hyper::Method::GET => {
            page(405, "Method not allowed.")
        }
        _ => page(404, "Not found."),
    }
}

fn deliver(slot: &OutcomeSlot, result: Result<CallbackOutcome, String>) {
    if let Some(tx) = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
        let _ = tx.send(result);
    }
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
        .header("cache-control", "no-store")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new())))
}

fn implicit_bridge_page() -> hyper::Response<Full<Bytes>> {
    let body = concat!(
        "<!DOCTYPE html><meta charset=\"utf-8\"><title>Cyber API</title>",
        "<p id=\"msg\">Completing sign-in…</p>",
        "<script>",
        "const msg = document.getElementById('msg');",
        "const frag = window.location.hash ? window.location.hash.slice(1) : '';",
        "if (!frag) {",
        "  msg.textContent = 'OAuth 2.0: the callback had no URL fragment.';",
        "} else {",
        "  history.replaceState(null, '', '/callback');",
        "  fetch('/callback/fragment', {",
        "    method: 'POST',",
        "    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },",
        "    body: frag",
        "  }).then(r => r.text()).then(t => { msg.textContent = t; })",
        "    .catch(e => { msg.textContent = 'OAuth 2.0: ' + e; });",
        "}",
        "</script>"
    );
    hyper::Response::builder()
        .status(200)
        .header("content-type", "text/html; charset=utf-8")
        .header("cache-control", "no-store")
        .body(Full::new(Bytes::from_static(body.as_bytes())))
        .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new())))
}

fn plain(status: u16, message: &str) -> hyper::Response<Full<Bytes>> {
    hyper::Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .header("cache-control", "no-store")
        .body(Full::new(Bytes::from(message.as_bytes().to_vec())))
        .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new())))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    async fn get(port: u16, path: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut buf = String::new();
        stream.read_to_string(&mut buf).await.expect("read");
        buf
    }

    async fn post_form(port: u16, path: &str, body: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut buf = String::new();
        stream.read_to_string(&mut buf).await.expect("read");
        buf
    }

    #[tokio::test]
    async fn the_callback_returns_the_code_when_the_state_matches() {
        let listener = CallbackListener::start(Handle::current(), 0, "s1".into(), CallbackKind::Code)
            .await
            .expect("listen");
        let (port, done, stop) = listener.into_wait();
        get(port, "/callback?code=abc&state=s1").await;
        assert_eq!(done.await.expect("sent"), Ok(CallbackOutcome::Code("abc".into())));
        drop(stop);
    }

    #[tokio::test]
    async fn a_callback_that_does_not_match_is_refused() {
        let listener = CallbackListener::start(Handle::current(), 0, "s1".into(), CallbackKind::Code)
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

    #[tokio::test]
    async fn implicit_callback_serves_bridge_page_and_reads_posted_fragment() {
        let listener = CallbackListener::start(Handle::current(), 0, "st-9".into(), CallbackKind::Implicit)
            .await
            .expect("listen");
        let (port, done, stop) = listener.into_wait();
        let html = get(port, "/callback").await;
        assert!(html.contains("fetch('/callback/fragment'"), "{html}");
        assert!(html.contains("history.replaceState"), "{html}");

        let reply = post_form(
            port,
            "/callback/fragment",
            "access_token=tok-xyz&token_type=Bearer&expires_in=1200&state=st-9",
        )
        .await;
        assert!(reply.contains("200 OK"), "{reply}");
        assert_eq!(
            done.await.expect("sent"),
            Ok(CallbackOutcome::Token(cyberapi_core::oauth::Token {
                access_token: "tok-xyz".into(),
                expires_in: Some(1200),
                refresh_token: None,
            }))
        );
        drop(stop);

        let listener = CallbackListener::start(Handle::current(), 0, "st-9".into(), CallbackKind::Implicit)
            .await
            .expect("listen");
        let (port, done, stop) = listener.into_wait();
        let bad = post_form(port, "/callback/fragment", "access_token=tok&state=wrong").await;
        assert!(bad.contains("400 Bad Request"), "{bad}");
        assert_eq!(
            done.await.expect("sent").expect_err("wrong state"),
            "OAuth 2.0: the callback's state did not match"
        );
        drop(stop);
    }
}
