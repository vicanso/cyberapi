//! How values are written for people to read (F15.6).

use crate::Timestamp;
use chrono::Datelike;
use serde::Serialize;

/// The spaces JSON is indented by wherever a reader sees it: a response, a
/// formatted body, an imported example, a copied value, an export, and the
/// JSON editors' Tab. Four, as the old app printed responses.
pub const JSON_INDENT: usize = 4;

/// `value` as JSON indented by [`JSON_INDENT`] spaces. What does not
/// serialize — a map with keys that are not strings — is empty.
pub fn json(value: &impl Serialize) -> String {
    let indent = [b' '; JSON_INDENT];
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(&indent);
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    match value.serialize(&mut serializer) {
        Ok(()) => String::from_utf8(out).unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// PERF-3: at most `limit` bytes of `bytes` as text, and whether there
/// was more: cut at a character, what is not UTF-8 replaced. Only the
/// bytes kept are read.
pub fn text_prefix(bytes: &[u8], limit: usize) -> (String, bool) {
    let (kept, cut) = if bytes.len() <= limit {
        (bytes, false)
    } else {
        let kept = &bytes[..limit];
        match std::str::from_utf8(kept) {
            // Only the last character was cut through.
            Err(error) if error.error_len().is_none() => (&kept[..error.valid_up_to()], true),
            _ => (kept, true),
        }
    };
    let text = String::from_utf8_lossy(kept).into_owned();
    // A replaced byte is three: still no more than asked.
    let (text, clipped) = clip(text, limit);
    (text, cut || clipped)
}

/// At most `limit` bytes of `text`, cut at a character.
fn clip(mut text: String, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text, false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    (text, true)
}

/// PERF-3: the start of `bytes`, JSON, indented as [`json`] indents it — at
/// most `limit` bytes, and whether there was more. Only as much of `bytes`
/// is read as that takes, so a 64 MB answer is not read whole to show its
/// first megabytes; what follows is not checked, and numbers and strings
/// stay as they came. Values one after another (JSON lines) are a line
/// apart.
pub fn json_prefix(bytes: &[u8], limit: usize) -> (String, bool) {
    let mut out: Vec<u8> = Vec::with_capacity(limit.min(bytes.len().saturating_mul(2)) + 64);
    let newline = |out: &mut Vec<u8>, depth: usize| {
        out.push(b'\n');
        out.extend(std::iter::repeat_n(b' ', depth * JSON_INDENT));
    };
    let (mut depth, mut at) = (0, 0);
    let (mut in_string, mut escaped, mut scalar) = (false, false, false);
    // A value on the top level ended: the next starts a line.
    let mut between = false;
    while at < bytes.len() && out.len() < limit {
        let byte = bytes[at];
        at += 1;
        if in_string {
            out.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
                between = depth == 0;
            }
            continue;
        }
        let starts = |out: &mut Vec<u8>, between: &mut bool| {
            if std::mem::take(between) {
                out.push(b'\n');
            }
        };
        match byte {
            b' ' | b'\t' | b'\n' | b'\r' => {
                between |= scalar && depth == 0;
                scalar = false;
            }
            b'"' => {
                starts(&mut out, &mut between);
                (in_string, scalar) = (true, false);
                out.push(byte);
            }
            b'{' | b'[' => {
                starts(&mut out, &mut between);
                scalar = false;
                let close = if byte == b'{' { b'}' } else { b']' };
                let next = bytes[at..]
                    .iter()
                    .position(|next| !next.is_ascii_whitespace())
                    .map(|skip| at + skip);
                // An empty one stays on its line.
                if let Some(next) = next.filter(|next| bytes[*next] == close) {
                    out.extend_from_slice(&[byte, close]);
                    at = next + 1;
                    between = depth == 0;
                    continue;
                }
                out.push(byte);
                depth += 1;
                newline(&mut out, depth);
            }
            b'}' | b']' => {
                scalar = false;
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(byte);
                between = depth == 0;
            }
            b',' => {
                scalar = false;
                out.push(byte);
                newline(&mut out, depth);
            }
            b':' => {
                scalar = false;
                out.extend_from_slice(b": ");
            }
            _ => {
                if !scalar {
                    starts(&mut out, &mut between);
                }
                scalar = true;
                out.push(byte);
            }
        }
    }
    let more = bytes[at..].iter().any(|byte| !byte.is_ascii_whitespace());
    let (text, clipped) = text_prefix(&out, limit);
    (text, more || clipped)
}

/// A time, short: `MM-DD HH:mm` within the reader's current year,
/// `YYYY-MM-DD` before it. Shown in `now`'s offset — the reader's — whatever
/// offset it was stored with. An unknown time (the epoch) is empty.
pub fn short_time(at: Timestamp, now: Timestamp) -> String {
    if at == Timestamp::default() {
        return String::new();
    }
    let local = at.get().with_timezone(now.get().offset());
    if local.year() == now.get().year() {
        local.format("%m-%d %H:%M").to_string()
    } else {
        local.format("%Y-%m-%d").to_string()
    }
}

/// How long ago something was, for a list that is read at a glance: the
/// moment, minutes, hours, the days of the last week; before that its date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ago {
    JustNow,
    Minutes(i64),
    Hours(i64),
    Days(i64),
    /// `MM-DD` within the reader's current year, `YYYY-MM-DD` before it;
    /// empty for an unknown time.
    On(String),
}

