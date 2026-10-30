//! B8: a `.http` file — VS Code's REST Client, JetBrains' HTTP Client — as
//! APIs, and their environment files as environments (F10.3).
//!
//! Requests are parted by `###` lines; the text after `###` names the one
//! after it, as a `# @name` comment does (`//` comments too). Before a
//! request, `@name = value` sets a file variable: these are the variables
//! every environment shares. A request is its line — `METHOD URL
//! [HTTP/x]`, a URL alone being a GET, its query going on over indented `?`
//! and `&` lines — its headers up to a blank line, and its body. The
//! comments over it describe it (F3); `# @no-redirect` does not follow
//! redirects (#71); `# @prompt name label` asks for `{{name}}` as it is sent
//! (D1). A body that reads a file (`< ./a.json`) stays as written; a
//! response handler (`> {% %}`, `> a.js`), where the response is written
//! (`>> a.json`) and an earlier response to compare (`<> a.json`) are left
//! out. A form's body is its fields, a multipart one's parts (a part that
//! reads a file is that file). GraphQL — `GRAPHQL url`, or REST Client's
//! `X-REQUEST-TYPE: GraphQL` — is a GraphQL body: the query, then its
//! variables after a blank line. `WEBSOCKET url` is a WebSocket, its first
//! message the body.
//!
//! The system variables this app has a function for become that function
//! ([`system`]); any other `{{$…}}` stays as written, as `{{name}}` does,
//! which is this app's own way only at the start of the address.
//!
//! An environment file — JetBrains' `http-client.env.json`, or its
//! `.private` one, or REST Client's `rest-client.environmentVariables`
//! setting — is an object of environments, each an object of variables:
//! `$shared` is the variables every environment shares, the others named
//! environments. A private file's values are no more secret here than the
//! public one's: the two look alike.

use super::{ImportBundle, ImportEnvironment, ImportItem, api, content_type_of, guess_content_type, kv};
use crate::graphql::GraphQlBody;
use crate::model::{ContentType, HttpMethod, KvParam};
use crate::prefs::HttpVersion;
use serde_json::{Map, Value};

const METHODS: [&str; 11] = [
    "GET",
    "POST",
    "PUT",
    "DELETE",
    "PATCH",
    "HEAD",
    "OPTIONS",
    "TRACE",
    "CONNECT",
    "GRAPHQL",
    "WEBSOCKET",
];

/// How a bare address starts: anything else needs a method before it.
const ADDRESS_STARTS: [&str; 7] = ["http://", "https://", "ws://", "wss://", "{{", "/", "localhost"];

struct RequestLine<'a> {
    method: &'a str,
    url: &'a str,
    version: Option<HttpVersion>,
    /// Whether it names its method: a line with only an address is a
    /// request only where the rest of the text says it is a `.http` file.
    named: bool,
}

/// `METHOD URL [HTTP/x]`, or a URL alone.
fn request_line(line: &str) -> Option<RequestLine<'_>> {
    let line = line.trim();
    let first = line.split_whitespace().next()?;
    let method = METHODS
        .iter()
        .find(|method| method.eq_ignore_ascii_case(first))
        .copied();
    let rest = match method {
        Some(_) => line[first.len()..].trim(),
        None => line,
    };
    let (url, version) = match rest.rsplit_once(char::is_whitespace) {
        Some((url, version)) if version.to_ascii_uppercase().starts_with("HTTP/") => {
            let version = version[5..].starts_with('2').then_some(HttpVersion::Http2);
            (url.trim(), version)
        }
        _ => (rest, None),
    };
    if url.is_empty() || url.contains(char::is_whitespace) {
        return None;
    }
    let address = ADDRESS_STARTS.iter().any(|start| {
        url.get(..start.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(start))
    });
    if method.is_none() && !address {
        return None;
    }
    Some(RequestLine {
        method: method.unwrap_or("GET"),
        url,
        version,
        named: method.is_some(),
    })
}

/// A comment's text: `# …` or `// …`, but not a `###` line.
fn comment(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.starts_with("###") {
        return None;
    }
    line.strip_prefix("//")
        .or_else(|| line.strip_prefix('#'))
        .map(str::trim)
}

