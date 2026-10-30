use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::coding::open_code::shell_env;
use crate::coding::{
    antigravity, claude_code, codebuddy, codex, dsh, gemini_cli, grok, hermes, kimi, oh_my_pi,
    omo_native, open_claw, open_code, pi, workbuddy, zcode,
};
use crate::db::helpers::{db_get, db_patch_fields};
use crate::db::schema::DbTable;

const MODULE_KEYS: [&str; 16] = [
    "opencode",
    "claude",
    "codex",
    "grok",
    "kimi",
    "openclaw",
    "geminicli",
    "antigravity",
    "zcode",
    "pi",
    "oh_my_pi",
    "omo_native",
    "hermes",
    "dsh",
    "workbuddy",
    "codebuddy",
];
const OMO_LEGACY_BASENAME: &str = "oh-my-opencode";
const OMO_CANONICAL_BASENAME: &str = "oh-my-openagent";
const OMO_UNIFIED_DIR: &str = ".omo";
const OMO_UNIFIED_BASENAME: &str = "omo";
pub const CODEX_DEFAULT_PROMPT_FILE_NAME: &str = "AGENTS.md";
pub const CODEX_OVERRIDE_PROMPT_FILE_NAME: &str = "AGENTS.override.md";
pub const CODEX_PROMPT_FILE_NAMES: [&str; 2] = [
    CODEX_DEFAULT_PROMPT_FILE_NAME,
    CODEX_OVERRIDE_PROMPT_FILE_NAME,
];

static RUNTIME_LOCATION_CACHE: LazyLock<RwLock<HashMap<&'static str, RuntimeLocationInfo>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static CLAUDE_PLUGINS_DIR_CACHE: LazyLock<RwLock<Option<PathBuf>>> =
    LazyLock::new(|| RwLock::new(None));

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeLocationMode {
    LocalWindows,
    WslDirect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslLocationInfo {
    pub distro: String,
    pub linux_path: String,
    pub linux_user_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLocationInfo {
    pub mode: RuntimeLocationMode,
    pub source: String,
    pub host_path: PathBuf,
    pub wsl: Option<WslLocationInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WslDirectModuleStatus {
    pub module: String,
    pub is_wsl_direct: bool,
    pub reason: Option<String>,
    pub source_path: Option<String>,
    pub linux_path: Option<String>,
    pub linux_user_root: Option<String>,
    pub distro: Option<String>,
}

pub fn is_wsl_unc_path(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    lower.starts_with("\\\\wsl\\")
        || lower.starts_with("\\\\wsl$\\")
        || lower.starts_with("\\\\wsl.localhost\\")
}

pub fn parse_wsl_unc_path(path: &str) -> Option<WslLocationInfo> {
    let trimmed = path.trim();
    if trimmed.is_empty() || !is_wsl_unc_path(trimmed) {
        return None;
    }

    let without_prefix = trimmed.trim_start_matches('\\');
    let mut segments = without_prefix
        .split('\\')
        .filter(|segment| !segment.is_empty());
    let host = segments.next()?.to_ascii_lowercase();
    if host != "wsl" && host != "wsl$" && host != "wsl.localhost" {
        return None;
    }

    let distro = segments.next()?.to_string();
    let linux_segments: Vec<String> = segments.map(|segment| segment.to_string()).collect();
    let linux_path = if linux_segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", linux_segments.join("/"))
    };

    let linux_user_root = detect_linux_user_root(&linux_path);

    Some(WslLocationInfo {
        distro,
        linux_path,
        linux_user_root,
    })
}

fn detect_linux_user_root(linux_path: &str) -> Option<String> {
    let segments: Vec<&str> = linux_path
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();

    if segments.len() >= 2 && segments[0] == "home" {
        return Some(format!("/home/{}", segments[1]));
    }

    if segments.first().copied() == Some("root") {
        return Some("/root".to_string());
    }

    None
}

pub fn build_windows_unc_path(distro: &str, linux_path: &str) -> PathBuf {
    let normalized_linux_path = linux_path.trim();
    let suffix = normalized_linux_path
        .trim_start_matches('/')
        .replace('/', "\\");
    if suffix.is_empty() {
        PathBuf::from(format!("\\\\wsl.localhost\\{}", distro))
    } else {
        PathBuf::from(format!("\\\\wsl.localhost\\{}\\{}", distro, suffix))
    }
}

pub fn expand_home_from_user_root(linux_user_root: Option<&str>, candidate: &str) -> String {
    if candidate == "~" {
        return linux_user_root.unwrap_or("~").to_string();
    }

    if let Some(rest) = candidate.strip_prefix("~/") {
        return match linux_user_root {
            Some(root) => format!("{}/{}", root.trim_end_matches('/'), rest),
            None => candidate.to_string(),
        };
    }

    candidate.to_string()
}

fn build_wsl_reason(_module: &str, _source_path: &str, _distro: &str) -> String {
    "wsl_direct_config_path".to_string()
}

pub fn module_status_from_location(
    module: &str,
    location: &RuntimeLocationInfo,
) -> WslDirectModuleStatus {
    match &location.wsl {
        Some(wsl) => WslDirectModuleStatus {
            module: module.to_string(),
            is_wsl_direct: true,
            reason: Some(build_wsl_reason(
                module,
                &location.host_path.to_string_lossy(),
                &wsl.distro,
            )),
            source_path: Some(location.host_path.to_string_lossy().to_string()),
            linux_path: Some(wsl.linux_path.clone()),
            linux_user_root: wsl.linux_user_root.clone(),
            distro: Some(wsl.distro.clone()),
        },
        None => WslDirectModuleStatus {
            module: module.to_string(),
            is_wsl_direct: false,
            reason: None,
            source_path: Some(location.host_path.to_string_lossy().to_string()),
            linux_path: None,
            linux_user_root: None,
            distro: None,
        },
    }
}

fn normalize_module_key(module: &str) -> Option<&'static str> {
    match module {
        "opencode" => Some("opencode"),
        "claude" | "claude_code" => Some("claude"),
        "codex" => Some("codex"),
        "grok" | "grok_cli" => Some("grok"),
        "kimi" | "kimi_cli" => Some("kimi"),
        "openclaw" => Some("openclaw"),
        "geminicli" | "gemini_cli" | "gemini" => Some("geminicli"),
        "antigravity" | "antigravity_cli" => Some("antigravity"),
        "zcode" | "zcode_cli" => Some("zcode"),
        "pi" => Some("pi"),
        "oh_my_pi" | "omp" => Some("oh_my_pi"),
        // OmO Native（`omo` 二进制 / senpi 引擎）与 OpenCode 插件版 OMO 是两套 edition，
        // 但共用 `~/.omo/omo.jsonc` 与 `~/.omo` 目录；模块 key 必须与插件版区分。
        "omo_native" | "omo" => Some("omo_native"),
        "hermes" => Some("hermes"),
        "dsh" => Some("dsh"),
        "workbuddy" => Some("workbuddy"),
        "codebuddy" => Some("codebuddy"),
        _ => None,
    }
}

fn get_cached_runtime_location(module: &str) -> Option<RuntimeLocationInfo> {
    let module = normalize_module_key(module)?;
    RUNTIME_LOCATION_CACHE
        .read()
        .ok()
        .and_then(|cache| cache.get(module).cloned())
}

fn set_cached_runtime_location(module: &'static str, location: RuntimeLocationInfo) {
    if let Ok(mut cache) = RUNTIME_LOCATION_CACHE.write() {
        cache.insert(module, location);
    }
}

fn get_cached_claude_plugins_dir() -> Option<PathBuf> {
    CLAUDE_PLUGINS_DIR_CACHE
        .read()
        .ok()
        .and_then(|cache| cache.clone())
}

fn set_cached_claude_plugins_dir(path: PathBuf) {
    if let Ok(mut cache) = CLAUDE_PLUGINS_DIR_CACHE.write() {
        *cache = Some(path);
    }
}

fn get_cached_or_fallback_runtime_location(module: &str) -> RuntimeLocationInfo {
    get_cached_runtime_location(module).unwrap_or_else(|| get_runtime_location_without_db(module))
}

async fn get_cached_or_refresh_runtime_location_async(
    db: &crate::db::SqliteDbState,
    module: &str,
) -> Result<RuntimeLocationInfo, String> {
    match get_cached_runtime_location(module) {
        Some(location) => Ok(location),
        None => refresh_runtime_location_cache_for_module_async(db, module).await,
    }
}

#[cfg(test)]
fn clear_runtime_location_cache() {
    if let Ok(mut cache) = RUNTIME_LOCATION_CACHE.write() {
        cache.clear();
    }
    if let Ok(mut cache) = CLAUDE_PLUGINS_DIR_CACHE.write() {
        *cache = None;
    }
}

pub async fn refresh_runtime_location_cache_for_module_async(
    db: &crate::db::SqliteDbState,
    module: &str,
) -> Result<RuntimeLocationInfo, String> {
    match normalize_module_key(module) {
        Some("opencode") => {
            let location = resolve_opencode_runtime_location_uncached_async(db).await?;
            set_cached_runtime_location("opencode", location.clone());
            Ok(location)
        }
        Some("claude") => {
            let location = resolve_claude_runtime_location_uncached_async(db).await?;
            let plugins_dir = resolve_claude_plugins_dir_uncached(&location);
            set_cached_runtime_location("claude", location.clone());
            set_cached_claude_plugins_dir(plugins_dir);
            Ok(location)
        }
        Some("codex") => {
            let location = resolve_codex_runtime_location_uncached_async(db).await?;
            set_cached_runtime_location("codex", location.clone());
            Ok(location)
        }
        Some("grok") => {
            let location = resolve_grok_runtime_location_uncached_async(db).await?;
            set_cached_runtime_location("grok", location.clone());
            Ok(location)
        }
        Some("kimi") => {
            let location = resolve_kimi_runtime_location_uncached_async(db).await?;
            set_cached_runtime_location("kimi", location.clone());
            Ok(location)
        }
        Some("workbuddy") => {
            let location = resolve_workbuddy_runtime_location_uncached_async(db).await?;
            set_cached_runtime_location("workbuddy", location.clone());
            Ok(location)
        }
        Some("codebuddy") => {
            let location = resolve_codebuddy_runtime_location_uncached_async(db).await?;
            set_cached_runtime_location("codebuddy", location.clone());
            Ok(location)
        }
        Some("openclaw") => {
            let location = resolve_openclaw_runtime_location_uncached_async(db).await?;
            set_cached_runtime_location("openclaw", location.clone());
            Ok(location)
        }
        Some("geminicli") => {
            let location = resolve_gemini_cli_runtime_location_uncached_async(db).await?;
            set_cached_runtime_location("geminicli", location.clone());
            Ok(location)
        }
        Some("antigravity") => {
            let location = resolve_antigravity_runtime_location_uncached_async(db).await?;
            set_cached_runtime_location("antigravity", location.clone());
            Ok(location)
        }
        Some("zcode") => {
            let location = resolve_zcode_runtime_location_uncached_async(db).await?;
            set_cached_runtime_location("zcode", location.clone());
            Ok(location)
        }
        Some("pi") => {
            let location = resolve_pi_runtime_location_uncached_async(db).await?;
            set_cached_runtime_location("pi", location.clone());
            Ok(location)
        }
        Some("oh_my_pi") => {
            let location = resolve_oh_my_pi_runtime_location_uncached_async(db).await?;
            set_cached_runtime_location("oh_my_pi", location.clone());
            Ok(location)
        }
        Some("omo_native") => {
            let location = resolve_omo_native_runtime_location_uncached_async(db).await?;
            set_cached_runtime_location("omo_native", location.clone());
            Ok(location)
        }
        Some("hermes") => {
            let (path, source) = hermes::get_hermes_config_dir_from_db_async(db).await?;
            let location = build_runtime_location(path, source);
            set_cached_runtime_location("hermes", location.clone());
            Ok(location)
        }
        Some("dsh") => {
            let (path, source) = dsh::get_dsh_config_dir_from_db_async(db).await?;
            let location = build_runtime_location(path, source);
            set_cached_runtime_location("dsh", location.clone());
            Ok(location)
        }
        Some(_) | None => Err(format!("Unsupported runtime module: {}", module)),
    }
}

pub async fn refresh_runtime_location_cache_async(
    db: &crate::db::SqliteDbState,
) -> Result<(), String> {
    for module in MODULE_KEYS {
        refresh_runtime_location_cache_for_module_async(db, module).await?;
    }

    Ok(())
}

pub fn get_wsl_direct_status_map(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<WslDirectModuleStatus>, String> {
    let _ = db;
    Ok(MODULE_KEYS
        .iter()
        .map(|module| {
            module_status_from_location(module, &get_cached_or_fallback_runtime_location(module))
        })
        .collect())
}

fn module_status_from_runtime_result(
    module: &str,
    runtime_result: Result<RuntimeLocationInfo, String>,
    fallback_location: &RuntimeLocationInfo,
) -> WslDirectModuleStatus {
    match runtime_result {
        Ok(location) => module_status_from_location(module, &location),
        Err(error) => {
            log::warn!(
                "Failed to resolve runtime location for module '{}' while building WSL direct status: {}. Falling back to non-database runtime resolution.",
                module,
                error
            );
            module_status_from_location(module, fallback_location)
        }
    }
}

async fn get_wsl_direct_status_with_fallback(
    db: &crate::db::SqliteDbState,
    module: &str,
) -> Result<WslDirectModuleStatus, String> {
    if !MODULE_KEYS.contains(&module) {
        return Err(format!("Unsupported runtime module: {}", module));
    }

    let fallback_location = get_cached_or_fallback_runtime_location(module);
    let runtime_result = match get_cached_runtime_location(module) {
        Some(location) => Ok(location),
        None => refresh_runtime_location_cache_for_module_async(db, module).await,
    };

    Ok(module_status_from_runtime_result(
        module,
        runtime_result,
        &fallback_location,
    ))
}

pub async fn get_wsl_direct_status_map_async(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<WslDirectModuleStatus>, String> {
    let mut statuses = Vec::with_capacity(MODULE_KEYS.len());

    for module in MODULE_KEYS {
        statuses.push(get_wsl_direct_status_with_fallback(db, module).await?);
    }

    Ok(statuses)
}

pub async fn get_wsl_direct_modules_async(
    db: &crate::db::SqliteDbState,
) -> Result<HashSet<String>, String> {
    Ok(get_wsl_direct_status_map_async(db)
        .await?
        .into_iter()
        .filter(|status| status.is_wsl_direct)
        .map(|status| status.module)
        .collect())
}

pub fn get_wsl_direct_status_for_module(
    db: &crate::db::SqliteDbState,
    module: &str,
) -> Result<WslDirectModuleStatus, String> {
    let _ = db;
    match normalize_module_key(module) {
        Some(module_key) => Ok(module_status_from_location(
            module_key,
            &get_cached_or_fallback_runtime_location(module_key),
        )),
        None => Err(format!("Unsupported runtime module: {}", module)),
    }
}

pub async fn get_wsl_direct_status_for_module_async(
    db: &crate::db::SqliteDbState,
    module: &str,
) -> Result<WslDirectModuleStatus, String> {
    get_wsl_direct_status_with_fallback(db, module).await
}

pub fn get_opencode_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("opencode"))
}

/// The cached OpenCode location without a database handle, for callers that
/// only have synchronous helpers (auth/config readers). Falls back to the
/// env/default resolution when the startup cache has not been filled yet.
pub fn get_opencode_runtime_location_cached() -> RuntimeLocationInfo {
    get_cached_or_fallback_runtime_location("opencode")
}

pub async fn get_opencode_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "opencode").await
}

