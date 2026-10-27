//! E1: WebSocket connections. An API whose method is
//! [`HttpMethod::WebSocket`](crate::model::HttpMethod) connects to its
//! address ([`address`]) with its query, headers and sign-in as any request
//! sends them; its body is the message the reader sends next, a template.
//! What went each way is kept as one entry of the history once the
//! connection ends: a JSON line a message ([`transcript`]), with when each
//! went or came, read back as events ([`events`]) as an event stream's are.

use crate::response::StreamInfo;
use crate::send::PreparedRequest;
use crate::sse::Event;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// The event type of a message the reader sent.
pub const SENT: &str = "sent";
/// Of one the server sent: text, or bytes shown in base64.
pub const RECEIVED: &str = "received";
pub const RECEIVED_BINARY: &str = "binary";

/// A message, either way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
}

/// The address connected to: `ws://` or `wss://` as written, `http(s)://`
/// as the WebSocket of the same, none as `ws://`.
pub fn address(uri: &str) -> String {
    let uri = uri.trim();
    match uri.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => format!("ws://{rest}"),
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => format!("wss://{rest}"),
        Some(_) => uri.to_string(),
        None => format!("ws://{uri}"),
    }
}

/// The address the connection is made to, as HTTP: `ws` as `http`, `wss`
/// as `https`. `None` for any other.
pub fn http_url(address: &str) -> Option<String> {
    let (scheme, rest) = address.split_once("://")?;
    match scheme.to_ascii_lowercase().as_str() {
        "ws" | "http" => Some(format!("http://{rest}")),
        "wss" | "https" => Some(format!("https://{rest}")),
        _ => None,
    }
}

/// One message as kept: which way, and its text — a binary one's bytes in
/// base64.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Line {
    sent: bool,
    text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    binary: bool,
}

/// The messages kept, `(sent, frame, when)` with when in milliseconds
/// after the connection opened: the body, a JSON line each, and when each
/// line ended as a stream's arrivals.
pub fn transcript(messages: &[(bool, Frame, u64)]) -> (Vec<u8>, Vec<(u64, u64)>) {
    let mut body = Vec::new();
    let mut arrivals = Vec::new();
    for (sent, frame, at) in messages {
        let line = match frame {
            Frame::Text(text) => Line {
                sent: *sent,
                text: text.clone(),
                binary: false,
            },
            Frame::Binary(bytes) => Line {
                sent: *sent,
                text: base64::engine::general_purpose::STANDARD.encode(bytes),
                binary: true,
            },
        };
        // A struct of strings and flags cannot fail to serialize.
        body.extend(serde_json::to_vec(&line).unwrap_or_default());
        body.push(b'\n');
        arrivals.push((body.len() as u64, *at));
    }
    (body, arrivals)
}

/// A message as the event list shows it, `at` milliseconds after the
/// connection opened.
pub fn event(sent: bool, frame: &Frame, at: u64) -> Event {
    let (event, data) = match (sent, frame) {
        (true, Frame::Text(text)) => (SENT, text.clone()),
        (true, Frame::Binary(bytes)) => (SENT, base64::engine::general_purpose::STANDARD.encode(bytes)),
        (false, Frame::Text(text)) => (RECEIVED, text.clone()),
        (false, Frame::Binary(bytes)) => (RECEIVED_BINARY, base64::engine::general_purpose::STANDARD.encode(bytes)),
    };
    Event {
        event: event.to_string(),
        data,
        at: Some(at),
        ..Default::default()
    }
}

/// The messages of a kept connection, as events. A line that does not read
/// is shown as it is.
pub fn events(body: &[u8], stream: Option<&StreamInfo>) -> Vec<Event> {
    let arrivals = stream.map(|stream| stream.arrivals.as_slice()).unwrap_or_default();
    let mut end = 0u64;
    body.split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            end += line.len() as u64 + 1;
            let at = arrivals.iter().find(|(arrived, _)| *arrived >= end).map(|(_, at)| *at);
            let (event, data) = match serde_json::from_slice::<Line>(line) {
                Ok(Line { sent: true, text, .. }) => (SENT, text),
                Ok(Line {
                    sent: false,
                    text,
                    binary,
                }) => (if binary { RECEIVED_BINARY } else { RECEIVED }, text),
                Err(_) => (RECEIVED, String::from_utf8_lossy(line).into_owned()),
            };
            Event {
                event: event.to_string(),
                data,
                at,
                ..Default::default()
            }
        })
        .collect()
}

/// F7.8: the connection as a websocat command — no cURL makes one.
pub fn websocat(prepared: &PreparedRequest) -> String {
    let quote = |text: &str| format!("'{}'", text.replace('\'', r"'\''"));
    let mut words = vec!["websocat".to_string()];
    if prepared.accept_invalid_certs {
        words.push("-k".into());
    }
    for (name, value) in &prepared.headers {
        words.push(format!("-H {}", quote(&format!("{name}: {value}"))));
    }
    words.push(quote(&prepared.url));
    words.join(" \\\n  ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn the_address_is_a_websockets() {
        assert_eq!(address("ws://a.test/chat"), "ws://a.test/chat");
        assert_eq!(address(" wss://a.test "), "wss://a.test");
        assert_eq!(address("http://a.test/x?y=1"), "ws://a.test/x?y=1");
        assert_eq!(address("HTTPS://a.test"), "wss://a.test");
        assert_eq!(address("localhost:8080/ws"), "ws://localhost:8080/ws");
        assert_eq!(http_url("wss://a.test/x").as_deref(), Some("https://a.test/x"));
        assert_eq!(http_url("ws://a.test").as_deref(), Some("http://a.test"));
        assert_eq!(http_url("ftp://a.test"), None);
    }

    #[test]
    fn what_went_each_way_is_kept_and_read_back() {
        let messages = [
            (true, Frame::Text("hello\nthere".into()), 5),
            (false, Frame::Text(r#"{"ok":true}"#.into()), 9),
            (false, Frame::Binary(vec![0, 1, 2]), 12),
        ];
        let (body, arrivals) = transcript(&messages);
        let text = String::from_utf8(body.clone()).expect("text");
        assert_eq!(
            text,
            "{\"sent\":true,\"text\":\"hello\\nthere\"}\n{\"sent\":false,\"text\":\"{\\\"ok\\\":true}\"}\n{\"sent\":false,\"text\":\"AAEC\",\"binary\":true}\n"
        );
        let stream = StreamInfo {
            arrivals,
            ..Default::default()
        };
        let read = events(&body, Some(&stream));
        let live: Vec<Event> = messages
            .iter()
            .map(|(sent, frame, at)| event(*sent, frame, *at))
            .collect();
        assert_eq!(read, live, "as it was shown live");
        assert_eq!(
            read.iter().map(|event| event.event.as_str()).collect::<Vec<_>>(),
            [SENT, RECEIVED, RECEIVED_BINARY]
        );
        assert_eq!(read[0].data, "hello\nthere");
        assert_eq!(read[2].at, Some(12));
    }

    #[test]
    fn its_code_is_a_websocat_command() {
        let prepared = PreparedRequest {
            url: "wss://a.test/chat?room=1".into(),
            headers: vec![("Authorization".into(), "Bearer it's".into())],
            accept_invalid_certs: true,
            ..Default::default()
        };
        assert_eq!(
            websocat(&prepared),
            "websocat \\\n  -k \\\n  -H 'Authorization: Bearer it'\\''s' \\\n  'wss://a.test/chat?room=1'"
        );
    }
}
