//! A project's workspace: the tree, the selected API's request editor and
//! its response side by side in resizable columns (F14.5), and the pins the
//! title bar shows.

mod pins;
mod request_editor;
mod response_panel;
mod tree_panel;

use crate::i18n;
use crate::states::{ApiState, AppStore, WorkspaceEvent, WorkspaceState};
use crate::views::name_dialog::open_name_dialog;
use cyberapi_core::model::{Collection, HttpMethod};
use cyberapi_core::prefs::ColumnWidths;
use cyberapi_core::tree::ItemKind;
use cyberapi_core::{CollectionId, ItemId};
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EntityId, FocusHandle, Focusable, Hsla, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement as _, Render, Styled as _, Subscription, WeakEntity, Window, px,
};
pub use pins::PinsBar;
use request_editor::RequestEditor;
use response_panel::ResponsePanel;
use rust_i18n::t;
use tree_panel::TreePanel;

gpui_kit::actions!(workspace, [NewApi, NewFolder, Send]);

const KEY_CONTEXT: &str = "Workspace";

/// F3.12, for both platforms: `secondary` is Cmd on macOS and Ctrl elsewhere
/// (#22 — the old app listened for Cmd alone).
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewApi, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-shift-n", NewFolder, Some(KEY_CONTEXT)),
        // F5.1 from the keyboard, wherever focus is in the workspace — in
        // a field too, ahead of the field's own Cmd/Ctrl-Enter (bound later,
        // so it wins over the kit's).
        KeyBinding::new("secondary-enter", Send, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-enter", Send, Some("Workspace > Input")),
    ]);
}

/// The color of a method's label (§6): DELETE red, the methods with a body
/// green, the rest blue.
pub fn method_color(method: HttpMethod, cx: &App) -> Hsla {
    let theme = cx.theme();
    match method {
        HttpMethod::Delete => theme.danger,
        method if method.has_body() => theme.success,
        _ => theme.info,
    }
}

/// Asks for a name and creates the item in `parent` (F3.4).
fn new_item(
    state: &WeakEntity<WorkspaceState>,
    parent: Option<ItemId>,
    kind: ItemKind,
    window: &mut Window,
    cx: &mut App,
) {
    let state = state.clone();
    let (title, placeholder) = match kind {
        ItemKind::Api => (t!("workspace.new_api"), t!("workspace.api_name_placeholder")),
        ItemKind::Folder => (t!("workspace.new_folder"), t!("workspace.folder_name_placeholder")),
    };
    open_name_dialog(
        window,
        cx,
        title.to_string().into(),
        placeholder.to_string().into(),
        t!("common.create").to_string().into(),
        move |name, cx| {
            let parent = parent.clone();
            let _ = state.update(cx, |state, cx| match kind {
                ItemKind::Api => state.create_api(parent, name, cx),
                ItemKind::Folder => state.create_folder(parent, name, cx),
            });
        },
    );
}

/// F3.6: name the item, and what goes with a folder, before deleting.
fn confirm_delete(
    state: &WeakEntity<WorkspaceState>,
    id: ItemId,
    name: &str,
    kind: ItemKind,
    window: &mut Window,
    cx: &mut App,
) {
    let state = state.clone();
    let (title, description) = match kind {
        ItemKind::Folder => (
            t!("workspace.delete_folder_title", name = name),
            t!("workspace.delete_folder_description"),
        ),
        ItemKind::Api => (
            t!("workspace.delete_api_title", name = name),
            t!("workspace.delete_api_description"),
        ),
    };
    let (title, description) = (title.to_string(), description.to_string());
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (state, id) = (state.clone(), id.clone());
        alert
            .title(title.clone())
            .description(description.clone())
            .confirm()
            .ok_text(t!("common.delete").to_string())
            .ok_variant(ButtonVariant::Danger)
            .cancel_text(t!("common.cancel").to_string())
            .on_ok(move |_, _, cx| {
                let _ = state.update(cx, |state, cx| state.delete(id.clone(), cx));
                true
            })
    });
}

/// The selected API's two columns, made for its `ApiState`.
struct Selected {
    api: EntityId,
    editor: Entity<RequestEditor>,
    response: Entity<ResponsePanel>,
}

