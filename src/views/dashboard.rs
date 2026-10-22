//! The project list (M2): find, order, open, create, edit and delete
//! projects. The data and the changes belong to `DashboardState`; this draws
//! them and forwards what the reader asks for.

use crate::i18n;
use crate::states::{AppStore, DashboardEvent, DashboardState, Load, ServerStatus};
use crate::views::group_dialog::open_group_dialog;
use crate::views::marked::marked;
use crate::views::name_dialog::ok_button;
use crate::views::open_access_dialog;
use crate::views::pick::pick;
use crate::views::sync_dialog::open_conflict_dialog;
use cyberapi_core::access::CollectionRole;
use cyberapi_core::collection::{SortOrder, group_names, group_of, grouped};
use cyberapi_core::format::{Ago, ago};
use cyberapi_core::model::Collection;
use cyberapi_core::{CollectionId, TeamId, Timestamp};
use cyberapi_store::{CollectionEdit, NewCollection};
use gpui_kit::assets::IconName as Icons;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::empty::{
    Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Colorize as _, Icon, IconName, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, Hsla,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div, px,
};
use rust_i18n::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

gpui_kit::actions!(dashboard, [FocusSearch]);

pub(crate) const KEY_CONTEXT: &str = "Dashboard";

/// `secondary-f` puts the caret in the search box, as zedis's list does.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("secondary-f", FocusSearch, Some(KEY_CONTEXT))]);
}

/// The same with Cmd, for a page in a Mac's browser, where `secondary` is
/// Ctrl (`workspace::bind_mac_page_keys`).
pub fn bind_mac_page_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("cmd-f", FocusSearch, Some(KEY_CONTEXT))]);
}

/// Every order the sort menu offers, in its order.
const SORT_ORDERS: [SortOrder; 5] = [
    SortOrder::LastModified,
    SortOrder::NameAsc,
    SortOrder::NameDesc,
    SortOrder::NewestFirst,
    SortOrder::OldestFirst,
];

fn sort_label(order: SortOrder) -> SharedString {
    match order {
        SortOrder::LastModified => t!("dashboard.sort_last_modified"),
        SortOrder::NameAsc => t!("dashboard.sort_name_asc"),
        SortOrder::NameDesc => t!("dashboard.sort_name_desc"),
        SortOrder::OldestFirst => t!("dashboard.sort_oldest_first"),
        SortOrder::NewestFirst => t!("dashboard.sort_newest_first"),
    }
    .to_string()
    .into()
}

