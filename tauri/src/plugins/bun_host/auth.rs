//! Read-only view of `plugin-auth.json`.
//!
//! The file is written by the Bun host and follows OpenCode's `auth.json`
//! shape: one entry per account, keyed by provider id (`id#2` for the second
//! account of the same provider). The UI needs the account list, not the
//! credentials, so this module deliberately projects only non-secret fields.
//! Reading happens under the same cross-process lock the host takes.

use serde::Serialize;
use serde_json::Value;

use crate::db::SqliteDbState;

use super::super::paths::PluginPaths;
use super::super::store;

/// One signed-in account, with every credential removed.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginAccountSummary {
    /// Storage key: the provider id, or `id#<slot>`.
    pub account: String,
    pub provider: String,
    /// `oauth` or `api`.
    pub auth_type: String,
    pub has_access: bool,
    pub has_refresh: bool,
    pub has_key: bool,
    /// Unix milliseconds from the auth file, when it records an expiry.
    pub expires_at: Option<i64>,
    /// Display name from the local account metadata, if the user named it.
    pub display_name: String,
    pub enabled: bool,
    pub sort_index: i64,
    #[serde(default)]
    pub auto_checkin: bool,
    #[serde(default)]
    pub last_checkin_outcome: Option<String>,
    #[serde(default)]
    pub last_checkin_at: Option<String>,
    #[serde(default)]
    pub last_checkin_message: Option<String>,
    #[serde(default)]
    pub last_checkin_day: Option<String>,
}

/// Enumerate the auth file. Returns an empty list when no one signed in yet.
pub fn list_accounts(
    db: &SqliteDbState,
    paths: &PluginPaths,
) -> Result<Vec<PluginAccountSummary>, String> {
    let _lock = store::acquire_auth_file_lock(paths)?;
    let bytes = match std::fs::read(paths.auth_path()) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "Failed to read '{}': {error}",
                paths.auth_path().display()
            ))
        }
    };
    let auth: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Invalid plugin auth file: {error}"))?;
    let Some(entries) = auth.as_object() else {
        return Err("Plugin auth file is not a JSON object".to_string());
    };

    let metas = store::list_account_metas(db).unwrap_or_default();
    let meta_for = |account: &str| metas.iter().find(|meta| meta.id == account);

    let mut accounts: Vec<PluginAccountSummary> = entries
        .iter()
        .filter_map(|(account, value)| {
            let object = value.as_object()?;
            let auth_type = object
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let has_text = |key: &str| {
                object
                    .get(key)
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
            };
            let meta = meta_for(account);
            Some(PluginAccountSummary {
                account: account.clone(),
                provider: account.split('#').next().unwrap_or(account).to_string(),
                auth_type,
                has_access: has_text("access"),
                has_refresh: has_text("refresh"),
                has_key: has_text("key"),
                expires_at: object.get("expires").and_then(Value::as_i64),
                display_name: meta
                    .map(|meta| meta.display_name.clone())
                    .unwrap_or_default(),
                enabled: meta.map(|meta| meta.enabled).unwrap_or(true),
                sort_index: meta.map(|meta| meta.sort_index).unwrap_or(0),
                auto_checkin: meta.map(|meta| meta.auto_checkin).unwrap_or(false),
                last_checkin_outcome: meta.and_then(|meta| meta.last_checkin_outcome.clone()),
                last_checkin_at: meta.and_then(|meta| meta.last_checkin_at.clone()),
                last_checkin_message: meta.and_then(|meta| meta.last_checkin_message.clone()),
                last_checkin_day: meta.and_then(|meta| meta.last_checkin_day.clone()),
            })
        })
        .collect();

    accounts.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.sort_index.cmp(&right.sort_index))
            .then_with(|| left.account.cmp(&right.account))
    });
    Ok(accounts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::types::PluginAccountMetaRecord;

    #[test]
    fn lists_accounts_without_leaking_credentials() {
        let root = tempfile::tempdir().unwrap();
        let paths = PluginPaths::new(root.path());
        paths.ensure_dirs().unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();

        store::upsert_account_meta(
            &db,
            &PluginAccountMetaRecord {
                id: "demo#2".to_string(),
                plugin_id: "demo-plugin".to_string(),
                display_name: "Secondary".to_string(),
                enabled: false,
                sort_index: 3,
                ..PluginAccountMetaRecord::default()
            },
        )
        .unwrap();

        std::fs::write(
            paths.auth_path(),
            serde_json::json!({
                "demo": {
                    "type": "oauth",
                    "access": "secret-access-token",
                    "refresh": "secret-refresh-token",
                    "expires": 1_800_000_000_000i64
                },
                "demo#2": {
                    "type": "api",
                    "key": "secret-api-key"
                }
            })
            .to_string(),
        )
        .unwrap();

        let accounts = list_accounts(&db, &paths).unwrap();
        assert_eq!(accounts.len(), 2);
        let primary = accounts.iter().find(|a| a.account == "demo").unwrap();
        assert_eq!(primary.provider, "demo");
        assert_eq!(primary.auth_type, "oauth");
        assert!(primary.has_access && primary.has_refresh && !primary.has_key);
        assert_eq!(primary.expires_at, Some(1_800_000_000_000));

        let secondary = accounts.iter().find(|a| a.account == "demo#2").unwrap();
        assert_eq!(secondary.provider, "demo");
        assert_eq!(secondary.display_name, "Secondary");
        assert!(!secondary.enabled);

        // The serialized DTO must not carry any credential value.
        let serialized = serde_json::to_string(&accounts).unwrap();
        assert!(!serialized.contains("secret-access-token"));
        assert!(!serialized.contains("secret-refresh-token"));
        assert!(!serialized.contains("secret-api-key"));
    }

    #[test]
    fn a_missing_auth_file_is_an_empty_account_list() {
        let root = tempfile::tempdir().unwrap();
        let paths = PluginPaths::new(root.path());
        paths.ensure_dirs().unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        assert!(list_accounts(&db, &paths).unwrap().is_empty());
    }
}
