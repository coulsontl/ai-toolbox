use crate::db::DbState;
use crate::auto_launch;
use super::adapter;
use super::types::AppSettings;

/// Get settings from database using adapter layer for fault tolerance
#[tauri::command]
pub async fn get_settings(state: tauri::State<'_, DbState>) -> Result<AppSettings, String> {
    let db = state.0.lock().await;

    // Use OMIT to exclude 'id' field which contains Thing type
    let mut result = db
        .query("SELECT * OMIT id FROM settings:`app` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query settings: {}", e))?;

    let records: Vec<serde_json::Value> = result
        .take(0)
        .map_err(|e| format!("Failed to parse settings: {}", e))?;

    if let Some(record) = records.first() {
        Ok(adapter::from_db_value(record.clone()))
    } else {
        // No settings found, use defaults
        Ok(AppSettings::default())
    }
}

/// Save settings to database using adapter layer
/// Uses DELETE + CREATE to completely bypass version conflicts
#[tauri::command]
pub async fn save_settings(
    state: tauri::State<'_, DbState>,
    settings: AppSettings,
) -> Result<(), String> {
    let db = state.0.lock().await;

    // Convert to JSON using adapter
    let json = adapter::to_db_value(&settings);

    // Delete old record and create new one (bypasses version conflicts)
    db.query("DELETE settings:`app`")
        .await
        .map_err(|e| format!("Failed to delete old record: {}", e))?;

    db.query("CREATE settings:`app` CONTENT $data")
        .bind(("data", json))
        .await
        .map_err(|e| format!("Failed to create record: {}", e))?;

    Ok(())
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
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to get current executable: {}", e))?;

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
        Command::new("open")
            .arg(&current_exe)
            .spawn()
            .map_err(|e| format!("Failed to spawn new process: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        Command::new(&current_exe)
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
