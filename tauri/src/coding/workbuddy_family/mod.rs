//! Shared WorkBuddy-family facts used by both the desktop app and the CLI.
//!
//! WorkBuddy desktop and CodeBuddy Code are two front ends over one engine, so
//! the pieces that must agree live here instead of being written twice:
//!
//! - `models_json`: the custom-model contract. Both products read the same file
//!   shape and the same `availableModels` semantics.
//! - `channel`: the "provider" view derived by grouping model entries, which is
//!   what both pages render.
//! - `runtime_config`: the payload both pages load.

pub mod channel;
pub mod gateway_provider;
pub mod models_json;
pub mod runtime_config;

/// Rejects a direct write to `models.json` while the gateway owns that file.
///
/// Takeover rewrites every custom entry's `url`/`apiKey` and keeps a backup of
/// the original file; restore overwrites the whole file from that backup. A
/// model edited in the page during takeover would be written into the rewritten
/// file and then discarded wholesale on restore, with no warning. Same rule as
/// Kimi's `ensure_gateway_direct_for_paths`.
pub fn ensure_models_file_direct(cli_key: crate::coding::proxy_gateway::types::GatewayCliKey) -> Result<(), String> {
    let paths = crate::coding::proxy_gateway::paths::ProxyGatewayPaths::new(
        crate::app_paths::resolved_data_dir(),
    );
    if crate::coding::proxy_gateway::cli_proxy::provider_switch_locked_by_manifest(&paths, cli_key) {
        let product = match cli_key {
            crate::coding::proxy_gateway::types::GatewayCliKey::Codebuddy => "CodeBuddy",
            _ => "WorkBuddy",
        };
        return Err(format!(
            "{product} is currently served by the proxy gateway; switch channels through the gateway entry, or restore direct mode first"
        ));
    }
    Ok(())
}
