//! B6: C#'s `HttpClient` (.NET 6 and later, top-level statements). The
//! headers about the body go on its content, as .NET has them; the
//! handler answers a Digest challenge itself.

use super::{Field, Parts, file_name, string};
use crate::send::PreparedRequest;

/// What .NET keeps with the content, not the request.
fn about_content(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("content-") || matches!(name.as_str(), "allow" | "expires" | "last-modified")
}

pub(super) fn http_client(prepared: &PreparedRequest, parts: &Parts<'_>, follow_redirects: bool) -> String {
    let mut lines = Vec::new();
    if prepared.digest.is_some() {
        lines.extend(["using System.Net;", ""].map(str::to_string));
    }
    let mut options = Vec::new();
    if !follow_redirects {
        options.push("    AllowAutoRedirect = false,".to_string());
    }
    if prepared.accept_invalid_certs {
        options.push(
            "    ServerCertificateCustomValidationCallback = HttpClientHandler.DangerousAcceptAnyServerCertificateValidator,"
                .to_string(),
        );
    }
    if let Some(login) = &prepared.digest {
        options.push(format!(
            "    Credentials = new NetworkCredential({}, {}),",
            string(&login.username),
            string(&login.password)
        ));
    }
    if options.is_empty() {
        lines.push("using var client = new HttpClient();".into());
    } else {
        lines.push("var handler = new HttpClientHandler".into());
        lines.push("{".into());
        lines.extend(options);
        lines.push("};".into());
        lines.push("using var client = new HttpClient(handler);".into());
    }
    lines.push(format!(
        "var request = new HttpRequestMessage(new HttpMethod({}), {});",
        string(prepared.method.as_str()),
        string(&prepared.url)
    ));
    let (content, request): (Vec<_>, Vec<_>) = parts.headers.iter().partition(|(name, _)| about_content(name));
    for (name, value) in request {
        lines.push(format!(
            "request.Headers.TryAddWithoutValidation({}, {});",
            string(name),
            string(value)
        ));
    }
    if let Some(fields) = &parts.fields {
        lines.push("var form = new MultipartFormDataContent();".into());
        for (name, field) in fields {
            lines.push(match field {
                Field::Text(value) => format!("form.Add(new StringContent({}), {});", string(value), string(name)),
                Field::File(path) => format!(
                    "form.Add(new StreamContent(File.OpenRead({})), {}, {});",
                    string(path),
                    string(name),
                    string(file_name(path))
                ),
            });
        }
        lines.push("request.Content = form;".into());
    } else if let Some(body) = &parts.body {
        lines.push(format!(
            "request.Content = new ByteArrayContent(System.Text.Encoding.UTF8.GetBytes({}));",
            string(body)
        ));
    } else if !content.is_empty() {
        lines.push("request.Content = new ByteArrayContent(Array.Empty<byte>());".into());
    }
    for (name, value) in content {
        lines.push(format!(
            "request.Content.Headers.TryAddWithoutValidation({}, {});",
            string(name),
            string(value)
        ));
    }
    lines.extend(
        [
            "var response = await client.SendAsync(request);",
            "Console.WriteLine((int)response.StatusCode);",
            "Console.WriteLine(await response.Content.ReadAsStringAsync());",
        ]
        .map(str::to_string),
    );
    lines.join("\n")
}
