//! B6: Java's `java.net.http.HttpClient` (Java 11 and later). It has no
//! multipart body of its own, so a form's is written out; the headers it
//! sets itself and refuses (`Host`, `Content-Length`…) are left to it.

use super::{BOUNDARY, Field, Parts, part_head, sends_files, string, unanswered_digest};
use crate::send::PreparedRequest;

/// What `HttpClient` writes itself and will not be given.
const RESTRICTED: [&str; 5] = ["host", "connection", "content-length", "expect", "upgrade"];

pub(super) fn http_client(prepared: &PreparedRequest, parts: &Parts<'_>, follow_redirects: bool) -> String {
    let mut imports = vec![
        "java.net.URI",
        "java.net.http.HttpClient",
        "java.net.http.HttpRequest",
        "java.net.http.HttpResponse",
    ];
    if parts.fields.is_some() {
        imports.extend([
            "java.nio.charset.StandardCharsets",
            "java.util.ArrayList",
            "java.util.List",
        ]);
    }
    if sends_files(parts) {
        imports.extend(["java.nio.file.Files", "java.nio.file.Path"]);
    }
    if prepared.accept_invalid_certs {
        imports.extend([
            "java.security.cert.X509Certificate",
            "javax.net.ssl.SSLContext",
            "javax.net.ssl.TrustManager",
            "javax.net.ssl.X509TrustManager",
        ]);
    }
    imports.sort();
    let mut lines: Vec<String> = imports.iter().map(|import| format!("import {import};")).collect();
    lines.extend(
        [
            "",
            "public class Main {",
            "    public static void main(String[] args) throws Exception {",
        ]
        .map(str::to_string),
    );
    if let Some(login) = &prepared.digest {
        lines.push(unanswered_digest("        //", &login.username, "HttpClient"));
    }
    if prepared.accept_invalid_certs {
        lines.extend(
            [
                "        SSLContext trustAll = SSLContext.getInstance(\"TLS\");",
                "        trustAll.init(null, new TrustManager[] {new X509TrustManager() {",
                "            public void checkClientTrusted(X509Certificate[] chain, String type) {}",
                "            public void checkServerTrusted(X509Certificate[] chain, String type) {}",
                "            public X509Certificate[] getAcceptedIssuers() { return new X509Certificate[0]; }",
                "        }}, null);",
                "        // Host names are still checked: -Djdk.internal.httpclient.disableHostnameVerification=true stops that.",
            ]
            .map(str::to_string),
        );
    }
    lines.push("        HttpClient client = HttpClient.newBuilder()".into());
    if follow_redirects {
        lines.push("            .followRedirects(HttpClient.Redirect.NORMAL)".into());
    }
    if prepared.accept_invalid_certs {
        lines.push("            .sslContext(trustAll)".into());
    }
    lines.push("            .build();".into());
    let body = if let Some(fields) = &parts.fields {
        let bytes = |text: &str| format!("        form.add({}.getBytes(StandardCharsets.UTF_8));", string(text));
        lines.push("        List<byte[]> form = new ArrayList<>();".into());
        for (name, field) in fields {
            match field {
                Field::Text(value) => lines.push(bytes(&format!("{}{value}\r\n", part_head(name, None)))),
                Field::File(path) => {
                    lines.push(bytes(&part_head(name, Some(path))));
                    lines.push(format!(
                        "        form.add(Files.readAllBytes(Path.of({})));",
                        string(path)
                    ));
                    lines.push(bytes("\r\n"));
                }
            }
        }
        lines.push(bytes(&format!("--{BOUNDARY}--\r\n")));
        "HttpRequest.BodyPublishers.ofByteArrays(form)".to_string()
    } else if let Some(body) = &parts.body {
        format!("HttpRequest.BodyPublishers.ofString({})", string(body))
    } else {
        "HttpRequest.BodyPublishers.noBody()".to_string()
    };
    lines.push(format!(
        "        HttpRequest request = HttpRequest.newBuilder(URI.create({}))",
        string(&prepared.url)
    ));
    lines.push(format!(
        "            .method({}, {body})",
        string(prepared.method.as_str())
    ));
    for (name, value) in &parts.headers {
        if RESTRICTED.contains(&name.to_ascii_lowercase().as_str()) {
            lines.push(format!("            // {name}: HttpClient sends its own."));
        } else {
            lines.push(format!("            .header({}, {})", string(name), string(value)));
        }
    }
    if parts.fields.is_some() {
        lines.push(format!(
            "            .header(\"Content-Type\", \"multipart/form-data; boundary={BOUNDARY}\")"
        ));
    }
    lines.extend(
        [
            "            .build();",
            "        HttpResponse<String> response = client.send(request, HttpResponse.BodyHandlers.ofString());",
            "        System.out.println(response.statusCode());",
            "        System.out.println(response.body());",
            "    }",
            "}",
        ]
        .map(str::to_string),
    );
    lines.join("\n")
}
