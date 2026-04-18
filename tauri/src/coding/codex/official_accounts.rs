use base64::Engine;
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use super::adapter;
use super::commands::{
    apply_config_internal, get_codex_root_dir_from_db_async, get_codex_root_dir_without_db,
};
use super::types::{CodexOfficialAccount, CodexOfficialAccountContent, CodexProvider};
use crate::coding::db_id::{db_new_id, db_record_id};
use crate::db::DbState;
use tauri::Emitter;

const CODEX_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_OAUTH_AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CODEX_OAUTH_DEFAULT_PORT: u16 = 1455;
const CODEX_OAUTH_CALLBACK_PATH: &str = "/auth/callback";
const LOCAL_OFFICIAL_ACCOUNT_ID: &str = "__local__";
const FIVE_HOUR_WINDOW_SECONDS: i64 = 18_000;
const WEEK_WINDOW_SECONDS: i64 = 604_800;
const AUTH_REFRESH_LEAD_SECONDS: i64 = 3 * 24 * 60 * 60;

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
    limit_5h_reset_at: Option<i64>,
    limit_weekly_reset_at: Option<i64>,
}

fn oauth_scopes() -> &'static str {
    "openid profile email offline_access api.connectors.read api.connectors.invoke"
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

fn wait_for_oauth_callback(state: &str) -> Result<String, String> {
    let listener = TcpListener::bind(("127.0.0.1", CODEX_OAUTH_DEFAULT_PORT)).map_err(|error| {
        format!(
            "Failed to listen on localhost:{}: {error}",
            CODEX_OAUTH_DEFAULT_PORT
        )
    })?;
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
                if start.elapsed() >= Duration::from_secs(300) {
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
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<OAuthTokenResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("Failed to build OAuth HTTP client: {error}"))?;

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

async fn refresh_oauth_token(refresh_token: &str) -> Result<OAuthTokenResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("Failed to build OAuth refresh client: {error}"))?;

    client
        .post(CODEX_OAUTH_TOKEN_URL)
        .form(&OAuthRefreshRequest {
            grant_type: "refresh_token",
            client_id: CODEX_OAUTH_CLIENT_ID,
            refresh_token,
        })
        .send()
        .await
        .map_err(|error| format!("Failed to refresh OAuth token: {error}"))?
        .error_for_status()
        .map_err(|error| format!("OAuth token refresh failed: {error}"))?
        .json::<OAuthTokenResponse>()
        .await
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
    let id_token = token_response
        .id_token
        .as_deref()
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
            "refresh_token": token_response.refresh_token.clone().unwrap_or_default(),
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

fn auth_has_official_runtime(auth: &Value) -> bool {
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

async fn ensure_fresh_official_runtime_auth(auth: &Value) -> Result<Value, String> {
    if !official_auth_needs_refresh(auth) {
        return Ok(auth.clone());
    }

    let refresh_token = auth
        .pointer("/tokens/refresh_token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Official account is missing refresh token".to_string())?;
    let refreshed_token_response = refresh_oauth_token(refresh_token).await?;
    build_auth_snapshot(&refreshed_token_response, Some(auth))
}

fn auth_json_from_snapshot(snapshot: &str) -> Result<Value, String> {
    serde_json::from_str(snapshot)
        .map_err(|error| format!("Failed to parse account snapshot: {error}"))
}

async fn read_auth_json_from_disk(
    db: Option<&surrealdb::Surreal<surrealdb::engine::local::Db>>,
) -> Result<Value, String> {
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    auth: &Value,
) -> Result<(), String> {
    let root_dir = get_codex_root_dir_from_db_async(db).await?;
    if !root_dir.exists() {
        fs::create_dir_all(&root_dir)
            .map_err(|error| format!("Failed to create Codex root directory: {error}"))?;
    }
    let auth_path = root_dir.join("auth.json");
    let content = serde_json::to_string_pretty(auth)
        .map_err(|error| format!("Failed to serialize auth.json: {error}"))?;
    fs::write(&auth_path, content).map_err(|error| format!("Failed to write auth.json: {error}"))
}

async fn query_provider(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
) -> Result<CodexProvider, String> {
    let provider_record_id = db_record_id("codex_provider", provider_id);
    let result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            provider_record_id
        ))
        .await
        .map_err(|error| format!("Failed to query provider: {error}"))?
        .take(0);

    match result {
        Ok(records) => records
            .first()
            .cloned()
            .map(adapter::from_db_value_provider)
            .ok_or_else(|| format!("Codex provider '{}' not found", provider_id)),
        Err(error) => Err(format!("Failed to deserialize provider: {error}")),
    }
}

