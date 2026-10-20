//! A project's tree of folders and APIs (M3): how it is built from the
//! stored records, which rows show under a keyword, and — in [`edit`] — what
//! a move, an insert or a removal rewrites.
//!
//! The stored form is the old one (4.2): each folder lists its children, and
//! the top level's order is a separate list. Nothing records an item's
//! parent, so the tree is worked out here, every time, from those lists.

mod drop;
mod edit;

pub use drop::DropPosition;
pub use edit::{Placement, Removal, TreeChange, TreeError};

use crate::ItemId;
use crate::model::{ApiSetting, Folder, HttpMethod, split_env_prefix};
use crate::search::matches_text_or_pinyin;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Everything a project's tree is made of, as the store keeps it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeData {
    pub folders: Vec<Folder>,
    pub apis: Vec<ApiSetting>,
    /// The order of the top level. An item missing from it comes first.
    pub top_order: Vec<ItemId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Folder,
    Api,
}

/// One item of a built [`Tree`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    id: ItemId,
    kind: ItemKind,
    name: String,
    method: HttpMethod,
    path: String,
    parent: Option<ItemId>,
    children: Vec<ItemId>,
}

impl Node {
    pub fn id(&self) -> &ItemId {
        &self.id
    }

    pub fn kind(&self) -> ItemKind {
        self.kind
    }

    pub fn is_folder(&self) -> bool {
        self.kind == ItemKind::Folder
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// An API's method; GET for a folder.
    pub fn method(&self) -> HttpMethod {
        self.method
    }

    /// An API's URI without its environment variable prefix (F3.1); empty
    /// for a folder.
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn parent(&self) -> Option<&ItemId> {
        self.parent.as_ref()
    }

    /// The children that exist, in order.
    pub fn children(&self) -> &[ItemId] {
        &self.children
    }
}

/// One line of the tree as it is drawn: what is visible under the current
/// keyword and expanded folders, depth first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: ItemId,
    pub kind: ItemKind,
    pub depth: usize,
    pub parent: Option<ItemId>,
    /// The last of the rows its parent shows. Always `false` at the top level.
    pub is_last_child: bool,
    /// A folder whose children are drawn below it: expanded, or any folder
    /// while a keyword is active (F3.2).
    pub open: bool,
}

/// A project's tree, built from its [`TreeData`].
#[derive(Clone, Debug, Default)]
pub struct Tree {
    nodes: Vec<Node>,
    index: HashMap<ItemId, usize>,
    roots: Vec<usize>,
}

impl Tree {
    /// Builds the tree (F3.1), repairing what the stored lists get wrong
    /// rather than losing items over it:
    ///
    /// - A child id that names nothing, or the folder itself, is skipped.
    /// - An item listed by two folders belongs to the first one in store
    ///   order. The old app moved an item by removing it from every folder
    ///   first, so this only happens to damaged data.
    /// - A folder moved into its own subtree (#4) left a cycle that nothing
    ///   reached, and the whole cycle vanished from the old app's tree. Here
    ///   the first unreachable item, in store order, is lifted to the top
    ///   level, which breaks the cycle and shows everything under it.
    ///
    /// The top level is every item no folder claims, ordered by `top_order`;
    /// those it does not list come first, APIs before folders, in store
    /// order — the old app's order.
    pub fn build(data: &TreeData) -> Self {
        let mut tree = Tree::default();
        for api in &data.apis {
            let (_, path) = split_env_prefix(&api.request.uri);
            tree.push(Node {
                id: api.id.clone(),
                kind: ItemKind::Api,
                name: api.name.clone(),
                method: api.request.method,
                path: path.to_string(),
                parent: None,
                children: Vec::new(),
            });
        }
        for folder in &data.folders {
            tree.push(Node {
                id: folder.id.clone(),
                kind: ItemKind::Folder,
                name: folder.name.clone(),
                method: HttpMethod::Get,
                path: String::new(),
                parent: None,
                children: Vec::new(),
            });
        }

        for folder in &data.folders {
            let Some(&folder_at) = tree.index.get(&folder.id) else {
                continue;
            };
            // A folder id that is also an API's resolves to the API, which
            // has no children to give.
            if tree.nodes[folder_at].kind != ItemKind::Folder {
                continue;
            }
            for child in &folder.children {
                if child == &folder.id {
                    continue;
                }
                let Some(&child_at) = tree.index.get(child) else {
                    continue;
                };
                if tree.nodes[child_at].parent.is_some() {
                    continue;
                }
                tree.nodes[child_at].parent = Some(folder.id.clone());
                tree.nodes[folder_at].children.push(child.clone());
            }
        }

        tree.roots = (0..tree.nodes.len())
            .filter(|&at| tree.nodes[at].parent.is_none())
            .collect();
        tree.lift_unreachable();

        let order: HashMap<&ItemId, usize> = data.top_order.iter().enumerate().map(|(at, id)| (id, at)).collect();
        // Unlisted is -1 in the old app: before every listed item. A stable
        // sort keeps their store order.
        tree.roots
            .sort_by_key(|&at| order.get(&tree.nodes[at].id).map_or(-1, |&position| position as i64));
        tree
    }

