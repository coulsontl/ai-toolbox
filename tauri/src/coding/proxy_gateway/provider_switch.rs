use super::cli_proxy;
use super::paths::ProxyGatewayPaths;
use super::provider_protocol;
use super::runtime::ProxyGatewayState;
use super::types::{GatewayCliKey, GatewayCliTakeoverStatus, GatewayProxyMode, ProxyGatewayStatus};
use crate::db::helpers::db_get;
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;
use tauri::{AppHandle, Emitter, Manager, Runtime};

pub async fn apply_or_switch_provider<R: Runtime>(
    app: &AppHandle<R>,
    cli_key: GatewayCliKey,
    provider_id: &str,
    from_tray: bool,
) -> Result<GatewayCliTakeoverStatus, String> {
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

    cli_proxy::ensure_proxyable_provider(db, cli_key, provider_id).await?;
    cli_proxy::restore_cli_direct(db, &paths, cli_key, &gateway_status).await?;
    let event_plan = gateway_takeover_switch_event_plan(from_tray);
    apply_provider_with_plan(app, cli_key, provider_id, event_plan).await?;

    let refreshed_gateway_status = current_gateway_status(&gateway_state)?;
    let mut next_status = cli_proxy::engage_single_cli(
        db,
        &paths,
        cli_key,
        &refreshed_gateway_status,
        provider_id.to_string(),
    )
    .await?;

    if previous_mode == GatewayProxyMode::Failover {
        let refreshed_gateway_status = current_gateway_status(&gateway_state)?;
        next_status =
            cli_proxy::engage_failover_cli(db, &paths, cli_key, &refreshed_gateway_status).await?;
    }

    gateway_state.clear_provider_cache()?;
    if let Some(payload) = event_plan.final_config_changed_payload {
        emit_gateway_cli_config_changed(app, payload);
    }
    if event_plan.emit_final_wsl_sync_request {
        emit_gateway_cli_wsl_sync_request(app, cli_key);
    }
    Ok(next_status)
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

fn provider_table(cli_key: GatewayCliKey) -> Option<DbTable> {
    match cli_key {
        GatewayCliKey::Claude => Some(DbTable::ClaudeProvider),
        GatewayCliKey::Codex => Some(DbTable::CodexProvider),
        GatewayCliKey::Grok => Some(DbTable::GrokProvider),
        GatewayCliKey::Gemini => Some(DbTable::GeminiCliProvider),
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
        GatewayCliKey::OpenCode => Err("This CLI is not supported by the gateway MVP".to_string()),
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
        GatewayCliKey::Gemini => {
            crate::coding::gemini_cli::commands::apply_config_internal_without_events(
                &db,
                app,
                provider_id,
            )
            .await
        }
        GatewayCliKey::OpenCode => Err("This CLI is not supported by the gateway MVP".to_string()),
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
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Failed to resolve app data directory: {error}"))?;
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
        GatewayCliKey::Codex => "wsl-sync-request-codex",
        GatewayCliKey::Grok => "wsl-sync-request-grok",
        GatewayCliKey::Gemini => "wsl-sync-request-geminicli",
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
}
