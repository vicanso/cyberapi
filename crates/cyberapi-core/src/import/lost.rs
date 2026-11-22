//! M1: items that an import could not represent directly (scripts, unrecognized
//! template expressions, unsupported auth kinds and unsupported request bodies),
//! grouped by target path so the import report tells the reader what did not come along.

use super::{ImportBundle, ImportItem};
use crate::graphql::GraphQlBody;
use crate::model::ContentType;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Which execution phase an unimported script belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptPhase {
    PreRequest,
    PostResponse,
    Test,
    Other,
}

/// Something in an imported document that could not be imported as-is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LostKind {
    Script { phase: ScriptPhase, source: String },
    Template { expr: String },
    Auth { kind: String },
    Body { kind: String },
}

/// One unimported piece on a collection root, folder or API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lost {
    /// `"Folder / Api"`, `"Folder"`, or `""` for the collection/root level.
    pub target: String,
    pub kind: LostKind,
}

/// Entries from [`Lost`] grouped under a single target path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LostGroup {
    pub target: String,
    pub items: Vec<LostKind>,
}

/// Groups `lost` entries by `target`, preserving first-seen order and placing
/// the project/collection root (`target == ""`) first when present.
pub fn group(lost: &[Lost]) -> Vec<LostGroup> {
    let mut groups: Vec<LostGroup> = Vec::new();
    for entry in lost {
        if let Some(existing) = groups.iter_mut().find(|group| group.target == entry.target) {
            existing.items.push(entry.kind.clone());
        } else {
            groups.push(LostGroup {
                target: entry.target.clone(),
                items: vec![entry.kind.clone()],
            });
        }
    }
    if let Some(root_idx) = groups.iter().position(|group| group.target.is_empty())
        && root_idx > 0
    {
        let root = groups.remove(root_idx);
        groups.insert(0, root);
    }
    groups
}

/// Joins folder/item path segments with `" / "`.
pub(super) fn join_path(prefix: &str, name: &str) -> String {
    let name = name.trim();
    if prefix.is_empty() {
        name.to_string()
    } else if name.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix} / {name}")
    }
}

/// Records an unimported script when `source` contains non-whitespace text,
/// stripping leading blank lines and trailing whitespace while keeping indentation.
pub(super) fn push_script(lost: &mut Vec<Lost>, target: &str, phase: ScriptPhase, source: &str) {
    let trimmed = source.trim_end();
    if trimmed.trim().is_empty() {
        return;
    }
    let lines: Vec<&str> = trimmed.lines().collect();
    let first = lines.iter().position(|line| !line.trim().is_empty()).unwrap_or(0);
    let normalized = lines[first..].join("\n").trim_end().to_string();
    if !normalized.is_empty() {
        lost.push(Lost {
            target: target.to_string(),
            kind: LostKind::Script {
                phase,
                source: normalized,
            },
        });
    }
}

/// Records an unsupported authentication kind if `kind` is non-empty.
pub(super) fn push_auth(lost: &mut Vec<Lost>, target: &str, kind: &str) {
    let kind = kind.trim();
    if !kind.is_empty() {
        lost.push(Lost {
            target: target.to_string(),
            kind: LostKind::Auth { kind: kind.to_string() },
        });
    }
}

/// Records an unsupported request body kind if `kind` is non-empty.
pub(super) fn push_body(lost: &mut Vec<Lost>, target: &str, kind: &str) {
    let kind = kind.trim();
    if !kind.is_empty() {
        lost.push(Lost {
            target: target.to_string(),
            kind: LostKind::Body { kind: kind.to_string() },
        });
    }
}

/// Scans `bundle` for unrecognized `{{...}}` template expressions (unknown
/// `$...` built-in variables and unknown `fn(...)` functions), deduplicated per target.
pub(super) fn scan_templates(bundle: &mut ImportBundle) {
    scan_bundle(bundle, scan_template_text);
}

/// Scans `bundle` for Insomnia Nunjucks `{% ... %}` tags, deduplicated per target.
pub(super) fn scan_insomnia_tags(bundle: &mut ImportBundle) {
    scan_bundle(bundle, scan_insomnia_text);
}

type TextScanner = fn(&str, &str, &mut HashSet<String>, &mut Vec<Lost>);

fn existing_templates_for(lost: &[Lost], target: &str) -> HashSet<String> {
    lost.iter()
        .filter(|entry| entry.target == target)
        .filter_map(|entry| match &entry.kind {
            LostKind::Template { expr } => Some(expr.clone()),
            _ => None,
        })
        .collect()
}

