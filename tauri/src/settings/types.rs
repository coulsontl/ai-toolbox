use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// WebDAV configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebDAVConfig {
    pub url: String,
    pub username: String,
    pub password: String,
    pub remote_path: String,
    pub host_label: String,
}

/// S3 configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct S3Config {
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
    pub region: String,
    pub prefix: String,
    pub endpoint_url: String,
    pub force_path_style: bool,
    pub public_domain: String,
}

/// Optional backup encryption configuration. `credential_ref` documents where the
/// password lives (fixed OS credential-store entry); the secret itself is never
/// stored in settings. Missing fields default to disabled encryption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupEncryptionConfig {
    pub enabled: bool,
    pub credential_ref: String,
}

impl Default for BackupEncryptionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            credential_ref: "keyring:ai-toolbox-backup-encryption".to_string(),
        }
    }
}

/// Custom file or directory included in backup archives
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackupCustomEntryType {
    File,
    Directory,
}

impl Default for BackupCustomEntryType {
    fn default() -> Self {
        Self::File
    }
}

/// `#[serde(default)]` only covers a MISSING key; an explicit `null` (hand
/// edit, older writer) would fail the whole entry and the adapter's filter_map
/// would silently drop the user's custom backup entry. Read null as empty.
fn null_as_empty_string_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}

/// User-defined local file/directory that should be included in backups
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BackupCustomEntry {
    pub id: String,
    pub name: String,
    pub source_path: String,
    pub restore_path: Option<String>,
    pub entry_type: BackupCustomEntryType,
    pub enabled: bool,
    /// Regex patterns excluding files/subdirectories from a directory entry.
    /// Each pattern is matched (standard regex search, case-sensitive) against
    /// the `/`-separated path relative to the entry root; a matched directory
    /// prunes its whole subtree. Only applied to directory entries at pack
    /// time, so restore stays consistent by construction.
    #[serde(default, deserialize_with = "null_as_empty_string_vec")]
    pub exclude_patterns: Vec<String>,
}

/// Filter rule for excluding specific file paths from backup/restore
///
/// Controls whether a path from a specific tool should be excluded from
/// the backup archive and skipped during restore.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupFileFilterRule {
    /// Tool identifier: opencode, claude, codex, grok, openclaw, geminicli
    pub tool: String,
    /// File path for the tool. UI values use portable paths such as
    /// "~/.codex/auth.json"; backup-internal relative paths are normalized when matching.
    pub file_path: String,
}

/// Select option for file filter rules, derived from files that would
/// currently be written under external-configs/<tool>/ in a backup archive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupFileFilterPathOption {
    pub tool: String,
    pub file_path: String,
}

impl Default for BackupFileFilterRule {
    fn default() -> Self {
        Self {
            tool: String::new(),
            file_path: String::new(),
        }
    }
}

/// The gateway statistics page's time-range selection, persisted so the tray
/// usage block can follow the same window.
///
/// `preset` is one of the frontend `GATEWAY_USAGE_RANGE_PRESETS` values. Only
/// `custom` carries absolute bounds (`start_date` / `end_date`, unix seconds);
/// relative presets are re-resolved from the current local clock every time the
/// tray menu is built, so "today"/"this week" never freeze at save time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayUsageRangeSelection {
    pub preset: String,
    /// Unix seconds; only meaningful when `preset == "custom"`.
    #[serde(default)]
    pub start_date: Option<i64>,
    /// Unix seconds; only meaningful when `preset == "custom"`.
    #[serde(default)]
    pub end_date: Option<i64>,
}

impl Default for GatewayUsageRangeSelection {
    fn default() -> Self {
        // Matches the statistics page's own default (`{ preset: 'today' }`).
        Self {
            preset: "today".to_string(),
            start_date: None,
            end_date: None,
        }
    }
}

/// Session-detail filter chip visibility persisted across app restarts.
///
/// Mirrors the frontend `SessionDetailWorkbench` role/content filters. All
/// booleans are "visible" (true = show). Missing/None means "all visible".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDetailFilters {
    pub role_filter: SessionRoleFilter,
    pub content_filter: SessionContentFilter,
}

