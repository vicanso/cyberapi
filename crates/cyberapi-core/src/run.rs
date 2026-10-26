//! A3: a run — every API of a folder or of the project, in the tree's
//! order, sent one after another, once or once for each row of a data file
//! (CSV with a header row, or a JSON array of objects). A row's values, and
//! what an API takes from its answer (A2, `assertions::Extraction`) for the
//! ones after it in the same pass, are read as custom variables
//! (`{{value(name)}}`) and as a URI's `{{name}}` prefix, in place of the
//! project's of the same name. They are the run's alone: nothing is written
//! to the project. What a run sent is kept in the history as any send is;
//! the run itself is only shown.

use crate::ItemId;
use crate::assertions::{Outcomes, tally};
use crate::model::{Variable, VariableCategory};
use crate::response::HttpResponse;
use crate::tree::{ItemKind, Tree, TreeData};
use serde_json::Value;
use snafu::Snafu;

/// A run's values, by name, in the order they were given; a later one of a
/// name stands for an earlier.
pub type Values = Vec<(String, String)>;

/// The APIs a run of `folder`, or of the whole project, sends, in the
/// tree's order: depth first, each folder's in its order.
pub fn plan(data: &TreeData, folder: Option<&ItemId>) -> Vec<ItemId> {
    let tree = Tree::build(data);
    let ids = match folder {
        Some(folder) => tree.descendants(folder),
        None => tree
            .top_level_ids()
            .into_iter()
            .flat_map(|top| std::iter::once(top.clone()).chain(tree.descendants(&top)))
            .collect(),
    };
    ids.into_iter()
        .filter(|id| tree.get(id).is_some_and(|node| node.kind() == ItemKind::Api))
        .collect()
}

/// `value` set as `name`: one given before of that name gives way.
pub fn set(values: &mut Values, name: &str, value: String) {
    values.retain(|(given, _)| given != name);
    values.push((name.to_string(), value));
}

/// The project's variables (already of the reader's environment) with a
/// run's `values` in place of the custom and environment variables of the
/// same name, and added as both where there is none.
pub fn with_values(variables: &[Variable], values: &[(String, String)]) -> Vec<Variable> {
    let given = |variable: &Variable| {
        matches!(
            variable.category,
            VariableCategory::Custom | VariableCategory::Environment
        ) && values.iter().any(|(name, _)| *name == variable.name)
    };
    let mut out: Vec<Variable> = variables.iter().filter(|variable| !given(variable)).cloned().collect();
    for (name, value) in values {
        for category in [VariableCategory::Environment, VariableCategory::Custom] {
            out.push(variable(category, name, value));
        }
    }
    out
}

/// A variable of no project's, as a run gives one.
fn variable(category: VariableCategory, name: &str, value: &str) -> Variable {
    Variable {
        id: Default::default(),
        collection: Default::default(),
        category,
        name: name.to_string(),
        value: value.to_string(),
        enabled: true,
        environment: None,
        created_at: Default::default(),
        updated_at: Default::default(),
    }
}

#[derive(Debug, Snafu, PartialEq, Eq)]
pub enum DataError {
    #[snafu(display("not JSON: {message}"))]
    Json { message: String },
    #[snafu(display("a JSON data file is an array of objects"))]
    NotRows,
    #[snafu(display("a quote opened on line {line} is not closed"))]
    Quote { line: usize },
    #[snafu(display("line {line} has {found} values; the header names {named}"))]
    Width { line: usize, found: usize, named: usize },
}

/// A data file's rows, each its values by name: a JSON array of objects (a
/// value that is not a string as JSON), or CSV whose first line names the
/// columns. Blank text has no rows: the run goes once.
pub fn rows(text: &str) -> Result<Vec<Values>, DataError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed.starts_with('[') {
        let json: Value = serde_json::from_str(trimmed).map_err(|error| DataError::Json {
            message: error.to_string(),
        })?;
        let Value::Array(items) = json else {
            return NotRowsSnafu.fail();
        };
        return items
            .into_iter()
            .map(|item| match item {
                Value::Object(map) => Ok(map
                    .into_iter()
                    .map(|(name, value)| {
                        let value = match value {
                            Value::String(text) => text,
                            other => other.to_string(),
                        };
                        (name, value)
                    })
                    .collect()),
                _ => NotRowsSnafu.fail(),
            })
            .collect();
    }
    let mut records = csv(trimmed)?.into_iter();
    let Some((_, header)) = records.next() else {
        return Ok(Vec::new());
    };
    let names: Vec<String> = header.into_iter().map(|name| name.trim().to_string()).collect();
    records
        .filter(|(_, record)| !(record.len() == 1 && record[0].is_empty()))
        .map(|(line, record)| {
            if record.len() != names.len() {
                return WidthSnafu {
                    line,
                    found: record.len(),
                    named: names.len(),
                }
                .fail();
            }
            Ok(names.iter().cloned().zip(record).collect())
        })
        .collect()
}

/// RFC 4180: records of fields split by `,`; a field in `"` may hold `,`,
/// line breaks and `""` for a quote. Each record with the line it starts
/// on.
fn csv(text: &str) -> Result<Vec<(usize, Vec<String>)>, DataError> {
    let mut records = Vec::new();
    let (mut record, mut field) = (Vec::new(), String::new());
    let (mut line, mut started) = (1, 1);
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if field.is_empty() => quoted = true,
            ',' if !quoted => record.push(std::mem::take(&mut field)),
            '\r' if !quoted && chars.peek() == Some(&'\n') => {}
            '\n' if !quoted => {
                record.push(std::mem::take(&mut field));
                records.push((started, std::mem::take(&mut record)));
                line += 1;
                started = line;
            }
            c => {
                if c == '\n' {
                    line += 1;
                }
                field.push(c);
            }
        }
    }
    if quoted {
        return QuoteSnafu { line: started }.fail();
    }
    record.push(field);
    records.push((started, record));
    Ok(records)
}

