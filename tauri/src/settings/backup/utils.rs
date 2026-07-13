use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use tauri::Manager;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::coding::open_code::shell_env;
use crate::coding::skills::central_repo::{resolve_central_repo_path_sync, skill_storage_dir_name};
use crate::coding::{claude_code, codex, gemini_cli, grok, pi, runtime_location};
use crate::settings::types::{
    BackupCustomEntry, BackupCustomEntryType, BackupFileFilterPathOption, BackupFileFilterRule,
};

const CUSTOM_BACKUP_MANIFEST_PATH: &str = "custom-backup/manifest.json";
const CUSTOM_BACKUP_PAYLOAD_DIR: &str = "custom-backup/payload";
const SQLITE_BACKUP_ZIP_PATH: &str = "sqlite/ai-toolbox.db";
const DB_MANIFEST_ZIP_PATH: &str = "db_manifest.json";

/// Get database directory path
pub fn get_db_path(app_handle: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    Ok(app_data_dir.join("database"))
}

pub fn add_sqlite_database_snapshot_to_zip<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    app_handle: &tauri::AppHandle,
    options: SimpleFileOptions,
) -> Result<(), String> {
    let sqlite_state = app_handle.state::<crate::db::SqliteDbState>();
    let schema_version = sqlite_state
        .with_conn(crate::db::migrations::get_user_version)
        .unwrap_or(0);
    let temp_path = std::env::temp_dir().join(format!(
        "ai-toolbox-sqlite-backup-{}.db",
        uuid::Uuid::new_v4().simple()
    ));

    let backup_result = sqlite_state.with_conn(|conn| {
        crate::db::backup::backup_to_path(conn, &temp_path)
            .map(|_| ())
            .map_err(|error| format!("Failed to create SQLite backup snapshot: {error}"))
    });

    if let Err(error) = backup_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }

    zip.add_directory("sqlite/", options)
        .map_err(|error| format!("Failed to add sqlite directory to backup zip: {error}"))?;
    add_path_to_zip(zip, &temp_path, SQLITE_BACKUP_ZIP_PATH, options)?;
    let _ = std::fs::remove_file(&temp_path);

    let manifest = build_db_manifest("sqlite", i64::from(schema_version));
    add_text_to_zip(
        zip,
        DB_MANIFEST_ZIP_PATH,
        &serde_json::to_string_pretty(&manifest)
            .map_err(|error| format!("Failed to serialize db manifest: {error}"))?,
        options,
    )?;

    Ok(())
}

pub fn restore_sqlite_database_snapshot_from_zip<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    app_handle: &tauri::AppHandle,
) -> Result<bool, String> {
    let Some(schema_version) = read_backup_schema_version(archive)? else {
        return Ok(false);
    };
    let target_version = i64::from(crate::db::migrations::TARGET_SCHEMA_VERSION);
    if schema_version > target_version {
        let error =
            crate::db::migrations::future_backup_schema_error(schema_version, target_version);
        return Err(crate::db::migrations::future_backup_schema_user_message(
            &error,
        ));
    }

    let Ok(mut sqlite_entry) = archive.by_name(SQLITE_BACKUP_ZIP_PATH) else {
        return Ok(false);
    };

    let temp_path = std::env::temp_dir().join(format!(
        "ai-toolbox-sqlite-restore-{}.db",
        uuid::Uuid::new_v4().simple()
    ));
    {
        let mut temp_file = File::create(&temp_path).map_err(|error| {
            format!(
                "Failed to create temporary SQLite restore file {}: {error}",
                temp_path.display()
            )
        })?;
        std::io::copy(&mut sqlite_entry, &mut temp_file)
            .map_err(|error| format!("Failed to extract SQLite backup snapshot: {error}"))?;
    }

    let sqlite_state = app_handle.state::<crate::db::SqliteDbState>();
    let restore_result = sqlite_state.with_conn_mut(|conn| {
        conn.restore(
            rusqlite::MAIN_DB,
            &temp_path,
            None::<fn(rusqlite::backup::Progress)>,
        )
        .map_err(|error| format!("Failed to restore SQLite backup snapshot: {error}"))
    });
    let _ = std::fs::remove_file(&temp_path);
    restore_result?;

    Ok(true)
}

#[cfg(not(target_os = "windows"))]
fn sanitize_claude_settings_string_for_current_os(content: &str) -> Result<Option<String>, String> {
    crate::coding::config_cleanup::sanitize_claude_settings_content_for_non_windows_target(content)
}

pub fn restore_claude_external_config_file<R: Read>(
    source: &mut R,
    outpath: &Path,
    relative_path: &str,
) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    {
        if relative_path == "settings.json" {
            let mut content = String::new();
            source
                .read_to_string(&mut content)
                .map_err(|error| format!("Failed to read Claude settings file: {error}"))?;
            let restored_content =
                sanitize_claude_settings_string_for_current_os(&content)?.unwrap_or(content);
            std::fs::write(outpath, restored_content)
                .map_err(|error| format!("Failed to restore Claude settings file: {error}"))?;
            return Ok(());
        }
    }

    #[cfg(target_os = "windows")]
    {
        let _ = relative_path;
    }

    let mut outfile = File::create(outpath).map_err(|e| format!("Failed to create file: {}", e))?;
    std::io::copy(source, &mut outfile).map_err(|e| format!("Failed to extract file: {}", e))?;
    Ok(())
}

pub fn sanitize_restored_claude_database_for_current_os(
    app_handle: &tauri::AppHandle,
) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    {
        use crate::db::helpers::{db_get, db_list, db_put};
        use crate::db::schema::DbTable;

        fn sanitize_string_field(
            record: &mut serde_json::Value,
            field_key: &str,
        ) -> Result<bool, String> {
            let Some(raw_value) = record.get(field_key).and_then(serde_json::Value::as_str) else {
                return Ok(false);
            };
            if raw_value.trim().is_empty() {
                return Ok(false);
            }
            let sanitized_value = match sanitize_claude_settings_string_for_current_os(raw_value) {
                Ok(Some(value)) => value,
                Ok(None) => return Ok(false),
                Err(error) => {
                    log::warn!(
                        "Skipped restored Claude database field sanitize: field={}, error={}",
                        field_key,
                        error
                    );
                    return Ok(false);
                }
            };
            record[field_key] = serde_json::Value::String(sanitized_value);
            Ok(true)
        }

        let sqlite_state = app_handle.state::<crate::db::SqliteDbState>();
        sqlite_state.with_conn(|conn| {
            if let Some(mut common_config) = db_get(conn, DbTable::ClaudeCommonConfig, "common")? {
                if sanitize_string_field(&mut common_config, "config")? {
                    db_put(conn, DbTable::ClaudeCommonConfig, "common", &common_config)?;
                }
            }

            for mut provider in db_list(conn, DbTable::ClaudeProvider, None)? {
                let Some(provider_id) = provider
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                else {
                    continue;
                };
                let mut changed = sanitize_string_field(&mut provider, "settings_config")?;
                changed |= sanitize_string_field(&mut provider, "extra_settings_config")?;
                if changed {
                    db_put(conn, DbTable::ClaudeProvider, &provider_id, &provider)?;
                }
            }

            Ok(())
        })?;
    }

    #[cfg(target_os = "windows")]
    {
        let _ = app_handle;
    }

    Ok(())
}

