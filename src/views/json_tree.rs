//! A JSON document drawn as a tree. The Text / Tree switch that shows it
//! changes only the document on screen; the default every document opens as
//! is the local setting ([`crate::json_view_local`]).

use cyberapi_core::json_view::{self, JsonNodeKind, JsonTreeNode};
use gpui_kit::component::button::{Button, ButtonGroup};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tree::{Tree, TreeEntry, TreeEvent, TreeItem, TreeState};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled, Subscription, WeakEntity, Window, div, px,
};
use rust_i18n::t;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Horizontal step per nesting level.
const INDENT: f32 = 16.;
/// Left inset of every row, chevron included.
const ROW_INSET: f32 = 8.;
/// Width a scalar row leaves blank where a container has its chevron.
const CHEVRON_WIDTH: f32 = 16.;

/// Text or Tree, as one outline group. `tree` is the pressed side.
/// `on_click` receives the group's selection: index 1 is Tree.
pub(crate) fn json_view_switch(
    id: &'static str,
    tree: bool,
    on_click: impl Fn(&Vec<usize>, &mut Window, &mut App) + 'static,
) -> ButtonGroup {
    ButtonGroup::new(id)
        .compact()
        .small()
        .outline()
        .child(
            Button::new((id, 0_usize))
                .label(t!("editor.json_view_text").to_string())
                .selected(!tree),
        )
        .child(
            Button::new((id, 1_usize))
                .label(t!("editor.json_view_tree").to_string())
                .selected(tree),
        )
        .on_click(on_click)
}

/// What a row shows. The tree item's id is the node's path.
struct Row {
    name: SharedString,
    /// `None` for the marker standing in for members past the cap.
    kind: Option<JsonNodeKind>,
    detail: SharedString,
}

pub(crate) struct JsonTree {
    tree_state: Entity<TreeState>,
    /// The text last drawn, so an unchanged body does not rebuild the rows.
    text: String,
    doc: Option<Value>,
    rows: Rc<HashMap<SharedString, Row>>,
    /// Containers the reader opened, kept across a rebuild.
    expanded: HashSet<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl JsonTree {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let tree_state = cx.new(|cx| TreeState::new(cx));
        let subscription = cx.subscribe(&tree_state, |this, _, event: &TreeEvent, _| match event {
            TreeEvent::Expanded(id) => {
                this.expanded.insert(id.clone());
            }
            TreeEvent::Collapsed(id) => {
                this.expanded.remove(id);
            }
        });
        Self {
            tree_state,
            text: String::new(),
            doc: None,
            rows: Rc::new(HashMap::new()),
            expanded: HashSet::new(),
            _subscriptions: vec![subscription],
        }
    }

    /// Show `text`. Blank or not JSON leaves the tree empty, and the view
    /// says so.
    pub(crate) fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if text == self.text {
            return;
        }
        self.text = text.to_string();
        self.doc = serde_json::from_str(text).ok();
        self.rebuild(cx);
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let mut rows = HashMap::new();
        let items = match &self.doc {
            Some(doc) => vec![self.item_for(&json_view::json_tree(doc), true, &mut rows)],
            None => Vec::new(),
        };
        self.rows = Rc::new(rows);
        self.tree_state.update(cx, |state, cx| state.set_items(items, cx));
        cx.notify();
    }

    fn item_for(&self, node: &JsonTreeNode, is_root: bool, rows: &mut HashMap<SharedString, Row>) -> TreeItem {
        let id: SharedString = node.path.clone().into();
        // A container past the depth cap is one row: its preview, not "0 members".
        let detail: SharedString = if node.preview.is_empty() {
            match node.kind {
                JsonNodeKind::Object => crate::i18n::counted("editor.json_tree_member", node.len).into(),
                JsonNodeKind::Array => crate::i18n::counted("editor.json_tree_item", node.len).into(),
                _ => SharedString::default(),
            }
        } else {
            node.preview.clone().into()
        };
        rows.insert(
            id.clone(),
            Row {
                name: node.name.clone().into(),
                kind: Some(node.kind),
                detail,
            },
        );
        let mut item = TreeItem::new(id.clone(), node.name.clone()).expanded(is_root || self.expanded.contains(&id));
        for child in &node.children {
            item = item.child(self.item_for(child, false, rows));
        }
        if node.omitted > 0 {
            let more_id: SharedString = format!("{}#more", node.path).into();
            let label: SharedString = t!("editor.json_tree_more", count = node.omitted).to_string().into();
            rows.insert(
                more_id.clone(),
                Row {
                    name: label.clone(),
                    kind: None,
                    detail: SharedString::default(),
                },
            );
            item = item.child(TreeItem::new(more_id, label).disabled(true));
        }
        item
    }

    fn copy(&mut self, path: SharedString, value: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = if value {
            self.doc
                .as_ref()
                .and_then(|doc| json_view::value_at(doc, path.as_ref()))
                .map(cyberapi_core::format::json)
        } else {
            Some(path.to_string())
        };
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            window.push_notification(Notification::success(t!("common.copied").to_string()), cx);
        }
    }
}

