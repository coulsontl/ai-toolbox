//! Codex background app-server awareness.
//!
//! Codex 0.16x runs its TUI on top of a long-lived app-server, and several
//! surfaces keep one alive after their window closes. All of them read the same
//! `~/.codex/config.toml` / `auth.json`, and all of them hold what they read:
//!
//! - `codex-rs/tui/src/daemon_startup.rs`: "File settings and defaults belong to
//!   the daemon until it restarts."
//! - `codex-rs/login/src/auth/manager.rs`: "External modifications to stored
//!   credentials will NOT be observed until `reload()` is called explicitly."
//! - openai/codex#49651 (closed as not planned): editing `config.toml` /
//!   `auth.json` behind a running app-server is not supported; restart it.
//!
//! So every AI Toolbox apply that rewrites those files leaves a running
//! app-server serving the previous provider. Closing the Codex window does not
//! help — the process is still there; only a reboot or a restart does. Issue
//! #443 is that report.
//!
//! Two kinds of holder are detected:
//!
//! 1. The CLI daemon, probed with `codex app-server daemon version` — upstream
//!    implements that as a pure probe (`app-server-daemon/src/lib.rs` returns
//!    before taking the operation lock, never starting or stopping anything).
//! 2. The ChatGPT desktop app, which ships its own `codex app-server`
//!    (Windows: the Store package; macOS: `ChatGPT.app`/`Codex.app`; Linux: the
//!    official `.deb`/`.rpm` package, public preview since 2026-08-11). It has
//!    no CLI restart entry point, so the only way to reload it is to restart
//!    it.
//!
//! The VS Code ChatGPT extension also runs an app-server and is deliberately
//! out of scope: reloading it is the extension host's job.

use std::path::PathBuf;
use std::time::Duration;

use crate::coding::cli_resolver;

/// A single local socket round-trip; a slow answer means the holder is wedged.
const DAEMON_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const DAEMON_RESTART_TIMEOUT: Duration = Duration::from_secs(60);
/// How long to wait for the desktop app to actually exit after `taskkill` /
/// `pkill`. Mirrors zcode-switch's 8s budget: long enough for a browser-based
/// app to tear down, short enough that a wedged process fails visibly.
const DESKTOP_APP_EXIT_TIMEOUT: Duration = Duration::from_secs(8);

/// Windows Store identity of the ChatGPT app (`Get-StartApps` reports the same
/// string). AUMID launches are the only way to start a packaged app by script.
#[cfg(target_os = "windows")]
const DESKTOP_APP_AUMID: &str = "shell:AppsFolder\\OpenAI.Codex_2p2nqsd0c76g0!App";

/// Bundle id shared by `ChatGPT.app` and `Codex.app`.
#[cfg(target_os = "macos")]
const DESKTOP_APP_BUNDLE_ID: &str = "com.openai.codex";

/// Linux launcher installed by the official `.deb`/`.rpm` (`/usr/bin/chatgpt`
/// is a symlink to a shell wrapper next to the real `ChatGPT` binary). Running
/// the wrapper rather than the binary keeps the packaged environment intact.
#[cfg(target_os = "linux")]
const DESKTOP_APP_LAUNCHER: &str = "chatgpt";

/// Which kind of process is holding the Codex config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodexConfigHolder {
    /// The managed CLI daemon (`codex app-server daemon`).
    Daemon,
    /// The ChatGPT desktop app's bundled app-server.
    DesktopApp,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexDaemonStatus {
    /// True when at least one holder is running.
    pub running: bool,
    /// Every holder found, in a stable order (daemon first).
    pub holders: Vec<CodexConfigHolder>,
    /// App-server version reported by the CLI daemon, when it answered.
    pub app_server_version: Option<String>,
}

fn resolve_codex_program() -> Result<PathBuf, String> {
    cli_resolver::resolve_local_cli_by_command_name("codex")
        .map(|program| program.path)
        .ok_or_else(|| cli_resolver::local_cli_missing_hint("codex"))
}

