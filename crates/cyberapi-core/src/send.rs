//! A saved request made ready to send (M5, F5.2–F5.4): the environment
//! variable put into the URI, the project's global headers added, the
//! template functions run, the body encoded, the query appended — what an
//! `HttpEngine` needs and nothing it has to look up.
//!
//! Header precedence (#3): the old app inserted the request's own headers,
//! then the global ones, then its defaults, each replacing a same-named one
//! before it — so a global header overrode the request's own. Here the
//! request wins: defaults, then global headers, then Basic auth, then the
//! request's own headers.
//!
//! The engine is `cyberapi-http`'s; in the browser build the prepared
//! request crosses the bridge as JSON.

use crate::model::{ContentType, HttpMethod, HttpRequest, KvParam, Variable, VariableCategory, split_env_prefix};
use crate::prefs::Timeouts;
use crate::template::{self, FileReader, Inputs, Needs, TemplateError};
use base64::Engine as _;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};

/// A request as it goes out.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedRequest {
    pub method: HttpMethod,
    pub url: String,
    /// One per name, ignoring case, in the order they are sent.
    pub headers: Vec<(String, String)>,
    #[serde(with = "base64_bytes")]
    pub body: Vec<u8>,
    /// Seconds, each above zero.
    pub timeouts: Timeouts,
    /// HTTPS goes ahead whatever the server's certificate (F14.9).
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

impl PreparedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Snafu, PartialEq, Eq)]
pub enum PrepareError {
    #[snafu(display("{source}"))]
    Template { source: TemplateError },
    #[snafu(display("{uri} is not a URL: {message}"))]
    Url { uri: String, message: String },
}

/// What [`prepare`] takes besides the request.
pub struct Context<'a> {
    /// The project's variables, any category.
    pub variables: &'a [Variable],
    pub template: &'a Inputs,
    pub files: &'a dyn FileReader,
    pub user_agent: &'a str,
    pub timeouts: Timeouts,
    /// F14.9.
    pub accept_invalid_certs: bool,
    /// Separates multipart fields; random, from the caller.
    pub boundary: &'a str,
}

fn enabled(variables: &[Variable], category: VariableCategory) -> impl Iterator<Item = &Variable> {
    variables
        .iter()
        .filter(move |variable| variable.enabled && variable.category == category)
}

/// F5.2: the URI with its environment variable's value in place of
/// `{{NAME}}` — the first enabled variable of that name; left as written when
/// there is none.
pub fn resolve_uri(uri: &str, variables: &[Variable]) -> String {
    let uri = uri.trim();
    match split_env_prefix(uri) {
        (Some(name), rest) => match enabled(variables, VariableCategory::Environment).find(|v| v.name == name) {
            Some(variable) => format!("{}{rest}", variable.value),
            None => uri.to_string(),
        },
        (None, _) => uri.to_string(),
    }
}

/// F5.2: the request with the environment variable resolved and the
/// project's enabled global headers added after its own.
pub fn fill_values(request: &HttpRequest, variables: &[Variable]) -> HttpRequest {
    let mut filled = request.clone();
    filled.uri = resolve_uri(&request.uri, variables);
    filled.headers.extend(
        enabled(variables, VariableCategory::GlobalRequestHeader).map(|variable| KvParam {
            key: variable.name.clone(),
            value: variable.value.clone(),
            enabled: true,
        }),
    );
    filled
}

/// The texts whose template functions run: the body, or its form values;
/// query and header values (not keys, not the URI); the global headers.
fn templated_texts<'a>(request: &'a HttpRequest, variables: &'a [Variable]) -> Vec<std::borrow::Cow<'a, str>> {
    let mut texts: Vec<std::borrow::Cow<'a, str>> = Vec::new();
    if request.method.has_body() {
        if request.content_type.is_key_value() {
            texts.extend(form_fields(&request.body).into_iter().map(|kv| kv.value.into()));
        } else {
            texts.push(request.body.as_str().into());
        }
    }
    let lists = [&request.query, &request.headers];
    texts.extend(
        lists
            .into_iter()
            .flatten()
            .filter(|kv| kv.enabled)
            .map(|kv| kv.value.as_str().into()),
    );
    texts.extend(enabled(variables, VariableCategory::GlobalRequestHeader).map(|v| v.value.as_str().into()));
    texts
}

