use chrono::Local;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::adapter;
use super::constants::{
    builtin_provider_name, is_builtin_provider, HERMES_CONFIG_FILE, HERMES_ENV_KEY,
    HERMES_PROMPT_FILE,
};
use super::types::*;
use crate::coding::db_id::db_new_id;
use crate::coding::open_code::shell_env;
use crate::coding::prompt_file::{read_prompt_content_file, write_prompt_content_file};
use crate::coding::runtime_location;
use crate::db::helpers::{
    db_delete, db_get, db_list, db_max_i64, db_patch_fields, db_put, db_update_applied_status,
};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use tauri::{Emitter, Manager, Runtime};

/// Top-level YAML keys managed by this module (or by Hermes itself) and thus
/// hidden from / preserved across the "Other settings" editor.
const HERMES_OTHER_SETTINGS_PROTECTED_KEYS: [&str; 5] = [
    "model",
    "custom_providers",
    "providers",
    "mcp_servers",
    "_config_version",
];

fn get_home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "Failed to get home directory".to_string())
}

/// Platform-default Hermes config directory.
///
/// Windows: `%LOCALAPPDATA%\hermes` (falling back to `<home>\AppData\Local\hermes`
/// when LOCALAPPDATA is unset/blank). macOS / Linux: `~/.hermes`.
fn default_hermes_config_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var_os("LOCALAPPDATA")
            .map(|value| value.to_string_lossy().trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                get_home_dir()
                    .unwrap_or_default()
                    .join("AppData")
                    .join("Local")
            });
        Ok(local.join("hermes"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(get_home_dir()?.join(".hermes"))
    }
}

fn get_hermes_config_dir_from_shell() -> Option<PathBuf> {
    shell_env::get_env_from_shell_config(HERMES_ENV_KEY)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

/// Resolve the config dir without consulting the DB: env -> shell -> default.
fn resolve_hermes_config_dir_without_db() -> Result<PathBuf, String> {
    if let Ok(env_path) = std::env::var(HERMES_ENV_KEY) {
        if !env_path.trim().is_empty() {
            return Ok(PathBuf::from(env_path));
        }
    }
    if let Some(shell_path) = get_hermes_config_dir_from_shell() {
        return Ok(shell_path);
    }
    default_hermes_config_dir()
}

/// `(path, source)` resolution without DB. Source is one of
/// `env` / `shell` / `default`, mirroring `runtime_location`.
pub(crate) fn resolve_hermes_path_without_db() -> (PathBuf, String) {
    if let Ok(env_path) = std::env::var(HERMES_ENV_KEY) {
        if !env_path.trim().is_empty() {
            return (PathBuf::from(env_path), "env".to_string());
        }
    }
    if let Some(shell_path) = get_hermes_config_dir_from_shell() {
        return (shell_path, "shell".to_string());
    }
    (
        default_hermes_config_dir().unwrap_or_default(),
        "default".to_string(),
    )
}

/// Custom config dir stored in the DB (id fixed to "common").
///
/// The shared runtime-location cache calls this resolver so WSL Direct status
/// follows the same directory precedence as Hermes's file operations.
pub async fn get_hermes_custom_config_dir_async(db: &SqliteDbState) -> Option<PathBuf> {
    db.with_conn(|conn| db_get(conn, DbTable::HermesSettingsConfig, "common"))
        .ok()
        .flatten()
        .and_then(|value| adapter::settings_from_db_value(value).config_dir)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

pub async fn get_hermes_config_dir_from_db_async(
    db: &SqliteDbState,
) -> Result<(PathBuf, String), String> {
    if let Some(custom) = get_hermes_custom_config_dir_async(db).await {
        return Ok((custom, "custom".to_string()));
    }
    Ok(resolve_hermes_path_without_db())
}

fn get_hermes_config_dir_from_db_sync(db: &SqliteDbState) -> Result<(PathBuf, String), String> {
    let custom = db
        .with_conn(|conn| db_get(conn, DbTable::HermesSettingsConfig, "common"))
        .ok()
        .flatten()
        .and_then(|value| adapter::settings_from_db_value(value).config_dir)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from);
    if let Some(custom) = custom {
        return Ok((custom, "custom".to_string()));
    }
    Ok(resolve_hermes_path_without_db())
}

pub fn get_hermes_root_path_info_from_db(db: &SqliteDbState) -> Result<HermesPathInfo, String> {
    let (path, source) = get_hermes_config_dir_from_db_sync(db)?;
    Ok(HermesPathInfo {
        path: path.to_string_lossy().to_string(),
        source,
    })
}

pub async fn get_hermes_root_path_info_from_db_async(
    db: &SqliteDbState,
) -> Result<HermesPathInfo, String> {
    let (path, source) = get_hermes_config_dir_from_db_async(db).await?;
    Ok(HermesPathInfo {
        path: path.to_string_lossy().to_string(),
        source,
    })
}

pub async fn get_hermes_root_dir_from_db_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_hermes_config_dir_from_db_async(db).await?.0)
}

pub fn get_hermes_config_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join(HERMES_CONFIG_FILE)
}

pub fn get_hermes_prompt_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join(HERMES_PROMPT_FILE)
}

pub async fn get_hermes_config_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_hermes_config_path_from_root(
        &get_hermes_root_dir_from_db_async(db).await?,
    ))
}

pub async fn get_hermes_prompt_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_hermes_prompt_path_from_root(
        &get_hermes_root_dir_from_db_async(db).await?,
    ))
}

// ---------------------------------------------------------------------------
// YAML I/O
// ---------------------------------------------------------------------------

/// Check if a line is a YAML top-level key (mapping key at column 0).
/// Mirrors cc-switch `is_top_level_key_line`: must start at column 0, not be a
/// comment / sequence item, and contain `:` followed by space/tab/EOL/CR.
fn is_top_level_key_line(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    let first_char = line.as_bytes()[0];
    if first_char == b' ' || first_char == b'\t' || first_char == b'#' || first_char == b'-' {
        return false;
    }
    if let Some(colon_pos) = line.find(':') {
        let after_colon = &line[colon_pos + 1..];
        after_colon.is_empty() || after_colon.starts_with([' ', '\t', '\r', '\n'])
    } else {
        false
    }
}

/// Remove duplicate top-level YAML sections, keeping the LAST occurrence of
/// each key. Ported from cc-switch: older section-append tooling could leave
/// several copies of a top-level key behind, which serde_yaml rejects outright
/// and bricks the panel. Keep-last matches PyYAML's last-wins semantics, i.e.
/// what Hermes actually runs with. No-op when there are no duplicates.
fn deduplicate_top_level_keys(raw: &str) -> String {
    use std::collections::HashMap;

    // Pass 1: locate every top-level key line as (key, byte offset).
    let mut sections: Vec<(&str, usize)> = Vec::new();
    let mut offset = 0;
    for line in raw.split('\n') {
        if is_top_level_key_line(line) {
            if let Some(colon_pos) = line.find(':') {
                sections.push((&line[..colon_pos], offset));
            }
        }
        offset += line.len() + 1;
    }

    let mut remaining: HashMap<&str, usize> = HashMap::new();
    for (key, _) in &sections {
        *remaining.entry(key).or_insert(0) += 1;
    }
    if remaining.values().all(|&count| count <= 1) {
        return raw.to_string();
    }

    // Pass 2: re-emit, dropping every section that has a later occurrence of
    // the same key. A section spans its key line to the next top-level key
    // (or EOF). Content before the first section (comments, `---`) is kept.
    let mut result = String::with_capacity(raw.len());
    let head_end = sections
        .first()
        .map(|&(_, start)| start)
        .unwrap_or(raw.len());
    result.push_str(&raw[..head_end]);

    for (i, &(key, start)) in sections.iter().enumerate() {
        let end = sections
            .get(i + 1)
            .map(|&(_, next_start)| next_start)
            .unwrap_or(raw.len());
        let count = remaining.get_mut(key).expect("key collected in pass 1");
        *count -= 1;
        if *count > 0 {
            log::warn!(
                "Hermes config: dropped duplicate top-level section '{key}' (keeping the last occurrence)"
            );
            continue;
        }
        result.push_str(&raw[start..end]);
    }

    result
}

