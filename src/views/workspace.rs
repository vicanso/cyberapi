//! A project's workspace: the tree on the left, the selected API on the
//! right (its editor comes with step 7 of `docs/architecture.md`), and the
//! pins the title bar shows.

mod pins;
mod tree_panel;

use crate::i18n;
use crate::states::{AppStore, WorkspaceEvent, WorkspaceState};
use crate::views::name_dialog::open_name_dialog;
use cyberapi_core::model::{Collection, HttpMethod};
use cyberapi_core::tree::ItemKind;
use cyberapi_core::{CollectionId, ItemId};
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, StyledExt as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Hsla, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement as _, Render, Styled as _, Subscription, WeakEntity, Window, div, px,
};
pub use pins::PinsBar;
use rust_i18n::t;
use tree_panel::TreePanel;

gpui_kit::actions!(workspace, [NewApi, NewFolder]);

const KEY_CONTEXT: &str = "Workspace";

/// F3.12, for both platforms: `secondary` is Cmd on macOS and Ctrl elsewhere
/// (#22 — the old app listened for Cmd alone).
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewApi, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-shift-n", NewFolder, Some(KEY_CONTEXT)),
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

pub struct WorkspaceView {
    id: CollectionId,
    state: Entity<WorkspaceState>,
    tree: Entity<TreePanel>,
    pins: Entity<PinsBar>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl WorkspaceView {
    pub fn new(collection: Collection, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let id = collection.id.clone();
        let storage = AppStore::global(cx).storage().clone();
        let state = cx.new(|cx| WorkspaceState::new(storage, collection, cx));
        let tree = cx.new(|cx| TreePanel::new(state.clone(), window, cx));
        let pins = cx.new(|cx| PinsBar::new(state.clone(), cx));
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe_in(&state, window, |_, _, event, window, cx| match event {
                WorkspaceEvent::Failed(error) => {
                    window.push_notification(Notification::error(i18n::store_error(error)), cx);
                }
            }),
        ];
        // The shortcuts reach the workspace from wherever focus is inside it.
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            id,
            state,
            tree,
            pins,
            focus,
            _subscriptions: subscriptions,
        }
    }

    pub fn collection_id(&self) -> &CollectionId {
        &self.id
    }

    /// F12.2: the pins, for the title bar.
    pub fn pins(&self) -> &Entity<PinsBar> {
        &self.pins
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let muted = cx.theme().muted_foreground;
        match state.selected_api() {
            Some(api) => v_flex()
                .flex_1()
                .h_full()
                .p_6()
                .gap_3()
                .child(div().text_lg().font_semibold().child(api.name.clone()))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_sm()
                        .child(
                            div()
                                .font_semibold()
                                .text_color(method_color(api.request.method, cx))
                                .child(api.request.method.as_str()),
                        )
                        .child(div().text_color(muted).child(api.request.uri.clone())),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(t!("workspace.editor_pending").to_string()),
                ),
            None => v_flex().flex_1().h_full().justify_center().items_center().child(
                Empty::new().header(
                    EmptyHeader::new()
                        .media(
                            EmptyMedia::new()
                                .with_variant(EmptyMediaVariant::Icon)
                                .child(Icon::new(IconName::FileText)),
                        )
                        .title(EmptyTitle::new().child(t!("workspace.nothing_selected").to_string()))
                        .description(EmptyDescription::new().child(t!("workspace.editor_pending").to_string())),
                ),
            ),
        }
    }
}

impl Focusable for WorkspaceView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree_width = AppStore::global(cx).state().read(cx).prefs().columns.tree;
        let state = self.state.downgrade();
        let folder_state = state.clone();
        h_flex()
            .size_full()
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            .on_action(move |_: &NewApi, window, cx| new_item(&state, None, ItemKind::Api, window, cx))
            .on_action(move |_: &NewFolder, window, cx| new_item(&folder_state, None, ItemKind::Folder, window, cx))
            .child(
                div()
                    .h_full()
                    .w(px(tree_width))
                    .flex_none()
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .child(self.tree.clone()),
            )
            .child(self.render_detail(cx))
    }
}
