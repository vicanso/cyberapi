//! A GraphQL body (`ContentType::GraphQl`): the query, its variables and the
//! operation to run, as the editor keeps them in `HttpRequest.body` — the
//! JSON of [`GraphQlBody`], as a form keeps the JSON of its fields — and as
//! a server is sent them, after GraphQL over HTTP: a POST's JSON body
//! `{"query", "variables", "operationName"}`, or a GET's query parameters.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A GraphQL request as the editor holds it. `variables` is text — JSON
/// that may use template functions, which run before it is read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GraphQlBody {
    pub query: String,
    pub variables: String,
    pub operation_name: String,
}

impl GraphQlBody {
    /// The body as stored. Text that is not one (a JSON or plain body the
    /// reader just switched from) is taken as the query, so nothing typed
    /// is lost; a JSON body that is already a GraphQL request
    /// (`{"query": …, "variables": {…}}`) is read as one.
    pub fn read(body: &str) -> Self {
        if body.trim().is_empty() {
            return Self::default();
        }
        if let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(body) {
            let text = |key: &str| fields.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
            if fields.contains_key("query") {
                let variables = match fields.get("variables") {
                    Some(Value::String(text)) => text.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(json) => crate::format::json(json),
                };
                return Self {
                    query: text("query"),
                    variables,
                    operation_name: text("operationName"),
                };
            }
        }
        Self {
            query: body.to_string(),
            ..Self::default()
        }
    }