fn read_backup_schema_version<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Option<i64>, String> {
    let Ok(mut manifest_entry) = archive.by_name(DB_MANIFEST_ZIP_PATH) else {
        return Ok(None);
    };

    let mut manifest_text = String::new();
    manifest_entry
        .read_to_string(&mut manifest_text)
        .map_err(|error| format!("Failed to read db manifest: {error}"))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text)
        .map_err(|error| format!("Failed to parse db manifest: {error}"))?;

    Ok(manifest
        .get("schema_version")
        .and_then(|value| value.as_i64()))
}

fn add_path_to_zip<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    source_path: &Path,
    zip_path: &str,
    options: SimpleFileOptions,
) -> Result<(), String> {
    zip.start_file(zip_path, options)
        .map_err(|error| format!("Failed to start {zip_path} in backup zip: {error}"))?;

    let mut file = File::open(source_path)
        .map_err(|error| format!("Failed to open {}: {error}", source_path.display()))?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .map_err(|error| format!("Failed to read {}: {error}", source_path.display()))?;
    zip.write_all(&buffer)
        .map_err(|error| format!("Failed to write {zip_path} to backup zip: {error}"))?;

    Ok(())
}

fn build_db_manifest(engine: &str, schema_version: i64) -> serde_json::Value {
    serde_json::json!({
        "engine": engine,
        "schema_version": schema_version,
        "app_version": env!("CARGO_PKG_VERSION"),
        "sqlite_path": SQLITE_BACKUP_ZIP_PATH,
    })
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

pub fn get_grok_restore_dir() -> Result<PathBuf, String> {
    grok::get_grok_root_dir_without_db()
}

pub fn harden_restored_sensitive_file(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, permissions).map_err(|error| {
            format!(
                "Failed to set secure permissions on restored sensitive file {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
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
    db: &crate::db::SqliteDbState,
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
    db: &crate::db::SqliteDbState,
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
    db: &crate::db::SqliteDbState,
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
    db: &crate::db::SqliteDbState,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_claude_prompt_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

pub async fn get_claude_mcp_path_from_db(
    db: &crate::db::SqliteDbState,
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
    db: &crate::db::SqliteDbState,
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
    db: &crate::db::SqliteDbState,
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
    db: &crate::db::SqliteDbState,
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
    db: &crate::db::SqliteDbState,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_codex_config_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

pub async fn get_grok_auth_path_from_db(
    db: &crate::db::SqliteDbState,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_grok_auth_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

pub async fn get_grok_config_path_from_db(
    db: &crate::db::SqliteDbState,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_grok_config_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

pub async fn get_grok_prompt_path_from_db(
    db: &crate::db::SqliteDbState,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_grok_prompt_path_async(db).await?;
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
    db: &crate::db::SqliteDbState,
) -> Result<Vec<PathBuf>, String> {
    let root_dir = runtime_location::get_codex_runtime_location_async(db)
        .await?
        .host_path;
    Ok(get_existing_codex_prompt_paths(&root_dir))
}

pub async fn get_codex_prompt_path_from_db(
    db: &crate::db::SqliteDbState,
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
    db: &crate::db::SqliteDbState,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_gemini_cli_env_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

pub async fn get_gemini_cli_settings_path_from_db(
    db: &crate::db::SqliteDbState,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_gemini_cli_settings_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

pub async fn get_gemini_cli_prompt_path_from_db(
    db: &crate::db::SqliteDbState,
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
    db: &crate::db::SqliteDbState,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_gemini_cli_oauth_creds_path_async(db).await?;
    Ok(path.exists().then_some(path))
}

pub async fn get_gemini_cli_tmp_dir_from_db(
    db: &crate::db::SqliteDbState,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_gemini_cli_tmp_dir_async(db).await?;
    Ok(path.is_dir().then_some(path))
}

pub async fn get_openclaw_config_path_from_db(
    db: &crate::db::SqliteDbState,
) -> Result<Option<PathBuf>, String> {
    let path = runtime_location::get_openclaw_runtime_location_async(db)
        .await?
        .host_path;
    Ok(path.exists().then_some(path))
}

pub async fn get_pi_runtime_file_path_from_db(
    db: &crate::db::SqliteDbState,
    file_name: &str,
) -> Result<Option<PathBuf>, String> {
    let root_dir = runtime_location::get_pi_runtime_location_async(db)
        .await?
        .host_path;
    let path = root_dir.join(file_name);
    Ok(path.exists().then_some(path))
}

fn backup_filter_option_path(tool: &str, relative_path: &str) -> Option<String> {
    let normalized_path = normalize_restore_entry_name(relative_path);
    let relative_path = normalized_path.trim().trim_start_matches('/');
    if relative_path.is_empty() || relative_path == "root-dir.txt" {
        return None;
    }

    let file_path = match tool {
        "opencode" if relative_path == "auth.json" => {
            format!("~/.local/share/opencode/{relative_path}")
        }
        "opencode" => format!("~/.config/opencode/{relative_path}"),
        "claude" if relative_path == ".claude.json" => "~/.claude.json".to_string(),
        "claude" => format!("~/.claude/{relative_path}"),
        "codex" => format!("~/.codex/{relative_path}"),
        "grok" => format!("~/.grok/{relative_path}"),
        "openclaw" => format!("~/.openclaw/{relative_path}"),
        "geminicli" => format!("~/.gemini/{relative_path}"),
        "pi" => format!("~/.pi/agent/{relative_path}"),
        _ => relative_path.to_string(),
    };

    Some(file_path)
}

fn push_backup_filter_option(
    options: &mut Vec<BackupFileFilterPathOption>,
    seen: &mut HashSet<(String, String)>,
    tool: &str,
    relative_path: &str,
) {
    let Some(file_path) = backup_filter_option_path(tool, relative_path) else {
        return;
    };
    let key = (tool.to_string(), file_path.clone());
    if seen.insert(key) {
        options.push(BackupFileFilterPathOption {
            tool: tool.to_string(),
            file_path,
        });
    }
}

fn push_backup_filter_option_for_path(
    options: &mut Vec<BackupFileFilterPathOption>,
    seen: &mut HashSet<(String, String)>,
    tool: &str,
    file_path: &Path,
) {
    if let Some(file_name) = file_path.file_name().and_then(|name| name.to_str()) {
        push_backup_filter_option(options, seen, tool, file_name);
    }
}

/// List selectable file filter paths based on files that would currently be
/// written under external-configs/<tool>/ in backup archives.
pub async fn list_backup_file_filter_path_options(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<BackupFileFilterPathOption>, String> {
    let mut options = Vec::new();
    let mut seen = HashSet::new();

    if let Some(path) = get_opencode_config_path_from_db(db).await? {
        push_backup_filter_option_for_path(&mut options, &mut seen, "opencode", &path);
    }
    if get_opencode_auth_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "opencode", "auth.json");
    }
    if get_opencode_prompt_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "opencode", "AGENTS.md");
    }

    if get_claude_settings_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "claude", "settings.json");
    }
    if get_claude_prompt_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "claude", "CLAUDE.md");
    }
    if get_claude_mcp_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "claude", ".claude.json");
    }

    if get_codex_auth_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "codex", "auth.json");
    }
    if get_codex_config_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "codex", "config.toml");
    }
    for prompt_path in get_codex_prompt_paths_from_db(db).await? {
        push_backup_filter_option_for_path(&mut options, &mut seen, "codex", &prompt_path);
    }
    if get_grok_auth_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "grok", "auth.json");
    }
    if get_grok_config_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "grok", "config.toml");
    }
    if get_grok_prompt_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "grok", "AGENTS.md");
    }

    if get_openclaw_config_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "openclaw", "openclaw.json");
    }

    if get_gemini_cli_env_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "geminicli", ".env");
    }
    if get_gemini_cli_settings_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "geminicli", "settings.json");
    }
    if let Some(prompt_path) = get_gemini_cli_prompt_path_from_db(db).await? {
        push_backup_filter_option_for_path(&mut options, &mut seen, "geminicli", &prompt_path);
    }
    if get_gemini_cli_oauth_creds_path_from_db(db).await?.is_some() {
        push_backup_filter_option(&mut options, &mut seen, "geminicli", "oauth_creds.json");
    }
    if let Some(tmp_dir) = get_gemini_cli_tmp_dir_from_db(db).await? {
        for entry in WalkDir::new(&tmp_dir) {
            let entry = entry.map_err(|e| format!("Failed to read Gemini CLI tmp entry: {}", e))?;
            let path = entry.path();
            if !path.is_file() || should_skip_system_file(path) {
                continue;
            }

            let relative_path = path
                .strip_prefix(&tmp_dir)
                .map_err(|e| format!("Failed to get Gemini CLI tmp relative path: {}", e))?
                .to_string_lossy()
                .replace('\\', "/");
            push_backup_filter_option(
                &mut options,
                &mut seen,
                "geminicli",
                &format!("tmp/{relative_path}"),
            );
        }
    }

    for file_name in [
        "settings.json",
        "auth.json",
        "models.json",
        "AGENTS.md",
        "SYSTEM.md",
        "APPEND_SYSTEM.md",
        "trust.json",
    ] {
        if get_pi_runtime_file_path_from_db(db, file_name)
            .await?
            .is_some()
        {
            push_backup_filter_option(&mut options, &mut seen, "pi", file_name);
        }
    }

    options.sort_by(|a, b| a.tool.cmp(&b.tool).then(a.file_path.cmp(&b.file_path)));
    Ok(options)
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
        if output_path.exists()
            && std::fs::symlink_metadata(&output_path)
                .map_err(|error| {
                    format!(
                        "Failed to inspect external config restore path {}: {error}",
                        output_path.display()
                    )
                })?
                .file_type()
                .is_symlink()
        {
            return Err(format!(
                "External config restore path contains a symlink: {}",
                output_path.display()
            ));
        }
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
    db: &crate::db::SqliteDbState,
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
        "grok" => {
            let location = runtime_location::get_grok_runtime_location_async(db)
                .await
                .ok()?;
            (location.source == "custom").then(|| location.host_path.to_string_lossy().to_string())
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
        "pi" => {
            let location = runtime_location::get_pi_runtime_location_async(db)
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
    let sqlite_state = app_handle.state::<crate::SqliteDbState>();
    resolve_central_repo_path_sync(app_handle, &sqlite_state)
        .map_err(|error| format!("Failed to get skills directory: {error:#}"))
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

/// Get model_pricing.json cache file path if it exists
pub fn get_model_pricing_cache_file() -> Option<PathBuf> {
    crate::db::model_pricing_seed::get_model_pricing_cache_path().filter(|p| p.exists())
}

/// Get gateway_provider_profiles.json cache file path if it exists
pub fn get_gateway_provider_profiles_cache_file() -> Option<PathBuf> {
    crate::coding::proxy_gateway::provider_profiles::get_gateway_provider_profiles_cache_path()
        .filter(|p| p.exists())
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
    db: &crate::db::SqliteDbState,
) -> Result<Vec<BackupCustomEntry>, String> {
    Ok(crate::settings::store::load_settings_from_sqlite_state(db)?.backup_custom_entries)
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
    db: &crate::db::SqliteDbState,
) -> Result<bool, String> {
    Ok(crate::settings::store::load_settings_from_sqlite_state(db)?.backup_image_assets_enabled)
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

fn normalize_zip_directory_path(zip_path: &str) -> String {
    let normalized_path = zip_path.replace('\\', "/");
    if normalized_path.ends_with('/') {
        normalized_path
    } else {
        format!("{normalized_path}/")
    }
}

fn add_directory_to_zip_once<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    added_directories: &mut HashSet<String>,
    zip_path: &str,
    options: SimpleFileOptions,
    context: &str,
) -> Result<(), String> {
    let normalized_path = normalize_zip_directory_path(zip_path);
    if !added_directories.insert(normalized_path.clone()) {
        return Ok(());
    }

    zip.add_directory(normalized_path, options)
        .map_err(|e| format!("Failed to add {context}: {}", e))
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

fn add_legacy_database_snapshot_to_zip<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    db_path: &Path,
    options: SimpleFileOptions,
) -> Result<(), String> {
    let has_files = add_legacy_database_files_to_zip(zip, db_path, options)?;

    if !has_files {
        zip.start_file("db/.backup_marker", options)
            .map_err(|e| format!("Failed to create marker file: {}", e))?;
        zip.write_all(b"AI Toolbox Backup")
            .map_err(|e| format!("Failed to write marker: {}", e))?;
    }

    Ok(())
}

fn add_legacy_database_files_to_zip<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    db_path: &Path,
    options: SimpleFileOptions,
) -> Result<bool, String> {
    if !db_path.exists() {
        return Ok(false);
    }

    if !db_path.is_dir() {
        return Err(format!(
            "Database path is not a directory: {}",
            db_path.display()
        ));
    }

    let mut has_files = false;

    // Add database files under db/ prefix
    for entry in WalkDir::new(db_path) {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let path = entry.path();
        let relative_path = path
            .strip_prefix(db_path)
            .map_err(|e| format!("Failed to get relative path: {}", e))?;

        if path.is_file() {
            // Skip system files and the empty legacy backup marker.
            if let Some(file_name) = path.file_name() {
                let name_str = file_name.to_string_lossy();
                if name_str == ".DS_Store"
                    || name_str.starts_with("._")
                    || name_str == ".backup_marker"
                {
                    continue;
                }
            }

            has_files = true;
            // Use forward slashes for cross-platform compatibility in zip files
            let relative_str = relative_path.to_string_lossy().replace('\\', "/");
            let name = format!("db/{}", relative_str);
            zip.start_file(name, options)
                .map_err(|e| format!("Failed to start file in zip: {}", e))?;

            let mut file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
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

    Ok(has_files)
}

fn normalize_backup_filter_rule_path(tool: &str, file_path: &str) -> String {
    let normalized_path = normalize_restore_entry_name(file_path);
    let normalized_path = normalized_path.trim().trim_start_matches('/');
    let external_config_prefix = format!("external-configs/{}/", tool);
    let path = normalized_path
        .strip_prefix(&external_config_prefix)
        .unwrap_or(normalized_path);

    let tool_prefixes: &[&str] = match tool {
        "opencode" => &["~/.config/opencode/", "~/.local/share/opencode/"],
        "claude" => &["~/.claude/"],
        "codex" => &["~/.codex/"],
        "grok" => &["~/.grok/"],
        "openclaw" => &["~/.openclaw/"],
        "geminicli" => &["~/.gemini/"],
        "pi" => &["~/.pi/agent/"],
        _ => &[],
    };

    if tool == "claude" && path == "~/.claude.json" {
        return ".claude.json".to_string();
    }

    for prefix in tool_prefixes {
        if let Some(relative_path) = path.strip_prefix(prefix) {
            return relative_path.trim_start_matches('/').to_string();
        }
    }

    path.to_string()
}

/// Check if a file should be excluded from backup/restore based on filter rules
pub fn should_exclude_from_backup(
    rules: &[BackupFileFilterRule],
    tool: &str,
    file_path: &str,
) -> bool {
    let normalized_file_path = normalize_backup_filter_rule_path(tool, file_path);
    rules.iter().any(|r| {
        r.tool == tool
            && normalize_backup_filter_rule_path(tool, &r.file_path) == normalized_file_path
    })
}

/// Check whether an external-configs entry should be filtered.
///
/// `relative_path` is the path inside one tool's config backup directory,
/// such as `auth.json`, `settings.json`, or `tmp/cache.json`.
pub fn should_filter_external_config_entry(
    rules: &[BackupFileFilterRule],
    tool: &str,
    relative_path: &str,
) -> bool {
    let normalized_path = normalize_restore_entry_name(relative_path);
    let file_name = normalized_path.trim_start_matches('/').trim();
    if file_name.is_empty() || file_name == "root-dir.txt" {
        return false;
    }

    should_exclude_from_backup(rules, tool, file_name)
}

fn add_external_config_file_to_zip<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    added_zip_directories: &mut HashSet<String>,
    file_path: &Path,
    tool: &str,
    relative_path: &str,
    filter_rules: &[BackupFileFilterRule],
    options: SimpleFileOptions,
) -> Result<(), String> {
    if should_filter_external_config_entry(filter_rules, tool, relative_path) {
        return Ok(());
    }

    let tool_directory = format!("external-configs/{}/", tool);
    add_directory_to_zip_once(
        zip,
        added_zip_directories,
        &tool_directory,
        options,
        &format!("{} directory", tool),
    )?;

    let zip_path = format!("{}{}", tool_directory, relative_path);
    add_file_to_zip(zip, file_path, &zip_path, options)
}

fn add_external_config_directory_contents_to_zip<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    source_dir: &Path,
    tool: &str,
    relative_prefix: &str,
    filter_rules: &[BackupFileFilterRule],
    options: SimpleFileOptions,
) -> Result<(), String> {
    if !source_dir.is_dir() {
        return Ok(());
    }

    let normalized_relative_prefix = relative_prefix.trim_matches('/');
    let zip_prefix = format!("external-configs/{}/{}", tool, normalized_relative_prefix);
    let normalized_zip_prefix = zip_prefix.trim_end_matches('/');
    zip.add_directory(format!("{}/", normalized_zip_prefix), options)
        .map_err(|e| format!("Failed to add directory to zip: {}", e))?;

    for entry in WalkDir::new(source_dir) {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let path = entry.path();
        let relative_path = path
            .strip_prefix(source_dir)
            .map_err(|e| format!("Failed to get relative path: {}", e))?;

        let relative_str = relative_path.to_string_lossy().replace('\\', "/");
        let tool_relative_path = if normalized_relative_prefix.is_empty() {
            relative_str.clone()
        } else {
            format!("{}/{}", normalized_relative_prefix, relative_str)
        };

        if tool == "grok"
            && normalized_relative_prefix == "plugins"
            && relative_path.components().any(|component| {
                matches!(
                    component.as_os_str().to_string_lossy().as_ref(),
                    ".git" | "node_modules" | "cache" | ".cache" | "build" | "dist" | "target"
                )
            })
        {
            continue;
        }

        if path.is_file() {
            if let Some(file_name) = path.file_name() {
                let name_str = file_name.to_string_lossy();
                if name_str == ".DS_Store" || name_str.starts_with("._") {
                    continue;
                }
            }
            if should_filter_external_config_entry(filter_rules, tool, &tool_relative_path) {
                continue;
            }

            let name = format!("{}/{}", normalized_zip_prefix, relative_str);
            add_file_to_zip(zip, path, &name, options)?;
        } else if path.is_dir() && !relative_path.as_os_str().is_empty() {
            let name = format!("{}/{}/", normalized_zip_prefix, relative_str);
            zip.add_directory(name, options)
                .map_err(|e| format!("Failed to add subdirectory to zip: {}", e))?;
        }
    }

    Ok(())
}

pub(crate) async fn write_backup_zip_contents<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    app_handle: &tauri::AppHandle,
    db_path: &Path,
    include_image_assets: bool,
    filter_rules: &[BackupFileFilterRule],
    options: SimpleFileOptions,
) -> Result<(), String> {
    let db_state = app_handle.state::<crate::SqliteDbState>();
    let db = db_state.db();
    let mut added_zip_directories = HashSet::new();

    add_legacy_database_snapshot_to_zip(zip, db_path, options)?;

    add_sqlite_database_snapshot_to_zip(zip, app_handle, options)?;

    // Add external-configs directory
    add_directory_to_zip_once(
        zip,
        &mut added_zip_directories,
        "external-configs/",
        options,
        "external-configs directory",
    )?;

    if let Some(custom_dir) = get_custom_root_dir_path_info(&db, "opencode").await {
        add_directory_to_zip_once(
            zip,
            &mut added_zip_directories,
            "external-configs/opencode/",
            options,
            "opencode directory",
        )?;
        add_text_to_zip(
            zip,
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
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &opencode_path,
            "opencode",
            &file_name,
            filter_rules,
            options,
        )?;
    }

    // Backup OpenCode auth.json if exists (check filter rules)
    if let Some(opencode_auth_path) = get_opencode_auth_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &opencode_auth_path,
            "opencode",
            "auth.json",
            filter_rules,
            options,
        )?;
    }

    if let Some(opencode_prompt_path) = get_opencode_prompt_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &opencode_prompt_path,
            "opencode",
            "AGENTS.md",
            filter_rules,
            options,
        )?;
    }

    if let Some(custom_root_dir) = get_custom_root_dir_path_info(&db, "claude").await {
        add_directory_to_zip_once(
            zip,
            &mut added_zip_directories,
            "external-configs/claude/",
            options,
            "claude directory",
        )?;
        add_text_to_zip(
            zip,
            "external-configs/claude/root-dir.txt",
            &custom_root_dir,
            options,
        )?;
    }

    // Backup Claude settings.json if exists
    if let Some(claude_path) = get_claude_settings_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &claude_path,
            "claude",
            "settings.json",
            filter_rules,
            options,
        )?;
    }

    if let Some(claude_prompt_path) = get_claude_prompt_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &claude_prompt_path,
            "claude",
            "CLAUDE.md",
            filter_rules,
            options,
        )?;
    }

    if let Some(claude_mcp_path) = get_claude_mcp_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &claude_mcp_path,
            "claude",
            ".claude.json",
            filter_rules,
            options,
        )?;
    }

    if let Some(custom_root_dir) = get_custom_root_dir_path_info(&db, "codex").await {
        add_directory_to_zip_once(
            zip,
            &mut added_zip_directories,
            "external-configs/codex/",
            options,
            "codex directory",
        )?;
        add_text_to_zip(
            zip,
            "external-configs/codex/root-dir.txt",
            &custom_root_dir,
            options,
        )?;
    }

    // Backup Codex auth.json if exists (check filter rules)
    if let Some(codex_auth_path) = get_codex_auth_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &codex_auth_path,
            "codex",
            "auth.json",
            filter_rules,
            options,
        )?;
    }

    // Backup Codex config.toml if exists
    if let Some(codex_config_path) = get_codex_config_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &codex_config_path,
            "codex",
            "config.toml",
            filter_rules,
            options,
        )?;
    }

    for codex_prompt_path in get_codex_prompt_paths_from_db(&db).await? {
        let zip_path = get_codex_prompt_backup_zip_path(&codex_prompt_path);
        let relative_path = zip_path.trim_start_matches("external-configs/codex/");
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &codex_prompt_path,
            "codex",
            relative_path,
            filter_rules,
            options,
        )?;
    }

    if let Some(custom_dir) = get_custom_root_dir_path_info(&db, "grok").await {
        add_directory_to_zip_once(
            zip,
            &mut added_zip_directories,
            "external-configs/grok/",
            options,
            "grok directory",
        )?;
        add_text_to_zip(
            zip,
            "external-configs/grok/root-dir.txt",
            &custom_dir,
            options,
        )?;
    }
    if let Some(path) = get_grok_auth_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &path,
            "grok",
            "auth.json",
            filter_rules,
            options,
        )?;
    }
    if let Some(path) = get_grok_config_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &path,
            "grok",
            "config.toml",
            filter_rules,
            options,
        )?;
    }
    if let Some(path) = get_grok_prompt_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &path,
            "grok",
            "AGENTS.md",
            filter_rules,
            options,
        )?;
    }
    let grok_plugins_dir = runtime_location::get_grok_runtime_location_async(&db)
        .await?
        .host_path
        .join("plugins");
    add_external_config_directory_contents_to_zip(
        zip,
        &grok_plugins_dir,
        "grok",
        "plugins",
        filter_rules,
        options,
    )?;

    if let Some(custom_dir) = get_custom_root_dir_path_info(&db, "openclaw").await {
        add_directory_to_zip_once(
            zip,
            &mut added_zip_directories,
            "external-configs/openclaw/",
            options,
            "openclaw directory",
        )?;
        add_text_to_zip(
            zip,
            "external-configs/openclaw/root-dir.txt",
            &custom_dir,
            options,
        )?;
    }

    if let Some(openclaw_config_path) = get_openclaw_config_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &openclaw_config_path,
            "openclaw",
            "openclaw.json",
            filter_rules,
            options,
        )?;
    }

    if let Some(custom_root_dir) = get_custom_root_dir_path_info(&db, "geminicli").await {
        add_directory_to_zip_once(
            zip,
            &mut added_zip_directories,
            "external-configs/geminicli/",
            options,
            "Gemini CLI directory",
        )?;
        add_text_to_zip(
            zip,
            "external-configs/geminicli/root-dir.txt",
            &custom_root_dir,
            options,
        )?;
    }

    if let Some(gemini_env_path) = get_gemini_cli_env_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &gemini_env_path,
            "geminicli",
            ".env",
            filter_rules,
            options,
        )?;
    }

    if let Some(gemini_settings_path) = get_gemini_cli_settings_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &gemini_settings_path,
            "geminicli",
            "settings.json",
            filter_rules,
            options,
        )?;
    }

    if let Some(gemini_prompt_path) = get_gemini_cli_prompt_path_from_db(&db).await? {
        let zip_path = get_gemini_cli_prompt_backup_zip_path(&gemini_prompt_path);
        let relative_path = zip_path.trim_start_matches("external-configs/geminicli/");
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &gemini_prompt_path,
            "geminicli",
            relative_path,
            filter_rules,
            options,
        )?;
    }

    if let Some(gemini_oauth_path) = get_gemini_cli_oauth_creds_path_from_db(&db).await? {
        add_external_config_file_to_zip(
            zip,
            &mut added_zip_directories,
            &gemini_oauth_path,
            "geminicli",
            "oauth_creds.json",
            filter_rules,
            options,
        )?;
    }

    if let Some(gemini_tmp_dir) = get_gemini_cli_tmp_dir_from_db(&db).await? {
        add_external_config_directory_contents_to_zip(
            zip,
            &gemini_tmp_dir,
            "geminicli",
            "tmp",
            filter_rules,
            options,
        )?;
    }

    if let Some(custom_root_dir) = get_custom_root_dir_path_info(&db, "pi").await {
        add_directory_to_zip_once(
            zip,
            &mut added_zip_directories,
            "external-configs/pi/",
            options,
            "Pi directory",
        )?;
        add_text_to_zip(
            zip,
            "external-configs/pi/root-dir.txt",
            &custom_root_dir,
            options,
        )?;
    }

    for file_name in [
        pi::constants::PI_SETTINGS_FILE,
        pi::constants::PI_AUTH_FILE,
        pi::constants::PI_MODELS_FILE,
        pi::constants::PI_PROMPT_FILE,
        "SYSTEM.md",
        "APPEND_SYSTEM.md",
        "trust.json",
    ] {
        if let Some(path) = get_pi_runtime_file_path_from_db(&db, file_name).await? {
            add_external_config_file_to_zip(
                zip,
                &mut added_zip_directories,
                &path,
                "pi",
                file_name,
                filter_rules,
                options,
            )?;
        }
    }

    // Backup models.dev.json cache if exists
    if let Some(models_cache_path) = get_models_cache_file() {
        add_file_to_zip(zip, &models_cache_path, "models.dev.json", options)?;
    }

    // Backup preset_models.json cache if exists
    if let Some(preset_models_cache_path) = get_preset_models_cache_file() {
        add_file_to_zip(
            zip,
            &preset_models_cache_path,
            "preset_models.json",
            options,
        )?;
    }

    // Backup model_pricing.json cache if exists
    if let Some(model_pricing_cache_path) = get_model_pricing_cache_file() {
        add_file_to_zip(
            zip,
            &model_pricing_cache_path,
            "model_pricing.json",
            options,
        )?;
    }

    // Backup gateway_provider_profiles.json cache if exists
    if let Some(provider_profiles_cache_path) = get_gateway_provider_profiles_cache_file() {
        add_file_to_zip(
            zip,
            &provider_profiles_cache_path,
            "gateway_provider_profiles.json",
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
                add_file_to_zip(zip, path, &name, options)?;
            } else if path.is_dir() && !relative_path.as_os_str().is_empty() {
                let relative_str = relative_path.to_string_lossy().replace('\\', "/");
                let name = format!("skills/{}/", relative_str);
                zip.add_directory(name, options)
                    .map_err(|e| format!("Failed to add skills subdirectory: {}", e))?;
            }
        }
    }

    if include_image_assets {
        add_image_assets_to_zip(app_handle, zip, options)?;
    }

    let backup_custom_entries = get_backup_custom_entries_from_db(&db).await?;
    add_custom_backup_entries_to_zip(zip, &backup_custom_entries, options)?;

    Ok(())
}

