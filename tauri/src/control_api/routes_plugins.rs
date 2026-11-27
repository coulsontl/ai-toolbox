//! `/v1/plugins/*` routes. Every mutation goes through the same Tauri command
//! the desktop page uses, so installs, runtime reloads and event broadcasts
//! keep a single implementation.

use std::collections::HashMap;

use hyper::Method;
use serde_json::{json, Value};
use tauri::Manager;

use super::approval::RiskLevel;
use super::error::{ApiError, ApiResult};
use super::redact::redact_value;
use super::routes::{approve_write, body_bool, body_optional_string};
use super::ServerContext;
use crate::plugins::installer;
use crate::plugins::store;
use crate::plugins::{commands as plugin_commands, PluginManagerState};

pub async fn dispatch(
    context: &ServerContext,
    method: &Method,
    segments: &[String],
    query: &HashMap<String, String>,
    body: Value,
) -> ApiResult<Value> {
    let tail = segments.get(1).map(String::as_str).unwrap_or_default();
    match (method, tail) {
        (&Method::GET, "") => {
            let records = plugin_commands::plugins_list_installed(manager_state(context))
                .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(records)?))
        }
        (&Method::POST, "install") => install(context, body).await,
        (&Method::POST, "reload") => reload(context, body).await,
        (&Method::GET, "providers") => {
            let listing = plugin_commands::plugins_list_providers(manager_state(context))
                .await
                .map_err(ApiError::from)?;
            Ok(redacted(listing))
        }
        // Global refresh route: `aitb plugin refresh-models` posts here.
        (&Method::POST, "providers")
            if segments.get(2).map(String::as_str) == Some("refresh-models") =>
        {
            refresh_models(context, None).await
        }
        (&Method::GET, "accounts") => {
            let accounts = plugin_commands::plugins_list_accounts(manager_state(context))
                .await
                .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(accounts)?))
        }
        (&Method::GET, "logs") => {
            let limit = query
                .get("limit")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(200)
                .min(2_000);
            let logs = plugin_commands::plugins_runtime_logs(manager_state(context), Some(limit))
                .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(logs)?))
        }
        (&Method::DELETE, "logs") => {
            approve_write(
                context,
                RiskLevel::L1,
                "plugin.logs.clear",
                "Clear the plugin runtime log buffer",
                None,
                false,
            )
            .await?;
            plugin_commands::plugins_clear_runtime_logs(manager_state(context))
                .map_err(ApiError::from)?;
            Ok(json!({"ok": true}))
        }
        (&Method::GET, "middleware") => {
            let states = plugin_commands::plugins_middleware_states(manager_state(context))
                .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(states)?))
        }
        _ => dispatch_nested(context, method, segments, query, body).await,
    }
}

