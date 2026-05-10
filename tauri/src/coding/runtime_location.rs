use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::coding::open_code::shell_env;
use crate::coding::{claude_code, codex, gemini_cli, open_claw, open_code};

const MODULE_KEYS: [&str; 5] = ["opencode", "claude", "codex", "openclaw", "geminicli"];
const OMO_LEGACY_BASENAME: &str = "oh-my-opencode";
const OMO_CANONICAL_BASENAME: &str = "oh-my-openagent";
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
        "openclaw" => Some("openclaw"),
        "geminicli" | "gemini_cli" | "gemini" => Some("geminicli"),
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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
        Some(_) | None => Err(format!("Unsupported runtime module: {}", module)),
    }
}

pub async fn refresh_runtime_location_cache_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<(), String> {
    for module in MODULE_KEYS {
        refresh_runtime_location_cache_for_module_async(db, module).await?;
    }

    Ok(())
}

pub fn get_wsl_direct_status_map(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Vec<WslDirectModuleStatus>, String> {
    let mut statuses = Vec::with_capacity(MODULE_KEYS.len());

    for module in MODULE_KEYS {
        statuses.push(get_wsl_direct_status_with_fallback(db, module).await?);
    }

    Ok(statuses)
}

pub fn get_wsl_direct_status_for_module(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    module: &str,
) -> Result<WslDirectModuleStatus, String> {
    get_wsl_direct_status_with_fallback(db, module).await
}

pub fn get_opencode_runtime_location_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("opencode"))
}

pub async fn get_opencode_runtime_location_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "opencode").await
}

pub fn get_opencode_config_dir_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    get_opencode_runtime_location_sync(db)?
        .host_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Failed to determine OpenCode config directory".to_string())
}

pub async fn get_opencode_config_dir_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    get_opencode_runtime_location_async(db)
        .await?
        .host_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Failed to determine OpenCode config directory".to_string())
}

pub fn get_opencode_prompt_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_opencode_config_dir_sync(db)?.join("AGENTS.md"))
}

pub async fn get_opencode_prompt_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_opencode_config_dir_async(db).await?.join("AGENTS.md"))
}

pub fn get_omo_config_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    let dir = get_opencode_config_dir_sync(db)?;
    Ok(resolve_omo_config_path_from_dir(&dir))
}

pub async fn get_omo_config_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    let dir = get_opencode_config_dir_async(db).await?;
    Ok(resolve_omo_config_path_from_dir(&dir))
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

pub fn get_omos_config_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_opencode_config_dir_sync(db)?.join("oh-my-opencode-slim.json"))
}

pub async fn get_omos_config_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_opencode_config_dir_async(db)
        .await?
        .join("oh-my-opencode-slim.json"))
}

pub fn get_opencode_wsl_target_path(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> String {
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

pub async fn get_opencode_wsl_target_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> String {
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

pub fn get_opencode_prompt_wsl_target_path(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> String {
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

pub async fn get_opencode_prompt_wsl_target_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> String {
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("claude"))
}

pub async fn get_claude_runtime_location_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "claude").await
}

async fn resolve_claude_runtime_location_uncached_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    let path_info = get_custom_path_from_query(
        db,
        "SELECT * OMIT id FROM claude_common_config:`common` LIMIT 1",
        |value| {
            crate::coding::claude_code::adapter::from_db_value_common(value)
                .root_dir
                .filter(|path| !path.trim().is_empty())
        },
    )
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

pub fn get_claude_settings_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_claude_runtime_location_sync(db)?
        .host_path
        .join("settings.json"))
}

pub async fn get_claude_settings_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_claude_runtime_location_async(db)
        .await?
        .host_path
        .join("settings.json"))
}

pub fn get_claude_plugin_config_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_claude_runtime_location_sync(db)?
        .host_path
        .join("config.json"))
}

pub async fn get_claude_plugin_config_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_claude_runtime_location_async(db)
        .await?
        .host_path
        .join("config.json"))
}

pub fn get_claude_plugins_dir_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    if let Some(path) = get_cached_claude_plugins_dir() {
        return Ok(path);
    }

    Ok(get_claude_runtime_location_sync(db)?
        .host_path
        .join("plugins"))
}

pub async fn get_claude_plugins_dir_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    if let Some(path) = get_cached_claude_plugins_dir() {
        return Ok(path);
    }

    let location = get_claude_runtime_location_async(db).await?;
    let plugins_dir = resolve_claude_plugins_dir_uncached(&location);
    set_cached_claude_plugins_dir(plugins_dir.clone());
    Ok(plugins_dir)
}

