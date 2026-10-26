//! A project's workspace: the tree, the selected API's request editor and
//! its response side by side in resizable columns (F14.5), and the pins and
//! the selected request the title bar shows.

mod assertions_editor;
mod auth_editor;
mod compare_dialog;
mod docs_editor;
mod event_stream;
mod import_dialog;
mod move_dialog;
mod palette;
mod pins;
mod project_title;
mod prompt_dialog;
mod recent_dialog;
mod request_editor;
mod request_settings;
mod response_panel;
mod run_dialog;
mod schema_browser;
mod settings_dialogs;
mod tree_panel;
mod versions_dialog;

use crate::APP_NAME;
use crate::i18n;
use crate::states::{ApiState, AppStore, ServerCopy, WorkspaceEvent, WorkspaceState};
use crate::views::name_dialog::open_name_dialog;
use crate::views::sync_dialog::open_conflict_dialog;
use cyberapi_core::import::{ImportBundle, ImportItem};
use cyberapi_core::model::{Collection, HttpMethod};
use cyberapi_core::prefs::ColumnWidths;
use cyberapi_core::send::unsent_body;
use cyberapi_core::tree::ItemKind;
use cyberapi_core::{CollectionId, ItemId};
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::empty::{
    Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::input::Search;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EntityId, FocusHandle, Focusable, Hsla, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement as _, Render, Styled as _, Subscription, WeakEntity, Window, px,
};
pub use pins::PinsBar;
pub use project_title::ProjectTitle;
// For the shortcuts' list.
use request_editor::{RequestEditor, RequestEditorEvent};
use response_panel::ResponsePanel;
use rust_i18n::t;
use tree_panel::TreePanel;
pub(crate) use tree_panel::{
    Close as TreeClose, Duplicate as TreeDuplicate, KEY_CONTEXT as TREE_CONTEXT, Next as TreeNext, Open as TreeOpen,
    Previous as TreePrevious, Remove as TreeRemove, Rename as TreeRename, Toggle as TreeToggle,
};

gpui_kit::actions!(workspace, [NewApi, NewFolder, Send, PasteCurl, FindApi, FocusUrl]);

// F12.2 from the keyboard: Cmd/Ctrl and a pin's number selects it.
gpui_kit::actions!(workspace, [Pin1, Pin2, Pin3, Pin4, Pin5, Pin6, Pin7, Pin8, Pin9]);

/// The action of the pin at `index` (counting from 0), for the first nine.
pub fn pin_action(index: usize) -> Option<Box<dyn gpui_kit::Action>> {
    let actions: [Box<dyn gpui_kit::Action>; 9] = [
        Box::new(Pin1),
        Box::new(Pin2),
        Box::new(Pin3),
        Box::new(Pin4),
        Box::new(Pin5),
        Box::new(Pin6),
        Box::new(Pin7),
        Box::new(Pin8),
        Box::new(Pin9),
    ];
    actions.into_iter().nth(index)
}

pub(crate) const KEY_CONTEXT: &str = "Workspace";

/// F3.12, for both platforms: `secondary` is Cmd on macOS and Ctrl elsewhere
/// (#22 — the old app listened for Cmd alone).
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewApi, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-shift-n", NewFolder, Some(KEY_CONTEXT)),
        // F10.5: pasting where no field takes it. A field binds the same keys
        // closer in, and keeps its own paste.
        KeyBinding::new("secondary-v", PasteCurl, Some(KEY_CONTEXT)),
        // F5.1 from the keyboard, wherever focus is in the workspace — in
        // a field too, ahead of the field's own Cmd/Ctrl-Enter (bound later,
        // so it wins over the kit's).
        KeyBinding::new("secondary-enter", Send, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-enter", Send, Some("Workspace > Input")),
        // #64: to the API search, Enter there opening the first found; a
        // field that cannot search hands the same keys on as its `Search`.
        KeyBinding::new("secondary-f", FindApi, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-l", FocusUrl, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-1", Pin1, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-2", Pin2, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-3", Pin3, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-4", Pin4, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-5", Pin5, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-6", Pin6, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-7", Pin7, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-8", Pin8, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-9", Pin9, Some(KEY_CONTEXT)),
    ]);
    tree_panel::bind_keys(cx);
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
        KeyBinding::new("cmd-v", PasteCurl, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-f", FindApi, Some(KEY_CONTEXT)),
    ]);
}

