use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use chrono::Local;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{Emitter, Manager};
use tempfile::NamedTempFile;
use tokio::sync::{watch, Mutex as AsyncMutex};

use super::adapter;
use super::commands::get_grok_auth_path_async;
use super::types::GrokOfficialAccount;
use crate::coding::db_id::{db_extract_id, db_new_id};
use crate::db::helpers::{
    db_delete, db_get, db_list, db_patch_fields, db_put, db_update_applied_status,
};
use crate::db::schema::{Cli, DbTable, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use crate::http_client;

const XAI_DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";
const XAI_ISSUER: &str = "https://auth.x.ai";
const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const XAI_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access conversations:read conversations:write";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// Official Grok CLI chat-proxy base (OAuth consumer billing/subscription live here, not api.x.ai).
const GROK_CLI_PROXY_BASE: &str = "https://cli-chat-proxy.grok.com/v1";
const GROK_CLI_CLIENT_VERSION: &str = "0.2.111";
const GROK_CLI_CLIENT_IDENTIFIER: &str = "grok-shell";
const GROK_CLI_TOKEN_AUTH: &str = "xai-grok-cli";
const GROK_CLI_USER_AGENT: &str = "grok-shell/0.2.111 (windows; x86_64)";
/// Access tokens last ~hours; refresh when remaining lifetime is within this lead.
const GROK_AUTH_REFRESH_LEAD_SECONDS: i64 = 30 * 60;
const GROK_AUTH_REFRESH_CACHE_TTL: Duration = Duration::from_secs(30);

static AUTH_SESSIONS: LazyLock<Mutex<HashMap<String, watch::Sender<bool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static AUTH_SESSION_STATUSES: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static OAUTH_REFRESH_LOCK: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));
static OAUTH_REFRESH_CACHE: LazyLock<AsyncMutex<HashMap<String, (Instant, TokenResponse)>>> =
    LazyLock::new(|| AsyncMutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokDeviceAuthStartResult {
    pub session_id: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub user_code: String,
    pub expires_at: i64,
    pub poll_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GrokAuthStatusEvent {
    session_id: String,
    status: String,
    message: Option<String>,
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscoveryResponse {
    issuer: String,
    device_authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: String,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: i64,
    interval: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserInfoResponse {
    sub: Option<String>,
    email: Option<String>,
    given_name: Option<String>,
    picture: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct GrokUsageSnapshot {
    plan_type: Option<String>,
    limit_weekly_text: Option<String>,
    limit_monthly_text: Option<String>,
    limit_weekly_reset_at: Option<i64>,
    limit_monthly_reset_at: Option<i64>,
}

#[tauri::command]
pub async fn start_grok_official_account_device_auth(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<GrokDeviceAuthStartResult, String> {
    ensure_official_provider(state.db(), &provider_id)?;
    if !AUTH_SESSIONS
        .lock()
        .map_err(|_| "Grok auth session lock is poisoned".to_string())?
        .is_empty()
    {
        return Err("A Grok device authorization session is already active".to_string());
    }
    let client = http_client::client_with_timeout(state.db(), 30).await?;
    let discovery: DiscoveryResponse = client
        .get(XAI_DISCOVERY_URL)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|error| format!("xAI discovery request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("xAI discovery failed: {error}"))?
        .json()
        .await
        .map_err(|error| format!("Failed to parse xAI discovery: {error}"))?;
    let device_endpoint = validate_xai_endpoint(
        &discovery.device_authorization_endpoint,
        "device_authorization_endpoint",
    )?;
    let issuer = validate_xai_endpoint(&discovery.issuer, "issuer")?;
    let token_endpoint = validate_xai_endpoint(&discovery.token_endpoint, "token_endpoint")?;
    let userinfo_endpoint =
        validate_xai_endpoint(&discovery.userinfo_endpoint, "userinfo_endpoint")?;
    let device: DeviceCodeResponse = client
        .post(device_endpoint)
        .header("Accept", "application/json")
        .form(&[("client_id", XAI_CLIENT_ID), ("scope", XAI_SCOPE)])
        .send()
        .await
        .map_err(|error| format!("xAI device code request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("xAI device code request failed: {error}"))?
        .json()
        .await
        .map_err(|error| format!("Failed to parse xAI device code response: {error}"))?;
    if device.device_code.trim().is_empty()
        || device.user_code.trim().is_empty()
        || device.verification_uri.trim().is_empty()
    {
        return Err("xAI device code response is incomplete".to_string());
    }

    let session_id = db_new_id();
    let interval = device.interval.unwrap_or(5).max(5);
    let expires_at = unix_now().saturating_add(device.expires_in.max(0));
    let (cancel_sender, cancel_receiver) = watch::channel(false);
    AUTH_SESSIONS
        .lock()
        .map_err(|_| "Grok auth session lock is poisoned".to_string())?
        .insert(session_id.clone(), cancel_sender);

    emit_status(&app, &session_id, "waiting_for_user", None, None);
    let poll_session_id = session_id.clone();
    let poll_provider_id = provider_id.clone();
    tauri::async_runtime::spawn(async move {
        poll_device_authorization(
            app,
            poll_session_id,
            poll_provider_id,
            issuer,
            token_endpoint,
            userinfo_endpoint,
            device.device_code,
            expires_at,
            interval,
            cancel_receiver,
        )
        .await;
    });

    Ok(GrokDeviceAuthStartResult {
        session_id,
        verification_uri: device.verification_uri,
        verification_uri_complete: device.verification_uri_complete,
        user_code: device.user_code,
        expires_at,
        poll_interval_seconds: interval,
    })
}

#[tauri::command]
pub fn cancel_grok_official_account_device_auth(session_id: String) -> Result<(), String> {
    let sender = AUTH_SESSIONS
        .lock()
        .map_err(|_| "Grok auth session lock is poisoned".to_string())?
        .remove(&session_id);
    if let Some(sender) = sender {
        let _ = sender.send(true);
    }
    Ok(())
}

#[tauri::command]
pub fn get_grok_official_account_auth_status(session_id: String) -> Result<String, String> {
    AUTH_SESSION_STATUSES
        .lock()
        .map_err(|_| "Grok auth status lock is poisoned".to_string())?
        .get(&session_id)
        .cloned()
        .ok_or_else(|| format!("Grok auth session '{session_id}' not found"))
}

#[tauri::command]
pub fn list_grok_official_accounts(
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
) -> Result<Vec<GrokOfficialAccount>, String> {
    let order = OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::created_at(OrderDirection::Asc),
    ]);
    state
        .db()
        .with_conn(|conn| db_list(conn, DbTable::CliOfficialAccount(Cli::Grok), Some(&order)))
        .map(|values| {
            values
                .into_iter()
                .filter(|value| {
                    value.get("provider_id").and_then(Value::as_str) == Some(provider_id.as_str())
                })
                .map(account_from_db_value)
                .collect()
        })
}

#[tauri::command]
pub async fn save_grok_official_local_account(
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
    name: Option<String>,
) -> Result<GrokOfficialAccount, String> {
    ensure_official_provider(state.db(), &provider_id)?;
    let auth_path = get_grok_auth_path_async(state.db()).await?;
    let snapshot = fs::read_to_string(&auth_path)
        .map_err(|error| format!("Failed to read {}: {error}", auth_path.display()))?;
    let value: Value = serde_json::from_str(&snapshot)
        .map_err(|error| format!("Invalid Grok auth.json: {error}"))?;
    let (scope_key, entry) = find_xai_auth_entry(&value)?;
    let account_snapshot = single_account_snapshot(scope_key, entry.clone());
    let (email, subject) = identity_from_snapshot(&account_snapshot);
    save_account(
        state.db(),
        &provider_id,
        name.or_else(|| email.clone())
            .unwrap_or_else(|| "xAI".to_string()),
        email,
        subject,
        serde_json::to_string_pretty(&account_snapshot)
            .map_err(|error| format!("Failed to serialize Grok account snapshot: {error}"))?,
        Some(format!("{XAI_ISSUER}/oauth2/token")),
        false,
    )
}

#[tauri::command]
pub async fn apply_grok_official_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    account_id: String,
) -> Result<(), String> {
    let account = get_account(state.db(), &account_id)?
        .ok_or_else(|| format!("Grok official account '{account_id}' not found"))?;
    // Refresh near-expiry tokens before writing live auth.json.
    let account = ensure_fresh_grok_account_auth(state.db(), Some(&app), account, false).await?;
    let snapshot = account
        .auth_snapshot
        .ok_or_else(|| "Grok official account snapshot is unavailable".to_string())?;
    let value: Value = serde_json::from_str(&snapshot)
        .map_err(|error| format!("Invalid Grok account snapshot: {error}"))?;
    let auth_path = get_grok_auth_path_async(state.db()).await?;
    let runtime = read_auth_json_or_empty(&auth_path)?;
    let merged = merge_account_snapshot_into_runtime(runtime, &value)?;
    write_auth_json(&auth_path, &merged)?;
    let now = Local::now().to_rfc3339();
    state.db().with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CliOfficialAccount(Cli::Grok), Some(&account_id), &now)
    })?;
    let _ = app.emit("config-changed", "window");
    emit_grok_sync(&app);
    Ok(())
}

/// Force-refresh OAuth tokens for a Grok official account (not subscription limits).
#[tauri::command]
pub async fn refresh_grok_official_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    account_id: String,
) -> Result<GrokOfficialAccount, String> {
    let account = get_account(state.db(), &account_id)?
        .ok_or_else(|| format!("Grok official account '{account_id}' not found"))?;
    match ensure_fresh_grok_account_auth(state.db(), Some(&app), account, true).await {
        Ok(value) => {
            let _ = clear_account_last_error(state.db(), &account_id);
            Ok(value)
        }
        Err(error) => {
            let _ = persist_usage_error(state.db(), &account_id, &error);
            Err(error)
        }
    }
}

/// Fetch Grok CLI subscription / weekly credits from cli-chat-proxy and persist limit fields.
/// Refreshes OAuth first when the access token is near expiry. Does not change is_applied.
#[tauri::command]
pub async fn refresh_grok_official_account_limits(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    account_id: String,
) -> Result<GrokOfficialAccount, String> {
    let account = get_account(state.db(), &account_id)?
        .ok_or_else(|| format!("Grok official account '{account_id}' not found"))?;
    ensure_official_provider(state.db(), &account.provider_id)?;
    let account = match ensure_fresh_grok_account_auth(state.db(), Some(&app), account, false).await
    {
        Ok(value) => {
            // Clear stale OAuth refresh errors once token is healthy again.
            let _ = clear_account_last_error(state.db(), &account_id);
            value
        }
        Err(error) => {
            let _ = persist_usage_error(state.db(), &account_id, &error);
            return Err(error);
        }
    };
    let snapshot_text = account
        .auth_snapshot
        .as_deref()
        .ok_or_else(|| "Grok official account snapshot is unavailable".to_string())?;
    let snapshot: Value = serde_json::from_str(snapshot_text)
        .map_err(|error| format!("Invalid Grok account snapshot: {error}"))?;
    let access_token = access_token_from_snapshot(&snapshot)
        .ok_or_else(|| "Grok official account is missing access token".to_string())?;
    let usage = match fetch_grok_usage_snapshot(state.db(), &access_token).await {
        Ok(value) => value,
        Err(error) => {
            let _ = persist_usage_error(state.db(), &account_id, &error);
            return Err(error);
        }
    };
    persist_usage_snapshot(state.db(), &account_id, &usage)
}

#[tauri::command]
pub async fn delete_grok_official_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    account_id: String,
) -> Result<(), String> {
    let account = get_account(state.db(), &account_id)?;
    state
        .db()
        .with_conn(|conn| db_delete(conn, DbTable::CliOfficialAccount(Cli::Grok), &account_id).map(|_| ()))?;
    if let Some(account) = account.filter(|account| account.is_applied) {
        let snapshot: Value = serde_json::from_str(
            account
                .auth_snapshot
                .as_deref()
                .ok_or_else(|| "Grok official account snapshot is unavailable".to_string())?,
        )
        .map_err(|error| format!("Invalid Grok account snapshot: {error}"))?;
        let scope_keys = snapshot
            .as_object()
            .map(|entries| entries.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let auth_path = get_grok_auth_path_async(state.db()).await?;
        remove_auth_scopes(&auth_path, &scope_keys)?;
        // If auth.json was fully removed, also clear the auto-synced WSL target.
        if !auth_path.exists() {
            #[cfg(target_os = "windows")]
            {
                let _ = crate::coding::wsl::remove_auto_synced_wsl_mapping_target(
                    state.inner(),
                    "grok-auth",
                )
                .await;
            }
        }
        emit_grok_sync(&app);
    }
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn logout_grok_official_runtime(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let auth_path = get_grok_auth_path_async(state.db()).await?;
    let scope_keys = match read_auth_json_or_empty(&auth_path)? {
        Value::Object(entries) => entries
            .into_iter()
            .filter_map(|(key, entry)| {
                let is_xai_scope = key == auth_scope_key(XAI_ISSUER, XAI_CLIENT_ID)
                    || (entry.get("oidc_issuer").and_then(Value::as_str) == Some(XAI_ISSUER)
                        && entry.get("oidc_client_id").and_then(Value::as_str)
                            == Some(XAI_CLIENT_ID));
                is_xai_scope.then_some(key)
            })
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    remove_auth_scopes(&auth_path, &scope_keys)?;
    if !auth_path.exists() {
        #[cfg(target_os = "windows")]
        {
            let _ = crate::coding::wsl::remove_auto_synced_wsl_mapping_target(
                state.inner(),
                "grok-auth",
            )
            .await;
        }
    }
    let now = Local::now().to_rfc3339();
    state.db().with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CliOfficialAccount(Cli::Grok), None, &now)
    })?;
    let _ = app.emit("config-changed", "window");
    emit_grok_sync(&app);
    Ok(())
}

async fn poll_device_authorization(
    app: tauri::AppHandle,
    session_id: String,
    provider_id: String,
    issuer: String,
    token_endpoint: String,
    userinfo_endpoint: String,
    device_code: String,
    expires_at: i64,
    mut interval_seconds: u64,
    mut cancel_receiver: watch::Receiver<bool>,
) {
    let result = async {
        let db_state = app.state::<SqliteDbState>();
        let client = http_client::client_with_timeout(db_state.db(), 30).await?;
        loop {
            if *cancel_receiver.borrow() {
                return Err("cancelled".to_string());
            }
            if unix_now() >= expires_at {
                return Err("expired".to_string());
            }
            let response = tokio::select! {
                changed = cancel_receiver.changed() => {
                    if changed.is_ok() && *cancel_receiver.borrow() {
                        return Err("cancelled".to_string());
                    }
                    continue;
                }
                response = client.post(&token_endpoint).form(&[
                    ("grant_type", DEVICE_GRANT_TYPE),
                    ("device_code", device_code.as_str()),
                    ("client_id", XAI_CLIENT_ID),
                ]).send() => response.map_err(|error| format!("xAI device token request failed: {error}"))?
            };
            let token: TokenResponse = response
                .json()
                .await
                .map_err(|error| format!("Failed to parse xAI device token response: {error}"))?;
            match token.error.as_deref() {
                Some("authorization_pending") => {}
                Some("slow_down") => interval_seconds = interval_seconds.saturating_add(5),
                Some("expired_token") => return Err("expired".to_string()),
                Some("access_denied") => return Err("denied".to_string()),
                Some(error) => {
                    return Err(format!(
                        "{error}: {}",
                        token.error_description.as_deref().unwrap_or("unknown error")
                    ))
                }
                None => {
                    emit_status(&app, &session_id, "authorized", None, None);
                    let snapshot = build_xai_auth_snapshot(
                        &client,
                        &token,
                        &issuer,
                        &userinfo_endpoint,
                        None,
                    )
                    .await?;
                    let (email, subject) = identity_from_snapshot(&snapshot);
                    let snapshot_text = serde_json::to_string_pretty(&snapshot)
                        .map_err(|error| format!("Failed to serialize Grok auth: {error}"))?;
                    emit_status(&app, &session_id, "saving", None, None);
                    // Align with Codex: OAuth login only persists the account
                    // (is_applied=false). Do not write auth.json or mark applied;
                    // users apply explicitly via apply_grok_official_account.
                    let account = save_account(
                        db_state.db(),
                        &provider_id,
                        email.clone().unwrap_or_else(|| "xAI".to_string()),
                        email,
                        subject,
                        snapshot_text,
                        Some(token_endpoint.clone()),
                        false,
                    )?;
                    let _ = app.emit("config-changed", "window");
                    return Ok(account.id);
                }
            }
            tokio::time::sleep(Duration::from_secs(interval_seconds)).await;
        }
    }
    .await;

    AUTH_SESSIONS
        .lock()
        .ok()
        .map(|mut sessions| sessions.remove(&session_id));
    match result {
        Ok(account_id) => emit_status(&app, &session_id, "completed", None, Some(account_id)),
        Err(status) if matches!(status.as_str(), "cancelled" | "expired" | "denied") => {
            emit_status(&app, &session_id, &status, None, None)
        }
        Err(error) => emit_status(&app, &session_id, "failed", Some(error), None),
    }
}

fn validate_xai_endpoint(raw: &str, field: &str) -> Result<String, String> {
    let url = Url::parse(raw).map_err(|error| format!("Invalid xAI {field}: {error}"))?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if url.scheme() != "https" || (host != "x.ai" && !host.ends_with(".x.ai")) {
        return Err(format!("xAI {field} must use HTTPS on x.ai"));
    }
    Ok(url.to_string())
}

async fn build_xai_auth_snapshot(
    client: &reqwest::Client,
    token: &TokenResponse,
    issuer: &str,
    userinfo_endpoint: &str,
    previous_snapshot: Option<&Value>,
) -> Result<Value, String> {
    let access_token = token
        .access_token
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "xAI token response missing access_token".to_string())?;
    let userinfo_endpoint = validate_xai_endpoint(userinfo_endpoint, "userinfo_endpoint")?;
    let userinfo: UserInfoResponse = client
        .get(userinfo_endpoint)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|error| format!("xAI userinfo request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("xAI userinfo request failed: {error}"))?
        .json()
        .await
        .map_err(|error| format!("Failed to parse xAI userinfo: {error}"))?;
    build_xai_auth_snapshot_from_userinfo(token, issuer, userinfo, previous_snapshot)
}

fn build_xai_auth_snapshot_from_userinfo(
    token: &TokenResponse,
    issuer: &str,
    userinfo: UserInfoResponse,
    previous_snapshot: Option<&Value>,
) -> Result<Value, String> {
    let access_token = token
        .access_token
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "xAI token response missing access_token".to_string())?;
    let claims = decode_jwt_claims(access_token).unwrap_or_else(|| json!({}));
    let client_id = claims
        .get("client_id")
        .and_then(Value::as_str)
        .unwrap_or(XAI_CLIENT_ID);
    let scope_key = auth_scope_key(issuer, client_id);
    let mut entry = previous_snapshot
        .and_then(|snapshot| find_auth_entry_by_scope(snapshot, &scope_key))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    entry.insert("key".to_string(), json!(access_token));
    entry.insert("auth_mode".to_string(), json!("oidc"));
    entry
        .entry("create_time".to_string())
        .or_insert_with(|| json!(chrono::Utc::now().to_rfc3339()));
    insert_optional_string(
        &mut entry,
        "user_id",
        userinfo
            .sub
            .as_deref()
            .or_else(|| claims.get("sub").and_then(Value::as_str)),
    );
    insert_optional_string(&mut entry, "email", userinfo.email.as_deref());
    insert_optional_string(&mut entry, "first_name", userinfo.given_name.as_deref());
    insert_optional_string(
        &mut entry,
        "profile_image_asset_id",
        userinfo.picture.as_deref(),
    );
    for field in ["principal_type", "principal_id", "team_id"] {
        insert_optional_string(&mut entry, field, claims.get(field).and_then(Value::as_str));
    }
    if let Some(value) = token
        .refresh_token
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        entry.insert("refresh_token".to_string(), json!(value));
    }
    if let Some(expires_in) = token.expires_in {
        entry.insert(
            "expires_at".to_string(),
            json!((chrono::Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339()),
        );
    }
    entry.insert(
        "oidc_issuer".to_string(),
        json!(issuer.trim_end_matches('/')),
    );
    entry.insert("oidc_client_id".to_string(), json!(client_id));
    Ok(single_account_snapshot(scope_key, Value::Object(entry)))
}

