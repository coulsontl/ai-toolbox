use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::Manager;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::coding::open_code::shell_env;
use crate::coding::skills::central_repo::skill_storage_dir_name;
use crate::coding::{claude_code, codex, runtime_location};

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
    let prompt_path = resolved_root_dir.join("AGENTS.md");

    if prompt_path.exists() {
        Ok(Some(prompt_path))
    } else {
        Ok(None)
    }
}

pub async fn get_codex_prompt_path_from_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_codex_prompt_path_async(db).await?;
    Ok(path.exists().then_some(path))
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

        if let Some(codex_prompt_path) = get_codex_prompt_path_from_db(&db).await? {
            let zip_path = "external-configs/codex/AGENTS.md";

            let _ = zip.add_directory("external-configs/codex/", options);

            add_file_to_zip(&mut zip, &codex_prompt_path, zip_path, options)?;
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

        zip.finish()
            .map_err(|e| format!("Failed to finish zip: {}", e))?;
    }

    Ok(buffer.into_inner())
}
