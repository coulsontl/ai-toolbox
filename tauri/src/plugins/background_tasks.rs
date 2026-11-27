//! Background maintenance for plugin accounts.
//!
//! The shared auth-refresh scheduler calls [`run_plugin_background_pass`] on
//! startup and every 15 minutes. The pass is deliberately lazy: it only asks
//! the Bun host for anything when the auth file has an OAuth account about to
//! expire, or when an account opted into the daily check-in is due. Otherwise
//! no Bun runtime is downloaded or started.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, FixedOffset, Utc};
use serde_json::{json, Value};

use crate::db::SqliteDbState;

use super::bun_host::auth::{self, PluginAccountSummary};
use super::store;
use super::types::{InstalledPluginRecord, PluginAccountMetaRecord};
use super::PluginManager;

/// Refresh an OAuth token once it is this close to expiry (or already past).
pub const REFRESH_LEAD_MS: i64 = 15 * 60 * 1000;
/// A failed check-in is retried after this long, within the same Beijing day.
pub const CHECKIN_RETRY_MINUTES: i64 = 30;

fn beijing_zone() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("UTC+8 is a valid fixed offset")
}

fn beijing_now() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&beijing_zone())
}

fn beijing_day(now: DateTime<FixedOffset>) -> String {
    now.format("%Y-%m-%d").to_string()
}

/// One startup/interval pass: renew due OAuth tokens and run due check-ins.
pub async fn run_plugin_background_pass(manager: &PluginManager) {
    let accounts = match auth::list_accounts(manager.db(), manager.paths()) {
        Ok(accounts) => accounts,
        Err(error) => {
            log::debug!("plugin background pass could not read plugin-auth.json: {error}");
            return;
        }
    };
    if accounts.is_empty() {
        return;
    }
    let records = store::list_installed_plugins(manager.db()).unwrap_or_default();
    let metas = store::list_account_metas(manager.db()).unwrap_or_default();
    let enabled_plugins: HashSet<String> = records
        .iter()
        .filter(|record| record.enabled)
        .map(|record| record.id.clone())
        .collect();
    let provider_plugins = provider_plugin_ids(&records);
    let meta_by_account: HashMap<&str, &PluginAccountMetaRecord> =
        metas.iter().map(|meta| (meta.id.as_str(), meta)).collect();
    let now = beijing_now();

    refresh_due_accounts(
        manager,
        &accounts,
        &meta_by_account,
        &provider_plugins,
        &enabled_plugins,
        now,
    )
    .await;
    checkin_due_accounts(
        manager,
        &accounts,
        &metas,
        &provider_plugins,
        &enabled_plugins,
        now,
    )
    .await;
}

