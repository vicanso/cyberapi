//! F5: an item copied or moved into another project. The project first —
//! one the reader can edit, found by name or pinyin — then the folder
//! there; picking it is the answer, as moving within the project is. A
//! move says before that the history stays behind.
//!
//! On the desktop a server project's copy is listed only as the server
//! says the reader edits it, so not while the server cannot be reached.

use super::move_dialog::destinations;
use crate::i18n;
use crate::states::{AppStore, WorkspaceState};
use crate::views::marked::marked;
use cyberapi_core::access::CollectionRole;
use cyberapi_core::model::Collection;
use cyberapi_core::search::matches_text_or_pinyin;
use cyberapi_core::tree::Tree;
use cyberapi_core::{CollectionId, ItemId};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window,
    div, px,
};
use rust_i18n::t;
use std::collections::HashMap;

pub fn open_transfer_dialog(
    workspace: WeakEntity<WorkspaceState>,
    id: ItemId,
    name: String,
    moving: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| TransferView::new(workspace, id, moving, window, cx));
    let search = view.read(cx).search.clone();
    let title = match moving {
        true => t!("transfer.move_title", name = name),
        false => t!("transfer.copy_title", name = name),
    }
    .to_string();
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(title.clone())
            .w(px(520.))
            .child(crate::views::dialog_body(view.clone()))
            // Picking is the answer: there is no button to confirm.
            .on_ok(|_, _, _| false)
    });
    // After the dialog took focus for itself, so typing starts in the field.
    search.focus_handle(cx).focus(window, cx);
}

/// A folder of the picked project, and the names of the folders above it.
type Folders = Vec<(ItemId, Vec<String>)>;

