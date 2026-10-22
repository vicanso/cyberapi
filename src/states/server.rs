//! The desktop's bridge (`docs/server-sync.md` §1, steps 5–6): the one
//! address it signs in to, who is signed in there, and which of this
//! machine's projects are copies of whose server projects. The session's
//! token is kept in a file of its own (`crate::server_local`), out of the
//! database and its backups, with the account it is — so that account's
//! copies show and are edited offline too. Another account's copies, or
//! any while signed out, stay in the database unseen.

use super::{AppStore, run_store};
use crate::server_local::{self, ServerFile};
use cyberapi_core::directory::Me;
use cyberapi_core::sync::CopyBinding;
use cyberapi_core::{AccountId, CollectionId};
use cyberapi_http::HttpEngine;
use cyberapi_store::{DirectoryStore, ServerCopyStore, Storage};
use cyberapi_store_bridge::{
    BridgeStore, EngineTransport, LoginForm, SignInError, bridge_address, session_me, sign_in, sign_out,
};
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
    /// The machine's copies of server projects, and every account's
    /// bindings once read.
    copies: Option<Arc<dyn ServerCopyStore>>,
    bound: Option<Vec<CopyBinding>>,
    /// The kept session was asked about once.
    checked: bool,
}

impl EventEmitter<ServerEvent> for ServerState {}

impl ServerState {
    /// What the file kept; a kept session is asked about at once, and the
    /// copies' bindings read.
    pub fn new(engine: Arc<dyn HttpEngine>, copies: Option<Arc<dyn ServerCopyStore>>, cx: &mut Context<Self>) -> Self {
        let file = server_local::read();
        let mut state = Self {
            engine,
            file,
            status: ServerStatus::SignedOut,
            signing_in: false,
            copies,
            bound: None,
            checked: false,
        };
        state.check(cx);
        state.reload_copies(cx);
        state
    }

    /// Who is signed in and which projects are whose are known: what may
    /// open is decided.
    pub fn ready(&self) -> bool {
        self.checked && (self.copies.is_none() || self.bound.is_some())
    }

    /// The project waiting to open may now. After this update: opening
    /// reads this state.
    fn became_ready(&mut self, was: bool, cx: &mut Context<Self>) {
        if !was
            && self.ready()
            && let Some(store) = cx.try_global::<AppStore>()
        {
            let app = store.state().clone();
            cx.defer(move |cx| app.update(cx, |app, cx| app.open_waiting(cx)));
        }
    }

    /// The account signed in — or whose session could not be asked about
    /// yet, whose copies are still theirs to work on.
    pub fn account(&self) -> Option<AccountId> {
        self.file.token.as_ref()?;
        self.file.account.as_deref().map(AccountId::new)
    }

    /// The server's stores, as the signed-in account.
    pub fn storage(&self) -> Option<Storage> {
        Some(BridgeStore::new(Arc::new(self.transport()?)).storage())
    }

    /// The server's accounts and teams (§9.4), as the signed-in account.
    pub fn directory(&self) -> Option<Arc<dyn DirectoryStore>> {
        Some(BridgeStore::new(Arc::new(self.transport()?)).directory())
    }

    /// The application's copy of who is signed in follows: the menu's
    /// account, the cards' teams.
    fn tell_app(&self, cx: &mut Context<Self>) {
        let me = match &self.status {
            ServerStatus::SignedIn(me) => Some(me.clone()),
            ServerStatus::SignedOut => None,
            ServerStatus::Checking | ServerStatus::Unreachable(_) => return,
        };
        if let Some(store) = cx.try_global::<AppStore>() {
            let app = store.state().clone();
            cx.defer(move |cx| app.update(cx, |app, cx| app.set_me(me, cx)));
        }
    }

    pub fn copies(&self) -> Option<&Arc<dyn ServerCopyStore>> {
        self.copies.as_ref()
    }

    /// Reads the bindings again: copies were made, synced or deleted.
    pub fn reload_copies(&mut self, cx: &mut Context<Self>) {
        let Some(copies) = &self.copies else { return };
        run_store(cx, copies.copies(), |this, result, cx| {
            let was = this.ready();
            match result {
                Ok(bound) => this.bound = Some(bound),
                Err(error) => {
                    tracing::warn!(%error, "the copies of server projects could not be read");
                    this.bound.get_or_insert_default();
                }
            }
            this.became_ready(was, cx);
            cx.notify();
        });
    }

    /// Bindings the database has now, before the next read: a copy just
    /// made, synced or edited opens and shows as one at once.
    pub fn note_bound(&mut self, bindings: Vec<CopyBinding>, cx: &mut Context<Self>) {
        let bound = self.bound.get_or_insert_default();
        for binding in bindings {
            bound.retain(|found| found.collection != binding.collection);
            bound.push(binding);
        }
        cx.notify();
    }

    /// A copy deleted, with its binding.
    pub fn forget_bound(&mut self, id: &CollectionId, cx: &mut Context<Self>) {
        if let Some(bound) = &mut self.bound {
            bound.retain(|found| &found.collection != id);
        }
        cx.notify();
    }

    /// Every account's bindings, once read.
    pub fn bound(&self) -> &[CopyBinding] {
        self.bound.as_deref().unwrap_or_default()
    }

    /// The binding of `id`, whoever's.
    pub fn binding(&self, id: &CollectionId) -> Option<&CopyBinding> {
        self.bound.as_ref()?.iter().find(|binding| &binding.collection == id)
    }

    /// The signed-in account's copy `id`, if it is one.
    pub fn mine(&self, id: &CollectionId) -> Option<&CopyBinding> {
        let account = self.account()?;
        self.binding(id).filter(|binding| binding.account == account)
    }

    /// A copy that is not the signed-in account's: not shown, not opened.
    pub fn hides(&self, id: &CollectionId) -> bool {
        self.binding(id).is_some() && self.mine(id).is_none()
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
            let was = self.ready();
            self.status = ServerStatus::SignedOut;
            self.checked = true;
            self.became_ready(was, cx);
            return cx.notify();
        };
        self.status = ServerStatus::Checking;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let answer = cx.background_spawn(async move { session_me(&transport).await }).await;
            let _ = this.update(cx, |this, cx| {
                let was = this.ready();
                this.status = match answer {
                    Ok(Some(me)) => {
                        let account = Some(me.account.id.as_str().to_string());
                        if this.file.account != account {
                            this.file.account = account;
                            server_local::write(&this.file);
                        }
                        ServerStatus::SignedIn(me)
                    }
                    Ok(None) => {
                        this.forget_token();
                        ServerStatus::SignedOut
                    }
                    Err(reason) => ServerStatus::Unreachable(reason),
                };
                this.checked = true;
                this.became_ready(was, cx);
                this.tell_app(cx);
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
                            account: None,
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
        self.tell_app(cx);
        cx.notify();
    }

    fn forget_token(&mut self) {
        let account = self.file.account.take();
        if self.file.token.take().is_some() || account.is_some() {
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
        let state = cx.new(|cx| ServerState::new(engine, None, cx));
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
            (kept.address.as_str(), kept.token.as_deref(), kept.account.as_deref()),
            ("https://bridge.test", Some("tok"), Some("a1"))
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
        assert_eq!(
            (kept.address.as_str(), kept.token, kept.account),
            ("https://bridge.test", None, None)
        );
        let seen = bridge.seen.lock().map(|seen| seen.clone()).unwrap_or_default();
        assert!(
            seen.iter()
                .any(|(url, bearer)| url == "https://bridge.test/v1/logout" && bearer.as_deref() == Some("Bearer tok"))
        );
    }
}