pub fn get_opencode_config_dir_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    get_opencode_runtime_location_sync(db)?
        .host_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Failed to determine OpenCode config directory".to_string())
}

pub async fn get_opencode_config_dir_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    get_opencode_runtime_location_async(db)
        .await?
        .host_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Failed to determine OpenCode config directory".to_string())
}

pub fn get_opencode_prompt_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_opencode_config_dir_sync(db)?.join("AGENTS.md"))
}

pub async fn get_opencode_prompt_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_opencode_config_dir_async(db).await?.join("AGENTS.md"))
}

pub async fn get_omo_config_path_async(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    if !uses_legacy_omo_config_async(db).await {
        return get_unified_omo_config_path_async(db).await;
    }
    let dir = get_opencode_config_dir_async(db).await?;
    Ok(resolve_omo_config_path_from_dir(&dir))
}

/// Whether the OMO runtime config should use the legacy flat file
/// (~/.config/opencode/oh-my-openagent.jsonc) instead of the unified
/// ~/.omo/omo.jsonc ([opencode] block) format. Driven by the
/// `opencode_use_legacy_oh_my_config` app setting.
pub fn uses_legacy_omo_config(db: &crate::db::SqliteDbState) -> bool {
    crate::settings::store::load_settings_from_sqlite_state(db)
        .map(|settings| settings.opencode_use_legacy_oh_my_config)
        .unwrap_or(false)
}

pub async fn uses_legacy_omo_config_async(db: &crate::db::SqliteDbState) -> bool {
    crate::settings::store::load_settings_from_sqlite_state_async(db)
        .await
        .map(|settings| settings.opencode_use_legacy_oh_my_config)
        .unwrap_or(false)
}

/// Resolve the unified `~/.omo/omo.jsonc` path, honoring a WSL Direct OpenCode
/// runtime. When opencode runs inside WSL, the plugin reads the WSL home's
/// `~/.omo/omo.jsonc`, so the path must be a UNC path derived from the WSL user
/// root (`get_opencode_runtime_location_async`). Otherwise fall back to the
/// local host home.
async fn get_unified_omo_config_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    if let Ok(location) = get_opencode_runtime_location_async(db).await {
        if let Some(wsl) = &location.wsl {
            let linux_config =
                expand_home_from_user_root(wsl.linux_user_root.as_deref(), "~/.omo/omo.jsonc");
            return Ok(build_windows_unc_path(&wsl.distro, &linux_config));
        }
    }
    resolve_unified_omo_config_path()
}

fn resolve_unified_omo_config_path() -> Result<PathBuf, String> {
    let home = get_home_dir()?;
    let dir = home.join(OMO_UNIFIED_DIR);
    let jsonc = dir.join(format!("{OMO_UNIFIED_BASENAME}.jsonc"));
    let json = dir.join(format!("{OMO_UNIFIED_BASENAME}.json"));
    Ok(if jsonc.exists() {
        jsonc
    } else if json.exists() {
        json
    } else {
        jsonc
    })
}

fn resolve_omo_config_path_from_dir(dir: &Path) -> PathBuf {
    let candidates = [
        dir.join(format!("{OMO_CANONICAL_BASENAME}.jsonc")),
        dir.join(format!("{OMO_CANONICAL_BASENAME}.json")),
        dir.join(format!("{OMO_LEGACY_BASENAME}.jsonc")),
        dir.join(format!("{OMO_LEGACY_BASENAME}.json")),
    ];

    candidates
        .into_iter()
        .find(|path| path.exists())
        .unwrap_or_else(|| dir.join(format!("{OMO_CANONICAL_BASENAME}.jsonc")))
}

pub fn get_omos_config_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_opencode_config_dir_sync(db)?.join("oh-my-opencode-slim.json"))
}

pub async fn get_omos_config_path_async(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_opencode_config_dir_async(db)
        .await?
        .join("oh-my-opencode-slim.json"))
}

pub fn get_opencode_wsl_target_path(db: &crate::db::SqliteDbState) -> String {
    get_opencode_runtime_location_sync(db)
        .ok()
        .and_then(|location| {
            location.wsl.map(|wsl| wsl.linux_path).or_else(|| {
                location
                    .host_path
                    .file_name()
                    .map(|name| format!("~/.config/opencode/{}", name.to_string_lossy()))
            })
        })
        .unwrap_or_else(|| "~/.config/opencode/opencode.jsonc".to_string())
}

pub async fn get_opencode_wsl_target_path_async(db: &crate::db::SqliteDbState) -> String {
    get_opencode_runtime_location_async(db)
        .await
        .ok()
        .and_then(|location| {
            location.wsl.map(|wsl| wsl.linux_path).or_else(|| {
                location
                    .host_path
                    .file_name()
                    .map(|name| format!("~/.config/opencode/{}", name.to_string_lossy()))
            })
        })
        .unwrap_or_else(|| "~/.config/opencode/opencode.jsonc".to_string())
}

pub fn get_opencode_prompt_wsl_target_path(db: &crate::db::SqliteDbState) -> String {
    get_opencode_runtime_location_sync(db)
        .ok()
        .and_then(|location| {
            location.wsl.map(|wsl| {
                let prompt_path = format!(
                    "{}/AGENTS.md",
                    wsl.linux_path
                        .rsplit_once('/')
                        .map(|(parent, _)| parent)
                        .unwrap_or("/")
                        .trim_end_matches('/')
                );
                if prompt_path.starts_with("//") {
                    "/AGENTS.md".to_string()
                } else {
                    prompt_path
                }
            })
        })
        .unwrap_or_else(|| "~/.config/opencode/AGENTS.md".to_string())
}

pub async fn get_opencode_prompt_wsl_target_path_async(db: &crate::db::SqliteDbState) -> String {
    get_opencode_runtime_location_async(db)
        .await
        .ok()
        .and_then(|location| {
            location.wsl.map(|wsl| {
                let prompt_path = format!(
                    "{}/AGENTS.md",
                    wsl.linux_path
                        .rsplit_once('/')
                        .map(|(parent, _)| parent)
                        .unwrap_or("/")
                        .trim_end_matches('/')
                );
                if prompt_path.starts_with("//") {
                    "/AGENTS.md".to_string()
                } else {
                    prompt_path
                }
            })
        })
        .unwrap_or_else(|| "~/.config/opencode/AGENTS.md".to_string())
}

pub fn get_claude_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("claude"))
}

pub async fn get_claude_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "claude").await
}

async fn resolve_claude_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let path_info =
        get_custom_path_from_record(db, DbTable::ClaudeCommonConfig, "common", |value| {
            crate::coding::claude_code::adapter::from_db_value_common(value)
                .root_dir
                .filter(|path| !path.trim().is_empty())
        })
        .await;

    let (path, source) = if let Some(path) = path_info {
        (PathBuf::from(path), "custom".to_string())
    } else if let Ok(env_path) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !env_path.trim().is_empty() {
            (PathBuf::from(env_path), "env".to_string())
        } else if let Some(shell_path) = shell_env::get_env_from_shell_config("CLAUDE_CONFIG_DIR") {
            (PathBuf::from(shell_path), "shell".to_string())
        } else {
            (
                claude_code::get_claude_default_root_dir()?,
                "default".to_string(),
            )
        }
    } else if let Some(shell_path) = shell_env::get_env_from_shell_config("CLAUDE_CONFIG_DIR") {
        (PathBuf::from(shell_path), "shell".to_string())
    } else {
        (
            claude_code::get_claude_default_root_dir()?,
            "default".to_string(),
        )
    };

    Ok(build_runtime_location(path, source))
}

pub fn get_claude_settings_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_claude_runtime_location_sync(db)?
        .host_path
        .join("settings.json"))
}

pub async fn get_claude_settings_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_claude_runtime_location_async(db)
        .await?
        .host_path
        .join("settings.json"))
}

pub fn get_claude_plugin_config_path_sync(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_claude_runtime_location_sync(db)?
        .host_path
        .join("config.json"))
}

pub async fn get_claude_plugin_config_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_claude_runtime_location_async(db)
        .await?
        .host_path
        .join("config.json"))
}

pub fn get_claude_plugins_dir_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    if let Some(path) = get_cached_claude_plugins_dir() {
        return Ok(path);
    }

    Ok(get_claude_runtime_location_sync(db)?
        .host_path
        .join("plugins"))
}

pub async fn get_claude_plugins_dir_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    if let Some(path) = get_cached_claude_plugins_dir() {
        return Ok(path);
    }

    let location = get_claude_runtime_location_async(db).await?;
    let plugins_dir = resolve_claude_plugins_dir_uncached(&location);
    set_cached_claude_plugins_dir(plugins_dir.clone());
    Ok(plugins_dir)
}

pub fn get_claude_prompt_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_claude_runtime_location_sync(db)?
        .host_path
        .join("CLAUDE.md"))
}

pub async fn get_claude_prompt_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_claude_runtime_location_async(db)
        .await?
        .host_path
        .join("CLAUDE.md"))
}

pub fn get_claude_mcp_config_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    let location = get_claude_runtime_location_sync(db)?;
    get_claude_mcp_config_path_from_location(&location)
}

pub async fn get_claude_mcp_config_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    let location = get_claude_runtime_location_async(db).await?;
    get_claude_mcp_config_path_from_location(&location)
}

fn get_claude_mcp_config_path_from_location(
    location: &RuntimeLocationInfo,
) -> Result<PathBuf, String> {
    if let Some(wsl) = &location.wsl {
        let linux_config_root = if location.source == "default" {
            wsl.linux_user_root
                .as_deref()
                .unwrap_or_else(|| wsl.linux_path.as_str())
        } else {
            wsl.linux_path.as_str()
        };
        Ok(build_windows_unc_path(
            &wsl.distro,
            &format!("{}/.claude.json", linux_config_root.trim_end_matches('/')),
        ))
    } else if location.source == "default" {
        Ok(get_home_dir()?.join(".claude.json"))
    } else {
        Ok(location.host_path.join(".claude.json"))
    }
}

fn resolve_claude_plugins_dir_uncached(location: &RuntimeLocationInfo) -> PathBuf {
    if let Ok(env_path) = std::env::var("CLAUDE_CODE_PLUGIN_CACHE_DIR") {
        if !env_path.trim().is_empty() {
            return PathBuf::from(env_path);
        }
    }

    if let Some(shell_path) = shell_env::get_env_from_shell_config("CLAUDE_CODE_PLUGIN_CACHE_DIR") {
        if !shell_path.trim().is_empty() {
            return PathBuf::from(shell_path);
        }
    }

    location.host_path.join("plugins")
}

fn get_home_dir() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .map_err(|_| "Failed to get home directory".to_string())
}

pub fn get_claude_wsl_target_path(db: &crate::db::SqliteDbState, file_name: &str) -> String {
    match get_claude_runtime_location_sync(db) {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.claude/{}", file_name)),
        Err(_) => format!("~/.claude/{}", file_name),
    }
}

pub async fn get_claude_wsl_target_path_async(
    db: &crate::db::SqliteDbState,
    file_name: &str,
) -> String {
    match get_claude_runtime_location_async(db).await {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.claude/{}", file_name)),
        Err(_) => format!("~/.claude/{}", file_name),
    }
}

pub fn get_claude_wsl_claude_json_path(db: &crate::db::SqliteDbState) -> String {
    match get_claude_runtime_location_sync(db) {
        Ok(location) => get_claude_wsl_claude_json_path_from_location(&location)
            .unwrap_or_else(|| "~/.claude.json".to_string()),
        Err(_) => "~/.claude.json".to_string(),
    }
}

pub async fn get_claude_wsl_claude_json_path_async(db: &crate::db::SqliteDbState) -> String {
    match get_claude_runtime_location_async(db).await {
        Ok(location) => get_claude_wsl_claude_json_path_from_location(&location)
            .unwrap_or_else(|| "~/.claude.json".to_string()),
        Err(_) => "~/.claude.json".to_string(),
    }
}

fn get_claude_wsl_claude_json_path_from_location(location: &RuntimeLocationInfo) -> Option<String> {
    let wsl = location.wsl.as_ref()?;
    let linux_config_root = if location.source == "default" {
        wsl.linux_user_root.as_deref()?
    } else {
        wsl.linux_path.as_str()
    };

    Some(format!(
        "{}/.claude.json",
        linux_config_root.trim_end_matches('/')
    ))
}

pub fn get_codex_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("codex"))
}

pub async fn get_codex_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "codex").await
}

