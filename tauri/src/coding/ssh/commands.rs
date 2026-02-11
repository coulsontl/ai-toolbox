use super::{adapter, session::SshSession, session::SshSessionState, sync};
use super::key_file;
use super::types::{
    SSHConnection, SSHConnectionResult, SSHFileMapping, SSHStatusResult, SSHSyncConfig,
    SyncProgress, SyncResult,
};
use crate::coding::{oh_my_opencode, oh_my_opencode_slim, open_code};
use crate::db::DbState;
use chrono::Local;
use tauri::{Emitter, Manager};

// ============================================================================
// 内部共享函数
// ============================================================================

/// Normalise the private key fields on an SSHConnection.
///
/// If the user pasted key content into `private_key_path` (detected by `-----BEGIN`),
/// move it to `private_key_content` and clear `private_key_path`.
fn normalise_key_fields(conn: &mut SSHConnection) {
    // If privateKeyPath actually contains key content, move it
    if key_file::is_private_key_content(&conn.private_key_path) {
        conn.private_key_content = conn.private_key_path.clone();
        conn.private_key_path.clear();
    }
}

/// Get the app data directory from the app handle.
fn get_app_data_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))
}

/// 内部共享函数：从数据库读取完整 SSH 配置
/// 参数 include_mappings 控制是否加载 file_mappings（mcp_sync/skills_sync 不需要）
pub async fn get_ssh_config_internal(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    include_mappings: bool,
) -> Result<SSHSyncConfig, String> {
    let config_result: Result<Vec<serde_json::Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM ssh_sync_config:`config` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query SSH config: {}", e))?
        .take(0);

    let connections_result: Result<Vec<serde_json::Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM ssh_connection ORDER BY sort_order, name")
        .await
        .map_err(|e| format!("Failed to query SSH connections: {}", e))?
        .take(0);

    let connections = connections_result
        .unwrap_or_default()
        .into_iter()
        .map(adapter::connection_from_db_value)
        .collect();

    let file_mappings = if include_mappings {
        let result: Result<Vec<serde_json::Value>, _> = db
            .query("SELECT *, type::string(id) as id FROM ssh_file_mapping ORDER BY module, name")
            .await
            .map_err(|e| format!("Failed to query SSH file mappings: {}", e))?
            .take(0);
        result
            .unwrap_or_default()
            .into_iter()
            .map(adapter::mapping_from_db_value)
            .collect()
    } else {
        vec![]
    };

    match config_result {
        Ok(records) if !records.is_empty() => Ok(adapter::config_from_db_value(
            records[0].clone(),
            file_mappings,
            connections,
        )),
        _ => Ok(SSHSyncConfig {
            file_mappings,
            connections,
            ..SSHSyncConfig::default()
        }),
    }
}

// ============================================================================
// SSH Config Commands
// ============================================================================

/// Get SSH sync configuration (config + connections + file mappings)
#[tauri::command]
pub async fn ssh_get_config(state: tauri::State<'_, DbState>) -> Result<SSHSyncConfig, String> {
    let db = state.0.lock().await;
    get_ssh_config_internal(&db, true).await
}

