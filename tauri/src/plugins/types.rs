//! Serializable DTOs for the gateway plugin system.
//!
//! Records are stored as camelCase JSON so the same shape crosses the Tauri
//! boundary without a translation layer. The SQLite row still carries its own
//! `created_at` / `updated_at` columns; payloads that need to show a timestamp
//! carry their own `installedAt` / `updatedAt` fields.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a plugin contributes to the gateway.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    Provider,
    Middleware,
    Both,
    /// A package that only contributes a custom page (`magpie.page` /
    /// `magpie.pages`) with no provider or middleware entry.
    Page,
    #[default]
    Unknown,
}

impl PluginKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::Middleware => "middleware",
            Self::Both => "both",
            Self::Page => "page",
            Self::Unknown => "unknown",
        }
    }

    /// Map a registry / package `kind` string onto the enum. Provider packages
    /// leave `kind` empty, which is why the empty string means Provider here.
    pub fn from_listing_kind(kind: &str) -> Self {
        match kind.trim().to_ascii_lowercase().as_str() {
            "" | "provider" => Self::Provider,
            "middleware" => Self::Middleware,
            "both" => Self::Both,
            "page" => Self::Page,
            _ => Self::Unknown,
        }
    }
}

/// One page a plugin contributes to the host window.
///
/// `entry_file` and `icon_file` are paths relative to the plugin package root,
/// always with forward slashes. Parsing rejects absolute paths, `..`, and any
/// file that resolves outside the package.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPageManifest {
    /// `default` for `magpie.page`, an explicit `id` or a safe slug for an
    /// entry of `magpie.pages`.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub titles: BTreeMap<String, String>,
    pub entry_file: String,
    #[serde(default)]
    pub icon_file: Option<String>,
    #[serde(default)]
    pub keep_alive: bool,
    /// Declared port of the plugin's local HTTP service (`magpie.page.service.port`).
    #[serde(default)]
    pub service_port: Option<u16>,
    /// Plugin option that moves `service_port` (`magpie.page.service.option`).
    #[serde(default)]
    pub service_option: Option<String>,
}

/// One item a plugin adds to one of the host's menus (`magpie.actions`).
///
/// Actions are stored on the installed record; a registered action adds the
/// owning plugin and the page it opens.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginActionManifest {
    /// Lower-case letters, digits and dashes (`^[a-z0-9][a-z0-9-]{0,39}$`).
    pub id: String,
    /// `provider` or `agent`.
    pub menu: String,
    pub title: String,
    #[serde(default)]
    pub titles: BTreeMap<String, String>,
}

/// One panel a plugin adds to one of the host's own pages (`magpie.panels`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPanelManifest {
    pub id: String,
    /// `provider`, `settings`, `usage` or `agents`.
    pub slot: String,
    pub title: String,
    #[serde(default)]
    pub titles: BTreeMap<String, String>,
    /// Panel HTML file, relative to the package root with forward slashes.
    pub entry_file: String,
}

/// One action of an enabled plugin, ready for the host's menus.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredPluginAction {
    pub plugin_id: String,
    pub plugin_name: String,
    /// The plugin page picking this action opens.
    pub page_id: String,
    pub id: String,
    pub menu: String,
    pub title: String,
    #[serde(default)]
    pub titles: BTreeMap<String, String>,
}

/// One panel of an enabled plugin, ready for the host's own pages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredPluginPanel {
    pub plugin_id: String,
    pub plugin_name: String,
    pub panel_id: String,
    pub slot: String,
    pub title: String,
    #[serde(default)]
    pub titles: BTreeMap<String, String>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

/// One asynchronous gateway hook's aggregate diagnostics, as the Bun host
/// reports them and the plugins page shows them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayHookStateDto {
    pub plugin_id: String,
    pub plugin_spec: String,
    pub plugin_name: String,
    pub calls: u64,
    pub failures: u64,
    pub avg_millis: f64,
    pub last_outcome: String,
    pub last_bytes: u64,
}

