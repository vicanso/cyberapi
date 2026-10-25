//! The response filter (#63): a JSONPath (RFC 9535) narrows a JSON body to
//! what it matches, a list of every match. `$` may be left out: `data[0]`
//! and `.data[0]` are `$.data[0]`.

use jsonpath_rust::JsonPath as _;
use serde_json::Value;

/// The longest reason a path does not read that the panel shows.
const REASON: usize = 160;

/// The matches of `path` in `doc`, as a list; or why the path does not
/// read, on one line.
pub fn filter(doc: &Value, path: &str) -> Result<Value, String> {
    let path = rooted(path);
    match doc.query(&path) {
        Ok(found) => Ok(Value::Array(found.into_iter().cloned().collect())),
        Err(error) => Err(reason(&error.to_string())),
    }
}

/// Why a path does not read, on one line. The parser draws the path with
/// a caret under the place; what it expected there, and where, is enough:
/// `expected comparable at 16`.
fn reason(message: &str) -> String {
    let expected = message.rsplit_once("= ").map(|(_, expected)| expected.trim());
    let at = message
        .split_once("--> ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .and_then(|place| place.rsplit_once(':'))
        .map(|(_, column)| column);
    let reason = match (expected, at) {
        (Some(expected), Some(at)) => format!("{expected} at {at}"),
        _ => message.split_whitespace().collect::<Vec<_>>().join(" "),
    };
    reason.chars().take(REASON).collect()
}

/// `path` starting at the root.
fn rooted(path: &str) -> String {
    let path = path.trim();
    if path.starts_with('$') {
        path.to_string()
    } else if path.starts_with(['.', '[']) {
        format!("${path}")
    } else {
        format!("$.{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn doc() -> Value {
        json!({"data": [{"name": "a", "id": 1}, {"name": "b", "id": 2, "tags": ["x"]}], "total": 2})
    }

    #[test]
    fn a_path_narrows_the_body_to_its_matches() {
        assert_eq!(filter(&doc(), "$.data[*].name"), Ok(json!(["a", "b"])));
        assert_eq!(filter(&doc(), "$..id"), Ok(json!([1, 2])));
        assert_eq!(filter(&doc(), "$.data[?@.id > 1].name"), Ok(json!(["b"])));
        assert_eq!(filter(&doc(), "$.total"), Ok(json!([2])));
        assert_eq!(filter(&doc(), "$.nothing"), Ok(json!([])));
    }

    #[test]
    fn the_root_may_be_left_out() {
        assert_eq!(filter(&doc(), "data[0].name"), Ok(json!(["a"])));
        assert_eq!(filter(&doc(), ".total"), Ok(json!([2])));
        assert_eq!(filter(&doc(), "['total']"), Ok(json!([2])));
        assert_eq!(filter(&doc(), "  $.total "), Ok(json!([2])));
    }

    #[test]
    fn a_path_that_does_not_read_says_why_on_a_line() {
        assert_eq!(
            filter(&doc(), "$.data[?@.id <"),
            Err("expected comparable at 15".to_string())
        );
        let reason = filter(&doc(), "$.data[").expect_err("not a path");
        assert!(!reason.is_empty() && !reason.contains('\n'), "{reason}");
        assert!(reason.chars().count() <= REASON);
    }
}
