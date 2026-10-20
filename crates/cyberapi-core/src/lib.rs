//! CyberAPI's domain (L0 in `docs/architecture.md`): the data model and the
//! pure rules over it. No I/O, no clock, no gpui — everything here is plain
//! data in and plain data out, so it is the same on the desktop, in the
//! browser and on the bridge, and it is tested without any of them.

pub mod access;
pub mod collection;
pub mod format;
mod id;
pub mod model;
pub mod prefs;
pub mod search;
mod time;
pub mod tree;

pub use id::{AccountId, CollectionId, ItemId, TeamId, VariableId};
pub use time::Timestamp;