async fn list_persisted_official_accounts(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
) -> Result<Vec<CodexOfficialAccount>, String> {
    let result: Result<Vec<Value>, _> = db
        .query(
            "SELECT *, type::string(id) as id FROM codex_official_account WHERE provider_id = $provider_id ORDER BY sort_index ASC, created_at ASC",
        )
        .bind(("provider_id", provider_id.to_string()))
        .await
        .map_err(|error| format!("Failed to query official accounts: {error}"))?
        .take(0);

    match result {
        Ok(records) => Ok(records
            .into_iter()
            .map(adapter::from_db_value_official_account)
            .collect()),
        Err(error) => Err(format!("Failed to deserialize official accounts: {error}")),
    }
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
        limit_5h_reset_at: None,
        limit_weekly_reset_at: None,
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

fn format_percent_label(value: f64) -> String {
    format!("{:.0}%", value.clamp(0.0, 100.0))
}

fn resolve_rate_windows(body: &Value) -> (Option<&Value>, Option<&Value>) {
    let rate_limit = body.get("rate_limit").unwrap_or(body);
    let primary = rate_limit
        .get("primary_window")
        .or_else(|| rate_limit.get("primaryWindow"))
        .or_else(|| body.get("five_hour"))
        .or_else(|| body.get("5_hour_window"))
        .or_else(|| body.get("fiveHourWindow"));
    let secondary = rate_limit
        .get("secondary_window")
        .or_else(|| rate_limit.get("secondaryWindow"))
        .or_else(|| body.get("seven_day"))
        .or_else(|| body.get("weekly_window"))
        .or_else(|| body.get("weeklyWindow"));
    (primary, secondary)
}

fn extract_limit_window_seconds(window: &Value) -> Option<i64> {
    window
        .get("limit_window_seconds")
        .or_else(|| window.get("limitWindowSeconds"))
        .and_then(Value::as_i64)
}

fn classify_rate_windows(body: &Value) -> (Option<&Value>, Option<&Value>) {
    let (primary, secondary) = resolve_rate_windows(body);
    let raw_windows = [primary, secondary];
    let mut short_window = None;
    let mut weekly_window = None;

    for window in raw_windows.into_iter().flatten() {
        match extract_limit_window_seconds(window) {
            Some(FIVE_HOUR_WINDOW_SECONDS) if short_window.is_none() => {
                short_window = Some(window);
            }
            Some(WEEK_WINDOW_SECONDS) if weekly_window.is_none() => {
                weekly_window = Some(window);
            }
            _ => {}
        }
    }

    if short_window.is_none() {
        short_window = primary.filter(|window| Some(*window) != weekly_window);
    }
    if weekly_window.is_none() {
        weekly_window = secondary.filter(|window| Some(*window) != short_window);
    }

    (short_window, weekly_window)
}

fn extract_reset_timestamp(window: &Value) -> Option<i64> {
    window
        .get("reset_at")
        .or_else(|| window.get("resetAt"))
        .or_else(|| window.get("resets_at"))
        .or_else(|| window.get("resetsAt"))
        .and_then(Value::as_i64)
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

fn parse_usage_snapshot(body: &Value, plan_type: Option<&str>) -> UsageSnapshot {
    let (short_window, weekly_window) = classify_rate_windows(body);
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
                .and_then(parse_remaining_percent_from_window)
                .map(format_percent_label)
        } else {
            None
        },
        limit_weekly_text: effective_weekly_window
            .and_then(parse_remaining_percent_from_window)
            .map(format_percent_label),
        limit_5h_reset_at: if has_short_window {
            short_window.and_then(extract_reset_timestamp)
        } else {
            None
        },
        limit_weekly_reset_at: effective_weekly_window.and_then(extract_reset_timestamp),
    }
}

