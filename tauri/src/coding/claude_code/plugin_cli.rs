use crate::coding::cli_resolver::{
    build_local_tokio_command, local_cli_missing_hint, resolve_local_claude_program,
};
use crate::coding::runtime_location::{RuntimeLocationInfo, RuntimeLocationMode};
use tokio::process::Command;

struct ClaudePluginCommand {
    command: Command,
    local_program_label: Option<String>,
}

fn build_claude_command(
    runtime_location: &RuntimeLocationInfo,
    args: &[&str],
) -> Result<ClaudePluginCommand, String> {
    match runtime_location.mode {
        RuntimeLocationMode::LocalWindows => {
            let claude_program = resolve_local_claude_program();
            let local_program_label = claude_program.path.display().to_string();
            let mut command = build_local_tokio_command(&claude_program.path);
            command.args(args);
            command.env("CLAUDE_CONFIG_DIR", &runtime_location.host_path);
            Ok(ClaudePluginCommand {
                command,
                local_program_label: Some(local_program_label),
            })
        }
        RuntimeLocationMode::WslDirect => {
            let wsl = runtime_location.wsl.as_ref().ok_or_else(|| {
                "Missing WSL runtime metadata for Claude plugin command".to_string()
            })?;
            let mut command = Command::new("wsl");
            command.args(["-d", &wsl.distro, "--exec", "env"]);
            command.arg(format!("CLAUDE_CONFIG_DIR={}", wsl.linux_path));
            command.arg("claude");
            command.args(args);
            Ok(ClaudePluginCommand {
                command,
                local_program_label: None,
            })
        }
    }
}

fn build_claude_spawn_error(error: &std::io::Error, local_program_label: Option<&str>) -> String {
    let base_message = format!("Failed to run Claude plugin command: {error}");
    if error.kind() == std::io::ErrorKind::NotFound {
        if let Some(label) = local_program_label {
            return format!(
                "{base_message}. attempted_program={label}. {}",
                local_cli_missing_hint("claude")
            );
        }
    }

    base_message
}

pub async fn run_claude_plugin_command(
    runtime_location: &RuntimeLocationInfo,
    args: &[&str],
) -> Result<(), String> {
    let ClaudePluginCommand {
        mut command,
        local_program_label,
    } = build_claude_command(runtime_location, args)?;

    let output = command
        .output()
        .await
        .map_err(|error| build_claude_spawn_error(&error, local_program_label.as_deref()))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr_output = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout_output = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let error_message = if !stderr_output.is_empty() {
        stderr_output
    } else if !stdout_output.is_empty() {
        stdout_output
    } else {
        "Unknown Claude plugin command failure".to_string()
    };

    Err(error_message)
}