pub struct DashboardView {
    state: Entity<DashboardState>,
    search: Entity<InputState>,
    /// The page's keys reach it from wherever focus is on it.
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl DashboardView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = AppStore::global(cx).clone();
        let scope = store.draft_scope(cx);
        let server = store.server().cloned();
        let state = cx.new(|cx| DashboardState::new(store.storage().clone(), scope, server, cx));
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("dashboard.search").to_string())
                .clean_on_escape()
        });
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            // The sort order lives in the application's settings, and the
            // drafts are the signed-in reader's.
            cx.observe(store.state(), |this, _, cx| {
                let scope = AppStore::global(cx).draft_scope(cx);
                this.state.update(cx, |state, cx| state.set_scope(scope, cx));
                cx.notify();
            }),
            cx.subscribe_in(&state, window, |_, this_state, event, window, cx| match event {
                DashboardEvent::Failed(error) => {
                    window.push_notification(Notification::error(i18n::store_error(error)), cx);
                }
                DashboardEvent::Deleted(id) => {
                    let app = AppStore::global(cx).state().clone();
                    app.update(cx, |app, cx| app.forget_project(id, cx));
                }
                // A new project is made to be worked in: it opens.
                DashboardEvent::Created(collection) => {
                    let app = AppStore::global(cx).state().clone();
                    app.update(cx, |app, cx| app.open_collection(collection.clone(), cx));
                }
                DashboardEvent::Unsynced => {
                    window.push_notification(Notification::info(t!("sync.first_change").to_string()), cx);
                }
                DashboardEvent::NotKept => {
                    window.push_notification(Notification::warning(t!("sync.not_kept").to_string()), cx);
                }
                DashboardEvent::Synced(collection) => {
                    window.push_notification(
                        Notification::success(t!("sync.synced_project", name = collection.name).to_string()),
                        cx,
                    );
                }
                DashboardEvent::Conflict(collection) => {
                    let (state, theirs) = (this_state.clone(), this_state.clone());
                    let (id, their_id) = (collection.id.clone(), collection.id.clone());
                    open_conflict_dialog(
                        &collection.name,
                        window,
                        cx,
                        move |cx| state.update(cx, |state, cx| state.sync(&id, true, cx)),
                        move |cx| theirs.update(cx, |state, cx| state.discard(&their_id, cx)),
                    );
                }
            }),
            cx.subscribe(&search, |this, _, event, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.open_first_found(cx),
                _ => {}
            }),
        ];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            state,
            search,
            focus,
            _subscriptions: subscriptions,
        }
    }

    /// Enter in the search: the first project it found, as the list
    /// shows them.
    fn open_first_found(&mut self, cx: &mut Context<Self>) {
        let keyword = self.search.read(cx).value().to_string();
        if keyword.trim().is_empty() {
            return;
        }
        let app = AppStore::global(cx).state().clone();
        let prefs = app.read(cx).prefs();
        let first = grouped(
            self.state.read(cx).collections(),
            &prefs.project_groups,
            &keyword,
            prefs.collection_sort,
        )
        .into_iter()
        .find_map(|group| group.collections.first().map(|collection| (*collection).clone()));
        if let Some(collection) = first {
            app.update(cx, |app, cx| app.open_collection(collection, cx));
        }
    }

    /// The form for a new project, or for changing `existing` (F2.2 / F2.3).
    fn open_editor(&mut self, existing: Option<Collection>, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("dashboard.name_placeholder").to_string())
                .default_value(existing.as_ref().map(|c| c.name.clone()).unwrap_or_default())
        });
        let description = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("common.optional").to_string())
                .default_value(existing.as_ref().map(|c| c.description.clone()).unwrap_or_default())
        });
        let (title, ok_label) = match existing {
            Some(_) => (t!("dashboard.edit_title"), t!("common.save")),
            None => (t!("dashboard.create_title"), t!("common.create")),
        };
        let (title, ok_label): (SharedString, SharedString) = (title.to_string().into(), ok_label.to_string().into());
        let state = self.state.clone();
        let name_missing = Rc::new(Cell::new(false));
        let focus_once = Cell::new(true);
        // The browser build's: the team a new project goes in, the reader's
        // own space unless chosen.
        let creating = existing.is_none();
        let team: Rc<RefCell<Option<TeamId>>> = Rc::default();

        window.open_dialog(cx, move |dialog, window, cx| {
            if focus_once.replace(false) {
                name.focus_handle(cx).focus(window, cx);
            }
            let on_ok = {
                let (name, description, state, existing, name_missing, team) = (
                    name.clone(),
                    description.clone(),
                    state.clone(),
                    existing.clone(),
                    name_missing.clone(),
                    team.clone(),
                );
                move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                    let name_value = name.read(cx).value().trim().to_string();
                    if name_value.is_empty() {
                        name_missing.set(true);
                        window.refresh();
                        return false;
                    }
                    let description_value = description.read(cx).value().trim().to_string();
                    state.update(cx, |state, cx| match &existing {
                        Some(collection) => state.update(
                            collection.id.clone(),
                            CollectionEdit {
                                name: name_value,
                                description: description_value,
                            },
                            cx,
                        ),
                        None => state.create(
                            NewCollection {
                                name: name_value,
                                description: description_value,
                                owner: team.borrow().clone(),
                            },
                            cx,
                        ),
                    });
                    true
                }
            };
            dialog
                .title(title.clone())
                .w(px(440.))
                .child(
                    v_flex()
                        .gap_4()
                        .child(field(
                            t!("common.name").to_string(),
                            Input::new(&name),
                            name_missing.get().then(|| t!("errors.name_required").to_string()),
                            cx,
                        ))
                        .child(field(
                            t!("common.description").to_string(),
                            Input::new(&description),
                            None,
                            cx,
                        ))
                        .when_some(creating.then(|| team_field(&team, cx)).flatten(), |this, field| {
                            this.child(field)
                        }),
                )
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                        .child(ok_button("collection-dialog-ok", ok_label.clone(), on_ok.clone())),
                )
                .on_ok(on_ok)
        });
    }

    /// F2.4: name the project and what goes with it before deleting.
    /// The reader's group for `collection`.
    fn open_group(&mut self, collection: Collection, window: &mut Window, cx: &mut Context<Self>) {
        let app = AppStore::global(cx).state().clone();
        let groups = &app.read(cx).prefs().project_groups;
        let current = group_of(groups, &collection.id).unwrap_or_default().to_string();
        let others: Vec<String> = group_names(groups)
            .into_iter()
            .filter(|name| *name != current)
            .collect();
        let id = collection.id.clone();
        open_group_dialog(
            window,
            cx,
            t!("dashboard.group_title", name = collection.name).to_string().into(),
            t!("dashboard.group_placeholder").to_string().into(),
            current,
            others,
            move |name, cx| app.update(cx, |app, cx| app.set_project_group(id.clone(), &name, cx)),
        );
    }

    /// A group of the reader's renamed — into another, merging them — or
    /// undone.
    fn open_rename_group(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let app = AppStore::global(cx).state().clone();
        let others: Vec<String> = group_names(&app.read(cx).prefs().project_groups)
            .into_iter()
            .filter(|other| *other != name)
            .collect();
        open_group_dialog(
            window,
            cx,
            t!("dashboard.rename_group_title", name = name).to_string().into(),
            t!("dashboard.rename_group_placeholder").to_string().into(),
            name.clone(),
            others,
            move |to, cx| app.update(cx, |app, cx| app.rename_group(&name, &to, cx)),
        );
    }

    /// Server-sync §3.3: the project's unsynced changes given up.
    fn confirm_discard(&mut self, collection: Collection, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (state, id) = (state.clone(), collection.id.clone());
            alert
                .title(t!("sync.discard_title").to_string())
                .description(t!("sync.discard_description").to_string())
                .confirm()
                .ok_text(t!("sync.discard").to_string())
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(t!("common.cancel").to_string())
                .on_ok(move |_, _, cx| {
                    state.update(cx, |state, cx| state.discard(&id, cx));
                    true
                })
        });
    }

    fn confirm_delete(&mut self, collection: Collection, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.clone();
        // How many APIs go, when that was read and there are some.
        let description = match self.state.read(cx).api_count(&collection.id) {
            Some(0) | None => t!("dashboard.delete_description").to_string(),
            Some(1) => t!("dashboard.delete_description_count", apis = t!("workspace.api_one")).to_string(),
            Some(count) => t!(
                "dashboard.delete_description_count",
                apis = t!("workspace.api_other", count = count)
            )
            .to_string(),
        };
        // Server-sync §2: the changes not synced go with it.
        let description = match self.state.read(cx).unsynced(&collection.id) {
            Some(_) => format!("{description}\n{}", t!("sync.delete_loses_draft")),
            None => description,
        };
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (state, id) = (state.clone(), collection.id.clone());
            alert
                .title(t!("dashboard.delete_title", name = collection.name).to_string())
                .description(description.clone())
                .confirm()
                .ok_text(t!("common.delete").to_string())
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(t!("common.cancel").to_string())
                .on_ok(move |_, _, cx| {
                    state.update(cx, |state, cx| state.delete(id.clone(), cx));
                    true
                })
        });
    }

    fn render_toolbar(
        &self,
        sort: SortOrder,
        total: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let shortcut = Kbd::binding_for_action(&FocusSearch, Some(KEY_CONTEXT), window);
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        h_flex()
            .px_6()
            .py_4()
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(div().text_lg().font_semibold().child(t!("dashboard.title").to_string()))
            .child(
                div()
                    .px_1p5()
                    .rounded_full()
                    .bg(cx.theme().muted)
                    .text_xs()
                    .text_color(muted)
                    .child(total.to_string()),
            )
            .child(
                Input::new(&self.search)
                    .w(px(280.))
                    .small()
                    .prefix(Icon::new(IconName::Search).text_color(muted))
                    .when_some(shortcut, |input, shortcut| input.suffix(shortcut)),
            )
            .child(
                Button::new("dashboard-sort")
                    .outline()
                    .small()
                    .icon(IconName::SortAscending)
                    .label(sort_label(sort))
                    .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                        SORT_ORDERS.into_iter().fold(menu, |menu, order| {
                            menu.item(PopupMenuItem::new(sort_label(order)).checked(order == sort).on_click(
                                move |_, _, cx| {
                                    let state = AppStore::global(cx).state().clone();
                                    state.update(cx, |state, cx| state.set_collection_sort(order, cx));
                                },
                            ))
                        })
                    }),
            )
            .child(div().flex_1())
            .child(
                Button::new("dashboard-new")
                    .primary()
                    .small()
                    .icon(IconName::Plus)
                    .label(t!("dashboard.new").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.open_editor(None, window, cx))),
            )
    }

    fn render_card(
        &self,
        collection: &Collection,
        keyword: &str,
        now: Timestamp,
        view: WeakEntity<Self>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let api_count = self.state.read(cx).api_count(&collection.id);
        let unsynced = self.state.read(cx).unsynced(&collection.id);
        let syncing = self.state.read(cx).syncing(&collection.id);
        let sync = unsynced.map(|behind| sync_chip(collection.id.clone(), behind, syncing, &self.state));
        let theme = cx.theme();
        let (ring, muted) = (theme.ring, theme.muted_foreground);
        let surface = card_surface(cx);
        let hover_bg = theme.list_hover;
        let border = theme.border;
        let key = collection.id.to_string();
        let opened = collection.clone();
        let actions = CardActions {
            collection: collection.clone(),
            view,
            // Changing the project is its managers' (§9.2); the desktop's
            // only reader manages everything. Its group is the reader's own.
            manages: collection.role.can_manage(),
            // Only a server's project is given to others.
            shares: collection.owner.is_some() && AppStore::global(cx).directory(cx).is_some(),
            unsynced: unsynced.is_some(),
        };
        // The same menu on a right click, as the tree's rows have theirs.
        let context_actions = actions.clone();
        let place = place_of(collection, cx);
        // Every card of a row as tall as the row, the time at the bottom.
        v_flex()
            .id(SharedString::from(format!("collection-{key}")))
            .justify_between()
            .gap_3()
            .p_4()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(border)
            .bg(surface)
            .cursor_pointer()
            .hover(move |style| style.border_color(ring).bg(hover_bg))
            .on_click(move |_, _, cx| {
                let state = AppStore::global(cx).state().clone();
                state.update(cx, |state, cx| state.open_collection(opened.clone(), cx));
            })
            .context_menu(move |menu, _, _| card_menu(menu, &context_actions))
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(project_mark(&collection.name, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_base()
                                            .font_semibold()
                                            .child(marked(&collection.name, keyword, cx)),
                                    )
                                    .child(
                                        // The menu's own clicks must not open the project.
                                        div()
                                            .id(SharedString::from(format!("collection-actions-{key}")))
                                            .on_click(|_, _, cx| cx.stop_propagation())
                                            .child(
                                                Button::new(SharedString::from(format!("collection-more-{key}")))
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(IconName::Ellipsis)
                                                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                                                        card_menu(menu, &actions)
                                                    }),
                                            ),
                                    ),
                            )
                            .child(description_line(&key, &collection.description, muted)),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .pl(px(44.))
                    .text_xs()
                    .text_color(muted)
                    .child(div().flex_1().child(updated_label(collection.updated_at, now)))
                    .children(sync)
                    .when_some(place, |this, place| this.child(div().truncate().child(place)))
                    .when_some(api_count, |this, count| {
                        this.child(
                            div()
                                .flex_none()
                                .child(t!("dashboard.api_count", count = count).to_string()),
                        )
                    }),
            )
    }

    fn render_body(&self, sort: SortOrder, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let keyword = self.search.read(cx).value().to_string();
        let state = self.state.read(cx);
        let load = state.load().clone();
        let total = state.collections().len();
        let app = AppStore::global(cx).state().read(cx);
        // The reader's groups, each with whether it is folded; a search
        // shows what it found, folded or not.
        let searching = !keyword.trim().is_empty();
        let groups: Vec<(Option<String>, Vec<Collection>, bool)> =
            grouped(state.collections(), &app.prefs().project_groups, &keyword, sort)
                .into_iter()
                .map(|group| {
                    let folded = !searching && app.is_group_folded(group.name.as_deref());
                    (group.name, group.collections.into_iter().cloned().collect(), folded)
                })
                .collect();

        match load {
            Load::Loading if total == 0 => h_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .gap_2()
                .text_color(cx.theme().muted_foreground)
                .child(Spinner::new())
                .child(t!("dashboard.loading").to_string())
                .into_any_element(),
            Load::Failed(error) if total == 0 => notice(
                IconName::CircleX,
                t!("dashboard.load_failed").to_string(),
                i18n::store_error(&error),
                Some(
                    Button::new("dashboard-retry")
                        .outline()
                        .label(t!("common.retry").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.state.update(cx, |state, cx| state.reload(cx)))),
                ),
            ),
            _ if total == 0 => notice(
                IconName::FolderOpen,
                t!("dashboard.empty_title").to_string(),
                t!("dashboard.empty_description").to_string(),
                Some(
                    Button::new("dashboard-empty-new")
                        .primary()
                        .icon(IconName::Plus)
                        .label(t!("dashboard.new").to_string())
                        .on_click(cx.listener(|this, _, window, cx| this.open_editor(None, window, cx))),
                ),
            ),
            _ if groups.is_empty() => notice(
                IconName::Search,
                t!("dashboard.no_match", keyword = keyword.trim()).to_string(),
                String::new(),
                None,
            ),
            _ => {
                let now = Timestamp::new(chrono::Local::now().fixed_offset());
                // Two columns at the narrowest window, more as it widens.
                let width = window.viewport_size().width;
                let columns = if width < px(1200.) {
                    2
                } else if width < px(1600.) {
                    3
                } else {
                    4
                };
                // No groups yet: the plain grid, without a heading over it.
                let body = if groups.iter().all(|(name, _, _)| name.is_none()) {
                    let collections: Vec<Collection> = groups.into_iter().flat_map(|(_, list, _)| list).collect();
                    self.render_grid(&collections, &keyword, columns, now, cx)
                } else {
                    let mut sections = Vec::new();
                    for (index, (name, collections, folded)) in groups.into_iter().enumerate() {
                        let header = self.render_group_header(index, name, collections.len(), folded, cx);
                        let grid = (!folded).then(|| self.render_grid(&collections, &keyword, columns, now, cx));
                        sections.push(v_flex().gap_3().child(header).children(grid));
                    }
                    v_flex().gap_6().children(sections).into_any_element()
                };
                div()
                    .id("dashboard-grid")
                    .flex_1()
                    .overflow_y_scrollbar()
                    .px_6()
                    .pt_4()
                    .pb_6()
                    .child(body)
                    .into_any_element()
            }
        }
    }

    fn render_grid(
        &self,
        collections: &[Collection],
        keyword: &str,
        columns: u16,
        now: Timestamp,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity().downgrade();
        let cards: Vec<AnyElement> = collections
            .iter()
            .map(|collection| {
                self.render_card(collection, keyword, now, view.clone(), cx)
                    .into_any_element()
            })
            .collect();
        div()
            .grid()
            .grid_cols(columns)
            .gap_4()
            .children(cards)
            .into_any_element()
    }

    /// A group's heading: a click folds or unfolds it; a named group can be
    /// renamed.
    fn render_group_header(
        &self,
        index: usize,
        name: Option<String>,
        count: usize,
        folded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity().downgrade();
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let label = name.clone().unwrap_or_else(|| t!("dashboard.ungrouped").to_string());
        let toggled = name.clone();
        h_flex()
            .id(("dashboard-group", index))
            .gap_2()
            .items_center()
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                let app = AppStore::global(cx).state().clone();
                app.update(cx, |app, cx| app.toggle_group(toggled.as_deref(), cx));
            })
            .child(
                Icon::new(if folded {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .xsmall()
                .text_color(muted),
            )
            .child(div().text_sm().font_semibold().child(label))
            .child(
                div()
                    .px_1p5()
                    .rounded_full()
                    .bg(theme.muted)
                    .text_xs()
                    .text_color(muted)
                    .child(count.to_string()),
            )
            .when_some(name, |this, name| {
                this.child(
                    // The menu's own clicks must not fold the group.
                    div()
                        .id(("dashboard-group-actions", index))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            Button::new(("dashboard-group-more", index))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Ellipsis)
                                .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                                    let (name, view) = (name.clone(), view.clone());
                                    menu.item(
                                        PopupMenuItem::new(t!("dashboard.rename_group").to_string())
                                            .icon(Icons::Pencil)
                                            .on_click(move |_, window, cx| {
                                                let _ = view.update(cx, |this, cx| {
                                                    this.open_rename_group(name.clone(), window, cx)
                                                });
                                            }),
                                    )
                                }),
                        ),
                )
            })
            .into_any_element()
    }
}