/// What the request's template functions need gathered before [`prepare`].
pub fn needs(request: &HttpRequest, variables: &[Variable]) -> Needs {
    let mut needs = Needs::default();
    for text in templated_texts(request, variables) {
        needs.add(template::needs(&text));
    }
    needs
}

/// A form or multipart body's fields; an unreadable list is none.
pub fn form_fields(body: &str) -> Vec<KvParam> {
    if body.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str(body).unwrap_or_default()
}

/// What `encodeURIComponent` leaves alone.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

pub fn encode_uri_component(text: &str) -> String {
    utf8_percent_encode(text, URI_COMPONENT).to_string()
}

/// The text a JSON-looking body is sent as: lines starting with `//` are
/// comments and go (4.3).
pub fn strip_json_comments(body: &str) -> String {
    let trimmed = body.trim();
    let looks_json = trimmed.len() >= 2
        && matches!(
            (trimmed.as_bytes()[0], trimmed.as_bytes()[trimmed.len() - 1]),
            (b'{', b'}') | (b'[', b']')
        );
    if !looks_json {
        return body.to_string();
    }
    body.split('\n')
        .filter(|line| !line.trim().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A JSON body indented by two spaces, as the editor's Format does it
/// (F4.7). Comment lines go, as sending drops them; `{{…}}` templates stay
/// as written, in a string or in place of a value. `Err` says why the body
/// is no JSON.
pub fn format_json(body: &str) -> Result<String, String> {
    let text = strip_json_comments(body);
    // Each template stands in as something JSON takes — its own string
    // outside a string, plain text inside one — and is put back after.
    let token = |n: usize| format!("\u{E000}{n}\u{E001}");
    let mut templates: Vec<(&str, bool)> = Vec::new();
    let mut held = String::with_capacity(text.len());
    let (mut in_string, mut escaped) = (false, false);
    let mut at = 0;
    while at < text.len() {
        if text[at..].starts_with("{{")
            && let Some(end) = text[at..].find("}}").map(|found| at + found + 2)
        {
            let stand_in = token(templates.len());
            if in_string {
                held.push_str(&stand_in);
            } else {
                held.push_str(&format!("\"{stand_in}\""));
            }
            templates.push((&text[at..end], in_string));
            at = end;
            continue;
        }
        let Some(ch) = text[at..].chars().next() else { break };
        match ch {
            '\\' if in_string => escaped = !escaped,
            '"' if !escaped => in_string = !in_string,
            _ => escaped = false,
        }
        if ch != '\\' {
            escaped = false;
        }
        held.push(ch);
        at += ch.len_utf8();
    }
    let value: serde_json::Value = serde_json::from_str(&held).map_err(|error| error.to_string())?;
    let mut pretty = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
    for (n, (template, in_string)) in templates.into_iter().enumerate() {
        let stand_in = if in_string {
            token(n)
        } else {
            format!("\"{}\"", token(n))
        };
        pretty = pretty.replacen(&stand_in, template, 1);
    }
    Ok(pretty)
}

/// Why a JSON body would not parse, as serde words it, its line and column
/// those the editor shows: comment lines and `{{…}}` templates are taken as
/// what they become (nothing, a value). `None` for a body that parses, or
/// an empty one.
pub fn json_problem(body: &str) -> Option<String> {
    if body.trim().is_empty() {
        return None;
    }
    // Blanked rather than taken out, so the places stay where they were.
    let mut text: String = body
        .split('\n')
        .map(|line| if line.trim_start().starts_with("//") { "" } else { line })
        .collect::<Vec<_>>()
        .join("\n");
    let mut from = 0;
    while let Some(start) = text[from..].find("{{").map(|at| from + at) {
        let Some(end) = text[start..].find("}}").map(|at| start + at + 2) else {
            break;
        };
        let value = format!("0{}", " ".repeat(text[start..end].chars().count() - 1));
        text.replace_range(start..end, &value);
        from = start + value.len();
    }
    serde_json::from_str::<serde_json::Value>(&text)
        .err()
        .map(|error| error.to_string())
}

/// Where a URI without a scheme goes: plain HTTP, as a browser's address bar
/// would. The old app refused it.
fn with_scheme(uri: &str) -> String {
    if uri.contains("://") {
        uri.to_string()
    } else {
        format!("http://{uri}")
    }
}

/// F5.2–F5.4: everything the engine needs.
pub fn prepare(request: &HttpRequest, cx: &Context<'_>) -> Result<PreparedRequest, PrepareError> {
    let render = |text: &str| template::render(text, cx.template, cx.files).context(TemplateSnafu);

    // The URL, then the enabled query appended to what it already has.
    let uri = with_scheme(&resolve_uri(&request.uri, cx.variables));
    let mut url = url::Url::parse(&uri).map_err(|error| PrepareError::Url {
        uri: uri.clone(),
        message: error.to_string(),
    })?;
    for kv in request.query.iter().filter(|kv| kv.enabled) {
        url.query_pairs_mut().append_pair(&kv.key, &render(&kv.value)?);
    }

    let mut headers = Headers::default();
    headers.set("User-Agent", cx.user_agent);
    for variable in enabled(cx.variables, VariableCategory::GlobalRequestHeader) {
        headers.set(&variable.name, &render(&variable.value)?);
    }
    if let Some(auth) = request.auth.iter().find(|kv| kv.enabled) {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", auth.key, auth.value));
        headers.set("Authorization", &format!("Basic {token}"));
    }
    for kv in request
        .headers
        .iter()
        .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
    {
        headers.set(kv.key.trim(), &render(&kv.value)?);
    }

    // F5.3: only POST, PUT and PATCH carry a body and its type.
    let mut body = Vec::new();
    if request.method.has_body() {
        let content_type = match request.content_type {
            ContentType::Form => {
                let mut pairs = Vec::new();
                for kv in form_fields(&request.body).iter().filter(|kv| kv.enabled) {
                    pairs.push(format!(
                        "{}={}",
                        encode_uri_component(&kv.key),
                        encode_uri_component(&render(&kv.value)?)
                    ));
                }
                body = pairs.join("&").into_bytes();
                request.content_type.as_str().to_string()
            }
            ContentType::Multipart => {
                body = multipart(&request.body, cx, &render)?;
                format!("multipart/form-data; boundary={}", cx.boundary)
            }
            _ => {
                body = render(&strip_json_comments(&request.body))?.into_bytes();
                request.content_type.as_str().to_string()
            }
        };
        // A Content-Type the request names itself stays.
        headers.set_default("Content-Type", &content_type);
    }

    Ok(PreparedRequest {
        method: request.method,
        url: url.to_string(),
        headers: headers.0,
        body,
        timeouts: cx.timeouts.effective(),
        accept_invalid_certs: cx.accept_invalid_certs,
    })
}

/// F5.3: enabled fields with a name; a `file://` value is the file's bytes,
/// sent under its file name (the old app sent the whole local path).
fn multipart(
    body: &str,
    cx: &Context<'_>,
    render: &dyn Fn(&str) -> Result<String, PrepareError>,
) -> Result<Vec<u8>, PrepareError> {
    let quote = |text: &str| text.replace('"', "%22").replace('\r', "%0D").replace('\n', "%0A");
    let mut out = Vec::new();
    for kv in form_fields(body).iter().filter(|kv| kv.enabled && !kv.key.is_empty()) {
        out.extend_from_slice(format!("--{}\r\n", cx.boundary).as_bytes());
        match kv.value.strip_prefix("file://") {
            Some(path) => {
                let bytes = cx.files.read(path, None).map_err(|message| PrepareError::Template {
                    source: TemplateError::ReadFile {
                        file: path.to_string(),
                        message,
                    },
                })?;
                let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                out.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {mime}\r\n\r\n",
                        quote(&kv.key),
                        quote(name)
                    )
                    .as_bytes(),
                );
                out.extend_from_slice(&bytes);
            }
            None => {
                out.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", quote(&kv.key)).as_bytes(),
                );
                out.extend_from_slice(render(&kv.value)?.as_bytes());
            }
        }
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{}--\r\n", cx.boundary).as_bytes());
    Ok(out)
}

