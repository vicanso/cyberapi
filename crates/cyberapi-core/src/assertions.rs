//! Declarative assertions (A1, `docs/feature-research-0.3.md` §5.1): what
//! an API's answer is checked against without a script — its status, a
//! header, a value its JSON holds, its body, how long it took, how big it
//! was. The API keeps them in its request; the panel checks whichever
//! response it shows against the ones the API has now.
//!
//! A6: whether the body's JSON conforms to a JSON Schema the assertion
//! holds as its text: draft 2020-12, or the draft its `$schema` names; its
//! `$ref`s within itself only — nothing is fetched. One an OpenAPI import
//! wrote is marked so, and is the one a sync from the source updates.
//!
//! A2: what is taken from an answer the same way — an [`Extraction`] —
//! under a name the requests after it in a run (A3) read as
//! `{{value(name)}}` (or as a URI's `{{name}}` prefix). Only a run uses
//! it: the value is the run's, never written to the project.

use crate::response::HttpResponse;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One check, as the editor's row holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Assertion {
    pub enabled: bool,
    pub subject: Subject,
    /// The header's name, or the JSONPath; the other subjects have none.
    pub target: String,
    pub op: Op,
    /// What the answer is compared with, as written: a number for the
    /// comparisons of size, a pattern for `Matches`, a type's name for
    /// `IsType`. `Exists` and `NotExists` have none. A6: a schema's text.
    pub expected: String,
    /// A6: written by an import, from the API's definition: a sync from
    /// the source replaces it, and leaves the reader's own.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub imported: bool,
}

impl Default for Assertion {
    fn default() -> Self {
        Self {
            enabled: true,
            subject: Subject::default(),
            target: String::new(),
            op: Op::default(),
            expected: String::new(),
            imported: false,
        }
    }
}

impl Assertion {
    /// A6: an enabled check that the body conforms to `schema`, written by
    /// an import.
    pub fn imported_schema(schema: &Value) -> Self {
        Self {
            subject: Subject::Schema,
            op: Op::Conforms,
            expected: crate::format::json(schema),
            imported: true,
            ..Default::default()
        }
    }
}

/// What of the answer is checked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Subject {
    /// The HTTP status; a gRPC call's status code.
    #[default]
    Status,
    /// A header's values, by its name in any case.
    Header,
    /// What a JSONPath finds in the body (`json_filter`).
    Json,
    /// The body as text.
    Body,
    /// The time it took, in milliseconds.
    Latency,
    /// Its size on the wire, in bytes.
    Size,
    /// A6: the body's JSON, against a JSON Schema.
    Schema,
}

impl Subject {
    pub const ALL: [Self; 7] = [
        Self::Status,
        Self::Header,
        Self::Json,
        Self::Body,
        Self::Latency,
        Self::Size,
        Self::Schema,
    ];

    /// What a run can take a value from (A2): not a schema.
    pub const EXTRACTED: [Self; 6] = [
        Self::Status,
        Self::Header,
        Self::Json,
        Self::Body,
        Self::Latency,
        Self::Size,
    ];

    /// Whether it names something: a header, a path.
    pub fn has_target(self) -> bool {
        matches!(self, Self::Header | Self::Json)
    }

    /// The comparisons that mean something for it, the first the default.
    pub fn ops(self) -> &'static [Op] {
        use Op::*;
        match self {
            Self::Status | Self::Latency | Self::Size => &[Equals, NotEquals, Less, Greater],
            Self::Header | Self::Body => &[Equals, NotEquals, Contains, NotContains, Matches, Exists, NotExists],
            Self::Json => &[
                Equals,
                NotEquals,
                Contains,
                NotContains,
                Matches,
                Exists,
                NotExists,
                Less,
                Greater,
                IsType,
            ],
            Self::Schema => &[Conforms],
        }
    }
}

/// How the found value is compared with the expected one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Op {
    #[default]
    Equals,
    NotEquals,
    Contains,
    NotContains,
    /// A regular expression finds a match in it.
    Matches,
    Exists,
    NotExists,
    Less,
    Greater,
    /// A JSON value's type: `string`, `number`, `boolean`, `null`,
    /// `array`, `object`.
    IsType,
    /// A6: it conforms to the schema given.
    Conforms,
}

impl Op {
    /// Whether it compares with a value the row gives.
    pub fn takes_value(self) -> bool {
        !matches!(self, Self::Exists | Self::NotExists)
    }
}

