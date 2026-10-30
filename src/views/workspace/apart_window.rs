//! F7: an open tab in a window of its own (desktop): its request and its
//! response side by side, while the main window's tab says where it went.
//! The window's editor is the tab's only one — two editors of one request
//! would each write back what they hold — so the main window shows none
//! while it is open. Closing the window puts the tab back; closing the tab,
//! or leaving the project, closes the window.

use super::request_editor::{RequestEditor, RequestEditorEvent};
use super::response_panel::ResponsePanel;
use super::{FocusUrl, KEY_CONTEXT, Send};
use crate::states::{ApiState, AppStore, Route, WorkspaceState};
use crate::{APP_NAME, CloseWindow};
use cyberapi_core::ItemId;
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel};
use gpui_kit::component::{ActiveTheme as _, TitleBar, h_flex, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Global, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Styled as _, Subscription, Window, WindowBounds, WindowOptions, div, px,
    size,
};
use std::collections::HashMap;
use tracing::error;

/// The tabs' own windows while they are open.
#[derive(Default)]
struct Open(HashMap<ItemId, AnyWindowHandle>);

impl Global for Open {}

/// Whether `window` is a tab's own: like the settings', it is not the app.
#[cfg(not(target_os = "macos"))]
pub fn is_apart_window(window: AnyWindowHandle, cx: &App) -> bool {
    cx.try_global::<Open>()
        .is_some_and(|open| open.0.values().any(|handle| *handle == window))
}

/// Opens `id`'s tab of `workspace` in a window of its own, or brings that
/// window forward.
pub fn open_apart(workspace: Entity<WorkspaceState>, id: ItemId, cx: &mut App) {
    if let Some(open) = cx.try_global::<Open>().and_then(|open| open.0.get(&id).copied())
        && open.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    let Some(api) = workspace.read(cx).kept_api(&id).cloned() else {
        workspace.update(cx, |workspace, cx| workspace.join(&id, cx));
        return;
    };
    let display = cx
        .active_window()
        .and_then(|window| window.update(cx, |_, window, cx| window.display(cx)).ok().flatten())
        .map(|display| display.id());
    let mut titlebar = TitleBar::title_bar_options();
    titlebar.title = Some(APP_NAME.into());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            display,
            size(px(1100.), px(720.)),
            cx,
        ))),
        window_min_size: Some(size(px(640.), px(420.))),
        titlebar: Some(titlebar),
        ..TitleBar::window_options()
    };
    let (built, kept) = (workspace.clone(), id.clone());
    match gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| ApartView::new(built, kept, api, window, cx))
    }) {
        Ok((handle, _)) => {
            cx.default_global::<Open>().0.insert(id, handle);
        }
        Err(e) => {
            error!(error = %e, "a tab's window could not be opened");
            workspace.update(cx, |workspace, cx| workspace.join(&id, cx));
        }
    }
}

/// Closes `id`'s own window, which puts its tab back.
pub fn close_apart(id: &ItemId, cx: &mut App) {
    if let Some(open) = cx.try_global::<Open>().and_then(|open| open.0.get(id).copied()) {
        let _ = open.update(cx, |_, window, _| window.remove_window());
    }
}

struct ApartView {
    workspace: Entity<WorkspaceState>,
    id: ItemId,
    api: Entity<ApiState>,
    editor: Entity<RequestEditor>,
    response: Entity<ResponsePanel>,
    columns: Entity<ResizableState>,
    focus: FocusHandle,
    /// What the system was last told the window is called.
    title: String,
    _subscriptions: Vec<Subscription>,
}

