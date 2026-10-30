//! B7: syncing the project from the address it was imported from. What the
//! address holds is fetched again and what syncing would do is told first
//! — the APIs added, updated and left as they are, and the project's that
//! the source no longer has, which stay — then it is merged as importing
//! again does when it updates what the project has.
//!
//! Q-1: the address is project data, which any editor sets; it is fetched
//! when the reader asks, not as the dialog opens, and one on this machine
//! or a private network is pointed out first.

use super::import_dialog::address;
use crate::i18n;
use crate::states::{AppStore, WorkspaceState};
use crate::views::name_dialog::ok_button;
use cyberapi_core::import::merge::{self, Preview};
use cyberapi_core::import::{self, ImportBundle};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Task, WeakEntity, Window, div, px,
};
use rust_i18n::t;

/// How many of the APIs the source no longer has are named.
const NAMED: usize = 8;

pub fn open_source_dialog(workspace: Entity<WorkspaceState>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| SourceView::new(workspace.downgrade(), window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        let sync = {
            let view = view.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| view.update(cx, |view, cx| view.sync(window, cx))
        };
        dialog
            .title(t!("source.title").to_string())
            .w(px(600.))
            .child(crate::views::dialog_body(view.clone()))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                    .child(ok_button("source-sync", t!("source.sync").to_string(), sync)),
            )
            // Enter in the address fetches; it is not Sync.
            .on_ok(|_, _, _| false)
    });
}

/// What the address held, and what syncing it would do.
struct Fetched {
    url: String,
    bundle: ImportBundle,
    preview: Preview,
    /// The first of the APIs the source no longer has, by name.
    missing: Vec<String>,
}

struct SourceView {
    workspace: WeakEntity<WorkspaceState>,
    url: Entity<InputState>,
    /// Whether the project keeps a source already.
    kept: bool,
    /// The address being fetched, and the fetch: dropped, it stops.
    fetching: Option<(String, Task<()>)>,
    fetched: Option<Fetched>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl SourceView {
    fn new(workspace: WeakEntity<WorkspaceState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let kept = workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).source().map(|source| source.url.clone()));
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://api.example.com/openapi.json")
                .default_value(kept.clone().unwrap_or_default())
        });
        let subscriptions = vec![
            cx.subscribe_in(&url, window, |this, _, event: &InputEvent, window, cx| {
                match event {
                    InputEvent::PressEnter { .. } => this.fetch(window, cx),
                    // What was fetched is told for its own address only, and
                    // a fetch of another stops (BUG-13).
                    InputEvent::Change => {
                        let typed = this.typed(cx);
                        if this.fetched.as_ref().is_some_and(|fetched| fetched.url != typed) {
                            this.fetched = None;
                            cx.notify();
                        }
                        if this.fetching.as_ref().is_some_and(|(url, _)| *url != typed) {
                            this.fetching = None;
                            cx.notify();
                        }
                    }
                    _ => {}
                }
            }),
        ];
        Self {
            workspace,
            url,
            kept: kept.is_some(),
            fetching: None,
            fetched: None,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn typed(&self, cx: &App) -> String {
        self.url.read(cx).value().trim().to_string()
    }

    /// What the address holds, and what syncing it would do, worked out
    /// off the main thread.
    fn fetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.typed(cx);
        let Some(url) = address(&typed).map(str::to_string) else {
            self.error = Some(t!("source.not_address").to_string());
            self.fetched = None;
            cx.notify();
            return;
        };
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let (tree, folder) = {
            let workspace = workspace.read(cx);
            (workspace.tree_data().clone(), workspace.source_folder())
        };
        let fetched = AppStore::global(cx).fetch_text(url.clone(), cx);
        self.error = None;
        self.fetched = None;
        let (fetching, typed_url) = (url.clone(), url.clone());
        let task = cx.spawn_in(window, async move |this, cx| {
            let worked = match fetched.await {
                Ok(text) => {
                    cx.background_spawn(async move {
                        let bundle = import::parse(&text).map_err(|failure| {
                            t!("import.unreadable", why = i18n::import_error(&failure)).to_string()
                        })?;
                        let preview = merge::preview(&tree, folder.as_ref(), &bundle.items);
                        let missing = preview
                            .missing
                            .iter()
                            .take(NAMED)
                            .filter_map(|id| tree.apis.iter().find(|api| &api.id == id))
                            .map(|api| api.name.clone())
                            .collect();
                        Ok(Fetched {
                            url,
                            bundle,
                            preview,
                            missing,
                        })
                    })
                    .await
                }
                Err(why) => Err(t!("import.fetch_failed", why = why).to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.fetching = None;
                // Another address typed meanwhile: this one's is not its.
                if address(&this.typed(cx)) != Some(fetching.as_str()) {
                    return cx.notify();
                }
                match worked {
                    Ok(fetched) => this.fetched = Some(fetched),
                    Err(why) => this.error = Some(why),
                }
                cx.notify();
            });
        });
        self.fetching = Some((typed_url, task));
        cx.notify();
    }

    /// Syncs what was fetched, and closes; fetches first when nothing for
    /// this address was.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(fetched) = self.fetched.take() else {
            if self.fetching.is_none() {
                self.fetch(window, cx);
            }
            return false;
        };
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                workspace.sync_from_source(fetched.url, fetched.bundle, cx);
            });
        }
        true
    }

    fn forget(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| workspace.forget_source(cx));
        }
        window.close_dialog(cx);
    }
}

