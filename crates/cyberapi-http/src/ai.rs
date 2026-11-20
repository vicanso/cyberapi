//! H3: the seam to the team's model (`docs/ai-assistant.md` §4). The page
//! (and a desktop signed in to a server) reaches it through the bridge, as
//! `cyberapi_store_bridge::BridgeAi`; tests script one.

use crate::Stop;
use cyberapi_core::ai::{AiEvent, Chat, ModelChange, ModelSettings};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// What an [`AiModel`] call answers in the end.
pub type AiFuture<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'static>>;

/// Told each event as the model's answer comes.
pub type AiTold = Arc<dyn Fn(AiEvent) + Send + Sync>;

pub trait AiModel: Send + Sync {
    /// The model's name, or `None` when there is no model to ask.
    fn model(&self) -> AiFuture<Option<String>>;
    /// Asks `chat`, telling `told` each event; the future ends with the
    /// turn, or when `stop` fires (sent, or its sender dropped). The page
    /// cannot count on a dropped future to end its fetch, so it is told.
    fn chat(&self, chat: Chat, told: AiTold, stop: Stop) -> AiFuture<()>;
    /// Administrators: the model in use, without its key; `None` when
    /// there is none.
    fn settings(&self) -> AiFuture<Option<ModelSettings>> {
        Box::pin(async { Err("the model cannot be set here".to_string()) })
    }
    /// Administrators: sets the model (a blank key keeps the one set), or
    /// with `None` puts the server's own back; answers the model now in use.
    fn save_settings(&self, change: Option<ModelChange>) -> AiFuture<Option<ModelSettings>> {
        let _ = change;
        Box::pin(async { Err("the model cannot be set here".to_string()) })
    }
}
