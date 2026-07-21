use crate::coding::proxy_gateway::types::{GatewayCliKey, GatewayProxyMode};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CliProxyManifest {
    pub schema_version: u32,
    pub managed_by: String,
    pub cli_key: GatewayCliKey,
    pub enabled: bool,
    pub mode: GatewayProxyMode,
    pub primary_provider_id: String,
    pub base_origin: String,
    pub created_at: String,
    pub updated_at: String,
    pub files: Vec<CliProxyManifestFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CliProxyManifestFile {
    pub kind: String,
    pub path: String,
    pub existed: bool,
    pub backup_rel_path: String,
    pub backup_sha256: Option<String>,
    pub backup_size: Option<u64>,
    pub managed_fields: Vec<String>,
}

impl CliProxyManifest {
    pub fn new(
        cli_key: GatewayCliKey,
        base_origin: String,
        timestamp: String,
        mode: GatewayProxyMode,
        primary_provider_id: String,
    ) -> Self {
        Self {
            schema_version: 1,
            managed_by: "ai-toolbox-proxy-gateway".to_string(),
            cli_key,
            enabled: true,
            mode,
            primary_provider_id,
            base_origin,
            created_at: timestamp.clone(),
            updated_at: timestamp,
            files: Vec::new(),
        }
    }
}

pub fn validate_backup_rel_path(path: &str) -> Result<(), String> {
    if path.contains(':') || path.contains('\\') {
        return Err("Manifest backup path must use a relative forward-slash path".to_string());
    }
    let path = Path::new(path);
    if path.is_absolute() {
        return Err("Manifest backup path must be relative".to_string());
    }
    for component in path.components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("Manifest backup path cannot escape the backup directory".to_string())
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_serializes_without_provider_data() {
        let mut manifest = CliProxyManifest::new(
            GatewayCliKey::Codex,
            "http://127.0.0.1:37123".to_string(),
            "2026-05-16T10:00:00Z".to_string(),
            GatewayProxyMode::Single,
            "provider-1".to_string(),
        );
        manifest.files.push(CliProxyManifestFile {
            kind: "codex_config_toml".to_string(),
            path: "C:\\Users\\User\\.codex\\config.toml".to_string(),
            existed: true,
            backup_rel_path: "backups/config.toml".to_string(),
            backup_sha256: Some("abc".to_string()),
            backup_size: Some(123),
            managed_fields: vec![
                "model_providers.custom.base_url".to_string(),
                "model_providers.custom.wire_api".to_string(),
                "model_providers.custom.experimental_bearer_token".to_string(),
            ],
        });

        let json = serde_json::to_string(&manifest).unwrap();

        assert!(json.contains("codex_config_toml"));
        assert!(json.contains("primary_provider_id"));
        assert!(!json.contains("settings_config"));
        assert!(!json.contains("api_key"));
    }

    #[test]
    fn backup_relative_path_accepts_normal_path() {
        assert!(validate_backup_rel_path("backups/config.toml").is_ok());
    }

    #[test]
    fn backup_relative_path_rejects_parent_escape() {
        assert!(validate_backup_rel_path("../config.toml").is_err());
        assert!(validate_backup_rel_path("backups/../../config.toml").is_err());
    }

    #[test]
    fn backup_relative_path_rejects_absolute_path() {
        assert!(validate_backup_rel_path("C:\\Users\\config.toml").is_err());
        assert!(validate_backup_rel_path("/tmp/config.toml").is_err());
    }
}
