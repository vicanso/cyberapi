//! The desktop's bridge (`docs/server-sync.md` §1, step 5): the one
//! address it signs in to, and who is signed in there. The session's token
//! is kept in a file of its own (`crate::server_local`), out of the
//! database and its backups. Signing in changes nothing about the projects
//! of this machine.

use crate::server_local::{self, ServerFile};
use cyberapi_core::directory::Me;
use cyberapi_http::HttpEngine;
use cyberapi_store_bridge::{EngineTransport, LoginForm, SignInError, bridge_address, session_me, sign_in, sign_out};
use gpui_kit::{AppContext as _, Context, EventEmitter};
use std::sync::Arc;

/// Where the session stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerStatus {
    SignedOut,
    /// Asking the bridge who the kept session is.
    Checking,
    SignedIn(Me),
    /// The bridge did not answer; the session is kept for the next try.
    Unreachable(String),
}

/// Why a sign-in did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignInFailure {
    BadAddress,
    Login(SignInError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerEvent {
    SignInFailed(SignInFailure),
    SignedIn,
}

pub struct ServerState {
    engine: Arc<dyn HttpEngine>,
    file: ServerFile,
    status: ServerStatus,
    /// A sign-in on its way.
    signing_in: bool,
}

impl EventEmitter<ServerEvent> for ServerState {}

impl ServerState {
    /// What the file kept; a kept session is asked about at once.
    pub fn new(engine: Arc<dyn HttpEngine>, cx: &mut Context<Self>) -> Self {
        let file = server_local::read();
        let mut state = Self {
            engine,
            file,
            status: ServerStatus::SignedOut,
            signing_in: false,
        };
        state.check(cx);
        state
    }

    pub fn address(&self) -> &str {
        &self.file.address
    }

    pub fn status(&self) -> &ServerStatus {
        &self.status
    }

    pub fn signing_in(&self) -> bool {
        self.signing_in
    }

    fn transport(&self) -> Option<EngineTransport> {
        let token = self.file.token.clone()?;
        Some(EngineTransport::new(
            self.engine.clone(),
            self.file.address.clone(),
            Some(token),
        ))
    }

    /// Asks the bridge who the kept session is. One it no longer knows is
    /// forgotten; one it could not be asked about stays.
    pub fn check(&mut self, cx: &mut Context<Self>) {
        let Some(transport) = self.transport() else {
            self.status = ServerStatus::SignedOut;
            return cx.notify();
        };
        self.status = ServerStatus::Checking;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let answer = cx.background_spawn(async move { session_me(&transport).await }).await;
            let _ = this.update(cx, |this, cx| {
                this.status = match answer {
                    Ok(Some(me)) => ServerStatus::SignedIn(me),
                    Ok(None) => {
                        this.forget_token();
                        ServerStatus::SignedOut
                    }
                    Err(reason) => ServerStatus::Unreachable(reason),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Signs in to the bridge at `address`, in place of any other.
    pub fn sign_in(&mut self, address: &str, name: String, password: String, cx: &mut Context<Self>) {
        if self.signing_in {
            return;
        }
        let Some(address) = bridge_address(address) else {
            return cx.emit(ServerEvent::SignInFailed(SignInFailure::BadAddress));
        };
        self.signing_in = true;
        cx.notify();
        let engine = self.engine.clone();
        let form = LoginForm {
            name: name.trim().to_string(),
            password,
            bearer: true,
        };
        let to = address.clone();
        cx.spawn(async move |this, cx| {
            let answer = cx.background_spawn(sign_in(engine, to, form)).await;
            let _ = this.update(cx, |this, cx| {
                this.signing_in = false;
                match answer {
                    Ok(token) => {
                        this.file = ServerFile {
                            address,
                            token: Some(token),
                        };
                        server_local::write(&this.file);
                        this.check(cx);
                        cx.emit(ServerEvent::SignedIn);
                    }
                    Err(error) => cx.emit(ServerEvent::SignInFailed(SignInFailure::Login(error))),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Ends the session, here at once and at the bridge when it answers.
    /// The address stays, for the next sign-in.
    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        if let Some(transport) = self.transport() {
            cx.background_spawn(async move {
                if let Err(reason) = sign_out(&transport).await {
                    tracing::warn!(%reason, "the bridge did not end the session");
                }
            })
            .detach();
        }
        self.forget_token();
        self.status = ServerStatus::SignedOut;
        cx.notify();
    }

    fn forget_token(&mut self) {
        if self.file.token.take().is_some() {
            server_local::write(&self.file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::response::Exchange;
    use cyberapi_http::{PreparedRequest, SendFuture};
    use gpui_kit::{Entity, TestAppContext};
    use std::sync::Mutex;

    /// A bridge that knows one password and one session.
    #[derive(Default)]
    struct Bridge {
        seen: Mutex<Vec<(String, Option<String>)>>,
    }

    impl HttpEngine for Bridge {
        fn send(&self, request: PreparedRequest) -> SendFuture {
            let bearer = request.header("authorization").map(str::to_string);
            if let Ok(mut seen) = self.seen.lock() {
                seen.push((request.url.clone(), bearer.clone()));
            }
            let body = String::from_utf8_lossy(&request.body).into_owned();
            let (status, body) = if request.url.ends_with("/v1/login") {
                if body.contains("\"password\":\"right\"") && body.contains("\"bearer\":true") {
                    (200, r#"{"token":"tok"}"#.to_string())
                } else {
                    (401, String::new())
                }
            } else if request.url.ends_with("/v1/store") && bearer.as_deref() == Some("Bearer tok") {
                let me = r#"{"account":{"id":"a1","name":"alice","displayName":"Alice","admin":false,"disabled":false},"teams":[]}"#;
                (200, format!(r#"{{"ok":{me}}}"#))
            } else if request.url.ends_with("/v1/logout") {
                (204, String::new())
            } else {
                (401, String::new())
            };
            Box::pin(async move {
                Ok(Exchange {
                    status,
                    body: body.into_bytes(),
                    ..Default::default()
                })
            })
        }
    }

    fn state(bridge: &Arc<Bridge>, cx: &mut TestAppContext) -> Entity<ServerState> {
        let engine: Arc<dyn HttpEngine> = bridge.clone();
        let state = cx.new(|cx| ServerState::new(engine, cx));
        cx.run_until_parked();
        state
    }

    #[gpui_kit::test]
    async fn a_session_is_kept_until_signed_out(cx: &mut TestAppContext) {
        let bridge = Arc::new(Bridge::default());
        let server = state(&bridge, cx);
        assert_eq!(
            server.read_with(cx, |state, _| state.status().clone()),
            ServerStatus::SignedOut
        );

        let failures = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let heard = failures.clone();
        cx.update(|cx| {
            cx.subscribe(&server, move |_, event: &ServerEvent, _| {
                if let ServerEvent::SignInFailed(failure) = event {
                    heard.borrow_mut().push(failure.clone());
                }
            })
            .detach();
        });
        server.update(cx, |state, cx| {
            state.sign_in("bridge.test", "alice".into(), "right".into(), cx)
        });
        server.update(cx, |state, cx| {
            state.sign_in("https://bridge.test/", "alice".into(), "wrong".into(), cx)
        });
        cx.run_until_parked();
        assert_eq!(
            failures.borrow().as_slice(),
            [SignInFailure::BadAddress, SignInFailure::Login(SignInError::WrongLogin)]
        );

        server.update(cx, |state, cx| {
            state.sign_in("https://bridge.test/", " alice ".into(), "right".into(), cx)
        });
        cx.run_until_parked();
        let signed_in = |cx: &mut TestAppContext, server: &Entity<ServerState>| {
            server.read_with(cx, |state, _| match state.status() {
                ServerStatus::SignedIn(me) => Some(me.account.name.clone()),
                _ => None,
            })
        };
        assert_eq!(signed_in(cx, &server), Some("alice".to_string()));
        let kept = server_local::read();
        assert_eq!(
            (kept.address.as_str(), kept.token.as_deref()),
            ("https://bridge.test", Some("tok"))
        );

        // The next start asks about the kept session.
        let again = state(&bridge, cx);
        assert_eq!(signed_in(cx, &again), Some("alice".to_string()));

        again.update(cx, |state, cx| state.sign_out(cx));
        cx.run_until_parked();
        assert_eq!(
            again.read_with(cx, |state, _| state.status().clone()),
            ServerStatus::SignedOut
        );
        let kept = server_local::read();
        assert_eq!((kept.address.as_str(), kept.token), ("https://bridge.test", None));
        let seen = bridge.seen.lock().map(|seen| seen.clone()).unwrap_or_default();
        assert!(
            seen.iter()
                .any(|(url, bearer)| url == "https://bridge.test/v1/logout" && bearer.as_deref() == Some("Bearer tok"))
        );
    }
}