fn identity_from_snapshot(snapshot: &Value) -> (Option<String>, Option<String>) {
    let entry = find_xai_auth_entry(snapshot)
        .ok()
        .map(|(_, entry)| entry)
        .unwrap_or(snapshot);
    let email = entry
        .get("email")
        .and_then(Value::as_str)
        .map(str::to_string);
    let subject = entry
        .get("user_id")
        .or_else(|| entry.get("principal_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    (email, subject)
}

fn decode_jwt_claims(token: &str) -> Option<Value> {
    let Some(payload) = token.split('.').nth(1) else {
        return None;
    };
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
}

fn auth_scope_key(issuer: &str, client_id: &str) -> String {
    format!("{}::{client_id}", issuer.trim_end_matches('/'))
}

fn find_auth_entry_by_scope<'a>(snapshot: &'a Value, scope_key: &str) -> Option<&'a Value> {
    snapshot.as_object()?.get(scope_key)
}

fn find_xai_auth_entry(snapshot: &Value) -> Result<(String, &Value), String> {
    let expected_key = auth_scope_key(XAI_ISSUER, XAI_CLIENT_ID);
    if let Some(entry) = find_auth_entry_by_scope(snapshot, &expected_key) {
        return Ok((expected_key, entry));
    }
    snapshot
        .as_object()
        .and_then(|entries| {
            entries.iter().find(|(_, entry)| {
                entry.get("oidc_issuer").and_then(Value::as_str) == Some(XAI_ISSUER)
                    && entry.get("oidc_client_id").and_then(Value::as_str) == Some(XAI_CLIENT_ID)
            })
        })
        .map(|(key, entry)| (key.clone(), entry))
        .ok_or_else(|| "Grok auth.json does not contain the xAI OAuth account scope".to_string())
}

