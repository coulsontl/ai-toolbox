use chrono::{DateTime, Utc};
use log::{error, info, warn};
use std::time::Duration;
use tauri::{Emitter, Manager};

use super::filename::is_managed_backup_filename;
use super::generate::generate_backup_file;
use super::repository::repository_client_from_settings;
use super::repository_settings::load_backup_repository_settings;
use super::webdav::{delete_webdav_backup_internal, list_webdav_backups_internal};
use crate::db::SqliteDbState;
use crate::http_client;
use crate::settings::store;

/// Start the auto-backup scheduler as a background task
pub fn start_auto_backup_scheduler(app_handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Initial delay: wait 30 seconds after startup
        tokio::time::sleep(Duration::from_secs(30)).await;

        info!("Auto-backup scheduler started");

        loop {
            // Check every 10 minutes
            if let Err(e) = check_and_perform_backup(&app_handle).await {
                warn!("Auto-backup check failed: {}", e);
            }

            tokio::time::sleep(Duration::from_secs(600)).await;
        }
    });
}

/// Read settings from DB and check if auto-backup should run
async fn check_and_perform_backup(app_handle: &tauri::AppHandle) -> Result<(), String> {
    let db_state = app_handle.state::<SqliteDbState>();
    let sqlite_state = app_handle.state::<SqliteDbState>();
    let settings = store::load_settings_from_sqlite_state(&sqlite_state)?;

    if !settings.auto_backup_enabled {
        return Ok(());
    }

    // Check if backup is due
    if !is_backup_due(
        &settings.last_auto_backup_time,
        settings.auto_backup_interval_days,
    ) {
        return Ok(());
    }

    match settings.backup_type.as_str() {
        "webdav" => {
            if settings.webdav.url.is_empty() {
                return Ok(());
            }

            info!("Auto-backup is due, performing WebDAV backup...");

            match perform_webdav_backup(app_handle, &db_state, &settings).await {
                Ok(()) => {
                    info!("Auto-backup completed successfully");

                    let now = Utc::now().to_rfc3339();
                    update_last_auto_backup_time(&sqlite_state, &now).await?;
                    let _ = app_handle.emit("auto-backup-completed", &now);

                    if settings.auto_backup_max_keep > 0 {
                        if let Err(e) = cleanup_old_webdav_backups(
                            &db_state,
                            &settings.webdav.url,
                            &settings.webdav.username,
                            &settings.webdav.password,
                            &settings.webdav.remote_path,
                            settings.auto_backup_max_keep,
                        )
                        .await
                        {
                            warn!("Auto-backup cleanup failed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Auto-backup failed: {}", e);

                    // Update last_auto_backup_time even on failure to prevent retry every 10 minutes
                    let now = Utc::now().to_rfc3339();
                    update_last_auto_backup_time(&sqlite_state, &now).await?;
                    let _ = app_handle.emit("auto-backup-failed", &e);
                }
            }

            Ok(())
        }
        "local" => {
            if settings.local_backup_path.is_empty() {
                return Ok(());
            }

            info!("Auto-backup is due, performing local backup...");

            match perform_local_backup(app_handle, &settings).await {
                Ok(()) => {
                    info!("Auto-backup (local) completed successfully");

                    let now = Utc::now().to_rfc3339();
                    update_last_auto_backup_time(&sqlite_state, &now).await?;
                    let _ = app_handle.emit("auto-backup-completed", &now);

                    if settings.auto_backup_max_keep > 0 {
                        if let Err(e) = cleanup_old_local_backups(
                            &settings.local_backup_path,
                            settings.auto_backup_max_keep,
                        ) {
                            warn!("Auto-backup local cleanup failed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Auto-backup (local) failed: {}", e);

                    // Update last_auto_backup_time even on failure to prevent retry every 10 minutes
                    let now = Utc::now().to_rfc3339();
                    update_last_auto_backup_time(&sqlite_state, &now).await?;
                    let _ = app_handle.emit("auto-backup-failed", &e);
                }
            }

            Ok(())
        }
        "repository" => {
            let repository_settings = load_backup_repository_settings(&sqlite_state)?;
            if repository_settings.config.is_blank_connection() {
                // Channel not configured yet: skip silently like the WebDAV/local
                // branches instead of emitting a failure event every interval.
                return Ok(());
            }

            info!("Auto-backup is due, performing repository backup...");

            match perform_repository_backup(app_handle, &db_state, repository_settings).await {
                Ok(client) => {
                    info!("Auto-backup (repository) completed successfully");

                    let now = Utc::now().to_rfc3339();
                    update_last_auto_backup_time(&sqlite_state, &now).await?;
                    let _ = app_handle.emit("auto-backup-completed", &now);

                    if settings.auto_backup_max_keep > 0 {
                        // Cleanup runs against the exact client/connection snapshot the
                        // upload just used; a settings change mid-upload can no longer
                        // redirect deletions to a different repository or directory.
                        if let Err(e) =
                            cleanup_old_repository_backups(&client, settings.auto_backup_max_keep)
                                .await
                        {
                            warn!("Auto-backup repository cleanup failed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Auto-backup (repository) failed: {}", e);

                    let now = Utc::now().to_rfc3339();
                    update_last_auto_backup_time(&sqlite_state, &now).await?;
                    let _ = app_handle.emit("auto-backup-failed", &e);
                }
            }

            Ok(())
        }
        _ => Ok(()),
    }
}

/// Check if a backup is due based on last backup time and interval
fn is_backup_due(last_time: &Option<String>, interval_days: u32) -> bool {
    let Some(last_time_str) = last_time else {
        return true;
    };

    let Ok(last_dt) = DateTime::parse_from_rfc3339(last_time_str) else {
        return true;
    };

    let elapsed = Utc::now().signed_duration_since(last_dt);
    let interval = chrono::Duration::days(interval_days as i64);

    elapsed >= interval
}

/// Perform a WebDAV backup
async fn perform_webdav_backup(
    app_handle: &tauri::AppHandle,
    db_state: &SqliteDbState,
    settings: &crate::settings::types::AppSettings,
) -> Result<(), String> {
    // Shared generation layer: zip + optional encryption + shared filename.
    let generated = generate_backup_file(app_handle, Some(&settings.webdav.host_label)).await?;

    let base_url = settings.webdav.url.trim_end_matches('/');
    let remote = settings.webdav.remote_path.trim_matches('/');
    let full_url = if remote.is_empty() {
        format!("{}/{}", base_url, generated.filename)
    } else {
        format!("{}/{}/{}", base_url, remote, generated.filename)
    };

    info!("Auto-backup: uploading to {}", full_url);

    let client = http_client::client_with_timeout(db_state, 300)
        .await
        .map_err(|e| {
            error!("Failed to create HTTP client: {}", e);
            e
        })?;

    let response = client
        .put(&full_url)
        .basic_auth(&settings.webdav.username, Some(&settings.webdav.password))
        .body(generated.bytes)
        .send()
        .await
        .map_err(|e| format!("Auto-backup upload failed: {}", e))?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "Auto-backup upload failed with status: {}",
            response.status()
        ))
    }
}

/// Perform a local backup
async fn perform_local_backup(
    app_handle: &tauri::AppHandle,
    settings: &crate::settings::types::AppSettings,
) -> Result<(), String> {
    let generated = generate_backup_file(app_handle, None).await?;

    let backup_dir = std::path::Path::new(&settings.local_backup_path);
    if !backup_dir.exists() {
        std::fs::create_dir_all(backup_dir)
            .map_err(|e| format!("Failed to create backup dir: {}", e))?;
    }

    let backup_file_path = backup_dir.join(&generated.filename);
    let temp_path = backup_dir.join(format!("{}.part", generated.filename));
    std::fs::write(&temp_path, &generated.bytes)
        .map_err(|e| format!("Failed to write backup file: {}", e))?;
    if let Err(error) = std::fs::rename(&temp_path, &backup_file_path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Failed to finalize backup file: {}", error));
    }

    info!("Auto-backup: saved to {:?}", backup_file_path);
    Ok(())
}

/// Perform a repository backup using the given connection snapshot. Returns the
/// client so the retention cleanup reuses the same repository/directory/credentials
/// instead of re-reading settings that may have changed during the upload.
async fn perform_repository_backup(
    app_handle: &tauri::AppHandle,
    db_state: &SqliteDbState,
    repository_settings: super::repository_settings::BackupRepositorySettings,
) -> Result<super::repository::RepositoryClient, String> {
    let client = repository_client_from_settings(db_state, &repository_settings).await?;
    let generated = generate_backup_file(app_handle, None).await?;
    client
        .upload_file(&generated.filename, &generated.bytes)
        .await?;
    info!(
        "Auto-backup: uploaded to repository as {}",
        generated.filename
    );
    Ok(client)
}

/// Update last_auto_backup_time in SQLite.
async fn update_last_auto_backup_time(
    sqlite_state: &SqliteDbState,
    time: &str,
) -> Result<(), String> {
    store::update_last_auto_backup_time_in_sqlite_state(sqlite_state, time)
}

/// Cleanup old WebDAV backups, keeping only the latest `max_keep` files
async fn cleanup_old_webdav_backups(
    db_state: &SqliteDbState,
    url: &str,
    username: &str,
    password: &str,
    remote_path: &str,
    max_keep: u32,
) -> Result<(), String> {
    let backups =
        list_webdav_backups_internal(db_state, url, username, password, remote_path).await?;

    if backups.len() <= max_keep as usize {
        return Ok(());
    }

    let to_delete = &backups[max_keep as usize..];
    info!(
        "Auto-backup cleanup: deleting {} old WebDAV backup(s)",
        to_delete.len()
    );

    for backup in to_delete {
        if let Err(e) = delete_webdav_backup_internal(
            db_state,
            url,
            username,
            password,
            remote_path,
            &backup.filename,
        )
        .await
        {
            warn!("Failed to delete old backup {}: {}", backup.filename, e);
        }
    }

    Ok(())
}

/// Cleanup old local backups, keeping only the latest `max_keep` files.
/// Both `.zip` and `.zip.enc` participate in sorting and retention.
fn cleanup_old_local_backups(backup_path: &str, max_keep: u32) -> Result<(), String> {
    let backup_dir = std::path::Path::new(backup_path);
    if !backup_dir.exists() {
        return Ok(());
    }

    let mut backup_files: Vec<_> = std::fs::read_dir(backup_dir)
        .map_err(|e| format!("Failed to read backup dir: {}", e))?
        .filter_map(|e| e.ok())
        .filter(|e| is_managed_backup_filename(&e.file_name().to_string_lossy()))
        .collect();

    if backup_files.len() <= max_keep as usize {
        return Ok(());
    }

    // Sort descending by the shared filename contract (most recent first)
    backup_files.sort_by(|a, b| {
        let a_key = super::filename::backup_sort_key(&a.file_name().to_string_lossy());
        let b_key = super::filename::backup_sort_key(&b.file_name().to_string_lossy());
        b_key.cmp(&a_key)
    });

    let to_delete = &backup_files[max_keep as usize..];
    info!(
        "Auto-backup cleanup: deleting {} old local backup(s)",
        to_delete.len()
    );

    for entry in to_delete {
        if let Err(e) = std::fs::remove_file(entry.path()) {
            warn!("Failed to delete old backup {:?}: {}", entry.file_name(), e);
        }
    }

    Ok(())
}

/// Cleanup old repository backups, keeping only the latest `max_keep` files.
/// Takes the same client the upload used — never re-reads the connection settings.
pub(crate) async fn cleanup_old_repository_backups(
    client: &super::repository::RepositoryClient,
    max_keep: u32,
) -> Result<(), String> {
    let listing = client.list_backups_detailed().await?;
    if !listing.complete {
        // A truncated remote listing would make retention decisions from an
        // incomplete view; skipping cleanup is always safe, wrong deletion is not.
        warn!("Auto-backup repository cleanup skipped: remote listing was truncated");
        return Ok(());
    }
    let backups = listing.backups;

    if backups.len() <= max_keep as usize {
        return Ok(());
    }

    let to_delete = &backups[max_keep as usize..];
    info!(
        "Auto-backup cleanup: deleting {} old repository backup(s)",
        to_delete.len()
    );

    for backup in to_delete {
        if let Err(e) = client.delete_file(&backup.filename, &backup.sha).await {
            warn!(
                "Failed to delete old repository backup {}: {}",
                backup.filename, e
            );
        }
    }

    Ok(())
}
