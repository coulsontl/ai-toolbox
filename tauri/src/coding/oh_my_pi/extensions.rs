use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use futures_util::future::join_all;
use serde_json::Value;
use tauri::Emitter;
use tokio::process::Command;

use super::constants::{OMP_ENV_KEY, OMP_EXTENSIONS_DIR};
use super::types::{
    OmpExtensionActionInput, OmpExtensionCommandResult, OmpExtensionInstallInput, OmpExtensionKind,
    OmpExtensionListResult, OmpExtensionScope, OmpExtensionSummary, OmpExtensionUpdateInput,
};
use crate::coding::cli_resolver::{
    apply_create_no_window_tokio, build_local_tokio_command, local_cli_missing_hint,
    resolve_local_omp_program,
};
use crate::coding::runtime_location::{self, RuntimeLocationInfo, RuntimeLocationMode};
use crate::coding::url_utils::encode_url_path_segment;
use crate::db::SqliteDbState;
use crate::http_client;

const NPM_REGISTRY_BASE_URL: &str = "https://registry.npmjs.org";
const NPM_LATEST_LOOKUP_TIMEOUT_SECS: u64 = 8;
const NPM_LATEST_LOOKUP_CONCURRENCY: usize = 6;

const WSL_OMP_COMMAND_SCRIPT: &str = r#"path_prefix=$1
omp_root=$2
shift 2
if [ -n "$path_prefix" ]; then
    PATH="$path_prefix${PATH:+:$PATH}"
    export PATH
fi
export PI_CODING_AGENT_DIR="$omp_root"
exec "$@""#;

struct OmpCommandInvocation {
    command: Command,
    local_program_label: Option<String>,
}

pub fn get_omp_extensions_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join(OMP_EXTENSIONS_DIR)
}

pub fn get_omp_packages_path_from_root(root_dir: &Path) -> PathBuf {
    // omp plugin 的 npm 包装在 ~/.omp/plugins/node_modules。
    root_dir
        .parent()
        .map(|config_root| config_root.join("plugins").join("node_modules"))
        .unwrap_or_else(|| root_dir.join("plugins").join("node_modules"))
}

pub async fn get_omp_extensions_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_omp_extensions_path_from_root(
        &runtime_location::get_oh_my_pi_runtime_location_async(db)
            .await?
            .host_path,
    ))
}

fn omp_wsl_path_prefix(linux_user_root: Option<&str>) -> String {
    let Some(linux_user_root) = linux_user_root.filter(|root| !root.trim().is_empty()) else {
        return String::new();
    };
    let linux_user_root = linux_user_root.trim_end_matches('/');
    [
        format!("{linux_user_root}/.local/share/mise/shims"),
        format!("{linux_user_root}/.asdf/shims"),
        format!("{linux_user_root}/.local/bin"),
        format!("{linux_user_root}/.bun/bin"),
        format!("{linux_user_root}/.volta/bin"),
        format!("{linux_user_root}/.local/share/fnm/aliases/default/bin"),
        format!("{linux_user_root}/.fnm/aliases/default/bin"),
        format!("{linux_user_root}/.fnm/current/bin"),
        format!("{linux_user_root}/.npm-global/bin"),
    ]
    .join(":")
}

