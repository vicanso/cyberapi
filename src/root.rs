//! The main window's content: the title bar and the page the route names.

use crate::APP_NAME;
use crate::i18n;
use crate::states::{AppEvent, AppStore, Route};
use crate::views::{
    DashboardView, WorkspaceView, open_accounts_dialog, open_cookies_dialog, open_password_dialog,
    open_settings_dialog, open_shortcuts_dialog, open_teams_dialog, open_variables_dialog,
};
use cyberapi_core::model::VariableCategory;
use cyberapi_core::prefs::ThemeMode;
#[cfg(not(target_family = "wasm"))]
use cyberapi_core::prefs::{MonitorPlacement, fills_monitor};
use gpui_kit::assets::IconName as Icons;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _, Theme, ThemeMode as ComponentThemeMode, TitleBar,
    WindowExt as _, h_flex, try_parse_color, v_flex,
};
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, PathPromptOptions,
    Render, Styled as _, Subscription, Window, div, hsla, px,
};
#[cfg(not(target_family = "wasm"))]
use gpui_kit::{Pixels, Task};
use rust_i18n::t;
use std::path::PathBuf;
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

#[cfg(not(target_family = "wasm"))]
fn monitor_id(window: &Window, cx: &App) -> Option<String> {
    Some(window.display(cx)?.uuid().ok()?.to_string())
}

#[cfg(not(target_family = "wasm"))]
fn window_is_zoomed(window: &Window, cx: &App) -> bool {
    if window.is_maximized() || window.is_fullscreen() || window.is_simple_fullscreen() {
        return true;
    }
    let Some(display) = window.display(cx) else {
        return false;
    };
    let visible = display.visible_bounds().size;
    fills_monitor(
        px_u32(window.bounds().size.width),
        px_u32(window.bounds().size.height),
        px_u32(visible.width),
        px_u32(visible.height),
    )
}

#[cfg(not(target_family = "wasm"))]
fn px_i32(value: Pixels) -> i32 {
    f32::from(value).round() as i32
}

#[cfg(not(target_family = "wasm"))]
fn px_u32(value: Pixels) -> u32 {
    let rounded = f32::from(value).round();
    if rounded.is_sign_negative() { 0 } else { rounded as u32 }
}

enum Page {
    Dashboard(Entity<DashboardView>),
    Workspace(Entity<WorkspaceView>),
}

pub struct CyberApi {
    page: Page,
    /// The settings the window was last drawn with, to see what changed.
    theme: ThemeMode,
    language: Option<String>,
    _subscriptions: Vec<Subscription>,
    /// A windowed position waiting until it stops moving. A zoom drops it,
    /// so an animation frame never becomes the size to restore.
    #[cfg(not(target_family = "wasm"))]
    pending_window: Option<(String, MonitorPlacement)>,
    #[cfg(not(target_family = "wasm"))]
    window_save: Option<Task<()>>,
    /// The latest sample was a zoom or a frame that fills the monitor.
    #[cfg(not(target_family = "wasm"))]
    zoomed: bool,
}