pub fn ago(at: Timestamp, now: Timestamp) -> Ago {
    if at == Timestamp::default() {
        return Ago::On(String::new());
    }
    let elapsed = now.get().signed_duration_since(at.get());
    match elapsed.num_minutes() {
        // A clock behind the store's reads as now.
        ..1 => Ago::JustNow,
        minutes @ 1..60 => Ago::Minutes(minutes),
        minutes if minutes < 24 * 60 => Ago::Hours(minutes / 60),
        minutes if minutes < 7 * 24 * 60 => Ago::Days(minutes / (24 * 60)),
        _ => {
            let local = at.get().with_timezone(now.get().offset());
            let format = if local.year() == now.get().year() {
                "%m-%d"
            } else {
                "%Y-%m-%d"
            };
            Ago::On(local.format(format).to_string())
        }
    }
}

/// Whole days from `now` until `at`, a day begun counting as one; none once
/// it is past. How long an invitation still lasts.
pub fn days_until(at: Timestamp, now: Timestamp) -> i64 {
    let seconds = at.get().signed_duration_since(now.get()).num_seconds();
    if seconds <= 0 {
        return 0;
    }
    (seconds + 24 * 60 * 60 - 1) / (24 * 60 * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PERF-3: the start of a JSON body, indented as the whole of it would
    /// be, without reading the rest.
    #[test]
    fn a_json_bodys_start_is_indented_as_the_whole() {
        let body = br#" {"a": [1, 2, {"b": null, "c": {}, "d": []}], "e": "x, y: {z}", "f": "q\"]" , "g": true} "#;
        let whole = json(&serde_json::from_slice::<serde_json::Value>(body).expect("JSON"));
        assert_eq!(json_prefix(body, usize::MAX), (whole.clone(), false));
        for limit in [1, 7, 20, 41, 80] {
            let (start, more) = json_prefix(body, limit);
            assert!(more && start.len() <= limit, "{limit}: {start:?}");
            assert!(whole.starts_with(&start), "{limit}: {start:?}");
        }
        // JSON lines: a line apart, as each would be.
        let lines = b"{\"a\": 1}\n{\"b\": [2]}\n";
        assert_eq!(
            json_prefix(lines, usize::MAX).0,
            "{\n    \"a\": 1\n}\n{\n    \"b\": [\n        2\n    ]\n}"
        );
        assert_eq!(json_prefix(b"1 2", usize::MAX).0, "1\n2");
    }

    /// A text's start is cut at a character, and is never longer than asked.
    #[test]
    fn a_texts_start_is_cut_at_a_character() {
        let text = "aé€".as_bytes();
        assert_eq!(text_prefix(text, 100), ("aé€".to_string(), false));
        assert_eq!(
            text_prefix(text, 4),
            ("aé".to_string(), true),
            "€ cut through is left out"
        );
        assert_eq!(text_prefix(text, 2), ("a".to_string(), true));
        let (bad, cut) = text_prefix(&[0xff, 0xff, b'a'], 2);
        assert!(cut && bad.len() <= 2, "{bad:?}");
    }

    #[test]
    fn json_is_indented_by_four_spaces_wherever_it_is_shown() {
        let value = serde_json::json!({"a": [1, {"b": null}]});
        assert_eq!(
            json(&value),
            "{\n    \"a\": [\n        1,\n        {\n            \"b\": null\n        }\n    ]\n}"
        );
        assert_eq!(JSON_INDENT, 4);
    }

    #[test]
    fn a_day_begun_counts() {
        let now = Timestamp::from_unix(1_700_000_000);
        let later = |seconds: i64| Timestamp::from_unix(1_700_000_000 + seconds);
        assert_eq!(days_until(later(7 * 24 * 60 * 60), now), 7);
        assert_eq!(days_until(later(7 * 24 * 60 * 60 - 60), now), 7);
        assert_eq!(days_until(later(60), now), 1);
        assert_eq!(days_until(later(-60), now), 0);
    }

    #[test]
    fn this_year_shows_the_day_and_time() {
        let now = Timestamp::parse_lenient("2026-10-20T21:00:00+08:00");
        assert_eq!(
            short_time(Timestamp::parse_lenient("2026-03-05T09:07:00+08:00"), now),
            "03-05 09:07"
        );
        // Stored in UTC, read in the reader's offset.
        assert_eq!(
            short_time(Timestamp::parse_lenient("2026-03-05T01:07:00Z"), now),
            "03-05 09:07"
        );
    }

    #[test]
    fn ago_reads_at_a_glance() {
        let now = Timestamp::parse_lenient("2026-10-20T21:00:00+08:00");
        let at = |text: &str| ago(Timestamp::parse_lenient(text), now);
        assert_eq!(at("2026-10-20T20:59:30+08:00"), Ago::JustNow);
        assert_eq!(
            at("2026-10-20T21:05:00+08:00"),
            Ago::JustNow,
            "a time ahead of the clock"
        );
        assert_eq!(at("2026-10-20T20:15:00+08:00"), Ago::Minutes(45));
        assert_eq!(at("2026-10-20T10:00:00Z"), Ago::Hours(3));
        assert_eq!(at("2026-10-14T21:00:00+08:00"), Ago::Days(6));
        assert_eq!(at("2026-10-13T20:00:00+08:00"), Ago::On("10-13".into()));
        assert_eq!(at("2023-07-15T10:00:00+08:00"), Ago::On("2023-07-15".into()));
        assert_eq!(ago(Timestamp::default(), now), Ago::On(String::new()));
    }

    #[test]
    fn earlier_years_show_the_date() {
        let now = Timestamp::parse_lenient("2026-10-20T21:00:00+08:00");
        assert_eq!(
            short_time(Timestamp::parse_lenient("2022-09-05T10:03:34+08:00"), now),
            "2022-09-05"
        );
        assert_eq!(short_time(Timestamp::default(), now), "");
    }
}
