//! H3: the team's model, through the bridge (`docs/ai-assistant.md` §4):
//! `POST /v1/ai` names it, `POST /v1/ai/chat` asks it and answers one
//! `AiEvent` a line as it comes.

use crate::Transport;
use cyberapi_core::ai::{AiEvent, Chat, ModelChange, ModelSettings};
use cyberapi_http::Stop;
use cyberapi_http::ai::{AiFuture, AiModel, AiTold};
use futures::StreamExt as _;
use std::sync::Arc;

pub const AI_PATH: &str = "/v1/ai";
pub const AI_CHAT_PATH: &str = "/v1/ai/chat";
pub const AI_SETTINGS_PATH: &str = "/v1/ai/settings";
pub const AI_SAVE_PATH: &str = "/v1/ai/settings/save";

/// The bridge's model, if it has one.
#[derive(Clone)]
pub struct BridgeAi {
    transport: Arc<dyn Transport>,
}

impl BridgeAi {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { transport }
    }
}

/// The complete lines of `buffer`, read as events; what follows the last
/// line break stays in it.
fn lines(buffer: &mut Vec<u8>) -> Vec<AiEvent> {
    let Some(end) = buffer.iter().rposition(|byte| *byte == b'\n') else {
        return Vec::new();
    };
    let complete: Vec<u8> = buffer.drain(..=end).collect();
    complete
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
        .map(|line| {
            serde_json::from_slice(line).unwrap_or_else(|error| AiEvent::Error {
                message: format!("the bridge's answer did not read: {error}"),
            })
        })
        .collect()
}

/// The bridge's answer about the model's settings: on 200 the settings,
/// else its reason, as it gave it.
fn settings_answer(response: crate::Response) -> Result<Option<ModelSettings>, String> {
    match response.status {
        200 => serde_json::from_slice(&response.body).map_err(|error| error.to_string()),
        403 => Err("only an administrator sets the model".into()),
        status => {
            let reason = String::from_utf8_lossy(&response.body).trim().to_string();
            Err(if reason.is_empty() {
                format!("the bridge answered {status}")
            } else {
                reason
            })
        }
    }
}

impl AiModel for BridgeAi {
    fn settings(&self) -> AiFuture<Option<ModelSettings>> {
        let posted = self.transport.post(AI_SETTINGS_PATH, Vec::new());
        Box::pin(async move { settings_answer(posted.await?) })
    }

    fn save_settings(&self, change: Option<ModelChange>) -> AiFuture<Option<ModelSettings>> {
        let body = serde_json::to_vec(&change).unwrap_or_default();
        let posted = self.transport.post(AI_SAVE_PATH, body);
        Box::pin(async move { settings_answer(posted.await?) })
    }

    fn model(&self) -> AiFuture<Option<String>> {
        let posted = self.transport.post(AI_PATH, Vec::new());
        Box::pin(async move {
            let response = posted.await?;
            match response.status {
                200 => {
                    let said: serde_json::Value =
                        serde_json::from_slice(&response.body).map_err(|error| error.to_string())?;
                    Ok(said["model"].as_str().map(str::to_string))
                }
                // A bridge from before H3 has no such route.
                404 | 405 => Ok(None),
                status => Err(format!("the bridge answered {status}")),
            }
        })
    }

    fn chat(&self, chat: Chat, told: AiTold, mut stop: Stop) -> AiFuture<()> {
        let body = serde_json::to_vec(&chat).unwrap_or_default();
        let posted = self.transport.post_stream(AI_CHAT_PATH, body);
        Box::pin(async move {
            let mut streamed = posted.await?;
            if streamed.status != 200 {
                return Err(format!("the bridge answered {}", streamed.status));
            }
            let mut buffer = Vec::new();
            loop {
                // Stopped: the pieces go with this, and with them the
                // fetch; the bridge sees the page leave and ends the call.
                let piece = match futures::future::select(streamed.pieces.next(), &mut stop).await {
                    futures::future::Either::Left((Some(piece), _)) => piece,
                    futures::future::Either::Left((None, _)) => break,
                    futures::future::Either::Right(_) => return Ok(()),
                };
                buffer.extend_from_slice(&piece?);
                for event in lines(&mut buffer) {
                    told(event);
                }
            }
            buffer.push(b'\n');
            for event in lines(&mut buffer) {
                told(event);
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_wait_for_their_end() {
        let mut buffer = br#"{"kind":"text","text":"a"}"#.to_vec();
        buffer.extend_from_slice(b"\n{\"kind\":\"te");
        assert_eq!(lines(&mut buffer), vec![AiEvent::Text { text: "a".into() }]);
        assert_eq!(buffer, b"{\"kind\":\"te");
        buffer.extend_from_slice(b"xt\",\"text\":\"b\"}\n");
        assert_eq!(lines(&mut buffer), vec![AiEvent::Text { text: "b".into() }]);
        assert!(buffer.is_empty());
    }
}
