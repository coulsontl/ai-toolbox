use super::key_file;
use super::types::{
    SSHConnection, SSHConnectionResult, SSHFileMapping, SSHStatusResult, SSHSyncConfig,
    SyncProgress, SyncResult,
};
use super::{adapter, session::SshSession, session::SshSessionState, sync};
use crate::coding::claude_code::plugin_metadata_sync;
use crate::coding::runtime_location;
use crate::db::helpers::{db_delete, db_delete_all, db_get, db_list, db_put};
use crate::db::schema::{DbTable, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
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

fn ssh_connection_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_integer("sort_order", OrderDirection::Asc)?,
        OrderField::json_text("name", OrderDirection::Asc)?,
    ]))
}

fn ssh_mapping_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_text("module", OrderDirection::Asc)?,
        OrderField::json_text("name", OrderDirection::Asc)?,
    ]))
}

fn load_ssh_config_record(state: &SqliteDbState) -> Result<Option<serde_json::Value>, String> {
    state.with_conn(|conn| db_get(conn, DbTable::SshSyncConfig, "config"))
}

fn load_ssh_connections(state: &SqliteDbState) -> Result<Vec<SSHConnection>, String> {
    let order = ssh_connection_order()?;
    state.with_conn(|conn| {
        Ok(db_list(conn, DbTable::SshConnection, Some(&order))?
            .into_iter()
            .map(adapter::connection_from_db_value)
            .collect())
    })
}

fn load_ssh_file_mappings(state: &SqliteDbState) -> Result<Vec<SSHFileMapping>, String> {
    let order = ssh_mapping_order()?;
    state.with_conn(|conn| {
        Ok(db_list(conn, DbTable::SshFileMapping, Some(&order))?
            .into_iter()
            .map(adapter::mapping_from_db_value)
            .collect())
    })
}

