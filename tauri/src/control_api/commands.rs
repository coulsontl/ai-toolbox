//! Tauri IPC commands for the control API settings page.
//!
//! These commands are intentionally separate from the HTTP surface: they are
//! only reachable from the desktop webview, never through the bearer-token
//! listener. None of them returns the bearer token.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tauri::State;

use super::routes::status_payload;
use super::settings::{save_control_api_settings, ControlApiApprovalMode, ControlApiSettings};
use super::{cancel_pending_approvals, start_if_enabled, stop, ControlApiState};

#[tauri::command]
pub fn control_api_get_settings(
    context: State<'_, ControlApiState>,
) -> Result<ControlApiSettings, String> {
    Ok(context.context.settings())
}

#[tauri::command]
pub async fn control_api_save_settings(
    context: State<'_, ControlApiState>,
    input: ControlApiSettings,
) -> Result<Value, String> {
    let saved = save_control_api_settings(&context.context.db, &input)?;
    context.context.update_settings(saved.clone());

    if saved.enabled {
        // Waiting approvals were opened under the previous policy. Read-only
        // must not leave a modal that would still let the caller through.
        if saved.approval_mode == ControlApiApprovalMode::ReadOnly {
            cancel_pending_approvals(&context.context, "approval_mode_read_only");
        }
        start_if_enabled(&context).await?;
    } else {
        stop(&context).await?;
    }
    Ok(status_payload(&context.context))
}

#[tauri::command]
pub fn control_api_status(context: State<'_, ControlApiState>) -> Value {
    let mut payload = status_payload(&context.context);
    payload["pendingApprovals"] = json!(context.context.approval.pending_infos());
    payload
}

#[tauri::command]
pub fn control_api_resolve_approval(
    context: State<'_, ControlApiState>,
    request_id: String,
    approved: bool,
) -> Value {
    let resolved = context.context.approval.resolve(&request_id, approved);
    json!({ "resolved": resolved })
}

#[tauri::command]
pub fn control_api_list_pending_approvals(context: State<'_, ControlApiState>) -> Value {
    json!(context.context.approval.pending_infos())
}

#[tauri::command]
pub async fn control_api_cli_status() -> Result<Value, String> {
    Ok(cli_status())
}

#[tauri::command]
pub async fn control_api_install_cli() -> Result<Value, String> {
    install_cli()?;
    Ok(cli_status())
}

#[tauri::command]
pub async fn control_api_uninstall_cli() -> Result<Value, String> {
    uninstall_cli()?;
    Ok(cli_status())
}

// ---------------------------------------------------------------------------
// `aitoolbox` installation
// ---------------------------------------------------------------------------

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "aitoolbox.exe"
    } else {
        "aitoolbox"
    }
}

fn install_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        dirs::data_local_dir()
            .map(|dir| dir.join("AI Toolbox").join("bin"))
            .ok_or_else(|| "Could not resolve %LOCALAPPDATA%".to_string())
    }
    #[cfg(not(windows))]
    {
        dirs::home_dir()
            .map(|dir| dir.join(".local").join("bin"))
            .ok_or_else(|| "Could not resolve the home directory".to_string())
    }
}

fn install_path() -> Result<PathBuf, String> {
    Ok(install_dir()?.join(binary_name()))
}

/// Looks for a build of `aitoolbox` in the usual places: next to the desktop
/// app, in a repo-local `cli/target/{release,debug}`, or an explicit override.
///
/// A sidecar declared through `bundle.externalBin` is installed next to the
/// main executable on every bundle we ship: the Windows install directory
/// (NSIS/MSI), `AI Toolbox.app/Contents/MacOS/`, `/usr/bin` for the Linux
/// deb/rpm packages, and the AppImage's internal `usr/bin`. The first
/// candidate below is therefore the bundled CLI and needs no environment
/// override. `dir.join("aitoolbox")` covers a sidecar placed in a sibling
/// subdirectory.
fn find_cli_binary() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(explicit) = std::env::var("AI_TOOLBOX_CLI_BINARY") {
        if !explicit.trim().is_empty() {
            candidates.push(PathBuf::from(explicit));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(binary_name()));
            candidates.push(dir.join("aitoolbox").join(binary_name()));
        }
        let mut ancestor = exe.parent();
        for _ in 0..5 {
            let Some(dir) = ancestor else { break };
            for profile in ["release", "debug"] {
                candidates.push(
                    dir.join("cli")
                        .join("target")
                        .join(profile)
                        .join(binary_name()),
                );
            }
            ancestor = dir.parent();
        }
    }
    #[cfg(debug_assertions)]
    {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        if let Some(root) = manifest.parent() {
            for profile in ["release", "debug"] {
                candidates.push(
                    root.join("cli")
                        .join("target")
                        .join(profile)
                        .join(binary_name()),
                );
            }
        }
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn cli_status() -> Value {
    let installed = install_path().ok().filter(|path| path.is_file());
    let source = find_cli_binary();
    let dir_on_path = install_dir()
        .map(|dir| path_contains(&dir))
        .unwrap_or(false);
    json!({
        "installed": installed.is_some(),
        "installPath": installed.map(|path| path.to_string_lossy().to_string()),
        "binarySource": source.map(|path| path.to_string_lossy().to_string()),
        "installDir": install_dir().ok().map(|dir| dir.to_string_lossy().to_string()),
        "onPath": dir_on_path,
        "version": env!("CARGO_PKG_VERSION"),
    })
}

fn install_cli() -> Result<(), String> {
    let source = find_cli_binary().ok_or_else(|| {
        "aitoolbox was not found. Build it first with `cargo build --release --manifest-path cli/Cargo.toml`."
            .to_string()
    })?;
    let dir = install_dir()?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("Failed to create {}: {error}", dir.display()))?;
    let target = dir.join(binary_name());
    std::fs::copy(&source, &target).map_err(|error| {
        format!(
            "Failed to copy {} to {}: {error}",
            source.display(),
            target.display()
        )
    })?;
    ensure_on_path(&dir)?;
    Ok(())
}