fn single_account_snapshot(scope_key: String, entry: Value) -> Value {
    let mut root = serde_json::Map::new();
    root.insert(scope_key, entry);
    Value::Object(root)
}

fn insert_optional_string(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        object.insert(key.to_string(), json!(value));
    }
}

fn read_auth_json_or_empty(path: &Path) -> Result<Value, String> {
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|error| format!("Invalid Grok auth.json: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(error) => Err(format!("Failed to read {}: {error}", path.display())),
    }
}

fn merge_account_snapshot_into_runtime(
    mut runtime: Value,
    account_snapshot: &Value,
) -> Result<Value, String> {
    let runtime_entries = runtime
        .as_object_mut()
        .ok_or_else(|| "Grok runtime auth.json must be an object".to_string())?;
    let account_entries = account_snapshot
        .as_object()
        .ok_or_else(|| "Grok account snapshot must be an object".to_string())?;
    for (scope_key, saved_entry) in account_entries {
        let merged_entry = match (runtime_entries.get(scope_key), saved_entry.as_object()) {
            (Some(current), Some(saved))
                if current.get("principal_id") == saved_entry.get("principal_id") =>
            {
                let mut merged = current.as_object().cloned().unwrap_or_default();
                for (key, value) in saved {
                    merged.insert(key.clone(), value.clone());
                }
                Value::Object(merged)
            }
            _ => saved_entry.clone(),
        };
        runtime_entries.insert(scope_key.clone(), merged_entry);
    }
    Ok(runtime)
}

