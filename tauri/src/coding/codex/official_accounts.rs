use base64::Engine;
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::sync::{watch, Mutex as AsyncMutex};

use super::adapter;
use super::commands::{
    apply_config_internal, get_codex_root_dir_from_db_async, get_codex_root_dir_without_db,
};
use super::constants::CODEX_LOCAL_PROVIDER_ID;
use super::types::{CodexOfficialAccount, CodexOfficialAccountContent, CodexProvider};
use crate::coding::db_id::db_new_id;
use crate::db::helpers::{
    db_delete, db_get, db_patch_fields, db_put, db_query_by_field, db_update_applied_status,
};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use tauri::{Emitter, Manager};

const CODEX_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_OAUTH_AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CODEX_OAUTH_DEFAULT_PORT: u16 = 1455;
const CODEX_OAUTH_CALLBACK_PATH: &str = "/auth/callback";
const CODEX_DEVICE_USER_CODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const CODEX_DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const CODEX_DEVICE_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const CODEX_DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const LOCAL_OFFICIAL_ACCOUNT_ID: &str = CODEX_LOCAL_PROVIDER_ID;
const FIVE_HOUR_WINDOW_SECONDS: i64 = 18_000;
const WEEK_WINDOW_SECONDS: i64 = 604_800;
const MONTH_WINDOW_MIN_SECONDS: i64 = 28 * 24 * 60 * 60;
const MONTH_WINDOW_MAX_SECONDS: i64 = 31 * 24 * 60 * 60;
const AUTH_REFRESH_LEAD_SECONDS: i64 = 3 * 24 * 60 * 60;
static OAUTH_REFRESH_LOCK: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));
static OAUTH_REFRESH_CACHE: LazyLock<
    AsyncMutex<HashMap<String, (std::time::Instant, OAuthTokenResponse)>>,
> = LazyLock::new(|| AsyncMutex::new(HashMap::new()));
static DEVICE_AUTH_SESSIONS: LazyLock<Mutex<HashMap<String, watch::Sender<bool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexOfficialAccountTokenCopyInput {
    pub provider_id: String,
    pub account_id: String,
    pub token_kind: String,
}

#[derive(Debug, Clone, Serialize)]
struct OAuthTokenRequest<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    code: &'a str,
    redirect_uri: &'a str,
    code_verifier: &'a str,
}

