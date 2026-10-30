//! F2: the command palette, ⌘/Ctrl-K — one field to find an API in any
//! project (by name, folder or method, pinyin too), open a project, switch
//! the environment or run a command. ↑ and ↓ move, Enter picks, Esc closes.

use super::recent_dialog::open_recent_dialog;
use super::run_dialog::open_run_dialog;
use super::{method_color, method_label, new_item, open_import_dialog};
use crate::states::{AppStore, FoundApi, PaletteState, WorkspaceState, found_in};
use crate::views::{open_cookies_dialog, open_settings, open_shortcuts_dialog, open_variables_dialog};
use cyberapi_core::EnvironmentId;
use cyberapi_core::model::{Collection, VariableCategory};
use cyberapi_core::search::matches_text_or_pinyin;
use cyberapi_core::tree::ItemKind;
use gpui_kit::assets::IconName as Icons;
use gpui_kit::component::input::{Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    WeakEntity, Window, div, px,
};
use rust_i18n::t;
use std::rc::Rc;

/// The most a search shows.
const SHOWN: usize = 60;

gpui_kit::actions!(palette, [OpenPalette]);

/// The main window's root, where the palette's key is heard on every page.
pub const ROOT_CONTEXT: &str = "CyberApi";

/// ⌘/Ctrl-K, on every page of the main window.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([crate::views::secondary_binding(
        cx,
        "secondary-k",
        OpenPalette,
        Some(ROOT_CONTEXT),
    )]);
}

/// The same with Cmd, for a page in a Mac's browser.
pub fn bind_mac_page_keys(cx: &mut App) {
    cx.bind_keys([gpui_kit::KeyBinding::new("cmd-k", OpenPalette, Some(ROOT_CONTEXT))]);
}

/// What a command does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    NewApi,
    NewFolder,
    NewScratch,
    Import,
    SyncSource,
    Run,
    Recent,
    Variables,
    Projects,
    Settings,
    Cookies,
    Shortcuts,
}

impl Command {
    fn label(self) -> String {
        match self {
            Self::NewApi => t!("workspace.new_api"),
            Self::NewFolder => t!("workspace.new_folder"),
            Self::NewScratch => t!("workspace.new_scratch"),
            Self::Import => t!("import.title"),
            Self::SyncSource => t!("source.open"),
            Self::Run => t!("run.project"),
            Self::Recent => t!("recent.open"),
            Self::Variables => t!("palette.variables"),
            Self::Projects => t!("palette.projects"),
            Self::Settings => t!("palette.settings"),
            Self::Cookies => t!("palette.cookies"),
            Self::Shortcuts => t!("palette.shortcuts"),
        }
        .to_string()
    }

    fn icon(self) -> Icons {
        match self {
            Self::NewApi => Icons::FilePlus,
            Self::NewFolder => Icons::FolderPlus,
            Self::NewScratch => Icons::FilePlus,
            Self::Import => Icons::Import,
            Self::SyncSource => Icons::RefreshCw,
            Self::Run => Icons::Play,
            Self::Recent => Icons::Clock,
            Self::Variables => Icons::Globe,
            Self::Projects => Icons::FolderOpen,
            Self::Settings => Icons::Settings,
            Self::Cookies => Icons::Cookie,
            Self::Shortcuts => Icons::Keyboard,
        }
    }
}

/// A line the palette offers.
#[derive(Clone)]
enum Item {
    Api(FoundApi),
    Environment(Option<EnvironmentId>, String),
    Command(Command),
    Project(Collection),
}

/// Opens the palette over the page; `workspace` is the open project's.
pub fn open_palette(workspace: Option<Entity<WorkspaceState>>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| PaletteView::new(workspace, window, cx));
    let input = view.read(cx).input.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .w(px(640.))
            .close_button(false)
            .on_ok(|_, _, _| false)
            .child(view.clone())
    });
    input.focus_handle(cx).focus(window, cx);
}

