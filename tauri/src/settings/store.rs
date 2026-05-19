use serde_json::Value;

use super::{adapter, types::AppSettings};
use crate::db::helpers::{db_get, db_patch_fields, db_put};
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;

const SETTINGS_ID: &str = "app";

pub fn load_settings_from_sqlite_state(
    sqlite_state: &SqliteDbState,
) -> Result<AppSettings, String> {
    sqlite_state.with_conn(load_settings_from_sqlite_conn)
}

pub fn save_settings_to_sqlite_state(
    sqlite_state: &SqliteDbState,
    settings: &AppSettings,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| save_settings_to_sqlite_conn(conn, settings))
}

pub fn update_last_auto_backup_time_in_sqlite_state(
    sqlite_state: &SqliteDbState,
    time: &str,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| {
        let updated = db_patch_fields(
            conn,
            DbTable::Settings,
            SETTINGS_ID,
            &[("last_auto_backup_time", Value::String(time.to_string()))],
        )?;

        if updated.is_none() {
            let mut payload = adapter::to_db_value(&AppSettings::default());
            if let Some(object) = payload.as_object_mut() {
                object.insert(
                    "last_auto_backup_time".to_string(),
                    Value::String(time.to_string()),
                );
            }
            db_put(conn, DbTable::Settings, SETTINGS_ID, &payload)?;
        }

        Ok(())
    })
}

pub fn load_settings_from_sqlite_conn(conn: &rusqlite::Connection) -> Result<AppSettings, String> {
    let record = db_get(conn, DbTable::Settings, SETTINGS_ID)?;
    Ok(record.map(adapter::from_db_value).unwrap_or_default())
}

pub fn save_settings_to_sqlite_conn(
    conn: &rusqlite::Connection,
    settings: &AppSettings,
) -> Result<(), String> {
    let json = adapter::to_db_value(settings);
    db_put(conn, DbTable::Settings, SETTINGS_ID, &json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDbState;

    #[test]
    fn sqlite_settings_round_trip_uses_adapter_defaults() {
        let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");

        let default_settings =
            load_settings_from_sqlite_state(&sqlite_state).expect("load default settings");
        assert_eq!(default_settings.theme, "system");
        assert_eq!(default_settings.proxy_mode, "system");
        assert!(default_settings.backup_image_assets_enabled);

        let mut settings = default_settings;
        settings.language = "en-US".to_string();
        settings.theme = "dark".to_string();
        settings.backup_image_assets_enabled = false;
        save_settings_to_sqlite_state(&sqlite_state, &settings).expect("save settings");

        let loaded = load_settings_from_sqlite_state(&sqlite_state).expect("reload settings");
        assert_eq!(loaded.language, "en-US");
        assert_eq!(loaded.theme, "dark");
        assert!(!loaded.backup_image_assets_enabled);
    }

    #[test]
    fn sqlite_last_auto_backup_time_update_creates_or_patches_settings() {
        let sqlite_state = SqliteDbState::in_memory_for_test().expect("sqlite");

        update_last_auto_backup_time_in_sqlite_state(&sqlite_state, "2026-05-19T00:00:00Z")
            .expect("create last backup time");
        let created = load_settings_from_sqlite_state(&sqlite_state).expect("load created");
        assert_eq!(
            created.last_auto_backup_time.as_deref(),
            Some("2026-05-19T00:00:00Z")
        );

        update_last_auto_backup_time_in_sqlite_state(&sqlite_state, "2026-05-20T00:00:00Z")
            .expect("patch last backup time");
        let patched = load_settings_from_sqlite_state(&sqlite_state).expect("load patched");
        assert_eq!(
            patched.last_auto_backup_time.as_deref(),
            Some("2026-05-20T00:00:00Z")
        );
    }
}
