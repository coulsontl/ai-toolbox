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
use crate::coding::claude_desktop::config_writer as claude_desktop_config_writer;
use crate::coding::runtime_location::{self, RuntimeLocationMode};
use crate::db::helpers::db_get;
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut, Item};

const GATEWAY_PROVIDER_ID: &str = "ai-toolbox-gateway";
const GATEWAY_API_KEY: &str = "ai-toolbox-gateway";
const CLAUDE_STANDARD_MODEL: &str = "claude-sonnet-5";
const CLAUDE_STANDARD_HAIKU_MODEL: &str = "claude-haiku-4-5";
const CLAUDE_STANDARD_SONNET_MODEL: &str = "claude-sonnet-5";
const CLAUDE_STANDARD_OPUS_MODEL: &str = "claude-opus-5";
const CLAUDE_STANDARD_FABLE_MODEL: &str = "claude-fable-5";
const CLAUDE_SETTINGS_KIND: &str = "claude_settings_json";
const CODEX_CONFIG_KIND: &str = "codex_config_toml";
const CODEX_AUTH_KIND: &str = "codex_auth_json";
const GROK_CONFIG_KIND: &str = "grok_config_toml";
const KIMI_CONFIG_KIND: &str = "kimi_config_toml";
const GEMINI_ENV_KIND: &str = "gemini_env";
const GEMINI_SETTINGS_KIND: &str = "gemini_settings_json";
const DESKTOP_NORMAL_CONFIG_KIND: &str = "claude_desktop_normal_config_json";
const DESKTOP_THREEP_CONFIG_KIND: &str = "claude_desktop_threep_config_json";
const DESKTOP_PROFILE_KIND: &str = "claude_desktop_profile_json";
const DESKTOP_META_KIND: &str = "claude_desktop_meta_json";

// Claude Desktop's gateway-managed fields, expressed as JSON paths so the
// manifest tracks what the gateway owns per file.
const DESKTOP_NORMAL_MANAGED_FIELDS: [&str; 1] = ["deploymentMode"];

const DESKTOP_THREEP_MANAGED_FIELDS: [&str; 6] = [
    "deploymentMode",
    "enterpriseConfig.disableDeploymentModeChooser",
    "enterpriseConfig.inferenceGatewayApiKey",
    "enterpriseConfig.inferenceGatewayAuthScheme",
    "enterpriseConfig.inferenceGatewayBaseUrl",
    "enterpriseConfig.inferenceProvider",
];

// The on-disk 3P profile file is wholly owned by the gateway.
const DESKTOP_PROFILE_MANAGED_FIELDS: [&str; 5] = [
    "inferenceGatewayApiKey",
    "inferenceGatewayBaseUrl",
    "inferenceProvider",
    "inferenceModels",
    "coworkEgressAllowedHosts",
];

const DESKTOP_META_MANAGED_FIELDS: [&str; 2] = ["appliedId", "entries"];

/// Sentinel token written into Claude Desktop's 3P profile auth so the gateway
/// profile remains syntactically complete; the local gateway does not require it.
const DESKTOP_GATEWAY_API_KEY: &str = GATEWAY_API_KEY;

const CLAUDE_MANAGED_FIELDS: [&str; 12] = [
    "env.ANTHROPIC_BASE_URL",
    "env.ANTHROPIC_AUTH_TOKEN",
    "env.ANTHROPIC_API_KEY",
    "env.ANTHROPIC_MODEL",
    "env.ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "env.ANTHROPIC_DEFAULT_SONNET_MODEL",
    "env.ANTHROPIC_DEFAULT_OPUS_MODEL",
    "env.ANTHROPIC_DEFAULT_FABLE_MODEL",
    "env.ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    "env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    "env.ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
    "env.ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
];

const CLAUDE_MODEL_FIELD_POINTERS: [&str; 9] = [
    "/env/ANTHROPIC_MODEL",
    "/env/ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "/env/ANTHROPIC_DEFAULT_SONNET_MODEL",
    "/env/ANTHROPIC_DEFAULT_OPUS_MODEL",
    "/env/ANTHROPIC_DEFAULT_FABLE_MODEL",
    "/env/ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    "/env/ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    "/env/ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
    "/env/ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
];

const CLAUDE_LEGACY_REASONING_MODEL_POINTER: &str = "/env/ANTHROPIC_REASONING_MODEL";

/// Default Codex model_provider id used by AI Toolbox / unified history.
const DEFAULT_CODEX_PROVIDER_ID: &str = "custom";

const CODEX_AUTH_MANAGED_FIELDS: [&str; 2] = ["OPENAI_API_KEY", "auth_mode"];
const GROK_CONFIG_MANAGED_FIELDS: [&str; 2] = ["models.default", "model.ai-toolbox-gateway"];

/// Default Kimi provider table key (official managed provider). Custom applied
/// providers project their own key, so the takeover target is resolved from
/// `default_model -> models.<key>.provider` at patch/status/WSL time.
const DEFAULT_KIMI_PROVIDER_KEY: &str = "managed:kimi-code";

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
    managed_fields: Vec<String>,
}

fn static_managed_fields(fields: &[&str]) -> Vec<String> {
    fields.iter().map(|field| (*field).to_string()).collect()
}

fn codex_config_managed_fields_for_provider(provider_id: &str) -> Vec<String> {
    vec![
        format!("model_providers.{provider_id}.base_url"),
        format!("model_providers.{provider_id}.wire_api"),
        format!("model_providers.{provider_id}.experimental_bearer_token"),
    ]
}

fn is_codex_gateway_managed_fields(managed_fields: &[String]) -> bool {
    managed_fields.iter().any(|field| {
        field == "model_providers.ai-toolbox-gateway"
            || (field.starts_with("model_providers.") && field.ends_with(".base_url"))
    })
}

fn kimi_config_managed_fields_for_provider(provider_key: &str) -> Vec<String> {
    vec![
        format!("providers.{provider_key}.type"),
        format!("providers.{provider_key}.base_url"),
        format!("providers.{provider_key}.api_key"),
    ]
}