/// A6: how many places a body does not conform at are said.
pub const SCHEMA_ERRORS: usize = 3;

/// The JSON types `IsType` knows, as written.
pub const TYPES: [&str; 6] = ["string", "number", "boolean", "null", "array", "object"];

/// What a check found: whether it held, and what was there — `None` when
/// nothing was (a header the answer lacks, a path that matches nothing).
#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
    pub passed: bool,
    pub actual: Option<String>,
}

/// Why a check could not be made at all.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// The request got no answer.
    NoResponse,
    /// The body is no JSON for a path to look in.
    NotJson,
    /// The path does not read: why (`json_filter`).
    Path(String),
    /// The pattern does not compile: why.
    Pattern(String),
    /// A comparison of size with something that is no number.
    NotANumber,
    /// A type `IsType` does not know.
    UnknownType,
    /// A header or a path with no name.
    NoTarget,
    /// A6: the schema does not read, or refers outside itself: why.
    Schema(String),
}

impl Problem {
    /// Why, in the short English an answer says (H1: MCP). The panel says
    /// the same in the reader's language.
    pub fn reason(&self) -> String {
        match self {
            Self::NoResponse => "no answer to check".into(),
            Self::NotJson => "the body is no JSON".into(),
            Self::Path(why) => format!("the path does not read: {why}"),
            Self::Pattern(why) => format!("the pattern does not read: {why}"),
            Self::NotANumber => "that is no number".into(),
            Self::UnknownType => format!("the type is one of {}", TYPES.join(", ")),
            Self::NoTarget => "it names no header or path".into(),
            Self::Schema(why) => format!("the schema does not read: {why}"),
        }
    }
}

/// The longest found value a check says it saw, in characters.
const SHOWN: usize = 200;

/// The value found, in the forms the comparisons need.
enum Found<'a> {
    Text(std::borrow::Cow<'a, str>),
    Number(f64),
    Json(Value),
}

impl Found<'_> {
    /// As compared: a JSON string without its quotes, anything else as
    /// JSON.
    fn text(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Self::Text(text) => std::borrow::Cow::Borrowed(text),
            Self::Number(number) => number_text(*number).into(),
            Self::Json(Value::String(text)) => std::borrow::Cow::Borrowed(text),
            Self::Json(value) => value.to_string().into(),
        }
    }

    /// As said: its start, when long.
    fn shown(&self) -> String {
        let text = self.text();
        match text.char_indices().nth(SHOWN) {
            Some((end, _)) => format!("{}…", &text[..end]),
            None => text.into_owned(),
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            Self::Number(number) => Some(*number),
            Self::Json(Value::Number(number)) => number.as_f64(),
            Self::Json(Value::String(text)) => text.trim().parse().ok(),
            Self::Text(text) => text.trim().parse().ok(),
            Self::Json(_) => None,
        }
    }
}

/// A whole number without its `.0`.
fn number_text(number: f64) -> String {
    if number.fract() == 0. && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        number.to_string()
    }
}

/// Checks `assertion` against `response`, whose body read as JSON is `json`
/// (`None` when it is no JSON): read once for every check of it.
pub fn check(assertion: &Assertion, response: &HttpResponse, json: Option<&Value>) -> Result<Checked, Problem> {
    if assertion.subject == Subject::Schema {
        return conforms(&assertion.expected, response, json);
    }
    let found = found(assertion.subject, &assertion.target, response, json)?;
    let actual = found.as_ref().map(Found::shown);
    let passed = compare(assertion.op, found.as_ref(), assertion.expected.trim())?;
    Ok(Checked { passed, actual })
}

/// A6: whether the body's JSON conforms to `schema` (its text); what does
/// not is said by the first [`SCHEMA_ERRORS`] places, each a JSON pointer
/// and why.
fn conforms(schema: &str, response: &HttpResponse, json: Option<&Value>) -> Result<Checked, Problem> {
    if response.status.is_none() {
        return Err(Problem::NoResponse);
    }
    let schema: Value = serde_json::from_str(schema).map_err(|error| Problem::Schema(error.to_string()))?;
    let validator = jsonschema::validator_for(&schema).map_err(|error| Problem::Schema(error.to_string()))?;
    let doc = json.ok_or(Problem::NotJson)?;
    let places: Vec<String> = validator
        .iter_errors(doc)
        .take(SCHEMA_ERRORS)
        .map(|error| {
            let at = error.instance_path().to_string();
            let at = if at.is_empty() { "/".to_string() } else { at };
            format!("{at}: {error}")
        })
        .collect();
    Ok(Checked {
        passed: places.is_empty(),
        actual: (!places.is_empty()).then(|| places.join("; ")),
    })
}

