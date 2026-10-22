//! The pages and dialogs (L4). Views draw what the states hold and forward
//! what the reader asks for; they never call a store.

mod access_dialog;
mod accounts_dialog;
mod cookies_dialog;
mod dashboard;
mod group_dialog;
mod inline_confirm;
mod json_tree;
mod kv_editor;
mod marked;
mod name_dialog;
mod password_dialog;
mod pick;
mod settings_dialog;
mod shortcuts_dialog;
mod startup;
mod teams_dialog;
mod variables_dialog;
mod workspace;

pub use access_dialog::open_access_dialog;
pub use accounts_dialog::open_accounts_dialog;
pub use cookies_dialog::open_cookies_dialog;
pub use dashboard::DashboardView;
pub use dashboard::{bind_keys as bind_dashboard_keys, bind_mac_page_keys as bind_dashboard_mac_page_keys};
pub use password_dialog::open_password_dialog;
pub use settings_dialog::open_settings_dialog;
pub use shortcuts_dialog::open_shortcuts_dialog;
pub use startup::StartupError;
pub use teams_dialog::open_teams_dialog;
pub use variables_dialog::open_variables_dialog;
pub use workspace::{WorkspaceView, bind_keys as bind_workspace_keys, bind_mac_page_keys};