impl ApartView {
    fn new(
        workspace: Entity<WorkspaceState>,
        id: ItemId,
        api: Entity<ApiState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| RequestEditor::new(workspace.clone(), api.clone(), window, cx));
        let response = cx.new(|cx| ResponsePanel::new(workspace.clone(), api.clone(), window, cx));
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let collection = workspace.read(cx).collection().id.clone();
        let app = AppStore::global(cx).state().clone();
        let subscriptions = vec![
            cx.observe_in(&workspace, window, |this, _, window, cx| this.follow(window, cx)),
            // The editor's width presets are of this window's columns.
            cx.subscribe_in(&editor, window, |this, _, event, window, cx| match event {
                RequestEditorEvent::EditorShare(share) => this.set_editor_share(*share, window, cx),
            }),
            // Leaving the project ends it.
            cx.observe_in(&app, window, move |_, app, window, cx| {
                let here = matches!(app.read(cx).route(), Route::Workspace(open) if open.id == collection);
                if !here {
                    window.remove_window();
                }
            }),
            // What was typed last goes back with the tab — while the tab is
            // still this window's. One started again from the server's or
            // an import's request, or closed, is not written over (BUG-5).
            cx.on_release(|this, cx| {
                let still = this
                    .workspace
                    .read(cx)
                    .kept_api(&this.id)
                    .is_some_and(|kept| kept.entity_id() == this.api.entity_id());
                if still {
                    this.editor.update(cx, |editor, cx| editor.flush(cx));
                }
                if cx
                    .try_global::<Open>()
                    .is_some_and(|open| open.0.contains_key(&this.id))
                {
                    cx.global_mut::<Open>().0.remove(&this.id);
                }
                let id = this.id.clone();
                this.workspace.update(cx, |workspace, cx| workspace.join(&id, cx));
            }),
        ];
        let mut view = Self {
            workspace,
            id,
            api,
            editor,
            response,
            columns: cx.new(|_| ResizableState::default()),
            focus,
            title: String::new(),
            _subscriptions: subscriptions,
        };
        view.follow(window, cx);
        view
    }

    /// The editor's column at `share` of the window; for this window only,
    /// so not kept.
    fn set_editor_share(&mut self, share: f32, window: &mut Window, cx: &mut Context<Self>) {
        let sizes = self.columns.read(cx).sizes().clone();
        let [editor, response] = sizes[..] else {
            return;
        };
        let width = (f32::from(editor) + f32::from(response)) * share;
        self.columns
            .update(cx, |columns, cx| columns.resize_panel(0, px(width), window, cx));
    }

    /// Closes the window once its tab is closed, or edited from scratch;
    /// otherwise names it after the API.
    fn follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.read(cx);
        let same = workspace
            .kept_api(&self.id)
            .is_some_and(|kept| kept.entity_id() == self.api.entity_id());
        if !workspace.is_apart(&self.id) || !same {
            window.remove_window();
            return;
        }
        let name = workspace
            .tabs()
            .into_iter()
            .find(|tab| tab.id == self.id)
            .map(|tab| tab.name)
            .unwrap_or_default();
        let title = format!("{APP_NAME} - {name}");
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }
        cx.notify();
    }
}

