//! The pages and dialogs (L4). Views draw what the states hold and forward
//! what the reader asks for; they never call a store.

mod access_dialog;
mod accounts_dialog;
mod cookies_dialog;
mod dashboard;
mod fixed_rem;
mod graphql_hints;
mod group_dialog;
mod inline_confirm;
mod invites;
mod json_tree;
mod kv_editor;
mod marked;
mod mcp_dialog;
mod name_dialog;
mod password_dialog;
mod pick;
mod server_section;
mod settings;
mod shortcuts_dialog;
mod startup;
mod sync_dialog;
mod teams_dialog;
mod template_hints;
mod update_dialog;
mod variables_dialog;
mod workspace;

pub use access_dialog::open_access_dialog;

/// What copying a request as code says: a gRPC call's is grpcurl's
/// whatever was asked for (#69).
pub fn copied_code(target: cyberapi_core::code::CodeTarget, code: &str) -> String {
    use cyberapi_core::code::CodeTarget;
    if code.starts_with("grpcurl ") {
        rust_i18n::t!("response.grpcurl_copied").to_string()
    } else if target == CodeTarget::Curl {
        rust_i18n::t!("response.curl_copied").to_string()
    } else {
        rust_i18n::t!("response.code_copied", what = target.label()).to_string()
    }
}
pub use accounts_dialog::open_accounts_dialog;
pub use cookies_dialog::open_cookies_dialog;
pub use dashboard::DashboardView;
pub use dashboard::{bind_keys as bind_dashboard_keys, bind_mac_page_keys as bind_dashboard_mac_page_keys};
pub use mcp_dialog::open_mcp_dialog;
pub use password_dialog::open_password_dialog;
#[cfg(not(any(target_os = "macos", target_family = "wasm")))]
pub use settings::is_settings_window;
pub use settings::open_settings;
pub use shortcuts_dialog::open_shortcuts_dialog;
pub use startup::StartupError;
pub use teams_dialog::open_teams_dialog;
pub use update_dialog::{follow_update, update_chip};
pub use variables_dialog::open_variables_dialog;
#[cfg(not(any(target_os = "macos", target_family = "wasm")))]
pub use workspace::is_apart_window;
pub use workspace::{
    OpenPalette, ROOT_CONTEXT, WorkspaceView, bind_keys as bind_workspace_keys, bind_mac_page_keys, bind_palette_keys,
    bind_palette_mac_page_keys, open_palette, open_project_auth,
};
#[cfg(not(target_family = "wasm"))]
pub use workspace::{close_apart, open_apart};

use cyberapi_core::format::JSON_INDENT;
use gpui_kit::component::input::TabSize;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, ColorName, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Action, AnyElement, App, InteractiveElement as _, IntoElement as _, KeyBinding, KeyBindingContextPredicate,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
};

/// A `secondary-` binding: Command on macOS, Control elsewhere.
///
/// gpui-pre 0.3.10's `KeyBinding::new` parses with `Keystroke::parse`, which
/// rejects that alias. `KeyBinding::load` resolves it from the host. Explicit
/// `cmd-` / `ctrl-` bindings stay on `KeyBinding::new`.
pub(crate) fn secondary_binding(cx: &App, keystrokes: &str, action: impl Action, context: Option<&str>) -> KeyBinding {
    let context = context.map(|context| {
        KeyBindingContextPredicate::parse(context)
            .unwrap_or_else(|err| panic!("key context {context:?}: {err}"))
            .into()
    });
    KeyBinding::load(keystrokes, Box::new(action), context, false, None, cx)
        .unwrap_or_else(|err| panic!("key binding {keystrokes:?}: {err}"))
}

/// The Tab of an editor that holds JSON: as many spaces as the JSON written
/// everywhere is indented by.
fn json_tab() -> TabSize {
    TabSize {
        tab_size: JSON_INDENT,
        hard_tabs: false,
    }
}

/// The word that a project has changes the server has not had
/// (`docs/server-sync.md` §3.2), on a card and in the title bar. Amber text
/// on a wash of it: the solid warning tag is white on yellow in the light
/// theme and dark on yellow in the dark one, hard to read in both.
fn unsynced_tag() -> Tag {
    Tag::color(ColorName::Amber).small().rounded_full()
}

/// A dialog's body, with room above and below it: the dialog clips its
/// body at the body's edges, and a field there lost the bottom of the ring
/// it draws when focused (gpui-component 0.8).
fn dialog_body(body: impl gpui_kit::IntoElement) -> gpui_kit::Div {
    v_flex().py_1().child(body)
}

/// Where a server project's sync button is, while its server cannot be
/// reached: that it cannot, why in the tooltip.
fn unreachable_note(id: SharedString, reason: &str, cx: &App) -> AnyElement {
    let hint = rust_i18n::t!("sync.unreachable_hint", reason = reason).to_string();
    let muted = cx.theme().muted_foreground;
    h_flex()
        .id(id)
        .flex_none()
        .gap_1()
        .items_center()
        .text_xs()
        .text_color(muted)
        .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
        .child(
            Icon::new(gpui_kit::assets::IconName::CloudOff)
                .xsmall()
                .text_color(muted),
        )
        .child(rust_i18n::t!("sync.unreachable").to_string())
        .into_any_element()
}
