use super::key_file;
use super::types::{
    SSHConnection, SSHConnectionResult, SSHFileMapping, SSHStatusResult, SSHSyncConfig,
    SyncProgress, SyncResult,
};
use super::{adapter, session::SshSession, session::SshSessionState, sync};
use crate::coding::claude_code::plugin_metadata_sync;
use crate::coding::db_id::db_record_id;
use crate::coding::runtime_location;
use crate::db::DbState;
use chrono::Local;
use std::path::Path;
use tauri::Emitter;

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
        let mappings: Vec<SSHFileMapping> = result
            .unwrap_or_default()
            .into_iter()
            .map(adapter::mapping_from_db_value)
            .collect();
        // Auto-insert missing default mappings for upgrading users
        backfill_default_mappings(db, mappings).await
    } else {
        vec![]
    };
    let module_statuses = runtime_location::get_wsl_direct_status_map_async(db).await?;

    match config_result {
        Ok(records) if !records.is_empty() => {
            let mut config =
                adapter::config_from_db_value(records[0].clone(), file_mappings, connections);
            config.module_statuses = module_statuses;
            Ok(config)
        }
        _ => Ok(SSHSyncConfig {
            file_mappings,
            connections,
            module_statuses,
            ..SSHSyncConfig::default()
        }),
    }
}

fn get_active_connection<'a>(config: &'a SSHSyncConfig) -> Result<&'a SSHConnection, String> {
    config
        .connections
        .iter()
        .find(|connection| connection.id == config.active_connection_id)
        .ok_or_else(|| format!("当前 SSH 活跃连接不存在: {}", config.active_connection_id))
}

async fn ensure_session_matches_active_connection(
    session: &mut SshSession,
    config: &SSHSyncConfig,
) -> Result<(), String> {
    let active_connection = get_active_connection(config)?;
    let current_connection_id = session.conn().map(|connection| connection.id.as_str());

    if current_connection_id != Some(config.active_connection_id.as_str()) {
        log::info!(
            "SSH session active connection mismatch, reconnecting: session_connection_id={:?}, target_connection_id={}, target_connection_name={}",
            current_connection_id,
            config.active_connection_id,
            active_connection.name
        );
        session.connect(active_connection).await?;
    }

    session.ensure_connected().await
}

pub async fn restore_ssh_session_from_saved_config(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    session_state: &SshSessionState,
) -> Result<(), String> {
    let config = get_ssh_config_internal(db, false).await?;
    if !config.enabled || config.active_connection_id.is_empty() {
        return Ok(());
    }

    let mut session = session_state.0.lock().await;
    ensure_session_matches_active_connection(&mut session, &config)
        .await
        .map_err(|error| format!("恢复 SSH 会话失败: {}", error))
}

// ============================================================================
// SSH Config Commands
// ============================================================================

