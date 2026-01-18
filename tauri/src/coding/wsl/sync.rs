use std::path::Path;
use std::process::Command;
use super::types::{FileMapping, SyncResult, WSLDetectResult};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Windows CREATE_NO_WINDOW flag to prevent console window from appearing
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Create a WSL command with proper flags for Windows GUI apps
/// This prevents console windows from flashing when running in release mode
fn create_wsl_command() -> Command {
    let mut cmd = Command::new("wsl");
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Detect if WSL is available and get list of distros
pub fn detect_wsl() -> WSLDetectResult {
    // Check if WSL is installed by running wsl --status
    let output = create_wsl_command()
        .args(["--status"])
        .output();

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

    // wsl --list outputs UTF-16 LE on Windows
    let stdout = if output.stdout.len() >= 2 && output.stdout[1] == 0 {
        // UTF-16 LE encoded (BOM or check for null bytes after ASCII)
        let utf16_data: Vec<u16> = output.stdout
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&utf16_data)
    } else {
        // UTF-8 encoded
        String::from_utf8_lossy(&output.stdout).to_string()
    };

    let distros: Vec<String> = stdout
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();

    Ok(distros)
}

/// Expand environment variables in a path
pub fn expand_env_vars(path: &str) -> Result<String, String> {
    let mut result = path.to_string();

    // Common Windows environment variables
    let vars = [
        ("USERPROFILE", std::env::var("USERPROFILE")),
        ("APPDATA", std::env::var("APPDATA")),
        ("LOCALAPPDATA", std::env::var("LOCALAPPDATA")),
        ("HOME", std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))),
    ];

    for (var, value) in vars {
        if let Ok(val) = value {
            result = result.replace(&format!("%{}%", var), &val);
            result = result.replace(&format!("${}", var), &val);
        }
    }

    Ok(result)
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
        sync_directory(&windows_path, &mapping.wsl_path, distro)
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

/// Sync a single file
pub fn sync_single_file(windows_path: &str, wsl_path: &str, distro: &str) -> Result<Vec<String>, String> {
    let wsl_source_path = windows_to_wsl_path(windows_path)?;

    // Expand ~ in WSL path
    let wsl_target_path = wsl_path.replace("~", "$HOME");

    // Create the WSL command
    let command = format!(
        "mkdir -p \"$(dirname \"{}\")\" && cp -f \"{}\" \"{}\"",
        wsl_target_path, wsl_source_path, wsl_target_path
    );

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to execute WSL command: {}", e))?;

    if output.status.success() {
        Ok(vec![format!("{} -> {}", windows_path, wsl_path)])
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("WSL sync failed: {}", stderr))
    }
}

/// Sync a directory (recursive copy)
pub fn sync_directory(windows_path: &str, wsl_path: &str, distro: &str) -> Result<Vec<String>, String> {
    let wsl_source_path = windows_to_wsl_path(windows_path)?;

    // Expand ~ in WSL path
    let wsl_target_path = wsl_path.replace("~", "$HOME");

    // Create the WSL command to copy directory recursively
    // Use cp -r to copy directory contents, remove target first to ensure clean sync
    let command = format!(
        "rm -rf \"{}\" 2>/dev/null; mkdir -p \"$(dirname \"{}\")\" && cp -r \"{}\" \"{}\"",
        wsl_target_path, wsl_target_path, wsl_source_path, wsl_target_path
    );

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to execute WSL directory command: {}", e))?;

    if output.status.success() {
        Ok(vec![format!("{} -> {}", windows_path, wsl_path)])
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("WSL directory sync failed: {}", stderr))
    }
}

/// Sync files matching a pattern
pub fn sync_pattern_files(windows_pattern: &str, wsl_target_dir: &str, distro: &str) -> Result<Vec<String>, String> {
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
        wsl_source_base, pattern,
        wsl_source_base, pattern,
        wsl_target_dir_expanded,
        wsl_source_base, pattern,
        wsl_target_dir_expanded
    );

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to execute WSL pattern command: {}", e))?;

    if output.status.success() {
        Ok(vec![format!("{} -> {}", windows_pattern, wsl_target_dir)])
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Pattern sync failures are often OK (just no files matching)
        if stderr.contains("cannot stat") || stderr.contains("No such file") {
            Ok(vec![])
        } else {
            Err(format!("WSL pattern sync failed: {}", stderr))
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
    }
}
