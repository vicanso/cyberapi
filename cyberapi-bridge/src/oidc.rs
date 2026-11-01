//! OpenID Connect sign-in (G5). Accounts still come only from invitations:
//! the first sign-in links a verified email to the account whose name is
//! that email, and a subject already linked signs in on its own. Nothing
//! here creates an account. The client secret, the PKCE verifier and the
//! one-time bearer token never leave this process.

use super::Bridge;
use crate::auth::{cookie, same_secret, session_cookie};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{Extensions, HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cyberapi_core::model::HttpMethod;
use cyberapi_core::prefs::Timeouts;
use cyberapi_http::{HttpEngine, PreparedRequest};
use cyberapi_store_bridge::BearerSession;
use cyberapi_store_sql::{Account, NamedAccount, OidcAccount, OidcBind};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const STATE_COOKIE: &str = "cyberapi_oidc";
const PENDING_FOR: Duration = Duration::from_secs(5 * 60);
const CACHE_FOR: Duration = Duration::from_secs(10 * 60);
const PENDING_LIMIT: usize = 256;
const PENDING_PER_ADDRESS: usize = 8;
const IDP_BODY_LIMIT: usize = 1024 * 1024;
const ID_LIMIT: usize = 255;

const NO_ACCOUNT: &str =
    "No account matches this sign-in. An administrator invites the account first, and its name is the email.";
const UNVERIFIED: &str = "The identity provider did not confirm this email.";
const TAKEN: &str = "That account is already linked to a different sign-in.";
const FAILED: &str = "Single sign-on did not finish. Try again.";
const PROVIDER: &str = "the identity provider could not be reached";

/// What the process was started with. Built only when the issuer, the
/// client id and the client secret are all set.
pub struct Settings {
    issuer: String,
    client_id: String,
    client_secret: String,
    redirect: Option<String>,
}

impl Settings {
    pub(super) fn issuer(&self) -> &str {
        &self.issuer
    }
}

/// Blank is unset. Any one of the three requires all three.
pub fn configure(
    issuer: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    redirect: Option<String>,
) -> Result<Option<Settings>, String> {
    let issuer = blank(issuer);
    let client_id = blank(client_id);
    let client_secret = blank(client_secret);
    let redirect = blank(redirect);
    match (issuer, client_id, client_secret) {
        (None, None, None) if redirect.is_none() => Ok(None),
        (Some(issuer), Some(client_id), Some(client_secret)) => Ok(Some(Settings {
            issuer: normalize_issuer(&issuer)?,
            client_id,
            client_secret,
            redirect: redirect.map(|url| check_redirect(&url)).transpose()?,
        })),
        _ => Err("single sign-on needs --oidc-issuer, --oidc-client-id and --oidc-client-secret together".into()),
    }
}

fn blank(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

/// Trimmed, one trailing slash removed. `http(s)` only, with no user, query
/// or fragment. A path stays, so `https://host/realms/name` is one issuer.
pub fn normalize_issuer(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    let trimmed = trimmed.strip_suffix('/').unwrap_or(trimmed);
    if trimmed.chars().any(char::is_whitespace) {
        return Err("the single sign-on issuer has a space".into());
    }
    let Some(rest) = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
    else {
        return Err("the single sign-on issuer starts with http:// or https://".into());
    };
    if rest.is_empty() || rest.starts_with('/') || rest.contains('@') || rest.contains('?') || rest.contains('#') {
        return Err("the single sign-on issuer has no user, query or fragment".into());
    }
    Ok(trimmed.to_string())
}

fn check_redirect(value: &str) -> Result<String, String> {
    if value.chars().any(char::is_whitespace) {
        return Err("the single sign-on redirect URL has a space".into());
    }
    let Some(rest) = value.strip_prefix("https://").or_else(|| value.strip_prefix("http://")) else {
        return Err("the single sign-on redirect URL starts with http:// or https://".into());
    };
    if rest.is_empty() || rest.starts_with('/') || rest.contains('@') || rest.contains('#') {
        return Err("the single sign-on redirect URL has no user or fragment".into());
    }
    Ok(value.to_string())
}

/// S256, as RFC 7636 defines it: base64url of the SHA-256, without padding.
pub fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn random_url(bytes: usize) -> Result<String, String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|error| error.to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

fn form_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fail {
    NoAccount,
    Unverified,
    Taken,
    Failed,
    Provider,
}

impl Fail {
    fn text(self) -> &'static str {
        match self {
            Self::NoAccount => NO_ACCOUNT,
            Self::Unverified => UNVERIFIED,
            Self::Taken => TAKEN,
            Self::Failed => FAILED,
            Self::Provider => PROVIDER,
        }
    }

    fn query(self) -> &'static str {
        match self {
            Self::NoAccount => "oidc=no-account",
            Self::Unverified => "oidc=unverified",
            Self::Taken => "oidc=taken",
            Self::Failed | Self::Provider => "oidc=failed",
        }
    }

    fn status(self) -> StatusCode {
        match self {
            Self::Taken => StatusCode::CONFLICT,
            Self::Provider => StatusCode::BAD_GATEWAY,
            Self::NoAccount | Self::Unverified | Self::Failed => StatusCode::BAD_REQUEST,
        }
    }
}

#[derive(Clone, Debug)]
enum Outcome {
    Token(String),
    Failed(Fail),
}

#[derive(Clone, Debug)]
enum Stage {
    Ready,
    Busy,
    Settled(Outcome),
}

