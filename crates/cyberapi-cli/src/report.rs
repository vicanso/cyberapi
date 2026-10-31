//! How a run is told in a terminal and in JUnit: a line for each send as it
//! ends, what did not hold under it, the cases CI reads.

use cyberapi_core::assertions::Assertion;
use cyberapi_core::response::format_duration;
pub use cyberapi_core::run::{Case, Place, place};
use cyberapi_service::StepResult;

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
    let retried = if result.attempts > 1 {
        format!("  (attempt {})", result.attempts)
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
                "{mark} {pass}{}  {} {}  {status}  {}{retried}",
                place.shown(),
                response.request.method.as_str(),
                response.sent_path(),
                format_duration(response.latency)
            )
        }
        Err(_) => format!("{mark} {pass}{}  not sent{retried}", place.shown()),
    }
}

/// What did not hold, a line each: why it was not sent, the error it got,
/// the assertions that failed — with what was there — or could not be
/// checked; with none to check, the error status.
pub fn details(result: &StepResult, assertions: &[Assertion]) -> Vec<String> {
    cyberapi_core::run::details(&result.response, &result.outcomes, assertions)
}

/// The send as JUnit has it; `assertions` are its API's.
pub fn case(result: &StepResult, place: &Place, numbered: bool, assertions: &[Assertion]) -> Case {
    cyberapi_core::run::case_of(
        result.pass,
        place,
        numbered,
        &result.response,
        &result.outcomes,
        assertions,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::assertions::{Checked, Op, Subject};
    use cyberapi_core::response::HttpResponse;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    fn result(status: Option<u16>, outcomes: cyberapi_core::assertions::Outcomes) -> StepResult {
        StepResult {
            pass: 1,
            step: 0,
            attempts: 1,
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
