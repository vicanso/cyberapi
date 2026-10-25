//! A GraphQL query indented (#70): each field on a line of its own, a
//! selection's fields a level in from it, arguments, variables and values
//! on the line they belong to, a blank line between definitions. Comments
//! stay where they were, the commas the reader wrote too; a template
//! expression (`{{value(id)}}`) is one word. A query that is not complete —
//! a bracket not closed, a string not ended — is not formatted: half a
//! query would be moved about as if it were whole.

use crate::format::JSON_INDENT;

/// Why a query is not formatted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unformatted {
    /// A string, a block string or a template expression not ended.
    StringNotClosed,
    /// A bracket not closed, or one closing what it did not open.
    Brackets,
    /// A character GraphQL has no use for.
    NotGraphQl(char),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Punct,
    Name,
    /// A number, a string or a template expression: a value as written.
    Value,
    Comment,
}

#[derive(Clone, Debug)]
struct Token<'a> {
    kind: Kind,
    text: &'a str,
    /// A comment with nothing before it on its line.
    own_line: bool,
}

const PUNCTS: [&str; 13] = ["!", "$", "&", "(", ")", ":", "=", "@", "[", "]", "{", "|", "}"];

fn lex(text: &str) -> Result<Vec<Token<'_>>, Unformatted> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let bytes = text.as_bytes();
    let mut tokens: Vec<Token<'_>> = Vec::new();
    let mut at = 0;
    let mut line_start = true;
    while at < bytes.len() {
        let rest = &text[at..];
        let byte = bytes[at];
        let start = at;
        let kind = match byte {
            b'\n' => {
                line_start = true;
                at += 1;
                continue;
            }
            b' ' | b'\t' | b'\r' => {
                at += 1;
                continue;
            }
            b'#' => {
                at += rest.find('\n').unwrap_or(rest.len());
                Kind::Comment
            }
            b',' => {
                at += 1;
                Kind::Punct
            }
            b'{' if rest.starts_with("{{") => {
                let end = rest.find("}}").ok_or(Unformatted::StringNotClosed)?;
                at += end + 2;
                Kind::Value
            }
            b'"' if rest.starts_with("\"\"\"") => {
                let mut end = 3;
                loop {
                    let found = rest[end..].find("\"\"\"").ok_or(Unformatted::StringNotClosed)?;
                    let close = end + found;
                    // `\"""` is the text, not the end.
                    if rest[..close].ends_with('\\') {
                        end = close + 3;
                        continue;
                    }
                    at += close + 3;
                    break;
                }
                Kind::Value
            }
            b'"' => {
                let mut escaped = false;
                let mut end = None;
                for (offset, ch) in rest.char_indices().skip(1) {
                    match ch {
                        '\n' => break,
                        '\\' if !escaped => escaped = true,
                        '"' if !escaped => {
                            end = Some(offset + 1);
                            break;
                        }
                        _ => escaped = false,
                    }
                }
                at += end.ok_or(Unformatted::StringNotClosed)?;
                Kind::Value
            }
            b'.' if rest.starts_with("...") => {
                at += 3;
                Kind::Punct
            }
            b'-' | b'0'..=b'9' => {
                at += 1;
                while at < bytes.len() && matches!(bytes[at], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
                    at += 1;
                }
                Kind::Value
            }
            b'_' | b'a'..=b'z' | b'A'..=b'Z' => {
                while at < bytes.len() && matches!(bytes[at], b'_' | b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z') {
                    at += 1;
                }
                Kind::Name
            }
            _ if PUNCTS.iter().any(|punct| rest.starts_with(punct)) => {
                at += 1;
                Kind::Punct
            }
            _ => {
                return Err(Unformatted::NotGraphQl(rest.chars().next().unwrap_or_default()));
            }
        };
        tokens.push(Token {
            own_line: kind == Kind::Comment && line_start,
            kind,
            text: &text[start..at],
        });
        line_start = false;
    }
    Ok(tokens)
}

/// What a bracket opened.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    /// A selection set: a field a line.
    Selection,
    /// Arguments, a list or an input object, closed by this: on one line.
    Value(char),
}

struct Printer {
    lines: Vec<String>,
    line: String,
    /// The indent of the line being written.
    line_indent: usize,
    indent: usize,
}

impl Printer {
    fn put(&mut self, text: &str, space: bool) {
        if self.line.is_empty() {
            self.line_indent = self.indent;
        } else if space {
            self.line.push(' ');
        }
        self.line.push_str(text);
    }

    fn flush(&mut self) {
        if !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            self.lines
                .push(format!("{}{line}", " ".repeat(self.line_indent * JSON_INDENT)));
        }
    }
}

