//! Every shortcut there is, in one list: they are bound in many places and
//! shown only beside some of them. The keys are read from the bindings, so
//! the list says what this platform takes.

use super::dashboard::{FocusSearch, KEY_CONTEXT as DASHBOARD};
use super::workspace::{
    CloseTab, FindApi, FocusUrl, KEY_CONTEXT as WORKSPACE, NewApi, NewFolder, NextTab, OpenPalette, PasteCurl, Pin1,
    Pin9, PreviousTab, ROOT_CONTEXT, Send as SendAction, TREE_CONTEXT as TREE, ToggleComment, TreeClose, TreeDuplicate,
    TreeNext, TreeOpen, TreePrevious, TreeRemove, TreeRename, TreeToggle,
};
use crate::this_build;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    Action, AnyElement, App, AsKeystroke as _, IntoElement, ParentElement as _, Styled as _, Window, div, px,
};
use rust_i18n::t;

pub fn open_shortcuts_dialog(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _, cx| {
        dialog
            .title(t!("shortcuts.title").to_string())
            .w(px(460.))
            .child(shortcuts(cx))
    });
}

/// The keys an action is bound to in `context`, as this platform writes them.
///
/// `context` is the binding's predicate (`Workspace`, or `Workspace > Input`),
/// not a focus stack. `Kbd::binding_for_action` parses it with `KeyContext`,
/// whose parser never consumes `>` and recurses until the process is killed.
fn keys(action: &dyn Action, context: &str, cx: &App) -> Vec<AnyElement> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let stroke = keymap.bindings_for_action(action).rev().find_map(|binding| {
        let same = binding
            .predicate()
            .is_some_and(|predicate| predicate.to_string() == context);
        same.then(|| binding.keystrokes().first().map(|key| key.as_keystroke().clone()))
            .flatten()
    });
    stroke
        .map(|stroke| vec![Kbd::new(stroke).into_any_element()])
        .unwrap_or_default()
}

fn shortcuts(cx: &mut App) -> impl IntoElement {
    let new_keys = this_build::takes_new_keys();
    let mut workspace = Vec::new();
    if new_keys {
        workspace.push((t!("workspace.new_api").to_string(), keys(&NewApi, WORKSPACE, cx)));
        workspace.push((t!("workspace.new_folder").to_string(), keys(&NewFolder, WORKSPACE, cx)));
    }
    workspace.push((t!("import.paste_curl").to_string(), keys(&PasteCurl, WORKSPACE, cx)));
    workspace.push((t!("shortcuts.send").to_string(), keys(&SendAction, WORKSPACE, cx)));
    workspace.push((
        t!("shortcuts.toggle_comment").to_string(),
        keys(&ToggleComment, "Workspace > Input", cx),
    ));
    workspace.push((t!("shortcuts.find_api").to_string(), keys(&FindApi, WORKSPACE, cx)));
    let browser_keys = this_build::takes_browser_keys();
    if browser_keys {
        workspace.push((t!("shortcuts.focus_url").to_string(), keys(&FocusUrl, WORKSPACE, cx)));
    }
    // F1: a browser keeps these for its own tabs.
    if browser_keys {
        workspace.push((t!("shortcuts.close_tab").to_string(), keys(&CloseTab, WORKSPACE, cx)));
        workspace.push((t!("shortcuts.switch_tab").to_string(), {
            let mut both = keys(&NextTab, WORKSPACE, cx);
            both.extend(keys(&PreviousTab, WORKSPACE, cx));
            both
        }));
    }
    if browser_keys {
        workspace.push((t!("shortcuts.pins").to_string(), {
            let mut both = keys(&Pin1, WORKSPACE, cx);
            if !both.is_empty() {
                both.push(div().child("–").into_any_element());
                both.extend(keys(&Pin9, WORKSPACE, cx));
            }
            both
        }));
    }
    let mut tree = vec![
        (t!("shortcuts.tree_move").to_string(), {
            let mut both = keys(&TreePrevious, TREE, cx);
            both.extend(keys(&TreeNext, TREE, cx));
            both
        }),
        (t!("shortcuts.tree_close").to_string(), keys(&TreeClose, TREE, cx)),
        (t!("shortcuts.tree_open").to_string(), keys(&TreeOpen, TREE, cx)),
        (t!("shortcuts.tree_toggle").to_string(), keys(&TreeToggle, TREE, cx)),
        (t!("workspace.rename").to_string(), keys(&TreeRename, TREE, cx)),
        (t!("common.delete").to_string(), keys(&TreeRemove, TREE, cx)),
    ];
    if browser_keys {
        tree.push((t!("workspace.duplicate").to_string(), keys(&TreeDuplicate, TREE, cx)));
    }
    let mut projects = vec![(
        t!("shortcuts.search_projects").to_string(),
        keys(&FocusSearch, DASHBOARD, cx),
    )];
    // F2: a browser on Windows or Linux takes Ctrl-K for its own search.
    if browser_keys {
        projects.insert(
            0,
            (
                t!("shortcuts.palette").to_string(),
                keys(&OpenPalette, ROOT_CONTEXT, cx),
            ),
        );
    }
    let muted = cx.theme().muted_foreground;
    let section = |title: String, rows: Vec<(String, Vec<AnyElement>)>| {
        v_flex()
            .gap_1()
            .child(div().text_xs().text_color(muted).child(title))
            .children(
                rows.into_iter()
                    .filter(|(_, keys)| !keys.is_empty())
                    .map(|(label, keys)| {
                        h_flex()
                            .h(px(28.))
                            .items_center()
                            .justify_between()
                            .text_sm()
                            .child(label)
                            .child(h_flex().gap_1().items_center().children(keys))
                    }),
            )
    };
    v_flex()
        .gap_4()
        .child(section(t!("shortcuts.workspace").to_string(), workspace))
        .child(section(t!("shortcuts.tree").to_string(), tree))
        .child(section(t!("shortcuts.projects").to_string(), projects))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    /// #93: listing the comment shortcut used to parse `Workspace > Input`
    /// as a focus context, and that parser never returns.
    #[gpui_kit::test]
    fn the_comment_shortcut_is_listed_without_hanging(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::views::workspace::bind_keys(cx);
            assert!(
                !keys(&ToggleComment, "Workspace > Input", cx).is_empty(),
                "the nested binding is shown"
            );
            assert!(!keys(&FindApi, WORKSPACE, cx).is_empty());
        });
    }
}
