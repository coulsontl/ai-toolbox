use super::types::{normalize_directory_excludes, FileMapping, SyncResult, WSLDetectResult};
use std::io::Write;
use std::path::Path;
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Windows CREATE_NO_WINDOW flag to prevent console window from appearing
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Create a WSL command with proper flags for Windows GUI apps
/// This prevents console windows from flashing when running in release mode
fn create_wsl_command() -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new("wsl");
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

pub fn manage_wsl_skill_target(
    distro: &str,
    source: &str,
    target: &str,
    central_root: &str,
    action: crate::coding::skills::remote_target::RemoteSkillTargetAction,
) -> Result<bool, String> {
    use crate::coding::skills::remote_target::{
        build_remote_skill_target_command, parse_remote_skill_target_result,
    };
    let command = build_remote_skill_target_command(source, target, central_root, action);
    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "sh", "-c", &command])
        .output()
        .map_err(|error| format!("Failed to maintain WSL skill target: {error}"))?;
    if !output.status.success() {
        return Err(decode_wsl_output(&output.stderr).trim().to_string());
    }
    parse_remote_skill_target_result(&decode_wsl_output(&output.stdout))
}

/// Check if bytes look like UTF-16 LE encoding.
///
/// WSL on Windows outputs UTF-16 LE for commands like `wsl --list`.
/// UTF-16 LE for ASCII text shows a pattern: each ASCII byte is followed by 0x00.
/// We check for BOM (FF FE) or sample multiple byte pairs to confirm the pattern.
fn looks_like_utf16_le(bytes: &[u8]) -> bool {
    if bytes.len() < 2 {
        return false;
    }

    // Check for UTF-16 LE BOM
    if bytes[0] == 0xFF && bytes[1] == 0xFE {
        return true;
    }

    // Sample up to 8 byte pairs: for ASCII-heavy content, every odd byte should be 0x00
    let sample_count = (bytes.len() / 2).min(8);
    if sample_count < 2 {
        return false;
    }

    let zero_count = bytes[..sample_count * 2]
        .chunks_exact(2)
        .filter(|pair| pair[0] != 0 && pair[1] == 0)
        .count();

    // If most sampled pairs match the "ASCII + 0x00" pattern, it's UTF-16 LE
    zero_count * 2 >= sample_count
}

/// Decode WSL command output which may be UTF-16 LE (Windows) or UTF-8 (Linux)
/// Also strips null characters to prevent SurrealDB panics
fn decode_wsl_output(bytes: &[u8]) -> String {
    let result = if looks_like_utf16_le(bytes) {
        let utf16_data: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&utf16_data)
    } else {
        String::from_utf8_lossy(bytes).to_string()
    };
    // Strip null characters to prevent SurrealDB panics
    result.replace('\0', "")
}

/// Get the effective distro to use: if configured distro doesn't exist,
/// try to find a matching one or use the first available distro
pub fn get_effective_distro(configured_distro: &str) -> Result<String, String> {
    let distros = get_wsl_distros()?;

    if distros.is_empty() {
        return Err("No WSL distros available".to_string());
    }

    // Check if configured distro exists exactly
    if distros.iter().any(|d| d == configured_distro) {
        return Ok(configured_distro.to_string());
    }

    // Try to find a distro that starts with the configured name (e.g., "Ubuntu" matches "Ubuntu-22.04")
    if let Some(matching) = distros.iter().find(|d| d.starts_with(configured_distro)) {
        log::info!(
            "WSL distro '{}' not found, using '{}' instead",
            configured_distro,
            matching
        );
        return Ok(matching.clone());
    }

    // Try to find a distro where configured name starts with it (e.g., "Ubuntu-22.04" matches "Ubuntu")
    if let Some(matching) = distros
        .iter()
        .find(|d| configured_distro.starts_with(d.as_str()))
    {
        log::info!(
            "WSL distro '{}' not found, using '{}' instead",
            configured_distro,
            matching
        );
        return Ok(matching.clone());
    }

    // Fall back to first available distro
    let first = distros.first().unwrap().clone();
    log::warn!(
        "WSL distro '{}' not found, falling back to '{}'",
        configured_distro,
        first
    );
    Ok(first)
}

