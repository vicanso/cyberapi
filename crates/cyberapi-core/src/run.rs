//! A3: a run — every API of a folder or of the project, in the tree's
//! order, sent one after another, once or once for each row of a data file
//! (CSV with a header row, or a JSON array of objects). A row's values, and
//! what an API takes from its answer (A2, `assertions::Extraction`) for the
//! ones after it in the same pass, are read as custom variables
//! (`{{value(name)}}`) and as a URI's `{{name}}` prefix, in place of the
//! project's of the same name. They are the run's alone: nothing is written
//! to the project. What a run sent is kept in the history as any send is;
//! the run itself is only shown.

use crate::assertions::{Outcomes, tally};
use crate::import::{self, Format, ImportEnvironment, ImportError};
use crate::model::{ApiSetting, Environment, Folder, Variable, VariableCategory};
use crate::response::HttpResponse;
use crate::sync::ProjectConfig;
use crate::tree::{ItemKind, Tree, TreeData};
use crate::{EnvironmentId, ItemId, VariableId};
use serde_json::Value;
use snafu::Snafu;

/// A run's values, by name, in the order they were given; a later one of a
/// name stands for an earlier.
pub type Values = Vec<Pair>;

/// A name and its value.
type Pair = (String, String);

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

/// A4: a project file as a run outside the app reads it — whatever the app
/// imports (`import::parse`). CyberAPI's own export keeps its ids, which a
/// `get` names an API by; any other format's items get ids of their own.
/// Its variables are the ones every environment shares, its named
/// environments and its auth the project's.
pub fn project(text: &str) -> Result<ProjectConfig, ImportError> {
    let bundle = import::parse(text)?;
    let mut config = ProjectConfig {
        name: bundle.name.clone().unwrap_or_default(),
        ..Default::default()
    };
    let tree = match import::detect(text)? {
        (Format::CyberApi, Some(json)) => import::tree_data(&json),
        _ => {
            let mut next = 0;
            let assigned = import::assign_ids(&bundle.items, &mut || {
                next += 1;
                ItemId::new(format!("item-{next}"))
            });
            TreeData {
                folders: assigned
                    .folders
                    .into_iter()
                    .map(|(id, name, children, settings)| Folder {
                        id,
                        collection: Default::default(),
                        name,
                        children,
                        settings,
                        created_at: Default::default(),
                        updated_at: Default::default(),
                    })
                    .collect(),
                apis: assigned
                    .apis
                    .into_iter()
                    .map(|(id, name, request)| ApiSetting {
                        id,
                        name,
                        request,
                        ..Default::default()
                    })
                    .collect(),
                top_order: assigned.top,
                ..Default::default()
            }
        }
    };
    config.folders = tree.folders;
    config.apis = tree.apis;
    config.top_order = tree.top_order;
    config.settings.auth = bundle.auth;
    add_environments(&mut config, &bundle.environment, &bundle.environments);
    Ok(config)
}

/// A4: `shared` as variables every environment has, and each of `named` as
/// an environment of the project's — the one of its name, in any case,
/// where it has one. A later one of a name stands for an earlier.
pub fn add_environments(config: &mut ProjectConfig, shared: &[(String, String)], named: &[ImportEnvironment]) {
    // Each environment, `None` for every one's, and what it is given.
    let mut layers: Vec<(Option<EnvironmentId>, &[Pair])> = vec![(None, shared)];
    for environment in named.iter().filter(|environment| !environment.name.trim().is_empty()) {
        let name = environment.name.trim();
        let id = match config
            .environments
            .iter()
            .find(|known| known.name.eq_ignore_ascii_case(name))
        {
            Some(known) => known.id.clone(),
            None => {
                let id = EnvironmentId::new(format!("environment-{}", config.environments.len() + 1));
                config.environments.push(Environment {
                    id: id.clone(),
                    name: name.to_string(),
                    ..Default::default()
                });
                id
            }
        };
        layers.push((Some(id), &environment.variables));
    }
    for (environment, variables) in layers {
        for (name, value) in variables {
            config.variables.retain(|known| {
                !(known.category == VariableCategory::Environment
                    && known.environment == environment
                    && known.name == *name)
            });
            config.variables.push(Variable {
                id: VariableId::new(format!("variable-{}", config.variables.len() + 1)),
                environment: environment.clone(),
                ..variable(VariableCategory::Environment, name, value)
            });
        }
    }
}

/// A4: one send of a run, as a JUnit test case.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Case {
    /// The API's name, and the data row's number when there are rows.
    pub name: String,
    /// Where it is: its folders, `/` between.
    pub group: String,
    pub seconds: f64,
    /// Why it did not hold; `None` when it did.
    pub failure: Option<String>,
}