pub fn get_claude_prompt_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_claude_runtime_location_sync(db)?
        .host_path
        .join("CLAUDE.md"))
}

pub async fn get_claude_prompt_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_claude_runtime_location_async(db)
        .await?
        .host_path
        .join("CLAUDE.md"))
}

pub fn get_claude_mcp_config_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    let location = get_claude_runtime_location_sync(db)?;
    get_claude_mcp_config_path_from_location(&location)
}

pub async fn get_claude_mcp_config_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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

pub fn get_claude_wsl_target_path(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    file_name: &str,
) -> String {
    match get_claude_runtime_location_sync(db) {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.claude/{}", file_name)),
        Err(_) => format!("~/.claude/{}", file_name),
    }
}

pub async fn get_claude_wsl_target_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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

pub fn get_claude_wsl_claude_json_path(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> String {
    match get_claude_runtime_location_sync(db) {
        Ok(location) => get_claude_wsl_claude_json_path_from_location(&location)
            .unwrap_or_else(|| "~/.claude.json".to_string()),
        Err(_) => "~/.claude.json".to_string(),
    }
}

pub async fn get_claude_wsl_claude_json_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> String {
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("codex"))
}

pub async fn get_codex_runtime_location_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "codex").await
}

async fn resolve_codex_runtime_location_uncached_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    let path_info = get_custom_path_from_query(
        db,
        "SELECT * OMIT id FROM codex_common_config:`common` LIMIT 1",
        |value| {
            crate::coding::codex::adapter::from_db_value_common(value)
                .root_dir
                .filter(|path| !path.trim().is_empty())
        },
    )
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

pub fn get_codex_auth_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_codex_runtime_location_sync(db)?
        .host_path
        .join("auth.json"))
}

pub async fn get_codex_auth_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_codex_runtime_location_async(db)
        .await?
        .host_path
        .join("auth.json"))
}

pub fn get_codex_config_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_codex_runtime_location_sync(db)?
        .host_path
        .join("config.toml"))
}

pub async fn get_codex_config_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
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

pub fn get_codex_prompt_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(resolve_codex_prompt_file_path(
        &get_codex_runtime_location_sync(db)?.host_path,
    ))
}

pub async fn get_codex_prompt_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(resolve_codex_prompt_file_path(
        &get_codex_runtime_location_async(db).await?.host_path,
    ))
}

pub fn get_codex_wsl_target_path(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    file_name: &str,
) -> String {
    match get_codex_runtime_location_sync(db) {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.codex/{}", file_name)),
        Err(_) => format!("~/.codex/{}", file_name),
    }
}

pub async fn get_codex_wsl_target_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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

pub fn get_gemini_cli_runtime_location_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("geminicli"))
}

pub async fn get_gemini_cli_runtime_location_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "geminicli").await
}

async fn resolve_gemini_cli_runtime_location_uncached_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    let path_info = get_custom_path_from_query(
        db,
        "SELECT * OMIT id FROM gemini_cli_common_config:`common` LIMIT 1",
        |value| {
            crate::coding::gemini_cli::adapter::from_db_value_common(value)
                .root_dir
                .filter(|path| !path.trim().is_empty())
        },
    )
    .await;

    let (path, source) = if let Some(path) = path_info {
        (PathBuf::from(path), "custom".to_string())
    } else {
        resolve_gemini_cli_path_without_db()
    };

    Ok(build_runtime_location(path, source))
}

pub fn get_gemini_cli_env_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_sync(db)?
        .host_path
        .join(".env"))
}

pub async fn get_gemini_cli_env_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_async(db)
        .await?
        .host_path
        .join(".env"))
}

pub fn get_gemini_cli_settings_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_sync(db)?
        .host_path
        .join("settings.json"))
}

pub async fn get_gemini_cli_settings_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_async(db)
        .await?
        .host_path
        .join("settings.json"))
}

pub fn get_gemini_cli_prompt_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    let location = get_gemini_cli_runtime_location_sync(db)?;
    Ok(gemini_cli::get_gemini_cli_prompt_path_from_root(
        &location.host_path,
    ))
}

pub async fn get_gemini_cli_prompt_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    let location = get_gemini_cli_runtime_location_async(db).await?;
    Ok(gemini_cli::get_gemini_cli_prompt_path_from_root(
        &location.host_path,
    ))
}

