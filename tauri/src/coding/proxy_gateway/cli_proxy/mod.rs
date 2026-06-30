pub mod manifest;

use self::manifest::{validate_backup_rel_path, CliProxyManifest, CliProxyManifestFile};
use super::paths::ProxyGatewayPaths;
use super::runtime::{
    load_candidate_providers, load_candidate_providers_with_settings_and_selection,
    provider_priority_entries, GatewayProviderSelection, UpstreamProvider,
};
use super::settings;
use super::types::{
    GatewayCliKey, GatewayCliStatusDot, GatewayCliTakeoverState, GatewayCliTakeoverStatus,
    GatewayManagedTarget, GatewayProxyMode, ProviderPriorityEntry, ProxyGatewaySettings,
    ProxyGatewayStatus, ProxyGatewayStopPreflight,
};
use crate::coding::runtime_location::{self, RuntimeLocationMode};
use crate::db::SqliteDbState;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut, Item};

const GATEWAY_PROVIDER_ID: &str = "ai-toolbox-gateway";
const GATEWAY_API_KEY: &str = "ai-toolbox-gateway";
const CLAUDE_STANDARD_MODEL: &str = "claude-sonnet-4-6";
const CLAUDE_STANDARD_HAIKU_MODEL: &str = "claude-haiku-4-5-20251001";
const CLAUDE_STANDARD_SONNET_MODEL: &str = "claude-sonnet-4-6";
const CLAUDE_STANDARD_OPUS_MODEL: &str = "claude-opus-4-7";
const CLAUDE_SETTINGS_KIND: &str = "claude_settings_json";
const CODEX_CONFIG_KIND: &str = "codex_config_toml";
const CODEX_AUTH_KIND: &str = "codex_auth_json";
const GEMINI_ENV_KIND: &str = "gemini_env";
const GEMINI_SETTINGS_KIND: &str = "gemini_settings_json";

const CLAUDE_MANAGED_FIELDS: [&str; 10] = [
    "env.ANTHROPIC_BASE_URL",
    "env.ANTHROPIC_AUTH_TOKEN",
    "env.ANTHROPIC_API_KEY",
    "env.ANTHROPIC_MODEL",
    "env.ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "env.ANTHROPIC_DEFAULT_SONNET_MODEL",
    "env.ANTHROPIC_DEFAULT_OPUS_MODEL",
    "env.ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    "env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    "env.ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
];

const CLAUDE_MODEL_FIELD_POINTERS: [&str; 7] = [
    "/env/ANTHROPIC_MODEL",
    "/env/ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "/env/ANTHROPIC_DEFAULT_SONNET_MODEL",
    "/env/ANTHROPIC_DEFAULT_OPUS_MODEL",
    "/env/ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    "/env/ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    "/env/ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
];

const CLAUDE_LEGACY_REASONING_MODEL_POINTER: &str = "/env/ANTHROPIC_REASONING_MODEL";

const CODEX_CONFIG_MANAGED_FIELDS: [&str; 2] =
    ["model_provider", "model_providers.ai-toolbox-gateway"];

const CODEX_AUTH_MANAGED_FIELDS: [&str; 2] = ["OPENAI_API_KEY", "auth_mode"];

const GEMINI_MANAGED_ENV_KEYS: [&str; 14] = [
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_GEMINI_BASE_URL",
    "GOOGLE_VERTEX_BASE_URL",
    "GOOGLE_GENAI_USE_GCA",
    "GOOGLE_GENAI_USE_VERTEXAI",
    "GEMINI_CLI_USE_COMPUTE_ADC",
    "GEMINI_CLI_CUSTOM_HEADERS",
    "GEMINI_MODEL",
    "GEMINI_API_KEY_AUTH_MECHANISM",
    "GOOGLE_GENAI_API_VERSION",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_CLOUD_PROJECT_ID",
    "GOOGLE_CLOUD_LOCATION",
];

const GEMINI_SETTINGS_MANAGED_FIELDS: [&str; 1] = ["security.auth.selectedType"];
const NO_PROXYABLE_PROVIDER_MESSAGE: &str = "No proxyable providers are configured. Official subscription providers use CLI-native OAuth and cannot be routed through the gateway.";

#[derive(Debug, Clone)]
struct CliProxyTarget {
    kind: &'static str,
    path: PathBuf,
    managed_fields: &'static [&'static str],
}

#[derive(Debug, Clone)]
struct CliProxyTargets {
    runtime_root: PathBuf,
    is_wsl_direct: bool,
    files: Vec<CliProxyTarget>,
}

#[derive(Debug, Clone, Default)]
struct GatewayStatusProxyDetails {
    mode: Option<GatewayProxyMode>,
    primary_provider_id: Option<String>,
    provider_priorities: Vec<ProviderPriorityEntry>,
}

