//! The standalone About dialog (R3): version, Git commit with copy button,
//! build time, platform/architecture, data model version, license, GitHub
//! homepage link, and desktop update check trigger.

use crate::APP_NAME;
#[cfg(not(target_family = "wasm"))]
use crate::states::{AppStore, UpdatePhase};
use crate::this_build;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, ClipboardItem, Image, ImageFormat, ImageSource, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window, div, img, px, rems,
};
use rust_i18n::t;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

fn app_icon() -> Arc<Image> {
    static ICON: OnceLock<Arc<Image>> = OnceLock::new();
    ICON.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../resources/icons/128x128@2x.png").to_vec(),
        ))
    })
    .clone()
}

/// Opens the standalone About dialog when no other dialog is active on `window`.
pub fn open_about_dialog(window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let copied = Rc::new(Cell::new(false));
    let icon = app_icon();
    window.open_dialog(cx, move |dialog, _, cx| {
        let copied = copied.clone();
        let icon = icon.clone();
        let theme = cx.theme();
        let (muted, border, card_bg) = (theme.muted_foreground, theme.border, theme.secondary.opacity(0.35));
        let repo = env!("CARGO_PKG_REPOSITORY");

        let header = h_flex()
            .gap_3()
            .items_center()
            .child(
                img(ImageSource::Image(icon))
                    .w(px(52.))
                    .h(px(52.))
                    .rounded_lg()
                    .flex_none(),
            )
            .child(
                v_flex()
                    .gap_0p5()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_base().font_semibold().child(APP_NAME))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(t!("about.description").to_string()),
                    ),
            );

        let row = |label: String, value: AnyElement| -> AnyElement {
            h_flex()
                .gap_3()
                .py_1()
                .items_center()
                .justify_between()
                .text_xs()
                .child(div().w(rems(6.5)).flex_none().text_color(muted).child(label))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .justify_end()
                        .items_center()
                        .gap_1()
                        .child(value),
                )
                .into_any_element()
        };

        let text_val = |value: SharedString| div().font_medium().truncate().child(value).into_any_element();

        let mut rows: Vec<AnyElement> = vec![row(
            t!("about.version").to_string(),
            text_val(cyberapi_core::version::VERSION.into()),
        )];

        if let Some(commit) = cyberapi_core::version::COMMIT {
            let is_copied = copied.get();
            let copied_flag = copied.clone();
            let copy_tip = if is_copied {
                t!("about.copied").to_string()
            } else {
                t!("about.copy_commit").to_string()
            };
            let commit_val = h_flex()
                .gap_1()
                .items_center()
                .child(div().font_medium().child(commit))
                .child(
                    Button::new("about-copy-commit")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(if is_copied { IconName::Check } else { IconName::Copy }))
                        .tooltip(copy_tip)
                        .on_click(move |_, window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(commit.to_string()));
                            copied_flag.set(true);
                            window.refresh();
                        }),
                )
                .into_any_element();
            rows.push(row(t!("about.commit").to_string(), commit_val));
        }

        if let Some(build_time) = cyberapi_core::version::BUILD_TIME {
            rows.push(row(t!("about.build_time").to_string(), text_val(build_time.into())));
        }

        rows.push(row(
            t!("about.platform").to_string(),
            text_val(this_build::platform().into()),
        ));

        if let Some(os_version) = this_build::os_version() {
            rows.push(row(t!("about.os_version").to_string(), text_val(os_version.into())));
        }

        rows.push(row(
            t!("about.model").to_string(),
            text_val(format!("v{}", cyberapi_core::version::MODEL).into()),
        ));

        rows.push(row(
            t!("about.license").to_string(),
            text_val(env!("CARGO_PKG_LICENSE").into()),
        ));

        let card = v_flex()
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(card_bg)
            .children(rows);

        let links = h_flex()
            .gap_2()
            .items_center()
            .justify_between()
            .child(
                Button::new("about-homepage")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::ExternalLink))
                    .label(t!("about.homepage").to_string())
                    .on_click(move |_, _, cx| cx.open_url(repo)),
            )
            .children(desktop_update_action(cx));

        dialog
            .title(t!("about.title").to_string())
            .w(px(420.))
            .child(super::dialog_body(
                v_flex().gap_3().child(header).child(card).child(links),
            ))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.close").to_string()))),
            )
            .on_ok(|_, _, _| false)
    });
}

#[cfg(not(target_family = "wasm"))]
fn desktop_update_action(cx: &App) -> Option<AnyElement> {
    let updates = cx.try_global::<AppStore>().and_then(|store| store.updates().cloned())?;
    let state = updates.read(cx);
    let phase = state.phase();
    if let Some(info) = state.available() {
        let label = t!("update.found", version = info.version).to_string();
        return Some(
            Button::new("about-update-available")
                .primary()
                .small()
                .label(label)
                .on_click(|_, window, cx| {
                    window.close_dialog(cx);
                    super::update_dialog::open_update_dialog(window, cx);
                })
                .into_any_element(),
        );
    }
    let checking = matches!(phase, UpdatePhase::Checking { .. });
    Some(
        Button::new("about-check-updates")
            .outline()
            .small()
            .loading(checking)
            .label(if checking {
                t!("update.checking").to_string()
            } else {
                t!("about.check_updates").to_string()
            })
            .on_click(move |_, window, cx| {
                window.close_dialog(cx);
                updates.update(cx, |updates, cx| updates.check(true, cx));
            })
            .into_any_element(),
    )
}

#[cfg(target_family = "wasm")]
fn desktop_update_action(_cx: &App) -> Option<AnyElement> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_metadata_is_present() {
        assert!(!cyberapi_core::version::VERSION.is_empty());
        assert!(cyberapi_core::version::BUILD_TIME.is_some());
        assert!(!this_build::platform().is_empty());
        assert!(!env!("CARGO_PKG_LICENSE").is_empty());
        assert!(!env!("CARGO_PKG_REPOSITORY").is_empty());
    }
}
