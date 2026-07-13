use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;
use tauri::Emitter;
use tokio::process::Command;

use crate::coding::cli_resolver::{build_local_tokio_command, resolve_local_grok_program};
use crate::coding::runtime_location::{self, RuntimeLocationInfo, RuntimeLocationMode};
use crate::db::SqliteDbState;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokPluginActionInput {
    pub plugin_id: String,
    pub source: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct GrokPluginBulkActionInput {
    pub enabled: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokPluginBulkActionResult {
    pub updated_count: usize,
    pub failures: Vec<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct GrokPluginWorkspaceRootInput {
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokPluginRuntimeStatus {
    mode: String,
    source: String,
    root_dir: String,
    config_path: String,
    plugins_dir: String,
    curated_marketplace_path: Option<String>,
    distro: Option<String>,
    linux_root_dir: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokInstalledPlugin {
    plugin_id: String,
    marketplace_name: String,
    name: String,
    display_name: Option<String>,
    description: Option<String>,
    category: Option<String>,
    installed_path: Option<String>,
    active_version: Option<String>,
    enabled: bool,
    has_skills: bool,
    has_mcp_servers: bool,
    has_apps: bool,
    capabilities: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokMarketplacePlugin {
    plugin_id: String,
    marketplace_name: String,
    marketplace_path: String,
    name: String,
    display_name: Option<String>,
    description: Option<String>,
    category: Option<String>,
    capabilities: Vec<String>,
    source_path: Option<String>,
    install_source: Option<String>,
    installed: bool,
    enabled: bool,
    install_available: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokPluginMarketplace {
    name: String,
    path: String,
    display_name: Option<String>,
    description: Option<String>,
    plugin_count: usize,
    is_curated: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokPluginWorkspaceRoot {
    path: String,
    status: String,
    resolution_source: Option<String>,
    resolved_marketplace_path: Option<String>,
    resolved_repo_root: Option<String>,
    error: Option<String>,
}

fn build_command(location: &RuntimeLocationInfo, args: &[&str]) -> Result<Command, String> {
    match location.mode {
        RuntimeLocationMode::LocalWindows => {
            let program = resolve_local_grok_program();
            let mut command = build_local_tokio_command(&program.path);
            command.args(args).env("GROK_HOME", &location.host_path);
            Ok(command)
        }
        RuntimeLocationMode::WslDirect => {
            let wsl = location.wsl.as_ref().ok_or_else(|| {
                "Missing WSL runtime metadata for Grok plugin command".to_string()
            })?;
            let mut command = Command::new("wsl");
            command
                .args(["-d", &wsl.distro, "--exec", "env"])
                .arg(format!("GROK_HOME={}", wsl.linux_path))
                .arg("grok")
                .args(args);
            Ok(command)
        }
    }
}

async fn run(db: &SqliteDbState, args: &[&str]) -> Result<String, String> {
    let location = runtime_location::get_grok_runtime_location_async(db).await?;
    let output = build_command(&location, args)?
        .output()
        .await
        .map_err(|error| format!("Failed to run Grok plugin command: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() { stdout } else { stderr })
}

fn plugin_capabilities(value: &Value) -> Vec<String> {
    let mut result = Vec::new();
    if value
        .get("skill_count")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        > 0
    {
        result.push("skills".to_string());
    }
    for (key, label) in [
        ("has_hooks", "hooks"),
        ("has_agents", "agents"),
        ("has_mcp", "mcp"),
    ] {
        if value.get(key).and_then(Value::as_bool).unwrap_or(false) {
            result.push(label.to_string());
        }
    }
    result
}

async fn list_values(db: &SqliteDbState, available: bool) -> Result<Vec<Value>, String> {
    let args = if available {
        &["plugin", "list", "--json", "--available"][..]
    } else {
        &["plugin", "list", "--json"][..]
    };
    serde_json::from_str(&run(db, args).await?)
        .map_err(|error| format!("Invalid Grok plugin JSON: {error}"))
}

#[tauri::command]
pub async fn get_grok_plugin_runtime_status(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<GrokPluginRuntimeStatus, String> {
    let location = runtime_location::get_grok_runtime_location_async(state.db()).await?;
    let root = location.host_path.to_string_lossy().to_string();
    Ok(GrokPluginRuntimeStatus {
        mode: if location.mode == RuntimeLocationMode::WslDirect {
            "wslDirect"
        } else {
            "local"
        }
        .to_string(),
        source: location.source,
        config_path: location
            .host_path
            .join("config.toml")
            .to_string_lossy()
            .to_string(),
        plugins_dir: location
            .host_path
            .join("plugins")
            .to_string_lossy()
            .to_string(),
        root_dir: root,
        curated_marketplace_path: None,
        distro: location.wsl.as_ref().map(|w| w.distro.clone()),
        linux_root_dir: location.wsl.map(|w| w.linux_path),
    })
}

#[tauri::command]
pub async fn list_grok_installed_plugins(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<GrokInstalledPlugin>, String> {
    Ok(list_values(state.db(), false)
        .await?
        .into_iter()
        .map(|v| GrokInstalledPlugin {
            plugin_id: v["name"].as_str().unwrap_or_default().to_string(),
            marketplace_name: v["marketplace"].as_str().unwrap_or_default().to_string(),
            name: v["name"].as_str().unwrap_or_default().to_string(),
            display_name: None,
            description: v["description"].as_str().map(str::to_string),
            category: None,
            installed_path: v["path"].as_str().map(str::to_string),
            active_version: v["version"].as_str().map(str::to_string),
            enabled: v["status"].as_str() != Some("disabled"),
            has_skills: v["skill_count"].as_u64().unwrap_or(0) > 0,
            has_mcp_servers: v["has_mcp"].as_bool().unwrap_or(false),
            has_apps: false,
            capabilities: plugin_capabilities(&v),
        })
        .collect())
}

#[tauri::command]
pub async fn list_grok_marketplace_plugins(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<GrokMarketplacePlugin>, String> {
    let location = runtime_location::get_grok_runtime_location_async(state.db()).await?;
    let install_sources = marketplace_plugin_install_sources(&location.host_path);
    let installed = list_values(state.db(), false)
        .await?
        .into_iter()
        .filter_map(|value| {
            value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect::<std::collections::HashSet<_>>();
    Ok(list_values(state.db(), true)
        .await?
        .into_iter()
        .filter(|v| v["status"].as_str() == Some("available"))
        .map(|v| {
            let name = v["name"].as_str().unwrap_or_default().to_string();
            let marketplace = v["marketplace"].as_str().unwrap_or_default().to_string();
            let install_source = install_sources
                .get(&(marketplace.clone(), name.clone()))
                .cloned();
            GrokMarketplacePlugin {
                plugin_id: name.clone(),
                marketplace_name: marketplace.clone(),
                marketplace_path: marketplace,
                name: name.clone(),
                display_name: None,
                description: v["description"].as_str().map(str::to_string),
                category: None,
                capabilities: plugin_capabilities(&v),
                source_path: install_source.clone(),
                installed: installed.contains(&name),
                enabled: false,
                install_available: install_source.is_some(),
                install_source,
            }
        })
        .collect())
}

fn marketplace_plugin_install_sources(
    root: &Path,
) -> std::collections::HashMap<(String, String), String> {
    let mut result = std::collections::HashMap::new();
    let cache_root = root.join("marketplace-cache");
    let Ok(entries) = fs::read_dir(cache_root) else {
        return result;
    };
    for entry in entries.flatten() {
        let repository_root = entry.path();
        let manifest_path = repository_root
            .join(".claude-plugin")
            .join("marketplace.json");
        let Ok(content) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<Value>(&content) else {
            continue;
        };
        let Some(marketplace_name) = manifest.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(plugins) = manifest.get("plugins").and_then(Value::as_array) else {
            continue;
        };
        for plugin in plugins {
            let Some(plugin_name) = plugin.get("name").and_then(Value::as_str) else {
                continue;
            };
            if let Some(source) = plugin
                .get("source")
                .and_then(|source| marketplace_install_source(&repository_root, source))
            {
                result.insert(
                    (marketplace_name.to_string(), plugin_name.to_string()),
                    source,
                );
            }
        }
    }
    result
}

fn marketplace_install_source(repository_root: &Path, source: &Value) -> Option<String> {
    if let Some(relative) = source.as_str() {
        let path = repository_root.join(relative);
        return Some(path.to_string_lossy().to_string());
    }
    let object = source.as_object()?;
    let url = object
        .get("url")
        .or_else(|| object.get("repo"))
        .and_then(Value::as_str)?;
    let reference = object.get("ref").and_then(Value::as_str);
    let subdirectory = object.get("path").and_then(Value::as_str);
    let mut install_source = url.to_string();
    if let Some(reference) = reference.filter(|value| !value.trim().is_empty()) {
        install_source.push('@');
        install_source.push_str(reference);
    }
    if let Some(subdirectory) = subdirectory.filter(|value| !value.trim().is_empty()) {
        install_source.push('#');
        install_source.push_str(subdirectory);
    }
    Some(install_source)
}

#[tauri::command]
pub async fn list_grok_marketplaces(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<GrokPluginMarketplace>, String> {
    let values: Vec<Value> =
        serde_json::from_str(&run(state.db(), &["plugin", "marketplace", "list", "--json"]).await?)
            .map_err(|error| format!("Invalid Grok marketplace JSON: {error}"))?;
    let mut plugin_counts = std::collections::HashMap::<String, usize>::new();
    for plugin in list_values(state.db(), true).await? {
        let Some(marketplace_name) = plugin.get("marketplace").and_then(Value::as_str) else {
            continue;
        };
        *plugin_counts
            .entry(marketplace_name.to_string())
            .or_default() += 1;
    }
    Ok(values
        .into_iter()
        .map(|v| {
            let name = v["name"].as_str().unwrap_or_default().to_string();
            GrokPluginMarketplace {
                path: v
                    .pointer("/source/url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                display_name: Some(name.clone()),
                description: None,
                plugin_count: plugin_counts.get(&name).copied().unwrap_or(0),
                is_curated: name == "claude-plugins-official",
                name,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn list_grok_plugin_workspace_roots(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<GrokPluginWorkspaceRoot>, String> {
    Ok(list_grok_marketplaces(state)
        .await?
        .into_iter()
        .map(|m| GrokPluginWorkspaceRoot {
            path: m.path.clone(),
            status: "ready".to_string(),
            resolution_source: Some("direct".to_string()),
            resolved_marketplace_path: Some(m.path),
            resolved_repo_root: None,
            error: None,
        })
        .collect())
}

async fn action(
    state: &tauri::State<'_, SqliteDbState>,
    app: &tauri::AppHandle,
    verb: &str,
    id: &str,
    tail: &[&str],
) -> Result<(), String> {
    let mut args = vec!["plugin", verb, id];
    args.extend_from_slice(tail);
    run(state.db(), &args).await?;
    let _ = app.emit("config-changed", "window");
    emit_grok_sync(app);
    Ok(())
}

#[cfg(target_os = "windows")]
fn emit_grok_sync(app: &tauri::AppHandle) {
    let _ = app.emit("wsl-sync-request-grok", ());
}

#[cfg(not(target_os = "windows"))]
fn emit_grok_sync(_app: &tauri::AppHandle) {}

#[tauri::command]
pub async fn install_grok_plugin(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokPluginActionInput,
) -> Result<(), String> {
    let source = input.source.as_deref().unwrap_or(&input.plugin_id);
    action(&state, &app, "install", source, &["--trust"]).await
}
#[tauri::command]
pub async fn enable_grok_plugin(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokPluginActionInput,
) -> Result<(), String> {
    action(&state, &app, "enable", &input.plugin_id, &[]).await
}
#[tauri::command]
pub async fn disable_grok_plugin(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokPluginActionInput,
) -> Result<(), String> {
    action(&state, &app, "disable", &input.plugin_id, &[]).await
}
#[tauri::command]
pub async fn uninstall_grok_plugin(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokPluginActionInput,
) -> Result<(), String> {
    action(&state, &app, "uninstall", &input.plugin_id, &["--confirm"]).await
}
#[tauri::command]
pub async fn update_grok_plugin(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokPluginActionInput,
) -> Result<(), String> {
    action(&state, &app, "update", &input.plugin_id, &[]).await
}
#[tauri::command]
pub async fn get_grok_plugin_details(
    state: tauri::State<'_, SqliteDbState>,
    input: GrokPluginActionInput,
) -> Result<String, String> {
    run(state.db(), &["plugin", "details", &input.plugin_id]).await
}
#[tauri::command]
pub async fn validate_grok_plugin(
    state: tauri::State<'_, SqliteDbState>,
    input: GrokPluginActionInput,
) -> Result<String, String> {
    let target = input.source.as_deref().unwrap_or(&input.plugin_id);
    run(state.db(), &["plugin", "validate", target]).await
}
#[tauri::command]
pub async fn update_grok_plugin_marketplace(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokPluginWorkspaceRootInput,
) -> Result<(), String> {
    run(
        state.db(),
        &["plugin", "marketplace", "update", &input.path],
    )
    .await?;
    let _ = app.emit("config-changed", "window");
    emit_grok_sync(&app);
    Ok(())
}
#[tauri::command]
pub async fn set_grok_installed_plugins_enabled(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokPluginBulkActionInput,
) -> Result<GrokPluginBulkActionResult, String> {
    let plugins = list_grok_installed_plugins(state.clone()).await?;
    let verb = if input.enabled { "enable" } else { "disable" };
    let mut updated = 0;
    let mut failures = Vec::new();
    for plugin in plugins {
        if plugin.enabled != input.enabled {
            match action(&state, &app, verb, &plugin.plugin_id, &[]).await {
                Ok(()) => updated += 1,
                Err(error) => failures.push(format!("{}: {error}", plugin.plugin_id)),
            }
        }
    }
    Ok(GrokPluginBulkActionResult {
        updated_count: updated,
        failures,
    })
}

#[tauri::command]
pub async fn add_grok_plugin_workspace_root(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokPluginWorkspaceRootInput,
) -> Result<(), String> {
    run(state.db(), &["plugin", "marketplace", "add", &input.path]).await?;
    let _ = app.emit("config-changed", "window");
    emit_grok_sync(&app);
    Ok(())
}
#[tauri::command]
pub async fn remove_grok_plugin_workspace_root(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: GrokPluginWorkspaceRootInput,
) -> Result<(), String> {
    run(
        state.db(),
        &["plugin", "marketplace", "remove", &input.path],
    )
    .await?;
    let _ = app.emit("config-changed", "window");
    emit_grok_sync(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marketplace_install_source_supports_relative_and_git_subdir_sources() {
        let repository_root = Path::new("/tmp/marketplace");
        assert_eq!(
            marketplace_install_source(repository_root, &Value::String("./plugins/review".into()))
                .as_deref(),
            Some("/tmp/marketplace/./plugins/review")
        );
        assert_eq!(
            marketplace_install_source(
                repository_root,
                &serde_json::json!({
                    "source": "git-subdir",
                    "url": "https://github.com/example/plugins.git",
                    "ref": "v1.2.3",
                    "path": "plugins/review"
                }),
            )
            .as_deref(),
            Some("https://github.com/example/plugins.git@v1.2.3#plugins/review")
        );
    }
}
