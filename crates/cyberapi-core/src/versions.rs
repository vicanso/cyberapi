//! G8: what changed between two versions of a project (`sync::ProjectConfig`)
//! — a version the server kept and the project now, or the version after
//! it: the folders and APIs added, removed, renamed, moved or edited, told
//! apart by their ids; the variables and environments likewise; and whether
//! the project's name, description, settings or order changed. An edited
//! API's request is compared line by line as [`request_text`] writes it.

use crate::model::{HttpMethod, HttpRequest, VariableCategory};
use crate::sync::ProjectConfig;
use crate::{EnvironmentId, ItemId};
use std::collections::HashMap;

/// What became of something both versions could have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Added,
    Removed,
    /// In both: renamed (from the name it had), moved to another folder,
    /// or edited — an API's request, a folder's settings, a variable's
    /// value or state.
    Changed {
        renamed_from: Option<String>,
        moved: bool,
        edited: bool,
    },
}

/// A folder or an API that changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemChange {
    pub id: ItemId,
    /// Its name in the newer version; the older's when it was removed.
    pub name: String,
    pub folder: bool,
    pub change: Change,
}

/// A variable or a named environment that changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedChange {
    pub name: String,
    /// A variable's category; `None` for an environment.
    pub category: Option<VariableCategory>,
    /// The environment a variable is in, by name; `None` for the shared
    /// layer, and for an environment itself.
    pub environment: Option<String>,
    pub change: Change,
}

/// What changed from one version to the other.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectChanges {
    /// The project's former name, when it was renamed.
    pub renamed_from: Option<String>,
    pub description: bool,
    /// How it signs in, which variables are secret, its source…
    pub settings: bool,
    /// Folders first, then APIs: those of the newer version in its order,
    /// then those removed.
    pub items: Vec<ItemChange>,
    pub variables: Vec<NamedChange>,
    pub environments: Vec<NamedChange>,
    /// Items in the same folders as before, put in another order.
    pub reordered: bool,
}

