//! G2: a project answering as a mock server would, from its APIs'
//! examples (F3). A request is matched to an API by its method and its
//! path — the API's address without its prefix variable, scheme, host and
//! query; a segment written `{{id}}`, `:id` or `{id}` matches any — the
//! more fixed segments the better. The API answers with the example the
//! `x-mock-example` header names; else the first whose conditions (M1:
//! query, header, or a JSONPath into a JSON body) all hold; else its first
//! example without conditions. An example may wait before answering (M2)
//! and send an event stream one event at a time (M3). Every answer lets
//! any origin read it, and a CORS preflight is answered, so a page being
//! built can call it.

use crate::ItemId;
use crate::model::{Example, HttpMethod, MockCondition, MockSource, split_env_prefix};
use crate::tree::TreeData;
use serde_json::{Value, json};
use std::cell::OnceCell;

/// The header that picks an example by its name.
pub const EXAMPLE_HEADER: &str = "x-mock-example";
/// The header an answer names its example in, percent-encoded (M5).
pub const USED_HEADER: &str = "x-mock-example-used";
/// The most of a request body a server reads for the conditions (M1).
pub const BODY_READ_LIMIT: usize = 1024 * 1024;

/// What answers where.
#[derive(Clone, Debug, Default)]
pub struct Routes(Vec<Route>);

#[derive(Clone, Debug)]
struct Route {
    api: ItemId,
    name: String,
    method: HttpMethod,
    /// `None` for one that matches any segment.
    segments: Vec<Option<String>>,
    examples: Vec<Example>,
}

/// A request to the mock, as its server read it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Asked<'a> {
    pub method: &'a str,
    /// The path, its query left on or not.
    pub path: &'a str,
    pub headers: &'a [(String, String)],
    /// Up to [`BODY_READ_LIMIT`] of it.
    pub body: &'a [u8],
}

impl Asked<'_> {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(asked, _)| asked.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// How a request is answered: what to send, and for the log, which API.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The API that answered, by name.
    pub api: Option<String>,
    /// The example it answered with, by name (M5).
    pub example: Option<String>,
    /// How long to wait before answering (M2), at most
    /// [`Example::MAX_DELAY_MS`].
    pub delay_ms: u32,
    /// Above 0, send [`Answer::events`] one by one with this pause between
    /// them (M3); only for an event stream.
    pub stream_interval_ms: u32,
}

impl Answer {
    /// The body cut into its events, each with the blank line that ends
    /// it; what follows the last blank line is one more.
    pub fn events(&self) -> Vec<Vec<u8>> {
        let mut events = Vec::new();
        let mut rest = self.body.as_slice();
        while !rest.is_empty() {
            let end = [b"\n\n".as_slice(), b"\r\n\r\n".as_slice()]
                .iter()
                .filter_map(|blank| {
                    rest.windows(blank.len())
                        .position(|window| window == *blank)
                        .map(|at| at + blank.len())
                })
                .min()
                .unwrap_or(rest.len());
            events.push(rest[..end].to_vec());
            rest = &rest[end..];
        }
        events
    }
}

/// A request the server answered, for its log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heard {
    /// Milliseconds since the epoch.
    pub at: i64,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub api: Option<String>,
    /// The example it answered with (M5).
    pub example: Option<String>,
}

impl Heard {
    /// `asked`, answered with `answer`, at `at`.
    pub fn of(asked: &Asked, answer: &Answer, at: i64) -> Self {
        Self {
            at,
            method: asked.method.to_string(),
            path: asked.path.to_string(),
            status: answer.status,
            api: answer.api.clone(),
            example: answer.example.clone(),
        }
    }
}

/// The path an API's address asks for, without its prefix variable,
/// scheme, host, query and fragment.
fn path_of(uri: &str) -> String {
    let (_, rest) = split_env_prefix(uri.trim());
    let rest = match rest.split_once("://") {
        Some((_, after)) => after.find('/').map_or("/", |at| &after[at..]),
        None => rest,
    };
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    format!("/{}", rest.trim_start_matches('/'))
}

fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|segment| !segment.is_empty()).collect()
}