/// Copies the bundled/build CLI over an already-installed one.
///
/// Deliberately scoped to users who installed the CLI themselves: when the
/// install target does not exist this is a no-op and never creates files or
/// touches PATH/shell rc. Any failure (including a target locked by a running
/// process) is returned to the caller so it can log and continue; desktop
/// startup must never block or fail on this.
pub fn sync_installed_cli_if_present() -> Result<bool, String> {
    let target = install_path()?;
    if !target.is_file() {
        return Ok(false);
    }
    let Some(source) = find_cli_binary() else {
        return Ok(false);
    };
    sync_cli_file(&source, &target)
}

/// Testable core of [`sync_installed_cli_if_present`]. Returns `true` when the
/// target was replaced.
fn sync_cli_file(source: &Path, target: &Path) -> Result<bool, String> {
    if !target.is_file() || !source.is_file() {
        return Ok(false);
    }
    if paths_refer_to_same_file(source, target) {
        return Ok(false);
    }
    if files_have_same_contents(source, target)? {
        return Ok(false);
    }
    replace_file_atomically(source, target)?;
    Ok(true)
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    let (Ok(left), Ok(right)) = (left.canonicalize(), right.canonicalize()) else {
        return false;
    };
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

/// Compares the length first so the common "same build" case does not read
/// both binaries.
fn files_have_same_contents(source: &Path, target: &Path) -> Result<bool, String> {
    let source_len = std::fs::metadata(source)
        .map_err(|error| format!("Failed to read {}: {error}", source.display()))?
        .len();
    let target_len = std::fs::metadata(target)
        .map_err(|error| format!("Failed to read {}: {error}", target.display()))?
        .len();
    if source_len != target_len {
        return Ok(false);
    }
    let source_bytes = std::fs::read(source)
        .map_err(|error| format!("Failed to read {}: {error}", source.display()))?;
    let target_bytes = std::fs::read(target)
        .map_err(|error| format!("Failed to read {}: {error}", target.display()))?;
    Ok(source_bytes == target_bytes)
}

/// Same-directory temp file + rename, so a failed copy never leaves a
/// half-written CLI behind. Unix installs must stay executable (0o755).
fn replace_file_atomically(source: &Path, target: &Path) -> Result<(), String> {
    let directory = target
        .parent()
        .ok_or_else(|| format!("Cannot resolve the directory of {}", target.display()))?;
    let file_name = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| binary_name().to_string());
    let temp = directory.join(format!(".{file_name}.sync-{}", std::process::id()));

    let replaced: std::io::Result<()> = (|| {
        std::fs::copy(source, &temp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&temp, target)?;
        Ok(())
    })();
    if let Err(error) = replaced {
        let _ = std::fs::remove_file(&temp);
        return Err(format!(
            "Failed to update {} from {}: {error}",
            target.display(),
            source.display()
        ));
    }
    Ok(())
}

fn uninstall_cli() -> Result<(), String> {
    let path = install_path()?;
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|error| format!("Failed to remove {}: {error}", path.display()))?;
    }
    remove_from_path(&install_dir()?)?;
    Ok(())
}

// --- PATH handling ---------------------------------------------------------

#[cfg(windows)]
fn user_path() -> String {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_CURRENT_USER);
    key.open_subkey("Environment")
        .ok()
        .and_then(|env| env.get_value::<String, _>("Path").ok())
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn user_path() -> String {
    std::env::var("PATH").unwrap_or_default()
}