/// Create a temporary backup zip file and return its contents as bytes
pub async fn create_backup_zip(
    app_handle: &tauri::AppHandle,
    db_path: &Path,
    include_image_assets: bool,
    filter_rules: &[BackupFileFilterRule],
) -> Result<Vec<u8>, String> {
    use std::io::Cursor;

    let mut buffer = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buffer);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        write_backup_zip_contents(
            &mut zip,
            app_handle,
            db_path,
            include_image_assets,
            filter_rules,
            options,
        )
        .await?;
        zip.finish()
            .map_err(|e| format!("Failed to finish zip: {}", e))?;
    }

    Ok(buffer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::{
        add_custom_backup_entries_to_zip, add_directory_to_zip_once,
        add_external_config_directory_contents_to_zip, add_external_config_file_to_zip,
        add_legacy_database_snapshot_to_zip, add_text_to_zip, build_db_manifest,
        get_codex_prompt_backup_zip_path, get_existing_codex_prompt_paths,
        get_gemini_cli_prompt_backup_zip_path, harden_restored_sensitive_file,
        is_filesystem_root_directory, normalize_backup_storage_path, normalize_restore_entry_name,
        resolve_external_config_restore_output_path, restore_custom_backup_entries,
        should_exclude_from_backup, should_filter_external_config_entry,
        CUSTOM_BACKUP_MANIFEST_PATH, SQLITE_BACKUP_ZIP_PATH,
    };
    use crate::settings::types::{BackupCustomEntry, BackupCustomEntryType, BackupFileFilterRule};
    use std::collections::HashSet;
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

    fn build_legacy_database_zip(db_path: &Path) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buffer);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            add_legacy_database_snapshot_to_zip(&mut zip, db_path, options)
                .expect("add legacy database snapshot");
            zip.finish().expect("finish zip");
        }
        buffer.into_inner()
    }

    #[test]
    fn duplicate_zip_directory_entries_are_written_once() {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buffer);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            let mut added_directories = HashSet::new();

            add_directory_to_zip_once(
                &mut zip,
                &mut added_directories,
                "external-configs/opencode",
                options,
                "opencode directory",
            )
            .expect("add opencode directory");
            add_text_to_zip(
                &mut zip,
                "external-configs/opencode/root-dir.txt",
                "/tmp/opencode",
                options,
            )
            .expect("add root dir marker");
            add_directory_to_zip_once(
                &mut zip,
                &mut added_directories,
                "external-configs/opencode/",
                options,
                "opencode directory",
            )
            .expect("skip duplicate opencode directory");

            zip.finish().expect("finish zip");
        }

        let archive = ZipArchive::new(Cursor::new(buffer.into_inner())).expect("zip archive");
        let opencode_directory_count = archive
            .file_names()
            .filter(|name| *name == "external-configs/opencode/")
            .count();

        assert_eq!(opencode_directory_count, 1);
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
    fn database_manifest_records_sqlite_snapshot() {
        let manifest = build_db_manifest("sqlite", 1);

        assert_eq!(
            manifest.get("engine").and_then(|value| value.as_str()),
            Some("sqlite")
        );
        assert_eq!(
            manifest
                .get("schema_version")
                .and_then(|value| value.as_i64()),
            Some(1)
        );
        assert_eq!(
            manifest.get("sqlite_path").and_then(|value| value.as_str()),
            Some(SQLITE_BACKUP_ZIP_PATH)
        );
        assert!(manifest.get("legacy_surrealdb_path").is_none());
    }

    #[test]
    fn legacy_database_snapshot_allows_missing_directory() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let missing_db_dir = temp_dir.path().join("database");

        let zip_data = build_legacy_database_zip(&missing_db_dir);
        let mut archive = ZipArchive::new(Cursor::new(zip_data)).expect("zip archive");
        let mut marker = String::new();
        archive
            .by_name("db/.backup_marker")
            .expect("backup marker")
            .read_to_string(&mut marker)
            .expect("read marker");

        assert_eq!(marker, "AI Toolbox Backup");
    }

    #[test]
    fn legacy_database_snapshot_skips_system_files() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let db_dir = temp_dir.path().join("database");
        fs::create_dir_all(db_dir.join("kv")).expect("create database dir");
        fs::write(db_dir.join(".DS_Store"), "finder").expect("write ds store");
        fs::write(db_dir.join("._metadata"), "metadata").expect("write apple metadata");
        fs::write(db_dir.join("kv").join("data"), "legacy").expect("write legacy data");

        let zip_data = build_legacy_database_zip(&db_dir);
        let mut archive = ZipArchive::new(Cursor::new(zip_data)).expect("zip archive");
        let mut legacy_data = String::new();
        archive
            .by_name("db/kv/data")
            .expect("legacy data")
            .read_to_string(&mut legacy_data)
            .expect("read legacy data");

        assert_eq!(legacy_data, "legacy");
        assert!(archive.by_name("db/.DS_Store").is_err());
        assert!(archive.by_name("db/._metadata").is_err());
        assert!(archive.by_name("db/.backup_marker").is_err());
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
    fn gemini_cli_prompt_backup_and_restore_paths_follow_context_file_name() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let gemini_root = temp_dir.path().join(".gemini");
        fs::create_dir_all(&gemini_root).expect("create gemini root");
        fs::write(
            gemini_root.join("settings.json"),
            serde_json::json!({
                "context": {
                    "fileName": "AGENTS.md"
                }
            })
            .to_string(),
        )
        .expect("write settings");
        fs::write(gemini_root.join("AGENTS.md"), "managed prompt").expect("write prompt");

        let prompt_path =
            crate::coding::gemini_cli::get_gemini_cli_prompt_path_from_root(&gemini_root);
        assert_eq!(prompt_path, gemini_root.join("AGENTS.md"));

        let zip_path = get_gemini_cli_prompt_backup_zip_path(&prompt_path);
        assert_eq!(zip_path, "external-configs/geminicli/AGENTS.md");

        let restore_root = temp_dir.path().join("restore-gemini");
        let restore_relative_path = zip_path.trim_start_matches("external-configs/geminicli/");
        let restored_path =
            resolve_external_config_restore_output_path(&restore_root, restore_relative_path)
                .expect("resolve restore path")
                .expect("path should exist");
        assert_eq!(restored_path, restore_root.join("AGENTS.md"));
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

    #[test]
    fn should_exclude_returns_true_for_matching_filter() {
        let rules = vec![BackupFileFilterRule {
            tool: "opencode".to_string(),
            file_path: "auth.json".to_string(),
        }];

        assert!(should_exclude_from_backup(&rules, "opencode", "auth.json"));
    }

    #[test]
    fn should_exclude_returns_false_for_different_tool() {
        let rules = vec![BackupFileFilterRule {
            tool: "opencode".to_string(),
            file_path: "auth.json".to_string(),
        }];

        assert!(!should_exclude_from_backup(&rules, "codex", "auth.json"));
    }

    #[test]
    fn should_exclude_returns_false_for_different_file() {
        let rules = vec![BackupFileFilterRule {
            tool: "opencode".to_string(),
            file_path: "auth.json".to_string(),
        }];

        assert!(!should_exclude_from_backup(
            &rules,
            "opencode",
            "settings.json"
        ));
    }

    #[test]
    fn should_exclude_returns_false_for_empty_rules() {
        let rules = vec![];

        assert!(!should_exclude_from_backup(&rules, "opencode", "auth.json"));
    }

    #[test]
    fn should_exclude_matches_any_matching_rule() {
        let rules = vec![
            BackupFileFilterRule {
                tool: "opencode".to_string(),
                file_path: "auth.json".to_string(),
            },
            BackupFileFilterRule {
                tool: "codex".to_string(),
                file_path: "auth.json".to_string(),
            },
        ];

        assert!(should_exclude_from_backup(&rules, "codex", "auth.json"));
    }

    #[test]
    fn should_exclude_matches_portable_tool_path() {
        let rules = vec![BackupFileFilterRule {
            tool: "codex".to_string(),
            file_path: "~/.codex/auth.json".to_string(),
        }];

        assert!(should_exclude_from_backup(&rules, "codex", "auth.json"));
        assert!(!should_exclude_from_backup(&rules, "opencode", "auth.json"));
    }

    #[test]
    fn should_exclude_matches_external_config_zip_path() {
        let rules = vec![BackupFileFilterRule {
            tool: "geminicli".to_string(),
            file_path: "external-configs/geminicli/settings.json".to_string(),
        }];

        assert!(should_exclude_from_backup(
            &rules,
            "geminicli",
            "settings.json"
        ));
    }

    #[test]
    fn should_exclude_maps_claude_home_mcp_path() {
        let rules = vec![BackupFileFilterRule {
            tool: "claude".to_string(),
            file_path: "~/.claude.json".to_string(),
        }];

        assert!(should_exclude_from_backup(&rules, "claude", ".claude.json"));
    }

    #[test]
    fn should_exclude_default_rules_filter_nothing() {
        let rules = crate::settings::types::default_backup_file_filter_rules();

        assert!(rules.is_empty());
        assert!(!should_exclude_from_backup(&rules, "opencode", "auth.json"));
        assert!(!should_exclude_from_backup(&rules, "codex", "auth.json"));
        assert!(!should_exclude_from_backup(&rules, "geminicli", ".env"));
        assert!(!should_exclude_from_backup(
            &rules,
            "geminicli",
            "oauth_creds.json"
        ));
    }

    #[test]
    fn should_exclude_explicit_rules_independent_per_tool() {
        let rules = vec![
            BackupFileFilterRule {
                tool: "opencode".to_string(),
                file_path: "auth.json".to_string(),
            },
            BackupFileFilterRule {
                tool: "codex".to_string(),
                file_path: "auth.json".to_string(),
            },
        ];

        assert!(should_exclude_from_backup(&rules, "opencode", "auth.json"));
        assert!(should_exclude_from_backup(&rules, "codex", "auth.json"));
    }

    #[test]
    fn should_filter_external_config_entry_matches_custom_non_default_file() {
        let rules = vec![BackupFileFilterRule {
            tool: "claude".to_string(),
            file_path: "settings.json".to_string(),
        }];

        assert!(should_filter_external_config_entry(
            &rules,
            "claude",
            "settings.json"
        ));
        assert!(!should_filter_external_config_entry(
            &rules,
            "claude",
            "CLAUDE.md"
        ));
    }

    #[test]
    fn should_filter_external_config_entry_matches_nested_relative_path() {
        let rules = vec![BackupFileFilterRule {
            tool: "geminicli".to_string(),
            file_path: "tmp/token.json".to_string(),
        }];

        assert!(should_filter_external_config_entry(
            &rules,
            "geminicli",
            "tmp/token.json"
        ));
        assert!(should_filter_external_config_entry(
            &rules,
            "geminicli",
            "\\tmp\\token.json"
        ));
        assert!(!should_filter_external_config_entry(
            &rules,
            "geminicli",
            "token.json"
        ));
    }

    #[test]
    fn should_filter_external_config_entry_never_filters_restore_metadata() {
        let rules = vec![BackupFileFilterRule {
            tool: "opencode".to_string(),
            file_path: "root-dir.txt".to_string(),
        }];

        assert!(!should_filter_external_config_entry(
            &rules,
            "opencode",
            "root-dir.txt"
        ));
    }

    #[test]
    fn external_config_file_filter_excludes_matching_zip_entry() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let auth_path = temp_dir.path().join("auth.json");
        let config_path = temp_dir.path().join("config.toml");
        fs::write(&auth_path, "{}").expect("write auth");
        fs::write(&config_path, "model = \"gpt-5\"").expect("write config");

        let rules = vec![BackupFileFilterRule {
            tool: "codex".to_string(),
            file_path: "~/.codex/auth.json".to_string(),
        }];

        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buffer);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            let mut added_directories = HashSet::new();

            add_external_config_file_to_zip(
                &mut zip,
                &mut added_directories,
                &auth_path,
                "codex",
                "auth.json",
                &rules,
                options,
            )
            .expect("filtered auth");
            add_external_config_file_to_zip(
                &mut zip,
                &mut added_directories,
                &config_path,
                "codex",
                "config.toml",
                &rules,
                options,
            )
            .expect("kept config");
            zip.finish().expect("finish zip");
        }

        let archive = ZipArchive::new(Cursor::new(buffer.into_inner())).expect("zip archive");
        let names: Vec<_> = archive.file_names().map(|name| name.to_string()).collect();

        assert!(!names
            .iter()
            .any(|name| name == "external-configs/codex/auth.json"));
        assert!(names
            .iter()
            .any(|name| name == "external-configs/codex/config.toml"));
    }

    #[test]
    fn external_config_directory_filter_excludes_matching_nested_zip_entry() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let tmp_dir = temp_dir.path().join("tmp");
        fs::create_dir_all(&tmp_dir).expect("create tmp");
        fs::write(tmp_dir.join("token.json"), "{}").expect("write token");
        fs::write(tmp_dir.join("cache.json"), "{}").expect("write cache");

        let rules = vec![BackupFileFilterRule {
            tool: "geminicli".to_string(),
            file_path: "~/.gemini/tmp/token.json".to_string(),
        }];

        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buffer);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            add_external_config_directory_contents_to_zip(
                &mut zip,
                &tmp_dir,
                "geminicli",
                "tmp",
                &rules,
                options,
            )
            .expect("add tmp directory");
            zip.finish().expect("finish zip");
        }

        let archive = ZipArchive::new(Cursor::new(buffer.into_inner())).expect("zip archive");
        let names: Vec<_> = archive.file_names().map(|name| name.to_string()).collect();

        assert!(!names
            .iter()
            .any(|name| name == "external-configs/geminicli/tmp/token.json"));
        assert!(names
            .iter()
            .any(|name| name == "external-configs/geminicli/tmp/cache.json"));
    }

    #[test]
    fn grok_plugin_backup_excludes_generated_artifacts() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let plugins_dir = temp_dir.path().join("plugins");
        fs::create_dir_all(plugins_dir.join("sample").join("node_modules"))
            .expect("create node_modules");
        fs::create_dir_all(plugins_dir.join("sample").join(".git")).expect("create git metadata");
        fs::create_dir_all(plugins_dir.join("sample").join("dist")).expect("create dist");
        fs::write(plugins_dir.join("sample").join("plugin.json"), "{}")
            .expect("write plugin manifest");
        fs::write(
            plugins_dir
                .join("sample")
                .join("node_modules")
                .join("cache.js"),
            "generated",
        )
        .expect("write generated dependency");
        fs::write(plugins_dir.join("sample").join(".git").join("HEAD"), "ref")
            .expect("write git metadata");
        fs::write(
            plugins_dir.join("sample").join("dist").join("bundle.js"),
            "built",
        )
        .expect("write build output");

        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buffer);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            add_external_config_directory_contents_to_zip(
                &mut zip,
                &plugins_dir,
                "grok",
                "plugins",
                &[],
                options,
            )
            .expect("add Grok plugins");
            zip.finish().expect("finish zip");
        }

        let archive = ZipArchive::new(Cursor::new(buffer.into_inner())).expect("zip archive");
        let names = archive.file_names().map(str::to_string).collect::<Vec<_>>();
        assert!(names
            .iter()
            .any(|name| name == "external-configs/grok/plugins/sample/plugin.json"));
        assert!(!names.iter().any(|name| name.contains("node_modules")));
        assert!(!names.iter().any(|name| name.contains("/.git/")));
        assert!(!names.iter().any(|name| name.contains("/dist/")));
    }

    /// Helper: simulate restore filtering logic for a given file path
    fn should_skip_restore_entry(
        filter_rules: &[BackupFileFilterRule],
        zip_entry_name: &str,
    ) -> bool {
        let file_name = normalize_restore_entry_name(zip_entry_name);
        for tool in [
            "opencode",
            "claude",
            "codex",
            "grok",
            "openclaw",
            "geminicli",
        ] {
            let prefix = format!("external-configs/{}/", tool);
            if let Some(relative_path) = file_name.strip_prefix(&prefix) {
                return should_filter_external_config_entry(filter_rules, tool, relative_path);
            }
        }

        false
    }

    #[test]
    fn restore_filter_skips_opencode_auth_json_when_rule_exists() {
        let rules = vec![BackupFileFilterRule {
            tool: "opencode".to_string(),
            file_path: "auth.json".to_string(),
        }];

        assert!(should_skip_restore_entry(
            &rules,
            "external-configs/opencode/auth.json"
        ));
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/opencode/opencode.json"
        ));
    }

    #[test]
    fn restore_filter_skips_codex_auth_json_when_rule_exists() {
        let rules = vec![BackupFileFilterRule {
            tool: "codex".to_string(),
            file_path: "auth.json".to_string(),
        }];

        assert!(should_skip_restore_entry(
            &rules,
            "external-configs/codex/auth.json"
        ));
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/codex/config.toml"
        ));
    }

    #[test]
    fn restore_filter_skips_grok_auth_json_when_rule_exists() {
        let rules = vec![BackupFileFilterRule {
            tool: "grok".to_string(),
            file_path: "auth.json".to_string(),
        }];

        assert!(should_skip_restore_entry(
            &rules,
            "external-configs/grok/auth.json"
        ));
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/grok/config.toml"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn restored_auth_files_are_hardened_to_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let auth_path = temp_dir.path().join("auth.json");
        fs::write(&auth_path, "{}").expect("write auth file");
        fs::set_permissions(&auth_path, fs::Permissions::from_mode(0o644))
            .expect("set initial permissions");

        harden_restored_sensitive_file(&auth_path).expect("harden auth file");

        let mode = fs::metadata(&auth_path)
            .expect("read auth metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn external_config_restore_path_rejects_existing_symlink_components() {
        use std::os::unix::fs::symlink;

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let restore_dir = temp_dir.path().join("restore");
        let outside_dir = temp_dir.path().join("outside");
        fs::create_dir_all(&restore_dir).expect("create restore dir");
        fs::create_dir_all(&outside_dir).expect("create outside dir");
        symlink(&outside_dir, restore_dir.join("plugins")).expect("create symlink");

        let error =
            resolve_external_config_restore_output_path(&restore_dir, "plugins/sample/plugin.json")
                .expect_err("symlink component must be rejected");
        assert!(error.contains("contains a symlink"));
    }

    #[test]
    fn restore_filter_skips_geminicli_env_when_rule_exists() {
        let rules = vec![BackupFileFilterRule {
            tool: "geminicli".to_string(),
            file_path: ".env".to_string(),
        }];

        assert!(should_skip_restore_entry(
            &rules,
            "external-configs/geminicli/.env"
        ));
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/geminicli/settings.json"
        ));
    }

    #[test]
    fn restore_filter_skips_geminicli_oauth_creds_when_rule_exists() {
        let rules = vec![BackupFileFilterRule {
            tool: "geminicli".to_string(),
            file_path: "oauth_creds.json".to_string(),
        }];

        assert!(should_skip_restore_entry(
            &rules,
            "external-configs/geminicli/oauth_creds.json"
        ));
    }

    #[test]
    fn restore_filter_does_not_skip_unrelated_files() {
        let rules = vec![BackupFileFilterRule {
            tool: "opencode".to_string(),
            file_path: "auth.json".to_string(),
        }];

        // Different tool same file
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/codex/auth.json"
        ));
        // Same tool different file
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/opencode/opencode.json"
        ));
        // Completely unrelated
        assert!(!should_skip_restore_entry(&rules, "sqlite/ai-toolbox.db"));
        assert!(!should_skip_restore_entry(
            &rules,
            "skills/my-skill/SKILL.md"
        ));
    }

    #[test]
    fn restore_filter_skips_custom_claude_settings_when_rule_exists() {
        let rules = vec![BackupFileFilterRule {
            tool: "claude".to_string(),
            file_path: "settings.json".to_string(),
        }];

        assert!(should_skip_restore_entry(
            &rules,
            "external-configs/claude/settings.json"
        ));
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/claude/CLAUDE.md"
        ));
    }

    #[test]
    fn restore_filter_with_default_rules_skips_nothing() {
        let rules = crate::settings::types::default_backup_file_filter_rules();

        assert!(rules.is_empty());
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/opencode/auth.json"
        ));
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/codex/auth.json"
        ));
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/geminicli/.env"
        ));
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/geminicli/oauth_creds.json"
        ));

        // Unmatched files should NOT be skipped
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/opencode/opencode.json"
        ));
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/claude/settings.json"
        ));
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/codex/config.toml"
        ));
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/openclaw/openclaw.json"
        ));
        assert!(!should_skip_restore_entry(
            &rules,
            "external-configs/geminicli/settings.json"
        ));
    }
}