#[derive(Debug, Clone, Serialize)]
struct OAuthRefreshRequest<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    refresh_token: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexDeviceAuthStartResult {
    pub session_id: String,
    pub verification_uri: String,
    pub user_code: String,
    pub expires_at: i64,
    pub poll_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexAuthStatusEvent {
    session_id: String,
    status: String,
    message: Option<String>,
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexDeviceUserCodeResponse {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
    #[serde(default)]
    interval: Value,
}

#[derive(Debug, Deserialize)]
struct CodexDeviceTokenResponse {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct JwtAuthClaims {
    #[serde(default)]
    chatgpt_plan_type: Option<String>,
    #[serde(default)]
    chatgpt_user_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    #[serde(default)]
    chatgpt_account_is_fedramp: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct JwtProfileClaims {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct JwtClaims {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    exp: Option<i64>,
    #[serde(rename = "https://api.openai.com/profile", default)]
    profile: Option<JwtProfileClaims>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    auth: Option<JwtAuthClaims>,
}

#[derive(Debug, Clone, Default)]
struct ParsedIdToken {
    email: Option<String>,
    plan_type: Option<String>,
    account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    chatgpt_account_is_fedramp: bool,
}

#[derive(Debug, Clone, Default)]
struct UsageSnapshot {
    limit_short_label: Option<String>,
    limit_5h_text: Option<String>,
    limit_weekly_text: Option<String>,
    limit_monthly_text: Option<String>,
    limit_5h_reset_at: Option<i64>,
    limit_weekly_reset_at: Option<i64>,
    limit_monthly_reset_at: Option<i64>,
}

fn oauth_scopes() -> &'static str {
    "openid profile email offline_access api.connectors.read api.connectors.invoke"
}

fn ensure_persisted_provider_id(provider_id: &str) -> Result<(), String> {
    if provider_id == CODEX_LOCAL_PROVIDER_ID {
        Err("The local Codex provider cannot manage official accounts".to_string())
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

fn generate_pkce_pair() -> (String, String) {
    let code_verifier = generate_random_urlsafe(32);
    let challenge_hash = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(challenge_hash);
    (code_verifier, code_challenge)
}

fn build_oauth_redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}{CODEX_OAUTH_CALLBACK_PATH}")
}

fn build_codex_authorize_url(redirect_uri: &str, state: &str, code_challenge: &str) -> String {
    let mut authorize_url = format!(
        "{CODEX_OAUTH_AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        encode_url_component(CODEX_OAUTH_CLIENT_ID),
        encode_url_component(redirect_uri),
        encode_url_component(oauth_scopes()),
        encode_url_component(state),
        encode_url_component(code_challenge),
    );
    let extra_params = [
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", "codex_cli_rs"),
    ];
    for (key, value) in extra_params {
        authorize_url.push('&');
        authorize_url.push_str(&encode_url_component(key));
        authorize_url.push('=');
        authorize_url.push_str(&encode_url_component(value));
    }
    authorize_url
}

fn open_browser(url: &str) -> Result<(), String> {
    tauri_plugin_opener::open_url(url, None::<&str>)
        .map_err(|error| format!("Failed to open OAuth login page: {error}"))
}

fn bind_oauth_listener() -> Result<(TcpListener, u16), String> {
    let listener = TcpListener::bind(("127.0.0.1", CODEX_OAUTH_DEFAULT_PORT))
        .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
        .map_err(|error| format!("Failed to bind Codex OAuth callback listener: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("Failed to resolve Codex OAuth callback port: {error}"))?
        .port();
    Ok((listener, port))
}

fn wait_for_oauth_callback(listener: TcpListener, state: &str) -> Result<String, String> {
    wait_for_oauth_callback_with_timeout(listener, state, Duration::from_secs(300))
}

fn wait_for_oauth_callback_with_timeout(
    listener: TcpListener,
    state: &str,
    timeout: Duration,
) -> Result<String, String> {
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("Failed to configure OAuth listener: {error}"))?;
    listener
        .set_ttl(64)
        .map_err(|error| format!("Failed to configure OAuth listener ttl: {error}"))?;
    let start = std::time::Instant::now();
    let (mut stream, _) = loop {
        match listener.accept() {
            Ok(connection) => break connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() >= timeout {
                    return Err("Timed out waiting for OAuth callback".to_string());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(format!("Failed to receive OAuth callback: {error}")),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| format!("Failed to set OAuth callback stream timeout: {error}"))?;

    let mut request_buffer = [0u8; 8192];
    let read_size = stream
        .read(&mut request_buffer)
        .map_err(|error| format!("Failed to read OAuth callback request: {error}"))?;
    let request = String::from_utf8_lossy(&request_buffer[..read_size]);
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| "OAuth callback request is empty".to_string())?;
    let request_path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "OAuth callback request line is invalid".to_string())?;
    let query_string = request_path
        .split_once('?')
        .map(|(_, query)| query)
        .ok_or_else(|| "OAuth callback is missing query string".to_string())?;
    let query_params = parse_query_string(query_string);

    let response_body = if let Some(error) = query_params.get("error") {
        format!("OAuth login failed: {error}")
    } else {
        "OAuth login completed. You can return to AI Toolbox.".to_string()
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();

    if let Some(error) = query_params.get("error") {
        return Err(format!("OAuth authorize failed: {error}"));
    }

    let returned_state = query_params
        .get("state")
        .ok_or_else(|| "OAuth callback missing state".to_string())?;
    if returned_state != state {
        return Err("OAuth callback state mismatch".to_string());
    }

    query_params
        .get("code")
        .cloned()
        .ok_or_else(|| "OAuth callback missing authorization code".to_string())
}

fn parse_query_string(query: &str) -> BTreeMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((url_decode(key), url_decode(value)))
        })
        .collect()
}

fn url_decode(value: &str) -> String {
    let mut bytes = Vec::with_capacity(value.len());
    let mut chars = value.as_bytes().iter().copied().peekable();
    while let Some(byte) = chars.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let high = chars.next();
                let low = chars.next();
                if let (Some(high), Some(low)) = (high, low) {
                    if let Ok(decoded) =
                        u8::from_str_radix(&String::from_utf8_lossy(&[high, low]), 16)
                    {
                        bytes.push(decoded);
                    }
                }
            }
            _ => bytes.push(byte),
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

async fn exchange_authorization_code(
    db: &SqliteDbState,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<OAuthTokenResponse, String> {
    let client = crate::http_client::client_with_timeout(db, 20).await?;

    client
        .post(CODEX_OAUTH_TOKEN_URL)
        .form(&OAuthTokenRequest {
            grant_type: "authorization_code",
            client_id: CODEX_OAUTH_CLIENT_ID,
            code,
            redirect_uri,
            code_verifier,
        })
        .send()
        .await
        .map_err(|error| format!("Failed to exchange authorization code: {error}"))?
        .error_for_status()
        .map_err(|error| format!("OAuth token exchange failed: {error}"))?
        .json::<OAuthTokenResponse>()
        .await
        .map_err(|error| format!("Failed to parse OAuth token response: {error}"))
}

async fn refresh_oauth_token(
    db: &SqliteDbState,
    refresh_token: &str,
) -> Result<OAuthTokenResponse, String> {
    let client = crate::http_client::client_with_timeout(db, 20).await?;
    let response = client
        .post(CODEX_OAUTH_TOKEN_URL)
        .form(&OAuthRefreshRequest {
            grant_type: "refresh_token",
            client_id: CODEX_OAUTH_CLIENT_ID,
            refresh_token,
        })
        .send()
        .await
        .map_err(|error| format!("Failed to refresh OAuth token: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed to read OAuth refresh response: {error}"))?;
    if !status.is_success() {
        let error_code = serde_json::from_str::<Value>(&body).ok().and_then(|value| {
            value
                .get("code")
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        if error_code.as_deref() == Some("refresh_token_reused")
            || body.contains("refresh_token_reused")
        {
            return Err(
                "refresh_token_reused: Codex refresh token has already been rotated; sign in again"
                    .to_string(),
            );
        }
        return Err(format!("OAuth token refresh failed ({status}): {body}"));
    }
    serde_json::from_str::<OAuthTokenResponse>(&body)
        .map_err(|error| format!("Failed to parse refreshed OAuth token response: {error}"))
}

fn decode_jwt_payload(value: &str) -> Result<JwtClaims, String> {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid JWT format".to_string());
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|error| format!("Failed to decode JWT payload: {error}"))?;
    serde_json::from_slice::<JwtClaims>(&payload)
        .map_err(|error| format!("Failed to parse JWT payload: {error}"))
}

fn parse_id_token(id_token: &str) -> ParsedIdToken {
    let claims = decode_jwt_payload(id_token).unwrap_or_default();
    let auth = claims.auth.unwrap_or_default();

    ParsedIdToken {
        email: claims
            .email
            .or_else(|| claims.profile.and_then(|profile| profile.email)),
        plan_type: auth.chatgpt_plan_type,
        account_id: auth.chatgpt_account_id,
        chatgpt_user_id: auth.chatgpt_user_id.or(auth.user_id),
        chatgpt_account_is_fedramp: auth.chatgpt_account_is_fedramp,
    }
}

fn build_auth_snapshot(
    token_response: &OAuthTokenResponse,
    existing_auth: Option<&Value>,
) -> Result<Value, String> {
    let existing_tokens = existing_auth.and_then(|value| value.get("tokens"));
    let id_token = token_response
        .id_token
        .as_deref()
        .or_else(|| {
            existing_tokens
                .and_then(|tokens| tokens.get("id_token"))
                .and_then(Value::as_str)
        })
        .ok_or_else(|| "OAuth response missing id_token".to_string())?;
    let parsed = parse_id_token(id_token);
    let account_id = parsed
        .account_id
        .clone()
        .ok_or_else(|| "OAuth response missing ChatGPT account id".to_string())?;

    let mut auth_object = existing_auth
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();

    auth_object.insert(
        "auth_mode".to_string(),
        Value::String("chatgpt".to_string()),
    );
    auth_object.insert(
        "tokens".to_string(),
        serde_json::json!({
            "id_token": id_token,
            "access_token": token_response.access_token,
            "refresh_token": token_response.refresh_token.clone().or_else(|| {
                existing_tokens
                    .and_then(|tokens| tokens.get("refresh_token"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            }).unwrap_or_default(),
            "account_id": account_id,
        }),
    );
    auth_object.insert(
        "last_refresh".to_string(),
        Value::String(Local::now().to_rfc3339()),
    );
    auth_object.remove("OPENAI_API_KEY");
    auth_object.remove("agent_identity");
    let _ = parsed.chatgpt_user_id;
    let _ = parsed.chatgpt_account_is_fedramp;

    Ok(Value::Object(auth_object))
}

pub(super) fn auth_has_official_runtime(auth: &Value) -> bool {
    let auth_mode = auth
        .get("auth_mode")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let access_token = auth
        .pointer("/tokens/access_token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let refresh_token = auth
        .pointer("/tokens/refresh_token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    auth_mode.is_some() && access_token.is_some() && refresh_token.is_some()
}

fn token_expiration_unix(token: &str) -> Option<i64> {
    decode_jwt_payload(token).ok()?.exp
}

fn official_auth_needs_refresh(auth: &Value) -> bool {
    let now = chrono::Utc::now().timestamp();
    let expiration = auth
        .pointer("/tokens/access_token")
        .and_then(|value| value.as_str())
        .and_then(token_expiration_unix)
        .or_else(|| {
            auth.pointer("/tokens/id_token")
                .and_then(|value| value.as_str())
                .and_then(token_expiration_unix)
        });

    matches!(expiration, Some(expiration) if expiration <= now + AUTH_REFRESH_LEAD_SECONDS)
}

async fn ensure_fresh_official_runtime_auth(
    db: &SqliteDbState,
    auth: &Value,
) -> Result<Value, String> {
    if !official_auth_needs_refresh(auth) {
        return Ok(auth.clone());
    }

    let _refresh_guard = OAUTH_REFRESH_LOCK.lock().await;
    if !official_auth_needs_refresh(auth) {
        return Ok(auth.clone());
    }
    let refresh_token = auth
        .pointer("/tokens/refresh_token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Official account is missing refresh token".to_string())?;
    let cached_response = OAUTH_REFRESH_CACHE
        .lock()
        .await
        .get(refresh_token)
        .filter(|(created_at, _)| created_at.elapsed() <= Duration::from_secs(30))
        .map(|(_, response)| response.clone());
    let refreshed_token_response = match cached_response {
        Some(response) => response,
        None => {
            let response = refresh_oauth_token(db, refresh_token).await?;
            OAUTH_REFRESH_CACHE.lock().await.insert(
                refresh_token.to_string(),
                (std::time::Instant::now(), response.clone()),
            );
            response
        }
    };
    build_auth_snapshot(&refreshed_token_response, Some(auth))
}

fn auth_json_from_snapshot(snapshot: &str) -> Result<Value, String> {
    serde_json::from_str(snapshot)
        .map_err(|error| format!("Failed to parse account snapshot: {error}"))
}

async fn read_auth_json_from_disk(db: Option<&crate::db::SqliteDbState>) -> Result<Value, String> {
    let root_dir = if let Some(db) = db {
        get_codex_root_dir_from_db_async(db).await?
    } else {
        get_codex_root_dir_without_db()?
    };
    let auth_path = root_dir.join("auth.json");
    if !auth_path.exists() {
        return Ok(serde_json::json!({}));
    }
    let content = fs::read_to_string(&auth_path)
        .map_err(|error| format!("Failed to read auth.json: {error}"))?;
    serde_json::from_str(&content).map_err(|error| format!("Failed to parse auth.json: {error}"))
}

async fn write_auth_json_to_disk(
    db: &crate::db::SqliteDbState,
    auth: &Value,
) -> Result<(), String> {
    let root_dir = get_codex_root_dir_from_db_async(db).await?;
    if !root_dir.exists() {
        fs::create_dir_all(&root_dir)
            .map_err(|error| format!("Failed to create Codex root directory: {error}"))?;
    }
    let auth_path = root_dir.join("auth.json");
    write_auth_json_atomic(&auth_path, auth)
}

fn write_auth_json_atomic(auth_path: &std::path::Path, auth: &Value) -> Result<(), String> {
    let root_dir = auth_path
        .parent()
        .ok_or_else(|| "Codex auth.json path has no parent directory".to_string())?;
    let mut temporary = NamedTempFile::new_in(&root_dir)
        .map_err(|error| format!("Failed to create temporary auth.json: {error}"))?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), auth)
        .map_err(|error| format!("Failed to serialize auth.json: {error}"))?;
    temporary
        .write_all(b"\n")
        .map_err(|error| format!("Failed to finalize auth.json: {error}"))?;
    temporary
        .persist(&auth_path)
        .map_err(|error| format!("Failed to replace auth.json: {}", error.error))?;
    set_codex_auth_permissions(&auth_path)
}

#[cfg(unix)]
fn set_codex_auth_permissions(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("Failed to set auth.json permissions: {error}"))
}

#[cfg(not(unix))]
fn set_codex_auth_permissions(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

async fn query_provider(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<CodexProvider, String> {
    db.with_conn(|conn| db_get(conn, DbTable::CodexProvider, provider_id))?
        .map(adapter::from_db_value_provider)
        .ok_or_else(|| format!("Codex provider '{}' not found", provider_id))
}

async fn list_persisted_official_accounts(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<Vec<CodexOfficialAccount>, String> {
    let provider_id_value = Value::String(provider_id.to_string());
    let provider_id_path = JsonFieldPath::new("provider_id")?;
    let order = OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::json_text("created_at", OrderDirection::Asc)?,
    ]);
    db.with_conn(|conn| {
        Ok(db_query_by_field(
            conn,
            DbTable::CodexOfficialAccount,
            &provider_id_path,
            &provider_id_value,
            Some(&order),
            None,
        )?
        .into_iter()
        .map(adapter::from_db_value_official_account)
        .collect())
    })
}

fn build_virtual_local_account(auth: &Value) -> CodexOfficialAccount {
    let parsed = auth
        .pointer("/tokens/id_token")
        .and_then(|value| value.as_str())
        .map(parse_id_token)
        .unwrap_or_default();
    let now = Local::now().to_rfc3339();

    CodexOfficialAccount {
        id: LOCAL_OFFICIAL_ACCOUNT_ID.to_string(),
        provider_id: String::new(),
        name: LOCAL_OFFICIAL_ACCOUNT_ID.to_string(),
        kind: "local".to_string(),
        email: parsed.email.clone(),
        auth_snapshot: Some(auth.to_string()),
        auth_mode: auth
            .get("auth_mode")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        account_id: auth
            .pointer("/tokens/account_id")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .or(parsed.account_id.clone()),
        plan_type: parsed.plan_type.clone(),
        last_refresh: auth
            .get("last_refresh")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        token_expires_at: auth
            .pointer("/tokens/access_token")
            .and_then(|value| value.as_str())
            .and_then(token_expiration_unix)
            .or_else(|| {
                auth.pointer("/tokens/id_token")
                    .and_then(|value| value.as_str())
                    .and_then(token_expiration_unix)
            }),
        access_token_preview: auth
            .pointer("/tokens/access_token")
            .and_then(|value| value.as_str())
            .and_then(|value| {
                let trimmed = value.trim();
                let char_count = trimmed.chars().count();
                if trimmed.is_empty() {
                    None
                } else if char_count <= 12 {
                    Some(trimmed.to_string())
                } else {
                    let head: String = trimmed.chars().take(6).collect();
                    let tail: String = trimmed
                        .chars()
                        .rev()
                        .take(6)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    Some(format!("{head}...{tail}"))
                }
            }),
        refresh_token_preview: auth
            .pointer("/tokens/refresh_token")
            .and_then(|value| value.as_str())
            .and_then(|value| {
                let trimmed = value.trim();
                let char_count = trimmed.chars().count();
                if trimmed.is_empty() {
                    None
                } else if char_count <= 12 {
                    Some(trimmed.to_string())
                } else {
                    let head: String = trimmed.chars().take(6).collect();
                    let tail: String = trimmed
                        .chars()
                        .rev()
                        .take(6)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    Some(format!("{head}...{tail}"))
                }
            }),
        limit_short_label: None,
        limit_5h_text: None,
        limit_weekly_text: None,
        limit_monthly_text: None,
        limit_5h_reset_at: None,
        limit_weekly_reset_at: None,
        limit_monthly_reset_at: None,
        last_limits_fetched_at: None,
        last_error: None,
        sort_index: None,
        is_applied: false,
        is_virtual: true,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn official_account_identity_matches_auth(account: &CodexOfficialAccount, auth: &Value) -> bool {
    let local_refresh_token = auth
        .pointer("/tokens/refresh_token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let local_account_id = auth
        .pointer("/tokens/account_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            auth.pointer("/tokens/id_token")
                .and_then(|value| value.as_str())
                .map(parse_id_token)
                .and_then(|parsed| parsed.account_id)
        });
    let local_email = auth
        .pointer("/tokens/id_token")
        .and_then(|value| value.as_str())
        .map(parse_id_token)
        .and_then(|parsed| parsed.email)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());

    let account_refresh_token = account
        .auth_snapshot
        .as_deref()
        .and_then(|snapshot| serde_json::from_str::<Value>(snapshot).ok())
        .and_then(|snapshot| {
            snapshot
                .pointer("/tokens/refresh_token")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });

    if let (Some(local_refresh_token), Some(account_refresh_token)) =
        (local_refresh_token, account_refresh_token.as_deref())
    {
        if local_refresh_token == account_refresh_token {
            return true;
        }
    }

    if let (Some(local_account_id), Some(account_account_id)) =
        (local_account_id.as_deref(), account.account_id.as_deref())
    {
        if local_account_id == account_account_id {
            return true;
        }
    }

    if let (Some(local_email), Some(account_email)) =
        (local_email.as_deref(), account.email.as_deref())
    {
        if local_email == account_email.trim().to_ascii_lowercase() {
            return true;
        }
    }

    false
}

fn should_show_virtual_local_account(
    persisted_accounts: &[CodexOfficialAccount],
    local_auth: &Value,
) -> bool {
    if !auth_has_official_runtime(local_auth) {
        return false;
    }

    !persisted_accounts
        .iter()
        .any(|account| official_account_identity_matches_auth(account, local_auth))
}

fn parse_remaining_percent_from_window(window: &Value) -> Option<f64> {
    if !window.is_object() {
        return None;
    }
    if let Some(used_percent) = window
        .get("used_percent")
        .and_then(Value::as_f64)
        .or_else(|| window.get("usedPercent").and_then(Value::as_f64))
    {
        return Some((100.0 - used_percent).clamp(0.0, 100.0));
    }

    let remaining = window
        .get("remaining_count")
        .and_then(Value::as_f64)
        .or_else(|| window.get("remainingCount").and_then(Value::as_f64));
    let total = window
        .get("total_count")
        .and_then(Value::as_f64)
        .or_else(|| window.get("totalCount").and_then(Value::as_f64));
    match (remaining, total) {
        (Some(remaining), Some(total)) if total > 0.0 => {
            Some((remaining / total * 100.0).clamp(0.0, 100.0))
        }
        _ => None,
    }
}

fn parse_remaining_percent_from_rate_window(window: RateWindowRef<'_>) -> Option<f64> {
    parse_remaining_percent_from_window(window.value).or_else(|| {
        let exhausted = window.limit_reached.unwrap_or(false) || window.allowed == Some(false);
        if exhausted && extract_reset_timestamp(window.value).is_some() {
            Some(0.0)
        } else {
            None
        }
    })
}

fn format_percent_label(value: f64) -> String {
    format!("{:.0}%", value.clamp(0.0, 100.0))
}

#[derive(Debug, Default)]
struct ResolvedRateWindows<'a> {
    primary: Option<RateWindowRef<'a>>,
    secondary: Option<RateWindowRef<'a>>,
    monthly: Option<RateWindowRef<'a>>,
    all: Vec<RateWindowRef<'a>>,
}

#[derive(Debug, Clone, Copy)]
struct RateLimitSource<'a> {
    value: &'a Value,
    limit_reached: Option<bool>,
    allowed: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
struct RateWindowRef<'a> {
    value: &'a Value,
    limit_reached: Option<bool>,
    allowed: Option<bool>,
}

fn looks_like_rate_window(value: &Value) -> bool {
    if !value.is_object() {
        return false;
    }

    extract_limit_window_seconds(value).is_some()
        || value.get("used_percent").is_some()
        || value.get("usedPercent").is_some()
        || value.get("remaining_count").is_some()
        || value.get("remainingCount").is_some()
        || value.get("total_count").is_some()
        || value.get("totalCount").is_some()
        || value.get("reset_at").is_some()
        || value.get("resetAt").is_some()
        || value.get("resets_at").is_some()
        || value.get("resetsAt").is_some()
        || value.get("reset_after_seconds").is_some()
        || value.get("resetAfterSeconds").is_some()
        || value.get("resets_in_seconds").is_some()
        || value.get("resetsInSeconds").is_some()
}

fn json_bool(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(value)) => Some(*value),
        Some(Value::String(value)) => match value.trim().to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn rate_limit_source_from_value(value: &Value) -> RateLimitSource<'_> {
    RateLimitSource {
        value,
        limit_reached: json_bool(
            value
                .get("limit_reached")
                .or_else(|| value.get("limitReached")),
        ),
        allowed: json_bool(value.get("allowed")),
    }
}

fn push_unique_rate_limit_source<'a>(
    sources: &mut Vec<RateLimitSource<'a>>,
    source: Option<&'a Value>,
) {
    if let Some(source) = source.filter(|value| value.is_object()) {
        if !sources
            .iter()
            .any(|existing| std::ptr::eq(existing.value, source))
        {
            sources.push(rate_limit_source_from_value(source));
        }
    }
}

fn first_rate_window_with_aliases<'a>(
    source: RateLimitSource<'a>,
    aliases: &[&str],
) -> Option<RateWindowRef<'a>> {
    aliases
        .iter()
        .find_map(|key| source.value.get(*key))
        .filter(|value| looks_like_rate_window(value))
        .map(|value| RateWindowRef {
            value,
            limit_reached: source.limit_reached,
            allowed: source.allowed,
        })
}

fn push_unique_rate_window<'a>(
    windows: &mut Vec<RateWindowRef<'a>>,
    window: Option<RateWindowRef<'a>>,
) {
    if let Some(window) = window.filter(|window| looks_like_rate_window(window.value)) {
        if !windows
            .iter()
            .any(|existing| std::ptr::eq(existing.value, window.value))
        {
            windows.push(window);
        }
    }
}

fn resolve_rate_windows(body: &Value) -> ResolvedRateWindows<'_> {
    let rate_limit = body
        .get("rate_limit")
        .or_else(|| body.get("rateLimit"))
        .unwrap_or(body);
    let mut sources = Vec::new();
    push_unique_rate_limit_source(&mut sources, Some(rate_limit));
    if !std::ptr::eq(rate_limit, body) {
        push_unique_rate_limit_source(&mut sources, Some(body));
    }
    if let Some(additional_rate_limits) = body
        .get("additional_rate_limits")
        .or_else(|| body.get("additionalRateLimits"))
        .and_then(Value::as_array)
    {
        for entry in additional_rate_limits {
            if let Some(entry_rate_limit) = entry
                .get("rate_limit")
                .or_else(|| entry.get("rateLimit"))
                .filter(|value| value.is_object())
            {
                push_unique_rate_limit_source(&mut sources, Some(entry_rate_limit));
            } else {
                push_unique_rate_limit_source(&mut sources, Some(entry));
            }
        }
    }

