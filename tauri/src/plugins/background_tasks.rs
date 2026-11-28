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
use tauri::{AppHandle, Emitter};

use crate::db::SqliteDbState;

use super::bun_host::auth::{self, PluginAccountSummary};
use super::gateway_bridge::PluginProviderRow;
use super::store;
use super::types::{InstalledPluginRecord, PluginAccountMetaRecord};
use super::{updater, PluginManager, PluginManagerState};

/// Refresh an OAuth token once it is this close to expiry (or already past).
pub const REFRESH_LEAD_MS: i64 = 15 * 60 * 1000;
/// Bounds a plugin's declared `renewLead`, so a broken manifest can neither
/// renew every minute nor defer renewal for days.
pub const MIN_REFRESH_LEAD_MS: i64 = 60_000;
pub const MAX_REFRESH_LEAD_MS: i64 = 86_400_000;
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

/// One startup/interval pass: renew due OAuth tokens, run due check-ins, and
/// run a due plugin update check. The account portion may be empty; the update
/// check is independent of plugin accounts.
pub async fn run_plugin_background_pass(manager: &PluginManagerState, app: Option<&AppHandle>) {
    let accounts = match auth::list_accounts(manager.db(), manager.paths()) {
        Ok(accounts) => accounts,
        Err(error) => {
            log::debug!("plugin background pass could not read plugin-auth.json: {error}");
            Vec::new()
        }
    };
    if !accounts.is_empty() {
        let records = store::list_installed_plugins(manager.db()).unwrap_or_default();
        let metas = store::list_account_metas(manager.db()).unwrap_or_default();
        let enabled_plugins: HashSet<String> = records
            .iter()
            .filter(|record| record.enabled)
            .map(|record| record.id.clone())
            .collect();
        let provider_plugins = provider_plugin_ids(&records);
        let provider_auto_checkin =
            store::resolve_provider_auto_checkin(manager.db()).unwrap_or_default();
        let meta_by_account: HashMap<&str, &PluginAccountMetaRecord> =
            metas.iter().map(|meta| (meta.id.as_str(), meta)).collect();
        let now = beijing_now();
        // Reading the proxy only touches SQLite; the host is still started
        // lazily by the first RPC that actually needs it.
        let proxy = manager.proxy_url().await.unwrap_or_default();
        // Renewal leads come from the last provider listing when one is
        // cached. Never ask the host for a listing here: that would start Bun
        // before any account is due.
        let provider_rows = manager.gateway().cached_provider_rows();

        refresh_due_accounts(
            manager,
            &accounts,
            &meta_by_account,
            &provider_plugins,
            &enabled_plugins,
            &provider_rows,
            &proxy,
            now,
        )
        .await;
        checkin_due_accounts(
            manager,
            &accounts,
            &meta_by_account,
            &provider_auto_checkin,
            &provider_plugins,
            &enabled_plugins,
            &proxy,
            now,
        )
        .await;
    }

    maybe_auto_update_plugins(manager, app).await;
}

