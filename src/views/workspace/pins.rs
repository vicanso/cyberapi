//! The pinned APIs as tags in the title bar (M12): the number, the method
//! and the name, the selected one marked; a click — or Cmd/Ctrl and the
//! number — selects, the close button unpins.

use super::{KEY_CONTEXT, method_color, method_label, pin_action};
use crate::states::WorkspaceState;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, StyledExt as _, h_flex};
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
            // The whole name, and the key that selects it. The current pin
            // is the brighter label; the others stay quiet, with no box.
            let (name, action) = (api.name.clone(), pin_action(index));
            let ink = if active {
                theme.foreground
            } else {
                theme.muted_foreground
            };
            let method_ink = method_color(api.request.method, cx);
            let method_ink = if active { method_ink } else { method_ink.opacity(0.55) };
            h_flex()
                .id(("pin", index))
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
                .h(px(24.))
                .pl_2()
                .gap_0p5()
                .items_center()
                .rounded(theme.radius)
                .text_xs()
                .text_color(ink)
                .cursor_pointer()
                .hover(|style| style.bg(theme.list_hover))
                .child(
                    h_flex()
                        .min_w_0()
                        .gap_1p5()
                        .items_center()
                        .child(div().child(format!("{}", index + 1)))
                        .child(
                            div()
                                .text_size(px(10.))
                                .font_bold()
                                .text_color(method_ink)
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
