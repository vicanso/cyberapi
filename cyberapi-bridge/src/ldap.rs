//! LDAP sign-in. A system administrator sets the directory on the page
//! (`/v1/ldap/settings/save`, kept in the store's `bridge_settings`); from
//! then on a sign-in whose local password does not hold is tried against
//! it: search for the name with the reader account, bind as what was found
//! with the password given. Someone the directory knows and this bridge
//! does not gets an account, not an administrator. The local password is
//! always tried first and an administrator never goes to the directory, so
//! a directory down or set wrong never locks an administrator out.

use crate::Bridge;
use crate::auth::signed_in;
use axum::extract::{Json, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use cyberapi_core::ldap::{LdapChange, LdapConfig, may_try};
use cyberapi_store_sql::{Account, NewAccount};
use ldap3::{LdapConnAsync, LdapConnSettings, Scope, SearchEntry};
use serde_json::json;
use std::sync::PoisonError;
use std::time::Duration;

/// The name the settings are kept under in `bridge_settings`.
const SETTING: &str = "ldap";
/// How long connecting may take.
const CONNECT: Duration = Duration::from_secs(5);
/// How long a whole sign-in or test may take.
const WHOLE: Duration = Duration::from_secs(10);
/// The LDAP result code of a wrong password.
const INVALID_CREDENTIALS: u32 = 49;

/// Someone the directory knows, as found.
pub struct Found {
    pub dn: String,
    pub display_name: Option<String>,
}

/// The settings kept in the store, if an administrator set them.
pub async fn kept(bridge: &Bridge) -> Result<Option<LdapConfig>, String> {
    let Some(text) = bridge
        .inner
        .store
        .bridge_setting(SETTING)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| format!("the kept LDAP settings did not read: {error}"))
}

/// Whether an administrator turned LDAP sign-in on.
pub fn is_on(bridge: &Bridge) -> bool {
    bridge
        .inner
        .ldap
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some()
}

fn current(bridge: &Bridge) -> Option<LdapConfig> {
    bridge.inner.ldap.read().unwrap_or_else(PoisonError::into_inner).clone()
}

async fn connect(config: &LdapConfig) -> Result<ldap3::Ldap, String> {
    let settings = LdapConnSettings::new()
        .set_conn_timeout(CONNECT)
        .set_starttls(config.start_tls);
    let (conn, mut ldap) = LdapConnAsync::with_settings(settings, &config.url)
        .await
        .map_err(|error| format!("could not connect: {error}"))?;
    ldap3::drive!(conn);
    if !config.bind_dn.is_empty() {
        ldap.simple_bind(&config.bind_dn, &config.bind_password)
            .await
            .and_then(ldap3::LdapResult::success)
            .map_err(|error| format!("the reader account could not bind: {error}"))?;
    }
    Ok(ldap)
}

/// Who `name` is in the directory when `password` is theirs: `None` when
/// the directory does not know them or the password is wrong; an error
/// when the directory could not be asked.
pub async fn authenticate(config: &LdapConfig, name: &str, password: &str) -> Result<Option<Found>, String> {
    if !may_try(name, password) {
        return Ok(None);
    }
    let asked = async {
        let mut ldap = connect(config).await?;
        let attributes = match config.display_attribute.as_str() {
            "" => vec!["1.1".to_string()],
            attribute => vec![attribute.to_string()],
        };
        let (entries, _) = ldap
            .search(&config.base_dn, Scope::Subtree, &config.filter_for(name), attributes)
            .await
            .and_then(ldap3::SearchResult::success)
            .map_err(|error| format!("the search failed: {error}"))?;
        // Nobody, or more than one: no sign-in.
        let mut entries = entries.into_iter();
        let (Some(entry), None) = (entries.next(), entries.next()) else {
            let _ = ldap.unbind().await;
            return Ok(None);
        };
        let entry = SearchEntry::construct(entry);
        let bound = ldap
            .simple_bind(&entry.dn, password)
            .await
            .map_err(|error| format!("the bind failed: {error}"))?;
        let _ = ldap.unbind().await;
        match bound.rc {
            0 => Ok(Some(Found {
                display_name: entry
                    .attrs
                    .get(&config.display_attribute)
                    .and_then(|values| values.first())
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                dn: entry.dn,
            })),
            INVALID_CREDENTIALS => Ok(None),
            rc => Err(format!("the bind answered {rc}: {}", bound.text)),
        }
    };
    tokio::time::timeout(WHOLE, asked)
        .await
        .unwrap_or_else(|_| Err("the directory took too long".into()))
}

/// Whether the directory can be reached and searched with `config`.
pub async fn check(config: &LdapConfig) -> Result<(), String> {
    let asked = async {
        let mut ldap = connect(config).await?;
        ldap.search(&config.base_dn, Scope::Base, "(objectClass=*)", vec!["1.1"])
            .await
            .and_then(ldap3::SearchResult::success)
            .map_err(|error| format!("the base DN could not be read: {error}"))?;
        let _ = ldap.unbind().await;
        Ok(())
    };
    tokio::time::timeout(WHOLE, asked)
        .await
        .unwrap_or_else(|_| Err("the directory took too long".into()))
}