impl GatewayStatusProxyDetails {
    fn from_manifest(manifest: &CliProxyManifest) -> Self {
        Self {
            mode: Some(manifest.mode),
            primary_provider_id: Some(manifest.primary_provider_id.clone()),
            provider_priorities: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ManifestReadError {
    Io(String),
    ManifestNeedsReengage(String),
    Parse(String),
}

impl std::fmt::Display for ManifestReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) | Self::ManifestNeedsReengage(message) | Self::Parse(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl ManifestReadError {
    fn needs_reengage(&self) -> bool {
        matches!(self, Self::ManifestNeedsReengage(_))
    }
}

pub async fn cli_takeover_statuses(
    db: &SqliteDbState,
    paths: &ProxyGatewayPaths,
    gateway_status: &ProxyGatewayStatus,
) -> Vec<GatewayCliTakeoverStatus> {
    let mut statuses = Vec::new();
    for cli_key in GatewayCliKey::supported_mvp() {
        statuses.push(cli_takeover_status(db, paths, cli_key, gateway_status).await);
    }
    statuses
}

pub async fn cli_takeover_status(
    db: &SqliteDbState,
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    gateway_status: &ProxyGatewayStatus,
) -> GatewayCliTakeoverStatus {
    if !is_supported_cli(cli_key) {
        return build_status(
            cli_key,
            GatewayCliTakeoverState::Unsupported,
            GatewayCliStatusDot::Gray,
            false,
            false,
            gateway_status.base_url.clone(),
            None,
            Vec::new(),
            Some("This CLI is not supported by the gateway MVP".to_string()),
        );
    }

    let targets = match resolve_targets(db, cli_key).await {
        Ok(targets) => targets,
        Err(error) => {
            return build_status(
                cli_key,
                GatewayCliTakeoverState::Error,
                GatewayCliStatusDot::Red,
                false,
                false,
                gateway_status.base_url.clone(),
                None,
                Vec::new(),
                Some(error),
            )
        }
    };
    let managed_targets = managed_targets_from_current(&targets);

    let manifest = match read_manifest(paths, cli_key) {
        Ok(manifest) => manifest,
        Err(error) => {
            let can_takeover = error.needs_reengage() && gateway_status.running;
            let can_restore_direct = error.needs_reengage();
            return build_status(
                cli_key,
                GatewayCliTakeoverState::Error,
                GatewayCliStatusDot::Red,
                can_takeover,
                can_restore_direct,
                gateway_status.base_url.clone(),
                Some(path_to_string(&targets.runtime_root)),
                managed_targets,
                Some(error.to_string()),
            );
        }
    };

    let Some(manifest) = manifest.filter(|manifest| manifest.enabled) else {
        let has_proxyable_provider = match has_proxyable_provider(db, cli_key).await {
            Ok(has_provider) => has_provider,
            Err(error) => {
                return build_status(
                    cli_key,
                    GatewayCliTakeoverState::Error,
                    GatewayCliStatusDot::Red,
                    false,
                    false,
                    gateway_status.base_url.clone(),
                    Some(path_to_string(&targets.runtime_root)),
                    managed_targets,
                    Some(error),
                )
            }
        };
        if !has_proxyable_provider {
            return build_status(
                cli_key,
                GatewayCliTakeoverState::NoProxyProvider,
                GatewayCliStatusDot::Orange,
                false,
                false,
                gateway_status.base_url.clone(),
                Some(path_to_string(&targets.runtime_root)),
                managed_targets,
                Some(NO_PROXYABLE_PROVIDER_MESSAGE.to_string()),
            );
        }
        return build_status(
            cli_key,
            GatewayCliTakeoverState::Direct,
            GatewayCliStatusDot::Gray,
            gateway_status.running,
            false,
            gateway_status.base_url.clone(),
            Some(path_to_string(&targets.runtime_root)),
            managed_targets,
            Some(if gateway_status.running {
                "CLI is using its direct provider configuration".to_string()
            } else {
                "Start the gateway before taking over this CLI".to_string()
            }),
        );
    };

    let manifest_targets = managed_targets_from_manifest(&manifest, &targets);
    let proxy_details = proxy_details_for_manifest(db, cli_key, &manifest).await;
    let restore_available = manifest_restore_available(paths, cli_key, &manifest);
    if !restore_available {
        return build_status_with_proxy_details(
            cli_key,
            GatewayCliTakeoverState::RestoreUnavailable,
            GatewayCliStatusDot::Red,
            false,
            false,
            Some(manifest.base_origin.clone()),
            Some(path_to_string(&targets.runtime_root)),
            manifest_targets,
            proxy_details,
            Some(
                "Gateway takeover manifest exists, but one or more backups are missing".to_string(),
            ),
        );
    }

    if !gateway_status.running {
        return build_status_with_proxy_details(
            cli_key,
            GatewayCliTakeoverState::GatewayStopped,
            GatewayCliStatusDot::Orange,
            false,
            true,
            Some(manifest.base_origin.clone()),
            Some(path_to_string(&targets.runtime_root)),
            manifest_targets,
            proxy_details,
            Some("Gateway is stopped while this CLI is still routed through it".to_string()),
        );
    }

    let has_proxyable_provider = match has_proxyable_provider(db, cli_key).await {
        Ok(has_provider) => has_provider,
        Err(error) => {
            return build_status_with_proxy_details(
                cli_key,
                GatewayCliTakeoverState::Error,
                GatewayCliStatusDot::Red,
                false,
                true,
                gateway_status
                    .base_url
                    .clone()
                    .or(Some(manifest.base_origin.clone())),
                Some(path_to_string(&targets.runtime_root)),
                manifest_targets,
                proxy_details,
                Some(error),
            )
        }
    };

    let current_origin = current_cli_gateway_endpoint(cli_key, &targets)
        .ok()
        .flatten();
    let expected_current = gateway_status.base_url.as_deref().map(|base_origin| {
        let effective_origin = settings::load_settings_from_sqlite_state(db)
            .map(|settings| {
                resolve_effective_base_origin(
                    base_origin,
                    targets.is_wsl_direct,
                    &settings.wsl_host,
                )
            })
            .unwrap_or_else(|error| {
                log::warn!("Failed to resolve gateway settings for CLI status: {error}");
                base_origin.to_string()
            });
        cli_gateway_endpoint(cli_key, &effective_origin)
    });
    let expected_manifest = cli_gateway_endpoint(cli_key, &manifest.base_origin);

    let (state, dot, message) = if current_origin.as_deref() == expected_current.as_deref() {
        (
            GatewayCliTakeoverState::TakeoverApplied,
            GatewayCliStatusDot::Green,
            Some("CLI is currently routed through the running gateway".to_string()),
        )
    } else if current_origin.as_deref() == Some(expected_manifest.as_str()) {
        (
            GatewayCliTakeoverState::OutdatedOrigin,
            GatewayCliStatusDot::Orange,
            Some(
                "Gateway listen address changed; take over again to refresh CLI config".to_string(),
            ),
        )
    } else {
        (
            GatewayCliTakeoverState::Drifted,
            GatewayCliStatusDot::Orange,
            Some("CLI config no longer matches the gateway manifest; take over again or restore direct mode".to_string()),
        )
    };

    let (state, dot, message) = if !has_proxyable_provider {
        (
            GatewayCliTakeoverState::NoProxyProvider,
            GatewayCliStatusDot::Orange,
            Some(NO_PROXYABLE_PROVIDER_MESSAGE.to_string()),
        )
    } else {
        (state, dot, message)
    };

    build_status_with_proxy_details(
        cli_key,
        state,
        dot,
        gateway_status.running && has_proxyable_provider,
        true,
        gateway_status
            .base_url
            .clone()
            .or(Some(manifest.base_origin)),
        Some(path_to_string(&targets.runtime_root)),
        manifest_targets,
        proxy_details,
        message,
    )
}

pub async fn engage_single_cli(
    db: &SqliteDbState,
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    gateway_status: &ProxyGatewayStatus,
    primary_provider_id: String,
) -> Result<GatewayCliTakeoverStatus, String> {
    if !is_supported_cli(cli_key) {
        return Err("This CLI is not supported by the gateway MVP".to_string());
    }
    let Some(base_origin) = gateway_status.base_url.as_deref() else {
        return Err("Start the proxy gateway before enabling Gateway proxy".to_string());
    };
    if !gateway_status.running {
        return Err("Start the proxy gateway before enabling Gateway proxy".to_string());
    }
    let primary_provider = load_proxyable_provider(db, cli_key, &primary_provider_id).await?;

    let targets = resolve_targets(db, cli_key).await?;
    let settings = settings::load_settings_from_sqlite_state(db)?;
    let effective_origin =
        resolve_effective_base_origin(base_origin, targets.is_wsl_direct, &settings.wsl_host);
    let manifest = prepare_manifest(
        paths,
        cli_key,
        &effective_origin,
        &targets,
        GatewayProxyMode::Single,
        &primary_provider_id,
    )?;
    let codex_auth_backup_content = codex_auth_backup_content_for_cli(paths, cli_key, &manifest)?;
    apply_gateway_config(
        cli_key,
        &targets,
        &effective_origin,
        Some(&primary_provider),
        GatewayProxyMode::Single,
        None,
        codex_auth_backup_content.as_deref(),
        codex_auth_preservation_enabled_for_cli(db, cli_key)?,
    )?;
    write_manifest(paths, cli_key, &manifest)?;
    Ok(cli_takeover_status(db, paths, cli_key, gateway_status).await)
}

pub async fn engage_failover_cli(
    db: &SqliteDbState,
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    gateway_status: &ProxyGatewayStatus,
) -> Result<GatewayCliTakeoverStatus, String> {
    if !is_supported_cli(cli_key) {
        return Err("This CLI is not supported by the gateway MVP".to_string());
    }
    let Some(mut manifest) = read_manifest(paths, cli_key).map_err(|error| error.to_string())?
    else {
        return Err(
            "Enable Gateway proxy on the applied provider before enabling failover".to_string(),
        );
    };
    if !manifest.enabled {
        return Err(
            "Enable Gateway proxy on the applied provider before enabling failover".to_string(),
        );
    }
    if manifest.mode == GatewayProxyMode::Single {
        let primary_provider =
            load_proxyable_provider(db, cli_key, &manifest.primary_provider_id).await?;
        let targets = resolve_targets(db, cli_key).await?;
        let codex_auth_backup_content =
            codex_auth_backup_content_for_cli(paths, cli_key, &manifest)?;
        apply_gateway_config(
            cli_key,
            &targets,
            &manifest.base_origin,
            Some(&primary_provider),
            GatewayProxyMode::Failover,
            None,
            codex_auth_backup_content.as_deref(),
            codex_auth_preservation_enabled_for_cli(db, cli_key)?,
        )?;
        manifest.mode = GatewayProxyMode::Failover;
        manifest.updated_at = chrono::Utc::now().to_rfc3339();
        write_manifest(paths, cli_key, &manifest)?;
    }
    Ok(cli_takeover_status(db, paths, cli_key, gateway_status).await)
}

pub async fn disengage_failover_cli(
    db: &SqliteDbState,
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    gateway_status: &ProxyGatewayStatus,
) -> Result<GatewayCliTakeoverStatus, String> {
    if !is_supported_cli(cli_key) {
        return Err("This CLI is not supported by the gateway MVP".to_string());
    }
    let Some(mut manifest) =
        read_manifest_for_reengage(paths, cli_key, GatewayProxyMode::Single, "legacy-provider")?
    else {
        return Ok(cli_takeover_status(db, paths, cli_key, gateway_status).await);
    };
    if manifest.enabled && manifest.mode == GatewayProxyMode::Failover {
        let primary_provider =
            load_proxyable_provider(db, cli_key, &manifest.primary_provider_id).await?;
        let targets = resolve_targets(db, cli_key).await?;
        let claude_backup_content = if cli_key == GatewayCliKey::Claude {
            backup_content(paths, cli_key, &manifest, CLAUDE_SETTINGS_KIND)?
                .or_else(|| Some("{}".to_string()))
        } else {
            None
        };
        let codex_auth_backup_content =
            codex_auth_backup_content_for_cli(paths, cli_key, &manifest)?;
        apply_gateway_config(
            cli_key,
            &targets,
            &manifest.base_origin,
            Some(&primary_provider),
            GatewayProxyMode::Single,
            claude_backup_content.as_deref(),
            codex_auth_backup_content.as_deref(),
            codex_auth_preservation_enabled_for_cli(db, cli_key)?,
        )?;
        manifest.mode = GatewayProxyMode::Single;
        manifest.updated_at = chrono::Utc::now().to_rfc3339();
        write_manifest(paths, cli_key, &manifest)?;
    }
    Ok(cli_takeover_status(db, paths, cli_key, gateway_status).await)
}

pub async fn restore_cli_direct(
    db: &SqliteDbState,
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    gateway_status: &ProxyGatewayStatus,
) -> Result<GatewayCliTakeoverStatus, String> {
    if !is_supported_cli(cli_key) {
        return Err("This CLI is not supported by the gateway MVP".to_string());
    }

    let targets = resolve_targets(db, cli_key).await?;
    let Some(mut manifest) =
        read_manifest_for_reengage(paths, cli_key, GatewayProxyMode::Single, "legacy-provider")?
    else {
        return Ok(cli_takeover_status(db, paths, cli_key, gateway_status).await);
    };
    if !manifest.enabled {
        return Ok(cli_takeover_status(db, paths, cli_key, gateway_status).await);
    }
    if !manifest_restore_available(paths, cli_key, &manifest) {
        return Err(
            "Cannot restore direct mode because one or more gateway backups are missing"
                .to_string(),
        );
    }

    restore_gateway_config(cli_key, paths, &targets, &manifest)?;
    manifest.enabled = false;
    manifest.updated_at = chrono::Utc::now().to_rfc3339();
    write_manifest(paths, cli_key, &manifest)?;
    Ok(cli_takeover_status(db, paths, cli_key, gateway_status).await)
}

pub async fn stop_preflight(
    db: &SqliteDbState,
    paths: &ProxyGatewayPaths,
    gateway_status: &ProxyGatewayStatus,
) -> ProxyGatewayStopPreflight {
    let statuses = cli_takeover_statuses(db, paths, gateway_status).await;
    let blocking_cli_takeovers: Vec<GatewayCliTakeoverStatus> =
        statuses.into_iter().filter(blocks_gateway_stop).collect();
    let allowed = blocking_cli_takeovers.is_empty();

    ProxyGatewayStopPreflight {
        allowed,
        message: if allowed {
            None
        } else {
            Some(
                "Restore gateway-taken-over CLIs to direct mode before stopping the gateway"
                    .to_string(),
            )
        },
        blocking_cli_takeovers,
    }
}

fn blocks_gateway_stop(status: &GatewayCliTakeoverStatus) -> bool {
    if status.can_restore_direct {
        return true;
    }
    matches!(
        status.state,
        GatewayCliTakeoverState::TakeoverApplied
            | GatewayCliTakeoverState::GatewayStopped
            | GatewayCliTakeoverState::OutdatedOrigin
            | GatewayCliTakeoverState::Drifted
            | GatewayCliTakeoverState::RestoreUnavailable
    )
}

pub fn provider_switch_locked_by_manifest(
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
) -> bool {
    match read_manifest(paths, cli_key) {
        Ok(Some(manifest)) => manifest.enabled,
        Ok(None) => false,
        Err(error) => error.needs_reengage(),
    }
}

pub fn wsl_synced_gateway_target_for_mapping(
    mapping_id: &str,
) -> Option<(GatewayCliKey, &'static str)> {
    match mapping_id {
        "claude-settings" => Some((GatewayCliKey::Claude, CLAUDE_SETTINGS_KIND)),
        "codex-config" => Some((GatewayCliKey::Codex, CODEX_CONFIG_KIND)),
        "geminicli-env" => Some((GatewayCliKey::Gemini, GEMINI_ENV_KIND)),
        _ => None,
    }
}

pub fn rewrite_wsl_synced_gateway_target_content(
    paths: &ProxyGatewayPaths,
    settings: &ProxyGatewaySettings,
    cli_key: GatewayCliKey,
    target_kind: &str,
    content: &str,
) -> Result<Option<String>, String> {
    let trimmed_wsl_host = settings.wsl_host.trim();
    if trimmed_wsl_host.is_empty() {
        return Ok(None);
    }

    let Some(manifest) = read_manifest(paths, cli_key)
        .map_err(|error| error.to_string())?
        .filter(|manifest| manifest.enabled)
    else {
        return Ok(None);
    };

    let Some(managed_file) = manifest.files.iter().find(|file| file.kind == target_kind) else {
        return Ok(None);
    };

    let wsl_origin = resolve_effective_base_origin(&manifest.base_origin, true, trimmed_wsl_host);
    if wsl_origin == manifest.base_origin {
        return Ok(None);
    }

    let windows_gateway_endpoint = cli_gateway_endpoint(cli_key, &manifest.base_origin);
    let wsl_gateway_endpoint = cli_gateway_endpoint(cli_key, &wsl_origin);

    match (cli_key, target_kind) {
        (GatewayCliKey::Claude, CLAUDE_SETTINGS_KIND)
            if managed_file
                .managed_fields
                .iter()
                .any(|field| field == "env.ANTHROPIC_BASE_URL") =>
        {
            rewrite_claude_wsl_gateway_content(
                content,
                &windows_gateway_endpoint,
                &wsl_gateway_endpoint,
            )
        }
        (GatewayCliKey::Codex, CODEX_CONFIG_KIND)
            if managed_file
                .managed_fields
                .iter()
                .any(|field| field == "model_providers.ai-toolbox-gateway") =>
        {
            rewrite_codex_wsl_gateway_content(
                content,
                &windows_gateway_endpoint,
                &wsl_gateway_endpoint,
            )
        }
        (GatewayCliKey::Gemini, GEMINI_ENV_KIND)
            if managed_file
                .managed_fields
                .iter()
                .any(|field| field == "GOOGLE_GEMINI_BASE_URL") =>
        {
            rewrite_gemini_wsl_gateway_content(
                content,
                &windows_gateway_endpoint,
                &wsl_gateway_endpoint,
            )
        }
        _ => Ok(None),
    }
}

async fn has_proxyable_provider(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
) -> Result<bool, String> {
    Ok(!load_candidate_providers(db, cli_key).await?.is_empty())
}

async fn load_proxyable_provider(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
    provider_id: &str,
) -> Result<UpstreamProvider, String> {
    load_candidate_providers(db, cli_key)
        .await?
        .into_iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| {
            format!(
                "Selected provider is not available for Gateway proxy. {NO_PROXYABLE_PROVIDER_MESSAGE}"
            )
        })
}

pub async fn ensure_proxyable_provider(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
    provider_id: &str,
) -> Result<(), String> {
    load_proxyable_provider(db, cli_key, provider_id)
        .await
        .map(|_| ())
}

fn is_supported_cli(cli_key: GatewayCliKey) -> bool {
    matches!(
        cli_key,
        GatewayCliKey::Claude | GatewayCliKey::Codex | GatewayCliKey::Gemini
    )
}

async fn resolve_targets(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
) -> Result<CliProxyTargets, String> {
    let location = match cli_key {
        GatewayCliKey::Claude => runtime_location::get_claude_runtime_location_async(db).await?,
        GatewayCliKey::Codex => runtime_location::get_codex_runtime_location_async(db).await?,
        GatewayCliKey::Gemini => {
            runtime_location::get_gemini_cli_runtime_location_async(db).await?
        }
        GatewayCliKey::OpenCode => {
            return Err(
                "OpenCode adapter is intentionally out of scope for the gateway MVP".to_string(),
            )
        }
    };
    let is_wsl_direct = location.mode == RuntimeLocationMode::WslDirect;
    let runtime_root = location.host_path;

    let files = match cli_key {
        GatewayCliKey::Claude => vec![CliProxyTarget {
            kind: CLAUDE_SETTINGS_KIND,
            path: runtime_root.join("settings.json"),
            managed_fields: &CLAUDE_MANAGED_FIELDS,
        }],
        GatewayCliKey::Codex => vec![
            CliProxyTarget {
                kind: CODEX_CONFIG_KIND,
                path: runtime_root.join("config.toml"),
                managed_fields: &CODEX_CONFIG_MANAGED_FIELDS,
            },
            CliProxyTarget {
                kind: CODEX_AUTH_KIND,
                path: runtime_root.join("auth.json"),
                managed_fields: &CODEX_AUTH_MANAGED_FIELDS,
            },
        ],
        GatewayCliKey::Gemini => vec![
            CliProxyTarget {
                kind: GEMINI_ENV_KIND,
                path: runtime_root.join(".env"),
                managed_fields: &GEMINI_MANAGED_ENV_KEYS,
            },
            CliProxyTarget {
                kind: GEMINI_SETTINGS_KIND,
                path: runtime_root.join("settings.json"),
                managed_fields: &GEMINI_SETTINGS_MANAGED_FIELDS,
            },
        ],
        GatewayCliKey::OpenCode => Vec::new(),
    };

    Ok(CliProxyTargets {
        runtime_root,
        is_wsl_direct,
        files,
    })
}

fn resolve_effective_base_origin(base_origin: &str, is_wsl_direct: bool, wsl_host: &str) -> String {
    let trimmed_wsl_host = wsl_host.trim();
    if !is_wsl_direct || trimmed_wsl_host.is_empty() {
        return base_origin.to_string();
    }
    replace_origin_host(base_origin, trimmed_wsl_host)
}

fn replace_origin_host(base_origin: &str, new_host: &str) -> String {
    let Some(scheme_separator) = base_origin.find("://") else {
        return base_origin.to_string();
    };
    let host_start = scheme_separator + 3;
    let Some(port_separator) = base_origin[host_start..].rfind(':') else {
        return base_origin.to_string();
    };
    let port_separator = host_start + port_separator;
    let port = &base_origin[port_separator..];
    if port.len() <= 1 {
        return base_origin.to_string();
    }
    format!("{}{}{}", &base_origin[..host_start], new_host, port)
}

fn build_status(
    cli_key: GatewayCliKey,
    state: GatewayCliTakeoverState,
    dot: GatewayCliStatusDot,
    can_takeover: bool,
    can_restore_direct: bool,
    gateway_origin: Option<String>,
    runtime_root: Option<String>,
    managed_targets: Vec<GatewayManagedTarget>,
    message: Option<String>,
) -> GatewayCliTakeoverStatus {
    build_status_with_proxy_details(
        cli_key,
        state,
        dot,
        can_takeover,
        can_restore_direct,
        gateway_origin,
        runtime_root,
        managed_targets,
        GatewayStatusProxyDetails::default(),
        message,
    )
}

fn build_status_with_proxy_details(
    cli_key: GatewayCliKey,
    state: GatewayCliTakeoverState,
    dot: GatewayCliStatusDot,
    can_takeover: bool,
    can_restore_direct: bool,
    gateway_origin: Option<String>,
    runtime_root: Option<String>,
    managed_targets: Vec<GatewayManagedTarget>,
    proxy_details: GatewayStatusProxyDetails,
    message: Option<String>,
) -> GatewayCliTakeoverStatus {
    GatewayCliTakeoverStatus {
        cli_key,
        state,
        dot,
        can_takeover,
        can_restore_direct,
        gateway_origin,
        runtime_root,
        managed_targets,
        mode: proxy_details.mode,
        primary_provider_id: proxy_details.primary_provider_id,
        provider_priorities: proxy_details.provider_priorities,
        message,
    }
}

async fn proxy_details_for_manifest(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
    manifest: &CliProxyManifest,
) -> GatewayStatusProxyDetails {
    let mut details = GatewayStatusProxyDetails::from_manifest(manifest);
    let selection = GatewayProviderSelection {
        mode: manifest.mode,
        primary_provider_id: manifest.primary_provider_id.clone(),
    };
    match load_candidate_providers_with_settings_and_selection(db, cli_key, None, Some(&selection))
        .await
    {
        Ok(providers) => {
            details.provider_priorities =
                priority_entries_for_manifest_providers(&providers, manifest);
        }
        Err(error) => {
            log::warn!("Failed to resolve gateway provider priorities: {error}");
        }
    }
    details
}

fn priority_entries_for_manifest_providers(
    providers: &[UpstreamProvider],
    manifest: &CliProxyManifest,
) -> Vec<ProviderPriorityEntry> {
    if providers.first().map(|provider| provider.id.as_str())
        == Some(manifest.primary_provider_id.as_str())
    {
        return provider_priority_entries(providers);
    }

    let first_index = match manifest.mode {
        GatewayProxyMode::Single => 0,
        GatewayProxyMode::Failover => 1,
    };
    providers
        .iter()
        .enumerate()
        .map(|(index, provider)| ProviderPriorityEntry {
            provider_id: provider.id.clone(),
            label: format!("P{}", index + first_index),
        })
        .collect()
}

fn managed_targets_from_current(targets: &CliProxyTargets) -> Vec<GatewayManagedTarget> {
    targets
        .files
        .iter()
        .map(|target| GatewayManagedTarget {
            kind: target.kind.to_string(),
            path: path_to_string(&target.path),
            existed: target.path.exists(),
        })
        .collect()
}

fn managed_targets_from_manifest(
    manifest: &CliProxyManifest,
    targets: &CliProxyTargets,
) -> Vec<GatewayManagedTarget> {
    if !manifest.files.is_empty() {
        return manifest
            .files
            .iter()
            .map(|file| GatewayManagedTarget {
                kind: file.kind.clone(),
                path: file.path.clone(),
                existed: file.existed,
            })
            .collect();
    }
    managed_targets_from_current(targets)
}

fn read_manifest(
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
) -> Result<Option<CliProxyManifest>, ManifestReadError> {
    let manifest_path = paths.manifest_path(cli_key);
    if !manifest_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&manifest_path).map_err(|error| {
        ManifestReadError::Io(format!(
            "Failed to read gateway manifest {}: {}",
            manifest_path.display(),
            error
        ))
    })?;
    let manifest = serde_json::from_str::<CliProxyManifest>(&content).map_err(|error| {
        let error_text = error.to_string();
        let message = if error_text.contains("missing field `mode`")
            || error_text.contains("missing field `primary_provider_id`")
        {
            format!(
                "Gateway proxy manifest {} was created by an older AI Toolbox version. Click Gateway proxy on the applied provider again to re-engage this CLI.",
                manifest_path.display()
            )
        } else {
            format!(
                "Failed to parse gateway manifest {}: {}",
                manifest_path.display(),
                error
            )
        };
        if error_text.contains("missing field `mode`")
            || error_text.contains("missing field `primary_provider_id`")
        {
            ManifestReadError::ManifestNeedsReengage(message)
        } else {
            ManifestReadError::Parse(message)
        }
    })?;
    Ok(Some(manifest))
}

fn read_manifest_for_reengage(
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    mode: GatewayProxyMode,
    primary_provider_id: &str,
) -> Result<Option<CliProxyManifest>, String> {
    match read_manifest(paths, cli_key) {
        Ok(manifest) => Ok(manifest),
        Err(error) if error.needs_reengage() => {
            read_legacy_manifest_for_reengage(paths, cli_key, mode, primary_provider_id)
        }
        Err(error) => Err(error.to_string()),
    }
}

fn read_legacy_manifest_for_reengage(
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    mode: GatewayProxyMode,
    primary_provider_id: &str,
) -> Result<Option<CliProxyManifest>, String> {
    let manifest_path = paths.manifest_path(cli_key);
    if !manifest_path.exists() {
        return Ok(None);
    }
    let mut value = read_json_file(&manifest_path)?;
    let root = ensure_json_object(&mut value);
    root.insert("mode".to_string(), Value::String(mode.as_str().to_string()));
    root.insert(
        "primary_provider_id".to_string(),
        Value::String(primary_provider_id.to_string()),
    );
    serde_json::from_value::<CliProxyManifest>(value)
        .map(Some)
        .map_err(|error| {
            format!(
                "Failed to parse legacy gateway manifest {}: {}",
                manifest_path.display(),
                error
            )
        })
}

fn write_manifest(
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    manifest: &CliProxyManifest,
) -> Result<(), String> {
    let manifest_path = paths.manifest_path(cli_key);
    if let Some(parent) = manifest_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create gateway manifest directory {}: {}",
                parent.display(),
                error
            )
        })?;
    }
    let content = serde_json::to_string_pretty(manifest)
        .map_err(|error| format!("Failed to serialize gateway manifest: {error}"))?;
    fs::write(&manifest_path, format!("{content}\n")).map_err(|error| {
        format!(
            "Failed to write gateway manifest {}: {}",
            manifest_path.display(),
            error
        )
    })
}