/// Detect if WSL is available and get list of distros
pub fn detect_wsl() -> WSLDetectResult {
    // Check if WSL is installed by running wsl --status
    let output = create_wsl_command().args(["--status"]).output();

    match output {
        Ok(result) => {
            if result.status.success() {
                // WSL is available, get distros
                match get_wsl_distros() {
                    Ok(distros) => WSLDetectResult {
                        available: true,
                        distros,
                        error: None,
                    },
                    Err(e) => WSLDetectResult {
                        available: true,
                        distros: vec![],
                        error: Some(e),
                    },
                }
            } else {
                WSLDetectResult {
                    available: false,
                    distros: vec![],
                    error: Some("WSL command failed".to_string()),
                }
            }
        }
        Err(e) => WSLDetectResult {
            available: false,
            distros: vec![],
            error: Some(format!("Failed to run WSL command: {}", e)),
        },
    }
}

/// Get list of available WSL distros
pub fn get_wsl_distros() -> Result<Vec<String>, String> {
    let output = create_wsl_command()
        .args(["--list", "--quiet"])
        .output()
        .map_err(|e| format!("Failed to run wsl --list: {}", e))?;

    if !output.status.success() {
        return Err("WSL list command failed".to_string());
    }

    let stdout = decode_wsl_output(&output.stdout);

    let distros: Vec<String> = stdout
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();

    Ok(distros)
}

/// Get running state of a specific WSL distro
/// Returns: "Running", "Stopped", or "Unknown"
pub fn get_wsl_distro_state(distro: &str) -> String {
    let output = match create_wsl_command().args(["--list", "--verbose"]).output() {
        Ok(o) => o,
        Err(_) => return "Unknown".to_string(),
    };

    if !output.status.success() {
        return "Unknown".to_string();
    }

    let stdout = decode_wsl_output(&output.stdout);

    // Parse output to find the distro's state
    // Format: "  NAME                   STATE           VERSION"
    //         "* Ubuntu                 Running         2"
    let mut is_header_skipped = false;
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Skip header line (starts with "NAME")
        if !is_header_skipped {
            let trimmed = line.trim_start();
            if trimmed.starts_with("NAME") {
                is_header_skipped = true;
            }
            continue;
        }

        // Parse line: [*] NAME STATE VERSION
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }

        // Get name and state from the line
        // Format: NAME STATE VERSION or * NAME STATE VERSION
        let (name, state) = if parts.len() >= 4 && parts[0] == "*" {
            // Default distro with * prefix
            (parts[1], parts[2])
        } else {
            // Normal distro
            (parts[0], parts[1])
        };

        if name == distro {
            // Normalize state to "Running" or "Stopped"
            return match state.to_lowercase().as_str() {
                "running" => "Running".to_string(),
                _ => "Stopped".to_string(),
            };
        }
    }

    "Unknown".to_string()
}

/// Expand environment variables in a path
pub fn expand_env_vars(path: &str) -> Result<String, String> {
    super::super::expand_local_path(path)
}

/// Query the real Linux home directory of the WSL distro's default user.
///
/// Used when we need a concrete absolute path that will be embedded as a value
/// inside files (e.g. Claude `known_marketplaces.json` `installLocation`).
/// Read/write helpers like `read_wsl_file` / `write_wsl_file` already expand
/// `~` via `$HOME` in the bash sub-shell, so they don't need this; only
/// in-file string values do, because Claude CLI 2.1.126+ does not expand `~`
/// when validating marketplace paths.
pub fn get_wsl_user_home(distro: &str) -> Result<String, String> {
    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", "echo $HOME"])
        .output()
        .map_err(|e| format!("Failed to query WSL home: {}", e))?;

    if !output.status.success() {
        let stderr = decode_wsl_output(&output.stderr);
        if stderr.contains("WSL_E_DISTRO_NOT_FOUND") || stderr.contains("not found") {
            return Err(format!("WSL distro '{}' not found", distro));
        }
        return Err(format!("WSL command failed: {}", stderr.trim()));
    }

    let home = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if home.is_empty() {
        return Err(format!("WSL distro '{}' returned empty $HOME", distro));
    }
    Ok(home)
}

/// Convert Windows path to WSL path
pub fn windows_to_wsl_path(windows_path: &str) -> Result<String, String> {
    let expanded = expand_env_vars(windows_path)?;

    // Convert C:\Users\... to /mnt/c/Users/...
    let wsl_path = expanded.replace('\\', "/");

    // Convert drive letter (e.g., C: to /mnt/c)
    if wsl_path.len() >= 2 && wsl_path.as_bytes()[1] == b':' {
        let drive = wsl_path.chars().next().unwrap().to_lowercase();
        let rest = &wsl_path[2..];
        // rest starts with "/" (e.g., "/Users/..."), so no extra "/" needed
        return Ok(format!("/mnt/{}{}", drive, rest));
    }

    Ok(wsl_path)
}