fn read_yaml_object_or_empty(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    // Heal duplicate top-level keys (e.g. from a config previously written by
    // older section-append tooling) before parsing; serde_yaml rejects them.
    let healed = deduplicate_top_level_keys(&content);
    let yaml: serde_yaml::Value = serde_yaml::from_str(&healed)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
    let parsed: Value = serde_json::to_value(yaml)
        .map_err(|error| format!("Failed to convert {}: {error}", path.display()))?;
    if parsed.is_object() {
        Ok(parsed)
    } else {
        Err(format!("{} must contain a YAML mapping", path.display()))
    }
}

/// Write bytes atomically via a same-directory temp file + rename, so a crash
/// mid-write never leaves a truncated `config.yaml`.
fn atomic_write_bytes(path: &Path, content: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }
    let temp_path = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    fs::write(&temp_path, content)
        .map_err(|error| format!("Failed to write temp file {}: {error}", temp_path.display()))?;
    fs::rename(&temp_path, path)
        .map_err(|error| format!("Failed to rename temp file to {}: {error}", path.display()))?;
    Ok(())
}

/// Convert a JSON value to a `serde_yaml` value (used to serialize a section).
fn json_value_to_yaml(value: &Value) -> Result<serde_yaml::Value, String> {
    let json_str = serde_json::to_string(value)
        .map_err(|error| format!("Failed to serialize JSON: {error}"))?;
    serde_yaml::from_str(&json_str)
        .map_err(|error| format!("Failed to convert JSON to YAML: {error}"))
}

/// Serialize a top-level section `key:` + value into a YAML fragment like:
///
/// ```yaml
/// model:
///   default: claude-opus-4-8
/// ```
fn serialize_yaml_section(key: &str, value: &Value) -> Result<String, String> {
    let mut mapping = serde_yaml::Mapping::new();
    mapping.insert(
        serde_yaml::Value::String(key.to_string()),
        json_value_to_yaml(value)?,
    );
    serde_yaml::to_string(&serde_yaml::Value::Mapping(mapping))
        .map_err(|error| format!("Failed to serialize YAML section '{key}': {error}"))
}

/// Find the byte range `(start_inclusive, end_exclusive)` of a top-level YAML
/// section (a mapping key at column 0). Mirrors cc-switch `find_yaml_section_range`.
fn find_yaml_section_range(raw: &str, section_key: &str) -> Option<(usize, usize)> {
    let target = format!("{section_key}:");
    let mut section_start = None;
    let mut offset = 0;
    for line in raw.split('\n') {
        if section_start.is_none() && is_top_level_key_line(line) && line.starts_with(&target) {
            // Verify exact match: after "key:" must be whitespace or EOL (\r for
            // CRLF files split on \n).
            let after_target = &line[target.len()..];
            if after_target.is_empty() || after_target.starts_with([' ', '\t', '\r']) {
                section_start = Some(offset);
            }
        } else if section_start.is_some() && is_top_level_key_line(line) {
            // Found the next top-level key — this is the end of our section.
            return Some((section_start.unwrap(), offset));
        }
        offset += line.len() + 1; // +1 for the \n
    }
    section_start.map(|start| (start, raw.len()))
}

/// Remove every top-level section with `section_key` from `raw`. Splices out
/// the duplicate copies an older append bug could leave behind.
fn remove_all_sections(raw: &str, section_key: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some((start, end)) = find_yaml_section_range(rest, section_key) {
        result.push_str(&rest[..start]);
        rest = &rest[end..];
    }
    result.push_str(rest);
    result
}

/// Replace a top-level YAML section in `raw`, or append it when absent.
///
/// Mirrors cc-switch `replace_yaml_section`: only the target section is touched
/// (byte-for-byte), so comments and unrelated sections elsewhere in the file
/// survive. When the section exists, any stale duplicate copies of the same key
/// after it are dropped (the healed read already picked the last one).
fn replace_yaml_section(raw: &str, section_key: &str, value: &Value) -> Result<String, String> {
    let serialized = serialize_yaml_section(section_key, value)?;

    if let Some((start, end)) = find_yaml_section_range(raw, section_key) {
        let mut result = String::with_capacity(raw.len());
        result.push_str(&raw[..start]);
        result.push_str(&serialized);
        // Drop duplicate copies of this key from the remainder.
        let remainder = remove_all_sections(&raw[end..], section_key);
        if !serialized.ends_with('\n') && !remainder.is_empty() && !remainder.starts_with('\n') {
            result.push('\n');
        }
        result.push_str(&remainder);
        Ok(result)
    } else {
        // Section not found — append at end.
        let mut result = raw.to_string();
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str(&serialized);
        if !result.ends_with('\n') {
            result.push('\n');
        }
        Ok(result)
    }
}

