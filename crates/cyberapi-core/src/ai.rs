//! H3: the assistant's conversation, whatever model answers it. The bridge
//! holds the team's model (`docs/ai-assistant.md` §4): it turns a [`Chat`]
//! into the request its protocol takes ([`ModelConfig::request`]) and reads
//! the model's event stream back as [`AiEvent`]s ([`Reader`]), which it
//! passes on to the page one per line. OpenAI's Chat Completions (and the
//! gateways that speak it) and Anthropic's Messages are understood.

use crate::sse;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The most a model writes in one turn unless told otherwise.
pub const MAX_TOKENS: u32 = 4096;
/// The Messages API revision asked for.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// How a model is spoken to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// Chat Completions, `POST {url}/chat/completions`.
    #[default]
    OpenAi,
    /// Messages, `POST {url}/messages`.
    Anthropic,
}

impl Protocol {
    /// `openai` or `anthropic`, any case.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "openai" => Some(Self::OpenAi),
            "anthropic" => Some(Self::Anthropic),
            _ => None,
        }
    }
}

/// A turn's speaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// A piece of a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Part {
    Text {
        text: String,
    },
    /// The model asked for a tool (an assistant's part).
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    /// What the tool answered (a user's part).
    ToolResult {
        id: String,
        content: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl Message {
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            parts: vec![Part::Text { text: text.into() }],
        }
    }
}

/// A tool the model may ask for; `parameters` is a JSON Schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// What the page asks the model: the instructions, the conversation so
/// far, and the tools it offers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chat {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
}

/// Why a turn ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stop {
    /// It said all it had to.
    #[default]
    Done,
    /// It waits for the tools it asked for.
    Tools,
    /// It ran out of tokens.
    Length,
}

/// What the bridge tells the page as the model answers, one per line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AiEvent {
    /// More of the answer's text.
    Text { text: String },
    /// A tool the model asks for, its arguments whole.
    ToolCall { id: String, name: String, arguments: Value },
    /// The turn ended; the tokens the model said it used, when it said.
    Done {
        stop: Stop,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u64>,
    },
    /// The turn failed: why, as the model or the bridge put it.
    Error { message: String },
}

/// The team's model, as the bridge's administrator configured it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelConfig {
    pub protocol: Protocol,
    /// The API's base, `https://api.openai.com/v1` say; the path is added.
    pub url: String,
    pub model: String,
    pub key: String,
    pub max_tokens: u32,
}

/// The model an administrator set on the page, as the bridge keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptModel {
    pub protocol: Protocol,
    pub url: String,
    pub model: String,
    pub key: String,
}

impl KeptModel {
    pub fn config(&self) -> ModelConfig {
        ModelConfig {
            protocol: self.protocol,
            url: self.url.clone(),
            model: self.model.clone(),
            key: self.key.clone(),
            max_tokens: MAX_TOKENS,
        }
    }
}

/// What the administrator's dialog shows: everything but the key.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSettings {
    pub protocol: Protocol,
    pub url: String,
    pub model: String,
    /// Whether a key is set; the key itself never leaves the bridge.
    pub has_key: bool,
    /// Set on the page (`true`) rather than by the bridge's options.
    pub on_page: bool,
}

/// The administrator's change: `None` puts the bridge's options back.
/// A blank key keeps the one set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelChange {
    pub protocol: Protocol,
    pub url: String,
    pub model: String,
    #[serde(default)]
    pub key: String,
}

impl ModelChange {
    /// The model to keep, `kept` giving the key a blank one keeps; or why not.
    pub fn applied(&self, kept: Option<&str>) -> Result<KeptModel, String> {
        let url = self.url.trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("the address starts with http:// or https://".into());
        }
        let model = self.model.trim();
        if model.is_empty() {
            return Err("the model's name is empty".into());
        }
        let key = match self.key.trim() {
            "" => kept.ok_or("the key is empty")?.to_string(),
            key => key.to_string(),
        };
        Ok(KeptModel {
            protocol: self.protocol,
            url: url.to_string(),
            model: model.to_string(),
            key,
        })
    }
}

