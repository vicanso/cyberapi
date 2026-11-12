//! The update check's face (R2): an animated checking indicator in the title
//! bar while checking, a version badge that opens a Popover with release notes
//! and "Skip This Version" / "Update & Restart" actions when a newer release
//! is out, inline download progress in the title bar while downloading, a
//! retry badge on failure, and the restart button once installed in place.
//! Only the desktop checks (`states/update.rs`).

use crate::states::{AppStore, UpdateEvent, UpdateInfo, UpdatePhase, UpdateState};
use cyberapi_core::response::format_size;
use gpui_kit::assets::IconName as Icons;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::progress::{Progress, ProgressCircle};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div,
    px, rems,
};
use rust_i18n::t;

/// Notes longer than this scroll in a box of their own.
const SHORT_NOTES: usize = 800;
/// Cap on rendered Markdown length so a huge multi-release changelog stays snappy.
const MAX_NOTES: usize = 20_000;

/// Wraps a title-bar update element so mouse-down events never bubble to
/// `TitleBar` and initiate a window drag.
fn stop_titlebar_drag(element: impl IntoElement) -> AnyElement {
    div()
        .id("titlebar-update-container")
        .flex_none()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(element)
        .into_any_element()
}

/// The title bar's update indicator (R2):
/// - animated checking spinner while checking for updates,
/// - version badge with release-notes Popover ("Skip" / "Update & Restart") when found,
/// - inline progress indicator (`ProgressCircle` + percentage) while downloading,
/// - retry badge if download/install failed,
/// - restart button once an in-place install has completed.
pub fn update_chip(cx: &App) -> Option<AnyElement> {
    let entity = AppStore::global(cx).updates()?.clone();
    let state = entity.read(cx);
    let phase = state.phase();
    match (phase, state.available()) {
        (UpdatePhase::Checking { .. }, _) => {
            let checking_tip = t!("update.checking_short").to_string();
            Some(stop_titlebar_drag(
                Button::new("title-update-checking")
                    .ghost()
                    .small()
                    .loading(true)
                    .disabled(true)
                    .label(t!("update.checking").to_string())
                    .tooltip(checking_tip),
            ))
        }
        (UpdatePhase::Installed, _) => {
            let restart_entity = entity.clone();
            Some(stop_titlebar_drag(
                Button::new("title-update-restart")
                    .primary()
                    .small()
                    .icon(Icons::RotateCw)
                    .label(t!("update.restart").to_string())
                    .tooltip(t!("update.installed_short").to_string())
                    .on_click(move |_, _, cx| restart_entity.update(cx, |state, cx| state.restart(cx))),
            ))
        }
        (UpdatePhase::Downloading, Some(info)) => {
            let pct = state.download_percent();
            let progress = state.download_progress();
            let label = match pct {
                Some(pct) => format!("{pct}%"),
                None => format!("v{}", info.version),
            };
            let tooltip = match progress {
                Some((done, total)) if total > 0 => format!(
                    "{} ({} / {})",
                    t!("update.downloading_short", version = info.version),
                    format_size(done),
                    format_size(total)
                ),
                _ => t!("update.downloading_short", version = info.version).to_string(),
            };
            let indicator: AnyElement = match pct {
                Some(pct) => ProgressCircle::new("title-update-progress")
                    .size(px(14.))
                    .value(pct as f32)
                    .color(cx.theme().primary)
                    .into_any_element(),
                None => Spinner::new().xsmall().into_any_element(),
            };
            let border = cx.theme().border;
            let bg = cx.theme().secondary.opacity(0.45);
            Some(stop_titlebar_drag(
                h_flex()
                    .id("title-update-downloading")
                    .h(px(24.))
                    .px_2()
                    .gap_1p5()
                    .items_center()
                    .rounded_md()
                    .border_1()
                    .border_color(border)
                    .bg(bg)
                    .text_xs()
                    .font_medium()
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .child(indicator)
                    .child(label),
            ))
        }
        (UpdatePhase::Idle, Some(info)) => {
            let info = info.clone();
            let failed = state.failed().is_some();
            let dismiss_hover_bg = cx.theme().foreground.opacity(0.12);
            let dismiss_fg = cx.theme().muted_foreground;
            let dismiss_tooltip = t!("update.dismiss_tooltip").to_string();
            let dismiss_entity = entity.clone();

            let trigger = if failed {
                Button::new("title-update")
                    .danger()
                    .small()
                    .icon(Icons::CircleAlert)
                    .label(format!("v{} · {}", info.version, t!("update.retry")))
                    .tooltip(t!("update.retry_tooltip").to_string())
            } else {
                Button::new("title-update")
                    .outline()
                    .small()
                    .icon(Icons::Download)
                    .label(format!("v{}", info.version))
                    .tooltip(t!("update.chip_tooltip").to_string())
                    .child(
                        div()
                            .id("title-update-dismiss")
                            .flex_none()
                            .ml_0p5()
                            .p_0p5()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(move |s| s.bg(dismiss_hover_bg))
                            .tooltip(move |window, cx| Tooltip::new(dismiss_tooltip.clone()).build(window, cx))
                            .child(Icon::new(Icons::Close).xsmall().text_color(dismiss_fg))
                            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                cx.stop_propagation();
                                dismiss_entity.update(cx, |state, cx| state.dismiss(cx));
                            }),
                    )
            };

            let popover_entity = entity.clone();
            Some(stop_titlebar_drag(
                Popover::new("title-update-popover")
                    .anchor(Anchor::TopRight)
                    .p_3()
                    .trigger(trigger)
                    .content(move |_, window, cx| {
                        let popover = cx.entity().downgrade();
                        render_popover_content(&popover_entity, &info, popover, window, cx)
                    }),
            ))
        }
        _ => None,
    }
}

