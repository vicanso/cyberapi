//! R11: HTTP Archive (HAR 1.2) export, symmetric with [`crate::import::har`].

use super::{effective_headers_and_query, flatten_apis, items, upsert_header};
use crate::ItemId;
use crate::graphql::GraphQlBody;
use crate::import::{split_query_pairs, split_uri_query};
use crate::model::{ApiSetting, ContentType, split_env_prefix};
use crate::prefs::HttpVersion;
use crate::send::{binary_file_name, form_fields, guessed_binary_content_type, is_file_value};
use crate::tree::TreeData;
use serde_json::{Map, Value, json};

/// Resolves leading `{{ENV}}` prefix (when non-secret value exists in `variables`),
/// defaults bare host-like addresses to `http://`, merges query parameters from
/// `query` with any query pairs already embedded in the URI, and returns the
/// final URL alongside all HAR `queryString` name/value entries.
fn resolve_url(raw_uri: &str, variables: &[(String, String)], query: &[(String, String)]) -> (String, Vec<Value>) {
    let trimmed = raw_uri.trim();
    let with_env = match split_env_prefix(trimmed) {
        (Some(name), rest) => {
            if let Some((_, base)) = variables
                .iter()
                .find(|(var, val)| var == name && !val.trim().is_empty())
            {
                let base = base.trim().trim_end_matches('/');
                if rest.is_empty() {
                    base.to_string()
                } else if rest.starts_with(['/', '?', '#']) {
                    format!("{base}{rest}")
                } else {
                    format!("{base}/{rest}")
                }
            } else {
                trimmed.to_string()
            }
        }
        (None, _) => trimmed.to_string(),
    };

    let normalized = if !with_env.is_empty()
        && !with_env.contains("://")
        && !with_env.starts_with('/')
        && !with_env.starts_with("{{")
    {
        format!("http://{with_env}")
    } else {
        with_env
    };

    let (base, raw_qs, fragment) = split_uri_query(&normalized);
    let mut query_entries: Vec<Value> = Vec::new();
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    let mut has_qs = false;

    if let Some(qs) = raw_qs {
        for (raw_k, raw_v) in split_query_pairs(qs) {
            let decoded_pairs: Vec<(String, String)> =
                url::form_urlencoded::parse(format!("{raw_k}={raw_v}").as_bytes())
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
            if let Some((k, v)) = decoded_pairs.into_iter().next() {
                query_entries.push(json!({"name": k, "value": v}));
            } else {
                query_entries.push(json!({"name": raw_k, "value": raw_v}));
            }
        }
        if !qs.is_empty() {
            has_qs = true;
        }
    }

    for (key, value) in query {
        query_entries.push(json!({"name": key, "value": value}));
        serializer.append_pair(key, value);
    }

    let appended = serializer.finish();
    let mut final_url = String::from(base);
    if has_qs {
        if let Some(qs) = raw_qs {
            final_url.push('?');
            final_url.push_str(qs);
        }
        if !appended.is_empty() {
            final_url.push('&');
            final_url.push_str(&appended);
        }
    } else if !appended.is_empty() {
        final_url.push('?');
        final_url.push_str(&appended);
    }

    if let Some(frag) = fragment {
        final_url.push('#');
        final_url.push_str(frag);
    }

    (final_url, query_entries)
}

fn header_value<'a>(headers: &'a [(String, String)], target: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(target))
        .map(|(_, v)| v.as_str())
}

fn header_array(headers: &[(String, String)]) -> Vec<Value> {
    headers
        .iter()
        .map(|(name, value)| json!({"name": name, "value": value}))
        .collect()
}

