//! The project's tree (M3): search, open and close, select, create, rename,
//! move by dragging, delete. The rows and every rule behind them are the
//! state's and core's; this draws the rows and turns pointer and keyboard
//! input into intent.

use super::{confirm_delete, export, method_color, new_item, open_import_dialog};
use crate::states::{Load, WorkspaceState};
use cyberapi_core::ItemId;
use cyberapi_core::tree::{DropPosition, ItemKind, Placement, Row};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::empty::{
    Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Bounds, Context, DragMoveEvent, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Point, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div, px,
};
use rust_i18n::t;

/// How far each level of the tree is indented.
const INDENT: f32 = 14.;
const ROW_HEIGHT: f32 = 28.;

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
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&search, |this, search, event, cx| {
                if matches!(event, InputEvent::Change) {
                    let keyword = search.read(cx).value().to_string();
                    this.state.update(cx, |state, cx| state.set_keyword(keyword, cx));
                }
            }),
        ];
        Self {
            state,
            search,
            renaming: None,
            scroll: ScrollHandle::new(),
            drawn: Vec::new(),
            drop_target: None,
            _subscriptions: subscriptions,
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
        h_flex()
            .gap_1()
            .p_2()
            .child(
                Input::new(&self.search)
                    .small()
                    .cleanable(true)
                    .prefix(Icon::new(IconName::Search).text_color(muted)),
            )
            .child(
                Button::new("tree-new")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip(t!("workspace.new").to_string())
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                        let (for_api, for_folder) = (state.clone(), state.clone());
                        let (for_import, for_export) = (state.clone(), state.clone());
                        menu.item(
                            PopupMenuItem::new(t!("workspace.new_api").to_string())
                                .on_click(move |_, window, cx| new_item(&for_api, None, ItemKind::Api, window, cx)),
                        )
                        .item(
                            PopupMenuItem::new(t!("workspace.new_folder").to_string()).on_click(
                                move |_, window, cx| new_item(&for_folder, None, ItemKind::Folder, window, cx),
                            ),
                        )
                        .separator()
                        // F10.1 / F11.1.
                        .item(
                            PopupMenuItem::new(t!("import.title").to_string()).on_click(move |_, window, cx| {
                                open_import_dialog(for_import.clone(), None, window, cx)
                            }),
                        )
                        .item(
                            PopupMenuItem::new(t!("export.project").to_string())
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

    fn render_row(&self, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let Some(node) = state.tree().get(&row.id) else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let selected = state.selected() == Some(&row.id);
        let pinned = state.is_pinned(&row.id);
        let (id, name, kind) = (row.id.clone(), node.name().to_string(), row.kind);
        let key = SharedString::from(format!("tree-row-{id}"));

        let icon: AnyElement = match kind {
            ItemKind::Folder => Icon::new(if row.open {
                IconName::FolderOpen
            } else {
                IconName::FolderClosed
            })
            .small()
            .text_color(theme.muted_foreground)
            .into_any_element(),
            ItemKind::Api => div()
                .w(px(46.))
                .flex_none()
                .text_xs()
                .font_semibold()
                .text_color(method_color(node.method(), cx))
                .child(node.method().as_str())
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
                .child(name.clone())
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
        let line_depth = match drop {
            Some(DropPosition::Below) if kind == ItemKind::Folder && row.open => row.depth + 1,
            _ => row.depth,
        };
        h_flex()
            .id(key)
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
            .when(!renaming, |this| {
                this.on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
            })
            .child(icon)
            .child(label)
            .when(pinned, |this| {
                this.child(
                    Icon::new(IconName::StarFill)
                        .xsmall()
                        .text_color(theme.muted_foreground),
                )
            })
            .on_click(
                cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| match kind {
                    ItemKind::Folder => {
                        this.state.update(cx, |state, cx| state.toggle_folder(&id, cx));
                    }
                    ItemKind::Api if event.click_count() >= 2 => {
                        this.start_rename(id.clone(), name.clone(), window, cx);
                    }
                    ItemKind::Api => this.state.update(cx, |state, cx| state.select(&id, cx)),
                }),
            )
            .context_menu(move |menu, _, cx| item_menu(menu, &view, &menu_state, &menu_id, &menu_name, kind, cx))
            .into_any_element()
    }

    fn render_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
                Some(
                    Button::new("tree-retry")
                        .outline()
                        .small()
                        .label(t!("common.retry").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.state.update(cx, |state, cx| state.reload(cx)))),
                ),
            ),
            _ if empty => {
                let state = self.state.downgrade();
                notice(
                    IconName::Inbox,
                    t!("workspace.empty_title").to_string(),
                    t!("workspace.empty_description").to_string(),
                    Some(
                        Button::new("tree-empty-new")
                            .primary()
                            .small()
                            .icon(IconName::Plus)
                            .label(t!("workspace.new_api").to_string())
                            .on_click(move |_, window, cx| new_item(&state, None, ItemKind::Api, window, cx)),
                    ),
                )
            }
            _ if rows.is_empty() => notice(
                IconName::Search,
                t!("workspace.no_match", keyword = keyword).to_string(),
                String::new(),
                None,
            ),
            _ => {
                let elements: Vec<AnyElement> = rows.iter().map(|row| self.render_row(row, cx)).collect();
                let end = self.drop_target == Some(DropTarget::End);
                let primary = cx.theme().primary;
                self.drawn = rows;
                div()
                    .id("tree-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px_1()
                    .pb_2()
                    .children(elements)
                    .when(end, |this| this.child(div().mx_2().h(px(2.)).bg(primary)))
                    .on_drag_move(
                        cx.listener(|this, event: &DragMoveEvent<DraggedItem>, _, cx| this.on_drag_move(event, cx)),
                    )
                    .on_drop(cx.listener(|this, dragged: &DraggedItem, _, cx| this.on_drop(dragged, cx)))
                    .into_any_element()
            }
        }
    }
}

