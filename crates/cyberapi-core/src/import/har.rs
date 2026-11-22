//! B5: an HTTP Archive (HAR 1.2), what a browser's developer tools save,
//! as APIs (F10.3).
//!
//! Each entry's request is an API named after its path, in a folder for
//! its host when the archive reaches more than one. What a browser loads
//! to draw a page — images, scripts, style sheets, fonts, media, as
//! Chrome's `_resourceType` says — and addresses that are not HTTP
//! (`data:`, `blob:`, an extension's) are left out, as is a request seen
//! before (the same method, URL and body). Headers the connection makes —
//! HTTP/2's pseudo headers, `Host`, `Content-Length`, `Connection` — are
//! left to the engine; the rest, cookies too, are kept as sent.

use super::lost::{self, Lost};
use super::{ImportBundle, ImportItem, api, content_type_of, guess_content_type, kv};
use crate::model::{ContentType, KvParam};
use serde_json::Value;

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// What a page loads to show itself, not an API.
const PAGE_PARTS: [&str; 6] = ["image", "stylesheet", "script", "font", "media", "manifest"];

/// What the connection, not the request, says.
const CONNECTION_HEADERS: [&str; 5] = [
    "host",
    "content-length",
    "connection",
    "keep-alive",
    "transfer-encoding",
];

/// `[{name, value}]`.
fn pairs(value: Option<&Value>) -> Vec<KvParam> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| kv(text(item, "name"), text(item, "value"), true))
                .collect()
        })
        .unwrap_or_default()
}

