//! B6: PHP's Guzzle. Every status is an answer (`http_errors`), as the app
//! takes it; Guzzle answers a Digest challenge itself.

use super::{Field, Parts, file_name};
use crate::send::PreparedRequest;

/// `'text'`: PHP reads nothing in single quotes but `\\` and `\'`.
fn php_string(text: &str) -> String {
    format!("'{}'", text.replace('\\', r"\\").replace('\'', r"\'"))
}

pub(super) fn guzzle(prepared: &PreparedRequest, parts: &Parts<'_>, follow_redirects: bool) -> String {
    let mut lines = [
        "<?php",
        "require 'vendor/autoload.php';",
        "",
        "use GuzzleHttp\\Client;",
        "",
        "$client = new Client();",
    ]
    .map(str::to_string)
    .to_vec();
    lines.push(format!(
        "$response = $client->request({}, {}, [",
        php_string(prepared.method.as_str()),
        php_string(&prepared.url)
    ));
    if !parts.headers.is_empty() {
        lines.push("    'headers' => [".into());
        for (name, value) in &parts.headers {
            lines.push(format!("        {} => {},", php_string(name), php_string(value)));
        }
        lines.push("    ],".into());
    }
    if let Some(fields) = &parts.fields {
        lines.push("    'multipart' => [".into());
        for (name, field) in fields {
            lines.push(match field {
                Field::Text(value) => format!(
                    "        ['name' => {}, 'contents' => {}],",
                    php_string(name),
                    php_string(value)
                ),
                Field::File(path) => format!(
                    "        ['name' => {}, 'contents' => fopen({}, 'r'), 'filename' => {}],",
                    php_string(name),
                    php_string(path),
                    php_string(file_name(path))
                ),
            });
        }
        lines.push("    ],".into());
    } else if let Some(body) = &parts.body {
        lines.push(format!("    'body' => {},", php_string(body)));
    }
    if let Some(login) = &prepared.digest {
        lines.push(format!(
            "    'auth' => [{}, {}, 'digest'],",
            php_string(&login.username),
            php_string(&login.password)
        ));
    }
    if !follow_redirects {
        lines.push("    'allow_redirects' => false,".into());
    }
    if prepared.accept_invalid_certs {
        lines.push("    'verify' => false,".into());
    }
    lines.push("    'http_errors' => false,".into());
    lines.push("]);".into());
    lines.push("echo $response->getStatusCode(), \"\\n\";".into());
    lines.push("echo $response->getBody(), \"\\n\";".into());
    lines.join("\n")
}
