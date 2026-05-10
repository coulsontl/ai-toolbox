use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::Manager;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::coding::open_code::shell_env;
use crate::coding::skills::central_repo::skill_storage_dir_name;
use crate::coding::{claude_code, codex, gemini_cli, runtime_location};
use crate::settings::types::{BackupCustomEntry, BackupCustomEntryType};

const CUSTOM_BACKUP_MANIFEST_PATH: &str = "custom-backup/manifest.json";
const CUSTOM_BACKUP_PAYLOAD_DIR: &str = "custom-backup/payload";

/// Get database directory path
pub fn get_db_path(app_handle: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    Ok(app_data_dir.join("database"))
}

/// Get home directory
fn get_home_dir() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .map_err(|_| "Failed to get home directory".to_string())
}

pub fn get_claude_restore_dir() -> Result<PathBuf, String> {
    claude_code::get_claude_root_dir_without_db()
}

pub fn get_codex_restore_dir() -> Result<PathBuf, String> {
    codex::get_codex_root_dir_without_db()
}

pub fn get_gemini_cli_restore_dir() -> Result<PathBuf, String> {
    gemini_cli::get_gemini_cli_root_dir_without_db()
}

/// Get OpenCode config file path using priority: system env > shell config > default
/// Note: This does NOT check database (common_config) because:
/// 1. For backup: the database common_config will be included in the backup
/// 2. For restore: the database doesn't exist yet, and will be restored from backup
pub fn get_opencode_config_path() -> Result<Option<PathBuf>, String> {
    // 1. Check system environment variable (highest priority for backup without DB)
    if let Ok(env_path) = std::env::var("OPENCODE_CONFIG") {
        if !env_path.is_empty() {
            let path = PathBuf::from(&env_path);
            if path.exists() {
                return Ok(Some(path));
            }
        }
    }

    // 2. Check shell configuration files
    if let Some(shell_path) = shell_env::get_env_from_shell_config("OPENCODE_CONFIG") {
        if !shell_path.is_empty() {
            let path = PathBuf::from(&shell_path);
            if path.exists() {
                return Ok(Some(path));
            }
        }
    }

    // 3. Check default paths
    let home_dir = get_home_dir()?;
    let config_dir = home_dir.join(".config").join("opencode");

    let json_path = config_dir.join("opencode.json");
    let jsonc_path = config_dir.join("opencode.jsonc");

    if json_path.exists() {
        Ok(Some(json_path))
    } else if jsonc_path.exists() {
        Ok(Some(jsonc_path))
    } else {
        Ok(None)
    }
}

pub async fn get_opencode_config_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let location = runtime_location::get_opencode_runtime_location_async(db).await?;
    Ok(location.host_path.exists().then_some(location.host_path))
}

/// Get the directory where OpenCode config should be restored to
/// Uses the same priority logic but returns directory path
pub fn get_opencode_restore_dir() -> Result<PathBuf, String> {
    // 1. Check system environment variable
    if let Ok(env_path) = std::env::var("OPENCODE_CONFIG") {
        if !env_path.is_empty() {
            let path = PathBuf::from(&env_path);
            if let Some(parent) = path.parent() {
                return Ok(parent.to_path_buf());
            }
        }
    }

    // 2. Check shell configuration files
    if let Some(shell_path) = shell_env::get_env_from_shell_config("OPENCODE_CONFIG") {
        if !shell_path.is_empty() {
            let path = PathBuf::from(&shell_path);
            if let Some(parent) = path.parent() {
                return Ok(parent.to_path_buf());
            }
        }
    }

    // 3. Return default directory
    let home_dir = get_home_dir()?;
    Ok(home_dir.join(".config").join("opencode"))
}

pub async fn get_opencode_restore_dir_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    runtime_location::get_opencode_config_dir_async(db).await
}

/// Get Claude settings.json path if it exists
pub fn get_claude_settings_path() -> Result<Option<PathBuf>, String> {
    let settings_path = claude_code::get_claude_root_dir_without_db()?.join("settings.json");

    if settings_path.exists() {
        Ok(Some(settings_path))
    } else {
        Ok(None)
    }
}

pub async fn get_claude_settings_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_claude_settings_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

/// Get Claude prompt file path if it exists
pub fn get_claude_prompt_path() -> Result<Option<PathBuf>, String> {
    let resolved_root_dir = claude_code::get_claude_root_dir_without_db()?;
    let prompt_path = resolved_root_dir.join("CLAUDE.md");

    if prompt_path.exists() {
        Ok(Some(prompt_path))
    } else {
        Ok(None)
    }
}

pub async fn get_claude_prompt_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_claude_prompt_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

pub async fn get_claude_mcp_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_claude_mcp_config_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

fn build_wsl_user_home_target(
    runtime_root_dir: &Path,
    home_relative_path: &str,
) -> Option<PathBuf> {
    let wsl = runtime_location::parse_wsl_unc_path(&runtime_root_dir.to_string_lossy())?;
    let linux_path = runtime_location::expand_home_from_user_root(
        wsl.linux_user_root.as_deref(),
        home_relative_path,
    );
    Some(runtime_location::build_windows_unc_path(
        &wsl.distro,
        &linux_path,
    ))
}

pub fn get_claude_mcp_restore_path(runtime_root_dir: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(runtime_root_dir) = runtime_root_dir {
        if let Some(path) = build_wsl_user_home_target(runtime_root_dir, "~/.claude.json") {
            return Ok(path);
        }
    }

    Ok(get_home_dir()?.join(".claude.json"))
}

/// Get OpenCode auth.json path if it exists
pub fn get_opencode_auth_path() -> Result<Option<PathBuf>, String> {
    let home_dir = get_home_dir()?;
    let auth_path = home_dir
        .join(".local")
        .join("share")
        .join("opencode")
        .join("auth.json");

    if auth_path.exists() {
        Ok(Some(auth_path))
    } else {
        Ok(None)
    }
}

pub async fn get_opencode_auth_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let location = runtime_location::get_opencode_runtime_location_async(db).await?;
    if let Some(wsl) = location.wsl {
        let auth_path = runtime_location::build_windows_unc_path(
            &wsl.distro,
            &runtime_location::expand_home_from_user_root(
                wsl.linux_user_root.as_deref(),
                "~/.local/share/opencode/auth.json",
            ),
        );
        Ok(auth_path.exists().then_some(auth_path))
    } else {
        get_opencode_auth_path()
    }
}

pub fn get_opencode_auth_restore_path(runtime_root_dir: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(runtime_root_dir) = runtime_root_dir {
        if let Some(path) =
            build_wsl_user_home_target(runtime_root_dir, "~/.local/share/opencode/auth.json")
        {
            return Ok(path);
        }
    }

    Ok(get_home_dir()?
        .join(".local")
        .join("share")
        .join("opencode")
        .join("auth.json"))
}

