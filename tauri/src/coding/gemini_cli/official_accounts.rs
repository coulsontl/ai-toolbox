use base64::Engine;
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use super::adapter;
use super::commands::{
    apply_config_internal, get_gemini_cli_oauth_creds_path_from_root, query_provider_by_id,
};
use super::types::{
    GeminiCliOfficialAccount, GeminiCliOfficialAccountContent,
    GeminiCliOfficialAccountTokenCopyInput,
};
use crate::coding::db_id::{db_new_id, db_record_id};
use crate::coding::runtime_location;
use crate::db::DbState;
use crate::http_client;
use tauri::Emitter;

const GEMINI_OAUTH_CLIENT_ID_ENV: &str = "GEMINI_CLI_OAUTH_CLIENT_ID";
const GEMINI_OAUTH_CLIENT_SECRET_ENV: &str = "GEMINI_CLI_OAUTH_CLIENT_SECRET";
const DEFAULT_GEMINI_OAUTH_CLIENT_ID_PARTS: &[&str] = &[
    "681",
    "255",
    "809",
    "395-",
    "oo8ft2oprdrnp9e3aqf6av3hmdib135j",
    ".apps.",
    "google",
    "usercontent.com",
];
const DEFAULT_GEMINI_OAUTH_CLIENT_SECRET_PARTS: &[&str] =
    &["GO", "CSPX-", "4uHgMPm-1o7Sk-geV6Cu5clXFsxl"];
const GEMINI_OAUTH_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GEMINI_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GEMINI_USER_INFO_URL: &str = "https://www.googleapis.com/oauth2/v1/userinfo?alt=json";
const GEMINI_CODE_ASSIST_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal";
const GEMINI_OAUTH_DEFAULT_PORT: u16 = 8085;
const GEMINI_OAUTH_CALLBACK_PATH: &str = "/oauth2callback";
const LOCAL_OFFICIAL_ACCOUNT_ID: &str = "__local__";
const AUTH_REFRESH_LEAD_SECONDS: i64 = 5 * 60;

#[derive(Debug, Clone, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct OAuthCodeRequest<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    client_secret: &'a str,
    code: &'a str,
    redirect_uri: &'a str,
}

#[derive(Debug, Clone, Serialize)]
struct OAuthRefreshRequest<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    client_secret: &'a str,
    refresh_token: &'a str,
}

#[derive(Debug, Clone, Default)]
struct GeminiQuotaSnapshot {
    project_id: Option<String>,
    plan_type: Option<String>,
    limit_weekly_text: Option<String>,
    limit_weekly_reset_at: Option<i64>,
}

#[derive(Debug, Clone)]
struct GeminiOAuthClient {
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

fn gemini_oauth_client() -> GeminiOAuthClient {
    GeminiOAuthClient {
        client_id: read_env_var(GEMINI_OAUTH_CLIENT_ID_ENV)
            .unwrap_or_else(|| join_parts(DEFAULT_GEMINI_OAUTH_CLIENT_ID_PARTS)),
        client_secret: read_env_var(GEMINI_OAUTH_CLIENT_SECRET_ENV)
            .unwrap_or_else(|| join_parts(DEFAULT_GEMINI_OAUTH_CLIENT_SECRET_PARTS)),
    }
}

fn oauth_scopes() -> [&'static str; 3] {
    [
        "https://www.googleapis.com/auth/cloud-platform",
        "https://www.googleapis.com/auth/userinfo.email",
        "https://www.googleapis.com/auth/userinfo.profile",
    ]
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

fn build_oauth_redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}{GEMINI_OAUTH_CALLBACK_PATH}")
}

fn build_gemini_authorize_url(client_id: &str, redirect_uri: &str, state: &str) -> String {
    format!(
        "{GEMINI_OAUTH_AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&access_type=offline&prompt=consent",
        encode_url_component(client_id),
        encode_url_component(redirect_uri),
        encode_url_component(&oauth_scopes().join(" ")),
        encode_url_component(state),
    )
}

