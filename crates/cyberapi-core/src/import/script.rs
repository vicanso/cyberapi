//! M3: static conversion of common Postman, Bruno (`.bru` / OpenCollection),
//! and Insomnia test / post-response scripts into native [`Assertion`]s and
//! [`Extraction`]s, without executing any JavaScript.
//!
//! Any statements that cannot be safely converted — including pre-request
//! scripts — are preserved verbatim and passed back to the caller so the
//! importer can append them to the API's description and record them in the
//! M1 import report ([`Lost`]).

use super::lost::{self, Lost, ScriptPhase};
use crate::assertions::{Assertion, Extraction, Op, Subject, TYPES};
use crate::model::{HttpRequest, KvParam};
use serde_json::Value;

/// What static analysis extracted from a single script block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ScriptConversion {
    pub assertions: Vec<Assertion>,
    pub extractions: Vec<Extraction>,
    /// Normalized statements that could not be converted (empty when everything
    /// in the script was converted or only comments/whitespace existed).
    pub unconverted: String,
}

/// Applies `source` (in `phase`) to `request`: converted assertions and
/// extractions are added directly, while any unconverted statements are
/// appended to `request.description` and pushed to `lost`.
pub(super) fn apply_script(
    request: &mut HttpRequest,
    lost: &mut Vec<Lost>,
    target: &str,
    phase: ScriptPhase,
    source: &str,
) {
    let conversion = convert_script(phase, source);
    request.assertions.extend(conversion.assertions);
    for extraction in conversion.extractions {
        push_extraction_unique(&mut request.extractions, extraction);
    }
    record_unconverted(request, lost, target, phase, &conversion.unconverted);
}

/// Converts Bruno `.bru` `vars:post-response` pairs into [`Extraction`]s when
/// the expression references a response JSONPath, header, status, body or time.
/// Anything else is preserved in `request.description` and `lost`.
pub(super) fn apply_bru_post_vars(request: &mut HttpRequest, lost: &mut Vec<Lost>, target: &str, vars: &[KvParam]) {
    let bindings = Bindings::default();
    let mut unconverted = Vec::new();
    for var in vars {
        let name = var.key.trim();
        let value = var.value.trim();
        if name.is_empty() {
            continue;
        }
        if let Some(expr) = resolve_expr(value, &bindings)
            && let Some((subject, ext_target)) = expr_to_extraction_target(&expr)
        {
            push_extraction_unique(
                &mut request.extractions,
                Extraction {
                    enabled: var.enabled,
                    name: name.to_string(),
                    subject,
                    target: ext_target,
                },
            );
        } else {
            unconverted.push(format_bru_pair(var.enabled, name, value));
        }
    }
    if !unconverted.is_empty() {
        record_unconverted(
            request,
            lost,
            target,
            ScriptPhase::PostResponse,
            &unconverted.join("\n"),
        );
    }
}

/// Converts Bruno `.bru` `assert` pairs (`expr: op value`) into [`Assertion`]s.
/// Unrecognized expressions or operators stay in `request.description` and `lost`.
pub(super) fn apply_bru_asserts(request: &mut HttpRequest, lost: &mut Vec<Lost>, target: &str, asserts: &[KvParam]) {
    let mut unconverted = Vec::new();
    for entry in asserts {
        let expr = entry.key.trim();
        let raw_rhs = entry.value.trim();
        if expr.is_empty() {
            continue;
        }
        let (op_word, rhs) = split_first_word(raw_rhs);
        if let Some(assertion) = convert_bru_assertion(expr, op_word, rhs, entry.enabled) {
            request.assertions.push(assertion);
        } else {
            unconverted.push(format_bru_pair(entry.enabled, expr, raw_rhs));
        }
    }
    if !unconverted.is_empty() {
        record_unconverted(request, lost, target, ScriptPhase::Test, &unconverted.join("\n"));
    }
}

