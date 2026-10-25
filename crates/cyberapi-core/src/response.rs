//! What came back (4.4, M7): the engine's [`Exchange`], the [`HttpResponse`]
//! kept in an API's history, and the rules for showing one.

use crate::media::{self, ImageType, Sniffed};
use crate::model::HttpRequest;
use crate::redirect;
use crate::send::base64_bytes;
use crate::{CollectionId, ItemId, Timestamp};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// How many responses an API keeps, newest first (F7.9).
pub const HISTORY_LIMIT: usize = 10;
/// The biggest body an API's history keeps (#17). A bigger one shows when it
/// arrives; the history keeps only its size.
pub const HISTORY_BODY_LIMIT: usize = 2 * 1024 * 1024;
/// The most of a text body the response panel shows; the rest is left out,
/// and the panel says so.
pub const DISPLAY_LIMIT: usize = 1024 * 1024;
/// The longest line the panel wraps (#51). Wrapping one line costs about
/// five microseconds a byte, all before the panel draws: a 600 KB body of
/// minified JSON, raw, held the window for three seconds. A longer line
/// scrolls sideways, and the panel says so.
pub const WRAP_LINE_LIMIT: usize = 64 * 1024;
/// The most of a body the engine reads, as it comes and once decompressed:
/// a bigger one is cut there, so one answer, or a small archive that
/// unpacks to gigabytes, cannot take the memory of a bridge everyone uses.
pub const RESPONSE_LIMIT: usize = 64 * 1024 * 1024;

/// Where the time of one exchange went (F5.6). Each phase is measured on its
/// own connection, so two requests at once no longer mix their numbers (#1).
/// A phase that did not happen — TLS over plain HTTP — is zero.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HttpStats {
    pub remote_addr: String,
    pub is_https: bool,
    /// The protocol the answer came in: `HTTP/1.1`, `HTTP/2`. Empty in
    /// entries kept before this was.
    pub version: String,
    /// The TLS cipher suite, when there was TLS.
    pub cipher: String,
    #[serde(with = "micros")]
    pub dns: Duration,
    #[serde(with = "micros")]
    pub tcp: Duration,
    #[serde(with = "micros")]
    pub tls: Duration,
    /// Writing the request.
    #[serde(with = "micros")]
    pub send: Duration,
    /// From the request written to the answer's start: its first byte over
    /// HTTP/1.1, its headers over HTTP/2, where the connection's own frames
    /// come first.
    #[serde(with = "micros")]
    pub server_processing: Duration,
    #[serde(with = "micros")]
    pub content_transfer: Duration,
    #[serde(with = "micros")]
    pub total: Duration,
}

/// What the engine answers for a request that reached a server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exchange {
    pub status: u16,
    /// Names in lower case, in the order they came, repeats kept.
    pub headers: Vec<(String, String)>,
    /// Decompressed.
    #[serde(with = "base64_bytes")]
    pub body: Vec<u8>,
    /// The body's size as it came, before decompression.
    pub body_size: u64,
    pub stats: HttpStats,
    /// The body was longer than [`RESPONSE_LIMIT`]: this is its start.
    #[serde(default)]
    pub truncated: bool,
    /// The trailers after the body (a gRPC call's status), names in lower
    /// case.
    #[serde(default)]
    pub trailers: Vec<(String, String)>,
}

/// One entry of an API's history (F7.9): an exchange, or why there was none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HttpResponse {
    /// Given by the history store.
    pub id: String,
    pub api: ItemId,
    pub collection: CollectionId,
    /// The request as the API held it, before variables and functions: what
    /// choosing this entry puts back in the editor (F7.10).
    pub request: HttpRequest,
    /// The URL it went to: variables resolved, the query in it. Empty in
    /// entries kept before this was.
    pub url: String,
    /// `None` when the request failed; [`Self::error`] says why (F5.7).
    pub status: Option<u16>,
    pub headers: Vec<(String, String)>,
    #[serde(with = "base64_bytes")]
    pub body: Vec<u8>,
    pub error: Option<String>,
    pub body_size: u64,
    /// Total time, never zero (F5.7).
    #[serde(with = "micros")]
    pub latency: Duration,
    pub stats: HttpStats,
    pub created_at: Timestamp,
    /// The body was too big to keep in the history (#17): its length, the
    /// body itself left out.
    pub body_omitted: Option<u64>,
    /// The body was longer than the engine reads ([`RESPONSE_LIMIT`]): this
    /// is its start.
    pub truncated: bool,
}

