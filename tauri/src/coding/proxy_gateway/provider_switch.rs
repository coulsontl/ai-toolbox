use super::cli_proxy;
use super::paths::ProxyGatewayPaths;
use super::provider_protocol;
use super::runtime::ProxyGatewayState;
use super::types::{
    GatewayCliKey, GatewayCliTakeoverStatus, GatewayProviderWriteSnapshot, GatewayProxyMode,
    ProxyGatewayStatus,
};
use crate::db::helpers::db_get;
use crate::db::schema::{Cli, DbTable};
use crate::db::SqliteDbState;
use tauri::{AppHandle, Emitter, Manager, Runtime};

pub async fn apply_or_switch_provider<R: Runtime>(
    app: &AppHandle<R>,
    cli_key: GatewayCliKey,
    provider_id: &str,
    from_tray: bool,
) -> Result<GatewayCliTakeoverStatus, String> {
    // Keep restore -> apply -> re-engage atomic with respect to a data-root switch.
    let _data_dir_transition = crate::app_paths::DATA_DIR_CHANGE_LOCK.lock().await;
    let gateway_state = app.state::<ProxyGatewayState>();
    let _switch_guard = gateway_state.provider_switch_lock.lock().await;
    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    let paths = proxy_gateway_paths(app)?;
    let gateway_status = current_gateway_status(&gateway_state)?;
    let current_status = cli_proxy::cli_takeover_status(db, &paths, cli_key, &gateway_status).await;

    let Some(previous_mode) = current_status.mode else {
        if current_status.can_restore_direct {
            return Err(current_status.message.unwrap_or_else(|| {
                "Restore direct mode before switching Gateway proxy providers".to_string()
            }));
        }
        if provider_needs_gateway_proxy_for_switch(db, cli_key, provider_id)? {
            crate::app_paths::ensure_no_pending_data_dir_change()?;
            if !gateway_status.running {
                return Err(
                    "Start the proxy gateway before applying a non-native protocol provider"
                        .to_string(),
                );
            }
            if !current_status.can_takeover {
                return Err(current_status.message.unwrap_or_else(|| {
                    "This provider uses a non-native protocol and must be applied through the Gateway"
                        .to_string()
                }));
            }

            let event_plan = gateway_takeover_switch_event_plan(from_tray);
            apply_provider_with_plan(app, cli_key, provider_id, event_plan).await?;

            let refreshed_gateway_status = current_gateway_status(&gateway_state)?;
            let next_status = cli_proxy::engage_single_cli(
                db,
                &paths,
                cli_key,
                &refreshed_gateway_status,
                provider_id.to_string(),
            )
            .await?;

            gateway_state.clear_provider_cache()?;
            if let Some(payload) = event_plan.final_config_changed_payload {
                emit_gateway_cli_config_changed(app, payload);
            }
            if event_plan.emit_final_wsl_sync_request {
                emit_gateway_cli_wsl_sync_request(app, cli_key);
            }
            return Ok(next_status);
        }
        apply_provider_with_plan(
            app,
            cli_key,
            provider_id,
            direct_provider_apply_event_plan(from_tray),
        )
        .await?;
        return Ok(cli_proxy::cli_takeover_status(db, &paths, cli_key, &gateway_status).await);
    };

    if !gateway_status.running {
        return Err("Start the proxy gateway before switching Gateway proxy providers".to_string());
    }
    if !current_status.can_restore_direct {
        return Err(current_status.message.unwrap_or_else(|| {
            "Restore direct mode before switching Gateway proxy providers".to_string()
        }));
    }

    let snapshot = GatewayProviderWriteSnapshot {
        mode: previous_mode,
        primary_provider_id: current_status.primary_provider_id.clone(),
        aggregate: current_status.aggregate.clone(),
    };

    crate::app_paths::ensure_no_pending_data_dir_change()?;
    cli_proxy::ensure_proxyable_provider(db, cli_key, provider_id).await?;
    cli_proxy::restore_cli_direct(db, &paths, cli_key, &gateway_status).await?;
    let event_plan = gateway_takeover_switch_event_plan(from_tray);
    apply_provider_with_plan(app, cli_key, provider_id, event_plan).await?;

    let refreshed_gateway_status = current_gateway_status(&gateway_state)?;
    // Applying a provider *is* the single-mode switch: re-engage the provider
    // the caller chose, not the one captured before the write. Failover and
    // aggregate replay the snapshot so their selection survives the round trip.
    let next_status = if snapshot.mode == GatewayProxyMode::Single {
        cli_proxy::engage_single_cli(
            db,
            &paths,
            cli_key,
            &refreshed_gateway_status,
            provider_id.to_string(),
        )
        .await?
    } else {
        reengage_cli_with_snapshot(db, &paths, cli_key, &refreshed_gateway_status, &snapshot)
            .await?
    };

    gateway_state.clear_provider_cache()?;
    if let Some(payload) = event_plan.final_config_changed_payload {
        emit_gateway_cli_config_changed(app, payload);
    }
    if event_plan.emit_final_wsl_sync_request {
        emit_gateway_cli_wsl_sync_request(app, cli_key);
    }
    Ok(next_status)
}

