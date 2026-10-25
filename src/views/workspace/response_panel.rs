//! The selected API's response (M7): status, time and size, the body, the
//! headers and cookies, where the time went, and the history.

// The action, named apart from the `Send` trait.
use super::{KEY_CONTEXT, Send as SendAction};
use crate::json_view_local::JsonView;
use crate::states::{ApiState, AppStore};
use crate::this_build;
use crate::views::json_tree::{JsonTree, response_view_switch};
use cyberapi_core::graphql;
use cyberapi_core::media::{self, HEX_LIMIT, ImageType};
use cyberapi_core::model::ContentType;
use cyberapi_core::response::{
    BodyKind, DISPLAY_LIMIT, HttpResponse, RESPONSE_LIMIT, StatusLevel, clip_text, format_duration, format_size,
    status_level, status_text, unwrappable_line,
};
use cyberapi_http::SendError;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::empty::{
    Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::{ScrollableElement as _, Scrollbar, ScrollbarAxis};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, StyledExt as _, WindowExt as _,
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, Hsla, Image, ImageFormat, ImageSource,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription, Window, div, img, px, relative,
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

/// The scales an image steps through, of its own size (F7.4).
const ZOOMS: [f32; 10] = [0.1, 0.25, 0.5, 0.75, 1., 1.5, 2., 3., 4., 8.];

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
    /// The editor holds that response's JSON indented (the pretty view), not
    /// as it came.
    shown_pretty: bool,
    /// The editor holds only the start of the body: this much of it
    /// (`DISPLAY_LIMIT` of text, `HEX_LIMIT` of bytes).
    clipped: Option<usize>,
    /// The editor holds bytes as hex, whose lines never wrap.
    hex: bool,
    /// The shown body as an image to draw, made once for each response.
    image: Option<Arc<Image>>,
    /// The image is drawn at this scale of its own size; `None` fits it to
    /// the panel, never larger than it is.
    zoom: Option<f32>,
    /// The image's bytes (an SVG's text) are shown in place of it.
    image_raw: bool,
    /// Where the scaled image is scrolled to.
    image_scroll: ScrollHandle,
    /// The longest line of what the editor holds, when too long to wrap
    /// (`WRAP_LINE_LIMIT`): that editor scrolls sideways, said in a note.
    long_line: Option<usize>,
    tab: ResponseTab,
    /// The body as a tree, when this response's JSON is shown that way.
    json_tree: Entity<JsonTree>,
    /// The view picked on the panel, bound to one response. Another response
    /// opens in the saved default.
    body_view: Option<(String, JsonView)>,
    /// Whether the body wraps long lines (F14.7).
    wrap: bool,
    _subscriptions: Vec<Subscription>,
}

