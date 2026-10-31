//! Named environments (#61, `docs/batch4-design.md` §5): a project's
//! variables of every category come in layers — the ones every environment
//! shares, and each named environment's. Sending in an environment takes,
//! for each name of each category, the environment's enabled variables if
//! it has one, and the shared ones otherwise; in none, the shared ones
//! alone, as before there were environments.

use crate::graphql::GraphQlBody;
use crate::model::{ContentType, Environment, HttpMethod, HttpRequest, Variable, VariableCategory};
use crate::{EnvironmentId, VariableId};

/// D6: whether sending `request` in an environment marked as production is
/// asked about first: whatever may change something there — an HTTP method
/// other than GET, HEAD or OPTIONS, a GraphQL mutation, a gRPC call, a
/// WebSocket connection. A GraphQL query or subscription is not, whatever
/// its method.
pub fn asks_first(request: &HttpRequest) -> bool {
    if request.content_type == ContentType::GraphQl
        && !matches!(request.method, HttpMethod::Grpc | HttpMethod::WebSocket)
    {
        return GraphQlBody::read(&request.body).is_mutation();
    }
    !matches!(request.method, HttpMethod::Get | HttpMethod::Head | HttpMethod::Options)
}

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

fn same_variable_name(category: VariableCategory, a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    if category == VariableCategory::GlobalRequestHeader {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// D8: distinct non-empty variable names across all layers of `category`,
/// in order of first appearance (shared layer first, then named environments).
pub fn matrix_names(variables: &[Variable], category: VariableCategory) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let shared = variables.iter().filter(|v| v.environment.is_none());
    let named = variables.iter().filter(|v| v.environment.is_some());
    for variable in shared.chain(named).filter(|v| v.category == category) {
        let name = variable.name.trim();
        if !name.is_empty() && !names.iter().any(|seen| same_variable_name(category, seen, name)) {
            names.push(name.to_string());
        }
    }
    names
}

/// D8: the variable representing `name` in `environment` (`None` for the
/// shared layer), preferring an enabled entry over a disabled one.
pub fn layer_variable<'a>(
    variables: &'a [Variable],
    category: VariableCategory,
    environment: Option<&EnvironmentId>,
    name: &str,
) -> Option<&'a Variable> {
    let mut matches = variables.iter().filter(|variable| {
        variable.category == category
            && variable.environment.as_ref() == environment
            && same_variable_name(category, &variable.name, name)
    });
    let first = matches.next()?;
    if first.enabled {
        return Some(first);
    }
    matches.find(|variable| variable.enabled).or(Some(first))
}

/// D8: redundant duplicate variable IDs within the same layer of `category`
/// (keeping the primary entry chosen by [`layer_variable`] for each name and
/// layer).
pub fn duplicate_ids(variables: &[Variable], category: VariableCategory) -> Vec<VariableId> {
    let mut kept: Vec<VariableId> = Vec::new();
    let mut duplicates: Vec<VariableId> = Vec::new();
    for variable in variables.iter().filter(|v| v.category == category) {
        let name = variable.name.trim();
        if name.is_empty() {
            continue;
        }
        if let Some(primary) = layer_variable(variables, category, variable.environment.as_ref(), name) {
            if primary.id == variable.id && !kept.contains(&variable.id) {
                kept.push(variable.id.clone());
            } else {
                duplicates.push(variable.id.clone());
            }
        }
    }
    duplicates
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn what_may_change_something_is_asked_about_first() {
        let request = |method: HttpMethod| HttpRequest {
            method,
            ..Default::default()
        };
        for method in [HttpMethod::Get, HttpMethod::Head, HttpMethod::Options] {
            assert!(!asks_first(&request(method)), "{method:?}");
        }
        for method in [
            HttpMethod::Post,
            HttpMethod::Put,
            HttpMethod::Patch,
            HttpMethod::Delete,
            HttpMethod::Grpc,
            HttpMethod::WebSocket,
        ] {
            assert!(asks_first(&request(method)), "{method:?}");
        }
        let graphql = |query: &str| HttpRequest {
            method: HttpMethod::Post,
            content_type: ContentType::GraphQl,
            body: GraphQlBody {
                query: query.into(),
                ..Default::default()
            }
            .to_body(),
            ..Default::default()
        };
        assert!(!asks_first(&graphql("{ me { id } }")), "a query, though POSTed");
        assert!(!asks_first(&graphql("subscription { tick }")));
        assert!(asks_first(&graphql("mutation { logout }")));
    }

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

    #[test]
    fn matrix_names_and_duplicates_group_by_layer_and_category_case_rules() {
        use VariableCategory::*;
        let mut vars = sample();
        vars.push(variable(Environment, "ONLY_PROD", "1", Some("prod")));
        vars.push(variable(Environment, "HOST", "dup-dev", Some("dev")));
        assert_eq!(matrix_names(&vars, Environment), ["HOST", "ONLY_PROD"]);
        assert_eq!(matrix_names(&vars, GlobalRequestHeader), ["X-Team", "X-Trace"]);
        assert_eq!(
            duplicate_ids(&vars, Environment),
            vec![VariableId::from("HOST-dup-dev")]
        );
    }
}
