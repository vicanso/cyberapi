//! R11: `.http` file export (REST Client / JetBrains HTTP Client), symmetric
//! with [`crate::import::http_file`].

use super::{effective_headers_and_query, flatten_apis, items_with_websocket, upsert_header};
use crate::ItemId;
use crate::graphql::GraphQlBody;
use crate::import::split_uri_query;
use crate::model::{ApiSetting, ContentType, HttpMethod, KvParam};
use crate::prefs::HttpVersion;
use crate::send::{binary_display_path, binary_file_name, form_fields, guessed_binary_content_type, is_file_value};
use crate::tree::TreeData;

/// Strips CR/LF characters so single-line fields stay on one line.
fn single_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// Percent-encodes URL query keys and values while keeping `{{...}}` template
/// spans intact so template expressions survive `.http` round-trips.
fn encode_query_part(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let push_encoded = |out: &mut String, segment: &str| {
        for byte in segment.bytes() {
            match byte {
                b' ' => out.push_str("%20"),
                b'%' => out.push_str("%25"),
                b'&' => out.push_str("%26"),
                b'=' => out.push_str("%3D"),
                b'#' => out.push_str("%23"),
                b'+' => out.push_str("%2B"),
                b'\r' => out.push_str("%0D"),
                b'\n' => out.push_str("%0A"),
                _ => out.push(byte as char),
            }
        }
    };

    while let Some(start) = rest.find("{{") {
        push_encoded(&mut out, &rest[..start]);
        let after = &rest[start..];
        if let Some(end) = after.find("}}") {
            let token = &after[..end + 2];
            out.push_str(&single_line(token));
            rest = &after[end + 2..];
        } else {
            push_encoded(&mut out, after);
            return out;
        }
    }
    push_encoded(&mut out, rest);
    out
}

fn build_url(raw_uri: &str, query: &[(String, String)]) -> String {
    let trimmed = single_line(raw_uri.trim());
    if query.is_empty() {
        return trimmed;
    }
    let (base, raw_qs, fragment) = split_uri_query(&trimmed);
    let mut url = String::from(base);
    if let Some(qs) = raw_qs {
        url.push('?');
        url.push_str(qs);
    }
    for (idx, (key, value)) in query.iter().enumerate() {
        let sep = if idx == 0 && raw_qs.is_none() { '?' } else { '&' };
        url.push(sep);
        url.push_str(&encode_query_part(key));
        url.push('=');
        url.push_str(&encode_query_part(value));
    }
    if let Some(frag) = fragment {
        url.push('#');
        url.push_str(frag);
    }
    url
}