async fn refresh_due_accounts(
    manager: &PluginManager,
    accounts: &[PluginAccountSummary],
    meta_by_account: &HashMap<&str, &PluginAccountMetaRecord>,
    provider_plugins: &HashMap<String, String>,
    enabled_plugins: &HashSet<String>,
    now: DateTime<FixedOffset>,
) {
    let now_ms = now.timestamp_millis();
    for account in accounts {
        if !refresh_due(account, now_ms) {
            continue;
        }
        let meta = meta_by_account.get(account.account.as_str()).copied();
        if !account_plugin_enabled(meta, &account.provider, provider_plugins, enabled_plugins) {
            continue;
        }
        let params = json!({
            "provider": account.provider,
            "account": account.account,
        });
        // The host honours `auth.keepAlive !== false` and does nothing when the
        // token is not actually due for the plugin's own lead window.
        match manager.host().keep_signed_in(params).await {
            Ok(value) => log::debug!(
                "plugin background pass renewed {}: {}",
                account.account,
                value
                    .get("renewed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            ),
            Err(error) => log::debug!(
                "plugin background pass could not renew {}: {error}",
                account.account
            ),
        }
    }
}

async fn checkin_due_accounts(
    manager: &PluginManager,
    accounts: &[PluginAccountSummary],
    metas: &[PluginAccountMetaRecord],
    provider_plugins: &HashMap<String, String>,
    enabled_plugins: &HashSet<String>,
    now: DateTime<FixedOffset>,
) {
    for meta in metas {
        if !meta.auto_checkin || !meta.enabled {
            continue;
        }
        let provider = provider_for_meta(meta);
        if provider.is_empty() {
            continue;
        }
        if !accounts
            .iter()
            .any(|account| account.account == meta.id && account.provider == provider)
        {
            continue;
        }
        if !account_plugin_enabled(Some(meta), provider, provider_plugins, enabled_plugins) {
            continue;
        }
        if !checkin_due(meta, now) {
            continue;
        }
        let params = json!({ "provider": provider, "account": meta.id });
        match manager.host().checkin(params).await {
            Ok(result) => {
                if let Err(error) =
                    record_checkin_result(manager.db(), provider, Some(&meta.id), &result)
                {
                    log::warn!(
                        "plugin background pass could not save the check-in of {}: {error}",
                        meta.id
                    );
                }
            }
            Err(error) => {
                let failed = json!({ "outcome": "failed", "message": error });
                if let Err(save_error) =
                    record_checkin_result(manager.db(), provider, Some(&meta.id), &failed)
                {
                    log::warn!(
                        "plugin background pass could not save the failed check-in of {}: {save_error}",
                        meta.id
                    );
                }
            }
        }
    }
}

fn provider_for_meta(meta: &PluginAccountMetaRecord) -> &str {
    let provider = meta.provider_id.trim();
    if !provider.is_empty() {
        return provider;
    }
    meta.id.split('#').next().unwrap_or_default()
}

/// Whether an account's plugin is installed and enabled. Accounts whose
/// provider cannot be tied to an installed plugin are left alone: no host
/// could serve them.
fn account_plugin_enabled(
    meta: Option<&PluginAccountMetaRecord>,
    provider: &str,
    provider_plugins: &HashMap<String, String>,
    enabled_plugins: &HashSet<String>,
) -> bool {
    if meta.is_some_and(|meta| !meta.enabled) {
        return false;
    }
    let plugin_id = meta
        .map(|meta| meta.plugin_id.trim())
        .filter(|plugin_id| !plugin_id.is_empty())
        .map(str::to_string)
        .or_else(|| provider_plugins.get(provider).cloned());
    plugin_id.is_some_and(|plugin_id| enabled_plugins.contains(&plugin_id))
}

/// Map every provider one installed plugin exposes to that plugin's id. The
/// first plugin wins, matching `providers` declarations' display order.
fn provider_plugin_ids(records: &[InstalledPluginRecord]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for record in records {
        for provider in &record.providers {
            map.entry(provider.clone())
                .or_insert_with(|| record.id.clone());
        }
    }
    map
}

/// An enabled OAuth account with an expiry that is due for renewal.
fn refresh_due(account: &PluginAccountSummary, now_ms: i64) -> bool {
    account.enabled
        && account.auth_type == "oauth"
        && account.has_refresh
        && account
            .expires_at
            .filter(|expires| *expires > 0)
            .is_some_and(|expires| expires - now_ms <= REFRESH_LEAD_MS)
}

/// Whether the daily check-in still has to be asked for in this Beijing day.
fn checkin_due(meta: &PluginAccountMetaRecord, now: DateTime<FixedOffset>) -> bool {
    let today = beijing_day(now);
    if meta.last_checkin_day.as_deref() != Some(today.as_str()) {
        return true;
    }
    if let Some(until) = meta
        .last_checkin_until
        .as_deref()
        .map(str::trim)
        .filter(|until| !until.is_empty())
    {
        if let Ok(until) = DateTime::parse_from_rfc3339(until) {
            // The vendor's own day has not turned yet.
            if now < until {
                return false;
            }
            return true;
        }
    }
    match meta.last_checkin_outcome.as_deref() {
        Some("failed") => meta
            .last_checkin_at
            .as_deref()
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
            .map(|at| now.signed_duration_since(at).num_minutes() >= CHECKIN_RETRY_MINUTES)
            .unwrap_or(true),
        Some("claimed" | "done" | "ineligible" | "inactive" | "captcha") => false,
        _ => true,
    }
}

/// Persist one manual check-in result onto the account's metadata. The record
/// is created when the account never had one (a plugin signed in from the
/// host without the UI touching its row yet).
pub fn record_checkin_result(
    db: &SqliteDbState,
    provider: &str,
    account: Option<&str>,
    result: &Value,
) -> Result<PluginAccountMetaRecord, String> {
    let provider = provider.trim();
    if provider.is_empty() {
        return Err("A provider is required to save a check-in".to_string());
    }
    let account_key = account
        .map(str::trim)
        .filter(|account| !account.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| provider.to_string());

    let mut record = find_account_meta(db, &account_key, provider)?;
    apply_checkin_result(&mut record, result, beijing_now());
    store::upsert_account_meta(db, &record)
}

fn find_account_meta(
    db: &SqliteDbState,
    account_key: &str,
    provider: &str,
) -> Result<PluginAccountMetaRecord, String> {
    let metas = store::list_account_metas(db)?;
    if let Some(record) = metas.iter().find(|meta| meta.id == account_key) {
        return Ok(record.clone());
    }
    let plugin_id =
        resolve_plugin_for_provider(db, provider).unwrap_or_else(|| provider.to_string());
    Ok(PluginAccountMetaRecord {
        id: account_key.to_string(),
        plugin_id,
        provider_id: provider.to_string(),
        ..PluginAccountMetaRecord::default()
    })
}

fn resolve_plugin_for_provider(db: &SqliteDbState, provider: &str) -> Option<String> {
    store::list_installed_plugins(db)
        .ok()?
        .into_iter()
        .find(|record| record.providers.iter().any(|value| value == provider))
        .map(|record| record.id)
}

fn apply_checkin_result(
    record: &mut PluginAccountMetaRecord,
    result: &Value,
    now: DateTime<FixedOffset>,
) {
    let outcome = result
        .get("outcome")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|outcome| !outcome.is_empty())
        .unwrap_or("failed")
        .to_string();
    record.last_checkin_day = Some(beijing_day(now));
    record.last_checkin_outcome = Some(outcome);
    record.last_checkin_at = Some(now.to_rfc3339());
    record.last_checkin_message = result
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string);
    record.last_checkin_credit = result.get("credit").and_then(Value::as_f64);
    record.last_checkin_streak = result.get("streak").and_then(Value::as_u64).or_else(|| {
        result
            .get("streak")
            .and_then(Value::as_f64)
            .filter(|streak| *streak >= 0.0)
            .map(|streak| streak.round() as u64)
    });
    record.last_checkin_until = result
        .get("until")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|until| !until.is_empty())
        .map(str::to_string);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDbState;
    use crate::plugins::types::PluginKind;

    fn meta() -> PluginAccountMetaRecord {
        PluginAccountMetaRecord {
            id: "demo".to_string(),
            plugin_id: "demo-plugin".to_string(),
            provider_id: "demo".to_string(),
            enabled: true,
            auto_checkin: true,
            ..PluginAccountMetaRecord::default()
        }
    }

    #[test]
    fn only_near_expiry_oauth_accounts_are_renewed() {
        let now_ms = 1_800_000_000_000i64;
        let account = PluginAccountSummary {
            account: "demo".to_string(),
            provider: "demo".to_string(),
            auth_type: "oauth".to_string(),
            has_access: true,
            has_refresh: true,
            expires_at: Some(now_ms + 5 * 60 * 1000),
            enabled: true,
            ..PluginAccountSummary::default()
        };
        assert!(refresh_due(&account, now_ms));

        let mut later = account.clone();
        later.expires_at = Some(now_ms + 60 * 60 * 1000);
        assert!(!refresh_due(&later, now_ms));

        let mut expired = account.clone();
        expired.expires_at = Some(now_ms - 60 * 60 * 1000);
        assert!(refresh_due(&expired, now_ms));

        let mut no_refresh = account.clone();
        no_refresh.has_refresh = false;
        assert!(!refresh_due(&no_refresh, now_ms));

        let mut disabled = account.clone();
        disabled.enabled = false;
        assert!(!refresh_due(&disabled, now_ms));

        let mut api_key = account;
        api_key.auth_type = "api".to_string();
        assert!(!refresh_due(&api_key, now_ms));
    }

    #[test]
    fn checkin_is_due_once_per_beijing_day_and_retries_failures() {
        let now = beijing_now();
        let today = beijing_day(now);

        let mut record = meta();
        assert!(checkin_due(&record, now));

        record.last_checkin_day = Some(today.clone());
        record.last_checkin_outcome = Some("claimed".to_string());
        record.last_checkin_at = Some(now.to_rfc3339());
        assert!(!checkin_due(&record, now));

        record.last_checkin_outcome = Some("failed".to_string());
        assert!(!checkin_due(&record, now));
        record.last_checkin_at = Some((now - chrono::Duration::minutes(31)).to_rfc3339());
        assert!(checkin_due(&record, now));

        // The vendor's own day boundary postpones the next ask.
        record.last_checkin_until = Some((now + chrono::Duration::hours(1)).to_rfc3339());
        assert!(!checkin_due(&record, now));
        record.last_checkin_until = Some((now - chrono::Duration::minutes(1)).to_rfc3339());
        assert!(checkin_due(&record, now));

        record.last_checkin_day = Some("2000-01-01".to_string());
        assert!(checkin_due(&record, now));
    }

    #[test]
    fn provider_mapping_uses_the_first_plugin_that_declares_it() {
        let records = vec![
            InstalledPluginRecord {
                id: "first".to_string(),
                providers: vec!["demo".to_string()],
                ..InstalledPluginRecord::default()
            },
            InstalledPluginRecord {
                id: "second".to_string(),
                providers: vec!["demo".to_string(), "other".to_string()],
                ..InstalledPluginRecord::default()
            },
        ];
        let map = provider_plugin_ids(&records);
        assert_eq!(map.get("demo").map(String::as_str), Some("first"));
        assert_eq!(map.get("other").map(String::as_str), Some("second"));
    }

    #[test]
    fn manual_checkin_results_are_persisted() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        store::upsert_installed_plugin(
            &db,
            &InstalledPluginRecord {
                id: "demo-plugin".to_string(),
                package_name: "demo-plugin".to_string(),
                kind: PluginKind::Provider,
                enabled: true,
                providers: vec!["demo".to_string()],
                ..InstalledPluginRecord::default()
            },
        )
        .unwrap();

        let saved = record_checkin_result(
            &db,
            "demo",
            None,
            &json!({
                "outcome": "claimed",
                "credit": 12.5,
                "streak": 3,
                "message": "checked in",
                "until": "2026-11-27T02:00:00Z"
            }),
        )
        .unwrap();
        assert_eq!(saved.id, "demo");
        assert_eq!(saved.plugin_id, "demo-plugin");
        assert_eq!(saved.last_checkin_outcome.as_deref(), Some("claimed"));
        assert_eq!(saved.last_checkin_credit, Some(12.5));
        assert_eq!(saved.last_checkin_streak, Some(3));
        assert_eq!(saved.last_checkin_message.as_deref(), Some("checked in"));
        assert!(saved.last_checkin_day.is_some());
        assert!(saved.last_checkin_at.is_some());
        assert_eq!(
            saved.last_checkin_until.as_deref(),
            Some("2026-11-27T02:00:00Z")
        );
        assert!(!saved.auto_checkin);

        // A second result updates the same row instead of adding one.
        record_checkin_result(
            &db,
            "demo",
            Some("demo"),
            &json!({"outcome": "failed", "message": "offline"}),
        )
        .unwrap();
        let listed = store::list_account_metas(&db).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].last_checkin_outcome.as_deref(), Some("failed"));
    }

    #[test]
    fn accounts_need_an_enabled_plugin() {
        let mut provider_plugins = HashMap::new();
        provider_plugins.insert("demo".to_string(), "demo-plugin".to_string());
        let enabled: HashSet<String> = ["demo-plugin".to_string()].into_iter().collect();
        let mut record = meta();
        assert!(account_plugin_enabled(
            Some(&record),
            "demo",
            &provider_plugins,
            &enabled
        ));

        record.enabled = false;
        assert!(!account_plugin_enabled(
            Some(&record),
            "demo",
            &provider_plugins,
            &enabled
        ));

        let disabled_plugins: HashSet<String> = HashSet::new();
        assert!(!account_plugin_enabled(
            Some(&meta()),
            "demo",
            &provider_plugins,
            &disabled_plugins
        ));
        assert!(!account_plugin_enabled(
            None,
            "unknown",
            &provider_plugins,
            &enabled
        ));
    }
}
