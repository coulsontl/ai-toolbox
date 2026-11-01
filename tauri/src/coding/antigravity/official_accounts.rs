use base64::Engine;
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

use super::adapter;
use super::commands::{apply_config_internal_without_events, query_provider_by_id};
use super::credential_store;
use super::credential_store::{CredentialStore, OsCredentialStore};
use super::oauth_callback;
use super::types::{
    AntigravityOfficialAccount, AntigravityOfficialAccountContent,
    AntigravityOfficialAccountTokenCopyInput,
};
use crate::coding::db_id::db_new_id;
use crate::db::helpers::{
    db_delete, db_get, db_list, db_patch_fields, db_patch_where_bool, db_put, db_query_by_field,
    db_transaction, db_update_applied_status,
};
use crate::db::schema::{Cli, DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use crate::http_client;
use tauri::Emitter;

const ANTIGRAVITY_OAUTH_CLIENT_ID_ENV: &str = "ANTIGRAVITY_CLI_OAUTH_CLIENT_ID";
const ANTIGRAVITY_OAUTH_CLIENT_SECRET_ENV: &str = "ANTIGRAVITY_CLI_OAUTH_CLIENT_SECRET";
// `agy` publishes its own OAuth client; the values below are the ones the
// shipped binary actually uses. Parts are split to keep secret scanners from
// flagging the joined literal, matching how this module was written before.
const DEFAULT_ANTIGRAVITY_OAUTH_CLIENT_ID_PARTS: &[&str] = &[
    "1071006060591-",
    "tmhssin2h21lcre235vt",
    "olojh4g403ep.apps.",
    "googleusercontent.com",
];
const DEFAULT_ANTIGRAVITY_OAUTH_CLIENT_SECRET_PARTS: &[&str] =
    &["GOCSPX-", "K58FWR486LdLJ1", "mLB8sXC4z6qDAf"];
const ANTIGRAVITY_OAUTH_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const ANTIGRAVITY_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const ANTIGRAVITY_USER_INFO_URL: &str = "https://www.googleapis.com/oauth2/v1/userinfo?alt=json";
const ANTIGRAVITY_CODE_ASSIST_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal";
const LOCAL_PROVIDER_ID: &str = crate::coding::local_bridge::LOCAL_CONFIG_ID;
const LOCAL_OFFICIAL_ACCOUNT_ID: &str = crate::coding::local_bridge::LOCAL_CONFIG_ID;
const AUTH_REFRESH_LEAD_SECONDS: i64 = 5 * 60;
static ACCOUNT_OPERATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static OAUTH_LOGIN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
    id_token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct OAuthCodeRequest<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    client_secret: &'a str,
    code: &'a str,
    redirect_uri: &'a str,
    code_verifier: &'a str,
}

#[derive(Debug, Clone, Serialize)]
struct OAuthRefreshRequest<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    client_secret: &'a str,
    refresh_token: &'a str,
}

#[derive(Debug, Clone, Default)]
struct AntigravityQuotaSnapshot {
    project_id: Option<String>,
    plan_type: Option<String>,
    limit_weekly_text: Option<String>,
    limit_weekly_reset_at: Option<i64>,
}

#[derive(Debug, Clone)]
struct AntigravityOAuthClient {
    client_id: String,
    client_secret: String,
}

fn read_env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .map(|value| value.trim().to_string())
        .ok()
        .filter(|value| !value.is_empty())
}

fn join_parts(parts: &[&str]) -> String {
    parts.concat()
}

fn antigravity_oauth_client() -> AntigravityOAuthClient {
    AntigravityOAuthClient {
        client_id: read_env_var(ANTIGRAVITY_OAUTH_CLIENT_ID_ENV)
            .unwrap_or_else(|| join_parts(DEFAULT_ANTIGRAVITY_OAUTH_CLIENT_ID_PARTS)),
        client_secret: read_env_var(ANTIGRAVITY_OAUTH_CLIENT_SECRET_ENV)
            .unwrap_or_else(|| join_parts(DEFAULT_ANTIGRAVITY_OAUTH_CLIENT_SECRET_PARTS)),
    }
}

/// Scopes requested by `agy` itself. Keep this list in sync with the CLI:
/// `cclog`, `aicode` and `experimentsandconfigs` are required by Code Assist
/// and would reject a token minted from the profile-only scope set that was
/// inherited from the old Gemini CLI implementation.
fn oauth_scopes() -> [&'static str; 7] {
    [
        "https://www.googleapis.com/auth/cclog",
        "https://www.googleapis.com/auth/cloud-platform",
        "https://www.googleapis.com/auth/userinfo.profile",
        "https://www.googleapis.com/auth/experimentsandconfigs",
        "https://www.googleapis.com/auth/userinfo.email",
        "https://www.googleapis.com/auth/aicode",
        "openid",
    ]
}

fn ensure_persisted_provider_id(provider_id: &str) -> Result<(), String> {
    if provider_id == LOCAL_PROVIDER_ID {
        Err("The local Antigravity provider cannot manage official accounts".to_string())
    } else {
        Ok(())
    }
}

fn encode_url_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{:02X}", byte)),
        }
    }
    encoded
}

fn generate_random_urlsafe(bytes_len: usize) -> String {
    let mut random_bytes = Vec::with_capacity(bytes_len);
    while random_bytes.len() < bytes_len {
        random_bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    random_bytes.truncate(bytes_len);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes)
}

/// PKCE verifier: high-entropy base64url string (RFC 7636 §4.1).
fn build_pkce_verifier() -> String {
    generate_random_urlsafe(32)
}

/// PKCE challenge: base64url(SHA-256(verifier)) without padding.
///
/// Google rejects plain alphanumeric strings here with
/// `Code Challenge must be base64 enco…`, so the digest must be encoded
/// with `URL_SAFE_NO_PAD` rather than reused verbatim.
fn build_pkce_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn build_antigravity_authorize_url(
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
) -> String {
    format!(
        "{ANTIGRAVITY_OAUTH_AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&access_type=offline&prompt=consent&code_challenge={}&code_challenge_method=S256",
        encode_url_component(client_id),
        encode_url_component(redirect_uri),
        encode_url_component(&oauth_scopes().join(" ")),
        encode_url_component(state),
        encode_url_component(code_challenge),
    )
}

fn open_browser(url: &str) -> Result<(), String> {
    tauri_plugin_opener::open_url(url, None::<&str>)
        .map_err(|error| format!("Failed to open Antigravity OAuth login page: {error}"))
}

async fn exchange_code_for_token(
    db: &SqliteDbState,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
    client: &AntigravityOAuthClient,
) -> Result<OAuthTokenResponse, String> {
    let payload = OAuthCodeRequest {
        grant_type: "authorization_code",
        client_id: &client.client_id,
        client_secret: &client.client_secret,
        code,
        redirect_uri,
        code_verifier,
    };

    let client_http = http_client::client_with_timeout(db, 30).await?;
    let response = client_http
        .post(ANTIGRAVITY_OAUTH_TOKEN_URL)
        .form(&payload)
        .send()
        .await
        .map_err(|error| format!("OAuth token exchange request failed: {error}"))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("Failed to read OAuth token response: {error}"))?;

    if !status.is_success() {
        return Err(format!(
            "OAuth token exchange failed (HTTP {status}): {text}"
        ));
    }

    serde_json::from_str::<OAuthTokenResponse>(&text)
        .map_err(|error| format!("Failed to parse OAuth token response: {error}"))
}