fn prepare_manifest(
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    base_origin: &str,
    targets: &CliProxyTargets,
    mode: GatewayProxyMode,
    primary_provider_id: &str,
) -> Result<CliProxyManifest, String> {
    let timestamp = chrono::Utc::now().to_rfc3339();
    let previous_manifest = read_manifest_for_reengage(paths, cli_key, mode, primary_provider_id)?;
    if let Some(previous_manifest) = previous_manifest
        .as_ref()
        .filter(|manifest| manifest.enabled)
    {
        if previous_manifest.primary_provider_id != primary_provider_id {
            return Err(
                "Restore direct mode before switching the primary Gateway proxy provider"
                    .to_string(),
            );
        }
    }
    let mut manifest = previous_manifest
        .filter(|manifest| manifest.enabled)
        .unwrap_or_else(|| {
            CliProxyManifest::new(
                cli_key,
                base_origin.to_string(),
                timestamp.clone(),
                mode,
                primary_provider_id.to_string(),
            )
        });
    manifest.enabled = true;
    manifest.mode = mode;
    manifest.primary_provider_id = primary_provider_id.to_string();
    manifest.base_origin = base_origin.to_string();
    manifest.updated_at = timestamp;

    let backup_dir = paths.backup_dir(cli_key);
    let mut files = Vec::new();
    for target in &targets.files {
        let target_path = path_to_string(&target.path);
        let existing_file = manifest
            .files
            .iter()
            .find(|file| file.kind == target.kind && file.path == target_path)
            .cloned();
        let file = match existing_file {
            Some(mut file) => {
                file.managed_fields = target
                    .managed_fields
                    .iter()
                    .map(|field| (*field).to_string())
                    .collect();
                file
            }
            None => backup_target_file(target, &backup_dir)?,
        };
        files.push(file);
    }
    manifest.files = files;
    Ok(manifest)
}

fn backup_target_file(
    target: &CliProxyTarget,
    backup_dir: &Path,
) -> Result<CliProxyManifestFile, String> {
    let backup_rel_path = format!("{}.bak", target.kind);
    validate_backup_rel_path(&backup_rel_path)?;
    let backup_path = backup_dir.join(&backup_rel_path);
    let existed = target.path.exists();
    let mut backup_sha256 = None;
    let mut backup_size = None;

    if existed {
        if let Some(parent) = backup_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create gateway backup directory {}: {}",
                    parent.display(),
                    error
                )
            })?;
        }
        let content = fs::read(&target.path).map_err(|error| {
            format!(
                "Failed to read CLI config before gateway takeover {}: {}",
                target.path.display(),
                error
            )
        })?;
        fs::write(&backup_path, &content).map_err(|error| {
            format!(
                "Failed to write gateway backup {}: {}",
                backup_path.display(),
                error
            )
        })?;
        backup_size = Some(content.len() as u64);
        backup_sha256 = Some(sha256_hex(&content));
    }

    Ok(CliProxyManifestFile {
        kind: target.kind.to_string(),
        path: path_to_string(&target.path),
        existed,
        backup_rel_path,
        backup_sha256,
        backup_size,
        managed_fields: target
            .managed_fields
            .iter()
            .map(|field| (*field).to_string())
            .collect(),
    })
}

fn manifest_restore_available(
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    manifest: &CliProxyManifest,
) -> bool {
    manifest.files.iter().all(|file| {
        validate_backup_rel_path(&file.backup_rel_path).is_ok()
            && (!file.existed
                || paths
                    .backup_dir(cli_key)
                    .join(&file.backup_rel_path)
                    .exists())
    })
}

