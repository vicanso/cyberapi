//! What a change to the tree rewrites (F3.4 / F3.6 / F3.7).
//!
//! Each `plan_*` reads the stored lists and answers with the lists that
//! change; a store writes exactly those, in one transaction, and a state can
//! apply the same plan to its copy. The rules live here once, for every
//! store.

use super::{Tree, TreeData};
use crate::ItemId;
use serde::{Deserialize, Serialize};
use snafu::{OptionExt, Snafu, ensure};
use std::collections::HashSet;

/// Where an item goes: into `parent` (the top level when `None`), before
/// `before`, or at the end when `before` is `None` or not found there.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub parent: Option<ItemId>,
    pub before: Option<ItemId>,
}

/// The stored lists a plan rewrites. Only what differs is listed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeChange {
    /// Folders whose `children` change, with the new list, in store order.
    pub folders: Vec<(ItemId, Vec<ItemId>)>,
    /// The new top-level order, when it changes.
    pub top_order: Option<Vec<ItemId>>,
}

impl TreeChange {
    pub fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.top_order.is_none()
    }
}

/// What removing an item deletes, and the lists that stop naming it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Removal {
    /// The removed item first if it is a folder, then every folder inside it.
    pub folders: Vec<ItemId>,
    /// The removed item if it is an API, then every API inside it.
    pub apis: Vec<ItemId>,
    pub change: TreeChange,
}

#[derive(Clone, Debug, PartialEq, Eq, Snafu)]
pub enum TreeError {
    #[snafu(display("{id} is not in this project's tree"))]
    UnknownItem { id: ItemId },
    #[snafu(display("there is no folder {id} in this project's tree"))]
    UnknownFolder { id: ItemId },
    #[snafu(display("{id} is an API, not a folder"))]
    NotAFolder { id: ItemId },
    #[snafu(display("folder {id} cannot go inside itself"))]
    Cycle { id: ItemId },
}

impl TreeData {
    /// Moves an existing item (F3.7). It leaves every folder that lists it —
    /// a damaged store may have more than one — and joins `to`.
    ///
    /// A folder cannot move into itself or anything inside it: the old app
    /// allowed it, and the whole subtree disappeared (#4). Moving into a
    /// folder also takes the item out of the top-level order, which the old
    /// app left behind.
    pub fn plan_move(&self, item: &ItemId, to: &Placement) -> Result<TreeChange, TreeError> {
        let tree = Tree::build(self);
        let node = tree.get(item).context(UnknownItemSnafu { id: item.clone() })?;
        if let Some(parent) = &to.parent {
            check_folder(&tree, parent)?;
            ensure!(
                !node.is_folder() || (parent != item && !tree.is_inside(parent, item)),
                CycleSnafu { id: item.clone() }
            );
        }

        let mut change = TreeChange::default();
        for folder in &self.folders {
            let mut children = folder.children.clone();
            children.retain(|child| child != item);
            if to.parent.as_ref() == Some(&folder.id) {
                insert_before(&mut children, item, to.before.as_ref());
            }
            if children != folder.children {
                change.folders.push((folder.id.clone(), children));
            }
        }

        let top_order = match &to.parent {
            // The order as shown, so items the stored list never named keep
            // the place they were drawn in.
            None => {
                let mut top = tree.top_level_ids();
                top.retain(|id| id != item);
                insert_before(&mut top, item, to.before.as_ref());
                top
            }
            Some(_) => {
                let mut top = self.top_order.clone();
                top.retain(|id| id != item);
                top
            }
        };
        if top_order != self.top_order {
            change.top_order = Some(top_order);
        }
        Ok(change)
    }

    /// Places a new item, not yet part of the tree (F3.4): at the end of
    /// `parent`, or first on the top level — where the old app showed a new
    /// item, since its order list never named one.
    pub fn plan_insert(&self, item: &ItemId, parent: Option<&ItemId>) -> Result<TreeChange, TreeError> {
        let tree = Tree::build(self);
        let mut change = TreeChange::default();
        match parent {
            Some(parent) => {
                check_folder(&tree, parent)?;
                for folder in self.folders.iter().filter(|folder| &folder.id == parent) {
                    let mut children = folder.children.clone();
                    children.retain(|child| child != item);
                    children.push(item.clone());
                    change.folders.push((folder.id.clone(), children));
                }
            }
            None => {
                let mut top = vec![item.clone()];
                top.extend(tree.top_level_ids().into_iter().filter(|id| id != item));
                change.top_order = Some(top);
            }
        }
        Ok(change)
    }

