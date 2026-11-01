//! 插件宿主相关的 Tauri 命令：Bun 运行时、宿主状态、插件安装与刷新。
//!
//! ── 为什么安装插件要 spawn 包管理器 ────────────────────────────
//! OpenCode 插件是 npm 包，它的依赖树也只有包管理器解得对（peer 依赖、workspace
//! 协议、可选原生依赖）。自己实现一套解包逻辑等于重写 npm，而插件生态一旦出现
//! 新形态就得跟着改。代价是安装需要网络与 `bun add`，失败时把包的 stderr 原样
//! 交给用户——那比一句「安装失败」有用得多。
//!
//! ── 安装目录 ──────────────────────────────────────────────────
//! `{data_dir}/relay/plugins/<id>/`，每个插件一个独立目录，各自一份
//! `package.json` 与 `node_modules`。共用一棵依赖树会让两个插件对同一个依赖的
//! 版本要求互相顶掉。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use super::runtime::{bun, host};
use super::store;
use super::types::{RelayPlugin, RelayPluginAccount, RelayPluginProvider};
use crate::SqliteDbState;

/// Bun 下载进度的 Tauri 事件名。
pub const BUN_PROGRESS_EVENT: &str = "relay://bun-progress";

fn plugins_root() -> PathBuf {
    bun::runtime_root()
        .parent()
        .map(|runtime| runtime.join("plugins"))
        .unwrap_or_else(|| bun::runtime_root().join("plugins"))
}

// ---- Bun 运行时 --------------------------------------------------------------

#[tauri::command]
pub async fn relay_bun_status() -> Result<bun::BunRuntimeStatus, String> {
    Ok(bun::status())
}

#[tauri::command]
pub async fn relay_bun_install(
    app: AppHandle,
    state: State<'_, SqliteDbState>,
) -> Result<bun::BunRuntimeStatus, String> {
    let emitter = app.clone();
    bun::install(&state, move |progress| {
        // A failed emit only means the window is gone; the download continues.
        let _ = emitter.emit(BUN_PROGRESS_EVENT, progress);
    })
    .await
}

#[tauri::command]
pub async fn relay_bun_uninstall() -> Result<(), String> {
    host::shutdown().await;
    bun::uninstall()
}

// ---- 宿主 --------------------------------------------------------------------

#[tauri::command]
pub async fn relay_host_status() -> Result<host::HostStatus, String> {
    Ok(host::status().await)
}

/// Restart the host with the plugins currently installed.
///
/// This is also the "retry" for a host that was stopped after repeated
/// crashes: it clears the counter and starts a fresh process.
#[tauri::command]
pub async fn relay_host_restart(
    state: State<'_, SqliteDbState>,
) -> Result<host::HostStatus, String> {
    host::shutdown().await;
    host::reset_failures().await;
    refresh_plugins(&state).await?;
    Ok(host::status().await)
}

#[tauri::command]
pub async fn relay_host_stop() -> Result<(), String> {
    host::shutdown().await;
    Ok(())
}

// ---- 插件 --------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct InstallPluginInput {
    /// An npm spec (`@magpie-community/opencode-zcode-auth@0.1.19`) or a local path.
    pub spec: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPluginResult {
    pub plugin: RelayPlugin,
    pub providers: Vec<host::HostProvider>,
    /// The host's own complaint about this package, if it made one.
    pub host_error: String,
}

#[tauri::command]
pub async fn relay_install_plugin(
    state: State<'_, SqliteDbState>,
    input: InstallPluginInput,
) -> Result<InstallPluginResult, String> {
    let spec = input.spec.trim().to_string();
    if spec.is_empty() {
        return Err("Enter an npm package name or a local path".to_string());
    }

    let root = plugins_root();
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("Failed to create the 2API plugin directory: {error}"))?;
    // One directory per spec: two plugins may depend on conflicting versions of
    // the same package, and a shared node_modules resolves only one of them.
    let dir = root.join(sanitize_dir_name(&spec));
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("Failed to create the plugin directory: {error}"))?;
    write_package_json(&dir, &spec)?;

    let bun_path = bun::installed_bun_path(bun::BUN_VERSION);
    if !bun_path.is_file() {
        return Err(
            "Install the Bun runtime first: the plugin host runs on it.".to_string(),
        );
    }
    run_bun(&bun_path, &dir, &["add", &spec, "--no-save"]).await?;
    let version = read_installed_version(&dir, &spec);

    let plugin = store::upsert_plugin(
        &state,
        RelayPlugin {
            id: String::new(),
            spec: spec.clone(),
            target: dir.display().to_string(),
            version,
            enabled: true,
            options_json: "{}".to_string(),
            installed_at: 0,
        },
    )
    .await?;

    // A new package deserves a fresh start: clear the crash counter and let the
    // next operation rebuild the host with it.
    host::reset_failures().await;
    host::shutdown().await;

    match refresh_plugins(&state).await {
        Ok(init) => Ok(InstallPluginResult {
            // Only this package's providers: the host reports every loaded
            // plugin on each init, and the page shows what this install added.
            providers: init
                .providers
                .into_iter()
                .filter(|provider| provider.spec == spec)
                .collect(),
            plugin,
            host_error: String::new(),
        }),
        Err(host_error) => {
            // The package installed but the host refused it. Keep the record so
            // the user can see and remove it, and report why.
            Ok(InstallPluginResult {
                plugin,
                providers: Vec::new(),
                host_error,
            })
        }
    }
}