impl Render for ApartView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A closed dialog leaves nothing focused: the window takes it back,
        // or ⌘W would hide the app and ⌘↵ go nowhere (BUG-17).
        if window.focused(cx).is_none() {
            self.focus.focus(window, cx);
        }
        let theme = cx.theme();
        let name = self
            .title
            .strip_prefix(&format!("{APP_NAME} - "))
            .unwrap_or_default()
            .to_string();
        let width = (f32::from(window.viewport_size().width) * 0.5).max(280.);
        let (send, focus_url) = (self.editor.downgrade(), self.editor.downgrade());
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            .on_action(move |_: &Send, window, cx| {
                let _ = send.update(cx, |editor, cx| editor.send(window, cx));
            })
            .on_action(move |_: &FocusUrl, window, cx| {
                let _ = focus_url.update(cx, |editor, cx| editor.focus_uri_now(window, cx));
            })
            // ⌘W closes this window, and only it: the tab goes back.
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(cx.listener(|_, _: &super::CloseTab, window, _| window.remove_window()))
            .child(TitleBar::new().child(h_flex().flex_1().justify_center().text_sm().child(name)))
            .child(
                div().flex_1().min_h_0().w_full().child(
                    h_resizable("apart-columns")
                        .with_state(&self.columns)
                        .child(
                            resizable_panel()
                                .size(px(width))
                                .size_range(px(280.)..px(4000.))
                                .child(self.editor.clone()),
                        )
                        .child(
                            resizable_panel()
                                .size_range(px(240.)..px(4000.))
                                .child(self.response.clone()),
                        ),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Extras;
    use cyberapi_core::prefs::AppPrefs;
    use cyberapi_http::{HttpEngine, PreparedRequest, SendFuture};
    use cyberapi_store::{MemoryStore, NewCollection};
    use gpui_kit::TestAppContext;
    use std::sync::Arc;

    struct NoEngine;

    impl HttpEngine for NoEngine {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            Box::pin(std::future::pending())
        }
    }

    fn windows(cx: &mut TestAppContext) -> usize {
        cx.update(|cx| cx.windows().len())
    }

    /// The tab's window opens with it, once; closing the window puts the
    /// tab back, and closing the tab closes the window.
    #[gpui_kit::test]
    async fn a_tabs_window_comes_and_goes_with_it(cx: &mut TestAppContext) {
        let store = MemoryStore::new();
        let storage = store.storage();
        cx.update(|cx| {
            gpui_kit::init(cx);
            AppStore::init(
                storage.clone(),
                Arc::new(NoEngine),
                AppPrefs::default(),
                Extras::default(),
                cx,
            );
        });
        let collection = storage
            .collections()
            .create(NewCollection {
                name: "p".into(),
                ..Default::default()
            })
            .await
            .expect("create");
        let workspace = cx.new(|cx| {
            let sender = AppStore::global(cx).sender(cx);
            WorkspaceState::new(sender, collection, cx)
        });
        cx.run_until_parked();
        workspace.update(cx, |workspace, cx| workspace.create_api(None, "a".into(), cx));
        cx.run_until_parked();
        let a = workspace.read_with(cx, |workspace, _| workspace.tabs()[0].id.clone());
        let part = |cx: &mut TestAppContext| {
            workspace.update(cx, |workspace, cx| workspace.part(&a, cx));
            cx.update(|cx| open_apart(workspace.clone(), a.clone(), cx));
            cx.run_until_parked();
        };

        part(cx);
        assert_eq!(windows(cx), 1);
        part(cx);
        assert_eq!(windows(cx), 1, "brought forward, not opened again");

        cx.update(|cx| close_apart(&a, cx));
        cx.run_until_parked();
        assert_eq!(windows(cx), 0);
        assert!(!workspace.read_with(cx, |workspace, _| workspace.is_apart(&a)), "back");

        part(cx);
        assert_eq!(windows(cx), 1);
        workspace.update(cx, |workspace, cx| workspace.close_tab(&a, cx));
        cx.run_until_parked();
        assert_eq!(windows(cx), 0, "the tab closed, its window with it");
        assert!(cx.read(|cx| cx.try_global::<Open>().is_none_or(|open| open.0.is_empty())));
    }

    /// What was typed in the window goes with it when it closes — unless
    /// the tab was started again from another request first (BUG-5): then
    /// it would write the old request back over the new one.
    #[gpui_kit::test]
    async fn closing_writes_back_only_the_tab_still_open(cx: &mut TestAppContext) {
        let store = MemoryStore::new();
        let storage = store.storage();
        cx.update(|cx| {
            gpui_kit::init(cx);
            AppStore::init(
                storage.clone(),
                Arc::new(NoEngine),
                AppPrefs::default(),
                Extras::default(),
                cx,
            );
        });
        let collection = storage
            .collections()
            .create(NewCollection {
                name: "p".into(),
                ..Default::default()
            })
            .await
            .expect("create");
        let workspace = cx.new(|cx| {
            let sender = AppStore::global(cx).sender(cx);
            WorkspaceState::new(sender, collection.clone(), cx)
        });
        cx.run_until_parked();
        workspace.update(cx, |workspace, cx| workspace.create_api(None, "list".into(), cx));
        cx.run_until_parked();
        let id = workspace.read_with(cx, |workspace, _| workspace.tabs()[0].id.clone());
        let api = workspace.read_with(cx, |workspace, _| workspace.kept_api(&id).cloned().expect("kept"));
        api.update(cx, |api, cx| api.set_uri("https://a.test/orders".into(), cx));
        cx.run_until_parked();
        let stored = || {
            let (storage, collection, id) = (storage.clone(), collection.id.clone(), id.clone());
            // The store is in memory: its calls finish as they are made.
            futures::executor::block_on(async move {
                let data = storage.tree().load(collection).await.expect("load");
                data.apis
                    .into_iter()
                    .find(|api| api.id == id)
                    .map(|api| api.request)
                    .expect("there")
            })
        };
        // Typed in the window, then the window closed.
        let apart = |cx: &mut TestAppContext| {
            workspace.update(cx, |workspace, cx| workspace.part(&id, cx));
            let api = workspace.read_with(cx, |workspace, _| workspace.kept_api(&id).cloned().expect("kept"));
            let (built, kept) = (workspace.clone(), id.clone());
            let (handle, view) = cx
                .update(|cx| {
                    gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| {
                        cx.new(|cx| ApartView::new(built, kept, api, window, cx))
                    })
                })
                .expect("open");
            cx.run_until_parked();
            (handle, view)
        };
        let type_uri = |handle: AnyWindowHandle, view: &Entity<ApartView>, text: &str, cx: &mut TestAppContext| {
            let editor = view.read_with(cx, |view, _| view.editor.clone());
            handle
                .update(cx, |_, window, cx| {
                    editor.update(cx, |editor, cx| editor.type_uri(text, window, cx))
                })
                .expect("open");
        };

        let (handle, view) = apart(cx);
        type_uri(handle, &view, "https://a.test/typed", cx);
        drop(view);
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        cx.run_until_parked();
        assert_eq!(stored().uri, "https://a.test/typed", "what was typed went with it");

        // Typed again; meanwhile an import updates the API, which starts
        // its tab again and so closes the window.
        api.update(cx, |api, cx| api.set_uri("https://a.test/orders".into(), cx));
        cx.run_until_parked();
        let (handle, view) = apart(cx);
        type_uri(handle, &view, "https://a.test/stale", cx);
        drop(view);
        let bundle = cyberapi_core::import::parse(
            r#"{"info": {"name": "Shop"}, "item": [
                {"name": "list", "request": {"method": "GET", "url": "https://a.test/orders",
                    "header": [{"key": "X-New", "value": "1"}]}}
            ]}"#,
        )
        .expect("a collection");
        workspace.update(cx, |workspace, cx| workspace.import_again(None, bundle, true, cx));
        cx.run_until_parked();
        assert_eq!(windows(cx), 0, "started again, the window closed");
        let request = stored();
        assert_eq!(request.uri, "https://a.test/orders", "not written back");
        assert!(
            request.headers.iter().any(|header| header.key == "X-New"),
            "the import's request stays: {:?}",
            request.headers
        );
    }
}
