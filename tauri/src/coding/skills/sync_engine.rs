use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::types::{SyncMode, SyncOutcome};

/// Sync directory using hybrid approach (try symlink, fallback to copy)
pub fn sync_dir_hybrid(source: &Path, target: &Path) -> Result<SyncOutcome> {
    ensure_source_dir(source)?;

    if std::fs::symlink_metadata(target).is_ok() {
        if is_same_link(target, source) {
            return Ok(SyncOutcome {
                mode_used: SyncMode::Symlink,
                target_path: target.to_path_buf(),
                replaced: false,
            });
        }
        anyhow::bail!("target already exists: {:?}", target);
    }

    ensure_parent_dir(target)?;

    if try_link_dir(source, target).is_ok() {
        return Ok(SyncOutcome {
            mode_used: SyncMode::Symlink,
            target_path: target.to_path_buf(),
            replaced: false,
        });
    }

    #[cfg(windows)]
    if try_junction(source, target).is_ok() {
        return Ok(SyncOutcome {
            mode_used: SyncMode::Junction,
            target_path: target.to_path_buf(),
            replaced: false,
        });
    }

    copy_dir_recursive(source, target)?;
    Ok(SyncOutcome {
        mode_used: SyncMode::Copy,
        target_path: target.to_path_buf(),
        replaced: false,
    })
}

/// Sync directory with overwrite option
pub fn sync_dir_hybrid_with_overwrite(
    source: &Path,
    target: &Path,
    overwrite: bool,
) -> Result<SyncOutcome> {
    ensure_source_dir(source)?;

    let mut did_replace = false;
    if std::fs::symlink_metadata(target).is_ok() {
        if is_same_link(target, source) {
            return Ok(SyncOutcome {
                mode_used: SyncMode::Symlink,
                target_path: target.to_path_buf(),
                replaced: false,
            });
        }

        if overwrite {
            remove_path_any(target)
                .with_context(|| format!("remove existing target {:?}", target))?;
            did_replace = true;
        } else {
            anyhow::bail!("target already exists: {:?}", target);
        }
    }

    sync_dir_hybrid(source, target).map(|mut out| {
        out.replaced = did_replace;
        out
    })
}

/// Sync directory using copy only with overwrite option
pub fn sync_dir_copy_with_overwrite(
    source: &Path,
    target: &Path,
    overwrite: bool,
) -> Result<SyncOutcome> {
    ensure_source_dir(source)?;

    let mut did_replace = false;
    if std::fs::symlink_metadata(target).is_ok() {
        if overwrite {
            remove_path_any(target)
                .with_context(|| format!("remove existing target {:?}", target))?;
            did_replace = true;
        } else {
            anyhow::bail!("target already exists: {:?}", target);
        }
    }

    ensure_parent_dir(target)?;
    copy_dir_recursive(source, target)?;

    Ok(SyncOutcome {
        mode_used: SyncMode::Copy,
        target_path: target.to_path_buf(),
        replaced: did_replace,
    })
}

/// Sync directory for a specific tool with overwrite option
/// Cursor doesn't support symlinks, so force copy for it
/// Custom tools can also opt-in to force copy via the force_copy parameter
pub fn sync_dir_for_tool_with_overwrite(
    tool_key: &str,
    source: &Path,
    target: &Path,
    overwrite: bool,
    force_copy: bool,
) -> Result<SyncOutcome> {
    // Cursor currently doesn't support symlinks/junctions
    // Custom tools can also force copy mode
    if tool_key.eq_ignore_ascii_case("cursor") || force_copy {
        return sync_dir_copy_with_overwrite(source, target, overwrite);
    }
    sync_dir_hybrid_with_overwrite(source, target, overwrite)
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create dir {:?}", parent))?;
    }
    Ok(())
}

pub(crate) fn ensure_source_dir(source: &Path) -> Result<()> {
    let meta = std::fs::metadata(source)
        .with_context(|| format!("source path is not a resolvable directory: {:?}", source))?;
    if !meta.is_dir() {
        anyhow::bail!("source path is not a directory: {:?}", source);
    }
    Ok(())
}