/// The color of a method's label (§6): DELETE red, the methods with a body
/// green, a gRPC call magenta, the rest blue.
pub fn method_color(method: HttpMethod, cx: &App) -> Hsla {
    let theme = cx.theme();
    match method {
        HttpMethod::Delete => theme.danger,
        HttpMethod::Grpc => theme.magenta,
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

pub use import_dialog::{open_import_dialog, open_import_project_dialog};
pub use palette::{
    OpenPalette, ROOT_CONTEXT, bind_keys as bind_palette_keys, bind_mac_page_keys as bind_palette_mac_page_keys,
    open_palette,
};
pub use settings_dialogs::open_project_auth;

/// F10.5: the cURL command on the clipboard as a new API in `parent`, opened.
/// The clipboard is read here, when the reader pastes or picks the menu
/// item, and never otherwise; a page's read goes through the browser, which
/// may ask the reader first.
fn paste_curl(state: &WeakEntity<WorkspaceState>, parent: Option<ItemId>, window: &mut Window, cx: &mut App) {
    if !state.upgrade().is_some_and(|state| state.read(cx).editable()) {
        return;
    }
    // Asked for now, while the paste or the click still counts as the
    // reader's: a page may read the clipboard only then.
    let read = cx.read_from_clipboard_async();
    let state = state.clone();
    window
        .spawn(cx, async move |cx| {
            let read = read.await;
            let _ = cx.update(|window, cx| {
                let text = match read {
                    Ok(item) => item.and_then(|item| item.text()).unwrap_or_default(),
                    Err(error) => {
                        let message = t!("import.clipboard_unreadable", reason = error.to_string());
                        return window.push_notification(Notification::warning(message.to_string()), cx);
                    }
                };
                match cyberapi_core::import::curl_command(&text) {
                    Some(bundle) => {
                        let unsent = bundle.items.iter().find_map(|item| match item {
                            ImportItem::Api { request, .. } => unsent_body(request).then_some(request.method),
                            _ => None,
                        });
                        let _ = state.update(cx, |state, cx| state.import(parent, bundle, cx));
                        if let Some(method) = unsent {
                            tell_unsent_body(method, window, cx);
                        }
                    }
                    None => window.push_notification(Notification::warning(t!("import.no_curl").to_string()), cx),
                }
            });
        })
        .detach();
}

/// A cURL command's body its method does not send (#62): kept, and said.
fn tell_unsent_body(method: HttpMethod, window: &mut Window, cx: &mut App) {
    let text = t!("editor.curl_body_unsent", method = method.as_str()).to_string();
    window.push_notification(Notification::warning(text), cx);
}

/// M11: the project or a folder as a file in the downloads folder — or, in
/// a page, on the clipboard.
fn export(
    state: &WeakEntity<WorkspaceState>,
    folder: Option<ItemId>,
    format: cyberapi_core::export::ExportFormat,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(state) = state.upgrade() else { return };
    let (name, json) = state.read(cx).export(folder.as_ref(), format);
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
    // What goes with a folder, counted.
    let (apis, folders) = state
        .upgrade()
        .map(|state| {
            let tree = state.read(cx).tree();
            let inside = tree.descendants(&id);
            let folders = inside
                .iter()
                .filter(|item| tree.get(item).is_some_and(|node| node.is_folder()))
                .count();
            (inside.len() - folders, folders)
        })
        .unwrap_or_default();
    let state = state.clone();
    let (title, description) = match kind {
        ItemKind::Folder if apis + folders == 0 => (
            t!("workspace.delete_folder_title", name = name),
            t!("workspace.delete_empty_folder_description"),
        ),
        ItemKind::Folder => {
            // Only what there is: "3 APIs", "3 APIs, 1 folder".
            let what = [(apis, "workspace.api"), (folders, "workspace.folder")]
                .into_iter()
                .filter(|(count, _)| *count > 0)
                .map(|(count, key)| i18n::counted(key, count))
                .collect::<Vec<_>>()
                .join(&t!("workspace.list_separator"));
            (
                t!("workspace.delete_folder_title", name = name),
                t!("workspace.delete_folder_description", what = what),
            )
        }
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

/// The project's state: a server project's copy (`docs/server-sync.md`
/// §3–4, §6) — every project of a page, the signed-in account's copies on
/// the desktop — or a project of this machine's, written at once.
fn open_workspace(collection: Collection, cx: &mut Context<WorkspaceState>) -> WorkspaceState {
    let store = AppStore::global(cx);
    let sender = store.sender(cx);
    if let Some(scope) = store.draft_scope(cx) {
        let copy = ServerCopy::page(&collection, scope);
        let server = store.storage().clone();
        return WorkspaceState::server_copy(sender, collection, server, copy, cx);
    }
    let desktop = store.server().and_then(|server| {
        let server = server.read(cx);
        server.mine(&collection.id)?;
        Some((server.storage()?, server.copies()?.clone(), server.account()?))
    });
    match desktop {
        Some((server, copies, account)) => {
            let copy = ServerCopy::desktop(&collection, store.storage().clone(), copies, account);
            WorkspaceState::server_copy(sender, collection, server, copy, cx)
        }
        None => WorkspaceState::new(sender, collection, cx),
    }
}

pub struct WorkspaceView {
    id: CollectionId,
    state: Entity<WorkspaceState>,
    tree: Entity<TreePanel>,
    pins: Entity<PinsBar>,
    title_name: Entity<ProjectTitle>,
    selected: Option<Selected>,
    columns: Entity<ResizableState>,
    focus: FocusHandle,
    /// The window title last set.
    title: String,
    _subscriptions: Vec<Subscription>,
}

impl WorkspaceView {
    pub fn new(collection: Collection, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| open_workspace(collection, cx));
        Self::with_state(state, window, cx)
    }

    /// #65: what the project was made from, imported once it has loaded.
    pub fn import_when_loaded(&mut self, import: std::sync::Arc<ImportBundle>, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.import_when_loaded(import, cx));
    }

    /// The page drawn again around a project already open: what it holds
    /// (the answers too, those the history could not keep) stays, and only
    /// the views are made anew — in a new language, say.
    pub fn with_state(state: Entity<WorkspaceState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let id = state.read(cx).collection().id.clone();
        let tree = cx.new(|cx| TreePanel::new(state.clone(), window, cx));
        let pins = cx.new(|cx| PinsBar::new(state.clone(), cx));
        let title_name = cx.new(|cx| ProjectTitle::new(state.clone(), cx));
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| {
                this.follow_selection(window, cx);
                this.follow_title(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(&state, window, |_, this_state, event, window, cx| match event {
                WorkspaceEvent::Imported { apis, folders, again } => {
                    let mut text = t!(
                        "import.done",
                        apis = i18n::counted("workspace.api", *apis),
                        folders = i18n::counted("import.folder", *folders)
                    )
                    .to_string();
                    // B3: what became of the APIs the project had.
                    if let Some((updated, left)) = again {
                        if *updated > 0 {
                            text += &t!("import.done_updated", apis = i18n::counted("workspace.api", *updated));
                        }
                        if *left > 0 {
                            text += &t!("import.done_left", apis = i18n::counted("workspace.api", *left));
                        }
                    }
                    window.push_notification(Notification::success(text), cx);
                }
                WorkspaceEvent::Failed(error) => {
                    window.push_notification(Notification::error(i18n::store_error(error)), cx);
                }
                WorkspaceEvent::Code(target, code) => {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(code.clone()));
                    window.push_notification(Notification::success(crate::views::copied_code(*target, code)), cx);
                }
                WorkspaceEvent::CurlFailed(message) => {
                    window.push_notification(Notification::error(message.clone()), cx);
                }
                WorkspaceEvent::FirstChange => {
                    window.push_notification(Notification::info(t!("sync.first_change").to_string()), cx);
                }
                WorkspaceEvent::NotKept => {
                    window.push_notification(Notification::warning(t!("sync.not_kept").to_string()), cx);
                }
                WorkspaceEvent::KeptUnsynced { behind } => {
                    let message = if *behind {
                        Notification::warning(t!("sync.kept_unsynced_behind").to_string())
                    } else {
                        Notification::info(t!("sync.kept_unsynced").to_string())
                    };
                    window.push_notification(message, cx);
                }
                WorkspaceEvent::NotPulled(error) => {
                    let reason = i18n::store_error(error);
                    window.push_notification(
                        Notification::warning(t!("sync.not_pulled", reason = reason).to_string()),
                        cx,
                    );
                }
                WorkspaceEvent::Synced => {
                    window.push_notification(Notification::success(t!("sync.synced").to_string()), cx);
                }
                WorkspaceEvent::TookServers => {
                    window.push_notification(Notification::success(t!("sync.took_servers").to_string()), cx);
                }
                WorkspaceEvent::NotIntoLocal => {
                    window.push_notification(Notification::warning(t!("sync.not_into_local").to_string()), cx);
                }
                WorkspaceEvent::Conflict => {
                    let state = this_state.clone();
                    let name = state.read(cx).collection().name.clone();
                    let theirs = state.clone();
                    open_conflict_dialog(
                        &name,
                        window,
                        cx,
                        move |cx| state.update(cx, |state, cx| state.sync(true, cx)),
                        move |cx| theirs.update(cx, |state, cx| state.take_server(cx)),
                    );
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
            title_name,
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

    /// F1.5: the window is named after the opened API, or the project
    /// while none is.
    fn follow_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let name = state
            .opened()
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
                let editor = cx.new(|cx| RequestEditor::new(state.clone(), api.clone(), window, cx));
                // An API just made: its address is what comes next.
                let id = api.read(cx).api().clone();
                if self.state.update(cx, |state, _| state.take_created(&id)) {
                    editor.update(cx, |editor, cx| editor.focus_uri(window, cx));
                }
                let response = cx.new(|cx| ResponsePanel::new(state.clone(), api.clone(), window, cx));
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

    /// F12.2: the pin with this number (from 0), if there is one, opened
    /// with the tree left as it is.
    fn find_api(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tree.update(cx, |tree, cx| tree.focus_search(window, cx));
    }

    fn select_pin(&mut self, index: usize, cx: &mut Context<Self>) {
        let id = self.state.read(cx).pinned().get(index).map(|api| api.id.clone());
        if let Some(id) = id {
            self.state.update(cx, |state, cx| state.open_pin(&id, cx));
        }
    }

    pub fn collection_id(&self) -> &CollectionId {
        &self.id
    }

    /// F12.2: the pins, for the title bar.
    pub fn pins(&self) -> &Entity<PinsBar> {
        &self.pins
    }

    /// The project's name and its sync, for the title bar.
    pub fn title_name(&self) -> &Entity<ProjectTitle> {
        &self.title_name
    }

    /// The workspace's state, for the title bar's menu.
    pub fn state(&self) -> &Entity<WorkspaceState> {
        &self.state
    }

    fn render_nothing_selected(&self, window: &Window, cx: &App) -> impl IntoElement {
        // F3.12's shortcuts, as this platform writes them, for a reader who
        // may change the tree.
        let muted = cx.theme().muted_foreground;
        let shortcuts: Vec<_> = [
            (&NewApi as &dyn gpui_kit::Action, t!("workspace.new_api"), true),
            (&NewFolder, t!("workspace.new_folder"), true),
            // Paste reaches every page.
            (&PasteCurl, t!("import.paste_curl"), false),
        ]
        .into_iter()
        .filter(|(_, _, new)| self.state.read(cx).editable() && (!new || crate::this_build::takes_new_keys()))
        .map(|(action, label, _)| (action, label))
        .filter_map(|(action, label)| {
            let shortcut = Kbd::binding_for_action(action, Some(KEY_CONTEXT), window)?;
            Some(
                h_flex()
                    .w(px(200.))
                    .justify_between()
                    .items_center()
                    .text_sm()
                    .text_color(muted)
                    .child(label.to_string())
                    .child(shortcut),
            )
        })
        .collect();
        // An empty project has nothing to select yet: what to start with.
        let state = self.state.read(cx);
        let (title, hint) = if state.tree().is_empty() && state.editable() {
            (t!("workspace.start_title"), t!("workspace.start_hint"))
        } else {
            (t!("workspace.nothing_selected"), t!("workspace.select_hint"))
        };
        v_flex().size_full().justify_center().items_center().child(
            v_flex().max_w(px(420.)).px_8().items_center().child(
                Empty::new()
                    .header(
                        EmptyHeader::new()
                            .media(
                                EmptyMedia::new()
                                    .with_variant(EmptyMediaVariant::Icon)
                                    .child(Icon::new(IconName::FileText)),
                            )
                            .title(EmptyTitle::new().child(title.to_string()))
                            .description(EmptyDescription::new().child(hint.to_string())),
                    )
                    .when(!shortcuts.is_empty(), |empty| {
                        empty.content(EmptyContent::new().child(v_flex().gap_2().children(shortcuts)))
                    }),
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
        let paste_state = state.clone();
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
            None => group.child(resizable_panel().child(self.render_nothing_selected(window, cx))),
        };
        let editor = self.selected.as_ref().map(|selected| selected.editor.downgrade());
        let url_editor = editor.clone();
        h_flex()
            .size_full()
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|this, _: &Pin1, _, cx| this.select_pin(0, cx)))
            .on_action(cx.listener(|this, _: &Pin2, _, cx| this.select_pin(1, cx)))
            .on_action(cx.listener(|this, _: &Pin3, _, cx| this.select_pin(2, cx)))
            .on_action(cx.listener(|this, _: &Pin4, _, cx| this.select_pin(3, cx)))
            .on_action(cx.listener(|this, _: &Pin5, _, cx| this.select_pin(4, cx)))
            .on_action(cx.listener(|this, _: &Pin6, _, cx| this.select_pin(5, cx)))
            .on_action(cx.listener(|this, _: &Pin7, _, cx| this.select_pin(6, cx)))
            .on_action(cx.listener(|this, _: &Pin8, _, cx| this.select_pin(7, cx)))
            .on_action(cx.listener(|this, _: &Pin9, _, cx| this.select_pin(8, cx)))
            .on_action(move |_: &NewApi, window, cx| new_item(&state, None, ItemKind::Api, window, cx))
            .on_action(move |_: &NewFolder, window, cx| new_item(&folder_state, None, ItemKind::Folder, window, cx))
            // Beside the open API; the tree, when it has the focus, answers
            // first with its cursor's folder.
            .on_action(move |_: &PasteCurl, window, cx| {
                let parent = paste_state.upgrade().and_then(|state| state.read(cx).selected_folder());
                paste_curl(&paste_state, parent, window, cx)
            })
            .on_action(move |_: &Send, window, cx| {
                if let Some(editor) = &editor {
                    let _ = editor.update(cx, |editor, cx| editor.send(window, cx));
                }
            })
            .on_action(cx.listener(|this, _: &FindApi, window, cx| this.find_api(window, cx)))
            .on_action(cx.listener(|this, _: &Search, window, cx| this.find_api(window, cx)))
            .on_action(move |_: &FocusUrl, window, cx| {
                if let Some(editor) = &url_editor {
                    let _ = editor.update(cx, |editor, cx| editor.focus_uri_now(window, cx));
                }
            })
            .child(
                group.on_resize(cx.listener(|this, columns: &Entity<ResizableState>, window, cx| {
                    this.columns_resized(columns, window, cx)
                })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::prefs::{SendPrefs, Timeouts};
    use cyberapi_core::template::NoFiles;
    use cyberapi_http::{HttpEngine, PreparedRequest, SendFuture};
    use cyberapi_service::Sender;
    use cyberapi_store::{MemoryStore, NewCollection};
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AnyWindowHandle, ClipboardItem, TestAppContext, div, size};
    use std::sync::Arc;

    struct NoEngine;

    impl HttpEngine for NoEngine {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            Box::pin(std::future::pending())
        }
    }

    struct Blank;

    impl Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    fn workspace(cx: &mut TestAppContext) -> (Entity<WorkspaceState>, AnyWindowHandle) {
        cx.update(gpui_kit::init);
        let storage = MemoryStore::new().storage();
        let collection = pollster::block_on(storage.collections().create(NewCollection {
            name: "p".into(),
            ..Default::default()
        }))
        .expect("create");
        let sender = Sender {
            storage,
            engine: Arc::new(NoEngine),
            files: Arc::new(NoFiles("no files in tests")),
            env: Arc::new(cyberapi_core::template::NoEnv("no environment in tests")),
            user_agent: "CyberAPI/test".into(),
            timeouts: Timeouts::default(),
            sending: SendPrefs::default(),
            inherited: Default::default(),
            environment: None,
        };
        let state = cx.new(|cx| WorkspaceState::new(sender, collection, cx));
        let window = cx.open_window(size(px(800.), px(600.)), |window, cx| {
            Root::new(cx.new(|_| Blank), window, cx)
        });
        cx.run_until_parked();
        (state, window.into())
    }

    fn paste(cx: &mut TestAppContext, state: &Entity<WorkspaceState>, window: AnyWindowHandle) {
        cx.update_window(window, |_, window, cx| paste_curl(&state.downgrade(), None, window, cx))
            .expect("the window is open");
        cx.run_until_parked();
    }

    /// A workspace with a field in it, counting the pastes it gets.
    struct Pasting {
        focus: FocusHandle,
        field: Entity<gpui_kit::component::input::InputState>,
        pasted: usize,
    }

    impl Render for Pasting {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .key_context(KEY_CONTEXT)
                .track_focus(&self.focus)
                .on_action(cx.listener(|this, _: &PasteCurl, _, _| this.pasted += 1))
                .child(gpui_kit::component::input::Input::new(&self.field))
        }
    }

    /// The paste key adds a command where no field has the focus, and is a
    /// field's own paste in one.
    #[gpui_kit::test]
    async fn a_field_keeps_its_paste(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            bind_keys(cx);
        });
        cx.write_to_clipboard(ClipboardItem::new_string("curl https://a.test".into()));
        let mut view = None;
        let window: AnyWindowHandle = cx
            .open_window(size(px(400.), px(300.)), |window, cx| {
                let pasting = cx.new(|cx| Pasting {
                    focus: cx.focus_handle(),
                    field: cx.new(|cx| gpui_kit::component::input::InputState::new(window, cx)),
                    pasted: 0,
                });
                view = Some(pasting.clone());
                Root::new(pasting, window, cx)
            })
            .into();
        let view = view.expect("built");
        let press = |cx: &mut TestAppContext, field: bool| {
            cx.update_window(window, |_, window, cx| {
                let pasting = view.read(cx);
                let focus = if field {
                    pasting.field.read(cx).focus_handle(cx)
                } else {
                    pasting.focus.clone()
                };
                focus.focus(window, cx);
                window.render_frame(cx);
                // ⌘V on a Mac, Ctrl-V elsewhere, as the binding says.
                window.press("secondary-v", cx);
            })
            .expect("the window is open");
            cx.run_until_parked();
        };
        press(cx, false);
        assert_eq!(cx.read(|cx| view.read(cx).pasted), 1);
        press(cx, true);
        assert_eq!(cx.read(|cx| view.read(cx).pasted), 1, "not the workspace's");
        let field = cx.read(|cx| view.read(cx).field.read(cx).value().to_string());
        assert_eq!(field, "curl https://a.test", "the field's own paste");
    }

    /// F10.5: what is on the clipboard is read when the reader pastes, and
    /// only a cURL command adds an API.
    #[gpui_kit::test]
    async fn a_pasted_curl_command_becomes_an_api(cx: &mut TestAppContext) {
        let (state, window) = workspace(cx);
        let count = |cx: &mut TestAppContext| state.read_with(cx, |state, _| state.rows().len());

        cx.write_to_clipboard(ClipboardItem::new_string("not a command".into()));
        paste(cx, &state, window);
        assert_eq!(count(cx), 0, "nothing but a cURL command is added");

        cx.write_to_clipboard(ClipboardItem::new_string(
            "curl 'https://a.test/v1/users/login' -H 'X-Trace: 1' --data-raw '{\"a\":1}'".into(),
        ));
        paste(cx, &state, window);
        state.read_with(cx, |state, _| {
            let api = state.opened_api().expect("the new API is opened");
            assert_eq!(api.name, "login");
            assert_eq!(api.request.method, HttpMethod::Post);
            assert_eq!(api.request.body, "{\"a\":1}");
        });
    }
}