async fn resolve_codex_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let path_info =
        get_custom_path_from_record(db, DbTable::CodexCommonConfig, "common", |value| {
            crate::coding::codex::adapter::from_db_value_common(value)
                .root_dir
                .filter(|path| !path.trim().is_empty())
        })
        .await;

    let (path, source) = if let Some(path) = path_info {
        (PathBuf::from(path), "custom".to_string())
    } else if let Ok(env_path) = std::env::var("CODEX_HOME") {
        if !env_path.trim().is_empty() {
            (PathBuf::from(env_path), "env".to_string())
        } else if let Some(shell_path) = shell_env::get_env_from_shell_config("CODEX_HOME") {
            (PathBuf::from(shell_path), "shell".to_string())
        } else {
            (codex::get_codex_default_root_dir()?, "default".to_string())
        }
    } else if let Some(shell_path) = shell_env::get_env_from_shell_config("CODEX_HOME") {
        (PathBuf::from(shell_path), "shell".to_string())
    } else {
        (codex::get_codex_default_root_dir()?, "default".to_string())
    };

    Ok(build_runtime_location(path, source))
}

pub fn get_codex_auth_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_codex_runtime_location_sync(db)?
        .host_path
        .join("auth.json"))
}

pub async fn get_codex_auth_path_async(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_codex_runtime_location_async(db)
        .await?
        .host_path
        .join("auth.json"))
}

pub fn get_codex_config_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_codex_runtime_location_sync(db)?
        .host_path
        .join("config.toml"))
}

pub async fn get_codex_config_path_async(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_codex_runtime_location_async(db)
        .await?
        .host_path
        .join("config.toml"))
}

fn prompt_file_has_content(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|content| !content.trim().is_empty())
        .unwrap_or(false)
}

pub fn resolve_codex_prompt_file_path(root_path: &Path) -> PathBuf {
    let override_path = root_path.join(CODEX_OVERRIDE_PROMPT_FILE_NAME);
    if prompt_file_has_content(&override_path) {
        return override_path;
    }

    let default_path = root_path.join(CODEX_DEFAULT_PROMPT_FILE_NAME);
    if prompt_file_has_content(&default_path) {
        return default_path;
    }

    if override_path.exists() {
        return override_path;
    }

    default_path
}

pub fn replace_path_file_name(path: &str, file_name: &str) -> String {
    let split_index = path
        .rfind(|ch| ch == '/' || ch == '\\')
        .map(|index| index + 1)
        .unwrap_or(0);
    format!("{}{}", &path[..split_index], file_name)
}

pub fn get_codex_prompt_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(resolve_codex_prompt_file_path(
        &get_codex_runtime_location_sync(db)?.host_path,
    ))
}

pub async fn get_codex_prompt_path_async(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(resolve_codex_prompt_file_path(
        &get_codex_runtime_location_async(db).await?.host_path,
    ))
}

pub fn get_codex_wsl_target_path(db: &crate::db::SqliteDbState, file_name: &str) -> String {
    match get_codex_runtime_location_sync(db) {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.codex/{}", file_name)),
        Err(_) => format!("~/.codex/{}", file_name),
    }
}

pub async fn get_codex_wsl_target_path_async(
    db: &crate::db::SqliteDbState,
    file_name: &str,
) -> String {
    match get_codex_runtime_location_async(db).await {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.codex/{}", file_name)),
        Err(_) => format!("~/.codex/{}", file_name),
    }
}

pub fn get_grok_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("grok"))
}

pub async fn get_grok_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "grok").await
}

async fn resolve_grok_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let custom_path =
        get_custom_path_from_record(db, DbTable::GrokCommonConfig, "common", |value| {
            crate::coding::grok::adapter::common_from_db_value(value)
                .root_dir
                .filter(|path| !path.trim().is_empty())
        })
        .await;
    let (path, source) = if let Some(path) = custom_path {
        (PathBuf::from(path), "custom".to_string())
    } else {
        resolve_grok_path_without_db()
    };
    Ok(build_runtime_location(path, source))
}

pub fn get_grok_config_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_grok_runtime_location_sync(db)?
        .host_path
        .join(grok::constants::GROK_CONFIG_FILE))
}

pub async fn get_grok_config_path_async(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_grok_runtime_location_async(db)
        .await?
        .host_path
        .join(grok::constants::GROK_CONFIG_FILE))
}

pub fn get_grok_auth_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_grok_runtime_location_sync(db)?
        .host_path
        .join(grok::constants::GROK_AUTH_FILE))
}

pub async fn get_grok_auth_path_async(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_grok_runtime_location_async(db)
        .await?
        .host_path
        .join(grok::constants::GROK_AUTH_FILE))
}

pub fn get_grok_prompt_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_grok_runtime_location_sync(db)?
        .host_path
        .join(grok::constants::GROK_PROMPT_FILE))
}

pub async fn get_grok_prompt_path_async(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_grok_runtime_location_async(db)
        .await?
        .host_path
        .join(grok::constants::GROK_PROMPT_FILE))
}

pub async fn get_grok_wsl_target_path_async(
    db: &crate::db::SqliteDbState,
    file_name: &str,
) -> String {
    match get_grok_runtime_location_async(db).await {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.grok/{file_name}")),
        Err(_) => format!("~/.grok/{file_name}"),
    }
}

pub fn get_zcode_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("zcode"))
}

pub async fn get_zcode_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "zcode").await
}

async fn resolve_zcode_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let custom_path =
        get_custom_path_from_record(db, DbTable::ZcodeCommonConfig, "common", |value| {
            crate::coding::zcode::adapter::from_db_value_common(value)
                .root_dir
                .filter(|path| !path.trim().is_empty())
        })
        .await;
    let (path, source) = if let Some(path) = custom_path {
        (PathBuf::from(path), "custom".to_string())
    } else {
        resolve_zcode_path_without_db()
    };
    Ok(build_runtime_location(path, source))
}

/// Resolves the ZCode data root without database access.
///
/// Precedence mirrors what the ZCode runtime itself honors: the
/// `ZCODE_DATA_BASE_DIR` process override, the same key exported from the
/// user's shell profile, then the desktop app's own `setting.json` record.
/// The `.zcode` suffix is appended only for the base-directory overrides,
/// because those name the parent directory rather than the data root.
/// ZCode's data root, resolved from the environment and on-disk settings only.
///
/// Exposed for the backup restore path, which runs before the database is
/// available.
pub fn resolve_zcode_root_dir_without_db() -> PathBuf {
    resolve_zcode_path_without_db().0
}

fn resolve_zcode_path_without_db() -> (PathBuf, String) {
    let env_key = zcode::constants::ZCODE_DATA_BASE_DIR_ENV;
    if let Ok(path) = std::env::var(env_key) {
        if !path.trim().is_empty() {
            return (
                PathBuf::from(path.trim()).join(zcode::constants::ZCODE_DEFAULT_ROOT_DIR_NAME),
                "env".to_string(),
            );
        }
    }
    if let Some(path) = shell_env::get_env_from_shell_config(env_key) {
        if !path.trim().is_empty() {
            return (
                PathBuf::from(path.trim()).join(zcode::constants::ZCODE_DEFAULT_ROOT_DIR_NAME),
                "shell".to_string(),
            );
        }
    }
    if let Some(path) = zcode_data_base_dir_from_setting_file() {
        return (path, "setting".to_string());
    }
    (
        dirs::home_dir()
            .map(|home| home.join(zcode::constants::ZCODE_DEFAULT_ROOT_DIR_NAME))
            .unwrap_or_else(|| PathBuf::from("~/.zcode")),
        "default".to_string(),
    )
}

/// Reads `dataBaseDir` from the desktop settings file.
///
/// `setting.json` always lives directly under the home directory; only the
/// data it points at moves, so this lookup never depends on the resolved root.
fn zcode_data_base_dir_from_setting_file() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let path = home
        .join(zcode::constants::ZCODE_DEFAULT_ROOT_DIR_NAME)
        .join(zcode::constants::ZCODE_SETTING_RELATIVE_PATH);
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let base = value.get("dataBaseDir")?.as_str()?.trim();
    if base.is_empty() {
        return None;
    }
    Some(PathBuf::from(base).join(zcode::constants::ZCODE_DEFAULT_ROOT_DIR_NAME))
}

pub fn get_zcode_config_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_zcode_runtime_location_sync(db)?
        .host_path
        .join(zcode::constants::ZCODE_PROVIDER_CONFIG_RELATIVE_PATH))
}

pub async fn get_zcode_config_path_async(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_zcode_runtime_location_async(db)
        .await?
        .host_path
        .join(zcode::constants::ZCODE_PROVIDER_CONFIG_RELATIVE_PATH))
}

/// ZCode reads MCP servers from the CLI config, not the provider registry.
pub fn get_zcode_mcp_config_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_zcode_runtime_location_sync(db)?
        .host_path
        .join(zcode::constants::ZCODE_CLI_CONFIG_RELATIVE_PATH))
}

pub async fn get_zcode_mcp_config_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_zcode_runtime_location_async(db)
        .await?
        .host_path
        .join(zcode::constants::ZCODE_CLI_CONFIG_RELATIVE_PATH))
}

pub fn get_zcode_prompt_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_zcode_runtime_location_sync(db)?
        .host_path
        .join(zcode::constants::ZCODE_PROMPT_FILE_NAME))
}

pub async fn get_zcode_prompt_path_async(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_zcode_runtime_location_async(db)
        .await?
        .host_path
        .join(zcode::constants::ZCODE_PROMPT_FILE_NAME))
}

pub async fn get_zcode_wsl_target_path_async(
    db: &crate::db::SqliteDbState,
    file_name: &str,
) -> String {
    match get_zcode_runtime_location_async(db).await {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.zcode/{file_name}")),
        Err(_) => format!("~/.zcode/{file_name}"),
    }
}

pub fn get_kimi_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("kimi"))
}

pub async fn get_kimi_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "kimi").await
}

async fn resolve_kimi_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let custom_path =
        get_custom_path_from_record(db, DbTable::KimiCommonConfig, "common", |value| {
            crate::coding::kimi::adapter::common_from_db_value(value)
                .root_dir
                .filter(|path| !path.trim().is_empty())
        })
        .await;
    let (path, source) = if let Some(path) = custom_path {
        (PathBuf::from(path), "custom".to_string())
    } else {
        resolve_kimi_path_without_db()
    };
    Ok(build_runtime_location(path, source))
}

fn resolve_kimi_path_without_db() -> (PathBuf, String) {
    if let Ok(path) = std::env::var(kimi::constants::KIMI_HOME_ENV_KEY) {
        if !path.trim().is_empty() {
            return (PathBuf::from(path), "env".to_string());
        }
    }
    if let Some(path) = shell_env::get_env_from_shell_config(kimi::constants::KIMI_HOME_ENV_KEY) {
        if !path.trim().is_empty() {
            return (PathBuf::from(path), "shell".to_string());
        }
    }
    (
        kimi::commands::get_kimi_default_root_dir()
            .unwrap_or_else(|_| PathBuf::from("~/.kimi-code")),
        "default".to_string(),
    )
}

pub fn get_kimi_config_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_kimi_runtime_location_sync(db)?
        .host_path
        .join(kimi::constants::KIMI_CONFIG_FILE))
}

pub async fn get_kimi_config_path_async(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_kimi_runtime_location_async(db)
        .await?
        .host_path
        .join(kimi::constants::KIMI_CONFIG_FILE))
}

/// Kimi Code CLI reads MCP servers from `<root>/mcp.json`, not config.toml.
pub fn get_kimi_mcp_config_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_kimi_runtime_location_sync(db)?
        .host_path
        .join(kimi::constants::KIMI_MCP_CONFIG_FILE))
}

pub async fn get_kimi_mcp_config_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_kimi_runtime_location_async(db)
        .await?
        .host_path
        .join(kimi::constants::KIMI_MCP_CONFIG_FILE))
}

pub fn get_kimi_prompt_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_kimi_runtime_location_sync(db)?
        .host_path
        .join(kimi::constants::KIMI_PROMPT_FILE))
}

pub async fn get_kimi_prompt_path_async(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_kimi_runtime_location_async(db)
        .await?
        .host_path
        .join(kimi::constants::KIMI_PROMPT_FILE))
}

pub async fn get_kimi_wsl_target_path_async(
    db: &crate::db::SqliteDbState,
    file_name: &str,
) -> String {
    match get_kimi_runtime_location_async(db).await {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.kimi-code/{file_name}")),
        Err(_) => format!("~/.kimi-code/{file_name}"),
    }
}

pub fn get_workbuddy_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("workbuddy"))
}

pub async fn get_workbuddy_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "workbuddy").await
}

async fn resolve_workbuddy_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let custom_path = get_custom_path_from_record(
        db,
        DbTable::WorkbuddySettingsConfig,
        workbuddy::constants::WORKBUDDY_SETTINGS_CONFIG_ID,
        |value| {
            workbuddy::adapter::settings_from_db_value(value.clone())
                .config_dir
                .filter(|path| !path.trim().is_empty())
        },
    )
    .await;
    let (path, source) = if let Some(path) = custom_path {
        (PathBuf::from(path), "custom".to_string())
    } else {
        workbuddy::paths::resolve_path_without_db()
    };
    Ok(build_runtime_location(path, source))
}

pub fn get_codebuddy_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("codebuddy"))
}

pub async fn get_codebuddy_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "codebuddy").await
}

async fn resolve_codebuddy_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let custom_path = get_custom_path_from_record(
        db,
        DbTable::CodebuddySettingsConfig,
        codebuddy::constants::CODEBUDDY_SETTINGS_CONFIG_ID,
        |value| {
            codebuddy::adapter::settings_from_db_value(value.clone())
                .config_dir
                .filter(|path| !path.trim().is_empty())
        },
    )
    .await;
    let (path, source) = if let Some(path) = custom_path {
        (PathBuf::from(path), "custom".to_string())
    } else {
        codebuddy::paths::resolve_path_without_db()
    };
    Ok(build_runtime_location(path, source))
}

pub fn get_gemini_cli_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("geminicli"))
}

pub async fn get_gemini_cli_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "geminicli").await
}

pub fn get_pi_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("pi"))
}

pub async fn get_pi_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "pi").await
}

pub fn get_oh_my_pi_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("oh_my_pi"))
}

pub async fn get_oh_my_pi_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "oh_my_pi").await
}