/// Headers by name, ignoring case; a later value replaces an earlier one in
/// its place.
#[derive(Default)]
struct Headers(Vec<(String, String)>);

impl Headers {
    fn set(&mut self, name: &str, value: &str) {
        match self.0.iter_mut().find(|(key, _)| key.eq_ignore_ascii_case(name)) {
            Some(found) => *found = (name.to_string(), value.to_string()),
            None => self.0.push((name.to_string(), value.to_string())),
        }
    }

    fn set_default(&mut self, name: &str, value: &str) {
        if !self.0.iter().any(|(key, _)| key.eq_ignore_ascii_case(name)) {
            self.0.push((name.to_string(), value.to_string()));
        }
    }
}

/// Bytes as base64 text in JSON: requests and responses cross the bridge.
pub mod base64_bytes {
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::NoFiles;
    use crate::{CollectionId, VariableId};
    use pretty_assertions::assert_eq;

    fn variable(category: VariableCategory, name: &str, value: &str, enabled: bool) -> Variable {
        Variable {
            id: VariableId::new(name),
            collection: CollectionId::new("c"),
            category,
            name: name.into(),
            value: value.into(),
            enabled,
            created_at: Default::default(),
            updated_at: Default::default(),
        }
    }

    fn kv(key: &str, value: &str) -> KvParam {
        KvParam {
            key: key.into(),
            value: value.into(),
            enabled: true,
        }
    }