/// What `subject` (named by `target`) is in `response`: `None` for nothing.
fn found<'a>(
    subject: Subject,
    target: &str,
    response: &'a HttpResponse,
    json: Option<&Value>,
) -> Result<Option<Found<'a>>, Problem> {
    let Some(status) = response.status else {
        return Err(Problem::NoResponse);
    };
    if subject.has_target() && target.trim().is_empty() {
        return Err(Problem::NoTarget);
    }
    Ok(match subject {
        Subject::Status => {
            let code = response.grpc_status().map_or(u32::from(status), |grpc| grpc.code);
            Some(Found::Number(f64::from(code)))
        }
        Subject::Latency => Some(Found::Number(response.latency.as_millis() as f64)),
        Subject::Size => Some(Found::Number(response.size() as f64)),
        Subject::Body => (!response.body.is_empty()).then(|| Found::Text(String::from_utf8_lossy(&response.body))),
        Subject::Header => {
            let name = target.trim();
            let values: Vec<&str> = response
                .headers
                .iter()
                .filter(|(header, _)| header.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
                .collect();
            (!values.is_empty()).then(|| Found::Text(values.join(", ").into()))
        }
        // Checked whole by `conforms`; nothing to take.
        Subject::Schema => None,
        Subject::Json => {
            let doc = json.ok_or(Problem::NotJson)?;
            let matches = crate::json_filter::filter(doc, target).map_err(Problem::Path)?;
            match matches {
                Value::Array(mut found) if found.len() == 1 => found.pop().map(Found::Json),
                Value::Array(found) if found.is_empty() => None,
                found => Some(Found::Json(found)),
            }
        }
    })
}

/// A2: a value taken from an answer during a run, as the editor's row holds
/// it: what of the answer, and the header or path when it names one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Extraction {
    pub enabled: bool,
    /// What the requests after it read it as: `{{value(name)}}`.
    pub name: String,
    pub subject: Subject,
    pub target: String,
}

impl Default for Extraction {
    fn default() -> Self {
        Self {
            enabled: true,
            name: String::new(),
            subject: Subject::Json,
            target: String::new(),
        }
    }
}

/// A2: what `extraction` takes from `response` (`json` as for [`check`]):
/// a JSON string without its quotes, any other value as JSON; `None` for
/// nothing there.
pub fn extract(
    extraction: &Extraction,
    response: &HttpResponse,
    json: Option<&Value>,
) -> Result<Option<String>, Problem> {
    Ok(found(extraction.subject, &extraction.target, response, json)?.map(|found| found.text().into_owned()))
}

/// A2: what every enabled, named extraction takes from `response`, by name;
/// what is not there, or cannot be read, is left out.
pub fn extract_all(extractions: &[Extraction], response: &HttpResponse) -> Vec<(String, String)> {
    let json = extractions
        .iter()
        .any(|extraction| extraction.enabled && extraction.subject == Subject::Json)
        .then(|| response.body_json())
        .flatten();
    extractions
        .iter()
        .filter(|extraction| extraction.enabled && !extraction.name.trim().is_empty())
        .filter_map(|extraction| {
            let value = extract(extraction, response, json.as_ref()).ok()??;
            Some((extraction.name.trim().to_string(), value))
        })
        .collect()
}

fn compare(op: Op, found: Option<&Found>, expected: &str) -> Result<bool, Problem> {
    let Some(found) = found else {
        // Nothing there: only that it is absent holds, and that it is not
        // some value.
        return Ok(matches!(op, Op::NotExists | Op::NotEquals | Op::NotContains));
    };
    Ok(match op {
        Op::Exists => true,
        Op::NotExists => false,
        Op::Equals => equals(found, expected),
        Op::NotEquals => !equals(found, expected),
        Op::Contains => contains(found, expected),
        Op::NotContains => !contains(found, expected),
        Op::Matches => regex::Regex::new(expected)
            .map_err(|error| Problem::Pattern(error.to_string().lines().last().unwrap_or_default().into()))?
            .is_match(&found.text()),
        Op::Less | Op::Greater => {
            let wanted: f64 = expected.parse().map_err(|_| Problem::NotANumber)?;
            match found.number() {
                Some(number) if op == Op::Less => number < wanted,
                Some(number) => number > wanted,
                None => false,
            }
        }
        Op::IsType => {
            if !TYPES.contains(&expected) {
                return Err(Problem::UnknownType);
            }
            json_type(found) == expected
        }
        // Only a schema conforms, and `conforms` checks it.
        Op::Conforms => false,
    })
}

