//! An event stream's events (`cyberapi_core::sse`), or a gRPC call's
//! streamed replies, a row each: when it came, its type, its id when the
//! stream gives ids (#72), the start of its data. The row picked shows its
//! data whole below the list, indented when it is JSON, under its id and
//! the reconnection time it asked for, if any. While the stream comes
//! the rows are the API state's live ones, and the list follows the newest
//! — unless the reader has scrolled back from it.

use crate::states::ApiState;
use cyberapi_core::format;
use cyberapi_core::response::HttpResponse;
use cyberapi_core::sse::Event;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::scroll::{Scrollbar, ScrollbarAxis};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, UniformListScrollHandle, Window, div, px, relative,
    uniform_list,
};
use rust_i18n::t;
use std::sync::Arc;

const ROW_HEIGHT: f32 = 26.;
const ID_WIDTH: f32 = 72.;
/// The most of an event's data a row shows: a long line is shaped whole,
/// though only its start fits.
const ROW_DATA: usize = 240;

pub struct EventStream {
    api: Entity<ApiState>,
    /// A kept response's events, read once; `None` while the live stream's
    /// are shown.
    kept: Option<(String, Arc<Vec<Event>>)>,
    scroll: UniformListScrollHandle,
    /// How many of the events were looked at for an id, and whether one
    /// had: the live stream's only grow, so only the new ones are looked
    /// at in each frame (PERF-6).
    ids: (usize, bool),
    /// The event picked, and its data in an editor.
    picked: Option<Picked>,
    _subscriptions: Vec<Subscription>,
}

struct Picked {
    index: usize,
    data: Entity<EditorState>,
    /// Its id and reconnection time, where it gave them.
    fields: Option<String>,
}

impl EventStream {
    pub fn new(api: Entity<ApiState>, cx: &mut Context<Self>) -> Self {
        // The live stream's rows grow as the API state takes them in.
        let watching = cx.observe(&api, |_, _, cx| cx.notify());
        Self {
            api,
            kept: None,
            scroll: UniformListScrollHandle::new(),
            ids: (0, false),
            picked: None,
            _subscriptions: vec![watching],
        }
    }

    /// Shows `response`'s events; the same response keeps its pick and
    /// scroll.
    pub fn show(&mut self, response: &HttpResponse, cx: &mut Context<Self>) {
        if self.kept.as_ref().is_some_and(|(id, _)| *id == response.id) {
            return;
        }
        let events = response.stream_events();
        self.kept = Some((response.id.clone(), Arc::new(events)));
        self.reset(cx);
    }

    /// Shows the stream coming now.
    pub fn show_live(&mut self, cx: &mut Context<Self>) {
        if self.kept.take().is_some() {
            self.reset(cx);
        }
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.ids = (0, false);
        self.picked = None;
        self.scroll = UniformListScrollHandle::new();
        cx.notify();
    }

    fn events<'a>(&'a self, cx: &'a App) -> &'a [Event] {
        match &self.kept {
            Some((_, events)) => events,
            None => self
                .api
                .read(cx)
                .live()
                .map(|live| live.events.as_slice())
                .unwrap_or_default(),
        }
    }