async fn resolve_oh_my_pi_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let path_info = normalize_stored_oh_my_pi_root_dir(db)?;

    let (path, source) = if let Some(path) = path_info {
        (PathBuf::from(path), "custom".to_string())
    } else {
        resolve_omp_path_without_db()
    };

    Ok(build_runtime_location(path, source))
}

fn normalize_stored_oh_my_pi_root_dir(
    db: &crate::db::SqliteDbState,
) -> Result<Option<String>, String> {
    // 先只做 DB 读取(normalize_omp_root_dir 里 contains_omp_runtime_layout 会对
    // root / root/agent 各做最多 5 次 is_file/is_dir,必须在连接闭包外进行,
    // 避免在 SQLite 锁持有的同时做文件 I/O)。
    let stored_root_dir: Option<String> = db.with_conn(|conn| {
        let Some(record) = db_get(conn, DbTable::OhMyPiSettingsConfig, "common")? else {
            return Ok(None);
        };
        Ok(
            crate::coding::oh_my_pi::adapter::settings_from_db_value(record)
                .root_dir
                .filter(|path| !path.trim().is_empty()),
        )
    })?;

    let Some(root_dir) = stored_root_dir else {
        return Ok(None);
    };

    // 文件系统探测(可能对不可达的 UNC/WSL 路径做 stat)在闭包外进行。
    let normalized_root_dir = crate::coding::oh_my_pi::normalize_omp_root_dir(&root_dir);
    if normalized_root_dir != root_dir {
        let _ = db.with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::OhMyPiSettingsConfig,
                "common",
                &[("root_dir", Value::String(normalized_root_dir.clone()))],
            )
        });
    }
    Ok(Some(normalized_root_dir))
}

/// OmO Native 引擎状态目录（`~/.omo/agent`）。
///
/// 决议顺序镜像上游 `packages/omo-senpi/src/components/agent-home/resolve-agent-home.ts`：
/// 1. 应用内自定义根目录（DB）
/// 2. `OMO_CODING_AGENT_DIR` / `SENPI_CODING_AGENT_DIR` / `PI_CODING_AGENT_DIR`（进程环境）
/// 3. 上述环境变量的 shell 配置（与 OMP/Pi 一致，覆盖新开终端才生效的场景）
/// 4. `~/.omo/agent`（哨兵 `settings.json`）
/// 5. pre-unification 扁平布局 `~/.omo`（哨兵 `settings.json`）
/// 6. `~/.senpi/agent`（无 branded 布局时的引擎回退）
fn resolve_omo_native_path_without_db() -> (PathBuf, String) {
    use omo_native::constants as omo_constants;

    for env_key in omo_constants::OMO_NATIVE_AGENT_DIR_ENV_KEYS {
        if let Ok(env_path) = std::env::var(env_key) {
            if !env_path.trim().is_empty() {
                return (PathBuf::from(env_path), "env".to_string());
            }
        }
    }

    for env_key in omo_constants::OMO_NATIVE_AGENT_DIR_ENV_KEYS {
        if let Some(shell_path) = shell_env::get_env_from_shell_config(env_key) {
            if !shell_path.trim().is_empty() {
                return (PathBuf::from(shell_path), "shell".to_string());
            }
        }
    }

    let home = get_home_dir().unwrap_or_else(|_| PathBuf::from("~"));
    let branded = home.join(omo_constants::OMO_NATIVE_CONFIG_DIR);
    let canonical = branded.join("agent");
    if canonical
        .join(omo_constants::OMO_NATIVE_AGENT_HOME_SENTINEL)
        .exists()
    {
        return (canonical, "default".to_string());
    }
    if branded
        .join(omo_constants::OMO_NATIVE_AGENT_HOME_SENTINEL)
        .exists()
    {
        // pre-unification 扁平布局：引擎状态直接写在 `~/.omo` 下。
        return (branded, "default".to_string());
    }

    let senpi_fallback = home.join(".senpi").join("agent");
    if senpi_fallback
        .join(omo_constants::OMO_NATIVE_AGENT_HOME_SENTINEL)
        .exists()
    {
        return (senpi_fallback, "default".to_string());
    }

    (canonical, "default".to_string())
}

pub fn get_omo_native_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("omo_native"))
}

pub async fn get_omo_native_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "omo_native").await
}

async fn resolve_omo_native_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let stored_root_dir: Option<String> = db.with_conn(|conn| {
        let Some(record) = db_get(conn, DbTable::OmoNativeSettingsConfig, "common")? else {
            return Ok(None);
        };
        Ok(omo_native::adapter::settings_from_db_value(record)
            .root_dir
            .filter(|path| !path.trim().is_empty()))
    })?;

    let (path, source) = match stored_root_dir {
        Some(path) => (PathBuf::from(path), "custom".to_string()),
        None => resolve_omo_native_path_without_db(),
    };

    Ok(build_runtime_location(path, source))
}

async fn resolve_pi_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let path_info = normalize_stored_pi_root_dir(db)?;

    let (path, source) = if let Some(path) = path_info {
        (PathBuf::from(path), "custom".to_string())
    } else {
        resolve_pi_path_without_db()
    };

    Ok(build_runtime_location(path, source))
}

fn normalize_stored_pi_root_dir(db: &crate::db::SqliteDbState) -> Result<Option<String>, String> {
    db.with_conn(|conn| {
        let Some(record) = db_get(conn, DbTable::PiSettingsConfig, "common")? else {
            return Ok(None);
        };
        let Some(root_dir) = crate::coding::pi::adapter::settings_from_db_value(record)
            .root_dir
            .filter(|path| !path.trim().is_empty())
        else {
            return Ok(None);
        };
        let normalized_root_dir = crate::coding::pi::normalize_pi_root_dir(&root_dir);
        if normalized_root_dir != root_dir {
            if let Err(error) = db_patch_fields(
                conn,
                DbTable::PiSettingsConfig,
                "common",
                &[("root_dir", Value::String(normalized_root_dir.clone()))],
            ) {
                log::warn!("Failed to persist normalized Pi root directory: {error}");
            }
        }
        Ok(Some(normalized_root_dir))
    })
}

async fn resolve_gemini_cli_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let path_info =
        get_custom_path_from_record(db, DbTable::GeminiCliCommonConfig, "common", |value| {
            crate::coding::gemini_cli::adapter::from_db_value_common(value)
                .root_dir
                .filter(|path| !path.trim().is_empty())
        })
        .await;

    let (path, source) = if let Some(path) = path_info {
        (PathBuf::from(path), "custom".to_string())
    } else {
        resolve_gemini_cli_path_without_db()
    };

    Ok(build_runtime_location(path, source))
}

pub fn get_gemini_cli_env_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_sync(db)?
        .host_path
        .join(".env"))
}

pub async fn get_gemini_cli_env_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_async(db)
        .await?
        .host_path
        .join(".env"))
}

pub fn get_gemini_cli_settings_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_sync(db)?
        .host_path
        .join("settings.json"))
}

pub async fn get_gemini_cli_settings_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_async(db)
        .await?
        .host_path
        .join("settings.json"))
}

pub fn get_gemini_cli_prompt_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    let location = get_gemini_cli_runtime_location_sync(db)?;
    Ok(gemini_cli::get_gemini_cli_prompt_path_from_root(
        &location.host_path,
    ))
}

pub async fn get_gemini_cli_prompt_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    let location = get_gemini_cli_runtime_location_async(db).await?;
    Ok(gemini_cli::get_gemini_cli_prompt_path_from_root(
        &location.host_path,
    ))
}

pub fn get_gemini_cli_oauth_creds_path_sync(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_sync(db)?
        .host_path
        .join("oauth_creds.json"))
}

pub async fn get_gemini_cli_oauth_creds_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_async(db)
        .await?
        .host_path
        .join("oauth_creds.json"))
}

pub fn get_gemini_cli_tmp_dir_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_sync(db)?
        .host_path
        .join("tmp"))
}

pub async fn get_gemini_cli_tmp_dir_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_async(db)
        .await?
        .host_path
        .join("tmp"))
}

pub fn get_gemini_cli_wsl_target_path(db: &crate::db::SqliteDbState, file_name: &str) -> String {
    match get_gemini_cli_runtime_location_sync(db) {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.gemini/{}", file_name)),
        Err(_) => format!("~/.gemini/{}", file_name),
    }
}

pub async fn get_gemini_cli_wsl_target_path_async(
    db: &crate::db::SqliteDbState,
    file_name: &str,
) -> String {
    match get_gemini_cli_runtime_location_async(db).await {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.gemini/{}", file_name)),
        Err(_) => format!("~/.gemini/{}", file_name),
    }
}

pub fn get_gemini_cli_prompt_wsl_target_path(db: &crate::db::SqliteDbState) -> String {
    let file_name = get_gemini_cli_prompt_path_sync(db)
        .ok()
        .and_then(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| gemini_cli::DEFAULT_GEMINI_CLI_PROMPT_FILE.to_string());
    get_gemini_cli_wsl_target_path(db, &file_name)
}

pub async fn get_gemini_cli_prompt_wsl_target_path_async(db: &crate::db::SqliteDbState) -> String {
    let file_name = get_gemini_cli_prompt_path_async(db)
        .await
        .ok()
        .and_then(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| gemini_cli::DEFAULT_GEMINI_CLI_PROMPT_FILE.to_string());
    get_gemini_cli_wsl_target_path_async(db, &file_name).await
}

pub fn get_antigravity_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("antigravity"))
}

pub async fn get_antigravity_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "antigravity").await
}

async fn resolve_antigravity_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let path_info =
        get_custom_path_from_record(db, DbTable::AntigravityCommonConfig, "common", |value| {
            crate::coding::antigravity::adapter::from_db_value_common(value)
                .root_dir
                .filter(|path| !path.trim().is_empty())
        })
        .await;

    let (path, source) = if let Some(path) = path_info {
        (PathBuf::from(path), "custom".to_string())
    } else {
        resolve_antigravity_path_without_db()
    };

    Ok(build_runtime_location(path, source))
}

pub fn get_antigravity_env_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_antigravity_runtime_location_sync(db)?
        .host_path
        .join(".env"))
}

pub async fn get_antigravity_env_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_antigravity_runtime_location_async(db)
        .await?
        .host_path
        .join(".env"))
}

pub fn get_antigravity_settings_path_sync(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_antigravity_runtime_location_sync(db)?
        .host_path
        .join("settings.json"))
}

pub async fn get_antigravity_settings_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_antigravity_runtime_location_async(db)
        .await?
        .host_path
        .join("settings.json"))
}

/// Antigravity CLI's skills dir: `<root>/skills`, following the custom root /
/// `ANTIGRAVITY_CLI_HOME` overrides the module honors. In WSL Direct the
/// effective root is the Linux one, so the UNC translation comes from the
/// location's WSL arm rather than from `~` expansion.
fn get_antigravity_cli_skills_path_from_location(location: &RuntimeLocationInfo) -> PathBuf {
    if let Some(wsl) = &location.wsl {
        let linux_skills_path = if location.source == "default" {
            expand_home_from_user_root(
                wsl.linux_user_root.as_deref(),
                "~/.gemini/antigravity-cli/skills",
            )
        } else {
            format!("{}/skills", wsl.linux_path.trim_end_matches('/'))
        };
        build_windows_unc_path(&wsl.distro, &linux_skills_path)
    } else {
        location.host_path.join("skills")
    }
}

/// Global rules file managed by AI Toolbox for `agy`.
///
/// This is NOT derived from the runtime root directory: `agy` reads global
/// rules from `~/.gemini/config/GEMINI.md` regardless of
/// `ANTIGRAVITY_CLI_HOME` or the in-app custom root directory. It only depends
/// on `$HOME`.
///
/// The one exception is WSL Direct: there `$HOME` is the Linux home, so the
/// rules file must be resolved inside the distribution. `linux_user_root`
/// carries that home (e.g. `/home/alice`) and is independent of the module's
/// runtime root, which is why this cannot reuse `location.host_path`.
fn antigravity_prompt_path_for_location(location: &RuntimeLocationInfo) -> PathBuf {
    if location.mode == RuntimeLocationMode::WslDirect {
        if let Some(wsl) = location.wsl.as_ref() {
            let linux_path = expand_home_from_user_root(
                wsl.linux_user_root.as_deref(),
                "~/.gemini/config/GEMINI.md",
            );
            if !linux_path.starts_with('~') {
                return build_windows_unc_path(&wsl.distro, &linux_path);
            }
        }
    }
    antigravity::get_antigravity_prompt_path()
        .unwrap_or_else(|_| PathBuf::from(antigravity::DEFAULT_ANTIGRAVITY_PROMPT_FILE))
}

pub fn get_antigravity_prompt_path_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(antigravity_prompt_path_for_location(
        &get_antigravity_runtime_location_sync(db)?,
    ))
}

pub async fn get_antigravity_prompt_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(antigravity_prompt_path_for_location(
        &get_antigravity_runtime_location_async(db).await?,
    ))
}

pub fn get_antigravity_tmp_dir_sync(db: &crate::db::SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_antigravity_runtime_location_sync(db)?
        .host_path
        .join("tmp"))
}

pub async fn get_antigravity_tmp_dir_async(
    db: &crate::db::SqliteDbState,
) -> Result<PathBuf, String> {
    Ok(get_antigravity_runtime_location_async(db)
        .await?
        .host_path
        .join("tmp"))
}

pub fn get_antigravity_wsl_target_path(db: &crate::db::SqliteDbState, file_name: &str) -> String {
    match get_antigravity_runtime_location_sync(db) {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.gemini/antigravity-cli/{}", file_name)),
        Err(_) => format!("~/.gemini/antigravity-cli/{}", file_name),
    }
}

pub async fn get_antigravity_wsl_target_path_async(
    db: &crate::db::SqliteDbState,
    file_name: &str,
) -> String {
    match get_antigravity_runtime_location_async(db).await {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.gemini/antigravity-cli/{}", file_name)),
        Err(_) => format!("~/.gemini/antigravity-cli/{}", file_name),
    }
}

pub fn get_antigravity_prompt_wsl_target_path(db: &crate::db::SqliteDbState) -> String {
    let _ = db;
    antigravity_prompt_wsl_target_path()
}

