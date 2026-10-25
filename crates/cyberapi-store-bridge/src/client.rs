//! The bridge for a client that is not its page — the desktop
//! (`docs/server-sync.md` §1, step 5). The page's login is a cookie the
//! page's scripts never see; such a client asks for the session's token
//! instead ([`LoginForm::bearer`]), keeps it, and sends it back as a bearer
//! token. Its calls go out on any [`HttpEngine`]: the desktop's own.

use crate::{Response, STORE_PATH, Transport};
use cyberapi_core::directory::Me;
use cyberapi_core::model::HttpMethod;
use cyberapi_core::prefs::{HttpVersion, Timeouts};
use cyberapi_http::{HttpEngine, PreparedRequest};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub const LOGIN_PATH: &str = "/v1/login";
pub const LOGOUT_PATH: &str = "/v1/logout";

/// A sign-in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginForm {
    pub name: String,
    pub password: String,
    /// Answer the session's token in the body, and set no cookie.
    #[serde(default)]
    pub bearer: bool,
    /// The page's "keep me signed in": the session lasts its 30 days.
    /// Off, it ends with the browser, and on the bridge within the day. A
    /// client that does not say keeps it, as a bearer session always does.
    #[serde(default = "kept")]
    pub remember: bool,
}

fn kept() -> bool {
    true
}

/// A bearer sign-in's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BearerSession {
    pub token: String,
}

/// Why a sign-in gave no session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignInError {
    WrongLogin,
    /// Too many failed tries: the bridge waits this many seconds.
    TooManyTries(u64),
    /// Nothing usable came back: why.
    Failed(String),
}

/// A bridge's address as typed: `http(s)://host[:port][/base]`, without a
/// slash at the end. `None` for anything else.
pub fn bridge_address(input: &str) -> Option<String> {
    let address = input.trim().trim_end_matches('/');
    let rest = address
        .strip_prefix("https://")
        .or_else(|| address.strip_prefix("http://"))?;
    let host = rest.split('/').next().unwrap_or_default();
    (!host.is_empty() && !address.chars().any(char::is_whitespace)).then(|| address.to_string())
}

/// The bridge at an address, through `engine`, signed in as `token`.
#[derive(Clone)]
pub struct EngineTransport {
    engine: Arc<dyn HttpEngine>,
    address: String,
    token: Option<String>,
}

impl EngineTransport {
    pub fn new(engine: Arc<dyn HttpEngine>, address: String, token: Option<String>) -> Self {
        Self { engine, address, token }
    }
}

impl Transport for EngineTransport {
    fn post(
        &self,
        path: &'static str,
        body: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<Response, String>> + Send>> {
        let (model, version) = crate::model_header();
        let mut headers = vec![
            ("content-type".to_string(), "application/json".to_string()),
            // Which shape this client keeps projects in (§2.1).
            (model.to_string(), version),
        ];
        if let Some(token) = &self.token {
            headers.push(("authorization".to_string(), format!("Bearer {token}")));
        }
        let request = PreparedRequest {
            method: HttpMethod::Post,
            url: format!("{}{path}", self.address),
            headers,
            body,
            timeouts: Timeouts::default().effective(),
            // The account's password and session go here: the server's
            // certificate is checked whatever the reader's requests take.
            accept_invalid_certs: false,
            http_version: HttpVersion::Auto,
            streams: false,
            // The environment's proxy, as curl would take it (#60).
            proxy: Default::default(),
        };
        let call = self.engine.send(request);
        Box::pin(async move {
            let exchange = call.await.map_err(|error| error.to_string())?;
            Ok(Response {
                status: exchange.status,
                body: exchange.body,
            })
        })
    }
}

/// Signs in to the bridge at `address`; answers the session's token.
pub async fn sign_in(engine: Arc<dyn HttpEngine>, address: String, form: LoginForm) -> Result<String, SignInError> {
    let transport = EngineTransport::new(engine, address, None);
    let body = serde_json::to_vec(&LoginForm { bearer: true, ..form })
        .map_err(|error| SignInError::Failed(error.to_string()))?;
    let response = transport.post(LOGIN_PATH, body).await.map_err(SignInError::Failed)?;
    match response.status {
        200 => serde_json::from_slice::<BearerSession>(&response.body)
            .map(|session| session.token)
            .map_err(|error| SignInError::Failed(error.to_string())),
        401 => Err(SignInError::WrongLogin),
        429 => Err(SignInError::TooManyTries(60)),
        status => Err(SignInError::Failed(format!(
            "the bridge answered {status}: {}",
            String::from_utf8_lossy(&response.body)
        ))),
    }
}

/// Who the session is; `None` when it is not one any more (signed out,
/// expired, the account disabled). An error when the bridge did not say.
pub async fn session_me(transport: &dyn Transport) -> Result<Option<Me>, String> {
    let body = serde_json::to_vec(&crate::Call::DirectoryMe).map_err(|error| error.to_string())?;
    let response = transport.post(STORE_PATH, body).await?;
    match response.status {
        200 => match serde_json::from_slice(&response.body).map_err(|error| error.to_string())? {
            crate::Reply::Ok(value) => serde_json::from_value(value)
                .map(Some)
                .map_err(|error| error.to_string()),
            crate::Reply::Err(error) => Err(error.to_string()),
        },
        401 => Ok(None),
        status => Err(format!(
            "the bridge answered {status}: {}",
            String::from_utf8_lossy(&response.body)
        )),
    }
}

/// Ends the session at the bridge.
pub async fn sign_out(transport: &dyn Transport) -> Result<(), String> {
    let response = transport.post(LOGOUT_PATH, Vec::new()).await?;
    match response.status {
        200..300 | 401 => Ok(()),
        status => Err(format!("the bridge answered {status}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_http_with_a_host() {
        assert_eq!(
            bridge_address(" https://api.test/ "),
            Some("https://api.test".to_string())
        );
        assert_eq!(
            bridge_address("http://127.0.0.1:7380/cyber/"),
            Some("http://127.0.0.1:7380/cyber".to_string())
        );
        assert_eq!(bridge_address("api.test"), None);
        assert_eq!(bridge_address("https://"), None);
        assert_eq!(bridge_address("ftp://api.test"), None);
        assert_eq!(bridge_address("https://api .test"), None);
    }
}
