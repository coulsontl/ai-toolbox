//! Keeps the WorkBuddy family's gateway provider rows in step with
//! `models.json`.
//!
//! The page never writes these rows: its channel list is a view over the file.
//! The gateway needs rows because takeover records a provider id in the
//! manifest and per-provider settings must survive a restart. So the rows are
//! **derived on demand** from the file, with one rule that matters:
//!
//! An existing row keeps its `id`, `sort_index` and `is_disabled`. Re-creating
//! rows on every sync would renumber ids, which breaks a manifest that already
//! points at one, and would silently re-enable a provider the user turned off.
//! Only genuinely new channels get appended.

use crate::coding::workbuddy_family::channel;
use crate::coding::workbuddy_family::gateway_provider;
use crate::coding::workbuddy_family::models_json::ModelsDocument;
use crate::db::schema::DbTable;
use crate::db::SqliteDbState;

/// Which store to sync. Kept separate from `GatewayCliKey` so this module does
/// not depend on the gateway's route types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FamilyStore {
    Workbuddy,
    Codebuddy,
}

impl FamilyStore {
    pub fn table(self) -> DbTable {
        match self {
            Self::Workbuddy => DbTable::WorkbuddyProvider,
            Self::Codebuddy => DbTable::CodebuddyProvider,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Workbuddy => "WorkBuddy",
            Self::Codebuddy => "CodeBuddy",
        }
    }
}

/// Result of one sync pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FamilySyncOutcome {
    /// Channels newly written to the store.
    pub inserted: usize,
    /// Channels already present (matched by `(baseUrl, apiKey)`).
    pub kept: usize,
    /// Rows whose channel is gone from `models.json`. Reported, not deleted:
    /// deleting would drop a manifest's target out from under an active
    /// takeover. The candidate loader skips them instead.
    pub orphaned: usize,
}

/// One channel as read back from the store, for matching.
struct ExistingRow {
    id: String,
    base_url: String,
    api_key: String,
}

fn read_existing_rows(db: &SqliteDbState, store: FamilyStore) -> Result<Vec<ExistingRow>, String> {
    let records = db.with_conn(|conn| crate::db::helpers::db_list(conn, store.table(), None))?;
    Ok(records
        .into_iter()
        .filter_map(|record| {
            let provider = gateway_provider::provider_from_db_value(record);
            let (base_url, api_key) =
                gateway_provider::channel_from_settings_config(&provider.settings_config)?;
            Some(ExistingRow {
                id: provider.id,
                base_url,
                api_key,
            })
        })
        .collect())
}