/// How the id of an answer the history did not keep starts: no store gives
/// one like it (they give ULIDs).
const UNKEPT: &str = "unkept-";
/// How the id of an answer a page keeps for the session starts: its API is
/// not on the server yet (#52).
const WAITING: &str = "waiting-";

impl HttpResponse {
    /// The id of an answer the history did not keep, `n` telling it from
    /// the others.
    pub fn unkept_id(n: u64) -> String {
        format!("{UNKEPT}{n}")
    }

    /// The id of an answer a page keeps until its API reaches the server,
    /// `n` telling it from the others.
    pub fn waiting_id(n: u64) -> String {
        format!("{WAITING}{n}")
    }

    /// The history kept this answer: it is still there once the project is
    /// opened again.
    pub fn kept(&self) -> bool {
        !self.id.starts_with(UNKEPT) && !self.waits_for_sync()
    }

    /// The page keeps this answer while the project is open, and the
    /// history takes it once its API is synced (#52, #53).
    pub fn waits_for_sync(&self) -> bool {
        self.id.starts_with(WAITING)
    }

    /// Every value of a header, ignoring case.
    pub fn header_values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> {
        self.headers
            .iter()
            .filter(move |(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Where this answer redirects to, when it is a redirect a send could
    /// follow (F14.8): its `Location`, against the address it came from.
    pub fn redirects_to(&self) -> Option<String> {
        redirect::target(&self.url, self.status?, &self.headers).map(String::from)
    }

    pub fn content_type(&self) -> String {
        self.header_values("content-type").collect::<Vec<_>>().join(" ")
    }

    /// F7.1. A declared type wins, unless it says only "bytes"
    /// (`application/octet-stream`): an image or a PDF among them is shown
    /// as one. With no type, the first bytes tell an image or a PDF; a
    /// UTF-8 body is text and an object or array is JSON; bytes that are
    /// not UTF-8 stay binary (#43).
    pub fn body_kind(&self) -> BodyKind {
        let declared = self.content_type();
        if declared.trim().is_empty() {
            return sniffed_kind(&self.body);
        }
        match body_kind(&declared) {
            BodyKind::Binary => match media::sniff(&self.body) {
                Some(Sniffed::Image(_)) => BodyKind::Image,
                Some(Sniffed::Pdf) => BodyKind::Pdf,
                None => BodyKind::Binary,
            },
            kind => kind,
        }
    }

    /// An image body's format (F7.4): what its bytes start as, or else
    /// what its content type names.
    pub fn image_type(&self) -> Option<ImageType> {
        if self.body_kind() != BodyKind::Image {
            return None;
        }
        Some(match media::sniff(&self.body) {
            Some(Sniffed::Image(kind)) => kind,
            _ => ImageType::declared(&self.content_type()).unwrap_or(ImageType::Other),
        })
    }

    /// An image body's width and height, when its header says (F7.4).
    pub fn image_size(&self) -> Option<(u32, u32)> {
        media::image_size(&self.body, self.image_type()?)
    }

    /// The body's media type: the declared one when it says what the body
    /// is shown as, otherwise the one its bytes were told as. For handing
    /// the body to something outside the app (F7.4).
    pub fn media_type(&self) -> String {
        let declared = self.content_type();
        let kind = self.body_kind();
        if !declared.trim().is_empty() && body_kind(&declared) == kind {
            return declared;
        }
        match kind {
            BodyKind::Json => "application/json".into(),
            BodyKind::Text => "text/plain; charset=utf-8".into(),
            BodyKind::Image => self.image_type().unwrap_or(ImageType::Other).media_type().into(),
            BodyKind::Pdf => "application/pdf".into(),
            BodyKind::Binary => "application/octet-stream".into(),
        }
    }

    /// F7.1, the pretty view: JSON indented by four spaces; text as it is;
    /// anything else, and JSON that does not parse, as the kind says.
    pub fn body_text(&self) -> Option<String> {
        match self.body_kind() {
            BodyKind::Json => {
                Some(pretty_json(&self.body).unwrap_or_else(|| String::from_utf8_lossy(&self.body).into()))
            }
            BodyKind::Text => Some(String::from_utf8_lossy(&self.body).into_owned()),
            BodyKind::Image | BodyKind::Pdf | BodyKind::Binary => None,
        }
    }

    /// The raw view (#51): JSON and text exactly as the server sent them,
    /// nothing reindented; anything else as the kind says.
    pub fn raw_body_text(&self) -> Option<String> {
        match self.body_kind() {
            BodyKind::Json | BodyKind::Text => Some(String::from_utf8_lossy(&self.body).into_owned()),
            BodyKind::Image | BodyKind::Pdf | BodyKind::Binary => None,
        }
    }

    /// The body as JSON, for the `get` template function.
    pub fn body_json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }

    /// F7.2: `content-length` when the server sent one, the body otherwise.
    pub fn size(&self) -> u64 {
        self.header_values("content-length")
            .next()
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(self.body_omitted.unwrap_or(self.body.len() as u64))
    }

    /// F7.2: for a body that came compressed, its size on the wire and that
    /// as a whole percentage (rounded up) of the body it decoded to.
    pub fn compression(&self) -> Option<(u64, u64)> {
        let decoded = self.body_omitted.unwrap_or(self.body.len() as u64);
        (self.body_size > 0 && self.body_size < decoded)
            .then(|| (self.body_size, (self.body_size * 100).div_ceil(decoded)))
    }

    /// The name to save the body as: the one the server offers (F7.5), or
    /// `response` with an extension for its kind.
    pub fn file_name(&self) -> String {
        if let Some(name) = self.attachment_name() {
            return name;
        }
        let extension = match self.body_kind() {
            BodyKind::Json => "json".to_string(),
            // A page opens as one, in the system's browser (F7.4).
            BodyKind::Text => {
                let content_type = self.content_type().to_ascii_lowercase();
                if content_type.contains("html") {
                    "html".to_string()
                } else if content_type.contains("xml") {
                    "xml".to_string()
                } else {
                    "txt".to_string()
                }
            }
            BodyKind::Image => self.image_type().unwrap_or(ImageType::Other).extension().to_string(),
            BodyKind::Pdf => "pdf".to_string(),
            BodyKind::Binary => {
                let content_type = self.content_type();
                let subtype = content_type
                    .split(';')
                    .next()
                    .and_then(|essence| essence.split_once('/'))
                    .map(|(_, subtype)| subtype.trim().to_ascii_lowercase())
                    .unwrap_or_default();
                let plain = !subtype.is_empty() && subtype.chars().all(|ch| ch.is_ascii_alphanumeric());
                if plain { subtype } else { "bin".into() }
            }
        };
        format!("response.{extension}")
    }

    /// What the history keeps of this response: all of it, or, for a body
    /// over [`HISTORY_BODY_LIMIT`], everything but the body (#17).
    pub fn for_history(&self) -> HttpResponse {
        if self.body.len() <= HISTORY_BODY_LIMIT {
            return self.clone();
        }
        HttpResponse {
            id: self.id.clone(),
            api: self.api.clone(),
            collection: self.collection.clone(),
            request: self.request.clone(),
            url: self.url.clone(),
            status: self.status,
            headers: self.headers.clone(),
            body: Vec::new(),
            error: self.error.clone(),
            body_size: self.body_size,
            latency: self.latency,
            stats: self.stats.clone(),
            created_at: self.created_at,
            body_omitted: Some(self.body.len() as u64),
            truncated: self.truncated,
        }
    }

    /// The headers grouped by name for display (F7.6): `Content-Type`, with
    /// every value of a repeated header.
    pub fn display_headers(&self) -> Vec<(String, Vec<String>)> {
        let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
        for (name, value) in &self.headers {
            let name = display_header_name(name);
            match grouped.iter_mut().find(|(key, _)| *key == name) {
                Some((_, values)) => values.push(value.clone()),
                None => grouped.push((name, vec![value.clone()])),
            }
        }
        grouped
    }

    /// F7.6: each `set-cookie` as `name=value`.
    pub fn cookies(&self) -> Vec<(String, String)> {
        self.header_values("set-cookie")
            .filter_map(|cookie| {
                let pair = cookie.split(';').next()?;
                let (name, value) = pair.split_once('=')?;
                Some((name.trim().to_string(), value.trim().to_string()))
            })
            .collect()
    }

    /// F7.5: the file name a `content-disposition` offers.
    pub fn attachment_name(&self) -> Option<String> {
        self.header_values("content-disposition").find_map(|value| {
            value.split(';').find_map(|part| {
                let (key, name) = part.trim().split_once('=')?;
                (key.trim().eq_ignore_ascii_case("filename"))
                    .then(|| name.trim().trim_matches('"').to_string())
                    .filter(|name| !name.is_empty())
            })
        })
    }
}

/// How a body is shown (F7.1, F7.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BodyKind {
    Json,
    Text,
    Image,
    Pdf,
    Binary,
}

/// A body whose response named no content type (#43): an image or a PDF
/// by its first bytes. Other bytes that are not UTF-8 stay binary; a zero
/// byte is UTF-8 and reads as text.
fn sniffed_kind(bytes: &[u8]) -> BodyKind {
    match media::sniff(bytes) {
        Some(Sniffed::Image(_)) => return BodyKind::Image,
        Some(Sniffed::Pdf) => return BodyKind::Pdf,
        None => {}
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return BodyKind::Binary;
    };
    let trimmed = text.trim();
    let json = (trimmed.starts_with('{') || trimmed.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(trimmed).is_ok();
    if json { BodyKind::Json } else { BodyKind::Text }
}

/// F7.1 / F7.4 by content type. XML and HTML read as text; the old app
/// showed XML as binary. An SVG is an image, though it is XML.
pub fn body_kind(content_type: &str) -> BodyKind {
    let lower = content_type.to_ascii_lowercase();
    if lower.contains("application/json") || lower.contains("+json") {
        BodyKind::Json
    } else if lower.contains("image") {
        BodyKind::Image
    } else if lower.contains("text") || lower.contains("javascript") || lower.contains("xml") {
        BodyKind::Text
    } else if lower.contains("pdf") {
        BodyKind::Pdf
    } else {
        BodyKind::Binary
    }
}

/// JSON indented as everywhere ([`crate::format::json`]). A body that is
/// not one value but one per line (NDJSON) is each line indented (F15.5).
pub fn pretty_json(bytes: &[u8]) -> Option<String> {
    fn pretty(value: &serde_json::Value) -> Option<String> {
        Some(crate::format::json(value)).filter(|text| !text.is_empty())
    }
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) {
        return pretty(&value);
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    if lines.len() < 2 {
        return None;
    }
    lines
        .iter()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|value| pretty(&value))
        })
        .collect::<Option<Vec<_>>>()
        .map(|values| values.join("\n"))
}

