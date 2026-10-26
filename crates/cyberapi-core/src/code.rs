//! A request as code to run elsewhere (#69): cURL, HTTPie, JavaScript's
//! `fetch`, Python's `requests`, Go's `net/http`; and (B6) Node's axios,
//! PHP's Guzzle, Rust's reqwest, Java's and C#'s `HttpClient`, Swift's
//! `URLSession`. Made, as the cURL command is (F7.8), from the
//! [`PreparedRequest`], so each sends what the app sends: variables
//! resolved, functions run, headers merged, the jar's cookies in. Redirects
//! are followed, and certificates taken, as the app's settings say. A gRPC
//! call is a grpcurl command, whatever is asked for.

mod axios;
mod csharp;
mod java;
mod php;
mod reqwest;
mod swift;

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
    /// B6.
    Axios,
    Php,
    Rust,
    Java,
    CSharp,
    Swift,
}

impl CodeTarget {
    /// The menu's groups, cURL first: the command line, scripts, compiled
    /// languages.
    pub const GROUPS: [&'static [Self]; 3] = [
        &[Self::Curl, Self::Httpie],
        &[Self::Fetch, Self::Axios, Self::Python, Self::Php],
        &[Self::Go, Self::Rust, Self::Java, Self::CSharp, Self::Swift],
    ];

    pub const ALL: [Self; 11] = [
        Self::Curl,
        Self::Httpie,
        Self::Fetch,
        Self::Axios,
        Self::Python,
        Self::Php,
        Self::Go,
        Self::Rust,
        Self::Java,
        Self::CSharp,
        Self::Swift,
    ];

    /// Its name, as the menu lists it: not translated.
    pub fn label(self) -> &'static str {
        match self {
            Self::Curl => "cURL",
            Self::Httpie => "HTTPie",
            Self::Fetch => "JavaScript fetch",
            Self::Python => "Python requests",
            Self::Go => "Go net/http",
            Self::Axios => "Node.js axios",
            Self::Php => "PHP Guzzle",
            Self::Rust => "Rust reqwest",
            Self::Java => "Java HttpClient",
            Self::CSharp => "C# HttpClient",
            Self::Swift => "Swift URLSession",
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
        CodeTarget::Axios => axios::axios(prepared, &parts, follow_redirects),
        CodeTarget::Php => php::guzzle(prepared, &parts, follow_redirects),
        CodeTarget::Rust => reqwest::reqwest(prepared, &parts, follow_redirects),
        CodeTarget::Java => java::http_client(prepared, &parts, follow_redirects),
        CodeTarget::CSharp => csharp::http_client(prepared, &parts, follow_redirects),
        CodeTarget::Swift => swift::url_session(prepared, &parts, follow_redirects),
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

/// The boundary of a multipart body written out by hand, for a library
/// with nothing to write one (Java's, Swift's).
const BOUNDARY: &str = "CyberAPIFormBoundary";

/// What goes before a multipart field's value: its boundary and headers.
fn part_head(name: &str, file: Option<&str>) -> String {
    let quoted = |text: &str| text.replace('"', "%22");
    match file {
        Some(file) => format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",
            quoted(name),
            quoted(file_name(file))
        ),
        None => format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n",
            quoted(name)
        ),
    }
}

/// Whether a multipart body sends a file.
fn sends_files(parts: &Parts<'_>) -> bool {
    parts
        .fields
        .iter()
        .flatten()
        .any(|(_, field)| matches!(field, Field::File(_)))
}

