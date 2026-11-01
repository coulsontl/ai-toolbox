use serde::{Deserialize, Serialize};

/// What a 2API channel is backed by.
///
/// `Http` is a plain upstream (base URL + key) that the existing gateway
/// runtime forwards to after protocol conversion. `Plugin` is served by an
/// installed OpenCode plugin: the plugin's `auth.loader` returns the `fetch`
/// that carries the request upstream, so the gateway never talks to the
/// vendor itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayChannelKind {
    #[default]
    Http,
    Plugin,
}

impl RelayChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Plugin => "plugin",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "plugin" => Self::Plugin,
            _ => Self::Http,
        }
    }
}

/// A channel a generic API client is routed to.
///
/// Not a `*_provider` table on purpose: those are bound to one CLI's runtime
/// config and are parsed per CLI in `proxy_gateway::runtime::providers`. A 2API
/// channel has no CLI, so it carries everything the runtime needs directly.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayChannel {
    pub id: String,
    pub name: String,
    pub kind: RelayChannelKind,
    /// Upstream origin for `Http` channels (a trailing `##` marks a full URL,
    /// same as the CLI provider tables).
    pub base_url: String,
    pub api_key: String,
    /// Upstream wire protocol, e.g. `openai_chat` / `anthropic_messages`.
    /// Only meaningful for `Http`; plugin channels derive it from the model's
    /// npm (`pluginProtocolFromNpm`).
    pub target_protocol: String,
    /// Which plugin provider serves this channel (`Plugin` only).
    pub plugin_provider_id: String,
    pub enabled: bool,
    pub sort_index: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Default for RelayChannel {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: RelayChannelKind::Http,
            base_url: String::new(),
            api_key: String::new(),
            target_protocol: String::new(),
            plugin_provider_id: String::new(),
            enabled: true,
            sort_index: 0,
            created_at: 0,
            updated_at: 0,
        }
    }
}

/// A model exposed by a 2API channel.
///
/// `npm` is what decides the wire protocol of a plugin-backed channel; it is
/// copied from the plugin's model entry when the channel is refreshed.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayModel {
    pub id: String,
    pub channel_id: String,
    pub model_id: String,
    pub display_name: String,
    pub npm: String,
    pub enabled: bool,
    pub sort_index: i64,
}

impl Default for RelayModel {
    fn default() -> Self {
        Self {
            id: String::new(),
            channel_id: String::new(),
            model_id: String::new(),
            display_name: String::new(),
            npm: String::new(),
            enabled: true,
            sort_index: 0,
        }
    }
}

/// An installed OpenCode plugin.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayPlugin {
    pub id: String,
    /// How it was added: an npm spec, a git spec, or a local path.
    pub spec: String,
    /// Where it lives on disk (the package directory).
    pub target: String,
    pub version: String,
    pub enabled: bool,
    /// The user's option values, handed to the plugin as its second argument.
    pub options_json: String,
    pub installed_at: i64,
}

impl Default for RelayPlugin {
    fn default() -> Self {
        Self {
            id: String::new(),
            spec: String::new(),
            target: String::new(),
            version: String::new(),
            enabled: true,
            options_json: "{}".to_string(),
            installed_at: 0,
        }
    }
}

/// One provider a plugin declares through its `auth` / `provider` hooks.
///
/// A single npm package can export several plugin functions (workbuddy
/// exports two, one per site), so `entry_export` records which export produced
/// this provider. Without it the host cannot tell them apart.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayPluginProvider {
    pub id: String,
    pub plugin_id: String,
    pub provider_id: String,
    pub label: String,
    /// The module export that produced this provider (e.g. `WorkBuddyAuthPlugin`).
    pub entry_export: String,
    pub npm: String,
    pub capabilities_json: String,
    pub enabled: bool,
}

impl Default for RelayPluginProvider {
    fn default() -> Self {
        Self {
            id: String::new(),
            plugin_id: String::new(),
            provider_id: String::new(),
            label: String::new(),
            entry_export: String::new(),
            npm: String::new(),
            capabilities_json: "{}".to_string(),
            enabled: true,
        }
    }
}

