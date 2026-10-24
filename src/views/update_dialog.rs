//! The update check's face, as zedis's: a chip in the title bar while a
//! newer release is out, the dialog that tells of it and installs it, and
//! what a check says (`states/update.rs`). Only the desktop checks.

use crate::states::{AppStore, UpdateEvent, UpdateInfo, UpdatePhase, UpdateState};
use cyberapi_core::response::format_size;
use gpui_kit::assets::IconName as Icons;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

/// Notes longer than this scroll in a box of their own.
const SHORT_NOTES: usize = 800;

/// The title bar's chip: the newer version, or the restart an install in
/// place waits for. Opens the dialog.
pub fn update_chip(cx: &App) -> Option<AnyElement> {
    let state = AppStore::global(cx).updates()?.read(cx);
    let (icon, label, tooltip) = match (state.phase(), state.available()) {
        (UpdatePhase::Installed, _) => (
            Icons::RotateCw,
            t!("update.restart").to_string(),
            t!("update.installed_short").to_string(),
        ),
        (_, Some(info)) => (
            Icons::Download,
            format!("v{}", info.version),
            t!("update.chip_tooltip").to_string(),
        ),
        _ => return None,
    };
    Some(
        Button::new("title-update")
            .ghost()
            .small()
            .icon(icon)
            .label(label)
            .tooltip(tooltip)
            .on_click(|_, window, cx| open_update_dialog(window, cx))
            .into_any_element(),
    )
}

/// What the main window says of a check: a newer release found at start, or
/// the answer when the reader asked there. A check asked for in the
/// settings is answered in them.
pub fn follow_update(event: &UpdateEvent, window: &mut Window, cx: &mut App) {
    let asked_here = window.is_window_active();
    let note = match event {
        UpdateEvent::Found { manual: false } => AppStore::global(cx)
            .updates()
            .and_then(|state| state.read(cx).available())
            .map(|info| Notification::info(t!("update.found_at_start", version = info.version).to_string())),
        UpdateEvent::Found { manual: true } if asked_here => {
            open_update_dialog(window, cx);
            None
        }
        UpdateEvent::UpToDate if asked_here => Some(Notification::success(
            t!("update.up_to_date", version = crate::VERSION).to_string(),
        )),
        UpdateEvent::CheckFailed(error) if asked_here => Some(Notification::error(
            t!("update.check_failed", error = error).to_string(),
        )),
        _ => None,
    };
    if let Some(note) = note {
        window.push_notification(note, cx);
    }
}

/// The newer release: its notes, and the way to it. It stays open while the
/// package comes down, and says how the install went.
pub fn open_update_dialog(window: &mut Window, cx: &mut App) {
    let Some(state) = AppStore::global(cx).updates().cloned() else {
        return;
    };
    let Some(info) = state.read(cx).available().cloned() else {
        return;
    };
    let title = t!("update.title", version = info.version).to_string();
    let prompt = cx.new(|cx| UpdatePrompt::new(state, info, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(title.clone())
            .w(px(520.))
            // Closed by its own buttons: a download under way finishes in it.
            .close_button(false)
            .keyboard(false)
            .overlay_closable(false)
            .child(prompt.clone())
    });
}

struct UpdatePrompt {
    state: Entity<UpdateState>,
    /// The release, as the dialog opened.
    info: UpdateInfo,
    /// Why the last download or install failed.
    failed: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl UpdatePrompt {
    fn new(state: Entity<UpdateState>, info: UpdateInfo, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe_in(&state, window, |this, _, event, window, cx| match event {
                UpdateEvent::HandedToOs(path) => {
                    window.close_dialog(cx);
                    let path = path.display().to_string();
                    window.push_notification(Notification::success(t!("update.handed", path = path).to_string()), cx);
                }
                UpdateEvent::InstallFailed(error) => {
                    this.failed = Some(error.clone());
                    cx.notify();
                }
                _ => {}
            }),
        ];
        Self {
            state,
            info,
            failed: None,
            _subscriptions: subscriptions,
        }
    }