/// Restores a taken-over CLI to direct mode before a provider/config write.
///
/// Returns the canonical snapshot the caller must hand back to
/// [`finish_provider_write`]. `None` means the CLI is not taken over, so the
/// write can run against the normal direct configuration.
pub async fn begin_provider_write<R: Runtime>(
    app: &AppHandle<R>,
    cli_key: GatewayCliKey,
) -> Result<Option<GatewayProviderWriteSnapshot>, String> {
    let _data_dir_transition = crate::app_paths::DATA_DIR_CHANGE_LOCK.lock().await;
    let gateway_state = app.state::<ProxyGatewayState>();
    let _switch_guard = gateway_state.provider_switch_lock.lock().await;
    begin_provider_write_locked(app, &gateway_state, cli_key).await
}

/// Replays the takeover captured by [`begin_provider_write`].
///
/// The frontend uses begin/finish around a Tauri save command; the Rust
/// [`with_gateway_reattach`] helper uses the same locked internals so both
/// paths re-engage single, failover and aggregate exactly the same way.
pub async fn finish_provider_write<R: Runtime>(
    app: &AppHandle<R>,
    cli_key: GatewayCliKey,
    snapshot: GatewayProviderWriteSnapshot,
) -> Result<GatewayCliTakeoverStatus, String> {
    let _data_dir_transition = crate::app_paths::DATA_DIR_CHANGE_LOCK.lock().await;
    let gateway_state = app.state::<ProxyGatewayState>();
    let _switch_guard = gateway_state.provider_switch_lock.lock().await;
    finish_provider_write_locked(app, &gateway_state, cli_key, &snapshot).await
}

/// Runs one write while preserving an active gateway takeover.
///
/// The write must only touch the provider/config source of truth (DB or global
/// config file); it must not itself call an engage/restore command, because the
/// data-dir and provider-switch locks are held across the whole sequence.
pub async fn with_gateway_reattach<R, T, F, Fut>(
    app: &AppHandle<R>,
    cli_key: GatewayCliKey,
    write: F,
) -> Result<T, String>
where
    R: Runtime,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let _data_dir_transition = crate::app_paths::DATA_DIR_CHANGE_LOCK.lock().await;
    let gateway_state = app.state::<ProxyGatewayState>();
    let _switch_guard = gateway_state.provider_switch_lock.lock().await;

    let Some(snapshot) = begin_provider_write_locked(app, &gateway_state, cli_key).await? else {
        return write().await;
    };

    let write_result = write().await;
    let reengage_result =
        finish_provider_write_locked(app, &gateway_state, cli_key, &snapshot).await;

    match (write_result, reengage_result) {
        (Ok(value), Ok(_)) => Ok(value),
        (Err(error), Ok(_)) => Err(error),
        (Ok(_), Err(error)) => Err(format!(
            "The write succeeded but the gateway takeover could not be re-engaged: {error}"
        )),
        (Err(write_error), Err(reengage_error)) => Err(format!(
            "{write_error}; additionally the gateway takeover could not be re-engaged: {reengage_error}"
        )),
    }
}