/// Numbers by value (`200` is `200.0`), JSON by value (`{"a":1}` however
/// spaced), anything else as text; a JSON string by its text, quoted or not.
fn equals(found: &Found, expected: &str) -> bool {
    match found {
        Found::Json(Value::String(text)) => {
            text == expected || serde_json::from_str::<String>(expected).is_ok_and(|unquoted| &unquoted == text)
        }
        Found::Json(value) => serde_json::from_str::<Value>(expected).is_ok_and(|wanted| json_equal(value, &wanted)),
        Found::Number(number) => expected.parse::<f64>().is_ok_and(|wanted| wanted == *number),
        Found::Text(text) => text == expected,
    }
}

/// JSON equality with numbers by value.
fn json_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| json_equal(a, b)),
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, value)| b.get(key).is_some_and(|other| json_equal(value, other)))
        }
        _ => a == b,
    }
}

/// Text holds it; a list holds an item equal to it; an object has it as a
/// key.
fn contains(found: &Found, expected: &str) -> bool {
    match found {
        Found::Json(Value::Array(items)) => items.iter().any(|item| equals(&Found::Json(item.clone()), expected)),
        Found::Json(Value::Object(fields)) => fields.contains_key(expected),
        found => found.text().contains(expected),
    }
}

fn json_type(found: &Found) -> &'static str {
    match found {
        Found::Number(_) => "number",
        Found::Text(_) => "string",
        Found::Json(value) => match value {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        },
    }
}

/// What [`check_all`] answers: each enabled assertion's place in the list
/// and its outcome.
pub type Outcomes = Vec<(usize, Result<Checked, Problem>)>;

/// Whether any enabled assertion looks in the body's JSON: only then is
/// the body read so.
pub fn reads_json(assertions: &[Assertion]) -> bool {
    assertions
        .iter()
        .any(|assertion| assertion.enabled && matches!(assertion.subject, Subject::Json | Subject::Schema))
}

/// Every enabled assertion's outcome, `json` as for [`check`].
pub fn check_all(assertions: &[Assertion], response: &HttpResponse, json: Option<&Value>) -> Outcomes {
    assertions
        .iter()
        .enumerate()
        .filter(|(_, assertion)| assertion.enabled)
        .map(|(index, assertion)| (index, check(assertion, response, json)))
        .collect()
}

/// One enabled assertion as an answer reports it (H1): where it sits in the
/// API's list, what it asked, and whether it held. A check that did not
/// hold also says what was there; one that could not be made says why.
/// Nothing of this is written into the history.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reported {
    pub index: usize,
    pub subject: Subject,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target: String,
    pub op: Op,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub expected: String,
    /// Set when the check was made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passed: Option<bool>,
    /// What was there, only when the check was made and did not hold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    /// Why the check could not be made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The enabled assertions' results for `response`. Empty when none is
/// enabled, so an answer can leave the field out. The body's JSON is read
/// only when one of them looks in it.
pub fn report(assertions: &[Assertion], response: &HttpResponse) -> Vec<Reported> {
    let json = reads_json(assertions).then(|| response.body_json()).flatten();
    check_all(assertions, response, json.as_ref())
        .into_iter()
        .filter_map(|(index, outcome)| {
            let assertion = assertions.get(index)?;
            Some(Reported::of(index, assertion, outcome))
        })
        .collect()
}

impl Reported {
    fn of(index: usize, assertion: &Assertion, outcome: Result<Checked, Problem>) -> Self {
        let (passed, actual, error) = match outcome {
            Ok(Checked { passed: true, .. }) => (Some(true), None, None),
            Ok(Checked { actual, .. }) => (Some(false), actual, None),
            Err(problem) => (None, None, Some(problem.reason())),
        };
        Self {
            index,
            subject: assertion.subject,
            target: assertion.target.clone(),
            op: assertion.op,
            // A6: a schema is too long to repeat in every answer.
            expected: match assertion.subject {
                Subject::Schema => String::new(),
                _ => assertion.expected.clone(),
            },
            passed,
            actual,
            error,
        }
    }
}