impl ProjectChanges {
    /// Nothing changed.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// An API's request as two of them are compared: its JSON, indented —
/// but for its body, its description and its examples, each written out
/// below it a line at a time, so a change inside one is a line of its own
/// (BUG-20). A gRPC call's definitions are told by their size and hash.
pub fn request_text(request: &HttpRequest) -> String {
    let rest = HttpRequest {
        body: String::new(),
        description: String::new(),
        examples: Vec::new(),
        request_schema: None,
        ..request.clone()
    };
    let mut text = serde_json::to_value(&rest)
        .map(|json| crate::format::json(&json))
        .unwrap_or_default();
    let mut part = |title: &str, body: &str| {
        if !body.trim().is_empty() {
            text += &format!("\n--- {title} ---\n{}", body.trim_end());
        }
    };
    match request.method {
        HttpMethod::Grpc => {
            let call = crate::grpc::GrpcBody::read(&request.body);
            let told = format!(
                "service: {}\nmethod: {}\nsource: {}\ndefinitions: {}",
                call.service,
                call.method,
                call.source,
                summed(&call.descriptors)
            );
            part("call", &told);
            part("message", &indented(&call.message));
        }
        _ if request.content_type == crate::model::ContentType::GraphQl => {
            let body = crate::graphql::GraphQlBody::read(&request.body);
            part("query", &body.query);
            part("variables", &indented(&body.variables));
            part("operation", &body.operation_name);
        }
        _ => part("body", &indented(&request.body)),
    }
    part("description", &request.description);
    if let Some(schema) = &request.request_schema {
        part("request schema", &indented(schema));
    }
    for example in &request.examples {
        let headers: String = example
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\n"))
            .collect();
        let told = format!("{}\n{headers}\n{}", example.status, indented(&example.body));
        part(&format!("example: {}", example.name), &told);
    }
    text
}

/// `text` indented as JSON, when it is; as it is otherwise.
fn indented(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .map(|json| crate::format::json(&json))
        .unwrap_or_else(|_| text.to_string())
}

/// What base64 `text` holds, told by its size and hash: a gRPC call's
/// definitions, too long a line to compare.
fn summed(text: &str) -> String {
    use sha2::Digest as _;
    let hash = sha2::Sha256::digest(text.as_bytes());
    let short: String = hash.iter().take(6).map(|byte| format!("{byte:02x}")).collect();
    format!("{} characters, sha256 {short}…", text.len())
}

/// Each item's folder (`None` the top level).
fn parents(config: &ProjectConfig) -> HashMap<&ItemId, &ItemId> {
    config
        .folders
        .iter()
        .flat_map(|folder| folder.children.iter().map(move |child| (child, &folder.id)))
        .collect()
}

/// What `items_old` became in `items_new`, by id, with what tells each
/// one: its id, its name, and whether its content is the same.
fn compared<T>(
    old: &[T],
    new: &[T],
    id: fn(&T) -> &ItemId,
    name: fn(&T) -> &str,
    same: fn(&T, &T) -> bool,
    moved: impl Fn(&ItemId) -> bool,
    folder: bool,
) -> Vec<ItemChange> {
    let older: HashMap<&ItemId, &T> = old.iter().map(|item| (id(item), item)).collect();
    let mut changes = Vec::new();
    for item in new {
        let change = match older.get(id(item)) {
            None => Some(Change::Added),
            Some(was) => {
                let renamed_from = (name(was) != name(item)).then(|| name(was).to_string());
                let (moved, edited) = (moved(id(item)), !same(was, item));
                (renamed_from.is_some() || moved || edited).then_some(Change::Changed {
                    renamed_from,
                    moved,
                    edited,
                })
            }
        };
        if let Some(change) = change {
            changes.push(ItemChange {
                id: id(item).clone(),
                name: name(item).to_string(),
                folder,
                change,
            });
        }
    }
    let newer: Vec<&ItemId> = new.iter().map(id).collect();
    for item in old {
        if !newer.contains(&id(item)) {
            changes.push(ItemChange {
                id: id(item).clone(),
                name: name(item).to_string(),
                folder,
                change: Change::Removed,
            });
        }
    }
    changes
}

/// What changed from `old` to `new`.
pub fn changes(old: &ProjectConfig, new: &ProjectConfig) -> ProjectChanges {
    let (parents_old, parents_new) = (parents(old), parents(new));
    let moved = |id: &ItemId| parents_old.get(id) != parents_new.get(id);
    let mut items = compared(
        &old.folders,
        &new.folders,
        |folder| &folder.id,
        |folder| folder.name.as_str(),
        |was, now| was.settings == now.settings,
        moved,
        true,
    );
    items.extend(compared(
        &old.apis,
        &new.apis,
        |api| &api.id,
        |api| api.name.as_str(),
        |was, now| was.request == now.request,
        moved,
        false,
    ));
    // The same items under each folder, in another order.
    let order_of = |config: &ProjectConfig| -> HashMap<Option<ItemId>, Vec<ItemId>> {
        let mut orders: HashMap<Option<ItemId>, Vec<ItemId>> = config
            .folders
            .iter()
            .map(|folder| (Some(folder.id.clone()), folder.children.clone()))
            .collect();
        orders.insert(None, config.top_order.clone());
        orders
    };
    let (orders_old, orders_new) = (order_of(old), order_of(new));
    let reordered = orders_new.iter().any(|(folder, order)| {
        orders_old.get(folder).is_some_and(|before| {
            let kept: Vec<&ItemId> = order.iter().filter(|id| before.contains(id)).collect();
            let was: Vec<&ItemId> = before.iter().filter(|id| order.contains(id)).collect();
            kept != was
        })
    });

    let environment_name = |config: &ProjectConfig, id: &Option<EnvironmentId>| {
        id.as_ref().map(|id| {
            config
                .environments
                .iter()
                .find(|environment| &environment.id == id)
                .map_or_else(|| id.to_string(), |environment| environment.name.clone())
        })
    };
    let mut variables = Vec::new();
    for variable in &new.variables {
        let change = match old.variables.iter().find(|was| was.id == variable.id) {
            None => Some(Change::Added),
            Some(was) => {
                let renamed_from = (was.name != variable.name).then(|| was.name.clone());
                let moved = was.environment != variable.environment || was.category != variable.category;
                let edited = was.value != variable.value || was.enabled != variable.enabled;
                (renamed_from.is_some() || moved || edited).then_some(Change::Changed {
                    renamed_from,
                    moved,
                    edited,
                })
            }
        };
        if let Some(change) = change {
            variables.push(NamedChange {
                name: variable.name.clone(),
                category: Some(variable.category),
                environment: environment_name(new, &variable.environment),
                change,
            });
        }
    }
    for variable in &old.variables {
        if !new.variables.iter().any(|now| now.id == variable.id) {
            variables.push(NamedChange {
                name: variable.name.clone(),
                category: Some(variable.category),
                environment: environment_name(old, &variable.environment),
                change: Change::Removed,
            });
        }
    }

    let mut environments = Vec::new();
    for environment in &new.environments {
        let change = match old.environments.iter().find(|was| was.id == environment.id) {
            None => Some(Change::Added),
            Some(was) if was.name != environment.name => Some(Change::Changed {
                renamed_from: Some(was.name.clone()),
                moved: false,
                edited: false,
            }),
            Some(_) => None,
        };
        if let Some(change) = change {
            environments.push(NamedChange {
                name: environment.name.clone(),
                category: None,
                environment: None,
                change,
            });
        }
    }
    for environment in &old.environments {
        if !new.environments.iter().any(|now| now.id == environment.id) {
            environments.push(NamedChange {
                name: environment.name.clone(),
                category: None,
                environment: None,
                change: Change::Removed,
            });
        }
    }

    ProjectChanges {
        renamed_from: (old.name != new.name).then(|| old.name.clone()),
        description: old.description != new.description,
        settings: old.settings != new.settings,
        items,
        variables,
        environments,
        reordered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ApiSetting, Environment, Folder, FolderSettings, HttpMethod, Variable};
    use pretty_assertions::assert_eq;

    fn api(id: &str, name: &str, uri: &str) -> ApiSetting {
        ApiSetting {
            id: id.into(),
            name: name.into(),
            request: HttpRequest {
                uri: uri.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn folder(id: &str, name: &str, children: &[&str]) -> Folder {
        Folder {
            id: id.into(),
            name: name.into(),
            children: children.iter().map(|child| ItemId::from(*child)).collect(),
            ..Default::default()
        }
    }

    fn variable(id: &str, name: &str, value: &str) -> Variable {
        Variable {
            id: id.into(),
            collection: "c".into(),
            category: VariableCategory::Environment,
            name: name.into(),
            value: value.into(),
            enabled: true,
            environment: None,
            created_at: Default::default(),
            updated_at: Default::default(),
        }
    }

    fn old() -> ProjectConfig {
        ProjectConfig {
            name: "shop".into(),
            folders: vec![folder("f-users", "users", &["a-list", "a-one"])],
            apis: vec![
                api("a-list", "list", "/users"),
                api("a-one", "one", "/users/{id}"),
                api("a-login", "login", "/login"),
            ],
            top_order: vec!["f-users".into(), "a-login".into()],
            variables: vec![variable("v-host", "HOST", "a"), variable("v-token", "TOKEN", "t")],
            environments: vec![Environment {
                id: "e-dev".into(),
                name: "dev".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn the_same_project_has_no_changes() {
        assert!(changes(&old(), &old()).is_empty());
    }

    #[test]
    fn items_are_told_apart_by_their_ids() {
        let mut new = old();
        new.name = "store".into();
        // Renamed and moved out of `users`; edited; removed; added.
        new.folders = vec![Folder {
            settings: FolderSettings {
                description: "people".into(),
                ..Default::default()
            },
            ..folder("f-users", "users", &["a-list"])
        }];
        new.apis = vec![
            api("a-list", "list", "/users?page=1"),
            api("a-one", "get one", "/users/{id}"),
            api("a-new", "logout", "/logout"),
        ];
        new.apis[2].request.method = HttpMethod::Post;
        new.top_order = vec!["a-one".into(), "f-users".into(), "a-new".into()];
        new.variables = vec![variable("v-host", "HOST", "b"), variable("v-key", "KEY", "k")];
        new.environments[0].name = "develop".into();
        let found = changes(&old(), &new);
        assert_eq!(found.renamed_from.as_deref(), Some("shop"));
        assert!(!found.description && !found.settings);
        let items: Vec<(&str, &Change)> = found
            .items
            .iter()
            .map(|item| (item.name.as_str(), &item.change))
            .collect();
        assert_eq!(
            items,
            [
                (
                    "users",
                    &Change::Changed {
                        renamed_from: None,
                        moved: false,
                        edited: true
                    }
                ),
                (
                    "list",
                    &Change::Changed {
                        renamed_from: None,
                        moved: false,
                        edited: true
                    }
                ),
                (
                    "get one",
                    &Change::Changed {
                        renamed_from: Some("one".into()),
                        moved: true,
                        edited: false
                    }
                ),
                ("logout", &Change::Added),
                ("login", &Change::Removed),
            ]
        );
        let variables: Vec<(&str, &Change)> = found
            .variables
            .iter()
            .map(|variable| (variable.name.as_str(), &variable.change))
            .collect();
        assert_eq!(
            variables,
            [
                (
                    "HOST",
                    &Change::Changed {
                        renamed_from: None,
                        moved: false,
                        edited: true
                    }
                ),
                ("KEY", &Change::Added),
                ("TOKEN", &Change::Removed),
            ]
        );
        assert_eq!(
            found.environments,
            [NamedChange {
                name: "develop".into(),
                category: None,
                environment: None,
                change: Change::Changed {
                    renamed_from: Some("dev".into()),
                    moved: false,
                    edited: false
                },
            }]
        );
        assert!(!found.reordered, "what moved is told as moved, not as reordered");
    }

    #[test]
    fn the_same_items_in_another_order_are_reordered() {
        let mut new = old();
        new.folders[0].children.reverse();
        let found = changes(&old(), &new);
        assert!(found.reordered);
        assert!(found.items.is_empty());
    }

    #[test]
    fn a_request_is_compared_as_indented_json() {
        let text = request_text(&api("a", "a", "/users").request);
        assert!(text.contains("\n    \"uri\": \"/users\""), "{text}");
    }

    /// BUG-20: a change inside a body, a description or an example is a
    /// line of its own; a gRPC call's definitions are not a line at all.
    #[test]
    fn a_requests_long_parts_are_compared_a_line_at_a_time() {
        use crate::model::Example;
        let request = |id: u32, note: &str| HttpRequest {
            body: format!(r#"{{"user": "ann", "id": {id}, "tags": ["a", "b"]}}"#),
            description: format!("Signs in.\n{note}"),
            examples: vec![Example {
                name: "ok".into(),
                status: 200,
                headers: vec![("content-type".into(), "application/json".into())],
                body: format!(r#"{{"id": {id}}}"#),
            }],
            ..Default::default()
        };
        let (old, new) = (request_text(&request(1, "Old.")), request_text(&request(2, "New.")));
        let changed: Vec<(&str, &str)> = old.lines().zip(new.lines()).filter(|(a, b)| a != b).collect();
        assert_eq!(
            changed,
            [
                (r#"    "id": 1,"#, r#"    "id": 2,"#),
                ("Old.", "New."),
                (r#"    "id": 1"#, r#"    "id": 2"#)
            ],
            "{new}"
        );
        assert!(new.contains("--- description ---\nSigns in.\nNew."), "{new}");

        let call = HttpRequest {
            method: HttpMethod::Grpc,
            body: crate::grpc::GrpcBody {
                service: "shop.Store".into(),
                method: "Get".into(),
                message: r#"{"id": "7"}"#.into(),
                descriptors: "A".repeat(100_000),
                source: "shop.proto".into(),
            }
            .to_body(),
            ..Default::default()
        };
        let text = request_text(&call);
        assert!(text.lines().all(|line| line.len() < 200), "no long line");
        assert!(text.contains("definitions: 100000 characters, sha256 "), "{text}");
        assert!(text.contains("--- message ---\n{\n    \"id\": \"7\"\n}"), "{text}");
    }
}
