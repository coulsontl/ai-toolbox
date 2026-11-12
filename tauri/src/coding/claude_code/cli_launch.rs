//! Launch Claude Code CLI with a temporary provider-specific settings file.
//!
//! Scheme A (aligned with cc-switch): create a minimal temp settings JSON that only
//! contains this provider's `env` fields, then run `claude --settings <temp>`.
//! Does NOT rewrite the user's real settings.json and does NOT change is_applied.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "windows")]
use std::process::Stdio;

use serde_json::{json, Map, Value};

use crate::coding::cli_resolver::resolve_local_claude_program;
use crate::coding::runtime_location::{RuntimeLocationInfo, RuntimeLocationMode};

#[cfg(target_os = "windows")]
use crate::coding::cli_launch_terminal::{
    build_windows_batch_script, escape_windows_batch_quoted_value, windows_batch_preamble,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Validate and normalize a user-selected working directory for CLI launch.
///
/// Returns `Ok(None)` when `cwd` is empty/absent (launch without `cd`).
/// Rejects paths with newlines (shell injection guard), verifies the path
/// exists and is a directory, and canonicalizes it. On Windows the canonical
/// `\\?\` verbatim prefix — which `cmd.exe` dislikes — is stripped so the
/// path is safe to embed in a `.bat` `cd /d` line.
pub fn resolve_launch_cwd(cwd: Option<String>) -> Result<Option<PathBuf>, String> {
    let Some(raw) = cwd.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };

    if raw.contains('\n') || raw.contains('\r') {
        return Err("Directory path contains illegal newline".to_string());
    }

    let path = Path::new(&raw);
    if !path.exists() {
        return Err(format!("Directory does not exist: {raw}"));
    }

    let resolved =
        std::fs::canonicalize(path).map_err(|e| format!("Failed to resolve directory: {e}"))?;
    if !resolved.is_dir() {
        return Err(format!(
            "Selected path is not a directory: {}",
            resolved.display()
        ));
    }

    #[cfg(target_os = "windows")]
    {
        let resolved = strip_verbatim_prefix(&resolved);
        return Ok(Some(resolved));
    }

    #[cfg(not(target_os = "windows"))]
    Ok(Some(resolved))
}

/// Strip the `\\?\` verbatim prefix that `std::fs::canonicalize` adds on Windows
/// so the path works inside `.bat` scripts and `cmd.exe`.
#[cfg(target_os = "windows")]
fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut components = path.components();
    let Some(first) = components.next() else {
        return path.to_path_buf();
    };

    match first {
        Component::Prefix(prefix) => match prefix.kind() {
            std::path::Prefix::VerbatimDisk(letter) => {
                let rest: PathBuf = components.collect();
                let mut result = PathBuf::from(format!("{}:", char::from(letter)));
                result.push(rest);
                result
            }
            std::path::Prefix::VerbatimUNC(server, share) => {
                let mut result = PathBuf::from(r"\\");
                result.push(server);
                result.push(share);
                let rest: PathBuf = components.collect();
                result.push(rest);
                result
            }
            _ => path.to_path_buf(),
        },
        _ => path.to_path_buf(),
    }
}

/// Build a minimal Claude settings object from a provider's stored settings_config JSON.
/// Only non-empty string env values are kept (same idea as cc-switch).
pub fn build_minimal_temp_settings(settings_config: &str) -> Result<Value, String> {
    let parsed: Value = serde_json::from_str(settings_config)
        .map_err(|e| format!("Failed to parse provider settings: {e}"))?;

    let mut env_map = Map::new();
    if let Some(env) = parsed.get("env").and_then(|value| value.as_object()) {
        for (key, value) in env {
            if let Some(string_value) = value.as_str() {
                let trimmed = string_value.trim();
                if !trimmed.is_empty() {
                    env_map.insert(key.clone(), Value::String(trimmed.to_string()));
                }
            }
        }
    }

    Ok(json!({ "env": Value::Object(env_map) }))
}

fn temp_settings_has_env(settings: &Value) -> bool {
    settings
        .get("env")
        .and_then(|value| value.as_object())
        .map(|env| !env.is_empty())
        .unwrap_or(false)
}