fn remove_path_any(path: &Path) -> Result<()> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err).with_context(|| format!("stat {:?}", path)),
    };
    let ft = meta.file_type();

    // Handle symlinks and junctions
    if ft.is_symlink() {
        #[cfg(unix)]
        {
            // On Unix (macOS, Linux), always use remove_file for symlinks
            // regardless of what they point to
            std::fs::remove_file(path).with_context(|| format!("remove symlink {:?}", path))?;
        }
        #[cfg(windows)]
        {
            // On Windows, directory junctions need remove_dir, not remove_file
            if path.is_dir() {
                std::fs::remove_dir(path)
                    .with_context(|| format!("remove dir junction {:?}", path))?;
            } else {
                std::fs::remove_file(path).with_context(|| format!("remove symlink {:?}", path))?;
            }
        }
        return Ok(());
    }
    if ft.is_dir() {
        std::fs::remove_dir_all(path).with_context(|| format!("remove dir {:?}", path))?;
        return Ok(());
    }
    std::fs::remove_file(path).with_context(|| format!("remove file {:?}", path))?;
    Ok(())
}

fn is_same_link(link_path: &Path, target: &Path) -> bool {
    if let Ok(existing) = std::fs::read_link(link_path) {
        return existing == target;
    }
    false
}

fn try_link_dir(source: &Path, target: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target)
            .with_context(|| format!("symlink {:?} -> {:?}", target, source))?;
        Ok(())
    }

    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(source, target)
            .with_context(|| format!("symlink {:?} -> {:?}", target, source))?;
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    anyhow::bail!("symlink not supported on this platform")
}

#[cfg(windows)]
fn try_junction(source: &Path, target: &Path) -> Result<()> {
    junction::create(source, target)
        .with_context(|| format!("junction {:?} -> {:?}", target, source))?;
    Ok(())
}

fn should_skip_copy(entry: &walkdir::DirEntry) -> bool {
    entry.file_name() == ".git"
}

/// Copy a skill directory, resolving top-level symlinks.
///
/// Files and directories directly inside `source` (alongside SKILL.md) that are
/// symlinks will be resolved first so the real content is copied into `target`.
/// On Windows, Git stores symlinks as text files containing the target path;
/// this function also handles that case.
/// Symlinks deeper in the tree are left as-is (skipped by `copy_dir_recursive`).
pub fn copy_skill_dir(source: &Path, target: &Path) -> Result<()> {
    std::fs::create_dir_all(target).with_context(|| format!("create dir {:?}", target))?;

    for entry in std::fs::read_dir(source).with_context(|| format!("read dir {:?}", source))? {
        let entry = entry?;
        let name = entry.file_name();

        if name == ".git" {
            continue;
        }

        let entry_path = entry.path();
        let dest = target.join(&name);

        // Resolve symlinks at the top level
        let real_path = match std::fs::symlink_metadata(&entry_path) {
            Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(&entry_path)
                .with_context(|| format!("resolve symlink {:?}", entry_path))?,
            Ok(meta) if meta.is_file() => {
                // On Windows, Git stores symlinks as small text files containing the target path.
                // Check if this might be a git-style symlink (small file with path content).
                if let Some(resolved) = try_resolve_git_symlink(&entry_path, source) {
                    resolved
                } else {
                    entry_path.clone()
                }
            }
            _ => entry_path.clone(),
        };

        let real_meta =
            std::fs::metadata(&real_path).with_context(|| format!("stat {:?}", real_path))?;

        if real_meta.is_dir() {
            copy_dir_recursive(&real_path, &dest)?;
        } else if real_meta.is_file() {
            std::fs::copy(&real_path, &dest)
                .with_context(|| format!("copy file {:?} -> {:?}", real_path, dest))?;
        }
    }

    Ok(())
}

/// Try to resolve a potential Git-style symlink file on Windows.
/// Git stores symlinks as text files containing the relative path when core.symlinks is false.
/// Returns Some(resolved_path) if the file looks like a symlink and target exists,
/// None otherwise.
fn try_resolve_git_symlink(file_path: &Path, _base_dir: &Path) -> Option<PathBuf> {
    // Only check small files (symlink paths are typically short)
    let meta = std::fs::metadata(file_path).ok()?;
    if meta.len() > 512 {
        return None;
    }

    let content = std::fs::read_to_string(file_path).ok()?;
    let content = content.trim();

    // Check if content looks like a relative path
    if content.is_empty() || content.contains('\n') {
        return None;
    }

    // Must start with . or contain path separators, and not contain spaces or special chars
    if !content.starts_with('.') && !content.starts_with('/') {
        return None;
    }

    // Resolve relative to the file's parent directory
    let parent = file_path.parent()?;
    let target = parent.join(content);

    // Canonicalize to get absolute path and verify it exists
    let resolved = std::fs::canonicalize(&target).ok()?;

    // Safety check: make sure we're not escaping to completely unrelated paths
    // The target should be within the same repository (base_dir or its parent tree)
    Some(resolved)
}

