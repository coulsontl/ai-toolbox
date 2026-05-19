use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde::Deserialize;
use tauri::Manager;

use super::central_repo::{merge_skill_settings_sqlite, read_skill_settings_i64_from_sqlite};

const CACHE_DIR_NAME: &str = "skills-git-cache";
const CACHE_META_FILE: &str = ".skills-cache.json";
pub const DEFAULT_GIT_CACHE_CLEANUP_DAYS: i64 = 30;
const MAX_GIT_CACHE_CLEANUP_DAYS: i64 = 3650;
pub const DEFAULT_GIT_CACHE_TTL_SECS: i64 = 60;

#[derive(Debug, Deserialize)]
struct RepoCacheMeta {
    last_fetched_ms: i64,
}

/// Get git cache cleanup days from settings
pub async fn get_git_cache_cleanup_days(state: &crate::SqliteDbState) -> i64 {
    read_skill_settings_i64_from_sqlite(state, "git_cache_cleanup_days")
        .unwrap_or(DEFAULT_GIT_CACHE_CLEANUP_DAYS)
}

/// Set git cache cleanup days in settings
pub async fn set_git_cache_cleanup_days(state: &crate::SqliteDbState, days: i64) -> Result<i64> {
    if !(0..=MAX_GIT_CACHE_CLEANUP_DAYS).contains(&days) {
        anyhow::bail!(
            "cleanup days must be between 0 and {}",
            MAX_GIT_CACHE_CLEANUP_DAYS
        );
    }

    let now = super::types::now_ms();

    merge_skill_settings_sqlite(
        state,
        serde_json::json!({
            "git_cache_cleanup_days": days,
            "updated_at": now,
        }),
    )
    .map_err(|e| anyhow::anyhow!("failed to save setting to SQLite: {}", e))?;

    Ok(days)
}

/// Get git cache TTL seconds from settings
pub async fn get_git_cache_ttl_secs(state: &crate::SqliteDbState) -> i64 {
    read_skill_settings_i64_from_sqlite(state, "git_cache_ttl_secs")
        .unwrap_or(DEFAULT_GIT_CACHE_TTL_SECS)
}

/// Cleanup old git cache directories
pub fn cleanup_git_cache_dirs<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    max_age: Duration,
) -> Result<usize> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .context("failed to resolve app cache dir")?;
    cleanup_git_cache_dirs_in(&cache_dir, max_age)
}

fn cleanup_git_cache_dirs_in(cache_dir: &Path, max_age: Duration) -> Result<usize> {
    let cache_root = cache_dir.join(CACHE_DIR_NAME);
    if !cache_root.exists() {
        return Ok(0);
    }

    let cutoff_ms = now_ms().saturating_sub(max_age.as_millis().try_into().unwrap_or(i64::MAX));
    let cutoff_time = SystemTime::now()
        .checked_sub(max_age)
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut removed = 0usize;
    let rd = match std::fs::read_dir(&cache_root) {
        Ok(v) => v,
        Err(err) => {
            return Err(anyhow::anyhow!(
                "failed to read cache dir {:?}: {}",
                cache_root,
                err
            ));
        }
    };

    for entry in rd.flatten() {
        let path: PathBuf = entry.path();
        if !path.is_dir() {
            continue;
        }

        if !path.join(".git").exists() {
            continue;
        }

        let meta_path = path.join(CACHE_META_FILE);
        let mut should_remove = false;

        if let Ok(raw) = std::fs::read_to_string(&meta_path) {
            if let Ok(meta) = serde_json::from_str::<RepoCacheMeta>(&raw) {
                if meta.last_fetched_ms > 0 && meta.last_fetched_ms <= cutoff_ms {
                    should_remove = true;
                }
            }
        }

        if !should_remove {
            let meta = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if modified <= cutoff_time {
                should_remove = true;
            }
        }

        if should_remove && std::fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }

    Ok(removed)
}

fn now_ms() -> i64 {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}