fn sanitize_provider_id_for_filename(provider_id: &str) -> String {
    provider_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn unix_rm_command(paths: &[String]) -> String {
    if paths.is_empty() {
        return ":".to_string();
    }
    format!(
        "rm -f {}",
        paths
            .iter()
            .map(|path| shell_single_quote(path))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn unix_trap_cleanup_line(paths: &[String]) -> String {
    format!("trap {} EXIT", shell_single_quote(&unix_rm_command(paths)))
}

fn unix_settings_echo_line(settings_path: Option<&str>) -> String {
    settings_path
        .map(|path| format!("echo {}", shell_single_quote(path)))
        .unwrap_or_else(|| {
            "echo \"(no provider env overrides; using current Claude login/settings)\"".to_string()
        })
}

/// Shared launcher script body for sh-compatible sessions (macOS, Linux, the
/// WSL inner script and the Windows Git Bash launcher): set config dir, cd,
/// run Claude, clean up temp files, then keep the window open until Enter.
fn build_unix_session_script(
    claude_line: &str,
    config_dir_line: &str,
    cd_line: &str,
    settings_echo: &str,
    cleanup_paths: &[String],
) -> String {
    format!(
        r#"#!/usr/bin/env sh
{trap_cleanup}
{config_dir_line}
{cd_line}
echo "Using temporary Claude provider settings (does not change applied config)"
{settings_echo}
{claude_line}
status=$?
{cleanup}
echo ""
echo "Claude exited with code $status. Press Enter to close."
read -r _
"#,
        trap_cleanup = unix_trap_cleanup_line(cleanup_paths),
        config_dir_line = config_dir_line,
        cd_line = cd_line,
        settings_echo = settings_echo,
        claude_line = claude_line,
        cleanup = unix_rm_command(cleanup_paths),
    )
}

fn build_claude_cli_args(settings_path: Option<&str>, full_access: bool) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(path) = settings_path {
        args.push("--settings".to_string());
        args.push(path.to_string());
    }
    if full_access {
        args.push("--dangerously-skip-permissions".to_string());
    }
    args
}

#[cfg(target_os = "windows")]
fn quote_for_batch(value: &str) -> String {
    // `%` expands even inside quotes, so double it before deciding whether the
    // argument needs quoting.
    let value = value.replace('%', "%%");
    if value.is_empty() {
        return "\"\"".to_string();
    }
    if !value.contains([' ', '\t', '"', '&', '|', '<', '>', '^', '%']) {
        return value;
    }
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn format_claude_command_line_with(
    claude_program: &str,
    settings_path: Option<&str>,
    full_access: bool,
    quote: fn(&str) -> String,
) -> String {
    let mut parts = vec![quote(claude_program)];
    for arg in build_claude_cli_args(settings_path, full_access) {
        parts.push(quote(&arg));
    }
    parts.join(" ")
}

fn format_claude_command_line(
    claude_program: &str,
    settings_path: Option<&str>,
    full_access: bool,
) -> String {
    #[cfg(target_os = "windows")]
    {
        format_claude_command_line_with(claude_program, settings_path, full_access, quote_for_batch)
    }
    #[cfg(not(target_os = "windows"))]
    {
        format_claude_command_line_with(
            claude_program,
            settings_path,
            full_access,
            shell_single_quote,
        )
    }
}

fn format_claude_command_line_for_unix(
    claude_program: &str,
    settings_path: Option<&str>,
    full_access: bool,
) -> String {
    format_claude_command_line_with(
        claude_program,
        settings_path,
        full_access,
        shell_single_quote,
    )
}

fn resolve_local_claude_command() -> String {
    let program = resolve_local_claude_program();
    program.path.to_string_lossy().to_string()
}

fn serialize_temp_settings(temp_settings: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(temp_settings)
        .map_err(|e| format!("Failed to serialize temp settings: {e}"))
}

fn write_local_temp_settings_file(safe_id: &str, temp_settings: &Value) -> Result<PathBuf, String> {
    let json = serialize_temp_settings(temp_settings)?;
    let mut temp_file = tempfile::Builder::new()
        .prefix(&format!("ai_toolbox_claude_{safe_id}_"))
        .suffix(".json")
        .tempfile_in(std::env::temp_dir())
        .map_err(|e| format!("Failed to create temp settings: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp_file
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("Failed to set temp settings permissions: {e}"))?;
    }
    temp_file
        .write_all(json.as_bytes())
        .map_err(|e| format!("Failed to write temp settings: {e}"))?;
    temp_file
        .flush()
        .map_err(|e| format!("Failed to flush temp settings: {e}"))?;
    let (_file, path) = temp_file
        .keep()
        .map_err(|e| format!("Failed to keep temp settings: {e}"))?;
    Ok(path)
}

fn random_temp_suffix() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn unix_export_claude_config_dir_line(claude_config_dir: &str) -> String {
    format!(
        "export CLAUDE_CONFIG_DIR={}",
        shell_single_quote(claude_config_dir)
    )
}

// Compiled on Windows (production use) and under `cfg(test)` (cross-platform
// unit test `detects_windows_batch_cli_shims` exercises the .cmd/.bat check).
// Without `test` in the gate, non-Windows lib builds report it as dead code.
#[cfg(any(target_os = "windows", test))]
fn is_windows_batch_script_path(value: &str) -> bool {
    let normalized = value.trim().trim_matches('"').to_ascii_lowercase();
    normalized.ends_with(".cmd") || normalized.ends_with(".bat")
}

#[cfg(target_os = "windows")]
fn windows_set_claude_config_dir_line(claude_config_dir: &str) -> String {
    format!(
        "set \"CLAUDE_CONFIG_DIR={}\"",
        escape_windows_batch_quoted_value(claude_config_dir)
    )
}

#[cfg(target_os = "windows")]
fn format_claude_batch_invocation_line(
    claude_program: &str,
    settings_path: Option<&str>,
    full_access: bool,
) -> String {
    let claude_line = format_claude_command_line(claude_program, settings_path, full_access);
    if is_windows_batch_script_path(claude_program) {
        format!("call {claude_line}")
    } else {
        claude_line
    }
}

#[cfg(target_os = "windows")]
fn write_wsl_temp_settings_file(
    distro: &str,
    linux_path: &str,
    temp_settings: &Value,
) -> Result<(), String> {
    let json = serialize_temp_settings(temp_settings)?;
    let mut child = Command::new("wsl")
        .args([
            "-d",
            distro,
            "--exec",
            "sh",
            "-c",
            "umask 077 && cat > \"$1\" && chmod 600 \"$1\"",
            "sh",
            linux_path,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Failed to create WSL temp settings: {e}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Failed to open WSL temp settings stdin".to_string())?;
    if let Err(error) = stdin.write_all(json.as_bytes()) {
        let _ = child.kill();
        return Err(format!("Failed to write WSL temp settings: {error}"));
    }
    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to finish WSL temp settings write: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Failed to create WSL temp settings (exit {:?}): {stderr}",
            output.status.code()
        ));
    }
    Ok(())
}

/// Write a temp settings file and open a system terminal that runs Claude CLI with it.
///
/// `preferred_terminal` selects the Windows terminal app used to open the
/// launcher (`cmd` / `powershell` / `wt` / `gitbash`); it is ignored on other
/// platforms and falls back to the system default when unavailable.
pub fn launch_claude_provider_cli_session(
    runtime_location: &RuntimeLocationInfo,
    provider_id: &str,
    settings_config: &str,
    full_access: bool,
    cwd: Option<&Path>,
    preferred_terminal: Option<&str>,
) -> Result<(), String> {
    let temp_settings = build_minimal_temp_settings(settings_config)?;
    let include_settings = temp_settings_has_env(&temp_settings);

    match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => launch_local_session(
            &runtime_location.host_path.to_string_lossy(),
            provider_id,
            &temp_settings,
            include_settings,
            full_access,
            cwd,
            preferred_terminal,
        ),
        RuntimeLocationMode::WslDirect => {
            let wsl = runtime_location
                .wsl
                .as_ref()
                .ok_or_else(|| "Missing WSL runtime metadata for Claude CLI launch".to_string())?;
            launch_wsl_session(
                &wsl.distro,
                &wsl.linux_path,
                provider_id,
                &temp_settings,
                include_settings,
                full_access,
                cwd,
                preferred_terminal,
            )
        }
    }
}

fn launch_local_session(
    claude_config_dir: &str,
    provider_id: &str,
    temp_settings: &Value,
    include_settings: bool,
    full_access: bool,
    cwd: Option<&Path>,
    preferred_terminal: Option<&str>,
) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    let _ = preferred_terminal;

    let safe_id = sanitize_provider_id_for_filename(provider_id);

    let settings_file = if include_settings {
        Some(write_local_temp_settings_file(&safe_id, temp_settings)?)
    } else {
        None
    };

    let settings_path_str = settings_file
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    let claude_program = resolve_local_claude_command();

    #[cfg(target_os = "windows")]
    {
        // `temp_dir` and `pid` are only consumed by the Windows terminal launcher
        // (named temp `.bat` files are PID-suffixed to avoid collisions). Keep them
        // Windows-gated so non-Windows builds don't warn about unused bindings.
        let temp_dir = std::env::temp_dir();
        let pid = std::process::id();
        launch_windows_terminal(
            &temp_dir,
            claude_config_dir,
            settings_file.as_deref(),
            settings_path_str.as_deref(),
            &claude_program,
            full_access,
            &safe_id,
            pid,
            cwd,
            preferred_terminal,
        )?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        launch_macos_terminal(
            claude_config_dir,
            settings_file.as_deref(),
            settings_path_str.as_deref(),
            &claude_program,
            full_access,
            cwd,
        )?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        launch_linux_terminal(
            claude_config_dir,
            settings_file.as_deref(),
            settings_path_str.as_deref(),
            &claude_program,
            full_access,
            cwd,
        )?;
        return Ok(());
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (
            settings_file,
            settings_path_str,
            claude_program,
            claude_config_dir,
            full_access,
            safe_id,
        );
        Err("Unsupported operating system for Claude CLI launch".to_string())
    }
}