/// One page of an enabled plugin, ready for the frontend's page sidebar.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredPluginPage {
    pub plugin_id: String,
    pub plugin_name: String,
    pub package_name: String,
    pub plugin_version: String,
    /// The plugin package's own icon name (`magpie.icon`), not a data URL.
    pub plugin_icon: String,
    pub page_id: String,
    pub title: String,
    #[serde(default)]
    pub titles: BTreeMap<String, String>,
    /// The page icon as `data:<mime>;base64,...`, when it exists and is small.
    #[serde(default)]
    pub icon_data_url: Option<String>,
    #[serde(default)]
    pub keep_alive: bool,
    #[serde(default)]
    pub service_port: Option<u16>,
    #[serde(default)]
    pub service_option: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

/// Everything a host page needs to load one plugin page through the local
/// page server.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPageSession {
    pub plugin_id: String,
    pub page_id: String,
    /// Token-scoped `http://127.0.0.1:<port>/_plugin/<token>/<entry>`.
    pub url: String,
    /// The plugin's own local service, when it declares one.
    #[serde(default)]
    pub service_url: Option<String>,
    #[serde(default = "empty_json_object")]
    pub options: Value,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub keep_alive: bool,
}

/// What a plugin package declares beyond its provider entry.
///
/// The installer resolves these from the package manifest. `unsupported`
/// lists the declarations this build does not consume yet, so the UI can say
/// so instead of pretending the whole package works.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginContributions {
    #[serde(default)]
    pub has_hooks: bool,
    #[serde(default)]
    pub hooks_entry: String,
    #[serde(default)]
    pub has_panels: bool,
    #[serde(default)]
    pub has_actions: bool,
    #[serde(default)]
    pub has_replaces: bool,
    #[serde(default)]
    pub has_agent: bool,
    #[serde(default)]
    pub has_privacy_models: bool,
    #[serde(default)]
    pub unsupported_features: Vec<String>,
}

/// One installed plugin package.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPluginRecord {
    pub id: String,
    pub spec: String,
    pub package_name: String,
    pub display_name: String,
    pub version: String,
    pub kind: PluginKind,
    pub enabled: bool,
    /// `npm` | `github` | `local`.
    pub source_type: String,
    #[serde(default)]
    pub market_id: Option<String>,
    #[serde(default)]
    pub local_path: String,
    #[serde(default)]
    pub middleware_entry: String,
    #[serde(default)]
    pub has_provider_entry: bool,
    #[serde(default)]
    pub providers: Vec<String>,
    /// Custom pages the package declares (`magpie.page` / `magpie.pages`).
    #[serde(default)]
    pub pages: Vec<PluginPageManifest>,
    /// Items the package adds to the host's menus (`magpie.actions`).
    #[serde(default)]
    pub actions: Vec<PluginActionManifest>,
    /// Panels the package adds to the host's own pages (`magpie.panels`).
    #[serde(default)]
    pub panels: Vec<PluginPanelManifest>,
    /// Reads the package asks its pages to be granted (`magpie.permissions`).
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Capability matrix resolved from the package manifest.
    #[serde(default)]
    pub contributions: PluginContributions,
    #[serde(default = "empty_json_object")]
    pub options: Value,
    #[serde(default)]
    pub summary: BTreeMap<String, String>,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub installed_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl InstalledPluginRecord {
    /// A package that is only UI: it has no provider entry, no asynchronous
    /// gateway hooks and no page declares a local service, so nothing of it
    /// belongs in the Bun host.
    pub fn is_page_only(&self) -> bool {
        !self.has_provider_entry
            && !self.contributions.has_hooks
            && (!self.pages.is_empty() || !self.panels.is_empty())
            && self.pages.iter().all(|page| page.service_port.is_none())
    }
}

/// One plugin tool as the tools MCP server and the plugins page name it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginToolDto {
    pub plugin_id: String,
    pub plugin_spec: String,
    pub plugin_name: String,
    pub tool_name: String,
    pub mcp_name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Every tool the enabled provider plugins offer, plus where the gateway
/// serves them as a Streamable HTTP MCP server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginToolsSummaryDto {
    pub endpoint_url: String,
    pub alias_url: String,
    pub gateway_running: bool,
    pub mcp_server_id: Option<String>,
    pub tools: Vec<PluginToolDto>,
}

/// One configured market registry.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketSourceRecord {
    pub id: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    /// The built-in Magpie Community source. It may be disabled but never
    /// deleted, so the offline fallback list always has an owner.
    pub builtin: bool,
    #[serde(default)]
    pub plugin_count: usize,
    #[serde(default)]
    pub last_synced_at: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}

