//! Read-only import from CC Switch (`~/.cc-switch/cc-switch.db`).
//!
//! - Providers: `list_cc_switch_providers` (per app_type)
//! - MCP: `list_cc_switch_mcp_servers` (table `mcp_servers`, used by MCP scan/import)
//! - Skills: discovered via disk `~/.cc-switch/skills` in skills onboarding (not this module)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use log::info;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

const CC_SWITCH_DB_CACHE_TTL_SECS: u64 = 30;
static CC_SWITCH_DB_CACHE: Mutex<Option<(bool, Instant)>> = Mutex::new(None);

const MSG_DB_NOT_FOUND: &str = "cc_switch_db_not_found";
const MSG_DB_OPEN_FAILED: &str = "cc_switch_db_open_failed";
const MSG_NO_PROVIDERS: &str = "cc_switch_no_providers";

/// Synthetic tool_key for MCP scan/import grouping (matches Skills EXTRA source key).
pub const CC_SWITCH_MCP_TOOL_KEY: &str = "cc_switch";
/// Display name in ImportMcpModal groups.
pub const CC_SWITCH_MCP_TOOL_NAME: &str = "CC Switch";

/// One MCP server row extracted from CCS `mcp_servers` (channel config only).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CcSwitchMcpCandidate {
    pub id: String,
    pub name: String,
    pub server_type: String,
    pub server_config: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CcSwitchProviderCandidate {
    /// Modal compare key: row tools use `ccs:{app}:{raw_id}`; map tools use raw_id.
    pub provider_id: String,
    pub raw_id: String,
    pub name: String,
    pub app_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub normalized_category: String,
    /// Row tools: JSON string ready for create*.settingsConfig.
    /// Map tools: object for merge (OpenClaw / OpenCode).
    pub settings_config: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_settings_config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url_preview: Option<String>,
    pub has_api_key: bool,
    pub is_local_endpoint: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_provider_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CcSwitchDiscovery {
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_path: Option<String>,
    pub providers: Vec<CcSwitchProviderCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn default_cc_switch_db_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".cc-switch").join("cc-switch.db"))
}

fn is_local_host(host: &str) -> bool {
    let host = host
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    host == "127.0.0.1" || host == "localhost" || host == "::1"
}

fn parse_host_from_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    // Prefer url crate-free parse for http(s)://host[:port]/path
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);
    let host = if host_port.starts_with('[') {
        host_port
            .trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or(host_port)
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    let host = host.trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn is_local_endpoint_url(url: Option<&str>) -> bool {
    url.and_then(parse_host_from_url)
        .map(|host| is_local_host(&host))
        .unwrap_or(false)
}

pub fn normalize_category(raw: Option<&str>) -> String {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some("official") => "official".to_string(),
        Some("aggregator") => "third_party".to_string(),
        Some("omo") => "omo".to_string(),
        Some("third_party") => "third_party".to_string(),
        Some("custom") => "custom".to_string(),
        _ => "custom".to_string(),
    }
}

fn source_provider_id(app_type: &str, raw_id: &str) -> String {
    format!("ccs:{app_type}:{raw_id}")
}

fn env_string_map(env: &Value) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(obj) = env.as_object() else {
        return out;
    };
    for (key, value) in obj {
        if key.trim().is_empty() {
            continue;
        }
        if let Some(s) = value.as_str() {
            out.insert(key.clone(), s.to_string());
            continue;
        }
        // Coerce simple scalars to string so CCS numeric-ish values still import.
        if value.is_number() || value.is_boolean() {
            out.insert(key.clone(), value.to_string());
        }
    }
    out
}

fn first_env_value(env: &BTreeMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| env.get(*key).map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
}