/// A4: a run as JUnit XML, which CI systems read: one suite named `suite`.
pub fn junit(suite: &str, cases: &[Case]) -> String {
    fn escaped(text: &str) -> String {
        text.chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
            .map(|c| match c {
                '&' => "&amp;".to_string(),
                '<' => "&lt;".to_string(),
                '>' => "&gt;".to_string(),
                '"' => "&quot;".to_string(),
                '\n' => "&#10;".to_string(),
                c => c.to_string(),
            })
            .collect()
    }
    let failures = cases.iter().filter(|case| case.failure.is_some()).count();
    let total: f64 = cases.iter().map(|case| case.seconds).sum();
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuites tests=\"{}\" failures=\"{failures}\" time=\"{total:.3}\">\n",
        cases.len()
    ));
    out.push_str(&format!(
        "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{failures}\" time=\"{total:.3}\">\n",
        escaped(suite),
        cases.len()
    ));
    for case in cases {
        let opening = format!(
            "    <testcase classname=\"{}\" name=\"{}\" time=\"{:.3}\"",
            escaped(&case.group),
            escaped(&case.name),
            case.seconds
        );
        match &case.failure {
            None => out.push_str(&format!("{opening}/>\n")),
            Some(why) => {
                let first = why.lines().next().unwrap_or_default();
                out.push_str(&format!(
                    "{opening}>\n      <failure message=\"{}\">{}</failure>\n    </testcase>\n",
                    escaped(first),
                    escaped(why)
                ));
            }
        }
    }
    out.push_str("  </testsuite>\n</testsuites>\n");
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

    /// A4: CyberAPI's own export keeps its ids; another format's items get
    /// ids of their own; variables and environments come along.
    #[test]
    fn a_project_file_reads_as_the_app_imports_it() {
        let own = r#"[
            {"id": "f1", "name": "users", "children": "a1,gone"},
            {"id": "a1", "name": "login", "category": "http", "setting": "{\"method\":\"POST\",\"uri\":\"{{HOST}}/login\"}"},
            {"id": "a2", "name": "me", "category": "http", "setting": "{\"uri\":\"{{HOST}}/me\"}"}
        ]"#;
        let config = project(own).expect("an export");
        let tree = config.tree();
        let names = |ids: Vec<ItemId>| ids.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        assert_eq!(names(plan(&tree, None)), ["a1", "a2"], "its ids, the folder's first");
        assert_eq!(config.folders[0].children, [ItemId::new("a1")], "what the file has");

        let postman = r#"{"info": {"name": "Shop"}, "item": [
            {"name": "orders", "item": [{"name": "list", "request": {"method": "GET", "url": "{{host}}/orders"}}]},
            {"name": "health", "request": {"method": "GET", "url": "{{host}}/h"}}
        ], "variable": [{"key": "host", "value": "https://shop.test"}],
        "auth": {"type": "bearer", "bearer": [{"key": "token", "value": "{{token}}"}]}}"#;
        let mut config = project(postman).expect("a collection");
        assert_eq!(config.name, "Shop");
        assert_eq!(config.apis.len(), 2);
        assert!(config.settings.auth.signs());
        assert_eq!(
            config
                .variables
                .iter()
                .map(|v| (v.name.as_str(), v.value.as_str(), v.environment.is_none()))
                .collect::<Vec<_>>(),
            [("host", "https://shop.test", true)]
        );
        add_environments(
            &mut config,
            &[("host".into(), "http://localhost".into())],
            &[ImportEnvironment {
                name: "CI".into(),
                variables: vec![("token".into(), "t".into())],
            }],
        );
        let shared: Vec<&str> = config
            .variables
            .iter()
            .filter(|v| v.environment.is_none())
            .map(|v| v.value.as_str())
            .collect();
        assert_eq!(shared, ["http://localhost"], "a later one stands for the earlier");
        assert_eq!(config.environments[0].name, "CI");
        assert!(matches!(project("nothing"), Err(ImportError::Unreadable { .. })));
    }

    #[test]
    fn a_run_reads_as_junit() {
        let xml = junit(
            "Shop <CI>",
            &[
                Case {
                    name: "login".into(),
                    group: "users".into(),
                    seconds: 0.012,
                    failure: None,
                },
                Case {
                    name: "me [2]".into(),
                    group: String::new(),
                    seconds: 0.5,
                    failure: Some("status equals 200: was 401\nsecond line".into()),
                },
            ],
        );
        assert_eq!(
            xml,
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                "<testsuites tests=\"2\" failures=\"1\" time=\"0.512\">\n",
                "  <testsuite name=\"Shop &lt;CI&gt;\" tests=\"2\" failures=\"1\" time=\"0.512\">\n",
                "    <testcase classname=\"users\" name=\"login\" time=\"0.012\"/>\n",
                "    <testcase classname=\"\" name=\"me [2]\" time=\"0.500\">\n",
                "      <failure message=\"status equals 200: was 401\">status equals 200: was 401&#10;second line</failure>\n",
                "    </testcase>\n",
                "  </testsuite>\n",
                "</testsuites>\n",
            )
        );
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