/// When the project last changed, as long ago as it was.
fn updated_label(at: Timestamp, now: Timestamp) -> String {
    let ago_text = |time: String| t!("dashboard.updated_ago", time = time).to_string();
    match ago(at, now) {
        Ago::JustNow => ago_text(t!("time.just_now").to_string()),
        Ago::Minutes(n) => ago_text(t!("time.minutes_ago", n = n).to_string()),
        Ago::Hours(n) => ago_text(t!("time.hours_ago", n = n).to_string()),
        Ago::Days(n) => ago_text(t!("time.days_ago", n = n).to_string()),
        Ago::On(date) => t!("dashboard.updated", time = date).to_string(),
    }
}

/// What a card's menu acts on.
#[derive(Clone)]
struct CardActions {
    collection: Collection,
    view: WeakEntity<DashboardView>,
    manages: bool,
    shares: bool,
    /// The project has changes the server has not had.
    unsynced: bool,
}

/// A card's word that its changes are not on the server yet — and that the
/// server moved on too, when it did — and the button that syncs them
/// (`docs/server-sync.md` §3.2, §3.3).
fn sync_chip(id: CollectionId, behind: bool, syncing: bool, state: &Entity<DashboardState>) -> impl IntoElement {
    let key = id.to_string();
    let (label, hint) = if behind {
        (t!("sync.unsynced_behind"), t!("sync.unsynced_behind_hint"))
    } else {
        (t!("sync.unsynced"), t!("sync.unsynced_hint"))
    };
    let hint = hint.to_string();
    let state = state.clone();
    h_flex()
        .id(SharedString::from(format!("collection-sync-{key}")))
        .flex_none()
        .items_center()
        .gap_1()
        // Its clicks must not open the project.
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .id(SharedString::from(format!("collection-unsynced-{key}")))
                .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                .child(Tag::warning().small().rounded_full().child(label.to_string())),
        )
        .child(
            Button::new(SharedString::from(format!("collection-push-{key}")))
                .ghost()
                .xsmall()
                .icon(Icons::CloudUpload)
                .tooltip(t!("sync.sync").to_string())
                .loading(syncing)
                .on_click(move |_, _, cx| state.update(cx, |state, cx| state.sync(&id, false, cx))),
        )
}

