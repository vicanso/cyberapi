//! BUG-22: a JSON Schema — what an answer is checked against (A6): the
//! import's, which reads OpenAPI 3.0's ways as 2020-12's, or the reader's
//! own — written as an OpenAPI 3.0 Schema Object, which is a subset of an
//! older JSON Schema with ways of its own:
//!
//! - a null allowed is `nullable: true`, not `null` among the types; more
//!   than one type is an `anyOf`;
//! - an exclusive bound is a flag beside the bound, not a number;
//! - `const` is a one-value `enum`, `examples` one `example`;
//! - a `$ref` to the schema's own `$defs` (or `definitions`) is written in,
//!   to [`MAX_DEPTH`] (past it, anything goes): in a document a `$ref`
//!   points into the document, which has no `$defs`;
//! - what 3.0 does not have (`$schema`, `if`, `prefixItems`, …) is left
//!   out: the schema says less, and other tools take it.

use serde_json::{Map, Value, json};

const MAX_DEPTH: usize = 8;

/// What 3.0's Schema Object does not have, left out.
const LEFT_OUT: [&str; 26] = [
    "$schema",
    "$id",
    "$anchor",
    "$dynamicRef",
    "$dynamicAnchor",
    "$vocabulary",
    "$comment",
    "$defs",
    "definitions",
    "prefixItems",
    "contains",
    "minContains",
    "maxContains",
    "dependentRequired",
    "dependentSchemas",
    "dependencies",
    "unevaluatedProperties",
    "unevaluatedItems",
    "if",
    "then",
    "else",
    "propertyNames",
    "patternProperties",
    "contentMediaType",
    "contentEncoding",
    "contentSchema",
];

/// `schema` as an OpenAPI 3.0 Schema Object.
pub(super) fn openapi_30(schema: &Value) -> Value {
    convert(schema, schema, 0)
}

/// `schema`, a part of `root`, `depth` `$ref`s in.
fn convert(root: &Value, schema: &Value, depth: usize) -> Value {
    let fields = match schema {
        // A schema that takes anything, or nothing.
        Value::Bool(true) => return json!({}),
        Value::Bool(false) => return json!({"not": {}}),
        Value::Object(fields) => fields,
        other => return other.clone(),
    };
    if let Some(target) = fields.get("$ref").and_then(Value::as_str) {
        return match local(root, target) {
            // Too deep — a schema that holds itself — anything goes.
            Some(_) if depth >= MAX_DEPTH => json!({}),
            Some(target) => convert(root, target, depth + 1),
            // Elsewhere: as it is, without what 3.0 ignores beside it.
            None => json!({ "$ref": target }),
        };
    }
    let mut out = Map::new();
    for (key, value) in fields {
        if LEFT_OUT.contains(&key.as_str()) {
            continue;
        }
        let value = match key.as_str() {
            "properties" => match value {
                Value::Object(named) => Value::Object(
                    named
                        .iter()
                        .map(|(name, schema)| (name.clone(), convert(root, schema, depth)))
                        .collect(),
                ),
                other => other.clone(),
            },
            // 3.0 takes a boolean here.
            "additionalProperties" if value.is_boolean() => value.clone(),
            "additionalProperties" | "not" => convert(root, value, depth),
            "items" => match value {
                // An older draft's tuple: each item one of them.
                Value::Array(items) => match items.as_slice() {
                    [] => json!({}),
                    [one] => convert(root, one, depth),
                    many => json!({ "anyOf": many.iter().map(|item| convert(root, item, depth)).collect::<Vec<_>>() }),
                },
                schema => convert(root, schema, depth),
            },
            "allOf" | "anyOf" | "oneOf" => match value {
                Value::Array(schemas) => {
                    Value::Array(schemas.iter().map(|schema| convert(root, schema, depth)).collect())
                }
                other => other.clone(),
            },
            // What is data, and the rest, as it is.
            _ => value.clone(),
        };
        out.insert(key.clone(), value);
    }
    if let Some(value) = out.remove("const") {
        out.entry("enum").or_insert_with(|| json!([value]));
    }
    if let Some(Value::Array(examples)) = out.remove("examples")
        && let Some(first) = examples.into_iter().next()
    {
        out.entry("example").or_insert(first);
    }
    bounds(&mut out);
    types(&mut out);
    null_member(&mut out);
    Value::Object(out)
}