async fn begin_provider_write_locked<R: Runtime>(
    app: &AppHandle<R>,
    gateway_state: &ProxyGatewayState,
    cli_key: GatewayCliKey,
) -> Result<Option<GatewayProviderWriteSnapshot>, String> {
    crate::app_paths::ensure_no_pending_data_dir_change()?;
    let db_state = app.state::<SqliteDbState>();
    let paths = proxy_gateway_paths(app)?;
    let gateway_status = current_gateway_status(gateway_state)?;
    let status = cli_proxy::cli_takeover_status(db_state.db(), &paths, cli_key, &gateway_status).await;
    let Some(mode) = status.mode else {
        return Ok(None);
    };
    if !gateway_status.running {
        return Err("Start the proxy gateway before editing a taken-over CLI".to_string());
    }
    if !status.can_restore_direct {
        return Err(status
            .message
            .unwrap_or_else(|| "Restore direct mode before editing this CLI".to_string()));
    }

    let snapshot = GatewayProviderWriteSnapshot {
        mode,
        primary_provider_id: status.primary_provider_id,
        aggregate: status.aggregate,
    };
    cli_proxy::restore_cli_direct(db_state.db(), &paths, cli_key, &gateway_status).await?;
    gateway_state.clear_provider_cache()?;
    Ok(Some(snapshot))
}

async fn finish_provider_write_locked<R: Runtime>(
    app: &AppHandle<R>,
    gateway_state: &ProxyGatewayState,
    cli_key: GatewayCliKey,
    snapshot: &GatewayProviderWriteSnapshot,
) -> Result<GatewayCliTakeoverStatus, String> {
    let db_state = app.state::<SqliteDbState>();
    let paths = proxy_gateway_paths(app)?;
    let gateway_status = current_gateway_status(gateway_state)?;
    if !gateway_status.running {
        return Err("Start the proxy gateway before re-engaging the CLI takeover".to_string());
    }
    let status = reengage_cli_with_snapshot(
        db_state.db(),
        &paths,
        cli_key,
        &gateway_status,
        snapshot,
    )
    .await?;
    gateway_state.clear_provider_cache()?;
    emit_gateway_cli_config_changed(app, "window");
    emit_gateway_cli_wsl_sync_request(app, cli_key);
    Ok(status)
}

async fn reengage_cli_with_snapshot(
    db: &SqliteDbState,
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    gateway_status: &ProxyGatewayStatus,
    snapshot: &GatewayProviderWriteSnapshot,
) -> Result<GatewayCliTakeoverStatus, String> {
    match snapshot.mode {
        GatewayProxyMode::Single => {
            let provider_id = snapshot.primary_provider_id.clone().ok_or_else(|| {
                "The single-mode takeover snapshot has no primary provider".to_string()
            })?;
            cli_proxy::engage_single_cli(db, paths, cli_key, gateway_status, provider_id).await
        }
        GatewayProxyMode::Failover => {
            cli_proxy::engage_failover_cli(db, paths, cli_key, gateway_status).await
        }
        GatewayProxyMode::Aggregate => {
            let aggregate = snapshot.aggregate.as_ref().ok_or_else(|| {
                "The aggregate takeover snapshot has no aggregate configuration".to_string()
            })?;
            let subagent_defaults =
                crate::coding::proxy_gateway::cli_proxy::manifest::AggregateSubagentDefaults {
                    model: aggregate
                        .subagent
                        .as_ref()
                        .and_then(|subagent| subagent.model.clone()),
                    reasoning_effort: aggregate
                        .subagent
                        .as_ref()
                        .and_then(|subagent| subagent.reasoning_effort.clone()),
                };
            cli_proxy::engage_aggregate_cli(
                db,
                paths,
                cli_key,
                gateway_status,
                aggregate.provider_ids.clone(),
                aggregate.separator.clone(),
                aggregate.aliases.clone(),
                aggregate.naming,
                aggregate.cross_site_failover,
                aggregate.subagent_exposed_models.clone(),
                subagent_defaults,
            )
            .await
        }
    }
}

