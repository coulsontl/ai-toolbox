use crate::coding::proxy_gateway::{
    aggregate_naming::{AggregateNamingMode, AggregateSlugEntry},
    types::{GatewayCliKey, GatewayProxyMode},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path};

/// Codex `[agents]` keys aggregate mode may write, and only these.
///
/// `[agents]` also carries user settings (`enabled`,
/// `max_concurrent_threads_per_session`, …) that must survive both the takeover
/// and the restore untouched, so the managed set is an explicit allowlist.
pub const CODEX_AGENT_MODEL_KEY: &str = "default_subagent_model";
pub const CODEX_AGENT_EFFORT_KEY: &str = "default_subagent_reasoning_effort";

/// The `[agents]` defaults an aggregate takeover writes.
///
/// Aggregate mode replaces the model list, so a bare-name default such as
/// `gpt-5.6-luna` must resolve through the published catalog (the hidden
/// bare-name aliases). This is opt-in: an all-`None` value leaves `[agents]`
/// exactly as the user wrote it, which is what single/failover mode does.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AggregateSubagentDefaults {
    /// `[agents] default_subagent_model`. `None` leaves the key alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `[agents] default_subagent_reasoning_effort`. `None` leaves it alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

impl AggregateSubagentDefaults {
    /// Whether this configuration manages `key`, i.e. whether the restore must
    /// put it back to its pre-takeover state.
    pub fn manages(&self, key: &str) -> bool {
        match key {
            CODEX_AGENT_MODEL_KEY => self.model.is_some(),
            CODEX_AGENT_EFFORT_KEY => self.reasoning_effort.is_some(),
            _ => false,
        }
    }

    /// Whether any `[agents]` key is managed at all.
    pub fn is_empty(&self) -> bool {
        self.model.is_none() && self.reasoning_effort.is_none()
    }

    /// Drop blank values so a cleared form field means "leave it alone" instead
    /// of writing an empty model name into the user's config.
    pub fn normalized(mut self) -> Self {
        self.model = self
            .model
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.reasoning_effort = self
            .reasoning_effort
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self
    }
}

/// Codex model catalog state captured right before aggregate mode overwrote it.
///
/// Aggregate mode rewrites both config.toml's `model_catalog_json` pointer and
/// the AI Toolbox-managed catalog file that pointer names, so leaving aggregate
/// mode must replay this snapshot. Without it the user's pre-takeover catalog
/// (single-site mappings, or a self-owned external file) is silently lost.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PreAggregateCodexCatalog {
    /// config.toml's top-level `model_catalog_json` before the engage, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    /// Raw content of `ai-toolbox-codex-model-catalog.json` before the engage,
    /// when the file existed. `None` means aggregate mode created the file, so
    /// restoring removes it instead of leaving a stale aggregate catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AggregateManifestConfig {
    /// Sites selected for aggregate routing, in the user's display order.
    #[serde(default)]
    pub provider_ids: Vec<String>,
    /// Separator between site id and upstream model name. Defaults to `.`.
    #[serde(default = "default_aggregate_separator")]
    pub separator: String,
    /// Per-site display/routing aliases. A missing entry falls back to the
    /// provider id so manifests written before aliases remain compatible.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aliases: BTreeMap<String, String>,
    /// How `(site, model)` pairs are named in the generated Codex catalog.
    #[serde(default)]
    pub naming: AggregateNamingMode,
    /// Whether aggregate routing may move a request to another selected site
    /// when the addressed site fails. Absent (manifests written before this
    /// field) means `false`: the request stays on the single named site.
    #[serde(default)]
    pub cross_site_failover: bool,
    /// Bare upstream model names promoted into Codex's visible catalog.
    /// Unticked names remain addressable as hidden aliases unless an identical
    /// visible site slug already exists. Absent or empty keeps the historical
    /// default of promoting none while publishing every declared bare name.
    ///
    /// Order is the user's tick order and is the priority order of the five
    /// `spawn_agent` model hints: the first entry is promoted to the lowest
    /// priority. A `Vec` keeps that order; a set would re-sort it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagent_exposed_models: Vec<String>,
    /// Slug table the Codex catalog was generated from, in publication order.
    ///
    /// Persisted so routing replays the exact table instead of re-deriving it
    /// from the currently enabled candidates: with `model_only` a site that
    /// disappears would otherwise renumber every later `#N` slug, silently
    /// pointing it at another site. `provider_ids` + `naming` rebuild the table
    /// for manifests written before this field existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slug_table: Vec<AggregateSlugEntry>,
    /// `[agents]` defaults the takeover wrote, if any. Persisted so the restore
    /// knows exactly which keys to reconcile — an absent field means this
    /// manifest predates the feature and owns no `[agents]` keys.
    #[serde(default, skip_serializing_if = "AggregateSubagentDefaults::is_empty")]
    pub subagent: AggregateSubagentDefaults,
    /// Codex catalog state captured before aggregate mode overwrote it. Absent
    /// in manifests written before the snapshot existed; those keep the legacy
    /// "drop our own pointer" cleanup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_aggregate_catalog: Option<PreAggregateCodexCatalog>,
}