fn wildcard(segment: &str) -> bool {
    segment.contains("{{") || segment.starts_with(':') || (segment.starts_with('{') && segment.ends_with('}'))
}

/// M1: conditions as their editor writes them, one per line:
/// `query user = 7`, `header X-Tier`, `body $.role = admin`.
pub fn conditions_text(when: &[MockCondition]) -> String {
    when.iter()
        .map(|condition| {
            let source = match condition.source {
                MockSource::Query => "query",
                MockSource::Header => "header",
                MockSource::Body => "body",
            };
            match condition.value.is_empty() {
                true => format!("{source} {}", condition.name),
                false => format!("{source} {} = {}", condition.name, condition.value),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// [`conditions_text`] read back; blank lines and `#` comments skipped. A
/// value follows ` = `; for a query or header a bare `=` will do, while a
/// JSONPath keeps its own `==`. `Err` is the first line (from 1) that does
/// not start with `query`, `header` or `body` and a name.
pub fn conditions_from_text(text: &str) -> Result<Vec<MockCondition>, usize> {
    let mut when = Vec::new();
    for (at, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (source, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let source = match source.to_ascii_lowercase().as_str() {
            "query" => MockSource::Query,
            "header" => MockSource::Header,
            "body" => MockSource::Body,
            _ => return Err(at + 1),
        };
        let (name, value) = match rest.split_once(" = ") {
            Some(split) => split,
            None if source != MockSource::Body => rest.split_once('=').unwrap_or((rest, "")),
            None => (rest, ""),
        };
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() {
            return Err(at + 1);
        }
        when.push(MockCondition {
            source,
            name: name.to_string(),
            value: value.to_string(),
        });
    }
    Ok(when)
}

/// An example's conditions; a row without a name is not one yet.
fn conditions(example: &Example) -> impl Iterator<Item = &MockCondition> {
    example
        .when
        .iter()
        .filter(|condition| !condition.name.trim().is_empty())
}

/// Whether `asked` meets `condition`; `json`, its body read as JSON once.
fn holds(condition: &MockCondition, asked: &Asked, json: &OnceCell<Option<Value>>) -> bool {
    let (name, wanted) = (condition.name.trim(), condition.value.trim());
    let equal = |found: &str| wanted.is_empty() || found.trim() == wanted;
    match condition.source {
        MockSource::Query => {
            let query = asked.path.split_once('?').map_or("", |(_, query)| query);
            let query = query.split('#').next().unwrap_or_default();
            url::form_urlencoded::parse(query.as_bytes()).any(|(key, value)| key == name && equal(&value))
        }
        MockSource::Header => asked
            .headers
            .iter()
            .any(|(key, value)| key.eq_ignore_ascii_case(name) && equal(value)),
        MockSource::Body => {
            let Some(doc) = json.get_or_init(|| serde_json::from_slice(asked.body).ok()) else {
                return false;
            };
            let Ok(Value::Array(found)) = crate::json_filter::filter(doc, name) else {
                return false;
            };
            found.iter().any(|value| match value {
                Value::String(text) => equal(text),
                other => equal(&other.to_string()),
            })
        }
    }
}

/// `asked`'s example of `examples`, by the rule the module says.
fn pick<'a>(examples: &'a [Example], asked: &Asked) -> Option<&'a Example> {
    let json = OnceCell::new();
    examples
        .iter()
        .find(|example| {
            let mut all = conditions(example).peekable();
            all.peek().is_some() && all.all(|condition| holds(condition, asked, &json))
        })
        .or_else(|| examples.iter().find(|example| conditions(example).next().is_none()))
}

impl Routes {
    /// The project's HTTP APIs; a gRPC call or a WebSocket answers none.
    pub fn of(data: &TreeData) -> Self {
        Self(
            data.apis
                .iter()
                .filter(|api| !matches!(api.request.method, HttpMethod::Grpc | HttpMethod::WebSocket))
                .map(|api| Route {
                    api: api.id.clone(),
                    name: api.name.clone(),
                    method: api.request.method,
                    segments: segments(&path_of(&api.request.uri))
                        .into_iter()
                        .map(|segment| (!wildcard(segment)).then(|| segment.to_string()))
                        .collect(),
                    examples: api.request.examples.clone(),
                })
                .collect(),
        )
    }

    /// How many APIs answer, and how many of them with an example.
    pub fn count(&self) -> (usize, usize) {
        let with = self.0.iter().filter(|route| !route.examples.is_empty()).count();
        (self.0.len(), with)
    }

    /// The answer to `asked`.
    pub fn answer(&self, asked: &Asked) -> Answer {
        let method = asked.method;
        let cors = |mut headers: Vec<(String, String)>| {
            headers.push(("Access-Control-Allow-Origin".into(), "*".into()));
            headers.push(("Access-Control-Expose-Headers".into(), "*".into()));
            headers
        };
        if method.eq_ignore_ascii_case("OPTIONS") && asked.header("access-control-request-method").is_some() {
            let mut headers = vec![
                (
                    "Access-Control-Allow-Methods".into(),
                    "GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS".into(),
                ),
                ("Access-Control-Max-Age".into(), "600".into()),
            ];
            if let Some(wanted) = asked
                .header("access-control-request-headers")
                .filter(|wanted| !wanted.is_empty())
            {
                headers.push(("Access-Control-Allow-Headers".into(), wanted.to_string()));
            }
            return Answer {
                status: 204,
                headers: cors(headers),
                ..Default::default()
            };
        }
        let path = asked.path.split(['?', '#']).next().unwrap_or_default();
        let found: Vec<&Route> = self.0.iter().filter(|route| route.matches(path)).collect();
        let json = |status: u16, error: String, api: Option<String>| Answer {
            status,
            headers: cors(vec![("Content-Type".into(), "application/json".into())]),
            body: crate::format::json(&json!({ "error": error })).into_bytes(),
            api,
            ..Default::default()
        };
        let Some(route) = found
            .iter()
            .filter(|route| route.method.as_str().eq_ignore_ascii_case(method))
            .max_by_key(|route| route.fixed())
        else {
            if found.is_empty() {
                return json(404, format!("no API of the project answers {method} {path}"), None);
            }
            let mut allowed: Vec<&str> = found.iter().map(|route| route.method.as_str()).collect();
            allowed.dedup();
            let mut answer = json(405, format!("{path} is not answered to {method}"), None);
            answer.headers.push(("Allow".into(), allowed.join(", ")));
            return answer;
        };
        let wanted = asked
            .header(EXAMPLE_HEADER)
            .map(str::trim)
            .filter(|name| !name.is_empty());
        let example = match wanted {
            Some(name) => route
                .examples
                .iter()
                .find(|example| example.name.eq_ignore_ascii_case(name)),
            None => pick(&route.examples, asked),
        };
        let Some(example) = example else {
            let why = match wanted {
                Some(name) => format!("{} has no example named {name:?}", route.name),
                None if route.examples.is_empty() => format!(
                    "{} has no example to answer with: keep one from its history",
                    route.name
                ),
                None => format!(
                    "{} has no example whose conditions this request meets, and none without conditions",
                    route.name
                ),
            };
            return json(404, why, Some(route.name.clone()));
        };
        // What the server sent about its body as it came, when, and who it
        // was, is not this one's.
        const DROPPED: [&str; 7] = [
            "content-length",
            "content-encoding",
            "transfer-encoding",
            "connection",
            "keep-alive",
            "date",
            "server",
        ];
        let mut headers: Vec<(String, String)> = example
            .headers
            .iter()
            .filter(|(name, _)| !DROPPED.contains(&name.to_ascii_lowercase().as_str()))
            .filter(|(name, _)| !name.to_ascii_lowercase().starts_with("access-control-"))
            .cloned()
            .collect();
        if example.content_type().is_none() {
            let kind = match serde_json::from_str::<Value>(&example.body) {
                Ok(_) => "application/json",
                Err(_) => "text/plain; charset=utf-8",
            };
            headers.push(("Content-Type".into(), kind.into()));
        }
        let streams = example
            .content_type()
            .is_some_and(|kind| kind.to_ascii_lowercase().starts_with("text/event-stream"));
        headers.push((
            USED_HEADER.into(),
            percent_encoding::utf8_percent_encode(&example.name, percent_encoding::CONTROLS).to_string(),
        ));
        Answer {
            status: example.status,
            headers: cors(headers),
            body: example.body.clone().into_bytes(),
            api: Some(route.name.clone()),
            example: Some(example.name.clone()),
            delay_ms: example.delay_ms.min(Example::MAX_DELAY_MS),
            stream_interval_ms: if streams {
                example.stream_interval_ms.min(Example::MAX_STREAM_INTERVAL_MS)
            } else {
                0
            },
        }
    }

    /// The API that answers, for tests and the log.
    pub fn api_of(&self, method: &str, path: &str) -> Option<&ItemId> {
        self.0
            .iter()
            .filter(|route| route.method.as_str().eq_ignore_ascii_case(method))
            .filter(|route| route.matches(path))
            .max_by_key(|route| route.fixed())
            .map(|route| &route.api)
    }
}

impl Route {
    /// Whether it answers `path` (no query), whatever the method.
    fn matches(&self, path: &str) -> bool {
        let asked = segments(path);
        self.segments.len() == asked.len()
            && self
                .segments
                .iter()
                .zip(&asked)
                .all(|(segment, asked)| segment.as_deref().is_none_or(|fixed| fixed == *asked))
    }

    /// How many of its segments are fixed: the more, the better a match.
    fn fixed(&self) -> usize {
        self.segments.iter().filter(|segment| segment.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ApiSetting, HttpRequest};
    use pretty_assertions::assert_eq;

    fn api(id: &str, method: HttpMethod, uri: &str, examples: Vec<Example>) -> ApiSetting {
        ApiSetting {
            id: ItemId::new(id),
            name: id.into(),
            request: HttpRequest {
                method,
                uri: uri.into(),
                examples,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn example(name: &str, status: u16, body: &str) -> Example {
        Example {
            name: name.into(),
            status,
            headers: vec![
                ("content-length".into(), "999".into()),
                ("Date".into(), "Tue, 27 Oct 2026 14:07:06 GMT".into()),
                ("x-trace".into(), "t".into()),
            ],
            body: body.into(),
            ..Default::default()
        }
    }

    /// `routes`' answer to `method` `path` with `headers` and `body`.
    fn ask(routes: &Routes, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Answer {
        let headers: Vec<(String, String)> = headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        routes.answer(&Asked {
            method,
            path,
            headers: &headers,
            body: body.as_bytes(),
        })
    }

    fn routes() -> Routes {
        Routes::of(&TreeData {
            apis: vec![
                api(
                    "user",
                    HttpMethod::Get,
                    "{{HOST}}/users/{{value(id)}}",
                    vec![example("ok", 200, r#"{"id":1}"#), example("missing", 404, "no")],
                ),
                api(
                    "me",
                    HttpMethod::Get,
                    "https://api.test/users/me?x=1",
                    vec![example("me", 200, "me")],
                ),
                api("make", HttpMethod::Post, "/users", Vec::new()),
                api("call", HttpMethod::Grpc, "localhost:50051", Vec::new()),
            ],
            ..Default::default()
        })
    }

    #[test]
    fn a_request_finds_its_api_by_method_and_path() {
        let routes = routes();
        assert_eq!(routes.count(), (3, 2), "a gRPC call answers none");
        assert_eq!(routes.api_of("GET", "/users/7"), Some(&ItemId::new("user")));
        assert_eq!(
            routes.api_of("GET", "/users/me"),
            Some(&ItemId::new("me")),
            "the more fixed"
        );
        assert_eq!(routes.api_of("get", "/users/me/"), Some(&ItemId::new("me")));
        assert_eq!(routes.api_of("POST", "/users"), Some(&ItemId::new("make")));
        assert_eq!(routes.api_of("GET", "/orders"), None);
    }

    #[test]
    fn it_answers_with_an_example() {
        let routes = routes();
        let answer = ask(&routes, "GET", "/users/7?verbose=1", &[], "");
        assert_eq!(answer.status, 200);
        assert_eq!(answer.body, br#"{"id":1}"#);
        assert_eq!(answer.api.as_deref(), Some("user"));
        assert!(answer.headers.contains(&("x-trace".into(), "t".into())));
        assert!(
            answer
                .headers
                .contains(&("Content-Type".into(), "application/json".into()))
        );
        assert!(
            !answer
                .headers
                .iter()
                .any(|(name, _)| name == "content-length" || name == "Date"),
            "not this answer's"
        );
        assert!(
            answer
                .headers
                .contains(&("Access-Control-Allow-Origin".into(), "*".into()))
        );

        let picked = ask(&routes, "GET", "/users/7", &[(EXAMPLE_HEADER, "Missing")], "");
        assert_eq!((picked.status, picked.body.as_slice()), (404, b"no".as_slice()));
        let text = ask(&routes, "GET", "/users/me", &[], "");
        assert!(
            text.headers
                .contains(&("Content-Type".into(), "text/plain; charset=utf-8".into()))
        );
    }

    #[test]
    fn what_cannot_be_answered_says_why() {
        let routes = routes();
        let none = ask(&routes, "GET", "/orders", &[], "");
        assert_eq!(none.status, 404);
        assert!(String::from_utf8_lossy(&none.body).contains("no API of the project answers GET /orders"));
        let no_example = ask(&routes, "POST", "/users", &[], "");
        assert_eq!((no_example.status, no_example.api.as_deref()), (404, Some("make")));
        assert!(String::from_utf8_lossy(&no_example.body).contains("no example"));
        let wrong = ask(&routes, "DELETE", "/users", &[], "");
        assert_eq!(wrong.status, 405);
        assert!(wrong.headers.contains(&("Allow".into(), "POST".into())));
        let unnamed = ask(&routes, "GET", "/users/7", &[(EXAMPLE_HEADER, "other")], "");
        let said: serde_json::Value = serde_json::from_slice(&unnamed.body).expect("JSON");
        assert_eq!(said["error"], "user has no example named \"other\"");
    }

    #[test]
    fn a_preflight_is_answered() {
        let answer = ask(
            &routes(),
            "OPTIONS",
            "/users/7",
            &[
                ("Access-Control-Request-Method", "POST"),
                ("Access-Control-Request-Headers", "content-type, x-token"),
            ],
            "",
        );
        assert_eq!(answer.status, 204);
        assert!(
            answer
                .headers
                .contains(&("Access-Control-Allow-Headers".into(), "content-type, x-token".into()))
        );
        assert!(
            answer
                .headers
                .contains(&("Access-Control-Allow-Origin".into(), "*".into()))
        );
    }

    fn when(source: MockSource, name: &str, value: &str) -> MockCondition {
        MockCondition {
            source,
            name: name.into(),
            value: value.into(),
        }
    }

    fn conditional() -> Routes {
        let mut vip = example("vip", 200, r#"{"level":"vip"}"#);
        vip.when = vec![
            when(MockSource::Query, "user", "7"),
            when(MockSource::Header, "X-Tier", ""),
        ];
        let mut empty = example("empty", 200, "[]");
        empty.when = vec![when(MockSource::Body, "$.page", "2")];
        let mut named = example("named", 201, "made");
        named.when = vec![when(MockSource::Body, "name", "Ann")];
        let mut blank = example("blank-row", 500, "never");
        blank.when = vec![when(MockSource::Query, " ", "x")];
        Routes::of(&TreeData {
            apis: vec![api(
                "list",
                HttpMethod::Post,
                "/items",
                vec![vip, empty, named, example("plain", 200, "plain"), blank],
            )],
            ..Default::default()
        })
    }

    #[test]
    fn conditions_pick_the_example() {
        let routes = conditional();
        let used = |answer: &Answer| answer.example.clone().unwrap_or_default();
        let vip = ask(&routes, "POST", "/items?user=7", &[("x-tier", "gold")], "");
        assert_eq!(used(&vip), "vip", "query equal, header present (any case)");
        let no_header = ask(&routes, "POST", "/items?user=7", &[], "");
        assert_eq!(used(&no_header), "plain", "every condition must hold");
        let other_user = ask(&routes, "POST", "/items?user=8", &[("X-Tier", "gold")], "");
        assert_eq!(used(&other_user), "plain");
        let encoded = ask(&routes, "POST", "/items?user=%37", &[("X-Tier", "")], "");
        assert_eq!(used(&encoded), "vip", "the query is decoded");
        let page = ask(&routes, "POST", "/items", &[], r#"{"page":2}"#);
        assert_eq!(used(&page), "empty", "a number matches its text");
        let rooted = ask(&routes, "POST", "/items", &[], r#"{"name":"Ann"}"#);
        assert_eq!(
            (used(&rooted), rooted.status),
            ("named".to_string(), 201),
            "`$` may be left out"
        );
        let not_json = ask(&routes, "POST", "/items", &[], "page=2");
        assert_eq!(used(&not_json), "plain", "a body condition needs JSON");
        let named = ask(&routes, "POST", "/items?user=7", &[(EXAMPLE_HEADER, "empty")], "");
        assert_eq!(used(&named), "empty", "the header's name wins over conditions");
    }

    #[test]
    fn without_a_fallback_it_says_why() {
        let mut only = example("only", 200, "x");
        only.when = vec![when(MockSource::Header, "x-a", "1")];
        let routes = Routes::of(&TreeData {
            apis: vec![api("one", HttpMethod::Get, "/one", vec![only])],
            ..Default::default()
        });
        let answer = ask(&routes, "GET", "/one", &[], "");
        assert_eq!((answer.status, answer.example), (404, None));
        assert!(String::from_utf8_lossy(&answer.body).contains("conditions"));
        assert_eq!(ask(&routes, "GET", "/one", &[("x-a", "1")], "").status, 200);
    }

    #[test]
    fn delays_and_streams_are_bounded_and_named() {
        let mut slow = example("慢 stream", 200, "data: a\n\nevent: b\r\ndata: 2\r\n\r\ndata: tail");
        slow.headers = vec![("Content-Type".into(), "text/event-stream; charset=utf-8".into())];
        slow.delay_ms = 99_000;
        slow.stream_interval_ms = 50_000;
        let mut flat = example("flat", 200, "{}");
        flat.stream_interval_ms = 100;
        let routes = Routes::of(&TreeData {
            apis: vec![
                api("sse", HttpMethod::Get, "/sse", vec![slow]),
                api("flat", HttpMethod::Get, "/flat", vec![flat]),
            ],
            ..Default::default()
        });
        let answer = ask(&routes, "GET", "/sse", &[], "");
        assert_eq!(answer.delay_ms, Example::MAX_DELAY_MS);
        assert_eq!(answer.stream_interval_ms, Example::MAX_STREAM_INTERVAL_MS);
        assert_eq!(
            answer.events(),
            vec![
                b"data: a\n\n".to_vec(),
                b"event: b\r\ndata: 2\r\n\r\n".to_vec(),
                b"data: tail".to_vec()
            ]
        );
        assert!(
            answer
                .headers
                .contains(&(USED_HEADER.into(), "%E6%85%A2 stream".into())),
            "a name that is no header value is encoded"
        );
        let flat = ask(&routes, "GET", "/flat", &[], "");
        assert_eq!(flat.stream_interval_ms, 0, "only an event stream is streamed");
    }

    #[test]
    fn conditions_read_and_write_as_text() {
        let text =
            "query user = 7\n\n# a note\nHEADER X-Tier\nheader x-env=prod\nbody $[?(@.a=='b')]\nbody $.role = a = b";
        let read = conditions_from_text(text).expect("reads");
        assert_eq!(
            read,
            vec![
                when(MockSource::Query, "user", "7"),
                when(MockSource::Header, "X-Tier", ""),
                when(MockSource::Header, "x-env", "prod"),
                when(MockSource::Body, "$[?(@.a=='b')]", ""),
                when(MockSource::Body, "$.role", "a = b"),
            ]
        );
        assert_eq!(conditions_from_text(&conditions_text(&read)), Ok(read));
        assert_eq!(conditions_from_text("query a\ncookie b"), Err(2), "an unknown source");
        assert_eq!(conditions_from_text("header = 1"), Err(1), "no name");
    }
}
