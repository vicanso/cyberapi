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
//! | `POST /v1/send` | sends a prepared request (`docs/architecture.md` §5), a few of an account's at a time ([`in_flight`]) |
//! | `POST /v1/send/stream` | the same, an event stream's answer told as it comes (NDJSON of `StreamReply`); the page leaving stops it |
//! | `POST /v1/mcp` | an AI assistant's tools, signed in with an account's MCP token ([`mcp`]) |
//! | anything else | the page's files ([`static_files`]) |
//!
//! Under `--base-path /prefix` all of it is below the prefix — `/prefix/`,
//! `/prefix/v1/login` — for a reverse proxy that keeps the prefix, and the
//! login cookie's path is the prefix's, so applications sharing the host
//! name do not get it. The routes are registered at their full paths:
//! axum's `nest` gives the inner router `/prefix` but not `/prefix/`, the
//! one address the page lives at.
//!
//! Failed sign-ins, setup tries and invitations that open nothing are counted ([`throttle`]) and turned
//! away with 429 once there are too many; serve the router with
//! `into_make_service_with_connect_info::<SocketAddr>` so they are counted
//! by address too.

mod auth;
mod in_flight;
mod mcp;
pub mod static_files;
mod throttle;

use auth::{hash_password, hash_token, new_token, session_cookie, signed_in, verify_password};
use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Json, Request, State};
use axum::http::{Extensions, HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use cyberapi_core::directory::{valid_account_name, valid_password};
use cyberapi_http::{HttpEngine, PreparedRequest, SendError, Watch, stop_signal};
use cyberapi_store::{Refusal, StoreError};
use cyberapi_store_bridge::{
    BearerSession, Call, LoginForm, SEND_PATH, SEND_STREAM_PATH, STORE_PATH, SendReply, StreamReply, dispatch,
};
use cyberapi_store_sql::{Acceptance, Account, NewAccount, SqlStore};
use futures::StreamExt as _;
use in_flight::InFlight;
use serde::{Deserialize, Serialize};
use static_files::Mounted;
pub use static_files::normalize_base_path;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use throttle::{Key, Throttle};
use tower_http::services::ServeDir;

/// How long a kept login lasts, in seconds: the page's "keep me signed in",
/// on by default, and every bearer session — a client keeps its token.
pub const KEPT_SESSION: i64 = 30 * 24 * 60 * 60;
/// A login not kept: its cookie ends with the browser, and the bridge lets
/// it go within the working day even while the browser stays open.
pub const UNKEPT_SESSION: i64 = 12 * 60 * 60;
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
    /// Where it is mounted, as [`normalize_base_path`] leaves it: empty for
    /// the root, else `/prefix`.
    base_path: String,
    throttle: Throttle,
    /// Each account's requests on their way through `/v1/send`.
    in_flight: Arc<InFlight>,
    /// Each account's MCP calls in the last minute.
    mcp_calls: mcp::Limiter,
}