pub async fn get_antigravity_prompt_wsl_target_path_async(db: &crate::db::SqliteDbState) -> String {
    let _ = db;
    antigravity_prompt_wsl_target_path()
}

/// Linux-side location of the global rules file AI Toolbox manages.
///
/// `agy` reads global rules from `~/.gemini/config/GEMINI.md` on every OS, so
/// the WSL/SSH target is a fixed path rather than one derived from the
/// (host-side) runtime root directory.
fn antigravity_prompt_wsl_target_path() -> String {
    format!(
        "~/.gemini/config/{}",
        antigravity::DEFAULT_ANTIGRAVITY_PROMPT_FILE
    )
}

pub fn get_openclaw_runtime_location_sync(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("openclaw"))
}

pub async fn get_openclaw_runtime_location_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "openclaw").await
}

pub fn get_openclaw_wsl_target_path(db: &crate::db::SqliteDbState) -> String {
    match get_openclaw_runtime_location_sync(db) {
        Ok(location) => location
            .wsl
            .map(|wsl| wsl.linux_path)
            .unwrap_or_else(|| "~/.openclaw/openclaw.json".to_string()),
        Err(_) => "~/.openclaw/openclaw.json".to_string(),
    }
}

pub async fn get_openclaw_wsl_target_path_async(db: &crate::db::SqliteDbState) -> String {
    match get_openclaw_runtime_location_async(db).await {
        Ok(location) => location
            .wsl
            .map(|wsl| wsl.linux_path)
            .unwrap_or_else(|| "~/.openclaw/openclaw.json".to_string()),
        Err(_) => "~/.openclaw/openclaw.json".to_string(),
    }
}

pub fn get_tool_skills_path_sync(db: &crate::db::SqliteDbState, tool_key: &str) -> Option<PathBuf> {
    match tool_key {
        "claude_code" => get_claude_runtime_location_sync(db)
            .ok()
            .map(|location| get_claude_skills_path_from_location(&location)),
        "codex" => get_codex_runtime_location_sync(db).ok().map(|location| {
            if let Some(wsl) = location.wsl {
                build_windows_unc_path(
                    &wsl.distro,
                    &expand_home_from_user_root(wsl.linux_user_root.as_deref(), "~/.codex/skills"),
                )
            } else {
                location.host_path.join("skills")
            }
        }),
        "grok" => get_grok_runtime_location_sync(db).ok().map(|location| {
            if let Some(wsl) = location.wsl {
                build_windows_unc_path(
                    &wsl.distro,
                    &expand_home_from_user_root(wsl.linux_user_root.as_deref(), "~/.grok/skills"),
                )
            } else {
                location.host_path.join("skills")
            }
        }),
        "kimi" => get_kimi_runtime_location_sync(db).ok().map(|location| {
            if let Some(wsl) = location.wsl {
                build_windows_unc_path(
                    &wsl.distro,
                    &expand_home_from_user_root(
                        wsl.linux_user_root.as_deref(),
                        "~/.kimi-code/skills",
                    ),
                )
            } else {
                location.host_path.join("skills")
            }
        }),
        "zcode" => get_zcode_runtime_location_sync(db).ok().map(|location| {
            if let Some(wsl) = location.wsl {
                build_windows_unc_path(
                    &wsl.distro,
                    &expand_home_from_user_root(wsl.linux_user_root.as_deref(), "~/.zcode/skills"),
                )
            } else {
                location.host_path.join("skills")
            }
        }),
        "opencode" => get_opencode_runtime_location_sync(db).ok().map(|location| {
            if let Some(wsl) = location.wsl {
                build_windows_unc_path(
                    &wsl.distro,
                    &expand_home_from_user_root(
                        wsl.linux_user_root.as_deref(),
                        "~/.config/opencode/skills",
                    ),
                )
            } else {
                location
                    .host_path
                    .parent()
                    .unwrap_or_else(|| location.host_path.as_path())
                    .join("skills")
            }
        }),
        "openclaw" => get_openclaw_runtime_location_sync(db).ok().map(|location| {
            if let Some(wsl) = location.wsl {
                build_windows_unc_path(
                    &wsl.distro,
                    &expand_home_from_user_root(
                        wsl.linux_user_root.as_deref(),
                        "~/.openclaw/skills",
                    ),
                )
            } else {
                location
                    .host_path
                    .parent()
                    .unwrap_or_else(|| location.host_path.as_path())
                    .join("skills")
            }
        }),
        "pi" => get_pi_runtime_location_sync(db)
            .ok()
            .map(|location| get_pi_skills_path_from_location(&location)),
        "oh_my_pi" => get_oh_my_pi_runtime_location_sync(db)
            .ok()
            .map(|location| get_omp_skills_path_from_location(&location)),
        "omo_native" => get_omo_native_runtime_location_sync(db)
            .ok()
            .map(|location| get_omo_native_skills_path_from_location(&location)),
        "gemini_cli" => get_gemini_cli_runtime_location_sync(db)
            .ok()
            .map(|location| get_gemini_cli_skills_path_from_location(&location)),
        "antigravity_cli" => get_antigravity_runtime_location_sync(db)
            .ok()
            .map(|location| get_antigravity_cli_skills_path_from_location(&location)),
        "hermes" => Some(
            get_cached_or_fallback_runtime_location("hermes")
                .host_path
                .join("skills"),
        ),
        "workbuddy" => get_workbuddy_runtime_location_sync(db).ok().map(|location| {
            if let Some(wsl) = location.wsl {
                build_windows_unc_path(
                    &wsl.distro,
                    &expand_home_from_user_root(
                        wsl.linux_user_root.as_deref(),
                        "~/.workbuddy/skills",
                    ),
                )
            } else {
                location.host_path.join("skills")
            }
        }),
        "codebuddy" => get_codebuddy_runtime_location_sync(db).ok().map(|location| {
            if let Some(wsl) = location.wsl {
                build_windows_unc_path(
                    &wsl.distro,
                    &expand_home_from_user_root(
                        wsl.linux_user_root.as_deref(),
                        "~/.codebuddy/skills",
                    ),
                )
            } else {
                location.host_path.join("skills")
            }
        }),
        _ => None,
    }
}

pub async fn get_tool_skills_path_async(
    db: &crate::db::SqliteDbState,
    tool_key: &str,
) -> Option<PathBuf> {
    match tool_key {
        "claude_code" => get_claude_runtime_location_async(db)
            .await
            .ok()
            .map(|location| get_claude_skills_path_from_location(&location)),
        "codex" => get_codex_runtime_location_async(db)
            .await
            .ok()
            .map(|location| {
                if let Some(wsl) = location.wsl {
                    build_windows_unc_path(
                        &wsl.distro,
                        &expand_home_from_user_root(
                            wsl.linux_user_root.as_deref(),
                            "~/.codex/skills",
                        ),
                    )
                } else {
                    location.host_path.join("skills")
                }
            }),
        "grok" => get_grok_runtime_location_async(db)
            .await
            .ok()
            .map(|location| {
                if let Some(wsl) = location.wsl {
                    build_windows_unc_path(
                        &wsl.distro,
                        &expand_home_from_user_root(
                            wsl.linux_user_root.as_deref(),
                            "~/.grok/skills",
                        ),
                    )
                } else {
                    location.host_path.join("skills")
                }
            }),
        "kimi" => get_kimi_runtime_location_async(db)
            .await
            .ok()
            .map(|location| {
                if let Some(wsl) = location.wsl {
                    build_windows_unc_path(
                        &wsl.distro,
                        &expand_home_from_user_root(
                            wsl.linux_user_root.as_deref(),
                            "~/.kimi-code/skills",
                        ),
                    )
                } else {
                    location.host_path.join("skills")
                }
            }),
        "zcode" => get_zcode_runtime_location_async(db)
            .await
            .ok()
            .map(|location| {
                if let Some(wsl) = location.wsl {
                    build_windows_unc_path(
                        &wsl.distro,
                        &expand_home_from_user_root(
                            wsl.linux_user_root.as_deref(),
                            "~/.zcode/skills",
                        ),
                    )
                } else {
                    location.host_path.join("skills")
                }
            }),
        "opencode" => get_opencode_runtime_location_async(db)
            .await
            .ok()
            .map(|location| {
                if let Some(wsl) = location.wsl {
                    build_windows_unc_path(
                        &wsl.distro,
                        &expand_home_from_user_root(
                            wsl.linux_user_root.as_deref(),
                            "~/.config/opencode/skills",
                        ),
                    )
                } else {
                    location
                        .host_path
                        .parent()
                        .unwrap_or_else(|| location.host_path.as_path())
                        .join("skills")
                }
            }),
        "openclaw" => get_openclaw_runtime_location_async(db)
            .await
            .ok()
            .map(|location| {
                if let Some(wsl) = location.wsl {
                    build_windows_unc_path(
                        &wsl.distro,
                        &expand_home_from_user_root(
                            wsl.linux_user_root.as_deref(),
                            "~/.openclaw/skills",
                        ),
                    )
                } else {
                    location
                        .host_path
                        .parent()
                        .unwrap_or_else(|| location.host_path.as_path())
                        .join("skills")
                }
            }),
        "pi" => get_pi_runtime_location_async(db)
            .await
            .ok()
            .map(|location| get_pi_skills_path_from_location(&location)),
        "oh_my_pi" => get_oh_my_pi_runtime_location_async(db)
            .await
            .ok()
            .map(|location| get_omp_skills_path_from_location(&location)),
        "omo_native" => get_omo_native_runtime_location_async(db)
            .await
            .ok()
            .map(|location| get_omo_native_skills_path_from_location(&location)),
        "gemini_cli" => get_gemini_cli_runtime_location_async(db)
            .await
            .ok()
            .map(|location| get_gemini_cli_skills_path_from_location(&location)),
        "antigravity_cli" => get_antigravity_runtime_location_async(db)
            .await
            .ok()
            .map(|location| get_antigravity_cli_skills_path_from_location(&location)),
        "hermes" => get_cached_or_refresh_runtime_location_async(db, "hermes")
            .await
            .ok()
            .map(|location| location.host_path.join("skills")),
        "workbuddy" => get_workbuddy_runtime_location_async(db)
            .await
            .ok()
            .map(|location| {
                if let Some(wsl) = location.wsl {
                    build_windows_unc_path(
                        &wsl.distro,
                        &expand_home_from_user_root(
                            wsl.linux_user_root.as_deref(),
                            "~/.workbuddy/skills",
                        ),
                    )
                } else {
                    location.host_path.join("skills")
                }
            }),
        "codebuddy" => get_codebuddy_runtime_location_async(db)
            .await
            .ok()
            .map(|location| {
                if let Some(wsl) = location.wsl {
                    build_windows_unc_path(
                        &wsl.distro,
                        &expand_home_from_user_root(
                            wsl.linux_user_root.as_deref(),
                            "~/.codebuddy/skills",
                        ),
                    )
                } else {
                    location.host_path.join("skills")
                }
            }),
        _ => None,
    }
}

fn get_claude_skills_path_from_location(location: &RuntimeLocationInfo) -> PathBuf {
    if let Some(wsl) = &location.wsl {
        let linux_skills_path = if location.source == "default" {
            expand_home_from_user_root(wsl.linux_user_root.as_deref(), "~/.claude/skills")
        } else {
            format!("{}/skills", wsl.linux_path.trim_end_matches('/'))
        };

        build_windows_unc_path(&wsl.distro, &linux_skills_path)
    } else {
        location.host_path.join("skills")
    }
}

fn get_pi_skills_path_from_location(location: &RuntimeLocationInfo) -> PathBuf {
    if let Some(wsl) = &location.wsl {
        let linux_skills_path = if location.source == "default" {
            expand_home_from_user_root(wsl.linux_user_root.as_deref(), "~/.pi/agent/skills")
        } else {
            format!("{}/skills", wsl.linux_path.trim_end_matches('/'))
        };

        build_windows_unc_path(&wsl.distro, &linux_skills_path)
    } else {
        location.host_path.join("skills")
    }
}

fn get_omp_skills_path_from_location(location: &RuntimeLocationInfo) -> PathBuf {
    // OMP native(priority 100)读 <agentDir>/skills(即 ~/.omp/agent/skills),
    // 而非 agents provider(priority 70,可被 skills.enableAgentsUser 关闭)的
    // ~/.agents/skills。同步目标必须跟随 agent 目录,否则同名 skill 会被
    // <agentDir>/skills 遮蔽、且可被一键关闭、与其他 .agents 工具互相污染。
    if let Some(wsl) = &location.wsl {
        build_windows_unc_path(
            &wsl.distro,
            &format!("{}/skills", wsl.linux_path.trim_end_matches('/')),
        )
    } else {
        location.host_path.join("skills")
    }
}

fn get_gemini_cli_skills_path_from_location(location: &RuntimeLocationInfo) -> PathBuf {
    // Gemini CLI is a root-dir module: skills live at <root>/skills
    // (~/.gemini/skills by default, or under a custom/env/shell root).
    if let Some(wsl) = &location.wsl {
        let linux_skills_path = if location.source == "default" {
            expand_home_from_user_root(wsl.linux_user_root.as_deref(), "~/.gemini/skills")
        } else {
            format!("{}/skills", wsl.linux_path.trim_end_matches('/'))
        };

        build_windows_unc_path(&wsl.distro, &linux_skills_path)
    } else {
        location.host_path.join("skills")
    }
}

fn get_omp_mcp_config_path_from_location(location: &RuntimeLocationInfo) -> PathBuf {
    if let Some(wsl) = &location.wsl {
        let linux_mcp_path = if location.source == "default" {
            expand_home_from_user_root(wsl.linux_user_root.as_deref(), "~/.omp/agent/mcp.json")
        } else {
            format!("{}/mcp.json", wsl.linux_path.trim_end_matches('/'))
        };

        build_windows_unc_path(&wsl.distro, &linux_mcp_path)
    } else {
        location.host_path.join("mcp.json")
    }
}

fn get_pi_mcp_config_path_from_location(location: &RuntimeLocationInfo) -> PathBuf {
    if let Some(wsl) = &location.wsl {
        let linux_mcp_path = if location.source == "default" {
            expand_home_from_user_root(wsl.linux_user_root.as_deref(), "~/.pi/agent/mcp.json")
        } else {
            format!("{}/mcp.json", wsl.linux_path.trim_end_matches('/'))
        };

        build_windows_unc_path(&wsl.distro, &linux_mcp_path)
    } else {
        location.host_path.join("mcp.json")
    }
}

