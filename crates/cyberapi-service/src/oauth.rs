//! C4: the OAuth 2.0 tokens a request is signed with (`cyberapi_core::oauth`).
//! They are the reader's, kept as cookies are: the app's in the reader's
//! store (`PrefStore::oauth_tokens`, read once, written whole after each
//! change), never in the project, its sync or a backup; a run's on the
//! command line and an AI assistant's call only in memory. A token is asked
//! for the first time a request needs it, asked again once it is out of
//! date — with its refresh token first, where it came with one — and once
//! the API answers 401 to a kept one.

use crate::send::Sender;
use cyberapi_core::certificates;
use cyberapi_core::oauth::{self, KeptToken, KeptTokens, OAuthLogin, Token};
use cyberapi_core::send::PreparedRequest;
use std::sync::Mutex;
use tracing::warn;
use web_time::{SystemTime, UNIX_EPOCH};

/// A token this close to its end, in seconds, is asked for again: it might
/// end on its way.
const MARGIN: i64 = 30;

/// The tokens had, by [`OAuthLogin::key`]: one per token endpoint, client,
/// user, scope and secret, so a changed one asks anew.
#[derive(Default)]
pub struct Tokens {
    /// Kept in the reader's store too, and read from there once.
    stored: bool,
    /// `None` until read.
    kept: Mutex<Option<KeptTokens>>,
    /// The writes of the whole set, one at a time in the order made: a
    /// later one landing first would be undone.
    writes: futures::lock::Mutex<()>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default()
}

impl Tokens {
    /// Kept in the store of the sender they sign for: the app's.
    pub fn stored() -> Self {
        Self {
            stored: true,
            ..Self::default()
        }
    }

    /// Read from the store, the first time.
    async fn load(&self, sender: &Sender) {
        if self.kept.lock().map(|kept| kept.is_some()).unwrap_or(true) {
            return;
        }
        let read = match self.stored {
            true => sender.storage.prefs().oauth_tokens().await.unwrap_or_else(|error| {
                warn!(%error, "the OAuth tokens could not be read");
                KeptTokens::default()
            }),
            false => KeptTokens::default(),
        };
        if let Ok(mut kept) = self.kept.lock()
            && kept.is_none()
        {
            *kept = Some(read);
        }
    }

    fn with<T>(&self, read: impl FnOnce(&mut KeptTokens) -> T) -> Option<T> {
        let mut kept = self.kept.lock().ok()?;
        Some(read(kept.get_or_insert_with(KeptTokens::default)))
    }

    /// The access token kept for `key`, while it has not ended.
    fn current(&self, key: &str) -> Option<String> {
        self.with(|kept| {
            let token = kept.0.get(key)?;
            token
                .expires_at
                .is_none_or(|ends| now() + MARGIN < ends)
                .then(|| token.access_token.clone())
        })
        .flatten()
    }

    fn refresh_token(&self, key: &str) -> Option<String> {
        self.with(|kept| kept.0.get(key)?.refresh_token.clone()).flatten()
    }

    /// `token`, asked at `asked` (seconds since the epoch), in place of the
    /// one `key` had; a refresh that gave no new refresh token leaves the
    /// old one.
    async fn keep(&self, sender: &Sender, key: &str, token: Token, asked: i64) {
        self.with(|kept| {
            let refresh_token = token
                .refresh_token
                .or_else(|| kept.0.get(key).and_then(|old| old.refresh_token.clone()));
            kept.0.insert(
                key.to_string(),
                KeptToken {
                    access_token: token.access_token,
                    refresh_token,
                    expires_at: token.expires_in.map(|seconds| asked + seconds as i64),
                },
            );
        });
        self.write(sender).await;
    }

    async fn forget(&self, sender: &Sender, key: &str) {
        if self.with(|kept| kept.0.remove(key)).flatten().is_some() {
            self.write(sender).await;
        }
    }

    /// The whole set into the store, but those ended with no refresh token
    /// to renew them.
    async fn write(&self, sender: &Sender) {
        if !self.stored {
            return;
        }
        let _turn = self.writes.lock().await;
        let Some(tokens) = self.with(|kept| {
            let at = now();
            kept.0
                .retain(|_, token| token.refresh_token.is_some() || token.expires_at.is_none_or(|ends| ends > at));
            kept.clone()
        }) else {
            return;
        };
        if let Err(error) = sender.storage.prefs().set_oauth_tokens(tokens).await {
            warn!(%error, "the OAuth tokens could not be kept");
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
    sender.tokens.load(sender).await;
    let kept = if anew {
        None
    } else {
        sender.tokens.current(&login.key())
    };
    let (token, signed) = match kept {
        Some(token) => (token, Signed::Kept),
        None => (ask(sender, &login, prepared).await?, Signed::Asked),
    };
    authorize(prepared, &token);
    Ok(signed)
}

/// F7.8: `prepared` signed with the token kept, if there is one; else
/// with words saying where one is had, for the code to be completed.
pub(crate) async fn sign_kept(sender: &Sender, prepared: &mut PreparedRequest) {
    if let Some(login) = &prepared.oauth {
        sender.tokens.load(sender).await;
        let token = sender
            .tokens
            .current(&login.key())
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
    let key = login.key();
    if let Some(refresh) = sender.tokens.refresh_token(&key) {
        let asked = now();
        if let Ok(token) = ask_with(sender, login, api, Some(&refresh)).await {
            let access = token.access_token.clone();
            sender.tokens.keep(sender, &key, token, asked).await;
            return Ok(access);
        }
    }
    sender.tokens.forget(sender, &key).await;
    let asked = now();
    let token = ask_with(sender, login, api, None)
        .await
        .map_err(|why| format!("OAuth 2.0: no token from {at}: {why}"))?;
    let access = token.access_token.clone();
    sender.tokens.keep(sender, &key, token, asked).await;
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
