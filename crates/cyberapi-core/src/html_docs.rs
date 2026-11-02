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
    let prefixes = leading_prefixes(data, &tree);
    let mut nav_html = String::new();
    let mut main_html = String::new();

    render_env_section(data, environments, variables, &prefixes, &mut main_html);
    walk_items(
        data,
        &tree,
        &top,
        &[],
        variables,
        &prefixes,
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

    // The same neutral surfaces, type, and method colors as the app. The
    // renderer does not know the reader's theme, so the page follows the OS.
    // Fonts are the bridge's, two levels up from `/v1/docs/{id}` (and from a
    // `--base-path`); a saved file falls back to the system face.
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — CyberAPI Docs</title>
<style>
@font-face {{
  font-family: "IBM Plex Sans";
  src: url("../../fonts/IBMPlexSans.ttf") format("truetype");
  font-weight: 400;
  font-style: normal;
  font-display: swap;
}}
@font-face {{
  font-family: "JetBrains Mono";
  src: url("../../fonts/JetBrainsMono-Regular.ttf") format("truetype");
  font-weight: 400;
  font-style: normal;
  font-display: swap;
}}
@font-face {{
  font-family: "JetBrains Mono";
  src: url("../../fonts/JetBrainsMono-Bold.ttf") format("truetype");
  font-weight: 700;
  font-style: normal;
  font-display: swap;
}}
:root {{
  color-scheme: light;
  --bg: #ffffff;
  --sidebar: #f5f5f5;
  --panel: #fafafa;
  --fg: #0a0a0a;
  --muted: #737373;
  --line: #e5e5e5;
  --input: #e5e5e5;
  --hover: #e5e5e5;
  --code-bg: #f5f5f5;
  --tabbar: #f5f5f5;
  --tab-active: #ffffff;
  --get: #06b6d4;
  --post: #22c55e;
  --delete: #ef4444;
  --grpc: #9333ea;
  --ws: #0891b2;
  --warn: #eab308;
  --selection: #bfdbfe;
}}
@media (prefers-color-scheme: dark) {{
  :root {{
    color-scheme: dark;
    --bg: #0a0a0a;
    --sidebar: #262626;
    --panel: #171717;
    --fg: #fafafa;
    --muted: #a3a3a3;
    --line: rgba(250, 250, 250, 0.12);
    --input: #2f2f2f;
    --hover: #525252;
    --code-bg: #0a0a0a;
    --tabbar: #0a0a0a;
    --tab-active: #171717;
    --get: #22d3ee;
    --post: #4ade80;
    --delete: #f87171;
    --grpc: #c084fc;
    --ws: #22d3ee;
    --warn: #facc15;
    --selection: #1d4ed8;
  }}
}}
* {{ box-sizing: border-box; }}
html {{ font-size: 14px; }}
body {{
  margin: 0;
  font-family: "IBM Plex Sans", -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Noto Sans SC", sans-serif;
  font-size: 14px;
  background: var(--bg);
  color: var(--fg);
  line-height: 1.5;
  -webkit-font-smoothing: antialiased;
}}
::selection {{ background: var(--selection); }}
.layout {{
  display: flex;
  align-items: flex-start;
  min-height: 100vh;
}}
.sidebar {{
  width: 280px;
  flex-shrink: 0;
  background: var(--sidebar);
  border-right: 1px solid var(--line);
  padding: 12px;
  position: sticky;
  top: 0;
  height: 100vh;
  overflow-y: auto;
}}
.sidebar h1 {{
  font-size: 14px;
  font-weight: 600;
  margin: 2px 4px 10px;
  word-break: break-word;
}}
.search {{
  width: 100%;
  height: 30px;
  padding: 0 10px;
  border: 1px solid var(--input);
  border-radius: 6px;
  background: var(--bg);
  color: var(--fg);
  font: inherit;
  font-size: 13px;
  margin-bottom: 10px;
}}
.search::placeholder {{ color: var(--muted); }}
.search:focus {{
  outline: none;
  border-color: var(--muted);
}}
.nav-folder {{
  font-size: 12px;
  font-weight: 600;
  color: var(--muted);
  margin: 14px 8px 4px;
}}
.nav-item {{
  display: flex;
  align-items: flex-start;
  gap: 6px;
  padding: 4px 8px;
  border-radius: 6px;
  text-decoration: none;
  color: var(--fg);
  font-size: 13px;
  line-height: 1.35;
}}
.nav-item:hover {{ background: var(--hover); color: var(--fg); }}
.nav-name {{
  min-width: 0;
  text-wrap: pretty;
  word-break: break-word;
}}
.content {{
  flex: 1;
  max-width: 1080px;
  padding: 24px 32px 48px;
  min-width: 0;
}}
.project-header {{
  margin-bottom: 20px;
  padding-bottom: 14px;
  border-bottom: 1px solid var(--line);
}}
.project-header h2 {{ margin: 0 0 4px 0; font-size: 20px; font-weight: 600; }}
.project-desc {{ margin: 0; color: var(--muted); font-size: 13px; white-space: pre-wrap; }}
.base-urls {{ margin: 4px 0 8px; }}
.base-urls .section-title {{ margin-top: 0; }}
.base-note {{ margin: 10px 0 4px; color: var(--muted); font-size: 13px; }}
.base-note code {{
  font-family: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
  color: var(--fg);
}}
.folder-heading {{
  margin: 28px 0 12px 0;
  padding-bottom: 6px;
  border-bottom: 1px solid var(--line);
}}
.folder-heading h3 {{ margin: 0; font-size: 16px; font-weight: 600; }}
.folder-desc {{ margin: 4px 0 0 0; color: var(--muted); font-size: 13px; white-space: pre-wrap; }}
.api-card {{
  background: var(--panel);
  border: 1px solid var(--line);
  border-radius: 8px;
  padding: 16px 18px;
  margin-bottom: 16px;
}}
.api-head {{ margin-bottom: 8px; }}
.address {{
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 0 1 auto;
  width: fit-content;
  max-width: 100%;
  min-width: 0;
  min-height: 30px;
  padding: 3px 10px;
  border: 1px solid var(--input);
  border-radius: 6px;
  background: var(--bg);
}}
.method {{
  flex: none;
  width: 4rem;
  font-family: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
  font-weight: 700;
  line-height: 1.35;
}}
.method-get {{ color: var(--get); }}
.method-post {{ color: var(--post); }}
.method-delete {{ color: var(--delete); }}
.method-grpc {{ color: var(--grpc); }}
.method-ws {{ color: var(--ws); }}
.uri {{
  font-family: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 13px;
  min-width: 0;
  word-break: break-all;
}}
.api-title {{
  font-size: 16px;
  font-weight: 600;
  margin: 0 0 8px 0;
}}
.auth {{
  flex: none;
  font-size: 12px;
  line-height: 1.4;
  color: var(--get);
  background: color-mix(in srgb, var(--get) 14%, transparent);
  border-radius: 6px;
  padding: 2px 8px;
}}
.api-desc {{
  color: var(--muted);
  font-size: 13px;
  white-space: pre-wrap;
  margin: 8px 0 12px 0;
}}
.section-title {{
  font-size: 13px;
  font-weight: 600;
  color: var(--fg);
  margin: 16px 0 6px 0;
}}
table {{
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
  margin-bottom: 8px;
}}
th, td {{
  text-align: left;
  padding: 6px 8px;
  border-bottom: 1px solid var(--line);
  vertical-align: top;
}}
th {{ color: var(--muted); font-weight: 600; font-size: 12px; }}
.params td.dim, .schema td:nth-child(2) {{ color: var(--muted); }}
.mono {{ font-family: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; }}
.req {{ color: var(--warn); font-weight: 600; }}
.schema .req {{ margin-left: 6px; }}
.opt {{ color: var(--muted); }}
pre {{
  background: var(--code-bg);
  border: 1px solid var(--line);
  padding: 12px;
  border-radius: 6px;
  overflow-x: auto;
  font-family: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 13px;
  line-height: 1.45;
  margin: 0 0 8px 0;
}}
.tabs {{
  display: flex;
  flex-wrap: wrap;
  width: fit-content;
  max-width: 100%;
  gap: 2px;
  padding: 2px;
  margin: 0 0 8px 0;
  background: var(--tabbar);
  border-radius: 8px;
}}
.tab-btn {{
  border: 0;
  background: transparent;
  color: var(--muted);
  font: inherit;
  font-size: 12px;
  padding: 4px 10px;
  border-radius: 6px;
  cursor: pointer;
}}
.tab-btn:hover {{ color: var(--fg); }}
.tab-btn.active {{
  background: var(--tab-active);
  color: var(--fg);
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

/// Names of a leading `{{variable}}` on any API, in the order the page walks
/// the tree. One name is shown as `BASE_URL`; two names each keep their own.
fn leading_prefixes(data: &TreeData, tree: &Tree) -> Vec<String> {
    let mut names = Vec::new();
    fn walk(data: &TreeData, tree: &Tree, ids: &[crate::ItemId], names: &mut Vec<String>) {
        for id in ids {
            if let Some(api) = data.apis.iter().find(|api| &api.id == id) {
                if let Some(name) = split_env_prefix(&api.request.uri).0
                    && !names.iter().any(|seen| seen == name)
                {
                    names.push(name.to_string());
                }
            } else if data.folders.iter().any(|folder| &folder.id == id) {
                let children = tree.get(id).map(|node| node.children().to_vec()).unwrap_or_default();
                walk(data, tree, &children, names);
            }
        }
    }
    walk(data, tree, &tree.top_level_ids(), &mut names);
    names
}

/// The address a reader sees. The path after a leading prefix is unchanged.
fn display_uri(uri: &str, prefixes: &[String]) -> String {
    let (name, rest) = split_env_prefix(uri);
    match name {
        Some(name) if prefixes.len() == 1 && prefixes[0] == name => format!("BASE_URL{rest}"),
        _ => uri.to_string(),
    }
}

fn render_env_section(
    data: &TreeData,
    environments: &[Environment],
    variables: &[Variable],
    prefixes: &[String],
    out: &mut String,
) {
    if prefixes.is_empty() {
        return;
    }
    let one = prefixes.len() == 1;
    // One prefix with no stored value still reads as BASE_URL in the paths.
    // An empty table under the project description would not explain it.
    if one
        && !variables
            .iter()
            .any(|var| var.category == VariableCategory::Environment && var.enabled && var.name == prefixes[0])
    {
        return;
    }
    let _ = writeln!(
        out,
        "<section class=\"base-urls\"><div class=\"section-title\">Base URL</div>"
    );
    for name in prefixes {
        if !one {
            let _ = writeln!(
                out,
                "<p class=\"base-note\">In paths: <code>{{{{{}}}}}</code></p>",
                escape(name)
            );
        }
        let mut rows: Vec<&Variable> = variables
            .iter()
            .filter(|var| var.category == VariableCategory::Environment && var.enabled && var.name == *name)
            .collect();
        rows.sort_by_key(|var| match &var.environment {
            None => (0, 0usize),
            Some(id) => (
                1,
                environments.iter().position(|env| &env.id == id).unwrap_or(usize::MAX),
            ),
        });
        if rows.is_empty() {
            continue;
        }
        let _ = writeln!(
            out,
            "<table><thead><tr><th>Environment</th><th>Value</th></tr></thead><tbody>"
        );
        for var in rows {
            let env_name = var
                .environment
                .as_ref()
                .and_then(|id| environments.iter().find(|env| &env.id == id))
                .map(|env| env.name.as_str())
                .unwrap_or("Shared");
            let val = if data.settings.secret_variables.contains(&var.id) {
                "***".to_string()
            } else {
                masked(&var.name, &var.value)
            };
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td class=\"mono\">{}</td></tr>",
                escape(env_name),
                escape(&val)
            );
        }
        let _ = writeln!(out, "</tbody></table>");
    }
    let _ = writeln!(out, "</section>");
}

#[allow(clippy::too_many_arguments)]
fn walk_items(
    data: &TreeData,
    tree: &Tree,
    ids: &[crate::ItemId],
    folder_path: &[String],
    variables: &[Variable],
    prefixes: &[String],
    nav: &mut String,
    main: &mut String,
) {
    for id in ids {
        if let Some(folder) = data.folders.iter().find(|f| &f.id == id) {
            let mut next_path = folder_path.to_vec();
            next_path.push(folder.name.clone());
            render_folder(folder, &next_path, nav, main);
            let children = tree.get(id).map(|node| node.children().to_vec()).unwrap_or_default();
            walk_items(data, tree, &children, &next_path, variables, prefixes, nav, main);
        } else if let Some(api) = data.apis.iter().find(|a| &a.id == id) {
            render_api(data, api, folder_path, variables, prefixes, nav, main);
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

#[allow(clippy::too_many_arguments)]
fn render_api(
    data: &TreeData,
    api: &ApiSetting,
    folder_path: &[String],
    variables: &[Variable],
    prefixes: &[String],
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
        "<a class=\"nav-item\" href=\"#{anchor}\" data-search=\"{}\"><span class=\"method {badge_cls}\">{method_str}</span><span class=\"nav-name\">{}</span></a>",
        escape(&search_key),
        escape(&api.name)
    );

    let _ = writeln!(
        main,
        "<article id=\"{anchor}\" class=\"api-card\" data-search=\"{}\">",
        escape(&search_key)
    );
    let inh = inherited(data, &api.id);
    let auth_badge = auth_label(inh.auth(&req.auth))
        .map(|label| format!("<span class=\"auth\">Auth: {}</span>", escape(&label)))
        .unwrap_or_default();
    let _ = writeln!(
        main,
        "<div class=\"api-head\"><h4 class=\"api-title\">{}</h4><div class=\"address\"><span class=\"method {badge_cls}\">{method_str}</span><code class=\"uri\">{}</code>{auth_badge}</div></div>",
        escape(&api.name),
        escape(&display_uri(&req.uri, prefixes))
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
    if req.method == HttpMethod::Grpc {
        let grpc = crate::grpc::GrpcBody::read(&req.body);
        let call = format!("{}/{}", grpc.service.trim(), grpc.method.trim());
        let has_call = !grpc.service.trim().is_empty() || !grpc.method.trim().is_empty();
        let has_msg = !grpc.message.trim().is_empty();
        if has_call || has_msg {
            let _ = writeln!(main, "<div class=\"section-title\">gRPC Call</div>");
            if has_call {
                let _ = writeln!(main, "<p class=\"mono\">{}</p>", escape(&call));
            }
            if has_msg {
                let _ = writeln!(main, "<pre><code>{}</code></pre>", escape(&masked_body(&grpc.message)));
            }
        }
    } else if req.content_type == ContentType::GraphQl && !req.body.trim().is_empty() {
        let gql = crate::graphql::GraphQlBody::read(&req.body);
        if !gql.query.trim().is_empty() {
            let _ = writeln!(main, "<div class=\"section-title\">GraphQL Query</div>");
            let _ = writeln!(main, "<pre><code>{}</code></pre>", escape(gql.query.trim()));
        }
        if !gql.variables.trim().is_empty() {
            let _ = writeln!(main, "<div class=\"section-title\">GraphQL Variables</div>");
            let _ = writeln!(main, "<pre><code>{}</code></pre>", escape(&masked_body(&gql.variables)));
        }
    } else if req.method.has_body()
        && !matches!(req.content_type, ContentType::Form | ContentType::Multipart)
        && !req.body.trim().is_empty()
    {
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
    render_code_snippets(data, api, variables, prefixes, main);

    let _ = writeln!(main, "</article>");
}

fn auth_label(auth: &crate::auth::Auth) -> Option<String> {
    use crate::auth::{Auth, KeyPlace};
    match auth {
        Auth::Inherit | Auth::None => None,
        Auth::Basic { .. } => Some("Basic".into()),
        Auth::Digest { .. } => Some("Digest".into()),
        Auth::Bearer { .. } => Some("Bearer".into()),
        Auth::ApiKey { name, place, .. } => {
            let trimmed = name.trim();
            let where_str = match place {
                KeyPlace::Header => "Header",
                KeyPlace::Query => "Query",
            };
            if trimmed.is_empty() {
                Some(format!("API Key ({where_str})"))
            } else {
                Some(format!("API Key ({where_str}: {trimmed})"))
            }
        }
        Auth::OAuth2(_) => Some("OAuth 2.0".into()),
    }
}

fn masked_auth_signed(auth: &crate::auth::Auth, query: bool) -> Option<crate::auth::Signed> {
    use crate::auth::{Auth, KeyPlace, Signed};
    match auth {
        Auth::Inherit | Auth::None => None,
        Auth::Basic { username, password } if !username.trim().is_empty() || !password.trim().is_empty() => {
            Some(Signed::Header {
                name: "Authorization".into(),
                value: format!("Basic <{}:{}>", username.trim(), masked("password", password.trim())),
            })
        }
        Auth::Digest { username, password } if !username.trim().is_empty() || !password.trim().is_empty() => {
            Some(Signed::Header {
                name: "Authorization".into(),
                value: format!("Digest <{}:{}>", username.trim(), masked("password", password.trim())),
            })
        }
        Auth::Bearer { token } if !token.trim().is_empty() => Some(Signed::Header {
            name: "Authorization".into(),
            value: format!("Bearer {}", masked("token", token.trim())),
        }),
        Auth::ApiKey { name, value, place } if !name.trim().is_empty() => {
            let name = name.trim().to_string();
            let value = masked("api_key", value.trim());
            if *place == KeyPlace::Query && query {
                Some(Signed::Query { name, value })
            } else {
                Some(Signed::Header { name, value })
            }
        }
        Auth::OAuth2(oauth) if !oauth.token_url.trim().is_empty() || !oauth.client_id.trim().is_empty() => {
            Some(Signed::Header {
                name: "Authorization".into(),
                value: format!("Bearer {}", crate::mcp::MASK),
            })
        }
        _ => None,
    }
}

struct DocParam {
    name: String,
    place: &'static str,
    value: String,
    description: String,
    schema_type: String,
    required: Option<bool>,
    format: String,
    enums: String,
}

impl DocParam {
    fn plain(name: String, place: &'static str, value: String, description: String) -> Self {
        Self {
            name,
            place,
            value,
            description,
            schema_type: String::new(),
            required: None,
            format: String::new(),
            enums: String::new(),
        }
    }

    /// Path, query and header rows show the four facts. Form and file rows
    /// do not, even when a row happens to carry them.
    fn from_kv(name: String, place: &'static str, value: String, kv: &crate::model::KvParam, facts: bool) -> Self {
        Self {
            name,
            place,
            value,
            description: kv.description.clone(),
            schema_type: if facts { kv.schema_type.clone() } else { String::new() },
            required: if facts { kv.required } else { None },
            format: if facts { kv.format.clone() } else { String::new() },
            enums: if facts { kv.enum_text() } else { String::new() },
        }
    }
}

fn required_cell(required: Option<bool>) -> &'static str {
    match required {
        Some(true) => "<span class=\"req\">required</span>",
        Some(false) => "<span class=\"opt\">optional</span>",
        None => "",
    }
}

fn render_params_table(data: &TreeData, api: &ApiSetting, out: &mut String) {
    let req = &api.request;
    let inh = inherited(data, &api.id);
    let auth = inh.auth(&req.auth);
    let auth_signed = masked_auth_signed(auth, req.method != HttpMethod::Grpc);
    let mut rows: Vec<DocParam> = Vec::new();

    for name in path_param_names(&req.uri) {
        let found = req.variables.iter().find(|kv| kv.key.trim() == name);
        let value = found.map(|kv| masked(&name, &kv.value)).unwrap_or_default();
        rows.push(match found {
            Some(kv) => DocParam::from_kv(name, "path", value, kv, true),
            None => DocParam::plain(name, "path", value, String::new()),
        });
    }
    for kv in req.query.iter().filter(|kv| kv.enabled && !kv.key.trim().is_empty()) {
        rows.push(DocParam::from_kv(
            kv.key.trim().to_string(),
            "query",
            masked(&kv.key, &kv.value),
            kv,
            true,
        ));
    }
    if let Some(crate::auth::Signed::Query { name, value }) = &auth_signed
        && !req.query.iter().any(|kv| kv.enabled && kv.key.trim() == name)
    {
        rows.push(DocParam::plain(name.clone(), "query", value.clone(), String::new()));
    }
    for header in &inh.headers {
        if header.enabled && !header.key.trim().is_empty() {
            rows.push(DocParam::from_kv(
                header.key.trim().to_string(),
                "header",
                masked(&header.key, &header.value),
                header,
                true,
            ));
        }
    }
    for kv in req.headers.iter().filter(|kv| kv.enabled && !kv.key.trim().is_empty()) {
        rows.push(DocParam::from_kv(
            kv.key.trim().to_string(),
            "header",
            masked(&kv.key, &kv.value),
            kv,
            true,
        ));
    }
    if let Some(crate::auth::Signed::Header { name, value }) = &auth_signed
        && !rows
            .iter()
            .any(|row| row.place == "header" && row.name.eq_ignore_ascii_case(name))
    {
        rows.push(DocParam::plain(name.clone(), "header", value.clone(), String::new()));
    }
    if req.method.has_body() && matches!(req.content_type, ContentType::Form | ContentType::Multipart) {
        for kv in form_fields(&req.body)
            .into_iter()
            .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
        {
            let place = if req.content_type == ContentType::Multipart && crate::send::is_file_value(&kv.value) {
                "file"
            } else {
                "form"
            };
            let shown = crate::send::page_file(&kv.value)
                .map(|(_, name)| name)
                .unwrap_or(kv.value.as_str());
            rows.push(DocParam::from_kv(
                kv.key.trim().to_string(),
                place,
                masked(&kv.key, shown),
                &kv,
                false,
            ));
        }
    }

    if rows.is_empty() {
        return;
    }
    // A column that is empty on every row is left out, so a name and a
    // description do not sit across a run of blank type cells.
    let show_type = rows.iter().any(|row| !row.schema_type.trim().is_empty());
    let show_required = rows.iter().any(|row| row.required.is_some());
    let show_format = rows.iter().any(|row| !row.format.trim().is_empty());
    let show_enum = rows.iter().any(|row| !row.enums.trim().is_empty());
    let show_value = rows.iter().any(|row| !row.value.trim().is_empty());
    let show_desc = rows.iter().any(|row| !row.description.trim().is_empty());
    let _ = writeln!(out, "<div class=\"section-title\">Parameters</div>");
    let _ = write!(out, "<table class=\"params\"><thead><tr><th>Name</th><th>In</th>");
    if show_type {
        let _ = write!(out, "<th>Type</th>");
    }
    if show_required {
        let _ = write!(out, "<th>Required</th>");
    }
    if show_format {
        let _ = write!(out, "<th>Format</th>");
    }
    if show_enum {
        let _ = write!(out, "<th>Enum</th>");
    }
    if show_value {
        let _ = write!(out, "<th>Value</th>");
    }
    if show_desc {
        let _ = write!(out, "<th>Description</th>");
    }
    let _ = writeln!(out, "</tr></thead><tbody>");
    for row in &rows {
        let _ = write!(
            out,
            "<tr><td class=\"mono\">{}</td><td class=\"mono dim\">{}</td>",
            escape(&row.name),
            row.place
        );
        if show_type {
            let _ = write!(out, "<td class=\"mono dim\">{}</td>", escape(&row.schema_type));
        }
        if show_required {
            let _ = write!(out, "<td>{}</td>", required_cell(row.required));
        }
        if show_format {
            let _ = write!(out, "<td class=\"mono dim\">{}</td>", escape(&row.format));
        }
        if show_enum {
            let _ = write!(out, "<td class=\"dim\">{}</td>", escape(&row.enums));
        }
        if show_value {
            let _ = write!(out, "<td class=\"mono\">{}</td>", escape(&row.value));
        }
        if show_desc {
            let _ = write!(out, "<td>{}</td>", escape(row.description.trim()));
        }
        let _ = writeln!(out, "</tr>");
    }
    let _ = writeln!(out, "</tbody></table>");
}

fn render_schema_table(title: &str, fields: &[SchemaField], out: &mut String) {
    let _ = writeln!(out, "<div class=\"section-title\">{}</div>", escape(title));
    let _ = writeln!(
        out,
        "<table class=\"schema\"><thead><tr><th>Field</th><th>Type</th><th>Description</th></tr></thead><tbody>"
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
    variables: &[Variable],
    prefixes: &[String],
    out: &mut String,
) {
    let req = &api.request;
    if req.method == HttpMethod::WebSocket {
        return;
    }
    let (prepared, masked_req) = sample_prepared(data, api, variables, prefixes);
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

fn sample_prepared(
    data: &TreeData,
    api: &ApiSetting,
    variables: &[Variable],
    prefixes: &[String],
) -> (PreparedRequest, HttpRequest) {
    let req = &api.request;
    // The same address the page shows. One leading variable is `BASE_URL`;
    // two keep `{{name}}`. Path parameters are still filled in.
    let mut url = replace_path_params(&display_uri(&req.uri, prefixes), |name| {
        req.variables
            .iter()
            .find(|kv| kv.enabled && kv.key.trim() == name)
            .map(|kv| masked(name, &kv.value))
            .filter(|v| !v.is_empty())
    });
    if req.method == HttpMethod::Grpc
        && !url.is_empty()
        && !url.contains("://")
        && !url.starts_with("BASE_URL")
        && !url.starts_with("{{")
    {
        url = format!("http://{url}");
    }

    let inh = inherited(data, &api.id);
    let auth = inh.auth(&req.auth);
    let auth_signed = masked_auth_signed(auth, req.method != HttpMethod::Grpc);

    let mut query_pairs: Vec<String> = req
        .query
        .iter()
        .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
        .map(|kv| {
            format!(
                "{}={}",
                crate::send::encode_uri_component(kv.key.trim()),
                crate::send::encode_uri_component(&masked(&kv.key, &kv.value))
            )
        })
        .collect();
    if let Some(crate::auth::Signed::Query { name, value }) = &auth_signed
        && !req.query.iter().any(|kv| kv.enabled && kv.key.trim() == name)
    {
        query_pairs.push(format!(
            "{}={}",
            crate::send::encode_uri_component(name),
            crate::send::encode_uri_component(value)
        ));
    }
    if req.method != HttpMethod::Grpc && !query_pairs.is_empty() {
        let sep = if url.contains('?') { '&' } else { '?' };
        url.push(sep);
        url.push_str(&query_pairs.join("&"));
    }

    let mut headers: Vec<(String, String)> = Vec::new();
    for v in variables {
        if v.category == VariableCategory::GlobalRequestHeader && v.enabled && !v.name.trim().is_empty() {
            let val = if data.settings.secret_variables.contains(&v.id) {
                crate::mcp::MASK.to_string()
            } else {
                masked(&v.name, &v.value)
            };
            headers.push((v.name.trim().to_string(), val));
        }
    }
    for h in &inh.headers {
        if h.enabled && !h.key.trim().is_empty() {
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case(h.key.trim()));
            headers.push((h.key.trim().to_string(), masked(&h.key, &h.value)));
        }
    }
    for h in req.headers.iter().filter(|h| h.enabled && !h.key.trim().is_empty()) {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case(h.key.trim()));
        headers.push((h.key.trim().to_string(), masked(&h.key, &h.value)));
    }
    if let Some(crate::auth::Signed::Header { name, value }) = auth_signed
        && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(&name))
    {
        headers.push((name, value));
    }

    let (prepared_body, masked_req_body, auto_content_type) = if req.method == HttpMethod::Grpc {
        let grpc = crate::grpc::GrpcBody::read(&req.body);
        let masked_grpc = crate::grpc::GrpcBody {
            service: grpc.service,
            method: grpc.method,
            message: masked_body(&grpc.message),
            ..Default::default()
        };
        (Vec::new(), masked_grpc.to_body(), None)
    } else if !req.method.has_body() || crate::send::body_is_blank(req) {
        (Vec::new(), String::new(), None)
    } else {
        match req.content_type {
            ContentType::Form => {
                let masked_fields: Vec<KvParam> = form_fields(&req.body)
                    .into_iter()
                    .map(|kv| KvParam {
                        value: masked(&kv.key, &kv.value),
                        ..kv
                    })
                    .collect();
                let encoded = masked_fields
                    .iter()
                    .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
                    .map(|kv| {
                        format!(
                            "{}={}",
                            crate::send::encode_uri_component(kv.key.trim()),
                            crate::send::encode_uri_component(&kv.value)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("&");
                let req_body = serde_json::to_string(&masked_fields).unwrap_or_default();
                (encoded.into_bytes(), req_body, Some(req.content_type.as_str()))
            }
            ContentType::Multipart => {
                let masked_fields: Vec<KvParam> = form_fields(&req.body)
                    .into_iter()
                    .map(|kv| KvParam {
                        value: masked(&kv.key, &kv.value),
                        ..kv
                    })
                    .collect();
                let req_body = serde_json::to_string(&masked_fields).unwrap_or_default();
                (Vec::new(), req_body, None)
            }
            ContentType::GraphQl => {
                let gql = crate::graphql::GraphQlBody::read(&req.body);
                let masked_gql = crate::graphql::GraphQlBody {
                    query: gql.query,
                    variables: masked_body(&gql.variables),
                    operation_name: gql.operation_name,
                };
                let payload = masked_gql
                    .payload()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|_| masked_gql.to_body());
                (payload.into_bytes(), masked_gql.to_body(), Some("application/json"))
            }
            _ => {
                let masked_str = masked_body(&req.body);
                (
                    masked_str.clone().into_bytes(),
                    masked_str,
                    Some(req.content_type.as_str()),
                )
            }
        }
    };

    if let Some(ct) = auto_content_type
        && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        headers.push(("Content-Type".into(), ct.into()));
    }

    let masked_req = HttpRequest {
        body: masked_req_body,
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
        body: prepared_body,
        ..Default::default()
    };
    (prepared, masked_req)
}

/// The tree's method colors: DELETE red, a body method green, gRPC magenta,
/// WebSocket cyan, and the rest the info cyan.
fn method_badge_class(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Delete => "method-delete",
        HttpMethod::Grpc => "method-grpc",
        HttpMethod::WebSocket => "method-ws",
        m if m.has_body() => "method-post",
        _ => "method-get",
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
                        ..Default::default()
                    }],
                    headers: vec![KvParam {
                        key: "Authorization".into(),
                        value: "Bearer super-secret-jwt".into(),
                        description: "Auth header".into(),
                        enabled: true,
                        ..Default::default()
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
        assert!(html.contains("class=\"method method-get\""));
        assert!(html.contains("text-wrap: pretty"));
        assert!(html.contains("../../fonts/IBMPlexSans.ttf"));
        assert!(html.contains("../../fonts/JetBrainsMono-Bold.ttf"));
        assert!(!html.contains("badge-get"));
        assert!(html.contains("order_id"));
        assert!(html.contains("The order identifier"));
        assert!(html.contains("Order ID"));
        let title_at = html.find("api-title\">Get Order").expect("title");
        let uri_at = html.find("BASE_URL/orders/:order_id").expect("address");
        assert!(title_at < uri_at, "the name is the heading and the address follows it");
        assert!(html.contains("BASE_URL/orders/ord_123"));
        assert!(html.contains(">Base URL<"));
        assert!(html.contains(">Shared<"));
        assert!(html.contains("https://api.payments.test"));
        assert!(!html.contains("{{HOST}}") && !html.contains(">HOST<"), "{html}");
        assert!(!html.contains("https://api.payments.test/orders"));
        assert!(!html.contains("super-secret-jwt"), "auth header value is masked");
        assert!(!html.contains("live_key_999"), "secret variable value is masked");
        assert!(
            !html.contains("MERCHANT_KEY"),
            "a variable that is not a path prefix is not listed"
        );
    }

    #[test]
    fn renders_form_grpc_graphql_and_auth_without_leaking_secrets_or_internal_json() {
        let collection = Collection {
            id: "01JPROJECT".into(),
            name: "Multi".into(),
            ..Default::default()
        };
        let form_body = serde_json::to_string(&vec![
            KvParam {
                key: "username".into(),
                value: "alice".into(),
                enabled: true,
                description: "User handle".into(),
                ..Default::default()
            },
            KvParam {
                key: "password".into(),
                value: "s3cr3t!".into(),
                enabled: true,
                description: "Secret password".into(),
                ..Default::default()
            },
        ])
        .expect("form json");
        let grpc_body = crate::grpc::GrpcBody {
            service: "shop.Store".into(),
            method: "Buy".into(),
            message: r#"{"sku":"A-1","apiKey":"secret-grpc-key"}"#.into(),
            descriptors: "AAAA_BASE64_DESCRIPTOR_BLOB".into(),
            ..Default::default()
        }
        .to_body();
        let gql_body = crate::graphql::GraphQlBody {
            query: "query GetUser($id: ID!) { user(id: $id) { name } }".into(),
            variables: r#"{"id":"42","token":"gql-secret-token"}"#.into(),
            operation_name: "GetUser".into(),
        }
        .to_body();
        let data = TreeData {
            folders: vec![],
            apis: vec![
                ApiSetting {
                    id: "01JFORM".into(),
                    name: "Login Form".into(),
                    request: HttpRequest {
                        method: HttpMethod::Post,
                        uri: "https://api.example.com/login".into(),
                        content_type: ContentType::Form,
                        query: vec![KvParam {
                            key: "redirect uri".into(),
                            value: "a&b=c".into(),
                            enabled: true,
                            ..Default::default()
                        }],
                        body: form_body,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ApiSetting {
                    id: "01JGRPC".into(),
                    name: "Buy Item".into(),
                    request: HttpRequest {
                        method: HttpMethod::Grpc,
                        uri: "grpc.example.com:50051".into(),
                        body: grpc_body,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ApiSetting {
                    id: "01JGQL".into(),
                    name: "User Query".into(),
                    request: HttpRequest {
                        method: HttpMethod::Post,
                        uri: "https://api.example.com/graphql".into(),
                        content_type: ContentType::GraphQl,
                        body: gql_body,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ],
            top_order: vec!["01JFORM".into(), "01JGRPC".into(), "01JGQL".into()],
            settings: ProjectSettings {
                auth: crate::auth::Auth::Bearer {
                    token: "proj-bearer-secret".into(),
                },
                ..Default::default()
            },
        };
        let html = render(&collection, &data, &[], &[]);
        assert!(html.contains("username=alice&amp;password=***"));
        assert!(!html.contains("s3cr3t!"));
        assert!(html.contains("redirect%20uri=a%26b%3Dc"));
        assert!(html.contains("shop.Store/Buy"));
        assert!(!html.contains("AAAA_BASE64_DESCRIPTOR_BLOB"));
        assert!(!html.contains("secret-grpc-key"));
        assert!(html.contains("GraphQL Query"));
        assert!(html.contains("GraphQL Variables"));
        assert!(!html.contains("gql-secret-token"));
        assert!(!html.contains("proj-bearer-secret"));
        assert!(html.contains("Bearer ***"));
    }

    #[test]
    fn parameter_tables_show_type_required_format_and_enum() {
        let collection = Collection {
            id: "01JPROJECT".into(),
            name: "Shop".into(),
            ..Default::default()
        };
        let data = TreeData {
            folders: vec![],
            apis: vec![ApiSetting {
                id: "01JAPI".into(),
                name: "List".into(),
                request: HttpRequest {
                    method: HttpMethod::Post,
                    uri: "/pets/{id}".into(),
                    variables: vec![
                        KvParam {
                            key: "id".into(),
                            enabled: true,
                            ..Default::default()
                        }
                        .with_param_schema("string", Some(true), "uuid", Vec::<String>::new()),
                    ],
                    query: vec![
                        KvParam {
                            key: "tags".into(),
                            value: "a".into(),
                            enabled: true,
                            ..Default::default()
                        }
                        .with_param_schema(
                            "array<string>",
                            Some(false),
                            "date-time",
                            ["open", "shut"],
                        ),
                        KvParam {
                            key: "note".into(),
                            value: "x".into(),
                            enabled: true,
                            ..Default::default()
                        },
                    ],
                    content_type: ContentType::Form,
                    body: serde_json::to_string(&[KvParam {
                        key: "file".into(),
                        value: "a".into(),
                        enabled: true,
                        ..Default::default()
                    }
                    .with_param_schema("string", Some(true), "binary", ["no"])])
                    .expect("form"),
                    ..Default::default()
                },
                ..Default::default()
            }],
            top_order: vec!["01JAPI".into()],
            ..Default::default()
        };
        let html = render(&collection, &data, &[], &[]);
        assert!(html.contains("<th>Type</th><th>Required</th><th>Format</th><th>Enum</th>"));
        assert!(html.contains("array&lt;string&gt;"));
        assert!(html.contains(">optional<"));
        assert!(html.contains("date-time"));
        assert!(html.contains("open | shut"));
        assert!(html.contains(">required<"));
        assert!(html.contains("uuid"));
        assert!(html.contains(">file<"));
        assert!(!html.contains("binary"), "a form row does not show parameter schema");
    }

    #[test]
    fn parameter_table_omits_a_column_that_is_empty() {
        let collection = Collection {
            id: "01JPROJECT".into(),
            name: "Shop".into(),
            ..Default::default()
        };
        let data = TreeData {
            folders: vec![],
            apis: vec![ApiSetting {
                id: "01JAPI".into(),
                name: "Token".into(),
                request: HttpRequest {
                    method: HttpMethod::Get,
                    uri: "/session".into(),
                    query: vec![KvParam {
                        key: "token".into(),
                        description: "Setup token".into(),
                        enabled: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ..Default::default()
            }],
            top_order: vec!["01JAPI".into()],
            ..Default::default()
        };
        let html = render(&collection, &data, &[], &[]);
        assert!(html.contains("<th>Name</th><th>In</th><th>Description</th>"));
        assert!(!html.contains("<th>Type</th>"));
        assert!(!html.contains("<th>Required</th>"));
        assert!(!html.contains("<th>Format</th>"));
        assert!(!html.contains("<th>Enum</th>"));
        assert!(!html.contains("<th>Value</th>"));
        assert!(html.contains("Setup token"));
    }

    #[test]
    fn two_leading_variables_keep_their_names() {
        let collection = Collection {
            id: "01JPROJECT".into(),
            name: "Shop".into(),
            ..Default::default()
        };
        let data = TreeData {
            folders: vec![],
            apis: vec![
                ApiSetting {
                    id: "01JAPI1".into(),
                    name: "One".into(),
                    request: HttpRequest {
                        method: HttpMethod::Get,
                        uri: "{{HOST}}/a".into(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ApiSetting {
                    id: "01JAPI2".into(),
                    name: "Two".into(),
                    request: HttpRequest {
                        method: HttpMethod::Get,
                        uri: "{{OTHER}}/b".into(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ],
            top_order: vec!["01JAPI1".into(), "01JAPI2".into()],
            ..Default::default()
        };
        let variables = vec![
            Variable {
                id: "01JHOST".into(),
                collection: "01JPROJECT".into(),
                name: "HOST".into(),
                value: "https://one.test".into(),
                category: VariableCategory::Environment,
                enabled: true,
                environment: None,
                created_at: Default::default(),
                updated_at: Default::default(),
            },
            Variable {
                id: "01JOTHER".into(),
                collection: "01JPROJECT".into(),
                name: "OTHER".into(),
                value: "https://two.test".into(),
                category: VariableCategory::Environment,
                enabled: true,
                environment: None,
                created_at: Default::default(),
                updated_at: Default::default(),
            },
        ];
        let html = render(&collection, &data, &[], &variables);
        assert!(html.contains("{{HOST}}/a"));
        assert!(html.contains("{{OTHER}}/b"));
        assert!(html.contains("In paths: <code>{{HOST}}</code>"));
        assert!(html.contains("In paths: <code>{{OTHER}}</code>"));
        assert!(html.contains("https://one.test"));
        assert!(html.contains("https://two.test"));
        assert!(!html.contains("BASE_URL"));
    }
}
