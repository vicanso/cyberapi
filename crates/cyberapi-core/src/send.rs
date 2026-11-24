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
//!
//! A gRPC call (`crate::grpc`) goes as the HTTP/2 POST gRPC is: its message
//! encoded with the definitions the API keeps, its headers the metadata,
//! no query.

use crate::auth::Signed;
use crate::digest::DigestLogin;
use crate::graphql::GraphQlBody;
use crate::grpc::{self, GrpcBody};
use crate::inherit::Inherited;
use crate::model::{
    ContentType, HttpMethod, HttpRequest, KvParam, Variable, VariableCategory, path_param_names, replace_path_params,
    split_env_prefix,
};
use crate::prefs::{HttpVersion, Timeouts};
use crate::proxy::ProxyPrefs;
use crate::response::{HeaderSource, SENT_BODY_LIMIT, SentHeader, SentRequest};
use crate::template::{self, FileReader, Inputs, Needs, TemplateError};
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
    /// F14.10.
    #[serde(default)]
    pub http_version: HttpVersion,
    /// The answer is a stream whatever its type says (a gRPC call's that
    /// streams from the server): a watched send reads it as it comes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub streams: bool,
    /// The proxy it goes through (#60). Never sent to the bridge: what a
    /// page asks of it goes direct, within its address rules.
    #[serde(skip)]
    pub proxy: ProxyPrefs,
    /// C2: what answers a server's Digest challenge, should it ask. Never
    /// sent to the bridge: the answer goes, as a header.
    #[serde(skip)]
    pub digest: Option<DigestLogin>,
    /// C4: what the token it is signed with is asked with; the service
    /// asks, and adds the `Authorization`. Never sent to the bridge: the
    /// token request and the token go, as requests and a header.
    #[serde(skip)]
    pub oauth: Option<crate::oauth::OAuthLogin>,
    /// C4: AWS SigV4 credentials used to sign this request (kept for `--aws-sigv4`
    /// cURL output and per-request re-signing in load tests; never sent to the bridge).
    #[serde(skip)]
    pub aws_sigv4: Option<crate::aws_sigv4::AwsSigV4Login>,
    /// R6: whether JWT Bearer signing contributed to this prepared request
    /// (kept for Copy-as-Code expiry notes and per-request re-signing in load tests).
    #[serde(skip)]
    pub jwt_bearer: bool,
    /// C5: the client certificate and the authorities its handshake takes
    /// (`certificates::tls_for`). Never sent to the bridge, which reads no
    /// files of its own for anyone.
    #[serde(skip)]
    pub tls: crate::certificates::Tls,
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
    /// The URI starts with `{{NAME}}` and no enabled environment variable
    /// has that name (#50): said as that, not as the URL it leaves behind;
    /// `within`, the named environment it was sent in (#61).
    #[snafu(display("there is no enabled environment variable {name}{}", within.as_ref().map(|within| format!(" in {within}")).unwrap_or_default()))]
    Environment { name: String, within: Option<String> },
    /// A GraphQL body's variables, once their templates ran, are not a
    /// JSON object.
    #[snafu(display("the GraphQL variables cannot be read: {message}"))]
    GraphQlVariables { message: String },
    /// A gRPC call cannot be made: no method chosen, no definitions, a
    /// message they do not read.
    #[snafu(display("{message}"))]
    Grpc { message: String },
    /// R6: signing a JWT Bearer token failed (missing secret, bad JSON, bad PEM).
    #[snafu(display("{message}"))]
    Jwt { message: String },
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
    /// F14.10.
    pub http_version: HttpVersion,
    /// #60.
    pub proxy: &'a ProxyPrefs,
    /// Separates multipart fields; random, from the caller.
    pub boundary: &'a str,
    /// What the API takes from its folders and project (#66).
    pub inherited: &'a Inherited,
    /// The named environment it is sent in (#61), for what an error says:
    /// `variables` are already the ones it reads (`environments::effective`).
    pub environment: Option<&'a str>,
}

fn enabled(variables: &[Variable], category: VariableCategory) -> impl Iterator<Item = &Variable> {
    variables
        .iter()
        .filter(move |variable| variable.enabled && variable.category == category)
}

/// The project's global headers that are sent: enabled, with a name and a
/// value (#50). A row left without one is unfinished, not a header to put
/// on every request; a request's own header may be empty on purpose.
fn global_headers(variables: &[Variable]) -> impl Iterator<Item = &Variable> {
    enabled(variables, VariableCategory::GlobalRequestHeader)
        .filter(|variable| !variable.name.trim().is_empty() && !variable.value.trim().is_empty())
}

/// The names of the global headers a request goes with.
pub fn global_header_names(variables: &[Variable]) -> Vec<String> {
    global_headers(variables)
        .map(|variable| variable.name.trim().to_string())
        .collect()
}

/// The `Accept-Encoding` a request goes with when it names none: every
/// coding the engine reads.
pub const ACCEPT_ENCODING: &str = "gzip, deflate, br, zstd";

/// #58: what went out — `prepared`, the last hop's, with the headers the
/// engine adds as it does (`Host` over HTTP/1.1, `Accept-Encoding`,
/// `Content-Length`), each said where it came from. `globals` are the
/// project's global headers' names; `jar` says the cookies are the jar's;
/// `version` is the HTTP version it went over, empty when it never did.
pub fn sent_request(
    prepared: &PreparedRequest,
    request: &HttpRequest,
    inherited: &Inherited,
    globals: &[String],
    jar: bool,
    version: &str,
) -> SentRequest {
    let named = |list: &[KvParam], name: &str| {
        list.iter()
            .any(|kv| kv.enabled && kv.key.trim().eq_ignore_ascii_case(name))
    };
    let from = |name: &str| {
        if name.eq_ignore_ascii_case("cookie") && jar {
            HeaderSource::Cookie
        } else if named(&request.headers, name) {
            HeaderSource::Request
        } else if inherited
            .auth(&request.auth)
            .header_name(takes_query(request))
            .is_some_and(|signed| signed.eq_ignore_ascii_case(name))
            || (matches!(inherited.auth(&request.auth), crate::auth::Auth::AwsSigV4(_))
                && (name.eq_ignore_ascii_case("x-amz-security-token")
                    || name.eq_ignore_ascii_case("x-amz-date")
                    || name.eq_ignore_ascii_case("x-amz-content-sha256")))
        {
            HeaderSource::Auth
        } else if inherited
            .sent_headers()
            .any(|kv| kv.key.trim().eq_ignore_ascii_case(name))
        {
            HeaderSource::Folder
        } else if globals.iter().any(|global| global.eq_ignore_ascii_case(name)) {
            HeaderSource::Global
        } else if name.eq_ignore_ascii_case("content-type") {
            HeaderSource::Body
        } else {
            HeaderSource::Default
        }
    };
    let header = |name: &str, value: &str, from: HeaderSource| SentHeader {
        name: name.to_string(),
        value: value.to_string(),
        from,
    };
    let mut headers = Vec::new();
    let h2 = version == "HTTP/2" || (version.is_empty() && prepared.http_version == HttpVersion::Http2);
    if !h2
        && prepared.header("host").is_none()
        && let Ok(url) = url::Url::parse(&prepared.url)
    {
        let host = url.host_str().unwrap_or_default();
        let authority = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };
        headers.push(header("Host", &authority, HeaderSource::Default));
    }
    if prepared.header("accept-encoding").is_none() {
        headers.push(header("Accept-Encoding", ACCEPT_ENCODING, HeaderSource::Default));
    }
    headers.extend(
        prepared
            .headers
            .iter()
            .map(|(name, value)| header(name, value, from(name))),
    );
    if !prepared.body.is_empty() && prepared.header("content-length").is_none() {
        let length = prepared.body.len().to_string();
        headers.push(header("Content-Length", &length, HeaderSource::Default));
    }
    let start = &prepared.body[..prepared.body.len().min(SENT_BODY_LIMIT)];
    let body = match std::str::from_utf8(start) {
        Ok(text) => Some(text.to_string()),
        // Cut inside a character: the text before it.
        Err(error) if error.error_len().is_none() => {
            Some(String::from_utf8_lossy(&start[..error.valid_up_to()]).into_owned())
        }
        Err(_) => None,
    };
    SentRequest {
        method: prepared.method.as_str().to_string(),
        url: prepared.url.clone(),
        headers,
        body,
        body_size: prepared.body.len() as u64,
    }
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

fn raw_path_param<'a>(
    request: &'a HttpRequest,
    variables: &'a [Variable],
    inherited: &'a Inherited,
    custom: &'a [(String, String)],
    name: &str,
) -> Option<&'a str> {
    if let Some(v) = variables
        .iter()
        .find(|v| v.id.as_str().is_empty() && v.enabled && v.name == name && !v.value.is_empty())
    {
        return Some(v.value.as_str());
    }
    if let Some((_, val)) = custom.iter().rfind(|(key, val)| key == name && !val.is_empty()) {
        return Some(val.as_str());
    }
    if let Some(kv) = request
        .variables
        .iter()
        .find(|kv| kv.enabled && kv.key.trim() == name && !kv.value.is_empty())
    {
        return Some(kv.value.as_str());
    }
    if let Some(kv) = inherited
        .variables
        .iter()
        .rev()
        .find(|kv| kv.enabled && kv.key.trim() == name && !kv.value.is_empty())
    {
        return Some(kv.value.as_str());
    }
    variables
        .iter()
        .find(|v| {
            v.enabled
                && matches!(v.category, VariableCategory::Custom | VariableCategory::Environment)
                && v.name == name
                && !v.value.is_empty()
        })
        .map(|v| v.value.as_str())
}

/// F12: resolves the environment prefix and substitutes any `:param` or
/// `{param}` path placeholders from the request's variables (or inherited /
/// custom / environment variables).
fn resolve_path_uri(
    request: &HttpRequest,
    cx: &Context<'_>,
    render: impl Fn(&str) -> Result<String, PrepareError>,
) -> Result<String, PrepareError> {
    let base = resolve_uri(&request.uri, cx.variables);
    let names = path_param_names(&request.uri);
    if names.is_empty() {
        return Ok(base);
    }
    let mut rendered_values: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for name in names {
        if let Some(raw) = raw_path_param(request, cx.variables, cx.inherited, &cx.template.custom, &name) {
            let val = render(raw)?;
            if !val.is_empty() {
                rendered_values.insert(name, encode_uri_component(&val));
            }
        }
    }
    Ok(replace_path_params(&base, |name| rendered_values.get(name).cloned()))
}