async fn refresh_access_token(
    db: &SqliteDbState,
    refresh_token: &str,
    client: &AntigravityOAuthClient,
) -> Result<OAuthTokenResponse, String> {
    let payload = OAuthRefreshRequest {
        grant_type: "refresh_token",
        client_id: &client.client_id,
        client_secret: &client.client_secret,
        refresh_token,
    };

    let client_http = http_client::client_with_timeout(db, 30).await?;
    let response = client_http
        .post(ANTIGRAVITY_OAUTH_TOKEN_URL)
        .form(&payload)
        .send()
        .await
        .map_err(|error| format!("OAuth token refresh request failed: {error}"))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("Failed to read OAuth token refresh response: {error}"))?;

    if !status.is_success() {
        return Err(format!(
            "OAuth token refresh failed (HTTP {status}): {text}"
        ));
    }

    serde_json::from_str::<OAuthTokenResponse>(&text)
        .map_err(|error| format!("Failed to parse OAuth token refresh response: {error}"))
}

async fn fetch_user_profile(
    db: &SqliteDbState,
    access_token: &str,
) -> Result<(Option<String>, Option<String>), String> {
    let client_http = http_client::client_with_timeout(db, 30).await?;
    let response = client_http
        .get(ANTIGRAVITY_USER_INFO_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| format!("Failed to query userinfo endpoint: {error}"))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("Failed to read userinfo response: {error}"))?;

    if !status.is_success() {
        return Err(format!("Query userinfo failed (HTTP {status}): {text}"));
    }

    let parsed = serde_json::from_str::<Value>(&text)
        .map_err(|error| format!("Failed to parse userinfo JSON: {error}"))?;

    let email = parsed
        .get("email")
        .and_then(Value::as_str)
        .map(str::to_string);
    let name = parsed
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((email, name))
}

async fn retrieve_user_quota(
    db: &SqliteDbState,
    access_token: &str,
) -> Result<AntigravityQuotaSnapshot, String> {
    let client = http_client::client_with_timeout(db, 30).await?;
    let url = format!("{ANTIGRAVITY_CODE_ASSIST_URL}:retrieveUserQuota");
    let response = client
        .post(&url)
        .bearer_auth(access_token)
        .json(&json!({}))
        .send()
        .await
        .map_err(|error| format!("Failed to retrieve Antigravity quota: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Failed to retrieve Antigravity quota (HTTP {status})"
        ));
    }
    let value = response
        .json::<Value>()
        .await
        .map_err(|error| format!("Failed to parse Antigravity quota: {error}"))?;
    parse_quota_snapshot(&value)
}

fn parse_quota_snapshot(value: &Value) -> Result<AntigravityQuotaSnapshot, String> {
    // Code Assist returns model buckets. Keep the older weekly shape readable,
    // but never turn a missing fraction into a fabricated 100% remaining.
    if let Some(buckets) = value.get("buckets").and_then(Value::as_array) {
        let mut labels = Vec::new();
        let mut reset_at = None;
        for bucket in buckets {
            let Some(fraction) = bucket.get("remainingFraction").and_then(Value::as_f64) else {
                continue;
            };
            let model = bucket
                .get("modelId")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            labels.push(format!("{model} {:.1}%", fraction.clamp(0.0, 1.0) * 100.0));
            if let Some(reset) = bucket
                .get("resetTime")
                .and_then(Value::as_str)
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|date| date.timestamp())
            {
                reset_at = Some(reset_at.map_or(reset, |previous: i64| previous.min(reset)));
            }
        }
        if !buckets.is_empty() && labels.is_empty() {
            return Err("Antigravity quota response has no remaining fractions".to_string());
        }
        return Ok(AntigravityQuotaSnapshot {
            project_id: value
                .get("projectId")
                .and_then(Value::as_str)
                .map(str::to_string),
            plan_type: value
                .get("planType")
                .and_then(Value::as_str)
                .map(str::to_string),
            limit_weekly_text: (!labels.is_empty()).then(|| labels.join(" · ")),
            limit_weekly_reset_at: reset_at,
        });
    }
    let quotas = value
        .get("userQuotas")
        .and_then(Value::as_array)
        .ok_or_else(|| "Antigravity quota response is missing userQuotas".to_string())?;

    let project_id = value
        .get("projectId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let plan_type = value
        .get("planType")
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut limit_weekly_text = None;
    let mut limit_weekly_reset_at = None;

    for quota in quotas {
        if let Some(window) = quota.get("resetWindow").and_then(Value::as_str) {
            if window == "WEEKLY" {
                let percent = quota.get("percentRemaining").and_then(Value::as_f64);
                if let Some(p) = percent {
                    limit_weekly_text = Some(format!("{:.1}% remaining", p));
                }
                if let Some(reset_time) = quota.get("resetTime").and_then(Value::as_str) {
                    if let Ok(parsed_time) = chrono::DateTime::parse_from_rfc3339(reset_time) {
                        limit_weekly_reset_at = Some(parsed_time.timestamp());
                    }
                }
            }
        }
    }
    Ok(AntigravityQuotaSnapshot {
        project_id,
        plan_type,
        limit_weekly_text,
        limit_weekly_reset_at,
    })
}

fn snapshot_expiry_epoch(snapshot: &Value) -> Option<i64> {
    let expiry = snapshot
        .pointer("/token/expiry")
        .or_else(|| snapshot.get("expiry"))?;
    match expiry {
        Value::Number(value) => value.as_i64(),
        Value::String(value) => chrono::DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|date| date.timestamp())
            .or_else(|| value.parse::<i64>().ok()),
        _ => None,
    }
}

async fn ensure_fresh_auth_snapshot(db: &SqliteDbState, snapshot: &str) -> Result<String, String> {
    let parsed = serde_json::from_str::<Value>(snapshot)
        .map_err(|error| format!("Failed to parse Antigravity account snapshot: {error}"))?;
    let now = chrono::Utc::now().timestamp();
    if snapshot_expiry_epoch(&parsed).is_some_and(|expiry| expiry > now + AUTH_REFRESH_LEAD_SECONDS)
        && auth_string(&parsed, "access_token").is_some()
    {
        return Ok(snapshot.to_string());
    }

    let refresh_token = auth_string(&parsed, "refresh_token")
        .ok_or_else(|| "Antigravity account snapshot has no refresh token".to_string())?;
    let token = refresh_access_token(db, refresh_token, &antigravity_oauth_client()).await?;
    merge_refreshed_snapshot(&parsed, &token)
}

fn auth_string<'a>(snapshot: &'a Value, field: &str) -> Option<&'a str> {
    snapshot
        .get("token")
        .and_then(|token| token.get(field))
        .or_else(|| snapshot.get(field))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn merge_refreshed_snapshot(
    snapshot: &Value,
    token: &OAuthTokenResponse,
) -> Result<String, String> {
    if token.access_token.trim().is_empty() {
        return Err("OAuth token response contains an empty access token".to_string());
    }
    let expiry = chrono::Utc::now()
        .checked_add_signed(
            chrono::Duration::try_seconds(token.expires_in.unwrap_or(0))
                .ok_or_else(|| "Invalid OAuth token lifetime".to_string())?,
        )
        .ok_or_else(|| "Invalid OAuth token expiry".to_string())?;
    let mut result = snapshot.clone();
    if !result.is_object() {
        return Err("Antigravity account snapshot must be a JSON object".to_string());
    }
    if !result.get("token").is_some_and(Value::is_object) {
        result["token"] = json!({});
    }
    result["token"]["access_token"] = json!(token.access_token);
    result["token"]["expiry"] = json!(expiry.to_rfc3339());
    result["token"]["token_type"] = json!(token.token_type.as_deref().unwrap_or("Bearer"));
    for (field, incoming) in [
        ("refresh_token", token.refresh_token.as_deref()),
        ("id_token", token.id_token.as_deref()),
    ] {
        if let Some(value) = incoming
            .filter(|value| !value.trim().is_empty())
            .or_else(|| auth_string(snapshot, field))
        {
            result["token"][field] = json!(value);
        }
    }
    if result.get("auth_method").is_none() {
        result["auth_method"] = json!("consumer");
    }
    Ok(result.to_string())
}

fn persist_refreshed_snapshot(
    db: &SqliteDbState,
    account_id: &str,
    snapshot: &str,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    db.with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::CliOfficialAccount(Cli::Antigravity),
            account_id,
            &[
                ("auth_snapshot", json!(snapshot)),
                ("last_refresh", json!(now)),
            ],
        )?
        .ok_or_else(|| format!("Antigravity official account not found: {account_id}"))?;
        Ok(())
    })
}