fn is_kimi_gateway_managed_fields(managed_fields: &[String]) -> bool {
    managed_fields
        .iter()
        .any(|field| field.starts_with("providers.") && field.ends_with(".base_url"))
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

    let mut targets = resolve_targets(db, cli_key).await?;
    let settings = settings::load_settings_from_sqlite_state(db)?;
    let effective_origin =
        resolve_effective_base_origin(base_origin, targets.is_wsl_direct, &settings.wsl_host);
    let mut manifest = prepare_manifest(
        paths,
        cli_key,
        &effective_origin,
        &targets,
        GatewayProxyMode::Single,
        &primary_provider_id,
    )?;
    // Persist enabled manifest + backup metadata before rewriting runtime files.
    // If apply_gateway_config fails mid-way, retry must still reuse the original .bak.
    sync_manifest_managed_fields(&mut manifest, &targets);
    write_manifest(paths, cli_key, &manifest)?;
    let codex_auth_backup_content = codex_auth_backup_content_for_cli(paths, cli_key, &manifest)?;
    if let Err(error) = apply_gateway_config(
        db,
        cli_key,
        &mut targets,
        &effective_origin,
        Some(&primary_provider),
        GatewayProxyMode::Single,
        None,
        codex_auth_backup_content.as_deref(),
        codex_auth_preservation_enabled_for_cli(db, cli_key)?,
    ) {
        // Early enabled manifest protects the original .bak, but a failed apply must not leave
        // the CLI looking "taken over" with a half-patched runtime config.
        let _ = restore_gateway_config(cli_key, paths, &targets, &manifest);
        manifest.enabled = false;
        manifest.updated_at = chrono::Utc::now().to_rfc3339();
        let _ = write_manifest(paths, cli_key, &manifest);
        return Err(error);
    }
    sync_manifest_managed_fields(&mut manifest, &targets);
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
        let mut targets = resolve_targets(db, cli_key).await?;
        let codex_auth_backup_content =
            codex_auth_backup_content_for_cli(paths, cli_key, &manifest)?;
        if let Err(error) = apply_gateway_config(
            db,
            cli_key,
            &mut targets,
            &manifest.base_origin,
            Some(&primary_provider),
            GatewayProxyMode::Failover,
            None,
            codex_auth_backup_content.as_deref(),
            codex_auth_preservation_enabled_for_cli(db, cli_key)?,
        ) {
            // Roll back half-applied failover fields to the single-mode gateway config.
            let _ = apply_gateway_config(
                db,
                cli_key,
                &mut targets,
                &manifest.base_origin,
                Some(&primary_provider),
                GatewayProxyMode::Single,
                None,
                codex_auth_backup_content.as_deref(),
                codex_auth_preservation_enabled_for_cli(db, cli_key)?,
            );
            return Err(error);
        }
        sync_manifest_managed_fields(&mut manifest, &targets);
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
        let mut targets = resolve_targets(db, cli_key).await?;
        let claude_backup_content = if cli_key == GatewayCliKey::Claude {
            backup_content(paths, cli_key, &manifest, CLAUDE_SETTINGS_KIND)?
                .or_else(|| Some("{}".to_string()))
        } else {
            None
        };
        let codex_auth_backup_content =
            codex_auth_backup_content_for_cli(paths, cli_key, &manifest)?;
        apply_gateway_config(
            db,
            cli_key,
            &mut targets,
            &manifest.base_origin,
            Some(&primary_provider),
            GatewayProxyMode::Single,
            claude_backup_content.as_deref(),
            codex_auth_backup_content.as_deref(),
            codex_auth_preservation_enabled_for_cli(db, cli_key)?,
        )?;
        sync_manifest_managed_fields(&mut manifest, &targets);
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
    // Drop original snapshots after a successful restore so the next engage re-backs up
    // the post-direct runtime files instead of reusing a stale first-engage .bak.
    clear_gateway_backups(paths, cli_key, &manifest);
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

/// A new data root cannot recover runtime files using manifests left in the old root.
pub(crate) fn ensure_data_dir_can_change(paths: &ProxyGatewayPaths) -> Result<(), String> {
    for cli_key in GatewayCliKey::supported_mvp() {
        let manifest = read_manifest(paths, cli_key).map_err(|error| error.to_string())?;
        if manifest.is_some_and(|manifest| manifest.enabled) {
            return Err(format!(
                "请先将 {} 恢复为直连，再备份并更改应用数据目录；网关接管记录不会自动迁移",
                cli_key.as_str(),
            ));
        }
    }
    Ok(())
}

pub fn wsl_synced_gateway_target_for_mapping(
    mapping_id: &str,
) -> Option<(GatewayCliKey, &'static str)> {
    match mapping_id {
        "claude-settings" => Some((GatewayCliKey::Claude, CLAUDE_SETTINGS_KIND)),
        "codex-config" => Some((GatewayCliKey::Codex, CODEX_CONFIG_KIND)),
        "grok-config" => Some((GatewayCliKey::Grok, GROK_CONFIG_KIND)),
        "kimi-config" => Some((GatewayCliKey::Kimi, KIMI_CONFIG_KIND)),
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
            if is_codex_gateway_managed_fields(&managed_file.managed_fields) =>
        {
            rewrite_codex_wsl_gateway_content(
                content,
                &windows_gateway_endpoint,
                &wsl_gateway_endpoint,
            )
        }
        (GatewayCliKey::Grok, GROK_CONFIG_KIND)
            if managed_file
                .managed_fields
                .iter()
                .any(|field| field == "model.ai-toolbox-gateway") =>
        {
            rewrite_grok_wsl_gateway_content(
                content,
                &windows_gateway_endpoint,
                &wsl_gateway_endpoint,
            )
        }
        (GatewayCliKey::Kimi, KIMI_CONFIG_KIND)
            if is_kimi_gateway_managed_fields(&managed_file.managed_fields) =>
        {
            rewrite_kimi_wsl_gateway_content(
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
        GatewayCliKey::Claude
            | GatewayCliKey::ClaudeDesktop
            | GatewayCliKey::Codex
            | GatewayCliKey::Grok
            | GatewayCliKey::Kimi
            | GatewayCliKey::Gemini
    )
}

async fn resolve_targets(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
) -> Result<CliProxyTargets, String> {
    // Claude Desktop is a config-file-path module (not a CLI root dir): it owns
    // four on-disk files (normal + 3p config, profile, meta). There is no WSL
    // Direct variant, so is_wsl_direct stays false.
    if cli_key == GatewayCliKey::ClaudeDesktop {
        let desktop = claude_desktop_config_writer::current_platform_paths()
            .map_err(|error| format!("Claude Desktop: {error}"))?;
        let files = vec![
            CliProxyTarget {
                kind: DESKTOP_NORMAL_CONFIG_KIND,
                path: desktop.normal_config_path,
                managed_fields: static_managed_fields(&DESKTOP_NORMAL_MANAGED_FIELDS),
            },
            CliProxyTarget {
                kind: DESKTOP_THREEP_CONFIG_KIND,
                path: desktop.threep_config_path,
                managed_fields: static_managed_fields(&DESKTOP_THREEP_MANAGED_FIELDS),
            },
            CliProxyTarget {
                kind: DESKTOP_PROFILE_KIND,
                path: desktop.profile_path,
                managed_fields: static_managed_fields(&DESKTOP_PROFILE_MANAGED_FIELDS),
            },
            CliProxyTarget {
                kind: DESKTOP_META_KIND,
                path: desktop.meta_path,
                managed_fields: static_managed_fields(&DESKTOP_META_MANAGED_FIELDS),
            },
        ];
        return Ok(CliProxyTargets {
            runtime_root: desktop.config_library_path,
            is_wsl_direct: false,
            files,
        });
    }

    let location = match cli_key {
        GatewayCliKey::Claude => runtime_location::get_claude_runtime_location_async(db).await?,
        GatewayCliKey::Codex => runtime_location::get_codex_runtime_location_async(db).await?,
        GatewayCliKey::Grok => runtime_location::get_grok_runtime_location_async(db).await?,
        GatewayCliKey::Kimi => runtime_location::get_kimi_runtime_location_async(db).await?,
        GatewayCliKey::Gemini => {
            runtime_location::get_gemini_cli_runtime_location_async(db).await?
        }
        GatewayCliKey::OpenCode => {
            return Err(
                "OpenCode adapter is intentionally out of scope for the gateway MVP".to_string(),
            )
        }
        GatewayCliKey::ClaudeDesktop => {
            unreachable!("ClaudeDesktop is handled by the early return in resolve_targets")
        }
    };
    let is_wsl_direct = location.mode == RuntimeLocationMode::WslDirect;
    let runtime_root = location.host_path;

    let files = match cli_key {
        GatewayCliKey::Claude => vec![CliProxyTarget {
            kind: CLAUDE_SETTINGS_KIND,
            path: runtime_root.join("settings.json"),
            managed_fields: static_managed_fields(&CLAUDE_MANAGED_FIELDS),
        }],
        GatewayCliKey::Codex => vec![
            CliProxyTarget {
                kind: CODEX_CONFIG_KIND,
                path: runtime_root.join("config.toml"),
                // Placeholder until patch refreshes to the live active provider id.
                managed_fields: codex_config_managed_fields_for_provider(DEFAULT_CODEX_PROVIDER_ID),
            },
            CliProxyTarget {
                kind: CODEX_AUTH_KIND,
                path: runtime_root.join("auth.json"),
                managed_fields: static_managed_fields(&CODEX_AUTH_MANAGED_FIELDS),
            },
        ],
        GatewayCliKey::Grok => vec![CliProxyTarget {
            kind: GROK_CONFIG_KIND,
            path: runtime_root.join("config.toml"),
            managed_fields: static_managed_fields(&GROK_CONFIG_MANAGED_FIELDS),
        }],
        GatewayCliKey::Kimi => vec![CliProxyTarget {
            kind: KIMI_CONFIG_KIND,
            path: runtime_root.join("config.toml"),
            managed_fields: kimi_config_managed_fields_for_provider(DEFAULT_KIMI_PROVIDER_KEY),
        }],
        GatewayCliKey::Gemini => vec![
            CliProxyTarget {
                kind: GEMINI_ENV_KIND,
                path: runtime_root.join(".env"),
                managed_fields: static_managed_fields(&GEMINI_MANAGED_ENV_KEYS),
            },
            CliProxyTarget {
                kind: GEMINI_SETTINGS_KIND,
                path: runtime_root.join("settings.json"),
                managed_fields: static_managed_fields(&GEMINI_SETTINGS_MANAGED_FIELDS),
            },
        ],
        GatewayCliKey::OpenCode => Vec::new(),
        GatewayCliKey::ClaudeDesktop => {
            unreachable!("ClaudeDesktop is handled by the early return in resolve_targets")
        }
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
    })?;
    crate::coding::proxy_gateway::runtime::clear_gateway_provider_selection_cache();
    Ok(())
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
                file.managed_fields = target.managed_fields.clone();
                file
            }
            None => backup_target_file(target, &backup_dir)?,
        };
        files.push(file);
    }
    manifest.files = files;
    Ok(manifest)
}

