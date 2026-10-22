//! The selected API's response (M7): status, time and size, the body, the
//! headers and cookies, where the time went, and the history.

// The action, named apart from the `Send` trait.
use super::{KEY_CONTEXT, Send as SendAction};
use crate::json_view_local::JsonView;
use crate::states::{ApiState, AppStore};
use crate::this_build;
use crate::views::json_tree::{JsonTree, json_view_switch};
use cyberapi_core::response::{
    BodyKind, DISPLAY_LIMIT, HttpResponse, StatusLevel, clip_text, format_duration, format_size, status_level,
    status_text,
};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::empty::{
    Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, Hsla, Image, ImageFormat, ImageSource,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window,
    div, img, px, relative,
};
use rust_i18n::t;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ResponseTab {
    Body,
    Headers,
    Cookies,
    Info,
}

const TABS: [ResponseTab; 4] = [
    ResponseTab::Body,
    ResponseTab::Headers,
    ResponseTab::Cookies,
    ResponseTab::Info,
];

pub struct ResponsePanel {
    api: Entity<ApiState>,
    /// The body as text, read only, for selecting and searching.
    body: Entity<EditorState>,
    /// Which response the body editor holds.
    shown: Option<String>,
    /// The editor holds only the start of a body over `DISPLAY_LIMIT`.
    clipped: bool,
    tab: ResponseTab,
    /// The body as a tree, when this response's JSON is shown that way.
    json_tree: Entity<JsonTree>,
    /// Bound to one response. Another response opens in the saved default.
    body_as_tree: Option<(String, bool)>,
    _subscriptions: Vec<Subscription>,
}