/// Get OpenCode prompt file path if it exists
pub fn get_opencode_prompt_path() -> Result<Option<PathBuf>, String> {
    if let Ok(env_path) = std::env::var("OPENCODE_CONFIG") {
        if !env_path.is_empty() {
            if let Some(prompt_path) = PathBuf::from(&env_path)
                .parent()
                .map(|path| path.join("AGENTS.md"))
                .filter(|path| path.exists())
            {
                return Ok(Some(prompt_path));
            }
        }
    }

    if let Some(shell_path) = shell_env::get_env_from_shell_config("OPENCODE_CONFIG") {
        if !shell_path.is_empty() {
            if let Some(prompt_path) = PathBuf::from(&shell_path)
                .parent()
                .map(|path| path.join("AGENTS.md"))
                .filter(|path| path.exists())
            {
                return Ok(Some(prompt_path));
            }
        }
    }

    let home_dir = get_home_dir()?;
    let prompt_path = home_dir.join(".config").join("opencode").join("AGENTS.md");

    if prompt_path.exists() {
        Ok(Some(prompt_path))
    } else {
        Ok(None)
    }
}

pub async fn get_opencode_prompt_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_opencode_prompt_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

/// Get Codex auth.json path if it exists
pub fn get_codex_auth_path() -> Result<Option<PathBuf>, String> {
    let resolved_root_dir = codex::get_codex_root_dir_without_db()?;
    let auth_path = resolved_root_dir.join("auth.json");

    if auth_path.exists() {
        Ok(Some(auth_path))
    } else {
        Ok(None)
    }
}

pub async fn get_codex_auth_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_codex_auth_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

/// Get Codex config.toml path if it exists
pub fn get_codex_config_path() -> Result<Option<PathBuf>, String> {
    let resolved_root_dir = codex::get_codex_root_dir_without_db()?;
    let config_path = resolved_root_dir.join("config.toml");

    if config_path.exists() {
        Ok(Some(config_path))
    } else {
        Ok(None)
    }
}

pub async fn get_codex_config_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_codex_config_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

/// Get Codex prompt file path if it exists
pub fn get_codex_prompt_path() -> Result<Option<PathBuf>, String> {
    let resolved_root_dir = codex::get_codex_root_dir_without_db()?;
    let prompt_path = runtime_location::resolve_codex_prompt_file_path(&resolved_root_dir);

    if prompt_path.exists() {
        Ok(Some(prompt_path))
    } else {
        Ok(None)
    }
}

fn get_existing_codex_prompt_paths(root_dir: &Path) -> Vec<PathBuf> {
    runtime_location::CODEX_PROMPT_FILE_NAMES
        .iter()
        .map(|file_name| root_dir.join(file_name))
        .filter(|path| path.exists())
        .collect()
}

pub async fn get_codex_prompt_paths_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Vec<PathBuf>, String> {
    let root_dir = runtime_location::get_codex_runtime_location_async(db)
        .await?
        .host_path;
    Ok(get_existing_codex_prompt_paths(&root_dir))
}

pub async fn get_codex_prompt_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_codex_prompt_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

pub fn get_codex_prompt_backup_zip_path(prompt_path: &Path) -> String {
    let file_name = prompt_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(runtime_location::CODEX_DEFAULT_PROMPT_FILE_NAME);
    format!("external-configs/codex/{file_name}")
}

pub async fn get_gemini_cli_env_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_gemini_cli_env_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

pub async fn get_gemini_cli_settings_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_gemini_cli_settings_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

pub async fn get_gemini_cli_prompt_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_gemini_cli_prompt_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

pub fn get_gemini_cli_prompt_backup_zip_path(prompt_path: &Path) -> String {
    let file_name = prompt_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(gemini_cli::DEFAULT_GEMINI_CLI_PROMPT_FILE);
    format!("external-configs/geminicli/{file_name}")
}

pub async fn get_gemini_cli_oauth_creds_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_gemini_cli_oauth_creds_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

pub async fn get_gemini_cli_tmp_dir_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_gemini_cli_tmp_dir_async(db).await?;
    Ok(path.is_dir().then_some(path))
}

pub async fn get_openclaw_config_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_openclaw_runtime_location_async(db)
        .await?
        .host_path;
    Ok(path.exists().then_some(path))
}

pub fn read_root_dir_override<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    entry_name: &str,
) -> Option<PathBuf> {
    let mut root_dir_file = archive.by_name(entry_name).ok()?;
    let mut custom_root_dir = String::new();
    let _ = root_dir_file.read_to_string(&mut custom_root_dir);
    let trimmed = custom_root_dir.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreWarning {
    pub tool: String,
    pub original_path: String,
    pub fallback_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub warnings: Vec<RestoreWarning>,
}

pub fn push_restore_warning(restore_result: &mut RestoreResult, warning: RestoreWarning) {
    if restore_result
        .warnings
        .iter()
        .any(|existing| existing == &warning)
    {
        return;
    }
    restore_result.warnings.push(warning);
}

pub fn normalize_restore_entry_name(raw_name: &str) -> String {
    raw_name.replace('\\', "/")
}

pub fn resolve_skills_restore_output_path(
    skills_dir: &Path,
    entry_name: &str,
) -> Result<Option<(PathBuf, Option<RestoreWarning>)>, String> {
    let normalized_entry = normalize_restore_entry_name(entry_name);
    let Some(relative_path) = normalized_entry.strip_prefix("skills/") else {
        return Ok(None);
    };

    if relative_path.is_empty() || normalized_entry.ends_with('/') {
        return Ok(None);
    }

    let trimmed_relative = relative_path.trim_start_matches('/');
    if trimmed_relative.is_empty() {
        return Ok(None);
    }

    let mut segments = Vec::new();
    for raw_segment in trimmed_relative.split('/') {
        let segment = raw_segment.trim();
        if segment.is_empty() || segment == "." || segment == ".." {
            continue;
        }
        segments.push(segment.to_string());
    }

    if segments.is_empty() {
        return Ok(None);
    }

    let normalized_segments: Vec<String> = segments
        .iter()
        .map(|segment| skill_storage_dir_name(segment))
        .collect();

    let mut normalized_relative = PathBuf::from(&normalized_segments[0]);
    for segment in normalized_segments.iter().skip(1) {
        normalized_relative.push(segment);
    }

    let fallback_path = skills_dir.join(&normalized_relative);
    let warning = if normalized_segments[0] != segments[0] {
        Some(RestoreWarning {
            tool: "skills".to_string(),
            original_path: segments[0].clone(),
            fallback_path: normalized_segments[0].clone(),
        })
    } else {
        None
    };

    Ok(Some((fallback_path, warning)))
}

pub fn resolve_external_config_restore_output_path(
    restore_dir: &Path,
    relative_path: &str,
) -> Result<Option<PathBuf>, String> {
    let normalized = normalize_restore_entry_name(relative_path);
    let trimmed_relative = normalized.trim_start_matches('/');
    if trimmed_relative.is_empty() {
        return Ok(None);
    }

    let mut output_path = restore_dir.to_path_buf();
    let mut has_segment = false;
    for raw_segment in trimmed_relative.split('/') {
        let segment = raw_segment.trim();
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." || segment.contains('\0') || segment.contains(':') {
            return Err(format!(
                "Invalid external config restore path: {}",
                relative_path
            ));
        }
        output_path.push(segment);
        has_segment = true;
    }

    if has_segment {
        Ok(Some(output_path))
    } else {
        Ok(None)
    }
}

