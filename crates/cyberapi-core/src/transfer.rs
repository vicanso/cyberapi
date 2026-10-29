//! F5: an item of a project's tree copied or moved into another project:
//! a folder with its settings and everything in it, an API with its whole
//! request, as an import into the other project gives them (new ids); and
//! the variables they read that the other project may not have.

use crate::ItemId;
use crate::import::ImportItem;
use crate::model::{HttpRequest, split_env_prefix};
use crate::tree::TreeData;
use serde_json::Value;
use std::collections::BTreeSet;

/// `id` of `tree`, and everything in it, as an import writes it; none when
/// the tree has no such item.
pub fn item(tree: &TreeData, id: &ItemId) -> Option<ImportItem> {
    fn walk(tree: &TreeData, id: &ItemId, seen: &mut Vec<ItemId>) -> Option<ImportItem> {
        if let Some(api) = tree.apis.iter().find(|api| &api.id == id) {
            return Some(ImportItem::Api {
                name: api.name.clone(),
                request: Box::new(api.request.clone()),
            });
        }
        let folder = tree.folders.iter().find(|folder| &folder.id == id)?;
        // A folder that lists one above it: the store's rules prevent it,
        // but a damaged tree must not take forever.
        if seen.contains(id) {
            return None;
        }
        seen.push(id.clone());
        Some(ImportItem::Folder {
            name: folder.name.clone(),
            children: folder
                .children
                .iter()
                .filter_map(|child| walk(tree, child, seen))
                .collect(),
            settings: folder.settings.clone(),
        })
    }
    walk(tree, id, &mut Vec::new())
}

/// The requests in `item`, and the variables its folders set (D5).
fn requests_and_own<'a>(item: &'a ImportItem, requests: &mut Vec<&'a HttpRequest>, own: &mut BTreeSet<String>) {
    match item {
        ImportItem::Api { request, .. } => {
            own.extend(request.variables.iter().map(|kv| kv.key.trim().to_string()));
            requests.push(request);
        }
        ImportItem::Folder { children, settings, .. } => {
            own.extend(settings.variables.iter().map(|kv| kv.key.trim().to_string()));
            for child in children {
                requests_and_own(child, requests, own);
            }
        }
    }
}

/// Each text of `value`, its descriptions and examples aside.
fn texts<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => out.push(text),
        Value::Array(items) => items.iter().for_each(|item| texts(item, out)),
        Value::Object(fields) => fields
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "description" | "examples"))
            .for_each(|(_, item)| texts(item, out)),
        _ => {}
    }
}

/// The variables `item` reads — an address's `{{NAME}}`, a `value(name)`
/// anywhere in a request — and does not set itself.
pub fn read_variables(item: &ImportItem) -> BTreeSet<String> {
    let (mut requests, mut own) = (Vec::new(), BTreeSet::new());
    requests_and_own(item, &mut requests, &mut own);
    let mut read = BTreeSet::new();
    for request in requests {
        if let (Some(name), _) = split_env_prefix(request.uri.trim()) {
            read.insert(name.to_string());
        }
        let json = serde_json::to_value(request).unwrap_or_default();
        let mut found = Vec::new();
        texts(&json, &mut found);
        for text in found {
            for expr in crate::template::parse(text) {
                if expr.chain.last().is_some_and(|name| name == "value")
                    && let Some(name) = expr.params.first().filter(|name| !name.is_empty())
                {
                    read.insert(name.clone());
                }
            }
        }
    }
    read.retain(|name| !own.contains(name));
    read
}

/// Those of `read` that are none of `names`.
pub fn missing<S: AsRef<str>>(read: &BTreeSet<String>, names: &[S]) -> Vec<String> {
    read.iter()
        .filter(|name| !names.iter().any(|given| given.as_ref() == name.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FolderSettings, KvParam};
    use crate::tree::fixture::sample;
    use pretty_assertions::assert_eq;

    fn kv(key: &str, value: &str) -> KvParam {
        KvParam {
            key: key.into(),
            value: value.into(),
            enabled: true,
        }
    }

    /// `sample()`: users ⊃ login, profile ⊃ me; orders ⊃ list; health on
    /// top.
    #[test]
    fn a_folder_goes_whole() {
        let mut tree = sample();
        tree.folders[0].settings = FolderSettings {
            description: "people".into(),
            ..Default::default()
        };
        let Some(ImportItem::Folder {
            name,
            children,
            settings,
        }) = item(&tree, &"users".into())
        else {
            panic!("a folder");
        };
        assert_eq!(name, "用户");
        assert_eq!(settings.description, "people");
        let names: Vec<&str> = children
            .iter()
            .map(|child| match child {
                ImportItem::Api { name, .. } | ImportItem::Folder { name, .. } => name.as_str(),
            })
            .collect();
        assert_eq!(names, ["登录", "profile"]);
        assert!(matches!(item(&tree, &"health".into()), Some(ImportItem::Api { .. })));
        assert!(item(&tree, &"nothing".into()).is_none());
    }

    #[test]
    fn what_it_reads_and_does_not_set() {
        let mut tree = sample();
        let login = tree
            .apis
            .iter_mut()
            .find(|api| api.id.as_str() == "login")
            .expect("login");
        login.request.headers = vec![kv("X-Tenant", "{{value(tenant)}}"), kv("X-Sign", "{{md5.value(key)}}")];
        login.request.body = r#"{"user": "{{value(user)}}"}"#.into();
        login.request.description = "{{value(not_read)}}".into();
        login.request.variables = vec![kv("user", "ann")];
        tree.folders[0].settings.variables = vec![kv("key", "k")];
        let users = item(&tree, &"users".into()).expect("a folder");
        assert_eq!(
            read_variables(&users).into_iter().collect::<Vec<_>>(),
            ["HOST", "tenant"],
            "set by the folder and the API themselves: not missing"
        );
        assert_eq!(missing(&read_variables(&users), &["HOST"]), ["tenant"]);
    }
}