/// Whether one send of a run held: it was answered, and every assertion it
/// checked held — or, with none to check, it was no error: below 400, a gRPC
/// call's status OK. Asking for a 404 is what an assertion is for.
pub fn held(response: &HttpResponse, outcomes: &Outcomes) -> bool {
    let Some(status) = response.status.filter(|_| response.error.is_none()) else {
        return false;
    };
    if !outcomes.is_empty() {
        let (passed, total) = tally(outcomes);
        return passed == total;
    }
    match response.grpc_status() {
        Some(grpc) => grpc.code == 0,
        None => status < 400,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CollectionId;
    use crate::model::{ApiCategory, ApiSetting, Folder};
    use pretty_assertions::assert_eq;

    fn values(pairs: &[(&str, &str)]) -> Values {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn a_run_goes_in_the_trees_order() {
        let api = |id: &str| ApiSetting {
            id: ItemId::new(id),
            collection: CollectionId::new("c"),
            name: id.into(),
            category: ApiCategory::Http,
            request: Default::default(),
            created_at: Default::default(),
            updated_at: Default::default(),
        };
        let folder = |id: &str, children: &[&str]| Folder {
            id: ItemId::new(id),
            collection: CollectionId::new("c"),
            name: id.into(),
            children: children.iter().map(|child| ItemId::new(*child)).collect(),
            settings: Default::default(),
            created_at: Default::default(),
            updated_at: Default::default(),
        };
        let data = TreeData {
            folders: vec![folder("f", &["a", "g"]), folder("g", &["b"])],
            apis: vec![api("a"), api("b"), api("c")],
            top_order: vec![ItemId::new("c"), ItemId::new("f")],
            settings: Default::default(),
        };
        let names = |ids: Vec<ItemId>| ids.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        assert_eq!(names(plan(&data, None)), ["c", "a", "b"]);
        assert_eq!(names(plan(&data, Some(&ItemId::new("f")))), ["a", "b"]);
        assert!(plan(&data, Some(&ItemId::new("missing"))).is_empty());
    }

    #[test]
    fn a_runs_values_stand_for_the_projects() {
        let project = [
            variable(VariableCategory::Environment, "HOST", "https://a.test"),
            variable(VariableCategory::Custom, "token", "old"),
            variable(VariableCategory::GlobalRequestHeader, "token", "header"),
        ];
        let mut given = values(&[("token", "first")]);
        set(&mut given, "id", "7".into());
        set(&mut given, "token", "new".into());
        assert_eq!(given, values(&[("id", "7"), ("token", "new")]));
        let run = with_values(&project, &given);
        let of = |category| -> Vec<(&str, &str)> {
            run.iter()
                .filter(|variable| variable.category == category)
                .map(|variable| (variable.name.as_str(), variable.value.as_str()))
                .collect()
        };
        assert_eq!(of(VariableCategory::Custom), [("id", "7"), ("token", "new")]);
        assert_eq!(
            of(VariableCategory::Environment),
            [("HOST", "https://a.test"), ("id", "7"), ("token", "new")]
        );
        assert_eq!(
            of(VariableCategory::GlobalRequestHeader),
            [("token", "header")],
            "a header is not a value"
        );
    }

    #[test]
    fn a_send_holds_by_its_assertions_or_else_its_status() {
        use crate::assertions::{Checked, Problem};
        let answered = |status: u16| HttpResponse {
            status: Some(status),
            ..Default::default()
        };
        let checked = |passed: bool| -> Outcomes { vec![(0, Ok(Checked { passed, actual: None }))] };
        assert!(held(&answered(200), &Vec::new()));
        assert!(!held(&answered(403), &Vec::new()), "an error, nothing asked");
        assert!(held(&answered(404), &checked(true)), "a 404 asked for");
        assert!(!held(&answered(200), &checked(false)));
        assert!(!held(&answered(200), &vec![(0, Err(Problem::NotJson))]));
        assert!(!held(&HttpResponse::default(), &Vec::new()), "no answer");
        let failed = HttpResponse {
            error: Some("reset".into()),
            ..answered(200)
        };
        assert!(!held(&failed, &Vec::new()));
    }

    #[test]
    fn data_rows_come_from_csv_or_json() {
        assert_eq!(
            rows("user,pass\nme,\"a,b\"\r\n\"you\",\"say \"\"hi\"\"\nthere\"\n\n").expect("csv"),
            [
                values(&[("user", "me"), ("pass", "a,b")]),
                values(&[("user", "you"), ("pass", "say \"hi\"\nthere")]),
            ]
        );
        assert_eq!(
            rows("a,b\n1,2,3"),
            Err(DataError::Width {
                line: 2,
                found: 3,
                named: 2
            })
        );
        assert_eq!(rows("a\n\"open"), Err(DataError::Quote { line: 2 }));
        assert_eq!(
            rows(r#" [{"id": 1, "name": "a", "ok": true}, {"id": "2"}] "#).expect("json"),
            [
                values(&[("id", "1"), ("name", "a"), ("ok", "true")]),
                values(&[("id", "2")])
            ]
        );
        assert_eq!(rows("[1]"), Err(DataError::NotRows));
        assert!(matches!(rows("[1"), Err(DataError::Json { .. })));
        assert_eq!(rows("  \n"), Ok(Vec::new()));
        assert_eq!(rows("only,names"), Ok(Vec::new()));
    }
}
