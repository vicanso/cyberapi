//! The pages and dialogs (L4). Views draw what the states hold and forward
//! what the reader asks for; they never call a store.

mod access_dialog;
mod accounts_dialog;
mod cookies_dialog;
mod dashboard;
mod group_dialog;
mod inline_confirm;
mod invites;
mod json_tree;
mod kv_editor;
mod marked;
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
mod variables_dialog;
mod workspace;

pub use access_dialog::open_access_dialog;
pub use accounts_dialog::open_accounts_dialog;
pub use cookies_dialog::open_cookies_dialog;
pub use dashboard::DashboardView;
pub use dashboard::{bind_keys as bind_dashboard_keys, bind_mac_page_keys as bind_dashboard_mac_page_keys};
pub use password_dialog::open_password_dialog;
#[cfg(not(any(target_os = "macos", target_family = "wasm")))]
pub use settings::is_settings_window;
pub use settings::open_settings;
pub use shortcuts_dialog::open_shortcuts_dialog;
pub use startup::StartupError;
pub use teams_dialog::open_teams_dialog;
pub use variables_dialog::open_variables_dialog;
pub use workspace::{WorkspaceView, bind_keys as bind_workspace_keys, bind_mac_page_keys};

use cyberapi_core::format::JSON_INDENT;
use gpui_kit::component::input::TabSize;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ColorName, Sizable as _};

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