impl ResponsePanel {
    pub fn new(api: Entity<ApiState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let body = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .soft_wrap(true)
                .searchable(true)
        });
        let app = AppStore::global(cx).state().clone();
        let subscriptions = vec![
            cx.observe_in(&api, window, |this, _, window, cx| this.sync(window, cx)),
            cx.observe(&app, |this, _, cx| {
                this.sync_response_tree(cx);
                cx.notify();
            }),
        ];
        let json_tree = cx.new(JsonTree::new);
        let mut panel = Self {
            api,
            body,
            shown: None,
            clipped: false,
            tab: ResponseTab::Body,
            json_tree,
            body_as_tree: None,
            _subscriptions: subscriptions,
        };
        panel.sync(window, cx);
        panel
    }

    /// The tree, when this response was switched to it; otherwise the default.
    fn response_shows_tree(&self, cx: &App) -> bool {
        let default = AppStore::global(cx).state().read(cx).json_view() == JsonView::Tree;
        match (&self.shown, &self.body_as_tree) {
            (Some(id), Some((bound, tree))) if bound == id => *tree,
            _ => default,
        }
    }

    fn shown_json(&self, cx: &App) -> bool {
        self.api.read(cx).shown_response().is_some_and(|response| {
            response.error.is_none()
                && response.body_omitted.is_none()
                && !response.body.is_empty()
                && response.body_kind() == BodyKind::Json
        })
    }

    fn sync_response_tree(&mut self, cx: &mut Context<Self>) {
        if !(self.shown_json(cx) && self.response_shows_tree(cx)) {
            return;
        }
        let text = self.body.read(cx).value().to_string();
        self.json_tree.update(cx, |tree, cx| tree.set_text(&text, cx));
    }

    /// Puts the shown response's body in the editor when it changes.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.api.read(cx).shown_response().cloned();
        let id = shown.as_ref().map(|response| response.id.clone());
        if id == self.shown && id.is_some() {
            cx.notify();
            return;
        }
        self.shown = id;
        let (language, text) = match &shown {
            Some(response) => (language_of(response), response.body_text().unwrap_or_default()),
            None => ("text", String::new()),
        };
        // A body that would make the editor crawl shows its start (#17).
        let (text, clipped) = clip_text(text, DISPLAY_LIMIT);
        self.clipped = clipped;
        self.body = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .line_number(true)
                .soft_wrap(true)
                .searchable(true)
                .default_value(text)
        });
        self.sync_response_tree(cx);
        cx.notify();
    }

    fn status_color(&self, status: u16, cx: &App) -> Hsla {
        let theme = cx.theme();
        match status_level(status) {
            StatusLevel::Success => theme.success,
            StatusLevel::ClientError => theme.warning,
            StatusLevel::ServerError => theme.danger,
        }
    }

    fn render_summary(&self, response: Option<&HttpResponse>, cx: &mut Context<Self>) -> AnyElement {
        let api = self.api.read(cx);
        let muted = cx.theme().muted_foreground;
        let today = chrono::Local::now().date_naive();
        // Each answer as a line of the history: its status in its colour,
        // how long it took, how big it was, when.
        let entries: Vec<(String, Hsla, String, String, String)> = api
            .responses()
            .iter()
            .map(|response| {
                let (status, color) = match response.status {
                    Some(code) => (code.to_string(), self.status_color(code, cx)),
                    None => (t!("response.failed").to_string(), cx.theme().danger),
                };
                let at = response.created_at.get().with_timezone(&chrono::Local);
                let when = if at.date_naive() == today {
                    at.format("%H:%M:%S")
                } else {
                    at.format("%m-%d %H:%M")
                };
                (
                    response.id.clone(),
                    color,
                    status,
                    format!("{} · {}", format_duration(response.latency), body_size(response)),
                    when.to_string(),
                )
            })
            .collect();
        let sending = api.sending_since();
        let shown_id = response.map(|r| r.id.clone());
        let this = cx.entity().downgrade();
        let history_menu = move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
            let menu = entries.iter().fold(menu, |menu, (id, color, status, measures, when)| {
                let (this, id) = (this.clone(), id.clone());
                let (color, status, measures, when) = (*color, status.clone(), measures.clone(), when.clone());
                menu.item(
                    PopupMenuItem::element(move |_, cx| {
                        h_flex()
                            .w(px(260.))
                            .gap_3()
                            .items_center()
                            .child(div().w(px(36.)).font_semibold().text_color(color).child(status.clone()))
                            .child(div().flex_1().child(measures.clone()))
                            .child(div().text_color(cx.theme().muted_foreground).child(when.clone()))
                    })
                    .checked(shown_id.as_deref() == Some(id.as_str()))
                    .on_click(move |_, _, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.api.update(cx, |api, cx| api.choose_response(&id, cx));
                        });
                    }),
                )
            });
            let clear = this.clone();
            menu.separator()
                .item(
                    PopupMenuItem::new(t!("response.clear_history").to_string()).on_click(move |_, _, cx| {
                        let _ = clear.update(cx, |this, cx| this.api.update(cx, |api, cx| api.clear_history(cx)));
                    }),
                )
        };
        let status: AnyElement = match (sending, response) {
            (Some(started), _) => h_flex()
                .gap_2()
                .items_center()
                .child(Spinner::new().small())
                .child(div().text_sm().text_color(muted).child(format!(
                    "{} {:.1}s",
                    t!("response.sending"),
                    started.elapsed().as_secs_f32()
                )))
                .into_any_element(),
            (None, Some(response)) => {
                let status: AnyElement = match response.status {
                    Some(code) => {
                        let color = self.status_color(code, cx);
                        h_flex()
                            .h(px(22.))
                            .px_2()
                            .items_center()
                            .rounded_full()
                            .bg(color.opacity(0.16))
                            .font_semibold()
                            .text_color(color)
                            .child(format!("{code} {}", status_text(code)))
                            .into_any_element()
                    }
                    None => {
                        let color = cx.theme().danger;
                        h_flex()
                            .h(px(22.))
                            .px_2()
                            .items_center()
                            .rounded_full()
                            .bg(color.opacity(0.16))
                            .font_semibold()
                            .text_color(color)
                            .child(t!("response.failed").to_string())
                            .into_any_element()
                    }
                };
                // F7.2: the size, and how much smaller it came.
                let size = match response.compression() {
                    Some((wire, percent)) => format!(
                        "{} ({})",
                        body_size(response),
                        t!("response.compressed", size = format_size(wire), percent = percent)
                    ),
                    None => body_size(response),
                };
                h_flex()
                    .gap_3()
                    .items_center()
                    .text_sm()
                    .child(status)
                    .child(div().text_color(muted).child(format_duration(response.latency)))
                    .child(div().text_color(muted).child(size))
                    .into_any_element()
            }
            (None, None) => div().into_any_element(),
        };
        h_flex()
            .gap_2()
            .px_3()
            .h(px(44.))
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().flex_1().min_w_0().child(status))
            .child(
                Button::new("response-curl")
                    .ghost()
                    .small()
                    .icon(gpui_kit::assets::IconName::Terminal)
                    .tooltip(t!("response.copy_curl").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.api.update(cx, |api, cx| api.copy_curl(cx)))),
            )
            .child(
                Button::new("response-history")
                    .ghost()
                    .small()
                    .icon(gpui_kit::assets::IconName::Clock)
                    .tooltip(t!("response.history").to_string())
                    .disabled(api.responses().is_empty())
                    .dropdown_menu_with_anchor(Anchor::TopRight, history_menu),
            )
            .into_any_element()
    }

    fn render_tabs(&self, response: &HttpResponse, cx: &mut Context<Self>) -> AnyElement {
        let label = |tab: &ResponseTab| match tab {
            ResponseTab::Body => t!("response.body").to_string(),
            ResponseTab::Headers => format!("{} ({})", t!("response.headers"), response.display_headers().len()),
            ResponseTab::Cookies => format!("{} ({})", t!("response.cookies"), response.cookies().len()),
            ResponseTab::Info => t!("response.info").to_string(),
        };
        let copies = self.tab == ResponseTab::Body
            && response.error.is_none()
            && response.body_omitted.is_none()
            && !response.body.is_empty()
            && matches!(response.body_kind(), BodyKind::Json | BodyKind::Text);
        TabBar::new("response-tabs")
            .underline()
            .small()
            .px_2()
            .selected_index(TABS.iter().position(|tab| *tab == self.tab).unwrap_or(0))
            .children(TABS.iter().map(|tab| Tab::new().label(label(tab))))
            .when(copies, |bar| {
                let page = this_build::opens_html() && response.content_type().to_ascii_lowercase().contains("html");
                let json = response.body_kind() == BodyKind::Json;
                bar.suffix(
                    h_flex()
                        .gap_0p5()
                        .items_center()
                        .when(json, |this| {
                            let tree = self.response_shows_tree(cx);
                            this.child(json_view_switch(
                                "response-json-view",
                                tree,
                                cx.listener(|this, picks: &Vec<usize>, _, cx| {
                                    let Some(id) = this.shown.clone() else {
                                        return;
                                    };
                                    this.body_as_tree = Some((id, picks.first().copied() == Some(1)));
                                    this.sync_response_tree(cx);
                                    cx.notify();
                                }),
                            ))
                        })
                        // F7.4 for a page: drawn by the system's browser.
                        .when(page, |this| {
                            this.child(
                                Button::new("response-open-page")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ExternalLink)
                                    .tooltip(t!("response.open_page").to_string())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.api.update(cx, |api, cx| api.view_body(cx));
                                    })),
                            )
                        })
                        .child(
                            Button::new("response-copy-body")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Copy)
                                .tooltip(t!("response.copy_body").to_string())
                                // The body as text, whole: the editor may
                                // show only its start.
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let text = this
                                        .api
                                        .read(cx)
                                        .shown_response()
                                        .and_then(|response| response.body_text());
                                    if let Some(text) = text {
                                        copy(text, window, cx);
                                    }
                                })),
                        ),
                )
            })
            .on_click(cx.listener(|this, index: &usize, _, cx| {
                if let Some(tab) = TABS.get(*index) {
                    this.tab = *tab;
                    cx.notify();
                }
            }))
            .into_any_element()
    }

    fn render_body(&self, response: &HttpResponse, cx: &mut Context<Self>) -> AnyElement {
        // F5.7: why it failed, as the failure it is.
        if let Some(error) = &response.error {
            let copied = error.clone();
            return v_flex()
                .p_3()
                .gap_2()
                .child(Alert::error("response-error", error.clone()).title(t!("response.failed").to_string()))
                // The reason, to search for or pass on: the text cannot be
                // selected.
                .child(
                    h_flex().justify_end().child(
                        Button::new("response-copy-error")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Copy)
                            .label(t!("common.copy").to_string())
                            .on_click(move |_, window, cx| copy(copied.clone(), window, cx)),
                    ),
                )
                .into_any_element();
        }
        if let Some(size) = response.body_omitted {
            return note(
                t!("response.omitted", size = format_size(size)).to_string(),
                Vec::new(),
                cx,
            );
        }
        // 204, a HEAD: nothing to show, said rather than an empty editor.
        if response.body.is_empty() {
            return note(t!("response.no_body").to_string(), Vec::new(), cx);
        }
        match response.body_kind() {
            BodyKind::Json | BodyKind::Text => {
                let tree = response.body_kind() == BodyKind::Json && self.response_shows_tree(cx);
                let body: AnyElement = if tree {
                    div().size_full().child(self.json_tree.clone()).into_any_element()
                } else {
                    Editor::new(&self.body)
                        .w_full()
                        .h_full()
                        .readonly(true)
                        .bordered(false)
                        .into_any_element()
                };
                v_flex()
                    .size_full()
                    .when(self.clipped, |this| {
                        let shown = format_size(DISPLAY_LIMIT as u64);
                        let text = t!("response.clipped", size = shown).to_string();
                        this.child(note(text, vec![save_button(cx)], cx))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .relative()
                            .child(div().absolute().inset_0().child(body)),
                    )
                    .into_any_element()
            }
            // F7.4.
            BodyKind::Image => match image_format(&response.content_type()) {
                Some(format) => div()
                    .size_full()
                    .p_3()
                    .flex()
                    .items_start()
                    .justify_center()
                    .child(
                        img(ImageSource::Image(Arc::new(Image::from_bytes(
                            format,
                            response.body.clone(),
                        ))))
                        .max_w_full()
                        .max_h_full(),
                    )
                    .into_any_element(),
                None => viewed_outside(t!("response.image", size = body_size(response)).to_string(), cx),
            },
            BodyKind::Pdf => viewed_outside(t!("response.pdf", size = body_size(response)).to_string(), cx),
            BodyKind::Binary => note(
                t!("response.binary", size = body_size(response)).to_string(),
                vec![save_button(cx)],
                cx,
            ),
        }
    }

    /// The rows, or `empty` said when there are none.
    fn render_headers(
        id: &'static str,
        rows: Vec<(String, String)>,
        empty: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if rows.is_empty() {
            return note(empty, Vec::new(), cx);
        }
        v_flex()
            .id(id)
            .size_full()
            .overflow_y_scrollbar()
            .p_2()
            .child(field_rows(id, rows, cx))
            .into_any_element()
    }

    /// F7.7: what the request was, then where its time went, each phase a
    /// bar on the request's timeline.
    fn render_info(&self, response: &HttpResponse, cx: &mut Context<Self>) -> AnyElement {
        let stats = &response.stats;
        let mut rows = Vec::new();
        // Where it went, as sent: variables resolved, the query in it.
        if !response.url.is_empty() {
            let method = response.request.method.as_str();
            rows.push((t!("response.url").to_string(), format!("{method} {}", response.url)));
        }
        rows.extend([
            (t!("response.api_id").to_string(), response.api.to_string()),
            (t!("response.remote").to_string(), stats.remote_addr.clone()),
        ]);
        if stats.is_https {
            rows.push((t!("response.cipher").to_string(), stats.cipher.clone()));
        }
        let theme = cx.theme();
        let phases = [
            (t!("response.dns"), stats.dns, theme.cyan),
            (t!("response.tcp"), stats.tcp, theme.blue),
            (t!("response.tls"), stats.tls, theme.magenta),
            (t!("response.send"), stats.send, theme.yellow),
            (t!("response.server"), stats.server_processing, theme.green),
            (t!("response.transfer"), stats.content_transfer, theme.red),
        ];
        // The phases may come to a little more than the total measured
        // around them; the line is long enough for both.
        let span = phases
            .iter()
            .map(|(_, duration, _)| *duration)
            .sum::<Duration>()
            .max(stats.total)
            .as_secs_f32();
        let (muted, track) = (theme.muted_foreground, theme.muted);
        let whole = div().absolute().inset_0().rounded_sm().bg(muted.opacity(0.5));
        let mut start = 0.;
        let timeline = v_flex()
            .gap_1()
            .px_2()
            .text_sm()
            .children(phases.into_iter().map(|(label, duration, color)| {
                let length = duration.as_secs_f32();
                let bar = (length > 0. && span > 0.).then(|| {
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(start / span))
                        .w(relative(length / span))
                        .min_w(px(2.))
                        .rounded_sm()
                        .bg(color)
                });
                start += length;
                timing_row(label.to_string(), duration, muted, bar, track)
            }))
            .child(
                timing_row(t!("response.total").to_string(), stats.total, muted, Some(whole), track)
                    .font_semibold()
                    .border_t_1()
                    .border_color(theme.border)
                    .pt_1(),
            );
        v_flex()
            .id("response-info")
            .size_full()
            .overflow_y_scrollbar()
            .p_2()
            .gap_3()
            .child(field_rows("response-info-fields", rows, cx))
            .child(timeline)
            .into_any_element()
    }
}