    let primary_aliases = [
        "primary_window",
        "primaryWindow",
        "five_hour",
        "5_hour_window",
        "fiveHourWindow",
    ];
    let secondary_aliases = [
        "secondary_window",
        "secondaryWindow",
        "seven_day",
        "weekly_window",
        "weeklyWindow",
    ];
    let monthly_aliases = [
        "monthly_window",
        "monthlyWindow",
        "month_window",
        "monthWindow",
        "thirty_day",
        "thirtyDay",
        "thirty_day_window",
        "thirtyDayWindow",
    ];
    let all_aliases = [
        "primary_window",
        "primaryWindow",
        "secondary_window",
        "secondaryWindow",
        "five_hour",
        "5_hour_window",
        "fiveHourWindow",
        "seven_day",
        "weekly_window",
        "weeklyWindow",
        "monthly_window",
        "monthlyWindow",
        "month_window",
        "monthWindow",
        "thirty_day",
        "thirtyDay",
        "thirty_day_window",
        "thirtyDayWindow",
    ];

    let primary = sources
        .iter()
        .find_map(|source| first_rate_window_with_aliases(*source, &primary_aliases));
    let secondary = sources
        .iter()
        .find_map(|source| first_rate_window_with_aliases(*source, &secondary_aliases));
    let monthly = sources
        .iter()
        .find_map(|source| first_rate_window_with_aliases(*source, &monthly_aliases));