fn backup_content(
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    manifest: &CliProxyManifest,
    kind: &str,
) -> Result<Option<String>, String> {
    let Some(file) = manifest.files.iter().find(|file| file.kind == kind) else {
        return Ok(None);
    };
    if !file.existed {
        return Ok(None);
    }
    validate_backup_rel_path(&file.backup_rel_path)?;
    let backup_path = paths.backup_dir(cli_key).join(&file.backup_rel_path);
    fs::read_to_string(&backup_path).map(Some).map_err(|error| {
        format!(
            "Failed to read gateway backup {}: {}",
            backup_path.display(),
            error
        )
    })
}

fn codex_auth_preservation_enabled_for_cli(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
) -> Result<bool, String> {
    if cli_key != GatewayCliKey::Codex {
        return Ok(false);
    }
    Ok(crate::settings::store::load_settings_from_sqlite_state(db)?
        .codex_preserve_official_auth_on_switch)
}

fn codex_auth_backup_content_for_cli(
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    manifest: &CliProxyManifest,
) -> Result<Option<String>, String> {
    if cli_key != GatewayCliKey::Codex {
        return Ok(None);
    }
    backup_content(paths, cli_key, manifest, CODEX_AUTH_KIND)
}

fn apply_gateway_config(
    cli_key: GatewayCliKey,
    targets: &CliProxyTargets,
    base_origin: &str,
    primary_provider: Option<&UpstreamProvider>,
    mode: GatewayProxyMode,
    claude_backup_content: Option<&str>,
    codex_auth_backup_content: Option<&str>,
    preserve_codex_official_auth: bool,
) -> Result<(), String> {
    match cli_key {
        GatewayCliKey::Claude => {
            let Some(primary_provider) = primary_provider else {
                return Err("Claude Gateway proxy requires a primary provider".to_string());
            };
            patch_claude_settings(
                required_target_path(targets, CLAUDE_SETTINGS_KIND)?,
                &cli_gateway_endpoint(cli_key, base_origin),
                primary_provider,
                mode == GatewayProxyMode::Failover,
                claude_backup_content,
            )
        }
        GatewayCliKey::Codex => {
            patch_codex_config(
                required_target_path(targets, CODEX_CONFIG_KIND)?,
                &cli_gateway_endpoint(cli_key, base_origin),
                preserve_codex_official_auth,
            )?;
            patch_codex_auth(
                required_target_path(targets, CODEX_AUTH_KIND)?,
                preserve_codex_official_auth,
                codex_auth_backup_content,
            )
        }
        GatewayCliKey::Gemini => {
            patch_gemini_env(
                required_target_path(targets, GEMINI_ENV_KIND)?,
                &cli_gateway_endpoint(cli_key, base_origin),
            )?;
            patch_gemini_settings(required_target_path(targets, GEMINI_SETTINGS_KIND)?)
        }
        GatewayCliKey::OpenCode => {
            Err("OpenCode adapter is intentionally out of scope".to_string())
        }
    }
}

fn restore_gateway_config(
    cli_key: GatewayCliKey,
    paths: &ProxyGatewayPaths,
    targets: &CliProxyTargets,
    manifest: &CliProxyManifest,
) -> Result<(), String> {
    match cli_key {
        GatewayCliKey::Claude => restore_claude_settings(
            required_target_path(targets, CLAUDE_SETTINGS_KIND)?,
            backup_content(paths, cli_key, manifest, CLAUDE_SETTINGS_KIND)?.as_deref(),
        ),
        GatewayCliKey::Codex => {
            restore_codex_config(
                required_target_path(targets, CODEX_CONFIG_KIND)?,
                backup_content(paths, cli_key, manifest, CODEX_CONFIG_KIND)?.as_deref(),
            )?;
            restore_codex_auth(
                required_target_path(targets, CODEX_AUTH_KIND)?,
                backup_content(paths, cli_key, manifest, CODEX_AUTH_KIND)?.as_deref(),
            )
        }
        GatewayCliKey::Gemini => {
            restore_gemini_env(
                required_target_path(targets, GEMINI_ENV_KIND)?,
                backup_content(paths, cli_key, manifest, GEMINI_ENV_KIND)?.as_deref(),
            )?;
            restore_gemini_settings(
                required_target_path(targets, GEMINI_SETTINGS_KIND)?,
                backup_content(paths, cli_key, manifest, GEMINI_SETTINGS_KIND)?.as_deref(),
            )
        }
        GatewayCliKey::OpenCode => {
            Err("OpenCode adapter is intentionally out of scope".to_string())
        }
    }
}

fn required_target_path<'a>(targets: &'a CliProxyTargets, kind: &str) -> Result<&'a Path, String> {
    targets
        .files
        .iter()
        .find(|target| target.kind == kind)
        .map(|target| target.path.as_path())
        .ok_or_else(|| format!("Missing gateway CLI target: {kind}"))
}

fn current_cli_gateway_endpoint(
    cli_key: GatewayCliKey,
    targets: &CliProxyTargets,
) -> Result<Option<String>, String> {
    match cli_key {
        GatewayCliKey::Claude => {
            current_claude_gateway_endpoint(required_target_path(targets, CLAUDE_SETTINGS_KIND)?)
        }
        GatewayCliKey::Codex => {
            current_codex_gateway_endpoint(required_target_path(targets, CODEX_CONFIG_KIND)?)
        }
        GatewayCliKey::Gemini => {
            current_gemini_gateway_endpoint(required_target_path(targets, GEMINI_ENV_KIND)?)
        }
        GatewayCliKey::OpenCode => Ok(None),
    }
}

fn cli_gateway_endpoint(cli_key: GatewayCliKey, base_origin: &str) -> String {
    let base_origin = base_origin.trim_end_matches('/');
    match cli_key {
        GatewayCliKey::Claude => format!("{base_origin}/anthropic"),
        GatewayCliKey::Codex => format!("{base_origin}/openai/v1"),
        GatewayCliKey::Gemini => format!("{base_origin}/gemini/v1beta"),
        GatewayCliKey::OpenCode => base_origin.to_string(),
    }
}

