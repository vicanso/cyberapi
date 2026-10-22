//! Checks every backend makes the same way.

use crate::StoreError;
use cyberapi_core::CollectionId;
use cyberapi_core::sync::ProjectConfig;
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
    let mut variables = HashSet::new();
    for variable in &mut config.variables {
        variable.collection = collection.clone();
        if !variables.insert(variable.id.clone()) {
            return Err(taken(&variable.id));
        }
    }
    Ok(config)
}

fn taken(id: &impl ToString) -> StoreError {
    StoreError::IdTaken { id: id.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::model::{ApiSetting, Folder};

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