fn save_account(
    db: &SqliteDbState,
    provider_id: &str,
    name: String,
    email: Option<String>,
    subject: Option<String>,
    snapshot: String,
    token_endpoint: Option<String>,
    is_applied: bool,
) -> Result<GrokOfficialAccount, String> {
    let id = db_new_id();
    let sort_index = db.with_conn(|conn| {
        Ok(crate::db::helpers::db_max_i64(
            conn,
            DbTable::CliOfficialAccount(Cli::Grok),
            &crate::db::schema::JsonFieldPath::new("sort_index")?,
        )?
        .map(|value| value as i32 + 1)
        .unwrap_or(0))
    })?;
    save_account_with_id(
        db,
        &id,
        provider_id,
        name,
        email,
        subject,
        snapshot,
        token_endpoint,
        is_applied,
        Some(sort_index),
        Local::now().to_rfc3339(),
    )
}

#[allow(clippy::too_many_arguments)]
fn save_account_with_id(
    db: &SqliteDbState,
    id: &str,
    provider_id: &str,
    name: String,
    email: Option<String>,
    subject: Option<String>,
    snapshot: String,
    token_endpoint: Option<String>,
    is_applied: bool,
    sort_index: Option<i32>,
    created_at: String,
) -> Result<GrokOfficialAccount, String> {
    let existing = get_account(db, id)?;
    let snapshot_value: Value = serde_json::from_str(&snapshot).unwrap_or_else(|_| json!({}));
    let auth_entry = find_xai_auth_entry(&snapshot_value)
        .ok()
        .map(|(_, entry)| entry)
        .unwrap_or(&snapshot_value);
    let expires_at = auth_entry
        .get("expires_at")
        .and_then(Value::as_i64)
        .or_else(|| {
            auth_entry
                .get("expires_at")
                .and_then(Value::as_str)
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.timestamp())
        });
    let updated_at = Local::now().to_rfc3339();
    // Preserve previously fetched limit fields when rotating OAuth tokens.
    let mut data = json!({
        "provider_id": provider_id,
        "name": name,
        "kind": "oauth",
        "email": email,
        "subject": subject,
        "auth_snapshot": snapshot,
        "token_endpoint": token_endpoint,
        "expires_at": expires_at,
        "last_refresh": updated_at.clone(),
        "last_error": null,
        "is_applied": is_applied,
        "sort_index": sort_index,
        "created_at": created_at,
        "updated_at": updated_at,
    });
    if let Some(existing) = existing {
        if let Some(object) = data.as_object_mut() {
            if let Some(plan_type) = existing.plan_type {
                object.insert("plan_type".to_string(), Value::String(plan_type));
            }
            if let Some(text) = existing.limit_weekly_text {
                object.insert("limit_weekly_text".to_string(), Value::String(text));
            }
            if let Some(text) = existing.limit_monthly_text {
                object.insert("limit_monthly_text".to_string(), Value::String(text));
            }
            if let Some(value) = existing.limit_weekly_reset_at {
                object.insert("limit_weekly_reset_at".to_string(), Value::from(value));
            }
            if let Some(value) = existing.limit_monthly_reset_at {
                object.insert("limit_monthly_reset_at".to_string(), Value::from(value));
            }
            if let Some(value) = existing.last_limits_fetched_at {
                object.insert("last_limits_fetched_at".to_string(), Value::String(value));
            }
        }
    }
    db.with_conn(|conn| db_put(conn, DbTable::CliOfficialAccount(Cli::Grok), id, &data))?;
    get_account(db, id)?.ok_or_else(|| "Failed to read saved Grok account".to_string())
}

fn get_account(db: &SqliteDbState, id: &str) -> Result<Option<GrokOfficialAccount>, String> {
    db.with_conn(|conn| db_get(conn, DbTable::CliOfficialAccount(Cli::Grok), id))
        .map(|value| value.map(account_from_db_value))
}

fn account_from_db_value(value: Value) -> GrokOfficialAccount {
    GrokOfficialAccount {
        id: db_extract_id(&value),
        provider_id: string_field(&value, "provider_id"),
        name: string_field(&value, "name"),
        kind: value
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("oauth")
            .to_string(),
        email: optional_string(&value, "email"),
        subject: optional_string(&value, "subject"),
        auth_snapshot: optional_string(&value, "auth_snapshot"),
        token_endpoint: optional_string(&value, "token_endpoint"),
        expires_at: value.get("expires_at").and_then(Value::as_i64),
        last_refresh: optional_string(&value, "last_refresh"),
        last_error: optional_string(&value, "last_error"),
        plan_type: optional_string(&value, "plan_type"),
        limit_weekly_text: optional_string(&value, "limit_weekly_text"),
        limit_monthly_text: optional_string(&value, "limit_monthly_text"),
        limit_weekly_reset_at: value.get("limit_weekly_reset_at").and_then(Value::as_i64),
        limit_monthly_reset_at: value.get("limit_monthly_reset_at").and_then(Value::as_i64),
        last_limits_fetched_at: optional_string(&value, "last_limits_fetched_at"),
        is_applied: value
            .get("is_applied")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        sort_index: value
            .get("sort_index")
            .and_then(Value::as_i64)
            .map(|v| v as i32),
        created_at: string_field(&value, "created_at"),
        updated_at: string_field(&value, "updated_at"),
    }
}

