//! D4: the project's newest responses, of any of its APIs, newest first.
//! Picking one opens its API and shows it there, its request put back, as
//! picking it from that API's history does.

use super::response_panel::HistoryRow;
use crate::i18n;
use crate::states::WorkspaceState;
use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, v_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

/// Opens the list, reading it again.
pub fn open_recent_dialog(workspace: Entity<WorkspaceState>, window: &mut Window, cx: &mut gpui_kit::App) {
    workspace.update(cx, |workspace, cx| workspace.load_recent(cx));
    let list = cx.new(|cx| RecentList::new(workspace, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(t!("recent.title").to_string())
            .w(px(560.))
            .on_ok(|_, _, _| false)
            .child(list.clone())
            .footer(
                DialogFooter::new().child(
                    Button::new("recent-close")
                        .label(t!("common.close").to_string())
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}

struct RecentList {
    workspace: Entity<WorkspaceState>,
    _observe: Subscription,
}

impl RecentList {
    fn new(workspace: Entity<WorkspaceState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&workspace, |_, _, cx| cx.notify());
        Self {
            workspace,
            _observe: observe,
        }
    }
}

impl Render for RecentList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let note = |text: String| div().py_6().text_sm().text_color(muted).child(text).into_any_element();
        let workspace = self.workspace.read(cx);
        let body = match workspace.recent() {
            None => note(t!("recent.loading").to_string()),
            Some(Err(error)) => note(i18n::store_error(error)),
            Some(Ok(responses)) if responses.is_empty() => note(t!("recent.none").to_string()),
            Some(Ok(responses)) => {
                let today = chrono::Local::now().date_naive();
                let hover = cx.theme().accent;
                let rows: Vec<_> = responses
                    .iter()
                    .enumerate()
                    .map(|(index, response)| {
                        let row = HistoryRow::of(response, today, cx);
                        let name = workspace.api_name(&response.api).map(str::to_string);
                        let (workspace, api, id) = (self.workspace.clone(), response.api.clone(), row.id.clone());
                        div()
                            .id(("recent-row", index))
                            .px_2()
                            .py_1p5()
                            .rounded_md()
                            .text_sm()
                            .cursor_pointer()
                            .hover(move |style| style.bg(hover))
                            .child(row.render(name.as_deref(), 500., cx))
                            .on_click(move |_, window, cx| {
                                workspace.update(cx, |workspace, cx| workspace.open_response(&api, &id, cx));
                                window.close_dialog(cx);
                            })
                    })
                    .collect();
                v_flex().gap_0p5().children(rows).into_any_element()
            }
        };
        v_flex()
            .gap_2()
            .child(div().text_xs().text_color(muted).child(t!("recent.hint").to_string()))
            .child(div().id("recent-list").max_h(px(440.)).overflow_y_scroll().child(body))
    }
}
