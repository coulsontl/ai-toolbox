//! Discovery file written next to the user's home directory.
//!
//! The file is the only hand-off between the desktop app and `aitoolbox`: the CLI
//! reads the loopback URL and bearer token from it. It never appears in API
//! responses or logs.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DISCOVERY_FILE_NAME: &str = "control.json";
pub const CONTROL_FILE_ENV: &str = "AI_TOOLBOX_CONTROL_FILE";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryFile {
    pub version: u32,
    pub pid: u32,
    pub url: String,
    pub token: String,
    pub app_version: String,
    pub data_dir: String,
    pub started_at: String,
}

pub fn default_discovery_path() -> PathBuf {
    if let Ok(path) = std::env::var(CONTROL_FILE_ENV) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".ai-toolbox").join(DISCOVERY_FILE_NAME)
}

/// Writes the discovery file atomically with owner-only permissions on Unix.
pub fn write_discovery_file(path: &Path, info: &DiscoveryFile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }
    let payload = serde_json::to_vec(info)
        .map_err(|error| format!("Failed to serialize discovery file: {error}"))?;
    let temp_path = path.with_extension(format!("tmp-{}", std::process::id()));
    {
        let mut file = fs::File::create(&temp_path)
            .map_err(|error| format!("Failed to create {}: {error}", temp_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = fs::Permissions::from_mode(0o600);
            file.set_permissions(permissions)
                .map_err(|error| format!("Failed to set discovery file permissions: {error}"))?;
        }
        file.write_all(&payload)
            .map_err(|error| format!("Failed to write {}: {error}", temp_path.display()))?;
        file.sync_all()
            .map_err(|error| format!("Failed to sync {}: {error}", temp_path.display()))?;
    }
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "Failed to replace discovery file {}: {error}",
            path.display()
        ));
    }
    Ok(())
}

/// Reads a discovery file. The CLI uses the same shape.
pub fn read_discovery_file(path: &Path) -> Result<DiscoveryFile, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))
}

/// Removes the file only when it still belongs to this process.
pub fn remove_discovery_file_if_owned(path: &Path, pid: u32) {
    let owned = read_discovery_file(path)
        .map(|file| file.pid == pid)
        .unwrap_or(false);
    if owned {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pid: u32) -> DiscoveryFile {
        DiscoveryFile {
            version: 1,
            pid,
            url: "http://127.0.0.1:54321".to_string(),
            token: "0123456789abcdef".to_string(),
            app_version: "1.2.4".to_string(),
            data_dir: "/tmp/data".to_string(),
            started_at: "2026-11-27T21:40:00+08:00".to_string(),
        }
    }

    #[test]
    fn discovery_file_round_trips_and_uses_camel_case() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.json");
        write_discovery_file(&path, &sample(42)).unwrap();
        let parsed = read_discovery_file(&path).unwrap();
        assert_eq!(parsed, sample(42));

        let raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["appVersion"], "1.2.4");
        assert_eq!(raw["dataDir"], "/tmp/data");
        assert_eq!(raw["startedAt"], "2026-11-27T21:40:00+08:00");
    }

    #[test]
    fn only_the_owning_process_deletes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.json");
        write_discovery_file(&path, &sample(7)).unwrap();
        remove_discovery_file_if_owned(&path, 8);
        assert!(path.exists());
        remove_discovery_file_if_owned(&path, 7);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn discovery_file_is_owner_only_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.json");
        write_discovery_file(&path, &sample(1)).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