/// Run `codex app-server daemon <args>` and report the process outcome.
async fn run_daemon_subcommand(
    arguments: &[&str],
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let program = resolve_codex_program()?;
    let mut command = cli_resolver::build_local_tokio_command(&program);
    command.arg("app-server").arg("daemon").args(arguments);
    command.kill_on_drop(true);

    match tokio::time::timeout(timeout, command.output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(format!("无法执行 `codex app-server daemon`: {error}")),
        Err(_) => Err(format!(
            "`codex app-server daemon {}` 超时（{}s）",
            arguments.join(" "),
            timeout.as_secs()
        )),
    }
}

fn output_text(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = if stdout.trim().is_empty() {
        stderr.to_string()
    } else {
        stdout.to_string()
    };
    combined.trim().to_string()
}

/// Pull `appServerVersion` out of `daemon version` output. Unparseable output
/// yields `None`; the caller still knows the daemon answered.
fn parse_app_server_version(output: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(output)
        .ok()?
        .get("appServerVersion")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

/// Whether a ChatGPT desktop app process is running.
///
/// Matched by image name only: the app spawns a dozen helper processes
/// (`--type=renderer`, `--type=crashpad-handler`, ...) and the one that matters
/// is the browser process, but any of them means the app is up. Executable
/// paths live under `Program Files\WindowsApps`, which is not readable without
/// elevation, so path-based matching would silently miss it.
#[cfg(target_os = "windows")]
fn desktop_app_running() -> bool {
    use std::process::Command;

    let mut command = Command::new("tasklist");
    command.args(["/FI", "IMAGENAME eq ChatGPT.exe", "/NH", "/FO", "CSV"]);
    cli_resolver::apply_create_no_window(&mut command);
    let Ok(output) = command.output() else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).contains("ChatGPT.exe")
}

/// macOS ships the same app as a bundle (`ChatGPT.app` or `Codex.app`); the
/// browser process is `ChatGPT` either way.
#[cfg(target_os = "macos")]
fn desktop_app_running() -> bool {
    use std::process::Command;

    let Ok(output) = Command::new("pgrep").args(["-x", "ChatGPT"]).output() else {
        return false;
    };
    output.status.success() && !output.stdout.is_empty()
}

/// Linux uses the same image name as macOS: the official package installs
/// `/usr/lib/chatgpt/ChatGPT` and its Electron helpers all run as `ChatGPT`.
#[cfg(target_os = "linux")]
fn desktop_app_running() -> bool {
    use std::process::Command;

    let Ok(output) = Command::new("pgrep").args(["-x", "ChatGPT"]).output() else {
        return false;
    };
    output.status.success() && !output.stdout.is_empty()
}

/// Anything else (BSD, …) has no known desktop app, so the daemon is the only
/// holder.
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn desktop_app_running() -> bool {
    false
}

/// Every process currently holding the Codex config.
#[tauri::command]
pub async fn get_codex_daemon_status() -> Result<CodexDaemonStatus, String> {
    let mut holders = Vec::new();
    let mut app_server_version = None;

    // A missing CLI (or one without the `daemon` subcommand) is not an error:
    // nothing can be holding the config through this path.
    if resolve_codex_program().is_ok() {
        match run_daemon_subcommand(&["version"], DAEMON_PROBE_TIMEOUT).await {
            Ok(output) if output.status.success() => {
                holders.push(CodexConfigHolder::Daemon);
                app_server_version = parse_app_server_version(&output_text(&output));
            }
            // Non-zero means nothing answered on the control socket; timeouts are
            // not proof of absence but are not actionable either.
            Ok(_) | Err(_) => {}
        }
    }

    let desktop_running = tokio::task::spawn_blocking(desktop_app_running)
        .await
        .unwrap_or(false);
    if desktop_running {
        holders.push(CodexConfigHolder::DesktopApp);
    }

    Ok(CodexDaemonStatus {
        running: !holders.is_empty(),
        holders,
        app_server_version,
    })
}