fn current_claude_gateway_endpoint(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let value = read_json_file(path)?;
    Ok(value
        .pointer("/env/ANTHROPIC_BASE_URL")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn current_codex_gateway_endpoint(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let document = parse_toml_file(path)?;
    let selected_provider = document
        .as_table()
        .get("model_provider")
        .and_then(Item::as_str);
    if selected_provider != Some(GATEWAY_PROVIDER_ID) {
        return Ok(None);
    }
    Ok(document
        .as_table()
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(GATEWAY_PROVIDER_ID))
        .and_then(Item::as_table)
        .and_then(|provider| provider.get("base_url"))
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn current_gemini_gateway_endpoint(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path).map_err(|error| {
        format!(
            "Failed to read Gemini CLI .env {}: {}",
            path.display(),
            error
        )
    })?;
    Ok(parse_env_content(&content)
        .remove("GOOGLE_GEMINI_BASE_URL")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

fn rewrite_claude_wsl_gateway_content(
    content: &str,
    windows_gateway_endpoint: &str,
    wsl_gateway_endpoint: &str,
) -> Result<Option<String>, String> {
    let mut value = serde_json::from_str::<Value>(content)
        .map_err(|error| format!("Failed to parse WSL Claude settings JSON: {error}"))?;
    let Some(env) = value.get_mut("env").and_then(Value::as_object_mut) else {
        return Ok(None);
    };

    let base_url_matches = env
        .get("ANTHROPIC_BASE_URL")
        .and_then(Value::as_str)
        .map(str::trim)
        == Some(windows_gateway_endpoint);
    let gateway_token_matches = env
        .get("ANTHROPIC_AUTH_TOKEN")
        .or_else(|| env.get("ANTHROPIC_API_KEY"))
        .and_then(Value::as_str)
        .map(str::trim)
        == Some(GATEWAY_API_KEY);

    if !base_url_matches || !gateway_token_matches {
        return Ok(None);
    }

    env.insert(
        "ANTHROPIC_BASE_URL".to_string(),
        Value::String(wsl_gateway_endpoint.to_string()),
    );
    let content = write_json_value_to_string(&value)?;
    Ok(Some(format!("{content}\n")))
}

fn rewrite_codex_wsl_gateway_content(
    content: &str,
    windows_gateway_endpoint: &str,
    wsl_gateway_endpoint: &str,
) -> Result<Option<String>, String> {
    let mut document = parse_toml_document(content, "WSL Codex config")?;
    let selected_provider = document
        .as_table()
        .get("model_provider")
        .and_then(Item::as_str);
    if selected_provider != Some(GATEWAY_PROVIDER_ID) {
        return Ok(None);
    }

    let Some(provider_table) = document
        .as_table_mut()
        .get_mut("model_providers")
        .and_then(Item::as_table_mut)
        .and_then(|providers| providers.get_mut(GATEWAY_PROVIDER_ID))
        .and_then(Item::as_table_mut)
    else {
        return Ok(None);
    };

    let base_url_matches = provider_table
        .get("base_url")
        .and_then(Item::as_str)
        .map(str::trim)
        == Some(windows_gateway_endpoint);
    if !base_url_matches {
        return Ok(None);
    }

    provider_table["base_url"] = value(wsl_gateway_endpoint);
    Ok(Some(document.to_string()))
}

fn rewrite_gemini_wsl_gateway_content(
    content: &str,
    windows_gateway_endpoint: &str,
    wsl_gateway_endpoint: &str,
) -> Result<Option<String>, String> {
    let env = parse_env_content(content);
    let base_url_matches = env.get("GOOGLE_GEMINI_BASE_URL").map(|value| value.trim())
        == Some(windows_gateway_endpoint);
    let gateway_key_matches =
        env.get("GEMINI_API_KEY").map(|value| value.trim()) == Some(GATEWAY_API_KEY);

    if !base_url_matches || !gateway_key_matches {
        return Ok(None);
    }

    let rewritten = merge_env_content(
        content,
        &BTreeMap::from([(
            "GOOGLE_GEMINI_BASE_URL".to_string(),
            wsl_gateway_endpoint.to_string(),
        )]),
    );
    Ok(Some(rewritten))
}

fn patch_claude_settings(
    path: &Path,
    gateway_endpoint: &str,
    primary_provider: &UpstreamProvider,
    write_model_fields: bool,
    backup_content: Option<&str>,
) -> Result<(), String> {
    let mut value = if path.exists() {
        read_json_file(path)?
    } else {
        Value::Object(Map::new())
    };
    let root = ensure_json_object(&mut value);
    let env = root
        .entry("env")
        .or_insert_with(|| Value::Object(Map::new()));
    let env = ensure_json_object(env);
    env.remove("ANTHROPIC_API_KEY");
    env.insert(
        "ANTHROPIC_BASE_URL".to_string(),
        Value::String(gateway_endpoint.to_string()),
    );
    env.insert(
        "ANTHROPIC_AUTH_TOKEN".to_string(),
        Value::String(GATEWAY_API_KEY.to_string()),
    );

    if write_model_fields {
        env.insert(
            "ANTHROPIC_MODEL".to_string(),
            Value::String(CLAUDE_STANDARD_MODEL.to_string()),
        );
        env.insert(
            "ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(),
            Value::String(CLAUDE_STANDARD_HAIKU_MODEL.to_string()),
        );
        env.insert(
            "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
            Value::String(CLAUDE_STANDARD_SONNET_MODEL.to_string()),
        );
        env.insert(
            "ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(),
            Value::String(CLAUDE_STANDARD_OPUS_MODEL.to_string()),
        );
        env.remove("ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME");
        env.remove("ANTHROPIC_DEFAULT_SONNET_MODEL_NAME");
        env.remove("ANTHROPIC_DEFAULT_OPUS_MODEL_NAME");
        if let Some(model_name) = provider_model_name(
            primary_provider.model_mapping.haiku_model.as_deref(),
            primary_provider.model_mapping.default_model.as_deref(),
        ) {
            env.insert(
                "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME".to_string(),
                Value::String(model_name),
            );
        }
        if let Some(model_name) = provider_model_name(
            primary_provider.model_mapping.sonnet_model.as_deref(),
            primary_provider.model_mapping.default_model.as_deref(),
        ) {
            env.insert(
                "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME".to_string(),
                Value::String(model_name),
            );
        }
        if let Some(model_name) = provider_model_name(
            primary_provider.model_mapping.opus_model.as_deref(),
            primary_provider.model_mapping.default_model.as_deref(),
        ) {
            env.insert(
                "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME".to_string(),
                Value::String(model_name),
            );
        }
    } else if let Some(backup_content) = backup_content {
        let backup = serde_json::from_str::<Value>(backup_content)
            .map_err(|error| format!("Failed to parse Claude gateway backup: {error}"))?;
        restore_json_pointer_fields(&mut value, Some(&backup), &CLAUDE_MODEL_FIELD_POINTERS);
    }
    write_json_file(path, &value)
}

fn provider_model_name(family_model: Option<&str>, default_model: Option<&str>) -> Option<String> {
    family_model
        .or(default_model)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn restore_claude_settings(path: &Path, backup_content: Option<&str>) -> Result<(), String> {
    let mut current = if path.exists() {
        read_json_file(path)?
    } else {
        Value::Object(Map::new())
    };
    let backup = backup_content
        .map(|content| serde_json::from_str::<Value>(content))
        .transpose()
        .map_err(|error| format!("Failed to parse Claude gateway backup: {error}"))?;
    restore_json_pointer_fields(
        &mut current,
        backup.as_ref(),
        &[
            "/env/ANTHROPIC_BASE_URL",
            "/env/ANTHROPIC_AUTH_TOKEN",
            "/env/ANTHROPIC_API_KEY",
            "/env/ANTHROPIC_MODEL",
            "/env/ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "/env/ANTHROPIC_DEFAULT_SONNET_MODEL",
            "/env/ANTHROPIC_DEFAULT_OPUS_MODEL",
            "/env/ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
            "/env/ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
            "/env/ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
        ],
    );
    // Older gateway takeovers wrote reasoning model as a managed field.
    restore_json_pointer_fields(
        &mut current,
        backup.as_ref(),
        &[CLAUDE_LEGACY_REASONING_MODEL_POINTER],
    );
    write_json_file(path, &current)
}

fn patch_codex_config(
    path: &Path,
    gateway_endpoint: &str,
    preserve_official_auth: bool,
) -> Result<(), String> {
    let mut document = read_or_new_toml_document(path)?;
    document["model_provider"] = value(GATEWAY_PROVIDER_ID);
    document["model_providers"][GATEWAY_PROVIDER_ID]["name"] = value("AI Toolbox Gateway");
    document["model_providers"][GATEWAY_PROVIDER_ID]["base_url"] = value(gateway_endpoint);
    document["model_providers"][GATEWAY_PROVIDER_ID]["wire_api"] = value("responses");
    document["model_providers"][GATEWAY_PROVIDER_ID]["requires_openai_auth"] = value(true);
    if preserve_official_auth {
        document["model_providers"][GATEWAY_PROVIDER_ID]["experimental_bearer_token"] =
            value(GATEWAY_API_KEY);
    } else if let Some(provider_table) =
        document["model_providers"][GATEWAY_PROVIDER_ID].as_table_like_mut()
    {
        provider_table.remove("experimental_bearer_token");
    }
    write_toml_file(path, &document)
}

fn restore_codex_config(path: &Path, backup_content: Option<&str>) -> Result<(), String> {
    let mut current = read_or_new_toml_document(path)?;
    let backup = backup_content
        .map(|content| parse_toml_document(content, "Codex gateway backup"))
        .transpose()?;

    if current
        .as_table()
        .get("model_provider")
        .and_then(Item::as_str)
        == Some(GATEWAY_PROVIDER_ID)
    {
        current.as_table_mut().remove("model_provider");
    }

    if let Some(providers_table) = current
        .as_table_mut()
        .get_mut("model_providers")
        .and_then(Item::as_table_mut)
    {
        providers_table.remove(GATEWAY_PROVIDER_ID);
    }

    if let Some(backup_document) = backup.as_ref() {
        if let Some(model_provider) = backup_document.as_table().get("model_provider").cloned() {
            current
                .as_table_mut()
                .insert("model_provider", model_provider);
        }
        if let Some(backup_providers) = backup_document
            .as_table()
            .get("model_providers")
            .and_then(Item::as_table)
        {
            for (provider_key, provider_item) in backup_providers.iter() {
                current["model_providers"][provider_key] = provider_item.clone();
            }
        }
    }

    remove_empty_toml_table(&mut current, "model_providers");
    write_toml_file(path, &current)
}

fn patch_codex_auth(
    path: &Path,
    preserve_official_auth: bool,
    backup_content: Option<&str>,
) -> Result<(), String> {
    if preserve_official_auth {
        return restore_codex_gateway_auth_fields(path, backup_content);
    }

    let mut value = if path.exists() {
        read_json_file(path)?
    } else {
        Value::Object(Map::new())
    };
    let root = ensure_json_object(&mut value);
    root.insert(
        "OPENAI_API_KEY".to_string(),
        Value::String(GATEWAY_API_KEY.to_string()),
    );
    root.insert("auth_mode".to_string(), Value::String("apikey".to_string()));
    write_json_file(path, &value)
}

fn restore_codex_gateway_auth_fields(
    path: &Path,
    backup_content: Option<&str>,
) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let current = read_json_file(path)?;
    if current.get("OPENAI_API_KEY").and_then(Value::as_str) != Some(GATEWAY_API_KEY) {
        return Ok(());
    }

    restore_codex_auth(path, backup_content)
}

fn restore_codex_auth(path: &Path, backup_content: Option<&str>) -> Result<(), String> {
    let mut current = if path.exists() {
        read_json_file(path)?
    } else {
        Value::Object(Map::new())
    };
    let backup = backup_content
        .map(|content| serde_json::from_str::<Value>(content))
        .transpose()
        .map_err(|error| format!("Failed to parse Codex gateway auth backup: {error}"))?;
    restore_json_pointer_fields(
        &mut current,
        backup.as_ref(),
        &["/OPENAI_API_KEY", "/auth_mode"],
    );
    write_json_file(path, &current)
}

fn patch_gemini_env(path: &Path, gateway_endpoint: &str) -> Result<(), String> {
    let existing_content = if path.exists() {
        fs::read_to_string(path).map_err(|error| {
            format!(
                "Failed to read Gemini CLI .env {}: {}",
                path.display(),
                error
            )
        })?
    } else {
        String::new()
    };
    let provider_env = BTreeMap::from([
        ("GEMINI_API_KEY".to_string(), GATEWAY_API_KEY.to_string()),
        (
            "GOOGLE_GEMINI_BASE_URL".to_string(),
            gateway_endpoint.to_string(),
        ),
    ]);
    write_text_file(path, &merge_env_content(&existing_content, &provider_env))
}

fn restore_gemini_env(path: &Path, backup_content: Option<&str>) -> Result<(), String> {
    let current = if path.exists() {
        fs::read_to_string(path).map_err(|error| {
            format!(
                "Failed to read Gemini CLI .env {}: {}",
                path.display(),
                error
            )
        })?
    } else {
        String::new()
    };
    let backup_env = backup_content.map(parse_env_content).unwrap_or_default();
    write_text_file(path, &restore_env_content(&current, &backup_env))
}

fn patch_gemini_settings(path: &Path) -> Result<(), String> {
    let mut value = if path.exists() {
        read_json_file(path)?
    } else {
        Value::Object(Map::new())
    };
    set_json_path_string(
        &mut value,
        &["security", "auth", "selectedType"],
        "gemini-api-key",
    );
    write_json_file(path, &value)
}

fn restore_gemini_settings(path: &Path, backup_content: Option<&str>) -> Result<(), String> {
    let mut current = if path.exists() {
        read_json_file(path)?
    } else {
        Value::Object(Map::new())
    };
    let backup = backup_content
        .map(|content| serde_json::from_str::<Value>(content))
        .transpose()
        .map_err(|error| format!("Failed to parse Gemini gateway settings backup: {error}"))?;
    restore_json_pointer_fields(
        &mut current,
        backup.as_ref(),
        &["/security/auth/selectedType"],
    );
    write_json_file(path, &current)
}

fn read_json_file(path: &Path) -> Result<Value, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read JSON file {}: {}", path.display(), error))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("Failed to parse JSON file {}: {}", path.display(), error))
}

fn write_json_file(path: &Path, value: &Value) -> Result<(), String> {
    let content = write_json_value_to_string(value)?;
    write_text_file(path, &format!("{content}\n"))
}

fn write_json_value_to_string(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map_err(|error| format!("Failed to serialize JSON value: {}", error))
}

fn write_text_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!("Failed to create directory {}: {}", parent.display(), error)
        })?;
    }
    fs::write(path, content)
        .map_err(|error| format!("Failed to write {}: {}", path.display(), error))
}

fn ensure_json_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value
        .as_object_mut()
        .expect("value was normalized to object")
}

fn set_json_path_string(value: &mut Value, path: &[&str], next_value: &str) {
    let mut current = value;
    for key in &path[..path.len().saturating_sub(1)] {
        let object = ensure_json_object(current);
        current = object
            .entry((*key).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    if let Some(last_key) = path.last() {
        ensure_json_object(current).insert(
            (*last_key).to_string(),
            Value::String(next_value.to_string()),
        );
    }
}

fn restore_json_pointer_fields(current: &mut Value, backup: Option<&Value>, pointers: &[&str]) {
    for pointer in pointers {
        match backup.and_then(|value| value.pointer(pointer)).cloned() {
            Some(value) => set_json_pointer(current, pointer, value),
            None => remove_json_pointer(current, pointer),
        }
    }
}

fn set_json_pointer(current: &mut Value, pointer: &str, next_value: Value) {
    let parts: Vec<&str> = pointer.trim_start_matches('/').split('/').collect();
    let mut value = current;
    for part in &parts[..parts.len().saturating_sub(1)] {
        let object = ensure_json_object(value);
        value = object
            .entry((*part).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    if let Some(last_part) = parts.last() {
        ensure_json_object(value).insert((*last_part).to_string(), next_value);
    }
}

fn remove_json_pointer(current: &mut Value, pointer: &str) {
    let parts: Vec<&str> = pointer.trim_start_matches('/').split('/').collect();
    if parts.is_empty() {
        return;
    }
    let mut value = current;
    for part in &parts[..parts.len().saturating_sub(1)] {
        let Some(next_value) = value
            .as_object_mut()
            .and_then(|object| object.get_mut(*part))
        else {
            return;
        };
        value = next_value;
    }
    if let Some(last_part) = parts.last() {
        if let Some(object) = value.as_object_mut() {
            object.remove(*last_part);
        }
    }
}

fn parse_toml_file(path: &Path) -> Result<DocumentMut, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read TOML file {}: {}", path.display(), error))?;
    parse_toml_document(&content, &path.display().to_string())
}

fn read_or_new_toml_document(path: &Path) -> Result<DocumentMut, String> {
    if path.exists() {
        parse_toml_file(path)
    } else {
        Ok(DocumentMut::new())
    }
}

fn parse_toml_document(content: &str, label: &str) -> Result<DocumentMut, String> {
    if content.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    content
        .parse::<DocumentMut>()
        .map_err(|error| format!("Failed to parse {label}: {error}"))
}

fn write_toml_file(path: &Path, document: &DocumentMut) -> Result<(), String> {
    let content = render_toml_document(document);
    write_text_file(path, &content)
}

fn render_toml_document(document: &DocumentMut) -> String {
    let content = document.to_string();
    let with_schema = if content.trim_start().starts_with("#:schema") {
        content
    } else {
        format!("#:schema none\n{content}")
    };
    if with_schema.ends_with('\n') {
        with_schema
    } else {
        format!("{with_schema}\n")
    }
}

fn remove_empty_toml_table(document: &mut DocumentMut, key: &str) {
    let should_remove = document
        .as_table()
        .get(key)
        .and_then(Item::as_table)
        .map(|table| table.is_empty())
        .unwrap_or(false);
    if should_remove {
        document.as_table_mut().remove(key);
    }
}

fn parse_env_line_key(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let candidate = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    let (key, _) = candidate.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        None
    } else {
        Some(key.to_string())
    }
}

fn parse_env_content(content: &str) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let candidate = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        let Some((key, raw_value)) = candidate.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = raw_value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        result.insert(key.to_string(), value);
    }
    result
}