fn access_token_from_snapshot(snapshot: &Value) -> Option<String> {
    let entry = find_xai_auth_entry(snapshot)
        .ok()
        .map(|(_, entry)| entry)
        .unwrap_or(snapshot);
    entry
        .get("key")
        .or_else(|| entry.get("access_token"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn refresh_token_from_snapshot(snapshot: &Value) -> Option<String> {
    let entry = find_xai_auth_entry(snapshot)
        .ok()
        .map(|(_, entry)| entry)
        .unwrap_or(snapshot);
    entry
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn token_endpoint_from_account(account: &GrokOfficialAccount, snapshot: &Value) -> Option<String> {
    account.token_endpoint.clone().or_else(|| {
        let entry = find_xai_auth_entry(snapshot)
            .ok()
            .map(|(_, entry)| entry)
            .unwrap_or(snapshot);
        entry
            .get("oidc_issuer")
            .and_then(Value::as_str)
            .map(|issuer| format!("{}/oauth2/token", issuer.trim_end_matches('/')))
    })
}

fn access_token_expiration_unix(snapshot: &Value) -> Option<i64> {
    let entry = find_xai_auth_entry(snapshot)
        .ok()
        .map(|(_, entry)| entry)
        .unwrap_or(snapshot);
    let from_field = entry.get("expires_at").and_then(|value| match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => chrono::DateTime::parse_from_rfc3339(text.trim())
            .ok()
            .map(|value| value.timestamp()),
        _ => None,
    });
    let from_jwt = access_token_from_snapshot(snapshot).and_then(|token| {
        decode_jwt_claims(&token).and_then(|claims| claims.get("exp").and_then(Value::as_i64))
    });
    match (from_field, from_jwt) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn grok_account_needs_refresh(snapshot: &Value) -> bool {
    let now = chrono::Utc::now().timestamp();
    match access_token_expiration_unix(snapshot) {
        Some(expires_at) => expires_at <= now + GROK_AUTH_REFRESH_LEAD_SECONDS,
        // Missing expiry: refresh conservatively so short-lived tokens do not silently die.
        None => true,
    }
}

async fn request_xai_token_refresh(
    db: &SqliteDbState,
    token_endpoint: &str,
    refresh_token: &str,
) -> Result<TokenResponse, String> {
    let client = http_client::client_with_timeout(db, 30).await?;
    let response = client
        .post(token_endpoint)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", XAI_CLIENT_ID),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .map_err(|error| format!("xAI token refresh failed: {error}"))?;
    let status = response.status();
    let body_text = response
        .text()
        .await
        .map_err(|error| format!("Failed to read xAI token refresh body: {error}"))?;
    let token: TokenResponse = serde_json::from_str(&body_text).unwrap_or(TokenResponse {
        access_token: None,
        refresh_token: None,
        expires_in: None,
        error: None,
        error_description: None,
    });
    if !status.is_success() || token.error.is_some() || token.access_token.is_none() {
        return Err(format_xai_token_refresh_error(
            status.as_u16(),
            &body_text,
            &token,
        ));
    }
    Ok(token)
}

fn format_xai_token_refresh_error(status: u16, body_text: &str, token: &TokenResponse) -> String {
    let error_code = token
        .error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let error_description = token
        .error_description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let body_looks_like_invalid_grant = body_text.to_ascii_lowercase().contains("invalid_grant");
    match (error_code, error_description) {
        (Some("invalid_grant"), Some(description)) => format!(
            "xAI refresh token is invalid or revoked (HTTP {status}, invalid_grant): {description}. Re-login with Device Code is required."
        ),
        (Some("invalid_grant"), None) => format!(
            "xAI refresh token is invalid or revoked (HTTP {status}, invalid_grant). Re-login with Device Code is required."
        ),
        (None, _) if status == 400 && body_looks_like_invalid_grant => format!(
            "xAI refresh token is invalid or revoked (HTTP {status}, invalid_grant). Re-login with Device Code is required."
        ),
        (None, _) if status == 400 => format!(
            "xAI token refresh failed (HTTP {status}). Refresh token is likely invalid or revoked; Re-login with Device Code is required."
        ),
        (Some(code), Some(description)) => {
            format!("xAI token refresh failed (HTTP {status}, {code}): {description}")
        }
        (Some(code), None) => format!("xAI token refresh failed (HTTP {status}, {code})"),
        (None, Some(description)) => {
            format!("xAI token refresh failed (HTTP {status}): {description}")
        }
        (None, None) => {
            let trimmed = body_text.trim();
            if trimmed.is_empty() {
                format!("xAI token refresh failed (HTTP {status})")
            } else {
                format!("xAI token refresh failed (HTTP {status}): {trimmed}")
            }
        }
    }
}

/// Refresh OAuth when forced or when access token is within the lead window.
/// When applied, also merges the new snapshot into live auth.json.
async fn ensure_fresh_grok_account_auth(
    db: &SqliteDbState,
    app: Option<&tauri::AppHandle>,
    account: GrokOfficialAccount,
    force: bool,
) -> Result<GrokOfficialAccount, String> {
    let snapshot_text = account
        .auth_snapshot
        .as_deref()
        .ok_or_else(|| "Grok official account snapshot is unavailable".to_string())?;
    let snapshot: Value = serde_json::from_str(snapshot_text)
        .map_err(|error| format!("Invalid Grok account snapshot: {error}"))?;
    if !force && !grok_account_needs_refresh(&snapshot) {
        return Ok(account);
    }

    let _refresh_guard = OAUTH_REFRESH_LOCK.lock().await;
    // Re-read after lock: another caller may have refreshed already.
    let account = get_account(db, &account.id)?
        .ok_or_else(|| format!("Grok official account '{}' not found", account.id))?;
    let snapshot_text = account
        .auth_snapshot
        .as_deref()
        .ok_or_else(|| "Grok official account snapshot is unavailable".to_string())?;
    let snapshot: Value = serde_json::from_str(snapshot_text)
        .map_err(|error| format!("Invalid Grok account snapshot: {error}"))?;
    if !force && !grok_account_needs_refresh(&snapshot) {
        return Ok(account);
    }

    let refresh_token = match refresh_token_from_snapshot(&snapshot) {
        Some(value) => value,
        None => {
            let error = "Grok account does not contain a refresh token. Re-login with Device Code is required.".to_string();
            let _ = persist_usage_error(db, &account.id, &error);
            return Err(error);
        }
    };
    let token_endpoint = match token_endpoint_from_account(&account, &snapshot) {
        Some(value) => value,
        None => {
            let error = "Grok account does not contain a token endpoint".to_string();
            let _ = persist_usage_error(db, &account.id, &error);
            return Err(error);
        }
    };
    let token_endpoint = match validate_xai_endpoint(&token_endpoint, "token_endpoint") {
        Ok(value) => value,
        Err(error) => {
            let _ = persist_usage_error(db, &account.id, &error);
            return Err(error);
        }
    };

    let cached_response = OAUTH_REFRESH_CACHE
        .lock()
        .await
        .get(&refresh_token)
        .filter(|(created_at, _)| created_at.elapsed() <= GROK_AUTH_REFRESH_CACHE_TTL)
        .map(|(_, response)| response.clone());
    let token_response = match cached_response {
        Some(response) => response,
        None => match request_xai_token_refresh(db, &token_endpoint, &refresh_token).await {
            Ok(response) => {
                OAUTH_REFRESH_CACHE
                    .lock()
                    .await
                    .insert(refresh_token.clone(), (Instant::now(), response.clone()));
                response
            }
            Err(error) => {
                let _ = persist_usage_error(db, &account.id, &error);
                return Err(error);
            }
        },
    };

    let entry = find_xai_auth_entry(&snapshot)
        .ok()
        .map(|(_, entry)| entry)
        .unwrap_or(&snapshot);
    let issuer = entry
        .get("oidc_issuer")
        .and_then(Value::as_str)
        .unwrap_or(XAI_ISSUER)
        .to_string();
    let userinfo_endpoint = format!("{}/oauth2/userinfo", issuer.trim_end_matches('/'));
    let client = match http_client::client_with_timeout(db, 30).await {
        Ok(value) => value,
        Err(error) => {
            let _ = persist_usage_error(db, &account.id, &error);
            return Err(error);
        }
    };
    let refreshed_snapshot = match build_xai_auth_snapshot(
        &client,
        &token_response,
        &issuer,
        &userinfo_endpoint,
        Some(&snapshot),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            let _ = persist_usage_error(db, &account.id, &error);
            return Err(error);
        }
    };
    let snapshot_text = match serde_json::to_string_pretty(&refreshed_snapshot) {
        Ok(value) => value,
        Err(error) => {
            let message = format!("Failed to serialize Grok auth snapshot: {error}");
            let _ = persist_usage_error(db, &account.id, &message);
            return Err(message);
        }
    };
    let (email, subject) = identity_from_snapshot(&refreshed_snapshot);
    let updated = match save_account_with_id(
        db,
        &account.id,
        &account.provider_id,
        account.name,
        email.or(account.email),
        subject.or(account.subject),
        snapshot_text,
        Some(token_endpoint),
        account.is_applied,
        account.sort_index,
        account.created_at,
    ) {
        Ok(value) => value,
        Err(error) => {
            let _ = persist_usage_error(db, &account.id, &error);
            return Err(error);
        }
    };
    // save_account_with_id rebuilds the row; clear any previous refresh error.
    let _ = clear_account_last_error(db, &updated.id);
    if updated.is_applied {
        let auth_path = match get_grok_auth_path_async(db).await {
            Ok(value) => value,
            Err(error) => {
                let _ = persist_usage_error(db, &updated.id, &error);
                return Err(error);
            }
        };
        let runtime = match read_auth_json_or_empty(&auth_path) {
            Ok(value) => value,
            Err(error) => {
                let _ = persist_usage_error(db, &updated.id, &error);
                return Err(error);
            }
        };
        let merged = match merge_account_snapshot_into_runtime(runtime, &refreshed_snapshot) {
            Ok(value) => value,
            Err(error) => {
                let _ = persist_usage_error(db, &updated.id, &error);
                return Err(error);
            }
        };
        if let Err(error) = write_auth_json(&auth_path, &merged) {
            let _ = persist_usage_error(db, &updated.id, &error);
            return Err(error);
        }
        if let Some(app) = app {
            emit_grok_sync(app);
            let _ = app.emit("config-changed", "window");
        }
    }
    get_account(db, &updated.id)?.ok_or_else(|| "Failed to read refreshed Grok account".to_string())
}

/// Background / startup pass entry used by `coding::auth_refresh`.
///
/// Refreshes OAuth for every persisted official account (applied and not).
/// `ensure_fresh` only writes live `auth.json` when the account is applied.
pub async fn refresh_applied_grok_accounts_if_needed(
    db: &SqliteDbState,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    let order = OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::created_at(OrderDirection::Asc),
    ]);
    let accounts = db
        .with_conn(|conn| db_list(conn, DbTable::CliOfficialAccount(Cli::Grok), Some(&order)))?
        .into_iter()
        .map(account_from_db_value)
        .collect::<Vec<_>>();
    for account in accounts {
        match ensure_fresh_grok_account_auth(db, Some(app), account, false).await {
            Ok(_) => {}
            Err(error) => {
                // ensure_fresh already persists last_error for OAuth failures.
                log::debug!("Grok official account background refresh failed: {error}");
            }
        }
    }
    Ok(())
}

fn apply_cli_proxy_headers(
    request: reqwest::RequestBuilder,
    access_token: &str,
) -> reqwest::RequestBuilder {
    request
        .bearer_auth(access_token)
        .header("X-XAI-Token-Auth", GROK_CLI_TOKEN_AUTH)
        .header("x-grok-client-version", GROK_CLI_CLIENT_VERSION)
        .header("x-grok-client-identifier", GROK_CLI_CLIENT_IDENTIFIER)
        .header("x-grok-client-mode", "headless")
        .header("Accept", "application/json")
        .header("User-Agent", GROK_CLI_USER_AGENT)
}

async fn fetch_grok_usage_snapshot(
    db: &SqliteDbState,
    access_token: &str,
) -> Result<GrokUsageSnapshot, String> {
    let client = http_client::client_with_timeout(db, 20).await?;

    // credits: weekly pool / creditUsagePercent / currentPeriod
    // default billing: monthlyLimit/used + calendar billingPeriod*
    // Free unified-billing credits responses often copy weekly bounds into billingPeriod*,
    // so monthly must come from the default billing payload, not credits.
    let credits_url = format!("{GROK_CLI_PROXY_BASE}/billing?format=credits");
    let credits_response = apply_cli_proxy_headers(client.get(credits_url), access_token)
        .send()
        .await
        .map_err(|error| format!("Failed to fetch Grok billing credits: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Grok billing credits request failed: {error}"))?;
    let credits_body = credits_response
        .json::<Value>()
        .await
        .map_err(|error| format!("Failed to parse Grok billing credits response: {error}"))?;

    let monthly_url = format!("{GROK_CLI_PROXY_BASE}/billing");
    let monthly_body = match apply_cli_proxy_headers(client.get(monthly_url), access_token)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => response.json::<Value>().await.ok(),
        _ => None,
    };

    let user_url = format!("{GROK_CLI_PROXY_BASE}/user?include=subscription");
    let subscription_tier = match apply_cli_proxy_headers(client.get(user_url), access_token)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => response
            .json::<Value>()
            .await
            .ok()
            .and_then(|body| parse_subscription_tier(&body)),
        _ => None,
    };

    Ok(parse_grok_usage_snapshot(
        &credits_body,
        monthly_body.as_ref(),
        subscription_tier.as_deref(),
        access_token,
    ))
}

