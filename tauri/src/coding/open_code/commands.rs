use indexmap::IndexMap;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tauri::Emitter;

use super::adapter;
use super::types::*;
use crate::coding::all_api_hub;
use crate::coding::db_id::db_new_id;
use crate::coding::prompt_file::{read_prompt_content_file, write_prompt_content_file};
use crate::coding::runtime_location;
use crate::coding::skills::commands::resync_all_skills_if_tool_path_changed;
use crate::db::helpers::{
    db_count, db_delete, db_get, db_list, db_max_i64, db_patch_fields, db_patch_where_bool, db_put,
};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;

// ============================================================================
// Helper Functions
// ============================================================================

const OMO_CANONICAL_PLUGIN: &str = "oh-my-openagent";
const OMO_LEGACY_PLUGIN: &str = "oh-my-opencode";
const OMO_SLIM_PLUGIN: &str = "oh-my-opencode-slim";

pub(crate) fn opencode_plugin_package_name(plugin_name: &str) -> &str {
    let trimmed_plugin_name = plugin_name.trim();
    if trimmed_plugin_name.is_empty() {
        return trimmed_plugin_name;
    }

    if !trimmed_plugin_name.starts_with('@') {
        return trimmed_plugin_name
            .rsplit_once('@')
            .map(|(package_name, _)| package_name)
            .filter(|package_name| !package_name.is_empty())
            .unwrap_or(trimmed_plugin_name);
    }

    let Some(scope_separator_index) = trimmed_plugin_name.find('/') else {
        return trimmed_plugin_name;
    };
    let package_and_suffix = &trimmed_plugin_name[scope_separator_index + 1..];
    let Some(version_separator_index) = package_and_suffix.rfind('@') else {
        return trimmed_plugin_name;
    };

    &trimmed_plugin_name[..scope_separator_index + 1 + version_separator_index]
}

pub(crate) fn normalize_opencode_plugin_name(plugin_name: &str) -> String {
    let trimmed_plugin_name = plugin_name.trim();
    if trimmed_plugin_name == OMO_LEGACY_PLUGIN {
        return "oh-my-openagent".to_string();
    }

    if let Some(version_suffix) = trimmed_plugin_name.strip_prefix("oh-my-opencode@") {
        return format!("oh-my-openagent@{}", version_suffix);
    }

    trimmed_plugin_name.to_string()
}

fn has_opencode_plugin_version_suffix(plugin_name: &str) -> bool {
    opencode_plugin_package_name(plugin_name) != plugin_name.trim()
}

fn normalize_opencode_plugin_entry(plugin_entry: &OpenCodePluginEntry) -> OpenCodePluginEntry {
    match plugin_entry {
        OpenCodePluginEntry::Name(plugin_name) => {
            OpenCodePluginEntry::Name(normalize_opencode_plugin_name(plugin_name))
        }
        OpenCodePluginEntry::NameWithOptions((plugin_name, plugin_options)) => {
            OpenCodePluginEntry::NameWithOptions((
                normalize_opencode_plugin_name(plugin_name),
                plugin_options.clone(),
            ))
        }
    }
}

fn plugin_entry_options(
    plugin_entry: &OpenCodePluginEntry,
) -> Option<&serde_json::Map<String, serde_json::Value>> {
    match plugin_entry {
        OpenCodePluginEntry::Name(_) => None,
        OpenCodePluginEntry::NameWithOptions((_, plugin_options)) => Some(plugin_options),
    }
}

fn build_opencode_plugin_entry(
    plugin_name: String,
    plugin_options: Option<serde_json::Map<String, serde_json::Value>>,
) -> OpenCodePluginEntry {
    match plugin_options {
        Some(plugin_options) => OpenCodePluginEntry::NameWithOptions((plugin_name, plugin_options)),
        None => OpenCodePluginEntry::Name(plugin_name),
    }
}

fn merged_opencode_plugin_entry(
    existing_entry: &OpenCodePluginEntry,
    candidate_entry: OpenCodePluginEntry,
) -> OpenCodePluginEntry {
    let existing_name = existing_entry.name();
    let candidate_name = candidate_entry.name();

    let merged_name = if existing_name != candidate_name
        && canonical_omo_plugin_package_name(existing_name) == Some(OMO_CANONICAL_PLUGIN)
        && canonical_omo_plugin_package_name(candidate_name) == Some(OMO_CANONICAL_PLUGIN)
    {
        match (
            has_opencode_plugin_version_suffix(existing_name),
            has_opencode_plugin_version_suffix(candidate_name),
        ) {
            (true, false) => existing_name.to_string(),
            (false, true) => candidate_name.to_string(),
            _ => candidate_name.to_string(),
        }
    } else {
        existing_name.to_string()
    };

    let merged_options = plugin_entry_options(existing_entry)
        .cloned()
        .or_else(|| plugin_entry_options(&candidate_entry).cloned());

    build_opencode_plugin_entry(merged_name, merged_options)
}

fn canonical_omo_plugin_package_name(plugin_name: &str) -> Option<&'static str> {
    match opencode_plugin_package_name(plugin_name) {
        OMO_CANONICAL_PLUGIN | OMO_LEGACY_PLUGIN => Some(OMO_CANONICAL_PLUGIN),
        OMO_SLIM_PLUGIN => Some(OMO_SLIM_PLUGIN),
        _ => None,
    }
}

pub(crate) fn is_opencode_plugin_equivalent(
    left_plugin_name: &str,
    right_plugin_name: &str,
) -> bool {
    let normalized_left = normalize_opencode_plugin_name(left_plugin_name);
    let normalized_right = normalize_opencode_plugin_name(right_plugin_name);

    match (
        canonical_omo_plugin_package_name(&normalized_left),
        canonical_omo_plugin_package_name(&normalized_right),
    ) {
        (Some(left_omo_package), Some(right_omo_package)) => left_omo_package == right_omo_package,
        _ => normalized_left == normalized_right,
    }
}

