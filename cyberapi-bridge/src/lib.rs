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
//! | `GET /v1/oidc` | whether single sign-on is on (`{"enabled": bool}`) |
//! | `POST /v1/oidc/start` | begins an OpenID Connect sign-in (`G5`) |
//! | `GET /v1/oidc/callback` | the provider's return: a session cookie, or a page the desktop closes |
//! | `GET /v1/oidc/poll` | the desktop's one read of the session token |
//! | `POST /v1/store` | one store call (`cyberapi_store_bridge::Call`) |
//! | `POST /v1/send` | sends a prepared request (`docs/architecture.md` §5), a few of an account's at a time ([`in_flight`]) |
//! | `POST /v1/send/stream` | the same, an event stream's answer told as it comes (NDJSON of `StreamReply`); the page leaving stops it |
//! | `POST /v1/mcp` | an AI assistant's tools, signed in with an account's MCP token ([`mcp`]) |
//! | `ANY /v1/mock/{project}`, `ANY /v1/mock/{project}/{*rest}` | a project's mock responses (`G2b`, [`mock`]), signed in by session or MCP token |
//! | `GET /v1/docs/{project}` | a project's read-only HTML documentation (`G3`, [`docs`]); public when the project says so, otherwise a session or MCP token, and a signed-out browser is sent to the sign-in page |
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
mod docs;
mod in_flight;
mod mcp;
mod mock;
mod oidc;
pub mod static_files;
mod throttle;

use auth::{hash_password, hash_token, new_token, session_cookie, signed_in, verify_password};
use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Json, Request, State};
use axum::http::{Extensions, HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{any, get, post};
use cyberapi_core::directory::{valid_account_name, valid_password};
use cyberapi_core::version::MODEL;
use cyberapi_http::{HttpEngine, PreparedRequest, Watch, stop_signal};
use cyberapi_store::{Refusal, StoreError};
use cyberapi_store_bridge::{
    BearerSession, Call, LoginForm, MODEL_HEADER, Reply, SEND_PATH, SEND_STREAM_PATH, STORE_PATH, SendReply,
    StreamReply, dispatch, refused_as_too_old,
};
use cyberapi_store_sql::{Acceptance, Account, NewAccount, SqlStore};
use futures::StreamExt as _;
use in_flight::InFlight;
pub use in_flight::MAX_SENDS;
use serde::{Deserialize, Serialize};
use static_files::Mounted;
pub use static_files::normalize_base_path;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use throttle::{Key, Throttle};
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate as _};
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
    /// The requests on their way through `/v1/send`, each account's and
    /// all of them.
    in_flight: Arc<InFlight>,
    /// Each account's MCP calls in the last minute.
    mcp_calls: mcp::Limiter,
    /// Cached mock route tables keyed by `(CollectionId, revision)` (PERF-1).
    mock_routes: mock::RouteCache,
    /// Rendered online documentation for one revision of a project (PERF-6).
    docs_html: Arc<docs::HtmlCache>,
    /// OpenID Connect, when the process was given an issuer and a client.
    oidc: Option<oidc::Service>,
}