fn path_contains(dir: &Path) -> bool {
    let needle = dir.to_string_lossy().to_string();
    user_path()
        .split(if cfg!(windows) { ';' } else { ':' })
        .any(|entry| entry.trim().trim_matches('"').eq_ignore_ascii_case(&needle))
}

#[cfg(windows)]
fn ensure_on_path(dir: &Path) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ};
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_CURRENT_USER);
    let env = key
        .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
        .map_err(|error| format!("Failed to open the user environment registry key: {error}"))?;
    let raw = env.get_raw_value("Path").ok();
    let (current, vtype) = match raw {
        Some(raw) => (decode_utf16le(&raw.bytes), raw.vtype),
        None => (String::new(), REG_EXPAND_SZ),
    };
    let target = dir.to_string_lossy().to_string();
    if current
        .split(';')
        .any(|entry| entry.trim().trim_matches('"').eq_ignore_ascii_case(&target))
    {
        return Ok(());
    }
    let next = if current.trim().is_empty() {
        target
    } else {
        format!("{};{}", current.trim_end_matches(';'), target)
    };
    let vtype = if vtype == REG_SZ {
        REG_SZ
    } else {
        REG_EXPAND_SZ
    };
    env.set_raw_value(
        "Path",
        &winreg::RegValue {
            bytes: encode_utf16le(&next),
            vtype,
        },
    )
    .map_err(|error| format!("Failed to update the user PATH: {error}"))?;
    Ok(())
}

#[cfg(windows)]
fn remove_from_path(dir: &Path) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ};
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_CURRENT_USER);
    let env = key
        .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
        .map_err(|error| format!("Failed to open the user environment registry key: {error}"))?;
    let Some(raw) = env.get_raw_value("Path").ok() else {
        return Ok(());
    };
    let (current, vtype) = (decode_utf16le(&raw.bytes), raw.vtype);
    let target = dir.to_string_lossy().to_string();
    let entries: Vec<&str> = current
        .split(';')
        .filter(|entry| !entry.trim().trim_matches('"').eq_ignore_ascii_case(&target))
        .collect();
    let next = entries.join(";");
    if next == current {
        return Ok(());
    }
    let vtype = if vtype == REG_SZ {
        REG_SZ
    } else {
        REG_EXPAND_SZ
    };
    env.set_raw_value(
        "Path",
        &winreg::RegValue {
            bytes: encode_utf16le(&next),
            vtype,
        },
    )
    .map_err(|error| format!("Failed to update the user PATH: {error}"))?;
    Ok(())
}

#[cfg(windows)]
fn encode_utf16le(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect()
}

#[cfg(windows)]
fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let mut decoded = String::from_utf16_lossy(&units);
    // Some writers terminate REG_EXPAND_SZ values with a NUL code unit. Left
    // in place it becomes part of the last PATH entry and corrupts it.
    while decoded.ends_with('\0') {
        decoded.pop();
    }
    decoded
}

#[cfg(not(windows))]
fn ensure_on_path(dir: &Path) -> Result<(), String> {
    let marker = unix_path_marker();
    let profile = unix_profile_path()?;
    let existing = std::fs::read_to_string(&profile).unwrap_or_default();
    if existing.contains(&marker) {
        return Ok(());
    }
    let line = format!(
        "\n{}\nexport PATH=\"{}:$PATH\"\n",
        marker,
        dir.to_string_lossy()
    );
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&profile)
        .map_err(|error| format!("Failed to open {}: {error}", profile.display()))?;
    file.write_all(line.as_bytes())
        .map_err(|error| format!("Failed to update {}: {error}", profile.display()))?;
    Ok(())
}

#[cfg(not(windows))]
fn remove_from_path(_dir: &Path) -> Result<(), String> {
    let marker = unix_path_marker();
    let profile = unix_profile_path()?;
    let Ok(existing) = std::fs::read_to_string(&profile) else {
        return Ok(());
    };
    if !existing.contains(&marker) {
        return Ok(());
    }
    let cleaned = strip_unix_path_block(&existing, &marker);
    if cleaned == existing {
        return Ok(());
    }
    std::fs::write(&profile, cleaned)
        .map_err(|error| format!("Failed to update {}: {error}", profile.display()))?;
    Ok(())
}