/// Write several top-level YAML sections to `config.yaml` via text-level
/// section replacement, preserving comments + unrelated sections (matches
/// cc-switch). Backs up the pre-write raw once when anything changed, and
/// skips the write entirely when the result is identical to the current file.
fn write_yaml_sections_with_backup<R: Runtime>(
    app: &tauri::AppHandle<R>,
    config_path: &Path,
    sections: &[(&str, Value)],
) -> Result<(), String> {
    let raw = if config_path.exists() {
        fs::read_to_string(config_path)
            .map_err(|error| format!("Failed to read {}: {error}", config_path.display()))?
    } else {
        String::new()
    };

    let mut result = raw.clone();
    let mut changed = false;
    for (key, value) in sections {
        let next = replace_yaml_section(&result, key, value)?;
        if next != result {
            result = next;
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }

    backup_hermes_config(app, config_path)?;
    atomic_write_bytes(config_path, result.as_bytes())
}

fn object_mut(value: &mut Value) -> Result<&mut Map<String, Value>, String> {
    value
        .as_object_mut()
        .ok_or_else(|| "Expected a YAML mapping object".to_string())
}

/// Serialize hermes `config.yaml` read-modify-write cycles so concurrent
/// tray/UI/MCP saves can't lose updates to each other (TOCTOU guard, mirrors
/// cc-switch `hermes_write_lock`). Shared with `mcp::hermes_mcp`.
pub(crate) fn hermes_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// How many write-time `config.yaml` backups to keep (oldest pruned).
const HERMES_CONFIG_BACKUP_RETAIN: usize = 10;

/// Snapshot the current `config.yaml` before it gets overwritten, so a bad
/// edit is recoverable. Backups live under the app data dir; the newest
/// `HERMES_CONFIG_BACKUP_RETAIN` copies are kept. No-op when the file is
/// missing or blank.
fn backup_hermes_config<R: Runtime>(
    app: &tauri::AppHandle<R>,
    config_path: &Path,
) -> Result<(), String> {
    if !config_path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(config_path)
        .map_err(|error| format!("Failed to read {}: {error}", config_path.display()))?;
    if raw.trim().is_empty() {
        return Ok(());
    }

    let backup_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Failed to resolve app data dir: {error}"))?
        .join("backups")
        .join("hermes");
    fs::create_dir_all(&backup_dir).map_err(|error| {
        format!(
            "Failed to create backup dir {}: {error}",
            backup_dir.display()
        )
    })?;

    let stamp = Local::now().format("%Y%m%d_%H%M%S");
    let base = format!("config_{stamp}");
    let mut backup_path = backup_dir.join(format!("{base}.yaml"));
    let mut counter = 1;
    while backup_path.exists() {
        backup_path = backup_dir.join(format!("{base}_{counter}.yaml"));
        counter += 1;
    }
    atomic_write_bytes(&backup_path, raw.as_bytes())?;
    prune_hermes_config_backups(&backup_dir)?;
    Ok(())
}

fn prune_hermes_config_backups(dir: &Path) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|error| format!("Failed to read backup dir {}: {error}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .map(|ext| ext == "yaml" || ext == "yml")
                .unwrap_or(false)
        })
        .map(|entry| entry.path())
        .collect();
    if entries.len() <= HERMES_CONFIG_BACKUP_RETAIN {
        return Ok(());
    }
    entries.sort_by_key(|path| {
        path.metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
    });
    let remove_count = entries.len() - HERMES_CONFIG_BACKUP_RETAIN;
    for stale in entries.iter().take(remove_count) {
        if let Err(error) = fs::remove_file(stale) {
            log::warn!(
                "Failed to remove old Hermes config backup {}: {error}",
                stale.display()
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// YAML <-> JSON provider helpers
// ---------------------------------------------------------------------------

/// Convert a provider's `models` dict to a UI-friendly ordered array
/// (re-inject `id` as a field).
fn models_dict_to_array(dict: Map<String, Value>) -> Value {
    let mut out = Vec::with_capacity(dict.len());
    for (id, value) in dict {
        let mut obj = match value {
            Value::Object(obj) => obj,
            Value::Null => Map::new(),
            other => {
                log::warn!(
                    "Hermes model entry for '{id}' has an unexpected shape: {other:?}; skipping"
                );
                continue;
            }
        };
        obj.insert("id".to_string(), Value::String(id));
        out.push(Value::Object(obj));
    }
    Value::Array(out)
}

/// Convert a provider's `models` array to Hermes' YAML dict shape, keyed by `id`.
/// Entries with missing/blank `id` are dropped; `id` is stripped from values.
fn models_array_to_dict(array: Vec<Value>) -> Value {
    let mut map = Map::new();
    for item in array {
        let Value::Object(mut obj) = item else {
            continue;
        };
        let Some(id) = obj
            .remove("id")
            .and_then(|v| v.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        map.insert(id, Value::Object(obj));
    }
    Value::Object(map)
}

/// Rewrite historical camelCase provider keys to Hermes' snake_case schema and
/// drop UI-only / legacy markers so they never reach YAML.
///
/// Mirrors cc-switch: `api` was a legacy DeepLink field that is neither a Hermes
/// field nor mappable to `api_mode`; `_cc_source` / `provider_key` are UI-only
/// markers injected on read. Unknown keys pass through untouched to keep
/// forward-compat with new Hermes fields (e.g. `request_timeout_seconds`).
fn sanitize_hermes_provider_keys(config: &mut Value) {
    const KEY_ALIASES: &[(&str, &str)] = &[
        ("baseUrl", "base_url"),
        ("apiKey", "api_key"),
        ("apiMode", "api_mode"),
        ("maxTokens", "max_tokens"),
        ("contextLength", "context_length"),
    ];
    const DROP_FIELDS: &[&str] = &["api", "_cc_source", "provider_key"];
    let Some(obj) = config.as_object_mut() else {
        return;
    };
    for (from, to) in KEY_ALIASES {
        if let Some(val) = obj.remove(*from) {
            obj.entry((*to).to_string()).or_insert(val);
        }
    }
    for field in DROP_FIELDS {
        obj.remove(*field);
    }
}

/// If `provider.models` is an array, convert it in-place to the dict shape.
fn normalize_provider_models_for_write(config: &mut Value) {
    let Some(obj) = config.as_object_mut() else {
        return;
    };
    let Some(models_val) = obj.get_mut("models") else {
        return;
    };
    if models_val.is_array() {
        let taken = std::mem::take(models_val);
        if let Value::Array(arr) = taken {
            *models_val = models_array_to_dict(arr);
        }
    }
}

/// If `provider.models` is a dict, convert it in-place to the array shape.
fn denormalize_provider_models_for_read(config: &mut Value) {
    let Some(obj) = config.as_object_mut() else {
        return;
    };
    let Some(models_val) = obj.get_mut("models") else {
        return;
    };
    if models_val.is_object() {
        let taken = std::mem::take(models_val);
        if let Value::Object(map) = taken {
            *models_val = models_dict_to_array(map);
        }
    }
}

// ---------------------------------------------------------------------------
// Model / other settings helpers
// ---------------------------------------------------------------------------

fn apply_string_field(object: &mut Map<String, Value>, key: &str, value: Option<String>) {
    match value {
        Some(value) if !value.trim().is_empty() => {
            object.insert(key.to_string(), json!(value.trim()));
        }
        Some(_) => {
            object.remove(key);
        }
        None => {}
    }
}

fn apply_number_field(object: &mut Map<String, Value>, key: &str, value: Option<u64>, clear: bool) {
    if clear {
        object.remove(key);
        return;
    }
    if let Some(value) = value {
        object.insert(key.to_string(), json!(value));
    }
}

fn model_settings_from_config(config: &Value) -> HermesModelSettingsInput {
    let model = config.get("model");
    HermesModelSettingsInput {
        default_model: model
            .and_then(|m| m.get("default"))
            .and_then(Value::as_str)
            .map(str::to_string),
        default_provider: model
            .and_then(|m| m.get("provider"))
            .and_then(Value::as_str)
            .map(str::to_string),
        base_url: model
            .and_then(|m| m.get("base_url"))
            .and_then(Value::as_str)
            .map(str::to_string),
        context_length: model
            .and_then(|m| m.get("context_length"))
            .and_then(Value::as_u64),
        max_tokens: model
            .and_then(|m| m.get("max_tokens"))
            .and_then(Value::as_u64),
        clear_context_length: false,
        clear_max_tokens: false,
    }
}

fn is_hermes_protected_key(key: &str) -> bool {
    HERMES_OTHER_SETTINGS_PROTECTED_KEYS.contains(&key)
}

fn build_other_settings(config: &Value) -> Value {
    let mut other = config.as_object().cloned().unwrap_or_default();
    for key in other.keys().cloned().collect::<Vec<_>>() {
        if is_hermes_protected_key(&key) {
            other.remove(&key);
        }
    }
    Value::Object(other)
}

// ---------------------------------------------------------------------------
// Provider views
// ---------------------------------------------------------------------------

fn get_custom_providers(object: &Map<String, Value>) -> Vec<(String, Value)> {
    object
        .get("custom_providers")
        .and_then(Value::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(|provider| {
                    provider
                        .get("name")
                        .and_then(Value::as_str)
                        .map(|name| (name.to_string(), provider.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn get_providers_dict(object: &Map<String, Value>) -> Vec<(String, Value)> {
    object
        .get("providers")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn model_ids_from_provider(provider: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(models) = provider.get("models").and_then(Value::as_object) {
        for key in models.keys() {
            ids.push(key.clone());
        }
    }
    if let Some(model) = provider.get("model").and_then(Value::as_str) {
        if !ids.iter().any(|id| id == model) {
            ids.push(model.to_string());
        }
    }
    ids
}

/// True when `name` lives in the read-only `providers:` dict but not in the
/// writable `custom_providers` list.
///
/// Matches cc-switch: a dict entry counts when its dict key OR its inner `name`
/// field equals `name`, and the entry is a mapping.
fn is_dict_only_provider(object: &Map<String, Value>, name: &str) -> bool {
    let list_has = object
        .get("custom_providers")
        .and_then(Value::as_array)
        .map(|array| {
            array
                .iter()
                .any(|p| p.get("name").and_then(Value::as_str) == Some(name))
        })
        .unwrap_or(false);
    if list_has {
        return false;
    }
    object
        .get("providers")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter().any(|(key, value)| {
                let key_matches = key.as_str() == name;
                let name_matches = value
                    .get("name")
                    .and_then(Value::as_str)
                    .map(|value_name| value_name == name)
                    .unwrap_or(false);
                (key_matches || name_matches) && value.is_object()
            })
        })
        .unwrap_or(false)
}

fn build_provider_views(config: &Value) -> Vec<HermesRuntimeProviderView> {
    let Some(object) = config.as_object() else {
        return Vec::new();
    };
    let default_provider = config
        .get("model")
        .and_then(|m| m.get("provider"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let default_model = config
        .get("model")
        .and_then(|m| m.get("default"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let custom = get_custom_providers(object);
    let dict = get_providers_dict(object);
    let custom_names: BTreeSet<String> = custom.iter().map(|(name, _)| name.clone()).collect();

    let mut keys = BTreeSet::new();
    for (name, _) in &custom {
        keys.insert(name.clone());
    }
    for (name, _) in &dict {
        keys.insert(name.clone());
    }
    if let Some(default_provider) = &default_provider {
        if !default_provider.trim().is_empty() {
            keys.insert(default_provider.clone());
        }
    }

    let mut views = Vec::new();
    for provider_key in keys {
        let is_read_only = !custom_names.contains(&provider_key)
            && dict.iter().any(|(name, _)| *name == provider_key);
        let is_default = default_provider.as_deref() == Some(provider_key.as_str());
        let is_builtin = is_builtin_provider(&provider_key);

        let mut raw = None;
        let mut from_custom_list = false;
        if let Some((_, value)) = custom.iter().find(|(name, _)| *name == provider_key) {
            raw = Some(value.clone());
            from_custom_list = true;
        } else if let Some((_, value)) = dict.iter().find(|(name, _)| *name == provider_key) {
            raw = Some(value.clone());
        }
        if let Some(value) = raw.as_mut() {
            // Heal legacy camelCase records (e.g. from older imports) before the
            // UI sees them, so editing doesn't reveal stale `baseUrl`/`apiKey`.
            if from_custom_list {
                sanitize_hermes_provider_keys(value);
            }
            denormalize_provider_models_for_read(value);
        }

        // Display name: prefer an explicit `display_name` field, then the
        // identity `name` (canonical for read-only `providers:` dict entries),
        // then the built-in table, then the provider key. This decouples the
        // friendly label from the list identity key (`name`), which custom
        // providers always store verbatim.
        let display_name = raw
            .as_ref()
            .and_then(|value| value.get("display_name").and_then(Value::as_str))
            .map(str::to_string)
            .or_else(|| {
                raw.as_ref()
                    .and_then(|value| value.get("name").and_then(Value::as_str))
                    .map(str::to_string)
            })
            .or_else(|| builtin_provider_name(&provider_key).map(str::to_string))
            .unwrap_or_else(|| provider_key.clone());

        let model_ids = raw
            .as_ref()
            .map(model_ids_from_provider)
            .unwrap_or_default();
        let credential = raw.as_ref().and_then(|value| value.get("api_key").cloned());
        let api_mode = raw
            .as_ref()
            .and_then(|value| value.get("api_mode").and_then(Value::as_str))
            .map(str::to_string);

        let mut warnings = Vec::new();
        if is_default {
            if raw.is_none() {
                if !is_builtin {
                    warnings.push(HermesProviderWarning::MissingProvider);
                }
            } else if let Some(default_model) = default_model.as_deref() {
                if !default_model.trim().is_empty()
                    && !model_ids.is_empty()
                    && !model_ids.iter().any(|id| id == default_model)
                {
                    warnings.push(HermesProviderWarning::MissingModel);
                }
            }
        }

        views.push(HermesRuntimeProviderView {
            provider_key,
            display_name,
            credential,
            api_mode,
            provider: raw,
            model_ids,
            is_builtin,
            is_read_only,
            is_default,
            warnings,
        });
    }

    views
}

fn builtin_providers() -> Vec<HermesBuiltinProvider> {
    super::constants::HERMES_BUILTIN_PROVIDERS
        .iter()
        .map(|(key, name)| HermesBuiltinProvider {
            key: (*key).to_string(),
            name: (*name).to_string(),
        })
        .collect()
}

fn emit_config_changed<R: Runtime>(app: &tauri::AppHandle<R>, payload: &str) {
    let _ = app.emit("config-changed", payload);
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-hermes", ());
}

// ---------------------------------------------------------------------------
// Root / path commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_hermes_default_config_dir() -> Result<String, String> {
    default_hermes_config_dir().map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_hermes_config_dir_without_db() -> Result<String, String> {
    resolve_hermes_config_dir_without_db().map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_hermes_root_path_info(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<HermesPathInfo, String> {
    get_hermes_root_path_info_from_db_async(state.db()).await
}

// ---------------------------------------------------------------------------
// Settings commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_hermes_settings_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Option<HermesSettingsConfig>, String> {
    Ok(state
        .db()
        .with_conn(|conn| db_get(conn, DbTable::HermesSettingsConfig, "common"))?
        .map(adapter::settings_from_db_value))
}

#[tauri::command]
pub async fn save_hermes_settings_config<R: Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle<R>,
    input: HermesSettingsConfigInput,
) -> Result<(), String> {
    let db = state.db();
    let existing = get_hermes_settings_config(state.clone()).await?;
    let config_dir = if input.clear_config_dir {
        None
    } else {
        input
            .config_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| existing.and_then(|value| value.config_dir))
    };
    let data = adapter::settings_to_db_value(config_dir.as_deref());
    db.with_conn(|conn| db_put(conn, DbTable::HermesSettingsConfig, "common", &data))?;
    runtime_location::refresh_runtime_location_cache_for_module_async(db, "hermes").await?;
    let _ = app.emit("wsl-config-changed", ());
    emit_config_changed(&app, "window");
    // The hermes skills dir is derived from the config root (<root>/skills);
    // changing the saved dir moves the runtime skills location, so ask the
    // skills pipeline to re-resolve targets for the hermes tool.
    let _ = app.emit("skills-changed", "window");
    Ok(())
}

// ---------------------------------------------------------------------------
// Runtime config
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn read_hermes_runtime_config(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<HermesRuntimeConfig, String> {
    let db = state.db();
    let root_path_info = get_hermes_root_path_info_from_db_async(&db).await?;
    let root_dir = PathBuf::from(&root_path_info.path);
    let config_path = get_hermes_config_path_from_root(&root_dir);
    let prompt_path = get_hermes_prompt_path_from_root(&root_dir);

    let config = read_yaml_object_or_empty(&config_path)?;

    Ok(HermesRuntimeConfig {
        root_path_info,
        config_path: config_path.to_string_lossy().to_string(),
        prompt_path: prompt_path.to_string_lossy().to_string(),
        model_settings: model_settings_from_config(&config),
        other_settings: build_other_settings(&config),
        providers: build_provider_views(&config),
        builtin_providers: builtin_providers(),
        config_content: fs::read_to_string(&config_path).ok(),
        prompt_content: fs::read_to_string(&prompt_path).ok(),
        config,
    })
}

// ---------------------------------------------------------------------------
// Provider commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn save_hermes_models_provider<R: tauri::Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle<R>,
    input: HermesModelsProviderInput,
) -> Result<HermesRuntimeConfig, String> {
    let provider_key = input.provider_key.trim();
    if provider_key.is_empty() {
        return Err("Provider name is required".to_string());
    }
    if !input.provider.is_object() {
        return Err("Hermes provider config must be an object".to_string());
    }

    let db = state.db();
    let config_path = get_hermes_config_path_async(&db).await?;
    let _guard = hermes_write_lock()
        .lock()
        .map_err(|_| "Hermes config write lock poisoned".to_string())?;
    let mut config = read_yaml_object_or_empty(&config_path)?;
    let object = object_mut(&mut config)?;

    if is_dict_only_provider(object, provider_key) {
        return Err(format!(
            "Provider '{provider_key}' is managed by Hermes' 'providers:' dict; edit it via Hermes UI"
        ));
    }

    let mut providers = object
        .get("custom_providers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut payload = input.provider;
    sanitize_hermes_provider_keys(&mut payload);
    normalize_provider_models_for_write(&mut payload);
    let first_model_id = payload
        .get("models")
        .and_then(Value::as_object)
        .and_then(|models| models.keys().next())
        .cloned();
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("name".to_string(), json!(provider_key));
        if let Some(model_id) = first_model_id {
            obj.insert("model".to_string(), json!(model_id));
        } else {
            obj.remove("model");
        }
    }

    if let Some(existing) = providers
        .iter_mut()
        .find(|provider| provider.get("name").and_then(Value::as_str) == Some(provider_key))
    {
        // Carry over on-disk fields the UI payload didn't include (e.g.
        // user-set `request_timeout_seconds` / `key_env`).
        if let (Some(existing_obj), Some(payload_obj)) =
            (existing.as_object(), payload.as_object_mut())
        {
            for (key, value) in existing_obj {
                if !payload_obj.contains_key(key) {
                    payload_obj.insert(key.clone(), value.clone());
                }
            }
        }
        *existing = payload;
    } else {
        providers.push(payload);
    }

    write_yaml_sections_with_backup(
        &app,
        &config_path,
        &[("custom_providers", Value::Array(providers))],
    )?;
    drop(_guard);
    emit_config_changed(&app, "window");
    read_hermes_runtime_config(state).await
}

#[tauri::command]
pub async fn delete_hermes_runtime_provider(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_key: String,
) -> Result<HermesRuntimeConfig, String> {
    let provider_key = provider_key.trim();
    if provider_key.is_empty() {
        return Err("Provider name is required".to_string());
    }
    let db = state.db();
    let config_path = get_hermes_config_path_async(&db).await?;
    let _guard = hermes_write_lock()
        .lock()
        .map_err(|_| "Hermes config write lock poisoned".to_string())?;
    let mut config = read_yaml_object_or_empty(&config_path)?;
    let object = object_mut(&mut config)?;

    if is_dict_only_provider(object, provider_key) {
        return Err(format!(
            "Provider '{provider_key}' is managed by Hermes' 'providers:' dict; edit it via Hermes UI"
        ));
    }

    let mut providers = object
        .get("custom_providers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let original_len = providers.len();
    providers.retain(|provider| provider.get("name").and_then(Value::as_str) != Some(provider_key));
    if providers.len() == original_len {
        // Nothing matched — leave the file untouched.
        drop(_guard);
        return read_hermes_runtime_config(state).await;
    }

    write_yaml_sections_with_backup(
        &app,
        &config_path,
        &[("custom_providers", Value::Array(providers))],
    )?;
    drop(_guard);
    emit_config_changed(&app, "window");
    read_hermes_runtime_config(state).await
}

// ---------------------------------------------------------------------------
// Model settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn save_hermes_model_settings(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: HermesModelSettingsInput,
) -> Result<HermesRuntimeConfig, String> {
    let db = state.db();
    let config_path = get_hermes_config_path_async(&db).await?;
    let _guard = hermes_write_lock()
        .lock()
        .map_err(|_| "Hermes config write lock poisoned".to_string())?;
    let mut config = read_yaml_object_or_empty(&config_path)?;
    let object = object_mut(&mut config)?;

    let mut model_section = object
        .get("model")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let model_object = object_mut(&mut model_section)?;
    apply_string_field(model_object, "default", input.default_model);
    apply_string_field(model_object, "provider", input.default_provider);
    apply_string_field(model_object, "base_url", input.base_url);
    apply_number_field(
        model_object,
        "context_length",
        input.context_length,
        input.clear_context_length,
    );
    apply_number_field(
        model_object,
        "max_tokens",
        input.max_tokens,
        input.clear_max_tokens,
    );
    write_yaml_sections_with_backup(&app, &config_path, &[("model", model_section)])?;
    drop(_guard);
    emit_config_changed(&app, "window");
    read_hermes_runtime_config(state).await
}

pub async fn apply_hermes_model_internal<R: Runtime>(
    db: &SqliteDbState,
    app: &tauri::AppHandle<R>,
    provider_key: &str,
    model_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    let provider_key = provider_key.trim();
    let model_id = model_id.trim();
    if provider_key.is_empty() {
        return Err("Provider name is required".to_string());
    }
    if model_id.is_empty() {
        return Err("Model id is required".to_string());
    }

    let config_path = get_hermes_config_path_async(db).await?;
    let _guard = hermes_write_lock()
        .lock()
        .map_err(|_| "Hermes config write lock poisoned".to_string())?;
    let mut config = read_yaml_object_or_empty(&config_path)?;
    let object = object_mut(&mut config)?;

    let mut model_section = object
        .get("model")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let model_object = object_mut(&mut model_section)?;
    model_object.insert("provider".to_string(), json!(provider_key));
    model_object.insert("default".to_string(), json!(model_id));
    write_yaml_sections_with_backup(app, &config_path, &[("model", model_section)])?;
    drop(_guard);
    emit_config_changed(app, if from_tray { "tray" } else { "window" });
    if let Err(error) =
        crate::settings::provider_list_state::record_provider_last_used_in_sqlite_state(
            db,
            "hermes",
            provider_key,
        )
    {
        log::warn!("Failed to record provider last-used for hermes:{provider_key}: {error}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Memory files (memories/MEMORY.md, memories/USER.md)
// ---------------------------------------------------------------------------
//
// Hermes persists two memory blobs under the config dir: `memories/MEMORY.md`
// (agent notes) and `memories/USER.md` (user profile), both snapshotted into
// the system prompt. Hermes' own Web UI only exposes on/off toggles and
// character budgets, so the content editor lives here. Budgets + toggles are
// stored in the top-level `memory:` section of config.yaml; Hermes truncates
// over-budget content at load time.

impl HermesMemoryKind {
    fn filename(self) -> &'static str {
        match self {
            Self::Memory => "MEMORY.md",
            Self::User => "USER.md",
        }
    }

    /// On-disk enable flag key inside the `memory:` section. The user-profile
    /// toggle is `user_profile_enabled` (not `user_enabled`).
    fn enable_key(self) -> &'static str {
        match self {
            Self::Memory => "memory_enabled",
            Self::User => "user_profile_enabled",
        }
    }
}

impl Default for HermesMemoryLimits {
    fn default() -> Self {
        Self {
            memory: 2200,
            user: 1375,
            memory_enabled: true,
            user_enabled: true,
        }
    }
}

fn memory_limits_from_config(config: &Value) -> HermesMemoryLimits {
    let mut out = HermesMemoryLimits::default();
    let Some(memory) = config.get("memory").and_then(Value::as_object) else {
        return out;
    };
    if let Some(v) = memory.get("memory_char_limit").and_then(Value::as_u64) {
        out.memory = v as usize;
    }
    if let Some(v) = memory.get("user_char_limit").and_then(Value::as_u64) {
        out.user = v as usize;
    }
    if let Some(v) = memory.get("memory_enabled").and_then(Value::as_bool) {
        out.memory_enabled = v;
    }
    if let Some(v) = memory.get("user_profile_enabled").and_then(Value::as_bool) {
        out.user_enabled = v;
    }
    out
}

async fn hermes_memory_path_async(
    db: &SqliteDbState,
    kind: HermesMemoryKind,
) -> Result<PathBuf, String> {
    Ok(get_hermes_root_dir_from_db_async(db)
        .await?
        .join("memories")
        .join(kind.filename()))
}

/// Read a Hermes memory file as a markdown blob; missing file becomes `""`.
#[tauri::command]
pub async fn get_hermes_memory(
    state: tauri::State<'_, SqliteDbState>,
    kind: HermesMemoryKind,
) -> Result<String, String> {
    let path = hermes_memory_path_async(state.db(), kind).await?;
    match fs::read_to_string(&path) {
        Ok(content) => Ok(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("Failed to read {}: {error}", path.display())),
    }
}

/// Atomically replace a Hermes memory file (creates `memories/` as needed).
#[tauri::command]
pub async fn set_hermes_memory(
    state: tauri::State<'_, SqliteDbState>,
    kind: HermesMemoryKind,
    content: String,
) -> Result<(), String> {
    let path = hermes_memory_path_async(state.db(), kind).await?;
    atomic_write_bytes(&path, content.as_bytes())
}

/// Read memory budgets + toggles from the `memory:` section of config.yaml.
#[tauri::command]
pub async fn get_hermes_memory_limits(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<HermesMemoryLimits, String> {
    let config_path = get_hermes_config_path_async(state.db()).await?;
    let config = read_yaml_object_or_empty(&config_path)?;
    Ok(memory_limits_from_config(&config))
}

/// Toggle the on/off flag for one memory blob, preserving every other field in
/// the `memory:` section (budgets, external provider settings, ...).
#[tauri::command]
pub async fn set_hermes_memory_enabled(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    kind: HermesMemoryKind,
    enabled: bool,
) -> Result<HermesMemoryLimits, String> {
    let db = state.db();
    let config_path = get_hermes_config_path_async(&db).await?;
    let limits = {
        let _guard = hermes_write_lock()
            .lock()
            .map_err(|_| "Hermes config write lock poisoned".to_string())?;
        let mut config = read_yaml_object_or_empty(&config_path)?;
        let memory_val = {
            let object = object_mut(&mut config)?;
            let memory_section = object
                .entry("memory".to_string())
                .or_insert_with(|| json!({}));
            object_mut(memory_section)?.insert(kind.enable_key().to_string(), json!(enabled));
            object.get("memory").cloned().unwrap_or_else(|| json!({}))
        };
        let limits = memory_limits_from_config(&config);
        write_yaml_sections_with_backup(&app, &config_path, &[("memory", memory_val)])?;
        limits
    };
    emit_config_changed(&app, "window");
    Ok(limits)
}

// ---------------------------------------------------------------------------
// Other settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn save_hermes_other_settings(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    other_settings: Value,
) -> Result<HermesRuntimeConfig, String> {
    if !other_settings.is_object() {
        return Err("Hermes other settings must be an object".to_string());
    }
    let db = state.db();
    let config_path = get_hermes_config_path_async(&db).await?;
    let _guard = hermes_write_lock()
        .lock()
        .map_err(|_| "Hermes config write lock poisoned".to_string())?;
    // Upsert only the non-protected keys the editor submitted, each as its own
    // text-level section replacement. Top-level keys absent from `other_settings`
    // (e.g. a newer Hermes key a stale frontend didn't know about, or one set by
    // a concurrent write) are preserved as-is — a full replace would silently
    // drop those keys and their comments.
    let sections: Vec<(&str, Value)> = other_settings
        .as_object()
        .unwrap()
        .iter()
        .filter(|(key, _)| !is_hermes_protected_key(key))
        .map(|(key, value)| (key.as_str(), value.clone()))
        .collect();
    write_yaml_sections_with_backup(&app, &config_path, &sections)?;
    drop(_guard);
    emit_config_changed(&app, "window");
    read_hermes_runtime_config(state).await
}

// ---------------------------------------------------------------------------
// Prompt CRUD
// ---------------------------------------------------------------------------

fn prompt_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![OrderField::json_integer(
        "sort_index",
        OrderDirection::Asc,
    )?]))
}

fn put_hermes_prompt_to_sqlite(
    db: &SqliteDbState,
    id: &str,
    content: &HermesPromptConfigContent,
) -> Result<(), String> {
    let value = adapter::prompt_to_db_value(content);
    db.with_conn(|conn| db_put(conn, DbTable::HermesPromptConfig, id, &value))
}

fn get_hermes_prompt_from_sqlite(
    db: &SqliteDbState,
    id: &str,
) -> Result<Option<HermesPromptConfig>, String> {
    Ok(db
        .with_conn(|conn| db_get(conn, DbTable::HermesPromptConfig, id))?
        .map(adapter::prompt_from_db_value))
}

async fn get_local_prompt_config(db: &SqliteDbState) -> Result<Option<HermesPromptConfig>, String> {
    let prompt_path = get_hermes_prompt_path_async(db).await?;
    if !prompt_path.exists() {
        return Ok(None);
    }
    let Some(content) = read_prompt_content_file(&prompt_path, "Hermes")? else {
        return Ok(None);
    };
    Ok(Some(HermesPromptConfig {
        id: "__local__".to_string(),
        name: "Local SOUL.md".to_string(),
        content,
        is_applied: false,
        sort_index: Some(-1),
        created_at: None,
        updated_at: None,
    }))
}

async fn write_prompt_content_to_file(
    db: &SqliteDbState,
    content: Option<&str>,
) -> Result<(), String> {
    let path = get_hermes_prompt_path_async(db).await?;
    write_prompt_content_file(&path, content, "Hermes")
}

#[tauri::command]
pub async fn list_hermes_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<HermesPromptConfig>, String> {
    let db = state.db();
    let mut prompts = db.with_conn(|conn| {
        Ok(
            db_list(conn, DbTable::HermesPromptConfig, Some(&prompt_order()?))?
                .into_iter()
                .map(adapter::prompt_from_db_value)
                .collect::<Vec<_>>(),
        )
    })?;
    if !prompts.iter().any(|prompt| prompt.is_applied) {
        if let Some(local_prompt) = get_local_prompt_config(&db).await? {
            prompts.insert(0, local_prompt);
        }
    }
    Ok(prompts)
}

#[tauri::command]
pub async fn create_hermes_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: HermesPromptConfigInput,
) -> Result<HermesPromptConfig, String> {
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let next_sort_index = db.with_conn(|conn| {
        Ok(db_max_i64(
            conn,
            DbTable::HermesPromptConfig,
            &JsonFieldPath::new("sort_index")?,
        )?
        .map(|value| value as i32 + 1)
        .unwrap_or(0))
    })?;
    let content = HermesPromptConfigContent {
        name: input.name,
        content: input.content,
        is_applied: false,
        sort_index: Some(next_sort_index),
        created_at: now.clone(),
        updated_at: now,
    };
    let prompt_id = db_new_id();
    put_hermes_prompt_to_sqlite(&db, &prompt_id, &content)?;
    let _ = app.emit("config-changed", "window");
    Ok(adapter::prompt_from_db_value(json!({
        "id": prompt_id,
        "name": content.name,
        "content": content.content,
        "is_applied": content.is_applied,
        "sort_index": content.sort_index,
        "created_at": content.created_at,
        "updated_at": content.updated_at
    })))
}

#[tauri::command]
pub async fn update_hermes_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: HermesPromptConfigInput,
) -> Result<HermesPromptConfig, String> {
    let config_id = input
        .id
        .ok_or_else(|| "ID is required for update".to_string())?;
    let db = state.db();
    let now = Local::now().to_rfc3339();
    let existing = get_hermes_prompt_from_sqlite(&db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{config_id}' not found"))?;
    let content = HermesPromptConfigContent {
        name: input.name,
        content: input.content.clone(),
        is_applied: existing.is_applied,
        sort_index: existing.sort_index,
        created_at: existing.created_at.unwrap_or_else(|| now.clone()),
        updated_at: now.clone(),
    };
    put_hermes_prompt_to_sqlite(&db, &config_id, &content)?;
    if existing.is_applied {
        write_prompt_content_to_file(&db, Some(input.content.as_str())).await?;
        emit_config_changed(&app, "window");
    } else {
        let _ = app.emit("config-changed", "window");
    }
    get_hermes_prompt_from_sqlite(&db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{config_id}' not found after update"))
}

#[tauri::command]
pub async fn delete_hermes_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    // Only delete the DB prompt record. The live SOUL.md on disk is kept so
    // deleting a saved prompt never wipes the local runtime prompt.
    let db = state.db();
    db.with_conn(|conn| db_delete(conn, DbTable::HermesPromptConfig, &id).map(|_| ()))?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

pub async fn apply_hermes_prompt_config_internal<R: Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    apply_hermes_prompt_config_internal_with_events(state, app, config_id, from_tray, true).await
}

pub async fn apply_hermes_prompt_config_internal_without_events<R: Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
) -> Result<(), String> {
    apply_hermes_prompt_config_internal_with_events(state, app, config_id, false, false).await
}

async fn apply_hermes_prompt_config_internal_with_events<R: Runtime>(
    state: tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle<R>,
    config_id: &str,
    from_tray: bool,
    emit_events: bool,
) -> Result<(), String> {
    let db = state.db();
    if config_id == "__local__" {
        let local_prompt = get_local_prompt_config(&db)
            .await?
            .ok_or_else(|| "Local Hermes prompt not found".to_string())?;
        write_prompt_content_to_file(&db, Some(local_prompt.content.as_str())).await?;
        if emit_events {
            emit_config_changed(app, if from_tray { "tray" } else { "window" });
        }
        return Ok(());
    }

    let prompt = get_hermes_prompt_from_sqlite(&db, config_id)?
        .ok_or_else(|| format!("Prompt config '{config_id}' not found"))?;
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::HermesPromptConfig, Some(config_id), &now)
    })?;
    write_prompt_content_to_file(&db, Some(prompt.content.as_str())).await?;
    if emit_events {
        emit_config_changed(app, if from_tray { "tray" } else { "window" });
    }
    Ok(())
}

#[tauri::command]
pub async fn apply_hermes_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    apply_hermes_prompt_config_internal(state, &app, &config_id, false).await
}

/// Disable the applied Hermes prompt: clear every applied flag and empty the
/// live `SOUL.md`, while keeping the DB record so it can be re-applied later.
#[tauri::command]
pub async fn disable_hermes_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    config_id: String,
) -> Result<(), String> {
    let db = state.db();
    get_hermes_prompt_from_sqlite(&db, &config_id)?
        .ok_or_else(|| format!("Prompt config '{config_id}' not found"))?;
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::HermesPromptConfig, None, &now)
    })?;
    write_prompt_content_to_file(&db, Some("")).await?;
    emit_config_changed(&app, "window");
    Ok(())
}

