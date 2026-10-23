//! The project's tree (M3): search, open and close, select, create, rename,
//! move by dragging, delete. The rows and every rule behind them are the
//! state's and core's; this draws the rows and turns pointer and keyboard
//! input into intent.

use super::move_dialog::open_move_dialog;
use super::{
    NewApi, NewFolder, PasteCurl, confirm_delete, export, method_color, method_label, new_item, open_import_dialog,
    paste_curl,
};
use crate::states::{Load, WorkspaceState};
use crate::this_build;
use crate::views::marked::marked;
use cyberapi_core::ItemId;
use cyberapi_core::tree::{DropPosition, ItemKind, Placement, Row};
use gpui_kit::assets::IconName as Icons;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::empty::{
    Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Bounds, Context, DragMoveEvent, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Pixels, Point, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div, px,
};
use rust_i18n::t;

/// How far each level of the tree is indented.
const INDENT: f32 = 14.;
const ROW_HEIGHT: f32 = 28.;

gpui_kit::actions!(tree, [Previous, Next, Close, Open, Toggle, Rename]);

pub(crate) const KEY_CONTEXT: &str = "Tree";

/// The tree from the keyboard, as a file list is walked: up and down move
/// (an API reached is opened), left closes a folder or goes to the one
/// above, right opens one or goes into it, Enter opens or closes, F2
/// renames. A field in the tree keeps these keys for itself.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", Previous, Some(KEY_CONTEXT)),
        KeyBinding::new("down", Next, Some(KEY_CONTEXT)),
        KeyBinding::new("left", Close, Some(KEY_CONTEXT)),
        KeyBinding::new("right", Open, Some(KEY_CONTEXT)),
        KeyBinding::new("enter", Toggle, Some(KEY_CONTEXT)),
        KeyBinding::new("f2", Rename, Some(KEY_CONTEXT)),
    ]);
}

/// An item whose name is being edited in place (F3.5).
struct Renaming {
    id: ItemId,
    input: Entity<InputState>,
    _events: Subscription,
}

/// The item being dragged (F3.7); it also draws what follows the pointer.
#[derive(Clone)]
struct DraggedItem {
    id: ItemId,
    name: SharedString,
}

impl Render for DraggedItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .text_sm()
            .shadow_md()
            .child(self.name.clone())
    }
}

/// Where the dragged item would land, as drawn.
#[derive(Clone, Debug, PartialEq)]
enum DropTarget {
    Row {
        id: ItemId,
        position: DropPosition,
    },
    /// Below every row: the end of the top level.
    End,
}

pub struct TreePanel {
    state: Entity<WorkspaceState>,
    search: Entity<InputState>,
    renaming: Option<Renaming>,
    scroll: ScrollHandle,
    /// The rows as last drawn; the pointer is read against them.
    drawn: Vec<Row>,
    drop_target: Option<DropTarget>,
    /// The tree's own focus, for its keys.
    focus: FocusHandle,
    /// The row the keys act on: an API or a folder. The selected API's row
    /// when there is none (or it has gone).
    cursor: Option<ItemId>,
    /// The selection the cursor was last put beside: one made elsewhere (a
    /// pin) takes the cursor with it.
    seen_selection: Option<ItemId>,
    _subscriptions: Vec<Subscription>,
}