fn scan_bundle(bundle: &mut ImportBundle, scanner: TextScanner) {
    let mut added = Vec::new();
    let mut root_seen = existing_templates_for(&bundle.lost, "");
    for text in bundle.auth.templates() {
        scanner("", text, &mut root_seen, &mut added);
    }
    for (_, value) in &bundle.environment {
        scanner("", value, &mut root_seen, &mut added);
    }
    for env in &bundle.environments {
        for (_, value) in &env.variables {
            scanner("", value, &mut root_seen, &mut added);
        }
    }
    walk_items(&bundle.items, "", &bundle.lost, scanner, &mut added);
    bundle.lost.extend(added);
}

fn walk_items(items: &[ImportItem], prefix: &str, existing_lost: &[Lost], scanner: TextScanner, added: &mut Vec<Lost>) {
    for item in items {
        match item {
            ImportItem::Folder {
                name,
                children,
                settings,
            } => {
                let target = join_path(prefix, name);
                let mut seen = existing_templates_for(existing_lost, &target);
                for text in settings.auth.templates() {
                    scanner(&target, text, &mut seen, added);
                }
                for kv in settings.headers.iter().chain(&settings.variables) {
                    scanner(&target, &kv.key, &mut seen, added);
                    scanner(&target, &kv.value, &mut seen, added);
                }
                walk_items(children, &target, existing_lost, scanner, added);
            }
            ImportItem::Api { name, request } => {
                let target = join_path(prefix, name);
                let mut seen = existing_templates_for(existing_lost, &target);
                scanner(&target, &request.uri, &mut seen, added);
                for kv in request.query.iter().chain(&request.headers) {
                    scanner(&target, &kv.key, &mut seen, added);
                    scanner(&target, &kv.value, &mut seen, added);
                }
                for kv in &request.variables {
                    scanner(&target, &kv.value, &mut seen, added);
                }
                for text in request.auth.templates() {
                    scanner(&target, text, &mut seen, added);
                }
                if request.content_type == ContentType::GraphQl {
                    let gql = GraphQlBody::read(&request.body);
                    scanner(&target, &gql.query, &mut seen, added);
                    scanner(&target, &gql.variables, &mut seen, added);
                } else if request.content_type.is_key_value() {
                    for kv in crate::send::form_fields(&request.body) {
                        scanner(&target, &kv.key, &mut seen, added);
                        scanner(&target, &kv.value, &mut seen, added);
                    }
                } else if request.content_type == ContentType::Binary {
                    if let Some(path) = request.body.trim().strip_prefix("file://") {
                        scanner(&target, path, &mut seen, added);
                    }
                } else {
                    scanner(&target, &request.body, &mut seen, added);
                }
            }
        }
    }
}

fn scan_template_text(target: &str, text: &str, seen: &mut HashSet<String>, out: &mut Vec<Lost>) {
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.get(1..).and_then(|tail| tail.find("}}")).map(|at| at + 1) else {
            break;
        };
        let inner = &after[..end];
        let whole = &rest[start..start + 2 + end + 2];
        if !inner.contains('\n') {
            let trimmed = inner.trim();
            if trimmed.starts_with('$') {
                if !crate::template::is_known_dollar_var(trimmed) && seen.insert(whole.to_string()) {
                    out.push(Lost {
                        target: target.to_string(),
                        kind: LostKind::Template {
                            expr: whole.to_string(),
                        },
                    });
                }
            } else if let Some(expr) = crate::template::parse_inner(whole, inner)
                && expr
                    .chain
                    .iter()
                    .all(|n| !n.is_empty() && !n.contains(char::is_whitespace))
                && expr.chain.iter().any(|n| !crate::template::is_known_function(n))
                && seen.insert(whole.to_string())
            {
                out.push(Lost {
                    target: target.to_string(),
                    kind: LostKind::Template {
                        expr: whole.to_string(),
                    },
                });
            }
        }
        rest = &after[end + 2..];
    }
}

