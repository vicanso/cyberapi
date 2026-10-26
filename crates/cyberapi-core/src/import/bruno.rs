//! B5: one Bruno request file (`.bru`) as an API, or one Bruno environment
//! file as variables (F10.3). A whole Bruno collection is a folder of
//! these, which is not read (picking a folder is for later); its YAML
//! export is OpenCollection's, `super::opencollection`.
//!
//! A `.bru` file is a run of blocks, `name {` to a `}` alone at the start
//! of a line (`name [` to `]` for a list). A block of pairs has a
//! `key: value` a line, one switched off behind `~`; a block of text (a
//! body, the docs) is indented two spaces. `meta` names the request; the
//! method's block (`get`, `post`…) holds the URL and which body and auth
//! it uses, which the `body:…` and `auth:…` blocks hold. `params:path`
//! fills the URL's `:name`s, this app having none of its own. Bruno's
//! `{{process.env.NAME}}` is this app's `{{env(NAME)}}` (D2); scripts,
//! tests, `vars:…` and the docs stay behind.

use super::{ImportBundle, ImportItem, api, content_type_of, kv};
use crate::auth::{Auth, KeyPlace};
use crate::graphql::GraphQlBody;
use crate::model::{ContentType, KvParam};
use crate::oauth::{ClientAuth, Grant, OAuth2};

const METHODS: [&str; 9] = [
    "get", "post", "put", "delete", "patch", "options", "head", "connect", "trace",
];

struct Block<'a> {
    name: &'a str,
    lines: Vec<&'a str>,
}

/// `name {` or `name [`, at the start of a line: the block's name and the
/// line that closes it.
fn opening(line: &str) -> Option<(&str, &'static str)> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let line = line.trim_end();
    let (name, close) = if let Some(name) = line.strip_suffix('{') {
        (name, "}")
    } else {
        (line.strip_suffix('[')?, "]")
    };
    let name = name.trim_end();
    let named = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'));
    named.then_some((name, close))
}

/// Whether `text` is a `.bru` file: its first block is a request's or an
/// environment's.
pub(super) fn is_bru(text: &str) -> bool {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .and_then(opening)
        .is_some_and(|(name, _)| name == "meta" || name == "vars" || METHODS.contains(&name))
}

fn blocks(text: &str) -> Vec<Block<'_>> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some((name, close)) = opening(line) else {
            continue;
        };
        let mut inner = Vec::new();
        for line in lines.by_ref() {
            if line.trim_end() == close {
                break;
            }
            inner.push(line);
        }
        out.push(Block { name, lines: inner });
    }
    out
}

/// `{{process.env.NAME}}` → `{{env(NAME)}}`.
pub(super) fn variables(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break;
        };
        out.push_str(&rest[..start]);
        match after[..end].trim().strip_prefix("process.env.") {
            Some(name) => out.push_str(&format!("{{{{env({name})}}}}")),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// A block of text, its two spaces of indent taken off.
fn text_of(block: &Block<'_>) -> String {
    let lines: Vec<&str> = block
        .lines
        .iter()
        .map(|line| line.strip_prefix("  ").unwrap_or(line.trim_start()))
        .collect();
    variables(lines.join("\n").trim_end())
}

/// A block of pairs: `key: value`, `~key: value` switched off, a quoted
/// key, and a value of several lines between `'''`s.
fn pairs(block: &Block<'_>) -> Vec<KvParam> {
    let mut out = Vec::new();
    let mut lines = block.lines.iter();
    while let Some(line) = lines.next() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (enabled, line) = match line.strip_prefix('~') {
            Some(rest) => (false, rest),
            None => (true, line),
        };
        let (key, value) = match line.strip_prefix('"').and_then(|rest| rest.split_once('"')) {
            Some((key, rest)) => (key, rest.trim_start().strip_prefix(':').unwrap_or(rest)),
            None => line.split_once(':').unwrap_or((line, "")),
        };
        let mut value = value.trim().to_string();
        if value == "'''" {
            let mut held = Vec::new();
            for line in lines.by_ref() {
                if line.trim() == "'''" {
                    break;
                }
                held.push(line.strip_prefix("    ").unwrap_or(line.trim_start()));
            }
            value = held.join("\n");
        }
        out.push(kv(key.trim(), &variables(&value), enabled));
    }
    out
}

