use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tauri::Emitter;
use tokio::process::Command;

use super::constants::{PI_ENV_KEY, PI_EXTENSIONS_DIR};
use super::types::{
    PiExtensionActionInput, PiExtensionCommandResult, PiExtensionInstallInput, PiExtensionKind,
    PiExtensionListResult, PiExtensionScope, PiExtensionSummary,
};
use crate::coding::cli_resolver::{
    apply_create_no_window_tokio, build_local_tokio_command, local_cli_missing_hint,
    resolve_local_pi_program,
};
use crate::coding::runtime_location::{self, RuntimeLocationInfo, RuntimeLocationMode};
use crate::db::SqliteDbState;

const NPM_LEGACY_PEER_DEPS_ENV_KEY: &str = "NPM_CONFIG_LEGACY_PEER_DEPS";
const NPM_LEGACY_PEER_DEPS_ENV_VALUE: &str = "true";
const WSL_PI_COMMAND_SCRIPT: &str = r#"path_prefix=$1
pi_root=$2
shift 2
if [ -n "$path_prefix" ]; then
    PATH="$path_prefix${PATH:+:$PATH}"
    export PATH
fi
export PI_CODING_AGENT_DIR="$pi_root"
exec "$@""#;

struct PiCommandInvocation {
    command: Command,
    local_program_label: Option<String>,
}

pub fn get_pi_extensions_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join(PI_EXTENSIONS_DIR)
}

pub fn get_pi_packages_path_from_root(root_dir: &Path) -> PathBuf {
    root_dir.join("npm").join("node_modules")
}

pub async fn get_pi_extensions_path_async(db: &SqliteDbState) -> Result<PathBuf, String> {
    Ok(get_pi_extensions_path_from_root(
        &runtime_location::get_pi_runtime_location_async(db)
            .await?
            .host_path,
    ))
}

fn pi_extension_npm_compat_env() -> [(&'static str, &'static str); 1] {
    [(NPM_LEGACY_PEER_DEPS_ENV_KEY, NPM_LEGACY_PEER_DEPS_ENV_VALUE)]
}

fn apply_pi_extension_npm_compat_env(command: &mut Command) {
    for (key, value) in pi_extension_npm_compat_env() {
        command.env(key, value);
    }
}

fn pi_wsl_path_prefix(linux_user_root: Option<&str>) -> String {
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

fn build_pi_command(
    runtime_location: &RuntimeLocationInfo,
    args: &[&str],
    offline: bool,
) -> Result<PiCommandInvocation, String> {
    match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => {
            let pi_program = resolve_local_pi_program();
            let local_program_label = pi_program.path.display().to_string();
            let mut command = build_local_tokio_command(&pi_program.path);
            command.args(args);
            command.env(PI_ENV_KEY, &runtime_location.host_path);
            apply_pi_extension_npm_compat_env(&mut command);
            if offline {
                command.env("PI_OFFLINE", "1");
            }
            Ok(PiCommandInvocation {
                command,
                local_program_label: Some(local_program_label),
            })
        }
        RuntimeLocationMode::WslDirect => {
            let wsl = runtime_location.wsl.as_ref().ok_or_else(|| {
                "Missing WSL runtime metadata for Pi extension command".to_string()
            })?;
            let local_program_label = format!("wsl -d {} -- pi", wsl.distro);
            let mut command = Command::new("wsl");
            apply_create_no_window_tokio(&mut command);
            command.args([
                "-d",
                &wsl.distro,
                "--exec",
                "/bin/sh",
                "-c",
                WSL_PI_COMMAND_SCRIPT,
                "ai-toolbox-pi",
                &pi_wsl_path_prefix(wsl.linux_user_root.as_deref()),
                &wsl.linux_path,
                "env",
            ]);
            for (key, value) in pi_extension_npm_compat_env() {
                command.arg(format!("{key}={value}"));
            }
            if offline {
                command.arg("PI_OFFLINE=1");
            }
            command.arg("pi");
            command.args(args);
            Ok(PiCommandInvocation {
                command,
                local_program_label: Some(local_program_label),
            })
        }
    }
}