/// How many of `outcomes` held, of how many.
pub fn tally(outcomes: &Outcomes) -> (usize, usize) {
    let passed = outcomes
        .iter()
        .filter(|(_, outcome)| matches!(outcome, Ok(Checked { passed: true, .. })))
        .count();
    (passed, outcomes.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    fn response() -> HttpResponse {
        HttpResponse {
            status: Some(201),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("Set-Cookie".into(), "a=1".into()),
                ("set-cookie".into(), "b=2".into()),
            ],
            body: br#"{"id": 7, "name": "tree", "ok": true, "tags": ["a", "b"], "price": 9.5, "none": null, "items": [{"n": 1}, {"n": 2}]}"#.to_vec(),
            latency: Duration::from_millis(120),
            ..Default::default()
        }
    }

    fn holds(subject: Subject, target: &str, op: Op, expected: &str) -> Result<Checked, Problem> {
        let assertion = Assertion {
            subject,
            target: target.into(),
            op,
            expected: expected.into(),
            ..Default::default()
        };
        let response = response();
        check(&assertion, &response, response.body_json().as_ref())
    }

    fn passes(subject: Subject, target: &str, op: Op, expected: &str) -> bool {
        holds(subject, target, op, expected).expect("checked").passed
    }

    /// A6: the body against a schema; where it does not conform, the first
    /// places; a schema that does not read, or reaches outside itself, is
    /// said to.
    #[test]
    fn a_body_conforms_to_a_schema_or_says_where_not() {
        let schema = |text: &str| holds(Subject::Schema, "", Op::Conforms, text);
        let conforms = schema(
            r##"{"type": "object", "required": ["id", "name"], "properties": {"id": {"type": "integer"},
                "items": {"type": "array", "items": {"$ref": "#/$defs/item"}}},
                "$defs": {"item": {"type": "object", "properties": {"n": {"type": "integer"}}}}}"##,
        )
        .expect("checked");
        assert_eq!(
            conforms,
            Checked {
                passed: true,
                actual: None
            }
        );
        let not = schema(
            r#"{"type": "object", "properties": {"id": {"type": "string"}, "name": {"maxLength": 2},
                "tags": {"items": {"type": "integer"}}}}"#,
        )
        .expect("checked");
        assert!(!not.passed);
        let said = not.actual.expect("where");
        assert_eq!(said.matches("; ").count(), SCHEMA_ERRORS - 1, "{said}");
        assert!(said.contains("/id: "), "{said}");
        // Draft 7, as its `$schema` says.
        assert!(
            schema(r#"{"$schema": "http://json-schema.org/draft-07/schema#", "type": "object"}"#)
                .expect("checked")
                .passed
        );
        assert!(matches!(schema("{not json"), Err(Problem::Schema(_))));
        assert!(matches!(
            schema(r#"{"$ref": "https://example.com/schema.json"}"#),
            Err(Problem::Schema(_))
        ));
        let text = HttpResponse {
            status: Some(200),
            body: b"plain".to_vec(),
            ..Default::default()
        };
        let assertion = Assertion::imported_schema(&serde_json::json!({"type": "object"}));
        assert_eq!(check(&assertion, &text, None), Err(Problem::NotJson));
        assert!(assertion.imported && assertion.enabled);
    }

    #[test]
    fn the_status_time_and_size_are_numbers() {
        use Op::*;
        assert!(passes(Subject::Status, "", Equals, "201"));
        assert!(passes(Subject::Status, "", Equals, " 201 "), "blanks around it");
        assert!(!passes(Subject::Status, "", Equals, "200"));
        assert!(passes(Subject::Status, "", Less, "300"));
        assert!(passes(Subject::Latency, "", Less, "500"));
        assert!(!passes(Subject::Latency, "", Greater, "500"));
        assert_eq!(holds(Subject::Latency, "", Less, "soon"), Err(Problem::NotANumber));
        let size = response().size();
        assert!(passes(Subject::Size, "", Equals, &size.to_string()));
        let checked = holds(Subject::Status, "", Equals, "201").expect("checked");
        assert_eq!(checked.actual.as_deref(), Some("201"));
    }

    /// A2: a value as the requests after it read it; what is not there,
    /// or not named, is left out.
    #[test]
    fn a_run_takes_values_from_an_answer() {
        let take = |name: &str, subject: Subject, target: &str| Extraction {
            name: name.into(),
            subject,
            target: target.into(),
            ..Default::default()
        };
        let extractions = [
            take("id", Subject::Json, "$.id"),
            take("name", Subject::Json, "$.name"),
            take("tags", Subject::Json, "$.tags"),
            take("type", Subject::Header, "content-type"),
            take("status", Subject::Status, ""),
            take("gone", Subject::Json, "$.missing"),
            take("", Subject::Json, "$.id"),
            Extraction {
                enabled: false,
                ..take("off", Subject::Status, "")
            },
        ];
        assert_eq!(
            extract_all(&extractions, &response()),
            [
                ("id".to_string(), "7".to_string()),
                ("name".to_string(), "tree".to_string()),
                ("tags".to_string(), r#"["a","b"]"#.to_string()),
                ("type".to_string(), "application/json".to_string()),
                ("status".to_string(), "201".to_string()),
            ]
        );
        let failed = HttpResponse {
            status: None,
            ..response()
        };
        assert!(
            extract_all(&extractions, &failed).is_empty(),
            "no answer, nothing taken"
        );
        assert_eq!(
            extract(
                &take("x", Subject::Json, "$["),
                &response(),
                response().body_json().as_ref()
            )
            .map(|_| ()),
            Err(Problem::Path(
                crate::json_filter::filter(&serde_json::json!({}), "$[").expect_err("bad path")
            ))
        );
    }

    #[test]
    fn a_header_by_its_name_in_any_case() {
        use Op::*;
        assert!(passes(Subject::Header, "content-type", Contains, "json"));
        assert!(passes(Subject::Header, "CONTENT-TYPE", Equals, "application/json"));
        assert!(passes(Subject::Header, "set-cookie", Equals, "a=1, b=2"), "every value");
        assert!(passes(Subject::Header, "x-missing", NotExists, ""));
        assert!(!passes(Subject::Header, "x-missing", Exists, ""));
        assert!(
            !passes(Subject::Header, "x-missing", Equals, "a"),
            "a missing header is no value"
        );
        assert!(passes(Subject::Header, "x-missing", NotEquals, "a"));
        assert!(passes(Subject::Header, "content-type", Matches, "^application/.+$"));
        assert!(matches!(
            holds(Subject::Header, "content-type", Matches, "("),
            Err(Problem::Pattern(_))
        ));
        assert_eq!(holds(Subject::Header, " ", Exists, ""), Err(Problem::NoTarget));
    }

    #[test]
    fn a_path_finds_json_values() {
        use Op::*;
        assert!(passes(Subject::Json, "id", Equals, "7"));
        assert!(passes(Subject::Json, "$.id", Equals, "7.0"), "numbers by value");
        assert!(passes(Subject::Json, "name", Equals, "tree"));
        assert!(passes(Subject::Json, "name", Equals, "\"tree\""), "quoted or not");
        assert!(passes(Subject::Json, "ok", Equals, "true"));
        assert!(
            !passes(Subject::Json, "ok", Equals, "\"true\""),
            "a string is no boolean"
        );
        assert!(passes(Subject::Json, "tags", Contains, "a"));
        assert!(passes(Subject::Json, "tags", Equals, r#"["a","b"]"#));
        assert!(passes(Subject::Json, "items[0]", Contains, "n"), "an object's key");
        assert!(
            passes(Subject::Json, "items[*].n", Equals, "[1, 2]"),
            "several matches are a list"
        );
        assert!(passes(Subject::Json, "price", Greater, "9"));
        assert!(passes(Subject::Json, "none", IsType, "null"));
        assert!(passes(Subject::Json, "tags", IsType, "array"));
        assert!(passes(Subject::Json, "none", Exists, ""), "null is there");
        assert!(passes(Subject::Json, "missing", NotExists, ""));
        assert!(passes(Subject::Json, "name", Matches, "^tr"));
        assert_eq!(holds(Subject::Json, "name", IsType, "text"), Err(Problem::UnknownType));
        assert!(matches!(holds(Subject::Json, "$[", Exists, ""), Err(Problem::Path(_))));
        let checked = holds(Subject::Json, "name", Equals, "x").expect("checked");
        assert_eq!(checked.actual.as_deref(), Some("tree"), "a string shown without quotes");
    }

    #[test]
    fn a_body_that_is_no_json_or_no_answer() {
        let text = HttpResponse {
            status: Some(200),
            body: b"plain".to_vec(),
            ..Default::default()
        };
        let path = Assertion {
            subject: Subject::Json,
            target: "a".into(),
            op: Op::Exists,
            ..Default::default()
        };
        assert_eq!(check(&path, &text, text.body_json().as_ref()), Err(Problem::NotJson));
        let body = Assertion {
            subject: Subject::Body,
            op: Op::Contains,
            expected: "lai".into(),
            ..Default::default()
        };
        assert!(check(&body, &text, None).expect("checked").passed);
        assert_eq!(check(&body, &HttpResponse::default(), None), Err(Problem::NoResponse));
        // A long value is said by its start.
        let long = HttpResponse {
            status: Some(200),
            body: "x".repeat(500).into_bytes(),
            ..Default::default()
        };
        let checked = check(&body, &long, None).expect("checked");
        assert_eq!(checked.actual.map(|actual| actual.chars().count()), Some(SHOWN + 1));
    }

    #[test]
    fn only_enabled_ones_count() {
        let assertions = vec![
            Assertion {
                expected: "201".into(),
                ..Default::default()
            },
            Assertion {
                enabled: false,
                expected: "500".into(),
                ..Default::default()
            },
            Assertion {
                expected: "200".into(),
                ..Default::default()
            },
        ];
        let outcomes = check_all(&assertions, &response(), None);
        assert_eq!(outcomes.iter().map(|(index, _)| *index).collect::<Vec<_>>(), [0, 2]);
        assert_eq!(tally(&outcomes), (1, 2));
        assert!(!reads_json(&assertions));
    }

    /// Kept in the request; a request with none writes nothing of them, as
    /// before there were any.
    #[test]
    fn a_request_keeps_its_assertions() {
        let request = crate::model::HttpRequest {
            assertions: vec![Assertion {
                subject: Subject::Json,
                target: "id".into(),
                expected: "7".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let text = serde_json::to_string(&request).expect("writes");
        assert!(
            text.contains(r#""assertions":[{"enabled":true,"subject":"json""#),
            "{text}"
        );
        let back: crate::model::HttpRequest = serde_json::from_str(&text).expect("reads");
        assert_eq!(back, request);
        let none = serde_json::to_string(&crate::model::HttpRequest::default()).expect("writes");
        assert!(!none.contains("assertions"), "{none}");
    }

    #[test]
    fn every_subject_offers_only_what_means_something() {
        for subject in Subject::EXTRACTED {
            assert!(!subject.ops().is_empty());
            assert_eq!(subject.ops()[0], Op::Equals, "equals first");
        }
        // A6: a schema only conforms.
        assert_eq!(Subject::Schema.ops(), [Op::Conforms]);
        assert!(!Subject::Status.ops().contains(&Op::Exists));
        assert!(Subject::Json.ops().contains(&Op::IsType));
        // Kept as written; an older row without `enabled` is on.
        let read: Assertion =
            serde_json::from_str(r#"{"subject":"json","target":"id","op":"equals","expected":"7"}"#).expect("reads");
        assert!(read.enabled);
    }

    /// H1: a pass says so and nothing more; a failure says what was there;
    /// a check that could not be made says why. A disabled row is left out.
    #[test]
    fn a_report_says_each_enabled_assertions_result() {
        let assertions = vec![
            Assertion {
                expected: "201".into(),
                ..Default::default()
            },
            Assertion {
                enabled: false,
                expected: "500".into(),
                ..Default::default()
            },
            Assertion {
                expected: "200".into(),
                ..Default::default()
            },
            Assertion {
                subject: Subject::Header,
                target: "X-Missing".into(),
                op: Op::Equals,
                expected: "1".into(),
                ..Default::default()
            },
            Assertion {
                subject: Subject::Json,
                target: String::new(),
                op: Op::Exists,
                ..Default::default()
            },
        ];
        let value = serde_json::to_value(report(&assertions, &response())).expect("writes");
        assert_eq!(
            value,
            serde_json::json!([
                {"index": 0, "subject": "status", "op": "equals", "expected": "201", "passed": true},
                {"index": 2, "subject": "status", "op": "equals", "expected": "200", "passed": false, "actual": "201"},
                {"index": 3, "subject": "header", "target": "X-Missing", "op": "equals", "expected": "1", "passed": false},
                {"index": 4, "subject": "json", "op": "exists", "error": "it names no header or path"},
            ])
        );
        assert!(report(&[], &response()).is_empty());
    }
}