/// A list block's names: `token,` a line.
fn names(block: &Block<'_>) -> Vec<String> {
    block
        .lines
        .iter()
        .map(|line| line.trim().trim_end_matches(',').trim())
        .filter(|name| !name.is_empty() && !name.starts_with('~'))
        .map(str::to_string)
        .collect()
}

/// `/users/:id` with `id` filled in.
pub(super) fn fill_path(url: &str, params: &[KvParam]) -> String {
    let mut out = url.to_string();
    for param in params.iter().filter(|param| param.enabled && !param.value.is_empty()) {
        let needle = format!(":{}", param.key);
        let mut from = 0;
        while let Some(at) = out[from..].find(&needle) {
            let (start, end) = (from + at, from + at + needle.len());
            let whole = out[..start].ends_with('/')
                && out[end..]
                    .chars()
                    .next()
                    .is_none_or(|next| matches!(next, '/' | '?' | '#'));
            if whole {
                out.replace_range(start..end, &param.value);
                from = start + param.value.len();
            } else {
                from = end;
            }
        }
    }
    out
}

fn auth(mode: &str, find: &dyn Fn(&str) -> Vec<KvParam>) -> Auth {
    let fields = |name: &str| {
        let fields = find(name);
        move |key: &str| {
            fields
                .iter()
                .find(|field| field.key == key)
                .map(|field| field.value.clone())
                .unwrap_or_default()
        }
    };
    match mode {
        "none" => Auth::None,
        "basic" => {
            let field = fields("auth:basic");
            Auth::Basic {
                username: field("username"),
                password: field("password"),
            }
        }
        "digest" => {
            let field = fields("auth:digest");
            Auth::Digest {
                username: field("username"),
                password: field("password"),
            }
        }
        "bearer" => Auth::Bearer {
            token: fields("auth:bearer")("token"),
        },
        "apikey" => {
            let field = fields("auth:apikey");
            Auth::ApiKey {
                name: field("key"),
                value: field("value"),
                place: if field("placement").contains("query") {
                    KeyPlace::Query
                } else {
                    KeyPlace::Header
                },
            }
        }
        // C4: the grants asked without a browser; Bruno puts the client
        // in the body unless told otherwise.
        "oauth2" => {
            let field = fields("auth:oauth2");
            let grant = match field("grant_type").as_str() {
                "client_credentials" => Grant::ClientCredentials,
                "password" => Grant::Password,
                _ => return Auth::Inherit,
            };
            Auth::from(OAuth2 {
                grant,
                token_url: field("access_token_url"),
                client_id: field("client_id"),
                client_secret: field("client_secret"),
                username: field("username"),
                password: field("password"),
                scope: field("scope"),
                client_auth: match field("credentials_placement").as_str() {
                    "basic_auth_header" => ClientAuth::Header,
                    _ => ClientAuth::Body,
                },
            })
        }
        // `inherit`, and what this app has no counterpart of.
        _ => Auth::Inherit,
    }
}