pub(crate) fn sanitize_opencode_plugin_list(
    plugin_entries: &[OpenCodePluginEntry],
) -> Vec<OpenCodePluginEntry> {
    let mut sanitized_plugin_entries: Vec<OpenCodePluginEntry> = Vec::new();

    for plugin_entry in plugin_entries {
        let normalized_plugin_entry = normalize_opencode_plugin_entry(plugin_entry);
        let normalized_plugin_name = normalized_plugin_entry.name().trim();
        if normalized_plugin_name.is_empty() {
            continue;
        }

        if let Some(existing_index) = sanitized_plugin_entries.iter().position(|existing_plugin| {
            is_opencode_plugin_equivalent(existing_plugin.name(), normalized_plugin_name)
        }) {
            let merged_plugin_entry = merged_opencode_plugin_entry(
                &sanitized_plugin_entries[existing_index],
                normalized_plugin_entry,
            );
            sanitized_plugin_entries[existing_index] = merged_plugin_entry;
            continue;
        }

        sanitized_plugin_entries.push(normalized_plugin_entry);
    }

    sanitized_plugin_entries
}

fn normalize_favorite_plugin_name(plugin_name: &str) -> String {
    normalize_opencode_plugin_name(plugin_name)
}

fn favorite_plugin_aliases(plugin_name: &str) -> Vec<String> {
    let normalized_plugin_name = normalize_favorite_plugin_name(plugin_name);
    if let Some(version_suffix) = normalized_plugin_name
        .strip_prefix("oh-my-openagent@")
        .map(|suffix| suffix.to_string())
    {
        return vec![
            normalized_plugin_name,
            format!("oh-my-opencode@{}", version_suffix),
        ];
    }

    if normalized_plugin_name == "oh-my-openagent" {
        return vec![normalized_plugin_name, "oh-my-opencode".to_string()];
    }

    vec![normalized_plugin_name]
}

fn favorite_plugin_record_name(record: &Value) -> String {
    record
        .get("plugin_name")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

fn favorite_plugin_record_created_at(record: &Value) -> &str {
    record
        .get("created_at")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
}

fn is_canonical_favorite_plugin_name(plugin_name: &str) -> bool {
    normalize_favorite_plugin_name(plugin_name) == plugin_name
}

fn should_replace_favorite_plugin_record(existing: &Value, candidate: &Value) -> bool {
    let existing_is_canonical =
        is_canonical_favorite_plugin_name(&favorite_plugin_record_name(existing));
    let candidate_is_canonical =
        is_canonical_favorite_plugin_name(&favorite_plugin_record_name(candidate));

    if existing_is_canonical != candidate_is_canonical {
        return candidate_is_canonical;
    }

    favorite_plugin_record_created_at(candidate) > favorite_plugin_record_created_at(existing)
}

fn dedupe_favorite_plugin_records(records: Vec<Value>) -> Vec<Value> {
    let mut records_by_plugin_name: IndexMap<String, Value> = IndexMap::new();

    for record in records {
        let normalized_plugin_name =
            normalize_favorite_plugin_name(&favorite_plugin_record_name(&record));

        if let Some(existing_record) = records_by_plugin_name.get(&normalized_plugin_name) {
            if should_replace_favorite_plugin_record(existing_record, &record) {
                records_by_plugin_name.insert(normalized_plugin_name, record);
            }
            continue;
        }

        records_by_plugin_name.insert(normalized_plugin_name, record);
    }

    let mut deduped_records: Vec<Value> = records_by_plugin_name.into_values().collect();
    deduped_records.sort_by(|left, right| {
        favorite_plugin_record_created_at(left).cmp(favorite_plugin_record_created_at(right))
    });
    deduped_records
}

async fn write_opencode_config_file(
    state: tauri::State<'_, SqliteDbState>,
    config: &OpenCodeConfig,
) -> Result<(), String> {
    let config_path_str = get_opencode_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }
    }

    let mut sanitized_config = config.clone();
    sanitized_config.plugin = sanitized_config
        .plugin
        .as_ref()
        .map(|plugin_names| sanitize_opencode_plugin_list(plugin_names))
        .filter(|plugin_names| !plugin_names.is_empty());

    let json_content = serde_json::to_string_pretty(&sanitized_config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    fs::write(config_path, json_content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        is_opencode_plugin_equivalent, opencode_plugin_package_name, sanitize_opencode_plugin_list,
    };
    use crate::coding::open_code::types::OpenCodePluginEntry;
    use serde_json::json;

    #[test]
    fn opencode_plugin_package_name_keeps_scoped_package_name() {
        assert_eq!(
            opencode_plugin_package_name("@movemama/opencode-legacy@latest"),
            "@movemama/opencode-legacy"
        );
        assert_eq!(
            opencode_plugin_package_name("superpowers@git+https://github.com/obra/superpowers.git"),
            "superpowers"
        );
    }

    #[test]
    fn opencode_plugin_equivalence_handles_scoped_and_legacy_plugins() {
        assert!(is_opencode_plugin_equivalent(
            "@movemama/opencode-legacy@latest",
            "@movemama/opencode-legacy@latest"
        ));
        assert!(!is_opencode_plugin_equivalent(
            "@movemama/opencode-legacy@latest",
            "@mohak34/opencode-notifier@latest"
        ));
        assert!(!is_opencode_plugin_equivalent(
            "@movemama/opencode-legacy@latest",
            "@movemama/opencode-legacy"
        ));
        assert!(is_opencode_plugin_equivalent(
            "oh-my-opencode",
            "oh-my-openagent@latest"
        ));
        assert!(is_opencode_plugin_equivalent(
            "oh-my-opencode-slim",
            "oh-my-opencode-slim@latest"
        ));
    }

    #[test]
    fn sanitize_opencode_plugin_list_dedupes_scoped_duplicates_and_canonicalizes_omo() {
        let plugin_names = vec![
            OpenCodePluginEntry::Name("@movemama/opencode-legacy@latest".to_string()),
            OpenCodePluginEntry::Name("@movemama/opencode-legacy@latest".to_string()),
            OpenCodePluginEntry::Name("@mohak34/opencode-notifier@latest".to_string()),
            OpenCodePluginEntry::Name("oh-my-opencode".to_string()),
            OpenCodePluginEntry::Name("oh-my-openagent@latest".to_string()),
        ];

        assert_eq!(
            sanitize_opencode_plugin_list(&plugin_names),
            vec![
                OpenCodePluginEntry::Name("@movemama/opencode-legacy@latest".to_string()),
                OpenCodePluginEntry::Name("@mohak34/opencode-notifier@latest".to_string()),
                OpenCodePluginEntry::Name("oh-my-openagent@latest".to_string()),
            ]
        );
    }

    #[test]
    fn sanitize_opencode_plugin_list_preserves_tuple_plugin_options() {
        let plugin_names = vec![
            OpenCodePluginEntry::Name("oh-my-opencode".to_string()),
            OpenCodePluginEntry::NameWithOptions((
                "custom-plugin".to_string(),
                json!({ "enabled": true }).as_object().cloned().unwrap(),
            )),
        ];

        assert_eq!(
            sanitize_opencode_plugin_list(&plugin_names),
            vec![
                OpenCodePluginEntry::Name("oh-my-openagent".to_string()),
                OpenCodePluginEntry::NameWithOptions((
                    "custom-plugin".to_string(),
                    json!({ "enabled": true }).as_object().cloned().unwrap(),
                )),
            ]
        );
    }

    #[test]
    fn sanitize_opencode_plugin_list_keeps_existing_options_when_canonical_name_changes() {
        let plugin_names = vec![
            OpenCodePluginEntry::NameWithOptions((
                "oh-my-opencode".to_string(),
                json!({ "enabled": true }).as_object().cloned().unwrap(),
            )),
            OpenCodePluginEntry::Name("oh-my-openagent@latest".to_string()),
        ];

        assert_eq!(
            sanitize_opencode_plugin_list(&plugin_names),
            vec![OpenCodePluginEntry::NameWithOptions((
                "oh-my-openagent@latest".to_string(),
                json!({ "enabled": true }).as_object().cloned().unwrap(),
            ))]
        );
    }

    #[test]
    fn sanitize_opencode_plugin_list_prefers_richer_entry_for_equivalent_plugins() {
        let plugin_names = vec![
            OpenCodePluginEntry::Name("custom-plugin".to_string()),
            OpenCodePluginEntry::NameWithOptions((
                "custom-plugin".to_string(),
                json!({ "mode": "strict" }).as_object().cloned().unwrap(),
            )),
        ];

        assert_eq!(
            sanitize_opencode_plugin_list(&plugin_names),
            vec![OpenCodePluginEntry::NameWithOptions((
                "custom-plugin".to_string(),
                json!({ "mode": "strict" }).as_object().cloned().unwrap(),
            ))]
        );
    }
}

