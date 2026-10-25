//! A GraphQL endpoint's schema, read from its introspection query, and what
//! the query editor offers at the caret from it: the fields of the type a
//! selection set selects from, a field's arguments, an argument's enum
//! values, the operation keywords at the top.
//!
//! The reading of the query is a scan, not a parse: it follows the braces
//! and the parentheses up to the caret, which is what an unfinished query —
//! the only kind one types in — still has.

use serde_json::Value;
use std::collections::HashMap;

/// The query that asks an endpoint for its schema.
pub const INTROSPECTION_QUERY: &str = "query IntrospectionQuery { __schema { \
queryType { name } mutationType { name } subscriptionType { name } \
types { kind name description \
fields(includeDeprecated: true) { name description args { name description type { ...TypeRef } } type { ...TypeRef } } \
inputFields { name description type { ...TypeRef } } \
enumValues(includeDeprecated: true) { name description } \
possibleTypes { name } } } } \
fragment TypeRef on __Type { kind name ofType { kind name ofType { kind name ofType { kind name \
ofType { kind name ofType { kind name ofType { kind name } } } } } } }";

/// A type as a field or an argument has it: as written (`[User!]!`), and
/// the named type under the lists and the `!`s.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeRef {
    pub display: String,
    pub named: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputDef {
    pub name: String,
    pub description: String,
    pub type_ref: TypeRef,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldDef {
    pub name: String,
    pub description: String,
    pub args: Vec<InputDef>,
    pub type_ref: TypeRef,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeDef {
    /// `OBJECT`, `INTERFACE`, `UNION`, `ENUM`, `INPUT_OBJECT`, `SCALAR`.
    pub kind: String,
    pub description: String,
    pub fields: Vec<FieldDef>,
    pub input_fields: Vec<InputDef>,
    pub enum_values: Vec<(String, String)>,
    /// A union's or an interface's members.
    pub possible_types: Vec<String>,
}

/// What an endpoint's introspection said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Schema {
    pub query: Option<String>,
    pub mutation: Option<String>,
    pub subscription: Option<String>,
    pub types: HashMap<String, TypeDef>,
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn type_ref(value: &Value) -> TypeRef {
    let inner = || value.get("ofType").map(type_ref).unwrap_or_default();
    match value.get("kind").and_then(Value::as_str) {
        Some("NON_NULL") => {
            let inner = inner();
            TypeRef {
                display: format!("{}!", inner.display),
                named: inner.named,
            }
        }
        Some("LIST") => {
            let inner = inner();
            TypeRef {
                display: format!("[{}]", inner.display),
                named: inner.named,
            }
        }
        _ => {
            let name = text(value, "name");
            TypeRef {
                display: name.clone(),
                named: name,
            }
        }
    }
}

fn inputs(value: Option<&Value>) -> Vec<InputDef> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| InputDef {
                    name: text(item, "name"),
                    description: text(item, "description"),
                    type_ref: item.get("type").map(type_ref).unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Schema {
    /// The schema an introspection answer holds, or why there is none: an
    /// answer that is not JSON, one with `errors` (an endpoint that turned
    /// introspection off says so there), one without `__schema`.
    pub fn from_introspection(body: &[u8]) -> Result<Self, String> {
        let answer: Value = serde_json::from_slice(body).map_err(|error| format!("not JSON: {error}"))?;
        let errors = crate::graphql::response_errors(body);
        let Some(schema) = answer.get("data").and_then(|data| data.get("__schema")) else {
            return Err(match errors.first() {
                Some(error) => error.clone(),
                None => "the answer has no __schema".into(),
            });
        };
        let root = |key: &str| {
            schema
                .get(key)
                .and_then(|root| root.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let types = schema
            .get("types")
            .and_then(Value::as_array)
            .map(|types| {
                types
                    .iter()
                    .map(|item| {
                        let fields = item
                            .get("fields")
                            .and_then(Value::as_array)
                            .map(|fields| {
                                fields
                                    .iter()
                                    .map(|field| FieldDef {
                                        name: text(field, "name"),
                                        description: text(field, "description"),
                                        args: inputs(field.get("args")),
                                        type_ref: field.get("type").map(type_ref).unwrap_or_default(),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let enum_values = item
                            .get("enumValues")
                            .and_then(Value::as_array)
                            .map(|values| {
                                values
                                    .iter()
                                    .map(|value| (text(value, "name"), text(value, "description")))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let possible_types = item
                            .get("possibleTypes")
                            .and_then(Value::as_array)
                            .map(|types| types.iter().map(|kind| text(kind, "name")).collect())
                            .unwrap_or_default();
                        let def = TypeDef {
                            kind: text(item, "kind"),
                            description: text(item, "description"),
                            fields,
                            input_fields: inputs(item.get("inputFields")),
                            enum_values,
                            possible_types,
                        };
                        (text(item, "name"), def)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            query: root("queryType"),
            mutation: root("mutationType"),
            subscription: root("subscriptionType"),
            types,
        })
    }

    /// The types it names that are not GraphQL's own (`__Type`, …).
    pub fn type_count(&self) -> usize {
        self.types.keys().filter(|name| !name.starts_with("__")).count()
    }

    /// The types a browser lists (#70): the operation types first — query,
    /// mutation, subscription — then the rest by name, without GraphQL's
    /// own; those whose name holds `filter`, in any case.
    pub fn browsed_types(&self, filter: &str) -> Vec<&str> {
        let filter = filter.trim().to_lowercase();
        let roots: Vec<&str> = [&self.query, &self.mutation, &self.subscription]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .filter(|name| self.types.contains_key(*name))
            .collect();
        let mut rest: Vec<&str> = self
            .types
            .keys()
            .map(String::as_str)
            .filter(|name| !name.starts_with("__") && !roots.contains(name))
            .collect();
        rest.sort_by_key(|name| name.to_lowercase());
        roots
            .into_iter()
            .chain(rest)
            .filter(|name| filter.is_empty() || name.to_lowercase().contains(&filter))
            .collect()
    }

    fn field(&self, of: &str, name: &str) -> Option<&FieldDef> {
        self.types.get(of)?.fields.iter().find(|field| field.name == name)
    }
}

/// What kind of thing a suggestion is, for its icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    Field,
    Argument,
    EnumValue,
    Keyword,
    Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    /// Its type, or what it is.
    pub detail: String,
    pub description: String,
    pub kind: SuggestionKind,
    /// What choosing it writes in place of the word at the caret.
    pub insert: String,
}

/// What the caret is offered, and the byte the word it completes starts at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completions {
    pub from: usize,
    pub items: Vec<Suggestion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Name(String),
    Punct(char),
    Spread,
}

/// The tokens of `text`: names, punctuation and `...`; strings, block
/// strings and comments skipped. And whether `text` ends inside one of
/// those, where nothing is offered.
fn tokens(text: &str) -> (Vec<Token>, bool) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    let mut open = false;
    while at < chars.len() {
        let ch = chars[at];
        if ch == '#' {
            while at < chars.len() && chars[at] != '\n' {
                at += 1;
            }
            open = at >= chars.len();
        } else if ch == '"' {
            let block = chars.get(at + 1) == Some(&'"') && chars.get(at + 2) == Some(&'"');
            if block {
                at += 3;
                while at < chars.len()
                    && !(chars[at] == '"' && chars.get(at + 1) == Some(&'"') && chars.get(at + 2) == Some(&'"'))
                {
                    at += 1;
                }
                open = at >= chars.len();
                at += 3;
            } else {
                at += 1;
                while at < chars.len() && chars[at] != '"' && chars[at] != '\n' {
                    if chars[at] == '\\' {
                        at += 1;
                    }
                    at += 1;
                }
                open = at >= chars.len();
                at += 1;
            }
        } else if ch == '.' && chars.get(at + 1) == Some(&'.') && chars.get(at + 2) == Some(&'.') {
            out.push(Token::Spread);
            at += 3;
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            let start = at;
            while at < chars.len() && (chars[at].is_ascii_alphanumeric() || chars[at] == '_') {
                at += 1;
            }
            out.push(Token::Name(chars[start..at].iter().collect()));
        } else {
            if !ch.is_whitespace() && ch != ',' {
                out.push(Token::Punct(ch));
            }
            at += 1;
        }
    }
    (out, open)
}

/// Where the scan is: inside a selection set of a type (its name, when
/// known), a field's arguments, or a value of them.
#[derive(Clone, Debug)]
enum Frame {
    Selection(Option<String>),
    /// The arguments of `field` of `of`; `None` for a directive's or the
    /// variables' list.
    Args {
        of: Option<String>,
        field: Option<String>,
    },
    /// An object or a list inside the arguments.
    Value,
}

/// What a `{` met at the selection level opens.
#[derive(Clone, Debug, Default)]
struct Pending {
    /// `query`, `mutation`, `subscription` met at the top.
    operation: Option<String>,
    /// The type of `fragment X on T` or `... on T`.
    on: Option<String>,
    /// The last field named at this level.
    field: Option<String>,
}

/// What `text` offers at byte `offset` with `schema`.
pub fn complete(schema: &Schema, text: &str, offset: usize) -> Completions {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let from = before
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || *ch == '_')
        .last()
        .map_or(offset, |(at, _)| at);
    let prefix = &text[from..offset];
    let (tokens, open) = tokens(&text[..from]);
    // A variable or a directive is no field; a comment or a string, no
    // place for one.
    if open || matches!(text[..from].chars().next_back(), Some('$' | '@')) {
        return Completions {
            from,
            items: Vec::new(),
        };
    }

    let mut stack: Vec<Frame> = Vec::new();
    let mut pending = Pending::default();
    let mut after_on = false;
    let mut after_fragment_name = false;
    let mut in_directive = false;
    let mut last_arg: Option<String> = None;
    for (index, token) in tokens.iter().enumerate() {
        let previous = index.checked_sub(1).map(|at| &tokens[at]);
        match (stack.last().cloned(), token) {
            // The top level.
            (None, Token::Name(name)) => match name.as_str() {
                "query" | "mutation" | "subscription" => pending.operation = Some(name.clone()),
                "fragment" => after_fragment_name = true,
                "on" if after_fragment_name => after_on = true,
                _ if after_on => {
                    pending.on = Some(name.clone());
                    after_on = false;
                    after_fragment_name = false;
                }
                _ => {}
            },
            (None, Token::Punct('{')) => {
                let root = match pending.operation.as_deref() {
                    Some("mutation") => schema.mutation.clone(),
                    Some("subscription") => schema.subscription.clone(),
                    _ => schema.query.clone(),
                };
                stack.push(Frame::Selection(pending.on.take().or(root)));
                pending = Pending::default();
            }
            (None, Token::Punct('(')) => stack.push(Frame::Args { of: None, field: None }),
            // A selection set.
            (Some(Frame::Selection(_)), Token::Name(name)) => {
                if after_on {
                    pending.on = Some(name.clone());
                    after_on = false;
                } else if matches!(previous, Some(Token::Spread)) && name == "on" {
                    after_on = true;
                } else if matches!(previous, Some(Token::Spread)) || in_directive {
                    // A fragment spread, or a directive's name.
                    in_directive = false;
                    pending.field = None;
                } else {
                    // A field, or after `alias:` the field it names.
                    pending.field = Some(name.clone());
                }
            }
            (Some(Frame::Selection(_)), Token::Punct('@')) => in_directive = true,
            (Some(Frame::Selection(of)), Token::Punct('(')) => {
                let field = pending.field.clone();
                stack.push(Frame::Args { of, field });
                last_arg = None;
            }
            (Some(Frame::Selection(of)), Token::Punct('{')) => {
                let inner = match pending.on.take() {
                    Some(on) => Some(on),
                    None => of
                        .as_deref()
                        .zip(pending.field.as_deref())
                        .and_then(|(of, field)| schema.field(of, field))
                        .map(|field| field.type_ref.named.clone()),
                };
                stack.push(Frame::Selection(inner));
                pending = Pending::default();
            }
            (Some(Frame::Selection(_)), Token::Punct('}')) => {
                stack.pop();
                pending = Pending::default();
            }
            // Arguments, and the values in them.
            (Some(Frame::Args { .. }), Token::Name(name)) => {
                if !matches!(previous, Some(Token::Punct(':'))) {
                    last_arg = Some(name.clone());
                }
            }
            (Some(Frame::Args { .. }), Token::Punct(')')) => {
                stack.pop();
            }
            (Some(Frame::Args { .. } | Frame::Value), Token::Punct('{' | '[')) => stack.push(Frame::Value),
            (Some(Frame::Value), Token::Punct('}' | ']')) => {
                stack.pop();
            }
            _ => {}
        }
    }

    let matches = |label: &str| label.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase());
    let mut items = Vec::new();
    match stack.last() {
        None => {
            for keyword in ["query", "mutation", "subscription", "fragment"] {
                if matches(keyword) {
                    items.push(Suggestion {
                        label: keyword.into(),
                        detail: "keyword".into(),
                        description: String::new(),
                        kind: SuggestionKind::Keyword,
                        insert: keyword.into(),
                    });
                }
            }
        }
        Some(Frame::Selection(Some(of))) => {
            if after_on || matches!(tokens.last(), Some(Token::Spread)) {
                // `... on |`: the types the selection may be.
                let possible = schema
                    .types
                    .get(of)
                    .map(|kind| kind.possible_types.clone())
                    .unwrap_or_default();
                for name in possible.into_iter().filter(|name| matches(name)) {
                    items.push(Suggestion {
                        detail: "type".into(),
                        description: String::new(),
                        kind: SuggestionKind::Type,
                        insert: name.clone(),
                        label: name,
                    });
                }
            } else if let Some(kind) = schema.types.get(of) {
                for field in kind.fields.iter().filter(|field| matches(&field.name)) {
                    items.push(Suggestion {
                        label: field.name.clone(),
                        detail: field.type_ref.display.clone(),
                        description: field.description.clone(),
                        kind: SuggestionKind::Field,
                        insert: field.name.clone(),
                    });
                }
                if matches("__typename") {
                    items.push(Suggestion {
                        label: "__typename".into(),
                        detail: "String!".into(),
                        description: String::new(),
                        kind: SuggestionKind::Field,
                        insert: "__typename".into(),
                    });
                }
            }
        }
        Some(Frame::Args {
            of: Some(of),
            field: Some(field),
        }) => {
            let Some(field) = schema.field(of, field) else {
                return Completions { from, items };
            };
            if matches!(tokens.last(), Some(Token::Punct(':'))) {
                // A value: an enum's values, or a Boolean's.
                let arg = last_arg
                    .as_deref()
                    .and_then(|name| field.args.iter().find(|arg| arg.name == name));
                if let Some(arg) = arg {
                    let values: Vec<(String, String)> = match schema.types.get(&arg.type_ref.named) {
                        Some(kind) if kind.kind == "ENUM" => kind.enum_values.clone(),
                        _ if arg.type_ref.named == "Boolean" => {
                            vec![("true".into(), String::new()), ("false".into(), String::new())]
                        }
                        _ => Vec::new(),
                    };
                    for (value, description) in values.into_iter().filter(|(value, _)| matches(value)) {
                        items.push(Suggestion {
                            detail: arg.type_ref.display.clone(),
                            description,
                            kind: SuggestionKind::EnumValue,
                            insert: value.clone(),
                            label: value,
                        });
                    }
                }
            } else {
                for arg in field.args.iter().filter(|arg| matches(&arg.name)) {
                    items.push(Suggestion {
                        label: arg.name.clone(),
                        detail: arg.type_ref.display.clone(),
                        description: arg.description.clone(),
                        kind: SuggestionKind::Argument,
                        insert: format!("{}: ", arg.name),
                    });
                }
            }
        }
        _ => {}
    }
    Completions { from, items }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `type Query { user(id: ID!, role: Role): User, users: [User!]! }`,
    /// `type User { id: ID!, name: String, friends: [User!] }`,
    /// `enum Role { ADMIN, GUEST }`, `type Mutation { rename(name: String!): User }`.
    fn schema() -> Schema {
        let named = |name: &str| json!({"kind": "OBJECT", "name": name});
        let scalar = |name: &str| json!({"kind": "SCALAR", "name": name});
        let non_null = |inner: Value| json!({"kind": "NON_NULL", "name": null, "ofType": inner});
        let list = |inner: Value| json!({"kind": "LIST", "name": null, "ofType": inner});
        let answer = json!({"data": {"__schema": {
            "queryType": {"name": "Query"},
            "mutationType": {"name": "Mutation"},
            "subscriptionType": null,
            "types": [
                {"kind": "OBJECT", "name": "Query", "fields": [
                    {"name": "user", "description": "One user", "type": named("User"), "args": [
                        {"name": "id", "type": non_null(scalar("ID"))},
                        {"name": "role", "type": {"kind": "ENUM", "name": "Role"}},
                        {"name": "active", "type": scalar("Boolean")}
                    ]},
                    {"name": "users", "type": non_null(list(non_null(named("User")))), "args": []},
                    {"name": "node", "type": {"kind": "INTERFACE", "name": "Node"}, "args": []}
                ]},
                {"kind": "OBJECT", "name": "User", "fields": [
                    {"name": "id", "type": non_null(scalar("ID")), "args": []},
                    {"name": "name", "type": scalar("String"), "args": []},
                    {"name": "friends", "type": list(non_null(named("User"))), "args": []}
                ]},
                {"kind": "INTERFACE", "name": "Node", "fields": [{"name": "id", "type": non_null(scalar("ID")), "args": []}],
                 "possibleTypes": [{"name": "User"}]},
                {"kind": "OBJECT", "name": "Mutation", "fields": [
                    {"name": "rename", "type": named("User"), "args": [{"name": "name", "type": non_null(scalar("String"))}]}
                ]},
                {"kind": "ENUM", "name": "Role", "enumValues": [{"name": "ADMIN"}, {"name": "GUEST"}]},
                {"kind": "OBJECT", "name": "__Type", "fields": []}
            ]
        }}});
        Schema::from_introspection(answer.to_string().as_bytes()).expect("a schema")
    }

    #[test]
    fn a_browser_lists_the_operation_types_first() {
        let schema = schema();
        assert_eq!(schema.browsed_types(""), ["Query", "Mutation", "Node", "Role", "User"]);
        assert_eq!(schema.browsed_types(" u "), ["Query", "Mutation", "User"]);
        assert!(schema.browsed_types("__").is_empty(), "GraphQL's own are not listed");
    }

    fn labels(text: &str) -> Vec<String> {
        let caret = text.find('|').expect("a caret");
        let text = text.replace('|', "");
        complete(&schema(), &text, caret)
            .items
            .into_iter()
            .map(|item| item.label)
            .collect()
    }

    #[test]
    fn the_introspection_answer_reads_as_a_schema() {
        let schema = schema();
        assert_eq!(schema.query.as_deref(), Some("Query"));
        assert_eq!(schema.type_count(), 5, "not __Type");
        let users = schema.field("Query", "users").expect("users");
        assert_eq!(
            users.type_ref,
            TypeRef {
                display: "[User!]!".into(),
                named: "User".into()
            }
        );
        let refused = br#"{"errors": [{"message": "introspection is disabled"}]}"#;
        assert_eq!(
            Schema::from_introspection(refused),
            Err("introspection is disabled".into())
        );
    }

    #[test]
    fn a_selection_set_offers_its_types_fields() {
        assert_eq!(labels("{ |"), ["user", "users", "node", "__typename"]);
        assert_eq!(labels("query Me { us|"), ["user", "users"]);
        assert_eq!(labels("{ user(id: 1) { |"), ["id", "name", "friends", "__typename"]);
        assert_eq!(labels("{ users { friends { na|"), ["name"], "through lists");
        assert_eq!(
            labels("{ me: user(id: \"x{\") { i|"),
            ["id"],
            "after an alias, past a brace in a string"
        );
        assert_eq!(labels("mutation { rename(name: \"a\") { n|"), ["name"]);
        assert_eq!(
            labels("{ user { id } us|"),
            ["user", "users"],
            "back out of the inner set"
        );
        assert_eq!(labels("fragment F on User { fr|"), ["friends"]);
        assert_eq!(labels("{ node { ... on User { na|"), ["name"]);
        assert_eq!(labels("{ node { ... on |"), ["User"]);
    }

    #[test]
    fn arguments_and_their_values_are_offered() {
        assert_eq!(labels("{ user(|"), ["id", "role", "active"]);
        assert_eq!(labels("{ user(id: 1, ro|"), ["role"]);
        assert_eq!(labels("{ user(role: |"), ["ADMIN", "GUEST"]);
        assert_eq!(labels("{ user(active: t|"), ["true"]);
        assert!(labels("{ user(id: |").is_empty(), "an ID is typed");
        assert!(labels("query Q($a: |").is_empty(), "no types for variables yet");
    }

    #[test]
    fn the_top_offers_the_keywords_and_variables_offer_nothing() {
        assert_eq!(labels("|"), ["query", "mutation", "subscription", "fragment"]);
        assert_eq!(labels("mu|"), ["mutation"]);
        assert!(labels("{ user(id: $|").is_empty());
        assert_eq!(labels("# {\n|").len(), 4, "a comment is no brace");
        assert!(labels("{ # na|").is_empty(), "and no word in it is offered");
        assert!(labels("{ user(role: \"A|").is_empty(), "nor one in a string");
        assert!(labels("{ user(role: \"\"\"\n A|").is_empty(), "nor in a block string");
    }

    #[test]
    fn the_word_at_the_caret_is_what_a_choice_replaces() {
        let text = "{ use }";
        let completions = complete(&schema(), text, 5);
        assert_eq!(completions.from, 2);
        assert_eq!(completions.items[0].insert, "user");
        let args = complete(&schema(), "{ user(", 7);
        assert_eq!(args.items[0].insert, "id: ");
    }
}
