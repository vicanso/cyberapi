//! The pages and dialogs (L4). Views draw what the states hold and forward
//! what the reader asks for; they never call a store.

mod dashboard;
mod name_dialog;
mod startup;
mod workspace;

pub use dashboard::DashboardView;
pub use startup::StartupError;
pub use workspace::{WorkspaceView, bind_keys as bind_workspace_keys};
