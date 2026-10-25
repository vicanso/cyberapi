//! Checks every backend makes the same way.

use crate::{Entity, Refusal, StoreError};
use cyberapi_core::environments::name_taken;
use cyberapi_core::model::Environment;
use cyberapi_core::sync::ProjectConfig;
use cyberapi_core::{CollectionId, EnvironmentId};
use std::collections::HashSet;

/// A project's or an item's name: required, kept without surrounding blanks.
pub fn required_name(name: &str) -> Result<String, StoreError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(StoreError::NameRequired);
    }
    Ok(name.to_string())
}

/// A configuration as a store writes it into `collection`: every name the
/// trees require is there, trimmed; every record says it is the project's;
/// no id is given twice — folders and APIs share theirs.
pub fn project_config(collection: &CollectionId, mut config: ProjectConfig) -> Result<ProjectConfig, StoreError> {
    config.name = required_name(&config.name)?;
    let mut items = HashSet::new();
    for folder in &mut config.folders {
        folder.name = required_name(&folder.name)?;
        folder.collection = collection.clone();
        if !items.insert(folder.id.clone()) {
            return Err(taken(&folder.id));
        }
    }
    for api in &mut config.apis {
        api.name = required_name(&api.name)?;
        api.collection = collection.clone();
        if !items.insert(api.id.clone()) {
            return Err(taken(&api.id));
        }
    }
    // Named environments (#61): one name each, each id once; a variable's
    // is one of them.
    let mut environments: Vec<Environment> = Vec::new();
    for environment in &mut config.environments {
        environment.name = environment_name(&environments, &environment.name, None)?;
        environment.collection = collection.clone();
        if environments.iter().any(|found| found.id == environment.id) {
            return Err(taken(&environment.id));
        }
        environments.push(environment.clone());
    }
    let mut variables = HashSet::new();
    for variable in &mut config.variables {
        variable.collection = collection.clone();
        if !variables.insert(variable.id.clone()) {
            return Err(taken(&variable.id));
        }
        if let Some(id) = &variable.environment {
            known_environment(&environments, id)?;
        }
    }
    Ok(config)
}

/// An environment's name (#61): required, trimmed, and not one of
/// `environments`' (but `except`'s, being renamed).
pub fn environment_name(
    environments: &[Environment],
    name: &str,
    except: Option<&EnvironmentId>,
) -> Result<String, StoreError> {
    let name = required_name(name)?;
    if name_taken(environments, &name, except) {
        return Err(StoreError::Refused {
            reason: Refusal::EnvironmentNameTaken,
        });
    }
    Ok(name)
}

/// A variable's environment is one of the project's.
pub fn known_environment(environments: &[Environment], id: &EnvironmentId) -> Result<(), StoreError> {
    if environments.iter().any(|environment| &environment.id == id) {
        Ok(())
    } else {
        Err(StoreError::not_found(Entity::Environment, id))
    }
}

fn taken(id: &impl ToString) -> StoreError {
    StoreError::IdTaken { id: id.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::model::{ApiSetting, Folder};

    /// #61: one name and one id per environment; a variable's is one of
    /// them.
    #[test]
    fn a_configs_environments_are_checked() {
        use cyberapi_core::model::Variable;
        let environment = |id: &str, name: &str| Environment {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        };
        let variable = |environment: Option<&str>| Variable {
            id: "v".into(),
            collection: "x".into(),
            category: cyberapi_core::model::VariableCategory::Custom,
            name: "token".into(),
            value: "t".into(),
            enabled: true,
            environment: environment.map(EnvironmentId::from),
            created_at: Default::default(),
            updated_at: Default::default(),
        };
        let config = ProjectConfig {
            name: "p".into(),
            environments: vec![environment("e1", " dev "), environment("e2", "prod")],
            variables: vec![variable(Some("e1"))],
            ..Default::default()
        };
        let mine = CollectionId::new("c");
        let checked = project_config(&mine, config.clone()).expect("fine");
        assert_eq!(checked.environments[0].name, "dev");
        assert!(
            checked
                .environments
                .iter()
                .all(|environment| environment.collection == mine)
        );

        let mut twice = config.clone();
        twice.environments[1].name = "DEV".into();
        assert_eq!(
            project_config(&mine, twice),
            Err(StoreError::Refused {
                reason: Refusal::EnvironmentNameTaken
            })
        );
        let mut same_id = config.clone();
        same_id.environments[1].id = "e1".into();
        assert_eq!(
            project_config(&mine, same_id),
            Err(StoreError::IdTaken { id: "e1".into() })
        );
        let mut nowhere = config;
        nowhere.variables = vec![variable(Some("gone"))];
        assert_eq!(
            project_config(&mine, nowhere),
            Err(StoreError::not_found(Entity::Environment, "gone"))
        );
    }

    #[test]
    fn a_config_is_made_the_projects_and_checked() {
        let config = ProjectConfig {
            name: " p ".into(),
            folders: vec![Folder {
                id: "a".into(),
                name: " f ".into(),
                ..Default::default()
            }],
            apis: vec![ApiSetting {
                id: "b".into(),
                name: "api".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mine = CollectionId::new("c");
        let checked = project_config(&mine, config.clone()).expect("fine");
        assert_eq!((checked.name.as_str(), checked.folders[0].name.as_str()), ("p", "f"));
        assert!(checked.apis.iter().all(|api| api.collection == mine));

        let mut twice = config.clone();
        twice.apis[0].id = "a".into();
        assert_eq!(
            project_config(&mine, twice),
            Err(StoreError::IdTaken { id: "a".into() })
        );
        let mut nameless = config;
        nameless.apis[0].name = " ".into();
        assert_eq!(project_config(&mine, nameless), Err(StoreError::NameRequired));
    }

    #[test]
    fn names_are_trimmed_and_required() {
        assert_eq!(required_name("  users  "), Ok("users".to_string()));
        assert_eq!(required_name(" \t"), Err(StoreError::NameRequired));
    }
}
