//! The application's settings (M14), laid out as zedis lays out its own:
//! the sections down the left, the chosen one's settings on the right, each
//! a row of what it is and what it does, with its control on the right. A
//! window of its own on the desktop; a dialog over the page in the browser,
//! which has one canvas a tab.

#[cfg(not(target_family = "wasm"))]
mod window;

#[cfg(not(any(target_os = "macos", target_family = "wasm")))]
pub use window::is_settings_window;

use crate::json_view_local::JsonView;
use crate::states::{AppStore, UpdateEvent, UpdatePhase};
use crate::this_build;
use crate::views::fixed_rem::fixed_rem;
use crate::views::inline_confirm::inline_confirm;
use crate::views::json_tree::json_view_label;
use crate::views::server_section::ServerSection;
use crate::views::update_dialog::open_update_dialog;
use cyberapi_core::prefs::{HttpVersion, SendPrefs, TextPrefs, ThemeMode, Timeouts, WindowPrefs};
use cyberapi_core::redirect::MAX_REDIRECTS;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, IndexPath, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, SharedString, Styled as _, Subscription, Task, Window, div, px,
    rems, size,
};
use rust_i18n::t;
use std::ops::RangeInclusive;
use std::time::Duration;

/// The project's home: its releases (and the nightly), its issues.
const HOMEPAGE: &str = env!("CARGO_PKG_REPOSITORY");

/// The languages offered (F15.2), each named in itself; `None` follows the
/// system.
const LANGUAGES: [(Option<&str>, &str); 3] = [(None, ""), (Some("zh-CN"), "中文"), (Some("en"), "English")];

/// F14.1, in the list's order.
const THEMES: [ThemeMode; 3] = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark];

/// F14.10, in the list's order.
const HTTP_VERSIONS: [HttpVersion; 3] = [HttpVersion::Auto, HttpVersion::Http1, HttpVersion::Http2];

/// How long typing in a number field waits before it counts.
const COMMIT_DELAY: Duration = Duration::from_millis(600);

/// Opens the settings in a window of their own, or brings that window
/// forward when it is open. `window` is the main one, which the Window
/// section sizes.
#[cfg(not(target_family = "wasm"))]
pub fn open_settings(window: &mut Window, cx: &mut App) {
    let main = window.window_handle();
    window::open(cx, move |window, cx| {
        cx.new(|cx| SettingsView::new(Some(main), window, cx))
    });
}

/// The page is one canvas, so a dialog over it, with a body tall enough to
/// scroll in: the page's height less the dialog's title and margins, up to
/// what the desktop's window shows.
#[cfg(target_family = "wasm")]
pub fn open_settings(window: &mut Window, cx: &mut App) {
    // One at a time: a second copy would stack on the first.
    if window.has_active_dialog(cx) {
        return;
    }
    let view = cx.new(|cx| SettingsView::new(None, window, cx));
    // Sized by the text as it was when opened: measured afresh, a dialog
    // centred on the page widened on both sides with each step of the text
    // size, and the step button ran from under the pointer.
    let rem = cx.theme().font_size;
    window.open_dialog(cx, move |dialog, window, _| {
        let viewport = window.viewport_size();
        let height = (viewport.height - px(160.)).min(rem * 40.).max(px(280.));
        dialog
            .title(t!("settings.title").to_string())
            // Within a narrow page, at the largest text too.
            .w((rem * 54.).min(viewport.width - px(32.)).max(px(480.)))
            .child(div().w_full().h(height).child(view.clone()))
            // Enter is a field's own: it sets a number; it does not close
            // the settings.
            .on_ok(|_, _, _| false)
    });
}

/// The groups the settings are in, in the order the list shows them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Section {
    #[default]
    Appearance,
    Editor,
    Requests,
    Window,
    Server,
    Data,
    About,
}

impl Section {
    fn title(self) -> String {
        match self {
            Self::Appearance => t!("settings.section_appearance"),
            Self::Editor => t!("settings.section_editor"),
            Self::Requests => t!("settings.section_requests"),
            Self::Window => t!("settings.section_window"),
            Self::Server => t!("server.title"),
            Self::Data => t!("settings.section_data"),
            Self::About => t!("settings.section_about"),
        }
        .to_string()
    }

    fn description(self) -> String {
        match self {
            Self::Appearance => t!("settings.section_appearance_desc"),
            Self::Editor => t!("settings.section_editor_desc"),
            Self::Requests => t!("settings.section_requests_desc"),
            Self::Window => t!("settings.section_window_desc"),
            Self::Server => t!("server.hint"),
            Self::Data => t!("settings.section_data_desc"),
            Self::About => t!("settings.section_about_desc"),
        }
        .to_string()
    }

