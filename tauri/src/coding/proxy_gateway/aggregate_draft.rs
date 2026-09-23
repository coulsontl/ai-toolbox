//! Persisted aggregate draft for the gateway settings page.
//!
//! The aggregate site selection is configuration the user edits *before* the
//! mode is engaged, so it cannot live only in component state (closing the Codex
//! drawer or leaving the gateway settings tab unmounts the editor) and it cannot
//! live in `ProxyGatewaySettings`: that blob is rewritten wholesale by the
//! settings panel's debounced auto-save, so any unrelated settings edit would
//! clobber a draft stored there. It is also not written into `manifest.json`,
//! which is the *enabled* takeover's source of truth and carries backup metadata
//! for single/failover takeovers that a draft edit must never touch.
//!
//! Reads are deliberately tolerant: a missing or unreadable draft degrades to
//! "no draft" and the frontend falls back to the currently applied provider,
//! because a broken convenience file must not break the settings panel.

use super::aggregate_naming::AggregateNamingMode;
use super::cli_proxy::manifest::AGGREGATE_DEFAULT_SEPARATOR;
use super::paths::ProxyGatewayPaths;
use super::types::{GatewayAggregateConfig, GatewayCliKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;

/// On-disk shape of a draft. Mirrors `GatewayAggregateConfig` but tolerates
/// hand-edited or older files that omit fields, and never carries the
/// engage-time fields: `slug_table` is allocated when the mode is engaged, and
/// the `[agents]` subagent defaults only exist while the mode is engaged, so a
/// draft must not make them look managed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
struct AggregateDraftRecord {
    provider_ids: Vec<String>,
    separator: String,
    aliases: BTreeMap<String, String>,
    naming: AggregateNamingMode,
    /// Whether a request may move to another selected site on failure. Absent
    /// in older draft files, which therefore default to `false`.
    cross_site_failover: bool,
    /// Declared bare model names selected for subagent exposure. Selected names
    /// are promoted into the visible catalog; unselected names remain
    /// addressable as hidden bare-name aliases (or reuse an identical visible
    /// site slug). Stale names are filtered when the draft is saved and the UI
    /// reports that removal. Empty keeps the backend's legacy behavior: no name
    /// is promoted, while every declared bare name stays addressable.
    ///
    /// Order is the user's tick order and is the priority order of the five
    /// `spawn_agent` model hints. A `Vec` keeps that order across the
    /// draft round-trip; a set would re-sort it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    subagent_exposed_models: Vec<String>,
}

impl From<AggregateDraftRecord> for GatewayAggregateConfig {
    fn from(record: AggregateDraftRecord) -> Self {
        let separator = record.separator.trim();
        Self {
            provider_ids: record.provider_ids,
            separator: if separator.is_empty() {
                AGGREGATE_DEFAULT_SEPARATOR.to_string()
            } else {
                separator.to_string()
            },
            aliases: record.aliases,
            cross_site_failover: record.cross_site_failover,
            naming: record.naming,
            subagent_exposed_models: record.subagent_exposed_models,
            subagent: None,
        }
    }
}

impl From<&GatewayAggregateConfig> for AggregateDraftRecord {
    fn from(config: &GatewayAggregateConfig) -> Self {
        Self {
            provider_ids: config.provider_ids.clone(),
            separator: config.separator.clone(),
            aliases: config.aliases.clone(),
            naming: config.naming,
            cross_site_failover: config.cross_site_failover,
            subagent_exposed_models: config.subagent_exposed_models.clone(),
        }
    }
}

/// Read the last saved aggregate draft, or `None` when nothing usable is stored.
pub fn load_aggregate_draft(
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
) -> Option<GatewayAggregateConfig> {
    let path = paths.aggregate_draft_path(cli_key);
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return None,
        Err(error) => {
            log::warn!(
                "Failed to read gateway aggregate draft {}: {error}",
                path.display()
            );
            return None;
        }
    };
    match serde_json::from_str::<AggregateDraftRecord>(&content) {
        Ok(record) => Some(record.into()),
        Err(error) => {
            log::warn!(
                "Failed to parse gateway aggregate draft {}: {error}",
                path.display()
            );
            None
        }
    }
}

/// Persist an already-validated aggregate draft.
pub fn write_aggregate_draft(
    paths: &ProxyGatewayPaths,
    cli_key: GatewayCliKey,
    config: &GatewayAggregateConfig,
) -> Result<(), String> {
    let path = paths.aggregate_draft_path(cli_key);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create gateway aggregate draft directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let content = serde_json::to_string_pretty(&AggregateDraftRecord::from(config))
        .map_err(|error| format!("Failed to serialize gateway aggregate draft: {error}"))?;
    fs::write(&path, format!("{content}\n")).map_err(|error| {
        format!(
            "Failed to write gateway aggregate draft {}: {error}",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(provider_ids: &[&str], separator: &str) -> GatewayAggregateConfig {
        GatewayAggregateConfig {
            provider_ids: provider_ids.iter().map(|id| (*id).to_string()).collect(),
            separator: separator.to_string(),
            aliases: BTreeMap::from([("site-a".to_string(), "a".to_string())]),
            naming: AggregateNamingMode::ModelAtSite,
            // Deliberately non-default so the round trip proves the new field
            // is persisted instead of silently collapsing to its default.
            cross_site_failover: true,
            // Deliberately non-default: a round trip that collapsed this to the
            // empty "publish everything" set would go unnoticed otherwise.
            subagent_exposed_models: vec!["gpt-5.6-luna".to_string()],
            subagent: None,
        }
    }

    #[test]
    fn draft_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        assert_eq!(load_aggregate_draft(&paths, GatewayCliKey::Codex), None);

        let expected = draft(&["site-a", "site-b"], "|");
        write_aggregate_draft(&paths, GatewayCliKey::Codex, &expected).unwrap();

        assert_eq!(
            load_aggregate_draft(&paths, GatewayCliKey::Codex),
            Some(expected)
        );
        // Drafts are CLI scoped: codex's draft must not leak into another CLI.
        assert_eq!(load_aggregate_draft(&paths, GatewayCliKey::Claude), None);
    }

    #[test]
    fn incomplete_draft_falls_back_to_defaults_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let path = paths.aggregate_draft_path(GatewayCliKey::Codex);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{\"provider_ids\": [\"site-a\"]}").unwrap();

        let loaded = load_aggregate_draft(&paths, GatewayCliKey::Codex).unwrap();

        assert_eq!(loaded.provider_ids, vec!["site-a".to_string()]);
        assert_eq!(loaded.separator, AGGREGATE_DEFAULT_SEPARATOR);
        assert!(loaded.aliases.is_empty());
        assert_eq!(loaded.naming, AggregateNamingMode::SiteModel);
        // A draft written before the exposure set existed keeps the default of
        // publishing every bare model instead of failing to load.
        assert!(loaded.subagent_exposed_models.is_empty());
    }

    #[test]
    fn unreadable_draft_is_reported_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let path = paths.aggregate_draft_path(GatewayCliKey::Codex);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{not json").unwrap();

        assert_eq!(load_aggregate_draft(&paths, GatewayCliKey::Codex), None);
    }
}
