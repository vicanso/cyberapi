//! The selected API's response (M7): status, time and size, the body, the
//! headers and cookies, where the time went, and the history.

use crate::states::ApiState;
use cyberapi_core::response::{
    BodyKind, DISPLAY_LIMIT, HttpResponse, StatusLevel, clip_text, format_duration, format_size, status_level,
    status_text,
};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, Hsla, Image, ImageFormat, ImageSource,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Subscription, Window, div, img, px,
};
use rust_i18n::t;
use std::sync::Arc;

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
        let subscriptions = vec![cx.observe_in(&api, window, |this, _, window, cx| this.sync(window, cx))];
        let mut panel = Self {
            api,
            body,
            shown: None,
            clipped: false,
            tab: ResponseTab::Body,
            _subscriptions: subscriptions,
        };
        panel.sync(window, cx);
        panel
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
            Some(response) => match response.body_kind() {
                BodyKind::Json => ("json", response.body_text().unwrap_or_default()),
                _ => ("text", response.body_text().unwrap_or_default()),
            },
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
        let responses: Vec<HttpResponse> = api.responses().to_vec();
        let sending = api.sending_since();
        let shown_id = response.map(|r| r.id.clone());
        let this = cx.entity().downgrade();
        let history_menu = move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
            let menu = responses.iter().fold(menu, |menu, response| {
                let (this, id) = (this.clone(), response.id.clone());
                let status = match response.status {
                    Some(status) => status.to_string(),
                    None => t!("response.failed").to_string(),
                };
                let label = format!(
                    "{status} · {} · {}",
                    format_size(response.size()),
                    response.created_at.get().format("%m-%d %H:%M:%S")
                );
                menu.item(
                    PopupMenuItem::new(label)
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
                        format_size(response.body_omitted.unwrap_or(response.body.len() as u64)),
                        t!("response.compressed", size = format_size(wire), percent = percent)
                    ),
                    None => format_size(response.size()),
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
        TabBar::new("response-tabs")
            .underline()
            .small()
            .px_2()
            .selected_index(TABS.iter().position(|tab| *tab == self.tab).unwrap_or(0))
            .children(TABS.iter().map(|tab| Tab::new().label(label(tab))))
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
            return div()
                .p_3()
                .child(Alert::error("response-error", error.clone()).title(t!("response.failed").to_string()))
                .into_any_element();
        }
        if let Some(size) = response.body_omitted {
            return note(
                t!("response.omitted", size = format_size(size)).to_string(),
                Vec::new(),
                cx,
            );
        }
        match response.body_kind() {
            BodyKind::Json | BodyKind::Text => v_flex()
                .size_full()
                .when(self.clipped, |this| {
                    let shown = format_size(DISPLAY_LIMIT as u64);
                    let text = t!("response.clipped", size = shown).to_string();
                    this.child(note(text, vec![save_button(cx)], cx))
                })
                .child(
                    div().flex_1().min_h_0().relative().child(
                        div()
                            .absolute()
                            .inset_0()
                            .child(Editor::new(&self.body).w_full().h_full().readonly(true).bordered(false)),
                    ),
                )
                .into_any_element(),
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

    fn render_headers(rows: Vec<(String, String)>, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .id("response-headers")
            .size_full()
            .overflow_y_scrollbar()
            .p_3()
            .gap_1()
            .text_sm()
            .children(rows.into_iter().map(|(name, value)| {
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(div().w(px(180.)).flex_none().text_color(muted).child(name))
                    .child(div().flex_1().min_w_0().child(value))
            }))
            .into_any_element()
    }

    /// F7.7: what the request was and where its time went.
    fn render_info(&self, response: &HttpResponse, cx: &mut Context<Self>) -> AnyElement {
        let stats = &response.stats;
        let mut rows = vec![
            (t!("response.api_id").to_string(), response.api.to_string()),
            (t!("response.remote").to_string(), stats.remote_addr.clone()),
        ];
        if stats.is_https {
            rows.push((t!("response.cipher").to_string(), stats.cipher.clone()));
        }
        for (label, duration) in [
            (t!("response.dns"), stats.dns),
            (t!("response.tcp"), stats.tcp),
            (t!("response.tls"), stats.tls),
            (t!("response.send"), stats.send),
            (t!("response.server"), stats.server_processing),
            (t!("response.transfer"), stats.content_transfer),
            (t!("response.total"), stats.total),
        ] {
            // A phase that did not happen: TLS over plain HTTP, DNS for an address.
            let shown = if duration.is_zero() {
                "—".to_string()
            } else {
                format_duration(duration)
            };
            rows.push((label.to_string(), shown));
        }
        Self::render_headers(rows, cx)
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

fn body_size(response: &HttpResponse) -> String {
    format_size(response.body.len() as u64)
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let response = self.api.read(cx).shown_response().cloned();
        let sending = self.api.read(cx).sending_since().is_some();
        let summary = self.render_summary(response.as_ref(), cx);
        let content: AnyElement = match &response {
            Some(response) => {
                let content = match self.tab {
                    ResponseTab::Body => self.render_body(response, cx),
                    ResponseTab::Headers => Self::render_headers(
                        response
                            .display_headers()
                            .into_iter()
                            .map(|(name, values)| (name, values.join("\n")))
                            .collect(),
                        cx,
                    ),
                    ResponseTab::Cookies => Self::render_headers(response.cookies(), cx),
                    ResponseTab::Info => self.render_info(response, cx),
                };
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_tabs(response, cx))
                    .child(div().flex_1().min_h_0().child(content))
                    .into_any_element()
            }
            None if sending => div().flex_1().into_any_element(),
            None => v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .child(
                    Empty::new().header(
                        EmptyHeader::new()
                            .media(
                                EmptyMedia::new()
                                    .with_variant(EmptyMediaVariant::Icon)
                                    .child(Icon::new(gpui_kit::assets::IconName::Send)),
                            )
                            .title(EmptyTitle::new().child(t!("response.empty_title").to_string()))
                            .description(EmptyDescription::new().child(t!("response.empty_description").to_string())),
                    ),
                )
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(summary)
            .child(content)
    }
}
