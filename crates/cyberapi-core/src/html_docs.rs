//! G3: a project rendered as a self-contained, read-only HTML documentation
//! page. Pure data in and HTML out (L0), so `cyberapi-bridge` serves it at
//! `GET /v1/docs/{project}` and unit tests verify it without a server.

use crate::assertions::Subject;
use crate::code::{CodeTarget, code};
use crate::inherit::inherited;
use crate::mcp::{masked, masked_body};
use crate::model::{
    ApiSetting, Collection, ContentType, Environment, Folder, HttpMethod, HttpRequest, KvParam, Variable,
    VariableCategory, path_param_names, replace_path_params, split_env_prefix,
};
use crate::schema_docs::{SchemaField, fields_of_schema};
use crate::send::{PreparedRequest, form_fields};
use crate::tree::{Tree, TreeData};
use std::fmt::Write as _;

const CODE_TARGETS: [CodeTarget; 5] = [
    CodeTarget::Curl,
    CodeTarget::Fetch,
    CodeTarget::Python,
    CodeTarget::Go,
    CodeTarget::Axios,
];

/// Renders `collection` and its `data`, `environments` and `variables` into a
/// standalone HTML5 document with secrets masked.
pub fn render(
    collection: &Collection,
    data: &TreeData,
    environments: &[Environment],
    variables: &[Variable],
) -> String {
    let tree = Tree::build(data);
    let top = tree.top_level_ids();
    let mut nav_html = String::new();
    let mut main_html = String::new();

    render_env_section(data, environments, variables, &mut main_html);
    walk_items(
        data,
        &tree,
        &top,
        &[],
        environments,
        variables,
        &mut nav_html,
        &mut main_html,
    );

    let title = escape(&collection.name);
    let desc = collection.description.trim();
    let desc_html = if desc.is_empty() {
        String::new()
    } else {
        format!("<p class=\"project-desc\">{}</p>", escape(desc))
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — CyberAPI Docs</title>
<style>
:root {{
  --bg: #f8fafc;
  --panel: #ffffff;
  --fg: #0f172a;
  --muted: #64748b;
  --border: #e2e8f0;
  --code-bg: #f1f5f9;
  --accent: #2563eb;
  --get: #0284c7;
  --post: #16a34a;
  --delete: #dc2626;
  --grpc: #9333ea;
  --ws: #0891b2;
  --warn: #d97706;
}}
@media (prefers-color-scheme: dark) {{
  :root {{
    --bg: #0b0f19;
    --panel: #111827;
    --fg: #f1f5f9;
    --muted: #94a3b8;
    --border: #1e293b;
    --code-bg: #0f172a;
    --accent: #3b82f6;
  }}
}}
* {{ box-sizing: border-box; }}
body {{
  margin: 0;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  background: var(--bg);
  color: var(--fg);
  line-height: 1.5;
}}
.layout {{
  display: flex;
  min-height: 100vh;
}}
.sidebar {{
  width: 280px;
  flex-shrink: 0;
  background: var(--panel);
  border-right: 1px solid var(--border);
  padding: 16px;
  position: sticky;
  top: 0;
  height: 100vh;
  overflow-y: auto;
}}
.sidebar h1 {{
  font-size: 16px;
  margin: 0 0 8px 0;
  word-break: break-word;
}}
.search {{
  width: 100%;
  padding: 6px 10px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--bg);
  color: var(--fg);
  font-size: 13px;
  margin-bottom: 12px;
}}
.nav-folder {{
  font-size: 12px;
  font-weight: 600;
  color: var(--muted);
  margin: 12px 0 4px 0;
  text-transform: uppercase;
  letter-spacing: 0.03em;
}}
.nav-item {{
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  border-radius: 5px;
  text-decoration: none;
  color: var(--fg);
  font-size: 13px;
}}
.nav-item:hover {{ background: var(--code-bg); }}
.content {{
  flex: 1;
  max-width: 980px;
  padding: 28px 36px;
  min-width: 0;
}}
.project-header {{
  margin-bottom: 24px;
  padding-bottom: 16px;
  border-bottom: 1px solid var(--border);
}}
.project-header h2 {{ margin: 0 0 6px 0; font-size: 24px; }}
.project-desc {{ margin: 0; color: var(--muted); white-space: pre-wrap; }}
.folder-heading {{
  margin: 32px 0 12px 0;
  padding-bottom: 6px;
  border-bottom: 1px solid var(--border);
}}
.folder-heading h3 {{ margin: 0; font-size: 18px; }}
.folder-desc {{ margin: 4px 0 0 0; color: var(--muted); font-size: 13px; white-space: pre-wrap; }}
.api-card {{
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 20px;
  margin-bottom: 20px;
}}
.api-head {{
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  margin-bottom: 8px;
}}
.badge {{
  display: inline-block;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  font-weight: 700;
  padding: 2px 7px;
  border-radius: 4px;
  color: #fff;
}}
.badge-get {{ background: var(--get); }}
.badge-post {{ background: var(--post); }}
.badge-delete {{ background: var(--delete); }}
.badge-grpc {{ background: var(--grpc); }}
.badge-ws {{ background: var(--ws); }}
.uri {{
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 13px;
  background: var(--code-bg);
  padding: 3px 8px;
  border-radius: 4px;
  word-break: break-all;
}}
.api-title {{
  font-size: 16px;
  font-weight: 600;
  margin: 0;
}}
.api-desc {{
  color: var(--muted);
  font-size: 13px;
  white-space: pre-wrap;
  margin: 8px 0 14px 0;
}}
.section-title {{
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--muted);
  margin: 16px 0 8px 0;
}}
table {{
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
  margin-bottom: 12px;
}}
th, td {{
  text-align: left;
  padding: 6px 10px;
  border-bottom: 1px solid var(--border);
  vertical-align: top;
}}
th {{ color: var(--muted); font-weight: 600; font-size: 12px; }}
.mono {{ font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; }}
.req {{ color: var(--warn); font-size: 11px; margin-left: 4px; }}
pre {{
  background: var(--code-bg);
  padding: 12px;
  border-radius: 6px;
  overflow-x: auto;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
  margin: 6px 0 12px 0;
}}
.tabs {{
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
  margin-bottom: 6px;
}}
.tab-btn {{
  border: 1px solid var(--border);
  background: var(--bg);
  color: var(--fg);
  font-size: 12px;
  padding: 3px 9px;
  border-radius: 4px;
  cursor: pointer;
}}
.tab-btn.active {{
  background: var(--accent);
  color: #fff;
  border-color: var(--accent);
}}
.hidden {{ display: none !important; }}
</style>
</head>
<body>
<div class="layout">
  <aside class="sidebar">
    <h1>{title}</h1>
    <input id="doc-search" class="search" type="search" placeholder="Filter APIs…">
    <nav id="doc-nav">{nav_html}</nav>
  </aside>
  <main class="content">
    <header class="project-header">
      <h2>{title}</h2>
      {desc_html}
    </header>
    {main_html}
  </main>
