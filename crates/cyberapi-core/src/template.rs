//! Template functions (M6): `{{fn(param)}}` and chains like
//! `{{b64.rf(a.png)}}` in a body, a query value or a header value.
//!
//! Parsing and the pure functions live here. What a function needs from the
//! world — a file, the reader's choice of a file, another API's last
//! response, a custom variable, the time — is gathered first by the caller
//! ([`needs`] says what), handed in as [`Inputs`] and a [`FileReader`], and
//! evaluation is then a pure function of them.
//!
//! Rules kept from the old app (`src/commands/fn.ts`): a chain runs right to
//! left, each function taking the one before's result; parameters split on
//! `,`, trimmed of spaces and quotes, one parameter is a string and several
//! a list; each result replaces the first occurrence of its expression; an
//! unknown function is an error: the old app passed its input through, so
//! `{{uuid()}}` went out as an empty value and nothing said why (#48). One
//! other rule is not kept: an expression without parentheses (`{{name}}`)
//! used to stop every expression after it from being read; it is now left
//! as it is and the rest still run.
//!
//! Functions the old app did not have (#67): `uuid`, `random`,
//! `randomString`, `now`, `hmacSha256`, `urlEncode`. Their randomness and
//! the local time's offset come in [`Inputs`] too.

use base64::Engine as _;
use chrono::format::{Item, StrftimeItems};
use hmac::{KeyInit as _, Mac as _};
use md5::Digest as _;
use serde_json::Value as Json;
use snafu::Snafu;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// One `{{…}}` with a parameter list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    /// The whole expression as written, braces included: what is replaced.
    pub text: String,
    /// The functions, in writing order (they run from the last).
    pub chain: Vec<String>,
    pub params: Vec<String>,
}

/// The base directory a file name is read from (`readFile(name, dir)`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BaseDir {
    #[default]
    Download,
    Document,
    Desktop,
}

impl BaseDir {
    /// `document` and `desktop`, in any case; anything else is downloads.
    pub fn parse_lenient(text: &str) -> Self {
        match text.to_ascii_lowercase().as_str() {
            "document" => Self::Document,
            "desktop" => Self::Desktop,
            _ => Self::Download,
        }
    }
}

#[derive(Debug, Snafu, PartialEq, Eq)]
pub enum TemplateError {
    /// `get` takes an API id and a path.
    #[snafu(display("get needs an API id and a path: {text}"))]
    GetParams { text: String },
    /// No such template function (#48).
    #[snafu(display("unknown function {name}"))]
    Unknown { name: String },
    #[snafu(display("{file} could not be read: {message}"))]
    ReadFile { file: String, message: String },
    /// A function given what it cannot take: `random(a)`.
    #[snafu(display("{name}({text}): {expected}"))]
    Params {
        name: String,
        text: String,
        expected: &'static str,
    },
}

/// Reads a file for `readFile` / `readTextFile`, and a multipart field's
/// `file://` value. The browser build has none to read.
pub trait FileReader {
    fn read(&self, file: &str, dir: Option<BaseDir>) -> Result<Vec<u8>, String>;
}

/// No files: every read fails with `message`.
pub struct NoFiles(pub &'static str);

impl FileReader for NoFiles {
    fn read(&self, _: &str, _: Option<BaseDir>) -> Result<Vec<u8>, String> {
        Err(self.0.to_string())
    }
}

/// What the expressions of some text need from the world.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    /// The APIs whose last response `get` reads.
    pub responses: Vec<String>,
    /// `openFile` asks the reader for a file.
    pub picks_file: bool,
}

impl Needs {
    pub fn add(&mut self, other: Needs) {
        for api in other.responses {
            if !self.responses.contains(&api) {
                self.responses.push(api);
            }
        }
        self.picks_file |= other.picks_file;
    }
}

