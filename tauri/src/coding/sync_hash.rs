//! Change detection for directory file mappings.
//!
//! The hash is intentionally metadata-based (relative path, size and mtime,
//! plus symlink target) instead of reading every file's content. Directory
//! mappings can contain hundreds of megabytes of binaries; reading all of them
//! just to decide whether to upload would defeat the purpose. A content change
//! that preserves both size and mtime is the only case this can miss, which is
//! acceptable for config/plugin directories.

use std::collections::HashSet;
use std::path::Path;
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};
use walkdir::{DirEntry, WalkDir};

pub const SYNC_HASH_MARKER_FILE: &str = ".ai-toolbox-synced-hash";

/// Marker path inside a synced directory.
pub fn marker_path(target: &str) -> String {
    format!("{}/{}", target.trim_end_matches('/'), SYNC_HASH_MARKER_FILE)
}

/// Build the hash seed so changing the remote target or the exclude list
/// invalidates an old marker even when the local content is unchanged.
pub fn hash_seed(target: &str, excluded_names: &[String]) -> String {
    format!("{}\n{}", target, excluded_names.join("\n"))
}

/// Hash a directory tree.
///
/// * `excluded_names` matches directory names at any depth, mirroring the
///   existing directory-sync exclude semantics.
/// * `follow_symlinks` must match the transfer implementation: SSH SFTP
///   follows links, while WSL `tar` preserves them.
pub fn hash_directory(
    path: &Path,
    excluded_names: &[String],
    follow_symlinks: bool,
    seed: &str,
) -> Result<String, String> {
    let excluded: HashSet<&str> = excluded_names.iter().map(String::as_str).collect();
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());

    for entry in WalkDir::new(path)
        .follow_links(follow_symlinks)
        // Directory iteration order is filesystem-dependent; sort so an
        // unchanged tree always produces the same hash.
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| !is_excluded_directory(entry, &excluded, follow_symlinks))
    {
        let entry = entry.map_err(|error| format!("遍历目录失败 {}: {}", path.display(), error))?;
        if entry.depth() == 0 {
            continue;
        }

        let relative = entry
            .path()
            .strip_prefix(path)
            .map_err(|error| format!("计算相对路径失败 {}: {}", entry.path().display(), error))?
            .to_string_lossy()
            .replace('\\', "/");
        // Length-prefix variable-length values so adjacent entries cannot
        // collide by concatenation.
        hasher.update((relative.len() as u64).to_le_bytes());
        hasher.update(relative.as_bytes());

        // Record the link target even when the transfer follows links: a symlink
        // repointed to a file with the same size/mtime must still invalidate the
        // hash. For !follow_symlinks this is the only content we see.
        // `file_type()` reports the followed target when `follow_links` is on,
        // so use `path_is_symlink()` to detect the link itself.
        if entry.path_is_symlink() {
            hasher.update(b"s");
            if let Ok(link_target) = std::fs::read_link(entry.path()) {
                let target = link_target.to_string_lossy();
                hasher.update((target.len() as u64).to_le_bytes());
                hasher.update(target.as_bytes());
            }
        }

        let metadata = if follow_symlinks {
            std::fs::metadata(entry.path())
        } else {
            std::fs::symlink_metadata(entry.path())
        }
        .map_err(|error| format!("读取元数据失败 {}: {}", entry.path().display(), error))?;

        if metadata.is_dir() {
            hasher.update(b"d");
        } else if metadata.is_file() {
            hasher.update(b"f");
            hasher.update(metadata.len().to_le_bytes());
            if let Ok(modified) = metadata.modified() {
                if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                    hasher.update(duration.as_secs().to_le_bytes());
                    hasher.update(duration.subsec_nanos().to_le_bytes());
                }
            }
        } else if metadata.file_type().is_symlink() {
            hasher.update(b"l");
        }
    }

    Ok(hex::encode(hasher.finalize()))
}

fn is_excluded_directory(
    entry: &DirEntry,
    excluded: &HashSet<&str>,
    follow_symlinks: bool,
) -> bool {
    if entry.depth() == 0 {
        return false;
    }
    if !excluded.contains(entry.file_name().to_string_lossy().as_ref()) {
        return false;
    }
    if follow_symlinks {
        std::fs::metadata(entry.path())
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false)
    } else {
        entry.file_type().is_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::{hash_directory, hash_seed, marker_path};

    #[test]
    fn marker_path_appends_the_marker_without_double_slashes() {
        assert_eq!(
            marker_path("~/.codex/plugins/"),
            "~/.codex/plugins/.ai-toolbox-synced-hash"
        );
        assert_eq!(
            marker_path("~/.codex/plugins"),
            "~/.codex/plugins/.ai-toolbox-synced-hash"
        );
    }

    #[test]
    fn hash_ignores_excluded_directories_and_changes_with_content() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir_all(root.join("skill")).unwrap();
        std::fs::write(root.join("skill/SKILL.md"), "first").unwrap();
        std::fs::create_dir_all(root.join("cache")).unwrap();
        std::fs::write(root.join("cache/ignored.bin"), "ignored").unwrap();

        let seed = hash_seed("~/.test", &[]);
        let first = hash_directory(root, &[], false, &seed).unwrap();
        let first_with_exclude =
            hash_directory(root, &["cache".to_string()], false, &seed).unwrap();
        assert_ne!(first, first_with_exclude);

        std::fs::write(root.join("cache/ignored.bin"), "changed").unwrap();
        let after_excluded_change =
            hash_directory(root, &["cache".to_string()], false, &seed).unwrap();
        assert_eq!(first_with_exclude, after_excluded_change);

        std::fs::write(root.join("skill/SKILL.md"), "second").unwrap();
        let after_content_change =
            hash_directory(root, &["cache".to_string()], false, &seed).unwrap();
        assert_ne!(first_with_exclude, after_content_change);
    }

    #[test]
    fn hash_changes_when_seed_changes() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("config.json"), "{}").unwrap();

        let first = hash_directory(directory.path(), &[], false, "remote-a").unwrap();
        let second = hash_directory(directory.path(), &[], false, "remote-b").unwrap();

        assert_ne!(first, second);
    }

    #[cfg(unix)]
    #[test]
    fn hash_records_preserved_symlink_targets() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("real.txt"), "content").unwrap();
        std::os::unix::fs::symlink("real.txt", root.join("link.txt")).unwrap();

        let first = hash_directory(root, &[], false, "seed").unwrap();
        std::fs::remove_file(root.join("link.txt")).unwrap();
        std::os::unix::fs::symlink("other.txt", root.join("link.txt")).unwrap();
        let second = hash_directory(root, &[], false, "seed").unwrap();

        assert_ne!(first, second);
    }

    #[cfg(unix)]
    #[test]
    fn hash_records_followed_symlink_targets_when_metadata_matches() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("a.txt"), "same").unwrap();
        std::fs::write(root.join("b.txt"), "same").unwrap();

        // Equal size and mtime: only the link target string can distinguish
        // these two trees when follow_symlinks is enabled.
        let timestamp =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        for name in ["a.txt", "b.txt"] {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(root.join(name))
                .unwrap();
            file.set_modified(timestamp).unwrap();
        }

        std::os::unix::fs::symlink("a.txt", root.join("link.txt")).unwrap();
        let first = hash_directory(root, &[], true, "seed").unwrap();
        std::fs::remove_file(root.join("link.txt")).unwrap();
        std::os::unix::fs::symlink("b.txt", root.join("link.txt")).unwrap();
        let second = hash_directory(root, &[], true, "seed").unwrap();

        assert_ne!(first, second);
    }
}
