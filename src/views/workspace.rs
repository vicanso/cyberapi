//! A project's workspace: the tree, the selected API's request editor and
//! its response side by side in resizable columns (F14.5), and the pins the
//! title bar shows.

mod import_dialog;
mod pins;
mod request_editor;
mod response_panel;
mod tree_panel;

use crate::APP_NAME;
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
use request_editor::{RequestEditor, RequestEditorEvent};
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

/// A page in a Mac's browser: `secondary` is Ctrl in any page, and the
/// kit gives its fields their Cmd shortcuts only in the macOS build, so a
/// Mac reader's Cmd-A, Cmd-Z, Cmd-Enter… did nothing. These are the kit's
/// macOS bindings, and Send's, beside the Ctrl ones the page keeps.
pub fn bind_mac_page_keys(cx: &mut App) {
    use gpui_kit::component::input::{
        Copy, Cut, DeleteToBeginningOfLine, DeleteToEndOfLine, DeleteToNextWordEnd, DeleteToPreviousWordStart, MoveEnd,
        MoveHome, MoveToEnd, MoveToNextWord, MoveToPreviousWord, MoveToStart, Paste, Redo, SelectAll, SelectToEnd,
        SelectToEndOfLine, SelectToNextWordEnd, SelectToPreviousWordStart, SelectToStart, SelectToStartOfLine, Undo,
    };
    const INPUT: Option<&str> = Some("Input");
    cx.bind_keys([
        KeyBinding::new("cmd-a", SelectAll, INPUT),
        KeyBinding::new("cmd-c", Copy, INPUT),
        KeyBinding::new("cmd-x", Cut, INPUT),
        KeyBinding::new("cmd-v", Paste, INPUT),
        KeyBinding::new("cmd-z", Undo, INPUT),
        KeyBinding::new("cmd-shift-z", Redo, INPUT),
        KeyBinding::new("cmd-left", MoveHome, INPUT),
        KeyBinding::new("cmd-right", MoveEnd, INPUT),
        KeyBinding::new("cmd-up", MoveToStart, INPUT),
        KeyBinding::new("cmd-down", MoveToEnd, INPUT),
        KeyBinding::new("alt-left", MoveToPreviousWord, INPUT),
        KeyBinding::new("alt-right", MoveToNextWord, INPUT),
        KeyBinding::new("shift-cmd-left", SelectToStartOfLine, INPUT),
        KeyBinding::new("shift-cmd-right", SelectToEndOfLine, INPUT),
        KeyBinding::new("cmd-shift-up", SelectToStart, INPUT),
        KeyBinding::new("cmd-shift-down", SelectToEnd, INPUT),
        KeyBinding::new("alt-shift-left", SelectToPreviousWordStart, INPUT),
        KeyBinding::new("alt-shift-right", SelectToNextWordEnd, INPUT),
        KeyBinding::new("cmd-backspace", DeleteToBeginningOfLine, INPUT),
        KeyBinding::new("cmd-delete", DeleteToEndOfLine, INPUT),
        KeyBinding::new("alt-backspace", DeleteToPreviousWordStart, INPUT),
        KeyBinding::new("alt-delete", DeleteToNextWordEnd, INPUT),
        KeyBinding::new("cmd-enter", Send, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-enter", Send, Some("Workspace > Input")),
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

/// The method short enough for a list's narrow column: `DEL`, `OPT`.
pub fn method_label(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Delete => "DEL",
        HttpMethod::Options => "OPT",
        HttpMethod::Connect => "CONN",
        method => method.as_str(),
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
    if !state.upgrade().is_some_and(|state| state.read(cx).editable()) {
        return;
    }
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

pub use import_dialog::open_import_dialog;

/// M11: the project or a folder as a file in the downloads folder — or, in
/// a page, on the clipboard.
fn export(state: &WeakEntity<WorkspaceState>, folder: Option<ItemId>, window: &mut Window, cx: &mut App) {
    let Some(state) = state.upgrade() else { return };
    let (name, json) = state.read(cx).export(folder.as_ref());
    if !crate::this_build::saves_files() {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(json));
        window.push_notification(Notification::success(t!("export.copied").to_string()), cx);
        return;
    }
    let saving = cx.background_spawn(async move { crate::this_build::save_to_downloads(&name, json.as_bytes()) });
    window
        .spawn(cx, async move |cx| {
            let saved = saving.await;
            let _ = cx.update(|window, cx| {
                let note = match saved {
                    Ok(path) => Notification::success(t!("export.saved", path = path).to_string()),
                    Err(message) => Notification::error(message),
                };
                window.push_notification(note, cx);
            });
        })
        .detach();
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
    _editor_events: Subscription,
}

pub struct WorkspaceView {
    id: CollectionId,
    state: Entity<WorkspaceState>,
    tree: Entity<TreePanel>,
    pins: Entity<PinsBar>,
    selected: Option<Selected>,
    columns: Entity<ResizableState>,
    focus: FocusHandle,
    /// The window title last set.
    title: String,
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
                this.follow_title(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(&state, window, |_, _, event, window, cx| match event {
                WorkspaceEvent::Imported { apis, folders } => {
                    window.push_notification(
                        Notification::success(t!("import.done", apis = apis, folders = folders).to_string()),
                        cx,
                    );
                }
                WorkspaceEvent::Failed(error) => {
                    window.push_notification(Notification::error(i18n::store_error(error)), cx);
                }
                WorkspaceEvent::Curl(command) => {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(command.clone()));
                    window.push_notification(Notification::success(t!("response.curl_copied").to_string()), cx);
                }
                WorkspaceEvent::CurlFailed(message) => {
                    window.push_notification(Notification::error(message.clone()), cx);
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
            title: String::new(),
            _subscriptions: subscriptions,
        };
        view.follow_selection(window, cx);
        view.follow_title(window, cx);
        view
    }

    /// F1.5: the window is named after the selected API, or the project
    /// while none is.
    fn follow_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let name = state
            .selected()
            .and_then(|id| state.tree().get(id))
            .map(|node| node.name().to_string())
            .unwrap_or_else(|| state.collection().name.clone());
        let title = format!("{APP_NAME} - {name}");
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }
    }

    /// Makes the editor and response columns of the selected API's state, or
    /// drops them when nothing is selected.
    fn follow_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let api: Option<Entity<ApiState>> = self.state.read(cx).api().cloned();
        // Two columns become three, or three two: the sizes start again from
        // the kept widths. Carried over, the empty column's measured width
        // and the new column's would overflow, and flex would squeeze the
        // tree to fit.
        if api.is_some() != self.selected.is_some() {
            self.columns = cx.new(|_| ResizableState::default());
        }
        match api {
            Some(api) if self.selected.as_ref().map(|s| s.api) != Some(api.entity_id()) => {
                let state = self.state.clone();
                let editor = cx.new(|cx| RequestEditor::new(state, api.clone(), window, cx));
                let response = cx.new(|cx| ResponsePanel::new(api.clone(), window, cx));
                let editor_events = cx.subscribe_in(&editor, window, |this, _, event, window, cx| match event {
                    RequestEditorEvent::EditorShare(share) => this.set_editor_share(*share, window, cx),
                });
                self.selected = Some(Selected {
                    api: api.entity_id(),
                    editor,
                    response,
                    _editor_events: editor_events,
                });
            }
            Some(_) => {}
            None => self.selected = None,
        }
    }

    /// F14.5: the widths kept — the tree in pixels, the editor as its share
    /// of the rest. Moving the tree's edge keeps that share: the editor's
    /// edge is the only one that changes it.
    fn columns_resized(&mut self, columns: &Entity<ResizableState>, window: &mut Window, cx: &mut Context<Self>) {
        let sizes = columns.read(cx).sizes().clone();
        let app = AppStore::global(cx).state().clone();
        let kept = app.read(cx).prefs().columns;
        let (widths, editor) = match sizes[..] {
            [tree, editor, response] => {
                let rest = f32::from(editor) + f32::from(response);
                if rest <= 0. {
                    return;
                }
                let tree = f32::from(tree);
                if (tree - kept.tree).abs() > 0.5 {
                    let widths = ColumnWidths { tree, ..kept };
                    (widths, Some(rest * kept.editor_ratio.clamp(0.2, 0.8)))
                } else {
                    let editor_ratio = (f32::from(editor) / rest).clamp(0.2, 0.8);
                    (ColumnWidths { tree, editor_ratio }, None)
                }
            }
            // Nothing selected: the tree and the space beside it.
            [tree, _] => (
                ColumnWidths {
                    tree: f32::from(tree),
                    ..kept
                },
                None,
            ),
            _ => return,
        };
        app.update(cx, |app, cx| app.set_columns(widths, cx));
        if let Some(editor) = editor {
            columns.update(cx, |columns, cx| columns.resize_panel(1, px(editor), window, cx));
        }
    }

    /// F4.14: the editor column at `share` of what the tree leaves, kept
    /// as a drag would keep it.
    fn set_editor_share(&mut self, share: f32, window: &mut Window, cx: &mut Context<Self>) {
        let sizes = self.columns.read(cx).sizes().clone();
        let [_, editor, response] = sizes[..] else {
            return;
        };
        let width = (f32::from(editor) + f32::from(response)) * share;
        let columns = self.columns.clone();
        columns.update(cx, |columns, cx| columns.resize_panel(1, px(width), window, cx));
        self.columns_resized(&columns, window, cx);
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

    fn render_nothing_selected(&self, _: &App) -> impl IntoElement {
        v_flex().size_full().justify_center().items_center().child(
            v_flex().max_w(px(420.)).px_8().items_center().child(
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
        // A closed dialog leaves nothing focused; the workspace takes it
        // back, so its keys (F3.12) keep working.
        if window.focused(cx).is_none() {
            self.focus.focus(window, cx);
        }
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
            None => group.child(resizable_panel().child(self.render_nothing_selected(cx))),
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
            .child(
                group.on_resize(cx.listener(|this, columns: &Entity<ResizableState>, window, cx| {
                    this.columns_resized(columns, window, cx)
                })),
            )
    }
}