fn is_restore_override_usable(path: &Path) -> bool {
    if path.as_os_str().is_empty() {
        return false;
    }

    let raw_path = path.to_string_lossy();
    if runtime_location::parse_wsl_unc_path(&raw_path).is_some() {
        return true;
    }

    path.is_absolute()
}

pub fn resolve_restore_dir_override(
    tool: &str,
    override_dir: Option<PathBuf>,
    fallback_dir: PathBuf,
) -> (PathBuf, Option<RestoreWarning>) {
    match override_dir {
        Some(custom_dir) if is_restore_override_usable(&custom_dir) => (custom_dir, None),
        Some(custom_dir) => (
            fallback_dir.clone(),
            Some(RestoreWarning {
                tool: tool.to_string(),
                original_path: custom_dir.to_string_lossy().to_string(),
                fallback_path: fallback_dir.to_string_lossy().to_string(),
            }),
        ),
        None => (fallback_dir, None),
    }
}

pub async fn get_custom_root_dir_path_info(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool: &str,
) -> Option<String> {
    match tool {
        "claude" => {
            let location = runtime_location::get_claude_runtime_location_async(db)
                .await
                .ok()?;
            if location.source == "custom" {
                Some(location.host_path.to_string_lossy().to_string())
            } else {
                None
            }
        }
        "codex" => {
            let location = runtime_location::get_codex_runtime_location_async(db)
                .await
                .ok()?;
            if location.source == "custom" {
                Some(location.host_path.to_string_lossy().to_string())
            } else {
                None
            }
        }
        "opencode" => {
            let location = runtime_location::get_opencode_runtime_location_async(db)
                .await
                .ok()?;
            if location.source == "custom" {
                location
                    .host_path
                    .parent()
                    .map(|path| path.to_string_lossy().to_string())
            } else {
                None
            }
        }
        "openclaw" => {
            let location = runtime_location::get_openclaw_runtime_location_async(db)
                .await
                .ok()?;
            if location.source == "custom" {
                location
                    .host_path
                    .parent()
                    .map(|path| path.to_string_lossy().to_string())
            } else {
                None
            }
        }
        "geminicli" => {
            let location = runtime_location::get_gemini_cli_runtime_location_async(db)
                .await
                .ok()?;
            if location.source == "custom" {
                Some(location.host_path.to_string_lossy().to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Get skills directory path
pub fn get_skills_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    Ok(app_data_dir.join("skills"))
}

pub fn get_image_assets_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    Ok(app_data_dir.join("image-studio").join("assets"))
}

/// Get models.dev.json cache file path if it exists
pub fn get_models_cache_file() -> Option<PathBuf> {
    crate::coding::open_code::free_models::get_models_cache_path().filter(|p| p.exists())
}

/// Get preset_models.json cache file path if it exists
pub fn get_preset_models_cache_file() -> Option<PathBuf> {
    crate::coding::preset_models::get_preset_models_cache_path().filter(|p| p.exists())
}

/// Add a file to zip archive with a specific path
fn add_file_to_zip<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    file_path: &Path,
    zip_path: &str,
    options: SimpleFileOptions,
) -> Result<(), String> {
    let mut file = File::open(file_path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .map_err(|e| format!("Failed to read file: {}", e))?;

    zip.start_file(zip_path, options)
        .map_err(|e| format!("Failed to start file in zip: {}", e))?;
    zip.write_all(&buffer)
        .map_err(|e| format!("Failed to write to zip: {}", e))?;

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CustomBackupManifest {
    version: u32,
    entries: Vec<CustomBackupManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CustomBackupManifestEntry {
    id: String,
    name: String,
    entry_type: BackupCustomEntryType,
    source_path: String,
    restore_path: String,
    payload_path: String,
}

fn strip_base_path(candidate: &str, base: &Path) -> Option<String> {
    let normalized_candidate = candidate.replace('\\', "/");
    let normalized_base = base.to_string_lossy().replace('\\', "/");
    let trimmed_base = normalized_base.trim_end_matches('/');

    let (candidate_cmp, base_cmp) = if cfg!(windows) {
        (
            normalized_candidate.to_lowercase(),
            trimmed_base.to_lowercase(),
        )
    } else {
        (normalized_candidate.clone(), trimmed_base.to_string())
    };

    if candidate_cmp == base_cmp {
        return Some(String::new());
    }

    let base_with_separator = format!("{}/", base_cmp);
    if candidate_cmp.starts_with(&base_with_separator) {
        return Some(normalized_candidate[trimmed_base.len() + 1..].to_string());
    }

    None
}

fn with_storage_prefix(prefix: &str, relative_path: &str) -> String {
    if relative_path.is_empty() {
        prefix.to_string()
    } else {
        format!("{}/{}", prefix, relative_path.trim_start_matches('/'))
    }
}

/// Normalize user-entered backup paths for portable storage.
///
/// Backup custom entries prefer existing tool path conventions:
/// `%APPDATA%/...` for config-dir-relative paths, `~/...` for home-relative
/// paths, and absolute paths only when no supported portable base matches.
pub fn normalize_backup_storage_path(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let normalized_input = trimmed.replace('\\', "/");
    let upper_input = normalized_input.to_uppercase();

    if upper_input == "%APPDATA%" {
        return "%APPDATA%".to_string();
    }
    if upper_input.starts_with("%APPDATA%/") {
        return with_storage_prefix("%APPDATA%", &normalized_input[10..]);
    }
    if normalized_input == "~" {
        return "~".to_string();
    }
    if let Some(relative_path) = normalized_input.strip_prefix("~/") {
        return with_storage_prefix("~", relative_path);
    }

    let expanded = crate::coding::expand_local_path(trimmed)
        .unwrap_or_else(|_| trimmed.to_string())
        .replace('\\', "/");
    let candidate = if expanded != normalized_input {
        expanded.as_str()
    } else {
        normalized_input.as_str()
    };

    if let Some(config_dir) = dirs::config_dir() {
        if let Some(relative_path) = strip_base_path(candidate, &config_dir) {
            return with_storage_prefix("%APPDATA%", &relative_path);
        }
    }

    if let Some(home_dir) = dirs::home_dir() {
        if let Some(relative_path) = strip_base_path(candidate, &home_dir) {
            return with_storage_prefix("~", &relative_path);
        }
    }

    candidate.to_string()
}

pub fn normalize_backup_custom_entry(entry: &BackupCustomEntry) -> BackupCustomEntry {
    let restore_path = entry.restore_path.as_ref().and_then(|path| {
        let normalized = normalize_backup_storage_path(path);
        (!normalized.is_empty()).then_some(normalized)
    });

    BackupCustomEntry {
        id: entry.id.trim().to_string(),
        name: entry.name.trim().to_string(),
        source_path: normalize_backup_storage_path(&entry.source_path),
        restore_path,
        entry_type: entry.entry_type.clone(),
        enabled: entry.enabled,
    }
}

pub fn resolve_backup_storage_path(storage_path: &str) -> Result<PathBuf, String> {
    let trimmed = storage_path.trim();
    if trimmed.is_empty() {
        return Err("Backup custom path is empty".to_string());
    }

    let normalized = trimmed.replace('\\', "/");
    let upper = normalized.to_uppercase();
    if normalized == "~"
        || normalized.starts_with("~/")
        || upper == "%APPDATA%"
        || upper.starts_with("%APPDATA%/")
    {
        return crate::coding::tools::resolve_storage_path(&normalized)
            .ok_or_else(|| format!("Failed to resolve backup path: {}", storage_path));
    }

    let expanded = crate::coding::expand_local_path(trimmed)?;
    if expanded != trimmed {
        return Ok(PathBuf::from(expanded));
    }

    crate::coding::tools::resolve_storage_path(trimmed)
        .ok_or_else(|| format!("Failed to resolve backup path: {}", storage_path))
}

fn safe_payload_segment(raw: &str) -> String {
    let segment: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();

    let trimmed = segment.trim_matches('.');
    if trimmed.is_empty() {
        "entry".to_string()
    } else {
        trimmed.to_string()
    }
}

fn safe_relative_zip_path(raw_relative_path: &str) -> Result<Option<String>, String> {
    let normalized = normalize_restore_entry_name(raw_relative_path);
    let mut segments = Vec::new();

    for raw_segment in normalized.split('/') {
        let segment = raw_segment.trim();
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." || segment.contains('\0') {
            return Err(format!(
                "Invalid relative path in custom backup payload: {}",
                raw_relative_path
            ));
        }
        segments.push(segment.to_string());
    }

    if segments.is_empty() {
        Ok(None)
    } else {
        Ok(Some(segments.join("/")))
    }
}

fn relative_path_for_zip(path: &Path, base_path: &Path) -> Result<Option<String>, String> {
    let relative_path = path
        .strip_prefix(base_path)
        .map_err(|e| format!("Failed to get custom backup relative path: {}", e))?;
    safe_relative_zip_path(&relative_path.to_string_lossy())
}

fn should_skip_system_file(path: &Path) -> bool {
    path.file_name()
        .map(|file_name| {
            let name = file_name.to_string_lossy();
            name == ".DS_Store" || name.starts_with("._")
        })
        .unwrap_or(false)
}

fn is_filesystem_root_directory(path: &Path) -> bool {
    let mut components = path.components();
    match (components.next(), components.next(), components.next()) {
        (Some(std::path::Component::RootDir), None, None) => true,
        (Some(std::path::Component::Prefix(_)), Some(std::path::Component::RootDir), None) => true,
        _ => false,
    }
}

fn custom_backup_payload_base(index: usize, entry_id: &str) -> String {
    format!(
        "{}/{:04}-{}",
        CUSTOM_BACKUP_PAYLOAD_DIR,
        index,
        safe_payload_segment(entry_id)
    )
}

pub async fn get_backup_custom_entries_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Vec<BackupCustomEntry>, String> {
    let mut result = db
        .query("SELECT * OMIT id FROM settings:`app` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query settings: {}", e))?;

    let records: Vec<serde_json::Value> = result
        .take(0)
        .map_err(|e| format!("Failed to parse settings: {}", e))?;

    Ok(records
        .first()
        .map(|record| crate::settings::adapter::from_db_value(record.clone()))
        .map(|settings| settings.backup_custom_entries)
        .unwrap_or_default())
}

pub fn add_custom_backup_entries_to_zip<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    entries: &[BackupCustomEntry],
    options: SimpleFileOptions,
) -> Result<(), String> {
    let enabled_entries: Vec<BackupCustomEntry> = entries
        .iter()
        .filter(|entry| entry.enabled)
        .map(normalize_backup_custom_entry)
        .collect();

    if enabled_entries.is_empty() {
        return Ok(());
    }

    zip.add_directory("custom-backup/", options)
        .map_err(|e| format!("Failed to add custom backup directory: {}", e))?;
    zip.add_directory(format!("{}/", CUSTOM_BACKUP_PAYLOAD_DIR), options)
        .map_err(|e| format!("Failed to add custom backup payload directory: {}", e))?;

    let mut manifest_entries = Vec::new();

    for (index, entry) in enabled_entries.iter().enumerate() {
        if entry.source_path.is_empty() {
            return Err(format!(
                "Custom backup entry '{}' has empty source path",
                entry.name
            ));
        }

        let source_path = resolve_backup_storage_path(&entry.source_path)?;
        let restore_path = entry
            .restore_path
            .clone()
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| entry.source_path.clone());
        let payload_base = custom_backup_payload_base(index, &entry.id);

        match entry.entry_type {
            BackupCustomEntryType::File => {
                if !source_path.is_file() {
                    return Err(format!(
                        "Custom backup entry '{}' is not a readable file: {}",
                        entry.name, entry.source_path
                    ));
                }

                let file_name = source_path
                    .file_name()
                    .map(|name| safe_payload_segment(&name.to_string_lossy()))
                    .unwrap_or_else(|| "file".to_string());
                let payload_path = format!("{}/{}", payload_base, file_name);
                zip.add_directory(format!("{}/", payload_base), options)
                    .map_err(|e| format!("Failed to add custom backup file directory: {}", e))?;
                add_file_to_zip(zip, &source_path, &payload_path, options)?;

                manifest_entries.push(CustomBackupManifestEntry {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    entry_type: entry.entry_type.clone(),
                    source_path: entry.source_path.clone(),
                    restore_path,
                    payload_path,
                });
            }
            BackupCustomEntryType::Directory => {
                if crate::coding::tools::is_root_directory(&entry.source_path)
                    || is_filesystem_root_directory(&source_path)
                {
                    return Err(format!(
                        "Custom backup entry '{}' points to a root directory: {}",
                        entry.name, entry.source_path
                    ));
                }
                if !source_path.is_dir() {
                    return Err(format!(
                        "Custom backup entry '{}' is not a readable directory: {}",
                        entry.name, entry.source_path
                    ));
                }

                let payload_path = format!("{}/", payload_base);
                zip.add_directory(&payload_path, options)
                    .map_err(|e| format!("Failed to add custom backup directory payload: {}", e))?;

                for entry_result in WalkDir::new(&source_path) {
                    let file_entry = entry_result
                        .map_err(|e| format!("Failed to read custom backup entry: {}", e))?;
                    let path = file_entry.path();
                    let Some(relative_path) = relative_path_for_zip(path, &source_path)? else {
                        continue;
                    };

                    if path.is_file() {
                        if should_skip_system_file(path) {
                            continue;
                        }

                        let zip_path = format!("{}{}", payload_path, relative_path);
                        add_file_to_zip(zip, path, &zip_path, options)?;
                    } else if path.is_dir() {
                        let zip_path = format!("{}{}/", payload_path, relative_path);
                        zip.add_directory(zip_path, options).map_err(|e| {
                            format!("Failed to add custom backup subdirectory: {}", e)
                        })?;
                    }
                }

                manifest_entries.push(CustomBackupManifestEntry {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    entry_type: entry.entry_type.clone(),
                    source_path: entry.source_path.clone(),
                    restore_path,
                    payload_path,
                });
            }
        }
    }

    let manifest = CustomBackupManifest {
        version: 1,
        entries: manifest_entries,
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("Failed to serialize custom backup manifest: {}", e))?;
    add_text_to_zip(zip, CUSTOM_BACKUP_MANIFEST_PATH, &manifest_json, options)?;

    Ok(())
}

fn find_zip_entry_by_normalized_name<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    normalized_name: &str,
) -> Option<String> {
    for index in 0..archive.len() {
        let Ok(file) = archive.by_index(index) else {
            continue;
        };
        let raw_name = file.name().to_string();
        if normalize_restore_entry_name(&raw_name) == normalized_name {
            return Some(raw_name);
        }
    }

    None
}

