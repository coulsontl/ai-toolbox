use super::types::{
    GatewayModelHealthItem, GatewayModelHealthScope, ModelHealthEntry, ModelHealthStateKind,
    ProviderHealthKey, ProviderModelHealthKey, ProxyGatewaySettings,
};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayFailureKind {
    Timeout,
    Connection,
    EmptyResponse,
    RateLimit,
    Upstream5xx,
    UpstreamBadRequest,
    ModelNotFound,
    Auth,
    RequestSchema,
    ClientCancelled,
    GatewayParse,
    /// The relay refused this turn because the connection is pinned to another
    /// model. The refusal is about the connection, not the model, so it must
    /// not move health — but it is not the client's cancel either.
    UpstreamModelLocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureScope {
    Model,
    Provider,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureWeight {
    pub scope: FailureScope,
    pub score: i32,
    pub category: &'static str,
}

pub fn classify_failure(kind: GatewayFailureKind) -> FailureWeight {
    match kind {
        GatewayFailureKind::Timeout => FailureWeight {
            scope: FailureScope::Model,
            score: 3,
            category: "timeout",
        },
        GatewayFailureKind::Connection => FailureWeight {
            scope: FailureScope::Model,
            score: 3,
            category: "connection",
        },
        GatewayFailureKind::EmptyResponse => FailureWeight {
            scope: FailureScope::Model,
            score: 2,
            category: "empty_response",
        },
        GatewayFailureKind::RateLimit => FailureWeight {
            scope: FailureScope::Model,
            score: 3,
            category: "rate_limit",
        },
        GatewayFailureKind::Upstream5xx => FailureWeight {
            scope: FailureScope::Model,
            score: 2,
            category: "upstream_5xx",
        },
        GatewayFailureKind::UpstreamBadRequest => FailureWeight {
            scope: FailureScope::Model,
            score: 1,
            category: "upstream_bad_request",
        },
        GatewayFailureKind::ModelNotFound => FailureWeight {
            scope: FailureScope::Model,
            score: 5,
            category: "model_not_found",
        },
        GatewayFailureKind::Auth => FailureWeight {
            scope: FailureScope::Provider,
            score: 5,
            category: "auth",
        },
        GatewayFailureKind::RequestSchema => FailureWeight {
            scope: FailureScope::None,
            score: 0,
            category: "request_schema",
        },
        GatewayFailureKind::ClientCancelled => FailureWeight {
            scope: FailureScope::None,
            score: 0,
            category: "client_cancelled",
        },
        GatewayFailureKind::GatewayParse => FailureWeight {
            scope: FailureScope::None,
            score: 0,
            category: "gateway_parse",
        },
        GatewayFailureKind::UpstreamModelLocked => FailureWeight {
            scope: FailureScope::None,
            score: 0,
            category: "websocket_model_locked",
        },
    }
}

#[derive(Clone)]
pub struct ModelHealthRegistry {
    settings: ProxyGatewaySettings,
    model_entries: HashMap<ProviderModelHealthKey, ModelHealthEntry>,
    provider_entries: HashMap<ProviderHealthKey, ModelHealthEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct ModelHealthSnapshot {
    schema_version: u32,
    model_entries: Vec<ModelHealthSnapshotModelEntry>,
    provider_entries: Vec<ModelHealthSnapshotProviderEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct ModelHealthSnapshotModelEntry {
    key: ProviderModelHealthKey,
    entry: ModelHealthEntry,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct ModelHealthSnapshotProviderEntry {
    key: ProviderHealthKey,
    entry: ModelHealthEntry,
}

impl ModelHealthRegistry {
    pub fn new(settings: ProxyGatewaySettings) -> Self {
        Self {
            settings,
            model_entries: HashMap::new(),
            provider_entries: HashMap::new(),
        }
    }

    pub fn update_settings(&mut self, settings: ProxyGatewaySettings) {
        self.settings = settings;
    }

    pub fn load(path: &std::path::Path, settings: ProxyGatewaySettings) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::new(settings));
        }
        let content = fs::read_to_string(path).map_err(|error| {
            format!(
                "Failed to read proxy gateway model health {}: {}",
                path.display(),
                error
            )
        })?;
        if content.trim().is_empty() {
            return Ok(Self::new(settings));
        }
        let snapshot: ModelHealthSnapshot = serde_json::from_str(&content).map_err(|error| {
            format!(
                "Failed to parse proxy gateway model health {}: {}",
                path.display(),
                error
            )
        })?;
        Ok(Self {
            settings,
            model_entries: snapshot
                .model_entries
                .into_iter()
                .map(|item| (item.key, item.entry))
                .collect(),
            provider_entries: snapshot
                .provider_entries
                .into_iter()
                .map(|item| (item.key, item.entry))
                .collect(),
        })
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create proxy gateway model health directory {}: {}",
                    parent.display(),
                    error
                )
            })?;
        }
        let snapshot = ModelHealthSnapshot {
            schema_version: 1,
            model_entries: self
                .model_entries
                .iter()
                .map(|(key, entry)| ModelHealthSnapshotModelEntry {
                    key: key.clone(),
                    entry: entry.clone(),
                })
                .collect(),
            provider_entries: self
                .provider_entries
                .iter()
                .map(|(key, entry)| ModelHealthSnapshotProviderEntry {
                    key: key.clone(),
                    entry: entry.clone(),
                })
                .collect(),
        };
        let content = serde_json::to_string_pretty(&snapshot)
            .map_err(|error| format!("Failed to serialize proxy gateway model health: {error}"))?;
        fs::write(path, format!("{content}\n")).map_err(|error| {
            format!(
                "Failed to write proxy gateway model health {}: {}",
                path.display(),
                error
            )
        })
    }

    pub fn health_items(&self) -> Vec<GatewayModelHealthItem> {
        let mut items = Vec::new();
        for (key, entry) in &self.model_entries {
            items.push(health_item_from_model_entry(key, entry));
        }
        for (key, entry) in &self.provider_entries {
            items.push(health_item_from_provider_entry(key, entry));
        }
        items.sort_by(|left, right| {
            left.cli_key
                .as_str()
                .cmp(right.cli_key.as_str())
                .then_with(|| left.provider_id.cmp(&right.provider_id))
                .then_with(|| left.upstream_model_id.cmp(&right.upstream_model_id))
        });
        items
    }

    pub fn model_entry(&self, key: &ProviderModelHealthKey) -> Option<&ModelHealthEntry> {
        self.model_entries.get(key)
    }

    pub fn provider_entry(&self, key: &ProviderHealthKey) -> Option<&ModelHealthEntry> {
        self.provider_entries.get(key)
    }

    pub fn is_model_available(&self, key: &ProviderModelHealthKey, now: DateTime<Utc>) -> bool {
        if !self.is_provider_available(&ProviderHealthKey::from(key), now) {
            return false;
        }

        self.model_entries
            .get(key)
            .map(|entry| !is_cooling(entry, now))
            .unwrap_or(true)
    }

    /// A WebSocket handshake has no model yet, so only provider cooldown applies.
    pub fn is_provider_available(&self, key: &ProviderHealthKey, now: DateTime<Utc>) -> bool {
        !self
            .provider_entries
            .get(key)
            .is_some_and(|entry| is_cooling(entry, now))
    }

    pub fn refresh_due_cooldowns(&mut self, now: DateTime<Utc>) {
        for entry in self
            .model_entries
            .values_mut()
            .chain(self.provider_entries.values_mut())
        {
            if entry.state == ModelHealthStateKind::CoolingDown
                && entry.next_retry_at.is_some_and(|retry_at| retry_at <= now)
            {
                entry.state = ModelHealthStateKind::Probing;
                entry.half_open_success_count = 0;
            }
        }
    }

    pub fn record_success(&mut self, key: &ProviderModelHealthKey) -> bool {
        let success_required = self.settings.half_open_success_required;
        let mut changed = false;
        let provider_key = ProviderHealthKey::from(key);
        if let Some(entry) = self.provider_entries.get_mut(&provider_key) {
            record_entry_success(entry, success_required);
            if *entry == ModelHealthEntry::default() {
                self.provider_entries.remove(&provider_key);
            }
            changed = true;
        }

        if let Some(entry) = self.model_entries.get_mut(key) {
            record_entry_success(entry, success_required);
            if *entry == ModelHealthEntry::default() {
                self.model_entries.remove(key);
            }
            changed = true;
        }

        changed
    }

    pub fn record_failure(
        &mut self,
        key: &ProviderModelHealthKey,
        kind: GatewayFailureKind,
        now: DateTime<Utc>,
    ) -> bool {
        let weight = classify_failure(kind);
        match weight.scope {
            FailureScope::None => false,
            FailureScope::Model => {
                let threshold = self.settings.model_failure_score_threshold;
                let entry = self.model_entries.entry(key.clone()).or_default();
                add_failure(entry, weight, &self.settings, now);
                if entry.failure_score >= threshold || entry.state == ModelHealthStateKind::Probing
                {
                    open_entry(entry, &self.settings, now);
                } else if entry.failure_score > 0 {
                    entry.state = ModelHealthStateKind::Degraded;
                }
                true
            }
            FailureScope::Provider => {
                let provider_key = ProviderHealthKey::from(key);
                let threshold = self.settings.model_failure_score_threshold;
                let entry = self.provider_entries.entry(provider_key).or_default();
                add_failure(entry, weight, &self.settings, now);
                if entry.failure_score >= threshold || entry.state == ModelHealthStateKind::Probing
                {
                    open_entry(entry, &self.settings, now);
                } else if entry.failure_score > 0 {
                    entry.state = ModelHealthStateKind::Degraded;
                }
                true
            }
        }
    }
}

