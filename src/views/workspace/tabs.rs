//! F1: the open APIs as tabs above the request editor: 36 px tall (aligned
//! with the tree's search header), showing the method and name on a single
//! line — plus the enclosing folder when two open tabs share a name — a dot
//! while its request is on its way, and the one in the editor drawn as the
//! page. A click opens one; the close button (on the open tab and the one
//! under the pointer) or a middle click closes it, and its menu closes the
//! others or all. More than the row holds scroll sideways, the open one
//! brought into view, and a button at the row's end lists them all (UI-1).

use super::{method_color, method_label};
use crate::states::WorkspaceState;
use cyberapi_core::ItemId;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, StyledExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, Context, Entity, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px, rems,
};
use rust_i18n::t;

pub struct TabsBar {
    state: Entity<WorkspaceState>,
    scroll: ScrollHandle,
    /// The tab open when last drawn: another one is scrolled into view.
    shown: Option<ItemId>,
    /// UI-1: some tabs are past the row's end, as last laid out.
    overflowing: bool,
    _subscription: Subscription,
}

impl TabsBar {
    pub fn new(state: Entity<WorkspaceState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            scroll: ScrollHandle::new(),
            shown: None,
            overflowing: false,
            _subscription: subscription,
        }
    }

    /// Whether the tabs fit is known once they are laid out: checked after
    /// each frame via a weak handle so closing the project does not keep its
    /// state alive till the next frame.
    fn watch_overflow(&self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = view.update(cx, |this, cx| {
                let overflowing = this.scroll.max_offset().x > px(0.5);
                if overflowing != this.overflowing {
                    this.overflowing = overflowing;
                    cx.notify();
                }
            });
        });
    }

    /// UI-1: every open tab, for when some are past the row's end: a choice
    /// opens it and brings it into view; closing the others or all of them
    /// is there too.
    fn render_all(&self) -> impl IntoElement + use<> {
        let state = self.state.downgrade();
        let scroll = self.scroll.clone();
        Button::new("api-tabs-all")
            .ghost()
            .xsmall()
            .flex_none()
            .mx_1()
            .icon(IconName::ChevronDown)
            .tooltip(t!("workspace.all_tabs").to_string())
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
                let Some(workspace) = state.upgrade() else {
                    return menu;
                };
                let workspace = workspace.read(cx);
                let opened = workspace.opened().cloned();
                let tabs: Vec<_> = workspace
                    .tabs()
                    .into_iter()
                    .map(|tab| (tab.id, tab.method, tab.name, tab.folders, tab.path))
                    .collect();
                let shared_names: std::collections::HashSet<String> = {
                    let mut seen = std::collections::HashSet::new();
                    let mut dup = std::collections::HashSet::new();
                    for (_, _, name, ..) in &tabs {
                        if !seen.insert(name.as_str()) {
                            dup.insert(name.clone());
                        }
                    }
                    dup
                };
                let opened_in_tabs = opened.clone().filter(|opened| tabs.iter().any(|(id, ..)| id == opened));
                let menu =
                    tabs.into_iter()
                        .enumerate()
                        .fold(menu, |menu, (index, (id, method, name, folders, path))| {
                            let (state, scroll) = (state.clone(), scroll.clone());
                            let checked = opened.as_ref() == Some(&id);
                            let disambiguate = (shared_names.contains(&name)
                                && (!folders.is_empty() || !path.is_empty()))
                            .then(|| {
                                if !folders.is_empty() {
                                    folders.rsplit(" / ").next().unwrap_or(&folders).to_string()
                                } else {
                                    path
                                }
                            });
                            let item = PopupMenuItem::element(move |_, cx| {
                                let muted = cx.theme().muted_foreground;
                                h_flex()
                                    .gap_1p5()
                                    .items_center()
                                    .max_w(px(320.))
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_size(rems(0.7))
                                            .font_bold()
                                            .text_color(method_color(method, cx))
                                            .child(method_label(method)),
                                    )
                                    .child(div().min_w_0().truncate().child(name.clone()))
                                    .when_some(disambiguate.clone(), |row, folder| {
                                        row.child(
                                            div()
                                                .flex_none()
                                                .max_w(px(80.))
                                                .truncate()
                                                .text_size(rems(0.68))
                                                .text_color(muted)
                                                .child(folder),
                                        )
                                    })
                            });
                            menu.item(item.checked(checked).on_click(move |_, _, cx| {
                                scroll.scroll_to_item(index);
                                let _ = state.update(cx, |state, cx| state.select(&id, cx));
                            }))
                        });
                let (others, all) = (state.clone(), state.clone());
                let menu = menu.separator();
                let menu = match opened_in_tabs {
                    Some(opened) => menu.item(
                        PopupMenuItem::new(t!("workspace.close_other_tabs").to_string()).on_click(move |_, _, cx| {
                            let _ = others.update(cx, |state, cx| state.close_other_tabs(&opened, cx));
                        }),
                    ),
                    None => menu,
                };
                menu.item(
                    PopupMenuItem::new(t!("workspace.close_all_tabs").to_string()).on_click(move |_, _, cx| {
                        let _ = all.update(cx, |state, cx| state.close_all_tabs(cx));
                    }),
                )
            })
    }
}

