//! Pure Claude Desktop config-on-disk writing logic.
//!
//! This module owns the platform path resolution and the exact byte-level write
//! order for applying a non-official provider / restoring the official one. It is
//! intentionally free of any `tauri` / DB dependency so it stays testable and
//! focused on file semantics only.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "windows", test))]
use std::time::SystemTime;

use serde_json::{json, Map, Value};

use super::constants::{
    ANTHROPIC_CLAUDE_ROUTE_PREFIX, CLAUDE_ROUTE_PREFIX, DIRECT_AUTH_TOKEN_ENV_KEY,
    DIRECT_BASE_URL_ENV_KEY, MANAGED_ENTERPRISE_CONFIG_KEYS, ONE_M_CONTEXT_MARKER, PROFILE_ID,
    PROFILE_NAME,
};
// `CONFIG_FILE` / `CONFIG_LIBRARY_DIR` are only read inside `paths_from_dirs`,
// which is macOS/Windows/test-gated; gate the imports to match so non-target
// lib builds don't warn about unused imports.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
use super::constants::{CONFIG_FILE, CONFIG_LIBRARY_DIR};
use super::types::{ClaudeDesktopMode, ClaudeDesktopPathInfo};

/// Resolved on-disk paths for Claude Desktop 3P files.
#[derive(Debug, Clone)]
pub struct ClaudeDesktopPaths {
    pub normal_config_path: PathBuf,
    pub threep_config_path: PathBuf,
    pub config_library_path: PathBuf,
    pub profile_path: PathBuf,
    pub meta_path: PathBuf,
}

/// A model spec written into the gateway profile's `inferenceModels`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayModelSpec {
    name: String,
    label_override: Option<String>,
    supports_1m: bool,
    tier_alias: Option<String>,
}

/// Byte snapshot of one managed file, used for rollback on apply failure.
#[derive(Debug, Clone)]
struct FileSnapshot {
    path: PathBuf,
    content: Option<Vec<u8>>,
}

pub fn is_supported_platform() -> bool {
    cfg!(any(target_os = "macos", windows))
}

/// Resolve the Claude Desktop 3P paths for the current platform.
/// Linux returns an error: phase 1 only supports macOS and Windows.
pub fn current_platform_paths() -> Result<ClaudeDesktopPaths, String> {
    #[cfg(target_os = "macos")]
    {
        let home = resolve_home_dir()?;
        let app_support = home.join("Library").join("Application Support");
        return Ok(paths_from_dirs(
            app_support.join("Claude"),
            app_support.join("Claude-3p"),
        ));
    }

    #[cfg(windows)]
    {
        let local_app_data = windows_local_app_data_dir();
        let roaming_app_data = windows_roaming_app_data_dir();
        return Ok(windows_paths_from_roots(&local_app_data, &roaming_app_data));
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err("当前平台暂不支持 Claude Desktop 3P 配置,第一阶段仅支持 macOS 和 Windows。".to_string())
    }
}

#[cfg(target_os = "macos")]
fn resolve_home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "Failed to resolve home directory".to_string())
}

#[cfg(windows)]
fn windows_local_app_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| windows_user_profile_dir().join("AppData").join("Local"))
}

#[cfg(windows)]
fn windows_roaming_app_data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| windows_user_profile_dir().join("AppData").join("Roaming"))
}

#[cfg(windows)]
fn windows_user_profile_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Claude Desktop keeps its standard (1P) and third-party (3P) user data in
/// separate directories. Which physical directory is live depends on the
/// install kind: a classic install uses `%APPDATA%\Claude` +
/// `%LOCALAPPDATA%\Claude-3p`, an MSIX package redirects `%APPDATA%` and
/// `%LOCALAPPDATA%` into `Packages\<family>\LocalCache`, and Desktop 3.x moved
/// the standard data to `%LOCALAPPDATA%\Claude-Data\claude_<family>`.
///
/// The live copy is the one Claude Desktop wrote last, so pick the candidate
/// with the newest Chromium/config marker rather than guessing the install kind.
#[cfg(any(target_os = "windows", test))]
fn windows_paths_from_roots(local_app_data: &Path, roaming_app_data: &Path) -> ClaudeDesktopPaths {
    let normal_dir = resolve_windows_normal_dir(local_app_data, roaming_app_data);
    let threep_dir = resolve_windows_threep_dir(local_app_data, roaming_app_data);
    paths_from_dirs(normal_dir, threep_dir)
}

#[cfg(any(target_os = "windows", test))]
fn resolve_windows_normal_dir(local_app_data: &Path, roaming_app_data: &Path) -> PathBuf {
    let candidates = windows_normal_dir_candidates(local_app_data, roaming_app_data);
    pick_last_used_dir(&candidates).unwrap_or_else(|| roaming_app_data.join("Claude"))
}

#[cfg(any(target_os = "windows", test))]
fn resolve_windows_threep_dir(local_app_data: &Path, roaming_app_data: &Path) -> PathBuf {
    let candidates = windows_threep_dir_candidates(local_app_data);
    if let Some(dir) = pick_last_used_dir(&candidates) {
        return dir;
    }

    // No 3P data yet. If the standard data already lives inside the MSIX
    // package, Windows redirects the app's `%LOCALAPPDATA%\Claude-3p` there,
    // so create the profile where the packaged app will actually read it.
    // A packaged install that has never run exposes no package data, so it
    // still falls back to the classic dirs; once the app runs once, its
    // `LocalCache` appears and the next resolve picks it up.
    let normal_dir = resolve_windows_normal_dir(local_app_data, roaming_app_data);
    if let Some(local_cache) = msix_local_cache_of(&normal_dir, local_app_data) {
        return local_cache.join("Local").join("Claude-3p");
    }

    local_app_data.join("Claude-3p")
}

/// Candidate standard-data directories, ordered from Desktop 3.x back to the
/// classic layout. `pick_last_used_dir` ranks them by their newest marker.
#[cfg(any(target_os = "windows", test))]
fn windows_normal_dir_candidates(local_app_data: &Path, roaming_app_data: &Path) -> Vec<PathBuf> {
    let mut candidates = windows_claude_data_dirs(local_app_data);
    candidates.push(roaming_app_data.join("Claude"));
    // Some older builds kept the standard data under LOCALAPPDATA.
    candidates.push(local_app_data.join("Claude"));
    for local_cache in windows_msix_local_cache_dirs(local_app_data) {
        candidates.push(local_cache.join("Roaming").join("Claude"));
    }
    candidates
}

