//! The selected API's response (M7): status, time and size, the body, the
//! headers and cookies, where the time went, and the history.

// The action, named apart from the `Send` trait.
use super::event_stream::EventStream;
use super::{KEY_CONTEXT, Send as SendAction};
use crate::json_view_local::JsonView;
use crate::states::{ApiState, AppStore, WorkspaceState};
use crate::this_build;
use crate::views::json_tree::{JsonTree, response_view_switch};
use cyberapi_core::code::CodeTarget;
use cyberapi_core::grpc::GrpcStatus;
use cyberapi_core::media::{self, HEX_LIMIT, ImageType};
use cyberapi_core::model::{ContentType, HttpMethod};
use cyberapi_core::response::{
    BodyKind, HeaderSource, HttpResponse, RESPONSE_LIMIT, SentHeader, SentRequest, StatusLevel, StreamEnd, TREE_LIMIT,
    clip_text, format_duration, format_size, status_level, status_text, unwrappable_line,
};
use cyberapi_http::SendError;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::empty::{
    Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
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
    Anchor, AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, FocusHandle, Hsla, Image, ImageFormat,
    ImageSource, InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription, Task, Window, div, img, px, relative,
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
    /// The whole exchange as `curl -v` prints it (#51).
    Wire,
}

/// A history entry as the menu lists it, and the project's recent list
/// (D4).
#[derive(Clone)]
pub(super) struct HistoryRow {
    pub(super) id: String,
    color: Hsla,
    status: String,
    measures: String,
    when: String,
    method: HttpMethod,
    path: String,
}

impl HistoryRow {
    /// Its status in its colour, how long it took, how big it was, when.
    pub(super) fn of(response: &HttpResponse, today: chrono::NaiveDate, cx: &App) -> Self {
        let (status, color) = match (response.grpc_status(), response.status) {
            (Some(grpc), _) => (grpc.code.to_string(), grpc_color(&grpc, cx)),
            (None, Some(code)) => (code.to_string(), status_color(code, cx)),
            (None, None) => (t!("response.failed").to_string(), cx.theme().danger),
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
        HistoryRow {
            id: response.id.clone(),
            color,
            status,
            measures,
            when: when.to_string(),
            method: response.request.method,
            path: response.sent_path(),
        }
    }

    /// Two lines: the answer, then what it sent; `name` before the path,
    /// the API it was of, where that is not plain.
    pub(super) fn render(&self, name: Option<&str>, width: f32, cx: &App) -> impl IntoElement + use<> {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .w(px(width))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .w(px(36.))
                            .font_semibold()
                            .text_color(self.color)
                            .child(self.status.clone()),
                    )
                    .child(div().flex_1().child(self.measures.clone()))
                    .child(div().text_color(muted).child(self.when.clone())),
            )
            // What it sent: the method, and where.
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .text_xs()
                    .child(
                        div()
                            .w(px(36.))
                            .flex_none()
                            .font_semibold()
                            .text_color(super::method_color(self.method, cx))
                            .child(super::method_label(self.method)),
                    )
                    .when_some(name.map(str::to_string), |row, name| {
                        row.child(div().flex_none().max_w(px(width * 0.4)).truncate().child(name))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(muted)
                            .child(self.path.clone()),
                    ),
            )
    }
}

/// The scales an image steps through, of its own size (F7.4).
const ZOOMS: [f32; 10] = [0.1, 0.25, 0.5, 0.75, 1., 1.5, 2., 3., 4., 8.];

const TABS: [ResponseTab; 5] = [
    ResponseTab::Body,
    ResponseTab::Headers,
    ResponseTab::Cookies,
    ResponseTab::Info,
    ResponseTab::Wire,
];
/// A gRPC call's: no cookies, metadata for headers (#73).
const GRPC_TABS: [ResponseTab; 4] = [
    ResponseTab::Body,
    ResponseTab::Headers,
    ResponseTab::Info,
    ResponseTab::Wire,
];

fn tabs_of(response: &HttpResponse) -> &'static [ResponseTab] {
    if response.request.method == HttpMethod::Grpc {
        &GRPC_TABS
    } else {
        &TABS
    }
}

/// What the response filter found (#63): in which response, for which
/// path — the matches, or why there are none to show; `None` while it runs.
struct Found {
    response: String,
    path: String,
    outcome: Option<Result<Arc<serde_json::Value>, NotFiltered>>,
}

/// What a filter found that is nothing: no match.
fn no_match(matches: &serde_json::Value) -> bool {
    matches.as_array().is_some_and(Vec::is_empty)
}

#[derive(Clone)]
enum NotFiltered {
    /// The path does not read: why.
    Path(String),
    /// The body is not one JSON document (NDJSON, say).
    NotJson,
    TooLarge(usize),
}