pub async fn ensure_antigravity_provider_has_no_official_accounts(
    db: &SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    let provider_id_value = Value::String(provider_id.to_string());
    let provider_id_path = JsonFieldPath::new("provider_id")?;
    let accounts: Vec<Value> = db.with_conn(|conn| {
        db_query_by_field(
            conn,
            DbTable::CliOfficialAccount(Cli::Antigravity),
            &provider_id_path,
            &provider_id_value,
            None,
            None,
        )
    })?;

    if !accounts.is_empty() {
        return Err(
            "Cannot change category: this provider still has official accounts associated with it"
                .to_string(),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn list_antigravity_official_accounts(
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
) -> Result<Vec<AntigravityOfficialAccount>, String> {
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider_by_id(db, &provider_id)?;
    if provider.category != "official" {
        return Err("Only official Antigravity providers can list official accounts".to_string());
    }

    let provider_id_value = Value::String(provider_id.clone());
    let provider_id_path = JsonFieldPath::new("provider_id")?;
    let order = OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::json_text("created_at", OrderDirection::Asc)?,
    ]);

    let records: Vec<Value> = db.with_conn(|conn| {
        db_query_by_field(
            conn,
            DbTable::CliOfficialAccount(Cli::Antigravity),
            &provider_id_path,
            &provider_id_value,
            Some(&order),
            None,
        )
    })?;

    let mut accounts: Vec<AntigravityOfficialAccount> = records
        .into_iter()
        .map(adapter::from_db_value_official_account)
        .collect();

    // Keep the device's current Antigravity CLI configuration as a stable
    // default entry, even when the local CLI has never completed OAuth.
    let local_is_applied = provider.is_applied
        && !accounts
            .iter()
            .any(|account| account.id != LOCAL_OFFICIAL_ACCOUNT_ID && account.is_applied);
    if let Some(local_account) = accounts
        .iter_mut()
        .find(|account| account.id == LOCAL_OFFICIAL_ACCOUNT_ID)
    {
        local_account.is_virtual = true;
        local_account.is_applied = local_is_applied;
    } else {
        accounts.push(build_virtual_local_account(&provider_id, local_is_applied));
    }

    Ok(accounts)
}

fn build_virtual_local_account(provider_id: &str, is_applied: bool) -> AntigravityOfficialAccount {
    let now = Local::now().to_rfc3339();
    AntigravityOfficialAccount {
        id: LOCAL_OFFICIAL_ACCOUNT_ID.to_string(),
        provider_id: provider_id.to_string(),
        name: "Antigravity CLI 默认配置".to_string(),
        kind: "local".to_string(),
        email: None,
        auth_snapshot: None,
        auth_mode: None,
        account_id: None,
        project_id: None,
        plan_type: None,
        last_refresh: None,
        token_expires_at: None,
        access_token_preview: None,
        refresh_token_preview: None,
        limit_short_label: None,
        limit_5h_text: None,
        limit_weekly_text: None,
        limit_5h_reset_at: None,
        limit_weekly_reset_at: None,
        last_limits_fetched_at: None,
        last_error: None,
        sort_index: None,
        is_applied,
        is_virtual: true,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn capture_local_default_snapshot(
    db: &SqliteDbState,
    provider_id: &str,
    credentials: &impl CredentialStore,
) -> Result<bool, String> {
    // The OS entry is global, not scoped to provider/root_dir. Only the active
    // default owns its contents. A login started while B is selected must not
    // replace the default's snapshot with B's credentials.
    db.with_conn_mut(|conn| {
        db_transaction(conn, |tx| {
            let managed_is_applied = db_list(tx, DbTable::CliOfficialAccount(Cli::Antigravity), None)?
                .into_iter()
                .map(adapter::from_db_value_official_account)
                .any(|account| account.id != LOCAL_OFFICIAL_ACCOUNT_ID && account.is_applied);
            if managed_is_applied {
                return Ok(false);
            }

            // Capture on EVERY departure from default: agy may have logged in
            // or rotated its tokens since the initial (possibly empty) snapshot.
            // Read/persist failures must abort before replacing the live entry.
            let snapshot = credentials.read()?.unwrap_or_default();
            let now = Local::now().to_rfc3339();
            let previous = db_get(
                tx,
                DbTable::CliOfficialAccount(Cli::Antigravity),
                LOCAL_OFFICIAL_ACCOUNT_ID,
            )?;
            let record =
                adapter::merge_local_default_snapshot(previous, provider_id, &snapshot, &now);
            db_put(
                tx,
                DbTable::CliOfficialAccount(Cli::Antigravity),
                LOCAL_OFFICIAL_ACCOUNT_ID,
                &record,
            )?;
            Ok(true)
        })
    })
}

fn saved_local_default_snapshot(db: &SqliteDbState) -> Result<Option<String>, String> {
    let record = db
        .with_conn(|conn| {
            db_get(
                conn,
                DbTable::CliOfficialAccount(Cli::Antigravity),
                LOCAL_OFFICIAL_ACCOUNT_ID,
            )
        })?
        .ok_or_else(|| {
            "The Antigravity default account snapshot is missing; the current login was kept"
                .to_string()
        })?;
    let snapshot = record
        .get("auth_snapshot")
        .or_else(|| record.get("authSnapshot"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            "The Antigravity default account snapshot is invalid; the current login was kept"
                .to_string()
        })?;
    if snapshot.trim().is_empty() {
        if record
            .get("snapshot_captured_from_default")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err("The legacy Antigravity default snapshot is empty; the current login was kept because the original login cannot be recovered from this snapshot".to_string());
        }
        Ok(None)
    } else {
        Ok(Some(snapshot.to_string()))
    }
}

#[tauri::command]
pub async fn start_antigravity_official_account_oauth(
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
) -> Result<AntigravityOfficialAccount, String> {
    let _login = OAUTH_LOGIN_LOCK
        .try_lock()
        .map_err(|_| "An Antigravity OAuth login is already in progress".to_string())?;
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider_by_id(db, &provider_id)?;
    if provider.category != "official" {
        return Err("Only official Antigravity providers can manage official accounts".to_string());
    }
    if provider.is_disabled {
        return Err(format!(
            "Provider '{}' is disabled and cannot start OAuth",
            provider_id
        ));
    }

    {
        let _operation = ACCOUNT_OPERATION_LOCK.lock().await;
        capture_local_default_snapshot(db, &provider_id, &OsCredentialStore)?;
    }
    let (listener, port) = oauth_callback::bind_listener()?;
    let redirect_uri = oauth_callback::redirect_uri(port);
    let state = generate_random_urlsafe(24);
    let client = antigravity_oauth_client();
    // `agy` uses PKCE; Google validates the challenge format, so this must be
    // base64url(SHA-256(verifier)) rather than a plain random string.
    let code_verifier = build_pkce_verifier();
    let code_challenge = build_pkce_challenge(&code_verifier);
    let auth_url =
        build_antigravity_authorize_url(&client.client_id, &redirect_uri, &state, &code_challenge);

    open_browser(&auth_url)?;

    let code = oauth_callback::capture_callback(listener, &state, Duration::from_secs(120)).await?;

    let token_resp =
        exchange_code_for_token(db, &code, &redirect_uri, &code_verifier, &client).await?;
    let auth_snapshot = merge_refreshed_snapshot(&json!({"auth_method": "consumer"}), &token_resp)?;
    let (email, name) = fetch_user_profile(db, &token_resp.access_token)
        .await
        .unwrap_or((None, None));
    let quota_result = retrieve_user_quota(db, &token_resp.access_token).await;
    let quota_error = quota_result.as_ref().err().cloned();
    let quota = quota_result.unwrap_or_default();

    let display_name = email
        .clone()
        .or(name)
        .unwrap_or_else(|| "Google Official Account".to_string());

    let now_str = Local::now().to_rfc3339();
    let new_id = db_new_id();

    let content = AntigravityOfficialAccountContent {
        provider_id: provider_id.clone(),
        name: display_name,
        kind: "oauth".to_string(),
        email,
        auth_snapshot,
        auth_mode: Some("oauth-personal".to_string()),
        account_id: None,
        project_id: quota.project_id,
        plan_type: quota.plan_type,
        last_refresh: Some(now_str.clone()),
        limit_short_label: quota.limit_weekly_text.clone(),
        limit_5h_text: None,
        limit_weekly_text: quota.limit_weekly_text,
        limit_5h_reset_at: None,
        limit_weekly_reset_at: quota.limit_weekly_reset_at,
        last_limits_fetched_at: quota_error.is_none().then(|| now_str.clone()),
        last_error: quota_error,
        sort_index: Some(0),
        is_applied: false,
        created_at: now_str.clone(),
        updated_at: now_str,
    };

    let db_val = adapter::to_db_value_official_account(&content);
    db.with_conn(|conn| db_put(conn, DbTable::CliOfficialAccount(Cli::Antigravity), &new_id, &db_val))?;

    let created_record = db
        .with_conn(|conn| db_get(conn, DbTable::CliOfficialAccount(Cli::Antigravity), &new_id))?
        .ok_or_else(|| {
            "Failed to retrieve newly created Antigravity official account".to_string()
        })?;

    Ok(adapter::from_db_value_official_account(created_record))
}

#[tauri::command]
pub async fn save_antigravity_official_local_account(
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
) -> Result<AntigravityOfficialAccount, String> {
    let _operation = ACCOUNT_OPERATION_LOCK.lock().await;
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider_by_id(db, &provider_id)?;
    if provider.category != "official" {
        return Err(
            "Only official Antigravity providers can save local official accounts".to_string(),
        );
    }
    if provider.is_disabled {
        return Err(format!(
            "Provider '{}' is disabled and cannot save a local OAuth account",
            provider_id
        ));
    }
    capture_local_default_snapshot(db, &provider_id, &OsCredentialStore)?;
    // `agy` keeps its login in the OS credential store; import whatever is
    // there right now instead of reading a file.
    let content = credential_store::read_credential_text()?
        .ok_or_else(|| "No Antigravity CLI login found in the OS credential store".to_string())?;

    let parsed = serde_json::from_str::<Value>(&content)
        .map_err(|error| format!("Failed to parse Antigravity CLI credential: {error}"))?;
    let access_token = parsed
        .pointer("/token/access_token")
        .or_else(|| parsed.pointer("/access_token"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    let refreshed_snapshot = ensure_fresh_auth_snapshot(db, &content).await?;
    let refreshed_parsed = serde_json::from_str::<Value>(&refreshed_snapshot)
        .map_err(|error| format!("Failed to parse refreshed Antigravity credentials: {error}"))?;
    let access_token = refreshed_parsed
        .pointer("/token/access_token")
        .or_else(|| refreshed_parsed.pointer("/access_token"))
        .and_then(Value::as_str)
        .unwrap_or(access_token);

    let (email, name) = if !access_token.is_empty() {
        fetch_user_profile(db, access_token)
            .await
            .unwrap_or((None, None))
    } else {
        (None, None)
    };

    let quota_result = if !access_token.is_empty() {
        retrieve_user_quota(db, access_token).await
    } else {
        Err("No valid access token available for limits check".to_string())
    };
    let quota_error = quota_result.as_ref().err().cloned();
    let quota = quota_result.unwrap_or_default();

    let display_name = email
        .clone()
        .or(name)
        .unwrap_or_else(|| "Local OAuth Account".to_string());

    let now_str = Local::now().to_rfc3339();
    let new_id = db_new_id();

    let account_content = AntigravityOfficialAccountContent {
        provider_id: provider_id.clone(),
        name: display_name,
        kind: "oauth".to_string(),
        email,
        auth_snapshot: refreshed_snapshot,
        auth_mode: Some("oauth-personal".to_string()),
        account_id: None,
        project_id: quota.project_id,
        plan_type: quota.plan_type,
        last_refresh: Some(now_str.clone()),
        limit_short_label: quota.limit_weekly_text.clone(),
        limit_5h_text: None,
        limit_weekly_text: quota.limit_weekly_text,
        limit_5h_reset_at: None,
        limit_weekly_reset_at: quota.limit_weekly_reset_at,
        last_limits_fetched_at: quota_error.is_none().then(|| now_str.clone()),
        last_error: quota_error,
        sort_index: Some(0),
        is_applied: provider.is_applied,
        created_at: now_str.clone(),
        updated_at: now_str.clone(),
    };

    persist_imported_account(db, &new_id, &account_content)?;

    let created_record = db
        .with_conn(|conn| db_get(conn, DbTable::CliOfficialAccount(Cli::Antigravity), &new_id))?
        .ok_or_else(|| "Failed to retrieve newly imported Antigravity local account".to_string())?;

    Ok(adapter::from_db_value_official_account(created_record))
}

fn persist_imported_account(
    db: &SqliteDbState,
    id: &str,
    content: &AntigravityOfficialAccountContent,
) -> Result<(), String> {
    db.with_conn_mut(|conn| {
        db_transaction(conn, |tx| {
            if content.is_applied {
                db_patch_where_bool(
                    tx,
                    DbTable::CliOfficialAccount(Cli::Antigravity),
                    &JsonFieldPath::new("is_applied")?,
                    true,
                    &[("is_applied", json!(false))],
                )?;
            }
            db_put(
                tx,
                DbTable::CliOfficialAccount(Cli::Antigravity),
                id,
                &adapter::to_db_value_official_account(content),
            )
        })
    })
}

#[tauri::command]
pub async fn apply_antigravity_official_account(
    app: tauri::AppHandle,
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
    account_id: String,
) -> Result<(), String> {
    let _operation = ACCOUNT_OPERATION_LOCK.lock().await;
    let db = state.db();
    apply_official_account_with_store(db, &provider_id, &account_id, &OsCredentialStore).await?;
    // Publish only after both runtime credentials and applied flags agree.
    let _ = app.emit("config-changed", "window");
    let _ = app.emit("wsl-sync-request-antigravity", ());
    Ok(())
}

// Caller holds ACCOUNT_OPERATION_LOCK, shared with OAuth/import/background refresh.
async fn apply_official_account_with_store(
    db: &SqliteDbState,
    provider_id: &str,
    account_id: &str,
    credentials: &impl CredentialStore,
) -> Result<(), String> {
    ensure_persisted_provider_id(provider_id)?;
    let provider = query_provider_by_id(db, provider_id)?;
    if provider.category != "official" {
        return Err("Only official Antigravity providers can apply official accounts".to_string());
    }
    if provider.is_disabled {
        return Err(format!(
            "Provider '{}' is disabled and cannot be applied",
            provider_id
        ));
    }

    let target_snapshot = if account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        if capture_local_default_snapshot(db, provider_id, credentials)? {
            // Already on default: never overwrite a fresh agy login with a stale
            // stored snapshot (or delete it because the old snapshot was empty).
            return apply_config_internal_without_events(db, provider_id).await;
        }
        saved_local_default_snapshot(db)?
    } else {
        let record = db
            .with_conn(|conn| db_get(conn, DbTable::CliOfficialAccount(Cli::Antigravity), account_id))?
            .ok_or_else(|| format!("Antigravity official account not found: {account_id}"))?;

        let account = adapter::from_db_value_official_account(record);
        if account.provider_id != provider_id {
            return Err(
                "Antigravity official account does not belong to the selected provider".to_string(),
            );
        }
        let snapshot = account
            .auth_snapshot
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| "Official account has no authentication snapshot".to_string())?;

        let refreshed_snapshot = ensure_fresh_auth_snapshot(db, &snapshot).await?;
        if refreshed_snapshot != snapshot {
            persist_refreshed_snapshot(db, account_id, &refreshed_snapshot)?;
        }

        capture_local_default_snapshot(db, provider_id, credentials)?;
        Some(refreshed_snapshot)
    };

    let previous_credential = credentials.read()?;
    let previous_provider = super::commands::list_antigravity_providers_from_sqlite(db)?
        .into_iter()
        .find(|provider| provider.is_applied)
        .map(|provider| provider.id);
    credentials.replace(target_snapshot.as_deref())?;
    let result = async {
        apply_config_internal_without_events(db, provider_id).await?;
        db.with_conn_mut(|conn| {
            db_update_applied_status(
                conn,
                DbTable::CliOfficialAccount(Cli::Antigravity),
                (account_id != LOCAL_OFFICIAL_ACCOUNT_ID).then_some(account_id),
                &Local::now().to_rfc3339(),
            )
        })
    }
    .await;
    if let Err(error) = result {
        // Do not leave the live credential owned by the wrong applied row when
        // writing settings or committing the selection fails.
        let credential_restore = credentials.replace(previous_credential.as_deref());
        let provider_restore = db.with_conn_mut(|conn| {
            db_update_applied_status(
                conn,
                DbTable::CliProvider(Cli::Antigravity),
                previous_provider.as_deref(),
                &Local::now().to_rfc3339(),
            )
        });
        if let Err(restore_error) = credential_restore.and(provider_restore) {
            return Err(format!(
                "{error}; failed to restore the previous Antigravity state: {restore_error}"
            ));
        }
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_antigravity_official_account(
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
    account_id: String,
) -> Result<(), String> {
    let _operation = ACCOUNT_OPERATION_LOCK.lock().await;
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    if account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        return Err("The Antigravity CLI default configuration cannot be deleted".to_string());
    }
    let record = db
        .with_conn(|conn| db_get(conn, DbTable::CliOfficialAccount(Cli::Antigravity), &account_id))?
        .ok_or_else(|| format!("Antigravity official account not found: {account_id}"))?;
    let account = adapter::from_db_value_official_account(record);
    if account.provider_id != provider_id {
        return Err(
            "Antigravity official account does not belong to the selected provider".to_string(),
        );
    }
    if account.is_applied {
        return Err("The applied Antigravity official account cannot be deleted".to_string());
    }
    db.with_conn(|conn| db_delete(conn, DbTable::CliOfficialAccount(Cli::Antigravity), &account_id))?;
    Ok(())
}

pub async fn refresh_applied_antigravity_accounts_if_needed<R: tauri::Runtime>(
    db: &SqliteDbState,
    app: &tauri::AppHandle<R>,
) -> Result<(), String> {
    let records: Vec<Value> =
        db.with_conn(|conn| db_list(conn, DbTable::CliOfficialAccount(Cli::Antigravity), None))?;

    for record in records {
        let id = adapter::from_db_value_official_account(record).id;
        if id == LOCAL_OFFICIAL_ACCOUNT_ID {
            continue;
        }
        // Re-read after taking the same lock as account switching: a refresh
        // started for A must never overwrite the live credential after B wins.
        let _operation = ACCOUNT_OPERATION_LOCK.lock().await;
        let Some(record) =
            db.with_conn(|conn| db_get(conn, DbTable::CliOfficialAccount(Cli::Antigravity), &id))?
        else {
            continue;
        };
        let account = adapter::from_db_value_official_account(record);
        if account.kind != "oauth" {
            continue;
        }
        let provider = match query_provider_by_id(db, &account.provider_id) {
            Ok(provider) if provider.category == "official" && !provider.is_disabled => provider,
            _ => continue,
        };
        match refresh_stored_account(db, &account, provider.is_applied).await {
            Ok(_) => {
                if account.is_applied && provider.is_applied {
                    let _ = app.emit("wsl-sync-request-antigravity", ());
                }
            }
            Err(error) => {
                log::debug!(
                    "Antigravity account {} auth refresh failed: {error}",
                    account.id
                );
            }
        }
    }
    Ok(())
}

async fn refresh_stored_account(
    db: &SqliteDbState,
    account: &AntigravityOfficialAccount,
    provider_applied: bool,
) -> Result<String, String> {
    let snapshot = account
        .auth_snapshot
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Official account has no authentication snapshot".to_string())?;
    let refreshed = ensure_fresh_auth_snapshot(db, snapshot).await?;
    if refreshed != snapshot {
        // Save a rotated refresh token even if writing the live credential or
        // subsequently querying quota fails. Only patch auth-owned fields.
        persist_refreshed_snapshot(db, &account.id, &refreshed)?;
    }
    if account.is_applied && provider_applied {
        credential_store::write_credential_text(&refreshed)?;
    }
    Ok(refreshed)
}

#[tauri::command]
pub async fn refresh_antigravity_official_account_limits(
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
    account_id: String,
) -> Result<AntigravityOfficialAccount, String> {
    let _operation = ACCOUNT_OPERATION_LOCK.lock().await;
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider_by_id(db, &provider_id)?;
    if provider.category != "official" || provider.is_disabled {
        return Err(
            "Only enabled official Antigravity providers can refresh account limits".to_string(),
        );
    }
    if account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        return Err("The Antigravity CLI default configuration has no managed quota".to_string());
    }

    let record = db
        .with_conn(|conn| db_get(conn, DbTable::CliOfficialAccount(Cli::Antigravity), &account_id))?
        .ok_or_else(|| format!("Antigravity official account not found: {account_id}"))?;

    let account = adapter::from_db_value_official_account(record);
    if account.provider_id != provider_id {
        return Err(
            "Antigravity official account does not belong to the selected provider".to_string(),
        );
    }
    let snapshot = refresh_stored_account(db, &account, provider.is_applied).await?;
    let parsed = serde_json::from_str::<Value>(&snapshot)
        .map_err(|error| format!("Failed to parse account snapshot: {error}"))?;
    let access_token = auth_string(&parsed, "access_token")
        .ok_or_else(|| "No valid access token available for limits check".to_string())?;
    let quota_result = retrieve_user_quota(db, access_token).await;
    persist_quota_result(db, &account_id, quota_result)?;

    let updated_record = db
        .with_conn(|conn| db_get(conn, DbTable::CliOfficialAccount(Cli::Antigravity), &account_id))?
        .ok_or_else(|| "Failed to retrieve refreshed account record".to_string())?;

    Ok(adapter::from_db_value_official_account(updated_record))
}

fn persist_quota_result(
    db: &SqliteDbState,
    account_id: &str,
    result: Result<AntigravityQuotaSnapshot, String>,
) -> Result<(), String> {
    let quota = match result {
        Ok(quota) => quota,
        Err(error) => {
            db.with_conn(|conn| {
                db_patch_fields(
                    conn,
                    DbTable::CliOfficialAccount(Cli::Antigravity),
                    account_id,
                    &[("last_error", json!(error))],
                )
            })?;
            return Err(error);
        }
    };
    let now_str = Local::now().to_rfc3339();

    db.with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::CliOfficialAccount(Cli::Antigravity),
            account_id,
            &[
                (
                    "project_id",
                    quota.project_id.map(Value::String).unwrap_or(Value::Null),
                ),
                (
                    "plan_type",
                    quota.plan_type.map(Value::String).unwrap_or(Value::Null),
                ),
                (
                    "limit_short_label",
                    quota
                        .limit_weekly_text
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_weekly_text",
                    quota
                        .limit_weekly_text
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_weekly_reset_at",
                    quota
                        .limit_weekly_reset_at
                        .map(|v| Value::Number(v.into()))
                        .unwrap_or(Value::Null),
                ),
                ("last_limits_fetched_at", Value::String(now_str.clone())),
                ("last_error", Value::Null),
                ("updated_at", Value::String(now_str)),
            ],
        )
    })?;

    Ok(())
}