pub fn list_model_health_items(
    path: &std::path::Path,
    settings: ProxyGatewaySettings,
) -> Result<Vec<GatewayModelHealthItem>, String> {
    Ok(ModelHealthRegistry::load(path, settings)?.health_items())
}

fn health_item_from_model_entry(
    key: &ProviderModelHealthKey,
    entry: &ModelHealthEntry,
) -> GatewayModelHealthItem {
    GatewayModelHealthItem {
        scope: GatewayModelHealthScope::Model,
        cli_key: key.cli_key,
        provider_id: key.provider_id.clone(),
        provider_name: None,
        upstream_model_id: Some(key.upstream_model_id.clone()),
        state: entry.state,
        failure_score: entry.failure_score,
        consecutive_open_count: entry.consecutive_open_count,
        half_open_success_count: entry.half_open_success_count,
        next_retry_at: entry.next_retry_at,
        last_failure_at: entry.last_failure_at,
        last_error_category: entry.last_error_category.clone(),
    }
}

fn health_item_from_provider_entry(
    key: &ProviderHealthKey,
    entry: &ModelHealthEntry,
) -> GatewayModelHealthItem {
    GatewayModelHealthItem {
        scope: GatewayModelHealthScope::Provider,
        cli_key: key.cli_key,
        provider_id: key.provider_id.clone(),
        provider_name: None,
        upstream_model_id: None,
        state: entry.state,
        failure_score: entry.failure_score,
        consecutive_open_count: entry.consecutive_open_count,
        half_open_success_count: entry.half_open_success_count,
        next_retry_at: entry.next_retry_at,
        last_failure_at: entry.last_failure_at,
        last_error_category: entry.last_error_category.clone(),
    }
}

