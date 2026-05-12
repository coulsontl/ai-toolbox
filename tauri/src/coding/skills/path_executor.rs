use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::sync_engine::{ensure_source_dir, sync_dir_for_tool_with_overwrite};
use super::types::{SyncMode, SyncOutcome};
use crate::coding::runtime_location;
use crate::coding::wsl;

fn parse_wsl_target_path(target: &Path) -> Option<runtime_location::WslLocationInfo> {
    target
        .to_str()
        .and_then(runtime_location::parse_wsl_unc_path)
}

pub fn sync_skill_to_target(
    tool_key: &str,
    source: &Path,
    target: &Path,
    overwrite: bool,
    force_copy: bool,
) -> Result<SyncOutcome> {
    ensure_source_dir(source)?;

    if let Some(wsl_target) = parse_wsl_target_path(target) {
        let source_path = source.to_string_lossy().to_string();
        let unc_target_path =
            runtime_location::build_windows_unc_path(&wsl_target.distro, &wsl_target.linux_path);

        if !overwrite && wsl::wsl_path_exists(&wsl_target.distro, &wsl_target.linux_path) {
            anyhow::bail!("target already exists: {:?}", target);
        }

        if overwrite {
            wsl::remove_wsl_path(&wsl_target.distro, &wsl_target.linux_path)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("remove existing WSL target {:?}", target))?;
        }

        wsl::sync_directory(&source_path, &wsl_target.linux_path, &wsl_target.distro)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("sync directory {:?} -> {:?}", source, target))?;

        return Ok(SyncOutcome {
            mode_used: SyncMode::Copy,
            target_path: unc_target_path,
            replaced: overwrite,
        });
    }

    sync_dir_for_tool_with_overwrite(tool_key, source, target, overwrite, force_copy)
}

pub fn remove_skill_target(target_path: &str) -> Result<()> {
    if let Some(wsl_target) = runtime_location::parse_wsl_unc_path(target_path) {
        return wsl::remove_wsl_path(&wsl_target.distro, &wsl_target.linux_path)
            .map_err(anyhow::Error::msg);
    }

    super::sync_engine::remove_path(target_path).map_err(anyhow::Error::msg)
}

pub fn sync_copy_target_path(source: &Path, target_path: &str) -> Result<SyncOutcome> {
    let target = PathBuf::from(target_path);
    sync_skill_to_target("copy", source, &target, true, true)
}

pub fn target_path_changed(previous_target_path: &str, next_target: &Path) -> bool {
    let next_target_path = next_target.to_string_lossy();
    previous_target_path.trim().to_ascii_lowercase() != next_target_path.trim().to_ascii_lowercase()
}