async fn get_opencode_prompt_file_path(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<std::path::PathBuf, String> {
    let config_path_str = get_opencode_config_path(state).await?;
    let config_path = Path::new(&config_path_str);
    let base_dir = config_path
        .parent()
        .map(|path| path.to_path_buf())
        .ok_or_else(|| "Failed to determine OpenCode config directory".to_string())?;

    Ok(base_dir.join("AGENTS.md"))
}

async fn get_local_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<OpenCodePromptConfig>, String> {
    let prompt_path = get_opencode_prompt_file_path(state).await?;
    let Some(prompt_content) = read_prompt_content_file(&prompt_path, "OpenCode")? else {
        return Ok(None);
    };

    let now = chrono::Local::now().to_rfc3339();
    Ok(Some(OpenCodePromptConfig {
        id: "__local__".to_string(),
        name: "default".to_string(),
        content: prompt_content,
        is_applied: true,
        sort_index: None,
        created_at: Some(now.clone()),
        updated_at: Some(now),
    }))
}

async fn write_prompt_content_to_file(
    state: tauri::State<'_, SqliteDbState>,
    prompt_content: Option<&str>,
) -> Result<(), String> {
    let prompt_path = get_opencode_prompt_file_path(state).await?;
    write_prompt_content_file(&prompt_path, prompt_content, "OpenCode")
}

fn emit_prompt_sync_requests<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) {
    #[cfg(target_os = "windows")]
    let _ = _app.emit("wsl-sync-request-opencode", ());
}

fn opencode_prompt_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::json_text("name", OrderDirection::Asc)?,
    ]))
}

fn list_opencode_prompts_from_sqlite(
    sqlite_state: &SqliteDbState,
) -> Result<Vec<OpenCodePromptConfig>, String> {
    let order = opencode_prompt_order()?;
    sqlite_state.with_conn(|conn| {
        Ok(db_list(conn, DbTable::OpenCodePromptConfig, Some(&order))?
            .into_iter()
            .map(adapter::from_db_value_prompt_config)
            .collect())
    })
}

fn get_opencode_prompt_from_sqlite(
    sqlite_state: &SqliteDbState,
    config_id: &str,
) -> Result<Option<OpenCodePromptConfig>, String> {
    sqlite_state.with_conn(|conn| {
        Ok(db_get(conn, DbTable::OpenCodePromptConfig, config_id)?
            .map(adapter::from_db_value_prompt_config))
    })
}

fn put_opencode_prompt_to_sqlite(
    sqlite_state: &SqliteDbState,
    config_id: &str,
    content: &OpenCodePromptConfigContent,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| {
        db_put(
            conn,
            DbTable::OpenCodePromptConfig,
            config_id,
            &adapter::to_db_value_prompt_config(content),
        )
    })
}

// ============================================================================
// OpenCode Commands
// ============================================================================