    fn push(&mut self, node: Node) {
        if self.index.contains_key(&node.id) {
            return;
        }
        self.index.insert(node.id.clone(), self.nodes.len());
        self.nodes.push(node);
    }

    /// Breaks the cycles nothing reaches. An unreachable item always has a
    /// parent (the parentless are roots), so walking up from one ends in a
    /// cycle; the cycle's first member in store order is detached and made a
    /// root, which reaches the rest of the cycle and whatever hangs below it.
    fn lift_unreachable(&mut self) {
        let mut reached = vec![false; self.nodes.len()];
        for root in self.roots.clone() {
            self.mark_reached(root, &mut reached);
        }
        for at in 0..self.nodes.len() {
            if reached[at] {
                continue;
            }
            let mut path: Vec<usize> = Vec::new();
            let mut current = at;
            let cycle = loop {
                if let Some(start) = path.iter().position(|&seen| seen == current) {
                    break &path[start..];
                }
                path.push(current);
                match self.nodes[current]
                    .parent
                    .as_ref()
                    .and_then(|parent| self.index.get(parent))
                {
                    Some(&parent_at) => current = parent_at,
                    // Unreachable: a parentless item is a root.
                    None => break &path[path.len() - 1..],
                }
            };
            let lifted = cycle.iter().copied().min().unwrap_or(at);
            if let Some(parent) = self.nodes[lifted].parent.take()
                && let Some(&parent_at) = self.index.get(&parent)
            {
                let id = self.nodes[lifted].id.clone();
                self.nodes[parent_at].children.retain(|child| child != &id);
            }
            self.roots.push(lifted);
            self.mark_reached(lifted, &mut reached);
        }
    }

    fn mark_reached(&self, at: usize, reached: &mut [bool]) {
        let mut stack = vec![at];
        while let Some(at) = stack.pop() {
            if reached[at] {
                continue;
            }
            reached[at] = true;
            stack.extend(self.nodes[at].children.iter().filter_map(|child| self.index.get(child)));
        }
    }

    pub fn get(&self, id: &ItemId) -> Option<&Node> {
        self.index.get(id).map(|&at| &self.nodes[at])
    }