struct Pending {
    verifier: String,
    nonce: String,
    bearer: bool,
    redirect: String,
    from: Option<IpAddr>,
    at: Instant,
    stage: Stage,
}

#[derive(Clone, Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

struct Cache {
    at: Instant,
    discovery: Discovery,
    jwks: JwkSet,
}

enum Begun {
    Go {
        verifier: String,
        nonce: String,
        redirect: String,
    },
    Busy,
    Again(Outcome),
    Missing,
}

/// The attempts in flight, and the provider's discovery document.
pub struct Service {
    settings: Settings,
    pending: Mutex<HashMap<String, Pending>>,
    cache: Mutex<Option<Cache>>,
}

impl Service {
    pub(super) fn new(settings: Settings) -> Self {
        Self {
            settings,
            pending: Mutex::new(HashMap::new()),
            cache: Mutex::new(None),
        }
    }

    pub(super) fn forget_expired(&self) -> usize {
        let mut pending = lock(&self.pending);
        let before = pending.len();
        forget_in(&mut pending, Instant::now());
        before - pending.len()
    }

    fn begin(&self, state: &str) -> Begun {
        let mut pending = lock(&self.pending);
        forget_in(&mut pending, Instant::now());
        let Some(entry) = pending.get_mut(state) else {
            return Begun::Missing;
        };
        match &entry.stage {
            Stage::Busy => Begun::Busy,
            Stage::Settled(outcome) => Begun::Again(outcome.clone()),
            Stage::Ready => {
                entry.stage = Stage::Busy;
                Begun::Go {
                    verifier: entry.verifier.clone(),
                    nonce: entry.nonce.clone(),
                    redirect: entry.redirect.clone(),
                }
            }
        }
    }

    fn bearer(&self, state: &str) -> Option<bool> {
        let mut pending = lock(&self.pending);
        forget_in(&mut pending, Instant::now());
        pending.get(state).map(|entry| entry.bearer)
    }

    fn settle(&self, state: &str, outcome: Outcome) {
        let mut pending = lock(&self.pending);
        if let Some(entry) = pending.get_mut(state) {
            entry.stage = Stage::Settled(outcome);
        }
    }

    fn remove(&self, state: &str) {
        lock(&self.pending).remove(state);
    }

    fn take(&self, state: &str) -> Option<Outcome> {
        let mut pending = lock(&self.pending);
        forget_in(&mut pending, Instant::now());
        match pending.get(state).map(|entry| entry.stage.clone()) {
            Some(Stage::Settled(_)) => match pending.remove(state).map(|entry| entry.stage) {
                Some(Stage::Settled(outcome)) => Some(outcome),
                _ => None,
            },
            _ => None,
        }
    }

    fn waiting(&self, state: &str) -> bool {
        let mut pending = lock(&self.pending);
        forget_in(&mut pending, Instant::now());
        matches!(
            pending.get(state).map(|entry| &entry.stage),
            Some(Stage::Ready | Stage::Busy)
        )
    }

