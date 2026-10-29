//! B7: syncing the project from the address it was imported from. What the
//! address holds is fetched again and what syncing would do is told first
//! — the APIs added, updated and left as they are, and the project's that
//! the source no longer has, which stay — then it is merged as importing
//! again does when it updates what the project has.

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
    fetching: Option<Task<()>>,
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
                    // What was fetched is told for its own address only.
                    InputEvent::Change => {
                        let typed = this.typed(cx);
                        if this.fetched.as_ref().is_some_and(|fetched| fetched.url != typed) {
                            this.fetched = None;
                            cx.notify();
                        }
                    }
                    _ => {}
                }
            }),
        ];
        let mut view = Self {
            workspace,
            url,
            kept: kept.is_some(),
            fetching: None,
            fetched: None,
            error: None,
            _subscriptions: subscriptions,
        };
        // The source the project keeps is fetched at once: syncing is what
        // the reader opened this for.
        if view.kept {
            view.fetch(window, cx);
        }
        view
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
        self.fetching = Some(cx.spawn_in(window, async move |this, cx| {
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
                match worked {
                    Ok(fetched) => this.fetched = Some(fetched),
                    Err(why) => this.error = Some(why),
                }
                cx.notify();
            });
        }));
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
        let danger = cx.theme().danger;
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