    fn id(self) -> &'static str {
        match self {
            Self::Appearance => "settings-appearance",
            Self::Editor => "settings-editor",
            Self::Requests => "settings-requests",
            Self::Window => "settings-window",
            Self::Server => "settings-server",
            Self::Data => "settings-data",
            Self::About => "settings-about",
        }
    }
}

/// What the Data section clears or resets, once confirmed (F15.3).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Clearing {
    History,
    Workspaces,
    Settings,
}

/// An entry of a list: its label, and its place in the list.
#[derive(Clone)]
struct Choice {
    label: SharedString,
    index: usize,
}

impl SelectItem for Choice {
    type Value = usize;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &usize {
        &self.index
    }
}

type Choices = Entity<SelectState<Vec<Choice>>>;

/// Which number a field holds, and so what leaving it saves.
#[derive(Clone, Copy)]
enum Number {
    Timeout,
    WindowSize,
}

pub struct SettingsView {
    section: Section,
    /// Where the focus goes when the section changes: the field that held
    /// it leaves with its section.
    focus_handle: FocusHandle,
    /// The language the lists and fields were built in. Their labels are
    /// strings made in `new`, so a language changed here builds the view
    /// again (see `render`).
    built_in: String,
    /// The main window, which the Window section sizes. The desktop's only.
    main: Option<AnyWindowHandle>,
    /// The rem the view opened with, which it keeps: changed here, the text
    /// size would move the rows, and its own buttons from under the pointer.
    rem: Pixels,
    theme: Choices,
    language: Choices,
    json_view: Choices,
    http_version: Choices,
    ui_size: Entity<InputState>,
    editor_size: Entity<InputState>,
    connect: Entity<InputState>,
    write: Entity<InputState>,
    read: Entity<InputState>,
    width: Entity<InputState>,
    height: Entity<InputState>,
    /// A number saved once typing pauses.
    pending: Option<Task<()>>,
    /// What the Data section is asking to confirm.
    confirming: Option<Clearing>,
    /// The desktop's bridge (`docs/server-sync.md` step 5).
    server: Option<Entity<ServerSection>>,
    /// What the last update check asked for here answered, when it found
    /// nothing to show.
    update_note: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    fn new(main: Option<AnyWindowHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let app = AppStore::global(cx).state().clone();
        let prefs = app.read(cx).prefs().clone();
        let mut subscriptions = vec![cx.observe(&app, |_, _, cx| cx.notify())];

        let theme = Self::choices(
            vec![
                t!("settings.theme_system").to_string(),
                t!("settings.theme_light").to_string(),
                t!("settings.theme_dark").to_string(),
            ],
            THEMES.iter().position(|theme| *theme == prefs.theme).unwrap_or(0),
            |index, cx| {
                let app = AppStore::global(cx).state().clone();
                app.update(cx, |app, cx| app.set_theme(THEMES[index], cx));
            },
            window,
            cx,
            &mut subscriptions,
        );
        let language = Self::choices(
            LANGUAGES
                .iter()
                .map(|(code, name)| match code {
                    None => t!("settings.language_system").to_string(),
                    Some(_) => name.to_string(),
                })
                .collect(),
            LANGUAGES
                .iter()
                .position(|(code, _)| *code == prefs.language.as_deref())
                .unwrap_or(0),
            |index, cx| {
                let app = AppStore::global(cx).state().clone();
                let language = LANGUAGES[index].0.map(str::to_string);
                app.update(cx, |app, cx| app.set_language(language, cx));
            },
            window,
            cx,
            &mut subscriptions,
        );
        let shown = app.read(cx).json_view();
        let json_view = Self::choices(
            JsonView::ALL.into_iter().map(json_view_label).collect(),
            JsonView::ALL.iter().position(|view| *view == shown).unwrap_or(0),
            |index, cx| {
                let app = AppStore::global(cx).state().clone();
                app.update(cx, |app, cx| app.set_json_view(JsonView::ALL[index], cx));
            },
            window,
            cx,
            &mut subscriptions,
        );

        let http_version = Self::choices(
            vec![
                t!("settings.http_auto").to_string(),
                t!("settings.http_1").to_string(),
                t!("settings.http_2").to_string(),
            ],
            HTTP_VERSIONS
                .iter()
                .position(|version| *version == prefs.sending.http_version)
                .unwrap_or(0),
            |index, cx| {
                let app = AppStore::global(cx).state().clone();
                let sending = SendPrefs {
                    http_version: HTTP_VERSIONS[index],
                    ..app.read(cx).prefs().sending
                };
                app.update(cx, |app, cx| app.set_sending(sending, cx));
            },
            window,
            cx,
            &mut subscriptions,
        );

        // F14.6.
        let ui_size = Self::text_size(
            prefs.text.ui(),
            TextPrefs::UI_SIZES,
            TextPrefs::DEFAULT_UI,
            |text| text.ui(),
            |text, size| text.ui_size = size,
            window,
            cx,
            &mut subscriptions,
        );
        let editor_size = Self::text_size(
            prefs.text.editor(),
            TextPrefs::EDITOR_SIZES,
            TextPrefs::DEFAULT_EDITOR,
            |text| text.editor(),
            |text, size| text.editor_size = size,
            window,
            cx,
            &mut subscriptions,
        );

        // Empty is the default, which the field shows greyed.
        let mut number = |value: u32, default: u32, kind: Number, window: &mut Window, cx: &mut Context<Self>| {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(default.to_string())
                    .default_value(if value == 0 { String::new() } else { value.to_string() })
            });
            subscriptions.push(
                cx.subscribe_in(&input, window, move |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Blur | InputEvent::PressEnter { .. } => this.commit(kind, window, cx),
                        // Typing counts once it pauses: a field left by a
                        // click on nothing focusable is never blurred.
                        InputEvent::Change => {
                            this.pending = Some(cx.spawn_in(window, async move |this, cx| {
                                cx.background_executor().timer(COMMIT_DELAY).await;
                                let _ = this.update_in(cx, |this, window, cx| this.commit(kind, window, cx));
                            }));
                        }
                        _ => {}
                    }
                }),
            );
            input
        };
        let connect = number(
            prefs.timeouts.connect,
            Timeouts::DEFAULT_CONNECT,
            Number::Timeout,
            window,
            cx,
        );
        let write = number(
            prefs.timeouts.write,
            Timeouts::DEFAULT_WRITE,
            Number::Timeout,
            window,
            cx,
        );
        let read = number(prefs.timeouts.read, Timeouts::DEFAULT_READ, Number::Timeout, window, cx);
        let width = number(
            prefs.window.width,
            WindowPrefs::DEFAULT_WIDTH,
            Number::WindowSize,
            window,
            cx,
        );
        let height = number(
            prefs.window.height,
            WindowPrefs::DEFAULT_HEIGHT,
            Number::WindowSize,
            window,
            cx,
        );

        let server = AppStore::global(cx)
            .server()
            .cloned()
            .map(|state| cx.new(|cx| ServerSection::new(state, window, cx)));
        if let Some(updates) = AppStore::global(cx).updates().cloned() {
            subscriptions.push(cx.observe(&updates, |_, _, cx| cx.notify()));
            subscriptions.push(cx.subscribe(&updates, |this: &mut Self, _, event, cx| {
                this.update_note = match event {
                    UpdateEvent::UpToDate => Some(t!("update.up_to_date", version = crate::VERSION).to_string()),
                    UpdateEvent::CheckFailed(error) => Some(t!("update.check_failed", error = error).to_string()),
                    _ => None,
                };
                cx.notify();
            }));
        }
        Self {
            section: Section::default(),
            focus_handle: cx.focus_handle(),
            built_in: rust_i18n::locale().to_string(),
            // The theme's size, which the window takes as its rem when it
            // draws: a new settings window's own is still the default here.
            rem: cx.theme().font_size,
            main,
            theme,
            language,
            json_view,
            http_version,
            ui_size,
            editor_size,
            connect,
            write,
            read,
            width,
            height,
            pending: None,
            confirming: None,
            server,
            update_note: None,
            _subscriptions: subscriptions,
        }
    }

    /// The view built anew, on the section the reader is in: after a new
    /// language, or the settings put back to their defaults.
    fn rebuild(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let section = self.section;
        *self = Self::new(self.main, window, cx);
        self.section = section;
        self.focus_handle.focus(window, cx);
    }

    /// A drop-down list of `labels` with `chosen` picked; `on_choose` gets
    /// the place of what is picked.
    fn choices(
        labels: Vec<String>,
        chosen: usize,
        on_choose: impl Fn(usize, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Choices {
        let items: Vec<Choice> = labels
            .into_iter()
            .enumerate()
            .map(|(index, label)| Choice {
                label: label.into(),
                index,
            })
            .collect();
        let state = cx.new(|cx| SelectState::new(items, Some(IndexPath::new(chosen)), window, cx));
        subscriptions.push(cx.subscribe(&state, move |_, _, event: &SelectEvent<Vec<Choice>>, cx| {
            if let SelectEvent::Confirm(Some(index)) = event {
                on_choose(*index, cx);
            }
        }));
        state
    }

    /// F14.6: a size in pixels within `sizes`, saved as it is typed or
    /// stepped once it is one; leaving the field puts what it shows right.
    /// The default is kept as `0`, so it follows a new default.
    #[allow(clippy::too_many_arguments)]
    fn text_size(
        value: u32,
        sizes: RangeInclusive<u32>,
        default: u32,
        current: fn(&TextPrefs) -> u32,
        set: fn(&mut TextPrefs, u32),
        window: &mut Window,
        cx: &mut Context<Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value.to_string())
                .step(1.)
                .min(f64::from(*sizes.start()))
                .max(f64::from(*sizes.end()))
        });
        let save = move |size: u32, cx: &mut App| {
            let app = AppStore::global(cx).state().clone();
            let mut text = app.read(cx).prefs().text;
            set(&mut text, if size == default { 0 } else { size });
            app.update(cx, |app, cx| app.set_text(text, cx));
        };
        subscriptions.push(
            cx.subscribe_in(&input, window, move |_, input, event: &InputEvent, window, cx| {
                let typed = input.read(cx).value().trim().parse::<u32>().ok();
                match event {
                    InputEvent::Change => {
                        if let Some(size) = typed.filter(|size| sizes.contains(size)) {
                            save(size, cx);
                        }
                    }
                    InputEvent::Blur | InputEvent::PressEnter { .. } => {
                        let text = AppStore::global(cx).state().read(cx).prefs().text;
                        let size = typed
                            .map(|size| size.clamp(*sizes.start(), *sizes.end()))
                            .unwrap_or(current(&text));
                        if input.read(cx).value().as_ref() != size.to_string() {
                            input.update(cx, |input, cx| input.set_value(size.to_string(), window, cx));
                        }
                        save(size, cx);
                    }
                    _ => {}
                }
            }),
        );
        input
    }

    fn commit(&mut self, kind: Number, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = None;
        match kind {
            Number::Timeout => self.save_timeouts(window, cx),
            Number::WindowSize => self.save_size(window, cx),
        }
    }

    /// A field's number: empty is `0`; `None` when it is not one.
    fn number(input: &Entity<InputState>, cx: &App) -> Option<u32> {
        let text = input.read(cx).value().trim().to_string();
        if text.is_empty() { Some(0) } else { text.parse().ok() }
    }

    /// F14.3.
    fn save_timeouts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let read = |input| Self::number(input, cx);
        let (Some(connect), Some(write), Some(read)) = (read(&self.connect), read(&self.write), read(&self.read))
        else {
            window.push_notification(Notification::warning(t!("settings.not_a_number").to_string()), cx);
            return;
        };
        let app = AppStore::global(cx).state().clone();
        app.update(cx, |app, cx| app.set_timeouts(Timeouts { connect, write, read }, cx));
    }

    /// F14.2: a size of its own, applied to the main window now; at least
    /// 900 × 600.
    fn save_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(width), Some(height)) = (Self::number(&self.width, cx), Self::number(&self.height, cx)) else {
            window.push_notification(Notification::warning(t!("settings.not_a_number").to_string()), cx);
            return;
        };
        let app = AppStore::global(cx).state().clone();
        let mut prefs = app.read(cx).prefs().window;
        prefs.width = if width == 0 {
            0
        } else {
            width.max(WindowPrefs::MIN_WIDTH)
        };
        prefs.height = if height == 0 {
            0
        } else {
            height.max(WindowPrefs::MIN_HEIGHT)
        };
        app.update(cx, |app, cx| app.set_window(prefs, cx));
        self.apply_window(prefs, cx);
    }

    fn set_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        let app = AppStore::global(cx).state().clone();
        let prefs = WindowPrefs {
            maximized,
            ..app.read(cx).prefs().window
        };
        app.update(cx, |app, cx| app.set_window(prefs, cx));
        self.apply_window(prefs, cx);
    }

    /// Puts the main window at the size the settings name, when they name
    /// one.
    fn apply_window(&self, prefs: WindowPrefs, cx: &mut App) {
        let Some(main) = self.main else {
            return;
        };
        let _ = main.update(cx, |_, window, _| {
            if prefs.maximized {
                if !window.is_maximized() {
                    window.zoom_window();
                }
            } else if prefs.width > 0 && prefs.height > 0 {
                if window.is_maximized() {
                    window.zoom_window();
                }
                window.resize(size(px(prefs.width as f32), px(prefs.height as f32)));
            }
        });
    }

    /// Which sections this build has: the Window only where the app sizes
    /// its window, the Server only where it signs in to one.
    fn sections(&self) -> Vec<Section> {
        [
            Section::Appearance,
            Section::Editor,
            Section::Requests,
            Section::Window,
            Section::Server,
            Section::Data,
            Section::About,
        ]
        .into_iter()
        .filter(|section| match section {
            Section::Window => this_build::sizes_window(),
            Section::Server => self.server.is_some(),
            _ => true,
        })
        .collect()
    }

    fn render_nav(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_none()
            .w(rems(11.))
            .h_full()
            .p_2()
            .gap_0p5()
            .border_r_1()
            .border_color(cx.theme().border)
            .children(self.sections().into_iter().map(|section| {
                ListItem::new(section.id())
                    .selected(section == self.section)
                    .child(div().text_sm().child(section.title()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.section = section;
                        this.confirming = None;
                        this.focus_handle.focus(window, cx);
                        cx.notify();
                    }))
            }))
    }

    fn render_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let prefs = AppStore::global(cx).state().read(cx).prefs().clone();
        match self.section {
            // The text size first: each step makes the rows above it
            // taller, so with none its buttons stay under the pointer.
            Section::Appearance => vec![
                setting_row(
                    t!("settings.ui_size").to_string(),
                    Some(size_description(
                        TextPrefs::UI_SIZES,
                        TextPrefs::DEFAULT_UI,
                        "settings.ui_size_desc",
                    )),
                    NumberInput::new(&self.ui_size).small(),
                    cx,
                ),
                setting_row(
                    t!("settings.theme").to_string(),
                    Some(t!("settings.theme_desc").to_string()),
                    Select::new(&self.theme).small(),
                    cx,
                ),
                setting_row(
                    t!("settings.language").to_string(),
                    Some(t!("settings.language_desc").to_string()),
                    Select::new(&self.language).small(),
                    cx,
                ),
            ],
            Section::Editor => vec![
                setting_row(
                    t!("settings.editor_size").to_string(),
                    Some(size_description(
                        TextPrefs::EDITOR_SIZES,
                        TextPrefs::DEFAULT_EDITOR,
                        "settings.editor_size_desc",
                    )),
                    NumberInput::new(&self.editor_size).small(),
                    cx,
                ),
                setting_row(
                    t!("settings.wrap").to_string(),
                    Some(t!("settings.wrap_desc").to_string()),
                    Switch::new("settings-wrap")
                        .checked(prefs.text.wrap)
                        .on_click(|checked, _, cx| {
                            let app = AppStore::global(cx).state().clone();
                            let text = TextPrefs {
                                wrap: *checked,
                                ..app.read(cx).prefs().text
                            };
                            app.update(cx, |app, cx| app.set_text(text, cx));
                        }),
                    cx,
                ),
                setting_row(
                    t!("settings.json_view").to_string(),
                    Some(t!("settings.json_view_desc").to_string()),
                    Select::new(&self.json_view).small(),
                    cx,
                ),
            ],
            Section::Requests => vec![
                setting_row(
                    t!("settings.http_version").to_string(),
                    Some(t!("settings.http_version_desc").to_string()),
                    Select::new(&self.http_version).small(),
                    cx,
                ),
                setting_row(
                    t!("settings.timeout_connect").to_string(),
                    Some(t!("settings.timeout_connect_desc", default = Timeouts::DEFAULT_CONNECT).to_string()),
                    Input::new(&self.connect).small(),
                    cx,
                ),
                setting_row(
                    t!("settings.timeout_write").to_string(),
                    Some(t!("settings.timeout_write_desc", default = Timeouts::DEFAULT_WRITE).to_string()),
                    Input::new(&self.write).small(),
                    cx,
                ),
                setting_row(
                    t!("settings.timeout_read").to_string(),
                    Some(t!("settings.timeout_read_desc", default = Timeouts::DEFAULT_READ).to_string()),
                    Input::new(&self.read).small(),
                    cx,
                ),
                setting_row(
                    t!("settings.follow_redirects").to_string(),
                    Some(t!("settings.follow_redirects_desc", count = MAX_REDIRECTS).to_string()),
                    Switch::new("settings-redirects")
                        .checked(prefs.sending.follow_redirects)
                        .on_click(set_sending(|sending, on| sending.follow_redirects = on)),
                    cx,
                ),
                setting_row(
                    t!("settings.verify_certs").to_string(),
                    Some(t!("settings.verify_certs_desc").to_string()),
                    Switch::new("settings-certs")
                        .checked(!prefs.sending.accept_invalid_certs)
                        .on_click(set_sending(|sending, on| sending.accept_invalid_certs = !on)),
                    cx,
                ),
            ],
            Section::Window => vec![
                setting_row(
                    t!("settings.maximized").to_string(),
                    Some(t!("settings.maximized_desc").to_string()),
                    Switch::new("settings-maximized")
                        .checked(prefs.window.maximized)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| this.set_maximized(*checked, cx))),
                    cx,
                ),
                setting_row(
                    t!("settings.width").to_string(),
                    Some(t!("settings.width_desc", min = WindowPrefs::MIN_WIDTH).to_string()),
                    Input::new(&self.width).small(),
                    cx,
                ),
                setting_row(
                    t!("settings.height").to_string(),
                    Some(t!("settings.height_desc", min = WindowPrefs::MIN_HEIGHT).to_string()),
                    Input::new(&self.height).small(),
                    cx,
                ),
            ],
            Section::Server => self
                .server
                .clone()
                .map(|server| server.into_any_element())
                .into_iter()
                .collect(),
            Section::Data => self.render_data(cx),
            Section::About => {
                let mut rows = self.render_updates(cx);
                rows.push(self.render_about(cx));
                rows
            }
        }
    }

    /// F15.3: where the data is, and what of the reader's own can be
    /// cleared. Each asks first, under its row.
    fn render_data(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows = vec![setting_row(
            t!("settings.data").to_string(),
            Some(this_build::data_location()),
            div().children(this_build::shows_data().then(|| {
                Button::new("settings-show-data")
                    .outline()
                    .small()
                    .label(t!("settings.show_data").to_string())
                    .on_click(|_, _, cx| this_build::show_data(cx))
            })),
            cx,
        )];
        for (clearing, id, label, description, action) in [
            (
                Clearing::History,
                "settings-clear-history",
                t!("settings.clear_history"),
                t!("settings.clear_history_description"),
                t!("settings.clear"),
            ),
            (
                Clearing::Workspaces,
                "settings-clear-workspaces",
                t!("settings.clear_workspaces"),
                t!("settings.clear_workspaces_description"),
                t!("settings.clear"),
            ),
            (
                Clearing::Settings,
                "settings-reset",
                t!("settings.reset"),
                t!("settings.reset_description"),
                t!("settings.reset_action"),
            ),
        ] {
            rows.push(setting_row(
                label.to_string(),
                Some(description.to_string()),
                Button::new(id)
                    .outline()
                    .small()
                    .label(action.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.confirming = Some(clearing);
                        cx.notify();
                    })),
                cx,
            ));
            rows.extend(self.render_confirm(clearing, cx));
        }
        rows
    }

    fn render_confirm(&self, clearing: Clearing, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.confirming != Some(clearing) {
            return None;
        }
        let (title, description, ok) = match clearing {
            Clearing::History => (
                t!("settings.clear_history"),
                t!("settings.clear_history_description"),
                t!("settings.clear"),
            ),
            Clearing::Workspaces => (
                t!("settings.clear_workspaces"),
                t!("settings.clear_workspaces_description"),
                t!("settings.clear"),
            ),
            Clearing::Settings => (
                t!("settings.reset_title"),
                t!("settings.reset_description"),
                t!("settings.reset_action"),
            ),
        };
        let cancel = cx.entity().downgrade();
        let view = cancel.clone();
        Some(
            inline_confirm(
                title.to_string(),
                description.to_string(),
                None,
                ok.to_string(),
                move |_, cx| {
                    let _ = cancel.update(cx, |this, cx| {
                        this.confirming = None;
                        cx.notify();
                    });
                },
                move |window, cx| {
                    let app = AppStore::global(cx).state().clone();
                    app.update(cx, |app, cx| match clearing {
                        Clearing::History => app.clear_history(cx),
                        Clearing::Workspaces => app.clear_workspaces(cx),
                        Clearing::Settings => app.reset_prefs(cx),
                    });
                    let _ = view.update(cx, |this, cx| {
                        this.confirming = None;
                        // Every field shows the default now.
                        if clearing == Clearing::Settings {
                            this.rebuild(window, cx);
                        }
                        cx.notify();
                    });
                },
                cx,
            )
            .into_any_element(),
        )
    }

    /// F1.7: what this build is, and where it comes from.
    /// The desktop's update check: asked for now, and whether it runs at
    /// start. A release found here is shown in the main window, where the
    /// title bar's chip opens it again.
    fn render_updates(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(updates) = AppStore::global(cx).updates().cloned() else {
            return Vec::new();
        };
        let state = updates.read(cx);
        let phase = state.phase();
        let (status, found) = match (phase, state.available()) {
            (UpdatePhase::Checking { .. }, _) => (t!("update.checking").to_string(), false),
            (UpdatePhase::Installed, _) => (t!("update.installed_short").to_string(), true),
            (UpdatePhase::Downloading, Some(info)) => {
                (t!("update.downloading_short", version = info.version).to_string(), true)
            }
            (_, Some(info)) => (t!("update.found", version = info.version).to_string(), true),
            _ => (
                self.update_note
                    .clone()
                    .unwrap_or_else(|| t!("update.check_desc", version = crate::VERSION).to_string()),
                false,
            ),
        };
        let control = if found {
            let main = self.main;
            Button::new("settings-update-show")
                .outline()
                .small()
                .label(t!("update.show").to_string())
                .on_click(move |_, _, cx| {
                    if let Some(main) = main {
                        let _ = main.update(cx, |_, window, cx| {
                            window.activate_window();
                            open_update_dialog(window, cx);
                        });
                    }
                })
        } else {
            Button::new("settings-update-check")
                .outline()
                .small()
                .label(t!("update.check").to_string())
                .loading(matches!(phase, UpdatePhase::Checking { .. }))
                .on_click(move |_, _, cx| updates.update(cx, |updates, cx| updates.check(true, cx)))
        };
        let auto_check = AppStore::global(cx).state().read(cx).prefs().updates.auto_check;
        vec![
            setting_row(t!("update.check").to_string(), Some(status), control, cx),
            setting_row(
                t!("update.auto_check").to_string(),
                Some(t!("update.auto_check_desc").to_string()),
                Switch::new("settings-update-auto")
                    .checked(auto_check)
                    .on_click(|checked, _, cx| {
                        let app = AppStore::global(cx).state().clone();
                        app.update(cx, |app, cx| app.set_auto_update_check(*checked, cx));
                    }),
                cx,
            ),
        ]
    }

    fn render_about(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let mut rows: Vec<(String, SharedString)> = vec![
            (t!("settings.version").to_string(), crate::VERSION.into()),
            (t!("settings.platform").to_string(), this_build::platform().into()),
        ];
        if let Some(version) = this_build::os_version() {
            rows.push((t!("settings.os_version").to_string(), version.into()));
        }
        rows.push(("User-Agent".to_string(), this_build::user_agent().into()));
        let line = |label: String, value: AnyElement| {
            h_flex()
                .gap_3()
                .py_1p5()
                .items_start()
                .text_sm()
                .child(div().w(rems(8.)).flex_none().text_color(muted).child(label))
                .child(h_flex().flex_1().min_w_0().child(value))
        };
        v_flex()
            // A long value (a browser's whole user agent) wraps inside the
            // dialog rather than running past its edge.
            .children(
                rows.into_iter()
                    .map(|(label, value)| line(label, div().flex_1().min_w_0().child(value).into_any_element())),
            )
            .child(line(
                t!("settings.homepage").to_string(),
                Button::new("settings-homepage")
                    .link()
                    .small()
                    .label(HOMEPAGE.trim_start_matches("https://"))
                    .on_click(|_, _, cx| cx.open_url(HOMEPAGE))
                    .into_any_element(),
            ))
            .into_any_element()
    }
}

