//! D4: two responses of an API's history compared, their bodies line by
//! line. A JSON body is compared as the panel shows it, indented, so a
//! changed value is a changed line, not the whole body; any other is its
//! text. What did not change is folded away but for a few lines around
//! each change.
//!
//! The comparison runs where the page runs too, on its one thread, and
//! takes a time that grows with how different the two are: each side is
//! cut at [`LINE_LIMIT`] lines and [`BYTE_LIMIT`] bytes first, and the
//! comparison says when it was.

use crate::response::HttpResponse;
use similar::{Algorithm, ChangeTag, TextDiff};

/// The most lines of each body compared.
pub const LINE_LIMIT: usize = 20_000;
/// The most of each body compared.
pub const BYTE_LIMIT: usize = 2 * 1024 * 1024;
/// The unchanged lines shown on each side of a change.
pub const CONTEXT: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Same,
    Removed,
    Added,
}

/// A line of the comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A line, with its number in the older body, the newer, or both.
    Line {
        change: Change,
        old: Option<usize>,
        new: Option<usize>,
        text: String,
    },
    /// So many unchanged lines, folded.
    Folded(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compared {
    pub rows: Vec<Row>,
    pub added: usize,
    pub removed: usize,
    /// A body was longer than what is compared of it.
    pub cut: bool,
}

impl Compared {
    /// The two are the same, as far as they were compared.
    pub fn same(&self) -> bool {
        self.added == 0 && self.removed == 0
    }
}

/// A response's body as it is compared: JSON indented as the panel shows
/// it, any other text as it came.
pub fn body_text(response: &HttpResponse) -> String {
    match serde_json::from_slice::<serde_json::Value>(&response.body) {
        Ok(json) if !response.body.is_empty() => crate::format::json(&json),
        _ => String::from_utf8_lossy(&response.body).into_owned(),
    }
}

/// `text` within the limits, and whether it had to be cut.
fn within_limits(text: &str) -> (&str, bool) {
    let mut end = text.len().min(BYTE_LIMIT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if let Some((at, _)) = text[..end].match_indices('\n').nth(LINE_LIMIT - 1) {
        end = at + 1;
    }
    (&text[..end], end < text.len())
}

/// The lines of `new` against those of `old`.
pub fn compare(old: &str, new: &str) -> Compared {
    let (old, old_cut) = within_limits(old);
    let (new, new_cut) = within_limits(new);
    let diff = TextDiff::configure().algorithm(Algorithm::Myers).diff_lines(old, new);
    let mut compared = Compared {
        cut: old_cut || new_cut,
        ..Default::default()
    };
    // The older body's next line not yet shown.
    let mut next = 0;
    for group in diff.grouped_ops(CONTEXT) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        if first.old_range().start > next {
            compared.rows.push(Row::Folded(first.old_range().start - next));
        }
        for op in &group {
            for change in diff.iter_changes(op) {
                let change_of = match change.tag() {
                    ChangeTag::Equal => Change::Same,
                    ChangeTag::Delete => {
                        compared.removed += 1;
                        Change::Removed
                    }
                    ChangeTag::Insert => {
                        compared.added += 1;
                        Change::Added
                    }
                };
                compared.rows.push(Row::Line {
                    change: change_of,
                    old: change.old_index().map(|at| at + 1),
                    new: change.new_index().map(|at| at + 1),
                    text: change.value().trim_end_matches(['\n', '\r']).to_string(),
                });
            }
        }
        next = last.old_range().end;
    }
    let lines = diff.old_slices().len();
    if !compared.rows.is_empty() && lines > next {
        compared.rows.push(Row::Folded(lines - next));
    }
    compared
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn line(change: Change, old: Option<usize>, new: Option<usize>, text: &str) -> Row {
        Row::Line {
            change,
            old,
            new,
            text: text.into(),
        }
    }

    #[test]
    fn a_change_shows_with_the_lines_around_it() {
        let old: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        let new = old
            .replace("line 10\n", "line ten\n")
            .replace("line 20\n", "line 20\nline 21\n");
        let compared = compare(&old, &new);
        assert_eq!((compared.added, compared.removed, compared.cut), (2, 1, false));
        assert_eq!(compared.rows[0], Row::Folded(6));
        assert_eq!(compared.rows[1], line(Change::Same, Some(7), Some(7), "line 7"));
        assert_eq!(compared.rows[4], line(Change::Removed, Some(10), None, "line 10"));
        assert_eq!(compared.rows[5], line(Change::Added, None, Some(10), "line ten"));
        assert_eq!(
            compared.rows[9],
            Row::Folded(4),
            "lines 14 to 17, between the two changes"
        );
        assert_eq!(
            compared.rows.last(),
            Some(&line(Change::Added, None, Some(21), "line 21"))
        );
        let same = compare(&old, &old);
        assert!(same.same() && same.rows.is_empty());
    }

    #[test]
    fn a_json_body_is_compared_indented() {
        let response = |body: &str| HttpResponse {
            body: body.as_bytes().to_vec(),
            ..Default::default()
        };
        let old = body_text(&response(r#"{"a":1,"b":[1,2]}"#));
        let new = body_text(&response(r#"{"a":2,"b":[1,2]}"#));
        assert_eq!(old, "{\n    \"a\": 1,\n    \"b\": [\n        1,\n        2\n    ]\n}");
        let compared = compare(&old, &new);
        assert_eq!((compared.added, compared.removed), (1, 1), "{:?}", compared.rows);
        assert_eq!(body_text(&response("not json")), "not json");
        assert_eq!(body_text(&response("")), "");
    }

    #[test]
    fn a_long_body_is_cut_and_says_so() {
        let long: String = (0..LINE_LIMIT + 5).map(|n| format!("{n}\n")).collect();
        let (kept, cut) = within_limits(&long);
        assert!(cut);
        assert_eq!(kept.lines().count(), LINE_LIMIT);
        assert!(compare(&long, "0\n").cut);
        let wide = "网".repeat(BYTE_LIMIT);
        let (kept, cut) = within_limits(&wide);
        assert!(cut && kept.len() <= BYTE_LIMIT);
    }
}
