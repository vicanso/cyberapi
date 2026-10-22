//! A JSON document as a tree of rows, for showing it instead of its text.
//!
//! Read only: a row's path is how its value is copied. A container shows at
//! most [`MAX_CHILDREN`] members, and nesting stops at [`MAX_DEPTH`], so a
//! huge document stays a tree rather than a freeze.

use serde_json::Value;

/// One step of a path into a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

/// What a value is, as the tree draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonNodeKind {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
}

impl JsonNodeKind {
    pub fn of(value: &Value) -> Self {
        match value {
            Value::Object(_) => Self::Object,
            Value::Array(_) => Self::Array,
            Value::String(_) => Self::String,
            Value::Number(_) => Self::Number,
            Value::Bool(_) => Self::Bool,
            Value::Null => Self::Null,
        }
    }

    pub fn is_container(self) -> bool {
        matches!(self, Self::Object | Self::Array)
    }
}

/// One row of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonTreeNode {
    /// JSONPath of this node: `$`, `$.name`, `$['odd key']`, `$[0]`.
    pub path: String,
    /// The member name, `[index]`, or `$` at the root.
    pub name: String,
    pub kind: JsonNodeKind,
    /// A scalar rendered for display, quoted and clipped for a string.
    /// Empty for a container that still has room to expand.
    pub preview: String,
    /// Members of a container, `0` for a scalar.
    pub len: usize,
    pub children: Vec<JsonTreeNode>,
    /// Members past [`MAX_CHILDREN`] that got no row.
    pub omitted: usize,
}

/// A container shows at most this many members.
pub const MAX_CHILDREN: usize = 500;

/// Characters of a string a row shows before clipping.
pub const PREVIEW_CHARS: usize = 80;

/// Nesting deeper than this is one row, not another level.
pub const MAX_DEPTH: usize = 32;

/// The tree of `doc`, rooted at `$`.
pub fn json_tree(doc: &Value) -> JsonTreeNode {
    let mut segments = Vec::new();
    build_node(doc, "$".to_string(), &mut segments)
}

/// The value at `path`, when the path is one [`json_tree`] renders and
/// something is there.
pub fn value_at<'a>(doc: &'a Value, path: &str) -> Option<&'a Value> {
    resolve(doc, &parse_path(path)?)
}

/// `segments` as a JSONPath. A key that is not a plain identifier goes in
/// brackets, so the path still names that one member.
pub fn render_path(segments: &[PathSegment]) -> String {
    let mut path = String::from("$");
    for segment in segments {
        match segment {
            PathSegment::Key(key) if is_identifier(key) => {
                path.push('.');
                path.push_str(key);
            }
            PathSegment::Key(key) => {
                path.push_str("['");
                for c in key.chars() {
                    match c {
                        '\\' => path.push_str("\\\\"),
                        '\'' => path.push_str("\\'"),
                        c => path.push(c),
                    }
                }
                path.push_str("']");
            }
            PathSegment::Index(index) => {
                path.push('[');
                path.push_str(&index.to_string());
                path.push(']');
            }
        }
    }
    path
}

fn build_node(value: &Value, name: String, segments: &mut Vec<PathSegment>) -> JsonTreeNode {
    let kind = JsonNodeKind::of(value);
    let path = render_path(segments);
    let capped = segments.len() >= MAX_DEPTH;
    let (len, children, omitted, preview) = match value {
        Value::Object(map) if !capped => {
            let mut children = Vec::with_capacity(map.len().min(MAX_CHILDREN));
            for (key, child) in map.iter().take(MAX_CHILDREN) {
                segments.push(PathSegment::Key(key.clone()));
                children.push(build_node(child, key.clone(), segments));
                segments.pop();
            }
            (
                map.len(),
                children,
                map.len().saturating_sub(MAX_CHILDREN),
                String::new(),
            )
        }
        Value::Array(items) if !capped => {
            let mut children = Vec::with_capacity(items.len().min(MAX_CHILDREN));
            for (index, child) in items.iter().enumerate().take(MAX_CHILDREN) {
                segments.push(PathSegment::Index(index));
                children.push(build_node(child, format!("[{index}]"), segments));
                segments.pop();
            }
            (
                items.len(),
                children,
                items.len().saturating_sub(MAX_CHILDREN),
                String::new(),
            )
        }
        _ => (0, Vec::new(), 0, preview(value)),
    };
    JsonTreeNode {
        path,
        name,
        kind,
        preview,
        len,
        children,
        omitted,
    }
}

fn preview(value: &Value) -> String {
    match value {
        Value::Object(_) => "{…}".to_string(),
        Value::Array(_) => "[…]".to_string(),
        Value::String(text) => {
            let clipped: String = text.chars().take(PREVIEW_CHARS).collect();
            let mut shown = serde_json::to_string(&clipped).unwrap_or_default();
            if clipped.chars().count() < text.chars().count() {
                shown.pop();
                shown.push('…');
                shown.push('"');
            }
            shown
        }
        other => other.to_string(),
    }
}

