//! Local Paseo config read/patch/write plus best-effort daemon reload.
//!
//! Paseo's default home is `~/.paseo`; `PASEO_HOME` overrides it. The config
//! file is tiny, so the whole JSON document is patched atomically. A running
//! daemon does not watch the file, so `paseo daemon reload` is attempted after
//! every successful write.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::coding::paseo::config::patch_paseo_config_text;
use crate::coding::paseo::models::build_models_for_provider;
use crate::coding::paseo::PaseoProvider;

const PASEO_RELOAD_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaseoLocalSyncOutcome {
    /// The config was rewritten and the daemon reload was attempted.
    Synced,
    /// `~/.paseo/config.json` does not exist; the target is not a Paseo host.
    ConfigMissing,
    /// The applied provider has no model list to mirror.
    NoModels,
}

pub fn local_paseo_home_path() -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var("PASEO_HOME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let expanded = crate::coding::expand_local_path(trimmed)?;
            return Ok(PathBuf::from(expanded));
        }
    }
    let home = dirs::home_dir().ok_or("Failed to resolve the user home directory")?;
    Ok(home.join(".paseo"))
}

pub fn local_paseo_config_path() -> Result<PathBuf, String> {
    Ok(local_paseo_home_path()?.join("config.json"))
}

/// Mirror the applied provider's model list into the local Paseo config.
///
/// Returns `ConfigMissing` when the host has no Paseo home, so callers can tell
/// "nothing to do" apart from a real write failure.
pub async fn sync_local_models(
    provider: PaseoProvider,
    settings_config: &str,
) -> Result<PaseoLocalSyncOutcome, String> {
    let models = build_models_for_provider(provider, settings_config)?;
    if models.is_empty() {
        return Ok(PaseoLocalSyncOutcome::NoModels);
    }

    let config_path = local_paseo_config_path()?;
    if !config_path.exists() {
        return Ok(PaseoLocalSyncOutcome::ConfigMissing);
    }

    let existing = crate::coding::file_io::read_text_file_with_timeout(
        config_path.clone(),
        "Paseo config",
    )
    .await?;
    let updated = patch_paseo_config_text(&existing, provider, &models)?;

    tauri::async_runtime::spawn_blocking(move || write_text_atomic(&config_path, &updated))
        .await
        .map_err(|error| format!("Failed to join Paseo config write task: {error}"))??;

    if let Err(error) = reload_local_paseo_daemon().await {
        // The file is already correct; a failed reload only means the running
        // daemon keeps the previous list until its next reload/restart.
        log::warn!("Paseo daemon reload after model sync failed: {error}");
    }

    Ok(PaseoLocalSyncOutcome::Synced)
}

fn write_text_atomic(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Paseo config path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create Paseo home {}: {error}", parent.display()))?;

    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("Failed to create a temporary Paseo config: {error}"))?;
    temporary
        .write_all(content.as_bytes())
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| format!("Failed to write the temporary Paseo config: {error}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::metadata(path)
            .map(|metadata| metadata.permissions())
            .unwrap_or_else(|_| std::fs::Permissions::from_mode(0o600));
        let _ = temporary.as_file().set_permissions(permissions);
    }

    temporary.persist(path).map_err(|error| {
        format!(
            "Failed to save the Paseo config {}: {error}",
            path.display()
        )
    })?;
    Ok(())
}

async fn reload_local_paseo_daemon() -> Result<(), String> {
    let program = resolve_local_paseo_cli().ok_or_else(|| {
        "Paseo CLI not found; restart the Paseo daemon manually to apply the config".to_string()
    })?;
    let home = local_paseo_home_path()?;
    let mut command = build_paseo_reload_command(&program, &home);

    let output = tokio::time::timeout(PASEO_RELOAD_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            format!(
                "Paseo daemon reload timed out after {}s",
                PASEO_RELOAD_TIMEOUT.as_secs()
            )
        })?
        .map_err(|error| format!("Failed to run the Paseo daemon reload: {error}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else {
        stdout.trim()
    };
    Err(format!("Paseo daemon reload failed: {detail}"))
}

fn resolve_local_paseo_cli() -> Option<PathBuf> {
    if let Some(program) = crate::coding::cli_resolver::resolve_local_cli_by_name("paseo") {
        return Some(program.path);
    }

    #[cfg(target_os = "windows")]
    if let Some(local_data_dir) = dirs::data_local_dir() {
        // The Windows desktop app bundles the CLI behind this launcher; it is
        // not on PATH and is the only way to reload its managed daemon.
        let candidate = local_data_dir
            .join("Programs")
            .join("Paseo")
            .join("resources")
            .join("bin")
            .join("paseo.cmd");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    #[cfg(target_os = "macos")]
    {
        let candidate =
            PathBuf::from("/Applications/Paseo.app/Contents/Resources/bin/paseo");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn build_paseo_reload_command(program: &Path, home: &Path) -> tokio::process::Command {
    // `.cmd` launchers must go through cmd.exe.
    let mut command = tokio::process::Command::new("cmd");
    command
        .arg("/C")
        .arg(program)
        .arg("daemon")
        .arg("reload")
        .arg("--home")
        .arg(home)
        .arg("--json");
    command.creation_flags(0x08000000);
    command.kill_on_drop(true);
    command
}

#[cfg(not(target_os = "windows"))]
fn build_paseo_reload_command(program: &Path, home: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command
        .arg("daemon")
        .arg("reload")
        .arg("--home")
        .arg(home)
        .arg("--json");
    command.kill_on_drop(true);
    command
}