impl ResponsePanel {
    pub fn new(api: Entity<ApiState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let app = AppStore::global(cx).state().clone();
        let wrap = app.read(cx).prefs().text.wrap;
        let body = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .tab_size(crate::views::json_tab())
                .soft_wrap(wrap)
                .searchable(true)
        });
        let subscriptions = vec![
            cx.observe_in(&api, window, |this, _, window, cx| this.sync(window, cx)),
            cx.observe_in(&app, window, |this, app, window, cx| {
                let wrap = app.read(cx).prefs().text.wrap;
                if wrap != this.wrap {
                    this.wrap = wrap;
                    let wraps = wrap && this.long_line.is_none() && !this.hex;
                    this.body.update(cx, |body, cx| body.set_soft_wrap(wraps, window, cx));
                }
                // A new default view may want the body in the other form.
                this.sync(window, cx);
                this.sync_response_tree(cx);
                cx.notify();
            }),
        ];
        let json_tree = cx.new(JsonTree::new);
        let mut panel = Self {
            api,
            body,
            shown: None,
            shown_pretty: false,
            clipped: None,
            hex: false,
            image: None,
            zoom: None,
            image_raw: false,
            image_scroll: ScrollHandle::new(),
            long_line: None,
            tab: ResponseTab::Body,
            json_tree,
            body_view: None,
            wrap,
            _subscriptions: subscriptions,
        };
        panel.sync(window, cx);
        panel
    }

    /// The view this response was switched to; otherwise the default.
    fn response_view(&self, cx: &App) -> JsonView {
        match (&self.shown, &self.body_view) {
            (Some(id), Some((bound, view))) if bound == id => *view,
            _ => AppStore::global(cx).state().read(cx).json_view(),
        }
    }

    fn response_shows_tree(&self, cx: &App) -> bool {
        self.response_view(cx) == JsonView::Tree
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

    /// Puts the shown response's body in the editor when it, or the form
    /// its view wants it in, changes: indented for the pretty view, as it
    /// came for the raw one (#51) and the tree, which reads the same JSON.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.api.read(cx).shown_response().cloned();
        let id = shown.as_ref().map(|response| response.id.clone());
        let changed = id != self.shown;
        self.shown = id.clone();
        let pretty = self.shown_json(cx) && self.response_view(cx) == JsonView::Pretty;
        if !changed && pretty == self.shown_pretty {
            cx.notify();
            return;
        }
        self.shown_pretty = pretty;
        if changed {
            // Another response opens fitted, as a picture.
            self.zoom = None;
            self.image_raw = false;
            self.image_scroll = ScrollHandle::new();
            self.image = shown.as_ref().and_then(|response| {
                let format = image_format(response.image_type()?)?;
                (response.error.is_none() && !response.body.is_empty())
                    .then(|| Arc::new(Image::from_bytes(format, response.body.clone())))
            });
        }
        let (language, text, clipped, hex) = match &shown {
            Some(response) => editor_text(response, pretty),
            None => ("text", String::new(), None, false),
        };
        self.clipped = clipped;
        self.hex = hex;
        self.long_line = unwrappable_line(&text);
        let wrap = self.wrap && self.long_line.is_none() && !hex;
        self.body = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                // Each line of hex starts with its offset.
                .line_number(!hex)
                .tab_size(crate::views::json_tab())
                .soft_wrap(wrap)
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
            StatusLevel::Redirect => theme.info,
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
                // A failure has no size: only how long it took to fail.
                let measures = match response.status {
                    Some(_) => format!("{} · {}", format_duration(response.latency), body_size(response)),
                    None => format_duration(response.latency),
                };
                (response.id.clone(), color, status, measures, when.to_string())
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
                // F7.2: the size, and how much smaller it came. A request
                // that failed has no answer to measure.
                let size = response.status.map(|_| match response.compression() {
                    Some((wire, percent)) => format!(
                        "{} ({})",
                        body_size(response),
                        t!("response.compressed", size = format_size(wire), percent = percent)
                    ),
                    None => body_size(response),
                });
                h_flex()
                    .gap_3()
                    .items_center()
                    .text_sm()
                    .child(status)
                    .child(div().text_color(muted).child(format_duration(response.latency)))
                    .children(size.map(|size| div().text_color(muted).child(size)))
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
        // F7.4: a picture, or its bytes; and the file, opened or kept.
        let pictured = self.tab == ResponseTab::Body
            && response.error.is_none()
            && response.body_omitted.is_none()
            && self.image.is_some();
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
                            this.child(response_view_switch(
                                "response-json-view",
                                self.response_view(cx),
                                cx.listener(|this, picks: &Vec<usize>, window, cx| {
                                    let (Some(id), Some(view)) = (
                                        this.shown.clone(),
                                        picks.first().and_then(|index| JsonView::ALL.get(*index)),
                                    ) else {
                                        return;
                                    };
                                    this.body_view = Some((id, *view));
                                    this.sync(window, cx);
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
                                // show only its start. Indented when it is
                                // shown so, as it came otherwise.
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let pretty = this.response_view(cx) == JsonView::Pretty;
                                    let text = this.api.read(cx).shown_response().and_then(|response| {
                                        if pretty {
                                            response.body_text()
                                        } else {
                                            response.raw_body_text()
                                        }
                                    });
                                    if let Some(text) = text {
                                        copy(text, window, cx);
                                    }
                                })),
                        ),
                )
            })
            .when(pictured, |bar| {
                bar.suffix(
                    h_flex()
                        .gap_0p5()
                        .items_center()
                        .child(image_view_switch(
                            self.image_raw,
                            cx.listener(|this, picks: &Vec<usize>, _, cx| {
                                this.image_raw = picks.first() == Some(&1);
                                cx.notify();
                            }),
                        ))
                        .child(
                            Button::new("response-open-image")
                                .ghost()
                                .xsmall()
                                .icon(IconName::ExternalLink)
                                .tooltip(t!("response.open_image").to_string())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.api.update(cx, |api, cx| api.view_body(cx));
                                })),
                        )
                        .child(
                            Button::new("response-save-image")
                                .ghost()
                                .xsmall()
                                .icon(gpui_kit::assets::IconName::Download)
                                .tooltip(t!("response.save_body").to_string())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.api.update(cx, |api, cx| api.save_body(cx));
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

    /// The body, under a note when the history did not keep the answer,
    /// one when the answer is a redirect (F14.8) and one when it was longer
    /// than the engine reads.
    fn render_cut_body(&self, response: &HttpResponse, cx: &mut Context<Self>) -> AnyElement {
        let body = self.render_body(response, cx);
        // Where it points, and how to go there by itself: with following
        // on, a send ends past every redirect, so only an answer kept
        // before shows the first.
        let redirect = response.redirects_to().map(|to| {
            let follows = AppStore::global(cx).state().read(cx).prefs().sending.follow_redirects;
            let text = if follows {
                t!("response.redirect", url = to)
            } else {
                t!("response.redirect_unfollowed", url = to)
            };
            let copy_to = Button::new("response-copy-location")
                .ghost()
                .xsmall()
                .icon(IconName::Copy)
                .label(t!("response.copy_location").to_string())
                .on_click(move |_, window, cx| copy(to.clone(), window, cx));
            note(text.to_string(), vec![copy_to], cx)
        });
        let cut = response.truncated.then(|| {
            let limit = format_size(RESPONSE_LIMIT as u64);
            note(t!("response.truncated", size = limit).to_string(), Vec::new(), cx)
        });
        // Not in the history: said, with why and for how long it stays.
        let unkept = (!response.kept()).then(|| {
            let text = if !response.waits_for_sync() {
                t!("response.unkept")
            } else if self.api.read(cx).local_only(cx) {
                t!("response.waiting_local")
            } else {
                t!("response.waiting")
            };
            note(text.to_string(), Vec::new(), cx)
        });
        // A GraphQL answer says it failed in its `errors`, often with a 200.
        let graphql = (response.request.content_type == ContentType::GraphQl)
            .then(|| graphql::response_errors(&response.body))
            .filter(|errors| !errors.is_empty())
            .map(|errors| {
                let shown: Vec<&str> = errors.iter().take(3).map(String::as_str).collect();
                let text = crate::i18n::counted("response.graphql_errors", errors.len());
                let danger = cx.theme().danger;
                v_flex()
                    .px_3()
                    .py_2()
                    .gap_0p5()
                    .text_sm()
                    .text_color(danger)
                    .child(text)
                    .children(shown.into_iter().map(|error| div().text_xs().child(error.to_string())))
                    .into_any_element()
            });
        if redirect.is_none() && cut.is_none() && unkept.is_none() && graphql.is_none() {
            return body;
        }
        v_flex()
            .size_full()
            .children(unkept)
            .children(graphql)
            .children(redirect)
            .children(cut)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    fn render_body(&self, response: &HttpResponse, cx: &mut Context<Self>) -> AnyElement {
        // F5.7: why it failed, as the failure it is.
        if let Some(error) = &response.error {
            let copied = error.clone();
            // F14.9: where to take a certificate no root vouches for.
            let certificate = SendError::is_untrusted_certificate(error).then(|| {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("response.untrusted_certificate").to_string())
            });
            return v_flex()
                .p_3()
                .gap_2()
                .child(Alert::error("response-error", error.clone()).title(t!("response.failed").to_string()))
                .children(certificate)
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
                    self.render_editor()
                };
                // Wrapping it would hold the window (#51): said, and for
                // JSON where it reads indented.
                let long_line = self.long_line.filter(|_| self.wrap && !tree).map(|longest| {
                    let size = format_size(longest as u64);
                    if response.body_kind() == BodyKind::Json {
                        t!("response.long_line_json", size = size)
                    } else {
                        t!("response.long_line", size = size)
                    }
                    .to_string()
                });
                v_flex()
                    .size_full()
                    .when_some(self.clipped, |this, shown| {
                        let text = t!("response.clipped", size = format_size(shown as u64)).to_string();
                        this.child(note(text, vec![save_button(cx)], cx))
                    })
                    .when_some(long_line, |this, text| this.child(note(text, Vec::new(), cx)))
                    .child(fill(body))
                    .into_any_element()
            }
            // F7.4.
            BodyKind::Image if self.image.is_some() => self.render_image(response, cx),
            BodyKind::Image => self.render_bytes(
                viewed_outside(t!("response.image", size = body_size(response)).to_string(), cx),
                cx,
            ),
            BodyKind::Pdf => self.render_bytes(
                viewed_outside(t!("response.pdf", size = body_size(response)).to_string(), cx),
                cx,
            ),
            BodyKind::Binary => self.render_bytes(
                note(
                    t!("response.binary", size = body_size(response)).to_string(),
                    vec![save_button(cx)],
                    cx,
                ),
                cx,
            ),
        }
    }

    /// The editor, read only, filling what is left of the panel.
    fn render_editor(&self) -> AnyElement {
        Editor::new(&self.body)
            .w_full()
            .h_full()
            .readonly(true)
            .bordered(false)
            .into_any_element()
    }

    /// A body that is not text: what it is and what can be done with it,
    /// then its bytes as hex.
    fn render_bytes(&self, about: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .size_full()
            .child(about)
            .when_some(self.clipped, |this, shown| {
                let text = t!("response.clipped", size = format_size(shown as u64)).to_string();
                this.child(note(text, Vec::new(), cx))
            })
            .child(fill(self.render_editor()))
            .into_any_element()
    }

    /// F7.4: the picture, fitted to the panel or at a scale of its own
    /// size, under its format and size; or its bytes in its place.
    fn render_image(&self, response: &HttpResponse, cx: &mut Context<Self>) -> AnyElement {
        let Some(image) = self.image.clone() else {
            return div().into_any_element();
        };
        if self.image_raw {
            return v_flex()
                .size_full()
                .when_some(self.clipped, |this, shown| {
                    let text = t!("response.clipped", size = format_size(shown as u64)).to_string();
                    this.child(note(text, Vec::new(), cx))
                })
                .child(fill(self.render_editor()))
                .into_any_element();
        }
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let kind = response.image_type().unwrap_or(ImageType::Other);
        let size = response.image_size();
        let about = match size {
            Some((width, height)) => format!("{} · {width} × {height} · {}", kind.label(), body_size(response)),
            None => format!("{} · {}", kind.label(), body_size(response)),
        };
        let picture = img(ImageSource::Image(image)).with_fallback(move || {
            div()
                .p_3()
                .text_sm()
                .text_color(muted)
                .child(t!("response.image_broken").to_string())
                .into_any_element()
        });
        let view = match (self.zoom, size) {
            (Some(scale), Some((width, height))) => {
                let (width, height) = (px(width as f32 * scale), px(height as f32 * scale));
                // As big as the picture and its margin, so both ways scroll.
                // The kit's `overflow_scrollbar` did not take the wheel here.
                let margin = px(12.);
                div()
                    .size_full()
                    .relative()
                    .child(
                        div()
                            .id("response-image")
                            .size_full()
                            .overflow_scroll()
                            .track_scroll(&self.image_scroll)
                            .child(
                                div()
                                    .flex_none()
                                    .w(width + margin * 2.)
                                    .h(height + margin * 2.)
                                    .p(margin)
                                    .child(picture.size_full()),
                            ),
                    )
                    .child(
                        div().absolute().inset_0().child(
                            Scrollbar::new(&self.image_scroll)
                                .axis(ScrollbarAxis::Both)
                                .viewport_from_layout(),
                        ),
                    )
                    .into_any_element()
            }
            _ => div()
                .size_full()
                .p_3()
                .flex()
                .items_start()
                .justify_center()
                .child(picture.max_w_full().max_h_full())
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(32.))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .flex_none()
                    .border_b_1()
                    .border_color(border)
                    .text_xs()
                    .text_color(muted)
                    .child(div().flex_1().min_w_0().truncate().child(about))
                    // Its own size is what a scale is of: without it, fitted only.
                    .children(size.map(|_| self.render_zoom(cx))),
            )
            .child(div().flex_1().min_h_0().child(view))
            .into_any_element()
    }

    /// Smaller, larger, and between fitted and its own size.
    fn render_zoom(&self, cx: &mut Context<Self>) -> AnyElement {
        let scale = self.zoom.unwrap_or(1.);
        let (smallest, largest) = (ZOOMS[0], ZOOMS[ZOOMS.len() - 1]);
        let (label, tooltip) = match self.zoom {
            None => (
                t!("response.zoom_fit").to_string(),
                t!("response.zoom_actual").to_string(),
            ),
            Some(scale) => (
                format!("{}%", (scale * 100.).round()),
                t!("response.zoom_to_fit").to_string(),
            ),
        };
        h_flex()
            .gap_0p5()
            .items_center()
            .child(
                Button::new("response-zoom-out")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Minus)
                    .tooltip(t!("response.zoom_out").to_string())
                    .disabled(self.zoom.is_some() && scale <= smallest)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.zoom = Some(zoom_step(this.zoom.unwrap_or(1.), false));
                        cx.notify();
                    })),
            )
            .child(
                Button::new("response-zoom")
                    .ghost()
                    .xsmall()
                    .label(label)
                    .tooltip(tooltip)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.zoom = match this.zoom {
                            Some(_) => None,
                            None => Some(1.),
                        };
                        cx.notify();
                    })),
            )
            .child(
                Button::new("response-zoom-in")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip(t!("response.zoom_in").to_string())
                    .disabled(scale >= largest)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.zoom = Some(zoom_step(this.zoom.unwrap_or(1.), true));
                        cx.notify();
                    })),
            )
            .into_any_element()
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
        rows.push((t!("response.api_id").to_string(), response.api.to_string()));
        // A request that never connected has no peer.
        if !stats.remote_addr.is_empty() {
            rows.push((t!("response.remote").to_string(), stats.remote_addr.clone()));
        }
        if !stats.version.is_empty() {
            rows.push((t!("response.version").to_string(), stats.version.clone()));
        }
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
        // Nothing was timed (it failed before it went out): no line of dashes.
        let timed = span > 0.;
        v_flex()
            .id("response-info")
            .size_full()
            .overflow_y_scrollbar()
            .p_2()
            .gap_3()
            .child(field_rows("response-info-fields", rows, cx))
            .when(timed, |this| this.child(timeline))
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
fn image_format(kind: ImageType) -> Option<ImageFormat> {
    match kind {
        ImageType::Png => Some(ImageFormat::Png),
        ImageType::Jpeg => Some(ImageFormat::Jpeg),
        ImageType::Gif => Some(ImageFormat::Gif),
        ImageType::Webp => Some(ImageFormat::Webp),
        ImageType::Svg => Some(ImageFormat::Svg),
        ImageType::Bmp => Some(ImageFormat::Bmp),
        ImageType::Tiff => Some(ImageFormat::Tiff),
        ImageType::Ico => Some(ImageFormat::Ico),
        ImageType::Pnm => Some(ImageFormat::Pnm),
        ImageType::Avif | ImageType::Heic | ImageType::Other => None,
    }
}