/// Names and values, each value copied by the button its row shows on
/// hover: the text itself cannot be selected.
fn field_rows(id: &'static str, rows: Vec<(String, String)>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (muted, hover) = (theme.muted_foreground, theme.list_hover);
    v_flex()
        .text_sm()
        .children(rows.into_iter().enumerate().map(move |(index, (name, value))| {
            let group = SharedString::from(format!("{id}-{index}"));
            let copied = value.clone();
            h_flex()
                .group(group.clone())
                .px_2()
                .py_1()
                .gap_3()
                .items_start()
                .rounded(theme.radius)
                .hover(move |style| style.bg(hover))
                .child(div().w(px(180.)).flex_none().text_color(muted).child(name))
                .child(div().flex_1().min_w_0().child(value))
                .child(
                    div()
                        .flex_none()
                        .invisible()
                        .group_hover(group, |style| style.visible())
                        .child(
                            Button::new((id, index))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Copy)
                                .tooltip(t!("common.copy").to_string())
                                .on_click(move |_, window, cx| copy(copied.clone(), window, cx)),
                        ),
                )
        }))
}

/// A phase of the timeline: its name, its bar where it fell, how long.
fn timing_row(
    label: String,
    duration: Duration,
    muted: Hsla,
    bar: Option<gpui_kit::Div>,
    track: Hsla,
) -> gpui_kit::Div {
    // A phase that did not happen: TLS over plain HTTP, DNS for an address.
    let shown = if duration.is_zero() {
        "—".to_string()
    } else {
        format_duration(duration)
    };
    h_flex()
        .gap_3()
        .py_0p5()
        .items_center()
        .child(div().w(px(180.)).flex_none().text_color(muted).child(label))
        .child(
            div()
                .flex_1()
                .min_w(px(60.))
                .h(px(8.))
                .relative()
                .rounded_sm()
                .bg(track)
                .children(bar),
        )
        .child(div().w(px(80.)).flex_none().text_right().child(shown))
}