/// Mirrors `models.json` channels into the store.
///
/// `config_root` is the product's configuration directory (the parent of
/// `models.json`).
pub fn sync_family_providers(
    db: &SqliteDbState,
    store: FamilyStore,
    config_root: &std::path::Path,
) -> Result<FamilySyncOutcome, String> {
    // `ModelsDocument::load`, not `read_to_string(...).unwrap_or_default() +
    // parse`: a missing `models.json` is the normal state for a product the
    // user has not added a custom model to yet, and `parse("")` rejects it as
    // invalid JSON — which surfaced as "models.json is not valid JSON" when
    // the user clicked "take over" on a fresh install. `load` returns an empty
    // document for a missing file and only errors on a real read/parse failure.
    let document = ModelsDocument::load(config_root)?;
    let channels = channel::build_channel_views(&document.models);

    let existing = read_existing_rows(db, store)?;
    let mut existing_by_key: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();
    for row in &existing {
        existing_by_key.insert((row.base_url.clone(), row.api_key.clone()), row.id.clone());
    }

    let mut outcome = FamilySyncOutcome::default();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

    db.with_conn(|conn| {
        for (index, channel) in channels.iter().enumerate() {
            let key = (channel.base_url.clone(), channel.api_key.clone());
            seen.insert(key.clone());
            if existing_by_key.contains_key(&key) {
                outcome.kept += 1;
                continue;
            }
            let name = if channel.vendor.trim().is_empty() {
                channel.name.clone()
            } else {
                channel.vendor.clone()
            };
            let payload = gateway_provider::provider_payload(
                &name,
                &channel.base_url,
                &channel.api_key,
                index as i32,
            );
            crate::db::helpers::db_create(conn, store.table(), &payload)
                .map_err(|error| format!("Failed to create provider row: {error}"))?;
            outcome.inserted += 1;
        }
        for row in &existing {
            if !seen.contains(&(row.base_url.clone(), row.api_key.clone())) {
                outcome.orphaned += 1;
                log::warn!(
                    "[proxy-gateway] {} provider '{}' has no channel in models.json; \
                     it stays in the store but is skipped as a candidate",
                    store.label(),
                    row.id
                );
            }
        }
        Ok(())
    })?;

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn write_models(dir: &std::path::Path, models: Value) {
        std::fs::write(
            dir.join("models.json"),
            serde_json::to_string_pretty(&models).unwrap(),
        )
        .unwrap();
    }

    fn test_db() -> SqliteDbState {
        SqliteDbState::in_memory_for_test().expect("in-memory db")
    }

    #[test]
    fn channels_are_inserted_once_and_kept_on_a_second_pass() {
        let dir = tempfile::tempdir().unwrap();
        write_models(
            dir.path(),
            json!({
                "models": [
                    {"id": "m1", "url": "https://a.example.com/v1/chat/completions",
                     "apiKey": "k1", "vendor": "Alpha"},
                    {"id": "m2", "url": "https://b.example.com/v1/chat/completions",
                     "apiKey": "k2", "vendor": "Beta"}
                ]
            }),
        );
        let db = test_db();

        let first = sync_family_providers(&db, FamilyStore::Workbuddy, dir.path()).unwrap();
        assert_eq!(first.inserted, 2);
        assert_eq!(first.kept, 0);

        let second = sync_family_providers(&db, FamilyStore::Workbuddy, dir.path()).unwrap();
        assert_eq!(second.inserted, 0, "a resync must not duplicate rows");
        assert_eq!(second.kept, 2);
    }

    #[test]
    fn a_channel_leaving_the_file_is_reported_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        write_models(
            dir.path(),
            json!({"models": [
                {"id": "m1", "url": "https://a.example.com/v1/chat/completions", "apiKey": "k1"}
            ]}),
        );
        let db = test_db();
        sync_family_providers(&db, FamilyStore::Workbuddy, dir.path()).unwrap();

        write_models(
            dir.path(),
            json!({"models": [
                {"id": "m2", "url": "https://b.example.com/v1/chat/completions", "apiKey": "k2"}
            ]}),
        );
        let outcome = sync_family_providers(&db, FamilyStore::Workbuddy, dir.path()).unwrap();
        assert_eq!(outcome.inserted, 1);
        assert_eq!(outcome.orphaned, 1, "the removed channel is reported");

        // The orphan row still exists: deleting it would break an active
        // manifest that points at it.
        let rows = read_existing_rows(&db, FamilyStore::Workbuddy).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn a_disabled_row_is_not_revived_by_a_resync() {
        let dir = tempfile::tempdir().unwrap();
        write_models(
            dir.path(),
            json!({"models": [
                {"id": "m1", "url": "https://a.example.com/v1/chat/completions", "apiKey": "k1"}
            ]}),
        );
        let db = test_db();
        sync_family_providers(&db, FamilyStore::Workbuddy, dir.path()).unwrap();

        // Disable it the way the settings page would.
        db.with_conn(|conn| {
            let records = crate::db::helpers::db_list(conn, FamilyStore::Workbuddy.table(), None)?;
            let id = records[0]["id"].as_str().unwrap().to_string();
            crate::db::helpers::db_patch_fields(
                conn,
                FamilyStore::Workbuddy.table(),
                &id,
                &[("is_disabled", json!(true))],
            )?;
            Ok(())
        })
        .unwrap();

        let outcome = sync_family_providers(&db, FamilyStore::Workbuddy, dir.path()).unwrap();
        assert_eq!(outcome.inserted, 0);
        assert_eq!(outcome.kept, 1);

        db.with_conn(|conn| {
            let records = crate::db::helpers::db_list(conn, FamilyStore::Workbuddy.table(), None)?;
            assert_eq!(records.len(), 1);
            assert_eq!(records[0]["is_disabled"], json!(true));
            Ok(())
        })
        .unwrap();
    }
}