/// Recursively copy directory contents
pub fn copy_dir_recursive(source: &Path, target: &Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !should_skip_copy(entry))
    {
        let entry = entry?;
        if should_skip_copy(&entry) {
            continue;
        }
        let relative = entry.path().strip_prefix(source)?;
        let target_path = target.join(relative);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target_path)
                .with_context(|| format!("create dir {:?}", target_path))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target_path)
                .with_context(|| format!("copy file {:?} -> {:?}", entry.path(), target_path))?;
        }
    }
    Ok(())
}

/// Remove path (file, dir, or symlink/junction)
pub fn remove_path(path: &str) -> Result<(), String> {
    let p = Path::new(path);

    // Use symlink_metadata to check if path exists (works for broken symlinks too)
    let meta = match std::fs::symlink_metadata(p) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.to_string()),
    };
    let ft = meta.file_type();

    // Handle symlinks and junctions
    if ft.is_symlink() {
        #[cfg(unix)]
        {
            // On Unix (macOS, Linux), always use remove_file for symlinks
            // regardless of what they point to
            std::fs::remove_file(p).map_err(|err| err.to_string())?;
        }
        #[cfg(windows)]
        {
            // On Windows, directory junctions need remove_dir, not remove_file
            // Check if it's a directory link by checking if the path is a dir
            // (symlink_metadata tells us it's a link, is_dir follows the link)
            if p.is_dir() {
                std::fs::remove_dir(p).map_err(|err| err.to_string())?;
            } else {
                std::fs::remove_file(p).map_err(|err| err.to_string())?;
            }
        }
        return Ok(());
    }

    if ft.is_dir() {
        std::fs::remove_dir_all(p).map_err(|err| err.to_string())?;
        return Ok(());
    }

    std::fs::remove_file(p).map_err(|err| err.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn create_self_symlink(path: &Path) {
        std::os::unix::fs::symlink(path, path).expect("create self symlink");
    }

    #[cfg(unix)]
    #[test]
    fn sync_engine_rejects_self_symlink_source_without_creating_target() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        create_self_symlink(&source);

        let result = sync_dir_hybrid(&source, &target);

        assert!(result.is_err());
        assert!(std::fs::symlink_metadata(&target).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn sync_engine_preserves_existing_target_when_overwrite_source_is_self_symlink() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        create_self_symlink(&source);
        std::fs::create_dir(&target).expect("create target");
        std::fs::write(target.join("keep.txt"), "keep").expect("write target file");

        let result = sync_dir_hybrid_with_overwrite(&source, &target, true);

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(target.join("keep.txt")).expect("read preserved target"),
            "keep"
        );
    }

    #[cfg(unix)]
    #[test]
    fn sync_engine_rejects_broken_symlink_source_for_copy_without_removing_target() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        let missing = temp.path().join("missing");
        let target = temp.path().join("target");
        std::os::unix::fs::symlink(&missing, &source).expect("create broken symlink");
        std::fs::create_dir(&target).expect("create target");
        std::fs::write(target.join("keep.txt"), "keep").expect("write target file");

        let result = sync_dir_copy_with_overwrite(&source, &target, true);

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(target.join("keep.txt")).expect("read preserved target"),
            "keep"
        );
    }

    #[cfg(unix)]
    #[test]
    fn sync_engine_rejects_broken_symlink_target_without_replacing_it() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        let missing = temp.path().join("missing");
        std::fs::create_dir(&source).expect("create source");
        std::fs::write(source.join("SKILL.md"), "---\nname: valid\n---\n")
            .expect("write source file");
        std::os::unix::fs::symlink(&missing, &target).expect("create broken target symlink");

        let result = sync_dir_hybrid(&source, &target);

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_link(&target).expect("target remains symlink"),
            missing
        );
        assert!(std::fs::metadata(&target).is_err());
    }

    #[test]
    fn sync_engine_syncs_valid_source_dir() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        std::fs::create_dir(&source).expect("create source");
        std::fs::write(source.join("SKILL.md"), "---\nname: valid\n---\n")
            .expect("write source file");

        let outcome =
            sync_dir_copy_with_overwrite(&source, &target, false).expect("sync valid dir");

        assert!(matches!(outcome.mode_used, SyncMode::Copy));
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).expect("read copied file"),
            "---\nname: valid\n---\n"
        );
    }
}