    fn insert(&self, state: String, entry: Pending) -> bool {
        let mut pending = lock(&self.pending);
        forget_in(&mut pending, Instant::now());
        if pending.len() >= PENDING_LIMIT {
            return false;
        }
        let from = entry.from;
        let here = pending.values().filter(|item| item.from == from).count();
        if here >= PENDING_PER_ADDRESS {
            return false;
        }
        pending.insert(state, entry);
        true
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn forget_in(pending: &mut HashMap<String, Pending>, now: Instant) {
    pending.retain(|_, entry| now.saturating_duration_since(entry.at) < PENDING_FOR);
}

#[derive(Deserialize)]
pub(super) struct StartForm {
    #[serde(default)]
    bearer: bool,
}

#[derive(Serialize)]
struct Started {
    url: String,
    state: String,
}

#[derive(Serialize)]
pub(super) struct Enabled {
    enabled: bool,
}

#[derive(Deserialize)]
pub(super) struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct PollQuery {
    state: Option<String>,
}

#[derive(Serialize)]
struct Still {
    pending: bool,
}

pub(super) async fn enabled(State(bridge): State<Bridge>) -> Json<Enabled> {
    Json(Enabled {
        enabled: bridge.inner.oidc.is_some(),
    })
}

pub(super) async fn start(
    State(bridge): State<Bridge>,
    headers: HeaderMap,
    extensions: Extensions,
    Json(form): Json<StartForm>,
) -> Response {
    let Some(oidc) = bridge.inner.oidc.as_ref() else {
        return (StatusCode::NOT_FOUND, "single sign-on is not set up").into_response();
    };
    let redirect = match callback_url(&bridge, &headers, oidc.settings.redirect.as_deref()) {
        Ok(redirect) => redirect,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    if let Err(error) = oidc.discovery(bridge.inner.engine.as_ref(), false).await {
        tracing::warn!(%error, "{PROVIDER}");
        return (StatusCode::BAD_GATEWAY, PROVIDER).into_response();
    }
    let (state, verifier, nonce) = match (random_url(32), random_url(32), random_url(32)) {
        (Ok(state), Ok(verifier), Ok(nonce)) => (state, verifier, nonce),
        _ => return super::internal("no randomness"),
    };
    let from = super::address(&extensions).and_then(|key| match key {
        super::Key::Address(ip) => Some(ip),
        _ => None,
    });
    let challenge = code_challenge(&verifier);
    let entry = Pending {
        verifier,
        nonce: nonce.clone(),
        bearer: form.bearer,
        redirect: redirect.clone(),
        from,
        at: Instant::now(),
        stage: Stage::Ready,
    };
    if !oidc.insert(state.clone(), entry) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "too many sign-ins are waiting; try again later",
        )
            .into_response();
    }
    let endpoint = match oidc.discovery(bridge.inner.engine.as_ref(), false).await {
        Ok(discovery) => discovery.authorization_endpoint,
        Err(error) => {
            oidc.remove(&state);
            tracing::warn!(%error, "{PROVIDER}");
            return (StatusCode::BAD_GATEWAY, PROVIDER).into_response();
        }
    };
    let url = authorize_url(
        &endpoint,
        &[
            ("response_type", "code"),
            ("client_id", oidc.settings.client_id.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("scope", "openid email"),
            ("state", state.as_str()),
            ("nonce", nonce.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ],
    );
    let mut response = Json(Started {
        url,
        state: state.clone(),
    })
    .into_response();
    if !form.bearer {
        let max_age = i64::try_from(PENDING_FOR.as_secs()).unwrap_or(300);
        let cookie = state_cookie(&state, max_age, bridge.inner.secure_cookie, &bridge.cookie_path());
        match HeaderValue::from_str(&cookie) {
            Ok(value) => {
                response.headers_mut().insert(header::SET_COOKIE, value);
            }
            Err(error) => return super::internal(error),
        }
    }
    response
}

pub(super) async fn callback(
    State(bridge): State<Bridge>,
    headers: HeaderMap,
    extensions: Extensions,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let Some(oidc) = bridge.inner.oidc.as_ref() else {
        return (StatusCode::NOT_FOUND, "single sign-on is not set up").into_response();
    };
    let Some(state) = query.state.as_deref().filter(|state| !state.is_empty()) else {
        return bridge.web_fail(Fail::Failed);
    };
    let Some(bearer) = oidc.bearer(state) else {
        return bridge.web_fail(Fail::Failed);
    };
    if !bearer {
        let given = cookie(&headers, STATE_COOKIE).unwrap_or_default();
        if !same_secret(&given, state) {
            bridge.count_failure(&extensions);
            return bridge.web_fail(Fail::Failed);
        }
    }
    if query.error.as_deref() == Some("access_denied") {
        return bridge.done(oidc, state, bearer, Outcome::Failed(Fail::Failed));
    }
    if query.error.is_some() || query.code.as_deref().unwrap_or("").is_empty() {
        bridge.count_failure(&extensions);
        return bridge.done(oidc, state, bearer, Outcome::Failed(Fail::Failed));
    }
    let (verifier, nonce, redirect) = match oidc.begin(state) {
        Begun::Go {
            verifier,
            nonce,
            redirect,
        } => (verifier, nonce, redirect),
        Begun::Busy => return (StatusCode::CONFLICT, "single sign-on is already finishing").into_response(),
        Begun::Again(outcome) => return bridge.finish(bearer, outcome),
        Begun::Missing => return bridge.web_fail(Fail::Failed),
    };
    let code = query.code.unwrap_or_default();
    let outcome = match exchange(&bridge, oidc, &code, &verifier, &nonce, &redirect).await {
        Ok(account) => match bridge.issue_session(&account, true).await {
            Ok(token) => Outcome::Token(token),
            Err(response) => {
                oidc.settle(state, Outcome::Failed(Fail::Failed));
                return *response;
            }
        },
        Err(fail) => {
            bridge.count_failure(&extensions);
            Outcome::Failed(fail)
        }
    };
    bridge.done(oidc, state, bearer, outcome)
}

pub(super) async fn poll(State(bridge): State<Bridge>, Query(query): Query<PollQuery>) -> Response {
    let Some(oidc) = bridge.inner.oidc.as_ref() else {
        return (StatusCode::NOT_FOUND, "single sign-on is not set up").into_response();
    };
    let Some(state) = query.state.as_deref() else {
        return (StatusCode::NOT_FOUND, FAILED).into_response();
    };
    if oidc.waiting(state) {
        return (StatusCode::ACCEPTED, Json(Still { pending: true })).into_response();
    }
    match oidc.take(state) {
        Some(Outcome::Token(token)) => Json(BearerSession { token }).into_response(),
        Some(Outcome::Failed(fail)) => (fail.status(), fail.text()).into_response(),
        None => (StatusCode::NOT_FOUND, FAILED).into_response(),
    }
}

async fn exchange(
    bridge: &Bridge,
    oidc: &Service,
    code: &str,
    verifier: &str,
    nonce: &str,
    redirect: &str,
) -> Result<Account, Fail> {
    let discovery = match oidc.discovery(bridge.inner.engine.as_ref(), false).await {
        Ok(discovery) => discovery,
        Err(error) => {
            tracing::warn!(%error, "{PROVIDER}");
            return Err(Fail::Provider);
        }
    };
    if !fits(&discovery.issuer) {
        tracing::warn!("the identity provider's issuer does not fit");
        return Err(Fail::Failed);
    }
    let id_token = match token_of(bridge.inner.engine.as_ref(), oidc, &discovery, code, verifier, redirect).await {
        Ok(token) => token,
        Err(error) => {
            tracing::warn!(%error, "{PROVIDER}");
            return Err(Fail::Provider);
        }
    };
    let identity = match checked_token(bridge, oidc, &discovery, &id_token, nonce).await {
        Ok(identity) => identity,
        Err(_) => return Err(Fail::Failed),
    };
    if !fits(&identity.subject) {
        return Err(Fail::Failed);
    }
    let bound = bridge
        .inner
        .store
        .find_oidc(&discovery.issuer, &identity.subject)
        .await
        .map_err(|error| {
            tracing::error!(%error, "a linked sign-in could not be read");
            Fail::Provider
        })?;
    let named = if bound.is_none() {
        bridge
            .inner
            .store
            .accounts_named(&identity.email)
            .await
            .map_err(|error| {
                tracing::error!(%error, "an account could not be found by name");
                Fail::Provider
            })?
    } else {
        Vec::new()
    };
    match choose(bound.as_ref(), &identity.email, identity.email_verified, &named) {
        Choice::SignIn(account) => Ok(account),
        Choice::Bind(account) => match bridge
            .inner
            .store
            .bind_oidc(&discovery.issuer, &identity.subject, &account.id)
            .await
        {
            Ok(OidcBind::Linked) => {
                tracing::info!(account = %account.id, "linked an OpenID Connect sign-in");
                Ok(account)
            }
            Ok(OidcBind::Taken) => Err(Fail::Taken),
            Err(error) => {
                tracing::error!(%error, "a sign-in could not be linked");
                Err(Fail::Provider)
            }
        },
        Choice::NoAccount => Err(Fail::NoAccount),
        Choice::Unverified => Err(Fail::Unverified),
        Choice::Taken => Err(Fail::Taken),
        Choice::Failed => Err(Fail::Failed),
    }
}

async fn checked_token(
    bridge: &Bridge,
    oidc: &Service,
    discovery: &Discovery,
    id_token: &str,
    nonce: &str,
) -> Result<Identity, TokenFail> {
    let jwks = match oidc.jwks(bridge.inner.engine.as_ref(), false).await {
        Ok(jwks) => jwks,
        Err(error) => {
            tracing::warn!(%error, "{PROVIDER}");
            return Err(TokenFail::Rejected);
        }
    };
    match validate_id_token(id_token, &jwks, &oidc.settings.client_id, &discovery.issuer, nonce) {
        Err(TokenFail::UnknownKid) => {}
        other => return other,
    }
    let jwks = match oidc.jwks(bridge.inner.engine.as_ref(), true).await {
        Ok(jwks) => jwks,
        Err(error) => {
            tracing::warn!(%error, "{PROVIDER}");
            return Err(TokenFail::Rejected);
        }
    };
    match validate_id_token(id_token, &jwks, &oidc.settings.client_id, &discovery.issuer, nonce) {
        Err(TokenFail::UnknownKid) => Err(TokenFail::Rejected),
        other => other,
    }
}

impl Service {
    async fn discovery(&self, engine: &dyn HttpEngine, refresh: bool) -> Result<Discovery, String> {
        if !refresh && let Some(cache) = self.fresh() {
            return Ok(cache.discovery);
        }
        let url = format!("{}/.well-known/openid-configuration", self.settings.issuer);
        let body = idp_body(engine, HttpMethod::Get, url, None).await?;
        let discovery: Discovery = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
        if !issuers_match(&self.settings.issuer, &discovery.issuer) {
            return Err(format!(
                "the identity provider says its issuer is {}, not {}",
                discovery.issuer, self.settings.issuer
            ));
        }
        let jwks = fetch_jwks(engine, &discovery.jwks_uri).await?;
        *lock(&self.cache) = Some(Cache {
            at: Instant::now(),
            discovery: discovery.clone(),
            jwks,
        });
        Ok(discovery)
    }

    async fn jwks(&self, engine: &dyn HttpEngine, refresh: bool) -> Result<JwkSet, String> {
        if !refresh && let Some(cache) = self.fresh() {
            return Ok(cache.jwks);
        }
        if refresh && let Some(cache) = self.fresh() {
            let jwks = fetch_jwks(engine, &cache.discovery.jwks_uri).await?;
            let discovery = cache.discovery.clone();
            *lock(&self.cache) = Some(Cache {
                at: Instant::now(),
                discovery,
                jwks: jwks.clone(),
            });
            return Ok(jwks);
        }
        self.discovery(engine, true).await?;
        lock(&self.cache)
            .as_ref()
            .map(|cache| cache.jwks.clone())
            .ok_or_else(|| "the identity provider returned no keys".to_string())
    }

    fn fresh(&self) -> Option<Cache> {
        let cache = lock(&self.cache);
        cache.as_ref().and_then(|cache| {
            (cache.at.elapsed() < CACHE_FOR).then(|| Cache {
                at: cache.at,
                discovery: cache.discovery.clone(),
                jwks: cache.jwks.clone(),
            })
        })
    }
}

fn issuers_match(configured: &str, published: &str) -> bool {
    normalize_issuer(published).is_ok_and(|normalized| normalized == configured)
}

async fn fetch_jwks(engine: &dyn HttpEngine, url: &str) -> Result<JwkSet, String> {
    let body = idp_body(engine, HttpMethod::Get, url.to_string(), None).await?;
    serde_json::from_slice(&body).map_err(|error| error.to_string())
}

async fn token_of(
    engine: &dyn HttpEngine,
    oidc: &Service,
    discovery: &Discovery,
    code: &str,
    verifier: &str,
    redirect: &str,
) -> Result<String, String> {
    let body = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&client_secret={}&code_verifier={}",
        form_encode(code),
        form_encode(redirect),
        form_encode(&oidc.settings.client_id),
        form_encode(&oidc.settings.client_secret),
        form_encode(verifier),
    );
    let bytes = idp_body(
        engine,
        HttpMethod::Post,
        discovery.token_endpoint.clone(),
        Some(body.into_bytes()),
    )
    .await?;
    let parsed: TokenResponse = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    parsed
        .id_token
        .ok_or_else(|| "the identity provider returned no id token".into())
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
}

async fn idp_body(
    engine: &dyn HttpEngine,
    method: HttpMethod,
    url: String,
    body: Option<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    let mut headers = Vec::new();
    let body = body.unwrap_or_default();
    if !body.is_empty() {
        headers.push((
            "content-type".to_string(),
            "application/x-www-form-urlencoded".to_string(),
        ));
    }
    let request = PreparedRequest {
        method,
        url,
        headers,
        body,
        timeouts: Timeouts {
            connect: 10,
            write: 10,
            read: 10,
        },
        accept_invalid_certs: false,
        ..Default::default()
    };
    let exchange = engine.send(request).await.map_err(|error| error.to_string())?;
    if !(200..300).contains(&exchange.status) {
        return Err(format!("the identity provider answered {}", exchange.status));
    }
    if exchange.body.len() > IDP_BODY_LIMIT {
        return Err("the identity provider's answer is too large".into());
    }
    Ok(exchange.body)
}

fn authorize_url(endpoint: &str, query: &[(&str, &str)]) -> String {
    let mut url = String::from(endpoint);
    url.push(if endpoint.contains('?') { '&' } else { '?' });
    for (index, (key, value)) in query.iter().enumerate() {
        if index > 0 {
            url.push('&');
        }
        url.push_str(key);
        url.push('=');
        url.push_str(&form_encode(value));
    }
    url
}

fn fits(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= ID_LIMIT
}

/// Where the provider sends the browser back. A configured URL is kept for
/// the token request; otherwise the forwarding headers, then `Host`.
fn callback_url(bridge: &Bridge, headers: &HeaderMap, configured: Option<&str>) -> Result<String, &'static str> {
    if let Some(configured) = configured {
        return Ok(configured.to_string());
    }
    let forwarded = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let host = forwarded("x-forwarded-host")
        .or_else(|| forwarded("host"))
        .ok_or("the sign-in address could not be decided")?;
    if host
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '/' | '\\' | '@' | '?'))
    {
        return Err("the sign-in address could not be decided");
    }
    let scheme = match forwarded("x-forwarded-proto") {
        Some(scheme) if scheme == "http" || scheme == "https" => scheme,
        Some(_) => return Err("the sign-in address could not be decided"),
        None if bridge.inner.secure_cookie => "https".to_string(),
        None => "http".to_string(),
    };
    Ok(format!("{scheme}://{host}{}/v1/oidc/callback", bridge.inner.base_path))
}