impl CyberApi {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppStore::global(cx).state().clone();
        apply_theme(state.read(cx).prefs().theme, window, cx);
        let subscriptions = vec![
            // F14.1: "follow the system" keeps following it.
            cx.observe_window_appearance(window, |_, window, cx| {
                let theme = AppStore::global(cx).state().read(cx).prefs().theme;
                if theme == ThemeMode::System {
                    apply_theme(theme, window, cx);
                }
            }),
            cx.observe_in(&state, window, |this, _, window, cx| {
                this.follow_settings(window, cx);
                this.follow_route(window, cx);
            }),
            cx.subscribe_in(&state, window, |this, _, event, window, cx| {
                let note = match event {
                    AppEvent::BackedUp(path) => Notification::success(t!("backup.saved", path = path).to_string()),
                    AppEvent::Restored => {
                        // The list again, read from what was restored.
                        let route = AppStore::global(cx).state().read(cx).route().clone();
                        this.page = Self::page_for(route, window, cx);
                        cx.notify();
                        Notification::success(t!("backup.restored").to_string())
                    }
                    AppEvent::Cleared => {
                        let route = AppStore::global(cx).state().read(cx).route().clone();
                        this.page = Self::page_for(route, window, cx);
                        cx.notify();
                        Notification::success(t!("settings.cleared").to_string())
                    }
                    AppEvent::Failed(error) => Notification::error(i18n::store_error(error)),
                    AppEvent::BackupFailed(failure) => Notification::error(i18n::backup_error(failure)),
                    AppEvent::FileFailed(message) => Notification::error(message.clone()),
                };
                window.push_notification(note, cx);
            }),
        ];
        let page = Self::page_for(state.read(cx).route().clone(), window, cx);
        let prefs = state.read(cx).prefs().clone();
        let root = Self {
            page,
            theme: prefs.theme,
            language: prefs.language,
            _subscriptions: subscriptions,
            #[cfg(not(target_family = "wasm"))]
            pending_window: None,
            #[cfg(not(target_family = "wasm"))]
            window_save: None,
            #[cfg(not(target_family = "wasm"))]
            zoomed: false,
        };
        Self::open(root, window, cx)
    }

    #[cfg(not(target_family = "wasm"))]
    fn open(mut self, window: &mut Window, cx: &mut Context<Self>) -> Self {
        self.watch_window(window, cx);
        self.set_title(window, cx);
        self
    }

    #[cfg(target_family = "wasm")]
    fn open(self, window: &mut Window, cx: &mut Context<Self>) -> Self {
        self.set_title(window, cx);
        self
    }

    /// The window's position on its monitor, written once it settles.
    /// macOS close only hides, so quitting flushes whatever is still waiting.
    #[cfg(not(target_family = "wasm"))]
    fn watch_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._subscriptions
            .push(cx.observe_window_bounds(window, |this, window, cx| {
                this.sample_window(window, cx);
            }));
        self._subscriptions
            .push(cx.observe_window_display(window, |this, window, cx| {
                this.sample_window(window, cx);
            }));
        self._subscriptions.push(cx.on_app_quit(|this, cx| {
            if !this.zoomed {
                this.commit_pending(cx);
            }
            this.pending_window = None;
            this.window_save = None;
            let state = AppStore::global(cx).state().clone();
            let (prefs, gate) = state.update(cx, |state, _| state.prefs_to_flush());
            let call = AppStore::global(cx).storage().prefs().set_app(prefs);
            let turn = gate.lock_arc();
            async move {
                let _turn = turn.await;
                if let Err(error) = call.await {
                    tracing::warn!(%error, "the settings could not be saved");
                }
            }
        }));
    }

    /// A settled windowed frame is kept. A zoom, a fullscreen, or a frame
    /// that already fills the monitor is not: that would replace the size
    /// from before the double-click.
    #[cfg(not(target_family = "wasm"))]
    fn sample_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = monitor_id(window, cx) else {
            return;
        };
        let zoomed = window_is_zoomed(window, cx);
        self.zoomed = zoomed;
        if zoomed {
            self.pending_window = None;
            self.window_save = None;
            let state = AppStore::global(cx).state().clone();
            state.update(cx, |state, cx| {
                state.note_monitor(&id, MonitorPlacement::default(), true, cx);
            });
            return;
        }
        let placement = MonitorPlacement {
            x: px_i32(window.bounds().origin.x),
            y: px_i32(window.bounds().origin.y),
            width: px_u32(window.viewport_size().width),
            height: px_u32(window.viewport_size().height),
        };
        if !placement.usable() {
            return;
        }
        if self
            .pending_window
            .as_ref()
            .is_some_and(|(pending_id, pending)| pending_id == &id && *pending == placement)
        {
            return;
        }
        self.pending_window = Some((id, placement));
        self.window_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            let _ = this.update(cx, |this, cx| this.commit_pending(cx));
        }));
    }

    #[cfg(not(target_family = "wasm"))]
    fn commit_pending(&mut self, cx: &mut Context<Self>) {
        let Some((id, placement)) = self.pending_window.take() else {
            return;
        };
        self.window_save = None;
        let state = AppStore::global(cx).state().clone();
        state.update(cx, |state, cx| {
            state.note_monitor(&id, placement, false, cx);
        });
    }

    fn page_for(route: Route, window: &mut Window, cx: &mut Context<Self>) -> Page {
        match route {
            // A new list each visit: what changed in a project shows in its
            // "last modified" on the way back.
            Route::Dashboard => Page::Dashboard(cx.new(|cx| DashboardView::new(window, cx))),
            Route::Workspace(collection) => Page::Workspace(cx.new(|cx| WorkspaceView::new(collection, window, cx))),
        }
    }

    /// F14.1 / F15.2: a new theme or language shows at once. A new language
    /// builds the page again, so the strings its fields were made with are
    /// the new language's too.
    fn follow_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prefs = AppStore::global(cx).state().read(cx).prefs().clone();
        if prefs.theme != self.theme {
            self.theme = prefs.theme;
            apply_theme(prefs.theme, window, cx);
        }
        if prefs.language != self.language {
            self.language = prefs.language.clone();
            crate::i18n::apply(prefs.language.as_deref());
            let route = AppStore::global(cx).state().read(cx).route().clone();
            self.page = Self::page_for(route, window, cx);
            window.refresh();
            cx.notify();
        }
    }

    /// Swaps the page when the route names another one.
    fn follow_route(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let route = AppStore::global(cx).state().read(cx).route().clone();
        let current = match (&self.page, &route) {
            (Page::Dashboard(_), Route::Dashboard) => true,
            (Page::Workspace(view), Route::Workspace(collection)) => view.read(cx).collection_id() == &collection.id,
            _ => false,
        };
        if !current {
            self.page = Self::page_for(route, window, cx);
            self.set_title(window, cx);
            cx.notify();
        }
    }

    /// F1.5: the project list is the app's name; a workspace names the
    /// window itself, after its selected API.
    fn set_title(&self, window: &mut Window, cx: &App) {
        if *AppStore::global(cx).state().read(cx).route() == Route::Dashboard {
            window.set_window_title(APP_NAME);
        }
    }

    fn render_title(&self, cx: &mut Context<Self>) -> AnyElement {
        let pins = match &self.page {
            Page::Workspace(view) => Some(view.read(cx).pins().clone()),
            Page::Dashboard(_) => None,
        };
        match AppStore::global(cx).state().read(cx).route() {
            Route::Dashboard => div().text_sm().child(APP_NAME).into_any_element(),
            // F1.9: Home > project, then the pins (F12.2). The selected
            // request is a sibling of this row, so it can sit in the middle.
            Route::Workspace(collection) => h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_1()
                .child(
                    Button::new("title-back")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ArrowLeft)
                        .label(t!("workspace.back").to_string())
                        .on_click(|_, _, cx| {
                            let state = AppStore::global(cx).state().clone();
                            state.update(cx, |state, cx| state.go_home(cx));
                        }),
                )
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child("/"))
                .child(
                    // A long name gives way to the pins beside it.
                    div()
                        .flex_none()
                        .max_w(px(240.))
                        .truncate()
                        .mr_3()
                        .text_sm()
                        .font_medium()
                        .child(collection.name.clone()),
                )
                .children(pins)
                .into_any_element(),
        }
    }
}

