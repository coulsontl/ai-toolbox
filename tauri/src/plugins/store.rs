//! SQLite CRUD for every `plugin_entity` kind.
//!
//! All rows go through the shared `db_*` helpers: the table is `(kind, id)`
//! keyed, so `DbTable::PluginEntity(kind)` scopes every statement and one kind
//! can never read another's rows.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::db::helpers::{db_delete, db_get, db_list, db_patch_fields, db_put};
use crate::db::schema::{DbTable, OrderDirection, OrderField, OrderSpec, PluginEntityKind};
use crate::db::SqliteDbState;

use super::market::{BUILTIN_MARKET_ID, BUILTIN_MARKET_NAME, BUILTIN_MARKET_URL};
use super::paths::PluginPaths;
use super::types::{
    InstalledPluginRecord, MarketSourceRecord, PluginAccountMetaRecord, PluginProviderOptionValue,
    PluginSettings,
};

const SETTINGS_RECORD_ID: &str = "settings";
const AUTH_LOCK_STALE: Duration = Duration::from_secs(10);
const AUTH_LOCK_TIMEOUT: Duration = Duration::from_secs(20);

fn table(kind: PluginEntityKind) -> DbTable {
    DbTable::PluginEntity(kind)
}

fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

fn record_to_value<T: Serialize>(record: &T) -> Result<Value, String> {
    let mut value = serde_json::to_value(record)
        .map_err(|error| format!("Failed to serialize plugin record: {error}"))?;
    if let Some(object) = value.as_object_mut() {
        // The row columns own these; a payload copy would either be stale or
        // overwrite the helper-managed timestamps on write.
        object.remove("id");
        object.remove("created_at");
        object.remove("updated_at");
    }
    Ok(value)
}

fn parse_value<T: DeserializeOwned>(value: Value, context: &str) -> Result<T, String> {
    serde_json::from_value(value)
        .map_err(|error| format!("Failed to decode {context} plugin record: {error}"))
}

fn insert_record<T: Serialize>(
    db: &SqliteDbState,
    kind: PluginEntityKind,
    id: &str,
    record: &T,
) -> Result<(), String> {
    let value = record_to_value(record)?;
    db.with_conn(|conn| db_put(conn, table(kind), id, &value))
}

// ---- installed plugins -----------------------------------------------------

pub fn list_installed_plugins(db: &SqliteDbState) -> Result<Vec<InstalledPluginRecord>, String> {
    let order = OrderSpec::single(OrderField::created_at(OrderDirection::Asc));
    let values =
        db.with_conn(|conn| db_list(conn, table(PluginEntityKind::InstalledPlugin), Some(&order)))?;
    values
        .into_iter()
        .map(|value| parse_value(value, "installed plugin"))
        .collect()
}

pub fn get_installed_plugin(
    db: &SqliteDbState,
    id: &str,
) -> Result<Option<InstalledPluginRecord>, String> {
    let value = db.with_conn(|conn| db_get(conn, table(PluginEntityKind::InstalledPlugin), id))?;
    value
        .map(|value| parse_value(value, "installed plugin"))
        .transpose()
}

pub fn upsert_installed_plugin(
    db: &SqliteDbState,
    record: &InstalledPluginRecord,
) -> Result<InstalledPluginRecord, String> {
    if record.id.trim().is_empty() {
        return Err("Installed plugin id cannot be empty".to_string());
    }
    let mut record = record.clone();
    let now = now_rfc3339();
    if record.installed_at.trim().is_empty() {
        record.installed_at = now.clone();
    }
    record.updated_at = now;
    insert_record(db, PluginEntityKind::InstalledPlugin, &record.id, &record)?;
    get_installed_plugin(db, &record.id)?
        .ok_or_else(|| format!("Installed plugin '{}' disappeared after write", record.id))
}

pub fn delete_installed_plugin(db: &SqliteDbState, id: &str) -> Result<bool, String> {
    db.with_conn(|conn| db_delete(conn, table(PluginEntityKind::InstalledPlugin), id))
}

pub fn set_plugin_enabled(
    db: &SqliteDbState,
    id: &str,
    enabled: bool,
) -> Result<InstalledPluginRecord, String> {
    let value = db.with_conn(|conn| {
        db_patch_fields(
            conn,
            table(PluginEntityKind::InstalledPlugin),
            id,
            &[
                ("enabled", Value::Bool(enabled)),
                ("updatedAt", Value::String(now_rfc3339())),
            ],
        )
    })?;
    value
        .map(|value| parse_value(value, "installed plugin"))
        .transpose()?
        .ok_or_else(|| format!("Plugin '{id}' is not installed"))
}