/// Sync a single file mapping to WSL
pub fn sync_file_mapping(mapping: &FileMapping, distro: &str) -> Result<Vec<String>, String> {
    let windows_path = expand_env_vars(&mapping.windows_path)?;

    if mapping.is_directory {
        // Directory mode: copy entire directory
        if !Path::new(&windows_path).exists() {
            return Ok(vec![]);
        }
        sync_directory_with_excludes(
            &windows_path,
            &mapping.wsl_path,
            distro,
            &mapping.directory_excludes,
        )
    } else if mapping.is_pattern {
        // Pattern mode: handle wildcards
        sync_pattern_files(&windows_path, &mapping.wsl_path, distro)
    } else {
        // Single file mode
        if !Path::new(&windows_path).exists() {
            return Ok(vec![]);
        }
        sync_single_file(&windows_path, &mapping.wsl_path, distro)
    }
}

/// Resolve a symlinked target to the file it points at.
///
/// `cat > target` and `cp -f src target` write through a symlink; `mv` onto the
/// link would replace the link itself with a regular file and silently detach it
/// from, say, a dotfiles repository. Resolving first keeps that behavior.
///
/// `readlink -m` rather than `-f`: a dotfiles symlink whose target has not been
/// checked out yet is a real case, and `-f` fails there (its failure would then
/// leave the link to be replaced). `-m` canonicalizes even when the target does
/// not exist, which is exactly the file we then create.
const RESOLVE_SYMLINK_TARGET: &str = "if [ -L \"$target\" ]; then \
     resolved=$(readlink -m \"$target\") || resolved=; \
     [ -n \"$resolved\" ] && target=\"$resolved\"; \
     true; \
     fi";

/// Build the shell snippet that replaces `target` atomically.
///
/// The payload arrives on stdin and is written to a sibling temp file first;
/// only a completed write is renamed onto the target. A truncated or failed
/// write therefore never replaces a file the user's CLI still needs to parse,
/// and the mode of an existing target is preserved (mktemp always creates 0600).
fn build_atomic_stdin_write_command(target: &str) -> String {
    format!(
        "set -e; \
         target=\"{}\"; \
         {RESOLVE_SYMLINK_TARGET}; \
         parent=$(dirname \"$target\"); \
         mkdir -p \"$parent\"; \
         tmp=$(mktemp \"$parent/.ai-toolbox-write.XXXXXX\"); \
         trap 'rm -f \"$tmp\"' EXIT; \
         cat > \"$tmp\"; \
         if [ -f \"$target\" ]; then chmod --reference=\"$target\" \"$tmp\" 2>/dev/null || true; fi; \
         mv -f \"$tmp\" \"$target\"; \
         trap - EXIT",
        target
    )
}

/// Build the shell snippet that copies `source` onto `target` atomically.
///
/// Same temp-then-rename shape as [`build_atomic_stdin_write_command`]: a `cp`
/// straight onto the target leaves a half-written file behind when it fails
/// midway.
fn build_atomic_file_copy_command(source: &str, target: &str) -> String {
    format!(
        "set -e; \
         source=\"{}\"; \
         target=\"{}\"; \
         {RESOLVE_SYMLINK_TARGET}; \
         parent=$(dirname \"$target\"); \
         mkdir -p \"$parent\"; \
         tmp=$(mktemp \"$parent/.ai-toolbox-sync.XXXXXX\"); \
         trap 'rm -f \"$tmp\"' EXIT; \
         cp -f \"$source\" \"$tmp\"; \
         if [ -f \"$target\" ]; then chmod --reference=\"$target\" \"$tmp\" 2>/dev/null || true; fi; \
         mv -f \"$tmp\" \"$target\"; \
         trap - EXIT",
        source, target
    )
}