impl Default for SessionDetailFilters {
    fn default() -> Self {
        Self {
            role_filter: SessionRoleFilter::default(),
            content_filter: SessionContentFilter::default(),
        }
    }
}

/// Role-level visibility: user vs assistant messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRoleFilter {
    pub user: bool,
    pub assistant: bool,
}

impl Default for SessionRoleFilter {
    fn default() -> Self {
        Self {
            user: true,
            assistant: true,
        }
    }
}

/// Content-level visibility: text, thinking, tool calls, commands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionContentFilter {
    pub text: bool,
    pub thinking: bool,
    pub tool_call: bool,
    pub command: bool,
}

impl Default for SessionContentFilter {
    fn default() -> Self {
        Self {
            text: true,
            thinking: true,
            tool_call: true,
            command: true,
        }
    }
}

/// Application settings
///
/// Note: This struct is no longer directly serialized to/from database.
/// Use the adapter layer (settings/adapter.rs) for all database operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub language: String,
    pub current_module: String,
    pub current_sub_tab: String,
    pub backup_type: String,
    pub local_backup_path: String,
    pub webdav: WebDAVConfig,
    pub s3: S3Config,
    /// Optional backup encryption: only the switch and the local credential-store
    /// reference live here. The password itself is stored in the OS credential store
    /// and never enters settings, backups, or logs.
    pub backup_encryption: BackupEncryptionConfig,
    pub last_backup_time: Option<String>,
    /// Include generated image files in backup zip (default: true)
    pub backup_image_assets_enabled: bool,
    /// Include optional (DB-backed) CLI runtime files under external-configs/ in backup zip
    /// (Codex / Claude / Grok / Gemini CLI; default: true).
    /// When false, those tools skip packaging/restore and re-apply from SQLite after restore.
    /// OpenCode / OpenClaw / Pi are always packaged and restored (subject to file filter rules).
    pub backup_cli_config_files_enabled: bool,
    /// User-defined files/directories to include in backup zip
    pub backup_custom_entries: Vec<BackupCustomEntry>,
    /// Launch on startup (default: true)
    pub launch_on_startup: bool,
    /// Minimize to tray on close instead of exiting (default: true)
    pub minimize_to_tray_on_close: bool,
    /// Start minimized to tray (default: false)
    pub start_minimized: bool,
    /// Enter lightweight mode at startup: destroy the main window before it is
    /// ever shown and keep only the tray/backend (default: false)
    pub start_lightweight: bool,
    /// Enter lightweight mode (destroy window, release WebView memory) instead
    /// of just hiding it when the user closes the window (default: false).
    /// Only effective while minimize_to_tray_on_close is true.
    pub lightweight_on_close: bool,
    /// Keep the computer awake (prevent idle sleep) while the app is running
    /// (default: false). Only inhibits idle sleep; manual sleep / lid close
    /// still works.
    pub keep_computer_awake: bool,
    /// Proxy mode for network requests: "direct", "custom", or "system" (default: "system")
    pub proxy_mode: String,
    /// Proxy URL for network requests (e.g., http://user:pass@proxy.com:8080 or socks5://proxy.com:1080)
    pub proxy_url: String,
    /// Theme mode: "light", "dark", or "system" (default: "system")
    pub theme: String,
    /// Enable auto backup (default: false)
    pub auto_backup_enabled: bool,
    /// Auto backup interval in days (default: 7)
    pub auto_backup_interval_days: u32,
    /// Max number of auto backups to keep, 0 = unlimited (default: 10)
    pub auto_backup_max_keep: u32,
    /// Last auto backup time in ISO 8601 format
    pub last_auto_backup_time: Option<String>,
    /// Auto check for updates on startup (default: true)
    pub auto_check_update: bool,
    /// Join the beta update channel: update checks also consider the rolling
    /// prerelease published on the fixed `beta` tag and always offer the
    /// newer of the stable and beta builds (default: false)
    pub receive_beta_updates: bool,
    /// Visible tabs in the tab bar (default: all tabs shown)
    pub visible_tabs: Vec<String>,
    /// Sidebar hidden state by page
    pub sidebar_hidden_by_page: HashMap<String, bool>,
    /// Allow clearing OMO/OMOS applied runtime config from OpenCode page (default: false)
    pub opencode_allow_clear_applied_oh_my_config: bool,
    /// Write OMO config to the legacy flat file (~/.config/opencode/oh-my-openagent.jsonc)
    /// instead of the unified ~/.omo/omo.jsonc ([opencode] block) format (default: false)
    pub opencode_use_legacy_oh_my_config: bool,
    /// Whether the user has already answered the one-time "has OMO been upgraded?"
    /// confirmation shown on the first Oh My OpenAgent apply (default: false)
    pub opencode_omo_upgrade_confirmed: bool,
    /// Dual-write `variant` alongside canonical `reasoning` when saving OMO agent/category
    /// configs. Works around upstream issue #6614 where the main agent loses effort when
    /// delegating to subagents under the `reasoning` key. Off (default) keeps current behavior
    /// (write `reasoning` only); on writes both fields with the same value. (default: false)
    pub opencode_dual_write_reasoning_variant: bool,
    /// Keep Codex official OAuth login when applying third-party providers (default: false)
    pub codex_preserve_official_auth_on_switch: bool,
    /// What to do after applying a Codex provider while a process still holds
    /// the previous config (issue #443): `prompt` shows the restart notification,
    /// `auto` restarts without asking, `off` does neither. Stored as a string so
    /// a future mode does not force a schema migration; unknown values fall back
    /// to `prompt` (see `web/utils/codexDaemonRestartMode.ts`).
    pub codex_daemon_restart_mode: String,
    /// Let official Codex sessions use the shared custom history bucket (default: false)
    pub codex_unified_session_history_enabled: bool,
    /// Append --dangerously-skip-permissions when launching Claude provider CLI (default: false)
    pub claude_cli_launch_full_access: bool,
    /// Preferred terminal app used when launching the Claude provider CLI on
    /// Windows. `None` or an unknown value falls back to `cmd` (system default).
    /// Known values: `cmd`, `powershell`, `wt` (Windows Terminal), `gitbash`.
    pub preferred_terminal: Option<String>,
    /// File filter rules for backup/restore
    pub backup_file_filter_rules: Vec<BackupFileFilterRule>,
    /// User-specified manual CLI paths by command name (e.g. `claude`, `opencode`,
    /// `grok`, `pi`, `omp`, `hermes`, `dsh`, `openclaw`). When present and the
    /// file exists, CLI execution prefers these paths over auto-discovery.
    pub cli_manual_paths: HashMap<String, String>,
    /// Session-detail filter chip visibility, persisted across app restarts.
    /// `None` means "no saved preference yet" (frontend defaults to all visible).
    pub session_detail_filters: Option<SessionDetailFilters>,
    /// Provider list sort mode per coding module (e.g. "kimi" -> "recent").
    /// Mode values: "custom" (drag order), "recent", "created", "name".
    pub provider_sort_modes: HashMap<String, String>,
    /// Last-used timestamp (RFC3339) per provider, keyed by "<module_key>:<provider_id>".
    /// Module keys follow the favorite-provider source convention
    /// (opencode/claudecode/claudedesktop/codex/grok/geminicli/kimi/openclaw/pi/omp/hermes/dsh).
    pub provider_last_used: HashMap<String, String>,
    /// Provider list layout per coding module (e.g. "claudecode" -> "card").
    /// Values: "list" (default) or "card" (compact grid view).
    pub provider_view_modes: HashMap<String, String>,
    /// Gateway statistics page time range, mirrored into the tray usage block.
    #[serde(default)]
    pub gateway_usage_range: GatewayUsageRangeSelection,
}