#[tauri::command]
pub async fn reorder_hermes_prompt_configs(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    for (index, id) in ids.iter().enumerate() {
        db.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::HermesPromptConfig,
                id,
                &[("sort_index", json!(index as i64))],
            )
            .map(|_| ())
        })?;
    }
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn save_hermes_local_prompt_config(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: HermesPromptConfigInput,
) -> Result<HermesPromptConfig, String> {
    let db = state.db();
    let content = if input.content.trim().is_empty() {
        get_local_prompt_config(&db)
            .await?
            .map(|prompt| prompt.content)
            .unwrap_or_default()
    } else {
        input.content
    };
    let created = create_hermes_prompt_config(
        state.clone(),
        app.clone(),
        HermesPromptConfigInput {
            id: None,
            name: input.name,
            content,
        },
    )
    .await?;
    apply_hermes_prompt_config_internal(state.clone(), &app, &created.id, false).await?;
    Ok(get_hermes_prompt_from_sqlite(state.db(), &created.id)?.unwrap_or(created))
}

// ============================================================================
// Hermes Web UI
// ============================================================================

/// 打开 Hermes Web UI:探测本地服务在线后,用系统浏览器打开。
/// 服务离线返回 `Err`,前端据此引导用户启动 dashboard。
#[tauri::command]
pub async fn open_hermes_web_ui(path: Option<String>) -> Result<(), String> {
    use super::web_ui;

    let port = web_ui::resolve_web_port();
    if !web_ui::probe_web_up(port).await {
        return Err("Hermes Web UI 未运行,请先启动 Hermes dashboard".to_string());
    }
    web_ui::open_web_ui_browser(port, path.as_deref())
}