fn extract_claude_candidate(
    raw_id: &str,
    name: &str,
    category: Option<&str>,
    settings: &Value,
    website_url: Option<String>,
    notes: Option<String>,
    icon: Option<String>,
    icon_color: Option<String>,
) -> Option<CcSwitchProviderCandidate> {
    let normalized = normalize_category(category);
    if normalized == "omo" {
        return None;
    }

    let env_value = settings.get("env").cloned().unwrap_or_else(|| json!({}));
    let env_map = env_string_map(&env_value);
    let env_object: Map<String, Value> = env_map
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();

    let settings_string = serde_json::to_string(&json!({ "env": env_object })).ok()?;

    // Optional extra: non-env top-level objects that look like settings.json fields.
    let mut extra_object = Map::new();
    if let Some(root) = settings.as_object() {
        for (key, value) in root {
            if key == "env" || key == "model" {
                continue;
            }
            if value.is_object() {
                extra_object.insert(key.clone(), value.clone());
            }
        }
    }
    let extra_settings_config = if extra_object.is_empty() {
        Some("{}".to_string())
    } else {
        Some(serde_json::to_string(&Value::Object(extra_object)).unwrap_or_else(|_| "{}".into()))
    };

    let base_url = first_env_value(&env_map, &["ANTHROPIC_BASE_URL"]);
    let api_key = first_env_value(&env_map, &["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"]);
    let model_preview = first_env_value(
        &env_map,
        &[
            "ANTHROPIC_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        ],
    );
    let source_id = source_provider_id("claude", raw_id);

    Some(CcSwitchProviderCandidate {
        provider_id: source_id.clone(),
        raw_id: raw_id.to_string(),
        name: name.to_string(),
        app_type: "claude".to_string(),
        category: category.map(|s| s.to_string()),
        normalized_category: normalized,
        settings_config: Value::String(settings_string),
        extra_settings_config,
        website_url,
        notes,
        icon,
        icon_color,
        base_url_preview: base_url.clone(),
        has_api_key: api_key.is_some(),
        is_local_endpoint: is_local_endpoint_url(base_url.as_deref()),
        model_preview,
        source_provider_id: Some(source_id),
    })
}

/// Keep only channel-related top-level keys when rebuilding Codex TOML.
const CODEX_TOP_LEVEL_KEEP: &[&str] = &[
    "model_provider",
    "model",
    "model_reasoning_effort",
    "disable_response_storage",
    "model_context_window",
    "model_max_output_tokens",
    "preferred_auth_method",
];

fn toml_value_to_string(value: &toml::Value) -> Option<String> {
    match value {
        toml::Value::String(s) => Some(s.clone()),
        toml::Value::Integer(i) => Some(i.to_string()),
        toml::Value::Float(f) => Some(f.to_string()),
        toml::Value::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

fn rebuild_codex_config_toml(raw_config: &str) -> Option<(String, Option<String>, Option<String>)> {
    let parsed: toml::Value = toml::from_str(raw_config).ok()?;
    let root = parsed.as_table()?;

    let mut out = toml::map::Map::new();

    for key in CODEX_TOP_LEVEL_KEEP {
        if let Some(value) = root.get(*key) {
            out.insert((*key).to_string(), value.clone());
        }
    }

    let model_provider_id = root
        .get("model_provider")
        .and_then(toml_value_to_string)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mut base_url: Option<String> = None;
    if let Some(providers) = root.get("model_providers").and_then(|v| v.as_table()) {
        let mut kept_providers = toml::map::Map::new();
        if let Some(ref id) = model_provider_id {
            if let Some(section) = providers.get(id) {
                kept_providers.insert(id.clone(), section.clone());
                if let Some(table) = section.as_table() {
                    base_url = table.get("base_url").and_then(toml_value_to_string);
                }
            }
        } else if providers.len() == 1 {
            // Single provider section without model_provider pointer.
            for (id, section) in providers {
                kept_providers.insert(id.clone(), section.clone());
                if let Some(table) = section.as_table() {
                    base_url = table.get("base_url").and_then(toml_value_to_string);
                }
                if out.get("model_provider").is_none() {
                    out.insert(
                        "model_provider".to_string(),
                        toml::Value::String(id.clone()),
                    );
                }
            }
        }

        if !kept_providers.is_empty() {
            out.insert(
                "model_providers".to_string(),
                toml::Value::Table(kept_providers),
            );
        }
    }

    // Empty shell: no provider section and no model — skip (e.g. MCP-only OneHub).
    let has_provider_table = out.get("model_providers").is_some();
    let has_model = out.get("model").is_some();
    if !has_provider_table && !has_model {
        return None;
    }

    let model_preview = out.get("model").and_then(toml_value_to_string);
    let config_string = toml::to_string(&toml::Value::Table(out)).ok()?;
    Some((config_string, base_url, model_preview))
}

fn extract_codex_candidate(
    raw_id: &str,
    name: &str,
    category: Option<&str>,
    settings: &Value,
    website_url: Option<String>,
    notes: Option<String>,
    icon: Option<String>,
    icon_color: Option<String>,
) -> Option<CcSwitchProviderCandidate> {
    let normalized = normalize_category(category);
    if normalized == "omo" {
        return None;
    }

    let auth_obj = settings.get("auth").cloned().unwrap_or_else(|| json!({}));
    let mut auth_map = Map::new();
    if let Some(obj) = auth_obj.as_object() {
        for (key, value) in obj {
            if let Some(s) = value.as_str() {
                if !s.trim().is_empty() {
                    auth_map.insert(key.clone(), Value::String(s.to_string()));
                }
            }
        }
    }

    let config_raw = settings
        .get("config")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let (config_string, base_url, model_preview) = if config_raw.trim().is_empty() {
        // Official empty config is still importable.
        (String::new(), None, None)
    } else {
        rebuild_codex_config_toml(&config_raw)?
    };

    // Official with empty auth+config: allow.
    let has_api_key = auth_map
        .get("OPENAI_API_KEY")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    if config_string.trim().is_empty() && auth_map.is_empty() && normalized != "official" {
        return None;
    }

    let settings_string = serde_json::to_string(&json!({
        "auth": Value::Object(auth_map),
        "config": config_string,
    }))
    .ok()?;

    let source_id = source_provider_id("codex", raw_id);

    Some(CcSwitchProviderCandidate {
        provider_id: source_id.clone(),
        raw_id: raw_id.to_string(),
        name: name.to_string(),
        app_type: "codex".to_string(),
        category: category.map(|s| s.to_string()),
        normalized_category: normalized,
        settings_config: Value::String(settings_string),
        extra_settings_config: None,
        website_url,
        notes,
        icon,
        icon_color,
        base_url_preview: base_url.clone(),
        has_api_key,
        is_local_endpoint: is_local_endpoint_url(base_url.as_deref()),
        model_preview,
        source_provider_id: Some(source_id),
    })
}

fn extract_gemini_candidate(
    raw_id: &str,
    name: &str,
    category: Option<&str>,
    settings: &Value,
    website_url: Option<String>,
    notes: Option<String>,
    icon: Option<String>,
    icon_color: Option<String>,
) -> Option<CcSwitchProviderCandidate> {
    let normalized = normalize_category(category);
    if normalized == "omo" {
        return None;
    }

    let env_value = settings.get("env").cloned().unwrap_or_else(|| json!({}));
    let env_map = env_string_map(&env_value);
    let env_object: Map<String, Value> = env_map
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();

    let mut config_object = Map::new();
    if let Some(cfg) = settings.get("config").and_then(|v| v.as_object()) {
        for (key, value) in cfg {
            if key == "mcpServers" {
                continue;
            }
            config_object.insert(key.clone(), value.clone());
        }
    }

    let settings_string = serde_json::to_string(&json!({
        "env": env_object,
        "config": Value::Object(config_object),
    }))
    .ok()?;

    let base_url = first_env_value(
        &env_map,
        &[
            "GOOGLE_GEMINI_BASE_URL",
            "GEMINI_BASE_URL",
            "GOOGLE_API_BASE_URL",
        ],
    );
    let api_key = first_env_value(
        &env_map,
        &["GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_GENAI_API_KEY"],
    );
    let model_preview = first_env_value(&env_map, &["GEMINI_MODEL", "GOOGLE_MODEL"]);
    let source_id = source_provider_id("gemini", raw_id);

    Some(CcSwitchProviderCandidate {
        provider_id: source_id.clone(),
        raw_id: raw_id.to_string(),
        name: name.to_string(),
        app_type: "gemini".to_string(),
        category: category.map(|s| s.to_string()),
        normalized_category: normalized,
        settings_config: Value::String(settings_string),
        extra_settings_config: None,
        website_url,
        notes,
        icon,
        icon_color,
        base_url_preview: base_url.clone(),
        has_api_key: api_key.is_some(),
        is_local_endpoint: is_local_endpoint_url(base_url.as_deref()),
        model_preview,
        source_provider_id: Some(source_id),
    })
}

fn open_readonly_db(path: &Path) -> Result<Connection, String> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("{MSG_DB_OPEN_FAILED}: {e}"))
}

fn parse_tags_json(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw.trim())
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Infer stdio/http/sse + normalized config from CCS `server_config` JSON.
/// Returns None when the row has no usable command/url.
///
/// Keeps unknown channel fields (e.g. `cwd`, timeouts) so import matches tool-file
/// parse behavior more closely than a strict whitelist. Drops only the transport
/// `type` key (stored separately as `server_type`) and normalizes URL aliases.
pub fn parse_cc_switch_mcp_config(config: &Value) -> Option<(String, Value)> {
    let explicit = config
        .get("type")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());

    let command = config
        .get("command")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let url = config
        .get("url")
        .or_else(|| config.get("serverUrl"))
        .or_else(|| config.get("httpUrl"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let server_type = match explicit.as_deref() {
        Some("sse") => "sse".to_string(),
        Some("http") => "http".to_string(),
        Some("stdio") | Some("local") => "stdio".to_string(),
        _ if command.is_some() => "stdio".to_string(),
        _ if url.is_some() => "http".to_string(),
        _ => return None,
    };

    if server_type == "stdio" {
        command.as_ref()?;
    } else {
        url.as_ref()?;
    }

    let mut obj = match config.as_object() {
        Some(map) => map.clone(),
        None => return None,
    };
    // Center store uses dedicated server_type; drop transport type from config body.
    obj.remove("type");
    // Normalize URL aliases used by some tool formats / CCS variants.
    if !obj
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some()
    {
        if let Some(aliased) = obj
            .remove("serverUrl")
            .or_else(|| obj.remove("httpUrl"))
            .filter(|v| v.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false))
        {
            obj.insert("url".to_string(), aliased);
        }
    } else {
        obj.remove("serverUrl");
        obj.remove("httpUrl");
    }

    Some((server_type, Value::Object(obj)))
}

fn list_mcp_from_db(path: &Path) -> Result<Vec<CcSwitchMcpCandidate>, String> {
    let conn = open_readonly_db(path)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, server_config, description, tags
             FROM mcp_servers
             ORDER BY name ASC, id ASC",
        )
        .map_err(|e| format!("{MSG_DB_OPEN_FAILED}: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4).unwrap_or_else(|_| "[]".to_string()),
            ))
        })
        .map_err(|e| format!("{MSG_DB_OPEN_FAILED}: {e}"))?;

    let mut out = Vec::new();
    for row in rows {
        let (id, name, server_config_str, description, tags_raw) =
            row.map_err(|e| format!("{MSG_DB_OPEN_FAILED}: {e}"))?;
        let name = name.trim().to_string();
        let name = if name.is_empty() {
            id.trim().to_string()
        } else {
            name
        };
        if name.is_empty() {
            continue;
        }
        let config: Value = match serde_json::from_str(&server_config_str) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some((server_type, server_config)) = parse_cc_switch_mcp_config(&config) else {
            continue;
        };
        let description = description
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        out.push(CcSwitchMcpCandidate {
            id,
            name,
            server_type,
            server_config,
            description,
            tags: parse_tags_json(&tags_raw),
        });
    }
    Ok(out)
}

