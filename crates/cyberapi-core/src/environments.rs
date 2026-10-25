//! Named environments (#61, `docs/batch4-design.md` §5): a project's
//! variables of every category come in layers — the ones every environment
//! shares, and each named environment's. Sending in an environment takes,
//! for each name of each category, the environment's enabled variables if
//! it has one, and the shared ones otherwise; in none, the shared ones
//! alone, as before there were environments.

use crate::EnvironmentId;
use crate::model::{Environment, Variable, VariableCategory};

/// The variables a send in `current` reads, as the old rules read a
/// project's: the shared ones not stood in for, then the environment's. A
/// global header's name is matched as headers are, without case; the
/// others as written.
pub fn effective(variables: &[Variable], current: Option<&EnvironmentId>) -> Vec<Variable> {
    let Some(current) = current else {
        return variables
            .iter()
            .filter(|variable| variable.environment.is_none())
            .cloned()
            .collect();
    };
    let same = |a: &Variable, b: &Variable| {
        a.category == b.category
            && if a.category == VariableCategory::GlobalRequestHeader {
                a.name.trim().eq_ignore_ascii_case(b.name.trim())
            } else {
                a.name == b.name
            }
    };
    let own: Vec<&Variable> = variables
        .iter()
        .filter(|variable| variable.environment.as_ref() == Some(current) && variable.enabled)
        .collect();
    variables
        .iter()
        .filter(|variable| variable.environment.is_none() && !own.iter().any(|mine| same(mine, variable)))
        .chain(own.iter().copied())
        .cloned()
        .collect()
}

/// Whether `name` is one of `environments`' already, `except` the one being
/// renamed: names are told apart without case or the blanks around them.
pub fn name_taken(environments: &[Environment], name: &str, except: Option<&EnvironmentId>) -> bool {
    let name = name.trim();
    environments
        .iter()
        .filter(|environment| Some(&environment.id) != except)
        .any(|environment| environment.name.trim().eq_ignore_ascii_case(name))
}

/// `environments` in `order`: those it names first, as it names them; the
/// rest after, as they were. An id that is none of them is passed over.
pub fn reordered(environments: &[Environment], order: &[EnvironmentId]) -> Vec<Environment> {
    let mut placed: Vec<Environment> = Vec::new();
    for id in order {
        if !placed.iter().any(|found| &found.id == id)
            && let Some(found) = environments.iter().find(|environment| &environment.id == id)
        {
            placed.push(found.clone());
        }
    }
    let rest: Vec<Environment> = environments
        .iter()
        .filter(|environment| !placed.iter().any(|found| found.id == environment.id))
        .cloned()
        .collect();
    placed.extend(rest);
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn variable(category: VariableCategory, name: &str, value: &str, environment: Option<&str>) -> Variable {
        Variable {
            id: format!("{name}-{value}").into(),
            collection: "c".into(),
            category,
            name: name.into(),
            value: value.into(),
            enabled: true,
            environment: environment.map(EnvironmentId::from),
            created_at: Default::default(),
            updated_at: Default::default(),
        }
    }

    fn values(variables: &[Variable]) -> Vec<String> {
        variables
            .iter()
            .map(|variable| format!("{}={}", variable.name, variable.value))
            .collect()
    }

    fn sample() -> Vec<Variable> {
        use VariableCategory::*;
        let mut off = variable(Custom, "token", "dev-off", Some("dev"));
        off.enabled = false;
        vec![
            variable(Environment, "HOST", "shared", None),
            variable(Custom, "token", "shared", None),
            variable(Custom, "user", "shared", None),
            variable(GlobalRequestHeader, "X-Team", "shared", None),
            variable(GlobalRequestHeader, "X-Trace", "shared", None),
            variable(Environment, "HOST", "dev", Some("dev")),
            off,
            variable(GlobalRequestHeader, "x-team", "dev", Some("dev")),
            variable(Environment, "HOST", "prod", Some("prod")),
        ]
    }

    #[test]
    fn an_environment_stands_in_for_the_shared_names_it_has() {
        let dev = effective(&sample(), Some(&"dev".into()));
        assert_eq!(
            values(&dev),
            [
                "token=shared",
                "user=shared",
                "X-Trace=shared",
                "HOST=dev",
                "x-team=dev"
            ],
            "a header by its name without case; a turned-off one stands in for nothing"
        );
        let none = effective(&sample(), None);
        assert_eq!(
            values(&none),
            [
                "HOST=shared",
                "token=shared",
                "user=shared",
                "X-Team=shared",
                "X-Trace=shared"
            ],
            "no environment: the shared ones, as before"
        );
        let gone = effective(&sample(), Some(&"gone".into()));
        assert_eq!(
            values(&gone),
            values(&none),
            "an environment that is not there has nothing"
        );
    }

    #[test]
    fn names_are_one_each_and_orders_keep_the_rest() {
        let environment = |id: &str, name: &str| Environment {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        };
        let list = [
            environment("a", "dev"),
            environment("b", "Prod"),
            environment("c", "qa"),
        ];
        assert!(name_taken(&list, " prod ", None));
        assert!(!name_taken(&list, "prod", Some(&"b".into())), "its own name");
        assert!(!name_taken(&list, "staging", None));
        let ids = |list: Vec<Environment>| list.into_iter().map(|e| e.id.to_string()).collect::<Vec<_>>();
        assert_eq!(
            ids(reordered(&list, &["c".into(), "x".into(), "a".into()])),
            ["c", "a", "b"]
        );
        assert_eq!(ids(reordered(&list, &[])), ["a", "b", "c"]);
    }
}