impl Bridge {
    /// The store's directory sets and checks passwords as sign-in does.
    /// `base_path` as [`normalize_base_path`] leaves it. At most
    /// `max_sends` requests of all accounts on their way at once
    /// ([`MAX_SENDS`] unless the administrator says otherwise).
    pub fn new(
        store: SqlStore,
        engine: Arc<dyn HttpEngine>,
        setup_token: String,
        secure_cookie: bool,
        base_path: String,
        max_sends: usize,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store: store.with_passwords(Arc::new(auth::Argon2Passwords)),
                engine,
                setup_token,
                secure_cookie,
                base_path,
                throttle: Throttle::default(),
                in_flight: Arc::new(InFlight::new(max_sends)),
                mcp_calls: mcp::Limiter::default(),
                mock_routes: mock::RouteCache::default(),
                docs_html: Arc::new(docs::HtmlCache::default()),
                oidc: None,
            }),
        }
    }

    /// Turns single sign-on on. The three of issuer, client id and client
    /// secret come together; a blank one counts as unset. Call this before
    /// the bridge is shared.
    pub fn with_single_sign_on(
        self,
        issuer: Option<String>,
        client_id: Option<String>,
        client_secret: Option<String>,
        redirect: Option<String>,
    ) -> Result<Self, String> {
        let Some(settings) = oidc::configure(issuer, client_id, client_secret, redirect)? else {
            return Ok(self);
        };
        tracing::info!(issuer = settings.issuer(), "single sign-on is on");
        let mut bridge = self;
        let inner = Arc::get_mut(&mut bridge.inner).ok_or_else(|| "the bridge is already shared".to_string())?;
        inner.oidc = Some(oidc::Service::new(settings));
        Ok(bridge)
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
                if let Some(oidc) = &bridge.inner.oidc {
                    let forgotten = oidc.forget_expired();
                    if forgotten > 0 {
                        tracing::info!(forgotten, "forgot single sign-on attempts that ran out");
                    }
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
            .route(&at("/v1/oidc"), get(oidc::enabled))
            .route(&at("/v1/oidc/start"), post(oidc::start))
            .route(&at("/v1/oidc/callback"), get(oidc::callback))
            .route(&at("/v1/oidc/poll"), get(oidc::poll))
            .route(&at(STORE_PATH), post(store_call))
            .route(&at(SEND_PATH), post(send_request))
            .route(&at(SEND_STREAM_PATH), post(send_watched))
            .route(
                &at("/v1/mcp"),
                post(mcp::post).get(mcp::not_allowed).delete(mcp::not_allowed),
            )
            .route(&at("/v1/mock/{project}"), any(mock::handle_root))
            .route(&at("/v1/mock/{project}/{*rest}"), any(mock::handle_path))
            .route(&at("/v1/docs/{project}"), get(docs::get))
            .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
            .fallback(move |State(bridge): State<Bridge>, request: Request| page(bridge, request, www.clone()))
            // Outside the body limit, so the limit counts the bytes after
            // gzip or zstd is undone. A request with no Content-Encoding
            // passes through.
            .layer(request_decompression())
            .layer(compression_layer())
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
        let kept = remember || bearer;
        match self.issue_session(&account, kept).await {
            Ok(token) if bearer => Json(BearerSession { token }).into_response(),
            Ok(token) => {
                let cookie = session_cookie(
                    &token,
                    kept.then_some(KEPT_SESSION),
                    self.inner.secure_cookie,
                    &self.cookie_path(),
                );
                ([(header::SET_COOKIE, cookie)], Json(account)).into_response()
            }
            Err(response) => *response,
        }
    }

    /// Makes a session and returns its token. The caller sets the cookie,
    /// or hands the token to a client that keeps it. Single sign-on always
    /// keeps the session. The error is boxed: an axum `Response` is larger
    /// than clippy allows in a `Result`.
    async fn issue_session(&self, account: &Account, kept: bool) -> Result<String, Box<Response>> {
        let token = new_token().map_err(|error| Box::new(internal(error)))?;
        let lifetime = if kept { KEPT_SESSION } else { UNKEPT_SESSION };
        let expires_at = auth::unix_now() + lifetime;
        self.inner
            .store
            .create_session(&hash_token(&token), &account.id, expires_at)
            .await
            .map_err(|error| Box::new(internal(error)))?;
        Ok(token)
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
    // A client keeping projects in an older shape reads them but does not
    // change them: it would drop what it cannot read (§2.1).
    let model = |headers: &HeaderMap| headers.get(MODEL_HEADER)?.to_str().ok()?.trim().parse::<u32>().ok();
    let reply = if refused_as_too_old(&call, model(&headers), MODEL) {
        Reply::Err(StoreError::Refused {
            reason: Refusal::ClientTooOld,
        })
    } else {
        let storage = bridge.inner.store.scoped(account.id.clone());
        let directory = bridge.inner.store.directory(account.id.clone());
        let versions = bridge.inner.store.versions(account.id);
        dispatch(&storage, directory.as_ref(), versions.as_ref(), call).await
    };
    let mut response = Json(reply).into_response();
    response.headers_mut().insert(MODEL_HEADER, HeaderValue::from(MODEL));
    response
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
    let sending = match bridge.inner.in_flight.start(&account.id) {
        Ok(sending) => sending,
        Err(busy) => return ndjson(axum::body::Body::from(StreamReply::Done(SendReply::Err(busy)).line())),
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
    let _sending = match bridge.inner.in_flight.start(&account.id) {
        Ok(sending) => sending,
        Err(busy) => return Json(SendReply::Err(busy)).into_response(),
    };
    let reply = match bridge.inner.engine.send(request).await {
        Ok(exchange) => SendReply::Ok(Box::new(exchange)),
        Err(error) => SendReply::Err(error),
    };
    Json(reply).into_response()
}

/// Compresses bridge answers with `zstd` (preferred) or `gzip`, while leaving
/// watched NDJSON streams unbuffered.
fn compression_layer() -> CompressionLayer<impl tower_http::compression::predicate::Predicate> {
    CompressionLayer::new()
        .gzip(true)
        .zstd(true)
        .compress_when(DefaultPredicate::new().and(NotForContentType::const_new("application/x-ndjson")))
}

/// Undoes `gzip` or `zstd` on a request body before the handler reads it.
/// Clients send gzip (pure Rust, including the page). zstd is accepted too.
fn request_decompression() -> tower_http::decompression::RequestDecompressionLayer {
    tower_http::decompression::RequestDecompressionLayer::new()
        .gzip(true)
        .zstd(true)
        .no_deflate()
        .no_br()
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt as _;
    use std::io::Read as _;
    use tower::ServiceExt as _;

    fn decode_gzip(bytes: &[u8]) -> String {
        let mut decoder = flate2::read::GzDecoder::new(bytes);
        let mut out = String::new();
        decoder.read_to_string(&mut out).expect("valid gzip");
        out
    }

    fn decode_zstd(bytes: &[u8]) -> String {
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(bytes).expect("zstd stream");
        let mut out = String::new();
        decoder.read_to_string(&mut out).expect("valid zstd");
        out
    }

    #[tokio::test]
    async fn compression_prefers_zstd_over_gzip_and_skips_ndjson_streams() {
        let payload = "cyberapi-bridge-response-".repeat(16);
        let expected_json = serde_json::json!({ "data": payload }).to_string();
        let router = Router::new()
            .route(
                "/v1/store",
                post({
                    let payload = payload.clone();
                    move || async move { Json(serde_json::json!({ "data": payload })) }
                }),
            )
            .route(
                "/v1/send/stream",
                post({
                    let line = format!("{expected_json}\n");
                    move || async move { ndjson(Body::from(line.clone())) }
                }),
            )
            .fallback(|request: Request| async move { static_files::serve(request.headers(), request.uri().path()) })
            .layer(compression_layer());

        // When both gzip and zstd are accepted, zstd is preferred for API and static responses.
        let req = Request::builder()
            .method("POST")
            .uri("/v1/store")
            .header(header::ACCEPT_ENCODING, "gzip, deflate, br, zstd")
            .body(Body::empty())
            .expect("request");
        let resp = router.clone().oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("zstd")
        );
        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        assert_eq!(decode_zstd(&bytes), expected_json);

        // Static page fallback is also compressed with zstd when accepted.
        let page_req = Request::builder()
            .method("GET")
            .uri("/")
            .header(header::ACCEPT_ENCODING, "gzip, zstd")
            .body(Body::empty())
            .expect("request");
        let page_resp = router.clone().oneshot(page_req).await.expect("response");
        assert_eq!(page_resp.status(), StatusCode::OK);
        assert_eq!(
            page_resp
                .headers()
                .get(header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("zstd")
        );
        let page_bytes = page_resp.into_body().collect().await.expect("body").to_bytes();
        assert!(decode_zstd(&page_bytes).contains("<!DOCTYPE html>") || decode_zstd(&page_bytes).contains("<html"));

        // When only gzip is accepted, gzip is used.
        let gzip_req = Request::builder()
            .method("POST")
            .uri("/v1/store")
            .header(header::ACCEPT_ENCODING, "gzip, deflate")
            .body(Body::empty())
            .expect("request");
        let gzip_resp = router.clone().oneshot(gzip_req).await.expect("response");
        assert_eq!(gzip_resp.status(), StatusCode::OK);
        assert_eq!(
            gzip_resp
                .headers()
                .get(header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("gzip")
        );
        let gzip_bytes = gzip_resp.into_body().collect().await.expect("body").to_bytes();
        assert_eq!(decode_gzip(&gzip_bytes), expected_json);

        // Streaming NDJSON (`/v1/send/stream`) is not compressed so events flush as they arrive.
        let stream_req = Request::builder()
            .method("POST")
            .uri("/v1/send/stream")
            .header(header::ACCEPT_ENCODING, "gzip, zstd")
            .body(Body::empty())
            .expect("request");
        let stream_resp = router.oneshot(stream_req).await.expect("response");
        assert_eq!(stream_resp.status(), StatusCode::OK);
        assert_eq!(stream_resp.headers().get(header::CONTENT_ENCODING), None);
    }

    #[tokio::test]
    async fn a_gzip_request_body_reaches_the_handler_as_json() {
        use std::io::Write as _;

        let json = br#"{"call":"collectionsList"}"#;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(json).expect("gzip");
        let gzip = encoder.finish().expect("gzip");
        let router = Router::new()
            .route("/v1/store", post(|body: axum::body::Bytes| async move { body }))
            .layer(request_decompression());
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/store")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::CONTENT_ENCODING, "gzip")
                    .body(Body::from(gzip))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.expect("body").to_bytes();
        assert_eq!(&bytes[..], json);
    }
}