/// Restart whatever is holding the Codex config so it reloads what AI Toolbox
/// just wrote.
///
/// `restart` (not `stop`) for the daemon: the user's next Codex launch must not
/// start from a socket-less state where the CLI falls back to a fresh embedded
/// server. The desktop app has no CLI entry point, so it is terminated and
/// relaunched through the shell.
#[tauri::command]
pub async fn restart_codex_daemon() -> Result<(), String> {
    let status = get_codex_daemon_status().await?;
    if status.holders.is_empty() {
        return Ok(());
    }

    let mut failures = Vec::new();

    if status.holders.contains(&CodexConfigHolder::Daemon) {
        match run_daemon_subcommand(&["restart"], DAEMON_RESTART_TIMEOUT).await {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                let detail = output_text(&output);
                failures.push(if detail.is_empty() {
                    format!("Codex 后台服务重启失败（退出码 {:?}）", output.status.code())
                } else {
                    format!("Codex 后台服务重启失败：{detail}")
                });
            }
            Err(error) => failures.push(error),
        }
    }

    if status.holders.contains(&CodexConfigHolder::DesktopApp) {
        if let Err(error) = restart_desktop_app().await {
            failures.push(error);
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// Restart the ChatGPT desktop app: terminate it, then launch it again through
/// the shell so the new app-server reads the config AI Toolbox just wrote.
///
/// Launching matters as much as closing — leaving the user to find the icon
/// again is the friction this whole prompt exists to remove. Windows Store
/// packages are launched by AUMID (`<PackageFamilyName>!App`); macOS by bundle
/// id, which covers both `ChatGPT.app` and `Codex.app`.
#[cfg(target_os = "windows")]
async fn restart_desktop_app() -> Result<(), String> {
    let kill_output = tokio::task::spawn_blocking(|| {
        let mut command = std::process::Command::new("taskkill");
        command.args(["/IM", "ChatGPT.exe", "/T", "/F"]);
        cli_resolver::apply_create_no_window(&mut command);
        command.output()
    })
    .await
    .map_err(|error| format!("重启 ChatGPT 客户端任务失败: {error}"))?
    .map_err(|error| format!("无法执行 taskkill: {error}"))?;

    // The exit code is NOT the success criterion: with `/T`, `taskkill` returns
    // 128 whenever any single child refuses to die ("The operation attempted is
    // not supported", seen with Store-package helpers) even though it killed
    // every process that matters. Trusting it would skip the relaunch and leave
    // the user with a closed app. Whether the app actually exited is decided by
    // the poll below, which is the only fact that governs the relaunch.
    //
    // That poll is also what keeps the relaunch working at all: `taskkill`
    // returns before the process is gone, and launching into a half-dead app
    // makes the new instance attach to the old one (the package is
    // single-instance) and the stale config survives. Same shape as
    // zcode-switch's `kill_zcode`, which polls instead of trusting the kill
    // exit code.
    if let Err(error) = wait_until_desktop_app_exits(DESKTOP_APP_EXIT_TIMEOUT).await {
        let detail = output_text(&kill_output);
        return Err(if detail.is_empty() {
            error
        } else {
            format!("{error}（taskkill 输出：{detail}）")
        });
    }

    // `explorer.exe` returns before the app is up and its exit code says nothing
    // about the launch, so this only reports that the request was handed off.
    let launch = tokio::task::spawn_blocking(|| {
        let mut command = std::process::Command::new("explorer.exe");
        command.arg(DESKTOP_APP_AUMID);
        cli_resolver::apply_create_no_window(&mut command);
        command.spawn()
    })
    .await
    .map_err(|error| format!("启动 ChatGPT 客户端任务失败: {error}"))?
    .map_err(|error| format!("无法启动 ChatGPT 客户端: {error}"))?;
    drop(launch);

    Ok(())
}

/// Poll until no ChatGPT desktop process is left.
async fn wait_until_desktop_app_exits(timeout: Duration) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let still_running = tokio::task::spawn_blocking(desktop_app_running)
            .await
            .unwrap_or(false);
        if !still_running {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "ChatGPT 客户端在 {}s 内未退出，已中止重启；请手动关闭后再试",
                timeout.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// macOS: terminate, then reopen by bundle id. `open -b` fails when the app is
/// not registered, which is the actionable error here.
#[cfg(target_os = "macos")]
async fn restart_desktop_app() -> Result<(), String> {
    let output = tokio::task::spawn_blocking(|| {
        std::process::Command::new("pkill")
            .args(["-x", "ChatGPT"])
            .output()
    })
    .await
    .map_err(|error| format!("重启 ChatGPT 客户端任务失败: {error}"))?
    .map_err(|error| format!("无法执行 pkill: {error}"))?;

    // `pkill` exits 1 when nothing matched; the holder exited between the probe
    // and this call, which is the outcome the caller wanted.
    if !output.status.success() && output.status.code() != Some(1) {
        let detail = output_text(&output);
        return Err(if detail.is_empty() {
            format!(
                "关闭 ChatGPT 客户端失败（退出码 {:?}）",
                output.status.code()
            )
        } else {
            format!("关闭 ChatGPT 客户端失败：{detail}")
        });
    }

    // Terminating is not the same as gone; reopen only once the old instance
    // has actually exited (see the Windows counterpart).
    wait_until_desktop_app_exits(DESKTOP_APP_EXIT_TIMEOUT).await?;

    let launch = tokio::task::spawn_blocking(|| {
        std::process::Command::new("open")
            .args(["-b", DESKTOP_APP_BUNDLE_ID])
            .status()
    })
    .await
    .map_err(|error| format!("启动 ChatGPT 客户端任务失败: {error}"))?
    .map_err(|error| format!("无法执行 open: {error}"))?;

    if launch.success() {
        Ok(())
    } else {
        Err(format!(
            "重新打开 ChatGPT 客户端失败（退出码 {:?}），请手动启动",
            launch.code()
        ))
    }
}

/// Linux: terminate, then relaunch through `chatgpt`. `setsid` detaches the new
/// instance from our process group so it survives AI Toolbox closing, and the
/// launcher is resolved by name (packaged at `/usr/bin/chatgpt`).
#[cfg(target_os = "linux")]
async fn restart_desktop_app() -> Result<(), String> {
    let output = tokio::task::spawn_blocking(|| {
        std::process::Command::new("pkill")
            .args(["-x", "ChatGPT"])
            .output()
    })
    .await
    .map_err(|error| format!("重启 ChatGPT 客户端任务失败: {error}"))?
    .map_err(|error| format!("无法执行 pkill: {error}"))?;

    // `pkill` exits 1 when nothing matched; the holder exited between the probe
    // and this call, which is the outcome the caller wanted.
    if !output.status.success() && output.status.code() != Some(1) {
        let detail = output_text(&output);
        return Err(if detail.is_empty() {
            format!(
                "关闭 ChatGPT 客户端失败（退出码 {:?}）",
                output.status.code()
            )
        } else {
            format!("关闭 ChatGPT 客户端失败：{detail}")
        });
    }

    wait_until_desktop_app_exits(DESKTOP_APP_EXIT_TIMEOUT).await?;

    let launch = tokio::task::spawn_blocking(|| {
        std::process::Command::new("setsid")
            .arg(DESKTOP_APP_LAUNCHER)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    })
    .await
    .map_err(|error| format!("启动 ChatGPT 客户端任务失败: {error}"))?
    .map_err(|error| format!("无法启动 ChatGPT 客户端: {error}"))?;
    drop(launch);

    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
async fn restart_desktop_app() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_app_server_version_from_lifecycle_output() {
        // Shape comes from `LifecycleOutput` in `app-server-daemon/src/lib.rs`.
        let output = r#"{"status":"running","appServerVersion":"0.164.0","socketPath":"/tmp/s"}"#;
        assert_eq!(parse_app_server_version(output), Some("0.164.0".to_string()));
    }

    #[test]
    fn unparseable_or_missing_field_yields_none() {
        assert_eq!(parse_app_server_version("not json"), None);
        assert_eq!(parse_app_server_version(r#"{"status":"running"}"#), None);
    }

    #[test]
    fn running_flag_follows_the_holder_list() {
        // The UI keys its prompt off `running`; an empty holder list must never
        // prompt, and any holder must always prompt.
        let none = CodexDaemonStatus {
            running: false,
            holders: vec![],
            app_server_version: None,
        };
        assert!(!none.running);
        let desktop_only = CodexDaemonStatus {
            running: true,
            holders: vec![CodexConfigHolder::DesktopApp],
            app_server_version: None,
        };
        assert!(desktop_only.running);
    }
}
