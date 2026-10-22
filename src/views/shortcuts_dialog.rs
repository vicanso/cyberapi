//! Every shortcut there is, in one list: they are bound in many places and
//! shown only beside some of them. The keys are read from the bindings, so
//! the list says what this platform takes.

use super::dashboard::{FocusSearch, KEY_CONTEXT as DASHBOARD};
use super::workspace::{
    KEY_CONTEXT as WORKSPACE, NewApi, NewFolder, Pin1, Pin9, Send as SendAction, TREE_CONTEXT as TREE, TreeClose,
    TreeNext, TreeOpen, TreePrevious, TreeRename, TreeToggle,
};
use crate::this_build;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{Action, AnyElement, App, IntoElement, ParentElement as _, Styled as _, Window, div, px};
use rust_i18n::t;

pub fn open_shortcuts_dialog(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, window, cx| {
        dialog
            .title(t!("shortcuts.title").to_string())
            .w(px(460.))
            .child(shortcuts(window, cx))
    });
}

/// The keys an action is bound to in `context`, as this platform writes them.
fn keys(action: &dyn Action, context: &str, window: &Window) -> Vec<AnyElement> {
    Kbd::binding_for_action(action, Some(context), window)
        .map(|kbd| vec![kbd.into_any_element()])
        .unwrap_or_default()
}

fn shortcuts(window: &mut Window, cx: &mut App) -> impl IntoElement {
    let new_keys = this_build::takes_new_keys();
    let mut workspace = Vec::new();
    if new_keys {
        workspace.push((t!("workspace.new_api").to_string(), keys(&NewApi, WORKSPACE, window)));
        workspace.push((
            t!("workspace.new_folder").to_string(),
            keys(&NewFolder, WORKSPACE, window),
        ));
    }
    workspace.push((t!("shortcuts.send").to_string(), keys(&SendAction, WORKSPACE, window)));
    if this_build::takes_pin_keys() {
        workspace.push((t!("shortcuts.pins").to_string(), {
            let mut both = keys(&Pin1, WORKSPACE, window);
            if !both.is_empty() {
                both.push(div().child("–").into_any_element());
                both.extend(keys(&Pin9, WORKSPACE, window));
            }
            both
        }));
    }
    let tree = vec![
        (t!("shortcuts.tree_move").to_string(), {
            let mut both = keys(&TreePrevious, TREE, window);
            both.extend(keys(&TreeNext, TREE, window));
            both
        }),
        (t!("shortcuts.tree_close").to_string(), keys(&TreeClose, TREE, window)),
        (t!("shortcuts.tree_open").to_string(), keys(&TreeOpen, TREE, window)),
        (t!("shortcuts.tree_toggle").to_string(), keys(&TreeToggle, TREE, window)),
        (t!("workspace.rename").to_string(), keys(&TreeRename, TREE, window)),
    ];
    let projects = vec![(
        t!("shortcuts.search_projects").to_string(),
        keys(&FocusSearch, DASHBOARD, window),
    )];
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
