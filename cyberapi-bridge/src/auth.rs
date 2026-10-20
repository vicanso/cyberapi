//! Passwords, tokens and the session cookie.

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use axum::http::{HeaderMap, StatusCode, header};
use cyberapi_store_sql::{Account, SqlStore};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

pub const SESSION_COOKIE: &str = "cyberapi_session";

/// Checked against when the name does not exist, so a sign-in for a missing
/// name costs what a wrong password does: a real hash, with the hasher's own
/// parameters, of a random password nobody kept.
pub static DECOY_HASH: LazyLock<String> = LazyLock::new(|| {
    new_token()
        .and_then(|password| hash_password(&password))
        .unwrap_or_default()
});

/// The password hasher. Tests use the cheapest parameters there are: a hash
/// records its own, so what they produce verifies the same way.
fn hasher() -> Argon2<'static> {
    #[cfg(not(test))]
    {
        Argon2::default()
    }
    #[cfg(test)]
    {
        Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(8, 1, 1, None).unwrap_or_default(),
        )
    }
}

/// Argon2id, PHC form. Slow on purpose: call it off the async workers.
pub fn hash_password(password: &str) -> Result<String, String> {
    hasher()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|e| e.to_string())
}

/// Whether `password` is the one `hash` was made from. Slow on purpose.
pub fn verify_password(hash: &str, password: &str) -> bool {
    hasher().verify_password(password.as_bytes(), hash).is_ok()
}

/// The store's directory hashes and checks passwords this way too.
pub struct Argon2Passwords;

impl cyberapi_store_sql::Passwords for Argon2Passwords {
    fn hash(&self, password: &str) -> Result<String, String> {
        hash_password(password)
    }

    fn verify(&self, hash: &str, password: &str) -> bool {
        verify_password(hash, password)
    }
}

/// A session token: 32 random bytes, hex.
pub fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(hex::encode(bytes))
}

/// What the store keeps of a token: a copy of the database is no login.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Compares two secrets in time that does not depend on where they differ.
pub fn same_secret(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    given.len() == expected.len() && given.iter().zip(expected).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// The session cookie: unreadable to scripts, sent only by this origin's
/// own pages. An empty token with no age clears it.
pub fn session_cookie(token: &str, max_age: i64, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}")
}

/// A cookie's value from the request.
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_string())
        .filter(|value| !value.is_empty())
}

/// The account whose session the request carries; 401 without one.
pub async fn signed_in(store: &SqlStore, headers: &HeaderMap) -> Result<Account, StatusCode> {
    let token = cookie(headers, SESSION_COOKIE).ok_or(StatusCode::UNAUTHORIZED)?;
    match store.session_account(&hash_token(&token), unix_now()).await {
        Ok(Some(account)) => Ok(account),
        Ok(None) => Err(StatusCode::UNAUTHORIZED),
        Err(error) => {
            tracing::error!(%error, "a session could not be checked");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn passwords_verify_only_themselves() {
        let hash = hash_password("correct horse").expect("hash");
        assert!(verify_password(&hash, "correct horse"));
        assert!(!verify_password(&hash, "wrong horse"));
        assert!(DECOY_HASH.starts_with("$argon2id$"), "the decoy is a real hash");
        assert!(!verify_password(&DECOY_HASH, ""));
    }

    #[test]
    fn cookies_are_read_by_name() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; cyberapi_session=abc; b=2"),
        );
        assert_eq!(cookie(&headers, SESSION_COOKIE), Some("abc".into()));
        assert_eq!(cookie(&headers, "missing"), None);
    }

    #[test]
    fn tokens_are_random_and_hashed() {
        let (a, b) = (new_token().expect("token"), new_token().expect("token"));
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert_eq!(hash_token(&a).len(), 64);
        assert!(same_secret("abc", "abc") && !same_secret("abc", "abd") && !same_secret("abc", "ab"));
    }
}
