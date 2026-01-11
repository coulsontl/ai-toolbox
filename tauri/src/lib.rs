#[allow(unused_imports)]
use tauri::{Listener, Manager};

use std::fs;
use std::sync::Arc;
use surrealdb::engine::local::SurrealKv;
use surrealdb::Surreal;
use tokio::sync::Mutex;

// Module declarations
pub mod auto_launch;
pub mod coding;
pub mod db;
pub mod http_client;
pub mod settings;
pub mod tray;
pub mod update;

// Re-export DbState for use in other modules
pub use db::DbState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let app_handle = app.handle().clone();

            // Create app data directory
            let app_data_dir = app_handle
                .path()
                .app_data_dir()
                .expect("Failed to get app data dir");

            if !app_data_dir.exists() {
                fs::create_dir_all(&app_data_dir).expect("Failed to create app data dir");
            }

            let db_path = app_data_dir.join("database");

            // Initialize SurrealDB
            tauri::async_runtime::block_on(async {
                let db = Surreal::new::<SurrealKv>(db_path)
                    .await
                    .expect("Failed to initialize SurrealDB");

                db.use_ns("ai_toolbox")
                    .use_db("main")
                    .await
                    .expect("Failed to select namespace and database");

                // Run database migrations
                db::run_migrations(&db)
                    .await
                    .expect("Failed to run database migrations");

                app.manage(DbState(Arc::new(Mutex::new(db))));
            });
            
            // Create system tray
            tray::create_tray(&app_handle).expect("Failed to create system tray");

            // Listen for config changes to refresh tray menu
            let app_handle_clone = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let value = app_handle_clone.clone();
                let value_for_closure = value.clone();
                let _listener = value.listen("config-changed", move |_event| {
                    let app = value_for_closure.app_handle().clone();
                    let _ = tauri::async_runtime::spawn(async move {
                        let _ = tray::refresh_tray_menus(&app).await;
                    });
                });
                
                // Keep this async block alive forever to prevent listener from being dropped
                std::future::pending::<()>().await;
            });
            
            // Enable auto-launch if setting is true
            let app_handle_clone = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let db_state = app_handle_clone.state::<DbState>();
                let db = db_state.0.lock().await;
                
                let mut result = db
                    .query("SELECT * OMIT id FROM settings:`app` LIMIT 1")
                    .await
                    .ok();
                
                if let Some(ref mut res) = result {
                    let records: Result<Vec<serde_json::Value>, _> = res.take(0);
                    if let Ok(records) = records {
                        if let Some(record) = records.first() {
                            let launch_on_startup = record
                                .get("launch_on_startup")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(true);
                            
                            if launch_on_startup {
                                let _ = auto_launch::enable_auto_launch();
                            }
                        }
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app_handle = window.app_handle().clone();
                
                // Check minimize_to_tray_on_close setting with default value
                let minimize_to_tray = {
                    let db_state = app_handle.state::<DbState>();
                    let db = db_state.0.blocking_lock();
                    
                    // Query settings synchronously using block_on
                    let query_result = tauri::async_runtime::block_on(async {
                        db.query("SELECT * OMIT id FROM settings:`app` LIMIT 1").await
                    });
                    
                    match query_result {
                        Ok(mut res) => {
                            let records: Result<Vec<serde_json::Value>, surrealdb::Error> = res.take(0);
                            match records {
                                Ok(records) => {
                                    if let Some(record) = records.first() {
                                        record
                                            .get("minimize_to_tray_on_close")
                                            .and_then(|v| v.as_bool())
                                            .unwrap_or(true)
                                    } else {
                                        true
                                    }
                                }
                                Err(_) => true,
                            }
                        }
                        Err(_) => true,
                    }
                };
                
                if minimize_to_tray {
                    // Hide window instead of closing
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.hide();
                        
                        // macOS: Hide dock icon
                        #[cfg(target_os = "macos")]
                        {
                            let _ = app_handle.hide();
                        }
                    }
                    // Prevent default close behavior
                    api.prevent_close();
                }
                // If minimize_to_tray is false, do nothing - window will close normally
            }
        })
        .invoke_handler(tauri::generate_handler![
            // Update
            update::check_for_updates,
            update::install_update,
            // Settings
            settings::get_settings,
            settings::save_settings,
            settings::set_auto_launch,
            settings::get_auto_launch_status,
            settings::restart_app,
            settings::test_proxy_connection,
            // Backup - Local
            settings::backup::backup_database,
            settings::backup::restore_database,
            settings::backup::get_database_path,
            settings::backup::open_app_data_dir,
            // Backup - WebDAV
            settings::backup::backup_to_webdav,
            settings::backup::list_webdav_backups,
            settings::backup::restore_from_webdav,
            // Claude Code
            coding::claude_code::list_claude_providers,
            coding::claude_code::create_claude_provider,
            coding::claude_code::update_claude_provider,
            coding::claude_code::delete_claude_provider,
            coding::claude_code::reorder_claude_providers,
            coding::claude_code::select_claude_provider,
            coding::claude_code::get_claude_config_path,
            coding::claude_code::reveal_claude_config_folder,
            coding::claude_code::read_claude_settings,
            coding::claude_code::apply_claude_config,
            coding::claude_code::get_claude_common_config,
            coding::claude_code::save_claude_common_config,
            coding::claude_code::get_claude_plugin_status,
            coding::claude_code::apply_claude_plugin_config,
            // OpenCode
            coding::open_code::get_opencode_config_path,
            coding::open_code::get_opencode_config_path_info,
            coding::open_code::read_opencode_config,
            coding::open_code::save_opencode_config,
            coding::open_code::get_opencode_common_config,
            coding::open_code::save_opencode_common_config,
            coding::open_code::fetch_provider_models,
            // Tray
            tray::refresh_tray_menu,
            // Oh My OpenCode
            coding::oh_my_opencode::list_oh_my_opencode_configs,
            coding::oh_my_opencode::create_oh_my_opencode_config,
            coding::oh_my_opencode::update_oh_my_opencode_config,
            coding::oh_my_opencode::delete_oh_my_opencode_config,
            coding::oh_my_opencode::apply_oh_my_opencode_config,
            coding::oh_my_opencode::reorder_oh_my_opencode_configs,
            coding::oh_my_opencode::get_oh_my_opencode_config_path_info,
            coding::oh_my_opencode::get_oh_my_opencode_global_config,
            coding::oh_my_opencode::save_oh_my_opencode_global_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