/// F5.2: the request with the environment variable resolved and the
/// project's enabled global headers added after its own.
pub fn fill_values(request: &HttpRequest, variables: &[Variable]) -> HttpRequest {
    let mut filled = request.clone();
    filled.uri = resolve_uri(&request.uri, variables);
    filled.headers.extend(global_headers(variables).map(|variable| KvParam {
        key: variable.name.trim().to_string(),
        value: variable.value.clone(),
        enabled: true,
        ..Default::default()
    }));
    filled
}

/// The enabled custom variables, followed by enabled environment variables
/// whose names are not already listed.
pub fn custom_and_env_values(variables: &[Variable]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = enabled(variables, VariableCategory::Custom)
        .map(|v| (v.name.clone(), v.value.clone()))
        .collect();
    for v in enabled(variables, VariableCategory::Environment) {
        if !out.iter().any(|(name, _)| name == &v.name) {
            out.push((v.name.clone(), v.value.clone()));
        }
    }
    out
}

/// The texts whose template functions run: the body, or its form values;
/// query and header values (not keys, not the URI); the global headers,
/// the folders' and the auth it signs in with (#66).
fn templated_texts<'a>(
    request: &'a HttpRequest,
    variables: &'a [Variable],
    inherited: &'a Inherited,
) -> Vec<std::borrow::Cow<'a, str>> {
    let mut texts: Vec<std::borrow::Cow<'a, str>> = Vec::new();
    let grpc = request.method == HttpMethod::Grpc;
    if grpc {
        texts.push(strip_json_comments(&GrpcBody::read(&request.body).message).into());
    } else if request.content_type == ContentType::GraphQl {
        // A GET sends it too, as parameters.
        let graphql = GraphQlBody::read(&request.body);
        texts.push(graphql.query.into());
        texts.push(strip_json_comments(&graphql.variables).into());
    } else if request.method.has_body() {
        if request.content_type.is_key_value() {
            texts.extend(form_fields(&request.body).into_iter().map(|kv| kv.value.into()));
        } else if request.content_type == ContentType::Binary {
            if let Some(path) = request.body.trim().strip_prefix("file://") {
                texts.push(path.into());
            }
        } else {
            texts.push(strip_json_comments(&request.body).into());
        }
    }
    // A gRPC call sends no query.
    let lists = [&request.query, &request.headers];
    texts.extend(
        lists
            .into_iter()
            .skip(usize::from(grpc))
            .flatten()
            .filter(|kv| kv.enabled)
            .map(|kv| kv.value.as_str().into()),
    );
    if !grpc {
        let path_names = path_param_names(&request.uri);
        for name in path_names {
            if let Some(raw) = raw_path_param(request, variables, inherited, &[], &name) {
                texts.push(raw.into());
            }
        }
    }
    texts.extend(global_headers(variables).map(|v| v.value.as_str().into()));
    texts.extend(inherited.sent_headers().map(|kv| kv.value.as_str().into()));
    texts.extend(inherited.auth(&request.auth).templates().into_iter().map(Into::into));
    for v in variables.iter().filter(|v| {
        v.enabled
            && matches!(v.category, VariableCategory::Custom | VariableCategory::Environment)
            && !v.name.is_empty()
            && v.value.contains("{{")
    }) {
        if texts.iter().any(|t| t.contains("{{") && t.contains(&v.name)) {
            texts.push(v.value.as_str().into());
        }
    }
    texts
}

/// What the request's template functions need gathered before [`prepare`].
pub fn needs(request: &HttpRequest, variables: &[Variable], inherited: &Inherited) -> Needs {
    let mut needs = Needs::default();
    for text in templated_texts(request, variables, inherited) {
        needs.add(template::needs(&text));
    }
    // F9 / R1: an imported `file://` multipart field or binary body with no
    // path yet asks for one as `{{openFile()}}` does.
    if request.method != HttpMethod::Grpc
        && request.method.has_body()
        && ((request.content_type == ContentType::Multipart
            && form_fields(&request.body)
                .iter()
                .any(|kv| kv.enabled && !kv.key.is_empty() && kv.value.trim() == "file://"))
            || (request.content_type == ContentType::Binary && request.body.trim() == "file://"))
    {
        needs.picks_file = true;
    }
    needs
}

/// R7: finds variables referenced by `request` (leading URI `{{ENV}}`,
/// path params `:p`/`{p}`, `{{value(name)}}` including chains like
/// `{{b64.value(name)}}`, and bare `{{name}}` across URI, query, headers,
/// body, path param values, global/inherited headers, and effective auth)
/// that currently have no non-empty value from session `extracted`, API own,
/// folder, active environment, or shared variables, and are produced by another
/// API's enabled extraction.
///
/// `variables` must already be filtered for the active environment with
/// secrets filled (empty string when a secret has no value).
/// `producers` must be ordered by preference (nearest in tree first, excluding
/// the current API itself): `(api_id, api_name, enabled_extraction_names)`.
pub fn missing_extractions(
    request: &HttpRequest,
    variables: &[Variable],
    inherited: &Inherited,
    extracted: &[(String, String)],
    producers: &[(crate::ItemId, String, Vec<String>)],
) -> Vec<crate::assertions::MissingExtraction> {
    if producers.is_empty() {
        return Vec::new();
    }
    let any_missing_candidate = producers
        .iter()
        .flat_map(|(_, _, names)| names.iter())
        .any(|name| !extracted.iter().any(|(known, _)| known == name));
    if !any_missing_candidate {
        return Vec::new();
    }

    let has_value = |name: &str, env_only: bool| -> bool {
        if extracted.iter().any(|(known, _)| known == name) {
            return true;
        }
        if request
            .variables
            .iter()
            .any(|kv| kv.enabled && kv.key.trim() == name && !kv.value.is_empty())
        {
            return true;
        }
        if inherited
            .variables
            .iter()
            .any(|kv| kv.enabled && kv.key.trim() == name && !kv.value.is_empty())
        {
            return true;
        }
        variables.iter().any(|v| {
            v.enabled
                && v.name == name
                && !v.value.is_empty()
                && if env_only {
                    v.category == VariableCategory::Environment
                } else {
                    matches!(v.category, VariableCategory::Custom | VariableCategory::Environment)
                }
        })
    };

    let mut refs: Vec<(String, bool)> = Vec::new();
    let push_ref = |refs: &mut Vec<(String, bool)>, name: &str, env_only: bool| {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Some(existing) = refs.iter_mut().find(|(seen, _)| seen == name) {
            if !env_only {
                existing.1 = false;
            }
        } else {
            refs.push((name.to_string(), env_only));
        }
    };

    if let (Some(name), _) = split_env_prefix(request.uri.trim()) {
        push_ref(&mut refs, name, true);
    }
    for name in path_param_names(&request.uri) {
        push_ref(&mut refs, &name, false);
    }
    for text in templated_texts(request, variables, inherited) {
        let mut rest = text.as_ref();
        while let Some(start) = rest.find("{{") {
            let after = &rest[start + 2..];
            let Some(end) = after.get(1..).and_then(|tail| tail.find("}}")).map(|at| at + 1) else {
                break;
            };
            let inner = &after[..end];
            let whole = &rest[start..start + 2 + end + 2];
            if let Some(expr) = template::parse_inner(whole, inner) {
                if expr.chain.last().map(String::as_str) == Some("value")
                    && let Some(param) = expr.params.first()
                {
                    push_ref(&mut refs, param, false);
                }
            } else {
                let trimmed = inner.trim();
                if !trimmed.is_empty()
                    && !trimmed.starts_with('$')
                    && !trimmed.contains(['(', ')', '.', ' ', '\t', '\r', '\n', '{', '}'])
                {
                    push_ref(&mut refs, trimmed, false);
                }
            }
            rest = &rest[start + 2 + end + 2..];
        }
    }

    let mut out: Vec<crate::assertions::MissingExtraction> = Vec::new();
    for (name, env_only) in refs {
        if has_value(&name, env_only) {
            continue;
        }
        let Some((producer_id, producer_name, _)) = producers
            .iter()
            .find(|(_, _, names)| names.iter().any(|candidate| candidate == &name))
        else {
            continue;
        };
        if let Some(existing) = out.iter_mut().find(|entry| &entry.producer_id == producer_id) {
            if !existing.variables.contains(&name) {
                existing.variables.push(name);
            }
        } else {
            out.push(crate::assertions::MissingExtraction {
                producer_id: producer_id.clone(),
                producer_name: producer_name.clone(),
                variables: vec![name],
            });
        }
    }
    out
}

/// A7b: whether `request` or any header/auth/variable it sends calls a
/// dynamic template function (`uuid`, `random`, `randomString`, `now`,
/// `timestamp`, `ts`).
pub fn has_dynamic_templates(request: &HttpRequest, variables: &[Variable], inherited: &Inherited) -> bool {
    templated_texts(request, variables, inherited)
        .iter()
        .any(|text| template::has_dynamic(text))
}

/// A form or multipart body's fields; an unreadable list is none.
pub fn form_fields(body: &str) -> Vec<KvParam> {
    if body.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str(body).unwrap_or_default()
}

/// Whether the request's body is empty: blank text, or no field on.
pub fn body_is_blank(request: &HttpRequest) -> bool {
    if request.content_type.is_key_value() {
        form_fields(&request.body).iter().all(|kv| !kv.enabled)
    } else {
        request.body.trim().is_empty()
    }
}