/// Get OpenCode config file path with priority: common config > system env > shell config > default
#[tauri::command]
pub async fn get_opencode_config_path(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<String, String> {
    // 1. Check common config (highest priority)
    if let Some(common_config) = get_opencode_common_config(state.clone()).await? {
        if let Some(custom_path) = common_config.config_path {
            if !custom_path.is_empty() {
                return Ok(custom_path);
            }
        }
    }

    // 2. Check system environment variable (second priority)
    if let Ok(env_path) = std::env::var("OPENCODE_CONFIG") {
        if !env_path.is_empty() {
            return Ok(env_path);
        }
    }

    // 3. Check shell configuration files (third priority)
    if let Some(shell_path) = super::shell_env::get_env_from_shell_config("OPENCODE_CONFIG") {
        if !shell_path.is_empty() {
            return Ok(shell_path);
        }
    }

    // 4. Return default path
    get_default_config_path()
}

/// Get OpenCode config path info including source
#[tauri::command]
pub async fn get_opencode_config_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<ConfigPathInfo, String> {
    // 1. Check common config (highest priority)
    if let Some(common_config) = get_opencode_common_config(state.clone()).await? {
        if let Some(custom_path) = common_config.config_path {
            if !custom_path.is_empty() {
                return Ok(ConfigPathInfo {
                    path: custom_path,
                    source: "custom".to_string(),
                });
            }
        }
    }

    // 2. Check system environment variable (second priority)
    if let Ok(env_path) = std::env::var("OPENCODE_CONFIG") {
        if !env_path.is_empty() {
            return Ok(ConfigPathInfo {
                path: env_path,
                source: "env".to_string(),
            });
        }
    }

    // 3. Check shell configuration files (third priority)
    if let Some(shell_path) = super::shell_env::get_env_from_shell_config("OPENCODE_CONFIG") {
        if !shell_path.is_empty() {
            return Ok(ConfigPathInfo {
                path: shell_path,
                source: "shell".to_string(),
            });
        }
    }

    // 4. Return default path
    let default_path = get_default_config_path()?;
    Ok(ConfigPathInfo {
        path: default_path,
        source: "default".to_string(),
    })
}

/// Helper function to get default config path
/// Returns the actual config file path (checks .jsonc first, then .json)
pub fn get_default_config_path() -> Result<String, String> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get home directory".to_string())?;

    let config_dir = Path::new(&home_dir).join(".config").join("opencode");

    // Check for .jsonc first, then .json
    let jsonc_path = config_dir.join("opencode.jsonc");
    let json_path = config_dir.join("opencode.json");

    if jsonc_path.exists() {
        Ok(jsonc_path.to_string_lossy().to_string())
    } else if json_path.exists() {
        Ok(json_path.to_string_lossy().to_string())
    } else {
        // Return default path for new file
        Ok(jsonc_path.to_string_lossy().to_string())
    }
}

/// Read OpenCode configuration file with detailed result
#[tauri::command]
pub async fn read_opencode_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<ReadConfigResult, String> {
    let config_path_str = get_opencode_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        return Ok(ReadConfigResult::NotFound {
            path: config_path_str,
        });
    }

    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            return Ok(ReadConfigResult::Error {
                error: format!("Failed to read config file: {}", e),
            });
        }
    };

    match json5::from_str::<OpenCodeConfig>(&content) {
        Ok(mut config) => {
            // Initialize provider if missing
            if config.provider.is_none() {
                config.provider = Some(IndexMap::<String, OpenCodeProvider>::new());
            }
            config.plugin = config
                .plugin
                .as_ref()
                .map(|plugin_names| sanitize_opencode_plugin_list(plugin_names))
                .filter(|plugin_names| !plugin_names.is_empty());

            // Fill missing name fields with provider key
            // Fill missing npm fields with smart default based on provider key/name
            if let Some(ref mut providers) = config.provider {
                for (key, provider) in providers.iter_mut() {
                    if provider.name.is_none() {
                        provider.name = Some(key.clone());
                    }
                    if provider.npm.is_none() {
                        // Smart npm inference based on provider key or name (case-insensitive)
                        let key_lower = key.to_lowercase();
                        let name_lower = provider
                            .name
                            .as_ref()
                            .map(|n| n.to_lowercase())
                            .unwrap_or_default();

                        let inferred_npm = if key_lower.contains("google")
                            || key_lower.contains("gemini")
                            || name_lower.contains("google")
                            || name_lower.contains("gemini")
                        {
                            "@ai-sdk/google"
                        } else if key_lower.contains("anthropic")
                            || key_lower.contains("claude")
                            || name_lower.contains("anthropic")
                            || name_lower.contains("claude")
                        {
                            "@ai-sdk/anthropic"
                        } else {
                            "@ai-sdk/openai-compatible"
                        };

                        provider.npm = Some(inferred_npm.to_string());
                    }
                }
            }

            Ok(ReadConfigResult::Success { config })
        }
        Err(e) => {
            // Truncate content preview to first 500 chars
            let preview = if content.len() > 500 {
                format!("{}...", &content[..500])
            } else {
                content
            };

            Ok(ReadConfigResult::ParseError {
                path: config_path_str,
                error: e.to_string(),
                content_preview: Some(preview),
            })
        }
    }
}

/// Backup OpenCode configuration file by renaming it with .bak.{timestamp} suffix
#[tauri::command]
pub async fn backup_opencode_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<String, String> {
    let config_path_str = get_opencode_config_path(state).await?;
    let config_path = Path::new(&config_path_str);

    if !config_path.exists() {
        return Err("Config file does not exist".to_string());
    }

    // Generate backup path with timestamp
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let backup_path_str = format!("{}.bak.{}", config_path_str, timestamp);
    let backup_path = Path::new(&backup_path_str);

    // Rename the file to backup
    fs::rename(config_path, backup_path)
        .map_err(|e| format!("Failed to backup config file: {}", e))?;

    Ok(backup_path_str.to_string())
}

/// Save OpenCode configuration file
#[tauri::command]
pub async fn save_opencode_config<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle<R>,
    config: OpenCodeConfig,
) -> Result<(), String> {
    apply_config_internal(state, &app, config, false).await
}

/// Internal function to save config and emit events
pub async fn apply_config_internal<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config: OpenCodeConfig,
    from_tray: bool,
) -> Result<(), String> {
    write_opencode_config_file(state.clone(), &config).await?;

    // Notify based on source
    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);

    // Trigger WSL sync via event (Windows only)
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-opencode", ());

    // Async sync providers to favorite DB in background (non-blocking)
    let db = state.db().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = sync_providers_from_config(&db, &config).await {
            eprintln!("Background sync_providers_from_config failed: {}", e);
        }
    });

    Ok(())
}

// ============================================================================
// OpenCode Prompt Config Commands
// ============================================================================

#[tauri::command]
pub async fn list_opencode_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<OpenCodePromptConfig>, String> {
    let prompts = list_opencode_prompts_from_sqlite(&state)?;
    if prompts.is_empty() {
        if let Some(local_config) = get_local_prompt_config(state).await? {
            return Ok(vec![local_config]);
        }
    }
    Ok(prompts)
}