async fn build_omp_command(
    runtime_location: &RuntimeLocationInfo,
    args: &[&str],
) -> Result<OmpCommandInvocation, String> {
    match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => {
            let omp_program = tauri::async_runtime::spawn_blocking(resolve_local_omp_program)
                .await
                .map_err(|error| format!("Failed to resolve OMP CLI: {error}"))?;
            let local_program_label = omp_program.path.display().to_string();
            let mut command = build_local_tokio_command(&omp_program.path);
            command.args(args);
            command.env(OMP_ENV_KEY, &runtime_location.host_path);
            Ok(OmpCommandInvocation {
                command,
                local_program_label: Some(local_program_label),
            })
        }
        RuntimeLocationMode::WslDirect => {
            let wsl = runtime_location
                .wsl
                .as_ref()
                .ok_or_else(|| "Missing WSL runtime metadata for OMP plugin command".to_string())?;
            let local_program_label = format!("wsl -d {} -- omp", wsl.distro);
            let mut command = Command::new("wsl");
            apply_create_no_window_tokio(&mut command);
            command.args([
                "-d",
                &wsl.distro,
                "--exec",
                "/bin/sh",
                "-c",
                WSL_OMP_COMMAND_SCRIPT,
                "ai-toolbox-omp",
                &omp_wsl_path_prefix(wsl.linux_user_root.as_deref()),
                &wsl.linux_path,
                "env",
            ]);
            command.arg("omp");
            command.args(args);
            Ok(OmpCommandInvocation {
                command,
                local_program_label: Some(local_program_label),
            })
        }
    }
}

/// Human-readable CLI label for the resolved `omp` used by plugin ops.
fn resolve_omp_cli_display_path(runtime_location: &RuntimeLocationInfo) -> Option<String> {
    match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => {
            Some(resolve_local_omp_program().path.display().to_string())
        }
        RuntimeLocationMode::WslDirect => runtime_location
            .wsl
            .as_ref()
            .map(|wsl| format!("wsl -d {} -- omp", wsl.distro)),
    }
}

/// Append resolved CLI path so multi-`omp` PATH installs are diagnosable from UI errors.
fn annotate_omp_command_error(message: String, local_program_label: Option<&str>) -> String {
    let Some(label) = local_program_label
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return message;
    };
    if message.contains(label) {
        return message;
    }
    format!("{message}\nomp_cli={label}")
}

fn build_omp_spawn_error(error: &std::io::Error, local_program_label: Option<&str>) -> String {
    let base_message = format!("Failed to run OMP plugin command: {error}");
    let manual_hint = crate::coding::cli_resolver::manual_cli_config_hint("omp");
    let with_hint = if error.kind() == std::io::ErrorKind::NotFound {
        let message = if let Some(label) = local_program_label {
            format!(
                "{base_message}. attempted_program={label}. {}",
                local_cli_missing_hint("omp")
            )
        } else {
            format!("{base_message}. {}", local_cli_missing_hint("omp"))
        };
        if manual_hint.is_empty() {
            message
        } else {
            format!("{message} {manual_hint}")
        }
    } else if !manual_hint.is_empty() {
        format!("{base_message} {manual_hint}")
    } else {
        base_message
    };
    annotate_omp_command_error(with_hint, local_program_label)
}

fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Copyable terminal guidance; uses the same root and WSL PATH as the runner.
pub(super) fn omp_login_command(location: &RuntimeLocationInfo) -> Option<String> {
    match location.mode {
        RuntimeLocationMode::LocalWindows => {
            let program = resolve_local_omp_program();
            let root = location.host_path.to_string_lossy();
            let program = program.path.to_string_lossy();
            if cfg!(target_os = "windows") {
                Some(format!(
                    "$env:PI_CODING_AGENT_DIR={}; & {} login openai-codex",
                    powershell_quote(&root),
                    powershell_quote(&program)
                ))
            } else {
                Some(format!(
                    "PI_CODING_AGENT_DIR={} {} login openai-codex",
                    posix_quote(&root),
                    posix_quote(&program)
                ))
            }
        }
        RuntimeLocationMode::WslDirect => {
            let wsl = location.wsl.as_ref()?;
            let args = [
                "wsl".to_string(),
                "-d".to_string(),
                wsl.distro.clone(),
                "--exec".to_string(),
                "/bin/sh".to_string(),
                "-c".to_string(),
                WSL_OMP_COMMAND_SCRIPT.to_string(),
                "ai-toolbox-omp".to_string(),
                omp_wsl_path_prefix(wsl.linux_user_root.as_deref()),
                wsl.linux_path.clone(),
                "env".to_string(),
                "omp".to_string(),
                "login".to_string(),
                "openai-codex".to_string(),
            ];
            let quote = if cfg!(target_os = "windows") {
                powershell_quote
            } else {
                posix_quote
            };
            let prefix = if cfg!(target_os = "windows") {
                "& "
            } else {
                ""
            };
            Some(format!(
                "{prefix}{}",
                args.iter()
                    .map(|arg| quote(arg))
                    .collect::<Vec<_>>()
                    .join(" ")
            ))
        }
    }
}

