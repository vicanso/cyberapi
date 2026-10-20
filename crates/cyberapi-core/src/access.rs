//! Who may do what with a project (`docs/architecture.md` §9). Only the
//! browser build has more than one reader; on the desktop every project is
//! [`CollectionRole::Manager`].

use serde::{Deserialize, Serialize};

/// A reader's role on one project, weakest first, so roles compare with `<`
/// and the strongest of several grants is their `max`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CollectionRole {
    /// Sees everything and sends requests; the history is the reader's own.
    Viewer,
    /// Also changes the tree, the requests and the variables.
    Editor,
    /// Also renames or deletes the project and decides who has access.
    #[default]
    Manager,
}

impl CollectionRole {
    pub fn can_edit(&self) -> bool {
        *self >= Self::Editor
    }

    pub fn can_manage(&self) -> bool {
        *self >= Self::Manager
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_are_ordered_by_strength() {
        assert!(CollectionRole::Viewer < CollectionRole::Editor);
        assert_eq!(
            [CollectionRole::Editor, CollectionRole::Viewer].into_iter().max(),
            Some(CollectionRole::Editor)
        );
        assert!(!CollectionRole::Viewer.can_edit());
        assert!(CollectionRole::Editor.can_edit() && !CollectionRole::Editor.can_manage());
        assert!(CollectionRole::Manager.can_manage());
    }
}
