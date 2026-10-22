//! The project's name in the title bar, as the workspace has it — a server
//! project's draft may name it otherwise than the server — and its sync
//! (`docs/server-sync.md` §3.2, §3.3): while the reader has changes the
//! server has not, it says so, and that the server moved on too when it
//! did, beside the button that syncs them. No sync for a project of this
//! machine's, or for a viewer.

use crate::states::WorkspaceState;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Sizable as _, StyledExt as _, h_flex};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

pub struct ProjectTitle {
    state: Entity<WorkspaceState>,
    _subscription: Subscription,
}

impl ProjectTitle {
    pub fn new(state: Entity<WorkspaceState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            _subscription: subscription,
        }
    }
}

impl Render for ProjectTitle {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let name = div()
            .flex_none()
            .max_w(px(240.))
            .truncate()
            .mr_3()
            .text_sm()
            .font_medium()
            .child(state.collection().name.clone());
        // A long name gives way to the pins beside it.
        if !state.unsynced() {
            return name.into_any_element();
        }
        let (label, hint) = if state.server_moved() {
            (t!("sync.unsynced_behind"), t!("sync.unsynced_behind_hint"))
        } else {
            (t!("sync.unsynced"), t!("sync.unsynced_hint"))
        };
        let hint = hint.to_string();
        let syncing = state.syncing();
        let state = self.state.clone();
        h_flex()
            .flex_none()
            .items_center()
            .gap_1()
            .mr_3()
            .child(name.mr_1())
            .child(
                div()
                    .id("sync-status")
                    .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                    .child(Tag::warning().small().rounded_full().child(label.to_string())),
            )
            .child(
                Button::new("sync-push")
                    .ghost()
                    .xsmall()
                    .icon(gpui_kit::assets::IconName::CloudUpload)
                    .label(t!("sync.sync").to_string())
                    .loading(syncing)
                    .on_click(move |_, _, cx| state.update(cx, |state, cx| state.sync(false, cx))),
            )
            .into_any_element()
    }
}