</div>
<script>
document.getElementById('doc-search')?.addEventListener('input', function(e) {{
  var q = (e.target.value || '').toLowerCase().trim();
  document.querySelectorAll('[data-search]').forEach(function(el) {{
    var match = !q || el.getAttribute('data-search').indexOf(q) !== -1;
    el.classList.toggle('hidden', !match);
  }});
}});
document.querySelectorAll('[data-tab-group]').forEach(function(btn) {{
  btn.addEventListener('click', function() {{
    var g = btn.getAttribute('data-tab-group');
    var t = btn.getAttribute('data-tab-target');
    document.querySelectorAll('[data-tab-group="' + g + '"]').forEach(function(b) {{
      b.classList.toggle('active', b === btn);
    }});
    document.querySelectorAll('[data-tab-pane="' + g + '"]').forEach(function(p) {{
      p.classList.toggle('hidden', p.id !== t);
    }});
  }});
}});
</script>
</body>
</html>"#
    )
}

fn render_env_section(data: &TreeData, environments: &[Environment], variables: &[Variable], out: &mut String) {
    let env_vars: Vec<&Variable> = variables
        .iter()
        .filter(|v| v.category == VariableCategory::Environment && v.enabled && !v.name.trim().is_empty())
        .collect();
    if env_vars.is_empty() && environments.is_empty() {
        return;
    }
    let _ = writeln!(
        out,
        "<section class=\"api-card\"><div class=\"section-title\">Environments &amp; Variables</div>"
    );
    let _ = writeln!(
        out,
        "<table><thead><tr><th>Name</th><th>Environment</th><th>Value</th></tr></thead><tbody>"
    );
    for var in env_vars {
        let env_name = var
            .environment
            .as_ref()
            .and_then(|id| environments.iter().find(|e| &e.id == id))
            .map(|e| e.name.as_str())
            .unwrap_or("Shared");
        let val = if data.settings.secret_variables.contains(&var.id) {
            "***".to_string()
        } else {
            masked(&var.name, &var.value)
        };
        let _ = writeln!(
            out,
            "<tr><td class=\"mono\">{}</td><td>{}</td><td class=\"mono\">{}</td></tr>",
            escape(&var.name),
            escape(env_name),
            escape(&val)
        );
    }
    let _ = writeln!(out, "</tbody></table></section>");
}