struct TransferView {
    workspace: WeakEntity<WorkspaceState>,
    id: ItemId,
    moving: bool,
    search: Entity<InputState>,
    /// The projects it can go into; `None` while they are read.
    projects: Option<Result<Vec<Collection>, String>>,
    /// The project picked, and its folders once read.
    picked: Option<(Collection, Option<Result<Folders, String>>)>,
    _reading: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl TransferView {
    fn new(
        workspace: WeakEntity<WorkspaceState>,
        id: ItemId,
        moving: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("transfer.search").to_string())
                .clean_on_escape()
        });
        let subscriptions = vec![cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
        })];
        let current = workspace
            .upgrade()
            .map(|workspace| workspace.read(cx).collection().id.clone());
        let mut view = Self {
            workspace,
            id,
            moving,
            search,
            projects: None,
            picked: None,
            _reading: None,
            _subscriptions: subscriptions,
        };
        view._reading = Some(view.read_projects(current, cx));
        view
    }

    /// The projects the reader can edit, but the one open. A desktop's
    /// server copy is the server's to say of.
    fn read_projects(&self, current: Option<CollectionId>, cx: &mut Context<Self>) -> Task<()> {
        let store = AppStore::global(cx);
        let storage = store.storage().clone();
        let server = store.server().cloned();
        cx.spawn(async move |this, cx| {
            let listed = cx.background_spawn(storage.collections().list()).await;
            let mut projects = match listed {
                Ok(projects) => projects,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.projects = Some(Err(i18n::store_error(&error)));
                        cx.notify();
                    });
                    return;
                }
            };
            // Another account's copies are not this reader's (server-sync
            // §1); the account's own are the server's to say of.
            let Ok((copies, server_storage)) = this.update(cx, |_, cx| match &server {
                Some(server) => {
                    let server = server.read(cx);
                    projects.retain(|project| !server.hides(&project.id));
                    let copies: Vec<CollectionId> = projects
                        .iter()
                        .filter(|project| server.mine(&project.id).is_some())
                        .map(|project| project.id.clone())
                        .collect();
                    (copies, server.storage())
                }
                None => (Vec::new(), None),
            }) else {
                return;
            };
            let mut roles: HashMap<CollectionId, CollectionRole> = HashMap::new();
            if let (false, Some(server_storage)) = (copies.is_empty(), server_storage)
                && let Ok(listed) = cx.background_spawn(server_storage.collections().list()).await
            {
                roles.extend(listed.into_iter().map(|project| (project.id, project.role)));
            }
            projects.retain(|project| {
                Some(&project.id) != current.as_ref()
                    && match copies.contains(&project.id) {
                        true => roles.get(&project.id).is_some_and(CollectionRole::can_edit),
                        false => project.role.can_edit(),
                    }
            });
            projects.sort_by_key(|project| std::cmp::Reverse(project.updated_at));
            let _ = this.update(cx, |this, cx| {
                this.projects = Some(Ok(projects));
                cx.notify();
            });
        })
    }

    /// `project` picked: its folders are read, and the search is theirs.
    fn pick(&mut self, project: Collection, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
        let storage = AppStore::global(cx).storage().clone();
        let id = project.id.clone();
        self.picked = Some((project, None));
        self._reading = Some(cx.spawn(async move |this, cx| {
            let read = cx.background_spawn(storage.tree().load(id)).await;
            let folders = read
                .map(|data| destinations(&Tree::build(&data), &ItemId::from("")))
                .map_err(|error| i18n::store_error(&error));
            let _ = this.update(cx, |this, cx| {
                if let Some((_, folders_now)) = &mut this.picked {
                    *folders_now = Some(folders);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picked = None;
        self._reading = None;
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
        cx.notify();
    }

    /// Into `folder` of the picked project; the dialog closes.
    fn go(&mut self, folder: Option<ItemId>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((project, _)) = self.picked.clone() else {
            return;
        };
        let (id, moving) = (self.id.clone(), self.moving);
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| workspace.transfer(id, project, folder, moving, cx));
        }
        window.close_dialog(cx);
    }

    fn note(text: String, cx: &App) -> AnyElement {
        div()
            .py_3()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text)
            .into_any_element()
    }

    fn project_rows(&self, keyword: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let projects = match &self.projects {
            None => return vec![Self::note(t!("transfer.loading").to_string(), cx)],
            Some(Err(why)) => return vec![Self::note(why.clone(), cx)],
            Some(Ok(projects)) => projects,
        };
        if projects.is_empty() {
            return vec![Self::note(t!("transfer.none").to_string(), cx)];
        }
        let theme = cx.theme();
        let (muted, hover, radius) = (theme.muted_foreground, theme.list_hover, theme.radius);
        let rows: Vec<AnyElement> = projects
            .iter()
            .filter(|project| keyword.is_empty() || matches_text_or_pinyin(&project.name, keyword))
            .enumerate()
            .map(|(index, project)| {
                let picked = project.clone();
                h_flex()
                    .id(("transfer-project", index))
                    .flex_none()
                    .h(px(30.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .rounded(radius)
                    .text_sm()
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .child(div().flex_none().child(marked(&project.name, keyword, cx)))
                    .when(!project.description.trim().is_empty(), |this| {
                        this.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(muted)
                                .child(project.description.clone()),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| this.pick(picked.clone(), window, cx)))
                    .into_any_element()
            })
            .collect();
        if rows.is_empty() {
            return vec![Self::note(t!("workspace.no_match", keyword = keyword).to_string(), cx)];
        }
        rows
    }

    fn folder_rows(&self, folders: &Folders, keyword: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme();
        let (muted, hover, radius) = (theme.muted_foreground, theme.list_hover, theme.radius);
        let top = (None, vec![t!("workspace.move_top").to_string()]);
        let rows: Vec<AnyElement> = std::iter::once(top)
            .chain(
                folders
                    .iter()
                    .map(|(folder, path)| (Some(folder.clone()), path.clone())),
            )
            .filter(|(folder, path)| {
                keyword.is_empty()
                    || folder.is_some() && path.last().is_some_and(|name| matches_text_or_pinyin(name, keyword))
            })
            .enumerate()
            .map(|(index, (folder, path))| {
                let (name, above) = path
                    .split_last()
                    .map(|(last, rest)| (last.clone(), rest.join(" / ")))
                    .unwrap_or_default();
                let icon = match folder {
                    Some(_) => IconName::FolderClosed,
                    None => IconName::Inbox,
                };
                h_flex()
                    .id(("transfer-folder", index))
                    .flex_none()
                    .h(px(30.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .rounded(radius)
                    .text_sm()
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .child(Icon::new(icon).small().text_color(muted))
                    .child(div().flex_none().child(marked(&name, keyword, cx)))
                    .when(!above.is_empty(), |this| {
                        this.child(div().min_w_0().truncate().text_xs().text_color(muted).child(above))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| this.go(folder.clone(), window, cx)))
                    .into_any_element()
            })
            .collect();
        if rows.is_empty() {
            return vec![Self::note(t!("workspace.no_match", keyword = keyword).to_string(), cx)];
        }
        rows
    }
}

impl Render for TransferView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let keyword = self.search.read(cx).value().trim().to_string();
        let muted = cx.theme().muted_foreground;
        let (header, rows) = match self.picked.clone() {
            None => (None, self.project_rows(&keyword, cx)),
            Some((project, folders)) => {
                let header = h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("transfer-back")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ChevronLeft)
                            .label(t!("transfer.back").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(t!("transfer.into", project = project.name).to_string()),
                    );
                let rows = match &folders {
                    None => vec![Self::note(t!("transfer.loading").to_string(), cx)],
                    Some(Err(why)) => vec![Self::note(why.clone(), cx)],
                    Some(Ok(folders)) => self.folder_rows(folders, &keyword, cx),
                };
                (Some(header), rows)
            }
        };
        v_flex()
            .gap_2()
            .children(header)
            .child(
                div().text_xs().text_color(muted).child(
                    match self.moving {
                        true => t!("transfer.move_hint"),
                        false => t!("transfer.copy_hint"),
                    }
                    .to_string(),
                ),
            )
            .child(Input::new(&self.search).prefix(Icon::new(IconName::Search).text_color(muted)))
            .child(
                v_flex()
                    .id("transfer-list")
                    .max_h(px(360.))
                    .overflow_y_scrollbar()
                    .children(rows),
            )
    }
}