fn parse_subscription_tier(body: &Value) -> Option<String> {
    body.get("subscriptionTier")
        .or_else(|| body.get("subscription_tier"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            body.get("user").and_then(|user| {
                user.get("subscriptionTier")
                    .or_else(|| user.get("subscription_tier"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
        })
}

fn subscription_tier_from_jwt(access_token: &str) -> Option<String> {
    let claims = decode_jwt_claims(access_token)?;
    let tier = claims.get("tier")?;
    if let Some(text) = tier
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Ok(number) = text.parse::<i64>() {
            return subscription_tier_from_number(number);
        }
        return Some(text.to_string());
    }
    tier.as_i64()
        .or_else(|| tier.as_f64().map(|value| value as i64))
        .and_then(subscription_tier_from_number)
}

fn subscription_tier_from_number(tier: i64) -> Option<String> {
    Some(
        match tier {
            0 => "free",
            1 => "supergrok",
            2 => "x_basic",
            3 => "x_premium",
            4 => "x_premium_plus",
            5 => "supergrok_heavy",
            6 => "supergrok_lite",
            _ => return None,
        }
        .to_string(),
    )
}

fn parse_grok_usage_snapshot(
    credits_body: &Value,
    monthly_body: Option<&Value>,
    subscription_tier: Option<&str>,
    access_token: &str,
) -> GrokUsageSnapshot {
    let credits_root = billing_config_root(credits_body);
    let monthly_root = monthly_body.map(billing_config_root);

    let credit_usage_percent = json_number(
        credits_root
            .get("creditUsagePercent")
            .or_else(|| credits_root.get("credit_usage_percent")),
    );

    let current_period = credits_root
        .get("currentPeriod")
        .or_else(|| credits_root.get("current_period"));
    let weekly_end = current_period
        .and_then(|period| period.get("end"))
        .and_then(Value::as_str);
    let weekly_reset_at = weekly_end.and_then(parse_rfc3339_timestamp);

    // Monthly only from default billing when monthlyLimit > 0.
    // Do NOT use credits billingPeriodEnd: free/unified responses reuse weekly bounds there.
    let monthly_limit = monthly_root.and_then(|root| {
        json_number(
            root.get("monthlyLimit")
                .or_else(|| root.get("monthly_limit")),
        )
    });
    let monthly_used = monthly_root.and_then(|root| {
        json_number(
            root.get("used")
                .or_else(|| root.get("totalUsed"))
                .or_else(|| root.get("includedUsed")),
        )
    });
    let has_monthly_quota = monthly_limit.filter(|value| *value > 0.0).is_some();
    let limit_monthly_text = if has_monthly_quota {
        monthly_limit.zip(monthly_used).map(|(limit, used)| {
            let remaining = ((limit - used) / limit * 100.0).clamp(0.0, 100.0);
            format_percent_label(remaining)
        })
    } else {
        None
    };
    let limit_monthly_reset_at = if has_monthly_quota {
        monthly_root
            .and_then(|root| {
                root.get("billingPeriodEnd")
                    .or_else(|| root.get("billing_period_end"))
            })
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_timestamp)
    } else {
        None
    };

    let plan_from_billing = plan_name_from_billing(credits_body, credits_root).or_else(|| {
        monthly_body
            .and_then(|body| monthly_root.and_then(|root| plan_name_from_billing(body, root)))
    });
    let plan_type = subscription_tier
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or(plan_from_billing)
        .or_else(|| subscription_tier_from_jwt(access_token))
        .or_else(|| Some("free".to_string()));

    let limit_weekly_text = credit_usage_percent
        .filter(|value| (0.0..=100.0).contains(value))
        .map(|used| format_percent_label((100.0 - used).clamp(0.0, 100.0)));

    GrokUsageSnapshot {
        plan_type,
        limit_weekly_text,
        limit_monthly_text,
        limit_weekly_reset_at: weekly_reset_at,
        limit_monthly_reset_at,
    }
}

fn billing_config_root(body: &Value) -> &Value {
    body.get("config")
        .filter(|value| value.is_object())
        .unwrap_or(body)
}

fn plan_name_from_billing(billing_body: &Value, root: &Value) -> Option<String> {
    for source in [billing_body, root] {
        if let Some(name) = source
            .get("subscriptionTier")
            .or_else(|| source.get("subscription_tier"))
            .or_else(|| source.get("planName"))
            .or_else(|| source.get("plan_name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(name.to_string());
        }
        for key in ["subscription", "plan", "membership"] {
            if let Some(object) = source.get(key).and_then(Value::as_object) {
                if let Some(name) = object
                    .get("name")
                    .or_else(|| object.get("displayName"))
                    .or_else(|| object.get("display_name"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Some(name.to_string());
                }
                if let Some(code) = object
                    .get("code")
                    .or_else(|| object.get("tier"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Some(code.to_string());
                }
            }
        }
    }
    None
}

fn json_number(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        Value::Object(object) => object.get("val").and_then(|nested| match nested {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        }),
        _ => None,
    }
}

fn parse_rfc3339_timestamp(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|value| value.timestamp())
}

fn format_percent_label(value: f64) -> String {
    if (value.fract()).abs() < f64::EPSILON {
        format!("{}%", value.round() as i64)
    } else {
        format!("{value:.1}%")
    }
}

fn persist_usage_snapshot(
    db: &SqliteDbState,
    account_id: &str,
    usage: &GrokUsageSnapshot,
) -> Result<GrokOfficialAccount, String> {
    let now = Local::now().to_rfc3339();
    let patch = [
        (
            "plan_type",
            usage
                .plan_type
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "limit_weekly_text",
            usage
                .limit_weekly_text
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "limit_monthly_text",
            usage
                .limit_monthly_text
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "limit_weekly_reset_at",
            usage
                .limit_weekly_reset_at
                .map(Value::from)
                .unwrap_or(Value::Null),
        ),
        (
            "limit_monthly_reset_at",
            usage
                .limit_monthly_reset_at
                .map(Value::from)
                .unwrap_or(Value::Null),
        ),
        ("last_limits_fetched_at", Value::String(now)),
        ("last_error", Value::Null),
    ];
    db.with_conn(|conn| {
        db_patch_fields(conn, DbTable::CliOfficialAccount(Cli::Grok), account_id, &patch)?;
        Ok(())
    })?;
    get_account(db, account_id)?.ok_or_else(|| "Failed to read updated Grok account".to_string())
}

fn persist_usage_error(db: &SqliteDbState, account_id: &str, error: &str) -> Result<(), String> {
    db.with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::CliOfficialAccount(Cli::Grok),
            account_id,
            &[("last_error", Value::String(error.to_string()))],
        )?;
        Ok(())
    })
}