fn open_browser(url: &str) -> Result<(), String> {
    tauri_plugin_opener::open_url(url, None::<&str>)
        .map_err(|error| format!("Failed to open Gemini OAuth login page: {error}"))
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

fn wait_for_oauth_callback(state: &str) -> Result<String, String> {
    let listener =
        TcpListener::bind(("127.0.0.1", GEMINI_OAUTH_DEFAULT_PORT)).map_err(|error| {
            format!(
                "Failed to listen on localhost:{}: {error}",
                GEMINI_OAUTH_DEFAULT_PORT
            )
        })?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("Failed to configure OAuth listener: {error}"))?;
    let start = std::time::Instant::now();
    let (mut stream, _) = loop {
        match listener.accept() {
            Ok(connection) => break connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() >= Duration::from_secs(300) {
                    return Err("Timed out waiting for Gemini OAuth callback".to_string());
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
        format!("Gemini OAuth login failed: {error}")
    } else {
        "Gemini OAuth login completed. You can return to AI Toolbox.".to_string()
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();

    if let Some(error) = query_params.get("error") {
        return Err(format!("Gemini OAuth authorize failed: {error}"));
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

async fn exchange_authorization_code(
    db_state: &DbState,
    oauth_client: &GeminiOAuthClient,
    code: &str,
    redirect_uri: &str,
) -> Result<OAuthTokenResponse, String> {
    let client = http_client::client_with_timeout(db_state, 30).await?;
    let response = client
        .post(GEMINI_OAUTH_TOKEN_URL)
        .form(&OAuthCodeRequest {
            grant_type: "authorization_code",
            client_id: oauth_client.client_id.as_str(),
            client_secret: oauth_client.client_secret.as_str(),
            code,
            redirect_uri,
        })
        .send()
        .await
        .map_err(|error| format!("Failed to exchange Gemini OAuth code: {error}"))?;
    parse_token_response(response).await
}

async fn refresh_oauth_token(
    db_state: &DbState,
    refresh_token: &str,
) -> Result<OAuthTokenResponse, String> {
    let oauth_client = gemini_oauth_client();
    let client = http_client::client_with_timeout(db_state, 30).await?;
    let response = client
        .post(GEMINI_OAUTH_TOKEN_URL)
        .form(&OAuthRefreshRequest {
            grant_type: "refresh_token",
            client_id: oauth_client.client_id.as_str(),
            client_secret: oauth_client.client_secret.as_str(),
            refresh_token,
        })
        .send()
        .await
        .map_err(|error| format!("Failed to refresh Gemini OAuth token: {error}"))?;
    parse_token_response(response).await
}

async fn parse_token_response(response: reqwest::Response) -> Result<OAuthTokenResponse, String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed to read Gemini OAuth token response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Gemini OAuth token request failed with HTTP {status}: {body}"
        ));
    }
    serde_json::from_str(&body)
        .map_err(|error| format!("Failed to parse Gemini OAuth token response: {error}"))
}

async fn fetch_user_info(db_state: &DbState, access_token: &str) -> Result<Value, String> {
    let client = http_client::client_with_timeout(db_state, 30).await?;
    let response = client
        .get(GEMINI_USER_INFO_URL)
        .bearer_auth(access_token)
        .header("Content-Type", "application/json")
        .send()
        .await
        .map_err(|error| format!("Failed to fetch Gemini OAuth user info: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed to read Gemini OAuth user info: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Gemini OAuth user info request failed with HTTP {status}: {body}"
        ));
    }
    serde_json::from_str(&body)
        .map_err(|error| format!("Failed to parse Gemini OAuth user info: {error}"))
}

