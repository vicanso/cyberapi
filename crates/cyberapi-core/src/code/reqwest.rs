//! B6: Rust's reqwest, blocking, so it runs without an async runtime.

use super::{Field, Parts, unanswered_digest};
use crate::send::PreparedRequest;

/// A Rust string literal: raw when it can be, so a body reads as written.
fn rust_string(text: &str) -> String {
    if text.contains('"') && !text.contains("\"#") && !text.contains(['\r', '\0']) {
        format!("r#\"{text}\"#")
    } else {
        format!("{text:?}")
    }
}

pub(super) fn reqwest(prepared: &PreparedRequest, parts: &Parts<'_>, follow_redirects: bool) -> String {
    let features = if parts.fields.is_some() {
        "\"blocking\" and \"multipart\" features"
    } else {
        "\"blocking\" feature"
    };
    let mut lines = vec![
        format!("// reqwest, with its {features}."),
        "use reqwest::blocking::Client;".to_string(),
    ];
    if parts.fields.is_some() {
        lines.push("use reqwest::blocking::multipart::Form;".into());
    }
    if !follow_redirects {
        lines.push("use reqwest::redirect::Policy;".into());
    }
    lines.push(String::new());
    lines.push("fn main() -> Result<(), Box<dyn std::error::Error>> {".into());
    if let Some(login) = &prepared.digest {
        lines.push(unanswered_digest("    //", &login.username, "reqwest"));
    }
    if follow_redirects && !prepared.accept_invalid_certs {
        lines.push("    let client = Client::new();".into());
    } else {
        lines.push("    let client = Client::builder()".into());
        if !follow_redirects {
            lines.push("        .redirect(Policy::none())".into());
        }
        if prepared.accept_invalid_certs {
            lines.push("        .danger_accept_invalid_certs(true)".into());
        }
        lines.push("        .build()?;".into());
    }
    if let Some(fields) = &parts.fields {
        lines.push("    let form = Form::new()".into());
        for (name, field) in fields {
            lines.push(match field {
                Field::Text(value) => format!("        .text({}, {})", rust_string(name), rust_string(value)),
                Field::File(path) => format!("        .file({}, {})?", rust_string(name), rust_string(path)),
            });
        }
        if let Some(last) = lines.last_mut() {
            last.push(';');
        }
    }
    lines.push("    let response = client".into());
    lines.push(format!(
        "        .request(reqwest::Method::{}, {})",
        prepared.method.as_str(),
        rust_string(&prepared.url)
    ));
    for (name, value) in &parts.headers {
        lines.push(format!(
            "        .header({}, {})",
            rust_string(name),
            rust_string(value)
        ));
    }
    if parts.fields.is_some() {
        lines.push("        .multipart(form)".into());
    } else if let Some(body) = &parts.body {
        lines.push(format!("        .body({})", rust_string(body)));
    }
    lines.extend(
        [
            "        .send()?;",
            "    println!(\"{}\", response.status());",
            "    println!(\"{}\", response.text()?);",
            "    Ok(())",
            "}",
        ]
        .map(str::to_string),
    );
    lines.join("\n")
}