/// A card's menu: changing the project for its managers (F2.3, §9.2), its
/// group for anyone — the group is the reader's own — and deleting it last
/// (F2.4).
fn card_menu(menu: PopupMenu, actions: &CardActions) -> PopupMenu {
    let CardActions {
        collection,
        view,
        manages,
        shares,
        unsynced,
    } = actions;
    let mut menu = menu;
    if *unsynced {
        let (synced, sync_view) = (collection.id.clone(), view.clone());
        let (discarded, discard_view) = (collection.clone(), view.clone());
        menu = menu
            .item(
                PopupMenuItem::new(t!("sync.sync").to_string())
                    .icon(Icons::CloudUpload)
                    .on_click(move |_, _, cx| {
                        let _ = sync_view.update(cx, |this, cx| {
                            this.state.update(cx, |state, cx| state.sync(&synced, false, cx))
                        });
                    }),
            )
            .item(
                PopupMenuItem::new(t!("sync.discard").to_string())
                    .icon(Icons::Undo2)
                    .on_click(move |_, window, cx| {
                        let _ = discard_view.update(cx, |this, cx| this.confirm_discard(discarded.clone(), window, cx));
                    }),
            )
            .separator();
    }
    if *manages {
        let (edited, edit_view) = (collection.clone(), view.clone());
        menu = menu.item(
            PopupMenuItem::new(t!("common.edit").to_string())
                .icon(Icons::Pencil)
                .on_click(move |_, window, cx| {
                    let _ = edit_view.update(cx, |this, cx| this.open_editor(Some(edited.clone()), window, cx));
                }),
        );
        if *shares {
            let (shared, share_view) = (collection.clone(), view.clone());
            menu = menu.item(
                PopupMenuItem::new(t!("access.menu").to_string())
                    .icon(Icons::Users)
                    .on_click(move |_, window, cx| {
                        let reload = share_view.clone();
                        open_access_dialog(
                            shared.clone(),
                            move |cx| {
                                let _ =
                                    reload.update(cx, |this, cx| this.state.update(cx, |state, cx| state.reload(cx)));
                            },
                            window,
                            cx,
                        );
                    }),
            );
        }
    }
    let (grouped, group_view) = (collection.clone(), view.clone());
    menu = menu.item(
        PopupMenuItem::new(t!("dashboard.group").to_string())
            .icon(Icons::FolderInput)
            .on_click(move |_, window, cx| {
                let _ = group_view.update(cx, |this, cx| this.open_group(grouped.clone(), window, cx));
            }),
    );
    if *manages {
        let (deleted, delete_view) = (collection.clone(), view.clone());
        menu = menu.separator().item(
            PopupMenuItem::new(t!("common.delete").to_string())
                .icon(Icons::Trash)
                .on_click(move |_, window, cx| {
                    let _ = delete_view.update(cx, |this, cx| this.confirm_delete(deleted.clone(), window, cx));
                }),
        );
    }
    menu
}

