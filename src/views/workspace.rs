//! A project's workspace. For now only its frame: the tree, the request editor
//! and the response come with step 6 of `docs/architecture.md`.

use cyberapi_core::model::Collection;
use gpui_kit::component::empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle};
use gpui_kit::component::{Icon, IconName, v_flex};
use gpui_kit::{Context, IntoElement, ParentElement as _, Render, Styled as _, Window};
use rust_i18n::t;

pub struct WorkspaceView {
    collection: Collection,
}

impl WorkspaceView {
    pub fn new(collection: Collection) -> Self {
        Self { collection }
    }

    pub fn collection(&self) -> &Collection {
        &self.collection
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().justify_center().items_center().child(
            Empty::new().header(
                EmptyHeader::new()
                    .media(
                        EmptyMedia::new()
                            .with_variant(EmptyMediaVariant::Icon)
                            .child(Icon::new(IconName::LayoutDashboard)),
                    )
                    .title(EmptyTitle::new().child(t!("workspace.pending_title").to_string()))
                    .description(EmptyDescription::new().child(t!("workspace.pending_description").to_string())),
            ),
        )
    }
}
