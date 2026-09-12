use super::types::{
    FileMapping, SyncProgress, SyncResult, WSLDetectResult, WSLErrorResult, WSLStatusResult,
    WSLSyncConfig,
};
use super::{adapter, sync};
use crate::coding::claude_code::plugin_metadata_sync;
use crate::coding::codex::constants::AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME;
use crate::coding::config_cleanup;
use crate::coding::dsh::constants::{
    DSH_CREDENTIALS_FILE, DSH_MCP_FILE, DSH_PROMPT_FILE, DSH_SETTINGS_FILE,
};
use crate::coding::oh_my_pi::constants::{
    OMP_CONFIG_FILE, OMP_MCP_FILE, OMP_MODELS_FILE, OMP_PROMPT_FILE,
};
use crate::coding::pi::constants::{
    PI_AUTH_FILE, PI_MCP_FILE, PI_MODELS_FILE, PI_PROMPT_FILE, PI_SETTINGS_FILE,
};
use crate::coding::proxy_gateway::{
    cli_proxy, paths::ProxyGatewayPaths, settings as proxy_gateway_settings,
    types::ProxyGatewaySettings,
};
use crate::coding::runtime_location;
use crate::db::helpers::{db_delete, db_delete_all, db_get, db_list, db_patch_fields, db_put};
use crate::db::schema::{DbTable, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use chrono::Local;
use std::path::Path;
use tauri::{Emitter, Manager};

// ============================================================================
// WSL Detection Commands
// ============================================================================

/// Detect WSL availability and get distro list
#[tauri::command]
pub fn wsl_detect() -> WSLDetectResult {
    sync::detect_wsl()
}

/// Check if a specific WSL distro is available
#[tauri::command]
pub fn wsl_check_distro(distro: String) -> WSLErrorResult {
    match sync::get_effective_distro(&distro) {
        Ok(_) => WSLErrorResult {
            available: true,
            error: None,
        },
        Err(e) => WSLErrorResult {
            available: false,
            error: Some(e),
        },
    }
}

/// Get running state of a specific WSL distro
#[tauri::command]
pub fn wsl_get_distro_state(distro: String) -> String {
    match sync::get_effective_distro(&distro) {
        Ok(effective_distro) => sync::get_wsl_distro_state(&effective_distro),
        Err(_) => "Unknown".to_string(),
    }
}

// ============================================================================
// WSL Config Commands
// ============================================================================

fn wsl_mapping_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_text("module", OrderDirection::Asc)?,
        OrderField::json_text("name", OrderDirection::Asc)?,
    ]))
}

fn load_wsl_config(state: &SqliteDbState) -> Result<WSLSyncConfig, String> {
    state.with_conn(|conn| {
        Ok(db_get(conn, DbTable::WslSyncConfig, "config")?
            .map(|record| adapter::config_from_db_value(record, vec![]))
            .unwrap_or_default())
    })
}

fn patch_wsl_config_record(
    state: &SqliteDbState,
    fields: &[(&str, serde_json::Value)],
) -> Result<(), String> {
    // Keep the complete read/modify/write under one connection lock. Settings,
    // file sync status, and Skills warnings can be updated independently.
    state.with_conn(|conn| {
        if db_get(conn, DbTable::WslSyncConfig, "config")?.is_none() {
            db_put(
                conn,
                DbTable::WslSyncConfig,
                "config",
                &adapter::config_to_db_value(&WSLSyncConfig::default()),
            )?;
        }
        db_patch_fields(conn, DbTable::WslSyncConfig, "config", fields)?;
        Ok(())
    })
}

fn save_wsl_config_record(state: &SqliteDbState, config: &WSLSyncConfig) -> Result<(), String> {
    patch_wsl_config_record(
        state,
        &[
            ("enabled", serde_json::json!(config.enabled)),
            ("distro", serde_json::json!(config.distro)),
        ],
    )
}

fn load_wsl_file_mappings(state: &SqliteDbState) -> Result<Vec<FileMapping>, String> {
    let order = wsl_mapping_order()?;
    state.with_conn(|conn| {
        Ok(db_list(conn, DbTable::WslFileMapping, Some(&order))?
            .into_iter()
            .map(adapter::mapping_from_db_value)
            .collect())
    })
}

/// Get WSL sync configuration
#[tauri::command]
pub async fn wsl_get_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<WSLSyncConfig, String> {
    let db = state.db();

    let config = load_wsl_config(db).unwrap_or_default();
    let file_mappings = load_wsl_file_mappings(db).unwrap_or_default();

    // Auto-insert missing default mappings for upgrading users
    let file_mappings = backfill_default_mappings(db, file_mappings).await;
    let module_statuses = runtime_location::get_wsl_direct_status_map_async(db).await?;

    Ok(WSLSyncConfig {
        file_mappings,
        module_statuses,
        ..config
    })
}

/// Save WSL sync configuration
#[tauri::command]
pub async fn wsl_save_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config: WSLSyncConfig,
) -> Result<(), String> {
    // Check if WSL sync is being enabled (was disabled, now enabled)
    let was_enabled = {
        let db = state.db();
        load_wsl_config(db)?.enabled
    };

    let is_being_enabled = !was_enabled && config.enabled;

    for mapping in config.file_mappings.iter() {
        validate_file_mapping_cleanup_paths(mapping)?;
    }

    {
        save_wsl_config_record(&state, &config)?;

        // Update file mappings - follow open_code/free_models pattern: use backtick format table:`id`
        for mapping in config.file_mappings.iter() {
            let mapping_data = adapter::mapping_to_db_value(mapping);
            state.with_conn(|conn| {
                db_put(conn, DbTable::WslFileMapping, &mapping.id, &mapping_data)
            })?;
        }
    }

    // Emit event to refresh UI
    let _ = app.emit("wsl-config-changed", ());

    // If WSL sync was just enabled, trigger a full sync
    if is_being_enabled {
        log::info!("WSL sync enabled, triggering full sync...");

        let result = do_full_sync(&state, &app, &config, None, None).await;

        if !result.errors.is_empty() {
            log::warn!("WSL full sync errors: {:?}", result.errors);
        }

        // Update sync status
        update_sync_status(state.inner(), &result).await?;

        // Emit sync completed event
        let _ = app.emit("wsl-sync-completed", result);
    }

    Ok(())
}

// ============================================================================
// File Mapping Commands
// ============================================================================

fn validate_file_mapping_cleanup_paths(mapping: &FileMapping) -> Result<(), String> {
    config_cleanup::cleanup_paths_for_mapping(
        mapping.is_directory,
        mapping.is_pattern,
        &mapping.wsl_path,
        &mapping.windows_path,
        &mapping.cleanup_paths,
    )
    .map(|_| ())
}

/// Add a new file mapping
#[tauri::command]
pub async fn wsl_add_file_mapping(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    mapping: FileMapping,
) -> Result<(), String> {
    validate_file_mapping_cleanup_paths(&mapping)?;
    let mapping_data = adapter::mapping_to_db_value(&mapping);
    state.with_conn(|conn| db_put(conn, DbTable::WslFileMapping, &mapping.id, &mapping_data))?;

    let _ = app.emit("wsl-config-changed", ());

    Ok(())
}

/// Update an existing file mapping
#[tauri::command]
pub async fn wsl_update_file_mapping(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    mapping: FileMapping,
) -> Result<(), String> {
    validate_file_mapping_cleanup_paths(&mapping)?;
    let mapping_data = adapter::mapping_to_db_value(&mapping);
    state.with_conn(|conn| db_put(conn, DbTable::WslFileMapping, &mapping.id, &mapping_data))?;

    let _ = app.emit("wsl-config-changed", ());

    Ok(())
}

/// Delete a file mapping
#[tauri::command]
pub async fn wsl_delete_file_mapping(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    state.with_conn(|conn| db_delete(conn, DbTable::WslFileMapping, &id).map(|_| ()))?;

    let _ = app.emit("wsl-config-changed", ());

    Ok(())
}

/// Delete all file mappings (reset)
#[tauri::command]
pub async fn wsl_reset_file_mappings(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    state.with_conn(|conn| db_delete_all(conn, DbTable::WslFileMapping).map(|_| ()))?;

    let _ = app.emit("wsl-config-changed", ());

    Ok(())
}

// ============================================================================
// Sync Commands
// ============================================================================