pub(super) async fn run_omp_command(
    runtime_location: &RuntimeLocationInfo,
    args: &[&str],
) -> Result<String, String> {
    let OmpCommandInvocation {
        mut command,
        local_program_label,
    } = build_omp_command(runtime_location, args).await?;

    command.kill_on_drop(true);
    let output = command
        .output()
        .await
        .map_err(|error| build_omp_spawn_error(&error, local_program_label.as_deref()))?;

    let stdout_output = String::from_utf8_lossy(&output.stdout).to_string();
    if output.status.success() {
        return Ok(stdout_output);
    }

    let stderr_output = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout_trimmed = stdout_output.trim().to_string();
    let failure_message = if !stderr_output.is_empty() {
        stderr_output
    } else if !stdout_trimmed.is_empty() {
        stdout_trimmed
    } else {
        "Unknown OMP plugin command failure".to_string()
    };
    Err(annotate_omp_command_error(
        failure_message,
        local_program_label.as_deref(),
    ))
}

async fn probe_omp_cli_version(runtime_location: &RuntimeLocationInfo) -> Option<String> {
    let version = run_omp_command(runtime_location, &["--version"])
        .await
        .ok()?;
    let trimmed = version.lines().next().unwrap_or(version.as_str()).trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_protected_local_extension_source(source: &str) -> bool {
    let source = source.trim();
    source.starts_with("omp-deck-") || source.starts_with("ai-toolbox-")
}

/// 解析 `omp plugin list --json` 输出:`{ "npm": [...], "marketplace": [...] }`。
/// npm 条目形如 `{ name, version, path, manifest, enabledFeatures, enabled }`。
/// marketplace 条目形如 `{ id, scope, entries: [{ scope, installPath, version, ... }], shadowedBy }`。
fn parse_plugin_list_json(raw: &str) -> Vec<OmpExtensionSummary> {
    let Ok(parsed) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let mut result = Vec::new();

    if let Some(npm) = parsed.get("npm").and_then(Value::as_array) {
        for entry in npm {
            let Some(name) = entry.get("name").and_then(Value::as_str) else {
                continue;
            };
            if name.trim().is_empty() {
                continue;
            }
            result.push(OmpExtensionSummary {
                id: format!("npm:{name}"),
                source: name.to_string(),
                scope: OmpExtensionScope::User,
                kind: OmpExtensionKind::Package,
                path: entry
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                built_in: false,
                current_version: entry
                    .get("version")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                latest_version: None,
                update_available: false,
            });
        }
    }

    if let Some(marketplace) = parsed.get("marketplace").and_then(Value::as_array) {
        for entry in marketplace {
            let Some(id) = entry.get("id").and_then(Value::as_str) else {
                continue;
            };
            if id.trim().is_empty() {
                continue;
            }
            let scope = entry
                .get("scope")
                .and_then(Value::as_str)
                .map(|value| {
                    if value == "project" {
                        OmpExtensionScope::Project
                    } else {
                        OmpExtensionScope::User
                    }
                })
                .unwrap_or(OmpExtensionScope::Unknown);
            let first_entry = entry
                .get("entries")
                .and_then(Value::as_array)
                .and_then(|entries| entries.first());
            result.push(OmpExtensionSummary {
                id: format!("marketplace:{id}"),
                source: id.to_string(),
                scope,
                kind: OmpExtensionKind::Package,
                path: first_entry
                    .and_then(|first| first.get("installPath").and_then(Value::as_str))
                    .map(str::to_string),
                built_in: false,
                current_version: first_entry
                    .and_then(|first| first.get("version").and_then(Value::as_str))
                    .map(str::to_string),
                latest_version: None,
                update_available: false,
            });
        }
    }

    result
}

fn scan_local_extensions(extensions_path: &Path) -> Result<Vec<OmpExtensionSummary>, String> {
    let entries = match fs::read_dir(extensions_path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "Failed to read OMP extensions directory {}: {error}",
                extensions_path.display()
            ));
        }
    };

    let mut result = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "Failed to read OMP extensions directory entry in {}: {error}",
                extensions_path.display()
            )
        })?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if file_name.starts_with('.') || file_name == "node_modules" || file_name.ends_with(".d.ts")
        {
            continue;
        }

        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "Failed to inspect OMP extension {}: {error}",
                path.display()
            )
        })?;
        let (source, kind) = if file_type.is_file() && file_name.ends_with(".ts") {
            (file_name.to_string(), OmpExtensionKind::LocalFile)
        } else if file_type.is_dir() && path.join("index.ts").is_file() {
            (file_name.to_string(), OmpExtensionKind::LocalDirectory)
        } else {
            continue;
        };

        result.push(OmpExtensionSummary {
            id: format!("local:{source}"),
            built_in: is_protected_local_extension_source(&source),
            source,
            scope: OmpExtensionScope::User,
            kind,
            path: Some(path.to_string_lossy().to_string()),
            current_version: None,
            latest_version: None,
            update_available: false,
        });
    }

    result.sort_by(|left, right| left.source.cmp(&right.source));
    Ok(result)
}

