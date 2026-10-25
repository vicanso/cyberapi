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
use gpui_kit::{AppContext as _, Context, EventEmitter, Task};
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
    /// The kept session asked about again: the bridge could not be
    /// reached, and the network may come back.
    _retry: Option<Task<()>>,
    /// How many times in a row it could not be: each retry waits longer
    /// (`pacing::retry_after`).
    failures: u32,
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
            _retry: None,
            failures: 0,
        };
        state.check(cx);
        state.reload_copies(cx);
        state
    }

    /// Which projects are whose is known: what may open is decided, by
    /// the account the file keeps. Not by the bridge's answer about the
    /// session — one slow or out of reach held every project shut until it
    /// timed out; a session it turns out to have ended closes its copy then
    /// (`AppState::set_me`).
    pub fn ready(&self) -> bool {
        self.copies.is_none() || self.bound.is_some()
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
            self.status = ServerStatus::SignedOut;
            return cx.notify();
        };
        // Asked again while out of reach, it stays so until it answers.
        if !matches!(self.status, ServerStatus::Unreachable(_)) {
            self.status = ServerStatus::Checking;
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let answer = cx.background_spawn(async move { session_me(&transport).await }).await;
            let _ = this.update(cx, |this, cx| {
                this.failures = match answer {
                    Err(_) => this.failures.saturating_add(1),
                    Ok(_) => 0,
                };
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
                    Err(reason) => {
                        this.retry_later(cx);
                        ServerStatus::Unreachable(reason)
                    }
                };
                this.tell_app(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// What a call to the bridge made elsewhere — a server project's poll,
    /// a sync — said of reaching it: out of reach, the status says so
    /// and the session is asked about again until it answers; an answer
    /// while out of reach asks at once.
    pub fn heard(&mut self, unreachable: Option<&str>, cx: &mut Context<Self>) {
        match (unreachable, &self.status) {
            (Some(reason), ServerStatus::SignedIn(_)) => {
                self.status = ServerStatus::Unreachable(reason.to_string());
                self.failures = 1;
                self.retry_later(cx);
                cx.notify();
            }
            (None, ServerStatus::Unreachable(_)) => self.check(cx),
            _ => {}
        }
    }

    /// The bridge out of reach is asked again a while later, longer after
    /// each failure, and on until it answers: the network back, who is
    /// signed in and their projects are known again without a restart.
    fn retry_later(&mut self, cx: &mut Context<Self>) {
        let wait = crate::pacing::retry_after(self.failures);
        self._retry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                // Signed in or out meanwhile: nothing to ask again.
                if matches!(this.status, ServerStatus::Unreachable(_)) {
                    this.check(cx);
                }
            });
        }));
    }

    /// Reads who is signed in again, staying signed in meanwhile: their
    /// teams change as they make, leave and are added to them, and a new
    /// project's team is picked from these. A session the bridge no longer
    /// knows, or a bridge out of reach, is [`Self::check`]'s to find.
    pub fn reload_me(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.status, ServerStatus::SignedIn(_)) {
            return;
        }
        let Some(transport) = self.transport() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let answer = cx.background_spawn(async move { session_me(&transport).await }).await;
            let _ = this.update(cx, |this, cx| {
                if let (Ok(Some(me)), ServerStatus::SignedIn(_)) = (answer, &this.status) {
                    this.status = ServerStatus::SignedIn(me);
                    this.tell_app(cx);
                    cx.notify();
                }
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
            remember: true,
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
        /// The session's teams, as JSON; none while empty.
        teams: Mutex<String>,
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
                let teams = self.teams.lock().map(|teams| teams.clone()).unwrap_or_default();
                let teams = if teams.is_empty() { "[]".to_string() } else { teams };
                let me = format!(
                    r#"{{"account":{{"id":"a1","name":"alice","displayName":"Alice","admin":false,"disabled":false}},"teams":{teams}}}"#
                );
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

    /// A bridge slow to answer, or out of reach, holds no project shut while
    /// the kept session is asked about: what may open is known from the
    /// copies' bindings and the account the file keeps.
    #[gpui_kit::test]
    async fn projects_open_while_the_kept_session_is_asked_about(cx: &mut TestAppContext) {
        struct Silent;
        impl HttpEngine for Silent {
            fn send(&self, _: PreparedRequest) -> SendFuture {
                Box::pin(std::future::pending())
            }
        }
        server_local::write(&ServerFile {
            address: "https://bridge.test".into(),
            token: Some("tok".into()),
            account: Some("a1".into()),
        });
        let copies: Arc<dyn ServerCopyStore> = Arc::new(cyberapi_store::MemoryStore::new());
        let server = cx.new(|cx| ServerState::new(Arc::new(Silent), Some(copies), cx));
        cx.run_until_parked();
        server.read_with(cx, |state, _| {
            assert_eq!(state.status(), &ServerStatus::Checking, "still asking");
            assert!(state.ready(), "and projects open meanwhile");
            assert_eq!(state.account(), Some(AccountId::new("a1")));
        });
    }

    /// A team made on the server after signing in is one to pick for a new
    /// project at once, not after a restart.
    #[gpui_kit::test]
    async fn the_teams_are_read_again_without_signing_in_again(cx: &mut TestAppContext) {
        let bridge = Arc::new(Bridge::default());
        let server = state(&bridge, cx);
        server.update(cx, |state, cx| {
            state.sign_in("https://bridge.test", "alice".into(), "right".into(), cx)
        });
        cx.run_until_parked();
        let teams = |cx: &mut TestAppContext| {
            server.read_with(cx, |state, _| match state.status() {
                ServerStatus::SignedIn(me) => me.teams.iter().map(|team| team.name.clone()).collect::<Vec<_>>(),
                _ => vec!["not signed in".to_string()],
            })
        };
        assert!(teams(cx).is_empty());

        if let Ok(mut kept) = bridge.teams.lock() {
            *kept = r#"[{"id":"t1","name":"new team","personal":false,"role":"owner"}]"#.into();
        }
        server.update(cx, |state, cx| state.reload_me(cx));
        cx.run_until_parked();
        assert_eq!(teams(cx), ["new team"]);
    }
}