/// An item's menu (F3.9 / F3.10).
fn item_menu(
    menu: PopupMenu,
    view: &WeakEntity<TreePanel>,
    state: &WeakEntity<WorkspaceState>,
    id: &ItemId,
    name: &str,
    kind: ItemKind,
    cx: &App,
) -> PopupMenu {
    let pinned = state.upgrade().is_some_and(|state| state.read(cx).is_pinned(id));
    let menu = match kind {
        ItemKind::Folder => {
            let (for_api, for_folder, parent) = (state.clone(), state.clone(), id.clone());
            let folder = parent.clone();
            let (for_import, into) = (state.clone(), id.clone());
            let (for_export, exported) = (state.clone(), id.clone());
            menu.item(
                PopupMenuItem::new(t!("workspace.new_api").to_string())
                    .on_click(move |_, window, cx| new_item(&for_api, Some(parent.clone()), ItemKind::Api, window, cx)),
            )
            .item(
                PopupMenuItem::new(t!("workspace.new_folder").to_string()).on_click(move |_, window, cx| {
                    new_item(&for_folder, Some(folder.clone()), ItemKind::Folder, window, cx)
                }),
            )
            .item(
                PopupMenuItem::new(t!("import.into_folder").to_string()).on_click(move |_, window, cx| {
                    open_import_dialog(for_import.clone(), Some(into.clone()), window, cx)
                }),
            )
            .item(
                PopupMenuItem::new(t!("export.folder").to_string())
                    .on_click(move |_, window, cx| export(&for_export, Some(exported.clone()), window, cx)),
            )
            .separator()
        }
        ItemKind::Api => {
            let (state, id) = (state.clone(), id.clone());
            let label = if pinned {
                t!("workspace.unpin")
            } else {
                t!("workspace.pin")
            };
            menu.item(PopupMenuItem::new(label.to_string()).on_click(move |_, _, cx| {
                let _ = state.update(cx, |state, cx| state.set_pinned(&id, !pinned, cx));
            }))
            .separator()
        }
    };
    let (rename_view, rename_id, rename_name) = (view.clone(), id.clone(), name.to_string());
    let (delete_state, delete_id, delete_name) = (state.clone(), id.clone(), name.to_string());
    menu.item(
        PopupMenuItem::new(t!("workspace.rename").to_string()).on_click(move |_, window, cx| {
            let _ = rename_view.update(cx, |this, cx| {
                this.start_rename(rename_id.clone(), rename_name.clone(), window, cx)
            });
        }),
    )
    .item(
        PopupMenuItem::new(t!("common.delete").to_string()).on_click(move |_, window, cx| {
            confirm_delete(&delete_state, delete_id.clone(), &delete_name, kind, window, cx)
        }),
    )
}

/// A centred state of the whole tree: empty, failed, nothing found.
fn notice(icon: IconName, title: String, description: String, action: Option<Button>) -> AnyElement {
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
                .when_some(action, |empty, action| empty.content(EmptyContent::new().child(action))),
        )
        .into_any_element()
}

impl Render for TreePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A drag let go outside the tree leaves no target behind.
        if !cx.has_active_drag() {
            self.drop_target = None;
        }
        v_flex()
            .size_full()
            .child(self.render_header(cx))
            .child(self.render_body(cx))
    }
}
