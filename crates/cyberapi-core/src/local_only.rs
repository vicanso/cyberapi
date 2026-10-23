//! A server project's items one reader keeps to themselves
//! (`docs/server-sync.md` §3.4): folders and APIs made here and never
//! synced, marked to stay here. A sync leaves them out; the server's
//! content, pulled, gets them back where they were. What the server has
//! cannot be marked: leaving it out of a push would delete it for everyone.

use crate::ItemId;
use crate::model::{ApiSetting, Folder, HttpRequest, VariableCategory};
use crate::sync::ProjectConfig;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

/// What one reader keeps of a server project for themselves.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LocalItems {
    /// The items marked, each with what it holds.
    pub roots: Vec<ItemId>,
    /// The folders and APIs the server had when the copy was last level
    /// with it: those cannot be marked. `None` until known.
    pub server: Option<BTreeSet<ItemId>>,
    /// The [`fingerprint`] of what syncs of the copy then.
    pub level: Option<u64>,
    /// The marked items as they are, and where each sits: what a pull puts
    /// back.
    pub kept: LocalPart,
}

/// The marked items' content and places.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LocalPart {
    pub folders: Vec<Folder>,
    pub apis: Vec<ApiSetting>,
    /// Where each outermost marked item sits.
    pub places: Vec<Place>,
}

/// An item's place: in a folder (`None`: the top level), after the synced
/// item before it — what stays put when others add theirs — or first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Place {
    pub item: ItemId,
    pub parent: Option<ItemId>,
    pub after: Option<ItemId>,
    /// Its position then: the order the kept ones go back in, and where one
    /// goes whose `after` is gone.
    pub index: usize,
}

impl Place {
    fn in_list(item: &ItemId, parent: Option<ItemId>, list: &[ItemId], members: &HashSet<ItemId>) -> Self {
        let index = list.iter().position(|entry| entry == item).unwrap_or(list.len());
        Self {
            item: item.clone(),
            parent,
            after: list[..index]
                .iter()
                .rev()
                .find(|entry| !members.contains(*entry))
                .cloned(),
            index,
        }
    }

    /// Puts the item back in `list`: after its synced neighbour, behind the
    /// ones put back there already.
    fn put_back(&self, list: &mut Vec<ItemId>, back: &HashSet<ItemId>) {
        let mut at = match &self.after {
            None => 0,
            Some(after) => match list.iter().position(|entry| entry == after) {
                Some(position) => position + 1,
                None => self.index.min(list.len()),
            },
        };
        while list.get(at).is_some_and(|entry| back.contains(entry)) {
            at += 1;
        }
        list.insert(at, self.item.clone());
    }
}

/// `item` and everything inside it, among `folders`.
fn subtree(folders: &HashMap<&ItemId, &Folder>, item: &ItemId, into: &mut HashSet<ItemId>) {
    if !into.insert(item.clone()) {
        return;
    }
    if let Some(folder) = folders.get(item) {
        for child in &folder.children {
            subtree(folders, child, into);
        }
    }
}

fn by_id(folders: &[Folder]) -> HashMap<&ItemId, &Folder> {
    folders.iter().map(|folder| (&folder.id, folder)).collect()
}

/// The folder that holds `item`, if one does.
fn parent_of<'a>(folders: &'a [Folder], item: &ItemId) -> Option<&'a Folder> {
    folders.iter().find(|folder| folder.children.contains(item))
}

/// Every folder and API of `config`.
pub fn ids(config: &ProjectConfig) -> BTreeSet<ItemId> {
    config
        .folders
        .iter()
        .map(|folder| folder.id.clone())
        .chain(config.apis.iter().map(|api| api.id.clone()))
        .collect()
}

impl LocalItems {
    /// The marked items and everything in them, among `folders`. A marked
    /// item that is gone counts for nothing.
    pub fn members(&self, folders: &[Folder]) -> HashSet<ItemId> {
        let folders = by_id(folders);
        let mut members = HashSet::new();
        for root in &self.roots {
            subtree(&folders, root, &mut members);
        }
        members
    }

    /// Whether `item` may be marked: it is not already, and neither it nor
    /// anything in it is on the server — or what the server has is not
    /// known yet.
    pub fn can_mark(&self, folders: &[Folder], item: &ItemId) -> bool {
        let Some(server) = &self.server else {
            return false;
        };
        if self.members(folders).contains(item) {
            return false;
        }
        let mut inside = HashSet::new();
        subtree(&by_id(folders), item, &mut inside);
        inside.iter().all(|id| !server.contains(id))
    }

