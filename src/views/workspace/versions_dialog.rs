//! G4: a server project's versions — what it was before each time it was
//! replaced whole (a sync, an upload, a version put back) — newest first,
//! each with when, by whom, and how much it held. An editor puts one back
//! once the project has nothing unsynced; what it replaces is kept as a
//! version too, so putting one back can be undone.
//!
//! G8: anyone compares a version with the project as the server has it now,
//! or with the version after it — what that sync changed: the folders and
//! APIs added, removed, renamed, moved or edited, the variables and
//! environments likewise; an edited API's request line by line.

use crate::i18n;
use crate::states::WorkspaceState;
use crate::views::inline_confirm::inline_confirm;
use cyberapi_core::ItemId;
use cyberapi_core::diff::{self, Row};
use cyberapi_core::model::VariableCategory;
use cyberapi_core::sync::ProjectConfig;
use cyberapi_core::versions::{self, Change, ItemChange, NamedChange, ProjectChanges};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::{ActiveTheme as _, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, px,
};
use rust_i18n::t;
use std::sync::Arc;

/// The most lines of a request's comparison shown.
const SHOWN_ROWS: usize = 400;

/// Opens the list, reading it again.
pub fn open_versions_dialog(workspace: Entity<WorkspaceState>, window: &mut Window, cx: &mut App) {
    workspace.update(cx, |workspace, cx| workspace.load_versions(cx));
    let view = cx.new(|cx| VersionsView::new(workspace, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(t!("versions.title").to_string())
            .w(px(560.))
            .on_ok(|_, _, _| false)
            .child(view.clone())
            .footer(
                DialogFooter::new().child(
                    Button::new("versions-close")
                        .label(t!("common.close").to_string())
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}

struct VersionsView {
    workspace: Entity<WorkspaceState>,
    /// The version being asked about before it is put back.
    confirming: Option<String>,
    /// G8: the version compared, in place of the list.
    comparing: Option<Comparing>,
    _observe: Subscription,
}

/// G8: two versions of the project, and what changed between them.
struct Compared {
    old: ProjectConfig,
    new: ProjectConfig,
    changes: ProjectChanges,
}

/// G8: a version being compared.
struct Comparing {
    /// Its place in the list, newest first.
    at: usize,
    /// Compared with the version after it, rather than the project now.
    with_next: bool,
    found: Option<Result<Arc<Compared>, String>>,
    /// The edited API whose request is shown, and the comparison of it.
    open: Option<(ItemId, Arc<diff::Compared>)>,
    _reading: Task<()>,
}

impl VersionsView {
    fn new(workspace: Entity<WorkspaceState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&workspace, |_, _, cx| cx.notify());
        Self {
            workspace,
            confirming: None,
            comparing: None,
            _observe: observe,
        }
    }

    /// G8: the version at `at` compared with the project now, or with the
    /// version after it.
    fn compare(&mut self, at: usize, with_next: bool, cx: &mut Context<Self>) {
        let workspace = self.workspace.read(cx);
        let Some(Ok(versions)) = workspace.versions() else {
            return;
        };
        let Some(version) = versions.get(at) else { return };
        // The newest's next version is the project now.
        let with_next = with_next && at > 0;
        let after = with_next.then(|| versions[at - 1].id.clone());
        let Some(reading) = workspace.compared_versions(version.id.clone(), after, cx) else {
            return;
        };
        let reading = cx.spawn(async move |this, cx| {
            let found = match reading.await {
                Ok((old, new)) => Ok(cx
                    .background_spawn(async move {
                        let changes = versions::changes(&old, &new);
                        Arc::new(Compared { old, new, changes })
                    })
                    .await),
                Err(error) => Err(i18n::store_error(&error)),
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(comparing) = &mut this.comparing {
                    comparing.found = Some(found);
                }
                cx.notify();
            });
        });
        self.confirming = None;
        self.comparing = Some(Comparing {
            at,
            with_next,
            found: None,
            open: None,
            _reading: reading,
        });
        cx.notify();
    }

    /// G8: an edited API's request shown line by line, or no longer.
    fn toggle_request(&mut self, id: ItemId, cx: &mut Context<Self>) {
        let Some(comparing) = &mut self.comparing else { return };
        if comparing.open.as_ref().is_some_and(|(open, _)| open == &id) {
            comparing.open = None;
        } else if let Some(Ok(found)) = &comparing.found {
            let text = |config: &ProjectConfig| {
                config
                    .apis
                    .iter()
                    .find(|api| api.id == id)
                    .map(|api| versions::request_text(&api.request))
                    .unwrap_or_default()
            };
            let compared = diff::compare(&text(&found.old), &text(&found.new));
            comparing.open = Some((id, Arc::new(compared)));
        }
        cx.notify();
    }
}

/// G8: what became of something, said briefly, and its colour.
fn change_words(change: &Change, cx: &App) -> (String, Hsla) {
    let theme = cx.theme();
    match change {
        Change::Added => (t!("versions.added").to_string(), theme.success),
        Change::Removed => (t!("versions.removed").to_string(), theme.danger),
        Change::Changed {
            renamed_from,
            moved,
            edited,
        } => {
            let mut words = Vec::new();
            if let Some(name) = renamed_from {
                words.push(t!("versions.renamed_from", name = name).to_string());
            }
            if *moved {
                words.push(t!("versions.moved").to_string());
            }
            if *edited {
                words.push(t!("versions.edited").to_string());
            }
            (words.join(&t!("workspace.list_separator")), theme.warning)
        }
    }
}

/// G8: a line of what changed: its kind, its name, what became of it.
fn change_line(kind: String, name: String, change: &Change, cx: &App) -> gpui_kit::Div {
    let (words, color) = change_words(change, cx);
    h_flex()
        .gap_2()
        .py_0p5()
        .text_sm()
        .child(
            div()
                .w(px(72.))
                .flex_none()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(kind),
        )
        .child(div().flex_1().min_w_0().truncate().child(name))
        .child(div().flex_none().text_xs().text_color(color).child(words))
}

fn category_name(category: Option<VariableCategory>) -> String {
    match category {
        Some(VariableCategory::Environment) => t!("versions.kind_variable"),
        Some(VariableCategory::Custom) => t!("versions.kind_custom"),
        Some(VariableCategory::GlobalRequestHeader) => t!("versions.kind_header"),
        None => t!("versions.kind_environment"),
    }
    .to_string()
}

fn named_line(change: &NamedChange, cx: &App) -> gpui_kit::Div {
    let name = match &change.environment {
        Some(environment) => format!("{} ({environment})", change.name),
        None => change.name.clone(),
    };
    change_line(category_name(change.category), name, &change.change, cx)
}

impl VersionsView {
    /// G8: what changed, in place of the list.
    fn render_comparing(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(comparing) = &self.comparing else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let note = |text: String| div().py_6().text_sm().text_color(muted).child(text).into_any_element();
        let when = match self.workspace.read(cx).versions() {
            Some(Ok(versions)) => versions
                .get(comparing.at)
                .map(|version| {
                    version
                        .saved_at
                        .get()
                        .with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default(),
            _ => String::new(),
        };
        let (at, with_next) = (comparing.at, comparing.with_next);
        let header = h_flex()
            .gap_2()
            .items_center()
            .child(
                Button::new("versions-back")
                    .ghost()
                    .xsmall()
                    .icon(gpui_kit::assets::IconName::ChevronLeft)
                    .label(t!("versions.back").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.comparing = None;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .child(t!("versions.comparing", time = when).to_string()),
            )
            .child(
                [t!("versions.with_now"), t!("versions.with_next")]
                    .into_iter()
                    .enumerate()
                    .filter(|(index, _)| *index == 0 || at > 0)
                    .fold(
                        ButtonGroup::new("versions-against").compact().xsmall().outline(),
                        |group, (index, label)| {
                            group.child(
                                Button::new(("versions-against", index))
                                    .label(label.to_string())
                                    .selected((index == 1) == with_next),
                            )
                        },
                    )
                    .on_click(cx.listener(move |this, picks: &Vec<usize>, _, cx| {
                        let next = picks.first() == Some(&1);
                        if next != with_next {
                            this.compare(at, next, cx);
                        }
                    })),
            );
        let body = match &comparing.found {
            None => note(t!("versions.loading").to_string()),
            Some(Err(why)) => note(why.clone()),
            Some(Ok(found)) if found.changes.is_empty() => note(t!("versions.unchanged").to_string()),
            Some(Ok(found)) => {
                let changes = &found.changes;
                let mut lines: Vec<AnyElement> = Vec::new();
                let project = t!("versions.kind_project").to_string();
                if let Some(was) = &changes.renamed_from {
                    lines.push(
                        change_line(
                            project.clone(),
                            found.new.name.clone(),
                            &Change::Changed {
                                renamed_from: Some(was.clone()),
                                moved: false,
                                edited: false,
                            },
                            cx,
                        )
                        .into_any_element(),
                    );
                }
                for (changed, key) in [
                    (changes.description, "versions.description"),
                    (changes.settings, "versions.settings"),
                    (changes.reordered, "versions.reordered"),
                ] {
                    if changed {
                        lines.push(
                            h_flex()
                                .gap_2()
                                .py_0p5()
                                .text_sm()
                                .child(
                                    div()
                                        .w(px(72.))
                                        .flex_none()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(project.clone()),
                                )
                                .child(div().child(t!(key).to_string()))
                                .into_any_element(),
                        );
                    }
                }
                for (index, item) in changes.items.iter().enumerate() {
                    lines.push(self.item_line(index, item, cx));
                }
                for change in changes.environments.iter().chain(&changes.variables) {
                    lines.push(named_line(change, cx).into_any_element());
                }
                v_flex().children(lines).into_any_element()
            }
        };
        v_flex()
            .gap_2()
            .child(header)
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("versions.compare_hint").to_string()),
            )
            .child(
                div()
                    .id("versions-changes")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .border_t_1()
                    .border_color(border)
                    .pt_1()
                    .child(body),
            )
            .into_any_element()
    }

    /// G8: a folder or an API that changed; an edited API opens its
    /// request, compared, under it.
    fn item_line(&self, index: usize, item: &ItemChange, cx: &mut Context<Self>) -> AnyElement {
        let kind = if item.folder {
            t!("versions.kind_folder")
        } else {
            t!("versions.kind_api")
        }
        .to_string();
        let line = change_line(kind, item.name.clone(), &item.change, cx);
        let opens = !item.folder && matches!(item.change, Change::Changed { edited: true, .. });
        if !opens {
            return line.into_any_element();
        }
        let open = self
            .comparing
            .as_ref()
            .and_then(|comparing| comparing.open.as_ref())
            .filter(|(open, _)| open == &item.id)
            .map(|(_, compared)| compared.clone());
        let id = item.id.clone();
        let theme = cx.theme();
        let rows: Vec<AnyElement> = open
            .iter()
            .flat_map(|compared| compared.rows.iter().take(SHOWN_ROWS))
            .map(|row| match row {
                Row::Folded(lines) => div()
                    .px_2()
                    .bg(theme.muted)
                    .text_color(theme.muted_foreground)
                    .child(t!("compare.folded", count = lines).to_string())
                    .into_any_element(),
                Row::Line { change, text, .. } => {
                    let (sign, background) = match change {
                        diff::Change::Same => (" ", None),
                        diff::Change::Removed => ("-", Some(theme.danger.opacity(0.15))),
                        diff::Change::Added => ("+", Some(theme.success.opacity(0.15))),
                    };
                    h_flex()
                        .when_some(background, |row, background| row.bg(background))
                        .child(div().w(px(16.)).flex_none().pl_1().child(sign))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .child(text.clone()),
                        )
                        .into_any_element()
                }
            })
            .collect();
        let cut = open
            .as_ref()
            .is_some_and(|compared| compared.cut || compared.rows.len() > SHOWN_ROWS);
        v_flex()
            .child(
                line.id(("versions-item", index))
                    .cursor_pointer()
                    .hover(|row| row.bg(theme.accent))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_request(id.clone(), cx))),
            )
            .when(open.is_some(), |this| {
                this.child(
                    v_flex()
                        .my_1()
                        .text_xs()
                        .font_family(theme.mono_font_family.clone())
                        .border_1()
                        .border_color(theme.border)
                        .rounded(theme.radius)
                        .children(rows)
                        .when(cut, |this| {
                            this.child(
                                div()
                                    .px_2()
                                    .text_color(theme.muted_foreground)
                                    .child(t!("versions.rows_cut").to_string()),
                            )
                        }),
                )
            })
            .into_any_element()
    }
}

