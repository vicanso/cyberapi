//! F6: an item deleted from a project's tree, kept a while so that it can be
//! put back as it was — its id and those of everything in it, where it
//! stood, a folder's settings and a request's whole content. Put back into
//! the project's configuration as it is by then, which is then written
//! whole (`TreeStore::replace`, which keeps the ids it brings and the
//! history of what stays). The history of the APIs that were deleted went
//! with them and does not come back.

use crate::ItemId;
use crate::model::{ApiSetting, Folder};
use crate::sync::ProjectConfig;
use crate::tree::TreeData;

/// What a delete took, and where it stood.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deleted {
    /// The item deleted: a folder or an API.
    pub item: ItemId,
    pub name: String,
    /// The folder it was in; `None` the top level.
    pub parent: Option<ItemId>,
    /// Its place among the parent's items.
    pub index: usize,
    /// It, if a folder, and the folders in it.
    pub folders: Vec<Folder>,
    /// It, if an API, and the APIs in it.
    pub apis: Vec<ApiSetting>,
}

impl Deleted {
    /// Whether `id` is one of what was deleted.
    pub fn holds(&self, id: &ItemId) -> bool {
        self.folders.iter().any(|folder| &folder.id == id) || self.apis.iter().any(|api| &api.id == id)
    }
}

/// What deleting `item` from `tree` takes: none when the tree has no such
/// item.
pub fn deleted(tree: &TreeData, item: &ItemId) -> Option<Deleted> {
    let mut folders = Vec::new();
    let mut apis = Vec::new();
    let mut waiting = vec![item.clone()];
    while let Some(id) = waiting.pop() {
        if let Some(folder) = tree.folders.iter().find(|folder| folder.id == id) {
            // A folder that lists one above it: the store's rules prevent
            // it, but a damaged tree must not take forever.
            if folders.iter().any(|seen: &Folder| seen.id == folder.id) {
                continue;
            }
            waiting.extend(folder.children.iter().cloned());
            folders.push(folder.clone());
        } else if let Some(api) = tree.apis.iter().find(|api| api.id == id) {
            apis.push(api.clone());
        }
    }
    let name = folders
        .iter()
        .find(|folder| &folder.id == item)
        .map(|folder| folder.name.clone())
        .or_else(|| apis.iter().find(|api| &api.id == item).map(|api| api.name.clone()))?;
    let (parent, index) = match tree.folders.iter().find(|folder| folder.children.contains(item)) {
        Some(folder) => (
            Some(folder.id.clone()),
            folder.children.iter().position(|child| child == item).unwrap_or(0),
        ),
        None => (None, tree.top_order.iter().position(|top| top == item).unwrap_or(0)),
    };
    Some(Deleted {
        item: item.clone(),
        name,
        parent,
        index,
        folders,
        apis,
    })
}

/// `deleted` put back into `config`, where it stood: in its folder at its
/// place, or at the end of the top level when its folder is gone too. What
/// `config` has of it already stays as it is. Whether anything was put
/// back: nothing, when it is all there again (BUG-24).
pub fn put_back(config: &mut ProjectConfig, deleted: &Deleted) -> bool {
    let mut changed = false;
    for folder in &deleted.folders {
        if !config.folders.iter().any(|found| found.id == folder.id) {
            config.folders.push(folder.clone());
            changed = true;
        }
    }
    for api in &deleted.apis {
        if !config.apis.iter().any(|found| found.id == api.id) {
            config.apis.push(api.clone());
            changed = true;
        }
    }
    let placed = config.top_order.contains(&deleted.item)
        || config
            .folders
            .iter()
            .any(|folder| folder.id != deleted.item && folder.children.contains(&deleted.item));
    if placed {
        return changed;
    }
    let parent = deleted
        .parent
        .as_ref()
        .and_then(|parent| config.folders.iter_mut().find(|folder| &folder.id == parent));
    let order = match parent {
        Some(folder) => &mut folder.children,
        None if deleted.parent.is_none() => &mut config.top_order,
        // Its folder is gone: the end of the top level.
        None => {
            config.top_order.push(deleted.item.clone());
            return true;
        }
    };
    let at = deleted.index.min(order.len());
    order.insert(at, deleted.item.clone());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::fixture::sample;
    use pretty_assertions::assert_eq;

    fn config(tree: &TreeData) -> ProjectConfig {
        ProjectConfig {
            folders: tree.folders.clone(),
            apis: tree.apis.clone(),
            top_order: tree.top_order.clone(),
            settings: tree.settings.clone(),
            ..Default::default()
        }
    }

    /// `sample()`: users ⊃ login, profile ⊃ me; orders ⊃ list; health on
    /// top.
    #[test]
    fn a_folder_comes_back_whole_where_it_was() {
        let tree = sample();
        let users = deleted(&tree, &"users".into()).expect("a folder");
        assert_eq!(users.name, "用户");
        assert_eq!(users.parent, None);
        let mut held: Vec<&str> = users
            .folders
            .iter()
            .map(|folder| folder.id.as_str())
            .chain(users.apis.iter().map(|api| api.id.as_str()))
            .collect();
        held.sort();
        assert_eq!(held, ["login", "me", "profile", "users"]);

        // Deleted, then something else changed, then put back.
        let mut after = config(&tree);
        after.folders.retain(|folder| !users.holds(&folder.id));
        after.apis.retain(|api| !users.holds(&api.id));
        after.top_order.retain(|id| id != &users.item);
        after.apis.iter_mut().for_each(|api| api.name.push('!'));
        assert!(put_back(&mut after, &users));
        assert_eq!(after.top_order, tree.top_order, "at its place");
        let before = config(&tree);
        for folder in &before.folders {
            assert!(after.folders.contains(folder), "{}", folder.id);
        }
        assert!(after.apis.iter().any(|api| api.id.as_str() == "me" && api.name == "me"));
        assert!(
            after.apis.iter().any(|api| api.name.ends_with('!')),
            "the rest as it is now"
        );

        // Twice is once.
        let once = after.clone();
        assert!(!put_back(&mut after, &users), "nothing put back");
        assert_eq!(after, once);
    }

    #[test]
    fn an_api_comes_back_into_its_folder_or_on_top_without_it() {
        let tree = sample();
        let me = deleted(&tree, &"me".into()).expect("an API");
        assert_eq!(me.parent.as_ref().map(ItemId::as_str), Some("profile"));
        let mut after = config(&tree);
        after.apis.retain(|api| api.id != me.item);
        for folder in &mut after.folders {
            folder.children.retain(|child| child != &me.item);
        }
        let mut back = after.clone();
        assert!(put_back(&mut back, &me));
        assert_eq!(back.folders, config(&tree).folders);

        // Its folder deleted since: the end of the top level.
        after.folders.retain(|folder| folder.id.as_str() != "profile");
        for folder in &mut after.folders {
            folder.children.retain(|child| child.as_str() != "profile");
        }
        assert!(put_back(&mut after, &me));
        assert_eq!(after.top_order.last(), Some(&me.item));
        assert!(deleted(&tree, &"nothing".into()).is_none());
    }
}
