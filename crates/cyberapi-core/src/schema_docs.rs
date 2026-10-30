//! F11: reading a JSON Schema as the nested list of input or output fields
//! the Docs tab shows — following its own `#/…` `$ref`s (stopping at cycles),
//! merging `allOf`, unwrapping `nullable` / `[T, "null"]` / two-branch
//! `anyOf` with `null`, and stepping into `properties` and `items`.
//!
//! [`infer_schema`] goes the other way: a JSON body or response becomes a
//! starter JSON Schema with empty `description` fields ready to fill in.

use serde_json::{Map, Value, json};
use std::collections::HashSet;

/// How deep nested fields are walked before stopping, so a recursive or
/// deeply nested schema stays bounded.
const MAX_DEPTH: usize = 12;

/// One field read off a JSON Schema for the Docs tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaField {
    /// Its own label on its row (`email`, `items[]`, `[]`, `#1`).
    pub name: String,
    /// Its path from the root (`user.email`, `items[].sku`).
    pub path: String,
    /// How many levels inside the root it sits (`0` at the top).
    pub depth: usize,
    /// Its type as a short label (`string`, `integer (int64)`, `string?`,
    /// `array<object>`).
    pub kind: String,
    /// Whether its enclosing object lists it in `required`.
    pub required: bool,
    /// Its `description`, together with any `enum` or `default` note.
    pub description: String,
}

/// The input or output fields `schema_json` describes, in display order, or
/// why `schema_json` does not read as a JSON Schema. An empty string yields
/// an empty list.
pub fn fields_of_schema(schema_json: &str) -> Result<Vec<SchemaField>, String> {
    if schema_json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let root: Value = serde_json::from_str(schema_json).map_err(|err| err.to_string())?;
    match &root {
        Value::Bool(_) => Ok(Vec::new()),
        Value::Object(_) => {
            let mut out = Vec::new();
            let mut visited = Vec::new();
            walk_root(&root, &root, &mut visited, &mut out);
            Ok(out)
        }
        _ => Err("a JSON Schema is a JSON object".into()),
    }
}

/// A starter JSON Schema read off an example JSON body or response, with
/// empty `"description"` fields on object properties ready to fill in.
pub fn infer_schema(value: &Value) -> Value {
    infer_node(value, false)
}

fn infer_node(value: &Value, with_description: bool) -> Value {
    let mut map = match value {
        Value::Null => Map::from_iter([("type".into(), json!(["string", "null"]))]),
        Value::Bool(_) => Map::from_iter([("type".into(), json!("boolean"))]),
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            Map::from_iter([("type".into(), json!("integer"))])
        }
        Value::Number(_) => Map::from_iter([("type".into(), json!("number"))]),
        Value::String(_) => Map::from_iter([("type".into(), json!("string"))]),
        Value::Array(items) => {
            let inner = items
                .first()
                .map(|first| infer_node(first, false))
                .unwrap_or_else(|| json!({}));
            Map::from_iter([("type".into(), json!("array")), ("items".into(), inner)])
        }
        Value::Object(fields) => {
            let properties: Map<String, Value> = fields
                .iter()
                .map(|(name, field)| (name.clone(), infer_node(field, true)))
                .collect();
            Map::from_iter([
                ("type".into(), json!("object")),
                ("properties".into(), Value::Object(properties)),
            ])
        }
    };
    if with_description {
        map.insert("description".into(), json!(""));
    }
    Value::Object(map)
}

/// `node` after following internal `#/…` `$ref`s that have not been entered
/// yet on `visited`. Returns the target schema and how many pointers were
/// pushed onto `visited` (so the caller can pop them after walking children).
fn deref<'a>(root: &'a Value, mut node: &'a Value, visited: &mut Vec<String>) -> (&'a Value, usize) {
    let mut pushed = 0;
    while let Some(reference) = node.get("$ref").and_then(Value::as_str) {
        let Some(pointer) = reference.strip_prefix('#') else {
            break;
        };
        let key = if pointer.is_empty() { "#" } else { pointer };
        if visited.iter().any(|seen| seen == key) {
            break;
        }
        let target = if pointer.is_empty() {
            Some(root)
        } else {
            root.pointer(pointer)
        };
        let Some(found) = target else {
            break;
        };
        visited.push(key.to_string());
        pushed += 1;
        node = found;
    }
    (node, pushed)
}

