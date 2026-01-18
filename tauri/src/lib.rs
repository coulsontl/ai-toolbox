#[allow(unused_imports)]
use tauri::{Listener, Manager};

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
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

/// Open a folder in the system file manager
/// If the path is a file, opens the parent directory
/// Creates the directory if it doesn't exist
#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    let path = Path::new(&path);

    // Determine the folder to open
    let folder = if path.is_file() {
        path.parent()
            .ok_or_else(|| "Cannot get parent directory".to_string())?
    } else {
        path
    };

    // Create directory if it doesn't exist
    if !folder.exists() {
        fs::create_dir_all(folder)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    // Open the folder using system default file manager
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(folder)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(folder)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(folder)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // When a second instance is launched, show and focus the existing window
            if let Some(window) = app.get_webview_window("main") {
                // macOS: Switch back to Regular mode to show in Dock
                #[cfg(target_os = "macos")]
                {
                    use tauri::ActivationPolicy;
                    let _ = app.set_activation_policy(ActivationPolicy::Regular);
                }
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
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

                // Initialize default provider models in database
                let db_state = DbState(Arc::new(Mutex::new(db.clone())));
                coding::open_code::free_models::init_default_provider_models(&db_state)
                    .await
                    .expect("Failed to initialize default provider models");

                // Initialize Claude Code provider from settings.json if database is empty
                coding::claude_code::commands::init_claude_provider_from_settings(&db)
                    .await
                    .expect("Failed to initialize Claude Code provider from settings");

                coding::codex::commands::init_codex_provider_from_settings(&db)
                    .await
                    .expect("Failed to initialize Codex provider from settings");

                app.manage(db_state);
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

            // Listen for WSL sync requests (Windows only)
            #[cfg(target_os = "windows")]
            {
                // OpenCode sync listener
                let app1 = app_handle.clone();
                let app1_clone = app1.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = app1.listen("wsl-sync-request-opencode", move |_event| {
                        let app = app1_clone.clone();
                        // Spawn background task without awaiting
                        tauri::async_runtime::spawn(async move {
                            // Re-obtain state inside the spawned task
                            let db_state = app.state::<crate::DbState>();
                            let result = coding::wsl::wsl_sync(db_state, app.clone(), Some("opencode".to_string())).await;
                            // Ignore result - fire and forget
                            let _ = result;
                        });
                    });

                    // Keep this async block alive forever to prevent listener from being dropped
                    std::future::pending::<()>().await;
                });

                // Claude sync listener
                let app2 = app_handle.clone();
                let app2_clone = app2.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = app2.listen("wsl-sync-request-claude", move |_event| {
                        let app = app2_clone.clone();
                        // Spawn background task without awaiting
                        tauri::async_runtime::spawn(async move {
                            // Re-obtain state inside the spawned task
                            let db_state = app.state::<crate::DbState>();
                            let result = coding::wsl::wsl_sync(db_state, app.clone(), Some("claude".to_string())).await;
                            // Ignore result - fire and forget
                            let _ = result;
                        });
                    });

                    // Keep this async block alive forever to prevent listener from being dropped
                    std::future::pending::<()>().await;
                });

                // Codex sync listener
                let app3 = app_handle.clone();
                let app3_clone = app3.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = app3.listen("wsl-sync-request-codex", move |_event| {
                        let app = app3_clone.clone();
                        // Spawn background task without awaiting
                        tauri::async_runtime::spawn(async move {
                            // Re-obtain state inside the spawned task
                            let db_state = app.state::<crate::DbState>();
                            let result = coding::wsl::wsl_sync(db_state, app.clone(), Some("codex".to_string())).await;
                            // Ignore result - fire and forget
                            let _ = result;
                        });
                    });

                    // Keep this async block alive forever to prevent listener from being dropped
                    std::future::pending::<()>().await;
                });
            }

            #[cfg(target_os = "windows")]
            // WSL sync on app startup (delayed to avoid blocking startup)
            {
                let app_clone = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    let db_state = app_clone.state::<crate::DbState>();
                    let app = app_clone.clone();
                    let _ = coding::wsl::wsl_sync(db_state, app, None).await;
                });
            }

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
                        
                        // macOS: Switch to Accessory mode to hide from Dock
                        #[cfg(target_os = "macos")]
                        {
                            use tauri::ActivationPolicy;
                            let _ = app_handle.set_activation_policy(ActivationPolicy::Accessory);
                        }
                    }
                    // Prevent default close behavior
                    api.prevent_close();
                }
                // If minimize_to_tray is false, do nothing - window will close normally
            }
        })
        .invoke_handler(tauri::generate_handler![
            // Common
            open_folder,
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
            coding::open_code::get_opencode_free_models,
            coding::open_code::get_provider_models,
            coding::open_code::get_opencode_unified_models,
            coding::open_code::get_opencode_auth_providers,
            coding::open_code::get_opencode_auth_config_path,
            coding::open_code::backup_opencode_config,
            // Codex
            coding::codex::get_codex_config_dir_path,
            coding::codex::get_codex_config_file_path,
            coding::codex::reveal_codex_config_folder,
            coding::codex::list_codex_providers,
            coding::codex::create_codex_provider,
            coding::codex::update_codex_provider,
            coding::codex::delete_codex_provider,
            coding::codex::repair_codex_providers,
            coding::codex::reorder_codex_providers,
            coding::codex::select_codex_provider,
            coding::codex::apply_codex_config,
            coding::codex::read_codex_settings,
            coding::codex::get_codex_common_config,
            coding::codex::save_codex_common_config,
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
            coding::oh_my_opencode::check_oh_my_opencode_config_exists,
            // WSL Sync
            coding::wsl::wsl_detect,
            coding::wsl::wsl_check_distro,
            coding::wsl::wsl_get_config,
            coding::wsl::wsl_save_config,
            coding::wsl::wsl_add_file_mapping,
            coding::wsl::wsl_update_file_mapping,
            coding::wsl::wsl_delete_file_mapping,
            coding::wsl::wsl_reset_file_mappings,
            coding::wsl::wsl_sync,
            coding::wsl::wsl_get_status,
            coding::wsl::wsl_test_path,
            coding::wsl::wsl_get_default_mappings,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Handle macOS dock icon click when app is hidden
            #[cfg(target_os = "macos")]
            {
                if let tauri::RunEvent::Reopen { .. } = event {
                    use tauri::ActivationPolicy;
                    // Switch back to Regular mode to show in Dock
                    let _ = app_handle.set_activation_policy(ActivationPolicy::Regular);
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
            }
            
            // Suppress unused variable warnings on non-macOS platforms
            #[cfg(not(target_os = "macos"))]
            {
                let _ = app_handle;
                let _ = event;
            }
        });
}
