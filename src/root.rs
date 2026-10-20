//! The main window's content: the title bar and the page the route names.

use crate::APP_NAME;
use crate::states::{AppStore, Route};
use crate::views::{DashboardView, WorkspaceView, open_variables_dialog};
use cyberapi_core::model::VariableCategory;
use cyberapi_core::prefs::ThemeMode;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _, Theme, ThemeMode as ComponentThemeMode, TitleBar, h_flex,
    v_flex,
};
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div,
};
use rust_i18n::t;

enum Page {
    Dashboard(Entity<DashboardView>),
    Workspace(Entity<WorkspaceView>),
}

pub struct CyberApi {
    page: Page,
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
            cx.observe_in(&state, window, |this, _, window, cx| this.follow_route(window, cx)),
        ];
        let page = Self::page_for(state.read(cx).route().clone(), window, cx);
        let root = Self {
            page,
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
    /// The settings menu (F14): in a project, its variables (M8) first.
    fn render_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = match &self.page {
            Page::Workspace(view) => Some(view.read(cx).state().downgrade()),
            Page::Dashboard(_) => None,
        };
        Button::new("title-settings")
            .ghost()
            .small()
            .icon(IconName::Settings)
            .tooltip(t!("menu.settings").to_string())
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu: PopupMenu, _, _| {
                let Some(workspace) = workspace.clone() else {
                    return menu.item(PopupMenuItem::new(t!("menu.no_project").to_string()).disabled(true));
                };
                [
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
                })
            })
    }
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
