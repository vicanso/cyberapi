//! M1: import report dialog listing scripts, unrecognized templates,
//! unsupported authentication kinds and unsupported request bodies that could
//! not be imported as-is, grouped by target location.

use crate::i18n;
use cyberapi_core::import::{Lost, LostGroup, LostKind, ScriptPhase, group_lost};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use rust_i18n::t;
use std::collections::HashSet;
use std::sync::Arc;

/// Maximum bytes of a script displayed inline when expanded, so huge scripts
/// do not stall main-thread layout; Copy always writes the full source.
const SCRIPT_PREVIEW_LIMIT: usize = 16 * 1024;

/// Opens the import report dialog when `lost` is non-empty.
pub fn open_import_report_dialog(lost: Arc<Vec<Lost>>, window: &mut Window, cx: &mut App) {
    if lost.is_empty() {
        return;
    }
    let view = cx.new(|_| ImportReportView::new(&lost));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(t!("import.report_title").to_string())
            .w(px(640.))
            .on_ok(|_, _, _| false)
            .child(crate::views::dialog_body(view.clone()))
            .footer(
                DialogFooter::new().child(
                    Button::new("import-report-close")
                        .primary()
                        .label(t!("common.close").to_string())
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}

struct ImportReportView {
    groups: Vec<LostGroup>,
    total: usize,
    expanded: HashSet<(usize, usize)>,
}

impl ImportReportView {
    fn new(lost: &[Lost]) -> Self {
        Self {
            groups: group_lost(lost),
            total: lost.len(),
            expanded: HashSet::new(),
        }
    }

    fn toggle(&mut self, key: (usize, usize), cx: &mut Context<Self>) {
        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
        cx.notify();
    }

    fn script_phase_label(phase: ScriptPhase) -> String {
        match phase {
            ScriptPhase::PreRequest => t!("import.lost_script_pre"),
            ScriptPhase::PostResponse => t!("import.lost_script_post"),
            ScriptPhase::Test => t!("import.lost_script_test"),
            ScriptPhase::Other => t!("import.lost_script_generic"),
        }
        .to_string()
    }

    fn render_item(
        &self,
        group_idx: usize,
        item_idx: usize,
        item: &LostKind,
        last: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, accent, secondary, mono, radius) = (
            theme.border,
            theme.muted_foreground,
            theme.accent,
            theme.secondary,
            theme.mono_font_family.clone(),
            theme.radius,
        );
        let row = match item {
            LostKind::Script { phase, source } => {
                let key = (group_idx, item_idx);
                let is_expanded = self.expanded.contains(&key);
                let lines = source.lines().count().max(1);
                let full_source = source.clone();
                let preview = if is_expanded {
                    if source.len() > SCRIPT_PREVIEW_LIMIT {
                        Some(format!(
                            "{}\n{}",
                            &source[..source.floor_char_boundary(SCRIPT_PREVIEW_LIMIT)],
                            t!("import.report_script_truncated")
                        ))
                    } else {
                        Some(source.clone())
                    }
                } else {
                    None
                };
                v_flex()
                    .child(
                        h_flex()
                            .gap_2()
                            .px_3()
                            .py_1p5()
                            .items_center()
                            .child(
                                h_flex()
                                    .id(ElementId::Name(
                                        format!("import-report-toggle-{group_idx}-{item_idx}").into(),
                                    ))
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1p5()
                                    .items_center()
                                    .cursor_pointer()
                                    .rounded(radius)
                                    .hover(move |this| this.bg(accent))
                                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(key, cx)))
                                    .child(
                                        Icon::new(if is_expanded {
                                            IconName::ChevronDown
                                        } else {
                                            IconName::ChevronRight
                                        })
                                        .xsmall()
                                        .text_color(muted),
                                    )
                                    .child(div().text_xs().font_medium().child(Self::script_phase_label(*phase)))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted)
                                            .child(i18n::counted("import.report_lines", lines)),
                                    ),
                            )
                            .child(
                                Button::new(("import-report-copy", group_idx * 10_000 + item_idx))
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(IconName::Copy))
                                    .tooltip(t!("common.copy").to_string())
                                    .on_click(move |_, window, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(full_source.clone()));
                                        window.push_notification(
                                            Notification::success(t!("import.report_script_copied").to_string()),
                                            cx,
                                        );
                                    }),
                            ),
                    )
                    .when_some(preview, move |this, preview| {
                        this.child(
                            div().px_3().pb_2().child(
                                div()
                                    .p_2()
                                    .rounded(radius)
                                    .border_1()
                                    .border_color(border)
                                    .bg(secondary.opacity(0.4))
                                    .text_xs()
                                    .font_family(mono)
                                    .overflow_x_hidden()
                                    .child(preview),
                            ),
                        )
                    })
                    .into_any_element()
            }
            LostKind::Template { expr } => h_flex()
                .gap_2()
                .px_3()
                .py_1p5()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("import.lost_template").to_string()),
                )
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(radius)
                        .bg(secondary.opacity(0.6))
                        .text_xs()
                        .font_family(mono)
                        .child(expr.clone()),
                )
                .into_any_element(),
            LostKind::Auth { kind } => div()
                .px_3()
                .py_1p5()
                .text_xs()
                .child(t!("import.lost_auth", kind = kind).to_string())
                .into_any_element(),
            LostKind::Body { kind } => div()
                .px_3()
                .py_1p5()
                .text_xs()
                .child(t!("import.lost_body", kind = kind).to_string())
                .into_any_element(),
        };
        div()
            .when(!last, |this| this.border_b_1().border_color(border))
            .child(row)
            .into_any_element()
    }

    fn render_group(&self, group_idx: usize, group: &LostGroup, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, secondary) = (theme.border, theme.muted_foreground, theme.secondary);
        let label = if group.target.is_empty() {
            t!("import.report_project").to_string()
        } else {
            group.target.clone()
        };
        let count = group.items.len();
        let last_item = count.saturating_sub(1);
        let rows: Vec<AnyElement> = group
            .items
            .iter()
            .enumerate()
            .map(|(item_idx, item)| self.render_item(group_idx, item_idx, item, item_idx == last_item, cx))
            .collect();
        v_flex()
            .border_b_1()
            .border_color(border)
            .child(
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_1p5()
                    .items_center()
                    .justify_between()
                    .bg(secondary.opacity(0.5))
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .font_semibold()
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(muted)
                            .child(i18n::counted("import.report_item", count)),
                    ),
            )
            .children(rows)
            .into_any_element()
    }
}