#[allow(clippy::too_many_arguments)]
fn walk_items(
    data: &TreeData,
    tree: &Tree,
    ids: &[crate::ItemId],
    folder_path: &[String],
    environments: &[Environment],
    variables: &[Variable],
    nav: &mut String,
    main: &mut String,
) {
    for id in ids {
        if let Some(folder) = data.folders.iter().find(|f| &f.id == id) {
            let mut next_path = folder_path.to_vec();
            next_path.push(folder.name.clone());
            render_folder(folder, &next_path, nav, main);
            let children = tree.get(id).map(|node| node.children().to_vec()).unwrap_or_default();
            walk_items(data, tree, &children, &next_path, environments, variables, nav, main);
        } else if let Some(api) = data.apis.iter().find(|a| &a.id == id) {
            render_api(data, api, folder_path, environments, variables, nav, main);
        }
    }
}

fn render_folder(folder: &Folder, path: &[String], nav: &mut String, main: &mut String) {
    let joined = path.join(" / ");
    let _ = writeln!(nav, "<div class=\"nav-folder\">{}</div>", escape(&joined));
    let desc = folder.settings.description.trim();
    let desc_html = if desc.is_empty() {
        String::new()
    } else {
        format!("<p class=\"folder-desc\">{}</p>", escape(desc))
    };
    let _ = writeln!(
        main,
        "<div class=\"folder-heading\"><h3>{}</h3>{desc_html}</div>",
        escape(&joined)
    );
}

fn render_api(
    data: &TreeData,
    api: &ApiSetting,
    folder_path: &[String],
    environments: &[Environment],
    variables: &[Variable],
    nav: &mut String,
    main: &mut String,
) {
    let req = &api.request;
    let badge_cls = method_badge_class(req.method);
    let method_str = req.method.as_str();
    let search_key = format!("{} {} {} {}", folder_path.join("/"), api.name, method_str, req.uri).to_lowercase();
    let anchor = format!("api-{}", api.id);

    let _ = writeln!(
        nav,
        "<a class=\"nav-item\" href=\"#{anchor}\" data-search=\"{}\"><span class=\"badge {badge_cls}\">{method_str}</span><span>{}</span></a>",
        escape(&search_key),
        escape(&api.name)
    );

    let _ = writeln!(
        main,
        "<article id=\"{anchor}\" class=\"api-card\" data-search=\"{}\">",
        escape(&search_key)
    );
    let _ = writeln!(
        main,
        "<div class=\"api-head\"><span class=\"badge {badge_cls}\">{method_str}</span><code class=\"uri\">{}</code><h4 class=\"api-title\">{}</h4></div>",
        escape(&req.uri),
        escape(&api.name)
    );
    if !req.description.trim().is_empty() {
        let _ = writeln!(main, "<div class=\"api-desc\">{}</div>", escape(req.description.trim()));
    }

    // Input parameters: path, query, headers (inherited + own), form/multipart fields.
    render_params_table(data, api, main);

    // F11: request body JSON Schema fields.
    if let Some(schema_text) = req.request_schema.as_deref().map(str::trim).filter(|s| !s.is_empty())
        && let Ok(fields) = fields_of_schema(schema_text)
        && !fields.is_empty()
    {
        render_schema_table("Request Body Schema", &fields, main);
    }

    // Request body sample/raw if present.
    if req.method.has_body() && !req.body.trim().is_empty() {
        let _ = writeln!(main, "<div class=\"section-title\">Request Body</div>");
        let _ = writeln!(main, "<pre><code>{}</code></pre>", escape(&masked_body(&req.body)));
    }

    // F11: response body JSON Schema assertion fields.
    if let Some(assertion) = req
        .assertions
        .iter()
        .find(|a| a.subject == Subject::Schema && !a.expected.trim().is_empty())
        && let Ok(fields) = fields_of_schema(&assertion.expected)
        && !fields.is_empty()
    {
        render_schema_table("Response Body Schema", &fields, main);
    }

    // F3: saved response examples.
    if !req.examples.is_empty() {
        let _ = writeln!(main, "<div class=\"section-title\">Response Examples</div>");
        for example in &req.examples {
            let body_pretty = serde_json::from_str::<serde_json::Value>(&example.body)
                .map(|v| crate::format::json(&v))
                .unwrap_or_else(|_| example.body.clone());
            let _ = writeln!(
                main,
                "<div><span class=\"mono\"><strong>{}</strong> — HTTP {}</span><pre><code>{}</code></pre></div>",
                escape(&example.name),
                example.status,
                escape(&masked_body(&body_pretty))
            );
        }
    }

    // B6: multi-language code snippets.
    render_code_snippets(data, api, environments, variables, main);

    let _ = writeln!(main, "</article>");
}

