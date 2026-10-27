//! E3: a GraphQL API whose address is `ws://` or `wss://` speaks
//! [graphql-transport-ws](https://github.com/enisdenjo/graphql-ws/blob/master/PROTOCOL.md).
//! An `http(s)` one stays a request; a subscription there is an event
//! stream. What a subscription yields is kept once it ends, one JSON line
//! an event, read back as an event stream's events are.

use crate::response::StreamInfo;
use crate::sse::Event;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The subprotocol, named in `Sec-WebSocket-Protocol`.
pub const PROTOCOL: &str = "graphql-transport-ws";

/// The one operation a connection runs.
pub const ID: &str = "1";

/// A result the server yielded.
pub const NEXT: &str = "next";
/// A terminal error for the operation.
pub const ERROR: &str = "error";

/// Whether `url` is a WebSocket address. The scheme only: `ws` or `wss`.
pub fn is_socket_url(url: &str) -> bool {
    matches!(scheme(url).as_deref(), Some("ws") | Some("wss"))
}

/// Whether the address was written as a WebSocket one, before a variable
/// in it is filled in. The button follows this; a `{{HOST}}` that turns
/// out to be one is decided when it is sent.
pub fn written_as_socket(uri: &str) -> bool {
    is_socket_url(uri.trim())
}

/// Whether sending it might open a subscription: written as one, or
/// starting with a template whose value may be.
pub fn might_subscribe(uri: &str) -> bool {
    let uri = uri.trim();
    written_as_socket(uri) || uri.starts_with('{')
}

fn scheme(url: &str) -> Option<String> {
    let (scheme, _) = url.trim().split_once("://")?;
    Some(scheme.to_ascii_lowercase())
}

/// What a server message means.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Ack,
    /// Answer it with [`pong`], echoing `payload` when it sent one.
    Ping(Option<Value>),
    Pong,
    /// A result. The text is the payload, indented when it is JSON.
    Next(String),
    /// Terminal. The text is the payload.
    Error(String),
    Complete,
    /// The server rejected the initialization.
    ConnectionError(String),
    /// Not a message this protocol speaks. Shown as it is.
    Other(String),
}

/// `text` as a server message. One that is no JSON object is [`Message::Other`].
pub fn read_message(text: &str) -> Message {
    let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(text) else {
        return Message::Other(text.to_string());
    };
    let kind = fields.get("type").and_then(Value::as_str).unwrap_or("");
    let payload = fields.get("payload");
    match kind {
        "connection_ack" => Message::Ack,
        "ping" => Message::Ping(payload.cloned()),
        "pong" => Message::Pong,
        "next" => Message::Next(shown(payload)),
        "error" => Message::Error(shown(payload)),
        "complete" => Message::Complete,
        "connection_error" => Message::ConnectionError(shown(payload)),
        _ => Message::Other(text.to_string()),
    }
}

/// A payload as a reader sees it: JSON indented, anything else its text.
fn shown(payload: Option<&Value>) -> String {
    match payload {
        Some(Value::String(text)) => text.clone(),
        Some(value) => crate::format::json(value),
        None => String::new(),
    }
}

/// `connection_init`. `authorization` goes in the payload when the request
/// has one, which is where a server of this protocol takes it.
pub fn connection_init(authorization: Option<&str>) -> String {
    match authorization {
        Some(value) => serde_json::json!({
            "type": "connection_init",
            "payload": { "Authorization": value },
        })
        .to_string(),
        None => "{\"type\":\"connection_init\"}".into(),
    }
}

/// The answer to a ping.
pub fn pong(payload: Option<&Value>) -> String {
    match payload {
        Some(payload) => serde_json::json!({"type": "pong", "payload": payload}).to_string(),
        None => "{\"type\":\"pong\"}".into(),
    }
}

/// `subscribe` for the operation, `payload` the query as a POST would send it.
pub fn subscribe(payload: &Value) -> String {
    serde_json::json!({"id": ID, "type": "subscribe", "payload": payload}).to_string()
}

/// The reader ends the operation.
pub fn complete() -> String {
    serde_json::json!({"id": ID, "type": "complete"}).to_string()
}