fn extract_auth_string(auth: &Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|pointer| {
        auth.pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn auth_access_token(auth: &Value) -> Option<String> {
    extract_auth_string(auth, &["/access_token", "/token/access_token"])
}

fn auth_refresh_token(auth: &Value) -> Option<String> {
    extract_auth_string(auth, &["/refresh_token", "/token/refresh_token"])
}

fn auth_project_id(auth: &Value) -> Option<String> {
    extract_auth_string(auth, &["/project_id", "/projectId"])
}

fn auth_email(auth: &Value) -> Option<String> {
    extract_auth_string(auth, &["/email"])
}

fn auth_expiry_seconds(auth: &Value) -> Option<i64> {
    auth.pointer("/expiry_date")
        .and_then(Value::as_i64)
        .map(|value| value / 1000)
        .or_else(|| {
            extract_auth_string(auth, &["/expiry", "/token/expiry"])
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
                .map(|value| value.timestamp())
        })
}

fn auth_needs_refresh(auth: &Value) -> bool {
    let now = chrono::Utc::now().timestamp();
    match auth_expiry_seconds(auth) {
        Some(expires_at) => expires_at <= now + AUTH_REFRESH_LEAD_SECONDS,
        None => auth_access_token(auth).is_none(),
    }
}

fn auth_has_official_runtime(auth: &Value) -> bool {
    auth_access_token(auth).is_some() && auth_refresh_token(auth).is_some()
}

fn current_expiry_millis(expires_in: Option<i64>) -> i64 {
    let expires_in = expires_in.unwrap_or(3600).max(1);
    chrono::Utc::now()
        .timestamp_millis()
        .saturating_add(expires_in.saturating_mul(1000))
}

fn build_auth_snapshot(
    token_response: &OAuthTokenResponse,
    existing_auth: Option<&Value>,
    user_info: Option<&Value>,
    project_id: Option<&str>,
    plan_type: Option<&str>,
) -> Value {
    let refresh_token = token_response
        .refresh_token
        .clone()
        .or_else(|| existing_auth.and_then(auth_refresh_token))
        .unwrap_or_default();
    let scope = token_response
        .scope
        .clone()
        .or_else(|| {
            existing_auth.and_then(|auth| extract_auth_string(auth, &["/scope", "/token/scope"]))
        })
        .unwrap_or_else(|| oauth_scopes().join(" "));
    let token_type = token_response
        .token_type
        .clone()
        .or_else(|| {
            existing_auth
                .and_then(|auth| extract_auth_string(auth, &["/token_type", "/token/type"]))
        })
        .unwrap_or_else(|| "Bearer".to_string());
    let expiry_date = current_expiry_millis(token_response.expires_in);
    let email = user_info
        .and_then(|info| info.get("email"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| existing_auth.and_then(auth_email));
    let project_id = project_id
        .map(str::to_string)
        .or_else(|| existing_auth.and_then(auth_project_id));
    let plan_type = plan_type.map(str::to_string).or_else(|| {
        existing_auth.and_then(|auth| extract_auth_string(auth, &["/plan_type", "/planType"]))
    });

    let mut snapshot = json!({
        "access_token": token_response.access_token,
        "refresh_token": refresh_token,
        "token_type": token_type,
        "scope": scope,
        "expiry_date": expiry_date,
        "auth_mode": "oauth-personal",
        "type": "gemini",
        "last_refresh": Local::now().to_rfc3339(),
        "token_uri": GEMINI_OAUTH_TOKEN_URL,
        "scopes": oauth_scopes(),
        "universe_domain": "googleapis.com",
    });
    if let Some(email) = email.filter(|value| !value.trim().is_empty()) {
        snapshot["email"] = Value::String(email);
    }
    if let Some(project_id) = project_id.filter(|value| !value.trim().is_empty()) {
        snapshot["project_id"] = Value::String(project_id.clone());
        snapshot["account_id"] = Value::String(project_id);
    }
    if let Some(plan_type) = plan_type.filter(|value| !value.trim().is_empty()) {
        snapshot["plan_type"] = Value::String(plan_type);
    }
    snapshot
}

fn runtime_oauth_creds_from_auth(auth: &Value) -> Value {
    let mut runtime = json!({
        "access_token": auth_access_token(auth).unwrap_or_default(),
        "refresh_token": auth_refresh_token(auth).unwrap_or_default(),
        "token_type": extract_auth_string(auth, &["/token_type", "/token/type"])
            .unwrap_or_else(|| "Bearer".to_string()),
        "scope": extract_auth_string(auth, &["/scope", "/token/scope"])
            .unwrap_or_else(|| oauth_scopes().join(" ")),
        "expiry_date": auth.pointer("/expiry_date").and_then(Value::as_i64).unwrap_or_else(|| {
            auth_expiry_seconds(auth)
                .map(|value| value.saturating_mul(1000))
                .unwrap_or_else(|| current_expiry_millis(Some(3600)))
        }),
    });
    if let Some(email) = auth_email(auth) {
        runtime["email"] = Value::String(email);
    }
    if let Some(project_id) = auth_project_id(auth) {
        runtime["project_id"] = Value::String(project_id.clone());
        runtime["account_id"] = Value::String(project_id);
    }
    if let Some(plan_type) = extract_auth_string(auth, &["/plan_type", "/planType"]) {
        runtime["plan_type"] = Value::String(plan_type);
    }
    runtime
}

async fn ensure_fresh_auth_snapshot(db_state: &DbState, auth: &Value) -> Result<Value, String> {
    if !auth_needs_refresh(auth) {
        return Ok(auth.clone());
    }
    let refresh_token = auth_refresh_token(auth)
        .ok_or_else(|| "Gemini official account is missing refresh token".to_string())?;
    let refreshed_token = refresh_oauth_token(db_state, &refresh_token).await?;
    Ok(build_auth_snapshot(
        &refreshed_token,
        Some(auth),
        None,
        auth_project_id(auth).as_deref(),
        extract_auth_string(auth, &["/plan_type", "/planType"]).as_deref(),
    ))
}

async fn read_oauth_creds_from_disk(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
) -> Result<Value, String> {
    let root_dir = runtime_location::get_gemini_cli_runtime_location_async(db)
        .await?
        .host_path;
    let auth_path = get_gemini_cli_oauth_creds_path_from_root(&root_dir);
    if !auth_path.exists() {
        return Ok(Value::Object(Default::default()));
    }
    let content = fs::read_to_string(&auth_path)
        .map_err(|error| format!("Failed to read Gemini CLI oauth_creds.json: {error}"))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("Failed to parse Gemini CLI oauth_creds.json: {error}"))
}

async fn write_oauth_creds_to_disk(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    auth: &Value,
) -> Result<(), String> {
    let root_dir = runtime_location::get_gemini_cli_runtime_location_async(db)
        .await?
        .host_path;
    let auth_path = get_gemini_cli_oauth_creds_path_from_root(&root_dir);
    if let Some(parent) = auth_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create Gemini CLI directory: {error}"))?;
    }
    let runtime_creds = runtime_oauth_creds_from_auth(auth);
    let content = serde_json::to_string_pretty(&runtime_creds)
        .map_err(|error| format!("Failed to serialize oauth_creds.json: {error}"))?;
    fs::write(auth_path, format!("{content}\n"))
        .map_err(|error| format!("Failed to write Gemini CLI oauth_creds.json: {error}"))
}

fn gemini_cli_user_agent() -> String {
    let os = if cfg!(target_os = "windows") {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        std::env::consts::ARCH
    };
    format!("GeminiCLI/0.31.0/unknown ({os}; {arch})")
}

async fn call_gemini_code_assist(
    client: &reqwest::Client,
    access_token: &str,
    endpoint: &str,
    body: &Value,
) -> Result<Value, String> {
    let response = client
        .post(format!("{GEMINI_CODE_ASSIST_URL}:{endpoint}"))
        .bearer_auth(access_token)
        .header("Content-Type", "application/json")
        .header("User-Agent", gemini_cli_user_agent())
        .header(
            "X-Goog-Api-Client",
            "google-genai-sdk/1.41.0 gl-node/v22.19.0",
        )
        .json(body)
        .send()
        .await
        .map_err(|error| format!("Failed to call Gemini Code Assist {endpoint}: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed to read Gemini Code Assist response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Gemini Code Assist {endpoint} failed with HTTP {status}: {body}"
        ));
    }
    serde_json::from_str(&body)
        .map_err(|error| format!("Failed to parse Gemini Code Assist {endpoint} response: {error}"))
}