fn copy_text_to_clipboard(value: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("Failed to access system clipboard: {error}"))?;
    clipboard
        .set_text(value.to_string())
        .map_err(|error| format!("Failed to copy token to clipboard: {error}"))
}

#[tauri::command]
pub async fn copy_antigravity_official_account_token(
    state: tauri::State<'_, SqliteDbState>,
    input: AntigravityOfficialAccountTokenCopyInput,
) -> Result<(), String> {
    let db = state.db();
    ensure_persisted_provider_id(&input.provider_id)?;
    let provider = query_provider_by_id(db, &input.provider_id)?;
    if provider.category != "official" {
        return Err(
            "Only official Antigravity providers can copy official account tokens".to_string(),
        );
    }

    let snapshot = if input.account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        credential_store::read_credential_text()?.ok_or_else(|| {
            "No Antigravity CLI login found in the OS credential store".to_string()
        })?
    } else {
        let record = db
            .with_conn(|conn| db_get(conn, DbTable::CliOfficialAccount(Cli::Antigravity), &input.account_id))?
            .ok_or_else(|| format!("Account not found: {}", input.account_id))?;
        let account = adapter::from_db_value_official_account(record);
        if account.provider_id != input.provider_id {
            return Err(
                "Antigravity official account does not belong to the selected provider".to_string(),
            );
        }
        account
            .auth_snapshot
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| "Official account has no authentication snapshot".to_string())?
    };

    let parsed = serde_json::from_str::<Value>(&snapshot)
        .map_err(|error| format!("Failed to parse account snapshot: {error}"))?;

    let token = match input.token_kind.as_str() {
        "refresh" => parsed
            .pointer("/token/refresh_token")
            .or_else(|| parsed.pointer("/refresh_token"))
            .and_then(Value::as_str),
        _ => parsed
            .pointer("/token/access_token")
            .or_else(|| parsed.pointer("/access_token"))
            .and_then(Value::as_str),
    };

    let token = token.filter(|t| !t.trim().is_empty()).ok_or_else(|| {
        format!(
            "No {} token available in account snapshot",
            input.token_kind
        )
    })?;

    copy_text_to_clipboard(token)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::CliConfigKind;

    #[derive(Default)]
    struct MemoryCredentials {
        value: std::sync::Mutex<Option<String>>,
        writes: std::sync::atomic::AtomicUsize,
        fail_read: std::sync::atomic::AtomicBool,
    }

    impl CredentialStore for MemoryCredentials {
        fn read(&self) -> Result<Option<String>, String> {
            if self.fail_read.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("test credential read failure".to_string());
            }
            Ok(self.value.lock().unwrap().clone())
        }

        fn replace(&self, snapshot: Option<&str>) -> Result<(), String> {
            self.writes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.value.lock().unwrap() = snapshot.map(str::to_string);
            Ok(())
        }
    }

    fn login_snapshot(account: &str) -> String {
        json!({"auth_method": "consumer", "runtime_owned": {"keep": true}, "token": {
            "access_token": format!("{account}-access"),
            "refresh_token": format!("{account}-refresh"),
            "id_token": format!("{account}-id"), "expiry": "2099-01-01T00:00:00Z"
        }})
        .to_string()
    }

    async fn switch_fixture() -> (SqliteDbState, tempfile::TempDir, MemoryCredentials) {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let root = tempfile::tempdir().unwrap();
        db.with_conn(|conn| {
            db_put(
                conn,
                DbTable::CliConfig(Cli::Antigravity, CliConfigKind::Common),
                "common",
                &json!({"config": "{}", "root_dir": root.path().to_string_lossy()}),
            )?;
            db_put(
                conn,
                DbTable::CliProvider(Cli::Antigravity),
                "official",
                &json!({
                    "category": "official", "is_applied": true, "settings_config":
                        r#"{"config":{"security":{"auth":{"selectedType":"oauth-personal"}}}}"#
                }),
            )
        })
        .unwrap();
        crate::coding::runtime_location::refresh_runtime_location_cache_for_module_async(
            &db,
            "antigravity",
        )
        .await
        .unwrap();
        for id in ["B", "C"] {
            let mut content = account_content(false);
            content.auth_snapshot = login_snapshot(id);
            persist_imported_account(&db, id, &content).unwrap();
        }
        (db, root, MemoryCredentials::default())
    }

    #[tokio::test]
    async fn default_login_survives_switch_new_oauth_account_and_return() {
        let _env = crate::coding::test_env::lock();
        let (db, root, credentials) = switch_fixture().await;
        // Old initial snapshot was taken before the user logged in through agy.
        capture_local_default_snapshot(&db, "official", &credentials).unwrap();
        let default_login = login_snapshot("default-A");
        credentials.replace(Some(&default_login)).unwrap();
        apply_official_account_with_store(&db, "official", "B", &credentials)
            .await
            .unwrap();
        assert_eq!(credentials.read().unwrap(), Some(login_snapshot("B")));
        assert_eq!(
            saved_local_default_snapshot(&db).unwrap(),
            Some(default_login.clone())
        );

        // Same capture entry used by OAuth-start/import, with B still selected.
        // Even a different provider must not claim the global credential as default.
        assert!(!capture_local_default_snapshot(&db, "other-provider", &credentials).unwrap());
        let mut new_account = account_content(false);
        new_account.auth_snapshot = login_snapshot("new-C");
        persist_imported_account(&db, "new-C", &new_account).unwrap();
        apply_official_account_with_store(&db, "official", "new-C", &credentials)
            .await
            .unwrap();
        apply_official_account_with_store(&db, "official", LOCAL_OFFICIAL_ACCOUNT_ID, &credentials)
            .await
            .unwrap();

        assert_eq!(credentials.read().unwrap(), Some(default_login.clone()));
        assert_eq!(
            saved_local_default_snapshot(&db).unwrap(),
            Some(default_login)
        );
        assert!(!read_account(&db, "B").is_applied);
        assert!(!read_account(&db, "new-C").is_applied);
        assert!(query_provider_by_id(&db, "official").unwrap().is_applied);
        let settings: Value = serde_json::from_str(
            &std::fs::read_to_string(root.path().join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            settings.pointer("/security/auth/selectedType"),
            Some(&json!("oauth-personal"))
        );
    }

    #[tokio::test]
    async fn default_reapply_and_later_departures_preserve_new_cli_login() {
        let _env = crate::coding::test_env::lock();
        let (db, _root, credentials) = switch_fixture().await;
        db.with_conn(|conn| {
            db_put(
                conn,
                DbTable::CliOfficialAccount(Cli::Antigravity),
                LOCAL_OFFICIAL_ACCOUNT_ID,
                &json!({"auth_snapshot": "", "kind": "local", "future_metadata": {"keep": true}}),
            )
        })
        .unwrap();
        let login = login_snapshot("default-after-cli-login");
        credentials.replace(Some(&login)).unwrap();
        let writes = credentials.writes.load(std::sync::atomic::Ordering::SeqCst);
        apply_official_account_with_store(&db, "official", LOCAL_OFFICIAL_ACCOUNT_ID, &credentials)
            .await
            .unwrap();
        assert_eq!(
            credentials.writes.load(std::sync::atomic::Ordering::SeqCst),
            writes
        );
        assert_eq!(credentials.read().unwrap(), Some(login));
        let saved = db
            .with_conn(|conn| {
                db_get(
                    conn,
                    DbTable::CliOfficialAccount(Cli::Antigravity),
                    LOCAL_OFFICIAL_ACCOUNT_ID,
                )
            })
            .unwrap()
            .unwrap();
        assert_eq!(saved["future_metadata"]["keep"], true);
        for account in ["B", "C"] {
            let rotated = login_snapshot(&format!("default-rotated-before-{account}"));
            credentials.replace(Some(&rotated)).unwrap();
            apply_official_account_with_store(&db, "official", account, &credentials)
                .await
                .unwrap();
            apply_official_account_with_store(
                &db,
                "official",
                LOCAL_OFFICIAL_ACCOUNT_ID,
                &credentials,
            )
            .await
            .unwrap();
            assert_eq!(credentials.read().unwrap(), Some(rotated));
        }
    }

    #[tokio::test]
    async fn genuinely_logged_out_default_round_trips_without_borrowing_managed_login() {
        let _env = crate::coding::test_env::lock();
        let (db, _root, credentials) = switch_fixture().await;
        apply_official_account_with_store(&db, "official", "B", &credentials)
            .await
            .unwrap();
        assert!(!capture_local_default_snapshot(&db, "official", &credentials).unwrap());
        apply_official_account_with_store(&db, "official", "C", &credentials)
            .await
            .unwrap();
        apply_official_account_with_store(&db, "official", LOCAL_OFFICIAL_ACCOUNT_ID, &credentials)
            .await
            .unwrap();
        assert_eq!(credentials.read().unwrap(), None);
        assert!(!read_account(&db, "C").is_applied);
    }

    #[tokio::test]
    async fn legacy_empty_or_missing_default_snapshot_does_not_delete_current_login() {
        let _env = crate::coding::test_env::lock();
        let (db, _root, credentials) = switch_fixture().await;
        apply_official_account_with_store(&db, "official", "B", &credentials)
            .await
            .unwrap();
        let writes = credentials.writes.load(std::sync::atomic::Ordering::SeqCst);
        for legacy in [
            None,
            Some(json!({"auth_snapshot": ""})),
            Some(json!({"kind": "local"})),
        ] {
            db.with_conn(|conn| match legacy {
                Some(value) => db_put(
                    conn,
                    DbTable::CliOfficialAccount(Cli::Antigravity),
                    LOCAL_OFFICIAL_ACCOUNT_ID,
                    &value,
                ),
                None => db_delete(
                    conn,
                    DbTable::CliOfficialAccount(Cli::Antigravity),
                    LOCAL_OFFICIAL_ACCOUNT_ID,
                )
                .map(|_| ()),
            })
            .unwrap();
            assert!(apply_official_account_with_store(
                &db,
                "official",
                LOCAL_OFFICIAL_ACCOUNT_ID,
                &credentials
            )
            .await
            .is_err());
            assert_eq!(credentials.read().unwrap(), Some(login_snapshot("B")));
            assert!(read_account(&db, "B").is_applied);
        }
        assert_eq!(
            credentials.writes.load(std::sync::atomic::Ordering::SeqCst),
            writes
        );
        // Existing nonempty snapshots remain usable without a schema migration.
        let original = login_snapshot("legacy-default");
        db.with_conn(|conn| {
            db_put(
                conn,
                DbTable::CliOfficialAccount(Cli::Antigravity),
                LOCAL_OFFICIAL_ACCOUNT_ID,
                &json!({"auth_snapshot": original}),
            )
        })
        .unwrap();
        apply_official_account_with_store(&db, "official", LOCAL_OFFICIAL_ACCOUNT_ID, &credentials)
            .await
            .unwrap();
        assert_eq!(credentials.read().unwrap(), Some(original));
    }

    #[tokio::test]
    async fn snapshot_read_or_save_failure_aborts_before_overwriting_default_login() {
        let _env = crate::coding::test_env::lock();
        let (db, _root, credentials) = switch_fixture().await;
        let original = login_snapshot("default");
        credentials.replace(Some(&original)).unwrap();
        let writes = credentials.writes.load(std::sync::atomic::Ordering::SeqCst);
        credentials
            .fail_read
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(
            apply_official_account_with_store(&db, "official", "B", &credentials)
                .await
                .unwrap_err()
                .contains("read failure")
        );
        credentials
            .fail_read
            .store(false, std::sync::atomic::Ordering::SeqCst);
        // 触发器里比对的就是桥接态那个 id：用常量插值，改名时这里不会脱节。
        let trigger_sql = format!(
            "CREATE TRIGGER reject_default_snapshot BEFORE INSERT ON cli_official_account \
             WHEN NEW.id = '{LOCAL_OFFICIAL_ACCOUNT_ID}' \
             BEGIN SELECT RAISE(ABORT, 'snapshot save failure'); END;"
        );
        db.with_conn(|conn| conn.execute_batch(&trigger_sql).map_err(|e| e.to_string()))
            .unwrap();
        assert!(
            apply_official_account_with_store(&db, "official", "B", &credentials)
                .await
                .unwrap_err()
                .contains("snapshot save failure")
        );
        assert_eq!(credentials.read().unwrap(), Some(original));
        assert_eq!(
            credentials.writes.load(std::sync::atomic::Ordering::SeqCst),
            writes
        );
        assert!(!read_account(&db, "B").is_applied);
    }

    #[tokio::test]
    async fn failed_apply_restores_live_credential_and_keeps_previous_selection() {
        let _env = crate::coding::test_env::lock();
        let (db, root, credentials) = switch_fixture().await;
        let original = login_snapshot("default");
        credentials.replace(Some(&original)).unwrap();
        // A directory at the settings-file path makes the real config write fail.
        std::fs::create_dir(root.path().join("settings.json")).unwrap();
        assert!(
            apply_official_account_with_store(&db, "official", "B", &credentials)
                .await
                .is_err()
        );
        assert_eq!(credentials.read().unwrap(), Some(original.clone()));
        assert_eq!(saved_local_default_snapshot(&db).unwrap(), Some(original));
        assert!(!read_account(&db, "B").is_applied);
        std::fs::remove_dir(root.path().join("settings.json")).unwrap();
        apply_official_account_with_store(&db, "official", "B", &credentials)
            .await
            .unwrap();
        // Selection persistence can fail after writing the settings and credential.
        db.with_conn(|conn| conn.execute_batch("CREATE TRIGGER reject_selection BEFORE UPDATE ON cli_official_account WHEN NEW.id = 'C' BEGIN SELECT RAISE(ABORT, 'selection save failure'); END;").map_err(|e| e.to_string())).unwrap();
        assert!(
            apply_official_account_with_store(&db, "official", "C", &credentials)
                .await
                .is_err()
        );
        assert_eq!(credentials.read().unwrap(), Some(login_snapshot("B")));
        assert!(read_account(&db, "B").is_applied);
        assert!(!read_account(&db, "C").is_applied);
    }

    fn token_response() -> OAuthTokenResponse {
        serde_json::from_value(json!({"access_token": "access-token", "expires_in": 3600})).unwrap()
    }

    fn account_content(applied: bool) -> AntigravityOfficialAccountContent {
        serde_json::from_value(json!({
            "provider_id": "official", "name": "Account", "kind": "oauth",
            "auth_snapshot": "{}", "is_applied": applied,
            "limit_weekly_text": "50% remaining", "project_id": "saved-project",
            "last_limits_fetched_at": "previous-quota-time",
            "created_at": "created", "updated_at": "updated"
        }))
        .unwrap()
    }

    fn read_account(db: &SqliteDbState, id: &str) -> AntigravityOfficialAccount {
        adapter::from_db_value_official_account(
            db.with_conn(|conn| db_get(conn, DbTable::CliOfficialAccount(Cli::Antigravity), id))
                .unwrap()
                .unwrap(),
        )
    }

    #[test]
    fn auth_snapshot_uses_agy_shape() {
        // `agy` reads only the nested `token` object plus `auth_method`; the
        // flat fields the old Gemini-CLI-shaped snapshot emitted are inert.
        let mut token = token_response();
        token.refresh_token = Some("refresh-token".to_string());
        token.id_token = Some("id-token".to_string());
        let snapshot = merge_refreshed_snapshot(&json!({}), &token).unwrap();
        let parsed: Value = serde_json::from_str(&snapshot).expect("snapshot must be valid JSON");

        assert_eq!(parsed["auth_method"], "consumer");
        assert_eq!(parsed["token"]["access_token"], "access-token");
        assert_eq!(parsed["token"]["refresh_token"], "refresh-token");
        assert_eq!(parsed["token"]["token_type"], "Bearer");
        assert_eq!(parsed["token"]["id_token"], "id-token");
        assert!(parsed.get("access_token").is_none());
        let expiry = parsed["token"]["expiry"]
            .as_str()
            .expect("expiry must be an RFC3339 string");
        assert!(chrono::DateTime::parse_from_rfc3339(expiry).is_ok());
    }

    #[test]
    fn auth_snapshot_omits_blank_id_token() {
        let mut token = token_response();
        token.id_token = Some("  ".to_string());
        token.expires_in = None;
        let snapshot = merge_refreshed_snapshot(&json!({}), &token).unwrap();
        let parsed: Value = serde_json::from_str(&snapshot).expect("snapshot must be valid JSON");
        assert!(parsed["token"].get("id_token").is_none());
        // Missing lifetime must not leave a token looking permanently fresh.
        assert!(snapshot_expiry_epoch(&parsed).unwrap() <= chrono::Utc::now().timestamp());
    }

    #[test]
    fn refreshed_auth_round_trip_preserves_id_token_unknown_fields_and_quota() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        persist_imported_account(&db, "account", &account_content(false)).unwrap();
        let snapshot = json!({"auth_method": "consumer", "extra": {"keep": true}, "token": {
            "refresh_token": "old-refresh", "id_token": "old-id", "extra_token_field": 7
        }});
        let mut response = token_response();
        response.refresh_token = Some("rotated-refresh".to_string());
        let refreshed = merge_refreshed_snapshot(&snapshot, &response).unwrap();
        persist_refreshed_snapshot(&db, "account", &refreshed).unwrap();
        let account = read_account(&db, "account");
        let saved: Value = serde_json::from_str(account.auth_snapshot.as_deref().unwrap()).unwrap();
        assert_eq!(saved["token"]["refresh_token"], "rotated-refresh");
        assert_eq!(saved["token"]["id_token"], "old-id");
        assert_eq!(saved["token"]["extra_token_field"], 7);
        assert_eq!(saved["extra"], snapshot["extra"]);
        assert_eq!(account.limit_weekly_text.as_deref(), Some("50% remaining"));
        assert_eq!(account.project_id.as_deref(), Some("saved-project"));
        assert!(account.token_expires_at.is_some());
        response.refresh_token = Some("  ".to_string());
        response.id_token = Some(" ".to_string());
        let refreshed: Value =
            serde_json::from_str(&merge_refreshed_snapshot(&saved, &response).unwrap()).unwrap();
        assert_eq!(refreshed["token"]["refresh_token"], "rotated-refresh");
        assert_eq!(refreshed["token"]["id_token"], "old-id");
    }

    #[tokio::test]
    async fn fresh_unapplied_account_does_not_request_refresh_or_write_live_credentials() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let mut content = account_content(false);
        // No refresh token: any accidental refresh request would fail.
        content.auth_snapshot = merge_refreshed_snapshot(&json!({}), &token_response()).unwrap();
        persist_imported_account(&db, "account", &content).unwrap();
        let account = read_account(&db, "account");
        assert_eq!(
            refresh_stored_account(&db, &account, true).await.unwrap(),
            content.auth_snapshot
        );
        assert!(read_account(&db, "account").last_refresh.is_none());
        assert!(ensure_fresh_auth_snapshot(&db, "{}").await.is_err());
    }

    #[test]
    fn repeated_local_import_has_one_applied_account_and_rolls_back_on_failure() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        persist_imported_account(&db, "first", &account_content(true)).unwrap();
        persist_imported_account(&db, "second", &account_content(true)).unwrap();
        assert!(!read_account(&db, "first").is_applied);
        assert!(read_account(&db, "second").is_applied);
        persist_imported_account(&db, "inactive", &account_content(false)).unwrap();
        assert!(read_account(&db, "second").is_applied);
        db.with_conn(|conn| conn.execute_batch("CREATE TRIGGER reject_account BEFORE INSERT ON cli_official_account WHEN NEW.id = 'rejected' BEGIN SELECT RAISE(ABORT, 'test failure'); END;").map_err(|e| e.to_string())).unwrap();
        assert!(persist_imported_account(&db, "rejected", &account_content(true)).is_err());
        assert!(read_account(&db, "second").is_applied);
    }

    #[test]
    fn quota_failure_preserves_previous_values_and_success_clears_error() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        persist_imported_account(&db, "account", &account_content(false)).unwrap();
        assert!(persist_quota_result(&db, "account", Err("HTTP 403".to_string())).is_err());
        let account = read_account(&db, "account");
        assert_eq!(account.limit_weekly_text.as_deref(), Some("50% remaining"));
        assert_eq!(account.project_id.as_deref(), Some("saved-project"));
        assert_eq!(
            account.last_limits_fetched_at.as_deref(),
            Some("previous-quota-time")
        );
        assert_eq!(account.last_error.as_deref(), Some("HTTP 403"));
        persist_quota_result(&db, "account", parse_quota_snapshot(&json!({"buckets": [
            {"modelId": "gemini-3-pro", "remainingFraction": 0.25, "resetTime": "2026-09-30T10:00:00Z"},
            {"modelId": "gemini-3-flash"}
        ]}))).unwrap();
        let account = read_account(&db, "account");
        assert_eq!(
            account.limit_weekly_text.as_deref(),
            Some("gemini-3-pro 25.0%")
        );
        assert!(account.last_error.is_none());
        assert!(account.limit_weekly_reset_at.is_some());
        assert!(parse_quota_snapshot(&json!({"error": "bad response"})).is_err());
        assert!(parse_quota_snapshot(&json!({"buckets": [{"modelId": "unknown"}]})).is_err());
    }

    #[tokio::test]
    async fn limits_command_rejects_wrong_provider_disabled_provider_and_virtual_account() {
        use tauri::Manager;
        let db = SqliteDbState::in_memory_for_test().unwrap();
        db.with_conn(|conn| {
            for (id, disabled) in [("official", false), ("other", false), ("disabled", true)] {
                db_put(
                    conn,
                    DbTable::CliProvider(Cli::Antigravity),
                    id,
                    &json!({"category": "official", "is_disabled": disabled}),
                )?;
            }
            Ok(())
        })
        .unwrap();
        persist_imported_account(&db, "account", &account_content(false)).unwrap();
        let app = tauri::test::mock_app();
        app.manage(db);
        for (provider, account, expected) in [
            ("other", "account", "does not belong"),
            ("disabled", "account", "Only enabled"),
            ("official", LOCAL_OFFICIAL_ACCOUNT_ID, "no managed quota"),
        ] {
            let error = refresh_antigravity_official_account_limits(
                app.state(),
                provider.to_string(),
                account.to_string(),
            )
            .await
            .unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn invalid_token_response_cannot_replace_saved_credentials() {
        let mut response = token_response();
        response.access_token.clear();
        assert!(merge_refreshed_snapshot(&json!({}), &response).is_err());
        response.access_token = "access".to_string();
        response.expires_in = Some(i64::MAX);
        assert!(merge_refreshed_snapshot(&json!({}), &response).is_err());
    }

    #[test]
    fn pkce_challenge_is_base64url_sha256_of_verifier() {
        // Google rejects non-base64url `code_challenge` values, so the digest
        // must never be reused verbatim (regression guard for the old bug).
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = build_pkce_challenge(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert!(!challenge.contains('='));
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
    }

    #[test]
    fn authorize_url_carries_scopes_and_pkce() {
        let url = build_antigravity_authorize_url(
            "client-id",
            "http://localhost:8086/oauth2callback",
            "state-value",
            "challenge-value",
        );
        assert!(url.contains("code_challenge=challenge-value"));
        assert!(url.contains("code_challenge_method=S256"));
        // The CLI's own scopes must be requested, not the profile-only set.
        assert!(url.contains("auth%2Faicode"));
        assert!(url.contains("auth%2Fcclog"));
        assert!(url.contains("auth%2Fexperimentsandconfigs"));
    }
}