/// 在用户终端里非阻塞启动 `hermes dashboard`(Hermes 的 web dashboard 进程)。
#[tauri::command]
pub async fn launch_hermes_dashboard() -> Result<(), String> {
    use super::web_ui;

    web_ui::launch_dashboard_in_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_keeps_last_toplevel_occurrence() {
        // Duplicates come from older section-append tooling; the last block is
        // the newest data and must win (PyYAML last-wins).
        let yaml = "\
model:
  default: gpt-4
agent:
  max_turns: 10
model:
  default: claude-opus-4-8
";
        let result = deduplicate_top_level_keys(yaml);
        assert_eq!(result.lines().filter(|line| *line == "model:").count(), 1);
        assert!(result.contains("claude-opus-4-8"));
        assert!(!result.contains("gpt-4"));
        assert!(result.contains("max_turns"));
    }

    #[test]
    fn dedup_is_identity_without_duplicates() {
        let yaml = "\
# Hermes config
model:
  default: gpt-4

agent:
  max_turns: 10
";
        assert_eq!(deduplicate_top_level_keys(yaml), yaml);
    }

    #[test]
    fn dedup_parenthesized_custom_providers_sequence_is_not_a_key() {
        // A `custom_providers:` sequence rebuild must not be mis-read as dupes.
        let yaml = "\
custom_providers:
  - name: openrouter
model:
  default: gpt-4
";
        assert_eq!(deduplicate_top_level_keys(yaml), yaml);
    }

    #[test]
    fn models_dict_to_array_reinjects_id() {
        let mut map = Map::new();
        map.insert("alpha".to_string(), json!({ "context_length": 10 }));
        map.insert("beta".to_string(), Value::Null);
        let arr = models_dict_to_array(map);
        let list = arr.as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["id"], "alpha");
        assert_eq!(list[0]["context_length"], 10);
        assert_eq!(list[1]["id"], "beta");
    }

    #[test]
    fn models_array_to_dict_strips_id_and_blank_ids() {
        let arr = vec![
            json!({ "id": "foo", "context_length": 1 }),
            json!({ "id": "   ", "context_length": 2 }),
            json!({ "context_length": 3 }),
        ];
        let dict = models_array_to_dict(arr);
        let obj = dict.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        assert!(obj.contains_key("foo"));
        assert!(!obj["foo"].as_object().unwrap().contains_key("id"));
    }

    #[test]
    fn sanitize_rewrites_camel_case_aliases() {
        let mut v = json!({
            "name": "test",
            "baseUrl": "https://api.example.com",
            "apiKey": "sk-123",
            "apiMode": "chat_completions",
            "provider_key": "x"
        });
        sanitize_hermes_provider_keys(&mut v);
        let obj = v.as_object().unwrap();
        assert_eq!(obj.get("base_url").unwrap(), "https://api.example.com");
        assert_eq!(obj.get("api_key").unwrap(), "sk-123");
        assert_eq!(obj.get("api_mode").unwrap(), "chat_completions");
        assert!(obj.get("baseUrl").is_none());
        assert!(obj.get("provider_key").is_none());
    }

    #[test]
    fn build_other_settings_hides_managed_keys() {
        let config = json!({
            "model": { "provider": "openrouter" },
            "custom_providers": [],
            "providers": { "anthropic": {} },
            "mcp_servers": {},
            "_config_version": 19,
            "agent": { "max_turns": 50 }
        });
        assert_eq!(
            build_other_settings(&config),
            json!({ "agent": { "max_turns": 50 } })
        );
    }

    #[test]
    fn sanitize_drops_legacy_api_and_ui_markers() {
        let mut v = json!({
            "base_url": "https://api.example.com",
            "api": "openai-completions",
            "_cc_source": "providers_dict",
            "provider_key": "anthropic",
            "request_timeout_seconds": 300,
        });
        sanitize_hermes_provider_keys(&mut v);
        let obj = v.as_object().unwrap();
        assert!(obj.get("api").is_none());
        assert!(obj.get("_cc_source").is_none());
        assert!(obj.get("provider_key").is_none());
        // Forward-compat unknown fields pass through untouched.
        assert_eq!(obj.get("request_timeout_seconds").unwrap(), 300);
    }

    #[test]
    fn section_replace_preserves_comments_and_unrelated_sections() {
        let raw = "\
# Hermes config
model:
  default: gpt-4   # user kept this comment

agent:
  max_turns: 10
";
        let new_providers = json!([{ "name": "openrouter", "api_key": "sk-or" }]);
        let result = replace_yaml_section(raw, "custom_providers", &new_providers).unwrap();
        assert!(result.contains("# Hermes config"));
        assert!(result.contains("gpt-4"));
        assert!(result.contains("# user kept this comment"));
        assert!(result.contains("max_turns: 10"));
        assert!(result.contains("custom_providers:"));
        // Result must be valid YAML again.
        assert!(serde_yaml::from_str::<serde_yaml::Value>(&result).is_ok());
    }

    #[test]
    fn section_replace_replaces_in_place_and_drops_residual_duplicates() {
        let raw = "\
model:
  default: gpt-4
agent:
  max_turns: 10
model:
  default: stale-copy
";
        let new_model = json!({ "default": "claude-opus-4-8" });
        let result = replace_yaml_section(raw, "model", &new_model).unwrap();
        assert_eq!(result.lines().filter(|line| *line == "model:").count(), 1);
        assert!(result.contains("claude-opus-4-8"));
        assert!(!result.contains("gpt-4"));
        assert!(!result.contains("stale-copy"));
        assert!(result.contains("max_turns"));
        assert!(serde_yaml::from_str::<serde_yaml::Value>(&result).is_ok());
    }

    #[test]
    fn section_replace_handles_crlf_without_duplicate_append() {
        let raw = "model:\r\n  default: gpt-4\r\nagent:\r\n  max_turns: 10\r\n";
        let new_model = json!({ "default": "claude" });
        let result = replace_yaml_section(raw, "model", &new_model).unwrap();
        assert_eq!(
            result
                .lines()
                .filter(|line| line.trim() == "model:")
                .count(),
            1
        );
        assert!(result.contains("claude"));
        assert!(result.contains("max_turns"));
    }

    #[test]
    fn section_replace_appends_new_section() {
        let raw = "model:\n  default: gpt-4\n";
        let result = replace_yaml_section(raw, "agent", &json!({ "max_turns": 50 })).unwrap();
        assert!(result.contains("agent:"));
        assert!(result.contains("max_turns: 50"));
        assert!(result.contains("gpt-4"));
    }

    #[test]
    fn section_replace_empty_raw_creates_section() {
        let result = replace_yaml_section("", "model", &json!({ "default": "gpt-4" })).unwrap();
        assert!(result.contains("model:"));
        assert!(result.contains("gpt-4"));
        assert!(result.ends_with('\n'));
    }

    #[test]
    fn write_yaml_sections_skips_unchanged_write() {
        // A no-op edit (identical section value) must produce identical raw.
        let raw = "model:\n  default: gpt-4\nagent:\n  max_turns: 10\n";
        let next = replace_yaml_section(raw, "model", &json!({ "default": "gpt-4" })).unwrap();
        assert_eq!(next, raw);
    }

    #[test]
    fn is_dict_only_provider_matches_key_or_inner_name() {
        let config = json!({
            "providers": {
                "weird-key": { "name": "deepseek", "base_url": "https://api.deepseek.com" }
            }
        });
        let object = config.as_object().unwrap();
        assert!(is_dict_only_provider(object, "weird-key"));
        assert!(is_dict_only_provider(object, "deepseek"));
        // Non-mapping dict entries don't count.
        assert!(!is_dict_only_provider(object, "missing"));
    }

    #[test]
    fn is_dict_only_provider_list_wins_on_collision() {
        let config = json!({
            "custom_providers": [ { "name": "deepseek", "api_key": "sk" } ],
            "providers": { "deepseek": { "name": "deepseek" } }
        });
        assert!(!is_dict_only_provider(
            config.as_object().unwrap(),
            "deepseek"
        ));
    }

    #[test]
    fn build_provider_views_prefers_display_name_over_key() {
        let config = json!({
            "custom_providers": [
                { "name": "deepseek-acct", "display_name": "DeepSeek", "base_url": "https://api.deepseek.com" }
            ]
        });
        let views = build_provider_views(&config);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].provider_key, "deepseek-acct");
        assert_eq!(views[0].display_name, "DeepSeek");
    }

    #[test]
    fn build_provider_views_falls_back_to_name_then_builtin_then_key() {
        // No display_name: falls back to the identity `name` field.
        let config = json!({
            "custom_providers": [ { "name": "my-provider", "base_url": "https://example.com" } ]
        });
        let views = build_provider_views(&config);
        assert_eq!(views[0].display_name, "my-provider");

        // No display_name and no custom name: builtin table wins for "anthropic".
        let builtin_config = json!({
            "providers": { "anthropic": { "base_url": "https://api.anthropic.com" } }
        });
        let builtin_views = build_provider_views(&builtin_config);
        let anthropic = builtin_views
            .iter()
            .find(|v| v.provider_key == "anthropic")
            .unwrap();
        assert_eq!(anthropic.display_name, "Anthropic");
    }

    #[test]
    fn model_settings_from_config_reads_section() {
        let config = json!({
            "model": {
                "default": "claude-opus-4-8",
                "provider": "openrouter",
                "base_url": "https://openrouter.ai/api/v1",
                "context_length": 200000
            }
        });
        let ms = model_settings_from_config(&config);
        assert_eq!(ms.default_model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(ms.default_provider.as_deref(), Some("openrouter"));
        assert_eq!(ms.context_length, Some(200000));
        assert_eq!(ms.max_tokens, None);
    }
}