fn is_identifier(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The segments of a path this module renders, or `None` for anything else.
pub fn parse_path(path: &str) -> Option<Vec<PathSegment>> {
    let mut rest = path.trim();
    rest = rest.strip_prefix('$').unwrap_or(rest);
    let mut segments = Vec::new();
    while !rest.is_empty() {
        if let Some(after_dot) = rest.strip_prefix('.') {
            let end = after_dot.find(['.', '[']).unwrap_or(after_dot.len());
            let name = &after_dot[..end];
            if name.is_empty() || name == "*" || name.starts_with('.') {
                return None;
            }
            segments.push(PathSegment::Key(name.to_string()));
            rest = &after_dot[end..];
        } else {
            let after_bracket = rest.strip_prefix('[')?;
            let (segment, consumed) = parse_bracket(after_bracket)?;
            segments.push(segment);
            rest = &after_bracket[consumed..];
        }
    }
    Some(segments)
}

/// One `[...]` segment. `text` starts just after the `[`. Answers the
/// segment and how many bytes of `text` it used, the closing bracket included.
fn parse_bracket(text: &str) -> Option<(PathSegment, usize)> {
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    if first == '\'' || first == '"' {
        let mut name = String::new();
        let mut escaped = false;
        for (at, c) in chars {
            match (escaped, c) {
                (true, c) => {
                    name.push(c);
                    escaped = false;
                }
                (false, '\\') => escaped = true,
                (false, c) if c == first => {
                    let after_quote = &text[at + c.len_utf8()..];
                    let close = after_quote.strip_prefix(']')?;
                    let consumed = text.len() - close.len();
                    return Some((PathSegment::Key(name), consumed));
                }
                (false, c) => name.push(c),
            }
        }
        return None;
    }
    let close = text.find(']')?;
    let index: usize = text[..close].trim().parse().ok()?;
    Some((PathSegment::Index(index), close + 1))
}

fn resolve<'a>(doc: &'a Value, segments: &[PathSegment]) -> Option<&'a Value> {
    let mut current = doc;
    for segment in segments {
        current = match segment {
            PathSegment::Key(key) => current.as_object()?.get(key)?,
            PathSegment::Index(index) => current.as_array()?.get(*index)?,
        };
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_document_is_a_tree_of_paths() {
        let doc = json!({"user": {"first name": "Ada", "n": 2}, "tags": [true, null]});
        let tree = json_tree(&doc);
        assert_eq!(tree.path, "$");
        assert_eq!(tree.kind, JsonNodeKind::Object);
        assert_eq!(tree.len, 2);
        assert_eq!(tree.children[0].path, "$.user");
        assert_eq!(tree.children[0].children[0].path, "$.user['first name']");
        assert_eq!(tree.children[0].children[0].preview, "\"Ada\"");
        assert_eq!(tree.children[0].children[1].preview, "2");
        assert_eq!(tree.children[1].children[0].name, "[0]");
        assert_eq!(tree.children[1].children[0].path, "$.tags[0]");
        assert_eq!(tree.children[1].children[1].kind, JsonNodeKind::Null);

        assert_eq!(value_at(&doc, "$.user['first name']"), Some(&json!("Ada")));
        assert_eq!(value_at(&doc, "$.tags[1]"), Some(&Value::Null));
        assert_eq!(value_at(&doc, "$"), Some(&doc));
        assert_eq!(value_at(&doc, "$.missing"), None);
        assert_eq!(value_at(&doc, "$.tags[*]"), None);
    }

    #[test]
    fn a_wide_container_and_a_long_string_are_cut() {
        let mut items = Vec::new();
        for n in 0..(MAX_CHILDREN + 3) {
            items.push(json!(n));
        }
        let tree = json_tree(&Value::Array(items));
        assert_eq!(tree.children.len(), MAX_CHILDREN);
        assert_eq!(tree.omitted, 3);
        assert_eq!(tree.children.last().map(|node| node.name.as_str()), Some("[499]"));

        let long = "字".repeat(PREVIEW_CHARS + 4);
        let shown = json_tree(&Value::String(long)).preview;
        assert!(shown.ends_with("…\""), "{shown}");
        assert!(shown.starts_with('"'));
    }

    #[test]
    fn nesting_stops_at_the_depth_cap() {
        let mut doc = json!(1);
        for _ in 0..MAX_DEPTH + 4 {
            doc = json!({"a": doc});
        }
        let tree = json_tree(&doc);
        fn depth(node: &JsonTreeNode) -> usize {
            1 + node.children.iter().map(depth).max().unwrap_or(0)
        }
        assert!(depth(&tree) <= MAX_DEPTH + 1, "depth {}", depth(&tree));
        fn leaf(node: &JsonTreeNode) -> &JsonTreeNode {
            node.children.first().map(leaf).unwrap_or(node)
        }
        let capped = leaf(&tree);
        assert_eq!(capped.preview, "{…}");
        assert!(capped.children.is_empty());
        assert_eq!(capped.len, 0);
    }
}