/// Internal full sync implementation (reusable)
pub(super) async fn do_full_sync(
    state: &SqliteDbState,
    app: &tauri::AppHandle,
    config: &WSLSyncConfig,
    module: Option<&str>,
    skip_modules: Option<&[String]>,
) -> SyncResult {
    // In particular, enabling sync receives a frontend config snapshot that
    // may predate a root-directory change. Resolve Direct status on the backend.
    let direct_modules = match runtime_location::get_wsl_direct_modules_async(state.db()).await {
        Ok(modules) => modules,
        Err(error) => {
            return SyncResult {
                success: false,
                synced_files: vec![],
                skipped_files: vec![],
                errors: vec![error],
                warnings: vec![],
            };
        }
    };
    let merged_skip_modules = merge_skip_modules(skip_modules, &direct_modules);

    // Get effective distro (auto-resolve if configured one doesn't exist)
    let distro = match sync::get_effective_distro(&config.distro) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("WSL full sync skipped: {}", e);
            return SyncResult {
                success: false,
                synced_files: vec![],
                skipped_files: vec![],
                errors: vec![e],
                warnings: vec![],
            };
        }
    };

    // Emit initial progress for file mappings
    let enabled_mappings: Vec<_> = config.file_mappings.iter().filter(|m| m.enabled).collect();
    let total_files = enabled_mappings.len() as u32;
    let _ = app.emit(
        "wsl-sync-progress",
        SyncProgress {
            phase: "files".to_string(),
            current_item: "准备中...".to_string(),
            current: 0,
            total: total_files,
            message: format!("文件同步: 0/{}", total_files),
            current_file: None,
        },
    );

    // Resolve effective local/WSL paths based on current runtime locations.
    let db = state.db();
    let file_mappings = resolve_dynamic_paths_with_db(&db, config.file_mappings.clone()).await;
    let gateway_wsl_rewrite_context = build_gateway_wsl_rewrite_context(&db, app);

    // Sync file mappings with progress
    let mut result = sync_mappings_with_progress(
        &file_mappings,
        &distro,
        module,
        Some(merged_skip_modules.as_slice()),
        app,
        gateway_wsl_rewrite_context.as_ref(),
    );

    let skip_claude = merged_skip_modules.iter().any(|m| m == "claude");
    if !skip_claude && (module.is_none() || module == Some("claude")) {
        if let Err(error) = rewrite_claude_plugin_metadata_in_wsl(&db, &distro).await {
            log::warn!("Claude plugin metadata WSL rewrite failed: {}", error);
            result
                .errors
                .push(format!("Claude plugins metadata rewrite: {}", error));
        }
    }

    // Also sync MCP and Skills to WSL (full sync)
    if config.sync_mcp {
        if let Err(e) = super::mcp_sync::sync_mcp_to_wsl(state, app.clone()).await {
            log::warn!("MCP WSL sync failed: {}", e);
            result.errors.push(format!("MCP sync: {}", e));
            result.success = false;
        }
    }
    if config.sync_skills {
        let mut skills_warnings = Vec::new();
        let skills_result = super::skills_sync::sync_skills_to_wsl_with_warnings(
            state,
            app.clone(),
            &mut skills_warnings,
        )
        .await;
        result.warnings.extend(skills_warnings);
        if let Err(e) = skills_result {
            log::warn!("Skills WSL sync failed: {}", e);
            result.errors.push(format!("Skills sync: {}", e));
            result.success = false;
        }
    }

    // Sync Claude Code onboarding status from Windows to WSL
    // Mirror the hasCompletedOnboarding field so WSL skips/shows initial setup accordingly
    if !skip_claude && (module.is_none() || module == Some("claude")) {
        if let Err(e) = sync_onboarding_to_wsl(state, &distro).await {
            log::warn!("Onboarding WSL sync failed: {}", e);
            result.errors.push(format!("Onboarding sync: {}", e));
            result.success = false;
        }
    }

    // Ensure OpenClaw config exists in WSL (create empty {} if missing)
    let skip_openclaw = merged_skip_modules.iter().any(|m| m == "openclaw");
    if !skip_openclaw && (module.is_none() || module == Some("openclaw")) {
        if let Err(e) = ensure_openclaw_config_in_wsl(state, &distro).await {
            log::warn!("OpenClaw WSL config init failed: {}", e);
        }
    }

    result
}

async fn rewrite_claude_plugin_metadata_in_wsl(
    db: &SqliteDbState,
    distro: &str,
) -> Result<(), String> {
    let source_plugins_root = runtime_location::get_claude_plugins_dir_async(db)
        .await?
        .to_string_lossy()
        .to_string();
    let target_plugins_root_raw =
        runtime_location::get_claude_wsl_target_path_async(db, "plugins").await;

    // Claude CLI 2.1.126+ validates marketplace `installLocation` / `installPath`
    // as a literal Linux path and does NOT expand `~`. The WSL read/write helpers
    // expand `~` via bash `$HOME`, so file paths still work — but the same `~` must
    // not survive into the JSON values we write back. Resolve the real Linux home
    // once and substitute it before handing the string to the rewrite helper.
    let target_plugins_root = expand_tilde_with_wsl_home(distro, &target_plugins_root_raw)?;

    for file_name in ["known_marketplaces.json", "installed_plugins.json"] {
        let target_file_path = format!(
            "{}/{}",
            target_plugins_root_raw.trim_end_matches('/'),
            file_name
        );
        let existing_content = sync::read_wsl_file(distro, &target_file_path)?;
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

        sync::write_wsl_file(distro, &target_file_path, &rewritten_content)?;
    }

    Ok(())
}

fn expand_tilde_with_wsl_home(distro: &str, path: &str) -> Result<String, String> {
    if !path.starts_with('~') {
        return Ok(path.to_string());
    }
    let home = sync::get_wsl_user_home(distro)?;
    Ok(runtime_location::expand_home_from_user_root(
        Some(&home),
        path,
    ))
}

#[derive(Debug, Clone)]
struct GatewayWslRewriteContext {
    paths: ProxyGatewayPaths,
    settings: ProxyGatewaySettings,
}

fn build_gateway_wsl_rewrite_context(
    db: &SqliteDbState,
    app: &tauri::AppHandle,
) -> Option<GatewayWslRewriteContext> {
    let settings = match proxy_gateway_settings::load_settings_from_sqlite_state(db) {
        Ok(settings) => settings,
        Err(error) => {
            log::warn!("Gateway WSL endpoint rewrite skipped: {}", error);
            return None;
        }
    };
    if settings.wsl_host.trim().is_empty() {
        return None;
    }

    let app_data_dir = match app.path().app_data_dir() {
        Ok(path) => path,
        Err(error) => {
            log::warn!("Gateway WSL endpoint rewrite skipped: {}", error);
            return None;
        }
    };

    Some(GatewayWslRewriteContext {
        paths: ProxyGatewayPaths::new(app_data_dir),
        settings,
    })
}

