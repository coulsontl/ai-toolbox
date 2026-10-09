use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::Local;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Emitter;
use tempfile::NamedTempFile;
use tokio::sync::{watch, Mutex as AsyncMutex};

use super::adapter;
use super::commands::{
    emit_kimi_sync, get_kimi_root_dir_from_db_async, list_kimi_providers_for_db,
};
use super::constants::KIMI_CREDENTIALS_DIR;
use crate::coding::local_bridge::LOCAL_CONFIG_ID;
use super::types::{KimiOfficialAccount, KimiProvider};
use crate::coding::db_id::{db_extract_id, db_new_id};
use crate::db::helpers::{
    db_delete, db_get, db_list, db_patch_fields, db_put, db_update_applied_status,
};
use crate::db::schema::{DbTable, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use crate::http_client;

/// Kimi OAuth host. Overridable via KIMI_CODE_OAUTH_HOST / KIMI_OAUTH_HOST.
pub fn kimi_oauth_host() -> String {
    std::env::var("KIMI_CODE_OAUTH_HOST")
        .ok()
        .or_else(|| std::env::var("KIMI_OAUTH_HOST").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://auth.kimi.com".to_string())
}

/// Client id registered for the official Kimi CLI (matches kimi_cli oauth.py).
const KIMI_OAUTH_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const KIMI_OAUTH_SCOPE: &str = "kimi-code";
/// auth.kimi.com exposes no OIDC discovery document (`.well-known` returns
/// 404); the real Kimi CLI hardcodes these paths on the OAuth host.
const KIMI_OAUTH_DEVICE_AUTH_PATH: &str = "/api/oauth/device_authorization";
const KIMI_OAUTH_TOKEN_PATH: &str = "/api/oauth/token";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const REFRESH_GRANT_TYPE: &str = "refresh_token";
/// Access tokens last ~15 minutes; refresh when remaining lifetime is within
/// this lead. Paired with the 10-minute Kimi patrol interval (auth_refresh),
/// interval passes fire ~5 minutes before expiry instead of after it.
const KIMI_AUTH_REFRESH_LEAD_SECONDS: i64 = 5 * 60;
/// The real Kimi CLI locates its credential file via the fixed storage key
/// `oauth/kimi-code` -> `credentials/kimi-code.json`; any other file name
/// would leave the CLI unable to see the login.
const KIMI_OFFICIAL_CREDENTIAL_NAME: &str = "kimi-code";

static AUTH_SESSIONS: LazyLock<Mutex<HashMap<String, watch::Sender<bool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static AUTH_SESSION_STATUSES: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static OAUTH_REFRESH_LOCK: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiDeviceAuthStartResult {
    pub session_id: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub user_code: String,
    pub expires_at: i64,
    pub poll_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct KimiAuthStatusEvent {
    session_id: String,
    status: String,
    message: Option<String>,
    account_id: Option<String>,
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

fn load_official_provider(db: &SqliteDbState, provider_id: &str) -> Result<KimiProvider, String> {
    let provider = db
        .with_conn(|conn| db_get(conn, DbTable::KimiProvider, provider_id))
        .map(|value| value.map(adapter::provider_from_db_value))?;
    match provider {
        Some(provider) if provider.category == "official" => Ok(provider),
        Some(_) => Err("Kimi provider is not an official provider".to_string()),
        None => Err(format!("Kimi provider '{provider_id}' not found")),
    }
}

/// Discovery endpoints are persisted and later receive refresh tokens, so they
/// must resolve to the configured OAuth host over HTTPS (same rule as grok's
/// `validate_xai_endpoint`).
fn validate_kimi_oauth_endpoint(raw: &str, field: &str) -> Result<String, String> {
    let url = Url::parse(raw).map_err(|error| format!("Invalid Kimi {field}: {error}"))?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let allowed_host = Url::parse(&kimi_oauth_host())
        .ok()
        .and_then(|base| base.host_str().map(|host| host.to_ascii_lowercase()))
        .unwrap_or_default();
    if url.scheme() != "https" || host.is_empty() || host != allowed_host {
        return Err(format!("Kimi {field} must use HTTPS on {allowed_host}"));
    }
    Ok(url.to_string())
}

/// Start a device-code login. Writes nothing until it succeeds.
///
/// It used to take a `provider_id` and refuse to start without a persisted
/// official provider row, which is why the caller had to create that row *first*
/// — and why cancelling left an empty shell in the provider list forever. The
/// row is now resolved at the moment the token exchange succeeds (see
/// `ensure_kimi_official_provider`), so an abandoned login leaves no trace.
#[tauri::command]
pub async fn start_kimi_official_account_device_auth(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<KimiDeviceAuthStartResult, String> {
    {
        let mut sessions = AUTH_SESSIONS
            .lock()
            .map_err(|_| "Kimi auth session lock is poisoned".to_string())?;
        sessions.retain(|_, sender| !sender.is_closed());
        if !sessions.is_empty() {
            return Err("A Kimi device authorization session is already active".to_string());
        }
        // Single-session limit: statuses of finished sessions are stale; drop
        // them so the map cannot grow unbounded across logins.
        if let Ok(mut statuses) = AUTH_SESSION_STATUSES.lock() {
            statuses.clear();
        }
    }
    let host = kimi_oauth_host();
    let client = http_client::client_with_timeout(state.db(), 30).await?;
    // The OAuth host publishes no discovery document; endpoints are constants
    // (same as the official CLI). Validation still guards env-overridden hosts.
    let device_endpoint = validate_kimi_oauth_endpoint(
        &format!("{host}{KIMI_OAUTH_DEVICE_AUTH_PATH}"),
        "device_authorization_endpoint",
    )?;
    let token_endpoint =
        validate_kimi_oauth_endpoint(&format!("{host}{KIMI_OAUTH_TOKEN_PATH}"), "token_endpoint")?;
    let device: DeviceCodeResponse = client
        .post(&device_endpoint)
        .form(&[
            ("client_id", KIMI_OAUTH_CLIENT_ID),
            ("scope", KIMI_OAUTH_SCOPE),
        ])
        .send()
        .await
        .map_err(|error| format!("Kimi device code request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Kimi device code request failed: {error}"))?
        .json()
        .await
        .map_err(|error| format!("Failed to parse Kimi device code response: {error}"))?;
    let session_id = db_new_id();
    let (sender, receiver) = watch::channel(false);
    {
        // Re-check under the lock: another start may have raced past the cheap
        // empty check above while discovery + device-code requests were in
        // flight (TOCTOU).
        let mut sessions = AUTH_SESSIONS
            .lock()
            .map_err(|_| "Kimi auth session lock is poisoned".to_string())?;
        sessions.retain(|_, sender| !sender.is_closed());
        if !sessions.is_empty() {
            return Err("A Kimi device authorization session is already active".to_string());
        }
        sessions.insert(session_id.clone(), sender);
    }
    AUTH_SESSION_STATUSES
        .lock()
        .map_err(|_| "Kimi auth session status lock is poisoned".to_string())?
        .insert(session_id.clone(), "waiting".to_string());

    let app_clone = app.clone();
    let state_db = state.db().clone();
    let session_id_clone = session_id.clone();
    let device_code = device.device_code.clone();
    let token_endpoint_clone = token_endpoint.clone();
    let poll_interval = device.interval.unwrap_or(5).max(5);
    tauri::async_runtime::spawn(async move {
        let deadline = SystemTime::now() + Duration::from_secs(device.expires_in.max(60) as u64);
        let mut current_interval = poll_interval;
        loop {
            let should_stop = *receiver.borrow();
            if should_stop {
                cleanup_auth_session(&session_id_clone);
                return;
            }
            if SystemTime::now() >= deadline {
                set_auth_session_status(&session_id_clone, "expired");
                cleanup_auth_session(&session_id_clone);
                let _ = app_clone.emit(
                    "kimi-auth-status",
                    KimiAuthStatusEvent {
                        session_id: session_id_clone.clone(),
                        status: "expired".to_string(),
                        message: Some("Device code expired".to_string()),
                        account_id: None,
                    },
                );
                return;
            }
            let client = match http_client::client_with_timeout(&state_db, 30).await {
                Ok(client) => client,
                Err(err) => {
                    log::warn!(
                        "[kimi-oauth] Failed to build HTTP client during token poll: {}",
                        err
                    );
                    tokio::time::sleep(Duration::from_secs(current_interval)).await;
                    continue;
                }
            };
            let response = client
                .post(&token_endpoint_clone)
                .form(&[
                    ("grant_type", DEVICE_GRANT_TYPE),
                    ("client_id", KIMI_OAUTH_CLIENT_ID),
                    ("device_code", &device_code),
                ])
                .send()
                .await;
            match response {
                Ok(response) => {
                    let status = response.status();
                    let body: TokenResponse = match response.json().await {
                        Ok(body) => body,
                        Err(err) => {
                            log::debug!(
                                "[kimi-oauth] Failed to parse device token JSON response: {}",
                                err
                            );
                            tokio::time::sleep(Duration::from_secs(current_interval)).await;
                            continue;
                        }
                    };
                    if let Some(access_token) = body.access_token {
                        cleanup_auth_session(&session_id_clone);
                        if let Err(error) = store_official_account(
                            &state_db,
                            &app_clone,
                            &session_id_clone,
                            &access_token,
                            body.refresh_token.as_deref(),
                            body.expires_in,
                            Some(&token_endpoint_clone),
                        )
                        .await
                        {
                            // A silent failure here would leave the UI polling
                            // "waiting" forever and the fresh tokens lost.
                            log::error!(
                                "[kimi-oauth] Failed to store Kimi official account: {error}"
                            );
                            set_auth_session_status(&session_id_clone, "failed");
                            let _ = app_clone.emit(
                                "kimi-auth-status",
                                KimiAuthStatusEvent {
                                    session_id: session_id_clone.clone(),
                                    status: "failed".to_string(),
                                    message: Some(error),
                                    account_id: None,
                                },
                            );
                        }
                        return;
                    }
                    if let Some(error) = body.error.as_deref() {
                        if error == "slow_down" {
                            current_interval = current_interval.saturating_add(5);
                            tokio::time::sleep(Duration::from_secs(current_interval)).await;
                            continue;
                        }
                        if error == "authorization_pending" {
                            tokio::time::sleep(Duration::from_secs(current_interval)).await;
                            continue;
                        }
                        if error == "access_denied" || error == "expired_token" {
                            set_auth_session_status(&session_id_clone, "failed");
                            cleanup_auth_session(&session_id_clone);
                            let _ = app_clone.emit(
                                "kimi-auth-status",
                                KimiAuthStatusEvent {
                                    session_id: session_id_clone.clone(),
                                    status: "failed".to_string(),
                                    message: body.error_description.clone(),
                                    account_id: None,
                                },
                            );
                            return;
                        }
                    }
                    if status.is_client_error() || status.is_server_error() {
                        log::debug!(
                            "[kimi-oauth] Received HTTP status {} during polling",
                            status
                        );
                        tokio::time::sleep(Duration::from_secs(current_interval)).await;
                        continue;
                    }
                    tokio::time::sleep(Duration::from_secs(current_interval)).await;
                }
                Err(err) => {
                    log::debug!(
                        "[kimi-oauth] Network error during device token poll: {}",
                        err
                    );
                    tokio::time::sleep(Duration::from_secs(current_interval)).await;
                }
            }
        }
    });

    Ok(KimiDeviceAuthStartResult {
        session_id,
        verification_uri: device.verification_uri,
        verification_uri_complete: device.verification_uri_complete,
        user_code: device.user_code,
        expires_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64 + device.expires_in.max(60))
            .unwrap_or(0),
        poll_interval_seconds: poll_interval,
    })
}

#[tauri::command]
pub fn cancel_kimi_official_account_device_auth(session_id: String) -> Result<(), String> {
    if let Some(sender) = AUTH_SESSIONS
        .lock()
        .map_err(|_| "Kimi auth session lock is poisoned".to_string())?
        .remove(&session_id)
    {
        let _ = sender.send(true);
    }
    AUTH_SESSION_STATUSES
        .lock()
        .map_err(|_| "Kimi auth session status lock is poisoned".to_string())?
        .insert(session_id, "cancelled".to_string());
    Ok(())
}

#[tauri::command]
pub fn get_kimi_official_account_auth_status(session_id: String) -> Result<String, String> {
    AUTH_SESSION_STATUSES
        .lock()
        .map_err(|_| "Kimi auth session status lock is poisoned".to_string())?
        .get(&session_id)
        .cloned()
        .ok_or_else(|| "Kimi auth session not found".to_string())
}

fn cleanup_auth_session(session_id: &str) {
    if let Ok(mut sessions) = AUTH_SESSIONS.lock() {
        sessions.remove(session_id);
    }
}

fn set_auth_session_status(session_id: &str, status: &str) {
    if let Ok(mut statuses) = AUTH_SESSION_STATUSES.lock() {
        statuses.insert(session_id.to_string(), status.to_string());
    }
}

/// The credential values a login can be recognised by.
///
/// Used as the *fallback* fingerprint when a login carries no profile yet —
/// see `fetch_managed_profile`. Two logins are the same account when they name
/// the same `user_id`; tokens decide only when that name is missing.
fn snapshot_token<'a>(snapshot: &'a Value, key: &str) -> Option<&'a str> {
    snapshot
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn same_official_login(left: &Value, right: &Value) -> bool {
    for key in ["refresh_token", "access_token"] {
        if let (Some(left), Some(right)) = (snapshot_token(left, key), snapshot_token(right, key)) {
            return left == right;
        }
    }
    false
}

/// What the managed platform says about the account behind an access token.
///
/// `GET {base}/me` — the endpoint the 2.x CLI reads its own profile from. The
/// nickname is what makes an account list legible, and the user id is a real
/// identity (unlike the tokens, which rotate on every refresh). Never fatal:
/// a missing nickname only means the row keeps the fallback label.
#[derive(Debug, Clone, Default)]
pub struct KimiManagedProfile {
    pub user_id: Option<String>,
    pub nickname: Option<String>,
    pub email: Option<String>,
    /// The plan's display name (`Free`, `Vivace`, …) — Kimi's `plan_type`.
    pub user_level_name: Option<String>,
}

fn profile_from_payload(payload: &Value) -> KimiManagedProfile {
    let field = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    KimiManagedProfile {
        user_id: field("user_id"),
        nickname: field("nickname"),
        email: field("email"),
        user_level_name: field("user_level_name"),
    }
}

pub async fn fetch_managed_profile(
    db: &SqliteDbState,
    access_token: &str,
) -> Result<KimiManagedProfile, String> {
    let client = http_client::client_with_timeout(db, 15).await?;
    let response = client
        .get(format!("{}/me", super::constants::KIMI_OFFICIAL_API_BASE_URL))
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| format!("Kimi profile request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Kimi profile request failed with status {}",
            response.status()
        ));
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|error| format!("Failed to parse Kimi profile response: {error}"))?;
    Ok(profile_from_payload(&payload))
}

/// Fill in a row's profile, and return the identity fields it now has.
///
/// Best-effort by design: the profile endpoint needs a live token, and a row
/// that cannot reach it must still be listed and still be switchable.
async fn resolve_account_profile(
    db: &SqliteDbState,
    snapshot: &Value,
) -> KimiManagedProfile {
    let Some(access_token) = snapshot_token(snapshot, "access_token") else {
        return KimiManagedProfile::default();
    };
    match fetch_managed_profile(db, access_token).await {
        Ok(profile) => profile,
        Err(error) => {
            log::debug!("[kimi-oauth] Could not read the managed profile: {error}");
            KimiManagedProfile::default()
        }
    }
}

/// Find the row a login belongs to.
///
/// Identity first, tokens second. A re-login of the same person mints a new
/// token pair, so a fingerprint alone would append a duplicate row for an
/// account already in the list; the profile's `user_id` survives re-login, and
/// that is what makes "sign in again" an update rather than a new entry. Tokens
/// remain the fallback for a row whose profile could not be read.
fn find_matching_kimi_official_account(
    db: &SqliteDbState,
    provider_id: &str,
    snapshot: &Value,
    profile: &KimiManagedProfile,
) -> Result<Option<KimiOfficialAccount>, String> {
    let accounts = list_kimi_official_accounts_with_state(db)?;
    let owned = || {
        accounts
            .iter()
            .filter(|account| account.provider_id == provider_id)
    };
    if let Some(user_id) = profile.user_id.as_deref() {
        if let Some(matched) = owned().find(|account| account.subject.as_deref() == Some(user_id)) {
            return Ok(Some(matched.clone()));
        }
    }
    Ok(owned()
        .find(|account| {
            read_account_snapshot(account)
                .is_some_and(|stored| same_official_login(&stored, snapshot))
        })
        .cloned())
}

/// Persist a completed login.
///
/// The row is matched by credential fingerprint (see
/// `find_matching_kimi_official_account`), so signing in again with the same
/// login updates its row while a different login appends one — which is what
/// makes several accounts under one official channel possible. `name` is left
/// empty: there is no identity to name the row with, and the section's label is
/// built on the frontend from what it does have (the login time).
async fn store_official_account(
    db: &SqliteDbState,
    app: &tauri::AppHandle,
    session_id: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_in: Option<i64>,
    token_endpoint: Option<&str>,
) -> Result<(), String> {
    let expires_at = expires_in.map(|seconds| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64 + seconds)
            .unwrap_or(0)
    });
    let now = Local::now().to_rfc3339();
    // The row a login hangs off, created on first need. Resolving it *here* —
    // rather than making the caller create one before the device flow starts —
    // is what keeps a cancelled login from leaving a shell behind.
    let provider_id = super::commands::ensure_kimi_official_provider(db)?.id;
    let snapshot = json!({
        "access_token": access_token,
        "refresh_token": refresh_token,
        "token_endpoint": token_endpoint,
        "expires_at": expires_at,
        "expires_in": expires_in,
        "scope": KIMI_OAUTH_SCOPE,
        "token_type": "Bearer",
    });
    // Re-login of the same login refreshes its row instead of appending a
    // duplicate. Matching happens after the profile is read: a re-login of a
    // known account is found by identity, not by the tokens it just replaced.
    // Who this login belongs to. Read while the token is fresh, and *before*
    // the row is written, so the very first login already has a name to show
    // instead of a timestamp — and before matching, so a re-login of a known
    // account updates its row instead of appending a twin.
    let profile = resolve_account_profile(db, &snapshot).await;
    let existing = find_matching_kimi_official_account(db, &provider_id, &snapshot, &profile)?;
    // Keep the previous ordering on re-login; resetting to 0 would silently
    // reorder the account list.
    let sort_index = existing
        .as_ref()
        .and_then(|account| account.sort_index)
        .unwrap_or(0);
    let id = existing
        .as_ref()
        .map(|account| account.id.clone())
        .unwrap_or_else(db_new_id);
    // Re-login must not demote an applied account: the background refresh loop
    // only rewrites the live credential file for applied accounts, so flipping
    // the flag would leave the CLI on a stale token after expiry.
    let is_applied = existing
        .as_ref()
        .map(|account| account.is_applied)
        .unwrap_or(false);
    let created_at = existing
        .map(|account| account.created_at)
        .unwrap_or_else(|| now.clone());
    let content = json!({
        "provider_id": provider_id,
        "name": "",
        "kind": "official",
        "email": profile.email,
        // The account's stable identity — what tells two logins apart in the
        // list, and what a re-login is matched against once tokens rotate.
        "subject": profile.user_id,
        "nickname": profile.nickname,
        "auth_snapshot": serde_json::to_string(&snapshot)
        .unwrap_or_default(),
        "expires_at": expires_at,
        "token_endpoint": token_endpoint,
        "last_refresh": now,
        "last_error": null,
        "plan_type": profile.user_level_name,
        "limit_weekly_text": null,
        "limit_monthly_text": null,
        "limit_weekly_reset_at": null,
        "limit_monthly_reset_at": null,
        "last_limits_fetched_at": null,
        "is_applied": is_applied,
        "sort_index": sort_index,
        "created_at": created_at,
        "updated_at": now,
    });
    db.with_conn(|conn| db_put(conn, DbTable::KimiOfficialAccount, &id, &content))?;
    if is_applied {
        // Push the fresh token into credentials/<name>.json right away so the
        // CLI never keeps running on the pre-login token.
        if let Some(updated) = get_account(db, &id)? {
            write_credential_file(db, &updated).await?;
        }
    }
    set_auth_session_status(session_id, "completed");
    let _ = app.emit(
        "kimi-auth-status",
        KimiAuthStatusEvent {
            session_id: session_id.to_string(),
            status: "completed".to_string(),
            message: None,
            account_id: Some(id.clone()),
        },
    );
    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Read the stored account rows, in display order.