    fn pick(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.picked.as_ref().is_some_and(|picked| picked.index == index) {
            self.picked = None;
            cx.notify();
            return;
        }
        let Some(event) = self.events(cx).get(index) else {
            return;
        };
        let (language, text) = match serde_json::from_str::<serde_json::Value>(&event.data) {
            Ok(json) => ("json", format::json(&json)),
            Err(_) => ("text", event.data.clone()),
        };
        let fields = [
            event
                .id
                .as_ref()
                .map(|id| t!("response.event_id_is", id = id).to_string()),
            event.retry.map(|ms| t!("response.event_retry_is", ms = ms).to_string()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let fields = (!fields.is_empty()).then(|| fields.join(" · "));
        let editor = cx.new(|cx| {
            crate::views::fix_editor_search(cx);
            EditorState::new(window, cx)
                .language(language)
                .line_number(false)
                .tab_size(crate::views::json_tab())
                .soft_wrap(true)
                .default_value(text)
        });
        self.picked = Some(Picked {
            index,
            data: editor,
            fields,
        });
        cx.notify();
    }

    fn row(&self, index: usize, event: &Event, ids: bool, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let picked = self.picked.as_ref().is_some_and(|picked| picked.index == index);
        let mut data: String = event.data.chars().take(ROW_DATA).collect();
        data = data.replace(['\n', '\r'], " ");
        h_flex()
            .id(("sse-event", index))
            .w_full()
            .h(px(ROW_HEIGHT))
            .px_3()
            .gap_3()
            .items_center()
            .text_xs()
            .cursor_pointer()
            .when(picked, |this| this.bg(theme.accent))
            .hover(|this| this.bg(theme.accent))
            .child(
                div()
                    .w(px(64.))
                    .flex_none()
                    .text_color(theme.muted_foreground)
                    .child(at(event.at)),
            )
            .child(
                div()
                    .w(px(96.))
                    .flex_none()
                    .truncate()
                    .text_color(theme.info)
                    .child(event.kind().to_string()),
            )
            .when(ids, |this| {
                this.child(
                    div()
                        .w(px(ID_WIDTH))
                        .flex_none()
                        .truncate()
                        .text_color(theme.muted_foreground)
                        .font_family(theme.mono_font_family.clone())
                        .child(event.id.clone().unwrap_or_default()),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_font_family.clone())
                    .child(data),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.pick(index, window, cx)))
    }
}

/// When an event came, after the head: `+1.250s`.
fn at(at: Option<u64>) -> String {
    match at {
        Some(ms) => format!("+{:.3}s", ms as f64 / 1000.),
        None => String::new(),
    }
}

impl Render for EventStream {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The id column, for a stream that gives ids.
        let (count, ids) = {
            let events = self.events(cx);
            let (looked, had) = match self.ids {
                (looked, had) if looked <= events.len() => (looked, had),
                _ => (0, false),
            };
            let had = had || events[looked..].iter().any(|event| event.id.is_some());
            (events.len(), had)
        };
        self.ids = (count, ids);
        let live = self.kept.is_none();
        // Following the newest while the reader is at it.
        if live && self.scroll.is_scrolled_to_end() != Some(false) {
            self.scroll.scroll_to_bottom();
        }
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        if count == 0 {
            let text = if live {
                t!("response.events_waiting")
            } else {
                t!("response.events_none")
            };
            return v_flex()
                .size_full()
                .p_3()
                .text_sm()
                .text_color(muted)
                .child(text.to_string())
                .into_any_element();
        }
        let list = uniform_list(
            "sse-events",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                let events = this.events(cx).get(range.clone()).unwrap_or_default();
                events
                    .iter()
                    .zip(range)
                    .map(|(event, index)| this.row(index, event, ids, cx).into_any_element())
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full();
        let header = h_flex()
            .h(px(ROW_HEIGHT))
            .px_3()
            .gap_3()
            .items_center()
            .text_xs()
            .text_color(muted)
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .w(px(64.))
                    .flex_none()
                    .child(t!("response.event_time").to_string()),
            )
            .child(
                div()
                    .w(px(96.))
                    .flex_none()
                    .child(t!("response.event_type").to_string()),
            )
            .when(ids, |this| {
                this.child(
                    div()
                        .w(px(ID_WIDTH))
                        .flex_none()
                        .child(t!("response.event_id").to_string()),
                )
            })
            .child(div().flex_1().child(t!("response.event_data").to_string()));
        v_flex()
            .size_full()
            .child(header)
            .child(
                div().flex_1().min_h_0().relative().child(list).child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(Scrollbar::new(&self.scroll).axis(ScrollbarAxis::Vertical)),
                ),
            )
            .when_some(self.picked.as_ref(), |this, picked| {
                this.child(
                    v_flex()
                        .h(relative(0.4))
                        .flex_none()
                        .border_t_1()
                        .border_color(border)
                        .when_some(picked.fields.clone(), |this, fields| {
                            this.child(
                                div()
                                    .px_3()
                                    .py_1()
                                    .text_xs()
                                    .text_color(muted)
                                    .border_b_1()
                                    .border_color(border)
                                    .child(fields),
                            )
                        })
                        .child(
                            div().flex_1().min_h_0().child(
                                Editor::new(&picked.data)
                                    .w_full()
                                    .h_full()
                                    .readonly(true)
                                    .bordered(false),
                            ),
                        ),
                )
            })
            .into_any_element()
    }
}