/// A labelled form row, with the reason it was refused under it.
fn field(label: String, input: impl IntoElement, error: Option<String>, cx: &App) -> impl IntoElement {
    v_flex()
        .gap_1p5()
        .child(div().text_sm().font_medium().child(label))
        .child(input)
        .when_some(error, |this, error| {
            this.child(div().text_xs().text_color(cx.theme().danger).child(error))
        })
}

/// A centred state of the whole list: empty, failed, nothing found.
fn notice(icon: IconName, title: String, description: String, action: Option<Button>) -> AnyElement {
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
                                .child(Icon::new(icon)),
                        )
                        .title(EmptyTitle::new().child(title))
                        .when(!description.is_empty(), |header| {
                            header.description(EmptyDescription::new().child(description))
                        }),
                )
                .when_some(action, |empty, action| empty.content(EmptyContent::new().child(action))),
        )
        .into_any_element()
}

impl Render for DashboardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A closed dialog leaves nothing focused; the page takes it back, so
        // its keys keep working.
        if window.focused(cx).is_none() {
            self.focus.focus(window, cx);
        }
        let sort = AppStore::global(cx).state().read(cx).prefs().collection_sort;
        let total = self.state.read(cx).collections().len();
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.search.focus_handle(cx).focus(window, cx);
            }))
            .child(self.render_toolbar(sort, total, window, cx))
            .child(self.render_body(sort, window, cx))
    }
}