#[cfg(any(target_os = "windows", test))]
fn windows_threep_dir_candidates(local_app_data: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![local_app_data.join("Claude-3p")];
    candidates.extend(windows_versioned_threep_dirs(local_app_data));
    for local_cache in windows_msix_local_cache_dirs(local_app_data) {
        candidates.push(local_cache.join("Local").join("Claude-3p"));
        candidates.push(local_cache.join("Roaming").join("Claude-3p"));
    }
    candidates
}

/// `%LOCALAPPDATA%\Claude-Data\claude_<family>` (Desktop 3.x). The folder name
/// is the lowercased MSIX package family; fall back to `Claude-Data` itself
/// when a build wrote directly there.
#[cfg(any(target_os = "windows", test))]
fn windows_claude_data_dirs(local_app_data: &Path) -> Vec<PathBuf> {
    let root = local_app_data.join("Claude-Data");
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(is_claude_package_dir_name)
                })
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    if dirs.is_empty() && root.is_dir() {
        dirs.push(root);
    }
    dirs
}

/// `LocalCache` directories of Claude Desktop's MSIX packages, matching both
/// the download package (`Claude_<publisher>`) and the Store one
/// (`AnthropicPBC.Claude_<publisher>`).
#[cfg(any(target_os = "windows", test))]
fn windows_msix_local_cache_dirs(local_app_data: &Path) -> Vec<PathBuf> {
    let packages = local_app_data.join("Packages");
    let mut dirs: Vec<PathBuf> = fs::read_dir(&packages)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(is_claude_package_dir_name)
                })
                .map(|path| path.join("LocalCache"))
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

#[cfg(any(target_os = "windows", test))]
fn is_claude_package_dir_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("claude_") || lower.starts_with("anthropicpbc.claude_")
}

/// Versioned siblings such as `Claude-3p-1.0.187`, kept for older installs.
#[cfg(any(target_os = "windows", test))]
fn windows_versioned_threep_dirs(local_app_data: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(local_app_data)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.to_ascii_lowercase().starts_with("claude-3p-"))
                })
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

/// Return the package `LocalCache` directory when `path` is inside an MSIX
/// package container (`%LOCALAPPDATA%\Packages\<family>\...`).
#[cfg(any(target_os = "windows", test))]
fn msix_local_cache_of(path: &Path, local_app_data: &Path) -> Option<PathBuf> {
    let packages = local_app_data.join("Packages");
    let rest = path.strip_prefix(&packages).ok()?;
    let family = rest.components().next()?;
    Some(packages.join(family.as_os_str()).join("LocalCache"))
}

/// Pick the candidate Claude Desktop wrote to last. `Local State` outranks
/// config files because another tool can leave a recent
/// `claude_desktop_config.json` in a folder the app never ran in (the legacy
/// `%LOCALAPPDATA%\Claude` stub is exactly that). When no candidate has a
/// marker, keep the first existing one (the caller's priority order).
#[cfg(any(target_os = "windows", test))]
fn pick_last_used_dir(candidates: &[PathBuf]) -> Option<PathBuf> {
    let mut first_existing: Option<PathBuf> = None;
    let mut newest: Option<(PathBuf, (u8, SystemTime))> = None;

    for candidate in candidates {
        if !candidate.is_dir() {
            continue;
        }
        if first_existing.is_none() {
            first_existing = Some(candidate.clone());
        }
        let Some(rank) = dir_last_used_rank(candidate) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(_, current)| rank > *current) {
            newest = Some((candidate.clone(), rank));
        }
    }

    newest.map(|(dir, _)| dir).or(first_existing)
}

/// `Local State` is Chromium's "the app ran here" marker; config/library files
/// still count, but rank below it so a stub folder cannot win.
#[cfg(any(target_os = "windows", test))]
fn dir_last_used_rank(dir: &Path) -> Option<(u8, SystemTime)> {
    if let Some(modified) = marker_mtime(&dir.join("Local State")) {
        return Some((2, modified));
    }
    [
        dir.join("ant-did"),
        dir.join(CONFIG_FILE),
        dir.join(CONFIG_LIBRARY_DIR).join("_meta.json"),
    ]
    .iter()
    .filter_map(|path| marker_mtime(path))
    .max()
    .map(|modified| (1, modified))
}

#[cfg(any(target_os = "windows", test))]
fn marker_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .and_then(|metadata| metadata.modified().ok())
}

// Compiled on macOS/Windows (production use) and under `cfg(test)` (cross-platform
// unit tests in `mod tests` build `ClaudeDesktopPaths` via this helper). Without
// `test` in the gate, Linux lib builds report it as dead code (phase 1 only
// supports macOS and Windows).
#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn paths_from_dirs(normal_dir: PathBuf, threep_dir: PathBuf) -> ClaudeDesktopPaths {
    let config_library_path = threep_dir.join(CONFIG_LIBRARY_DIR);
    let profile_path = config_library_path.join(format!("{PROFILE_ID}.json"));
    let meta_path = config_library_path.join("_meta.json");

    ClaudeDesktopPaths {
        normal_config_path: normal_dir.join(CONFIG_FILE),
        threep_config_path: threep_dir.join(CONFIG_FILE),
        config_library_path,
        profile_path,
        meta_path,
    }
}

/// Build the frontend-facing path info; returns `supported=false` on Linux.
pub fn get_claude_desktop_path_info() -> ClaudeDesktopPathInfo {
    if !is_supported_platform() {
        return ClaudeDesktopPathInfo {
            supported: false,
            normal_config_path: None,
            threep_config_path: None,
            config_library_path: None,
            profile_path: None,
            meta_path: None,
            message: Some(
                "当前平台暂不支持 Claude Desktop 3P 配置,第一阶段仅支持 macOS 和 Windows。"
                    .to_string(),
            ),
        };
    }

    match current_platform_paths() {
        Ok(paths) => ClaudeDesktopPathInfo {
            supported: true,
            normal_config_path: Some(paths.normal_config_path.to_string_lossy().to_string()),
            threep_config_path: Some(paths.threep_config_path.to_string_lossy().to_string()),
            config_library_path: Some(paths.config_library_path.to_string_lossy().to_string()),
            profile_path: Some(paths.profile_path.to_string_lossy().to_string()),
            meta_path: Some(paths.meta_path.to_string_lossy().to_string()),
            message: None,
        },
        Err(message) => ClaudeDesktopPathInfo {
            supported: false,
            normal_config_path: None,
            threep_config_path: None,
            config_library_path: None,
            profile_path: None,
            meta_path: None,
            message: Some(message),
        },
    }
}

