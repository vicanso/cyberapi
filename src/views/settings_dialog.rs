//! The application's settings (M14), in a dialog: theme, language, request
//! timeouts, the window's size on the desktop, what this build is, and a way
//! back to the defaults.

use crate::states::AppStore;
use crate::this_build;
use crate::views::inline_confirm::inline_confirm;
use cyberapi_core::prefs::{ThemeMode, Timeouts, WindowPrefs};
use gpui_kit::assets::IconName as Icons;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Task, Window, div, px, size,
};
use rust_i18n::t;
use std::time::Duration;

/// The project's home: its releases (and the nightly), its issues.
const HOMEPAGE: &str = env!("CARGO_PKG_REPOSITORY");

/// The languages offered (F15.2): `None` follows the system.
const LANGUAGES: [(Option<&str>, &str); 4] = [
    (None, ""),
    (Some("zh-CN"), "中文"),
    (Some("en"), "English"),
    (Some("uk"), "Українська"),
];

pub fn open_settings_dialog(window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| SettingsView::new(window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(t!("settings.title").to_string())
            .w(px(560.))
            .child(view.clone())
    });
}

/// What the dialog clears or resets, once confirmed (F15.3).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Clearing {
    Settings,
    History,
    Workspaces,
}

struct SettingsView {
    connect: Entity<InputState>,
    write: Entity<InputState>,
    read: Entity<InputState>,
    width: Entity<InputState>,
    height: Entity<InputState>,
    /// A number saved once typing pauses.
    pending: Option<Task<()>>,
    /// What the dialog is asking to confirm.
    confirming: Option<Clearing>,
    _subscriptions: Vec<Subscription>,
}

/// How long typing in a number field waits before it counts.
const COMMIT_DELAY: Duration = Duration::from_millis(600);

impl SettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let prefs = AppStore::global(cx).state().read(cx).prefs().clone();
        // Empty is the default, which the field shows greyed.
        let number = |value: u32, default: u32, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(default.to_string())
                    .default_value(if value == 0 { String::new() } else { value.to_string() })
            })
        };
        let connect = number(prefs.timeouts.connect, Timeouts::DEFAULT_CONNECT, window, cx);
        let write = number(prefs.timeouts.write, Timeouts::DEFAULT_WRITE, window, cx);
        let read = number(prefs.timeouts.read, Timeouts::DEFAULT_READ, window, cx);
        let width = number(prefs.window.width, WindowPrefs::DEFAULT_WIDTH, window, cx);
        let height = number(prefs.window.height, WindowPrefs::DEFAULT_HEIGHT, window, cx);
        let mut subscriptions = Vec::new();
        for (input, size) in [
            (&connect, false),
            (&write, false),
            (&read, false),
            (&width, true),
            (&height, true),
        ] {
            subscriptions.push(
                cx.subscribe_in(input, window, move |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Blur | InputEvent::PressEnter { .. } => this.commit(size, window, cx),
                        // Typing counts once it pauses: a field left by a click
                        // on nothing focusable is never blurred.
                        InputEvent::Change => {
                            this.pending = Some(cx.spawn_in(window, async move |this, cx| {
                                cx.background_executor().timer(COMMIT_DELAY).await;
                                let _ = this.update_in(cx, |this, window, cx| this.commit(size, window, cx));
                            }));
                        }
                        _ => {}
                    }
                }),
            );
        }
        Self {
            connect,
            write,
            read,
            width,
            height,
            pending: None,
            confirming: None,
            _subscriptions: subscriptions,
        }
    }

    fn commit(&mut self, size: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = None;
        if size {
            self.save_size(window, cx);
        } else {
            self.save_timeouts(window, cx);
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

    /// F14.2: a size of its own, applied now; at least 900 × 600.
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
        apply_window(prefs, window);
        cx.notify();
    }

    fn set_maximized(&mut self, maximized: bool, window: &mut Window, cx: &mut Context<Self>) {
        let app = AppStore::global(cx).state().clone();
        let prefs = WindowPrefs {
            maximized,
            ..app.read(cx).prefs().window
        };
        app.update(cx, |app, cx| app.set_window(prefs, cx));
        apply_window(prefs, window);
        cx.notify();
    }

    fn ask(&mut self, clearing: Clearing, cx: &mut Context<Self>) {
        self.confirming = Some(clearing);
        cx.notify();
    }

    /// F15.3, asked inside the dialog: the browser build draws no dialog
    /// stacked on this one. Done, the dialog closes.
    fn render_confirm(&self, clearing: Clearing, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.confirming != Some(clearing) {
            return None;
        }
        let (title, description, ok) = match clearing {
            Clearing::Settings => (
                t!("settings.reset_title"),
                t!("settings.reset_description"),
                t!("settings.reset"),
            ),
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
        };
        let view = cx.entity().downgrade();
        Some(
            inline_confirm(
                title.to_string(),
                description.to_string(),
                None,
                ok.to_string(),
                move |_, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.confirming = None;
                        cx.notify();
                    });
                },
                move |window, cx| {
                    let app = AppStore::global(cx).state().clone();
                    app.update(cx, |app, cx| match clearing {
                        Clearing::Settings => app.reset_prefs(cx),
                        Clearing::History => app.clear_history(cx),
                        Clearing::Workspaces => app.clear_workspaces(cx),
                    });
                    window.close_dialog(cx);
                },
                cx,
            )
            .into_any_element(),
        )
    }

    /// F15.3: the reader's responses and their state of every project.
    fn render_local_data(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let row = |id: &'static str, label: String, clearing: Clearing, cx: &mut Context<Self>| {
            h_flex()
                .gap_3()
                .items_center()
                .text_sm()
                .child(div().flex_1().min_w_0().child(label))
                .child(
                    Button::new(id)
                        .outline()
                        .xsmall()
                        .label(t!("settings.clear").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| this.ask(clearing, cx))),
                )
        };
        v_flex()
            .gap_2()
            .child(row(
                "clear-history",
                t!("settings.clear_history").to_string(),
                Clearing::History,
                cx,
            ))
            .children(self.render_confirm(Clearing::History, cx))
            .child(row(
                "clear-workspaces",
                t!("settings.clear_workspaces").to_string(),
                Clearing::Workspaces,
                cx,
            ))
            .children(self.render_confirm(Clearing::Workspaces, cx))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("settings.local_data_hint").to_string()),
            )
    }

    fn render_reset(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.render_confirm(Clearing::Settings, cx) {
            Some(confirm) => confirm,
            None => h_flex()
                .justify_end()
                .child(
                    Button::new("settings-reset")
                        .ghost()
                        .small()
                        .label(t!("settings.reset").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.ask(Clearing::Settings, cx))),
                )
                .into_any_element(),
        }
    }
}