///
/// No virtual row is added here: this is the storage view, used by the write
/// paths. The frontend-facing list is `list_kimi_official_accounts`.
pub fn list_kimi_official_accounts_with_state(
    state: &SqliteDbState,
) -> Result<Vec<KimiOfficialAccount>, String> {
    let order = OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::created_at(OrderDirection::Asc),
    ]);
    state
        .with_conn(|conn| db_list(conn, DbTable::KimiOfficialAccount, Some(&order)))
        .map(|values| {
            values
                .into_iter()
                .map(account_from_db_value)
                .collect::<Vec<_>>()
        })
}

/// The live login as a read-only row, when nothing stored represents it.
///
/// Same idea as Codex's virtual account: the CLI can be signed in with an
/// account this app has never captured, and a list that does not say so is
/// lying about what is in use. The row carries no snapshot — it mirrors the
/// credential file — and its only action is "save", which is what turns it into
/// a stored account.
///
/// It is deliberately **not** synthesized in `list_kimi_official_accounts_with_state`:
/// that function feeds the write paths (adoption, refresh, apply), and a
/// synthetic row leaking into them would look like an account that has no row.
fn build_virtual_local_account(
    snapshot: &Value,
    profile: &KimiManagedProfile,
) -> KimiOfficialAccount {
    KimiOfficialAccount {
        id: LOCAL_CONFIG_ID.to_string(),
        provider_id: String::new(),
        name: String::new(),
        kind: "local".to_string(),
        email: profile.email.clone(),
        subject: profile.user_id.clone(),
        nickname: profile.nickname.clone(),
        auth_snapshot: Some(snapshot.to_string()),
        token_endpoint: snapshot
            .get("token_endpoint")
            .and_then(Value::as_str)
            .map(str::to_string),
        expires_at: snapshot.get("expires_at").and_then(Value::as_i64),
        last_refresh: None,
        last_error: None,
        plan_type: profile.user_level_name.clone(),
        limit_weekly_text: None,
        limit_monthly_text: None,
        limit_weekly_reset_at: None,
        limit_monthly_reset_at: None,
        last_limits_fetched_at: None,
        is_applied: false,
        sort_index: None,
        created_at: String::new(),
        updated_at: String::new(),
        is_virtual: true,
    }
}

