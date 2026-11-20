//! LDAP sign-in for the bridge: the settings a system administrator makes
//! on the page, and the search filter a name goes into. The bridge does the
//! talking (`cyberapi-bridge/src/ldap.rs`); this is what can be checked
//! without a server.

use crate::directory::valid_account_name;
use serde::{Deserialize, Serialize};

/// Where the name goes in the user filter.
pub const NAME_PLACEHOLDER: &str = "{username}";
/// The user filter when none is given: OpenLDAP's people.
pub const DEFAULT_FILTER: &str = "(uid={username})";
/// The attribute a new account's display name comes from by default.
pub const DEFAULT_DISPLAY_ATTRIBUTE: &str = "cn";

/// The settings as the bridge keeps them, the bind password with them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LdapConfig {
    /// `ldap://host:389` or `ldaps://host:636`.
    pub url: String,
    /// Upgrade an `ldap://` connection with StartTLS before binding.
    #[serde(default)]
    pub start_tls: bool,
    /// The account that searches for people; empty for an anonymous search.
    #[serde(default)]
    pub bind_dn: String,
    #[serde(default)]
    pub bind_password: String,
    /// Where people are searched, `ou=people,dc=example,dc=com`.
    pub base_dn: String,
    /// Finds one person by name, [`NAME_PLACEHOLDER`] standing for it.
    pub user_filter: String,
    /// What a new account is called on screen; its name when empty.
    #[serde(default)]
    pub display_attribute: String,
}

impl LdapConfig {
    /// The filter that finds `name`, escaped so a name is only ever a value.
    pub fn filter_for(&self, name: &str) -> String {
        self.user_filter.replace(NAME_PLACEHOLDER, &escape_filter_value(name))
    }

    /// What the administrator's dialog shows: everything but the password.
    pub fn settings(&self) -> LdapSettings {
        LdapSettings {
            url: self.url.clone(),
            start_tls: self.start_tls,
            bind_dn: self.bind_dn.clone(),
            has_bind_password: !self.bind_password.is_empty(),
            base_dn: self.base_dn.clone(),
            user_filter: self.user_filter.clone(),
            display_attribute: self.display_attribute.clone(),
        }
    }
}

/// The settings without the bind password, for the page.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LdapSettings {
    pub url: String,
    pub start_tls: bool,
    pub bind_dn: String,
    /// Whether a bind password is kept; it never leaves the bridge.
    pub has_bind_password: bool,
    pub base_dn: String,
    pub user_filter: String,
    pub display_attribute: String,
}

/// The administrator's change. A blank bind password keeps the one kept
/// (with the bind DN unchanged), so the dialog never needs it back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LdapChange {
    pub url: String,
    #[serde(default)]
    pub start_tls: bool,
    #[serde(default)]
    pub bind_dn: String,
    #[serde(default)]
    pub bind_password: String,
    pub base_dn: String,
    #[serde(default)]
    pub user_filter: String,
    #[serde(default)]
    pub display_attribute: String,
}

/// Why a change was not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LdapProblem {
    /// Not `ldap://` or `ldaps://`, or no host.
    Url,
    /// StartTLS on an `ldaps://` connection, already encrypted.
    StartTlsOnLdaps,
    BaseDn,
    /// Not one `(…)` with [`NAME_PLACEHOLDER`] in it.
    Filter,
    /// A bind DN with no password to go with it.
    BindPassword,
}

impl LdapChange {
    /// The settings to keep, `kept` giving the bind password a blank one
    /// keeps; or what is wrong.
    pub fn applied(&self, kept: Option<&LdapConfig>) -> Result<LdapConfig, LdapProblem> {
        let url = self.url.trim();
        let host = url
            .strip_prefix("ldaps://")
            .or_else(|| url.strip_prefix("ldap://"))
            .ok_or(LdapProblem::Url)?;
        if host.trim_matches('/').is_empty() || host.contains(char::is_whitespace) {
            return Err(LdapProblem::Url);
        }
        if self.start_tls && url.starts_with("ldaps://") {
            return Err(LdapProblem::StartTlsOnLdaps);
        }
        let base_dn = self.base_dn.trim();
        if base_dn.is_empty() {
            return Err(LdapProblem::BaseDn);
        }
        let user_filter = match self.user_filter.trim() {
            "" => DEFAULT_FILTER,
            filter => filter,
        };
        if !(user_filter.starts_with('(') && user_filter.ends_with(')') && user_filter.contains(NAME_PLACEHOLDER)) {
            return Err(LdapProblem::Filter);
        }
        let bind_dn = self.bind_dn.trim();
        let bind_password = match (bind_dn.is_empty(), self.bind_password.as_str()) {
            // Anonymous: no password either.
            (true, _) => String::new(),
            (false, "") => kept
                .filter(|kept| kept.bind_dn == bind_dn && !kept.bind_password.is_empty())
                .map(|kept| kept.bind_password.clone())
                .ok_or(LdapProblem::BindPassword)?,
            (false, given) => given.to_string(),
        };
        Ok(LdapConfig {
            url: url.to_string(),
            start_tls: self.start_tls,
            bind_dn: bind_dn.to_string(),
            bind_password,
            base_dn: base_dn.to_string(),
            user_filter: user_filter.to_string(),
            display_attribute: self.display_attribute.trim().to_string(),
        })
    }
}