    struct OneFile;

    impl FileReader for OneFile {
        fn read(&self, file: &str, _: Option<template::BaseDir>) -> Result<Vec<u8>, String> {
            match file {
                "/home/me/a.png" => Ok(vec![1, 2, 3]),
                _ => Err("missing".into()),
            }
        }
    }

    fn prepare_with(request: &HttpRequest, variables: &[Variable]) -> Result<PreparedRequest, PrepareError> {
        prepare(
            request,
            &Context {
                variables,
                template: &Inputs {
                    now: 7,
                    ..Default::default()
                },
                files: &OneFile,
                user_agent: "CyberAPI/test",
                timeouts: Timeouts::default(),
                accept_invalid_certs: false,
                boundary: "XYZ",
            },
        )
    }

    #[test]
    fn json_is_formatted_with_its_templates_as_written() {
        let body =
            "{\"z\":1,\n// a note\n\"a\":{{value(x)}},\"c\":\"id-{{get(01J, token)}}\",\"q\":\"say \\\"{{hi}}\\\"\"}";
        assert_eq!(
            format_json(body),
            Ok("{\n  \"z\": 1,\n  \"a\": {{value(x)}},\n  \"c\": \"id-{{get(01J, token)}}\",\n  \"q\": \"say \\\"{{hi}}\\\"\"\n}".to_string())
        );
        assert!(format_json("{\"a\": 1 \"b\": 2}").is_err());
    }

    #[test]
    fn a_json_body_s_problem_is_placed_where_the_editor_shows_it() {
        // Comments and templates are what they become: this parses.
        let fine = "{\n  // the tenant\n  \"a\": {{value(id)}},\n  \"b\": \"{{get(01J, token)}}\"\n}";
        assert_eq!(json_problem(fine), None);
        assert_eq!(json_problem("  "), None);
        // A comma missing on line 4, counted with the comment line in.
        let broken = "{\n  // the tenant\n  \"a\": 1\n  \"b\": 2\n}";
        let problem = json_problem(broken).expect("a problem");
        assert!(problem.contains("line 4 column 3"), "{problem}");
    }

