//! Post-restore re-apply of DB-applied providers/prompts onto local CLI config files.
//!
//! Runs after restore when CLI runtime files were skipped or missing from the backup zip.
//! The recovery stays serial at the orchestration layer, reuses existing merge paths, and
//! suppresses per-item events so WSL sync is not spawned concurrently during recovery.
//!
//! Important: never call public `list_*_providers` helpers that import/temp-load from local
//! files when the DB is empty — those would pollute the restored database.

use log::{info, warn};
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::coding::proxy_gateway::cli_proxy;
use crate::coding::proxy_gateway::paths::ProxyGatewayPaths;
use crate::coding::proxy_gateway::types::GatewayCliKey;
use crate::coding::runtime_location;
use crate::db::helpers::{db_list, db_query_by_bool};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use crate::settings::backup::utils::REAPPLY_APPLIED_FLAG_FILENAME;

const PER_CLI_TIMEOUT: Duration = Duration::from_secs(30);
const PATH_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const LOCAL_ID: &str = "__local__";

#[derive(Debug, Default)]
pub struct ReapplySummary {
    pub applied: Vec<String>,
    pub warnings: Vec<String>,
    /// WSL file-mapping modules whose local runtime files were actually rewritten.
    pub changed_modules: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreReapplyMode {
    None,
    Full,
    PluginsOnly,
}

/// Select one mutually exclusive restore re-apply path. A full re-apply already includes the
/// OpenCode companion plugins, so it always takes precedence over the resync-only path.
pub fn restore_reapply_mode(need_full_reapply: bool, need_resync: bool) -> RestoreReapplyMode {
    if need_full_reapply {
        RestoreReapplyMode::Full
    } else if need_resync {
        RestoreReapplyMode::PluginsOnly
    } else {
        RestoreReapplyMode::None
    }
}

#[derive(Debug, Default)]
struct ReapplyCliResult {
    applied: Vec<String>,
    warnings: Vec<String>,
}

pub async fn reapply_applied_runtime_after_restore<R: Runtime>(
    app: &AppHandle<R>,
) -> ReapplySummary {
    let mut summary = ReapplySummary::default();

    // Runtime location cache is refreshed by the startup recovery task before this runs.
    // Fixed serial order; one CLI task is awaited before the next task starts.
    let codex_app = app.clone();
    reapply_cli(&mut summary, "codex", async move {
        reapply_codex(&codex_app).await
    })
    .await;

    let claude_app = app.clone();
    reapply_cli(&mut summary, "claude", async move {
        reapply_claude(&claude_app).await
    })
    .await;

    let grok_app = app.clone();
    reapply_cli(&mut summary, "grok", async move {
        reapply_grok(&grok_app).await
    })
    .await;

    let kimi_app = app.clone();
    reapply_cli(&mut summary, "kimi", async move {
        reapply_kimi(&kimi_app).await
    })
    .await;

    let gemini_app = app.clone();
    reapply_cli(&mut summary, "gemini", async move {
        reapply_gemini(&gemini_app).await
    })
    .await;

    let opencode_app = app.clone();
    reapply_cli(&mut summary, "opencode", async move {
        reapply_opencode_prompt_only(&opencode_app).await
    })
    .await;

    let pi_app = app.clone();
    reapply_cli(&mut summary, "pi", async move { reapply_pi(&pi_app).await }).await;

    let omp_app = app.clone();
    reapply_cli(&mut summary, "oh_my_pi", async move {
        reapply_omp(&omp_app).await
    })
    .await;

    let claude_desktop_app = app.clone();
    reapply_cli(&mut summary, "claude_desktop", async move {
        reapply_claude_desktop(&claude_desktop_app).await
    })
    .await;

    let hermes_app = app.clone();
    reapply_cli(&mut summary, "hermes", async move {
        reapply_hermes(&hermes_app).await
    })
    .await;

    let dsh_app = app.clone();
    reapply_cli(
        &mut summary,
        "dsh",
        async move { reapply_dsh(&dsh_app).await },
    )
    .await;

    let plugin_summary = reapply_applied_opencode_plugins_after_restore(app).await;
    summary.applied.extend(plugin_summary.applied);
    summary.warnings.extend(plugin_summary.warnings);
    for module in plugin_summary.changed_modules {
        push_unique(&mut summary.changed_modules, &module);
    }

    // OpenCode main config, OpenClaw config, and Pi provider/model state are runtime-file-owned.
    // They are intentionally preserved locally instead of being reconstructed from SQLite.
    info!(
        "Post-restore re-apply finished: applied={}, warnings={}",
        summary.applied.len(),
        summary.warnings.len()
    );
    summary
}

/// Re-apply only the DB-applied OpenCode companion plugin configurations restored by SQLite.
/// This is used by ordinary restore resyncs where complete CLI runtime re-application is neither
/// needed nor allowed.
pub async fn reapply_applied_opencode_plugins_after_restore<R: Runtime>(
    app: &AppHandle<R>,
) -> ReapplySummary {
    let mut summary = ReapplySummary::default();

    let openagent_app = app.clone();
    reapply_cli(&mut summary, "oh-my-openagent", async move {
        reapply_oh_my_openagent(&openagent_app).await
    })
    .await;

    let slim_app = app.clone();
    reapply_cli(&mut summary, "oh-my-opencode-slim", async move {
        reapply_oh_my_opencode_slim(&slim_app).await
    })
    .await;

    let _ = app.emit("config-changed", "restore-reapply");
    summary
}

async fn reapply_cli<Fut>(summary: &mut ReapplySummary, label: &str, work: Fut)
where
    Fut: Future<Output = ReapplyCliResult> + Send + 'static,
{
    // Keep the apply work in a separate task. If a synchronous filesystem call inside the
    // async apply chain stalls, the outer recovery task can still observe the timeout, abort
    // the future, and continue to the next CLI.
    let mut task = tokio::spawn(work);
    match tokio::time::timeout(PER_CLI_TIMEOUT, &mut task).await {
        Ok(Ok(result)) => {
            if !result.applied.is_empty() {
                if let Some(module) = wsl_module_for_reapply_label(label) {
                    push_unique(&mut summary.changed_modules, module);
                }
            }
            summary.applied.extend(
                result
                    .applied
                    .into_iter()
                    .map(|item| format!("{label}:{item}")),
            );
            for warning_message in result.warnings {
                let message = format!("{label}: {warning_message}");
                warn!("Post-restore re-apply warning: {message}");
                summary.warnings.push(message);
            }
        }
        Ok(Err(join_error)) => {
            let message = format!("{label}: recovery task failed: {join_error}");
            warn!("Post-restore re-apply failed: {message}");
            summary.warnings.push(message);
        }
        Err(_) => {
            task.abort();
            let message = format!("{label}: timed out after {}s", PER_CLI_TIMEOUT.as_secs());
            warn!("Post-restore re-apply failed: {message}");
            summary.warnings.push(message);
        }
    }
}

fn wsl_module_for_reapply_label(label: &str) -> Option<&'static str> {
    match label {
        "codex" => Some("codex"),
        "claude" => Some("claude"),
        "grok" => Some("grok"),
        "kimi" => Some("kimi"),
        "gemini" => Some("geminicli"),
        "opencode" | "oh-my-openagent" | "oh-my-opencode-slim" => Some("opencode"),
        "pi" => Some("pi"),
        "oh_my_pi" => Some("oh_my_pi"),
        "claude_desktop" => Some("claude_desktop"),
        "hermes" => Some("hermes"),
        "dsh" => Some("dsh"),
        _ => None,
    }
}