async fn fetch_usage_snapshot(
    access_token: &str,
    plan_type: Option<&str>,
) -> Result<UsageSnapshot, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("Failed to build usage HTTP client: {error}"))?;

    let response = client
        .get(CODEX_USAGE_URL)
        .header("Authorization", format!("Bearer {access_token}"))
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AI-Toolbox Codex OAuth",
        )
        .header("Content-Type", "application/json")
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
        limit_5h_reset_at: usage_snapshot.and_then(|snapshot| snapshot.limit_5h_reset_at),
        limit_weekly_reset_at: usage_snapshot.and_then(|snapshot| snapshot.limit_weekly_reset_at),
        last_limits_fetched_at: usage_snapshot.map(|_| now.clone()),
        last_error: None,
        sort_index: None,
        is_applied: false,
        created_at: now.clone(),
        updated_at: now,
    })
}

async fn save_official_account(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    content: &CodexOfficialAccountContent,
) -> Result<CodexOfficialAccount, String> {
    let record_id = db_record_id("codex_official_account", &db_new_id());
    db.query(&format!("CREATE {} CONTENT $data", record_id))
        .bind(("data", adapter::to_db_value_official_account(content)))
        .await
        .map_err(|error| format!("Failed to create official account: {error}"))?;

    let result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|error| format!("Failed to query created official account: {error}"))?
        .take(0);

    match result {
        Ok(records) => records
            .first()
            .cloned()
            .map(adapter::from_db_value_official_account)
            .ok_or_else(|| "Failed to load created official account".to_string()),
        Err(error) => Err(format!(
            "Failed to deserialize created official account: {error}"
        )),
    }
}

async fn find_matching_official_account(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
    auth: &Value,
) -> Result<Option<CodexOfficialAccount>, String> {
    let accounts = list_persisted_official_accounts(db, provider_id).await?;
    Ok(accounts
        .into_iter()
        .find(|account| official_account_identity_matches_auth(account, auth)))
}

async fn update_official_account_apply_status(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    account_id: Option<&str>,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    db.query(
        "UPDATE codex_official_account SET is_applied = false, updated_at = $now WHERE is_applied = true",
    )
    .bind(("now", now.clone()))
    .await
    .map_err(|error| format!("Failed to clear official account apply state: {error}"))?;

    if let Some(account_id) = account_id {
        let record_id = db_record_id("codex_official_account", account_id);
        db.query(&format!(
            "UPDATE {} SET is_applied = true, updated_at = $now",
            record_id
        ))
        .bind(("now", now))
        .await
        .map_err(|error| format!("Failed to mark official account as applied: {error}"))?;
    }

    Ok(())
}

async fn load_official_account(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    account_id: &str,
) -> Result<CodexOfficialAccount, String> {
    let record_id = db_record_id("codex_official_account", account_id);
    let result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|error| format!("Failed to query official account: {error}"))?
        .take(0);

    match result {
        Ok(records) => records
            .first()
            .cloned()
            .map(adapter::from_db_value_official_account)
            .ok_or_else(|| format!("Official account '{}' not found", account_id)),
        Err(error) => Err(format!("Failed to deserialize official account: {error}")),
    }
}

async fn persist_usage_snapshot(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    account_id: &str,
    usage_snapshot: &UsageSnapshot,
    last_error: Option<&str>,
) -> Result<CodexOfficialAccount, String> {
    let now = Local::now().to_rfc3339();
    let record_id = db_record_id("codex_official_account", account_id);
    db.query(&format!(
        "UPDATE {} SET limit_short_label = $limit_short_label, limit_5h_text = $limit_5h_text, limit_weekly_text = $limit_weekly_text, limit_5h_reset_at = $limit_5h_reset_at, limit_weekly_reset_at = $limit_weekly_reset_at, last_limits_fetched_at = $last_limits_fetched_at, last_error = $last_error, updated_at = $updated_at",
        record_id
    ))
    .bind(("limit_short_label", usage_snapshot.limit_short_label.clone()))
    .bind(("limit_5h_text", usage_snapshot.limit_5h_text.clone()))
    .bind(("limit_weekly_text", usage_snapshot.limit_weekly_text.clone()))
    .bind(("limit_5h_reset_at", usage_snapshot.limit_5h_reset_at))
    .bind(("limit_weekly_reset_at", usage_snapshot.limit_weekly_reset_at))
    .bind(("last_limits_fetched_at", Some(now.clone())))
    .bind(("last_error", last_error.map(|value| value.to_string())))
    .bind(("updated_at", now))
    .await
    .map_err(|error| format!("Failed to update official account usage: {error}"))?;

    load_official_account(db, account_id).await
}