    #[test]
    fn the_environment_variable_and_query_make_the_url() {
        let variables = [
            variable(VariableCategory::Environment, "HOST", "http://off", false),
            variable(VariableCategory::Environment, "HOST", "https://api.test", true),
            variable(VariableCategory::Environment, "HOST", "http://second", true),
        ];
        let request = HttpRequest {
            uri: " {{HOST}}/users?a=1 ".into(),
            query: vec![kv("q", "a b&c"), kv("t", "{{ts()}}"), KvParam::default()],
            ..Default::default()
        };
        let prepared = prepare_with(&request, &variables).expect("prepare");
        assert_eq!(prepared.url, "https://api.test/users?a=1&q=a+b%26c&t=7");
        assert_eq!(prepared.method, HttpMethod::Get);
        assert_eq!(prepared.timeouts, Timeouts::default().effective());

        // No such variable: the URI stays as written.
        assert_eq!(resolve_uri("{{NOPE}}/x", &variables), "{{NOPE}}/x");
        let broken = HttpRequest {
            uri: "http://[::1/x".into(),
            ..Default::default()
        };
        assert!(matches!(
            prepare_with(&broken, &variables),
            Err(PrepareError::Url { .. })
        ));
        // No scheme is plain HTTP.
        let bare = HttpRequest {
            uri: "localhost:8080/a".into(),
            ..Default::default()
        };
        assert_eq!(
            prepare_with(&bare, &[]).expect("prepare").url,
            "http://localhost:8080/a"
        );
    }

    #[test]
    fn the_requests_own_headers_win() {
        let variables = [
            variable(VariableCategory::GlobalRequestHeader, "X-Team", "global", true),
            variable(VariableCategory::GlobalRequestHeader, "X-Trace", "{{ts()}}", true),
            variable(VariableCategory::GlobalRequestHeader, "X-Off", "no", false),
        ];
        let request = HttpRequest {
            uri: "http://a.test".into(),
            headers: vec![
                kv("x-team", "mine"),
                kv("user-agent", "curl/8"),
                KvParam {
                    key: "X-Disabled".into(),
                    value: "v".into(),
                    enabled: false,
                },
            ],
            auth: vec![
                KvParam {
                    key: "off".into(),
                    value: "x".into(),
                    enabled: false,
                },
                kv("user", "pass"),
                kv("second", "ignored"),
            ],
            ..Default::default()
        };
        let prepared = prepare_with(&request, &variables).expect("prepare");
        assert_eq!(
            prepared.headers,
            [
                ("user-agent".to_string(), "curl/8".to_string()),
                ("x-team".to_string(), "mine".to_string()),
                ("X-Trace".to_string(), "7".to_string()),
                ("Authorization".to_string(), "Basic dXNlcjpwYXNz".to_string()),
            ]
        );
        assert!(prepared.body.is_empty());
    }

    #[test]
    fn bodies_go_only_with_post_put_and_patch() {
        let mut request = HttpRequest {
            uri: "http://a.test".into(),
            body: "{\n  // a comment\n  \"t\": \"{{ts()}}\"\n}".into(),
            ..Default::default()
        };
        assert!(prepare_with(&request, &[]).expect("prepare").body.is_empty());

        request.method = HttpMethod::Post;
        let prepared = prepare_with(&request, &[]).expect("prepare");
        assert_eq!(String::from_utf8_lossy(&prepared.body), "{\n  \"t\": \"7\"\n}");
        assert_eq!(prepared.header("content-type"), Some("application/json"));

        // The request's own Content-Type stays.
        request.headers = vec![kv("Content-Type", "application/vnd.api+json")];
        let prepared = prepare_with(&request, &[]).expect("prepare");
        assert_eq!(prepared.header("Content-Type"), Some("application/vnd.api+json"));
    }

