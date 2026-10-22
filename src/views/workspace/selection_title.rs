//! The selection, drawn in the middle of the title bar: the folders it sits
//! in, then — for an API — its method and name.

use super::method_color;
use crate::states::WorkspaceState;
use cyberapi_core::model::HttpMethod;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px,
};

pub struct SelectionTitle {
    state: Entity<WorkspaceState>,
    _subscription: Subscription,
}

impl SelectionTitle {
    pub fn new(state: Entity<WorkspaceState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            _subscription: subscription,
        }
    }
}

/// Folders from the top of the tree down to the parent, the method when the
/// selection is an API, and the selection's own name.
fn selection(state: &WorkspaceState) -> Option<(Vec<String>, Option<HttpMethod>, String)> {
    let node = state.tree().get(state.selected()?)?;
    let mut folders = Vec::new();
    let mut parent = node.parent().cloned();
    while let Some(id) = parent {
        let Some(folder) = state.tree().get(&id) else { break };
        folders.push(folder.name().to_string());
        parent = folder.parent().cloned();
    }
    folders.reverse();
    let method = (!node.is_folder()).then(|| node.method());
    Some((folders, method, node.name().to_string()))
}

impl Render for SelectionTitle {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = selection(self.state.read(cx));
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        h_flex()
            .id("selection-title")
            .flex_none()
            .items_center()
            .when_some(shown, |row, (folders, method, name)| {
                let mut tip = folders.clone();
                tip.push(match method {
                    Some(method) => format!("{} {name}", method.as_str()),
                    None => name.clone(),
                });
                let tip = tip.join(" / ");
                let folders = (!folders.is_empty()).then(|| folders.join(" / "));
                row.min_w_0()
                    .max_w(px(420.))
                    .h(px(24.))
                    .px_2()
                    .gap_1p5()
                    .overflow_hidden()
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .when_some(folders, |row, folders| {
                        row.child(
                            div()
                                .min_w_0()
                                .max_w(px(180.))
                                .truncate()
                                .text_sm()
                                .text_color(muted)
                                .child(folders),
                        )
                        .child(div().flex_none().text_sm().text_color(muted).child("/"))
                    })
                    .when_some(method, |row, method| {
                        row.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .font_bold()
                                .text_color(method_color(method, cx))
                                .child(method.as_str()),
                        )
                    })
                    .child(
                        div()
                            .min_w_0()
                            .max_w(px(220.))
                            .truncate()
                            .text_sm()
                            .font_medium()
                            .text_color(foreground)
                            .child(name),
                    )
            })
    }
}