    let mut all = Vec::new();
    push_unique_rate_window(&mut all, primary);
    push_unique_rate_window(&mut all, secondary);
    push_unique_rate_window(&mut all, monthly);
    for source in sources {
        for key in all_aliases {
            push_unique_rate_window(
                &mut all,
                source.value.get(key).map(|value| RateWindowRef {
                    value,
                    limit_reached: source.limit_reached,
                    allowed: source.allowed,
                }),
            );
        }
        if let Some(object) = source.value.as_object() {
            for value in object.values() {
                push_unique_rate_window(
                    &mut all,
                    Some(RateWindowRef {
                        value,
                        limit_reached: source.limit_reached,
                        allowed: source.allowed,
                    }),
                );
            }
        }
    }

    ResolvedRateWindows {
        primary,
        secondary,
        monthly,
        all,
    }
}

fn extract_limit_window_seconds(window: &Value) -> Option<i64> {
    window
        .get("limit_window_seconds")
        .or_else(|| window.get("limitWindowSeconds"))
        .and_then(Value::as_i64)
}

fn is_month_window_seconds(seconds: i64) -> bool {
    (MONTH_WINDOW_MIN_SECONDS..=MONTH_WINDOW_MAX_SECONDS).contains(&seconds)
}

fn is_selected_window(candidate: RateWindowRef<'_>, selected: Option<RateWindowRef<'_>>) -> bool {
    match selected {
        Some(selected) => std::ptr::eq(candidate.value, selected.value),
        None => false,
    }
}

fn classify_rate_windows(
    body: &Value,
) -> (
    Option<RateWindowRef<'_>>,
    Option<RateWindowRef<'_>>,
    Option<RateWindowRef<'_>>,
) {
    let resolved_windows = resolve_rate_windows(body);
    let mut short_window = None;
    let mut weekly_window = None;
    let mut monthly_window = None;

    for window in resolved_windows.all.iter().copied() {
        match extract_limit_window_seconds(window.value) {
            Some(FIVE_HOUR_WINDOW_SECONDS) if short_window.is_none() => {
                short_window = Some(window);
            }
            Some(WEEK_WINDOW_SECONDS) if weekly_window.is_none() => {
                weekly_window = Some(window);
            }
            Some(seconds) if is_month_window_seconds(seconds) && monthly_window.is_none() => {
                monthly_window = Some(window);
            }
            _ => {}
        }
    }

    if monthly_window.is_none() {
        monthly_window = resolved_windows.monthly;
    }
    if short_window.is_none() {
        short_window = resolved_windows.primary.filter(|window| {
            !is_selected_window(*window, weekly_window)
                && !is_selected_window(*window, monthly_window)
        });
    }
    if weekly_window.is_none() {
        weekly_window = resolved_windows.secondary.filter(|window| {
            !is_selected_window(*window, short_window)
                && !is_selected_window(*window, monthly_window)
        });
    }

    (short_window, weekly_window, monthly_window)
}

fn extract_reset_timestamp_at(window: &Value, now_timestamp: i64) -> Option<i64> {
    if let Some(reset_at) = window
        .get("reset_at")
        .or_else(|| window.get("resetAt"))
        .or_else(|| window.get("resets_at"))
        .or_else(|| window.get("resetsAt"))
        .and_then(Value::as_i64)
    {
        return Some(reset_at);
    }

    window
        .get("reset_after_seconds")
        .or_else(|| window.get("resetAfterSeconds"))
        .or_else(|| window.get("resets_in_seconds"))
        .or_else(|| window.get("resetsInSeconds"))
        .and_then(Value::as_i64)
        .filter(|seconds| *seconds > 0)
        .and_then(|seconds| now_timestamp.checked_add(seconds))
}

fn extract_reset_timestamp(window: &Value) -> Option<i64> {
    extract_reset_timestamp_at(window, Local::now().timestamp())
}

fn plan_type_has_short_window(plan_type: Option<&str>) -> bool {
    !matches!(
        plan_type
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("free")
    )
}

fn usage_plan_type_from_auth(auth_snapshot: &Value) -> Option<String> {
    auth_snapshot
        .pointer("/tokens/id_token")
        .and_then(|value| value.as_str())
        .map(parse_id_token)
        .and_then(|parsed| parsed.plan_type)
}

fn usage_account_id_from_auth(auth_snapshot: &Value) -> Option<String> {
    auth_snapshot
        .pointer("/tokens/account_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            auth_snapshot
                .pointer("/tokens/id_token")
                .and_then(|value| value.as_str())
                .map(parse_id_token)
                .and_then(|parsed| parsed.account_id)
        })
}

fn parse_usage_snapshot(body: &Value, plan_type: Option<&str>) -> UsageSnapshot {
    let (short_window, weekly_window, monthly_window) = classify_rate_windows(body);
    let has_short_window = plan_type_has_short_window(plan_type);
    let effective_weekly_window = if has_short_window {
        weekly_window
    } else {
        weekly_window.or(short_window)
    };

    UsageSnapshot {
        limit_short_label: has_short_window.then(|| "5h".to_string()),
        limit_5h_text: if has_short_window {
            short_window
                .and_then(parse_remaining_percent_from_rate_window)
                .map(format_percent_label)
        } else {
            None
        },
        limit_weekly_text: effective_weekly_window
            .and_then(parse_remaining_percent_from_rate_window)
            .map(format_percent_label),
        limit_monthly_text: monthly_window
            .and_then(parse_remaining_percent_from_rate_window)
            .map(format_percent_label),
        limit_5h_reset_at: if has_short_window {
            short_window.and_then(|window| extract_reset_timestamp(window.value))
        } else {
            None
        },
        limit_weekly_reset_at: effective_weekly_window
            .and_then(|window| extract_reset_timestamp(window.value)),
        limit_monthly_reset_at: monthly_window
            .and_then(|window| extract_reset_timestamp(window.value)),
    }
}

async fn fetch_usage_snapshot(
    db: &crate::db::SqliteDbState,
    access_token: &str,
    account_id: Option<&str>,
    plan_type: Option<&str>,
) -> Result<UsageSnapshot, String> {
    let client = crate::http_client::client_with_timeout(db, 20).await?;

    let mut request = client
        .get(CODEX_USAGE_URL)
        .header("Authorization", format!("Bearer {access_token}"))
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AI-Toolbox Codex OAuth",
        )
        .header("Content-Type", "application/json");
    if let Some(account_id) = account_id
        .map(str::trim)
        .filter(|account_id| !account_id.is_empty())
    {
        request = request.header("Chatgpt-Account-Id", account_id);
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("Failed to fetch usage: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Codex usage request failed: {error}"))?;

    let body = response
        .json::<Value>()
        .await
        .map_err(|error| format!("Failed to parse usage response: {error}"))?;
    Ok(parse_usage_snapshot(&body, plan_type))
}

fn build_account_content_from_auth_snapshot(
    provider_id: &str,
    auth_snapshot: &Value,
    usage_snapshot: Option<&UsageSnapshot>,
    name_override: Option<&str>,
) -> Result<CodexOfficialAccountContent, String> {
    let parsed = auth_snapshot
        .pointer("/tokens/id_token")
        .and_then(|value| value.as_str())
        .map(parse_id_token)
        .unwrap_or_default();
    let now = Local::now().to_rfc3339();
    let account_id = auth_snapshot
        .pointer("/tokens/account_id")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .or(parsed.account_id.clone());
    let email = parsed.email.clone();
    let display_name = name_override
        .map(|value| value.to_string())
        .or_else(|| email.clone())
        .or_else(|| account_id.clone())
        .unwrap_or_else(|| "official-account".to_string());

    Ok(CodexOfficialAccountContent {
        provider_id: provider_id.to_string(),
        name: display_name,
        kind: "oauth".to_string(),
        email,
        auth_snapshot: serde_json::to_string(auth_snapshot)
            .map_err(|error| format!("Failed to serialize account auth snapshot: {error}"))?,
        auth_mode: auth_snapshot
            .get("auth_mode")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        account_id,
        plan_type: parsed.plan_type,
        last_refresh: auth_snapshot
            .get("last_refresh")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        limit_short_label: usage_snapshot.and_then(|snapshot| snapshot.limit_short_label.clone()),
        limit_5h_text: usage_snapshot.and_then(|snapshot| snapshot.limit_5h_text.clone()),
        limit_weekly_text: usage_snapshot.and_then(|snapshot| snapshot.limit_weekly_text.clone()),
        limit_monthly_text: usage_snapshot.and_then(|snapshot| snapshot.limit_monthly_text.clone()),
        limit_5h_reset_at: usage_snapshot.and_then(|snapshot| snapshot.limit_5h_reset_at),
        limit_weekly_reset_at: usage_snapshot.and_then(|snapshot| snapshot.limit_weekly_reset_at),
        limit_monthly_reset_at: usage_snapshot.and_then(|snapshot| snapshot.limit_monthly_reset_at),
        last_limits_fetched_at: usage_snapshot.map(|_| now.clone()),
        last_error: None,
        sort_index: None,
        is_applied: false,
        created_at: now.clone(),
        updated_at: now,
    })
}