fn render_popover_content(
    state_entity: &Entity<UpdateState>,
    info: &UpdateInfo,
    popover: WeakEntity<PopoverState>,
    _window: &mut Window,
    cx: &mut Context<PopoverState>,
) -> AnyElement {
    let state = state_entity.read(cx);
    let failed_error = state.failed().map(str::to_string);
    let muted = cx.theme().muted_foreground;
    let danger = cx.theme().danger;

    let header = h_flex()
        .gap_2()
        .items_baseline()
        .justify_between()
        .child(
            div()
                .text_sm()
                .font_semibold()
                .child(t!("update.title", version = info.version).to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(muted)
                .child(t!("update.versions", current = crate::VERSION, latest = info.version).to_string()),
        );

    let notes = render_notes_markdown("popover-update-notes", &info.notes, px(240.), cx);

    let skip_state = state_entity.clone();
    let skip_popover = popover.clone();
    let skip_btn = Button::new("popover-update-skip")
        .ghost()
        .small()
        .label(t!("update.skip").to_string())
        .on_click(move |_, window, cx| {
            let _ = skip_popover.update(cx, |pop, cx| pop.dismiss(window, cx));
            skip_state.update(cx, |state, cx| state.skip(cx));
        });

    let page_url = info.page.clone();
    let actions = match &info.asset {
        Some(asset) => {
            let install_state = state_entity.clone();
            let install_popover = popover.clone();
            let install_label = if asset.kind == "dmg" {
                t!("update.update_and_restart").to_string()
            } else {
                t!("update.download_install").to_string()
            };
            let page_url_btn = page_url.clone();
            let release_tip = t!("update.release_page").to_string();
            h_flex()
                .gap_2()
                .items_center()
                .child(skip_btn)
                .child(div().flex_1())
                .when(!page_url_btn.is_empty(), move |this| {
                    this.child(
                        Button::new("popover-update-page")
                            .ghost()
                            .small()
                            .icon(Icons::ExternalLink)
                            .tooltip(release_tip.clone())
                            .on_click(move |_, _, cx| cx.open_url(&page_url_btn)),
                    )
                })
                .child(
                    Button::new("popover-update-install")
                        .primary()
                        .small()
                        .label(install_label)
                        .on_click(move |_, window, cx| {
                            let _ = install_popover.update(cx, |pop, cx| pop.dismiss(window, cx));
                            install_state.update(cx, |state, cx| state.download_and_install(cx));
                        }),
                )
        }
        None => {
            let page_popover = popover;
            h_flex()
                .gap_2()
                .items_center()
                .child(skip_btn)
                .child(div().flex_1())
                .child(
                    Button::new("popover-update-page")
                        .primary()
                        .small()
                        .label(t!("update.release_page").to_string())
                        .on_click(move |_, window, cx| {
                            let _ = page_popover.update(cx, |pop, cx| pop.dismiss(window, cx));
                            if !page_url.is_empty() {
                                cx.open_url(&page_url);
                            }
                        }),
                )
        }
    };

    v_flex()
        .w(px(380.))
        .gap_2p5()
        .text_sm()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(header)
        .children(notes)
        .when(info.asset.is_none(), |this| {
            this.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("update.no_package").to_string()),
            )
        })
        .when_some(failed_error, |this, error| {
            this.child(
                div()
                    .text_xs()
                    .text_color(danger)
                    .child(t!("update.install_failed", error = error).to_string()),
            )
        })
        .child(actions)
        .into_any_element()
}

fn render_notes_markdown(
    id: &'static str,
    raw_notes: &str,
    max_height: gpui_kit::Pixels,
    cx: &App,
) -> Option<AnyElement> {
    let trimmed = raw_notes.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut notes = trimmed.to_string();
    if notes.chars().count() > MAX_NOTES {
        notes = notes.chars().take(MAX_NOTES).collect();
        notes.push('…');
    }
    let style = TextViewStyle::default()
        .paragraph_gap(rems(0.5))
        .heading_font_size(|level, _base| match level {
            1 => px(17.),
            2 => px(15.),
            3 => px(14.),
            _ => px(13.),
        });
    let long = notes.chars().count() > SHORT_NOTES || notes.lines().count() > 10;
    let text = TextView::markdown(id, notes).style(style);
    Some(if long {
        div()
            .id((id, 1usize))
            .w_full()
            .h(max_height)
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .overflow_y_scrollbar()
            .child(text)
            .into_any_element()
    } else {
        div().w_full().child(text).into_any_element()
    })
}