fn launch_wsl_session(
    distro: &str,
    claude_config_dir: &str,
    provider_id: &str,
    temp_settings: &Value,
    include_settings: bool,
    full_access: bool,
    cwd: Option<&Path>,
    preferred_terminal: Option<&str>,
) -> Result<(), String> {
    let safe_id = sanitize_provider_id_for_filename(provider_id);
    let pid = std::process::id();
    let temp_suffix = random_temp_suffix();
    let linux_settings_file_name = format!("ai_toolbox_claude_{safe_id}_{pid}_{temp_suffix}.json");
    let linux_settings_path = format!("/tmp/{linux_settings_file_name}");
    let linux_script = format!("/tmp/ai_toolbox_claude_launch_{safe_id}_{pid}.sh");

    if include_settings {
        #[cfg(target_os = "windows")]
        {
            write_wsl_temp_settings_file(distro, &linux_settings_path, temp_settings)?;
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = temp_settings;
        }
    }

    let settings_arg = if include_settings {
        Some(linux_settings_path.as_str())
    } else {
        None
    };
    // Inside WSL, rely on distro PATH for `claude` and always use shell quoting.
    let claude_line = format_claude_command_line_for_unix("claude", settings_arg, full_access);
    let mut cleanup_paths = Vec::new();
    if include_settings {
        cleanup_paths.push(linux_settings_path.clone());
    }
    cleanup_paths.push(linux_script.clone());

    // Convert the user-selected Windows working directory to a WSL Linux path so
    // the inner shell can `cd` into it. Falls back to the raw value if conversion
    // fails (e.g. already a Linux path or a UNC path muda handled elsewhere).
    let cd_line = cwd
        .map(|path| {
            let windows_path = path.to_string_lossy();
            let linux_path =
                crate::coding::wsl::windows_to_wsl_path(&windows_path).unwrap_or_else(|_| {
                    // If it is already a WSL UNC path, parse_wsl_unc_path extracts the
                    // Linux path directly; otherwise keep the raw value as a best effort.
                    crate::coding::runtime_location::parse_wsl_unc_path(&windows_path)
                        .map(|info| info.linux_path)
                        .unwrap_or_else(|| windows_path.to_string())
                });
            format!("cd {}", shell_single_quote(&linux_path))
        })
        .unwrap_or_default();

    let settings_echo = if include_settings {
        format!("echo {}", shell_single_quote(&linux_settings_path))
    } else {
        unix_settings_echo_line(None)
    };
    let inner = build_unix_session_script(
        &claude_line,
        &unix_export_claude_config_dir_line(claude_config_dir),
        &cd_line,
        &settings_echo,
        &cleanup_paths,
    );

    #[cfg(target_os = "windows")]
    {
        // Open Windows terminal that attaches to WSL and runs the script.
        let temp_dir = std::env::temp_dir();
        let bat_file = temp_dir.join(format!("ai_toolbox_claude_wsl_{safe_id}_{pid}.bat"));
        let escaped_inner = inner.replace('\r', "");
        // Pass script via wsl bash -lc. Use base64-free simple path: write .sh on UNC then bash it.
        let unc_script = PathBuf::from(format!(
            r"\\wsl.localhost\{}\tmp\ai_toolbox_claude_launch_{}_{}.sh",
            distro, safe_id, pid
        ));
        fs::write(
            &unc_script,
            format!("#!/usr/bin/env bash\n{escaped_inner}\n"),
        )
        .map_err(|e| format!("Failed to write WSL launch script: {e}"))?;

        let content = format!(
            "{preamble}\
wsl -d {distro} --exec bash {script}\r\n\
del \"%~f0\" >nul 2>&1\r\n",
            preamble = windows_batch_preamble(),
            distro = distro,
            script = linux_script,
        );
        fs::write(&bat_file, content)
            .map_err(|e| format!("Failed to write batch launcher: {e}"))?;
        let result = if normalize_preferred_terminal(preferred_terminal) == Some("gitbash") {
            launch_git_bash_wsl_session(distro, &linux_script)
        } else {
            launch_bat_in_preferred_terminal(&bat_file, preferred_terminal)
        };
        if result.is_ok() && normalize_preferred_terminal(preferred_terminal) == Some("gitbash") {
            // Same as the local path: the .bat launcher is never executed.
            let _ = fs::remove_file(&bat_file);
        }
        fall_back_to_cmd_on_failure(result, preferred_terminal, &bat_file)?;
        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (distro, claude_config_dir, inner, preferred_terminal);
        Err("WSL Direct Claude CLI launch is only supported on Windows".to_string())
    }
}