/// Get SSH sync configuration (config + connections + file mappings)
#[tauri::command]
pub async fn ssh_get_config(state: tauri::State<'_, DbState>) -> Result<SSHSyncConfig, String> {
    let db = state.db();
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
        let db = state.db();
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
        let db = state.db();

        // Save config
        let config_data = adapter::config_to_db_value(&config);
        db.query("UPSERT ssh_sync_config:`config` CONTENT $data")
            .bind(("data", config_data))
            .await
            .map_err(|e| format!("Failed to save SSH config: {}", e))?;

        // Update file mappings
        for mapping in config.file_mappings.iter() {
            let mapping_data = adapter::mapping_to_db_value(mapping);
            let record_id = db_record_id("ssh_file_mapping", &mapping.id);
            db.query(&format!("UPSERT {} CONTENT $data", record_id))
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
            let _ = session.connect(conn).await;
        }
    } else if !config.enabled {
        // 禁用时断开主连接
        session.disconnect().await;

        // 清除同步状态，避免残留错误信息
        let db = state.db();
        let _ = db
            .query("UPDATE ssh_sync_config SET last_sync_status = NONE, last_sync_error = NONE WHERE id = ssh_sync_config:`config`")
            .await;
    }

    // Emit event to refresh UI
    let _ = app.emit("ssh-config-changed", ());

    // If SSH sync was just enabled, trigger a full sync
    if is_being_enabled && !config.active_connection_id.is_empty() {
        log::info!("SSH sync enabled, triggering full sync...");

        if session.try_acquire_sync_lock() {
            let _ = session.ensure_connected().await;
            let result = do_full_sync(&state, &app, &session, &config, None, None).await;
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
    let db = state.db();

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

    let db = state.db();

    let conn_data = adapter::connection_to_db_value(&connection);
    let record_id = db_record_id("ssh_connection", &connection.id);
    db.query(&format!("UPSERT {} CONTENT $data", record_id))
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

    let db = state.db();

    let conn_data = adapter::connection_to_db_value(&connection);
    let record_id = db_record_id("ssh_connection", &connection.id);
    db.query(&format!("UPSERT {} CONTENT $data", record_id))
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
    let db = state.db();

    let record_id = db_record_id("ssh_connection", &id);
    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|e| format!("Failed to delete SSH connection: {}", e))?;

    // 如果删除的是当前活跃连接，清除 active_connection_id
    db.query("UPDATE ssh_sync_config SET active_connection_id = '' WHERE id = ssh_sync_config:`config` AND active_connection_id = $id")
        .bind(("id", id))
        .await
        .map_err(|e| format!("Failed to clear active connection: {}", e))?;

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
        let db = state.db();
        db.query("UPDATE ssh_sync_config SET active_connection_id = $id WHERE id = ssh_sync_config:`config`")
            .bind(("id", connection_id.clone()))
            .await
            .map_err(|e| format!("Failed to set active connection: {}", e))?;
    }

    // 切换连接：找到目标连接并建立主连接
    let config = ssh_get_config(state.clone()).await?;
    if config.enabled {
        if let Some(conn) = config.connections.iter().find(|c| c.id == connection_id) {
            let mut session = session_state.0.lock().await;
            if session.connect(conn).await.is_ok() && session.try_acquire_sync_lock() {
                let result = do_full_sync(&state, &app, &session, &config, None, None).await;
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
pub async fn ssh_test_connection(mut connection: SSHConnection) -> SSHConnectionResult {
    normalise_key_fields(&mut connection);

    sync::test_connection(&connection).await
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
    let db = state.db();

    let mapping_data = adapter::mapping_to_db_value(&mapping);
    let record_id = db_record_id("ssh_file_mapping", &mapping.id);
    db.query(&format!("UPSERT {} CONTENT $data", record_id))
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
    let db = state.db();

    let mapping_data = adapter::mapping_to_db_value(&mapping);
    let record_id = db_record_id("ssh_file_mapping", &mapping.id);
    db.query(&format!("UPSERT {} CONTENT $data", record_id))
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
    let db = state.db();

    let record_id = db_record_id("ssh_file_mapping", &id);
    db.query(&format!("DELETE {}", record_id))
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
    let db = state.db();

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
    skip_modules: Option<&[String]>,
) -> SyncResult {
    let total_mapping_count = config.file_mappings.len();
    let enabled_mapping_count = config.file_mappings.iter().filter(|m| m.enabled).count();
    let disabled_mapping_count = total_mapping_count.saturating_sub(enabled_mapping_count);
    log::info!(
        "SSH full sync start: module={:?}, skip_modules={:?}, mappings_total={}, mappings_enabled={}, mappings_disabled={}, sync_mcp={}, sync_skills={}",
        module,
        skip_modules,
        total_mapping_count,
        enabled_mapping_count,
        disabled_mapping_count,
        config.sync_mcp,
        config.sync_skills
    );

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
            current_file: None,
        },
    );

    // Resolve dynamic config paths
    let db = state.db();
    let file_mappings = resolve_dynamic_paths_with_db(&db, config.file_mappings.clone()).await;
    log::info!(
        "SSH full sync resolved dynamic mappings: resolved_count={}",
        file_mappings.len()
    );

    // Sync file mappings with progress
    let mut result =
        sync_mappings_with_progress(&file_mappings, session, module, skip_modules, app).await;
    log::info!(
        "SSH full sync file stage completed: synced_files={}, skipped_files={}, errors={}",
        result.synced_files.len(),
        result.skipped_files.len(),
        result.errors.len()
    );
    if result.errors.is_empty() && result.synced_files.is_empty() {
        log::warn!(
            "SSH full sync file stage uploaded zero files: skipped_files={}, module={:?}, skip_modules={:?}",
            result.skipped_files.len(),
            module,
            skip_modules
        );
    }

    let skip_claude = skip_modules
        .map(|modules| modules.iter().any(|m| m == "claude"))
        .unwrap_or(false);
    if !skip_claude && (module.is_none() || module == Some("claude")) {
        if let Err(error) = rewrite_claude_plugin_metadata_on_remote(&db, session).await {
            log::warn!("Claude plugin metadata SSH rewrite failed: {}", error);
            result
                .errors
                .push(format!("Claude plugins metadata rewrite: {}", error));
        }
    }

    // Also sync MCP and Skills
    if config.sync_mcp {
        log::info!("SSH full sync entering MCP sync stage");
        if let Err(e) = super::mcp_sync::sync_mcp_to_ssh(state, session, app.clone()).await {
            log::warn!("MCP SSH sync failed: {}", e);
            result.errors.push(format!("MCP sync: {}", e));
            result.success = false;
        }
    } else {
        log::info!("SSH full sync skipped MCP sync stage because sync_mcp=false");
    }
    if config.sync_skills {
        log::info!("SSH full sync entering Skills sync stage");
        if let Err(e) = super::skills_sync::sync_skills_to_ssh(state, session, app.clone()).await {
            log::warn!("Skills SSH sync failed: {}", e);
            result.errors.push(format!("Skills sync: {}", e));
            result.success = false;
        }
    } else {
        log::info!("SSH full sync skipped Skills sync stage because sync_skills=false");
    }

    // Ensure OpenClaw config exists on remote (create empty {} if missing)
    let skip_openclaw = skip_modules
        .map(|modules| modules.iter().any(|m| m == "openclaw"))
        .unwrap_or(false);
    if !skip_openclaw && (module.is_none() || module == Some("openclaw")) {
        log::info!("SSH full sync ensuring OpenClaw remote config exists");
        if let Err(e) = ensure_openclaw_config_on_remote(state, session).await {
            log::warn!("OpenClaw SSH config init failed: {}", e);
        }
    } else {
        log::info!(
            "SSH full sync skipped OpenClaw remote config init: module={:?}, skip_openclaw={}",
            module,
            skip_openclaw
        );
    }

    log::info!(
        "SSH full sync completed: success={}, synced_files={}, skipped_files={}, errors={}",
        result.success,
        result.synced_files.len(),
        result.skipped_files.len(),
        result.errors.len()
    );
    result
}

async fn rewrite_claude_plugin_metadata_on_remote(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    session: &SshSession,
) -> Result<(), String> {
    let source_plugins_root = runtime_location::get_claude_plugins_dir_async(db)
        .await?
        .to_string_lossy()
        .to_string();
    let target_plugins_root_raw =
        runtime_location::get_claude_wsl_target_path_async(db, "plugins").await;

    // Claude CLI 2.1.126+ does not expand `~` inside `installLocation` /
    // `installPath`. Resolve the remote `$HOME` once so the values we write back
    // are absolute Linux paths, while the read/write helpers still consume the
    // original `~`-prefixed path through shell expansion.
    let target_plugins_root =
        expand_tilde_with_remote_home(session, &target_plugins_root_raw).await?;

    for file_name in ["known_marketplaces.json", "installed_plugins.json"] {
        let target_file_path = format!(
            "{}/{}",
            target_plugins_root_raw.trim_end_matches('/'),
            file_name
        );
        let existing_content = sync::read_remote_file(session, &target_file_path).await?;
        if existing_content.trim().is_empty() {
            continue;
        }

        let Some(rewritten_content) =
            plugin_metadata_sync::rewrite_claude_plugin_metadata_if_needed(
                file_name,
                &existing_content,
                &source_plugins_root,
                &target_plugins_root,
            )?
        else {
            continue;
        };

        sync::write_remote_file(session, &target_file_path, &rewritten_content).await?;
    }

    Ok(())
}

async fn expand_tilde_with_remote_home(session: &SshSession, path: &str) -> Result<String, String> {
    if !path.starts_with('~') {
        return Ok(path.to_string());
    }
    let home = sync::get_remote_user_home(session).await?;
    Ok(runtime_location::expand_home_from_user_root(
        Some(&home),
        path,
    ))
}

/// Sync file mappings with progress events
async fn sync_mappings_with_progress(
    mappings: &[SSHFileMapping],
    session: &SshSession,
    module_filter: Option<&str>,
    skip_modules: Option<&[String]>,
    app: &tauri::AppHandle,
) -> SyncResult {
    let mut synced_files = vec![];
    let mut skipped_files = vec![];
    let mut errors = vec![];
    let mut filtered_mappings = Vec::new();
    let mut disabled_mapping_count = 0usize;
    let mut filtered_by_module_count = 0usize;
    let mut filtered_by_skip_modules_count = 0usize;

    for mapping in mappings {
        if !mapping.enabled {
            disabled_mapping_count += 1;
            log::trace!(
                "SSH sync mapping skipped: id={}, name={}, module={}, reason=disabled",
                mapping.id,
                mapping.name,
                mapping.module
            );
            continue;
        }
        if module_filter.is_some() && Some(mapping.module.as_str()) != module_filter {
            filtered_by_module_count += 1;
            log::trace!(
                "SSH sync mapping skipped: id={}, name={}, module={}, reason=module_filter_mismatch, module_filter={:?}",
                mapping.id,
                mapping.name,
                mapping.module,
                module_filter
            );
            continue;
        }
        if skip_modules
            .map(|skip| {
                skip.iter()
                    .any(|module_name| module_name == &mapping.module)
            })
            .unwrap_or(false)
        {
            filtered_by_skip_modules_count += 1;
            log::trace!(
                "SSH sync mapping skipped: id={}, name={}, module={}, reason=skip_modules, skip_modules={:?}",
                mapping.id,
                mapping.name,
                mapping.module,
                skip_modules
            );
            continue;
        }
        filtered_mappings.push(mapping);
    }

    let total = filtered_mappings.len() as u32;
    log::info!(
        "SSH sync mapping filter summary: total_mappings={}, selected_mappings={}, disabled_mappings={}, filtered_by_module={}, filtered_by_skip_modules={}, module_filter={:?}, skip_modules={:?}",
        mappings.len(),
        filtered_mappings.len(),
        disabled_mapping_count,
        filtered_by_module_count,
        filtered_by_skip_modules_count,
        module_filter,
        skip_modules
    );

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
                current_file: None,
            },
        );

        let report_current_file = |current_file: String| {
            let _ = app.emit(
                "ssh-sync-progress",
                SyncProgress {
                    phase: "files".to_string(),
                    current_item: mapping.name.clone(),
                    current,
                    total,
                    message: format!("文件同步: {}/{} - {}", current, total, mapping.name),
                    current_file: Some(current_file),
                },
            );
        };

        match sync::sync_file_mapping_with_progress(mapping, session, Some(&report_current_file))
            .await
        {
            Ok(mut files) => {
                match reconcile_codex_prompt_files_on_ssh(
                    mapping,
                    session,
                    Some(&report_current_file),
                )
                .await
                {
                    Ok(prompt_files) => files.extend(prompt_files),
                    Err(error) => errors.push(format!("{}: {}", mapping.name, error)),
                }
                if files.is_empty() {
                    log::warn!(
                        "SSH sync mapping produced no uploaded files: id={}, name={}, module={}, local_path={}, remote_path={}",
                        mapping.id,
                        mapping.name,
                        mapping.module,
                        mapping.local_path,
                        mapping.remote_path
                    );
                    skipped_files.push(mapping.name.clone());
                    continue;
                }
                log::trace!(
                    "SSH sync mapping uploaded files: id={}, name={}, module={}, uploaded_count={}, remote_path={}",
                    mapping.id,
                    mapping.name,
                    mapping.module,
                    files.len(),
                    mapping.remote_path
                );
                synced_files.extend(files);
            }
            Err(e) => {
                log::warn!(
                    "SSH sync mapping failed: id={}, name={}, module={}, local_path={}, remote_path={}, error={}",
                    mapping.id,
                    mapping.name,
                    mapping.module,
                    mapping.local_path,
                    mapping.remote_path,
                    e
                );
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

async fn reconcile_codex_prompt_files_on_ssh(
    mapping: &SSHFileMapping,
    session: &SshSession,
    current_file_reporter: Option<&(dyn Fn(String) + Send + Sync)>,
) -> Result<Vec<String>, String> {
    if mapping.id != "codex-prompt" || mapping.is_directory || mapping.is_pattern {
        return Ok(vec![]);
    }

    let mut synced_files = Vec::new();
    for file_name in runtime_location::CODEX_PROMPT_FILE_NAMES {
        let local_path = runtime_location::replace_path_file_name(&mapping.local_path, file_name);
        let remote_path = runtime_location::replace_path_file_name(&mapping.remote_path, file_name);
        let expanded_local_path = sync::expand_local_path(&local_path)?;

        if Path::new(&expanded_local_path).exists() {
            if local_path == mapping.local_path && remote_path == mapping.remote_path {
                continue;
            }
            synced_files.extend(
                sync::sync_single_file_with_progress(
                    &expanded_local_path,
                    &remote_path,
                    session,
                    current_file_reporter,
                )
                .await?,
            );
        } else {
            sync::remove_remote_path(session, &remote_path).await?;
            synced_files.push(format!("removed stale Codex prompt: {}", remote_path));
        }
    }

    Ok(synced_files)
}

/// Execute SSH sync
#[tauri::command]
pub async fn ssh_sync(
    state: tauri::State<'_, DbState>,
    session_state: tauri::State<'_, SshSessionState>,
    app: tauri::AppHandle,
    module: Option<String>,
    skip_modules: Option<Vec<String>>,
) -> Result<SyncResult, String> {
    let config = ssh_get_config(state.clone()).await?;
    let active_connection = config
        .connections
        .iter()
        .find(|connection| connection.id == config.active_connection_id);
    let enabled_mapping_count = config
        .file_mappings
        .iter()
        .filter(|mapping| mapping.enabled)
        .count();
    log::info!(
        "SSH sync requested: module={:?}, skip_modules={:?}, enabled={}, active_connection_id={}, active_connection_name={:?}, mappings_total={}, mappings_enabled={}",
        module,
        skip_modules,
        config.enabled,
        config.active_connection_id,
        active_connection.map(|connection| connection.name.as_str()),
        config.file_mappings.len(),
        enabled_mapping_count
    );

    if !config.enabled || config.active_connection_id.is_empty() {
        log::warn!(
            "SSH sync request rejected: enabled={}, active_connection_id='{}'",
            config.enabled,
            config.active_connection_id
        );
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
        log::warn!(
            "SSH sync request ignored because another sync is already running: module={:?}, skip_modules={:?}",
            module,
            skip_modules
        );
        return Ok(SyncResult {
            success: false,
            synced_files: vec![],
            skipped_files: vec![],
            errors: vec!["另一个同步操作正在进行中".to_string()],
        });
    }

    // 确保会话绑定到当前 active connection，并在需要时自动重连
    if let Err(e) = ensure_session_matches_active_connection(&mut session, &config).await {
        session.release_sync_lock();
        log::warn!(
            "SSH sync connection check failed: connection_id={}, error={}",
            config.active_connection_id,
            e
        );
        return Ok(SyncResult {
            success: false,
            synced_files: vec![],
            skipped_files: vec![],
            errors: vec![format!("SSH 连接失败: {}", e)],
        });
    }

    let result = do_full_sync(
        &state,
        &app,
        &session,
        &config,
        module.as_deref(),
        skip_modules.as_deref(),
    )
    .await;

    session.release_sync_lock();

    update_sync_status(state.inner(), &result).await?;
    let _ = app.emit("ssh-sync-completed", result.clone());
    log::info!(
        "SSH sync finished: success={}, synced_files={}, skipped_files={}, errors={}, module={:?}, skip_modules={:?}",
        result.success,
        result.synced_files.len(),
        result.skipped_files.len(),
        result.errors.len(),
        module,
        skip_modules
    );
    if result.success && result.synced_files.is_empty() {
        log::warn!(
            "SSH sync finished without uploading main file mappings: skipped_files={}, module={:?}, skip_modules={:?}",
            result.skipped_files.len(),
            module,
            skip_modules
        );
    }

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

/// Auto-insert any default mappings whose IDs are missing from the database.
/// This ensures upgrading users get newly added default mappings (e.g. OpenClaw).
///
/// Uses a version guard (`ssh_defaults_version`) so the migration runs only once
/// per schema bump. If the user deletes a backfilled mapping afterwards, it will
/// NOT be re-added.
async fn backfill_default_mappings(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    mut file_mappings: Vec<SSHFileMapping>,
) -> Vec<SSHFileMapping> {
    // Bump this number whenever new default mappings are added.
    const CURRENT_DEFAULTS_VERSION: u64 = 5;

    // Read stored version
    let stored_version: u64 = db
        .query("SELECT version FROM ssh_sync_config:`defaults_version` LIMIT 1")
        .await
        .ok()
        .and_then(|mut r| {
            let vals: Result<Vec<serde_json::Value>, _> = r.take(0);
            vals.ok()
        })
        .and_then(|records| records.first().cloned())
        .and_then(|v| v.get("version").and_then(|v| v.as_u64()))
        .unwrap_or(0);

    if stored_version >= CURRENT_DEFAULTS_VERSION {
        return file_mappings;
    }

    // Collect existing IDs
    let existing_ids: std::collections::HashSet<String> =
        file_mappings.iter().map(|m| m.id.clone()).collect();

    for default_mapping in default_file_mappings() {
        if !existing_ids.contains(&default_mapping.id) {
            let mapping_data = adapter::mapping_to_db_value(&default_mapping);
            let record_id =
                crate::coding::db_id::db_record_id("ssh_file_mapping", &default_mapping.id);
            if let Err(e) = db
                .query(&format!("UPSERT {} CONTENT $data", record_id))
                .bind(("data", mapping_data))
                .await
            {
                log::warn!(
                    "Failed to backfill SSH mapping '{}': {}",
                    default_mapping.id,
                    e
                );
                continue;
            }
            log::info!("Backfilled default SSH mapping: {}", default_mapping.id);
            file_mappings.push(default_mapping);
        }
    }

    // Mark migration as done
    let _ = db
        .query("UPSERT ssh_sync_config:`defaults_version` CONTENT { version: $v }")
        .bind(("v", CURRENT_DEFAULTS_VERSION))
        .await;

    file_mappings
}

/// Dynamically resolve config file paths for OpenCode and Oh My OpenAgent.
pub fn resolve_dynamic_paths(mappings: Vec<SSHFileMapping>) -> Vec<SSHFileMapping> {
    mappings
        .into_iter()
        .map(|mapping| {
            match mapping.id.as_str() {
                _ => {}
            }
            mapping
        })
        .collect()
}

pub async fn resolve_dynamic_paths_with_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    mappings: Vec<SSHFileMapping>,
) -> Vec<SSHFileMapping> {
    let mut resolved = Vec::with_capacity(mappings.len());
    for mut mapping in resolve_dynamic_paths(mappings) {
        match mapping.id.as_str() {
            "opencode-main" => {
                if let Ok(location) =
                    runtime_location::get_opencode_runtime_location_async(db).await
                {
                    mapping.local_path = location.host_path.to_string_lossy().to_string();
                    mapping.remote_path = location
                        .wsl
                        .map(|wsl| wsl.linux_path)
                        .unwrap_or_else(|| "~/.config/opencode/opencode.jsonc".to_string());
                }
            }
            "opencode-oh-my" => {
                if let Ok(path) = runtime_location::get_omo_config_path_async(db).await {
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path = path
                        .to_str()
                        .and_then(runtime_location::parse_wsl_unc_path)
                        .map(|wsl| wsl.linux_path)
                        .unwrap_or_else(|| {
                            path.file_name()
                                .map(|name| {
                                    format!("~/.config/opencode/{}", name.to_string_lossy())
                                })
                                .unwrap_or_else(|| {
                                    "~/.config/opencode/oh-my-openagent.jsonc".to_string()
                                })
                        });
                }
            }
            "opencode-oh-my-slim" => {
                if let Ok(path) = runtime_location::get_omos_config_path_async(db).await {
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path = path
                        .to_str()
                        .and_then(runtime_location::parse_wsl_unc_path)
                        .map(|wsl| wsl.linux_path)
                        .unwrap_or_else(|| {
                            "~/.config/opencode/oh-my-opencode-slim.json".to_string()
                        });
                }
            }
            "opencode-prompt" => {
                if let Ok(path) = runtime_location::get_opencode_prompt_path_async(db).await {
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path = path
                        .to_str()
                        .and_then(runtime_location::parse_wsl_unc_path)
                        .map(|wsl| wsl.linux_path)
                        .unwrap_or_else(|| "~/.config/opencode/AGENTS.md".to_string());
                }
            }
            "claude-settings" => {
                if let Ok(path) = runtime_location::get_claude_settings_path_async(db).await {
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path =
                        runtime_location::get_claude_wsl_target_path_async(db, "settings.json")
                            .await;
                }
            }
            "claude-config" => {
                if let Ok(path) = runtime_location::get_claude_plugin_config_path_async(db).await {
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path =
                        runtime_location::get_claude_wsl_target_path_async(db, "config.json").await;
                }
            }
            "claude-prompt" => {
                if let Ok(path) = runtime_location::get_claude_prompt_path_async(db).await {
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path =
                        runtime_location::get_claude_wsl_target_path_async(db, "CLAUDE.md").await;
                }
            }
            "claude-plugins" => {
                if let Ok(path) = runtime_location::get_claude_plugins_dir_async(db).await {
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path =
                        runtime_location::get_claude_wsl_target_path_async(db, "plugins").await;
                }
            }
            "codex-auth" => {
                if let Ok(path) = runtime_location::get_codex_auth_path_async(db).await {
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path =
                        runtime_location::get_codex_wsl_target_path_async(db, "auth.json").await;
                }
            }
            "codex-config" => {
                if let Ok(path) = runtime_location::get_codex_config_path_async(db).await {
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path =
                        runtime_location::get_codex_wsl_target_path_async(db, "config.toml").await;
                }
            }
            "codex-prompt" => {
                if let Ok(path) = runtime_location::get_codex_prompt_path_async(db).await {
                    let file_name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(runtime_location::CODEX_DEFAULT_PROMPT_FILE_NAME);
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path =
                        runtime_location::get_codex_wsl_target_path_async(db, file_name).await;
                }
            }
            "codex-plugins" => {
                if let Ok(location) = runtime_location::get_codex_runtime_location_async(db).await {
                    mapping.local_path = location
                        .host_path
                        .join("plugins")
                        .to_string_lossy()
                        .to_string();
                    mapping.remote_path = location
                        .wsl
                        .map(|wsl| format!("{}/plugins", wsl.linux_path.trim_end_matches('/')))
                        .unwrap_or_else(|| "~/.codex/plugins".to_string());
                }
            }
            "openclaw-config" => {
                if let Ok(location) =
                    runtime_location::get_openclaw_runtime_location_async(db).await
                {
                    mapping.local_path = location.host_path.to_string_lossy().to_string();
                    mapping.remote_path = location
                        .wsl
                        .map(|wsl| wsl.linux_path)
                        .unwrap_or_else(|| "~/.openclaw/openclaw.json".to_string());
                }
            }
            "geminicli-env" => {
                if let Ok(path) = runtime_location::get_gemini_cli_env_path_async(db).await {
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path =
                        runtime_location::get_gemini_cli_wsl_target_path_async(db, ".env").await;
                }
            }
            "geminicli-settings" => {
                if let Ok(path) = runtime_location::get_gemini_cli_settings_path_async(db).await {
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path =
                        runtime_location::get_gemini_cli_wsl_target_path_async(db, "settings.json")
                            .await;
                }
            }
            "geminicli-prompt" => {
                if let Ok(path) = runtime_location::get_gemini_cli_prompt_path_async(db).await {
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path =
                        runtime_location::get_gemini_cli_prompt_wsl_target_path_async(db).await;
                }
            }
            "geminicli-oauth" => {
                if let Ok(path) = runtime_location::get_gemini_cli_oauth_creds_path_async(db).await
                {
                    mapping.local_path = path.to_string_lossy().to_string();
                    mapping.remote_path = runtime_location::get_gemini_cli_wsl_target_path_async(
                        db,
                        "oauth_creds.json",
                    )
                    .await;
                }
            }
            _ => {}
        }
        resolved.push(mapping);
    }
    resolved
}

/// Update sync status in database
pub async fn update_sync_status(state: &DbState, result: &SyncResult) -> Result<(), String> {
    let db = state.db();

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
            directory_excludes: vec![],
        },
        SSHFileMapping {
            id: "opencode-oh-my".to_string(),
            name: "Oh My OpenAgent 配置".to_string(),
            module: "opencode".to_string(),
            local_path: "~/.config/opencode/oh-my-openagent.jsonc".to_string(),
            remote_path: "~/.config/opencode/oh-my-openagent.jsonc".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
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
            directory_excludes: vec![],
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
            directory_excludes: vec![],
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
            directory_excludes: vec![],
        },
        SSHFileMapping {
            id: "opencode-prompt".to_string(),
            name: "OpenCode 全局提示词".to_string(),
            module: "opencode".to_string(),
            local_path: "~/.config/opencode/AGENTS.md".to_string(),
            remote_path: "~/.config/opencode/AGENTS.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
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
            directory_excludes: vec![],
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
            directory_excludes: vec![],
        },
        SSHFileMapping {
            id: "claude-prompt".to_string(),
            name: "Claude Code 全局提示词".to_string(),
            module: "claude".to_string(),
            local_path: "~/.claude/CLAUDE.md".to_string(),
            remote_path: "~/.claude/CLAUDE.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
        },
        SSHFileMapping {
            id: "claude-plugins".to_string(),
            name: "Claude Code 插件目录".to_string(),
            module: "claude".to_string(),
            local_path: "~/.claude/plugins".to_string(),
            remote_path: "~/.claude/plugins".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: true,
            directory_excludes: super::types::default_directory_excludes(),
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
            directory_excludes: vec![],
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
            directory_excludes: vec![],
        },
        SSHFileMapping {
            id: "codex-prompt".to_string(),
            name: "Codex 全局提示词".to_string(),
            module: "codex".to_string(),
            local_path: "~/.codex/AGENTS.md".to_string(),
            remote_path: "~/.codex/AGENTS.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
        },
        SSHFileMapping {
            id: "codex-plugins".to_string(),
            name: "Codex 插件目录".to_string(),
            module: "codex".to_string(),
            local_path: "~/.codex/plugins".to_string(),
            remote_path: "~/.codex/plugins".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: true,
            directory_excludes: super::types::default_directory_excludes(),
        },
        // OpenClaw
        SSHFileMapping {
            id: "openclaw-config".to_string(),
            name: "OpenClaw 配置".to_string(),
            module: "openclaw".to_string(),
            local_path: "~/.openclaw/openclaw.json".to_string(),
            remote_path: "~/.openclaw/openclaw.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
        },
        // Gemini CLI
        SSHFileMapping {
            id: "geminicli-env".to_string(),
            name: "Gemini CLI 环境变量".to_string(),
            module: "geminicli".to_string(),
            local_path: "~/.gemini/.env".to_string(),
            remote_path: "~/.gemini/.env".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
        },
        SSHFileMapping {
            id: "geminicli-settings".to_string(),
            name: "Gemini CLI 设置".to_string(),
            module: "geminicli".to_string(),
            local_path: "~/.gemini/settings.json".to_string(),
            remote_path: "~/.gemini/settings.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
        },
        SSHFileMapping {
            id: "geminicli-prompt".to_string(),
            name: "Gemini CLI 全局提示词".to_string(),
            module: "geminicli".to_string(),
            local_path: "~/.gemini/GEMINI.md".to_string(),
            remote_path: "~/.gemini/GEMINI.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
        },
        SSHFileMapping {
            id: "geminicli-oauth".to_string(),
            name: "Gemini CLI OAuth 凭证".to_string(),
            module: "geminicli".to_string(),
            local_path: "~/.gemini/oauth_creds.json".to_string(),
            remote_path: "~/.gemini/oauth_creds.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
        },
    ]
}