/// CodeBuddy's user-scope MCP file lives beside the config directory, not in it:
/// `<parent>/.codebuddy.json` for a root of `<parent>/.codebuddy`.
fn codebuddy_mcp_path_in(root: &Path) -> PathBuf {
    root.join(codebuddy::constants::CODEBUDDY_USER_MCP_FILE)
}

pub fn get_tool_mcp_config_path_sync(
    db: &crate::db::SqliteDbState,
    tool_key: &str,
) -> Option<PathBuf> {
    match tool_key {
        "claude_code" => get_claude_mcp_config_path_sync(db).ok(),
        "codex" => get_codex_config_path_sync(db).ok(),
        "grok" => get_grok_config_path_sync(db).ok(),
        "kimi" => get_kimi_mcp_config_path_sync(db).ok(),
        "zcode" => get_zcode_mcp_config_path_sync(db).ok(),
        "opencode" => get_opencode_runtime_location_sync(db)
            .ok()
            .map(|location| location.host_path),
        "openclaw" => get_openclaw_runtime_location_sync(db)
            .ok()
            .map(|location| location.host_path),
        "pi" => get_pi_runtime_location_sync(db)
            .ok()
            .map(|location| get_pi_mcp_config_path_from_location(&location)),
        "oh_my_pi" => get_oh_my_pi_runtime_location_sync(db)
            .ok()
            .map(|location| get_omp_mcp_config_path_from_location(&location)),
        "omo_native" => get_omo_native_runtime_location_sync(db)
            .ok()
            .map(|location| get_omo_native_mcp_config_path_from_location(&location)),
        // The desktop app writes `mcp.json`; the CLI reads its own `.mcp.json`
        // before that file, so the two modules must not point at the same one.
        "workbuddy" => get_workbuddy_runtime_location_sync(db)
            .ok()
            .map(|location| {
                location
                    .host_path
                    .join(workbuddy::constants::WORKBUDDY_MCP_CONFIG_FILE)
            }),
        // CodeBuddy user-scope MCP is the **sibling** `~/.codebuddy.json`, not a
        // file inside the config directory. Verified against the real CLI
        // (v2.178.1): `codebuddy mcp add -s user` writes top-level
        // `mcpServers` there, and neither `~/.codebuddy/.mcp.json` nor
        // `~/.codebuddy/mcp.json` is read or created. This value wins over
        // `builtin.rs` (`detection::resolve_mcp_config_path_with_db` tries
        // runtime_location first), so pointing it at `.mcp.json` made every
        // MCP sync report success while the CLI ignored the file.
        "codebuddy" => get_codebuddy_runtime_location_sync(db)
            .ok()
            .and_then(|location| location.host_path.parent().map(codebuddy_mcp_path_in)),
        _ => None,
    }
}

pub async fn get_tool_mcp_config_path_async(
    db: &crate::db::SqliteDbState,
    tool_key: &str,
) -> Option<PathBuf> {
    match tool_key {
        "claude_code" => get_claude_mcp_config_path_async(db).await.ok(),
        "codex" => get_codex_config_path_async(db).await.ok(),
        "grok" => get_grok_config_path_async(db).await.ok(),
        "kimi" => get_kimi_mcp_config_path_async(db).await.ok(),
        "zcode" => get_zcode_mcp_config_path_async(db).await.ok(),
        "opencode" => get_opencode_runtime_location_async(db)
            .await
            .ok()
            .map(|location| location.host_path),
        "openclaw" => get_openclaw_runtime_location_async(db)
            .await
            .ok()
            .map(|location| location.host_path),
        "pi" => get_pi_runtime_location_async(db)
            .await
            .ok()
            .map(|location| get_pi_mcp_config_path_from_location(&location)),
        "oh_my_pi" => get_oh_my_pi_runtime_location_async(db)
            .await
            .ok()
            .map(|location| get_omp_mcp_config_path_from_location(&location)),
        "omo_native" => get_omo_native_runtime_location_async(db)
            .await
            .ok()
            .map(|location| get_omo_native_mcp_config_path_from_location(&location)),
        // The desktop app writes `mcp.json`; the CLI reads its own `.mcp.json`
        // before that file, so the two modules must not point at the same one.
        "workbuddy" => get_workbuddy_runtime_location_async(db)
            .await
            .ok()
            .map(|location| {
                location
                    .host_path
                    .join(workbuddy::constants::WORKBUDDY_MCP_CONFIG_FILE)
            }),
        // Sibling `~/.codebuddy.json`, not a file inside the config directory —
        // see the sync counterpart above for the real-CLI evidence.
        "codebuddy" => get_codebuddy_runtime_location_async(db)
            .await
            .ok()
            .and_then(|location| location.host_path.parent().map(codebuddy_mcp_path_in)),
        _ => None,
    }
}

fn get_omo_native_skills_path_from_location(location: &RuntimeLocationInfo) -> PathBuf {
    location
        .host_path
        .join(omo_native::constants::OMO_NATIVE_SKILLS_DIR)
}

fn get_omo_native_mcp_config_path_from_location(location: &RuntimeLocationInfo) -> PathBuf {
    location
        .host_path
        .join(omo_native::constants::OMO_NATIVE_MCP_FILE)
}

fn build_runtime_location(path: PathBuf, source: String) -> RuntimeLocationInfo {
    let wsl = path.to_str().and_then(parse_wsl_unc_path);

    RuntimeLocationInfo {
        mode: if wsl.is_some() {
            RuntimeLocationMode::WslDirect
        } else {
            RuntimeLocationMode::LocalWindows
        },
        source,
        host_path: path,
        wsl,
    }
}

fn get_runtime_location_without_db(module: &str) -> RuntimeLocationInfo {
    let (path, source) = resolve_config_path_without_db(module);
    build_runtime_location(path, source)
}

fn resolve_config_path_without_db(module: &str) -> (PathBuf, String) {
    match module {
        "opencode" => resolve_opencode_path_without_db(),
        "claude" => resolve_claude_path_without_db(),
        "codex" => resolve_codex_path_without_db(),
        "grok" => resolve_grok_path_without_db(),
        "kimi" => resolve_kimi_path_without_db(),
        "openclaw" => resolve_openclaw_path_without_db(),
        "geminicli" => resolve_gemini_cli_path_without_db(),
        "antigravity" => resolve_antigravity_path_without_db(),
        "zcode" => resolve_zcode_path_without_db(),
        "pi" => resolve_pi_path_without_db(),
        "oh_my_pi" => resolve_omp_path_without_db(),
        "omo_native" => resolve_omo_native_path_without_db(),
        "hermes" => hermes::commands::resolve_hermes_path_without_db(),
        "dsh" => dsh::commands::resolve_dsh_path_without_db(),
        "workbuddy" => workbuddy::paths::resolve_path_without_db(),
        "codebuddy" => codebuddy::paths::resolve_path_without_db(),
        _ => (PathBuf::new(), "default".to_string()),
    }
}

fn resolve_opencode_path_without_db() -> (PathBuf, String) {
    if let Ok(env_path) = std::env::var("OPENCODE_CONFIG") {
        if !env_path.trim().is_empty() {
            return (PathBuf::from(env_path), "env".to_string());
        }
    }

    if let Some(shell_path) = shell_env::get_env_from_shell_config("OPENCODE_CONFIG") {
        if !shell_path.trim().is_empty() {
            return (PathBuf::from(shell_path), "shell".to_string());
        }
    }

    // `get_default_config_path` already honors `OPENCODE_CONFIG_DIR`.
    (
        PathBuf::from(
            open_code::get_default_config_path()
                .unwrap_or_else(|_| "~/.config/opencode/opencode.jsonc".to_string()),
        ),
        "default".to_string(),
    )
}

/// Claude Code's root directory, resolved from the environment and shell
/// config only.
///
/// Exposed for the backup restore path (which runs before the database is
/// available) and for `claude_code::get_claude_root_dir_without_db`, which used
/// to carry a second, byte-identical copy of this logic.
pub fn resolve_claude_root_dir_without_db() -> PathBuf {
    resolve_claude_path_without_db().0
}

fn resolve_claude_path_without_db() -> (PathBuf, String) {
    if let Ok(env_path) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !env_path.trim().is_empty() {
            return (PathBuf::from(env_path), "env".to_string());
        }
    }

    if let Some(shell_path) = shell_env::get_env_from_shell_config("CLAUDE_CONFIG_DIR") {
        if !shell_path.trim().is_empty() {
            return (PathBuf::from(shell_path), "shell".to_string());
        }
    }

    (
        claude_code::get_claude_default_root_dir().unwrap_or_else(|_| PathBuf::from("~/.claude")),
        "default".to_string(),
    )
}

/// Codex's root directory, resolved from the environment and shell config only.
///
/// Same reason as `resolve_claude_root_dir_without_db`: the restore path needs
/// it without a database, and `codex::get_codex_root_dir_without_db` used to
/// carry a second copy.
pub fn resolve_codex_root_dir_without_db() -> PathBuf {
    resolve_codex_path_without_db().0
}

fn resolve_codex_path_without_db() -> (PathBuf, String) {
    if let Ok(env_path) = std::env::var("CODEX_HOME") {
        if !env_path.trim().is_empty() {
            return (PathBuf::from(env_path), "env".to_string());
        }
    }

    if let Some(shell_path) = shell_env::get_env_from_shell_config("CODEX_HOME") {
        if !shell_path.trim().is_empty() {
            return (PathBuf::from(shell_path), "shell".to_string());
        }
    }

    (
        codex::get_codex_default_root_dir().unwrap_or_else(|_| PathBuf::from("~/.codex")),
        "default".to_string(),
    )
}

fn resolve_grok_path_without_db() -> (PathBuf, String) {
    if let Ok(env_path) = std::env::var(grok::constants::GROK_ENV_KEY) {
        if !env_path.trim().is_empty() {
            return (PathBuf::from(env_path), "env".to_string());
        }
    }
    if let Some(shell_path) = shell_env::get_env_from_shell_config(grok::constants::GROK_ENV_KEY) {
        if !shell_path.trim().is_empty() {
            return (PathBuf::from(shell_path), "shell".to_string());
        }
    }
    (
        grok::get_grok_default_root_dir().unwrap_or_else(|_| PathBuf::from("~/.grok")),
        "default".to_string(),
    )
}

fn resolve_openclaw_path_without_db() -> (PathBuf, String) {
    (
        PathBuf::from(
            open_claw::get_default_config_path_for_runtime()
                .unwrap_or_else(|_| "~/.openclaw/openclaw.json".to_string()),
        ),
        "default".to_string(),
    )
}

fn resolve_gemini_cli_path_without_db() -> (PathBuf, String) {
    if let Some(env_root_dir) = gemini_cli::get_gemini_cli_root_dir_from_env() {
        return (env_root_dir, "env".to_string());
    }

    if let Some(shell_root_dir) =
        shell_env::get_env_from_shell_config(gemini_cli::GEMINI_CLI_HOME_ENV_KEY)
            .and_then(|home_dir| gemini_cli::get_gemini_cli_root_dir_from_home_override(&home_dir))
    {
        return (shell_root_dir, "shell".to_string());
    }

    (
        gemini_cli::get_gemini_cli_default_root_dir()
            .unwrap_or_else(|_| PathBuf::from("~/.gemini")),
        "default".to_string(),
    )
}

fn resolve_antigravity_path_without_db() -> (PathBuf, String) {
    if let Some(env_root_dir) = antigravity::get_antigravity_root_dir_from_env() {
        return (env_root_dir, "env".to_string());
    }

    if let Some(shell_root_dir) = shell_env::get_env_from_shell_config(
        antigravity::ANTIGRAVITY_CLI_HOME_ENV_KEY,
    )
    .and_then(|home_dir| antigravity::get_antigravity_root_dir_from_home_override(&home_dir))
    {
        return (shell_root_dir, "shell".to_string());
    }

    (
        antigravity::get_antigravity_default_root_dir()
            .unwrap_or_else(|_| PathBuf::from("~/.gemini/antigravity-cli")),
        "default".to_string(),
    )
}

fn resolve_pi_path_without_db() -> (PathBuf, String) {
    if let Ok(env_path) = std::env::var(pi::constants::PI_ENV_KEY) {
        if !env_path.trim().is_empty() {
            return (PathBuf::from(env_path), "env".to_string());
        }
    }

    if let Some(shell_path) = shell_env::get_env_from_shell_config(pi::constants::PI_ENV_KEY) {
        if !shell_path.trim().is_empty() {
            return (PathBuf::from(shell_path), "shell".to_string());
        }
    }

    (
        pi::get_pi_default_root_dir().unwrap_or_else(|_| PathBuf::from("~/.pi/agent")),
        "default".to_string(),
    )
}

fn resolve_omp_path_without_db() -> (PathBuf, String) {
    if let Ok(env_path) = std::env::var(oh_my_pi::constants::OMP_ENV_KEY) {
        if !env_path.trim().is_empty() {
            return (PathBuf::from(env_path), "env".to_string());
        }
    }

    if let Some(shell_path) = shell_env::get_env_from_shell_config(oh_my_pi::constants::OMP_ENV_KEY)
    {
        if !shell_path.trim().is_empty() {
            return (PathBuf::from(shell_path), "shell".to_string());
        }
    }

    (
        oh_my_pi::get_omp_default_root_dir().unwrap_or_else(|_| PathBuf::from("~/.omp/agent")),
        "default".to_string(),
    )
}

async fn resolve_opencode_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let (path, source) = resolve_opencode_config_path_async(db).await?;
    Ok(build_runtime_location(path, source))
}