#[tauri::command]
pub async fn relay_uninstall_plugin(
    state: State<'_, SqliteDbState>,
    id: String,
) -> Result<(), String> {
    let plugins = store::list_plugins(&state).await?;
    let Some(plugin) = plugins.into_iter().find(|plugin| plugin.id == id) else {
        return Ok(());
    };
    store::delete_plugin(&state, &id).await?;
    // The package directory goes too: leaving it would make the next install of
    // the same spec pick up a half-removed tree.
    let target = PathBuf::from(&plugin.target);
    if target.starts_with(plugins_root()) && target.exists() {
        let _ = std::fs::remove_dir_all(&target);
    }
    host::shutdown().await;
    Ok(())
}

#[tauri::command]
pub async fn relay_refresh_plugins(
    state: State<'_, SqliteDbState>,
) -> Result<host::HostInitResult, String> {
    refresh_plugins(&state).await
}

// ---- 账号 --------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SignInInput {
    pub provider: String,
    /// The account key to sign in to; empty creates one.
    #[serde(default)]
    pub account: String,
    /// `api` for a pasted key, `oauth` for the browser flow.
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub inputs: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInResult {
    pub provider: String,
    pub account: String,
    /// Empty when the sign-in finished; otherwise where to send the user and
    /// the session to hand the code back with.
    pub session: String,
    pub url: String,
    pub instructions: String,
}

