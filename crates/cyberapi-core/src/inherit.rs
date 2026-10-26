//! What an API takes from the folders it is in and from its project (#66,
//! `docs/batch4-design.md` §4). An API that inherits its auth signs in as
//! the nearest folder above it that does not inherit, or else as the
//! project. The folders' headers all go, the outermost folder's first, so
//! an inner folder's replaces an outer one's of the same name; the global
//! headers go before them and the API's own after (#3).

use crate::ItemId;
use crate::auth::Auth;
use crate::model::{Folder, KvParam};
use crate::tree::TreeData;

/// Where the auth an API inherits comes from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AuthSource {
    /// The project's, which no folder above stood in for; inheriting there
    /// too is none.
    #[default]
    Project,
    Folder {
        id: ItemId,
        name: String,
    },
}

/// What an API, or a folder, takes from above it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inherited {
    /// The folders' headers, the outermost folder's first.
    pub headers: Vec<KvParam>,
    /// What an `Auth::Inherit` of its own signs in with.
    pub auth: Auth,
    /// Where `auth` comes from.
    pub from: AuthSource,
}

impl Inherited {
    /// What `own` signs in with: itself, or what it inherits.
    pub fn auth<'a>(&'a self, own: &'a Auth) -> &'a Auth {
        if own.is_inherit() { &self.auth } else { own }
    }

    /// The folders' headers that go: enabled, and named.
    pub fn sent_headers(&self) -> impl Iterator<Item = &KvParam> {
        self.headers.iter().filter(|kv| kv.enabled && !kv.key.trim().is_empty())
    }
}

/// The folders `item` — an API or a folder — is in, the nearest first.
pub fn folders_above<'a>(tree: &'a TreeData, item: &ItemId) -> Vec<&'a Folder> {
    let mut above: Vec<&Folder> = Vec::new();
    let mut current = item;
    while let Some(parent) = tree.folders.iter().find(|folder| folder.children.contains(current)) {
        // A folder that lists one above it: the store's rules prevent it,
        // but a damaged tree must not hang a send.
        if above.iter().any(|seen| seen.id == parent.id) {
            break;
        }
        above.push(parent);
        current = &parent.id;
    }
    above
}

/// What `item` takes from the folders above it and the project.
pub fn inherited(tree: &TreeData, item: &ItemId) -> Inherited {
    let above = folders_above(tree, item);
    let headers = above
        .iter()
        .rev()
        .flat_map(|folder| folder.settings.headers.iter().cloned())
        .collect();
    let (auth, from) = above
        .iter()
        .find(|folder| !folder.settings.auth.is_inherit())
        .map(|folder| {
            (
                folder.settings.auth.clone(),
                AuthSource::Folder {
                    id: folder.id.clone(),
                    name: folder.name.clone(),
                },
            )
        })
        .unwrap_or_else(|| (tree.settings.auth.clone(), AuthSource::Project));
    Inherited { headers, auth, from }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FolderSettings, ProjectSettings};
    use crate::tree::fixture::{folder, sample};
    use pretty_assertions::assert_eq;

    fn kv(key: &str, value: &str, enabled: bool) -> KvParam {
        KvParam {
            key: key.into(),
            value: value.into(),
            enabled,
        }
    }

    fn bearer(token: &str) -> Auth {
        Auth::Bearer { token: token.into() }
    }

    /// `sample()`: users ⊃ login, profile ⊃ me; orders ⊃ list; health on
    /// top.
    fn tree() -> TreeData {
        let mut tree = sample();
        let set = |tree: &mut TreeData, id: &str, settings: FolderSettings| {
            tree.folders
                .iter_mut()
                .find(|folder| folder.id.as_str() == id)
                .expect("a folder")
                .settings = settings;
        };
        set(
            &mut tree,
            "users",
            FolderSettings {
                auth: bearer("users"),
                headers: vec![kv("X-Team", "users", true), kv("X-Off", "1", false)],
                description: String::new(),
            },
        );
        set(
            &mut tree,
            "profile",
            FolderSettings {
                auth: Auth::Inherit,
                headers: vec![kv("X-Team", "profile", true)],
                description: String::new(),
            },
        );
        set(
            &mut tree,
            "orders",
            FolderSettings {
                auth: Auth::None,
                headers: vec![],
                description: String::new(),
            },
        );
        tree.settings = ProjectSettings {
            auth: bearer("project"),
        };
        tree
    }

    #[test]
    fn the_nearest_folder_that_says_signs_in() {
        let tree = tree();
        let me = inherited(&tree, &"me".into());
        assert_eq!(me.auth, bearer("users"), "profile inherits, users says");
        assert_eq!(
            me.from,
            AuthSource::Folder {
                id: "users".into(),
                name: "用户".into()
            }
        );
        // None stops it: the project's is not reached.
        assert_eq!(inherited(&tree, &"list".into()).auth, Auth::None);
        let health = inherited(&tree, &"health".into());
        assert_eq!((health.auth, health.from), (bearer("project"), AuthSource::Project));
        // An API's own comes first; inheriting is what is above.
        let own = bearer("own");
        assert_eq!(me.auth(&own), &own);
        assert_eq!(me.auth(&Auth::Inherit), &bearer("users"));
        // Nothing anywhere: the project's inherit, which is none.
        let plain = inherited(&sample(), &"me".into());
        assert_eq!((plain.auth, plain.from), (Auth::Inherit, AuthSource::Project));
    }

    #[test]
    fn the_folders_headers_go_outermost_first() {
        let tree = tree();
        let me = inherited(&tree, &"me".into());
        assert_eq!(
            me.headers,
            [
                kv("X-Team", "users", true),
                kv("X-Off", "1", false),
                kv("X-Team", "profile", true)
            ]
        );
        assert_eq!(
            me.sent_headers().map(|kv| kv.value.as_str()).collect::<Vec<_>>(),
            ["users", "profile"]
        );
        // A folder takes what is above it, not its own.
        let profile = inherited(&tree, &"profile".into());
        assert_eq!(profile.headers.len(), 2);
        assert!(inherited(&tree, &"users".into()).headers.is_empty());
    }

    #[test]
    fn a_folder_that_lists_itself_does_not_hang() {
        let mut tree = sample();
        tree.folders.push(folder("loop", "loop", &["loop", "x"]));
        assert_eq!(folders_above(&tree, &"x".into()).len(), 1);
    }
}