fn state_cookie(value: &str, max_age: i64, secure: bool, path: &str) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{STATE_COOKIE}={value}; Path={path}; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")
}

impl Bridge {
    fn count_failure(&self, extensions: &Extensions) {
        if let Some(key) = super::address(extensions) {
            self.inner.throttle.failed(&[key], Instant::now());
        }
    }

    fn web_fail(&self, fail: Fail) -> Response {
        self.redirect(Some(fail.query()), None)
    }

    /// A browser sign-in is answered at once, so its attempt — and the
    /// session token — is not kept. A desktop client polls for the token.
    fn done(&self, oidc: &Service, state: &str, bearer: bool, outcome: Outcome) -> Response {
        if bearer {
            oidc.settle(state, outcome.clone());
        } else {
            oidc.remove(state);
        }
        self.finish(bearer, outcome)
    }

    fn finish(&self, bearer: bool, outcome: Outcome) -> Response {
        if bearer {
            let signed_in = matches!(outcome, Outcome::Token(_));
            return html_page(signed_in);
        }
        match outcome {
            Outcome::Token(token) => self.redirect(None, Some(&token)),
            Outcome::Failed(Fail::Provider) => {
                let mut response = (StatusCode::BAD_GATEWAY, PROVIDER).into_response();
                let cookie = state_cookie("", 0, self.inner.secure_cookie, &self.cookie_path());
                if let Ok(value) = HeaderValue::from_str(&cookie) {
                    response.headers_mut().append(header::SET_COOKIE, value);
                }
                response
            }
            Outcome::Failed(fail) => self.web_fail(fail),
        }
    }

