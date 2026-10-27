//! G2: a project answering as a mock server would, from its APIs'
//! examples (F3). A request is matched to an API by its method and its
//! path — the API's address without its prefix variable, scheme, host and
//! query; a segment written `{{id}}`, `:id` or `{id}` matches any — the
//! more fixed segments the better. It is answered with the API's first
//! example, or the one the `x-mock-example` header names. Every answer
//! lets any origin read it, and a CORS preflight is answered, so a page
//! being built can call it.

use crate::ItemId;
use crate::model::{Example, HttpMethod, split_env_prefix};
use crate::tree::TreeData;
use serde_json::json;

/// The header that picks an example by its name.
pub const EXAMPLE_HEADER: &str = "x-mock-example";

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

/// How a request is answered: what to send, and for the log, which API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The API that answered, by name.
    pub api: Option<String>,
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

    /// The answer to `method` `path` (its query left on or not); `wanted`
    /// names an example; `preflight_headers`, what a CORS preflight asks to send.
    pub fn answer(&self, method: &str, path: &str, wanted: Option<&str>, preflight_headers: Option<&str>) -> Answer {
        let cors = |mut headers: Vec<(String, String)>| {
            headers.push(("Access-Control-Allow-Origin".into(), "*".into()));
            headers.push(("Access-Control-Expose-Headers".into(), "*".into()));
            headers
        };
        if method.eq_ignore_ascii_case("OPTIONS") && preflight_headers.is_some() {
            let mut headers = vec![
                (
                    "Access-Control-Allow-Methods".into(),
                    "GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS".into(),
                ),
                ("Access-Control-Max-Age".into(), "600".into()),
            ];
            if let Some(asked) = preflight_headers.filter(|asked| !asked.is_empty()) {
                headers.push(("Access-Control-Allow-Headers".into(), asked.to_string()));
            }
            return Answer {
                status: 204,
                headers: cors(headers),
                body: Vec::new(),
                api: None,
            };
        }
        let path = path.split(['?', '#']).next().unwrap_or_default();
        let asked = segments(path);
        let matches = |route: &&Route| {
            route.segments.len() == asked.len()
                && route
                    .segments
                    .iter()
                    .zip(&asked)
                    .all(|(segment, asked)| segment.as_deref().is_none_or(|fixed| fixed == *asked))
        };
        let fixed = |route: &&Route| route.segments.iter().filter(|segment| segment.is_some()).count();
        let found: Vec<&Route> = self.0.iter().filter(matches).collect();
        let json = |status: u16, error: String, api: Option<String>| Answer {
            status,
            headers: cors(vec![("Content-Type".into(), "application/json".into())]),
            body: crate::format::json(&json!({ "error": error })).into_bytes(),
            api,
        };
        let Some(route) = found
            .iter()
            .filter(|route| route.method.as_str().eq_ignore_ascii_case(method))
            .max_by_key(|route| fixed(route))
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
        let example = match wanted.map(str::trim).filter(|name| !name.is_empty()) {
            Some(name) => route
                .examples
                .iter()
                .find(|example| example.name.eq_ignore_ascii_case(name)),
            None => route.examples.first(),
        };
        let Some(example) = example else {
            let why = match wanted {
                Some(name) => format!("{} has no example named {name:?}", route.name),
                None => format!(
                    "{} has no example to answer with: keep one from its history",
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
            let kind = match serde_json::from_str::<serde_json::Value>(&example.body) {
                Ok(_) => "application/json",
                Err(_) => "text/plain; charset=utf-8",
            };
            headers.push(("Content-Type".into(), kind.into()));
        }
        Answer {
            status: example.status,
            headers: cors(headers),
            body: example.body.clone().into_bytes(),
            api: Some(route.name.clone()),
        }
    }

    /// The API that answers, for tests and the log.
    pub fn api_of(&self, method: &str, path: &str) -> Option<&ItemId> {
        let asked = segments(path);
        self.0
            .iter()
            .filter(|route| route.method.as_str().eq_ignore_ascii_case(method))
            .filter(|route| {
                route.segments.len() == asked.len()
                    && route
                        .segments
                        .iter()
                        .zip(&asked)
                        .all(|(segment, asked)| segment.as_deref().is_none_or(|fixed| fixed == *asked))
            })
            .max_by_key(|route| route.segments.iter().filter(|segment| segment.is_some()).count())
            .map(|route| &route.api)
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
        }
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
        let answer = routes.answer("GET", "/users/7?verbose=1", None, None);
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

        let picked = routes.answer("GET", "/users/7", Some("Missing"), None);
        assert_eq!((picked.status, picked.body.as_slice()), (404, b"no".as_slice()));
        let text = routes.answer("GET", "/users/me", None, None);
        assert!(
            text.headers
                .contains(&("Content-Type".into(), "text/plain; charset=utf-8".into()))
        );
    }

    #[test]
    fn what_cannot_be_answered_says_why() {
        let routes = routes();
        let none = routes.answer("GET", "/orders", None, None);
        assert_eq!(none.status, 404);
        assert!(String::from_utf8_lossy(&none.body).contains("no API of the project answers GET /orders"));
        let no_example = routes.answer("POST", "/users", None, None);
        assert_eq!((no_example.status, no_example.api.as_deref()), (404, Some("make")));
        assert!(String::from_utf8_lossy(&no_example.body).contains("no example"));
        let wrong = routes.answer("DELETE", "/users", None, None);
        assert_eq!(wrong.status, 405);
        assert!(wrong.headers.contains(&("Allow".into(), "POST".into())));
        let unnamed = routes.answer("GET", "/users/7", Some("other"), None);
        let said: serde_json::Value = serde_json::from_slice(&unnamed.body).expect("JSON");
        assert_eq!(said["error"], "user has no example named \"other\"");
    }

    #[test]
    fn a_preflight_is_answered() {
        let answer = routes().answer("OPTIONS", "/users/7", None, Some("content-type, x-token"));
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
}
