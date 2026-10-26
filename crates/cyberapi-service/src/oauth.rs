//! C4: the OAuth 2.0 tokens a request is signed with (`cyberapi_core::oauth`).
//! They are the reader's, like cookies, but kept only in memory while the
//! app runs: never in the project, its sync or a backup. A token is asked
//! for the first time a request needs it, asked again once it is out of
//! date — with its refresh token first, where it came with one — and once
//! the API answers 401 to a kept one.

use crate::send::Sender;
use cyberapi_core::certificates;
use cyberapi_core::oauth::{self, OAuthLogin, Token};
use cyberapi_core::send::PreparedRequest;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use web_time::Instant;

/// A token this close to its end is asked for again: it might end on its
/// way.
const MARGIN: Duration = Duration::from_secs(30);

/// The tokens had, by what they were asked with: one per token endpoint,
/// client, user and scope — and secret, so a changed one asks anew.
#[derive(Default)]
pub struct Tokens(Mutex<HashMap<OAuthLogin, Kept>>);

struct Kept {
    token: Token,
    /// When it ends; none, until the API refuses it.
    ends: Option<Instant>,
}

impl Tokens {
    /// The access token kept for `login`, while it has not ended.
    fn current(&self, login: &OAuthLogin) -> Option<String> {
        let kept = self.0.lock().ok()?;
        let kept = kept.get(login)?;
        kept.ends
            .is_none_or(|ends| Instant::now() + MARGIN < ends)
            .then(|| kept.token.access_token.clone())
    }

    fn refresh_token(&self, login: &OAuthLogin) -> Option<String> {
        self.0.lock().ok()?.get(login)?.token.refresh_token.clone()
    }

    fn keep(&self, login: &OAuthLogin, mut token: Token, asked: Instant) {
        let ends = token.expires_in.map(|seconds| asked + Duration::from_secs(seconds));
        if let Ok(mut kept) = self.0.lock() {
            // A refresh that gave no new refresh token leaves the old one.
            if token.refresh_token.is_none() {
                token.refresh_token = kept.get(login).and_then(|kept| kept.token.refresh_token.clone());
            }
            kept.insert(login.clone(), Kept { token, ends });
        }
    }

    fn forget(&self, login: &OAuthLogin) {
        if let Ok(mut kept) = self.0.lock() {
            kept.remove(login);
        }
    }
}

/// How an OAuth request came by its token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Signed {
    /// It signs in otherwise.
    No,
    /// The one kept: the API may have stopped taking it.
    Kept,
    Asked,
}

/// C4: `prepared` with `Authorization: Bearer` and its token — the one
/// kept, unless it has ended or `anew` says the API refused it; else one
/// asked for and kept. `Err` says why there is none.
pub(crate) async fn sign(sender: &Sender, prepared: &mut PreparedRequest, anew: bool) -> Result<Signed, String> {
    let Some(login) = prepared.oauth.clone() else {
        return Ok(Signed::No);
    };
    let kept = if anew { None } else { sender.tokens.current(&login) };
    let (token, signed) = match kept {
        Some(token) => (token, Signed::Kept),
        None => (ask(sender, &login, prepared).await?, Signed::Asked),
    };
    authorize(prepared, &token);
    Ok(signed)
}

/// F7.8: `prepared` signed with the token kept, if there is one; else
/// with words saying where one is had, for the code to be completed.
pub(crate) fn sign_kept(sender: &Sender, prepared: &mut PreparedRequest) {
    if let Some(login) = &prepared.oauth {
        let token = sender
            .tokens
            .current(login)
            .unwrap_or_else(|| format!("<an access token from {}>", login.token_url));
        authorize(prepared, &token);
    }
}

fn authorize(prepared: &mut PreparedRequest, token: &str) {
    prepared
        .headers
        .retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
    prepared.headers.push(("Authorization".into(), oauth::bearer(token)));
}

/// A token for `login`, asked of its endpoint as `api` would be sent and
/// kept: with the refresh token kept, if there is one and it still works;
/// else with the grant.
async fn ask(sender: &Sender, login: &OAuthLogin, api: &PreparedRequest) -> Result<String, String> {
    let at = login.token_url.as_str();
    if !matches!(url::Url::parse(at), Ok(url) if matches!(url.scheme(), "http" | "https")) {
        return Err(match at.is_empty() {
            true => "OAuth 2.0: there is no token URL".to_string(),
            false => format!("OAuth 2.0: the token URL {at} is no http or https address"),
        });
    }
    if let Some(refresh) = sender.tokens.refresh_token(login) {
        let asked = Instant::now();
        if let Ok(token) = ask_with(sender, login, api, Some(&refresh)).await {
            let access = token.access_token.clone();
            sender.tokens.keep(login, token, asked);
            return Ok(access);
        }
    }
    sender.tokens.forget(login);
    let asked = Instant::now();
    let token = ask_with(sender, login, api, None)
        .await
        .map_err(|why| format!("OAuth 2.0: no token from {at}: {why}"))?;
    let access = token.access_token.clone();
    sender.tokens.keep(login, token, asked);
    Ok(access)
}

async fn ask_with(
    sender: &Sender,
    login: &OAuthLogin,
    api: &PreparedRequest,
    refresh: Option<&str>,
) -> Result<Token, String> {
    let mut asking = login.token_request(api, refresh);
    asking.tls = certificates::tls_for(&sender.sending.certificates, &asking.url);
    let exchange = sender.engine.send(asking).await.map_err(|error| error.to_string())?;
    oauth::read_token(exchange.status, &exchange.body)
}