/// List MCP servers from CCS DB (default `~/.cc-switch/cc-switch.db`).
/// Missing DB or empty table → empty Vec (not an error). Never logs secrets.
pub fn list_cc_switch_mcp_servers(
    db_path: Option<&Path>,
) -> Result<Vec<CcSwitchMcpCandidate>, String> {
    let path = match db_path {
        Some(p) => p.to_path_buf(),
        None => match default_cc_switch_db_path() {
            Some(p) => p,
            None => return Ok(vec![]),
        },
    };
    if !path.is_file() {
        return Ok(vec![]);
    }
    list_mcp_from_db(&path)
}

fn list_from_db(path: &Path, app_type: &str) -> Result<Vec<CcSwitchProviderCandidate>, String> {
    let conn = open_readonly_db(path)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, settings_config, category, website_url, notes, icon, icon_color
             FROM providers
             WHERE app_type = ?1
             ORDER BY COALESCE(sort_index, 999999), created_at ASC, id ASC",
        )
        .map_err(|e| format!("{MSG_DB_OPEN_FAILED}: {e}"))?;

    let rows = stmt
        .query_map([app_type], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(|e| format!("{MSG_DB_OPEN_FAILED}: {e}"))?;

    let mut providers = Vec::new();
    for row in rows {
        let (id, name, settings_str, category, website_url, notes, icon, icon_color) =
            row.map_err(|e| format!("{MSG_DB_OPEN_FAILED}: {e}"))?;
        let settings: Value = serde_json::from_str(&settings_str).unwrap_or(Value::Null);

        let candidate = match app_type {
            "claude" => extract_claude_candidate(
                &id,
                &name,
                category.as_deref(),
                &settings,
                website_url,
                notes,
                icon,
                icon_color,
            ),
            "codex" => extract_codex_candidate(
                &id,
                &name,
                category.as_deref(),
                &settings,
                website_url,
                notes,
                icon,
                icon_color,
            ),
            "gemini" => extract_gemini_candidate(
                &id,
                &name,
                category.as_deref(),
                &settings,
                website_url,
                notes,
                icon,
                icon_color,
            ),
            // Phase 3: openclaw / opencode
            _ => None,
        };

        if let Some(item) = candidate {
            providers.push(item);
        }
    }

    Ok(providers)
}

