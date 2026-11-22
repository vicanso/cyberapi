//! F1: the open APIs as tabs above the request editor: 36 px tall (aligned
//! with the tree's search header), showing the method and name on a single
//! line — plus the enclosing folder when two open tabs share a name — a dot
//! while its request is on its way, an inline close button, and the active
//! tab styled seamlessly into the editor pane (modeled on zedis's workspace
//! tab strip). Clicking a tab selects it in place without reshuffling the
//! bar; a middle click closes it, and its context menu closes it, the
//! others, those to the right, or all. More than the row holds scroll
//! sideways with trackpad or vertical mouse wheel, the open one brought into
//! view, and a button at the row's end lists them all (UI-1).

use super::{method_color, method_label};
use crate::states::WorkspaceState;
use cyberapi_core::ItemId;
use gpui_kit::base::InteractiveElementExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableMask;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Colorize as _, IconName, Sizable as _, StyledExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, App, Context, Entity, Hsla, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Pixels,
    Render, ScrollHandle, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px, rems,
};
use rust_i18n::t;

/// Height aligned with the left tree panel's search header (`36 px`).
const TAB_BAR_HEIGHT: Pixels = px(36.);

/// Width cap so a long API name truncates cleanly without pushing sibling tabs
/// off screen (matching zedis's compact pill cap).
const TAB_TITLE_MAX_WIDTH: Pixels = px(160.);

/// Thickness of the active tab's top brand-accent indicator bar.
const TAB_ACTIVE_ACCENT_HEIGHT: Pixels = px(2.);

/// Recessed trough background for the tab bar so inactive tabs read clearly
/// distinct from the active tab's `background` fill in both light and dark
/// themes (modeled on zedis `workspace_tab_bar_background`).
fn tab_bar_background(cx: &App) -> Hsla {
    let theme = cx.theme();
    let bg = theme.background;
    let tab_bar = theme.tab_bar;
    if (tab_bar.l - bg.l).abs() >= 0.03 {
        tab_bar
    } else if theme.is_dark() {
        if bg.l > 0.08 { bg.darken(0.22) } else { bg.lighten(0.6) }
    } else {
        bg.darken(0.04)
    }
}