/// Sync file mappings with progress events
fn sync_mappings_with_progress(
    mappings: &[FileMapping],
    distro: &str,
    module_filter: Option<&str>,
    skip_modules: Option<&[String]>,
    app: &tauri::AppHandle,
    gateway_wsl_rewrite_context: Option<&GatewayWslRewriteContext>,
) -> SyncResult {
    let mut synced_files = vec![];
    let mut skipped_files = vec![];
    let mut errors = vec![];

    let filtered_mappings: Vec<_> = mappings
        .iter()
        .filter(|m| m.enabled)
        .filter(|m| module_filter.is_none() || Some(m.module.as_str()) == module_filter)
        .filter(|m| skip_modules.map_or(true, |skip| !skip.iter().any(|s| s == &m.module)))
        .collect();

    let total = filtered_mappings.len() as u32;

    for (idx, mapping) in filtered_mappings.iter().enumerate() {
        let current = (idx + 1) as u32;

        // Emit progress
        let _ = app.emit(
            "wsl-sync-progress",
            SyncProgress {
                phase: "files".to_string(),
                current_item: mapping.name.clone(),
                current,
                total,
                message: format!("文件同步: {}/{} - {}", current, total, mapping.name),
                current_file: None,
            },
        );

        match sync::sync_file_mapping(mapping, distro) {
            Ok(mut files) => {
                if !files.is_empty() {
                    match rewrite_gateway_managed_wsl_copy(
                        mapping,
                        distro,
                        gateway_wsl_rewrite_context,
                    ) {
                        Ok(Some(rewritten_file)) => files.push(rewritten_file),
                        Ok(None) => {}
                        Err(error) => errors.push(format!("{}: {}", mapping.name, error)),
                    }

                    match cleanup_synced_file_in_wsl(mapping, distro) {
                        Ok(Some(cleaned_file)) => files.push(cleaned_file),
                        Ok(None) => {}
                        Err(error) => errors.push(format!("{}: {}", mapping.name, error)),
                    }
                }

                match reconcile_codex_prompt_files_in_wsl(mapping, distro) {
                    Ok(prompt_files) => files.extend(prompt_files),
                    Err(error) => errors.push(format!("{}: {}", mapping.name, error)),
                }
                match sync_codex_model_catalog_in_wsl(mapping, distro) {
                    Ok(catalog_files) => files.extend(catalog_files),
                    Err(error) => errors.push(format!("{}: {}", mapping.name, error)),
                }
                if files.is_empty() {
                    skipped_files.push(mapping.name.clone());
                    continue;
                }
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
        warnings: vec![],
    }
}

fn rewrite_gateway_managed_wsl_copy(
    mapping: &FileMapping,
    distro: &str,
    context: Option<&GatewayWslRewriteContext>,
) -> Result<Option<String>, String> {
    if mapping.is_directory || mapping.is_pattern {
        return Ok(None);
    }
    let Some(context) = context else {
        return Ok(None);
    };
    let Some((cli_key, target_kind)) =
        cli_proxy::wsl_synced_gateway_target_for_mapping(&mapping.id)
    else {
        return Ok(None);
    };

    let content = sync::read_wsl_file(distro, &mapping.wsl_path)?;
    let Some(rewritten_content) = cli_proxy::rewrite_wsl_synced_gateway_target_content(
        &context.paths,
        &context.settings,
        cli_key,
        target_kind,
        &content,
    )?
    else {
        return Ok(None);
    };

    sync::write_wsl_file(distro, &mapping.wsl_path, &rewritten_content)?;
    Ok(Some(format!(
        "Gateway WSL endpoint rewrite: {}",
        mapping.wsl_path
    )))
}

fn cleanup_synced_file_in_wsl(
    mapping: &FileMapping,
    distro: &str,
) -> Result<Option<String>, String> {
    let mut cleanup_paths = Vec::new();
    if mapping.id == "claude-settings" {
        cleanup_paths.extend(
            config_cleanup::CLAUDE_NON_WINDOWS_TARGET_CLEANUP_PATHS
                .iter()
                .map(|path| (*path).to_string()),
        );
    }
    cleanup_paths.extend(mapping.cleanup_paths.iter().cloned());

    let cleanup_paths = config_cleanup::cleanup_paths_for_mapping(
        mapping.is_directory,
        mapping.is_pattern,
        &mapping.wsl_path,
        &mapping.windows_path,
        &cleanup_paths,
    )?;
    if cleanup_paths.is_empty() {
        return Ok(None);
    }

    let format = config_cleanup::cleanup_file_format_for_mapping_paths(
        &mapping.wsl_path,
        &mapping.windows_path,
    )
    .ok_or_else(|| "字段清理路径仅支持 JSON/TOML 单文件映射".to_string())?;
    let content = sync::read_wsl_file(distro, &mapping.wsl_path)?;
    let Some(cleaned_content) =
        config_cleanup::apply_cleanup_paths_to_content(&content, format, &cleanup_paths)?
    else {
        return Ok(None);
    };

    sync::write_wsl_file(distro, &mapping.wsl_path, &cleaned_content)?;
    Ok(Some(format!("Field cleanup: {}", mapping.wsl_path)))
}

fn reconcile_codex_prompt_files_in_wsl(
    mapping: &FileMapping,
    distro: &str,
) -> Result<Vec<String>, String> {
    if mapping.id != "codex-prompt" || mapping.is_directory || mapping.is_pattern {
        return Ok(vec![]);
    }

    if runtime_location::parse_wsl_unc_path(&mapping.windows_path).is_some() {
        return Ok(vec![]);
    }

    let mut synced_files = Vec::new();
    for file_name in runtime_location::CODEX_PROMPT_FILE_NAMES {
        let windows_path =
            runtime_location::replace_path_file_name(&mapping.windows_path, file_name);
        let wsl_path = runtime_location::replace_path_file_name(&mapping.wsl_path, file_name);
        let expanded_windows_path = sync::expand_env_vars(&windows_path)?;

        if Path::new(&expanded_windows_path).exists() {
            if windows_path == mapping.windows_path && wsl_path == mapping.wsl_path {
                continue;
            }
            synced_files.extend(sync::sync_single_file(
                &expanded_windows_path,
                &wsl_path,
                distro,
            )?);
        } else {
            sync::remove_wsl_path(distro, &wsl_path)?;
            synced_files.push(format!("removed stale Codex prompt: {}", wsl_path));
        }
    }

    Ok(synced_files)
}

fn codex_config_uses_ai_toolbox_model_catalog(config_toml: &str) -> bool {
    let Ok(document) = config_toml.parse::<toml_edit::DocumentMut>() else {
        return false;
    };

    document
        .get("model_catalog_json")
        .and_then(|item| item.as_str())
        == Some(AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME)
}

fn sync_codex_model_catalog_in_wsl(
    mapping: &FileMapping,
    distro: &str,
) -> Result<Vec<String>, String> {
    if mapping.id != "codex-config" || mapping.is_directory || mapping.is_pattern {
        return Ok(vec![]);
    }

    if runtime_location::parse_wsl_unc_path(&mapping.windows_path).is_some() {
        return Ok(vec![]);
    }

    let expanded_config_path = sync::expand_env_vars(&mapping.windows_path)?;
    if !Path::new(&expanded_config_path).exists() {
        return Ok(vec![]);
    }

    let config_toml = std::fs::read_to_string(&expanded_config_path)
        .map_err(|error| format!("Failed to read Codex config.toml: {}", error))?;
    if !codex_config_uses_ai_toolbox_model_catalog(&config_toml) {
        return Ok(vec![]);
    }

    let windows_catalog_path = runtime_location::replace_path_file_name(
        &mapping.windows_path,
        AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME,
    );
    let wsl_catalog_path = runtime_location::replace_path_file_name(
        &mapping.wsl_path,
        AI_TOOLBOX_CODEX_MODEL_CATALOG_FILENAME,
    );
    let expanded_catalog_path = sync::expand_env_vars(&windows_catalog_path)?;

    if Path::new(&expanded_catalog_path).exists() {
        sync::sync_single_file(&expanded_catalog_path, &wsl_catalog_path, distro)
    } else {
        sync::remove_wsl_path(distro, &wsl_catalog_path)?;
        Ok(vec![format!(
            "removed stale Codex model catalog: {}",
            wsl_catalog_path
        )])
    }
}

/// Sync all files or specific module to WSL
#[tauri::command]
pub async fn wsl_sync(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    module: Option<String>,
    skip_modules: Option<Vec<String>>,
) -> Result<SyncResult, String> {
    let config = wsl_get_config(state.clone()).await?;

    let result = do_full_sync(
        &state,
        &app,
        &config,
        module.as_deref(),
        skip_modules.as_deref(),
    )
    .await;

    // Update sync status
    update_sync_status(state.inner(), &result).await?;

    // Emit event to update UI
    let _ = app.emit("wsl-sync-completed", result.clone());

    Ok(result)
}

/// Whether WSL automatic sync triggers are enabled.
///
/// Automatic triggers include startup sync and event-driven sync from
/// model/MCP/skills changes. Manual sync is intentionally not gated by this.
pub async fn is_wsl_auto_sync_enabled(state: &SqliteDbState) -> bool {
    state
        .with_conn(|conn| db_get(conn, DbTable::WslSyncConfig, "config"))
        .ok()
        .flatten()
        .and_then(|record| record.get("enabled").and_then(|v| v.as_bool()))
        .unwrap_or(false)
}

/// Remove the WSL target for an enabled file mapping when automatic sync is on.
///
/// Normal file sync intentionally skips missing local sources. Delete-style tool
/// actions must call this helper before clearing local state, otherwise the WSL
/// target keeps the stale runtime file.
pub async fn remove_auto_synced_wsl_mapping_target(
    state: &SqliteDbState,
    mapping_id: &str,
) -> Result<bool, String> {
    let db = state.db();

    let config = load_wsl_config(db).unwrap_or_default();

    if !config.enabled {
        return Ok(false);
    }

    let file_mappings = load_wsl_file_mappings(db).unwrap_or_default();

    let file_mappings = backfill_default_mappings(db, file_mappings).await;
    let file_mappings = resolve_dynamic_paths_with_db(db, file_mappings).await;
    let Some(mapping) = file_mappings
        .into_iter()
        .find(|mapping| mapping.id == mapping_id)
    else {
        return Ok(false);
    };

    if !mapping.enabled {
        return Ok(false);
    }

    if mapping.is_directory || mapping.is_pattern {
        return Err(format!(
            "Refusing to remove non-file WSL mapping target '{}'",
            mapping.id
        ));
    }

    if runtime_location::parse_wsl_unc_path(&mapping.windows_path).is_some() {
        return Ok(false);
    }

    let distro = sync::get_effective_distro(&config.distro)?;
    sync::remove_wsl_path(&distro, &mapping.wsl_path)?;

    Ok(true)
}

/// Get current WSL sync status
#[tauri::command]
pub async fn wsl_get_status(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<WSLStatusResult, String> {
    let config = wsl_get_config(state).await?;

    let wsl_available = if config.enabled {
        sync::get_effective_distro(&config.distro).is_ok()
    } else {
        false
    };

    Ok(WSLStatusResult {
        wsl_available,
        last_sync_time: config.last_sync_time,
        last_sync_status: config.last_sync_status,
        last_sync_error: config.last_sync_error,
        last_sync_warnings: config.last_sync_warnings,
        module_statuses: config.module_statuses,
    })
}

/// Test if a Windows path exists and can be accessed
#[tauri::command]
pub fn wsl_test_path(windows_path: String) -> Result<bool, String> {
    let expanded = sync::expand_env_vars(&windows_path)?;
    Ok(std::path::Path::new(&expanded).exists())
}

/// Get default file mappings
#[tauri::command]
pub fn wsl_get_default_mappings() -> Vec<FileMapping> {
    default_file_mappings()
}

// ============================================================================
// WSL UI Commands
// ============================================================================

/// Open WSL terminal for a specific distro
#[tauri::command]
#[cfg(target_os = "windows")]
pub fn wsl_open_terminal(distro: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let effective_distro = sync::get_effective_distro(&distro)?;

    std::process::Command::new("cmd")
        .args(["/c", "start", "wsl", "-d", &effective_distro, "--cd", "~"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Failed to open WSL terminal: {}", e))?;

    Ok(())
}

#[tauri::command]
#[cfg(not(target_os = "windows"))]
pub fn wsl_open_terminal(_distro: String) -> Result<(), String> {
    Err("WSL is only available on Windows".to_string())
}

/// Open Windows Explorer to WSL user's home directory
#[tauri::command]
#[cfg(target_os = "windows")]
pub fn wsl_open_folder(distro: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let effective_distro = sync::get_effective_distro(&distro)?;

    // Get actual home directory from WSL (handles root user whose home is /root, not /home/root)
    let output = std::process::Command::new("wsl")
        .args([
            "-d",
            &effective_distro,
            "--exec",
            "bash",
            "-c",
            "echo $HOME",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to get WSL home directory: {}", e))?;

    let home_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if home_dir.is_empty() {
        return Err("Failed to get WSL home directory".to_string());
    }

    // Convert WSL path (e.g. /root or /home/user) to UNC path: \\wsl$\<distro>\root or \\wsl$\<distro>\home\user
    let home_unix = home_dir.replace('/', "\\");
    let wsl_path = format!(r"\\wsl$\{}{}", effective_distro, home_unix);
    std::process::Command::new("explorer.exe")
        .arg(&wsl_path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Failed to open WSL folder: {}", e))?;

    Ok(())
}

#[tauri::command]
#[cfg(not(target_os = "windows"))]
pub fn wsl_open_folder(_distro: String) -> Result<(), String> {
    Err("WSL is only available on Windows".to_string())
}

// ============================================================================
// Internal Functions
// ============================================================================

/// Auto-insert newly added default mappings.
///
/// Uses a version guard (`wsl_defaults_version`) so the migration runs only once
/// per schema bump. If the user deletes a backfilled mapping afterwards, it will
/// NOT be re-added.
async fn backfill_default_mappings(
    db: &SqliteDbState,
    mut file_mappings: Vec<FileMapping>,
) -> Vec<FileMapping> {
    // Bump this number whenever new default mappings are added.
    const CURRENT_DEFAULTS_VERSION: u64 = 16;
    const DEFAULTS_VERSION_BEFORE_AGENT_DIRECTORIES: u64 = 7;
    const DEFAULT_MAPPING_IDS_ADDED_IN_V8: &[&str] = &["opencode-agents"];
    const DEFAULT_MAPPING_IDS_ADDED_IN_V9: &[&str] =
        &["grok-auth", "grok-config", "grok-prompt", "grok-plugins"];
    const DEFAULT_MAPPING_IDS_ADDED_IN_V11: &[&str] = &[
        "omp-config",
        "omp-models",
        "omp-mcp",
        "omp-agents",
        "omp-rules",
    ];
    const DEFAULT_MAPPING_IDS_ADDED_IN_V12: &[&str] = &["hermes-config", "hermes-prompt"];
    const DEFAULT_MAPPING_IDS_ADDED_IN_V13: &[&str] =
        &["dsh-config", "dsh-credentials", "dsh-prompt"];
    const DEFAULT_MAPPING_IDS_ADDED_IN_V14: &[&str] = &["dsh-mcp"];
    const DEFAULT_MAPPING_IDS_ADDED_IN_V16: &[&str] = &[
        "kimi-config",
        "kimi-prompt",
        "kimi-credentials",
        "kimi-plugins",
    ];

    // Read stored version
    let stored_version: u64 = db
        .with_conn(|conn| db_get(conn, DbTable::WslSyncConfig, "defaults_version"))
        .ok()
        .flatten()
        .and_then(|value| value.get("version").and_then(|value| value.as_u64()))
        .unwrap_or(0);

    if stored_version >= CURRENT_DEFAULTS_VERSION {
        return file_mappings;
    }

    let existing_mapping_count = file_mappings.len();
    let existing_ids: std::collections::HashSet<String> =
        file_mappings.iter().map(|m| m.id.clone()).collect();

    for default_mapping in default_file_mappings() {
        if !existing_ids.contains(&default_mapping.id)
            && (should_backfill_default_mapping(
                stored_version,
                existing_mapping_count,
                &default_mapping.id,
                DEFAULTS_VERSION_BEFORE_AGENT_DIRECTORIES,
                DEFAULT_MAPPING_IDS_ADDED_IN_V8,
            ) || should_backfill_versioned_mapping(
                stored_version,
                9,
                &default_mapping.id,
                DEFAULT_MAPPING_IDS_ADDED_IN_V9,
            ) || should_backfill_versioned_mapping(
                stored_version,
                11,
                &default_mapping.id,
                DEFAULT_MAPPING_IDS_ADDED_IN_V11,
            ) || should_backfill_versioned_mapping(
                stored_version,
                12,
                &default_mapping.id,
                DEFAULT_MAPPING_IDS_ADDED_IN_V12,
            ) || should_backfill_versioned_mapping(
                stored_version,
                13,
                &default_mapping.id,
                DEFAULT_MAPPING_IDS_ADDED_IN_V13,
            ) || should_backfill_versioned_mapping(
                stored_version,
                14,
                &default_mapping.id,
                DEFAULT_MAPPING_IDS_ADDED_IN_V14,
            ) || should_backfill_versioned_mapping(
                stored_version,
                16,
                &default_mapping.id,
                DEFAULT_MAPPING_IDS_ADDED_IN_V16,
            ))
        {
            let mapping_data = adapter::mapping_to_db_value(&default_mapping);
            if let Err(e) = db.with_conn(|conn| {
                db_put(
                    conn,
                    DbTable::WslFileMapping,
                    &default_mapping.id,
                    &mapping_data,
                )
            }) {
                log::warn!(
                    "Failed to backfill WSL mapping '{}': {}",
                    default_mapping.id,
                    e
                );
                continue;
            }
            log::info!("Backfilled default WSL mapping: {}", default_mapping.id);
            file_mappings.push(default_mapping);
        }
    }

    // v10: migrate the persisted Oh My OpenAgent mapping from the legacy default
    // (~/.config/opencode/oh-my-openagent.jsonc) to the unified ~/.omo/omo.jsonc,
    // so the WSL/SSH settings page shows the path that dynamic sync actually uses.
    if stored_version < 10 {
        const OLD_OMO_MAPPING_PATH: &str = "~/.config/opencode/oh-my-openagent.jsonc";
        const NEW_OMO_MAPPING_PATH: &str = "~/.omo/omo.jsonc";
        for mapping in file_mappings.iter_mut() {
            if mapping.id == "opencode-oh-my"
                && (mapping.windows_path == OLD_OMO_MAPPING_PATH
                    || mapping.wsl_path == OLD_OMO_MAPPING_PATH)
            {
                let mapping_id = mapping.id.clone();
                mapping.windows_path = NEW_OMO_MAPPING_PATH.to_string();
                mapping.wsl_path = NEW_OMO_MAPPING_PATH.to_string();
                let data = adapter::mapping_to_db_value(mapping);
                if let Err(e) =
                    db.with_conn(|conn| db_put(conn, DbTable::WslFileMapping, &mapping_id, &data))
                {
                    log::warn!("Failed to migrate WSL mapping '{}': {}", mapping_id, e);
                } else {
                    log::info!(
                        "Migrated WSL mapping '{}' to {}",
                        mapping_id,
                        NEW_OMO_MAPPING_PATH
                    );
                }
            }
        }
    }

    // v15: drop the Claude Desktop default mapping. Claude Desktop is a Windows
    // GUI app that cannot run inside WSL, so syncing its config there was
    // meaningless (the mapping was disabled-by-default anyway). Remove any
    // persisted row so the generic sync_mappings loop stops touching it, even
    // if a user had manually enabled it.
    if stored_version < 15 {
        if let Err(e) =
            db.with_conn(|conn| db_delete(conn, DbTable::WslFileMapping, "claude-desktop-config"))
        {
            log::warn!("Failed to drop legacy claude-desktop-config mapping: {}", e);
        }
    }

    // Mark migration as done
    let version_data = serde_json::json!({ "version": CURRENT_DEFAULTS_VERSION });
    let _ = db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::WslSyncConfig,
            "defaults_version",
            &version_data,
        )
    });

    file_mappings
}

fn should_backfill_versioned_mapping(
    stored_version: u64,
    introduced_version: u64,
    mapping_id: &str,
    introduced_mapping_ids: &[&str],
) -> bool {
    stored_version < introduced_version && introduced_mapping_ids.contains(&mapping_id)
}

fn should_backfill_default_mapping(
    stored_version: u64,
    existing_mapping_count: usize,
    mapping_id: &str,
    previous_defaults_version: u64,
    new_mapping_ids: &[&str],
) -> bool {
    if stored_version == 0 && existing_mapping_count == 0 {
        return true;
    }
    if stored_version < previous_defaults_version {
        return true;
    }
    new_mapping_ids.contains(&mapping_id)
}

/// Dynamically resolve config file paths for OpenCode and Oh My OpenAgent.
/// This ensures we sync the actual config file format (.jsonc or .json) being used
pub(super) fn resolve_dynamic_paths(mappings: Vec<FileMapping>) -> Vec<FileMapping> {
    // Keep a minimal fallback for paths that do not require DB.
    mappings
        .into_iter()
        .map(|mapping| {
            match mapping.id.as_str() {
                "opencode-prompt" => {
                    // prompt path is resolved by the async wrapper
                }
                _ => {}
            }
            mapping
        })
        .collect()
}

pub(super) async fn resolve_dynamic_paths_with_db(
    db: &SqliteDbState,
    mappings: Vec<FileMapping>,
) -> Vec<FileMapping> {
    let mut resolved = Vec::with_capacity(mappings.len());
    let legacy_omo_config = runtime_location::uses_legacy_omo_config_async(db).await;
    for mut mapping in resolve_dynamic_paths(mappings) {
        match mapping.id.as_str() {
            "opencode-main" => {
                if let Ok(location) =
                    runtime_location::get_opencode_runtime_location_async(db).await
                {
                    mapping.windows_path = location.host_path.to_string_lossy().to_string();
                    mapping.wsl_path = match location.wsl {
                        Some(wsl) => wsl.linux_path,
                        None => runtime_location::get_opencode_wsl_target_path_async(db).await,
                    };
                }
            }
            "opencode-oh-my" => {
                if let Ok(path) = runtime_location::get_omo_config_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path = path
                        .to_str()
                        .and_then(runtime_location::parse_wsl_unc_path)
                        .map(|wsl| wsl.linux_path)
                        .unwrap_or_else(|| {
                            if !legacy_omo_config {
                                "~/.omo/omo.jsonc".to_string()
                            } else {
                                path.file_name()
                                    .map(|name| {
                                        format!("~/.config/opencode/{}", name.to_string_lossy())
                                    })
                                    .unwrap_or_else(|| {
                                        "~/.config/opencode/oh-my-openagent.jsonc".to_string()
                                    })
                            }
                        });
                }
            }
            "opencode-oh-my-slim" => {
                if let Ok(path) = runtime_location::get_omos_config_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path = path
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
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path = if let Some(wsl) =
                        path.to_str().and_then(runtime_location::parse_wsl_unc_path)
                    {
                        wsl.linux_path
                    } else {
                        runtime_location::get_opencode_prompt_wsl_target_path_async(db).await
                    };
                }
            }
            "claude-settings" => {
                if let Ok(path) = runtime_location::get_claude_settings_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_claude_wsl_target_path_async(db, "settings.json")
                            .await;
                }
            }
            "claude-config" => {
                if let Ok(path) = runtime_location::get_claude_plugin_config_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_claude_wsl_target_path_async(db, "config.json").await;
                }
            }
            "claude-prompt" => {
                if let Ok(path) = runtime_location::get_claude_prompt_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_claude_wsl_target_path_async(db, "CLAUDE.md").await;
                }
            }
            "claude-plugins" => {
                if let Ok(path) = runtime_location::get_claude_plugins_dir_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_claude_wsl_target_path_async(db, "plugins").await;
                }
            }
            "codex-auth" => {
                if let Ok(path) = runtime_location::get_codex_auth_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_codex_wsl_target_path_async(db, "auth.json").await;
                }
            }
            "codex-config" => {
                if let Ok(path) = runtime_location::get_codex_config_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_codex_wsl_target_path_async(db, "config.toml").await;
                }
            }
            "codex-prompt" => {
                if let Ok(path) = runtime_location::get_codex_prompt_path_async(db).await {
                    let file_name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(runtime_location::CODEX_DEFAULT_PROMPT_FILE_NAME);
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_codex_wsl_target_path_async(db, file_name).await;
                }
            }
            "codex-plugins" => {
                if let Ok(location) = runtime_location::get_codex_runtime_location_async(db).await {
                    mapping.windows_path = location
                        .host_path
                        .join("plugins")
                        .to_string_lossy()
                        .to_string();
                    mapping.wsl_path = location
                        .wsl
                        .map(|wsl| format!("{}/plugins", wsl.linux_path.trim_end_matches('/')))
                        .unwrap_or_else(|| "~/.codex/plugins".to_string());
                }
            }
            "grok-auth" => {
                if let Ok(path) = runtime_location::get_grok_auth_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_grok_wsl_target_path_async(db, "auth.json").await;
                }
            }
            "grok-config" => {
                if let Ok(path) = runtime_location::get_grok_config_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_grok_wsl_target_path_async(db, "config.toml").await;
                }
            }
            "grok-prompt" => {
                if let Ok(path) = runtime_location::get_grok_prompt_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_grok_wsl_target_path_async(db, "AGENTS.md").await;
                }
            }
            "grok-plugins" => {
                if let Ok(location) = runtime_location::get_grok_runtime_location_async(db).await {
                    mapping.windows_path = location
                        .host_path
                        .join("plugins")
                        .to_string_lossy()
                        .to_string();
                    mapping.wsl_path = location
                        .wsl
                        .map(|wsl| format!("{}/plugins", wsl.linux_path.trim_end_matches('/')))
                        .unwrap_or_else(|| "~/.grok/plugins".to_string());
                }
            }
            "kimi-config" => {
                if let Ok(path) = runtime_location::get_kimi_config_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_kimi_wsl_target_path_async(db, "config.toml").await;
                }
            }
            "kimi-prompt" => {
                if let Ok(path) = runtime_location::get_kimi_prompt_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_kimi_wsl_target_path_async(db, "AGENTS.md").await;
                }
            }
            "kimi-credentials" | "kimi-plugins" => {
                if let Ok(location) = runtime_location::get_kimi_runtime_location_async(db).await {
                    let dir_name = if mapping.id == "kimi-credentials" {
                        crate::coding::kimi::constants::KIMI_CREDENTIALS_DIR
                    } else {
                        crate::coding::kimi::constants::KIMI_PLUGINS_DIR
                    };
                    mapping.windows_path = location
                        .host_path
                        .join(dir_name)
                        .to_string_lossy()
                        .to_string();
                    mapping.wsl_path = location
                        .wsl
                        .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), dir_name))
                        .unwrap_or_else(|| format!("~/.kimi-code/{dir_name}"));
                }
            }
            "openclaw-config" => {
                if let Ok(location) =
                    runtime_location::get_openclaw_runtime_location_async(db).await
                {
                    mapping.windows_path = location.host_path.to_string_lossy().to_string();
                    mapping.wsl_path = match location.wsl {
                        Some(wsl) => wsl.linux_path,
                        None => runtime_location::get_openclaw_wsl_target_path_async(db).await,
                    };
                }
            }
            "geminicli-env" => {
                if let Ok(path) = runtime_location::get_gemini_cli_env_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_gemini_cli_wsl_target_path_async(db, ".env").await;
                }
            }
            "geminicli-settings" => {
                if let Ok(path) = runtime_location::get_gemini_cli_settings_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_gemini_cli_wsl_target_path_async(db, "settings.json")
                            .await;
                }
            }
            "geminicli-prompt" => {
                if let Ok(path) = runtime_location::get_gemini_cli_prompt_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path =
                        runtime_location::get_gemini_cli_prompt_wsl_target_path_async(db).await;
                }
            }
            "geminicli-oauth" => {
                if let Ok(path) = runtime_location::get_gemini_cli_oauth_creds_path_async(db).await
                {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path = runtime_location::get_gemini_cli_wsl_target_path_async(
                        db,
                        "oauth_creds.json",
                    )
                    .await;
                }
            }
            "pi-settings" => {
                if let Ok(path) = crate::coding::pi::get_pi_settings_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path = pi_wsl_target_path(db, PI_SETTINGS_FILE).await;
                }
            }
            "pi-auth" => {
                if let Ok(path) = crate::coding::pi::get_pi_auth_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path = pi_wsl_target_path(db, PI_AUTH_FILE).await;
                }
            }
            "pi-models" => {
                if let Ok(path) = crate::coding::pi::get_pi_models_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path = pi_wsl_target_path(db, PI_MODELS_FILE).await;
                }
            }
            "pi-mcp" => {
                if let Ok(path) = crate::coding::pi::get_pi_mcp_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path = pi_wsl_target_path(db, PI_MCP_FILE).await;
                }
            }
            "pi-prompt" => {
                if let Ok(path) = crate::coding::pi::get_pi_prompt_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path = pi_wsl_target_path(db, PI_PROMPT_FILE).await;
                }
            }
            "pi-system" => {
                if let Ok(location) = runtime_location::get_pi_runtime_location_async(db).await {
                    mapping.windows_path = location
                        .host_path
                        .join("SYSTEM.md")
                        .to_string_lossy()
                        .to_string();
                    mapping.wsl_path = pi_wsl_target_path_from_location(&location, "SYSTEM.md");
                }
            }
            "pi-append-system" => {
                if let Ok(location) = runtime_location::get_pi_runtime_location_async(db).await {
                    mapping.windows_path = location
                        .host_path
                        .join("APPEND_SYSTEM.md")
                        .to_string_lossy()
                        .to_string();
                    mapping.wsl_path =
                        pi_wsl_target_path_from_location(&location, "APPEND_SYSTEM.md");
                }
            }
            "pi-trust" => {
                if let Ok(location) = runtime_location::get_pi_runtime_location_async(db).await {
                    mapping.windows_path = location
                        .host_path
                        .join("trust.json")
                        .to_string_lossy()
                        .to_string();
                    mapping.wsl_path = pi_wsl_target_path_from_location(&location, "trust.json");
                }
            }
            "omp-config" => {
                if let Ok(path) = crate::coding::oh_my_pi::get_omp_config_path_async(db).await {
                    let file_name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| OMP_CONFIG_FILE.to_string());
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path = omp_wsl_target_path(db, &file_name).await;
                }
            }
            "omp-models" => {
                if let Ok(path) = crate::coding::oh_my_pi::get_omp_models_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path = omp_wsl_target_path(db, OMP_MODELS_FILE).await;
                }
            }
            "omp-mcp" => {
                if let Ok(path) = crate::coding::oh_my_pi::get_omp_mcp_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path = omp_wsl_target_path(db, OMP_MCP_FILE).await;
                }
            }
            "omp-agents" => {
                if let Ok(path) = crate::coding::oh_my_pi::get_omp_prompt_path_async(db).await {
                    mapping.windows_path = path.to_string_lossy().to_string();
                    mapping.wsl_path = omp_wsl_target_path(db, OMP_PROMPT_FILE).await;
                }
            }
            "omp-rules" => {
                if let Ok(location) =
                    runtime_location::get_oh_my_pi_runtime_location_async(db).await
                {
                    mapping.windows_path = location
                        .host_path
                        .join("RULES.md")
                        .to_string_lossy()
                        .to_string();
                    mapping.wsl_path = omp_wsl_target_path_from_location(&location, "RULES.md");
                }
            }
            "hermes-config" | "hermes-prompt" => {
                if let Ok((config_dir, _)) =
                    crate::coding::hermes::get_hermes_config_dir_from_db_async(db).await
                {
                    let file_name = if mapping.id == "hermes-config" {
                        crate::coding::hermes::constants::HERMES_CONFIG_FILE
                    } else {
                        crate::coding::hermes::constants::HERMES_PROMPT_FILE
                    };
                    mapping.windows_path = config_dir.join(file_name).to_string_lossy().to_string();
                    mapping.wsl_path = format!("~/.hermes/{file_name}");
                }
            }
            "dsh-config" | "dsh-credentials" | "dsh-prompt" | "dsh-mcp" => {
                if let Ok((config_dir, _)) =
                    crate::coding::dsh::get_dsh_config_dir_from_db_async(db).await
                {
                    let file_name = match mapping.id.as_str() {
                        "dsh-credentials" => DSH_CREDENTIALS_FILE,
                        "dsh-prompt" => DSH_PROMPT_FILE,
                        "dsh-mcp" => DSH_MCP_FILE,
                        _ => DSH_SETTINGS_FILE,
                    };
                    mapping.windows_path = config_dir.join(file_name).to_string_lossy().to_string();
                    mapping.wsl_path = format!("~/.dsh/{file_name}");
                }
            }
            _ => {}
        }
        resolved.push(mapping);
    }
    resolved
}