/// Puts the window at the size the settings name, when they name one.
fn apply_window(prefs: WindowPrefs, window: &mut Window) {
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
}

/// A row of choices, the chosen one filled, each with its icon if it has
/// one.
fn choices<T: PartialEq + Clone + 'static>(
    id: &'static str,
    options: Vec<(T, String, Option<Icons>)>,
    chosen: T,
    on_choose: impl Fn(T, &mut Window, &mut App) + Clone + 'static,
) -> impl IntoElement {
    h_flex().gap_1().flex_wrap().children(
        options
            .into_iter()
            .enumerate()
            .map(move |(index, (value, label, icon))| {
                let on_choose = on_choose.clone();
                let selected = value == chosen;
                Button::new((id, index))
                    .small()
                    .label(label)
                    .when_some(icon, |button, icon| button.icon(icon))
                    .map(|button| if selected { button.primary() } else { button.outline() })
                    .on_click(move |_, window, cx| on_choose(value.clone(), window, cx))
            }),
    )
}

fn section(title: String, content: impl IntoElement) -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(div().text_sm().font_semibold().child(title))
        .child(content)
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prefs = AppStore::global(cx).state().read(cx).prefs().clone();
        let muted = cx.theme().muted_foreground;
        let theme = choices(
            "settings-theme",
            vec![
                (
                    ThemeMode::System,
                    t!("settings.theme_system").to_string(),
                    Some(Icons::Monitor),
                ),
                (
                    ThemeMode::Light,
                    t!("settings.theme_light").to_string(),
                    Some(Icons::Sun),
                ),
                (
                    ThemeMode::Dark,
                    t!("settings.theme_dark").to_string(),
                    Some(Icons::Moon),
                ),
            ],
            prefs.theme,
            |theme, _, cx| {
                let app = AppStore::global(cx).state().clone();
                app.update(cx, |app, cx| app.set_theme(theme, cx));
            },
        );
        let language = choices(
            "settings-language",
            LANGUAGES
                .iter()
                .map(|(code, name)| {
                    let label = if code.is_none() {
                        t!("settings.language_system").to_string()
                    } else {
                        name.to_string()
                    };
                    (code.map(str::to_string), label, None)
                })
                .collect(),
            prefs.language.clone(),
            |language, _, cx| {
                let app = AppStore::global(cx).state().clone();
                app.update(cx, |app, cx| app.set_language(language, cx));
            },
        );
        let field = |label: String, input: &Entity<InputState>| {
            h_flex()
                .gap_2()
                .items_center()
                .child(div().text_sm().text_color(muted).child(label))
                .child(div().w(px(80.)).child(Input::new(input).small()))
        };
        let timeouts = v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_4()
                    .child(field(t!("settings.timeout_connect").to_string(), &self.connect))
                    .child(field(t!("settings.timeout_write").to_string(), &self.write))
                    .child(field(t!("settings.timeout_read").to_string(), &self.read)),
            )
            .child(
                div().text_xs().text_color(muted).child(
                    t!(
                        "settings.timeout_hint",
                        connect = Timeouts::DEFAULT_CONNECT,
                        write = Timeouts::DEFAULT_WRITE,
                        read = Timeouts::DEFAULT_READ
                    )
                    .to_string(),
                ),
            );
        let window_size: Option<AnyElement> =
            this_build::sizes_window().then(|| {
                section(
                    t!("settings.window").to_string(),
                    h_flex()
                        .gap_4()
                        .items_center()
                        .child(
                            Checkbox::new("settings-maximized")
                                .label(t!("settings.maximized").to_string())
                                .checked(prefs.window.maximized)
                                .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                    this.set_maximized(*checked, window, cx)
                                })),
                        )
                        .child(field(t!("settings.width").to_string(), &self.width))
                        .child(field(t!("settings.height").to_string(), &self.height)),
                )
                .into_any_element()
            });
        // F1.7.
        let mut about_rows: Vec<(String, SharedString)> = vec![
            (t!("settings.version").to_string(), crate::VERSION.into()),
            (t!("settings.platform").to_string(), this_build::platform().into()),
        ];
        if let Some(version) = this_build::os_version() {
            about_rows.push((t!("settings.os_version").to_string(), version.into()));
        }
        about_rows.extend([
            ("User-Agent".to_string(), this_build::user_agent().into()),
            (t!("settings.data").to_string(), this_build::data_location().into()),
        ]);
        let data_row = about_rows.len() - 1;
        // Where the app comes from: releases, nightly builds, issues.
        let homepage = h_flex()
            .gap_3()
            .text_sm()
            .child(
                div()
                    .w(px(110.))
                    .flex_none()
                    .text_color(muted)
                    .child(t!("settings.homepage").to_string()),
            )
            .child(
                Button::new("settings-homepage")
                    .link()
                    .small()
                    .label(HOMEPAGE.trim_start_matches("https://"))
                    .on_click(|_, _, cx| cx.open_url(HOMEPAGE)),
            );
        let about = v_flex()
            .gap_1()
            .text_sm()
            .children(about_rows.into_iter().enumerate().map(|(index, (label, value))| {
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(div().w(px(110.)).flex_none().text_color(muted).child(label))
                    .child(div().flex_1().min_w_0().child(value))
                    // The folder, opened where the system shows files.
                    .when(index == data_row && this_build::shows_data(), |row| {
                        row.child(
                            Button::new("settings-show-data")
                                .outline()
                                .xsmall()
                                .label(t!("settings.show_data").to_string())
                                .on_click(|_, _, cx| this_build::show_data(cx)),
                        )
                    })
            }))
            .child(homepage);
        // Scrolls inside the dialog rather than running off the window.
        let room = (window.viewport_size().height - px(220.)).max(px(240.));
        v_flex()
            .id("settings")
            .max_h(room)
            .overflow_y_scrollbar()
            .pr_2()
            .gap_5()
            .child(section(t!("settings.theme").to_string(), theme))
            .child(section(t!("settings.language").to_string(), language))
            .child(section(t!("settings.timeouts").to_string(), timeouts))
            .children(window_size)
            .child(section(t!("settings.about").to_string(), about))
            .child(section(
                t!("settings.local_data").to_string(),
                self.render_local_data(cx),
            ))
            .child(self.render_reset(cx))
    }
}
