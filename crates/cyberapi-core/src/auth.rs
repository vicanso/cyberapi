//! How a request signs in (#66, `docs/batch4-design.md` §4): inherited,
//! none, Basic, a bearer token, or an API key in a header or the query.
//! Its values are templates, rendered as the request's others are.
//!
//! What the old app and the versions before #66 kept — a list whose first
//! enabled entry was Basic's user and password — reads as `Basic`; nothing
//! writes it any more.

use crate::digest::DigestLogin;
use crate::model::KvParam;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// How a request signs in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Auth {
    /// The folder's, or the project's above it. Until those can have one,
    /// none.
    #[default]
    Inherit,
    /// None, whatever is above.
    None,
    Basic {
        #[serde(default)]
        username: String,
        #[serde(default)]
        password: String,
    },
    Bearer {
        #[serde(default)]
        token: String,
    },
    /// C2: a user and a password, answering the server's challenge
    /// (`crate::digest`).
    Digest {
        #[serde(default)]
        username: String,
        #[serde(default)]
        password: String,
    },
    /// A key under a name of the API's choosing.
    ApiKey {
        #[serde(default)]
        name: String,
        #[serde(default)]
        value: String,
        #[serde(default)]
        place: KeyPlace,
    },
}

/// Where an API key goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyPlace {
    #[default]
    Header,
    Query,
}

/// What signing in adds to a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signed {
    Header { name: String, value: String },
    Query { name: String, value: String },
}

impl Auth {
    /// The kinds, in the order the editor offers them.
    pub const KINDS: [AuthKind; 6] = [
        AuthKind::Inherit,
        AuthKind::None,
        AuthKind::Basic,
        AuthKind::Bearer,
        AuthKind::ApiKey,
        AuthKind::Digest,
    ];

    pub fn is_inherit(&self) -> bool {
        *self == Self::Inherit
    }

    pub fn kind(&self) -> AuthKind {
        match self {
            Self::Inherit => AuthKind::Inherit,
            Self::None => AuthKind::None,
            Self::Basic { .. } => AuthKind::Basic,
            Self::Bearer { .. } => AuthKind::Bearer,
            Self::ApiKey { .. } => AuthKind::ApiKey,
            Self::Digest { .. } => AuthKind::Digest,
        }
    }

    /// Whether it signs in itself, rather than inheriting or not at all.
    pub fn signs(&self) -> bool {
        matches!(
            self,
            Self::Basic { .. } | Self::Bearer { .. } | Self::ApiKey { .. } | Self::Digest { .. }
        )
    }

    /// What the old list said: its first enabled entry, Basic's user and
    /// password; none, nothing of the request's own.
    pub fn from_legacy(list: &[KvParam]) -> Self {
        match list.iter().find(|kv| kv.enabled) {
            Some(kv) => Self::Basic {
                username: kv.key.clone(),
                password: kv.value.clone(),
            },
            None => Self::Inherit,
        }
    }

    /// The texts that are templates: Basic's and Digest's user and
    /// password, the token, the key's value.
    pub fn templates(&self) -> Vec<&str> {
        match self {
            Self::Basic { username, password } | Self::Digest { username, password } => vec![username, password],
            Self::Bearer { token } => vec![token],
            Self::ApiKey { value, .. } => vec![value],
            Self::Inherit | Self::None => Vec::new(),
        }
    }

    /// The name of the header it sends, if it sends one — Digest's goes
    /// once the server asks for it. A call with no query (gRPC: `query`
    /// false) sends its key as a header too.
    pub fn header_name(&self, query: bool) -> Option<&str> {
        match self {
            Self::Basic { .. } | Self::Bearer { .. } | Self::Digest { .. } => Some("Authorization"),
            Self::ApiKey { name, place, .. } if *place == KeyPlace::Header || !query => {
                Some(name.trim()).filter(|name| !name.is_empty())
            }
            _ => None,
        }
    }

    /// The entry of the request's own that goes in its place, the request's
    /// winning (#3): an enabled header of the name it sends, or a query
    /// parameter of its key's. `Some((true, name))` for a header.
    pub fn shadowed(&self, headers: &[KvParam], query_list: &[KvParam], query: bool) -> Option<(bool, String)> {
        let on =
            |list: &[KvParam], same: &dyn Fn(&str) -> bool| list.iter().any(|kv| kv.enabled && same(kv.key.trim()));
        if let Some(name) = self.header_name(query) {
            return on(headers, &|key| key.eq_ignore_ascii_case(name)).then(|| (true, name.to_string()));
        }
        match self {
            Self::ApiKey { name, .. } if query && !name.trim().is_empty() => {
                let name = name.trim();
                on(query_list, &|key| key == name).then(|| (false, name.to_string()))
            }
            _ => None,
        }
    }

    /// What it adds to a request, its templates rendered by `render`: a
    /// header, a query parameter, or nothing (inheriting, none, a key with
    /// no name, Digest until the server asks). A call with no query takes
    /// its key as a header.
    pub fn sign<E>(&self, query: bool, render: impl Fn(&str) -> Result<String, E>) -> Result<Option<Signed>, E> {
        Ok(match self {
            Self::Inherit | Self::None | Self::Digest { .. } => None,
            Self::Basic { username, password } => {
                let pair = format!("{}:{}", render(username)?, render(password)?);
                Some(Signed::Header {
                    name: "Authorization".into(),
                    value: format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(pair)),
                })
            }
            Self::Bearer { token } => Some(Signed::Header {
                name: "Authorization".into(),
                value: format!("Bearer {}", render(token)?),
            }),
            Self::ApiKey { name, value, place } => {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return Ok(None);
                }
                let value = render(value)?;
                if *place == KeyPlace::Query && query {
                    Some(Signed::Query { name, value })
                } else {
                    Some(Signed::Header { name, value })
                }
            }
        })
    }
}

