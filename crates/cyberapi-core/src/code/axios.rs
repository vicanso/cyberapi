//! B6: Node's axios. Every status is an answer, as the app takes it, not
//! a thrown error; the body is read as text.

use super::{Field, Parts, file_name, sends_files, string, unanswered_digest};
use crate::send::PreparedRequest;

pub(super) fn axios(prepared: &PreparedRequest, parts: &Parts<'_>, follow_redirects: bool) -> String {
    let mut lines = vec!["import axios from \"axios\";".to_string()];
    if sends_files(parts) {
        lines.push("import { openAsBlob } from \"node:fs\";".into());
    }
    if prepared.accept_invalid_certs {
        lines.push("import https from \"node:https\";".into());
    }
    lines.push(String::new());
    if let Some(login) = &prepared.digest {
        lines.push(unanswered_digest("//", &login.username, "axios"));
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
    lines.push("const response = await axios.request({".into());
    lines.push(format!("    method: {},", string(prepared.method.as_str())));
    lines.push(format!("    url: {},", string(&prepared.url)));
    if !parts.headers.is_empty() {
        lines.push("    headers: {".into());
        for (name, value) in &parts.headers {
            lines.push(format!("        {}: {},", string(name), string(value)));
        }
        lines.push("    },".into());
    }
    if parts.fields.is_some() {
        lines.push("    data: body,".into());
    } else if let Some(body) = &parts.body {
        lines.push(format!("    data: {},", string(body)));
    }
    if !follow_redirects {
        lines.push("    maxRedirects: 0,".into());
    }
    if prepared.accept_invalid_certs {
        lines.push("    httpsAgent: new https.Agent({ rejectUnauthorized: false }),".into());
    }
    lines.push("    responseType: \"text\",".into());
    lines.push("    validateStatus: () => true,".into());
    lines.push("});".into());
    lines.push("console.log(response.status, response.data);".into());
    lines.join("\n")
}
