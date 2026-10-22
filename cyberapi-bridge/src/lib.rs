//! The server of CyberAPI's browser build (`docs/architecture.md` §2.1, §9.5):
//! it signs accounts in, answers the page's store calls on the account's
//! scoped store, and serves the page itself — one origin for all three, so
//! the login cookie can stay `SameSite=Strict`.
//!
//! | Route | |
//! |---|---|
//! | `GET /v1/setup` | whether the first administrator is still to be made |
//! | `POST /v1/setup` | makes it, with the setup token, and signs it in |
//! | `POST /v1/login`, `POST /v1/logout` | the session cookie — or, for a client of its own (the desktop), the session's token, sent back as a bearer token |
//! | `POST /v1/invite` | what an invitation's token opens, while it can be used |
//! | `POST /v1/invite/accept` | makes the account it invites, and signs it in |
//! | `GET /v1/session` | who is signed in, or 401 |
//! | `POST /v1/store` | one store call (`cyberapi_store_bridge::Call`) |
//! | `POST /v1/send` | sends a prepared request (`docs/architecture.md` §5) |
//! | anything else | the page's files ([`static_files`]) |
//!
//! Failed sign-ins, setup tries and invitations that open nothing are counted ([`throttle`]) and turned
//! away with 429 once there are too many; serve the router with
//! `into_make_service_with_connect_info::<SocketAddr>` so they are counted
//! by address too.

mod auth;
pub mod static_files;
mod throttle;

use auth::{hash_password, hash_token, new_token, session_cookie, signed_in, verify_password};
use axum::Router;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Json, State};
use axum::http::{Extensions, HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use cyberapi_core::directory::{valid_account_name, valid_password};
use cyberapi_http::{HttpEngine, PreparedRequest};
use cyberapi_store::{Refusal, StoreError};
use cyberapi_store_bridge::{BearerSession, Call, LoginForm, SEND_PATH, STORE_PATH, SendReply, dispatch};
use cyberapi_store_sql::{Acceptance, Account, NewAccount, SqlStore};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use throttle::{Key, Throttle};
use tower_http::services::ServeDir;

/// How long a login lasts.
pub const SESSION_DAYS: i64 = 30;
/// A request body or a kept response, base64 in JSON.
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
/// How often the logins that ran out are forgotten.
const HOUSEKEEPING: Duration = Duration::from_secs(60 * 60);

#[derive(Clone)]
pub struct Bridge {
    inner: Arc<Inner>,
}

struct Inner {
    store: SqlStore,
    /// Sends for every account: a `NativeEngine` that only reaches the
    /// addresses its `TargetPolicy` allows.
    engine: Arc<dyn HttpEngine>,
    /// What `POST /v1/setup` must be shown, while no account exists.
    setup_token: String,
    /// Mark the cookie `Secure`: on, unless the bridge is reached over plain
    /// HTTP (development).
    secure_cookie: bool,
    throttle: Throttle,
}

impl Bridge {
    /// The store's directory sets and checks passwords as sign-in does.
    pub fn new(store: SqlStore, engine: Arc<dyn HttpEngine>, setup_token: String, secure_cookie: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                store: store.with_passwords(Arc::new(auth::Argon2Passwords)),
                engine,
                setup_token,
                secure_cookie,
                throttle: Throttle::default(),
            }),
        }
    }

    /// Forgets the logins that ran out, now and every hour after, for as
    /// long as the runtime runs.
    pub fn spawn_housekeeping(&self) -> tokio::task::JoinHandle<()> {
        let bridge = self.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(HOUSEKEEPING);
            loop {
                every.tick().await;
                match bridge.inner.store.delete_expired_sessions(auth::unix_now()).await {
                    Ok(0) => {}
                    Ok(forgotten) => tracing::info!(forgotten, "forgot the logins that ran out"),
                    Err(error) => tracing::warn!(%error, "the logins that ran out could not be forgotten"),
                }
            }
        })
    }

    pub fn store(&self) -> &SqlStore {
        &self.inner.store
    }

    /// The routes, with the page behind them: the one compiled in, or the
    /// files in `www`.
    pub fn router(&self, www: Option<PathBuf>) -> Router {
        let api = Router::new()
            .route("/v1/setup", get(setup_needed).post(setup))
            .route("/v1/login", post(login))
            .route("/v1/logout", post(logout))
            .route("/v1/invite", post(invite_info))
            .route("/v1/invite/accept", post(accept_invite))
            .route("/v1/session", get(session))
            .route(STORE_PATH, post(store_call))
            .route(SEND_PATH, post(send_request))
            .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
            .with_state(self.clone());
        match www {
            Some(dir) => api.fallback_service(ServeDir::new(dir)),
            None => api.fallback(static_files::serve),
        }
    }

    /// Signs `account` in, answering it with the cookie that carries the
    /// new session — or, for a `bearer` client, the session's token.
    async fn sign_in(&self, account: Account, bearer: bool) -> Response {
        let token = match new_token() {
            Ok(token) => token,
            Err(error) => return internal(error),
        };
        let expires_at = auth::unix_now() + SESSION_DAYS * 24 * 60 * 60;
        if let Err(error) = self
            .inner
            .store
            .create_session(&hash_token(&token), &account.id, expires_at)
            .await
        {
            return internal(error);
        }
        if bearer {
            return Json(BearerSession { token }).into_response();
        }
        let cookie = session_cookie(&token, SESSION_DAYS * 24 * 60 * 60, self.inner.secure_cookie);
        ([(header::SET_COOKIE, cookie)], Json(account)).into_response()
    }
}

