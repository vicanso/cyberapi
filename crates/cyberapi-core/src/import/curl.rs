//! A cURL command as an API (F10.3).
//!
//! As before: `-X`, `-H`, `-d` (a body turns GET into POST), `-u` (Basic
//! auth), `-A`, `-I`, `--compressed`; the URL's query becomes the query list;
//! the body's type comes from `Content-Type`, or JSON when it looks like it,
//! or a form. Unlike before: `-b` is the `Cookie` header it is, not
//! `Set-Cookie` (#14); `--data-raw`, `--data-binary` and `--data-urlencode`
//! are bodies; `-F` makes a multipart body; `-G` puts the data in the query.

use super::{ImportBundle, ImportError, ImportItem, NoUrlSnafu, content_type_of, guess_content_type, kv, split_url};
use crate::auth::Auth;
use crate::model::{ContentType, HttpMethod, HttpRequest, KvParam};

/// The words of a shell command: quotes, backslash escapes and line
/// continuations as a POSIX shell reads them.
fn words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                in_word = true;
                for inner in chars.by_ref() {
                    if inner == '\'' {
                        break;
                    }
                    word.push(inner);
                }
            }
            '"' => {
                in_word = true;
                while let Some(inner) = chars.next() {
                    match inner {
                        '"' => break,
                        '\\' => match chars.peek() {
                            Some(next @ ('"' | '\\' | '$' | '`')) => {
                                word.push(*next);
                                chars.next();
                            }
                            Some('\n') => {
                                chars.next();
                            }
                            _ => word.push('\\'),
                        },
                        other => word.push(other),
                    }
                }
            }
            '\\' => match chars.next() {
                Some('\n') | Some('\r') => {}
                Some(next) => {
                    in_word = true;
                    word.push(next);
                }
                None => {}
            },
            ch if ch.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            other => {
                in_word = true;
                word.push(other);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// `-XPOST` as `-X POST`; `--data=x` as `--data x`.
fn split_attached(word: &str) -> (String, Option<String>) {
    if let Some(long) = word.strip_prefix("--")
        && let Some((flag, value)) = long.split_once('=')
    {
        return (format!("--{flag}"), Some(value.to_string()));
    }
    let short_with_value = ['X', 'H', 'd', 'u', 'A', 'b', 'F', 'e'];
    let mut chars = word.chars();
    if chars.next() == Some('-')
        && let Some(flag) = chars.next()
        && short_with_value.contains(&flag)
        && word.len() > 2
    {
        return (format!("-{flag}"), Some(word[2..].to_string()));
    }
    (word.to_string(), None)
}

pub(super) fn parse(text: &str) -> Result<ImportBundle, ImportError> {
    let words = words(text);
    let mut method: Option<String> = None;
    let mut url: Option<String> = None;
    let mut headers: Vec<KvParam> = Vec::new();
    let mut data: Vec<String> = Vec::new();
    let mut form: Vec<KvParam> = Vec::new();
    let mut auth = Auth::Inherit;
    let mut head = false;
    let mut get = false;
    let mut index = 1;
    while index < words.len() {
        let (flag, attached) = split_attached(&words[index]);
        let mut value = || {
            attached.clone().or_else(|| {
                index += 1;
                words.get(index).cloned()
            })
        };
        match flag.as_str() {
            "-X" | "--request" => method = value(),
            "-H" | "--header" => {
                if let Some(header) = value()
                    && let Some((name, header_value)) = header.split_once(':')
                {
                    headers.push(kv(name.trim(), header_value.trim(), true));
                }
            }
            "-d" | "--data" | "--data-ascii" | "--data-raw" | "--data-binary" => data.extend(value()),
            "--data-urlencode" => {
                if let Some(pair) = value() {
                    data.push(match pair.split_once('=') {
                        Some((name, raw)) => format!("{name}={}", crate::send::encode_uri_component(raw)),
                        None => crate::send::encode_uri_component(&pair),
                    });
                }
            }
            "-F" | "--form" => {
                if let Some(field) = value()
                    && let Some((name, field_value)) = field.split_once('=')
                {
                    let field_value = match field_value.strip_prefix('@') {
                        Some(path) => format!("file://{}", path.split(';').next().unwrap_or(path)),
                        None => field_value.to_string(),
                    };
                    form.push(kv(name, &field_value, true));
                }
            }
            "-u" | "--user" => {
                if let Some(user) = value() {
                    let (name, password) = user.split_once(':').unwrap_or((&user, ""));
                    auth = Auth::Basic {
                        username: name.to_string(),
                        password: password.to_string(),
                    };
                }
            }
            "-A" | "--user-agent" => {
                if let Some(agent) = value() {
                    headers.push(kv("User-Agent", &agent, true));
                }
            }
            "-b" | "--cookie" => {
                if let Some(cookie) = value() {
                    headers.push(kv("Cookie", &cookie, true));
                }
            }
            "-e" | "--referer" => {
                if let Some(referer) = value() {
                    headers.push(kv("Referer", &referer, true));
                }
            }
            "--url" => url = value(),
            "-I" | "--head" => head = true,
            "-G" | "--get" => get = true,
            // Flags with a value this does not use.
            "-o" | "--output" | "-m" | "--max-time" | "--connect-timeout" | "-x" | "--proxy" | "-w" | "--write-out"
            | "--cacert" | "--cert" | "--key" | "-c" | "--cookie-jar" | "-T" | "--upload-file" => {
                let _ = value();
            }
            other if !other.starts_with('-') && url.is_none() => url = Some(other.to_string()),
            _ => {}
        }
        index += 1;
    }
    let url = url.ok_or(NoUrlSnafu.build())?;
    let (uri, mut query) = split_url(&url);
    let body = data.join("&");
    if get && !body.is_empty() {
        let (_, more) = split_url(&format!("?{body}"));
        query.extend(more);
    }
    let body = if get { String::new() } else { body };

    // Content-Type becomes the body's type when it is one the editor has.
    let mut content_type = None;
    headers.retain(|header| {
        if header.key.eq_ignore_ascii_case("content-type")
            && let Some(known) = content_type_of(&header.value)
        {
            content_type = Some(known);
            return false;
        }
        true
    });
    let has_body = !body.is_empty() || !form.is_empty();
    // A body makes a GET a POST, even a GET asked for (F10.3).
    let method = match (method.map(|method| HttpMethod::parse_lenient(&method)), head) {
        (Some(HttpMethod::Get) | None, false) if has_body => HttpMethod::Post,
        (Some(method), _) => method,
        (None, true) => HttpMethod::Head,
        (None, false) => HttpMethod::Get,
    };
    let (content_type, body) = if !form.is_empty() {
        (ContentType::Multipart, serde_json::to_string(&form).unwrap_or_default())
    } else {
        let content_type = content_type.unwrap_or_else(|| match guess_content_type(&body) {
            ContentType::Json => ContentType::Json,
            _ if body.is_empty() => ContentType::Json,
            _ => ContentType::Form,
        });
        let body = if content_type == ContentType::Form {
            let (_, fields) = split_url(&format!("?{body}"));
            serde_json::to_string(&fields).unwrap_or_default()
        } else {
            body
        };
        (content_type, body)
    };
    let name = uri
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty() && !segment.contains(':'))
        .unwrap_or(&uri)
        .to_string();
    Ok(ImportBundle {
        items: vec![ImportItem::Api {
            name,
            request: Box::new(HttpRequest {
                method,
                uri,
                content_type,
                body,
                query,
                headers,
                auth,
                ..Default::default()
            }),
        }],
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn request(text: &str) -> HttpRequest {
        match parse(text).expect("a command").items.remove(0) {
            ImportItem::Api { request, .. } => *request,
            other => panic!("not an API: {other:?}"),
        }
    }

    #[test]
    fn shell_words_are_read_like_a_shell() {
        assert_eq!(
            words("curl -H 'a: b c' \"x\\\"y\" it\\'s \\\n --data-raw '{\"k\":1}'"),
            ["curl", "-H", "a: b c", "x\"y", "it's", "--data-raw", "{\"k\":1}"]
        );
    }

    #[test]
    fn a_delete_keeps_its_body_and_type() {
        let got = request(
            "curl -X DELETE http://127.0.0.1:8765/echo -H 'Content-Type: application/json' -d '{\"ids\":[1,2]}'",
        );
        assert_eq!(got.method, HttpMethod::Delete);
        assert_eq!(got.content_type, ContentType::Json);
        assert_eq!(got.body, "{\"ids\":[1,2]}");
        assert!(got.headers.is_empty());
        assert!(got.has_body_tab());
    }

    #[test]
    fn a_json_post_from_a_browser() {
        let got = request(
            r#"curl 'https://api.test/v1/users?page=2' -H 'content-type: application/json; charset=utf-8' -H 'X-Token: t' -b 'sid=1' --data-raw '{"name":"tree"}' --compressed"#,
        );
        assert_eq!(got.method, HttpMethod::Post);
        assert_eq!(got.uri, "https://api.test/v1/users");
        assert_eq!(got.query, [kv("page", "2", true)]);
        assert_eq!(got.content_type, ContentType::Json);
        assert_eq!(got.body, r#"{"name":"tree"}"#);
        assert_eq!(got.headers, [kv("X-Token", "t", true), kv("Cookie", "sid=1", true)]);
    }

    #[test]
    fn forms_auth_and_methods() {
        let form = request("curl -XPUT https://a.test -d 'a=1&b=x%20y' -u me:secret -A agent/1");
        assert_eq!(form.method, HttpMethod::Put);
        assert_eq!(form.content_type, ContentType::Form);
        assert_eq!(
            serde_json::from_str::<Vec<KvParam>>(&form.body).expect("fields"),
            [kv("a", "1", true), kv("b", "x y", true)]
        );
        assert_eq!(
            form.auth,
            Auth::Basic {
                username: "me".into(),
                password: "secret".into()
            }
        );
        assert_eq!(form.headers, [kv("User-Agent", "agent/1", true)]);

        let get_with_body = request("curl -X GET https://a.test -H 'content-type: application/json' -d '{}'");
        assert_eq!(get_with_body.method, HttpMethod::Post);
        assert_eq!(request("curl -X GET https://a.test").method, HttpMethod::Get);

        let multipart = request("curl https://a.test/up -F 'note=hi' -F 'file=@/tmp/a.png;type=image/png'");
        assert_eq!(
            (multipart.method, multipart.content_type),
            (HttpMethod::Post, ContentType::Multipart)
        );
        assert_eq!(
            serde_json::from_str::<Vec<KvParam>>(&multipart.body).expect("fields"),
            [kv("note", "hi", true), kv("file", "file:///tmp/a.png", true)]
        );

        let got = request("curl -G https://a.test/s -d q=rust");
        assert_eq!(
            (got.method, got.query.clone(), got.body.as_str()),
            (HttpMethod::Get, vec![kv("q", "rust", true)], "")
        );
        assert_eq!(request("curl -I https://a.test").method, HttpMethod::Head);
        assert_eq!(parse("curl -X GET"), Err(ImportError::NoUrl));
    }
}