fn serialize_env_value(value: &str) -> String {
    if value
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '\'' | '#' | '='))
    {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

fn merge_env_content(existing_content: &str, provider_env: &BTreeMap<String, String>) -> String {
    let managed_keys: BTreeSet<&str> = GEMINI_MANAGED_ENV_KEYS.into_iter().collect();
    let mut lines: Vec<String> = existing_content
        .lines()
        .filter(|line| {
            parse_env_line_key(line)
                .map(|key| !managed_keys.contains(key.as_str()))
                .unwrap_or(true)
        })
        .map(str::to_string)
        .collect();

    if !lines.is_empty()
        && !lines
            .last()
            .map(|line| line.trim().is_empty())
            .unwrap_or(false)
    {
        lines.push(String::new());
    }

    for (key, value) in provider_env {
        if managed_keys.contains(key.as_str()) && !value.trim().is_empty() {
            lines.push(format!("{}={}", key, serialize_env_value(value.trim())));
        }
    }

    while lines
        .last()
        .map(|line| line.trim().is_empty())
        .unwrap_or(false)
    {
        lines.pop();
    }

    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn restore_env_content(current_content: &str, backup_env: &BTreeMap<String, String>) -> String {
    let backup_managed_env: BTreeMap<String, String> = backup_env
        .iter()
        .filter(|(key, _)| GEMINI_MANAGED_ENV_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    merge_env_content(current_content, &backup_managed_env)
}

fn sha256_hex(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claude_test_provider(
        default_model: Option<&str>,
        haiku_model: Option<&str>,
        sonnet_model: Option<&str>,
        opus_model: Option<&str>,
    ) -> UpstreamProvider {
        UpstreamProvider {
            cli_key: GatewayCliKey::Claude,
            id: "provider-1".to_string(),
            name: "Provider 1".to_string(),
            base_url: "https://api.example.com".to_string(),
            api_key: "key".to_string(),
            target_protocol:
                crate::coding::proxy_gateway::transformer::AiProtocol::AnthropicMessages,
            auth_strategy:
                crate::coding::proxy_gateway::runtime::ProviderAuthStrategy::AnthropicApiKey,
            is_full_url: false,
            sort_index: Some(0),
            meta: super::super::types::ProviderGatewayMeta::default(),
            model_mapping: super::super::runtime::UpstreamModelMapping {
                default_model: default_model.map(str::to_string),
                haiku_model: haiku_model.map(str::to_string),
                sonnet_model: sonnet_model.map(str::to_string),
                opus_model: opus_model.map(str::to_string),
                reasoning_model: None,
            },
        }
    }

    fn test_proxy_gateway_settings(wsl_host: &str) -> ProxyGatewaySettings {
        let mut settings = ProxyGatewaySettings::default();
        settings.wsl_host = wsl_host.to_string();
        settings
    }

    fn test_manifest_with_file(
        cli_key: GatewayCliKey,
        file_kind: &str,
        managed_fields: &[&str],
    ) -> CliProxyManifest {
        let mut manifest = CliProxyManifest::new(
            cli_key,
            "http://127.0.0.1:37123".to_string(),
            "2026-05-17T00:00:00Z".to_string(),
            GatewayProxyMode::Single,
            "provider-1".to_string(),
        );
        manifest.files.push(CliProxyManifestFile {
            kind: file_kind.to_string(),
            path: "C:\\Users\\User\\runtime-config".to_string(),
            existed: true,
            backup_rel_path: "backups/runtime-config".to_string(),
            backup_sha256: None,
            backup_size: None,
            managed_fields: managed_fields
                .iter()
                .map(|field| field.to_string())
                .collect(),
        });
        manifest
    }

    #[test]
    fn claude_takeover_and_restore_only_manage_gateway_env_keys() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        write_json_file(
            &settings_path,
            &json!({
                "env": {
                    "CLAUDE_CODE_ENABLE_TELEMETRY": false,
                    "ANTHROPIC_BASE_URL": "https://old.example.com",
                    "ANTHROPIC_AUTH_TOKEN": "old-token"
                },
                "hooks": {"keep": true}
            }),
        )
        .unwrap();
        let backup = fs::read_to_string(&settings_path).unwrap();
        let primary_provider = claude_test_provider(
            Some("provider-default"),
            Some("provider-haiku"),
            Some("provider-sonnet"),
            Some("provider-opus"),
        );

        patch_claude_settings(
            &settings_path,
            "http://127.0.0.1:37123/anthropic",
            &primary_provider,
            true,
            None,
        )
        .unwrap();
        let patched = read_json_file(&settings_path).unwrap();
        assert_eq!(
            patched
                .pointer("/env/ANTHROPIC_BASE_URL")
                .and_then(Value::as_str),
            Some("http://127.0.0.1:37123/anthropic")
        );
        assert_eq!(
            patched
                .pointer("/env/ANTHROPIC_MODEL")
                .and_then(Value::as_str),
            Some(CLAUDE_STANDARD_MODEL)
        );
        assert_eq!(
            patched
                .pointer("/env/ANTHROPIC_DEFAULT_SONNET_MODEL")
                .and_then(Value::as_str),
            Some(CLAUDE_STANDARD_SONNET_MODEL)
        );
        assert_eq!(patched.pointer("/env/ANTHROPIC_REASONING_MODEL"), None);
        assert_eq!(
            patched
                .pointer("/env/ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME")
                .and_then(Value::as_str),
            Some("provider-haiku")
        );
        assert_eq!(
            patched
                .pointer("/env/ANTHROPIC_DEFAULT_SONNET_MODEL_NAME")
                .and_then(Value::as_str),
            Some("provider-sonnet")
        );
        assert_eq!(
            patched
                .pointer("/env/ANTHROPIC_DEFAULT_OPUS_MODEL_NAME")
                .and_then(Value::as_str),
            Some("provider-opus")
        );
        assert_eq!(
            patched
                .pointer("/env/CLAUDE_CODE_ENABLE_TELEMETRY")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            patched.pointer("/hooks/keep").and_then(Value::as_bool),
            Some(true)
        );

        restore_claude_settings(&settings_path, Some(&backup)).unwrap();
        let restored = read_json_file(&settings_path).unwrap();
        assert_eq!(
            restored
                .pointer("/env/ANTHROPIC_BASE_URL")
                .and_then(Value::as_str),
            Some("https://old.example.com")
        );
        assert!(restored.pointer("/env/ANTHROPIC_MODEL").is_none());
        assert!(restored.pointer("/env/ANTHROPIC_REASONING_MODEL").is_none());
        assert!(restored
            .pointer("/env/ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME")
            .is_none());
        assert!(restored
            .pointer("/env/ANTHROPIC_DEFAULT_SONNET_MODEL_NAME")
            .is_none());
        assert!(restored
            .pointer("/env/ANTHROPIC_DEFAULT_OPUS_MODEL_NAME")
            .is_none());
        assert_eq!(
            restored
                .pointer("/env/CLAUDE_CODE_ENABLE_TELEMETRY")
                .and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn claude_model_name_fields_fall_back_to_default_model() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        let primary_provider = claude_test_provider(Some("provider-default"), None, None, None);

        patch_claude_settings(
            &settings_path,
            "http://127.0.0.1:37123/anthropic",
            &primary_provider,
            true,
            None,
        )
        .unwrap();

        let patched = read_json_file(&settings_path).unwrap();
        assert_eq!(
            patched
                .pointer("/env/ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME")
                .and_then(Value::as_str),
            Some("provider-default")
        );
        assert_eq!(
            patched
                .pointer("/env/ANTHROPIC_DEFAULT_SONNET_MODEL_NAME")
                .and_then(Value::as_str),
            Some("provider-default")
        );
        assert_eq!(
            patched
                .pointer("/env/ANTHROPIC_DEFAULT_OPUS_MODEL_NAME")
                .and_then(Value::as_str),
            Some("provider-default")
        );
    }

    #[test]
    fn claude_model_name_fields_are_omitted_without_provider_models() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        write_json_file(
            &settings_path,
            &json!({
                "env": {
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME": "stale-haiku",
                    "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "stale-sonnet",
                    "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME": "stale-opus"
                }
            }),
        )
        .unwrap();
        let primary_provider = claude_test_provider(None, None, None, None);

        patch_claude_settings(
            &settings_path,
            "http://127.0.0.1:37123/anthropic",
            &primary_provider,
            true,
            None,
        )
        .unwrap();

        let patched = read_json_file(&settings_path).unwrap();
        assert!(patched
            .pointer("/env/ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME")
            .is_none());
        assert!(patched
            .pointer("/env/ANTHROPIC_DEFAULT_SONNET_MODEL_NAME")
            .is_none());
        assert!(patched
            .pointer("/env/ANTHROPIC_DEFAULT_OPUS_MODEL_NAME")
            .is_none());
    }

    #[test]
    fn claude_single_restore_without_backup_removes_failover_model_fields() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        write_json_file(
            &settings_path,
            &json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:37123/anthropic",
                    "ANTHROPIC_AUTH_TOKEN": GATEWAY_API_KEY,
                    "ANTHROPIC_MODEL": CLAUDE_STANDARD_MODEL,
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL": CLAUDE_STANDARD_HAIKU_MODEL,
                    "ANTHROPIC_DEFAULT_SONNET_MODEL": CLAUDE_STANDARD_SONNET_MODEL,
                    "ANTHROPIC_DEFAULT_OPUS_MODEL": CLAUDE_STANDARD_OPUS_MODEL,
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME": "provider-haiku",
                    "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "provider-sonnet",
                    "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME": "provider-opus"
                }
            }),
        )
        .unwrap();
        let primary_provider = claude_test_provider(Some("provider-default"), None, None, None);

        patch_claude_settings(
            &settings_path,
            "http://127.0.0.1:37123/anthropic",
            &primary_provider,
            false,
            Some("{}"),
        )
        .unwrap();

        let patched = read_json_file(&settings_path).unwrap();
        assert_eq!(
            patched
                .pointer("/env/ANTHROPIC_BASE_URL")
                .and_then(Value::as_str),
            Some("http://127.0.0.1:37123/anthropic")
        );
        for pointer in CLAUDE_MODEL_FIELD_POINTERS {
            assert!(
                patched.pointer(pointer).is_none(),
                "{pointer} should be removed"
            );
        }
    }

    #[test]
    fn restore_claude_settings_handles_legacy_reasoning_model_field() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        write_json_file(
            &settings_path,
            &json!({
                "env": {
                    "ANTHROPIC_REASONING_MODEL": CLAUDE_STANDARD_MODEL
                }
            }),
        )
        .unwrap();

        restore_claude_settings(
            &settings_path,
            Some(
                r#"{
                    "env": {
                        "ANTHROPIC_REASONING_MODEL": "user-reasoning-model"
                    }
                }"#,
            ),
        )
        .unwrap();
        let restored = read_json_file(&settings_path).unwrap();
        assert_eq!(
            restored
                .pointer("/env/ANTHROPIC_REASONING_MODEL")
                .and_then(Value::as_str),
            Some("user-reasoning-model")
        );

        restore_claude_settings(&settings_path, Some("{}")).unwrap();
        let restored = read_json_file(&settings_path).unwrap();
        assert!(restored.pointer("/env/ANTHROPIC_REASONING_MODEL").is_none());
    }

    #[test]
    fn codex_takeover_keeps_runtime_sections_and_restore_removes_gateway_provider() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        write_text_file(
            &config_path,
            r#"
model_provider = "custom"

[model_providers.custom]
name = "Custom"
base_url = "https://old.example.com/v1"

[mcp_servers.keep]
command = "node"
"#,
        )
        .unwrap();
        let backup = fs::read_to_string(&config_path).unwrap();

        patch_codex_config(&config_path, "http://127.0.0.1:37123/openai/v1", false).unwrap();
        let patched = parse_toml_file(&config_path).unwrap();
        assert_eq!(
            patched["model_provider"].as_str(),
            Some(GATEWAY_PROVIDER_ID)
        );
        assert_eq!(
            patched["model_providers"][GATEWAY_PROVIDER_ID]["base_url"].as_str(),
            Some("http://127.0.0.1:37123/openai/v1")
        );
        assert!(patched["model_providers"][GATEWAY_PROVIDER_ID]
            .as_table_like()
            .expect("gateway provider table")
            .get("experimental_bearer_token")
            .is_none());
        assert_eq!(
            patched["mcp_servers"]["keep"]["command"].as_str(),
            Some("node")
        );

        restore_codex_config(&config_path, Some(&backup)).unwrap();
        let restored = parse_toml_file(&config_path).unwrap();
        assert_eq!(restored["model_provider"].as_str(), Some("custom"));
        assert!(restored["model_providers"]
            .get(GATEWAY_PROVIDER_ID)
            .is_none());
        assert_eq!(
            restored["mcp_servers"]["keep"]["command"].as_str(),
            Some("node")
        );
    }

    #[test]
    fn codex_takeover_with_auth_preservation_writes_config_token_and_keeps_auth() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let auth_path = dir.path().join("auth.json");
        write_json_file(
            &auth_path,
            &json!({
                "auth_mode": "chatgpt",
                "tokens": {"access_token": "official-access"},
                "last_refresh": "2026-06-14T00:00:00Z"
            }),
        )
        .unwrap();
        let original_auth = read_json_file(&auth_path).unwrap();

        patch_codex_config(&config_path, "http://127.0.0.1:37123/openai/v1", true).unwrap();
        patch_codex_auth(&auth_path, true, None).unwrap();

        let patched_config = parse_toml_file(&config_path).unwrap();
        assert_eq!(
            patched_config["model_provider"].as_str(),
            Some(GATEWAY_PROVIDER_ID)
        );
        assert_eq!(
            patched_config["model_providers"][GATEWAY_PROVIDER_ID]["experimental_bearer_token"]
                .as_str(),
            Some(GATEWAY_API_KEY)
        );

        let patched_auth = read_json_file(&auth_path).unwrap();
        assert_eq!(patched_auth, original_auth);
    }

    #[test]
    fn codex_takeover_with_auth_preservation_restores_previous_gateway_auth() {
        let dir = tempfile::tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        write_json_file(
            &auth_path,
            &json!({
                "OPENAI_API_KEY": GATEWAY_API_KEY,
                "auth_mode": "apikey",
                "tokens": {"access_token": "official-access"}
            }),
        )
        .unwrap();
        let backup = serde_json::to_string_pretty(&json!({
            "auth_mode": "chatgpt",
            "tokens": {"access_token": "official-access"}
        }))
        .unwrap();

        patch_codex_auth(&auth_path, true, Some(&backup)).unwrap();

        let patched_auth = read_json_file(&auth_path).unwrap();
        assert_eq!(patched_auth.get("OPENAI_API_KEY"), None);
        assert_eq!(
            patched_auth.get("auth_mode").and_then(Value::as_str),
            Some("chatgpt")
        );
        assert_eq!(
            patched_auth
                .pointer("/tokens/access_token")
                .and_then(Value::as_str),
            Some("official-access")
        );
    }

    #[test]
    fn codex_auth_restore_preserves_runtime_owned_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        write_json_file(
            &auth_path,
            &json!({
                "OPENAI_API_KEY": "old",
                "auth_mode": "apikey",
                "tokens": {"access": "keep"}
            }),
        )
        .unwrap();
        let backup = fs::read_to_string(&auth_path).unwrap();

        patch_codex_auth(&auth_path, false, None).unwrap();
        let patched = read_json_file(&auth_path).unwrap();
        assert_eq!(
            patched.get("OPENAI_API_KEY").and_then(Value::as_str),
            Some(GATEWAY_API_KEY)
        );
        assert_eq!(
            patched.pointer("/tokens/access").and_then(Value::as_str),
            Some("keep")
        );

        restore_codex_auth(&auth_path, Some(&backup)).unwrap();
        let restored = read_json_file(&auth_path).unwrap();
        assert_eq!(
            restored.get("OPENAI_API_KEY").and_then(Value::as_str),
            Some("old")
        );
        assert_eq!(
            restored.pointer("/tokens/access").and_then(Value::as_str),
            Some("keep")
        );
    }

    #[test]
    fn gemini_takeover_and_restore_keep_unmanaged_env_and_settings() {
        let dir = tempfile::tempdir().unwrap();
        let env_path = dir.path().join(".env");
        let settings_path = dir.path().join("settings.json");
        write_text_file(
            &env_path,
            "OTHER=1\nGEMINI_API_KEY=old\nGOOGLE_GEMINI_BASE_URL=https://old.example.com/v1beta\n",
        )
        .unwrap();
        write_json_file(
            &settings_path,
            &json!({
                "security": {"auth": {"selectedType": "oauth-personal"}},
                "ui": {"theme": "dark"}
            }),
        )
        .unwrap();
        let env_backup = fs::read_to_string(&env_path).unwrap();
        let settings_backup = fs::read_to_string(&settings_path).unwrap();

        patch_gemini_env(&env_path, "http://127.0.0.1:37123/gemini/v1beta").unwrap();
        patch_gemini_settings(&settings_path).unwrap();
        let patched_env = fs::read_to_string(&env_path).unwrap();
        assert!(patched_env.contains("OTHER=1"));
        assert!(patched_env.contains("GEMINI_API_KEY=ai-toolbox-gateway"));
        assert!(patched_env.contains("GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:37123/gemini/v1beta"));
        let patched_settings = read_json_file(&settings_path).unwrap();
        assert_eq!(
            patched_settings
                .pointer("/security/auth/selectedType")
                .and_then(Value::as_str),
            Some("gemini-api-key")
        );
        assert_eq!(
            patched_settings
                .pointer("/ui/theme")
                .and_then(Value::as_str),
            Some("dark")
        );

        restore_gemini_env(&env_path, Some(&env_backup)).unwrap();
        restore_gemini_settings(&settings_path, Some(&settings_backup)).unwrap();
        let restored_env = fs::read_to_string(&env_path).unwrap();
        assert!(restored_env.contains("OTHER=1"));
        assert!(restored_env.contains("GEMINI_API_KEY=old"));
        assert!(restored_env.contains("GOOGLE_GEMINI_BASE_URL=https://old.example.com/v1beta"));
        let restored_settings = read_json_file(&settings_path).unwrap();
        assert_eq!(
            restored_settings
                .pointer("/security/auth/selectedType")
                .and_then(Value::as_str),
            Some("oauth-personal")
        );
    }

    #[test]
    fn restore_availability_requires_backups_for_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let manifest = CliProxyManifest {
            schema_version: 1,
            managed_by: "ai-toolbox-proxy-gateway".to_string(),
            cli_key: GatewayCliKey::Claude,
            enabled: true,
            mode: GatewayProxyMode::Single,
            primary_provider_id: "provider-1".to_string(),
            base_origin: "http://127.0.0.1:37123".to_string(),
            created_at: "2026-05-17T00:00:00Z".to_string(),
            updated_at: "2026-05-17T00:00:00Z".to_string(),
            files: vec![CliProxyManifestFile {
                kind: CLAUDE_SETTINGS_KIND.to_string(),
                path: "settings.json".to_string(),
                existed: true,
                backup_rel_path: format!("{CLAUDE_SETTINGS_KIND}.bak"),
                backup_sha256: None,
                backup_size: None,
                managed_fields: Vec::new(),
            }],
        };

        assert!(!manifest_restore_available(
            &paths,
            GatewayCliKey::Claude,
            &manifest
        ));
        fs::create_dir_all(paths.backup_dir(GatewayCliKey::Claude)).unwrap();
        fs::write(
            paths
                .backup_dir(GatewayCliKey::Claude)
                .join(format!("{CLAUDE_SETTINGS_KIND}.bak")),
            "{}",
        )
        .unwrap();
        assert!(manifest_restore_available(
            &paths,
            GatewayCliKey::Claude,
            &manifest
        ));
    }

    #[test]
    fn manifest_mode_state_machine_round_trip_restores_direct_config() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path().join("app-data"));
        let settings_path = dir.path().join("runtime").join("settings.json");
        write_json_file(
            &settings_path,
            &json!({"env": {"ANTHROPIC_BASE_URL": "https://original.example.com"}}),
        )
        .unwrap();
        let targets = CliProxyTargets {
            runtime_root: dir.path().join("runtime"),
            is_wsl_direct: false,
            files: vec![CliProxyTarget {
                kind: CLAUDE_SETTINGS_KIND,
                path: settings_path.clone(),
                managed_fields: &CLAUDE_MANAGED_FIELDS,
            }],
        };
        let primary_provider = claude_test_provider(
            Some("provider-default"),
            Some("provider-haiku"),
            Some("provider-sonnet"),
            Some("provider-opus"),
        );

        let single_manifest = prepare_manifest(
            &paths,
            GatewayCliKey::Claude,
            "http://127.0.0.1:37123",
            &targets,
            GatewayProxyMode::Single,
            "provider-1",
        )
        .unwrap();
        apply_gateway_config(
            GatewayCliKey::Claude,
            &targets,
            "http://127.0.0.1:37123",
            Some(&primary_provider),
            GatewayProxyMode::Single,
            None,
            None,
            false,
        )
        .unwrap();
        write_manifest(&paths, GatewayCliKey::Claude, &single_manifest).unwrap();

        let engaged = read_manifest(&paths, GatewayCliKey::Claude)
            .unwrap()
            .unwrap();
        assert!(engaged.enabled);
        assert_eq!(engaged.mode, GatewayProxyMode::Single);
        assert_eq!(engaged.primary_provider_id, "provider-1");

        let mut failover_manifest = engaged.clone();
        failover_manifest.mode = GatewayProxyMode::Failover;
        write_manifest(&paths, GatewayCliKey::Claude, &failover_manifest).unwrap();
        let failover = read_manifest(&paths, GatewayCliKey::Claude)
            .unwrap()
            .unwrap();
        assert_eq!(failover.mode, GatewayProxyMode::Failover);
        assert_eq!(failover.primary_provider_id, "provider-1");

        let mut single_again_manifest = failover.clone();
        single_again_manifest.mode = GatewayProxyMode::Single;
        write_manifest(&paths, GatewayCliKey::Claude, &single_again_manifest).unwrap();
        let single_again = read_manifest(&paths, GatewayCliKey::Claude)
            .unwrap()
            .unwrap();
        assert_eq!(single_again.mode, GatewayProxyMode::Single);

        restore_gateway_config(GatewayCliKey::Claude, &paths, &targets, &single_again).unwrap();
        let mut restored_manifest = single_again;
        restored_manifest.enabled = false;
        write_manifest(&paths, GatewayCliKey::Claude, &restored_manifest).unwrap();

        let restored = read_json_file(&settings_path).unwrap();
        assert_eq!(
            restored
                .pointer("/env/ANTHROPIC_BASE_URL")
                .and_then(Value::as_str),
            Some("https://original.example.com")
        );
        let final_manifest = read_manifest(&paths, GatewayCliKey::Claude)
            .unwrap()
            .unwrap();
        assert!(!final_manifest.enabled);
        assert_eq!(final_manifest.mode, GatewayProxyMode::Single);
        assert_eq!(final_manifest.primary_provider_id, "provider-1");
    }

    #[test]
    fn old_manifest_without_mode_requires_reengage() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let manifest_path = paths.manifest_path(GatewayCliKey::Claude);
        write_text_file(
            &manifest_path,
            r#"{
  "schema_version": 1,
  "managed_by": "ai-toolbox-proxy-gateway",
  "cli_key": "claude",
  "enabled": true,
  "base_origin": "http://127.0.0.1:37123",
  "created_at": "2026-05-17T00:00:00Z",
  "updated_at": "2026-05-17T00:00:00Z",
  "files": []
}
"#,
        )
        .unwrap();

        let error = read_manifest(&paths, GatewayCliKey::Claude).unwrap_err();

        assert!(error.needs_reengage());
        assert!(error.to_string().contains("Click Gateway proxy"));
    }

    #[test]
    fn provider_switch_lock_tracks_enabled_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let mut manifest = CliProxyManifest::new(
            GatewayCliKey::Claude,
            "http://127.0.0.1:37123".to_string(),
            "2026-05-17T00:00:00Z".to_string(),
            GatewayProxyMode::Single,
            "provider-1".to_string(),
        );

        assert!(!provider_switch_locked_by_manifest(
            &paths,
            GatewayCliKey::Claude
        ));

        write_manifest(&paths, GatewayCliKey::Claude, &manifest).unwrap();
        assert!(provider_switch_locked_by_manifest(
            &paths,
            GatewayCliKey::Claude
        ));

        manifest.enabled = false;
        write_manifest(&paths, GatewayCliKey::Claude, &manifest).unwrap();
        assert!(!provider_switch_locked_by_manifest(
            &paths,
            GatewayCliKey::Claude
        ));
    }

    #[test]
    fn wsl_gateway_mapping_targets_are_limited_to_gateway_managed_files() {
        assert_eq!(
            wsl_synced_gateway_target_for_mapping("claude-settings"),
            Some((GatewayCliKey::Claude, CLAUDE_SETTINGS_KIND))
        );
        assert_eq!(
            wsl_synced_gateway_target_for_mapping("codex-config"),
            Some((GatewayCliKey::Codex, CODEX_CONFIG_KIND))
        );
        assert_eq!(
            wsl_synced_gateway_target_for_mapping("geminicli-env"),
            Some((GatewayCliKey::Gemini, GEMINI_ENV_KIND))
        );
        assert_eq!(
            wsl_synced_gateway_target_for_mapping("geminicli-settings"),
            None
        );
        assert_eq!(wsl_synced_gateway_target_for_mapping("codex-auth"), None);
    }

    #[test]
    fn wsl_gateway_rewrite_updates_only_claude_gateway_base_url() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let manifest = test_manifest_with_file(
            GatewayCliKey::Claude,
            CLAUDE_SETTINGS_KIND,
            &CLAUDE_MANAGED_FIELDS,
        );
        write_manifest(&paths, GatewayCliKey::Claude, &manifest).unwrap();

        let content = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "http://127.0.0.1:37123/anthropic",
                "ANTHROPIC_AUTH_TOKEN": "ai-toolbox-gateway",
                "OTHER_LOCAL": "http://127.0.0.1:9999"
            },
            "hooks": {
                "local": "http://127.0.0.1:8899/hook"
            }
        })
        .to_string();

        let rewritten = rewrite_wsl_synced_gateway_target_content(
            &paths,
            &test_proxy_gateway_settings("172.20.10.1"),
            GatewayCliKey::Claude,
            CLAUDE_SETTINGS_KIND,
            &content,
        )
        .unwrap()
        .unwrap();
        let rewritten_json = serde_json::from_str::<Value>(&rewritten).unwrap();

        assert_eq!(
            rewritten_json
                .pointer("/env/ANTHROPIC_BASE_URL")
                .and_then(Value::as_str),
            Some("http://172.20.10.1:37123/anthropic")
        );
        assert_eq!(
            rewritten_json
                .pointer("/env/OTHER_LOCAL")
                .and_then(Value::as_str),
            Some("http://127.0.0.1:9999")
        );
        assert_eq!(
            rewritten_json
                .pointer("/hooks/local")
                .and_then(Value::as_str),
            Some("http://127.0.0.1:8899/hook")
        );
    }

    #[test]
    fn wsl_gateway_rewrite_skips_claude_without_gateway_sentinel() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let manifest = test_manifest_with_file(
            GatewayCliKey::Claude,
            CLAUDE_SETTINGS_KIND,
            &CLAUDE_MANAGED_FIELDS,
        );
        write_manifest(&paths, GatewayCliKey::Claude, &manifest).unwrap();

        let content = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "http://127.0.0.1:37123/anthropic",
                "ANTHROPIC_AUTH_TOKEN": "user-token"
            }
        })
        .to_string();

        let rewritten = rewrite_wsl_synced_gateway_target_content(
            &paths,
            &test_proxy_gateway_settings("172.20.10.1"),
            GatewayCliKey::Claude,
            CLAUDE_SETTINGS_KIND,
            &content,
        )
        .unwrap();

        assert!(rewritten.is_none());
    }

    #[test]
    fn wsl_gateway_rewrite_updates_codex_gateway_provider_only() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let manifest = test_manifest_with_file(
            GatewayCliKey::Codex,
            CODEX_CONFIG_KIND,
            &CODEX_CONFIG_MANAGED_FIELDS,
        );
        write_manifest(&paths, GatewayCliKey::Codex, &manifest).unwrap();

        let content = r#"