async fn persist_refreshed_account_snapshot(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    account: &CodexOfficialAccount,
    refreshed_snapshot: &Value,
) -> Result<CodexOfficialAccount, String> {
    let parsed_content = build_account_content_from_auth_snapshot(
        &account.provider_id,
        refreshed_snapshot,
        None,
        Some(&account.name),
    )?;
    let record_id = db_record_id("codex_official_account", &account.id);
    db.query(&format!("UPDATE {} CONTENT $data", record_id))
        .bind((
            "data",
            adapter::to_db_value_official_account(&CodexOfficialAccountContent {
                is_applied: account.is_applied,
                created_at: account.created_at.clone(),
                updated_at: Local::now().to_rfc3339(),
                limit_short_label: account.limit_short_label.clone(),
                limit_5h_text: account.limit_5h_text.clone(),
                limit_weekly_text: account.limit_weekly_text.clone(),
                limit_5h_reset_at: account.limit_5h_reset_at,
                limit_weekly_reset_at: account.limit_weekly_reset_at,
                last_limits_fetched_at: account.last_limits_fetched_at.clone(),
                last_error: account.last_error.clone(),
                sort_index: account.sort_index,
                ..parsed_content
            }),
        ))
        .await
        .map_err(|error| {
            format!("Failed to persist refreshed official account snapshot: {error}")
        })?;

    load_official_account(db, &account.id).await
}