pub(super) fn parse(text: &str) -> ImportBundle {
    let blocks = blocks(text);
    let find = |name: &str| blocks.iter().find(|block| block.name == name);
    let dict = |name: &str| find(name).map(pairs).unwrap_or_default();
    let Some(method) = blocks.iter().find(|block| METHODS.contains(&block.name)) else {
        // An environment: its variables, and its secrets' names.
        let mut environment: Vec<(String, String)> = dict("vars")
            .into_iter()
            .filter(|kv| kv.enabled && !kv.key.is_empty())
            .map(|kv| (kv.key, kv.value))
            .collect();
        environment.extend(
            find("vars:secret")
                .map(names)
                .unwrap_or_default()
                .into_iter()
                .map(|name| (name, String::new())),
        );
        return ImportBundle {
            environment,
            ..Default::default()
        };
    };
    let meta = dict("meta");
    let own = dict(method.name);
    let field = |fields: &[KvParam], key: &str| {
        fields
            .iter()
            .find(|field| field.key == key)
            .map(|field| field.value.clone())
            .unwrap_or_default()
    };
    let url = fill_path(&field(&own, "url"), &dict("params:path"));
    let (name, mut built) = api(&field(&meta, "name"), method.name, &url);
    let listed = dict("params:query");
    if !listed.is_empty() {
        built.query = listed;
    }
    built.headers = dict("headers");
    if let Some(at) = built
        .headers
        .iter()
        .position(|header| header.key.eq_ignore_ascii_case("content-type") && content_type_of(&header.value).is_some())
    {
        let header = built.headers.remove(at);
        built.content_type = content_type_of(&header.value).unwrap_or_default();
    }
    // What the method's block says, or the body there is.
    let mode = match field(&own, "body") {
        mode if !mode.is_empty() => mode,
        _ if field(&meta, "type") == "graphql" => "graphql".to_string(),
        _ => blocks
            .iter()
            .find_map(|block| block.name.strip_prefix("body:"))
            .map(|kind| match kind {
                "form-urlencoded" => "formUrlEncoded",
                "multipart-form" => "multipartForm",
                other => other,
            })
            .unwrap_or_default()
            .to_string(),
    };
    let text_block = |name: &str| find(name).map(text_of).unwrap_or_default();
    match mode.as_str() {
        "json" => (built.content_type, built.body) = (ContentType::Json, text_block("body:json")),
        "xml" => (built.content_type, built.body) = (ContentType::Xml, text_block("body:xml")),
        "text" | "sparql" => {
            (built.content_type, built.body) = (ContentType::Plain, text_block(&format!("body:{mode}")))
        }
        "formUrlEncoded" => {
            built.content_type = ContentType::Form;
            built.body = serde_json::to_string(&dict("body:form-urlencoded")).unwrap_or_default();
        }
        "multipartForm" => {
            built.content_type = ContentType::Multipart;
            let fields: Vec<KvParam> = dict("body:multipart-form")
                .into_iter()
                .map(|mut field| {
                    // `@file(a.png|b.png) @contentType(image/png)`: the
                    // first file.
                    if let Some(files) = field.value.strip_prefix("@file(") {
                        let files = files.split(')').next().unwrap_or_default();
                        field.value = format!("file://{}", files.split('|').next().unwrap_or_default());
                    }
                    field
                })
                .collect();
            built.body = serde_json::to_string(&fields).unwrap_or_default();
        }
        "graphql" => {
            built.content_type = ContentType::GraphQl;
            built.body = GraphQlBody {
                query: text_block("body:graphql"),
                variables: text_block("body:graphql:vars"),
                operation_name: String::new(),
            }
            .to_body();
        }
        _ => {}
    }
    built.auth = auth(&field(&own, "auth"), &dict);
    ImportBundle {
        items: vec![ImportItem::Api {
            name,
            request: Box::new(built),
        }],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HttpMethod;
    use pretty_assertions::assert_eq;

    fn only(bundle: &ImportBundle) -> (&str, &crate::model::HttpRequest) {
        match &bundle.items[..] {
            [ImportItem::Api { name, request }] => (name, request),
            other => panic!("one API: {other:?}"),
        }
    }

    #[test]
    fn a_request_file() {
        let text = r#"meta {
  name: Create order
  type: http
  seq: 2
}

post {
  url: {{host}}/shops/:shop/orders?draft=1
  body: json
  auth: bearer
}

params:query {
  draft: 1
  ~lang: en
}

params:path {
  shop: 42
}

headers {
  Content-Type: application/json
  ~X-Debug: 1
  X-Key: {{process.env.SHOP_KEY}}
}

auth:bearer {
  token: {{token}}
}

body:json {
  {
    "sku": "a",
    "nested": {
      "b": 1
    }
  }
}

body:text {
  not this one
}

script:pre-request {
  bru.setVar("a", 1);
}

docs {
  # Orders
}
"#;
        assert!(is_bru(text));
        let bundle = parse(text);
        let (name, request) = only(&bundle);
        assert_eq!(name, "Create order");
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.uri, "{{host}}/shops/42/orders");
        assert_eq!(request.query, [kv("draft", "1", true), kv("lang", "en", false)]);
        assert_eq!(
            request.headers,
            [kv("X-Debug", "1", false), kv("X-Key", "{{env(SHOP_KEY)}}", true)]
        );
        assert_eq!(request.content_type, ContentType::Json);
        assert_eq!(
            request.body,
            "{\n  \"sku\": \"a\",\n  \"nested\": {\n    \"b\": 1\n  }\n}"
        );
        assert_eq!(
            request.auth,
            Auth::Bearer {
                token: "{{token}}".into()
            }
        );
    }

    #[test]
    fn forms_graphql_and_auth_kinds() {
        let form = r#"meta {
  name: login
}