fn copy_zip_entry_to_path<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    raw_entry_name: &str,
    output_path: &Path,
) -> Result<(), String> {
    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create custom backup parent directory: {}", e))?;
        }
    }

    let mut input_file = archive
        .by_name(raw_entry_name)
        .map_err(|e| format!("Failed to read custom backup payload: {}", e))?;
    let mut output_file = File::create(output_path)
        .map_err(|e| format!("Failed to create custom backup file: {}", e))?;
    std::io::copy(&mut input_file, &mut output_file)
        .map_err(|e| format!("Failed to restore custom backup file: {}", e))?;

    Ok(())
}

pub fn restore_custom_backup_entries<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<(), String> {
    let manifest_raw_name =
        match find_zip_entry_by_normalized_name(archive, CUSTOM_BACKUP_MANIFEST_PATH) {
            Some(name) => name,
            None => return Ok(()),
        };

    let mut manifest_file = archive
        .by_name(&manifest_raw_name)
        .map_err(|e| format!("Failed to read custom backup manifest: {}", e))?;
    let mut manifest_content = String::new();
    manifest_file
        .read_to_string(&mut manifest_content)
        .map_err(|e| format!("Failed to read custom backup manifest: {}", e))?;
    drop(manifest_file);

    let manifest: CustomBackupManifest = serde_json::from_str(&manifest_content)
        .map_err(|e| format!("Failed to parse custom backup manifest: {}", e))?;

    if manifest.version != 1 {
        return Err(format!(
            "Unsupported custom backup manifest version: {}",
            manifest.version
        ));
    }

    for entry in manifest.entries {
        let target_path = resolve_backup_storage_path(&entry.restore_path)?;
        let normalized_payload = normalize_restore_entry_name(&entry.payload_path);

        match entry.entry_type {
            BackupCustomEntryType::File => {
                let raw_payload_name =
                    find_zip_entry_by_normalized_name(archive, &normalized_payload).ok_or_else(
                        || {
                            format!(
                                "Custom backup payload missing for '{}': {}",
                                entry.name, entry.payload_path
                            )
                        },
                    )?;
                copy_zip_entry_to_path(archive, &raw_payload_name, &target_path)?;
            }
            BackupCustomEntryType::Directory => {
                std::fs::create_dir_all(&target_path)
                    .map_err(|e| format!("Failed to create custom backup directory: {}", e))?;

                let payload_prefix = if normalized_payload.ends_with('/') {
                    normalized_payload
                } else {
                    format!("{}/", normalized_payload)
                };
                let mut payload_entries = Vec::new();

                for index in 0..archive.len() {
                    let file = archive
                        .by_index(index)
                        .map_err(|e| format!("Failed to read custom backup payload: {}", e))?;
                    let raw_name = file.name().to_string();
                    let normalized_name = normalize_restore_entry_name(&raw_name);
                    if !normalized_name.starts_with(&payload_prefix)
                        || normalized_name.ends_with('/')
                    {
                        continue;
                    }

                    let relative_name = &normalized_name[payload_prefix.len()..];
                    let Some(safe_relative_name) = safe_relative_zip_path(relative_name)? else {
                        continue;
                    };
                    payload_entries.push((raw_name, safe_relative_name));
                }

                for (raw_name, relative_name) in payload_entries {
                    let relative_path =
                        PathBuf::from(crate::coding::tools::to_platform_path(&relative_name));
                    let output_path = target_path.join(relative_path);
                    copy_zip_entry_to_path(archive, &raw_name, &output_path)?;
                }
            }
        }
    }

    Ok(())
}