fn pop_visited(visited: &mut Vec<String>, count: usize) {
    visited.truncate(visited.len().saturating_sub(count));
}

fn is_null_branch(root: &Value, node: &Value) -> bool {
    let mut seen = Vec::new();
    let (resolved, _) = deref(root, node, &mut seen);
    resolved.get("type").and_then(Value::as_str) == Some("null")
}

/// Unwraps a 2-branch `anyOf` / `oneOf` where one branch is `{"type": "null"}`,
/// returning the non-null branch and marking it nullable.
fn unwrap_nullable<'a>(root: &'a Value, node: &'a Value) -> (&'a Value, bool) {
    for keyword in ["anyOf", "oneOf"] {
        if let Some(branches) = node.get(keyword).and_then(Value::as_array)
            && branches.len() == 2
        {
            if is_null_branch(root, &branches[0]) && !is_null_branch(root, &branches[1]) {
                return (&branches[1], true);
            }
            if is_null_branch(root, &branches[1]) && !is_null_branch(root, &branches[0]) {
                return (&branches[0], true);
            }
        }
    }
    (node, false)
}

/// Collects merged object `properties` (in order) and `required` names from
/// `node` and any `allOf` subschemas inside it.
fn collect_object<'a>(
    root: &'a Value,
    node: &'a Value,
    visited: &mut Vec<String>,
    properties: &mut Vec<(String, &'a Value)>,
    required: &mut HashSet<String>,
) {
    let (resolved, pushed) = deref(root, node, visited);
    let (inner, _) = unwrap_nullable(root, resolved);
    let (inner, inner_pushed) = if std::ptr::eq(inner, resolved) {
        (resolved, 0)
    } else {
        deref(root, inner, visited)
    };

    if let Some(req) = inner.get("required").and_then(Value::as_array) {
        for name in req.iter().filter_map(Value::as_str) {
            required.insert(name.to_string());
        }
    }
    if let Some(all_of) = inner.get("allOf").and_then(Value::as_array) {
        for sub in all_of {
            collect_object(root, sub, visited, properties, required);
        }
    }
    if let Some(props) = inner.get("properties").and_then(Value::as_object) {
        for (name, prop) in props {
            if let Some(slot) = properties.iter_mut().find(|(existing, _)| existing == name) {
                slot.1 = prop;
            } else {
                properties.push((name.clone(), prop));
            }
        }
    }

    pop_visited(visited, inner_pushed + pushed);
}

/// Polymorphic `oneOf` / `anyOf` branches (excluding the 2-branch nullable
/// case already unwrapped).
fn union_branches<'a>(root: &'a Value, node: &'a Value) -> Option<&'a [Value]> {
    for keyword in ["oneOf", "anyOf"] {
        if let Some(branches) = node.get(keyword).and_then(Value::as_array) {
            if branches.len() == 2 && (is_null_branch(root, &branches[0]) || is_null_branch(root, &branches[1])) {
                continue;
            }
            if !branches.is_empty() {
                return Some(branches.as_slice());
            }
        }
    }
    None
}

fn has_subfields(root: &Value, node: &Value, visited: &mut Vec<String>) -> bool {
    let mut props = Vec::new();
    let mut req = HashSet::new();
    collect_object(root, node, visited, &mut props, &mut req);
    if !props.is_empty() {
        return true;
    }
    let (resolved, pushed) = deref(root, node, visited);
    let (inner, _) = unwrap_nullable(root, resolved);
    let (inner, inner_pushed) = if std::ptr::eq(inner, resolved) {
        (resolved, 0)
    } else {
        deref(root, inner, visited)
    };
    let branches_have_props = union_branches(root, inner).is_some_and(|branches| {
        branches.iter().any(|branch| {
            let mut branch_props = Vec::new();
            let mut branch_req = HashSet::new();
            collect_object(root, branch, visited, &mut branch_props, &mut branch_req);
            !branch_props.is_empty()
        })
    });
    pop_visited(visited, inner_pushed + pushed);
    branches_have_props
}