fn clear_account_last_error(db: &SqliteDbState, account_id: &str) -> Result<(), String> {
    db.with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::CliOfficialAccount(Cli::Grok),
            account_id,
            &[("last_error", Value::Null)],
        )?;
        Ok(())
    })
}

fn ensure_official_provider(db: &SqliteDbState, provider_id: &str) -> Result<(), String> {
    let provider = db
        .with_conn(|conn| db_get(conn, DbTable::CliProvider(Cli::Grok), provider_id))?
        .map(adapter::provider_from_db_value)
        .ok_or_else(|| format!("Grok provider '{provider_id}' not found"))?;
    if provider.category != "official" {
        return Err("Grok official accounts require an official provider".to_string());
    }
    Ok(())
}

/// Clear every Grok official-account applied marker (used when leaving official provider).
pub async fn clear_all_grok_official_account_apply_status(
    db: &SqliteDbState,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CliOfficialAccount(Cli::Grok), None, &now)
    })?;
    Ok(())
}

/// Align account applied tags with the live auth.json identity for an official provider.
pub async fn sync_grok_official_account_apply_status(
    db: &SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    let auth_path = get_grok_auth_path_async(db).await?;
    let runtime = read_auth_json_or_empty(&auth_path)?;
    let matched_account_id = if find_xai_auth_entry(&runtime).is_ok() {
        let (email, subject) = identity_from_snapshot(&runtime);
        list_persisted_official_accounts(db, provider_id)?
            .into_iter()
            .find(|account| {
                official_account_identity_matches(account, email.as_deref(), subject.as_deref())
            })
            .map(|account| account.id)
    } else {
        None
    };
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(
            conn,
            DbTable::CliOfficialAccount(Cli::Grok),
            matched_account_id.as_deref(),
            &now,
        )
    })?;
    Ok(())
}

fn list_persisted_official_accounts(
    db: &SqliteDbState,
    provider_id: &str,
) -> Result<Vec<GrokOfficialAccount>, String> {
    let order = OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::created_at(OrderDirection::Asc),
    ]);
    db.with_conn(|conn| db_list(conn, DbTable::CliOfficialAccount(Cli::Grok), Some(&order)))
        .map(|values| {
            values
                .into_iter()
                .filter(|value| {
                    value.get("provider_id").and_then(Value::as_str) == Some(provider_id)
                })
                .map(account_from_db_value)
                .collect()
        })
}

fn official_account_identity_matches(
    account: &GrokOfficialAccount,
    email: Option<&str>,
    subject: Option<&str>,
) -> bool {
    let account_email = account
        .email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    let account_subject = account
        .subject
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let email = email
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    let subject = subject.map(str::trim).filter(|value| !value.is_empty());

    match (account_subject, subject) {
        (Some(left), Some(right)) if left == right => return true,
        _ => {}
    }
    match (account_email, email) {
        (Some(left), Some(right)) if left == right => true,
        _ => false,
    }
}

fn write_auth_json(path: &Path, value: &Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| format!("Failed to create temp auth file: {error}"))?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), value)
        .map_err(|error| format!("Failed to serialize Grok auth.json: {error}"))?;
    temporary
        .write_all(b"\n")
        .map_err(|error| format!("Failed to finalize Grok auth.json: {error}"))?;
    temporary
        .persist(path)
        .map_err(|error| format!("Failed to replace {}: {}", path.display(), error.error))?;
    set_auth_permissions(path)?;
    Ok(())
}

fn remove_auth_json(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("Failed to remove {}: {error}", path.display()))?;
    }
    Ok(())
}

fn remove_auth_scopes(path: &Path, scope_keys: &[String]) -> Result<(), String> {
    if scope_keys.is_empty() || !path.exists() {
        return Ok(());
    }
    let mut runtime = read_auth_json_or_empty(path)?;
    let entries = runtime
        .as_object_mut()
        .ok_or_else(|| "Grok runtime auth.json must be an object".to_string())?;
    for scope_key in scope_keys {
        entries.remove(scope_key);
    }
    if entries.is_empty() {
        remove_auth_json(path)
    } else {
        write_auth_json(path, &runtime)
    }
}

#[cfg(unix)]
fn set_auth_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("Failed to set {} permissions: {error}", path.display()))
}

#[cfg(not(unix))]
fn set_auth_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn emit_status(
    app: &tauri::AppHandle,
    session_id: &str,
    status: &str,
    message: Option<String>,
    account_id: Option<String>,
) {
    if let Ok(mut statuses) = AUTH_SESSION_STATUSES.lock() {
        statuses.insert(session_id.to_string(), status.to_string());
        if statuses.len() > 64 {
            if let Some(oldest_key) = statuses.keys().next().cloned() {
                statuses.remove(&oldest_key);
            }
        }
    }
    let _ = app.emit(
        "grok-auth-status",
        GrokAuthStatusEvent {
            session_id: session_id.to_string(),
            status: status.to_string(),
            message,
            account_id,
        },
    );
}

#[cfg(target_os = "windows")]
fn emit_grok_sync(app: &tauri::AppHandle) {
    let _ = app.emit("wsl-sync-request-grok", ());
}