fn merge_official_runtime_auth(existing_auth: &Value, next_auth: &Value) -> Value {
    let mut merged = existing_auth.as_object().cloned().unwrap_or_default();
    merged.remove("OPENAI_API_KEY");

    let runtime_keys = ["auth_mode", "tokens", "last_refresh", "agent_identity"];
    for key in runtime_keys {
        if let Some(value) = next_auth.get(key) {
            merged.insert(key.to_string(), value.clone());
        } else {
            merged.remove(key);
        }
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

#[tauri::command]
pub async fn list_codex_official_accounts(
    state: tauri::State<'_, DbState>,
    provider_id: String,
) -> Result<Vec<CodexOfficialAccount>, String> {
    let db = state.db();
    let provider = query_provider(&db, &provider_id).await?;
    let mut accounts = list_persisted_official_accounts(&db, &provider_id).await?;
    let local_auth = read_auth_json_from_disk(Some(&db)).await?;
    if provider.category == "official" && should_show_virtual_local_account(&accounts, &local_auth)
    {
        accounts.push(assign_provider_id(
            build_virtual_local_account(&local_auth),
            &provider_id,
        ));
    }
    Ok(accounts)
}

#[tauri::command]
pub async fn start_codex_official_account_oauth(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<CodexOfficialAccount, String> {
    let db = state.db();
    let provider = query_provider(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can add official accounts".to_string());
    }

    let existing_auth = read_auth_json_from_disk(Some(&db)).await?;
    let oauth_state = generate_random_urlsafe(32);
    let (code_verifier, code_challenge) = generate_pkce_pair();
    let redirect_uri = build_oauth_redirect_uri(CODEX_OAUTH_DEFAULT_PORT);
    let authorize_url = build_codex_authorize_url(&redirect_uri, &oauth_state, &code_challenge);

    open_browser(&authorize_url)?;
    let authorization_code =
        tokio::task::spawn_blocking(move || wait_for_oauth_callback(&oauth_state))
            .await
            .map_err(|error| format!("OAuth callback task failed: {error}"))??;
    let token_response =
        exchange_authorization_code(&authorization_code, &redirect_uri, &code_verifier).await?;
    let auth_snapshot = build_auth_snapshot(&token_response, Some(&existing_auth))?;
    let plan_type = usage_plan_type_from_auth(&auth_snapshot);
    let usage_snapshot = fetch_usage_snapshot(&token_response.access_token, plan_type.as_deref())
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
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<CodexOfficialAccount, String> {
    let db = state.db();
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

    let usage_snapshot = local_auth
        .pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|access_token| async {
            fetch_usage_snapshot(
                access_token,
                usage_plan_type_from_auth(&local_auth).as_deref(),
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
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
    account_id: String,
) -> Result<(), String> {
    let db = state.db();
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
        let refreshed_auth = ensure_fresh_official_runtime_auth(&local_auth).await?;
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
        let refreshed_snapshot = ensure_fresh_official_runtime_auth(&snapshot_auth).await?;
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
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
    account_id: String,
) -> Result<(), String> {
    let db = state.db();
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

    let record_id = db_record_id("codex_official_account", &account_id);
    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|error| format!("Failed to delete official account: {error}"))?;

    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn refresh_codex_official_account_limits(
    state: tauri::State<'_, DbState>,
    provider_id: String,
    account_id: String,
) -> Result<CodexOfficialAccount, String> {
    let db = state.db();
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
        let usage_snapshot = fetch_usage_snapshot(access_token, plan_type.as_deref()).await?;
        let mut account = assign_provider_id(build_virtual_local_account(&auth), &provider_id);
        account.limit_short_label = usage_snapshot.limit_short_label;
        account.limit_5h_text = usage_snapshot.limit_5h_text;
        account.limit_weekly_text = usage_snapshot.limit_weekly_text;
        account.limit_5h_reset_at = usage_snapshot.limit_5h_reset_at;
        account.limit_weekly_reset_at = usage_snapshot.limit_weekly_reset_at;
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
    let usage_snapshot = fetch_usage_snapshot(access_token, account.plan_type.as_deref()).await?;

    persist_usage_snapshot(&db, &account_id, &usage_snapshot, None).await
}

#[tauri::command]
pub async fn copy_codex_official_account_token(
    state: tauri::State<'_, DbState>,
    input: CodexOfficialAccountTokenCopyInput,
) -> Result<(), String> {
    let db = state.db();
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
            limit_5h_reset_at: None,
            limit_weekly_reset_at: None,
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
            limit_5h_reset_at: None,
            limit_weekly_reset_at: None,
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
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
) -> Result<(), String> {
    let result: Result<Vec<Value>, _> = db
        .query("SELECT count() AS count FROM codex_official_account WHERE provider_id = $provider_id GROUP ALL")
        .bind(("provider_id", provider_id.to_string()))
        .await
        .map_err(|error| format!("Failed to query official account count: {error}"))?
        .take(0);

    let count = result
        .ok()
        .and_then(|records| records.first().cloned())
        .and_then(|record| record.get("count").and_then(Value::as_i64))
        .unwrap_or(0);
    if count > 0 {
        return Err(
            "Please delete all official accounts under this provider before deleting the provider"
                .to_string(),
        );
    }
    Ok(())
}

pub async fn codex_provider_has_official_accounts(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
) -> Result<bool, String> {
    let result: Result<Vec<Value>, _> = db
        .query("SELECT count() AS count FROM codex_official_account WHERE provider_id = $provider_id GROUP ALL")
        .bind(("provider_id", provider_id.to_string()))
        .await
        .map_err(|error| format!("Failed to query official account count: {error}"))?
        .take(0);

    let count = result
        .ok()
        .and_then(|records| records.first().cloned())
        .and_then(|record| record.get("count").and_then(Value::as_i64))
        .unwrap_or(0);
    Ok(count > 0)
}

pub async fn clear_all_codex_official_account_apply_status(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    db.query(
        "UPDATE codex_official_account SET is_applied = false, updated_at = $now WHERE is_applied = true",
    )
    .bind(("now", now))
    .await
    .map_err(|error| format!("Failed to clear official account apply state: {error}"))?;
    Ok(())
}

pub async fn sync_codex_official_account_apply_status(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
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