/// Save SSH sync configuration (enabled, active_connection_id, etc.)
#[tauri::command]
pub async fn ssh_save_config(
    state: tauri::State<'_, DbState>,
    session_state: tauri::State<'_, SshSessionState>,
    app: tauri::AppHandle,
    config: SSHSyncConfig,
) -> Result<(), String> {
    // Check if being enabled
    let was_enabled = {
        let db = state.0.lock().await;
        let result: Result<Vec<serde_json::Value>, _> = db
            .query("SELECT enabled FROM ssh_sync_config:`config` LIMIT 1")
            .await
            .map_err(|e| format!("Failed to query SSH config: {}", e))?
            .take(0);
        result
            .ok()
            .and_then(|records| records.first().cloned())
            .and_then(|v| v.get("enabled").and_then(|e| e.as_bool()))
            .unwrap_or(false)
    };

    let is_being_enabled = !was_enabled && config.enabled;

    {
        let db = state.0.lock().await;

        // Save config
        let config_data = adapter::config_to_db_value(&config);
        db.query("UPSERT ssh_sync_config:`config` CONTENT $data")
            .bind(("data", config_data))
            .await
            .map_err(|e| format!("Failed to save SSH config: {}", e))?;

        // Update file mappings
        for mapping in config.file_mappings.iter() {
            let mapping_data = adapter::mapping_to_db_value(mapping);
            let mapping_id = mapping.id.clone();  // 克隆以满足 'static 生命周期
            db.query("UPSERT type::thing('ssh_file_mapping', $id) CONTENT $data")
                .bind(("id", mapping_id))
                .bind(("data", mapping_data))
                .await
                .map_err(|e| format!("Failed to save SSH file mapping: {}", e))?;
        }
    }

    // 连接生命周期管理
    let mut session = session_state.0.lock().await;
    if config.enabled && !config.active_connection_id.is_empty() {
        // 找到目标连接并建立/切换主连接
        if let Some(conn) = config
            .connections
            .iter()
            .find(|c| c.id == config.active_connection_id)
        {
            let _ = session.connect(conn);
        }
    } else if !config.enabled {
        // 禁用时断开主连接
        session.disconnect();
    }

    // Emit event to refresh UI
    let _ = app.emit("ssh-config-changed", ());

    // If SSH sync was just enabled, trigger a full sync
    if is_being_enabled {
        log::info!("SSH sync enabled, triggering full sync...");

        if session.try_acquire_sync_lock() {
            let _ = session.ensure_connected();
            let result = do_full_sync(&state, &app, &session, &config, None).await;
            session.release_sync_lock();

            if !result.errors.is_empty() {
                log::warn!("SSH full sync errors: {:?}", result.errors);
            }

            update_sync_status(state.inner(), &result).await?;
            let _ = app.emit("ssh-sync-completed", result);
        }
    }

    Ok(())
}

// ============================================================================
// SSH Connection Commands
// ============================================================================

