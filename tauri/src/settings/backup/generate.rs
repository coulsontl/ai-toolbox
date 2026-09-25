//! Shared backup generation layer.
//!
//! Reads the current backup settings, builds the zip via the existing
//! `write_backup_zip_contents` pipeline (scope, filters, SQLite, external files are
//! untouched), applies optional encryption, and produces the final file bytes plus
//! the shared filename. Storage layers (local / WebDAV / repository) only receive
//! finished bytes and never query provider/MCP/Skills tables themselves.

use tauri::Manager;
use zeroize::Zeroizing;

use super::credentials::{self, backup_error};
use super::encryption::{self, CryptoError};
use super::filename;
use super::utils::{create_backup_zip, get_db_path};
use crate::db::SqliteDbState;
use crate::settings::store;

pub struct GeneratedBackup {
    pub bytes: Vec<u8>,
    pub filename: String,
    pub encrypted: bool,
}

fn crypto_error_string(error: CryptoError) -> String {
    match error {
        CryptoError::PasswordRequired => backup_error(
            "passwordRequired",
            "settings.backupSettings.encryption.errors.passwordRequired",
            "encryption is enabled but no password is stored on this machine",
        ),
        CryptoError::AuthFailed => backup_error(
            "passwordWrong",
            "settings.backupSettings.encryption.errors.passwordWrong",
            "backup decryption failed",
        ),
        CryptoError::InvalidFormat => backup_error(
            "invalidBackup",
            "settings.backupSettings.encryption.errors.invalidBackup",
            "backup payload format is invalid",
        ),
        CryptoError::RandomFailed => backup_error(
            "encryptionFailed",
            "settings.backupSettings.encryption.errors.encryption",
            "secure random generation failed",
        ),
    }
}

/// Build the finished backup file for any storage channel.
///
/// When encryption is enabled the password comes from the local credential store.
/// If the credential is unavailable the whole backup fails — encrypted auto backups
/// must never silently degrade to plaintext uploads.
pub async fn generate_backup_file(
    app_handle: &tauri::AppHandle,
    host_label: Option<&str>,
) -> Result<GeneratedBackup, String> {
    let db_path = get_db_path(app_handle)?;
    if !db_path.exists() {
        std::fs::create_dir_all(&db_path)
            .map_err(|e| format!("Failed to create database dir: {}", e))?;
    }

    let sqlite_state = app_handle.state::<SqliteDbState>();
    let settings = store::load_settings_from_sqlite_state(&sqlite_state)?;
    let zip_data = create_backup_zip(
        app_handle,
        &db_path,
        settings.backup_image_assets_enabled,
        settings.backup_cli_config_files_enabled,
        &settings.backup_file_filter_rules,
    )
    .await?;

    let encrypted = settings.backup_encryption.enabled;
    let bytes = if encrypted {
        let stored = tauri::async_runtime::spawn_blocking(credentials::read_password)
            .await
            .map_err(|error| error.to_string())??;
        let password = Zeroizing::new(
            stored.ok_or_else(|| crypto_error_string(CryptoError::PasswordRequired))?,
        );
        tauri::async_runtime::spawn_blocking(move || encryption::encrypt(&zip_data, &password))
            .await
            .map_err(|error| error.to_string())?
            .map_err(crypto_error_string)?
    } else {
        zip_data
    };

    let filename = filename::generate_backup_filename(host_label, encrypted);
    Ok(GeneratedBackup {
        bytes,
        filename,
        encrypted,
    })
}
