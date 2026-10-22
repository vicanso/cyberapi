//! Invitations in a dialog (`docs/server-sync.md` §5): the link just made,
//! to copy again, and the reader's invitations that can still be used, each
//! taken back with a click. The accounts dialog lists those into no team,
//! the teams dialog those into the team it shows.

use crate::i18n;
use crate::states::DirectoryState;
use cyberapi_core::TeamId;
use cyberapi_core::Timestamp;
use cyberapi_core::directory::Invite;
use cyberapi_core::format::days_until;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{AnyElement, App, ClipboardItem, Entity, IntoElement, ParentElement as _, Styled as _, div};
use rust_i18n::t;

/// Puts `link` on the clipboard and says so.
pub fn copy_link(link: &str, window: &mut gpui_kit::Window, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(link.to_string()));
    window.push_notification(Notification::success(t!("invites.copied").to_string()), cx);
}

/// The open invitations into `team` (`None`: into no team), or nothing
/// when there are none.
pub fn invites_section(state: &Entity<DirectoryState>, team: Option<&TeamId>, cx: &App) -> Option<AnyElement> {
    let directory = state.read(cx);
    let invites: Vec<Invite> = directory.invites(team).cloned().collect();
    if invites.is_empty() {
        return None;
    }
    let made = directory
        .made()
        .filter(|(id, _)| invites.iter().any(|invite| &invite.id == id))
        .map(|(_, link)| link.clone());
    let theme = cx.theme();
    let (muted, border) = (theme.muted_foreground, theme.border);
    let now = Timestamp::new(chrono::Local::now().fixed_offset());
    let rows = invites.into_iter().enumerate().map(|(index, invite)| {
        let what = match &invite.place {
            Some(place) => t!("invites.as_role", role = i18n::team_role(place.role)).to_string(),
            None => t!("invites.own_account").to_string(),
        };
        let lasts = t!("invites.expires_in", days = days_until(invite.expires_at, now)).to_string();
        let (state, id) = (state.clone(), invite.id.clone());
        h_flex()
            .gap_2()
            .py_0p5()
            .items_center()
            .text_sm()
            .child(div().flex_1().min_w_0().truncate().child(what))
            .child(div().text_xs().text_color(muted).child(lasts))
            .child(
                Button::new(("invite-revoke", index))
                    .ghost()
                    .xsmall()
                    .label(t!("invites.revoke").to_string())
                    .on_click(move |_, _, cx| {
                        let id = id.clone();
                        state.update(cx, |state, cx| state.revoke_invite(id, cx));
                    }),
            )
    });
    Some(
        v_flex()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(border)
            .child(div().text_xs().text_color(muted).child(t!("invites.open").to_string()))
            .children(made.map(|link| {
                let copied = link.clone();
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .font_family(theme.mono_font_family.clone())
                            .child(link),
                    )
                    .child(
                        Button::new("invite-copy")
                            .outline()
                            .xsmall()
                            .icon(IconName::Copy)
                            .label(t!("common.copy").to_string())
                            .on_click(move |_, window, cx| copy_link(&copied, window, cx)),
                    )
            }))
            .children(rows)
            .into_any_element(),
    )
}
