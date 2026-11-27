//! Serializable DTOs for the gateway plugin system.
//!
//! Records are stored as camelCase JSON so the same shape crosses the Tauri
//! boundary without a translation layer. The SQLite row still carries its own
//! `created_at` / `updated_at` columns; payloads that need to show a timestamp
//! carry their own `installedAt` / `updatedAt` fields.

use std::collections::BTreeMap;

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
    /// Reads the package asks its pages to be granted (`magpie.permissions`).
    #[serde(default)]
    pub permissions: Vec<String>,
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
    /// A package that is only a page: it has no provider entry and no page
    /// declares a local service, so nothing of it belongs in the Bun host.
    pub fn is_page_only(&self) -> bool {
        !self.has_provider_entry
            && !self.pages.is_empty()
            && self.pages.iter().all(|page| page.service_port.is_none())
    }
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
    /// Whether market and npm requests go through the app proxy.
    #[serde(default = "default_true")]
    pub follow_app_proxy: bool,
}

impl Default for PluginSettings {
    fn default() -> Self {
        Self {
            bun_manual_path: String::new(),
            npm_registry: default_npm_registry(),
            follow_app_proxy: true,
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
        assert!(record.permissions.is_empty());
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
            permissions: Vec::new(),
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

        let parsed: PluginSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(parsed.npm_registry, settings.npm_registry);
        assert!(parsed.follow_app_proxy);
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