fn record_entry_success(entry: &mut ModelHealthEntry, half_open_success_required: u32) {
    if entry.state == ModelHealthStateKind::Probing {
        entry.half_open_success_count += 1;
        if entry.half_open_success_count < half_open_success_required {
            return;
        }
    }

    *entry = ModelHealthEntry::default();
}

fn add_failure(
    entry: &mut ModelHealthEntry,
    weight: FailureWeight,
    settings: &ProxyGatewaySettings,
    now: DateTime<Utc>,
) {
    if failure_window_elapsed(entry, settings, now) {
        entry.failure_score = 0;
        entry.half_open_success_count = 0;
        entry.last_error_category = None;
        if entry.state == ModelHealthStateKind::Degraded {
            entry.state = ModelHealthStateKind::Healthy;
        }
    }

    entry.failure_score += weight.score;
    entry.last_failure_at = Some(now);
    entry.last_error_category = Some(weight.category.to_string());
}

fn failure_window_elapsed(
    entry: &ModelHealthEntry,
    settings: &ProxyGatewaySettings,
    now: DateTime<Utc>,
) -> bool {
    entry.last_failure_at.is_some_and(|last_failure_at| {
        now.signed_duration_since(last_failure_at)
            > Duration::seconds(settings.model_failure_window_seconds as i64)
    })
}