fn build_post_data(api: &ApiSetting, headers: &[(String, String)]) -> Option<(Value, i64, String)> {
    let request = &api.request;
    if !request.has_body_tab() {
        return None;
    }

    match request.content_type {
        ContentType::Json | ContentType::Xml | ContentType::Plain if !request.body.trim().is_empty() => {
            let mime = header_value(headers, "content-type")
                .unwrap_or_else(|| request.content_type.as_str())
                .to_string();
            let size = request.body.len() as i64;
            Some((
                json!({
                    "mimeType": mime,
                    "text": request.body,
                }),
                size,
                mime,
            ))
        }
        ContentType::Form => {
            let fields: Vec<_> = form_fields(&request.body)
                .into_iter()
                .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
                .collect();
            if fields.is_empty() {
                return None;
            }
            let mime = ContentType::Form.as_str().to_string();
            let params: Vec<Value> = fields
                .iter()
                .map(|kv| json!({"name": kv.key.trim(), "value": kv.value}))
                .collect();
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            for kv in &fields {
                serializer.append_pair(kv.key.trim(), &kv.value);
            }
            let encoded = serializer.finish();
            let size = encoded.len() as i64;
            Some((
                json!({
                    "mimeType": mime,
                    "params": params,
                    "text": encoded,
                }),
                size,
                mime,
            ))
        }
        ContentType::Multipart => {
            let fields: Vec<_> = form_fields(&request.body)
                .into_iter()
                .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
                .collect();
            if fields.is_empty() {
                return None;
            }
            let mime = ContentType::Multipart.as_str().to_string();
            let params: Vec<Value> = fields
                .iter()
                .map(|kv| {
                    if is_file_value(&kv.value) {
                        let file_name = binary_file_name(&kv.value)
                            .unwrap_or_else(|| kv.value.trim().strip_prefix("file://").unwrap_or(""));
                        let content_type = if file_name.is_empty() {
                            ContentType::Binary.as_str().to_string()
                        } else {
                            mime_guess::from_path(file_name).first_or_octet_stream().to_string()
                        };
                        json!({
                            "name": kv.key.trim(),
                            "fileName": file_name,
                            "contentType": content_type,
                        })
                    } else {
                        json!({
                            "name": kv.key.trim(),
                            "value": kv.value,
                        })
                    }
                })
                .collect();
            Some((
                json!({
                    "mimeType": mime,
                    "params": params,
                    "text": "",
                }),
                -1,
                mime,
            ))
        }
        ContentType::Binary if !request.body.trim().is_empty() => {
            let mime = header_value(headers, "content-type")
                .map(str::to_string)
                .unwrap_or_else(|| guessed_binary_content_type(&request.body));
            let mut post_data = Map::new();
            post_data.insert("mimeType".into(), json!(mime));
            post_data.insert("text".into(), json!(""));
            if let Some(name) = binary_file_name(&request.body) {
                post_data.insert("comment".into(), json!(name));
            }
            Some((Value::Object(post_data), -1, mime))
        }
        ContentType::GraphQl => {
            let gql = GraphQlBody::read(&request.body);
            if gql.query.trim().is_empty() {
                return None;
            }
            let mime = ContentType::Json.as_str().to_string();
            let payload_text = match gql.payload() {
                Ok(value) => serde_json::to_string(&value).unwrap_or_default(),
                Err(_) => {
                    let mut fields = Map::new();
                    fields.insert("query".into(), json!(gql.query));
                    if !gql.variables.trim().is_empty() {
                        fields.insert("variables".into(), json!(gql.variables));
                    }
                    if !gql.operation_name.trim().is_empty() {
                        fields.insert("operationName".into(), json!(gql.operation_name.trim()));
                    }
                    serde_json::to_string(&Value::Object(fields)).unwrap_or_default()
                }
            };
            let size = payload_text.len() as i64;
            Some((
                json!({
                    "mimeType": mime,
                    "text": payload_text,
                }),
                size,
                mime,
            ))
        }
        _ => None,
    }
}