#[tauri::command]
pub async fn create_opencode_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OpenCodePromptConfigInput,
) -> Result<OpenCodePromptConfig, String> {
    let now = chrono::Local::now().to_rfc3339();
    let next_sort_index = state.with_conn(|conn| {
        Ok(db_max_i64(
            conn,
            DbTable::OpenCodePromptConfig,
            &JsonFieldPath::new("sort_index")?,
        )?
        .map(|value| value as i32 + 1)
        .unwrap_or(0))
    })?;

    let content = OpenCodePromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: false,
        sort_index: Some(next_sort_index),
        created_at: now.clone(),
        updated_at: now,
    };

    let prompt_id = db_new_id();
    put_opencode_prompt_to_sqlite(&state, &prompt_id, &content)?;

    let _ = app.emit("config-changed", "window");

    Ok(OpenCodePromptConfig {
        id: prompt_id,
        name: content.name,
        content: content.content,
        is_applied: content.is_applied,
        sort_index: content.sort_index,
        created_at: Some(content.created_at),
        updated_at: Some(content.updated_at),
    })
}

#[tauri::command]
pub async fn update_opencode_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OpenCodePromptConfigInput,
) -> Result<OpenCodePromptConfig, String> {
    let config_id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;
    let existing_prompt = get_opencode_prompt_from_sqlite(&state, &config_id)?;

    let (created_at, is_applied, sort_index) = if let Some(prompt) = existing_prompt {
        (
            prompt
                .created_at
                .unwrap_or_else(|| chrono::Local::now().to_rfc3339()),
            prompt.is_applied,
            prompt.sort_index,
        )
    } else {
        return Err(format!("Prompt config '{}' not found", config_id));
    };

    let now = chrono::Local::now().to_rfc3339();
    let content = OpenCodePromptConfigContent {
        name: input.name,
        content: input.content.clone(),
        is_applied,
        sort_index,
        created_at,
        updated_at: now.clone(),
    };
    put_opencode_prompt_to_sqlite(&state, &config_id, &content)?;

    if is_applied {
        write_prompt_content_to_file(state.clone(), Some(input.content.as_str())).await?;
        emit_prompt_sync_requests(&app);
    }

    let _ = app.emit("config-changed", "window");

    Ok(OpenCodePromptConfig {
        id: config_id,
        name: content.name,
        content: content.content,
        is_applied,
        sort_index,
        created_at: Some(content.created_at),
        updated_at: Some(now),
    })
}

#[tauri::command]
pub async fn delete_opencode_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    state.with_conn(|conn| db_delete(conn, DbTable::OpenCodePromptConfig, &id).map(|_| ()))?;

    let _ = app.emit("config-changed", "window");

    Ok(())
}

pub async fn apply_prompt_config_internal<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    if config_id == "__local__" {
        let local_prompt = get_local_prompt_config(state.clone())
            .await?
            .ok_or_else(|| "Local default prompt not found".to_string())?;
        write_prompt_content_to_file(state, Some(local_prompt.content.as_str())).await?;

        let payload = if from_tray { "tray" } else { "window" };
        let _ = app.emit("config-changed", payload);
        emit_prompt_sync_requests(app);

        return Ok(());
    }

    let prompt_config = get_opencode_prompt_from_sqlite(&state, config_id)?
        .ok_or_else(|| format!("Prompt config '{}' not found", config_id))?;

    let now = chrono::Local::now().to_rfc3339();

    state.with_conn(|conn| {
        db_patch_where_bool(
            conn,
            DbTable::OpenCodePromptConfig,
            &JsonFieldPath::new("is_applied")?,
            true,
            &[
                ("is_applied", serde_json::Value::Bool(false)),
                ("updated_at", serde_json::Value::String(now.clone())),
            ],
        )?;
        db_patch_fields(
            conn,
            DbTable::OpenCodePromptConfig,
            config_id,
            &[
                ("is_applied", serde_json::Value::Bool(true)),
                ("updated_at", serde_json::Value::String(now.clone())),
            ],
        )?;
        Ok(())
    })?;

    write_prompt_content_to_file(state.clone(), Some(prompt_config.content.as_str())).await?;

    let payload = if from_tray { "tray" } else { "window" };
    let _ = app.emit("config-changed", payload);
    emit_prompt_sync_requests(app);

    Ok(())
}

#[tauri::command]
pub async fn apply_opencode_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    apply_prompt_config_internal(state, &app, &config_id, false).await
}

#[tauri::command]
pub async fn reorder_opencode_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    ids: Vec<String>,
) -> Result<(), String> {
    for (index, id) in ids.iter().enumerate() {
        state.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::OpenCodePromptConfig,
                id,
                &[(
                    "sort_index",
                    serde_json::Value::Number((index as i64).into()),
                )],
            )
            .map(|_| ())
        })?;
    }
    let _ = app.emit("config-changed", "window");

    Ok(())
}

#[tauri::command]
pub async fn save_opencode_local_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OpenCodePromptConfigInput,
) -> Result<OpenCodePromptConfig, String> {
    let prompt_content = if input.content.trim().is_empty() {
        get_local_prompt_config(state.clone())
            .await?
            .map(|config| config.content)
            .unwrap_or_default()
    } else {
        input.content
    };

    let created = create_opencode_prompt_config(
        state.clone(),
        app.clone(),
        OpenCodePromptConfigInput {
            id: None,
            name: input.name,
            content: prompt_content,
        },
    )
    .await?;

    apply_prompt_config_internal(state.clone(), &app, &created.id, false).await?;

    Ok(get_opencode_prompt_from_sqlite(&state, &created.id)?.unwrap_or(created))
}

// ============================================================================
// OpenCode Common Config Commands
// ============================================================================

/// Get OpenCode common config
#[tauri::command]
pub async fn get_opencode_common_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<OpenCodeCommonConfig>, String> {
    state.with_conn(|conn| {
        Ok(db_get(conn, DbTable::OpenCodeCommonConfig, "common")?.map(adapter::from_db_value))
    })
}

