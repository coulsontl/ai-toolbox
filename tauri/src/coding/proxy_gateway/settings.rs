use super::listen::validate_settings;
use super::types::ProxyGatewaySettings;
use crate::db::helpers::{db_get, db_put};
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;
use serde_json::Value;

const SETTINGS_ID: &str = "gateway";

pub fn load_settings_from_sqlite_state(
    sqlite_state: &SqliteDbState,
) -> Result<ProxyGatewaySettings, String> {
    sqlite_state.with_conn(|conn| {
        let Some(record) = db_get(conn, DbTable::ProxyGatewaySettings, SETTINGS_ID)? else {
            return Ok(ProxyGatewaySettings::default());
        };
        settings_from_value(record)
    })
}

pub fn save_settings_to_sqlite_state(
    sqlite_state: &SqliteDbState,
    settings: ProxyGatewaySettings,
) -> Result<ProxyGatewaySettings, String> {
    let settings = normalize_settings(settings)?;
    let data = serde_json::to_value(&settings)
        .map_err(|error| format!("Failed to serialize proxy gateway settings: {error}"))?;
    sqlite_state
        .with_conn(|conn| db_put(conn, DbTable::ProxyGatewaySettings, SETTINGS_ID, &data))?;
    Ok(settings)
}

pub fn save_settings(
    sqlite_state: &SqliteDbState,
    settings: ProxyGatewaySettings,
) -> Result<ProxyGatewaySettings, String> {
    save_settings_to_sqlite_state(sqlite_state, settings)
}

pub fn settings_from_value(value: Value) -> Result<ProxyGatewaySettings, String> {
    let settings: ProxyGatewaySettings =
        serde_json::from_value(value).unwrap_or_else(|_| ProxyGatewaySettings::default());
    normalize_loaded_settings(settings)
}

pub fn normalize_settings(
    mut settings: ProxyGatewaySettings,
) -> Result<ProxyGatewaySettings, String> {
    normalize_common_settings(&mut settings)?;
    validate_settings(&settings)?;
    Ok(settings)
}

fn normalize_loaded_settings(
    mut settings: ProxyGatewaySettings,
) -> Result<ProxyGatewaySettings, String> {
    normalize_common_settings(&mut settings)?;
    clamp_zero_timeouts_for_legacy_settings(&mut settings);
    validate_settings(&settings)?;
    Ok(settings)
}

fn normalize_common_settings(settings: &mut ProxyGatewaySettings) -> Result<(), String> {
    if settings.enabled_cli_keys.is_empty() {
        settings.enabled_cli_keys = ProxyGatewaySettings::default().enabled_cli_keys;
    }
    settings.retryable_status_codes = super::retryable_status::normalize_retryable_status_codes(
        &settings.retryable_status_codes,
    )?;
    Ok(())
}

