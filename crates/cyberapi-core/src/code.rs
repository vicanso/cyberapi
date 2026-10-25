//! A request as code to run elsewhere (#69): cURL, HTTPie, JavaScript's
//! `fetch`, Python's `requests`, Go's `net/http`. Made, as the cURL command
//! is (F7.8), from the [`PreparedRequest`], so each sends what the app
//! sends: variables resolved, functions run, headers merged, the jar's
//! cookies in. Redirects are followed, and certificates taken, as the app's
//! settings say. A gRPC call is a grpcurl command, whatever is asked for.

use crate::curl::curl;
use crate::model::{ContentType, HttpMethod, HttpRequest};
use crate::send::{PreparedRequest, form_fields};

/// What a request is copied as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CodeTarget {
    Curl,
    Httpie,
    Fetch,
    Python,
    Go,
}

impl CodeTarget {
    pub const ALL: [Self; 5] = [Self::Curl, Self::Httpie, Self::Fetch, Self::Python, Self::Go];

    /// Its name, as the menu lists it: not translated.
    pub fn label(self) -> &'static str {
        match self {
            Self::Curl => "cURL",
            Self::Httpie => "HTTPie",
            Self::Fetch => "JavaScript fetch",
            Self::Python => "Python requests",
            Self::Go => "Go net/http",
        }
    }
}

/// `request` is what the API holds, for its multipart fields.
pub fn code(target: CodeTarget, prepared: &PreparedRequest, request: &HttpRequest, follow_redirects: bool) -> String {
    if request.method == HttpMethod::Grpc || target == CodeTarget::Curl {
        return curl(prepared, request, &[], follow_redirects);
    }
    let parts = Parts::of(prepared, request);
    match target {
        CodeTarget::Curl => unreachable!("made above"),
        CodeTarget::Httpie => httpie(prepared, &parts, follow_redirects),
        CodeTarget::Fetch => fetch(prepared, &parts, follow_redirects),
        CodeTarget::Python => python(prepared, &parts, follow_redirects),
        CodeTarget::Go => go(prepared, &parts, follow_redirects),
    }
}

/// A multipart field: text, or a file's path.
enum Field {
    Text(String),
    File(String),
}

/// What every target writes out.
struct Parts<'a> {
    headers: Vec<(&'a str, &'a str)>,
    /// The body as text, JSON on one line; `None` for none, or a
    /// multipart body, whose fields stand for it.
    body: Option<String>,
    fields: Option<Vec<(String, Field)>>,
}

impl<'a> Parts<'a> {
    fn of(prepared: &'a PreparedRequest, request: &HttpRequest) -> Self {
        let multipart = prepared.method.has_body() && request.content_type == ContentType::Multipart;
        let fields = multipart.then(|| {
            form_fields(&request.body)
                .into_iter()
                .filter(|kv| kv.enabled && !kv.key.is_empty())
                .map(|kv| {
                    let field = match kv.value.strip_prefix("file://") {
                        Some(path) => Field::File(path.to_string()),
                        None => Field::Text(kv.value),
                    };
                    (kv.key, field)
                })
                .collect()
        });
        let body = (!multipart && !prepared.body.is_empty()).then(|| {
            let text = String::from_utf8_lossy(&prepared.body);
            serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .filter(|_| matches!(request.content_type, ContentType::Json | ContentType::GraphQl))
                .map(|json| json.to_string())
                .unwrap_or_else(|| text.into_owned())
        });
        // Each library writes its own multipart type, with its boundary.
        let headers = prepared
            .headers
            .iter()
            .filter(|(name, _)| !(multipart && name.eq_ignore_ascii_case("content-type")))
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        Self { headers, body, fields }
    }
}

/// A string literal JavaScript, Python and Go all read: JSON's.
fn string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