/// Sync a single file
pub fn sync_single_file(
    windows_path: &str,
    wsl_path: &str,
    distro: &str,
) -> Result<Vec<String>, String> {
    let wsl_source_path = windows_to_wsl_path(windows_path)?;

    // Expand ~ in WSL path
    let wsl_target_path = wsl_path.replace("~", "$HOME");
    let command = build_atomic_file_copy_command(&wsl_source_path, &wsl_target_path);

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to execute WSL command: {}", e))?;

    if output.status.success() {
        Ok(vec![format!("{} -> {}", windows_path, wsl_path)])
    } else {
        let stderr = decode_wsl_output(&output.stderr);
        Err(format!("WSL sync failed: {}", stderr.trim()))
    }
}

fn normalize_directory_target_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_directory_copy_command(wsl_source_path: &str, wsl_target_path: &str) -> String {
    format!(
        "set -e; \
         source=\"{}\"; \
         target=\"{}\"; \
         parent=$(dirname \"$target\"); \
         mkdir -p \"$parent\"; \
         tmp=$(mktemp -d \"$parent/.ai-toolbox-sync.XXXXXX\"); \
         trap 'rm -rf \"$tmp\"' EXIT; \
         cp -R -P \"$source\"/. \"$tmp\"/; \
         rm -rf \"$target\"; \
         mv \"$tmp\" \"$target\"; \
         trap - EXIT",
        wsl_source_path, wsl_target_path
    )
}

/// Build a directory copy command that excludes entries whose name matches any
/// of the exclude list, at any nesting depth (recursive), matching the SSH
/// semantic. Excludes are passed as bash positional parameters ($1..$n) and
/// turned into `--exclude` args inside `tar`, so the command text itself needs
/// no escaping; `--exclude=name` matches any path segment named `name`.
fn build_directory_copy_command_with_excludes(
    wsl_source_path: &str,
    wsl_target_path: &str,
) -> String {
    format!(
        "set -e; set -o pipefail; \
         source=\"{}\"; \
         target=\"{}\"; \
         parent=$(dirname \"$target\"); \
         mkdir -p \"$parent\"; \
         tmp=$(mktemp -d \"$parent/.ai-toolbox-sync.XXXXXX\"); \
         trap 'rm -rf \"$tmp\"' EXIT; \
         excl_args=(); \
         for excl in \"$@\"; do excl_args+=(--exclude=\"$excl\"); done; \
         tar -C \"$source\" \"${{excl_args[@]}}\" -cf - . | tar -C \"$tmp\" -xf -; \
         rm -rf \"$target\"; \
         mv \"$tmp\" \"$target\"; \
         trap - EXIT",
        wsl_source_path, wsl_target_path
    )
}

/// Sync a directory (recursive copy) without exclusions.
pub fn sync_directory(
    windows_path: &str,
    wsl_path: &str,
    distro: &str,
) -> Result<Vec<String>, String> {
    sync_directory_with_excludes(windows_path, wsl_path, distro, &[])
}