#[cfg(target_os = "windows")]
fn launch_windows_terminal(
    temp_dir: &Path,
    claude_config_dir: &str,
    settings_file: Option<&Path>,
    settings_path: Option<&str>,
    claude_program: &str,
    full_access: bool,
    safe_id: &str,
    pid: u32,
    cwd: Option<&Path>,
    preferred_terminal: Option<&str>,
) -> Result<(), String> {
    let bat_file = temp_dir.join(format!("ai_toolbox_claude_launch_{safe_id}_{pid}.bat"));
    let claude_line =
        format_claude_batch_invocation_line(claude_program, settings_path, full_access);
    let config_dir_line = windows_set_claude_config_dir_line(claude_config_dir);
    let settings_display = settings_path
        .map(escape_windows_batch_quoted_value)
        .unwrap_or_else(|| "(none)".to_string());
    let delete_settings = settings_file
        .map(|path| {
            format!(
                "del \"{}\" >nul 2>&1\r\n",
                escape_windows_batch_quoted_value(&path.to_string_lossy())
            )
        })
        .unwrap_or_default();

    let setup_lines = format!("{config_dir_line}\r\n");
    let launch_lines = format!(
        "echo Using temporary Claude provider settings (does not change applied config)\r\n\
echo \"{settings_display}\"\r\n\
{claude_line}\r\n",
        settings_display = settings_display,
        claude_line = claude_line,
    );
    let content = build_windows_batch_script(&setup_lines, cwd, &launch_lines, &delete_settings);

    fs::write(&bat_file, content).map_err(|e| format!("Failed to write batch launcher: {e}"))?;
    let preferred = normalize_preferred_terminal(preferred_terminal);
    let result = if preferred == Some("gitbash") {
        launch_git_bash_terminal(
            temp_dir,
            claude_config_dir,
            settings_file,
            settings_path,
            claude_program,
            full_access,
            safe_id,
            pid,
            cwd,
        )
    } else {
        launch_bat_in_preferred_terminal(&bat_file, preferred_terminal)
    };
    if result.is_ok() && preferred == Some("gitbash") {
        // Git Bash never executes the .bat launcher, so its self-delete line
        // never runs; clean it up here instead.
        let _ = fs::remove_file(&bat_file);
    }
    fall_back_to_cmd_on_failure(result, preferred_terminal, &bat_file)
}