/// `'text'` for a POSIX shell.
fn shell(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The file name of a path, as a multipart field sends it.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn httpie(prepared: &PreparedRequest, parts: &Parts<'_>, follow_redirects: bool) -> String {
    let mut words = vec!["http".to_string(), "--verbose".to_string()];
    if follow_redirects {
        words.push("--follow".into());
    }
    if prepared.accept_invalid_certs {
        words.push("--verify=no".into());
    }
    if parts.fields.is_some() {
        words.push("--multipart".into());
    }
    if let Some(body) = &parts.body {
        words.push(format!("--raw {}", shell(body)));
    }
    words.push(prepared.method.as_str().to_string());
    words.push(shell(&prepared.url));
    for (name, value) in &parts.headers {
        // `Name;` is a header sent empty; `Name:` would leave it out.
        let item = if value.is_empty() {
            format!("{name};")
        } else {
            format!("{name}:{value}")
        };
        words.push(shell(&item));
    }
    for (name, field) in parts.fields.iter().flatten() {
        let item = match field {
            Field::Text(value) => format!("{name}={value}"),
            Field::File(path) => format!("{name}@{path}"),
        };
        words.push(shell(&item));
    }
    words.join(" ")
}

fn fetch(prepared: &PreparedRequest, parts: &Parts<'_>, follow_redirects: bool) -> String {
    let mut lines = Vec::new();
    let files = parts
        .fields
        .iter()
        .flatten()
        .any(|(_, field)| matches!(field, Field::File(_)));
    if files {
        lines.push("import { openAsBlob } from \"node:fs\";".to_string());
        lines.push(String::new());
    }
    if prepared.accept_invalid_certs {
        lines.push("// Certificates are not checked: in Node, run this with NODE_TLS_REJECT_UNAUTHORIZED=0.".into());
    }
    if let Some(fields) = &parts.fields {
        lines.push("const body = new FormData();".into());
        for (name, field) in fields {
            lines.push(match field {
                Field::Text(value) => format!("body.append({}, {});", string(name), string(value)),
                Field::File(path) => format!(
                    "body.append({}, await openAsBlob({}), {});",
                    string(name),
                    string(path),
                    string(file_name(path))
                ),
            });
        }
        lines.push(String::new());
    }
    lines.push(format!("const response = await fetch({}, {{", string(&prepared.url)));
    lines.push(format!("    method: {},", string(prepared.method.as_str())));
    if !parts.headers.is_empty() {
        lines.push("    headers: {".into());
        for (name, value) in &parts.headers {
            lines.push(format!("        {}: {},", string(name), string(value)));
        }
        lines.push("    },".into());
    }
    if parts.fields.is_some() {
        lines.push("    body,".into());
    } else if let Some(body) = &parts.body {
        lines.push(format!("    body: {},", string(body)));
    }
    if !follow_redirects {
        lines.push("    redirect: \"manual\",".into());
    }
    lines.push("});".into());
    lines.push("console.log(response.status, await response.text());".into());
    lines.join("\n")
}

fn python(prepared: &PreparedRequest, parts: &Parts<'_>, follow_redirects: bool) -> String {
    let mut lines = vec![
        "import requests".to_string(),
        String::new(),
        "response = requests.request(".to_string(),
        format!("    {},", string(prepared.method.as_str())),
        format!("    {},", string(&prepared.url)),
    ];
    if !parts.headers.is_empty() {
        lines.push("    headers={".into());
        for (name, value) in &parts.headers {
            lines.push(format!("        {}: {},", string(name), string(value)));
        }
        lines.push("    },".into());
    }
    // Every field as a file's: text alone would go URL-encoded.
    if let Some(fields) = &parts.fields {
        lines.push("    files={".into());
        for (name, field) in fields {
            lines.push(match field {
                Field::Text(value) => format!("        {}: (None, {}),", string(name), string(value)),
                Field::File(path) => format!("        {}: open({}, \"rb\"),", string(name), string(path)),
            });
        }
        lines.push("    },".into());
    } else if let Some(body) = &parts.body {
        lines.push(format!("    data={}.encode(),", string(body)));
    }
    lines.push(format!(
        "    allow_redirects={},",
        if follow_redirects { "True" } else { "False" }
    ));
    if prepared.accept_invalid_certs {
        lines.push("    verify=False,".into());
    }
    lines.push(")".into());
    lines.push("print(response.status_code)".into());
    lines.push("print(response.text)".into());
    lines.join("\n")
}

/// A Go string literal: raw when it can be, so a body reads as written.
fn go_string(text: &str) -> String {
    if text.contains(['`', '\r']) {
        string(text)
    } else {
        format!("`{text}`")
    }
}

fn go(prepared: &PreparedRequest, parts: &Parts<'_>, follow_redirects: bool) -> String {
    let multipart = parts.fields.is_some();
    let files = parts
        .fields
        .iter()
        .flatten()
        .any(|(_, field)| matches!(field, Field::File(_)));
    let mut imports = vec!["\"fmt\"", "\"io\"", "\"net/http\""];
    if multipart {
        imports.extend(["\"bytes\"", "\"mime/multipart\""]);
    }
    if files {
        imports.push("\"os\"");
    }
    if parts.body.is_some() {
        imports.push("\"strings\"");
    }
    if prepared.accept_invalid_certs {
        imports.push("\"crypto/tls\"");
    }
    imports.sort();
    imports.dedup();
    let check = ["\tif err != nil {", "\t\tpanic(err)", "\t}"].map(str::to_string);
    let mut lines = vec!["package main".to_string(), String::new(), "import (".to_string()];
    lines.extend(imports.iter().map(|import| format!("\t{import}")));
    lines.extend([")".to_string(), String::new(), "func main() {".to_string()]);
    let body = if let Some(fields) = &parts.fields {
        lines.push("\tvar body bytes.Buffer".into());
        lines.push("\tform := multipart.NewWriter(&body)".into());
        for (index, (name, field)) in fields.iter().enumerate() {
            match field {
                Field::Text(value) => lines.push(format!("\tform.WriteField({}, {})", string(name), go_string(value))),
                Field::File(path) => {
                    let (file, part) = (format!("file{index}"), format!("part{index}"));
                    lines.push(format!("\t{file}, err := os.Open({})", string(path)));
                    lines.extend(check.clone());
                    lines.push(format!("\tdefer {file}.Close()"));
                    lines.push(format!(
                        "\t{part}, err := form.CreateFormFile({}, {})",
                        string(name),
                        string(file_name(path))
                    ));
                    lines.extend(check.clone());
                    lines.push(format!("\tio.Copy({part}, {file})"));
                }
            }
        }
        lines.push("\tform.Close()".into());
        "&body".to_string()
    } else if let Some(body) = &parts.body {
        format!("strings.NewReader({})", go_string(body))
    } else {
        "nil".to_string()
    };
    lines.push(format!(
        "\treq, err := http.NewRequest({}, {}, {body})",
        string(prepared.method.as_str()),
        string(&prepared.url)
    ));
    lines.extend(check.clone());
    for (name, value) in &parts.headers {
        // Go sends the request's Host from its own field.
        if name.eq_ignore_ascii_case("host") {
            lines.push(format!("\treq.Host = {}", string(value)));
        } else {
            lines.push(format!("\treq.Header.Set({}, {})", string(name), string(value)));
        }
    }
    if multipart {
        lines.push("\treq.Header.Set(\"Content-Type\", form.FormDataContentType())".into());
    }
    lines.push("\tclient := &http.Client{".into());
    if !follow_redirects {
        lines.push(
            "\t\tCheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },".into(),
        );
    }
    if prepared.accept_invalid_certs {
        // Aligned with the field above it, as gofmt has it.
        let gap = if follow_redirects { " " } else { "     " };
        lines.push(format!(
            "\t\tTransport:{gap}&http.Transport{{TLSClientConfig: &tls.Config{{InsecureSkipVerify: true}}}},"
        ));
    }
    lines.push("\t}".into());
    lines.push("\tres, err := client.Do(req)".into());
    lines.extend(check);
    lines.extend(
        [
            "\tdefer res.Body.Close()",
            "\tdata, _ := io.ReadAll(res.Body)",
            "\tfmt.Println(res.Status)",
            "\tfmt.Println(string(data))",
            "}",
        ]
        .map(str::to_string),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::KvParam;
    use pretty_assertions::assert_eq;

    fn post() -> (PreparedRequest, HttpRequest) {
        let prepared = PreparedRequest {
            method: HttpMethod::Post,
            url: "https://api.test/v1/orders?x=1".into(),
            headers: vec![
                ("Authorization".into(), "Bearer t-1".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
            body: b"{\n  \"name\": \"o'neil\"\n}".to_vec(),
            ..Default::default()
        };
        let request = HttpRequest {
            method: HttpMethod::Post,
            content_type: ContentType::Json,
            ..Default::default()
        };
        (prepared, request)
    }

    fn multipart() -> (PreparedRequest, HttpRequest) {
        let kv = |key: &str, value: &str| KvParam {
            key: key.into(),
            value: value.into(),
            enabled: true,
        };
        let prepared = PreparedRequest {
            method: HttpMethod::Post,
            url: "https://api.test/upload".into(),
            headers: vec![("Content-Type".into(), "multipart/form-data; boundary=B".into())],
            body: b"--B...".to_vec(),
            ..Default::default()
        };
        let request = HttpRequest {
            method: HttpMethod::Post,
            content_type: ContentType::Multipart,
            body: serde_json::to_string(&[kv("a", "1"), kv("f", "file:///tmp/x.png")]).expect("json"),
            ..Default::default()
        };
        (prepared, request)
    }

    #[test]
    fn httpie_sends_the_body_raw() {
        let (prepared, request) = post();
        assert_eq!(
            code(CodeTarget::Httpie, &prepared, &request, false),
            r#"http --verbose --raw '{"name":"o'\''neil"}' POST 'https://api.test/v1/orders?x=1' 'Authorization:Bearer t-1' 'Content-Type:application/json'"#
        );
        let (prepared, request) = multipart();
        let insecure = PreparedRequest {
            accept_invalid_certs: true,
            ..prepared
        };
        assert_eq!(
            code(CodeTarget::Httpie, &insecure, &request, true),
            "http --verbose --follow --verify=no --multipart POST 'https://api.test/upload' 'a=1' 'f@/tmp/x.png'"
        );
    }

    #[test]
    fn fetch_writes_an_options_object() {
        let (prepared, request) = post();
        assert_eq!(
            code(CodeTarget::Fetch, &prepared, &request, false),
            r#"const response = await fetch("https://api.test/v1/orders?x=1", {
    method: "POST",
    headers: {
        "Authorization": "Bearer t-1",
        "Content-Type": "application/json",
    },
    body: "{\"name\":\"o'neil\"}",
    redirect: "manual",
});
console.log(response.status, await response.text());"#
        );
        let (prepared, request) = multipart();
        assert_eq!(
            code(CodeTarget::Fetch, &prepared, &request, true),
            r#"import { openAsBlob } from "node:fs";

const body = new FormData();
body.append("a", "1");
body.append("f", await openAsBlob("/tmp/x.png"), "x.png");

const response = await fetch("https://api.test/upload", {
    method: "POST",
    body,
});
console.log(response.status, await response.text());"#
        );
    }

    #[test]
    fn python_uses_requests() {
        let (prepared, request) = post();
        let insecure = PreparedRequest {
            accept_invalid_certs: true,
            ..prepared
        };
        assert_eq!(
            code(CodeTarget::Python, &insecure, &request, true),
            r#"import requests

response = requests.request(
    "POST",
    "https://api.test/v1/orders?x=1",
    headers={
        "Authorization": "Bearer t-1",
        "Content-Type": "application/json",
    },
    data="{\"name\":\"o'neil\"}".encode(),
    allow_redirects=True,
    verify=False,
)
print(response.status_code)
print(response.text)"#
        );
        let (prepared, request) = multipart();
        let python = code(CodeTarget::Python, &prepared, &request, false);
        assert!(
            python.contains(
                "    files={\n        \"a\": (None, \"1\"),\n        \"f\": open(\"/tmp/x.png\", \"rb\"),\n    },"
            ),
            "{python}"
        );
        assert!(!python.contains("multipart/form-data"), "requests writes its own");
    }

    #[test]
    fn go_builds_a_request_and_a_client() {
        let (prepared, request) = post();
        assert_eq!(
            code(CodeTarget::Go, &prepared, &request, false),
            r#"package main

import (
	"fmt"
	"io"
	"net/http"
	"strings"
)

func main() {
	req, err := http.NewRequest("POST", "https://api.test/v1/orders?x=1", strings.NewReader(`{"name":"o'neil"}`))
	if err != nil {
		panic(err)
	}
	req.Header.Set("Authorization", "Bearer t-1")
	req.Header.Set("Content-Type", "application/json")
	client := &http.Client{
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	}
	res, err := client.Do(req)
	if err != nil {
		panic(err)
	}
	defer res.Body.Close()
	data, _ := io.ReadAll(res.Body)
	fmt.Println(res.Status)
	fmt.Println(string(data))
}"#
        );
        let (prepared, request) = multipart();
        let go = code(CodeTarget::Go, &prepared, &request, true);
        for line in [
            "\t\"mime/multipart\"",
            "\tform.WriteField(\"a\", `1`)",
            "\tfile1, err := os.Open(\"/tmp/x.png\")",
            "\tpart1, err := form.CreateFormFile(\"f\", \"x.png\")",
            "\treq, err := http.NewRequest(\"POST\", \"https://api.test/upload\", &body)",
            "\treq.Header.Set(\"Content-Type\", form.FormDataContentType())",
        ] {
            assert!(go.lines().any(|candidate| candidate == line), "{line}\n{go}");
        }
        assert!(!go.contains("CheckRedirect"), "following, as Go does");
        let insecure = PreparedRequest {
            accept_invalid_certs: true,
            ..post().0
        };
        let go = code(CodeTarget::Go, &insecure, &request, false);
        assert!(
            go.contains("\t\tTransport:     &http.Transport{TLSClientConfig: &tls.Config{InsecureSkipVerify: true}},"),
            "aligned under CheckRedirect, as gofmt has it\n{go}"
        );
    }

    #[test]
    fn a_grpc_call_is_grpcurls_whatever_is_asked() {
        let (prepared, _) = post();
        let request = HttpRequest {
            method: HttpMethod::Grpc,
            ..Default::default()
        };
        for target in CodeTarget::ALL {
            assert!(code(target, &prepared, &request, false).starts_with("grpcurl "));
        }
    }
}
