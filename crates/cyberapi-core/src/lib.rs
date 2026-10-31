//! CyberAPI's domain (L0 in `docs/architecture.md`): the data model and the
//! pure rules over it. No I/O, no clock, no gpui — everything here is plain
//! data in and plain data out, so it is the same on the desktop, in the
//! browser and on the bridge, and it is tested without any of them.

pub mod access;
pub mod assertions;
pub mod auth;
pub mod backup;
pub mod certificates;
pub mod code;
pub mod collection;
pub mod cookie;
pub mod curl;
pub mod diff;
pub mod digest;
pub mod directory;
pub mod environments;
pub mod export;
pub mod find_replace;
pub mod format;
pub mod graphql;
pub mod graphql_format;
pub mod graphql_schema;
pub mod graphql_ws;
pub mod grpc;
pub mod html_docs;
mod id;
pub mod import;
pub mod inherit;
pub mod json_filter;
pub mod json_view;
pub mod load;
pub mod local_only;
pub mod mcp;
pub mod media;
pub mod mock;
pub mod model;
pub mod oauth;
pub mod prefs;
pub mod proxy;
pub mod redirect;
pub mod response;
pub mod run;
pub mod schema_docs;
pub mod search;
pub mod secrets;
pub mod send;
pub mod sse;
pub mod sync;
pub mod template;
pub mod template_hints;
mod time;
pub mod transfer;
pub mod tree;
pub mod undo;
pub mod update;
pub mod version;
pub mod versions;
pub mod ws;

pub use id::{AccountId, CollectionId, EnvironmentId, InviteId, ItemId, McpTokenId, TeamId, VariableId};
pub use time::Timestamp;