async fn resolve_opencode_config_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<(PathBuf, String), String> {
    let custom_path =
        get_custom_path_from_record(db, DbTable::OpenCodeCommonConfig, "common", |value| {
            open_code::adapter::from_db_value(value)
                .config_path
                .filter(|path| !path.trim().is_empty())
        })
        .await;

    if let Some(path) = custom_path {
        return Ok((PathBuf::from(path), "custom".to_string()));
    }

    if let Ok(env_path) = std::env::var("OPENCODE_CONFIG") {
        if !env_path.trim().is_empty() {
            return Ok((PathBuf::from(env_path), "env".to_string()));
        }
    }

    if let Some(shell_path) = shell_env::get_env_from_shell_config("OPENCODE_CONFIG") {
        if !shell_path.trim().is_empty() {
            return Ok((PathBuf::from(shell_path), "shell".to_string()));
        }
    }

    // `OPENCODE_CONFIG_DIR` is honored inside `get_default_config_path`.
    Ok((
        PathBuf::from(open_code::get_default_config_path()?),
        "default".to_string(),
    ))
}

async fn resolve_openclaw_runtime_location_uncached_async(
    db: &crate::db::SqliteDbState,
) -> Result<RuntimeLocationInfo, String> {
    let (path, source) = resolve_openclaw_config_path_async(db).await?;
    Ok(build_runtime_location(path, source))
}

async fn resolve_openclaw_config_path_async(
    db: &crate::db::SqliteDbState,
) -> Result<(PathBuf, String), String> {
    let custom_path =
        get_custom_path_from_record(db, DbTable::OpenClawCommonConfig, "common", |value| {
            open_claw::adapter::from_db_value(value)
                .config_path
                .filter(|path| !path.trim().is_empty())
        })
        .await;

    if let Some(path) = custom_path {
        return Ok((PathBuf::from(path), "custom".to_string()));
    }

    let path = open_claw::get_default_config_path_for_runtime()?;
    Ok((PathBuf::from(path), "default".to_string()))
}

async fn get_custom_path_from_record<F>(
    db: &crate::db::SqliteDbState,
    table: DbTable,
    id: &str,
    extractor: F,
) -> Option<String>
where
    F: Fn(Value) -> Option<String>,
{
    let record = db.with_conn(|conn| db_get(conn, table, id)).ok()??;
    extractor(record)
}

#[cfg(test)]
mod tests {
    use super::{
        clear_runtime_location_cache, expand_home_from_user_root, get_claude_mcp_config_path_async,
        get_claude_mcp_config_path_from_location, get_claude_mcp_config_path_sync,
        get_claude_plugin_config_path_async, get_claude_plugin_config_path_sync,
        get_claude_plugins_dir_async, get_claude_plugins_dir_sync, get_claude_prompt_path_async,
        get_antigravity_prompt_path_async, get_claude_prompt_path_sync,
        get_claude_runtime_location_async,
        get_claude_runtime_location_sync, get_claude_settings_path_async,
        get_claude_settings_path_sync, get_claude_wsl_claude_json_path_async,
        get_claude_wsl_target_path_async, get_omo_config_path_async,
        get_tool_mcp_config_path_async, get_tool_mcp_config_path_sync, get_tool_skills_path_async,
        get_tool_skills_path_sync, module_status_from_runtime_result,
        refresh_runtime_location_cache_for_module_async, replace_path_file_name,
        resolve_codex_prompt_file_path, set_cached_runtime_location, uses_legacy_omo_config_async,
        RuntimeLocationInfo, RuntimeLocationMode, WslLocationInfo, CODEX_DEFAULT_PROMPT_FILE_NAME,
        CODEX_OVERRIDE_PROMPT_FILE_NAME,
    };
    use crate::db::helpers::{db_delete, db_get, db_put};
    use crate::db::schema::DbTable;
    use crate::db::SqliteDbState;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use tokio::sync::Mutex;

    static TEST_RUNTIME_LOCATION_LOCK: std::sync::LazyLock<Mutex<()>> =
        std::sync::LazyLock::new(|| Mutex::new(()));

    struct EnvVarGuard {
        key: &'static str,
        previous_value: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &PathBuf) -> Self {
            let previous_value = std::env::var_os(key);
            std::env::set_var(key, value);
            Self {
                key,
                previous_value,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous_value {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    async fn create_test_db() -> (tempfile::TempDir, SqliteDbState) {
        let temp_dir = tempfile::tempdir().expect("create temp db dir");
        let db = SqliteDbState::in_memory_for_test().expect("open sqlite test db");
        (temp_dir, db)
    }

    fn save_claude_common_config(db: &SqliteDbState, root_dir: &str) {
        db.with_conn(|conn| {
            db_put(
                conn,
                DbTable::ClaudeCommonConfig,
                "common",
                &serde_json::json!({
                    "config": "{}",
                    "root_dir": root_dir,
                }),
            )
        })
        .expect("save claude common config");
    }

    fn delete_claude_common_config(db: &SqliteDbState) {
        db.with_conn(|conn| db_delete(conn, DbTable::ClaudeCommonConfig, "common").map(|_| ()))
            .expect("delete claude common config");
    }

    /// Regression for Claude marketplace `installLocation` not being expanded.
    ///
    /// Claude CLI 2.1.126+ refuses to recognise marketplaces whose
    /// `installLocation` still contains `~`. The WSL/SSH sync paths must
    /// resolve `~` against the remote user's real `$HOME` before handing the
    /// string to `plugin_metadata_sync`. The actual substitution is delegated
    /// to `expand_home_from_user_root`, so we lock that behaviour down here.
    #[test]
    fn expand_home_from_user_root_handles_tilde_paths() {
        // Bare `~` resolves to the supplied home root.
        assert_eq!(
            expand_home_from_user_root(Some("/home/tester"), "~"),
            "/home/tester"
        );

        // `~/...` is rewritten with the real home and trailing slashes are
        // collapsed so we don't end up with `/home/tester//.claude/plugins`.
        assert_eq!(
            expand_home_from_user_root(Some("/home/tester/"), "~/.claude/plugins"),
            "/home/tester/.claude/plugins"
        );

        // Absolute paths are returned verbatim.
        assert_eq!(
            expand_home_from_user_root(Some("/home/tester"), "/etc/claude"),
            "/etc/claude"
        );

        // Without a known home root, the original `~` path is preserved so
        // callers can detect the failure (and surface a real error instead of
        // silently writing a broken `installLocation`).
        assert_eq!(
            expand_home_from_user_root(None, "~/.claude/plugins"),
            "~/.claude/plugins"
        );
    }

    #[test]
    fn codex_prompt_path_prefers_non_empty_override_file() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        std::fs::write(
            temp_dir.path().join(CODEX_DEFAULT_PROMPT_FILE_NAME),
            "default prompt",
        )
        .expect("write default prompt");
        std::fs::write(
            temp_dir.path().join(CODEX_OVERRIDE_PROMPT_FILE_NAME),
            "override prompt",
        )
        .expect("write override prompt");

        assert_eq!(
            resolve_codex_prompt_file_path(temp_dir.path()),
            temp_dir.path().join(CODEX_OVERRIDE_PROMPT_FILE_NAME)
        );
    }

    #[test]
    fn codex_prompt_path_falls_back_to_default_when_override_is_empty() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        std::fs::write(
            temp_dir.path().join(CODEX_DEFAULT_PROMPT_FILE_NAME),
            "default prompt",
        )
        .expect("write default prompt");
        std::fs::write(
            temp_dir.path().join(CODEX_OVERRIDE_PROMPT_FILE_NAME),
            "  \n",
        )
        .expect("write empty override prompt");

        assert_eq!(
            resolve_codex_prompt_file_path(temp_dir.path()),
            temp_dir.path().join(CODEX_DEFAULT_PROMPT_FILE_NAME)
        );
    }

    #[test]
    fn codex_prompt_path_uses_empty_override_as_write_target_without_default_content() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        std::fs::write(
            temp_dir.path().join(CODEX_OVERRIDE_PROMPT_FILE_NAME),
            "  \n",
        )
        .expect("write empty override prompt");

        assert_eq!(
            resolve_codex_prompt_file_path(temp_dir.path()),
            temp_dir.path().join(CODEX_OVERRIDE_PROMPT_FILE_NAME)
        );
    }

    #[test]
    fn replace_path_file_name_handles_unix_and_windows_paths() {
        assert_eq!(
            replace_path_file_name("~/.codex/AGENTS.md", CODEX_OVERRIDE_PROMPT_FILE_NAME),
            "~/.codex/AGENTS.override.md"
        );
        assert_eq!(
            replace_path_file_name(
                r"C:\Users\tester\.codex\AGENTS.override.md",
                CODEX_DEFAULT_PROMPT_FILE_NAME,
            ),
            r"C:\Users\tester\.codex\AGENTS.md"
        );
    }

    fn local_home_claude_json_path() -> PathBuf {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .map(PathBuf::from)
            .expect("resolve local home dir")
            .join(".claude.json")
    }

    fn local_home_dir() -> PathBuf {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .map(PathBuf::from)
            .expect("resolve local home dir")
    }

    #[tokio::test]
    async fn omo_config_path_uses_unified_by_default_and_legacy_with_setting() {
        let _guard = TEST_RUNTIME_LOCATION_LOCK.lock().await;
        clear_runtime_location_cache();
        let (_temp_dir, db) = create_test_db().await;

        // Default (no setting): unified ~/.omo/omo.jsonc
        assert!(!uses_legacy_omo_config_async(&db).await);
        let unified = get_omo_config_path_async(&db).await.unwrap();
        assert_eq!(unified, local_home_dir().join(".omo").join("omo.jsonc"));

        // Switch to legacy mode and point OPENCODE_CONFIG at a temp dir so the
        // legacy path resolution is deterministic.
        let mut settings = crate::settings::store::load_settings_from_sqlite_state(&db).unwrap();
        settings.opencode_use_legacy_oh_my_config = true;
        crate::settings::store::save_settings_to_sqlite_state(&db, &settings).unwrap();

        let temp_dir = tempfile::tempdir().unwrap();
        let opencode_config = temp_dir.path().join("opencode.jsonc");
        let _env = EnvVarGuard::set("OPENCODE_CONFIG", &opencode_config);
        clear_runtime_location_cache();

        assert!(uses_legacy_omo_config_async(&db).await);
        let legacy = get_omo_config_path_async(&db).await.unwrap();
        assert_eq!(legacy, temp_dir.path().join("oh-my-openagent.jsonc"));
    }

    #[tokio::test]
    async fn omo_unified_wsl_direct_custom_opencode_config_uses_wsl_home() {
        let _guard = TEST_RUNTIME_LOCATION_LOCK.lock().await;
        clear_runtime_location_cache();
        let (_temp_dir, db) = create_test_db().await;
        let location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::WslDirect,
            source: "custom".to_string(),
            host_path: PathBuf::from(
                r"\\wsl.localhost\Ubuntu\home\tester\.config\opencode\opencode.jsonc",
            ),
            wsl: Some(WslLocationInfo {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/tester/.config/opencode/opencode.jsonc".to_string(),
                linux_user_root: Some("/home/tester".to_string()),
            }),
        };
        set_cached_runtime_location("opencode", location);

        let unified = get_omo_config_path_async(&db).await.unwrap();