/// A sign-in the local password refused, tried against the directory:
/// `local` is the enabled account of that name, if there is one. The
/// account to sign in as, made when the directory knows them and this
/// bridge does not.
pub async fn sign_in(bridge: &Bridge, local: Option<Account>, name: &str, password: &str) -> Option<Account> {
    // An administrator signs in with their own password only.
    if local.as_ref().is_some_and(|account| account.admin) {
        return None;
    }
    let config = current(bridge)?;
    let found = match authenticate(&config, name, password).await {
        Ok(found) => found?,
        Err(error) => {
            tracing::warn!(%error, name, "the LDAP directory could not be asked");
            return None;
        }
    };
    if let Some(account) = local {
        // An account made before LDAP accounts were marked, or invited and
        // now signing in through the directory: an LDAP one from now on.
        if let Err(error) = bridge.inner.store.mark_ldap_account(&account.id).await {
            tracing::warn!(%error, account = %account.id, "the account could not be marked an LDAP one");
        }
        tracing::info!(account = %account.id, dn = %found.dn, "ldap sign-in");
        return Some(account);
    }
    // Someone new: an account whose own password nobody knows.
    let unusable = crate::auth::new_token().ok()?;
    let hash = tokio::task::spawn_blocking(move || crate::auth::hash_password(&unusable))
        .await
        .ok()?
        .ok()?;
    let made = bridge
        .inner
        .store
        .create_ldap_account(NewAccount {
            name: name.to_string(),
            display_name: found.display_name.unwrap_or_default(),
            password_hash: hash,
            admin: false,
        })
        .await;
    if let Err(error) = made {
        // Taken by an account that is disabled: it stays so.
        tracing::info!(%error, name, "no account was made for an LDAP sign-in");
        return None;
    }
    let account = bridge.inner.store.find_login(name).await.ok().flatten()?.0;
    tracing::info!(account = %account.id, dn = %found.dn, "ldap sign-in made an account");
    Some(account)
}

/// Whether LDAP sign-in is on (`{"enabled": bool}`), for the sign-in page
/// to say so. Nothing else about the directory.
pub async fn enabled(State(bridge): State<Bridge>) -> Response {
    Json(json!({ "enabled": current(&bridge).is_some() })).into_response()
}

/// A system administrator's session, or the answer to anyone else.
async fn administrator(bridge: &Bridge, headers: &HeaderMap) -> Result<Account, StatusCode> {
    let account = signed_in(&bridge.inner.store, headers).await?;
    if !account.admin {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(account)
}

/// The settings without the bind password, or `null`: administrators only.
pub async fn settings(State(bridge): State<Bridge>, headers: HeaderMap) -> Response {
    if let Err(status) = administrator(&bridge, &headers).await {
        return status.into_response();
    }
    Json(current(&bridge).map(|config| config.settings())).into_response()
}

fn problem(problem: cyberapi_core::ldap::LdapProblem) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "problem": problem }))).into_response()
}

/// Sets the directory, or with `null` turns LDAP sign-in off; answers the
/// settings now in use as [`settings`] does.
pub async fn save(
    State(bridge): State<Bridge>,
    headers: HeaderMap,
    Json(change): Json<Option<LdapChange>>,
) -> Response {
    let account = match administrator(&bridge, &headers).await {
        Ok(account) => account,
        Err(status) => return status.into_response(),
    };
    let now = match change {
        Some(change) => match change.applied(current(&bridge).as_ref()) {
            Ok(config) => Some(config),
            Err(refused) => return problem(refused),
        },
        None => None,
    };
    let text = now
        .as_ref()
        .map(|config| serde_json::to_string(config).unwrap_or_default());
    if let Err(error) = bridge.inner.store.set_bridge_setting(SETTING, text.as_deref()).await {
        tracing::error!(%error, "the LDAP settings could not be kept");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    tracing::info!(
        account = %account.id,
        url = now.as_ref().map(|config| config.url.as_str()).unwrap_or(""),
        base_dn = now.as_ref().map(|config| config.base_dn.as_str()).unwrap_or(""),
        "the LDAP settings were set"
    );
    let settings = now.as_ref().map(LdapConfig::settings);
    *bridge.inner.ldap.write().unwrap_or_else(PoisonError::into_inner) = now;
    Json(settings).into_response()
}

/// Tries the settings in the dialog before they are saved (a blank bind
/// password is the kept one): `{"ok": true}` or `{"ok": false, "error"}`.
pub async fn test(State(bridge): State<Bridge>, headers: HeaderMap, Json(change): Json<LdapChange>) -> Response {
    if let Err(status) = administrator(&bridge, &headers).await {
        return status.into_response();
    }
    let config = match change.applied(current(&bridge).as_ref()) {
        Ok(config) => config,
        Err(refused) => return problem(refused),
    };
    match check(&config).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(error) => Json(json!({ "ok": false, "error": error })).into_response(),
    }
}
