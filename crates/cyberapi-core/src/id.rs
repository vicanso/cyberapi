//! Identifiers. All are ULID strings in practice, but nothing here relies on
//! that: an id read from an old database or an import is taken as it is.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! string_id {
    ($(#[$attr:meta])* $name:ident) => {
        $(#[$attr])*
        #[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                Self(id.to_string())
            }
        }

        impl From<String> for $name {
            fn from(id: String) -> Self {
                Self(id)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id!(
    /// A project (`api_collections`).
    CollectionId
);
string_id!(
    /// A node of a project's tree: a folder or an API. The two share one id
    /// space, because a folder's `children` lists both kinds side by side.
    ItemId
);
string_id!(
    /// An environment variable, a custom variable or a global request header.
    VariableId
);
string_id!(
    /// A team of the browser build's accounts (`docs/architecture.md` §9).
    /// Every project there belongs to one; the desktop has none.
    TeamId
);
string_id!(
    /// An account of the browser build. The desktop has none.
    AccountId
);
string_id!(
    /// An invitation to make an account (`docs/server-sync.md` §5).
    InviteId
);
string_id!(
    /// An MCP token: how an AI assistant signs in as an account at the
    /// bridge's `/v1/mcp` (`docs/DEPLOY.md`, AI assistants).
    McpTokenId
);
