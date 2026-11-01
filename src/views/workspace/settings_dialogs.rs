//! What a folder and the project set for their APIs (#66): a folder's auth
//! and the headers every API in it sends, the project's auth — and what
//! the folder is for (F3), and the variables its APIs read (D5). Kept when
//! Save is clicked; a viewer reads them and closes.

use super::auth_editor::{AuthEditor, AuthFor};
use super::import_dialog::address;
use super::request_editor::{COMMON_HEADERS, HEADER_EXAMPLE};
use crate::states::WorkspaceState;
use crate::views::kv_editor::{KvEditor, KvOptions};
use crate::views::name_dialog::ok_button;
use crate::views::template_hints::HintSource;
use cyberapi_core::ItemId;
use cyberapi_core::model::{FolderSettings, ProjectSettings};
use cyberapi_core::prefs::{DEFAULT_SOURCE_SYNC_MINUTES, SOURCE_SYNC_INTERVALS};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

/// D5: a line of a variables list as text.
pub(super) const VARIABLE_EXAMPLE: &str = "userId: 42";

/// A part of a dialog: its title, what it is for, and its fields.
fn section(title: String, about: String, body: impl IntoElement, cx: &App) -> AnyElement {
    v_flex()
        .gap_2()
        .child(
            v_flex()
                .gap_0p5()
                .child(div().text_sm().font_medium().child(title))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(about)),
        )
        .child(body)
        .into_any_element()
}

/// Cancel and Save for an editor; Close alone for a viewer.
fn footer(
    id: &'static str,
    editable: bool,
    save: impl Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static,
) -> DialogFooter {
    let leave = if editable {
        t!("common.cancel")
    } else {
        t!("common.close")
    }
    .to_string();
    let footer = DialogFooter::new().child(DialogClose::new().trigger(move |button| button.label(leave.clone())));
    if editable {
        footer.child(ok_button(id, t!("common.save").to_string(), save))
    } else {
        footer
    }
}

struct FolderSettingsView {
    workspace: gpui_kit::WeakEntity<WorkspaceState>,
    id: ItemId,
    description: Entity<EditorState>,
    /// B9: the folder's OpenAPI / import source URL.
    source: Entity<InputState>,
    source_error: Option<String>,
    /// B10: pending periodic auto-sync choice and interval (minutes) for this folder's source URL.
    want_auto_sync: Option<bool>,
    want_sync_interval: Option<u32>,
    editable: bool,
    auth: Entity<AuthEditor>,
    headers: Entity<KvEditor>,
    /// D5.
    variables: Entity<KvEditor>,
    _subscriptions: Vec<Subscription>,
}

impl FolderSettingsView {
    fn effective_sync(&self, url: &str, cx: &App) -> (bool, u32) {
        let (ws_sync, ws_interval) = self
            .workspace
            .upgrade()
            .map(|ws| {
                let ws = ws.read(cx);
                (ws.auto_syncs_source(url), ws.source_sync_interval(url))
            })
            .unwrap_or((false, DEFAULT_SOURCE_SYNC_MINUTES));
        (
            self.want_auto_sync.unwrap_or(ws_sync),
            self.want_sync_interval.unwrap_or(ws_interval),
        )
    }

    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let typed = self.source.read(cx).value().trim().to_string();
        let source = if typed.is_empty() {
            None
        } else {
            match address(&typed) {
                Some(url) => Some(url.to_string()),
                None => {
                    self.source_error = Some(t!("source.not_address").to_string());
                    cx.notify();
                    return false;
                }
            }
        };
        let sync_choice = source.as_deref().map(|url| {
            let (auto_sync, interval) = self.effective_sync(url, cx);
            (url.to_string(), auto_sync, interval)
        });
        let settings = FolderSettings {
            auth: self.auth.read(cx).auth(cx),
            headers: self.headers.read(cx).list(cx),
            description: self.description.read(cx).value().trim_end().to_string(),
            variables: self.variables.read(cx).list(cx),
            source,
        };
        let id = self.id.clone();
        self.want_auto_sync = None;
        self.want_sync_interval = None;
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |state, cx| {
                state.set_folder_settings(id, settings, cx);
                if let Some((url, want_auto_sync, want_interval)) = sync_choice {
                    state.set_auto_sync_source(url, want_auto_sync, want_interval, cx);
                }
            });
        }
        true
    }
}

