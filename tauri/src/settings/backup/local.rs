use std::fs;
use std::io::Cursor;
use std::path::Path;
use zip::ZipArchive;

use super::encryption;
use super::generate::generate_backup_file;
use super::restore::{prepare_backup_bytes, restore_from_archive};
use super::utils::{get_db_path, RestoreResult};

/// Backup database to a zip file in the configured local directory.
/// The file is written through a temp file and renamed so a failure never leaves a
/// backup that looks complete.
#[tauri::command]
pub async fn backup_database(
    app_handle: tauri::AppHandle,
    backup_path: String,
) -> Result<String, String> {
    let generated = generate_backup_file(&app_handle, None).await?;

    let backup_dir = Path::new(&backup_path);
    if !backup_dir.exists() {
        fs::create_dir_all(backup_dir)
            .map_err(|e| format!("Failed to create backup dir: {}", e))?;
    }

    let backup_file_path = backup_dir.join(&generated.filename);
    let temp_path = backup_dir.join(format!("{}.part", generated.filename));
    fs::write(&temp_path, &generated.bytes)
        .map_err(|e| format!("Failed to write backup file: {}", e))?;
    if let Err(error) = fs::rename(&temp_path, &backup_file_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("Failed to finalize backup file: {}", error));
    }

    Ok(backup_file_path.to_string_lossy().to_string())
}

/// Restore database from a local backup file. Encrypted files are detected by their
/// header (never by extension); decryption happens before any restore write.
#[tauri::command]
pub async fn restore_database(
    app_handle: tauri::AppHandle,
    zip_file_path: String,
    skip_cli_custom_roots: Option<bool>,
    restore_password: Option<String>,
) -> Result<RestoreResult, String> {
    let skip_cli_custom_roots = skip_cli_custom_roots.unwrap_or(false);
    if !Path::new(&zip_file_path).exists() {
        return Err("Backup file does not exist".to_string());
    }
    let zip_path = zip_file_path.clone();
    // Encrypted backups are read into memory and decrypted first; plaintext archives
    // keep streaming from the file without a full in-memory copy.
    if is_encrypted_backup_file(Path::new(&zip_path))? {
        let bytes = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<u8>, String> {
            let bytes =
                fs::read(&zip_path).map_err(|e| format!("Failed to open backup file: {}", e))?;
            prepare_backup_bytes(bytes, restore_password.as_deref())
        })
        .await
        .map_err(|error| error.to_string())??;
        let mut archive = ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| format!("Failed to read zip archive: {}", e))?;
        restore_from_archive(&app_handle, &mut archive, skip_cli_custom_roots)
    } else {
        let file =
            fs::File::open(&zip_path).map_err(|e| format!("Failed to open backup file: {}", e))?;
        let mut archive =
            ZipArchive::new(file).map_err(|e| format!("Failed to read zip archive: {}", e))?;
        restore_from_archive(&app_handle, &mut archive, skip_cli_custom_roots)
    }
}

fn is_encrypted_backup_file(path: &Path) -> Result<bool, String> {
    use std::io::Read;
    let mut file =
        fs::File::open(path).map_err(|e| format!("Failed to open backup file: {}", e))?;
    let header_length = encryption::ENCRYPTION_MAGIC.len();
    let mut header = vec![0u8; header_length];
    let mut read_total = 0usize;
    while read_total < header_length {
        let read = file
            .read(&mut header[read_total..])
            .map_err(|e| format!("Failed to read backup file: {}", e))?;
        if read == 0 {
            break;
        }
        read_total += read;
    }
    header.truncate(read_total);
    Ok(encryption::is_encrypted(&header))
}

/// Get database directory path for frontend
#[tauri::command]
pub fn get_database_path(app_handle: tauri::AppHandle) -> Result<String, String> {
    let db_path = get_db_path(&app_handle)?;
    Ok(db_path.to_string_lossy().to_string())
}

/// Open the app data directory in the file explorer
#[tauri::command]
pub fn open_app_data_dir(app_handle: tauri::AppHandle) -> Result<(), String> {
    let _ = app_handle;
    let app_data_dir = crate::app_paths::resolved_data_dir();

    // Ensure directory exists
    if !app_data_dir.exists() {
        fs::create_dir_all(&app_data_dir)
            .map_err(|e| format!("Failed to create app data directory: {}", e))?;
    }

    // Open in file explorer (platform-specific)
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&app_data_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&app_data_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&app_data_dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    Ok(())
}
