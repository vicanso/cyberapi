//! The page's side of the LDAP settings (`cyberapi-bridge/src/ldap.rs`):
//! read, saved and tried by a system administrator.

use crate::{Response, Transport};
use cyberapi_core::ldap::{LdapChange, LdapProblem, LdapSettings};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub const LDAP_SETTINGS_PATH: &str = "/v1/ldap/settings";
pub const LDAP_SAVE_PATH: &str = "/v1/ldap/settings/save";
pub const LDAP_TEST_PATH: &str = "/v1/ldap/test";

/// Why the bridge did not do it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LdapRefusal {
    /// The settings are not right.
    Problem(LdapProblem),
    /// Not answered as asked: why, as it said.
    Failed(String),
}

pub type LdapFuture<T> = Pin<Box<dyn Future<Output = Result<T, LdapRefusal>> + Send + 'static>>;

#[derive(Clone)]
pub struct BridgeLdap {
    transport: Arc<dyn Transport>,
}

fn refused(response: &Response) -> LdapRefusal {
    if response.status == 422
        && let Ok(said) = serde_json::from_slice::<serde_json::Value>(&response.body)
        && let Ok(problem) = serde_json::from_value::<LdapProblem>(said["problem"].clone())
    {
        return LdapRefusal::Problem(problem);
    }
    let said = String::from_utf8_lossy(&response.body).trim().to_string();
    LdapRefusal::Failed(if said.is_empty() {
        format!("the bridge answered {}", response.status)
    } else {
        said
    })
}

fn settings(response: Response) -> Result<Option<LdapSettings>, LdapRefusal> {
    match response.status {
        200 => serde_json::from_slice(&response.body).map_err(|error| LdapRefusal::Failed(error.to_string())),
        _ => Err(refused(&response)),
    }
}

impl BridgeLdap {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { transport }
    }

    /// The directory sign-ins fall back to, without its bind password.
    pub fn settings(&self) -> LdapFuture<Option<LdapSettings>> {
        let posted = self.transport.post(LDAP_SETTINGS_PATH, Vec::new());
        Box::pin(async move { settings(posted.await.map_err(LdapRefusal::Failed)?) })
    }

    /// Sets it, or with `None` turns LDAP sign-in off.
    pub fn save(&self, change: Option<LdapChange>) -> LdapFuture<Option<LdapSettings>> {
        let posted = self
            .transport
            .post(LDAP_SAVE_PATH, serde_json::to_vec(&change).unwrap_or_default());
        Box::pin(async move { settings(posted.await.map_err(LdapRefusal::Failed)?) })
    }

    /// Whether `change` reaches and reads the directory.
    pub fn test(&self, change: LdapChange) -> LdapFuture<()> {
        let posted = self
            .transport
            .post(LDAP_TEST_PATH, serde_json::to_vec(&change).unwrap_or_default());
        Box::pin(async move {
            let response = posted.await.map_err(LdapRefusal::Failed)?;
            if response.status != 200 {
                return Err(refused(&response));
            }
            let said: serde_json::Value =
                serde_json::from_slice(&response.body).map_err(|error| LdapRefusal::Failed(error.to_string()))?;
            match said["ok"].as_bool() {
                Some(true) => Ok(()),
                _ => Err(LdapRefusal::Failed(
                    said["error"].as_str().unwrap_or_default().to_string(),
                )),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_problem_reads_as_one() {
        let response = Response {
            status: 422,
            body: br#"{"problem":"baseDn"}"#.to_vec(),
        };
        assert_eq!(refused(&response), LdapRefusal::Problem(LdapProblem::BaseDn));
        let response = Response {
            status: 403,
            body: Vec::new(),
        };
        assert_eq!(
            refused(&response),
            LdapRefusal::Failed("the bridge answered 403".into())
        );
    }
}
