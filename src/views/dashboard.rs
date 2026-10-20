//! The project list (M2): find, order, open, create, edit and delete
//! projects. The data and the changes belong to `DashboardState`; this draws
//! them and forwards what the reader asks for.

use crate::i18n;
use crate::states::{AppStore, DashboardEvent, DashboardState, Load};
use crate::views::name_dialog::ok_button;
use crate::views::open_access_dialog;
use crate::views::pick::pick;
use cyberapi_core::access::CollectionRole;
use cyberapi_core::collection::{SortOrder, filter_and_sort};
use cyberapi_core::format::short_time;
use cyberapi_core::model::Collection;
use cyberapi_core::{TeamId, Timestamp};
use cyberapi_store::{CollectionEdit, NewCollection};
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::empty::{
    Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity,
    Window, div, px,
};
use rust_i18n::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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
    _subscriptions: Vec<Subscription>,
}

impl DashboardView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = AppStore::global(cx).clone();
        let state = cx.new(|cx| DashboardState::new(store.storage().clone(), cx));
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("dashboard.search").to_string())
                .clean_on_escape()
        });
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            // The sort order lives in the application's settings.
            cx.observe(store.state(), |_, _, cx| cx.notify()),
            cx.subscribe_in(&state, window, |_, _, event, window, cx| match event {
                DashboardEvent::Failed(error) => {
                    window.push_notification(Notification::error(i18n::store_error(error)), cx);
                }
            }),
            cx.subscribe(&search, |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        Self {
            state,
            search,
            _subscriptions: subscriptions,
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
    fn confirm_delete(&mut self, collection: Collection, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (state, id) = (state.clone(), collection.id.clone());
            alert
                .title(t!("dashboard.delete_title", name = collection.name).to_string())
                .description(t!("dashboard.delete_description").to_string())
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

    fn render_toolbar(&self, sort: SortOrder, total: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        h_flex()
            .px_6()
            .py_4()
            .gap_3()
            .items_center()
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
                    .w(px(240.))
                    .small()
                    .prefix(Icon::new(IconName::Search).text_color(muted)),
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
        now: Timestamp,
        view: WeakEntity<Self>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (border, ring, muted) = (theme.border, theme.ring, theme.muted_foreground);
        let key = collection.id.to_string();
        let opened = collection.clone();
        let (edited, deleted, shared) = (collection.clone(), collection.clone(), collection.clone());
        let (edit_view, delete_view, share_view) = (view.clone(), view.clone(), view);
        // Changing the project is its managers' (§9.2); the desktop's only
        // reader manages everything.
        let manages = collection.role.can_manage();
        let shares = AppStore::global(cx).directory().is_some();
        let place = place_of(collection, cx);
        v_flex()
            .id(SharedString::from(format!("collection-{key}")))
            .gap_2()
            .p_4()
            .min_h(px(116.))
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(border)
            .bg(theme.background)
            .cursor_pointer()
            .hover(move |style| style.border_color(ring))
            .on_click(move |_, _, cx| {
                let state = AppStore::global(cx).state().clone();
                state.update(cx, |state, cx| state.open_collection(opened.clone(), cx));
            })
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_semibold()
                            .child(collection.name.clone()),
                    )
                    .when(manages, |this| {
                        this.child(
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
                                            let (edited, deleted) = (edited.clone(), deleted.clone());
                                            let (edit_view, delete_view) = (edit_view.clone(), delete_view.clone());
                                            let menu =
                                                menu.item(PopupMenuItem::new(t!("common.edit").to_string()).on_click(
                                                    move |_, window, cx| {
                                                        let _ = edit_view.update(cx, |this, cx| {
                                                            this.open_editor(Some(edited.clone()), window, cx)
                                                        });
                                                    },
                                                ));
                                            let menu = if shares {
                                                let (shared, share_view) = (shared.clone(), share_view.clone());
                                                menu.item(PopupMenuItem::new(t!("access.menu").to_string()).on_click(
                                                    move |_, window, cx| {
                                                        let reload = share_view.clone();
                                                        open_access_dialog(
                                                            shared.clone(),
                                                            move |cx| {
                                                                let _ = reload.update(cx, |this, cx| {
                                                                    this.state.update(cx, |state, cx| state.reload(cx))
                                                                });
                                                            },
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                ))
                                            } else {
                                                menu
                                            };
                                            menu.separator().item(
                                                PopupMenuItem::new(t!("common.delete").to_string()).on_click(
                                                    move |_, window, cx| {
                                                        let _ = delete_view.update(cx, |this, cx| {
                                                            this.confirm_delete(deleted.clone(), window, cx)
                                                        });
                                                    },
                                                ),
                                            )
                                        }),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(muted)
                    .line_clamp(2)
                    .child(collection.description.clone()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .text_color(muted)
                    .child(
                        div()
                            .flex_1()
                            .child(t!("dashboard.updated", time = short_time(collection.updated_at, now)).to_string()),
                    )
                    .when_some(place, |this, place| this.child(div().truncate().child(place))),
            )
    }

    fn render_body(&self, sort: SortOrder, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let keyword = self.search.read(cx).value().to_string();
        let state = self.state.read(cx);
        let load = state.load().clone();
        let total = state.collections().len();
        let shown: Vec<Collection> = filter_and_sort(state.collections(), &keyword, sort)
            .into_iter()
            .cloned()
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
            _ if shown.is_empty() => notice(
                IconName::Search,
                t!("dashboard.no_match", keyword = keyword.trim()).to_string(),
                String::new(),
                None,
            ),
            _ => {
                let now = Timestamp::new(chrono::Local::now().fixed_offset());
                let view = cx.entity().downgrade();
                // Two columns at the narrowest window, more as it widens.
                let width = window.viewport_size().width;
                let columns = if width < px(1200.) {
                    2
                } else if width < px(1600.) {
                    3
                } else {
                    4
                };
                let cards: Vec<AnyElement> = shown
                    .iter()
                    .map(|collection| self.render_card(collection, now, view.clone(), cx).into_any_element())
                    .collect();
                div()
                    .id("dashboard-grid")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_6()
                    .pb_6()
                    .child(div().grid().grid_cols(columns).gap_4().children(cards))
                    .into_any_element()
            }
        }
    }
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
        let sort = AppStore::global(cx).state().read(cx).prefs().collection_sort;
        let total = self.state.read(cx).collections().len();
        v_flex()
            .size_full()
            .child(self.render_toolbar(sort, total, cx))
            .child(self.render_body(sort, window, cx))
    }
}

/// The browser build's: the team a project belongs to, unless the reader's
/// own space, and the reader's role where it is less than managing.
fn place_of(collection: &Collection, cx: &App) -> Option<String> {
    AppStore::global(cx).directory()?;
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
    AppStore::global(cx).directory()?;
    let teams = AppStore::global(cx).state().read(cx).me()?.teams.clone();
    let chosen = team
        .borrow()
        .clone()
        .or_else(|| teams.first().map(|first| first.id.clone()));
    let label = teams
        .iter()
        .find(|candidate| Some(&candidate.id) == chosen.as_ref())
        .map(|candidate| candidate.name.clone())
        .unwrap_or_default();
    let options = teams.iter().map(|team| (team.id.clone(), team.name.clone())).collect();
    let team = team.clone();
    Some(
        v_flex()
            .gap_1p5()
            .child(div().text_sm().font_medium().child(t!("dashboard.team").to_string()))
            .child(h_flex().child(pick(
                "collection-team",
                label,
                options,
                chosen,
                false,
                move |id, window, _| {
                    *team.borrow_mut() = Some(id);
                    window.refresh();
                },
            )))
            .into_any_element(),
    )
}
