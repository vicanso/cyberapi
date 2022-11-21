#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use tracing_subscriber::prelude::*;

mod commands;
mod cookies;
mod entities;
mod error;
mod http_request;
mod schemas;
mod util;

fn main() {
    tracing_subscriber::registry()
        .with(http_request::HTTPTraceLayer)
        .init();
    // // a builder for `FmtSubscriber`.
    // let subscriber = FmtSubscriber::builder()
    //     // all spans/events with a level higher than TRACE (e.g, debug, info, warn, etc.)
    //     // will be written to stdout.
    //     .with_max_level(Level::TRACE)
    //     // completes the builder.
    //     .finish();

    // tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let context = tauri::generate_context!();
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path_resolver().app_data_dir().unwrap();
            util::set_app_dir(dir.to_str().unwrap().to_string());
            Ok(())
        })
        .menu(tauri::Menu::os_default(&context.package_info().name))
        .invoke_handler(tauri::generate_handler![
            commands::close_splashscreen,
            commands::init_tables,
            commands::export_tables,
            commands::import_tables,
            commands::add_api_setting,
            commands::update_api_setting,
            commands::list_api_setting,
            commands::delete_api_settings,
            commands::add_api_folder,
            commands::update_api_folder,
            commands::list_api_folder,
            commands::delete_api_folder,
            commands::add_api_collection,
            commands::update_api_collection,
            commands::list_api_collection,
            commands::delete_api_collection,
            commands::do_http_request,
            commands::list_cookie,
            commands::delete_cookie,
            commands::add_cookie,
            commands::clear_cookie,
            commands::add_variable,
            commands::update_variable,
            commands::delete_variable,
            commands::list_variable,
            commands::get_latest_version,
            commands::add_version,
        ])
        .run(context)
        .expect("error while running tauri application");
}
