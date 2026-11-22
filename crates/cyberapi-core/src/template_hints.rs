//! Completing a template expression as it is typed (M6): after `{{` or a
//! chain's `.`, the functions; inside `value(`, the project's custom
//! variables; inside `get(`, its APIs, and then the fields of that API's
//! last answer, a level at a time. Where the caret is decides what is
//! offered and which text a choice replaces; the view shows the list and
//! makes the edit, fetching the answer [`path_api`] names first.
//!
//! An expression is one line, as [`crate::template::parse`] reads it, so only
//! the caret's line is looked at.

pub use crate::faker::{DYNAMIC_VARS, DynCategory, DynamicVar};
use crate::model::{KvParam, Variable, VariableCategory};
use crate::search::matches_text_or_pinyin;
use crate::secrets;
use crate::template::json_path;
use crate::tree::{Node, Tree};
use crate::{EnvironmentId, ItemId, VariableId};
use serde_json::Value as Json;
use std::ops::Range;
use std::sync::Arc;

/// The items of a list offered as the next level: enough to see its shape.
const SHOWN_ITEMS: usize = 10;
/// How much of a field's value its entry shows.
const SHOWN_VALUE: usize = 40;

/// A function a template calls, as the list offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Function {
    pub name: &'static str,
    /// The short name, which works the same.
    pub alias: Option<&'static str>,
    /// Whether it reads a parameter: `timestamp()` and `openFile()` do not.
    pub takes_parameter: bool,
    /// Whether it reads this machine — its files, its environment
    /// variables — which a page has none of.
    pub on_machine: bool,
}

const fn function(
    name: &'static str,
    alias: Option<&'static str>,
    takes_parameter: bool,
    on_machine: bool,
) -> Function {
    Function {
        name,
        alias,
        takes_parameter,
        on_machine,
    }
}

/// Every function [`crate::template::evaluate`] knows, the ones a request
/// reaches for most first.
pub const FUNCTIONS: [Function; 26] = [
    function("value", None, true, false),
    function("get", Some("g"), true, false),
    function("timestamp", Some("ts"), false, false),
    function("now", None, true, false),
    function("uuid", None, false, false),
    function("random", None, true, false),
    function("randomString", None, true, false),
    function("cnName", None, false, false),
    function("cnPhone", None, false, false),
    function("cnIdCard", None, false, false),
    function("base64", Some("b64"), true, false),
    function("base64Decode", Some("b64d"), true, false),
    function("urlEncode", None, true, false),
    function("urlDecode", None, true, false),
    function("md5", None, true, false),
    function("sha1", None, true, false),
    function("sha256", None, true, false),
    function("sha512", None, true, false),
    function("hmacSha1", None, true, false),
    function("hmacSha256", None, true, false),
    function("hmacSha512", None, true, false),
    function("prompt", None, true, false),
    function("readTextFile", Some("rtf"), true, true),
    function("readFile", Some("rf"), true, true),
    function("openFile", Some("of"), false, true),
    function("env", None, true, true),
];

/// An API `get` can read, as the list names it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApiHint {
    pub id: String,
    pub name: String,
    /// The folders it is in, `a / b`; empty on the top level.
    pub place: String,
}

/// Every API of `tree`, with the folders it is in, the ones nearest `here`
/// (the API being edited) first: a request reads its neighbours' answers
/// most, a login beside the calls that use its token. Near is the fewest
/// steps through the folders from one to the other; a tie keeps the tree's
/// order, and `here` itself goes last. Without `here`, the tree's order.
pub fn apis(tree: &Tree, here: Option<&ItemId>) -> Vec<ApiHint> {
    fn walk<'a>(
        tree: &'a Tree,
        node: &'a Node,
        folders: &mut Vec<&'a Node>,
        found: &mut Vec<(ApiHint, Vec<&'a ItemId>)>,
    ) {
        if !node.is_folder() {
            let place: Vec<&str> = folders.iter().map(|folder| folder.name()).collect();
            let hint = ApiHint {
                id: node.id().as_str().to_string(),
                name: node.name().to_string(),
                place: place.join(" / "),
            };
            found.push((hint, folders.iter().map(|folder| folder.id()).collect()));
            return;
        }
        folders.push(node);
        for child in node.children().iter().filter_map(|id| tree.get(id)) {
            walk(tree, child, folders, found);
        }
        folders.pop();
    }
    let mut found = Vec::new();
    for root in tree.roots() {
        walk(tree, root, &mut Vec::new(), &mut found);
    }
    let here = here.and_then(|here| {
        let (_, folders) = found.iter().find(|(hint, _)| hint.id == here.as_str())?;
        Some((here.as_str(), folders.clone()))
    });
    if let Some((here, at)) = here {
        found.sort_by_key(|(hint, folders)| (hint.id == here, steps(&at, folders)));
    }
    found.into_iter().map(|(hint, _)| hint).collect()
}

/// The folders passed going from one API's folders (from the top) to
/// another's: up to where they part, then down.
fn steps(from: &[&ItemId], to: &[&ItemId]) -> usize {
    let shared = from.iter().zip(to).take_while(|(a, b)| a == b).count();
    from.len() + to.len() - 2 * shared
}