async fn save_official_account(
    db: &crate::db::SqliteDbState,
    content: &CodexOfficialAccountContent,
) -> Result<CodexOfficialAccount, String> {
    let account_id = db_new_id();
    let payload = adapter::to_db_value_official_account(content);
    db.with_conn(|conn| db_put(conn, DbTable::CodexOfficialAccount, &account_id, &payload))?;
    load_official_account(db, &account_id).await
}

async fn find_matching_official_account(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
    auth: &Value,
) -> Result<Option<CodexOfficialAccount>, String> {
    let accounts = list_persisted_official_accounts(db, provider_id).await?;
    Ok(accounts
        .into_iter()
        .find(|account| official_account_identity_matches_auth(account, auth)))
}

async fn update_official_account_apply_status(
    db: &crate::db::SqliteDbState,
    account_id: Option<&str>,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CodexOfficialAccount, account_id, &now)
    })?;

    Ok(())
}

async fn load_official_account(
    db: &crate::db::SqliteDbState,
    account_id: &str,
) -> Result<CodexOfficialAccount, String> {
    db.with_conn(|conn| db_get(conn, DbTable::CodexOfficialAccount, account_id))?
        .map(adapter::from_db_value_official_account)
        .ok_or_else(|| format!("Official account '{}' not found", account_id))
}

async fn persist_usage_snapshot(
    db: &crate::db::SqliteDbState,
    account_id: &str,
    usage_snapshot: &UsageSnapshot,
    last_error: Option<&str>,
) -> Result<CodexOfficialAccount, String> {
    let now = Local::now().to_rfc3339();
    db.with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::CodexOfficialAccount,
            account_id,
            &[
                (
                    "limit_short_label",
                    usage_snapshot
                        .limit_short_label
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_5h_text",
                    usage_snapshot
                        .limit_5h_text
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_weekly_text",
                    usage_snapshot
                        .limit_weekly_text
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_monthly_text",
                    usage_snapshot
                        .limit_monthly_text
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_5h_reset_at",
                    usage_snapshot
                        .limit_5h_reset_at
                        .map(|value| serde_json::json!(value))
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_weekly_reset_at",
                    usage_snapshot
                        .limit_weekly_reset_at
                        .map(|value| serde_json::json!(value))
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_monthly_reset_at",
                    usage_snapshot
                        .limit_monthly_reset_at
                        .map(|value| serde_json::json!(value))
                        .unwrap_or(Value::Null),
                ),
                ("last_limits_fetched_at", Value::String(now.clone())),
                (
                    "last_error",
                    last_error
                        .map(|value| Value::String(value.to_string()))
                        .unwrap_or(Value::Null),
                ),
                ("updated_at", Value::String(now)),
            ],
        )
        .map(|_| ())
    })?;

    load_official_account(db, account_id).await
}

async fn persist_refreshed_account_snapshot(
    db: &crate::db::SqliteDbState,
    account: &CodexOfficialAccount,
    refreshed_snapshot: &Value,
) -> Result<CodexOfficialAccount, String> {
    let parsed_content = build_account_content_from_auth_snapshot(
        &account.provider_id,
        refreshed_snapshot,
        None,
        Some(&account.name),
    )?;
    let payload = adapter::to_db_value_official_account(&CodexOfficialAccountContent {
        is_applied: account.is_applied,
        created_at: account.created_at.clone(),
        updated_at: Local::now().to_rfc3339(),
        limit_short_label: account.limit_short_label.clone(),
        limit_5h_text: account.limit_5h_text.clone(),
        limit_weekly_text: account.limit_weekly_text.clone(),
        limit_monthly_text: account.limit_monthly_text.clone(),
        limit_5h_reset_at: account.limit_5h_reset_at,
        limit_weekly_reset_at: account.limit_weekly_reset_at,
        limit_monthly_reset_at: account.limit_monthly_reset_at,
        last_limits_fetched_at: account.last_limits_fetched_at.clone(),
        last_error: account.last_error.clone(),
        sort_index: account.sort_index,
        ..parsed_content
    });
    db.with_conn(|conn| db_put(conn, DbTable::CodexOfficialAccount, &account.id, &payload))?;

    load_official_account(db, &account.id).await
}

fn merge_official_runtime_auth(existing_auth: &Value, next_auth: &Value) -> Value {
    let mut merged = existing_auth.as_object().cloned().unwrap_or_default();
    merged.remove("OPENAI_API_KEY");

    for key in ["auth_mode", "last_refresh", "agent_identity"] {
        if let Some(value) = next_auth.get(key) {
            merged.insert(key.to_string(), value.clone());
        }
    }

    if let Some(next_tokens) = next_auth.get("tokens").and_then(Value::as_object) {
        let mut tokens = existing_auth
            .get("tokens")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, value) in next_tokens {
            tokens.insert(key.clone(), value.clone());
        }
        merged.insert("tokens".to_string(), Value::Object(tokens));
    }

    Value::Object(merged)
}

fn extract_token_from_auth_snapshot(auth: &Value, token_kind: &str) -> Result<String, String> {
    let token_pointer = match token_kind {
        "access" => "/tokens/access_token",
        "refresh" => "/tokens/refresh_token",
        _ => return Err("Unsupported official account token kind".to_string()),
    };
    auth.pointer(token_pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Official account {} token is missing", token_kind))
}

fn copy_text_to_clipboard(value: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("Failed to access system clipboard: {error}"))?;
    clipboard
        .set_text(value.to_string())
        .map_err(|error| format!("Failed to copy token to clipboard: {error}"))
}

fn assign_provider_id(
    mut account: CodexOfficialAccount,
    provider_id: &str,
) -> CodexOfficialAccount {
    account.provider_id = provider_id.to_string();
    account
}

pub async fn list_codex_official_accounts_for_provider(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<Vec<CodexOfficialAccount>, String> {
    ensure_persisted_provider_id(provider_id)?;
    let provider = query_provider(db, provider_id).await?;
    let mut accounts = list_persisted_official_accounts(db, provider_id).await?;
    let local_auth = read_auth_json_from_disk(Some(db)).await?;
    if provider.category == "official" && should_show_virtual_local_account(&accounts, &local_auth)
    {
        accounts.push(assign_provider_id(
            build_virtual_local_account(&local_auth),
            provider_id,
        ));
    }
    Ok(accounts)
}

#[tauri::command]
pub async fn list_codex_official_accounts(
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
) -> Result<Vec<CodexOfficialAccount>, String> {
    let db = state.db();
    list_codex_official_accounts_for_provider(&db, &provider_id).await
}

#[tauri::command]
pub async fn start_codex_official_account_device_auth(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<CodexDeviceAuthStartResult, String> {
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider(state.db(), &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can add official accounts".to_string());
    }
    if !DEVICE_AUTH_SESSIONS
        .lock()
        .map_err(|_| "Codex device auth session lock is poisoned".to_string())?
        .is_empty()
    {
        return Err("A Codex device authorization session is already active".to_string());
    }

    let client = crate::http_client::client_with_timeout(state.db(), 30).await?;
    let response = client
        .post(CODEX_DEVICE_USER_CODE_URL)
        .json(&serde_json::json!({ "client_id": CODEX_OAUTH_CLIENT_ID }))
        .send()
        .await
        .map_err(|error| format!("Failed to request Codex device code: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed to read Codex device code response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Codex device code request failed ({status}): {body}"
        ));
    }
    let device: CodexDeviceUserCodeResponse = serde_json::from_str(&body)
        .map_err(|error| format!("Failed to parse Codex device code response: {error}"))?;
    if device.device_auth_id.trim().is_empty() || device.user_code.trim().is_empty() {
        return Err("Codex device code response is incomplete".to_string());
    }
    let poll_interval_seconds = match &device.interval {
        Value::Number(value) => value.as_u64().unwrap_or(5),
        Value::String(value) => value.parse::<u64>().unwrap_or(5),
        _ => 5,
    }
    .max(1);
    let session_id = db_new_id();
    let expires_at = chrono::Utc::now().timestamp() + 15 * 60;
    let (cancel_sender, cancel_receiver) = watch::channel(false);
    DEVICE_AUTH_SESSIONS
        .lock()
        .map_err(|_| "Codex device auth session lock is poisoned".to_string())?
        .insert(session_id.clone(), cancel_sender);

    emit_codex_auth_status(&app, &session_id, "waiting_for_user", None, None);
    let poll_session_id = session_id.clone();
    let poll_user_code = device.user_code.clone();
    tauri::async_runtime::spawn(async move {
        poll_codex_device_auth(
            app,
            poll_session_id,
            provider_id,
            device.device_auth_id,
            poll_user_code,
            poll_interval_seconds,
            expires_at,
            cancel_receiver,
        )
        .await;
    });

    Ok(CodexDeviceAuthStartResult {
        session_id,
        verification_uri: CODEX_DEVICE_VERIFICATION_URL.to_string(),
        user_code: device.user_code,
        expires_at,
        poll_interval_seconds,
    })
}

#[tauri::command]
pub fn cancel_codex_official_account_device_auth(session_id: String) -> Result<(), String> {
    if let Some(sender) = DEVICE_AUTH_SESSIONS
        .lock()
        .map_err(|_| "Codex device auth session lock is poisoned".to_string())?
        .remove(&session_id)
    {
        let _ = sender.send(true);
    }
    Ok(())
}

async fn poll_codex_device_auth(
    app: tauri::AppHandle,
    session_id: String,
    provider_id: String,
    device_auth_id: String,
    user_code: String,
    poll_interval_seconds: u64,
    expires_at: i64,
    mut cancel_receiver: watch::Receiver<bool>,
) {
    let result = async {
        let db_state = app.state::<SqliteDbState>();
        let db = db_state.db();
        let client = crate::http_client::client_with_timeout(db, 30).await?;
        loop {
            if *cancel_receiver.borrow() {
                return Err("cancelled".to_string());
            }
            if chrono::Utc::now().timestamp() >= expires_at {
                return Err("expired".to_string());
            }
            let response = client
                .post(CODEX_DEVICE_TOKEN_URL)
                .json(&serde_json::json!({
                    "device_auth_id": device_auth_id,
                    "user_code": user_code,
                }))
                .send()
                .await
                .map_err(|error| format!("Failed to poll Codex device authorization: {error}"))?;
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|error| format!("Failed to read Codex device authorization: {error}"))?;
            if status.is_success() {
                let token: CodexDeviceTokenResponse = serde_json::from_str(&body)
                    .map_err(|error| format!("Invalid Codex device token response: {error}"))?;
                if token.authorization_code.trim().is_empty()
                    || token.code_verifier.trim().is_empty()
                {
                    return Err("Codex device token response is incomplete".to_string());
                }
                let existing_auth = read_auth_json_from_disk(Some(&db)).await?;
                let token_response = exchange_authorization_code(
                    db,
                    &token.authorization_code,
                    CODEX_DEVICE_REDIRECT_URI,
                    &token.code_verifier,
                )
                .await?;
                let auth_snapshot = build_auth_snapshot(&token_response, Some(&existing_auth))?;
                let content = build_account_content_from_auth_snapshot(
                    &provider_id,
                    &auth_snapshot,
                    None,
                    None,
                )?;
                let account = save_official_account(db, &content).await?;
                return Ok(account.id);
            }
            if status.as_u16() != 403 && status.as_u16() != 404 {
                return Err(format!(
                    "Codex device authorization failed ({status}): {body}"
                ));
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(poll_interval_seconds)) => {}
                changed = cancel_receiver.changed() => {
                    if changed.is_ok() && *cancel_receiver.borrow() {
                        return Err("cancelled".to_string());
                    }
                }
            }
        }
    }
    .await;

    DEVICE_AUTH_SESSIONS
        .lock()
        .ok()
        .and_then(|mut sessions| sessions.remove(&session_id));
    match result {
        Ok(account_id) => {
            emit_codex_auth_status(&app, &session_id, "authorized", None, Some(account_id));
            let _ = app.emit("config-changed", "window");
        }
        Err(error) if error == "cancelled" || error == "expired" => {
            emit_codex_auth_status(&app, &session_id, &error, None, None);
        }
        Err(error) => emit_codex_auth_status(&app, &session_id, "error", Some(error), None),
    }
}