/// Save OpenCode common config
#[tauri::command]
pub async fn save_opencode_common_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config: OpenCodeCommonConfig,
) -> Result<(), String> {
    let db = state.db();
    let previous_skills_path = runtime_location::get_tool_skills_path_async(&db, "opencode").await;

    let json_data = adapter::to_db_value(&config);

    db.with_conn(|conn| db_put(conn, DbTable::OpenCodeCommonConfig, "common", &json_data))?;
    runtime_location::refresh_runtime_location_cache_for_module_async(&db, "opencode").await?;

    resync_all_skills_if_tool_path_changed(app, state.inner(), "opencode", previous_skills_path)
        .await;

    Ok(())
}

// ============================================================================
// Free Models Commands
// ============================================================================

/// Get OpenCode free models from opencode channel
/// Returns free models where cost.input and cost.output are both 0
#[tauri::command]
pub async fn get_opencode_free_models(
    state: tauri::State<'_, SqliteDbState>,
    force_refresh: Option<bool>,
) -> Result<GetFreeModelsResponse, String> {
    let (free_models, from_cache, updated_at) =
        super::free_models::get_free_models(&state, force_refresh.unwrap_or(false)).await?;
    let total = free_models.len();

    Ok(GetFreeModelsResponse {
        free_models,
        total,
        from_cache,
        updated_at,
    })
}

/// Get provider models data by provider_id
/// Returns the complete model information for a specific provider
#[tauri::command]
pub async fn get_provider_models(
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
) -> Result<Option<ProviderModelsData>, String> {
    super::free_models::get_provider_models_internal(&state, &provider_id).await
}

// ============================================================================
// Unified Models Commands
// ============================================================================

