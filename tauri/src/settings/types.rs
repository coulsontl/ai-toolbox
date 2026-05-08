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
    pub last_backup_time: Option<String>,
    /// Include generated image files in backup zip (default: true)
    pub backup_image_assets_enabled: bool,
    /// Launch on startup (default: true)
    pub launch_on_startup: bool,
    /// Minimize to tray on close instead of exiting (default: true)
    pub minimize_to_tray_on_close: bool,
    /// Start minimized to tray (default: false)
    pub start_minimized: bool,
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
    /// Visible tabs in the tab bar (default: all tabs shown)
    pub visible_tabs: Vec<String>,
    /// Sidebar hidden state by page
    pub sidebar_hidden_by_page: HashMap<String, bool>,
    /// Allow clearing OMO/OMOS applied runtime config from OpenCode page (default: false)
    pub opencode_allow_clear_applied_oh_my_config: bool,
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
            last_backup_time: None,
            backup_image_assets_enabled: true,
            launch_on_startup: true,
            minimize_to_tray_on_close: true,
            start_minimized: false,
            proxy_mode: "system".to_string(),
            proxy_url: String::new(),
            theme: "system".to_string(),
            auto_backup_enabled: false,
            auto_backup_interval_days: 7,
            auto_backup_max_keep: 10,
            last_auto_backup_time: None,
            auto_check_update: true,
            visible_tabs: vec![
                "opencode".to_string(),
                "claudecode".to_string(),
                "codex".to_string(),
                "openclaw".to_string(),
                "image".to_string(),
                "ssh".to_string(),
                "wsl".to_string(),
            ],
            sidebar_hidden_by_page: default_sidebar_hidden_by_page(),
            opencode_allow_clear_applied_oh_my_config: false,
        }
    }
}

pub fn default_sidebar_hidden_by_page() -> HashMap<String, bool> {
    HashMap::from([
        ("opencode".to_string(), false),
        ("claudecode".to_string(), false),
        ("codex".to_string(), false),
        ("openclaw".to_string(), false),
    ])
}