/// `@name = value`: a file variable.
fn variable(line: &str) -> Option<(&str, &str)> {
    let (name, value) = line.trim().strip_prefix('@')?.split_once('=')?;
    let name = name.trim();
    (!name.is_empty() && !name.contains(char::is_whitespace)).then_some((name, value.trim()))
}

/// `@name rest` or `@name = rest` in a comment: the directive and the rest.
fn directive(comment: &str) -> Option<(&str, &str)> {
    let after = comment.strip_prefix('@')?;
    let end = after
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(after.len());
    let rest = after[end..].trim_start().trim_start_matches('=').trim();
    Some((&after[..end], rest))
}

/// Whether `text` is a `.http` file: its first request comes before
/// anything else but comments and variables, and names its method — or
/// the file has a `###` line or a variable to say it is one.
pub(super) fn is_http(text: &str) -> bool {
    let marked = text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("###") || variable(line).is_some()
    });
    let lines: Vec<&str> = text.lines().collect();
    let mut at = 0;
    while let Some(line) = lines.get(at) {
        let line = line.trim();
        if line.is_empty() || line.starts_with("###") || comment(line).is_some() || variable(line).is_some() {
            at += 1;
            continue;
        }
        if let Some(next) = after_script(&lines, at) {
            at = next;
            continue;
        }
        return request_line(line).is_some_and(|request| request.named || marked);
    }
    false
}

/// Where what follows a pre-request script that starts at `lines[at]` is:
/// `< {% … %}`, to the line that closes it, or `< file.js`. `None` when
/// that line starts none. JetBrains runs them before the request; they are
/// left out, as the handlers after a response are (BUG-16).
fn after_script(lines: &[&str], at: usize) -> Option<usize> {
    let after = lines.get(at)?.trim_start().strip_prefix('<')?;
    let rest = after.trim_start();
    if let Some(script) = rest.strip_prefix("{%") {
        if script.contains("%}") {
            return Some(at + 1);
        }
        let closed = lines[at + 1..].iter().position(|line| line.contains("%}"));
        return Some(closed.map_or(lines.len(), |end| at + 1 + end + 1));
    }
    // A file's name comes after a space: `<?xml` is not one.
    (after.starts_with(char::is_whitespace) && !rest.is_empty()).then_some(at + 1)
}

/// What a system variable is in this app's functions; `None` for one it
/// has none for. `{{$randomInt a b}}` leaves out `b`, `random` takes both.
fn system(variable: &str) -> Option<String> {
    let mut words = variable.split_whitespace();
    let name = words.next()?;
    let args: Vec<&str> = words.collect();
    // `-1 d` as `now`'s shift; REST Client's years, months and
    // milliseconds are not one.
    let shift = |args: &[&str]| -> Option<String> {
        match args {
            [] => Some(String::new()),
            [count, unit] if count.parse::<i64>().is_ok() && ["s", "m", "h", "d", "w"].contains(unit) => {
                Some(format!(", {count}{unit}"))
            }
            _ => None,
        }
    };
    let call = match (name, args.as_slice()) {
        ("$guid" | "$uuid" | "$random.uuid", []) => "uuid()".to_string(),
        ("$timestamp", []) => "timestamp()".to_string(),
        ("$timestamp", offset) => format!("now(%s{})", shift(offset)?),
        ("$isoTimestamp", []) => "now(iso, utc)".to_string(),
        ("$datetime", ["iso8601", offset @ ..]) => format!("now(iso{}, utc)", shift(offset)?),
        ("$localDatetime", ["iso8601", offset @ ..]) => format!("now(iso{})", shift(offset)?),
        ("$randomInt", []) => "random(0, 1000)".to_string(),
        ("$randomInt", [low, high]) => {
            let (low, high) = (low.parse::<i64>().ok()?, high.parse::<i64>().ok()?);
            format!("random({low}, {})", high - 1)
        }
        ("$processEnv", [name]) if !name.starts_with('%') => format!("env({name})"),
        _ => {
            // `$random.integer(1, 5)` has a space in it.
            let call = |prefix: &str| variable.strip_prefix(prefix)?.strip_suffix(')');
            if let Some(name) = name.strip_prefix("$env.").filter(|_| args.is_empty()) {
                format!("env({name})")
            } else if let Some(count) = call("$random.alphanumeric(") {
                format!("randomString({})", count.trim())
            } else {
                let (low, high) = call("$random.integer(")?.split_once(',')?;
                let (low, high) = (low.trim().parse::<i64>().ok()?, high.trim().parse::<i64>().ok()?);
                format!("random({low}, {})", high - 1)
            }
        }
    };
    Some(format!("{{{{{call}}}}}"))
}