pub fn get_gemini_cli_oauth_creds_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_sync(db)?
        .host_path
        .join("oauth_creds.json"))
}

pub async fn get_gemini_cli_oauth_creds_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_async(db)
        .await?
        .host_path
        .join("oauth_creds.json"))
}

pub fn get_gemini_cli_tmp_dir_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_sync(db)?
        .host_path
        .join("tmp"))
}

pub async fn get_gemini_cli_tmp_dir_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<PathBuf, String> {
    Ok(get_gemini_cli_runtime_location_async(db)
        .await?
        .host_path
        .join("tmp"))
}

pub fn get_gemini_cli_wsl_target_path(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    file_name: &str,
) -> String {
    match get_gemini_cli_runtime_location_sync(db) {
        Ok(location) => location
            .wsl
            .map(|wsl| format!("{}/{}", wsl.linux_path.trim_end_matches('/'), file_name))
            .unwrap_or_else(|| format!("~/.gemini/{}", file_name)),
        Err(_) => format!("~/.gemini/{}", file_name),
    }
}

pub async fn get_gemini_cli_wsl_target_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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

pub fn get_gemini_cli_prompt_wsl_target_path(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> String {
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

pub async fn get_gemini_cli_prompt_wsl_target_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> String {
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

pub fn get_openclaw_runtime_location_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    let _ = db;
    Ok(get_cached_or_fallback_runtime_location("openclaw"))
}

pub async fn get_openclaw_runtime_location_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    get_cached_or_refresh_runtime_location_async(db, "openclaw").await
}

pub fn get_openclaw_wsl_target_path(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> String {
    match get_openclaw_runtime_location_sync(db) {
        Ok(location) => location
            .wsl
            .map(|wsl| wsl.linux_path)
            .unwrap_or_else(|| "~/.openclaw/openclaw.json".to_string()),
        Err(_) => "~/.openclaw/openclaw.json".to_string(),
    }
}

pub async fn get_openclaw_wsl_target_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> String {
    match get_openclaw_runtime_location_async(db).await {
        Ok(location) => location
            .wsl
            .map(|wsl| wsl.linux_path)
            .unwrap_or_else(|| "~/.openclaw/openclaw.json".to_string()),
        Err(_) => "~/.openclaw/openclaw.json".to_string(),
    }
}

pub fn get_tool_skills_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool_key: &str,
) -> Option<PathBuf> {
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
        _ => None,
    }
}

pub async fn get_tool_skills_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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

pub fn get_tool_mcp_config_path_sync(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool_key: &str,
) -> Option<PathBuf> {
    match tool_key {
        "claude_code" => get_claude_mcp_config_path_sync(db).ok(),
        "codex" => get_codex_config_path_sync(db).ok(),
        "opencode" => get_opencode_runtime_location_sync(db)
            .ok()
            .map(|location| location.host_path),
        "openclaw" => get_openclaw_runtime_location_sync(db)
            .ok()
            .map(|location| location.host_path),
        _ => None,
    }
}

pub async fn get_tool_mcp_config_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool_key: &str,
) -> Option<PathBuf> {
    match tool_key {
        "claude_code" => get_claude_mcp_config_path_async(db).await.ok(),
        "codex" => get_codex_config_path_async(db).await.ok(),
        "opencode" => get_opencode_runtime_location_async(db)
            .await
            .ok()
            .map(|location| location.host_path),
        "openclaw" => get_openclaw_runtime_location_async(db)
            .await
            .ok()
            .map(|location| location.host_path),
        _ => None,
    }
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
        "openclaw" => resolve_openclaw_path_without_db(),
        "geminicli" => resolve_gemini_cli_path_without_db(),
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

    (
        PathBuf::from(
            open_code::get_default_config_path()
                .unwrap_or_else(|_| "~/.config/opencode/opencode.jsonc".to_string()),
        ),
        "default".to_string(),
    )
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

async fn resolve_opencode_runtime_location_uncached_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    let (path, source) = resolve_opencode_config_path_async(db).await?;
    Ok(build_runtime_location(path, source))
}

async fn resolve_opencode_config_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<(PathBuf, String), String> {
    let custom_path = get_custom_path_from_query(
        db,
        "SELECT *, type::string(id) as id FROM opencode_common_config:`common` LIMIT 1",
        |value| {
            open_code::adapter::from_db_value(value)
                .config_path
                .filter(|path| !path.trim().is_empty())
        },
    )
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

    Ok((
        PathBuf::from(open_code::get_default_config_path()?),
        "default".to_string(),
    ))
}