async fn read_live_login_snapshot(db: &SqliteDbState) -> Option<Value> {
    let path = get_kimi_root_dir_from_db_async(db)
        .await
        .ok()?
        .join(KIMI_CREDENTIALS_DIR)
        .join(format!("{KIMI_OFFICIAL_CREDENTIAL_NAME}.json"));
    let raw = fs::read_to_string(path).ok()?;
    let snapshot: Value = serde_json::from_str(&raw).ok()?;
    // An empty or missing access token means the file is not a usable login
    // (the CLI writes it before the token exchange completes).
    snapshot
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|token| !token.is_empty())
        .then_some(snapshot)
}

/// The account list the frontend sees: stored rows plus, when the live login is
/// not represented by any of them, one virtual row for it.
pub async fn list_kimi_official_accounts_for_frontend(
    db: &SqliteDbState,
) -> Result<Vec<KimiOfficialAccount>, String> {
    let mut accounts = list_kimi_official_accounts_with_state(db)?;
    let Some(snapshot) = read_live_login_snapshot(db).await else {
        return Ok(accounts);
    };
    let already_represented = accounts
        .iter()
        .filter_map(read_account_snapshot)
        .any(|stored| same_official_login(&stored, &snapshot));
    if !already_represented {
        // A row that has never been saved has no stored profile either, so ask
        // the platform who it is — the list would otherwise show "signed in at
        // 22:20" for a login whose owner has a perfectly good nickname.
        let profile = resolve_account_profile(db, &snapshot).await;
        accounts.insert(0, build_virtual_local_account(&snapshot, &profile));
    }
    Ok(accounts)
}