/// Converts OpenCollection `/runtime/assertions` objects into [`Assertion`]s.
pub(super) fn apply_opencollection_asserts(
    request: &mut HttpRequest,
    lost: &mut Vec<Lost>,
    target: &str,
    entries: &[Value],
) {
    let mut unconverted = Vec::new();
    for entry in entries {
        let expr = entry
            .get("expression")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let operator = entry.get("operator").and_then(Value::as_str).unwrap_or_default().trim();
        if expr.is_empty() && operator.is_empty() {
            continue;
        }
        let enabled = !entry.get("disabled").and_then(Value::as_bool).unwrap_or(false);
        let value = match entry.get("value") {
            Some(Value::String(s)) => s.trim().to_string(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        };
        if let Some(assertion) = convert_bru_assertion(expr, operator, &value, enabled) {
            request.assertions.push(assertion);
        } else {
            let prefix = if enabled { "" } else { "~" };
            let rhs = if value.is_empty() {
                operator.to_string()
            } else {
                format!("{operator} {value}")
            };
            unconverted.push(format!("{prefix}{expr}: {rhs}").trim_end().to_string());
        }
    }
    if !unconverted.is_empty() {
        record_unconverted(request, lost, target, ScriptPhase::Test, &unconverted.join("\n"));
    }
}

/// Appends unconverted script statements to `description` as a fenced JS code
/// block under a phase heading, preserving any existing description above it.
pub(super) fn append_unconverted_to_description(description: &mut String, phase: ScriptPhase, source: &str) {
    let normalized = normalize_script(source);
    if normalized.is_empty() {
        return;
    }
    let heading = match phase {
        ScriptPhase::PreRequest => "#### Pre-request script (not converted)",
        ScriptPhase::PostResponse => "#### Post-response script (not converted)",
        ScriptPhase::Test => "#### Test script (not converted)",
        ScriptPhase::Other => "#### Script (not converted)",
    };
    let max_ticks = max_backtick_run(&normalized);
    let fence = "`".repeat(max_ticks.max(2) + 1);
    let block = format!("{heading}\n\n{fence}js\n{normalized}\n{fence}");
    if description.trim().is_empty() {
        *description = block;
    } else {
        let existing = description.trim_end().to_string();
        *description = format!("{existing}\n\n{block}");
    }
}

/// Statically converts a script in `phase` without executing JavaScript.
pub(super) fn convert_script(phase: ScriptPhase, source: &str) -> ScriptConversion {
    let normalized = normalize_script(source);
    if normalized.is_empty() {
        return ScriptConversion::default();
    }
    if phase == ScriptPhase::PreRequest {
        // Pre-request scripts run before any response exists; keep non-comment
        // scripts verbatim as unconverted.
        let stmts = split_statements(&normalized);
        let has_code = stmts.iter().any(|stmt| !strip_js_comments(&stmt.raw).trim().is_empty());
        return ScriptConversion {
            assertions: Vec::new(),
            extractions: Vec::new(),
            unconverted: if has_code { normalized } else { String::new() },
        };
    }
    let outcome = convert_block(&normalized, &Bindings::default());
    ScriptConversion {
        assertions: outcome.assertions,
        extractions: outcome.extractions,
        unconverted: outcome.unconverted_lines.join("\n"),
    }
}

fn record_unconverted(request: &mut HttpRequest, lost: &mut Vec<Lost>, target: &str, phase: ScriptPhase, source: &str) {
    if normalize_script(source).is_empty() {
        return;
    }
    append_unconverted_to_description(&mut request.description, phase, source);
    lost::push_script(lost, target, phase, source);
}

fn push_extraction_unique(out: &mut Vec<Extraction>, extraction: Extraction) {
    if !out.iter().any(|existing| {
        existing.enabled == extraction.enabled
            && existing.name == extraction.name
            && existing.subject == extraction.subject
            && existing.target == extraction.target
    }) {
        out.push(extraction);
    }
}

fn format_bru_pair(enabled: bool, key: &str, value: &str) -> String {
    let prefix = if enabled { "" } else { "~" };
    if value.is_empty() {
        format!("{prefix}{key}:")
    } else {
        format!("{prefix}{key}: {value}")
    }
}

fn normalize_script(source: &str) -> String {
    let trimmed = source.trim_end();
    if trimmed.trim().is_empty() {
        return String::new();
    }
    let lines: Vec<&str> = trimmed.lines().collect();
    let first = lines.iter().position(|line| !line.trim().is_empty()).unwrap_or(0);
    lines[first..].join("\n").trim_end().to_string()
}

fn max_backtick_run(text: &str) -> usize {
    let mut best = 0usize;
    let mut current = 0usize;
    for ch in text.chars() {
        if ch == '`' {
            current += 1;
            best = best.max(current);
        } else {
            current = 0;
        }
    }
    best
}

fn split_first_word(text: &str) -> (&str, &str) {
    let trimmed = text.trim();
    match trimmed.find(char::is_whitespace) {
        Some(idx) => (&trimmed[..idx], trimmed[idx..].trim_start()),
        None => (trimmed, ""),
    }
}

// ---------------------------------------------------------------------------
// Response expressions & local variable bindings
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum PathSeg {
    Field(String),
    Index(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ExprRef {
    Status,
    Latency,
    Size,
    BodyText,
    Json { path: Vec<PathSeg>, bare_res_body: bool },
    HeadersMap,
    Header(String),
}

#[derive(Clone, Debug)]
struct AliasDecl {
    names: Vec<String>,
    raw: String,
    from_unconverted_dep: bool,
}

#[derive(Clone, Debug, Default)]
struct Bindings {
    vars: Vec<(String, ExprRef)>,
}

impl Bindings {
    fn get(&self, name: &str) -> Option<&ExprRef> {
        self.vars.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    fn set(&mut self, name: String, value: ExprRef) {
        if let Some((_, existing)) = self.vars.iter_mut().find(|(k, _)| k == &name) {
            *existing = value;
        } else {
            self.vars.push((name, value));
        }
    }
}

fn is_ascii_ident(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

fn format_json_path(path: &[PathSeg]) -> String {
    let mut out = String::from("$");
    for seg in path {
        match seg {
            PathSeg::Field(key) => {
                if !key.is_empty() && is_ascii_ident(key) && !key.starts_with('$') {
                    out.push('.');
                    out.push_str(key);
                } else {
                    let escaped = key.replace('\\', "\\\\").replace('\'', "\\'");
                    out.push_str(&format!("['{escaped}']"));
                }
            }
            PathSeg::Index(idx) => out.push_str(&format!("[{idx}]")),
        }
    }
    out
}

fn expr_to_assertion_subject(expr: &ExprRef, op: Op) -> Option<(Subject, String)> {
    match expr {
        ExprRef::Status => Some((Subject::Status, String::new())),
        ExprRef::Latency => Some((Subject::Latency, String::new())),
        ExprRef::Size => Some((Subject::Size, String::new())),
        ExprRef::BodyText => Some((Subject::Body, String::new())),
        ExprRef::Header(name) => Some((Subject::Header, name.clone())),
        ExprRef::HeadersMap => None,
        ExprRef::Json { path, bare_res_body } => {
            if !path.is_empty() {
                Some((Subject::Json, format_json_path(path)))
            } else if *bare_res_body && op != Op::IsType {
                Some((Subject::Body, String::new()))
            } else {
                Some((Subject::Json, "$".to_string()))
            }
        }
    }
}

fn expr_to_extraction_target(expr: &ExprRef) -> Option<(Subject, String)> {
    match expr {
        ExprRef::Status => Some((Subject::Status, String::new())),
        ExprRef::Latency => Some((Subject::Latency, String::new())),
        ExprRef::Size => Some((Subject::Size, String::new())),
        ExprRef::BodyText => Some((Subject::Body, String::new())),
        ExprRef::Header(name) => Some((Subject::Header, name.clone())),
        ExprRef::HeadersMap => None,
        ExprRef::Json { path, bare_res_body } => {
            if !path.is_empty() {
                Some((Subject::Json, format_json_path(path)))
            } else if *bare_res_body {
                Some((Subject::Body, String::new()))
            } else {
                Some((Subject::Json, "$".to_string()))
            }
        }
    }
}

fn validated_assertion(enabled: bool, subject: Subject, target: String, op: Op, expected: String) -> Option<Assertion> {
    if !subject.ops().contains(&op) {
        return None;
    }
    if subject.has_target() && target.trim().is_empty() {
        return None;
    }
    if op.takes_value() && expected.is_empty() {
        // An assertion expecting "" matches nothing if inactive, keep only when valid
        return None;
    }
    Some(Assertion {
        enabled,
        subject,
        target,
        op,
        expected,
        imported: false,
    })
}

fn validated_assertion_allow_empty_expected(
    enabled: bool,
    subject: Subject,
    target: String,
    op: Op,
    expected: String,
) -> Option<Assertion> {
    if !subject.ops().contains(&op) {
        return None;
    }
    if subject.has_target() && target.trim().is_empty() {
        return None;
    }
    Some(Assertion {
        enabled,
        subject,
        target,
        op,
        expected,
        imported: false,
    })
}

fn convert_bru_assertion(expr: &str, op_word: &str, raw_rhs: &str, enabled: bool) -> Option<Assertion> {
    let bindings = Bindings::default();
    let parsed_expr = resolve_expr(expr, &bindings)?;
    let (op, expected) = match op_word {
        "eq" => (Op::Equals, parse_bru_value(raw_rhs)?),
        "neq" => (Op::NotEquals, parse_bru_value(raw_rhs)?),
        "gt" => (Op::Greater, parse_number_literal(raw_rhs)?),
        "lt" => (Op::Less, parse_number_literal(raw_rhs)?),
        "contains" => (Op::Contains, parse_bru_value(raw_rhs)?),
        "notContains" => (Op::NotContains, parse_bru_value(raw_rhs)?),
        "matches" => {
            let pattern = parse_regex_or_string(raw_rhs)?;
            (Op::Matches, pattern)
        }
        "isDefined" => (Op::Exists, String::new()),
        "isUndefined" => (Op::NotExists, String::new()),
        "isNull" => (Op::Equals, "null".to_string()),
        "isString" => (Op::IsType, "string".to_string()),
        "isNumber" => (Op::IsType, "number".to_string()),
        "isBoolean" => (Op::IsType, "boolean".to_string()),
        "isArray" => (Op::IsType, "array".to_string()),
        _ => return None,
    };
    let (subject, target) = expr_to_assertion_subject(&parsed_expr, op)?;
    if op == Op::IsType && subject != Subject::Json {
        return None;
    }
    validated_assertion_allow_empty_expected(enabled, subject, target, op, expected)
}

fn parse_bru_value(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(s) = parse_string_literal(trimmed) {
        return Some(s);
    }
    Some(trimmed.to_string())
}

// ---------------------------------------------------------------------------
// Block & Statement conversion
// ---------------------------------------------------------------------------

#[derive(Default)]
struct BlockOutcome {
    assertions: Vec<Assertion>,
    extractions: Vec<Extraction>,
    unconverted_lines: Vec<String>,
}

struct PendingUnconverted {
    raw: String,
    is_alias_decl: Option<AliasDecl>,
}

fn convert_block(source: &str, outer_bindings: &Bindings) -> BlockOutcome {
    let stmts = split_statements(source);
    let mut bindings = outer_bindings.clone();
    let mut assertions = Vec::new();
    let mut extractions = Vec::new();
    let mut pending: Vec<PendingUnconverted> = Vec::new();
    let phase = ScriptPhase::Test;

    for stmt in stmts {
        let code = strip_js_comments(&stmt.raw);
        let code = code.trim();
        let code = code.strip_suffix(';').map(str::trim).unwrap_or(code);
        if code.is_empty() {
            continue;
        }

        // 1. Test / describe wrapper: pm.test(...), insomnia.test(...), test(...), it(...), describe(...)
        if let Some(body) = parse_test_wrapper(code) {
            let inner = convert_block(body, &bindings);
            let converted_count = inner.assertions.len() + inner.extractions.len();
            if converted_count > 0 {
                assertions.extend(inner.assertions);
                for ext in inner.extractions {
                    push_extraction_unique(&mut extractions, ext);
                }
                for line in inner.unconverted_lines {
                    pending.push(PendingUnconverted {
                        raw: line,
                        is_alias_decl: None,
                    });
                }
            } else {
                // Keep empty or wholly non-convertible test wrappers intact so
                // users see them in description & M1 import report.
                pending.push(PendingUnconverted {
                    raw: stmt.raw,
                    is_alias_decl: None,
                });
            }
            continue;
        }

        // 2. Variable declaration: const json = pm.response.json() / const { token } = json.data
        if let Some((new_vars, names)) = parse_var_decl(code, &bindings) {
            for (name, val) in new_vars {
                bindings.set(name, val);
            }
            pending.push(PendingUnconverted {
                raw: stmt.raw.clone(),
                is_alias_decl: Some(AliasDecl {
                    names,
                    raw: stmt.raw,
                    from_unconverted_dep: false,
                }),
            });
            continue;
        }

        // 3. Extraction setters: pm.environment.set / bru.setVar / insomnia.environment.set / ...
        if let Some(ext) = convert_extraction_stmt(code, phase, &bindings) {
            push_extraction_unique(&mut extractions, ext);
            continue;
        }

        // 4. Assertions: pm.response.to.*, pm.expect(...), expect(...), tests[...], assert(...)
        if let Some(assertion) = convert_assertion_stmt(code, &bindings) {
            assertions.push(assertion);
            continue;
        }

        // 5. Fallback: unconverted statement
        pending.push(PendingUnconverted {
            raw: stmt.raw,
            is_alias_decl: None,
        });
    }

    // Filter out alias declarations unless some genuinely unconverted statement
    // references the alias identifier.
    let mut non_alias_sources: Vec<String> = pending
        .iter()
        .filter(|item| item.is_alias_decl.is_none())
        .map(|item| item.raw.clone())
        .collect();

    if !non_alias_sources.is_empty() {
        // Walk backwards so transitive alias dependencies are retained if needed.
        for item in pending.iter_mut().rev() {
            let Some(decl) = &mut item.is_alias_decl else {
                continue;
            };
            if decl
                .names
                .iter()
                .any(|name| non_alias_sources.iter().any(|src| contains_ident(src, name)))
            {
                decl.from_unconverted_dep = true;
                non_alias_sources.push(decl.raw.clone());
            }
        }
    }

    let unconverted_lines = pending
        .into_iter()
        .filter_map(|item| match item.is_alias_decl {
            None => Some(item.raw),
            Some(decl) if decl.from_unconverted_dep => Some(decl.raw),
            Some(_) => None,
        })
        .collect();

    BlockOutcome {
        assertions,
        extractions,
        unconverted_lines,
    }
}

fn contains_ident(source: &str, ident: &str) -> bool {
    let bytes = source.as_bytes();
    let target = ident.as_bytes();
    if target.is_empty() || bytes.len() < target.len() {
        return false;
    }
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut from = 0usize;
    while let Some(pos) = source[from..].find(ident) {
        let start = from + pos;
        let end = start + target.len();
        let before_ok = start == 0 || !is_word(bytes[start - 1]);
        let after_ok = end == bytes.len() || !is_word(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = start + ident.len();
    }
    false
}

fn parse_test_wrapper(code: &str) -> Option<&str> {
    let prefixes = ["pm.test", "insomnia.test", "test", "it", "describe"];
    let open = find_call_after_prefix(code, &prefixes)?;
    let close = matching_delimiter(code, open, '(', ')')?;
    if !code[close + 1..].trim().is_empty() {
        return None;
    }
    let args = split_top_level(&code[open + 1..close], ',');
    if args.len() < 2 {
        return None;
    }
    let cb = args.last()?.trim();
    extract_callback_body(cb)
}

fn extract_callback_body(cb: &str) -> Option<&str> {
    let cb = cb.trim();
    // [async] function [name](...) { body }
    let after_async = cb.strip_prefix("async").map(str::trim_start).unwrap_or(cb);
    if let Some(rest) = after_async.strip_prefix("function")
        && rest.starts_with(|c: char| c.is_whitespace() || c == '(')
    {
        let open_paren = find_outside_strings(cb, '(')?;
        let close_paren = matching_delimiter(cb, open_paren, '(', ')')?;
        let after_params = cb[close_paren + 1..].trim_start();
        let brace_rel = after_params.starts_with('{').then_some(0)?;
        let open_brace = cb.len() - after_params.len() + brace_rel;
        let close_brace = matching_delimiter(cb, open_brace, '{', '}')?;
        if !cb[close_brace + 1..].trim().is_empty() {
            return None;
        }
        return Some(&cb[open_brace + 1..close_brace]);
    }
    // Arrow function: [async] (...) => ... OR [async] ident => ...
    let arrow = find_top_level_str(cb, "=>")?;
    let rhs = cb[arrow + 2..].trim();
    if rhs.starts_with('{') {
        let rhs_start = cb.len() - cb[arrow + 2..].trim_start().len();
        let close_brace = matching_delimiter(cb, rhs_start, '{', '}')?;
        if !cb[close_brace + 1..].trim().is_empty() {
            return None;
        }
        return Some(&cb[rhs_start + 1..close_brace]);
    }
    Some(rhs)
}

type ParsedBindings = (Vec<(String, ExprRef)>, Vec<String>);
type CallRoot = (&'static str, fn(&str) -> Option<ExprRef>);

fn parse_var_decl(code: &str, bindings: &Bindings) -> Option<ParsedBindings> {
    let rest = ["const", "let", "var"].into_iter().find_map(|kw| {
        let tail = code.strip_prefix(kw)?;
        tail.chars()
            .next()
            .is_some_and(char::is_whitespace)
            .then(|| tail.trim())
    })?;
    let eq = find_top_level_assignment(rest)?;
    let lhs = rest[..eq].trim();
    let rhs = rest[eq + 1..].trim();
    let target_expr = resolve_expr(rhs, bindings)?;

    if is_ascii_ident(lhs) {
        let name = lhs.to_string();
        return Some((vec![(name.clone(), target_expr)], vec![name]));
    }

    // Simple flat object destructuring: { token, user_id: userId } = json.data
    if lhs.starts_with('{')
        && lhs.ends_with('}')
        && let ExprRef::Json { path, .. } = target_expr
    {
        let inner = &lhs[1..lhs.len() - 1];
        let mut bound = Vec::new();
        let mut names = Vec::new();
        for part in inner.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if part.starts_with("...") || part.contains('=') {
                return None;
            }
            let (prop, alias) = match part.split_once(':') {
                Some((prop, alias)) => (prop.trim(), alias.trim()),
                None => (part, part),
            };
            if !is_ascii_ident(prop) || !is_ascii_ident(alias) {
                return None;
            }
            let mut sub_path = path.clone();
            sub_path.push(PathSeg::Field(prop.to_string()));
            bound.push((
                alias.to_string(),
                ExprRef::Json {
                    path: sub_path,
                    bare_res_body: false,
                },
            ));
            names.push(alias.to_string());
        }
        if !bound.is_empty() {
            return Some((bound, names));
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Extraction conversion
// ---------------------------------------------------------------------------

fn convert_extraction_stmt(code: &str, phase: ScriptPhase, bindings: &Bindings) -> Option<Extraction> {
    if phase == ScriptPhase::PreRequest {
        return None;
    }
    let setters = [
        "pm.environment.set",
        "pm.collectionVariables.set",
        "pm.variables.set",
        "pm.globals.set",
        "postman.setEnvironmentVariable",
        "postman.setGlobalVariable",
        "bru.setVar",
        "bru.setEnvVar",
        "bru.setGlobalEnvVar",
        "insomnia.environment.set",
        "insomnia.variables.set",
        "insomnia.collectionVariables.set",
        "insomnia.globals.set",
    ];
    let open = find_call_after_prefix(code, &setters)?;
    let close = matching_delimiter(code, open, '(', ')')?;
    if !code[close + 1..].trim().is_empty() {
        return None;
    }
    let args = split_top_level(&code[open + 1..close], ',');
    if args.len() != 2 {
        return None;
    }
    let var_name = parse_string_literal(args[0].trim())?;
    let var_name = var_name.trim();
    if var_name.is_empty() {
        return None;
    }
    let expr = resolve_expr(args[1].trim(), bindings)?;
    let (subject, target) = expr_to_extraction_target(&expr)?;
    Some(Extraction {
        enabled: true,
        name: var_name.to_string(),
        subject,
        target,
    })
}

// ---------------------------------------------------------------------------
// Assertion conversion
// ---------------------------------------------------------------------------

fn convert_assertion_stmt(code: &str, bindings: &Bindings) -> Option<Assertion> {
    // 1. Postman / Insomnia fluent response assertions: pm.response.to.*, insomnia.response.to.*
    for root in ["pm.response", "insomnia.response"] {
        if let Some(chain) = strip_member_prefix(code, root) {
            return convert_fluent_response_chain(chain);
        }
    }

    // 2. Chai / Jest expect(...): pm.expect(...), insomnia.expect(...), expect(...)
    if let Some(open) = find_call_after_prefix(code, &["pm.expect", "insomnia.expect", "expect"]) {
        let close = matching_delimiter(code, open, '(', ')')?;
        let target_arg = code[open + 1..close].trim();
        let rest = code[close + 1..].trim_start();
        if !rest.is_empty() {
            let expr = resolve_expr(target_arg, bindings)?;
            return convert_expect_chain(&expr, rest);
        }
    }

    // 3. Legacy Postman: tests["..."] = <bool expr>
    if let Some(after_tests) = strip_ident_prefix(code, "tests") {
        let full_open = after_tests.trim_start();
        if full_open.starts_with('[')
            && let Some(close) = matching_delimiter(full_open, 0, '[', ']')
        {
            let key_raw = &full_open[1..close];
            if parse_string_literal(key_raw.trim()).is_some()
                && let Some(after_eq) = full_open[close + 1..].trim_start().strip_prefix('=')
                && !after_eq.starts_with('=')
            {
                return convert_boolean_expr(after_eq.trim(), bindings);
            }
        }
    }

    // 4. Node / Chai assert calls
    if let Some(assertion) = convert_assert_call(code, bindings) {
        return Some(assertion);
    }

    // 5. Direct boolean / comparison expression at top level
    convert_boolean_expr(code, bindings)
}

fn convert_fluent_response_chain(chain: &str) -> Option<Assertion> {
    let segs = parse_dot_chain(chain)?;
    let mut negated = false;
    let mut i = 0usize;
    let expect_to = segs.first()?;
    if expect_to.name != "to" || expect_to.args.is_some() {
        return None;
    }
    i += 1;
    while i < segs.len() {
        let name = segs[i].name;
        if segs[i].args.is_none() && matches!(name, "be" | "have") {
            i += 1;
            continue;
        }
        if segs[i].args.is_none() && name == "not" {
            negated = !negated;
            i += 1;
            continue;
        }
        break;
    }
    if i != segs.len() - 1 {
        return None;
    }
    let last = &segs[i];
    match (last.name, last.args) {
        ("status", Some(args_str)) => {
            let args = split_top_level(args_str, ',');
            if args.len() != 1 {
                return None;
            }
            let num = parse_number_literal(args[0].trim())?;
            let op = if negated { Op::NotEquals } else { Op::Equals };
            validated_assertion(true, Subject::Status, String::new(), op, num)
        }
        ("ok" | "created" | "accepted" | "badRequest" | "unauthorized" | "forbidden" | "notFound", None) => {
            let code = match last.name {
                "ok" => "200",
                "created" => "201",
                "accepted" => "202",
                "badRequest" => "400",
                "unauthorized" => "401",
                "forbidden" => "403",
                "notFound" => "404",
                _ => return None,
            };
            let op = if negated { Op::NotEquals } else { Op::Equals };
            validated_assertion(true, Subject::Status, String::new(), op, code.to_string())
        }
        ("header", Some(args_str)) => {
            let args = split_top_level(args_str, ',');
            match args.as_slice() {
                [header_arg] => {
                    let header = parse_string_literal(header_arg.trim())?;
                    let op = if negated { Op::NotExists } else { Op::Exists };
                    validated_assertion_allow_empty_expected(true, Subject::Header, header, op, String::new())
                }
                [header_arg, val_arg] => {
                    let header = parse_string_literal(header_arg.trim())?;
                    let val = parse_static_expected(val_arg.trim())?;
                    let op = if negated { Op::NotEquals } else { Op::Equals };
                    validated_assertion(true, Subject::Header, header, op, val)
                }
                _ => None,
            }
        }
        ("body", Some(args_str)) => {
            let args = split_top_level(args_str, ',');
            if args.len() != 1 {
                return None;
            }
            let arg = args[0].trim();
            if let Some(pattern) = parse_regex_literal(arg) {
                if negated {
                    return None;
                }
                return validated_assertion(true, Subject::Body, String::new(), Op::Matches, pattern);
            }
            let val = parse_static_expected(arg)?;
            let op = if negated { Op::NotEquals } else { Op::Equals };
            validated_assertion(true, Subject::Body, String::new(), op, val)
        }
        ("jsonBody", Some(args_str)) => {
            let args = split_top_level(args_str, ',');
            match args.as_slice() {
                [path_arg] => {
                    let path = parse_string_literal(path_arg.trim())?;
                    let path = normalize_json_path_str(&path)?;
                    let op = if negated { Op::NotExists } else { Op::Exists };
                    validated_assertion_allow_empty_expected(true, Subject::Json, path, op, String::new())
                }
                [path_arg, val_arg] => {
                    let path = parse_string_literal(path_arg.trim())?;
                    let path = normalize_json_path_str(&path)?;
                    let val = parse_static_expected(val_arg.trim())?;
                    let op = if negated { Op::NotEquals } else { Op::Equals };
                    validated_assertion(true, Subject::Json, path, op, val)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn normalize_json_path_str(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('$') {
        Some(trimmed.to_string())
    } else if trimmed.starts_with('.') || trimmed.starts_with('[') {
        Some(format!("${trimmed}"))
    } else {
        Some(format!("$.{trimmed}"))
    }
}

fn convert_expect_chain(expr: &ExprRef, chain_str: &str) -> Option<Assertion> {
    let segs = parse_dot_chain(chain_str)?;
    let mut negated = false;
    let mut idx = 0usize;
    while idx < segs.len() {
        let seg = &segs[idx];
        if seg.args.is_none() && seg.name == "not" {
            negated = !negated;
            idx += 1;
            continue;
        }
        if seg.args.is_none() && is_chai_filler(seg.name) {
            idx += 1;
            continue;
        }
        break;
    }
    if idx != segs.len() - 1 {
        return None;
    }
    let last = &segs[idx];
    // Support Jest `.not.toBe(...)` style where `.not` already flipped `negated`.
    let (op, target_expr, expected, allow_empty) = match (last.name, last.args) {
        ("eql" | "equal" | "equals" | "eq" | "toBe" | "toEqual" | "toStrictEqual", Some(args_str)) => {
            let arg = single_arg(args_str)?;
            if arg == "undefined" {
                let op = if negated { Op::Exists } else { Op::NotExists };
                (op, expr.clone(), String::new(), true)
            } else {
                let val = parse_static_expected(arg)?;
                let op = if negated { Op::NotEquals } else { Op::Equals };
                (op, expr.clone(), val, false)
            }
        }
        ("include" | "includes" | "contain" | "contains" | "toContain", Some(args_str)) => {
            let arg = single_arg(args_str)?;
            let val = parse_static_expected(arg)?;
            let op = if negated { Op::NotContains } else { Op::Contains };
            (op, expr.clone(), val, false)
        }
        ("match" | "toMatch", Some(args_str)) => {
            if negated {
                return None;
            }
            let arg = single_arg(args_str)?;
            let pattern = parse_regex_or_string(arg)?;
            (Op::Matches, expr.clone(), pattern, false)
        }
        ("below" | "lt" | "lessThan" | "toBeLessThan", Some(args_str)) => {
            if negated {
                return None;
            }
            let arg = single_arg(args_str)?;
            let num = parse_number_literal(arg)?;
            (Op::Less, expr.clone(), num, false)
        }
        ("above" | "gt" | "greaterThan" | "toBeGreaterThan", Some(args_str)) => {
            if negated {
                return None;
            }
            let arg = single_arg(args_str)?;
            let num = parse_number_literal(arg)?;
            (Op::Greater, expr.clone(), num, false)
        }
        ("exist", None) | ("toBeDefined", Some("")) => {
            let op = if negated { Op::NotExists } else { Op::Exists };
            (op, expr.clone(), String::new(), true)
        }
        ("undefined", None) | ("toBeUndefined", Some("")) => {
            let op = if negated { Op::Exists } else { Op::NotExists };
            (op, expr.clone(), String::new(), true)
        }
        ("null", None) | ("toBeNull", Some("")) => {
            let op = if negated { Op::NotEquals } else { Op::Equals };
            (op, expr.clone(), "null".to_string(), false)
        }
        ("true", None) => {
            let op = if negated { Op::NotEquals } else { Op::Equals };
            (op, expr.clone(), "true".to_string(), false)
        }
        ("false", None) => {
            let op = if negated { Op::NotEquals } else { Op::Equals };
            (op, expr.clone(), "false".to_string(), false)
        }
        ("a" | "an", Some(args_str)) => {
            if negated {
                return None;
            }
            let arg = single_arg(args_str)?;
            let ty = parse_string_literal(arg)?.to_ascii_lowercase();
            if !TYPES.contains(&ty.as_str()) {
                return None;
            }
            (Op::IsType, expr.clone(), ty, false)
        }
        ("property" | "ownProperty", Some(args_str)) => {
            let args = split_top_level(args_str, ',');
            let (key, val_opt) = match args.as_slice() {
                [k] => (parse_string_literal(k.trim())?, None),
                [k, v] => (parse_string_literal(k.trim())?, Some(parse_static_expected(v.trim())?)),
                _ => return None,
            };
            let sub_expr = match expr {
                ExprRef::Json { path, .. } => {
                    let mut next = path.clone();
                    next.push(PathSeg::Field(key));
                    ExprRef::Json {
                        path: next,
                        bare_res_body: false,
                    }
                }
                ExprRef::HeadersMap => ExprRef::Header(key),
                _ => return None,
            };
            match val_opt {
                None => {
                    let op = if negated { Op::NotExists } else { Op::Exists };
                    (op, sub_expr, String::new(), true)
                }
                Some(val) => {
                    let op = if negated { Op::NotEquals } else { Op::Equals };
                    (op, sub_expr, val, false)
                }
            }
        }
        ("oneOf", Some(args_str)) => {
            let arg = single_arg(args_str)?.trim();
            if !(arg.starts_with('[') && arg.ends_with(']')) {
                return None;
            }
            let items = split_top_level(&arg[1..arg.len() - 1], ',');
            let [only] = items.as_slice() else {
                return None;
            };
            let val = parse_static_expected(only.trim())?;
            let op = if negated { Op::NotEquals } else { Op::Equals };
            (op, expr.clone(), val, false)
        }
        _ => return None,
    };

    let (subject, target) = expr_to_assertion_subject(&target_expr, op)?;
    if op == Op::IsType && subject != Subject::Json {
        return None;
    }
    if allow_empty {
        validated_assertion_allow_empty_expected(true, subject, target, op, expected)
    } else {
        validated_assertion(true, subject, target, op, expected)
    }
}

fn single_arg(args_str: &str) -> Option<&str> {
    let args = split_top_level(args_str, ',');
    if args.len() == 1 { Some(args[0].trim()) } else { None }
}

fn is_chai_filler(word: &str) -> bool {
    matches!(
        word,
        "to" | "be"
            | "been"
            | "is"
            | "that"
            | "which"
            | "and"
            | "has"
            | "have"
            | "with"
            | "at"
            | "of"
            | "same"
            | "but"
            | "does"
            | "still"
            | "also"
            | "deep"
            | "own"
            | "nested"
            | "really"
            | "strictly"
    )
}

fn convert_assert_call(code: &str, bindings: &Bindings) -> Option<Assertion> {
    let (fn_name, open) = [
        "assert.strictEqual",
        "assert.equal",
        "assert.notStrictEqual",
        "assert.notEqual",
        "assert.isBelow",
        "assert.isAbove",
        "assert",
    ]
    .into_iter()
    .find_map(|prefix| find_call_after_prefix(code, &[prefix]).map(|open| (prefix, open)))?;
    let close = matching_delimiter(code, open, '(', ')')?;
    if !code[close + 1..].trim().is_empty() {
        return None;
    }
    let args = split_top_level(&code[open + 1..close], ',');
    match fn_name {
        "assert" => {
            if args.is_empty() || args.len() > 2 {
                return None;
            }
            if args.len() == 2 && parse_string_literal(args[1].trim()).is_none() {
                return None;
            }
            convert_boolean_expr(args[0].trim(), bindings)
        }
        "assert.equal" | "assert.strictEqual" | "assert.notEqual" | "assert.notStrictEqual" => {
            if args.len() < 2 || args.len() > 3 {
                return None;
            }
            let op = if fn_name.contains("not") {
                Op::NotEquals
            } else {
                Op::Equals
            };
            convert_binary_sides(args[0].trim(), args[1].trim(), op, bindings)
        }
        "assert.isBelow" | "assert.isAbove" => {
            if args.len() < 2 || args.len() > 3 {
                return None;
            }
            let op = if fn_name.ends_with("Below") {
                Op::Less
            } else {
                Op::Greater
            };
            let expr = resolve_expr(args[0].trim(), bindings)?;
            let num = parse_number_literal(args[1].trim())?;
            let (subject, target) = expr_to_assertion_subject(&expr, op)?;
            validated_assertion(true, subject, target, op, num)
        }
        _ => None,
    }
}

fn convert_boolean_expr(expr_str: &str, bindings: &Bindings) -> Option<Assertion> {
    let expr_str = strip_outer_parens(expr_str.trim());
    // Binary comparison operators at top level
    for (op_tok, op) in [
        ("===", Op::Equals),
        ("!==", Op::NotEquals),
        ("==", Op::Equals),
        ("!=", Op::NotEquals),
    ] {
        if let Some(pos) = find_top_level_str(expr_str, op_tok) {
            let lhs = expr_str[..pos].trim();
            let rhs = expr_str[pos + op_tok.len()..].trim();
            // Reject `indexOf(...) === -1` (handled below) or chained operators
            if let Some(a) = convert_index_of(lhs, rhs, op, bindings) {
                return Some(a);
            }
            return convert_binary_sides(lhs, rhs, op, bindings);
        }
    }
    for (op_tok, op, flipped_op) in [("<", Op::Less, Op::Greater), (">", Op::Greater, Op::Less)] {
        if let Some(pos) = find_top_level_str(expr_str, op_tok) {
            let before = expr_str[..pos].chars().next_back();
            let after = expr_str[pos + 1..].chars().next();
            if before == Some('=') || after == Some('=') || after == Some('>') {
                continue;
            }
            let lhs = expr_str[..pos].trim();
            let rhs = expr_str[pos + 1..].trim();
            if let Some(expr) = resolve_expr(lhs, bindings)
                && let Some(num) = parse_number_literal(rhs)
            {
                let (subject, target) = expr_to_assertion_subject(&expr, op)?;
                return validated_assertion(true, subject, target, op, num);
            }
            if let Some(expr) = resolve_expr(rhs, bindings)
                && let Some(num) = parse_number_literal(lhs)
            {
                let (subject, target) = expr_to_assertion_subject(&expr, flipped_op)?;
                return validated_assertion(true, subject, target, flipped_op, num);
            }
            return None;
        }
    }

    // Negated or direct `<expr>.includes("x")` / `.has("x")`
    let (negated, inner) = match expr_str.strip_prefix('!') {
        Some(rest) if !rest.starts_with('=') => (true, strip_outer_parens(rest.trim())),
        _ => (false, expr_str),
    };
    if let Some((base_str, arg_str)) = split_method_call(inner, &["includes", "has"])
        && let Some(expr) = resolve_expr(base_str, bindings)
        && let Some(val) = parse_static_expected(arg_str)
    {
        let op = if negated { Op::NotContains } else { Op::Contains };
        let (subject, target) = expr_to_assertion_subject(&expr, op)?;
        return validated_assertion(true, subject, target, op, val);
    }
    None
}

fn convert_index_of(lhs: &str, rhs: &str, eq_op: Op, bindings: &Bindings) -> Option<Assertion> {
    let ((base_str, arg_str), neg_one_is_not_found) = if let Some(call) = split_method_call(lhs, &["indexOf"]) {
        (call, rhs.trim() == "-1")
    } else {
        let call = split_method_call(rhs, &["indexOf"])?;
        (call, lhs.trim() == "-1")
    };
    if !neg_one_is_not_found {
        return None;
    }
    let expr = resolve_expr(base_str, bindings)?;
    let val = parse_string_literal(arg_str.trim())?;
    let op = match eq_op {
        Op::Equals => Op::NotContains,
        Op::NotEquals => Op::Contains,
        _ => return None,
    };
    let (subject, target) = expr_to_assertion_subject(&expr, op)?;
    validated_assertion(true, subject, target, op, val)
}

fn split_method_call<'a>(expr: &'a str, names: &[&str]) -> Option<(&'a str, &'a str)> {
    let open = expr.rfind('(')?;
    if matching_delimiter(expr, open, '(', ')')? != expr.len() - 1 {
        return None;
    }
    let before = expr[..open].trim_end();
    for name in names {
        if let Some(base) = before.strip_suffix(name) {
            let base = base.trim_end();
            if let Some(base) = base.strip_suffix('.') {
                let arg = expr[open + 1..expr.len() - 1].trim();
                return Some((base.trim(), arg));
            }
        }
    }
    None
}

fn convert_binary_sides(lhs: &str, rhs: &str, op: Op, bindings: &Bindings) -> Option<Assertion> {
    if let Some(expr) = resolve_expr(lhs, bindings) {
        if rhs == "undefined" {
            let exist_op = if op == Op::Equals { Op::NotExists } else { Op::Exists };
            let (subject, target) = expr_to_assertion_subject(&expr, exist_op)?;
            return validated_assertion_allow_empty_expected(true, subject, target, exist_op, String::new());
        }
        if let Some(expected) = parse_static_expected(rhs) {
            let (subject, target) = expr_to_assertion_subject(&expr, op)?;
            return validated_assertion(true, subject, target, op, expected);
        }
    }
    if let Some(expr) = resolve_expr(rhs, bindings) {
        if lhs == "undefined" {
            let exist_op = if op == Op::Equals { Op::NotExists } else { Op::Exists };
            let (subject, target) = expr_to_assertion_subject(&expr, exist_op)?;
            return validated_assertion_allow_empty_expected(true, subject, target, exist_op, String::new());
        }
        if let Some(expected) = parse_static_expected(lhs) {
            let (subject, target) = expr_to_assertion_subject(&expr, op)?;
            return validated_assertion(true, subject, target, op, expected);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Response expression resolver (`pm.response.*`, `res.*`, `insomnia.*`, aliases)
// ---------------------------------------------------------------------------

fn resolve_expr(raw: &str, bindings: &Bindings) -> Option<ExprRef> {
    let trimmed = strip_outer_parens(raw.trim());
    if trimmed.is_empty() {
        return None;
    }

    // `JSON.parse(inner)`
    if let Some(open) = find_call_after_prefix(trimmed, &["JSON.parse"]) {
        let close = matching_delimiter(trimmed, open, '(', ')')?;
        let inner = resolve_expr(&trimmed[open + 1..close], bindings)?;
        let base = match inner {
            ExprRef::BodyText => ExprRef::Json {
                path: Vec::new(),
                bare_res_body: false,
            },
            ExprRef::Json {
                path,
                bare_res_body: true,
            } if path.is_empty() => ExprRef::Json {
                path: Vec::new(),
                bare_res_body: false,
            },
            _ => return None,
        };
        return parse_postfix(base, &trimmed[close + 1..]);
    }

    let call_roots: [CallRoot; 5] = [
        ("pm.response.headers.get", |arg| {
            parse_string_literal(arg.trim()).map(ExprRef::Header)
        }),
        ("insomnia.response.headers.get", |arg| {
            parse_string_literal(arg.trim()).map(ExprRef::Header)
        }),
        ("res.getHeader", |arg| {
            parse_string_literal(arg.trim()).map(ExprRef::Header)
        }),
        ("postman.getResponseHeader", |arg| {
            parse_string_literal(arg.trim()).map(ExprRef::Header)
        }),
        ("pm.response.json", |arg| {
            arg.trim().is_empty().then_some(ExprRef::Json {
                path: Vec::new(),
                bare_res_body: false,
            })
        }),
    ];
    for (prefix, ctor) in call_roots {
        if let Some(open) = find_call_prefix_at_start(trimmed, prefix) {
            let close = matching_delimiter(trimmed, open, '(', ')')?;
            if let Some(base) = ctor(&trimmed[open + 1..close]) {
                return parse_postfix(base, &trimmed[close + 1..]);
            }
        }
    }

    let more_call_roots: [CallRoot; 8] = [
        ("insomnia.response.json", |arg| {
            arg.trim().is_empty().then_some(ExprRef::Json {
                path: Vec::new(),
                bare_res_body: false,
            })
        }),
        ("pm.response.text", |arg| {
            arg.trim().is_empty().then_some(ExprRef::BodyText)
        }),
        ("insomnia.response.text", |arg| {
            arg.trim().is_empty().then_some(ExprRef::BodyText)
        }),
        ("res.getBody", |arg| {
            arg.trim().is_empty().then_some(ExprRef::Json {
                path: Vec::new(),
                bare_res_body: true,
            })
        }),
        ("res.getStatus", |arg| arg.trim().is_empty().then_some(ExprRef::Status)),
        ("res.getResponseTime", |arg| {
            arg.trim().is_empty().then_some(ExprRef::Latency)
        }),
        ("res.getHeaders", |arg| {
            arg.trim().is_empty().then_some(ExprRef::HeadersMap)
        }),
        ("pm.response.size().body", |_| Some(ExprRef::Size)),
    ];
    for (prefix, ctor) in more_call_roots {
        if prefix == "pm.response.size().body" {
            if let Some(rest) = strip_member_prefix(trimmed, "pm.response.size().body") {
                return parse_postfix(ExprRef::Size, rest);
            }
            if let Some(rest) = strip_member_prefix(trimmed, "pm.response.size().total") {
                return parse_postfix(ExprRef::Size, rest);
            }
            continue;
        }
        if let Some(open) = find_call_prefix_at_start(trimmed, prefix) {
            let close = matching_delimiter(trimmed, open, '(', ')')?;
            if let Some(base) = ctor(&trimmed[open + 1..close]) {
                return parse_postfix(base, &trimmed[close + 1..]);
            }
        }
    }

    let prop_roots: [(&str, ExprRef); 13] = [
        ("pm.response.code", ExprRef::Status),
        ("insomnia.response.code", ExprRef::Status),
        ("pm.response.responseTime", ExprRef::Latency),
        ("insomnia.response.responseTime", ExprRef::Latency),
        ("pm.response.responseSize", ExprRef::Size),
        ("pm.response.headers", ExprRef::HeadersMap),
        ("insomnia.response.headers", ExprRef::HeadersMap),
        ("res.status", ExprRef::Status),
        ("res.responseTime", ExprRef::Latency),
        (
            "res.body",
            ExprRef::Json {
                path: Vec::new(),
                bare_res_body: true,
            },
        ),
        ("res.headers", ExprRef::HeadersMap),
        ("responseCode.code", ExprRef::Status),
        ("responseHeaders", ExprRef::HeadersMap),
    ];
    for (prefix, base) in prop_roots {
        if let Some(rest) = strip_member_prefix(trimmed, prefix) {
            return parse_postfix(base, rest);
        }
    }

    for (prefix, base) in [("responseTime", ExprRef::Latency), ("responseBody", ExprRef::BodyText)] {
        if let Some(rest) = strip_member_prefix(trimmed, prefix) {
            return parse_postfix(base, rest);
        }
    }

    // Bound local variable alias
    let end = trimmed
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
        .unwrap_or(trimmed.len());
    let head = &trimmed[..end];
    if !is_ascii_ident(head) {
        return None;
    }
    let rest = trimmed[end..].trim_start();
    // Do not treat an unknown function call `foo(...)` as a bound variable.
    if rest.starts_with('(') {
        return None;
    }
    let base = bindings.get(head)?.clone();
    parse_postfix(base, rest)
}

fn find_call_prefix_at_start(expr: &str, prefix: &str) -> Option<usize> {
    let tail = expr.strip_prefix(prefix)?;
    let ws = tail.len() - tail.trim_start().len();
    tail.trim_start().starts_with('(').then_some(prefix.len() + ws)
}

fn find_call_after_prefix(expr: &str, prefixes: &[&str]) -> Option<usize> {
    for prefix in prefixes {
        if let Some(open) = find_call_prefix_at_start(expr, prefix) {
            return Some(open);
        }
    }
    None
}

fn strip_member_prefix<'a>(expr: &'a str, prefix: &str) -> Option<&'a str> {
    let tail = expr.strip_prefix(prefix)?;
    let trimmed = tail.trim_start();
    if trimmed.is_empty()
        || trimmed.starts_with('.')
        || trimmed.starts_with("?.")
        || trimmed.starts_with('[')
        || trimmed.starts_with("?.[")
    {
        Some(trimmed)
    } else {
        None
    }
}

fn strip_ident_prefix<'a>(expr: &'a str, ident: &str) -> Option<&'a str> {
    let tail = expr.strip_prefix(ident)?;
    let next = tail.chars().next();
    match next {
        Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '$' => None,
        _ => Some(tail),
    }
}

fn parse_postfix(mut expr: ExprRef, mut rest: &str) -> Option<ExprRef> {
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return Some(expr);
        }
        if let Some(after_opt) = rest.strip_prefix("?.") {
            rest = after_opt;
        } else if let Some(after_dot) = rest.strip_prefix('.') {
            rest = after_dot;
        } else if rest.starts_with('[') {
            let close = matching_delimiter(rest, 0, '[', ']')?;
            let inner = rest[1..close].trim();
            expr = apply_bracket(&expr, inner)?;
            rest = &rest[close + 1..];
            continue;
        } else {
            return None;
        }

        // After `.` or `?.`, could be `[...]` (for `?.[0]`) or `.ident`
        rest = rest.trim_start();
        if rest.starts_with('[') {
            let close = matching_delimiter(rest, 0, '[', ']')?;
            let inner = rest[1..close].trim();
            expr = apply_bracket(&expr, inner)?;
            rest = &rest[close + 1..];
            continue;
        }

        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
            .unwrap_or(rest.len());
        let prop = &rest[..end];
        if prop.is_empty() {
            return None;
        }
        let after_prop = rest[end..].trim_start();
        if after_prop.starts_with('(') {
            if matches!(expr, ExprRef::HeadersMap) && prop == "get" {
                let close = matching_delimiter(after_prop, 0, '(', ')')?;
                let header = parse_string_literal(after_prop[1..close].trim())?;
                expr = ExprRef::Header(header);
                rest = &after_prop[close + 1..];
                continue;
            }
            // Reject method calls such as `.toLowerCase()`, `.split()`, `.trim()`
            return None;
        }
        expr = apply_field(&expr, prop)?;
        rest = after_prop;
    }
}

fn apply_field(expr: &ExprRef, field: &str) -> Option<ExprRef> {
    match expr {
        ExprRef::HeadersMap => Some(ExprRef::Header(field.to_string())),
        ExprRef::Json { path, .. } => {
            if field == "length" {
                return None;
            }
            let mut next = path.clone();
            next.push(PathSeg::Field(field.to_string()));
            Some(ExprRef::Json {
                path: next,
                bare_res_body: false,
            })
        }
        _ => None,
    }
}

fn apply_bracket(expr: &ExprRef, inner: &str) -> Option<ExprRef> {
    match expr {
        ExprRef::HeadersMap => parse_string_literal(inner).map(ExprRef::Header),
        ExprRef::Json { path, .. } => {
            let seg = if let Ok(index) = inner.parse::<usize>() {
                PathSeg::Index(index)
            } else {
                PathSeg::Field(parse_string_literal(inner)?)
            };
            let mut next = path.clone();
            next.push(seg);
            Some(ExprRef::Json {
                path: next,
                bare_res_body: false,
            })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Chai dot-chain parser & literals
// ---------------------------------------------------------------------------

struct ChainSeg<'a> {
    name: &'a str,
    args: Option<&'a str>,
}

fn parse_dot_chain(raw: &str) -> Option<Vec<ChainSeg<'_>>> {
    let mut rest = raw.trim();
    let mut out = Vec::new();
    while !rest.is_empty() {
        rest = rest.strip_prefix('.')?.trim_start();
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
            .unwrap_or(rest.len());
        let name = &rest[..end];
        if name.is_empty() {
            return None;
        }
        rest = rest[end..].trim_start();
        if rest.starts_with('(') {
            let close = matching_delimiter(rest, 0, '(', ')')?;
            let args = &rest[1..close];
            out.push(ChainSeg { name, args: Some(args) });
            rest = rest[close + 1..].trim_start();
        } else {
            out.push(ChainSeg { name, args: None });
        }
    }
    (!out.is_empty()).then_some(out)
}

fn parse_static_expected(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(s) = parse_string_literal(trimmed) {
        return Some(s);
    }
    if let Some(n) = parse_number_literal(trimmed) {
        return Some(n);
    }
    if matches!(trimmed, "true" | "false" | "null") {
        return Some(trimmed.to_string());
    }
    if (trimmed.starts_with('[') || trimmed.starts_with('{')) && serde_json::from_str::<Value>(trimmed).is_ok() {
        return Some(trimmed.to_string());
    }
    None
}

fn parse_number_literal(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let check = trimmed.strip_prefix('-').unwrap_or(trimmed);
    let first = check.chars().next()?;
    if !first.is_ascii_digit() {
        return None;
    }
    trimmed
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .map(|_| trimmed.to_string())
}

fn parse_string_literal(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let quote = trimmed.chars().next()?;
    if !matches!(quote, '\'' | '"' | '`') || trimmed.len() < 2 || !trimmed.ends_with(quote) {
        return None;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(ch) = chars.next() {
        if quote == '`' && ch == '$' && chars.peek() == Some(&'{') {
            // Template interpolation `${...}` is dynamic.
            return None;
        }
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let esc = chars.next()?;
        match esc {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '\\' => out.push('\\'),
            '\'' => out.push('\''),
            '"' => out.push('"'),
            '`' => out.push('`'),
            '/' => out.push('/'),
            '0' => out.push('\0'),
            'x' => {
                let hi = chars.next()?;
                let lo = chars.next()?;
                let code = u8::from_str_radix(&format!("{hi}{lo}"), 16).ok()?;
                out.push(code as char);
            }
            'u' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    let mut hex = String::new();
                    for c in chars.by_ref() {
                        if c == '}' {
                            break;
                        }
                        hex.push(c);
                    }
                    let cp = u32::from_str_radix(&hex, 16).ok()?;
                    out.push(char::from_u32(cp)?);
                } else {
                    let mut hex = String::with_capacity(4);
                    for _ in 0..4 {
                        hex.push(chars.next()?);
                    }
                    let cp = u32::from_str_radix(&hex, 16).ok()?;
                    out.push(char::from_u32(cp)?);
                }
            }
            other => out.push(other),
        }
    }
    Some(out)
}

fn parse_regex_or_string(raw: &str) -> Option<String> {
    if let Some(re) = parse_regex_literal(raw) {
        return Some(re);
    }
    let s = parse_string_literal(raw)?;
    regex::Regex::new(&s).ok().map(|_| s)
}

fn parse_regex_literal(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let mut in_class = false;
    let mut close_slash = None;
    let mut i = 1usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            i += 2;
            continue;
        }
        if b == b'[' {
            in_class = true;
        } else if b == b']' && in_class {
            in_class = false;
        } else if b == b'/' && !in_class {
            close_slash = Some(i);
            break;
        }
        i += 1;
    }
    let close = close_slash?;
    let body = &trimmed[1..close];
    if body.contains('\n') {
        return None;
    }
    let flags = &trimmed[close + 1..];
    let mut inline_flags = String::new();
    for ch in flags.chars() {
        match ch {
            'i' | 'm' | 's' => {
                if !inline_flags.contains(ch) {
                    inline_flags.push(ch);
                }
            }
            'g' | 'u' | 'd' | 'v' => {}
            _ => return None,
        }
    }
    let unescaped_slashes = unescape_regex_slashes(body);
    let pattern = if inline_flags.is_empty() {
        unescaped_slashes
    } else {
        format!("(?{inline_flags}){unescaped_slashes}")
    };
    regex::Regex::new(&pattern).ok().map(|_| pattern)
}

fn unescape_regex_slashes(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek() == Some(&'/') {
            out.push('/');
            chars.next();
        } else if ch == '\\'
            && let Some(next) = chars.next()
        {
            out.push('\\');
            out.push(next);
        } else {
            out.push(ch);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Comment/quote/regex/bracket-aware JS statement splitter & helpers
// ---------------------------------------------------------------------------

struct StmtSlice {
    raw: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LexState {
    Normal,
    SingleQuote,
    DoubleQuote,
    Template,
    LineComment,
    BlockComment,
    Regex { in_class: bool },
}

fn can_regex_precede(last_sig: Option<char>, last_word: &str) -> bool {
    if matches!(
        last_word,
        "return" | "typeof" | "void" | "delete" | "await" | "yield" | "throw" | "case" | "in" | "of" | "else"
    ) {
        return true;
    }
    match last_sig {
        None => true,
        Some(c) => matches!(
            c,
            '(' | '['
                | '{'
                | ','
                | ';'
                | ':'
                | '='
                | '!'
                | '&'
                | '|'
                | '?'
                | '+'
                | '-'
                | '*'
                | '%'
                | '~'
                | '^'
                | '<'
                | '>'
        ),
    }
}

fn next_significant_bytes(bytes: &[u8], mut i: usize) -> Option<&[u8]> {
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        return Some(&bytes[i..]);
    }
    None
}

fn next_continues_expr(bytes: &[u8], after_nl: usize) -> bool {
    let Some(rest) = next_significant_bytes(bytes, after_nl) else {
        return false;
    };
    let first = rest[0];
    if matches!(
        first,
        b'.' | b')'
            | b']'
            | b'}'
            | b','
            | b';'
            | b'?'
            | b':'
            | b'&'
            | b'|'
            | b'+'
            | b'-'
            | b'*'
            | b'%'
            | b'<'
            | b'>'
            | b'='
    ) {
        return true;
    }
    for kw in ["else", "catch", "finally"] {
        if rest.starts_with(kw.as_bytes())
            && rest
                .get(kw.len())
                .is_none_or(|b| !b.is_ascii_alphanumeric() && *b != b'_' && *b != b'$')
        {
            return true;
        }
    }
    false
}

fn prev_continues_expr(last_sig: Option<char>, last_word: &str) -> bool {
    if matches!(
        last_word,
        "return" | "const" | "let" | "var" | "await" | "typeof" | "throw" | "else"
    ) {
        return true;
    }
    matches!(
        last_sig,
        Some('.' | ',' | '+' | '-' | '*' | '%' | '=' | '(' | '[' | '{' | '?' | ':' | '&' | '|' | '!' | '<' | '>')
    )
}

fn split_statements(src: &str) -> Vec<StmtSlice> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut state = LexState::Normal;
    let mut template_expr_depths: Vec<(usize, usize, usize)> = Vec::new();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut stmt_start: Option<usize> = None;
    let mut last_sig: Option<char> = None;
    let mut current_word = String::new();
    let mut last_word = String::new();
    let mut i = 0usize;

    let push_stmt = |out: &mut Vec<StmtSlice>, start_opt: &mut Option<usize>, end: usize| {
        if let Some(start) = start_opt.take() {
            let slice = src[start..end].trim();
            if !slice.is_empty() {
                out.push(StmtSlice { raw: slice.to_string() });
            }
        }
    };

    while i < bytes.len() {
        let b = bytes[i];
        match state {
            LexState::Normal => {
                if b == b' ' || b == b'\t' || b == b'\r' {
                    if !current_word.is_empty() {
                        last_word = std::mem::take(&mut current_word);
                    }
                    i += 1;
                    continue;
                }
                if b == b'\n' {
                    if !current_word.is_empty() {
                        last_word = std::mem::take(&mut current_word);
                    }
                    if paren == 0
                        && bracket == 0
                        && brace == 0
                        && stmt_start.is_some()
                        && !prev_continues_expr(last_sig, &last_word)
                        && !next_continues_expr(bytes, i + 1)
                    {
                        push_stmt(&mut out, &mut stmt_start, i);
                        last_sig = None;
                        last_word.clear();
                    }
                    i += 1;
                    continue;
                }
                // Comments
                if i + 1 < bytes.len() && b == b'/' && bytes[i + 1] == b'/' {
                    if !current_word.is_empty() {
                        last_word = std::mem::take(&mut current_word);
                    }
                    state = LexState::LineComment;
                    i += 2;
                    continue;
                }
                if i + 1 < bytes.len() && b == b'/' && bytes[i + 1] == b'*' {
                    if !current_word.is_empty() {
                        last_word = std::mem::take(&mut current_word);
                    }
                    state = LexState::BlockComment;
                    i += 2;
                    continue;
                }

                if stmt_start.is_none() {
                    stmt_start = Some(i);
                }

                if b == b'\'' {
                    last_word.clear();
                    current_word.clear();
                    last_sig = Some('\'');
                    state = LexState::SingleQuote;
                    i += 1;
                    continue;
                }
                if b == b'"' {
                    last_word.clear();
                    current_word.clear();
                    last_sig = Some('"');
                    state = LexState::DoubleQuote;
                    i += 1;
                    continue;
                }
                if b == b'`' {
                    last_word.clear();
                    current_word.clear();
                    last_sig = Some('`');
                    state = LexState::Template;
                    i += 1;
                    continue;
                }
                if b == b'/' {
                    let word_for_check = if current_word.is_empty() {
                        last_word.as_str()
                    } else {
                        current_word.as_str()
                    };
                    if can_regex_precede(last_sig, word_for_check) {
                        last_word.clear();
                        current_word.clear();
                        state = LexState::Regex { in_class: false };
                        i += 1;
                        continue;
                    }
                }

                if b == b'(' {
                    paren += 1;
                } else if b == b')' {
                    paren = paren.saturating_sub(1);
                } else if b == b'[' {
                    bracket += 1;
                } else if b == b']' {
                    bracket = bracket.saturating_sub(1);
                } else if b == b'{' {
                    brace += 1;
                } else if b == b'}' {
                    if let Some(&(p, br, bc)) = template_expr_depths.last()
                        && (paren, bracket, brace) == (p, br, bc + 1)
                    {
                        template_expr_depths.pop();
                        brace = bc;
                        state = LexState::Template;
                        last_sig = Some('`');
                        i += 1;
                        continue;
                    }
                    brace = brace.saturating_sub(1);
                }

                if b == b';' && paren == 0 && bracket == 0 && brace == 0 {
                    push_stmt(&mut out, &mut stmt_start, i + 1);
                    last_sig = None;
                    last_word.clear();
                    current_word.clear();
                    i += 1;
                    continue;
                }

                if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' {
                    current_word.push(b as char);
                } else {
                    if !current_word.is_empty() {
                        last_word = std::mem::take(&mut current_word);
                    } else {
                        last_word.clear();
                    }
                }
                last_sig = Some(b as char);
                i += 1;
            }
            LexState::SingleQuote => {
                if b == b'\\' {
                    i += 2;
                } else if b == b'\'' {
                    state = LexState::Normal;
                    i += 1;
                } else {
                    i += 1;
                }
            }
            LexState::DoubleQuote => {
                if b == b'\\' {
                    i += 2;
                } else if b == b'"' {
                    state = LexState::Normal;
                    i += 1;
                } else {
                    i += 1;
                }
            }
            LexState::Template => {
                if b == b'\\' {
                    i += 2;
                } else if i + 1 < bytes.len() && b == b'$' && bytes[i + 1] == b'{' {
                    template_expr_depths.push((paren, bracket, brace));
                    brace += 1;
                    state = LexState::Normal;
                    last_sig = Some('{');
                    i += 2;
                } else if b == b'`' {
                    state = LexState::Normal;
                    i += 1;
                } else {
                    i += 1;
                }
            }
            LexState::LineComment => {
                if b == b'\n' {
                    state = LexState::Normal;
                    if paren == 0
                        && bracket == 0
                        && brace == 0
                        && stmt_start.is_some()
                        && !prev_continues_expr(last_sig, &last_word)
                        && !next_continues_expr(bytes, i + 1)
                    {
                        push_stmt(&mut out, &mut stmt_start, i);
                        last_sig = None;
                        last_word.clear();
                    }
                }
                i += 1;
            }
            LexState::BlockComment => {
                if i + 1 < bytes.len() && b == b'*' && bytes[i + 1] == b'/' {
                    state = LexState::Normal;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            LexState::Regex { in_class } => {
                if b == b'\\' {
                    i += 2;
                } else if b == b'[' {
                    state = LexState::Regex { in_class: true };
                    i += 1;
                } else if b == b']' && in_class {
                    state = LexState::Regex { in_class: false };
                    i += 1;
                } else if b == b'/' && !in_class {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    last_sig = Some('/');
                    state = LexState::Normal;
                } else if b == b'\n' {
                    state = LexState::Normal;
                    i += 1;
                } else {
                    i += 1;
                }
            }
        }
    }

    push_stmt(&mut out, &mut stmt_start, src.len());
    out
}

fn strip_js_comments(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut state = LexState::Normal;
    let mut last_sig: Option<char> = None;
    let mut current_word = String::new();
    let mut last_word = String::new();
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        match state {
            LexState::Normal => {
                if i + 1 < bytes.len() && b == b'/' && bytes[i + 1] == b'/' {
                    state = LexState::LineComment;
                    i += 2;
                    continue;
                }
                if i + 1 < bytes.len() && b == b'/' && bytes[i + 1] == b'*' {
                    state = LexState::BlockComment;
                    out.push(b' ');
                    i += 2;
                    continue;
                }
                if b == b'\'' {
                    state = LexState::SingleQuote;
                    last_sig = Some('\'');
                    last_word.clear();
                    current_word.clear();
                } else if b == b'"' {
                    state = LexState::DoubleQuote;
                    last_sig = Some('"');
                    last_word.clear();
                    current_word.clear();
                } else if b == b'`' {
                    state = LexState::Template;
                    last_sig = Some('`');
                    last_word.clear();
                    current_word.clear();
                } else if b == b'/' {
                    let w = if current_word.is_empty() {
                        last_word.as_str()
                    } else {
                        current_word.as_str()
                    };
                    if can_regex_precede(last_sig, w) {
                        state = LexState::Regex { in_class: false };
                        last_word.clear();
                        current_word.clear();
                    } else {
                        last_sig = Some('/');
                    }
                } else if b.is_ascii_whitespace() {
                    if !current_word.is_empty() {
                        last_word = std::mem::take(&mut current_word);
                    }
                } else if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' {
                    current_word.push(b as char);
                    last_sig = Some(b as char);
                } else {
                    if !current_word.is_empty() {
                        last_word = std::mem::take(&mut current_word);
                    } else {
                        last_word.clear();
                    }
                    last_sig = Some(b as char);
                }
                out.push(b);
                i += 1;
            }
            LexState::SingleQuote => {
                out.push(b);
                if b == b'\\' && i + 1 < bytes.len() {
                    out.push(bytes[i + 1]);
                    i += 2;
                } else {
                    if b == b'\'' {
                        state = LexState::Normal;
                    }
                    i += 1;
                }
            }
            LexState::DoubleQuote => {
                out.push(b);
                if b == b'\\' && i + 1 < bytes.len() {
                    out.push(bytes[i + 1]);
                    i += 2;
                } else {
                    if b == b'"' {
                        state = LexState::Normal;
                    }
                    i += 1;
                }
            }
            LexState::Template => {
                out.push(b);
                if b == b'\\' && i + 1 < bytes.len() {
                    out.push(bytes[i + 1]);
                    i += 2;
                } else {
                    if b == b'`' {
                        state = LexState::Normal;
                    }
                    i += 1;
                }
            }
            LexState::LineComment => {
                if b == b'\n' {
                    out.push(b);
                    state = LexState::Normal;
                }
                i += 1;
            }
            LexState::BlockComment => {
                if i + 1 < bytes.len() && b == b'*' && bytes[i + 1] == b'/' {
                    state = LexState::Normal;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            LexState::Regex { in_class } => {
                out.push(b);
                if b == b'\\' && i + 1 < bytes.len() {
                    out.push(bytes[i + 1]);
                    i += 2;
                } else if b == b'[' {
                    state = LexState::Regex { in_class: true };
                    i += 1;
                } else if b == b']' && in_class {
                    state = LexState::Regex { in_class: false };
                    i += 1;
                } else if b == b'/' && !in_class {
                    state = LexState::Normal;
                    last_sig = Some('/');
                    i += 1;
                } else {
                    i += 1;
                }
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn scan_normal_step(bytes: &[u8], i: usize, state: &mut LexState, last_sig: &mut Option<char>) -> Option<usize> {
    let b = bytes[i];
    match *state {
        LexState::Normal => {
            if i + 1 < bytes.len() && b == b'/' && bytes[i + 1] == b'/' {
                *state = LexState::LineComment;
                return Some(i + 2);
            }
            if i + 1 < bytes.len() && b == b'/' && bytes[i + 1] == b'*' {
                *state = LexState::BlockComment;
                return Some(i + 2);
            }
            if b == b'\'' {
                *last_sig = Some('\'');
                *state = LexState::SingleQuote;
                return Some(i + 1);
            }
            if b == b'"' {
                *last_sig = Some('"');
                *state = LexState::DoubleQuote;
                return Some(i + 1);
            }
            if b == b'`' {
                *last_sig = Some('`');
                *state = LexState::Template;
                return Some(i + 1);
            }
            if b == b'/' && can_regex_precede(*last_sig, "") {
                *state = LexState::Regex { in_class: false };
                return Some(i + 1);
            }
            if !b.is_ascii_whitespace() {
                *last_sig = Some(b as char);
            }
            None
        }
        LexState::SingleQuote => {
            if b == b'\\' {
                Some((i + 2).min(bytes.len()))
            } else {
                if b == b'\'' {
                    *state = LexState::Normal;
                }
                Some(i + 1)
            }
        }
        LexState::DoubleQuote => {
            if b == b'\\' {
                Some((i + 2).min(bytes.len()))
            } else {
                if b == b'"' {
                    *state = LexState::Normal;
                }
                Some(i + 1)
            }
        }
        LexState::Template => {
            if b == b'\\' {
                Some((i + 2).min(bytes.len()))
            } else {
                if b == b'`' {
                    *state = LexState::Normal;
                }
                Some(i + 1)
            }
        }
        LexState::LineComment => {
            if b == b'\n' {
                *state = LexState::Normal;
            }
            Some(i + 1)
        }
        LexState::BlockComment => {
            if i + 1 < bytes.len() && b == b'*' && bytes[i + 1] == b'/' {
                *state = LexState::Normal;
                Some(i + 2)
            } else {
                Some(i + 1)
            }
        }
        LexState::Regex { in_class } => {
            if b == b'\\' {
                Some((i + 2).min(bytes.len()))
            } else if b == b'[' {
                *state = LexState::Regex { in_class: true };
                Some(i + 1)
            } else if b == b']' && in_class {
                *state = LexState::Regex { in_class: false };
                Some(i + 1)
            } else if b == b'/' && !in_class {
                *last_sig = Some('/');
                *state = LexState::Normal;
                Some(i + 1)
            } else {
                Some(i + 1)
            }
        }
    }
}

fn matching_delimiter(text: &str, open_idx: usize, open_ch: char, close_ch: char) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(open_idx) != Some(&(open_ch as u8)) {
        return None;
    }
    let mut depth = 0usize;
    let mut state = LexState::Normal;
    let mut last_sig = None;
    let mut i = open_idx;
    while i < bytes.len() {
        if let Some(next_i) = scan_normal_step(bytes, i, &mut state, &mut last_sig) {
            i = next_i;
            continue;
        }
        let b = bytes[i];
        if b == open_ch as u8 {
            depth += 1;
        } else if b == close_ch as u8 {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn split_top_level(text: &str, sep: char) -> Vec<&str> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut state = LexState::Normal;
    let mut last_sig = None;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if let Some(next_i) = scan_normal_step(bytes, i, &mut state, &mut last_sig) {
            i = next_i;
            continue;
        }
        let b = bytes[i];
        match b {
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b'{' => brace += 1,
            b'}' => brace = brace.saturating_sub(1),
            _ if b == sep as u8 && paren == 0 && bracket == 0 && brace == 0 => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let tail = &text[start..];
    if !tail.trim().is_empty() || !out.is_empty() {
        // Ignore a trailing comma after the last argument in JS calls like `[a, b,]`.
        if !tail.trim().is_empty() {
            out.push(tail);
        }
    }
    out
}

fn find_outside_strings(text: &str, ch: char) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut state = LexState::Normal;
    let mut last_sig = None;
    let mut i = 0usize;
    while i < bytes.len() {
        if let Some(next_i) = scan_normal_step(bytes, i, &mut state, &mut last_sig) {
            i = next_i;
            continue;
        }
        if bytes[i] == ch as u8 {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_top_level_str(text: &str, needle: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut state = LexState::Normal;
    let mut last_sig = None;
    let mut i = 0usize;
    while i < bytes.len() {
        if state == LexState::Normal && paren == 0 && bracket == 0 && brace == 0 && text[i..].starts_with(needle) {
            return Some(i);
        }
        if let Some(next_i) = scan_normal_step(bytes, i, &mut state, &mut last_sig) {
            i = next_i;
            continue;
        }
        match bytes[i] {
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b'{' => brace += 1,
            b'}' => brace = brace.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    None
}

fn find_top_level_assignment(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut state = LexState::Normal;
    let mut last_sig = None;
    let mut i = 0usize;
    while i < bytes.len() {
        if let Some(next_i) = scan_normal_step(bytes, i, &mut state, &mut last_sig) {
            i = next_i;
            continue;
        }
        match bytes[i] {
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b'{' => brace += 1,
            b'}' => brace = brace.saturating_sub(1),
            b'=' if paren == 0
                && bracket == 0
                && brace == 0
                && bytes.get(i + 1) != Some(&b'=')
                && bytes.get(i + 1) != Some(&b'>') =>
            {
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn strip_outer_parens(mut text: &str) -> &str {
    loop {
        text = text.trim();
        if text.starts_with('(')
            && let Some(close) = matching_delimiter(text, 0, '(', ')')
            && close == text.len() - 1
        {
            text = &text[1..close];
            continue;
        }
        return text;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assertions::{check, extract_all};
    use crate::response::HttpResponse;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    fn assert_row(subject: Subject, target: &str, op: Op, expected: &str) -> Assertion {
        Assertion {
            enabled: true,
            subject,
            target: target.into(),
            op,
            expected: expected.into(),
            imported: false,
        }
    }

    fn extract_row(name: &str, subject: Subject, target: &str) -> Extraction {
        Extraction {
            enabled: true,
            name: name.into(),
            subject,
            target: target.into(),
        }
    }

    #[test]
    fn postman_script_converts_assertions_and_extractions_cleanly() {
        let script = r#"
// Verify login response
const json = pm.response.json();
const { user_id: userId, role } = json.data;

pm.test("status and timing", function () {
    pm.response.to.have.status(200);
    pm.expect(pm.response.code).to.eql(200);
    pm.expect(pm.response.responseTime).to.be.below(500);
    pm.response.to.have.header("Content-Type");
    pm.response.to.have.header("Content-Type", "application/json");
    pm.expect(pm.response.headers.get("X-Trace")).to.include("tr-");
    pm.expect(pm.response.text()).to.include("token-123");
});

pm.test("body fields", () => {
    pm.expect(json.data.token).to.be.a("string");
    pm.expect(json['meta-info'][0].name).to.match(/^alice/i);
    pm.expect(json.data.deletedAt).to.be.null;
    pm.expect(json.error).to.be.undefined;
    pm.expect(json).to.have.property("ok", true);
    pm.expect(userId).to.eql(42);
    pm.expect(role).to.not.eql("guest");
});

pm.environment.set("token", json.data.token);
pm.collectionVariables.set("userId", userId);
pm.variables.set("statusCode", pm.response.code);
pm.globals.set("reqTrace", pm.response.headers.get("X-Trace"));
postman.setEnvironmentVariable("rawBody", pm.response.text());
"#;

        let conv = convert_script(ScriptPhase::Test, script);
        assert_eq!(conv.unconverted, "");
        assert_eq!(
            conv.assertions,
            vec![
                assert_row(Subject::Status, "", Op::Equals, "200"),
                assert_row(Subject::Status, "", Op::Equals, "200"),
                assert_row(Subject::Latency, "", Op::Less, "500"),
                assert_row(Subject::Header, "Content-Type", Op::Exists, ""),
                assert_row(Subject::Header, "Content-Type", Op::Equals, "application/json"),
                assert_row(Subject::Header, "X-Trace", Op::Contains, "tr-"),
                assert_row(Subject::Body, "", Op::Contains, "token-123"),
                assert_row(Subject::Json, "$.data.token", Op::IsType, "string"),
                assert_row(Subject::Json, "$['meta-info'][0].name", Op::Matches, "(?i)^alice"),
                assert_row(Subject::Json, "$.data.deletedAt", Op::Equals, "null"),
                assert_row(Subject::Json, "$.error", Op::NotExists, ""),
                assert_row(Subject::Json, "$.ok", Op::Equals, "true"),
                assert_row(Subject::Json, "$.data.user_id", Op::Equals, "42"),
                assert_row(Subject::Json, "$.data.role", Op::NotEquals, "guest"),
            ]
        );
        assert_eq!(
            conv.extractions,
            vec![
                extract_row("token", Subject::Json, "$.data.token"),
                extract_row("userId", Subject::Json, "$.data.user_id"),
                extract_row("statusCode", Subject::Status, ""),
                extract_row("reqTrace", Subject::Header, "X-Trace"),
                extract_row("rawBody", Subject::Body, ""),
            ]
        );

        // Converted assertions & extractions pass against an actual HttpResponse.
        let body = serde_json::json!({
            "ok": true,
            "data": {
                "token": "token-123",
                "user_id": 42,
                "role": "admin",
                "deletedAt": null
            },
            "meta-info": [{"name": "Alice"}]
        })
        .to_string()
        .into_bytes();
        let response = HttpResponse {
            status: Some(200),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("X-Trace".into(), "tr-99".into()),
            ],
            body,
            latency: Duration::from_millis(120),
            ..Default::default()
        };
        let parsed_json = response.body_json();
        for assertion in &conv.assertions {
            let checked = check(assertion, &response, parsed_json.as_ref()).expect("check ok");
            assert!(checked.passed, "failed assertion: {assertion:?}");
        }
        assert_eq!(
            extract_all(&conv.extractions, &response),
            vec![
                ("token".into(), "token-123".into()),
                ("userId".into(), "42".into()),
                ("statusCode".into(), "200".into()),
                ("reqTrace".into(), "tr-99".into()),
                ("rawBody".into(), String::from_utf8_lossy(&response.body).into_owned()),
            ]
        );
    }

    #[test]
    fn bruno_and_insomnia_scripts_convert_cleanly() {
        let bru_script = r#"
test("order created", () => {
    expect(res.getStatus()).to.equal(201);
    expect(res.status).to.be.above(199);
    expect(res.getResponseTime()).to.be.below(300);
    expect(res.body.id).to.exist;
    expect(res.getBody().items[0].sku).to.equal("A-1");
    expect(res.getHeader("x-request-id")).to.exist;
});
bru.setVar("orderId", res.body.id);
bru.setEnvVar("reqId", res.headers['x-request-id']);
"#;
        let conv = convert_script(ScriptPhase::Test, bru_script);
        assert_eq!(conv.unconverted, "");
        assert_eq!(
            conv.assertions,
            vec![
                assert_row(Subject::Status, "", Op::Equals, "201"),
                assert_row(Subject::Status, "", Op::Greater, "199"),
                assert_row(Subject::Latency, "", Op::Less, "300"),
                assert_row(Subject::Json, "$.id", Op::Exists, ""),
                assert_row(Subject::Json, "$.items[0].sku", Op::Equals, "A-1"),
                assert_row(Subject::Header, "x-request-id", Op::Exists, ""),
            ]
        );
        assert_eq!(
            conv.extractions,
            vec![
                extract_row("orderId", Subject::Json, "$.id"),
                extract_row("reqId", Subject::Header, "x-request-id"),
            ]
        );

        let insomnia_script = r#"
const data = insomnia.response.json();
insomnia.test("ok", async () => {
    insomnia.response.to.have.status(200);
    insomnia.expect(data.token).to.exist;
});
insomnia.environment.set("token", data.token);
insomnia.variables.set("etag", insomnia.response.headers.get("etag"));
"#;
        let conv = convert_script(ScriptPhase::PostResponse, insomnia_script);
        assert_eq!(conv.unconverted, "");
        assert_eq!(
            conv.assertions,
            vec![
                assert_row(Subject::Status, "", Op::Equals, "200"),
                assert_row(Subject::Json, "$.token", Op::Exists, ""),
            ]
        );
        assert_eq!(
            conv.extractions,
            vec![
                extract_row("token", Subject::Json, "$.token"),
                extract_row("etag", Subject::Header, "etag"),
            ]
        );
    }

    #[test]
    fn mixed_script_keeps_unconverted_statements_and_referenced_aliases() {
        let script = r#"
const json = pm.response.json();
pm.test("mixed", () => {
    pm.response.to.have.status(200);
    console.log("items:", json.items.length);
});
pm.environment.set("firstId", json.items[0].id);
pm.environment.set("computed", Date.now());
"#;
        let conv = convert_script(ScriptPhase::Test, script);
        assert_eq!(
            conv.assertions,
            vec![assert_row(Subject::Status, "", Op::Equals, "200")]
        );
        assert_eq!(
            conv.extractions,
            vec![extract_row("firstId", Subject::Json, "$.items[0].id")]
        );
        assert_eq!(
            conv.unconverted,
            "const json = pm.response.json();\nconsole.log(\"items:\", json.items.length);\npm.environment.set(\"computed\", Date.now());"
        );
    }

    #[test]
    fn unsafe_or_dynamic_patterns_stay_unconverted() {
        let script = r#"
const json = pm.response.json();
pm.test("empty wrapper", () => {});
pm.expect(json.items.length).to.eql(3);
pm.expect(pm.response.code).to.be.oneOf([200, 201]);
pm.expect(json.id).to.eql(expectedId);
bru.setVar("literal", 123);
pm.expect(json.name).to.match(/(?=foo)bar/);
"#;
        let conv = convert_script(ScriptPhase::Test, script);
        assert!(conv.assertions.is_empty(), "{:?}", conv.assertions);
        assert!(conv.extractions.is_empty(), "{:?}", conv.extractions);
        assert!(conv.unconverted.contains("pm.test(\"empty wrapper\", () => {});"));
        assert!(conv.unconverted.contains("json.items.length"));
        assert!(conv.unconverted.contains("oneOf([200, 201])"));
        assert!(conv.unconverted.contains("expectedId"));
        assert!(conv.unconverted.contains("bru.setVar(\"literal\", 123);"));
        assert!(conv.unconverted.contains("(?=foo)bar"));
    }

    #[test]
    fn legacy_postman_and_multiline_chains_convert() {
        let script = r#"
tests["status is 200"] = responseCode.code === 200;
tests["body ok"] = responseBody.includes("success");
pm.expect(pm.response.json().user.name)
    .to
    .eql("alice");
"#;
        let conv = convert_script(ScriptPhase::Test, script);
        assert_eq!(conv.unconverted, "");
        assert_eq!(
            conv.assertions,
            vec![
                assert_row(Subject::Status, "", Op::Equals, "200"),
                assert_row(Subject::Body, "", Op::Contains, "success"),
                assert_row(Subject::Json, "$.user.name", Op::Equals, "alice"),
            ]
        );
    }

    #[test]
    fn bruno_declarative_assert_and_post_vars_convert_and_preserve_unknown() {
        use super::super::kv;
        let mut req = HttpRequest {
            description: "Creates an order.".into(),
            ..Default::default()
        };
        let mut lost = Vec::new();
        apply_bru_post_vars(
            &mut req,
            &mut lost,
            "Create",
            &[
                kv("token", "res.body.data.token", true),
                kv("traceId", "res.headers['x-trace']", false),
                kv("itemCount", "res.body.items.length", true),
            ],
        );
        apply_bru_asserts(
            &mut req,
            &mut lost,
            "Create",
            &[
                kv("res.status", "eq 201", true),
                kv("res.body.id", "isDefined", true),
                kv("res.responseTime", "lt 400", true),
                kv("res.body.name", "matches /^order/i", true),
                kv("res.body.tags", "isArray", false),
                kv("res.body.items", "length 2", true),
            ],
        );
        assert_eq!(
            req.extractions,
            vec![
                extract_row("token", Subject::Json, "$.data.token"),
                Extraction {
                    enabled: false,
                    name: "traceId".into(),
                    subject: Subject::Header,
                    target: "x-trace".into(),
                },
            ]
        );
        assert_eq!(
            req.assertions,
            vec![
                assert_row(Subject::Status, "", Op::Equals, "201"),
                assert_row(Subject::Json, "$.id", Op::Exists, ""),
                assert_row(Subject::Latency, "", Op::Less, "400"),
                assert_row(Subject::Json, "$.name", Op::Matches, "(?i)^order"),
                Assertion {
                    enabled: false,
                    subject: Subject::Json,
                    target: "$.tags".into(),
                    op: Op::IsType,
                    expected: "array".into(),
                    imported: false,
                },
            ]
        );
        assert_eq!(
            lost,
            vec![
                Lost {
                    target: "Create".into(),
                    kind: super::super::LostKind::Script {
                        phase: ScriptPhase::PostResponse,
                        source: "itemCount: res.body.items.length".into(),
                    },
                },
                Lost {
                    target: "Create".into(),
                    kind: super::super::LostKind::Script {
                        phase: ScriptPhase::Test,
                        source: "res.body.items: length 2".into(),
                    },
                },
            ]
        );
        assert!(
            req.description
                .starts_with("Creates an order.\n\n#### Post-response script (not converted)")
        );
        assert!(req.description.contains("itemCount: res.body.items.length"));
        assert!(req.description.contains("res.body.items: length 2"));
    }

    #[test]
    fn description_fence_uses_more_backticks_than_source() {
        let mut desc = String::new();
        append_unconverted_to_description(&mut desc, ScriptPhase::Test, "const s = ```hi```;");
        assert!(desc.starts_with("#### Test script (not converted)\n\n````js\nconst s = ```hi```;\n````"));
    }
}