impl TreePanel {
    pub fn new(state: Entity<WorkspaceState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("workspace.search").to_string())
                .clean_on_escape()
        });
        let subscriptions = vec![
            cx.observe(&state, |this, state, cx| {
                let selected = state.read(cx).selected().cloned();
                if selected != this.seen_selection {
                    this.seen_selection = selected;
                    this.cursor = None;
                }
                cx.notify();
            }),
            cx.subscribe_in(&search, window, |this, search, event, window, cx| match event {
                InputEvent::Change => {
                    let keyword = search.read(cx).value().to_string();
                    this.state.update(cx, |state, cx| state.set_keyword(keyword, cx));
                }
                // What was searched for, opened: the first API found.
                InputEvent::PressEnter { .. } => {
                    let rows = this.state.read(cx).rows();
                    if let Some(index) = rows.iter().position(|row| row.kind == ItemKind::Api) {
                        this.focus.focus(window, cx);
                        this.put_cursor(&rows, index, cx);
                    }
                }
                _ => {}
            }),
        ];
        Self {
            state,
            search,
            renaming: None,
            scroll: ScrollHandle::new(),
            drawn: Vec::new(),
            drop_target: None,
            focus: cx.focus_handle(),
            cursor: None,
            seen_selection: None,
            _subscriptions: subscriptions,
        }
    }

    /// Ctrl+N / Ctrl+Shift+N create inside the row that has the keys (#45).
    /// The workspace still handles the same keys from the editor, at the root.
    fn new_here(&mut self, kind: ItemKind, window: &mut Window, cx: &mut Context<Self>) {
        let parent = self.cursor_folder(cx);
        new_item(&self.state.downgrade(), parent, kind, window, cx);
    }

    /// F10.5, where the cursor is.
    fn paste_here(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = self.cursor_folder(cx);
        paste_curl(&self.state.downgrade(), parent, window, cx);
    }

    /// The folder a new item at the cursor goes in.
    fn cursor_folder(&self, cx: &App) -> Option<ItemId> {
        let rows = self.state.read(cx).rows();
        self.cursor_at(&rows, cx)
            .and_then(|index| rows.get(index))
            .and_then(Row::insertion_parent)
    }

    /// Where the cursor is among `rows`.
    fn cursor_at(&self, rows: &[Row], cx: &App) -> Option<usize> {
        let selected = self.state.read(cx).selected();
        let find = |id: Option<&ItemId>| id.and_then(|id| rows.iter().position(|row| &row.id == id));
        find(self.cursor.as_ref()).or_else(|| find(selected))
    }

    /// Moves the cursor to `rows[index]`, in view; an API there is opened.
    fn put_cursor(&mut self, rows: &[Row], index: usize, cx: &mut Context<Self>) {
        let Some(row) = rows.get(index) else { return };
        if row.kind == ItemKind::Api {
            self.state.update(cx, |state, cx| state.select(&row.id, cx));
        }
        self.cursor = Some(row.id.clone());
        self.scroll.scroll_to_item(index);
        cx.notify();
    }

    fn step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let rows = self.state.read(cx).rows();
        let last = rows.len().saturating_sub(1);
        let index = match self.cursor_at(&rows, cx) {
            Some(index) if forward => (index + 1).min(last),
            Some(index) => index.saturating_sub(1),
            None if forward => 0,
            None => last,
        };
        self.put_cursor(&rows, index, cx);
    }

    /// Left: an open folder closes; anything else goes to its folder.
    /// Right: a closed folder opens; an open one is gone into. A search
    /// opens every folder, so it only moves.
    fn close_or_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let rows = self.state.read(cx).rows();
        let Some(index) = self.cursor_at(&rows, cx) else { return };
        let row = &rows[index];
        let searching = !self.state.read(cx).keyword().trim().is_empty();
        let folder = row.kind == ItemKind::Folder;
        if folder && row.open != open && !searching {
            let id = row.id.clone();
            self.cursor = Some(id.clone());
            self.state.update(cx, |state, cx| state.toggle_folder(&id, cx));
        } else if open {
            if folder && rows.get(index + 1).is_some_and(|next| next.depth > row.depth) {
                self.put_cursor(&rows, index + 1, cx);
            }
        } else if let Some(parent) = rows
            .iter()
            .position(|candidate| Some(&candidate.id) == row.parent.as_ref())
        {
            self.put_cursor(&rows, parent, cx);
        }
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let rows = self.state.read(cx).rows();
        if let Some(row) = self.cursor_at(&rows, cx).map(|index| &rows[index])
            && row.kind == ItemKind::Folder
        {
            let id = row.id.clone();
            self.state.update(cx, |state, cx| state.toggle_folder(&id, cx));
        }
    }

    fn rename_at_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) || !self.state.read(cx).editable() {
            return;
        }
        let rows = self.state.read(cx).rows();
        let Some(index) = self.cursor_at(&rows, cx) else { return };
        let id = rows[index].id.clone();
        let name = self.state.read(cx).tree().get(&id).map(|node| node.name().to_string());
        if let Some(name) = name {
            self.start_rename(id, name, window, cx);
        }
    }

    /// The target under the pointer, if dropping there moves `moving`. Rows
    /// are all [`ROW_HEIGHT`] tall, so the row is the pointer's offset into
    /// the scrolled list. A folder's middle half means into it; the edges,
    /// and an API's halves, mean beside.
    fn drop_target_at(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        moving: &ItemId,
        cx: &App,
    ) -> Option<DropTarget> {
        if !bounds.contains(&position) {
            return None;
        }
        let y = f32::from(position.y - bounds.origin.y - self.scroll.offset().y) / ROW_HEIGHT;
        let target = match self.drawn.get(y.max(0.).floor() as usize) {
            None => DropTarget::End,
            Some(row) => {
                let within = y.fract();
                let position = match row.kind {
                    ItemKind::Folder if within < 0.25 => DropPosition::Above,
                    ItemKind::Folder if within > 0.75 => DropPosition::Below,
                    ItemKind::Folder => DropPosition::Over,
                    ItemKind::Api if within < 0.5 => DropPosition::Above,
                    ItemKind::Api => DropPosition::Below,
                };
                DropTarget::Row {
                    id: row.id.clone(),
                    position,
                }
            }
        };
        self.placement(moving, &target, cx).map(|_| target)
    }

    fn placement(&self, moving: &ItemId, target: &DropTarget, cx: &App) -> Option<Placement> {
        let tree = self.state.read(cx).tree();
        match target {
            DropTarget::Row { id, position } => {
                let open = self.drawn.iter().any(|row| &row.id == id && row.open);
                tree.drop_placement(moving, id, *position, open)
            }
            DropTarget::End => tree.contains(moving).then(Placement::default),
        }
    }

    fn on_drag_move(&mut self, event: &DragMoveEvent<DraggedItem>, cx: &mut Context<Self>) {
        let moving = event.drag(cx).id.clone();
        let target = self.drop_target_at(event.event.position, event.bounds, &moving, cx);
        if self.drop_target != target {
            self.drop_target = target;
            cx.notify();
        }
    }

    /// F3.7: the drop moves the item where the indicator showed.
    fn on_drop(&mut self, dragged: &DraggedItem, cx: &mut Context<Self>) {
        let Some(target) = self.drop_target.take() else {
            return;
        };
        if let Some(placement) = self.placement(&dragged.id, &target, cx) {
            let id = dragged.id.clone();
            self.state.update(cx, |state, cx| state.move_item(id, placement, cx));
        }
        cx.notify();
    }

    /// F3.5: the name becomes an input; Enter or leaving it saves, Escape
    /// does not.
    fn start_rename(&mut self, id: ItemId, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        input.focus_handle(cx).focus(window, cx);
        // Typing replaces the old name; an arrow key keeps it.
        input.update(cx, |input, cx| input.select_all(window, cx));
        let events = cx.subscribe(&input, |this, _, event, cx| match event {
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_rename(cx),
            _ => {}
        });
        self.renaming = Some(Renaming {
            id,
            input,
            _events: events,
        });
        cx.notify();
    }

    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(renaming) = self.renaming.take() else {
            return;
        };
        let name = renaming.input.read(cx).value().trim().to_string();
        let unchanged = self
            .state
            .read(cx)
            .tree()
            .get(&renaming.id)
            .is_some_and(|node| node.name() == name);
        // An empty name is not saved (F3.5): the old one stays.
        if !name.is_empty() && !unchanged {
            self.state.update(cx, |state, cx| state.rename(renaming.id, name, cx));
        }
        cx.notify();
    }

    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
            cx.notify();
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let state = self.state.downgrade();
        let editable = self.state.read(cx).editable();
        // `muted` and `border` are the same tone in the dark theme.
        let border = cx.theme().foreground.opacity(0.12);
        h_flex()
            .h(px(44.))
            .px_2()
            .gap_1()
            .items_center()
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    // Down from the search goes on into what it found.
                    .capture_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                        let keystroke = &event.keystroke;
                        if keystroke.key == "down" && !keystroke.modifiers.modified() {
                            cx.stop_propagation();
                            this.focus.focus(window, cx);
                            let rows = this.state.read(cx).rows();
                            let index = this.cursor_at(&rows, cx).unwrap_or(0);
                            this.put_cursor(&rows, index, cx);
                        }
                    }))
                    .child(
                        Input::new(&self.search)
                            .small()
                            .w_full()
                            .cleanable(true)
                            .prefix(Icon::new(IconName::Search).text_color(muted)),
                    ),
            )
            .child(
                Button::new("tree-new")
                    .ghost()
                    .small()
                    // A viewer's only choice is exporting.
                    .icon(if editable { IconName::Plus } else { IconName::Ellipsis })
                    .tooltip(
                        if editable {
                            t!("workspace.new")
                        } else {
                            t!("export.project")
                        }
                        .to_string(),
                    )
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                        let (for_api, for_folder) = (state.clone(), state.clone());
                        let (for_import, for_export) = (state.clone(), state.clone());
                        let for_paste = state.clone();
                        let menu = if editable {
                            menu.item(
                                PopupMenuItem::new(t!("workspace.new_api").to_string())
                                    .icon(Icons::FilePlus)
                                    // Its shortcut beside it, where it reaches the app;
                                    // the click is the handler's.
                                    .when(this_build::takes_new_keys(), |item| item.action(Box::new(NewApi)))
                                    .on_click(move |_, window, cx| new_item(&for_api, None, ItemKind::Api, window, cx)),
                            )
                            .item(
                                PopupMenuItem::new(t!("workspace.new_folder").to_string())
                                    .icon(Icons::FolderPlus)
                                    .when(this_build::takes_new_keys(), |item| item.action(Box::new(NewFolder)))
                                    .on_click(move |_, window, cx| {
                                        new_item(&for_folder, None, ItemKind::Folder, window, cx)
                                    }),
                            )
                            .separator()
                            // F10.5: the clipboard is read on this click.
                            .item(
                                PopupMenuItem::new(t!("import.paste_curl").to_string())
                                    .icon(Icons::ClipboardPaste)
                                    .action(Box::new(PasteCurl))
                                    .on_click(move |_, window, cx| paste_curl(&for_paste, None, window, cx)),
                            )
                            // F10.1.
                            .item(
                                PopupMenuItem::new(t!("import.title").to_string())
                                    .icon(Icons::Import)
                                    .on_click(move |_, window, cx| {
                                        open_import_dialog(for_import.clone(), None, window, cx)
                                    }),
                            )
                        } else {
                            menu
                        };
                        // F11.1: a viewer may export too.
                        menu.item(
                            PopupMenuItem::new(t!("export.project").to_string())
                                .icon(Icons::Download)
                                .on_click(move |_, window, cx| export(&for_export, None, window, cx)),
                        )
                    }),
            )
            .child(
                Button::new("tree-collapse")
                    .ghost()
                    .small()
                    .icon(IconName::ChevronsUpDown)
                    .tooltip(t!("workspace.collapse_all").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |state, cx| state.collapse_all(cx));
                    })),
            )
    }

    fn render_row(&self, row: &Row, cursor: bool, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let Some(node) = state.tree().get(&row.id) else {
            return div().into_any_element();
        };
        let editable = state.editable();
        let theme = cx.theme();
        let selected = state.selected() == Some(&row.id);
        let pinned = state.is_pinned(&row.id);
        // What a folder holds, folded or not.
        let count = (row.kind == ItemKind::Folder)
            .then(|| state.tree().api_count(&row.id))
            .filter(|count| *count > 0);
        let (id, name, kind) = (row.id.clone(), node.name().to_string(), row.kind);
        let key = SharedString::from(format!("tree-row-{id}"));

        let icon: AnyElement = match kind {
            ItemKind::Folder => h_flex()
                .gap_1()
                .items_center()
                .child(
                    Icon::new(if row.open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .xsmall()
                    .text_color(theme.muted_foreground),
                )
                .child(
                    Icon::new(if row.open {
                        IconName::FolderOpen
                    } else {
                        IconName::FolderClosed
                    })
                    .small()
                    .text_color(if row.open {
                        theme.foreground
                    } else {
                        theme.muted_foreground
                    }),
                )
                .into_any_element(),
            // The method as coloured text, short enough for one width: a
            // filled badge on every row outweighs the names beside it.
            ItemKind::Api => div()
                .w(px(34.))
                .flex_none()
                .text_size(px(10.5))
                .font_bold()
                .text_color(method_color(node.method(), cx))
                .child(method_label(node.method()))
                .into_any_element(),
        };
        let label: AnyElement = match &self.renaming {
            Some(renaming) if renaming.id == id => div()
                .flex_1()
                // Escape belongs to the rename, before the input sees it.
                .capture_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.cancel_rename(cx);
                    }
                }))
                .child(Input::new(&renaming.input).xsmall())
                .into_any_element(),
            _ => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .when(selected, |this| this.font_medium())
                .child(marked(&name, state.keyword(), cx))
                .into_any_element(),
        };

        let view = cx.entity().downgrade();
        let menu_state = self.state.downgrade();
        let (menu_id, menu_name) = (id.clone(), name.clone());
        let renaming = self.renaming.as_ref().is_some_and(|renaming| renaming.id == id);
        let dragged = DraggedItem {
            id: id.clone(),
            name: name.clone().into(),
        };
        let drop = match &self.drop_target {
            Some(DropTarget::Row { id: target, position }) if *target == id => Some(*position),
            _ => None,
        };
        // A line between rows, indented to the level the item would join.
        let guide = theme.foreground.opacity(0.1);
        let pin_group = SharedString::from(format!("tree-pin-{id}"));
        let line_depth = match drop {
            Some(DropPosition::Below) if kind == ItemKind::Folder && row.open => row.depth + 1,
            _ => row.depth,
        };
        h_flex()
            .id(key)
            .group(pin_group.clone())
            .relative()
            .h(px(ROW_HEIGHT))
            .pl(px(8. + INDENT * row.depth as f32))
            .pr_2()
            .gap_1p5()
            .items_center()
            .rounded(theme.radius)
            .text_sm()
            .cursor_pointer()
            .when(selected, |this| this.bg(theme.list_active))
            .when(!selected, |this| this.hover(|style| style.bg(theme.list_hover)))
            .when(drop == Some(DropPosition::Over), |this| this.bg(theme.drop_target))
            .when_some(
                drop.filter(|position| *position != DropPosition::Over),
                |this, position| {
                    let line = div()
                        .absolute()
                        .left(px(8. + INDENT * line_depth as f32))
                        .right_0()
                        .h(px(2.))
                        .bg(theme.primary);
                    this.child(match position {
                        DropPosition::Above => line.top_0(),
                        _ => line.bottom_0(),
                    })
                },
            )
            .when(!renaming && editable, |this| {
                this.on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
            })
            // A line down each level the row is inside, under its folder's
            // chevron: where a folder's contents end shows at a glance.
            .children((0..row.depth).map(|level| {
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(8. + INDENT * level as f32 + 5.5))
                    .w(px(1.))
                    .bg(guide)
            }))
            // Where the keys are, while they reach the tree.
            .when(cursor, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.ring),
                )
            })
            .child(icon)
            .child(label)
            .when_some(count, |this, count| {
                this.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(count.to_string()),
                )
            })
            .when(kind == ItemKind::Api, |this| {
                let pin_id = id.clone();
                let tip = if pinned {
                    t!("workspace.unpin")
                } else {
                    t!("workspace.pin")
                }
                .to_string();
                let (icon, color) = if pinned {
                    (IconName::StarFill, theme.warning)
                } else {
                    (IconName::Star, theme.muted_foreground)
                };
                // Faint until the row is hovered, so a long list is not a column
                // of stars. Pinned ones stay lit. The click is the star's,
                // not the row's.
                this.child(
                    div()
                        .flex_none()
                        .when(!pinned, |star| {
                            star.opacity(0.45)
                                .group_hover(pin_group.clone(), |style| style.opacity(1.))
                        })
                        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .child(
                            Button::new(SharedString::from(format!("tree-pin-{pin_id}")))
                                .ghost()
                                .xsmall()
                                .compact()
                                .tooltip(tip.clone())
                                .accessibility_label(tip)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.state.update(cx, |state, cx| {
                                        let pinned = state.is_pinned(&pin_id);
                                        state.set_pinned(&pin_id, !pinned, cx);
                                    });
                                }))
                                .child(Icon::new(icon).xsmall().text_color(color)),
                        ),
                )
            })
            .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                // A click hands the keys to the tree, at this row.
                this.focus.focus(window, cx);
                this.cursor = Some(id.clone());
                this.click(&id, kind, event.click_count(), editable, &name, window, cx);
            }))
            .context_menu(move |menu, _, cx| {
                item_menu(menu, &view, &menu_state, &menu_id, &menu_name, kind, editable, cx)
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn click(
        &mut self,
        id: &ItemId,
        kind: ItemKind,
        clicks: usize,
        editable: bool,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match kind {
            ItemKind::Folder => {
                self.state.update(cx, |state, cx| state.toggle_folder(id, cx));
            }
            ItemKind::Api if clicks >= 2 && editable => {
                self.start_rename(id.clone(), name.to_string(), window, cx);
            }
            ItemKind::Api => self.state.update(cx, |state, cx| state.select(id, cx)),
        }
    }

    fn render_body(&mut self, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let load = state.load().clone();
        let empty = state.tree().is_empty();
        let keyword = state.keyword().trim().to_string();
        let rows = state.rows();
        let muted = cx.theme().muted_foreground;
        match load {
            Load::Loading if empty => h_flex()
                .flex_1()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(muted)
                .child(Spinner::new())
                .child(t!("workspace.loading").to_string())
                .into_any_element(),
            Load::Failed(error) if empty => notice(
                IconName::CircleX,
                t!("workspace.load_failed").to_string(),
                crate::i18n::store_error(&error),
                vec![
                    Button::new("tree-retry")
                        .outline()
                        .small()
                        .label(t!("common.retry").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.state.update(cx, |state, cx| state.reload(cx)))),
                ],
            ),
            _ if empty => {
                let (state, for_import) = (self.state.downgrade(), self.state.downgrade());
                // A viewer's empty project has nothing to offer.
                let actions = self.state.read(cx).editable().then(|| {
                    vec![
                        Button::new("tree-empty-new")
                            .primary()
                            .small()
                            .icon(IconName::Plus)
                            .label(t!("workspace.new_api").to_string())
                            .on_click(move |_, window, cx| new_item(&state, None, ItemKind::Api, window, cx)),
                        // F10.1: a cURL command, a Postman or OpenAPI file…
                        Button::new("tree-empty-import")
                            .outline()
                            .small()
                            .icon(Icons::Import)
                            .label(t!("import.title").to_string())
                            .on_click(move |_, window, cx| open_import_dialog(for_import.clone(), None, window, cx)),
                    ]
                });
                notice(
                    IconName::Inbox,
                    t!("workspace.empty_title").to_string(),
                    t!("workspace.empty_description").to_string(),
                    actions.unwrap_or_default(),
                )
            }
            _ if rows.is_empty() => notice(
                IconName::Search,
                t!("workspace.no_match", keyword = keyword).to_string(),
                String::new(),
                Vec::new(),
            ),
            _ => {
                let cursor = focused.then(|| self.cursor_at(&rows, cx)).flatten();
                let elements: Vec<AnyElement> = rows
                    .iter()
                    .enumerate()
                    .map(|(index, row)| self.render_row(row, cursor == Some(index), cx))
                    .collect();
                let end = self.drop_target == Some(DropTarget::End);
                let primary = cx.theme().primary;
                self.drawn = rows;
                let list = div()
                    .id("tree-rows")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px_1()
                    .pb_2()
                    .children(elements)
                    .when(end, |this| this.child(div().mx_2().h(px(2.)).bg(primary)))
                    .on_drag_move(
                        cx.listener(|this, event: &DragMoveEvent<DraggedItem>, _, cx| this.on_drag_move(event, cx)),
                    )
                    .on_drop(cx.listener(|this, dragged: &DraggedItem, _, cx| this.on_drop(dragged, cx)));
                // The list keeps its own handle (a drop is placed by its
                // offset), so its scrollbar is laid over it rather than the
                // kit's scroll wrapper, which brings a handle of its own.
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(list)
                    .child(div().absolute().inset_0().child(Scrollbar::vertical(&self.scroll)))
                    .into_any_element()
            }
        }
    }
}