/// What the list is made from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sources {
    /// The enabled custom variables, name and value.
    pub custom: Vec<(String, String)>,
    pub apis: Vec<ApiHint>,
    /// Whether the functions that read this machine are offered.
    pub machine: bool,
    /// The JSON of the last answer of the API [`path_api`] names, when it
    /// has one.
    pub response: Option<Arc<Json>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HintKind {
    Function(Function),
    DynamicVar(DynamicVar),
    /// A custom variable, and its value.
    Custom(String),
    Api(ApiHint),
    /// A field of an answer, for a `get` path.
    Field(Field),
}

/// What a field of an answer holds, as its entry says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Field {
    /// An object, and how many fields it has.
    Object(usize),
    /// A list, and how many items.
    Array(usize),
    /// Anything else, as JSON, its start.
    Value(String),
}

/// One entry of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    /// What the list shows.
    pub label: String,
    pub kind: HintKind,
    /// The bytes of the text a choice replaces: what was typed of it, and
    /// the closing `)` and `}}` already after the caret that it writes again.
    pub replace: Range<usize>,
    /// What goes there; the caret ends after it.
    pub insert: String,
    /// How much of `label`, from its start, is what was typed: what a list
    /// marks. None where it matched further in, or by its pinyin.
    pub matched: usize,
}

