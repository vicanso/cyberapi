//! F15: project-wide find and replace dialog — searches across URI, query,
//! headers, and body with live preview, then writes via `TreeStore::replace`
//! and offers an Undo notification.

use super::{method_color, method_label};
use crate::i18n::counted;
use crate::states::WorkspaceState;
use crate::views::name_dialog::ok_button;
use cyberapi_core::ItemId;
use cyberapi_core::find_replace::{self, FindScope, ReplaceHit, ReplacePlan};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

pub fn open_find_replace_dialog(
    workspace: Entity<WorkspaceState>,
    folder: Option<ItemId>,
    window: &mut Window,
    cx: &mut App,
) {
    let title_name = {
        let ws = workspace.read(cx);
        folder
            .as_ref()
            .and_then(|id| ws.tree().get(id))
            .map(|node| node.name().to_string())
            .unwrap_or_else(|| ws.collection().name.clone())
    };
    let view = cx.new(|cx| FindReplaceView::new(workspace, folder, window, cx));
    window.open_dialog(cx, move |dialog, _, cx| {
        let has_hits = !view.read(cx).plan.hits.is_empty();
        let apply_view = view.clone();
        dialog
            .title(t!("find_replace.title", name = &title_name).to_string())
            .w(px(640.))
            .child(crate::views::dialog_body(view.clone()))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                    .child(if has_hits {
                        ok_button(
                            "find-replace-apply",
                            t!("find_replace.apply").to_string(),
                            move |_, _, cx| {
                                apply_view.update(cx, |this, cx| this.apply(cx));
                                true
                            },
                        )
                        .into_any_element()
                    } else {
                        Button::new("find-replace-apply")
                            .primary()
                            .disabled(true)
                            .label(t!("find_replace.apply").to_string())
                            .into_any_element()
                    }),
            )
            .on_ok(|_, _, _| false)
    });
}

struct FindReplaceView {
    workspace: Entity<WorkspaceState>,
    folder: Option<ItemId>,
    find: Entity<InputState>,
    replace: Entity<InputState>,
    scope: FindScope,
    plan: ReplacePlan,
    _subscriptions: Vec<Subscription>,
}

impl FindReplaceView {
    fn new(
        workspace: Entity<WorkspaceState>,
        folder: Option<ItemId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let find = cx.new(|cx| {
            let input = InputState::new(window, cx).placeholder(t!("find_replace.find_placeholder").to_string());
            input.focus(window, cx);
            input
        });
        let replace =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("find_replace.replace_placeholder").to_string()));
        let subscriptions = vec![
            cx.subscribe(&find, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh_preview(cx);
                }
            }),
            cx.subscribe(&replace, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh_preview(cx);
                }
            }),
            cx.observe(&workspace, |this, _, cx| {
                this.refresh_preview(cx);
            }),
        ];
        Self {
            workspace,
            folder,
            find,
            replace,
            scope: FindScope::default(),
            plan: ReplacePlan::default(),
            _subscriptions: subscriptions,
        }
    }

    fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        let find_text = self.find.read(cx).value().to_string();
        let replace_text = self.replace.read(cx).value().to_string();
        let data = self.workspace.read(cx).tree_data_with_open(cx);
        self.plan = find_replace::preview(&data, self.folder.as_ref(), &find_text, &replace_text, self.scope);
        cx.notify();
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let plan = self.plan.clone();
        self.workspace.update(cx, |ws, cx| ws.apply_find_replace(plan, cx));
    }

    fn render_hit(index: usize, hit: &ReplaceHit, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (border, fg, muted, mono) = (
            theme.border,
            theme.foreground,
            theme.muted_foreground,
            theme.mono_font_family.clone(),
        );
        let mut parts = Vec::new();
        if hit.uri_matches > 0 {
            parts.push(format!("URI ({})", hit.uri_matches));
        }
        if hit.query_matches > 0 {
            parts.push(format!("Query ({})", hit.query_matches));
        }
        if hit.header_matches > 0 {
            parts.push(format!("Headers ({})", hit.header_matches));
        }
        if hit.body_matches > 0 {
            parts.push(format!("Body ({})", hit.body_matches));
        }
        let where_summary = parts.join(", ");
        let path_label = if hit.place.folders.is_empty() {
            hit.place.name.clone()
        } else {
            format!("{} / {}", hit.place.folders, hit.place.name)
        };

        v_flex()
            .id(("find-replace-hit", index))
            .py_1p5()
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .w(px(44.))
                            .flex_none()
                            .text_xs()
                            .font_bold()
                            .text_color(method_color(hit.method, cx))
                            .child(method_label(hit.method)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .font_medium()
                            .text_color(fg)
                            .child(path_label),
                    )
                    .child(div().flex_none().text_xs().text_color(muted).child(where_summary)),
            )
            .when(hit.uri_matches > 0, |this| {
                this.child(
                    div()
                        .pl_2()
                        .truncate()
                        .text_xs()
                        .font_family(mono)
                        .text_color(muted)
                        .child(format!("{} → {}", hit.before.uri, hit.after.uri)),
                )
            })
            .into_any_element()
    }
}