/// A sign-in for a plugin provider.
///
/// `auth_json` holds OpenCode's `auth.json` shape verbatim (`{type:"api",key}`
/// or `{type:"oauth",access,refresh,expires}`); plugins read and write it
/// through `getAuth()` and `client.auth.set`, so its shape must not be
/// reshaped by us.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayPluginAccount {
    pub id: String,
    pub plugin_provider_id: String,
    /// The plugin-auth store key (`<provider>` or `<provider>#<slot>`).
    pub account_key: String,
    pub display_name: String,
    pub auth_json: String,
    pub last_used_at: i64,
}

impl Default for RelayPluginAccount {
    fn default() -> Self {
        Self {
            id: String::new(),
            plugin_provider_id: String::new(),
            account_key: String::new(),
            display_name: String::new(),
            auth_json: "{}".to_string(),
            last_used_at: 0,
        }
    }
}

/// An inbound key a 2API client authenticates with.
///
/// Only the hash is stored; the plaintext key is shown once at creation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayAccessKey {
    pub id: String,
    pub name: String,
    pub key_hash: String,
    /// Leading characters, so the user can tell keys apart in a list.
    pub key_prefix: String,
    pub enabled: bool,
    pub last_used_at: i64,
    pub created_at: i64,
}

impl Default for RelayAccessKey {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            key_hash: String::new(),
            key_prefix: String::new(),
            enabled: true,
            last_used_at: 0,
            created_at: 0,
        }
    }
}

/// A daily check-in plan.
///
/// The plugin presses its vendor's check-in through `auth.checkin`; this plan
/// only decides when that happens. The shape follows agent2api's schedule
/// (`time` window, per-provider overrides, retry policy) without importing any
/// of its vendor-specific business logic.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayCheckinSchedule {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    /// Daily trigger time, `HH:MM` in local time.
    pub time: String,
    /// Window end, `HH:MM`; empty means no window.
    pub time_end: String,
    pub providers: Vec<String>,
    /// Per-provider trigger override (`provider id -> HH:MM`).
    pub provider_times: std::collections::BTreeMap<String, String>,
    /// Retry pass time, `HH:MM`; empty turns the retry pass off.
    pub retry_time: String,
    /// Minutes between retries for a neutral (not failed) outcome; 0 = no retry.
    pub retry_gap_minutes: i64,
    /// Retries per account per day; 0 = no retry.
    pub retry_limit: i64,
    pub last_fired_date: String,
    pub account_plan_json: String,
}

impl Default for RelayCheckinSchedule {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            enabled: true,
            time: "00:01".to_string(),
            time_end: String::new(),
            providers: Vec::new(),
            provider_times: std::collections::BTreeMap::new(),
            retry_time: String::new(),
            retry_gap_minutes: 0,
            retry_limit: 0,
            last_fired_date: String::new(),
            account_plan_json: "{}".to_string(),
        }
    }
}

/// One check-in attempt, kept as a browsable ledger row.
///
/// Written even when it fails, because the point is to answer "what did this
/// account get, and when" long after the run. A failed write never fails the
/// check-in itself — the vendor already granted the credit by then.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayCheckinRecord {
    pub id: String,
    pub ts: i64,
    pub date: String,
    /// `schedule` / `manual` / `single` / `startup`.
    pub source: String,
    pub plugin_provider_id: String,
    pub provider: String,
    pub account_id: String,
    pub account_name: String,
    /// `claimed` / `done` / `ineligible` / `inactive` / `captcha` / `failed`.
    pub outcome: String,
    pub ok: bool,
    pub credit: f64,
    pub streak: i64,
    pub message: String,
    /// The vendor's own day boundary, when it told us.
    pub until: String,
}

impl Default for RelayCheckinRecord {
    fn default() -> Self {
        Self {
            id: String::new(),
            ts: 0,
            date: String::new(),
            source: String::new(),
            plugin_provider_id: String::new(),
            provider: String::new(),
            account_id: String::new(),
            account_name: String::new(),
            outcome: String::new(),
            ok: false,
            credit: 0.0,
            streak: 0,
            message: String::new(),
            until: String::new(),
        }
    }
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}
