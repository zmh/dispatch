mod classifier;
mod commands;
mod models;
mod slack;
mod storage;

use commands::AppState;
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::Emitter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_dir = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("haystack");
    std::fs::create_dir_all(&app_dir).expect("Failed to create app data directory");

    let db_path = app_dir.join("haystack.db");
    let db = storage::Database::new(db_path.to_str().unwrap())
        .expect("Failed to initialize database");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            db: Arc::new(db),
        })
        .setup(|app| {
            // App submenu
            let settings_item =
                MenuItem::with_id(app, "settings", "Settings...", true, Some("CmdOrCtrl+Comma"))?;
            let separator = PredefinedMenuItem::separator(app)?;
            let quit = PredefinedMenuItem::quit(app, Some("Quit Haystack"))?;
            let app_submenu = Submenu::with_items(
                app,
                "Haystack",
                true,
                &[&settings_item, &separator, &quit],
            )?;

            // Edit submenu (standard macOS text editing)
            let undo = PredefinedMenuItem::undo(app, None)?;
            let redo = PredefinedMenuItem::redo(app, None)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let cut = PredefinedMenuItem::cut(app, None)?;
            let copy = PredefinedMenuItem::copy(app, None)?;
            let paste = PredefinedMenuItem::paste(app, None)?;
            let select_all = PredefinedMenuItem::select_all(app, None)?;
            let edit_submenu = Submenu::with_items(
                app,
                "Edit",
                true,
                &[&undo, &redo, &sep2, &cut, &copy, &paste, &select_all],
            )?;

            // Window submenu
            let minimize = PredefinedMenuItem::minimize(app, None)?;
            let fullscreen = PredefinedMenuItem::fullscreen(app, None)?;
            let window_submenu = Submenu::with_items(
                app,
                "Window",
                true,
                &[&minimize, &fullscreen],
            )?;

            let menu = Menu::with_items(app, &[&app_submenu, &edit_submenu, &window_submenu])?;
            app.set_menu(menu)?;

            app.on_menu_event(|app_handle, event| {
                if event.id() == "settings" {
                    let _ = app_handle.emit("open-settings", ());
                }
            });

            // Set squircle app icon at runtime (macOS doesn't mask unsigned dev builds)
            #[cfg(target_os = "macos")]
            {
                use cocoa::base::id;
                use objc::{class, msg_send, sel, sel_impl};
                let icon_data = include_bytes!("../icons/icon.png");
                unsafe {
                    let data: id = msg_send![class!(NSData), dataWithBytes:icon_data.as_ptr() length:icon_data.len()];
                    let image: id = msg_send![class!(NSImage), alloc];
                    let image: id = msg_send![image, initWithData: data];
                    let ns_app: id = msg_send![class!(NSApplication), sharedApplication];
                    let _: () = msg_send![ns_app, setApplicationIconImage: image];
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_messages,
            commands::get_message_counts,
            commands::refresh_inbox,
            commands::archive_message,
            commands::snooze_message,
            commands::star_message,
            commands::open_link,
            commands::get_settings,
            commands::save_settings,
            commands::populate_slack_cache,
            commands::search_slack_users,
            commands::search_slack_channels,
            commands::get_slack_cache_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