/// Directories that may hold Claude Desktop's standard (1P) user data on the
/// current platform. Session import scans every candidate because old installs
/// can leave their data in a layout the current app no longer writes.
pub fn standard_data_dirs() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let Ok(home) = resolve_home_dir() else {
            return Vec::new();
        };
        vec![home
            .join("Library")
            .join("Application Support")
            .join("Claude")]
    }

    #[cfg(windows)]
    {
        let local_app_data = windows_local_app_data_dir();
        let roaming_app_data = windows_roaming_app_data_dir();
        windows_normal_dir_candidates(&local_app_data, &roaming_app_data)
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        dirs::config_dir()
            .map(|dir| vec![dir.join("Claude")])
            .unwrap_or_default()
    }
}

/// Read the provider's mode from its `meta.claude_desktop_mode`.
pub fn provider_mode(meta: Option<&Value>) -> ClaudeDesktopMode {
    let raw = meta
        .and_then(|m| m.get("claude_desktop_mode"))
        .and_then(Value::as_str);
    match raw {
        Some("proxy") => ClaudeDesktopMode::Proxy,
        _ => ClaudeDesktopMode::Direct,
    }
}

/// Claude Desktop model menu only accepts `claude-<role>-<id>` (optionally prefixed
/// with `anthropic/`) as a safe id, and rejects any `[1m]` marker or degraded
/// value like `claude-sonnet-`.
pub fn is_claude_safe_model_id(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    if normalized.contains(ONE_M_CONTEXT_MARKER) {
        return false;
    }

    let Some(route_tail) = normalized
        .strip_prefix(ANTHROPIC_CLAUDE_ROUTE_PREFIX)
        .or_else(|| normalized.strip_prefix(CLAUDE_ROUTE_PREFIX))
    else {
        return false;
    };

    ["sonnet-", "opus-", "haiku-", "fable-"]
        .iter()
        .any(|prefix| {
            route_tail
                .strip_prefix(prefix)
                .is_some_and(|rest| !rest.is_empty())
        })
}

/// Apply a non-official provider to the on-disk profile, with rollback.
pub fn apply_provider_to_paths(
    settings_config: Value,
    meta: Option<&Value>,
    paths: &ClaudeDesktopPaths,
) -> Result<(), String> {
    validate_provider(&settings_config, meta)?;
    with_rollback(paths, || {
        apply_provider_to_paths_inner(&settings_config, meta, paths)
    })
}

/// Point the Claude Desktop 3P profile at the local gateway, with rollback.
///
/// This is the takeover / proxy wiring path: it writes deploymentMode 3p, a
/// gateway profile whose `inferenceGatewayBaseUrl` is the gateway's
/// claude-desktop inbound endpoint and `inferenceGatewayApiKey` is a sentinel
/// token, then applies the profile id in the meta file.
pub fn apply_gateway_proxy_profile(
    paths: &ClaudeDesktopPaths,
    gateway_base_url: &str,
    gateway_api_key: &str,
    model_specs: Option<&[GatewayModelSpec]>,
) -> Result<(), String> {
    log::warn!(
        "[desktop-config] apply_gateway_proxy_profile entry: specs={:?}",
        model_specs.map(|s| s.iter().map(|x| x.name.clone()).collect::<Vec<_>>())
    );
    let result = with_rollback(paths, || {
        let profile = build_gateway_profile(gateway_base_url, gateway_api_key, model_specs);
        log::warn!(
            "[desktop-config] built profile inferenceModels={:?}",
            profile.get("inferenceModels")
        );
        write_deployment_mode(&paths.normal_config_path, "3p")?;
        write_deployment_mode(&paths.threep_config_path, "3p")?;
        atomic_write_json(&paths.profile_path, &profile)?;
        write_meta(&paths.meta_path, Some(PROFILE_ID))?;
        log::warn!("[desktop-config] all writes OK");
        Ok(())
    });
    log::warn!(
        "[desktop-config] apply_gateway_proxy_profile result = {:?}",
        result.is_ok()
    );
    result
}

/// Restore the official (1P) mode, with rollback.
pub fn restore_official(paths: &ClaudeDesktopPaths) -> Result<(), String> {
    with_rollback(paths, || restore_official_inner(paths))
}

fn validate_provider(settings_config: &Value, meta: Option<&Value>) -> Result<(), String> {
    if !settings_config.is_object() {
        return Err("Claude Desktop provider configuration must be a JSON object".to_string());
    }

    match provider_mode(meta) {
        ClaudeDesktopMode::Direct => {
            direct_gateway_credentials(settings_config)?;
            direct_inference_model_specs(meta, Some(settings_config))?;
            Ok(())
        }
        // The local gateway is not wired yet; keep the mode branch present but reject.
        ClaudeDesktopMode::Proxy => {
            Err("Claude Desktop 本地路由模式尚在建设中,请先使用直连模式".to_string())
        }
    }
}

fn apply_provider_to_paths_inner(
    settings_config: &Value,
    meta: Option<&Value>,
    paths: &ClaudeDesktopPaths,
) -> Result<(), String> {
    // Proxy mode is rejected earlier in validation; only Direct reaches here today.
    let (base_url, api_key) = direct_gateway_credentials(settings_config)?;
    let model_specs = direct_inference_model_specs(meta, Some(settings_config))?;
    let profile = build_gateway_profile(
        &base_url,
        &api_key,
        (!model_specs.is_empty()).then_some(model_specs.as_slice()),
    );

    write_deployment_mode(&paths.normal_config_path, "3p")?;
    write_deployment_mode(&paths.threep_config_path, "3p")?;
    atomic_write_json(&paths.profile_path, &profile)?;
    write_meta(&paths.meta_path, Some(PROFILE_ID))?;

    Ok(())
}