/// A body the request keeps but does not send: one its method has no Body
/// tab for, as a cURL command's `-X OPTIONS -d …` (#62).
pub fn unsent_body(request: &HttpRequest) -> bool {
    !request.has_body_tab() && !body_is_blank(request)
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

/// [`encode_uri_component`] of bytes that need not be UTF-8 (BUG-27).
pub fn encode_uri_bytes(bytes: &[u8]) -> String {
    percent_encoding::percent_encode(bytes, URI_COMPONENT).to_string()
}

/// Whether non-comment line `at` ends with a `,` that only trails because
/// every non-blank line between it and the closing `}` or `]` after it is a
/// `//` comment.
fn has_commented_trailing_comma(lines: &[&str], at: usize) -> bool {
    if !lines[at].trim_end().ends_with(',') {
        return false;
    }
    let mut saw_comment = false;
    for &next in &lines[at + 1..] {
        let trimmed = next.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("//") {
            saw_comment = true;
            continue;
        }
        return saw_comment && (trimmed.starts_with('}') || trimmed.starts_with(']'));
    }
    false
}

/// The text a JSON-looking body is sent as: lines starting with `//` are
/// comments and go (4.3), along with a trailing comma left before `}` or `]`
/// when the final entry after it is commented out.
pub fn strip_json_comments(body: &str) -> String {
    let lines: Vec<&str> = body.split('\n').collect();
    let kept: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect();
    let first = kept.iter().find_map(|line| line.trim().bytes().next());
    let last = kept.iter().rev().find_map(|line| line.trim().bytes().next_back());
    let looks_json = matches!((first, last), (Some(b'{'), Some(b'}')) | (Some(b'['), Some(b']')));
    if !looks_json {
        return body.to_string();
    }
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
        .map(|(i, &line)| {
            if has_commented_trailing_comma(&lines, i) {
                let trimmed = line.trim_end_matches([' ', '\t', '\r']);
                let prefix = trimmed.strip_suffix(',').unwrap_or(trimmed);
                format!("{prefix}{}", &line[trimmed.len()..])
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Toggles `marker` (`"//"` for JSON, `"#"` for GraphQL) line comments across
/// the lines covered by `selected` in `text`, returning
/// `(replace_range, replacement, new_selection)`.
pub fn toggle_line_comment(
    text: &str,
    selected: std::ops::Range<usize>,
    marker: &str,
) -> (std::ops::Range<usize>, String, std::ops::Range<usize>) {
    struct Line<'a> {
        start: usize,
        end: usize,
        text: &'a str,
    }

    let start = selected.start.min(text.len());
    let end = selected.end.min(text.len()).max(start);
    let mut lines = Vec::new();
    let mut pos = 0;
    for raw in text.split('\n') {
        let line_end = pos + raw.len();
        lines.push(Line {
            start: pos,
            end: line_end,
            text: raw,
        });
        pos = line_end + 1;
    }
    if lines.is_empty() {
        lines.push(Line {
            start: 0,
            end: 0,
            text: "",
        });
    }

    let start_row = lines
        .iter()
        .position(|line| start <= line.end)
        .unwrap_or(lines.len() - 1);
    let mut end_row = lines.iter().position(|line| end <= line.end).unwrap_or(lines.len() - 1);
    let ended_at_next_line_start = end > start && end_row > start_row && end == lines[end_row].start;
    if ended_at_next_line_start {
        end_row -= 1;
    }

    let target = &lines[start_row..=end_row];
    let replace_range = target.first().map(|l| l.start).unwrap_or(0)..target.last().map(|l| l.end).unwrap_or(0);
    let has_non_blank = target.iter().any(|l| !l.text.trim().is_empty());
    let uncomment = has_non_blank
        && target
            .iter()
            .filter(|l| !l.text.trim().is_empty())
            .all(|l| l.text.trim_start().starts_with(marker));
    let min_indent = if !uncomment && has_non_blank {
        target
            .iter()
            .filter(|l| !l.text.trim().is_empty())
            .map(|l| l.text.len() - l.text.trim_start_matches([' ', '\t']).len())
            .min()
            .unwrap_or(0)
    } else {
        0
    };
    let marker_with_space = format!("{marker} ");
    let insert_len = marker_with_space.len();

    let mut replaced_lines = Vec::with_capacity(target.len());
    let mut new_start = start;
    let mut new_end = end;
    let mut running_offset = replace_range.start;

    for (idx, line) in target.iter().enumerate() {
        let content_len = line.text.trim_end_matches('\r').len();
        let cr = &line.text[content_len..];
        let content = &line.text[..content_len];
        let (new_line, indent, delta): (String, usize, isize) = if uncomment {
            let indent = content.len() - content.trim_start_matches([' ', '\t']).len();
            let after = &content[indent..];
            let removed = if after.starts_with(&marker_with_space) {
                marker_with_space.len()
            } else if after.starts_with(marker) {
                marker.len()
            } else {
                0
            };
            (
                format!("{}{}{cr}", &content[..indent], &content[indent + removed..]),
                indent,
                -(removed as isize),
            )
        } else if has_non_blank && content.trim().is_empty() {
            (line.text.to_string(), 0, 0)
        } else {
            let indent = if has_non_blank { min_indent } else { content.len() };
            (
                format!("{}{marker_with_space}{}{cr}", &content[..indent], &content[indent..]),
                indent,
                insert_len as isize,
            )
        };

        let map_col = |col: usize| -> usize {
            if delta >= 0 {
                let added = delta as usize;
                if col >= indent { col + added } else { col }
            } else {
                let removed = (-delta) as usize;
                if col <= indent {
                    col
                } else if col <= indent + removed {
                    indent
                } else {
                    col - removed
                }
            }
        };

        if idx == 0 {
            let col = start.saturating_sub(line.start);
            new_start = running_offset + map_col(col);
        }
        if idx + 1 == target.len() && !ended_at_next_line_start {
            let col = end.saturating_sub(line.start);
            new_end = running_offset + map_col(col);
        }
        running_offset += new_line.len() + 1;
        replaced_lines.push(new_line);
    }

    let replacement = replaced_lines.join("\n");
    if ended_at_next_line_start {
        new_end = replace_range.start + replacement.len() + 1;
    }
    (replace_range, replacement, new_start..new_end.max(new_start))
}

/// Toggles `//` line comments on the line(s) covered by `selected` in `text`.
pub fn toggle_json_comment(
    text: &str,
    selected: std::ops::Range<usize>,
) -> (std::ops::Range<usize>, String, std::ops::Range<usize>) {
    toggle_line_comment(text, selected, "//")
}

/// A JSON body indented as everywhere ([`crate::format::json`]), as the
/// editor's Format does it (F4.7). Comment lines go, as sending drops them; `{{…}}` templates stay
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
    let mut pretty = crate::format::json(&value);
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
    let lines: Vec<&str> = body.split('\n').collect();
    let mut text: String = lines
        .iter()
        .enumerate()
        .map(|(i, &line)| {
            if line.trim_start().starts_with("//") {
                String::new()
            } else if has_commented_trailing_comma(&lines, i) {
                let trimmed = line.trim_end_matches([' ', '\t', '\r']);
                let prefix = trimmed.strip_suffix(',').unwrap_or(trimmed);
                format!("{prefix} {}", &line[trimmed.len()..])
            } else {
                line.to_string()
            }
        })
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
    // Read, not kept: only whether it parses is asked.
    serde_json::from_str::<serde::de::IgnoredAny>(&text)
        .err()
        .map(|error| error.to_string())
}

/// A10: why a JSON body does not conform to its `request_schema` (F11), by
/// the first 3 JSON pointer places. `None` when the body is blank, still has
/// `{{…}}` templates, does not parse as JSON, the schema is blank or
/// invalid, or the body conforms.
pub fn request_schema_problem(body: &str, schema: &str) -> Option<String> {
    let body = body.trim();
    let schema = schema.trim();
    if body.is_empty() || schema.is_empty() || body.contains("{{") {
        return None;
    }
    let text = strip_json_comments(body);
    let doc: serde_json::Value = serde_json::from_str(&text).ok()?;
    let schema: serde_json::Value = serde_json::from_str(schema).ok()?;
    let validator = jsonschema::validator_for(&schema).ok()?;
    let places: Vec<String> = validator
        .iter_errors(&doc)
        .take(3)
        .map(|error| {
            let at = error.instance_path().to_string();
            let at = if at.is_empty() { "/".to_string() } else { at };
            format!("{at}: {error}")
        })
        .collect();
    (!places.is_empty()).then(|| places.join("; "))
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

fn rendered(text: &str, cx: &Context<'_>) -> Result<String, PrepareError> {
    template::render(text, cx.template, cx.files).context(TemplateSnafu)
}

/// #50: a URI that starts with an environment variable no enabled one is
/// named as.
fn check_environment(request: &HttpRequest, cx: &Context<'_>) -> Result<(), PrepareError> {
    match split_env_prefix(request.uri.trim()) {
        (Some(name), _)
            if !enabled(cx.variables, VariableCategory::Environment).any(|variable| variable.name == name) =>
        {
            Err(PrepareError::Environment {
                name: name.to_string(),
                within: cx.environment.map(str::to_string),
            })
        }
        _ => Ok(()),
    }
}

/// Whether a request has a query: a gRPC call has none, and takes an API
/// key as metadata.
fn takes_query(request: &HttpRequest) -> bool {
    request.method != HttpMethod::Grpc
}

/// The headers every request sends, each replacing a same-named one
/// before it (#3): the User-Agent, the project's global headers, its
/// folders' (the outermost's first), its auth (#66), the request's own.
/// Also returns whether JWT Bearer signed the `Authorization` header.
fn sent_headers(request: &HttpRequest, cx: &Context<'_>) -> Result<(Headers, bool), PrepareError> {
    let mut headers = Headers::default();
    headers.set("User-Agent", cx.user_agent);
    for variable in global_headers(cx.variables) {
        headers.set(variable.name.trim(), &rendered(&variable.value, cx)?);
    }
    for kv in cx.inherited.sent_headers() {
        headers.set(kv.key.trim(), &rendered(&kv.value, cx)?);
    }
    let auth = cx.inherited.auth(&request.auth);
    if let Some(Signed::Header { name, value }) = auth.sign(takes_query(request), |text| rendered(text, cx))? {
        headers.set(&name, &value);
    }
    let request_has_auth_header = request
        .headers
        .iter()
        .any(|kv| kv.enabled && kv.key.trim().eq_ignore_ascii_case("authorization"));
    let mut jwt_signed = false;
    if !request_has_auth_header
        && let crate::auth::Auth::JwtBearer(jwt) = auth
        && (jwt.place == crate::auth::KeyPlace::Header || !takes_query(request))
    {
        let login = jwt.login(|text| rendered(text, cx))?;
        let token = crate::jwt_bearer::sign(&login).map_err(|message| PrepareError::Jwt { message })?;
        headers.set("Authorization", &crate::jwt_bearer::header_value(&login.prefix, &token));
        jwt_signed = true;
    }
    for kv in request
        .headers
        .iter()
        .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
    {
        headers.set(kv.key.trim(), &rendered(&kv.value, cx)?);
    }
    Ok((headers, jwt_signed))
}

/// A gRPC call of `path` (`/package.Service/Method`) at the request's
/// address, its metadata, `body` the framed messages: an API's own call,
/// or the reflection one asking for its definitions.
pub fn grpc_call(
    request: &HttpRequest,
    cx: &Context<'_>,
    path: &str,
    body: Vec<u8>,
) -> Result<PreparedRequest, PrepareError> {
    check_environment(request, cx)?;
    let address = grpc::address_url(&resolve_uri(&request.uri, cx.variables));
    let mut url = url::Url::parse(&address).map_err(|error| PrepareError::Url {
        uri: address.clone(),
        message: error.to_string(),
    })?;
    let base = url.path().trim_end_matches('/').to_string();
    url.set_path(&format!("{base}{path}"));
    let (mut headers, jwt_bearer) = sent_headers(request, cx)?;
    headers.set("Content-Type", "application/grpc");
    headers.set("TE", "trailers");
    let oauth = oauth_login(request, cx, &headers)?;
    Ok(PreparedRequest {
        method: HttpMethod::Post,
        url: url.to_string(),
        headers: headers.0,
        body,
        timeouts: cx.timeouts.effective(),
        accept_invalid_certs: cx.accept_invalid_certs,
        http_version: HttpVersion::Http2,
        streams: false,
        proxy: cx.proxy.clone(),
        // gRPC servers ask for no Digest.
        digest: None,
        oauth,
        aws_sigv4: None,
        jwt_bearer,
        tls: Default::default(),
    })
}

/// F5.2–F5.4: everything the engine needs.
pub fn prepare(request: &HttpRequest, cx: &Context<'_>) -> Result<PreparedRequest, PrepareError> {
    let render = |text: &str| rendered(text, cx);

    if request.method == HttpMethod::Grpc {
        let grpc = GrpcBody::read(&request.body);
        let body = grpc
            .request(&render(&strip_json_comments(&grpc.message))?)
            .map_err(|message| PrepareError::Grpc { message })?;
        let mut call = grpc_call(request, cx, &grpc.path(), body)?;
        call.streams = grpc.server_streams();
        return Ok(call);
    }
    check_environment(request, cx)?;
    let socket = request.method == HttpMethod::WebSocket;
    let resolved_uri = resolve_path_uri(request, cx, render)?;
    // The URL, then the enabled query appended to what it already has.
    let uri = match socket {
        // E1: a WebSocket's address.
        true => crate::ws::address(&resolved_uri),
        false => with_scheme(&resolved_uri),
    };
    let mut url = url::Url::parse(&uri).map_err(|error| PrepareError::Url {
        uri: uri.clone(),
        message: error.to_string(),
    })?;
    for kv in request.query.iter().filter(|kv| kv.enabled) {
        url.query_pairs_mut().append_pair(&kv.key, &render(&kv.value)?);
    }
    // An API key in the query (#66), unless the request names it itself.
    if let Some(Signed::Query { name, value }) = cx.inherited.auth(&request.auth).sign(true, render)?
        && !url.query_pairs().any(|(key, _)| key == name)
    {
        url.query_pairs_mut().append_pair(&name, &value);
    }
    let mut jwt_bearer = false;
    if let crate::auth::Auth::JwtBearer(jwt) = cx.inherited.auth(&request.auth)
        && jwt.place == crate::auth::KeyPlace::Query
    {
        let login = jwt.login(render)?;
        let param = login.effective_query_param().to_string();
        if !url.query_pairs().any(|(key, _)| key == param) {
            let token = crate::jwt_bearer::sign(&login).map_err(|message| PrepareError::Jwt { message })?;
            url.query_pairs_mut().append_pair(&param, &token);
            jwt_bearer = true;
        }
    }
    // GraphQL over HTTP: a GET asks with parameters, a POST with a body.
    let graphql = (request.content_type == ContentType::GraphQl)
        .then(|| -> Result<GraphQlBody, PrepareError> {
            let stored = GraphQlBody::read(&request.body);
            Ok(GraphQlBody {
                query: render(&stored.query)?,
                variables: render(&strip_json_comments(&stored.variables))?,
                operation_name: stored.operation_name,
            })
        })
        .transpose()?;
    let graphql_failed = |message: String| PrepareError::GraphQlVariables { message };
    if let Some(graphql) = graphql.as_ref().filter(|_| !request.method.has_body()) {
        for (key, value) in graphql.query_params().map_err(graphql_failed)? {
            url.query_pairs_mut().append_pair(&key, &value);
        }
    }

    let (mut headers, jwt_header_signed) = sent_headers(request, cx)?;
    jwt_bearer |= jwt_header_signed;

    // F5.3: only POST, PUT, PATCH and DELETE carry a body and its type; a
    // request with an empty body sends no automatic Content-Type (#62, #83).
    let mut body = Vec::new();
    if request.method.has_body() && !body_is_blank(request) {
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
            ContentType::Binary => {
                let (bytes, mime) = binary_body(&request.body, cx, &render)?;
                body = bytes;
                mime
            }
            ContentType::GraphQl => {
                // A subscription is answered as an event stream by the
                // servers that answer it over HTTP (graphql-sse asks for it).
                if graphql.as_ref().is_some_and(GraphQlBody::is_subscription) {
                    headers.set_default("Accept", crate::sse::CONTENT_TYPE);
                }
                let payload = graphql
                    .as_ref()
                    .map(GraphQlBody::payload)
                    .transpose()
                    .map_err(graphql_failed)?;
                body = payload
                    .map(|payload| payload.to_string())
                    .unwrap_or_default()
                    .into_bytes();
                ContentType::Json.as_str().to_string()
            }
            _ => {
                body = render(&strip_json_comments(&request.body))?.into_bytes();
                request.content_type.as_str().to_string()
            }
        };
        if !body.is_empty() || request.content_type == ContentType::Binary {
            // A Content-Type the request names itself stays.
            headers.set_default("Content-Type", &content_type);
        }
    }
    // E1: a WebSocket's body is the message it sends next, not the
    // handshake's.
    if socket {
        body = render(&strip_json_comments(&request.body))?.into_bytes();
    }

    // C2: unless the request names its own Authorization (#3).
    let digest = match names_authorization(&headers) {
        true => None,
        false => cx.inherited.auth(&request.auth).digest_login(render)?,
    };
    let oauth = oauth_login(request, cx, &headers)?;
    // C4: AWS SigV4 signs as the final step once templates, URL, headers, and
    // body are finalized, unless the request names its own Authorization (#3).
    let aws_sigv4 = match names_authorization(&headers) {
        true => None,
        false => cx.inherited.auth(&request.auth).aws_sigv4_login(render)?,
    };
    if let Some(login) = &aws_sigv4 {
        for (name, value) in
            crate::aws_sigv4::sign(request.method.as_str(), &url, &headers.0, &body, login, cx.template.now)
        {
            headers.set(&name, &value);
        }
    }
    Ok(PreparedRequest {
        method: request.method,
        url: url.to_string(),
        headers: headers.0,
        body,
        timeouts: cx.timeouts.effective(),
        accept_invalid_certs: cx.accept_invalid_certs,
        // A WebSocket upgrades an HTTP/1.1 connection.
        http_version: if socket { HttpVersion::Http1 } else { cx.http_version },
        proxy: cx.proxy.clone(),
        streams: false,
        digest,
        oauth,
        aws_sigv4,
        jwt_bearer,
        // By host, as each hop goes (the service's).
        tls: Default::default(),
    })
}

fn names_authorization(headers: &Headers) -> bool {
    headers
        .0
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
}

/// C4: what OAuth's token is asked with, unless the request names its own
/// Authorization (#3).
fn oauth_login(
    request: &HttpRequest,
    cx: &Context<'_>,
    headers: &Headers,
) -> Result<Option<crate::oauth::OAuthLogin>, PrepareError> {
    match names_authorization(headers) {
        true => Ok(None),
        false => cx.inherited.auth(&request.auth).oauth_login(|text| rendered(text, cx)),
    }
}

/// F9b: a file a page picked. `webfile://<id>/<filename>`. The id is not a
/// filesystem path; the bytes stay in that page, and a synced request does
/// not carry them.
pub const PAGE_FILE: &str = "webfile://";

/// The value a page stores for a picked file. `name` is the file's own
/// name, with no directory.
pub fn page_file_marker(id: &str, name: &str) -> String {
    format!("{PAGE_FILE}{id}/{name}")
}

/// `(id, filename)` when `value` is a file a page picked.
pub fn page_file(value: &str) -> Option<(&str, &str)> {
    let rest = value.trim().strip_prefix(PAGE_FILE)?;
    let (id, name) = rest.split_once('/')?;
    (!id.is_empty() && !name.is_empty()).then_some((id, name))
}

/// A multipart field that sends a file: a desktop path, or a page's pick.
pub fn is_file_value(value: &str) -> bool {
    value.trim().starts_with("file://") || page_file(value).is_some()
}

/// R1: the display file name of a `ContentType::Binary` body (`file://` path
/// or `webfile://<id>/<name>` marker); `None` when no file is picked.
pub fn binary_file_name(body: &str) -> Option<&str> {
    if let Some((_, name)) = page_file(body) {
        return Some(name);
    }
    let trimmed = body.trim();
    let path = trimmed.strip_prefix("file://").unwrap_or(trimmed).trim();
    path.rsplit(['/', '\\']).next().filter(|name| !name.is_empty())
}

/// R1: the user-visible path (on desktop) or filename (for an in-page file)
/// of a `ContentType::Binary` body, without internal `file://` or `webfile://`
/// markers; `None` when empty or unpicked (`"file://"`).
pub fn binary_display_path(body: &str) -> Option<&str> {
    if let Some((_, name)) = page_file(body) {
        return Some(name);
    }
    let trimmed = body.trim();
    let path = trimmed.strip_prefix("file://").unwrap_or(trimmed).trim();
    (!path.is_empty()).then_some(path)
}

/// R1: default `Content-Type` guessed from the filename of a `ContentType::Binary`
/// body, falling back to `application/octet-stream`.
pub fn guessed_binary_content_type(body: &str) -> String {
    binary_file_name(body)
        .map(|name| mime_guess::from_path(name).first_or_octet_stream().to_string())
        .unwrap_or_else(|| ContentType::Binary.as_str().to_string())
}

/// R1: reads a `ContentType::Binary` body file (`webfile://` or `file://`)
/// and guesses its default MIME type from the file name.
fn binary_body(
    raw: &str,
    cx: &Context<'_>,
    render: &dyn Fn(&str) -> Result<String, PrepareError>,
) -> Result<(Vec<u8>, String), PrepareError> {
    if let Some((id, name)) = page_file(raw) {
        let bytes = cx
            .files
            .read(&format!("{PAGE_FILE}{id}"), None)
            .map_err(|message| PrepareError::Template {
                source: TemplateError::ReadFile {
                    file: name.to_string(),
                    message,
                },
            })?;
        let mime = mime_guess::from_path(name).first_or_octet_stream().to_string();
        return Ok((bytes, mime));
    }
    let trimmed = raw.trim();
    let raw_path = trimmed.strip_prefix("file://").unwrap_or(trimmed);
    let rendered = render(raw_path)?;
    let path = match rendered.trim() {
        "" => cx.template.answers.file.as_deref().unwrap_or_default(),
        other => other,
    };
    let bytes = cx.files.read(path, None).map_err(|message| PrepareError::Template {
        source: TemplateError::ReadFile {
            file: path.to_string(),
            message,
        },
    })?;
    let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
    Ok((bytes, mime))
}

/// F5.3: enabled fields with a name; a `file://` value is the file's bytes,
/// sent under its file name (the old app sent the whole local path). A
/// page's pick (`webfile://`) is the bytes that page is holding.
fn multipart(
    body: &str,
    cx: &Context<'_>,
    render: &dyn Fn(&str) -> Result<String, PrepareError>,
) -> Result<Vec<u8>, PrepareError> {
    let quote = |text: &str| text.replace('"', "%22").replace('\r', "%0D").replace('\n', "%0A");
    let mut out = Vec::new();
    for kv in form_fields(body).iter().filter(|kv| kv.enabled && !kv.key.is_empty()) {
        out.extend_from_slice(format!("--{}\r\n", cx.boundary).as_bytes());
        if let Some((id, name)) = page_file(&kv.value) {
            let bytes = cx
                .files
                .read(&format!("{PAGE_FILE}{id}"), None)
                .map_err(|message| PrepareError::Template {
                    source: TemplateError::ReadFile {
                        file: name.to_string(),
                        message,
                    },
                })?;
            let mime = mime_guess::from_path(name).first_or_octet_stream();
            out.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {mime}\r\n\r\n",
                    quote(&kv.key),
                    quote(name)
                )
                .as_bytes(),
            );
            out.extend_from_slice(&bytes);
        } else if let Some(path) = kv.value.strip_prefix("file://") {
            let rendered = render(path)?;
            let path = match rendered.trim() {
                "" => cx.template.answers.file.as_deref().unwrap_or_default(),
                other => other,
            };
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
        } else {
            out.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", quote(&kv.key)).as_bytes(),
            );
            out.extend_from_slice(render(&kv.value)?.as_bytes());
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
    use crate::auth::{Auth, KeyPlace};
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
            environment: None,
            created_at: Default::default(),
            updated_at: Default::default(),
        }
    }

    fn kv(key: &str, value: &str) -> KvParam {
        KvParam {
            key: key.into(),
            value: value.into(),
            enabled: true,
            ..Default::default()
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
        prepare_inheriting(request, variables, &Inherited::default())
    }

    fn prepare_inheriting(
        request: &HttpRequest,
        variables: &[Variable],
        inherited: &Inherited,
    ) -> Result<PreparedRequest, PrepareError> {
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
                http_version: HttpVersion::Auto,
                proxy: &ProxyPrefs::default(),
                boundary: "XYZ",
                inherited,
                environment: None,
            },
        )
    }

    #[test]
    fn json_is_formatted_with_its_templates_as_written() {
        let body =
            "{\"z\":1,\n// a note\n\"a\":{{value(x)}},\"c\":\"id-{{get(01J, token)}}\",\"q\":\"say \\\"{{hi}}\\\"\"}";
        assert_eq!(
            format_json(body),
            Ok("{\n    \"z\": 1,\n    \"a\": {{value(x)}},\n    \"c\": \"id-{{get(01J, token)}}\",\n    \"q\": \"say \\\"{{hi}}\\\"\"\n}".to_string())
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
        // Commenting out the final property leaves a comma on the line above,
        // which both validation and sending strip.
        let last_commented = "{\n  \"a\": 1,\n  // \"b\": 2\n}";
        assert_eq!(json_problem(last_commented), None);
        assert_eq!(strip_json_comments(last_commented), "{\n  \"a\": 1\n}");
    }

    #[test]
    fn toggling_json_comments_comments_and_uncomments_lines_while_keeping_selection() {
        let original = "{\n  \"a\": 1,\n  \"b\": 2\n}";
        // Caret on `"b"` on line 3 (offset 14..14).
        let (range, replacement, selection) = toggle_json_comment(original, 14..14);
        assert_eq!(range, 12..20);
        assert_eq!(replacement, "  // \"b\": 2");
        assert_eq!(selection, 17..17);

        let mut commented = original.to_string();
        commented.replace_range(range, &replacement);
        assert_eq!(commented, "{\n  \"a\": 1,\n  // \"b\": 2\n}");
        assert_eq!(strip_json_comments(&commented), "{\n  \"a\": 1\n}");
        assert_eq!(json_problem(&commented), None);

        // Toggle again at the new caret position restores the original line and caret.
        let (range, replacement, selection) = toggle_json_comment(&commented, selection);
        commented.replace_range(range, &replacement);
        assert_eq!(commented, original);
        assert_eq!(selection, 14..14);

        // Multi-line selection across both properties comments both at their shared indent.
        let (range, replacement, selection) = toggle_json_comment(original, 4..18);
        let mut both = original.to_string();
        both.replace_range(range, &replacement);
        assert_eq!(both, "{\n  // \"a\": 1,\n  // \"b\": 2\n}");
        assert_eq!(strip_json_comments(&both), "{\n}");
        let (range, replacement, _) = toggle_json_comment(&both, selection);
        both.replace_range(range, &replacement);
        assert_eq!(both, original);
    }

    #[test]
    fn a_json_body_is_checked_against_its_request_schema() {
        let schema = r#"{
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            }
        }"#;
        assert_eq!(request_schema_problem(r#"{"name": "Ada", "age": 30}"#, schema), None);
        assert_eq!(request_schema_problem(r#"{"name": "{{user}}"}"#, schema), None);
        let problem = request_schema_problem(r#"{"age": "thirty"}"#, schema).expect("schema mismatch");
        assert!(problem.contains("name"), "{problem}");
        assert!(problem.contains("/age"), "{problem}");
    }

    /// E1: a WebSocket goes to its `ws://` address over HTTP/1.1, signed
    /// in as any request, its body the message rendered and no body type.
    #[test]
    fn a_websocket_is_prepared_with_its_message() {
        let variables = [variable(
            VariableCategory::Environment,
            "HOST",
            "https://chat.test",
            true,
        )];
        let request = HttpRequest {
            method: HttpMethod::WebSocket,
            uri: "{{HOST}}/ws".into(),
            query: vec![kv("room", "lobby")],
            auth: Auth::Bearer { token: "t".into() },
            body: "{\"at\": {{ts()}}}".into(),
            ..Default::default()
        };
        let prepared = prepare_with(&request, &variables).expect("prepare");
        assert_eq!(prepared.url, "wss://chat.test/ws?room=lobby");
        assert_eq!(prepared.http_version, HttpVersion::Http1);
        assert_eq!(prepared.header("authorization"), Some("Bearer t"));
        assert_eq!(prepared.header("content-type"), None);
        assert_eq!(String::from_utf8_lossy(&prepared.body), "{\"at\": 7}");
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

        // No such variable: the URI stays as written, and sending says why.
        assert_eq!(resolve_uri("{{NOPE}}/x", &variables), "{{NOPE}}/x");
        let unknown = HttpRequest {
            uri: "{{NOPE}}/x".into(),
            ..Default::default()
        };
        assert_eq!(
            prepare_with(&unknown, &variables),
            Err(PrepareError::Environment {
                name: "NOPE".into(),
                within: None
            })
        );
        let disabled = [variable(VariableCategory::Environment, "HOST", "http://off", false)];
        let only_off = HttpRequest {
            uri: "{{HOST}}/x".into(),
            ..Default::default()
        };
        assert_eq!(
            prepare_with(&only_off, &disabled),
            Err(PrepareError::Environment {
                name: "HOST".into(),
                within: None
            }),
            "a disabled one is none"
        );
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
            // Unfinished rows (#50): no value, no name.
            variable(VariableCategory::GlobalRequestHeader, "X-Empty", " ", true),
            variable(VariableCategory::GlobalRequestHeader, " ", "orphan", true),
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
                    ..Default::default()
                },
            ],
            auth: Auth::Basic {
                username: "user".into(),
                password: "pass".into(),
            },
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

    /// #66: a token or a key is a template; a key goes in the header or
    /// the query it names, the request's own of the same name winning.
    /// #66: the folders' headers go after the global ones and before the
    /// request's own; inheriting signs in as above it.
    #[test]
    fn an_api_takes_its_folders_headers_and_auth() {
        let variables = [variable(
            VariableCategory::GlobalRequestHeader,
            "X-Team",
            "global",
            true,
        )];
        let inherited = Inherited {
            headers: vec![
                kv("X-Team", "outer"),
                kv("X-Team", "inner"),
                kv("X-Folder", "{{ts()}}"),
                KvParam {
                    key: "X-Off".into(),
                    value: "1".into(),
                    enabled: false,
                    ..Default::default()
                },
                kv("Authorization", "Basic folder-header"),
            ],
            auth: Auth::Bearer {
                token: "f{{ts()}}".into(),
            },
            ..Default::default()
        };
        let request = HttpRequest {
            uri: "http://a.test".into(),
            headers: vec![kv("X-Folder", "mine")],
            ..Default::default()
        };
        let prepared = prepare_inheriting(&request, &variables, &inherited).expect("prepare");
        assert_eq!(prepared.header("x-team"), Some("inner"));
        assert_eq!(prepared.header("x-folder"), Some("mine"), "the request's own wins");
        assert_eq!(prepared.header("x-off"), None);
        assert_eq!(
            prepared.header("authorization"),
            Some("Bearer f7"),
            "auth over a folder's header"
        );
        let own = HttpRequest {
            auth: Auth::None,
            ..request
        };
        let prepared = prepare_inheriting(&own, &variables, &inherited).expect("prepare");
        assert_eq!(
            prepared.header("authorization"),
            Some("Basic folder-header"),
            "none of its own: the folder's header stays"
        );
    }

    #[test]
    fn auth_signs_with_its_templates_rendered() {
        let bearer = HttpRequest {
            uri: "http://a.test".into(),
            auth: Auth::Bearer {
                token: "{{ts()}}".into(),
            },
            ..Default::default()
        };
        assert_eq!(
            prepare_with(&bearer, &[]).expect("prepare").header("authorization"),
            Some("Bearer 7")
        );
        let request = |place| HttpRequest {
            uri: "http://a.test/v1?x=1".into(),
            auth: Auth::ApiKey {
                name: "api_key".into(),
                value: "k{{ts()}}".into(),
                place,
            },
            ..Default::default()
        };
        let in_header = prepare_with(&request(KeyPlace::Header), &[]).expect("prepare");
        assert_eq!(in_header.header("api_key"), Some("k7"));
        assert_eq!(in_header.url, "http://a.test/v1?x=1");
        let in_query = prepare_with(&request(KeyPlace::Query), &[]).expect("prepare");
        assert_eq!(in_query.url, "http://a.test/v1?x=1&api_key=k7");
        assert_eq!(in_query.header("api_key"), None);
        let named = HttpRequest {
            query: vec![kv("api_key", "mine")],
            ..request(KeyPlace::Query)
        };
        assert_eq!(
            prepare_with(&named, &[]).expect("prepare").url,
            "http://a.test/v1?x=1&api_key=mine"
        );
    }

    #[test]
    fn bodies_go_only_with_post_put_patch_and_delete() {
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

        // #83: an empty POST body sends no automatic Content-Type, while an
        // explicit Content-Type header stays.
        let empty_post = HttpRequest {
            method: HttpMethod::Post,
            uri: "http://a.test".into(),
            content_type: ContentType::Json,
            body: String::new(),
            ..Default::default()
        };
        let prepared = prepare_with(&empty_post, &[]).expect("prepare");
        assert!(prepared.body.is_empty());
        assert_eq!(prepared.header("content-type"), None);
        let explicit = HttpRequest {
            headers: vec![kv("Content-Type", "application/json")],
            ..empty_post
        };
        assert_eq!(
            prepare_with(&explicit, &[]).expect("prepare").header("content-type"),
            Some("application/json")
        );
    }

    /// #58: what went out, each header said where it came from, with the
    /// ones the engine adds.
    #[test]
    fn the_sent_request_says_where_each_header_came_from() {
        let request = HttpRequest {
            method: HttpMethod::Post,
            headers: vec![kv("X-Trace", "{{uuid()}}")],
            auth: Auth::Basic {
                username: "u".into(),
                password: "p".into(),
            },
            ..Default::default()
        };
        let prepared = PreparedRequest {
            method: HttpMethod::Post,
            url: "http://a.test:8080/v1?x=1".into(),
            headers: vec![
                ("User-Agent".into(), "CyberAPI/test".into()),
                ("X-Tenant".into(), "t1".into()),
                ("X-Folder".into(), "f".into()),
                ("Authorization".into(), "Basic dTpw".into()),
                ("X-Trace".into(), "4f1e".into()),
                ("Content-Type".into(), "application/json".into()),
                ("Cookie".into(), "sid=1".into()),
            ],
            body: b"{\"a\": 1}".to_vec(),
            ..Default::default()
        };
        let inherited = Inherited {
            headers: vec![kv("x-folder", "f")],
            ..Default::default()
        };
        let sent = sent_request(&prepared, &request, &inherited, &["x-tenant".into()], true, "HTTP/1.1");
        let seen: Vec<(&str, &str, HeaderSource)> = sent
            .headers
            .iter()
            .map(|header| (header.name.as_str(), header.value.as_str(), header.from))
            .collect();
        assert_eq!(
            seen,
            [
                ("Host", "a.test:8080", HeaderSource::Default),
                ("Accept-Encoding", ACCEPT_ENCODING, HeaderSource::Default),
                ("User-Agent", "CyberAPI/test", HeaderSource::Default),
                ("X-Tenant", "t1", HeaderSource::Global),
                ("X-Folder", "f", HeaderSource::Folder),
                ("Authorization", "Basic dTpw", HeaderSource::Auth),
                ("X-Trace", "4f1e", HeaderSource::Request),
                ("Content-Type", "application/json", HeaderSource::Body),
                ("Cookie", "sid=1", HeaderSource::Cookie),
                ("Content-Length", "8", HeaderSource::Default),
            ]
        );
        assert_eq!(
            (sent.method.as_str(), sent.url.as_str()),
            ("POST", "http://a.test:8080/v1?x=1")
        );
        assert_eq!((sent.body.as_deref(), sent.body_size), (Some("{\"a\": 1}"), 8));

        // Over HTTP/2 there is no Host; a body that is not text is not kept
        // as text; a long one keeps its start.
        let binary = PreparedRequest {
            body: vec![0, 0, 0, 0, 3, 0xff, 0xfe, 1],
            ..prepared.clone()
        };
        let sent = sent_request(&binary, &request, &inherited, &[], false, "HTTP/2");
        assert!(sent.headers.iter().all(|header| header.name != "Host"));
        assert_eq!(
            sent.headers
                .iter()
                .find(|header| header.name == "Cookie")
                .map(|header| header.from),
            Some(HeaderSource::Default),
            "the request's own, not the jar's"
        );
        assert_eq!((sent.body, sent.body_size), (None, 8));
        let long = PreparedRequest {
            body: "é".repeat(SENT_BODY_LIMIT).into_bytes(),
            ..prepared
        };
        let sent = sent_request(&long, &request, &inherited, &[], false, "HTTP/1.1");
        assert_eq!(sent.body.map(|body| body.len()), Some(SENT_BODY_LIMIT));
        assert_eq!(sent.body_size, 2 * SENT_BODY_LIMIT as u64);
    }

    #[test]
    fn a_delete_sends_a_body_only_when_it_has_one() {
        let mut request = HttpRequest {
            method: HttpMethod::Delete,
            uri: "http://a.test".into(),
            body: "{\"ids\": [1, 2]}".into(),
            ..Default::default()
        };
        let prepared = prepare_with(&request, &[]).expect("prepare");
        assert_eq!(String::from_utf8_lossy(&prepared.body), "{\"ids\": [1, 2]}");
        assert_eq!(prepared.header("content-type"), Some("application/json"));
        assert!(!unsent_body(&request));

        // No body, no Content-Type: a DELETE as it went before.
        request.body = " \n".into();
        let prepared = prepare_with(&request, &[]).expect("prepare");
        assert!(prepared.body.is_empty());
        assert_eq!(prepared.header("content-type"), None);
        request.content_type = ContentType::Form;
        request.body = serde_json::to_string(&[KvParam {
            key: "a".into(),
            value: "1".into(),
            enabled: false,
            ..Default::default()
        }])
        .expect("json");
        assert_eq!(
            prepare_with(&request, &[]).expect("prepare").header("content-type"),
            None
        );

        // An OPTIONS keeps its body unsent, which can be said.
        request.method = HttpMethod::Options;
        request.content_type = ContentType::Json;
        request.body = "{}".into();
        assert!(prepare_with(&request, &[]).expect("prepare").body.is_empty());
        assert!(unsent_body(&request));
    }

    #[test]
    fn a_graphql_post_sends_json_and_a_get_sends_parameters() {
        let body = crate::graphql::GraphQlBody {
            query: "query User($id: ID!) { user(id: $id) { name } }".into(),
            variables: "{\n    // the stamp\n    \"id\": \"{{ts()}}\"\n}".into(),
            operation_name: "User".into(),
        };
        let mut request = HttpRequest {
            method: HttpMethod::Post,
            uri: "http://a.test/graphql".into(),
            content_type: ContentType::GraphQl,
            body: body.to_body(),
            ..Default::default()
        };
        let prepared = prepare_with(&request, &[]).expect("prepare");
        let sent: serde_json::Value = serde_json::from_slice(&prepared.body).expect("JSON");
        assert_eq!(
            sent,
            serde_json::json!({
                "query": "query User($id: ID!) { user(id: $id) { name } }",
                "variables": {"id": "7"},
                "operationName": "User"
            }),
            "templates run, comments go"
        );
        assert_eq!(prepared.header("content-type"), Some("application/json"));
        assert_eq!(prepared.header("accept"), None, "a query is answered with JSON");

        // A subscription asks for an event stream, unless the request
        // asks for something else.
        let subscription = HttpRequest {
            body: crate::graphql::GraphQlBody {
                query: "subscription { tick }".into(),
                ..Default::default()
            }
            .to_body(),
            ..request.clone()
        };
        let prepared = prepare_with(&subscription, &[]).expect("prepare");
        assert_eq!(prepared.header("accept"), Some("text/event-stream"));
        let own = HttpRequest {
            headers: vec![kv("Accept", "multipart/mixed")],
            ..subscription
        };
        let prepared = prepare_with(&own, &[]).expect("prepare");
        assert_eq!(prepared.header("accept"), Some("multipart/mixed"));

        request.method = HttpMethod::Get;
        let prepared = prepare_with(&request, &[]).expect("prepare");
        assert!(prepared.body.is_empty());
        let url = url::Url::parse(&prepared.url).expect("a URL");
        let params: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        assert_eq!(params[1], ("variables".to_string(), r#"{"id":"7"}"#.to_string()));
        assert_eq!(params[2], ("operationName".to_string(), "User".to_string()));

        request.method = HttpMethod::Post;
        request.body = crate::graphql::GraphQlBody {
            query: "{ a }".into(),
            variables: "{ broken".into(),
            ..Default::default()
        }
        .to_body();
        assert!(matches!(
            prepare_with(&request, &[]),
            Err(PrepareError::GraphQlVariables { .. })
        ));
    }

    #[test]
    fn form_fields_are_encoded_like_encode_uri_component() {
        let fields = vec![
            kv("a b", "1+1=2 & {{ts()}}"),
            KvParam {
                key: "off".into(),
                value: "x".into(),
                enabled: false,
                ..Default::default()
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

        // F9: a bare `file://` asks for a file before sending and takes the
        // picked path; templates in `file://…` paths also resolve.
        let unpicked = HttpRequest {
            body: serde_json::to_string(&[kv("avatar", "file://")]).expect("json"),
            ..request.clone()
        };
        assert!(needs(&unpicked, &[], &Inherited::default()).picks_file);
        let picked = prepare(
            &unpicked,
            &Context {
                variables: &[],
                template: &Inputs {
                    answers: template::Answers {
                        file: Some("/home/me/a.png".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                files: &OneFile,
                user_agent: "CyberAPI/test",
                timeouts: Timeouts::default(),
                accept_invalid_certs: false,
                http_version: HttpVersion::Auto,
                proxy: &ProxyPrefs::default(),
                boundary: "XYZ",
                inherited: &Inherited::default(),
                environment: None,
            },
        )
        .expect("picked file");
        assert!(String::from_utf8_lossy(&picked.body).contains(
            "Content-Disposition: form-data; name=\"avatar\"; filename=\"a.png\"\r\nContent-Type: image/png"
        ));

        // F9b: a page's pick is bytes held under an id, not a filesystem path,
        // and it does not ask for a path before sending.
        let page = HttpRequest {
            body: serde_json::to_string(&[kv("shot", "webfile://abc/photo.png")]).expect("json"),
            ..request.clone()
        };
        assert!(!needs(&page, &[], &Inherited::default()).picks_file);
        assert_eq!(page_file("webfile://abc/photo.png"), Some(("abc", "photo.png")));
        assert!(page_file("file:///tmp/a.png").is_none());
        struct Held;
        impl FileReader for Held {
            fn read(&self, file: &str, _: Option<template::BaseDir>) -> Result<Vec<u8>, String> {
                if file == "webfile://abc" {
                    Ok(b"png".to_vec())
                } else {
                    Err("missing".into())
                }
            }
        }
        let sent = prepare(
            &page,
            &Context {
                variables: &[],
                template: &Inputs::default(),
                files: &Held,
                user_agent: "CyberAPI/test",
                timeouts: Timeouts::default(),
                accept_invalid_certs: false,
                http_version: HttpVersion::Auto,
                proxy: &ProxyPrefs::default(),
                boundary: "XYZ",
                inherited: &Inherited::default(),
                environment: None,
            },
        )
        .expect("page file");
        let body = String::from_utf8_lossy(&sent.body);
        assert!(body.contains(
            "Content-Disposition: form-data; name=\"shot\"; filename=\"photo.png\"\r\nContent-Type: image/png\r\n\r\npng"
        ));
        assert!(!body.contains("webfile://"));

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
                    http_version: HttpVersion::Auto,
                    proxy: &ProxyPrefs::default(),
                    boundary: "B",
                    inherited: &Inherited::default(),
                    environment: None,
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

    /// A gRPC call goes as an HTTP/2 POST to its method's path at the
    /// address, the message encoded, the metadata its headers, no query.
    #[test]
    fn a_grpc_call_is_an_http2_post_of_its_encoded_message() {
        let variables = [variable(
            VariableCategory::Environment,
            "SHOP",
            "grpcs://shop.test",
            true,
        )];
        let call = crate::grpc::tests::body("Get");
        let request = HttpRequest {
            method: HttpMethod::Grpc,
            uri: "{{SHOP}}".into(),
            body: GrpcBody {
                message: r#"{"id": "{{timestamp()}}"}"#.into(),
                ..call.clone()
            }
            .to_body(),
            query: vec![kv("q", "1")],
            headers: vec![kv("authorization", "Bearer t"), kv("Content-Type", "text/plain")],
            ..Default::default()
        };
        let prepared = prepare_with(&request, &variables).expect("prepared");
        assert_eq!(prepared.method, HttpMethod::Post);
        assert_eq!(prepared.url, "https://shop.test/shop.Store/Get");
        assert_eq!(prepared.http_version, HttpVersion::Http2);
        assert_eq!(prepared.header("content-type"), Some("application/grpc"));
        assert_eq!(prepared.header("te"), Some("trailers"));
        assert_eq!(prepared.header("authorization"), Some("Bearer t"));
        assert_eq!(prepared.body, [0, 0, 0, 0, 3, 0x0a, 1, b'7'], "the template ran");
        assert_eq!(
            needs(&request, &variables, &Inherited::default()).responses,
            Vec::<String>::new()
        );

        let unknown = HttpRequest {
            body: GrpcBody {
                message: r#"{"nope": 1}"#.into(),
                ..call
            }
            .to_body(),
            ..request
        };
        assert!(matches!(
            prepare_with(&unknown, &variables),
            Err(PrepareError::Grpc { message }) if message.contains("shop.Ask")
        ));
    }

    #[test]
    fn needs_cover_body_query_headers_global_headers_and_auth() {
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
            auth: Auth::Bearer {
                token: "{{get(t, a)}}".into(),
            },
            ..Default::default()
        };
        let inherited = Inherited {
            headers: vec![kv("F", "{{get(f, a)}}")],
            ..Default::default()
        };
        let needs = needs(&request, &variables, &inherited);
        assert_eq!(needs.responses, ["b", "q", "g", "f", "t"]);
        assert!(needs.picks_file);
    }

    #[test]
    fn fill_values_is_what_the_old_app_sent_before_functions() {
        let variables = [
            variable(VariableCategory::Environment, "HOST", "https://h", true),
            variable(VariableCategory::GlobalRequestHeader, "X-G", "g", true),
            variable(VariableCategory::GlobalRequestHeader, "X-Empty", "", true),
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

    #[test]
    fn path_params_fall_back_when_blank_and_evaluate_templates_from_folder_or_run_values() {
        let request = HttpRequest {
            uri: "https://api.example.com/orders/:id/items/{sku}".into(),
            // `id` has an empty value row (as synced from URI), `sku` has a default.
            variables: vec![kv("id", ""), kv("sku", "req-sku")],
            ..Default::default()
        };
        let inherited = Inherited {
            variables: vec![kv("id", "{{timestamp()}}")],
            ..Default::default()
        };
        let values = inherited.values(&request.variables, &[("sku".into(), "run-sku".into())]);
        let vars = crate::run::with_values(&[], &values);
        let inputs = Inputs {
            now: 7,
            custom: vars
                .iter()
                .filter(|v| v.category == VariableCategory::Custom)
                .map(|v| (v.name.clone(), v.value.clone()))
                .collect(),
            ..Default::default()
        };
        let prepared = prepare(
            &request,
            &Context {
                variables: &vars,
                template: &inputs,
                files: &NoFiles("no files"),
                user_agent: "CyberAPI",
                timeouts: Timeouts::default(),
                accept_invalid_certs: false,
                http_version: HttpVersion::Auto,
                proxy: &ProxyPrefs::default(),
                boundary: "B",
                inherited: &inherited,
                environment: None,
            },
        )
        .expect("prepared");
        assert_eq!(prepared.url, "https://api.example.com/orders/7/items/run-sku");

        // Also verify direct Inputs.custom (without run::with_values) overrides request.variables
        let direct_inputs = Inputs {
            now: 9,
            custom: vec![("sku".into(), "direct-{{timestamp()}}".into())],
            ..Default::default()
        };
        let direct_prepared = prepare(
            &request,
            &Context {
                variables: &[],
                template: &direct_inputs,
                files: &NoFiles("no files"),
                user_agent: "CyberAPI",
                timeouts: Timeouts::default(),
                accept_invalid_certs: false,
                http_version: HttpVersion::Auto,
                proxy: &ProxyPrefs::default(),
                boundary: "B",
                inherited: &inherited,
                environment: None,
            },
        )
        .expect("direct prepared");
        assert_eq!(direct_prepared.url, "https://api.example.com/orders/9/items/direct-9");
    }

    #[test]
    fn binary_body_sends_file_bytes_with_guessed_or_overridden_content_type() {
        assert_eq!(binary_file_name("file:///tmp/photo.png"), Some("photo.png"));
        assert_eq!(binary_file_name("webfile://abc/report.pdf"), Some("report.pdf"));
        assert_eq!(binary_file_name("file://"), None);
        assert_eq!(binary_display_path("file:///tmp/photo.png"), Some("/tmp/photo.png"));
        assert_eq!(binary_display_path("webfile://abc/report.pdf"), Some("report.pdf"));
        assert_eq!(binary_display_path("file://"), None);
        assert_eq!(guessed_binary_content_type("file:///tmp/photo.png"), "image/png");
        assert_eq!(guessed_binary_content_type(""), "application/octet-stream");

        struct FakeFiles;
        impl FileReader for FakeFiles {
            fn read(&self, file: &str, _: Option<template::BaseDir>) -> Result<Vec<u8>, String> {
                match file {
                    "/tmp/photo.png" => Ok(vec![0x89, 0x50, 0x4e, 0x47]),
                    "/picked/data.bin" => Ok(vec![1, 2, 3]),
                    "webfile://abc" => Ok(b"%PDF-1.4".to_vec()),
                    _ => Err("missing file".into()),
                }
            }
        }
        let run_prepare = |req: &HttpRequest, answers: template::Answers| {
            let inputs = Inputs {
                answers,
                ..Default::default()
            };
            let proxy = ProxyPrefs::default();
            let inherited = Inherited::default();
            prepare(
                req,
                &Context {
                    variables: &[],
                    template: &inputs,
                    files: &FakeFiles,
                    user_agent: "CyberAPI/test",
                    timeouts: Timeouts::default(),
                    accept_invalid_certs: false,
                    http_version: HttpVersion::Auto,
                    proxy: &proxy,
                    boundary: "B",
                    inherited: &inherited,
                    environment: None,
                },
            )
        };

        let request = HttpRequest {
            method: HttpMethod::Post,
            uri: "https://api.test/upload".into(),
            content_type: ContentType::Binary,
            body: "file:///tmp/photo.png".into(),
            ..Default::default()
        };
        assert!(!needs(&request, &[], &Inherited::default()).picks_file);
        let prepared = run_prepare(&request, Default::default()).expect("prepared");
        assert_eq!(prepared.body, vec![0x89, 0x50, 0x4e, 0x47]);
        assert_eq!(prepared.header("content-type"), Some("image/png"));
        let sent = sent_request(&prepared, &request, &Inherited::default(), &[], false, "HTTP/2");
        assert_eq!(sent.body, None);
        assert_eq!(sent.body_size, 4);

        let custom = HttpRequest {
            headers: vec![kv("Content-Type", "image/webp")],
            ..request.clone()
        };
        let custom_prepared = run_prepare(&custom, Default::default()).expect("custom");
        assert_eq!(custom_prepared.header("content-type"), Some("image/webp"));

        let web = HttpRequest {
            body: "webfile://abc/report.pdf".into(),
            ..request.clone()
        };
        let web_prepared = run_prepare(&web, Default::default()).expect("web");
        assert_eq!(web_prepared.body, b"%PDF-1.4");
        assert_eq!(web_prepared.header("content-type"), Some("application/pdf"));

        let unpicked = HttpRequest {
            body: "file://".into(),
            ..request.clone()
        };
        assert!(needs(&unpicked, &[], &Inherited::default()).picks_file);
        let picked = run_prepare(
            &unpicked,
            template::Answers {
                file: Some("/picked/data.bin".into()),
                ..Default::default()
            },
        )
        .expect("picked");
        assert_eq!(picked.body, vec![1, 2, 3]);
        assert_eq!(picked.header("content-type"), Some("application/octet-stream"));

        let missing = HttpRequest {
            body: "file:///nope.bin".into(),
            ..request
        };
        assert_eq!(
            run_prepare(&missing, Default::default()),
            Err(PrepareError::Template {
                source: TemplateError::ReadFile {
                    file: "/nope.bin".into(),
                    message: "missing file".into(),
                }
            })
        );
    }

    #[test]
    fn missing_extractions_detects_unresolved_references_and_clears_when_provided() {
        let login_id = crate::ItemId::new("login");
        let refresh_id = crate::ItemId::new("refresh");
        let producers = vec![
            (
                login_id.clone(),
                "Login".to_string(),
                vec!["token".to_string(), "baseUrl".to_string(), "userId".to_string()],
            ),
            (
                refresh_id.clone(),
                "Refresh".to_string(),
                vec!["refreshToken".to_string()],
            ),
        ];
        let req = HttpRequest {
            method: HttpMethod::Post,
            uri: "{{baseUrl}}/users/:userId".into(),
            headers: vec![kv("X-Refresh", "{{value(refreshToken)}}")],
            body: r#"{"sig": "{{b64.value(sig)}}", "id": "{{$randomUUID}}", "now": "{{timestamp()}}"}"#.into(),
            auth: Auth::Bearer {
                token: "{{token}}".into(),
            },
            ..Default::default()
        };

        let found = missing_extractions(&req, &[], &Inherited::default(), &[], &producers);
        assert_eq!(
            found,
            vec![
                crate::assertions::MissingExtraction {
                    producer_id: login_id.clone(),
                    producer_name: "Login".into(),
                    variables: vec!["baseUrl".into(), "userId".into(), "token".into()],
                },
                crate::assertions::MissingExtraction {
                    producer_id: refresh_id.clone(),
                    producer_name: "Refresh".into(),
                    variables: vec!["refreshToken".into()],
                },
            ]
        );

        // Empty placeholder variables in the project do not hide the prompt,
        // while non-empty variables or extracted values suppress it.
        let var = |category, name: &str, value: &str| Variable {
            id: crate::VariableId::new(name),
            collection: crate::CollectionId::new("col"),
            category,
            name: name.into(),
            value: value.into(),
            enabled: true,
            environment: None,
            created_at: crate::Timestamp::default(),
            updated_at: crate::Timestamp::default(),
        };
        let empty_env = var(VariableCategory::Custom, "token", "");
        let non_empty_env = var(VariableCategory::Environment, "baseUrl", "https://api.example.com");
        let partially_satisfied = missing_extractions(
            &req,
            &[empty_env, non_empty_env],
            &Inherited::default(),
            &[("userId".into(), "42".into()), ("refreshToken".into(), "r1".into())],
            &producers,
        );
        assert_eq!(
            partially_satisfied,
            vec![crate::assertions::MissingExtraction {
                producer_id: login_id.clone(),
                producer_name: "Login".into(),
                variables: vec!["token".into()],
            }]
        );

        // Chained {{b64.value(...)}} is detected too when a producer owns it.
        let chain_req = HttpRequest {
            method: HttpMethod::Get,
            uri: "https://api.example.com/me".into(),
            headers: vec![kv("X-Sig", "{{b64.value(token)}}")],
            ..Default::default()
        };
        assert_eq!(
            missing_extractions(&chain_req, &[], &Inherited::default(), &[], &producers),
            vec![crate::assertions::MissingExtraction {
                producer_id: login_id,
                producer_name: "Login".into(),
                variables: vec!["token".into()],
            }]
        );
        assert!(
            missing_extractions(
                &chain_req,
                &[],
                &Inherited::default(),
                &[("token".into(), "abc".into())],
                &producers
            )
            .is_empty()
        );
    }

    #[test]
    fn jwt_bearer_signs_header_and_query_with_templates_and_respects_overrides() {
        use crate::auth::{Auth, KeyPlace};
        use crate::jwt_bearer::{JwtAlgorithm, JwtBearer};
        use jsonwebtoken::{DecodingKey, Validation, decode};

        let request = HttpRequest {
            method: HttpMethod::Post,
            uri: "https://api.test/orders".into(),
            auth: Auth::from(JwtBearer {
                algorithm: JwtAlgorithm::HS256,
                secret: "{{jwt_secret}}".into(),
                secret_base64: false,
                payload: r#"{"sub":"{{user}}","iat":{{timestamp()}}}"#.into(),
                header: r#"{"kid":"k1"}"#.into(),
                place: KeyPlace::Header,
                prefix: "JWT".into(),
                query_param: "token".into(),
            }),
            ..Default::default()
        };
        let inputs = Inputs {
            now: 7,
            custom: vec![
                ("jwt_secret".into(), "super-secret".into()),
                ("user".into(), "alice".into()),
            ],
            ..Default::default()
        };
        let run_prepare = |req: &HttpRequest| {
            let proxy = ProxyPrefs::default();
            let inherited = Inherited::default();
            prepare(
                req,
                &Context {
                    variables: &[],
                    template: &inputs,
                    files: &OneFile,
                    user_agent: "CyberAPI/test",
                    timeouts: Timeouts::default(),
                    accept_invalid_certs: false,
                    http_version: HttpVersion::Auto,
                    proxy: &proxy,
                    boundary: "XYZ",
                    inherited: &inherited,
                    environment: None,
                },
            )
        };
        let prepared = run_prepare(&request).expect("prepared");
        assert!(prepared.jwt_bearer);
        let auth_header = prepared.header("authorization").expect("Authorization");
        let token = auth_header.strip_prefix("JWT ").expect("JWT prefix");
        let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = false;
        validation.required_spec_claims.clear();
        let dec_key = DecodingKey::from_secret(b"super-secret").expect("secret key");
        let decoded = decode::<serde_json::Value>(token, &dec_key, &validation).expect("valid jwt");
        assert_eq!(decoded.header.kid.as_deref(), Some("k1"));
        assert_eq!(decoded.claims["sub"], serde_json::json!("alice"));
        assert_eq!(decoded.claims["iat"], serde_json::json!(7));

        // Query placement with empty secret errors cleanly; query placement with secret appends query param.
        let empty_query_req = HttpRequest {
            auth: Auth::from(JwtBearer {
                place: KeyPlace::Query,
                query_param: "access_token".into(),
                ..Default::default()
            }),
            ..request.clone()
        };
        assert!(matches!(run_prepare(&empty_query_req), Err(PrepareError::Jwt { .. })));

        let query_req = HttpRequest {
            auth: Auth::from(JwtBearer {
                algorithm: JwtAlgorithm::HS256,
                secret: "super-secret".into(),
                payload: r#"{"scope":"read"}"#.into(),
                place: KeyPlace::Query,
                query_param: "access_token".into(),
                ..Default::default()
            }),
            ..request.clone()
        };
        let query_prepared = run_prepare(&query_req).expect("query prepared");
        assert!(query_prepared.jwt_bearer);
        assert!(query_prepared.header("authorization").is_none());
        let parsed = url::Url::parse(&query_prepared.url).expect("url");
        let (_, q_tok) = parsed
            .query_pairs()
            .find(|(k, _)| k == "access_token")
            .expect("query jwt");
        let dec_key = DecodingKey::from_secret(b"super-secret").expect("secret key");
        let decoded_q = decode::<serde_json::Value>(q_tok.as_ref(), &dec_key, &validation).expect("valid query jwt");
        assert_eq!(decoded_q.claims["scope"], serde_json::json!("read"));

        // Explicit Authorization header on the request wins (#3) without failing on missing secret.
        let override_req = HttpRequest {
            headers: vec![kv("Authorization", "Bearer manual")],
            auth: Auth::from(JwtBearer::default()),
            ..request
        };
        let override_prepared = run_prepare(&override_req).expect("override ok");
        assert!(!override_prepared.jwt_bearer);
        assert_eq!(override_prepared.header("authorization"), Some("Bearer manual"));
    }
}