fn emit_codex_auth_status(
    app: &tauri::AppHandle,
    session_id: &str,
    status: &str,
    message: Option<String>,
    account_id: Option<String>,
) {
    let _ = app.emit(
        "codex-auth-status",
        CodexAuthStatusEvent {
            session_id: session_id.to_string(),
            status: status.to_string(),
            message,
            account_id,
        },
    );
}

#[tauri::command]
pub async fn start_codex_official_account_oauth(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<CodexOfficialAccount, String> {
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can add official accounts".to_string());
    }

    let existing_auth = read_auth_json_from_disk(Some(&db)).await?;
    let oauth_state = generate_random_urlsafe(32);
    let (code_verifier, code_challenge) = generate_pkce_pair();
    let (oauth_listener, callback_port) = bind_oauth_listener()?;
    let redirect_uri = build_oauth_redirect_uri(callback_port);
    let authorize_url = build_codex_authorize_url(&redirect_uri, &oauth_state, &code_challenge);

    open_browser(&authorize_url)?;
    let authorization_code =
        tokio::task::spawn_blocking(move || wait_for_oauth_callback(oauth_listener, &oauth_state))
            .await
            .map_err(|error| format!("OAuth callback task failed: {error}"))??;
    let token_response =
        exchange_authorization_code(&db, &authorization_code, &redirect_uri, &code_verifier)
            .await?;
    let auth_snapshot = build_auth_snapshot(&token_response, Some(&existing_auth))?;
    let plan_type = usage_plan_type_from_auth(&auth_snapshot);
    let usage_account_id = usage_account_id_from_auth(&auth_snapshot);
    let usage_snapshot = fetch_usage_snapshot(
        &db,
        &token_response.access_token,
        usage_account_id.as_deref(),
        plan_type.as_deref(),
    )
    .await
    .ok();
    let content = build_account_content_from_auth_snapshot(
        &provider_id,
        &auth_snapshot,
        usage_snapshot.as_ref(),
        None,
    )?;
    let account = save_official_account(&db, &content).await?;

    let _ = app.emit("config-changed", "window");
    Ok(account)
}

#[tauri::command]
pub async fn save_codex_official_local_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<CodexOfficialAccount, String> {
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can save local official accounts".to_string());
    }

    let local_auth = read_auth_json_from_disk(Some(&db)).await?;
    if !auth_has_official_runtime(&local_auth) {
        return Err("Current local auth.json does not contain an official Codex login".to_string());
    }

    if let Some(existing_account) =
        find_matching_official_account(&db, &provider_id, &local_auth).await?
    {
        if provider.is_applied {
            update_official_account_apply_status(&db, Some(&existing_account.id)).await?;
        }
        let _ = app.emit("config-changed", "window");
        return load_official_account(&db, &existing_account.id).await;
    }

    let usage_plan_type = usage_plan_type_from_auth(&local_auth);
    let usage_account_id = usage_account_id_from_auth(&local_auth);
    let usage_snapshot = local_auth
        .pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|access_token| async {
            fetch_usage_snapshot(
                &db,
                access_token,
                usage_account_id.as_deref(),
                usage_plan_type.as_deref(),
            )
            .await
        });

    let usage_snapshot = match usage_snapshot {
        Some(request) => request.await.ok(),
        None => None,
    };

    let mut content = build_account_content_from_auth_snapshot(
        &provider_id,
        &local_auth,
        usage_snapshot.as_ref(),
        None,
    )?;
    content.is_applied = provider.is_applied;

    let account = save_official_account(&db, &content).await?;
    if provider.is_applied {
        update_official_account_apply_status(&db, Some(&account.id)).await?;
    }

    let _ = app.emit("config-changed", "window");
    load_official_account(&db, &account.id).await
}

#[tauri::command]
pub async fn apply_codex_official_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
    account_id: String,
) -> Result<(), String> {
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can apply official accounts".to_string());
    }
    if provider.is_disabled {
        return Err(format!(
            "Provider '{}' is disabled and cannot be applied",
            provider_id
        ));
    }

    if account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        let local_auth = read_auth_json_from_disk(Some(&db)).await?;
        if !auth_has_official_runtime(&local_auth) {
            return Err(
                "Current local auth.json does not contain an official Codex login".to_string(),
            );
        }
        let refreshed_auth = ensure_fresh_official_runtime_auth(&db, &local_auth).await?;
        let merged_auth = merge_official_runtime_auth(&local_auth, &refreshed_auth);
        apply_config_internal(&db, &app, &provider_id, false).await?;
        write_auth_json_to_disk(&db, &merged_auth).await?;
        update_official_account_apply_status(&db, None).await?;
    } else {
        let account = load_official_account(&db, &account_id).await?;
        if account.provider_id != provider_id {
            return Err("Official account does not belong to the selected provider".to_string());
        }
        let current_auth = read_auth_json_from_disk(Some(&db)).await?;
        let snapshot = account
            .auth_snapshot
            .as_deref()
            .ok_or_else(|| "Official account snapshot is missing".to_string())?;
        let snapshot_auth = auth_json_from_snapshot(snapshot)?;
        let refreshed_snapshot = ensure_fresh_official_runtime_auth(&db, &snapshot_auth).await?;
        if refreshed_snapshot != snapshot_auth {
            let _ = persist_refreshed_account_snapshot(&db, &account, &refreshed_snapshot).await?;
        }
        let merged_auth = merge_official_runtime_auth(&current_auth, &refreshed_snapshot);
        apply_config_internal(&db, &app, &provider_id, false).await?;
        write_auth_json_to_disk(&db, &merged_auth).await?;
        update_official_account_apply_status(&db, Some(&account_id)).await?;
    }

    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn delete_codex_official_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
    account_id: String,
) -> Result<(), String> {
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    if account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        return Err("The local official account cannot be deleted".to_string());
    }

    let account = load_official_account(&db, &account_id).await?;
    if account.provider_id != provider_id {
        return Err("Official account does not belong to the selected provider".to_string());
    }
    if account.is_applied {
        return Err("The applied official account cannot be deleted".to_string());
    }

    db.with_conn(|conn| db_delete(conn, DbTable::CodexOfficialAccount, &account_id).map(|_| ()))?;

    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn refresh_codex_official_account_limits(
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
    account_id: String,
) -> Result<CodexOfficialAccount, String> {
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can refresh official account usage".to_string());
    }

    if account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        let auth = read_auth_json_from_disk(Some(&db)).await?;
        if !auth_has_official_runtime(&auth) {
            return Err(
                "Current local auth.json does not contain an official Codex login".to_string(),
            );
        }
        let access_token = auth
            .pointer("/tokens/access_token")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Local official auth is missing access token".to_string())?;
        let plan_type = usage_plan_type_from_auth(&auth);
        let usage_account_id = usage_account_id_from_auth(&auth);
        let usage_snapshot = fetch_usage_snapshot(
            &db,
            access_token,
            usage_account_id.as_deref(),
            plan_type.as_deref(),
        )
        .await?;
        let mut account = assign_provider_id(build_virtual_local_account(&auth), &provider_id);
        account.limit_short_label = usage_snapshot.limit_short_label;
        account.limit_5h_text = usage_snapshot.limit_5h_text;
        account.limit_weekly_text = usage_snapshot.limit_weekly_text;
        account.limit_monthly_text = usage_snapshot.limit_monthly_text;
        account.limit_5h_reset_at = usage_snapshot.limit_5h_reset_at;
        account.limit_weekly_reset_at = usage_snapshot.limit_weekly_reset_at;
        account.limit_monthly_reset_at = usage_snapshot.limit_monthly_reset_at;
        account.last_limits_fetched_at = Some(Local::now().to_rfc3339());
        return Ok(account);
    }

    let account = load_official_account(&db, &account_id).await?;
    if account.provider_id != provider_id {
        return Err("Official account does not belong to the selected provider".to_string());
    }
    let snapshot = account
        .auth_snapshot
        .as_deref()
        .ok_or_else(|| "Official account snapshot is missing".to_string())?;
    let auth_snapshot = auth_json_from_snapshot(snapshot)?;
    let access_token = auth_snapshot
        .pointer("/tokens/access_token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Official account is missing access token".to_string())?;
    let usage_account_id = account
        .account_id
        .clone()
        .or_else(|| usage_account_id_from_auth(&auth_snapshot));
    let usage_snapshot = fetch_usage_snapshot(
        &db,
        access_token,
        usage_account_id.as_deref(),
        account.plan_type.as_deref(),
    )
    .await?;

    persist_usage_snapshot(&db, &account_id, &usage_snapshot, None).await
}