fn omp_wsl_target_path_from_location(
    location: &runtime_location::RuntimeLocationInfo,
    file_name: &str,
) -> String {
    location
        .wsl
        .as_ref()
        .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
        .unwrap_or_else(|| format!("~/.omp/agent/{file_name}"))
}

async fn omp_wsl_target_path(db: &SqliteDbState, file_name: &str) -> String {
    runtime_location::get_oh_my_pi_runtime_location_async(db)
        .await
        .map(|location| omp_wsl_target_path_from_location(&location, file_name))
        .unwrap_or_else(|_| format!("~/.omp/agent/{file_name}"))
}

fn pi_wsl_target_path_from_location(
    location: &runtime_location::RuntimeLocationInfo,
    file_name: &str,
) -> String {
    location
        .wsl
        .as_ref()
        .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
        .unwrap_or_else(|| format!("~/.pi/agent/{file_name}"))
}

async fn pi_wsl_target_path(db: &SqliteDbState, file_name: &str) -> String {
    runtime_location::get_pi_runtime_location_async(db)
        .await
        .map(|location| pi_wsl_target_path_from_location(&location, file_name))
        .unwrap_or_else(|_| format!("~/.pi/agent/{file_name}"))
}

/// Update sync status in database
pub(super) async fn update_sync_status(
    state: &SqliteDbState,
    result: &SyncResult,
) -> Result<(), String> {
    let (status, error) = if result.success {
        ("success".to_string(), None)
    } else {
        let error_msg = result.errors.join("; ");
        ("error".to_string(), Some(error_msg))
    };

    let now = Local::now().to_rfc3339();

    patch_wsl_config_record(
        state,
        &[
            ("last_sync_time", serde_json::json!(now)),
            ("last_sync_status", serde_json::json!(status)),
            ("last_sync_error", serde_json::json!(error)),
        ],
    )
}