/// `query` indented; or why it is not formatted.
pub fn format_query(query: &str) -> Result<String, Unformatted> {
    let tokens = lex(query)?;
    let mut printer = Printer {
        lines: Vec::new(),
        line: String::new(),
        line_indent: 0,
        indent: 0,
    };
    let mut open: Vec<Open> = Vec::new();
    // A definition just ended: the next starts after a blank line.
    let mut ended = false;
    let mut previous: Option<&Token<'_>> = None;
    let mut before_previous: Option<&Token<'_>> = None;
    for token in &tokens {
        let prev = previous.map(|token| token.text);
        if token.kind == Kind::Comment {
            if token.own_line {
                printer.flush();
                if ended {
                    printer.lines.push(String::new());
                    ended = false;
                }
            }
            printer.put(token.text, true);
            printer.flush();
            continue;
        }
        if ended {
            printer.flush();
            printer.lines.push(String::new());
            ended = false;
        }
        let value = matches!(open.last(), Some(Open::Value(_)));
        let selection = open.last() == Some(&Open::Selection);
        // Where a field (or a spread) starts in a selection set.
        let starts_field = selection
            && (token.text == "..."
                || (matches!(token.kind, Kind::Name | Kind::Value)
                    && !matches!(prev, Some("..." | "@" | ":" | "$"))
                    && !(prev == Some("on") && before_previous.map(|token| token.text) == Some("..."))));
        if starts_field {
            printer.flush();
        }
        let space = match (prev, token.text) {
            (None, _) => false,
            (_, ")" | "]" | "," | ":" | "!" | "(") => false,
            (Some("(" | "[" | "$" | "@"), _) => false,
            (Some("..."), text) => text == "on",
            (Some("{"), _) if value => false,
            (_, "}") if value => false,
            _ => true,
        };
        match token.text {
            "{" if token.kind == Kind::Punct && value => {
                printer.put("{", space);
                open.push(Open::Value('}'));
            }
            "{" if token.kind == Kind::Punct => {
                printer.put("{", true);
                printer.flush();
                open.push(Open::Selection);
                printer.indent += 1;
            }
            "}" if token.kind == Kind::Punct => match open.pop() {
                Some(Open::Value('}')) => printer.put("}", false),
                Some(Open::Selection) => {
                    printer.flush();
                    printer.indent -= 1;
                    printer.put("}", false);
                    printer.flush();
                    ended = open.is_empty();
                }
                _ => return Err(Unformatted::Brackets),
            },
            "(" | "[" if token.kind == Kind::Punct => {
                printer.put(token.text, space);
                open.push(Open::Value(if token.text == "(" { ')' } else { ']' }));
            }
            ")" | "]" if token.kind == Kind::Punct => {
                let close = token.text.chars().next().unwrap_or_default();
                if open.pop() != Some(Open::Value(close)) {
                    return Err(Unformatted::Brackets);
                }
                printer.put(token.text, false);
            }
            _ => printer.put(token.text, space),
        }
        before_previous = previous;
        previous = Some(token);
    }
    if !open.is_empty() {
        return Err(Unformatted::Brackets);
    }
    printer.flush();
    Ok(printer.lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn a_query_is_indented_a_field_a_line() {
        let query = "query Q($id: ID!,$n:Int=2){user(id:$id){name friends(first:$n){edges{node{id}}}}}";
        let formatted = "\
query Q($id: ID!, $n: Int = 2) {
    user(id: $id) {
        name
        friends(first: $n) {
            edges {
                node {
                    id
                }
            }
        }
    }
}";
        assert_eq!(format_query(query).as_deref(), Ok(formatted));
        assert_eq!(format_query(formatted).as_deref(), Ok(formatted), "formatted already");
    }

    #[test]
    fn spreads_directives_values_and_comments_keep_their_place() {
        let query = r#"{me{...F ... on User{admin:isAdmin @include(if:$x)} posts(filter:{tags:["a","b"],min:1}){id}}}
# the fragment
fragment F on User{id # its id
name}"#;
        let formatted = r#"{
    me {
        ...F
        ... on User {
            admin: isAdmin @include(if: $x)
        }
        posts(filter: {tags: ["a", "b"], min: 1}) {
            id
        }
    }
}

# the fragment
fragment F on User {
    id # its id
    name
}"#;
        assert_eq!(format_query(query).as_deref(), Ok(formatted));
        assert_eq!(format_query(formatted).as_deref(), Ok(formatted));
    }

    #[test]
    fn strings_and_template_expressions_are_kept_whole() {
        let query = r#"{ user(id: {{value(id)}}, note: "a { ( \" b", bio: """x } y""") { name } }"#;
        assert_eq!(
            format_query(query).as_deref(),
            Ok(r#"{
    user(id: {{value(id)}}, note: "a { ( \" b", bio: """x } y""") {
        name
    }
}"#)
        );
    }

    #[test]
    fn half_a_query_is_left_alone() {
        assert_eq!(format_query("{ user(id: 1) {"), Err(Unformatted::Brackets));
        assert_eq!(format_query("{ user(id: 1 }"), Err(Unformatted::Brackets));
        assert_eq!(format_query("{ a } }"), Err(Unformatted::Brackets));
        assert_eq!(format_query("{ user(name: \"tr) }"), Err(Unformatted::StringNotClosed));
        assert_eq!(format_query("{ a(b: {{value(x) }"), Err(Unformatted::StringNotClosed));
        assert_eq!(format_query("{ a ^ }"), Err(Unformatted::NotGraphQl('^')));
        assert_eq!(format_query("").as_deref(), Ok(""));
    }
}