fn clear_gateway_backups(
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    manifest: &CliProxyManifest,
) {
    let backup_dir = paths.backup_dir(cli_key);
    for file in &manifest.files {
        if validate_backup_rel_path(&file.backup_rel_path).is_err() {
            continue;
        }
        let backup_path = backup_dir.join(&file.backup_rel_path);
        if backup_path.exists() {
            if let Err(error) = fs::remove_file(&backup_path) {
                log::warn!(
                    "Failed to remove gateway backup {} after restore: {}",
                    backup_path.display(),
                    error
                );
            }
        }
    }
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

    // Never re-backup over an existing original snapshot. A failed first engage can leave a
    // patched runtime file without an enabled manifest; retry must keep the first .bak.
    if backup_path.exists() {
        if let Ok(content) = fs::read(&backup_path) {
            backup_size = Some(content.len() as u64);
            backup_sha256 = Some(sha256_hex(&content));
        }
        return Ok(CliProxyManifestFile {
            kind: target.kind.to_string(),
            path: path_to_string(&target.path),
            // Backup presence means the original file existed when first engaged.
            existed: true,
            backup_rel_path,
            backup_sha256,
            backup_size,
            managed_fields: target.managed_fields.clone(),
        });
    }

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
        managed_fields: target.managed_fields.clone(),
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

fn sync_manifest_managed_fields(manifest: &mut CliProxyManifest, targets: &CliProxyTargets) {
    for target in &targets.files {
        if let Some(file) = manifest
            .files
            .iter_mut()
            .find(|file| file.kind == target.kind)
        {
            file.managed_fields = target.managed_fields.clone();
        }
    }
}

fn apply_gateway_config(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
    targets: &mut CliProxyTargets,
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
        GatewayCliKey::ClaudeDesktop => {
            let Some(primary_provider) = primary_provider else {
                return Err("Claude Desktop Gateway proxy requires a primary provider".to_string());
            };
            log::debug!("[desktop-gateway] apply_gateway_config entry");
            let model_specs = desktop_gateway_model_specs(db, &primary_provider.id)?;
            log::debug!(
                "[desktop-gateway] model_specs count = {}",
                model_specs.len()
            );
            let result = apply_desktop_gateway_config(
                targets,
                &cli_gateway_endpoint(cli_key, base_origin),
                model_specs,
            );
            log::debug!(
                "[desktop-gateway] apply_desktop_gateway_config result = {:?}",
                result.is_ok()
            );
            result
        }
        GatewayCliKey::Codex => {
            let provider_id = patch_codex_config(
                required_target_path(targets, CODEX_CONFIG_KIND)?,
                &cli_gateway_endpoint(cli_key, base_origin),
                preserve_codex_official_auth,
            )?;
            if let Some(target) = targets
                .files
                .iter_mut()
                .find(|file| file.kind == CODEX_CONFIG_KIND)
            {
                target.managed_fields = codex_config_managed_fields_for_provider(&provider_id);
            }
            patch_codex_auth(
                required_target_path(targets, CODEX_AUTH_KIND)?,
                preserve_codex_official_auth,
                codex_auth_backup_content,
            )
        }
        GatewayCliKey::Grok => patch_grok_config(
            required_target_path(targets, GROK_CONFIG_KIND)?,
            &cli_gateway_endpoint(cli_key, base_origin),
        ),
        GatewayCliKey::Kimi => {
            let provider_key = patch_kimi_config(
                required_target_path(targets, KIMI_CONFIG_KIND)?,
                &cli_gateway_endpoint(cli_key, base_origin),
            )?;
            if let Some(target) = targets
                .files
                .iter_mut()
                .find(|file| file.kind == KIMI_CONFIG_KIND)
            {
                target.managed_fields = kimi_config_managed_fields_for_provider(&provider_key);
            }
            Ok(())
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
        GatewayCliKey::Claude => {
            let path = required_target_path(targets, CLAUDE_SETTINGS_KIND)?;
            if should_delete_gateway_created_file(manifest, CLAUDE_SETTINGS_KIND) {
                return delete_if_exists(path);
            }
            restore_claude_settings(
                path,
                backup_content(paths, cli_key, manifest, CLAUDE_SETTINGS_KIND)?.as_deref(),
            )
        }
        GatewayCliKey::ClaudeDesktop => {
            let desktop = claude_desktop_config_writer::current_platform_paths()
                .map_err(|error| format!("Claude Desktop: {error}"))?;
            claude_desktop_config_writer::restore_official(&desktop)
        }
        GatewayCliKey::Codex => {
            let config_path = required_target_path(targets, CODEX_CONFIG_KIND)?;
            if should_delete_gateway_created_file(manifest, CODEX_CONFIG_KIND) {
                delete_if_exists(config_path)?;
            } else {
                restore_codex_config(
                    config_path,
                    backup_content(paths, cli_key, manifest, CODEX_CONFIG_KIND)?.as_deref(),
                )?;
            }
            let auth_path = required_target_path(targets, CODEX_AUTH_KIND)?;
            if should_delete_gateway_created_file(manifest, CODEX_AUTH_KIND) {
                delete_if_exists(auth_path)
            } else {
                restore_codex_auth(
                    auth_path,
                    backup_content(paths, cli_key, manifest, CODEX_AUTH_KIND)?.as_deref(),
                )
            }
        }
        GatewayCliKey::Grok => {
            let path = required_target_path(targets, GROK_CONFIG_KIND)?;
            if should_delete_gateway_created_file(manifest, GROK_CONFIG_KIND) {
                return delete_if_exists(path);
            }
            restore_grok_config(
                path,
                backup_content(paths, cli_key, manifest, GROK_CONFIG_KIND)?.as_deref(),
            )
        }
        GatewayCliKey::Kimi => {
            let path = required_target_path(targets, KIMI_CONFIG_KIND)?;
            if should_delete_gateway_created_file(manifest, KIMI_CONFIG_KIND) {
                return delete_if_exists(path);
            }
            restore_kimi_config(
                path,
                backup_content(paths, cli_key, manifest, KIMI_CONFIG_KIND)?.as_deref(),
                manifest
                    .files
                    .iter()
                    .find(|file| file.kind == KIMI_CONFIG_KIND)
                    .map(|file| file.managed_fields.as_slice())
                    .unwrap_or_default(),
            )
        }
        GatewayCliKey::Gemini => {
            let env_path = required_target_path(targets, GEMINI_ENV_KIND)?;
            if should_delete_gateway_created_file(manifest, GEMINI_ENV_KIND) {
                delete_if_exists(env_path)?;
            } else {
                restore_gemini_env(
                    env_path,
                    backup_content(paths, cli_key, manifest, GEMINI_ENV_KIND)?.as_deref(),
                )?;
            }
            let settings_path = required_target_path(targets, GEMINI_SETTINGS_KIND)?;
            if should_delete_gateway_created_file(manifest, GEMINI_SETTINGS_KIND) {
                delete_if_exists(settings_path)
            } else {
                restore_gemini_settings(
                    settings_path,
                    backup_content(paths, cli_key, manifest, GEMINI_SETTINGS_KIND)?.as_deref(),
                )
            }
        }
        GatewayCliKey::OpenCode => {
            Err("OpenCode adapter is intentionally out of scope".to_string())
        }
    }
}

fn should_delete_gateway_created_file(manifest: &CliProxyManifest, kind: &str) -> bool {
    manifest
        .files
        .iter()
        .find(|file| file.kind == kind)
        .is_some_and(|file| !file.existed)
}

fn delete_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path).map_err(|error| {
            format!(
                "Failed to delete gateway-created config {}: {}",
                path.display(),
                error
            )
        })?;
    }
    Ok(())
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
        GatewayCliKey::ClaudeDesktop => current_claude_desktop_gateway_endpoint(
            required_target_path(targets, DESKTOP_PROFILE_KIND)?,
        ),
        GatewayCliKey::Codex => {
            current_codex_gateway_endpoint(required_target_path(targets, CODEX_CONFIG_KIND)?)
        }
        GatewayCliKey::Grok => {
            current_grok_gateway_endpoint(required_target_path(targets, GROK_CONFIG_KIND)?)
        }
        GatewayCliKey::Kimi => {
            current_kimi_gateway_endpoint(required_target_path(targets, KIMI_CONFIG_KIND)?)
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
        GatewayCliKey::ClaudeDesktop => format!("{base_origin}/claude-desktop"),
        GatewayCliKey::Codex => format!("{base_origin}/openai/v1"),
        GatewayCliKey::Grok => format!("{base_origin}/grok/v1"),
        GatewayCliKey::Kimi => format!("{base_origin}/kimi/v1"),
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

/// Read the Claude Desktop 3P profile's `inferenceGatewayBaseUrl` (the gateway
/// endpoint Claude Desktop is currently routed through).
fn current_claude_desktop_gateway_endpoint(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let value = read_json_file(path)?;
    Ok(value
        .get("inferenceGatewayBaseUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

/// Read the primary Claude Desktop provider's model routes and turn them into
/// gateway-profile `inferenceModels` specs so the app menu shows the mapped models.
fn desktop_gateway_model_specs(
    db: &SqliteDbState,
    provider_id: &str,
) -> Result<Vec<claude_desktop_config_writer::GatewayModelSpec>, String> {
    let raw = db.with_conn(|conn| db_get(conn, DbTable::ClaudeDesktopProvider, provider_id))?;
    let Some(raw) = raw else {
        log::debug!("[desktop-gateway] no provider row found for id {provider_id}");
        return Ok(Vec::new());
    };
    let provider = crate::coding::claude_desktop::adapter::from_db_value_provider(raw);
    log::debug!(
        "[desktop-gateway] reading model specs for provider '{}' (category={}, applied={})",
        provider.name,
        provider.category,
        provider.is_applied
    );
    let settings_config = serde_json::from_str::<Value>(&provider.settings_config).ok();
    Ok(claude_desktop_config_writer::desktop_proxy_model_specs(
        provider.meta.as_ref(),
        settings_config.as_ref(),
    ))
}

/// Rewrite the Claude Desktop 3P files so the app is routed through the local
/// gateway. Writes deploymentMode 3p, a gateway profile pointing at
/// `gateway_endpoint` with a sentinel api key, applies the profile id, and
/// surfaces the provider's mapped models in `inferenceModels`.
fn apply_desktop_gateway_config(
    _targets: &CliProxyTargets,
    gateway_endpoint: &str,
    model_specs: Vec<claude_desktop_config_writer::GatewayModelSpec>,
) -> Result<(), String> {
    log::debug!(
        "[desktop-gateway] writing gateway profile with {} model specs",
        model_specs.len()
    );
    let desktop = claude_desktop_config_writer::current_platform_paths()
        .map_err(|error| format!("Claude Desktop: {error}"))?;
    let model_specs = (!model_specs.is_empty()).then_some(model_specs.as_slice());
    claude_desktop_config_writer::apply_gateway_proxy_profile(
        &desktop,
        gateway_endpoint,
        DESKTOP_GATEWAY_API_KEY,
        model_specs,
    )
}

fn active_codex_model_provider_id(document: &DocumentMut) -> Option<String> {
    document
        .as_table()
        .get("model_provider")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Active model_provider for takeover. Defaults to `custom` when unset.
/// Legacy `ai-toolbox-gateway` is not treated as the session bucket — callers
/// migrate off it during patch.
fn resolve_codex_takeover_provider_id(document: &DocumentMut) -> String {
    match active_codex_model_provider_id(document) {
        Some(id) if id == GATEWAY_PROVIDER_ID => DEFAULT_CODEX_PROVIDER_ID.to_string(),
        Some(id) => id,
        None => DEFAULT_CODEX_PROVIDER_ID.to_string(),
    }
}

/// Active Kimi provider table key for takeover, resolved through the CLI's own
/// lookup chain: `default_model` -> `[models.<key>].provider`. Custom applied
/// providers project their own key (e.g. `axonhub`), so patching only
/// `managed:kimi-code` would leave CLI traffic bypassing the gateway. Falls
/// back to the official managed provider when the chain is incomplete.
fn resolve_kimi_takeover_provider_key(document: &DocumentMut) -> String {
    document
        .as_table()
        .get("default_model")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|model_key| {
            document
                .as_table()
                .get("models")
                .and_then(Item::as_table_like)
                .and_then(|models| models.get(model_key))
                .and_then(Item::as_table_like)
                .and_then(|model| model.get("provider"))
                .and_then(Item::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| DEFAULT_KIMI_PROVIDER_KEY.to_string())
}

fn codex_provider_base_url(document: &DocumentMut, provider_id: &str) -> Option<String> {
    document
        .as_table()
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(provider_id))
        .and_then(Item::as_table)
        .and_then(|provider| provider.get("base_url"))
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn codex_provider_exists(document: &DocumentMut, provider_id: &str) -> bool {
    document
        .as_table()
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(provider_id))
        .is_some()
}

fn restorable_codex_model_provider_id(backup_document: &DocumentMut) -> Option<String> {
    let provider_id = active_codex_model_provider_id(backup_document)?;
    if provider_id != GATEWAY_PROVIDER_ID {
        return Some(provider_id);
    }

    if codex_provider_exists(backup_document, DEFAULT_CODEX_PROVIDER_ID) {
        Some(DEFAULT_CODEX_PROVIDER_ID.to_string())
    } else {
        None
    }
}

fn current_codex_gateway_endpoint(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let document = parse_toml_file(path)?;
    // Prefer active provider table (new strategy). Fall back to legacy sentinel.
    if let Some(active_id) = active_codex_model_provider_id(&document) {
        if let Some(base_url) = codex_provider_base_url(&document, &active_id) {
            return Ok(Some(base_url));
        }
    }
    Ok(codex_provider_base_url(&document, GATEWAY_PROVIDER_ID))
}

fn current_grok_gateway_endpoint(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let document = parse_toml_file(path)?;
    let selected_model = document
        .as_table()
        .get("models")
        .and_then(Item::as_table)
        .and_then(|models| models.get("default"))
        .and_then(Item::as_str);
    if selected_model != Some(GATEWAY_PROVIDER_ID) {
        return Ok(None);
    }
    Ok(document
        .as_table()
        .get("model")
        .and_then(Item::as_table)
        .and_then(|models| models.get(GATEWAY_PROVIDER_ID))
        .and_then(Item::as_table_like)
        .and_then(|model| model.get("base_url"))
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn current_kimi_gateway_endpoint(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let document = parse_toml_file(path)?;
    let provider_key = resolve_kimi_takeover_provider_key(&document);
    let provider_table = document
        .as_table()
        .get("providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(&provider_key))
        .and_then(Item::as_table_like);
    if provider_table
        .and_then(|provider| provider.get("api_key"))
        .and_then(Item::as_str)
        .map(str::trim)
        != Some(GATEWAY_API_KEY)
    {
        return Ok(None);
    }
    Ok(provider_table
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
    let mut changed = false;

    // New strategy: rewrite active provider base_url when it matches Windows gateway.
    let active_ids: Vec<String> = match active_codex_model_provider_id(&document) {
        Some(id) => vec![id],
        None => Vec::new(),
    };
    // Also rewrite legacy sentinel table if present.
    let mut candidate_ids = active_ids;
    if !candidate_ids.iter().any(|id| id == GATEWAY_PROVIDER_ID) {
        candidate_ids.push(GATEWAY_PROVIDER_ID.to_string());
    }

    for provider_id in candidate_ids {
        let Some(provider_table) = document
            .as_table_mut()
            .get_mut("model_providers")
            .and_then(Item::as_table_mut)
            .and_then(|providers| providers.get_mut(provider_id.as_str()))
            .and_then(Item::as_table_mut)
        else {
            continue;
        };
        if provider_table
            .get("base_url")
            .and_then(Item::as_str)
            .map(str::trim)
            != Some(windows_gateway_endpoint)
        {
            continue;
        }
        provider_table["base_url"] = value(wsl_gateway_endpoint);
        changed = true;
    }

    if changed {
        Ok(Some(document.to_string()))
    } else {
        Ok(None)
    }
}

fn rewrite_grok_wsl_gateway_content(
    content: &str,
    windows_gateway_endpoint: &str,
    wsl_gateway_endpoint: &str,
) -> Result<Option<String>, String> {
    let mut document = parse_toml_document(content, "WSL Grok config")?;
    let selected_model = document
        .as_table()
        .get("models")
        .and_then(Item::as_table)
        .and_then(|models| models.get("default"))
        .and_then(Item::as_str);
    if selected_model != Some(GATEWAY_PROVIDER_ID) {
        return Ok(None);
    }
    let Some(model_table) = document
        .as_table_mut()
        .get_mut("model")
        .and_then(Item::as_table_mut)
        .and_then(|models| models.get_mut(GATEWAY_PROVIDER_ID))
        .and_then(Item::as_table_like_mut)
    else {
        return Ok(None);
    };
    if model_table
        .get("base_url")
        .and_then(Item::as_str)
        .map(str::trim)
        != Some(windows_gateway_endpoint)
    {
        return Ok(None);
    }
    model_table.insert("base_url", value(wsl_gateway_endpoint));
    Ok(Some(document.to_string()))
}

fn rewrite_kimi_wsl_gateway_content(
    content: &str,
    windows_gateway_endpoint: &str,
    wsl_gateway_endpoint: &str,
) -> Result<Option<String>, String> {
    let mut document = parse_toml_document(content, "WSL Kimi config")?;
    let provider_key = resolve_kimi_takeover_provider_key(&document);
    let Some(provider_table) = document
        .as_table_mut()
        .get_mut("providers")
        .and_then(Item::as_table_mut)
        .and_then(|providers| providers.get_mut(&provider_key))
        .and_then(Item::as_table_like_mut)
    else {
        return Ok(None);
    };
    if provider_table
        .get("api_key")
        .and_then(Item::as_str)
        .map(str::trim)
        != Some(GATEWAY_API_KEY)
    {
        return Ok(None);
    }
    if provider_table
        .get("base_url")
        .and_then(Item::as_str)
        .map(str::trim)
        != Some(windows_gateway_endpoint)
    {
        return Ok(None);
    }
    provider_table.insert("base_url", value(wsl_gateway_endpoint));
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
        env.insert(
            "ANTHROPIC_DEFAULT_FABLE_MODEL".to_string(),
            Value::String(CLAUDE_STANDARD_FABLE_MODEL.to_string()),
        );
        env.remove("ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME");
        env.remove("ANTHROPIC_DEFAULT_SONNET_MODEL_NAME");
        env.remove("ANTHROPIC_DEFAULT_OPUS_MODEL_NAME");
        env.remove("ANTHROPIC_DEFAULT_FABLE_MODEL_NAME");
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
        if let Some(model_name) = provider_model_name(
            primary_provider
                .model_mapping
                .fable_model
                .as_deref()
                .or(primary_provider.model_mapping.opus_model.as_deref()),
            primary_provider.model_mapping.default_model.as_deref(),
        ) {
            env.insert(
                "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME".to_string(),
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
            "/env/ANTHROPIC_DEFAULT_FABLE_MODEL",
            "/env/ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
            "/env/ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
            "/env/ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
            "/env/ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
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
) -> Result<String, String> {
    let mut document = read_or_new_toml_document(path)?;
    let provider_id = resolve_codex_takeover_provider_id(&document);

    // Keep model_provider on the user's active id (default custom) so session
    // history buckets stay continuous. Do not introduce ai-toolbox-gateway.
    document["model_provider"] = value(provider_id.as_str());

    if document.get("model_providers").is_none() {
        let mut parent = toml_edit::Table::new();
        parent.set_implicit(true);
        document["model_providers"] = Item::Table(parent);
    }

    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or_else(|| "Codex [model_providers] must be a table".to_string())?;

    // Drop legacy sentinel so config no longer shows two providers.
    providers.remove(GATEWAY_PROVIDER_ID);

    if !providers.contains_key(provider_id.as_str()) {
        providers.insert(provider_id.as_str(), Item::Table(toml_edit::Table::new()));
    }
    let provider_table = providers
        .get_mut(provider_id.as_str())
        .and_then(Item::as_table_mut)
        .ok_or_else(|| format!("Codex [model_providers.{provider_id}] must be a table"))?;

    provider_table["base_url"] = value(gateway_endpoint);
    provider_table["wire_api"] = value("responses");
    if provider_table.get("requires_openai_auth").is_none() {
        provider_table["requires_openai_auth"] = value(true);
    }
    if preserve_official_auth {
        provider_table["experimental_bearer_token"] = value(GATEWAY_API_KEY);
    } else {
        provider_table.remove("experimental_bearer_token");
    }

    write_toml_file(path, &document)?;
    Ok(provider_id)
}

fn remove_codex_gateway_managed_provider(
    document: &mut DocumentMut,
    provider_id: &str,
    backup_provider_keys: &BTreeSet<String>,
) {
    if provider_id == GATEWAY_PROVIDER_ID || backup_provider_keys.contains(provider_id) {
        return;
    }

    let Some(providers_table) = document
        .as_table_mut()
        .get_mut("model_providers")
        .and_then(Item::as_table_mut)
    else {
        return;
    };

    let should_remove_provider = match providers_table
        .get_mut(provider_id)
        .and_then(Item::as_table_mut)
    {
        Some(provider_table) => {
            provider_table.remove("base_url");
            provider_table.remove("wire_api");
            provider_table.remove("experimental_bearer_token");
            provider_table.remove("requires_openai_auth");
            provider_table.is_empty()
        }
        None => false,
    };
    if should_remove_provider {
        providers_table.remove(provider_id);
    }
}

fn patch_grok_config(path: &Path, gateway_endpoint: &str) -> Result<(), String> {
    let mut document = read_or_new_toml_document(path)?;
    document["models"]["default"] = value(GATEWAY_PROVIDER_ID);
    let model_root = document["model"].or_insert(Item::Table(toml_edit::Table::new()));
    let model_root = model_root
        .as_table_mut()
        .ok_or_else(|| "Grok [model] must be a table".to_string())?;
    let mut gateway_model = toml_edit::Table::new();
    gateway_model["model"] = value("grok-build");
    gateway_model["name"] = value("AI Toolbox Gateway");
    gateway_model["base_url"] = value(gateway_endpoint);
    gateway_model["api_key"] = value(GATEWAY_API_KEY);
    gateway_model["api_backend"] = value("responses");
    model_root.insert(GATEWAY_PROVIDER_ID, Item::Table(gateway_model));
    write_toml_file(path, &document)
}

fn patch_kimi_config(path: &Path, gateway_endpoint: &str) -> Result<String, String> {
    let mut document = read_or_new_toml_document(path)?;
    let provider_key = resolve_kimi_takeover_provider_key(&document);
    let providers_root = document["providers"].or_insert(Item::Table(toml_edit::Table::new()));
    let providers_root = providers_root
        .as_table_mut()
        .ok_or_else(|| "Kimi [providers] must be a table".to_string())?;
    let managed_provider = providers_root
        .entry(&provider_key)
        .or_insert(Item::Table(toml_edit::Table::new()))
        .as_table_like_mut()
        .ok_or_else(|| "Kimi active provider entry must be a table".to_string())?;
    managed_provider.insert("type", value("openai"));
    managed_provider.insert("base_url", value(gateway_endpoint));
    managed_provider.insert("api_key", value(GATEWAY_API_KEY));
    write_toml_file(path, &document)?;
    Ok(provider_key)
}

fn restore_grok_config(path: &Path, backup_content: Option<&str>) -> Result<(), String> {
    let mut current = read_or_new_toml_document(path)?;
    let backup = backup_content
        .map(|content| parse_toml_document(content, "Grok gateway backup"))
        .transpose()?;

    if current
        .as_table()
        .get("models")
        .and_then(Item::as_table)
        .and_then(|models| models.get("default"))
        .and_then(Item::as_str)
        == Some(GATEWAY_PROVIDER_ID)
    {
        if let Some(models) = current
            .as_table_mut()
            .get_mut("models")
            .and_then(Item::as_table_mut)
        {
            models.remove("default");
        }
    }
    if let Some(models) = current
        .as_table_mut()
        .get_mut("model")
        .and_then(Item::as_table_mut)
    {
        models.remove(GATEWAY_PROVIDER_ID);
    }

    if let Some(backup_document) = backup.as_ref() {
        if let Some(default_model) = backup_document
            .as_table()
            .get("models")
            .and_then(Item::as_table)
            .and_then(|models| models.get("default"))
            .cloned()
        {
            current["models"]["default"] = default_model;
        }
        if let Some(gateway_model) = backup_document
            .as_table()
            .get("model")
            .and_then(Item::as_table)
            .and_then(|models| models.get(GATEWAY_PROVIDER_ID))
            .cloned()
        {
            current["model"][GATEWAY_PROVIDER_ID] = gateway_model;
        }
    }

    remove_empty_toml_table(&mut current, "models");
    remove_empty_toml_table(&mut current, "model");
    write_toml_file(path, &current)
}

fn restore_codex_config(path: &Path, backup_content: Option<&str>) -> Result<(), String> {
    let mut current = read_or_new_toml_document(path)?;
    let backup = backup_content
        .map(|content| parse_toml_document(content, "Codex gateway backup"))
        .transpose()?;
    let managed_provider_id = resolve_codex_takeover_provider_id(&current);
    let restored_model_provider = backup.as_ref().and_then(restorable_codex_model_provider_id);
    let backup_provider_items: Vec<(String, Item)> = backup
        .as_ref()
        .and_then(|backup_document| {
            backup_document
                .as_table()
                .get("model_providers")
                .and_then(Item::as_table)
        })
        .map(|backup_providers| {
            backup_providers
                .iter()
                .filter(|(provider_key, _)| *provider_key != GATEWAY_PROVIDER_ID)
                .map(|(provider_key, provider_item)| {
                    (provider_key.to_string(), provider_item.clone())
                })
                .collect()
        })
        .unwrap_or_default();
    let backup_provider_keys: BTreeSet<String> = backup_provider_items
        .iter()
        .map(|(provider_key, _)| provider_key.clone())
        .collect();

    // Always strip legacy sentinel.
    if let Some(providers_table) = current
        .as_table_mut()
        .get_mut("model_providers")
        .and_then(Item::as_table_mut)
    {
        providers_table.remove(GATEWAY_PROVIDER_ID);
    }
    if current
        .as_table()
        .get("model_provider")
        .and_then(Item::as_str)
        == Some(GATEWAY_PROVIDER_ID)
    {
        current.as_table_mut().remove("model_provider");
    }

    match restored_model_provider {
        Some(model_provider) => {
            current["model_provider"] = value(model_provider);
        }
        None => {
            current.as_table_mut().remove("model_provider");
        }
    }

    remove_codex_gateway_managed_provider(
        &mut current,
        &managed_provider_id,
        &backup_provider_keys,
    );

    // Restore each backup provider table (includes pre-takeover base_url).
    for (provider_key, provider_item) in backup_provider_items {
        current["model_providers"][&provider_key] = provider_item;
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

/// Managed Kimi provider fields. Takeover only rewrites these three fields on
/// the active provider table, so restore must also be field-level: a whole-file
/// rollback would revert unrelated edits made during the takeover window.
const KIMI_MANAGED_PROVIDER_FIELDS: [&str; 3] = ["type", "base_url", "api_key"];

/// Provider table keys recorded by a manifest's managed fields
/// (`providers.<key>.type` / `.base_url` / `.api_key`).
fn kimi_managed_provider_keys_from_fields(managed_fields: &[String]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for field in managed_fields {
        let Some(rest) = field.strip_prefix("providers.") else {
            continue;
        };
        let Some(stripped) = ["type", "base_url", "api_key"]
            .iter()
            .find_map(|suffix| rest.strip_suffix(suffix))
        else {
            continue;
        };
        // Drop the separator dot left behind by the suffix strip.
        let key = stripped.strip_suffix('.').unwrap_or(stripped);
        if !key.is_empty() && !keys.iter().any(|existing| existing == key) {
            keys.push(key.to_string());
        }
    }
    keys
}

fn restore_kimi_config(
    path: &Path,
    backup_content: Option<&str>,
    managed_fields: &[String],
) -> Result<(), String> {
    let mut current = read_or_new_toml_document(path)?;
    let backup = backup_content
        .map(|content| parse_toml_document(content, "Kimi gateway backup"))
        .transpose()?;

    // Prefer the provider keys recorded at patch time (manifest managed
    // fields); fall back to the CLI's own lookup chain for legacy manifests.
    let mut provider_keys = kimi_managed_provider_keys_from_fields(managed_fields);
    let resolved_key = resolve_kimi_takeover_provider_key(&current);
    // Also always restore the key the live file itself resolves as managed: a
    // crash between apply and the manifest re-sync leaves the manifest pointing
    // at the placeholder key while the real provider table was patched.
    // Placeholder-key restore is a harmless no-op; this one covers the drift.
    if !provider_keys.iter().any(|key| key == &resolved_key) {
        provider_keys.push(resolved_key);
    }

    for provider_key in provider_keys {
        // 1. Clear the managed fields from the active provider table.
        let mut table_left_empty = false;
        if let Some(provider_table) = current
            .as_table_mut()
            .get_mut("providers")
            .and_then(Item::as_table_mut)
            .and_then(|providers| providers.get_mut(&provider_key))
            .and_then(Item::as_table_like_mut)
        {
            for field in KIMI_MANAGED_PROVIDER_FIELDS {
                provider_table.remove(field);
            }
            table_left_empty = provider_table.is_empty();
        }
        // 2. Restore the managed fields from the backup.
        let backup_table = backup
            .as_ref()
            .and_then(|document| document.as_table().get("providers"))
            .and_then(Item::as_table_like)
            .and_then(|providers| providers.get(&provider_key))
            .and_then(Item::as_table_like);
        match backup_table {
            Some(backup_table) => {
                // Defensive mirror of `patch_kimi_config`'s table access: a
                // user-corrupted non-table shape must fail cleanly instead of
                // panicking the restore path — restore is the last-resort net
                // that brings the CLI back to direct connectivity.
                let providers_root =
                    current["providers"].or_insert(Item::Table(toml_edit::Table::new()));
                let providers_root = providers_root
                    .as_table_mut()
                    .ok_or_else(|| "Kimi [providers] must be a table".to_string())?;
                let provider_table = providers_root
                    .entry(provider_key.as_str())
                    .or_insert(Item::Table(toml_edit::Table::new()))
                    .as_table_like_mut()
                    .ok_or_else(|| "Kimi active provider entry must be a table".to_string())?;
                for field in KIMI_MANAGED_PROVIDER_FIELDS {
                    if let Some(backup_value) = backup_table.get(field) {
                        provider_table.insert(field, backup_value.clone());
                    }
                }
            }
            // A missing backup table with no leftover fields means the patch
            // created the provider table itself: drop it instead of leaving a
            // hollow entry behind.
            None if table_left_empty => {
                if let Some(providers) = current
                    .as_table_mut()
                    .get_mut("providers")
                    .and_then(Item::as_table_mut)
                {
                    providers.remove(&provider_key);
                }
            }
            None => {}
        }
    }
    // Drop an empty [providers] root that only existed for the gateway entry.
    if current
        .as_table()
        .get("providers")
        .and_then(Item::as_table_like)
        .is_some_and(|providers| providers.is_empty())
    {
        current.as_table_mut().remove("providers");
    }
    write_toml_file(path, &current)
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

    #[test]
    fn data_directory_switch_requires_all_cli_takeovers_to_be_restored() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(temporary.path());
        assert!(ensure_data_dir_can_change(&paths).is_ok());
        for cli_key in GatewayCliKey::supported_mvp() {
            let mut manifest = CliProxyManifest::new(
                cli_key,
                "http://127.0.0.1:8080".into(),
                "test".into(),
                GatewayProxyMode::Single,
                "provider".into(),
            );
            write_manifest(&paths, cli_key, &manifest).unwrap();
            assert!(ensure_data_dir_can_change(&paths)
                .unwrap_err()
                .contains(cli_key.as_str()));
            manifest.enabled = false;
            write_manifest(&paths, cli_key, &manifest).unwrap();
            assert!(ensure_data_dir_can_change(&paths).is_ok());
        }
        fs::write(paths.manifest_path(GatewayCliKey::Codex), "{broken").unwrap();
        assert!(ensure_data_dir_can_change(&paths).is_err());
    }
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
                auto_review_model: None,
                haiku_model: haiku_model.map(str::to_string),
                sonnet_model: sonnet_model.map(str::to_string),
                opus_model: opus_model.map(str::to_string),
                fable_model: None,
                reasoning_model: None,
                rewrite_rules: Vec::new(),
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
        let mut primary_provider = claude_test_provider(
            Some("provider-default"),
            Some("provider-haiku"),
            Some("provider-sonnet"),
            Some("provider-opus"),
        );
        primary_provider.model_mapping.fable_model = Some("provider-fable".to_string());

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
        assert_eq!(
            patched
                .pointer("/env/ANTHROPIC_DEFAULT_FABLE_MODEL")
                .and_then(Value::as_str),
            Some(CLAUDE_STANDARD_FABLE_MODEL)
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
                .pointer("/env/ANTHROPIC_DEFAULT_FABLE_MODEL_NAME")
                .and_then(Value::as_str),
            Some("provider-fable")
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
        assert!(restored
            .pointer("/env/ANTHROPIC_DEFAULT_FABLE_MODEL_NAME")
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
        assert_eq!(
            patched
                .pointer("/env/ANTHROPIC_DEFAULT_FABLE_MODEL_NAME")
                .and_then(Value::as_str),
            Some("provider-default")
        );
    }

    #[test]
    fn claude_fable_model_name_fields_fall_back_to_opus_model() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        let primary_provider = claude_test_provider(None, None, None, Some("provider-opus"));

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
                .pointer("/env/ANTHROPIC_DEFAULT_FABLE_MODEL_NAME")
                .and_then(Value::as_str),
            Some("provider-opus")
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
                    "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME": "stale-opus",
                    "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME": "stale-fable"
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
        assert!(patched
            .pointer("/env/ANTHROPIC_DEFAULT_FABLE_MODEL_NAME")
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
                    "ANTHROPIC_DEFAULT_FABLE_MODEL": CLAUDE_STANDARD_FABLE_MODEL,
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME": "provider-haiku",
                    "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "provider-sonnet",
                    "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME": "provider-opus",
                    "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME": "provider-fable"
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
    fn codex_takeover_rewrites_active_provider_base_url_without_second_provider() {
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

        let provider_id =
            patch_codex_config(&config_path, "http://127.0.0.1:37123/openai/v1", false).unwrap();
        assert_eq!(provider_id, "custom");
        let patched = parse_toml_file(&config_path).unwrap();
        assert_eq!(patched["model_provider"].as_str(), Some("custom"));
        assert_eq!(
            patched["model_providers"]["custom"]["base_url"].as_str(),
            Some("http://127.0.0.1:37123/openai/v1")
        );
        assert_eq!(
            patched["model_providers"]["custom"]["wire_api"].as_str(),
            Some("responses")
        );
        assert_eq!(
            patched["model_providers"]["custom"]["name"].as_str(),
            Some("Custom")
        );
        assert!(patched["model_providers"]
            .get(GATEWAY_PROVIDER_ID)
            .is_none());
        assert!(patched["model_providers"]["custom"]
            .as_table_like()
            .expect("custom provider table")
            .get("experimental_bearer_token")
            .is_none());
        assert_eq!(
            patched["mcp_servers"]["keep"]["command"].as_str(),
            Some("node")
        );

        restore_codex_config(&config_path, Some(&backup)).unwrap();
        let restored = parse_toml_file(&config_path).unwrap();
        assert_eq!(restored["model_provider"].as_str(), Some("custom"));
        assert_eq!(
            restored["model_providers"]["custom"]["base_url"].as_str(),
            Some("https://old.example.com/v1")
        );
        assert!(restored["model_providers"]
            .get(GATEWAY_PROVIDER_ID)
            .is_none());
        assert_eq!(
            restored["mcp_servers"]["keep"]["command"].as_str(),
            Some("node")
        );
    }

    #[test]
    fn codex_restore_removes_provider_created_when_backup_lacked_provider_table() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        write_text_file(
            &config_path,
            r#"
[mcp_servers.keep]
command = "node"
"#,
        )
        .unwrap();
        let backup = fs::read_to_string(&config_path).unwrap();

        let provider_id =
            patch_codex_config(&config_path, "http://127.0.0.1:37123/openai/v1", false).unwrap();
        assert_eq!(provider_id, "custom");
        let patched = parse_toml_file(&config_path).unwrap();
        assert_eq!(patched["model_provider"].as_str(), Some("custom"));
        assert_eq!(
            patched["model_providers"]["custom"]["base_url"].as_str(),
            Some("http://127.0.0.1:37123/openai/v1")
        );

        restore_codex_config(&config_path, Some(&backup)).unwrap();
        let restored = parse_toml_file(&config_path).unwrap();
        assert!(restored.as_table().get("model_provider").is_none());
        assert!(restored.as_table().get("model_providers").is_none());
        assert_eq!(
            restored["mcp_servers"]["keep"]["command"].as_str(),
            Some("node")
        );
    }

    #[test]
    fn codex_restore_without_backup_removes_created_gateway_provider() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");

        let provider_id =
            patch_codex_config(&config_path, "http://127.0.0.1:37123/openai/v1", false).unwrap();
        assert_eq!(provider_id, "custom");

        restore_codex_config(&config_path, None).unwrap();
        let restored = parse_toml_file(&config_path).unwrap();
        assert!(restored.as_table().get("model_provider").is_none());
        assert!(restored.as_table().get("model_providers").is_none());
    }

    #[test]
    fn codex_takeover_clears_legacy_gateway_provider_table() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        write_text_file(
            &config_path,
            r#"
model_provider = "ai-toolbox-gateway"

[model_providers.custom]
name = "Custom"
base_url = "https://old.example.com/v1"

[model_providers.ai-toolbox-gateway]
name = "AI Toolbox Gateway"
base_url = "http://127.0.0.1:9999/openai/v1"
"#,
        )
        .unwrap();
        let backup = fs::read_to_string(&config_path).unwrap();

        let provider_id =
            patch_codex_config(&config_path, "http://127.0.0.1:37123/openai/v1", false).unwrap();
        assert_eq!(provider_id, "custom");
        let patched = parse_toml_file(&config_path).unwrap();
        assert_eq!(patched["model_provider"].as_str(), Some("custom"));
        assert!(patched["model_providers"]
            .get(GATEWAY_PROVIDER_ID)
            .is_none());
        assert_eq!(
            patched["model_providers"]["custom"]["base_url"].as_str(),
            Some("http://127.0.0.1:37123/openai/v1")
        );

        restore_codex_config(&config_path, Some(&backup)).unwrap();
        let restored = parse_toml_file(&config_path).unwrap();
        assert_eq!(restored["model_provider"].as_str(), Some("custom"));
        assert!(restored["model_providers"]
            .get(GATEWAY_PROVIDER_ID)
            .is_none());
        assert_eq!(
            restored["model_providers"]["custom"]["base_url"].as_str(),
            Some("https://old.example.com/v1")
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

        let provider_id =
            patch_codex_config(&config_path, "http://127.0.0.1:37123/openai/v1", true).unwrap();
        assert_eq!(provider_id, "custom");
        patch_codex_auth(&auth_path, true, None).unwrap();

        let patched_config = parse_toml_file(&config_path).unwrap();
        assert_eq!(patched_config["model_provider"].as_str(), Some("custom"));
        assert_eq!(
            patched_config["model_providers"]["custom"]["experimental_bearer_token"].as_str(),
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
        let mut targets = CliProxyTargets {
            runtime_root: dir.path().join("runtime"),
            is_wsl_direct: false,
            files: vec![CliProxyTarget {
                kind: CLAUDE_SETTINGS_KIND,
                path: settings_path.clone(),
                managed_fields: static_managed_fields(&CLAUDE_MANAGED_FIELDS),
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
        let db = crate::db::SqliteDbState::in_memory_for_test().unwrap();
        apply_gateway_config(
            &db,
            GatewayCliKey::Claude,
            &mut targets,
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
    fn wsl_gateway_rewrite_updates_active_codex_provider_base_url_only() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let managed = codex_config_managed_fields_for_provider("custom");
        let managed_refs: Vec<&str> = managed.iter().map(String::as_str).collect();
        let manifest =
            test_manifest_with_file(GatewayCliKey::Codex, CODEX_CONFIG_KIND, &managed_refs);
        write_manifest(&paths, GatewayCliKey::Codex, &manifest).unwrap();

        let content = r#"
model_provider = "custom"

[model_providers.custom]
name = "Custom"
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
            rewritten_document["model_providers"]["custom"]["base_url"].as_str(),
            Some("http://172.20.10.1:37123/openai/v1")
        );
        assert_eq!(
            rewritten_document["model_providers"]["local"]["base_url"].as_str(),
            Some("http://127.0.0.1:9999/v1")
        );
    }

    #[test]
    fn resolve_kimi_takeover_provider_key_follows_default_model_provider() {
        let document = parse_toml_document(
            r#"
default_model = "axonhub/k2"

[models."axonhub/k2"]
provider = "axonhub"
model = "k2"
"#,
            "Kimi config",
        )
        .unwrap();
        assert_eq!(resolve_kimi_takeover_provider_key(&document), "axonhub");
    }

    #[test]
    fn resolve_kimi_takeover_provider_key_accepts_inline_models_table() {
        // Inline `models = { ... }` tables must resolve like header tables.
        let document = parse_toml_document(
            "default_model = \"axonhub/k2\"\nmodels = { \"axonhub/k2\" = { provider = \"axonhub\" } }\n",
            "Kimi config",
        )
        .unwrap();
        assert_eq!(resolve_kimi_takeover_provider_key(&document), "axonhub");
    }

    #[test]
    fn resolve_kimi_takeover_provider_key_falls_back_to_managed_provider() {
        let missing_default = parse_toml_document(
            "[providers.\"managed:kimi-code\"]\ntype = \"kimi\"\n",
            "Kimi config",
        )
        .unwrap();
        assert_eq!(
            resolve_kimi_takeover_provider_key(&missing_default),
            DEFAULT_KIMI_PROVIDER_KEY
        );

        let missing_models =
            parse_toml_document("default_model = \"kimi-code/k3\"\n", "Kimi config").unwrap();
        assert_eq!(
            resolve_kimi_takeover_provider_key(&missing_models),
            DEFAULT_KIMI_PROVIDER_KEY
        );

        let missing_provider_field = parse_toml_document(
            "default_model = \"kimi-code/k3\"\n\n[models.\"kimi-code/k3\"]\nmodel = \"k3\"\n",
            "Kimi config",
        )
        .unwrap();
        assert_eq!(
            resolve_kimi_takeover_provider_key(&missing_provider_field),
            DEFAULT_KIMI_PROVIDER_KEY
        );
    }

    #[test]
    fn patch_kimi_config_rewrites_active_custom_provider() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
default_model = "axonhub/k2"

[models."axonhub/k2"]
provider = "axonhub"
model = "k2"

[providers.axonhub]
type = "openai"
base_url = "https://axonhub.example.com/v1"
api_key = "real-key"

[providers.axonhub.env]
KIMI_CODE_CUSTOM_HEADERS = "x-trace: 1"

[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1"
"#,
        )
        .unwrap();

        let provider_key = patch_kimi_config(&path, "http://127.0.0.1:37123/kimi/v1").unwrap();
        assert_eq!(provider_key, "axonhub");

        let document = parse_toml_file(&path).unwrap();
        assert_eq!(
            document["providers"]["axonhub"]["base_url"].as_str(),
            Some("http://127.0.0.1:37123/kimi/v1")
        );
        assert_eq!(
            document["providers"]["axonhub"]["api_key"].as_str(),
            Some(GATEWAY_API_KEY)
        );
        assert_eq!(
            document["providers"]["axonhub"]["type"].as_str(),
            Some("openai")
        );
        // Unrelated provider tables and user fields stay untouched.
        assert_eq!(
            document["providers"]["axonhub"]["env"]["KIMI_CODE_CUSTOM_HEADERS"].as_str(),
            Some("x-trace: 1")
        );
        assert_eq!(
            document["providers"]["managed:kimi-code"]["base_url"].as_str(),
            Some("https://api.kimi.com/coding/v1")
        );
    }

    #[test]
    fn patch_kimi_config_without_active_chain_targets_managed_provider() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "[providers.\"managed:kimi-code\"]\ntype = \"kimi\"\nbase_url = \"https://api.kimi.com/coding/v1\"\n",
        )
        .unwrap();

        let provider_key = patch_kimi_config(&path, "http://127.0.0.1:37123/kimi/v1").unwrap();
        assert_eq!(provider_key, DEFAULT_KIMI_PROVIDER_KEY);

        let document = parse_toml_file(&path).unwrap();
        assert_eq!(
            document["providers"]["managed:kimi-code"]["base_url"].as_str(),
            Some("http://127.0.0.1:37123/kimi/v1")
        );
        assert_eq!(
            document["providers"]["managed:kimi-code"]["api_key"].as_str(),
            Some(GATEWAY_API_KEY)
        );
    }

    #[test]
    fn current_kimi_gateway_endpoint_follows_active_provider_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
default_model = "axonhub/k2"

[models."axonhub/k2"]
provider = "axonhub"

[providers.axonhub]
type = "openai"
base_url = "http://127.0.0.1:37123/kimi/v1"
api_key = "ai-toolbox-gateway"
"#,
        )
        .unwrap();
        assert_eq!(
            current_kimi_gateway_endpoint(&path).unwrap(),
            Some("http://127.0.0.1:37123/kimi/v1".to_string())
        );

        // Legacy false-green: managed provider holds gateway values but the
        // active model chain points at a direct custom provider.
        fs::write(
            &path,
            r#"
default_model = "axonhub/k2"

[models."axonhub/k2"]
provider = "axonhub"

[providers.axonhub]
type = "openai"
base_url = "https://axonhub.example.com/v1"
api_key = "real-key"

[providers."managed:kimi-code"]
type = "openai"
base_url = "http://127.0.0.1:37123/kimi/v1"
api_key = "ai-toolbox-gateway"
"#,
        )
        .unwrap();
        assert_eq!(current_kimi_gateway_endpoint(&path).unwrap(), None);
    }

    #[test]
    fn kimi_patch_and_restore_round_trip_via_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = r#"
default_model = "axonhub/k2"

[models."axonhub/k2"]
provider = "axonhub"
model = "k2"

[providers.axonhub]
type = "openai"
base_url = "https://axonhub.example.com/v1"
api_key = "real-key"
"#;
        fs::write(&path, original).unwrap();

        patch_kimi_config(&path, "http://127.0.0.1:37123/kimi/v1").unwrap();
        assert_ne!(fs::read_to_string(&path).unwrap(), original);

        let managed = kimi_config_managed_fields_for_provider("axonhub");
        restore_kimi_config(&path, Some(original), &managed).unwrap();
        let restored =
            parse_toml_document(&fs::read_to_string(&path).unwrap(), "restored Kimi config")
                .unwrap();
        assert_eq!(
            restored["providers"]["axonhub"]["type"].as_str(),
            Some("openai")
        );
        assert_eq!(
            restored["providers"]["axonhub"]["base_url"].as_str(),
            Some("https://axonhub.example.com/v1")
        );
        assert_eq!(
            restored["providers"]["axonhub"]["api_key"].as_str(),
            Some("real-key")
        );
        assert_eq!(restored["default_model"].as_str(), Some("axonhub/k2"));
    }

    #[test]
    fn kimi_restore_is_field_level_and_keeps_unmanaged_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = r#"
default_model = "axonhub/k2"

[models."axonhub/k2"]
provider = "axonhub"
model = "k2"

[providers.axonhub]
type = "openai"
base_url = "https://axonhub.example.com/v1"
api_key = "real-key"
"#;
        fs::write(&path, original).unwrap();
        patch_kimi_config(&path, "http://127.0.0.1:37123/kimi/v1").unwrap();

        // User edits unrelated parts of config.toml during the takeover window.
        let drifted = fs::read_to_string(&path).unwrap()
            + "\n[providers.axonhub.env]\nKIMI_CODE_CUSTOM_HEADERS = \"x-trace: 1\"\n\n[mcp_servers.docs]\ncommand = \"docs-mcp\"\n";
        fs::write(&path, drifted).unwrap();

        let managed = kimi_config_managed_fields_for_provider("axonhub");
        restore_kimi_config(&path, Some(original), &managed).unwrap();

        let restored =
            parse_toml_document(&fs::read_to_string(&path).unwrap(), "restored Kimi config")
                .unwrap();
        assert_eq!(
            restored["providers"]["axonhub"]["base_url"].as_str(),
            Some("https://axonhub.example.com/v1")
        );
        assert_eq!(
            restored["providers"]["axonhub"]["api_key"].as_str(),
            Some("real-key")
        );
        // Unmanaged edits made during the takeover window survive the restore.
        assert_eq!(
            restored["providers"]["axonhub"]["env"]["KIMI_CODE_CUSTOM_HEADERS"].as_str(),
            Some("x-trace: 1")
        );
        assert_eq!(
            restored["mcp_servers"]["docs"]["command"].as_str(),
            Some("docs-mcp")
        );
    }

    #[test]
    fn kimi_restore_without_backup_clears_managed_fields_only() {
        // Missing backup (e.g. manifest without the file entry) must not blank
        // the whole config: only the managed fields are cleared.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = r#"
default_model = "axonhub/k2"

[models."axonhub/k2"]
provider = "axonhub"
model = "k2"

[providers.axonhub]
type = "openai"
base_url = "https://axonhub.example.com/v1"
api_key = "real-key"
"#;
        fs::write(&path, original).unwrap();
        patch_kimi_config(&path, "http://127.0.0.1:37123/kimi/v1").unwrap();

        let managed = kimi_config_managed_fields_for_provider("axonhub");
        restore_kimi_config(&path, None, &managed).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("default_model"));
        assert!(content.contains("[models.\"axonhub/k2\"]"));
        let restored = parse_toml_document(&content, "restored Kimi config").unwrap();
        // The provider table only carried managed fields, so it is dropped
        // entirely instead of being left hollow or blanked by a full-file
        // overwrite.
        assert!(restored.get("providers").is_none());
    }

    #[test]
    fn kimi_restore_drops_gateway_created_provider_table() {
        // Backup has no [providers.axonhub]: the patch created the table, so
        // restore removes it once the managed fields are cleared.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = r#"
default_model = "axonhub/k2"

[models."axonhub/k2"]
provider = "axonhub"
model = "k2"
"#;
        fs::write(&path, original).unwrap();
        patch_kimi_config(&path, "http://127.0.0.1:37123/kimi/v1").unwrap();

        let managed = kimi_config_managed_fields_for_provider("axonhub");
        restore_kimi_config(&path, Some(original), &managed).unwrap();

        let restored =
            parse_toml_document(&fs::read_to_string(&path).unwrap(), "restored Kimi config")
                .unwrap();
        assert!(restored.get("providers").is_none());
        assert_eq!(restored["default_model"].as_str(), Some("axonhub/k2"));
        assert_eq!(
            restored["models"]["axonhub/k2"]["provider"].as_str(),
            Some("axonhub")
        );
    }

    #[test]
    fn wsl_gateway_rewrite_updates_active_kimi_provider_base_url_only() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let managed = kimi_config_managed_fields_for_provider("axonhub");
        let managed_refs: Vec<&str> = managed.iter().map(String::as_str).collect();
        let manifest =
            test_manifest_with_file(GatewayCliKey::Kimi, KIMI_CONFIG_KIND, &managed_refs);
        write_manifest(&paths, GatewayCliKey::Kimi, &manifest).unwrap();

        let content = r#"
default_model = "axonhub/k2"

[models."axonhub/k2"]
provider = "axonhub"
model = "k2"

[providers.axonhub]
type = "openai"
base_url = "http://127.0.0.1:37123/kimi/v1"
api_key = "ai-toolbox-gateway"

[providers.local]
type = "openai"
base_url = "http://127.0.0.1:9999/v1"
"#;

        let rewritten = rewrite_wsl_synced_gateway_target_content(
            &paths,
            &test_proxy_gateway_settings("172.20.10.1"),
            GatewayCliKey::Kimi,
            KIMI_CONFIG_KIND,
            content,
        )
        .unwrap()
        .unwrap();
        let rewritten_document = parse_toml_document(&rewritten, "rewritten Kimi config").unwrap();

        assert_eq!(
            rewritten_document["providers"]["axonhub"]["base_url"].as_str(),
            Some("http://172.20.10.1:37123/kimi/v1")
        );
        assert_eq!(
            rewritten_document["providers"]["local"]["base_url"].as_str(),
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
                managed_fields: static_managed_fields(&CLAUDE_MANAGED_FIELDS),
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
    fn successful_restore_clears_backup_so_next_engage_rebacks_current_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path().join("app-data"));
        let settings_path = dir.path().join("runtime").join("settings.json");
        write_json_file(
            &settings_path,
            &json!({"env": {"ANTHROPIC_BASE_URL": "https://original.example.com", "ANTHROPIC_API_KEY": "old-key"}}),
        )
        .unwrap();
        let targets = CliProxyTargets {
            runtime_root: dir.path().join("runtime"),
            is_wsl_direct: false,
            files: vec![CliProxyTarget {
                kind: CLAUDE_SETTINGS_KIND,
                path: settings_path.clone(),
                managed_fields: static_managed_fields(&CLAUDE_MANAGED_FIELDS),
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
        let backup_path = paths
            .backup_dir(GatewayCliKey::Claude)
            .join(format!("{CLAUDE_SETTINGS_KIND}.bak"));
        assert!(backup_path.exists(), "first engage must create .bak");

        // Successful restore must delete the snapshot and disable the manifest so later
        // engages re-backup the post-direct runtime files.
        clear_gateway_backups(&paths, GatewayCliKey::Claude, &first_manifest);
        assert!(
            !backup_path.exists(),
            "successful restore must remove .bak snapshots"
        );
        let mut restored_manifest = first_manifest;
        restored_manifest.enabled = false;
        restored_manifest.updated_at = chrono::Utc::now().to_rfc3339();
        write_manifest(&paths, GatewayCliKey::Claude, &restored_manifest).unwrap();

        // User edits runtime after direct restore, then re-engages.
        write_json_file(
            &settings_path,
            &json!({"env": {"ANTHROPIC_BASE_URL": "https://updated.example.com", "ANTHROPIC_API_KEY": "new-key"}}),
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
            Some("https://updated.example.com"),
            "re-engage after restore must back up the current post-direct runtime"
        );
        assert_eq!(
            backup_json
                .pointer("/env/ANTHROPIC_API_KEY")
                .and_then(Value::as_str),
            Some("new-key")
        );
    }

    #[test]
    fn retry_after_patch_failure_without_manifest_does_not_overwrite_original_backup() {
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
                managed_fields: static_managed_fields(&CLAUDE_MANAGED_FIELDS),
            }],
        };

        // First engage prepares backup but never lands an enabled manifest (apply failed).
        let _ = prepare_manifest(
            &paths,
            GatewayCliKey::Claude,
            "http://127.0.0.1:37123",
            &targets,
            GatewayProxyMode::Single,
            "provider-1",
        )
        .unwrap();
        write_json_file(
            &settings_path,
            &json!({"env": {"ANTHROPIC_BASE_URL": "http://127.0.0.1:37123/anthropic"}}),
        )
        .unwrap();

        let retry_manifest = prepare_manifest(
            &paths,
            GatewayCliKey::Claude,
            "http://127.0.0.1:37123",
            &targets,
            GatewayProxyMode::Single,
            "provider-1",
        )
        .unwrap();
        let backup = backup_content(
            &paths,
            GatewayCliKey::Claude,
            &retry_manifest,
            CLAUDE_SETTINGS_KIND,
        )
        .unwrap()
        .unwrap();
        let backup_json = serde_json::from_str::<Value>(&backup).unwrap();

        assert_eq!(
            backup_json
                .pointer("/env/ANTHROPIC_BASE_URL")
                .and_then(Value::as_str),
            Some("https://original.example.com"),
            "retry must keep the pre-patch original backup"
        );
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

    #[test]
    fn grok_takeover_round_trip_preserves_unknown_toml_fields() {
        let directory = tempfile::tempdir().expect("temp dir");
        let config_path = directory.path().join("config.toml");
        let backup = r#"
[models]
default = "user-model"
fallback = "fallback-model"

[model.user-model]
model = "upstream-user-model"
base_url = "https://api.example.com/v1"
api_backend = "responses"

[mcp_servers.filesystem]
command = "npx"

[custom]
keep = true
"#;
        fs::write(&config_path, backup).expect("write fixture");

        patch_grok_config(&config_path, "http://127.0.0.1:37123/grok/v1")
            .expect("patch Grok config");
        let patched = fs::read_to_string(&config_path).expect("read patched config");
        assert_eq!(
            current_grok_gateway_endpoint(&config_path).expect("read endpoint"),
            Some("http://127.0.0.1:37123/grok/v1".to_string())
        );
        assert!(patched.contains("[mcp_servers.filesystem]"));
        assert!(patched.contains("keep = true"));

        restore_grok_config(&config_path, Some(backup)).expect("restore Grok config");
        let restored = fs::read_to_string(&config_path).expect("read restored config");
        let document = restored
            .parse::<DocumentMut>()
            .expect("parse restored config");
        assert_eq!(document["models"]["default"].as_str(), Some("user-model"));
        assert_eq!(
            document["models"]["fallback"].as_str(),
            Some("fallback-model")
        );
        assert!(document["model"].get(GATEWAY_PROVIDER_ID).is_none());
        assert_eq!(document["custom"]["keep"].as_bool(), Some(true));
        assert!(document.get("mcp_servers").is_some());
    }

    #[test]
    fn grok_wsl_rewrite_changes_only_managed_gateway_model() {
        let content = r#"
[models]
default = "ai-toolbox-gateway"

[model.ai-toolbox-gateway]
model = "grok-build"
base_url = "http://127.0.0.1:37123/grok/v1"
api_key = "ai-toolbox-gateway"
api_backend = "responses"
"#;
        let rewritten = rewrite_grok_wsl_gateway_content(
            content,
            "http://127.0.0.1:37123/grok/v1",
            "http://172.20.0.1:37123/grok/v1",
        )
        .expect("rewrite")
        .expect("managed content");
        assert!(rewritten.contains("http://172.20.0.1:37123/grok/v1"));
    }

    #[test]
    fn restore_deletes_gateway_created_file_when_it_did_not_exist_before() {
        // G-03 regression guard: when the gateway created a config file that
        // did not exist before engaging, restore must delete it rather than
        // leaving an empty `{}` file behind.
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("settings.json");
        // The gateway wrote the file during engage; it did not pre-exist.
        std::fs::write(&file_path, "{\"managed\":\"by-gateway\"}").unwrap();
        let manifest = CliProxyManifest {
            schema_version: 1,
            managed_by: "ai-toolbox".to_string(),
            cli_key: GatewayCliKey::Claude,
            enabled: true,
            mode: GatewayProxyMode::Single,
            primary_provider_id: "provider-1".to_string(),
            base_origin: "http://127.0.0.1:37123".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            files: vec![CliProxyManifestFile {
                kind: CLAUDE_SETTINGS_KIND.to_string(),
                path: path_to_string(&file_path),
                existed: false,
                backup_rel_path: format!("{CLAUDE_SETTINGS_KIND}.bak"),
                backup_sha256: None,
                backup_size: None,
                managed_fields: Vec::new(),
            }],
        };

        assert!(
            should_delete_gateway_created_file(&manifest, CLAUDE_SETTINGS_KIND),
            "a file the gateway created (existed=false) must be flagged for deletion"
        );
        delete_if_exists(&file_path).unwrap();
        assert!(
            !file_path.exists(),
            "restore must delete the gateway-created file"
        );
    }

    #[test]
    fn restore_keeps_pre_existing_file_when_it_existed_before() {
        // The inverse case: a file that pre-existed (existed=true) must NOT be
        // flagged for deletion; restore restores its backup instead.
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("settings.json");
        std::fs::write(&file_path, "{\"pre\":\"existing\"}").unwrap();
        let manifest = CliProxyManifest {
            schema_version: 1,
            managed_by: "ai-toolbox".to_string(),
            cli_key: GatewayCliKey::Claude,
            enabled: true,
            mode: GatewayProxyMode::Single,
            primary_provider_id: "provider-1".to_string(),
            base_origin: "http://127.0.0.1:37123".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            files: vec![CliProxyManifestFile {
                kind: CLAUDE_SETTINGS_KIND.to_string(),
                path: path_to_string(&file_path),
                existed: true,
                backup_rel_path: format!("{CLAUDE_SETTINGS_KIND}.bak"),
                backup_sha256: None,
                backup_size: None,
                managed_fields: Vec::new(),
            }],
        };

        assert!(
            !should_delete_gateway_created_file(&manifest, CLAUDE_SETTINGS_KIND),
            "a pre-existing file (existed=true) must not be deleted on restore"
        );
        assert!(
            file_path.exists(),
            "pre-existing file must be untouched by the delete path"
        );
    }
}