fn render_params_table(data: &TreeData, api: &ApiSetting, out: &mut String) {
    let req = &api.request;
    let inh = inherited(data, &api.id);
    let mut rows: Vec<(String, &'static str, String, String)> = Vec::new();

    for name in path_param_names(&req.uri) {
        let found = req.variables.iter().find(|kv| kv.key.trim() == name);
        let value = found.map(|kv| masked(&name, &kv.value)).unwrap_or_default();
        let desc = found.map(|kv| kv.description.clone()).unwrap_or_default();
        rows.push((name, "path", value, desc));
    }
    for kv in req.query.iter().filter(|kv| kv.enabled && !kv.key.trim().is_empty()) {
        rows.push((
            kv.key.trim().to_string(),
            "query",
            masked(&kv.key, &kv.value),
            kv.description.clone(),
        ));
    }
    for header in &inh.headers {
        if header.enabled && !header.key.trim().is_empty() {
            rows.push((
                header.key.trim().to_string(),
                "header",
                masked(&header.key, &header.value),
                header.description.clone(),
            ));
        }
    }
    for kv in req.headers.iter().filter(|kv| kv.enabled && !kv.key.trim().is_empty()) {
        rows.push((
            kv.key.trim().to_string(),
            "header",
            masked(&kv.key, &kv.value),
            kv.description.clone(),
        ));
    }
    if req.method.has_body() && matches!(req.content_type, ContentType::Form | ContentType::Multipart) {
        for kv in form_fields(&req.body)
            .into_iter()
            .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
        {
            let place = if req.content_type == ContentType::Multipart && kv.value.starts_with("file://") {
                "file"
            } else {
                "form"
            };
            rows.push((
                kv.key.trim().to_string(),
                place,
                masked(&kv.key, &kv.value),
                kv.description.clone(),
            ));
        }
    }

    if rows.is_empty() {
        return;
    }
    let _ = writeln!(out, "<div class=\"section-title\">Parameters</div>");
    let _ = writeln!(
        out,
        "<table><thead><tr><th>Name</th><th>In</th><th>Value</th><th>Description</th></tr></thead><tbody>"
    );
    for (name, place, val, desc) in rows {
        let _ = writeln!(
            out,
            "<tr><td class=\"mono\">{}</td><td class=\"mono\">{}</td><td class=\"mono\">{}</td><td>{}</td></tr>",
            escape(&name),
            place,
            escape(&val),
            escape(desc.trim())
        );
    }
    let _ = writeln!(out, "</tbody></table>");
}

fn render_schema_table(title: &str, fields: &[SchemaField], out: &mut String) {
    let _ = writeln!(out, "<div class=\"section-title\">{}</div>", escape(title));
    let _ = writeln!(
        out,
        "<table><thead><tr><th>Field</th><th>Type</th><th>Description</th></tr></thead><tbody>"
    );
    for field in fields {
        let indent = "&nbsp;&nbsp;".repeat(field.depth.min(8));
        let req_badge = if field.required {
            "<span class=\"req\">required</span>"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "<tr><td class=\"mono\">{indent}{}{req_badge}</td><td class=\"mono\">{}</td><td>{}</td></tr>",
            escape(&field.name),
            escape(&field.kind),
            escape(field.description.trim())
        );
    }
    let _ = writeln!(out, "</tbody></table>");
}

fn render_code_snippets(
    data: &TreeData,
    api: &ApiSetting,
    _environments: &[Environment],
    variables: &[Variable],
    out: &mut String,
) {
    let req = &api.request;
    if req.method == HttpMethod::WebSocket {
        return;
    }
    let (prepared, masked_req) = sample_prepared(data, api, variables);
    let targets: &[CodeTarget] = if req.method == HttpMethod::Grpc {
        &[CodeTarget::Curl]
    } else {
        &CODE_TARGETS
    };
    let group = format!("code-{}", api.id);
    let _ = writeln!(
        out,
        "<div class=\"section-title\">Code Examples</div><div class=\"tabs\">"
    );
    for (idx, target) in targets.iter().enumerate() {
        let active = if idx == 0 { " active" } else { "" };
        let pane_id = format!("{group}-{idx}");
        let label = if req.method == HttpMethod::Grpc {
            "grpcurl"
        } else {
            target.label()
        };
        let _ = writeln!(
            out,
            "<button type=\"button\" class=\"tab-btn{active}\" data-tab-group=\"{group}\" data-tab-target=\"{pane_id}\">{}</button>",
            escape(label)
        );
    }
    let _ = writeln!(out, "</div>");
    for (idx, target) in targets.iter().enumerate() {
        let hidden = if idx == 0 { "" } else { " class=\"hidden\"" };
        let pane_id = format!("{group}-{idx}");
        let snippet = code(*target, &prepared, &masked_req, true);
        let _ = writeln!(
            out,
            "<pre id=\"{pane_id}\" data-tab-pane=\"{group}\"{hidden}><code>{}</code></pre>",
            escape(&snippet)
        );
    }
}

fn sample_prepared(data: &TreeData, api: &ApiSetting, variables: &[Variable]) -> (PreparedRequest, HttpRequest) {
    let req = &api.request;
    let (env_name, rest) = split_env_prefix(&req.uri);
    let base = env_name
        .and_then(|name| {
            variables
                .iter()
                .find(|v| {
                    v.category == VariableCategory::Environment
                        && v.enabled
                        && v.name == name
                        && !data.settings.secret_variables.contains(&v.id)
                })
                .map(|v| v.value.trim_end_matches('/').to_string())
        })
        .unwrap_or_else(|| {
            if env_name.is_some() {
                "https://example.com".to_string()
            } else {
                String::new()
            }
        });
    let raw_url = if base.is_empty() {
        rest.to_string()
    } else if rest.starts_with('/') || rest.is_empty() {
        format!("{base}{rest}")
    } else {
        format!("{base}/{rest}")
    };
    let mut url = replace_path_params(&raw_url, |name| {
        req.variables
            .iter()
            .find(|kv| kv.enabled && kv.key.trim() == name)
            .map(|kv| masked(name, &kv.value))
            .filter(|v| !v.is_empty())
    });
    let query_pairs: Vec<String> = req
        .query
        .iter()
        .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
        .map(|kv| format!("{}={}", kv.key.trim(), masked(&kv.key, &kv.value)))
        .collect();
    if !query_pairs.is_empty() {
        let sep = if url.contains('?') { '&' } else { '?' };
        url.push(sep);
        url.push_str(&query_pairs.join("&"));
    }

    let inh = inherited(data, &api.id);
    let mut headers: Vec<(String, String)> = Vec::new();
    for h in &inh.headers {
        if h.enabled && !h.key.trim().is_empty() {
            headers.push((h.key.trim().to_string(), masked(&h.key, &h.value)));
        }
    }
    for h in req.headers.iter().filter(|h| h.enabled && !h.key.trim().is_empty()) {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case(h.key.trim()));
        headers.push((h.key.trim().to_string(), masked(&h.key, &h.value)));
    }
    if req.method.has_body()
        && !req.body.trim().is_empty()
        && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        headers.push(("Content-Type".into(), req.content_type.as_str().into()));
    }

    let masked_body_str = masked_body(&req.body);
    let masked_req = HttpRequest {
        body: masked_body_str.clone(),
        headers: req
            .headers
            .iter()
            .map(|kv| KvParam {
                value: masked(&kv.key, &kv.value),
                ..kv.clone()
            })
            .collect(),
        ..req.clone()
    };
    let prepared = PreparedRequest {
        method: req.method,
        url,
        headers,
        body: if req.method.has_body() {
            masked_body_str.into_bytes()
        } else {
            Vec::new()
        },
        ..Default::default()
    };
    (prepared, masked_req)
}