/// `text` with its system variables as functions and its `@prompt`
/// variables (name, label) asked for.
fn rewrite(text: &str, prompts: &[(String, String)]) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break;
        };
        out.push_str(&rest[..start]);
        let inside = after[..end].trim();
        let asked = prompts
            .iter()
            .find(|(name, _)| name == inside)
            .map(|(_, label)| format!("{{{{prompt({label})}}}}"));
        match asked.or_else(|| system(inside)) {
            Some(call) => out.push_str(&call),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// A `Content-Type` value's parameter: `boundary=x`.
fn parameter(value: &str, name: &str) -> Option<String> {
    value.split(';').skip(1).find_map(|part| {
        let (key, value) = part.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().trim_matches('"').to_string())
    })
}

/// A multipart body's parts as fields; a part that reads a file is that
/// file. `None` when it has none.
fn multipart(body: &str, boundary: &str) -> Option<Vec<KvParam>> {
    let delimiter = format!("--{boundary}");
    let mut fields = Vec::new();
    for part in body.split(delimiter.as_str()).skip(1) {
        if part.starts_with("--") {
            break;
        }
        let part = part.trim_start_matches(['\r', '\n']);
        let Some((head, content)) = part.split_once("\n\n") else {
            continue;
        };
        let name = head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            if !key.trim().eq_ignore_ascii_case("content-disposition") {
                return None;
            }
            parameter(&format!("x;{value}"), "name")
        });
        let Some(name) = name else {
            continue;
        };
        let content = content.trim_end_matches(['\r', '\n']);
        // A file is `< path` on its line, a space after the `<`: `<?xml`,
        // `<html>` are text (BUG-25).
        let file = content
            .trim()
            .strip_prefix('<')
            .filter(|path| path.starts_with(char::is_whitespace) && !path.contains('\n'))
            .map(str::trim)
            .filter(|path| !path.is_empty());
        let value = match file {
            Some(path) => format!("file://{path}"),
            None => content.to_string(),
        };
        fields.push(kv(&name, &value, true));
    }
    (!fields.is_empty()).then_some(fields)
}

/// A GraphQL body: the query, and the variables after a blank line — an
/// object whose first key is quoted, which a selection set's is not.
fn graphql(body: &str) -> GraphQlBody {
    let mut at = 0;
    while let Some(found) = body[at..].find("\n\n") {
        let split = at + found;
        let variables = body[split..].trim();
        let object = variables
            .strip_prefix('{')
            .is_some_and(|inside| inside.trim_start().starts_with(['"', '}']));
        if object {
            return GraphQlBody {
                query: body[..split].trim_end().to_string(),
                variables: variables.to_string(),
                operation_name: String::new(),
            };
        }
        at = split + 2;
    }
    GraphQlBody {
        query: body.to_string(),
        ..Default::default()
    }
}

/// The blocks between `###` lines: the name after the `###`, and the lines.
fn blocks(text: &str) -> Vec<(&str, Vec<&str>)> {
    let mut blocks = vec![("", Vec::new())];
    for line in text.lines() {
        match line.trim_start().strip_prefix("###") {
            Some(title) => blocks.push((title.trim(), Vec::new())),
            None => blocks.last_mut().expect("one at least").1.push(line),
        }
    }
    blocks
}

/// Whether a body's line is where what is not the body starts: a response
/// handler, where the response goes, a response to compare.
fn after_body(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("> ") || line.starts_with(">{%") || line.starts_with(">>") || line.starts_with("<> ")
}