// ============================================================================
// All API Hub import (Hermes)
// ============================================================================

#[tauri::command]
pub async fn list_hermes_all_api_hub_providers(
) -> Result<crate::coding::all_api_hub::AllApiHubProvidersResult, String> {
    let discovery = crate::coding::all_api_hub::list_provider_candidates()?;
    let providers = crate::coding::all_api_hub::build_all_api_hub_items(
        &discovery.providers,
        crate::coding::all_api_hub::candidate_to_hermes_provider,
    );
    Ok(crate::coding::all_api_hub::AllApiHubProvidersResult {
        found: discovery.found,
        profiles: discovery.profiles,
        providers,
        message: discovery.message,
    })
}

#[tauri::command]
pub async fn resolve_hermes_all_api_hub_providers(
    state: tauri::State<'_, crate::db::SqliteDbState>,
    request: crate::coding::all_api_hub::AllApiHubResolveRequest,
) -> Result<Vec<crate::coding::all_api_hub::AllApiHubProviderItem>, String> {
    let providers = crate::coding::all_api_hub::resolve_provider_candidates_with_keys(
        &state,
        &request.provider_ids,
    )
    .await?;
    Ok(crate::coding::all_api_hub::build_all_api_hub_items(
        &providers,
        crate::coding::all_api_hub::candidate_to_hermes_provider,
    ))
}
