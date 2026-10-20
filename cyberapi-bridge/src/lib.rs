//! The server of CyberAPI's browser build (`docs/architecture.md` §2.1, §9.5):
//! it signs accounts in, answers the page's store calls on the account's
//! scoped store, and serves the page itself — one origin for all three, so
//! the login cookie can stay `SameSite=Strict`.
//!
//! | Route | |
//! |---|---|
//! | `GET /v1/setup` | whether the first administrator is still to be made |
//! | `POST /v1/setup` | makes it, with the setup token, and signs it in |
//! | `POST /v1/login`, `POST /v1/logout` | the session cookie |
//! | `GET /v1/session` | who is signed in, or 401 |
//! | `POST /v1/store` | one store call (`cyberapi_store_bridge::Call`) |
//! | anything else | the page's files |

mod auth;

use auth::{SESSION_COOKIE, hash_password, hash_token, new_token, session_cookie, signed_in, verify_password};
use axum::Router;
use axum::extract::{Json, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use cyberapi_store_bridge::{Call, STORE_PATH, dispatch};
use cyberapi_store_sql::{Account, NewAccount, SqlStore};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::ServeDir;

/// How long a login lasts.
pub const SESSION_DAYS: i64 = 30;
const MIN_PASSWORD_CHARS: usize = 8;
const MAX_NAME_CHARS: usize = 64;

#[derive(Clone)]
pub struct Bridge {
    inner: Arc<Inner>,
}

struct Inner {
    store: SqlStore,
    /// What `POST /v1/setup` must be shown, while no account exists.
    setup_token: String,
    /// Mark the cookie `Secure`: on, unless the bridge is reached over plain
    /// HTTP (development).
    secure_cookie: bool,
}

impl Bridge {
    pub fn new(store: SqlStore, setup_token: String, secure_cookie: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                setup_token,
                secure_cookie,
            }),
        }
    }

    pub fn store(&self) -> &SqlStore {
        &self.inner.store
    }

    /// The routes, with the page's files from `www` behind them.
    pub fn router(&self, www: Option<PathBuf>) -> Router {
        let api = Router::new()
            .route("/v1/setup", get(setup_needed).post(setup))
            .route("/v1/login", post(login))
            .route("/v1/logout", post(logout))
            .route("/v1/session", get(session))
            .route(STORE_PATH, post(store_call))
            .with_state(self.clone());
        match www {
            Some(dir) => api.fallback_service(ServeDir::new(dir)),
            None => api,
        }
    }

    /// Signs `account` in, answering it with the cookie that carries the
    /// new session.
    async fn sign_in(&self, account: Account) -> Response {
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

/// The first administrator, made with the setup token — which counts only
/// while there is no account at all.
async fn setup(State(bridge): State<Bridge>, Json(form): Json<SetupForm>) -> Response {
    match bridge.inner.store.account_count().await {
        Ok(0) => {}
        Ok(_) => return (StatusCode::CONFLICT, "the bridge already has accounts").into_response(),
        Err(error) => return internal(error),
    }
    if !auth::same_secret(&form.token, &bridge.inner.setup_token) {
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
        .create_account(NewAccount {
            name: form.name.clone(),
            display_name: display_name.clone(),
            password_hash: hash,
            admin: true,
        })
        .await
    {
        Ok(id) => id,
        Err(error) => return internal(error),
    };
    let account = Account {
        id,
        name: form.name,
        display_name,
        admin: true,
    };
    bridge.sign_in(account).await
}

fn check_new_account(name: &str, password: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() || name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_whitespace) {
        return Err("a name is 1 to 64 characters, with no spaces");
    }
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err("a password has at least 8 characters");
    }
    Ok(())
}

#[derive(Deserialize)]
struct LoginForm {
    name: String,
    password: String,
}

async fn login(State(bridge): State<Bridge>, Json(form): Json<LoginForm>) -> Response {
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
        Some(account) if verified => bridge.sign_in(account).await,
        _ => (StatusCode::UNAUTHORIZED, "wrong name or password").into_response(),
    }
}

async fn logout(State(bridge): State<Bridge>, headers: HeaderMap) -> Response {
    if let Some(token) = auth::cookie(&headers, SESSION_COOKIE)
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
    let storage = bridge.inner.store.scoped(account.id);
    Json(dispatch(&storage, call).await).into_response()
}