    /// Whether `item` is marked and not inside another marked folder: what
    /// "sync it" undoes.
    pub fn is_outer(&self, folders: &[Folder], item: &ItemId) -> bool {
        self.roots.contains(item)
            && !parent_of(folders, item).is_some_and(|parent| self.members(folders).contains(&parent.id))
    }

    pub fn mark(&mut self, item: &ItemId) {
        if !self.roots.contains(item) {
            self.roots.push(item.clone());
        }
    }

    pub fn unmark(&mut self, item: &ItemId) {
        self.roots.retain(|root| root != item);
    }

    /// Whether `item` may go into `parent`: the server's items do not go
    /// into a marked folder, where a sync would leave them out.
    pub fn may_move(&self, folders: &[Folder], item: &ItemId, parent: Option<&ItemId>) -> bool {
        let members = self.members(folders);
        if !parent.is_some_and(|parent| members.contains(parent)) {
            return true;
        }
        let Some(server) = &self.server else {
            return false;
        };
        let mut inside = HashSet::new();
        subtree(&by_id(folders), item, &mut inside);
        inside.iter().all(|id| !server.contains(id))
    }

    /// Before `item` goes into `parent` (`folders` as they are): one leaving
    /// a marked folder stays the reader's, marked on its own.
    pub fn moving(&mut self, folders: &[Folder], item: &ItemId, parent: Option<&ItemId>) {
        let members = self.members(folders);
        if members.contains(item) && !parent.is_some_and(|parent| members.contains(parent)) {
            self.mark(item);
        }
    }

    /// What syncs of `config`: the marked items left out, and out of every
    /// order that held them.
    pub fn syncable(&self, config: &ProjectConfig) -> ProjectConfig {
        let members = self.members(&config.folders);
        if members.is_empty() {
            return config.clone();
        }
        let mut config = config.clone();
        config.folders.retain(|folder| !members.contains(&folder.id));
        config.apis.retain(|api| !members.contains(&api.id));
        for folder in &mut config.folders {
            folder.children.retain(|child| !members.contains(child));
        }
        config.top_order.retain(|item| !members.contains(item));
        config
    }

    /// Keeps the marked items of `config` (the copy, as it is) and where
    /// they sit. A mark whose item is gone goes too.
    pub fn keep(&mut self, config: &ProjectConfig) {
        let present = ids(config);
        self.roots.retain(|root| present.contains(root));
        let members = self.members(&config.folders);
        let places = self
            .roots
            .iter()
            .filter(|root| self.is_outer(&config.folders, root))
            .map(|root| match parent_of(&config.folders, root) {
                Some(parent) => Place::in_list(root, Some(parent.id.clone()), &parent.children, &members),
                None => Place::in_list(root, None, &config.top_order, &members),
            })
            .collect();
        self.kept = LocalPart {
            folders: config
                .folders
                .iter()
                .filter(|folder| members.contains(&folder.id))
                .cloned()
                .collect(),
            apis: config
                .apis
                .iter()
                .filter(|api| members.contains(&api.id))
                .cloned()
                .collect(),
            places,
        };
    }

    /// `config` (the server's) with the kept items back where they sat: in
    /// their folder while it is there, else on the top level.
    pub fn restore(&self, mut config: ProjectConfig) -> ProjectConfig {
        let present = ids(&config);
        let kept = &self.kept;
        config.folders.extend(
            kept.folders
                .iter()
                .filter(|folder| !present.contains(&folder.id))
                .cloned(),
        );
        config
            .apis
            .extend(kept.apis.iter().filter(|api| !present.contains(&api.id)).cloned());
        let mut places: Vec<&Place> = kept
            .places
            .iter()
            .filter(|place| !present.contains(&place.item))
            .collect();
        // In the order they were, so ones after the same neighbour keep
        // theirs; the ones whose folder is gone last, at the end of the top
        // level, where they take no one's place.
        places.sort_by_key(|place| place.index);
        let folders: HashSet<ItemId> = config.folders.iter().map(|folder| folder.id.clone()).collect();
        let (homeless, placed): (Vec<_>, Vec<_>) = places
            .into_iter()
            .partition(|place| place.parent.as_ref().is_some_and(|parent| !folders.contains(parent)));
        let mut back = HashSet::new();
        for place in placed {
            let folder = place
                .parent
                .as_ref()
                .and_then(|parent| config.folders.iter_mut().find(|folder| &folder.id == parent));
            match folder {
                Some(folder) => place.put_back(&mut folder.children, &back),
                None => place.put_back(&mut config.top_order, &back),
            }
            back.insert(place.item.clone());
        }
        config
            .top_order
            .extend(homeless.into_iter().map(|place| place.item.clone()));
        config
    }