/// Get unified model list combining custom providers and official providers from auth.json
/// Returns all available models sorted by display name
#[tauri::command]
pub async fn get_opencode_unified_models(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<UnifiedModelOption>, String> {
    // Read auth.json to get official provider ids
    let auth_channels = super::free_models::read_auth_channels();

    // Read config to get custom providers
    let result = read_opencode_config(state.clone()).await?;
    let custom_providers = match result {
        ReadConfigResult::Success { config } => config.provider,
        _ => None,
    };

    // Get unified model list
    let models =
        super::free_models::get_unified_models(&state, custom_providers.as_ref(), &auth_channels)
            .await;

    Ok(models)
}

// ============================================================================
// Official Auth Providers Commands
// ============================================================================

/// Get official auth providers data from auth.json
/// Returns providers split into standalone (not in custom config) and merged (models only)
#[tauri::command]
pub async fn get_opencode_auth_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<GetAuthProvidersResponse, String> {
    // Read config to get custom providers
    let result = read_opencode_config(state.clone()).await?;
    let custom_providers = match result {
        ReadConfigResult::Success { config } => config.provider,
        _ => None,
    };

    // Get auth providers data
    let response =
        super::free_models::get_auth_providers_data(&state, custom_providers.as_ref()).await;

    Ok(response)
}

// ============================================================================
// Favorite Plugin Commands
// ============================================================================

/// Default favorite plugins to initialize on first use
const DEFAULT_FAVORITE_PLUGINS: &[&str] = &[
    "oh-my-openagent@latest",
    "oh-my-opencode-slim",
    "opencode-antigravity-auth",
    "opencode-openai-codex-auth",
    "opencode-omit-max-tokens",
    "opencode-axonhub-tracing",
];

/// Initialize default favorite plugins if database is empty
async fn init_default_favorite_plugins(db: &crate::db::SqliteDbState) -> Result<(), String> {
    let now = chrono::Local::now().to_rfc3339();

    for plugin_name in DEFAULT_FAVORITE_PLUGINS {
        let normalized_plugin_name = normalize_favorite_plugin_name(plugin_name);
        let existing = db.with_conn(|conn| {
            db_get(
                conn,
                DbTable::OpenCodeFavoritePlugin,
                &normalized_plugin_name,
            )
        })?;
        if existing.is_some() {
            continue;
        }
        let payload = serde_json::json!({
            "plugin_name": normalized_plugin_name,
            "created_at": now,
        });
        db.with_conn(|conn| {
            db_put(
                conn,
                DbTable::OpenCodeFavoritePlugin,
                payload
                    .get("plugin_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                &payload,
            )
        })?;
    }

    Ok(())
}

fn favorite_plugin_order() -> OrderSpec {
    OrderSpec::single(
        OrderField::json_text("created_at", OrderDirection::Asc)
            .expect("valid favorite plugin order field"),
    )
}

fn favorite_provider_order() -> OrderSpec {
    OrderSpec::single(
        OrderField::json_text("created_at", OrderDirection::Asc)
            .expect("valid favorite provider order field"),
    )
}

fn list_favorite_plugin_records(db: &crate::db::SqliteDbState) -> Result<Vec<Value>, String> {
    let order = favorite_plugin_order();
    db.with_conn(|conn| db_list(conn, DbTable::OpenCodeFavoritePlugin, Some(&order)))
}

fn list_favorite_provider_records(db: &crate::db::SqliteDbState) -> Result<Vec<Value>, String> {
    let order = favorite_provider_order();
    db.with_conn(|conn| db_list(conn, DbTable::OpenCodeFavoriteProvider, Some(&order)))
}

fn favorite_plugin_payload(plugin_name: &str, created_at: &str) -> Value {
    serde_json::json!({
        "plugin_name": plugin_name,
        "created_at": created_at,
    })
}

fn favorite_provider_payload(
    provider_id: &str,
    provider_config: &OpenCodeProvider,
    diagnostics: Option<OpenCodeDiagnosticsConfig>,
    created_at: &str,
    updated_at: &str,
) -> Result<Value, String> {
    let npm = provider_config.npm.clone().unwrap_or_default();
    let base_url = provider_config
        .options
        .as_ref()
        .and_then(|options| options.base_url.clone())
        .unwrap_or_default();
    let provider_config_json = serde_json::to_value(provider_config)
        .map_err(|error| format!("Failed to serialize provider config: {}", error))?;

    Ok(serde_json::json!({
        "provider_id": provider_id,
        "npm": npm,
        "base_url": base_url,
        "provider_config": provider_config_json,
        "diagnostics": diagnostics,
        "created_at": created_at,
        "updated_at": updated_at,
    }))
}

fn find_favorite_plugin_record(
    db: &crate::db::SqliteDbState,
    aliases: &[String],
) -> Result<Option<Value>, String> {
    let records = list_favorite_plugin_records(db)?;
    Ok(dedupe_favorite_plugin_records(records)
        .into_iter()
        .find(|record| {
            let plugin_name = favorite_plugin_record_name(record);
            aliases
                .iter()
                .any(|alias| is_opencode_plugin_equivalent(alias, &plugin_name))
        }))
}

fn find_favorite_provider_record(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<Option<Value>, String> {
    Ok(list_favorite_provider_records(db)?
        .into_iter()
        .find(|record| record.get("provider_id").and_then(Value::as_str) == Some(provider_id)))
}

/// List all favorite plugins
/// Auto-initializes default plugins if database is empty
#[tauri::command]
pub async fn list_opencode_favorite_plugins(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<OpenCodeFavoritePlugin>, String> {
    let db = state.db();

    let is_empty = db.with_conn(|conn| db_count(conn, DbTable::OpenCodeFavoritePlugin))? == 0;

    // Initialize default plugins if empty
    if is_empty {
        init_default_favorite_plugins(db).await?;
    }

    let plugins = dedupe_favorite_plugin_records(list_favorite_plugin_records(db)?)
        .into_iter()
        .map(adapter::from_db_value_favorite_plugin)
        .collect();
    Ok(plugins)
}

/// Add a favorite plugin
/// Returns the created plugin, or existing one if already exists
#[tauri::command]
pub async fn add_opencode_favorite_plugin(
    state: tauri::State<'_, SqliteDbState>,
    plugin_name: String,
) -> Result<OpenCodeFavoritePlugin, String> {
    let db = state.db();
    let now = chrono::Local::now().to_rfc3339();
    let normalized_plugin_name = normalize_favorite_plugin_name(&plugin_name);
    let plugin_aliases = favorite_plugin_aliases(&plugin_name);

    if let Some(record) = find_favorite_plugin_record(db, &plugin_aliases)? {
        return Ok(adapter::from_db_value_favorite_plugin(record));
    }

    let payload = favorite_plugin_payload(&normalized_plugin_name, &now);
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::OpenCodeFavoritePlugin,
            &normalized_plugin_name,
            &payload,
        )
    })?;

    let record = db
        .with_conn(|conn| {
            db_get(
                conn,
                DbTable::OpenCodeFavoritePlugin,
                &normalized_plugin_name,
            )
        })?
        .ok_or_else(|| "Failed to find favorite plugin after insert".to_string())?;
    Ok(adapter::from_db_value_favorite_plugin(record))
}

/// Delete a favorite plugin by plugin name
#[tauri::command]
pub async fn delete_opencode_favorite_plugin(
    state: tauri::State<'_, SqliteDbState>,
    plugin_name: String,
) -> Result<(), String> {
    let db = state.db();
    let plugin_aliases = favorite_plugin_aliases(&plugin_name);
    let records = list_favorite_plugin_records(db)?;
    for record in records {
        let record_plugin_name = favorite_plugin_record_name(&record);
        let should_delete = plugin_aliases
            .iter()
            .any(|alias| is_opencode_plugin_equivalent(alias, &record_plugin_name));
        if !should_delete {
            continue;
        }
        let Some(id) = record.get("id").and_then(Value::as_str) else {
            continue;
        };
        db.with_conn(|conn| db_delete(conn, DbTable::OpenCodeFavoritePlugin, id).map(|_| ()))?;
    }

    Ok(())
}

// ============================================================================
// Favorite Provider Commands
// ============================================================================

/// Sync providers from config file to database with diff comparison.
/// - Identical records are skipped
/// - Changed records are updated
/// - New providers are inserted
async fn sync_providers_from_config(
    db: &crate::db::SqliteDbState,
    config: &OpenCodeConfig,
) -> Result<(), String> {
    let providers = match config.provider {
        Some(ref p) => p,
        None => return Ok(()),
    };

    // Fetch all existing favorite providers in one query
    let existing_records = list_favorite_provider_records(db)?;

    // Build a lookup map: provider_id -> (npm, base_url, provider_config_json)
    let mut existing_map: std::collections::HashMap<String, (String, String, Value)> =
        std::collections::HashMap::new();
    for record in &existing_records {
        if let Some(provider_id) = record.get("provider_id").and_then(|v| v.as_str()) {
            let npm = record
                .get("npm")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let base_url = record
                .get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let provider_config_val = record
                .get("provider_config")
                .cloned()
                .unwrap_or(Value::Null);
            existing_map.insert(
                provider_id.to_string(),
                (npm, base_url, provider_config_val),
            );
        }
    }

    let now = chrono::Local::now().to_rfc3339();

    for (provider_id, provider_config) in providers.iter() {
        let npm = provider_config.npm.clone().unwrap_or_default();
        let base_url = provider_config
            .options
            .as_ref()
            .and_then(|o| o.base_url.clone())
            .unwrap_or_default();
        let provider_config_json = serde_json::to_value(provider_config)
            .map_err(|e| format!("Failed to serialize provider config: {}", e))?;

        if let Some((existing_npm, existing_base_url, existing_config)) =
            existing_map.get(provider_id)
        {
            // Record exists - check if anything changed
            if *existing_npm == npm
                && *existing_base_url == base_url
                && *existing_config == provider_config_json
            {
                // Identical, skip
                continue;
            }
        }

        let existing_record = find_favorite_provider_record(db, provider_id)?;
        let created_at = existing_record
            .as_ref()
            .and_then(|record| record.get("created_at"))
            .and_then(Value::as_str)
            .unwrap_or(&now)
            .to_string();
        let record_id = existing_record
            .as_ref()
            .and_then(|record| record.get("id"))
            .and_then(Value::as_str)
            .unwrap_or(provider_id)
            .to_string();
        let payload = serde_json::json!({
            "provider_id": provider_id,
            "npm": npm,
            "base_url": base_url,
            "provider_config": provider_config_json,
            "created_at": created_at,
            "updated_at": now,
        });
        db.with_conn(|conn| {
            db_put(
                conn,
                DbTable::OpenCodeFavoriteProvider,
                &record_id,
                &payload,
            )
        })?;
    }

    Ok(())
}

/// List all favorite providers
/// Pure SELECT query - sync is handled by apply_config_internal on config save
#[tauri::command]
pub async fn list_opencode_favorite_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<OpenCodeFavoriteProvider>, String> {
    let db = state.db();

    let providers = list_favorite_provider_records(db)?
        .into_iter()
        .filter_map(adapter::from_db_value_favorite_provider)
        .collect();
    Ok(providers)
}

/// Upsert (create or update) a favorite provider
/// Called automatically when user adds/modifies a provider
#[tauri::command]
pub async fn upsert_opencode_favorite_provider(
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
    provider_config: OpenCodeProvider,
    diagnostics: Option<OpenCodeDiagnosticsConfig>,
) -> Result<OpenCodeFavoriteProvider, String> {
    let db = state.db();
    let now = chrono::Local::now().to_rfc3339();

    // Read existing record to preserve created_at and diagnostics if not provided
    let existing_record = find_favorite_provider_record(db, &provider_id)?
        .and_then(adapter::from_db_value_favorite_provider);
    let created_at = existing_record
        .as_ref()
        .map(|record| record.created_at.clone())
        .unwrap_or_else(|| now.clone());
    let diagnostics_to_save = diagnostics.or_else(|| {
        existing_record
            .as_ref()
            .and_then(|record| record.diagnostics.clone())
    });
    let record_id = existing_record
        .as_ref()
        .map(|record| record.id.clone())
        .unwrap_or_else(|| provider_id.clone());
    let payload = favorite_provider_payload(
        &provider_id,
        &provider_config,
        diagnostics_to_save,
        &created_at,
        &now,
    )?;

    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::OpenCodeFavoriteProvider,
            &record_id,
            &payload,
        )
    })?;

    // Fetch and return the record
    let record = db
        .with_conn(|conn| db_get(conn, DbTable::OpenCodeFavoriteProvider, &record_id))?
        .ok_or_else(|| "Failed to find favorite provider after upsert".to_string())?;
    adapter::from_db_value_favorite_provider(record)
        .ok_or_else(|| "Failed to parse favorite provider".to_string())
}

