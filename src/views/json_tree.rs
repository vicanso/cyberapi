//! A JSON document drawn as a tree. The switch that shows it — Text / Tree
//! for a request's body, Raw / Pretty / Tree for a response's — changes only
//! the document on screen; the default every document opens as is the local
//! setting ([`crate::json_view_local`]).

use crate::json_view_local::JsonView;
use crate::states::ApiState;
use cyberapi_core::assertions::{Assertion, Extraction, Op, Subject};
use cyberapi_core::json_view::{self, JsonNodeKind, JsonTreeNode};
use cyberapi_core::prefs::RequestTab;
use gpui_kit::component::button::{Button, ButtonGroup};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tree::{Tree, TreeEntry, TreeEvent, TreeItem, TreeState};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled, Subscription, Task, WeakEntity, Window, div, px,
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
/// PERF-3b: JSON bodies larger than this are parsed and turned into a
/// `JsonTreeNode` on a background thread so multi-megabyte responses never
/// stall the main thread when switching to the Tree view.
const ASYNC_PARSE_BYTES: usize = 256 * 1024;

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

/// A view's name, as the response's switch and the settings offer it.
pub(crate) fn json_view_label(view: JsonView) -> String {
    match view {
        JsonView::Raw => t!("editor.json_view_raw"),
        JsonView::Pretty => t!("editor.json_view_pretty"),
        JsonView::Tree => t!("editor.json_view_tree"),
    }
    .to_string()
}

/// Raw, Pretty or Tree for a response's JSON, as one outline group, `view`
/// pressed. `on_click` receives the group's selection: an index into
/// [`JsonView::ALL`].
pub(crate) fn response_view_switch(
    id: &'static str,
    view: JsonView,
    on_click: impl Fn(&Vec<usize>, &mut Window, &mut App) + 'static,
) -> ButtonGroup {
    JsonView::ALL
        .into_iter()
        .enumerate()
        .fold(
            ButtonGroup::new(id).compact().small().outline(),
            |group, (index, choice)| {
                group.child(
                    Button::new((id, index))
                        .label(json_view_label(choice))
                        .selected(choice == view),
                )
            },
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
    /// The API whose response this tree displays (`None` for a request's
    /// body tree).
    api: Option<Entity<ApiState>>,
    /// The text last drawn, so an unchanged body does not rebuild the rows.
    text: String,
    doc: Option<Value>,
    rows: Rc<HashMap<SharedString, Row>>,
    /// Containers the reader opened, kept across a rebuild.
    expanded: HashSet<SharedString>,
    /// What the view says when the text is not JSON.
    invalid: SharedString,
    /// Why there is no tree to show, said in its place.
    unavailable: Option<SharedString>,
    /// PERF-3b: whether a large JSON body is being parsed in the background.
    loading: bool,
    parse_task: Option<Task<()>>,
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
            api: None,
            text: String::new(),
            doc: None,
            rows: Rc::new(HashMap::new()),
            expanded: HashSet::new(),
            invalid: t!("editor.json_tree_invalid").to_string().into(),
            unavailable: None,
            loading: false,
            parse_task: None,
            _subscriptions: vec![subscription],
        }
    }

    /// A tree of `api`'s response, which the reader cannot fix: its note
    /// says only that the body is not JSON, and its row menu offers
    /// copying a `{{g(…)}}` reference or adding an assertion / extraction
    /// (A8).
    pub(crate) fn for_response(mut self, api: Entity<ApiState>) -> Self {
        self.api = Some(api);
        self.invalid = t!("response.json_tree_invalid").to_string().into();
        self
    }

    /// Shows `reason` in place of a tree.
    pub(crate) fn set_unavailable(&mut self, reason: SharedString, cx: &mut Context<Self>) {
        self.parse_task = None;
        self.loading = false;
        self.text.clear();
        self.doc = None;
        self.unavailable = Some(reason);
        self.rebuild_from_node(None, cx);
    }

    /// Show `text`. Blank or not JSON leaves the tree empty, and the view
    /// says so.
    pub(crate) fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if text == self.text && self.unavailable.is_none() {
            return;
        }
        self.parse_task = None;
        self.unavailable = None;
        self.text = text.to_string();
        if text.len() > ASYNC_PARSE_BYTES {
            self.loading = true;
            self.doc = None;
            self.rebuild_from_node(None, cx);
            let owned = self.text.clone();
            let work = cx.background_spawn(async move {
                let doc: Option<Value> = serde_json::from_str(&owned).ok();
                let root = doc.as_ref().map(json_view::json_tree);
                (doc, root)
            });
            self.parse_task = Some(cx.spawn(async move |this, cx| {
                let (doc, root) = work.await;
                let _ = this.update(cx, |this, cx| {
                    this.loading = false;
                    this.parse_task = None;
                    this.doc = doc;
                    this.rebuild_from_node(root.as_ref(), cx);
                });
            }));
            return;
        }
        self.loading = false;
        self.doc = serde_json::from_str(text).ok();
        let root = self.doc.as_ref().map(json_view::json_tree);
        self.rebuild_from_node(root.as_ref(), cx);
    }

    fn rebuild_from_node(&mut self, root: Option<&JsonTreeNode>, cx: &mut Context<Self>) {
        let mut rows = HashMap::new();
        let items = match root {
            Some(node) => vec![self.item_for(node, true, &mut rows)],
            None => Vec::new(),
        };
        self.rows = Rc::new(rows);
        self.tree_state.update(cx, |state, cx| state.set_items(items, cx));
        cx.notify();
    }

    fn item_for(&self, node: &JsonTreeNode, is_root: bool, rows: &mut HashMap<SharedString, Row>) -> TreeItem {
        let id: SharedString = node.path.clone().into();
        // A container past the depth cap is one row: its preview, not "0 fields".
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

    /// A8: copies `{{g(<api>, <path>)}}` for `path` into the clipboard.
    fn copy_get_expr(&mut self, path: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        let Some(api) = &self.api else {
            return;
        };
        let api_id = api.read(cx).api().as_str().to_string();
        let get_path = path
            .strip_prefix("$.")
            .or_else(|| path.strip_prefix('$'))
            .unwrap_or(path.as_ref());
        let expr = format!("{{{{g({api_id}, {get_path})}}}}");
        cx.write_to_clipboard(ClipboardItem::new_string(expr));
        window.push_notification(Notification::success(t!("common.copied").to_string()), cx);
    }

    /// A8: adds a JSONPath assertion for `path` against its current value in
    /// the response tree and opens the Assertions tab.
    fn add_assertion(&mut self, path: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let Some(value) = self
            .doc
            .as_ref()
            .and_then(|doc| json_view::value_at(doc, path.as_ref()))
        else {
            return;
        };
        let (op, expected) = match value {
            Value::String(text) => (Op::Equals, text.clone()),
            Value::Number(num) => (Op::Equals, num.to_string()),
            Value::Bool(flag) => (Op::Equals, flag.to_string()),
            Value::Null => (Op::IsType, "null".into()),
            Value::Object(_) => (Op::IsType, "object".into()),
            Value::Array(_) => (Op::IsType, "array".into()),
        };
        let assertion = Assertion {
            enabled: true,
            subject: Subject::Json,
            target: path.to_string(),
            op,
            expected,
            imported: false,
        };
        let workspace = api.read(cx).workspace().upgrade();
        api.update(cx, |api, cx| api.add_assertion(assertion, cx));
        if let Some(workspace) = workspace {
            workspace.update(cx, |workspace, cx| {
                workspace.set_tab(&api, RequestTab::Assertions, cx);
            });
        }
        window.push_notification(
            Notification::success(t!("response.json_tree_assertion_added", path = path.as_ref()).to_string()),
            cx,
        );
    }

    /// A8: adds a JSONPath extraction for `path` (naming the variable after
    /// the last key segment) and opens the Assertions tab.
    fn add_extraction(&mut self, path: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let name = suggested_extraction_name(path.as_ref());
        let extraction = Extraction {
            enabled: true,
            name: name.clone(),
            subject: Subject::Json,
            target: path.to_string(),
        };
        let workspace = api.read(cx).workspace().upgrade();
        api.update(cx, |api, cx| api.add_extraction(extraction, cx));
        if let Some(workspace) = workspace {
            workspace.update(cx, |workspace, cx| {
                workspace.set_tab(&api, RequestTab::Assertions, cx);
            });
        }
        window.push_notification(
            Notification::success(t!("response.json_tree_extraction_added", name = name).to_string()),
            cx,
        );
    }
}