/// A block's request, and the file variables before it.
fn request(title: &str, lines: &[&str], environment: &mut Vec<(String, String)>) -> Option<ImportItem> {
    let mut name = title.to_string();
    let mut notes: Vec<&str> = Vec::new();
    let mut prompts: Vec<(String, String)> = Vec::new();
    let mut no_redirect = false;
    let mut at = 0;
    while let Some(line) = lines.get(at) {
        if line.trim().is_empty() {
            at += 1;
            continue;
        }
        if let Some(comment) = comment(line) {
            match directive(comment) {
                Some(("name", given)) if !given.is_empty() => name = given.to_string(),
                Some(("no-redirect", _)) => no_redirect = true,
                Some(("prompt", rest)) => {
                    let (variable, label) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                    let label = match label.trim() {
                        "" => variable,
                        label => label,
                    };
                    // A label is one of `prompt`'s parameters.
                    let label: String = label
                        .chars()
                        .map(|c| match c {
                            ',' => '，',
                            '(' => '（',
                            ')' => '）',
                            other => other,
                        })
                        .collect();
                    if !variable.is_empty() {
                        prompts.push((variable.to_string(), label));
                    }
                }
                Some(_) => {}
                None if !comment.is_empty() => notes.push(comment),
                None => {}
            }
            at += 1;
            continue;
        }
        if let Some((key, value)) = variable(line) {
            environment.retain(|(known, _)| known != key);
            environment.push((key.to_string(), value.to_string()));
            at += 1;
            continue;
        }
        if let Some(next) = after_script(lines, at) {
            at = next;
            continue;
        }
        break;
    }
    let line = request_line(lines.get(at)?)?;
    at += 1;
    let mut url = line.url.to_string();
    while let Some(next) = lines.get(at) {
        let trimmed = next.trim_start();
        if next.starts_with(char::is_whitespace) && trimmed.starts_with(['?', '&']) {
            url.push_str(trimmed.trim_end());
            at += 1;
        } else {
            break;
        }
    }
    let mut headers: Vec<KvParam> = Vec::new();
    while let Some(next) = lines.get(at) {
        let trimmed = next.trim();
        if trimmed.is_empty() {
            at += 1;
            break;
        }
        if after_body(trimmed) {
            break;
        }
        if comment(trimmed).is_some() {
            at += 1;
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            break;
        };
        headers.push(kv(key.trim(), &rewrite(value.trim(), &prompts), true));
        at += 1;
    }
    let body_lines: Vec<&str> = lines[at.min(lines.len())..]
        .iter()
        .take_while(|line| !after_body(line))
        .copied()
        .collect();
    let body = rewrite(body_lines.join("\n").trim_matches(['\n', '\r']).trim_end(), &prompts);

    let method = line.method.to_ascii_uppercase();
    let graphql_header = headers.iter().position(|header| {
        header.key.eq_ignore_ascii_case("x-request-type") && header.value.eq_ignore_ascii_case("graphql")
    });
    let (name, mut built) = api(&name, &method, &rewrite(&url, &prompts));
    built.description = notes.join("\n");
    built.settings.http_version = line.version;
    if no_redirect {
        built.settings.follow_redirects = Some(false);
    }
    let content_type = headers
        .iter()
        .position(|header| header.key.eq_ignore_ascii_case("content-type"));
    if method == "GRAPHQL" || graphql_header.is_some() {
        if let Some(at) = graphql_header {
            headers.remove(at);
        }
        headers.retain(|header| !header.key.eq_ignore_ascii_case("content-type"));
        built.method = HttpMethod::Post;
        built.content_type = ContentType::GraphQl;
        built.body = graphql(&body).to_body();
    } else if method == "WEBSOCKET" {
        built.method = HttpMethod::WebSocket;
        // The messages are parted by `===` lines; the first is the one to
        // send.
        built.body = body
            .split('\n')
            .collect::<Vec<_>>()
            .split(|line| line.trim_start().starts_with("==="))
            .map(|lines| lines.join("\n"))
            .find(|message| !message.trim().is_empty())
            .unwrap_or_default()
            .trim()
            .to_string();
    } else if let Some(at) = content_type {
        let value = headers[at].value.clone();
        match content_type_of(&value) {
            Some(ContentType::Form) => {
                let joined: String = body.lines().map(str::trim).collect();
                let fields: Vec<KvParam> = url::form_urlencoded::parse(joined.as_bytes())
                    .map(|(key, value)| kv(&key, &value, true))
                    .collect();
                headers.remove(at);
                built.content_type = ContentType::Form;
                built.body = serde_json::to_string(&fields).unwrap_or_default();
            }
            Some(ContentType::Multipart) => {
                match parameter(&value, "boundary").and_then(|boundary| multipart(&body, &boundary)) {
                    Some(fields) => {
                        headers.remove(at);
                        built.content_type = ContentType::Multipart;
                        built.body = serde_json::to_string(&fields).unwrap_or_default();
                    }
                    None => (built.content_type, built.body) = (ContentType::Plain, body),
                }
            }
            Some(ContentType::GraphQl) => {
                headers.remove(at);
                built.content_type = ContentType::GraphQl;
                built.body = GraphQlBody {
                    query: body,
                    ..Default::default()
                }
                .to_body();
            }
            Some(kind) => {
                headers.remove(at);
                (built.content_type, built.body) = (kind, body);
            }
            // A type the editor has no name for stays a header, which goes
            // before the editor's.
            None if body.is_empty() => {}
            None => (built.content_type, built.body) = (guess_content_type(&body), body),
        }
    } else if !body.is_empty() {
        (built.content_type, built.body) = (guess_content_type(&body), body);
    }
    built.headers = headers;
    Some(ImportItem::Api {
        name,
        request: Box::new(built),
    })
}

