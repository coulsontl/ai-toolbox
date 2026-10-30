//! WorkBuddy desktop's per-account identity files.
//!
//! The desktop app scopes these files by account, not by config root:
//!
//! ```text
//! ~/.workbuddy/storage/skeleton/account-snapshot.json  → primary.uid
//! ~/.workbuddy/user-<uid>-<enterpriseId|personal>/
//!   IDENTITY.md  USER.md  SOUL.md  BOOTSTRAP.md  workspace-state.json
//! ```
//!
//! The directory name comes straight from the app
//! (`userStorageDirName`: ``user-${uid}-${enterpriseId || 'personal'}``), so it
//! is derived here rather than globbed — a stale directory from a previous
//! account must not be mistaken for the active one.
//!
//! CodeBuddy Code has no equivalent: its user-level memory file is
//! `~/.codebuddy/CODEBUDDY.md`, handled by the CLI module.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// The four identity documents the desktop app seeds and reads.
///
/// `workspace-state.json` is deliberately excluded: it is app-owned
/// bookkeeping (template versions), not a user-editable prompt.
pub const IDENTITY_FILES: [&str; 4] = ["IDENTITY.md", "USER.md", "SOUL.md", "BOOTSTRAP.md"];

/// Returns `true` for a filename this module is allowed to write.
pub fn is_identity_file(file_name: &str) -> bool {
    IDENTITY_FILES.contains(&file_name)
}

/// Reads the signed-in account's uid from the account snapshot.
///
/// Returns `None` before the user has ever signed in, which is the signal the
/// caller uses to disable identity editing with an explanation instead of
/// reporting an error.
pub fn read_active_uid(config_root: &Path) -> Option<String> {
    let path = config_root
        .join("storage")
        .join("skeleton")
        .join("account-snapshot.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    value
        .get("primary")
        .and_then(|primary| primary.get("uid"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|uid| !uid.is_empty())
        .map(str::to_string)
}

/// Reads `primary.enterpriseId`; an enterprise account gets its own directory.
pub fn read_active_enterprise_id(config_root: &Path) -> Option<String> {
    let path = config_root
        .join("storage")
        .join("skeleton")
        .join("account-snapshot.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    value
        .get("primary")
        .and_then(|primary| primary.get("enterpriseId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// Resolves the active identity directory, or `None` when no account is signed in.
///
/// The directory name embeds values read from `account-snapshot.json`, so both
/// are treated as untrusted: anything that could escape the config root (a path
/// separator, `..`, a drive letter) makes the whole capability unavailable
/// rather than resolving to a directory outside it.
pub fn active_identity_dir(config_root: &Path) -> Option<PathBuf> {
    let uid = read_active_uid(config_root)?;
    let scope = read_active_enterprise_id(config_root).unwrap_or_else(|| "personal".to_string());
    if !is_safe_path_component(&uid) || !is_safe_path_component(&scope) {
        return None;
    }
    Some(config_root.join(format!("user-{uid}-{scope}")))
}

/// `true` when `value` can be used verbatim as one directory name under the
/// config root. Rejects separators, `..`, and the Windows drive/stream colons.
fn is_safe_path_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains(['/', '\\', ':', '\0'])
}

/// Reads one identity file; a missing file is `None`, not an error.
pub fn read_identity_file(identity_dir: &Path, file_name: &str) -> Result<Option<String>, String> {
    if !is_identity_file(file_name) {
        return Err(format!("Unsupported identity file: {file_name}"));
    }
    let path = identity_dir.join(file_name);
    if !path.is_file() {
        return Ok(None);
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))
}

/// Writes one identity file atomically.
///
/// The desktop app polls `IDENTITY.md` every 2s, so a partial write would be
/// read back as truncated content.
pub fn write_identity_file(
    identity_dir: &Path,
    file_name: &str,
    content: &str,
) -> Result<(), String> {
    if !is_identity_file(file_name) {
        return Err(format!("Unsupported identity file: {file_name}"));
    }
    let path = identity_dir.join(file_name);
    super::models_json::atomic_write(&path, content.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root_with_snapshot(uid: &str, enterprise: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let skeleton = dir.path().join("storage").join("skeleton");
        std::fs::create_dir_all(&skeleton).expect("skeleton");
        // Build the snapshot through `serde_json` rather than `format!`: a
        // hostile uid containing a backslash would otherwise produce illegal
        // JSON, the parse would fail for the wrong reason, and the case would
        // pass without ever exercising the path-component guard.
        let mut primary = serde_json::Map::new();
        primary.insert("uid".to_string(), serde_json::json!(uid));
        if let Some(enterprise) = enterprise {
            primary.insert("enterpriseId".to_string(), serde_json::json!(enterprise));
        }
        let snapshot = serde_json::json!({ "primary": primary });
        std::fs::write(
            skeleton.join("account-snapshot.json"),
            serde_json::to_string(&snapshot).expect("serialize"),
        )
        .expect("snapshot");
        dir
    }

    #[test]
    fn a_signed_in_account_gets_its_own_scoped_directory() {
        let dir = root_with_snapshot("abc123", None);
        let identity_dir = active_identity_dir(dir.path()).expect("personal scope");
        assert_eq!(identity_dir, dir.path().join("user-abc123-personal"));

        let enterprise = root_with_snapshot("abc123", Some("ent-9"));
        let identity_dir = active_identity_dir(enterprise.path()).expect("enterprise scope");
        assert_eq!(identity_dir, enterprise.path().join("user-abc123-ent-9"));
    }

    #[test]
    fn no_snapshot_means_the_capability_is_unavailable() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(active_identity_dir(dir.path()).is_none());
    }

    /// The uid comes from a file the product writes, so it is treated as
    /// untrusted: a value that would escape the config root disables the whole
    /// capability instead of resolving to a directory outside it.
    #[test]
    fn a_uid_that_could_escape_the_config_root_is_refused() {
        for hostile in ["../../evil", "..", "a/b", "a\\b", "C:evil", ""] {
            let dir = root_with_snapshot(hostile, None);
            assert!(
                active_identity_dir(dir.path()).is_none(),
                "uid {hostile:?} must not resolve to a directory"
            );
        }
    }

    #[test]
    fn identity_files_are_limited_to_the_four_documents() {
        assert!(is_identity_file("IDENTITY.md"));
        assert!(is_identity_file("BOOTSTRAP.md"));
        assert!(!is_identity_file("../models.json"));
        assert!(!is_identity_file("models.json"));
    }
}
