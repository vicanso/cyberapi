//! A cURL command as an API (F10.3).
//!
//! As before: `-X`, `-H`, `-d` (a body turns GET into POST), `-u` (Basic
//! auth), `-A`, `-I`, `--compressed`; the URL's query becomes the query list;
//! the body's type comes from `Content-Type`, or JSON when it looks like it,
//! or a form. Unlike before: `-b` is the `Cookie` header it is, not
//! `Set-Cookie` (#14); `--data-raw`, `--data-binary` and `--data-urlencode`
//! are bodies; `-F` makes a multipart body; `-G` puts the data in the query.

use super::lost;
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
    let short_with_value = ['X', 'H', 'd', 'u', 'A', 'b', 'F', 'e', 'T'];
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
    let mut binary_files: Vec<String> = Vec::new();
    let mut upload_file: Option<String> = None;
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
            "-d" | "--data" | "--data-ascii" | "--data-binary" => {
                if let Some(val) = value() {
                    if let Some(path) = val.strip_prefix('@').map(str::trim).filter(|path| !path.is_empty()) {
                        binary_files.push(path.to_string());
                    } else {
                        data.push(val);
                    }
                }
            }
            "--data-raw" => data.extend(value()),
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
            "-T" | "--upload-file" => {
                if let Some(path) = value() {
                    let path = path.trim();
                    if !path.is_empty() {
                        upload_file = Some(path.to_string());
                    }
                }
            }
            // Flags with a value this does not use.
            "-o" | "--output" | "-m" | "--max-time" | "--connect-timeout" | "-x" | "--proxy" | "-w" | "--write-out"
            | "--cacert" | "--cert" | "--key" | "-c" | "--cookie-jar" => {
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

    let binary_path = if !get && form.is_empty() {
        upload_file
            .as_deref()
            .or_else(|| (data.is_empty() && binary_files.len() == 1).then(|| binary_files[0].as_str()))
            .map(str::to_string)
    } else {
        None
    };

    // Content-Type becomes the body's type when it is one the editor has.
    // For binary file bodies, preserve any non-default Content-Type header.
    let mut content_type = None;
    headers.retain(|header| {
        if header.key.eq_ignore_ascii_case("content-type")
            && let Some(known) = content_type_of(&header.value)
        {
            if binary_path.is_some() && known != ContentType::Binary {
                return true;
            }
            content_type = Some(known);
            return false;
        }
        true
    });
    let has_body = !body.is_empty() || !form.is_empty() || binary_path.is_some();
    // `-T` defaults to PUT; any body turns an implicit GET into POST (#F10.3).
    let method = match (method.map(|method| HttpMethod::parse_lenient(&method)), head) {
        (None, false) if upload_file.is_some() => HttpMethod::Put,
        (Some(HttpMethod::Get) | None, false) if has_body => HttpMethod::Post,
        (Some(method), _) => method,
        (None, true) => HttpMethod::Head,
        (None, false) => HttpMethod::Get,
    };
    let (content_type, body) = if !form.is_empty() {
        (ContentType::Multipart, serde_json::to_string(&form).unwrap_or_default())
    } else if let Some(path) = &binary_path {
        (ContentType::Binary, format!("file://{path}"))
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
    let mut lost = Vec::new();
    if binary_path.is_none() {
        for path in &binary_files {
            lost::push_body(&mut lost, &name, &format!("file ({path})"));
        }
    }
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
        lost,
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

    #[test]
    fn file_upload_and_binary_file_flags_import_as_binary_bodies() {
        let upload = super::super::parse("curl -T /tmp/blob.bin https://a.test/upload").expect("parsed");
        assert!(upload.lost.is_empty());
        let ImportItem::Api { request: put_req, .. } = &upload.items[0] else {
            panic!("api");
        };
        assert_eq!(put_req.method, HttpMethod::Put);
        assert_eq!(put_req.content_type, ContentType::Binary);
        assert_eq!(put_req.body, "file:///tmp/blob.bin");

        let binary =
            super::super::parse("curl -H 'Content-Type: image/png' --data-binary @payload.png https://a.test/bin")
                .expect("parsed");
        assert!(binary.lost.is_empty());
        let ImportItem::Api { request: bin_req, .. } = &binary.items[0] else {
            panic!("api");
        };
        assert_eq!(bin_req.method, HttpMethod::Post);
        assert_eq!(bin_req.content_type, ContentType::Binary);
        assert_eq!(bin_req.body, "file://payload.png");
        assert_eq!(bin_req.headers, vec![kv("Content-Type", "image/png", true)]);

        // `--data-raw` takes `@` literally, not as a file reference.
        let raw = super::super::parse("curl --data-raw '@literal' https://a.test/raw").expect("parsed");
        assert!(raw.lost.is_empty());
    }
}