/// One event as kept.
#[derive(Serialize, Deserialize)]
struct Line<'a> {
    event: &'a str,
    data: &'a str,
}

/// `event` as a line of the body that is kept.
pub fn line(event: &str, data: &str) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&Line { event, data }).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

/// The events of a kept subscription. A line that does not read is shown
/// as it is.
pub fn events(body: &[u8], stream: Option<&StreamInfo>) -> Vec<Event> {
    let arrivals = stream.map(|stream| stream.arrivals.as_slice()).unwrap_or_default();
    let mut end = 0u64;
    body.split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            end += line.len() as u64 + 1;
            let at = arrivals.iter().find(|(arrived, _)| *arrived >= end).map(|(_, at)| *at);
            let (event, data) = match serde_json::from_slice::<OwnedLine>(line) {
                Ok(kept) => (kept.event, kept.data),
                Err(_) => ("message".into(), String::from_utf8_lossy(line).into_owned()),
            };
            Event {
                event,
                data,
                at,
                ..Default::default()
            }
        })
        .collect()
}

#[derive(Deserialize)]
struct OwnedLine {
    event: String,
    data: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn an_address_is_a_socket_by_its_scheme() {
        assert!(is_socket_url("wss://a.test/graphql"));
        assert!(written_as_socket("  WS://a.test "));
        assert!(!is_socket_url("https://a.test"));
        assert!(!written_as_socket("{{HOST}}/graphql"));
        assert!(might_subscribe("{{HOST}}/graphql"));
        assert!(might_subscribe("wss://a.test"));
        assert!(!might_subscribe("https://a.test/graphql"));
    }

    #[test]
    fn the_client_speaks_and_a_server_message_is_read() {
        assert_eq!(connection_init(None), "{\"type\":\"connection_init\"}");
        assert!(connection_init(Some("Bearer t")).contains("\"Authorization\":\"Bearer t\""));
        let payload = serde_json::json!({"query": "subscription { ping }"});
        assert!(subscribe(&payload).contains("\"type\":\"subscribe\""));
        assert_eq!(complete(), format!("{{\"id\":\"{ID}\",\"type\":\"complete\"}}"));
        assert_eq!(pong(None), "{\"type\":\"pong\"}");

        assert_eq!(read_message("{\"type\":\"connection_ack\"}"), Message::Ack);
        assert_eq!(
            read_message("{\"type\":\"ping\",\"payload\":{\"n\":1}}"),
            Message::Ping(Some(serde_json::json!({"n": 1})))
        );
        assert_eq!(read_message("{\"type\":\"pong\"}"), Message::Pong);
        assert_eq!(
            read_message("{\"id\":\"1\",\"type\":\"next\",\"payload\":{\"data\":{\"ping\":\"ok\"}}}"),
            Message::Next("{\n    \"data\": {\n        \"ping\": \"ok\"\n    }\n}".into())
        );
        assert!(matches!(
            read_message("{\"type\":\"error\",\"payload\":[{\"message\":\"no\"}]}"),
            Message::Error(_)
        ));
        assert_eq!(read_message("{\"id\":\"1\",\"type\":\"complete\"}"), Message::Complete);
        assert!(matches!(
            read_message("{\"type\":\"connection_error\",\"payload\":\"nope\"}"),
            Message::ConnectionError(_)
        ));
        assert!(matches!(read_message("hello"), Message::Other(_)));
    }

    #[test]
    fn what_a_subscription_yielded_is_kept_and_read_back() {
        let mut body = line(NEXT, "{\n    \"data\": {}\n}");
        body.extend(line(ERROR, "nope"));
        let stream = StreamInfo {
            arrivals: vec![(body.iter().position(|byte| *byte == b'\n').unwrap_or(0) as u64 + 1, 15)],
            ..Default::default()
        };
        let events = events(&body, Some(&stream));
        assert_eq!(events[0].event, NEXT);
        assert_eq!(events[0].at, Some(15));
        assert_eq!(events[1].event, ERROR);
        assert_eq!(events[1].data, "nope");
        assert_eq!(events[1].at, None);
    }
}