#[cfg(not(target_os = "windows"))]
fn emit_grok_sync(_app: &tauri::AppHandle) {}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_discovery_endpoint_outside_xai() {
        assert!(validate_xai_endpoint("https://auth.x.ai/token", "token").is_ok());
        assert!(validate_xai_endpoint("https://sub.auth.x.ai/token", "token").is_ok());
        assert!(validate_xai_endpoint("http://auth.x.ai/token", "token").is_err());
        assert!(validate_xai_endpoint("https://x.ai.evil.example/token", "token").is_err());
    }

    #[test]
    fn parse_usage_snapshot_weekly_remaining_percent() {
        let credits = json!({
            "config": {
                "creditUsagePercent": 11.0,
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "start": "2026-07-08T00:00:00Z",
                    "end": "2026-07-15T00:00:00Z"
                },
                // Free/unified credits may mirror weekly bounds into billingPeriod*;
                // parser must ignore these for monthly.
                "billingPeriodStart": "2026-07-08T00:00:00Z",
                "billingPeriodEnd": "2026-07-15T00:00:00Z"
            }
        });
        let monthly = json!({
            "config": {
                "monthlyLimit": { "val": 0 },
                "used": { "val": 0 },
                "billingPeriodEnd": "2026-08-01T00:00:00Z"
            }
        });
        let snapshot =
            parse_grok_usage_snapshot(&credits, Some(&monthly), Some("SuperGrokPro"), "not-a-jwt");
        assert_eq!(snapshot.plan_type.as_deref(), Some("SuperGrokPro"));
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("89%"));
        assert_eq!(
            snapshot.limit_weekly_reset_at,
            parse_rfc3339_timestamp("2026-07-15T00:00:00Z")
        );
        assert!(snapshot.limit_monthly_text.is_none());
        assert!(snapshot.limit_monthly_reset_at.is_none());
    }

    #[test]
    fn parse_usage_snapshot_free_without_credit_percent() {
        let credits = json!({
            "config": {
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "start": "2026-07-22T00:00:00+00:00",
                    "end": "2026-07-29T00:00:00+00:00"
                },
                "onDemandCap": { "val": 0 },
                "onDemandUsed": { "val": 0 },
                "isUnifiedBillingUser": true,
                "billingPeriodStart": "2026-07-22T00:00:00+00:00",
                "billingPeriodEnd": "2026-07-29T00:00:00+00:00"
            }
        });
        let monthly = json!({
            "config": {
                "monthlyLimit": { "val": 0 },
                "used": { "val": 0 },
                "billingPeriodStart": "2026-07-01T00:00:00+00:00",
                "billingPeriodEnd": "2026-08-01T00:00:00+00:00"
            }
        });
        let snapshot = parse_grok_usage_snapshot(&credits, Some(&monthly), None, "not-a-jwt");
        assert_eq!(snapshot.plan_type.as_deref(), Some("free"));
        assert!(snapshot.limit_weekly_text.is_none());
        assert_eq!(
            snapshot.limit_weekly_reset_at,
            parse_rfc3339_timestamp("2026-07-29T00:00:00+00:00")
        );
        // monthlyLimit=0 → no monthly quota UI, even if calendar billing period exists
        assert!(snapshot.limit_monthly_text.is_none());
        assert!(snapshot.limit_monthly_reset_at.is_none());
    }

    #[test]
    fn parse_usage_snapshot_monthly_remaining() {
        let credits = json!({
            "config": {
                "creditUsagePercent": 0.0,
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "start": "2026-07-22T00:00:00Z",
                    "end": "2026-07-29T00:00:00Z"
                }
            }
        });
        let monthly = json!({
            "config": {
                "monthlyLimit": { "val": 100 },
                "used": { "val": 25 },
                "billingPeriodEnd": "2026-08-01T00:00:00Z"
            }
        });
        let snapshot =
            parse_grok_usage_snapshot(&credits, Some(&monthly), Some("supergrok"), "not-a-jwt");
        assert_eq!(snapshot.limit_monthly_text.as_deref(), Some("75%"));
        assert_eq!(
            snapshot.limit_monthly_reset_at,
            parse_rfc3339_timestamp("2026-08-01T00:00:00Z")
        );
        assert_eq!(
            snapshot.limit_weekly_reset_at,
            parse_rfc3339_timestamp("2026-07-29T00:00:00Z")
        );
    }

    #[test]
    fn subscription_tier_from_jwt_number() {
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"tier":5}"#);
        let token = format!("header.{claims}.signature");
        assert_eq!(
            subscription_tier_from_jwt(&token).as_deref(),
            Some("supergrok_heavy")
        );
    }

    #[test]
    fn format_xai_token_refresh_error_surfaces_invalid_grant() {
        let token = TokenResponse {
            access_token: None,
            refresh_token: None,
            expires_in: None,
            error: Some("invalid_grant".to_string()),
            error_description: Some("Refresh token has been revoked".to_string()),
        };
        let message = format_xai_token_refresh_error(400, r#"{"error":"invalid_grant"}"#, &token);
        assert!(message.contains("invalid_grant"));
        assert!(message.contains("Refresh token has been revoked"));
        assert!(message.contains("Re-login"));
    }

    #[test]
    fn format_xai_token_refresh_error_maps_bare_http_400() {
        let token = TokenResponse {
            access_token: None,
            refresh_token: None,
            expires_in: None,
            error: None,
            error_description: None,
        };
        let message = format_xai_token_refresh_error(400, "", &token);
        assert!(message.contains("HTTP 400"));
        assert!(message.contains("Re-login"));
    }

    #[test]
    fn grok_account_needs_refresh_respects_lead_window() {
        let scope_key = auth_scope_key(XAI_ISSUER, XAI_CLIENT_ID);
        let far = chrono::Utc::now().timestamp() + 2 * 60 * 60;
        let near = chrono::Utc::now().timestamp() + 10 * 60;
        let past = chrono::Utc::now().timestamp() - 60;
        let make = |expires: Option<i64>| {
            let mut entry = json!({
                "key": "token",
                "refresh_token": "r",
            });
            if let Some(expires_at) = expires {
                entry
                    .as_object_mut()
                    .expect("object")
                    .insert("expires_at".to_string(), json!(expires_at));
            }
            single_account_snapshot(scope_key.clone(), entry)
        };
        assert!(!grok_account_needs_refresh(&make(Some(far))));
        assert!(grok_account_needs_refresh(&make(Some(near))));
        assert!(grok_account_needs_refresh(&make(Some(past))));
        assert!(grok_account_needs_refresh(&make(None)));
    }

    #[test]
    fn refresh_merge_preserves_official_schema_and_unknown_fields() {
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            br#"{"sub":"user-1","client_id":"b1a00492-073a-47ea-816f-4c329264a828","principal_type":"User","principal_id":"principal-1","team_id":"team-1"}"#,
        );
        let access_token = format!("header.{claims}.signature");
        let scope_key = auth_scope_key(XAI_ISSUER, XAI_CLIENT_ID);
        let previous = single_account_snapshot(
            scope_key.clone(),
            json!({
                "key": "old-access",
                "auth_mode": "oidc",
                "create_time": "2026-01-01T00:00:00Z",
                "refresh_token": "old-refresh",
                "runtime_owned": { "keep": true },
                "oidc_issuer": XAI_ISSUER,
                "oidc_client_id": XAI_CLIENT_ID
            }),
        );
        let snapshot = build_xai_auth_snapshot_from_userinfo(
            &TokenResponse {
                access_token: Some(access_token.clone()),
                refresh_token: None,
                expires_in: Some(3600),
                error: None,
                error_description: None,
            },
            XAI_ISSUER,
            UserInfoResponse {
                sub: Some("user-1".to_string()),
                email: Some("user@example.com".to_string()),
                given_name: Some("User".to_string()),
                picture: Some("https://example.com/avatar".to_string()),
            },
            Some(&previous),
        )
        .expect("build snapshot");
        let entry = &snapshot[&scope_key];
        assert_eq!(entry["key"], access_token);
        assert_eq!(entry["refresh_token"], "old-refresh");
        assert_eq!(entry["auth_mode"], "oidc");
        assert_eq!(entry["create_time"], "2026-01-01T00:00:00Z");
        assert_eq!(entry["principal_id"], "principal-1");
        assert_eq!(entry["runtime_owned"]["keep"], true);
        assert!(entry["expires_at"].as_str().is_some());
        assert!(entry.get("access_token").is_none());
        assert!(entry.get("id_token").is_none());
    }

    #[test]
    fn runtime_merge_preserves_other_scopes_and_same_account_enrichment() {
        let scope_key = auth_scope_key(XAI_ISSUER, XAI_CLIENT_ID);
        let runtime = json!({
            "other-scope": { "key": "keep" },
            scope_key.clone(): {
                "principal_id": "principal-1",
                "team_name": "Runtime Team",
                "key": "old"
            }
        });
        let saved = single_account_snapshot(
            scope_key.clone(),
            json!({ "principal_id": "principal-1", "key": "new" }),
        );
        let merged = merge_account_snapshot_into_runtime(runtime, &saved).expect("merge runtime");
        assert_eq!(merged["other-scope"]["key"], "keep");
        assert_eq!(merged[&scope_key]["key"], "new");
        assert_eq!(merged[&scope_key]["team_name"], "Runtime Team");
    }

    #[test]
    fn logout_scope_removal_preserves_other_auth_entries() {
        let temp = tempfile::tempdir().expect("temp dir");
        let auth_path = temp.path().join("auth.json");
        let scope_key = auth_scope_key(XAI_ISSUER, XAI_CLIENT_ID);
        write_auth_json(
            &auth_path,
            &json!({
                scope_key.clone(): { "key": "remove" },
                "other-scope": { "key": "keep" }
            }),
        )
        .expect("write auth");

        remove_auth_scopes(&auth_path, std::slice::from_ref(&scope_key)).expect("remove xAI scope");
        let remaining = read_auth_json_or_empty(&auth_path).expect("read remaining auth");
        assert!(remaining.get(&scope_key).is_none());
        assert_eq!(remaining["other-scope"]["key"], "keep");

        remove_auth_scopes(&auth_path, &["other-scope".to_string()]).expect("remove last scope");
        assert!(!auth_path.exists());
    }
}