impl Bridge {
    /// The store's directory sets and checks passwords as sign-in does.
    /// `base_path` as [`normalize_base_path`] leaves it.
    pub fn new(
        store: SqlStore,
        engine: Arc<dyn HttpEngine>,
        setup_token: String,
        secure_cookie: bool,
        base_path: String,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store: store.with_passwords(Arc::new(auth::Argon2Passwords)),
                engine,
                setup_token,
                secure_cookie,
                base_path,
                throttle: Throttle::default(),
                in_flight: Arc::default(),
                mcp_calls: mcp::Limiter::default(),
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

    /// The routes, below the mount, with the page behind them: the one
    /// compiled in, or the files in `www`.
    pub fn router(&self, www: Option<PathBuf>) -> Router {
        let at = |path: &str| format!("{}{path}", self.inner.base_path);
        let www = www.map(Arc::new);
        Router::new()
            .route(&at("/v1/setup"), get(setup_needed).post(setup))
            .route(&at("/v1/login"), post(login))
            .route(&at("/v1/logout"), post(logout))
            .route(&at("/v1/invite"), post(invite_info))
            .route(&at("/v1/invite/accept"), post(accept_invite))
            .route(&at("/v1/session"), get(session))
            .route(&at(STORE_PATH), post(store_call))
            .route(&at(SEND_PATH), post(send_request))
            .route(&at(SEND_STREAM_PATH), post(send_watched))
            .route(
                &at("/v1/mcp"),
                post(mcp::post).get(mcp::not_allowed).delete(mcp::not_allowed),
            )
            .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
            .fallback(move |State(bridge): State<Bridge>, request: Request| page(bridge, request, www.clone()))
            .with_state(self.clone())
    }

    /// The login cookie's path: the mount's, with its slash.
    fn cookie_path(&self) -> String {
        format!("{}/", self.inner.base_path)
    }

    /// Signs `account` in, answering it with the cookie that carries the
    /// new session — or, for a `bearer` client, the session's token. A
    /// session not `remember`ed has a cookie that ends with the browser, and
    /// a shorter life on the bridge; a bearer session is always kept.
    async fn sign_in(&self, account: Account, bearer: bool, remember: bool) -> Response {
        let token = match new_token() {
            Ok(token) => token,
            Err(error) => return internal(error),
        };
        let kept = remember || bearer;
        let lifetime = if kept { KEPT_SESSION } else { UNKEPT_SESSION };
        let expires_at = auth::unix_now() + lifetime;
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
        let cookie = session_cookie(
            &token,
            kept.then_some(KEPT_SESSION),
            self.inner.secure_cookie,
            &self.cookie_path(),
        );
        ([(header::SET_COOKIE, cookie)], Json(account)).into_response()
    }
}

/// The page's files, below the mount and nowhere else. The mount without
/// its slash is sent to the address with it: the page resolves everything
/// relative to where it was loaded from.
async fn page(bridge: Bridge, request: Request, www: Option<Arc<PathBuf>>) -> Response {
    let base = bridge.inner.base_path.clone();
    let path = request.uri().path().to_string();
    match static_files::mounted(&base, &path) {
        Mounted::Outside => static_files::not_found(),
        Mounted::NeedsSlash => {
            let query = request.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
            Redirect::temporary(&format!("{base}/{query}")).into_response()
        }
        Mounted::File(below) => match www {
            None => static_files::serve(request.headers(), below),
            Some(dir) => from_dir(&dir, request, below).await,
        },
    }
}

/// `--www`: the file below the mount, out of a directory.
async fn from_dir(dir: &Path, request: Request, below: &str) -> Response {
    let (mut parts, body) = request.into_parts();
    let query = parts.uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    parts.uri = match format!("{below}{query}").parse() {
        Ok(uri) => uri,
        Err(_) => return static_files::not_found(),
    };
    match ServeDir::new(dir).try_call(Request::from_parts(parts, body)).await {
        Ok(response) => response.map(Body::new).into_response(),
        Err(error) => internal(error),
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
    /// "Keep me signed in" ([`cyberapi_store_bridge::LoginForm::remember`]).
    #[serde(default = "kept")]
    remember: bool,
}

fn kept() -> bool {
    true
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
    bridge.sign_in(account, false, form.remember).await
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
            bridge.sign_in(account, form.bearer, form.remember).await
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
    /// "Keep me signed in".
    #[serde(default = "kept")]
    remember: bool,
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
        Ok(account) => bridge.sign_in(account, false, form.remember).await,
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
        [(
            header::SET_COOKIE,
            session_cookie("", Some(0), bridge.inner.secure_cookie, &bridge.cookie_path()),
        )],
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

/// Sends as [`send_request`] does, an event stream's answer told to the page
/// as it comes: one `StreamReply` a line, the stream's head and pieces,
/// then the exchange, its body left out when it came in pieces. The page
/// leaving drops the answer, and with it the request.
async fn send_watched(
    State(bridge): State<Bridge>,
    headers: HeaderMap,
    Json(request): Json<PreparedRequest>,
) -> Response {
    let account = match signed_in(&bridge.inner.store, &headers).await {
        Ok(account) => account,
        Err(status) => return status.into_response(),
    };
    let Some(sending) = bridge.inner.in_flight.start(&account.id) else {
        let busy = SendError::Busy {
            limit: in_flight::PER_ACCOUNT,
        };
        return ndjson(axum::body::Body::from(StreamReply::Done(SendReply::Err(busy)).line()));
    };
    let (lines, answer) = futures::channel::mpsc::unbounded::<Vec<u8>>();
    let told = lines.clone();
    // The page stops a stream by leaving, never by this.
    let (keep_going, stop) = stop_signal();
    let watch = Watch {
        progress: Arc::new(move |progress| {
            let _ = told.unbounded_send(StreamReply::Progress(progress).line());
        }),
        stop,
    };
    let engine = bridge.inner.engine.clone();
    let task = tokio::spawn(async move {
        let _sending = sending;
        let _keep_going = keep_going;
        let reply = match engine.send_watched(request, watch).await {
            Ok(mut exchange) => {
                if exchange.stream.is_some() {
                    exchange.body.clear();
                }
                SendReply::Ok(Box::new(exchange))
            }
            Err(error) => SendReply::Err(error),
        };
        let _ = lines.unbounded_send(StreamReply::Done(reply).line());
    });
    // Goes with the answer: a page that leaves ends the request.
    let request = AbortOnDrop(task.abort_handle());
    let body = answer.map(move |line| {
        let _ = &request;
        Ok::<_, std::convert::Infallible>(line)
    });
    ndjson(axum::body::Body::from_stream(body))
}

/// An NDJSON answer that goes as it comes: a proxy that buffers answers
/// (nginx) passes this one on so too.
fn ndjson(body: axum::body::Body) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/x-ndjson"),
            (header::CACHE_CONTROL, "no-store"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        body,
    )
        .into_response()
}

/// Ends a spawned task when its owner goes.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Sends for a signed-in account, a few of its requests at a time. A
/// request the engine refuses or that fails is a normal answer, like a
/// store's refusal.
async fn send_request(
    State(bridge): State<Bridge>,
    headers: HeaderMap,
    Json(request): Json<PreparedRequest>,
) -> Response {
    let account = match signed_in(&bridge.inner.store, &headers).await {
        Ok(account) => account,
        Err(status) => return status.into_response(),
    };
    let Some(_sending) = bridge.inner.in_flight.start(&account.id) else {
        let busy = SendError::Busy {
            limit: in_flight::PER_ACCOUNT,
        };
        return Json(SendReply::Err(busy)).into_response();
    };
    let reply = match bridge.inner.engine.send(request).await {
        Ok(exchange) => SendReply::Ok(Box::new(exchange)),
        Err(error) => SendReply::Err(error),
    };
    Json(reply).into_response()
}