/// One step off the page, the same lift zedis gives a connection card.
/// `muted` is a whole panel and sat too far from the theme background.
fn card_surface(cx: &App) -> Hsla {
    let theme = cx.theme();
    if theme.is_dark() {
        theme.background.lighten(1.0)
    } else {
        theme.background.darken(0.02)
    }
}

/// A card's description on one line under the name, the whole of it on
/// hover. The line is there without one too, so every card is as tall.
fn description_line(key: &str, description: &str, muted: Hsla) -> impl IntoElement {
    let description = SharedString::from(description.trim().to_string());
    div()
        .id(SharedString::from(format!("collection-description-{key}")))
        .h(px(18.))
        .text_xs()
        .text_color(muted)
        .truncate()
        .child(description.clone())
        .when(!description.is_empty(), |this| {
            this.tooltip(move |window, cx| Tooltip::new(description.clone()).build(window, cx))
        })
}

/// A stable tinted initial, so neighbouring projects are not the same slab.
/// Hues only: the theme's primary is a grey in the dark theme, which read
/// as a project switched off.
fn project_mark(name: &str, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let palette = [theme.blue, theme.green, theme.yellow, theme.magenta, theme.cyan];
    let index = name.bytes().fold(0usize, |sum, byte| sum.wrapping_add(byte as usize));
    let color = palette[index % palette.len()];
    let letter = name.chars().next().unwrap_or('·').to_uppercase().to_string();
    h_flex()
        .size(px(32.))
        .flex_none()
        .rounded(theme.radius)
        .items_center()
        .justify_center()
        .bg(color.opacity(0.18))
        .text_sm()
        .font_semibold()
        .text_color(color)
        .child(letter)
}