impl Render for FindReplaceView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let find_empty = self.find.read(cx).value().is_empty();
        let scope = self.scope;

        let summary = if find_empty {
            t!("find_replace.hint").to_string()
        } else if self.plan.hits.is_empty() {
            t!("find_replace.no_matches").to_string()
        } else {
            t!(
                "find_replace.matches",
                occurrences = self.plan.total_occurrences,
                apis = counted("workspace.api", self.plan.hits.len())
            )
            .to_string()
        };

        let rows: Vec<AnyElement> = self
            .plan
            .hits
            .iter()
            .enumerate()
            .map(|(idx, hit)| Self::render_hit(idx, hit, cx))
            .collect();

        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(72.))
                                    .flex_none()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(t!("find_replace.find").to_string()),
                            )
                            .child(div().flex_1().child(Input::new(&self.find).small())),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(72.))
                                    .flex_none()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(t!("find_replace.replace").to_string()),
                            )
                            .child(div().flex_1().child(Input::new(&self.replace).small())),
                    ),
            )
            .child(
                h_flex()
                    .gap_4()
                    .flex_wrap()
                    .items_center()
                    .child(
                        Checkbox::new("fr-scope-uri")
                            .small()
                            .checked(scope.uri)
                            .label(t!("find_replace.scope_uri").to_string())
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.scope.uri = *checked;
                                this.refresh_preview(cx);
                            })),
                    )
                    .child(
                        Checkbox::new("fr-scope-query")
                            .small()
                            .checked(scope.query)
                            .label(t!("find_replace.scope_query").to_string())
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.scope.query = *checked;
                                this.refresh_preview(cx);
                            })),
                    )
                    .child(
                        Checkbox::new("fr-scope-headers")
                            .small()
                            .checked(scope.headers)
                            .label(t!("find_replace.scope_headers").to_string())
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.scope.headers = *checked;
                                this.refresh_preview(cx);
                            })),
                    )
                    .child(
                        Checkbox::new("fr-scope-body")
                            .small()
                            .checked(scope.body)
                            .label(t!("find_replace.scope_body").to_string())
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.scope.body = *checked;
                                this.refresh_preview(cx);
                            })),
                    )
                    .child(
                        Checkbox::new("fr-case-sensitive")
                            .small()
                            .checked(scope.case_sensitive)
                            .label(t!("find_replace.case_sensitive").to_string())
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.scope.case_sensitive = *checked;
                                this.refresh_preview(cx);
                            })),
                    ),
            )
            .child(div().text_xs().text_color(muted).child(summary))
            .when(!rows.is_empty(), |this| {
                this.child(
                    div()
                        .id("find-replace-results")
                        .max_h(px(280.))
                        .overflow_y_scrollbar()
                        .border_1()
                        .border_color(border)
                        .rounded_md()
                        .children(rows),
                )
            })
    }
}
