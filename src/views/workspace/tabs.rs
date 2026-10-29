//! F1: the open APIs as tabs above the request editor: the method and the
//! name, a dot while its request is on its way, the one in the editor drawn
//! as the page. A click opens one; the close button (on the open tab and
//! the one under the pointer) or a middle click closes it, and its menu
//! closes the others or all. More than the row holds scroll sideways, the
//! open one brought into view.

use super::{method_color, method_label};
use crate::states::WorkspaceState;
use cyberapi_core::ItemId;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, StyledExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px, rems,
};
use rust_i18n::t;

pub struct TabsBar {
    state: Entity<WorkspaceState>,
    scroll: ScrollHandle,
    /// The tab open when last drawn: another one is scrolled into view.
    shown: Option<ItemId>,
    _subscription: Subscription,
}

impl TabsBar {
    pub fn new(state: Entity<WorkspaceState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            scroll: ScrollHandle::new(),
            shown: None,
            _subscription: subscription,
        }
    }
}

impl Render for TabsBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let opened = state.opened().cloned();
        let editable = state.editable();
        let tabs: Vec<_> = state
            .tabs()
            .into_iter()
            .map(|tab| {
                let sending = state.tab_sending(&tab.id, cx);
                (tab.id, tab.method, tab.name, sending, tab.scratch)
            })
            .collect();
        if opened != self.shown {
            if let Some(at) = tabs.iter().position(|(id, ..)| Some(id) == opened.as_ref()) {
                self.scroll.scroll_to_item(at);
            }
            self.shown = opened.clone();
        }
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let row = tabs
            .into_iter()
            .enumerate()
            .map(|(index, (id, method, name, sending, scratch))| {
                let active = opened.as_ref() == Some(&id);
                let group = SharedString::from(format!("api-tab-{index}"));
                let (open_id, close_id, middle_id) = (id.clone(), id.clone(), id.clone());
                let menu_state = self.state.downgrade();
                let tip = name.clone();
                h_flex()
                    .id(("api-tab", index))
                    .group(group.clone())
                    .flex_none()
                    .max_w(px(200.))
                    .h_full()
                    .pl_2()
                    .pr_0p5()
                    .gap_1p5()
                    .items_center()
                    .border_r_1()
                    .border_color(border)
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
                    .child(
                        div()
                            .flex_none()
                            .text_size(rems(0.7))
                            .font_bold()
                            .text_color(method_color(method, cx))
                            .child(method_label(method)),
                    )
                    .child(div().min_w_0().truncate().child(name.clone()))
                    .when(sending, |tab| {
                        tab.child(div().flex_none().size(px(6.)).rounded_full().bg(theme.primary))
                    })
                    .child(
                        div()
                            .flex_none()
                            .when(!active, |close| {
                                close.invisible().group_hover(group, |style| style.visible())
                            })
                            .child(
                                Button::new(("close-api-tab", index))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .tooltip(t!("workspace.close_tab").to_string())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.state.update(cx, |state, cx| state.close_tab(&close_id, cx));
                                    })),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.update(cx, |state, cx| state.select(&open_id, cx));
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
            .id("api-tabs")
            .flex_none()
            .w_full()
            .h(px(30.))
            .items_center()
            .border_b_1()
            .border_color(border)
            .bg(theme.secondary)
            .text_color(muted)
            .overflow_x_scroll()
            .track_scroll(&self.scroll)
            .children(row.collect::<Vec<_>>())
    }
}
