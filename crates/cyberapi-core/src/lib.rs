//! CyberAPI's domain (L0 in `docs/architecture.md`): the data model and the
//! pure rules over it. No I/O, no clock, no gpui — everything here is plain
//! data in and plain data out, so it is the same on the desktop, in the
//! browser and on the bridge, and it is tested without any of them.

pub mod access;
pub mod backup;
pub mod collection;
pub mod cookie;
pub mod curl;
pub mod directory;
pub mod format;
mod id;
pub mod import;
pub mod json_view;
pub mod model;
pub mod prefs;
pub mod response;
pub mod search;
pub mod send;
pub mod sync;
pub mod template;
mod time;
pub mod tree;

pub use id::{AccountId, CollectionId, InviteId, ItemId, TeamId, VariableId};
pub use time::Timestamp;
