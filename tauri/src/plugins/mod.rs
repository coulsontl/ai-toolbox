//! Gateway plugin system.
//!
//! [`PluginManager`] owns the shared database handle, paths and the runtimes:
//! the Bun host for provider plugins, the in-process rquickjs engine for
//! gateway middleware, and the lazily started loopback server for plugin
//! pages. Commands reach them through the manager so there is exactly one
//! host process, one compiled middleware set and one page server.

pub mod background_tasks;
pub mod bun_host;
pub mod bun_runtime;
pub mod cli_integration;
pub mod commands;
pub mod gateway_bridge;
pub mod installer;
pub mod market;
pub mod middleware_rt;
pub mod page_bridge;
pub mod page_server;
pub mod page_storage;
pub mod paths;
pub mod store;
pub mod tools_mcp;
pub mod types;
pub mod updater;

use std::path::PathBuf;
use std::sync::Arc;

use crate::db::SqliteDbState;
use crate::http_client;

use bun_host::BunHost;
use bun_runtime::BunRuntime;
use gateway_bridge::PluginGatewayState;
use middleware_rt::MiddlewareRuntime;
use page_server::PluginPageServer;
use paths::PluginPaths;
use types::PluginSettings;

/// Shared plugin state managed by Tauri as `Arc<PluginManager>`.
pub struct PluginManager {
    db: SqliteDbState,
    paths: PluginPaths,
    bun: Arc<BunRuntime>,
    host: Arc<BunHost>,
    middleware: Arc<MiddlewareRuntime>,
    gateway: PluginGatewayState,
    page_server: Arc<PluginPageServer>,
    /// Serializes package mutations: installs, uninstalls, record edits,
    /// reloads and update checks. Background updates use `try_lock`.
    pub(crate) update_lock: tokio::sync::Mutex<()>,
}

pub type PluginManagerState = Arc<PluginManager>;

impl PluginManager {
    pub fn new(db: SqliteDbState, app_data_dir: PathBuf) -> PluginManagerState {
        let paths = PluginPaths::new(app_data_dir);
        let bun = Arc::new(BunRuntime::new(db.clone(), paths.clone()));
        let host = Arc::new(BunHost::new(db.clone(), paths.clone(), bun.clone()));
        let middleware = Arc::new(MiddlewareRuntime::new(db.clone(), paths.clone()));
        Arc::new(Self {
            db,
            paths,
            bun,
            host,
            middleware,
            gateway: PluginGatewayState::new(),
            page_server: Arc::new(PluginPageServer::new()),
            update_lock: tokio::sync::Mutex::new(()),
        })
    }

    pub fn db(&self) -> &SqliteDbState {
        &self.db
    }

    pub fn paths(&self) -> &PluginPaths {
        &self.paths
    }

    pub fn bun(&self) -> &Arc<BunRuntime> {
        &self.bun
    }

    pub fn host(&self) -> &Arc<BunHost> {
        &self.host
    }

    pub fn middleware(&self) -> &Arc<MiddlewareRuntime> {
        &self.middleware
    }

    /// Account rotation and provider listing cache shared by gateway requests.
    pub fn gateway(&self) -> &PluginGatewayState {
        &self.gateway
    }

    /// The lazily started loopback server for plugin pages.
    pub fn page_server(&self) -> &Arc<PluginPageServer> {
        &self.page_server
    }

    pub fn settings(&self) -> Result<PluginSettings, String> {
        store::get_plugin_settings(&self.db)
    }

    /// HTTP client for market and npm requests. Honors the per-plugin
    /// `followAppProxy` switch: when off, requests bypass the app proxy.
    pub async fn http_client(&self) -> Result<reqwest::Client, String> {
        http_client_for(&self.db, 30).await
    }

    /// The proxy URL the Bun host should use for a plugin's own vendor
    /// requests. Empty when the per-plugin switch is off, the app proxy is
    /// not a fixed URL (system mode), or no proxy is configured.
    pub async fn proxy_url(&self) -> Result<String, String> {
        if !store::get_plugin_settings(&self.db)?.follow_app_proxy {
            return Ok(String::new());
        }
        let (proxy_mode, proxy_url) = http_client::get_proxy_from_settings(&self.db).await?;
        if proxy_mode != http_client::ProxyMode::Custom {
            return Ok(String::new());
        }
        Ok(normalize_proxy_url(&proxy_url))
    }
}

/// Prefix a scheme the way the app's HTTP clients do, and keep only the
/// proxy kinds Bun's `fetch` accepts (`http` / `https`). Anything else is
/// treated as "no proxy", matching the gateway's plugin request path.
fn normalize_proxy_url(proxy_url: &str) -> String {
    let trimmed = proxy_url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let normalized = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    if normalized.starts_with("http://") || normalized.starts_with("https://") {
        normalized
    } else {
        String::new()
    }
}