/// What the main window says of a check: a newer release found at start,
/// the dialog when the reader asked for a check, or automatic restart once
/// an in-place install lands.
pub fn follow_update(event: &UpdateEvent, window: &mut Window, cx: &mut App) {
    let asked_here = window.is_window_active();
    let note = match event {
        UpdateEvent::Found { manual: false } => AppStore::global(cx)
            .updates()
            .and_then(|state| state.read(cx).available())
            .map(|info| {
                Notification::info(t!("update.found_at_start", version = info.version).to_string()).action(|_, _, _| {
                    Button::new("update-toast-show")
                        .small()
                        .primary()
                        .label(t!("update.show").to_string())
                        .on_click(|_, window, cx| open_update_dialog(window, cx))
                })
            }),
        UpdateEvent::Found { manual: true } => {
            window.activate_window();
            open_update_dialog(window, cx);
            None
        }
        UpdateEvent::Installed => {
            window.close_dialog(cx);
            crate::this_build::relaunch();
            cx.quit();
            None
        }
        UpdateEvent::HandedToOs(path) => {
            window.close_dialog(cx);
            let path = path.display().to_string();
            Some(Notification::success(t!("update.handed", path = path).to_string()))
        }
        UpdateEvent::InstallFailed(error) => {
            let updates = AppStore::global(cx).updates().cloned();
            Some(
                Notification::error(t!("update.install_failed", error = error).to_string()).action(move |_, _, _| {
                    let updates = updates.clone();
                    Button::new("update-toast-retry")
                        .small()
                        .primary()
                        .label(t!("update.retry").to_string())
                        .on_click(move |_, _, cx| {
                            if let Some(updates) = &updates {
                                updates.update(cx, |state, cx| state.download_and_install(cx));
                            }
                        })
                }),
            )
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
            .on_ok(|_, _, _| false)
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
        let initial_failed = state.read(cx).failed().map(str::to_string);
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe_in(&state, window, |this, _, event, _window, cx| {
                if let UpdateEvent::InstallFailed(error) = event {
                    this.failed = Some(error.clone());
                    // Fall back to the release page so the user can still download manually.
                    if !this.info.page.is_empty() {
                        cx.open_url(&this.info.page);
                    }
                    cx.notify();
                }
            }),
        ];
        Self {
            state,
            info,
            failed: initial_failed,
            _subscriptions: subscriptions,
        }
    }

    fn render_notes(&self, cx: &App) -> Option<AnyElement> {
        render_notes_markdown("update-notes", &self.info.notes, px(280.), cx)
    }

    fn render_progress(&self, done: u64, total: u64, cx: &App) -> AnyElement {
        let pct = (done * 100).checked_div(total).unwrap_or(0).min(100);
        let muted = cx.theme().muted_foreground;
        let left = if total > 0 {
            format!("{} · {pct}%", t!("update.downloading_progress"))
        } else if let Some(asset) = &self.info.asset {
            t!("update.downloading", name = asset.name, size = format_size(done)).to_string()
        } else {
            t!("update.downloading_progress").to_string()
        };
        let right = if total > 0 {
            format!("{} / {}", format_size(done), format_size(total))
        } else {
            format_size(done)
        };
        v_flex()
            .w_full()
            .gap_1p5()
            .child(Progress::new("update-progress").value(pct as f32))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(div().text_sm().child(left))
                    .child(div().text_sm().text_color(muted).child(right)),
            )
            .into_any_element()
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
                        // A disk image replaces the running app in place and
                        // restarts automatically; anything else is handed to
                        // the system.
                        .label(if asset.kind == "dmg" {
                            t!("update.update_and_restart").to_string()
                        } else {
                            t!("update.download_install").to_string()
                        })
                        .loading(phase == UpdatePhase::Downloading)
                        .disabled(phase != UpdatePhase::Idle)
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            state.update(cx, |state, cx| state.download_and_install(cx));
                        }),
                )
                .into_any_element(),
            None => row.child(open_page("update-page").primary()).into_any_element(),
        }
    }
}

impl Render for UpdatePrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let phase = state.phase();
        let progress = state.download_progress();
        let muted = cx.theme().muted_foreground;
        let status = match (phase, &self.info.asset) {
            (UpdatePhase::Installed, _) => Some(t!("update.installed").to_string()),
            (_, None) => Some(t!("update.no_package").to_string()),
            _ => None,
        };
        let footer = match (phase, progress) {
            (UpdatePhase::Downloading, Some((done, total))) => self.render_progress(done, total, cx),
            _ => self.render_buttons(phase),
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
            .child(footer)
    }
}
