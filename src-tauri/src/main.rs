#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
// mod database;
mod schemas;
mod util;

fn main() {
    let context = tauri::generate_context!();
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path_resolver().app_dir().unwrap();
            util::set_app_dir(dir.to_str().unwrap().to_string());
            Ok(())
        })
        .menu(tauri::Menu::os_default(&context.package_info().name))
        .invoke_handler(tauri::generate_handler![
            commands::close_splashscreen,
            commands::add_api_setting,
            commands::update_api_setting,
            commands::list_api_setting,
            commands::add_api_folder,
            commands::update_api_folder,
            commands::list_api_folder,
        ])
        .run(context)
        .expect("error while running tauri application");
}