        assert_eq!(
            unified.to_string_lossy(),
            r"\\wsl.localhost\Ubuntu\home\tester\.omo\omo.jsonc"
        );
    }

    #[tokio::test]
    async fn antigravity_prompt_path_wsl_direct_uses_wsl_home() {
        let _guard = TEST_RUNTIME_LOCATION_LOCK.lock().await;
        clear_runtime_location_cache();
        let (_temp_dir, db) = create_test_db().await;
        let location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::WslDirect,
            source: "custom".to_string(),
            host_path: PathBuf::from(
                r"\\wsl.localhost\Ubuntu\home\tester\.gemini\antigravity-cli",
            ),
            wsl: Some(WslLocationInfo {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/tester/.gemini/antigravity-cli".to_string(),
                linux_user_root: Some("/home/tester".to_string()),
            }),
        };
        set_cached_runtime_location("antigravity", location);

        let prompt_path = get_antigravity_prompt_path_async(&db).await.unwrap();

        // The rules file depends on $HOME, not on the module runtime root.
        assert_eq!(
            prompt_path.to_string_lossy(),
            r"\\wsl.localhost\Ubuntu\home\tester\.gemini\config\GEMINI.md"
        );
    }

    #[test]
    fn runtime_result_uses_resolved_wsl_location_when_available() {
        let fallback_location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::LocalWindows,
            source: "default".to_string(),
            host_path: PathBuf::from("C:\\Users\\tester\\.codex"),
            wsl: None,
        };
        let resolved_location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::WslDirect,
            source: "custom".to_string(),
            host_path: PathBuf::from("\\\\wsl.localhost\\Ubuntu\\home\\tester\\.codex"),
            wsl: Some(WslLocationInfo {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/tester/.codex".to_string(),
                linux_user_root: Some("/home/tester".to_string()),
            }),
        };

        let status =
            module_status_from_runtime_result("codex", Ok(resolved_location), &fallback_location);

        assert!(status.is_wsl_direct);
        assert_eq!(status.module, "codex");
        assert_eq!(status.distro.as_deref(), Some("Ubuntu"));
        assert_eq!(status.linux_path.as_deref(), Some("/home/tester/.codex"));
    }

    #[test]
    fn runtime_result_falls_back_to_non_database_location_on_error() {
        let fallback_location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::LocalWindows,
            source: "default".to_string(),
            host_path: PathBuf::from("C:\\Users\\tester\\.codex"),
            wsl: None,
        };

        let status = module_status_from_runtime_result(
            "codex",
            Err("simulated db decode failure".to_string()),
            &fallback_location,
        );

        assert_eq!(status.module, "codex");
        assert!(!status.is_wsl_direct);
        assert_eq!(
            status.source_path.as_deref(),
            Some("C:\\Users\\tester\\.codex")
        );
        assert_eq!(status.reason, None);
    }

    #[tokio::test]
    async fn claude_runtime_helpers_read_cached_custom_root_without_requery() {
        let _guard = TEST_RUNTIME_LOCATION_LOCK.lock().await;
        clear_runtime_location_cache();
        let (temp_dir, db) = create_test_db().await;
        let custom_root = temp_dir.path().join("custom-claude");

        save_claude_common_config(&db, &custom_root.to_string_lossy());

        let refreshed = refresh_runtime_location_cache_for_module_async(&db, "claude")
            .await
            .expect("refresh claude runtime cache");
        assert_eq!(refreshed.source, "custom");
        assert_eq!(refreshed.host_path, custom_root);

        delete_claude_common_config(&db);

        let sync_location = get_claude_runtime_location_sync(&db).expect("sync helper reads cache");
        let async_location = get_claude_runtime_location_async(&db)
            .await
            .expect("async helper reads cache");

        assert_eq!(sync_location.source, "custom");
        assert_eq!(async_location.source, "custom");
        assert_eq!(sync_location.host_path, refreshed.host_path);
        assert_eq!(async_location.host_path, refreshed.host_path);
    }

    #[tokio::test]
    async fn claude_default_local_derived_paths_use_default_layout() {
        let _guard = TEST_RUNTIME_LOCATION_LOCK.lock().await;
        clear_runtime_location_cache();
        let (_temp_dir, db) = create_test_db().await;
        let home_dir = local_home_dir();
        let default_root = home_dir.join(".claude");
        let location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::LocalWindows,
            source: "default".to_string(),
            host_path: default_root.clone(),
            wsl: None,
        };
        set_cached_runtime_location("claude", location.clone());

        assert_eq!(
            get_claude_mcp_config_path_from_location(&location).expect("default local mcp path"),
            local_home_claude_json_path()
        );
        assert_eq!(
            get_claude_settings_path_sync(&db).expect("default local settings path"),
            default_root.join("settings.json")
        );
        assert_eq!(
            get_claude_prompt_path_sync(&db).expect("default local prompt path"),
            default_root.join("CLAUDE.md")
        );
        assert_eq!(
            get_claude_plugin_config_path_sync(&db).expect("default local plugin config path"),
            default_root.join("config.json")
        );
        assert_eq!(
            get_claude_plugins_dir_sync(&db).expect("default local plugins dir"),
            default_root.join("plugins")
        );
        assert_eq!(
            get_tool_skills_path_sync(&db, "claude_code").expect("default local skills path"),
            default_root.join("skills")
        );
        assert_eq!(
            get_claude_wsl_target_path_async(&db, "settings.json").await,
            "~/.claude/settings.json"
        );
        assert_eq!(
            get_claude_wsl_target_path_async(&db, "plugins").await,
            "~/.claude/plugins"
        );
        assert_eq!(
            get_claude_wsl_claude_json_path_async(&db).await,
            "~/.claude.json"
        );
    }

    #[tokio::test]
    async fn claude_explicit_default_root_uses_config_dir_mcp_layout() {
        let home_dir = local_home_dir();
        let explicit_default_root = home_dir.join(".claude");
        let location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::LocalWindows,
            source: "env".to_string(),
            host_path: explicit_default_root.clone(),
            wsl: None,
        };

        assert_eq!(
            get_claude_mcp_config_path_from_location(&location).expect("explicit root mcp path"),
            explicit_default_root.join(".claude.json")
        );
    }

    #[tokio::test]
    async fn claude_custom_local_derived_paths_use_custom_root_and_remote_default_layout() {
        let _guard = TEST_RUNTIME_LOCATION_LOCK.lock().await;
        clear_runtime_location_cache();
        let (temp_dir, db) = create_test_db().await;
        let custom_root = temp_dir.path().join("custom-claude");

        save_claude_common_config(&db, &custom_root.to_string_lossy());
        refresh_runtime_location_cache_for_module_async(&db, "claude")
            .await
            .expect("refresh claude runtime cache");

        assert_eq!(
            get_claude_mcp_config_path_sync(&db).expect("sync mcp path"),
            custom_root.join(".claude.json")
        );
        assert_eq!(
            get_claude_mcp_config_path_async(&db)
                .await
                .expect("async mcp path"),
            custom_root.join(".claude.json")
        );
        assert_eq!(
            get_claude_settings_path_sync(&db).expect("sync settings path"),
            custom_root.join("settings.json")
        );
        assert_eq!(
            get_claude_settings_path_async(&db)
                .await
                .expect("async settings path"),
            custom_root.join("settings.json")
        );
        assert_eq!(
            get_claude_prompt_path_sync(&db).expect("sync prompt path"),
            custom_root.join("CLAUDE.md")
        );
        assert_eq!(
            get_claude_prompt_path_async(&db)
                .await
                .expect("async prompt path"),
            custom_root.join("CLAUDE.md")
        );
        assert_eq!(
            get_claude_plugin_config_path_sync(&db).expect("sync plugin config path"),
            custom_root.join("config.json")
        );
        assert_eq!(
            get_claude_plugin_config_path_async(&db)
                .await
                .expect("async plugin config path"),
            custom_root.join("config.json")
        );
        assert_eq!(
            get_claude_plugins_dir_sync(&db).expect("sync plugins dir"),
            custom_root.join("plugins")
        );
        assert_eq!(
            get_claude_plugins_dir_async(&db)
                .await
                .expect("async plugins dir"),
            custom_root.join("plugins")
        );
        assert_eq!(
            get_tool_skills_path_sync(&db, "claude_code").expect("sync skills path"),
            custom_root.join("skills")
        );

        assert_eq!(
            get_claude_wsl_target_path_async(&db, "settings.json").await,
            "~/.claude/settings.json"
        );
        assert_eq!(
            get_claude_wsl_target_path_async(&db, "config.json").await,
            "~/.claude/config.json"
        );
        assert_eq!(
            get_claude_wsl_target_path_async(&db, "CLAUDE.md").await,
            "~/.claude/CLAUDE.md"
        );
        assert_eq!(
            get_claude_wsl_target_path_async(&db, "plugins").await,
            "~/.claude/plugins"
        );
        assert_eq!(
            get_claude_wsl_claude_json_path_async(&db).await,
            "~/.claude.json"
        );
    }

    #[tokio::test]
    async fn claude_plugins_dir_uses_cached_plugin_cache_override() {
        let _guard = TEST_RUNTIME_LOCATION_LOCK.lock().await;
        clear_runtime_location_cache();
        let (temp_dir, db) = create_test_db().await;
        let custom_root = temp_dir.path().join("custom-claude");
        let plugin_cache_dir = temp_dir.path().join("claude-plugin-cache");
        let _env_guard = EnvVarGuard::set("CLAUDE_CODE_PLUGIN_CACHE_DIR", &plugin_cache_dir);

        save_claude_common_config(&db, &custom_root.to_string_lossy());
        refresh_runtime_location_cache_for_module_async(&db, "claude")
            .await
            .expect("refresh claude runtime cache");

        std::env::remove_var("CLAUDE_CODE_PLUGIN_CACHE_DIR");

        assert_eq!(
            get_claude_plugins_dir_sync(&db).expect("sync plugins dir from cache"),
            plugin_cache_dir
        );
        assert_eq!(
            get_claude_plugins_dir_async(&db)
                .await
                .expect("async plugins dir from cache"),
            plugin_cache_dir
        );
        assert_eq!(
            get_claude_settings_path_sync(&db).expect("settings still use config root"),
            custom_root.join("settings.json")
        );
        assert_eq!(
            get_claude_wsl_target_path_async(&db, "plugins").await,
            "~/.claude/plugins"
        );
    }

    #[tokio::test]
    async fn claude_wsl_direct_default_root_uses_default_linux_layout() {
        let _guard = TEST_RUNTIME_LOCATION_LOCK.lock().await;
        clear_runtime_location_cache();
        let (_temp_dir, db) = create_test_db().await;
        let location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::WslDirect,
            source: "default".to_string(),
            host_path: PathBuf::from(r"\\wsl.localhost\Ubuntu\home\tester\.claude"),
            wsl: Some(WslLocationInfo {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/tester/.claude".to_string(),
                linux_user_root: Some("/home/tester".to_string()),
            }),
        };
        set_cached_runtime_location("claude", location);

        assert_eq!(
            get_claude_mcp_config_path_sync(&db)
                .expect("default wsl direct mcp path")
                .to_string_lossy(),
            r"\\wsl.localhost\Ubuntu\home\tester\.claude.json"
        );
        assert_eq!(
            get_tool_skills_path_sync(&db, "claude_code")
                .expect("default wsl direct skills path")
                .to_string_lossy(),
            r"\\wsl.localhost\Ubuntu\home\tester\.claude\skills"
        );
        assert_eq!(
            get_claude_wsl_target_path_async(&db, "settings.json").await,
            "/home/tester/.claude/settings.json"
        );
        assert_eq!(
            get_claude_wsl_target_path_async(&db, "plugins").await,
            "/home/tester/.claude/plugins"
        );
        assert_eq!(
            get_claude_wsl_claude_json_path_async(&db).await,
            "/home/tester/.claude.json"
        );
    }

    #[tokio::test]
    async fn claude_wsl_direct_custom_root_paths_use_linux_config_root() {
        let _guard = TEST_RUNTIME_LOCATION_LOCK.lock().await;
        clear_runtime_location_cache();
        let (_temp_dir, db) = create_test_db().await;
        let wsl_root = r"\\wsl.localhost\Ubuntu\home\tester\custom-claude";

        save_claude_common_config(&db, wsl_root);
        refresh_runtime_location_cache_for_module_async(&db, "claude")
            .await
            .expect("refresh claude runtime cache");
        let wsl_root_path = PathBuf::from(wsl_root);

        assert_eq!(
            get_claude_settings_path_sync(&db).expect("wsl direct settings path"),
            wsl_root_path.join("settings.json")
        );
        assert_eq!(
            get_claude_prompt_path_sync(&db).expect("wsl direct prompt path"),
            wsl_root_path.join("CLAUDE.md")
        );
        assert_eq!(
            get_claude_plugin_config_path_sync(&db).expect("wsl direct plugin config path"),
            wsl_root_path.join("config.json")
        );
        assert_eq!(
            get_claude_plugins_dir_sync(&db).expect("wsl direct plugins dir"),
            wsl_root_path.join("plugins")
        );
        assert_eq!(
            get_claude_wsl_target_path_async(&db, "settings.json").await,
            "/home/tester/custom-claude/settings.json"
        );
        assert_eq!(
            get_claude_wsl_target_path_async(&db, "config.json").await,
            "/home/tester/custom-claude/config.json"
        );
        assert_eq!(
            get_claude_wsl_target_path_async(&db, "CLAUDE.md").await,
            "/home/tester/custom-claude/CLAUDE.md"
        );
        assert_eq!(
            get_claude_wsl_target_path_async(&db, "plugins").await,
            "/home/tester/custom-claude/plugins"
        );
        assert_eq!(
            get_claude_wsl_claude_json_path_async(&db).await,
            "/home/tester/custom-claude/.claude.json"
        );
        assert_eq!(
            get_claude_mcp_config_path_sync(&db)
                .expect("claude wsl direct mcp path")
                .to_string_lossy(),
            r"\\wsl.localhost\Ubuntu\home\tester\custom-claude\.claude.json"
        );
        assert_eq!(
            get_tool_skills_path_sync(&db, "claude_code")
                .expect("claude skills path")
                .to_string_lossy(),
            r"\\wsl.localhost\Ubuntu\home\tester\custom-claude\skills"
        );
    }

    #[tokio::test]
    async fn pi_wsl_direct_custom_root_skills_path_uses_linux_root() {
        let _guard = TEST_RUNTIME_LOCATION_LOCK.lock().await;
        clear_runtime_location_cache();
        let (_temp_dir, db) = create_test_db().await;
        let location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::WslDirect,
            source: "custom".to_string(),
            host_path: PathBuf::from(r"\\wsl.localhost\Ubuntu\home\tester\custom-pi"),
            wsl: Some(WslLocationInfo {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/tester/custom-pi".to_string(),
                linux_user_root: Some("/home/tester".to_string()),
            }),
        };
        set_cached_runtime_location("pi", location);

        assert_eq!(
            get_tool_skills_path_sync(&db, "pi")
                .expect("pi sync skills path")
                .to_string_lossy(),
            r"\\wsl.localhost\Ubuntu\home\tester\custom-pi\skills"
        );
        assert_eq!(
            get_tool_skills_path_async(&db, "pi")
                .await
                .expect("pi async skills path")
                .to_string_lossy(),
            r"\\wsl.localhost\Ubuntu\home\tester\custom-pi\skills"
        );
        assert_eq!(
            get_tool_mcp_config_path_sync(&db, "pi")
                .expect("pi sync mcp path")
                .to_string_lossy(),
            r"\\wsl.localhost\Ubuntu\home\tester\custom-pi\mcp.json"
        );
        assert_eq!(
            get_tool_mcp_config_path_async(&db, "pi")
                .await
                .expect("pi async mcp path")
                .to_string_lossy(),
            r"\\wsl.localhost\Ubuntu\home\tester\custom-pi\mcp.json"
        );
    }

    #[tokio::test]
    async fn pi_runtime_refresh_preserves_unconfirmed_stored_wsl_dot_pi_root() {
        let _guard = TEST_RUNTIME_LOCATION_LOCK.lock().await;
        clear_runtime_location_cache();
        let (_temp_dir, db) = create_test_db().await;
        db.with_conn(|conn| {
            db_put(
                conn,
                DbTable::PiSettingsConfig,
                "common",
                &serde_json::json!({
                    "root_dir": r"\\wsl.localhost\Ubuntu\home\tester\.pi",
                    "updated_at": "2026-01-01T00:00:00Z"
                }),
            )
        })
        .expect("save legacy Pi root");

        let location = refresh_runtime_location_cache_for_module_async(&db, "pi")
            .await
            .expect("refresh Pi runtime cache");

        assert_eq!(
            location.host_path.to_string_lossy(),
            r"\\wsl.localhost\Ubuntu\home\tester\.pi"
        );
        assert_eq!(
            location.wsl.as_ref().map(|wsl| wsl.linux_path.as_str()),
            Some("/home/tester/.pi")
        );
        let stored_record = db
            .with_conn(|conn| db_get(conn, DbTable::PiSettingsConfig, "common"))
            .expect("read migrated Pi root")
            .expect("Pi settings record");
        assert_eq!(
            stored_record
                .get("root_dir")
                .and_then(serde_json::Value::as_str),
            Some(r"\\wsl.localhost\Ubuntu\home\tester\.pi")
        );
    }
}