pub fn set_plugin_options(
    db: &SqliteDbState,
    id: &str,
    options: &Value,
) -> Result<InstalledPluginRecord, String> {
    if !options.is_object() {
        return Err("Plugin options must be a JSON object".to_string());
    }
    let value = db.with_conn(|conn| {
        db_patch_fields(
            conn,
            table(PluginEntityKind::InstalledPlugin),
            id,
            &[
                ("options", options.clone()),
                ("updatedAt", Value::String(now_rfc3339())),
            ],
        )
    })?;
    value
        .map(|value| parse_value(value, "installed plugin"))
        .transpose()?
        .ok_or_else(|| format!("Plugin '{id}' is not installed"))
}

// ---- market sources ---------------------------------------------------------

fn default_builtin_market_source() -> MarketSourceRecord {
    MarketSourceRecord {
        id: BUILTIN_MARKET_ID.to_string(),
        name: BUILTIN_MARKET_NAME.to_string(),
        url: BUILTIN_MARKET_URL.to_string(),
        enabled: true,
        builtin: true,
        plugin_count: 0,
        last_synced_at: None,
        last_error: None,
    }
}

/// Make sure the seeded row exists. v38 seeds it, but a database restored from
/// a partial backup may still lack it; the built-in fallback must always have
/// an owner the UI can show and disable.
pub fn ensure_builtin_market_source(db: &SqliteDbState) -> Result<MarketSourceRecord, String> {
    if let Some(existing) = get_market_source(db, BUILTIN_MARKET_ID)? {
        return Ok(existing);
    }
    upsert_market_source(db, &default_builtin_market_source())
}

