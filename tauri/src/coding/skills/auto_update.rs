//! Scheduled auto-update task for managed skills.
//!
//! Runs a full update-all pass on a cron schedule (5-field, local timezone) when enabled.
//! Mirrors the `auth_refresh` scheduler pattern: a startup pass plus a tick-loop that
//! fires whenever the scheduled time after the last run has passed. Failures are logged
//! only, never surfaced as UI: auto-update is background/silent by design.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::db::SqliteDbState;

use super::builtin::ensure_builtin_skills;
use super::commands::update_all_skills_internal;
use super::cron_utils::parse_cron;
use super::skill_store;

const GLOBAL_INITIAL_DELAY: Duration = Duration::from_secs(60);
const TICK_GRANULARITY: Duration = Duration::from_secs(60);

static STARTED: AtomicBool = AtomicBool::new(false);

/// Start the skills auto-update loop once per process.
pub fn start(app: AppHandle) {
    if STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    tauri::async_runtime::spawn(async move {
        // The built-in Skill must exist on a fresh launch even before the user
        // opens the Skills page. This task is started once during app setup,
        // so it doubles as the startup wiring for that idempotent ensure.
        if let Some(db) = app.try_state::<SqliteDbState>() {
            if let Err(error) = ensure_builtin_skills(db.inner()).await {
                log::warn!("Failed to ensure built-in Skills at startup: {}", error);
            }
        }

        tokio::time::sleep(GLOBAL_INITIAL_DELAY).await;

        // `last_run` tracks when we last executed a pass; cron fires strictly after it.
        let mut last_run = chrono::Local::now();

        // Startup pass: run once if enabled, so skills refresh even without a manual trigger.
        if let Some(db) = app.try_state::<SqliteDbState>() {
            let prefs = skill_store::get_skill_preferences(db.inner())
                .await
                .unwrap_or_default();
            if prefs.auto_update_enabled {
                run_update_pass(&app, "startup");
                last_run = chrono::Local::now();
            }
        }

        loop {
            tokio::time::sleep(TICK_GRANULARITY).await;

            let Some(db) = app.try_state::<SqliteDbState>() else {
                continue;
            };
            let prefs = match skill_store::get_skill_preferences(db.inner()).await {
                Ok(prefs) => prefs,
                Err(_) => continue,
            };
            if !prefs.auto_update_enabled {
                continue;
            }

            let schedule = prefs.auto_update_schedule.trim().to_string();
            let cron = match parse_cron(&schedule) {
                Ok(cron) => cron,
                Err(error) => {
                    log::debug!("skills auto-update has invalid schedule '{schedule}': {error}");
                    continue;
                }
            };

            let now = chrono::Local::now();
            let next = match cron.find_next_occurrence(&last_run, false) {
                Ok(next) => next,
                Err(error) => {
                    log::debug!("skills auto-update couldn't resolve next fire: {error}");
                    continue;
                }
            };
            if next > now {
                continue;
            }
            run_update_pass(&app, "schedule");
            last_run = now;
        }
    });
}

fn run_update_pass(app: &AppHandle, kind: &'static str) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(db) = app.try_state::<SqliteDbState>() else {
            return;
        };
        // Scheduled pass stays silent: do not emit progress / bother the UI.
        match update_all_skills_internal(&app, &db, false).await {
            Ok(result) => {
                log::debug!(
                    "skills auto-update {kind} pass done: {}/{} updated, {} errors",
                    result.updated.len(),
                    result.total,
                    result.errors.len()
                );
                for err in result.errors {
                    log::debug!(
                        "skills auto-update {kind} failed for {}: {}",
                        err.name,
                        err.error
                    );
                }
            }
            Err(error) => log::debug!("skills auto-update {kind} pass failed: {error}"),
        }
    });
}