fn httpie(prepared: &PreparedRequest, parts: &Parts<'_>, follow_redirects: bool) -> String {
    let mut words = vec!["http".to_string(), "--verbose".to_string()];
    if follow_redirects {
        words.push("--follow".into());
    }
    if prepared.accept_invalid_certs {
        words.push("--verify=no".into());
    }
    if let Some(login) = &prepared.digest {
        words.push("--auth-type=digest".into());
        words.push(format!(
            "--auth={}",
            shell(&format!("{}:{}", login.username, login.password))
        ));
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
    if let Some(login) = &prepared.digest {
        lines.push(unanswered_digest("//", &login.username, "fetch"));
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
            // #77: Node sends it; a browser drops it, unasked.
            if name.eq_ignore_ascii_case("cookie") {
                lines.push(
                    "        // A browser drops this header; there, credentials: \"include\" sends the browser's own cookies."
                        .into(),
                );
            }
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

/// C2: a comment for a library that answers no Digest challenge itself.
fn unanswered_digest(comment: &str, username: &str, library: &str) -> String {
    format!(
        "{comment} It signs in with Digest as {}, which {library} does not answer by itself.",
        string(username)
    )
}

fn python(prepared: &PreparedRequest, parts: &Parts<'_>, follow_redirects: bool) -> String {
    let mut lines = vec!["import requests".to_string()];
    if prepared.digest.is_some() {
        lines.push("from requests.auth import HTTPDigestAuth".to_string());
    }
    lines.extend([
        String::new(),
        "response = requests.request(".to_string(),
        format!("    {},", string(prepared.method.as_str())),
        format!("    {},", string(&prepared.url)),
    ]);
    if let Some(login) = &prepared.digest {
        lines.push(format!(
            "    auth=HTTPDigestAuth({}, {}),",
            string(&login.username),
            string(&login.password)
        ));
    }
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
    if let Some(login) = &prepared.digest {
        lines.push(unanswered_digest("\t//", &login.username, "net/http"));
    }
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

    /// C5: cURL presents the client certificate and trusts the authority.
    #[test]
    fn curl_takes_the_certificates() {
        use crate::certificates::{CertificateFormat, ClientCertificate, Tls};
        let request = HttpRequest::default();
        let with = |client: ClientCertificate, authorities: Vec<String>| {
            let prepared = PreparedRequest {
                method: HttpMethod::Get,
                url: "https://api.test/".into(),
                tls: Tls {
                    client: Some(client),
                    authorities,
                },
                ..Default::default()
            };
            code(CodeTarget::Curl, &prepared, &request, false)
        };
        let pem = ClientCertificate {
            host: "api.test".into(),
            certificate: "/certs/me.pem".into(),
            key: "/certs/me.key".into(),
            ..Default::default()
        };
        assert!(
            with(pem.clone(), vec!["/certs/ca.pem".into()])
                .contains("--cert '/certs/me.pem' --key '/certs/me.key' --cacert '/certs/ca.pem'")
        );
        let bundle = ClientCertificate {
            format: CertificateFormat::Pkcs12,
            certificate: "/certs/me.p12".into(),
            password: "pw".into(),
            ..pem
        };
        let two = with(bundle, vec!["a.pem".into(), "b.pem".into()]);
        assert!(two.contains("--cert-type P12 --cert '/certs/me.p12:pw'"), "{two}");
        assert!(!two.contains("--cacert"), "curl takes one bundle: {two}");
    }

    /// C2: what answers a Digest challenge, where the target can.
    #[test]
    fn digest_is_answered_where_the_target_can() {
        let prepared = PreparedRequest {
            method: HttpMethod::Get,
            url: "https://cam.test/ISAPI/System/deviceInfo".into(),
            digest: Some(crate::digest::DigestLogin {
                username: "admin".into(),
                password: "it's".into(),
            }),
            ..Default::default()
        };
        let request = HttpRequest::default();
        let of = |target| code(target, &prepared, &request, false);
        assert!(
            of(CodeTarget::Curl).contains(r"--digest -u 'admin:it'\''s'"),
            "{}",
            of(CodeTarget::Curl)
        );
        assert!(of(CodeTarget::Httpie).contains(r"--auth-type=digest --auth='admin:it'\''s'"));
        let python = of(CodeTarget::Python);
        assert!(python.starts_with("import requests\nfrom requests.auth import HTTPDigestAuth\n"));
        assert!(
            python.contains(r#"    auth=HTTPDigestAuth("admin", "it's"),"#),
            "{python}"
        );
        assert!(of(CodeTarget::Fetch).contains(r#"// It signs in with Digest as "admin", which fetch does not"#));
        assert!(of(CodeTarget::Go).contains(r#"	// It signs in with Digest as "admin", which net/http does not"#));
        // B6: Guzzle, .NET and URLSession answer it; the others say so.
        assert!(of(CodeTarget::Php).contains(r"    'auth' => ['admin', 'it\'s', 'digest'],"));
        assert!(of(CodeTarget::CSharp).contains(r#"    Credentials = new NetworkCredential("admin", "it's"),"#));
        assert!(of(CodeTarget::Swift).contains(
            r#"return (.useCredential, URLCredential(user: "admin", password: "it's", persistence: .forSession))"#
        ));
        for (target, library) in [
            (CodeTarget::Axios, "axios"),
            (CodeTarget::Rust, "reqwest"),
            (CodeTarget::Java, "HttpClient"),
        ] {
            assert!(
                of(target).contains(&format!(
                    r#"// It signs in with Digest as "admin", which {library} does not"#
                )),
                "{}",
                of(target)
            );
        }
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

    /// #77: a browser's fetch drops a Cookie header it is given.
    #[test]
    fn fetch_says_a_browser_drops_its_cookie_header() {
        let prepared = PreparedRequest {
            method: HttpMethod::Get,
            url: "http://127.0.0.1:8765/echo".into(),
            headers: vec![
                ("X-Pm".into(), "1".into()),
                ("Cookie".into(), "a=1; session=abc".into()),
            ],
            ..Default::default()
        };
        assert_eq!(
            code(CodeTarget::Fetch, &prepared, &HttpRequest::default(), true),
            r#"const response = await fetch("http://127.0.0.1:8765/echo", {
    method: "GET",
    headers: {
        "X-Pm": "1",
        // A browser drops this header; there, credentials: "include" sends the browser's own cookies.
        "Cookie": "a=1; session=abc",
    },
});
console.log(response.status, await response.text());"#
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

    /// B6: every status an answer, the body read as text.
    #[test]
    fn axios_takes_every_status() {
        let (prepared, request) = post();
        assert_eq!(
            code(CodeTarget::Axios, &prepared, &request, false),
            r#"import axios from "axios";

const response = await axios.request({
    method: "POST",
    url: "https://api.test/v1/orders?x=1",
    headers: {
        "Authorization": "Bearer t-1",
        "Content-Type": "application/json",
    },
    data: "{\"name\":\"o'neil\"}",
    maxRedirects: 0,
    responseType: "text",
    validateStatus: () => true,
});
console.log(response.status, response.data);"#
        );
        let (prepared, request) = multipart();
        let insecure = PreparedRequest {
            accept_invalid_certs: true,
            ..prepared
        };
        let axios = code(CodeTarget::Axios, &insecure, &request, true);
        for line in [
            "import { openAsBlob } from \"node:fs\";",
            "import https from \"node:https\";",
            "body.append(\"f\", await openAsBlob(\"/tmp/x.png\"), \"x.png\");",
            "    data: body,",
            "    httpsAgent: new https.Agent({ rejectUnauthorized: false }),",
        ] {
            assert!(axios.lines().any(|candidate| candidate == line), "{line}\n{axios}");
        }
        assert!(!axios.contains("maxRedirects"), "following, as axios does");
    }

    #[test]
    fn guzzle_quotes_as_php_reads_single_quotes() {
        let (prepared, request) = post();
        let insecure = PreparedRequest {
            accept_invalid_certs: true,
            ..prepared
        };
        assert_eq!(
            code(CodeTarget::Php, &insecure, &request, true),
            r#"<?php
require 'vendor/autoload.php';

use GuzzleHttp\Client;

$client = new Client();
$response = $client->request('POST', 'https://api.test/v1/orders?x=1', [
    'headers' => [
        'Authorization' => 'Bearer t-1',
        'Content-Type' => 'application/json',
    ],
    'body' => '{"name":"o\'neil"}',
    'verify' => false,
    'http_errors' => false,
]);
echo $response->getStatusCode(), "\n";
echo $response->getBody(), "\n";"#
        );
        let (prepared, request) = multipart();
        let php = code(CodeTarget::Php, &prepared, &request, false);
        assert!(
            php.contains(
                "    'multipart' => [\n        ['name' => 'a', 'contents' => '1'],\n        ['name' => 'f', 'contents' => fopen('/tmp/x.png', 'r'), 'filename' => 'x.png'],\n    ],"
            ),
            "{php}"
        );
        assert!(php.contains("    'allow_redirects' => false,"));
        assert!(!php.contains("multipart/form-data"), "Guzzle writes its own");
    }

    #[test]
    fn reqwest_blocks_and_reads_a_body_as_written() {
        let (prepared, request) = post();
        assert_eq!(
            code(CodeTarget::Rust, &prepared, &request, false),
            r##"// reqwest, with its "blocking" feature.
use reqwest::blocking::Client;
use reqwest::redirect::Policy;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder()
        .redirect(Policy::none())
        .build()?;
    let response = client
        .request(reqwest::Method::POST, "https://api.test/v1/orders?x=1")
        .header("Authorization", "Bearer t-1")
        .header("Content-Type", "application/json")
        .body(r#"{"name":"o'neil"}"#)
        .send()?;
    println!("{}", response.status());
    println!("{}", response.text()?);
    Ok(())
}"##
        );
        let (prepared, request) = multipart();
        let rust = code(CodeTarget::Rust, &prepared, &request, true);
        assert!(rust.starts_with("// reqwest, with its \"blocking\" and \"multipart\" features."));
        assert!(
            rust.contains("    let client = Client::new();\n    let form = Form::new()\n        .text(\"a\", \"1\")\n        .file(\"f\", \"/tmp/x.png\")?;\n"),
            "{rust}"
        );
        assert!(rust.contains("        .multipart(form)\n"));
    }

    #[test]
    fn java_writes_a_form_out_and_leaves_what_it_sets_itself() {
        let (prepared, request) = post();
        assert_eq!(
            code(CodeTarget::Java, &prepared, &request, true),
            r#"import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;

public class Main {
    public static void main(String[] args) throws Exception {
        HttpClient client = HttpClient.newBuilder()
            .followRedirects(HttpClient.Redirect.NORMAL)
            .build();
        HttpRequest request = HttpRequest.newBuilder(URI.create("https://api.test/v1/orders?x=1"))
            .method("POST", HttpRequest.BodyPublishers.ofString("{\"name\":\"o'neil\"}"))
            .header("Authorization", "Bearer t-1")
            .header("Content-Type", "application/json")
            .build();
        HttpResponse<String> response = client.send(request, HttpResponse.BodyHandlers.ofString());
        System.out.println(response.statusCode());
        System.out.println(response.body());
    }
}"#
        );
        let (mut prepared, request) = multipart();
        prepared.headers.push(("Host".into(), "api.test".into()));
        prepared.accept_invalid_certs = true;
        let java = code(CodeTarget::Java, &prepared, &request, false);
        for line in [
            "import java.nio.file.Files;",
            "import javax.net.ssl.SSLContext;",
            "        form.add(\"--CyberAPIFormBoundary\\r\\nContent-Disposition: form-data; name=\\\"a\\\"\\r\\n\\r\\n1\\r\\n\".getBytes(StandardCharsets.UTF_8));",
            "        form.add(Files.readAllBytes(Path.of(\"/tmp/x.png\")));",
            "        form.add(\"--CyberAPIFormBoundary--\\r\\n\".getBytes(StandardCharsets.UTF_8));",
            "            .method(\"POST\", HttpRequest.BodyPublishers.ofByteArrays(form))",
            "            // Host: HttpClient sends its own.",
            "            .header(\"Content-Type\", \"multipart/form-data; boundary=CyberAPIFormBoundary\")",
            "            .sslContext(trustAll)",
        ] {
            assert!(java.lines().any(|candidate| candidate == line), "{line}\n{java}");
        }
        assert!(!java.contains("followRedirects"), "HttpClient follows none by default");
    }

    #[test]
    fn csharp_puts_the_bodys_headers_on_its_content() {
        let (prepared, request) = post();
        assert_eq!(
            code(CodeTarget::CSharp, &prepared, &request, false),
            r#"var handler = new HttpClientHandler
{
    AllowAutoRedirect = false,
};
using var client = new HttpClient(handler);
var request = new HttpRequestMessage(new HttpMethod("POST"), "https://api.test/v1/orders?x=1");
request.Headers.TryAddWithoutValidation("Authorization", "Bearer t-1");
request.Content = new ByteArrayContent(System.Text.Encoding.UTF8.GetBytes("{\"name\":\"o'neil\"}"));
request.Content.Headers.TryAddWithoutValidation("Content-Type", "application/json");
var response = await client.SendAsync(request);
Console.WriteLine((int)response.StatusCode);
Console.WriteLine(await response.Content.ReadAsStringAsync());"#
        );
        let (prepared, request) = multipart();
        let csharp = code(CodeTarget::CSharp, &prepared, &request, true);
        assert!(csharp.starts_with("using var client = new HttpClient();\n"), "{csharp}");
        assert!(csharp.contains(
            "form.Add(new StringContent(\"1\"), \"a\");\nform.Add(new StreamContent(File.OpenRead(\"/tmp/x.png\")), \"f\", \"x.png\");\nrequest.Content = form;"
        ));
        // A content header without a body still has content to go on.
        let get = PreparedRequest {
            method: HttpMethod::Get,
            url: "https://api.test/".into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            ..Default::default()
        };
        assert!(
            code(CodeTarget::CSharp, &get, &HttpRequest::default(), true).contains(
                "request.Content = new ByteArrayContent(Array.Empty<byte>());\nrequest.Content.Headers.TryAddWithoutValidation(\"Content-Type\", \"application/json\");"
            )
        );
    }

    #[test]
    fn swift_needs_a_delegate_only_for_what_it_changes() {
        let (prepared, request) = post();
        assert_eq!(
            code(CodeTarget::Swift, &prepared, &request, true),
            r#"import Foundation

var request = URLRequest(url: URL(string: "https://api.test/v1/orders?x=1")!)
request.httpMethod = "POST"
request.setValue("Bearer t-1", forHTTPHeaderField: "Authorization")
request.setValue("application/json", forHTTPHeaderField: "Content-Type")
request.httpBody = Data("{\"name\":\"o'neil\"}".utf8)

let session = URLSession.shared
let (data, response) = try await session.data(for: request)
print((response as! HTTPURLResponse).statusCode)
print(String(decoding: data, as: UTF8.self))"#
        );
        let (prepared, request) = multipart();
        let insecure = PreparedRequest {
            accept_invalid_certs: true,
            ..prepared
        };
        let swift = code(CodeTarget::Swift, &insecure, &request, false);
        for line in [
            "final class Delegate: NSObject, URLSessionTaskDelegate {",
            "        nil",
            "        return (.useCredential, URLCredential(trust: trust))",
            "body.append(try Data(contentsOf: URL(fileURLWithPath: \"/tmp/x.png\")))",
            "request.setValue(\"multipart/form-data; boundary=CyberAPIFormBoundary\", forHTTPHeaderField: \"Content-Type\")",
            "let session = URLSession(configuration: .default, delegate: Delegate(), delegateQueue: nil)",
        ] {
            assert!(swift.lines().any(|candidate| candidate == line), "{line}\n{swift}");
        }
        // No interpolation, and a control character as Swift writes one.
        let odd = PreparedRequest {
            method: HttpMethod::Post,
            url: "https://api.test/".into(),
            body: b"a\\(b)\x07".to_vec(),
            ..Default::default()
        };
        let plain = HttpRequest {
            content_type: ContentType::Plain,
            ..Default::default()
        };
        assert!(code(CodeTarget::Swift, &odd, &plain, true).contains(r#"request.httpBody = Data("a\\(b)\u{7}".utf8)"#));
    }

    #[test]
    fn the_menu_groups_every_target_once_curl_first() {
        let grouped: Vec<CodeTarget> = CodeTarget::GROUPS
            .iter()
            .flat_map(|group| group.iter().copied())
            .collect();
        assert_eq!(grouped, CodeTarget::ALL);
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
