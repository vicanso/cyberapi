//! When a server project moved on since the reader's copy of it
//! (`docs/server-sync.md` §3.3): their changes over the server's, the
//! server's over theirs, or neither. Nothing is merged.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{App, ParentElement as _, SharedString, Styled as _, Window, div, px};
use rust_i18n::t;
use std::rc::Rc;

/// Asks about `name`'s conflict; `mine` pushes the reader's copy over the
/// server's, `servers` takes the server's in place of it.
pub fn open_conflict_dialog(
    name: &str,
    window: &mut Window,
    cx: &mut App,
    mine: impl Fn(&mut App) + 'static,
    servers: impl Fn(&mut App) + 'static,
) {
    let title: SharedString = t!("sync.conflict_title", name = name).to_string().into();
    let (mine, servers) = (Rc::new(mine), Rc::new(servers));
    window.open_dialog(cx, move |dialog, _, cx| {
        let (mine, servers) = (mine.clone(), servers.clone());
        dialog
            .title(title.clone())
            .w(px(460.))
            .child(
                v_flex()
                    .gap_2()
                    .text_sm()
                    .child(t!("sync.conflict_description").to_string())
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("sync.conflict_hint").to_string()),
                    ),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                    // Each as wide as the cancel button, which the kit lays
                    // out to fill its share: three equal choices.
                    .child(
                        div().size_full().child(
                            Button::new("conflict-servers")
                                .outline()
                                .label(t!("sync.use_servers").to_string())
                                .on_click(move |_, window, cx| {
                                    servers(cx);
                                    window.close_dialog(cx);
                                }),
                        ),
                    )
                    .child(
                        div().size_full().child(
                            Button::new("conflict-mine")
                                .with_variant(ButtonVariant::Danger)
                                .label(t!("sync.use_mine").to_string())
                                .on_click(move |_, window, cx| {
                                    mine(cx);
                                    window.close_dialog(cx);
                                }),
                        ),
                    ),
            )
            // No button is the default: Enter chooses nothing.
            .on_ok(|_, _, _| false)
    });
}