/// Sync a directory recursively, skipping entries whose path segment name matches
/// `directory_excludes` at any nesting depth.
pub fn sync_directory_with_excludes(
    windows_path: &str,
    wsl_path: &str,
    distro: &str,
    directory_excludes: &[String],
) -> Result<Vec<String>, String> {
    let wsl_source_path = windows_to_wsl_path(windows_path)?;

    // Expand ~ in WSL path
    let wsl_target_path = normalize_directory_target_path(&wsl_path.replace("~", "$HOME"));

    // First, check if source path exists in WSL
    let check_command = format!(
        "if [ -e \"{}\" ]; then echo exists; else echo notfound; fi",
        wsl_source_path
    );
    let check_output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &check_command])
        .output()
        .map_err(|e| format!("Failed to check WSL source path: {}", e))?;

    let check_result = decode_wsl_output(&check_output.stdout).trim().to_string();
    if check_result == "notfound" {
        let source_path_expanded = std::path::Path::new(windows_path);
        if source_path_expanded.exists() {
            return Err(format!(
                "WSL directory sync failed: Windows path '{}' does not exist or is not accessible from WSL. \
                 Converted WSL path: '{}'. Please check if WSL can access Windows drives.",
                windows_path, wsl_source_path
            ));
        } else {
            return Ok(vec![]); // Source doesn't exist, skip sync
        }
    }

    let excludes = normalize_directory_excludes(directory_excludes);

    // Copy into a temporary directory first, then replace the target only after
    // the recursive copy succeeds. Preserve symlinks inside the source instead
    // of dereferencing them; plugin caches can contain stale "latest" links.
    let command = if excludes.is_empty() {
        build_directory_copy_command(&wsl_source_path, &wsl_target_path)
    } else {
        build_directory_copy_command_with_excludes(&wsl_source_path, &wsl_target_path)
    };

    let mut cmd = create_wsl_command();
    cmd.args(["-d", distro, "--exec", "bash", "-c", command.as_str()]);
    if !excludes.is_empty() {
        // `bash -c script arg0 arg1...` maps arg0 to $0 and the rest to $@; the
        // loop in the command reads them via "$@".
        cmd.arg("wsl-sync");
        cmd.args(excludes.iter());
    }
    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute WSL directory command: {}", e))?;

    if output.status.success() {
        Ok(vec![format!("{} -> {}", windows_path, wsl_path)])
    } else {
        let stderr = decode_wsl_output(&output.stderr).trim().to_string();
        let stdout = decode_wsl_output(&output.stdout).trim().to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        // Provide more detailed error information
        if stderr.is_empty() && stdout.is_empty() {
            Err(format!(
                "WSL directory sync failed: Command returned exit code {} but produced no output. \
                 Source: '{}', Target WSL: '{}', WSL converted source: '{}'",
                exit_code, windows_path, wsl_target_path, wsl_source_path
            ))
        } else if !stderr.is_empty() {
            Err(format!(
                "WSL directory sync failed: {}. Source: '{}', Target: '{}', Exit code: {}",
                stderr, windows_path, wsl_path, exit_code
            ))
        } else {
            Err(format!(
                "WSL directory sync failed: {}. Source: '{}', Target: '{}', Exit code: {}",
                stdout, windows_path, wsl_path, exit_code
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_directory_target_path_trims_trailing_slashes() {
        assert_eq!(
            normalize_directory_target_path("$HOME/.codex/plugins/"),
            "$HOME/.codex/plugins"
        );
        assert_eq!(normalize_directory_target_path("/"), "/");
    }

    #[test]
    fn atomic_stdin_write_uses_temp_file_before_replacing_target() {
        let command = build_atomic_stdin_write_command("$HOME/.claude.json");

        assert!(command.contains("tmp=$(mktemp \"$parent/.ai-toolbox-write.XXXXXX\")"));
        assert!(command.contains("cat > \"$tmp\""));
        assert!(command.contains("mv -f \"$tmp\" \"$target\""));
        // The payload must never reach the real target before the rename.
        assert!(!command.contains("cat > \"$target\""));

        let write_index = command.find("cat > \"$tmp\"").expect("write command");
        let replace_index = command.find("mv -f").expect("replace command");
        assert!(write_index < replace_index);
    }

    #[test]
    fn atomic_writes_resolve_symlinked_targets_before_replacing_them() {
        // `cat > link` writes through the link; `mv` onto the link would replace
        // the link with a plain file and detach a dotfiles-managed config.
        for command in [
            build_atomic_stdin_write_command("$HOME/.claude.json"),
            build_atomic_file_copy_command("/tmp/src.json", "$HOME/.claude.json"),
        ] {
            assert!(command.contains("if [ -L \"$target\" ]; then"));
            // `-m` (& not `-f`): a dotfiles link whose target is not checked out
            // yet must still resolve, otherwise the link gets replaced.
            assert!(command.contains("readlink -m \"$target\""));
            assert!(!command.contains("readlink -f"));

            let resolve_index = command.find("readlink -m").expect("resolve");
            let mktemp_index = command.find("mktemp").expect("mktemp");
            assert!(resolve_index < mktemp_index);
        }
    }

    #[test]
    fn single_file_copy_goes_through_a_temp_file() {
        let command = build_atomic_file_copy_command(
            "/mnt/c/Users/Test User/.claude/settings.json",
            "$HOME/.claude/settings.json",
        );

        assert!(command.contains("cp -f \"$source\" \"$tmp\""));
        assert!(command.contains("mv -f \"$tmp\" \"$target\""));
        assert!(!command.contains("cp -f \"$source\" \"$target\""));
        assert!(command.contains("trap 'rm -f \"$tmp\"' EXIT"));
    }

    #[test]
    fn directory_copy_command_uses_temp_dir_before_replacing_target() {
        let command = build_directory_copy_command(
            "/mnt/c/Users/Test User/.codex/plugins",
            "$HOME/.codex/plugins",
        );

        assert!(command.contains("mktemp -d \"$parent/.ai-toolbox-sync.XXXXXX\""));
        assert!(command.contains("cp -R -P \"$source\"/. \"$tmp\"/"));
        assert!(!command.contains("-L"));
        assert!(command.contains("rm -rf \"$target\"; mv \"$tmp\" \"$target\""));

        let copy_index = command.find("cp -R -P").expect("copy command");
        let replace_index = command.find("rm -rf \"$target\"").expect("replace command");
        assert!(copy_index < replace_index);
    }

    #[test]
    fn directory_copy_command_with_excludes_builds_tar_exclude_args() {
        let command = build_directory_copy_command_with_excludes(
            "/mnt/c/Users/Test User/.config/opencode/agents",
            "$HOME/.config/opencode/agents",
        );

        assert!(command.contains("excl_args+=(--exclude=\"$excl\")"));
        assert!(command
            .contains("tar -C \"$source\" \"${excl_args[@]}\" -cf - . | tar -C \"$tmp\" -xf -"));
        assert!(command.contains("rm -rf \"$target\"; mv \"$tmp\" \"$target\""));
    }
}

/// Sync files matching a pattern
pub fn sync_pattern_files(
    windows_pattern: &str,
    wsl_target_dir: &str,
    distro: &str,
) -> Result<Vec<String>, String> {
    // Convert Windows path to WSL path
    let wsl_source_dir = windows_to_wsl_path(windows_pattern)?;

    // Extract the directory and pattern
    let (wsl_source_base, pattern) = if let Some(last_slash) = wsl_source_dir.rfind('/') {
        let base = &wsl_source_dir[..last_slash];
        let pattern = &wsl_source_dir[last_slash + 1..];
        (base, pattern)
    } else {
        (".", &wsl_source_dir[..])
    };

    // Expand ~ in WSL path
    let wsl_target_dir_expanded = wsl_target_dir.replace("~", "$HOME");

    // Create the WSL command to sync pattern files
    let command = format!(
        "mkdir -p \"{}\" && \
         if [ -f \"{}\"/{} ]; then \
             cp -f \"{}\"/{} \"{}\" && \
             echo \"synced\"; \
         else \
             shopt -s nullglob dotglob; \
             files=\"{}\"/{}; \
             if [ -n \"$files\" ]; then \
                 cp -f $files \"{}\" 2>/dev/null && echo \"synced\" || true; \
             fi; \
         fi",
        wsl_target_dir_expanded,
        wsl_source_base,
        pattern,
        wsl_source_base,
        pattern,
        wsl_target_dir_expanded,
        wsl_source_base,
        pattern,
        wsl_target_dir_expanded
    );

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to execute WSL pattern command: {}", e))?;

    if output.status.success() {
        Ok(vec![format!("{} -> {}", windows_pattern, wsl_target_dir)])
    } else {
        let stderr = decode_wsl_output(&output.stderr).trim().to_string();
        let stdout = decode_wsl_output(&output.stdout).trim().to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        // Pattern sync failures are often OK (just no files matching)
        if stderr.contains("cannot stat")
            || stderr.contains("No such file")
            || stderr.contains("No such file or directory")
        {
            Ok(vec![])
        } else if stderr.is_empty() && stdout.is_empty() {
            // Silent failure - might just be no files matching pattern
            Ok(vec![])
        } else if !stderr.is_empty() {
            Err(format!(
                "WSL pattern sync failed: {}. Pattern: '{}', Target: '{}', Exit code: {}",
                stderr, windows_pattern, wsl_target_dir, exit_code
            ))
        } else {
            Err(format!(
                "WSL pattern sync failed: {}. Pattern: '{}', Target: '{}', Exit code: {}",
                stdout, windows_pattern, wsl_target_dir, exit_code
            ))
        }
    }
}

/// Sync all enabled file mappings for a module (or all modules if module is None)
pub fn sync_mappings(
    mappings: &[FileMapping],
    distro: &str,
    module_filter: Option<&str>,
) -> SyncResult {
    let mut synced_files = vec![];
    let mut skipped_files = vec![];
    let mut errors = vec![];

    let filtered_mappings: Vec<_> = mappings
        .iter()
        .filter(|m| m.enabled)
        .filter(|m| module_filter.is_none() || Some(m.module.as_str()) == module_filter)
        .collect();

    for mapping in filtered_mappings {
        match sync_file_mapping(mapping, distro) {
            Ok(files) if files.is_empty() => {
                skipped_files.push(mapping.name.clone());
            }
            Ok(files) => {
                synced_files.extend(files);
            }
            Err(e) => {
                errors.push(format!("{}: {}", mapping.name, e));
            }
        }
    }

    SyncResult {
        success: errors.is_empty(),
        synced_files,
        skipped_files,
        errors,
        warnings: vec![],
    }
}

// ============================================================================
// WSL File Operations
// ============================================================================

/// Check if content looks like valid UTF-8 text config (not binary/corrupted/wrong encoding)
///
/// `read_wsl_file` uses `String::from_utf8_lossy`, which replaces invalid UTF-8 bytes
/// with U+FFFD (�). If the content contains replacement characters, it means the file
/// is not valid UTF-8 (likely GBK/GB2312 on Chinese Windows systems).
pub fn check_file_encoding(content: &str, file_path: &str) -> Result<(), String> {
    if content.contains('\u{FFFD}') {
        let msg = format!(
            "文件 {} 编码不是 UTF-8（可能是 GBK/GB2312），请手动转换后重试。\n\
             修复方法：\n\
             · WSL 中执行:  iconv -f GBK -t UTF-8 \"{}\" -o \"{}.tmp\" && mv \"{}.tmp\" \"{}\"\n\
             · Windows 中: 用 VS Code 打开文件 → 右下角点击编码 → 选择「通过编码重新打开」→ 选 GBK → 再选「通过编码保存」→ 选 UTF-8",
            file_path, file_path, file_path, file_path, file_path
        );
        log::warn!("{}", msg);
        return Err(msg);
    }

    // Check for binary/corrupted content: high ratio of non-printable characters
    let non_printable_count = content
        .chars()
        .take(256)
        .filter(|c| !c.is_ascii_graphic() && !c.is_ascii_whitespace())
        .count();
    let sample_len = content.chars().take(256).count().max(1);
    if non_printable_count * 10 >= sample_len {
        let msg = format!(
            "文件 {} 内容疑似二进制或已损坏，请检查文件内容是否正确",
            file_path
        );
        log::warn!("{}", msg);
        return Err(msg);
    }

    Ok(())
}

/// Read a file from WSL as raw string (no encoding check or conversion).
///
/// Uses `String::from_utf8_lossy` — suitable for files we control (hash files, etc.)
/// where encoding issues are not expected. For user-facing config files that may
/// have encoding problems (GBK, etc.), use `read_wsl_file` instead.
pub fn read_wsl_file_raw(distro: &str, wsl_path: &str) -> Result<String, String> {
    let wsl_target = wsl_path.replace("~", "$HOME");

    let command = format!(
        "if [ -f \"{}\" ]; then cat \"{}\"; else echo ''; fi",
        wsl_target, wsl_target
    );

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to read WSL file: {}", e))?;

    // Check for WSL errors (distro not found, etc.) - these come as UTF-16 on Windows
    if !output.status.success() {
        let stderr = decode_wsl_output(&output.stderr);
        if stderr.contains("WSL_E_DISTRO_NOT_FOUND") || stderr.contains("not found") {
            return Err(format!("WSL distro '{}' not found", distro));
        }
        return Err(format!("WSL command failed: {}", stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Read a file from WSL, with automatic encoding detection and GBK-to-UTF-8 conversion.
///
/// Built on top of `read_wsl_file_raw`, adding encoding validation and auto-conversion.
/// Use this for user-facing config files (claude.json, opencode.json, config.toml, etc.)
///
/// Flow:
/// 1. Read raw content via `read_wsl_file_raw`
/// 2. Validate encoding via `check_file_encoding` — if valid UTF-8, return directly
/// 3. If non-UTF-8 detected, try iconv GBK→UTF-8
/// 4. If iconv succeeds and passes validation, return converted content
/// 5. If iconv fails, return error with instructions for the user
pub fn read_wsl_file(distro: &str, wsl_path: &str) -> Result<String, String> {
    let content = read_wsl_file_raw(distro, wsl_path)?;

    // Already valid UTF-8 — return directly
    if check_file_encoding(&content, wsl_path).is_ok() {
        return Ok(content);
    }

    // Non-UTF-8 detected, try iconv GBK→UTF-8 conversion
    log::warn!(
        "File {} is non-UTF-8, attempting iconv GBK→UTF-8...",
        wsl_path
    );

    let wsl_target = wsl_path.replace("~", "$HOME");
    let convert_command = format!("iconv -f GBK -t UTF-8 \"{}\" 2>/dev/null", wsl_target);

    let convert_output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &convert_command])
        .output()
        .map_err(|e| format!("Failed to run iconv: {}", e))?;

    if !convert_output.status.success() {
        // iconv itself failed (command error)
        return Err(format!(
            "文件 {} 编码不是 UTF-8，自动转换失败。请手动转换后重试。\n\
             修复方法：\n\
             · WSL 中执行:  iconv -f GBK -t UTF-8 \"{}\" -o \"{}.tmp\" && mv \"{}.tmp\" \"{}\"\n\
             · Windows 中: 用 VS Code 打开文件 → 右下角点击编码 → 选择「通过编码重新打开」→ 选 GBK → 再选「通过编码保存」→ 选 UTF-8",
            wsl_path, wsl_path, wsl_path, wsl_path, wsl_path
        ));
    }

    let converted = String::from_utf8_lossy(&convert_output.stdout).to_string();

    // Verify conversion result
    if check_file_encoding(&converted, wsl_path).is_ok() {
        log::info!("Auto-converted {} from GBK to UTF-8", wsl_path);
        return Ok(converted);
    }

    // iconv ran but output is still not valid UTF-8 — encoding is not GBK either
    Err(format!(
        "文件 {} 编码不是 UTF-8 也不是 GBK，自动转换失败。请检查文件编码或内容是否损坏。",
        wsl_path
    ))
}

/// Write content to a WSL file.
///
/// The write goes to a temp file in the target directory and is renamed onto
/// the target only after `cat` completes. Several writers can reach the same
/// WSL config file (this app's sync tasks and the user's CLI inside WSL), and a
/// plain `cat > target` truncates first, so an interrupted or racing write can
/// leave the CLI reading a file that is a prefix plus a tail of another write.
pub fn write_wsl_file(distro: &str, wsl_path: &str, content: &str) -> Result<(), String> {
    let wsl_target = wsl_path.replace("~", "$HOME");
    let command = build_atomic_stdin_write_command(&wsl_target);

    let mut child = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn WSL command: {}", e))?;

    // `write_all` fails with EPIPE whenever the script exits before reading
    // stdin (mkdir/mktemp/readlink failing under `set -e`). The useful reason is
    // on stderr, so a write error must not return before it is collected: the
    // user would only ever see "Broken pipe".
    let write_error = child
        .stdin
        .take()
        .and_then(|mut stdin| stdin.write_all(content.as_bytes()).err());

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to wait for WSL command: {}", e))?;

    if output.status.success() && write_error.is_none() {
        return Ok(());
    }

    let stderr = decode_wsl_output(&output.stderr);
    let reason = if !stderr.trim().is_empty() {
        stderr.trim().to_string()
    } else if let Some(error) = write_error {
        format!("failed to send content: {}", error)
    } else {
        output.status.to_string()
    };

    Err(format!("WSL write failed: {} ({})", reason, wsl_path))
}
/// Remove a file or directory in WSL
pub fn remove_wsl_path(distro: &str, wsl_path: &str) -> Result<(), String> {
    // 安全检查：禁止删除空路径或根路径
    let trimmed = wsl_path.trim();
    if trimmed.is_empty() || trimmed == "/" || trimmed == "~" || trimmed == "$HOME" {
        return Err(format!("拒绝删除危险路径: '{}'", wsl_path));
    }

    let wsl_target = wsl_path.replace("~", "$HOME");
    let command = format!("rm -rf \"{}\"", wsl_target);

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to remove WSL path: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = decode_wsl_output(&output.stderr);
        Err(format!("WSL remove failed: {}", stderr.trim()))
    }
}

/// List subdirectories in a WSL directory
pub fn list_wsl_dir(distro: &str, wsl_path: &str) -> Result<Vec<String>, String> {
    let wsl_target = wsl_path.replace("~", "$HOME");
    let command = format!(
        "if [ -d \"{}\" ]; then ls -1 \"{}\"; fi",
        wsl_target, wsl_target
    );

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to list WSL dir: {}", e))?;

    Ok(decode_wsl_output(&output.stdout)
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

pub fn wsl_path_exists(distro: &str, wsl_path: &str) -> bool {
    let wsl_target = wsl_path.replace("~", "$HOME");
    let command = format!("[ -e \"{}\" ] && echo yes || echo no", wsl_target);

    if let Ok(output) = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
    {
        decode_wsl_output(&output.stdout).trim() == "yes"
    } else {
        false
    }
}