    fn redirect(&self, query: Option<&str>, session: Option<&str>) -> Response {
        let location = match query {
            Some(query) => format!("{}/?{query}", self.inner.base_path),
            None => format!("{}/", self.inner.base_path),
        };
        let mut cookies = Vec::new();
        if let Some(token) = session {
            cookies.push(session_cookie(
                token,
                Some(super::KEPT_SESSION),
                self.inner.secure_cookie,
                &self.cookie_path(),
            ));
        }
        cookies.push(state_cookie("", 0, self.inner.secure_cookie, &self.cookie_path()));
        see_other(&location, &cookies)
    }
}

fn see_other(location: &str, cookies: &[String]) -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(location) {
        headers.insert(header::LOCATION, value);
    }
    for cookie in cookies {
        if let Ok(value) = HeaderValue::from_str(cookie) {
            headers.append(header::SET_COOKIE, value);
        }
    }
    (StatusCode::SEE_OTHER, headers).into_response()
}

fn html_page(signed_in: bool) -> Response {
    let (english, chinese) = if signed_in {
        (
            "Signed in. You can close this window and return to Cyber API.",
            "已登录。可以关闭这个窗口，回到 Cyber API。",
        )
    } else {
        (
            "Single sign-on did not finish. You can close this window and return to Cyber API.",
            "单点登录没有完成。可以关闭这个窗口，回到 Cyber API。",
        )
    };
    let body =
        format!("<!doctype html><meta charset=\"utf-8\"><title>Cyber API</title><p>{english}</p><p>{chinese}</p>");
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response()
}