/// 内部共享函数：从数据库读取完整 SSH 配置
/// 参数 include_file_mappings 控制是否加载 file_mappings（mcp_sync/skills_sync 不需要）
pub async fn get_ssh_config_internal(
    db: &SqliteDbState,
    include_file_mappings: bool,
) -> Result<SSHSyncConfig, String> {
    let file_mappings = if include_file_mappings {
        let file_mappings = load_ssh_file_mappings(db)?;
        backfill_default_file_mappings(db, file_mappings).await
    } else {
        vec![]
    };
    let config_record = load_ssh_config_record(db)?;
    let connections = load_ssh_connections(db)?;
    let module_statuses = runtime_location::get_wsl_direct_status_map_async(db).await?;

    match config_record {
        Some(record) => {
            let mut config = adapter::config_from_db_value(record, file_mappings, connections);
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
    db: &SqliteDbState,
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

/// Get SSH sync configuration (config + connections + file file_mappings)
#[tauri::command]
pub async fn ssh_get_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<SSHSyncConfig, String> {
    let db = state.db();
    get_ssh_config_internal(db, true).await
}

/// Save SSH sync configuration (enabled, active_connection_id, etc.)
#[tauri::command]
pub async fn ssh_save_config(
    state: tauri::State<'_, SqliteDbState>,
    session_state: tauri::State<'_, SshSessionState>,
    app: tauri::AppHandle,
    config: SSHSyncConfig,
) -> Result<(), String> {
    // Check if being enabled
    let was_enabled = {
        let db = state.db();
        load_ssh_config_record(db)?
            .and_then(|record| record.get("enabled").and_then(|value| value.as_bool()))
            .unwrap_or(false)
    };

    let is_being_enabled = !was_enabled && config.enabled;

    {
        let config_data = adapter::config_to_db_value(&config);
        state.with_conn(|conn| db_put(conn, DbTable::SshSyncConfig, "config", &config_data))?;

        // Update file file_mappings
        for mapping in config.file_mappings.iter() {
            let mapping_data = adapter::mapping_to_db_value(mapping);
            state.with_conn(|conn| {
                db_put(conn, DbTable::SshFileMapping, &mapping.id, &mapping_data)
            })?;
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
        let mut config_data = load_ssh_config_record(state.db())?
            .unwrap_or_else(|| adapter::config_to_db_value(&SSHSyncConfig::default()));
        if let Some(payload) = config_data.as_object_mut() {
            payload.insert("last_sync_status".to_string(), serde_json::Value::Null);
            payload.insert("last_sync_error".to_string(), serde_json::Value::Null);
        }
        state.with_conn(|conn| db_put(conn, DbTable::SshSyncConfig, "config", &config_data))?;
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
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<SSHConnection>, String> {
    load_ssh_connections(state.db())
}

/// Create a new SSH connection preset
#[tauri::command]
pub async fn ssh_create_connection(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    mut connection: SSHConnection,
) -> Result<(), String> {
    normalise_key_fields(&mut connection);

    let conn_data = adapter::connection_to_db_value(&connection);
    state.with_conn(|conn| db_put(conn, DbTable::SshConnection, &connection.id, &conn_data))?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

/// Update an existing SSH connection preset
#[tauri::command]
pub async fn ssh_update_connection(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    mut connection: SSHConnection,
) -> Result<(), String> {
    normalise_key_fields(&mut connection);

    let conn_data = adapter::connection_to_db_value(&connection);
    state.with_conn(|conn| db_put(conn, DbTable::SshConnection, &connection.id, &conn_data))?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

/// Delete an SSH connection preset
#[tauri::command]
pub async fn ssh_delete_connection(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    state.with_conn(|conn| {
        db_delete(conn, DbTable::SshConnection, &id)?;
        if let Some(mut config_data) = db_get(conn, DbTable::SshSyncConfig, "config")? {
            if config_data
                .get("active_connection_id")
                .and_then(|value| value.as_str())
                == Some(id.as_str())
            {
                if let Some(payload) = config_data.as_object_mut() {
                    payload.insert(
                        "active_connection_id".to_string(),
                        serde_json::Value::String(String::new()),
                    );
                }
                db_put(conn, DbTable::SshSyncConfig, "config", &config_data)?;
            }
        }
        Ok(())
    })?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

/// Set active connection (and optionally trigger sync)
#[tauri::command]
pub async fn ssh_set_active_connection(
    state: tauri::State<'_, SqliteDbState>,
    session_state: tauri::State<'_, SshSessionState>,
    app: tauri::AppHandle,
    connection_id: String,
) -> Result<(), String> {
    {
        state.with_conn(|conn| {
            let mut config_data = db_get(conn, DbTable::SshSyncConfig, "config")?
                .unwrap_or_else(|| adapter::config_to_db_value(&SSHSyncConfig::default()));
            if let Some(payload) = config_data.as_object_mut() {
                payload.insert(
                    "active_connection_id".to_string(),
                    serde_json::Value::String(connection_id.clone()),
                );
            }
            db_put(conn, DbTable::SshSyncConfig, "config", &config_data)
        })?;
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
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    mapping: SSHFileMapping,
) -> Result<(), String> {
    let mapping_data = adapter::mapping_to_db_value(&mapping);
    state.with_conn(|conn| db_put(conn, DbTable::SshFileMapping, &mapping.id, &mapping_data))?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

/// Update an existing SSH file mapping
#[tauri::command]
pub async fn ssh_update_file_mapping(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    mapping: SSHFileMapping,
) -> Result<(), String> {
    let mapping_data = adapter::mapping_to_db_value(&mapping);
    state.with_conn(|conn| db_put(conn, DbTable::SshFileMapping, &mapping.id, &mapping_data))?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

/// Delete an SSH file mapping
#[tauri::command]
pub async fn ssh_delete_file_mapping(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    state.with_conn(|conn| db_delete(conn, DbTable::SshFileMapping, &id).map(|_| ()))?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

/// Reset all SSH file file_mappings
#[tauri::command]
pub async fn ssh_reset_file_mappings(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    state.with_conn(|conn| db_delete_all(conn, DbTable::SshFileMapping).map(|_| ()))?;

    let _ = app.emit("ssh-config-changed", ());
    Ok(())
}

// ============================================================================
// Sync Commands
// ============================================================================

/// Internal full sync implementation
pub async fn do_full_sync(
    state: &SqliteDbState,
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
        "SSH full sync start: module={:?}, skip_modules={:?}, file_mappings_total={}, file_mappings_enabled={}, file_mappings_disabled={}, sync_mcp={}, sync_skills={}",
        module,
        skip_modules,
        total_mapping_count,
        enabled_mapping_count,
        disabled_mapping_count,
        config.sync_mcp,
        config.sync_skills
    );

    // Emit initial progress
    let enabled_file_mappings: Vec<_> = config.file_mappings.iter().filter(|m| m.enabled).collect();
    let total_files = enabled_file_mappings.len() as u32;
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
        "SSH full sync resolved dynamic file_mappings: resolved_count={}",
        file_mappings.len()
    );

    // Sync file file_mappings with progress
    let mut result =
        sync_file_mappings_with_progress(&file_mappings, session, module, skip_modules, app).await;
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
    db: &SqliteDbState,
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

/// Sync file file_mappings with progress events
async fn sync_file_mappings_with_progress(
    file_mappings: &[SSHFileMapping],
    session: &SshSession,
    module_filter: Option<&str>,
    skip_modules: Option<&[String]>,
    app: &tauri::AppHandle,
) -> SyncResult {
    let mut synced_files = vec![];
    let mut skipped_files = vec![];
    let mut errors = vec![];
    let mut filtered_file_mappings = Vec::new();
    let mut disabled_mapping_count = 0usize;
    let mut filtered_by_module_count = 0usize;
    let mut filtered_by_skip_modules_count = 0usize;

    for mapping in file_mappings {
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
        filtered_file_mappings.push(mapping);
    }

    let total = filtered_file_mappings.len() as u32;
    log::info!(
        "SSH sync mapping filter summary: total_file_mappings={}, selected_file_mappings={}, disabled_file_mappings={}, filtered_by_module={}, filtered_by_skip_modules={}, module_filter={:?}, skip_modules={:?}",
        file_mappings.len(),
        filtered_file_mappings.len(),
        disabled_mapping_count,
        filtered_by_module_count,
        filtered_by_skip_modules_count,
        module_filter,
        skip_modules
    );

    for (idx, mapping) in filtered_file_mappings.iter().enumerate() {
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
    state: tauri::State<'_, SqliteDbState>,
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
        "SSH sync requested: module={:?}, skip_modules={:?}, enabled={}, active_connection_id={}, active_connection_name={:?}, file_mappings_total={}, file_mappings_enabled={}",
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
            "SSH sync finished without uploading main file file_mappings: skipped_files={}, module={:?}, skip_modules={:?}",
            result.skipped_files.len(),
            module,
            skip_modules
        );
    }

    Ok(result)
}

/// Get SSH sync status
#[tauri::command]
pub async fn ssh_get_status(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<SSHStatusResult, String> {
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

/// Auto-insert any default file_mappings whose IDs are missing from the database.
/// This ensures upgrading users get newly added default file_mappings (e.g. OpenClaw).
///
/// Uses a version guard (`ssh_defaults_version`) so the migration runs only once
/// per schema bump. If the user deletes a backfilled mapping afterwards, it will
/// NOT be re-added.
async fn backfill_default_file_mappings(
    db: &SqliteDbState,
    mut file_mappings: Vec<SSHFileMapping>,
) -> Vec<SSHFileMapping> {
    // Bump this number whenever new default file_mappings are added.
    const CURRENT_DEFAULTS_VERSION: u64 = 5;

    // Read stored version
    let stored_version: u64 = db
        .with_conn(|conn| db_get(conn, DbTable::SshSyncConfig, "defaults_version"))
        .ok()
        .flatten()
        .and_then(|value| value.get("version").and_then(|value| value.as_u64()))
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
            if let Err(e) = db.with_conn(|conn| {
                db_put(
                    conn,
                    DbTable::SshFileMapping,
                    &default_mapping.id,
                    &mapping_data,
                )
            }) {
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
    let version_data = serde_json::json!({ "version": CURRENT_DEFAULTS_VERSION });
    let _ = db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::SshSyncConfig,
            "defaults_version",
            &version_data,
        )
    });

    file_mappings
}

/// Dynamically resolve config file paths for OpenCode and Oh My OpenAgent.
pub fn resolve_dynamic_paths(file_mappings: Vec<SSHFileMapping>) -> Vec<SSHFileMapping> {
    file_mappings
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
    db: &SqliteDbState,
    file_mappings: Vec<SSHFileMapping>,
) -> Vec<SSHFileMapping> {
    let mut resolved = Vec::with_capacity(file_mappings.len());
    for mut mapping in resolve_dynamic_paths(file_mappings) {
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
pub async fn update_sync_status(state: &SqliteDbState, result: &SyncResult) -> Result<(), String> {
    let (status, error) = if result.success {
        ("success".to_string(), None)
    } else {
        let error_msg = result.errors.join("; ");
        ("error".to_string(), Some(error_msg))
    };

    let now = Local::now().to_rfc3339();

    let mut config_data = load_ssh_config_record(state)?
        .unwrap_or_else(|| adapter::config_to_db_value(&SSHSyncConfig::default()));
    if let Some(payload) = config_data.as_object_mut() {
        payload.insert("last_sync_time".to_string(), serde_json::Value::String(now));
        payload.insert(
            "last_sync_status".to_string(),
            serde_json::Value::String(status),
        );
        payload.insert(
            "last_sync_error".to_string(),
            error
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        );
    }
    state.with_conn(|conn| db_put(conn, DbTable::SshSyncConfig, "config", &config_data))?;

    Ok(())
}

/// Get default file file_mappings for SSH sync
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
            directory_excludes: super::types::default_directory_excludes_for_mapping(
                super::types::CLAUDE_PLUGINS_MAPPING_ID,
            ),
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
    state: &SqliteDbState,
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

#[cfg(test)]
mod tests {
    use super::default_file_mappings;

    #[test]
    fn claude_plugins_default_mapping_keeps_plugin_cache_available() {
        let mapping = default_file_mappings()
            .into_iter()
            .find(|mapping| mapping.id == "claude-plugins")
            .expect("claude-plugins default mapping exists");

        assert!(mapping.directory_excludes.contains(&".venv".to_string()));
        assert!(mapping
            .directory_excludes
            .contains(&"node_modules".to_string()));
        assert!(!mapping.directory_excludes.contains(&"cache".to_string()));
    }

    #[test]
    fn codex_plugins_default_mapping_still_uses_generic_directory_excludes() {
        let mapping = default_file_mappings()
            .into_iter()
            .find(|mapping| mapping.id == "codex-plugins")
            .expect("codex-plugins default mapping exists");

        assert!(mapping.directory_excludes.contains(&"cache".to_string()));
    }
}
