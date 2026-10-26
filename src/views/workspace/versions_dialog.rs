//! G4: a server project's versions — what it was before each time it was
//! replaced whole (a sync, an upload, a version put back) — newest first,
//! each with when, by whom, and how much it held. An editor puts one back
//! once the project has nothing unsynced; what it replaces is kept as a
//! version too, so putting one back can be undone.

use crate::i18n;
use crate::states::WorkspaceState;
use crate::views::inline_confirm::inline_confirm;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

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
    _observe: Subscription,
}

impl VersionsView {
    fn new(workspace: Entity<WorkspaceState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&workspace, |_, _, cx| cx.notify());
        Self {
            workspace,
            confirming: None,
            _observe: observe,
        }
    }
}

impl Render for VersionsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
    }
}