enum Choice {
    SignIn(Account),
    Bind(Account),
    NoAccount,
    Unverified,
    Taken,
    Failed,
}

/// An enabled linked subject signs in. A disabled one does not fall through
/// onto another account. Otherwise one enabled account whose name is the
/// verified email, and which is not linked yet, is linked.
fn choose(bound: Option<&OidcAccount>, email: &str, verified: bool, named: &[NamedAccount]) -> Choice {
    match bound {
        Some(found) if !found.disabled => return Choice::SignIn(found.account.clone()),
        Some(_) => return Choice::Failed,
        None => {}
    }
    if email.is_empty() {
        return Choice::NoAccount;
    }
    let matches: Vec<_> = named
        .iter()
        .filter(|item| item.account.name.eq_ignore_ascii_case(email))
        .collect();
    match *matches.as_slice() {
        [] => Choice::NoAccount,
        [one] if one.linked => Choice::Taken,
        [_] if !verified => Choice::Unverified,
        [one] => Choice::Bind(one.account.clone()),
        _ => Choice::Failed,
    }
}

struct Identity {
    subject: String,
    email: String,
    email_verified: bool,
}

#[derive(Debug)]
enum TokenFail {
    Rejected,
    UnknownKid,
}

#[derive(Deserialize)]
struct IdClaims {
    sub: String,
    #[serde(default)]
    nonce: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    email_verified: EmailFlag,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EmailFlag {
    Bool(bool),
    Text(String),
}

impl Default for EmailFlag {
    fn default() -> Self {
        Self::Bool(false)
    }
}

impl EmailFlag {
    fn verified(&self) -> bool {
        match self {
            Self::Bool(value) => *value,
            Self::Text(value) => value == "true",
        }
    }
}

fn accepted(alg: Algorithm) -> bool {
    matches!(
        alg,
        Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::ES256
            | Algorithm::ES384
    )
}

/// RSA and EC only. `none`, HMAC and EdDSA are refused. A missing `kid`
/// is accepted when the set has exactly one key.
fn validate_id_token(
    token: &str,
    jwks: &JwkSet,
    audience: &str,
    issuer: &str,
    nonce: &str,
) -> Result<Identity, TokenFail> {
    let header = decode_header(token).map_err(|_| TokenFail::Rejected)?;
    if !accepted(header.alg) {
        return Err(TokenFail::Rejected);
    }
    let jwk = match header.kid.as_deref() {
        Some(kid) => jwks
            .keys
            .iter()
            .find(|key| key.common.key_id.as_deref() == Some(kid))
            .ok_or(TokenFail::UnknownKid)?,
        None if jwks.keys.len() == 1 => &jwks.keys[0],
        None => return Err(TokenFail::Rejected),
    };
    let key = DecodingKey::from_jwk(jwk).map_err(|_| TokenFail::Rejected)?;
    let mut validation = Validation::new(header.alg);
    validation.set_audience(&[audience]);
    validation.set_issuer(&[issuer]);
    let data = decode::<IdClaims>(token, &key, &validation).map_err(|_| TokenFail::Rejected)?;
    if data.claims.sub.is_empty() || !same_secret(&data.claims.nonce, nonce) {
        return Err(TokenFail::Rejected);
    }
    Ok(Identity {
        subject: data.claims.sub,
        email: data.claims.email,
        email_verified: data.claims.email_verified.verified(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};

    const PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDRplhAo1gLGYrm\n\
bT6l6jK69uP6NaP5LARK5jh06+XXsOhAfFv5U4wx6MY1zoczzPlQDsY3zfYTqoMa\n\
uAYOpGI0PUWu3tohkc+zp+reYKZFzKJ/EsUVwGjaCjLOMTR6MRWuO4mdKGCJlbTm\n\
gMG5YkZUIwwGaiHxXoiuxC2AoV8UGUmkqRR6A9gScr7ULOj2aMLVK/E5jCZUtC7A\n\
pD17nhWyGIu9hgNBBzeBhqYK4QBCfQE7Tb6ikdITdq2ZgCJ2a6BAEDqeP29LCL7t\n\
8qXT1uvBVRAq93ULaWOmZWigTAY+X9bVs9Z786BTqhTVokYi2L73Z6TElRV8RmBJ\n\
GFD3S5uPAgMBAAECggEAAxtqu7TLdojvqVkrIqqlL9R1Le+1S2uoAupy2JwxQ2vN\n\
EM+ZTe3yV9JyDrk7k3NjhfQUYx57md4oEdYEpq15Sx7oNVVoeMV3KdvAhTd8DRUP\n\
LS4X5Dw1EZvoOkgVQ3Lp7Opp1P25M/nyvNTz98Rk89379hGE5LEGaCHQd/f+kuJe\n\
uzaJv66iV5KZNj/eCHCCEWLm9kB8s/9DzbcjA5OOT0rNvy33kBV47mB7WGygoQcm\n\
r3iTzw9uSa32ntAajqg3wk2d24pVKXFw7nZwuWo7mFgr0+Lna/tzzDk4vtNUKhQ3\n\
pVgLopmsDvMOa7qcLECYDpiR6/4m/zfMbAmXEl8kcQKBgQD/UjaYwYHzMEr5V6a8\n\
PLxpDhYKgoLz90idLlRkSVzjpJUEsHmAowDyRHPsi1dQWjD1Fdis4LSp8Gj0EL4s\n\
ceACUw97AP/xrqokw6sBUdQCJzLWvf2Ttj5Mjf7d5t+m0Pyxrws4oSKDPOkjGsud\n\
rnPzumEvZ5lAi+EeHRQaLYFzaQKBgQDSNQt50VUeZlTl+6fzL8KB3iyqIQXIPseH\n\
+a8bA2pN9Ku68M899pL5aIdB3Oxl74JOGa2WY9Hh2RzuwjaC3SUN9J9pbHm0arKL\n\
R7OUu14avLkD5/uH8BEOEZ4zW5x1MiwOfKAmeff8WwZ3YLpLObeV8pJG067MGyXC\n\
FViterNQNwKBgQCMLAqlWreJV94Rfjmf5ohZ9M3M/1H9tqk7AhB67nv8zoKxjGFo\n\
ahU+7K3oE6fgCWQPaesHHmaRVdeJBxghNnORUyCQmz/o48etX28mzlHizLpwguYQ\n\
jmEtjld6ZCItZ1cUBdHXST+1ZV9pUnCvJPOrv4tTh0Dv2tvAuJ4ssQb84QKBgDfs\n\
3OzAAMFhwFyfo4NTxY//qCprNrqb2UIX77pkIjgzTC0kPD7kBx5L7RBc0pgcLLLS\n\
qhhE3mYT+VqcWRxf8afQ2UiXD5oseOwNW33Ki7LZWu/rHESCXkc78LcBPJMBki8Y\n\
gRM7h/qfzQyds3KoHGvMaZZmYy7yWg1U2I7vAh3pAoGAIjd/BMM49YgebSDIzV0H\n\
F+Oyt4dXnKezjuYAluqg6Bs7fcTH7kJuBAULkXJ34ufk23OCSHPq6TiAWJ0KFMnK\n\
bTR6ECL51BkeG31zrLUta3d18wFoUswt6DKeZ8Mmfe35wGdVjc2qYdkshD03vVrN\n\
6e5ffUYneSq6ZoZLQUaEsZw=\n\
-----END PRIVATE KEY-----\n";

    const N: &str = "0aZYQKNYCxmK5m0-peoyuvbj-jWj-SwESuY4dOvl17DoQHxb-VOMMejGNc6HM8z5UA7GN832E6qDGrgGDqRiND1Frt7aIZHPs6fq3mCmRcyifxLFFcBo2goyzjE0ejEVrjuJnShgiZW05oDBuWJGVCMMBmoh8V6IrsQtgKFfFBlJpKkUegPYEnK-1Czo9mjC1SvxOYwmVLQuwKQ9e54VshiLvYYDQQc3gYamCuEAQn0BO02-opHSE3atmYAidmugQBA6nj9vSwi-7fKl09brwVUQKvd1C2ljpmVooEwGPl_W1bPWe_OgU6oU1aJGIti-92ekxJUVfEZgSRhQ90ubjw";

    fn jwks() -> JwkSet {
        serde_json::from_str(&format!(
            r#"{{"keys":[{{"kty":"RSA","kid":"test","alg":"RS256","n":"{N}","e":"AQAB"}}]}}"#
        ))
        .expect("a jwks")
    }

    fn account(name: &str) -> Account {
        Account {
            id: cyberapi_core::AccountId::new("account-1"),
            name: name.to_string(),
            display_name: name.to_string(),
            admin: false,
        }
    }

    fn sign(claims: &[(&str, serde_json::Value)], kid: Option<&str>) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = kid.map(str::to_string);
        let mut body = serde_json::Map::new();
        for (key, value) in claims {
            body.insert((*key).to_string(), value.clone());
        }
        let key = EncodingKey::from_rsa_pem(PEM.as_bytes()).expect("a key");
        encode(&header, &serde_json::Value::Object(body), &key).expect("a token")
    }

    fn sample(extra: &[(&str, serde_json::Value)]) -> String {
        let mut claims = vec![
            ("iss", serde_json::json!("https://idp.example")),
            ("sub", serde_json::json!("subject-1")),
            ("aud", serde_json::json!("client")),
            ("exp", serde_json::json!(4_000_000_000_u64)),
            ("nonce", serde_json::json!("nonce-1")),
            ("email", serde_json::json!("ada@example.com")),
            ("email_verified", serde_json::json!(true)),
        ];
        for (key, value) in extra {
            if let Some(found) = claims.iter_mut().find(|(name, _)| *name == *key) {
                found.1 = value.clone();
            } else {
                claims.push((key, value.clone()));
            }
        }
        sign(&claims, Some("test"))
    }

    #[test]
    fn pkce_matches_the_rfc_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(code_challenge(verifier), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn an_issuer_is_an_http_url_without_a_user() {
        assert_eq!(
            normalize_issuer(" https://idp.example/realms/app/ ").expect("issuer"),
            "https://idp.example/realms/app"
        );
        assert!(normalize_issuer("idp.example").is_err());
        assert!(normalize_issuer("https://user:secret@idp.example").is_err());
        assert!(normalize_issuer("https://idp.example/?x=1").is_err());
        assert!(normalize_issuer("https://idp.example/#frag").is_err());
    }

    #[test]
    fn the_three_settings_come_together() {
        assert!(configure(None, None, None, None).expect("unset").is_none());
        assert!(configure(Some("https://idp.example".into()), None, None, None).is_err());
        let settings = configure(
            Some(" https://idp.example/ ".into()),
            Some(" client ".into()),
            Some(" secret ".into()),
            None,
        )
        .expect("set")
        .expect("on");
        assert_eq!(settings.issuer(), "https://idp.example");
    }

    #[test]
    fn a_disabled_subject_does_not_bind_someone_else() {
        let ada = account("Ada@Example.com");
        let bound = OidcAccount {
            account: ada.clone(),
            disabled: true,
        };
        assert!(matches!(
            choose(Some(&bound), "ada@example.com", true, &[]),
            Choice::Failed
        ));
        let enabled = OidcAccount {
            account: ada.clone(),
            disabled: false,
        };
        assert!(matches!(choose(Some(&enabled), "", false, &[]), Choice::SignIn(_)));
    }

    #[test]
    fn an_email_links_one_unbound_account() {
        let ada = account("Ada@Example.com");
        let named = [NamedAccount {
            account: ada,
            linked: false,
        }];
        assert!(matches!(choose(None, "ada@example.com", true, &named), Choice::Bind(_)));
        assert!(matches!(
            choose(None, "ada@example.com", false, &named),
            Choice::Unverified
        ));
        assert!(matches!(choose(None, "", true, &named), Choice::NoAccount));
        let linked = [NamedAccount {
            account: account("Ada@Example.com"),
            linked: true,
        }];
        assert!(matches!(choose(None, "ada@example.com", true, &linked), Choice::Taken));
        let two = [
            NamedAccount {
                account: account("Ada@Example.com"),
                linked: false,
            },
            NamedAccount {
                account: account("ada@example.com"),
                linked: false,
            },
        ];
        assert!(matches!(choose(None, "ada@example.com", true, &two), Choice::Failed));
    }

    #[test]
    fn a_token_is_refused_for_the_usual_reasons() {
        let keys = jwks();
        let good = validate_id_token(&sample(&[]), &keys, "client", "https://idp.example", "nonce-1").expect("valid");
        assert_eq!(good.subject, "subject-1");
        assert!(good.email_verified);

        let as_text = sample(&[("email_verified", serde_json::json!("true"))]);
        assert!(
            validate_id_token(&as_text, &keys, "client", "https://idp.example", "nonce-1")
                .expect("text")
                .email_verified
        );

        let unverified = sample(&[("email_verified", serde_json::json!(false))]);
        assert!(
            !validate_id_token(&unverified, &keys, "client", "https://idp.example", "nonce-1")
                .expect("flag")
                .email_verified
        );

        assert!(validate_id_token(&sample(&[]), &keys, "other", "https://idp.example", "nonce-1").is_err());
        assert!(validate_id_token(&sample(&[]), &keys, "client", "https://other.example", "nonce-1").is_err());
        assert!(
            validate_id_token(
                &sample(&[("exp", serde_json::json!(10))]),
                &keys,
                "client",
                "https://idp.example",
                "nonce-1"
            )
            .is_err()
        );
        assert!(validate_id_token(&sample(&[]), &keys, "client", "https://idp.example", "other-nonce").is_err());

        let mut broken = sample(&[]);
        broken.pop();
        broken.push('A');
        assert!(validate_id_token(&broken, &keys, "client", "https://idp.example", "nonce-1").is_err());

        let hmac = format!(
            "{}.{}.sig",
            URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256"}"#),
            URL_SAFE_NO_PAD.encode(r#"{"sub":"x"}"#)
        );
        assert!(matches!(
            validate_id_token(&hmac, &keys, "client", "https://idp.example", "nonce-1"),
            Err(TokenFail::Rejected)
        ));
        let none = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#),
            URL_SAFE_NO_PAD.encode(r#"{"sub":"x"}"#)
        );
        assert!(validate_id_token(&none, &keys, "client", "https://idp.example", "nonce-1").is_err());

        let mut two = keys.clone();
        let mut extra = two.keys[0].clone();
        extra.common.key_id = Some("other".into());
        two.keys.push(extra);
        let no_kid = sign(
            &[
                ("iss", serde_json::json!("https://idp.example")),
                ("sub", serde_json::json!("subject-1")),
                ("aud", serde_json::json!("client")),
                ("exp", serde_json::json!(4_000_000_000_u64)),
                ("nonce", serde_json::json!("nonce-1")),
            ],
            None,
        );
        assert!(matches!(
            validate_id_token(&no_kid, &two, "client", "https://idp.example", "nonce-1"),
            Err(TokenFail::Rejected)
        ));
        assert!(validate_id_token(&no_kid, &keys, "client", "https://idp.example", "nonce-1").is_ok());
        assert!(matches!(
            validate_id_token(&sample(&[]), &two, "client", "https://idp.example", "nonce-1"),
            Err(TokenFail::UnknownKid) | Ok(_)
        ));
    }
}