fn scan_insomnia_text(target: &str, text: &str, seen: &mut HashSet<String>, out: &mut Vec<Lost>) {
    let mut rest = text;
    while let Some(start) = rest.find("{%") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("%}") else {
            break;
        };
        let inner = &after[..end];
        let whole = &rest[start..start + 2 + end + 2];
        if !inner.contains('\n') && !inner.trim().is_empty() && seen.insert(whole.to_string()) {
            out.push(Lost {
                target: target.to_string(),
                kind: LostKind::Template {
                    expr: whole.to_string(),
                },
            });
        }
        rest = &after[end + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Auth;
    use crate::import::{ImportEnvironment, kv};
    use crate::model::{FolderSettings, HttpMethod, HttpRequest};
    use pretty_assertions::assert_eq;

    #[test]
    fn group_orders_root_first_and_preserves_item_order() {
        let lost = vec![
            Lost {
                target: "Orders / Create".into(),
                kind: LostKind::Auth { kind: "hawk".into() },
            },
            Lost {
                target: String::new(),
                kind: LostKind::Script {
                    phase: ScriptPhase::PreRequest,
                    source: "console.log(1)".into(),
                },
            },
            Lost {
                target: "Orders / Create".into(),
                kind: LostKind::Template {
                    expr: "{{$randomEmail}}".into(),
                },
            },
            Lost {
                target: "Users / List".into(),
                kind: LostKind::Body { kind: "file".into() },
            },
        ];
        assert_eq!(
            group(&lost),
            vec![
                LostGroup {
                    target: String::new(),
                    items: vec![LostKind::Script {
                        phase: ScriptPhase::PreRequest,
                        source: "console.log(1)".into(),
                    }],
                },
                LostGroup {
                    target: "Orders / Create".into(),
                    items: vec![
                        LostKind::Auth { kind: "hawk".into() },
                        LostKind::Template {
                            expr: "{{$randomEmail}}".into(),
                        },
                    ],
                },
                LostGroup {
                    target: "Users / List".into(),
                    items: vec![LostKind::Body { kind: "file".into() }],
                },
            ]
        );
    }

    #[test]
    fn scan_templates_flags_only_unknown_expressions_and_deduplicates_per_target() {
        let mut bundle = ImportBundle {
            auth: Auth::Bearer {
                token: "{{$unknownBearer}}".into(),
            },
            environment: vec![("user".into(), "{{$unknownEnvVar}}".into())],
            environments: vec![ImportEnvironment {
                name: "dev".into(),
                variables: vec![("ok".into(), "{{uuid()}}".into())],
            }],
            items: vec![ImportItem::Folder {
                name: "Orders".into(),
                settings: FolderSettings {
                    headers: vec![kv("X-F", "{{customFn(a)}}", true)],
                    ..Default::default()
                },
                children: vec![ImportItem::Api {
                    name: "Create".into(),
                    request: Box::new(HttpRequest {
                        method: HttpMethod::Post,
                        uri: "{{host}}/orders/{{$guid}}/{{user.id}}".into(),
                        query: vec![
                            kv("email", "{{$unknownQuery}}", true),
                            kv("ok_faker", "{{$randomEmail}}", true),
                        ],
                        headers: vec![kv("X-Email", "{{$unknownQuery}}", true)],
                        content_type: ContentType::Json,
                        body:
                            r#"{"at":"{{$timestamp}}","fn":"{{b64.uuid()}}","name":"{{cnName()}}","bad":"{{nope(1)}}"}"#
                                .into(),
                        ..Default::default()
                    }),
                }],
            }],
            ..Default::default()
        };
        scan_templates(&mut bundle);
        assert_eq!(
            bundle.lost,
            vec![
                Lost {
                    target: String::new(),
                    kind: LostKind::Template {
                        expr: "{{$unknownBearer}}".into(),
                    },
                },
                Lost {
                    target: String::new(),
                    kind: LostKind::Template {
                        expr: "{{$unknownEnvVar}}".into(),
                    },
                },
                Lost {
                    target: "Orders".into(),
                    kind: LostKind::Template {
                        expr: "{{customFn(a)}}".into(),
                    },
                },
                Lost {
                    target: "Orders / Create".into(),
                    kind: LostKind::Template {
                        expr: "{{$unknownQuery}}".into(),
                    },
                },
                Lost {
                    target: "Orders / Create".into(),
                    kind: LostKind::Template {
                        expr: "{{nope(1)}}".into(),
                    },
                },
            ]
        );
    }

    #[test]
    fn push_script_skips_blank_and_keeps_indentation() {
        let mut lost = Vec::new();
        push_script(&mut lost, "Api", ScriptPhase::Test, "   \n  \n");
        assert!(lost.is_empty());
        push_script(
            &mut lost,
            "Api",
            ScriptPhase::Test,
            "\n\n  pm.test('ok', () => {\n    pm.response.to.have.status(200);\n  });\n\n",
        );
        assert_eq!(
            lost,
            vec![Lost {
                target: "Api".into(),
                kind: LostKind::Script {
                    phase: ScriptPhase::Test,
                    source: "  pm.test('ok', () => {\n    pm.response.to.have.status(200);\n  });".into(),
                },
            }]
        );
    }
}