#[tauri::command]
pub async fn list_kimi_official_accounts(
    state: tauri::State<'_, SqliteDbState>,
) -> Result<Vec<KimiOfficialAccount>, String> {
    list_kimi_official_accounts_for_frontend(state.inner()).await
}

/// Store the live login as a proper account.
///
/// The only action a virtual row offers: it exists because the app has not
/// captured that login yet, and once it is captured the virtual row disappears
/// on the next list (nothing matches it any more) while the stored row carries
/// the switch/delete actions instead.
pub async fn save_kimi_official_local_login(
    db: &SqliteDbState,
) -> Result<KimiOfficialAccount, String> {
    let snapshot = read_live_login_snapshot(db)
        .await
        .ok_or_else(|| "No live Kimi login to save".to_string())?;
    let provider = super::commands::ensure_kimi_official_provider(db)?;
    let profile = resolve_account_profile(db, &snapshot).await;
    if let Some(account) =
        find_matching_kimi_official_account(db, &provider.id, &snapshot, &profile)?
    {
        // Already captured (a second click, or another window got there first):
        // nothing to write.
        return Ok(account);
    }
    let now = Local::now().to_rfc3339();
    let id = db_new_id();
    let content = json!({
        "provider_id": provider.id,
        "name": "",
        "kind": "official",
        "email": profile.email,
        "subject": profile.user_id,
        "nickname": profile.nickname,
        "auth_snapshot": snapshot.to_string(),
        "expires_at": snapshot.get("expires_at").and_then(Value::as_i64),
        "token_endpoint": snapshot
            .get("token_endpoint")
            .and_then(Value::as_str)
            .map(str::to_string),
        "last_refresh": now,
        "last_error": null,
        "plan_type": profile.user_level_name,
        "limit_weekly_text": null,
        "limit_monthly_text": null,
        "limit_weekly_reset_at": null,
        "limit_monthly_reset_at": null,
        "last_limits_fetched_at": null,
        // Saving does not make it the applied account; the live file already is
        // it, and "apply" stays the explicit action for switching.
        "is_applied": false,
        "sort_index": 0,
        "created_at": now,
        "updated_at": now,
    });
    db.with_conn(|conn| db_put(conn, DbTable::KimiOfficialAccount, &id, &content))?;
    get_account(db, &id)?.ok_or_else(|| "Failed to read back the saved Kimi account".to_string())
}