/// One listing after every market has been merged and npm metadata applied.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketPluginListing {
    pub package: String,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default)]
    pub community: bool,
    #[serde(default)]
    pub replaces: Option<String>,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub summary: BTreeMap<String, String>,
    /// Every market that lists this package, in source order.
    #[serde(default)]
    pub market_ids: Vec<String>,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub installed_version: Option<String>,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub weekly_downloads: Option<u64>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
}

/// The singleton plugin settings record (`plugin_entity` / `settings`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettings {
    /// Explicit Bun executable; empty means "find `bun` on PATH".
    #[serde(default)]
    pub bun_manual_path: String,
    #[serde(default = "default_npm_registry")]
    pub npm_registry: String,
    /// Whether market/npm requests and the plugin host's vendor calls go
    /// through the app proxy.
    #[serde(default = "default_true")]
    pub follow_app_proxy: bool,
    /// Explicit per-provider auto check-in switches, keyed by provider id.
    ///
    /// Providers absent from this map fall back to the legacy account-level
    /// `PluginAccountMetaRecord::auto_checkin` flags; once the user toggles a
    /// channel, its explicit value wins from then on.
    #[serde(default)]
    pub provider_auto_checkin: HashMap<String, bool>,
    /// Whether the background pass checks enabled, unpinned npm plugins.
    #[serde(default)]
    pub auto_update_enabled: bool,
    /// How often the background pass checks for updates; clamped on save.
    #[serde(default = "default_auto_update_interval_hours")]
    pub auto_update_interval_hours: u32,
    /// Backend-owned timestamp of the last full check. Only
    /// `store::set_last_auto_update_check_at` may change it.
    #[serde(default)]
    pub last_auto_update_check_at: Option<String>,
}

impl Default for PluginSettings {
    fn default() -> Self {
        Self {
            bun_manual_path: String::new(),
            npm_registry: default_npm_registry(),
            follow_app_proxy: true,
            provider_auto_checkin: HashMap::new(),
            auto_update_enabled: false,
            auto_update_interval_hours: default_auto_update_interval_hours(),
            last_auto_update_check_at: None,
        }
    }
}

/// Per-provider gateway options supplied to a plugin.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginProviderOptionValue {
    pub provider_id: String,
    #[serde(default = "empty_json_object")]
    pub options: Value,
    #[serde(default)]
    pub updated_at: String,
}

/// Display metadata and cached quota / check-in snapshots for one account.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginAccountMetaRecord {
    pub id: String,
    pub plugin_id: String,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub sort_index: i64,
    #[serde(default = "empty_json_object")]
    pub quota: Value,
    #[serde(default)]
    pub quota_updated_at: Option<String>,
    #[serde(default = "empty_json_object")]
    pub checkin: Value,
    /// Whether the background pass checks this account in once a day.
    #[serde(default)]
    pub auto_checkin: bool,
    /// Beijing day (`YYYY-MM-DD`) of the last automatic or manual check-in.
    #[serde(default)]
    pub last_checkin_day: Option<String>,
    /// The vendor's own day boundary (`until`) reported by the check-in.
    #[serde(default)]
    pub last_checkin_until: Option<String>,
    /// `claimed` | `done` | `ineligible` | `inactive` | `captcha` | `failed`.
    #[serde(default)]
    pub last_checkin_outcome: Option<String>,
    #[serde(default)]
    pub last_checkin_at: Option<String>,
    #[serde(default)]
    pub last_checkin_message: Option<String>,
    #[serde(default)]
    pub last_checkin_credit: Option<f64>,
    #[serde(default)]
    pub last_checkin_streak: Option<u64>,
    #[serde(default)]
    pub updated_at: String,
}

impl Default for PluginAccountMetaRecord {
    fn default() -> Self {
        Self {
            id: String::new(),
            plugin_id: String::new(),
            provider_id: String::new(),
            display_name: String::new(),
            enabled: true,
            sort_index: 0,
            quota: empty_json_object(),
            quota_updated_at: None,
            checkin: empty_json_object(),
            auto_checkin: false,
            last_checkin_day: None,
            last_checkin_until: None,
            last_checkin_outcome: None,
            last_checkin_at: None,
            last_checkin_message: None,
            last_checkin_credit: None,
            last_checkin_streak: None,
            updated_at: String::new(),
        }
    }
}