impl Auth {
    /// C2: Digest's user and password, rendered by `render`.
    pub fn digest_login<E>(&self, render: impl Fn(&str) -> Result<String, E>) -> Result<Option<DigestLogin>, E> {
        Ok(match self {
            Self::Digest { username, password } => Some(DigestLogin {
                username: render(username)?,
                password: render(password)?,
            }),
            _ => None,
        })
    }
}

/// An [`Auth`]'s kind, without its values: what the editor's list picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AuthKind {
    Inherit,
    None,
    Basic,
    Bearer,
    ApiKey,
    Digest,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::convert::Infallible;

    fn kv(key: &str, value: &str, enabled: bool) -> KvParam {
        KvParam {
            key: key.into(),
            value: value.into(),
            enabled,
        }
    }

    fn signed(auth: &Auth, query: bool) -> Option<Signed> {
        auth.sign(query, |text| Ok::<_, Infallible>(text.replace("{{T}}", "rendered")))
            .expect("rendered")
    }

    #[test]
    fn the_old_list_reads_as_basic() {
        assert_eq!(
            Auth::from_legacy(&[kv("off", "x", false), kv("me", "secret", true), kv("b", "c", true)]),
            Auth::Basic {
                username: "me".into(),
                password: "secret".into()
            }
        );
        assert_eq!(Auth::from_legacy(&[kv("off", "x", false)]), Auth::Inherit);
        assert_eq!(Auth::from_legacy(&[]), Auth::Inherit);
    }

    #[test]
    fn each_kind_signs_as_it_says() {
        let basic = Auth::Basic {
            username: "me".into(),
            password: "{{T}}".into(),
        };
        assert_eq!(
            signed(&basic, true),
            Some(Signed::Header {
                name: "Authorization".into(),
                value: format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode("me:rendered")
                ),
            })
        );
        let bearer = Auth::Bearer { token: "{{T}}".into() };
        assert_eq!(
            signed(&bearer, true),
            Some(Signed::Header {
                name: "Authorization".into(),
                value: "Bearer rendered".into()
            })
        );
        let key = |place| Auth::ApiKey {
            name: " X-Api-Key ".into(),
            value: "{{T}}".into(),
            place,
        };
        assert_eq!(
            signed(&key(KeyPlace::Query), true),
            Some(Signed::Query {
                name: "X-Api-Key".into(),
                value: "rendered".into()
            })
        );
        assert_eq!(
            signed(&key(KeyPlace::Query), false),
            Some(Signed::Header {
                name: "X-Api-Key".into(),
                value: "rendered".into()
            }),
            "a call with no query takes it as a header"
        );
        assert_eq!(key(KeyPlace::Query).header_name(true), None);
        assert_eq!(key(KeyPlace::Query).header_name(false), Some("X-Api-Key"));
        assert_eq!(bearer.header_name(true), Some("Authorization"));
        let nameless = Auth::ApiKey {
            name: " ".into(),
            value: "v".into(),
            place: KeyPlace::Header,
        };
        assert_eq!(signed(&nameless, true), None);
        assert_eq!(nameless.header_name(true), None);
        assert_eq!(signed(&Auth::Inherit, true), None);
        assert_eq!(signed(&Auth::None, true), None);
    }

    #[test]
    fn the_requests_own_entry_of_the_same_name_is_told() {
        let bearer = Auth::Bearer { token: "t".into() };
        let headers = [kv("authorization", "Bearer mine", true), kv("X-Key", "k", false)];
        assert_eq!(
            bearer.shadowed(&headers, &[], true),
            Some((true, "Authorization".into()))
        );
        assert_eq!(bearer.shadowed(&headers[1..], &[], true), None, "not one turned off");
        let key = Auth::ApiKey {
            name: "X-Key".into(),
            value: "v".into(),
            place: KeyPlace::Query,
        };
        let query = [kv("X-Key", "mine", true)];
        assert_eq!(key.shadowed(&[], &query, true), Some((false, "X-Key".into())));
        assert_eq!(
            key.shadowed(&[], &[kv("x-key", "v", true)], true),
            None,
            "a query is exact"
        );
        // A gRPC call: the key is metadata, and only headers stand in.
        assert_eq!(key.shadowed(&[], &query, false), None);
        assert_eq!(
            key.shadowed(&[kv("x-key", "m", true)], &query, false),
            Some((true, "X-Key".into()))
        );
        assert_eq!(Auth::Inherit.shadowed(&headers, &query, true), None);
    }

    #[test]
    fn it_is_kept_by_its_kind() {
        let auth = Auth::ApiKey {
            name: "key".into(),
            value: "v".into(),
            place: KeyPlace::Query,
        };
        let json = serde_json::to_string(&auth).expect("JSON");
        assert_eq!(json, r#"{"kind":"apiKey","name":"key","value":"v","place":"query"}"#);
        assert_eq!(serde_json::from_str::<Auth>(&json).expect("read"), auth);
        assert_eq!(
            serde_json::from_str::<Auth>(r#"{"kind":"bearer"}"#).expect("read"),
            Auth::Bearer { token: String::new() }
        );
    }
}
