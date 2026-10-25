//! CyberAPI's domain (L0 in `docs/architecture.md`): the data model and the
//! pure rules over it. No I/O, no clock, no gpui — everything here is plain
//! data in and plain data out, so it is the same on the desktop, in the
//! browser and on the bridge, and it is tested without any of them.

pub mod access;
pub mod assertions;
pub mod auth;
pub mod backup;
pub mod code;
pub mod collection;
pub mod cookie;
pub mod curl;
pub mod directory;
pub mod environments;
pub mod format;
pub mod graphql;
pub mod graphql_format;
pub mod graphql_schema;
pub mod grpc;
mod id;
pub mod import;
pub mod inherit;
pub mod json_filter;
pub mod json_view;
pub mod local_only;
pub mod mcp;
pub mod media;
pub mod model;
pub mod prefs;
pub mod proxy;
pub mod redirect;
pub mod response;
pub mod search;
pub mod send;
pub mod sse;
pub mod sync;
pub mod template;
pub mod template_hints;
mod time;
pub mod tree;
pub mod update;
pub mod version;

pub use id::{AccountId, CollectionId, EnvironmentId, InviteId, ItemId, McpTokenId, TeamId, VariableId};
pub use time::Timestamp;