/// `value` as an LDAP filter value (RFC 4515): `*`, `(`, `)`, `\` and NUL
/// escaped, so a name cannot widen or rewrite the search.
pub fn escape_filter_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '*' => escaped.push_str("\\2a"),
            '(' => escaped.push_str("\\28"),
            ')' => escaped.push_str("\\29"),
            '\\' => escaped.push_str("\\5c"),
            '\0' => escaped.push_str("\\00"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Whether `name` and `password` may be tried against the directory at
/// all. An empty password is refused outright: to many servers a bind with
/// one is an anonymous bind, and succeeds.
pub fn may_try(name: &str, password: &str) -> bool {
    valid_account_name(name) && !password.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change() -> LdapChange {
        LdapChange {
            url: " ldap://ldap.example.com:389 ".into(),
            bind_dn: "cn=reader,dc=example,dc=com".into(),
            bind_password: "secret".into(),
            base_dn: "ou=people,dc=example,dc=com".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_name_is_only_ever_a_value_in_the_filter() {
        let config = change().applied(None).expect("valid");
        assert_eq!(config.filter_for("alice"), "(uid=alice)");
        assert_eq!(config.filter_for("*"), "(uid=\\2a)");
        assert_eq!(
            config.filter_for("x)(|(uid=*"),
            "(uid=x\\29\\28|\\28uid=\\2a)",
            "no second clause"
        );
        assert_eq!(escape_filter_value("a\\b\0"), "a\\5cb\\00");
    }

    #[test]
    fn a_change_is_checked_and_keeps_the_password_when_left_blank() {
        let config = change().applied(None).expect("valid");
        assert_eq!(config.url, "ldap://ldap.example.com:389");
        assert_eq!(config.user_filter, DEFAULT_FILTER);
        let blank = LdapChange {
            bind_password: String::new(),
            ..change()
        };
        assert_eq!(blank.applied(Some(&config)).expect("kept").bind_password, "secret");
        assert_eq!(blank.applied(None), Err(LdapProblem::BindPassword));
        let other_dn = LdapChange {
            bind_dn: "cn=other,dc=example,dc=com".into(),
            ..blank.clone()
        };
        assert_eq!(
            other_dn.applied(Some(&config)),
            Err(LdapProblem::BindPassword),
            "a password goes with its DN"
        );
        let anonymous = LdapChange {
            bind_dn: String::new(),
            ..change()
        };
        assert_eq!(anonymous.applied(None).expect("valid").bind_password, "");

        let bad = |edit: fn(&mut LdapChange)| {
            let mut change = change();
            edit(&mut change);
            change.applied(None).expect_err("refused")
        };
        assert_eq!(bad(|c| c.url = "http://ldap".into()), LdapProblem::Url);
        assert_eq!(bad(|c| c.url = "ldap://".into()), LdapProblem::Url);
        assert_eq!(
            bad(|c| {
                c.url = "ldaps://ldap".into();
                c.start_tls = true;
            }),
            LdapProblem::StartTlsOnLdaps
        );
        assert_eq!(bad(|c| c.base_dn = " ".into()), LdapProblem::BaseDn);
        assert_eq!(bad(|c| c.user_filter = "(uid=bob)".into()), LdapProblem::Filter);
        assert_eq!(bad(|c| c.user_filter = "uid={username}".into()), LdapProblem::Filter);
    }

    #[test]
    fn an_empty_password_is_never_tried() {
        assert!(may_try("alice", "pw"));
        assert!(!may_try("alice", ""));
        assert!(!may_try("", "pw"));
        assert!(!may_try("a b", "pw"));
    }

    #[test]
    fn the_page_never_sees_the_password() {
        let settings = change().applied(None).expect("valid").settings();
        assert!(settings.has_bind_password);
        assert!(!serde_json::to_string(&settings).expect("json").contains("secret"));
    }
}