#[tauri::command]
pub async fn save_kimi_official_local_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
) -> Result<KimiOfficialAccount, String> {
    let account = save_kimi_official_local_login(state.db()).await?;
    let _ = app.emit("config-changed", "window");
    Ok(account)
}

fn account_from_db_value(value: Value) -> KimiOfficialAccount {
    KimiOfficialAccount {
        id: db_extract_id(&value),
        provider_id: value
            .get("provider_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        kind: value
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("official")
            .to_string(),
        email: value
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_string),
        subject: value
            .get("subject")
            .and_then(Value::as_str)
            .map(str::to_string),
        nickname: value
            .get("nickname")
            .and_then(Value::as_str)
            .map(str::to_string),
        auth_snapshot: value
            .get("auth_snapshot")
            .and_then(Value::as_str)
            .map(str::to_string),
        token_endpoint: value
            .get("token_endpoint")
            .and_then(Value::as_str)
            .map(str::to_string),
        expires_at: value.get("expires_at").and_then(Value::as_i64),
        last_refresh: value
            .get("last_refresh")
            .and_then(Value::as_str)
            .map(str::to_string),
        last_error: value
            .get("last_error")
            .and_then(Value::as_str)
            .map(str::to_string),
        plan_type: value
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        limit_weekly_text: value
            .get("limit_weekly_text")
            .and_then(Value::as_str)
            .map(str::to_string),
        limit_monthly_text: value
            .get("limit_monthly_text")
            .and_then(Value::as_str)
            .map(str::to_string),
        limit_weekly_reset_at: value.get("limit_weekly_reset_at").and_then(Value::as_i64),
        limit_monthly_reset_at: value.get("limit_monthly_reset_at").and_then(Value::as_i64),
        last_limits_fetched_at: value
            .get("last_limits_fetched_at")
            .and_then(Value::as_str)
            .map(str::to_string),
        is_applied: value
            .get("is_applied")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        sort_index: value
            .get("sort_index")
            .and_then(Value::as_i64)
            .map(|value| value as i32),
        created_at: value
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        updated_at: value
            .get("updated_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        // Only the synthesizer sets this; a stored row is by definition not
        // virtual.
        is_virtual: false,
    }
}

fn get_account(db: &SqliteDbState, id: &str) -> Result<Option<KimiOfficialAccount>, String> {
    db.with_conn(|conn| db_get(conn, DbTable::KimiOfficialAccount, id))
        .map(|value| value.map(account_from_db_value))
}

fn read_account_snapshot(account: &KimiOfficialAccount) -> Option<Value> {
    account
        .auth_snapshot
        .as_deref()
        .and_then(|snapshot| serde_json::from_str(snapshot).ok())
}

/// Write the live OAuth credential file at `credentials/<name>.json` (0600).
///
/// The file name is the fixed storage key the real Kimi CLI resolves its login
/// through — the module comment on `KIMI_OFFICIAL_CREDENTIAL_NAME` has the
/// detail — so it is deliberately *not* taken from the account row. There is one
/// live credential; the account rows are snapshots, and switching writes the
/// chosen snapshot here. Coupling the two is what used to cap Kimi at a single
/// account: every row wanted its own file name, and only one name works.
pub async fn write_credential_file(
    db: &SqliteDbState,
    account: &KimiOfficialAccount,
) -> Result<(), String> {
    let snapshot = read_account_snapshot(account)
        .ok_or_else(|| format!("Kimi account '{}' has no auth snapshot", account.id))?;
    let root_dir = get_kimi_root_dir_from_db_async(db).await?;
    let credentials_dir = root_dir.join(KIMI_CREDENTIALS_DIR);
    fs::create_dir_all(&credentials_dir)
        .map_err(|error| format!("Failed to create {}: {error}", credentials_dir.display()))?;
    let file_path = credentials_dir.join(format!("{KIMI_OFFICIAL_CREDENTIAL_NAME}.json"));
    let temp = NamedTempFile::new_in(&credentials_dir)
        .map_err(|error| format!("Failed to create temp credential file: {error}"))?;
    let mut file = temp;
    file.write_all(snapshot.to_string().as_bytes())
        .map_err(|error| format!("Failed to write credential file: {error}"))?;
    file.flush()
        .map_err(|error| format!("Failed to flush credential file: {error}"))?;
    file.persist(&file_path)
        .map_err(|error| format!("Failed to persist credential file: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&file_path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[tauri::command]
pub async fn apply_kimi_official_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    account_id: String,
) -> Result<(), String> {
    let account = get_account(state.db(), &account_id)?
        .ok_or_else(|| format!("Kimi official account '{account_id}' not found"))?;
    // Ensure the provider row exists and mark it applied so config.toml projection
    // can write the official channel credentials into [providers.<name>].
    let provider_id = account.provider_id.clone();
    let provider = load_official_provider(state.db(), &provider_id)?;
    if provider.is_disabled {
        return Err("Disabled Kimi provider cannot be applied".to_string());
    }
    // Applying an official account re-projects config.toml; that rewrite must
    // not happen while the gateway owns the file.
    super::commands::ensure_kimi_gateway_direct(&app)?;
    write_credential_file(state.db(), &account).await?;
    // Re-project the applied provider BEFORE flipping the applied status: the
    // projection cleans up the previously applied provider's tables by reading
    // `get_applied_provider`, which must still see the OLD provider here (same
    // order as `select_kimi_provider_internal_with_sync`).
    super::commands::apply_kimi_provider_to_file(state.db(), &provider_id).await?;
    let now = Local::now().to_rfc3339();
    state.db().with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::KimiOfficialAccount, Some(&account_id), &now)
    })?;
    state.db().with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::KimiProvider, Some(&provider_id), &now)
    })?;
    let _ = app.emit("config-changed", "window");
    emit_kimi_sync(&app);
    Ok(())
}

