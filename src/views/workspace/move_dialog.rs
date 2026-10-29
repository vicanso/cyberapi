//! Moving an item into another folder by picking it (F3.7 without the drag):
//! a big tree's far folders, found by name or pinyin.

use crate::states::WorkspaceState;
use crate::views::marked::marked;
use cyberapi_core::ItemId;
use cyberapi_core::search::matches_text_or_pinyin;
use cyberapi_core::tree::{Placement, Tree};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Focusable as _, InteractiveElement as _, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, Window, div, px,
};
use rust_i18n::t;

/// Every folder `moving` may go into, depth first, with the names of the
/// folders above it: not `moving` itself, nor anything inside it.
pub(super) fn destinations(tree: &Tree, moving: &ItemId) -> Vec<(ItemId, Vec<String>)> {
    fn walk(tree: &Tree, ids: &[ItemId], above: &[String], moving: &ItemId, out: &mut Vec<(ItemId, Vec<String>)>) {
        for id in ids {
            let Some(node) = tree.get(id) else { continue };
            if !node.is_folder() || id == moving {
                continue;
            }
            let mut path = above.to_vec();
            path.push(node.name().to_string());
            out.push((id.clone(), path.clone()));
            walk(tree, node.children(), &path, moving, out);
        }
    }
    let mut out = Vec::new();
    walk(tree, &tree.top_level_ids(), &[], moving, &mut out);
    out
}

pub fn open_move_dialog(
    state: WeakEntity<WorkspaceState>,
    id: ItemId,
    name: String,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(tree) = state.upgrade().map(|state| state.read(cx).tree().clone()) else {
        return;
    };
    let current = tree.get(&id).and_then(|node| node.parent()).cloned();
    let folders = destinations(&tree, &id);
    let search = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(t!("workspace.move_search").to_string())
            .clean_on_escape()
    });
    let field = search.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let keyword = search.read(cx).value().trim().to_string();
        let theme = cx.theme();
        let (muted, hover, active) = (theme.muted_foreground, theme.list_hover, theme.list_active);
        // The top level first, then each folder by its path; the one the
        // item is in now is ticked.
        let top = (None, vec![t!("workspace.move_top").to_string()]);
        let rows = std::iter::once(top)
            .chain(
                folders
                    .iter()
                    .map(|(folder, path)| (Some(folder.clone()), path.clone())),
            )
            .filter(|(folder, path)| {
                keyword.is_empty()
                    || folder.is_some() && path.last().is_some_and(|name| matches_text_or_pinyin(name, &keyword))
            })
            .enumerate()
            .map(|(index, (folder, path))| {
                let here = folder == current;
                let (state, id) = (state.clone(), id.clone());
                let target = folder.clone();
                let (name, above) = path
                    .split_last()
                    .map(|(last, rest)| (last.clone(), rest.join(" / ")))
                    .unwrap_or_default();
                h_flex()
                    .id(("move-to", index))
                    // Rows keep their height and the list scrolls.
                    .flex_none()
                    .h(px(30.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .rounded(theme.radius)
                    .text_sm()
                    .cursor_pointer()
                    .when(here, |this| this.bg(active))
                    .hover(move |style| style.bg(hover))
                    .child(
                        Icon::new(if folder.is_some() {
                            IconName::FolderClosed
                        } else {
                            IconName::Inbox
                        })
                        .small()
                        .text_color(muted),
                    )
                    .child(div().flex_none().child(marked(&name, &keyword, cx)))
                    .when(!above.is_empty(), |this| {
                        this.child(div().min_w_0().truncate().text_xs().text_color(muted).child(above))
                    })
                    .child(div().flex_1())
                    .when(here, |this| {
                        this.child(Icon::new(IconName::Check).small().text_color(muted))
                    })
                    .on_click(move |_, window, cx| {
                        if !here {
                            let placement = Placement {
                                parent: target.clone(),
                                before: None,
                            };
                            let _ = state.update(cx, |state, cx| state.move_item(id.clone(), placement, cx));
                        }
                        window.close_dialog(cx);
                    })
            })
            .collect::<Vec<_>>();
        let empty = rows.is_empty();
        dialog
            .title(t!("workspace.move_title", name = name).to_string())
            .w(px(520.))
            .child(crate::views::dialog_body(
                v_flex()
                    .gap_2()
                    .child(Input::new(&search).prefix(Icon::new(IconName::Search).text_color(muted)))
                    .child(
                        v_flex()
                            .id("move-to-list")
                            .max_h(px(360.))
                            .overflow_y_scrollbar()
                            .children(rows)
                            .when(empty, |this| {
                                this.child(
                                    div()
                                        .py_3()
                                        .text_sm()
                                        .text_color(muted)
                                        .child(t!("workspace.no_match", keyword = keyword).to_string()),
                                )
                            }),
                    ),
            ))
            // Picking is the answer: there is no button to confirm.
            .on_ok(|_, _, _| false)
    });
    // After the dialog took focus for itself, so typing starts in the field.
    field.focus_handle(cx).focus(window, cx);
}

