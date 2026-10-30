//! D4: two responses of an API's history side by side: their bodies line
//! by line (`cyberapi_core::diff`), what was taken out on red, what came in
//! on green, what stayed folded but for the lines around each change. It
//! starts with the response shown against the one before it; either can be
//! changed.

use crate::states::ApiState;
use cyberapi_core::diff::{self, Change, Compared, Row};
use cyberapi_core::response::HttpResponse;
use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarAxis};
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{ActiveTheme as _, IndexPath, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Task, UniformListScrollHandle, Window, div, px, uniform_list,
};
use rust_i18n::t;
use std::sync::Arc;

const ROW_HEIGHT: f32 = 20.;

/// Opens the comparison over `api`'s history; nothing when it has fewer
/// than two responses.
pub fn open_compare_dialog(api: Entity<ApiState>, window: &mut Window, cx: &mut App) {
    if api.read(cx).responses().len() < 2 {
        return;
    }
    let view = cx.new(|cx| CompareView::new(api, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(t!("compare.title").to_string())
            .w(px(860.))
            .on_ok(|_, _, _| false)
            .child(view.clone())
            .footer(
                DialogFooter::new().child(
                    Button::new("compare-close")
                        .label(t!("common.close").to_string())
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}

/// A response as the lists offer it: its status and when.
#[derive(Clone)]
struct Entry {
    label: SharedString,
    index: usize,
}

impl SelectItem for Entry {
    type Value = usize;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &usize {
        &self.index
    }
}

type Entries = Entity<SelectState<Vec<Entry>>>;

struct CompareView {
    api: Entity<ApiState>,
    /// The history as it was when the dialog opened, newest first, as a
    /// list shows it: the two compared are read whole (PERF-9).
    responses: Vec<Arc<HttpResponse>>,
    older: Entries,
    newer: Entries,
    /// `None` while it is worked out.
    compared: Option<Arc<Compared>>,
    /// A body the history did not keep, of the two compared.
    omitted: bool,
    /// Why the two could not be read.
    failed: Option<String>,
    working: Option<Task<()>>,
    list: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl CompareView {
    fn new(api: Entity<ApiState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = api.clone();
        let api = api.read(cx);
        let responses: Vec<Arc<HttpResponse>> = api.responses().to_vec();
        let shown = api
            .shown_response()
            .and_then(|shown| responses.iter().position(|response| response.id == shown.id))
            .unwrap_or(0);
        // The one shown against the one before it; the oldest against the
        // one after it.
        let (newer_at, older_at) = if shown + 1 < responses.len() {
            (shown, shown + 1)
        } else {
            (shown - 1, shown)
        };
        let today = chrono::Local::now().date_naive();
        let entries: Vec<Entry> = responses
            .iter()
            .enumerate()
            .map(|(index, response)| Entry {
                label: entry_label(response, today).into(),
                index,
            })
            .collect();
        let older = cx.new(|cx| SelectState::new(entries.clone(), Some(IndexPath::new(older_at)), window, cx));
        let newer = cx.new(|cx| SelectState::new(entries, Some(IndexPath::new(newer_at)), window, cx));
        let changed = |this: &mut Self, _: Entries, event: &SelectEvent<Vec<Entry>>, cx: &mut Context<Self>| {
            if let SelectEvent::Confirm(Some(_)) = event {
                this.compare(cx);
            }
        };
        let subscriptions = vec![cx.subscribe(&older, changed), cx.subscribe(&newer, changed)];
        let mut view = Self {
            api: state,
            responses,
            older,
            newer,
            compared: None,
            omitted: false,
            failed: None,
            working: None,
            list: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        view.compare(cx);
        view
    }

    fn picked(&self, list: &Entries, cx: &App) -> Option<Arc<HttpResponse>> {
        let index = *list.read(cx).selected_value()?;
        self.responses.get(index).cloned()
    }

    /// The two read whole, and their bodies compared, away from the
    /// window: a long one takes a while.
    fn compare(&mut self, cx: &mut Context<Self>) {
        let (Some(older), Some(newer)) = (self.picked(&self.older, cx), self.picked(&self.newer, cx)) else {
            return;
        };
        let (older, newer) = {
            let api = self.api.read(cx);
            (api.whole_response(&older.id), api.whole_response(&newer.id))
        };
        self.compared = None;
        self.failed = None;
        let work = cx.background_spawn(async move {
            let (older, newer) = (older.await?, newer.await?);
            let omitted = older.body_omitted.is_some() || newer.body_omitted.is_some();
            Ok::<_, cyberapi_store::StoreError>((
                omitted,
                diff::compare(&diff::body_text(&older), &diff::body_text(&newer)),
            ))
        });
        self.working = Some(cx.spawn(async move |this, cx| {
            let compared = work.await;
            let _ = this.update(cx, |this, cx| {
                match compared {
                    Ok((omitted, compared)) => {
                        this.omitted = omitted;
                        this.compared = Some(Arc::new(compared));
                    }
                    Err(error) => this.failed = Some(crate::i18n::store_error(&error)),
                }
                this.working = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn row(&self, row: &Row, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let number = |at: Option<usize>| {
            div()
                .w(px(44.))
                .flex_none()
                .text_right()
                .pr_2()
                .text_color(theme.muted_foreground)
                .child(at.map(|at| at.to_string()).unwrap_or_default())
        };
        match row {
            Row::Folded(lines) => h_flex()
                .w_full()
                .h(px(ROW_HEIGHT))
                .px_2()
                .bg(theme.muted)
                .text_color(theme.muted_foreground)
                .child(t!("compare.folded", count = lines).to_string())
                .into_any_element(),
            Row::Line { change, old, new, text } => {
                let (sign, background) = match change {
                    Change::Same => (" ", None),
                    Change::Removed => ("-", Some(theme.danger.opacity(0.15))),
                    Change::Added => ("+", Some(theme.success.opacity(0.15))),
                };
                h_flex()
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .when_some(background, |row, background| row.bg(background))
                    .child(number(*old))
                    .child(number(*new))
                    .child(div().w(px(16.)).flex_none().child(sign))
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
        }
    }
}

/// `200 · 12:01:03`, as a list offers a response.
fn entry_label(response: &HttpResponse, today: chrono::NaiveDate) -> String {
    let status = match response.status {
        Some(code) => code.to_string(),
        None => t!("response.failed").to_string(),
    };
    let at = response.created_at.get().with_timezone(&chrono::Local);
    let when = if at.date_naive() == today {
        at.format("%H:%M:%S")
    } else {
        at.format("%m-%d %H:%M")
    };
    format!("{status} · {when}")
}

impl Render for CompareView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let mono = theme.mono_font_family.clone();
        // A body the history did not keep has nothing to compare.
        let omitted = self.omitted;
        let summary = match &self.compared {
            None if self.failed.is_some() => self.failed.clone().unwrap_or_default(),
            None => t!("compare.working").to_string(),
            Some(compared) if compared.same() => t!("compare.same").to_string(),
            Some(compared) => t!("compare.counts", added = compared.added, removed = compared.removed).to_string(),
        };
        let cut = self.compared.as_ref().is_some_and(|compared| compared.cut);
        let rows = self.compared.clone();
        let count = rows.as_ref().map_or(0, |compared| compared.rows.len());
        let list = uniform_list(
            "compare-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                let Some(compared) = &rows else {
                    return Vec::new();
                };
                compared.rows[range]
                    .iter()
                    .map(|row| this.row(row, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.list)
        .size_full();
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .child(t!("compare.older").to_string())
                    .child(div().w(px(200.)).child(Select::new(&self.older).small()))
                    .child(t!("compare.newer").to_string())
                    .child(div().w(px(200.)).child(Select::new(&self.newer).small()))
                    .child(div().flex_1())
                    .child(div().text_color(muted).child(summary)),
            )
            .when(omitted, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("compare.omitted").to_string()),
                )
            })
            .when(cut, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("compare.cut", lines = diff::LINE_LIMIT).to_string()),
                )
            })
            .child(
                div()
                    .h(px(440.))
                    .relative()
                    .border_1()
                    .border_color(border)
                    .rounded_md()
                    .text_xs()
                    .font_family(mono)
                    .child(list)
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .child(Scrollbar::new(&self.list).axis(ScrollbarAxis::Vertical)),
                    ),
            )
    }
}