/// What the caller gathered for [`render`].
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    /// Unix seconds, for `timestamp`.
    pub now: i64,
    /// The project's enabled custom variables, in store order (`value`).
    pub custom: Vec<(String, String)>,
    /// The last response's JSON of each API [`Needs::responses`] named that
    /// has one.
    pub responses: HashMap<String, Json>,
    /// The file the reader picked for `openFile`, if they picked one.
    pub picked_file: Option<String>,
    /// The local time's offset from UTC, in seconds, for `now`.
    pub utc_offset: i32,
    /// What `uuid`, `random` and `randomString` draw from.
    pub draws: Draws,
}

/// Random numbers from a seed the caller picks at random for each send:
/// each draw is new, across every text of a request, and a fixed seed (a
/// test's) gives the same ones each time.
#[derive(Clone, Debug, Default)]
pub struct Draws {
    seed: u64,
    taken: Arc<AtomicU64>,
}

impl Draws {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            taken: Arc::default(),
        }
    }

    /// SplitMix64 of the seed and how many draws came before.
    fn next(&self) -> u64 {
        let taken = self.taken.fetch_add(1, Ordering::Relaxed);
        let mut z = self
            .seed
            .wrapping_add(taken.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A whole number from `min` to `max`, both included.
    fn between(&self, min: i64, max: i64) -> i64 {
        let span = (i128::from(max) - i128::from(min) + 1) as u128;
        (i128::from(min) + (u128::from(self.next()) % span) as i128) as i64
    }
}

/// Every expression in `text`, in order.
pub fn parse(text: &str) -> Vec<Expr> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        // Lazy, like the old `\{\{([\s\S]+?)\}\}`: the first `}}` that leaves
        // at least one character inside.
        let Some(end) = after.get(1..).and_then(|tail| tail.find("}}")).map(|at| at + 1) else {
            break;
        };
        let inner = &after[..end];
        let whole = &rest[start..start + 2 + end + 2];
        if let Some(expr) = parse_inner(whole, inner) {
            found.push(expr);
        }
        rest = &rest[start + 2 + end + 2..];
    }
    found
}

fn parse_inner(whole: &str, inner: &str) -> Option<Expr> {
    let open = inner.find('(')?;
    let close = open + 1 + inner[open + 1..].find(')')?;
    let names = format!("{}{}", &inner[..open], &inner[close + 1..]);
    Some(Expr {
        text: whole.to_string(),
        chain: names.split('.').map(str::to_string).collect(),
        params: split_params(&inner[open + 1..close]),
    })
}

fn split_params(text: &str) -> Vec<String> {
    text.split(',')
        .map(|param| param.trim().trim_matches('\'').trim_matches('"').to_string())
        .collect()
}

/// What the expressions in `text` need gathered before [`render`].
pub fn needs(text: &str) -> Needs {
    let mut needs = Needs::default();
    for expr in parse(text) {
        for name in &expr.chain {
            match name.as_str() {
                "get" | "g" => {
                    if let Some(api) = expr.params.first().filter(|_| expr.params.len() == 2)
                        && !needs.responses.contains(api)
                    {
                        needs.responses.push(api.clone());
                    }
                }
                "openFile" | "of" => needs.picks_file = true,
                _ => {}
            }
        }
    }
    needs
}

/// What flows along a chain.
#[derive(Clone, Debug, PartialEq)]
enum Value {
    Text(String),
    List(Vec<String>),
    Bytes(Vec<u8>),
    Json(Json),
}

impl Value {
    fn from_params(params: &[String]) -> Self {
        match params {
            [one] => Self::Text(one.clone()),
            many => Self::List(many.to_vec()),
        }
    }