/// Combined read model for the provider list UI: per-module sort preference,
/// the last-used timestamp map consumed by the "recently used" sort mode, and
/// the layout preference (list vs compact card grid).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProviderListState {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub sort_modes: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub last_used: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub view_modes: HashMap<String, String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: String::new(),
            current_module: String::new(),
            current_sub_tab: String::new(),
            backup_type: String::new(),
            local_backup_path: String::new(),
            webdav: WebDAVConfig::default(),
            s3: S3Config::default(),
            backup_encryption: BackupEncryptionConfig::default(),
            last_backup_time: None,
            backup_image_assets_enabled: true,
            backup_cli_config_files_enabled: true,
            backup_custom_entries: Vec::new(),
            launch_on_startup: true,
            minimize_to_tray_on_close: true,
            start_minimized: false,
            start_lightweight: false,
            lightweight_on_close: false,
            keep_computer_awake: false,
            proxy_mode: "system".to_string(),
            proxy_url: String::new(),
            theme: "system".to_string(),
            auto_backup_enabled: false,
            auto_backup_interval_days: 7,
            auto_backup_max_keep: 10,
            last_auto_backup_time: None,
            auto_check_update: true,
            receive_beta_updates: false,
            visible_tabs: vec![
                "opencode".to_string(),
                "claudecode".to_string(),
                "claudedesktop".to_string(),
                "codex".to_string(),
                "grok".to_string(),
                "geminicli".to_string(),
                "antigravity".to_string(),
                "zcode".to_string(),
                "kimi".to_string(),
                "openclaw".to_string(),
                "pi".to_string(),
                "oh_my_pi".to_string(),
                "omo_native".to_string(),
                "hermes".to_string(),
                "dsh".to_string(),
                "workbuddy".to_string(),
                "codebuddy".to_string(),
                "gateway".to_string(),
                "image".to_string(),
                "ssh".to_string(),
                "wsl".to_string(),
            ],
            sidebar_hidden_by_page: default_sidebar_hidden_by_page(),
            opencode_allow_clear_applied_oh_my_config: false,
            opencode_use_legacy_oh_my_config: false,
            opencode_omo_upgrade_confirmed: false,
            opencode_dual_write_reasoning_variant: false,
            codex_preserve_official_auth_on_switch: false,
            codex_daemon_restart_mode: default_codex_daemon_restart_mode(),
            codex_unified_session_history_enabled: false,
            claude_cli_launch_full_access: false,
            preferred_terminal: None,
            backup_file_filter_rules: default_backup_file_filter_rules(),
            cli_manual_paths: HashMap::new(),
            session_detail_filters: None,
            provider_sort_modes: HashMap::new(),
            provider_last_used: HashMap::new(),
            provider_view_modes: HashMap::new(),
            gateway_usage_range: GatewayUsageRangeSelection::default(),
        }
    }
}

