//! Per-CLI applied-account refresh passes used by the shared scheduler.

use tauri::AppHandle;
use tauri::Manager;

use crate::db::SqliteDbState;

pub(super) async fn grok_refresh_applied_pass(
    db: &SqliteDbState,
    app: &AppHandle,
) -> Result<(), String> {
    crate::coding::grok::refresh_applied_grok_accounts_if_needed(db, app).await
}

pub(super) async fn codex_refresh_applied_pass(
    db: &SqliteDbState,
    app: &AppHandle,
) -> Result<(), String> {
    crate::coding::codex::refresh_applied_codex_accounts_if_needed(db, app).await
}

pub(super) async fn gemini_cli_refresh_applied_pass(
    db: &SqliteDbState,
    app: &AppHandle,
) -> Result<(), String> {
    crate::coding::gemini_cli::refresh_applied_gemini_cli_accounts_if_needed(db, app).await
}

pub(super) async fn kimi_refresh_applied_pass(
    db: &SqliteDbState,
    app: &AppHandle,
) -> Result<(), String> {
    crate::coding::kimi::refresh_applied_kimi_accounts_if_needed(db, app).await
}

pub(super) async fn antigravity_refresh_applied_pass(
    db: &SqliteDbState,
    app: &AppHandle,
) -> Result<(), String> {
    crate::coding::antigravity::refresh_applied_antigravity_accounts_if_needed(db, app).await
}

pub(super) async fn plugins_background_pass(
    _db: &SqliteDbState,
    app: &AppHandle,
) -> Result<(), String> {
    let Some(manager) = app.try_state::<crate::plugins::PluginManagerState>() else {
        return Err("PluginManagerState is not registered yet".to_string());
    };
    crate::plugins::background_tasks::run_plugin_background_pass(manager.inner(), Some(app)).await;
    Ok(())
}