pub fn list_market_sources(db: &SqliteDbState) -> Result<Vec<MarketSourceRecord>, String> {
    ensure_builtin_market_source(db)?;
    let order = OrderSpec::single(OrderField::created_at(OrderDirection::Asc));
    let values =
        db.with_conn(|conn| db_list(conn, table(PluginEntityKind::MarketSource), Some(&order)))?;
    let mut records: Vec<MarketSourceRecord> = values
        .into_iter()
        .map(|value| parse_value(value, "market source"))
        .collect::<Result<_, _>>()?;
    // The built-in source is always first; the rest keep insertion order.
    records.sort_by(|left, right| {
        right
            .builtin
            .cmp(&left.builtin)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(records)
}

pub fn get_market_source(
    db: &SqliteDbState,
    id: &str,
) -> Result<Option<MarketSourceRecord>, String> {
    let value = db.with_conn(|conn| db_get(conn, table(PluginEntityKind::MarketSource), id))?;
    value
        .map(|value| parse_value(value, "market source"))
        .transpose()
}

pub fn upsert_market_source(
    db: &SqliteDbState,
    source: &MarketSourceRecord,
) -> Result<MarketSourceRecord, String> {
    if source.id.trim().is_empty() {
        return Err("Market source id cannot be empty".to_string());
    }
    if source.name.trim().is_empty() {
        return Err("Market source name cannot be empty".to_string());
    }
    if source.url.trim().is_empty() {
        return Err("Market source URL cannot be empty".to_string());
    }
    let mut source = source.clone();
    if source.id == BUILTIN_MARKET_ID {
        source.builtin = true;
    }
    insert_record(db, PluginEntityKind::MarketSource, &source.id, &source)?;
    get_market_source(db, &source.id)?
        .ok_or_else(|| format!("Market source '{}' disappeared after write", source.id))
}

/// Delete a custom market source. The built-in one may be disabled but never
/// removed: its fallback snapshot is the only offline catalog the UI has.
pub fn delete_market_source(db: &SqliteDbState, id: &str) -> Result<bool, String> {
    let Some(source) = get_market_source(db, id)? else {
        return Ok(false);
    };
    if source.builtin || id == BUILTIN_MARKET_ID {
        return Err(
            "The built-in Magpie Community market cannot be deleted; disable it instead"
                .to_string(),
        );
    }
    db.with_conn(|conn| db_delete(conn, table(PluginEntityKind::MarketSource), id))
}

// ---- settings ---------------------------------------------------------------

pub fn get_plugin_settings(db: &SqliteDbState) -> Result<PluginSettings, String> {
    let value =
        db.with_conn(|conn| db_get(conn, table(PluginEntityKind::Settings), SETTINGS_RECORD_ID))?;
    match value {
        None => Ok(PluginSettings::default()),
        Some(value) => parse_value(value, "plugin settings"),
    }
}

pub fn save_plugin_settings(
    db: &SqliteDbState,
    settings: &PluginSettings,
) -> Result<PluginSettings, String> {
    insert_record(db, PluginEntityKind::Settings, SETTINGS_RECORD_ID, settings)?;
    get_plugin_settings(db)
}

// ---- provider options -------------------------------------------------------

pub fn list_provider_options(db: &SqliteDbState) -> Result<Vec<PluginProviderOptionValue>, String> {
    let order = OrderSpec::single(OrderField::json_text("providerId", OrderDirection::Asc)?);
    let values =
        db.with_conn(|conn| db_list(conn, table(PluginEntityKind::ProviderOption), Some(&order)))?;
    values
        .into_iter()
        .map(|value| parse_value(value, "provider option"))
        .collect()
}

pub fn set_provider_option(
    db: &SqliteDbState,
    provider_id: &str,
    options: &Value,
) -> Result<PluginProviderOptionValue, String> {
    let provider_id = provider_id.trim();
    if provider_id.is_empty() {
        return Err("Provider id cannot be empty".to_string());
    }
    if !options.is_object() {
        return Err("Provider options must be a JSON object".to_string());
    }
    let record = PluginProviderOptionValue {
        provider_id: provider_id.to_string(),
        options: options.clone(),
        updated_at: now_rfc3339(),
    };
    insert_record(db, PluginEntityKind::ProviderOption, provider_id, &record)?;
    let value =
        db.with_conn(|conn| db_get(conn, table(PluginEntityKind::ProviderOption), provider_id))?;
    value
        .map(|value| parse_value(value, "provider option"))
        .transpose()?
        .ok_or_else(|| format!("Provider option '{provider_id}' disappeared after write"))
}

// ---- account metadata -------------------------------------------------------

pub fn list_account_metas(db: &SqliteDbState) -> Result<Vec<PluginAccountMetaRecord>, String> {
    let order = OrderSpec::single(OrderField::json_integer("sortIndex", OrderDirection::Asc)?);
    let values =
        db.with_conn(|conn| db_list(conn, table(PluginEntityKind::AccountMeta), Some(&order)))?;
    values
        .into_iter()
        .map(|value| parse_value(value, "plugin account meta"))
        .collect()
}

pub fn upsert_account_meta(
    db: &SqliteDbState,
    record: &PluginAccountMetaRecord,
) -> Result<PluginAccountMetaRecord, String> {
    if record.id.trim().is_empty() {
        return Err("Plugin account meta id cannot be empty".to_string());
    }
    if record.plugin_id.trim().is_empty() {
        return Err("Plugin account meta must name its plugin".to_string());
    }
    let mut record = record.clone();
    record.updated_at = now_rfc3339();
    insert_record(db, PluginEntityKind::AccountMeta, &record.id, &record)?;
    let value =
        db.with_conn(|conn| db_get(conn, table(PluginEntityKind::AccountMeta), &record.id))?;
    value
        .map(|value| parse_value(value, "plugin account meta"))
        .transpose()?
        .ok_or_else(|| {
            format!(
                "Plugin account meta '{}' disappeared after write",
                record.id
            )
        })
}

pub fn delete_account_meta(db: &SqliteDbState, id: &str) -> Result<bool, String> {
    db.with_conn(|conn| db_delete(conn, table(PluginEntityKind::AccountMeta), id))
}

// ---- auth file lock ---------------------------------------------------------

/// Guard for the cross-process `plugin-auth.json` lock. Dropping it removes the
/// lock file; Rust and the Bun host both take the same path.
pub struct AuthFileLock {
    path: PathBuf,
}

impl Drop for AuthFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Take `plugin-auth.json.lock` exclusively.
///
/// A lock left behind by a host that died is removed once it is older than
/// [`AUTH_LOCK_STALE`]; acquisition gives up after [`AUTH_LOCK_TIMEOUT`] so a
/// caller can never hang the async runtime forever.
pub fn acquire_auth_file_lock(paths: &PluginPaths) -> Result<AuthFileLock, String> {
    let lock_path = paths.auth_lock_path();
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }

    let started = std::time::Instant::now();
    loop {
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
        {
            Ok(_) => return Ok(AuthFileLock { path: lock_path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::metadata(&lock_path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                    .map(|age| age > AUTH_LOCK_STALE)
                    .unwrap_or(false);
                if stale {
                    let _ = fs::remove_file(&lock_path);
                    continue;
                }
                if started.elapsed() > AUTH_LOCK_TIMEOUT {
                    return Err(format!(
                        "Timed out waiting for the plugin auth lock {}",
                        lock_path.display()
                    ));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => {
                return Err(format!(
                    "Failed to take the plugin auth lock {}: {error}",
                    lock_path.display()
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_db() -> SqliteDbState {
        SqliteDbState::in_memory_for_test().expect("in-memory database")
    }

    #[test]
    fn builtin_market_source_can_be_disabled_but_not_deleted() {
        let db = test_db();
        let sources = list_market_sources(&db).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, BUILTIN_MARKET_ID);
        assert!(sources[0].builtin);
        assert!(sources[0].enabled);

        let mut disabled = sources[0].clone();
        disabled.enabled = false;
        upsert_market_source(&db, &disabled).unwrap();
        let sources = list_market_sources(&db).unwrap();
        assert!(!sources[0].enabled);

        let error = delete_market_source(&db, BUILTIN_MARKET_ID).unwrap_err();
        assert!(error.contains("cannot be deleted"), "{error}");
        assert!(get_market_source(&db, BUILTIN_MARKET_ID).unwrap().is_some());
    }

    #[test]
    fn installed_plugin_roundtrips_through_the_store() {
        let db = test_db();
        let record = InstalledPluginRecord {
            id: "demo".to_string(),
            spec: "@scope/demo@1.0.0".to_string(),
            package_name: "@scope/demo".to_string(),
            display_name: "Demo".to_string(),
            version: "1.0.0".to_string(),
            kind: crate::plugins::types::PluginKind::Provider,
            enabled: true,
            source_type: "npm".to_string(),
            market_id: Some("magpie-community".to_string()),
            has_provider_entry: true,
            options: json!({ "region": "global" }),
            ..InstalledPluginRecord::default()
        };
        let saved = upsert_installed_plugin(&db, &record).unwrap();
        assert!(!saved.installed_at.is_empty());
        assert!(!saved.updated_at.is_empty());

        let listed = list_installed_plugins(&db).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].options["region"], "global");

        let disabled = set_plugin_enabled(&db, "demo", false).unwrap();
        assert!(!disabled.enabled);
        let updated = set_plugin_options(&db, "demo", &json!({ "region": "cn" })).unwrap();
        assert_eq!(updated.options["region"], "cn");

        assert!(delete_installed_plugin(&db, "demo").unwrap());
        assert!(get_installed_plugin(&db, "demo").unwrap().is_none());
    }

    #[test]
    fn plugin_settings_roundtrip_and_default() {
        let db = test_db();
        assert_eq!(get_plugin_settings(&db).unwrap(), PluginSettings::default());

        let settings = PluginSettings {
            bun_manual_path: "C:/bun/bun.exe".to_string(),
            npm_registry: "https://registry.npmmirror.com".to_string(),
            follow_app_proxy: false,
        };
        let saved = save_plugin_settings(&db, &settings).unwrap();
        assert_eq!(saved, settings);
        assert_eq!(get_plugin_settings(&db).unwrap(), settings);
    }

    #[test]
    fn provider_options_and_account_metas_are_scoped_to_their_kinds() {
        let db = test_db();
        let option =
            set_provider_option(&db, "opencode-zen-free", &json!({ "region": "us" })).unwrap();
        assert_eq!(option.provider_id, "opencode-zen-free");
        assert_eq!(list_provider_options(&db).unwrap().len(), 1);

        let meta = PluginAccountMetaRecord {
            id: "acct-1".to_string(),
            plugin_id: "demo".to_string(),
            display_name: "Primary".to_string(),
            enabled: true,
            sort_index: 2,
            ..PluginAccountMetaRecord::default()
        };
        upsert_account_meta(&db, &meta).unwrap();
        let metas = list_account_metas(&db).unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].display_name, "Primary");
        assert_eq!(metas[0].sort_index, 2);
        assert!(delete_account_meta(&db, "acct-1").unwrap());

        // Account meta and provider option must not leak into installed plugins.
        assert!(list_installed_plugins(&db).unwrap().is_empty());
    }

    #[test]
    fn auth_lock_file_is_created_and_removed() {
        let root = tempfile::tempdir().unwrap();
        let paths = PluginPaths::new(root.path());
        paths.ensure_dirs().unwrap();

        let lock = acquire_auth_file_lock(&paths).unwrap();
        assert!(paths.auth_lock_path().exists());
        drop(lock);
        assert!(!paths.auth_lock_path().exists());
    }

    #[test]
    fn auth_lock_recovers_a_stale_lock_file() {
        let root = tempfile::tempdir().unwrap();
        let paths = PluginPaths::new(root.path());
        paths.ensure_dirs().unwrap();
        let lock_path = paths.auth_lock_path();
        fs::write(&lock_path, b"left by a dead host").unwrap();
        let file = fs::File::options().write(true).open(&lock_path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(60))
            .unwrap();
        drop(file);

        let lock = acquire_auth_file_lock(&paths).expect("stale lock must be reclaimed");
        assert!(lock_path.exists());
        drop(lock);
        assert!(!lock_path.exists());
    }
}