fn copy(text: String, window: &mut Window, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    window.push_notification(Notification::success(t!("common.copied").to_string()), cx);
}

/// The grammar a text body is highlighted with, where it is compiled in
/// (the desktop): HTML's takes XML too.
fn language_of(response: &HttpResponse) -> &'static str {
    let content_type = response.content_type().to_ascii_lowercase();
    match response.body_kind() {
        BodyKind::Json => "json",
        _ if content_type.contains("html") || content_type.contains("xml") => "html",
        _ => "text",
    }
}

/// F7.4 for the formats an image element can draw.
fn image_format(content_type: &str) -> Option<ImageFormat> {
    let lower = content_type.to_ascii_lowercase();
    [
        ("png", ImageFormat::Png),
        ("jpeg", ImageFormat::Jpeg),
        ("jpg", ImageFormat::Jpeg),
        ("gif", ImageFormat::Gif),
        ("webp", ImageFormat::Webp),
        ("svg", ImageFormat::Svg),
        ("bmp", ImageFormat::Bmp),
        ("tif", ImageFormat::Tiff),
        ("icon", ImageFormat::Ico),
        ("portable-", ImageFormat::Pnm),
    ]
    .into_iter()
    .find(|(name, _)| lower.contains(name))
    .map(|(_, format)| format)
}