    pub fn contains(&self, id: &ItemId) -> bool {
        self.index.contains_key(id)
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The top level, in display order.
    pub fn roots(&self) -> impl Iterator<Item = &Node> {
        self.roots.iter().map(|&at| &self.nodes[at])
    }

    /// The top level's ids, in display order.
    pub fn top_level_ids(&self) -> Vec<ItemId> {
        self.roots().map(|node| node.id.clone()).collect()
    }

    /// Whether `id` is somewhere inside `ancestor` (not `ancestor` itself).
    pub fn is_inside(&self, id: &ItemId, ancestor: &ItemId) -> bool {
        let mut current = self.get(id).and_then(Node::parent);
        while let Some(parent) = current {
            if parent == ancestor {
                return true;
            }
            current = self.get(parent).and_then(Node::parent);
        }
        false
    }

    /// Everything inside `id`, depth first, not including `id`.
    pub fn descendants(&self, id: &ItemId) -> Vec<ItemId> {
        let mut found = Vec::new();
        let mut stack: Vec<&ItemId> = self
            .get(id)
            .map(|node| node.children.iter().rev().collect())
            .unwrap_or_default();
        while let Some(child) = stack.pop() {
            found.push(child.clone());
            if let Some(node) = self.get(child) {
                stack.extend(node.children.iter().rev());
            }
        }
        found
    }

    /// The item after `id` among its siblings: in its folder, or on the top
    /// level.
    pub fn next_sibling(&self, id: &ItemId) -> Option<&ItemId> {
        let node = self.get(id)?;
        let siblings: Vec<&ItemId> = match &node.parent {
            Some(parent) => self.get(parent)?.children.iter().collect(),
            None => self.roots().map(Node::id).collect(),
        };
        let at = siblings.iter().position(|sibling| *sibling == id)?;
        siblings.get(at + 1).copied()
    }

    /// The rows to draw (F3.1 / F3.2).
    ///
    /// Without a keyword, a folder's children show when it is in `expanded`.
    /// With one, every folder shows its children, and an item shows when it
    /// matches, sits inside a match, or contains a match. An item matches when
    /// its method is the keyword (`post`), its path contains it, or its name
    /// does by text or pinyin.
    pub fn rows(&self, expanded: &HashSet<ItemId>, keyword: &str) -> Vec<Row> {
        let keyword = keyword.trim().to_lowercase();
        let visible = if keyword.is_empty() {
            None
        } else {
            let mut visible = vec![false; self.nodes.len()];
            for &root in &self.roots {
                self.mark_visible(root, &keyword, &mut visible);
            }
            Some(visible)
        };
        let mut rows = Vec::new();
        let roots: Vec<usize> = self.visible_of(&self.roots, visible.as_deref());
        for at in roots {
            self.push_rows(at, 0, false, expanded, visible.as_deref(), &mut rows);
        }
        rows
    }

    fn visible_of(&self, ats: &[usize], visible: Option<&[bool]>) -> Vec<usize> {
        ats.iter()
            .copied()
            .filter(|&at| visible.is_none_or(|visible| visible[at]))
            .collect()
    }

    fn push_rows(
        &self,
        at: usize,
        depth: usize,
        is_last_child: bool,
        expanded: &HashSet<ItemId>,
        visible: Option<&[bool]>,
        rows: &mut Vec<Row>,
    ) {
        let node = &self.nodes[at];
        let open = node.is_folder() && (visible.is_some() || expanded.contains(&node.id));
        rows.push(Row {
            id: node.id.clone(),
            kind: node.kind,
            depth,
            parent: node.parent.clone(),
            is_last_child,
            open,
        });
        if !open {
            return;
        }
        let children: Vec<usize> = node
            .children
            .iter()
            .filter_map(|child| self.index.get(child).copied())
            .collect();
        let children = self.visible_of(&children, visible);
        let count = children.len();
        for (position, child) in children.into_iter().enumerate() {
            self.push_rows(child, depth + 1, position + 1 == count, expanded, visible, rows);
        }
    }

    /// Marks what shows under `keyword` in the subtree at `at`; answers
    /// whether `at` itself shows.
    fn mark_visible(&self, at: usize, keyword: &str, visible: &mut [bool]) -> bool {
        let node = &self.nodes[at];
        if self.matches(node, keyword) {
            // A match shows everything inside it.
            let mut stack = vec![at];
            while let Some(at) = stack.pop() {
                visible[at] = true;
                stack.extend(self.nodes[at].children.iter().filter_map(|child| self.index.get(child)));
            }
            return true;
        }
        let mut any = false;
        for child in &node.children {
            if let Some(&child_at) = self.index.get(child) {
                any |= self.mark_visible(child_at, keyword, visible);
            }
        }
        visible[at] = any;
        any
    }

    fn matches(&self, node: &Node, keyword: &str) -> bool {
        (node.kind == ItemKind::Api && node.method.as_str().eq_ignore_ascii_case(keyword))
            || node.path.to_lowercase().contains(keyword)
            || matches_text_or_pinyin(&node.name, keyword)
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    //! A small tree for the tests of this module and its children.

    use super::TreeData;
    use crate::model::{ApiSetting, Folder, HttpMethod, HttpRequest};
    use crate::{CollectionId, ItemId};

    pub fn ids(list: &[&str]) -> Vec<ItemId> {
        list.iter().map(|&id| ItemId::from(id)).collect()
    }

    pub fn folder(id: &str, name: &str, children: &[&str]) -> Folder {
        Folder {
            id: id.into(),
            collection: CollectionId::from("c"),
            name: name.into(),
            children: ids(children),
            ..Default::default()
        }
    }

    pub fn api(id: &str, name: &str, method: HttpMethod, uri: &str) -> ApiSetting {
        ApiSetting {
            id: id.into(),
            collection: CollectionId::from("c"),
            name: name.into(),
            request: HttpRequest {
                method,
                uri: uri.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// ```text
    /// users/            (folder "用户")
    ///   login           POST {{HOST}}/login
    ///   profile/        (folder)
    ///     me            GET  {{HOST}}/users/me
    /// orders/           (folder)
    ///   list            GET  {{HOST}}/orders
    /// health            GET  /health
    /// ```
    /// `top_order` lists `users`, `orders`, `health`.
    pub fn sample() -> TreeData {
        TreeData {
            folders: vec![
                folder("users", "用户", &["login", "profile"]),
                folder("profile", "profile", &["me"]),
                folder("orders", "orders", &["list"]),
            ],
            apis: vec![
                api("login", "登录", HttpMethod::Post, "{{HOST}}/login"),
                api("me", "me", HttpMethod::Get, "{{HOST}}/users/me"),
                api("list", "list", HttpMethod::Get, "{{HOST}}/orders"),
                api("health", "health", HttpMethod::Get, "/health"),
            ],
            top_order: ids(&["users", "orders", "health"]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use pretty_assertions::assert_eq;

    fn row_ids(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth), row.id))
            .collect()
    }

    fn expanded(list: &[&str]) -> HashSet<ItemId> {
        ids(list).into_iter().collect()
    }

    #[test]
    fn builds_the_hierarchy_in_top_order() {
        let tree = Tree::build(&sample());
        assert_eq!(tree.top_level_ids(), ids(&["users", "orders", "health"]));
        let users = tree.get(&"users".into()).expect("users");
        assert_eq!(users.children(), ids(&["login", "profile"]).as_slice());
        assert_eq!(tree.get(&"me".into()).and_then(Node::parent), Some(&"profile".into()));
        assert_eq!(tree.get(&"login".into()).map(Node::path), Some("/login"));
    }

    #[test]
    fn unlisted_items_come_first_apis_before_folders() {
        let mut data = sample();
        data.top_order = ids(&["health"]);
        let tree = Tree::build(&data);
        // `orders` and `users` are unlisted: store order, folders after APIs
        // (there are no unlisted top-level APIs here), then the listed one.
        assert_eq!(tree.top_level_ids(), ids(&["users", "orders", "health"]));

        data.apis.push(api("new", "new", HttpMethod::Get, "/new"));
        let tree = Tree::build(&data);
        assert_eq!(tree.top_level_ids(), ids(&["new", "users", "orders", "health"]));
    }

    #[test]
    fn dangling_and_self_references_are_skipped() {
        let mut data = sample();
        data.folders[2].children = ids(&["ghost", "orders", "list"]);
        let tree = Tree::build(&data);
        assert_eq!(
            tree.get(&"orders".into()).map(Node::children),
            Some(ids(&["list"]).as_slice())
        );
    }

    #[test]
    fn an_item_listed_twice_belongs_to_the_first_folder() {
        let mut data = sample();
        data.folders[2].children = ids(&["list", "login"]);
        let tree = Tree::build(&data);
        assert_eq!(tree.get(&"login".into()).and_then(Node::parent), Some(&"users".into()));
        assert_eq!(
            tree.get(&"orders".into()).map(Node::children),
            Some(ids(&["list"]).as_slice())
        );
    }

    #[test]
    fn a_cycle_is_lifted_to_the_top_level() {
        // `users` was dragged into `profile`, which is inside it (#4).
        let mut data = sample();
        data.folders[1].children = ids(&["me", "users"]);
        let tree = Tree::build(&data);
        // `users` comes first in store order among the unreachable, so it is
        // the one lifted; `profile` stays inside it.
        assert!(tree.get(&"users".into()).and_then(Node::parent).is_none());
        assert_eq!(
            tree.get(&"profile".into()).and_then(Node::parent),
            Some(&"users".into())
        );
        assert_eq!(
            tree.get(&"profile".into()).map(Node::children),
            Some(ids(&["me"]).as_slice())
        );
        assert_eq!(tree.top_level_ids(), ids(&["users", "orders", "health"]));
        let all = tree.rows(&expanded(&["users", "profile", "orders"]), "");
        assert_eq!(all.len(), 7);
    }

    #[test]
    fn rows_follow_expanded_folders() {
        let tree = Tree::build(&sample());
        assert_eq!(row_ids(&tree.rows(&HashSet::new(), "")), ["users", "orders", "health"]);
        assert_eq!(
            row_ids(&tree.rows(&expanded(&["users", "profile"]), "")),
            ["users", "  login", "  profile", "    me", "orders", "health"]
        );
        // A collapsed parent hides an expanded child folder.
        assert_eq!(
            row_ids(&tree.rows(&expanded(&["profile"]), "")),
            ["users", "orders", "health"]
        );
    }

    #[test]
    fn last_child_is_among_what_shows() {
        let tree = Tree::build(&sample());
        let rows = tree.rows(&expanded(&["users", "profile"]), "");
        let last: Vec<&str> = rows
            .iter()
            .filter(|row| row.is_last_child)
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(last, ["profile", "me"]);

        // The old app counted a dangling last id, and then no child was last.
        let mut data = sample();
        data.folders[0].children = ids(&["login", "profile", "ghost"]);
        let rows = Tree::build(&data).rows(&expanded(&["users"]), "");
        assert!(rows.iter().any(|row| row.id.as_str() == "profile" && row.is_last_child));
    }

    #[test]
    fn keyword_matches_method_path_and_name() {
        let tree = Tree::build(&sample());
        let none = HashSet::new();
        // Method: exact, any case. Folders open regardless of `expanded`.
        assert_eq!(row_ids(&tree.rows(&none, "post")), ["users", "  login"]);
        // Path, without the environment prefix.
        assert_eq!(row_ids(&tree.rows(&none, "/orders")), ["orders", "  list"]);
        assert!(tree.rows(&none, "HOST").is_empty());
        // Name by pinyin: a matching folder shows everything inside it.
        assert_eq!(
            row_ids(&tree.rows(&none, "yh")),
            ["users", "  login", "  profile", "    me"]
        );
        assert_eq!(row_ids(&tree.rows(&none, "denglu")), ["users", "  login"]);
        assert!(tree.rows(&none, "nothing").is_empty());
    }

    #[test]
    fn subtree_queries() {
        let tree = Tree::build(&sample());
        assert!(tree.is_inside(&"me".into(), &"users".into()));
        assert!(!tree.is_inside(&"users".into(), &"users".into()));
        assert!(!tree.is_inside(&"list".into(), &"users".into()));
        assert_eq!(tree.descendants(&"users".into()), ids(&["login", "profile", "me"]));
        assert_eq!(tree.next_sibling(&"login".into()), Some(&"profile".into()));
        assert_eq!(tree.next_sibling(&"profile".into()), None);
        assert_eq!(tree.next_sibling(&"orders".into()), Some(&"health".into()));
    }
}