/// What `$ref` points to in `root` itself: `#`, `#/$defs/User`.
fn local<'a>(root: &'a Value, target: &str) -> Option<&'a Value> {
    let pointer = target.strip_prefix('#')?;
    root.pointer(pointer)
}

/// 2020-12's exclusive bound is the bound itself; 3.0's is a flag on
/// `minimum` / `maximum`. With both, the stricter holds.
fn bounds(out: &mut Map<String, Value>) {
    for (flag, bound, lower) in [
        ("exclusiveMinimum", "minimum", true),
        ("exclusiveMaximum", "maximum", false),
    ] {
        let Some(exclusive) = out.get(flag).and_then(Value::as_f64) else {
            continue;
        };
        let stricter = out.get(bound).and_then(Value::as_f64).is_none_or(|inclusive| {
            if lower {
                exclusive >= inclusive
            } else {
                exclusive <= inclusive
            }
        });
        let value = out.remove(flag);
        if stricter && let Some(value) = value {
            out.insert(bound.into(), value);
            out.insert(flag.into(), json!(true));
        }
    }
}

/// `type` as one name: `null` among the types is `nullable`, more than one
/// an `anyOf`.
fn types(out: &mut Map<String, Value>) {
    let kinds: Vec<String> = match out.get("type") {
        Some(Value::Array(kinds)) => kinds.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(kind)) if kind == "null" => vec![kind.clone()],
        _ => return,
    };
    out.remove("type");
    let (nulls, kinds): (Vec<String>, Vec<String>) = kinds.into_iter().partition(|kind| kind == "null");
    match kinds.as_slice() {
        [] => {}
        [one] => {
            out.insert("type".into(), json!(one));
        }
        many => {
            let any: Vec<Value> = many.iter().map(|kind| json!({ "type": kind })).collect();
            match out.get_mut("anyOf") {
                // Beside an `anyOf` of its own: both hold.
                Some(_) => {
                    let all = out.entry("allOf").or_insert_with(|| json!([]));
                    if let Value::Array(all) = all {
                        all.push(json!({ "anyOf": any }));
                    }
                }
                None => {
                    out.insert("anyOf".into(), Value::Array(any));
                }
            }
        }
    }
    if !nulls.is_empty() {
        out.insert("nullable".into(), json!(true));
    }
}