put {
  url: https://a.test/login
  body: formUrlEncoded
  auth: apikey
}

auth:apikey {
  key: X-Key
  value: abc
  placement: queryparams
}

body:form-urlencoded {
  user: me
  ~debug: 1
  "a:b": c: d
}
"#;
        let bundle = parse(form);
        let (_, request) = only(&bundle);
        assert_eq!(request.content_type, ContentType::Form);
        assert_eq!(
            crate::send::form_fields(&request.body),
            [kv("user", "me", true), kv("debug", "1", false), kv("a:b", "c: d", true)]
        );
        assert_eq!(
            request.auth,
            Auth::ApiKey {
                name: "X-Key".into(),
                value: "abc".into(),
                place: KeyPlace::Query
            }
        );
        let multipart = "post {\n  url: https://a.test/up\n  body: multipartForm\n  auth: inherit\n}\n\nbody:multipart-form {\n  note: hi\n  file: @file(/tmp/a.png|/tmp/b.png) @contentType(image/png)\n}\n";
        assert!(is_bru(multipart), "a method's block first");
        let bundle = parse(multipart);
        let (name, request) = only(&bundle);
        assert_eq!(name, "https://a.test/up", "no name: its URL");
        assert_eq!(
            crate::send::form_fields(&request.body),
            [kv("note", "hi", true), kv("file", "file:///tmp/a.png", true)]
        );
        assert_eq!(request.auth, Auth::Inherit);
        let graphql = "meta {\n  name: me\n  type: graphql\n}\n\npost {\n  url: https://a.test/graphql\n  auth: none\n}\n\nbody:graphql {\n  query {\n    me { id }\n  }\n}\n\nbody:graphql:vars {\n  {\"id\": 1}\n}\n";
        let bundle = parse(graphql);
        let (_, request) = only(&bundle);
        assert_eq!(request.content_type, ContentType::GraphQl);
        let body = GraphQlBody::read(&request.body);
        assert_eq!(body.query, "query {\n  me { id }\n}");
        assert_eq!(body.variables, "{\"id\": 1}");
        assert_eq!(request.auth, Auth::None);
    }

    #[test]
    fn an_environment_file_and_what_is_not_one() {
        let text = "vars {\n  host: http://localhost:3000\n  ~old: 1\n}\nvars:secret [\n  token,\n  key\n]\n";
        assert!(is_bru(text));
        assert_eq!(
            parse(text).environment,
            [
                ("host".to_string(), "http://localhost:3000".to_string()),
                ("token".to_string(), String::new()),
                ("key".to_string(), String::new())
            ]
        );
        assert!(!is_bru("{\"meta\": {}}"));
        assert!(!is_bru("openapi: 3.0.0\ninfo: {}"));
        assert!(!is_bru("  get {"), "a block starts a line");
        assert_eq!(
            fill_path("/a/:id/:idx?x=:id", &[kv("id", "7", true)]),
            "/a/7/:idx?x=:id"
        );
    }
}