#[tauri::command]
pub async fn delete_kimi_official_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    account_id: String,
) -> Result<(), String> {
    let account = get_account(state.db(), &account_id)?
        .ok_or_else(|| format!("Kimi official account '{account_id}' not found"))?;
    // Deleting the applied account would leave its credential projection in
    // config.toml without an owner (same rule as "the applied provider cannot
    // be deleted"); restore direct / apply another account first.
    if account.is_applied {
        return Err("The applied Kimi official account cannot be deleted".to_string());
    }
    // The live credential file is deliberately left alone: it holds the
    // *applied* account's snapshot, and the applied account cannot be deleted —
    // so a row being removed here never owns that file. Deleting it would sign
    // the CLI out of the account still in use.
    state
        .db()
        .with_conn(|conn| db_delete(conn, DbTable::KimiOfficialAccount, &account_id).map(|_| ()))?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// One-time fix-up for logins written before the credential file name was fixed.
///
/// Account rows used to be *named after* their credential file, so a login from
/// an older build sits next to `credentials/<that old name>.json` — a file the
/// real CLI never reads, since it resolves its login through the fixed
/// `kimi-code` key. Rename that file if it is still lying around.
///
/// Rows are no longer renamed: `name` carries a display label now (and is empty
/// when there is no identity to label with), so rewriting it here would fight
/// the label the frontend builds.
pub async fn migrate_legacy_credential_names(db: &SqliteDbState) -> Result<(), String> {
    let legacy_names = list_kimi_official_accounts_with_state(db)?
        .into_iter()
        .map(|account| account.name)
        .filter(|name| !name.is_empty() && name != KIMI_OFFICIAL_CREDENTIAL_NAME)
        .collect::<Vec<_>>();
    if legacy_names.is_empty() {
        return Ok(());
    }
    let credentials_dir = get_kimi_root_dir_from_db_async(db)
        .await?
        .join(KIMI_CREDENTIALS_DIR);
    let target_path = credentials_dir.join(format!("{KIMI_OFFICIAL_CREDENTIAL_NAME}.json"));
    if target_path.exists() {
        return Ok(());
    }
    for name in legacy_names {
        let legacy_path = credentials_dir.join(format!("{name}.json"));
        if !legacy_path.exists() {
            continue;
        }
        // Best-effort: the applied account's next refresh rewrites the file at
        // the fixed name anyway, so a failure here only delays the fix.
        if let Err(error) = fs::rename(&legacy_path, &target_path) {
            log::warn!(
                "[kimi-oauth] Failed to rename legacy credential file {}: {error}",
                legacy_path.display()
            );
        }
        return Ok(());
    }
    Ok(())
}

/// Bootstrap an official setup that only the Kimi CLI knows about.
///
/// A fresh install has no provider rows and no account rows, while the CLI may
/// already be signed in — its credential file is sitting right there. Import
/// that login as the official channel: one provider row marked applied (nothing
/// else is) plus one applied account, so the list shows what is really in use
/// instead of an empty official card. Same shape as Codex's
/// `import_codex_default_provider_from_local_files`.
///
/// Deliberately narrow, because this writes rows the user never confirmed:
///
/// - only on an install with **no provider rows at all** — so it can never
///   invent an official channel under someone who deliberately switched to a
///   relay. A login that shows up later is surfaced by the virtual row instead,
///   which needs no import and saves nothing until asked;
/// - only when the account list is empty (an existing account means this app
///   already owns the state);
/// - only from a credential file that actually parses into an access token;
/// - never fatal: a failure here must not stop the refresh pass, and the file
///   is left exactly as it was.
pub async fn adopt_live_login_as_account(db: &SqliteDbState) -> Result<(), String> {
    if !list_kimi_official_accounts_with_state(db)?.is_empty() {
        return Ok(());
    }
    if !list_kimi_providers_for_db(db)?.is_empty() {
        return Ok(());
    }
    let credential_path = get_kimi_root_dir_from_db_async(db)
        .await?
        .join(KIMI_CREDENTIALS_DIR)
        .join(format!("{KIMI_OFFICIAL_CREDENTIAL_NAME}.json"));
    let raw = match fs::read_to_string(&credential_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Failed to read Kimi credentials: {error}")),
    };
    let snapshot: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("Invalid Kimi credential file: {error}"))?;
    // An empty or missing access token means the file is not a usable login
    // (the CLI writes it before the token exchange completes).
    let has_access_token = snapshot
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|token| !token.is_empty());
    if !has_access_token {
        return Ok(());
    }
    let provider = super::commands::ensure_kimi_official_provider(db)?;
    let profile = resolve_account_profile(db, &snapshot).await;
    let now = Local::now().to_rfc3339();
    // The provider table was empty a moment ago, and the CLI is signed in, so
    // the official channel is what the live config.toml already points at.
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::KimiProvider, Some(&provider.id), &now)
    })?;
    let content = json!({
        "provider_id": provider.id,
        "name": "",
        "kind": "official",
        "email": profile.email,
        "subject": profile.user_id,
        "nickname": profile.nickname,
        // Stored verbatim so `read_account_snapshot` hands the refresh loop
        // the same fields the CLI wrote.
        "auth_snapshot": snapshot.to_string(),
        "expires_at": snapshot.get("expires_at").and_then(Value::as_i64),
        "token_endpoint": snapshot
            .get("token_endpoint")
            .and_then(Value::as_str)
            .map(str::to_string),
        "last_refresh": now,
        "last_error": null,
        "plan_type": profile.user_level_name,
        "limit_weekly_text": null,
        "limit_monthly_text": null,
        "limit_weekly_reset_at": null,
        "limit_monthly_reset_at": null,
        "last_limits_fetched_at": null,
        "is_applied": true,
        "sort_index": 0,
        "created_at": now,
        "updated_at": now,
    });
    db.with_conn(|conn| db_put(conn, DbTable::KimiOfficialAccount, &db_new_id(), &content))?;
    log::info!(
        "[kimi-oauth] Adopted the live CLI login at {} as the official channel",
        credential_path.display()
    );
    Ok(())
}