    /// As lodash's `toString` would print it, but JSON for objects.
    fn into_text(self) -> String {
        match self {
            Self::Text(text) => text,
            Self::List(items) => items.join(","),
            Self::Bytes(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Self::Json(Json::String(text)) => text,
            Self::Json(Json::Null) => String::new(),
            Self::Json(json) => json.to_string(),
        }
    }

    fn into_bytes(self) -> Vec<u8> {
        match self {
            Self::Bytes(bytes) => bytes,
            other => other.into_text().into_bytes(),
        }
    }

    /// The file and directory of `readFile(file, dir?)`.
    fn file_params(&self) -> (String, Option<BaseDir>) {
        match self {
            Self::List(items) => (
                items.first().cloned().unwrap_or_default(),
                items
                    .get(1)
                    .filter(|dir| !dir.is_empty())
                    .map(|dir| BaseDir::parse_lenient(dir)),
            ),
            other => (other.clone().into_text(), None),
        }
    }
}

/// Runs one expression.
pub fn evaluate(expr: &Expr, inputs: &Inputs, files: &dyn FileReader) -> Result<String, TemplateError> {
    let mut value = Value::from_params(&expr.params);
    for name in expr.chain.iter().rev() {
        value = match name.as_str() {
            "readTextFile" | "rtf" => Value::Text(String::from_utf8_lossy(&read(&value, files)?).into_owned()),
            "readFile" | "rf" => Value::Bytes(read(&value, files)?),
            "base64" | "b64" => Value::Text(base64::engine::general_purpose::STANDARD.encode(value.into_bytes())),
            "md5" => Value::Text(hex(&md5::Md5::digest(value.into_text().as_bytes()))),
            "sha256" => Value::Text(hex(&sha2::Sha256::digest(value.into_text().as_bytes()))),
            "openFile" | "of" => match &inputs.picked_file {
                Some(file) => Value::Text(file.clone()),
                None => value,
            },
            "timestamp" | "ts" => Value::Text(inputs.now.to_string()),
            "uuid" => Value::Text(uuid(&inputs.draws)),
            "random" => Value::Text(random(&value, &inputs.draws)?.to_string()),
            "randomString" => Value::Text(random_string(&value, &inputs.draws)?),
            "now" => Value::Text(now(&value, inputs)?),
            "hmacSha256" => Value::Text(hmac_sha256(&value)?),
            "urlEncode" => Value::Text(crate::send::encode_uri_component(&value.into_text())),
            "get" | "g" => {
                let text = value.into_text();
                let [api, path] = text.split(',').collect::<Vec<_>>()[..] else {
                    return GetParamsSnafu { text }.fail();
                };
                match inputs.responses.get(api.trim()) {
                    Some(json) => Value::Json(json_path(json, path.trim()).cloned().unwrap_or(Json::Null)),
                    None => Value::Text(text),
                }
            }
            "value" => {
                let name = value.clone().into_text();
                match inputs.custom.iter().find(|(custom, _)| *custom == name) {
                    Some((_, found)) => Value::Text(found.clone()),
                    None => value,
                }
            }
            _ => return UnknownSnafu { name: name.clone() }.fail(),
        };
    }
    Ok(value.into_text())
}

/// A chain's parameters, one by one: none for `f()`.
fn params(value: &Value) -> Vec<String> {
    match value {
        Value::List(items) => items.clone(),
        Value::Text(text) if text.is_empty() => Vec::new(),
        other => vec![other.clone().into_text()],
    }
}

fn params_error(name: &str, value: &Value, expected: &'static str) -> TemplateError {
    TemplateError::Params {
        name: name.to_string(),
        text: params(value).join(", "),
        expected,
    }
}

/// A version 4 UUID.
fn uuid(draws: &Draws) -> String {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&draws.next().to_be_bytes());
    bytes[8..].copy_from_slice(&draws.next().to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// `random()` is 0 to 100, `random(max)` 0 to `max`, `random(min, max)`
/// `min` to `max`, both ends included.
fn random(value: &Value, draws: &Draws) -> Result<i64, TemplateError> {
    let numbers = params(value)
        .iter()
        .map(|param| param.parse::<i64>())
        .collect::<Result<Vec<_>, _>>();
    let (min, max) = match numbers.as_deref() {
        Ok([]) => (0, 100),
        Ok([max]) => (0, *max),
        Ok([min, max]) => (*min, *max),
        _ => (1, 0),
    };
    if min > max {
        return Err(params_error("random", value, "whole numbers, min then max"));
    }
    Ok(draws.between(min, max))
}

const RANDOM_STRING: usize = 16;
const RANDOM_STRING_LIMIT: usize = 1024;

/// Letters and digits: 16 of them, or as many as asked, up to 1024.
fn random_string(value: &Value, draws: &Draws) -> Result<String, TemplateError> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let length = match params(value).as_slice() {
        [] => RANDOM_STRING,
        [length] => length
            .parse::<usize>()
            .ok()
            .filter(|length| *length <= RANDOM_STRING_LIMIT)
            .ok_or_else(|| params_error("randomString", value, "a length up to 1024"))?,
        _ => return Err(params_error("randomString", value, "a length up to 1024")),
    };
    Ok((0..length)
        .map(|_| char::from(ALPHABET[(draws.next() % ALPHABET.len() as u64) as usize]))
        .collect())
}

/// The time now: ISO 8601 (`2026-10-25T14:30:00+08:00`), or in a strftime
/// format (`%Y-%m-%d %H:%M:%S`); local, or UTC when the last parameter is
/// `utc`. `now(utc)` is ISO 8601 in UTC.
fn now(value: &Value, inputs: &Inputs) -> Result<String, TemplateError> {
    let mut params = params(value);
    let utc = params.last().is_some_and(|last| last.eq_ignore_ascii_case("utc"));
    if utc {
        params.pop();
    }
    let format = match params.as_slice() {
        [] => "",
        [format] => format.as_str(),
        _ => return Err(params_error("now", value, "a format, then utc or nothing")),
    };
    let offset = if utc { 0 } else { inputs.utc_offset };
    let at = chrono::DateTime::from_timestamp(inputs.now, 0)
        .unwrap_or_default()
        .with_timezone(
            &chrono::FixedOffset::east_opt(offset).unwrap_or(chrono::FixedOffset::east_opt(0).expect("UTC")),
        );
    if format.is_empty() || format.eq_ignore_ascii_case("iso") {
        return Ok(at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    let items: Vec<Item> = StrftimeItems::new(format).collect();
    if items.contains(&Item::Error) {
        return Err(params_error("now", value, "a strftime format such as %Y-%m-%d"));
    }
    Ok(at.format_with_items(items.into_iter()).to_string())
}

/// HMAC-SHA256 of the data under the key, in hex. The data is everything
/// after the first parameter, commas and all.
fn hmac_sha256(value: &Value) -> Result<String, TemplateError> {
    let params = params(value);
    let [key, data @ ..] = params.as_slice() else {
        return Err(params_error("hmacSha256", value, "a key, then the data"));
    };
    if data.is_empty() {
        return Err(params_error("hmacSha256", value, "a key, then the data"));
    }
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(key.as_bytes()).expect("HMAC takes a key of any length");
    mac.update(data.join(",").as_bytes());
    Ok(hex(&mac.finalize().into_bytes()))
}

fn read(value: &Value, files: &dyn FileReader) -> Result<Vec<u8>, TemplateError> {
    let (file, dir) = value.file_params();
    files
        .read(&file, Some(dir.unwrap_or_default()))
        .map_err(|message| TemplateError::ReadFile { file, message })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// lodash `get`: `data.0.name`, and `data[0].name`.
pub(crate) fn json_path<'a>(json: &'a Json, path: &str) -> Option<&'a Json> {
    path.replace('[', ".")
        .replace(']', "")
        .split('.')
        .filter(|key| !key.is_empty())
        .try_fold(json, |at, key| match at {
            Json::Object(map) => map.get(key),
            Json::Array(items) => key.parse::<usize>().ok().and_then(|index| items.get(index)),
            _ => None,
        })
}

/// `text` with each expression replaced by its result.
pub fn render(text: &str, inputs: &Inputs, files: &dyn FileReader) -> Result<String, TemplateError> {
    let mut out = text.to_string();
    for expr in parse(text) {
        let result = evaluate(&expr, inputs, files)?;
        out = out.replacen(&expr.text, &result, 1);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    struct Files;

    impl FileReader for Files {
        fn read(&self, file: &str, dir: Option<BaseDir>) -> Result<Vec<u8>, String> {
            match (file, dir) {
                ("a.txt", Some(BaseDir::Download)) => Ok(b"hello".to_vec()),
                ("a.txt", Some(BaseDir::Desktop)) => Ok(b"desk".to_vec()),
                _ => Err("no such file".into()),
            }
        }
    }

    fn inputs() -> Inputs {
        Inputs {
            now: 1_700_000_000,
            custom: vec![("token".into(), "t-1".into())],
            responses: HashMap::from([(
                "api-1".to_string(),
                serde_json::json!({"data": [{"name": "tree", "tags": {"a": 1}}]}),
            )]),
            picked_file: Some("/tmp/x.png".into()),
            utc_offset: 8 * 3600,
            draws: Draws::new(7),
        }
    }

    fn run(text: &str) -> Result<String, TemplateError> {
        render(text, &inputs(), &Files)
    }

    #[test]
    fn parses_chains_and_params() {
        assert_eq!(
            parse(r#"a {{b64.rf("a.txt", 'desktop')}} b {{ts()}}"#),
            [
                Expr {
                    text: r#"{{b64.rf("a.txt", 'desktop')}}"#.into(),
                    chain: vec!["b64".into(), "rf".into()],
                    params: vec!["a.txt".into(), "desktop".into()],
                },
                Expr {
                    text: "{{ts()}}".into(),
                    chain: vec!["ts".into()],
                    params: vec!["".into()],
                },
            ]
        );
    }

    #[test]
    fn functions_run_right_to_left() {
        assert_eq!(run("{{rtf(a.txt)}}").as_deref(), Ok("hello"));
        assert_eq!(run("{{rtf(a.txt, Desktop)}}").as_deref(), Ok("desk"));
        assert_eq!(run("{{b64.rf(a.txt)}}").as_deref(), Ok("aGVsbG8="));
        assert_eq!(run("{{md5(abc)}}").as_deref(), Ok("900150983cd24fb0d6963f7d28e17f72"));
        assert_eq!(
            run("{{sha256(abc)}}").as_deref(),
            Ok("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        assert_eq!(
            run("{{md5.value(token)}}").as_deref(),
            Ok("f3e90ce87a25538f5a4be79a0a7c0fa5")
        );
        assert_eq!(run("t={{ts()}}").as_deref(), Ok("t=1700000000"));
        assert_eq!(run("{{of()}}").as_deref(), Ok("/tmp/x.png"));
    }

    #[test]
    fn get_reads_another_apis_last_response() {
        assert_eq!(run("{{get(api-1, data.0.name)}}").as_deref(), Ok("tree"));
        assert_eq!(run("{{g(api-1, data[0].tags)}}").as_deref(), Ok(r#"{"a":1}"#));
        assert_eq!(run("{{get(api-1, data.9)}}").as_deref(), Ok(""));
        // No response yet: the parameters pass through, as they did.
        assert_eq!(run("{{get(api-2, data)}}").as_deref(), Ok("api-2,data"));
        assert_eq!(
            run("{{get(api-1)}}"),
            Err(TemplateError::GetParams { text: "api-1".into() })
        );
    }

    #[test]
    fn an_unknown_function_is_an_error_and_each_text_is_replaced_once() {
        assert_eq!(run("{{nope(x)}}"), Err(TemplateError::Unknown { name: "nope".into() }));
        assert_eq!(run("{{uuid1()}}"), Err(TemplateError::Unknown { name: "uuid1".into() }));
        // A known function whose variable is missing still passes the name
        // through: that is "not found", not "not a function".
        assert_eq!(run("{{value(missing)}}").as_deref(), Ok("missing"));
        assert_eq!(run("{{ts()}} {{ts()}}").as_deref(), Ok("1700000000 1700000000"));
    }

    #[test]
    fn uuids_and_random_values_differ_each_time() {
        let uuids = render("{{uuid()}} {{uuid()}}", &inputs(), &Files).expect("render");
        let (a, b) = uuids.split_once(' ').expect("two");
        assert_ne!(a, b);
        for uuid in [a, b] {
            let parts: Vec<&str> = uuid.split('-').collect();
            assert_eq!(
                parts.iter().map(|part| part.len()).collect::<Vec<_>>(),
                [8, 4, 4, 4, 12]
            );
            assert!(parts[2].starts_with('4'), "version 4: {uuid}");
            assert!("89ab".contains(&parts[3][..1]), "the RFC 4122 variant: {uuid}");
        }
        // The same seed, the same draws.
        assert_eq!(
            render("{{uuid()}}", &inputs(), &Files),
            render("{{uuid()}}", &inputs(), &Files)
        );

        for _ in 0..200 {
            let n: i64 = run("{{random(-2, 2)}}").expect("random").parse().expect("a number");
            assert!((-2..=2).contains(&n));
            let n: i64 = run("{{random(3)}}").expect("random").parse().expect("a number");
            assert!((0..=3).contains(&n));
        }
        assert_eq!(run("{{random(5, 5)}}").as_deref(), Ok("5"));
        assert!(run("{{random()}}").is_ok());
        assert_eq!(
            run("{{random(9, 1)}}"),
            Err(TemplateError::Params {
                name: "random".into(),
                text: "9, 1".into(),
                expected: "whole numbers, min then max",
            })
        );
        assert!(run("{{random(a)}}").is_err());

        let text = run("{{randomString()}}").expect("random string");
        assert_eq!(text.len(), 16);
        assert!(text.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(run("{{randomString(40)}}").expect("random string").len(), 40);
        assert_eq!(run("{{randomString(0)}}").as_deref(), Ok(""));
        assert!(run("{{randomString(5000)}}").is_err());
    }

    #[test]
    fn now_is_iso_8601_or_formatted_local_or_utc() {
        // 1_700_000_000 is 2023-11-14 22:13:20 UTC; the inputs are UTC+8.
        assert_eq!(run("{{now()}}").as_deref(), Ok("2023-11-15T06:13:20+08:00"));
        assert_eq!(run("{{now(utc)}}").as_deref(), Ok("2023-11-14T22:13:20Z"));
        assert_eq!(run("{{now(%Y-%m-%d %H:%M:%S)}}").as_deref(), Ok("2023-11-15 06:13:20"));
        assert_eq!(run("{{now('%Y%m%d', UTC)}}").as_deref(), Ok("20231114"));
        assert_eq!(run("{{now(iso, utc)}}").as_deref(), Ok("2023-11-14T22:13:20Z"));
        assert!(run("{{now(%Q)}}").is_err(), "not a format: an error, not a panic");
        assert!(run("{{now(a, b, c)}}").is_err());
    }

    #[test]
    fn hmac_and_url_encoding() {
        // RFC 4231, test case 2.
        assert_eq!(
            run("{{hmacSha256(Jefe, what do ya want for nothing?)}}").as_deref(),
            Ok("5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843")
        );
        // The data keeps its commas.
        let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(b"k").expect("key");
        mac.update(b"a,b");
        assert_eq!(run("{{hmacSha256(k, a,b)}}"), Ok(hex(&mac.finalize().into_bytes())));
        assert!(run("{{hmacSha256(only-a-key)}}").is_err());
        assert_eq!(run("{{urlEncode(a b&c=d/é)}}").as_deref(), Ok("a%20b%26c%3Dd%2F%C3%A9"));
        assert_eq!(run("{{urlEncode.value(token)}}").as_deref(), Ok("t-1"));
    }

    #[test]
    fn plain_braces_are_left_and_do_not_stop_the_rest() {
        assert_eq!(run("{{HOST}} {{value(token)}}").as_deref(), Ok("{{HOST}} t-1"));
        assert_eq!(run("{\"a\": {{}}}").as_deref(), Ok("{\"a\": {{}}}"));
    }

    #[test]
    fn a_failed_read_names_the_file() {
        assert_eq!(
            run("{{rf(b.txt)}}"),
            Err(TemplateError::ReadFile {
                file: "b.txt".into(),
                message: "no such file".into()
            })
        );
    }

    #[test]
    fn needs_lists_responses_and_the_file_picker() {
        let text = "{{get(api-1, a)}} {{g(api-1, b)}} {{b64.rf.of()}} {{get(api-3, c)}}";
        assert_eq!(
            needs(text),
            Needs {
                responses: vec!["api-1".into(), "api-3".into()],
                picks_file: true,
            }
        );
    }
}