fn entry_response(api: &ApiSetting, http_version: &str) -> Value {
    if let Some(example) = api.request.examples.first() {
        let mime_type = example.content_type().unwrap_or("");
        json!({
            "status": example.status,
            "statusText": crate::response::status_text(example.status),
            "httpVersion": http_version,
            "cookies": [],
            "headers": header_array(&example.headers),
            "content": {
                "size": example.body.len(),
                "mimeType": mime_type,
                "text": example.body,
            },
            "redirectURL": "",
            "headersSize": -1,
            "bodySize": example.body.len(),
        })
    } else {
        json!({
            "status": 0,
            "statusText": "",
            "httpVersion": http_version,
            "cookies": [],
            "headers": [],
            "content": {
                "size": 0,
                "mimeType": "",
            },
            "redirectURL": "",
            "headersSize": -1,
            "bodySize": 0,
        })
    }
}

/// Exports `folder` (or the whole project) as an HTTP Archive (HAR 1.2) JSON
/// document. Only non-secret active environment `variables` and
/// `global_headers` should be passed in.
pub fn har(
    data: &TreeData,
    folder: Option<&ItemId>,
    variables: &[(String, String)],
    global_headers: &[(String, String)],
) -> String {
    let apis = flatten_apis(&items(data, folder));
    let mut entries = Vec::with_capacity(apis.len());

    for (api, folder_path) in apis {
        let (mut headers, query) = effective_headers_and_query(data, api, global_headers);
        let (url, query_string) = resolve_url(&api.request.uri, variables, &query);
        let http_version = if api.request.settings.http_version == Some(HttpVersion::Http2) {
            "HTTP/2.0"
        } else {
            "HTTP/1.1"
        };

        let post_data = build_post_data(api, &headers);
        let body_size = if let Some((_, size, ref mime)) = post_data {
            if header_value(&headers, "content-type").is_none() {
                upsert_header(&mut headers, "Content-Type", mime);
            }
            size
        } else {
            0
        };

        let mut request_obj = Map::new();
        request_obj.insert("method".into(), json!(api.request.method.as_str()));
        request_obj.insert("url".into(), json!(url));
        request_obj.insert("httpVersion".into(), json!(http_version));
        request_obj.insert("cookies".into(), json!([]));
        request_obj.insert("headers".into(), Value::Array(header_array(&headers)));
        request_obj.insert("queryString".into(), Value::Array(query_string));
        request_obj.insert("headersSize".into(), json!(-1));
        request_obj.insert("bodySize".into(), json!(body_size));
        if let Some((pd, _, _)) = post_data {
            request_obj.insert("postData".into(), pd);
        }

        let comment = if folder_path.is_empty() {
            api.name.clone()
        } else {
            format!("{folder_path} / {}", api.name)
        };

        let mut entry = Map::new();
        entry.insert("startedDateTime".into(), json!("1970-01-01T00:00:00.000Z"));
        entry.insert("time".into(), json!(0));
        if !comment.trim().is_empty() {
            entry.insert("comment".into(), json!(comment));
        }
        entry.insert("request".into(), Value::Object(request_obj));
        entry.insert("response".into(), entry_response(api, http_version));
        entry.insert("cache".into(), json!({}));
        entry.insert("timings".into(), json!({"send": 0, "wait": 0, "receive": 0}));
        entries.push(Value::Object(entry));
    }

    let document = json!({
        "log": {
            "version": "1.2",
            "creator": {
                "name": "CyberAPI",
                "version": crate::version::VERSION,
            },
            "entries": entries,
        }
    });
    crate::format::json(&document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CollectionId;
    use crate::auth::{Auth, KeyPlace};
    use crate::model::{
        ApiCategory, Example, Folder, FolderSettings, HttpMethod, HttpRequest, KvParam, ProjectSettings,
    };
    use crate::prefs::RequestSettings;
    use crate::send::page_file_marker;
    use pretty_assertions::assert_eq;

    fn kv(key: &str, value: &str, enabled: bool) -> KvParam {
        KvParam {
            key: key.into(),
            value: value.into(),
            enabled,
            ..Default::default()
        }
    }

    fn api(id: &str, name: &str, request: HttpRequest) -> ApiSetting {
        ApiSetting {
            id: ItemId::new(id),
            collection: CollectionId::new("c1"),
            name: name.into(),
            category: ApiCategory::Http,
            request,
            ..Default::default()
        }
    }

    #[test]
    fn har_exports_project_with_bodies_headers_auth_examples_and_skips_grpc_ws() {
        let data = TreeData {
            folders: vec![Folder {
                id: ItemId::new("f1"),
                collection: CollectionId::new("c1"),
                name: "Users".into(),
                children: vec![ItemId::new("a_json"), ItemId::new("a_multi"), ItemId::new("a_grpc")],
                settings: FolderSettings {
                    headers: vec![kv("X-Folder", "users", true)],
                    auth: Auth::Bearer {
                        token: "folder-tok".into(),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }],
            apis: vec![
                api(
                    "a_json",
                    "Create User",
                    HttpRequest {
                        method: HttpMethod::Post,
                        uri: "{{HOST}}/v1/users?existing=yes".into(),
                        query: vec![kv("role", "admin", true), kv("off", "skip", false)],
                        headers: vec![kv("X-Custom", "v1", true)],
                        content_type: ContentType::Json,
                        body: r#"{"name":"Alice"}"#.into(),
                        settings: RequestSettings {
                            http_version: Some(HttpVersion::Http2),
                            ..Default::default()
                        },
                        examples: vec![Example {
                            name: "ok".into(),
                            status: 201,
                            headers: vec![("Content-Type".into(), "application/json".into())],
                            body: r#"{"id":1}"#.into(),
                        }],
                        ..Default::default()
                    },
                ),
                api(
                    "a_multi",
                    "Upload Avatar",
                    HttpRequest {
                        method: HttpMethod::Post,
                        uri: "https://api.example.com/upload".into(),
                        content_type: ContentType::Multipart,
                        body: serde_json::to_string(&vec![
                            kv("label", "avatar", true),
                            kv("file", "/tmp/pic.png", false),
                            kv("photo", &page_file_marker("wf1", "photo.png"), true),
                        ])
                        .expect("json"),
                        ..Default::default()
                    },
                ),
                api(
                    "a_form",
                    "Login Form",
                    HttpRequest {
                        method: HttpMethod::Post,
                        uri: "auth.example.com/login".into(),
                        auth: Auth::ApiKey {
                            name: "api_key".into(),
                            value: "k-123".into(),
                            place: KeyPlace::Query,
                        },
                        content_type: ContentType::Form,
                        body: serde_json::to_string(&vec![kv("user", "ann", true), kv("pass", "s e c", true)])
                            .expect("json"),
                        ..Default::default()
                    },
                ),
                api(
                    "a_bin",
                    "Upload Binary",
                    HttpRequest {
                        method: HttpMethod::Put,
                        uri: "https://api.example.com/bin".into(),
                        content_type: ContentType::Binary,
                        body: "file:///tmp/report.pdf".into(),
                        ..Default::default()
                    },
                ),
                api(
                    "a_gql",
                    "Get Viewer",
                    HttpRequest {
                        method: HttpMethod::Post,
                        uri: "https://api.example.com/graphql".into(),
                        content_type: ContentType::GraphQl,
                        body: GraphQlBody {
                            query: "query Me { me { id } }".into(),
                            variables: r#"{"limit": 1}"#.into(),
                            operation_name: "Me".into(),
                        }
                        .to_body(),
                        ..Default::default()
                    },
                ),
                api(
                    "a_grpc",
                    "SayHello",
                    HttpRequest {
                        method: HttpMethod::Grpc,
                        uri: "localhost:50051".into(),
                        ..Default::default()
                    },
                ),
                api(
                    "a_ws",
                    "Stream",
                    HttpRequest {
                        method: HttpMethod::WebSocket,
                        uri: "wss://api.example.com/ws".into(),
                        ..Default::default()
                    },
                ),
            ],
            top_order: vec![
                ItemId::new("f1"),
                ItemId::new("a_form"),
                ItemId::new("a_bin"),
                ItemId::new("a_gql"),
                ItemId::new("a_ws"),
            ],
            settings: ProjectSettings::default(),
        };

        let env = vec![("HOST".into(), "https://api.example.com/".into())];
        let globals = vec![("X-Global".into(), "g1".into())];
        let text = har(&data, None, &env, &globals);
        let parsed: Value = serde_json::from_str(&text).expect("valid json");

        assert_eq!(parsed["log"]["version"], "1.2");
        assert_eq!(parsed["log"]["creator"]["name"], "CyberAPI");
        let entries = parsed["log"]["entries"].as_array().expect("entries");
        assert_eq!(entries.len(), 5, "grpc and ws skipped");

        // Entry 0: JSON in folder
        let e0 = &entries[0];
        assert_eq!(e0["comment"], "Users / Create User");
        assert_eq!(e0["request"]["httpVersion"], "HTTP/2.0");
        assert_eq!(
            e0["request"]["url"],
            "https://api.example.com/v1/users?existing=yes&role=admin"
        );
        assert_eq!(e0["request"]["postData"]["mimeType"], "application/json");
        assert_eq!(e0["request"]["postData"]["text"], r#"{"name":"Alice"}"#);
        assert_eq!(e0["response"]["status"], 201);
        assert_eq!(e0["response"]["statusText"], "Created");
        assert_eq!(e0["response"]["content"]["text"], r#"{"id":1}"#);
        let e0_headers = e0["request"]["headers"].as_array().expect("headers");
        assert!(e0_headers.iter().any(|h| h["name"] == "X-Global" && h["value"] == "g1"));
        assert!(
            e0_headers
                .iter()
                .any(|h| h["name"] == "X-Folder" && h["value"] == "users")
        );
        assert!(
            e0_headers
                .iter()
                .any(|h| h["name"] == "Authorization" && h["value"] == "Bearer folder-tok")
        );

        // Entry 1: Multipart with webfile
        let e1_params = entries[1]["request"]["postData"]["params"].as_array().expect("params");
        assert_eq!(e1_params.len(), 2);
        assert_eq!(e1_params[0], json!({"name": "label", "value": "avatar"}));
        assert_eq!(e1_params[1]["fileName"], "photo.png");
        assert_eq!(e1_params[1]["contentType"], "image/png");

        // Entry 2: Form + bare URL defaulted to http:// + ApiKey in query
        assert_eq!(
            entries[2]["request"]["url"],
            "http://auth.example.com/login?api_key=k-123"
        );
        assert_eq!(entries[2]["request"]["postData"]["text"], "user=ann&pass=s+e+c");

        // Entry 3: Binary
        assert_eq!(entries[3]["request"]["postData"]["mimeType"], "application/pdf");
        assert_eq!(entries[3]["request"]["postData"]["comment"], "report.pdf");
        assert_eq!(entries[3]["request"]["bodySize"], -1);

        // Entry 4: GraphQL
        let gql_text = entries[4]["request"]["postData"]["text"].as_str().expect("text");
        let gql_val: Value = serde_json::from_str(gql_text).expect("gql json");
        assert_eq!(gql_val["query"], "query Me { me { id } }");
        assert_eq!(gql_val["variables"], json!({"limit": 1}));
        assert_eq!(gql_val["operationName"], "Me");

        // Folder-only export
        let folder_text = har(&data, Some(&ItemId::new("f1")), &env, &globals);
        let folder_parsed: Value = serde_json::from_str(&folder_text).expect("folder json");
        assert_eq!(folder_parsed["log"]["entries"].as_array().expect("arr").len(), 2);

        // Round-trip through HAR importer
        let (fmt, _) = crate::import::detect(&text).expect("detect");
        assert_eq!(fmt, crate::import::Format::Har);
        let bundle = crate::import::parse(&text).expect("round-trip har import");
        assert!(bundle.lost.is_empty(), "{:?}", bundle.lost);
        assert_eq!(bundle.counts(), (5, 2));
    }
}