/// An item's menu (F3.9 / F3.10). A viewer only pins and exports.
#[allow(clippy::too_many_arguments)]
fn item_menu(
    menu: PopupMenu,
    view: &WeakEntity<TreePanel>,
    state: &WeakEntity<WorkspaceState>,
    id: &ItemId,
    name: &str,
    kind: ItemKind,
    editable: bool,
    cx: &App,
) -> PopupMenu {
    let pinned = state.upgrade().is_some_and(|state| state.read(cx).is_pinned(id));
    let menu = match kind {
        ItemKind::Folder => {
            let (for_api, for_folder, parent) = (state.clone(), state.clone(), id.clone());
            let folder = parent.clone();
            let (for_import, into) = (state.clone(), id.clone());
            let (for_paste, paste_into) = (state.clone(), id.clone());
            let (for_export, exported) = (state.clone(), id.clone());
            let menu = if editable {
                menu.item(
                    PopupMenuItem::new(t!("workspace.new_api").to_string())
                        .icon(Icons::FilePlus)
                        .on_click(move |_, window, cx| {
                            new_item(&for_api, Some(parent.clone()), ItemKind::Api, window, cx)
                        }),
                )
                .item(
                    PopupMenuItem::new(t!("workspace.new_folder").to_string())
                        .icon(Icons::FolderPlus)
                        .on_click(move |_, window, cx| {
                            new_item(&for_folder, Some(folder.clone()), ItemKind::Folder, window, cx)
                        }),
                )
                .item(
                    PopupMenuItem::new(t!("import.paste_curl_into").to_string())
                        .icon(Icons::ClipboardPaste)
                        .on_click(move |_, window, cx| paste_curl(&for_paste, Some(paste_into.clone()), window, cx)),
                )
                .item(
                    PopupMenuItem::new(t!("import.into_folder").to_string())
                        .icon(Icons::Import)
                        .on_click(move |_, window, cx| {
                            open_import_dialog(for_import.clone(), Some(into.clone()), window, cx)
                        }),
                )
            } else {
                menu
            };
            menu.item(
                PopupMenuItem::new(t!("export.folder").to_string())
                    .icon(Icons::Download)
                    .on_click(move |_, window, cx| export(&for_export, Some(exported.clone()), window, cx)),
            )
        }
        ItemKind::Api => {
            let (state, id) = (state.clone(), id.clone());
            let (config_state, config_id) = (state.clone(), id.clone());
            let (curl_state, curl_id) = (state.clone(), id.clone());
            let (label, pin_icon) = if pinned {
                (t!("workspace.unpin"), Icons::StarOff)
            } else {
                (t!("workspace.pin"), Icons::Star)
            };
            // F3.9: what a viewer may do too.
            menu.item(
                PopupMenuItem::new(t!("workspace.copy_config").to_string())
                    .icon(Icons::Copy)
                    .on_click(move |_, window, cx| {
                        let config = config_state
                            .upgrade()
                            .and_then(|state| state.read(cx).api_config(&config_id));
                        if let Some(config) = config {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(config));
                            let copied = t!("workspace.config_copied").to_string();
                            window.push_notification(Notification::success(copied), cx);
                        }
                    }),
            )
            .item(
                PopupMenuItem::new(t!("response.copy_curl").to_string())
                    .icon(Icons::Terminal)
                    .on_click(move |_, _, cx| {
                        let _ = curl_state.update(cx, |state, cx| state.copy_curl(&curl_id, cx));
                    }),
            )
            .item(
                PopupMenuItem::new(label.to_string())
                    .icon(pin_icon)
                    .on_click(move |_, _, cx| {
                        let _ = state.update(cx, |state, cx| state.set_pinned(&id, !pinned, cx));
                    }),
            )
        }
    };
    if !editable {
        return menu;
    }
    let (rename_view, rename_id, rename_name) = (view.clone(), id.clone(), name.to_string());
    let (delete_state, delete_id, delete_name) = (state.clone(), id.clone(), name.to_string());
    let (move_state, move_id, move_name) = (state.clone(), id.clone(), name.to_string());
    let menu = menu.separator().item(
        PopupMenuItem::new(t!("workspace.move_to").to_string())
            .icon(Icons::FolderInput)
            .on_click(move |_, window, cx| {
                open_move_dialog(move_state.clone(), move_id.clone(), move_name.clone(), window, cx)
            }),
    );
    // An API's copy, to change beside it.
    let menu = match kind {
        ItemKind::Api => {
            let (copy_state, copy_id) = (state.clone(), id.clone());
            let copy_name = t!("workspace.copy_name", name = name).to_string();
            menu.item(
                PopupMenuItem::new(t!("workspace.duplicate").to_string())
                    .icon(Icons::CopyPlus)
                    .on_click(move |_, _, cx| {
                        let _ = copy_state.update(cx, |state, cx| state.duplicate(&copy_id, copy_name.clone(), cx));
                    }),
            )
        }
        ItemKind::Folder => menu,
    };
    menu.item(
        PopupMenuItem::new(t!("workspace.rename").to_string())
            .icon(Icons::Pencil)
            .action(Box::new(Rename))
            .on_click(move |_, window, cx| {
                let _ = rename_view.update(cx, |this, cx| {
                    this.start_rename(rename_id.clone(), rename_name.clone(), window, cx)
                });
            }),
    )
    .item(
        PopupMenuItem::new(t!("common.delete").to_string())
            .icon(Icons::Trash)
            .on_click(move |_, window, cx| {
                confirm_delete(&delete_state, delete_id.clone(), &delete_name, kind, window, cx)
            }),
    )
}