/// Build the `start` arguments that open a launcher `.bat` in the preferred
/// Windows terminal. Returns `(start args, terminal label)`; `None`, empty or
/// unknown preferences fall back to `cmd` (the system default).
#[cfg(any(target_os = "windows", test))]
fn windows_terminal_start_args(
    preferred_terminal: Option<&str>,
    bat_path: &str,
) -> (Vec<String>, &'static str) {
    let bat = bat_path.to_string();
    match preferred_terminal.map(|value| value.trim()) {
        Some("powershell") => (
            vec![
                "powershell".to_string(),
                "-NoExit".to_string(),
                "-Command".to_string(),
                format!("& '{bat}'"),
            ],
            "PowerShell",
        ),
        Some("wt") => (
            vec!["wt".to_string(), "cmd".to_string(), "/K".to_string(), bat],
            "Windows Terminal",
        ),
        _ => (vec!["cmd".to_string(), "/K".to_string(), bat], "cmd"),
    }
}

/// Open a launcher `.bat` in the user's preferred Windows terminal.
#[cfg(target_os = "windows")]
fn launch_bat_in_preferred_terminal(
    bat_file: &Path,
    preferred_terminal: Option<&str>,
) -> Result<(), String> {
    let bat_path = bat_file.to_string_lossy().to_string();
    let (args, terminal_name) =
        windows_terminal_start_args(normalize_preferred_terminal(preferred_terminal), &bat_path);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_windows_start_command(&arg_refs, terminal_name)
}

/// Normalize a stored terminal preference: trim and treat empty as unset.
#[cfg(target_os = "windows")]
fn normalize_preferred_terminal(preferred_terminal: Option<&str>) -> Option<&str> {
    preferred_terminal
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

/// When a non-default preferred terminal fails to start (e.g. Windows Terminal
/// or Git Bash not installed), fall back to `cmd` so the CLI still opens.
#[cfg(target_os = "windows")]
fn fall_back_to_cmd_on_failure(
    result: Result<(), String>,
    preferred_terminal: Option<&str>,
    bat_file: &Path,
) -> Result<(), String> {
    let preferred = normalize_preferred_terminal(preferred_terminal);
    if result.is_err() && preferred.is_some() && preferred != Some("cmd") {
        log::warn!(
            "Preferred terminal {preferred:?} failed to launch, falling back to cmd: {:?}",
            result.as_ref().err()
        );
        return run_windows_start_command(&["cmd", "/K", &bat_file.to_string_lossy()], "cmd");
    }
    result
}

/// Convert a Windows path to its MSYS/Git-Bash POSIX form (`C:\a\b` →
/// `/c/a/b`, `\\srv\share` → `//srv/share`). Bash-internal operations (`cd`,
/// exec, script cleanup) need the POSIX form; arguments consumed by native
/// Windows programs keep their Windows form.
#[cfg(any(target_os = "windows", test))]
fn windows_path_to_msys_path(path: &Path) -> String {
    let win = path.to_string_lossy().replace('/', "\\");
    // Strip device prefixes: \\?\C:\a → C:\a, \\?\UNC\srv\share → \\srv\share.
    let win = if let Some(rest) = win.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{}", rest)
    } else if let Some(rest) = win.strip_prefix("\\\\?\\") {
        rest.to_string()
    } else {
        win
    };
    if let Some(rest) = win.strip_prefix("\\\\") {
        return format!("//{}", rest.replace('\\', "/"));
    }
    let mut chars = win.chars();
    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) => {
            let rest = chars.as_str().replace('\\', "/");
            let trimmed = rest.trim_start_matches('/');
            if trimmed.is_empty() {
                format!("/{}", drive.to_ascii_lowercase())
            } else {
                format!("/{}/{}", drive.to_ascii_lowercase(), trimmed)
            }
        }
        // Relative path: only normalize separators.
        _ => win.replace('\\', "/"),
    }
}

/// Resolve the Claude program for a Git Bash script: npm-style `.cmd`/`.bat`
/// shims cannot be exec'd by bash, so prefer their extensionless sh sibling
/// when it exists, then convert to the MSYS POSIX form.
#[cfg(any(target_os = "windows", test))]
fn resolve_bash_claude_command(claude_program: &str) -> String {
    let program = Path::new(claude_program);
    if is_windows_batch_script_path(claude_program) {
        let sibling = program.with_extension("");
        if sibling.is_file() {
            return windows_path_to_msys_path(&sibling);
        }
    }
    windows_path_to_msys_path(program)
}

/// Locate Git for Windows' mintty terminal (`usr\bin\mintty.exe` under the
/// install root): standard install roots first, then the install root derived
/// from `git.exe` on PATH. Returns `None` when Git Bash is not installed.
#[cfg(target_os = "windows")]
fn find_git_bash_mintty() -> Option<PathBuf> {
    let mut roots = vec![
        PathBuf::from("C:\\Program Files\\Git"),
        PathBuf::from("C:\\Program Files (x86)\\Git"),
    ];
    if let Some(local_data_dir) = dirs::data_local_dir() {
        roots.push(local_data_dir.join("Programs").join("Git"));
    }
    if let Ok(output) = Command::new("where")
        .arg("git")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(first_line) = stdout.lines().next() {
                let git_path = PathBuf::from(first_line.trim());
                // git.exe can live at <root>\cmd\git.exe, <root>\bin\git.exe or
                // <root>\mingw64\bin\git.exe; walk up a few levels to the root.
                for ancestor_level in 1..=4 {
                    if let Some(root) = git_path.ancestors().nth(ancestor_level) {
                        roots.push(root.to_path_buf());
                    }
                }
            }
        }
    }

    roots
        .into_iter()
        .map(|root| root.join("usr").join("bin").join("mintty.exe"))
        .find(|mintty| mintty.is_file())
}