/// Plugin HTTP client with an explicit timeout; the Bun download needs a much
/// longer one than a market refresh.
pub(crate) async fn http_client_for(
    db: &SqliteDbState,
    timeout_secs: u64,
) -> Result<reqwest::Client, String> {
    if store::get_plugin_settings(db)?.follow_app_proxy {
        http_client::client_with_timeout(db, timeout_secs).await
    } else {
        http_client::create_client_no_proxy(timeout_secs)
    }
}

/// Dedicated Tauri IPC dispatcher for all `plugins_*` commands.
///
/// Kept behind `#[inline(never)]` out of `lib.rs`'s main `generate_handler!`:
/// in unoptimized Windows debug builds, LLVM allocates a separate stack slot
/// per match arm, so folding 51 more commands into the 700-command root
/// handler overflows Windows' default 1 MiB main-thread stack (`0xc000041d`).
#[inline(never)]
pub fn invoke_handler(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
    let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
        // Install, markets and settings
        commands::plugins_list_installed,
        commands::plugins_install,
        commands::plugins_uninstall,
        commands::plugins_reload,
        commands::plugins_check_updates,
        commands::plugins_update_plugins,
        commands::plugins_set_enabled,
        commands::plugins_set_options,
        commands::plugins_list_markets,
        commands::plugins_add_market,
        commands::plugins_update_market,
        commands::plugins_remove_market,
        commands::plugins_refresh_markets,
        commands::plugins_market_listings,
        commands::plugins_get_settings,
        commands::plugins_save_settings,
        // Bun runtime, host, middleware and provider accounts
        commands::plugins_bun_status,
        commands::plugins_bun_ensure,
        commands::plugins_bun_probe_path,
        commands::plugins_host_status,
        commands::plugins_host_restart,
        commands::plugins_runtime_logs,
        commands::plugins_clear_runtime_logs,
        commands::plugins_middleware_states,
        commands::plugins_gateway_hook_states,
        commands::plugins_list_providers,
        commands::plugins_refresh_provider_models,
        commands::plugins_login_prompt,
        commands::plugins_login_validate,
        commands::plugins_login_authorize,
        commands::plugins_login_callback,
        commands::plugins_login_api_key,
        commands::plugins_login_import_files,
        commands::plugins_login_cancel,
        commands::plugins_list_accounts,
        commands::plugins_account_sign_out,
        commands::plugins_account_check,
        commands::plugins_account_usage,
        commands::plugins_account_checkin,
        commands::plugins_account_set_auto_checkin,
        commands::plugins_set_provider_auto_checkin,
        commands::plugins_account_set_enabled,
        commands::plugins_set_provider_option,
        // Plugin tools -> central MCP server
        commands::plugins_list_tools,
        commands::plugins_sync_tools_mcp_server,
        // Custom pages, menus and panels (`magpie.page` / `magpie.pages` /
        // `magpie.actions` / `magpie.panels`)
        commands::plugins_list_pages,
        commands::plugins_open_page,
        commands::plugins_close_page,
        commands::plugins_list_actions,
        commands::plugins_list_panels,
        commands::plugins_open_panel,
        commands::plugins_close_panel,
        commands::plugins_page_bridge_read,
        commands::plugins_page_bridge_set_options,
        commands::plugins_page_bridge_copy_image,
        commands::plugins_page_bridge_save_image,
        commands::plugins_page_bridge_kv,
        commands::plugins_page_bridge_files,
        // Plugin providers -> coding CLI tabs
        cli_integration::plugins_list_cli_targets,
        cli_integration::plugins_add_provider_to_cli,
        cli_integration::plugins_sync_cli_provider_models,
    ];
    handler(invoke)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_resolves_paths_and_default_settings() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let manager = PluginManager::new(db, PathBuf::from("/data/app"));
        assert_eq!(manager.paths().root(), PathBuf::from("/data/app/plugins"));
        assert!(manager.settings().unwrap().follow_app_proxy);
        assert!(!manager.middleware().has_active_middlewares());
    }

    #[test]
    fn the_host_proxy_accepts_http_and_https_urls_only() {
        assert_eq!(normalize_proxy_url(""), "");
        assert_eq!(
            normalize_proxy_url("proxy.example.com:8080"),
            "http://proxy.example.com:8080"
        );
        assert_eq!(
            normalize_proxy_url("https://proxy.example.com:8443"),
            "https://proxy.example.com:8443"
        );
        // Bun's fetch has no SOCKS proxy option; the host stays direct.
        assert_eq!(normalize_proxy_url("socks5://proxy.example.com:1080"), "");
    }

    #[tokio::test]
    async fn proxy_url_is_empty_without_a_custom_app_proxy() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let manager = PluginManager::new(db, PathBuf::from("/data/app"));
        assert_eq!(manager.proxy_url().await.unwrap(), "");
    }
}