/// Delete a favorite provider from database
#[tauri::command]
pub async fn delete_opencode_favorite_provider(
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
) -> Result<(), String> {
    let db = state.db();

    let records = list_favorite_provider_records(db)?;
    for record in records {
        if record.get("provider_id").and_then(Value::as_str) != Some(&provider_id) {
            continue;
        }
        let Some(id) = record.get("id").and_then(Value::as_str) else {
            continue;
        };
        db.with_conn(|conn| db_delete(conn, DbTable::OpenCodeFavoriteProvider, id).map(|_| ()))?;
    }

    Ok(())
}

#[tauri::command]
pub async fn list_opencode_all_api_hub_providers(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<OpenCodeAllApiHubProvidersResult, String> {
    let _ = state;
    let discovery = all_api_hub::list_provider_candidates()?;

    let providers = discovery
        .providers
        .iter()
        .map(|candidate| OpenCodeAllApiHubProvider {
            provider_id: candidate.provider_id.clone(),
            name: candidate.name.clone(),
            npm: candidate.npm.clone(),
            base_url: candidate.base_url.clone(),
            requires_browser_open: candidate
                .auth_type
                .as_deref()
                .map(|value| value.trim().eq_ignore_ascii_case("cookie"))
                .unwrap_or(false),
            is_disabled: candidate.is_disabled,
            has_api_key: candidate
                .api_key
                .as_ref()
                .map(|v| !v.is_empty())
                .unwrap_or(false),
            api_key_preview: candidate
                .api_key
                .as_ref()
                .map(|value| all_api_hub::mask_api_key_preview(value)),
            balance_usd: candidate.balance_usd,
            balance_cny: candidate.balance_cny,
            site_name: candidate.site_name.clone(),
            site_type: candidate.site_type.clone(),
            account_label: candidate.account_label.clone(),
            source_profile_name: candidate.source_profile_name.clone(),
            source_extension_id: candidate.source_extension_id.clone(),
            provider_config: all_api_hub::candidate_to_opencode_provider(candidate),
        })
        .collect();

    Ok(OpenCodeAllApiHubProvidersResult {
        found: discovery.found,
        profiles: discovery.profiles,
        providers,
        message: discovery.message,
    })
}

#[tauri::command]
pub async fn resolve_opencode_all_api_hub_providers(
    state: tauri::State<'_, SqliteDbState>,
    request: ResolveOpenCodeAllApiHubProvidersRequest,
) -> Result<Vec<OpenCodeAllApiHubProvider>, String> {
    let providers =
        all_api_hub::resolve_provider_candidates_with_keys(&state, &request.provider_ids).await?;

    Ok(providers
        .iter()
        .map(|candidate| OpenCodeAllApiHubProvider {
            provider_id: candidate.provider_id.clone(),
            name: candidate.name.clone(),
            npm: candidate.npm.clone(),
            base_url: candidate.base_url.clone(),
            requires_browser_open: candidate
                .auth_type
                .as_deref()
                .map(|value| value.trim().eq_ignore_ascii_case("cookie"))
                .unwrap_or(false),
            is_disabled: candidate.is_disabled,
            has_api_key: candidate
                .api_key
                .as_ref()
                .map(|v| !v.is_empty())
                .unwrap_or(false),
            api_key_preview: candidate
                .api_key
                .as_ref()
                .map(|value| all_api_hub::mask_api_key_preview(value)),
            balance_usd: candidate.balance_usd,
            balance_cny: candidate.balance_cny,
            site_name: candidate.site_name.clone(),
            site_type: candidate.site_type.clone(),
            account_label: candidate.account_label.clone(),
            source_profile_name: candidate.source_profile_name.clone(),
            source_extension_id: candidate.source_extension_id.clone(),
            provider_config: all_api_hub::candidate_to_opencode_provider(candidate),
        })
        .collect())
}