/// What syncing would do, said in lines.
fn told(fetched: &Fetched) -> Vec<String> {
    let preview = &fetched.preview;
    let (apis, folders) = preview.added;
    let mut lines = Vec::new();
    if apis == 0 && folders == 0 && preview.updated == 0 {
        lines.push(t!("source.up_to_date").to_string());
    } else {
        lines.push(
            t!(
                "source.would",
                added = i18n::counted("workspace.api", apis),
                folders = i18n::counted("import.folder", folders),
                updated = i18n::counted("workspace.api", preview.updated)
            )
            .to_string(),
        );
    }
    if preview.same > 0 {
        lines.push(t!("source.same", apis = i18n::counted("workspace.api", preview.same)).to_string());
    }
    if !preview.missing.is_empty() {
        let mut names = fetched.missing.join(t!("source.name_separator").as_ref());
        if preview.missing.len() > fetched.missing.len() {
            names += &t!("source.and_more");
        }
        lines.push(
            t!(
                "source.missing",
                apis = i18n::counted("workspace.api", preview.missing.len()),
                names = names
            )
            .to_string(),
        );
    }
    lines
}

impl Render for SourceView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let (danger, warning) = (cx.theme().danger, cx.theme().warning);
        let typed = self.typed(cx);
        let local = address(&typed).is_some_and(import::local_address);
        let idle = self.fetching.is_none() && self.fetched.is_none() && self.error.is_none();
        v_flex()
            .gap_3()
            .child(div().text_sm().text_color(muted).child(t!("source.about").to_string()))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().child(Input::new(&self.url).small()))
                    .child(
                        Button::new("source-fetch")
                            .outline()
                            .small()
                            .loading(self.fetching.is_some())
                            .label(t!("import.fetch").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.fetch(window, cx))),
                    ),
            )
            .when(local, |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(warning)
                        .child(t!("source.local").to_string()),
                )
            })
            .when(idle && address(&typed).is_some(), |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(t!("source.fetch_first").to_string()),
                )
            })
            .when_some(self.error.clone(), |this, why| {
                this.child(div().text_sm().text_color(danger).child(why))
            })
            .when_some(self.fetched.as_ref(), |this, fetched| {
                this.child(
                    v_flex()
                        .gap_1()
                        .text_sm()
                        .children(told(fetched).into_iter().map(|line| div().child(line))),
                )
            })
            .when(self.kept, |this| {
                this.child(
                    h_flex().child(
                        Button::new("source-forget")
                            .ghost()
                            .xsmall()
                            .label(t!("source.forget").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.forget(window, cx))),
                    ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Extras, SignOut};
    use cyberapi_core::model::{ImportSource, ProjectSettings};
    use cyberapi_core::prefs::AppPrefs;
    use cyberapi_http::{HttpEngine, PreparedRequest, SendFuture};
    use cyberapi_store::{MemoryStore, NewCollection};
    use gpui_kit::component::Root;
    use gpui_kit::{AnyWindowHandle, TestAppContext, size};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts what it is asked to send, and answers nothing it can read.
    struct Counting(Arc<AtomicUsize>);

    impl HttpEngine for Counting {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            self.0.fetch_add(1, Ordering::Relaxed);
            Box::pin(async {
                Ok(cyberapi_core::response::Exchange {
                    status: 200,
                    body: b"{}".to_vec(),
                    ..Default::default()
                })
            })
        }
    }

    /// What the dialog shows.
    struct Showing(Entity<SourceView>);

    impl Render for Showing {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    /// Q-1: the source a project keeps is not fetched as the dialog opens,
    /// only once the reader asks; one on a private network is pointed out.
    #[gpui_kit::test]
    async fn the_source_is_fetched_when_asked(cx: &mut TestAppContext) {
        let store = MemoryStore::new();
        let storage = store.storage();
        let collection = pollster::block_on(storage.collections().create(NewCollection {
            name: "p".into(),
            ..Default::default()
        }))
        .expect("create");
        let source = ImportSource {
            url: "http://10.0.0.5/openapi.json".into(),
            folder: None,
        };
        pollster::block_on(storage.tree().set_project_settings(
            collection.id.clone(),
            ProjectSettings {
                source: Some(source),
                ..Default::default()
            },
        ))
        .expect("a source");
        let sent = Arc::new(AtomicUsize::new(0));
        let sign_out: SignOut = Arc::new(|| Box::pin(async {}));
        let engine = Arc::new(Counting(sent.clone()));
        cx.update(|cx| {
            gpui_kit::init(cx);
            AppStore::init(
                storage,
                engine,
                AppPrefs::default(),
                Extras {
                    sign_out: Some(sign_out),
                    ..Extras::default()
                },
                cx,
            );
        });
        let workspace = cx.new(|cx| {
            let sender = AppStore::global(cx).sender(cx);
            WorkspaceState::new(sender, collection, cx)
        });
        cx.run_until_parked();
        let mut view = None;
        let window: AnyWindowHandle = cx
            .open_window(size(px(800.), px(600.)), |window, cx| {
                let source = cx.new(|cx| SourceView::new(workspace.downgrade(), window, cx));
                view = Some(source.clone());
                let showing = cx.new(|_| Showing(source));
                Root::new(showing, window, cx)
            })
            .into();
        let view = view.expect("built");
        cx.run_until_parked();
        assert_eq!(sent.load(Ordering::Relaxed), 0, "not fetched as it opens");
        let typed = cx.read(|cx| view.read(cx).typed(cx));
        assert_eq!(typed, "http://10.0.0.5/openapi.json", "its address, shown");
        assert!(import::local_address(&typed), "pointed out");

        cx.update_window(window, |_, window, cx| {
            view.update(cx, |view, cx| view.fetch(window, cx))
        })
        .expect("the window is open");
        cx.run_until_parked();
        assert_eq!(sent.load(Ordering::Relaxed), 1, "fetched once asked");
    }
}
