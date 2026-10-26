//! Completing a template expression as it is typed (M6): after `{{` or a
//! chain's `.`, the functions; inside `value(`, the project's custom
//! variables; inside `get(`, its APIs, and then the fields of that API's
//! last answer, a level at a time. Where the caret is decides what is
//! offered and which text a choice replaces; the view shows the list and
//! makes the edit, fetching the answer [`path_api`] names first.
//!
//! An expression is one line, as [`crate::template::parse`] reads it, so only
//! the caret's line is looked at.

use crate::ItemId;
use crate::search::matches_text_or_pinyin;
use crate::template::json_path;
use crate::tree::{Node, Tree};
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
    /// Whether it needs this machine's files, which a page has none of.
    pub uses_files: bool,
}

const fn function(
    name: &'static str,
    alias: Option<&'static str>,
    takes_parameter: bool,
    uses_files: bool,
) -> Function {
    Function {
        name,
        alias,
        takes_parameter,
        uses_files,
    }
}

/// Every function [`crate::template::evaluate`] knows, the ones a request
/// reaches for most first.
pub const FUNCTIONS: [Function; 16] = [
    function("value", None, true, false),
    function("get", Some("g"), true, false),
    function("timestamp", Some("ts"), false, false),
    function("now", None, true, false),
    function("uuid", None, false, false),
    function("random", None, true, false),
    function("randomString", None, true, false),
    function("base64", Some("b64"), true, false),
    function("urlEncode", None, true, false),
    function("md5", None, true, false),
    function("sha256", None, true, false),
    function("hmacSha256", None, true, false),
    function("prompt", None, true, false),
    function("readTextFile", Some("rtf"), true, true),
    function("readFile", Some("rf"), true, true),
    function("openFile", Some("of"), false, true),
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
    /// Whether the functions that read files are offered.
    pub files: bool,
    /// The JSON of the last answer of the API [`path_api`] names, when it
    /// has one.
    pub response: Option<Arc<Json>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HintKind {
    Function(Function),
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
    /// Naming a function: `{{md` or `{{b64.r`.
    Function(&'a str),
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
            Spot::Function(typed) | Spot::Custom(typed) | Spot::Api(typed) => typed,
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
        // A name is an identifier: its start, or its alias's.
        Spot::Function(typed) => FUNCTIONS
            .iter()
            .filter(|function| sources.files || !function.uses_files)
            .filter(|function| {
                [Some(function.name), function.alias]
                    .into_iter()
                    .flatten()
                    .any(|name| name.to_lowercase().starts_with(&typed.to_lowercase()))
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
            .collect(),
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
            let typed = inner.rsplit('.').next().unwrap_or(inner);
            identifier(typed).then_some(Spot::Function(typed))
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
            files: true,
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
    fn braces_open_the_functions() {
        let all: Vec<&str> = FUNCTIONS.iter().map(|function| function.name).collect();
        assert_eq!(labels("{{|", &sources()), all);
        assert_eq!(labels(r#"{"a": "{{|"}"#, &sources()), all, "in a JSON string");
        assert_eq!(labels("{{m|", &sources()), ["md5"], "a name's start only");
        assert_eq!(labels("{{RTF|", &sources()), ["readTextFile"], "by its alias, any case");
        assert_eq!(
            labels("{{b64.re|", &sources()),
            ["readTextFile", "readFile"],
            "in a chain"
        );
        // A page has no files to read.
        let page = Sources {
            files: false,
            ..sources()
        };
        assert_eq!(labels("{{|", &page).len(), FUNCTIONS.len() - 3);
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
}