/// F7.2: how big the body is — as it came, or as it was decoded from a
/// compressed one.
fn body_size(response: &HttpResponse) -> String {
    match response.compression() {
        Some(_) => format_size(response.body_omitted.unwrap_or(response.body.len() as u64)),
        None => format_size(response.size()),
    }
}

/// F7.4: a body the app cannot draw, which something outside it can.
fn viewed_outside(text: String, cx: &mut Context<ResponsePanel>) -> AnyElement {
    let view = Button::new("response-view")
        .outline()
        .xsmall()
        .label(t!("response.view_outside").to_string())
        .on_click(cx.listener(|this, _, _, cx| {
            this.api.update(cx, |api, cx| api.view_body(cx));
        }));
    note(text, vec![view, save_button(cx)], cx)
}

/// Saves the body whole (F7.5).
fn save_button(cx: &mut Context<ResponsePanel>) -> Button {
    Button::new("response-save")
        .outline()
        .xsmall()
        .label(t!("response.save_body").to_string())
        .on_click(cx.listener(|this, _, _, cx| {
            this.api.update(cx, |api, cx| api.save_body(cx));
        }))
}

/// A line about the body, and what can be done with it.
fn note(text: String, buttons: Vec<Button>, cx: &mut Context<ResponsePanel>) -> AnyElement {
    h_flex()
        .p_3()
        .gap_3()
        .items_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(div().flex_1().min_w_0().child(text))
        .children(buttons)
        .into_any_element()
}

