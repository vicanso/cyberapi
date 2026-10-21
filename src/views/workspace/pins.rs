//! The pinned APIs as tags in the title bar (M12): the number, the method
//! and the name, the selected one marked; a click selects, the close button
//! unpins.

use super::{method_color, method_label};
use crate::states::WorkspaceState;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, StyledExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

pub struct PinsBar {
    state: Entity<WorkspaceState>,
    _subscription: Subscription,
}

impl PinsBar {
    pub fn new(state: Entity<WorkspaceState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            _subscription: subscription,
        }
    }
}

impl Render for PinsBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let selected = state.selected().cloned();
        let theme = cx.theme();
        let tags = state.pinned().into_iter().enumerate().map(|(index, api)| {
            let active = selected.as_ref() == Some(&api.id);
            let (select_id, unpin_id) = (api.id.clone(), api.id.clone());
            h_flex()
                .id(("pin", index))
                .flex_none()
                .max_w(px(200.))
                .h(px(24.))
                .pl_2()
                .gap_0p5()
                .items_center()
                .rounded(theme.radius)
                .border_1()
                .text_xs()
                .cursor_pointer()
                .map(|this| {
                    if active {
                        this.border_color(theme.primary).bg(theme.list_active)
                    } else {
                        this.border_color(theme.border)
                            .hover(|style| style.bg(theme.list_hover))
                    }
                })
                .child(
                    h_flex()
                        .min_w_0()
                        .gap_1p5()
                        .items_center()
                        .child(div().text_color(theme.muted_foreground).child(format!("{}", index + 1)))
                        .child(
                            div()
                                .text_size(px(10.))
                                .font_bold()
                                .text_color(method_color(api.request.method, cx))
                                .child(method_label(api.request.method)),
                        )
                        .child(div().min_w_0().truncate().child(api.name.clone())),
                )
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
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.state.update(cx, |state, cx| state.select(&select_id, cx));
                }))
        });
        h_flex()
            .id("pins")
            .flex_1()
            .min_w_0()
            .gap_1()
            .items_center()
            .overflow_x_scroll()
            .children(tags.collect::<Vec<_>>())
    }
}