/// A request ready to send: where, its headers and its JSON body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl ModelConfig {
    /// `chat` as this model's protocol takes it, streamed.
    pub fn request(&self, chat: &Chat) -> ModelRequest {
        let base = self.url.trim().trim_end_matches('/');
        let json_type = ("Content-Type".to_string(), "application/json".to_string());
        let accept = ("Accept".to_string(), "text/event-stream".to_string());
        match self.protocol {
            Protocol::OpenAi => ModelRequest {
                url: format!("{base}/chat/completions"),
                headers: vec![
                    json_type,
                    accept,
                    ("Authorization".into(), format!("Bearer {}", self.key)),
                ],
                body: openai_body(self, chat).to_string().into_bytes(),
            },
            Protocol::Anthropic => ModelRequest {
                url: format!("{base}/messages"),
                headers: vec![
                    json_type,
                    accept,
                    ("x-api-key".into(), self.key.clone()),
                    ("anthropic-version".into(), ANTHROPIC_VERSION.into()),
                ],
                body: anthropic_body(self, chat).to_string().into_bytes(),
            },
        }
    }
}

fn openai_body(config: &ModelConfig, chat: &Chat) -> Value {
    let mut messages = Vec::new();
    if !chat.system.is_empty() {
        messages.push(json!({ "role": "system", "content": chat.system }));
    }
    for message in &chat.messages {
        let text: String = message
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        match message.role {
            Role::User => {
                // A tool's answer is a message of its own.
                for part in &message.parts {
                    if let Part::ToolResult { id, content } = part {
                        messages.push(json!({ "role": "tool", "tool_call_id": id, "content": content }));
                    }
                }
                if !text.is_empty() {
                    messages.push(json!({ "role": "user", "content": text }));
                }
            }
            Role::Assistant => {
                let calls: Vec<Value> = message
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        Part::ToolCall { id, name, arguments } => Some(json!({
                            "id": id,
                            "type": "function",
                            "function": { "name": name, "arguments": arguments.to_string() },
                        })),
                        _ => None,
                    })
                    .collect();
                let mut said =
                    json!({ "role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text) } });
                if !calls.is_empty() {
                    said["tool_calls"] = json!(calls);
                }
                messages.push(said);
            }
        }
    }
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "max_tokens": config.max_tokens,
        "stream": true,
    });
    if !chat.tools.is_empty() {
        body["tools"] = json!(
            chat.tools
                .iter()
                .map(|tool| json!({
                    "type": "function",
                    "function": { "name": tool.name, "description": tool.description, "parameters": tool.parameters },
                }))
                .collect::<Vec<_>>()
        );
    }
    body
}

fn anthropic_body(config: &ModelConfig, chat: &Chat) -> Value {
    let messages: Vec<Value> = chat
        .messages
        .iter()
        .map(|message| {
            let content: Vec<Value> = message
                .parts
                .iter()
                .map(|part| match part {
                    Part::Text { text } => json!({ "type": "text", "text": text }),
                    Part::ToolCall { id, name, arguments } => {
                        json!({ "type": "tool_use", "id": id, "name": name, "input": arguments })
                    }
                    Part::ToolResult { id, content } => {
                        json!({ "type": "tool_result", "tool_use_id": id, "content": content })
                    }
                })
                .collect();
            json!({
                "role": match message.role { Role::User => "user", Role::Assistant => "assistant" },
                "content": content,
            })
        })
        .collect();
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "max_tokens": config.max_tokens,
        "stream": true,
    });
    if !chat.system.is_empty() {
        body["system"] = json!(chat.system);
    }
    if !chat.tools.is_empty() {
        body["tools"] = json!(
            chat.tools
                .iter()
                .map(|tool| json!({ "name": tool.name, "description": tool.description, "input_schema": tool.parameters }))
                .collect::<Vec<_>>()
        );
    }
    body
}

/// A tool call whose arguments are still coming.
#[derive(Default)]
struct Partial {
    id: String,
    name: String,
    arguments: String,
}

impl Partial {
    fn event(self) -> AiEvent {
        let arguments = match self.arguments.trim() {
            "" => json!({}),
            text => serde_json::from_str(text).unwrap_or_else(|_| json!({ "_unread": text })),
        };
        AiEvent::ToolCall {
            id: self.id,
            name: self.name,
            arguments,
        }
    }
}

