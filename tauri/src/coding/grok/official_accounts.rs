use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use chrono::Local;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{Emitter, Manager};
use tempfile::NamedTempFile;
use tokio::sync::watch;

use super::adapter;
use super::commands::get_grok_auth_path_async;
use super::types::GrokOfficialAccount;
use crate::coding::db_id::{db_extract_id, db_new_id};
use crate::db::helpers::{db_delete, db_get, db_list, db_put, db_update_applied_status};
use crate::db::schema::{DbTable, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use crate::http_client;

const XAI_DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";
const XAI_ISSUER: &str = "https://auth.x.ai";
const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const XAI_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access conversations:read conversations:write";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

static AUTH_SESSIONS: LazyLock<Mutex<HashMap<String, watch::Sender<bool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static AUTH_SESSION_STATUSES: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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

#[derive(Debug, Deserialize)]
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
        .with_conn(|conn| db_list(conn, DbTable::GrokOfficialAccount, Some(&order)))
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
        db_update_applied_status(conn, DbTable::GrokOfficialAccount, Some(&account_id), &now)
    })?;
    let _ = app.emit("config-changed", "window");
    emit_grok_sync(&app);
    Ok(())
}

#[tauri::command]
pub async fn refresh_grok_official_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    account_id: String,
) -> Result<GrokOfficialAccount, String> {
    let account = get_account(state.db(), &account_id)?
        .ok_or_else(|| format!("Grok official account '{account_id}' not found"))?;
    let snapshot_text = account
        .auth_snapshot
        .clone()
        .ok_or_else(|| "Grok official account snapshot is unavailable".to_string())?;
    let mut snapshot: Value = serde_json::from_str(&snapshot_text)
        .map_err(|error| format!("Invalid Grok account snapshot: {error}"))?;
    let (_, entry) = find_xai_auth_entry(&snapshot)?;
    let refresh_token = entry
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Grok account does not contain a refresh token".to_string())?
        .to_string();
    let token_endpoint = account
        .token_endpoint
        .clone()
        .or_else(|| {
            entry
                .get("oidc_issuer")
                .and_then(Value::as_str)
                .map(|issuer| format!("{}/oauth2/token", issuer.trim_end_matches('/')))
        })
        .ok_or_else(|| "Grok account does not contain a token endpoint".to_string())?;
    let token_endpoint = validate_xai_endpoint(&token_endpoint, "token_endpoint")?;
    let client = http_client::client_with_timeout(state.db(), 30).await?;
    let response: TokenResponse = client
        .post(token_endpoint.clone())
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", XAI_CLIENT_ID),
            ("refresh_token", refresh_token.as_str()),
        ])
        .send()
        .await
        .map_err(|error| format!("xAI token refresh failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("xAI token refresh failed: {error}"))?
        .json()
        .await
        .map_err(|error| format!("Failed to parse xAI token refresh: {error}"))?;
    let issuer = entry
        .get("oidc_issuer")
        .and_then(Value::as_str)
        .unwrap_or(XAI_ISSUER)
        .to_string();
    let userinfo_endpoint = format!("{}/oauth2/userinfo", issuer.trim_end_matches('/'));
    snapshot = build_xai_auth_snapshot(
        &client,
        &response,
        &issuer,
        &userinfo_endpoint,
        Some(&snapshot),
    )
    .await?;
    let snapshot_text = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("Failed to serialize Grok auth snapshot: {error}"))?;
    let (email, subject) = identity_from_snapshot(&snapshot);
    let updated = save_account_with_id(
        state.db(),
        &account_id,
        &account.provider_id,
        account.name,
        email.or(account.email),
        subject.or(account.subject),
        snapshot_text,
        Some(token_endpoint),
        account.is_applied,
        account.sort_index,
        account.created_at,
    )?;
    if updated.is_applied {
        let auth_path = get_grok_auth_path_async(state.db()).await?;
        let runtime = read_auth_json_or_empty(&auth_path)?;
        let merged = merge_account_snapshot_into_runtime(runtime, &snapshot)?;
        write_auth_json(&auth_path, &merged)?;
        emit_grok_sync(&app);
    }
    Ok(updated)
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
        .with_conn(|conn| db_delete(conn, DbTable::GrokOfficialAccount, &account_id).map(|_| ()))?;
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
            let _ =
                crate::coding::wsl::remove_auto_synced_wsl_mapping_target(state.inner(), "grok-auth")
                    .await;
        }
    }
    let now = Local::now().to_rfc3339();
    state.db().with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::GrokOfficialAccount, None, &now)
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
                    let account = save_account(
                        db_state.db(),
                        &provider_id,
                        email.clone().unwrap_or_else(|| "xAI".to_string()),
                        email,
                        subject,
                        snapshot_text,
                        Some(token_endpoint.clone()),
                        true,
                    )?;
                    write_auth_json(&get_grok_auth_path_async(db_state.db()).await?, &snapshot)?;
                    let now = Local::now().to_rfc3339();
                    db_state.db().with_conn_mut(|conn| {
                        db_update_applied_status(
                            conn,
                            DbTable::GrokOfficialAccount,
                            Some(&account.id),
                            &now,
                        )
                    })?;
                    emit_grok_sync(&app);
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
            DbTable::GrokOfficialAccount,
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
    let data = json!({
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
    db.with_conn(|conn| db_put(conn, DbTable::GrokOfficialAccount, id, &data))?;
    get_account(db, id)?.ok_or_else(|| "Failed to read saved Grok account".to_string())
}

fn get_account(db: &SqliteDbState, id: &str) -> Result<Option<GrokOfficialAccount>, String> {
    db.with_conn(|conn| db_get(conn, DbTable::GrokOfficialAccount, id))
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

fn ensure_official_provider(db: &SqliteDbState, provider_id: &str) -> Result<(), String> {
    let provider = db
        .with_conn(|conn| db_get(conn, DbTable::GrokProvider, provider_id))?
        .map(adapter::provider_from_db_value)
        .ok_or_else(|| format!("Grok provider '{provider_id}' not found"))?;
    if provider.category != "official" {
        return Err("Grok official accounts require an official provider".to_string());
    }
    Ok(())
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