pub(super) fn parse(text: &str) -> ImportBundle {
    let mut environment = Vec::new();
    let items = blocks(text)
        .into_iter()
        .filter_map(|(title, lines)| request(title, &lines, &mut environment))
        .collect();
    ImportBundle {
        items,
        environment,
        ..Default::default()
    }
}

/// The environments of an environment file: REST Client's setting, or the
/// file itself — an object of objects, keys starting with `$` aside but
/// for `$shared`, one of them with a value at least.
fn environments_of(json: &Value) -> Option<&Map<String, Value>> {
    let map = json.as_object()?;
    let map = match map.get("rest-client.environmentVariables") {
        Some(Value::Object(inner)) => inner,
        _ => map,
    };
    let named = || {
        map.iter()
            .filter(|(name, _)| !name.starts_with('$') || *name == "$shared")
            .map(|(_, environment)| environment)
    };
    let looks = named().next().is_some()
        && named().all(Value::is_object)
        && named().any(|environment| {
            environment
                .as_object()
                .is_some_and(|variables| variables.values().any(|value| scalar(value).is_some()))
        });
    looks.then_some(map)
}

/// A variable's value; an object (JetBrains' `SSLConfiguration`), a list
/// or null is none.
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

pub(super) fn is_environment(json: &Value) -> bool {
    environments_of(json).is_some()
}

pub(super) fn parse_environment(json: &Value) -> ImportBundle {
    let Some(map) = environments_of(json) else {
        return ImportBundle::default();
    };
    let variables = |environment: &Value| -> Vec<(String, String)> {
        environment
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(name, value)| Some((name.clone(), scalar(value)?)))
            .collect()
    };
    let mut bundle = ImportBundle::default();
    for (name, environment) in map {
        match name.as_str() {
            "$shared" => bundle.environment = variables(environment),
            name if name.starts_with('$') => {}
            name => bundle.environments.push(ImportEnvironment {
                name: name.to_string(),
                variables: variables(environment),
            }),
        }
    }
    bundle
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HttpRequest;
    use pretty_assertions::assert_eq;

    fn apis(bundle: &ImportBundle) -> Vec<(&str, &HttpRequest)> {
        bundle
            .items
            .iter()
            .map(|item| match item {
                ImportItem::Api { name, request } => (name.as_str(), &**request),
                ImportItem::Folder { .. } => panic!("no folders"),
            })
            .collect()
    }

    const FILE: &str = r#"@host = https://shop.test
@token = t-1

# Signs in.
# @name login
POST {{host}}/login HTTP/1.1
Content-Type: application/json

{
    "user": "ann",
    "at": "{{$timestamp}}"
}

> {% client.global.set("token", response.body.token); %}

### list orders
// @no-redirect
GET {{host}}/orders
    ?page=2
    &size={{$randomInt 1 10}}
Authorization: Bearer {{token}}
X-Trace: {{$guid}}

>> orders.json

###
# @prompt otp One-time code, from the app
POST {{host}}/verify
Content-Type: application/x-www-form-urlencoded