async fn resolve_openclaw_runtime_location_uncached_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<RuntimeLocationInfo, String> {
    let (path, source) = resolve_openclaw_config_path_async(db).await?;
    Ok(build_runtime_location(path, source))
}

async fn resolve_openclaw_config_path_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<(PathBuf, String), String> {
    let custom_path = get_custom_path_from_query(
        db,
        "SELECT *, type::string(id) as id FROM openclaw_common_config:`common` LIMIT 1",
        |value| {
            open_claw::adapter::from_db_value(value)
                .config_path
                .filter(|path| !path.trim().is_empty())
        },
    )
    .await;

    if let Some(path) = custom_path {
        return Ok((PathBuf::from(path), "custom".to_string()));
    }

    let path = open_claw::get_default_config_path_for_runtime()?;
    Ok((PathBuf::from(path), "default".to_string()))
}

async fn get_custom_path_from_query<F>(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    query: &str,
    extractor: F,
) -> Option<String>
where
    F: Fn(Value) -> Option<String>,
{
    let mut result = db.query(query).await.ok()?;
    let records: Vec<Value> = result.take(0).ok()?;
    let record = records.into_iter().next()?;
    extractor(record)
}

#[cfg(test)]
mod tests {
    use super::{
        CODEX_DEFAULT_PROMPT_FILE_NAME, CODEX_OVERRIDE_PROMPT_FILE_NAME, RuntimeLocationInfo,
        RuntimeLocationMode, WslLocationInfo, clear_runtime_location_cache,
        expand_home_from_user_root, get_claude_mcp_config_path_async,
        get_claude_mcp_config_path_from_location, get_claude_mcp_config_path_sync,
        get_claude_plugin_config_path_async, get_claude_plugin_config_path_sync,
        get_claude_plugins_dir_async, get_claude_plugins_dir_sync, get_claude_prompt_path_async,
        get_claude_prompt_path_sync, get_claude_runtime_location_async,
        get_claude_runtime_location_sync, get_claude_settings_path_async,
        get_claude_settings_path_sync, get_claude_wsl_claude_json_path_async,
        get_claude_wsl_target_path_async, get_tool_skills_path_sync,
        module_status_from_runtime_result, refresh_runtime_location_cache_for_module_async,
        replace_path_file_name, resolve_codex_prompt_file_path, set_cached_runtime_location,
    };
    use std::ffi::OsString;
    use std::path::PathBuf;
    use surrealdb::Surreal;
    use surrealdb::engine::local::SurrealKv;
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

    async fn create_test_db() -> (tempfile::TempDir, Surreal<surrealdb::engine::local::Db>) {
        let temp_dir = tempfile::tempdir().expect("create temp db dir");
        let db_path = temp_dir.path().join("surreal");
        let db = Surreal::new::<SurrealKv>(db_path)
            .await
            .expect("open surreal test db");
        db.use_ns("ai_toolbox")
            .use_db("main")
            .await
            .expect("select surreal test namespace");
        (temp_dir, db)
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

        db.query("UPSERT claude_common_config:`common` CONTENT $data")
            .bind((
                "data",
                serde_json::json!({
                    "config": "{}",
                    "root_dir": custom_root.to_string_lossy().to_string(),
                }),
            ))
            .await
            .expect("save claude common config");

        let refreshed = refresh_runtime_location_cache_for_module_async(&db, "claude")
            .await
            .expect("refresh claude runtime cache");
        assert_eq!(refreshed.source, "custom");
        assert_eq!(refreshed.host_path, custom_root);

        db.query("DELETE claude_common_config:`common`")
            .await
            .expect("delete claude common config");

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

        db.query("UPSERT claude_common_config:`common` CONTENT $data")
            .bind((
                "data",
                serde_json::json!({
                    "config": "{}",
                    "root_dir": custom_root.to_string_lossy().to_string(),
                }),
            ))
            .await
            .expect("save claude common config");
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

        db.query("UPSERT claude_common_config:`common` CONTENT $data")
            .bind((
                "data",
                serde_json::json!({
                    "config": "{}",
                    "root_dir": custom_root.to_string_lossy().to_string(),
                }),
            ))
            .await
            .expect("save claude common config");
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

        db.query("UPSERT claude_common_config:`common` CONTENT $data")
            .bind((
                "data",
                serde_json::json!({
                    "config": "{}",
                    "root_dir": wsl_root,
                }),
            ))
            .await
            .expect("save claude common config");
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
}
