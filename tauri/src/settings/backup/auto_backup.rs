use chrono::{DateTime, Local, Utc};
use log::{error, info, warn};
use std::time::Duration;
use tauri::{Emitter, Manager};

use super::utils::{create_backup_zip, get_db_path};
use super::webdav::{delete_webdav_backup_internal, list_webdav_backups_internal};
use crate::db::DbState;
use crate::http_client;
use crate::settings::adapter;

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
    let db_state = app_handle.state::<DbState>();
    let settings = read_settings(&db_state).await?;

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
                    update_last_auto_backup_time(&db_state, &now).await?;
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
                    update_last_auto_backup_time(&db_state, &now).await?;
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
                    update_last_auto_backup_time(&db_state, &now).await?;
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
                    update_last_auto_backup_time(&db_state, &now).await?;
                    let _ = app_handle.emit("auto-backup-failed", &e);
                }
            }

            Ok(())
        }
        _ => Ok(()),
    }
}

/// Read AppSettings from database
async fn read_settings(db_state: &DbState) -> Result<crate::settings::types::AppSettings, String> {
    let db = db_state.db();

    let mut result = db
        .query("SELECT * OMIT id FROM settings:`app` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query settings: {}", e))?;

    let records: Vec<serde_json::Value> = result
        .take(0)
        .map_err(|e| format!("Failed to parse settings: {}", e))?;

    if let Some(record) = records.first() {
        Ok(adapter::from_db_value(record.clone()))
    } else {
        Ok(crate::settings::types::AppSettings::default())
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
    db_state: &DbState,
    settings: &crate::settings::types::AppSettings,
) -> Result<(), String> {
    let db_path = get_db_path(app_handle)?;
    let zip_data =
        create_backup_zip(app_handle, &db_path, settings.backup_image_assets_enabled).await?;

    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let host = settings.webdav.host_label.trim();
    let backup_filename = if host.is_empty() {
        format!("ai-toolbox-backup-{}.zip", timestamp)
    } else {
        format!("ai-toolbox-backup-{}_{}.zip", timestamp, host)
    };

    let base_url = settings.webdav.url.trim_end_matches('/');
    let remote = settings.webdav.remote_path.trim_matches('/');
    let full_url = if remote.is_empty() {
        format!("{}/{}", base_url, backup_filename)
    } else {
        format!("{}/{}/{}", base_url, remote, backup_filename)
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
        .body(zip_data)
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
    let db_path = get_db_path(app_handle)?;
    let zip_data =
        create_backup_zip(app_handle, &db_path, settings.backup_image_assets_enabled).await?;

    let backup_dir = std::path::Path::new(&settings.local_backup_path);
    if !backup_dir.exists() {
        std::fs::create_dir_all(backup_dir)
            .map_err(|e| format!("Failed to create backup dir: {}", e))?;
    }

    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let backup_filename = format!("ai-toolbox-backup-{}.zip", timestamp);
    let backup_file_path = backup_dir.join(&backup_filename);

    std::fs::write(&backup_file_path, &zip_data)
        .map_err(|e| format!("Failed to write backup file: {}", e))?;

    info!("Auto-backup: saved to {:?}", backup_file_path);
    Ok(())
}

/// Update last_auto_backup_time in database directly
async fn update_last_auto_backup_time(db_state: &DbState, time: &str) -> Result<(), String> {
    let db = db_state.db();
    let time_owned = time.to_string();

    db.query("UPDATE settings:`app` SET last_auto_backup_time = $time")
        .bind(("time", time_owned))
        .await
        .map_err(|e| format!("Failed to update last_auto_backup_time: {}", e))?;

    Ok(())
}

/// Cleanup old WebDAV backups, keeping only the latest `max_keep` files
async fn cleanup_old_webdav_backups(
    db_state: &DbState,
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

/// Cleanup old local backups, keeping only the latest `max_keep` files
fn cleanup_old_local_backups(backup_path: &str, max_keep: u32) -> Result<(), String> {
    let backup_dir = std::path::Path::new(backup_path);
    if !backup_dir.exists() {
        return Ok(());
    }

    let mut backup_files: Vec<_> = std::fs::read_dir(backup_dir)
        .map_err(|e| format!("Failed to read backup dir: {}", e))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name_str = name.to_string_lossy();
            name_str.starts_with("ai-toolbox-backup-") && name_str.ends_with(".zip")
        })
        .collect();

    if backup_files.len() <= max_keep as usize {
        return Ok(());
    }

    // Sort descending by filename (most recent first)
    backup_files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

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
