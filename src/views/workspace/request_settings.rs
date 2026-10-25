//! An API's own sending settings (#71): its HTTP version, timeouts, whether
//! it follows redirects and checks certificates — each following the
//! settings until set here, and saying what the settings say. A gRPC call
//! has only the timeouts and the certificates: it speaks HTTP/2 and is not
//! redirected.

use crate::states::{ApiState, AppStore};
use crate::views::settings::{Choice, setting_row};
use cyberapi_core::model::HttpMethod;
use cyberapi_core::prefs::{HttpVersion, RequestSettings, SendPrefs, Timeouts};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme as _, IndexPath, Sizable as _, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, div,
};
use rust_i18n::t;

type Choices = Entity<SelectState<Vec<Choice>>>;

/// The HTTP versions a list offers after "the settings'".
const VERSIONS: [HttpVersion; 3] = [HttpVersion::Auto, HttpVersion::Http1, HttpVersion::Http2];

pub struct RequestSettingsView {
    api: Entity<ApiState>,
    http_version: Choices,
    follow_redirects: Choices,
    verify_certs: Choices,
    connect: Entity<InputState>,
    write: Entity<InputState>,
    read: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

fn version_label(version: HttpVersion) -> String {
    match version {
        HttpVersion::Auto => t!("settings.http_auto"),
        HttpVersion::Http1 => t!("settings.http_1"),
        HttpVersion::Http2 => t!("settings.http_2"),
    }
    .to_string()
}

fn on_off(on: bool) -> String {
    if on {
        t!("editor.setting_on")
    } else {
        t!("editor.setting_off")
    }
    .to_string()
}

/// "The settings' (what they say)", the first of each list.
fn theirs(what: String) -> String {
    t!("editor.setting_theirs", what = what).to_string()
}

impl RequestSettingsView {
    pub fn new(api: Entity<ApiState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let own = api.read(cx).request().settings;
        let prefs = AppStore::global(cx).state().read(cx).prefs().clone();
        let (sending, timeouts): (SendPrefs, Timeouts) = (prefs.sending, prefs.timeouts.effective());
        let mut subscriptions = Vec::new();

        // Each list: the settings' first, then the choices; `set` writes the
        // one picked (`None`: the settings') into the API's settings.
        let mut list = |labels: Vec<String>,
                        chosen: usize,
                        set: fn(&mut RequestSettings, usize),
                        window: &mut Window,
                        cx: &mut Context<Self>| {
            let items = labels
                .into_iter()
                .enumerate()
                .map(|(index, label)| Choice {
                    label: label.into(),
                    index,
                })
                .collect();
            let state = cx.new(|cx| SelectState::new(items, Some(IndexPath::new(chosen)), window, cx));
            subscriptions.push(cx.subscribe(
                &state,
                move |this: &mut Self, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(index)) = event {
                        this.change(cx, |settings| set(settings, *index));
                    }
                },
            ));
            state
        };
        let http_version = list(
            std::iter::once(theirs(version_label(sending.http_version)))
                .chain(VERSIONS.iter().map(|version| version_label(*version)))
                .collect(),
            own.http_version
                .and_then(|version| VERSIONS.iter().position(|each| *each == version))
                .map_or(0, |index| index + 1),
            |settings, index| settings.http_version = index.checked_sub(1).map(|index| VERSIONS[index]),
            window,
            cx,
        );
        let pick = |own: Option<bool>| match own {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        };
        let follow_redirects = list(
            vec![theirs(on_off(sending.follow_redirects)), on_off(true), on_off(false)],
            pick(own.follow_redirects),
            |settings, index| {
                settings.follow_redirects = match index {
                    1 => Some(true),
                    2 => Some(false),
                    _ => None,
                }
            },
            window,
            cx,
        );
        // Shown as "check", kept as "accept any".
        let verify_certs = list(
            vec![
                theirs(on_off(!sending.accept_invalid_certs)),
                on_off(true),
                on_off(false),
            ],
            pick(own.accept_invalid_certs.map(|accept| !accept)),
            |settings, index| {
                settings.accept_invalid_certs = match index {
                    1 => Some(false),
                    2 => Some(true),
                    _ => None,
                }
            },
            window,
            cx,
        );

        // A timeout: empty follows the settings, whose value is its
        // placeholder; a whole number of seconds is the API's own.
        let mut seconds =
            |value: u32, settings: u32, set: fn(&mut Timeouts, u32), window: &mut Window, cx: &mut Context<Self>| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(theirs(settings.to_string()))
                        .default_value(if value > 0 { value.to_string() } else { String::new() })
                });
                subscriptions.push(
                    cx.subscribe(&input, move |this: &mut Self, input, event: &InputEvent, cx| {
                        if let InputEvent::Change = event {
                            let text = input.read(cx).value().trim().to_string();
                            let value = match text.parse::<u32>() {
                                Ok(value) => value,
                                Err(_) if text.is_empty() => 0,
                                // Not a number yet: nothing changes.
                                Err(_) => return,
                            };
                            this.change(cx, |settings| set(&mut settings.timeouts, value));
                        }
                    }),
                );
                input
            };
        let connect = seconds(own.timeouts.connect, timeouts.connect, |t, v| t.connect = v, window, cx);
        let write = seconds(own.timeouts.write, timeouts.write, |t, v| t.write = v, window, cx);
        let read = seconds(own.timeouts.read, timeouts.read, |t, v| t.read = v, window, cx);
        Self {
            api,
            http_version,
            follow_redirects,
            verify_certs,
            connect,
            write,
            read,
            _subscriptions: subscriptions,
        }
    }

    fn change(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut RequestSettings)) {
        let mut settings = self.api.read(cx).request().settings;
        change(&mut settings);
        self.api.update(cx, |api, cx| api.set_settings(settings, cx));
    }
}

impl Render for RequestSettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let grpc = self.api.read(cx).request().method == HttpMethod::Grpc;
        let mut rows: Vec<AnyElement> = Vec::new();
        if !grpc {
            rows.push(setting_row(
                t!("settings.http_version").to_string(),
                None,
                Select::new(&self.http_version).small(),
                cx,
            ));
        }
        for (label, input) in [
            (t!("settings.timeout_connect"), &self.connect),
            (t!("settings.timeout_write"), &self.write),
            (t!("settings.timeout_read"), &self.read),
        ] {
            rows.push(setting_row(label.to_string(), None, Input::new(input).small(), cx));
        }
        if !grpc {
            rows.push(setting_row(
                t!("settings.follow_redirects").to_string(),
                None,
                Select::new(&self.follow_redirects).small(),
                cx,
            ));
        }
        rows.push(setting_row(
            t!("settings.verify_certs").to_string(),
            None,
            Select::new(&self.verify_certs).small(),
            cx,
        ));
        v_flex()
            .id("request-settings")
            .size_full()
            .overflow_y_scrollbar()
            .px_3()
            .py_2()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .pb_1()
                    .child(t!("editor.settings_hint").to_string()),
            )
            .children(rows)
    }
}