/// Spawn mintty running `/usr/bin/bash --login` with the given bash arguments.
#[cfg(target_os = "windows")]
fn spawn_mintty_bash(mintty: &Path, bash_args: &[String]) -> Result<(), String> {
    Command::new(mintty)
        .arg("-t")
        .arg("AI Toolbox Claude")
        .arg("/usr/bin/bash")
        // Keep native-child arguments verbatim (e.g. WSL `/tmp/...` paths); the
        // launcher script already carries Windows forms where native programs
        // need them.
        .env("MSYS_NO_PATHCONV", "1")
        .env("MSYS2_ARG_CONV_EXCL", "*")
        .args(bash_args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to launch Git Bash: {e}"))
}

/// Open the Git Bash launcher script in mintty. Bash needs POSIX forms for its
/// own operations (`cd`, script path, trap cleanup) while Claude and its
/// `--settings` argument keep their Windows forms.
#[cfg(target_os = "windows")]
fn launch_git_bash_terminal(
    temp_dir: &Path,
    claude_config_dir: &str,
    settings_file: Option<&Path>,
    settings_path: Option<&str>,
    claude_program: &str,
    full_access: bool,
    safe_id: &str,
    pid: u32,
    cwd: Option<&Path>,
) -> Result<(), String> {
    let mintty =
        find_git_bash_mintty().ok_or_else(|| "Git Bash (mintty.exe) not found".to_string())?;

    let claude_command = resolve_bash_claude_command(claude_program);
    let claude_line = format_claude_command_line_with(
        &claude_command,
        settings_path,
        full_access,
        shell_single_quote,
    );
    let config_dir_line = unix_export_claude_config_dir_line(claude_config_dir);
    let cd_line = cwd
        .map(|path| {
            format!(
                "cd {}",
                shell_single_quote(&windows_path_to_msys_path(path))
            )
        })
        .unwrap_or_default();

    let script_file = temp_dir.join(format!("ai_toolbox_claude_launch_{safe_id}_{pid}.sh"));
    let mut cleanup_paths = Vec::new();
    if let Some(path) = settings_file {
        cleanup_paths.push(path.to_string_lossy().to_string());
    }
    cleanup_paths.push(windows_path_to_msys_path(&script_file));
    let script = build_unix_session_script(
        &claude_line,
        &config_dir_line,
        &cd_line,
        &unix_settings_echo_line(settings_path),
        &cleanup_paths,
    );
    fs::write(&script_file, script)
        .map_err(|e| format!("Failed to write Git Bash launcher: {e}"))?;

    spawn_mintty_bash(
        &mintty,
        &[
            "--login".to_string(),
            windows_path_to_msys_path(&script_file),
        ],
    )
}

/// Open a WSL Direct Claude session in Git Bash: run the existing WSL launch
/// command from bash so the inner Linux script keeps its own cleanup and pause.
#[cfg(target_os = "windows")]
fn launch_git_bash_wsl_session(distro: &str, linux_script: &str) -> Result<(), String> {
    let mintty =
        find_git_bash_mintty().ok_or_else(|| "Git Bash (mintty.exe) not found".to_string())?;
    let inner = format!(
        "wsl -d {distro} --exec bash {}",
        shell_single_quote(linux_script)
    );
    spawn_mintty_bash(&mintty, &["--login".to_string(), "-c".to_string(), inner])
}

#[cfg(target_os = "windows")]
fn run_windows_start_command(args: &[&str], terminal_name: &str) -> Result<(), String> {
    // `start` treats the first quoted argument as a window title; keep an empty title.
    let mut full_args: Vec<String> = vec!["/C".into(), "start".into(), "".into()];
    for arg in args {
        full_args.push((*arg).to_string());
    }
    // Quote the last path-like arg if needed is already done by callers for settings;
    // ensure bat path with spaces works when passed as a single argv element.

    // `start` hands off to a long-lived terminal. Capturing stdio with
    // `output()` would keep waiting for the inherited pipe handles until that
    // terminal closes, so the launch command never returns and the UI stays in
    // its loading state. Wait for `cmd` itself only; the terminal inherits no
    // stdio handles from us.
    let status = Command::new("cmd")
        .args(&full_args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to launch {terminal_name}: {e}"))?;

    if !status.success() {
        return Err(format!(
            "{terminal_name} launch failed (exit {:?})",
            status.code()
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_macos_terminal(
    claude_config_dir: &str,
    settings_file: Option<&Path>,
    settings_path: Option<&str>,
    claude_program: &str,
    full_access: bool,
    cwd: Option<&Path>,
) -> Result<(), String> {
    let claude_line = format_claude_command_line(claude_program, settings_path, full_access);
    let config_dir_line = unix_export_claude_config_dir_line(claude_config_dir);
    let cd_line = cwd
        .map(|path| format!("cd {}", shell_single_quote(&path.to_string_lossy())))
        .unwrap_or_default();
    let script_file = std::env::temp_dir().join(format!(
        "ai_toolbox_claude_launch_{}.sh",
        std::process::id()
    ));
    let mut cleanup_paths = Vec::new();
    if let Some(path) = settings_file {
        cleanup_paths.push(path.to_string_lossy().to_string());
    }
    cleanup_paths.push(script_file.to_string_lossy().to_string());
    let script = build_unix_session_script(
        &claude_line,
        &config_dir_line,
        &cd_line,
        &unix_settings_echo_line(settings_path),
        &cleanup_paths,
    );

    fs::write(&script_file, &script).map_err(|e| format!("Failed to write launch script: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script_file, fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to set script permissions: {e}"))?;
    }

    // Prefer a simple Terminal.app launch; keep KISS.
    let applescript = format!(
        r#"tell application "Terminal"
    activate
    do script {cmd}
end tell"#,
        cmd = applescript_string_literal(&format!(
            "exec sh {}",
            shell_single_quote(&script_file.to_string_lossy())
        )),
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(applescript)
        .output()
        .map_err(|e| format!("Failed to run osascript: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Terminal.app launch failed (exit {:?}): {stderr}",
            output.status.code()
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn applescript_string_literal(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(target_os = "linux")]
fn launch_linux_terminal(
    claude_config_dir: &str,
    settings_file: Option<&Path>,
    settings_path: Option<&str>,
    claude_program: &str,
    full_access: bool,
    cwd: Option<&Path>,
) -> Result<(), String> {
    let claude_line = format_claude_command_line(claude_program, settings_path, full_access);
    let config_dir_line = unix_export_claude_config_dir_line(claude_config_dir);
    let cd_line = cwd
        .map(|path| format!("cd {}", shell_single_quote(&path.to_string_lossy())))
        .unwrap_or_default();
    let script_file = std::env::temp_dir().join(format!(
        "ai_toolbox_claude_launch_{}.sh",
        std::process::id()
    ));
    let mut cleanup_paths = Vec::new();
    if let Some(path) = settings_file {
        cleanup_paths.push(path.to_string_lossy().to_string());
    }
    cleanup_paths.push(script_file.to_string_lossy().to_string());
    let script = build_unix_session_script(
        &claude_line,
        &config_dir_line,
        &cd_line,
        &unix_settings_echo_line(settings_path),
        &cleanup_paths,
    );

    fs::write(&script_file, &script).map_err(|e| format!("Failed to write launch script: {e}"))?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script_file, fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("Failed to set script permissions: {e}"))?;

    let terminals = [
        ("gnome-terminal", vec!["--".to_string()]),
        ("konsole", vec!["-e".to_string()]),
        ("xfce4-terminal", vec!["-e".to_string()]),
        ("x-terminal-emulator", vec!["-e".to_string()]),
        ("alacritty", vec!["-e".to_string()]),
        ("kitty", vec!["-e".to_string()]),
    ];

    let mut last_error = String::from("No usable terminal found");
    for (terminal, args) in terminals {
        let mut command = Command::new(terminal);
        command.args(&args);
        command.arg("sh");
        command.arg(&script_file);
        match command.spawn() {
            Ok(_) => return Ok(()),
            Err(e) => last_error = format!("Failed to launch {terminal}: {e}"),
        }
    }

    let _ = fs::remove_file(&script_file);
    if let Some(path) = settings_file {
        let _ = fs::remove_file(path);
    }
    Err(last_error)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    use super::write_local_temp_settings_file;
    use super::{
        build_minimal_temp_settings, format_claude_command_line, is_windows_batch_script_path,
        resolve_bash_claude_command, sanitize_provider_id_for_filename,
        unix_export_claude_config_dir_line, windows_path_to_msys_path, windows_terminal_start_args,
    };

    #[test]
    fn minimal_settings_keeps_non_empty_env_only() {
        let settings = build_minimal_temp_settings(
            r#"{
              "env": {
                "ANTHROPIC_BASE_URL": " https://example.com ",
                "ANTHROPIC_AUTH_TOKEN": "sk-test",
                "EMPTY": "  ",
                "NUMBER_AS_STRING": "1"
              },
              "model": "should-not-copy"
            }"#,
        )
        .unwrap();

        let env = settings.get("env").unwrap().as_object().unwrap();
        assert_eq!(
            env.get("ANTHROPIC_BASE_URL").unwrap().as_str().unwrap(),
            "https://example.com"
        );
        assert_eq!(
            env.get("ANTHROPIC_AUTH_TOKEN").unwrap().as_str().unwrap(),
            "sk-test"
        );
        assert!(!env.contains_key("EMPTY"));
        assert!(!settings.as_object().unwrap().contains_key("model"));
    }

    #[test]
    fn command_line_includes_full_access_flag() {
        let line = format_claude_command_line("claude", Some(r"C:\temp\a.json"), true);
        assert!(line.contains("--settings"));
        assert!(line.contains("--dangerously-skip-permissions"));
        assert!(line.starts_with("claude ") || line.starts_with("'claude' "));

        let unix_line =
            super::format_claude_command_line_for_unix("claude", Some("/tmp/a.json"), true);
        assert!(unix_line.contains("--dangerously-skip-permissions"));
        assert!(unix_line.contains("'/tmp/a.json'") || unix_line.contains("/tmp/a.json"));
    }

    #[test]
    fn sanitize_provider_id() {
        assert_eq!(
            sanitize_provider_id_for_filename("my/provider:1"),
            "my_provider_1"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn launch_cwd_preserves_disk_and_unc_prefixes() {
        for (input, expected) in [
            (r"\\?\C:\Users\tester\project", r"C:\Users\tester\project"),
            (r"\\?\D:\GitHub\ai-toolbox", r"D:\GitHub\ai-toolbox"),
            (r"\\?\UNC\server\share\project", r"\\server\share\project"),
            (r"D:\project", r"D:\project"),
        ] {
            assert_eq!(
                super::strip_verbatim_prefix(std::path::Path::new(input)),
                std::path::PathBuf::from(expected)
            );
        }

        let directory = tempfile::tempdir().unwrap();
        let resolved = super::resolve_launch_cwd(Some(directory.path().display().to_string()))
            .unwrap()
            .unwrap();
        assert!(resolved.is_absolute());
        assert_eq!(
            resolved.canonicalize().unwrap(),
            directory.path().canonicalize().unwrap()
        );
        assert!(!resolved.to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn unix_config_dir_export_is_shell_quoted() {
        assert_eq!(
            unix_export_claude_config_dir_line("/tmp/claude root"),
            "export CLAUDE_CONFIG_DIR='/tmp/claude root'"
        );
    }

    #[test]
    fn unix_trap_cleanup_quotes_nested_shell_paths() {
        let line = super::unix_trap_cleanup_line(&["/tmp/provider settings.json".to_string()]);

        assert_eq!(
            line,
            r#"trap 'rm -f '"'"'/tmp/provider settings.json'"'"'' EXIT"#
        );
    }

    #[test]
    fn detects_windows_batch_cli_shims() {
        assert!(is_windows_batch_script_path(
            r"C:\Users\tester\AppData\Roaming\npm\claude.cmd"
        ));
        assert!(is_windows_batch_script_path("claude.BAT"));
        assert!(!is_windows_batch_script_path("claude.exe"));
        assert!(!is_windows_batch_script_path("claude"));
    }

    #[test]
    fn windows_paths_convert_to_msys_form() {
        let convert = |value: &str| windows_path_to_msys_path(std::path::Path::new(value));
        assert_eq!(convert(r"C:\Users\t\claude.cmd"), "/c/Users/t/claude.cmd");
        assert_eq!(convert(r"C:\"), "/c");
        assert_eq!(convert(r"C:/a/b"), "/c/a/b");
        assert_eq!(convert(r"\\?\C:\a\b"), "/c/a/b");
        assert_eq!(convert(r"\\srv\share\x"), "//srv/share/x");
        assert_eq!(convert(r"\\?\UNC\srv\share\x"), "//srv/share/x");
        assert_eq!(convert(r"relative\path"), "relative/path");
    }

    #[test]
    fn bash_claude_command_prefers_extensionless_shim() {
        let dir = tempfile::tempdir().unwrap();
        let cmd_shim = dir.path().join("claude.cmd");
        std::fs::write(&cmd_shim, b"noop").unwrap();
        let sh_shim = dir.path().join("claude");
        std::fs::write(&sh_shim, b"noop").unwrap();

        // .cmd shims resolve to their extensionless sh sibling.
        assert_eq!(
            resolve_bash_claude_command(&cmd_shim.to_string_lossy()),
            windows_path_to_msys_path(&sh_shim)
        );

        // Native executables are used as-is.
        let exe = dir.path().join("claude.exe");
        std::fs::write(&exe, b"noop").unwrap();
        assert_eq!(
            resolve_bash_claude_command(&exe.to_string_lossy()),
            windows_path_to_msys_path(&exe)
        );
    }

    #[test]
    fn windows_terminal_args_follow_preference() {
        let (args, name) = windows_terminal_start_args(Some("powershell"), r"C:\t a\b.bat");
        assert_eq!(name, "PowerShell");
        assert_eq!(args[0], "powershell");
        assert!(args.contains(&"-NoExit".to_string()));
        assert_eq!(args.last().unwrap(), "& 'C:\\t a\\b.bat'");

        let (args, name) = windows_terminal_start_args(Some("wt"), r"C:\t\b.bat");
        assert_eq!(name, "Windows Terminal");
        assert_eq!(
            args,
            vec![
                "wt".to_string(),
                "cmd".to_string(),
                "/K".to_string(),
                r"C:\t\b.bat".to_string()
            ]
        );

        // Missing, empty, default and unknown preferences all fall back to cmd.
        for preference in [None, Some(""), Some("   "), Some("cmd"), Some("hyper")] {
            let (args, name) = windows_terminal_start_args(preference, r"C:\t\b.bat");
            assert_eq!(name, "cmd");
            assert_eq!(
                args,
                vec![
                    "cmd".to_string(),
                    "/K".to_string(),
                    r"C:\t\b.bat".to_string()
                ]
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_batch_cli_shims_use_call() {
        let line = super::format_claude_batch_invocation_line(
            r"C:\Users\tester\AppData\Roaming\npm\claude.cmd",
            Some(r"C:\Temp\settings.json"),
            false,
        );

        assert!(line.starts_with("call "));
        assert!(line.contains("claude.cmd"));
        assert!(line.contains("--settings"));
    }

    #[cfg(unix)]
    #[test]
    fn local_temp_settings_file_is_private_on_unix() {
        let settings =
            build_minimal_temp_settings(r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"secret"}}"#).unwrap();
        let path = write_local_temp_settings_file("test", &settings).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;

        let _ = fs::remove_file(&path);

        assert_eq!(mode, 0o600);
    }
}