impl Render for FolderSettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (border, danger, warning, muted) = (
            cx.theme().border,
            cx.theme().danger,
            cx.theme().warning,
            cx.theme().muted_foreground,
        );
        let typed = self.source.read(cx).value().trim().to_string();
        let valid_url = address(&typed);
        let local = valid_url.is_some_and(cyberapi_core::import::local_address);
        let (auto_sync, sync_interval) = valid_url
            .map(|url| self.effective_sync(url, cx))
            .unwrap_or((false, DEFAULT_SOURCE_SYNC_MINUTES));
        v_flex()
            .gap_5()
            .child(section(
                t!("docs.description").to_string(),
                t!("folder_settings.description_hint").to_string(),
                div()
                    .h(px(96.))
                    .border_1()
                    .border_color(border)
                    .rounded_md()
                    .child(Editor::new(&self.description).w_full().h_full().bordered(false)),
                cx,
            ))
            .child(section(
                t!("folder_settings.source_title").to_string(),
                t!("folder_settings.source_hint").to_string(),
                v_flex()
                    .gap_2()
                    .child(Input::new(&self.source).small().disabled(!self.editable))
                    .when_some(self.source_error.clone(), |this, why| {
                        this.child(div().text_xs().text_color(danger).child(why))
                    })
                    .when(local, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(warning)
                                .child(t!("source.local").to_string()),
                        )
                    })
                    .when(self.editable && valid_url.is_some(), |this| {
                        this.child(
                            v_flex()
                                .gap_1p5()
                                .child(
                                    Checkbox::new("folder-source-auto-sync")
                                        .checked(auto_sync)
                                        .label(t!("source.auto_sync").to_string())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.want_auto_sync = Some(!auto_sync);
                                            if this.want_sync_interval.is_none() {
                                                this.want_sync_interval = Some(sync_interval);
                                            }
                                            cx.notify();
                                        })),
                                )
                                .when(auto_sync, |col| {
                                    col.child(
                                        h_flex()
                                            .gap_1p5()
                                            .items_center()
                                            .pl_6()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .child(t!("source.sync_interval").to_string()),
                                            )
                                            .children(SOURCE_SYNC_INTERVALS.into_iter().map(|mins| {
                                                let chosen = sync_interval == mins;
                                                let label = t!("source.interval_minutes", minutes = mins).to_string();
                                                Button::new(("folder-source-interval", mins as usize))
                                                    .xsmall()
                                                    .when(chosen, |b| b.primary())
                                                    .when(!chosen, |b| b.outline())
                                                    .label(label)
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.want_auto_sync = Some(true);
                                                        this.want_sync_interval = Some(mins);
                                                        cx.notify();
                                                    }))
                                            })),
                                    )
                                }),
                        )
                    }),
                cx,
            ))
            .child(section(
                t!("editor.auth").to_string(),
                t!("folder_settings.auth_hint").to_string(),
                self.auth.clone(),
                cx,
            ))
            .child(section(
                t!("editor.headers").to_string(),
                t!("folder_settings.headers_hint").to_string(),
                self.headers.clone(),
                cx,
            ))
            .child(section(
                t!("own_variables.title").to_string(),
                t!("own_variables.folder_hint").to_string(),
                self.variables.clone(),
                cx,
            ))
    }
}

