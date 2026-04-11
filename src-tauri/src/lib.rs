mod classifier;
mod commands;
mod models;
mod slack;
mod storage;

use commands::AppState;
use std::sync::Arc;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let data_dir = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let app_dir = data_dir.join("dispatch");

    // Migrate from old "haystack" data directory if it exists
    let old_app_dir = data_dir.join("haystack");
    if old_app_dir.exists() && !app_dir.exists() {
        std::fs::rename(&old_app_dir, &app_dir)
            .expect("Failed to migrate haystack data directory to dispatch");
    }

    std::fs::create_dir_all(&app_dir).expect("Failed to create app data directory");

    // Migrate old database filename if it exists
    let old_db_path = app_dir.join("haystack.db");
    let db_path = app_dir.join("dispatch.db");
    if old_db_path.exists() && !db_path.exists() {
        std::fs::rename(&old_db_path, &db_path)
            .expect("Failed to migrate haystack.db to dispatch.db");
    }
    let db = storage::Database::new(db_path.to_str().expect("Database path contains invalid UTF-8"))
        .expect("Failed to initialize database");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState {
            db: Arc::new(db),
        })
        .setup(|app| {
            // App submenu
            let about_item =
                MenuItem::with_id(app, "about", "About Dispatch", true, None::<&str>)?;
            let check_updates_item =
                MenuItem::with_id(app, "check_updates", "Check for Updates...", true, None::<&str>)?;
            let sep_about = PredefinedMenuItem::separator(app)?;
            let settings_item =
                MenuItem::with_id(app, "settings", "Settings...", true, Some("CmdOrCtrl+Comma"))?;
            let separator = PredefinedMenuItem::separator(app)?;
            let hide = PredefinedMenuItem::hide(app, None)?;
            let hide_others = PredefinedMenuItem::hide_others(app, None)?;
            let show_all = PredefinedMenuItem::show_all(app, None)?;
            let sep_quit = PredefinedMenuItem::separator(app)?;
            let quit = PredefinedMenuItem::quit(app, Some("Quit Dispatch"))?;
            let app_submenu = Submenu::with_items(
                app,
                "Dispatch",
                true,
                &[&about_item, &check_updates_item, &sep_about, &settings_item, &separator, &hide, &hide_others, &show_all, &sep_quit, &quit],
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
                } else if event.id() == "about" {
                    let _ = app_handle.emit("open-about", ());
                } else if event.id() == "check_updates" {
                    let handle = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = handle.emit("update-checking", ());
                        let updater = match handle.updater() {
                            Ok(u) => u,
                            Err(e) => {
                                eprintln!("Updater init failed: {}", e);
                                let _ = handle.emit("update-error", e.to_string());
                                return;
                            }
                        };
                        match updater.check().await {
                            Ok(Some(update)) => {
                                let _ = handle.emit("update-available", update.version.clone());
                                match update.download_and_install(|_, _| {}, || {}).await {
                                    Ok(_) => {
                                        let _ = handle.emit("update-installed", update.version.clone());
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to install update: {}", e);
                                        let _ = handle.emit("update-error", e.to_string());
                                    }
                                }
                            }
                            Ok(None) => {
                                let _ = handle.emit("no-update", ());
                            }
                            Err(e) => {
                                eprintln!("Update check failed: {}", e);
                                let _ = handle.emit("update-error", e.to_string());
                            }
                        }
                    });
                }
            });

            // Set app icon at runtime (dev builds don't have a proper .app bundle)
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

            // Background snooze checker: every 30s, unsnooze due messages and notify
            let db_for_snooze = Arc::clone(&app.state::<AppState>().db);
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    match db_for_snooze.unsnooze_due_messages() {
                        Ok(count) if count > 0 => {
                            let enabled = db_for_snooze
                                .get_setting("notifications_enabled")
                                .ok()
                                .flatten()
                                .map(|v| v == "true")
                                .unwrap_or(true);
                            if enabled {
                                let (title, body) = if count == 1 {
                                    ("Snoozed message returned".to_string(), "A snoozed message is back in your inbox".to_string())
                                } else {
                                    (format!("{} snoozed messages returned", count), format!("{} snoozed messages are back in your inbox", count))
                                };
                                let _ = app_handle.notification().builder().title(&title).body(&body).show();
                            }
                            let _ = app_handle.emit("snooze-returned", count);
                        }
                        _ => {}
                    }
                }
            });

            // Background update checker: wait 5s after startup, then check for updates
            let update_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let updater = match update_handle.updater() {
                    Ok(u) => u,
                    Err(e) => { eprintln!("Updater init failed: {}", e); return; }
                };
                match updater.check().await {
                    Ok(Some(update)) => {
                        let version = update.version.clone();
                        let _ = update_handle.emit("update-available", version.clone());
                        if update.download_and_install(|_, _| {}, || {}).await.is_ok() {
                            let _ = update_handle.emit("update-installed", version);
                        }
                    }
                    _ => {}
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_messages,
            commands::get_messages_by_status,
            commands::get_starred_messages,
            commands::get_message_counts,
            commands::refresh_inbox,
            commands::mark_done_message,
            commands::snooze_message,
            commands::star_message,
            commands::open_link,
            commands::get_settings,
            commands::save_settings,
            commands::populate_slack_cache,
            commands::search_slack_users,
            commands::search_slack_channels,
            commands::get_slack_cache_status,
            commands::set_window_theme,
            commands::test_slack_connection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