#[tauri::command]
pub async fn relay_sign_in(
    state: State<'_, SqliteDbState>,
    input: SignInInput,
) -> Result<SignInResult, String> {
    let (link, init) = start_host(&state).await?;
    let method = init
        .providers
        .iter()
        .find(|provider| provider.id == input.provider)
        .map(|provider| provider.methods.clone())
        .unwrap_or_default();
    let is_oauth = method
        .iter()
        .any(|m| m.get("type").and_then(serde_json::Value::as_str) == Some("oauth"));

    let reply = if is_oauth || input.method == "oauth" {
        link.request(
            "authorize",
            serde_json::json!({
                "provider": input.provider,
                "method": oauth_method_index(&method),
                "inputs": input.inputs,
                "account": input.account,
            }),
        )
        .await?
    } else {
        link.request(
            "apiKey",
            serde_json::json!({
                "provider": input.provider,
                "method": 0,
                "inputs": input.inputs,
                "key": input.key,
                "account": input.account,
            }),
        )
        .await?
    };

    // The host answers either with a saved account or with a pending session.
    if let Some(session) = reply.get("session").and_then(serde_json::Value::as_str) {
        return Ok(SignInResult {
            provider: input.provider,
            account: String::new(),
            session: session.to_string(),
            url: reply
                .get("url")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            instructions: reply
                .get("instructions")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }

    let account = reply
        .get("account")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    // `client.auth.set` in the plugin already wrote the credentials; the host
    // mirrored them in its own store, and this pulls that copy into SQLite.
    sync_accounts_from_host(&state, &link, &input.provider).await?;
    Ok(SignInResult {
        provider: input.provider,
        account,
        session: String::new(),
        url: String::new(),
        instructions: String::new(),
    })
}

#[derive(Debug, Deserialize)]
pub struct OAuthCallbackInput {
    pub session: String,
    pub code: String,
}

#[tauri::command]
pub async fn relay_oauth_callback(
    state: State<'_, SqliteDbState>,
    input: OAuthCallbackInput,
) -> Result<SignInResult, String> {
    let link = ensure_link(&state).await?;
    let reply = link
        .request(
            "callback",
            serde_json::json!({ "session": input.session, "code": input.code }),
        )
        .await?;
    let provider = reply
        .get("provider")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    sync_accounts_from_host(&state, &link, &provider).await?;
    Ok(SignInResult {
        account: reply
            .get("account")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        provider,
        session: String::new(),
        url: String::new(),
        instructions: String::new(),
    })
}

#[tauri::command]
pub async fn relay_sign_out(
    state: State<'_, SqliteDbState>,
    provider: String,
    account: String,
) -> Result<(), String> {
    let link = ensure_link(&state).await?;
    link.request(
        "setAuth",
        serde_json::json!({ "key": account, "auth": serde_json::Value::Null }),
    )
    .await?;
    let accounts = store::list_plugin_accounts(&state, None).await?;
    for existing in accounts {
        let same_account = existing.account_key == account
            || (!account.is_empty() && existing.account_key.starts_with(&format!("{account}#")));
        if same_account {
            store::delete_plugin_account(&state, &existing.id).await?;
        }
    }
    let _ = provider;
    Ok(())
}

// ---- 内部 --------------------------------------------------------------------

/// Rebuild the host from what is installed, and persist what it reports.
pub async fn refresh_plugins(state: &SqliteDbState) -> Result<host::HostInitResult, String> {
    let plugins = store::list_plugins(state).await?;
    let specs: Vec<host::HostPluginSpec> = plugins
        .iter()
        .filter(|plugin| plugin.enabled)
        .map(|plugin| host::HostPluginSpec {
            spec: plugin.spec.clone(),
            target: plugin.target.clone(),
            options: serde_json::from_str(&plugin.options_json).ok(),
        })
        .collect();
    let (auth, options) = host_payload(state).await?;
    let directory = plugins_root();
    std::fs::create_dir_all(&directory).ok();
    let init = host::restart(specs, auth, options, &directory).await?;
    persist_host_view(state, &init).await?;
    Ok(init)
}

/// Start the host if it is not running, returning its init reply.
pub async fn start_host(state: &SqliteDbState) -> Result<(std::sync::Arc<super::runtime::HostLink>, host::HostInitResult), String> {
    let plugins = store::list_plugins(state).await?;
    let specs: Vec<host::HostPluginSpec> = plugins
        .iter()
        .filter(|plugin| plugin.enabled)
        .map(|plugin| host::HostPluginSpec {
            spec: plugin.spec.clone(),
            target: plugin.target.clone(),
            options: serde_json::from_str(&plugin.options_json).ok(),
        })
        .collect();
    let (auth, options) = host_payload(state).await?;
    let directory = plugins_root();
    std::fs::create_dir_all(&directory).ok();
    let (link, init) = host::ensure(specs, auth, options, &directory).await?;
    persist_host_view(state, &init).await?;
    Ok((link, init))
}

async fn ensure_link(state: &SqliteDbState) -> Result<std::sync::Arc<super::runtime::HostLink>, String> {
    let (link, _init) = start_host(state).await?;
    Ok(link)
}

/// Everything the host is told at init: every account's verbatim auth blob and
/// the per-provider option values the user set.
async fn host_payload(
    state: &SqliteDbState,
) -> Result<(serde_json::Value, serde_json::Value), String> {
    let accounts = store::list_plugin_accounts(state, None).await?;
    let mut auth = serde_json::Map::new();
    for account in accounts {
        if account.account_key.is_empty() {
            continue;
        }
        // Stored verbatim: a plugin that wrote an OAuth blob gets back exactly
        // what it wrote, because any reshaping makes it reject its own token.
        let value: serde_json::Value = serde_json::from_str(&account.auth_json)
            .unwrap_or_else(|_| serde_json::json!({}));
        auth.insert(account.account_key, value);
    }
    let plugins = store::list_plugins(state).await?;
    let mut options = serde_json::Map::new();
    for plugin in plugins {
        // `options_json` is the user's per-provider switches, not the plugin's
        // second argument; the host reads it as `chosen`.
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&plugin.options_json) {
            if let Some(map) = parsed.as_object() {
                for (provider, value) in map {
                    options.insert(provider.clone(), value.clone());
                }
            }
        }
    }
    Ok((serde_json::Value::Object(auth), serde_json::Value::Object(options)))
}

/// Persist the providers the host reported.
///
/// Each provider carries the package and export that declared it, so it is
/// attributed to the plugin row even when one package exports several plugin
/// functions (workbuddy exports two, one per site).
async fn persist_host_view(
    state: &SqliteDbState,
    init: &host::HostInitResult,
) -> Result<(), String> {
    let plugins = store::list_plugins(state).await?;
    for provider in &init.providers {
        let Some(plugin_id) = plugins
            .iter()
            .find(|plugin| plugin.spec == provider.spec)
            .map(|plugin| plugin.id.clone())
        else {
            // A provider whose package is no longer installed: its row would be
            // unreachable, so it is left alone rather than re-created.
            continue;
        };
        let existing = store::list_plugin_providers(state, Some(&plugin_id)).await?;
        let row = existing
            .into_iter()
            .find(|row| row.provider_id == provider.id)
            .unwrap_or_default();
        store::upsert_plugin_provider(
            state,
            RelayPluginProvider {
                id: row.id,
                plugin_id,
                provider_id: provider.id.clone(),
                label: if provider.name.is_empty() {
                    provider.id.clone()
                } else {
                    provider.name.clone()
                },
                entry_export: provider.export_name.clone(),
                npm: provider.npm.clone(),
                capabilities_json: serde_json::json!({
                    "models": provider.models,
                    "methods": provider.methods,
                    "options": provider.options,
                    "usage": provider.usage,
                    "checkin": provider.checkin,
                    "single": provider.single,
                    "listError": provider.list_error,
                    "file": provider.file,
                })
                .to_string(),
                enabled: row.enabled,
            },
        )
        .await?;
    }
    Ok(())
}

fn oauth_method_index(methods: &[serde_json::Value]) -> usize {
    methods
        .iter()
        .position(|m| m.get("type").and_then(serde_json::Value::as_str) == Some("oauth"))
        .unwrap_or(0)
}

fn sanitize_dir_name(spec: &str) -> String {
    let base = spec.rsplit('/').next().unwrap_or(spec);
    base.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn write_package_json(dir: &Path, _spec: &str) -> Result<(), String> {
    let path = dir.join("package.json");
    if path.is_file() {
        return Ok(());
    }
    std::fs::write(&path, "{\"name\":\"relay-plugins\",\"private\":true}\n")
        .map_err(|error| format!("Failed to write the plugin package.json: {error}"))
}

async fn run_bun(bun_path: &Path, dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = crate::coding::cli_resolver::build_local_tokio_command(bun_path);
    command.args(args).current_dir(dir).kill_on_drop(true);
    let output = command
        .output()
        .await
        .map_err(|error| format!("Failed to run bun {}: {error}", args.join(" ")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        let detail = if stderr.trim().is_empty() { stdout } else { stderr };
        return Err(format!("bun {} failed:\n{}", args.join(" "), detail.trim()));
    }
    Ok(stdout)
}

/// The version the package manager actually resolved, not the spec's range.
fn read_installed_version(dir: &Path, spec: &str) -> String {
    let package_name = spec_name(spec);
    let manifest = dir
        .join("node_modules")
        .join(package_name.replace('/', std::path::MAIN_SEPARATOR_STR))
        .join("package.json");
    std::fs::read_to_string(manifest)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.get("version").and_then(serde_json::Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

/// `@scope/pkg@1.2.3` → `@scope/pkg`.
fn spec_name(spec: &str) -> String {
    if let Some(rest) = spec.strip_prefix('@') {
        // The version separator is the `@` after the slash.
        return match rest.split_once('/') {
            Some((scope, tail)) => {
                let name = tail.split('@').next().unwrap_or(tail);
                format!("@{scope}/{name}")
            }
            None => spec.to_string(),
        };
    }
    spec.split('@').next().unwrap_or(spec).to_string()
}

/// Pull the host's current auth store back into SQLite.
///
/// The plugin writes credentials through `client.auth.set`, which the host keeps
/// in memory and reports as an `auth` event. Reading it back here (instead of
/// trusting the event alone) means a sign-in that finished while the UI was
/// disconnected is still persisted.
async fn sync_accounts_from_host(
    state: &SqliteDbState,
    link: &std::sync::Arc<super::runtime::HostLink>,
    provider: &str,
) -> Result<(), String> {
    let reply = link
        .request("listAuth", serde_json::json!({ "provider": provider }))
        .await?;
    let Some(map) = reply.as_object() else {
        return Ok(());
    };
    let providers = store::list_plugin_providers(state, None).await?;
    for (key, value) in map {
        let Some(provider_row) = providers.iter().find(|row| row.provider_id == provider) else {
            continue;
        };
        let accounts = store::list_plugin_accounts(state, Some(&provider_row.id)).await?;
        let existing = accounts.iter().find(|account| &account.account_key == key);
        store::upsert_plugin_account(
            state,
            RelayPluginAccount {
                id: existing.map(|account| account.id.clone()).unwrap_or_default(),
                plugin_provider_id: provider_row.id.clone(),
                account_key: key.clone(),
                display_name: existing
                    .map(|account| account.display_name.clone())
                    .unwrap_or_else(|| key.clone()),
                auth_json: value.to_string(),
                last_used_at: super::types::now_ms(),
            },
        )
        .await?;
    }
    Ok(())
}

/// The providers the live host process currently serves.
///
/// Asked of the process rather than read from SQLite, so the page shows what is
/// actually serving requests — a plugin that failed to load is absent here even
/// though its row still exists.
#[tauri::command]
pub async fn relay_host_providers(
    state: State<'_, SqliteDbState>,
) -> Result<Vec<host::HostProvider>, String> {
    let link = ensure_link(&state).await?;
    let reply = link.request("providers", serde_json::json!({})).await?;
    serde_json::from_value(reply)
        .map_err(|error| format!("Failed to read the plugin host's provider list: {error}"))
}