/// Ensure OpenClaw config file exists on the remote SSH host.
///
/// Checks if `~/.openclaw/openclaw.json` exists on the remote.
/// If the file is missing, creates it with an empty JSON object `{}`.
async fn ensure_openclaw_config_on_remote(
    state: &DbState,
    session: &SshSession,
) -> Result<(), String> {
    let remote_path = runtime_location::get_openclaw_wsl_target_path_async(&state.db()).await;
    let shell_remote_path = shell_path_literal(&remote_path);
    let check_cmd = format!(
        "test -f {} && echo EXISTS || echo MISSING",
        shell_remote_path
    );
    let output = session.exec_command(&check_cmd).await?;

    if output.trim() == "EXISTS" {
        return Ok(());
    }

    // Create directory and write default config
    let parent_path = remote_path
        .rsplit_once('/')
        .map(|(parent, _)| {
            if parent.is_empty() {
                "/".to_string()
            } else {
                parent.to_string()
            }
        })
        .unwrap_or_else(|| ".".to_string());
    let shell_parent_path = shell_path_literal(&parent_path);
    let create_cmd = format!(
        "mkdir -p {} && printf '{{}}' > {}",
        shell_parent_path, shell_remote_path
    );
    session.exec_command(&create_cmd).await?;
    log::info!("Created default OpenClaw config on remote: {}", remote_path);

    Ok(())
}

fn shell_path_literal(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        return format!("~/'{}'", rest.replace('\'', "'\\''"));
    }

    if path == "~" {
        return "~".to_string();
    }

    format!("'{}'", path.replace('\'', "'\\''"))
}