    /// Removes an item and, for a folder, everything inside it (F3.6). Every
    /// remaining list stops naming what was removed; the old app left those
    /// ids behind (#5).
    pub fn plan_remove(&self, item: &ItemId) -> Result<Removal, TreeError> {
        let tree = Tree::build(self);
        ensure!(tree.contains(item), UnknownItemSnafu { id: item.clone() });
        let mut removal = Removal::default();
        let mut gone: HashSet<ItemId> = HashSet::new();
        for id in std::iter::once(item.clone()).chain(tree.descendants(item)) {
            match tree.get(&id) {
                Some(found) if found.is_folder() => removal.folders.push(id.clone()),
                _ => removal.apis.push(id.clone()),
            }
            gone.insert(id);
        }

        for folder in self.folders.iter().filter(|folder| !gone.contains(&folder.id)) {
            let children: Vec<ItemId> = folder
                .children
                .iter()
                .filter(|child| !gone.contains(*child))
                .cloned()
                .collect();
            if children != folder.children {
                removal.change.folders.push((folder.id.clone(), children));
            }
        }
        if self.top_order.iter().any(|id| gone.contains(id)) {
            removal.change.top_order = Some(
                self.top_order
                    .iter()
                    .filter(|id| !gone.contains(*id))
                    .cloned()
                    .collect(),
            );
        }
        Ok(removal)
    }

    /// Applies a plan to this copy, as the store applied it to its own.
    pub fn apply(&mut self, change: &TreeChange) {
        for (id, children) in &change.folders {
            for folder in self.folders.iter_mut().filter(|folder| &folder.id == id) {
                folder.children = children.clone();
            }
        }
        if let Some(top_order) = &change.top_order {
            self.top_order = top_order.clone();
        }
    }

    pub fn apply_removal(&mut self, removal: &Removal) {
        self.folders.retain(|folder| !removal.folders.contains(&folder.id));
        self.apis.retain(|api| !removal.apis.contains(&api.id));
        self.apply(&removal.change);
    }
}

fn check_folder(tree: &Tree, id: &ItemId) -> Result<(), TreeError> {
    let node = tree.get(id).context(UnknownFolderSnafu { id: id.clone() })?;
    ensure!(node.is_folder(), NotAFolderSnafu { id: id.clone() });
    Ok(())
}