/// A centred state of the whole tree: empty, failed, nothing found.
fn notice(icon: IconName, title: String, description: String, actions: Vec<Button>) -> AnyElement {
    v_flex()
        .flex_1()
        .justify_center()
        .items_center()
        .p_4()
        .child(
            Empty::new()
                .header(
                    EmptyHeader::new()
                        .media(
                            EmptyMedia::new()
                                .with_variant(EmptyMediaVariant::Icon)
                                .child(Icon::new(icon)),
                        )
                        .title(EmptyTitle::new().child(title))
                        .when(!description.is_empty(), |header| {
                            header.description(EmptyDescription::new().child(description))
                        }),
                )
                .when(!actions.is_empty(), |empty| {
                    empty.content(EmptyContent::new().child(h_flex().gap_2().children(actions)))
                }),
        )
        .into_any_element()
}

impl Render for TreePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A drag let go outside the tree leaves no target behind.
        if !cx.has_active_drag() {
            self.drop_target = None;
        }
        let focused = self.focus.is_focused(window);
        v_flex()
            .size_full()
            .bg(cx.theme().muted)
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Previous, window, cx| this.step(false, window, cx)))
            .on_action(cx.listener(|this, _: &Next, window, cx| this.step(true, window, cx)))
            .on_action(cx.listener(|this, _: &Close, window, cx| this.close_or_open(false, window, cx)))
            .on_action(cx.listener(|this, _: &Open, window, cx| this.close_or_open(true, window, cx)))
            .on_action(cx.listener(|this, _: &Toggle, window, cx| this.toggle(window, cx)))
            .on_action(cx.listener(|this, _: &Rename, window, cx| this.rename_at_cursor(window, cx)))
            .on_action(cx.listener(|this, _: &NewApi, window, cx| this.new_here(ItemKind::Api, window, cx)))
            .on_action(cx.listener(|this, _: &PasteCurl, window, cx| this.paste_here(window, cx)))
            .on_action(cx.listener(|this, _: &NewFolder, window, cx| this.new_here(ItemKind::Folder, window, cx)))
            .child(self.render_header(cx))
            .child(self.render_body(focused, cx))
    }
}