pub async fn get_backup_image_assets_enabled_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<bool, String> {
    let mut result = db
        .query("SELECT * OMIT id FROM settings:`app` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query settings: {}", e))?;

    let records: Vec<serde_json::Value> = result
        .take(0)
        .map_err(|e| format!("Failed to parse settings: {}", e))?;

    Ok(records
        .first()
        .map(|record| crate::settings::adapter::from_db_value(record.clone()))
        .map(|settings| settings.backup_image_assets_enabled)
        .unwrap_or(true))
}

pub fn add_text_to_zip<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    zip_path: &str,
    content: &str,
    options: SimpleFileOptions,
) -> Result<(), String> {
    zip.start_file(zip_path, options)
        .map_err(|e| format!("Failed to start text file in zip: {}", e))?;
    zip.write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write text to zip: {}", e))?;
    Ok(())
}

pub fn add_image_assets_to_zip<W: Write + std::io::Seek>(
    app_handle: &tauri::AppHandle,
    zip: &mut ZipWriter<W>,
    options: SimpleFileOptions,
) -> Result<(), String> {
    let image_assets_dir = get_image_assets_dir(app_handle)?;
    if !image_assets_dir.exists() {
        return Ok(());
    }

    zip.add_directory("image-studio/assets/", options)
        .map_err(|e| format!("Failed to add image assets directory: {}", e))?;

    for entry in WalkDir::new(&image_assets_dir) {
        let entry = entry.map_err(|e| format!("Failed to read image asset entry: {}", e))?;
        let path = entry.path();
        let relative_path = path
            .strip_prefix(&image_assets_dir)
            .map_err(|e| format!("Failed to get image asset relative path: {}", e))?;

        if path.is_file() {
            if let Some(file_name) = path.file_name() {
                let name_str = file_name.to_string_lossy();
                if name_str == ".DS_Store" || name_str.starts_with("._") {
                    continue;
                }
            }

            let relative_str = relative_path.to_string_lossy().replace('\\', "/");
            let name = format!("image-studio/assets/{}", relative_str);
            add_file_to_zip(zip, path, &name, options)?;
        } else if path.is_dir() && !relative_path.as_os_str().is_empty() {
            let relative_str = relative_path.to_string_lossy().replace('\\', "/");
            let name = format!("image-studio/assets/{}/", relative_str);
            zip.add_directory(name, options)
                .map_err(|e| format!("Failed to add image asset subdirectory: {}", e))?;
        }
    }

    Ok(())
}