/// One setting: what it is and what it does on the left, its control on
/// the right, as zedis lays its settings out.
pub(super) fn setting_row(
    label: String,
    description: Option<String>,
    control: impl IntoElement,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .gap_8()
        .py_2()
        .child(
            // `min_w_0` lets a long description wrap rather than push the
            // control out of the row.
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().child(label))
                .children(description.map(|text| div().text_xs().text_color(muted).child(text))),
        )
        // The controls line up on the right; `flex_none` keeps their column
        // against a long description.
        .child(h_flex().w(rems(14.)).flex_none().justify_end().child(control))
        .into_any_element()
}

/// The text of a size row: its range and its default.
fn size_description(sizes: RangeInclusive<u32>, default: u32, key: &str) -> String {
    t!(key, min = sizes.start(), max = sizes.end(), default = default).to_string()
}

/// F14.8, F14.9: what a switch changes in how requests go out.
fn set_sending(change: fn(&mut SendPrefs, bool)) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
    move |checked, _, cx| {
        let app = AppStore::global(cx).state().clone();
        let mut sending = app.read(cx).prefs().sending;
        change(&mut sending, *checked);
        app.update(cx, |app, cx| app.set_sending(sending, cx));
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if *rust_i18n::locale() != *self.built_in {
            self.rebuild(window, cx);
        }
        let muted = cx.theme().muted_foreground;
        let header = v_flex()
            .gap_1()
            .pt_5()
            .pb_4()
            .child(div().text_sm().font_bold().child(self.section.title()))
            .child(div().text_xs().text_color(muted).child(self.section.description()));
        let view = h_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .items_start()
            .child(self.render_nav(cx))
            .child(
                v_flex()
                    .id("settings-rows")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .px_6()
                    .pb_4()
                    .child(header)
                    .children(self.render_rows(cx))
                    .overflow_y_scrollbar(),
            );
        fixed_rem(self.rem, view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Extras;
    use cyberapi_core::prefs::AppPrefs;
    use cyberapi_http::{HttpEngine, PreparedRequest, SendFuture};
    use cyberapi_store::MemoryStore;
    use gpui_kit::TestAppContext;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use std::sync::Arc;

    struct NoEngine;

    impl HttpEngine for NoEngine {
        fn send(&self, _: PreparedRequest) -> SendFuture {
            Box::pin(std::future::pending())
        }
    }

    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<SettingsView>) {
        let store = MemoryStore::new();
        cx.update(|cx| {
            gpui_kit::init(cx);
            AppStore::init(
                store.storage(),
                Arc::new(NoEngine),
                AppPrefs::default(),
                Extras::default(),
                cx,
            );
        });
        let mut view = None;
        // Tall enough that the Data section's confirmation is on screen: a
        // click lands only on what is.
        let handle = cx.open_window(size(px(760.), px(1000.)), |window, cx| {
            let settings = cx.new(|cx| SettingsView::new(None, window, cx));
            view = Some(settings.clone());
            Root::new(settings, window, cx)
        });
        (handle.into(), view.expect("the window built the settings"))
    }

    fn prefs(cx: &mut TestAppContext) -> AppPrefs {
        cx.read(|cx| AppStore::global(cx).state().read(cx).prefs().clone())
    }

    fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, ids: &[&'static str]) {
        for id in ids {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click(*id, cx);
            })
            .expect("the window is open");
            cx.run_until_parked();
        }
    }

    /// Puts `text` in a field and leaves it, as a person does.
    fn enter(cx: &mut TestAppContext, handle: AnyWindowHandle, input: &Entity<InputState>, text: &str) -> String {
        cx.update_window(handle, |_, window, cx| {
            input.update(cx, |input, cx| {
                input.set_value(text.to_string(), window, cx);
                cx.emit(InputEvent::Blur);
            });
        })
        .expect("the window is open");
        cx.run_until_parked();
        cx.read(|cx| input.read(cx).value().to_string())
    }

    /// Each section lists its own settings, and its switches save what they
    /// say.
    #[gpui_kit::test]
    async fn the_sections_hold_their_switches(cx: &mut TestAppContext) {
        let (handle, _) = open(cx);
        let shown = |cx: &mut TestAppContext, id: &'static str| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find(id).is_some()
            })
            .expect("the window is open")
        };
        assert!(!shown(cx, "settings-wrap"), "the Editor section's, not the first's");

        click(cx, handle, &["settings-editor", "settings-wrap"]);
        assert!(!prefs(cx).text.wrap);
        click(
            cx,
            handle,
            &["settings-requests", "settings-redirects", "settings-certs"],
        );
        assert_eq!(
            prefs(cx).sending,
            SendPrefs {
                follow_redirects: true,
                accept_invalid_certs: true,
                http_version: HttpVersion::Auto,
            }
        );
        assert!(!shown(cx, "settings-wrap"));
    }

    /// F14.6: a size is kept within its range, and the default as `0`.
    #[gpui_kit::test]
    async fn a_text_size_is_saved_within_its_range(cx: &mut TestAppContext) {
        let (handle, view) = open(cx);
        let ui = cx.read(|cx| view.read(cx).ui_size.clone());
        assert_eq!(cx.read(|cx| ui.read(cx).value().to_string()), "14");

        assert_eq!(enter(cx, handle, &ui, "16"), "16");
        assert_eq!(prefs(cx).text.ui_size, 16);
        assert_eq!(enter(cx, handle, &ui, "25"), "18", "the largest offered");
        assert_eq!(prefs(cx).text.ui_size, 18);
        assert_eq!(enter(cx, handle, &ui, "big"), "18", "what is in effect");
        assert_eq!(enter(cx, handle, &ui, "14"), "14");
        assert_eq!(prefs(cx).text.ui_size, 0, "the default follows a new default");

        let editor = cx.read(|cx| view.read(cx).editor_size.clone());
        assert_eq!(enter(cx, handle, &editor, "5"), "11", "the smallest offered");
        assert_eq!(prefs(cx).text.editor_size, 11);
    }

    /// The lists save what is picked in them; putting every setting back
    /// shows the defaults in every field.
    #[gpui_kit::test]
    async fn the_lists_save_and_a_reset_shows_the_defaults(cx: &mut TestAppContext) {
        let (handle, view) = open(cx);
        let pick = |cx: &mut TestAppContext, list: fn(&SettingsView) -> Choices, index: usize| {
            cx.update(|cx| {
                list(view.read(cx)).update(cx, |_, cx| cx.emit(SelectEvent::Confirm(Some(index))));
            });
            cx.run_until_parked();
        };
        pick(cx, |view| view.theme.clone(), 2);
        assert_eq!(prefs(cx).theme, ThemeMode::Dark);
        pick(cx, |view| view.language.clone(), 2);
        assert_eq!(prefs(cx).language.as_deref(), Some("en"));
        pick(cx, |view| view.http_version.clone(), 1);
        assert_eq!(prefs(cx).sending.http_version, HttpVersion::Http1);
        let connect = cx.read(|cx| view.read(cx).connect.clone());
        enter(cx, handle, &connect, "30");
        assert_eq!(prefs(cx).timeouts.connect, 30);

        click(cx, handle, &["settings-data", "settings-reset", "inline-ok"]);
        assert_eq!(prefs(cx).theme, ThemeMode::System);
        assert_eq!(prefs(cx).timeouts, Timeouts::default());
        assert_eq!(prefs(cx).sending, SendPrefs::default());
        let connect = cx.read(|cx| view.read(cx).connect.clone());
        assert_eq!(
            cx.read(|cx| connect.read(cx).value().to_string()),
            "",
            "the field was built anew"
        );
        assert_eq!(
            cx.read(|cx| view.read(cx).section),
            Section::Data,
            "on the same section"
        );
    }
}