    #[test]
    fn form_fields_are_encoded_like_encode_uri_component() {
        let fields = vec![
            kv("a b", "1+1=2 & {{ts()}}"),
            KvParam {
                key: "off".into(),
                value: "x".into(),
                enabled: false,
            },
            kv("名", "值!*'()"),
        ];
        let request = HttpRequest {
            method: HttpMethod::Put,
            uri: "http://a.test".into(),
            content_type: ContentType::Form,
            body: serde_json::to_string(&fields).expect("json"),
            ..Default::default()
        };
        let prepared = prepare_with(&request, &[]).expect("prepare");
        assert_eq!(
            String::from_utf8_lossy(&prepared.body),
            "a%20b=1%2B1%3D2%20%26%207&%E5%90%8D=%E5%80%BC!*'()"
        );
        assert_eq!(
            prepared.header("Content-Type"),
            Some("application/x-www-form-urlencoded")
        );
    }

    #[test]
    fn multipart_sends_files_by_name() {
        let fields = vec![
            kv("note", "hi {{ts()}}"),
            kv("file", "file:///home/me/a.png"),
            kv("", "nameless"),
        ];
        let request = HttpRequest {
            method: HttpMethod::Post,
            uri: "http://a.test".into(),
            content_type: ContentType::Multipart,
            body: serde_json::to_string(&fields).expect("json"),
            ..Default::default()
        };
        let prepared = prepare_with(&request, &[]).expect("prepare");
        let mut expected = b"--XYZ\r\nContent-Disposition: form-data; name=\"note\"\r\n\r\nhi 7\r\n\
--XYZ\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\nContent-Type: image/png\r\n\r\n"
            .to_vec();
        expected.extend_from_slice(&[1, 2, 3]);
        expected.extend_from_slice(b"\r\n--XYZ--\r\n");
        assert_eq!(prepared.body, expected);
        assert_eq!(
            prepared.header("content-type"),
            Some("multipart/form-data; boundary=XYZ")
        );

        let missing = HttpRequest {
            body: serde_json::to_string(&[kv("f", "file:///nope")]).expect("json"),
            ..request
        };
        assert_eq!(
            prepare(
                &missing,
                &Context {
                    variables: &[],
                    template: &Inputs::default(),
                    files: &NoFiles("no files here"),
                    user_agent: "",
                    timeouts: Timeouts::default(),
                    accept_invalid_certs: false,
                    boundary: "B",
                }
            ),
            Err(PrepareError::Template {
                source: TemplateError::ReadFile {
                    file: "/nope".into(),
                    message: "no files here".into()
                }
            })
        );
    }

    #[test]
    fn needs_cover_body_query_headers_and_global_headers() {
        let variables = [variable(
            VariableCategory::GlobalRequestHeader,
            "X",
            "{{get(g, a)}}",
            true,
        )];
        let request = HttpRequest {
            method: HttpMethod::Post,
            body: "{{get(b, a)}}".into(),
            query: vec![kv("q", "{{get(q, a)}}")],
            headers: vec![kv("h", "{{of()}}")],
            ..Default::default()
        };
        let needs = needs(&request, &variables);
        assert_eq!(needs.responses, ["b", "q", "g"]);
        assert!(needs.picks_file);
    }

    #[test]
    fn fill_values_is_what_the_old_app_sent_before_functions() {
        let variables = [
            variable(VariableCategory::Environment, "HOST", "https://h", true),
            variable(VariableCategory::GlobalRequestHeader, "X-G", "g", true),
        ];
        let request = HttpRequest {
            uri: "{{HOST}}/a".into(),
            headers: vec![kv("X-A", "a")],
            ..Default::default()
        };
        let filled = fill_values(&request, &variables);
        assert_eq!(filled.uri, "https://h/a");
        assert_eq!(filled.headers, [kv("X-A", "a"), kv("X-G", "g")]);
    }

    #[test]
    fn prepared_requests_cross_the_bridge_as_json() {
        let prepared = PreparedRequest {
            body: vec![0, 159, 146, 150],
            ..Default::default()
        };
        let json = serde_json::to_string(&prepared).expect("json");
        assert!(json.contains("\"body\":\"AJ+Slg==\""), "{json}");
        assert_eq!(serde_json::from_str::<PreparedRequest>(&json).expect("read"), prepared);
    }
}