/// A setup token for when none is configured.
pub fn new_setup_token() -> Result<String, String> {
    new_token()
}

fn internal(error: impl std::fmt::Display) -> Response {
    tracing::error!(%error, "a request failed");
    (StatusCode::INTERNAL_SERVER_ERROR, "the bridge could not do that").into_response()
}

/// Where a request came from, when the server was told.
fn address(extensions: &Extensions) -> Option<Key> {
    extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(from)| Key::Address(from.ip()))
}

/// 429, saying when to try again, when `keys` have failed too often.
fn throttled(bridge: &Bridge, keys: &[Key]) -> Option<Response> {
    let wait = bridge.inner.throttle.wait(keys, Instant::now())?;
    let seconds = wait.as_secs().max(1);
    Some(
        (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, seconds.to_string())],
            "too many failed attempts; try again later",
        )
            .into_response(),
    )
}

#[derive(Serialize)]
struct SetupNeeded {
    needed: bool,
}

async fn setup_needed(State(bridge): State<Bridge>) -> Response {
    match bridge.inner.store.account_count().await {
        Ok(count) => Json(SetupNeeded { needed: count == 0 }).into_response(),
        Err(error) => internal(error),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupForm {
    token: String,
    name: String,
    #[serde(default)]
    display_name: String,
    password: String,
}

fn already_set_up() -> Response {
    (StatusCode::CONFLICT, "the bridge already has accounts").into_response()
}

/// The first administrator, made with the setup token — which counts only
/// while there is no account at all. Of two setups at once, one makes it.
async fn setup(State(bridge): State<Bridge>, extensions: Extensions, Json(form): Json<SetupForm>) -> Response {
    match bridge.inner.store.account_count().await {
        Ok(0) => {}
        Ok(_) => return already_set_up(),
        Err(error) => return internal(error),
    }
    let keys: Vec<Key> = [Some(Key::Setup), address(&extensions)].into_iter().flatten().collect();
    if let Some(refused) = throttled(&bridge, &keys) {
        return refused;
    }
    if !auth::same_secret(&form.token, &bridge.inner.setup_token) {
        bridge.inner.throttle.failed(&keys, Instant::now());
        return (StatusCode::FORBIDDEN, "wrong setup token").into_response();
    }
    if let Err(reason) = check_new_account(&form.name, &form.password) {
        return (StatusCode::BAD_REQUEST, reason).into_response();
    }
    let password = form.password;
    let hash = match tokio::task::spawn_blocking(move || hash_password(&password)).await {
        Ok(Ok(hash)) => hash,
        Ok(Err(error)) => return internal(error),
        Err(error) => return internal(error),
    };
    let display_name = if form.display_name.trim().is_empty() {
        form.name.clone()
    } else {
        form.display_name.trim().to_string()
    };
    let id = match bridge
        .inner
        .store
        .create_first_admin(NewAccount {
            name: form.name.clone(),
            display_name: display_name.clone(),
            password_hash: hash,
            admin: true,
        })
        .await
    {
        Ok(Some(id)) => id,
        Ok(None) => return already_set_up(),
        Err(error) => return internal(error),
    };
    let account = Account {
        id,
        name: form.name,
        display_name,
        admin: true,
    };
    bridge.sign_in(account, false).await
}

fn check_new_account(name: &str, password: &str) -> Result<(), &'static str> {
    if !valid_account_name(name) {
        return Err("a name is 1 to 64 characters, with no spaces");
    }
    if !valid_password(password) {
        return Err("a password has at least 8 characters");
    }
    Ok(())
}

async fn login(State(bridge): State<Bridge>, extensions: Extensions, Json(form): Json<LoginForm>) -> Response {
    let name = Key::name(&form.name);
    let keys: Vec<Key> = [Some(name.clone()), address(&extensions)]
        .into_iter()
        .flatten()
        .collect();
    if let Some(refused) = throttled(&bridge, &keys) {
        return refused;
    }
    let found = match bridge.inner.store.find_login(&form.name).await {
        Ok(found) => found,
        Err(error) => return internal(error),
    };
    // A missing name costs the same Argon2 check as a wrong password, so the
    // answer's timing does not say which it was.
    let (account, hash) = match found {
        Some((account, hash)) => (Some(account), hash),
        None => (None, auth::DECOY_HASH.clone()),
    };
    let password = form.password;
    let verified = tokio::task::spawn_blocking(move || verify_password(&hash, &password))
        .await
        .unwrap_or(false);
    match account {
        Some(account) if verified => {
            bridge.inner.throttle.succeeded(&name);
            bridge.sign_in(account, form.bearer).await
        }
        _ => {
            bridge.inner.throttle.failed(&keys, Instant::now());
            (StatusCode::UNAUTHORIZED, "wrong name or password").into_response()
        }
    }
}

#[derive(Deserialize)]
struct InviteForm {
    token: String,
}

/// Who sent the invitation, and to which team, for the page to say before
/// the account is made (`docs/server-sync.md` §5).
async fn invite_info(State(bridge): State<Bridge>, extensions: Extensions, Json(form): Json<InviteForm>) -> Response {
    let keys: Vec<Key> = address(&extensions).into_iter().collect();
    if let Some(refused) = throttled(&bridge, &keys) {
        return refused;
    }
    match bridge.inner.store.invite_info(&form.token).await {
        Ok(info) => Json(info).into_response(),
        Err(error) => invite_refused(&bridge, &keys, error),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AcceptForm {
    token: String,
    name: String,
    #[serde(default)]
    display_name: String,
    password: String,
}

/// The account an invitation asks for, made and signed in. Nothing is made
/// for an invitation used, revoked or run out, or a name already taken.
async fn accept_invite(State(bridge): State<Bridge>, extensions: Extensions, Json(form): Json<AcceptForm>) -> Response {
    let keys: Vec<Key> = address(&extensions).into_iter().collect();
    if let Some(refused) = throttled(&bridge, &keys) {
        return refused;
    }
    let acceptance = Acceptance {
        name: form.name,
        display_name: form.display_name,
        password: form.password,
    };
    match bridge.inner.store.accept_invite(&form.token, acceptance).await {
        Ok(account) => bridge.sign_in(account, false).await,
        Err(error) => invite_refused(&bridge, &keys, error),
    }
}

/// A refusal as the page shows it. An invitation that opens nothing counts
/// as a failed try, as a wrong password does.
fn invite_refused(bridge: &Bridge, keys: &[Key], error: StoreError) -> Response {
    let StoreError::Refused { reason } = error else {
        return internal(error);
    };
    match reason {
        Refusal::InviteGone => {
            bridge.inner.throttle.failed(keys, Instant::now());
            (StatusCode::GONE, "this invitation was used, taken back or has run out").into_response()
        }
        Refusal::NameTaken => (StatusCode::CONFLICT, "that name is taken").into_response(),
        Refusal::BadAccountName => {
            (StatusCode::BAD_REQUEST, "a name is 1 to 64 characters, with no spaces").into_response()
        }
        Refusal::PasswordTooShort => (StatusCode::BAD_REQUEST, "a password has at least 8 characters").into_response(),
        other => internal(format!("{other:?}")),
    }
}

async fn logout(State(bridge): State<Bridge>, headers: HeaderMap) -> Response {
    if let Some(token) = auth::session_token(&headers)
        && let Err(error) = bridge.inner.store.delete_session(&hash_token(&token)).await
    {
        return internal(error);
    }
    (
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, session_cookie("", 0, bridge.inner.secure_cookie))],
    )
        .into_response()
}

async fn session(State(bridge): State<Bridge>, headers: HeaderMap) -> Response {
    match signed_in(&bridge.inner.store, &headers).await {
        Ok(account) => Json(account).into_response(),
        Err(status) => status.into_response(),
    }
}

/// One store call, on the signed-in account's view of the store.
async fn store_call(State(bridge): State<Bridge>, headers: HeaderMap, Json(call): Json<Call>) -> Response {
    let account = match signed_in(&bridge.inner.store, &headers).await {
        Ok(account) => account,
        Err(status) => return status.into_response(),
    };
    let storage = bridge.inner.store.scoped(account.id.clone());
    let directory = bridge.inner.store.directory(account.id);
    Json(dispatch(&storage, directory.as_ref(), call).await).into_response()
}

/// Sends for a signed-in account. A request the engine refuses or that
/// fails is a normal answer, like a store's refusal.
async fn send_request(
    State(bridge): State<Bridge>,
    headers: HeaderMap,
    Json(request): Json<PreparedRequest>,
) -> Response {
    if let Err(status) = signed_in(&bridge.inner.store, &headers).await {
        return status.into_response();
    }
    let reply = match bridge.inner.engine.send(request).await {
        Ok(exchange) => SendReply::Ok(exchange),
        Err(error) => SendReply::Err(error),
    };
    Json(reply).into_response()
}