fn suggested_extraction_name(path: &str) -> String {
    let candidate = path
        .trim_end_matches(']')
        .rsplit(['.', '[', '\''])
        .find(|part| !part.is_empty() && *part != "$" && !part.chars().all(|ch| ch.is_ascii_digit()))
        .unwrap_or("value");
    candidate.to_string()
}

impl Render for JsonTree {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.loading {
            return h_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .p_4()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(Spinner::new())
                .child(t!("dashboard.loading").to_string())
                .into_any_element();
        }
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
                        .child(self.unavailable.clone().unwrap_or_else(|| self.invalid.clone())),
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
    let (has_value, is_response, is_scratch) = tree
        .read_with(cx, |tree, cx| {
            let has_value = tree.rows.get(&id).and_then(|row| row.kind).is_some();
            let is_response = tree.api.is_some();
            let is_scratch = tree.api.as_ref().is_some_and(|api| api.read(cx).is_scratch());
            (has_value, is_response, is_scratch)
        })
        .unwrap_or((false, false, false));
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
    let mut menu = menu
        .item(item(false, t!("editor.json_tree_copy_path").to_string()))
        .item(item(true, t!("editor.json_tree_copy_value").to_string()));
    if is_response {
        menu = menu.separator();
        if id.as_ref() != "$" && !is_scratch {
            let tree = tree.clone();
            let id = id.clone();
            menu = menu.item(
                PopupMenuItem::new(t!("response.json_tree_copy_get").to_string())
                    .icon(IconName::Copy)
                    .on_click(move |_, window, cx| {
                        let _ = tree.update(cx, |tree, cx| tree.copy_get_expr(id.clone(), window, cx));
                    }),
            );
        }
        {
            let tree = tree.clone();
            let id = id.clone();
            menu = menu.item(
                PopupMenuItem::new(t!("response.json_tree_add_assertion").to_string())
                    .icon(IconName::CircleCheck)
                    .on_click(move |_, window, cx| {
                        let _ = tree.update(cx, |tree, cx| tree.add_assertion(id.clone(), window, cx));
                    }),
            );
        }
        {
            let tree = tree.clone();
            let id = id.clone();
            menu = menu.item(
                PopupMenuItem::new(t!("response.json_tree_add_extraction").to_string())
                    .icon(IconName::Plus)
                    .on_click(move |_, window, cx| {
                        let _ = tree.update(cx, |tree, cx| tree.add_extraction(id.clone(), window, cx));
                    }),
            );
        }
    }
    menu
}