impl Render for JsonTree {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.doc.is_none() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .p_4()
                .child(
                    div()
                        .text_sm()
                        .text_center()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("editor.json_tree_invalid").to_string()),
                )
                .into_any_element();
        }
        let rows = self.rows.clone();
        let tree = cx.entity().downgrade();
        Tree::new(&self.tree_state, move |ix, entry, _selected, _window, cx| {
            render_row(ix, entry, &rows, cx)
        })
        .context_menu(move |_ix, entry, menu, window, cx| row_menu(&tree, entry, menu, window, cx))
        .size_full()
        .into_any_element()
    }
}

fn render_row(ix: usize, entry: &TreeEntry, rows: &HashMap<SharedString, Row>, cx: &App) -> ListItem {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let row = rows.get(&entry.item().id);
    let name = row.map(|row| row.name.clone()).unwrap_or_default();
    let kind = row.and_then(|row| row.kind);
    let detail = row.map(|row| row.detail.clone()).unwrap_or_default();
    let detail_color = match kind {
        Some(JsonNodeKind::String) => theme.green,
        Some(JsonNodeKind::Number) => theme.blue,
        Some(JsonNodeKind::Bool) => theme.magenta,
        _ => muted,
    };
    let scalar = kind.is_some_and(|kind| !kind.is_container());
    let chevron = entry.is_folder().then(|| {
        if entry.is_expanded() {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        }
    });
    ListItem::new(("json-row", ix)).w_full().py_0p5().px_2().child(
        h_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .items_center()
            .pl(px(INDENT * entry.depth() as f32 + ROW_INSET))
            .font_family(theme.mono_font_family.clone())
            .text_xs()
            .child(match chevron {
                Some(icon) => Icon::new(icon).xsmall().text_color(muted).into_any_element(),
                None => div().w(px(CHEVRON_WIDTH)).flex_shrink_0().into_any_element(),
            })
            .child(div().text_color(theme.foreground).whitespace_nowrap().child(name))
            .when(scalar, |this| this.child(div().text_color(muted).child(":")))
            .when(!detail.is_empty(), |this| {
                this.child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_color(detail_color)
                        .child(detail),
                )
            }),
    )
}

fn row_menu(
    tree: &WeakEntity<JsonTree>,
    entry: &TreeEntry,
    menu: PopupMenu,
    _window: &mut Window,
    cx: &mut Context<TreeState>,
) -> PopupMenu {
    let id = entry.item().id.clone();
    let has_value = tree
        .read_with(cx, |tree, _| tree.rows.get(&id).and_then(|row| row.kind))
        .ok()
        .flatten()
        .is_some();
    if !has_value {
        return menu;
    }
    let item = |value: bool, label: String| {
        let tree = tree.clone();
        let id = id.clone();
        PopupMenuItem::new(label)
            .icon(IconName::Copy)
            .on_click(move |_, window, cx| {
                let _ = tree.update(cx, |tree, cx| tree.copy(id.clone(), value, window, cx));
            })
    };
    menu.item(item(false, t!("editor.json_tree_copy_path").to_string()))
        .item(item(true, t!("editor.json_tree_copy_value").to_string()))
}