impl Render for VersionsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.comparing.is_some() {
            return self.render_comparing(cx);
        }
        let muted = cx.theme().muted_foreground;
        let note = |text: String| div().py_6().text_sm().text_color(muted).child(text).into_any_element();
        let workspace = self.workspace.read(cx);
        let can_restore = workspace.editable() && !workspace.unsynced();
        let hint = if !workspace.editable() {
            t!("versions.hint_viewer")
        } else if workspace.unsynced() {
            t!("versions.hint_unsynced")
        } else {
            t!("versions.hint")
        };
        let body = match workspace.versions() {
            None => note(t!("versions.loading").to_string()),
            Some(Err(error)) => note(i18n::store_error(error)),
            Some(Ok(versions)) if versions.is_empty() => note(t!("versions.none").to_string()),
            Some(Ok(versions)) => {
                let border = cx.theme().border;
                let rows: Vec<AnyElement> = versions
                    .iter()
                    .enumerate()
                    .map(|(at, version)| {
                        let when = version
                            .saved_at
                            .get()
                            .with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M")
                            .to_string();
                        let held = t!("versions.held", apis = version.apis, folders = version.folders).to_string();
                        let id = version.id.clone();
                        let confirming = self.confirming.as_deref() == Some(version.id.as_str());
                        let row = h_flex()
                            .id(("version", at))
                            .gap_3()
                            .py_1p5()
                            .items_center()
                            .text_sm()
                            .child(div().w(px(130.)).flex_none().child(when))
                            .child(div().flex_1().min_w_0().truncate().child(version.by.clone()))
                            .child(div().text_xs().text_color(muted).child(held))
                            // G8: anyone compares.
                            .child(
                                Button::new(("version-compare", at))
                                    .ghost()
                                    .xsmall()
                                    .label(t!("versions.compare").to_string())
                                    .on_click(cx.listener(move |this, _, _, cx| this.compare(at, false, cx))),
                            )
                            .when(can_restore, |row| {
                                let id = id.clone();
                                row.child(
                                    Button::new(("version-restore", at))
                                        .ghost()
                                        .xsmall()
                                        .label(t!("versions.restore").to_string())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.confirming = Some(id.clone());
                                            cx.notify();
                                        })),
                                )
                            });
                        let confirm = confirming.then(|| {
                            let (workspace, id) = (self.workspace.clone(), version.id.clone());
                            let view = cx.entity().downgrade();
                            let back = view.clone();
                            inline_confirm(
                                t!("versions.confirm_title").to_string(),
                                t!("versions.confirm").to_string(),
                                None,
                                t!("versions.restore").to_string(),
                                move |_, cx| {
                                    let _ = back.update(cx, |this, cx| {
                                        this.confirming = None;
                                        cx.notify();
                                    });
                                },
                                move |_, cx| {
                                    workspace.update(cx, |workspace, cx| workspace.restore_version(id.clone(), cx));
                                    let _ = view.update(cx, |this, cx| {
                                        this.confirming = None;
                                        cx.notify();
                                    });
                                },
                                cx,
                            )
                        });
                        v_flex()
                            .border_t_1()
                            .border_color(border)
                            .child(row)
                            .children(confirm)
                            .into_any_element()
                    })
                    .collect();
                v_flex().children(rows).into_any_element()
            }
        };
        v_flex()
            .gap_2()
            .child(div().text_xs().text_color(muted).child(hint.to_string()))
            .child(
                div()
                    .id("versions-list")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .child(body),
            )
            .into_any_element()
    }
}