/// List all SSH connection presets
#[tauri::command]
pub async fn ssh_list_connections(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<SSHConnection>, String> {
    let db = state.0.lock().await;

    let result: Result<Vec<serde_json::Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM ssh_connection ORDER BY sort_order, name")
        .await
        .map_err(|e| format!("Failed to query SSH connections: {}", e))?
        .take(0);

    match result {
        Ok(records) => Ok(records
            .into_iter()
            .map(adapter::connection_from_db_value)
            .collect()),
        Err(_) => Ok(vec![]),
    }
}

/// Create a new SSH connection preset
#[tauri::command]
pub async fn ssh_create_connection(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    mut connection: SSHConnection,
) -> Result<(), String> {
    normalise_key_fields(&mut connection);

    // Ensure key file exists on disk if content is provided
    if !connection.private_key_content.is_empty() {
        let app_data_dir = get_app_data_dir(&app)?;
        key_file::ensure_key_file(&app_data_dir, &connection.private_key_content)?;
    }

    let db = state.0.lock().await;

    let conn_data = adapter::connection_to_db_value(&connection);
    let conn_id = connection.id.clone();  // 克隆以满足 'static 生命周期
    db.query("UPSERT type::thing('ssh_connection', $id) CONTENT $data")
        .bind(("id", conn_id))
        .bind(("data", conn_data))
        .await
        .map_err(|e| format!("Failed to create SSH connection: {}", e))?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

/// Update an existing SSH connection preset
#[tauri::command]
pub async fn ssh_update_connection(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    mut connection: SSHConnection,
) -> Result<(), String> {
    normalise_key_fields(&mut connection);
    let app_data_dir = get_app_data_dir(&app)?;

    // Load old connection to compare key content
    {
        let db = state.0.lock().await;
        let old_result: Result<Vec<serde_json::Value>, _> = db
            .query("SELECT *, type::string(id) as id FROM ssh_connection WHERE id = type::thing('ssh_connection', $id)")
            .bind(("id", connection.id.clone()))
            .await
            .map_err(|e| format!("Failed to query old connection: {}", e))?
            .take(0);

        if let Ok(records) = old_result {
            if let Some(record) = records.into_iter().next() {
                let old_conn = adapter::connection_from_db_value(record);
                // If old key content is different from new, remove old key file
                if !old_conn.private_key_content.is_empty()
                    && old_conn.private_key_content != connection.private_key_content
                {
                    key_file::remove_key_file(&app_data_dir, &old_conn.private_key_content);
                }
            }
        }
    }

    // Ensure new key file exists
    if !connection.private_key_content.is_empty() {
        key_file::ensure_key_file(&app_data_dir, &connection.private_key_content)?;
    }

    let db = state.0.lock().await;

    let conn_data = adapter::connection_to_db_value(&connection);
    let conn_id = connection.id.clone();  // 克隆以满足 'static 生命周期
    db.query("UPSERT type::thing('ssh_connection', $id) CONTENT $data")
        .bind(("id", conn_id))
        .bind(("data", conn_data))
        .await
        .map_err(|e| format!("Failed to update SSH connection: {}", e))?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

/// Delete an SSH connection preset
#[tauri::command]
pub async fn ssh_delete_connection(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    // Load connection to get key content before deleting
    let old_result: Result<Vec<serde_json::Value>, _> = db
        .query("SELECT *, type::string(id) as id FROM ssh_connection WHERE id = type::thing('ssh_connection', $id)")
        .bind(("id", id.clone()))
        .await
        .map_err(|e| format!("Failed to query connection: {}", e))?
        .take(0);

    if let Ok(records) = old_result {
        if let Some(record) = records.into_iter().next() {
            let old_conn = adapter::connection_from_db_value(record);
            if !old_conn.private_key_content.is_empty() {
                if let Ok(app_data_dir) = get_app_data_dir(&app) {
                    key_file::remove_key_file(&app_data_dir, &old_conn.private_key_content);
                }
            }
        }
    }

    db.query("DELETE ssh_connection WHERE id = type::thing('ssh_connection', $id)")
        .bind(("id", id))
        .await
        .map_err(|e| format!("Failed to delete SSH connection: {}", e))?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

/// Set active connection (and optionally trigger sync)
#[tauri::command]
pub async fn ssh_set_active_connection(
    state: tauri::State<'_, DbState>,
    session_state: tauri::State<'_, SshSessionState>,
    app: tauri::AppHandle,
    connection_id: String,
) -> Result<(), String> {
    {
        let db = state.0.lock().await;
        db.query("UPDATE ssh_sync_config SET active_connection_id = $id WHERE id = ssh_sync_config:`config`")
            .bind(("id", connection_id.clone()))
            .await
            .map_err(|e| format!("Failed to set active connection: {}", e))?;
    }

    // 切换连接：找到目标连接并建立主连接
    let config = ssh_get_config(state.clone()).await?;
    if config.enabled {
        if let Some(conn) = config
            .connections
            .iter()
            .find(|c| c.id == connection_id)
        {
            let mut session = session_state.0.lock().await;
            if session.connect(conn).is_ok() && session.try_acquire_sync_lock() {
                let result = do_full_sync(&state, &app, &session, &config, None).await;
                session.release_sync_lock();
                let _ = update_sync_status(state.inner(), &result).await;
                let _ = app.emit("ssh-sync-completed", result);
            }
        }
    }

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

/// Test an SSH connection (async, non-blocking)
#[tauri::command]
pub async fn ssh_test_connection(
    app: tauri::AppHandle,
    mut connection: SSHConnection,
) -> SSHConnectionResult {
    normalise_key_fields(&mut connection);

    let app_data_dir = match get_app_data_dir(&app) {
        Ok(dir) => dir,
        Err(e) => {
            return SSHConnectionResult {
                connected: false,
                error: Some(e),
                server_info: None,
            }
        }
    };

    // Ensure key file exists for testing
    if !connection.private_key_content.is_empty() {
        if let Err(e) = key_file::ensure_key_file(&app_data_dir, &connection.private_key_content) {
            return SSHConnectionResult {
                connected: false,
                error: Some(e),
                server_info: None,
            };
        }
    }

    // 在 blocking 线程池中执行，避免阻塞 tokio runtime
    tokio::task::spawn_blocking(move || sync::test_connection(&connection, &app_data_dir))
        .await
        .unwrap_or(SSHConnectionResult {
            connected: false,
            error: Some("测试连接任务异常".to_string()),
            server_info: None,
        })
}

// ============================================================================
// File Mapping Commands
// ============================================================================

/// Add a new SSH file mapping
#[tauri::command]
pub async fn ssh_add_file_mapping(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    mapping: SSHFileMapping,
) -> Result<(), String> {
    let db = state.0.lock().await;

    let mapping_data = adapter::mapping_to_db_value(&mapping);
    let mapping_id = mapping.id.clone();  // 克隆以满足 'static 生命周期
    db.query("UPSERT type::thing('ssh_file_mapping', $id) CONTENT $data")
        .bind(("id", mapping_id))
        .bind(("data", mapping_data))
        .await
        .map_err(|e| format!("Failed to add SSH file mapping: {}", e))?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

/// Update an existing SSH file mapping
#[tauri::command]
pub async fn ssh_update_file_mapping(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    mapping: SSHFileMapping,
) -> Result<(), String> {
    let db = state.0.lock().await;

    let mapping_data = adapter::mapping_to_db_value(&mapping);
    let mapping_id = mapping.id.clone();  // 克隆以满足 'static 生命周期
    db.query("UPSERT type::thing('ssh_file_mapping', $id) CONTENT $data")
        .bind(("id", mapping_id))
        .bind(("data", mapping_data))
        .await
        .map_err(|e| format!("Failed to update SSH file mapping: {}", e))?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

/// Delete an SSH file mapping
#[tauri::command]
pub async fn ssh_delete_file_mapping(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let db = state.0.lock().await;

    db.query("DELETE ssh_file_mapping WHERE id = type::thing('ssh_file_mapping', $id)")
        .bind(("id", id))
        .await
        .map_err(|e| format!("Failed to delete SSH file mapping: {}", e))?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

/// Reset all SSH file mappings
#[tauri::command]
pub async fn ssh_reset_file_mappings(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let db = state.0.lock().await;

    db.query("DELETE ssh_file_mapping")
        .await
        .map_err(|e| format!("Failed to reset SSH file mappings: {}", e))?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

// ============================================================================
// Sync Commands
// ============================================================================

/// Internal full sync implementation
pub async fn do_full_sync(
    state: &DbState,
    app: &tauri::AppHandle,
    session: &SshSession,
    config: &SSHSyncConfig,
    module: Option<&str>,
) -> SyncResult {
    // Emit initial progress
    let enabled_mappings: Vec<_> = config.file_mappings.iter().filter(|m| m.enabled).collect();
    let total_files = enabled_mappings.len() as u32;
    let _ = app.emit(
        "ssh-sync-progress",
        SyncProgress {
            phase: "files".to_string(),
            current_item: "准备中...".to_string(),
            current: 0,
            total: total_files,
            message: format!("文件同步: 0/{}", total_files),
        },
    );

    // Resolve dynamic config paths
    let file_mappings = resolve_dynamic_paths(config.file_mappings.clone());

    // Sync file mappings with progress
    let mut result = sync_mappings_with_progress(&file_mappings, session, module, app);

    // Also sync MCP and Skills
    if config.sync_mcp {
        if let Err(e) = super::mcp_sync::sync_mcp_to_ssh(state, session, app.clone()).await {
            log::warn!("MCP SSH sync failed: {}", e);
            result.errors.push(format!("MCP sync: {}", e));
            result.success = false;
        }
    }
    if config.sync_skills {
        if let Err(e) = super::skills_sync::sync_skills_to_ssh(state, session, app.clone()).await {
            log::warn!("Skills SSH sync failed: {}", e);
            result.errors.push(format!("Skills sync: {}", e));
            result.success = false;
        }
    }

    result
}

/// Sync file mappings with progress events
fn sync_mappings_with_progress(
    mappings: &[SSHFileMapping],
    session: &SshSession,
    module_filter: Option<&str>,
    app: &tauri::AppHandle,
) -> SyncResult {
    let mut synced_files = vec![];
    let mut skipped_files = vec![];
    let mut errors = vec![];

    let filtered_mappings: Vec<_> = mappings
        .iter()
        .filter(|m| m.enabled)
        .filter(|m| module_filter.is_none() || Some(m.module.as_str()) == module_filter)
        .collect();

    let total = filtered_mappings.len() as u32;

    for (idx, mapping) in filtered_mappings.iter().enumerate() {
        let current = (idx + 1) as u32;

        let _ = app.emit(
            "ssh-sync-progress",
            SyncProgress {
                phase: "files".to_string(),
                current_item: mapping.name.clone(),
                current,
                total,
                message: format!("文件同步: {}/{} - {}", current, total, mapping.name),
            },
        );

        match sync::sync_file_mapping(mapping, session) {
            Ok(files) if files.is_empty() => {
                skipped_files.push(mapping.name.clone());
            }
            Ok(files) => {
                synced_files.extend(files);
            }
            Err(e) => {
                errors.push(format!("{}: {}", mapping.name, e));
            }
        }
    }

    SyncResult {
        success: errors.is_empty(),
        synced_files,
        skipped_files,
        errors,
    }
}

/// Execute SSH sync
#[tauri::command]
pub async fn ssh_sync(
    state: tauri::State<'_, DbState>,
    session_state: tauri::State<'_, SshSessionState>,
    app: tauri::AppHandle,
    module: Option<String>,
) -> Result<SyncResult, String> {
    let config = ssh_get_config(state.clone()).await?;

    if !config.enabled {
        return Ok(SyncResult {
            success: false,
            synced_files: vec![],
            skipped_files: vec![],
            errors: vec!["SSH 同步未启用".to_string()],
        });
    }

    let mut session = session_state.0.lock().await;

    // 并发控制：如果正在同步，直接返回
    if !session.try_acquire_sync_lock() {
        return Ok(SyncResult {
            success: false,
            synced_files: vec![],
            skipped_files: vec![],
            errors: vec!["另一个同步操作正在进行中".to_string()],
        });
    }

    // 确保连接可用（自动重连）
    if let Err(e) = session.ensure_connected() {
        session.release_sync_lock();
        return Ok(SyncResult {
            success: false,
            synced_files: vec![],
            skipped_files: vec![],
            errors: vec![format!("SSH 连接失败: {}", e)],
        });
    }

    let result = do_full_sync(&state, &app, &session, &config, module.as_deref()).await;

    session.release_sync_lock();

    update_sync_status(state.inner(), &result).await?;
    let _ = app.emit("ssh-sync-completed", result.clone());

    Ok(result)
}

/// Get SSH sync status
#[tauri::command]
pub async fn ssh_get_status(state: tauri::State<'_, DbState>) -> Result<SSHStatusResult, String> {
    let config = ssh_get_config(state).await?;

    let active_connection_name = if config.enabled && !config.active_connection_id.is_empty() {
        config
            .connections
            .iter()
            .find(|c| c.id == config.active_connection_id)
            .map(|c| c.name.clone())
    } else {
        None
    };

    Ok(SSHStatusResult {
        ssh_available: config.enabled && active_connection_name.is_some(),
        active_connection_name,
        last_sync_time: config.last_sync_time,
        last_sync_status: config.last_sync_status,
        last_sync_error: config.last_sync_error,
    })
}

/// Test if a local path exists
#[tauri::command]
pub fn ssh_test_local_path(local_path: String) -> Result<bool, String> {
    let expanded = sync::expand_local_path(&local_path)?;
    Ok(std::path::Path::new(&expanded).exists())
}

/// Get default file mappings for SSH
#[tauri::command]
pub fn ssh_get_default_mappings() -> Vec<SSHFileMapping> {
    default_file_mappings()
}

// ============================================================================
// Internal Functions
// ============================================================================

/// Dynamically resolve config file paths for opencode and oh-my-opencode
pub fn resolve_dynamic_paths(mappings: Vec<SSHFileMapping>) -> Vec<SSHFileMapping> {
    mappings
        .into_iter()
        .map(|mut mapping| {
            match mapping.id.as_str() {
                "opencode-main" => {
                    if let Ok(actual_path) = open_code::get_default_config_path() {
                        if let Some(filename) = std::path::Path::new(&actual_path).file_name() {
                            let filename_str = filename.to_string_lossy();
                            mapping.local_path = actual_path.clone();
                            mapping.remote_path =
                                format!("~/.config/opencode/{}", filename_str);
                        }
                    }
                }
                "opencode-oh-my" => {
                    if let Ok(actual_path) = oh_my_opencode::get_oh_my_opencode_config_path() {
                        if let Some(filename) = actual_path.file_name() {
                            let filename_str = filename.to_string_lossy();
                            mapping.local_path = actual_path.to_string_lossy().to_string();
                            mapping.remote_path =
                                format!("~/.config/opencode/{}", filename_str);
                        }
                    }
                }
                "opencode-oh-my-slim" => {
                    if let Ok(actual_path) =
                        oh_my_opencode_slim::get_oh_my_opencode_slim_config_path()
                    {
                        if let Some(filename) = actual_path.file_name() {
                            let filename_str = filename.to_string_lossy();
                            mapping.local_path = actual_path.to_string_lossy().to_string();
                            mapping.remote_path =
                                format!("~/.config/opencode/{}", filename_str);
                        }
                    }
                }
                _ => {}
            }
            mapping
        })
        .collect()
}

/// Update sync status in database
pub async fn update_sync_status(
    state: &DbState,
    result: &SyncResult,
) -> Result<(), String> {
    let db = state.0.lock().await;

    let (status, error) = if result.success {
        ("success".to_string(), None)
    } else {
        let error_msg = result.errors.join("; ");
        ("error".to_string(), Some(error_msg))
    };

    let now = Local::now().to_rfc3339();

    db.query("UPDATE ssh_sync_config SET last_sync_time = $time, last_sync_status = $status, last_sync_error = $error WHERE id = ssh_sync_config:`config`")
        .bind(("time", now))
        .bind(("status", status))
        .bind(("error", error))
        .await
        .map_err(|e| format!("Failed to update SSH sync status: {}", e))?;

    Ok(())
}

/// Get default file mappings for SSH sync
pub fn default_file_mappings() -> Vec<SSHFileMapping> {
    vec![
        // OpenCode
        SSHFileMapping {
            id: "opencode-main".to_string(),
            name: "OpenCode 主配置".to_string(),
            module: "opencode".to_string(),
            local_path: "~/.config/opencode/opencode.jsonc".to_string(),
            remote_path: "~/.config/opencode/opencode.jsonc".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
        SSHFileMapping {
            id: "opencode-oh-my".to_string(),
            name: "Oh My OpenCode 配置".to_string(),
            module: "opencode".to_string(),
            local_path: "~/.config/opencode/oh-my-opencode.jsonc".to_string(),
            remote_path: "~/.config/opencode/oh-my-opencode.jsonc".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
        SSHFileMapping {
            id: "opencode-oh-my-slim".to_string(),
            name: "Oh My OpenCode Slim 配置".to_string(),
            module: "opencode".to_string(),
            local_path: "~/.config/opencode/oh-my-opencode-slim.json".to_string(),
            remote_path: "~/.config/opencode/oh-my-opencode-slim.json".to_string(),
            enabled: false,
            is_pattern: false,
            is_directory: false,
        },
        SSHFileMapping {
            id: "opencode-auth".to_string(),
            name: "OpenCode 认证信息".to_string(),
            module: "opencode".to_string(),
            local_path: "~/.local/share/opencode/auth.json".to_string(),
            remote_path: "~/.local/share/opencode/auth.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
        SSHFileMapping {
            id: "opencode-plugins".to_string(),
            name: "OpenCode 插件文件".to_string(),
            module: "opencode".to_string(),
            local_path: "~/.config/opencode/*.mjs".to_string(),
            remote_path: "~/.config/opencode/".to_string(),
            enabled: true,
            is_pattern: true,
            is_directory: false,
        },
        // Claude Code
        SSHFileMapping {
            id: "claude-settings".to_string(),
            name: "Claude Code 设置".to_string(),
            module: "claude".to_string(),
            local_path: "~/.claude/settings.json".to_string(),
            remote_path: "~/.claude/settings.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
        SSHFileMapping {
            id: "claude-config".to_string(),
            name: "Claude Code 配置".to_string(),
            module: "claude".to_string(),
            local_path: "~/.claude/config.json".to_string(),
            remote_path: "~/.claude/config.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
        // Codex
        SSHFileMapping {
            id: "codex-auth".to_string(),
            name: "Codex 认证".to_string(),
            module: "codex".to_string(),
            local_path: "~/.codex/auth.json".to_string(),
            remote_path: "~/.codex/auth.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
        SSHFileMapping {
            id: "codex-config".to_string(),
            name: "Codex 配置".to_string(),
            module: "codex".to_string(),
            local_path: "~/.codex/config.toml".to_string(),
            remote_path: "~/.codex/config.toml".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
        },
    ]
}
