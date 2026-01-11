use chrono::Local;
use std::fs;
use std::path::PathBuf;
use zip::ZipArchive;

use super::utils::{create_backup_zip, get_db_path, get_opencode_restore_dir};
use crate::db::DbState;
use crate::http_client;

/// Backup database to WebDAV server
#[tauri::command]
pub async fn backup_to_webdav(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    url: String,
    username: String,
    password: String,
    remote_path: String,
) -> Result<String, String> {
    let db_path = get_db_path(&app_handle)?;

    // Ensure database directory exists
    if !db_path.exists() {
        fs::create_dir_all(&db_path)
            .map_err(|e| format!("Failed to create database dir: {}", e))?;
    }

    // Create backup zip in memory
    let zip_data = create_backup_zip(&db_path)?;

    // Generate backup filename with timestamp
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let backup_filename = format!("ai-toolbox-backup-{}.zip", timestamp);

    // Build WebDAV URL
    let base_url = url.trim_end_matches('/');
    let remote = remote_path.trim_matches('/');
    let full_url = if remote.is_empty() {
        format!("{}/{}", base_url, backup_filename)
    } else {
        format!("{}/{}/{}", base_url, remote, backup_filename)
    };

    // Upload to WebDAV using PUT request with proxy support
    let client = http_client::client(&state).await?;
    let response = client
        .put(&full_url)
        .basic_auth(&username, Some(&password))
        .body(zip_data)
        .send()
        .await
        .map_err(|e| format!("Failed to upload to WebDAV: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "WebDAV upload failed with status: {}",
            response.status()
        ));
    }

    Ok(full_url)
}

/// List backup files from WebDAV server
#[tauri::command]
pub async fn list_webdav_backups(
    state: tauri::State<'_, DbState>,
    url: String,
    username: String,
    password: String,
    remote_path: String,
) -> Result<Vec<String>, String> {
    // Build WebDAV URL
    let base_url = url.trim_end_matches('/');
    let remote = remote_path.trim_matches('/');
    let folder_url = if remote.is_empty() {
        format!("{}/", base_url)
    } else {
        format!("{}/{}/", base_url, remote)
    };

    // Send PROPFIND request to list files with proxy support
    let client = http_client::client(&state).await?;
    let response = client
        .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &folder_url)
        .basic_auth(&username, Some(&password))
        .header("Depth", "1")
        .send()
        .await
        .map_err(|e| format!("Failed to list WebDAV files: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "WebDAV list failed with status: {}",
            response.status()
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    // Parse XML response to extract backup files
    // WebDAV returns XML like: <D:href>/path/to/ai-toolbox-backup-20250101-120000.zip</D:href>
    // Use regex to extract filenames from href tags
    use regex::Regex;
    let re = Regex::new(r"ai-toolbox-backup-\d{8}-\d{6}\.zip").unwrap();

    let mut backups = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for cap in re.find_iter(&body) {
        let filename = cap.as_str();
        if seen.insert(filename.to_string()) {
            backups.push(filename.to_string());
        }
    }

    backups.sort();
    backups.reverse(); // Most recent first
    Ok(backups)
}

/// Get home directory
fn get_home_dir() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .map_err(|_| "Failed to get home directory".to_string())
}