/// Parse `npm:name` / `npm:@scope/name` and optional trailing `@version` pin.
/// Returns `(package_name, pinned_version)` when the source is an npm package.
fn parse_npm_package_source(source: &str) -> Option<(String, Option<String>)> {
    let trimmed = source.trim();
    let without_prefix = trimmed.strip_prefix("npm:")?;
    if without_prefix.is_empty() {
        return None;
    }

    if let Some(rest) = without_prefix.strip_prefix('@') {
        let (name_part, version_part) = match rest.rsplit_once('@') {
            Some((name, version)) if name.contains('/') => (name, Some(version)),
            _ => (rest, None),
        };
        if name_part.is_empty() || !name_part.contains('/') {
            return None;
        }
        let package_name = format!("@{name_part}");
        let pinned = version_part
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .map(str::to_string);
        return Some((package_name, pinned));
    }

    let (name_part, version_part) = match without_prefix.rsplit_once('@') {
        Some((name, version)) if !name.is_empty() => (name, Some(version)),
        _ => (without_prefix, None),
    };
    if name_part.is_empty() || name_part.contains('/') {
        return None;
    }
    let pinned = version_part
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(str::to_string);
    Some((name_part.to_string(), pinned))
}

fn is_version_newer(latest: &str, current: &str) -> bool {
    let parse = |value: &str| -> Vec<u64> {
        value
            .trim()
            .trim_start_matches('v')
            .split(|ch: char| !ch.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<u64>().ok())
            .collect()
    };
    let latest_parts = parse(latest);
    let current_parts = parse(current);
    if latest_parts.is_empty() || current_parts.is_empty() {
        return latest.trim() != current.trim() && !latest.trim().is_empty();
    }

    let max_len = latest_parts.len().max(current_parts.len());
    for index in 0..max_len {
        let left = latest_parts.get(index).copied().unwrap_or(0);
        let right = current_parts.get(index).copied().unwrap_or(0);
        if left > right {
            return true;
        }
        if left < right {
            return false;
        }
    }
    false
}