fn type_label(root: &Value, original: &Value, resolved: &Value, extra_nullable: bool) -> String {
    let mut nullable = extra_nullable
        || original.get("nullable").and_then(Value::as_bool) == Some(true)
        || resolved.get("nullable").and_then(Value::as_bool) == Some(true);

    let mut base = match resolved.get("type") {
        Some(Value::String(kind)) if kind == "null" => "null".to_string(),
        Some(Value::String(kind)) => kind.clone(),
        Some(Value::Array(kinds)) => {
            let mut non_null = Vec::new();
            for kind in kinds.iter().filter_map(Value::as_str) {
                if kind == "null" {
                    nullable = true;
                } else {
                    non_null.push(kind);
                }
            }
            if non_null.is_empty() {
                "null".to_string()
            } else {
                non_null.join(" | ")
            }
        }
        _ => {
            if resolved.get("properties").is_some() || resolved.get("allOf").is_some() {
                "object".to_string()
            } else if resolved.get("items").is_some() {
                "array".to_string()
            } else if let Some(branches) = union_branches(root, resolved) {
                let mut names = Vec::new();
                for branch in branches {
                    let mut seen = Vec::new();
                    let (target, _) = deref(root, branch, &mut seen);
                    let label = target
                        .get("title")
                        .and_then(Value::as_str)
                        .filter(|title| !title.trim().is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| type_label(root, branch, target, false));
                    if !label.is_empty() && !names.contains(&label) {
                        names.push(label);
                    }
                }
                if names.is_empty() {
                    "any".to_string()
                } else {
                    names.join(" | ")
                }
            } else if resolved.get("enum").is_some() {
                "enum".to_string()
            } else {
                "any".to_string()
            }
        }
    };

    if base == "array"
        && let Some(items) = resolved.get("items")
    {
        let mut seen = Vec::new();
        let (target, _) = deref(root, items, &mut seen);
        let (target, item_nullable) = unwrap_nullable(root, target);
        let (target, _) = deref(root, target, &mut seen);
        let inner = type_label(root, items, target, item_nullable);
        base = format!("array<{inner}>");
    } else if let Some(format) = resolved
        .get("format")
        .or_else(|| original.get("format"))
        .and_then(Value::as_str)
        .filter(|format| !format.trim().is_empty())
    {
        base = format!("{base} ({format})");
    }

    if nullable && base != "null" && !base.ends_with('?') {
        base.push('?');
    }
    base
}

fn compact_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn description_of(original: &Value, resolved: &Value) -> String {
    let own = original
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let target = resolved
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let mut parts: Vec<String> = Vec::new();
    if let Some(desc) = own.or(target) {
        parts.push(desc.to_string());
    }
    if let Some(values) = resolved
        .get("enum")
        .or_else(|| original.get("enum"))
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
    {
        let listed: Vec<String> = values.iter().take(12).map(compact_value).collect();
        let more = if values.len() > 12 { ", …" } else { "" };
        parts.push(format!("enum: {}{more}", listed.join(" | ")));
    }
    if let Some(default) = resolved
        .get("default")
        .or_else(|| original.get("default"))
        .filter(|value| !value.is_null())
    {
        parts.push(format!("default: {}", compact_value(default)));
    }
    parts.join(" · ")
}

fn walk_root(root: &Value, node: &Value, visited: &mut Vec<String>, out: &mut Vec<SchemaField>) {
    let (resolved, pushed) = deref(root, node, visited);
    let (unwrapped, nullable) = unwrap_nullable(root, resolved);
    let (inner, inner_pushed) = if std::ptr::eq(unwrapped, resolved) {
        (resolved, 0)
    } else {
        deref(root, unwrapped, visited)
    };

    let is_array = inner.get("type").and_then(Value::as_str) == Some("array") || inner.get("items").is_some();
    if is_array {
        let kind = type_label(root, node, inner, nullable);
        let description = description_of(node, inner);
        out.push(SchemaField {
            name: "[]".into(),
            path: "[]".into(),
            depth: 0,
            kind,
            required: false,
            description,
        });
        if let Some(items) = inner.get("items") {
            walk_children(root, items, "[]", 1, visited, out);
        }
        pop_visited(visited, inner_pushed + pushed);
        return;
    }

    let before = out.len();
    walk_children(root, inner, "", 0, visited, out);
    // A root scalar schema (`{"type": "string", "description": "JWT"}`): show
    // one row for the body itself when there were no object properties.
    if out.len() == before {
        let kind = type_label(root, node, inner, nullable);
        let description = description_of(node, inner);
        if kind != "object" || !description.is_empty() {
            out.push(SchemaField {
                name: "(body)".into(),
                path: String::new(),
                depth: 0,
                kind,
                required: false,
                description,
            });
        }
    }

    pop_visited(visited, inner_pushed + pushed);
}