pub struct ResponsePanel {
    api: Entity<ApiState>,
    /// Keeps each API's response filter.
    workspace: Entity<WorkspaceState>,
    focus: FocusHandle,
    /// #63: the JSONPath a JSON body is narrowed by, in Pretty and Tree.
    filter: Entity<InputState>,
    /// The filter's field shows: opened, or a filter kept for this API.
    filtering: bool,
    /// #63: the field that finds text in `body` or `wire`. Driven here rather
    /// than through the kit's search overlay, whose `anchor_offset` check
    /// re-selects the query after the first character typed.
    search: Entity<InputState>,
    /// Whether the find bar is open.
    searching: bool,
    /// Whether the find bar matches without regard to case.
    search_case_insensitive: bool,
    /// #58: the sent request's secrets show unmasked.
    reveal: bool,
    found: Option<Found>,
    _finding: Option<Task<()>>,
    /// The path whose matches the editor holds; `None` for the whole body.
    shown_filter: Option<String>,
    /// The body as text, read only, for selecting and searching.
    body: Entity<EditorState>,
    /// #51: the whole exchange as text, read only.
    wire: Entity<EditorState>,
    /// The response the wire editor holds, whether its secrets showed, and
    /// how many bytes of the exchange it holds.
    wire_of: Option<(String, bool, usize)>,
    /// The wire editor holds only the exchange's start.
    wire_clipped: bool,
    /// Which response the body editor holds.
    shown: Option<String>,
    /// The editor holds that response's JSON indented (the pretty view), not
    /// as it came.
    shown_pretty: bool,
    /// The editor holds only the start of the body: this much of it
    /// (the settings' display limit of text, `HEX_LIMIT` of bytes).
    clipped: Option<usize>,
    /// How many bytes of a text body, and of the wire view, the editors
    /// hold. From the settings (`SendPrefs::display_limit`).
    display_limit: usize,
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
    /// The response the tree was last given, and the filter it was given
    /// through.
    tree_of: Option<(String, Option<String>)>,
    /// The view picked on the panel, bound to one response. Another response
    /// opens in the saved default.
    body_view: Option<(String, JsonView)>,
    /// Whether the body wraps long lines (F14.7).
    wrap: bool,
    /// An event stream's events: the shown response's, or the live one's.
    events: Entity<EventStream>,
    /// The event stream shown as it came rather than as events, bound to
    /// one response.
    stream_raw: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl ResponsePanel {
    pub fn new(
        workspace: Entity<WorkspaceState>,
        api: Entity<ApiState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let app = AppStore::global(cx).state().clone();
        let kept = workspace.read(cx).response_filter(api.read(cx).api()).to_string();
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("response.filter_placeholder").to_string())
                .clean_on_escape()
                .default_value(kept.clone())
        });
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("response.search_placeholder").to_string()));
        let wrap = app.read(cx).prefs().text.wrap;
        let display_limit = app.read(cx).prefs().sending.display_limit();
        let body = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .tab_size(crate::views::json_tab())
                .soft_wrap(wrap)
                .searchable(false)
        });
        let wire = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("text")
                .line_number(false)
                .soft_wrap(wrap)
                .searchable(false)
        });
        let subscriptions = vec![
            cx.observe_in(&api, window, |this, _, window, cx| this.sync(window, cx)),
            cx.subscribe_in(&filter, window, |this, filter, event, window, cx| {
                if let InputEvent::Change = event {
                    let text = filter.read(cx).value().to_string();
                    let api = this.api.read(cx).api().clone();
                    this.workspace
                        .update(cx, |workspace, cx| workspace.set_response_filter(&api, &text, cx));
                    this.sync(window, cx);
                }
            }),
            cx.subscribe_in(&search, window, |this, _, event, _, cx| match event {
                InputEvent::Change => {
                    this.apply_search(true, cx);
                    cx.notify();
                }
                InputEvent::PressEnter { shift, .. } => {
                    this.step_search(!shift, cx);
                    cx.notify();
                }
                _ => {}
            }),
            cx.observe_in(&app, window, |this, app, window, cx| {
                let wrap = app.read(cx).prefs().text.wrap;
                if wrap != this.wrap {
                    this.wrap = wrap;
                    let wraps = wrap && this.long_line.is_none() && !this.hex;
                    this.body.update(cx, |body, cx| body.set_soft_wrap(wraps, window, cx));
                    // Set again, wrapped as it now says.
                    this.wire_of = None;
                }
                // A new default view may want the body in the other form.
                this.sync(window, cx);
                this.sync_response_tree(cx);
                cx.notify();
            }),
        ];
        let json_tree = cx.new(|cx| JsonTree::new(cx).for_response());
        let events = {
            let api = api.clone();
            cx.new(|cx| EventStream::new(api, cx))
        };
        let mut panel = Self {
            api,
            workspace,
            focus: cx.focus_handle(),
            filter,
            filtering: !kept.is_empty(),
            search,
            searching: false,
            search_case_insensitive: true,
            reveal: false,
            found: None,
            _finding: None,
            shown_filter: None,
            body,
            wire,
            wire_of: None,
            wire_clipped: false,
            shown: None,
            shown_pretty: false,
            clipped: None,
            display_limit,
            hex: false,
            image: None,
            zoom: None,
            image_raw: false,
            image_scroll: ScrollHandle::new(),
            long_line: None,
            tab: ResponseTab::Body,
            json_tree,
            tree_of: None,
            body_view: None,
            wrap,
            events,
            stream_raw: None,
            _subscriptions: subscriptions,
        };
        panel.sync(window, cx);
        panel
    }

    /// The body editor's text. Tests read it to see which response the panel shows.
    #[cfg(test)]
    pub(super) fn shown_body(&self, cx: &App) -> String {
        self.body.read(cx).value().to_string()
    }

    #[cfg(test)]
    pub(super) fn focus_body(&self, window: &mut Window, cx: &mut App) {
        self.body.update(cx, |body, cx| body.focus(window, cx));
    }

    #[cfg(test)]
    pub(super) fn search_query(&self, cx: &App) -> String {
        self.search.read(cx).value().to_string()
    }

    #[cfg(test)]
    pub(super) fn search_matches(&self, cx: &App) -> usize {
        self.body.read(cx).search_session().matcher.len()
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

    /// The filter that applies now: one typed, on a JSON body shown Pretty
    /// or as a tree.
    fn active_filter(&self, cx: &App) -> Option<String> {
        let path = self.filter.read(cx).value().trim().to_string();
        (self.filtering && !path.is_empty() && self.shown_json(cx) && self.response_view(cx) != JsonView::Raw)
            .then_some(path)
    }

    /// What the active filter found in the shown response, once found.
    fn found_now(&self, cx: &App) -> Option<&Found> {
        let path = self.active_filter(cx)?;
        self.found
            .as_ref()
            .filter(|found| Some(&found.response) == self.shown.as_ref() && found.path == path)
    }

    /// The matches of the active filter, when it found them.
    fn matches(&self, cx: &App) -> Option<Arc<serde_json::Value>> {
        self.found_now(cx)?.outcome.as_ref()?.as_ref().ok().cloned()
    }

    /// Runs the active filter over the shown body, away from the window,
    /// unless it has run already; the body shows again once it has.
    fn find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(path), Some(response)) = (self.active_filter(cx), self.api.read(cx).shown_response().cloned()) else {
            return;
        };
        let running = self
            .found
            .as_ref()
            .is_some_and(|found| found.response == response.id && found.path == path);
        if running {
            return;
        }
        self.found = Some(Found {
            response: response.id.clone(),
            path: path.clone(),
            outcome: None,
        });
        let finding = cx.background_spawn({
            let path = path.clone();
            async move {
                if response.body.len() > TREE_LIMIT {
                    return Err(NotFiltered::TooLarge(response.body.len()));
                }
                let doc: serde_json::Value =
                    serde_json::from_slice(&response.body).map_err(|_| NotFiltered::NotJson)?;
                cyberapi_core::json_filter::filter(&doc, &path)
                    .map(Arc::new)
                    .map_err(NotFiltered::Path)
            }
        });
        let response_id = self.shown.clone().unwrap_or_default();
        self._finding = Some(cx.spawn_in(window, async move |this, cx| {
            let outcome = finding.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.found = Some(Found {
                    response: response_id,
                    path,
                    outcome: Some(outcome),
                });
                this.sync(window, cx);
            });
        }));
    }

    /// The editor's and the tree's filter: the active one's path once it
    /// found what to show; the whole body when there is none, or it found
    /// nothing to show — no match, which the status line says (#74); what
    /// they hold now while it runs.
    fn wanted_filter(&self, cx: &App) -> Option<String> {
        let path = self.active_filter(cx)?;
        match self.found_now(cx).map(|found| &found.outcome) {
            Some(Some(Ok(matches))) if !no_match(matches) => Some(path),
            Some(None) => self.shown_filter.clone(),
            _ => None,
        }
    }

    /// Gives the tree the shown response's whole body, not the part the
    /// text views show (#57) — or what the filter found in it (#63).
    fn sync_response_tree(&mut self, cx: &mut Context<Self>) {
        // A filter still running: the tree stays as it is till it is done.
        let running = matches!(self.found_now(cx).map(|found| &found.outcome), Some(None));
        if !(self.shown_json(cx) && self.response_shows_tree(cx)) || running {
            return;
        }
        let wanted = self.shown.clone().map(|id| (id, self.wanted_filter(cx)));
        if self.tree_of == wanted {
            return;
        }
        self.tree_of = wanted;
        if let Some(matches) = self.matches(cx).filter(|_| self.wanted_filter(cx).is_some()) {
            let text = matches.to_string();
            self.json_tree.update(cx, |tree, cx| tree.set_text(&text, cx));
            return;
        }
        let Some(response) = self.api.read(cx).shown_response() else {
            return;
        };
        if response.body.len() > TREE_LIMIT {
            let size = format_size(response.body.len() as u64);
            let reason = t!("response.json_tree_too_large", size = size).to_string().into();
            self.json_tree.update(cx, |tree, cx| tree.set_unavailable(reason, cx));
            return;
        }
        let text = String::from_utf8_lossy(&response.body).into_owned();
        self.json_tree.update(cx, |tree, cx| tree.set_text(&text, cx));
    }

    /// Puts the shown response's body in the editor when it, or the form
    /// its view wants it in, changes: indented for the pretty view, as it
    /// came for the raw one (#51) and the tree, which reads the same JSON.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // An event stream's events: the one coming now, or the shown one's.
        let live = self.api.read(cx).live().is_some();
        let shown = self.api.read(cx).shown_response().cloned();
        match &shown {
            _ if live => self.events.update(cx, |events, cx| events.show_live(cx)),
            Some(response) if shows_events(response) => {
                self.events.update(cx, |events, cx| events.show(response, cx));
            }
            _ => {}
        }
        let id = shown.as_ref().map(|response| response.id.clone());
        let changed = id != self.shown;
        self.shown = id.clone();
        self.find(window, cx);
        let pretty = self.shown_json(cx) && self.response_view(cx) == JsonView::Pretty;
        let filtered = self.wanted_filter(cx).filter(|_| pretty);
        // The amount shown can change while this response stays open.
        let limit = AppStore::global(cx).state().read(cx).prefs().sending.display_limit();
        let limit_changed = limit != self.display_limit;
        self.display_limit = limit;
        if !changed && pretty == self.shown_pretty && filtered == self.shown_filter && !limit_changed {
            self.sync_response_tree(cx);
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
        let matches = self.matches(cx).filter(|_| filtered.is_some());
        // #74: the filter the editor shows the matches of — none while it
        // runs on a new response, whose whole body shows meanwhile, so the
        // matches replace it once found.
        self.shown_filter = filtered.filter(|_| matches.is_some());
        let limit = self.display_limit;
        let (language, text, clipped, hex) = match (&shown, matches) {
            // #63: what the filter found, indented.
            (Some(_), Some(matches)) => {
                let (text, clipped) = clip_text(cyberapi_core::format::json(&*matches), limit);
                ("json", text, clipped.then_some(limit), false)
            }
            (Some(response), None) => editor_text(response, pretty, limit),
            (None, _) => ("text", String::new(), None, false),
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
                .searchable(false)
                .default_value(text)
        });
        self.sync_response_tree(cx);
        self.apply_search(false, cx);
        cx.notify();
    }

    fn render_summary(&self, response: Option<&HttpResponse>, cx: &mut Context<Self>) -> AnyElement {
        let api = self.api.read(cx);
        let muted = cx.theme().muted_foreground;
        let today = chrono::Local::now().date_naive();
        // Each answer as a line of the history.
        let entries: Vec<HistoryRow> = api
            .responses()
            .iter()
            .map(|response| HistoryRow::of(response, today, cx))
            .collect();
        let sending = api.sending_since();
        // A1: how many of the API's assertions this answer holds up to.
        let tally = api.checks().map(|outcomes| cyberapi_core::assertions::tally(&outcomes));
        // An event stream coming: its status, how much of it, whether it
        // is being stopped.
        let live = api
            .live()
            .map(|live| (live.status, live.events.len(), live.body.len(), live.stopping));
        let shown_id = response.map(|r| r.id.clone());
        // F3: a viewer's examples would be kept nowhere.
        let editable = api.editable();
        let this = cx.entity().downgrade();
        let history_menu = move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
            // F7.10: picking one also puts its request back (#68).
            let menu = menu.label(t!("response.history_restores").to_string());
            let menu = entries.iter().fold(menu, |menu, row| {
                let (this, id) = (this.clone(), row.id.clone());
                let row = row.clone();
                menu.item(
                    PopupMenuItem::element(move |_, cx| row.render(None, 300., cx))
                        .checked(shown_id.as_deref() == Some(id.as_str()))
                        .on_click(move |_, _, cx| {
                            let _ = this.update(cx, |this, cx| {
                                this.api.update(cx, |api, cx| api.choose_response(&id, cx));
                            });
                        }),
                )
            });
            let (clear, compare, recent) = (this.clone(), this.clone(), this.clone());
            let example = this.clone();
            let comparable = entries.len() >= 2;
            menu.separator()
                // F3: the response shown, kept with the API.
                .when(editable && shown_id.is_some(), |menu| {
                    menu.item(
                        PopupMenuItem::new(t!("docs.keep_example").to_string()).on_click(move |_, window, cx| {
                            if let Some(this) = example.upgrade() {
                                let api = this.read(cx).api.clone();
                                keep_example(api, window, cx);
                            }
                        }),
                    )
                })
                // D4.
                .when(comparable, |menu| {
                    menu.item(
                        PopupMenuItem::new(t!("compare.open").to_string()).on_click(move |_, window, cx| {
                            if let Some(this) = compare.upgrade() {
                                let api = this.read(cx).api.clone();
                                super::compare_dialog::open_compare_dialog(api, window, cx);
                            }
                        }),
                    )
                })
                .item(
                    PopupMenuItem::new(t!("recent.open").to_string()).on_click(move |_, window, cx| {
                        let workspace = recent
                            .upgrade()
                            .and_then(|this| this.read(cx).api.read(cx).workspace().upgrade());
                        if let Some(workspace) = workspace {
                            super::recent_dialog::open_recent_dialog(workspace, window, cx);
                        }
                    }),
                )
                .item(
                    PopupMenuItem::new(t!("response.clear_history").to_string()).on_click(move |_, _, cx| {
                        let _ = clear.update(cx, |this, cx| this.api.update(cx, |api, cx| api.clear_history(cx)));
                    }),
                )
        };
        let status: AnyElement = match (sending, response) {
            (Some(started), _) => {
                let text = match live {
                    Some((.., true)) => t!("response.stream_stopping").to_string(),
                    Some((_, events, size, false)) => format!(
                        "{} · {} · {}",
                        crate::i18n::counted("response.events_count", events),
                        format_size(size as u64),
                        super::waited(started, true)
                    ),
                    None => format!("{} {}", t!("response.sending"), super::waited(started, false)),
                };
                h_flex()
                    .gap_2()
                    .items_center()
                    .when_some(live.filter(|(status, ..)| *status > 0), |this, (status, ..)| {
                        this.child(pill(
                            format!("{status} {}", status_text(status)),
                            status_color(status, cx),
                        ))
                    })
                    .child(Spinner::new().small())
                    .child(div().text_sm().text_color(muted).child(text))
                    .into_any_element()
            }
            (None, Some(response)) => {
                // A gRPC call's own status, where it gave one.
                let shown = match (response.grpc_status(), response.status) {
                    (Some(grpc), _) => Some((format!("{} {}", grpc.code, grpc.name()), grpc_color(&grpc, cx))),
                    (None, Some(code)) => Some((format!("{code} {}", status_text(code)), status_color(code, cx))),
                    (None, None) => None,
                };
                let status = match shown {
                    Some((text, color)) => pill(text, color),
                    None => pill(t!("response.failed").to_string(), cx.theme().danger),
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
                // A1: green when every one held; the assertions on a click.
                let checked = tally.map(|(passed, total)| {
                    let color = if passed == total {
                        cx.theme().green
                    } else {
                        cx.theme().danger
                    };
                    let (workspace, api) = (self.workspace.downgrade(), self.api.clone());
                    div()
                        .id("response-assertions")
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            let _ = workspace.update(cx, |workspace, cx| {
                                workspace.set_tab(&api, cyberapi_core::prefs::RequestTab::Assertions, cx)
                            });
                        })
                        .child(pill(
                            t!("assertions.passed", passed = passed, total = total).to_string(),
                            color,
                        ))
                });
                h_flex()
                    .gap_3()
                    .items_center()
                    .text_sm()
                    .child(status)
                    .child(div().text_color(muted).child(format_duration(response.latency)))
                    .children(size.map(|size| div().text_color(muted).child(size)))
                    .children(checked)
                    .into_any_element()
            }
            (None, None) => div().into_any_element(),
        };
        let filterable = sending.is_none()
            && response.is_some_and(|response| !shows_events(response))
            && self.shown_json(cx)
            && self.response_view(cx) != JsonView::Raw;
        let searchable = self.searchable_editor(cx).is_some();
        h_flex()
            .gap_2()
            .px_3()
            .h(px(44.))
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().flex_1().min_w_0().child(status))
            // #63: finding text in the shown editor, and narrowing a JSON
            // body in Pretty and Tree. Here, not by the view switch: the
            // tab bar has no room left.
            .when(searchable, |this| {
                this.child(
                    Button::new("response-search")
                        .ghost()
                        .small()
                        .icon(IconName::Search)
                        .selected(self.searching)
                        .tooltip_with_action(
                            t!("response.search").to_string(),
                            &gpui_kit::component::input::Search,
                            Some("Input"),
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_search(window, cx))),
                )
            })
            .when(filterable, |this| {
                this.child(
                    Button::new("response-filter")
                        .ghost()
                        .small()
                        .icon(gpui_kit::assets::IconName::Funnel)
                        .selected(self.filtering)
                        .tooltip(t!("response.filter").to_string())
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_filter(window, cx))),
                )
            })
            .child({
                let button = Button::new("response-curl")
                    .ghost()
                    .small()
                    .icon(gpui_kit::assets::IconName::Terminal);
                // A gRPC call is a grpcurl command; any other request is
                // offered in every client's code (#69).
                if self.api.read(cx).request().method == HttpMethod::Grpc {
                    button
                        .tooltip(t!("response.copy_grpcurl").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.api.update(cx, |api, cx| api.copy_code(CodeTarget::Curl, cx))
                        }))
                        .into_any_element()
                } else {
                    let api = self.api.downgrade();
                    button
                        .tooltip(t!("response.copy_as").to_string())
                        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| code_menu(menu, &api))
                        .into_any_element()
                }
            })
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

    /// The tab shown: Body when the one picked is not one this response has.
    fn tab_for(&self, response: &HttpResponse) -> ResponseTab {
        if tabs_of(response).contains(&self.tab) {
            self.tab
        } else {
            ResponseTab::Body
        }
    }

    fn render_tabs(&self, response: &HttpResponse, cx: &mut Context<Self>) -> AnyElement {
        let grpc = response.request.method == HttpMethod::Grpc;
        let label = |tab: &ResponseTab| match tab {
            ResponseTab::Body => t!("response.body").to_string(),
            ResponseTab::Headers => {
                let name = if grpc {
                    t!("editor.grpc_metadata")
                } else {
                    t!("response.headers")
                };
                format!("{} ({})", name, response.display_headers().len())
            }
            ResponseTab::Cookies => format!("{} ({})", t!("response.cookies"), response.cookies().len()),
            ResponseTab::Info => t!("response.info").to_string(),
            ResponseTab::Wire => t!("response.wire").to_string(),
        };
        let tabs = tabs_of(response);
        let shown = self.tab_for(response);
        let copies = shown == ResponseTab::Body
            && response.error.is_none()
            && response.body_omitted.is_none()
            && !response.body.is_empty()
            && matches!(response.body_kind(), BodyKind::Json | BodyKind::Text);
        // F7.4: a picture, or its bytes; and the file, opened or kept.
        let pictured = shown == ResponseTab::Body
            && response.error.is_none()
            && response.body_omitted.is_none()
            && self.image.is_some();
        TabBar::new("response-tabs")
            .underline()
            .small()
            .px_2()
            .selected_index(tabs.iter().position(|tab| *tab == shown).unwrap_or(0))
            .children(tabs.iter().map(|tab| Tab::new().label(label(tab))))
            .when(copies, |bar| {
                let page = this_build::opens_html() && response.content_type().to_ascii_lowercase().contains("html");
                let stream = shows_events(response);
                // A stream's own switch stands in for the JSON one.
                let json = response.body_kind() == BodyKind::Json && !stream;
                let raw = self.stream_raw.as_ref() == Some(&response.id);
                bar.suffix(
                    h_flex()
                        .gap_0p5()
                        .items_center()
                        // An event stream as events, or as it came.
                        .when(stream, |this| {
                            this.child(
                                [t!("response.events"), t!("editor.json_view_raw")]
                                    .into_iter()
                                    .enumerate()
                                    .fold(
                                        ButtonGroup::new("response-stream-view").compact().small().outline(),
                                        |group, (index, label)| {
                                            group.child(
                                                Button::new(("response-stream-view", index))
                                                    .label(label.to_string())
                                                    .selected((index == 1) == raw),
                                            )
                                        },
                                    )
                                    .on_click(cx.listener(|this, picks: &Vec<usize>, _, cx| {
                                        this.stream_raw = match picks.first() {
                                            Some(1) => this.shown.clone(),
                                            _ => None,
                                        };
                                        this.apply_search(false, cx);
                                        cx.notify();
                                    })),
                            )
                        })
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
                                    // What the filter found, when it narrows the body (#63).
                                    if let Some(matches) = this.matches(cx).filter(|_| this.wanted_filter(cx).is_some())
                                    {
                                        return copy(cyberapi_core::format::json(&*matches), window, cx);
                                    }
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
                                this.apply_search(false, cx);
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
            .when(shown == ResponseTab::Wire, |bar| {
                // #58's switch: the sent headers that read like secrets.
                let secrets = response
                    .sent
                    .as_ref()
                    .is_some_and(|sent| sent.headers.iter().any(SentHeader::secret));
                bar.suffix(
                    h_flex()
                        .gap_0p5()
                        .items_center()
                        .when(secrets, |this| {
                            this.child(self.reveal_button("response-wire-reveal", cx))
                        })
                        .child(
                            Button::new("response-copy-wire")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Copy)
                                .tooltip(t!("response.copy_wire").to_string())
                                // Whole, and masked as it is shown.
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let reveal = this.reveal;
                                    let text = this
                                        .api
                                        .read(cx)
                                        .shown_response()
                                        .map(|response| response.exchange(reveal));
                                    if let Some(text) = text {
                                        copy(text, window, cx);
                                    }
                                })),
                        ),
                )
            })
            .on_click(cx.listener(move |this, index: &usize, _, cx| {
                if let Some(tab) = tabs.get(*index) {
                    this.tab = *tab;
                    this.apply_search(false, cx);
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
            // As this request went: its own setting, or the settings' (#71).
            let settings = AppStore::global(cx).state().read(cx).prefs().sending.follow_redirects;
            let follows = response.request.settings.follow_redirects.unwrap_or(settings);
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
            let mut buttons = vec![copy_to];
            if !follows {
                let (workspace, api) = (self.workspace.downgrade(), self.api.clone());
                buttons.push(
                    Button::new("response-open-settings")
                        .ghost()
                        .xsmall()
                        .label(t!("response.open_api_settings").to_string())
                        .on_click(move |_, _, cx| {
                            let _ = workspace.update(cx, |workspace, cx| {
                                workspace.set_tab(&api, cyberapi_core::prefs::RequestTab::Settings, cx)
                            });
                        }),
                );
            }
            note(text.to_string(), buttons, cx)
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
            .then(|| self.api.read(cx).graphql_errors(response))
            .filter(|errors| !errors.messages.is_empty())
            .map(|errors| {
                let shown: Vec<&str> = errors.messages.iter().take(3).map(String::as_str).collect();
                let text = crate::i18n::counted("response.graphql_errors", errors.messages.len());
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
        // A gRPC call that failed says why in its status; a reply its
        // definitions do not read is shown as the bytes it came as.
        let grpc = response.grpc_status().filter(|status| !status.is_ok()).map(|status| {
            let danger = cx.theme().danger;
            let title = t!(
                "response.grpc_failed",
                status = format!("{} {}", status.code, status.name())
            )
            .to_string();
            v_flex()
                .px_3()
                .py_2()
                .gap_0p5()
                .text_sm()
                .text_color(danger)
                .child(title)
                .when(!status.message.is_empty(), |this| {
                    this.child(div().text_xs().child(status.message.clone()))
                })
                .into_any_element()
        });
        let unread = response
            .notice
            .as_ref()
            .map(|why| note(t!("response.grpc_unread", reason = why).to_string(), Vec::new(), cx));
        // How an event stream ended, when its server did not end it (one cut
        // at the limit is said by `cut`).
        let stream_end = response.stream.as_ref().and_then(|stream| match &stream.ended {
            StreamEnd::Stopped => Some(t!("response.stream_stopped").to_string()),
            StreamEnd::Quiet { seconds } => Some(t!("response.stream_quiet", seconds = seconds).to_string()),
            StreamEnd::Failed { message } => Some(t!("response.stream_failed", reason = message).to_string()),
            StreamEnd::Closed | StreamEnd::Cut => None,
        });
        let stream_end = stream_end.map(|text| note(text, Vec::new(), cx));
        let notes: Vec<AnyElement> = [unkept, graphql, grpc, unread, stream_end, redirect, cut]
            .into_iter()
            .flatten()
            .collect();
        if notes.is_empty() {
            return body;
        }
        v_flex()
            .size_full()
            .children(notes)
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
                    // C5: the desktop can trust the server's authority.
                    .child(
                        match crate::this_build::reads_files() {
                            true => t!("response.untrusted_certificate_desktop"),
                            false => t!("response.untrusted_certificate"),
                        }
                        .to_string(),
                    )
            });
            // The bridge sends to public addresses only. The alert is the
            // English reason; this says the same in the reader's language.
            let public_only = SendError::is_forbidden_target(error).then(|| {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("response.public_only").to_string())
            });
            return v_flex()
                .p_3()
                .gap_2()
                .child(Alert::error("response-error", error.clone()).title(t!("response.failed").to_string()))
                .children(certificate)
                .children(public_only)
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
        if shows_events(response) && self.stream_raw.as_ref() != Some(&response.id) {
            return self.events.clone().into_any_element();
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
                // #63: narrowed in Pretty and Tree.
                let filters =
                    self.filtering && response.body_kind() == BodyKind::Json && self.response_view(cx) != JsonView::Raw;
                let searches = self.searching && !tree;
                v_flex()
                    .size_full()
                    .when(filters, |this| this.child(self.render_filter(cx)))
                    .when(searches, |this| this.child(self.render_search(&self.body, cx)))
                    // The tree has the whole body.
                    .when_some(self.clipped.filter(|_| !tree), |this, shown| {
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
    /// #63: the filter's field, and what it found.
    fn render_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, danger, warning) = (theme.muted_foreground, theme.danger, theme.warning);
        let status = match self.found_now(cx).map(|found| &found.outcome) {
            // #74: said so the whole body is not taken for what it found.
            Some(Some(Ok(matches))) if no_match(matches) => Some((t!("response.filter_no_match").to_string(), warning)),
            Some(Some(Ok(matches))) => Some((
                crate::i18n::counted("response.filter_matches", matches.as_array().map_or(0, Vec::len)),
                muted,
            )),
            Some(Some(Err(NotFiltered::Path(why)))) => {
                Some((t!("response.filter_invalid", why = why).to_string(), danger))
            }
            Some(Some(Err(NotFiltered::NotJson))) => Some((t!("response.filter_not_json").to_string(), danger)),
            Some(Some(Err(NotFiltered::TooLarge(size)))) => Some((
                t!("response.filter_too_large", size = format_size(*size as u64)).to_string(),
                danger,
            )),
            _ => None,
        };
        v_flex()
            .px_3()
            .py_1p5()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Input::new(&self.filter)
                    .small()
                    .cleanable(true)
                    .prefix(Icon::new(gpui_kit::assets::IconName::Funnel).small().text_color(muted)),
            )
            .when_some(status, |this, (text, color)| {
                this.child(div().text_xs().text_color(color).child(text))
            })
            .into_any_element()
    }

    /// The filter's field opens, to type in; closed, its filter goes.
    fn toggle_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filtering = !self.filtering;
        if self.filtering {
            self.filter.update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.filter.update(cx, |input, cx| input.set_value("", window, cx));
            let api = self.api.read(cx).api().clone();
            self.workspace
                .update(cx, |workspace, cx| workspace.set_response_filter(&api, "", cx));
        }
        self.sync(window, cx);
        cx.notify();
    }

    /// The editor currently shown that [`Self::search`] can search, when
    /// there is one.
    fn searchable_editor(&self, cx: &App) -> Option<Entity<EditorState>> {
        let api = self.api.read(cx);
        if api.sending_since().is_some() || api.live().is_some() {
            return None;
        }
        let response = api.shown_response()?;
        match self.tab_for(response) {
            ResponseTab::Wire => Some(self.wire.clone()),
            ResponseTab::Body => {
                if response.error.is_some() || response.body_omitted.is_some() || response.body.is_empty() {
                    return None;
                }
                if shows_events(response) && self.stream_raw.as_ref() != Some(&response.id) {
                    return None;
                }
                match response.body_kind() {
                    BodyKind::Json => (!self.response_shows_tree(cx)).then(|| self.body.clone()),
                    BodyKind::Text | BodyKind::Pdf | BodyKind::Binary => Some(self.body.clone()),
                    BodyKind::Image => (self.image.is_none() || self.image_raw).then(|| self.body.clone()),
                }
            }
            ResponseTab::Headers | ResponseTab::Cookies | ResponseTab::Info => None,
        }
    }

    /// #63: opens the find bar for the shown editor and focuses its field,
    /// seeding it from the editor's selection when there is one; leaves the
    /// shortcut to the workspace when no editor is shown.
    pub(super) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.searchable_editor(cx) else {
            cx.propagate();
            return;
        };
        cx.stop_propagation();
        self.searching = true;
        let selected = editor.read(cx).selected_text().to_string();
        if !selected.is_empty() {
            self.search
                .update(cx, |input, cx| input.set_value(selected, window, cx));
        }
        self.apply_search(true, cx);
        self.search.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    fn toggle_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.searching {
            self.close_search(window, cx);
        } else {
            self.open_search(window, cx);
        }
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.searching = false;
        let focus = self.searchable_editor(cx);
        self.apply_search(false, cx);
        if let Some(editor) = focus {
            editor.update(cx, |editor, cx| editor.focus(window, cx));
        }
        cx.notify();
    }

    fn apply_search(&self, scroll: bool, cx: &mut Context<Self>) {
        let active = self.searchable_editor(cx);
        let query = if self.searching {
            self.search.read(cx).value().to_string()
        } else {
            String::new()
        };
        let case_insensitive = self.search_case_insensitive;
        for editor in [&self.body, &self.wire] {
            let is_active = active
                .as_ref()
                .is_some_and(|shown| shown.entity_id() == editor.entity_id());
            editor.update(cx, |editor, cx| {
                if !is_active || query.is_empty() {
                    editor.close_search(cx);
                } else {
                    editor.set_search_query(query.clone(), case_insensitive, cx);
                    if scroll && !editor.search_session().matcher.is_empty() {
                        let _ = editor.previous_search_match(cx);
                        let _ = editor.next_search_match(cx);
                    }
                }
            });
        }
    }

    fn step_search(&self, forward: bool, cx: &mut Context<Self>) {
        let Some(editor) = self.searchable_editor(cx) else {
            return;
        };
        editor.update(cx, |editor, cx| {
            if forward {
                let _ = editor.next_search_match(cx);
            } else {
                let _ = editor.previous_search_match(cx);
            }
        });
    }

    /// #63: the find bar above `editor`, with match count and navigation.
    fn render_search(&self, editor: &Entity<EditorState>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, warning) = (theme.muted_foreground, theme.warning);
        let query_non_empty = !self.search.read(cx).value().is_empty();
        let (has_matches, count_label) = {
            let matcher = &editor.read(cx).search_session().matcher;
            (!matcher.is_empty(), matcher.label())
        };
        h_flex()
            .id("response-search-bar")
            .px_3()
            .py_1p5()
            .gap_1p5()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.search)
                        .small()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).small().text_color(muted))
                        .suffix(
                            Button::new("response-search-case")
                                .xsmall()
                                .compact()
                                .text()
                                .icon(IconName::CaseSensitive)
                                .selected(!self.search_case_insensitive)
                                .tooltip(t!("response.search_match_case").to_string())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.search_case_insensitive = !this.search_case_insensitive;
                                    this.apply_search(true, cx);
                                    cx.notify();
                                })),
                        ),
                ),
            )
            .when(query_non_empty, |this| {
                this.child(
                    div()
                        .px_1()
                        .text_xs()
                        .text_color(if has_matches { muted } else { warning })
                        .child(count_label),
                )
            })
            .child(
                Button::new("response-search-prev")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronUp)
                    .tooltip(t!("response.search_prev").to_string())
                    .disabled(!has_matches)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.step_search(false, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("response-search-next")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronDown)
                    .tooltip(t!("response.search_next").to_string())
                    .disabled(!has_matches)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.step_search(true, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("response-search-close")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(t!("common.close").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.close_search(window, cx))),
            )
            .into_any_element()
    }

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
            .when(self.searching, |this| this.child(self.render_search(&self.body, cx)))
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
                .when(self.searching, |this| this.child(self.render_search(&self.body, cx)))
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
        // #60: the peer above is the proxy's.
        if !stats.proxy.is_empty() {
            rows.push((t!("response.proxy").to_string(), stats.proxy.clone()));
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
        let sent = response.sent.as_ref().map(|sent| self.render_sent(sent, cx));
        v_flex()
            .id("response-info")
            .size_full()
            .overflow_y_scrollbar()
            .p_2()
            .gap_3()
            .child(field_rows("response-info-fields", rows, cx))
            .when(timed, |this| this.child(timeline))
            .children(sent)
            .into_any_element()
    }

    /// #58: shows or masks the sent headers that read like secrets, in
    /// Details and on the wire alike.
    fn reveal_button(&self, id: &'static str, cx: &mut Context<Self>) -> Button {
        let (icon, label) = if self.reveal {
            (IconName::EyeOff, t!("response.hide_secrets"))
        } else {
            (IconName::Eye, t!("response.show_secrets"))
        };
        Button::new(id)
            .ghost()
            .xsmall()
            .icon(icon)
            .label(label.to_string())
            .on_click(cx.listener(|this, _, _, cx| {
                this.reveal = !this.reveal;
                cx.notify();
            }))
    }

    /// #51: the wire editor holds `response`'s exchange, its secrets as
    /// the switch says; set once for each.
    fn sync_wire(&mut self, response: &HttpResponse, window: &mut Window, cx: &mut Context<Self>) {
        let key = (response.id.clone(), self.reveal, self.display_limit);
        if self.wire_of.as_ref() == Some(&key) {
            return;
        }
        self.wire_of = Some(key);
        // As the body: its start, and no wrapping a line too long to.
        let (text, clipped) = response.exchange_within(self.reveal, self.display_limit);
        self.wire_clipped = clipped;
        let wraps = self.wrap && unwrappable_line(&text).is_none();
        self.wire.update(cx, |editor, cx| {
            editor.set_soft_wrap(wraps, window, cx);
            editor.set_value(text, window, cx);
        });
        self.apply_search(false, cx);
    }

    /// #51: the request as it went out and the answer as it came, as
    /// `curl -v` prints them.
    fn render_wire(&self, response: &HttpResponse, cx: &mut Context<Self>) -> AnyElement {
        // Kept before the sent request was (#58): the answer alone.
        let unsent = response
            .sent
            .is_none()
            .then(|| note(t!("response.wire_unsent").to_string(), Vec::new(), cx));
        let clipped = self.wire_clipped.then(|| {
            let size = format_size(self.display_limit as u64);
            note(t!("response.wire_clipped", size = size).to_string(), Vec::new(), cx)
        });
        v_flex()
            .size_full()
            .when(self.searching, |this| this.child(self.render_search(&self.wire, cx)))
            .children(unsent)
            .children(clipped)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(Editor::new(&self.wire).w_full().h_full().readonly(true).bordered(false)),
            )
            .into_any_element()
    }

    /// #58: the request as it went out — every header, said where it came
    /// from, and the body — what reads like a secret masked till shown.
    fn render_sent(&self, sent: &SentRequest, cx: &mut Context<Self>) -> AnyElement {
        let reveal = self.reveal;
        let secret = SentHeader::secret;
        let secrets = sent.headers.iter().any(secret);
        let rows: Vec<(String, String, String)> = sent
            .headers
            .iter()
            .map(|header| {
                let value = if !reveal && secret(header) {
                    "••••••".to_string()
                } else {
                    header.value.clone()
                };
                (header.name.clone(), value, header_source(header.from))
            })
            .collect();
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let shown = sent.body.as_ref().map_or(0, String::len) as u64;
        let body_note = match &sent.body {
            Some(_) if sent.body_size > shown => Some(t!(
                "response.sent_body_cut",
                size = format_size(shown),
                total = format_size(sent.body_size)
            )),
            None if sent.body_size > 0 => Some(t!("response.sent_binary", size = format_size(sent.body_size))),
            _ => None,
        };
        let body = sent.body.clone().filter(|body| !body.is_empty()).map(|body| {
            div()
                .px_2()
                .py_1p5()
                .rounded(theme.radius)
                .bg(theme.muted)
                .text_xs()
                .font_family(theme.mono_font_family.clone())
                .child(body)
        });
        v_flex()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .px_2()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(t!("response.sent_title").to_string()),
                    )
                    .when(secrets, |this| {
                        this.child(self.reveal_button("response-sent-reveal", cx))
                    }),
            )
            .child(sent_rows(rows, cx))
            .when(body.is_some() || body_note.is_some(), |this| {
                this.child(
                    v_flex()
                        .px_2()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child(t!("response.sent_body").to_string()),
                        )
                        .when_some(body_note, |this, note| {
                            this.child(div().text_xs().text_color(muted).child(note.to_string()))
                        })
                        .children(body),
                )
            })
            .into_any_element()
    }
}