/// Human-readable CLI label for the resolved `pi` used by extension ops.
fn resolve_pi_cli_display_path(runtime_location: &RuntimeLocationInfo) -> Option<String> {
    match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => {
            Some(resolve_local_pi_program().path.display().to_string())
        }
        RuntimeLocationMode::WslDirect => runtime_location
            .wsl
            .as_ref()
            .map(|wsl| format!("wsl -d {} -- pi", wsl.distro)),
    }
}

/// Append resolved CLI path so multi-`pi` PATH installs are diagnosable from UI errors.
fn annotate_pi_command_error(message: String, local_program_label: Option<&str>) -> String {
    let Some(label) = local_program_label
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return message;
    };
    if message.contains(label) {
        return message;
    }
    format!("{message}\npi_cli={label}")
}

fn build_pi_spawn_error(error: &std::io::Error, local_program_label: Option<&str>) -> String {
    let base_message = format!("Failed to run Pi extension command: {error}");
    let with_hint = if error.kind() == std::io::ErrorKind::NotFound {
        if let Some(label) = local_program_label {
            format!(
                "{base_message}. attempted_program={label}. {}",
                local_cli_missing_hint("pi")
            )
        } else {
            format!("{base_message}. {}", local_cli_missing_hint("pi"))
        }
    } else {
        base_message
    };
    annotate_pi_command_error(with_hint, local_program_label)
}

async fn run_pi_command(
    runtime_location: &RuntimeLocationInfo,
    args: &[&str],
    offline: bool,
) -> Result<String, String> {
    let PiCommandInvocation {
        mut command,
        local_program_label,
    } = build_pi_command(runtime_location, args, offline)?;

    let output = command
        .output()
        .await
        .map_err(|error| build_pi_spawn_error(&error, local_program_label.as_deref()))?;

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
        "Unknown Pi extension command failure".to_string()
    };
    Err(annotate_pi_command_error(
        failure_message,
        local_program_label.as_deref(),
    ))
}

async fn probe_pi_cli_version(runtime_location: &RuntimeLocationInfo) -> Option<String> {
    let version = run_pi_command(runtime_location, &["--version"], true)
        .await
        .ok()?;
    let trimmed = version.lines().next().unwrap_or(version.as_str()).trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_unknown_no_approve_option_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let mentions_unknown_option = lower.contains("unknown option")
        || lower.contains("unknown argument")
        || lower.contains("unrecognized option");
    let mentions_no_approve = lower.contains("--no-approve")
        || lower.contains("-no-approve")
        || lower.contains("'-na'")
        || lower.contains("\"-na\"")
        || lower.split_whitespace().any(|token| {
            token.trim_matches(|c| c == '\'' || c == '"' || c == ',' || c == '.') == "-na"
        });
    mentions_unknown_option && mentions_no_approve
}

fn args_without_no_approve<'a>(args: &[&'a str]) -> Vec<&'a str> {
    args.iter()
        .copied()
        .filter(|arg| *arg != "--no-approve" && *arg != "-na")
        .collect()
}

/// Prefer `--no-approve` so non-interactive extension ops skip project-trust prompts.
/// Older / non-official `pi` builds may reject the flag on some subcommands (e.g. `list`);
/// fall back once without it so the UI still works.
async fn run_pi_command_preferring_no_approve(
    runtime_location: &RuntimeLocationInfo,
    args: &[&str],
    offline: bool,
) -> Result<(String, Vec<String>), String> {
    let used_args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    match run_pi_command(runtime_location, args, offline).await {
        Ok(output) => Ok((output, used_args)),
        Err(error)
            if args
                .iter()
                .any(|arg| *arg == "--no-approve" || *arg == "-na")
                && is_unknown_no_approve_option_error(&error) =>
        {
            let fallback_args = args_without_no_approve(args);
            let output = run_pi_command(runtime_location, &fallback_args, offline).await?;
            Ok((
                output,
                fallback_args
                    .iter()
                    .map(|arg| (*arg).to_string())
                    .collect(),
            ))
        }
        Err(error) => Err(error),
    }
}