fn walk_children(
    root: &Value,
    node: &Value,
    prefix: &str,
    depth: usize,
    visited: &mut Vec<String>,
    out: &mut Vec<SchemaField>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let mut properties = Vec::new();
    let mut required = HashSet::new();
    collect_object(root, node, visited, &mut properties, &mut required);

    for (name, prop) in properties {
        let is_req = required.contains(&name);
        walk_property(root, &name, prop, is_req, prefix, depth, visited, out);
    }

    let (resolved, pushed) = deref(root, node, visited);
    let (unwrapped, _) = unwrap_nullable(root, resolved);
    let (inner, inner_pushed) = if std::ptr::eq(unwrapped, resolved) {
        (resolved, 0)
    } else {
        deref(root, unwrapped, visited)
    };
    if let Some(branches) = union_branches(root, inner) {
        let any_structured = branches.iter().any(|branch| has_subfields(root, branch, visited));
        if any_structured {
            for (index, branch) in branches.iter().enumerate() {
                let (branch_resolved, branch_pushed) = deref(root, branch, visited);
                let title = branch_resolved
                    .get("title")
                    .or_else(|| branch.get("title"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|title| !title.is_empty());
                let label = match title {
                    Some(title) => format!("#{} {title}", index + 1),
                    None => format!("#{}", index + 1),
                };
                let branch_path = if prefix.is_empty() {
                    label.clone()
                } else {
                    format!("{prefix}.{label}")
                };
                out.push(SchemaField {
                    name: label,
                    path: branch_path.clone(),
                    depth,
                    kind: type_label(root, branch, branch_resolved, false),
                    required: false,
                    description: description_of(branch, branch_resolved),
                });
                walk_children(root, branch_resolved, &branch_path, depth + 1, visited, out);
                pop_visited(visited, branch_pushed);
            }
        }
    }
    pop_visited(visited, inner_pushed + pushed);
}

#[allow(clippy::too_many_arguments)]
fn walk_property(
    root: &Value,
    name: &str,
    prop: &Value,
    required: bool,
    prefix: &str,
    depth: usize,
    visited: &mut Vec<String>,
    out: &mut Vec<SchemaField>,
) {
    let (resolved, pushed) = deref(root, prop, visited);
    let (unwrapped, nullable) = unwrap_nullable(root, resolved);
    let (inner, inner_pushed) = if std::ptr::eq(unwrapped, resolved) {
        (resolved, 0)
    } else {
        deref(root, unwrapped, visited)
    };

    let is_array = inner.get("type").and_then(Value::as_str) == Some("array") || inner.get("items").is_some();
    let items_structured = is_array
        && inner
            .get("items")
            .is_some_and(|items| has_subfields(root, items, visited));

    let shown_name = if items_structured {
        format!("{name}[]")
    } else {
        name.to_string()
    };
    let path = match (prefix.is_empty(), items_structured) {
        (true, true) => format!("{name}[]"),
        (true, false) => name.to_string(),
        (false, true) => format!("{prefix}.{name}[]"),
        (false, false) => format!("{prefix}.{name}"),
    };
    let kind = type_label(root, prop, inner, nullable);
    let description = description_of(prop, inner);

    out.push(SchemaField {
        name: shown_name,
        path: path.clone(),
        depth,
        kind,
        required,
        description,
    });

    if depth < MAX_DEPTH {
        if items_structured {
            if let Some(items) = inner.get("items") {
                walk_children(root, items, &path, depth + 1, visited, out);
            }
        } else {
            walk_children(root, inner, &path, depth + 1, visited, out);
        }
    }

    pop_visited(visited, inner_pushed + pushed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn nested_objects_arrays_and_refs_become_an_indented_field_list() {
        let schema = json!({
            "type": "object",
            "required": ["email", "items"],
            "properties": {
                "email": {
                    "type": "string",
                    "format": "email",
                    "description": "Account email"
                },
                "nickname": {
                    "anyOf": [
                        { "type": "string" },
                        { "type": "null" }
                    ],
                    "description": "Optional handle"
                },
                "role": {
                    "type": "string",
                    "enum": ["admin", "member"],
                    "default": "member",
                    "description": "Access level"
                },
                "items": {
                    "type": "array",
                    "description": "Line items",
                    "items": { "$ref": "#/$defs/LineItem" }
                }
            },
            "$defs": {
                "LineItem": {
                    "allOf": [
                        {
                            "type": "object",
                            "required": ["sku"],
                            "properties": {
                                "sku": { "type": "string", "description": "Product SKU" }
                            }
                        },
                        {
                            "type": "object",
                            "properties": {
                                "qty": { "type": "integer", "format": "int32", "default": 1 }
                            }
                        }
                    ]
                }
            }
        });

        let fields = fields_of_schema(&schema.to_string()).expect("valid schema");
        assert_eq!(
            fields,
            vec![
                SchemaField {
                    name: "email".into(),
                    path: "email".into(),
                    depth: 0,
                    kind: "string (email)".into(),
                    required: true,
                    description: "Account email".into(),
                },
                SchemaField {
                    name: "nickname".into(),
                    path: "nickname".into(),
                    depth: 0,
                    kind: "string?".into(),
                    required: false,
                    description: "Optional handle".into(),
                },
                SchemaField {
                    name: "role".into(),
                    path: "role".into(),
                    depth: 0,
                    kind: "string".into(),
                    required: false,
                    description: "Access level · enum: admin | member · default: member".into(),
                },
                SchemaField {
                    name: "items[]".into(),
                    path: "items[]".into(),
                    depth: 0,
                    kind: "array<object>".into(),
                    required: true,
                    description: "Line items".into(),
                },
                SchemaField {
                    name: "sku".into(),
                    path: "items[].sku".into(),
                    depth: 1,
                    kind: "string".into(),
                    required: true,
                    description: "Product SKU".into(),
                },
                SchemaField {
                    name: "qty".into(),
                    path: "items[].qty".into(),
                    depth: 1,
                    kind: "integer (int32)".into(),
                    required: false,
                    description: "default: 1".into(),
                },
            ]
        );
    }

    #[test]
    fn recursive_refs_stop_without_looping() {
        let schema = json!({
            "$ref": "#/$defs/Node",
            "$defs": {
                "Node": {
                    "type": "object",
                    "required": ["name"],
                    "properties": {
                        "name": { "type": "string" },
                        "children": {
                            "type": "array",
                            "items": { "$ref": "#/$defs/Node" }
                        }
                    }
                }
            }
        });

        let fields = fields_of_schema(&schema.to_string()).expect("recursive schema terminates");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "name");
        assert_eq!(fields[1].name, "children");
        assert_eq!(fields[1].kind, "array<object>");
    }

    #[test]
    fn a_top_level_array_shows_its_root_and_item_fields() {
        let schema = json!({
            "type": "array",
            "description": "Users on this page",
            "items": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "integer", "description": "User ID" }
                }
            }
        });

        let fields = fields_of_schema(&schema.to_string()).expect("array schema");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "[]");
        assert_eq!(fields[0].kind, "array<object>");
        assert_eq!(fields[0].description, "Users on this page");
        assert_eq!(fields[1].name, "id");
        assert_eq!(fields[1].depth, 1);
        assert!(fields[1].required);
    }

    #[test]
    fn inferring_a_schema_leaves_empty_descriptions_on_properties() {
        let example = json!({
            "user": "ann",
            "active": true,
            "roles": ["admin"]
        });
        let schema = infer_schema(&example);
        assert_eq!(
            schema,
            json!({
                "type": "object",
                "properties": {
                    "user": { "type": "string", "description": "" },
                    "active": { "type": "boolean", "description": "" },
                    "roles": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": ""
                    }
                }
            })
        );
    }
}