/// F3: asks for a name and keeps the response shown as an example; says
/// why when it cannot be one.
fn keep_example(api: Entity<ApiState>, window: &mut Window, cx: &mut App) {
    let suggested = api
        .read(cx)
        .shown_response()
        .and_then(|shown| shown.status)
        .map(|status| format!("{status} {}", status_text(status)))
        .unwrap_or_default();
    crate::views::name_dialog::open_name_dialog_with(
        window,
        cx,
        t!("docs.keep_example_title").to_string().into(),
        t!("docs.example_name").to_string().into(),
        suggested,
        t!("common.save").to_string().into(),
        move |name, window, cx| {
            let kept = api.update(cx, |api, cx| api.keep_example(&name, cx));
            let note = if kept {
                Notification::success(t!("docs.example_kept").to_string())
            } else {
                Notification::warning(t!("docs.example_refused").to_string())
            };
            window.push_notification(note, cx);
        },
    );
}

/// #69: an item for each client a request is copied as, cURL first; B6:
/// in their groups, a line between.
fn code_menu(menu: PopupMenu, api: &gpui_kit::WeakEntity<ApiState>) -> PopupMenu {
    CodeTarget::GROUPS.iter().enumerate().fold(menu, |menu, (at, group)| {
        let menu = if at == 0 { menu } else { menu.separator() };
        group.iter().fold(menu, |menu, &target| {
            let api = api.clone();
            menu.item(PopupMenuItem::new(target.label()).on_click(move |_, _, cx| {
                let _ = api.update(cx, |api, cx| api.copy_code(target, cx));
            }))
        })
    })
}

