//! How a run is told in a terminal and in JUnit: a line for each send as it
//! ends, what did not hold under it, the cases CI reads.

use cyberapi_core::ItemId;
use cyberapi_core::assertions::{Assertion, Checked, Op, Problem, Subject, TYPES};
use cyberapi_core::response::format_duration;
use cyberapi_core::run::Case;
use cyberapi_core::tree::Tree;
use cyberapi_service::StepResult;

/// Where an API is: its folders' names, and its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub folders: String,
    pub name: String,
}

impl Place {
    fn shown(&self) -> String {
        if self.folders.is_empty() {
            self.name.clone()
        } else {
            format!("{}/{}", self.folders, self.name)
        }
    }
}

pub fn place(tree: &Tree, api: &ItemId) -> Place {
    let name = tree.get(api).map(|node| node.name().to_string()).unwrap_or_default();
    let mut folders = Vec::new();
    let mut parent = tree.get(api).and_then(|node| node.parent()).cloned();
    while let Some(folder) = parent {
        let node = tree.get(&folder);
        folders.push(node.map(|node| node.name().to_string()).unwrap_or_default());
        parent = node.and_then(|node| node.parent()).cloned();
    }
    folders.reverse();
    Place {
        folders: folders.join("/"),
        name,
    }
}

/// `a, b and c`; `none` for nothing.
pub fn listed(names: &[&str]) -> String {
    match names {
        [] => "none".into(),
        [one] => format!("{one:?}"),
        [rest @ .., last] => format!(
            "{} and {last:?}",
            rest.iter()
                .map(|name| format!("{name:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// `✓ [2] users/login  POST /login  200  12 ms`.
pub fn line(result: &StepResult, place: &Place, numbered: bool) -> String {
    let mark = if result.held() { "✓" } else { "✗" };
    let pass = if numbered {
        format!("[{}] ", result.pass + 1)
    } else {
        String::new()
    };
    match &result.response {
        Ok(response) => {
            let status = match (response.grpc_status(), response.status) {
                (Some(grpc), _) => format!("{} {}", grpc.code, grpc.name()),
                (None, Some(code)) => code.to_string(),
                (None, None) => "failed".into(),
            };
            format!(
                "{mark} {pass}{}  {} {}  {status}  {}",
                place.shown(),
                response.request.method.as_str(),
                response.sent_path(),
                format_duration(response.latency)
            )
        }
        Err(_) => format!("{mark} {pass}{}  not sent", place.shown()),
    }
}

fn subject(assertion: &Assertion) -> String {
    match assertion.subject {
        Subject::Status => "status".into(),
        Subject::Header => format!("header {}", assertion.target),
        Subject::Json => assertion.target.clone(),
        Subject::Body => "body".into(),
        Subject::Latency => "time (ms)".into(),
        Subject::Size => "size (bytes)".into(),
    }
}

fn op(op: Op) -> &'static str {
    match op {
        Op::Equals => "equals",
        Op::NotEquals => "is not",
        Op::Contains => "contains",
        Op::NotContains => "does not contain",
        Op::Matches => "matches",
        Op::Exists => "exists",
        Op::NotExists => "does not exist",
        Op::Less => "is less than",
        Op::Greater => "is more than",
        Op::IsType => "is of type",
    }
}

fn problem(problem: &Problem) -> String {
    match problem {
        Problem::NoResponse => "no answer to check".into(),
        Problem::NotJson => "the body is no JSON".into(),
        Problem::Path(why) => format!("the path does not read: {why}"),
        Problem::Pattern(why) => format!("the pattern does not read: {why}"),
        Problem::NotANumber => "that is no number".into(),
        Problem::UnknownType => format!("the type is one of {}", TYPES.join(", ")),
        Problem::NoTarget => "it names no header or path".into(),
    }
}

/// What did not hold, a line each: why it was not sent, the error it got,
/// the assertions that failed — with what was there — or could not be
/// checked; with none to check, the error status.
pub fn details(result: &StepResult, assertions: &[Assertion]) -> Vec<String> {
    let response = match &result.response {
        Ok(response) => response,
        Err(why) => return vec![why.clone()],
    };
    let mut lines: Vec<String> = response.error.iter().cloned().collect();
    for (index, outcome) in &result.outcomes {
        let Some(assertion) = assertions.get(*index) else {
            continue;
        };
        let asked = match assertion.op.takes_value() {
            true => format!(
                "{} {} {}",
                subject(assertion),
                op(assertion.op),
                assertion.expected.trim()
            ),
            false => format!("{} {}", subject(assertion), op(assertion.op)),
        };
        match outcome {
            Ok(Checked { passed: true, .. }) => {}
            Ok(Checked { actual, .. }) => lines.push(match actual {
                Some(actual) => format!("✗ {asked}: was {actual}"),
                None => format!("✗ {asked}: nothing there"),
            }),
            Err(why) => lines.push(format!("! {asked}: {}", problem(why))),
        }
    }
    if lines.is_empty() && result.outcomes.is_empty() && !result.held() {
        lines.push("an error status, and no assertion asks for it".into());
    }
    lines
}

/// The send as JUnit has it; `assertions` are its API's.
pub fn case(result: &StepResult, place: &Place, numbered: bool, assertions: &[Assertion]) -> Case {
    let name = if numbered {
        format!("{} [{}]", place.name, result.pass + 1)
    } else {
        place.name.clone()
    };
    let seconds = result
        .response
        .as_ref()
        .map(|response| response.latency.as_secs_f64())
        .unwrap_or_default();
    Case {
        name,
        group: place.folders.clone(),
        seconds,
        failure: (!result.held()).then(|| {
            let lines = details(result, assertions);
            match lines.is_empty() {
                true => "did not hold".into(),
                false => lines.join("\n"),
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::response::HttpResponse;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    fn result(status: Option<u16>, outcomes: cyberapi_core::assertions::Outcomes) -> StepResult {
        StepResult {
            pass: 1,
            step: 0,
            response: Ok(HttpResponse {
                status,
                url: "https://a.test/login?x=1".into(),
                latency: Duration::from_millis(12),
                ..Default::default()
            }),
            outcomes,
            extracted: Vec::new(),
        }
    }

    #[test]
    fn a_send_is_told_in_a_line_and_what_failed_under_it() {
        let place = Place {
            folders: "users".into(),
            name: "login".into(),
        };
        let held = result(Some(200), Vec::new());
        assert_eq!(
            line(&held, &place, true),
            "✓ [2] users/login  GET /login?x=1  200  12 ms"
        );
        let assertion = Assertion {
            subject: Subject::Status,
            op: Op::Equals,
            expected: "201".into(),
            ..Default::default()
        };
        let failed = result(
            Some(200),
            vec![(
                0,
                Ok(Checked {
                    passed: false,
                    actual: Some("200".into()),
                }),
            )],
        );
        assert!(line(&failed, &place, false).starts_with("✗ users/login"));
        assert_eq!(
            details(&failed, std::slice::from_ref(&assertion)),
            ["✗ status equals 201: was 200"]
        );
        assert_eq!(
            details(&result(Some(500), Vec::new()), &[]),
            ["an error status, and no assertion asks for it"]
        );
        let unsent = StepResult {
            response: Err("there is no enabled environment variable HOST".into()),
            ..result(None, Vec::new())
        };
        assert_eq!(line(&unsent, &place, false), "✗ users/login  not sent");
        let junit = case(&unsent, &place, true, &[]);
        assert_eq!(junit.name, "login [2]");
        assert_eq!(junit.group, "users");
        assert_eq!(
            junit.failure.as_deref(),
            Some("there is no enabled environment variable HOST")
        );
        assert_eq!(case(&held, &place, false, &[]).failure, None);
        assert_eq!(
            case(&failed, &place, false, &[assertion]).failure.as_deref(),
            Some("✗ status equals 201: was 200")
        );
    }

    #[test]
    fn names_are_listed() {
        assert_eq!(listed(&[]), "none");
        assert_eq!(listed(&["dev"]), "\"dev\"");
        assert_eq!(listed(&["dev", "ci", "prod"]), "\"dev\", \"ci\" and \"prod\"");
    }
}