/// Run one due auto-update check. Disabled, not-due, or candidate-free passes
/// return before any HTTP request or Bun start.
async fn maybe_auto_update_plugins(manager: &PluginManagerState, app: Option<&AppHandle>) {
    let settings = match store::get_plugin_settings(manager.db()) {
        Ok(settings) => settings,
        Err(error) => {
            log::warn!("plugin auto-update could not read settings: {error}");
            return;
        }
    };
    if !settings.auto_update_enabled {
        return;
    }
    if !updater::auto_update_due(&settings, Utc::now()) {
        return;
    }
    let records = match store::list_installed_plugins(manager.db()) {
        Ok(records) => records,
        Err(error) => {
            log::warn!("plugin auto-update could not read installed plugins: {error}");
            return;
        }
    };
    if updater::auto_update_candidates(&records).is_empty() {
        return;
    }

    let Ok(_update_guard) = manager.update_lock.try_lock() else {
        return;
    };
    match updater::check_and_update_plugins_locked(manager, None, true, false, true).await {
        Ok(summary) => {
            if summary.updated_count == 0 {
                return;
            }
            let updated_ids: Vec<&str> = summary
                .items
                .iter()
                .filter(|item| item.status == "updated")
                .map(|item| item.plugin_id.as_str())
                .collect();
            if let Some(app) = app {
                let _ = app.emit(
                    "plugins-updated",
                    json!({
                        "updatedCount": summary.updated_count,
                        "updatedIds": updated_ids,
                    }),
                );
            } else {
                log::info!(
                    "plugin auto-update completed without an AppHandle: {} updated",
                    summary.updated_count
                );
            }
        }
        Err(error) => {
            log::warn!("plugin auto-update pass failed: {error}");
            manager.host().record_log("updater", "error", &error);
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn refresh_due_accounts(
    manager: &PluginManager,
    accounts: &[PluginAccountSummary],
    meta_by_account: &HashMap<&str, &PluginAccountMetaRecord>,
    provider_plugins: &HashMap<String, String>,
    enabled_plugins: &HashSet<String>,
    provider_rows: &[PluginProviderRow],
    proxy: &str,
    now: DateTime<FixedOffset>,
) {
    let now_ms = now.timestamp_millis();
    for account in accounts {
        let lead_ms = renew_lead_ms_for(&account.provider, provider_rows);
        if !refresh_due(account, now_ms, lead_ms) {
            continue;
        }
        let meta = meta_by_account.get(account.account.as_str()).copied();
        if !account_plugin_enabled(meta, &account.provider, provider_plugins, enabled_plugins) {
            continue;
        }
        let params = json!({
            "provider": account.provider,
            "account": account.account,
            "proxy": proxy,
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

#[allow(clippy::too_many_arguments)]
async fn checkin_due_accounts(
    manager: &PluginManager,
    accounts: &[PluginAccountSummary],
    meta_by_account: &HashMap<&str, &PluginAccountMetaRecord>,
    provider_auto_checkin: &HashMap<String, bool>,
    provider_plugins: &HashMap<String, String>,
    enabled_plugins: &HashSet<String>,
    proxy: &str,
    now: DateTime<FixedOffset>,
) {
    for (provider, account_key) in due_checkin_accounts(
        accounts,
        meta_by_account,
        provider_auto_checkin,
        provider_plugins,
        enabled_plugins,
        now,
    ) {
        let params = json!({ "provider": provider, "account": account_key, "proxy": proxy });
        match manager.host().checkin(params).await {
            Ok(result) => {
                if let Err(error) =
                    record_checkin_result(manager.db(), &provider, Some(&account_key), &result)
                {
                    log::warn!(
                        "plugin background pass could not save the check-in of {account_key}: {error}"
                    );
                }
            }
            Err(error) => {
                let failed = json!({ "outcome": "failed", "message": error });
                if let Err(save_error) =
                    record_checkin_result(manager.db(), &provider, Some(&account_key), &failed)
                {
                    log::warn!(
                        "plugin background pass could not save the failed check-in of {account_key}: {save_error}"
                    );
                }
            }
        }
    }
}

/// Accounts the daily pass must check in now.
///
/// The channel switch is the only gate: when a provider is on, every enabled
/// account of that provider is due independently (each keeps its own
/// `last_checkin_*` bookkeeping). An off or missing channel switch returns no
/// candidate, so the caller never starts Bun for a channel nobody opted into.
fn due_checkin_accounts(
    accounts: &[PluginAccountSummary],
    meta_by_account: &HashMap<&str, &PluginAccountMetaRecord>,
    provider_auto_checkin: &HashMap<String, bool>,
    provider_plugins: &HashMap<String, String>,
    enabled_plugins: &HashSet<String>,
    now: DateTime<FixedOffset>,
) -> Vec<(String, String)> {
    let mut due = Vec::new();
    for account in accounts {
        if !account.enabled {
            continue;
        }
        if provider_auto_checkin
            .get(account.provider.as_str())
            .copied()
            != Some(true)
        {
            continue;
        }
        let meta = meta_by_account.get(account.account.as_str()).copied();
        if !account_plugin_enabled(meta, &account.provider, provider_plugins, enabled_plugins) {
            continue;
        }
        let synthetic;
        let due_meta = if let Some(meta) = meta {
            meta
        } else {
            // An account signed in directly by the host may have no metadata
            // row yet; it starts due and the result creates the row.
            synthetic = PluginAccountMetaRecord {
                id: account.account.clone(),
                provider_id: account.provider.clone(),
                ..PluginAccountMetaRecord::default()
            };
            &synthetic
        };
        if !checkin_due(due_meta, now) {
            continue;
        }
        due.push((account.provider.clone(), account.account.clone()));
    }
    due
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
fn refresh_due(account: &PluginAccountSummary, now_ms: i64, lead_ms: i64) -> bool {
    account.enabled
        && account.auth_type == "oauth"
        && account.has_refresh
        && account
            .expires_at
            .filter(|expires| *expires > 0)
            .is_some_and(|expires| expires - now_ms <= lead_ms)
}

/// The renewal lead a provider declared in the last cached listing, clamped
/// to a sane range. No cached row (or a plugin that does not renew through
/// the host) keeps the historical 15-minute lead.
fn renew_lead_ms_for(provider: &str, rows: &[PluginProviderRow]) -> i64 {
    rows.iter()
        .find(|row| row.id == provider)
        .filter(|row| row.renews)
        .and_then(|row| row.renew_lead_ms)
        .map(|lead| lead.clamp(MIN_REFRESH_LEAD_MS, MAX_REFRESH_LEAD_MS))
        .unwrap_or(REFRESH_LEAD_MS)
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

pub(super) fn find_account_meta(
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
        assert!(refresh_due(&account, now_ms, REFRESH_LEAD_MS));

        let mut later = account.clone();
        later.expires_at = Some(now_ms + 60 * 60 * 1000);
        assert!(!refresh_due(&later, now_ms, REFRESH_LEAD_MS));

        let mut expired = account.clone();
        expired.expires_at = Some(now_ms - 60 * 60 * 1000);
        assert!(refresh_due(&expired, now_ms, REFRESH_LEAD_MS));

        let mut no_refresh = account.clone();
        no_refresh.has_refresh = false;
        assert!(!refresh_due(&no_refresh, now_ms, REFRESH_LEAD_MS));

        let mut disabled = account.clone();
        disabled.enabled = false;
        assert!(!refresh_due(&disabled, now_ms, REFRESH_LEAD_MS));

        let mut api_key = account;
        api_key.auth_type = "api".to_string();
        assert!(!refresh_due(&api_key, now_ms, REFRESH_LEAD_MS));
    }

    #[test]
    fn a_declared_renew_lead_moves_the_renewal_window() {
        let now_ms = 1_800_000_000_000i64;
        let mut account = PluginAccountSummary {
            account: "demo".to_string(),
            provider: "demo".to_string(),
            auth_type: "oauth".to_string(),
            has_access: true,
            has_refresh: true,
            expires_at: Some(now_ms + 20 * 60 * 1000),
            enabled: true,
            ..PluginAccountSummary::default()
        };

        // 20 minutes out: not due under the default 15-minute lead.
        assert!(!refresh_due(
            &account,
            now_ms,
            renew_lead_ms_for("demo", &[])
        ));

        // A plugin that asks for a 30-minute lead is due now.
        let rows = vec![PluginProviderRow {
            id: "demo".to_string(),
            renews: true,
            renew_lead_ms: Some(30 * 60 * 1000),
            ..PluginProviderRow::default()
        }];
        assert_eq!(renew_lead_ms_for("demo", &rows), 30 * 60 * 1000);
        assert!(refresh_due(
            &account,
            now_ms,
            renew_lead_ms_for("demo", &rows)
        ));

        // The window is clamped, and a provider without a cached row keeps
        // the historical default.
        let clamped = vec![PluginProviderRow {
            id: "demo".to_string(),
            renews: true,
            renew_lead_ms: Some(25 * 60 * 60 * 1000),
            ..PluginProviderRow::default()
        }];
        assert_eq!(renew_lead_ms_for("demo", &clamped), MAX_REFRESH_LEAD_MS);
        assert_eq!(renew_lead_ms_for("other", &rows), REFRESH_LEAD_MS);
        assert_eq!(
            renew_lead_ms_for(
                "demo",
                &[PluginProviderRow {
                    id: "demo".to_string(),
                    renews: false,
                    renew_lead_ms: Some(30 * 60 * 1000),
                    ..PluginProviderRow::default()
                }]
            ),
            REFRESH_LEAD_MS
        );

        account.expires_at = Some(now_ms + 60 * 60 * 1000);
        assert!(!refresh_due(&account, now_ms, 30 * 60 * 1000));
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

    fn account(provider: &str, key: &str, enabled: bool) -> PluginAccountSummary {
        PluginAccountSummary {
            account: key.to_string(),
            provider: provider.to_string(),
            enabled,
            ..PluginAccountSummary::default()
        }
    }

    fn channel_map(provider: &str, enabled: bool) -> HashMap<String, bool> {
        HashMap::from([(provider.to_string(), enabled)])
    }

    fn plugin_maps() -> (HashMap<String, String>, HashSet<String>) {
        (
            HashMap::from([("demo".to_string(), "demo-plugin".to_string())]),
            HashSet::from(["demo-plugin".to_string()]),
        )
    }

    #[test]
    fn channel_switch_checks_in_every_enabled_account() {
        let accounts = vec![
            account("demo", "demo", true),
            account("demo", "demo#2", true),
            account("demo", "demo#3", false),
        ];
        let (provider_plugins, enabled_plugins) = plugin_maps();
        let due = due_checkin_accounts(
            &accounts,
            &HashMap::new(),
            &channel_map("demo", true),
            &provider_plugins,
            &enabled_plugins,
            beijing_now(),
        );
        assert_eq!(
            due,
            vec![
                ("demo".to_string(), "demo".to_string()),
                ("demo".to_string(), "demo#2".to_string()),
            ]
        );
    }

    #[test]
    fn a_channel_switch_that_is_off_or_missing_starts_no_checkin() {
        let accounts = vec![account("demo", "demo", true)];
        let (provider_plugins, enabled_plugins) = plugin_maps();
        let now = beijing_now();

        assert!(due_checkin_accounts(
            &accounts,
            &HashMap::new(),
            &channel_map("demo", false),
            &provider_plugins,
            &enabled_plugins,
            now,
        )
        .is_empty());
        assert!(due_checkin_accounts(
            &accounts,
            &HashMap::new(),
            &HashMap::new(),
            &provider_plugins,
            &enabled_plugins,
            now,
        )
        .is_empty());
    }

    #[test]
    fn a_disabled_plugin_or_settled_account_is_skipped() {
        let now = beijing_now();
        let today = beijing_day(now);
        let accounts = vec![account("demo", "demo", true)];
        let (provider_plugins, enabled_plugins) = plugin_maps();
        let settled = PluginAccountMetaRecord {
            id: "demo".to_string(),
            provider_id: "demo".to_string(),
            enabled: true,
            last_checkin_day: Some(today),
            last_checkin_outcome: Some("claimed".to_string()),
            last_checkin_at: Some(now.to_rfc3339()),
            ..PluginAccountMetaRecord::default()
        };
        let metas = HashMap::from([("demo", &settled)]);

        assert!(due_checkin_accounts(
            &accounts,
            &metas,
            &channel_map("demo", true),
            &provider_plugins,
            &enabled_plugins,
            now,
        )
        .is_empty());

        // An empty enabled-plugin set means no host can serve the account.
        assert!(due_checkin_accounts(
            &accounts,
            &HashMap::new(),
            &channel_map("demo", true),
            &provider_plugins,
            &HashSet::new(),
            now,
        )
        .is_empty());
    }

    #[test]
    fn legacy_account_flags_enable_the_channel_without_an_explicit_switch() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        store::upsert_account_meta(
            &db,
            &PluginAccountMetaRecord {
                id: "demo".to_string(),
                plugin_id: "demo-plugin".to_string(),
                provider_id: "demo".to_string(),
                enabled: true,
                auto_checkin: true,
                ..PluginAccountMetaRecord::default()
            },
        )
        .unwrap();
        let switches = store::resolve_provider_auto_checkin(&db).unwrap();
        let (provider_plugins, enabled_plugins) = plugin_maps();
        let due = due_checkin_accounts(
            &[
                account("demo", "demo", true),
                account("demo", "demo#2", true),
            ],
            &HashMap::new(),
            &switches,
            &provider_plugins,
            &enabled_plugins,
            beijing_now(),
        );
        assert_eq!(due.len(), 2);

        // Once the channel is explicitly turned off, the legacy flag no
        // longer schedules anything.
        store::set_provider_auto_checkin(&db, "demo", false).unwrap();
        let switches = store::resolve_provider_auto_checkin(&db).unwrap();
        assert!(due_checkin_accounts(
            &[account("demo", "demo", true)],
            &HashMap::new(),
            &switches,
            &provider_plugins,
            &enabled_plugins,
            beijing_now(),
        )
        .is_empty());
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