/// At most `limit` bytes of `text`, cut at a character, and whether it was
/// cut.
/// The longest line of `text`, in bytes, when it is longer than
/// [`WRAP_LINE_LIMIT`]: too long to wrap.
pub fn unwrappable_line(text: &str) -> Option<usize> {
    text.split('\n')
        .map(str::len)
        .max()
        .filter(|longest| *longest > WRAP_LINE_LIMIT)
}

pub fn clip_text(mut text: String, limit: usize) -> (String, bool) {
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

/// `content-type` → `Content-Type` (F7.6).
pub fn display_header_name(name: &str) -> String {
    name.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// How a status reads (F7.3): its color in the response panel. The old
/// app read 3xx as success; here a redirect is its own, as the panel says
/// where it points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusLevel {
    Success,
    Redirect,
    ClientError,
    ServerError,
}

pub fn status_level(status: u16) -> StatusLevel {
    match status {
        500.. => StatusLevel::ServerError,
        400..500 => StatusLevel::ClientError,
        300..400 => StatusLevel::Redirect,
        _ => StatusLevel::Success,
    }
}

/// The reason phrase (F7.3); empty for a code without one.
pub fn status_text(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        101 => "Switching Protocols",
        102 => "Processing",
        103 => "Early Hints",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        207 => "Multi-Status",
        208 => "Already Reported",
        226 => "IM Used",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        305 => "Use Proxy",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Request Entity Too Large",
        414 => "Request URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Requested Range Not Satisfiable",
        417 => "Expectation Failed",
        418 => "I'm a teapot",
        421 => "Misdirected Request",
        422 => "Unprocessable Entity",
        423 => "Locked",
        424 => "Failed Dependency",
        425 => "Too Early",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        506 => "Variant Also Negotiates",
        507 => "Insufficient Storage",
        508 => "Loop Detected",
        510 => "Not Extended",
        511 => "Network Authentication Required",
        _ => "",
    }
}