code={{otp}}
&remember=true
"#;

    #[test]
    fn a_file_of_requests() {
        assert!(is_http(FILE));
        let bundle = parse(FILE);
        assert_eq!(
            bundle.environment,
            [
                ("host".to_string(), "https://shop.test".to_string()),
                ("token".to_string(), "t-1".to_string())
            ]
        );
        let apis = apis(&bundle);
        assert_eq!(apis.len(), 3);

        let (name, login) = apis[0];
        assert_eq!(name, "login");
        assert_eq!(login.method, HttpMethod::Post);
        assert_eq!(login.uri, "{{host}}/login");
        assert_eq!(login.description, "Signs in.");
        assert_eq!(login.content_type, ContentType::Json);
        assert!(login.headers.is_empty(), "the type is the editor's");
        assert_eq!(
            login.body,
            "{\n    \"user\": \"ann\",\n    \"at\": \"{{timestamp()}}\"\n}"
        );
        assert_eq!(login.settings.http_version, None);

        let (name, orders) = apis[1];
        assert_eq!(name, "list orders");
        assert_eq!(orders.method, HttpMethod::Get);
        assert_eq!(orders.settings.follow_redirects, Some(false));
        assert_eq!(
            orders.query,
            [kv("page", "2", true), kv("size", "{{random(1, 9)}}", true)]
        );
        assert_eq!(
            orders.headers,
            [
                kv("Authorization", "Bearer {{token}}", true),
                kv("X-Trace", "{{uuid()}}", true)
            ]
        );
        assert_eq!(orders.body, "", "where the response goes is no body");

        let (_, verify) = apis[2];
        assert_eq!(verify.content_type, ContentType::Form);
        let fields: Vec<KvParam> = serde_json::from_str(&verify.body).expect("fields");
        assert_eq!(
            fields,
            [
                kv("code", "{{prompt(One-time code， from the app)}}", true),
                kv("remember", "true", true)
            ]
        );
    }

    #[test]
    fn graphql_websocket_multipart_and_http2() {
        let text = r#"GRAPHQL https://api.test/graphql

query User($id: ID!) {
    user(id: $id) { name }
}

{"id": "{{$random.uuid}}"}

###
POST https://api.test/graphql
X-REQUEST-TYPE: GraphQL
Content-Type: application/json

{ viewer { login } }

###
WEBSOCKET wss://api.test/ws
Content-Type: application/json

===
{"hello": 1}
=== wait-for-server
{"again": 2}

###
POST https://api.test/upload HTTP/2
Content-Type: multipart/form-data; boundary=Edge

--Edge
Content-Disposition: form-data; name="title"

A, B
--Edge
Content-Disposition: form-data; name="image"; filename="a.png"
Content-Type: image/png

< ./a.png
--Edge
Content-Disposition: form-data; name="meta"
Content-Type: application/xml

<?xml version="1.0"?>
<meta/>
--Edge
Content-Disposition: form-data; name="page"

<html></html>
--Edge--
"#;
        assert!(is_http(text));
        let bundle = parse(text);
        let apis = apis(&bundle);
        let (_, user) = apis[0];
        assert_eq!(
            (user.method, user.content_type),
            (HttpMethod::Post, ContentType::GraphQl)
        );
        let body: GraphQlBody = serde_json::from_str(&user.body).expect("GraphQL");
        assert_eq!(body.query, "query User($id: ID!) {\n    user(id: $id) { name }\n}");
        assert_eq!(body.variables, r#"{"id": "{{uuid()}}"}"#);

        let (_, viewer) = apis[1];
        assert!(viewer.headers.is_empty());
        let body: GraphQlBody = serde_json::from_str(&viewer.body).expect("GraphQL");
        assert_eq!(
            (body.query.as_str(), body.variables.as_str()),
            ("{ viewer { login } }", "")
        );

        let (_, socket) = apis[2];
        assert_eq!(socket.method, HttpMethod::WebSocket);
        assert_eq!(socket.body, r#"{"hello": 1}"#);

        let (_, upload) = apis[3];
        assert_eq!(upload.settings.http_version, Some(HttpVersion::Http2));
        assert_eq!(upload.content_type, ContentType::Multipart);
        let fields: Vec<KvParam> = serde_json::from_str(&upload.body).expect("fields");
        assert_eq!(
            fields,
            [
                kv("title", "A, B", true),
                kv("image", "file://./a.png", true),
                // BUG-25: no file to read.
                kv("meta", "<?xml version=\"1.0\"?>\n<meta/>", true),
                kv("page", "<html></html>", true),
            ]
        );
    }

    #[test]
    fn system_variables_it_has_and_has_not() {
        let rewritten = rewrite(
            "{{$timestamp -1 d}} {{$datetime iso8601}} {{$localDatetime iso8601 2 h}} {{$isoTimestamp}} \
             {{$processEnv HOME}} {{$env.USER}} {{$random.alphanumeric(8)}} {{$random.integer(1, 5)}} \
             {{$randomInt}} {{$datetime rfc1123}} {{$timestamp 1 y}} {{$dotenv KEY}} {{$processEnv %name}} {{login.response.body.$.token}}",
            &[],
        );
        assert_eq!(
            rewritten,
            "{{now(%s, -1d)}} {{now(iso, utc)}} {{now(iso, 2h)}} {{now(iso, utc)}} \
             {{env(HOME)}} {{env(USER)}} {{randomString(8)}} {{random(1, 4)}} \
             {{random(0, 1000)}} {{$datetime rfc1123}} {{$timestamp 1 y}} {{$dotenv KEY}} {{$processEnv %name}} {{login.response.body.$.token}}"
        );
    }

    #[test]
    fn what_is_not_a_http_file() {
        assert!(is_http("GET https://a.test"));
        assert!(is_http("# hello\nPOST /login\n"));
        assert!(is_http("@host = https://a.test\n\n{{host}}/ping"));
        // A bare address is what to fetch an import from (B3).
        assert!(!is_http("https://a.test/openapi.json"));
        assert!(!is_http("# a comment\nopenapi: 3.0.0\npaths: {}\n"));
        assert!(!is_http("get: {summary: x}"));
        assert!(!is_http(r#"{"item": []}"#));
        assert!(!is_http("<?xml version=\"1.0\"?>\n<a>GET</a>"));
    }

    /// BUG-16: a pre-request script before a request is left out, and the
    /// request kept — the first of the file too.
    #[test]
    fn a_pre_request_script_is_left_out() {
        let file = r#"< {%
    request.variables.set("id", "7");
%}
GET https://shop.test/orders/{{id}}

###
< ./sign.js
// @name signed
POST https://shop.test/orders

###
< {% request.variables.set("a", "1"); %}
DELETE https://shop.test/orders/1
"#;
        assert!(is_http(file));
        let bundle = parse(file);
        let found: Vec<(&str, HttpMethod, &str)> = apis(&bundle)
            .into_iter()
            .map(|(name, request)| (name, request.method, request.uri.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                (
                    "https://shop.test/orders/{{id}}",
                    HttpMethod::Get,
                    "https://shop.test/orders/{{id}}"
                ),
                ("signed", HttpMethod::Post, "https://shop.test/orders"),
                (
                    "https://shop.test/orders/1",
                    HttpMethod::Delete,
                    "https://shop.test/orders/1"
                ),
            ]
        );
    }

    #[test]
    fn environment_files() {
        let jetbrains: Value = serde_json::from_str(
            r#"{"$schema": "https://x", "dev": {"host": "http://localhost:8080", "port": 8080, "ssl": {"SSLConfiguration": {}}},
                "prod": {"host": "https://shop.test", "debug": false, "nothing": null}}"#,
        )
        .expect("JSON");
        assert!(is_environment(&jetbrains));
        let bundle = parse_environment(&jetbrains);
        assert!(bundle.environment.is_empty());
        assert_eq!(
            bundle.environments,
            [
                ImportEnvironment {
                    name: "dev".into(),
                    variables: vec![
                        ("host".into(), "http://localhost:8080".into()),
                        ("port".into(), "8080".into())
                    ],
                },
                ImportEnvironment {
                    name: "prod".into(),
                    variables: vec![
                        ("host".into(), "https://shop.test".into()),
                        ("debug".into(), "false".into())
                    ],
                },
            ]
        );

        let settings: Value = serde_json::from_str(
            r#"{"editor.fontSize": 13, "rest-client.environmentVariables": {"$shared": {"version": "v1"}, "local": {"host": "http://localhost"}}}"#,
        )
        .expect("JSON");
        let bundle = parse_environment(&settings);
        assert_eq!(bundle.environment, [("version".to_string(), "v1".to_string())]);
        assert_eq!(bundle.environments.len(), 1);

        for not in [r#"{"id": "a", "name": "b"}"#, "{}", r#"{"a": {"b": [1]}}"#, "[]"] {
            assert!(!is_environment(&serde_json::from_str(not).expect("JSON")), "{not}");
        }
    }
}
