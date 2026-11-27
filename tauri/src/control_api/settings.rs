//! Persisted control-plane settings.
//!
//! Stored as its own `settings` row (`id = control_api`) instead of inside
//! `settings/app`: `save_settings` is a full-document replace, and adding this
//! section to `AppSettings` would let any old frontend snapshot silently reset
//! the approval policy. The independent row keeps the two save paths disjoint.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::helpers::{db_get, db_put};
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;

pub const CONTROL_API_SETTINGS_ID: &str = "control_api";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ControlApiApprovalMode {
    /// Every write (L1 and L2) asks the desktop user first.
    #[default]
    AlwaysAsk,
    /// Reversible changes pass; only destructive/trust-boundary actions ask.
    AskHighRisk,
    /// Any caller holding the local bearer token is trusted.
    AutoApprove,
    /// Read-only endpoints only.
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlApiSettings {
    pub enabled: bool,
    pub approval_mode: ControlApiApprovalMode,
    pub auto_approve_local_plugin_reload: bool,
    pub approval_timeout_secs: u64,
}

impl Default for ControlApiSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            approval_mode: ControlApiApprovalMode::AlwaysAsk,
            auto_approve_local_plugin_reload: false,
            approval_timeout_secs: 60,
        }
    }
}

impl ControlApiSettings {
    /// Keeps the UI from persisting values the approval hub cannot honour.
    pub fn normalized(mut self) -> Self {
        self.approval_timeout_secs = self.approval_timeout_secs.clamp(15, 300);
        self
    }
}

fn value_to_settings(value: &Value) -> ControlApiSettings {
    let defaults = ControlApiSettings::default();
    let approval_mode = value
        .get("approval_mode")
        .or_else(|| value.get("approvalMode"))
        .and_then(Value::as_str)
        .and_then(|raw| {
            serde_json::from_value::<ControlApiApprovalMode>(Value::String(raw.to_string())).ok()
        })
        .unwrap_or(defaults.approval_mode);
    ControlApiSettings {
        enabled: value
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(defaults.enabled),
        approval_mode,
        auto_approve_local_plugin_reload: value
            .get("auto_approve_local_plugin_reload")
            .or_else(|| value.get("autoApproveLocalPluginReload"))
            .and_then(Value::as_bool)
            .unwrap_or(defaults.auto_approve_local_plugin_reload),
        approval_timeout_secs: value
            .get("approval_timeout_secs")
            .or_else(|| value.get("approvalTimeoutSecs"))
            .and_then(Value::as_u64)
            .unwrap_or(defaults.approval_timeout_secs),
    }
    .normalized()
}

pub fn load_control_api_settings(db: &SqliteDbState) -> Result<ControlApiSettings, String> {
    db.with_conn(|conn| {
        Ok(db_get(conn, DbTable::Settings, CONTROL_API_SETTINGS_ID)?
            .as_ref()
            .map(value_to_settings)
            .unwrap_or_default())
    })
}

pub fn save_control_api_settings(
    db: &SqliteDbState,
    settings: &ControlApiSettings,
) -> Result<ControlApiSettings, String> {
    let settings = settings.clone().normalized();
    let payload = serde_json::to_value(&settings)
        .map_err(|error| format!("Failed to serialize control API settings: {error}"))?;
    db.with_conn(|conn| db_put(conn, DbTable::Settings, CONTROL_API_SETTINGS_ID, &payload))?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_require_approval_and_keep_the_timeout_in_range() {
        let settings = ControlApiSettings::default();
        assert!(settings.enabled);
        assert_eq!(settings.approval_mode, ControlApiApprovalMode::AlwaysAsk);
        assert!(!settings.auto_approve_local_plugin_reload);
        assert_eq!(settings.approval_timeout_secs, 60);

        let clamped = ControlApiSettings {
            approval_timeout_secs: 1,
            ..ControlApiSettings::default()
        }
        .normalized();
        assert_eq!(clamped.approval_timeout_secs, 15);
        let clamped = ControlApiSettings {
            approval_timeout_secs: 9_999,
            ..ControlApiSettings::default()
        }
        .normalized();
        assert_eq!(clamped.approval_timeout_secs, 300);
    }

    #[test]
    fn settings_round_trip_through_the_independent_settings_row() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        assert_eq!(
            load_control_api_settings(&db).unwrap(),
            ControlApiSettings::default()
        );

        let saved = save_control_api_settings(
            &db,
            &ControlApiSettings {
                enabled: false,
                approval_mode: ControlApiApprovalMode::AskHighRisk,
                auto_approve_local_plugin_reload: true,
                approval_timeout_secs: 120,
            },
        )
        .unwrap();
        assert_eq!(saved.approval_timeout_secs, 120);
        assert_eq!(load_control_api_settings(&db).unwrap(), saved);

        // The app-level settings row stays untouched.
        let app_row = db
            .with_conn(|conn| db_get(conn, DbTable::Settings, crate::settings::store::SETTINGS_ID))
            .unwrap();
        assert!(app_row.is_none());
    }

    #[test]
    fn legacy_camel_case_and_partial_rows_fall_back_per_field() {
        let partial = value_to_settings(&json!({
            "approvalMode": "auto_approve",
            "approvalTimeoutSecs": 5000,
        }));
        assert!(partial.enabled);
        assert_eq!(partial.approval_mode, ControlApiApprovalMode::AutoApprove);
        assert_eq!(partial.approval_timeout_secs, 300);

        let broken = value_to_settings(&json!({"approval_mode": "not-a-mode"}));
        assert_eq!(broken.approval_mode, ControlApiApprovalMode::AlwaysAsk);
    }

    #[test]
    fn settings_json_uses_snake_case_for_the_frontend_and_database() {
        let value = serde_json::to_value(ControlApiSettings::default()).unwrap();
        assert_eq!(value["approval_mode"], "always_ask");
        assert!(value.get("approvalMode").is_none());
    }
}
