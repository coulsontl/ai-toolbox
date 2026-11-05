//! Shared terminal launcher for the provider cards' "Launch CLI" action.
//!
//! Callers describe the command, environment, working directory and temp files
//! to clean up; this module owns the platform-specific terminal plumbing
//! (Windows `.bat` + preferred terminal, Git Bash, macOS Terminal.app, Linux
//! terminal emulators and the WSL bridge).
//!
//! The launcher never rewrites the tool's live configuration: everything it
//! needs travels in `args` / `env`, and `cleanup_paths` are removed when the
//! CLI exits.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Everything a launched session needs. `env` is applied before the command
/// runs (e.g. `CLAUDE_CONFIG_DIR` / `CODEX_HOME`); `cleanup_paths` are removed
/// on exit, including when the CLI fails.
#[derive(Debug, Clone)]
pub struct CliLaunchSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    pub cleanup_paths: Vec<PathBuf>,
    /// Shown above the command in the terminal, e.g. which temporary profile is
    /// in use. Keep it one short line.
    pub banner: String,
    /// Provider id sanitized for use in temp file names.
    pub safe_id: String,
    pub preferred_terminal: Option<String>,
}

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
        Ok(Some(strip_verbatim_prefix(&resolved)))
    }

    #[cfg(not(target_os = "windows"))]
    {
        Ok(Some(resolved))
    }
}

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

/// Open the CLI in a local terminal window.
pub fn launch_local(spec: &CliLaunchSpec) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        launch_local_windows(spec)
    }

    #[cfg(target_os = "macos")]
    {
        launch_local_macos(spec)
    }

    #[cfg(target_os = "linux")]
    {
        launch_local_linux(spec)
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = spec;
        Err("CLI launch is not supported on this platform".to_string())
    }
}

/// Open the CLI inside a WSL distro. `spec.program` and `spec.env` values are
/// expected to be Linux-side (the caller resolves the distro's `PATH` itself).
/// `preferred_terminal` only decides which Windows terminal hosts the `wsl`
/// command.
pub fn launch_wsl(distro: &str, spec: &CliLaunchSpec) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        launch_wsl_windows(distro, spec)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (distro, spec);
        Err("WSL CLI launch is only supported on Windows".to_string())
    }
}

// ---------------------------------------------------------------------------
// Shared script helpers
// ---------------------------------------------------------------------------

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

