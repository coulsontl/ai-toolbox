//! Launch Claude Code CLI with a temporary provider-specific settings file.
//!
//! Scheme A (aligned with cc-switch): create a minimal temp settings JSON that only
//! contains this provider's `env` fields, then run `claude --settings <temp>`.
//! Does NOT rewrite the user's real settings.json and does NOT change is_applied.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Map, Value};

use crate::coding::cli_resolver::resolve_local_claude_program;
use crate::coding::runtime_location::{RuntimeLocationInfo, RuntimeLocationMode};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

fn escape_windows_batch_value(value: &str) -> String {
    value
        .replace('^', "^^")
        .replace('%', "%%")
        .replace('&', "^&")
        .replace('|', "^|")
        .replace('<', "^<")
        .replace('>', "^>")
        .replace('(', "^(")
        .replace(')', "^)")
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

fn quote_for_batch(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    if !value.contains([' ', '\t', '"', '&', '|', '<', '>', '^', '%']) {
        return value.to_string();
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

fn is_windows_batch_script_path(value: &str) -> bool {
    let normalized = value.trim().trim_matches('"').to_ascii_lowercase();
    normalized.ends_with(".cmd") || normalized.ends_with(".bat")
}

#[cfg(target_os = "windows")]
fn windows_set_claude_config_dir_line(claude_config_dir: &str) -> String {
    format!(
        "set \"CLAUDE_CONFIG_DIR={}\"",
        escape_windows_batch_value(claude_config_dir)
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
pub fn launch_claude_provider_cli_session(
    runtime_location: &RuntimeLocationInfo,
    provider_id: &str,
    settings_config: &str,
    full_access: bool,
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
) -> Result<(), String> {
    let temp_dir = std::env::temp_dir();
    let safe_id = sanitize_provider_id_for_filename(provider_id);
    let pid = std::process::id();

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
        launch_windows_terminal(
            &temp_dir,
            claude_config_dir,
            settings_file.as_deref(),
            settings_path_str.as_deref(),
            &claude_program,
            full_access,
            &safe_id,
            pid,
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
            pid,
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
    let cleanup = unix_rm_command(&cleanup_paths);
    let trap_cleanup = unix_trap_cleanup_line(&cleanup_paths);

    let inner = format!(
        r#"{trap_cleanup}
{config_dir}
echo "Using temporary Claude provider settings (does not change applied config)"
{settings_echo}
{claude_line}
status=$?
{cleanup}
echo ""
echo "Claude exited with code $status. Press Enter to close."
read -r _
"#,
        trap_cleanup = trap_cleanup,
        config_dir = unix_export_claude_config_dir_line(claude_config_dir),
        settings_echo = if include_settings {
            format!("echo {}", shell_single_quote(&linux_settings_path))
        } else {
            "echo \"(no provider env overrides; using current Claude login/settings)\"".to_string()
        },
        claude_line = claude_line,
        cleanup = cleanup,
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
            "@echo off\r\n\
wsl -d {distro} --exec bash {script}\r\n\
del \"%~f0\" >nul 2>&1\r\n",
            distro = distro,
            script = linux_script,
        );
        fs::write(&bat_file, content)
            .map_err(|e| format!("Failed to write batch launcher: {e}"))?;
        run_windows_start_command(&["cmd", "/K", &bat_file.to_string_lossy()], "cmd")?;
        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (distro, claude_config_dir, inner);
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
) -> Result<(), String> {
    let bat_file = temp_dir.join(format!("ai_toolbox_claude_launch_{safe_id}_{pid}.bat"));
    let claude_line =
        format_claude_batch_invocation_line(claude_program, settings_path, full_access);
    let config_dir_line = windows_set_claude_config_dir_line(claude_config_dir);
    let settings_display = settings_path
        .map(|path| escape_windows_batch_value(path))
        .unwrap_or_else(|| "(none)".to_string());
    let delete_settings = settings_file
        .map(|path| {
            format!(
                "del \"{}\" >nul 2>&1\r\n",
                escape_windows_batch_value(&path.to_string_lossy())
            )
        })
        .unwrap_or_default();

    let content = format!(
        "@echo off\r\n\
{config_dir_line}\r\n\
echo Using temporary Claude provider settings (does not change applied config)\r\n\
echo {settings_display}\r\n\
{claude_line}\r\n\
{delete_settings}\
del \"%~f0\" >nul 2>&1\r\n",
        settings_display = settings_display,
        claude_line = claude_line,
        config_dir_line = config_dir_line,
        delete_settings = delete_settings,
    );

    fs::write(&bat_file, content).map_err(|e| format!("Failed to write batch launcher: {e}"))?;
    let bat_path = bat_file.to_string_lossy().to_string();
    run_windows_start_command(&["cmd", "/K", &bat_path], "cmd")
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

    let output = Command::new("cmd")
        .args(&full_args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to launch {terminal_name}: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{terminal_name} launch failed (exit {:?}): {stderr}",
            output.status.code()
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
) -> Result<(), String> {
    let claude_line = format_claude_command_line(claude_program, settings_path, full_access);
    let config_dir_line = unix_export_claude_config_dir_line(claude_config_dir);
    let script_file = std::env::temp_dir().join(format!(
        "ai_toolbox_claude_launch_{}.sh",
        std::process::id()
    ));
    let mut cleanup_paths = Vec::new();
    if let Some(path) = settings_file {
        cleanup_paths.push(path.to_string_lossy().to_string());
    }
    cleanup_paths.push(script_file.to_string_lossy().to_string());
    let trap_cleanup = unix_trap_cleanup_line(&cleanup_paths);
    let script = format!(
        r#"#!/usr/bin/env sh
{trap_cleanup}
{config_dir_line}
echo "Using temporary Claude provider settings (does not change applied config)"
{settings_echo}
{claude_line}
echo ""
echo "Claude exited. Press Enter to close."
read -r _
"#,
        trap_cleanup = trap_cleanup,
        config_dir_line = config_dir_line,
        settings_echo = settings_path
            .map(|path| format!("echo {}", shell_single_quote(path)))
            .unwrap_or_else(|| {
                "echo \"(no provider env overrides; using current Claude login/settings)\""
                    .to_string()
            }),
        claude_line = claude_line,
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
) -> Result<(), String> {
    let claude_line = format_claude_command_line(claude_program, settings_path, full_access);
    let config_dir_line = unix_export_claude_config_dir_line(claude_config_dir);
    let script_file = std::env::temp_dir().join(format!(
        "ai_toolbox_claude_launch_{}.sh",
        std::process::id()
    ));
    let mut cleanup_paths = Vec::new();
    if let Some(path) = settings_file {
        cleanup_paths.push(path.to_string_lossy().to_string());
    }
    cleanup_paths.push(script_file.to_string_lossy().to_string());
    let trap_cleanup = unix_trap_cleanup_line(&cleanup_paths);
    let script = format!(
        r#"#!/usr/bin/env sh
{trap_cleanup}
{config_dir_line}
echo "Using temporary Claude provider settings (does not change applied config)"
{settings_echo}
{claude_line}
echo ""
echo "Claude exited. Press Enter to close."
read -r _
"#,
        trap_cleanup = trap_cleanup,
        config_dir_line = config_dir_line,
        settings_echo = settings_path
            .map(|path| format!("echo {}", shell_single_quote(path)))
            .unwrap_or_else(|| {
                "echo \"(no provider env overrides; using current Claude login/settings)\""
                    .to_string()
            }),
        claude_line = claude_line,
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
        sanitize_provider_id_for_filename, unix_export_claude_config_dir_line,
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
