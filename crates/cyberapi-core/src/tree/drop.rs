//! Where a dragged item lands (F3.7). The view turns the pointer into a
//! target row and a [`DropPosition`]; this turns those into a [`Placement`].

use super::{Placement, Tree};
use crate::ItemId;

/// Where on the target row the item was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DropPosition {
    /// Before the target, beside it.
    Above,
    /// Into the target folder, at the end. On an API it means `Above`.
    Over,
    /// After the target, beside it; into an open folder, first.
    Below,
}

impl Tree {
    /// The placement for dropping `moving` at `position` on `target`, whose
    /// children are drawn below it when `target_open`. `None` when the drop
    /// would do nothing or is not allowed: onto itself, or a folder into its
    /// own subtree (#4).
    ///
    /// The old app worked from row indices and had both edges of a row mean
    /// "before it", with a special case that sent a drop above a folder into
    /// the folder ending just above. With two edges that mean what they show
    /// — above is before, below is after — that case is simply `Below` on the
    /// last child.
    pub fn drop_placement(
        &self,
        moving: &ItemId,
        target: &ItemId,
        position: DropPosition,
        target_open: bool,
    ) -> Option<Placement> {
        if moving == target {
            return None;
        }
        let moving_node = self.get(moving)?;
        let target_node = self.get(target)?;
        let placement = match position {
            DropPosition::Over if target_node.is_folder() => Placement {
                parent: Some(target.clone()),
                before: None,
            },
            DropPosition::Above | DropPosition::Over => Placement {
                parent: target_node.parent().cloned(),
                before: Some(target.clone()),
            },
            DropPosition::Below if target_node.is_folder() && target_open => Placement {
                parent: Some(target.clone()),
                before: target_node.children().iter().find(|child| *child != moving).cloned(),
            },
            DropPosition::Below => Placement {
                parent: target_node.parent().cloned(),
                before: self.next_sibling_skipping(target, moving).cloned(),
            },
        };
        if moving_node.is_folder()
            && let Some(parent) = &placement.parent
            && (parent == moving || self.is_inside(parent, moving))
        {
            return None;
        }
        Some(placement)
    }

    /// The sibling after `id`, passing over `skip` — the item being moved
    /// cannot be what it is placed before.
    fn next_sibling_skipping(&self, id: &ItemId, skip: &ItemId) -> Option<&ItemId> {
        let next = self.next_sibling(id)?;
        if next == skip {
            self.next_sibling(next)
        } else {
            Some(next)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixture::*;
    use super::*;
    use pretty_assertions::assert_eq;

    fn place(moving: &str, target: &str, position: DropPosition, open: bool) -> Option<Placement> {
        Tree::build(&sample()).drop_placement(&moving.into(), &target.into(), position, open)
    }

    fn at(parent: Option<&str>, before: Option<&str>) -> Option<Placement> {
        Some(Placement {
            parent: parent.map(ItemId::from),
            before: before.map(ItemId::from),
        })
    }

    #[test]
    fn above_is_before_the_target() {
        assert_eq!(
            place("me", "login", DropPosition::Above, false),
            at(Some("users"), Some("login"))
        );
        assert_eq!(
            place("me", "orders", DropPosition::Above, false),
            at(None, Some("orders"))
        );
    }

    #[test]
    fn over_a_folder_is_into_it() {
        assert_eq!(
            place("health", "orders", DropPosition::Over, false),
            at(Some("orders"), None)
        );
        // Over an API is above it.
        assert_eq!(
            place("me", "login", DropPosition::Over, false),
            at(Some("users"), Some("login"))
        );
    }

    #[test]
    fn below_is_after_the_target() {
        assert_eq!(
            place("me", "login", DropPosition::Below, false),
            at(Some("users"), Some("profile"))
        );
        // After the last child: the end of its folder.
        assert_eq!(
            place("me", "profile", DropPosition::Below, false),
            at(Some("users"), None)
        );
        assert_eq!(place("me", "health", DropPosition::Below, false), at(None, None));
        // Below an open folder: its first child.
        assert_eq!(
            place("health", "users", DropPosition::Below, true),
            at(Some("users"), Some("login"))
        );
        // Below a closed one: after it.
        assert_eq!(
            place("health", "users", DropPosition::Below, false),
            at(None, Some("orders"))
        );
    }

    #[test]
    fn the_moving_item_is_never_what_it_goes_before() {
        // `profile` already follows `login`: after `login` passes over it.
        assert_eq!(
            place("profile", "login", DropPosition::Below, false),
            at(Some("users"), None)
        );
        assert_eq!(
            place("login", "users", DropPosition::Below, true),
            at(Some("users"), Some("profile"))
        );
    }

    #[test]
    fn refused_drops() {
        assert_eq!(place("users", "users", DropPosition::Over, false), None);
        assert_eq!(place("users", "profile", DropPosition::Over, false), None);
        assert_eq!(place("users", "me", DropPosition::Above, false), None);
        assert_eq!(place("ghost", "users", DropPosition::Over, false), None);
    }
}