async fn dispatch_nested(
    context: &ServerContext,
    method: &Method,
    segments: &[String],
    query: &HashMap<String, String>,
    body: Value,
) -> ApiResult<Value> {
    let tail = segments.get(1).map(String::as_str).unwrap_or_default();
    let second = segments.get(2).map(String::as_str).unwrap_or_default();
    let third = segments.get(3).map(String::as_str).unwrap_or_default();

    match (method, tail, second, third) {
        // /plugins/providers/:provider/options
        (&Method::GET, "providers", provider, "options") => {
            let options = store::list_provider_options(manager_state(context).db())
                .map_err(ApiError::from)?
                .into_iter()
                .find(|option| option.provider_id == provider)
                .map(|option| option.options)
                .unwrap_or_else(|| json!({}));
            Ok(redacted(json!({
                "provider": provider,
                "options": options,
            })))
        }
        (&Method::PUT, "providers", provider, "options") => {
            approve_write(
                context,
                RiskLevel::L1,
                "plugin.provider.options",
                format!("Update plugin provider options for '{provider}'"),
                Some(provider.to_string()),
                false,
            )
            .await?;
            let options = body
                .get("options")
                .cloned()
                .ok_or_else(|| ApiError::bad_request("'options' is required"))?;
            let saved = plugin_commands::plugins_set_provider_option(
                manager_state(context),
                provider.to_string(),
                options,
            )
            .await
            .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(saved)?))
        }
        // /plugins/providers/:provider/refresh-models
        (&Method::POST, "providers", provider, "refresh-models") => {
            refresh_models(context, Some(provider)).await
        }
        // /plugins/providers/:provider/link-cli
        (&Method::POST, "providers", provider, "link-cli") => {
            // Parse and validate before asking the user: an approval summary
            // built from the path alone can be spoofed by a body that names a
            // different provider than the URL.
            let input: crate::plugins::cli_integration::AddPluginProviderToCliInput =
                serde_json::from_value(body).map_err(|error| {
                    ApiError::bad_request(format!("Invalid link-cli payload: {error}"))
                })?;
            validate_link_cli(provider, &input)?;
            approve_write(
                context,
                RiskLevel::L1,
                "plugin.provider.link_cli",
                format!(
                    "Add plugin provider '{}' from '{}' to '{}'",
                    input.provider_id, input.plugin_spec, input.cli_key
                ),
                Some(provider.to_string()),
                false,
            )
            .await?;
            let result = crate::plugins::cli_integration::plugins_add_provider_to_cli(
                context.app.clone(),
                context.app.state::<crate::db::SqliteDbState>(),
                manager_state(context),
                context
                    .app
                    .state::<crate::coding::proxy_gateway::ProxyGatewayState>(),
                input,
            )
            .await
            .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(result)?))
        }
        // /plugins/accounts/check|usage|checkin
        (&Method::POST, "accounts", "check", "") => {
            approve_write(
                context,
                RiskLevel::L1,
                "plugin.account.check",
                "Check plugin account status",
                body.get("provider")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                false,
            )
            .await?;
            account_call(context, body, AccountCall::Check).await
        }
        (&Method::POST, "accounts", "usage", "") => {
            approve_write(
                context,
                RiskLevel::L1,
                "plugin.account.usage",
                "Fetch plugin account usage",
                body.get("provider")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                false,
            )
            .await?;
            account_call(context, body, AccountCall::Usage).await
        }
        (&Method::POST, "accounts", "checkin", "") => {
            approve_write(
                context,
                RiskLevel::L1,
                "plugin.account.checkin",
                "Run a plugin account check-in",
                body.get("provider")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                false,
            )
            .await?;
            account_call(context, body, AccountCall::Checkin).await
        }
        // /plugins/:id/enable|disable|options
        (&Method::POST, id, "enable", "") => {
            approve_write(
                context,
                RiskLevel::L1,
                "plugin.enable",
                format!("Enable plugin '{id}'"),
                Some(id.to_string()),
                false,
            )
            .await?;
            let record =
                plugin_commands::plugins_set_enabled(manager_state(context), id.to_string(), true)
                    .await
                    .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(record)?))
        }
        (&Method::POST, id, "disable", "") => {
            approve_write(
                context,
                RiskLevel::L1,
                "plugin.disable",
                format!("Disable plugin '{id}'"),
                Some(id.to_string()),
                false,
            )
            .await?;
            let record =
                plugin_commands::plugins_set_enabled(manager_state(context), id.to_string(), false)
                    .await
                    .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(record)?))
        }
        (&Method::GET, id, "options", "") => {
            let manager = manager_state(context);
            let record = store::get_installed_plugin(manager.db(), id)
                .map_err(ApiError::from)?
                .ok_or_else(|| ApiError::not_found(format!("Plugin '{id}' is not installed")))?;
            Ok(redacted(record.options))
        }
        (&Method::PUT, id, "options", "") => {
            approve_write(
                context,
                RiskLevel::L1,
                "plugin.options",
                format!("Update plugin options for '{id}'"),
                Some(id.to_string()),
                false,
            )
            .await?;
            let options = body
                .get("options")
                .cloned()
                .ok_or_else(|| ApiError::bad_request("'options' is required"))?;
            let record = plugin_commands::plugins_set_options(
                manager_state(context),
                id.to_string(),
                options,
            )
            .await
            .map_err(ApiError::from)?;
            Ok(redacted(serde_json::to_value(record)?))
        }
        // /plugins/:id
        (&Method::GET, id, "", "") => {
            let manager = manager_state(context);
            let record = store::get_installed_plugin(manager.db(), id)
                .map_err(ApiError::from)?
                .ok_or_else(|| ApiError::not_found(format!("Plugin '{id}' is not installed")))?;
            Ok(redacted(serde_json::to_value(record)?))
        }
        (&Method::DELETE, id, "", "") => {
            approve_write(
                context,
                RiskLevel::L2,
                "plugin.uninstall",
                format!("Uninstall plugin '{id}'"),
                Some(id.to_string()),
                false,
            )
            .await?;
            let purge_data = purge_data_requested(query, &body);
            plugin_commands::plugins_uninstall(
                manager_state(context),
                id.to_string(),
                Some(purge_data),
            )
            .await
            .map_err(ApiError::from)?;
            Ok(json!({"ok": true, "id": id, "purgeData": purge_data}))
        }
        _ => Err(ApiError::not_found(format!(
            "Unknown plugin route '/{}'",
            segments.join("/")
        ))),
    }
}

