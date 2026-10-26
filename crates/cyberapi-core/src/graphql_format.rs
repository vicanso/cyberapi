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

/// Where the caret at `caret` in `before` goes in `after`, the same query
/// formatted (#75): after as many characters other than white space as
/// came before it, for formatting moves white space alone.
pub fn caret_after_format(before: &str, caret: usize, after: &str) -> usize {
    let wanted = before
        .get(..caret)
        .unwrap_or(before)
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .count();
    if wanted == 0 {
        return 0;
    }
    after
        .char_indices()
        .filter(|(_, ch)| !ch.is_whitespace())
        .nth(wanted - 1)
        .map_or(after.len(), |(at, ch)| at + ch.len_utf8())
}

/// Where a field picked in the schema browser goes in `query` with its
/// caret at `caret`, and what is written there (#75). In a selection set,
/// at the caret — past the word it is in, on a line of its own at the end
/// of a line. Anywhere else — before the operation, in arguments — at the
/// end of the selection set around the caret, or of the query's first one.
/// An empty query becomes a selection set of the field; one that does not
/// read, or has no selection set, takes it at the caret as a set would.
pub fn field_insertion(query: &str, caret: usize, field: &str) -> (usize, String) {
    let mut caret = caret.min(query.len());
    while !query.is_char_boundary(caret) {
        caret -= 1;
    }
    if query.trim().is_empty() {
        return (0, format!("{{\n{}{field}\n}}", " ".repeat(JSON_INDENT)));
    }
    let Ok(tokens) = lex(query) else {
        return at_the_caret(query, caret, field);
    };
    let span = |token: &Token<'_>| {
        let start = token.text.as_ptr() as usize - query.as_ptr() as usize;
        (start, start + token.text.len())
    };
    // Inside a word, a string or a comment: past it.
    if let Some((_, end)) = tokens
        .iter()
        .map(span)
        .find(|(start, end)| *start < caret && caret < *end)
    {
        caret = end;
    }
    // What is open at the caret, and where each selection set closes.
    let mut open: Vec<(Open, usize)> = Vec::new();
    let mut at_caret: Option<Vec<(Open, usize)>> = None;
    let mut closes: Vec<(usize, usize)> = Vec::new();
    let mut first_selection = None;
    for token in tokens.iter().filter(|token| token.kind == Kind::Punct) {
        let (start, _) = span(token);
        if at_caret.is_none() && start >= caret {
            at_caret = Some(open.clone());
        }
        let value = matches!(open.last(), Some((Open::Value(_), _)));
        match token.text {
            "{" if value => open.push((Open::Value('}'), start)),
            "{" => {
                first_selection = first_selection.or(Some(start));
                open.push((Open::Selection, start));
            }
            "(" => open.push((Open::Value(')'), start)),
            "[" => open.push((Open::Value(']'), start)),
            "}" | ")" | "]" => {
                if let Some((Open::Selection, opened)) = open.pop() {
                    closes.push((opened, start));
                }
            }
            _ => {}
        }
    }
    let at_caret = at_caret.unwrap_or(open);
    if matches!(at_caret.last(), Some((Open::Selection, _))) {
        return at_the_caret(query, caret, field);
    }
    let around = at_caret
        .iter()
        .rev()
        .find(|(open, _)| *open == Open::Selection)
        .map(|(_, opened)| *opened)
        .or(first_selection);
    match around.and_then(|opened| closes.iter().find(|(start, _)| *start == opened)) {
        Some((_, close)) => before_close(query, *close, field),
        None => at_the_caret(query, caret, field),
    }
}

/// `field` at the caret in a selection set, apart from the words beside it.
fn at_the_caret(query: &str, caret: usize, field: &str) -> (usize, String) {
    let line_start = query[..caret].rfind('\n').map_or(0, |at| at + 1);
    let line_end = query[caret..].find('\n').map_or(query.len(), |at| caret + at);
    let (before, after) = (&query[line_start..caret], &query[caret..line_end]);
    let indent: String = query[line_start..]
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect();
    // At the end of a line: on the next, a level in when the line opens a set.
    if !before.trim().is_empty() && after.trim().is_empty() {
        let inner = if before.trim_end().ends_with('{') {
            " ".repeat(JSON_INDENT)
        } else {
            String::new()
        };
        return (line_end, format!("\n{indent}{inner}{field}"));
    }
    // At the start of a line: on a line of its own above it.
    if before.trim().is_empty() && !after.trim().is_empty() {
        return (line_start + indent.len(), format!("{field}\n{indent}"));
    }
    let prev = query[..caret].chars().next_back();
    let next = query[caret..].chars().next();
    let lead = if prev.is_some_and(|ch| !ch.is_whitespace() && ch != '{') {
        " "
    } else {
        ""
    };
    let trail = if next.is_some_and(|ch| !ch.is_whitespace() && ch != '}') {
        " "
    } else {
        ""
    };
    (caret, format!("{lead}{field}{trail}"))
}