fn unix_export_lines(env: &[(String, String)]) -> String {
    env.iter()
        .map(|(key, value)| format!("export {key}={}", shell_single_quote(value)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Shared launcher script body for sh-compatible sessions (macOS, Linux, the
/// WSL inner script and the Windows Git Bash launcher): set env, cd, run the
/// CLI, clean up temp files, then keep the window open until Enter.
fn build_unix_session_script(
    program_line: &str,
    env_lines: &str,
    cd_line: &str,
    banner: &str,
    cleanup_paths: &[String],
) -> String {
    let banner = format!("echo {}", shell_single_quote(banner));
    format!(
        r#"#!/usr/bin/env sh
{trap_cleanup}
{env_lines}
{cd_line}
{banner}
{program_line}
status=$?
{cleanup}
echo ""
echo "CLI exited with code $status. Press Enter to close."
read -r _
"#,
        trap_cleanup = unix_trap_cleanup_line(cleanup_paths),
        env_lines = env_lines,
        cd_line = cd_line,
        banner = banner,
        program_line = program_line,
        cleanup = unix_rm_command(cleanup_paths),
    )
}

fn unix_cleanup_paths(spec: &CliLaunchSpec, script_path: &Path) -> Vec<String> {
    let mut paths: Vec<String> = spec
        .cleanup_paths
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect();
    paths.push(script_path.to_string_lossy().to_string());
    paths
}

fn unix_command_line(program: &str, args: &[String], quote: fn(&str) -> String) -> String {
    let mut parts = vec![quote(program)];
    parts.extend(args.iter().map(|arg| quote(arg)));
    parts.join(" ")
}

fn unix_cd_line(cwd: Option<&Path>) -> String {
    cwd.map(|path| format!("cd {}", shell_single_quote(&path.to_string_lossy())))
        .unwrap_or_default()
}

fn random_temp_suffix() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
fn quote_for_batch(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    if !value.contains([' ', '\t', '"', '&', '|', '<', '>', '^', '%']) {
        return value.to_string();
    }
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(any(target_os = "windows", test))]
fn is_windows_batch_script_path(value: &str) -> bool {
    let normalized = value.trim().trim_matches('"').to_ascii_lowercase();
    normalized.ends_with(".cmd") || normalized.ends_with(".bat")
}

#[cfg(target_os = "windows")]
fn batch_set_lines(env: &[(String, String)]) -> String {
    env.iter()
        .map(|(key, value)| {
            format!(
                "set \"{}={}\"",
                key,
                escape_windows_batch_value(value)
            )
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

#[cfg(target_os = "windows")]
fn batch_command_line(program: &str, args: &[String]) -> String {
    let line = unix_command_line(program, args, quote_for_batch);
    if is_windows_batch_script_path(program) {
        format!("call {line}")
    } else {
        line
    }
}

#[cfg(target_os = "windows")]
fn launch_local_windows(spec: &CliLaunchSpec) -> Result<(), String> {
    let temp_dir = std::env::temp_dir();
    let pid = std::process::id();
    let bat_file = temp_dir.join(format!("ai_toolbox_cli_{}_{pid}.bat", spec.safe_id));

    let env_lines = batch_set_lines(&spec.env);
    let cd_line = spec
        .cwd
        .as_ref()
        .map(|path| {
            format!(
                "cd /d \"{}\"\r\n",
                escape_windows_batch_value(&path.to_string_lossy())
            )
        })
        .unwrap_or_default();
    let delete_lines = spec
        .cleanup_paths
        .iter()
        .map(|path| {
            format!(
                "del \"{}\" >nul 2>&1\r\n",
                escape_windows_batch_value(&path.to_string_lossy())
            )
        })
        .collect::<String>();
    let command_line = batch_command_line(&spec.program, &spec.args);

    let content = format!(
        "@echo off\r\n\
{env_lines}\r\n\
{cd_line}\
echo {banner}\r\n\
{command_line}\r\n\
{delete_lines}\
del \"%~f0\" >nul 2>&1\r\n",
        env_lines = env_lines,
        cd_line = cd_line,
        banner = escape_windows_batch_value(&spec.banner),
        command_line = command_line,
        delete_lines = delete_lines,
    );

    fs::write(&bat_file, content).map_err(|e| format!("Failed to write batch launcher: {e}"))?;

    let preferred = normalize_preferred_terminal(spec.preferred_terminal.as_deref());
    let result = if preferred == Some("gitbash") {
        launch_git_bash_terminal(&temp_dir, spec, pid)
    } else {
        launch_bat_in_preferred_terminal(&bat_file, spec.preferred_terminal.as_deref())
    };
    if result.is_ok() && preferred == Some("gitbash") {
        // Git Bash never executes the .bat launcher, so its self-delete line
        // never runs; clean it up here instead.
        let _ = fs::remove_file(&bat_file);
    }
    fall_back_to_cmd_on_failure(result, spec.preferred_terminal.as_deref(), &bat_file)
}

#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
fn normalize_preferred_terminal(preferred_terminal: Option<&str>) -> Option<&str> {
    preferred_terminal
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

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

#[cfg(target_os = "windows")]
fn run_windows_start_command(args: &[&str], terminal_name: &str) -> Result<(), String> {
    // `start` treats the first quoted argument as a window title; keep an empty title.
    let mut full_args: Vec<String> = vec!["/C".into(), "start".into(), "".into()];
    for arg in args {
        full_args.push((*arg).to_string());
    }

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

#[cfg(any(target_os = "windows", test))]
fn windows_path_to_msys_path(path: &Path) -> String {
    let win = path.to_string_lossy().replace('/', "\\");
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
        _ => win.replace('\\', "/"),
    }
}

/// Resolve a program for a Git Bash script: npm-style `.cmd`/`.bat` shims
/// cannot be exec'd by bash, so prefer their extensionless sh sibling when it
/// exists, then convert to the MSYS POSIX form.
#[cfg(any(target_os = "windows", test))]
fn resolve_bash_program(program: &str) -> String {
    let path = Path::new(program);
    if is_windows_batch_script_path(program) {
        let sibling = path.with_extension("");
        if sibling.is_file() {
            return windows_path_to_msys_path(&sibling);
        }
    }
    windows_path_to_msys_path(path)
}

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

#[cfg(target_os = "windows")]
fn spawn_mintty_bash(mintty: &Path, title: &str, bash_args: &[String]) -> Result<(), String> {
    Command::new(mintty)
        .arg("-t")
        .arg(title)
        .arg("/usr/bin/bash")
        .env("MSYS_NO_PATHCONV", "1")
        .env("MSYS2_ARG_CONV_EXCL", "*")
        .args(bash_args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to launch Git Bash: {e}"))
}

#[cfg(target_os = "windows")]
fn launch_git_bash_terminal(
    temp_dir: &Path,
    spec: &CliLaunchSpec,
    pid: u32,
) -> Result<(), String> {
    let mintty =
        find_git_bash_mintty().ok_or_else(|| "Git Bash (mintty.exe) not found".to_string())?;

    let program = resolve_bash_program(&spec.program);
    let command_line = unix_command_line(&program, &spec.args, shell_single_quote);
    let env_lines = unix_export_lines(&spec.env);
    let cd_line = spec
        .cwd
        .as_ref()
        .map(|path| {
            format!(
                "cd {}",
                shell_single_quote(&windows_path_to_msys_path(path))
            )
        })
        .unwrap_or_default();

    let script_file = temp_dir.join(format!("ai_toolbox_cli_{}_{pid}.sh", spec.safe_id));
    let cleanup_paths = unix_cleanup_paths(spec, &script_file);
    let script = build_unix_session_script(
        &command_line,
        &env_lines,
        &cd_line,
        &spec.banner,
        &cleanup_paths,
    );
    fs::write(&script_file, script)
        .map_err(|e| format!("Failed to write Git Bash launcher: {e}"))?;

    spawn_mintty_bash(
        &mintty,
        "AI Toolbox CLI",
        &[
            "--login".to_string(),
            windows_path_to_msys_path(&script_file),
        ],
    )
}

#[cfg(target_os = "windows")]
fn launch_wsl_windows(distro: &str, spec: &CliLaunchSpec) -> Result<(), String> {
    let pid = std::process::id();
    let suffix = random_temp_suffix();
    let linux_script = format!("/tmp/ai_toolbox_cli_{}_{pid}_{suffix}.sh", spec.safe_id);

    let command_line = unix_command_line(&spec.program, &spec.args, shell_single_quote);
    let env_lines = unix_export_lines(&spec.env);
    let cd_line = unix_cd_line(spec.cwd.as_deref());
    let mut cleanup_paths: Vec<String> = spec
        .cleanup_paths
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect();
    cleanup_paths.push(linux_script.clone());
    let script = build_unix_session_script(
        &command_line,
        &env_lines,
        &cd_line,
        &spec.banner,
        &cleanup_paths,
    );

    let unc_script = PathBuf::from(format!(
        r"\\wsl.localhost\{}\tmp\ai_toolbox_cli_{}_{pid}_{suffix}.sh",
        distro, spec.safe_id
    ));
    fs::write(&unc_script, &script).map_err(|e| format!("Failed to write WSL launch script: {e}"))?;

    let temp_dir = std::env::temp_dir();
    let bat_file = temp_dir.join(format!("ai_toolbox_cli_wsl_{}_{pid}.bat", spec.safe_id));
    let content = format!(
        "@echo off\r\n\
wsl -d {distro} --exec bash {script}\r\n\
del \"%~f0\" >nul 2>&1\r\n",
        distro = distro,
        script = linux_script,
    );
    fs::write(&bat_file, content).map_err(|e| format!("Failed to write batch launcher: {e}"))?;

    let preferred = normalize_preferred_terminal(spec.preferred_terminal.as_deref());
    let result = if preferred == Some("gitbash") {
        let mintty = find_git_bash_mintty()
            .ok_or_else(|| "Git Bash (mintty.exe) not found".to_string())?;
        let inner = format!("wsl -d {distro} --exec bash {}", shell_single_quote(&linux_script));
        spawn_mintty_bash(
            &mintty,
            "AI Toolbox CLI",
            &["--login".to_string(), "-c".to_string(), inner],
        )
    } else {
        launch_bat_in_preferred_terminal(&bat_file, spec.preferred_terminal.as_deref())
    };
    if result.is_ok() && preferred == Some("gitbash") {
        let _ = fs::remove_file(&bat_file);
    }
    fall_back_to_cmd_on_failure(result, spec.preferred_terminal.as_deref(), &bat_file)
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn launch_local_macos(spec: &CliLaunchSpec) -> Result<(), String> {
    let script_file = std::env::temp_dir().join(format!(
        "ai_toolbox_cli_{}_{}.sh",
        spec.safe_id,
        std::process::id()
    ));
    let command_line = unix_command_line(&spec.program, &spec.args, shell_single_quote);
    let cleanup_paths = unix_cleanup_paths(spec, &script_file);
    let script = build_unix_session_script(
        &command_line,
        &unix_export_lines(&spec.env),
        &unix_cd_line(spec.cwd.as_deref()),
        &spec.banner,
        &cleanup_paths,
    );

    fs::write(&script_file, &script).map_err(|e| format!("Failed to write launch script: {e}"))?;
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script_file, fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to set script permissions: {e}"))?;
    }

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

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn launch_local_linux(spec: &CliLaunchSpec) -> Result<(), String> {
    let script_file = std::env::temp_dir().join(format!(
        "ai_toolbox_cli_{}_{}.sh",
        spec.safe_id,
        std::process::id()
    ));
    let command_line = unix_command_line(&spec.program, &spec.args, shell_single_quote);
    let cleanup_paths = unix_cleanup_paths(spec, &script_file);
    let script = build_unix_session_script(
        &command_line,
        &unix_export_lines(&spec.env),
        &unix_cd_line(spec.cwd.as_deref()),
        &spec.banner,
        &cleanup_paths,
    );

    fs::write(&script_file, &script).map_err(|e| format!("Failed to write launch script: {e}"))?;
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script_file, fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to set script permissions: {e}"))?;
    }

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
    for path in &spec.cleanup_paths {
        let _ = fs::remove_file(path);
    }
    Err(last_error)
}

#[cfg(test)]
mod tests {
    use super::{
        is_windows_batch_script_path, resolve_bash_program, shell_single_quote,
        windows_path_to_msys_path,
    };
    use std::path::Path;

    #[test]
    fn detects_windows_batch_cli_shims() {
        assert!(is_windows_batch_script_path("C:\\x\\codex.cmd"));
        assert!(is_windows_batch_script_path("\"C:\\x\\codex.BAT\""));
        assert!(!is_windows_batch_script_path("C:\\x\\codex.exe"));
    }

    #[test]
    fn windows_paths_convert_to_msys_form() {
        assert_eq!(
            windows_path_to_msys_path(Path::new("C:\\Program Files\\Codex\\codex.exe")),
            "/c/Program Files/Codex/codex.exe"
        );
        assert_eq!(
            windows_path_to_msys_path(Path::new("\\\\server\\share\\codex")),
            "//server/share/codex"
        );
    }

    #[test]
    fn shell_quoting_escapes_single_quotes() {
        assert_eq!(shell_single_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn batch_shims_resolve_to_extensionless_sibling_when_present() {
        // Without a sibling the shim path is still converted to POSIX form.
        let resolved = resolve_bash_program("C:\\missing\\codex.cmd");
        assert_eq!(resolved, "/c/missing/codex.cmd");
    }
}