/// The UI's text size, the rem everything is measured in. The kit's 16 px
/// is a document's; a tool's panels read better denser, as Zed's do.
const UI_FONT_SIZE: f32 = 14.;
/// The editors' face, the same on every platform (`this_build::add_fonts`).
const MONO_FONT: &str = "JetBrains Mono";

fn apply_theme(mode: ThemeMode, window: &mut Window, cx: &mut App) {
    match mode {
        ThemeMode::System => Theme::sync_system_appearance(Some(window), cx),
        ThemeMode::Light => Theme::change(ComponentThemeMode::Light, Some(window), cx),
        ThemeMode::Dark => Theme::change(ComponentThemeMode::Dark, Some(window), cx),
    }
    // A theme change may set its own size and faces; these stay.
    // Ghost hover paints `accent`, and the dark variant then halves its alpha.
    // The stock accent sits on the page, so the wash disappears. A list row
    // paints the same token solid, so the step stops at neutral-200 / 600:
    // neutral-300 / 500 read as a bar. `Theme::update` copies the color onto
    // `tokens.accent`, which is what the button reads.
    let dark = Theme::global(cx).mode.is_dark();
    let accent = try_parse_color(if dark { "neutral-600" } else { "neutral-200" }).expect("neutral scale");
    Theme::update(cx, |theme| {
        theme.font_size = px(UI_FONT_SIZE);
        theme.mono_font_family = MONO_FONT.into();
        theme.accent = accent;
        // The dark theme's backdrop hardly darkens a page that is dark already;
        // a dialog should stand off it as it does in the light one.
        if dark {
            theme.overlay = hsla(0., 0., 0., 0.55);
        }
    });
    window.refresh();
}