/// Where the caret is in an expression.
enum Spot<'a> {
    /// Naming a function (and unchained `$random*` variables): `{{md`, `{{b64.r`.
    Function { typed: &'a str, chained: bool },
    /// Naming a `$...` dynamic variable: `{{$` or `{{$randomEm`.
    DynamicVar(&'a str),
    /// The first parameter of `value(`.
    Custom(&'a str),
    /// The first parameter of `get(`.
    Api(&'a str),
    /// The path of `get(api, `: the levels typed whole, and the one typed
    /// into.
    Path {
        api: &'a str,
        parent: &'a str,
        typed: &'a str,
    },
}

impl<'a> Spot<'a> {
    /// What was typed of the part the caret is in.
    fn typed(&self) -> &'a str {
        match self {
            Spot::Function { typed, .. } | Spot::DynamicVar(typed) | Spot::Custom(typed) | Spot::Api(typed) => typed,
            Spot::Path { typed, .. } => typed,
        }
    }
}

/// The spot of the caret at byte `caret` of `text`, and the rest of its
/// line.
fn caret_spot(text: &str, caret: usize) -> Option<(Spot<'_>, &str)> {
    if !text.is_char_boundary(caret) {
        return None;
    }
    let line_start = text[..caret].rfind('\n').map_or(0, |at| at + 1);
    let line_end = text[caret..].find('\n').map_or(text.len(), |at| caret + at);
    Some((spot(&text[line_start..caret])?, &text[caret..line_end]))
}

/// The API whose last answer the caret's `get` path reads, while one is
/// typed: what a view fetches before asking for the hints.
pub fn path_api(text: &str, caret: usize) -> Option<String> {
    match caret_spot(text, caret)? {
        (Spot::Path { api, .. }, _) => Some(api.to_string()),
        _ => None,
    }
}

/// Whether the caret is in an expression being typed, where the hints are
/// this module's — not, say, a GraphQL query's around it.
pub fn in_expression(text: &str, caret: usize) -> bool {
    caret_spot(text, caret).is_some()
}

/// Whether the caret is where a part of an expression starts, nothing of it
/// typed: after `{{` or `.`, in `value(` or `get(`, after `get`'s comma. A
/// choice writes up to such a place, and the list opens again there.
pub fn starts_part(text: &str, caret: usize) -> bool {
    caret_spot(text, caret).is_some_and(|(spot, _)| spot.typed().is_empty())
}

/// The entries for the caret at byte `caret` of `text`, best first; none
/// where no expression is being typed.
pub fn hints(text: &str, caret: usize, sources: &Sources) -> Vec<Hint> {
    let Some((spot, after)) = caret_spot(text, caret) else {
        return Vec::new();
    };
    let typed = spot.typed();
    let start = caret - typed.len();
    // A label is a path's whole: what was typed is its last level.
    let shown_before = match &spot {
        Spot::Path { parent: "", .. } => 0,
        Spot::Path { parent, .. } => parent.len() + 1,
        _ => 0,
    };
    let hint = |label: String, kind: HintKind, (insert, kept): (String, usize)| {
        let typed_start = label
            .get(shown_before..)
            .is_some_and(|rest| rest.to_lowercase().starts_with(&typed.to_lowercase()));
        Hint {
            matched: if typed_start { shown_before + typed.len() } else { 0 },
            label,
            kind,
            replace: start..caret + kept,
            insert,
        }
    };
    match spot {
        // A name is an identifier: its start, or its alias's, followed by
        // unchained `$random*` dynamic variables.
        Spot::Function { typed, chained } => {
            let lower = typed.to_lowercase();
            let mut out: Vec<Hint> = FUNCTIONS
                .iter()
                .filter(|function| sources.machine || !function.on_machine)
                .filter(|function| {
                    [Some(function.name), function.alias]
                        .into_iter()
                        .flatten()
                        .any(|name| name.to_lowercase().starts_with(&lower))
                })
                .map(|function| {
                    let edit = if !function.takes_parameter {
                        let (braces, kept) = closing_braces(after);
                        (format!("{}(){braces}", function.name), kept)
                    } else if after.starts_with('(') {
                        (function.name.to_string(), 0)
                    } else {
                        (format!("{}(", function.name), 0)
                    };
                    hint(function.name.to_string(), HintKind::Function(*function), edit)
                })
                .collect();
            if !chained {
                let (braces, kept) = closing_braces(after);
                out.extend(
                    DYNAMIC_VARS
                        .iter()
                        .filter(|var| {
                            typed.is_empty()
                                || var
                                    .name
                                    .strip_prefix('$')
                                    .is_some_and(|rest| rest.to_lowercase().starts_with(&lower))
                        })
                        .map(|var| {
                            let matched = if var.name.to_lowercase().starts_with(&lower) {
                                typed.len()
                            } else {
                                0
                            };
                            Hint {
                                label: var.name.to_string(),
                                kind: HintKind::DynamicVar(*var),
                                replace: start..caret + kept,
                                insert: format!("{}{braces}", var.name),
                                matched,
                            }
                        }),
                );
            }
            out
        }
        Spot::DynamicVar(typed) => {
            let lower = typed.to_lowercase();
            let needle = lower.strip_prefix('$').unwrap_or("");
            let (braces, kept) = closing_braces(after);
            ranked(DYNAMIC_VARS.iter().filter_map(|var| {
                let name_lower = var.name.to_lowercase();
                let (rank, matched) = if typed == "$" {
                    (0, 1)
                } else if name_lower.starts_with(&lower) {
                    (0, typed.len())
                } else if !needle.is_empty() && name_lower.contains(needle) {
                    (1, 0)
                } else {
                    return None;
                };
                Some((
                    rank,
                    Hint {
                        label: var.name.to_string(),
                        kind: HintKind::DynamicVar(*var),
                        replace: start..caret + kept,
                        insert: format!("{}{braces}", var.name),
                        matched,
                    },
                ))
            }))
        }
        Spot::Custom(typed) => ranked(sources.custom.iter().filter_map(|(name, value)| {
            let rank = rank(name, typed)?;
            let (closing, kept) = closing(after);
            let edit = (format!("{name}{closing}"), kept);
            Some((rank, hint(name.clone(), HintKind::Custom(value.clone()), edit)))
        })),
        Spot::Api(typed) => ranked(sources.apis.iter().filter_map(|api| {
            let rank = rank(&api.name, typed).or_else(|| rank(&api.id, typed))?;
            let insert = match after.trim_start().starts_with(',') {
                true => api.id.clone(),
                false => format!("{}, ", api.id),
            };
            Some((rank, hint(api.name.clone(), HintKind::Api(api.clone()), (insert, 0))))
        })),
        Spot::Path { parent, typed, .. } => {
            let Some(json) = &sources.response else {
                return Vec::new();
            };
            let level = match parent {
                "" => Some(json.as_ref()),
                parent => json_path(json, parent),
            };
            ranked(fields(level).into_iter().filter_map(|(key, value)| {
                let rank = rank(&key, typed)?;
                let path = match parent {
                    "" => key.clone(),
                    parent => format!("{parent}.{key}"),
                };
                // One with fields of its own opens the next level; a value
                // closes the expression.
                let (field, opens) = field(value);
                let edit = match opens {
                    true => (format!("{key}."), 0),
                    false => {
                        let (closing, kept) = closing(after);
                        (format!("{key}{closing}"), kept)
                    }
                };
                Some((rank, hint(path, HintKind::Field(field), edit)))
            }))
        }
    }
}

/// The fields of `level` a path can name: an object's keys (but those a
/// path cannot spell, with `.` or brackets in them), a list's first items.
fn fields(level: Option<&Json>) -> Vec<(String, &Json)> {
    match level {
        Some(Json::Object(map)) => map
            .iter()
            .filter(|(key, _)| !key.is_empty() && !key.contains(['.', '[', ']']))
            .map(|(key, value)| (key.clone(), value))
            .collect(),
        Some(Json::Array(items)) => items
            .iter()
            .take(SHOWN_ITEMS)
            .enumerate()
            .map(|(index, value)| (index.to_string(), value))
            .collect(),
        _ => Vec::new(),
    }
}

/// What `value` holds, and whether it has a level of its own to open.
fn field(value: &Json) -> (Field, bool) {
    match value {
        Json::Object(map) => (Field::Object(map.len()), !map.is_empty()),
        Json::Array(items) => (Field::Array(items.len()), !items.is_empty()),
        other => {
            let text = other.to_string();
            let shown = match text.char_indices().nth(SHOWN_VALUE) {
                Some((cut, _)) => format!("{}…", &text[..cut]),
                None => text,
            };
            (Field::Value(shown), false)
        }
    }
}

/// What the caret is in, from its line up to it.
fn spot<'a>(before: &'a str) -> Option<Spot<'a>> {
    let open = before.rfind("{{")?;
    let inner = &before[open + 2..];
    // Closed already, or its parameters are: the caret is past it.
    if inner.contains("}}") || inner.contains(')') {
        return None;
    }
    let identifier = |text: &str| text.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    match inner.find('(') {
        None => {
            if let Some(rest) = inner.strip_prefix('$') {
                return rest
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
                    .then_some(Spot::DynamicVar(inner));
            }
            let chained = inner.contains('.');
            let typed = inner.rsplit('.').next().unwrap_or(inner);
            identifier(typed).then_some(Spot::Function { typed, chained })
        }
        Some(paren) => {
            let name = inner[..paren].rsplit('.').next().unwrap_or_default();
            let parameter = &inner[paren + 1..];
            // A parameter typed after any spaces and quote.
            let unquoted = |text: &'a str| text.trim_start().trim_start_matches(['\'', '"']);
            match (name, parameter.split_once(',')) {
                ("value", None) => Some(Spot::Custom(unquoted(parameter))),
                ("get" | "g", None) => Some(Spot::Api(unquoted(parameter))),
                // `get`'s path, a level at a time: what follows its last
                // dot is the level typed into.
                ("get" | "g", Some((api, path))) if !path.contains(',') => {
                    let path = unquoted(path);
                    let (parent, typed) = path.rsplit_once('.').unwrap_or(("", path));
                    if typed.contains(['[', ']']) {
                        return None;
                    }
                    let api = api.trim().trim_matches(['\'', '"']);
                    Some(Spot::Path { api, parent, typed })
                }
                _ => None,
            }
        }
    }
}