/// Replace the persisted Skills sync warnings. Only the Skills sync chain
/// calls this so that unrelated silent chains (e.g. file mappings) cannot
/// clear or mix warnings from a different sub-run.
pub(super) async fn update_sync_warnings(
    state: &SqliteDbState,
    warnings: &[String],
) -> Result<(), String> {
    patch_wsl_config_record(
        state,
        &[("last_sync_warnings", serde_json::json!(warnings))],
    )
}

/// Get default file mappings
pub fn default_file_mappings() -> Vec<FileMapping> {
    vec![
        // OpenCode
        FileMapping {
            id: "opencode-main".to_string(),
            name: "OpenCode 主配置".to_string(),
            module: "opencode".to_string(),
            windows_path: "~/.config/opencode/opencode.jsonc".to_string(),
            wsl_path: "~/.config/opencode/opencode.jsonc".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "opencode-oh-my".to_string(),
            name: "Oh My OpenAgent 配置".to_string(),
            module: "opencode".to_string(),
            windows_path: "~/.omo/omo.jsonc".to_string(),
            wsl_path: "~/.omo/omo.jsonc".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "opencode-oh-my-slim".to_string(),
            name: "Oh My OpenCode Slim 配置".to_string(),
            module: "opencode".to_string(),
            windows_path: "~/.config/opencode/oh-my-opencode-slim.json".to_string(),
            wsl_path: "~/.config/opencode/oh-my-opencode-slim.json".to_string(),
            enabled: false, // Disabled by default: this file is optional and not present on all systems
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "opencode-auth".to_string(),
            name: "OpenCode 认证信息".to_string(),
            module: "opencode".to_string(),
            windows_path: "~/.local/share/opencode/auth.json".to_string(),
            wsl_path: "~/.local/share/opencode/auth.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "opencode-plugins".to_string(),
            name: "OpenCode 插件文件".to_string(),
            module: "opencode".to_string(),
            windows_path: "~/.config/opencode/*.mjs".to_string(),
            wsl_path: "~/.config/opencode/".to_string(),
            enabled: true,
            is_pattern: true,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "opencode-prompt".to_string(),
            name: "OpenCode 全局提示词".to_string(),
            module: "opencode".to_string(),
            windows_path: "~/.config/opencode/AGENTS.md".to_string(),
            wsl_path: "~/.config/opencode/AGENTS.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "opencode-agents".to_string(),
            name: "OpenCode Agent 配置（agents）".to_string(),
            module: "opencode".to_string(),
            windows_path: "~/.config/opencode/agents".to_string(),
            wsl_path: "~/.config/opencode/agents".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: true,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        // ClaudeCode
        FileMapping {
            id: "claude-settings".to_string(),
            name: "Claude Code 设置".to_string(),
            module: "claude".to_string(),
            windows_path: "~/.claude/settings.json".to_string(),
            wsl_path: "~/.claude/settings.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "claude-config".to_string(),
            name: "Claude Code 配置".to_string(),
            module: "claude".to_string(),
            windows_path: "~/.claude/config.json".to_string(),
            wsl_path: "~/.claude/config.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "claude-prompt".to_string(),
            name: "Claude Code 全局提示词".to_string(),
            module: "claude".to_string(),
            windows_path: "~/.claude/CLAUDE.md".to_string(),
            wsl_path: "~/.claude/CLAUDE.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "claude-plugins".to_string(),
            name: "Claude Code 插件目录".to_string(),
            module: "claude".to_string(),
            windows_path: "~/.claude/plugins".to_string(),
            wsl_path: "~/.claude/plugins".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: true,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        // Codex
        FileMapping {
            id: "codex-auth".to_string(),
            name: "Codex 认证".to_string(),
            module: "codex".to_string(),
            windows_path: "~/.codex/auth.json".to_string(),
            wsl_path: "~/.codex/auth.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "codex-config".to_string(),
            name: "Codex 配置".to_string(),
            module: "codex".to_string(),
            windows_path: "~/.codex/config.toml".to_string(),
            wsl_path: "~/.codex/config.toml".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "codex-prompt".to_string(),
            name: "Codex 全局提示词".to_string(),
            module: "codex".to_string(),
            windows_path: "~/.codex/AGENTS.md".to_string(),
            wsl_path: "~/.codex/AGENTS.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "codex-plugins".to_string(),
            name: "Codex 插件目录".to_string(),
            module: "codex".to_string(),
            windows_path: "~/.codex/plugins".to_string(),
            wsl_path: "~/.codex/plugins".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: true,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        // Grok
        FileMapping {
            id: "grok-auth".to_string(),
            name: "Grok 认证".to_string(),
            module: "grok".to_string(),
            windows_path: "~/.grok/auth.json".to_string(),
            wsl_path: "~/.grok/auth.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "grok-config".to_string(),
            name: "Grok 配置".to_string(),
            module: "grok".to_string(),
            windows_path: "~/.grok/config.toml".to_string(),
            wsl_path: "~/.grok/config.toml".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "grok-prompt".to_string(),
            name: "Grok 全局提示词".to_string(),
            module: "grok".to_string(),
            windows_path: "~/.grok/AGENTS.md".to_string(),
            wsl_path: "~/.grok/AGENTS.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "grok-plugins".to_string(),
            name: "Grok 插件目录".to_string(),
            module: "grok".to_string(),
            windows_path: "~/.grok/plugins".to_string(),
            wsl_path: "~/.grok/plugins".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: true,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        // OpenClaw
        FileMapping {
            id: "openclaw-config".to_string(),
            name: "OpenClaw 配置".to_string(),
            module: "openclaw".to_string(),
            windows_path: "~/.openclaw/openclaw.json".to_string(),
            wsl_path: "~/.openclaw/openclaw.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        // Gemini CLI
        FileMapping {
            id: "geminicli-env".to_string(),
            name: "Gemini CLI 环境变量".to_string(),
            module: "geminicli".to_string(),
            windows_path: "~/.gemini/.env".to_string(),
            wsl_path: "~/.gemini/.env".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "geminicli-settings".to_string(),
            name: "Gemini CLI 设置".to_string(),
            module: "geminicli".to_string(),
            windows_path: "~/.gemini/settings.json".to_string(),
            wsl_path: "~/.gemini/settings.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "geminicli-prompt".to_string(),
            name: "Gemini CLI 全局提示词".to_string(),
            module: "geminicli".to_string(),
            windows_path: "~/.gemini/GEMINI.md".to_string(),
            wsl_path: "~/.gemini/GEMINI.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "geminicli-oauth".to_string(),
            name: "Gemini CLI OAuth 凭证".to_string(),
            module: "geminicli".to_string(),
            windows_path: "~/.gemini/oauth_creds.json".to_string(),
            wsl_path: "~/.gemini/oauth_creds.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        // Pi
        FileMapping {
            id: "pi-settings".to_string(),
            name: "Pi 设置".to_string(),
            module: "pi".to_string(),
            windows_path: "~/.pi/agent/settings.json".to_string(),
            wsl_path: "~/.pi/agent/settings.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "pi-auth".to_string(),
            name: "Pi 认证信息".to_string(),
            module: "pi".to_string(),
            windows_path: "~/.pi/agent/auth.json".to_string(),
            wsl_path: "~/.pi/agent/auth.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "pi-models".to_string(),
            name: "Pi 模型供应商".to_string(),
            module: "pi".to_string(),
            windows_path: "~/.pi/agent/models.json".to_string(),
            wsl_path: "~/.pi/agent/models.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "pi-mcp".to_string(),
            name: "Pi MCP 配置".to_string(),
            module: "pi".to_string(),
            windows_path: "~/.pi/agent/mcp.json".to_string(),
            wsl_path: "~/.pi/agent/mcp.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "pi-prompt".to_string(),
            name: "Pi 全局提示词".to_string(),
            module: "pi".to_string(),
            windows_path: "~/.pi/agent/AGENTS.md".to_string(),
            wsl_path: "~/.pi/agent/AGENTS.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "pi-system".to_string(),
            name: "Pi System Prompt".to_string(),
            module: "pi".to_string(),
            windows_path: "~/.pi/agent/SYSTEM.md".to_string(),
            wsl_path: "~/.pi/agent/SYSTEM.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "pi-append-system".to_string(),
            name: "Pi 追加 System Prompt".to_string(),
            module: "pi".to_string(),
            windows_path: "~/.pi/agent/APPEND_SYSTEM.md".to_string(),
            wsl_path: "~/.pi/agent/APPEND_SYSTEM.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "pi-trust".to_string(),
            name: "Pi 信任记录".to_string(),
            module: "pi".to_string(),
            windows_path: "~/.pi/agent/trust.json".to_string(),
            wsl_path: "~/.pi/agent/trust.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "omp-config".to_string(),
            name: "Oh My Pi 配置".to_string(),
            module: "oh_my_pi".to_string(),
            windows_path: "~/.omp/agent/config.yml".to_string(),
            wsl_path: "~/.omp/agent/config.yml".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "omp-models".to_string(),
            name: "Oh My Pi 模型供应商".to_string(),
            module: "oh_my_pi".to_string(),
            windows_path: "~/.omp/agent/models.yml".to_string(),
            wsl_path: "~/.omp/agent/models.yml".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "omp-mcp".to_string(),
            name: "Oh My Pi MCP 配置".to_string(),
            module: "oh_my_pi".to_string(),
            windows_path: "~/.omp/agent/mcp.json".to_string(),
            wsl_path: "~/.omp/agent/mcp.json".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "omp-agents".to_string(),
            name: "Oh My Pi 全局提示词".to_string(),
            module: "oh_my_pi".to_string(),
            windows_path: "~/.omp/agent/AGENTS.md".to_string(),
            wsl_path: "~/.omp/agent/AGENTS.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "omp-rules".to_string(),
            name: "Oh My Pi 规则".to_string(),
            module: "oh_my_pi".to_string(),
            windows_path: "~/.omp/agent/RULES.md".to_string(),
            wsl_path: "~/.omp/agent/RULES.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        // Hermes - runtime config.yaml + authored global prompt.
        FileMapping {
            id: "hermes-config".to_string(),
            name: "Hermes 配置".to_string(),
            module: "hermes".to_string(),
            windows_path: "~/.hermes/config.yaml".to_string(),
            wsl_path: "~/.hermes/config.yaml".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "hermes-prompt".to_string(),
            name: "Hermes 全局提示词".to_string(),
            module: "hermes".to_string(),
            windows_path: "~/.hermes/SOUL.md".to_string(),
            wsl_path: "~/.hermes/SOUL.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        // dsh - runtime settings.yaml + separate credentials file + authored prompt.
        FileMapping {
            id: "dsh-config".to_string(),
            name: "DeepSeek Harness 配置".to_string(),
            module: "dsh".to_string(),
            windows_path: "~/.dsh/settings.yaml".to_string(),
            wsl_path: "~/.dsh/settings.yaml".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "dsh-credentials".to_string(),
            name: "DeepSeek Harness 凭据".to_string(),
            module: "dsh".to_string(),
            windows_path: "~/.dsh/.credentials.yaml".to_string(),
            wsl_path: "~/.dsh/.credentials.yaml".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "dsh-prompt".to_string(),
            name: "DeepSeek Harness 全局提示词".to_string(),
            module: "dsh".to_string(),
            windows_path: "~/.dsh/AGENTS.md".to_string(),
            wsl_path: "~/.dsh/AGENTS.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "dsh-mcp".to_string(),
            name: "DeepSeek Harness MCP 配置".to_string(),
            module: "dsh".to_string(),
            windows_path: "~/.dsh/cordis.patch.yml".to_string(),
            wsl_path: "~/.dsh/cordis.patch.yml".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        // Kimi Code CLI - runtime config.toml + authored prompt + credentials + plugins.
        FileMapping {
            id: "kimi-config".to_string(),
            name: "Kimi Code CLI 配置".to_string(),
            module: "kimi".to_string(),
            windows_path: "~/.kimi-code/config.toml".to_string(),
            wsl_path: "~/.kimi-code/config.toml".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "kimi-prompt".to_string(),
            name: "Kimi Code CLI 全局提示词".to_string(),
            module: "kimi".to_string(),
            windows_path: "~/.kimi-code/AGENTS.md".to_string(),
            wsl_path: "~/.kimi-code/AGENTS.md".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: false,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "kimi-credentials".to_string(),
            name: "Kimi Code CLI 凭据".to_string(),
            module: "kimi".to_string(),
            windows_path: "~/.kimi-code/credentials".to_string(),
            wsl_path: "~/.kimi-code/credentials".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: true,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
        FileMapping {
            id: "kimi-plugins".to_string(),
            name: "Kimi Code CLI 插件目录".to_string(),
            module: "kimi".to_string(),
            windows_path: "~/.kimi-code/plugins".to_string(),
            wsl_path: "~/.kimi-code/plugins".to_string(),
            enabled: true,
            is_pattern: false,
            is_directory: true,
            directory_excludes: vec![],
            cleanup_paths: vec![],
        },
    ]
}

// ============================================================================
// Onboarding Sync
// ============================================================================

/// Sync Claude Code onboarding status (hasCompletedOnboarding) from Windows to WSL.
///
/// Reads the Windows-side ~/.claude.json status and mirrors it to WSL's ~/.claude.json,
/// preserving all other fields in the WSL file.
async fn sync_onboarding_to_wsl(state: &SqliteDbState, distro: &str) -> Result<(), String> {
    // 1. Read Windows-side onboarding status
    let db = state.db();
    let windows_config_path = runtime_location::get_claude_mcp_config_path_async(&db).await?;
    let windows_status = read_claude_onboarding_status_from_path(&windows_config_path)?;

    // 2. Read existing WSL ~/.claude.json
    let wsl_config_path = runtime_location::get_claude_wsl_claude_json_path_async(&db).await;
    let existing_content = sync::read_wsl_file(distro, wsl_config_path.as_str())?;

    // 3. Parse JSON or create empty object
    let mut config: serde_json::Value = if existing_content.trim().is_empty() {
        serde_json::json!({})
    } else {
        json5::from_str(&existing_content)
            .map_err(|e| format!("Failed to parse WSL claude.json: {}", e))?
    };

    let obj = config
        .as_object_mut()
        .ok_or("WSL claude.json is not a JSON object")?;

    // 4. Check current WSL-side value
    let wsl_status = obj
        .get("hasCompletedOnboarding")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // 5. Skip if already in sync
    if wsl_status == windows_status {
        return Ok(());
    }

    // 6. Update the field
    if windows_status {
        obj.insert(
            "hasCompletedOnboarding".to_string(),
            serde_json::Value::Bool(true),
        );
    } else {
        obj.remove("hasCompletedOnboarding");
    }

    // 7. Write back to WSL
    let content = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    sync::write_wsl_file(distro, wsl_config_path.as_str(), &content)?;

    log::info!(
        "Synced onboarding status to WSL: hasCompletedOnboarding={}",
        windows_status
    );

    Ok(())
}

/// Ensure OpenClaw config file exists in WSL.
///
/// Checks if `~/.openclaw/openclaw.json` exists in the target WSL distro.
/// If the file is missing, creates it with an empty JSON object `{}`.
async fn ensure_openclaw_config_in_wsl(state: &SqliteDbState, distro: &str) -> Result<(), String> {
    let db = state.db();
    let config_path = runtime_location::get_openclaw_wsl_target_path_async(&db).await;
    let content = sync::read_wsl_file(distro, config_path.as_str());

    match content {
        Ok(c) if !c.trim().is_empty() => {
            // File exists and has content, nothing to do
            Ok(())
        }
        _ => {
            // File missing or empty – write_wsl_file already does mkdir -p
            sync::write_wsl_file(distro, config_path.as_str(), "{}")?;
            log::info!("Created default OpenClaw config in WSL: {}", config_path);
            Ok(())
        }
    }
}

fn merge_skip_modules(
    skip_modules: Option<&[String]>,
    direct_modules: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut merged = skip_modules.map(|items| items.to_vec()).unwrap_or_default();
    for module in direct_modules {
        if !merged.iter().any(|item| item == module) {
            merged.push(module.clone());
        }
    }
    merged
}

fn read_claude_onboarding_status_from_path(path: &std::path::Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }

    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read config file: {}", e))?;

    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse config file: {}", e))?;

    Ok(value
        .get("hasCompletedOnboarding")
        .and_then(|v| v.as_bool())
        .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::{
        codex_config_uses_ai_toolbox_model_catalog, default_file_mappings,
        should_backfill_default_mapping, should_backfill_versioned_mapping,
    };

    #[tokio::test]
    async fn saving_sync_preferences_preserves_latest_skills_warnings() {
        let state = crate::db::SqliteDbState::in_memory_for_test().unwrap();
        super::save_wsl_config_record(&state, &super::WSLSyncConfig::default()).unwrap();
        let mut stale_config = super::load_wsl_config(&state).unwrap();
        let warnings = vec!["Keep external skill directory".to_string()];
        super::update_sync_warnings(&state, &warnings)
            .await
            .unwrap();
        let file_result = super::SyncResult {
            success: false,
            synced_files: vec![],
            skipped_files: vec![],
            errors: vec!["File sync failed".to_string()],
            warnings: vec![],
        };
        super::update_sync_status(&state, &file_result)
            .await
            .unwrap();
        let synced_config = super::load_wsl_config(&state).unwrap();
        stale_config.distro = "Debian".to_string();
        super::save_wsl_config_record(&state, &stale_config).unwrap();

        let reloaded = super::load_wsl_config(&state).unwrap();
        assert_eq!(reloaded.distro, "Debian");
        assert_eq!(reloaded.last_sync_warnings, warnings);
        assert_eq!(reloaded.last_sync_time, synced_config.last_sync_time);
        assert_eq!(reloaded.last_sync_status, "error");
        assert_eq!(
            reloaded.last_sync_error.as_deref(),
            Some("File sync failed")
        );

        // Only a later Skills run can replace or clear its diagnostic snapshot.
        super::update_sync_warnings(&state, &[]).await.unwrap();
        let cleared = super::load_wsl_config(&state).unwrap();
        assert!(cleared.last_sync_warnings.is_empty());
        assert_eq!(cleared.last_sync_status, "error");
        assert_eq!(cleared.distro, "Debian");
    }

    #[test]
    fn concurrent_sync_status_and_warnings_preserve_both_snapshots() {
        let state = crate::db::SqliteDbState::in_memory_for_test().unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            for write_warnings in [true, false] {
                let state = &state;
                let barrier = &barrier;
                scope.spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .unwrap();
                    for index in 0..32 {
                        barrier.wait();
                        if write_warnings {
                            runtime
                                .block_on(super::update_sync_warnings(
                                    state,
                                    &[format!("warning-{index}")],
                                ))
                                .unwrap();
                        } else {
                            runtime
                                .block_on(super::update_sync_status(
                                    state,
                                    &super::SyncResult {
                                        success: false,
                                        synced_files: vec![],
                                        skipped_files: vec![],
                                        errors: vec![format!("error-{index}")],
                                        warnings: vec![],
                                    },
                                ))
                                .unwrap();
                        }
                        barrier.wait();
                    }
                });
            }
        });
        let config = super::load_wsl_config(&state).unwrap();
        assert_eq!(config.last_sync_warnings, ["warning-31"]);
        assert_eq!(config.last_sync_error.as_deref(), Some("error-31"));
    }

    #[test]
    fn pi_mcp_default_mapping_is_regular_file() {
        let mapping = default_file_mappings()
            .into_iter()
            .find(|mapping| mapping.id == "pi-mcp")
            .expect("pi-mcp default mapping exists");

        assert_eq!(mapping.module, "pi");
        assert_eq!(mapping.windows_path, "~/.pi/agent/mcp.json");
        assert_eq!(mapping.wsl_path, "~/.pi/agent/mcp.json");
        assert!(!mapping.is_directory);
    }

    #[test]
    fn defaults_backfill_v7_only_adds_pi_mcp_for_existing_v6_users() {
        assert!(should_backfill_default_mapping(
            6,
            4,
            "pi-mcp",
            6,
            &["pi-mcp"],
        ));
        assert!(!should_backfill_default_mapping(
            6,
            4,
            "codex-config",
            6,
            &["pi-mcp"],
        ));
    }

    #[test]
    fn defaults_backfill_v8_only_adds_opencode_agents_directory_for_existing_v7_users() {
        assert!(should_backfill_default_mapping(
            7,
            4,
            "opencode-agents",
            7,
            &["opencode-agents"],
        ));
        assert!(!should_backfill_default_mapping(
            7,
            4,
            "opencode-prompt",
            7,
            &["opencode-agents"],
        ));
    }

    #[test]
    fn defaults_backfill_v9_only_adds_grok_mappings_for_existing_v8_users() {
        for mapping_id in ["grok-auth", "grok-config", "grok-prompt", "grok-plugins"] {
            assert!(should_backfill_versioned_mapping(
                8,
                9,
                mapping_id,
                &["grok-auth", "grok-config", "grok-prompt", "grok-plugins"],
            ));
        }
        assert!(!should_backfill_versioned_mapping(
            8,
            9,
            "codex-config",
            &["grok-auth", "grok-config", "grok-prompt", "grok-plugins"],
        ));
        assert!(!should_backfill_versioned_mapping(
            9,
            9,
            "grok-config",
            &["grok-auth", "grok-config", "grok-prompt", "grok-plugins"],
        ));
    }

    #[test]
    fn defaults_backfill_v16_only_adds_kimi_mappings_for_existing_v15_users() {
        let kimi_ids = [
            "kimi-config",
            "kimi-prompt",
            "kimi-credentials",
            "kimi-plugins",
        ];
        for mapping_id in kimi_ids {
            assert!(should_backfill_versioned_mapping(
                15, 16, mapping_id, &kimi_ids,
            ));
        }
        assert!(!should_backfill_versioned_mapping(
            15,
            16,
            "grok-config",
            &kimi_ids,
        ));
        assert!(!should_backfill_versioned_mapping(
            16,
            16,
            "kimi-config",
            &kimi_ids,
        ));
    }

    #[test]
    fn opencode_agents_default_mapping_is_the_only_agent_directory() {
        let mappings = default_file_mappings();
        // OpenCode's canonical Markdown Agent directory is the plural `agents/`;
        // the singular `agent/` is only a legacy alias and is no longer shipped.
        assert!(mappings.iter().all(|m| m.id != "opencode-agent"));
        let mapping = mappings
            .iter()
            .find(|mapping| mapping.id == "opencode-agents")
            .expect("OpenCode agents mapping exists");
        assert!(mapping.is_directory);
        assert_eq!(mapping.windows_path, "~/.config/opencode/agents");
        assert_eq!(mapping.wsl_path, "~/.config/opencode/agents");
    }

    #[test]
    fn defaults_backfill_keeps_new_install_full_defaults() {
        assert!(should_backfill_default_mapping(
            0,
            0,
            "codex-config",
            6,
            &["pi-mcp"],
        ));
    }

    #[test]
    fn detects_ai_toolbox_codex_model_catalog_pointer() {
        assert!(codex_config_uses_ai_toolbox_model_catalog(
            r#"model_catalog_json = "ai-toolbox-codex-model-catalog.json""#
        ));
    }

    #[test]
    fn ignores_external_codex_model_catalog_pointer() {
        assert!(!codex_config_uses_ai_toolbox_model_catalog(
            r#"model_catalog_json = "external-catalog.json""#
        ));
        assert!(!codex_config_uses_ai_toolbox_model_catalog(
            r#"model_catalog_json = "subdir/ai-toolbox-codex-model-catalog.json""#
        ));
        assert!(!codex_config_uses_ai_toolbox_model_catalog(
            "model_catalog_json = ["
        ));
    }
}