model_provider = "ai-toolbox-gateway"

[model_providers.ai-toolbox-gateway]
name = "AI Toolbox Gateway"
base_url = "http://127.0.0.1:37123/openai/v1"

[model_providers.local]
name = "Local service"
base_url = "http://127.0.0.1:9999/v1"
"#;

        let rewritten = rewrite_wsl_synced_gateway_target_content(
            &paths,
            &test_proxy_gateway_settings("172.20.10.1"),
            GatewayCliKey::Codex,
            CODEX_CONFIG_KIND,
            content,
        )
        .unwrap()
        .unwrap();
        let rewritten_document = parse_toml_document(&rewritten, "rewritten Codex config").unwrap();

        assert_eq!(
            rewritten_document["model_providers"]["ai-toolbox-gateway"]["base_url"].as_str(),
            Some("http://172.20.10.1:37123/openai/v1")
        );
        assert_eq!(
            rewritten_document["model_providers"]["local"]["base_url"].as_str(),
            Some("http://127.0.0.1:9999/v1")
        );
    }

    #[test]
    fn wsl_gateway_rewrite_updates_gemini_gateway_env_only() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let manifest = test_manifest_with_file(
            GatewayCliKey::Gemini,
            GEMINI_ENV_KIND,
            &GEMINI_MANAGED_ENV_KEYS,
        );
        write_manifest(&paths, GatewayCliKey::Gemini, &manifest).unwrap();

        let content = "OTHER=http://127.0.0.1:9999\nGEMINI_API_KEY=ai-toolbox-gateway\nGOOGLE_GEMINI_BASE_URL=http://127.0.0.1:37123/gemini/v1beta\n";

        let rewritten = rewrite_wsl_synced_gateway_target_content(
            &paths,
            &test_proxy_gateway_settings("172.20.10.1"),
            GatewayCliKey::Gemini,
            GEMINI_ENV_KIND,
            content,
        )
        .unwrap()
        .unwrap();
        let rewritten_env = parse_env_content(&rewritten);

        assert_eq!(
            rewritten_env
                .get("GOOGLE_GEMINI_BASE_URL")
                .map(String::as_str),
            Some("http://172.20.10.1:37123/gemini/v1beta")
        );
        assert_eq!(
            rewritten_env.get("OTHER").map(String::as_str),
            Some("http://127.0.0.1:9999")
        );
    }

    #[test]
    fn retakeover_reuses_original_backup_instead_of_backing_up_gateway_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path().join("app-data"));
        let settings_path = dir.path().join("runtime").join("settings.json");
        write_json_file(
            &settings_path,
            &json!({"env": {"ANTHROPIC_BASE_URL": "https://original.example.com"}}),
        )
        .unwrap();
        let targets = CliProxyTargets {
            runtime_root: dir.path().join("runtime"),
            is_wsl_direct: false,
            files: vec![CliProxyTarget {
                kind: CLAUDE_SETTINGS_KIND,
                path: settings_path.clone(),
                managed_fields: &CLAUDE_MANAGED_FIELDS,
            }],
        };

        let first_manifest = prepare_manifest(
            &paths,
            GatewayCliKey::Claude,
            "http://127.0.0.1:37123",
            &targets,
            GatewayProxyMode::Single,
            "provider-1",
        )
        .unwrap();
        write_manifest(&paths, GatewayCliKey::Claude, &first_manifest).unwrap();
        write_json_file(
            &settings_path,
            &json!({"env": {"ANTHROPIC_BASE_URL": "http://127.0.0.1:37123/anthropic"}}),
        )
        .unwrap();

        let second_manifest = prepare_manifest(
            &paths,
            GatewayCliKey::Claude,
            "http://127.0.0.1:37124",
            &targets,
            GatewayProxyMode::Single,
            "provider-1",
        )
        .unwrap();
        let backup = backup_content(
            &paths,
            GatewayCliKey::Claude,
            &second_manifest,
            CLAUDE_SETTINGS_KIND,
        )
        .unwrap()
        .unwrap();
        let backup_json = serde_json::from_str::<Value>(&backup).unwrap();

        assert_eq!(
            backup_json
                .pointer("/env/ANTHROPIC_BASE_URL")
                .and_then(Value::as_str),
            Some("https://original.example.com")
        );
        assert_eq!(second_manifest.files.len(), 1);
    }

    #[test]
    fn gateway_endpoints_are_cli_specific() {
        assert_eq!(
            cli_gateway_endpoint(GatewayCliKey::Claude, "http://127.0.0.1:37123/"),
            "http://127.0.0.1:37123/anthropic"
        );
        assert_eq!(
            cli_gateway_endpoint(GatewayCliKey::Codex, "http://127.0.0.1:37123"),
            "http://127.0.0.1:37123/openai/v1"
        );
        assert_eq!(
            cli_gateway_endpoint(GatewayCliKey::Gemini, "http://127.0.0.1:37123"),
            "http://127.0.0.1:37123/gemini/v1beta"
        );
    }

    #[test]
    fn replace_origin_host_swaps_loopback_to_lan_ip() {
        assert_eq!(
            replace_origin_host("http://127.0.0.1:37123", "192.168.1.20"),
            "http://192.168.1.20:37123"
        );
    }

    #[test]
    fn replace_origin_host_preserves_scheme_and_port() {
        assert_eq!(
            replace_origin_host("https://localhost:38443", "10.0.0.8"),
            "https://10.0.0.8:38443"
        );
    }

    #[test]
    fn replace_origin_host_returns_original_when_no_port() {
        assert_eq!(
            replace_origin_host("http://127.0.0.1", "192.168.1.20"),
            "http://127.0.0.1"
        );
    }

    #[test]
    fn resolve_effective_base_origin_uses_wsl_host_when_wsl_direct() {
        assert_eq!(
            resolve_effective_base_origin("http://127.0.0.1:37123", true, " 192.168.1.20 "),
            "http://192.168.1.20:37123"
        );
    }

    #[test]
    fn resolve_effective_base_origin_ignores_wsl_host_when_not_wsl_direct() {
        assert_eq!(
            resolve_effective_base_origin("http://127.0.0.1:37123", false, "192.168.1.20"),
            "http://127.0.0.1:37123"
        );
    }

    #[test]
    fn resolve_effective_base_origin_ignores_empty_wsl_host() {
        assert_eq!(
            resolve_effective_base_origin("http://127.0.0.1:37123", true, " "),
            "http://127.0.0.1:37123"
        );
    }

    #[test]
    fn restore_env_content_removes_gateway_env_when_backup_has_no_managed_values() {
        let restored = restore_env_content(
            "OTHER=1\nGEMINI_API_KEY=ai-toolbox-gateway\nGOOGLE_GEMINI_BASE_URL=http://127.0.0.1:37123/gemini/v1beta\n",
            &BTreeMap::new(),
        );

        assert_eq!(restored, "OTHER=1\n");
    }

    #[test]
    fn stop_preflight_blocks_enabled_manifest_even_when_restore_backup_is_missing() {
        let status = build_status(
            GatewayCliKey::Claude,
            GatewayCliTakeoverState::RestoreUnavailable,
            GatewayCliStatusDot::Red,
            true,
            false,
            Some("http://127.0.0.1:37123".to_string()),
            Some("runtime".to_string()),
            Vec::new(),
            Some("backup missing".to_string()),
        );

        assert!(blocks_gateway_stop(&status));
    }

    #[test]
    fn stop_preflight_does_not_block_direct_cli() {
        let status = build_status(
            GatewayCliKey::Claude,
            GatewayCliTakeoverState::Direct,
            GatewayCliStatusDot::Gray,
            true,
            false,
            Some("http://127.0.0.1:37123".to_string()),
            Some("runtime".to_string()),
            Vec::new(),
            None,
        );

        assert!(!blocks_gateway_stop(&status));
    }

    #[test]
    fn stop_preflight_blocks_no_provider_only_when_cli_is_still_taken_over() {
        let taken_over_status = build_status(
            GatewayCliKey::Claude,
            GatewayCliTakeoverState::NoProxyProvider,
            GatewayCliStatusDot::Orange,
            false,
            true,
            Some("http://127.0.0.1:37123".to_string()),
            Some("runtime".to_string()),
            Vec::new(),
            Some(NO_PROXYABLE_PROVIDER_MESSAGE.to_string()),
        );
        let direct_status = build_status(
            GatewayCliKey::Claude,
            GatewayCliTakeoverState::NoProxyProvider,
            GatewayCliStatusDot::Orange,
            false,
            false,
            Some("http://127.0.0.1:37123".to_string()),
            Some("runtime".to_string()),
            Vec::new(),
            Some(NO_PROXYABLE_PROVIDER_MESSAGE.to_string()),
        );

        assert!(blocks_gateway_stop(&taken_over_status));
        assert!(!blocks_gateway_stop(&direct_status));
    }

    #[test]
    fn stop_preflight_blocks_error_status_when_restore_is_available() {
        let status = build_status(
            GatewayCliKey::Claude,
            GatewayCliTakeoverState::Error,
            GatewayCliStatusDot::Red,
            false,
            true,
            Some("http://127.0.0.1:37123".to_string()),
            Some("runtime".to_string()),
            Vec::new(),
            Some("provider parse failed".to_string()),
        );

        assert!(blocks_gateway_stop(&status));
    }
}
