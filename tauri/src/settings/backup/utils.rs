use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::coding::open_code::shell_env;

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

/// Get Claude settings.json path if it exists
pub fn get_claude_settings_path() -> Result<Option<PathBuf>, String> {
    let home_dir = get_home_dir()?;
    let settings_path = home_dir.join(".claude").join("settings.json");

    if settings_path.exists() {
        Ok(Some(settings_path))
    } else {
        Ok(None)
    }
}

/// Get Codex auth.json path if it exists
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

/// Get Codex auth.json path if it exists
pub fn get_codex_auth_path() -> Result<Option<PathBuf>, String> {
    let home_dir = get_home_dir()?;
    let auth_path = home_dir.join(".codex").join("auth.json");

    if auth_path.exists() {
        Ok(Some(auth_path))
    } else {
        Ok(None)
    }
}

/// Get Codex config.toml path if it exists
pub fn get_codex_config_path() -> Result<Option<PathBuf>, String> {
    let home_dir = get_home_dir()?;
    let config_path = home_dir.join(".codex").join("config.toml");

    if config_path.exists() {
        Ok(Some(config_path))
    } else {
        Ok(None)
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

/// Create a temporary backup zip file and return its contents as bytes
pub fn create_backup_zip(app_handle: &tauri::AppHandle, db_path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Cursor;

    let mut buffer = Cursor::new(Vec::new());

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

        // Backup OpenCode config if exists
        if let Some(opencode_path) = get_opencode_config_path()? {
            let file_name = opencode_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("opencode.json");
            let zip_path = format!("external-configs/opencode/{}", file_name);

            zip.add_directory("external-configs/opencode/", options)
                .map_err(|e| format!("Failed to add opencode directory: {}", e))?;

            add_file_to_zip(&mut zip, &opencode_path, &zip_path, options)?;
        }

        // Backup OpenCode auth.json if exists
        if let Some(opencode_auth_path) = get_opencode_auth_path()? {
            let zip_path = "external-configs/opencode/auth.json";

            // Directory may already exist from opencode config backup
            let _ = zip.add_directory("external-configs/opencode/", options);

            add_file_to_zip(&mut zip, &opencode_auth_path, zip_path, options)?;
        }

        // Backup Claude settings.json if exists
        if let Some(claude_path) = get_claude_settings_path()? {
            let zip_path = "external-configs/claude/settings.json";

            zip.add_directory("external-configs/claude/", options)
                .map_err(|e| format!("Failed to add claude directory: {}", e))?;

            add_file_to_zip(&mut zip, &claude_path, zip_path, options)?;
        }

        // Backup Codex auth.json if exists
        if let Some(codex_auth_path) = get_codex_auth_path()? {
            let zip_path = "external-configs/codex/auth.json";

            zip.add_directory("external-configs/codex/", options)
                .map_err(|e| format!("Failed to add codex directory: {}", e))?;

            add_file_to_zip(&mut zip, &codex_auth_path, zip_path, options)?;
        }

        // Backup Codex config.toml if exists
        if let Some(codex_config_path) = get_codex_config_path()? {
            let zip_path = "external-configs/codex/config.toml";

            // Directory may already exist from auth.json backup
            let _ = zip.add_directory("external-configs/codex/", options);

            add_file_to_zip(&mut zip, &codex_config_path, zip_path, options)?;
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

        zip.finish()
            .map_err(|e| format!("Failed to finish zip: {}", e))?;
    }

    Ok(buffer.into_inner())
}