/// Removes the marker comment and the `export PATH=...` line that follows it.
///
/// The installer appends both lines as one block; removing only the comment
/// would leave a dead export behind and `uninstall` would never fully undo
/// `install`. Kept platform-independent so it is testable on every host.
#[cfg_attr(windows, allow(dead_code))]
fn strip_unix_path_block(existing: &str, marker: &str) -> String {
    let mut cleaned_lines = Vec::new();
    let mut drop_following_export = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.contains(marker.trim()) {
            drop_following_export = true;
            continue;
        }
        if drop_following_export {
            if trimmed.starts_with("export PATH=") {
                drop_following_export = false;
                continue;
            }
            // A blank line between the comment and its export is allowed;
            // anything else means the user edited the block, so keep it.
            if trimmed.is_empty() {
                continue;
            }
            drop_following_export = false;
        }
        cleaned_lines.push(line);
    }
    let mut cleaned = cleaned_lines.join("\n");
    if !cleaned.is_empty() {
        cleaned.push('\n');
    }
    cleaned
}

#[cfg(not(windows))]
fn unix_path_marker() -> String {
    "# AI Toolbox: add ~/.local/bin to PATH".to_string()
}

#[cfg(not(windows))]
fn unix_profile_path() -> Result<PathBuf, String> {
    let home =
        dirs::home_dir().ok_or_else(|| "Could not resolve the home directory".to_string())?;
    Ok(home.join(".profile"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_paths_are_absolute_and_named_after_the_binary() {
        let path = install_path().unwrap();
        assert!(path.is_absolute());
        assert_eq!(path.file_name().unwrap().to_string_lossy(), binary_name());
    }

    #[test]
    fn status_shape_never_contains_a_token() {
        let status = cli_status();
        let text = serde_json::to_string(&status).unwrap();
        assert!(!text.contains("token"));
    }

    #[test]
    fn find_cli_binary_does_not_panic_when_missing() {
        // On a clean checkout `cli/target` may not exist; the function must
        // simply return None rather than erroring.
        let _ = find_cli_binary();
    }

    fn temp_cli_pair(
        source: &[u8],
        target: Option<&[u8]>,
    ) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("aitoolbox-source");
        let target_path = directory.path().join("aitoolbox");
        std::fs::write(&source_path, source).unwrap();
        if let Some(contents) = target {
            std::fs::write(&target_path, contents).unwrap();
        }
        (directory, source_path, target_path)
    }

    #[test]
    fn sync_cli_file_does_nothing_when_the_cli_is_not_installed() {
        let (_directory, source, target) = temp_cli_pair(b"new-build", None);
        assert!(!sync_cli_file(&source, &target).unwrap());
        assert!(
            !target.exists(),
            "the sync must never create the install target"
        );
    }

    #[test]
    fn sync_cli_file_skips_identical_contents() {
        let (_directory, source, target) = temp_cli_pair(b"same-build", Some(b"same-build"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        assert!(!sync_cli_file(&source, &target).unwrap());
        assert_eq!(std::fs::read(&target).unwrap(), b"same-build");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // A replacement would have rewritten the mode to 0o755.
            assert_eq!(
                std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn sync_cli_file_replaces_a_stale_target() {
        let (_directory, source, target) = temp_cli_pair(b"new-build", Some(b"old-build"));
        assert!(sync_cli_file(&source, &target).unwrap());
        assert_eq!(std::fs::read(&target).unwrap(), b"new-build");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
    }

    #[test]
    fn unix_path_block_removes_both_the_marker_and_its_export() {
        let marker = "# AI Toolbox: add ~/.local/bin to PATH";
        let profile = concat!(
            "export EDITOR=vim\n",
            "\n",
            "# AI Toolbox: add ~/.local/bin to PATH\n",
            "export PATH=\"$HOME/.local/bin:$PATH\"\n",
            "export OTHER=1\n",
        );
        let cleaned = strip_unix_path_block(profile, marker);
        assert!(!cleaned.contains(".local/bin"), "cleaned: {cleaned}");
        assert!(cleaned.contains("export EDITOR=vim"));
        assert!(cleaned.contains("export OTHER=1"));
    }

    #[test]
    fn unix_path_block_keeps_unrelated_path_exports() {
        let marker = "# AI Toolbox: add ~/.local/bin to PATH";
        let profile = concat!(
            "# Another tool\n",
            "export PATH=\"$HOME/sdk/bin:$PATH\"\n",
            "# AI Toolbox: add ~/.local/bin to PATH\n",
            "export PATH=\"$HOME/.local/bin:$PATH\"\n",
        );
        let cleaned = strip_unix_path_block(profile, marker);
        assert!(cleaned.contains("$HOME/sdk/bin"), "cleaned: {cleaned}");
        assert!(!cleaned.contains(".local/bin"), "cleaned: {cleaned}");
    }

    #[cfg(windows)]
    #[test]
    fn utf16_path_decoding_drops_a_trailing_nul_terminator() {
        let mut bytes = encode_utf16le("C:\\Tools\\aitoolbox");
        bytes.extend_from_slice(&[0, 0]);
        assert_eq!(decode_utf16le(&bytes), "C:\\Tools\\aitoolbox");
    }
}