/// What the editor holds for `response`, in which grammar, how much of the
/// body that is when not all of it, and whether it is hex: JSON and text as
/// the view wants them (#51), an SVG's text, any other body's bytes.
fn editor_text(response: &HttpResponse, pretty: bool) -> (&'static str, String, Option<usize>, bool) {
    let text = match response.body_kind() {
        BodyKind::Json | BodyKind::Text if pretty => response.body_text(),
        BodyKind::Json | BodyKind::Text => response.raw_body_text(),
        BodyKind::Image if response.image_type() == Some(ImageType::Svg) => {
            Some(String::from_utf8_lossy(&response.body).into_owned())
        }
        BodyKind::Image | BodyKind::Pdf | BodyKind::Binary => {
            let shown = &response.body[..response.body.len().min(HEX_LIMIT)];
            let clipped = (shown.len() < response.body.len()).then_some(HEX_LIMIT);
            return ("text", media::hex_dump(shown), clipped, true);
        }
    };
    // A body that would make the editor crawl shows its start (#17).
    let (text, clipped) = clip_text(text.unwrap_or_default(), DISPLAY_LIMIT);
    (language_of(response), text, clipped.then_some(DISPLAY_LIMIT), false)
}

/// The next scale of [`ZOOMS`] past `scale`, larger or smaller.
fn zoom_step(scale: f32, larger: bool) -> f32 {
    let next = if larger {
        ZOOMS.iter().find(|step| **step > scale + f32::EPSILON)
    } else {
        ZOOMS.iter().rev().find(|step| **step < scale - f32::EPSILON)
    };
    next.copied().unwrap_or(scale)
}

/// The image, or its bytes (an SVG's text) in its place.
fn image_view_switch(raw: bool, on_click: impl Fn(&Vec<usize>, &mut Window, &mut App) + 'static) -> ButtonGroup {
    ButtonGroup::new("response-image-view")
        .compact()
        .small()
        .outline()
        .child(
            Button::new(("response-image-view", 0usize))
                .label(t!("response.image_view").to_string())
                .selected(!raw),
        )
        .child(
            Button::new(("response-image-view", 1usize))
                .label(t!("editor.json_view_raw").to_string())
                .selected(raw),
        )
        .on_click(on_click)
}

/// `body` over all the room left below the notes.
fn fill(body: AnyElement) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .relative()
        .child(div().absolute().inset_0().child(body))
        .into_any_element()
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
                    ResponseTab::Body => self.render_cut_body(response, cx),
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