fn format_pi_command_owned(args: &[String]) -> String {
    format!("pi {}", args.join(" "))
}

fn is_cli_package_source(source: &str) -> bool {
    let lower_source = source.trim().to_ascii_lowercase();
    ["npm:", "file:", "github:", "git:", "http:", "https:"]
        .iter()
        .any(|prefix| lower_source.starts_with(prefix))
}

fn is_protected_local_extension_source(source: &str) -> bool {
    let source = source.trim();
    source.starts_with("pi-deck-") || source.starts_with("ai-toolbox-")
}

fn parse_list_output(raw: &str) -> Vec<PiExtensionSummary> {
    let mut result = Vec::new();
    let mut scope = PiExtensionScope::Unknown;
    let mut pending_index: Option<usize> = None;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("User packages:") {
            scope = PiExtensionScope::User;
            pending_index = None;
            continue;
        }
        if trimmed.eq_ignore_ascii_case("Project packages:") {
            scope = PiExtensionScope::Project;
            pending_index = None;
            continue;
        }
        if is_cli_package_source(trimmed) {
            result.push(PiExtensionSummary {
                id: format!("{}:{}", scope_id(scope), trimmed),
                source: trimmed.to_string(),
                scope,
                kind: PiExtensionKind::Package,
                path: None,
                built_in: false,
                current_version: None,
            });
            pending_index = Some(result.len() - 1);
            continue;
        }
        if let Some(index) = pending_index {
            if result[index].path.is_none() {
                result[index].path = Some(trimmed.to_string());
            }
        }
    }

    result
}

fn scope_id(scope: PiExtensionScope) -> &'static str {
    match scope {
        PiExtensionScope::User => "user",
        PiExtensionScope::Project => "project",
        PiExtensionScope::Unknown => "unknown",
    }
}

fn scan_local_extensions(extensions_path: &Path) -> Result<Vec<PiExtensionSummary>, String> {
    let entries = match fs::read_dir(extensions_path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "Failed to read Pi extensions directory {}: {error}",
                extensions_path.display()
            ));
        }
    };

    let mut result = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "Failed to read Pi extensions directory entry in {}: {error}",
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
            format!("Failed to inspect Pi extension {}: {error}", path.display())
        })?;
        let (source, kind) = if file_type.is_file() && file_name.ends_with(".ts") {
            (file_name.to_string(), PiExtensionKind::LocalFile)
        } else if file_type.is_dir() && path.join("index.ts").is_file() {
            (file_name.to_string(), PiExtensionKind::LocalDirectory)
        } else {
            continue;
        };

        result.push(PiExtensionSummary {
            id: format!("local:{source}"),
            built_in: is_protected_local_extension_source(&source),
            source,
            scope: PiExtensionScope::User,
            kind,
            path: Some(path.to_string_lossy().to_string()),
            current_version: None,
        });
    }

    result.sort_by(|left, right| left.source.cmp(&right.source));
    Ok(result)
}