async fn fetch_npm_latest_version(client: &reqwest::Client, package_name: &str) -> Option<String> {
    let package_url = format!(
        "{}/{}",
        NPM_REGISTRY_BASE_URL,
        encode_url_path_segment(package_name)
    );
    let response = client
        .get(&package_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, "AI-Toolbox")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let metadata = response.json::<Value>().await.ok()?;
    metadata
        .get("dist-tags")
        .and_then(|dist_tags| dist_tags.get("latest"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

async fn enrich_npm_update_availability(
    db: &SqliteDbState,
    extensions: Vec<OmpExtensionSummary>,
) -> Vec<OmpExtensionSummary> {
    let client = match http_client::client_with_timeout(db, NPM_LATEST_LOOKUP_TIMEOUT_SECS).await {
        Ok(client) => client,
        Err(_) => return extensions,
    };

    let mut package_names = Vec::new();
    let mut seen_names = HashSet::new();
    for extension in &extensions {
        if extension.kind != OmpExtensionKind::Package {
            continue;
        }
        // Use the prefixed `id` (e.g. `npm:name` / `marketplace:...`) for npm
        // lookup; `source` must stay raw for `omp plugin uninstall`/`upgrade`.
        // `parse_npm_package_source` requires the `npm:` prefix, so only npm
        // entries (whose id is `npm:...`) resolve — marketplace entries are
        // skipped here.
        let Some((package_name, pinned_version)) = parse_npm_package_source(&extension.id) else {
            continue;
        };
        if pinned_version.is_some() {
            continue;
        }
        if extension.current_version.is_none() {
            continue;
        }
        let key = package_name.to_ascii_lowercase();
        if seen_names.insert(key) {
            package_names.push(package_name);
        }
    }

    if package_names.is_empty() {
        return extensions;
    }

    let mut latest_by_package = HashMap::new();
    for chunk in package_names.chunks(NPM_LATEST_LOOKUP_CONCURRENCY) {
        let lookups = chunk.iter().map(|package_name| {
            let client = client.clone();
            let package_name = package_name.clone();
            async move {
                let latest = fetch_npm_latest_version(&client, &package_name).await;
                (package_name, latest)
            }
        });
        for (package_name, latest) in join_all(lookups).await {
            if let Some(version) = latest {
                latest_by_package.insert(package_name.to_ascii_lowercase(), version);
            }
        }
    }

    extensions
        .into_iter()
        .map(|mut extension| {
            if extension.kind != OmpExtensionKind::Package {
                return extension;
            }
            let Some((package_name, pinned_version)) = parse_npm_package_source(&extension.id)
            else {
                return extension;
            };
            if pinned_version.is_some() {
                return extension;
            }
            let Some(latest_version) = latest_by_package
                .get(&package_name.to_ascii_lowercase())
                .cloned()
            else {
                return extension;
            };
            let update_available = extension
                .current_version
                .as_deref()
                .map(|current| is_version_newer(&latest_version, current))
                .unwrap_or(false);
            extension.latest_version = Some(latest_version);
            extension.update_available = update_available;
            extension
        })
        .collect()
}

fn merge_extensions(
    omp_extensions: Vec<OmpExtensionSummary>,
    local_extensions: Vec<OmpExtensionSummary>,
) -> Vec<OmpExtensionSummary> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();

    for extension in omp_extensions {
        seen.insert(extension_identity(&extension));
        merged.push(extension);
    }
    for extension in local_extensions {
        let identity = extension_identity(&extension);
        if seen.insert(identity) {
            merged.push(extension);
        }
    }

    merged
}

fn extension_identity(extension: &OmpExtensionSummary) -> String {
    extension
        .path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .unwrap_or(&extension.source)
        .to_string()
}

fn delete_local_extension(
    extensions_path: &Path,
    input: &OmpExtensionActionInput,
) -> Result<(), String> {
    let source = input.source.trim();
    if source.is_empty() {
        return Err("OMP extension source cannot be empty".to_string());
    }
    if is_protected_local_extension_source(source) {
        return Err("Built-in OMP extension cannot be deleted".to_string());
    }

    let target_path = input
        .path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| extensions_path.join(source));
    let canonical_extensions_path = fs::canonicalize(extensions_path).map_err(|error| {
        format!(
            "Failed to resolve OMP extensions directory {}: {error}",
            extensions_path.display()
        )
    })?;
    let canonical_target_path = fs::canonicalize(&target_path).map_err(|error| {
        format!(
            "Failed to resolve OMP extension path {}: {error}",
            target_path.display()
        )
    })?;
    if !canonical_target_path.starts_with(&canonical_extensions_path) {
        return Err(format!(
            "OMP extension path is outside extensions directory: {}",
            canonical_target_path.display()
        ));
    }

    if canonical_target_path.is_dir() {
        fs::remove_dir_all(&canonical_target_path).map_err(|error| {
            format!(
                "Failed to delete OMP extension directory {}: {error}",
                canonical_target_path.display()
            )
        })
    } else {
        fs::remove_file(&canonical_target_path).map_err(|error| {
            format!(
                "Failed to delete OMP extension file {}: {error}",
                canonical_target_path.display()
            )
        })
    }
}

fn emit_extensions_changed(app: &tauri::AppHandle, payload: &str) {
    let _ = app.emit("config-changed", payload);
}

#[tauri::command]
pub async fn list_omp_extensions(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<OmpExtensionListResult, String> {
    let db = state.db();
    let runtime_location = runtime_location::get_oh_my_pi_runtime_location_async(&db).await?;
    let extensions_path = get_omp_extensions_path_from_root(&runtime_location.host_path);
    let packages_path = get_omp_packages_path_from_root(&runtime_location.host_path);
    let raw = run_omp_command(&runtime_location, &["plugin", "list", "--json"]).await?;
    let omp_extensions = parse_plugin_list_json(&raw);
    let local_extensions = scan_local_extensions(&extensions_path)?;
    let merged = merge_extensions(omp_extensions, local_extensions);
    let extensions = enrich_npm_update_availability(&db, merged).await;
    let cli_path = resolve_omp_cli_display_path(&runtime_location);
    let cli_version = probe_omp_cli_version(&runtime_location).await;

    Ok(OmpExtensionListResult {
        extensions_path: extensions_path.to_string_lossy().to_string(),
        packages_path: packages_path.to_string_lossy().to_string(),
        extensions,
        raw,
        cli_path,
        cli_version,
    })
}

#[tauri::command]
pub async fn install_omp_extension(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OmpExtensionInstallInput,
) -> Result<OmpExtensionCommandResult, String> {
    let source = input.source.trim();
    if source.is_empty() {
        return Err("OMP extension source cannot be empty".to_string());
    }

    let db = state.db();
    let runtime_location = runtime_location::get_oh_my_pi_runtime_location_async(&db).await?;
    let args = ["plugin", "install", source];
    let output = run_omp_command(&runtime_location, &args).await?;
    emit_extensions_changed(&app, "omp-extensions");

    Ok(OmpExtensionCommandResult {
        command: format!("omp {}", args.join(" ")),
        output: output.trim().to_string(),
    })
}

#[tauri::command]
pub async fn uninstall_omp_extension(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: OmpExtensionActionInput,
) -> Result<OmpExtensionCommandResult, String> {
    let source = input.source.trim();
    if source.is_empty() {
        return Err("OMP extension source cannot be empty".to_string());
    }

    let db = state.db();
    let runtime_location = runtime_location::get_oh_my_pi_runtime_location_async(&db).await?;
    let extensions_path = get_omp_extensions_path_from_root(&runtime_location.host_path);
    let kind = input.kind.unwrap_or(OmpExtensionKind::Package);

    if kind != OmpExtensionKind::Package {
        delete_local_extension(&extensions_path, &input)?;
        emit_extensions_changed(&app, "omp-extensions");
        return Ok(OmpExtensionCommandResult {
            command: format!("delete {}", source),
            output: String::new(),
        });
    }

    let args = ["plugin", "uninstall", source];
    let output = run_omp_command(&runtime_location, &args).await?;
    emit_extensions_changed(&app, "omp-extensions");

    Ok(OmpExtensionCommandResult {
        command: format!("omp {}", args.join(" ")),
        output: output.trim().to_string(),
    })
}

#[tauri::command]
pub async fn update_omp_extensions(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: Option<OmpExtensionUpdateInput>,
) -> Result<OmpExtensionCommandResult, String> {
    let db = state.db();
    let runtime_location = runtime_location::get_oh_my_pi_runtime_location_async(&db).await?;
    let single_source = input
        .as_ref()
        .and_then(|value| value.source.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    // 单插件:`omp plugin upgrade <name>`;全部:`omp plugin upgrade`。
    let args: Vec<String> = if let Some(source) = single_source.as_deref() {
        vec![
            "plugin".to_string(),
            "upgrade".to_string(),
            source.to_string(),
        ]
    } else {
        vec!["plugin".to_string(), "upgrade".to_string()]
    };
    let output = run_omp_command(
        &runtime_location,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .await?;
    emit_extensions_changed(&app, "omp-extensions");

    Ok(OmpExtensionCommandResult {
        command: format!("omp {}", args.join(" ")),
        output: output.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plugin_list_json_with_npm_and_marketplace() {
        let raw = r#"{
  "npm": [
    {
      "name": "context-mode",
      "version": "1.2.3",
      "path": "/home/tester/.omp/plugins/node_modules/context-mode",
      "manifest": {},
      "enabledFeatures": null,
      "enabled": true
    }
  ],
  "marketplace": [
    {
      "id": "exa",
      "scope": "user",
      "entries": [
        {
          "scope": "user",
          "installPath": "/home/tester/.omp/plugins/exa",
          "version": "0.9.1",
          "installedAt": "2026-01-01T00:00:00.000Z",
          "lastUpdated": "2026-01-01T00:00:00.000Z"
        }
      ]
    }
  ]
}"#;

        let extensions = parse_plugin_list_json(raw);

        assert_eq!(extensions.len(), 2);
        assert_eq!(extensions[0].source, "context-mode");
        assert_eq!(extensions[0].current_version.as_deref(), Some("1.2.3"));
        assert_eq!(
            extensions[0].path.as_deref(),
            Some("/home/tester/.omp/plugins/node_modules/context-mode")
        );
        assert_eq!(extensions[1].source, "exa");
        assert_eq!(extensions[1].current_version.as_deref(), Some("0.9.1"));
        assert_eq!(extensions[1].scope, OmpExtensionScope::User);
    }

    #[test]
    fn parses_npm_package_source_with_optional_pin() {
        assert_eq!(
            parse_npm_package_source("npm:context-mode"),
            Some(("context-mode".to_string(), None))
        );
        assert_eq!(
            parse_npm_package_source("npm:context-mode@1.2.3"),
            Some(("context-mode".to_string(), Some("1.2.3".to_string())))
        );
        assert_eq!(
            parse_npm_package_source("npm:@scope/name@0.1.0"),
            Some(("@scope/name".to_string(), Some("0.1.0".to_string())))
        );
        assert_eq!(parse_npm_package_source("github:owner/repo"), None);
        assert_eq!(parse_npm_package_source("file:./local"), None);
    }

    #[test]
    fn npm_plugin_list_entries_resolve_npm_source_from_id() {
        // parse_plugin_list_json stores the raw package name in `source` (kept
        // for `omp plugin uninstall`) and the `npm:`-prefixed id. The update
        // check parses from `id`, so this asserts the id round-trips through
        // parse_npm_package_source while the bare source is preserved.
        let raw = r#"{
            "npm": [
                { "name": "context-mode", "version": "0.9.1",
                  "path": "/home/t/.omp/plugins/node_modules/context-mode",
                  "manifest": {}, "enabledFeatures": [], "enabled": true },
                { "name": "exa", "version": "1.2.3",
                  "path": "/home/t/.omp/plugins/node_modules/exa",
                  "manifest": {}, "enabledFeatures": [], "enabled": true }
            ],
            "marketplace": []
        }"#;
        let extensions = parse_plugin_list_json(raw);
        assert_eq!(extensions.len(), 2);

        // source stays bare (used for uninstall/upgrade)
        assert_eq!(extensions[0].source, "context-mode");
        assert_eq!(extensions[1].source, "exa");
        // id carries the npm: prefix and resolves to the package name
        assert_eq!(extensions[0].id, "npm:context-mode");
        assert_eq!(extensions[1].id, "npm:exa");
        for extension in &extensions {
            let (package_name, _pinned) =
                parse_npm_package_source(&extension.id).expect("npm id must parse");
            assert_eq!(package_name, extension.source);
        }
    }

    #[test]
    fn compares_semverish_versions_for_update_detection() {
        assert!(is_version_newer("1.2.4", "1.2.3"));
        assert!(is_version_newer("2.0.0", "1.9.9"));
        assert!(!is_version_newer("1.2.3", "1.2.3"));
        assert!(!is_version_newer("1.2.3", "1.2.4"));
        assert!(is_version_newer("v1.3.0", "1.2.9"));
    }

    #[test]
    fn scans_local_file_and_directory_extensions() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let extensions_path = temp_dir.path();
        fs::write(
            extensions_path.join("single.ts"),
            "export default () => {};",
        )
        .expect("write file extension");
        fs::write(extensions_path.join("types.d.ts"), "").expect("write dts");
        fs::create_dir(extensions_path.join("directory")).expect("mkdir directory");
        fs::write(
            extensions_path.join("directory").join("index.ts"),
            "export default () => {};",
        )
        .expect("write directory extension");

        let extensions = scan_local_extensions(extensions_path).expect("scan");
        let sources: Vec<_> = extensions
            .iter()
            .map(|extension| (extension.source.as_str(), extension.kind))
            .collect();

        assert_eq!(
            sources,
            vec![
                ("directory", OmpExtensionKind::LocalDirectory),
                ("single.ts", OmpExtensionKind::LocalFile),
            ]
        );
    }

    #[test]
    fn packages_path_resolves_under_config_root_plugins() {
        let root = PathBuf::from("~").join(".omp").join("agent");
        assert_eq!(
            get_omp_packages_path_from_root(&root),
            PathBuf::from("~")
                .join(".omp")
                .join("plugins")
                .join("node_modules")
        );
    }
    #[test]
    fn login_guidance_quotes_shell_values_and_selected_wsl_root() {
        assert_eq!(posix_quote("/tmp/it's $root"), "'/tmp/it'\"'\"'s $root'");
        assert_eq!(powershell_quote("C:\\it's $root"), "'C:\\it''s $root'");
        let location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::WslDirect,
            source: "custom".to_string(),
            host_path: PathBuf::from(r"\\wsl.localhost\Ubuntu\home\tester\custom root"),
            wsl: Some(crate::coding::runtime_location::WslLocationInfo {
                distro: "Ubuntu user's".to_string(),
                linux_path: "/home/tester/custom root".to_string(),
                linux_user_root: Some("/home/tester".to_string()),
            }),
        };
        let command = omp_login_command(&location).unwrap();
        assert!(command.contains("'/home/tester/custom root'"));
        assert!(command.contains("export PI_CODING_AGENT_DIR="));
        assert!(command.contains("/home/tester/.bun/bin"));
        assert!(command.ends_with("'env' 'omp' 'login' 'openai-codex'"));
        assert!(!command.contains("wsl.localhost"));
        let quote = if cfg!(target_os = "windows") {
            powershell_quote
        } else {
            posix_quote
        };
        assert!(command.contains(&quote("Ubuntu user's")));
    }
}