/// Background refresh: refresh access tokens that are within the lead window.
/// Non-applied accounts only update the SQLite record; the applied account also
/// rewrites the live credential file and emits sync events.
pub async fn refresh_applied_kimi_accounts_if_needed<R: tauri::Runtime>(
    db: &SqliteDbState,
    app: &tauri::AppHandle<R>,
) -> Result<(), String> {
    let state = db;
    let _guard = OAUTH_REFRESH_LOCK.lock().await;
    migrate_legacy_credential_names(state).await?;
    adopt_live_login_as_account(state).await?;
    let accounts = list_kimi_official_accounts_with_state(state)?;
    for account in accounts {
        let Some(expires_at) = account.expires_at else {
            continue;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        if now + KIMI_AUTH_REFRESH_LEAD_SECONDS < expires_at {
            continue;
        }
        let Some(snapshot) = read_account_snapshot(&account) else {
            continue;
        };
        let Some(refresh_token) = snapshot.get("refresh_token").and_then(Value::as_str) else {
            continue;
        };
        // Prefer the endpoint captured at login; records created before the
        // field existed only have the default derivation.
        let token_endpoint = snapshot
            .get("token_endpoint")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| account.token_endpoint.clone())
            .unwrap_or_else(|| format!("{}{KIMI_OAUTH_TOKEN_PATH}", kimi_oauth_host()));
        let token_endpoint = match validate_kimi_oauth_endpoint(&token_endpoint, "token_endpoint") {
            Ok(endpoint) => endpoint,
            Err(error) => {
                // The persisted endpoint no longer matches the configured host;
                // skip this account instead of shipping the refresh token there.
                if let Err(record_error) =
                    record_account_refresh_error(state.db(), &account.id, &error).await
                {
                    log::warn!(
                        "[kimi-oauth] Failed to record refresh error for {}: {record_error}",
                        account.id
                    );
                }
                continue;
            }
        };
        let refresh_result: Result<TokenResponse, String> = async {
            let client = http_client::client_with_timeout(state.db(), 30).await?;
            let response = client
                .post(&token_endpoint)
                .form(&[
                    ("grant_type", REFRESH_GRANT_TYPE),
                    ("client_id", KIMI_OAUTH_CLIENT_ID),
                    ("refresh_token", refresh_token),
                ])
                .send()
                .await
                .map_err(|error| format!("Kimi token refresh request failed: {error}"))?
                .error_for_status()
                .map_err(|error| format!("Kimi token refresh failed: {error}"))?
                .json()
                .await
                .map_err(|error| format!("Failed to parse Kimi token refresh response: {error}"))?;
            Ok(response)
        }
        .await;
        // One account failing must not block refresh of the remaining ones.
        let response = match refresh_result {
            Ok(response) => response,
            Err(error) => {
                // Recording the failure is best-effort: a DB hiccup must not
                // abort the refresh pass for the remaining accounts.
                if let Err(record_error) =
                    record_account_refresh_error(state.db(), &account.id, &error).await
                {
                    log::warn!(
                        "[kimi-oauth] Failed to record refresh error for {}: {record_error}",
                        account.id
                    );
                }
                continue;
            }
        };
        let Some(access_token) = response.access_token else {
            let reason = response
                .error_description
                .or(response.error)
                .unwrap_or_else(|| "Kimi token refresh returned no access token".to_string());
            if let Err(record_error) =
                record_account_refresh_error(state.db(), &account.id, &reason).await
            {
                log::warn!(
                    "[kimi-oauth] Failed to record refresh error for {}: {record_error}",
                    account.id
                );
            }
            continue;
        };
        let expires_at_new = response.expires_in.map(|seconds| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64 + seconds)
                .unwrap_or(0)
        });
        let new_snapshot = json!({
            "access_token": access_token,
            "refresh_token": response.refresh_token.as_deref().unwrap_or(refresh_token),
            "token_endpoint": token_endpoint,
            "expires_at": expires_at_new,
            "expires_in": response.expires_in,
            "scope": KIMI_OAUTH_SCOPE,
            "token_type": "Bearer",
        });
        let now = Local::now().to_rfc3339();
        state.db().with_conn(|conn| {
            db_patch_fields(
                conn,
                DbTable::KimiOfficialAccount,
                &account.id,
                &[
                    ("auth_snapshot", json!(new_snapshot.to_string())),
                    ("expires_at", json!(expires_at_new)),
                    ("last_refresh", Value::String(now.clone())),
                    ("last_error", Value::Null),
                    ("updated_at", Value::String(now)),
                ],
            )
            .map(|_| ())
        })?;
        if account.is_applied {
            // The refreshed OAuth access token lives only in
            // credentials/<name>.json (rewritten below); config.toml's
            // [providers].api_key is the provider row's static key, not the
            // OAuth token, so no config.toml re-projection is needed here.
            if let Some(updated) = get_account(state.db(), &account.id)? {
                write_credential_file(state.db(), &updated).await?;
            }
            let _ = app.emit("config-changed", "window");
            emit_kimi_sync(app);
        }
    }
    Ok(())
}

/// Persist a refresh failure on the account row so it stays observable.
async fn record_account_refresh_error(
    db: &SqliteDbState,
    account_id: &str,
    message: &str,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    db.with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::KimiOfficialAccount,
            account_id,
            &[
                ("last_error", Value::String(message.to_string())),
                ("updated_at", Value::String(now)),
            ],
        )
        .map(|_| ())
    })
}

pub async fn clear_all_kimi_official_account_apply_status(
    db: &SqliteDbState,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::KimiOfficialAccount, None, &now)
    })?;
    Ok(())
}

pub async fn sync_kimi_official_account_apply_status(
    db: &SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        let accounts = db_list(conn, DbTable::KimiOfficialAccount, None)?;
        for account in accounts {
            let account_id = db_extract_id(&account);
            let owns_provider =
                account.get("provider_id").and_then(Value::as_str) == Some(provider_id);
            db_patch_fields(
                conn,
                DbTable::KimiOfficialAccount,
                &account_id,
                &[
                    ("is_applied", Value::Bool(owns_provider)),
                    ("updated_at", Value::String(now.clone())),
                ],
            )
            .map(|_| ())?;
        }
        Ok(())
    })
}