fn header_present(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

fn choose_boundary(fields: &[KvParam]) -> String {
    let base = "CyberApiBoundary";
    let contains = |candidate: &str| fields.iter().any(|kv| kv.value.contains(candidate));
    if !contains(base) {
        return base.to_string();
    }
    for i in 1usize.. {
        let candidate = format!("{base}{i}");
        if !contains(&candidate) {
            return candidate;
        }
    }
    base.to_string()
}

fn build_body(api: &ApiSetting, headers: &mut Vec<(String, String)>) -> Option<String> {
    let request = &api.request;
    if request.method == HttpMethod::WebSocket {
        return (!request.body.trim().is_empty()).then(|| request.body.clone());
    }
    if !request.has_body_tab() {
        return None;
    }

    match request.content_type {
        ContentType::GraphQl => {
            let gql = GraphQlBody::read(&request.body);
            if gql.query.trim().is_empty() {
                return None;
            }
            let mut out = gql.query.trim_end().to_string();
            if !gql.variables.trim().is_empty() {
                out.push_str("\n\n");
                out.push_str(gql.variables.trim());
            }
            Some(out)
        }
        ContentType::Json | ContentType::Xml | ContentType::Plain if !request.body.trim().is_empty() => {
            if !header_present(headers, "content-type") {
                upsert_header(headers, "Content-Type", request.content_type.as_str());
            }
            Some(request.body.clone())
        }
        ContentType::Form => {
            let fields: Vec<_> = form_fields(&request.body)
                .into_iter()
                .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
                .collect();
            if fields.is_empty() {
                return None;
            }
            if !header_present(headers, "content-type") {
                upsert_header(headers, "Content-Type", ContentType::Form.as_str());
            }
            let encoded: Vec<String> = fields
                .iter()
                .map(|kv| format!("{}={}", encode_query_part(kv.key.trim()), encode_query_part(&kv.value)))
                .collect();
            Some(encoded.join("&"))
        }
        ContentType::Multipart => {
            let fields: Vec<_> = form_fields(&request.body)
                .into_iter()
                .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
                .collect();
            if fields.is_empty() {
                return None;
            }
            let boundary = choose_boundary(&fields);
            upsert_header(
                headers,
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            );
            let mut parts = String::new();
            for kv in &fields {
                let name = single_line(kv.key.trim()).replace('"', "");
                parts.push_str(&format!("--{boundary}\n"));
                if is_file_value(&kv.value) {
                    let path = binary_display_path(&kv.value).unwrap_or_default();
                    let fname = binary_file_name(&kv.value).unwrap_or(path);
                    let safe_fname = single_line(fname).replace('"', "");
                    if safe_fname.is_empty() {
                        parts.push_str(&format!("Content-Disposition: form-data; name=\"{name}\"\n\n"));
                    } else {
                        parts.push_str(&format!(
                            "Content-Disposition: form-data; name=\"{name}\"; filename=\"{safe_fname}\"\n\n"
                        ));
                    }
                    if !path.is_empty() {
                        parts.push_str(&format!("< {}\n", single_line(path)));
                    } else {
                        parts.push('\n');
                    }
                } else {
                    parts.push_str(&format!("Content-Disposition: form-data; name=\"{name}\"\n\n"));
                    parts.push_str(&kv.value);
                    if !kv.value.ends_with('\n') {
                        parts.push('\n');
                    }
                }
            }
            parts.push_str(&format!("--{boundary}--"));
            Some(parts)
        }
        ContentType::Binary if !request.body.trim().is_empty() => {
            if !header_present(headers, "content-type") {
                upsert_header(headers, "Content-Type", &guessed_binary_content_type(&request.body));
            }
            binary_display_path(&request.body).map(|path| format!("< {}", single_line(path)))
        }
        _ => None,
    }
}

/// Exports `folder` (or the whole project) as a `.http` file compatible with
/// VS Code REST Client and JetBrains HTTP Client. Only non-secret `variables`
/// and `global_headers` should be passed in.
pub fn http_file(
    data: &TreeData,
    folder: Option<&ItemId>,
    variables: &[(String, String)],
    global_headers: &[(String, String)],
) -> String {
    let apis = flatten_apis(&items_with_websocket(data, folder));
    let mut sections: Vec<String> = Vec::new();

    let mut var_lines = Vec::new();
    for (name, value) in variables {
        let name = name.trim();
        if !name.is_empty() && !name.contains(char::is_whitespace) {
            var_lines.push(format!("@{name} = {}", single_line(value)));
        }
    }
    if !var_lines.is_empty() {
        sections.push(var_lines.join("\n"));
    }

    for (api, folder_path) in apis {
        let mut lines = Vec::new();
        let title = if folder_path.is_empty() {
            single_line(&api.name)
        } else {
            format!("{} / {}", single_line(&folder_path), single_line(&api.name))
        };
        lines.push(format!("### {title}"));

        if !api.request.description.trim().is_empty() {
            for desc_line in api.request.description.trim().lines() {
                if desc_line.trim().is_empty() {
                    lines.push("#".into());
                } else {
                    lines.push(format!("# {}", desc_line.trim_end()));
                }
            }
        }

        if api.request.settings.follow_redirects == Some(false) {
            lines.push("# @no-redirect".into());
        }

        let (mut headers, query) = effective_headers_and_query(data, api, global_headers);
        let body = build_body(api, &mut headers);
        let url = build_url(&api.request.uri, &query);
        let version_suffix = if api.request.method != HttpMethod::WebSocket
            && api.request.settings.http_version == Some(HttpVersion::Http2)
        {
            " HTTP/2"
        } else {
            ""
        };
        let method = if api.request.method == HttpMethod::WebSocket {
            "WEBSOCKET"
        } else if api.request.content_type == ContentType::GraphQl && api.request.method == HttpMethod::Post {
            "GRAPHQL"
        } else {
            api.request.method.as_str()
        };
        lines.push(format!("{method} {url}{version_suffix}"));

        for (key, value) in &headers {
            lines.push(format!("{}: {}", single_line(key), single_line(value)));
        }

        if let Some(body) = body {
            lines.push(String::new());
            lines.push(body);
        }

        sections.push(lines.join("\n"));
    }

    if sections.is_empty() {
        String::new()
    } else {
        format!("{}\n", sections.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CollectionId;
    use crate::auth::{Auth, KeyPlace};
    use crate::model::{ApiCategory, Folder, FolderSettings, HttpMethod, HttpRequest, ProjectSettings};
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
    fn http_file_exports_variables_folders_methods_bodies_and_round_trips() {
        let data = TreeData {
            folders: vec![Folder {
                id: ItemId::new("f1"),
                collection: CollectionId::new("c1"),
                name: "Orders".into(),
                children: vec![ItemId::new("a_list"), ItemId::new("a_bin"), ItemId::new("a_grpc")],
                settings: FolderSettings {
                    headers: vec![kv("X-Folder", "orders", true)],
                    auth: Auth::Bearer {
                        token: "{{token}}".into(),
                    },
                    ..Default::default()
                },
                ..Default::default()
            }],
            apis: vec![
                api(
                    "a_list",
                    "List Orders",
                    HttpRequest {
                        method: HttpMethod::Get,
                        uri: "{{host}}/orders".into(),
                        description: "Fetches current orders.".into(),
                        query: vec![
                            kv("page", "1", true),
                            kv("tag", "a&b={{tagVar}}", true),
                            kv("skip", "no", false),
                        ],
                        headers: vec![kv("Accept", "application/json", true)],
                        settings: RequestSettings {
                            follow_redirects: Some(false),
                            http_version: Some(HttpVersion::Http2),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                ),
                api(
                    "a_bin",
                    "Send Blob",
                    HttpRequest {
                        method: HttpMethod::Put,
                        uri: "{{host}}/blob".into(),
                        content_type: ContentType::Binary,
                        body: "file:///tmp/data.bin".into(),
                        ..Default::default()
                    },
                ),
                api(
                    "a_form",
                    "Submit Form",
                    HttpRequest {
                        method: HttpMethod::Post,
                        uri: "{{host}}/form".into(),
                        auth: Auth::ApiKey {
                            name: "X-Key".into(),
                            value: "k1".into(),
                            place: KeyPlace::Header,
                        },
                        content_type: ContentType::Form,
                        body: serde_json::to_string(&vec![kv("user", "a b", true), kv("id", "{{uid}}", true)])
                            .expect("json"),
                        ..Default::default()
                    },
                ),
                api(
                    "a_multi",
                    "Upload Multi",
                    HttpRequest {
                        method: HttpMethod::Post,
                        uri: "{{host}}/upload".into(),
                        content_type: ContentType::Multipart,
                        body: serde_json::to_string(&vec![
                            kv("title", "hello", true),
                            kv("doc", "file:///tmp/readme.md", true),
                            kv("avatar", &page_file_marker("w1", "avatar.png"), true),
                        ])
                        .expect("json"),
                        ..Default::default()
                    },
                ),
                api(
                    "a_gql",
                    "Viewer Query",
                    HttpRequest {
                        method: HttpMethod::Post,
                        uri: "{{host}}/graphql".into(),
                        content_type: ContentType::GraphQl,
                        body: GraphQlBody {
                            query: "query User($id: ID!) { user(id: $id) { name } }".into(),
                            variables: r#"{"id": "42"}"#.into(),
                            operation_name: String::new(),
                        }
                        .to_body(),
                        ..Default::default()
                    },
                ),
                api(
                    "a_ws",
                    "Events Socket",
                    HttpRequest {
                        method: HttpMethod::WebSocket,
                        uri: "wss://api.test/ws".into(),
                        body: r#"{"hello":"world"}"#.into(),
                        ..Default::default()
                    },
                ),
                api(
                    "a_grpc",
                    "Ignored Grpc",
                    HttpRequest {
                        method: HttpMethod::Grpc,
                        uri: "localhost:50051".into(),
                        ..Default::default()
                    },
                ),
            ],
            top_order: vec![
                ItemId::new("f1"),
                ItemId::new("a_form"),
                ItemId::new("a_multi"),
                ItemId::new("a_gql"),
                ItemId::new("a_ws"),
            ],
            settings: ProjectSettings::default(),
        };

        let vars = vec![
            ("host".into(), "https://api.test".into()),
            ("token".into(), "secret-free".into()),
        ];
        let globals = vec![("X-App".into(), "cyber".into())];
        let out = http_file(&data, None, &vars, &globals);

        assert!(out.contains("@host = https://api.test\n"));
        assert!(out.contains("### Orders / List Orders\n"));
        assert!(out.contains("# Fetches current orders.\n"));
        assert!(out.contains("# @no-redirect\n"));
        assert!(out.contains("GET {{host}}/orders?page=1&tag=a%26b%3D{{tagVar}} HTTP/2\n"));
        assert!(out.contains("< /tmp/data.bin\n"));
        assert!(out.contains("GRAPHQL {{host}}/graphql\n"));
        assert!(out.contains("WEBSOCKET wss://api.test/ws\n"));
        assert!(!out.contains("Ignored Grpc"));

        // Folder-only export
        let folder_out = http_file(&data, Some(&ItemId::new("f1")), &vars, &globals);
        assert!(folder_out.contains("### List Orders\n"));
        assert!(folder_out.contains("### Send Blob\n"));
        assert!(!folder_out.contains("Submit Form"));

        // Round-trip via importer
        let (fmt, _) = crate::import::detect(&out).expect("detect");
        assert_eq!(fmt, crate::import::Format::Http);
        let bundle = crate::import::parse(&out).expect("round-trip .http parse");
        assert_eq!(bundle.environment, vars);
        assert_eq!(bundle.items.len(), 6);

        let crate::import::ImportItem::Api { name, request } = &bundle.items[0] else {
            panic!("api 0");
        };
        assert_eq!(name, "Orders / List Orders");
        assert_eq!(request.description, "Fetches current orders.");
        assert_eq!(request.settings.follow_redirects, Some(false));
        assert_eq!(request.settings.http_version, Some(HttpVersion::Http2));
        assert_eq!(
            request.query,
            vec![kv("page", "1", true), kv("tag", "a&b={{tagVar}}", true)]
        );
        assert!(
            request
                .headers
                .iter()
                .any(|h| h.key == "Authorization" && h.value == "Bearer {{token}}")
        );

        let crate::import::ImportItem::Api { request: form_req, .. } = &bundle.items[2] else {
            panic!("api 2");
        };
        assert_eq!(form_req.content_type, ContentType::Form);
        assert_eq!(
            form_fields(&form_req.body),
            vec![kv("user", "a b", true), kv("id", "{{uid}}", true)]
        );
        assert!(form_req.headers.iter().any(|h| h.key == "X-Key" && h.value == "k1"));

        let crate::import::ImportItem::Api { request: multi_req, .. } = &bundle.items[3] else {
            panic!("api 3");
        };
        assert_eq!(multi_req.content_type, ContentType::Multipart);
        assert_eq!(
            form_fields(&multi_req.body),
            vec![
                kv("title", "hello", true),
                kv("doc", "file:///tmp/readme.md", true),
                kv("avatar", "file://avatar.png", true),
            ]
        );

        let crate::import::ImportItem::Api { request: gql_req, .. } = &bundle.items[4] else {
            panic!("api 4");
        };
        assert_eq!(gql_req.content_type, ContentType::GraphQl);
        let gql = GraphQlBody::read(&gql_req.body);
        assert_eq!(gql.query, "query User($id: ID!) { user(id: $id) { name } }");
        assert_eq!(gql.variables, r#"{"id": "42"}"#);

        let crate::import::ImportItem::Api { request: ws_req, .. } = &bundle.items[5] else {
            panic!("api 5");
        };
        assert_eq!(ws_req.method, HttpMethod::WebSocket);
        assert_eq!(ws_req.uri, "wss://api.test/ws");
        assert_eq!(ws_req.body, r#"{"hello":"world"}"#);
    }
}