impl CyberApi {
    /// The settings menu (F14): in a project, its variables (M8) first; on
    /// the project list, where this build backs up, backing up and
    /// restoring (F13.3); where it signs in, the account last (§9).
    fn render_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = match &self.page {
            Page::Workspace(view) => Some(view.read(cx).state().downgrade()),
            Page::Dashboard(_) => None,
        };
        let backs_up = AppStore::global(cx).backups().is_some();
        let signs_in = AppStore::global(cx).signs_in();
        Button::new("title-settings")
            .ghost()
            .small()
            .icon(IconName::Settings)
            .tooltip(t!("menu.settings").to_string())
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu: PopupMenu, _, cx| {
                let menu =
                    match workspace.clone() {
                        Some(workspace) => [
                            (VariableCategory::Environment, t!("variables.env_title"), Icons::Globe),
                            (VariableCategory::Custom, t!("variables.custom_title"), Icons::Braces),
                            (
                                VariableCategory::GlobalRequestHeader,
                                t!("variables.headers_title"),
                                Icons::Heading,
                            ),
                        ]
                        .into_iter()
                        .fold(menu, |menu, (category, label, icon)| {
                            let workspace = workspace.clone();
                            menu.item(PopupMenuItem::new(label.to_string()).icon(icon).on_click(
                                move |_, window, cx| {
                                    if let Some(workspace) = workspace.upgrade() {
                                        open_variables_dialog(&workspace, category, window, cx);
                                    }
                                },
                            ))
                        }),
                        None if backs_up => menu
                            .item(
                                PopupMenuItem::new(t!("backup.back_up").to_string())
                                    .icon(Icons::DatabaseBackup)
                                    .on_click(|_, _, cx| {
                                        let state = AppStore::global(cx).state().clone();
                                        state.update(cx, |state, cx| state.back_up(cx));
                                    }),
                            )
                            .item(
                                PopupMenuItem::new(t!("backup.restore").to_string())
                                    .icon(Icons::ArchiveRestore)
                                    .on_click(|_, window, cx| pick_backup(window, cx)),
                            ),
                        None => menu.item(PopupMenuItem::new(t!("menu.no_project").to_string()).disabled(true)),
                    };
                let menu = menu
                    .separator()
                    .item(
                        PopupMenuItem::new(t!("cookies.title").to_string())
                            .icon(Icons::Cookie)
                            .on_click(|_, window, cx| open_cookies_dialog(window, cx)),
                    )
                    .item(
                        PopupMenuItem::new(t!("settings.title").to_string())
                            .icon(Icons::Settings)
                            .on_click(|_, window, cx| open_settings_dialog(window, cx)),
                    )
                    .item(
                        PopupMenuItem::new(t!("shortcuts.title").to_string())
                            .icon(Icons::Keyboard)
                            .on_click(|_, window, cx| open_shortcuts_dialog(window, cx)),
                    );
                if signs_in { account_items(menu, cx) } else { menu }
            })
    }
}

/// Who is signed in, and what they do about it (§9).
fn account_items(menu: PopupMenu, cx: &App) -> PopupMenu {
    let me = AppStore::global(cx).state().read(cx).me().map(|me| me.account.clone());
    let admin = me.as_ref().is_some_and(|me| me.admin);
    let menu = match &me {
        Some(me) => menu.separator().item(
            PopupMenuItem::new(t!("account.signed_in_as", name = me.label()).to_string())
                .icon(Icons::CircleUser)
                .disabled(true),
        ),
        None => menu.separator(),
    };
    let menu = menu.item(
        PopupMenuItem::new(t!("account.teams").to_string())
            .icon(Icons::Users)
            .on_click(|_, window, cx| open_teams_dialog(window, cx)),
    );
    let menu = if admin {
        menu.item(
            PopupMenuItem::new(t!("account.accounts").to_string())
                .icon(Icons::UserCog)
                .on_click(|_, window, cx| open_accounts_dialog(window, cx)),
        )
    } else {
        menu
    };
    menu.item(
        PopupMenuItem::new(t!("account.change_password").to_string())
            .icon(Icons::KeyRound)
            .on_click(|_, window, cx| open_password_dialog(window, cx)),
    )
    .item(
        PopupMenuItem::new(t!("account.sign_out").to_string())
            .icon(Icons::LogOut)
            .on_click(|_, _, cx| {
                let state = AppStore::global(cx).state().clone();
                state.update(cx, |state, cx| state.sign_out(cx));
            }),
    )
}

/// F13.2: the backup to restore, picked and then confirmed.
fn pick_backup(window: &mut Window, cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: None,
    });
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let _ = cx.update(|window, cx| confirm_restore(path, window, cx));
        })
        .detach();
}

fn confirm_restore(path: PathBuf, window: &mut Window, cx: &mut App) {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let path = path.clone();
        alert
            .title(t!("backup.restore_title", name = name).to_string())
            .description(t!("backup.restore_description").to_string())
            .confirm()
            .ok_text(t!("backup.restore_ok").to_string())
            .ok_variant(ButtonVariant::Danger)
            .cancel_text(t!("common.cancel").to_string())
            .on_ok(move |_, _, cx| {
                let state = AppStore::global(cx).state().clone();
                state.update(cx, |state, cx| state.restore(path.clone(), cx));
                true
            })
    });
}

impl Render for CyberApi {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match &self.page {
            Page::Dashboard(view) => view.clone().into_any_element(),
            Page::Workspace(view) => view.clone().into_any_element(),
        };
        // Equal sides keep the selected request in the middle of the bar.
        // On the project list the middle is empty, so the name stays left
        // and the menu stays right.
        let selection = match &self.page {
            Page::Workspace(view) => Some(view.read(cx).selection_title().clone()),
            Page::Dashboard(_) => None,
        };
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .child(self.render_title(cx)),
                        )
                        .children(selection)
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .justify_end()
                                .pr_2()
                                .child(self.render_menu(cx)),
                        ),
                ),
            )
            .child(div().flex_1().min_h_0().child(page))
    }
}