/// Default for `codex_daemon_restart_mode`: keep the existing behaviour, where
/// a running holder triggers the restart notification (issue #443).
pub const DEFAULT_CODEX_DAEMON_RESTART_MODE: &str = "prompt";

pub fn default_codex_daemon_restart_mode() -> String {
    DEFAULT_CODEX_DAEMON_RESTART_MODE.to_string()
}

pub fn default_sidebar_hidden_by_page() -> HashMap<String, bool> {
    // Keep this key set in sync with the frontend SIDEBAR_PAGE_KEYS list in
    // web/services/settingsApi.ts. A missing key here means a brand-new
    // database (no sidebar_hidden_by_page field yet) would not surface that
    // page's default visible state. The reader in adapter.rs no longer drops
    // keys outside this set, but the defaults still need to be complete.
    HashMap::from([
        ("opencode".to_string(), false),
        ("claudecode".to_string(), false),
        ("claudedesktop".to_string(), false),
        ("codex".to_string(), false),
        ("grok".to_string(), false),
        ("geminicli".to_string(), false),
        ("antigravity".to_string(), false),
        ("zcode".to_string(), false),
        ("kimi".to_string(), false),
        ("openclaw".to_string(), false),
        ("pi".to_string(), false),
        ("oh_my_pi".to_string(), false),
        ("omo_native".to_string(), false),
        ("hermes".to_string(), false),
        ("dsh".to_string(), false),
        ("workbuddy".to_string(), false),
        ("codebuddy".to_string(), false),
    ])
}

/// Default filter rules for backup file filtering.
///
/// File filtering is a user-managed extension. New users should not receive
/// implicit rules here.
pub fn default_backup_file_filter_rules() -> Vec<BackupFileFilterRule> {
    Vec::new()
}