/// An entry's request as an API, its host, and any unimported binary body; `None` when it is not one.
fn entry(entry: &Value) -> Option<(String, ImportItem, Option<String>)> {
    if PAGE_PARTS.contains(&text(entry, "_resourceType")) {
        return None;
    }
    let request = entry.get("request")?;
    let url = text(request, "url");
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str().unwrap_or_default().to_string();
    let path = parsed.path();
    let (_, mut built) = api("", text(request, "method"), url);
    built.headers = pairs(request.get("headers"));
    built.headers.retain(|header| {
        let name = header.key.to_ascii_lowercase();
        !name.starts_with(':') && !CONNECTION_HEADERS.contains(&name.as_str())
    });
    built.headers.retain(|header| {
        if header.key.eq_ignore_ascii_case("content-type")
            && let Some(known) = content_type_of(&header.value)
        {
            built.content_type = known;
            return false;
        }
        true
    });
    let mut lost_body: Option<String> = None;
    if let Some(data) = request.get("postData") {
        let mime = text(data, "mimeType");
        if let Some(known) = content_type_of(mime) {
            built.content_type = known;
        }
        let params = data.get("params").and_then(Value::as_array);
        match built.content_type {
            ContentType::Form => {
                let fields = match params {
                    Some(params) if !params.is_empty() => pairs(data.get("params")),
                    _ => super::split_url(&format!("?{}", text(data, "text"))).1,
                };
                built.body = serde_json::to_string(&fields).unwrap_or_default();
            }
            ContentType::Multipart => {
                // A file sent is only its name here: the path is the
                // reader's to pick again.
                let fields: Vec<KvParam> = params
                    .map(|params| {
                        params
                            .iter()
                            .map(|param| match param.get("fileName").and_then(Value::as_str) {
                                Some(file) => kv(text(param, "name"), &format!("file://{file}"), true),
                                None => kv(text(param, "name"), text(param, "value"), true),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                built.body = serde_json::to_string(&fields).unwrap_or_default();
            }
            _ => {
                let encoding = text(data, "encoding");
                let raw_text = text(data, "text");
                let params_empty = params.is_none_or(Vec::is_empty);
                let is_binary = built.content_type == ContentType::Binary
                    || encoding == "base64"
                    || (!mime.is_empty() && raw_text.is_empty() && params_empty);
                if is_binary {
                    built.content_type = ContentType::Binary;
                    built.body = "file://".to_string();
                    if !mime.is_empty()
                        && !mime.eq_ignore_ascii_case(ContentType::Binary.as_str())
                        && !built
                            .headers
                            .iter()
                            .any(|header| header.key.eq_ignore_ascii_case("content-type"))
                    {
                        built.headers.push(kv("Content-Type", mime, true));
                    }
                    if encoding == "base64" && !raw_text.is_empty() {
                        lost_body = Some(if mime.is_empty() {
                            "binary (base64 inline data)".to_string()
                        } else {
                            format!("{mime} (base64 inline data)")
                        });
                    }
                } else {
                    built.body = raw_text.to_string();
                    if content_type_of(mime).is_none() && !built.body.is_empty() {
                        built.content_type = guess_content_type(&built.body);
                    }
                }
            }
        }
    }
    let name = if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    };
    Some((
        host,
        ImportItem::Api {
            name,
            request: Box::new(built),
        },
        lost_body,
    ))
}

struct HostEntries {
    host: String,
    items: Vec<(ImportItem, Option<String>)>,
}

pub(super) fn parse(json: &Value) -> ImportBundle {
    let entries = json
        .pointer("/log/entries")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut hosts: Vec<HostEntries> = Vec::new();
    for (host, item, lost_body) in entries.iter().filter_map(entry) {
        let ImportItem::Api { request, .. } = &item else {
            continue;
        };
        let key = (
            format!("{} {}?{:?}", request.method.as_str(), request.uri, request.query),
            request.body.clone(),
        );
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        match hosts.iter_mut().find(|known| known.host == host) {
            Some(known) => known.items.push((item, lost_body)),
            None => hosts.push(HostEntries {
                host,
                items: vec![(item, lost_body)],
            }),
        }
    }
    let mut lost: Vec<Lost> = Vec::new();
    let items = if hosts.len() == 1 {
        hosts
            .into_iter()
            .flat_map(|bucket| bucket.items)
            .map(|(item, body)| {
                if let Some(kind) = body
                    && let ImportItem::Api { name, .. } = &item
                {
                    lost::push_body(&mut lost, name, &kind);
                }
                item
            })
            .collect()
    } else {
        hosts
            .into_iter()
            .map(|bucket| {
                let host = bucket.host;
                let children = bucket
                    .items
                    .into_iter()
                    .map(|(item, body)| {
                        if let Some(kind) = body
                            && let ImportItem::Api { name, .. } = &item
                        {
                            let target = lost::join_path(&host, name);
                            lost::push_body(&mut lost, &target, &kind);
                        }
                        item
                    })
                    .collect();
                ImportItem::Folder {
                    name: host,
                    children,
                    settings: Default::default(),
                }
            })
            .collect()
    };
    ImportBundle {
        items,
        lost,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HttpMethod;
    use pretty_assertions::assert_eq;

    fn request(item: &ImportItem) -> &crate::model::HttpRequest {
        match item {
            ImportItem::Api { request, .. } => request,
            ImportItem::Folder { .. } => panic!("an API"),
        }
    }

    #[test]
    fn a_browsers_archive_brings_its_calls() {
        let json: Value = serde_json::from_str(
            r#"{"log": {"version": "1.2", "creator": {"name": "WebInspector"}, "entries": [
                {"_resourceType": "document", "request": {"method": "GET", "url": "https://shop.test/",
                    "headers": [{"name": ":authority", "value": "shop.test"}, {"name": "accept", "value": "text/html"}]}},
                {"_resourceType": "image", "request": {"method": "GET", "url": "https://shop.test/logo.png", "headers": []}},
                {"_resourceType": "xhr", "request": {"method": "POST", "url": "https://shop.test/v1/orders?draft=1",
                    "headers": [{"name": "Host", "value": "shop.test"}, {"name": "Content-Type", "value": "application/json"},
                                {"name": "Content-Length", "value": "12"}, {"name": "Cookie", "value": "sid=1"}],
                    "postData": {"mimeType": "application/json", "text": "{\"sku\": \"a\"}"}}},
                {"_resourceType": "xhr", "request": {"method": "POST", "url": "https://shop.test/v1/orders?draft=1",
                    "headers": [], "postData": {"mimeType": "application/json", "text": "{\"sku\": \"a\"}"}}},
                {"request": {"method": "POST", "url": "https://shop.test/login", "headers": [],
                    "postData": {"mimeType": "application/x-www-form-urlencoded; charset=UTF-8", "text": "user=me&pass=a%20b"}}},
                {"request": {"method": "POST", "url": "https://shop.test/upload", "headers": [],
                    "postData": {"mimeType": "multipart/form-data; boundary=x", "params": [
                        {"name": "note", "value": "hi"}, {"name": "file", "fileName": "a.png", "contentType": "image/png"}]}}},
                {"request": {"method": "GET", "url": "data:image/png;base64,AAAA", "headers": []}}
            ]}}"#,
        )
        .expect("JSON");
        let bundle = parse(&json);
        let names: Vec<&str> = bundle
            .items
            .iter()
            .map(|item| match item {
                ImportItem::Api { name, .. } => name.as_str(),
                ImportItem::Folder { name, .. } => name.as_str(),
            })
            .collect();
        assert_eq!(
            names,
            ["/", "/v1/orders", "/login", "/upload"],
            "one host, no folder; the image, the repeat and the data: URL left out"
        );
        assert_eq!(request(&bundle.items[0]).headers, [kv("accept", "text/html", true)]);
        let order = request(&bundle.items[1]);
        assert_eq!(order.method, HttpMethod::Post);
        assert_eq!(order.uri, "https://shop.test/v1/orders");
        assert_eq!(order.query, [kv("draft", "1", true)]);
        assert_eq!(order.headers, [kv("Cookie", "sid=1", true)]);
        assert_eq!(order.content_type, ContentType::Json);
        assert_eq!(order.body, "{\"sku\": \"a\"}");
        let login = request(&bundle.items[2]);
        assert_eq!(login.content_type, ContentType::Form);
        assert_eq!(
            crate::send::form_fields(&login.body),
            [kv("user", "me", true), kv("pass", "a b", true)]
        );
        let upload = request(&bundle.items[3]);
        assert_eq!(upload.content_type, ContentType::Multipart);
        assert_eq!(
            crate::send::form_fields(&upload.body),
            [kv("note", "hi", true), kv("file", "file://a.png", true)]
        );
    }

    #[test]
    fn each_host_is_a_folder() {
        let json: Value = serde_json::from_str(
            r#"{"log": {"entries": [
                {"request": {"method": "GET", "url": "https://a.test/x", "headers": []}},
                {"request": {"method": "GET", "url": "https://b.test/y", "headers": []}},
                {"request": {"method": "PUT", "url": "https://a.test/z", "headers": [],
                    "postData": {"mimeType": "application/vnd.api+json", "text": "[1]"}}}
            ]}}"#,
        )
        .expect("JSON");
        let bundle = parse(&json);
        let [
            ImportItem::Folder {
                name: first,
                children: a,
                ..
            },
            ImportItem::Folder { name: second, .. },
        ] = &bundle.items[..]
        else {
            panic!("two folders: {:?}", bundle.items);
        };
        assert_eq!((first.as_str(), second.as_str()), ("a.test", "b.test"));
        assert_eq!(a.len(), 2);
        assert_eq!(
            request(&a[1]).content_type,
            ContentType::Json,
            "a type this app has no name for is guessed from the body"
        );
    }

    #[test]
    fn binary_and_base64_bodies_are_imported_and_inline_base64_is_reported() {
        use super::super::LostKind;
        let json = r#"{"log": {"version": "1.2", "entries": [
            {"request": {"method": "POST", "url": "https://a.test/bin", "headers": [],
                "postData": {"mimeType": "application/octet-stream", "encoding": "base64", "text": "AQID"}}},
            {"request": {"method": "POST", "url": "https://b.test/empty", "headers": [],
                "postData": {"mimeType": "application/pdf"}}}
        ]}}"#;
        let bundle = super::super::parse(json).expect("parsed");
        let ImportItem::Folder { children: a, .. } = &bundle.items[0] else {
            panic!("folder a");
        };
        let ImportItem::Folder { children: b, .. } = &bundle.items[1] else {
            panic!("folder b");
        };
        assert_eq!(request(&a[0]).content_type, ContentType::Binary);
        assert_eq!(request(&a[0]).body, "file://");
        assert_eq!(request(&b[0]).content_type, ContentType::Binary);
        assert_eq!(request(&b[0]).body, "file://");
        assert!(
            request(&b[0])
                .headers
                .iter()
                .any(|h| h.key == "Content-Type" && h.value == "application/pdf")
        );
        assert_eq!(
            bundle.lost,
            vec![Lost {
                target: "a.test / /bin".into(),
                kind: LostKind::Body {
                    kind: "application/octet-stream (base64 inline data)".into(),
                },
            }]
        );
    }
}