/// F1: where a scratch request is saved into the project. Picking a folder
/// (or the top level) is the answer; there is no second dialog.
pub fn open_save_scratch_dialog(
    state: WeakEntity<WorkspaceState>,
    id: ItemId,
    name: String,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(tree) = state.upgrade().map(|state| state.read(cx).tree().clone()) else {
        return;
    };
    let folders = destinations(&tree, &id);
    let search = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(t!("workspace.move_search").to_string())
            .clean_on_escape()
    });
    let field = search.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let keyword = search.read(cx).value().trim().to_string();
        let theme = cx.theme();
        let (muted, hover) = (theme.muted_foreground, theme.list_hover);
        let top = (None, vec![t!("workspace.move_top").to_string()]);
        let rows = std::iter::once(top)
            .chain(
                folders
                    .iter()
                    .map(|(folder, path)| (Some(folder.clone()), path.clone())),
            )
            .filter(|(folder, path)| {
                keyword.is_empty()
                    || folder.is_some() && path.last().is_some_and(|name| matches_text_or_pinyin(name, &keyword))
            })
            .enumerate()
            .map(|(index, (folder, path))| {
                let (state, id) = (state.clone(), id.clone());
                let target = folder.clone();
                let (name, above) = path
                    .split_last()
                    .map(|(last, rest)| (last.clone(), rest.join(" / ")))
                    .unwrap_or_default();
                h_flex()
                    .id(("save-scratch-to", index))
                    .flex_none()
                    .h(px(30.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .rounded(theme.radius)
                    .text_sm()
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .child(
                        Icon::new(if folder.is_some() {
                            IconName::FolderClosed
                        } else {
                            IconName::Inbox
                        })
                        .small()
                        .text_color(muted),
                    )
                    .child(div().flex_none().child(marked(&name, &keyword, cx)))
                    .when(!above.is_empty(), |this| {
                        this.child(div().min_w_0().truncate().text_xs().text_color(muted).child(above))
                    })
                    .on_click(move |_, window, cx| {
                        let _ = state.update(cx, |state, cx| state.save_scratch_into(id.clone(), target.clone(), cx));
                        window.close_dialog(cx);
                    })
            })
            .collect::<Vec<_>>();
        let empty = rows.is_empty();
        dialog
            .title(t!("workspace.save_scratch_title", name = name).to_string())
            .w(px(520.))
            .child(crate::views::dialog_body(
                v_flex()
                    .gap_2()
                    .child(Input::new(&search).prefix(Icon::new(IconName::Search).text_color(muted)))
                    .child(
                        v_flex()
                            .id("save-scratch-list")
                            .max_h(px(360.))
                            .overflow_y_scrollbar()
                            .children(rows)
                            .when(empty, |this| {
                                this.child(
                                    div()
                                        .py_3()
                                        .text_sm()
                                        .text_color(muted)
                                        .child(t!("workspace.no_match", keyword = keyword).to_string()),
                                )
                            }),
                    ),
            ))
            .on_ok(|_, _, _| false)
    });
    field.focus_handle(cx).focus(window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::model::Folder;
    use cyberapi_core::tree::TreeData;

    fn folder(id: &str, children: &[&str]) -> Folder {
        Folder {
            id: id.into(),
            name: id.into(),
            children: children.iter().map(|&child| child.into()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn an_item_goes_anywhere_but_into_itself() {
        let tree = Tree::build(&TreeData {
            folders: vec![
                folder("a", &["b"]),
                folder("b", &["c"]),
                folder("c", &[]),
                folder("d", &[]),
            ],
            top_order: vec!["a".into(), "d".into()],
            ..Default::default()
        });
        let paths = |moving: &str| -> Vec<String> {
            destinations(&tree, &moving.into())
                .into_iter()
                .map(|(_, path)| path.join("/"))
                .collect()
        };
        assert_eq!(paths("x"), ["a", "a/b", "a/b/c", "d"]);
        // A folder, not into itself or what it holds.
        assert_eq!(paths("b"), ["a", "d"]);
    }
}