pub fn empty_json_object() -> Value {
    Value::Object(Default::default())
}

fn default_npm_registry() -> String {
    "https://registry.npmjs.org".to_string()
}

fn default_auto_update_interval_hours() -> u32 {
    24
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plugin_kind_serializes_to_the_frontend_spelling() {
        assert_eq!(
            serde_json::to_value(PluginKind::Provider).unwrap(),
            json!("provider")
        );
        assert_eq!(
            serde_json::to_value(PluginKind::Middleware).unwrap(),
            json!("middleware")
        );
        assert_eq!(
            serde_json::to_value(PluginKind::Both).unwrap(),
            json!("both")
        );
        assert_eq!(
            serde_json::to_value(PluginKind::Page).unwrap(),
            json!("page")
        );
        assert_eq!(
            serde_json::to_value(PluginKind::Unknown).unwrap(),
            json!("unknown")
        );
        assert_eq!(PluginKind::from_listing_kind(""), PluginKind::Provider);
        assert_eq!(
            PluginKind::from_listing_kind("MIDDLEWARE"),
            PluginKind::Middleware
        );
        assert_eq!(PluginKind::from_listing_kind("page"), PluginKind::Page);
        assert_eq!(PluginKind::Page.as_str(), "page");
    }

    #[test]
    fn page_only_records_never_reach_the_bun_host() {
        let page = PluginPageManifest {
            id: "default".to_string(),
            entry_file: "page/index.html".to_string(),
            ..PluginPageManifest::default()
        };
        let mut record = InstalledPluginRecord {
            pages: vec![page.clone()],
            ..InstalledPluginRecord::default()
        };
        assert!(record.is_page_only());

        record.has_provider_entry = true;
        assert!(!record.is_page_only());

        record.has_provider_entry = false;
        record.pages[0].service_port = Some(3477);
        assert!(!record.is_page_only());

        record.pages.clear();
        assert!(!record.is_page_only());

        // A panel-only package is UI too, unless it also contributes hooks.
        record.panels.push(PluginPanelManifest {
            id: "main".to_string(),
            entry_file: "panel/index.html".to_string(),
            ..PluginPanelManifest::default()
        });
        assert!(record.is_page_only());
        record.contributions.has_hooks = true;
        assert!(!record.is_page_only());
    }

    #[test]
    fn pages_and_permissions_default_to_empty_for_old_records() {
        let record: InstalledPluginRecord = serde_json::from_value(json!({
            "id": "demo",
            "spec": "demo",
            "packageName": "demo",
            "displayName": "Demo",
            "version": "1.0.0",
            "kind": "provider",
            "enabled": true,
            "sourceType": "npm"
        }))
        .unwrap();
        assert!(record.pages.is_empty());
        assert!(record.panels.is_empty());
        assert!(record.actions.is_empty());
        assert!(record.permissions.is_empty());
        assert_eq!(record.contributions, PluginContributions::default());
    }

    #[test]
    fn contributions_use_the_frontend_camel_case_shape() {
        let contributions = PluginContributions {
            has_hooks: true,
            hooks_entry: "index.hooks.ts".to_string(),
            has_panels: true,
            has_actions: false,
            has_replaces: true,
            has_agent: false,
            has_privacy_models: true,
            unsupported_features: vec![
                "replaces".to_string(),
                "agent".to_string(),
                "privacyModels".to_string(),
            ],
        };
        let value = serde_json::to_value(&contributions).unwrap();
        assert_eq!(value["hasHooks"], true);
        assert_eq!(value["hooksEntry"], "index.hooks.ts");
        assert_eq!(value["hasPanels"], true);
        assert_eq!(value["hasActions"], false);
        assert_eq!(value["hasReplaces"], true);
        assert_eq!(value["hasAgent"], false);
        assert_eq!(value["hasPrivacyModels"], true);
        assert_eq!(value["unsupportedFeatures"][2], "privacyModels");
        assert!(value.get("has_hooks").is_none());
    }

    #[test]
    fn action_panel_and_hook_state_dtos_use_the_frontend_camel_case_shape() {
        let action = RegisteredPluginAction {
            plugin_id: "demo".to_string(),
            plugin_name: "Demo".to_string(),
            page_id: "default".to_string(),
            id: "refresh".to_string(),
            menu: "provider".to_string(),
            title: "Refresh".to_string(),
            titles: BTreeMap::from([("zh".to_string(), "刷新".to_string())]),
        };
        let value = serde_json::to_value(&action).unwrap();
        assert_eq!(value["pluginId"], "demo");
        assert_eq!(value["pluginName"], "Demo");
        assert_eq!(value["pageId"], "default");
        assert_eq!(value["menu"], "provider");
        assert_eq!(value["titles"]["zh"], "刷新");
        assert!(value.get("plugin_id").is_none());

        let panel = RegisteredPluginPanel {
            plugin_id: "demo".to_string(),
            plugin_name: "Demo".to_string(),
            panel_id: "settings-1".to_string(),
            slot: "settings".to_string(),
            title: "Panel".to_string(),
            titles: BTreeMap::new(),
            permissions: vec!["usage:summary".to_string()],
        };
        let value = serde_json::to_value(&panel).unwrap();
        assert_eq!(value["panelId"], "settings-1");
        assert_eq!(value["slot"], "settings");
        assert_eq!(value["permissions"][0], "usage:summary");

        let state = GatewayHookStateDto {
            plugin_id: "demo".to_string(),
            plugin_spec: "demo".to_string(),
            plugin_name: "Demo".to_string(),
            calls: 3,
            failures: 1,
            avg_millis: 12.5,
            last_outcome: "ready".to_string(),
            last_bytes: 64,
        };
        let value = serde_json::to_value(&state).unwrap();
        assert_eq!(value["pluginSpec"], "demo");
        assert_eq!(value["avgMillis"], 12.5);
        assert_eq!(value["lastOutcome"], "ready");
        assert_eq!(value["lastBytes"], 64);
    }

    #[test]
    fn records_use_camel_case_on_the_wire() {
        let record = InstalledPluginRecord {
            id: "p1".to_string(),
            spec: "@scope/pkg".to_string(),
            package_name: "@scope/pkg".to_string(),
            display_name: "Pkg".to_string(),
            version: "1.0.0".to_string(),
            kind: PluginKind::Provider,
            enabled: true,
            source_type: "npm".to_string(),
            market_id: Some("magpie-community".to_string()),
            local_path: String::new(),
            middleware_entry: String::new(),
            has_provider_entry: true,
            providers: vec!["demo".to_string()],
            pages: Vec::new(),
            actions: Vec::new(),
            panels: Vec::new(),
            permissions: Vec::new(),
            contributions: PluginContributions::default(),
            options: json!({ "region": "global" }),
            summary: BTreeMap::new(),
            icon: "demo".to_string(),
            installed_at: "2026-11-26T00:00:00Z".to_string(),
            updated_at: "2026-11-26T00:00:00Z".to_string(),
            last_error: None,
        };
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(value["packageName"], "@scope/pkg");
        assert_eq!(value["displayName"], "Pkg");
        assert_eq!(value["sourceType"], "npm");
        assert_eq!(value["marketId"], "magpie-community");
        assert_eq!(value["hasProviderEntry"], true);
        assert_eq!(value["installedAt"], "2026-11-26T00:00:00Z");
        assert!(value.get("package_name").is_none());
    }

    #[test]
    fn settings_default_to_the_public_registry_and_app_proxy() {
        let settings = PluginSettings::default();
        assert_eq!(settings.npm_registry, "https://registry.npmjs.org");
        assert!(settings.follow_app_proxy);
        assert!(settings.bun_manual_path.is_empty());
        assert!(!settings.auto_update_enabled);
        assert_eq!(settings.auto_update_interval_hours, 24);
        assert!(settings.last_auto_update_check_at.is_none());

        let parsed: PluginSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(parsed.npm_registry, settings.npm_registry);
        assert!(parsed.follow_app_proxy);
        assert!(!parsed.auto_update_enabled);
        assert_eq!(parsed.auto_update_interval_hours, 24);
        assert!(parsed.last_auto_update_check_at.is_none());
    }

    #[test]
    fn account_meta_defaults_to_enabled_true() {
        let meta = PluginAccountMetaRecord::default();
        assert!(meta.enabled);
        assert!(!meta.auto_checkin);
        assert!(meta.quota.is_object());
        assert!(meta.checkin.is_object());
    }
}