fn new_folder_settings_view(
    workspace: &Entity<WorkspaceState>,
    id: ItemId,
    window: &mut Window,
    cx: &mut App,
) -> Option<(String, bool, Entity<FolderSettingsView>)> {
    let (folder, editable) = {
        let state = workspace.read(cx);
        (state.folder(&id).cloned()?, state.editable())
    };
    let auth = cx.new(|cx| AuthEditor::new(AuthFor::Folder(id.clone()), workspace.clone(), window, cx));
    let hints = HintSource::new(workspace);
    let headers = cx.new(|cx| {
        let options = KvOptions {
            presets: &COMMON_HEADERS,
            example: HEADER_EXAMPLE,
            ..Default::default()
        };
        KvEditor::new(&folder.settings.headers, options, window, cx).hints(hints)
    });
    let variables = cx.new(|cx| {
        let options = KvOptions {
            example: VARIABLE_EXAMPLE,
            ..Default::default()
        };
        // Used as they are, not evaluated (D5): no template hints (Q-4).
        KvEditor::new(&folder.settings.variables, options, window, cx)
    });
    let description = cx.new(|cx| {
        let mut editor = EditorState::new(window, cx)
            .line_number(false)
            .soft_wrap(true)
            .placeholder(t!("docs.placeholder").to_string());
        editor.set_value(folder.settings.description.clone(), window, cx);
        crate::views::fix_editor_search(cx);
        editor
    });
    let source = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder("https://api.example.com/openapi.json")
            .default_value(folder.settings.source.clone().unwrap_or_default())
    });
    let ws_weak = workspace.downgrade();
    let view = cx.new(|cx| {
        let mut subscriptions = vec![cx.subscribe(&source, |this: &mut FolderSettingsView, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.source_error = None;
                this.want_auto_sync = None;
                this.want_sync_interval = None;
                cx.notify();
            }
        })];
        if let Some(ws) = ws_weak.upgrade() {
            subscriptions.push(cx.observe(&ws, |_, _, cx| cx.notify()));
        }
        FolderSettingsView {
            workspace: ws_weak,
            id,
            description,
            source,
            source_error: None,
            want_auto_sync: None,
            want_sync_interval: None,
            editable,
            auth,
            headers,
            variables,
            _subscriptions: subscriptions,
        }
    });
    let title = t!("folder_settings.title", name = folder.name).to_string();
    Some((title, editable, view))
}

/// #66: a folder's auth and headers.
pub fn open_folder_settings(workspace: &Entity<WorkspaceState>, id: ItemId, window: &mut Window, cx: &mut App) {
    let Some((title, editable, view)) = new_folder_settings_view(workspace, id, window, cx) else {
        return;
    };
    window.open_dialog(cx, move |dialog, _, _| {
        let save = {
            let view_for_save = view.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| view_for_save.update(cx, |view, cx| view.save(cx))
        };
        dialog
            .title(title.clone())
            .w(px(660.))
            .child(crate::views::dialog_body(view.clone()))
            .footer(footer("folder-settings-save", editable, save))
            // Enter in a field is the field's, not Save.
            .on_ok(|_, _, _| false)
    });
}