/// Where a sent header came from, as its row says it.
fn header_source(from: HeaderSource) -> String {
    match from {
        HeaderSource::Request => t!("response.from_request"),
        HeaderSource::Global => t!("response.from_global"),
        HeaderSource::Auth => t!("response.from_auth"),
        HeaderSource::Folder => t!("response.from_folder"),
        HeaderSource::Cookie => t!("response.from_cookie"),
        HeaderSource::Body => t!("response.from_body"),
        HeaderSource::Default => t!("response.from_default"),
    }
    .to_string()
}

/// A sent request's headers: name, value and where it came from, the
/// value — as shown — copied by the button its row shows on hover.
fn sent_rows(rows: Vec<(String, String, String)>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (muted, hover) = (theme.muted_foreground, theme.list_hover);
    v_flex()
        .text_sm()
        .children(rows.into_iter().enumerate().map(move |(index, (name, value, from))| {
            let group = SharedString::from(format!("response-sent-{index}"));
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
                .child(div().flex_none().text_xs().text_color(muted).child(from))
                .child(
                    div()
                        .flex_none()
                        .invisible()
                        .group_hover(group, |style| style.visible())
                        .child(
                            Button::new(("response-sent-copy", index))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Copy)
                                .tooltip(t!("common.copy").to_string())
                                .on_click(move |_, window, cx| copy(copied.clone(), window, cx)),
                        ),
                )
        }))
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
/// `limit` is how much text to show. Hex keeps its own `HEX_LIMIT`.
fn editor_text(response: &HttpResponse, pretty: bool, limit: usize) -> (&'static str, String, Option<usize>, bool) {
    // A body that would make the editor crawl shows its start (#17), and
    // only that start is read (PERF-3).
    let (text, clipped) = match response.body_kind() {
        BodyKind::Json | BodyKind::Text => response.shown_body_text(pretty, limit).unwrap_or_default(),
        BodyKind::Image if response.image_type() == Some(ImageType::Svg) => {
            cyberapi_core::format::text_prefix(&response.body, limit)
        }
        BodyKind::Image | BodyKind::Pdf | BodyKind::Binary => {
            let shown = &response.body[..response.body.len().min(HEX_LIMIT)];
            let clipped = (shown.len() < response.body.len()).then_some(HEX_LIMIT);
            return ("text", media::hex_dump(shown), clipped, true);
        }
    };
    (language_of(response), text, clipped.then_some(limit), false)
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
fn status_color(status: u16, cx: &App) -> Hsla {
    let theme = cx.theme();
    match status_level(status) {
        StatusLevel::Success => theme.success,
        StatusLevel::Redirect => theme.info,
        StatusLevel::ClientError => theme.warning,
        StatusLevel::ServerError => theme.danger,
    }
}

/// A gRPC call's status in its colour: OK green, one the caller can mend
/// (an argument, a permission, what is not found) yellow, the server's red.
fn grpc_color(status: &GrpcStatus, cx: &App) -> Hsla {
    let theme = cx.theme();
    match status.code {
        0 => theme.success,
        1 | 3 | 5 | 6 | 7 | 9 | 11 | 16 => theme.warning,
        _ => theme.danger,
    }
}

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
/// A status in its colour.
fn pill(text: String, color: Hsla) -> AnyElement {
    h_flex()
        .h(px(22.))
        .px_2()
        .items_center()
        .rounded_full()
        .bg(color.opacity(0.16))
        .font_semibold()
        .text_color(color)
        .child(text)
        .into_any_element()
}

/// Whether a response's body shows as events: a stream's, whole.
fn shows_events(response: &HttpResponse) -> bool {
    response.error.is_none() && response.body_omitted.is_none() && response.notice.is_none() && response.is_stream()
}

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
        if let Some(response) = &response
            && self.tab_for(response) == ResponseTab::Wire
        {
            self.sync_wire(response, window, cx);
        }
        let summary = self.render_summary(response.as_deref(), cx);
        let live = self.api.read(cx).live().is_some();
        let content: AnyElement = match &response {
            // An event stream as it comes.
            _ if live => self.events.clone().into_any_element(),
            Some(response) => {
                let content = match self.tab_for(response) {
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
                    ResponseTab::Wire => self.render_wire(response, cx),
                };
                v_flex()
                    .flex_1()
                    .min_h_0()
                    // The last answer, while the next is on its way, is no
                    // longer this request's (#43): faded until it comes.
                    .when(sending, |this| this.opacity(0.45))
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
            .track_focus(&self.focus)
            .border_l_1()
            .border_color(cx.theme().border)
            .on_action(cx.listener(|this, _: &gpui_kit::component::input::Search, window, cx| {
                this.open_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &super::FindApi, window, cx| {
                this.open_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &gpui_kit::component::input::Escape, window, cx| {
                if this.searching {
                    this.close_search(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .child(summary)
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::editor_text;
    use cyberapi_core::response::HttpResponse;

    /// Text stops at the settings' limit. Bytes keep the hex limit.
    #[test]
    fn a_text_body_is_clipped_at_the_settings_limit_and_bytes_are_not() {
        let text = HttpResponse {
            headers: vec![("content-type".into(), "text/plain".into())],
            body: b"abcdef".to_vec(),
            ..Default::default()
        };
        let (_, shown, clipped, hex) = editor_text(&text, false, 4);
        assert!(!hex);
        assert_eq!(shown, "abcd");
        assert_eq!(clipped, Some(4));
        let (_, shown, clipped, _) = editor_text(&text, false, 6);
        assert_eq!(shown, "abcdef");
        assert_eq!(clipped, None);

        let bytes = HttpResponse {
            headers: vec![("content-type".into(), "application/octet-stream".into())],
            body: vec![0, 1, 2, 3, 255],
            ..Default::default()
        };
        let (_, _, clipped, hex) = editor_text(&bytes, false, 1);
        assert!(hex);
        assert_eq!(clipped, None, "hex keeps its own limit");
    }
}