/// `aitb plugin remove --purge-data` sends a query parameter; older callers
/// send a JSON body. Query wins when both are present.
fn purge_data_requested(query: &HashMap<String, String>, body: &Value) -> bool {
    query
        .get("purgeData")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
        .or_else(|| body_bool(body, "purgeData"))
        .unwrap_or(false)
}

/// Validates the body before the approval summary is built: a summary from the
/// path alone could describe one provider while the body adds another.
fn validate_link_cli(
    provider: &str,
    input: &crate::plugins::cli_integration::AddPluginProviderToCliInput,
) -> ApiResult<()> {
    if input.provider_id.trim().is_empty() {
        return Err(ApiError::bad_request("'providerId' is required"));
    }
    if input.provider_id != provider {
        return Err(ApiError::bad_request(format!(
            "Path provider '{provider}' does not match body providerId '{}'",
            input.provider_id
        )));
    }
    if input.plugin_spec.trim().is_empty() {
        return Err(ApiError::bad_request("'pluginSpec' is required"));
    }
    if input.cli_key.trim().is_empty() {
        return Err(ApiError::bad_request("'cliKey' is required"));
    }
    Ok(())
}

async fn refresh_models(
    context: &ServerContext,
    provider: Option<&str>,
) -> ApiResult<Value> {
    let summary = match provider {
        Some(provider) => format!("Refresh models for plugin provider '{provider}'"),
        None => "Refresh models for every plugin provider".to_string(),
    };
    approve_write(
        context,
        RiskLevel::L1,
        "plugin.provider.refresh_models",
        summary,
        provider.map(str::to_string),
        false,
    )
    .await?;
    let listing = plugin_commands::plugins_refresh_provider_models(manager_state(context))
        .await
        .map_err(ApiError::from)?;
    Ok(redacted(listing))
}

async fn install(context: &ServerContext, body: Value) -> ApiResult<Value> {
    let spec = super::routes::body_string(&body, "spec")?;
    let market_id = body_optional_string(&body, "marketId");
    let source = installer::parse_source(&spec).map_err(ApiError::bad_request)?;
    let risk = match source {
        installer::PluginSource::Local { .. } => RiskLevel::L1,
        _ => RiskLevel::L2,
    };
    approve_write(
        context,
        risk,
        "plugin.install",
        format!("Install plugin from '{spec}'"),
        Some(spec.clone()),
        false,
    )
    .await?;
    let record = plugin_commands::plugins_install(manager_state(context), spec, market_id)
        .await
        .map_err(ApiError::from)?;
    Ok(redacted(serde_json::to_value(record)?))
}