pub struct TabsBar {
    state: Entity<WorkspaceState>,
    scroll: ScrollHandle,
    /// The tab open and its index when last drawn: a selection or reorder
    /// change scrolls it back into view.
    shown: Option<(ItemId, usize)>,
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
    /// opens it in place and brings it into view; closing the others, those
    /// to the right, or all of them is there too.
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
                let tab_count = tabs.len();
                let opened_pos = opened
                    .as_ref()
                    .and_then(|opened| tabs.iter().position(|(id, ..)| id == opened));
                let opened_in_tabs = opened_pos.and_then(|pos| tabs.get(pos).map(|(id, ..)| (id.clone(), pos)));
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
                                let _ = state.update(cx, |state, cx| state.select_tab(&id, cx));
                            }))
                        });
                let (others, right, all) = (state.clone(), state.clone(), state.clone());
                let menu = menu.separator();
                let menu = match opened_in_tabs {
                    Some((opened, pos)) => {
                        let others_id = opened.clone();
                        let menu = menu.item(
                            PopupMenuItem::new(t!("workspace.close_other_tabs").to_string()).on_click(
                                move |_, _, cx| {
                                    let _ = others.update(cx, |state, cx| state.close_other_tabs(&others_id, cx));
                                },
                            ),
                        );
                        if pos + 1 < tab_count {
                            menu.item(
                                PopupMenuItem::new(t!("workspace.close_right_tabs").to_string()).on_click(
                                    move |_, _, cx| {
                                        let _ = right.update(cx, |state, cx| state.close_right_tabs(&opened, cx));
                                    },
                                ),
                            )
                        } else {
                            menu
                        }
                    }
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
        let current_pos = opened.as_ref().and_then(|id| {
            tabs.iter()
                .position(|(tab_id, ..)| tab_id == id)
                .map(|at| (id.clone(), at))
        });
        if current_pos != self.shown {
            if let Some((_, at)) = &current_pos {
                self.scroll.scroll_to_item(*at);
            }
            self.shown = current_pos;
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
        let tab_bar_bg = tab_bar_background(cx);
        let theme = cx.theme();
        let (border, muted, active_bg, foreground, primary, hover_bg) = (
            theme.border,
            theme.muted_foreground,
            theme.background,
            theme.foreground,
            theme.primary,
            theme.list_hover,
        );
        let tab_count = tabs.len();
        let row = tabs
            .into_iter()
            .enumerate()
            .map(|(index, (id, method, name, folders, path, sending, scratch))| {
                let active = opened.as_ref() == Some(&id);
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
                let label_color = if active { foreground } else { muted.opacity(0.88) };
                let method_tint = if active {
                    method_color(method, cx)
                } else {
                    method_color(method, cx).opacity(0.72)
                };
                h_flex()
                    .id(("api-tab", index))
                    .relative()
                    .flex_none()
                    .h_full()
                    .pl_2p5()
                    .pr_1()
                    .gap_1p5()
                    .items_center()
                    .border_r_1()
                    .border_color(border)
                    .text_xs()
                    .cursor_pointer()
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    // Active tab merges into the editor surface below without a
                    // bottom hairline; inactive tabs keep a 1px boundary on the trough.
                    .when(active, |tab| {
                        tab.bg(active_bg).child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .h(TAB_ACTIVE_ACCENT_HEIGHT)
                                .bg(primary),
                        )
                    })
                    .when(!active, |tab| {
                        tab.border_b_1()
                            .border_color(border)
                            .hover(move |style| style.bg(hover_bg))
                    })
                    .child(
                        div()
                            .flex_none()
                            .text_size(rems(0.7))
                            .font_bold()
                            .text_color(method_tint)
                            .child(method_label(method)),
                    )
                    .child(
                        div()
                            .max_w(TAB_TITLE_MAX_WIDTH)
                            .min_w_0()
                            .truncate()
                            .text_color(label_color)
                            .when(active, |this| this.font_semibold())
                            .child(name.clone()),
                    )
                    .when_some(disambiguate, |tab, folder| {
                        tab.child(
                            div()
                                .flex_none()
                                .max_w(px(64.))
                                .truncate()
                                .text_size(rems(0.68))
                                .text_color(muted.opacity(0.75))
                                .child(folder),
                        )
                    })
                    .when(sending, |tab| {
                        tab.child(div().flex_none().size(px(6.)).rounded_full().bg(primary))
                    })
                    .child(
                        div()
                            .flex_none()
                            .when(!active, |this| this.opacity(0.45).hover(|style| style.opacity(1.0)))
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
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.update(cx, |state, cx| state.select_tab(&open_id, cx));
                    }))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.state.update(cx, |state, cx| state.close_tab(&middle_id, cx));
                        }),
                    )
                    .context_menu(move |menu, _, _| {
                        let (close, others, right, all) = (
                            menu_state.clone(),
                            menu_state.clone(),
                            menu_state.clone(),
                            menu_state.clone(),
                        );
                        let (close_id, others_id, right_id) = (id.clone(), id.clone(), id.clone());
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
                        let menu = menu
                            .item(PopupMenuItem::new(t!("workspace.close_tab").to_string()).on_click(
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
                            );
                        let menu = if index + 1 < tab_count {
                            menu.item(
                                PopupMenuItem::new(t!("workspace.close_right_tabs").to_string()).on_click(
                                    move |_, _, cx| {
                                        let _ = right.update(cx, |state, cx| state.close_right_tabs(&right_id, cx));
                                    },
                                ),
                            )
                        } else {
                            menu
                        };
                        menu.item(PopupMenuItem::new(t!("workspace.close_all_tabs").to_string()).on_click(
                            move |_, _, cx| {
                                let _ = all.update(cx, |state, cx| state.close_all_tabs(cx));
                            },
                        ))
                    })
            });

        let pills = h_flex()
            .id("api-tabs")
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .overflow_x_scroll()
            .lock_scroll_axis()
            .track_scroll(&self.scroll)
            .children(row.collect::<Vec<_>>())
            // Continues the bottom 1px boundary across unused trough width.
            .child(div().flex_1().h_full().border_b_1().border_color(border));

        h_flex()
            .flex_none()
            .w_full()
            .h(TAB_BAR_HEIGHT)
            .items_center()
            .bg(tab_bar_bg)
            .text_color(muted)
            .child(
                h_flex()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .child(pills)
                    .child(ScrollableMask::vertical_wheel(&self.scroll)),
            )
            .when_some(all, |bar, all| {
                bar.child(
                    h_flex()
                        .flex_none()
                        .h_full()
                        .items_center()
                        .border_l_1()
                        .border_b_1()
                        .border_color(border)
                        .child(all),
                )
            })
    }
}