impl Render for TabsBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.watch_overflow(window, cx);
        let all = self.overflowing.then(|| self.render_all());
        let state = self.state.read(cx);
        let opened = state.opened().cloned();
        let editable = state.editable();
        let tabs: Vec<_> = state
            .tabs()
            .into_iter()
            .map(|tab| {
                let sending = state.tab_sending(&tab.id, cx);
                (
                    tab.id,
                    tab.method,
                    tab.name,
                    tab.folders,
                    tab.path,
                    sending,
                    tab.scratch,
                )
            })
            .collect();
        if opened != self.shown {
            if let Some(at) = tabs.iter().position(|(id, ..)| Some(id) == opened.as_ref()) {
                self.scroll.scroll_to_item(at);
            }
            self.shown = opened.clone();
        }
        let shared_names: std::collections::HashSet<String> = {
            let mut seen = std::collections::HashSet::new();
            let mut dup = std::collections::HashSet::new();
            for (_, _, name, ..) in &tabs {
                if !seen.insert(name.as_str()) {
                    dup.insert(name.clone());
                }
            }
            dup
        };
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        // UI-1: between two tabs neither of which is open (a pin is), the
        // theme's border sits on the same grey as the tabs in the dark
        // theme: a stronger line keeps each one apart.
        let divider = muted.opacity(0.3);
        let row = tabs
            .into_iter()
            .enumerate()
            .map(|(index, (id, method, name, folders, path, sending, scratch))| {
                let active = opened.as_ref() == Some(&id);
                let group = SharedString::from(format!("api-tab-{index}"));
                let (open_id, close_id, middle_id) = (id.clone(), id.clone(), id.clone());
                let menu_state = self.state.downgrade();
                let disambiguate =
                    (shared_names.contains(&name) && (!folders.is_empty() || !path.is_empty())).then(|| {
                        if !folders.is_empty() {
                            folders.rsplit(" / ").next().unwrap_or(&folders).to_string()
                        } else {
                            path.clone()
                        }
                    });
                let tip = if !folders.is_empty() {
                    format!("{folders} / {name}")
                } else if !path.is_empty() {
                    format!("{name} · {path}")
                } else {
                    name.clone()
                };
                h_flex()
                    .id(("api-tab", index))
                    .group(group.clone())
                    .relative()
                    .flex_none()
                    .max_w(px(240.))
                    .h_full()
                    .px_4()
                    .gap_1p5()
                    .items_center()
                    .border_r_1()
                    .border_color(divider)
                    .text_xs()
                    .cursor_pointer()
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .map(|tab| {
                        if active {
                            tab.bg(theme.background).text_color(theme.foreground)
                        } else {
                            tab.bg(theme.secondary)
                                .text_color(theme.secondary_foreground)
                                .hover(|style| style.bg(theme.secondary_hover))
                        }
                    })
                    .when(active, |tab| {
                        tab.child(div().absolute().top_0().left_0().right_0().h(px(2.)).bg(theme.primary))
                    })
                    .child(
                        div()
                            .flex_none()
                            .text_size(rems(0.7))
                            .font_bold()
                            .text_color(method_color(method, cx))
                            .child(method_label(method)),
                    )
                    .child(div().min_w_0().truncate().child(name.clone()))
                    .when_some(disambiguate, |tab, folder| {
                        tab.child(
                            div()
                                .flex_none()
                                .max_w(px(72.))
                                .truncate()
                                .text_size(rems(0.68))
                                .text_color(muted)
                                .child(folder),
                        )
                    })
                    .when(sending, |tab| {
                        tab.child(div().flex_none().size(px(6.)).rounded_full().bg(theme.primary))
                    })
                    .child(
                        div()
                            .absolute()
                            .top(px(2.))
                            .right(px(2.))
                            .invisible()
                            .group_hover(group, |style| style.visible())
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                Button::new(("close-api-tab", index))
                                    .ghost()
                                    .xsmall()
                                    .compact()
                                    .icon(IconName::Close)
                                    .tooltip(t!("workspace.close_tab").to_string())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.state.update(cx, |state, cx| state.close_tab(&close_id, cx));
                                    })),
                            ),
                    )
                    .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                        this.state.update(cx, |state, cx| {
                            if event.click_count() == 2 {
                                state.close_tab(&open_id, cx);
                            } else {
                                state.select(&open_id, cx);
                            }
                        });
                    }))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.state.update(cx, |state, cx| state.close_tab(&middle_id, cx));
                        }),
                    )
                    .context_menu(move |menu, _, _| {
                        let (close, others, all) = (menu_state.clone(), menu_state.clone(), menu_state.clone());
                        let (close_id, others_id) = (id.clone(), id.clone());
                        let menu = if scratch && editable {
                            let (save, save_id, save_name) = (menu_state.clone(), id.clone(), name.clone());
                            menu.item(PopupMenuItem::new(t!("workspace.save_scratch").to_string()).on_click(
                                move |_, window, cx| {
                                    super::move_dialog::open_save_scratch_dialog(
                                        save.clone(),
                                        save_id.clone(),
                                        save_name.clone(),
                                        window,
                                        cx,
                                    );
                                },
                            ))
                        } else {
                            menu
                        };
                        // F7: on the desktop, in a window of its own.
                        let menu = if crate::this_build::opens_windows() {
                            let (apart, apart_id) = (menu_state.clone(), id.clone());
                            menu.item(
                                PopupMenuItem::new(t!("apart.open").to_string()).on_click(move |_, _, cx| {
                                    let _ = apart.update(cx, |state, cx| state.part(&apart_id, cx));
                                }),
                            )
                            .separator()
                        } else {
                            menu
                        };
                        menu.item(PopupMenuItem::new(t!("workspace.close_tab").to_string()).on_click(
                            move |_, _, cx| {
                                let _ = close.update(cx, |state, cx| state.close_tab(&close_id, cx));
                            },
                        ))
                        .item(
                            PopupMenuItem::new(t!("workspace.close_other_tabs").to_string()).on_click(
                                move |_, _, cx| {
                                    let _ = others.update(cx, |state, cx| state.close_other_tabs(&others_id, cx));
                                },
                            ),
                        )
                        .item(
                            PopupMenuItem::new(t!("workspace.close_all_tabs").to_string()).on_click(move |_, _, cx| {
                                let _ = all.update(cx, |state, cx| state.close_all_tabs(cx));
                            }),
                        )
                    })
            });
        h_flex()
            .flex_none()
            .w_full()
            .h(px(36.))
            .items_center()
            .border_b_1()
            .border_color(border)
            .bg(theme.secondary)
            .text_color(muted)
            .child(
                h_flex()
                    .id("api-tabs")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .overflow_x_scroll()
                    .track_scroll(&self.scroll)
                    .children(row.collect::<Vec<_>>()),
            )
            .when_some(all, |bar, all| {
                bar.child(div().flex_none().w(px(1.)).h_full().bg(divider)).child(all)
            })
    }
}
