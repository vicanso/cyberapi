//! Server-sent events (`text/event-stream`): a body read as it comes, one
//! event per blank-line-ended block of `field: value` lines, after the
//! HTML specification's event stream format. [`Parser`] takes the body in
//! the pieces it came in, so an event has the time its last piece arrived;
//! [`events`] reads a kept body with the arrivals its [`StreamInfo`] has.
//!
//! Unlike a browser's `EventSource`, an event that names its type and has
//! no data is kept (a server's `event: ping` is worth seeing), and so is
//! one the stream ended in the middle of.

use crate::response::StreamInfo;

/// The content type of an event stream.
pub const CONTENT_TYPE: &str = "text/event-stream";

/// Whether headers say the body is an event stream.
pub fn is_event_stream(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type") && value.trim().to_ascii_lowercase().starts_with(CONTENT_TYPE)
    })
}

/// One event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Event {
    /// Its type; empty is `message`.
    pub event: String,
    /// Its `data` lines, joined by line breaks.
    pub data: String,
    /// The `id` it set, if it set one.
    pub id: Option<String>,
    /// The reconnection time it asked for, in milliseconds.
    pub retry: Option<u64>,
    /// When it came, in milliseconds after the head: when the piece that
    /// ended it did.
    pub at: Option<u64>,
}

impl Event {
    pub fn kind(&self) -> &str {
        if self.event.is_empty() { "message" } else { &self.event }
    }

    fn is_empty(&self) -> bool {
        self.event.is_empty() && self.data.is_empty() && self.id.is_none() && self.retry.is_none()
    }
}

/// Reads an event stream as it comes.
#[derive(Default)]
pub struct Parser {
    /// The line not ended yet.
    line: Vec<u8>,
    /// A carriage return ended the last piece: a line feed starting the
    /// next one is the same line break.
    after_cr: bool,
    started: bool,
    /// The event being read, and whether a `data` line has come for it.
    current: Event,
    has_data: bool,
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    /// The events `piece` ends, the stream's pieces before it already fed;
    /// `at` is when it came.
    pub fn feed(&mut self, piece: &[u8], at: Option<u64>) -> Vec<Event> {
        let mut events = Vec::new();
        let mut bytes = piece;
        if !self.started && !bytes.is_empty() {
            self.started = true;
            bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        }
        for &byte in bytes {
            if std::mem::take(&mut self.after_cr) && byte == b'\n' {
                continue;
            }
            match byte {
                b'\n' | b'\r' => {
                    self.after_cr = byte == b'\r';
                    let line = std::mem::take(&mut self.line);
                    if let Some(event) = self.line_ended(&line, at) {
                        events.push(event);
                    }
                }
                _ => self.line.push(byte),
            }
        }
        events
    }

    /// The event the stream ended in the middle of, if it had begun one.
    pub fn finish(mut self, at: Option<u64>) -> Option<Event> {
        if !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            self.line_ended(&line, at);
        }
        let event = std::mem::take(&mut self.current);
        (!event.is_empty()).then_some(Event { at, ..event })
    }

    fn line_ended(&mut self, line: &[u8], at: Option<u64>) -> Option<Event> {
        if line.is_empty() {
            let event = std::mem::take(&mut self.current);
            self.has_data = false;
            return (!event.is_empty()).then_some(Event { at, ..event });
        }
        // A comment: a keep-alive, most often.
        if line[0] == b':' {
            return None;
        }
        let line = String::from_utf8_lossy(line);
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line.as_ref(), ""),
        };
        match field {
            "event" => self.current.event = value.to_string(),
            "data" => {
                if self.has_data {
                    self.current.data.push('\n');
                }
                self.current.data.push_str(value);
                self.has_data = true;
            }
            "id" if !value.contains('\0') => self.current.id = Some(value.to_string()),
            "retry" => {
                if let Ok(retry) = value.parse() {
                    self.current.retry = Some(retry);
                }
            }
            _ => {}
        }
        None
    }
}

/// The events of a kept body, each with its time where the stream kept its
/// pieces' arrivals.
pub fn events(body: &[u8], stream: Option<&StreamInfo>) -> Vec<Event> {
    let mut parser = Parser::new();
    let mut events = Vec::new();
    let mut from = 0;
    let mut last = None;
    for &(end, at) in stream.map(|stream| stream.arrivals.as_slice()).unwrap_or_default() {
        let end = (end as usize).min(body.len());
        if end <= from {
            continue;
        }
        events.extend(parser.feed(&body[from..end], Some(at)));
        from = end;
        last = Some(at);
    }
    if from < body.len() {
        events.extend(parser.feed(&body[from..], last));
    }
    events.extend(parser.finish(last));
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::response::StreamEnd;
    use pretty_assertions::assert_eq;

    fn event(kind: &str, data: &str) -> Event {
        Event {
            event: kind.into(),
            data: data.into(),
            ..Default::default()
        }
    }

    #[test]
    fn events_are_read_whatever_their_pieces() {
        let body = "\u{feff}: ping\n\nevent: delta\ndata: a\ndata: b\nid: 7\nretry: 300\n\ndata:no space\r\n\r\nevent: ping\n\n";
        let whole = events(body.as_bytes(), None);
        let expected = vec![
            Event {
                id: Some("7".into()),
                retry: Some(300),
                ..event("delta", "a\nb")
            },
            event("", "no space"),
            event("ping", ""),
        ];
        assert_eq!(whole, expected);
        assert_eq!(whole[1].kind(), "message");
        // Byte by byte, a line break split between pieces included.
        let mut parser = Parser::new();
        let mut split = Vec::new();
        for byte in body.as_bytes() {
            split.extend(parser.feed(&[*byte], None));
        }
        assert_eq!(split, expected);
    }

    #[test]
    fn each_event_has_the_time_of_the_piece_that_ended_it() {
        let body = b"data: one\n\ndata: tw";
        let stream = StreamInfo {
            arrivals: vec![(5, 10), (11, 20), (19, 35)],
            ended: StreamEnd::Stopped,
        };
        let read = events(body, Some(&stream));
        assert_eq!(
            read.iter().map(|e| (e.data.as_str(), e.at)).collect::<Vec<_>>(),
            [("one", Some(20)), ("tw", Some(35))]
        );
        assert!(events(b": only a comment\n", None).is_empty());
    }

    #[test]
    fn the_content_type_says_a_stream() {
        let headers = |value: &str| vec![("Content-Type".to_string(), value.to_string())];
        assert!(is_event_stream(&headers("text/event-stream; charset=utf-8")));
        assert!(is_event_stream(&headers(" Text/Event-Stream")));
        assert!(!is_event_stream(&headers("application/json")));
    }
}