    /// The copy is level with the server, whose content is `syncable`.
    pub fn level_with(&mut self, syncable: &ProjectConfig) {
        self.server = Some(ids(syncable));
        self.level = Some(fingerprint(syncable));
    }

    /// Whether the copy (`config`) has what the server has not; `None`
    /// while the level is not known.
    pub fn unsynced(&self, config: &ProjectConfig) -> Option<bool> {
        Some(fingerprint(&self.syncable(config)) != self.level?)
    }
}

/// What a reader can change of a configuration: names, places, requests,
/// variables — not when each was last written, which moves with what is
/// around it.
#[derive(Serialize)]
struct Content<'a> {
    name: &'a str,
    description: &'a str,
    folders: Vec<(&'a ItemId, &'a str, &'a [ItemId])>,
    apis: Vec<(&'a ItemId, &'a str, &'a HttpRequest)>,
    top_order: &'a [ItemId],
    variables: Vec<(&'a str, VariableCategory, &'a str, &'a str, bool)>,
}

/// A stable digest of `config`'s content, in any order its parts were read.
pub fn fingerprint(config: &ProjectConfig) -> u64 {
    let mut folders: Vec<_> = config
        .folders
        .iter()
        .map(|folder| (&folder.id, folder.name.as_str(), folder.children.as_slice()))
        .collect();
    folders.sort_by(|a, b| a.0.cmp(b.0));
    let mut apis: Vec<_> = config
        .apis
        .iter()
        .map(|api| (&api.id, api.name.as_str(), &api.request))
        .collect();
    apis.sort_by(|a, b| a.0.cmp(b.0));
    let mut variables: Vec<_> = config
        .variables
        .iter()
        .map(|variable| {
            (
                variable.id.as_str(),
                variable.category,
                variable.name.as_str(),
                variable.value.as_str(),
                variable.enabled,
            )
        })
        .collect();
    variables.sort_by(|a, b| a.0.cmp(b.0));
    let content = Content {
        name: &config.name,
        description: &config.description,
        folders,
        apis,
        top_order: &config.top_order,
        variables,
    };
    let bytes = serde_json::to_vec(&content).unwrap_or_default();
    // FNV-1a: the same digest in every build and every version of Rust.
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Timestamp;
    use pretty_assertions::assert_eq;

    fn folder(id: &str, children: &[&str]) -> Folder {
        Folder {
            id: id.into(),
            name: id.into(),
            children: children.iter().map(|&child| child.into()).collect(),
            ..Default::default()
        }
    }

    fn api(id: &str) -> ApiSetting {
        ApiSetting {
            id: id.into(),
            name: id.into(),
            ..Default::default()
        }
    }

    /// The server's `users` (with `login`) and `ping`; this reader's `mine`
    /// folder (with `scratch`) in `users`, and `note` on the top level.
    fn copy() -> ProjectConfig {
        ProjectConfig {
            name: "shop".into(),
            folders: vec![folder("users", &["login", "mine"]), folder("mine", &["scratch"])],
            apis: vec![api("login"), api("scratch"), api("ping"), api("note")],
            top_order: vec!["users".into(), "note".into(), "ping".into()],
            ..Default::default()
        }
    }

    fn server() -> ProjectConfig {
        ProjectConfig {
            name: "shop".into(),
            folders: vec![folder("users", &["login"])],
            apis: vec![api("login"), api("ping")],
            top_order: vec!["users".into(), "ping".into()],
            ..Default::default()
        }
    }

    fn local() -> LocalItems {
        let mut local = LocalItems::default();
        local.level_with(&server());
        local
    }

    #[test]
    fn only_what_the_server_does_not_have_is_marked() {
        let config = copy();
        let mut local = local();
        assert!(local.can_mark(&config.folders, &"mine".into()));
        assert!(local.can_mark(&config.folders, &"note".into()));
        assert!(!local.can_mark(&config.folders, &"login".into()), "the server's");
        assert!(!local.can_mark(&config.folders, &"users".into()), "holds the server's");
        assert!(
            !LocalItems::default().can_mark(&config.folders, &"note".into()),
            "nothing while the server's are not known"
        );

        local.mark(&"mine".into());
        assert!(
            !local.can_mark(&config.folders, &"scratch".into()),
            "marked with its folder"
        );
        assert!(local.is_outer(&config.folders, &"mine".into()));
        let members = local.members(&config.folders);
        assert_eq!(members, HashSet::from(["mine".into(), "scratch".into()]));
    }

    #[test]
    fn a_sync_leaves_the_marked_out_and_a_pull_puts_them_back() {
        let mut local = local();
        local.mark(&"mine".into());
        local.mark(&"note".into());
        let config = copy();
        let syncable = local.syncable(&config);
        assert_eq!(syncable.folders, vec![folder("users", &["login"])]);
        assert_eq!(
            syncable.apis.iter().map(|api| api.id.as_str()).collect::<Vec<_>>(),
            ["login", "ping"]
        );
        assert_eq!(syncable.top_order, vec![ItemId::from("users"), "ping".into()]);

        local.keep(&config);
        // Someone else added `logout` before `login` and `health` up top.
        let mut moved_on = server();
        moved_on.folders[0].children.insert(0, "logout".into());
        moved_on.apis.extend([api("logout"), api("health")]);
        moved_on.top_order.insert(0, "health".into());
        let restored = local.restore(moved_on);
        assert_eq!(
            restored.folders[0].children,
            vec![ItemId::from("logout"), "login".into(), "mine".into()]
        );
        assert_eq!(
            restored.top_order,
            vec![ItemId::from("health"), "users".into(), "note".into(), "ping".into()]
        );
        assert!(restored.folders.contains(&folder("mine", &["scratch"])));
        assert!(restored.apis.iter().any(|api| api.id.as_str() == "scratch"));

        // The folder it sat in is gone: the top level has it.
        let mut gone = server();
        gone.folders.clear();
        gone.top_order = vec!["ping".into()];
        let restored = local.restore(gone);
        assert_eq!(
            restored.top_order,
            vec![ItemId::from("ping"), "note".into(), "mine".into()]
        );
        assert_eq!(
            local.restore(restored.clone()),
            restored,
            "putting back twice adds nothing"
        );
    }

    #[test]
    fn the_copy_is_unsynced_by_what_syncs_of_it() {
        let mut local = local();
        assert_eq!(LocalItems::default().unsynced(&server()), None, "not known");
        assert_eq!(local.unsynced(&server()), Some(false));
        assert_eq!(local.unsynced(&copy()), Some(true), "new items to sync");
        local.mark(&"mine".into());
        local.mark(&"note".into());
        assert_eq!(local.unsynced(&copy()), Some(false), "only the reader's own are new");

        let mut edited = copy();
        edited.apis[1].request.uri = "https://a.test".into();
        edited.folders[0].updated_at = Timestamp::from_unix(9);
        assert_eq!(
            local.unsynced(&edited),
            Some(false),
            "a change to the reader's own; times are not content"
        );
        edited.apis[0].request.uri = "https://a.test".into();
        assert_eq!(local.unsynced(&edited), Some(true), "a change to the server's");
    }

    #[test]
    fn the_servers_stay_out_of_a_marked_folder_and_the_readers_stay_theirs() {
        let config = copy();
        let mut local = local();
        local.mark(&"mine".into());
        let mine = ItemId::from("mine");
        assert!(!local.may_move(&config.folders, &"login".into(), Some(&mine)));
        assert!(
            local.may_move(&config.folders, &"note".into(), Some(&mine)),
            "a new one may"
        );
        assert!(local.may_move(&config.folders, &"scratch".into(), None));

        local.moving(&config.folders, &"scratch".into(), Some(&"users".into()));
        assert!(
            local.roots.contains(&"scratch".into()),
            "out of the folder, still the reader's"
        );
        local.moving(&config.folders, &"note".into(), None);
        assert!(!local.roots.contains(&"note".into()), "a synced one stays synced");
    }

    #[test]
    fn keeping_drops_marks_of_what_is_gone() {
        let mut local = local();
        local.mark(&"mine".into());
        local.mark(&"deleted".into());
        local.keep(&copy());
        assert_eq!(local.roots, vec![ItemId::from("mine")]);
        assert_eq!(
            local.kept.places,
            vec![Place {
                item: "mine".into(),
                parent: Some("users".into()),
                after: Some("login".into()),
                index: 1,
            }]
        );
        let round: LocalItems = serde_json::from_str(&serde_json::to_string(&local).expect("json")).expect("json");
        assert_eq!(round, local);
    }
}