    /// What `HttpRequest.body` keeps.
    pub fn to_body(&self) -> String {
        // A struct of strings cannot fail to serialize.
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The variables, read: `None` when there are none, otherwise the
    /// object they must be.
    fn variables_json(&self) -> Result<Option<Value>, String> {
        let text = crate::send::strip_json_comments(&self.variables);
        if text.trim().is_empty() {
            return Ok(None);
        }
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(fields)) => Ok(Some(Value::Object(fields))),
            Ok(Value::Null) => Ok(None),
            Ok(_) => Err("they are not a JSON object".into()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// A POST's body: `{"query": …, "variables": {…}, "operationName": …}`,
    /// the last two only when given. The variables must be a JSON object.
    pub fn payload(&self) -> Result<Value, String> {
        let mut fields = Map::new();
        fields.insert("query".into(), Value::String(self.query.clone()));
        if let Some(variables) = self.variables_json()? {
            fields.insert("variables".into(), variables);
        }
        if !self.operation_name.trim().is_empty() {
            fields.insert("operationName".into(), Value::String(self.operation_name.trim().into()));
        }
        Ok(Value::Object(fields))
    }

    /// Whether the operation it runs is a subscription.
    pub fn is_subscription(&self) -> bool {
        self.operation() == Some("subscription")
    }

    /// D6: whether the operation it runs is a mutation.
    pub fn is_mutation(&self) -> bool {
        self.operation() == Some("mutation")
    }

    /// What the operation it runs is — `query`, `mutation` or
    /// `subscription` — the one named `operation_name`, or the document's
    /// first; `None` when there is none. A scan of the top level of the
    /// query, its comments and strings skipped.
    pub fn operation(&self) -> Option<&'static str> {
        let wanted = self.operation_name.trim();
        let mut depth = 0usize;
        let mut chars = self.query.chars().peekable();
        let mut words: Vec<String> = Vec::new();
        while let Some(ch) = chars.next() {
            match ch {
                '#' => while chars.next_if(|ch| *ch != '\n').is_some() {},
                '"' => {
                    while let Some(ch) = chars.next() {
                        match ch {
                            '\\' => {
                                chars.next();
                            }
                            '"' => break,
                            _ => {}
                        }
                    }
                }
                '{' | '(' => {
                    // An operation's keyword and name are the words at the
                    // top level just before its selection or variables.
                    if depth == 0 {
                        let (keyword, name) = match words.as_slice() {
                            [.., keyword, name] if is_operation(keyword) => (keyword.as_str(), name.as_str()),
                            [.., keyword] if is_operation(keyword) => (keyword.as_str(), ""),
                            // A bare selection set is a query.
                            [] => ("query", ""),
                            _ => ("", ""),
                        };
                        if !keyword.is_empty() && (wanted.is_empty() || name == wanted) {
                            return ["query", "mutation", "subscription"]
                                .into_iter()
                                .find(|known| *known == keyword);
                        }
                    }
                    words.clear();
                    depth += 1;
                }
                '}' | ')' => depth = depth.saturating_sub(1),
                ch if ch.is_alphanumeric() || ch == '_' => {
                    let mut word = String::from(ch);
                    while let Some(ch) = chars.next_if(|ch| ch.is_alphanumeric() || *ch == '_') {
                        word.push(ch);
                    }
                    if depth == 0 {
                        words.push(word);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// A GET's query parameters: `query`, and `variables` (compact JSON) and
    /// `operationName` when given.
    pub fn query_params(&self) -> Result<Vec<(String, String)>, String> {
        let mut params = vec![("query".to_string(), self.query.clone())];
        if let Some(variables) = self.variables_json()? {
            params.push(("variables".into(), variables.to_string()));
        }
        if !self.operation_name.trim().is_empty() {
            params.push(("operationName".into(), self.operation_name.trim().into()));
        }
        Ok(params)
    }
}

fn is_operation(word: &str) -> bool {
    matches!(word, "query" | "mutation" | "subscription")
}

/// What a GraphQL response says went wrong: each of its `errors`' message,
/// with the path it came from when it has one. Empty for a response that
/// is not GraphQL's or has none.
pub fn response_errors(body: &[u8]) -> Vec<String> {
    serde_json::from_slice::<Value>(body)
        .map(|json| errors_in(&json))
        .unwrap_or_default()
}

/// [`response_errors`] of a body read already (PERF-2).
pub fn errors_in(json: &Value) -> Vec<String> {
    let Some(Value::Array(errors)) = json.get("errors") else {
        return Vec::new();
    };
    errors
        .iter()
        .map(|error| {
            let message = error.get("message").and_then(Value::as_str).unwrap_or("error");
            let path: Vec<String> = error
                .get("path")
                .and_then(Value::as_array)
                .map(|path| {
                    path.iter()
                        .map(|part| match part {
                            Value::String(name) => name.clone(),
                            other => other.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            if path.is_empty() {
                message.to_string()
            } else {
                format!("{} ({})", message, path.join("."))
            }
        })
        .collect()
}

/// An error a response places in the query (#70): what it says, and where
/// — the line and the column, from 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorPlace {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

/// Every place the response's `errors` name in the query.
pub fn response_error_places(body: &[u8]) -> Vec<ErrorPlace> {
    serde_json::from_slice::<Value>(body)
        .map(|json| error_places_in(&json))
        .unwrap_or_default()
}

/// [`response_error_places`] of a body read already (PERF-2).
pub fn error_places_in(json: &Value) -> Vec<ErrorPlace> {
    let Some(Value::Array(errors)) = json.get("errors") else {
        return Vec::new();
    };
    errors
        .iter()
        .flat_map(|error| {
            let message = error.get("message").and_then(Value::as_str).unwrap_or("error");
            error
                .get("locations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(move |place| {
                    let at = |key: &str| place.get(key).and_then(Value::as_u64).filter(|at| *at > 0);
                    Some(ErrorPlace {
                        message: message.to_string(),
                        line: at("line")? as usize,
                        column: at("column")? as usize,
                    })
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_errors_locations_place_it_in_the_query() {
        let body = json!({"errors": [
            {"message": "Cannot query field \"boom\"", "locations": [{"line": 2, "column": 5}, {"line": 3, "column": 1}]},
            {"message": "no place"},
            {"message": "bad place", "locations": [{"line": 0, "column": 4}, {"line": "2"}]}
        ]})
        .to_string();
        let places = response_error_places(body.as_bytes());
        let at: Vec<(usize, usize)> = places.iter().map(|place| (place.line, place.column)).collect();
        assert_eq!(at, [(2, 5), (3, 1)]);
        assert_eq!(places[0].message, "Cannot query field \"boom\"");
        assert!(response_error_places(b"not json").is_empty());
    }

    #[test]
    fn the_body_reads_back_what_it_keeps() {
        let body = GraphQlBody {
            query: "query Me { me { id } }".into(),
            variables: "{\n    \"a\": 1\n}".into(),
            operation_name: "Me".into(),
        };
        assert_eq!(GraphQlBody::read(&body.to_body()), body);
        assert_eq!(GraphQlBody::read(""), GraphQlBody::default());
    }

    #[test]
    fn text_from_another_body_type_is_not_lost() {
        assert_eq!(GraphQlBody::read("{ me { id } }").query, "{ me { id } }");
        assert_eq!(GraphQlBody::read(r#"{"name": "x"}"#).query, r#"{"name": "x"}"#);
        let request = GraphQlBody::read(r#"{"query": "{ a }", "variables": {"id": 1}, "operationName": "A"}"#);
        assert_eq!(request.query, "{ a }");
        assert_eq!(request.variables, "{\n    \"id\": 1\n}");
        assert_eq!(request.operation_name, "A");
    }

    #[test]
    fn a_post_sends_json_and_a_get_sends_parameters() {
        let body = GraphQlBody {
            query: "{ a }".into(),
            variables: "{\n    // the user\n    \"id\": 1,\n    \"x\": null\n}".into(),
            operation_name: String::new(),
        };
        assert_eq!(
            body.payload(),
            Ok(json!({"query": "{ a }", "variables": {"id": 1, "x": null}}))
        );
        assert_eq!(
            body.query_params(),
            Ok(vec![
                ("query".into(), "{ a }".into()),
                ("variables".into(), r#"{"id":1,"x":null}"#.into())
            ])
        );
        let bare = GraphQlBody {
            query: "{ a }".into(),
            operation_name: " A ".into(),
            ..Default::default()
        };
        assert_eq!(bare.payload(), Ok(json!({"query": "{ a }", "operationName": "A"})));
        let wrong = GraphQlBody {
            variables: "[1]".into(),
            ..Default::default()
        };
        assert!(wrong.payload().is_err());
        let broken = GraphQlBody {
            variables: "{".into(),
            ..Default::default()
        };
        assert!(broken.payload().is_err());
    }

    #[test]
    fn a_subscription_is_told_from_the_operation_sent() {
        let body = |query: &str, operation: &str| GraphQlBody {
            query: query.into(),
            operation_name: operation.into(),
            ..Default::default()
        };
        assert!(body("subscription { tick }", "").is_subscription());
        assert!(body("# a note { \nsubscription OnTick($n: Int) { tick(n: $n) }", "").is_subscription());
        assert!(!body("{ me }", "").is_subscription());
        assert!(!body("query Q { a(s: \"subscription {\") }", "").is_subscription());
        let both = "query Q { a } subscription S { b }";
        assert!(!body(both, "").is_subscription(), "the first runs");
        assert!(body(both, "S").is_subscription());
        assert!(!body("fragment F on T { a } query { ...F }", "").is_subscription());
        // D6.
        assert!(body("mutation Out { logout }", "").is_mutation());
        assert!(!body(both, "S").is_mutation());
        assert_eq!(body("{ me }", "").operation(), Some("query"));
        assert_eq!(body("fragment F on T { a }", "").operation(), None);
    }

    #[test]
    fn a_responses_errors_are_told_with_their_path() {
        let body =
            br#"{"data": null, "errors": [{"message": "no user", "path": ["user", 0, "name"]}, {"message": "slow"}]}"#;
        assert_eq!(response_errors(body), ["no user (user.0.name)", "slow"]);
        assert!(response_errors(br#"{"data": {}}"#).is_empty());
        assert!(response_errors(b"<html>").is_empty());
    }
}