/// Reads a model's event stream as [`AiEvent`]s, piece by piece.
pub struct Reader {
    protocol: Protocol,
    sse: sse::Parser,
    /// By the place the model gave each call.
    calls: BTreeMap<u64, Partial>,
    stop: Option<Stop>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    ended: bool,
}

impl Reader {
    pub fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            sse: sse::Parser::new(),
            calls: BTreeMap::new(),
            stop: None,
            input_tokens: None,
            output_tokens: None,
            ended: false,
        }
    }

    /// The events `piece` of the stream completes.
    pub fn feed(&mut self, piece: &[u8]) -> Vec<AiEvent> {
        let events = self.sse.feed(piece, None);
        let mut out = Vec::new();
        for event in events {
            if self.ended {
                break;
            }
            match self.protocol {
                Protocol::OpenAi => self.openai(&event.data, &mut out),
                Protocol::Anthropic => self.anthropic(event.kind(), &event.data, &mut out),
            }
        }
        out
    }

    /// The stream ended: what is left, and its end. A stream cut short
    /// with nothing said of its end is an error.
    pub fn finish(mut self) -> Vec<AiEvent> {
        if self.ended {
            return Vec::new();
        }
        let mut out: Vec<AiEvent> = std::mem::take(&mut self.calls)
            .into_values()
            .map(Partial::event)
            .collect();
        match self.stop {
            Some(stop) => out.push(AiEvent::Done {
                stop,
                input_tokens: self.input_tokens,
                output_tokens: self.output_tokens,
            }),
            None => out.push(AiEvent::Error {
                message: "the model's answer ended before it said it was done".into(),
            }),
        }
        out
    }

    fn end(&mut self, out: &mut Vec<AiEvent>) {
        out.extend(std::mem::take(&mut self.calls).into_values().map(Partial::event));
        out.push(AiEvent::Done {
            stop: self.stop.unwrap_or_default(),
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        });
        self.ended = true;
    }

    fn fail(&mut self, message: String, out: &mut Vec<AiEvent>) {
        out.push(AiEvent::Error { message });
        self.ended = true;
    }

    fn openai(&mut self, data: &str, out: &mut Vec<AiEvent>) {
        if data.trim() == "[DONE]" {
            return self.end(out);
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            return;
        };
        if let Some(error) = chunk.get("error") {
            return self.fail(error_message(error), out);
        }
        if let Some(usage) = chunk.get("usage").filter(|usage| usage.is_object()) {
            self.input_tokens = usage["prompt_tokens"].as_u64().or(self.input_tokens);
            self.output_tokens = usage["completion_tokens"].as_u64().or(self.output_tokens);
        }
        let Some(choice) = chunk["choices"].get(0) else {
            return;
        };
        let delta = &choice["delta"];
        if let Some(text) = delta["content"].as_str().filter(|text| !text.is_empty()) {
            out.push(AiEvent::Text { text: text.into() });
        }
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let at = call["index"].as_u64().unwrap_or_default();
            let partial = self.calls.entry(at).or_default();
            if let Some(id) = call["id"].as_str() {
                partial.id = id.into();
            }
            if let Some(name) = call["function"]["name"].as_str() {
                partial.name.push_str(name);
            }
            if let Some(arguments) = call["function"]["arguments"].as_str() {
                partial.arguments.push_str(arguments);
            }
        }
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.stop = Some(match reason {
                "tool_calls" | "function_call" => Stop::Tools,
                "length" => Stop::Length,
                _ => Stop::Done,
            });
        }
    }

    fn anthropic(&mut self, kind: &str, data: &str, out: &mut Vec<AiEvent>) {
        let Ok(event) = serde_json::from_str::<Value>(data) else {
            return;
        };
        let at = event["index"].as_u64().unwrap_or_default();
        match kind {
            "message_start" => {
                self.input_tokens = event["message"]["usage"]["input_tokens"].as_u64();
            }
            "content_block_start" => {
                let block = &event["content_block"];
                if block["type"] == "tool_use" {
                    self.calls.insert(
                        at,
                        Partial {
                            id: block["id"].as_str().unwrap_or_default().into(),
                            name: block["name"].as_str().unwrap_or_default().into(),
                            arguments: String::new(),
                        },
                    );
                }
            }
            "content_block_delta" => {
                let delta = &event["delta"];
                match delta["type"].as_str() {
                    Some("text_delta") => {
                        if let Some(text) = delta["text"].as_str().filter(|text| !text.is_empty()) {
                            out.push(AiEvent::Text { text: text.into() });
                        }
                    }
                    Some("input_json_delta") => {
                        if let (Some(partial), Some(json)) = (self.calls.get_mut(&at), delta["partial_json"].as_str()) {
                            partial.arguments.push_str(json);
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(partial) = self.calls.remove(&at) {
                    out.push(partial.event());
                }
            }
            "message_delta" => {
                if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                    self.stop = Some(match reason {
                        "tool_use" => Stop::Tools,
                        "max_tokens" => Stop::Length,
                        _ => Stop::Done,
                    });
                }
                self.output_tokens = event["usage"]["output_tokens"].as_u64().or(self.output_tokens);
            }
            "message_stop" => self.end(out),
            "error" => self.fail(error_message(&event["error"]), out),
            _ => {}
        }
    }
}

/// What a model's error says, from either protocol's `error` object.
fn error_message(error: &Value) -> String {
    error["message"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

/// Why a model refused a request, from its status and the body it sent:
/// the `error.message` either protocol writes, else the body's start.
pub fn refusal(status: u16, body: &[u8]) -> String {
    let said = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("error").map(error_message))
        .unwrap_or_else(|| crate::mcp::cut(String::from_utf8_lossy(body).trim(), 300));
    format!("the model answered {status}: {said}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_keeps_the_key_when_left_blank_and_checks_the_rest() {
        let change = ModelChange {
            protocol: Protocol::OpenAi,
            url: " https://ai.example/v1 ".into(),
            model: "gpt".into(),
            key: String::new(),
        };
        assert_eq!(change.applied(Some("sk-old")).expect("valid").key, "sk-old");
        assert_eq!(
            change.applied(Some("sk-old")).expect("valid").url,
            "https://ai.example/v1"
        );
        assert!(change.applied(None).is_err());
        let new_key = ModelChange {
            key: "sk-new".into(),
            ..change.clone()
        };
        assert_eq!(new_key.applied(Some("sk-old")).expect("valid").key, "sk-new");
        assert!(
            ModelChange {
                url: "ai.example".into(),
                ..new_key.clone()
            }
            .applied(None)
            .is_err()
        );
        assert!(
            ModelChange {
                model: " ".into(),
                ..new_key
            }
            .applied(None)
            .is_err()
        );
    }
    use pretty_assertions::assert_eq;

    fn config(protocol: Protocol) -> ModelConfig {
        ModelConfig {
            protocol,
            url: "https://models.test/v1/".into(),
            model: "m-1".into(),
            key: "k".into(),
            max_tokens: MAX_TOKENS,
        }
    }

    fn chat() -> Chat {
        Chat {
            system: "Be brief.".into(),
            messages: vec![
                Message::text(Role::User, "List the APIs."),
                Message {
                    role: Role::Assistant,
                    parts: vec![Part::ToolCall {
                        id: "c1".into(),
                        name: "list_apis".into(),
                        arguments: json!({ "project": "shop" }),
                    }],
                },
                Message {
                    role: Role::User,
                    parts: vec![Part::ToolResult {
                        id: "c1".into(),
                        content: "[]".into(),
                    }],
                },
            ],
            tools: vec![Tool {
                name: "list_apis".into(),
                description: "Lists them.".into(),
                parameters: json!({ "type": "object" }),
            }],
        }
    }

    fn body(request: &ModelRequest) -> Value {
        serde_json::from_slice(&request.body).expect("JSON")
    }

    #[test]
    fn an_openai_request() {
        let request = config(Protocol::OpenAi).request(&chat());
        assert_eq!(request.url, "https://models.test/v1/chat/completions");
        assert!(request.headers.contains(&("Authorization".into(), "Bearer k".into())));
        let body = body(&request);
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0], json!({ "role": "system", "content": "Be brief." }));
        assert_eq!(
            body["messages"][2]["tool_calls"][0]["function"]["arguments"],
            r#"{"project":"shop"}"#
        );
        assert_eq!(body["messages"][2]["content"], Value::Null);
        assert_eq!(
            body["messages"][3],
            json!({ "role": "tool", "tool_call_id": "c1", "content": "[]" })
        );
        assert_eq!(body["tools"][0]["function"]["name"], "list_apis");
    }

    #[test]
    fn an_anthropic_request() {
        let request = config(Protocol::Anthropic).request(&chat());
        assert_eq!(request.url, "https://models.test/v1/messages");
        assert!(request.headers.contains(&("x-api-key".into(), "k".into())));
        assert!(
            request
                .headers
                .contains(&("anthropic-version".into(), ANTHROPIC_VERSION.into()))
        );
        let body = body(&request);
        assert_eq!(body["system"], "Be brief.");
        assert_eq!(body["max_tokens"], MAX_TOKENS);
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][1]["content"][0]["input"], json!({ "project": "shop" }));
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "c1");
        assert_eq!(body["tools"][0]["input_schema"], json!({ "type": "object" }));
    }

    /// Feeds `stream` in small pieces, as a network would.
    fn read(protocol: Protocol, stream: &str) -> Vec<AiEvent> {
        let mut reader = Reader::new(protocol);
        let mut events = Vec::new();
        for piece in stream.as_bytes().chunks(7) {
            events.extend(reader.feed(piece));
        }
        events.extend(reader.finish());
        events
    }

    #[test]
    fn an_openai_stream_with_text_and_tool_calls() {
        let stream = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Let me \"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"look.\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"type\":\"function\",\"function\":{\"name\":\"get_api\",\"arguments\":\"\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"api\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"c2\",\"function\":{\"name\":\"list_apis\",\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"login\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":5}}\n\n",
            "data: [DONE]\n\n",
        );
        assert_eq!(
            read(Protocol::OpenAi, stream),
            vec![
                AiEvent::Text { text: "Let me ".into() },
                AiEvent::Text { text: "look.".into() },
                AiEvent::ToolCall {
                    id: "c1".into(),
                    name: "get_api".into(),
                    arguments: json!({ "api": "login" })
                },
                AiEvent::ToolCall {
                    id: "c2".into(),
                    name: "list_apis".into(),
                    arguments: json!({})
                },
                AiEvent::Done {
                    stop: Stop::Tools,
                    input_tokens: Some(12),
                    output_tokens: Some(5)
                },
            ]
        );
    }

    #[test]
    fn an_anthropic_stream_with_text_and_a_tool_call() {
        let stream = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":30}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: ping\ndata: {\"type\":\"ping\"}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"好的\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"get_api\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"api\\\": \\\"lo\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"gin\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":9}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        assert_eq!(
            read(Protocol::Anthropic, stream),
            vec![
                AiEvent::Text { text: "好的".into() },
                AiEvent::ToolCall {
                    id: "t1".into(),
                    name: "get_api".into(),
                    arguments: json!({ "api": "login" })
                },
                AiEvent::Done {
                    stop: Stop::Tools,
                    input_tokens: Some(30),
                    output_tokens: Some(9)
                },
            ]
        );
    }

    #[test]
    fn errors_and_streams_cut_short() {
        let anthropic = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
        assert_eq!(
            read(Protocol::Anthropic, anthropic),
            vec![AiEvent::Error {
                message: "Overloaded".into()
            }]
        );
        let cut = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hal\"}}]}\n\n";
        assert!(matches!(
            read(Protocol::OpenAi, cut).last(),
            Some(AiEvent::Error { .. })
        ));
        let stopped =
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"x\"},\"finish_reason\":\"length\"}]}\n\n";
        assert!(
            matches!(
                read(Protocol::OpenAi, stopped).last(),
                Some(AiEvent::Done { stop: Stop::Length, .. })
            ),
            "a gateway that sends no [DONE] still ends"
        );
        assert_eq!(
            refusal(401, br#"{"error":{"message":"Incorrect API key"}}"#),
            "the model answered 401: Incorrect API key"
        );
        assert_eq!(refusal(502, b"bad gateway"), "the model answered 502: bad gateway");
    }

    #[test]
    fn events_go_one_per_line() {
        let event = AiEvent::Done {
            stop: Stop::Done,
            input_tokens: None,
            output_tokens: Some(3),
        };
        let line = serde_json::to_string(&event).expect("JSON");
        assert_eq!(line, r#"{"kind":"done","stop":"done","output_tokens":3}"#);
        assert_eq!(serde_json::from_str::<AiEvent>(&line).expect("reads"), event);
    }
}