async fn reload(context: &ServerContext, body: Value) -> ApiResult<Value> {
    let id = body_optional_string(&body, "id");
    let local_reload = id
        .as_deref()
        .and_then(|id| {
            store::get_installed_plugin(manager_state(context).db(), id)
                .ok()
                .flatten()
        })
        .map(|record| record.source_type == "local")
        .unwrap_or(false);
    approve_write(
        context,
        RiskLevel::L1,
        "plugin.reload",
        match &id {
            Some(id) => format!("Reload plugin '{id}' and its runtimes"),
            None => "Reload all plugins and runtimes".to_string(),
        },
        id.clone(),
        local_reload,
    )
    .await?;
    let records = crate::plugins::commands::reload_plugins(
        &manager_state(context).inner().clone(),
        &context.app,
        id.as_deref(),
    )
    .await
    .map_err(ApiError::from)?;
    Ok(redacted(serde_json::to_value(records)?))
}

enum AccountCall {
    Check,
    Usage,
    Checkin,
}

async fn account_call(context: &ServerContext, body: Value, call: AccountCall) -> ApiResult<Value> {
    let provider = super::routes::body_string(&body, "provider")?;
    let account = body_optional_string(&body, "account");
    let proxy = body_optional_string(&body, "proxy");
    let state = manager_state(context);
    let value = match call {
        AccountCall::Check => {
            plugin_commands::plugins_account_check(state, provider, account, proxy)
                .await
                .map_err(ApiError::from)?
        }
        AccountCall::Usage => {
            plugin_commands::plugins_account_usage(state, provider, account, proxy)
                .await
                .map_err(ApiError::from)?
        }
        AccountCall::Checkin => {
            plugin_commands::plugins_account_checkin(state, provider, account, proxy)
                .await
                .map_err(ApiError::from)?
        }
    };
    Ok(redacted(value))
}

fn manager_state(context: &ServerContext) -> tauri::State<'_, PluginManagerState> {
    context.app.state::<PluginManagerState>()
}

fn redacted(mut value: Value) -> Value {
    redact_value(&mut value);
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::cli_integration::AddPluginProviderToCliInput;
    use crate::plugins::gateway_bridge::PluginAccountStrategy;

    fn link_input(provider_id: &str, plugin_spec: &str) -> AddPluginProviderToCliInput {
        AddPluginProviderToCliInput {
            cli_key: "codex".to_string(),
            plugin_spec: plugin_spec.to_string(),
            provider_id: provider_id.to_string(),
            display_name: Some("Demo".to_string()),
            account_strategy: PluginAccountStrategy::default(),
            account_key: None,
            model_ids: Vec::new(),
        }
    }

    #[test]
    fn purge_data_accepts_query_or_body_and_query_wins() {
        let mut query = HashMap::new();
        assert!(!purge_data_requested(&query, &json!({})));
        assert!(purge_data_requested(&query, &json!({"purgeData": true})));

        query.insert("purgeData".to_string(), "1".to_string());
        assert!(purge_data_requested(&query, &json!({"purgeData": false})));

        // An explicit false in the query beats a true body.
        query.insert("purgeData".to_string(), "false".to_string());
        assert!(!purge_data_requested(&query, &json!({"purgeData": true})));
    }

    #[test]
    fn link_cli_requires_matching_provider_and_a_plugin_spec() {
        assert!(validate_link_cli("demo", &link_input("demo", "@acme/plugin")).is_ok());

        let mismatch = validate_link_cli("demo", &link_input("other", "@acme/plugin")).unwrap_err();
        assert_eq!(mismatch.status, 400);

        let empty_spec = validate_link_cli("demo", &link_input("demo", "   ")).unwrap_err();
        assert_eq!(empty_spec.status, 400);

        let mut missing_cli = link_input("demo", "@acme/plugin");
        missing_cli.cli_key = String::new();
        assert_eq!(validate_link_cli("demo", &missing_cli).unwrap_err().status, 400);
    }
}