fn read_package_current_version(extension: &PiExtensionSummary) -> Option<String> {
    if extension.kind != PiExtensionKind::Package {
        return None;
    }
    let path = extension.path.as_deref()?;
    let package_json_path = Path::new(path).join("package.json");
    let raw = fs::read_to_string(package_json_path).ok()?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    parsed
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn enrich_current_versions(extensions: Vec<PiExtensionSummary>) -> Vec<PiExtensionSummary> {
    extensions
        .into_iter()
        .map(|extension| PiExtensionSummary {
            current_version: read_package_current_version(&extension),
            ..extension
        })
        .collect()
}

fn merge_extensions(
    pi_extensions: Vec<PiExtensionSummary>,
    local_extensions: Vec<PiExtensionSummary>,
) -> Vec<PiExtensionSummary> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();

    for extension in pi_extensions {
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

fn extension_identity(extension: &PiExtensionSummary) -> String {
    extension
        .path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .unwrap_or(&extension.source)
        .to_string()
}

fn delete_local_extension(
    extensions_path: &Path,
    input: &PiExtensionActionInput,
) -> Result<(), String> {
    let source = input.source.trim();
    if source.is_empty() {
        return Err("Pi extension source cannot be empty".to_string());
    }
    if is_protected_local_extension_source(source) {
        return Err("Built-in Pi extension cannot be deleted".to_string());
    }

    let target_path = input
        .path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| extensions_path.join(source));
    let canonical_extensions_path = fs::canonicalize(extensions_path).map_err(|error| {
        format!(
            "Failed to resolve Pi extensions directory {}: {error}",
            extensions_path.display()
        )
    })?;
    let canonical_target_path = fs::canonicalize(&target_path).map_err(|error| {
        format!(
            "Failed to resolve Pi extension path {}: {error}",
            target_path.display()
        )
    })?;
    if !canonical_target_path.starts_with(&canonical_extensions_path) {
        return Err(format!(
            "Pi extension path is outside extensions directory: {}",
            canonical_target_path.display()
        ));
    }

    if canonical_target_path.is_dir() {
        fs::remove_dir_all(&canonical_target_path).map_err(|error| {
            format!(
                "Failed to delete Pi extension directory {}: {error}",
                canonical_target_path.display()
            )
        })
    } else {
        fs::remove_file(&canonical_target_path).map_err(|error| {
            format!(
                "Failed to delete Pi extension file {}: {error}",
                canonical_target_path.display()
            )
        })
    }
}

fn emit_extensions_changed(app: &tauri::AppHandle, payload: &str) {
    let _ = app.emit("config-changed", payload);
}

#[tauri::command]
pub async fn list_pi_extensions(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<PiExtensionListResult, String> {
    let db = state.db();
    let runtime_location = runtime_location::get_pi_runtime_location_async(&db).await?;
    let extensions_path = get_pi_extensions_path_from_root(&runtime_location.host_path);
    let packages_path = get_pi_packages_path_from_root(&runtime_location.host_path);
    let (raw, _) =
        run_pi_command_preferring_no_approve(&runtime_location, &["list", "--no-approve"], true)
            .await?;
    let pi_extensions = enrich_current_versions(parse_list_output(&raw));
    let local_extensions = scan_local_extensions(&extensions_path)?;
    let cli_path = resolve_pi_cli_display_path(&runtime_location);
    let cli_version = probe_pi_cli_version(&runtime_location).await;

    Ok(PiExtensionListResult {
        extensions_path: extensions_path.to_string_lossy().to_string(),
        packages_path: packages_path.to_string_lossy().to_string(),
        extensions: merge_extensions(pi_extensions, local_extensions),
        raw,
        cli_path,
        cli_version,
    })
}

#[tauri::command]
pub async fn install_pi_extension(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: PiExtensionInstallInput,
) -> Result<PiExtensionCommandResult, String> {
    let source = input.source.trim();
    if source.is_empty() {
        return Err("Pi extension source cannot be empty".to_string());
    }

    let db = state.db();
    let runtime_location = runtime_location::get_pi_runtime_location_async(&db).await?;
    let args = ["install", source, "--no-approve"];
    let (output, used_args) =
        run_pi_command_preferring_no_approve(&runtime_location, &args, true).await?;
    emit_extensions_changed(&app, "pi-extensions");

    Ok(PiExtensionCommandResult {
        command: format_pi_command_owned(&used_args),
        output: output.trim().to_string(),
    })
}

#[tauri::command]
pub async fn uninstall_pi_extension(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    input: PiExtensionActionInput,
) -> Result<PiExtensionCommandResult, String> {
    let source = input.source.trim();
    if source.is_empty() {
        return Err("Pi extension source cannot be empty".to_string());
    }

    let db = state.db();
    let runtime_location = runtime_location::get_pi_runtime_location_async(&db).await?;
    let extensions_path = get_pi_extensions_path_from_root(&runtime_location.host_path);
    let kind = input.kind.unwrap_or(PiExtensionKind::Package);

    if kind != PiExtensionKind::Package {
        delete_local_extension(&extensions_path, &input)?;
        emit_extensions_changed(&app, "pi-extensions");
        return Ok(PiExtensionCommandResult {
            command: format!("delete {}", source),
            output: String::new(),
        });
    }

    let mut args = vec!["remove", source];
    if input.scope == Some(PiExtensionScope::Project) {
        args.push("-l");
    }
    args.push("--no-approve");

    let (output, used_args) =
        run_pi_command_preferring_no_approve(&runtime_location, &args, true).await?;
    emit_extensions_changed(&app, "pi-extensions");

    Ok(PiExtensionCommandResult {
        command: format_pi_command_owned(&used_args),
        output: output.trim().to_string(),
    })
}

#[tauri::command]
pub async fn update_pi_extensions(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<PiExtensionCommandResult, String> {
    let db = state.db();
    let runtime_location = runtime_location::get_pi_runtime_location_async(&db).await?;
    // Prefer --no-approve for CLI builds that still accept it on update; fall back if rejected.
    let args = ["update", "--extensions", "--no-approve"];
    let (output, used_args) =
        run_pi_command_preferring_no_approve(&runtime_location, &args, false).await?;
    emit_extensions_changed(&app, "pi-extensions");

    Ok(PiExtensionCommandResult {
        command: format_pi_command_owned(&used_args),
        output: output.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_wsl_path_prefix_includes_common_user_install_locations() {
        let prefix = pi_wsl_path_prefix(Some("/home/tester"));
        assert_eq!(
            prefix,
            "/home/tester/.local/share/mise/shims:\
/home/tester/.asdf/shims:\
/home/tester/.local/bin:\
/home/tester/.bun/bin:\
/home/tester/.volta/bin:\
/home/tester/.local/share/fnm/aliases/default/bin:\
/home/tester/.fnm/aliases/default/bin:\
/home/tester/.fnm/current/bin:\
/home/tester/.npm-global/bin"
        );
        assert_eq!(pi_wsl_path_prefix(None), "");
    }

    #[test]
    fn build_pi_command_keeps_wsl_paths_and_cli_args_out_of_shell_script() {
        let runtime_location = RuntimeLocationInfo {
            mode: RuntimeLocationMode::WslDirect,
            source: "custom".to_string(),
            host_path: PathBuf::from(
                r"\\wsl.localhost\Ubuntu\home\test user\.pi;echo injected\agent",
            ),
            wsl: Some(runtime_location::WslLocationInfo {
                distro: "Ubuntu".to_string(),
                linux_path: "/home/test user/.pi;echo injected/agent".to_string(),
                linux_user_root: Some("/home/test user".to_string()),
            }),
        };
        let package_source = "file:/tmp/extension dir;$(touch /tmp/injected)";
        let invocation = build_pi_command(
            &runtime_location,
            &["install", package_source, "--no-approve"],
            true,
        )
        .expect("build WSL Pi command");
        let command = invocation.command.as_std();
        let command_args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(command.get_program(), "wsl");
        assert_eq!(
            command_args[0..7],
            [
                "-d",
                "Ubuntu",
                "--exec",
                "/bin/sh",
                "-c",
                WSL_PI_COMMAND_SCRIPT,
                "ai-toolbox-pi"
            ]
        );
        assert_eq!(
            command_args[7],
            "/home/test user/.local/share/mise/shims:\
/home/test user/.asdf/shims:\
/home/test user/.local/bin:\
/home/test user/.bun/bin:\
/home/test user/.volta/bin:\
/home/test user/.local/share/fnm/aliases/default/bin:\
/home/test user/.fnm/aliases/default/bin:\
/home/test user/.fnm/current/bin:\
/home/test user/.npm-global/bin"
        );
        assert_eq!(command_args[8], "/home/test user/.pi;echo injected/agent");
        assert_eq!(
            &command_args[9..],
            [
                "env",
                "NPM_CONFIG_LEGACY_PEER_DEPS=true",
                "PI_OFFLINE=1",
                "pi",
                "install",
                package_source,
                "--no-approve",
            ]
        );
        assert!(!WSL_PI_COMMAND_SCRIPT.contains("test user"));
        assert!(!WSL_PI_COMMAND_SCRIPT.contains("injected"));
        assert!(WSL_PI_COMMAND_SCRIPT.contains("${PATH:+:$PATH}"));
    }

    #[test]
    fn parses_pi_list_output_with_user_and_project_scopes() {
        let raw = r#"
User packages:
  npm:context-mode
    /home/tester/.pi/agent/npm/node_modules/context-mode
Project packages:
  github:owner/repo
    /project/.pi/extensions/repo
"#;

        let extensions = parse_list_output(raw);

        assert_eq!(extensions.len(), 2);
        assert_eq!(extensions[0].source, "npm:context-mode");
        assert_eq!(extensions[0].scope, PiExtensionScope::User);
        assert_eq!(extensions[0].kind, PiExtensionKind::Package);
        assert_eq!(
            extensions[0].path.as_deref(),
            Some("/home/tester/.pi/agent/npm/node_modules/context-mode")
        );
        assert_eq!(extensions[1].source, "github:owner/repo");
        assert_eq!(extensions[1].scope, PiExtensionScope::Project);
    }

    #[test]
    fn detects_unknown_no_approve_option_errors() {
        assert!(is_unknown_no_approve_option_error(
            "Unknown option --no-approve for \"list\". Use \"pi --help\" or \"pi list\"."
        ));
        assert!(is_unknown_no_approve_option_error(
            "error: unknown option '-na'"
        ));
        assert!(!is_unknown_no_approve_option_error(
            "Failed to install package npm:foo"
        ));
        assert!(!is_unknown_no_approve_option_error(
            "Unknown option --offline for \"list\"."
        ));
    }

    #[test]
    fn strips_no_approve_flags_from_args() {
        assert_eq!(
            args_without_no_approve(&["list", "--no-approve"]),
            vec!["list"]
        );
        assert_eq!(
            args_without_no_approve(&["remove", "npm:foo", "-l", "-na"]),
            vec!["remove", "npm:foo", "-l"]
        );
        assert_eq!(
            args_without_no_approve(&["update", "--extensions"]),
            vec!["update", "--extensions"]
        );
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
        fs::create_dir(extensions_path.join("without-index")).expect("mkdir without index");

        let extensions = scan_local_extensions(extensions_path).expect("scan");
        let sources: Vec<_> = extensions
            .iter()
            .map(|extension| (extension.source.as_str(), extension.kind))
            .collect();

        assert_eq!(
            sources,
            vec![
                ("directory", PiExtensionKind::LocalDirectory),
                ("single.ts", PiExtensionKind::LocalFile),
            ]
        );
    }

    #[test]
    fn pi_extension_npm_env_uses_legacy_peer_deps() {
        assert_eq!(
            pi_extension_npm_compat_env(),
            [(NPM_LEGACY_PEER_DEPS_ENV_KEY, NPM_LEGACY_PEER_DEPS_ENV_VALUE)]
        );
    }

    #[test]
    fn annotates_command_errors_with_resolved_pi_cli_path() {
        assert_eq!(
            annotate_pi_command_error(
                "Unknown option --no-approve for \"list\".".to_string(),
                Some("/opt/homebrew/bin/pi"),
            ),
            "Unknown option --no-approve for \"list\".\npi_cli=/opt/homebrew/bin/pi"
        );
        assert_eq!(
            annotate_pi_command_error(
                "already mentions /usr/local/bin/pi".to_string(),
                Some("/usr/local/bin/pi"),
            ),
            "already mentions /usr/local/bin/pi"
        );
        assert_eq!(
            annotate_pi_command_error("no label".to_string(), None),
            "no label"
        );
    }
}
