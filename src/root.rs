//! The main window's content: the title bar and the page the route names.

use crate::APP_NAME;
use crate::i18n;
use crate::states::{AppEvent, AppStore, Route};
use crate::views::{
    DashboardView, WorkspaceView, open_accounts_dialog, open_cookies_dialog, open_password_dialog,
    open_settings_dialog, open_teams_dialog, open_variables_dialog,
};
use cyberapi_core::model::VariableCategory;
use cyberapi_core::prefs::ThemeMode;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _, Theme, ThemeMode as ComponentThemeMode, TitleBar,
    WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, PathPromptOptions,
    Render, Styled as _, Subscription, Window, div,
};
use rust_i18n::t;
use std::path::PathBuf;

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
        };
        root.set_title(window, cx);
        root
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

    /// F1.5: the window is named after what it shows.
    fn set_title(&self, window: &mut Window, cx: &App) {
        let title = match AppStore::global(cx).state().read(cx).route() {
            Route::Dashboard => APP_NAME.to_string(),
            Route::Workspace(collection) => format!("{APP_NAME} - {}", collection.name),
        };
        window.set_window_title(&title);
    }

    fn render_title(&self, cx: &mut Context<Self>) -> AnyElement {
        let pins = match &self.page {
            Page::Workspace(view) => Some(view.read(cx).pins().clone()),
            Page::Dashboard(_) => None,
        };
        match AppStore::global(cx).state().read(cx).route() {
            Route::Dashboard => div().text_sm().child(APP_NAME).into_any_element(),
            // F1.9: Home > project, then the pins (F12.2).
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
                    div()
                        .flex_none()
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

fn apply_theme(mode: ThemeMode, window: &mut Window, cx: &mut App) {
    match mode {
        ThemeMode::System => Theme::sync_system_appearance(Some(window), cx),
        ThemeMode::Light => Theme::change(ComponentThemeMode::Light, Some(window), cx),
        ThemeMode::Dark => Theme::change(ComponentThemeMode::Dark, Some(window), cx),
    }
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
                let menu = match workspace.clone() {
                    Some(workspace) => [
                        (VariableCategory::Environment, t!("variables.env_title")),
                        (VariableCategory::Custom, t!("variables.custom_title")),
                        (VariableCategory::GlobalRequestHeader, t!("variables.headers_title")),
                    ]
                    .into_iter()
                    .fold(menu, |menu, (category, label)| {
                        let workspace = workspace.clone();
                        menu.item(PopupMenuItem::new(label.to_string()).on_click(move |_, window, cx| {
                            if let Some(workspace) = workspace.upgrade() {
                                open_variables_dialog(&workspace, category, window, cx);
                            }
                        }))
                    }),
                    None if backs_up => menu
                        .item(
                            PopupMenuItem::new(t!("backup.back_up").to_string()).on_click(|_, _, cx| {
                                let state = AppStore::global(cx).state().clone();
                                state.update(cx, |state, cx| state.back_up(cx));
                            }),
                        )
                        .item(
                            PopupMenuItem::new(t!("backup.restore").to_string())
                                .on_click(|_, window, cx| pick_backup(window, cx)),
                        ),
                    None => menu.item(PopupMenuItem::new(t!("menu.no_project").to_string()).disabled(true)),
                };
                let menu = menu
                    .separator()
                    .item(
                        PopupMenuItem::new(t!("cookies.title").to_string())
                            .on_click(|_, window, cx| open_cookies_dialog(window, cx)),
                    )
                    .item(
                        PopupMenuItem::new(t!("settings.title").to_string())
                            .on_click(|_, window, cx| open_settings_dialog(window, cx)),
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
        Some(me) => menu
            .separator()
            .item(PopupMenuItem::new(t!("account.signed_in_as", name = me.label()).to_string()).disabled(true)),
        None => menu.separator(),
    };
    let menu = menu.item(
        PopupMenuItem::new(t!("account.teams").to_string()).on_click(|_, window, cx| open_teams_dialog(window, cx)),
    );
    let menu = if admin {
        menu.item(
            PopupMenuItem::new(t!("account.accounts").to_string())
                .on_click(|_, window, cx| open_accounts_dialog(window, cx)),
        )
    } else {
        menu
    };
    menu.item(
        PopupMenuItem::new(t!("account.change_password").to_string())
            .on_click(|_, window, cx| open_password_dialog(window, cx)),
    )
    .item(
        PopupMenuItem::new(t!("account.sign_out").to_string()).on_click(|_, _, cx| {
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
                        .pr_2()
                        .items_center()
                        .justify_between()
                        .child(self.render_title(cx))
                        .child(self.render_menu(cx)),
                ),
            )
            .child(div().flex_1().min_h_0().child(page))
    }
}