impl Render for ImportReportView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, warning) = (theme.border, theme.warning);
        let summary = t!(
            "import.report_summary",
            items = i18n::counted("import.report_item", self.total),
            targets = i18n::counted("import.report_target", self.groups.len())
        )
        .to_string();
        let groups: Vec<AnyElement> = self
            .groups
            .iter()
            .enumerate()
            .map(|(idx, group)| self.render_group(idx, group, cx))
            .collect();
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Icon::new(IconName::TriangleAlert).small().text_color(warning))
                    .child(div().text_sm().child(summary)),
            )
            .child(
                div()
                    .id("import-report-list")
                    .max_h(px(420.))
                    .overflow_y_scrollbar()
                    .border_1()
                    .border_color(border)
                    .rounded_md()
                    .children(groups),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AnyWindowHandle, TestAppContext, size};

    #[gpui_kit::test]
    async fn import_report_groups_toggles_and_renders(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let lost = vec![
            Lost {
                target: "Orders / Create".into(),
                kind: LostKind::Script {
                    phase: ScriptPhase::Test,
                    source: "pm.test('ok', () => {});".into(),
                },
            },
            Lost {
                target: String::new(),
                kind: LostKind::Auth { kind: "hawk".into() },
            },
            Lost {
                target: "Orders / Create".into(),
                kind: LostKind::Template {
                    expr: "{{$randomEmail}}".into(),
                },
            },
            Lost {
                target: "Orders / Create".into(),
                kind: LostKind::Body { kind: "file".into() },
            },
        ];
        let mut view = None;
        let window: AnyWindowHandle = cx
            .open_window(size(px(680.), px(520.)), |window, cx| {
                let report = cx.new(|_| ImportReportView::new(&lost));
                view = Some(report.clone());
                Root::new(report, window, cx)
            })
            .into();
        let view = view.expect("built");
        cx.read(|cx| {
            let report = view.read(cx);
            assert_eq!(report.total, 4);
            assert_eq!(report.groups.len(), 2);
            assert_eq!(report.groups[0].target, "");
            assert_eq!(report.groups[1].target, "Orders / Create");
            assert!(report.expanded.is_empty());
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            view.update(cx, |report, cx| report.toggle((1, 0), cx));
            window.render_frame(cx);
        })
        .expect("window is open");
        cx.read(|cx| {
            assert!(view.read(cx).expanded.contains(&(1, 0)));
        });
    }
}