/// A server project's: the team it belongs to, unless the reader's own
/// space, and the reader's role where it is less than managing.
fn place_of(collection: &Collection, cx: &App) -> Option<String> {
    collection.owner.as_ref()?;
    let me = AppStore::global(cx).state().read(cx).me()?;
    let team = me.teams.iter().find(|team| Some(&team.id) == collection.owner.as_ref());
    let team = match team {
        Some(team) if team.personal => None,
        Some(team) => Some(team.name.clone()),
        None => Some(t!("dashboard.shared").to_string()),
    };
    let role = (collection.role < CollectionRole::Manager).then(|| i18n::collection_role(collection.role));
    match (team, role) {
        (Some(team), Some(role)) => Some(format!("{team} · {role}")),
        (one, other) => one.or(other),
    }
}

/// The browser build's choice of team for a new project.
fn team_field(team: &Rc<RefCell<Option<TeamId>>>, cx: &App) -> Option<AnyElement> {
    let store = AppStore::global(cx);
    // A page's projects are its teams'; the desktop's, signed in, this
    // machine's (`None`) or a team's on its server (`docs/server-sync.md`
    // §2).
    let (teams, here) = if store.signs_in() {
        (store.state().read(cx).me()?.teams.clone(), None)
    } else {
        let server = store.server()?.read(cx);
        server.account()?;
        let ServerStatus::SignedIn(me) = server.status() else {
            return None;
        };
        (me.teams.clone(), Some(t!("dashboard.this_computer").to_string()))
    };
    let chosen = match &here {
        Some(_) => team.borrow().clone(),
        None => team
            .borrow()
            .clone()
            .or_else(|| teams.first().map(|first| first.id.clone())),
    };
    let label = teams
        .iter()
        .find(|candidate| Some(&candidate.id) == chosen.as_ref())
        .map(|candidate| candidate.name.clone())
        .or_else(|| here.clone())
        .unwrap_or_default();
    let options: Vec<(Option<TeamId>, String)> = here
        .map(|here| (None, here))
        .into_iter()
        .chain(teams.iter().map(|team| (Some(team.id.clone()), team.name.clone())))
        .collect();
    let team = team.clone();
    Some(
        v_flex()
            .gap_1p5()
            .child(div().text_sm().font_medium().child(t!("dashboard.team").to_string()))
            .child(h_flex().child(pick(
                "collection-team",
                label,
                options,
                Some(chosen),
                false,
                move |id, window, _| {
                    *team.borrow_mut() = id;
                    window.refresh();
                },
            )))
            .into_any_element(),
    )
}