/// An `anyOf` / `oneOf` with `{type: null}` among them, as the import
/// writes a nullable `$ref`: the rest, `nullable`.
fn null_member(out: &mut Map<String, Value>) {
    for key in ["anyOf", "oneOf"] {
        let Some(Value::Array(schemas)) = out.get_mut(key) else {
            continue;
        };
        let before = schemas.len();
        // A `{type: null}` reads as `{nullable: true}` by now.
        schemas.retain(|schema| schema != &json!({"nullable": true}));
        if schemas.len() == before {
            continue;
        }
        out.insert("nullable".into(), json!(true));
        let rest = match out.remove(key) {
            Some(Value::Array(rest)) => rest,
            _ => continue,
        };
        match <[Value; 1]>::try_from(rest) {
            // The one left, in place, unless it says what is said here.
            Ok([Value::Object(one)]) if !one.contains_key("$ref") && one.keys().all(|key| !out.contains_key(key)) => {
                out.extend(one);
            }
            Ok([one]) => {
                let all = out.entry("allOf").or_insert_with(|| json!([]));
                if let Value::Array(all) = all {
                    all.push(one);
                }
            }
            Err(rest) if rest.is_empty() => {}
            Err(rest) => {
                out.insert(key.into(), Value::Array(rest));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_null_allowed_is_nullable() {
        assert_eq!(
            openapi_30(&json!({"type": ["string", "null"], "maxLength": 3})),
            json!({"type": "string", "nullable": true, "maxLength": 3})
        );
        assert_eq!(openapi_30(&json!({"type": "null"})), json!({"nullable": true}));
        assert_eq!(
            openapi_30(&json!({"type": ["integer", "string"]})),
            json!({"anyOf": [{"type": "integer"}, {"type": "string"}]})
        );
        // The import's nullable `$ref`, written in.
        assert_eq!(
            openapi_30(&json!({"anyOf": [{"type": "object", "required": ["id"]}, {"type": "null"}]})),
            json!({"type": "object", "required": ["id"], "nullable": true})
        );
        assert_eq!(
            openapi_30(&json!({"anyOf": [{"$ref": "https://elsewhere.test/a.json"}, {"type": "null"}]})),
            json!({"allOf": [{"$ref": "https://elsewhere.test/a.json"}], "nullable": true})
        );
        assert_eq!(
            openapi_30(&json!({"oneOf": [{"type": "string"}, {"type": "integer"}, {"type": "null"}]})),
            json!({"oneOf": [{"type": "string"}, {"type": "integer"}], "nullable": true})
        );
    }

    #[test]
    fn an_exclusive_bound_is_a_flag() {
        assert_eq!(
            openapi_30(&json!({"type": "integer", "exclusiveMinimum": 0, "exclusiveMaximum": 10})),
            json!({"type": "integer", "minimum": 0, "exclusiveMinimum": true, "maximum": 10, "exclusiveMaximum": true})
        );
        // With an inclusive bound too, the stricter.
        assert_eq!(
            openapi_30(&json!({"minimum": 5, "exclusiveMinimum": 0, "maximum": 8, "exclusiveMaximum": 8})),
            json!({"minimum": 5, "maximum": 8, "exclusiveMaximum": true})
        );
        // Already 3.0's.
        assert_eq!(
            openapi_30(&json!({"minimum": 1, "exclusiveMinimum": true})),
            json!({"minimum": 1, "exclusiveMinimum": true})
        );
    }

    #[test]
    fn its_own_definitions_are_written_in() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://shop.test/order.json",
            "type": "object",
            "properties": {
                "owner": {"$ref": "#/$defs/Owner"},
                "tags": {"type": "array", "items": {"$ref": "#/definitions/Tag"}},
                "next": {"$ref": "#"},
                "kind": {"const": "order"},
                "any": true,
            },
            "$defs": {"Owner": {"type": "object", "properties": {"name": {"type": ["string", "null"]}}}},
            "definitions": {"Tag": {"type": "string", "examples": ["a", "b"]}},
            "if": {"required": ["kind"]},
            "then": {"required": ["owner"]},
        });
        let out = openapi_30(&schema);
        assert_eq!(
            out["properties"]["owner"],
            json!({"type": "object", "properties": {"name": {"type": "string", "nullable": true}}})
        );
        assert_eq!(
            out["properties"]["tags"]["items"],
            json!({"type": "string", "example": "a"})
        );
        assert_eq!(out["properties"]["kind"], json!({"enum": ["order"]}));
        assert_eq!(out["properties"]["any"], json!({}));
        for left_out in ["$schema", "$id", "$defs", "definitions", "if", "then"] {
            assert!(out.get(left_out).is_none(), "{left_out} is not 3.0's");
        }
        // A schema that holds itself, as deep as it goes and no further.
        let mut next = &out["properties"]["next"];
        let mut depth = 0;
        while next.get("properties").is_some() {
            next = &next["properties"]["next"];
            depth += 1;
        }
        assert_eq!((depth, next), (MAX_DEPTH, &json!({})));
        assert!(
            !out.to_string().contains("$ref"),
            "no reference left to a place the document lacks"
        );
    }
}