fn push_unique(items: &mut Vec<String>, item: &str) {
    if !items.iter().any(|existing| existing == item) {
        items.push(item.to_string());
    }
}

/// Return the modules that a recovery-scoped full WSL sync must skip.
///
/// MCP and Skills are still handled once by `wsl_sync`; only unrelated CLI file mappings are
/// excluded so local runtime files protected by the backup setting are not propagated by accident.
pub fn unchanged_wsl_modules(changed_modules: &[String]) -> Vec<String> {
    const ALL_WSL_FILE_MODULES: &[&str] = &[
        "opencode",
        "claude",
        "codex",
        "grok",
        "kimi",
        "openclaw",
        "geminicli",
        "pi",
        "oh_my_pi",
        "claude_desktop",
        "hermes",
        "dsh",
    ];

    ALL_WSL_FILE_MODULES
        .iter()
        .filter(|module| !changed_modules.iter().any(|changed| changed == **module))
        .map(|module| (*module).to_string())
        .collect()
}

async fn probe_runtime_path(path: PathBuf) -> Result<(), String> {
    let display_path = path.to_string_lossy().to_string();
    let probe_task = tokio::task::spawn_blocking(move || match std::fs::symlink_metadata(&path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "runtime path is not accessible ({}): {error}",
            path.display()
        )),
    });

    match tokio::time::timeout(PATH_PROBE_TIMEOUT, probe_task).await {
        Ok(Ok(result)) => result,
        Ok(Err(join_error)) => Err(format!(
            "runtime path probe failed ({display_path}): {join_error}"
        )),
        Err(_) => Err(format!(
            "runtime path probe timed out after {}s ({display_path})",
            PATH_PROBE_TIMEOUT.as_secs()
        )),
    }
}