/// How well `typed` names `candidate`: 0 its start, 1 elsewhere in it, its
/// pinyin included; `None` not at all.
fn rank(candidate: &str, typed: &str) -> Option<u8> {
    if candidate.to_lowercase().starts_with(&typed.to_lowercase()) {
        Some(0)
    } else if matches_text_or_pinyin(candidate, typed) {
        Some(1)
    } else {
        None
    }
}

/// The matches, the better ranks first, each rank in the sources' order.
fn ranked(found: impl Iterator<Item = (u8, Hint)>) -> Vec<Hint> {
    let mut found: Vec<(u8, Hint)> = found.collect();
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, hint)| hint).collect()
}

/// What closes a parameter list and its expression, and how much of it
/// already follows the caret, to be written over.
fn closing(after: &str) -> (&'static str, usize) {
    let paren = usize::from(after.starts_with(')'));
    let (_, braces) = closing_braces(&after[paren..]);
    (")}}", paren + braces)
}

/// `}}`, and how much of it already follows the caret.
fn closing_braces(after: &str) -> (&'static str, usize) {
    ("}}", if after.starts_with("}}") { 2 } else { 0 })
}

/// UI-9: what kind of variable reference a `{{...}}` expression is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VarRefKind {
    /// `{{value(NAME)}}` (custom variable, or folder/API override).
    Value,
    /// Leading `{{NAME}}` (environment prefix, or folder/API override).
    EnvPrefix,
}

/// UI-9: a variable reference found in text, with its byte range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarRef {
    pub name: String,
    pub kind: VarRefKind,
    /// Byte range of the whole `{{...}}` expression in the input string.
    pub range: Range<usize>,
}

/// UI-9: which layer a variable's effective value came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VarSource {
    /// The API's own variable / path parameter.
    ApiOwn,
    /// A folder's variable (carrying the folder's name).
    Folder(String),
    /// A named environment's variable (carrying the environment's name).
    NamedEnv(String),
    /// The project's shared layer (all environments).
    Shared,
    /// Not defined or disabled in the active environment.
    Undefined,
}

/// UI-9: a variable's effective value and where it came from, with secrets
/// already masked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedVar {
    pub name: String,
    pub value: Option<String>,
    pub source: VarSource,
    pub secret: bool,
}

/// Parses the variable reference inside `inner` (between `{{` and `}}`) whose
/// full expression spans `range` in the document.
fn parse_var_ref(inner: &str, range: Range<usize>) -> Option<VarRef> {
    let trimmed = inner.trim();
    if let Some(rest) = trimmed.strip_prefix("value(")
        && let Some(end) = rest.find(')')
    {
        let name = rest[..end].trim().trim_matches(['\'', '"']).trim();
        if !name.is_empty() {
            return Some(VarRef {
                name: name.to_string(),
                kind: VarRefKind::Value,
                range,
            });
        }
    }
    if range.start == 0 && !trimmed.is_empty() && !trimmed.starts_with('$') && !trimmed.contains(['(', ')', '.', ' ']) {
        return Some(VarRef {
            name: trimmed.to_string(),
            kind: VarRefKind::EnvPrefix,
            range,
        });
    }
    None
}

/// UI-9: the `{{value(NAME)}}` or leading `{{ENV}}` variable reference at byte
/// `offset` of `text`, if any.
pub fn expression_at(text: &str, offset: usize) -> Option<VarRef> {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    let line_start = text[..offset].rfind('\n').map_or(0, |at| at + 1);
    let line_end = text[offset..].find('\n').map_or(text.len(), |at| offset + at);
    let line = &text[line_start..line_end];
    let mut cursor = 0;
    while let Some(rel_open) = line[cursor..].find("{{") {
        let open = cursor + rel_open;
        let Some(rel_close) = line[open + 2..].find("}}") else {
            break;
        };
        let close = open + 2 + rel_close + 2;
        let doc_range = (line_start + open)..(line_start + close);
        if offset >= doc_range.start && offset <= doc_range.end {
            return parse_var_ref(&line[open + 2..close - 2], doc_range);
        }
        cursor = close;
    }
    None
}

/// UI-9: every `{{value(NAME)}}` or leading `{{ENV}}` variable reference in
/// `text`, in order of appearance.
pub fn referenced_vars(text: &str) -> Vec<VarRef> {
    let mut found = Vec::new();
    let mut line_start = 0;
    for line in text.split('\n') {
        let mut cursor = 0;
        while let Some(rel_open) = line[cursor..].find("{{") {
            let open = cursor + rel_open;
            let Some(rel_close) = line[open + 2..].find("}}") else {
                break;
            };
            let close = open + 2 + rel_close + 2;
            let doc_range = (line_start + open)..(line_start + close);
            if let Some(var_ref) = parse_var_ref(&line[open + 2..close - 2], doc_range) {
                found.push(var_ref);
            }
            cursor = close;
        }
        line_start += line.len() + 1;
    }
    found
}