fn provider_needs_gateway_proxy_for_switch(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
    provider_id: &str,
) -> Result<bool, String> {
    let Some(table) = provider_table(cli_key) else {
        return Ok(false);
    };
    let Some(record) = db.with_conn(|conn| db_get(conn, table, provider_id))? else {
        return Ok(false);
    };
    let category = record
        .get("category")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("custom");
    let settings_config = record
        .get("settings_config")
        .or_else(|| record.get("settingsConfig"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("{}");

    Ok(provider_protocol::provider_needs_gateway_proxy(
        cli_key,
        category,
        record.get("meta"),
        settings_config,
    ))
}

pub(crate) fn provider_table(cli_key: GatewayCliKey) -> Option<DbTable> {
    match cli_key {
        GatewayCliKey::Claude => Some(DbTable::CliProvider(Cli::Claude)),
        GatewayCliKey::ClaudeDesktop => Some(DbTable::CliProvider(Cli::ClaudeDesktop)),
        GatewayCliKey::Codex => Some(DbTable::CliProvider(Cli::Codex)),
        GatewayCliKey::Grok => Some(DbTable::CliProvider(Cli::Grok)),
        GatewayCliKey::Kimi => Some(DbTable::CliProvider(Cli::Kimi)),
        GatewayCliKey::Gemini => Some(DbTable::CliProvider(Cli::Gemini)),
        GatewayCliKey::Antigravity => Some(DbTable::CliProvider(Cli::Antigravity)),
        GatewayCliKey::Workbuddy => Some(DbTable::CliProvider(Cli::Workbuddy)),
        GatewayCliKey::Codebuddy => Some(DbTable::CliProvider(Cli::Codebuddy)),
        GatewayCliKey::OpenCode => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectProviderApplyMode {
    WithEvents { from_tray: bool },
    WithoutEvents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProviderSwitchEventPlan {
    direct_apply_mode: DirectProviderApplyMode,
    final_config_changed_payload: Option<&'static str>,
    emit_final_wsl_sync_request: bool,
}

fn direct_provider_apply_event_plan(from_tray: bool) -> ProviderSwitchEventPlan {
    ProviderSwitchEventPlan {
        direct_apply_mode: DirectProviderApplyMode::WithEvents { from_tray },
        final_config_changed_payload: None,
        emit_final_wsl_sync_request: false,
    }
}

fn gateway_takeover_switch_event_plan(from_tray: bool) -> ProviderSwitchEventPlan {
    ProviderSwitchEventPlan {
        direct_apply_mode: DirectProviderApplyMode::WithoutEvents,
        final_config_changed_payload: Some(config_changed_payload(from_tray)),
        emit_final_wsl_sync_request: true,
    }
}

fn config_changed_payload(from_tray: bool) -> &'static str {
    if from_tray {
        "tray"
    } else {
        "window"
    }
}

async fn apply_provider_with_plan<R: Runtime>(
    app: &AppHandle<R>,
    cli_key: GatewayCliKey,
    provider_id: &str,
    event_plan: ProviderSwitchEventPlan,
) -> Result<(), String> {
    match event_plan.direct_apply_mode {
        DirectProviderApplyMode::WithEvents { from_tray } => {
            apply_direct_provider(app, cli_key, provider_id, from_tray).await
        }
        DirectProviderApplyMode::WithoutEvents => {
            apply_direct_provider_without_events(app, cli_key, provider_id).await
        }
    }
}

async fn apply_direct_provider<R: Runtime>(
    app: &AppHandle<R>,
    cli_key: GatewayCliKey,
    provider_id: &str,
    from_tray: bool,
) -> Result<(), String> {
    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    match cli_key {
        GatewayCliKey::Claude => {
            crate::coding::claude_code::commands::apply_config_internal_with_sync(
                &db,
                app,
                provider_id,
                from_tray,
                true,
            )
            .await
        }
        GatewayCliKey::ClaudeDesktop => {
            crate::coding::claude_desktop::commands::apply_config_internal_with_sync(
                &db,
                app,
                provider_id,
                from_tray,
                true,
            )
            .await
        }
        GatewayCliKey::Codex => {
            crate::coding::codex::commands::apply_config_internal_with_sync(
                &db,
                app,
                provider_id,
                from_tray,
                true,
            )
            .await
        }
        GatewayCliKey::Grok => {
            crate::coding::grok::commands::select_grok_provider_internal_with_sync(
                &db,
                app,
                provider_id,
                from_tray,
                true,
            )
            .await
        }
        GatewayCliKey::Kimi => {
            crate::coding::kimi::commands::select_kimi_provider_internal_with_sync(
                &db,
                app,
                provider_id,
                from_tray,
                true,
            )
            .await
        }
        GatewayCliKey::Gemini => {
            crate::coding::gemini_cli::commands::apply_config_internal_with_sync(
                &db,
                app,
                provider_id,
                from_tray,
                true,
            )
            .await
        }
        GatewayCliKey::Antigravity => {
            crate::coding::antigravity::commands::apply_config_internal_with_sync(
                app,
                &db,
                provider_id,
                from_tray,
            )
            .await
        }
        GatewayCliKey::OpenCode => Err("This CLI is not supported by the gateway MVP".to_string()),
        // The family products have no per-provider apply: their custom models all
        // live in one file and several are usable at once, so there is no single
        // "current" provider to switch to. Takeover works at the file level.
        GatewayCliKey::Workbuddy | GatewayCliKey::Codebuddy => Err(
            "This product has no per-provider apply; use the gateway takeover instead".to_string(),
        ),
    }
}

async fn apply_direct_provider_without_events<R: Runtime>(
    app: &AppHandle<R>,
    cli_key: GatewayCliKey,
    provider_id: &str,
) -> Result<(), String> {
    let db_state = app.state::<SqliteDbState>();
    let db = db_state.db();
    match cli_key {
        GatewayCliKey::Claude => {
            crate::coding::claude_code::commands::apply_config_internal_without_events(
                &db,
                app,
                provider_id,
            )
            .await
        }
        GatewayCliKey::ClaudeDesktop => {
            crate::coding::claude_desktop::commands::apply_config_internal_without_events(
                &db,
                app,
                provider_id,
            )
            .await
        }
        GatewayCliKey::Codex => {
            crate::coding::codex::commands::apply_config_internal_without_events(
                &db,
                app,
                provider_id,
            )
            .await
        }
        GatewayCliKey::Grok => {
            crate::coding::grok::commands::select_grok_provider_internal_without_events(
                &db,
                app,
                provider_id,
            )
            .await
        }
        GatewayCliKey::Kimi => {
            crate::coding::kimi::commands::select_kimi_provider_internal_with_sync(
                &db,
                app,
                provider_id,
                false,
                false,
            )
            .await
        }
        GatewayCliKey::Gemini => {
            crate::coding::gemini_cli::commands::apply_config_internal_without_events(
                &db,
                app,
                provider_id,
            )
            .await
        }
        GatewayCliKey::Antigravity => {
            crate::coding::antigravity::commands::apply_config_internal_without_events(
                &db,
                provider_id,
            )
            .await
        }
        GatewayCliKey::OpenCode => Err("This CLI is not supported by the gateway MVP".to_string()),
        GatewayCliKey::Workbuddy | GatewayCliKey::Codebuddy => Err(
            "This product has no per-provider apply; use the gateway takeover instead".to_string(),
        ),
    }
}

fn current_gateway_status(gateway_state: &ProxyGatewayState) -> Result<ProxyGatewayStatus, String> {
    let manager = gateway_state
        .manager
        .lock()
        .map_err(|_| "Proxy gateway manager lock poisoned".to_string())?;
    Ok(manager.status())
}

fn proxy_gateway_paths<R: Runtime>(app: &AppHandle<R>) -> Result<ProxyGatewayPaths, String> {
    let app_data_dir = crate::app_paths::resolved_data_dir();
    let _ = app; // data dir is resolved from the bootstrap override cache
    Ok(ProxyGatewayPaths::new(app_data_dir))
}

fn emit_gateway_cli_config_changed<R: Runtime>(app: &AppHandle<R>, payload: &'static str) {
    if let Err(error) = app.emit("config-changed", payload) {
        log::warn!("Failed to emit config-changed after gateway CLI config change: {error}");
    }
}

fn emit_gateway_cli_wsl_sync_request<R: Runtime>(app: &AppHandle<R>, cli_key: GatewayCliKey) {
    let event_name = match cli_key {
        GatewayCliKey::Claude => "wsl-sync-request-claude",
        GatewayCliKey::ClaudeDesktop => "wsl-sync-request-claudedesktop",
        GatewayCliKey::Codex => "wsl-sync-request-codex",
        GatewayCliKey::Grok => "wsl-sync-request-grok",
        GatewayCliKey::Kimi => "wsl-sync-request-kimi",
        GatewayCliKey::Gemini => "wsl-sync-request-geminicli",
        GatewayCliKey::Antigravity => "wsl-sync-request-antigravity",
        GatewayCliKey::Workbuddy => "wsl-sync-request-workbuddy",
        GatewayCliKey::Codebuddy => "wsl-sync-request-codebuddy",
        GatewayCliKey::OpenCode => return,
    };
    if let Err(error) = app.emit(event_name, ()) {
        log::warn!("Failed to emit {event_name} after gateway CLI config change: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_apply_from_tray_keeps_tray_refresh() {
        assert_eq!(
            direct_provider_apply_event_plan(true),
            ProviderSwitchEventPlan {
                direct_apply_mode: DirectProviderApplyMode::WithEvents { from_tray: true },
                final_config_changed_payload: None,
                emit_final_wsl_sync_request: false,
            }
        );
    }

    #[test]
    fn gateway_takeover_switch_suppresses_intermediate_apply_events() {
        assert_eq!(
            gateway_takeover_switch_event_plan(true).direct_apply_mode,
            DirectProviderApplyMode::WithoutEvents
        );
    }

    #[test]
    fn gateway_takeover_switch_emits_single_final_tray_refresh() {
        assert_eq!(
            gateway_takeover_switch_event_plan(true),
            ProviderSwitchEventPlan {
                direct_apply_mode: DirectProviderApplyMode::WithoutEvents,
                final_config_changed_payload: Some("tray"),
                emit_final_wsl_sync_request: true,
            }
        );
    }

    #[test]
    fn gateway_takeover_switch_uses_window_payload_outside_tray() {
        assert_eq!(
            gateway_takeover_switch_event_plan(false).final_config_changed_payload,
            Some("window")
        );
    }

    #[test]
    fn provider_write_snapshot_round_trips_the_aggregate_manifest() {
        // The frontend sends the snapshot back verbatim after the save. If the
        // camelCase field names or the aggregate fields drift, the replay
        // silently degrades to single/failover and drops the user's selection.
        let snapshot = GatewayProviderWriteSnapshot {
            mode: crate::coding::proxy_gateway::types::GatewayProxyMode::Aggregate,
            primary_provider_id: None,
            aggregate: Some(
                crate::coding::proxy_gateway::types::GatewayAggregateConfig {
                    provider_ids: vec!["site-a".to_string(), "site-b".to_string()],
                    separator: "|".to_string(),
                    aliases: [("gpt".to_string(), "site-a|gpt".to_string())]
                        .into_iter()
                        .collect(),
                    naming: crate::coding::proxy_gateway::aggregate_naming::AggregateNamingMode::ModelAtSite,
                    cross_site_failover: true,
                    subagent_exposed_models: vec!["gpt".to_string()],
                    subagent: Some(
                        crate::coding::proxy_gateway::types::GatewayAggregateSubagentDefaults {
                            model: Some("site-a|gpt".to_string()),
                            reasoning_effort: Some("high".to_string()),
                        },
                    ),
                },
            ),
        };
        let serialized = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(serialized["mode"], "aggregate");
        assert_eq!(serialized["aggregate"]["provider_ids"][1], "site-b");
        assert_eq!(serialized["aggregate"]["cross_site_failover"], true);
        assert_eq!(
            serialized["aggregate"]["subagent"]["reasoning_effort"],
            "high"
        );
        let restored: GatewayProviderWriteSnapshot = serde_json::from_value(serialized).unwrap();
        assert_eq!(restored, snapshot);
    }
}