fn gateway_locked<R: Runtime>(_app: &AppHandle<R>, cli_key: GatewayCliKey) -> bool {
    let paths = ProxyGatewayPaths::new(crate::app_paths::resolved_data_dir());
    cli_proxy::provider_switch_locked_by_manifest(&paths, cli_key)
}

fn record_id(record: &serde_json::Value) -> Option<String> {
    record
        .get("id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn record_is_disabled(record: &serde_json::Value) -> bool {
    record
        .get("is_disabled")
        .or_else(|| record.get("isDisabled"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn query_applied_records(
    db: &SqliteDbState,
    table: DbTable,
) -> Result<Vec<serde_json::Value>, String> {
    db.with_conn(|conn| {
        db_query_by_bool(
            conn,
            table,
            &JsonFieldPath::new("is_applied")?,
            true,
            None,
            None,
        )
    })
}

fn first_applied_provider_id(db: &SqliteDbState, table: DbTable) -> Result<Option<String>, String> {
    let records = query_applied_records(db, table)?;
    Ok(records.into_iter().find_map(|record| {
        let id = record_id(&record)?;
        if id == LOCAL_ID || record_is_disabled(&record) {
            None
        } else {
            Some(id)
        }
    }))
}

fn first_applied_prompt_id(db: &SqliteDbState, table: DbTable) -> Result<Option<String>, String> {
    let records = query_applied_records(db, table)?;
    Ok(records.into_iter().find_map(|record| {
        let id = record_id(&record)?;
        if id == LOCAL_ID {
            None
        } else {
            Some(id)
        }
    }))
}

fn resolve_record_id(
    result: &mut ReapplyCliResult,
    item_type: &str,
    record_result: Result<Option<String>, String>,
) -> Option<String> {
    match record_result {
        Ok(record_id) => record_id,
        Err(error) => {
            result
                .warnings
                .push(format!("failed to query applied {item_type}: {error}"));
            None
        }
    }
}

async fn apply_record<F, Fut>(
    result: &mut ReapplyCliResult,
    item_type: &str,
    record_id: Option<String>,
    apply: F,
) where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let Some(record_id) = record_id else {
        return;
    };
    match apply(record_id.clone()).await {
        Ok(()) => result.applied.push(format!("{item_type}:{record_id}")),
        Err(error) => result
            .warnings
            .push(format!("{item_type}:{record_id}: {error}")),
    }
}

async fn reapply_codex<R: Runtime>(app: &AppHandle<R>) -> ReapplyCliResult {
    use crate::coding::codex;

    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let mut result = ReapplyCliResult::default();
    let provider_id = resolve_record_id(
        &mut result,
        "provider",
        first_applied_provider_id(&db, DbTable::CodexProvider),
    );
    let prompt_id = resolve_record_id(
        &mut result,
        "prompt",
        first_applied_prompt_id(&db, DbTable::CodexPromptConfig),
    );
    if provider_id.is_none() && prompt_id.is_none() {
        return result;
    }

    match runtime_location::get_codex_runtime_location_async(&db).await {
        Ok(location) => {
            if let Err(error) = probe_runtime_path(location.host_path).await {
                result.warnings.push(error);
                return result;
            }
        }
        Err(error) => {
            result
                .warnings
                .push(format!("failed to resolve runtime path: {error}"));
            return result;
        }
    }

    if gateway_locked(app, GatewayCliKey::Codex) {
        if let Some(provider_id) = provider_id {
            result.warnings.push(format!(
                "provider:{provider_id}: gateway takeover is active; direct provider projection was skipped"
            ));
        }
    } else {
        apply_record(
            &mut result,
            "provider",
            provider_id,
            |provider_id| async move {
                codex::apply_config_internal_without_events(&db, app, &provider_id).await
            },
        )
        .await;
    }

    apply_record(&mut result, "prompt", prompt_id, |prompt_id| async move {
        codex::apply_prompt_config_internal_without_events(app.state(), app, &prompt_id).await
    })
    .await;
    result
}

async fn reapply_claude<R: Runtime>(app: &AppHandle<R>) -> ReapplyCliResult {
    use crate::coding::claude_code;

    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let mut result = ReapplyCliResult::default();
    let provider_id = resolve_record_id(
        &mut result,
        "provider",
        first_applied_provider_id(&db, DbTable::ClaudeProvider),
    );
    let prompt_id = resolve_record_id(
        &mut result,
        "prompt",
        first_applied_prompt_id(&db, DbTable::ClaudePromptConfig),
    );
    if provider_id.is_none() && prompt_id.is_none() {
        return result;
    }

    match runtime_location::get_claude_runtime_location_async(&db).await {
        Ok(location) => {
            if let Err(error) = probe_runtime_path(location.host_path).await {
                result.warnings.push(error);
                return result;
            }
        }
        Err(error) => {
            result
                .warnings
                .push(format!("failed to resolve runtime path: {error}"));
            return result;
        }
    }

    if gateway_locked(app, GatewayCliKey::Claude) {
        if let Some(provider_id) = provider_id {
            result.warnings.push(format!(
                "provider:{provider_id}: gateway takeover is active; direct provider projection was skipped"
            ));
        }
    } else {
        apply_record(
            &mut result,
            "provider",
            provider_id,
            |provider_id| async move {
                claude_code::apply_config_internal_without_events(&db, app, &provider_id).await
            },
        )
        .await;
    }

    apply_record(&mut result, "prompt", prompt_id, |prompt_id| async move {
        claude_code::apply_prompt_config_internal_without_events(app.state(), app, &prompt_id).await
    })
    .await;
    result
}

async fn reapply_grok<R: Runtime>(app: &AppHandle<R>) -> ReapplyCliResult {
    use crate::coding::grok;

    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let mut result = ReapplyCliResult::default();
    let provider_id = resolve_record_id(
        &mut result,
        "provider",
        first_applied_provider_id(&db, DbTable::GrokProvider),
    );
    let prompt_id = resolve_record_id(
        &mut result,
        "prompt",
        first_applied_prompt_id(&db, DbTable::GrokPromptConfig),
    );
    if provider_id.is_none() && prompt_id.is_none() {
        return result;
    }

    match runtime_location::get_grok_runtime_location_async(&db).await {
        Ok(location) => {
            if let Err(error) = probe_runtime_path(location.host_path).await {
                result.warnings.push(error);
                return result;
            }
        }
        Err(error) => {
            result
                .warnings
                .push(format!("failed to resolve runtime path: {error}"));
            return result;
        }
    }

    if gateway_locked(app, GatewayCliKey::Grok) {
        if let Some(provider_id) = provider_id {
            result.warnings.push(format!(
                "provider:{provider_id}: gateway takeover is active; direct provider projection was skipped"
            ));
        }
    } else {
        apply_record(
            &mut result,
            "provider",
            provider_id,
            |provider_id| async move {
                grok::select_grok_provider_internal_without_events(&db, app, &provider_id).await
            },
        )
        .await;
    }

    apply_record(&mut result, "prompt", prompt_id, |prompt_id| async move {
        grok::apply_grok_prompt_config_internal_without_events(&db, app, &prompt_id).await
    })
    .await;
    result
}

async fn reapply_kimi<R: Runtime>(app: &AppHandle<R>) -> ReapplyCliResult {
    use crate::coding::kimi;

    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let mut result = ReapplyCliResult::default();
    let provider_id = resolve_record_id(
        &mut result,
        "provider",
        first_applied_provider_id(&db, DbTable::KimiProvider),
    );
    let prompt_id = resolve_record_id(
        &mut result,
        "prompt",
        first_applied_prompt_id(&db, DbTable::KimiPromptConfig),
    );
    if provider_id.is_none() && prompt_id.is_none() {
        return result;
    }

    match runtime_location::get_kimi_runtime_location_async(&db).await {
        Ok(location) => {
            if let Err(error) = probe_runtime_path(location.host_path).await {
                result.warnings.push(error);
                return result;
            }
        }
        Err(error) => {
            result
                .warnings
                .push(format!("failed to resolve runtime path: {error}"));
            return result;
        }
    }

    if gateway_locked(app, GatewayCliKey::Kimi) {
        if let Some(provider_id) = provider_id {
            result.warnings.push(format!(
                "provider:{provider_id}: gateway takeover is active; direct provider projection was skipped"
            ));
        }
    } else {
        apply_record(
            &mut result,
            "provider",
            provider_id,
            |provider_id| async move {
                kimi::select_kimi_provider_internal_with_sync(&db, app, &provider_id, false, false)
                    .await
            },
        )
        .await;
    }

    apply_record(&mut result, "prompt", prompt_id, |prompt_id| async move {
        kimi::apply_kimi_prompt_config_internal_without_events(&db, app, &prompt_id).await
    })
    .await;
    result
}

async fn reapply_gemini<R: Runtime>(app: &AppHandle<R>) -> ReapplyCliResult {
    use crate::coding::gemini_cli;

    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let mut result = ReapplyCliResult::default();
    let provider_id = resolve_record_id(
        &mut result,
        "provider",
        first_applied_provider_id(&db, DbTable::GeminiCliProvider),
    );
    let prompt_id = resolve_record_id(
        &mut result,
        "prompt",
        first_applied_prompt_id(&db, DbTable::GeminiCliPromptConfig),
    );
    if provider_id.is_none() && prompt_id.is_none() {
        return result;
    }

    match runtime_location::get_gemini_cli_runtime_location_async(&db).await {
        Ok(location) => {
            if let Err(error) = probe_runtime_path(location.host_path).await {
                result.warnings.push(error);
                return result;
            }
        }
        Err(error) => {
            result
                .warnings
                .push(format!("failed to resolve runtime path: {error}"));
            return result;
        }
    }

    if gateway_locked(app, GatewayCliKey::Gemini) {
        if let Some(provider_id) = provider_id {
            result.warnings.push(format!(
                "provider:{provider_id}: gateway takeover is active; direct provider projection was skipped"
            ));
        }
    } else {
        apply_record(
            &mut result,
            "provider",
            provider_id,
            |provider_id| async move {
                gemini_cli::apply_config_internal_without_events(&db, app, &provider_id).await
            },
        )
        .await;
    }

    apply_record(&mut result, "prompt", prompt_id, |prompt_id| async move {
        gemini_cli::apply_prompt_config_internal_without_events(app.state(), app, &prompt_id).await
    })
    .await;
    result
}

async fn reapply_opencode_prompt_only<R: Runtime>(app: &AppHandle<R>) -> ReapplyCliResult {
    use crate::coding::open_code;

    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let mut result = ReapplyCliResult::default();
    let prompt_id = resolve_record_id(
        &mut result,
        "prompt",
        first_applied_prompt_id(&db, DbTable::OpenCodePromptConfig),
    );
    if prompt_id.is_none() {
        return result;
    }

    match runtime_location::get_opencode_prompt_path_async(&db).await {
        Ok(path) => {
            if let Err(error) = probe_runtime_path(path).await {
                result.warnings.push(error);
                return result;
            }
        }
        Err(error) => {
            result
                .warnings
                .push(format!("failed to resolve prompt path: {error}"));
            return result;
        }
    }

    apply_record(&mut result, "prompt", prompt_id, |prompt_id| async move {
        open_code::apply_prompt_config_internal_without_events(app.state(), app, &prompt_id).await
    })
    .await;
    result
}

async fn reapply_pi<R: Runtime>(app: &AppHandle<R>) -> ReapplyCliResult {
    use crate::coding::pi;

    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let mut result = ReapplyCliResult::default();
    let prompt_id = resolve_record_id(
        &mut result,
        "prompt",
        first_applied_prompt_id(&db, DbTable::PiPromptConfig),
    );
    if prompt_id.is_none() {
        return result;
    }

    match pi::get_pi_prompt_path_async(&db).await {
        Ok(path) => {
            if let Err(error) = probe_runtime_path(path).await {
                result.warnings.push(error);
                return result;
            }
        }
        Err(error) => {
            result
                .warnings
                .push(format!("failed to resolve prompt path: {error}"));
            return result;
        }
    }

    apply_record(&mut result, "prompt", prompt_id, |prompt_id| async move {
        pi::apply_pi_prompt_config_internal_without_events(app.state(), app, &prompt_id).await
    })
    .await;
    result
}

async fn reapply_omp<R: Runtime>(app: &AppHandle<R>) -> ReapplyCliResult {
    use crate::coding::oh_my_pi;

    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let mut result = ReapplyCliResult::default();
    let prompt_id = resolve_record_id(
        &mut result,
        "prompt",
        first_applied_prompt_id(&db, DbTable::OhMyPiPromptConfig),
    );
    if prompt_id.is_none() {
        return result;
    }

    match oh_my_pi::get_omp_prompt_path_async(&db).await {
        Ok(path) => {
            if let Err(error) = probe_runtime_path(path).await {
                result.warnings.push(error);
                return result;
            }
        }
        Err(error) => {
            result
                .warnings
                .push(format!("failed to resolve prompt path: {error}"));
            return result;
        }
    }

    apply_record(&mut result, "prompt", prompt_id, |prompt_id| async move {
        oh_my_pi::apply_omp_prompt_config_internal_without_events(app.state(), app, &prompt_id)
            .await
    })
    .await;
    result
}

async fn reapply_claude_desktop<R: Runtime>(app: &AppHandle<R>) -> ReapplyCliResult {
    use crate::coding::claude_desktop;

    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let mut result = ReapplyCliResult::default();
    // Claude Desktop is DB-backed (optional), so reapplying the applied provider
    // reconstructs the 3P profile files when their disk copies were skipped.
    let provider_id = resolve_record_id(
        &mut result,
        "provider",
        first_applied_provider_id(&db, DbTable::ClaudeDesktopProvider),
    );
    if provider_id.is_none() {
        return result;
    }

    apply_record(
        &mut result,
        "provider",
        provider_id,
        |provider_id| async move {
            claude_desktop::apply_config_internal_without_events(&db, app, &provider_id).await
        },
    )
    .await;
    result
}

async fn reapply_hermes<R: Runtime>(app: &AppHandle<R>) -> ReapplyCliResult {
    use crate::coding::hermes;

    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let mut result = ReapplyCliResult::default();
    // Hermes provider/model fact source is the runtime config.yaml (always
    // restored from the backup), so only the DB-backed prompt preset is reapplied.
    let prompt_id = resolve_record_id(
        &mut result,
        "prompt",
        first_applied_prompt_id(&db, DbTable::HermesPromptConfig),
    );
    if prompt_id.is_none() {
        return result;
    }

    apply_record(&mut result, "prompt", prompt_id, |prompt_id| async move {
        hermes::apply_hermes_prompt_config_internal_without_events(app.state(), app, &prompt_id)
            .await
    })
    .await;
    result
}

async fn reapply_dsh<R: Runtime>(app: &AppHandle<R>) -> ReapplyCliResult {
    use crate::coding::dsh;

    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let mut result = ReapplyCliResult::default();
    // dsh provider/model fact source is the runtime settings.yaml (always
    // restored from the backup), so only the DB-backed prompt preset is reapplied.
    let prompt_id = resolve_record_id(
        &mut result,
        "prompt",
        first_applied_prompt_id(&db, DbTable::DshPromptConfig),
    );
    if prompt_id.is_none() {
        return result;
    }

    match dsh::get_dsh_prompt_path_async(&db).await {
        Ok(path) => {
            if let Err(error) = probe_runtime_path(path).await {
                result.warnings.push(error);
                return result;
            }
        }
        Err(error) => {
            result
                .warnings
                .push(format!("failed to resolve prompt path: {error}"));
            return result;
        }
    }

    apply_record(&mut result, "prompt", prompt_id, |prompt_id| async move {
        dsh::apply_dsh_prompt_config_internal_without_events(app.state(), app, &prompt_id).await
    })
    .await;
    result
}

fn list_all_records(db: &SqliteDbState, table: DbTable) -> Result<Vec<serde_json::Value>, String> {
    let order = OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)
            .unwrap_or_else(|_| OrderField::created_at(OrderDirection::Asc)),
        OrderField::created_at(OrderDirection::Asc),
    ]);
    db.with_conn(|conn| db_list(conn, table, Some(&order)))
}