impl Render for ResponsePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let response = self.api.read(cx).shown_response().cloned();
        let sending = self.api.read(cx).sending_since().is_some();
        let summary = self.render_summary(response.as_deref(), cx);
        let content: AnyElement = match &response {
            Some(response) => {
                let content = match self.tab {
                    ResponseTab::Body => self.render_body(response, cx),
                    ResponseTab::Headers => Self::render_headers(
                        "response-headers",
                        response
                            .display_headers()
                            .into_iter()
                            .map(|(name, values)| (name, values.join("\n")))
                            .collect(),
                        t!("response.no_headers").to_string(),
                        cx,
                    ),
                    ResponseTab::Cookies => Self::render_headers(
                        "response-cookies",
                        response.cookies(),
                        t!("response.no_cookies").to_string(),
                        cx,
                    ),
                    ResponseTab::Info => self.render_info(response, cx),
                };
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_tabs(response, cx))
                    .child(div().flex_1().min_h_0().child(content))
                    .into_any_element()
            }
            // The first answer on its way, where it will show; the time it
            // has taken is in the line above.
            None if sending => v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .child(Spinner::new().large().color(cx.theme().muted_foreground))
                .into_any_element(),
            None => {
                // The shortcut as this platform writes it: ⌘↵, Ctrl+Enter.
                let shortcut = Kbd::binding_for_action(&SendAction, Some(KEY_CONTEXT), window);
                let muted = cx.theme().muted_foreground;
                v_flex()
                    .flex_1()
                    .justify_center()
                    .items_center()
                    .child(
                        Empty::new()
                            .header(
                                EmptyHeader::new()
                                    .media(
                                        EmptyMedia::new()
                                            .with_variant(EmptyMediaVariant::Icon)
                                            .child(Icon::new(gpui_kit::assets::IconName::Send)),
                                    )
                                    .title(EmptyTitle::new().child(t!("response.empty_title").to_string()))
                                    .description(
                                        EmptyDescription::new().child(t!("response.empty_description").to_string()),
                                    ),
                            )
                            .when_some(shortcut, |empty, shortcut| {
                                empty.content(
                                    EmptyContent::new().child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .text_sm()
                                            .text_color(muted)
                                            .child(shortcut)
                                            .child(t!("editor.send").to_string()),
                                    ),
                                )
                            }),
                    )
                    .into_any_element()
            }
        };
        v_flex()
            .size_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(summary)
            .child(content)
    }
}