fn insert_before(list: &mut Vec<ItemId>, item: &ItemId, before: Option<&ItemId>) {
    match before.and_then(|before| list.iter().position(|id| id == before)) {
        Some(at) => list.insert(at, item.clone()),
        None => list.push(item.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixture::*;
    use super::*;
    use crate::model::HttpMethod;
    use pretty_assertions::assert_eq;

    fn to(parent: Option<&str>, before: Option<&str>) -> Placement {
        Placement {
            parent: parent.map(ItemId::from),
            before: before.map(ItemId::from),
        }
    }

    fn moved(data: &TreeData, item: &str, placement: Placement) -> TreeData {
        let change = data.plan_move(&item.into(), &placement).expect("a valid move");
        let mut data = data.clone();
        data.apply(&change);
        data
    }

    fn children(data: &TreeData, folder: &str) -> Vec<ItemId> {
        Tree::build(data)
            .get(&folder.into())
            .expect("folder")
            .children()
            .to_vec()
    }

    #[test]
    fn into_a_folder_at_the_end() {
        let data = sample();
        let change = data
            .plan_move(&"health".into(), &to(Some("orders"), None))
            .expect("move");
        assert_eq!(
            change,
            TreeChange {
                folders: vec![("orders".into(), ids(&["list", "health"]))],
                top_order: Some(ids(&["users", "orders"])),
            }
        );
    }

    #[test]
    fn before_a_sibling() {
        let data = moved(&sample(), "profile", to(Some("users"), Some("login")));
        assert_eq!(children(&data, "users"), ids(&["profile", "login"]));
        assert_eq!(data.top_order, sample().top_order);
    }

    #[test]
    fn out_to_the_top_level() {
        let data = moved(&sample(), "me", to(None, Some("orders")));
        assert!(children(&data, "profile").is_empty());
        assert_eq!(data.top_order, ids(&["users", "me", "orders", "health"]));

        let data = moved(&sample(), "me", to(None, None));
        assert_eq!(data.top_order, ids(&["users", "orders", "health", "me"]));
    }

    #[test]
    fn the_top_level_order_is_the_one_shown() {
        // The stored list names only `health`; the others were drawn first.
        let mut data = sample();
        data.top_order = ids(&["health"]);
        let data = moved(&data, "health", to(None, Some("orders")));
        assert_eq!(data.top_order, ids(&["users", "health", "orders"]));
    }

    #[test]
    fn a_folder_cannot_go_inside_itself() {
        let data = sample();
        assert_eq!(
            data.plan_move(&"users".into(), &to(Some("profile"), None)),
            Err(TreeError::Cycle { id: "users".into() })
        );
        assert_eq!(
            data.plan_move(&"users".into(), &to(Some("users"), None)),
            Err(TreeError::Cycle { id: "users".into() })
        );
        // An API into a folder it is already in is fine.
        assert!(data.plan_move(&"login".into(), &to(Some("users"), None)).is_ok());
    }

    #[test]
    fn bad_targets_are_refused() {
        let data = sample();
        assert_eq!(
            data.plan_move(&"ghost".into(), &to(None, None)),
            Err(TreeError::UnknownItem { id: "ghost".into() })
        );
        assert_eq!(
            data.plan_move(&"me".into(), &to(Some("ghost"), None)),
            Err(TreeError::UnknownFolder { id: "ghost".into() })
        );
        assert_eq!(
            data.plan_move(&"me".into(), &to(Some("health"), None)),
            Err(TreeError::NotAFolder { id: "health".into() })
        );
    }

    #[test]
    fn a_move_leaves_every_folder_that_listed_the_item() {
        let mut data = sample();
        data.folders[2].children = ids(&["list", "login"]);
        let data = moved(&data, "login", to(Some("profile"), None));
        assert_eq!(data.folders[0].children, ids(&["profile"]));
        assert_eq!(data.folders[1].children, ids(&["me", "login"]));
        assert_eq!(data.folders[2].children, ids(&["list"]));
    }

    #[test]
    fn a_new_item_goes_first_on_top_or_last_in_its_folder() {
        let data = sample();
        let change = data.plan_insert(&"new".into(), None).expect("insert");
        assert_eq!(change.top_order, Some(ids(&["new", "users", "orders", "health"])));
        assert!(change.folders.is_empty());

        let change = data.plan_insert(&"new".into(), Some(&"users".into())).expect("insert");
        assert_eq!(
            change.folders,
            vec![("users".into(), ids(&["login", "profile", "new"]))]
        );
        assert_eq!(change.top_order, None);

        assert_eq!(
            data.plan_insert(&"new".into(), Some(&"login".into())),
            Err(TreeError::NotAFolder { id: "login".into() })
        );
    }

    #[test]
    fn removing_a_folder_takes_everything_inside() {
        let mut data = sample();
        // A stale reference elsewhere goes too (#5).
        data.folders[2].children = ids(&["list", "me"]);
        let removal = data.plan_remove(&"users".into()).expect("remove");
        assert_eq!(removal.folders, ids(&["users", "profile"]));
        assert_eq!(removal.apis, ids(&["login", "me"]));
        assert_eq!(removal.change.folders, vec![("orders".into(), ids(&["list"]))]);
        assert_eq!(removal.change.top_order, Some(ids(&["orders", "health"])));

        data.apply_removal(&removal);
        let tree = Tree::build(&data);
        assert_eq!(tree.top_level_ids(), ids(&["orders", "health"]));
        assert!(!tree.contains(&"me".into()));
    }

    #[test]
    fn removing_an_api_leaves_its_folder() {
        let data = sample();
        let removal = data.plan_remove(&"login".into()).expect("remove");
        assert!(removal.folders.is_empty());
        assert_eq!(removal.apis, ids(&["login"]));
        assert_eq!(removal.change.folders, vec![("users".into(), ids(&["profile"]))]);
        assert_eq!(removal.change.top_order, None);
    }

    #[test]
    fn a_plan_applied_rebuilds_the_same_tree_the_store_would() {
        let mut data = sample();
        data.apis.push(api("new", "new", HttpMethod::Get, "/new"));
        let change = data
            .plan_insert(&"new".into(), Some(&"profile".into()))
            .expect("insert");
        data.apply(&change);
        assert_eq!(children(&data, "profile"), ids(&["me", "new"]));
    }
}