/// `1.2 KB`, `340 B` — sizes in the response panel (F7.2).
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.;
    let mut unit = 0;
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// `12 ms`, `1.25 s`.
pub fn format_duration(duration: Duration) -> String {
    let millis = duration.as_secs_f64() * 1000.;
    if millis < 1. {
        format!("{:.2} ms", millis)
    } else if millis < 1000. {
        format!("{} ms", millis.round())
    } else {
        format!("{:.2} s", millis / 1000.)
    }
}

/// Durations as whole microseconds in JSON.
pub mod micros {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(u64::try_from(duration.as_micros()).unwrap_or(u64::MAX))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_micros(u64::deserialize(deserializer)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn response(headers: &[(&str, &str)], body: &[u8]) -> HttpResponse {
        HttpResponse {
            status: Some(200),
            headers: headers
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            body: body.to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn a_big_body_is_not_kept_but_its_size_is() {
        let small = response(&[], b"ok");
        assert_eq!(small.for_history(), small);
        let big = response(&[], &vec![b'x'; HISTORY_BODY_LIMIT + 1]);
        let kept = big.for_history();
        assert!(kept.body.is_empty());
        assert_eq!(kept.body_omitted, Some(HISTORY_BODY_LIMIT as u64 + 1));
        assert_eq!(kept.size(), big.size(), "the size still shows");
    }

    #[test]
    fn a_redirect_says_where_it_points() {
        let found = HttpResponse {
            status: Some(302),
            url: "http://api.test/v1/old?x=1".into(),
            ..response(&[("Location", "/users")], b"")
        };
        assert_eq!(found.redirects_to().as_deref(), Some("http://api.test/users"));
        let modified = HttpResponse {
            status: Some(304),
            ..found.clone()
        };
        assert_eq!(modified.redirects_to(), None, "not modified points nowhere");
        let old = HttpResponse {
            url: String::new(),
            ..found
        };
        assert_eq!(old.redirects_to(), None, "kept before the address was");
    }

    #[test]
    fn ndjson_is_indented_line_by_line() {
        assert_eq!(
            pretty_json(b"{\"a\":1}\n\n{\"b\":[2]}\n").as_deref(),
            Some("{\n    \"a\": 1\n}\n{\n    \"b\": [\n        2\n    ]\n}")
        );
        assert_eq!(pretty_json(b"{\"a\":1}\nnot json"), None);
        assert_eq!(pretty_json(b"not json"), None);
    }

    #[test]
    fn a_body_is_saved_under_the_name_offered_or_its_kind() {
        let offered = response(&[("content-disposition", "attachment; filename=\"a.csv\"")], b"x");
        assert_eq!(offered.file_name(), "a.csv");
        assert_eq!(
            response(&[("content-type", "application/json")], b"{}").file_name(),
            "response.json"
        );
        assert_eq!(
            response(&[("content-type", "text/html; charset=utf-8")], b"<p>").file_name(),
            "response.html"
        );
        assert_eq!(
            response(&[("content-type", "image/png")], b"").file_name(),
            "response.png"
        );
        assert_eq!(
            response(&[("content-type", "application/x-thing")], b"").file_name(),
            "response.bin"
        );
        assert_eq!(
            response(&[("content-type", "image/svg+xml")], b"<svg/>").file_name(),
            "response.svg",
            "a type that is not one word names its image"
        );
        assert_eq!(
            response(&[("content-type", "image/x-icon")], b"").file_name(),
            "response.ico"
        );
        assert_eq!(
            response(&[("content-type", "application/octet-stream")], PNG).file_name(),
            "response.png",
            "bytes told as an image by their start"
        );
        assert_eq!(response(&[], b"%PDF-1.4\n").file_name(), "response.pdf");
    }

    #[test]
    fn text_is_clipped_at_a_character() {
        assert_eq!(clip_text("abc".into(), 5), ("abc".into(), false));
        assert_eq!(clip_text("a中b".into(), 2), ("a".into(), true));
    }

    #[test]
    fn a_line_too_long_to_wrap_is_found() {
        let long = "x".repeat(WRAP_LINE_LIMIT + 1);
        assert_eq!(unwrappable_line(&long), Some(WRAP_LINE_LIMIT + 1));
        assert_eq!(unwrappable_line(&format!("{{\n{long}\n}}")), Some(WRAP_LINE_LIMIT + 1));
        // Many lines at the limit, which is the pretty view of a big body,
        // wrap: only one line's length counts.
        let lines = vec!["y".repeat(WRAP_LINE_LIMIT); 4].join("\n");
        assert_eq!(unwrappable_line(&lines), None);
        assert_eq!(unwrappable_line(""), None);
    }

    #[test]
    fn bodies_are_shown_by_content_type() {
        let json = response(
            &[("content-type", "application/json; charset=utf-8")],
            br#"{"a":[1,2]}"#,
        );
        assert_eq!(json.body_kind(), BodyKind::Json);
        assert_eq!(
            json.body_text().as_deref(),
            Some("{\n    \"a\": [\n        1,\n        2\n    ]\n}")
        );
        assert_eq!(
            json.raw_body_text().as_deref(),
            Some(r#"{"a":[1,2]}"#),
            "the raw view keeps it as it came"
        );
        let broken = response(&[("content-type", "application/json")], b"{nope");
        assert_eq!(broken.body_text().as_deref(), Some("{nope"));

        assert_eq!(body_kind("text/html"), BodyKind::Text);
        assert_eq!(body_kind("application/javascript"), BodyKind::Text);
        assert_eq!(body_kind("application/xml"), BodyKind::Text);
        assert_eq!(body_kind("application/problem+json"), BodyKind::Json);
        assert_eq!(body_kind("image/png"), BodyKind::Image);
        assert_eq!(body_kind("image/svg+xml"), BodyKind::Image, "drawn, though XML");
        assert_eq!(body_kind("application/pdf"), BodyKind::Pdf);
        assert_eq!(body_kind("application/octet-stream"), BodyKind::Binary);
        // No type and not UTF-8: still binary. A zero byte is UTF-8, so it
        // is text; the byte that is not is what stays binary (#43).
        assert_eq!(response(&[], b"\xff").body_text(), None);
        assert_eq!(response(&[], b"\xff").raw_body_text(), None);
    }

    #[test]
    fn a_body_without_a_content_type_is_sniffed() {
        assert_eq!(response(&[], br#"{"ok":true}"#).body_kind(), BodyKind::Json);
        assert_eq!(response(&[], b"hello").body_kind(), BodyKind::Text);
        assert_eq!(response(&[], b"\xff").body_kind(), BodyKind::Binary);
        assert_eq!(
            response(&[("content-type", "application/octet-stream")], br#"{"ok":true}"#).body_kind(),
            BodyKind::Binary
        );
        assert_eq!(
            response(&[("content-type", "  ")], b"hello").body_kind(),
            BodyKind::Text
        );
    }

    /// A PNG's first bytes, and its header: 640 × 480.
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x02\x80\x00\x00\x01\xe0\x08\x06\x00\x00\x00";

    #[test]
    fn an_image_or_a_pdf_is_told_by_its_first_bytes_when_the_type_does_not_say() {
        let untyped = response(&[], PNG);
        assert_eq!(untyped.body_kind(), BodyKind::Image);
        assert_eq!(untyped.image_type(), Some(ImageType::Png));
        assert_eq!(untyped.image_size(), Some((640, 480)));
        assert_eq!(untyped.media_type(), "image/png");
        let bytes = response(&[("content-type", "application/octet-stream")], PNG);
        assert_eq!(bytes.body_kind(), BodyKind::Image);
        assert_eq!(bytes.media_type(), "image/png", "what it was told as");
        assert_eq!(response(&[], b"%PDF-1.7\n").body_kind(), BodyKind::Pdf);
        assert_eq!(response(&[], b"%PDF-1.7\n").media_type(), "application/pdf");
        assert_eq!(
            response(&[("content-type", "application/octet-stream")], b"PK\x03\x04").body_kind(),
            BodyKind::Binary
        );
        // The bytes, over a type that names another format: they are what
        // the image is decoded as.
        let mislabeled = response(&[("content-type", "image/jpeg")], PNG);
        assert_eq!(mislabeled.image_type(), Some(ImageType::Png));
        assert_eq!(mislabeled.media_type(), "image/jpeg", "a declared image type is kept");
        let svg = response(&[("content-type", "image/svg+xml")], br#"<svg width="3" height="2"/>"#);
        assert_eq!(svg.image_type(), Some(ImageType::Svg));
        assert_eq!(svg.image_size(), Some((3, 2)));
        assert_eq!(
            response(&[("content-type", "image/jxl")], b"\xff\x0a").image_type(),
            Some(ImageType::Other)
        );
        assert_eq!(response(&[], b"hello").image_type(), None);
    }

    #[test]
    fn size_prefers_content_length() {
        assert_eq!(response(&[("content-length", " 1234 ")], b"ab").size(), 1234);
        assert_eq!(response(&[], b"ab").size(), 2);
        assert_eq!(format_size(340), "340 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn compression_is_the_wire_against_the_body() {
        let mut resp = response(&[], &[b'a'; 1000]);
        assert_eq!(resp.compression(), None, "not compressed");
        resp.body_size = 1000;
        assert_eq!(resp.compression(), None, "the same size");
        resp.body_size = 241;
        assert_eq!(resp.compression(), Some((241, 25)));
        resp.body = Vec::new();
        resp.body_omitted = Some(1000);
        assert_eq!(resp.compression(), Some((241, 25)), "a body kept out of the history");
    }

    #[test]
    fn headers_display_grouped_and_capitalized() {
        let resp = response(
            &[
                ("content-type", "text/plain"),
                ("set-cookie", "a=1; Path=/"),
                ("x-request-id", "r"),
                ("set-cookie", "b=2"),
                ("content-disposition", "attachment; filename=\"report.csv\""),
            ],
            b"",
        );
        assert_eq!(
            resp.display_headers(),
            [
                ("Content-Type".to_string(), vec!["text/plain".to_string()]),
                (
                    "Set-Cookie".to_string(),
                    vec!["a=1; Path=/".to_string(), "b=2".to_string()]
                ),
                ("X-Request-Id".to_string(), vec!["r".to_string()]),
                (
                    "Content-Disposition".to_string(),
                    vec!["attachment; filename=\"report.csv\"".to_string()]
                ),
            ]
        );
        assert_eq!(
            resp.cookies(),
            [("a".to_string(), "1".to_string()), ("b".to_string(), "2".to_string())]
        );
        assert_eq!(resp.attachment_name().as_deref(), Some("report.csv"));
    }

    #[test]
    fn statuses_read_and_level() {
        assert_eq!(status_text(200), "OK");
        assert_eq!(status_text(418), "I'm a teapot");
        assert_eq!(status_text(299), "");
        assert_eq!(status_level(204), StatusLevel::Success);
        assert_eq!(status_level(302), StatusLevel::Redirect);
        assert_eq!(status_level(304), StatusLevel::Redirect);
        assert_eq!(status_level(404), StatusLevel::ClientError);
        assert_eq!(status_level(503), StatusLevel::ServerError);
    }

    #[test]
    fn durations_read_in_the_unit_that_fits() {
        assert_eq!(format_duration(Duration::from_micros(250)), "0.25 ms");
        assert_eq!(format_duration(Duration::from_millis(12)), "12 ms");
        assert_eq!(format_duration(Duration::from_millis(1250)), "1.25 s");
    }

    #[test]
    fn responses_cross_the_bridge_as_json() {
        let resp = HttpResponse {
            id: "r".into(),
            latency: Duration::from_micros(1500),
            stats: HttpStats {
                dns: Duration::from_micros(10),
                ..Default::default()
            },
            ..response(&[("a", "b")], b"\xff")
        };
        let json = serde_json::to_string(&resp).expect("json");
        assert!(json.contains("\"latency\":1500"), "{json}");
        assert_eq!(serde_json::from_str::<HttpResponse>(&json).expect("read"), resp);
    }
}