fn restore_official_inner(paths: &ClaudeDesktopPaths) -> Result<(), String> {
    write_deployment_mode(&paths.normal_config_path, "1p")?;
    write_deployment_mode(&paths.threep_config_path, "1p")?;
    remove_managed_enterprise_config(&paths.threep_config_path)?;

    if paths.profile_path.exists() {
        fs::remove_file(&paths.profile_path).map_err(|error| {
            format!(
                "Failed to delete profile file {}: {error}",
                paths.profile_path.display()
            )
        })?;
    }
    write_meta(&paths.meta_path, None)?;

    Ok(())
}

/// Extract direct-mode credentials from `settings_config.env`.
pub fn direct_gateway_credentials(settings_config: &Value) -> Result<(String, String), String> {
    let env = settings_config
        .get("env")
        .and_then(Value::as_object)
        .ok_or_else(|| "Claude Desktop 直连供应商缺少 env 配置".to_string())?;

    let base_url = required_env(&env, DIRECT_BASE_URL_ENV_KEY, "ANTHROPIC_BASE_URL")?;
    let api_key = required_env(&env, DIRECT_AUTH_TOKEN_ENV_KEY, "ANTHROPIC_AUTH_TOKEN")?;
    Ok((base_url, api_key))
}

fn required_env(env: &Map<String, Value>, key: &str, label: &str) -> Result<String, String> {
    env.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Claude Desktop 直连供应商缺少 {label}"))
}

/// Build the gateway profile JSON. When `model_specs` is None or empty the
/// `inferenceModels` field is omitted entirely.
pub fn build_gateway_profile(
    base_url: &str,
    api_key: &str,
    model_specs: Option<&[GatewayModelSpec]>,
) -> Value {
    let mut profile = json!({
        "coworkEgressAllowedHosts": ["*"],
        "disableDeploymentModeChooser": true,
        "inferenceGatewayApiKey": api_key,
        "inferenceGatewayAuthScheme": "bearer",
        "inferenceGatewayBaseUrl": base_url,
        "inferenceProvider": "gateway"
    });

    if let Some(model_specs) = model_specs {
        if !model_specs.is_empty() {
            profile["inferenceModels"] =
                Value::Array(model_specs.iter().map(inference_model_json).collect());
        }
    }

    profile
}

fn inference_model_json(spec: &GatewayModelSpec) -> Value {
    if spec.supports_1m || spec.label_override.is_some() || spec.tier_alias.is_some() {
        let mut item = json!({ "name": spec.name });
        if let Some(label_override) = spec.label_override.as_deref() {
            item["labelOverride"] = json!(label_override);
        }
        if spec.supports_1m {
            item["supports1m"] = json!(true);
        }
        if let Some(tier_alias) = spec.tier_alias.as_deref() {
            item["anthropicFamilyTier"] = json!(tier_alias);
        }
        item
    } else {
        Value::String(spec.name.clone())
    }
}

/// Effective desktop role route map. Meta routes
/// (`claudeDesktopModelRoutes`) take precedence; providers imported from Claude
/// Code carry their role models in `settings_config.env`
/// (`ANTHROPIC_DEFAULT_*_MODEL`) instead, so fall back to deriving the same
/// routes from there until the row is re-saved through the Desktop form.
fn effective_desktop_routes(
    meta: Option<&Value>,
    settings_config: Option<&Value>,
) -> Option<Map<String, Value>> {
    if let Some(routes) = meta
        .and_then(|m| {
            m.get("claudeDesktopModelRoutes")
                .or_else(|| m.get("claude_desktop_model_routes"))
        })
        .and_then(Value::as_object)
    {
        return Some(routes.clone());
    }
    desktop_routes_from_settings(settings_config?)
}

fn desktop_routes_from_settings(settings_config: &Value) -> Option<Map<String, Value>> {
    let env = settings_config.get("env").and_then(Value::as_object)?;
    const ROLE_ROUTES: [(&str, &str, &str); 4] = [
        (
            "claude-sonnet-5",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
        ),
        (
            "claude-opus-5",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
        ),
        (
            "claude-fable-5",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
        ),
        (
            "claude-haiku-4-5",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
        ),
    ];
    let mut routes = Map::new();
    for (route_id, model_key, name_key) in ROLE_ROUTES {
        let Some(model) = env
            .get(model_key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|model| !model.is_empty())
        else {
            continue;
        };
        let mut route = Map::new();
        route.insert("model".to_string(), Value::String(model.to_string()));
        if let Some(label) = env
            .get(name_key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|label| !label.is_empty())
        {
            route.insert(
                "labelOverride".to_string(),
                Value::String(label.to_string()),
            );
        }
        route.insert("supports1m".to_string(), Value::Bool(false));
        routes.insert(route_id.to_string(), Value::Object(route));
    }
    (!routes.is_empty()).then_some(routes)
}