fn is_cooling(entry: &ModelHealthEntry, now: DateTime<Utc>) -> bool {
    entry.state == ModelHealthStateKind::CoolingDown
        && entry.next_retry_at.is_none_or(|retry_at| retry_at > now)
}

fn open_entry(entry: &mut ModelHealthEntry, settings: &ProxyGatewaySettings, now: DateTime<Utc>) {
    entry.state = ModelHealthStateKind::CoolingDown;
    entry.consecutive_open_count = entry.consecutive_open_count.saturating_add(1);
    entry.half_open_success_count = 0;
    let exponent = entry.consecutive_open_count.saturating_sub(1).min(16);
    let multiplier = 2_u64.saturating_pow(exponent);
    let cooldown_seconds = settings
        .model_base_cooldown_seconds
        .saturating_mul(multiplier)
        .min(settings.model_max_cooldown_seconds);
    entry.next_retry_at = Some(now + Duration::seconds(cooldown_seconds as i64));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::proxy_gateway::types::GatewayCliKey;

    fn key(model: &str) -> ProviderModelHealthKey {
        ProviderModelHealthKey {
            cli_key: GatewayCliKey::Claude,
            provider_id: "provider-a".to_string(),
            upstream_model_id: model.to_string(),
        }
    }

    fn test_settings() -> ProxyGatewaySettings {
        ProxyGatewaySettings {
            model_failure_score_threshold: 5,
            model_base_cooldown_seconds: 60,
            model_max_cooldown_seconds: 600,
            half_open_success_required: 2,
            ..ProxyGatewaySettings::default()
        }
    }

    #[test]
    fn request_schema_failure_has_no_health_penalty() {
        let failure = classify_failure(GatewayFailureKind::RequestSchema);
        assert_eq!(failure.scope, FailureScope::None);
        assert_eq!(failure.score, 0);
    }

    #[test]
    fn auth_failure_is_provider_wide() {
        let failure = classify_failure(GatewayFailureKind::Auth);
        assert_eq!(failure.scope, FailureScope::Provider);
        assert_eq!(failure.score, 5);
    }

    #[test]
    fn model_failure_does_not_affect_other_models_on_same_provider() {
        let now = Utc::now();
        let mut registry = ModelHealthRegistry::new(test_settings());
        let haiku = key("haiku");
        let opus = key("opus");

        registry.record_failure(&haiku, GatewayFailureKind::ModelNotFound, now);

        assert!(!registry.is_model_available(&haiku, now));
        assert!(registry.is_model_available(&opus, now));
    }

    #[test]
    fn rate_limit_accumulates_degraded_before_cooling() {
        let now = Utc::now();
        let mut registry = ModelHealthRegistry::new(test_settings());
        let haiku = key("haiku");

        registry.record_failure(&haiku, GatewayFailureKind::RateLimit, now);
        let entry = registry.model_entry(&haiku).unwrap();

        assert_eq!(entry.state, ModelHealthStateKind::Degraded);
        assert!(registry.is_model_available(&haiku, now));
    }

    #[test]
    fn repeated_rate_limit_enters_cooling() {
        let now = Utc::now();
        let mut registry = ModelHealthRegistry::new(test_settings());
        let haiku = key("haiku");

        registry.record_failure(&haiku, GatewayFailureKind::RateLimit, now);
        registry.record_failure(&haiku, GatewayFailureKind::RateLimit, now);

        let entry = registry.model_entry(&haiku).unwrap();
        assert_eq!(entry.state, ModelHealthStateKind::CoolingDown);
        assert_eq!(entry.next_retry_at, Some(now + Duration::seconds(60)));
    }

    #[test]
    fn cooling_transitions_to_probing_after_retry_time() {
        let now = Utc::now();
        let mut registry = ModelHealthRegistry::new(test_settings());
        let haiku = key("haiku");

        registry.record_failure(&haiku, GatewayFailureKind::ModelNotFound, now);
        registry.refresh_due_cooldowns(now + Duration::seconds(61));

        assert_eq!(
            registry.model_entry(&haiku).unwrap().state,
            ModelHealthStateKind::Probing
        );
    }

    #[test]
    fn probing_requires_configured_success_count() {
        let now = Utc::now();
        let mut registry = ModelHealthRegistry::new(test_settings());
        let haiku = key("haiku");

        registry.record_failure(&haiku, GatewayFailureKind::ModelNotFound, now);
        registry.refresh_due_cooldowns(now + Duration::seconds(61));
        registry.record_success(&haiku);
        assert_eq!(
            registry.model_entry(&haiku).unwrap().state,
            ModelHealthStateKind::Probing
        );

        registry.record_success(&haiku);
        assert!(registry.model_entry(&haiku).is_none());
    }

    #[test]
    fn probing_failure_reopens_with_longer_backoff() {
        let now = Utc::now();
        let mut registry = ModelHealthRegistry::new(test_settings());
        let haiku = key("haiku");

        registry.record_failure(&haiku, GatewayFailureKind::ModelNotFound, now);
        registry.refresh_due_cooldowns(now + Duration::seconds(61));
        registry.record_failure(
            &haiku,
            GatewayFailureKind::Upstream5xx,
            now + Duration::seconds(62),
        );

        let entry = registry.model_entry(&haiku).unwrap();
        assert_eq!(entry.state, ModelHealthStateKind::CoolingDown);
        assert_eq!(entry.consecutive_open_count, 2);
        assert_eq!(entry.next_retry_at, Some(now + Duration::seconds(62 + 120)));
    }

    #[test]
    fn auth_failure_cools_provider_and_blocks_all_models() {
        let now = Utc::now();
        let mut registry = ModelHealthRegistry::new(test_settings());
        let haiku = key("haiku");
        let opus = key("opus");

        registry.record_failure(&haiku, GatewayFailureKind::Auth, now);

        assert!(!registry.is_model_available(&haiku, now));
        assert!(!registry.is_model_available(&opus, now));
        assert!(registry
            .provider_entry(&ProviderHealthKey::from(&haiku))
            .is_some());
    }

    #[test]
    fn provider_probe_success_resets_provider_health() {
        let now = Utc::now();
        let mut registry = ModelHealthRegistry::new(test_settings());
        let haiku = key("haiku");
        let provider_key = ProviderHealthKey::from(&haiku);

        registry.record_failure(&haiku, GatewayFailureKind::Auth, now);
        registry.refresh_due_cooldowns(now + Duration::seconds(61));
        assert_eq!(
            registry.provider_entry(&provider_key).unwrap().state,
            ModelHealthStateKind::Probing
        );

        registry.record_success(&haiku);
        assert_eq!(
            registry.provider_entry(&provider_key).unwrap().state,
            ModelHealthStateKind::Probing
        );

        registry.record_success(&haiku);
        assert!(registry.provider_entry(&provider_key).is_none());

        let next_failure_at = now + Duration::seconds(70);
        registry.record_failure(&haiku, GatewayFailureKind::Auth, next_failure_at);
        let entry = registry.provider_entry(&provider_key).unwrap();
        assert_eq!(entry.consecutive_open_count, 1);
        assert_eq!(
            entry.next_retry_at,
            Some(next_failure_at + Duration::seconds(60))
        );
    }

    #[test]
    fn model_failure_score_resets_after_failure_window() {
        let now = Utc::now();
        let mut settings = test_settings();
        settings.model_failure_window_seconds = 30;
        let mut registry = ModelHealthRegistry::new(settings);
        let haiku = key("haiku");

        registry.record_failure(&haiku, GatewayFailureKind::RateLimit, now);
        let entry = registry.model_entry(&haiku).unwrap();
        assert_eq!(entry.state, ModelHealthStateKind::Degraded);
        assert_eq!(entry.failure_score, 3);
        assert_eq!(entry.last_failure_at, Some(now));

        let outside_window = now + Duration::seconds(31);
        registry.record_failure(&haiku, GatewayFailureKind::RateLimit, outside_window);
        let entry = registry.model_entry(&haiku).unwrap();
        assert_eq!(entry.state, ModelHealthStateKind::Degraded);
        assert_eq!(entry.failure_score, 3);
        assert_eq!(entry.last_failure_at, Some(outside_window));
        assert!(registry.is_model_available(&haiku, outside_window));

        let inside_window = outside_window + Duration::seconds(1);
        registry.record_failure(&haiku, GatewayFailureKind::RateLimit, inside_window);
        let entry = registry.model_entry(&haiku).unwrap();
        assert_eq!(entry.state, ModelHealthStateKind::CoolingDown);
        assert_eq!(entry.failure_score, 6);
    }

    #[test]
    fn model_health_registry_persists_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model-health.json");
        let now = Utc::now();
        let mut registry = ModelHealthRegistry::new(test_settings());
        let haiku = key("haiku");

        registry.record_failure(&haiku, GatewayFailureKind::ModelNotFound, now);
        registry.save(&path).unwrap();

        let loaded = ModelHealthRegistry::load(&path, test_settings()).unwrap();
        let entry = loaded.model_entry(&haiku).unwrap();
        assert_eq!(entry.state, ModelHealthStateKind::CoolingDown);
        assert_eq!(
            entry.last_error_category.as_deref(),
            Some("model_not_found")
        );
        assert_eq!(loaded.health_items().len(), 1);
    }

    #[test]
    fn record_failure_incomplete_stream_maps_to_empty_response_score() {
        // `amend_health_after_stream` maps stream Incomplete -> EmptyResponse
        // (score 2, Model scope). Verify the underlying contract: a single
        // Incomplete failure accumulates the EmptyResponse score.
        let mut registry = ModelHealthRegistry::new(test_settings());
        let key = key("incomplete");
        let now = Utc::now();
        registry.record_failure(&key, GatewayFailureKind::EmptyResponse, now);
        let entry = registry.model_entry(&key).expect("entry created");
        assert_eq!(entry.failure_score, 2);
        assert_eq!(entry.last_error_category.as_deref(), Some("empty_response"));
    }

    #[test]
    fn record_failure_client_cancelled_is_health_neutral() {
        // `amend_health_after_stream` skips Canceled by design (ClientCancelled
        // scores 0, scope None), so a client disconnect never penalizes the
        // provider. Verify no entry is ever created for a canceled outcome.
        let mut registry = ModelHealthRegistry::new(test_settings());
        let key = key("canceled");
        let now = Utc::now();
        registry.record_failure(&key, GatewayFailureKind::ClientCancelled, now);
        assert!(
            registry.model_entry(&key).is_none(),
            "ClientCancelled must not create a health entry"
        );
    }
}