    fn render_notes(&self, cx: &App) -> Option<AnyElement> {
        let notes = self.info.notes.trim();
        if notes.is_empty() {
            return None;
        }
        let text = TextView::markdown("update-notes", notes.to_string());
        Some(if notes.chars().count() > SHORT_NOTES || notes.lines().count() > 12 {
            div()
                .id("update-notes-scroll")
                .max_h(px(280.))
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .overflow_y_scrollbar()
                .child(text)
                .into_any_element()
        } else {
            div().child(text).into_any_element()
        })
    }

    fn render_buttons(&self, phase: UpdatePhase) -> AnyElement {
        let state = self.state.clone();
        let later = Button::new("update-later")
            .outline()
            .label(t!("update.later").to_string())
            .disabled(phase == UpdatePhase::Downloading)
            .on_click(|_, window, cx| window.close_dialog(cx));
        if phase == UpdatePhase::Installed {
            return h_flex()
                .gap_2()
                .justify_end()
                .child(later)
                .child(
                    Button::new("update-restart")
                        .primary()
                        .label(t!("update.restart_now").to_string())
                        .on_click(move |_, _, cx| state.update(cx, |state, cx| state.restart(cx))),
                )
                .into_any_element();
        }
        let page = self.info.page.clone();
        let skip = {
            let state = state.clone();
            Button::new("update-skip")
                .ghost()
                .label(t!("update.skip").to_string())
                .disabled(phase == UpdatePhase::Downloading)
                .on_click(move |_, window, cx| {
                    state.update(cx, |state, cx| state.skip(cx));
                    window.close_dialog(cx);
                })
        };
        let open_page = |id| {
            let page = page.clone();
            Button::new(id)
                .label(t!("update.release_page").to_string())
                .on_click(move |_, _, cx| cx.open_url(&page))
        };
        let row = h_flex().gap_2().child(skip).child(div().flex_1()).child(later);
        match &self.info.asset {
            Some(asset) => row
                .child(open_page("update-page").outline())
                .child(
                    Button::new("update-download")
                        .primary()
                        // A disk image goes in by itself; anything else is
                        // the system's to install.
                        .label(if asset.kind == "dmg" {
                            t!("update.download_install").to_string()
                        } else {
                            t!("update.download").to_string()
                        })
                        .loading(phase == UpdatePhase::Downloading)
                        .disabled(phase != UpdatePhase::Idle)
                        .on_click(move |_, _, cx| state.update(cx, |state, cx| state.download_and_install(cx))),
                )
                .into_any_element(),
            None => row.child(open_page("update-page").primary()).into_any_element(),
        }
    }
}

impl Render for UpdatePrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let phase = self.state.read(cx).phase();
        let muted = cx.theme().muted_foreground;
        let status = match (phase, &self.info.asset) {
            (UpdatePhase::Installed, _) => Some(t!("update.installed").to_string()),
            (UpdatePhase::Downloading, Some(asset)) => {
                Some(t!("update.downloading", name = asset.name, size = format_size(asset.size)).to_string())
            }
            (_, None) => Some(t!("update.no_package").to_string()),
            _ => None,
        };
        v_flex()
            .gap_3()
            .text_sm()
            .child(div().text_color(muted).child(t!(
                "update.versions",
                current = crate::VERSION,
                latest = self.info.version
            )))
            .children(self.render_notes(cx))
            .children(status.map(|status| div().child(status)))
            .children(
                self.failed
                    .as_ref()
                    .filter(|_| phase == UpdatePhase::Idle)
                    .map(|error| {
                        div()
                            .text_color(cx.theme().danger)
                            .child(t!("update.install_failed", error = error).to_string())
                    }),
            )
            .child(self.render_buttons(phase))
    }
}