#[tauri::command]
pub async fn copy_codex_official_account_token(
    state: tauri::State<'_, SqliteDbState>,
    input: CodexOfficialAccountTokenCopyInput,
) -> Result<(), String> {
    let db = state.db();
    ensure_persisted_provider_id(&input.provider_id)?;
    let provider = query_provider(&db, &input.provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can copy official account tokens".to_string());
    }

    let auth = if input.account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        let local_auth = read_auth_json_from_disk(Some(&db)).await?;
        if !auth_has_official_runtime(&local_auth) {
            return Err(
                "Current local auth.json does not contain an official Codex login".to_string(),
            );
        }
        local_auth
    } else {
        let account = load_official_account(&db, &input.account_id).await?;
        if account.provider_id != input.provider_id {
            return Err("Official account does not belong to the selected provider".to_string());
        }
        let snapshot = account
            .auth_snapshot
            .as_deref()
            .ok_or_else(|| "Official account snapshot is missing".to_string())?;
        auth_json_from_snapshot(snapshot)?
    };

    let token = extract_token_from_auth_snapshot(&auth, input.token_kind.trim())?;
    copy_text_to_clipboard(&token)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_id_token(account_id: &str) -> String {
        let payload = serde_json::json!({
            "email": "user@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id
            }
        });
        format!(
            "e30.{}.sig",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&payload).expect("serialize JWT payload"))
        )
    }

    #[test]
    fn auth_has_official_runtime_requires_auth_mode_and_both_tokens() {
        assert!(auth_has_official_runtime(&serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "access-token",
                "refresh_token": "refresh-token"
            }
        })));

        assert!(!auth_has_official_runtime(&serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "access-token"
            }
        })));
        assert!(!auth_has_official_runtime(&serde_json::json!({
            "OPENAI_API_KEY": "sk-test",
            "auth_mode": "apikey"
        })));
        assert!(!auth_has_official_runtime(&serde_json::json!({})));
    }

    #[test]
    fn refreshed_auth_preserves_rotated_fields_omitted_by_server() {
        let existing_id_token = test_id_token("account-1");
        let existing = serde_json::json!({
            "auth_mode": "chatgpt",
            "runtime_owned": true,
            "tokens": {
                "id_token": existing_id_token,
                "access_token": "old-access",
                "refresh_token": "old-refresh",
                "account_id": "account-1"
            }
        });
        let response = OAuthTokenResponse {
            access_token: "new-access".to_string(),
            refresh_token: None,
            id_token: None,
        };

        let merged = build_auth_snapshot(&response, Some(&existing)).expect("build auth snapshot");

        assert_eq!(
            merged
                .pointer("/tokens/access_token")
                .and_then(Value::as_str),
            Some("new-access")
        );
        assert_eq!(
            merged
                .pointer("/tokens/refresh_token")
                .and_then(Value::as_str),
            Some("old-refresh")
        );
        assert_eq!(
            merged.pointer("/tokens/id_token").and_then(Value::as_str),
            Some(existing_id_token.as_str())
        );
        assert_eq!(
            merged.get("runtime_owned").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn browser_oauth_authorize_url_uses_actual_callback_port_and_official_scopes() {
        let redirect_uri = build_oauth_redirect_uri(24680);
        let authorize_url = build_codex_authorize_url(&redirect_uri, "state-value", "challenge");

        assert_eq!(redirect_uri, "http://localhost:24680/auth/callback");
        assert!(
            authorize_url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A24680%2Fauth%2Fcallback")
        );
        assert!(authorize_url.contains(
            "scope=openid%20profile%20email%20offline_access%20api.connectors.read%20api.connectors.invoke"
        ));
        assert!(authorize_url.contains("code_challenge_method=S256"));
        assert!(authorize_url.contains("codex_cli_simplified_flow=true"));
        assert!(authorize_url.contains("originator=codex_cli_rs"));
    }

    #[test]
    fn browser_oauth_falls_back_when_default_port_is_occupied() {
        let occupied_listener = TcpListener::bind(("127.0.0.1", CODEX_OAUTH_DEFAULT_PORT)).ok();
        let (_callback_listener, callback_port) = bind_oauth_listener().expect("bind callback");

        if occupied_listener.is_some() {
            assert_ne!(callback_port, CODEX_OAUTH_DEFAULT_PORT);
        }
        assert_ne!(callback_port, 0);
    }

    #[test]
    fn browser_oauth_callback_rejects_invalid_state() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind callback listener");
        let address = listener.local_addr().expect("callback address");
        let callback_thread =
            std::thread::spawn(move || wait_for_oauth_callback(listener, "expected-state"));

        let mut stream = std::net::TcpStream::connect(address).expect("connect callback");
        stream
            .write_all(
                b"GET /auth/callback?code=auth-code&state=wrong-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .expect("write callback request");
        let error = callback_thread
            .join()
            .expect("callback thread")
            .expect_err("invalid state must fail");
        assert!(error.contains("state mismatch"));
    }

    #[test]
    fn browser_oauth_callback_times_out_without_connection() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind callback listener");
        let error = wait_for_oauth_callback_with_timeout(
            listener,
            "expected-state",
            Duration::from_millis(10),
        )
        .expect_err("callback must time out");
        assert!(error.contains("Timed out"));
    }

    #[test]
    fn device_auth_cancel_signals_and_removes_session() {
        let session_id = format!("test-device-session-{}", uuid::Uuid::new_v4());
        let (cancel_sender, cancel_receiver) = watch::channel(false);
        DEVICE_AUTH_SESSIONS
            .lock()
            .expect("device session lock")
            .insert(session_id.clone(), cancel_sender);

        cancel_codex_official_account_device_auth(session_id.clone()).expect("cancel device auth");

        assert!(*cancel_receiver.borrow());
        assert!(!DEVICE_AUTH_SESSIONS
            .lock()
            .expect("device session lock")
            .contains_key(&session_id));
    }

    #[test]
    fn official_runtime_merge_preserves_unknown_runtime_fields() {
        let existing = serde_json::json!({
            "OPENAI_API_KEY": "remove-me",
            "auth_mode": "chatgpt",
            "last_refresh": "runtime-refresh",
            "runtime_root": { "keep": true },
            "tokens": {
                "access_token": "old-access",
                "refresh_token": "old-refresh",
                "runtime_token_metadata": { "keep": true }
            }
        });
        let next = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "new-access",
                "refresh_token": "new-refresh",
                "id_token": "new-id"
            }
        });

        let merged = merge_official_runtime_auth(&existing, &next);
        assert!(merged.get("OPENAI_API_KEY").is_none());
        assert_eq!(merged["runtime_root"]["keep"], true);
        assert_eq!(merged["last_refresh"], "runtime-refresh");
        assert_eq!(merged["tokens"]["access_token"], "new-access");
        assert_eq!(merged["tokens"]["runtime_token_metadata"]["keep"], true);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_auth_writer_replaces_content_and_sets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let auth_path = temp_dir.path().join("auth.json");
        fs::write(&auth_path, "{\"old\":true}\n").expect("write old auth");

        write_auth_json_atomic(
            &auth_path,
            &serde_json::json!({
                "auth_mode": "chatgpt",
                "tokens": { "access_token": "new-access" },
                "runtime_owned": true
            }),
        )
        .expect("write auth atomically");

        let written: Value =
            serde_json::from_str(&fs::read_to_string(&auth_path).expect("read written auth"))
                .expect("parse written auth");
        assert_eq!(written["runtime_owned"], true);
        assert!(written.get("old").is_none());
        assert_eq!(
            fs::metadata(&auth_path)
                .expect("auth metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn parse_usage_snapshot_free_treats_primary_week_window_as_weekly_limit() {
        let body = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 25.0,
                    "limit_window_seconds": WEEK_WINDOW_SECONDS,
                    "reset_at": 12345
                }
            }
        });

        let snapshot = parse_usage_snapshot(&body, Some("free"));

        assert_eq!(snapshot.limit_short_label, None);
        assert_eq!(snapshot.limit_5h_text, None);
        assert_eq!(snapshot.limit_5h_reset_at, None);
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("75%"));
        assert_eq!(snapshot.limit_weekly_reset_at, Some(12345));
        assert_eq!(snapshot.limit_monthly_text, None);
        assert_eq!(snapshot.limit_monthly_reset_at, None);
    }

    #[test]
    fn parse_usage_snapshot_free_falls_back_to_primary_window_as_weekly_limit() {
        let body = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "remaining_count": 3.0,
                    "total_count": 4.0,
                    "reset_at": 67890
                }
            }
        });

        let snapshot = parse_usage_snapshot(&body, Some("free"));

        assert_eq!(snapshot.limit_short_label, None);
        assert_eq!(snapshot.limit_5h_text, None);
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("75%"));
        assert_eq!(snapshot.limit_weekly_reset_at, Some(67890));
        assert_eq!(snapshot.limit_monthly_text, None);
        assert_eq!(snapshot.limit_monthly_reset_at, None);
    }

    #[test]
    fn parse_usage_snapshot_non_free_uses_window_duration_before_order() {
        let body = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 10.0,
                    "limit_window_seconds": WEEK_WINDOW_SECONDS,
                    "reset_at": 111
                },
                "secondary_window": {
                    "used_percent": 40.0,
                    "limit_window_seconds": FIVE_HOUR_WINDOW_SECONDS,
                    "reset_at": 222
                }
            }
        });

        let snapshot = parse_usage_snapshot(&body, Some("plus"));

        assert_eq!(snapshot.limit_short_label.as_deref(), Some("5h"));
        assert_eq!(snapshot.limit_5h_text.as_deref(), Some("60%"));
        assert_eq!(snapshot.limit_5h_reset_at, Some(222));
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("90%"));
        assert_eq!(snapshot.limit_weekly_reset_at, Some(111));
        assert_eq!(snapshot.limit_monthly_text, None);
        assert_eq!(snapshot.limit_monthly_reset_at, None);
    }

    #[test]
    fn parse_usage_snapshot_detects_monthly_window() {
        let body = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 10.0,
                    "limit_window_seconds": WEEK_WINDOW_SECONDS,
                    "reset_at": 111
                },
                "secondary_window": {
                    "used_percent": 40.0,
                    "limit_window_seconds": FIVE_HOUR_WINDOW_SECONDS,
                    "reset_at": 222
                },
                "monthly_window": {
                    "used_percent": 70.0,
                    "limit_window_seconds": 30 * 24 * 60 * 60,
                    "reset_at": 333
                }
            }
        });

        let snapshot = parse_usage_snapshot(&body, Some("plus"));

        assert_eq!(snapshot.limit_short_label.as_deref(), Some("5h"));
        assert_eq!(snapshot.limit_5h_text.as_deref(), Some("60%"));
        assert_eq!(snapshot.limit_5h_reset_at, Some(222));
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("90%"));
        assert_eq!(snapshot.limit_weekly_reset_at, Some(111));
        assert_eq!(snapshot.limit_monthly_text.as_deref(), Some("30%"));
        assert_eq!(snapshot.limit_monthly_reset_at, Some(333));
    }

    #[test]
    fn parse_usage_snapshot_detects_monthly_window_from_additional_rate_limits() {
        let body = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 10.0,
                    "limit_window_seconds": FIVE_HOUR_WINDOW_SECONDS,
                    "reset_at": 111
                },
                "secondary_window": {
                    "used_percent": 40.0,
                    "limit_window_seconds": WEEK_WINDOW_SECONDS,
                    "reset_at": 222
                }
            },
            "additional_rate_limits": [
                {
                    "limit_name": "monthly",
                    "rate_limit": {
                        "primary_window": {
                            "used_percent": 70.0,
                            "limit_window_seconds": 30 * 24 * 60 * 60,
                            "reset_at": 333
                        }
                    }
                }
            ]
        });

        let snapshot = parse_usage_snapshot(&body, Some("plus"));

        assert_eq!(snapshot.limit_5h_text.as_deref(), Some("90%"));
        assert_eq!(snapshot.limit_5h_reset_at, Some(111));
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("60%"));
        assert_eq!(snapshot.limit_weekly_reset_at, Some(222));
        assert_eq!(snapshot.limit_monthly_text.as_deref(), Some("30%"));
        assert_eq!(snapshot.limit_monthly_reset_at, Some(333));
    }

    #[test]
    fn extract_reset_timestamp_uses_relative_seconds_when_absolute_timestamp_is_missing() {
        let window = serde_json::json!({
            "reset_after_seconds": 90
        });

        assert_eq!(extract_reset_timestamp_at(&window, 1_000), Some(1_090));
    }

    #[test]
    fn parse_usage_snapshot_uses_limit_reached_hint_when_percent_is_missing() {
        let body = serde_json::json!({
            "rate_limit": {
                "limitReached": "true",
                "primary_window": {
                    "limit_window_seconds": FIVE_HOUR_WINDOW_SECONDS,
                    "reset_at": 111
                },
                "secondary_window": {
                    "used_percent": 40.0,
                    "limit_window_seconds": WEEK_WINDOW_SECONDS,
                    "reset_at": 222
                }
            }
        });

        let snapshot = parse_usage_snapshot(&body, Some("plus"));

        assert_eq!(snapshot.limit_5h_text.as_deref(), Some("0%"));
        assert_eq!(snapshot.limit_5h_reset_at, Some(111));
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("60%"));
        assert_eq!(snapshot.limit_weekly_reset_at, Some(222));
    }

    #[test]
    fn should_hide_virtual_local_account_when_refresh_token_matches_saved_account() {
        let local_auth = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "local-access",
                "refresh_token": "same-refresh",
                "account_id": "acc-1",
                "id_token": "header.payload.sig"
            }
        });
        let persisted_account = CodexOfficialAccount {
            id: "saved-1".to_string(),
            provider_id: "provider-1".to_string(),
            name: "saved".to_string(),
            kind: "oauth".to_string(),
            email: Some("saved@example.com".to_string()),
            auth_snapshot: Some(
                serde_json::json!({
                    "auth_mode": "chatgpt",
                    "tokens": {
                        "access_token": "saved-access",
                        "refresh_token": "same-refresh",
                        "account_id": "acc-1",
                    }
                })
                .to_string(),
            ),
            auth_mode: Some("chatgpt".to_string()),
            account_id: Some("acc-1".to_string()),
            plan_type: None,
            last_refresh: None,
            token_expires_at: None,
            access_token_preview: None,
            refresh_token_preview: None,
            limit_short_label: None,
            limit_5h_text: None,
            limit_weekly_text: None,
            limit_monthly_text: None,
            limit_5h_reset_at: None,
            limit_weekly_reset_at: None,
            limit_monthly_reset_at: None,
            last_limits_fetched_at: None,
            last_error: None,
            sort_index: None,
            is_applied: false,
            is_virtual: false,
            created_at: String::new(),
            updated_at: String::new(),
        };

        assert!(!should_show_virtual_local_account(
            &[persisted_account],
            &local_auth
        ));
    }

    #[test]
    fn should_show_virtual_local_account_when_local_auth_is_not_saved() {
        let local_auth = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "local-access",
                "refresh_token": "local-refresh",
                "account_id": "acc-local"
            }
        });
        let persisted_account = CodexOfficialAccount {
            id: "saved-1".to_string(),
            provider_id: "provider-1".to_string(),
            name: "saved".to_string(),
            kind: "oauth".to_string(),
            email: Some("saved@example.com".to_string()),
            auth_snapshot: Some(
                serde_json::json!({
                    "auth_mode": "chatgpt",
                    "tokens": {
                        "access_token": "saved-access",
                        "refresh_token": "saved-refresh",
                        "account_id": "acc-saved"
                    }
                })
                .to_string(),
            ),
            auth_mode: Some("chatgpt".to_string()),
            account_id: Some("acc-saved".to_string()),
            plan_type: None,
            last_refresh: None,
            token_expires_at: None,
            access_token_preview: None,
            refresh_token_preview: None,
            limit_short_label: None,
            limit_5h_text: None,
            limit_weekly_text: None,
            limit_monthly_text: None,
            limit_5h_reset_at: None,
            limit_weekly_reset_at: None,
            limit_monthly_reset_at: None,
            last_limits_fetched_at: None,
            last_error: None,
            sort_index: None,
            is_applied: false,
            is_virtual: false,
            created_at: String::new(),
            updated_at: String::new(),
        };

        assert!(should_show_virtual_local_account(
            &[persisted_account],
            &local_auth
        ));
    }
}

pub async fn ensure_codex_provider_has_no_official_accounts(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    if codex_provider_has_official_accounts(db, provider_id).await? {
        return Err(
            "Please delete all official accounts under this provider before deleting the provider"
                .to_string(),
        );
    }
    Ok(())
}

pub async fn codex_provider_has_official_accounts(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<bool, String> {
    Ok(!list_persisted_official_accounts(db, provider_id)
        .await?
        .is_empty())
}

pub async fn clear_all_codex_official_account_apply_status(
    db: &crate::db::SqliteDbState,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CodexOfficialAccount, None, &now)
    })?;
    Ok(())
}

pub async fn sync_codex_official_account_apply_status(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    let local_auth = read_auth_json_from_disk(Some(db)).await?;
    if !auth_has_official_runtime(&local_auth) {
        return update_official_account_apply_status(db, None).await;
    }

    let accounts = list_persisted_official_accounts(db, provider_id).await?;
    let matched_account_id = accounts
        .iter()
        .find(|account| official_account_identity_matches_auth(account, &local_auth))
        .map(|account| account.id.clone());

    update_official_account_apply_status(db, matched_account_id.as_deref()).await
}
