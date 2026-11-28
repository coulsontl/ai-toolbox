//! Plugin update checks and application.
//!
//! The updater deliberately reuses the existing install path
//! (`installer::install_plugin`) and npm lookup (`market::fetch_npm_latest_version`)
//! so a plugin update behaves exactly like a fresh install followed by restoring
//! the user-owned record fields. All package mutations are serialized by
//! `PluginManager::update_lock`.

use std::cmp::Ordering;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::commands;
use super::installer;
use super::market;
use super::store;
use super::types::{InstalledPluginRecord, PluginSettings};
use super::PluginManagerState;

const STATUS_UP_TO_DATE: &str = "up_to_date";
const STATUS_AVAILABLE: &str = "available";
const STATUS_UPDATED: &str = "updated";
const STATUS_SKIPPED: &str = "skipped";
const STATUS_FAILED: &str = "failed";

const SKIP_LOCAL: &str = "Local plugins are not updated automatically";
const SKIP_PINNED: &str = "The plugin version is pinned in its spec";
const SKIP_NO_PACKAGE: &str = "The plugin has no npm package name";
const SKIP_GITHUB: &str =
    "GitHub plugins can only be refreshed with a single-plugin update";
const SKIP_UNSUPPORTED: &str = "Unsupported plugin source";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginUpdateItem {
    pub plugin_id: String,
    pub package_name: String,
    pub display_name: String,
    pub current_version: String,
    pub latest_version: String,
    /// `up_to_date` | `available` | `updated` | `skipped` | `failed`.
    pub status: String,
    /// Skip reason, error text, or a short success note.
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginUpdateSummary {
    pub checked_at: String,
    pub updated_count: usize,
    pub available_count: usize,
    pub failed_count: usize,
    pub items: Vec<PluginUpdateItem>,
}

/// Whether an npm spec pins a concrete version or dist-tag.
///
/// `pkg`, `pkg@latest` and `@scope/pkg` are updatable; `pkg@1.2.3`,
/// `pkg@^1.0.0`, `pkg@beta` and `@scope/pkg@1.2.3` are pinned and must never
/// be updated (installing latest and restoring the pinned spec would leave the
/// disk and the record permanently inconsistent).
fn is_pinned_npm_spec(spec: &str) -> bool {
    let (_, version) = installer::split_npm_spec(spec.trim());
    match version {
        Some(version) => {
            let version = version.trim();
            !version.is_empty() && !version.eq_ignore_ascii_case("latest")
        }
        None => false,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedVersion {
    core: [u64; 3],
    prerelease: Vec<PrereleasePart>,
}

#[derive(Debug, PartialEq, Eq)]
enum PrereleasePart {
    Numeric(u64),
    Alpha(String),
}

/// Strict semver-ish parser: optional `v`, 1–3 numeric core segments, optional
/// prerelease, ignored build metadata. Invalid input returns `None`; the
/// caller treats "invalid" as "not newer" rather than guessing.
fn parse_version(raw: &str) -> Option<ParsedVersion> {
    let raw = raw.trim();
    let raw = raw
        .strip_prefix('v')
        .or_else(|| raw.strip_prefix('V'))
        .unwrap_or(raw);
    // Build metadata never affects precedence.
    let raw = raw.split('+').next().unwrap_or_default();
    let (core_text, prerelease_text) = match raw.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (raw, None),
    };

    let parts: Vec<&str> = core_text.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut core = [0u64; 3];
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        if part.len() > 1 && part.starts_with('0') {
            return None;
        }
        core[index] = part.parse().ok()?;
    }

    let prerelease = match prerelease_text {
        Some(text) => {
            if text.is_empty() {
                return None;
            }
            let mut identifiers = Vec::new();
            for part in text.split('.') {
                if part.is_empty() {
                    return None;
                }
                if part.bytes().all(|byte| byte.is_ascii_digit()) {
                    if part.len() > 1 && part.starts_with('0') {
                        return None;
                    }
                    identifiers.push(PrereleasePart::Numeric(part.parse().ok()?));
                } else {
                    if !part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                    {
                        return None;
                    }
                    identifiers.push(PrereleasePart::Alpha(part.to_string()));
                }
            }
            identifiers
        }
        None => Vec::new(),
    };

    Some(ParsedVersion { core, prerelease })
}

fn compare_prerelease(left: &[PrereleasePart], right: &[PrereleasePart]) -> Ordering {
    match (left.is_empty(), right.is_empty()) {
        (true, true) => Ordering::Equal,
        // A release outranks any prerelease of the same core version.
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            for (left_part, right_part) in left.iter().zip(right) {
                let order = match (left_part, right_part) {
                    (PrereleasePart::Numeric(left), PrereleasePart::Numeric(right)) => {
                        left.cmp(right)
                    }
                    (PrereleasePart::Numeric(_), PrereleasePart::Alpha(_)) => Ordering::Less,
                    (PrereleasePart::Alpha(_), PrereleasePart::Numeric(_)) => Ordering::Greater,
                    (PrereleasePart::Alpha(left), PrereleasePart::Alpha(right)) => left.cmp(right),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
            left.len().cmp(&right.len())
        }
    }
}

fn is_newer_version(current: &str, latest: &str) -> bool {
    let Some(current) = parse_version(current) else {
        return false;
    };
    let Some(latest) = parse_version(latest) else {
        return false;
    };
    current
        .core
        .cmp(&latest.core)
        .then_with(|| compare_prerelease(&current.prerelease, &latest.prerelease))
        == Ordering::Less
}

pub(super) fn auto_update_candidates(
    records: &[InstalledPluginRecord],
) -> Vec<&InstalledPluginRecord> {
    records
        .iter()
        .filter(|record| {
            record.enabled
                && record.source_type == "npm"
                && !record.package_name.trim().is_empty()
                && !is_pinned_npm_spec(&record.spec)
        })
        .collect()
}

pub(super) fn auto_update_due(settings: &PluginSettings, now: DateTime<Utc>) -> bool {
    if !settings.auto_update_enabled {
        return false;
    }
    let Some(last) = settings
        .last_auto_update_check_at
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    let Ok(last) = DateTime::parse_from_rfc3339(last) else {
        return true;
    };
    let interval_hours = settings.auto_update_interval_hours.clamp(1, 720) as i64;
    now.signed_duration_since(last.with_timezone(&Utc)) >= Duration::hours(interval_hours)
}

/// Restore the fields the user owns after `install_plugin` rewrites the record.
///
/// `install_plugin` itself preserves enabled/options when the reinstall is
/// "clean", but it always writes `pkg@latest` as the spec and may prefer the
/// new package's default options. Restoring explicitly keeps the user's spec,
/// options, enabled switch, install time and market origin intact.
fn restore_user_owned_fields(
    updated: &mut InstalledPluginRecord,
    original: &InstalledPluginRecord,
) {
    updated.spec = original.spec.clone();
    updated.options = original.options.clone();
    updated.enabled = original.enabled;
    updated.installed_at = original.installed_at.clone();
    updated.market_id = original.market_id.clone();
}

fn base_item(record: &InstalledPluginRecord) -> PluginUpdateItem {
    let display_name = if !record.display_name.trim().is_empty() {
        record.display_name.clone()
    } else if !record.package_name.trim().is_empty() {
        record.package_name.clone()
    } else {
        record.id.clone()
    };
    PluginUpdateItem {
        plugin_id: record.id.clone(),
        package_name: record.package_name.clone(),
        display_name,
        current_version: record.version.clone(),
        latest_version: String::new(),
        status: String::new(),
        message: String::new(),
    }
}

fn skipped_item(record: &InstalledPluginRecord, message: &str) -> PluginUpdateItem {
    PluginUpdateItem {
        status: STATUS_SKIPPED.to_string(),
        message: message.to_string(),
        ..base_item(record)
    }
}

fn failed_item(
    record: &InstalledPluginRecord,
    latest_version: &str,
    message: &str,
) -> PluginUpdateItem {
    PluginUpdateItem {
        latest_version: latest_version.to_string(),
        status: STATUS_FAILED.to_string(),
        message: message.to_string(),
        ..base_item(record)
    }
}

/// Manual/desktop entry. Full checks record `lastAutoUpdateCheckAt`; a single
/// plugin check does not.
pub async fn check_and_update_plugins(
    manager: &PluginManagerState,
    plugin_id: Option<&str>,
    apply: bool,
    manual: bool,
) -> Result<PluginUpdateSummary, String> {
    let _guard = manager.update_lock.lock().await;
    check_and_update_plugins_locked(manager, plugin_id, apply, manual, true).await
}

/// L0 check-only entry used by the Control API. It never writes settings.
pub async fn check_plugins_read_only(
    manager: &PluginManagerState,
    plugin_id: Option<&str>,
) -> Result<PluginUpdateSummary, String> {
    let _guard = manager.update_lock.lock().await;
    check_and_update_plugins_locked(manager, plugin_id, false, true, false).await
}

/// The caller must hold `manager.update_lock`.
pub(super) async fn check_and_update_plugins_locked(
    manager: &PluginManagerState,
    plugin_id: Option<&str>,
    apply: bool,
    manual: bool,
    record_check_time: bool,
) -> Result<PluginUpdateSummary, String> {
    let settings = store::get_plugin_settings(manager.db())?;
    let records = store::list_installed_plugins(manager.db())?;
    let checked_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

    let targets: Vec<&InstalledPluginRecord> = if !manual {
        auto_update_candidates(&records)
    } else if let Some(id) = plugin_id {
        let record = records
            .iter()
            .find(|record| record.id == id)
            .ok_or_else(|| format!("Plugin '{id}' is not installed"))?;
        vec![record]
    } else {
        records.iter().collect()
    };

    let needs_npm = targets.iter().any(|record| record.source_type == "npm");
    let client = if needs_npm {
        Some(manager.http_client().await?)
    } else {
        None
    };

    let mut items = Vec::with_capacity(targets.len());
    for record in targets {
        if record.source_type == "local" {
            items.push(skipped_item(record, SKIP_LOCAL));
            continue;
        }
        if record.source_type == "npm" && record.package_name.trim().is_empty() {
            items.push(skipped_item(record, SKIP_NO_PACKAGE));
            continue;
        }
        if is_pinned_npm_spec(&record.spec) {
            items.push(skipped_item(record, SKIP_PINNED));
            continue;
        }

        match record.source_type.as_str() {
            "npm" => {
                let Some(client) = client.as_ref() else {
                    items.push(failed_item(
                        record,
                        "",
                        "The npm HTTP client is unavailable",
                    ));
                    continue;
                };
                let latest =
                    match market::fetch_npm_latest_version(
                        client,
                        &settings.npm_registry,
                        &record.package_name,
                    )
                    .await
                    {
                        Ok(latest) => latest,
                        Err(error) => {
                            record_failure(manager, record, &error);
                            items.push(failed_item(record, "", &error));
                            continue;
                        }
                    };

                if !is_newer_version(&record.version, &latest) {
                    items.push(PluginUpdateItem {
                        latest_version: latest,
                        status: STATUS_UP_TO_DATE.to_string(),
                        ..base_item(record)
                    });
                    continue;
                }
                if !apply {
                    items.push(PluginUpdateItem {
                        latest_version: latest,
                        status: STATUS_AVAILABLE.to_string(),
                        ..base_item(record)
                    });
                    continue;
                }

                match apply_npm_update(manager, record).await {
                    Ok(saved) => items.push(PluginUpdateItem {
                        latest_version: saved.version.clone(),
                        status: STATUS_UPDATED.to_string(),
                        ..base_item(record)
                    }),
                    Err(error) => {
                        record_failure(manager, record, &error);
                        items.push(failed_item(record, &latest, &error));
                    }
                }
            }
            "github" => {
                // A full check has no registry version to compare; only a
                // single-plugin apply may refresh the source.
                if !apply || plugin_id.is_none() {
                    items.push(skipped_item(record, SKIP_GITHUB));
                    continue;
                }
                match apply_github_update(manager, record).await {
                    Ok(saved) => items.push(PluginUpdateItem {
                        latest_version: saved.version.clone(),
                        status: STATUS_UPDATED.to_string(),
                        message: "Reinstalled from source".to_string(),
                        ..base_item(record)
                    }),
                    Err(error) => {
                        record_failure(manager, record, &error);
                        items.push(failed_item(record, "", &error));
                    }
                }
            }
            _ => items.push(skipped_item(record, SKIP_UNSUPPORTED)),
        }
    }

    let updated_count = items
        .iter()
        .filter(|item| item.status == STATUS_UPDATED)
        .count();
    let available_count = items
        .iter()
        .filter(|item| item.status == STATUS_AVAILABLE)
        .count();
    let failed_count = items
        .iter()
        .filter(|item| item.status == STATUS_FAILED)
        .count();

    if updated_count > 0 {
        commands::reload_runtimes(manager).await;
    }

    if record_check_time && plugin_id.is_none() {
        if let Err(error) = store::set_last_auto_update_check_at(manager.db(), &checked_at) {
            log::warn!("failed to record the plugin update check time: {error}");
        }
    }

    Ok(PluginUpdateSummary {
        checked_at,
        updated_count,
        available_count,
        failed_count,
        items,
    })
}

async fn apply_npm_update(
    manager: &PluginManagerState,
    original: &InstalledPluginRecord,
) -> Result<InstalledPluginRecord, String> {
    let installed = installer::install_plugin(
        manager.db(),
        manager.paths(),
        manager.bun(),
        &format!("{}@latest", original.package_name),
        original.market_id.clone(),
    )
    .await?;
    let mut restored = installed;
    restore_user_owned_fields(&mut restored, original);
    store::upsert_installed_plugin(manager.db(), &restored)
}

async fn apply_github_update(
    manager: &PluginManagerState,
    original: &InstalledPluginRecord,
) -> Result<InstalledPluginRecord, String> {
    let installed = installer::install_plugin(
        manager.db(),
        manager.paths(),
        manager.bun(),
        &original.spec,
        original.market_id.clone(),
    )
    .await?;
    let mut restored = installed;
    restore_user_owned_fields(&mut restored, original);
    store::upsert_installed_plugin(manager.db(), &restored)
}

fn record_failure(
    manager: &PluginManagerState,
    record: &InstalledPluginRecord,
    error: &str,
) {
    log::warn!("plugin update failed for {}: {error}", record.id);
    manager
        .host()
        .record_log("updater", "error", &format!("{}: {error}", record.id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDbState;
    use crate::plugins::PluginManager;
    use chrono::TimeZone;
    use serde_json::json;

    fn test_db() -> SqliteDbState {
        SqliteDbState::in_memory_for_test().expect("in-memory database")
    }

    fn record(id: &str, source: &str, spec: &str, version: &str) -> InstalledPluginRecord {
        InstalledPluginRecord {
            id: id.to_string(),
            source_type: source.to_string(),
            spec: spec.to_string(),
            package_name: if source == "npm" {
                id.to_string()
            } else {
                String::new()
            },
            version: version.to_string(),
            enabled: true,
            ..InstalledPluginRecord::default()
        }
    }

    #[test]
    fn pinned_specs_are_only_versions_and_dist_tags() {
        assert!(!is_pinned_npm_spec("demo"));
        assert!(!is_pinned_npm_spec("@scope/demo"));
        assert!(!is_pinned_npm_spec("demo@latest"));
        assert!(!is_pinned_npm_spec("@scope/demo@LATEST"));
        assert!(!is_pinned_npm_spec("demo@"));

        assert!(is_pinned_npm_spec("demo@1.2.3"));
        assert!(is_pinned_npm_spec("@scope/demo@^1.0.0"));
        assert!(is_pinned_npm_spec("demo@~0.2.0"));
        assert!(is_pinned_npm_spec("demo@beta"));
    }

    #[test]
    fn semver_comparison_covers_prereleases_and_invalid_input() {
        assert!(is_newer_version("1.2.3", "1.2.4"));
        assert!(!is_newer_version("1.2.3", "1.2.3"));
        assert!(!is_newer_version("1.2.4", "1.2.3"));

        assert!(is_newer_version("1.0.0-alpha", "1.0.0-alpha.1"));
        assert!(is_newer_version("1.0.0-alpha.1", "1.0.0-beta"));
        assert!(is_newer_version("1.0.0-beta", "1.0.0-rc"));
        assert!(is_newer_version("1.0.0-rc.1", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "1.0.0-rc.1"));

        assert!(!is_newer_version("1.0.0+build", "1.0.0"));
        assert!(is_newer_version("1.0.0+build", "1.0.1+build"));
        assert!(is_newer_version("v1.2.3", "1.2.4"));
        assert!(is_newer_version("1.2", "1.2.1"));
        assert!(is_newer_version("1", "1.0.1"));

        assert!(!is_newer_version("", "1.0.0"));
        assert!(!is_newer_version("latest", "1.0.0"));
        assert!(!is_newer_version("1.0.0.0", "1.0.1"));
        assert!(!is_newer_version("01.2.3", "1.2.4"));
    }

    #[test]
    fn candidates_are_enabled_unpinned_npm_records_only() {
        let mut records = vec![
            record("enabled", "npm", "enabled@latest", "1.0.0"),
            record("disabled", "npm", "disabled", "1.0.0"),
            record("pinned", "npm", "pinned@1.0.0", "1.0.0"),
            record("github", "github", "github:owner/repo", "1.0.0"),
            record("local", "local", "/tmp/demo", "1.0.0"),
        ];
        records[1].enabled = false;
        let mut empty = record("empty", "npm", "empty", "1.0.0");
        empty.package_name.clear();
        records.push(empty);

        let ids: Vec<&str> = auto_update_candidates(&records)
            .into_iter()
            .map(|record| record.id.as_str())
            .collect();
        assert_eq!(ids, vec!["enabled"]);
    }

    #[test]
    fn due_checks_respect_the_enabled_switch_and_interval() {
        let now = Utc.with_ymd_and_hms(2026, 11, 28, 12, 0, 0).unwrap();
        let mut settings = PluginSettings::default();
        assert!(!auto_update_due(&settings, now));

        settings.auto_update_enabled = true;
        assert!(auto_update_due(&settings, now));

        settings.last_auto_update_check_at = Some("2026-11-28T12:00:00Z".to_string());
        settings.auto_update_interval_hours = 24;
        assert!(!auto_update_due(&settings, now));

        settings.last_auto_update_check_at = Some("2026-11-27T12:00:00Z".to_string());
        assert!(auto_update_due(&settings, now));

        settings.last_auto_update_check_at = Some("not-a-date".to_string());
        assert!(auto_update_due(&settings, now));

        settings.last_auto_update_check_at = Some(String::new());
        assert!(auto_update_due(&settings, now));

        settings.last_auto_update_check_at = Some("2026-11-29T12:00:00Z".to_string());
        assert!(!auto_update_due(&settings, now));

        settings.last_auto_update_check_at = Some("2026-11-28T11:30:00Z".to_string());
        settings.auto_update_interval_hours = 0;
        assert!(!auto_update_due(&settings, now));

        settings.auto_update_interval_hours = 721;
        settings.last_auto_update_check_at = Some("2026-10-01T12:00:00Z".to_string());
        assert!(auto_update_due(&settings, now));
    }

    #[test]
    fn restoring_user_fields_keeps_the_new_package_metadata() {
        let original = InstalledPluginRecord {
            spec: "demo".to_string(),
            options: json!({ "region": "cn" }),
            enabled: false,
            installed_at: "2026-01-01T00:00:00Z".to_string(),
            market_id: Some("magpie-community".to_string()),
            version: "1.0.0".to_string(),
            ..InstalledPluginRecord::default()
        };
        let mut updated = InstalledPluginRecord {
            spec: "demo@latest".to_string(),
            options: json!({ "region": "global" }),
            enabled: true,
            installed_at: "2026-11-28T00:00:00Z".to_string(),
            market_id: None,
            version: "1.1.0".to_string(),
            display_name: "Demo".to_string(),
            ..InstalledPluginRecord::default()
        };

        restore_user_owned_fields(&mut updated, &original);

        assert_eq!(updated.spec, "demo");
        assert_eq!(updated.options, json!({ "region": "cn" }));
        assert!(!updated.enabled);
        assert_eq!(updated.installed_at, "2026-01-01T00:00:00Z");
        assert_eq!(updated.market_id.as_deref(), Some("magpie-community"));
        assert_eq!(updated.version, "1.1.0");
        assert_eq!(updated.display_name, "Demo");
    }

    #[test]
    fn summary_serializes_with_camel_case_keys() {
        let summary = PluginUpdateSummary {
            checked_at: "2026-11-28T12:00:00Z".to_string(),
            updated_count: 0,
            available_count: 0,
            failed_count: 0,
            items: vec![PluginUpdateItem {
                plugin_id: "demo".to_string(),
                package_name: "demo".to_string(),
                display_name: "Demo".to_string(),
                current_version: "1.0.0".to_string(),
                latest_version: "1.1.0".to_string(),
                status: STATUS_AVAILABLE.to_string(),
                message: String::new(),
            }],
        };
        let value = serde_json::to_value(&summary).unwrap();
        assert_eq!(value["checkedAt"], "2026-11-28T12:00:00Z");
        assert_eq!(value["updatedCount"], 0);
        assert_eq!(value["items"][0]["pluginId"], "demo");
        assert_eq!(value["items"][0]["currentVersion"], "1.0.0");
        assert_eq!(value["items"][0]["latestVersion"], "1.1.0");
        assert_eq!(value["items"][0]["status"], "available");
    }

    #[tokio::test]
    async fn empty_full_checks_write_the_timestamp_without_network() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let manager = PluginManager::new(db.clone(), dir.path().to_path_buf());

        let summary = check_and_update_plugins(&manager, None, false, true)
            .await
            .unwrap();
        assert!(summary.items.is_empty());
        assert_eq!(summary.updated_count, 0);
        assert_eq!(
            store::get_plugin_settings(&db)
                .unwrap()
                .last_auto_update_check_at
                .as_deref(),
            Some(summary.checked_at.as_str())
        );
    }

    #[tokio::test]
    async fn check_only_does_not_write_the_timestamp() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let manager = PluginManager::new(db.clone(), dir.path().to_path_buf());

        check_plugins_read_only(&manager, None).await.unwrap();
        assert!(store::get_plugin_settings(&db)
            .unwrap()
            .last_auto_update_check_at
            .is_none());

        let mut settings = PluginSettings::default();
        settings.auto_update_enabled = true;
        store::save_plugin_settings(&db, &settings).unwrap();
        check_plugins_read_only(&manager, None).await.unwrap();
        assert!(store::get_plugin_settings(&db)
            .unwrap()
            .last_auto_update_check_at
            .is_none());
    }

    #[tokio::test]
    async fn a_missing_plugin_id_is_an_error() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let manager = PluginManager::new(db, dir.path().to_path_buf());

        let error = check_and_update_plugins(&manager, Some("missing"), false, true)
            .await
            .unwrap_err();
        assert!(error.contains("not installed"));
    }
}