/// Build direct-mode model specs from the effective routes.
/// Direct mode forbids model mapping: each route's upstream must equal its id.
fn direct_inference_model_specs(
    meta: Option<&Value>,
    settings_config: Option<&Value>,
) -> Result<Vec<GatewayModelSpec>, String> {
    let Some(routes) = effective_desktop_routes(meta, settings_config) else {
        return Ok(Vec::new());
    };

    let mut result = Vec::new();
    for (route_id, route) in &routes {
        let supports_1m = route
            .get("supports_1m")
            .or_else(|| route.get("supports1m"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let route_id = route_id.trim();
        if route_id.is_empty() {
            continue;
        }
        if !is_claude_safe_model_id(route_id) {
            return Err(format!(
                "Claude Desktop 直连模型必须使用 claude-* 或 anthropic/claude-* 名称: {route_id}"
            ));
        }
        let upstream_model = route
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if !upstream_model.is_empty() && upstream_model != route_id {
            return Err(format!(
                "Claude Desktop 直连模式不能映射模型: {route_id} -> {upstream_model};非 Claude 官方模型请使用本地路由模式"
            ));
        }
        result.push(GatewayModelSpec {
            name: route_id.to_string(),
            label_override: route
                .get("label_override")
                .or_else(|| route.get("labelOverride"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            supports_1m,
            tier_alias: route
                .get("tier_alias")
                .or_else(|| route.get("tierAlias"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_lowercase())
                .filter(|value| {
                    matches!(
                        value.as_str(),
                        "haiku" | "sonnet" | "opus" | "fable" | "mythos"
                    )
                }),
        });
    }

    // Sort supports_1m=true first within each name so dedup keeps the 1M variant.
    result.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| b.supports_1m.cmp(&a.supports_1m))
    });
    let mut seen: HashSet<String> = HashSet::new();
    result.retain(|spec| seen.insert(spec.name.clone()));
    Ok(result)
}

/// Build gateway proxy-mode model specs from the effective routes.
/// Unlike direct mode, proxy mode routes the claude-safe `route_id` to an arbitrary
/// upstream model via the local gateway, so the upstream model is ignored here and
/// only the claude-safe route_id + labelOverride + supports1m surface in the menu.
pub fn desktop_proxy_model_specs(
    meta: Option<&Value>,
    settings_config: Option<&Value>,
) -> Vec<GatewayModelSpec> {
    let Some(routes) = effective_desktop_routes(meta, settings_config) else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for (route_id, route) in &routes {
        let supports_1m = route
            .get("supports_1m")
            .or_else(|| route.get("supports1m"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let route_id = route_id.trim();
        if route_id.is_empty() || !is_claude_safe_model_id(route_id) {
            continue;
        }
        result.push(GatewayModelSpec {
            name: route_id.to_string(),
            label_override: route
                .get("label_override")
                .or_else(|| route.get("labelOverride"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            supports_1m,
            tier_alias: route
                .get("tier_alias")
                .or_else(|| route.get("tierAlias"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_lowercase())
                .filter(|value| {
                    matches!(
                        value.as_str(),
                        "haiku" | "sonnet" | "opus" | "fable" | "mythos"
                    )
                }),
        });
    }

    result.sort_by(|a, b| a.name.cmp(&b.name));
    result.dedup_by(|a, b| a.name == b.name);
    result
}

/// True when the effective routes contain at least one route whose upstream
/// `model` differs from its claude-safe `route_id`. Such a provider cannot be
/// written in Direct mode and must be routed through the local gateway.
pub fn has_routing_models(meta: Option<&Value>, settings_config: Option<&Value>) -> bool {
    let Some(routes) = effective_desktop_routes(meta, settings_config) else {
        return false;
    };
    routes.iter().any(|(route_id, route)| {
        let model = route.get("model").and_then(Value::as_str).unwrap_or("");
        !model.is_empty() && model.trim() != route_id.trim()
    })
}

// ============================================================================
// File primitives
// ============================================================================

fn read_json_or_empty(path: &Path) -> Result<Value, String> {
    let value = if path.exists() {
        let content = fs::read_to_string(path)
            .map_err(|error| format!("Failed to read config file {}: {error}", path.display()))?;
        serde_json::from_str::<Value>(&content).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    if value.is_object() {
        Ok(value)
    } else {
        Ok(json!({}))
    }
}

/// Read a JSON file into a Value, treating a missing or non-object file as `{}`.
pub fn read_json_file_or_empty(path: &Path) -> Result<Value, String> {
    read_json_or_empty(path)
}

/// Read the applied profile JSON (best-effort; non-object -> `{}`).
pub fn read_profile_json(path: &Path) -> Value {
    read_json_or_empty(path).unwrap_or_else(|_| json!({}))
}

fn write_deployment_mode(path: &Path, mode: &str) -> Result<(), String> {
    let mut value = read_json_or_empty(path)?;
    if !value.is_object() {
        value = json!({});
    }
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "deploymentMode".to_string(),
            Value::String(mode.to_string()),
        );
    }
    atomic_write_json(path, &value)
}

fn remove_managed_enterprise_config(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    let mut value = read_json_or_empty(path)?;
    let Some(obj) = value.as_object_mut() else {
        return Ok(());
    };
    let Some(enterprise) = obj
        .get_mut("enterpriseConfig")
        .and_then(Value::as_object_mut)
    else {
        return Ok(());
    };

    for key in MANAGED_ENTERPRISE_CONFIG_KEYS {
        enterprise.remove(key);
    }

    if enterprise.is_empty() {
        obj.remove("enterpriseConfig");
    }

    atomic_write_json(path, &value)
}

fn write_meta(path: &Path, applied_profile_id: Option<&str>) -> Result<(), String> {
    let mut value = read_json_or_empty(path)?;
    if !value.is_object() {
        value = json!({});
    }

    let obj = value.as_object_mut().expect("just normalized to object");
    let mut entries = obj
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    entries.retain(|entry| entry.get("id").and_then(Value::as_str) != Some(PROFILE_ID));

    match applied_profile_id {
        Some(id) => {
            entries.push(json!({
                "id": PROFILE_ID,
                "name": PROFILE_NAME
            }));
            obj.insert("appliedId".to_string(), Value::String(id.to_string()));
        }
        None => {
            let should_clear_applied = obj
                .get("appliedId")
                .and_then(Value::as_str)
                .is_some_and(|id| id == PROFILE_ID);
            if should_clear_applied {
                if let Some(next_id) = entries
                    .iter()
                    .find_map(|entry| entry.get("id").and_then(Value::as_str))
                {
                    obj.insert("appliedId".to_string(), Value::String(next_id.to_string()));
                } else {
                    obj.remove("appliedId");
                }
            }
        }
    }

    obj.insert("entries".to_string(), Value::Array(entries));
    atomic_write_json(path, &value)
}

/// Read `appliedId` from the meta file, if present.
pub fn read_applied_id(path: &Path) -> Option<String> {
    read_json_or_empty(path).ok().and_then(|value| {
        value
            .get("appliedId")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

/// Whether the meta file already lists our profile id in its entries.
pub fn meta_has_profile_entry(path: &Path) -> bool {
    read_json_or_empty(path)
        .ok()
        .and_then(|value| value.get("entries").and_then(Value::as_array).cloned())
        .is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.get("id").and_then(Value::as_str) == Some(PROFILE_ID))
        })
}

// ============================================================================
// Atomic write + deterministic (sorted) JSON output
// ============================================================================

/// Recursively rebuild a Value so object keys are sorted alphabetically, giving
/// deterministic file output regardless of serde_json `preserve_order`.
fn sort_json_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map
                .into_iter()
                .map(|(key, value)| (key, sort_json_value(value)))
                .collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut sorted = Map::new();
            for (key, value) in entries {
                sorted.insert(key, value);
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_json_value).collect()),
        other => other,
    }
}

/// Atomically write a JSON Value with pretty-printed, key-sorted output.
fn atomic_write_json(path: &Path, value: &Value) -> Result<(), String> {
    let sorted = sort_json_value(value.clone());
    let content = serde_json::to_string_pretty(&sorted)
        .map_err(|error| format!("Failed to serialize JSON: {error}"))?;
    atomic_write_bytes(path, format!("{content}\n").as_bytes())
}

fn atomic_write_bytes(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Failed to resolve parent of {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create directory {}: {error}", parent.display()))?;

    let temp_path = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    fs::write(&temp_path, content)
        .map_err(|error| format!("Failed to write temp file {}: {error}", temp_path.display()))?;
    fs::rename(&temp_path, path)
        .map_err(|error| format!("Failed to rename temp file to {}: {error}", path.display()))?;
    Ok(())
}

// ============================================================================
// Snapshot / rollback
// ============================================================================

fn with_rollback(
    paths: &ClaudeDesktopPaths,
    op: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let snapshots = snapshot_files(paths)?;
    match op() {
        Ok(()) => Ok(()),
        Err(error) => match restore_snapshots(&snapshots) {
            Ok(()) => Err(error),
            Err(rollback_error) => {
                log::error!(
                    "Failed to rollback Claude Desktop config after error: {rollback_error}"
                );
                Err(format!("{error}; rollback failed: {rollback_error}"))
            }
        },
    }
}

fn snapshot_files(paths: &ClaudeDesktopPaths) -> Result<Vec<FileSnapshot>, String> {
    [
        &paths.normal_config_path,
        &paths.threep_config_path,
        &paths.profile_path,
        &paths.meta_path,
    ]
    .into_iter()
    .map(|path| {
        let content = if path.exists() {
            Some(
                fs::read(path)
                    .map_err(|error| format!("Failed to snapshot {}: {error}", path.display()))?,
            )
        } else {
            None
        };
        Ok(FileSnapshot {
            path: path.clone(),
            content,
        })
    })
    .collect()
}

fn restore_snapshots(snapshots: &[FileSnapshot]) -> Result<(), String> {
    for snapshot in snapshots {
        match &snapshot.content {
            Some(content) => {
                if let Some(parent) = snapshot.path.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("Failed to create directory {}: {error}", parent.display())
                    })?;
                }
                atomic_write_bytes(&snapshot.path, content)?;
            }
            None => {
                if snapshot.path.exists() {
                    fs::remove_file(&snapshot.path).map_err(|error| {
                        format!("Failed to delete {}: {error}", snapshot.path.display())
                    })?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths(home: &Path) -> ClaudeDesktopPaths {
        let app_support = home.join("Library").join("Application Support");
        paths_from_dirs(app_support.join("Claude"), app_support.join("Claude-3p"))
    }

    fn direct_provider() -> (Value, Option<Value>) {
        let settings = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://gateway.example.com",
                "ANTHROPIC_AUTH_TOKEN": "test-token"
            }
        });
        (settings, None)
    }

    fn windows_roots(temp: &Path) -> (PathBuf, PathBuf) {
        (temp.join("Local"), temp.join("Roaming"))
    }

    fn write_marker(path: &Path, age_secs: u64) {
        fs::create_dir_all(path.parent().expect("marker parent")).expect("create marker dir");
        let file = fs::File::create(path).expect("create marker");
        file.set_modified(
            SystemTime::now()
                .checked_sub(std::time::Duration::from_secs(age_secs))
                .expect("marker mtime"),
        )
        .expect("set marker mtime");
    }

    #[test]
    fn windows_classic_layout_uses_roaming_and_local_threep() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let (local, roaming) = windows_roots(temp.path());
        write_marker(&roaming.join("Claude").join("Local State"), 10);
        write_marker(&local.join("Claude-3p").join("Local State"), 10);

        let paths = windows_paths_from_roots(&local, &roaming);

        assert_eq!(
            paths.normal_config_path,
            roaming.join("Claude").join(CONFIG_FILE)
        );
        assert_eq!(
            paths.threep_config_path,
            local.join("Claude-3p").join(CONFIG_FILE)
        );
    }

    #[test]
    fn windows_msix_layout_uses_package_local_cache() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let (local, roaming) = windows_roots(temp.path());
        let local_cache = local
            .join("Packages")
            .join("Claude_pzs8sxrjxfjjc")
            .join("LocalCache");
        write_marker(
            &local_cache
                .join("Roaming")
                .join("Claude")
                .join("Local State"),
            10,
        );
        write_marker(
            &local_cache
                .join("Local")
                .join("Claude-3p")
                .join("Local State"),
            10,
        );

        let paths = windows_paths_from_roots(&local, &roaming);

        assert_eq!(
            paths.normal_config_path,
            local_cache.join("Roaming").join("Claude").join(CONFIG_FILE)
        );
        assert_eq!(
            paths.threep_config_path,
            local_cache
                .join("Local")
                .join("Claude-3p")
                .join(CONFIG_FILE)
        );
    }

    #[test]
    fn windows_claude_data_wins_over_stale_roaming() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let (local, roaming) = windows_roots(temp.path());
        let claude_data = local.join("Claude-Data").join("claude_pzs8sxrjxfjjc");
        write_marker(&claude_data.join("Local State"), 10);
        write_marker(&roaming.join("Claude").join("Local State"), 3_600);
        write_marker(&local.join("Claude-3p").join("Local State"), 10);

        let paths = windows_paths_from_roots(&local, &roaming);

        assert_eq!(paths.normal_config_path, claude_data.join(CONFIG_FILE));
        assert_eq!(
            paths.threep_config_path,
            local.join("Claude-3p").join(CONFIG_FILE)
        );
    }

    #[test]
    fn windows_legacy_stub_config_does_not_beat_real_app_data() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let (local, roaming) = windows_roots(temp.path());
        write_marker(&roaming.join("Claude").join("Local State"), 3_600);
        // The legacy `%LOCALAPPDATA%\Claude` stub only holds a recent config.
        write_marker(&local.join("Claude").join(CONFIG_FILE), 10);
        write_marker(&local.join("Claude-3p").join("Local State"), 10);

        let paths = windows_paths_from_roots(&local, &roaming);

        assert_eq!(
            paths.normal_config_path,
            roaming.join("Claude").join(CONFIG_FILE)
        );
    }

    #[test]
    fn windows_prefers_recent_threep_copy_over_package() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let (local, roaming) = windows_roots(temp.path());
        let local_cache = local
            .join("Packages")
            .join("Claude_pzs8sxrjxfjjc")
            .join("LocalCache");
        write_marker(
            &local_cache
                .join("Local")
                .join("Claude-3p")
                .join("Local State"),
            3_600,
        );
        write_marker(&local.join("Claude-3p").join("Local State"), 10);

        let paths = windows_paths_from_roots(&local, &roaming);

        assert_eq!(
            paths.threep_config_path,
            local.join("Claude-3p").join(CONFIG_FILE)
        );
    }

    #[test]
    fn windows_packaged_install_with_existing_local_cache_creates_threep_in_package() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let (local, roaming) = windows_roots(temp.path());
        let local_cache = local
            .join("Packages")
            .join("AnthropicPBC.Claude_fnn82j28hfe8t")
            .join("LocalCache");
        write_marker(
            &local_cache.join("Roaming").join("Claude").join(CONFIG_FILE),
            10,
        );

        let paths = windows_paths_from_roots(&local, &roaming);

        assert_eq!(
            paths.normal_config_path,
            local_cache.join("Roaming").join("Claude").join(CONFIG_FILE)
        );
        assert_eq!(
            paths.threep_config_path,
            local_cache
                .join("Local")
                .join("Claude-3p")
                .join(CONFIG_FILE)
        );
    }

    #[test]
    fn windows_fresh_install_defaults_to_standard_dirs() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let (local, roaming) = windows_roots(temp.path());

        let paths = windows_paths_from_roots(&local, &roaming);

        assert_eq!(
            paths.normal_config_path,
            roaming.join("Claude").join(CONFIG_FILE)
        );
        assert_eq!(
            paths.threep_config_path,
            local.join("Claude-3p").join(CONFIG_FILE)
        );
    }

    #[test]
    fn windows_versioned_threep_dir_is_a_candidate() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let (local, roaming) = windows_roots(temp.path());
        write_marker(&roaming.join("Claude").join("Local State"), 10);
        write_marker(&local.join("Claude-3p-1.0.187").join("Local State"), 10);

        let paths = windows_paths_from_roots(&local, &roaming);

        assert_eq!(
            paths.threep_config_path,
            local.join("Claude-3p-1.0.187").join(CONFIG_FILE)
        );
    }

    #[test]
    fn windows_markerless_dirs_keep_candidate_order() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let (local, roaming) = windows_roots(temp.path());
        fs::create_dir_all(roaming.join("Claude")).expect("roaming Claude");
        fs::create_dir_all(local.join("Claude")).expect("local Claude stub");
        fs::create_dir_all(local.join("Claude-3p")).expect("real 3p");
        fs::create_dir_all(local.join("Claude-3p-1.0.187")).expect("versioned 3p");

        let paths = windows_paths_from_roots(&local, &roaming);

        assert_eq!(
            paths.normal_config_path,
            roaming.join("Claude").join(CONFIG_FILE)
        );
        assert_eq!(
            paths.threep_config_path,
            local.join("Claude-3p").join(CONFIG_FILE)
        );
    }

    #[test]
    fn windows_ignores_non_claude_package_dirs() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let (local, roaming) = windows_roots(temp.path());
        write_marker(
            &local
                .join("Packages")
                .join("OtherPublisher.App_x")
                .join("LocalCache")
                .join("Roaming")
                .join("Claude")
                .join("Local State"),
            10,
        );

        let paths = windows_paths_from_roots(&local, &roaming);

        assert_eq!(
            paths.normal_config_path,
            roaming.join("Claude").join(CONFIG_FILE)
        );
        assert_eq!(
            paths.threep_config_path,
            local.join("Claude-3p").join(CONFIG_FILE)
        );
    }

    #[test]
    fn rejects_1m_suffix_and_degraded_ids() {
        assert!(!is_claude_safe_model_id("claude-sonnet-4-6 [1m]"));
        assert!(!is_claude_safe_model_id("claude-sonnet-"));
        assert!(!is_claude_safe_model_id("claude-gpt-5"));
        assert!(!is_claude_safe_model_id("sonnet-4-6"));
        assert!(is_claude_safe_model_id("claude-sonnet-4-6"));
        assert!(is_claude_safe_model_id("anthropic/claude-opus-4-8"));
    }

    #[test]
    fn apply_writes_3p_profile_and_meta() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let paths = test_paths(temp.path());
        let (settings, meta) = direct_provider();

        apply_provider_to_paths(settings, meta.as_ref(), &paths).expect("apply");

        let normal: Value = read_json_file_or_empty(&paths.normal_config_path).expect("normal");
        let threep: Value = read_json_file_or_empty(&paths.threep_config_path).expect("threep");
        let profile = read_profile_json(&paths.profile_path);
        let meta_path: Value = read_json_file_or_empty(&paths.meta_path).expect("meta");

        assert_eq!(normal["deploymentMode"], json!("3p"));
        assert_eq!(threep["deploymentMode"], json!("3p"));
        assert_eq!(profile["inferenceProvider"], json!("gateway"));
        assert_eq!(
            profile["inferenceGatewayBaseUrl"],
            json!("https://gateway.example.com")
        );
        assert_eq!(profile["inferenceGatewayApiKey"], json!("test-token"));
        assert_eq!(meta_path["appliedId"], json!(PROFILE_ID));
        assert!(meta_path["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .any(|entry| entry["id"] == json!(PROFILE_ID) && entry["name"] == json!(PROFILE_NAME)));
    }

    #[test]
    fn restore_switches_to_1p_and_removes_profile() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let paths = test_paths(temp.path());
        let (settings, meta) = direct_provider();

        apply_provider_to_paths(settings, meta.as_ref(), &paths).expect("apply");
        restore_official(&paths).expect("restore");

        let normal: Value = read_json_file_or_empty(&paths.normal_config_path).expect("normal");
        let threep: Value = read_json_file_or_empty(&paths.threep_config_path).expect("threep");
        let meta_path: Value = read_json_file_or_empty(&paths.meta_path).expect("meta");

        assert_eq!(normal["deploymentMode"], json!("1p"));
        assert_eq!(threep["deploymentMode"], json!("1p"));
        assert!(!paths.profile_path.exists());
        assert!(meta_path.get("appliedId").is_none());
    }

    #[test]
    fn apply_gateway_proxy_profile_points_desktop_at_gateway() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let paths = test_paths(temp.path());

        apply_gateway_proxy_profile(
            &paths,
            "http://127.0.0.1:37123/claude-desktop",
            "ai-toolbox-gateway",
            None,
        )
        .expect("apply gateway proxy profile");

        let normal: Value = read_json_file_or_empty(&paths.normal_config_path).expect("normal");
        let threep: Value = read_json_file_or_empty(&paths.threep_config_path).expect("threep");
        let profile = read_profile_json(&paths.profile_path);
        let meta_path: Value = read_json_file_or_empty(&paths.meta_path).expect("meta");

        assert_eq!(normal["deploymentMode"], json!("3p"));
        assert_eq!(threep["deploymentMode"], json!("3p"));
        assert_eq!(profile["inferenceProvider"], json!("gateway"));
        assert_eq!(
            profile["inferenceGatewayBaseUrl"],
            json!("http://127.0.0.1:37123/claude-desktop")
        );
        assert_eq!(
            profile["inferenceGatewayApiKey"],
            json!("ai-toolbox-gateway")
        );
        assert_eq!(meta_path["appliedId"], json!(PROFILE_ID));

        // Restore switches back to official (1p) and removes the profile.
        restore_official(&paths).expect("restore");
        let normal_after: Value =
            read_json_file_or_empty(&paths.normal_config_path).expect("normal after");
        assert_eq!(normal_after["deploymentMode"], json!("1p"));
        assert!(!paths.profile_path.exists());
    }

    #[test]
    fn direct_mode_rejects_model_mapping() {
        let (settings, _) = direct_provider();
        let meta = json!({
            "claude_desktop_mode": "direct",
            "claude_desktop_model_routes": {
                "claude-sonnet-4-6": { "model": "mimo-v2.5-pro", "supports_1m": true }
            }
        });
        let err =
            validate_provider(&settings, Some(&meta)).expect_err("direct mapping should fail");
        assert!(err.contains("不能映射模型"));
    }

    #[test]
    fn output_is_key_sorted() {
        let value = json!({
            "z": 1,
            "a": { "y": 2, "b": 3 },
            "m": [4, { "q": 5, "p": 6 }]
        });
        let sorted = sort_json_value(value);
        let text = serde_json::to_string(&sorted).expect("serialize");
        assert_eq!(text, r#"{"a":{"b":3,"y":2},"m":[4,{"p":6,"q":5}],"z":1}"#);
    }

    #[test]
    fn proxy_specs_carry_supports1m_and_tier_alias() {
        // Direct mode forbids mapping (upstream must equal route_id), so the
        // supports1m / anthropicFamilyTier wire fields are exercised via the proxy
        // specs path, which reads them from meta.claudeDesktopModelRoutes.
        let settings = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://gateway.example.com",
                "ANTHROPIC_AUTH_TOKEN": "test-token"
            }
        });
        let meta = json!({
            "claude_desktop_model_routes": {
                "claude-sonnet-5": {
                    "model": "glm-4.6",
                    "labelOverride": "GLM 4.6",
                    "supports1m": true,
                    "tierAlias": "sonnet"
                },
                "claude-opus-5": {
                    "model": "claude-opus-5",
                    "tierAlias": "opus"
                }
            }
        });

        let specs = desktop_proxy_model_specs(Some(&meta), Some(&settings));
        let by_name: std::collections::HashMap<String, &GatewayModelSpec> =
            specs.iter().map(|spec| (spec.name.clone(), spec)).collect();

        let sonnet = by_name.get("claude-sonnet-5").expect("sonnet spec");
        assert!(sonnet.supports_1m);
        assert_eq!(sonnet.tier_alias.as_deref(), Some("sonnet"));
        assert_eq!(sonnet.label_override.as_deref(), Some("GLM 4.6"));

        let opus = by_name.get("claude-opus-5").expect("opus spec");
        assert!(!opus.supports_1m);
        assert_eq!(opus.tier_alias.as_deref(), Some("opus"));

        // Wire output: build_gateway_profile must emit anthropicFamilyTier +
        // supports1m on the object form (never the bare-string form).
        let profile = build_gateway_profile("https://gw", "key", Some(&specs));
        let models = profile["inferenceModels"]
            .as_array()
            .expect("inferenceModels array");
        let sonnet_obj = models
            .iter()
            .find(|item| item["name"] == json!("claude-sonnet-5"))
            .expect("sonnet entry");
        assert_eq!(sonnet_obj["supports1m"], json!(true));
        assert_eq!(sonnet_obj["anthropicFamilyTier"], json!("sonnet"));
        assert_eq!(sonnet_obj["labelOverride"], json!("GLM 4.6"));

        let opus_obj = models
            .iter()
            .find(|item| item["name"] == json!("claude-opus-5"))
            .expect("opus entry");
        assert_eq!(opus_obj["anthropicFamilyTier"], json!("opus"));
        // opus has no supports1m and no labelOverride, but tier_alias forces the
        // object form (not a bare string).
        assert!(opus_obj.get("supports1m").is_none());
    }

    #[test]
    fn proxy_specs_reject_invalid_tier_alias() {
        let settings = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://gateway.example.com",
                "ANTHROPIC_AUTH_TOKEN": "test-token"
            }
        });
        let meta = json!({
            "claude_desktop_model_routes": {
                "claude-sonnet-5": {
                    "model": "glm-4.6",
                    "tierAlias": "Opan" // typo, not a legal tier
                }
            }
        });
        let specs = desktop_proxy_model_specs(Some(&meta), Some(&settings));
        let sonnet = specs
            .iter()
            .find(|spec| spec.name == "claude-sonnet-5")
            .expect("sonnet spec");
        assert!(
            sonnet.tier_alias.is_none(),
            "invalid tier alias must be dropped"
        );
    }
}