struct PaletteView {
    workspace: Option<WeakEntity<WorkspaceState>>,
    /// The open project's id, to tell its APIs from the others'.
    open: Option<Collection>,
    index: Entity<PaletteState>,
    input: Entity<InputState>,
    /// What the search finds, in the order shown: worked out when the
    /// search or what it searches changes, not in every frame (PERF-11).
    items: Rc<[Item]>,
    chosen: usize,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl PaletteView {
    fn new(workspace: Option<Entity<WorkspaceState>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Read where the tree is, not copied.
        let open = workspace.as_ref().map(|workspace| {
            let workspace = workspace.read(cx);
            let collection = workspace.collection().clone();
            let apis = found_in(&collection, workspace.tree_data());
            (collection, apis)
        });
        let collection = open.as_ref().map(|(collection, _)| collection.clone());
        let index = cx.new(|cx| PaletteState::new(open, cx));
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("palette.placeholder").to_string()));
        let subscriptions = vec![
            cx.observe(&index, |this, _, cx| {
                this.find(cx);
                cx.notify();
            }),
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    // The other projects' APIs show only for a search.
                    if !this.input.read(cx).value().trim().is_empty() {
                        this.index.update(cx, |index, cx| index.read_others(cx));
                    }
                    this.find(cx);
                    this.chosen = 0;
                    this.scroll.scroll_to_item(0);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.pick(this.chosen, window, cx),
                _ => {}
            }),
        ];
        let mut view = Self {
            workspace: workspace.map(|workspace| workspace.downgrade()),
            open: collection,
            index,
            input,
            items: Rc::from([]),
            chosen: 0,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        view.find(cx);
        view
    }

    fn find(&mut self, cx: &App) {
        self.items = self.items(cx).into();
    }

    fn commands(&self, cx: &App) -> Vec<Command> {
        let mut commands = Vec::new();
        if let Some(workspace) = self.workspace.as_ref().and_then(WeakEntity::upgrade) {
            commands.extend([
                Command::NewApi,
                Command::NewFolder,
                Command::NewScratch,
                Command::Import,
            ]);
            // B7: an editor's, as importing is.
            if workspace.read(cx).editable() {
                commands.push(Command::SyncSource);
            }
            commands.extend([Command::Run, Command::Recent, Command::Variables, Command::Projects]);
        }
        commands.extend([Command::Settings, Command::Cookies, Command::Shortcuts]);
        commands
    }

    /// What the search finds, in the order shown.
    fn items(&self, cx: &App) -> Vec<Item> {
        let query = self.input.read(cx).value().trim().to_string();
        let found = |text: &str| query.is_empty() || matches_text_or_pinyin(text, &query);
        let index = self.index.read(cx);
        let open_id = self.open.as_ref().map(|collection| &collection.id);
        let api_text = |api: &FoundApi| format!("{} {} {}", api.folders, api.name, api.method.as_str());
        let (own, others): (Vec<&FoundApi>, Vec<&FoundApi>) =
            index.apis().iter().partition(|api| Some(&api.collection.id) == open_id);
        let mut items: Vec<Item> = Vec::new();
        let own_shown = if query.is_empty() { 12 } else { SHOWN };
        items.extend(
            own.into_iter()
                .filter(|api| found(&api_text(api)))
                .take(own_shown)
                .cloned()
                .map(Item::Api),
        );
        if let Some(workspace) = self.workspace.as_ref().and_then(WeakEntity::upgrade) {
            let workspace = workspace.read(cx);
            let none = t!("environments.none").to_string();
            let mut environments = vec![(None, none)];
            environments.extend(
                workspace
                    .named_environments()
                    .iter()
                    .map(|environment| (Some(environment.id.clone()), environment.name.clone())),
            );
            if environments.len() > 1 {
                items.extend(
                    environments
                        .into_iter()
                        .filter(|(_, name)| found(&format!("{} {name}", t!("palette.environment"))))
                        .map(|(id, name)| Item::Environment(id, name)),
                );
            }
        }
        items.extend(
            self.commands(cx)
                .into_iter()
                .filter(|command| found(&command.label()))
                .map(Item::Command),
        );
        items.extend(
            index
                .projects()
                .iter()
                .filter(|project| Some(&project.id) != open_id && found(&project.name))
                .cloned()
                .map(Item::Project),
        );
        if !query.is_empty() {
            items.extend(
                others
                    .into_iter()
                    .filter(|api| found(&api_text(api)))
                    .cloned()
                    .map(Item::Api),
            );
        }
        items.truncate(SHOWN);
        items
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = self.items.len();
        if count == 0 {
            return;
        }
        self.chosen = (self.chosen as isize + by).rem_euclid(count as isize) as usize;
        self.scroll.scroll_to_item(self.chosen);
        cx.notify();
    }

    fn pick(&mut self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(at).cloned() else {
            return;
        };
        window.close_dialog(cx);
        let workspace = self.workspace.clone();
        let state = workspace.as_ref().and_then(WeakEntity::upgrade);
        let app = AppStore::global(cx).state().clone();
        match item {
            Item::Api(found) => match &state {
                Some(state) if Some(&found.collection.id) == self.open.as_ref().map(|open| &open.id) => {
                    state.update(cx, |state, cx| state.select(&found.api, cx));
                }
                _ => app.update(cx, |app, cx| app.open_at(found.collection, found.api, cx)),
            },
            Item::Project(collection) => app.update(cx, |app, cx| app.open_collection(collection, cx)),
            Item::Environment(id, _) => {
                if let Some(state) = state {
                    state.update(cx, |state, cx| state.set_environment(id, cx));
                }
            }
            Item::Command(command) => match (command, workspace, state) {
                (Command::NewApi, Some(weak), _) => new_item(&weak, None, ItemKind::Api, window, cx),
                (Command::NewFolder, Some(weak), _) => new_item(&weak, None, ItemKind::Folder, window, cx),
                (Command::NewScratch, Some(weak), _) => {
                    let _ = weak.update(cx, |state, cx| {
                        state.new_scratch(t!("workspace.scratch_name").into(), cx)
                    });
                }
                (Command::Import, Some(weak), _) => open_import_dialog(weak, None, window, cx),
                (Command::SyncSource, _, Some(state)) => super::source_dialog::open_source_dialog(state, window, cx),
                (Command::Run, _, Some(state)) => open_run_dialog(state, None, window, cx),
                (Command::Recent, _, Some(state)) => open_recent_dialog(state, window, cx),
                (Command::Variables, _, Some(state)) => {
                    open_variables_dialog(&state, VariableCategory::Environment, window, cx)
                }
                (Command::Projects, ..) => app.update(cx, |app, cx| app.go_home(cx)),
                (Command::Settings, ..) => open_settings(window, cx),
                (Command::Cookies, ..) => open_cookies_dialog(window, cx),
                (Command::Shortcuts, ..) => open_shortcuts_dialog(window, cx),
                _ => {}
            },
        }
    }

    fn row(&self, at: usize, item: &Item, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, accent) = (theme.muted_foreground, theme.accent);
        let (lead, title, note): (AnyElement, String, String) = match item {
            Item::Api(found) => {
                let mut note = found.folders.clone();
                if Some(&found.collection.id) != self.open.as_ref().map(|open| &open.id) {
                    note = if note.is_empty() {
                        found.collection.name.clone()
                    } else {
                        format!("{} · {note}", found.collection.name)
                    };
                }
                (
                    div()
                        .w(px(40.))
                        .text_xs()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(method_color(found.method, cx))
                        .child(method_label(found.method))
                        .into_any_element(),
                    found.name.clone(),
                    note,
                )
            }
            Item::Environment(_, name) => (
                Icon::new(Icons::Globe).small().text_color(muted).into_any_element(),
                name.clone(),
                t!("palette.environment").to_string(),
            ),
            Item::Command(command) => (
                Icon::new(command.icon()).small().text_color(muted).into_any_element(),
                command.label(),
                t!("palette.command").to_string(),
            ),
            Item::Project(collection) => (
                Icon::new(Icons::Folder).small().text_color(muted).into_any_element(),
                collection.name.clone(),
                t!("palette.project").to_string(),
            ),
        };
        h_flex()
            .id(("palette-row", at))
            .px_2()
            .py_1p5()
            .gap_2()
            .items_center()
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            .when(at == self.chosen, |row| row.bg(accent))
            .hover(move |style| style.bg(accent))
            .child(div().w(px(40.)).flex_none().flex().justify_center().child(lead))
            .child(div().flex_1().min_w_0().truncate().child(title))
            .child(
                div()
                    .flex_none()
                    .max_w(px(240.))
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(note),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.pick(at, window, cx)))
            .into_any_element()
    }
}

impl Render for PaletteView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let items = self.items.clone();
        if self.chosen >= items.len() {
            self.chosen = 0;
        }
        let rows: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .map(|(at, item)| self.row(at, item, cx))
            .collect();
        let reading = self.index.read(cx).reading();
        v_flex()
            .gap_2()
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.step(-1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.step(1, cx);
                cx.stop_propagation();
            }))
            .child(Input::new(&self.input).prefix(Icon::new(Icons::Search).small().text_color(muted)))
            .child(
                div()
                    .id("palette-items")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(v_flex().gap_0p5().children(rows)),
            )
            .child(
                h_flex()
                    .text_xs()
                    .text_color(muted)
                    .child(div().flex_1().child(SharedString::from(t!("palette.keys").to_string())))
                    .when(reading, |this| this.child(t!("palette.reading").to_string())),
            )
    }
}