fn method_badge_class(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Delete => "badge-delete",
        HttpMethod::Grpc => "badge-grpc",
        HttpMethod::WebSocket => "badge-ws",
        m if m.has_body() => "badge-post",
        _ => "badge-get",
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assertions::Assertion;
    use crate::model::{Example, ProjectSettings};

    #[test]
    fn renders_project_tree_schemas_examples_and_masks_secrets() {
        let collection = Collection {
            id: "01JPROJECT".into(),
            name: "Payments <API>".into(),
            description: "Order & checkout service".into(),
            ..Default::default()
        };
        let secret_var_id: crate::VariableId = "01JSECRET".into();
        let data = TreeData {
            folders: vec![],
            apis: vec![ApiSetting {
                id: "01JAPI1".into(),
                name: "Get Order".into(),
                request: HttpRequest {
                    method: HttpMethod::Get,
                    uri: "{{HOST}}/orders/:order_id".into(),
                    description: "Fetches a single order.".into(),
                    variables: vec![KvParam {
                        key: "order_id".into(),
                        value: "ord_123".into(),
                        description: "The order identifier".into(),
                        enabled: true,
                    }],
                    headers: vec![KvParam {
                        key: "Authorization".into(),
                        value: "Bearer super-secret-jwt".into(),
                        description: "Auth header".into(),
                        enabled: true,
                    }],
                    assertions: vec![Assertion {
                        subject: Subject::Schema,
                        expected: r#"{"type":"object","required":["id"],"properties":{"id":{"type":"string","description":"Order ID"}}}"#.into(),
                        ..Default::default()
                    }],
                    examples: vec![Example {
                        name: "Found".into(),
                        status: 200,
                        headers: vec![],
                        body: r#"{"id":"ord_123"}"#.into(),
                    }],
                    ..Default::default()
                },
                ..Default::default()
            }],
            top_order: vec!["01JAPI1".into()],
            settings: ProjectSettings {
                secret_variables: vec![secret_var_id.clone()],
                ..Default::default()
            },
        };
        let variables = vec![
            Variable {
                id: "01JHOST".into(),
                collection: "01JPROJECT".into(),
                name: "HOST".into(),
                value: "https://api.payments.test".into(),
                category: VariableCategory::Environment,
                enabled: true,
                environment: None,
                created_at: Default::default(),
                updated_at: Default::default(),
            },
            Variable {
                id: secret_var_id,
                collection: "01JPROJECT".into(),
                name: "MERCHANT_KEY".into(),
                value: "live_key_999".into(),
                category: VariableCategory::Environment,
                enabled: true,
                environment: None,
                created_at: Default::default(),
                updated_at: Default::default(),
            },
        ];

        let html = render(&collection, &data, &[], &variables);
        assert!(html.contains("Payments &lt;API&gt;"));
        assert!(html.contains("Get Order"));
        assert!(html.contains("order_id"));
        assert!(html.contains("The order identifier"));
        assert!(html.contains("Order ID"));
        assert!(html.contains("https://api.payments.test/orders/ord_123"));
        assert!(!html.contains("super-secret-jwt"), "auth header value is masked");
        assert!(!html.contains("live_key_999"), "secret variable value is masked");
    }
}