fn first_applied_oh_my_config_id(
    db: &SqliteDbState,
    table: DbTable,
) -> Result<Option<String>, String> {
    if let Some(config_id) = first_applied_provider_id(db, table)? {
        return Ok(Some(config_id));
    }
    Ok(list_all_records(db, table)?.into_iter().find_map(|record| {
        let applied = record
            .get("is_applied")
            .or_else(|| record.get("isApplied"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if !applied || record_is_disabled(&record) {
            return None;
        }
        record_id(&record).filter(|id| id != LOCAL_ID)
    }))
}

async fn reapply_oh_my_openagent<R: Runtime>(app: &AppHandle<R>) -> ReapplyCliResult {
    use crate::coding::oh_my_openagent;

    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let mut result = ReapplyCliResult::default();
    let config_id = resolve_record_id(
        &mut result,
        "config",
        first_applied_oh_my_config_id(&db, DbTable::OhMyOpenAgentConfig),
    );
    if config_id.is_none() {
        return result;
    }

    match runtime_location::get_omo_config_path_async(&db).await {
        Ok(path) => {
            if let Err(error) = probe_runtime_path(path).await {
                result.warnings.push(error);
                return result;
            }
        }
        Err(error) => {
            result
                .warnings
                .push(format!("failed to resolve config path: {error}"));
            return result;
        }
    }

    apply_record(&mut result, "config", config_id, |config_id| async move {
        oh_my_openagent::apply_config_internal_without_events(&db, app, &config_id).await
    })
    .await;
    result
}

async fn reapply_oh_my_opencode_slim<R: Runtime>(app: &AppHandle<R>) -> ReapplyCliResult {
    use crate::coding::oh_my_opencode_slim;

    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let mut result = ReapplyCliResult::default();
    let config_id = resolve_record_id(
        &mut result,
        "config",
        first_applied_oh_my_config_id(&db, DbTable::OhMyOpenCodeSlimConfig),
    );
    if config_id.is_none() {
        return result;
    }

    match runtime_location::get_omos_config_path_async(&db).await {
        Ok(path) => {
            if let Err(error) = probe_runtime_path(path).await {
                result.warnings.push(error);
                return result;
            }
        }
        Err(error) => {
            result
                .warnings
                .push(format!("failed to resolve config path: {error}"));
            return result;
        }
    }

    apply_record(&mut result, "config", config_id, |config_id| async move {
        oh_my_opencode_slim::apply_config_internal_without_events(&db, app, &config_id).await
    })
    .await;
    result
}

/// Path of the post-restore re-apply flag under app data.
pub fn reapply_flag_path(app_data_dir: &std::path::Path) -> std::path::PathBuf {
    app_data_dir.join(REAPPLY_APPLIED_FLAG_FILENAME)
}

#[cfg(test)]
mod tests {
    use super::{
        apply_record, resolve_record_id, restore_reapply_mode, unchanged_wsl_modules,
        wsl_module_for_reapply_label, ReapplyCliResult, RestoreReapplyMode,
    };

    #[tokio::test]
    async fn failed_provider_step_does_not_prevent_prompt_step() {
        let mut result = ReapplyCliResult::default();
        apply_record(
            &mut result,
            "provider",
            Some("provider-1".to_string()),
            |_| async { Err("provider failed".to_string()) },
        )
        .await;
        apply_record(
            &mut result,
            "prompt",
            Some("prompt-1".to_string()),
            |_| async { Ok(()) },
        )
        .await;

        assert_eq!(result.applied, vec!["prompt:prompt-1"]);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("provider failed"));
    }

    #[test]
    fn query_error_is_recorded_without_panicking() {
        let mut result = ReapplyCliResult::default();
        let record_id = resolve_record_id(
            &mut result,
            "prompt",
            Err("database unavailable".to_string()),
        );

        assert!(record_id.is_none());
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("database unavailable"));
    }

    #[test]
    fn reapply_labels_map_to_existing_wsl_modules() {
        assert_eq!(wsl_module_for_reapply_label("gemini"), Some("geminicli"));
        assert_eq!(
            wsl_module_for_reapply_label("oh-my-openagent"),
            Some("opencode")
        );
        assert_eq!(wsl_module_for_reapply_label("pi"), Some("pi"));
    }

    #[test]
    fn recovery_wsl_sync_skips_unmodified_cli_modules() {
        let changed_modules = vec!["codex".to_string(), "opencode".to_string()];
        let skipped_modules = unchanged_wsl_modules(&changed_modules);

        assert!(!skipped_modules.contains(&"codex".to_string()));
        assert!(!skipped_modules.contains(&"opencode".to_string()));
        assert!(skipped_modules.contains(&"openclaw".to_string()));
        assert!(skipped_modules.contains(&"geminicli".to_string()));
        assert!(skipped_modules.contains(&"pi".to_string()));
    }

    #[test]
    fn full_restore_reapply_takes_precedence_over_plugin_only_reapply() {
        assert_eq!(restore_reapply_mode(true, true), RestoreReapplyMode::Full);
    }

    #[test]
    fn resync_without_full_flag_selects_plugin_only_reapply() {
        assert_eq!(
            restore_reapply_mode(false, true),
            RestoreReapplyMode::PluginsOnly
        );
        assert_eq!(restore_reapply_mode(false, false), RestoreReapplyMode::None);
    }
}