fn code_assist_metadata() -> Value {
    json!({
        "ideType": "IDE_UNSPECIFIED",
        "platform": "PLATFORM_UNSPECIFIED",
        "pluginType": "GEMINI",
    })
}

fn extract_project_id(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.trim().to_string()).filter(|value| !value.is_empty()),
        Value::Object(object) => object
            .get("id")
            .or_else(|| object.get("projectId"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

fn default_tier_id(load_response: &Value) -> String {
    load_response
        .get("allowedTiers")
        .and_then(Value::as_array)
        .and_then(|tiers| {
            tiers.iter().find_map(|tier| {
                let is_default = tier
                    .get("isDefault")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if !is_default {
                    return None;
                }
                tier.get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
        })
        .unwrap_or_else(|| "legacy-tier".to_string())
}

async fn resolve_gemini_project_and_tier(
    client: &reqwest::Client,
    access_token: &str,
    requested_project_id: Option<&str>,
) -> Result<(Option<String>, Option<String>), String> {
    let metadata = code_assist_metadata();
    let mut load_body = json!({ "metadata": metadata.clone() });
    if let Some(project_id) = requested_project_id.filter(|value| !value.trim().is_empty()) {
        load_body["cloudaicompanionProject"] = Value::String(project_id.trim().to_string());
    }
    let load_response =
        call_gemini_code_assist(client, access_token, "loadCodeAssist", &load_body).await?;
    let tier_id = default_tier_id(&load_response);
    let mut project_id = requested_project_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            load_response
                .get("cloudaicompanionProject")
                .and_then(extract_project_id)
        });

    if project_id.is_none() {
        let auto_body = json!({
            "tierId": tier_id,
            "metadata": metadata,
        });
        for _ in 0..15 {
            let onboard_response =
                call_gemini_code_assist(client, access_token, "onboardUser", &auto_body).await?;
            if onboard_response
                .get("done")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                project_id = onboard_response
                    .get("response")
                    .and_then(|response| response.get("cloudaicompanionProject"))
                    .and_then(extract_project_id);
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    if let Some(project_id) = project_id.clone() {
        let onboard_body = json!({
            "tierId": tier_id,
            "metadata": code_assist_metadata(),
            "cloudaicompanionProject": project_id,
        });
        for _ in 0..12 {
            let onboard_response =
                call_gemini_code_assist(client, access_token, "onboardUser", &onboard_body).await?;
            if onboard_response
                .get("done")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                if let Some(response_project_id) = onboard_response
                    .get("response")
                    .and_then(|response| response.get("cloudaicompanionProject"))
                    .and_then(extract_project_id)
                {
                    return Ok((Some(response_project_id), Some(tier_id)));
                }
                return Ok((Some(project_id), Some(tier_id)));
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    Ok((project_id, Some(tier_id)))
}

fn classify_gemini_model(model_id: &str) -> &str {
    if model_id.contains("flash-lite") {
        "Flash Lite"
    } else if model_id.contains("flash") {
        "Flash"
    } else if model_id.contains("pro") {
        "Pro"
    } else {
        model_id
    }
}

fn parse_reset_time(value: Option<&str>) -> Option<i64> {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp())
}

fn format_remaining_percent(value: f64) -> String {
    format!("{:.0}%", (value * 100.0).clamp(0.0, 100.0))
}

async fn fetch_quota_snapshot(
    db_state: &DbState,
    auth: &Value,
) -> Result<GeminiQuotaSnapshot, String> {
    let access_token = auth_access_token(auth)
        .ok_or_else(|| "Gemini official account is missing access token".to_string())?;
    let client = http_client::client_with_timeout(db_state, 30).await?;
    let (resolved_project_id, plan_type) =
        resolve_gemini_project_and_tier(&client, &access_token, auth_project_id(auth).as_deref())
            .await?;
    let mut quota_body = json!({});
    if let Some(project_id) = resolved_project_id.as_deref() {
        quota_body["project"] = Value::String(project_id.to_string());
    }
    let quota_response =
        call_gemini_code_assist(&client, &access_token, "retrieveUserQuota", &quota_body).await?;
    let mut buckets: BTreeMap<String, (f64, Option<i64>)> = BTreeMap::new();

    for bucket in quota_response
        .get("buckets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let model_id = bucket
            .get("modelId")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let category = classify_gemini_model(model_id).to_string();
        let remaining = bucket
            .get("remainingFraction")
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        let reset_at = parse_reset_time(bucket.get("resetTime").and_then(Value::as_str));
        let entry = buckets.entry(category).or_insert((remaining, reset_at));
        if remaining < entry.0 {
            *entry = (remaining, reset_at);
        }
    }

    let order = |name: &str| match name {
        "Pro" => 0,
        "Flash" => 1,
        "Flash Lite" => 2,
        _ => 3,
    };
    let mut items: Vec<_> = buckets.into_iter().collect();
    items.sort_by_key(|(name, _)| order(name));

    let limit_weekly_reset_at = items.iter().filter_map(|(_, (_, reset))| *reset).min();
    let limit_weekly_text = if items.is_empty() {
        None
    } else {
        Some(
            items
                .into_iter()
                .map(|(name, (remaining, _))| {
                    format!("{name} {}", format_remaining_percent(remaining))
                })
                .collect::<Vec<_>>()
                .join(" · "),
        )
    };

    Ok(GeminiQuotaSnapshot {
        project_id: resolved_project_id,
        plan_type,
        limit_weekly_text,
        limit_weekly_reset_at,
    })
}

fn auth_json_from_snapshot(snapshot: &str) -> Result<Value, String> {
    serde_json::from_str(snapshot)
        .map_err(|error| format!("Failed to parse Gemini account snapshot: {error}"))
}

fn build_virtual_local_account(auth: &Value) -> GeminiCliOfficialAccount {
    let now = Local::now().to_rfc3339();
    let auth_snapshot = auth.to_string();
    GeminiCliOfficialAccount {
        id: LOCAL_OFFICIAL_ACCOUNT_ID.to_string(),
        provider_id: String::new(),
        name: LOCAL_OFFICIAL_ACCOUNT_ID.to_string(),
        kind: "local".to_string(),
        email: auth_email(auth),
        auth_snapshot: Some(auth_snapshot.clone()),
        auth_mode: Some("oauth-personal".to_string()),
        account_id: auth_project_id(auth),
        project_id: auth_project_id(auth),
        plan_type: extract_auth_string(auth, &["/plan_type", "/planType"]),
        last_refresh: extract_auth_string(auth, &["/last_refresh", "/lastRefresh"]),
        token_expires_at: auth_expiry_seconds(auth),
        access_token_preview: auth_access_token(auth).and_then(|value| mask_token_preview(&value)),
        refresh_token_preview: auth_refresh_token(auth)
            .and_then(|value| mask_token_preview(&value)),
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

fn mask_token_preview(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let char_count = trimmed.chars().count();
    if char_count <= 12 {
        return Some(trimmed.to_string());
    }
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

fn official_account_identity_matches_auth(
    account: &GeminiCliOfficialAccount,
    auth: &Value,
) -> bool {
    let local_refresh_token = auth_refresh_token(auth);
    let local_project_id = auth_project_id(auth);
    let local_email = auth_email(auth).map(|value| value.trim().to_ascii_lowercase());

    let account_refresh_token = account
        .auth_snapshot
        .as_deref()
        .and_then(|snapshot| serde_json::from_str::<Value>(snapshot).ok())
        .and_then(|snapshot| auth_refresh_token(&snapshot));

    if let (Some(local_refresh_token), Some(account_refresh_token)) = (
        local_refresh_token.as_deref(),
        account_refresh_token.as_deref(),
    ) {
        if local_refresh_token == account_refresh_token {
            return true;
        }
    }
    if let (Some(local_project_id), Some(account_project_id)) =
        (local_project_id.as_deref(), account.project_id.as_deref())
    {
        if local_project_id == account_project_id {
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
    persisted_accounts: &[GeminiCliOfficialAccount],
    local_auth: &Value,
) -> bool {
    auth_has_official_runtime(local_auth)
        && !persisted_accounts
            .iter()
            .any(|account| official_account_identity_matches_auth(account, local_auth))
}

fn build_account_content_from_auth_snapshot(
    provider_id: &str,
    auth_snapshot: &Value,
    quota_snapshot: Option<&GeminiQuotaSnapshot>,
    name_override: Option<&str>,
    last_error: Option<&str>,
) -> Result<GeminiCliOfficialAccountContent, String> {
    let now = Local::now().to_rfc3339();
    let project_id = quota_snapshot
        .and_then(|snapshot| snapshot.project_id.clone())
        .or_else(|| auth_project_id(auth_snapshot));
    let email = auth_email(auth_snapshot);
    let display_name = name_override
        .map(str::to_string)
        .or_else(|| email.clone())
        .or_else(|| project_id.clone())
        .unwrap_or_else(|| "official-account".to_string());
    let plan_type = quota_snapshot
        .and_then(|snapshot| snapshot.plan_type.clone())
        .or_else(|| extract_auth_string(auth_snapshot, &["/plan_type", "/planType"]));

    let mut snapshot = auth_snapshot.clone();
    if let Some(project_id) = project_id.as_deref() {
        snapshot["project_id"] = Value::String(project_id.to_string());
        snapshot["account_id"] = Value::String(project_id.to_string());
    }
    if let Some(plan_type) = plan_type.as_deref() {
        snapshot["plan_type"] = Value::String(plan_type.to_string());
    }

    Ok(GeminiCliOfficialAccountContent {
        provider_id: provider_id.to_string(),
        name: display_name,
        kind: "oauth".to_string(),
        email,
        auth_snapshot: serde_json::to_string(&snapshot)
            .map_err(|error| format!("Failed to serialize Gemini account snapshot: {error}"))?,
        auth_mode: Some("oauth-personal".to_string()),
        account_id: project_id.clone(),
        project_id,
        plan_type,
        last_refresh: extract_auth_string(&snapshot, &["/last_refresh", "/lastRefresh"]),
        limit_short_label: None,
        limit_5h_text: None,
        limit_weekly_text: quota_snapshot.and_then(|snapshot| snapshot.limit_weekly_text.clone()),
        limit_5h_reset_at: None,
        limit_weekly_reset_at: quota_snapshot.and_then(|snapshot| snapshot.limit_weekly_reset_at),
        last_limits_fetched_at: quota_snapshot.map(|_| now.clone()),
        last_error: last_error.map(str::to_string),
        sort_index: None,
        is_applied: false,
        created_at: now.clone(),
        updated_at: now,
    })
}

async fn list_persisted_official_accounts(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
) -> Result<Vec<GeminiCliOfficialAccount>, String> {
    let result: Result<Vec<Value>, _> = db
        .query(
            "SELECT *, type::string(id) as id FROM gemini_cli_official_account WHERE provider_id = $provider_id ORDER BY sort_index ASC, created_at ASC",
        )
        .bind(("provider_id", provider_id.to_string()))
        .await
        .map_err(|error| format!("Failed to query Gemini official accounts: {error}"))?
        .take(0);

    result
        .map(|records| {
            records
                .into_iter()
                .map(adapter::from_db_value_official_account)
                .collect()
        })
        .map_err(|error| format!("Failed to deserialize Gemini official accounts: {error}"))
}

async fn load_official_account(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    account_id: &str,
) -> Result<GeminiCliOfficialAccount, String> {
    let record_id = db_record_id("gemini_cli_official_account", account_id);
    let result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|error| format!("Failed to query Gemini official account: {error}"))?
        .take(0);

    result
        .map_err(|error| format!("Failed to deserialize Gemini official account: {error}"))?
        .first()
        .cloned()
        .map(adapter::from_db_value_official_account)
        .ok_or_else(|| format!("Gemini official account '{}' not found", account_id))
}

async fn save_official_account(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    content: &GeminiCliOfficialAccountContent,
) -> Result<GeminiCliOfficialAccount, String> {
    let record_id = db_record_id("gemini_cli_official_account", &db_new_id());
    db.query(&format!("CREATE {} CONTENT $data", record_id))
        .bind(("data", adapter::to_db_value_official_account(content)))
        .await
        .map_err(|error| format!("Failed to create Gemini official account: {error}"))?;
    let result: Result<Vec<Value>, _> = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|error| format!("Failed to query created Gemini official account: {error}"))?
        .take(0);
    result
        .map_err(|error| format!("Failed to deserialize created Gemini official account: {error}"))?
        .first()
        .cloned()
        .map(adapter::from_db_value_official_account)
        .ok_or_else(|| "Failed to load created Gemini official account".to_string())
}

async fn find_matching_official_account(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
    auth: &Value,
) -> Result<Option<GeminiCliOfficialAccount>, String> {
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
        "UPDATE gemini_cli_official_account SET is_applied = false, updated_at = $now WHERE is_applied = true",
    )
    .bind(("now", now.clone()))
    .await
    .map_err(|error| format!("Failed to clear Gemini official account apply state: {error}"))?;

    if let Some(account_id) = account_id {
        let record_id = db_record_id("gemini_cli_official_account", account_id);
        db.query(&format!(
            "UPDATE {} SET is_applied = true, updated_at = $now",
            record_id
        ))
        .bind(("now", now))
        .await
        .map_err(|error| format!("Failed to mark Gemini official account as applied: {error}"))?;
    }
    Ok(())
}

async fn persist_account_content(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    account: &GeminiCliOfficialAccount,
    content: GeminiCliOfficialAccountContent,
) -> Result<GeminiCliOfficialAccount, String> {
    let record_id = db_record_id("gemini_cli_official_account", &account.id);
    db.query(&format!("UPDATE {} CONTENT $data", record_id))
        .bind((
            "data",
            adapter::to_db_value_official_account(&GeminiCliOfficialAccountContent {
                is_applied: account.is_applied,
                created_at: account.created_at.clone(),
                sort_index: account.sort_index,
                updated_at: Local::now().to_rfc3339(),
                ..content
            }),
        ))
        .await
        .map_err(|error| format!("Failed to update Gemini official account: {error}"))?;
    load_official_account(db, &account.id).await
}

async fn persist_account_error(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    account_id: &str,
    error_message: &str,
) -> Result<GeminiCliOfficialAccount, String> {
    let now = Local::now().to_rfc3339();
    let record_id = db_record_id("gemini_cli_official_account", account_id);
    db.query(&format!(
        "UPDATE {} SET last_error = $last_error, updated_at = $updated_at",
        record_id
    ))
    .bind(("last_error", Some(error_message.to_string())))
    .bind(("updated_at", now))
    .await
    .map_err(|error| format!("Failed to update Gemini official account error: {error}"))?;
    load_official_account(db, account_id).await
}

fn assign_provider_id(
    mut account: GeminiCliOfficialAccount,
    provider_id: &str,
) -> GeminiCliOfficialAccount {
    account.provider_id = provider_id.to_string();
    account
}

fn emit_sync_requests<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-geminicli", ());
}

pub async fn ensure_gemini_cli_provider_has_no_official_accounts(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    provider_id: &str,
) -> Result<(), String> {
    let accounts = list_persisted_official_accounts(db, provider_id).await?;
    if accounts.is_empty() {
        Ok(())
    } else {
        Err(
            "Delete Gemini official accounts before changing or deleting this official provider"
                .to_string(),
        )
    }
}

#[tauri::command]
pub async fn list_gemini_cli_official_accounts(
    state: tauri::State<'_, DbState>,
    provider_id: String,
) -> Result<Vec<GeminiCliOfficialAccount>, String> {
    let db = state.db();
    let provider = query_provider_by_id(&db, &provider_id).await?;
    let mut accounts = list_persisted_official_accounts(&db, &provider_id).await?;
    let local_auth = read_oauth_creds_from_disk(&db).await?;
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
pub async fn start_gemini_cli_official_account_oauth(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<GeminiCliOfficialAccount, String> {
    let db = state.db();
    let provider = query_provider_by_id(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Gemini providers can add official accounts".to_string());
    }

    let oauth_client = gemini_oauth_client();
    let oauth_state = generate_random_urlsafe(32);
    let redirect_uri = build_oauth_redirect_uri(GEMINI_OAUTH_DEFAULT_PORT);
    let authorize_url =
        build_gemini_authorize_url(&oauth_client.client_id, &redirect_uri, &oauth_state);

    open_browser(&authorize_url)?;
    let authorization_code =
        tokio::task::spawn_blocking(move || wait_for_oauth_callback(&oauth_state))
            .await
            .map_err(|error| format!("Gemini OAuth callback task failed: {error}"))??;

    let token_response = exchange_authorization_code(
        state.inner(),
        &oauth_client,
        &authorization_code,
        &redirect_uri,
    )
    .await?;
    let user_info = fetch_user_info(state.inner(), &token_response.access_token)
        .await
        .ok();
    let initial_snapshot =
        build_auth_snapshot(&token_response, None, user_info.as_ref(), None, None);
    let quota_result = fetch_quota_snapshot(state.inner(), &initial_snapshot).await;
    let refreshed_snapshot = match quota_result.as_ref() {
        Ok(snapshot) => build_auth_snapshot(
            &token_response,
            Some(&initial_snapshot),
            user_info.as_ref(),
            snapshot.project_id.as_deref(),
            snapshot.plan_type.as_deref(),
        ),
        Err(_) => initial_snapshot,
    };
    let content = build_account_content_from_auth_snapshot(
        &provider_id,
        &refreshed_snapshot,
        quota_result.as_ref().ok(),
        None,
        quota_result.as_ref().err().map(String::as_str),
    )?;
    let account = save_official_account(&db, &content).await?;

    let _ = app.emit("config-changed", "window");
    Ok(account)
}

#[tauri::command]
pub async fn save_gemini_cli_official_local_account(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<GeminiCliOfficialAccount, String> {
    let db = state.db();
    let provider = query_provider_by_id(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Gemini providers can save local official accounts".to_string());
    }

    let local_auth = read_oauth_creds_from_disk(&db).await?;
    if !auth_has_official_runtime(&local_auth) {
        return Err("Current oauth_creds.json does not contain a Gemini OAuth login".to_string());
    }
    let refreshed_auth = ensure_fresh_auth_snapshot(state.inner(), &local_auth).await?;
    if refreshed_auth != local_auth {
        write_oauth_creds_to_disk(&db, &refreshed_auth).await?;
    }

    let quota_result = fetch_quota_snapshot(state.inner(), &refreshed_auth).await;
    if let Some(existing_account) =
        find_matching_official_account(&db, &provider_id, &refreshed_auth).await?
    {
        let content = build_account_content_from_auth_snapshot(
            &provider_id,
            &refreshed_auth,
            quota_result.as_ref().ok(),
            Some(&existing_account.name),
            quota_result.as_ref().err().map(String::as_str),
        )?;
        let account = persist_account_content(&db, &existing_account, content).await?;
        if provider.is_applied {
            update_official_account_apply_status(&db, Some(&account.id)).await?;
        }
        let _ = app.emit("config-changed", "window");
        return load_official_account(&db, &account.id).await;
    }

    let mut content = build_account_content_from_auth_snapshot(
        &provider_id,
        &refreshed_auth,
        quota_result.as_ref().ok(),
        None,
        quota_result.as_ref().err().map(String::as_str),
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
pub async fn apply_gemini_cli_official_account(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
    account_id: String,
) -> Result<(), String> {
    let db = state.db();
    let provider = query_provider_by_id(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Gemini providers can apply official accounts".to_string());
    }
    if provider.is_disabled {
        return Err(format!(
            "Provider '{}' is disabled and cannot be applied",
            provider_id
        ));
    }

    if account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        let local_auth = read_oauth_creds_from_disk(&db).await?;
        if !auth_has_official_runtime(&local_auth) {
            return Err(
                "Current oauth_creds.json does not contain a Gemini OAuth login".to_string(),
            );
        }
        let refreshed_auth = ensure_fresh_auth_snapshot(state.inner(), &local_auth).await?;
        write_oauth_creds_to_disk(&db, &refreshed_auth).await?;
        apply_config_internal(&db, &app, &provider_id, false).await?;
        update_official_account_apply_status(&db, None).await?;
    } else {
        let account = load_official_account(&db, &account_id).await?;
        if account.provider_id != provider_id {
            return Err(
                "Gemini official account does not belong to the selected provider".to_string(),
            );
        }
        let snapshot = account
            .auth_snapshot
            .as_deref()
            .ok_or_else(|| "Gemini official account snapshot is missing".to_string())?;
        let snapshot_auth = auth_json_from_snapshot(snapshot)?;
        let refreshed_auth = ensure_fresh_auth_snapshot(state.inner(), &snapshot_auth).await?;
        if refreshed_auth != snapshot_auth {
            let content = build_account_content_from_auth_snapshot(
                &provider_id,
                &refreshed_auth,
                None,
                Some(&account.name),
                account.last_error.as_deref(),
            )?;
            let _ = persist_account_content(&db, &account, content).await?;
        }
        write_oauth_creds_to_disk(&db, &refreshed_auth).await?;
        apply_config_internal(&db, &app, &provider_id, false).await?;
        update_official_account_apply_status(&db, Some(&account_id)).await?;
    }

    let _ = app.emit("config-changed", "window");
    emit_sync_requests(&app);
    Ok(())
}

#[tauri::command]
pub async fn delete_gemini_cli_official_account(
    state: tauri::State<'_, DbState>,
    app: tauri::AppHandle,
    provider_id: String,
    account_id: String,
) -> Result<(), String> {
    let db = state.db();
    if account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        return Err("The local Gemini official account cannot be deleted".to_string());
    }
    let account = load_official_account(&db, &account_id).await?;
    if account.provider_id != provider_id {
        return Err("Gemini official account does not belong to the selected provider".to_string());
    }
    if account.is_applied {
        return Err("The applied Gemini official account cannot be deleted".to_string());
    }
    let record_id = db_record_id("gemini_cli_official_account", &account_id);
    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|error| format!("Failed to delete Gemini official account: {error}"))?;
    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn refresh_gemini_cli_official_account_limits(
    state: tauri::State<'_, DbState>,
    provider_id: String,
    account_id: String,
) -> Result<GeminiCliOfficialAccount, String> {
    let db = state.db();
    let provider = query_provider_by_id(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err(
            "Only official Gemini providers can refresh official account quota".to_string(),
        );
    }

    if account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        let local_auth = read_oauth_creds_from_disk(&db).await?;
        if !auth_has_official_runtime(&local_auth) {
            return Err(
                "Current oauth_creds.json does not contain a Gemini OAuth login".to_string(),
            );
        }
        let refreshed_auth = ensure_fresh_auth_snapshot(state.inner(), &local_auth).await?;
        if refreshed_auth != local_auth {
            write_oauth_creds_to_disk(&db, &refreshed_auth).await?;
        }
        let quota_snapshot = fetch_quota_snapshot(state.inner(), &refreshed_auth).await?;
        let mut account =
            assign_provider_id(build_virtual_local_account(&refreshed_auth), &provider_id);
        account.project_id = quota_snapshot.project_id.clone().or(account.project_id);
        account.account_id = account.project_id.clone();
        account.plan_type = quota_snapshot.plan_type;
        account.limit_weekly_text = quota_snapshot.limit_weekly_text;
        account.limit_weekly_reset_at = quota_snapshot.limit_weekly_reset_at;
        account.last_limits_fetched_at = Some(Local::now().to_rfc3339());
        return Ok(account);
    }

    let account = load_official_account(&db, &account_id).await?;
    if account.provider_id != provider_id {
        return Err("Gemini official account does not belong to the selected provider".to_string());
    }
    let snapshot = account
        .auth_snapshot
        .as_deref()
        .ok_or_else(|| "Gemini official account snapshot is missing".to_string())?;
    let snapshot_auth = auth_json_from_snapshot(snapshot)?;
    let refreshed_auth = ensure_fresh_auth_snapshot(state.inner(), &snapshot_auth).await?;
    match fetch_quota_snapshot(state.inner(), &refreshed_auth).await {
        Ok(quota_snapshot) => {
            let content = build_account_content_from_auth_snapshot(
                &provider_id,
                &refreshed_auth,
                Some(&quota_snapshot),
                Some(&account.name),
                None,
            )?;
            persist_account_content(&db, &account, content).await
        }
        Err(error) => persist_account_error(&db, &account_id, &error).await,
    }
}

fn extract_token_from_auth_snapshot(auth: &Value, token_kind: &str) -> Result<String, String> {
    match token_kind {
        "access" => auth_access_token(auth),
        "refresh" => auth_refresh_token(auth),
        _ => return Err("Unsupported Gemini official account token kind".to_string()),
    }
    .ok_or_else(|| format!("Gemini official account {} token is missing", token_kind))
}

fn copy_text_to_clipboard(value: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("Failed to access system clipboard: {error}"))?;
    clipboard
        .set_text(value.to_string())
        .map_err(|error| format!("Failed to copy token to clipboard: {error}"))
}

#[tauri::command]
pub async fn copy_gemini_cli_official_account_token(
    state: tauri::State<'_, DbState>,
    input: GeminiCliOfficialAccountTokenCopyInput,
) -> Result<(), String> {
    let db = state.db();
    let provider = query_provider_by_id(&db, &input.provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Gemini providers can copy official account tokens".to_string());
    }

    let auth = if input.account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        let local_auth = read_oauth_creds_from_disk(&db).await?;
        if !auth_has_official_runtime(&local_auth) {
            return Err(
                "Current oauth_creds.json does not contain a Gemini OAuth login".to_string(),
            );
        }
        local_auth
    } else {
        let account = load_official_account(&db, &input.account_id).await?;
        if account.provider_id != input.provider_id {
            return Err(
                "Gemini official account does not belong to the selected provider".to_string(),
            );
        }
        let snapshot = account
            .auth_snapshot
            .as_deref()
            .ok_or_else(|| "Gemini official account snapshot is missing".to_string())?;
        auth_json_from_snapshot(snapshot)?
    };

    let token = extract_token_from_auth_snapshot(&auth, input.token_kind.trim())?;
    copy_text_to_clipboard(&token)?;
    Ok(())
}