pub fn add_directory_contents_to_zip<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    source_dir: &Path,
    zip_prefix: &str,
    options: SimpleFileOptions,
) -> Result<(), String> {
    if !source_dir.is_dir() {
        return Ok(());
    }

    let normalized_prefix = zip_prefix.trim_end_matches('/');
    zip.add_directory(format!("{}/", normalized_prefix), options)
        .map_err(|e| format!("Failed to add directory to zip: {}", e))?;

    for entry in WalkDir::new(source_dir) {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let path = entry.path();
        let relative_path = path
            .strip_prefix(source_dir)
            .map_err(|e| format!("Failed to get relative path: {}", e))?;

        if path.is_file() {
            if let Some(file_name) = path.file_name() {
                let name_str = file_name.to_string_lossy();
                if name_str == ".DS_Store" || name_str.starts_with("._") {
                    continue;
                }
            }

            let relative_str = relative_path.to_string_lossy().replace('\\', "/");
            let name = format!("{}/{}", normalized_prefix, relative_str);
            add_file_to_zip(zip, path, &name, options)?;
        } else if path.is_dir() && !relative_path.as_os_str().is_empty() {
            let relative_str = relative_path.to_string_lossy().replace('\\', "/");
            let name = format!("{}/{}/", normalized_prefix, relative_str);
            zip.add_directory(name, options)
                .map_err(|e| format!("Failed to add subdirectory to zip: {}", e))?;
        }
    }

    Ok(())
}

