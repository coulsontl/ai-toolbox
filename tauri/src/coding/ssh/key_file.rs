//! SSH private key file management
//!
//! When users paste private key content directly instead of providing a file path,
//! the key content is stored in the database and materialized as a file under
//! `<app_data_dir>/.ssh/<md5_hex>`. On backup/restore to another device the file
//! is recreated automatically from the database content.

use std::fs;
use std::path::{Path, PathBuf};

/// Check whether the given string looks like a PEM private key (content, not a path).
pub fn is_private_key_content(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("-----BEGIN")
}

/// Return the `.ssh` directory under the given app data directory, creating it if needed.
pub fn ssh_key_dir(app_data_dir: &Path) -> Result<PathBuf, String> {
    let dir = app_data_dir.join(".ssh");
    if !dir.exists() {
        fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create .ssh directory: {}", e))?;
    }
    Ok(dir)
}

/// Compute MD5 hex digest of the given content.
pub fn md5_hex(content: &str) -> String {
    format!("{:x}", md5::compute(content.trim()))
}

/// Materialise a private-key file on disk from its content.
/// Returns the absolute path to the written file.
pub fn ensure_key_file(app_data_dir: &Path, content: &str) -> Result<String, String> {
    let dir = ssh_key_dir(app_data_dir)?;
    let hash = md5_hex(content);
    let file_path = dir.join(&hash);

    if !file_path.exists() {
        fs::write(&file_path, content.trim())
            .map_err(|e| format!("Failed to write key file: {}", e))?;

        // On Unix, ssh requires key files to have restricted permissions (0600)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            let _ = fs::set_permissions(&file_path, perms);
        }

        log::info!("SSH key file created: {:?}", file_path);
    }

    Ok(file_path.to_string_lossy().to_string())
}

/// Remove a key file identified by its content MD5.
pub fn remove_key_file(app_data_dir: &Path, content: &str) {
    if content.trim().is_empty() {
        return;
    }
    let hash = md5_hex(content);
    let dir = match ssh_key_dir(app_data_dir) {
        Ok(d) => d,
        Err(_) => return,
    };
    let file_path = dir.join(&hash);
    if file_path.exists() {
        let _ = fs::remove_file(&file_path);
        log::info!("SSH key file removed: {:?}", file_path);
    }
}

/// Resolve the effective private key file path for an SSH connection.
///
/// - If `private_key_content` is non-empty (starts with `-----BEGIN`), materialise
///   the key file and return its path.
/// - Otherwise fall back to `private_key_path` (user-supplied path).
pub fn resolve_key_path(
    app_data_dir: &Path,
    private_key_path: &str,
    private_key_content: &str,
) -> Result<String, String> {
    if !private_key_content.trim().is_empty() && is_private_key_content(private_key_content) {
        ensure_key_file(app_data_dir, private_key_content)
    } else {
        Ok(private_key_path.to_string())
    }
}
