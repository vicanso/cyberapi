//! CyberAPI's use cases (L2 in `docs/architecture.md` §3): what the states
//! ask for, written once over the store and engine traits so the desktop and
//! the browser run the same code.

pub mod backup;
mod send;

pub use send::{SendFailure, Sender, curl, introspect, reflect, send, send_watched};
