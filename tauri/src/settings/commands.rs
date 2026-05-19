use super::store;
use super::types::AppSettings;
use crate::auto_launch;
use crate::db::SqliteDbState;
use crate::tray;

/// Get settings from database using adapter layer for fault tolerance
#[tauri::command]
pub async fn get_settings(
    sqlite_state: tauri::State<'_, SqliteDbState>,
) -> Result<AppSettings, String> {
    store::load_settings_from_sqlite_state(&sqlite_state)
}

/// Save settings to database using adapter layer
/// Uses UPSERT to handle both create and update
#[tauri::command]
pub async fn save_settings(
    sqlite_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    settings: AppSettings,
) -> Result<(), String> {
    store::save_settings_to_sqlite_state(&sqlite_state, &settings)?;

    if let Err(err) = tray::refresh_tray_menus(&app).await {
        log::warn!("Failed to refresh tray after saving settings: {err}");
    }

    Ok(())
}

/// Normalize a backup custom entry path for portable storage and display.
#[tauri::command]
pub fn normalize_backup_custom_entry_path(path: String) -> String {
    crate::settings::backup::utils::normalize_backup_storage_path(&path)
}

/// Set auto launch on startup
#[tauri::command]
pub fn set_auto_launch(enabled: bool) -> Result<(), String> {
    if enabled {
        auto_launch::enable_auto_launch()
            .map_err(|e| format!("Failed to enable auto launch: {}", e))
    } else {
        auto_launch::disable_auto_launch()
            .map_err(|e| format!("Failed to disable auto launch: {}", e))
    }
}

/// Get auto launch status
#[tauri::command]
pub fn get_auto_launch_status() -> Result<bool, String> {
    auto_launch::is_auto_launch_enabled()
        .map_err(|e| format!("Failed to check auto launch status: {}", e))
}

/// Restart the application
#[tauri::command]
pub fn restart_app() -> Result<(), String> {
    // Get the current executable path
    let current_exe =
        std::env::current_exe().map_err(|e| format!("Failed to get current executable: {}", e))?;

    // Spawn a new instance and exit the current one
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        // Use cmd /c start to spawn a new process and return immediately
        Command::new("cmd")
            .args(&["/c", "start", "", current_exe.to_string_lossy().as_ref()])
            .spawn()
            .map_err(|e| format!("Failed to spawn new process: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        // On macOS, we need to open the .app bundle, not the binary directly.
        // The binary is at: /path/to/App.app/Contents/MacOS/binary
        // We need to get: /path/to/App.app
        let app_bundle = current_exe
            .parent() // Contents/MacOS
            .and_then(|p| p.parent()) // Contents
            .and_then(|p| p.parent()); // App.app

        match app_bundle {
            Some(bundle_path) if bundle_path.extension().map_or(false, |ext| ext == "app") => {
                Command::new("open")
                    .arg("-n") // Open a new instance
                    .arg(bundle_path)
                    .spawn()
                    .map_err(|e| format!("Failed to spawn new process: {}", e))?;
            }
            _ => {
                // Fallback: if not in a bundle, just run the binary directly
                Command::new(&current_exe)
                    .spawn()
                    .map_err(|e| format!("Failed to spawn new process: {}", e))?;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
        Command::new(&current_exe)
            .args(args)
            .env("AI_TOOLBOX_RESTART_WAIT_LOCK", "1")
            .spawn()
            .map_err(|e| format!("Failed to spawn new process: {}", e))?;
    }

    // Exit the current instance
    std::process::exit(0);
}

/// Test proxy connection
#[tauri::command]
pub async fn test_proxy_connection(proxy_url: String) -> Result<(), String> {
    crate::http_client::test_proxy(&proxy_url).await
}