/// #66: the project's auth, for the APIs and folders that inherit.
pub fn open_project_auth(workspace: &Entity<WorkspaceState>, window: &mut Window, cx: &mut App) {
    let editable = workspace.read(cx).editable();
    let auth = cx.new(|cx| AuthEditor::new(AuthFor::Project, workspace.clone(), window, cx));
    let workspace = workspace.downgrade();
    window.open_dialog(cx, move |dialog, _, cx| {
        let save = {
            let (auth, workspace) = (auth.clone(), workspace.clone());
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                if let Some(workspace) = workspace.upgrade() {
                    // The rest of the project's settings stay (D3).
                    let settings = ProjectSettings {
                        auth: auth.read(cx).auth(cx),
                        ..workspace.read(cx).project_settings().clone()
                    };
                    workspace.update(cx, |state, cx| state.set_project_settings(settings, cx));
                }
                true
            }
        };
        dialog
            .title(t!("project_auth.title").to_string())
            .w(px(600.))
            .child(crate::views::dialog_body(section(
                t!("editor.auth").to_string(),
                t!("project_auth.hint").to_string(),
                auth.clone(),
                cx,
            )))
            .footer(footer("project-auth-save", editable, save))
            .on_ok(|_, _, _| false)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Extras;
    use crate::states::AppStore;
    use cyberapi_core::prefs::AppPrefs;
    use cyberapi_http::{HttpEngine, PreparedRequest, SendFuture};
    use cyberapi_store::{MemoryStore, NewCollection};
    use gpui_kit::component::Root;
    use gpui_kit::{AnyWindowHandle, TestAppContext, size};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingEngine(Arc<AtomicUsize>);

    impl HttpEngine for CountingEngine {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            self.0.fetch_add(1, Ordering::Relaxed);
            Box::pin(async {
                Ok(cyberapi_core::response::Exchange {
                    status: 200,
                    body: br#"{"openapi":"3.0.3","paths":{"/v1/items":{"get":{"summary":"List items"}}}}"#.to_vec(),
                    ..Default::default()
                })
            })
        }
    }

    struct Showing(Entity<FolderSettingsView>);

    impl Render for Showing {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    /// B10: FolderSettingsView saves the source URL + periodic auto-sync +
    /// interval, does not carry over auto-sync to a newly typed URL without
    /// opt-in, and prunes the old source when cleared.
    #[gpui_kit::test]
    async fn folder_settings_periodic_sync_requires_per_url_opt_in_and_prunes_on_clear(cx: &mut TestAppContext) {
        let store = MemoryStore::new();
        let storage = store.storage();
        let collection = pollster::block_on(storage.collections().create(NewCollection {
            name: "p".into(),
            ..Default::default()
        }))
        .expect("create");
        let folder = pollster::block_on(
            storage
                .tree()
                .create_folder(collection.id.clone(), None, "orders".into()),
        )
        .expect("folder")
        .item
        .id;

        let sent = Arc::new(AtomicUsize::new(0));
        let engine = Arc::new(CountingEngine(sent.clone()));
        cx.update(|cx| {
            gpui_kit::init(cx);
            AppStore::init(storage, engine, AppPrefs::default(), Extras::default(), cx);
        });
        let workspace = cx.new(|cx| {
            let sender = AppStore::global(cx).sender(cx);
            WorkspaceState::new(sender, collection, cx)
        });
        cx.run_until_parked();

        let mut view = None;
        let folder_id = folder.clone();
        let window: AnyWindowHandle = cx
            .open_window(size(px(800.), px(600.)), |window, cx| {
                let (_, _, built) = new_folder_settings_view(&workspace, folder_id, window, cx).expect("view");
                view = Some(built.clone());
                let showing = cx.new(|_| Showing(built));
                Root::new(showing, window, cx)
            })
            .into();
        let view = view.expect("built");
        cx.run_until_parked();

        // 1. Set URL + enable auto-sync with 5m interval and save.
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.source.update(cx, |s, cx| {
                    s.set_value("https://orders.test/openapi.json", window, cx);
                });
                view.want_auto_sync = Some(true);
                view.want_sync_interval = Some(5);
                assert!(view.save(cx));
            });
        })
        .expect("window");
        cx.run_until_parked();
        assert_eq!(sent.load(Ordering::Relaxed), 1, "initial auto-sync fetched and merged");
        workspace.read_with(cx, |ws, _| {
            assert_eq!(ws.folder_source(&folder), Some("https://orders.test/openapi.json"));
            assert!(ws.auto_syncs_source("https://orders.test/openapi.json"));
            assert_eq!(ws.source_sync_interval("https://orders.test/openapi.json"), 5);
            assert_eq!(ws.tree_data().apis.len(), 1);
        });

        // 2. Typing a different URL without checking auto-sync does NOT carry over auto-sync!
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.source.update(cx, |s, cx| {
                    s.set_value("http://10.0.0.8/openapi.json", window, cx);
                });
                assert_eq!(
                    view.effective_sync("http://10.0.0.8/openapi.json", cx),
                    (false, DEFAULT_SOURCE_SYNC_MINUTES)
                );
                assert!(view.save(cx));
            });
        })
        .expect("window");
        cx.run_until_parked();
        assert_eq!(sent.load(Ordering::Relaxed), 1, "untrusted new URL was not fetched");
        workspace.read_with(cx, |ws, _| {
            assert_eq!(ws.folder_source(&folder), Some("http://10.0.0.8/openapi.json"));
            assert!(!ws.auto_syncs_source("http://10.0.0.8/openapi.json"));
        });

        // 3. Opt in to the second URL, then clear the source URL and save -> prunes opt-in.
        workspace.update(cx, |ws, cx| {
            ws.set_auto_sync_source("http://10.0.0.8/openapi.json".into(), true, 15, cx);
        });
        cx.run_until_parked();
        assert_eq!(sent.load(Ordering::Relaxed), 2);
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.source.update(cx, |s, cx| {
                    s.set_value("", window, cx);
                });
                assert!(view.save(cx));
            });
        })
        .expect("window");
        cx.run_until_parked();
        workspace.read_with(cx, |ws, _| {
            assert_eq!(ws.folder_source(&folder), None);
            assert!(!ws.auto_syncs_source("http://10.0.0.8/openapi.json"));
        });
    }
}