/// Create a temporary backup zip file and return its contents as bytes
pub async fn create_backup_zip(
    app_handle: &tauri::AppHandle,
    db_path: &Path,
    include_image_assets: bool,
) -> Result<Vec<u8>, String> {
    use std::io::Cursor;

    let mut buffer = Cursor::new(Vec::new());
    let db_state = app_handle.state::<crate::DbState>();
    let db = db_state.db();

    {
        let mut zip = ZipWriter::new(&mut buffer);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        let mut has_files = false;

        // Add database files under db/ prefix
        for entry in WalkDir::new(db_path) {
            let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
            let path = entry.path();
            let relative_path = path
                .strip_prefix(db_path)
                .map_err(|e| format!("Failed to get relative path: {}", e))?;

            if path.is_file() {
                // Skip system files like .DS_Store
                if let Some(file_name) = path.file_name() {
                    let name_str = file_name.to_string_lossy();
                    if name_str == ".DS_Store" || name_str.starts_with("._") {
                        continue;
                    }
                }

                has_files = true;
                // Use forward slashes for cross-platform compatibility in zip files
                let relative_str = relative_path.to_string_lossy().replace('\\', "/");
                let name = format!("db/{}", relative_str);
                zip.start_file(name, options)
                    .map_err(|e| format!("Failed to start file in zip: {}", e))?;

                let mut file =
                    File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
                let mut file_buffer = Vec::new();
                file.read_to_end(&mut file_buffer)
                    .map_err(|e| format!("Failed to read file: {}", e))?;
                zip.write_all(&file_buffer)
                    .map_err(|e| format!("Failed to write to zip: {}", e))?;
            } else if path.is_dir() && !relative_path.as_os_str().is_empty() {
                // Use forward slashes for cross-platform compatibility in zip files
                let relative_str = relative_path.to_string_lossy().replace('\\', "/");
                let name = format!("db/{}/", relative_str);
                zip.add_directory(name, options)
                    .map_err(|e| format!("Failed to add directory to zip: {}", e))?;
            }
        }

        if !has_files {
            zip.start_file("db/.backup_marker", options)
                .map_err(|e| format!("Failed to create marker file: {}", e))?;
            zip.write_all(b"AI Toolbox Backup")
                .map_err(|e| format!("Failed to write marker: {}", e))?;
        }

        // Add external-configs directory
        zip.add_directory("external-configs/", options)
            .map_err(|e| format!("Failed to add external-configs directory: {}", e))?;

        if let Some(custom_dir) = get_custom_root_dir_path_info(&db, "opencode").await {
            zip.add_directory("external-configs/opencode/", options)
                .map_err(|e| format!("Failed to add opencode directory: {}", e))?;
            add_text_to_zip(
                &mut zip,
                "external-configs/opencode/root-dir.txt",
                &custom_dir,
                options,
            )?;
        }

        // Backup OpenCode config if exists
        if let Some(opencode_path) = get_opencode_config_path_from_db(&db).await? {
            let file_name = opencode_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "opencode.json".to_string());
            let zip_path = format!("external-configs/opencode/{}", file_name);

            zip.add_directory("external-configs/opencode/", options)
                .map_err(|e| format!("Failed to add opencode directory: {}", e))?;

            add_file_to_zip(&mut zip, &opencode_path, &zip_path, options)?;
        }

        // Backup OpenCode auth.json if exists
        if let Some(opencode_auth_path) = get_opencode_auth_path_from_db(&db).await? {
            let zip_path = "external-configs/opencode/auth.json";

            // Directory may already exist from opencode config backup
            let _ = zip.add_directory("external-configs/opencode/", options);

            add_file_to_zip(&mut zip, &opencode_auth_path, zip_path, options)?;
        }

        if let Some(opencode_prompt_path) = get_opencode_prompt_path_from_db(&db).await? {
            let zip_path = "external-configs/opencode/AGENTS.md";

            let _ = zip.add_directory("external-configs/opencode/", options);

            add_file_to_zip(&mut zip, &opencode_prompt_path, zip_path, options)?;
        }

        if let Some(custom_root_dir) = get_custom_root_dir_path_info(&db, "claude").await {
            zip.add_directory("external-configs/claude/", options)
                .map_err(|e| format!("Failed to add claude directory: {}", e))?;
            add_text_to_zip(
                &mut zip,
                "external-configs/claude/root-dir.txt",
                &custom_root_dir,
                options,
            )?;
        }

        // Backup Claude settings.json if exists
        if let Some(claude_path) = get_claude_settings_path_from_db(&db).await? {
            let zip_path = "external-configs/claude/settings.json";

            zip.add_directory("external-configs/claude/", options)
                .map_err(|e| format!("Failed to add claude directory: {}", e))?;

            add_file_to_zip(&mut zip, &claude_path, zip_path, options)?;
        }

        if let Some(claude_prompt_path) = get_claude_prompt_path_from_db(&db).await? {
            let zip_path = "external-configs/claude/CLAUDE.md";

            let _ = zip.add_directory("external-configs/claude/", options);

            add_file_to_zip(&mut zip, &claude_prompt_path, zip_path, options)?;
        }

        if let Some(claude_mcp_path) = get_claude_mcp_path_from_db(&db).await? {
            let zip_path = "external-configs/claude/.claude.json";
            let _ = zip.add_directory("external-configs/claude/", options);
            add_file_to_zip(&mut zip, &claude_mcp_path, zip_path, options)?;
        }

        if let Some(custom_root_dir) = get_custom_root_dir_path_info(&db, "codex").await {
            zip.add_directory("external-configs/codex/", options)
                .map_err(|e| format!("Failed to add codex directory: {}", e))?;
            add_text_to_zip(
                &mut zip,
                "external-configs/codex/root-dir.txt",
                &custom_root_dir,
                options,
            )?;
        }

        // Backup Codex auth.json if exists
        if let Some(codex_auth_path) = get_codex_auth_path_from_db(&db).await? {
            let zip_path = "external-configs/codex/auth.json";

            zip.add_directory("external-configs/codex/", options)
                .map_err(|e| format!("Failed to add codex directory: {}", e))?;

            add_file_to_zip(&mut zip, &codex_auth_path, zip_path, options)?;
        }

        // Backup Codex config.toml if exists
        if let Some(codex_config_path) = get_codex_config_path_from_db(&db).await? {
            let zip_path = "external-configs/codex/config.toml";

            // Directory may already exist from auth.json backup
            let _ = zip.add_directory("external-configs/codex/", options);

            add_file_to_zip(&mut zip, &codex_config_path, zip_path, options)?;
        }

        for codex_prompt_path in get_codex_prompt_paths_from_db(&db).await? {
            let zip_path = get_codex_prompt_backup_zip_path(&codex_prompt_path);

            let _ = zip.add_directory("external-configs/codex/", options);

            add_file_to_zip(&mut zip, &codex_prompt_path, &zip_path, options)?;
        }

        if let Some(custom_dir) = get_custom_root_dir_path_info(&db, "openclaw").await {
            zip.add_directory("external-configs/openclaw/", options)
                .map_err(|e| format!("Failed to add openclaw directory: {}", e))?;
            add_text_to_zip(
                &mut zip,
                "external-configs/openclaw/root-dir.txt",
                &custom_dir,
                options,
            )?;
        }

        if let Some(openclaw_config_path) = get_openclaw_config_path_from_db(&db).await? {
            let zip_path = "external-configs/openclaw/openclaw.json";
            let _ = zip.add_directory("external-configs/openclaw/", options);
            add_file_to_zip(&mut zip, &openclaw_config_path, zip_path, options)?;
        }

        if let Some(custom_root_dir) = get_custom_root_dir_path_info(&db, "geminicli").await {
            zip.add_directory("external-configs/geminicli/", options)
                .map_err(|e| format!("Failed to add Gemini CLI directory: {}", e))?;
            add_text_to_zip(
                &mut zip,
                "external-configs/geminicli/root-dir.txt",
                &custom_root_dir,
                options,
            )?;
        }

        if let Some(gemini_env_path) = get_gemini_cli_env_path_from_db(&db).await? {
            let zip_path = "external-configs/geminicli/.env";
            let _ = zip.add_directory("external-configs/geminicli/", options);
            add_file_to_zip(&mut zip, &gemini_env_path, zip_path, options)?;
        }

        if let Some(gemini_settings_path) = get_gemini_cli_settings_path_from_db(&db).await? {
            let zip_path = "external-configs/geminicli/settings.json";
            let _ = zip.add_directory("external-configs/geminicli/", options);
            add_file_to_zip(&mut zip, &gemini_settings_path, zip_path, options)?;
        }

        if let Some(gemini_prompt_path) = get_gemini_cli_prompt_path_from_db(&db).await? {
            let zip_path = get_gemini_cli_prompt_backup_zip_path(&gemini_prompt_path);
            let _ = zip.add_directory("external-configs/geminicli/", options);
            add_file_to_zip(&mut zip, &gemini_prompt_path, &zip_path, options)?;
        }

        if let Some(gemini_oauth_path) = get_gemini_cli_oauth_creds_path_from_db(&db).await? {
            let zip_path = "external-configs/geminicli/oauth_creds.json";
            let _ = zip.add_directory("external-configs/geminicli/", options);
            add_file_to_zip(&mut zip, &gemini_oauth_path, zip_path, options)?;
        }

        if let Some(gemini_tmp_dir) = get_gemini_cli_tmp_dir_from_db(&db).await? {
            add_directory_contents_to_zip(
                &mut zip,
                &gemini_tmp_dir,
                "external-configs/geminicli/tmp",
                options,
            )?;
        }

        // Backup models.dev.json cache if exists
        if let Some(models_cache_path) = get_models_cache_file() {
            add_file_to_zip(&mut zip, &models_cache_path, "models.dev.json", options)?;
        }

        // Backup preset_models.json cache if exists
        if let Some(preset_models_cache_path) = get_preset_models_cache_file() {
            add_file_to_zip(
                &mut zip,
                &preset_models_cache_path,
                "preset_models.json",
                options,
            )?;
        }

        // Backup skills directory if exists
        let skills_dir = get_skills_dir(app_handle)?;
        if skills_dir.exists() {
            zip.add_directory("skills/", options)
                .map_err(|e| format!("Failed to add skills directory: {}", e))?;

            for entry in WalkDir::new(&skills_dir) {
                let entry = entry.map_err(|e| format!("Failed to read skills entry: {}", e))?;
                let path = entry.path();
                let relative_path = path
                    .strip_prefix(&skills_dir)
                    .map_err(|e| format!("Failed to get relative path: {}", e))?;

                if path.is_file() {
                    // Skip system files
                    if let Some(file_name) = path.file_name() {
                        let name_str = file_name.to_string_lossy();
                        if name_str == ".DS_Store" || name_str.starts_with("._") {
                            continue;
                        }
                    }

                    let relative_str = relative_path.to_string_lossy().replace('\\', "/");
                    let name = format!("skills/{}", relative_str);
                    add_file_to_zip(&mut zip, path, &name, options)?;
                } else if path.is_dir() && !relative_path.as_os_str().is_empty() {
                    let relative_str = relative_path.to_string_lossy().replace('\\', "/");
                    let name = format!("skills/{}/", relative_str);
                    zip.add_directory(name, options)
                        .map_err(|e| format!("Failed to add skills subdirectory: {}", e))?;
                }
            }
        }

        if include_image_assets {
            add_image_assets_to_zip(app_handle, &mut zip, options)?;
        }

        let backup_custom_entries = get_backup_custom_entries_from_db(&db).await?;
        add_custom_backup_entries_to_zip(&mut zip, &backup_custom_entries, options)?;

        zip.finish()
            .map_err(|e| format!("Failed to finish zip: {}", e))?;
    }

    Ok(buffer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::{
        CUSTOM_BACKUP_MANIFEST_PATH, add_custom_backup_entries_to_zip,
        get_codex_prompt_backup_zip_path, get_existing_codex_prompt_paths,
        is_filesystem_root_directory, normalize_backup_storage_path,
        resolve_external_config_restore_output_path, restore_custom_backup_entries,
    };
    use crate::settings::types::{BackupCustomEntry, BackupCustomEntryType};
    use std::fs;
    use std::io::{Cursor, Read};
    use std::path::Path;
    use zip::write::SimpleFileOptions;
    use zip::{ZipArchive, ZipWriter};

    fn build_zip(entries: &[BackupCustomEntry]) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buffer);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            add_custom_backup_entries_to_zip(&mut zip, entries, options)
                .expect("add custom entries");
            zip.finish().expect("finish zip");
        }
        buffer.into_inner()
    }

    #[test]
    fn normalize_backup_storage_path_preserves_supported_prefixes() {
        assert_eq!(
            normalize_backup_storage_path("~\\.config\\opencode\\custom.json"),
            "~/.config/opencode/custom.json"
        );
        assert_eq!(
            normalize_backup_storage_path("%APPDATA%\\Code\\User\\mcp.json"),
            "%APPDATA%/Code/User/mcp.json"
        );
    }

    #[test]
    fn external_config_restore_path_rejects_traversal() {
        let root = Path::new("restore-root");
        let output_path =
            resolve_external_config_restore_output_path(root, "tmp/project/session.jsonl")
                .expect("resolve safe path")
                .expect("path should exist");
        assert_eq!(
            output_path,
            root.join("tmp").join("project").join("session.jsonl")
        );

        let leading_slash_path =
            resolve_external_config_restore_output_path(root, "/tmp/project/session.jsonl")
                .expect("resolve normalized path")
                .expect("path should exist");
        assert_eq!(
            leading_slash_path,
            root.join("tmp").join("project").join("session.jsonl")
        );

        assert!(resolve_external_config_restore_output_path(root, "../settings.json").is_err());
        assert!(resolve_external_config_restore_output_path(root, "tmp/../settings.json").is_err());
        assert!(resolve_external_config_restore_output_path(root, "C:/settings.json").is_err());
    }

    #[test]
    fn codex_prompt_backup_path_preserves_active_prompt_file_name() {
        assert_eq!(
            get_codex_prompt_backup_zip_path(Path::new("/tmp/.codex/AGENTS.override.md")),
            "external-configs/codex/AGENTS.override.md"
        );
        assert_eq!(
            get_codex_prompt_backup_zip_path(Path::new("/tmp/.codex/AGENTS.md")),
            "external-configs/codex/AGENTS.md"
        );
    }

    #[test]
    fn codex_prompt_backup_collects_default_and_override_files() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        fs::write(temp_dir.path().join("AGENTS.md"), "base").expect("write base prompt");
        fs::write(temp_dir.path().join("AGENTS.override.md"), "override")
            .expect("write override prompt");

        let file_names: Vec<_> = get_existing_codex_prompt_paths(temp_dir.path())
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert_eq!(file_names, vec!["AGENTS.md", "AGENTS.override.md"]);
    }

    #[test]
    fn custom_file_entry_is_written_with_manifest() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let source_path = temp_dir.path().join("custom.json");
        fs::write(&source_path, "{\"ok\":true}").expect("write source");
        let source_path_str = source_path.to_string_lossy().to_string();
        let entries = vec![BackupCustomEntry {
            id: "file-entry".to_string(),
            name: "File Entry".to_string(),
            source_path: source_path_str.clone(),
            restore_path: None,
            entry_type: BackupCustomEntryType::File,
            enabled: true,
        }];

        let zip_data = build_zip(&entries);
        let mut archive = ZipArchive::new(Cursor::new(zip_data)).expect("zip archive");

        let mut manifest = String::new();
        archive
            .by_name(CUSTOM_BACKUP_MANIFEST_PATH)
            .expect("manifest")
            .read_to_string(&mut manifest)
            .expect("read manifest");
        let normalized_source_path = normalize_backup_storage_path(&source_path_str);
        assert!(manifest.contains("\"source_path\""));
        assert!(manifest.contains(&normalized_source_path));

        let mut payload = String::new();
        archive
            .by_name("custom-backup/payload/0000-file-entry/custom.json")
            .expect("payload")
            .read_to_string(&mut payload)
            .expect("read payload");
        assert_eq!(payload, "{\"ok\":true}");
    }

    #[test]
    fn custom_directory_entry_rejects_filesystem_root() {
        assert!(is_filesystem_root_directory(Path::new("/")));

        let mut buffer = Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(&mut buffer);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let entries = vec![BackupCustomEntry {
            id: "root-entry".to_string(),
            name: "Root Entry".to_string(),
            source_path: "/".to_string(),
            restore_path: None,
            entry_type: BackupCustomEntryType::Directory,
            enabled: true,
        }];

        let error = add_custom_backup_entries_to_zip(&mut zip, &entries, options)
            .expect_err("filesystem root should be rejected");
        assert!(error.contains("root directory"));
    }

    #[test]
    fn custom_directory_entry_restores_without_deleting_extra_files() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let source_dir = temp_dir.path().join("source");
        let restore_dir = temp_dir.path().join("restore");
        fs::create_dir_all(source_dir.join("nested")).expect("create source");
        fs::write(source_dir.join("nested").join("config.json"), "{}").expect("write source");
        fs::create_dir_all(&restore_dir).expect("create restore");
        fs::write(restore_dir.join("extra.txt"), "keep").expect("write extra");

        let entries = vec![BackupCustomEntry {
            id: "dir-entry".to_string(),
            name: "Dir Entry".to_string(),
            source_path: source_dir.to_string_lossy().to_string(),
            restore_path: Some(restore_dir.to_string_lossy().to_string()),
            entry_type: BackupCustomEntryType::Directory,
            enabled: true,
        }];

        let zip_data = build_zip(&entries);
        let mut archive = ZipArchive::new(Cursor::new(zip_data)).expect("zip archive");
        restore_custom_backup_entries(&mut archive).expect("restore custom entries");

        assert_eq!(
            fs::read_to_string(restore_dir.join("nested").join("config.json"))
                .expect("read restored"),
            "{}"
        );
        assert_eq!(
            fs::read_to_string(restore_dir.join("extra.txt")).expect("read extra"),
            "keep"
        );
    }
}
