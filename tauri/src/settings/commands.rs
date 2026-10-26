use super::store;
use super::types::{
    AppSettings, BackupFileFilterPathOption, GatewayUsageRangeSelection, SessionDetailFilters,
};
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

/// Save settings to database using adapter layer.
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

/// Load the persisted session-detail filter visibility. Returns `None` when no
/// record exists yet; the frontend falls back to "all visible".
#[tauri::command]
pub async fn get_session_detail_filters(
    sqlite_state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<SessionDetailFilters>, String> {
    store::get_session_detail_filters_from_sqlite_state(&sqlite_state)
}

/// Persist only the nested `session_detail_filters` key.
#[tauri::command]
pub async fn save_session_detail_filters(
    sqlite_state: tauri::State<'_, SqliteDbState>,
    filters: SessionDetailFilters,
) -> Result<(), String> {
    store::save_session_detail_filters_to_sqlite_state(&sqlite_state, &filters)
}

/// Read the gateway statistics page's persisted time range.
#[tauri::command]
pub async fn get_gateway_usage_range(
    sqlite_state: tauri::State<'_, SqliteDbState>,
) -> Result<GatewayUsageRangeSelection, String> {
    Ok(store::load_settings_from_sqlite_state(&sqlite_state)?.gateway_usage_range)
}

/// Persist the gateway statistics page's time range and rebuild the tray menu
/// so the usage block immediately reflects the new window.
#[tauri::command]
pub async fn save_gateway_usage_range(
    sqlite_state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    range: GatewayUsageRangeSelection,
) -> Result<(), String> {
    store::save_gateway_usage_range_to_sqlite_state(&sqlite_state, &range)?;
    if let Err(err) = tray::refresh_tray_menus(&app).await {
        log::warn!("Failed to refresh tray after saving gateway usage range: {err}");
    }
    Ok(())
}

/// Read the provider list UI state: per-module sort modes + last-used map.
#[tauri::command]
pub async fn get_provider_list_state(
    sqlite_state: tauri::State<'_, SqliteDbState>,
) -> Result<super::types::ProviderListState, String> {
    super::provider_list_state::get_provider_list_state(&sqlite_state)
}

/// Persist one module's provider list sort mode.
#[tauri::command]
pub async fn save_provider_sort_mode(
    sqlite_state: tauri::State<'_, SqliteDbState>,
    module: String,
    mode: String,
) -> Result<(), String> {
    super::provider_list_state::save_provider_sort_mode_in_sqlite_state(
        &sqlite_state,
        &module,
        &mode,
    )
}

/// Persist one module's provider list layout (`list` / `card`).
#[tauri::command]
pub async fn save_provider_view_mode(
    sqlite_state: tauri::State<'_, SqliteDbState>,
    module: String,
    mode: String,
) -> Result<(), String> {
    super::provider_list_state::save_provider_view_mode_in_sqlite_state(
        &sqlite_state,
        &module,
        &mode,
    )
}

/// Record that a provider was just applied/selected. Frontend fallback for
/// module paths where only the frontend knows the provider id (e.g. "set as
/// default model" actions in file-based tabs); DB-backed apply flows record
/// directly inside their backend apply functions.
#[tauri::command]
pub async fn record_provider_last_used(
    sqlite_state: tauri::State<'_, SqliteDbState>,
    module: String,
    provider_id: String,
) -> Result<(), String> {
    super::provider_list_state::record_provider_last_used_in_sqlite_state(
        &sqlite_state,
        &module,
        &provider_id,
    )
}

/// Normalize a backup custom entry path for portable storage and display.
#[tauri::command]
pub fn normalize_backup_custom_entry_path(path: String) -> String {
    crate::settings::backup::utils::normalize_backup_storage_path(&path)
}

/// Probe a manual CLI path's version (live) without persisting anything.
/// Used by the "More Options" modal to display the version of a saved path and
/// to validate a newly entered path before saving.
#[tauri::command]
pub async fn probe_manual_cli_version(path: String) -> Result<String, String> {
    crate::coding::cli_resolver::probe_cli_version(&path).await
}

/// Auto-detect the current local CLI path for a command name and return it so
/// the "More Options" input can be pre-filled. Does not persist anything.
#[tauri::command]
pub async fn detect_manual_cli_path(command_name: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let program = crate::coding::cli_resolver::resolve_local_cli_by_command_name(&command_name)
            .ok_or_else(|| {
                format!("未检测到 `{command_name}` CLI，请确认已安装或手动选择路径。")
            })?;
        Ok(program.path.to_string_lossy().to_string())
    })
    .await
    .map_err(|error| format!("CLI 路径探测任务失败: {error}"))?
}

/// Persist a manual CLI path for a given command name (e.g. `opencode`, `pi`,
/// `claude`, `grok`). Validates the path by probing `--version` first; only a
/// usable CLI is saved. Returns the probed version.
#[tauri::command]
pub async fn set_manual_cli_path(
    sqlite_state: tauri::State<'_, SqliteDbState>,
    command_name: String,
    path: String,
) -> Result<String, String> {
    let trimmed = path.trim().to_string();
    let version = if trimmed.is_empty() {
        String::new()
    } else {
        crate::coding::cli_resolver::validate_manual_cli_path(&trimmed).await?
    };

    let mut settings = store::load_settings_from_sqlite_state(&sqlite_state)?;
    if trimmed.is_empty() {
        settings.cli_manual_paths.remove(&command_name);
    } else {
        settings.cli_manual_paths.insert(command_name, trimmed);
    }
    store::save_settings_to_sqlite_state(&sqlite_state, &settings)?;

    Ok(version)
}

/// List backup file paths that can currently be excluded by tool.
#[tauri::command]
pub async fn list_backup_file_filter_path_options(
    sqlite_state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<BackupFileFilterPathOption>, String> {
    crate::settings::backup::utils::list_backup_file_filter_path_options(&sqlite_state).await
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

/// Keep the computer awake (prevent idle sleep). Off = restore normal behavior.
#[tauri::command]
pub async fn set_keep_awake(enabled: bool) -> Result<(), String> {
    crate::keep_awake::set_keep_awake(enabled)
        .await
        .map_err(|error| error.to_string())
}

/// Restart the application
#[tauri::command]
pub fn restart_app(_app: tauri::AppHandle) -> Result<(), String> {
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
        let launch_target = if crate::is_appimage_runtime() {
            crate::appimage_reexec_target()?
        } else {
            current_exe
        };
        let mut restart = Command::new(launch_target);
        restart.args(args).env("AI_TOOLBOX_RESTART_WAIT_LOCK", "1");
        crate::linux_webview::configure_restart_environment(&mut restart);
        tauri_plugin_single_instance::destroy(&_app);
        restart
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
