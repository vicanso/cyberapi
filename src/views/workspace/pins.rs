//! The pinned APIs as tabs in the title bar (M12), after a divider and a pin
//! so they read as a bar of their own, not more of the path before them:
//! the number (where Cmd/Ctrl and it opens the pin), the method and the
//! name; the one in the editor in the tree's selected color. A click opens
//! it without moving the tree's selection; the close button, shown on the
//! open tab and on the one under the pointer, unpins. More than the bar
//! holds: a button at its end lists them all.

use super::{KEY_CONTEXT, method_color, method_label, pin_action};
use crate::states::WorkspaceState;
use gpui_kit::assets::IconName as Icons;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px, rems,
};
use rust_i18n::t;

pub struct PinsBar {
    state: Entity<WorkspaceState>,
    /// The row of tags, which scrolls sideways when they do not fit.
    scroll: ScrollHandle,
    /// Some tags are past the bar's end, as last laid out.
    overflowing: bool,
    _subscription: Subscription,
}

impl PinsBar {
    pub fn new(state: Entity<WorkspaceState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            scroll: ScrollHandle::new(),
            overflowing: false,
            _subscription: subscription,
        }
    }

    /// Whether the tags fit is known once they are laid out: checked after
    /// each frame, drawn again only when it changed.
    fn watch_overflow(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.on_next_frame(window, |this, _, cx| {
            let overflowing = this.scroll.max_offset().x > px(0.5);
            if overflowing != this.overflowing {
                this.overflowing = overflowing;
                cx.notify();
            }
        });
    }
}

impl Render for PinsBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.watch_overflow(window, cx);
        let all = self.overflowing.then(|| self.render_all());
        let state = self.state.read(cx);
        let selected = state.opened().cloned();
        let pinned = state.pinned();
        let any = !pinned.is_empty();
        let shortcuts = crate::this_build::takes_browser_keys();
        let theme = cx.theme();
        let tags = pinned.into_iter().enumerate().map(|(index, api)| {
            let active = selected.as_ref() == Some(&api.id);
            let (select_id, unpin_id) = (api.id.clone(), api.id.clone());
            let group = SharedString::from(format!("pin-{index}"));
            // The whole name, and the key that selects it.
            let action = pin_action(index).filter(|_| shortcuts);
            let name = api.name.clone();
            h_flex()
                .id(("pin", index))
                .group(group.clone())
                .tooltip(move |window, cx| {
                    let tooltip = Tooltip::new(name.clone());
                    match &action {
                        Some(action) => tooltip.action(action.as_ref(), Some(KEY_CONTEXT)),
                        None => tooltip,
                    }
                    .build(window, cx)
                })
                .flex_none()
                .max_w(px(200.))
                .h(px(22.))
                .pl_2()
                .gap_0p5()
                .items_center()
                .rounded(theme.radius)
                .border_1()
                .text_xs()
                .cursor_pointer()
                .map(|tag| {
                    if active {
                        tag.bg(theme.list_active)
                            .border_color(theme.list_active_border)
                            .text_color(theme.foreground)
                    } else {
                        tag.bg(theme.secondary)
                            .border_color(theme.border)
                            .text_color(theme.secondary_foreground)
                            .hover(|style| style.bg(theme.secondary_hover))
                    }
                })
                .child(
                    h_flex()
                        .min_w_0()
                        .gap_1p5()
                        .items_center()
                        .when(shortcuts, |row| {
                            row.child(div().text_color(theme.muted_foreground).child(format!("{}", index + 1)))
                        })
                        .child(
                            div()
                                .text_size(rems(0.7))
                                .font_bold()
                                .text_color(method_color(api.request.method, cx))
                                .child(method_label(api.request.method)),
                        )
                        .child(div().min_w_0().truncate().child(api.name.clone())),
                )
                .child(
                    div()
                        .flex_none()
                        .when(!active, |close| {
                            close.invisible().group_hover(group, |style| style.visible())
                        })
                        .child(
                            Button::new(("unpin", index))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .tooltip(t!("workspace.unpin").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.state
                                        .update(cx, |state, cx| state.set_pinned(&unpin_id, false, cx));
                                })),
                        ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.state.update(cx, |state, cx| state.open_pin(&select_id, cx));
                }))
        });
        let row = h_flex()
            .id("pins")
            .flex_1()
            .min_w_0()
            .gap_1()
            .items_center()
            .overflow_x_scroll()
            .track_scroll(&self.scroll)
            .children(tags.collect::<Vec<_>>());
        h_flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .when(any, |bar| {
                bar.child(div().flex_none().mx_2().w(px(1.)).h(px(16.)).bg(theme.border))
                    .child(
                        Icon::new(Icons::Pin)
                            .xsmall()
                            .flex_none()
                            .mr_1p5()
                            .text_color(theme.muted_foreground),
                    )
            })
            .child(row)
            .when_some(all, |bar, all| bar.child(all))
    }
}

impl PinsBar {
    /// Every pin, for when some are past the bar's end: a choice opens it,
    /// and brings its tag into view.
    fn render_all(&self) -> impl IntoElement + use<> {
        let state = self.state.downgrade();
        let scroll = self.scroll.clone();
        Button::new("pins-all")
            .ghost()
            .xsmall()
            .flex_none()
            .icon(IconName::ChevronDown)
            .tooltip(t!("workspace.all_pins").to_string())
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
                let Some(workspace) = state.upgrade() else {
                    return menu;
                };
                let workspace = workspace.read(cx);
                let opened = workspace.opened().cloned();
                let pins: Vec<_> = workspace
                    .pinned()
                    .into_iter()
                    .map(|api| (api.id.clone(), api.request.method, api.name.clone()))
                    .collect();
                pins.into_iter()
                    .enumerate()
                    .fold(menu, |menu, (index, (id, method, name))| {
                        let (state, scroll) = (state.clone(), scroll.clone());
                        let checked = opened.as_ref() == Some(&id);
                        let item = PopupMenuItem::element(move |_, cx| {
                            h_flex()
                                .gap_1p5()
                                .items_center()
                                .max_w(px(320.))
                                .child(
                                    div()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("{}", index + 1)),
                                )
                                .child(
                                    div()
                                        .text_size(rems(0.7))
                                        .font_bold()
                                        .text_color(method_color(method, cx))
                                        .child(method_label(method)),
                                )
                                .child(div().min_w_0().truncate().child(name.clone()))
                        });
                        menu.item(item.checked(checked).on_click(move |_, _, cx| {
                            scroll.scroll_to_item(index);
                            let _ = state.update(cx, |state, cx| state.open_pin(&id, cx));
                        }))
                    })
            })
    }
}
