//! B6: Swift's `URLSession`, with top-level `await`. A form's multipart body
//! is written out, `URLSession` having none of its own; not following
//! redirects, taking any certificate and answering a Digest challenge are
//! a delegate's.

use super::{BOUNDARY, Field, Parts, part_head};
use crate::send::PreparedRequest;

/// A Swift string literal. Every backslash is escaped, so nothing reads
/// as an interpolation.
fn swift_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str(r"\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str(r"\n"),
            '\r' => out.push_str(r"\r"),
            '\t' => out.push_str(r"\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(super) fn url_session(prepared: &PreparedRequest, parts: &Parts<'_>, follow_redirects: bool) -> String {
    let mut lines = vec!["import Foundation".to_string(), String::new()];
    let delegated = !follow_redirects || prepared.accept_invalid_certs || prepared.digest.is_some();
    if delegated {
        lines.push("final class Delegate: NSObject, URLSessionTaskDelegate {".into());
        let mut methods: Vec<Vec<String>> = Vec::new();
        if !follow_redirects {
            methods.push(
                [
                    "    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest) async -> URLRequest? {",
                    "        nil",
                    "    }",
                ]
                .map(str::to_string)
                .to_vec(),
            );
        }
        if prepared.accept_invalid_certs {
            methods.push(
                [
                    "    func urlSession(_ session: URLSession, didReceive challenge: URLAuthenticationChallenge) async -> (URLSession.AuthChallengeDisposition, URLCredential?) {",
                    "        guard let trust = challenge.protectionSpace.serverTrust else { return (.performDefaultHandling, nil) }",
                    "        return (.useCredential, URLCredential(trust: trust))",
                    "    }",
                ]
                .map(str::to_string)
                .to_vec(),
            );
        }
        if let Some(login) = &prepared.digest {
            methods.push(vec![
                "    func urlSession(_ session: URLSession, task: URLSessionTask, didReceive challenge: URLAuthenticationChallenge) async -> (URLSession.AuthChallengeDisposition, URLCredential?) {".to_string(),
                "        guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodHTTPDigest else {".to_string(),
                "            return (.performDefaultHandling, nil)".to_string(),
                "        }".to_string(),
                format!(
                    "        return (.useCredential, URLCredential(user: {}, password: {}, persistence: .forSession))",
                    swift_string(&login.username),
                    swift_string(&login.password)
                ),
                "    }".to_string(),
            ]);
        }
        lines.extend(methods.join(&String::new()));
        lines.push("}".into());
        lines.push(String::new());
    }
    lines.push(format!(
        "var request = URLRequest(url: URL(string: {})!)",
        swift_string(&prepared.url)
    ));
    lines.push(format!(
        "request.httpMethod = {}",
        swift_string(prepared.method.as_str())
    ));
    for (name, value) in &parts.headers {
        lines.push(format!(
            "request.setValue({}, forHTTPHeaderField: {})",
            swift_string(value),
            swift_string(name)
        ));
    }
    if let Some(fields) = &parts.fields {
        let append = |text: &str| format!("body.append(Data({}.utf8))", swift_string(text));
        lines.push("var body = Data()".into());
        for (name, field) in fields {
            match field {
                Field::Text(value) => lines.push(append(&format!("{}{value}\r\n", part_head(name, None)))),
                Field::File(path) => {
                    lines.push(append(&part_head(name, Some(path))));
                    lines.push(format!(
                        "body.append(try Data(contentsOf: URL(fileURLWithPath: {})))",
                        swift_string(path)
                    ));
                    lines.push(append("\r\n"));
                }
            }
        }
        lines.push(append(&format!("--{BOUNDARY}--\r\n")));
        lines.push(format!(
            "request.setValue(\"multipart/form-data; boundary={BOUNDARY}\", forHTTPHeaderField: \"Content-Type\")"
        ));
        lines.push("request.httpBody = body".into());
    } else if let Some(body) = &parts.body {
        lines.push(format!("request.httpBody = Data({}.utf8)", swift_string(body)));
    }
    lines.push(String::new());
    if delegated {
        lines
            .push("let session = URLSession(configuration: .default, delegate: Delegate(), delegateQueue: nil)".into());
    } else {
        lines.push("let session = URLSession.shared".into());
    }
    lines.extend(
        [
            "let (data, response) = try await session.data(for: request)",
            "print((response as! HTTPURLResponse).statusCode)",
            "print(String(decoding: data, as: UTF8.self))",
        ]
        .map(str::to_string),
    );
    lines.join("\n")
}