/// Restore database from WebDAV server
#[tauri::command]
pub async fn restore_from_webdav(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    url: String,
    username: String,
    password: String,
    remote_path: String,
    filename: String,
) -> Result<(), String> {
    let db_path = get_db_path(&app_handle)?;

    // Build WebDAV URL
    let base_url = url.trim_end_matches('/');
    let remote = remote_path.trim_matches('/');
    let full_url = if remote.is_empty() {
        format!("{}/{}", base_url, filename)
    } else {
        format!("{}/{}/{}", base_url, remote, filename)
    };

    // Download from WebDAV with proxy support
    let client = http_client::client(&state).await?;
    let response = client
        .get(&full_url)
        .basic_auth(&username, Some(&password))
        .send()
        .await
        .map_err(|e| format!("Failed to download from WebDAV: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "WebDAV download failed with status: {}",
            response.status()
        ));
    }

    let zip_data = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    // Extract zip contents
    let cursor = std::io::Cursor::new(zip_data);
    let mut archive =
        ZipArchive::new(cursor).map_err(|e| format!("Failed to read zip archive: {}", e))?;

    // Check if this is a new format backup (with db/ prefix) or old format
    let is_new_format = (0..archive.len()).any(|i| {
        archive
            .by_index(i)
            .map(|f| f.name().starts_with("db/"))
            .unwrap_or(false)
    });

    // Remove existing database directory
    if db_path.exists() {
        fs::remove_dir_all(&db_path)
            .map_err(|e| format!("Failed to remove existing database: {}", e))?;
    }

    // Create database directory
    fs::create_dir_all(&db_path)
        .map_err(|e| format!("Failed to create database directory: {}", e))?;

    // Get home directory for external configs
    let home_dir = get_home_dir()?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry: {}", e))?;

        let file_name = file.name().to_string();

        // Skip backup marker file
        if file_name == ".backup_marker" || file_name == "db/.backup_marker" {
            continue;
        }

        // Handle files based on new or old format
        if is_new_format {
            if file_name.starts_with("db/") {
                // Database files
                let relative_path = &file_name[3..]; // Remove "db/" prefix
                if relative_path.is_empty() {
                    continue;
                }

                let outpath = db_path.join(relative_path);

                if file_name.ends_with('/') {
                    fs::create_dir_all(&outpath)
                        .map_err(|e| format!("Failed to create directory: {}", e))?;
                } else {
                    if let Some(parent) = outpath.parent() {
                        if !parent.exists() {
                            fs::create_dir_all(parent)
                                .map_err(|e| format!("Failed to create parent directory: {}", e))?;
                        }
                    }
                    let mut outfile = std::fs::File::create(&outpath)
                        .map_err(|e| format!("Failed to create file: {}", e))?;
                    std::io::copy(&mut file, &mut outfile)
                        .map_err(|e| format!("Failed to extract file: {}", e))?;
                }
            } else if file_name.starts_with("external-configs/opencode/") {
                // OpenCode config - restore to appropriate directory based on env/shell/default
                let relative_path = &file_name[26..]; // Remove "external-configs/opencode/" prefix
                if relative_path.is_empty() || file_name.ends_with('/') {
                    continue;
                }

                let opencode_dir = get_opencode_restore_dir()?;
                if !opencode_dir.exists() {
                    fs::create_dir_all(&opencode_dir)
                        .map_err(|e| format!("Failed to create opencode config directory: {}", e))?;
                }

                let outpath = opencode_dir.join(relative_path);
                let mut outfile = std::fs::File::create(&outpath)
                    .map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
            } else if file_name.starts_with("external-configs/claude/") {
                // Claude settings
                let relative_path = &file_name[24..]; // Remove "external-configs/claude/" prefix
                if relative_path.is_empty() || file_name.ends_with('/') {
                    continue;
                }

                let claude_dir = home_dir.join(".claude");
                if !claude_dir.exists() {
                    fs::create_dir_all(&claude_dir)
                        .map_err(|e| format!("Failed to create claude config directory: {}", e))?;
                }

                let outpath = claude_dir.join(relative_path);
                let mut outfile = std::fs::File::create(&outpath)
                    .map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
            }
        } else {
            // Old format: all files are database files
            let outpath = db_path.join(&file_name);

            if file_name.ends_with('/') {
                fs::create_dir_all(&outpath)
                    .map_err(|e| format!("Failed to create directory: {}", e))?;
            } else {
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent)
                            .map_err(|e| format!("Failed to create parent directory: {}", e))?;
                    }
                }
                let mut outfile = std::fs::File::create(&outpath)
                    .map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;
            }
        }
    }

    Ok(())
}
