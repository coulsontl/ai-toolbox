use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::sync_engine::{
    ensure_source_dir, ensure_source_target_not_overlapping, sync_dir_for_tool_with_overwrite,
};
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

pub fn remove_skill_target_checked(source: &Path, target_path: &str) -> Result<()> {
    if runtime_location::parse_wsl_unc_path(target_path).is_none() {
        let target = PathBuf::from(target_path);
        if !is_direct_link_target(&target) {
            ensure_source_target_not_overlapping(source, &target)?;
        }
    }

    remove_skill_target(target_path)
}

fn is_direct_link_target(target: &Path) -> bool {
    if std::fs::symlink_metadata(target)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
    {
        return true;
    }

    #[cfg(windows)]
    {
        junction::exists(target).unwrap_or(false)
    }

    #[cfg(not(windows))]
    {
        false
    }
}

pub fn sync_copy_target_path(source: &Path, target_path: &str) -> Result<SyncOutcome> {
    let target = PathBuf::from(target_path);
    sync_skill_to_target("copy", source, &target, true, true)
}

pub fn target_path_changed(previous_target_path: &str, next_target: &Path) -> bool {
    let next_target_path = next_target.to_string_lossy();
    previous_target_path.trim().to_ascii_lowercase() != next_target_path.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn checked_remove_deletes_direct_symlink_without_touching_source() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        std::fs::create_dir(&source).expect("create source");
        std::fs::write(source.join("SKILL.md"), "---\nname: valid\n---\n")
            .expect("write source file");
        std::os::unix::fs::symlink(&source, &target).expect("create target symlink");

        remove_skill_target_checked(&source, &target.to_string_lossy())
            .expect("remove direct link");

        assert!(source.exists());
        assert!(source.join("SKILL.md").exists());
        assert!(std::fs::symlink_metadata(&target).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn checked_remove_rejects_parent_symlink_resolving_to_source() {
        let temp = tempfile::tempdir().expect("temp dir");
        let central = temp.path().join("central");
        let runtime_skills = temp.path().join("runtime-skills");
        let source = central.join("drools-rule-dev");
        let target = runtime_skills.join("drools-rule-dev");

        std::fs::create_dir_all(&source).expect("create source");
        std::fs::write(source.join("SKILL.md"), "---\nname: drools-rule-dev\n---\n")
            .expect("write source file");
        std::os::unix::fs::symlink(&central, &runtime_skills)
            .expect("link runtime skills to central repo");

        let result = remove_skill_target_checked(&source, &target.to_string_lossy());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("same path"));
        assert!(source.exists());
        assert_eq!(
            std::fs::read_to_string(source.join("SKILL.md")).expect("source survives"),
            "---\nname: drools-rule-dev\n---\n"
        );
    }
}