fn default_aggregate_separator() -> String {
    AGGREGATE_DEFAULT_SEPARATOR.to_string()
}

impl Default for AggregateManifestConfig {
    fn default() -> Self {
        Self {
            provider_ids: Vec::new(),
            separator: default_aggregate_separator(),
            aliases: BTreeMap::new(),
            naming: AggregateNamingMode::default(),
            cross_site_failover: false,
            subagent_exposed_models: Vec::new(),
            slug_table: Vec::new(),
            subagent: AggregateSubagentDefaults::default(),
            pre_aggregate_catalog: None,
        }
    }
}

/// Default separator between the site id and the upstream model name in
/// aggregate mode. `.` keeps the generated slugs acceptable to Codex's
/// telemetry tags (unlike `:`) while still being readable.
pub const AGGREGATE_DEFAULT_SEPARATOR: &str = ".";

/// Validate a user-supplied aggregate separator.
///
/// The separator must be non-empty and must not contain characters that are
/// legal inside a site id, otherwise `<site_id><sep><model>` becomes ambiguous
/// and cannot be split back reliably.
pub fn validate_aggregate_separator(separator: &str) -> Result<(), String> {
    if separator.is_empty() {
        return Err("Aggregate separator must not be empty".to_string());
    }
    if separator
        .chars()
        .any(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err("Aggregate separator must not contain letters, digits, '_' or '-'".to_string());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CliProxyManifest {
    pub schema_version: u32,
    pub managed_by: String,
    pub cli_key: GatewayCliKey,
    pub enabled: bool,
    pub mode: GatewayProxyMode,
    pub primary_provider_id: String,
    pub base_origin: String,
    pub created_at: String,
    pub updated_at: String,
    pub files: Vec<CliProxyManifestFile>,
    /// Aggregate-mode routing config. Absent for single/failover manifests, and
    /// absent in manifests written before aggregate mode existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<AggregateManifestConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CliProxyManifestFile {
    pub kind: String,
    pub path: String,
    pub existed: bool,
    pub backup_rel_path: String,
    pub backup_sha256: Option<String>,
    pub backup_size: Option<u64>,
    pub managed_fields: Vec<String>,
}

impl CliProxyManifest {
    pub fn new(
        cli_key: GatewayCliKey,
        base_origin: String,
        timestamp: String,
        mode: GatewayProxyMode,
        primary_provider_id: String,
    ) -> Self {
        Self {
            schema_version: 1,
            managed_by: "ai-toolbox-proxy-gateway".to_string(),
            cli_key,
            enabled: true,
            mode,
            primary_provider_id,
            base_origin,
            created_at: timestamp.clone(),
            updated_at: timestamp,
            files: Vec::new(),
            aggregate: None,
        }
    }

    /// Attach aggregate routing config and switch the manifest to aggregate mode.
    pub fn with_aggregate(
        mut self,
        provider_ids: Vec<String>,
        separator: String,
        aliases: BTreeMap<String, String>,
        naming: AggregateNamingMode,
        cross_site_failover: bool,
        slug_table: Vec<AggregateSlugEntry>,
    ) -> Self {
        self.mode = GatewayProxyMode::Aggregate;
        self.aggregate = Some(AggregateManifestConfig {
            provider_ids,
            separator,
            aliases,
            naming,
            cross_site_failover,
            // `with_aggregate` keeps the historical "publish every bare model"
            // default; callers narrow it through
            // `with_aggregate_subagent_exposed_models`.
            subagent_exposed_models: Vec::new(),
            slug_table,
            subagent: AggregateSubagentDefaults::default(),
            pre_aggregate_catalog: None,
        });
        self
    }

    /// Set the bare model names promoted into the generated visible catalog.
    /// Empty (the default) promotes none while keeping every bare model
    /// addressable.
    ///
    /// Mirrors `with_aggregate_subagent_defaults`: `with_aggregate` keeps its
    /// call sites untouched, and the exposure set is attached separately so a
    /// manifest written without it means "promote none; publish every bare
    /// model as addressable".
    pub fn with_aggregate_subagent_exposed_models(
        mut self,
        subagent_exposed_models: Vec<String>,
    ) -> Self {
        if let Some(aggregate) = self.aggregate.as_mut() {
            aggregate.subagent_exposed_models = subagent_exposed_models;
        }
        self
    }

    /// Set the `[agents]` defaults this aggregate takeover manages.
    pub fn with_aggregate_subagent_defaults(mut self, subagent: AggregateSubagentDefaults) -> Self {
        if let Some(aggregate) = self.aggregate.as_mut() {
            aggregate.subagent = subagent;
        }
        self
    }

    /// Attach the pre-aggregate Codex catalog snapshot so leaving aggregate
    /// mode restores it. `None` keeps the legacy pointer-only cleanup.
    pub fn with_pre_aggregate_catalog(
        mut self,
        snapshot: Option<PreAggregateCodexCatalog>,
    ) -> Self {
        if let Some(aggregate) = self.aggregate.as_mut() {
            aggregate.pre_aggregate_catalog = snapshot;
        }
        self
    }
}

pub fn validate_backup_rel_path(path: &str) -> Result<(), String> {
    if path.contains(':') || path.contains('\\') {
        return Err("Manifest backup path must use a relative forward-slash path".to_string());
    }
    let path = Path::new(path);
    if path.is_absolute() {
        return Err("Manifest backup path must be relative".to_string());
    }
    for component in path.components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("Manifest backup path cannot escape the backup directory".to_string())
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_serializes_without_provider_data() {
        let mut manifest = CliProxyManifest::new(
            GatewayCliKey::Codex,
            "http://127.0.0.1:37123".to_string(),
            "2026-05-16T10:00:00Z".to_string(),
            GatewayProxyMode::Single,
            "provider-1".to_string(),
        );
        manifest.files.push(CliProxyManifestFile {
            kind: "codex_config_toml".to_string(),
            path: "C:\\Users\\User\\.codex\\config.toml".to_string(),
            existed: true,
            backup_rel_path: "backups/config.toml".to_string(),
            backup_sha256: Some("abc".to_string()),
            backup_size: Some(123),
            managed_fields: vec![
                "model_providers.custom.base_url".to_string(),
                "model_providers.custom.wire_api".to_string(),
                "model_providers.custom.experimental_bearer_token".to_string(),
            ],
        });

        let json = serde_json::to_string(&manifest).unwrap();

        assert!(json.contains("codex_config_toml"));
        assert!(json.contains("primary_provider_id"));
        assert!(!json.contains("settings_config"));
        assert!(!json.contains("api_key"));
    }

    #[test]
    fn backup_relative_path_accepts_normal_path() {
        assert!(validate_backup_rel_path("backups/config.toml").is_ok());
    }

    #[test]
    fn backup_relative_path_rejects_parent_escape() {
        assert!(validate_backup_rel_path("../config.toml").is_err());
        assert!(validate_backup_rel_path("backups/../../config.toml").is_err());
    }

    #[test]
    fn backup_relative_path_rejects_absolute_path() {
        assert!(validate_backup_rel_path("C:\\Users\\config.toml").is_err());
        assert!(validate_backup_rel_path("/tmp/config.toml").is_err());
    }

    #[test]
    fn aggregate_manifest_defaults_naming_for_older_manifests() {
        let parsed: AggregateManifestConfig = serde_json::from_value(serde_json::json!({
            "provider_ids": ["site-a"],
            "separator": "."
        }))
        .unwrap();

        assert!(parsed.aliases.is_empty());
        assert_eq!(parsed.naming, AggregateNamingMode::SiteModel);
        // No persisted table: routing rebuilds it from `provider_ids` + `naming`.
        assert!(parsed.slug_table.is_empty());
        assert!(parsed.pre_aggregate_catalog.is_none());
    }

    #[test]
    fn aggregate_manifest_round_trips_the_pre_aggregate_catalog_snapshot() {
        let snapshot = PreAggregateCodexCatalog {
            pointer: Some("ai-toolbox-codex-model-catalog.json".to_string()),
            file_content: Some("{\"models\":[]}".to_string()),
        };
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["pointer"], "ai-toolbox-codex-model-catalog.json");

        let parsed: AggregateManifestConfig = serde_json::from_value(serde_json::json!({
            "provider_ids": ["site-a"],
            "separator": ".",
            "pre_aggregate_catalog": {
                "pointer": "external.json",
                "file_content": null
            }
        }))
        .unwrap();
        let restored = parsed.pre_aggregate_catalog.unwrap();
        assert_eq!(restored.pointer.as_deref(), Some("external.json"));
        assert!(restored.file_content.is_none());
    }
}