/// UI-9: resolves `name` in `category` following CyberAPI's precedence:
/// API own non-empty variable -> nearest folder variable -> active named
/// environment -> shared project layer -> undefined.
///
/// Secret variables always return a masked value (`secrets::MASK`) when
/// non-empty, never leaking the underlying secret.
pub fn resolve_with_source(
    name: &str,
    category: VariableCategory,
    variables: &[Variable],
    secret_ids: &[VariableId],
    active_env: Option<(&EnvironmentId, &str)>,
    folders: &[(&str, &[KvParam])],
    api_own: &[KvParam],
) -> ResolvedVar {
    if let Some(kv) = api_own
        .iter()
        .rev()
        .find(|kv| kv.enabled && kv.key.trim() == name && !kv.value.is_empty())
    {
        return ResolvedVar {
            name: name.to_string(),
            value: Some(kv.value.clone()),
            source: VarSource::ApiOwn,
            secret: false,
        };
    }
    // `folders` is nearest-first (as `inherit::folders_above` returns).
    for (folder_name, vars) in folders {
        if let Some(kv) = vars.iter().rev().find(|kv| kv.enabled && kv.key.trim() == name) {
            return ResolvedVar {
                name: name.to_string(),
                value: Some(kv.value.clone()),
                source: VarSource::Folder((*folder_name).to_string()),
                secret: false,
            };
        }
    }
    let mask_if_secret = |v: &Variable| {
        let secret = secret_ids.contains(&v.id);
        let value = if secret {
            if v.value.is_empty() {
                String::new()
            } else {
                secrets::MASK.to_string()
            }
        } else {
            v.value.clone()
        };
        (value, secret)
    };
    if let Some((env_id, env_name)) = active_env
        && let Some(v) = variables
            .iter()
            .find(|v| v.category == category && v.enabled && v.environment.as_ref() == Some(env_id) && v.name == name)
    {
        let (value, secret) = mask_if_secret(v);
        return ResolvedVar {
            name: name.to_string(),
            value: Some(value),
            source: VarSource::NamedEnv(env_name.to_string()),
            secret,
        };
    }
    if let Some(v) = variables
        .iter()
        .find(|v| v.category == category && v.enabled && v.environment.is_none() && v.name == name)
    {
        let (value, secret) = mask_if_secret(v);
        return ResolvedVar {
            name: name.to_string(),
            value: Some(value),
            source: VarSource::Shared,
            secret,
        };
    }
    ResolvedVar {
        name: name.to_string(),
        value: None,
        source: VarSource::Undefined,
        secret: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn sources() -> Sources {
        Sources {
            custom: vec![("token".into(), "t-1".into()), ("tenant".into(), "acme".into())],
            apis: vec![
                ApiHint {
                    id: "01LOGIN".into(),
                    name: "登录".into(),
                    place: "用户".into(),
                },
                ApiHint {
                    id: "01PING".into(),
                    name: "ping".into(),
                    place: String::new(),
                },
            ],
            machine: true,
            response: None,
        }
    }

    /// With the login's last answer, which a `get` path reads.
    fn answered() -> Sources {
        Sources {
            response: Some(Arc::new(serde_json::json!({
                "token": "abc.def",
                "data": {"items": [{"id": 1}, {"id": 2}], "total": 2, "empty": []},
            }))),
            ..sources()
        }
    }

    /// The labels offered with the caret at `|`.
    fn labels(text: &str, sources: &Sources) -> Vec<String> {
        let caret = text.find('|').expect("a caret");
        let text = text.replace('|', "");
        hints(&text, caret, sources)
            .into_iter()
            .map(|hint| hint.label)
            .collect()
    }

    /// The text after choosing `label` with the caret at `|`.
    fn chosen(text: &str, label: &str) -> String {
        chosen_from(text, label, &sources())
    }

    fn chosen_from(text: &str, label: &str, sources: &Sources) -> String {
        let caret = text.find('|').expect("a caret");
        let mut text = text.replace('|', "");
        let hint = hints(&text, caret, sources)
            .into_iter()
            .find(|hint| hint.label == label)
            .expect("offered");
        text.replace_range(hint.replace.clone(), &hint.insert);
        let caret = hint.replace.start + hint.insert.len();
        text.insert(caret, '|');
        text
    }

    #[test]
    fn braces_open_the_functions_and_dynamic_vars() {
        let all: Vec<&str> = FUNCTIONS
            .iter()
            .map(|function| function.name)
            .chain(DYNAMIC_VARS.iter().map(|var| var.name))
            .collect();
        assert_eq!(labels("{{|", &sources()), all);
        assert_eq!(labels(r#"{"a": "{{|"}"#, &sources()), all, "in a JSON string");
        assert_eq!(labels("{{m|", &sources()), ["md5"], "a name's start only");
        assert_eq!(labels("{{RTF|", &sources()), ["readTextFile"], "by its alias, any case");
        assert_eq!(
            labels("{{b64.|", &sources()).len(),
            FUNCTIONS.len(),
            "dynamic vars do not chain after '.'"
        );
        assert_eq!(
            labels("{{b64.re|", &sources()),
            ["readTextFile", "readFile"],
            "in a chain"
        );
        // A page has no files or environment variables to read.
        let page = Sources {
            machine: false,
            ..sources()
        };
        assert_eq!(labels("{{|", &page).len(), FUNCTIONS.len() - 4 + DYNAMIC_VARS.len());
    }

    #[test]
    fn dollar_prefix_offers_and_completes_dynamic_vars_and_cn_functions() {
        let all_dyn: Vec<&str> = DYNAMIC_VARS.iter().map(|var| var.name).collect();
        assert_eq!(labels("{{$|", &sources()), all_dyn);
        assert_eq!(labels("{{$|}}", &sources()), all_dyn);
        assert_eq!(
            labels("{{$randomEm|", &sources()),
            ["$randomEmail"],
            "prefix match with '$'"
        );
        assert_eq!(
            labels("{{$email|", &sources()),
            ["$randomEmail", "$randomExampleEmail"],
            "substring match after '$'"
        );
        assert_eq!(
            labels("{{randomEm|", &sources()),
            ["$randomEmail"],
            "also matches without typing '$'"
        );
        assert_eq!(
            labels("{{cn|", &sources()),
            ["cnName", "cnPhone", "cnIdCard"],
            "Chinese data generators"
        );
        assert_eq!(chosen("{{$|}}", "$randomEmail"), "{{$randomEmail}}|");
        assert_eq!(chosen("{{$randomEm|", "$randomEmail"), "{{$randomEmail}}|");
        assert_eq!(chosen("{{randomEm|}}", "$randomEmail"), "{{$randomEmail}}|");
        assert_eq!(chosen("{{cn|", "cnPhone"), "{{cnPhone()}}|");
        assert_eq!(
            expression_at("{{$randomUrl}}/v1", 3),
            None,
            "dynamic vars are not env prefixes"
        );
    }

    #[test]
    fn nothing_outside_an_open_expression() {
        assert!(labels("|", &sources()).is_empty());
        assert!(labels("{|", &sources()).is_empty());
        assert!(labels("{{ts()}} |", &sources()).is_empty(), "closed");
        assert!(labels("{{md5(a)|", &sources()).is_empty(), "its parameters are closed");
        assert!(labels("{{md5(|", &sources()).is_empty(), "md5 takes any text");
        assert!(labels("{{zz|", &sources()).is_empty(), "no such function");
        assert!(labels("{{a b|", &sources()).is_empty());
        assert!(labels("{{\n|", &sources()).is_empty(), "another line");
        assert!(
            labels("{{get(01PING, |", &sources()).is_empty(),
            "the path is the reader's"
        );
    }

    #[test]
    fn value_offers_the_custom_variables() {
        assert_eq!(labels("{{value(|", &sources()), ["token", "tenant"]);
        assert_eq!(labels("{{value(to|", &sources()), ["token"]);
        assert_eq!(labels("{{value( 'te|", &sources()), ["tenant"]);
        assert_eq!(labels("{{b64.value(|", &sources()), ["token", "tenant"]);
    }

    #[test]
    fn get_offers_the_apis_by_name_or_pinyin() {
        assert_eq!(labels("{{get(|", &sources()), ["登录", "ping"]);
        assert_eq!(labels("{{g(dl|", &sources()), ["登录"], "the initials");
        assert_eq!(labels("{{get(01P|", &sources()), ["ping"], "an id typed");
    }

    #[test]
    fn a_choice_replaces_what_was_typed_and_closes_what_it_opens() {
        assert_eq!(chosen("{{ti|", "timestamp"), "{{timestamp()}}|");
        assert_eq!(chosen("{{ti|}}", "timestamp"), "{{timestamp()}}|", "closed already");
        assert_eq!(
            chosen("a {{m| b", "md5"),
            "a {{md5(| b",
            "the parameter is the reader's"
        );
        assert_eq!(chosen("{{m|(x)}}", "md5"), "{{md5|(x)}}");
        assert_eq!(chosen("{{va|", "value"), "{{value(|");
        assert_eq!(chosen("{{value(t|", "token"), "{{value(token)}}|");
        assert_eq!(chosen("{{value(t|}}", "token"), "{{value(token)}}|");
        assert_eq!(chosen("{{value(t|)}}", "token"), "{{value(token)}}|");
        assert_eq!(chosen("{{value(t|) x", "token"), "{{value(token)}}| x");
        assert_eq!(chosen(r#""{{get(|""#, "登录"), r#""{{get(01LOGIN, |""#);
        assert_eq!(chosen("{{get(|, data)}}", "ping"), "{{get(01PING|, data)}}");
    }

    #[test]
    fn the_apis_come_in_the_trees_order_with_their_folders() {
        let tree = Tree::build(&crate::tree::fixture::sample());
        let found: Vec<(String, String, String)> = apis(&tree, None)
            .into_iter()
            .map(|api| (api.id, api.name, api.place))
            .collect();
        let expected = [
            ("login", "登录", "用户"),
            ("me", "me", "用户 / profile"),
            ("list", "list", "orders"),
            ("health", "health", ""),
        ]
        .map(|(id, name, place)| (id.to_string(), name.to_string(), place.to_string()));
        assert_eq!(found, expected);
    }

    #[test]
    fn the_apis_near_the_one_edited_come_first() {
        let tree = Tree::build(&crate::tree::fixture::sample());
        let order = |here: &str| -> Vec<String> {
            apis(&tree, Some(&ItemId::from(here)))
                .into_iter()
                .map(|api| api.id)
                .collect()
        };
        // In profile, inside 用户: the folder above, the top level, then
        // orders; itself last.
        assert_eq!(order("me"), ["login", "health", "list", "me"]);
        // In 用户: its subfolder's and the top level's, a step each and in
        // the tree's order, then orders'.
        assert_eq!(order("login"), ["me", "health", "list", "login"]);
        // On the top level: 用户's and orders' a step down, profile's two.
        assert_eq!(order("health"), ["login", "list", "me", "health"]);
    }

    #[test]
    fn a_match_still_goes_before_a_nearer_api() {
        let near_first = Sources {
            apis: vec![
                ApiHint {
                    id: "01NEAR".into(),
                    name: "订单详情".into(),
                    place: String::new(),
                },
                ApiHint {
                    id: "01FAR".into(),
                    name: "dl".into(),
                    place: String::new(),
                },
            ],
            ..sources()
        };
        assert_eq!(labels("{{get(|", &near_first), ["订单详情", "dl"]);
        assert_eq!(
            labels("{{get(d|", &near_first),
            ["dl", "订单详情"],
            "its name starts so"
        );
    }

    #[test]
    fn a_get_path_offers_the_answers_fields_a_level_at_a_time() {
        let answered = answered();
        assert_eq!(
            labels("{{get(01LOGIN, |", &answered),
            ["token", "data"],
            "in the answer's order"
        );
        assert_eq!(labels("{{get(01LOGIN, da|", &answered), ["data"]);
        assert_eq!(
            labels(r#""{{g('01LOGIN', 'data.|""#, &answered),
            ["data.items", "data.total", "data.empty"]
        );
        assert_eq!(
            labels("{{get(01LOGIN, data.items.|", &answered),
            ["data.items.0", "data.items.1"]
        );
        assert_eq!(labels("{{get(01LOGIN, data.items.0.|", &answered), ["data.items.0.id"]);
        assert!(labels("{{get(01LOGIN, nope.|", &answered).is_empty(), "no such field");
        assert!(labels("{{get(01LOGIN, |", &sources()).is_empty(), "no answer to read");
        let kinds: Vec<HintKind> = {
            let text = "{{get(01LOGIN, data.";
            hints(text, text.len(), &answered)
                .into_iter()
                .map(|hint| hint.kind)
                .collect()
        };
        assert_eq!(
            kinds,
            [
                HintKind::Field(Field::Array(2)),
                HintKind::Field(Field::Value("2".into())),
                HintKind::Field(Field::Array(0)),
            ]
        );
    }

    #[test]
    fn a_field_with_fields_opens_the_next_level_and_a_value_closes() {
        let answered = answered();
        assert_eq!(
            chosen_from("{{get(01LOGIN, t|", "token", &answered),
            "{{get(01LOGIN, token)}}|"
        );
        assert_eq!(
            chosen_from("{{get(01LOGIN, t|)}}", "token", &answered),
            "{{get(01LOGIN, token)}}|"
        );
        assert_eq!(
            chosen_from("{{get(01LOGIN, d|", "data", &answered),
            "{{get(01LOGIN, data.|"
        );
        assert_eq!(
            chosen_from("{{get(01LOGIN, data.|", "data.empty", &answered),
            "{{get(01LOGIN, data.empty)}}|",
            "an empty list has nothing to open"
        );
    }

    #[test]
    fn the_api_a_path_reads_is_known_before_its_answer() {
        let at = |text: &str| {
            let caret = text.find('|').expect("a caret");
            path_api(&text.replace('|', ""), caret)
        };
        assert_eq!(at("{{get(01LOGIN, |").as_deref(), Some("01LOGIN"));
        assert_eq!(at("{{g( '01PING' , data.x|").as_deref(), Some("01PING"));
        assert_eq!(at("{{get(01LOGIN|"), None, "still naming the API");
        assert_eq!(at("{{value(a, |"), None);
    }

    #[test]
    fn a_fresh_part_is_where_the_list_opens_again() {
        let fresh = |text: &str| {
            let caret = text.find('|').expect("a caret");
            starts_part(&text.replace('|', ""), caret)
        };
        for text in [
            "{{|",
            "{{b64.|",
            "{{value(|",
            "{{get(|",
            "{{get(01P, |",
            "{{get(01P, data.|",
        ] {
            assert!(fresh(text), "{text}");
        }
        for text in ["|", "{{va|", "{{get(01P, da|", "{{md5(|", "{{ts()}}|"] {
            assert!(!fresh(text), "{text}");
        }
    }

    #[test]
    fn an_expression_is_told_from_the_text_around_it() {
        let inside = |text: &str| {
            let caret = text.find('|').expect("a caret");
            in_expression(&text.replace('|', ""), caret)
        };
        for text in ["{ user(id: \"{{va|", "{{get(01P, da|", "{{|"] {
            assert!(inside(text), "{text}");
        }
        for text in ["{ us|", "{{ts()}} us|", "|"] {
            assert!(!inside(text), "{text}");
        }
    }

    #[test]
    fn what_was_typed_is_marked_at_a_labels_start() {
        let marked = |text: &str, sources: &Sources| -> Vec<(String, usize)> {
            let caret = text.find('|').expect("a caret");
            hints(&text.replace('|', ""), caret, sources)
                .into_iter()
                .map(|hint| (hint.label, hint.matched))
                .collect()
        };
        assert_eq!(marked("{{MD|", &sources()), [("md5".to_string(), 2)]);
        assert_eq!(
            marked("{{get(dl|", &sources()),
            [("登录".to_string(), 0)],
            "by its pinyin"
        );
        assert_eq!(
            marked("{{get(01LOGIN, data.it|", &answered()),
            [("data.items".to_string(), 7)],
            "the path's levels before it too"
        );
    }

    #[test]
    fn a_caret_inside_a_character_offers_nothing() {
        assert!(hints("{{登", 3, &sources()).is_empty());
    }

    #[test]
    fn expression_at_finds_value_and_env_prefix_references() {
        let body = "{\n  \"token\": \"{{value(API_TOKEN)}}\",\n  \"enc\": \"{{value('SECRET').b64}}\"\n}";
        let pos = body.find("API_TOKEN").expect("token offset") + 2;
        let found = expression_at(body, pos).expect("value ref");
        assert_eq!(found.name, "API_TOKEN");
        assert_eq!(found.kind, VarRefKind::Value);
        assert_eq!(&body[found.range], "{{value(API_TOKEN)}}");

        let enc_pos = body.find("SECRET").expect("secret offset");
        let enc = expression_at(body, enc_pos).expect("chained value ref");
        assert_eq!(enc.name, "SECRET");
        assert_eq!(enc.kind, VarRefKind::Value);
        assert_eq!(&body[enc.range], "{{value('SECRET').b64}}");

        assert_eq!(expression_at(body, 2), None, "outside any expression");

        let uri = "{{PROD}}/v1/users/{{value(UID)}}";
        let env_ref = expression_at(uri, 3).expect("leading env prefix");
        assert_eq!(env_ref.name, "PROD");
        assert_eq!(env_ref.kind, VarRefKind::EnvPrefix);
        assert_eq!(&uri[env_ref.range], "{{PROD}}");

        let all = referenced_vars(uri);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "PROD");
        assert_eq!(all[0].kind, VarRefKind::EnvPrefix);
        assert_eq!(all[1].name, "UID");
        assert_eq!(all[1].kind, VarRefKind::Value);
    }

    #[test]
    fn resolve_with_source_respects_precedence_and_masks_secrets() {
        let staging_id = EnvironmentId::from("env-staging");
        let secret_id = VariableId::from("var-secret");
        let make_var = |id: &str, name: &str, value: &str, enabled: bool, category: VariableCategory, env| Variable {
            id: VariableId::from(id),
            collection: crate::CollectionId::from("col-1"),
            category,
            name: name.into(),
            value: value.into(),
            enabled,
            environment: env,
            created_at: crate::Timestamp::default(),
            updated_at: crate::Timestamp::default(),
        };
        let vars = vec![
            make_var(
                "v1",
                "HOST",
                "https://shared.test",
                true,
                VariableCategory::Environment,
                None,
            ),
            make_var(
                "v2",
                "HOST",
                "https://staging.test",
                true,
                VariableCategory::Environment,
                Some(staging_id.clone()),
            ),
            make_var(
                "var-secret",
                "API_KEY",
                "super-secret-value",
                true,
                VariableCategory::Custom,
                None,
            ),
            make_var("v4", "OFF", "nope", false, VariableCategory::Custom, None),
        ];
        let secret_ids = [secret_id];
        let inner_folder = [KvParam {
            key: "HOST".into(),
            value: "https://folder.test".into(),
            enabled: true,
            ..Default::default()
        }];
        let api_own = [KvParam {
            key: "HOST".into(),
            value: "https://api.test".into(),
            enabled: true,
            ..Default::default()
        }];

        // API own overrides folder and env.
        let r = resolve_with_source(
            "HOST",
            VariableCategory::Environment,
            &vars,
            &secret_ids,
            Some((&staging_id, "Staging")),
            &[("Users", &inner_folder)],
            &api_own,
        );
        assert_eq!(r.value.as_deref(), Some("https://api.test"));
        assert_eq!(r.source, VarSource::ApiOwn);

        // Folder overrides named env.
        let r = resolve_with_source(
            "HOST",
            VariableCategory::Environment,
            &vars,
            &secret_ids,
            Some((&staging_id, "Staging")),
            &[("Users", &inner_folder)],
            &[],
        );
        assert_eq!(r.value.as_deref(), Some("https://folder.test"));
        assert_eq!(r.source, VarSource::Folder("Users".into()));

        // Named env overrides shared.
        let r = resolve_with_source(
            "HOST",
            VariableCategory::Environment,
            &vars,
            &secret_ids,
            Some((&staging_id, "Staging")),
            &[],
            &[],
        );
        assert_eq!(r.value.as_deref(), Some("https://staging.test"));
        assert_eq!(r.source, VarSource::NamedEnv("Staging".into()));

        // Shared fallback when no named env active.
        let r = resolve_with_source(
            "HOST",
            VariableCategory::Environment,
            &vars,
            &secret_ids,
            None,
            &[],
            &[],
        );
        assert_eq!(r.value.as_deref(), Some("https://shared.test"));
        assert_eq!(r.source, VarSource::Shared);

        // Secret is masked.
        let r = resolve_with_source("API_KEY", VariableCategory::Custom, &vars, &secret_ids, None, &[], &[]);
        assert_eq!(r.value.as_deref(), Some(secrets::MASK));
        assert_eq!(r.source, VarSource::Shared);
        assert!(r.secret);

        // Disabled variable is undefined.
        let r = resolve_with_source("OFF", VariableCategory::Custom, &vars, &secret_ids, None, &[], &[]);
        assert_eq!(r.value, None);
        assert_eq!(r.source, VarSource::Undefined);
    }
}