/// `field` last in the selection set closed at `close`.
fn before_close(query: &str, close: usize, field: &str) -> (usize, String) {
    let line_start = query[..close].rfind('\n').map_or(0, |at| at + 1);
    let lead = &query[line_start..close];
    // `}` on a line of its own: a line a level in from it, before it.
    if line_start > 0 && lead.trim().is_empty() {
        return (line_start - 1, format!("\n{lead}{}{field}", " ".repeat(JSON_INDENT)));
    }
    match query[..close].chars().next_back() {
        Some(ch) if ch.is_whitespace() => (close, format!("{field} ")),
        _ => (close, format!(" {field}")),
    }
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

    /// `query` with `field` put where `field_insertion` says.
    fn inserted(query: &str, caret: usize, field: &str) -> String {
        let (at, text) = field_insertion(query, caret, field);
        let mut query = query.to_string();
        query.insert_str(at, &text);
        query
    }

    /// #75 as reported: Format left the caret at the start, and the field
    /// went in before `query`.
    #[test]
    fn a_field_picked_with_the_caret_outside_goes_last_in_the_operation() {
        let typed = "query Q { users { id name } hello }";
        let formatted = format_query(typed).expect("formatted");
        assert_eq!(
            inserted(&formatted, 0, "email"),
            "query Q {\n    users {\n        id\n        name\n    }\n    hello\n    email\n}"
        );
        assert_eq!(
            inserted("{ users(first: 1) { id } }", 15, "email"),
            "{ users(first: 1) { id } email }",
            "in arguments: the set around them"
        );
        assert_eq!(
            inserted("query { a(x: \"{\") # {\n b }", 0, "c"),
            "query { a(x: \"{\") # {\n b c }",
            "braces in strings and comments are none"
        );
    }

    #[test]
    fn format_keeps_the_caret_where_it_was() {
        let typed = "query Q { users { id name } hello }";
        let formatted = format_query(typed).expect("formatted");
        let after_name = typed.find("name").expect("there") + "name".len();
        let caret = caret_after_format(typed, after_name, &formatted);
        assert_eq!(&formatted[..caret], "query Q {\n    users {\n        id\n        name");
        assert_eq!(caret_after_format(typed, 0, &formatted), 0);
        assert_eq!(caret_after_format(typed, typed.len(), &formatted), formatted.len());
        // And the field picked there goes on the next line, in the same set.
        assert_eq!(
            inserted(&formatted, caret, "email"),
            "query Q {\n    users {\n        id\n        name\n        email\n    }\n    hello\n}"
        );
    }

    #[test]
    fn a_field_picked_in_a_selection_set_goes_at_the_caret_apart_from_its_neighbours() {
        let query = "query Q {\n    users {\n        id\n        name\n    }\n}";
        let in_name = query.find("name").expect("there") + 2;
        assert_eq!(
            inserted(query, in_name, "email"),
            "query Q {\n    users {\n        id\n        name\n        email\n    }\n}",
            "inside a word: past it"
        );
        let after_open = query.find("users {").expect("there") + "users {".len();
        assert_eq!(
            inserted(query, after_open, "email"),
            "query Q {\n    users {\n        email\n        id\n        name\n    }\n}",
            "after an opening brace: a level in"
        );
        let line_start = query.find("        id").expect("there");
        assert_eq!(
            inserted(query, line_start, "email"),
            "query Q {\n    users {\n        email\n        id\n        name\n    }\n}",
            "before a line's field: a line of its own"
        );
        assert_eq!(inserted("{ a b }", 3, "c"), "{ a c b }", "one line: spaced");
        assert_eq!(inserted("{a}", 2, "c"), "{a c}");
    }

    #[test]
    fn an_empty_query_becomes_a_selection_set_and_a_broken_one_takes_it_as_it_is() {
        assert_eq!(inserted("", 0, "hello"), "{\n    hello\n}");
        assert_eq!(inserted("  \n", 1, "hello"), "{\n    hello\n}  \n");
        assert_eq!(inserted("{ a \"b", 2, "c"), "{ c a \"b");
    }
}