#[tauri::command]
pub fn has_cc_switch_db() -> bool {
    if let Ok(guard) = CC_SWITCH_DB_CACHE.lock() {
        if let Some((cached, at)) = *guard {
            if at.elapsed().as_secs() < CC_SWITCH_DB_CACHE_TTL_SECS {
                return cached;
            }
        }
    }

    let found = default_cc_switch_db_path()
        .map(|path| path.is_file())
        .unwrap_or(false);

    if !found {
        info!("CC Switch db check: default path not found");
    }

    if let Ok(mut guard) = CC_SWITCH_DB_CACHE.lock() {
        *guard = Some((found, Instant::now()));
    }

    found
}

#[tauri::command]
pub fn list_cc_switch_providers(
    app_type: String,
    db_path: Option<String>,
) -> Result<CcSwitchDiscovery, String> {
    let app_type = app_type.trim().to_string();
    if app_type.is_empty() {
        return Ok(CcSwitchDiscovery {
            found: false,
            db_path: None,
            providers: vec![],
            message: Some(MSG_NO_PROVIDERS.to_string()),
        });
    }

    let path = match db_path.filter(|s| !s.trim().is_empty()) {
        Some(custom) => PathBuf::from(custom),
        None => match default_cc_switch_db_path() {
            Some(p) => p,
            None => {
                return Ok(CcSwitchDiscovery {
                    found: false,
                    db_path: None,
                    providers: vec![],
                    message: Some(MSG_DB_NOT_FOUND.to_string()),
                });
            }
        },
    };

    if !path.is_file() {
        return Ok(CcSwitchDiscovery {
            found: false,
            db_path: Some(path.display().to_string()),
            providers: vec![],
            message: Some(MSG_DB_NOT_FOUND.to_string()),
        });
    }

    match list_from_db(&path, &app_type) {
        Ok(providers) => {
            let message = if providers.is_empty() {
                Some(MSG_NO_PROVIDERS.to_string())
            } else {
                None
            };
            Ok(CcSwitchDiscovery {
                found: true,
                db_path: Some(path.display().to_string()),
                providers,
                message,
            })
        }
        Err(err) => {
            info!("CC Switch list failed: {err}");
            Ok(CcSwitchDiscovery {
                found: true,
                db_path: Some(path.display().to_string()),
                providers: vec![],
                message: Some(if err.starts_with(MSG_DB_OPEN_FAILED) {
                    MSG_DB_OPEN_FAILED.to_string()
                } else {
                    err
                }),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_category_mapping() {
        assert_eq!(normalize_category(Some("official")), "official");
        assert_eq!(normalize_category(Some("aggregator")), "third_party");
        assert_eq!(normalize_category(Some("")), "custom");
        assert_eq!(normalize_category(None), "custom");
        assert_eq!(normalize_category(Some("omo")), "omo");
    }

    #[test]
    fn local_endpoint_detection() {
        assert!(is_local_endpoint_url(Some("http://127.0.0.1:8317/v1")));
        assert!(is_local_endpoint_url(Some("http://localhost:8080")));
        assert!(!is_local_endpoint_url(Some(
            "http://192.168.31.3:3018/claude"
        )));
        assert!(!is_local_endpoint_url(Some(
            "https://vps.example.com/claude"
        )));
    }

    #[test]
    fn extract_claude_copies_env_and_models() {
        let settings = json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "sk-test",
                "ANTHROPIC_BASE_URL": "https://example.com/claude",
                "ANTHROPIC_MODEL": "glm-4.6",
                "ANTHROPIC_DEFAULT_SONNET_MODEL": "glm-4.6",
                "API_TIMEOUT_MS": "3000000",
                "DISABLE_TELEMETRY": "1"
            },
            "model": "haiku"
        });
        let candidate = extract_claude_candidate(
            "p1",
            "Test",
            Some("aggregator"),
            &settings,
            None,
            None,
            None,
            None,
        )
        .expect("candidate");

        assert_eq!(candidate.normalized_category, "third_party");
        assert_eq!(
            candidate.source_provider_id.as_deref(),
            Some("ccs:claude:p1")
        );
        assert!(candidate.has_api_key);
        assert!(!candidate.is_local_endpoint);
        assert_eq!(candidate.model_preview.as_deref(), Some("glm-4.6"));

        let settings_str = candidate
            .settings_config
            .as_str()
            .expect("settings as string");
        let parsed: Value = serde_json::from_str(settings_str).unwrap();
        let env = parsed.get("env").and_then(|v| v.as_object()).unwrap();
        assert_eq!(
            env.get("API_TIMEOUT_MS").and_then(|v| v.as_str()),
            Some("3000000")
        );
        assert_eq!(
            env.get("DISABLE_TELEMETRY").and_then(|v| v.as_str()),
            Some("1")
        );
        assert!(parsed.get("model").is_none());
    }

    #[test]
    fn rebuild_codex_strips_mcp_and_keeps_channel() {
        let raw = r#"
model_provider = "cli-proxy"
model = "qwen3-coder-plus"
model_reasoning_effort = "high"
disable_response_storage = true

[model_providers.cli-proxy]
name = "cli-proxy"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://192.168.31.3:8317/v1"

[mcp_servers.mcp-router]
type = "stdio"
command = "npx"
"#;
        let (config, base_url, model) = rebuild_codex_config_toml(raw).expect("rebuild");
        assert!(config.contains("model_provider"));
        assert!(config.contains("qwen3-coder-plus"));
        assert!(config.contains("disable_response_storage"));
        assert!(config.contains("model_providers"));
        assert!(!config.contains("mcp_servers"));
        assert_eq!(base_url.as_deref(), Some("http://192.168.31.3:8317/v1"));
        assert_eq!(model.as_deref(), Some("qwen3-coder-plus"));
    }

    #[test]
    fn rebuild_codex_skips_mcp_only_shell() {
        let raw = r#"
[mcp_servers.mcp-router]
type = "stdio"
command = "npx"
"#;
        assert!(rebuild_codex_config_toml(raw).is_none());
    }

    #[test]
    fn extract_codex_skips_non_official_empty_shell_but_keeps_official_empty() {
        let settings = json!({
            "auth": {},
            "config": ""
        });

        assert!(extract_codex_candidate(
            "c1",
            "Empty Custom",
            Some("custom"),
            &settings,
            None,
            None,
            None,
            None,
        )
        .is_none());

        let candidate = extract_codex_candidate(
            "c2",
            "Empty Official",
            Some("official"),
            &settings,
            None,
            None,
            None,
            None,
        )
        .expect("official empty candidate");

        assert_eq!(candidate.normalized_category, "official");
        assert!(!candidate.has_api_key);
        let settings_str = candidate.settings_config.as_str().unwrap();
        let parsed: Value = serde_json::from_str(settings_str).unwrap();
        assert_eq!(parsed.get("config").and_then(|v| v.as_str()), Some(""));
        assert!(parsed
            .get("auth")
            .and_then(|v| v.as_object())
            .is_some_and(|auth| auth.is_empty()));
    }

    #[test]
    fn extract_gemini_copies_env_drops_mcp() {
        let settings = json!({
            "env": {
                "GEMINI_MODEL": "gemini-3.1-pro-preview",
                "GEMINI_API_KEY": "sk-test"
            },
            "config": {
                "general": { "previewFeatures": true },
                "mcpServers": { "x": {} }
            }
        });
        let candidate = extract_gemini_candidate(
            "g1",
            "Gemini",
            Some("custom"),
            &settings,
            None,
            None,
            None,
            None,
        )
        .expect("candidate");
        assert_eq!(
            candidate.source_provider_id.as_deref(),
            Some("ccs:gemini:g1")
        );
        assert!(candidate.has_api_key);
        let settings_str = candidate.settings_config.as_str().unwrap();
        let parsed: Value = serde_json::from_str(settings_str).unwrap();
        assert!(parsed.pointer("/config/mcpServers").is_none());
        assert_eq!(
            parsed.pointer("/env/GEMINI_MODEL").and_then(|v| v.as_str()),
            Some("gemini-3.1-pro-preview")
        );
    }

    #[test]
    fn parse_mcp_stdio_without_type() {
        let config = json!({
            "command": "npx",
            "args": ["-y", "@mcp_router/cli@latest", "connect"],
            "env": { "MCPR_TOKEN": "secret" }
        });
        let (ty, cfg) = parse_cc_switch_mcp_config(&config).expect("stdio");
        assert_eq!(ty, "stdio");
        assert_eq!(cfg.get("command").and_then(|v| v.as_str()), Some("npx"));
        assert!(cfg.get("args").is_some());
        assert!(cfg.get("env").is_some());
    }

    #[test]
    fn parse_mcp_http_with_type() {
        let config = json!({
            "type": "http",
            "url": "https://example.com/mcp",
            "headers": { "Authorization": "Bearer x" }
        });
        let (ty, cfg) = parse_cc_switch_mcp_config(&config).expect("http");
        assert_eq!(ty, "http");
        assert_eq!(
            cfg.get("url").and_then(|v| v.as_str()),
            Some("https://example.com/mcp")
        );
        assert!(cfg.get("headers").is_some());
        assert!(cfg.get("type").is_none());
    }

    #[test]
    fn parse_mcp_sse_type() {
        let config = json!({
            "type": "sse",
            "url": "https://example.com/sse"
        });
        let (ty, cfg) = parse_cc_switch_mcp_config(&config).expect("sse");
        assert_eq!(ty, "sse");
        assert_eq!(
            cfg.get("url").and_then(|v| v.as_str()),
            Some("https://example.com/sse")
        );
    }

    #[test]
    fn parse_mcp_preserves_extra_fields_and_url_aliases() {
        let stdio = json!({
            "type": "local",
            "command": "uvx",
            "args": ["mcp-server-fetch"],
            "cwd": "/tmp/work",
            "startup_timeout_sec": 30,
            "env": { "FOO": "1" }
        });
        let (ty, cfg) = parse_cc_switch_mcp_config(&stdio).expect("local stdio");
        assert_eq!(ty, "stdio");
        assert!(cfg.get("type").is_none());
        assert_eq!(cfg.get("cwd").and_then(|v| v.as_str()), Some("/tmp/work"));
        assert_eq!(
            cfg.get("startup_timeout_sec").and_then(|v| v.as_i64()),
            Some(30)
        );

        let http = json!({
            "type": "http",
            "serverUrl": "https://example.com/mcp",
            "headers": { "Authorization": "Bearer x" },
            "timeout": 5000
        });
        let (ty, cfg) = parse_cc_switch_mcp_config(&http).expect("serverUrl alias");
        assert_eq!(ty, "http");
        assert_eq!(
            cfg.get("url").and_then(|v| v.as_str()),
            Some("https://example.com/mcp")
        );
        assert!(cfg.get("serverUrl").is_none());
        assert_eq!(cfg.get("timeout").and_then(|v| v.as_i64()), Some(5000));
    }

    #[test]
    fn parse_mcp_skips_empty_shell() {
        assert!(parse_cc_switch_mcp_config(&json!({})).is_none());
        assert!(parse_cc_switch_mcp_config(&json!({ "env": {} })).is_none());
        assert!(parse_cc_switch_mcp_config(&json!({ "type": "local" })).is_none());
    }

    #[test]
    fn parse_tags_json_basic() {
        assert_eq!(
            parse_tags_json(r#"["a","b"]"#),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(parse_tags_json("not-json").is_empty());
        assert!(parse_tags_json("[]").is_empty());
    }
}