pub struct WorkspaceView {
    id: CollectionId,
    state: Entity<WorkspaceState>,
    tree: Entity<TreePanel>,
    pins: Entity<PinsBar>,
    selected: Option<Selected>,
    columns: Entity<ResizableState>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl WorkspaceView {
    pub fn new(collection: Collection, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let id = collection.id.clone();
        let sender = AppStore::global(cx).sender(cx);
        let state = cx.new(|cx| WorkspaceState::new(sender, collection, cx));
        let tree = cx.new(|cx| TreePanel::new(state.clone(), window, cx));
        let pins = cx.new(|cx| PinsBar::new(state.clone(), cx));
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| {
                this.follow_selection(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(&state, window, |_, _, event, window, cx| match event {
                WorkspaceEvent::Failed(error) => {
                    window.push_notification(Notification::error(i18n::store_error(error)), cx);
                }
            }),
        ];
        // The shortcuts reach the workspace from wherever focus is inside it.
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let columns = cx.new(|_| ResizableState::default());
        let mut view = Self {
            id,
            state,
            tree,
            pins,
            selected: None,
            columns,
            focus,
            _subscriptions: subscriptions,
        };
        view.follow_selection(window, cx);
        view
    }

    /// Makes the editor and response columns of the selected API's state, or
    /// drops them when nothing is selected.
    fn follow_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let api: Option<Entity<ApiState>> = self.state.read(cx).api().cloned();
        match api {
            Some(api) if self.selected.as_ref().map(|s| s.api) != Some(api.entity_id()) => {
                let state = self.state.clone();
                let editor = cx.new(|cx| RequestEditor::new(state, api.clone(), window, cx));
                let response = cx.new(|cx| ResponsePanel::new(api.clone(), window, cx));
                self.selected = Some(Selected {
                    api: api.entity_id(),
                    editor,
                    response,
                });
            }
            Some(_) => {}
            None => self.selected = None,
        }
    }

    /// F14.5: the widths kept — the tree in pixels, the editor as its share
    /// of the rest.
    fn columns_resized(&mut self, columns: &Entity<ResizableState>, cx: &mut Context<Self>) {
        let sizes = columns.read(cx).sizes().clone();
        let [tree, editor, response] = sizes[..] else {
            return;
        };
        let rest = f32::from(editor) + f32::from(response);
        if rest <= 0. {
            return;
        }
        let widths = ColumnWidths {
            tree: f32::from(tree),
            editor_ratio: (f32::from(editor) / rest).clamp(0.2, 0.8),
        };
        let app = AppStore::global(cx).state().clone();
        app.update(cx, |app, cx| app.set_columns(widths, cx));
    }

    pub fn collection_id(&self) -> &CollectionId {
        &self.id
    }

    /// F12.2: the pins, for the title bar.
    pub fn pins(&self) -> &Entity<PinsBar> {
        &self.pins
    }

    /// The workspace's state, for the title bar's menu.
    pub fn state(&self) -> &Entity<WorkspaceState> {
        &self.state
    }

    fn render_nothing_selected(&self) -> impl IntoElement {
        v_flex().size_full().justify_center().items_center().child(
            Empty::new().header(
                EmptyHeader::new()
                    .media(
                        EmptyMedia::new()
                            .with_variant(EmptyMediaVariant::Icon)
                            .child(Icon::new(IconName::FileText)),
                    )
                    .title(EmptyTitle::new().child(t!("workspace.nothing_selected").to_string()))
                    .description(EmptyDescription::new().child(t!("workspace.select_hint").to_string())),
            ),
        )
    }
}

impl Focusable for WorkspaceView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let columns = AppStore::global(cx).state().read(cx).prefs().columns;
        let state = self.state.downgrade();
        let folder_state = state.clone();
        // The editor's first width: its share of what the tree leaves.
        let rest = (f32::from(window.viewport_size().width) - columns.tree).max(400.);
        let editor_width = rest * columns.editor_ratio.clamp(0.2, 0.8);
        let group = h_resizable("workspace-columns").with_state(&self.columns).child(
            resizable_panel()
                .size(px(columns.tree))
                .size_range(px(180.)..px(640.))
                .child(self.tree.clone()),
        );
        let group = match &self.selected {
            Some(selected) => group
                .child(
                    resizable_panel()
                        .size(px(editor_width))
                        .size_range(px(280.)..px(4000.))
                        .child(selected.editor.clone()),
                )
                .child(
                    resizable_panel()
                        .size_range(px(240.)..px(4000.))
                        .child(selected.response.clone()),
                ),
            None => group.child(resizable_panel().child(self.render_nothing_selected())),
        };
        let editor = self.selected.as_ref().map(|selected| selected.editor.downgrade());
        h_flex()
            .size_full()
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            .on_action(move |_: &NewApi, window, cx| new_item(&state, None, ItemKind::Api, window, cx))
            .on_action(move |_: &NewFolder, window, cx| new_item(&folder_state, None, ItemKind::Folder, window, cx))
            .on_action(move |_: &Send, window, cx| {
                if let Some(editor) = &editor {
                    let _ = editor.update(cx, |editor, cx| editor.send(window, cx));
                }
            })
            .child(group.on_resize(
                cx.listener(|this, columns: &Entity<ResizableState>, _, cx| this.columns_resized(columns, cx)),
            ))
    }
}