fn clamp_zero_timeouts_for_legacy_settings(settings: &mut ProxyGatewaySettings) {
    if settings.streaming_first_byte_timeout_secs == 0 {
        settings.streaming_first_byte_timeout_secs = 1;
    }
    if settings.streaming_idle_timeout_secs == 0 {
        settings.streaming_idle_timeout_secs = 1;
    }
    if settings.non_streaming_timeout_secs == 0 {
        settings.non_streaming_timeout_secs = 1;
    }
    for app_config in settings.app_configs.values_mut() {
        if matches!(app_config.streaming_first_byte_timeout_secs, Some(0)) {
            app_config.streaming_first_byte_timeout_secs = Some(1);
        }
        if matches!(app_config.streaming_idle_timeout_secs, Some(0)) {
            app_config.streaming_idle_timeout_secs = Some(1);
        }
        if matches!(app_config.non_streaming_timeout_secs, Some(0)) {
            app_config.non_streaming_timeout_secs = Some(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::proxy_gateway::types::{AppProxyConfig, GatewayCliKey};
    use crate::db::helpers::db_put;
    use crate::db::schema::DbTable;
    use crate::db::SqliteDbState;
    use serde_json::json;

    #[test]
    fn legacy_settings_without_session_usage_toggle_default_to_enabled() {
        // A settings record written before `session_usage_enabled` existed must
        // load with the toggle defaulting to on, and the next save persists it.
        let loaded = settings_from_value(json!({
            "listen_host": "127.0.0.1",
            "listen_port": 37123,
            "request_log_enabled": true,
            "metrics_enabled": false,
            "enabled_cli_keys": ["claude"],
        }))
        .unwrap();
        assert!(loaded.session_usage_enabled);

        let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");
        let saved = save_settings_to_sqlite_state(&sqlite_state, loaded).unwrap();
        assert!(saved.session_usage_enabled);
        let reloaded = load_settings_from_sqlite_state(&sqlite_state).unwrap();
        assert!(reloaded.session_usage_enabled);
    }

    #[test]
    fn missing_settings_fields_use_defaults() {
        let settings = settings_from_value(json!({})).unwrap();
        assert_eq!(settings.listen_host, "127.0.0.1");
        assert_eq!(settings.listen_port, 37123);
        assert!(settings.metrics_enabled);
        assert!(settings.session_usage_enabled);
        assert!(!settings.enabled_on_startup);
        assert_eq!(settings.per_provider_retry_count, 0);
        assert_eq!(settings.max_retry_count, 8);
        assert_eq!(settings.retry_interval_secs, 1);
        assert_eq!(
            settings.retryable_status_codes,
            super::super::retryable_status::DEFAULT_RETRYABLE_STATUS_CODES_COMPACT
        );
        assert!(settings.thinking_rectifier_enabled);
        assert!(settings.responses_encrypted_content_rectifier_enabled);
        assert!(!settings.lossy_rejection_enabled);
    }

    #[test]
    fn retryable_status_codes_are_normalized_on_load() {
        let settings = settings_from_value(json!({
            "retryable_status_codes": "429, 400, 502-504",
        }))
        .unwrap();
        assert_eq!(settings.retryable_status_codes, "400,429,502-504");
    }

    #[test]
    fn invalid_retryable_status_codes_are_rejected() {
        assert!(settings_from_value(json!({
            "retryable_status_codes": "abc",
        }))
        .is_err());
    }

    #[test]
    fn enabled_on_startup_preserves_explicit_true() {
        let settings = settings_from_value(json!({
            "enabled_on_startup": true,
        }))
        .unwrap();

        assert!(settings.enabled_on_startup);
    }

    #[test]
    fn thinking_and_responses_rectifier_settings_are_independent() {
        let responses_only = settings_from_value(json!({
            "thinking_rectifier_enabled": false,
            "responses_encrypted_content_rectifier_enabled": true,
        }))
        .unwrap();
        assert!(!responses_only.thinking_rectifier_enabled);
        assert!(responses_only.responses_encrypted_content_rectifier_enabled);

        let thinking_only = settings_from_value(json!({
            "thinking_rectifier_enabled": true,
            "responses_encrypted_content_rectifier_enabled": false,
        }))
        .unwrap();
        assert!(thinking_only.thinking_rectifier_enabled);
        assert!(!thinking_only.responses_encrypted_content_rectifier_enabled);
    }

    #[test]
    fn empty_enabled_cli_keys_are_repaired_to_mvp_defaults() {
        let settings = settings_from_value(json!({
            "enabled_cli_keys": []
        }))
        .unwrap();

        assert_eq!(settings.enabled_cli_keys, GatewayCliKey::supported_mvp());
    }

    #[test]
    fn invalid_persisted_host_is_rejected() {
        assert!(settings_from_value(json!({
            "listen_host": "http://127.0.0.1"
        }))
        .is_err());
    }

    #[test]
    fn retry_count_cannot_exceed_global_retry_count() {
        assert!(settings_from_value(json!({
            "per_provider_retry_count": 3,
            "max_retry_count": 2,
        }))
        .is_err());
    }

    #[test]
    fn sqlite_settings_round_trip_uses_defaults_and_validation() {
        let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");

        let defaults = load_settings_from_sqlite_state(&sqlite_state).expect("load defaults");
        assert_eq!(defaults.listen_host, "127.0.0.1");
        assert_eq!(defaults.listen_port, 37123);

        let mut settings = defaults;
        settings.listen_port = 38123;
        settings.enabled_on_startup = true;
        save_settings_to_sqlite_state(&sqlite_state, settings).expect("save settings");

        let loaded = load_settings_from_sqlite_state(&sqlite_state).expect("reload settings");
        assert_eq!(loaded.listen_port, 38123);
        assert!(loaded.enabled_on_startup);
    }

    #[test]
    fn zero_timeouts_are_rejected_at_save_time() {
        // Each timeout field must be rejected when set to 0 so the runtime
        // never builds a zero Duration from persisted settings.
        let mut settings = ProxyGatewaySettings::default();
        settings.streaming_first_byte_timeout_secs = 0;
        assert!(normalize_settings(settings).is_err());

        let mut settings = ProxyGatewaySettings::default();
        settings.streaming_idle_timeout_secs = 0;
        assert!(normalize_settings(settings).is_err());

        let mut settings = ProxyGatewaySettings::default();
        settings.non_streaming_timeout_secs = 0;
        assert!(normalize_settings(settings).is_err());
    }

    #[test]
    fn zero_app_timeouts_are_rejected_at_save_time() {
        let mut settings = ProxyGatewaySettings::default();
        settings.app_configs.insert(
            GatewayCliKey::Codex,
            AppProxyConfig {
                streaming_first_byte_timeout_secs: Some(0),
                ..AppProxyConfig::default()
            },
        );
        assert!(normalize_settings(settings).is_err());

        let mut settings = ProxyGatewaySettings::default();
        settings.app_configs.insert(
            GatewayCliKey::Codex,
            AppProxyConfig {
                streaming_idle_timeout_secs: Some(0),
                ..AppProxyConfig::default()
            },
        );
        assert!(normalize_settings(settings).is_err());

        let mut settings = ProxyGatewaySettings::default();
        settings.app_configs.insert(
            GatewayCliKey::Codex,
            AppProxyConfig {
                non_streaming_timeout_secs: Some(0),
                ..AppProxyConfig::default()
            },
        );
        assert!(normalize_settings(settings).is_err());
    }

    #[test]
    fn legacy_zero_timeouts_are_clamped_on_load() {
        let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");
        sqlite_state
            .with_conn(|conn| {
                db_put(
                    conn,
                    DbTable::ProxyGatewaySettings,
                    SETTINGS_ID,
                    &json!({
                        "streaming_first_byte_timeout_secs": 0,
                        "streaming_idle_timeout_secs": 0,
                        "non_streaming_timeout_secs": 0,
                        "app_configs": {
                            "codex": {
                                "streaming_first_byte_timeout_secs": 0,
                                "streaming_idle_timeout_secs": 0,
                                "non_streaming_timeout_secs": 0
                            }
                        }
                    }),
                )
            })
            .expect("seed legacy settings");

        let settings = load_settings_from_sqlite_state(&sqlite_state).expect("load settings");
        assert_eq!(settings.streaming_first_byte_timeout_secs, 1);
        assert_eq!(settings.streaming_idle_timeout_secs, 1);
        assert_eq!(settings.non_streaming_timeout_secs, 1);

        let codex_config = settings.effective_app_config(GatewayCliKey::Codex);
        assert_eq!(codex_config.streaming_first_byte_timeout_secs, 1);
        assert_eq!(codex_config.streaming_idle_timeout_secs, 1);
        assert_eq!(codex_config.non_streaming_timeout_secs, 1);
    }

    #[test]
    fn positive_timeouts_are_accepted() {
        let settings = settings_from_value(json!({
            "streaming_first_byte_timeout_secs": 1,
            "streaming_idle_timeout_secs": 1,
            "non_streaming_timeout_secs": 1,
        }))
        .unwrap();
        assert_eq!(settings.streaming_first_byte_timeout_secs, 1);
        assert_eq!(settings.streaming_idle_timeout_secs, 1);
        assert_eq!(settings.non_streaming_timeout_secs, 1);
    }

    #[test]
    fn positive_app_timeouts_are_accepted_and_effective() {
        let settings = settings_from_value(json!({
            "app_configs": {
                "codex": {
                    "streaming_first_byte_timeout_secs": 2,
                    "streaming_idle_timeout_secs": 3,
                    "non_streaming_timeout_secs": 4,
                }
            }
        }))
        .unwrap();

        let codex_config = settings.effective_app_config(GatewayCliKey::Codex);
        assert_eq!(codex_config.streaming_first_byte_timeout_secs, 2);
        assert_eq!(codex_config.streaming_idle_timeout_secs, 3);
        assert_eq!(codex_config.non_streaming_timeout_secs, 4);
    }
}
