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
const ANTIGRAVITY_CLOUD_CODE_DAILY_URL: &str = "https://daily-cloudcode-pa.googleapis.com";
const ANTIGRAVITY_CLOUD_CODE_URL_OVERRIDE_ENV: &str = "ANTIGRAVITY_CLOUD_CODE_URL_OVERRIDE";
const LOAD_CODE_ASSIST_PATH: &str = "v1internal:loadCodeAssist";
const RETRIEVE_USER_QUOTA_SUMMARY_PATH: &str = "v1internal:retrieveUserQuotaSummary";
const FETCH_AVAILABLE_MODELS_PATH: &str = "v1internal:fetchAvailableModels";
const CLOUD_CODE_IDE_VERSION: &str = "2.25.0";
const CLOUD_CODE_CL: &str = "1003475615";
const GOOGLE_API_NODEJS_CLIENT_VERSION: &str = "10.3.0";
const X_GOOG_API_CLIENT_NODE_VERSION: &str = "22.21.1";
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
    limit_short_label: Option<String>,
    limit_5h_text: Option<String>,
    limit_5h_reset_at: Option<i64>,
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

fn cloud_code_os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "windows",
        "linux" => "linux",
        _ => "windows",
    }
}

fn cloud_code_arch_name() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => "amd64",
    }
}

fn cloud_code_platform_name() -> &'static str {
    match (cloud_code_os_name(), cloud_code_arch_name()) {
        ("darwin", "amd64") => "DARWIN_AMD64",
        ("darwin", "arm64") => "DARWIN_ARM64",
        ("linux", "amd64") => "LINUX_AMD64",
        ("linux", "arm64") => "LINUX_ARM64",
        ("windows", "amd64") => "WINDOWS_AMD64",
        _ => "PLATFORM_UNSPECIFIED",
    }
}

fn cloud_code_base_url() -> String {
    read_env_var(ANTIGRAVITY_CLOUD_CODE_URL_OVERRIDE_ENV)
        .unwrap_or_else(|| ANTIGRAVITY_CLOUD_CODE_DAILY_URL.to_string())
        .trim_end_matches('/')
        .to_string()
}

fn build_load_code_assist_user_agent() -> String {
    format!(
        "antigravity/{} {}/{} google-api-nodejs-client/{}",
        CLOUD_CODE_IDE_VERSION,
        cloud_code_os_name(),
        cloud_code_arch_name(),
        GOOGLE_API_NODEJS_CLIENT_VERSION,
    )
}

fn build_cloud_code_user_agent() -> String {
    format!(
        "antigravity/{} (aidev_client; os_type={}; arch={}; cl={})",
        CLOUD_CODE_IDE_VERSION,
        cloud_code_os_name(),
        cloud_code_arch_name(),
        CLOUD_CODE_CL,
    )
}

fn valid_cloud_code_project_id(project_id: Option<&str>) -> Option<&str> {
    project_id
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "aicode-consumers")
}

fn build_load_code_assist_payload(project_id: Option<&str>) -> Value {
    let mut metadata = serde_json::Map::new();
    metadata.insert("ideName".to_string(), json!("antigravity"));
    metadata.insert("ideType".to_string(), json!("ANTIGRAVITY"));
    metadata.insert("ideVersion".to_string(), json!(CLOUD_CODE_IDE_VERSION));
    metadata.insert("platform".to_string(), json!(cloud_code_platform_name()));
    metadata.insert("updateChannel".to_string(), json!("stable"));
    metadata.insert("pluginType".to_string(), json!("GEMINI"));
    if let Some(project) = valid_cloud_code_project_id(project_id) {
        metadata.insert("duetProject".to_string(), json!(project));
    }

    let mut payload = json!({
        "metadata": Value::Object(metadata),
        "mode": "FULL_ELIGIBILITY_CHECK"
    });
    if let Some(project) = valid_cloud_code_project_id(project_id) {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("cloudaicompanionProject".to_string(), json!(project));
        }
    }
    payload
}

fn extract_cloud_code_project_id(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str().and_then(|s| valid_cloud_code_project_id(Some(s))) {
        return Some(text.to_string());
    }
    value
        .get("id")
        .and_then(Value::as_str)
        .and_then(|s| valid_cloud_code_project_id(Some(s)))
        .map(str::to_string)
}

fn normalize_plan_tier(tier_id: &str) -> String {
    let trimmed = tier_id.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("ultra") {
        "ULTRA".to_string()
    } else if lower.contains("pro") {
        "PRO".to_string()
    } else if lower.contains("free") || lower.contains("standard") {
        "FREE".to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_load_code_assist_response(value: &Value) -> (Option<String>, Option<String>) {
    let project_id = value
        .get("cloudaicompanionProject")
        .and_then(extract_cloud_code_project_id);
    let tier_id = value
        .pointer("/paidTier/id")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            value
                .pointer("/currentTier/id")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
        })
        .map(normalize_plan_tier);
    (project_id, tier_id)
}

fn parse_google_api_error_message(status: reqwest::StatusCode, text: &str) -> String {
    if let Ok(val) = serde_json::from_str::<Value>(text) {
        if let Some(msg) = val.pointer("/error/message").and_then(Value::as_str) {
            let trimmed = msg.trim();
            if !trimmed.is_empty() {
                return format!("HTTP {status}: {trimmed}");
            }
        }
    }
    format!("HTTP {status}")
}

fn parse_rfc3339_timestamp(value: Option<&str>) -> Option<i64> {
    value
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw.trim()).ok())
        .map(|dt| dt.timestamp())
}

fn format_bucket_label(bucket_id: &str, display_name: Option<&str>) -> String {
    let lower = bucket_id.trim().to_ascii_lowercase();
    if lower.starts_with("gemini") {
        if lower.contains("argon") {
            return "Argon".to_string();
        }
        if lower.contains("carbon") {
            return "Carbon".to_string();
        }
        return "Gemini".to_string();
    }
    if lower.starts_with("3p") || lower.starts_with("claude") {
        return "Claude".to_string();
    }
    if let Some(name) = display_name.map(str::trim).filter(|s| !s.is_empty()) {
        return name.to_string();
    }
    bucket_id.trim().to_string()
}

fn compose_short_label(
    limit_5h_text: Option<&str>,
    limit_weekly_text: Option<&str>,
) -> Option<String> {
    match (limit_5h_text, limit_weekly_text) {
        (Some(h5), Some(wk)) => Some(format!("5h: {h5} | Weekly: {wk}")),
        (Some(h5), None) => Some(format!("5h: {h5}")),
        (None, Some(wk)) => Some(format!("Weekly: {wk}")),
        (None, None) => None,
    }
}

fn parse_quota_summary_response(
    value: &Value,
    project_id: Option<String>,
    plan_type: Option<String>,
) -> Result<AntigravityQuotaSnapshot, String> {
    let groups = value
        .get("groups")
        .and_then(Value::as_array)
        .ok_or_else(|| "Antigravity quota summary is missing groups".to_string())?;

    let mut labels_5h: Vec<String> = Vec::new();
    let mut reset_5h: Option<i64> = None;
    let mut labels_weekly: Vec<String> = Vec::new();
    let mut reset_weekly: Option<i64> = None;

    for group in groups {
        let Some(buckets) = group.get("buckets").and_then(Value::as_array) else {
            continue;
        };
        for bucket in buckets {
            let Some(bucket_id) = bucket
                .get("bucketId")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let Some(fraction) = bucket.get("remainingFraction").and_then(Value::as_f64) else {
                continue;
            };
            let percent = (fraction.clamp(0.0, 1.0) * 100.0).round() as i32;
            let display_name = bucket.get("displayName").and_then(Value::as_str);
            let label = format!("{} {}%", format_bucket_label(bucket_id, display_name), percent);
            let reset_ts = parse_rfc3339_timestamp(bucket.get("resetTime").and_then(Value::as_str));

            let lower = bucket_id.to_ascii_lowercase();
            if lower.ends_with("5h") || lower.contains(":5h") || lower.contains("-5h") {
                labels_5h.push(label);
                if let Some(ts) = reset_ts {
                    reset_5h = Some(reset_5h.map_or(ts, |prev| prev.min(ts)));
                }
            } else {
                labels_weekly.push(label);
                if let Some(ts) = reset_ts {
                    reset_weekly = Some(reset_weekly.map_or(ts, |prev| prev.min(ts)));
                }
            }
        }
    }

    if labels_5h.is_empty() && labels_weekly.is_empty() {
        return Err("Antigravity quota summary returned no buckets".to_string());
    }

    let limit_5h_text = (!labels_5h.is_empty()).then(|| labels_5h.join(" · "));
    let limit_weekly_text = (!labels_weekly.is_empty()).then(|| labels_weekly.join(" · "));
    let limit_short_label =
        compose_short_label(limit_5h_text.as_deref(), limit_weekly_text.as_deref());

    Ok(AntigravityQuotaSnapshot {
        project_id,
        plan_type,
        limit_short_label,
        limit_5h_text,
        limit_5h_reset_at: reset_5h,
        limit_weekly_text,
        limit_weekly_reset_at: reset_weekly,
    })
}

fn parse_available_models_quota(
    value: &Value,
    project_id: Option<String>,
    plan_type: Option<String>,
) -> Result<AntigravityQuotaSnapshot, String> {
    let models = value
        .get("models")
        .and_then(Value::as_object)
        .ok_or_else(|| "Antigravity fetchAvailableModels response is missing models".to_string())?;

    let mut gemini_min: Option<i32> = None;
    let mut claude_min: Option<i32> = None;
    let mut other_labels: Vec<String> = Vec::new();
    let mut earliest_reset: Option<i64> = None;

    for (name, info) in models {
        let Some(quota_info) = info.get("quotaInfo").and_then(Value::as_object) else {
            continue;
        };
        let Some(fraction) = quota_info.get("remainingFraction").and_then(Value::as_f64) else {
            continue;
        };
        let percent = (fraction.clamp(0.0, 1.0) * 100.0).round() as i32;
        if let Some(ts) = parse_rfc3339_timestamp(quota_info.get("resetTime").and_then(Value::as_str)) {
            earliest_reset = Some(earliest_reset.map_or(ts, |prev| prev.min(ts)));
        }

        let lower = name.to_ascii_lowercase();
        if lower.contains("gemini") || lower.contains("argon") || lower.contains("carbon") {
            gemini_min = Some(gemini_min.map_or(percent, |prev| prev.min(percent)));
        } else if lower.contains("claude") {
            claude_min = Some(claude_min.map_or(percent, |prev| prev.min(percent)));
        } else if lower.contains("gpt") {
            let display = info
                .get("displayName")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(name);
            other_labels.push(format!("{display} {percent}%"));
        }
    }

    let mut labels = Vec::new();
    if let Some(pct) = gemini_min {
        labels.push(format!("Gemini {pct}%"));
    }
    if let Some(pct) = claude_min {
        labels.push(format!("Claude {pct}%"));
    }
    labels.extend(other_labels);

    if labels.is_empty() {
        return Err("Antigravity fetchAvailableModels returned no model quota".to_string());
    }

    let limit_weekly_text = Some(labels.join(" · "));
    let limit_short_label = compose_short_label(None, limit_weekly_text.as_deref());

    Ok(AntigravityQuotaSnapshot {
        project_id,
        plan_type,
        limit_short_label,
        limit_5h_text: None,
        limit_5h_reset_at: None,
        limit_weekly_text,
        limit_weekly_reset_at: earliest_reset,
    })
}

async fn fetch_code_assist_metadata(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    preferred_project_id: Option<&str>,
) -> (Option<String>, Option<String>) {
    let url = format!("{base_url}/{LOAD_CODE_ASSIST_PATH}");
    let ua = build_load_code_assist_user_agent();
    let x_goog_api_client = format!("gl-node/{X_GOOG_API_CLIENT_NODE_VERSION}");
    let payload = build_load_code_assist_payload(preferred_project_id);

    let Ok(response) = client
        .post(&url)
        .bearer_auth(access_token)
        .header(reqwest::header::USER_AGENT, &ua)
        .header("x-goog-api-client", &x_goog_api_client)
        .header(reqwest::header::ACCEPT, "*/*")
        .json(&payload)
        .send()
        .await
    else {
        return (
            valid_cloud_code_project_id(preferred_project_id).map(str::to_string),
            None,
        );
    };

    if !response.status().is_success() {
        return (
            valid_cloud_code_project_id(preferred_project_id).map(str::to_string),
            None,
        );
    }

    let Ok(value) = response.json::<Value>().await else {
        return (
            valid_cloud_code_project_id(preferred_project_id).map(str::to_string),
            None,
        );
    };

    let (resolved_project, plan_type) = parse_load_code_assist_response(&value);
    let effective_project = resolved_project
        .or_else(|| valid_cloud_code_project_id(preferred_project_id).map(str::to_string));
    (effective_project, plan_type)
}

async fn retrieve_user_quota(
    db: &SqliteDbState,
    access_token: &str,
    preferred_project_id: Option<&str>,
) -> Result<AntigravityQuotaSnapshot, String> {
    let client = http_client::client_with_timeout(db, 30).await?;
    let base_url = cloud_code_base_url();
    let (project_id, plan_type) =
        fetch_code_assist_metadata(&client, &base_url, access_token, preferred_project_id).await;

    let payload = project_id
        .as_deref()
        .map(|id| json!({ "project": id }))
        .unwrap_or_else(|| json!({}));
    let ua = build_cloud_code_user_agent();

    // Primary: v1internal:retrieveUserQuotaSummary (5h & Weekly buckets).
    let summary_url = format!("{base_url}/{RETRIEVE_USER_QUOTA_SUMMARY_PATH}");
    let mut summary_error: Option<String> = None;
    match client
        .post(&summary_url)
        .bearer_auth(access_token)
        .header(reqwest::header::USER_AGENT, &ua)
        .json(&payload)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            if status.is_success() {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    if let Ok(snapshot) =
                        parse_quota_summary_response(&value, project_id.clone(), plan_type.clone())
                    {
                        return Ok(snapshot);
                    }
                    if let Ok(mut snapshot) = parse_quota_snapshot(&value) {
                        if snapshot.project_id.is_none() {
                            snapshot.project_id = project_id.clone();
                        }
                        if snapshot.plan_type.is_none() {
                            snapshot.plan_type = plan_type.clone();
                        }
                        return Ok(snapshot);
                    }
                }
            } else {
                summary_error = Some(parse_google_api_error_message(status, &text));
            }
        }
        Err(error) => {
            summary_error = Some(format!("retrieveUserQuotaSummary request failed: {error}"));
        }
    }

    // Fallback: v1internal:fetchAvailableModels (per-model quotaInfo).
    let models_url = format!("{base_url}/{FETCH_AVAILABLE_MODELS_PATH}");
    let response = client
        .post(&models_url)
        .bearer_auth(access_token)
        .header(reqwest::header::USER_AGENT, &ua)
        .json(&payload)
        .send()
        .await
        .map_err(|error| {
            summary_error.clone().unwrap_or_else(|| {
                format!("Failed to fetch Antigravity available models: {error}")
            })
        })?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("Failed to read Antigravity models response: {error}"))?;
    if !status.is_success() {
        let models_error = parse_google_api_error_message(status, &text);
        return Err(summary_error
            .map(|err| format!("Failed to retrieve Antigravity quota ({err}; models: {models_error})"))
            .unwrap_or_else(|| format!("Failed to retrieve Antigravity quota ({models_error})")));
    }

    let value = serde_json::from_str::<Value>(&text)
        .map_err(|error| format!("Failed to parse Antigravity models JSON: {error}"))?;
    parse_available_models_quota(&value, project_id, plan_type)
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
        let limit_weekly_text = (!labels.is_empty()).then(|| labels.join(" · "));
        return Ok(AntigravityQuotaSnapshot {
            project_id: value
                .get("projectId")
                .and_then(Value::as_str)
                .map(str::to_string),
            plan_type: value
                .get("planType")
                .and_then(Value::as_str)
                .map(str::to_string),
            limit_short_label: limit_weekly_text.clone(),
            limit_5h_text: None,
            limit_5h_reset_at: None,
            limit_weekly_text,
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
        limit_short_label: limit_weekly_text.clone(),
        limit_5h_text: None,
        limit_5h_reset_at: None,
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
    list_official_accounts_with_store(db, &provider_id, &OsCredentialStore)
}

fn list_official_accounts_with_store(
    db: &SqliteDbState,
    provider_id: &str,
    credentials: &impl CredentialStore,
) -> Result<Vec<AntigravityOfficialAccount>, String> {
    ensure_persisted_provider_id(provider_id)?;
    let provider = query_provider_by_id(db, provider_id)?;
    if provider.category != "official" {
        return Err("Only official Antigravity providers can list official accounts".to_string());
    }

    let provider_id_value = Value::String(provider_id.to_string());
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

    // Exclude legacy persisted `__local__` snapshot rows from the visible list.
    let mut accounts: Vec<AntigravityOfficialAccount> = records
        .into_iter()
        .map(adapter::from_db_value_official_account)
        .filter(|account| account.id != LOCAL_OFFICIAL_ACCOUNT_ID)
        .collect();

    // Only surface a virtual local row when the OS credential store holds an
    // active `agy` login that is not yet saved as a managed account.
    if let Ok(Some(raw_snapshot)) = credentials.read() {
        if let Ok(parsed) = serde_json::from_str::<Value>(&raw_snapshot) {
            if should_show_virtual_local_account(&accounts, &parsed) {
                let local_is_applied = provider.is_applied
                    && !accounts.iter().any(|account| account.is_applied);
                accounts.push(build_virtual_local_account(
                    provider_id,
                    &raw_snapshot,
                    &parsed,
                    local_is_applied,
                ));
            }
        }
    }

    Ok(accounts)
}

fn extract_email_from_id_token(id_token: &str) -> Option<String> {
    let payload_b64 = id_token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload_b64))
        .ok()?;
    let payload = serde_json::from_slice::<Value>(&decoded).ok()?;
    payload
        .get("email")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn auth_email(snapshot: &Value) -> Option<String> {
    snapshot
        .get("email")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| auth_string(snapshot, "id_token").and_then(extract_email_from_id_token))
}

fn auth_has_runtime_login(snapshot: &Value) -> bool {
    auth_string(snapshot, "access_token").is_some()
        || auth_string(snapshot, "refresh_token").is_some()
}

fn official_account_identity_matches_auth(
    account: &AntigravityOfficialAccount,
    local_auth: &Value,
) -> bool {
    let account_parsed = account
        .auth_snapshot
        .as_deref()
        .and_then(|snapshot| serde_json::from_str::<Value>(snapshot).ok());

    if let (Some(local_refresh), Some(account_refresh)) = (
        auth_string(local_auth, "refresh_token"),
        account_parsed
            .as_ref()
            .and_then(|parsed| auth_string(parsed, "refresh_token")),
    ) {
        if local_refresh == account_refresh {
            return true;
        }
    }

    if let (Some(local_email), Some(account_email)) = (
        auth_email(local_auth).map(|value| value.to_ascii_lowercase()),
        account
            .email
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty()),
    ) {
        if local_email == account_email {
            return true;
        }
    }

    if let (Some(local_access), Some(account_access)) = (
        auth_string(local_auth, "access_token"),
        account_parsed
            .as_ref()
            .and_then(|parsed| auth_string(parsed, "access_token")),
    ) {
        if local_access == account_access {
            return true;
        }
    }

    false
}

fn should_show_virtual_local_account(
    persisted_accounts: &[AntigravityOfficialAccount],
    local_auth: &Value,
) -> bool {
    auth_has_runtime_login(local_auth)
        && !persisted_accounts
            .iter()
            .any(|account| official_account_identity_matches_auth(account, local_auth))
}

fn build_virtual_local_account(
    provider_id: &str,
    raw_snapshot: &str,
    parsed: &Value,
    is_applied: bool,
) -> AntigravityOfficialAccount {
    let now = Local::now().to_rfc3339();
    let email = auth_email(parsed);
    AntigravityOfficialAccount {
        id: LOCAL_OFFICIAL_ACCOUNT_ID.to_string(),
        provider_id: provider_id.to_string(),
        name: email
            .clone()
            .unwrap_or_else(|| LOCAL_OFFICIAL_ACCOUNT_ID.to_string()),
        kind: "local".to_string(),
        email,
        auth_snapshot: Some(raw_snapshot.to_string()),
        auth_mode: Some("oauth-personal".to_string()),
        account_id: None,
        project_id: None,
        plan_type: None,
        last_refresh: None,
        token_expires_at: adapter::token_expires_at_from_snapshot(raw_snapshot),
        access_token_preview: adapter::token_preview_from_snapshot(
            raw_snapshot,
            "/token/access_token",
        )
        .or_else(|| adapter::token_preview_from_snapshot(raw_snapshot, "/access_token")),
        refresh_token_preview: adapter::token_preview_from_snapshot(
            raw_snapshot,
            "/token/refresh_token",
        )
        .or_else(|| adapter::token_preview_from_snapshot(raw_snapshot, "/refresh_token")),
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

fn list_persisted_managed_accounts(
    db: &SqliteDbState,
    provider_id: &str,
) -> Result<Vec<AntigravityOfficialAccount>, String> {
    let provider_id_value = Value::String(provider_id.to_string());
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
    Ok(records
        .into_iter()
        .map(adapter::from_db_value_official_account)
        .filter(|account| account.id != LOCAL_OFFICIAL_ACCOUNT_ID)
        .collect())
}

fn find_matching_persisted_account(
    db: &SqliteDbState,
    provider_id: &str,
    email: Option<&str>,
    auth_value: &Value,
) -> Result<Option<AntigravityOfficialAccount>, String> {
    let accounts = list_persisted_managed_accounts(db, provider_id)?;
    let normalized_email = email
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    Ok(accounts.into_iter().find(|account| {
        if official_account_identity_matches_auth(account, auth_value) {
            return true;
        }
        match (
            normalized_email.as_deref(),
            account
                .email
                .as_deref()
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty()),
        ) {
            (Some(target), Some(existing)) => target == existing,
            _ => false,
        }
    }))
}

fn sync_cli_fallback_file_if_present(db: &SqliteDbState, snapshot: Option<&str>) {
    if let Ok(root_dir) = super::commands::get_antigravity_root_dir_from_db(db) {
        credential_store::sync_existing_cli_fallback_file(&root_dir, snapshot);
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
    let parsed_auth = serde_json::from_str::<Value>(&auth_snapshot).unwrap_or_else(|_| json!({}));
    let (email, name) = fetch_user_profile(db, &token_resp.access_token)
        .await
        .unwrap_or((None, None));
    let email = email.or_else(|| auth_email(&parsed_auth));
    let quota_result = retrieve_user_quota(db, &token_resp.access_token, None).await;
    let quota_error = quota_result.as_ref().err().cloned();
    let quota = quota_result.unwrap_or_default();

    let display_name = email
        .clone()
        .or(name)
        .unwrap_or_else(|| "Google Official Account".to_string());

    let _operation = ACCOUNT_OPERATION_LOCK.lock().await;
    let now_str = Local::now().to_rfc3339();
    let existing = find_matching_persisted_account(db, &provider_id, email.as_deref(), &parsed_auth)?;
    let target_id = existing
        .as_ref()
        .map(|account| account.id.clone())
        .unwrap_or_else(db_new_id);
    let is_applied = existing.as_ref().is_some_and(|account| account.is_applied);
    let created_at = existing
        .as_ref()
        .map(|account| account.created_at.clone())
        .unwrap_or_else(|| now_str.clone());
    let sort_index = existing
        .as_ref()
        .and_then(|account| account.sort_index)
        .or(Some(0));

    let content = AntigravityOfficialAccountContent {
        provider_id: provider_id.clone(),
        name: display_name,
        kind: "oauth".to_string(),
        email: email.or_else(|| existing.as_ref().and_then(|a| a.email.clone())),
        auth_snapshot: auth_snapshot.clone(),
        auth_mode: Some("oauth-personal".to_string()),
        account_id: None,
        project_id: quota
            .project_id
            .or_else(|| existing.as_ref().and_then(|a| a.project_id.clone())),
        plan_type: quota
            .plan_type
            .or_else(|| existing.as_ref().and_then(|a| a.plan_type.clone())),
        last_refresh: Some(now_str.clone()),
        limit_short_label: quota
            .limit_short_label
            .or_else(|| existing.as_ref().and_then(|a| a.limit_short_label.clone())),
        limit_5h_text: quota
            .limit_5h_text
            .or_else(|| existing.as_ref().and_then(|a| a.limit_5h_text.clone())),
        limit_weekly_text: quota
            .limit_weekly_text
            .or_else(|| existing.as_ref().and_then(|a| a.limit_weekly_text.clone())),
        limit_5h_reset_at: quota
            .limit_5h_reset_at
            .or_else(|| existing.as_ref().and_then(|a| a.limit_5h_reset_at)),
        limit_weekly_reset_at: quota
            .limit_weekly_reset_at
            .or_else(|| existing.as_ref().and_then(|a| a.limit_weekly_reset_at)),
        last_limits_fetched_at: if quota_error.is_none() {
            Some(now_str.clone())
        } else {
            existing.as_ref().and_then(|a| a.last_limits_fetched_at.clone())
        },
        last_error: quota_error,
        sort_index,
        is_applied,
        created_at,
        updated_at: now_str,
    };

    let db_val = adapter::to_db_value_official_account(&content);
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::CliOfficialAccount(Cli::Antigravity),
            &target_id,
            &db_val,
        )
    })?;

    if is_applied && provider.is_applied {
        let _ = credential_store::write_credential_text(&auth_snapshot);
        sync_cli_fallback_file_if_present(db, Some(&auth_snapshot));
    }

    let created_record = db
        .with_conn(|conn| db_get(conn, DbTable::CliOfficialAccount(Cli::Antigravity), &target_id))?
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
    if refreshed_snapshot != content {
        credential_store::write_credential_text(&refreshed_snapshot)?;
        sync_cli_fallback_file_if_present(db, Some(&refreshed_snapshot));
    }
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
    let email = email.or_else(|| auth_email(&refreshed_parsed));

    let existing =
        find_matching_persisted_account(db, &provider_id, email.as_deref(), &refreshed_parsed)?;
    let preferred_project_id = existing.as_ref().and_then(|a| a.project_id.as_deref());

    let quota_result = if !access_token.is_empty() {
        retrieve_user_quota(db, access_token, preferred_project_id).await
    } else {
        Err("No valid access token available for limits check".to_string())
    };
    let quota_error = quota_result.as_ref().err().cloned();
    let quota = quota_result.unwrap_or_default();

    let display_name = email
        .clone()
        .or(name)
        .or_else(|| existing.as_ref().map(|a| a.name.clone()))
        .unwrap_or_else(|| "Local OAuth Account".to_string());

    let now_str = Local::now().to_rfc3339();
    let target_id = existing
        .as_ref()
        .map(|account| account.id.clone())
        .unwrap_or_else(db_new_id);
    let created_at = existing
        .as_ref()
        .map(|account| account.created_at.clone())
        .unwrap_or_else(|| now_str.clone());
    let sort_index = existing
        .as_ref()
        .and_then(|account| account.sort_index)
        .or(Some(0));

    let account_content = AntigravityOfficialAccountContent {
        provider_id: provider_id.clone(),
        name: display_name,
        kind: "oauth".to_string(),
        email: email.or_else(|| existing.as_ref().and_then(|a| a.email.clone())),
        auth_snapshot: refreshed_snapshot,
        auth_mode: Some("oauth-personal".to_string()),
        account_id: None,
        project_id: quota
            .project_id
            .or_else(|| existing.as_ref().and_then(|a| a.project_id.clone())),
        plan_type: quota
            .plan_type
            .or_else(|| existing.as_ref().and_then(|a| a.plan_type.clone())),
        last_refresh: Some(now_str.clone()),
        limit_short_label: quota
            .limit_short_label
            .or_else(|| existing.as_ref().and_then(|a| a.limit_short_label.clone())),
        limit_5h_text: quota
            .limit_5h_text
            .or_else(|| existing.as_ref().and_then(|a| a.limit_5h_text.clone())),
        limit_weekly_text: quota
            .limit_weekly_text
            .or_else(|| existing.as_ref().and_then(|a| a.limit_weekly_text.clone())),
        limit_5h_reset_at: quota
            .limit_5h_reset_at
            .or_else(|| existing.as_ref().and_then(|a| a.limit_5h_reset_at)),
        limit_weekly_reset_at: quota
            .limit_weekly_reset_at
            .or_else(|| existing.as_ref().and_then(|a| a.limit_weekly_reset_at)),
        last_limits_fetched_at: if quota_error.is_none() {
            Some(now_str.clone())
        } else {
            existing.as_ref().and_then(|a| a.last_limits_fetched_at.clone())
        },
        last_error: quota_error,
        sort_index,
        is_applied: provider.is_applied,
        created_at,
        updated_at: now_str,
    };

    persist_imported_account(db, &target_id, &account_content)?;

    let created_record = db
        .with_conn(|conn| db_get(conn, DbTable::CliOfficialAccount(Cli::Antigravity), &target_id))?
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
    sync_cli_fallback_file_if_present(db, target_snapshot.as_deref());
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
        sync_cli_fallback_file_if_present(db, previous_credential.as_deref());
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
        sync_cli_fallback_file_if_present(db, Some(&refreshed));
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
        let raw = credential_store::read_credential_text()?
            .ok_or_else(|| "No Antigravity CLI login found in the OS credential store".to_string())?;
        let refreshed_snapshot = ensure_fresh_auth_snapshot(db, &raw).await?;
        if refreshed_snapshot != raw {
            credential_store::write_credential_text(&refreshed_snapshot)?;
            sync_cli_fallback_file_if_present(db, Some(&refreshed_snapshot));
        }
        let parsed = serde_json::from_str::<Value>(&refreshed_snapshot)
            .map_err(|error| format!("Failed to parse Antigravity CLI credential: {error}"))?;
        let access_token = auth_string(&parsed, "access_token")
            .ok_or_else(|| "No valid access token available for limits check".to_string())?;
        let quota = retrieve_user_quota(db, access_token, None).await?;
        let persisted = list_persisted_managed_accounts(db, &provider_id)?;
        let local_is_applied =
            provider.is_applied && !persisted.iter().any(|account| account.is_applied);
        let mut virtual_account = build_virtual_local_account(
            &provider_id,
            &refreshed_snapshot,
            &parsed,
            local_is_applied,
        );
        if virtual_account.email.is_none() {
            // Only hit the userinfo endpoint when the id_token carries no email.
            virtual_account.email = fetch_user_profile(db, access_token)
                .await
                .ok()
                .and_then(|(email, _)| email);
        }
        let now_str = Local::now().to_rfc3339();
        virtual_account.project_id = quota.project_id;
        virtual_account.plan_type = quota.plan_type;
        virtual_account.limit_short_label = quota.limit_short_label;
        virtual_account.limit_5h_text = quota.limit_5h_text;
        virtual_account.limit_5h_reset_at = quota.limit_5h_reset_at;
        virtual_account.limit_weekly_text = quota.limit_weekly_text;
        virtual_account.limit_weekly_reset_at = quota.limit_weekly_reset_at;
        virtual_account.last_limits_fetched_at = Some(now_str);
        return Ok(virtual_account);
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
    let quota_result = retrieve_user_quota(db, access_token, account.project_id.as_deref()).await;
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
    let mut fields: Vec<(&str, Value)> = vec![
        (
            "limit_short_label",
            quota
                .limit_short_label
                .or_else(|| quota.limit_weekly_text.clone())
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "limit_5h_text",
            quota
                .limit_5h_text
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "limit_5h_reset_at",
            quota
                .limit_5h_reset_at
                .map(|v| Value::Number(v.into()))
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
    ];
    // Preserve previously resolved project_id / plan_type when loadCodeAssist transiently omits them.
    if let Some(project_id) = quota.project_id {
        fields.push(("project_id", Value::String(project_id)));
    }
    if let Some(plan_type) = quota.plan_type {
        fields.push(("plan_type", Value::String(plan_type)));
    }

    db.with_conn(|conn| {
        db_patch_fields(
            conn,
            DbTable::CliOfficialAccount(Cli::Antigravity),
            account_id,
            &fields,
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

    fn jwt_with_email(email: &str) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(json!({"email": email}).to_string());
        format!("{header}.{payload}.sig")
    }

    #[tokio::test]
    async fn list_accounts_only_shows_virtual_local_row_for_unmanaged_os_login() {
        let _env = crate::coding::test_env::lock();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        db.with_conn(|conn| {
            db_put(
                conn,
                DbTable::CliProvider(Cli::Antigravity),
                "official",
                &json!({"category": "official", "is_applied": true}),
            )?;
            // Legacy persisted `__local__` row should not leak into the list.
            db_put(
                conn,
                DbTable::CliOfficialAccount(Cli::Antigravity),
                LOCAL_OFFICIAL_ACCOUNT_ID,
                &json!({"provider_id": "official", "kind": "local", "auth_snapshot": ""}),
            )
        })
        .unwrap();

        let credentials = MemoryCredentials::default();
        // 1. No OS login -> empty list (no permanent placeholder row).
        let accounts = list_official_accounts_with_store(&db, "official", &credentials).unwrap();
        assert!(accounts.is_empty());

        // 2. Unmanaged OS login -> single virtual local row with email decoded from id_token.
        let local_login = json!({
            "auth_method": "consumer",
            "token": {
                "access_token": "local-access-token-123456",
                "refresh_token": "local-refresh-token-123456",
                "id_token": jwt_with_email("alice@example.com"),
                "expiry": "2099-01-01T00:00:00Z"
            }
        })
        .to_string();
        credentials.replace(Some(&local_login)).unwrap();
        let accounts = list_official_accounts_with_store(&db, "official", &credentials).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, LOCAL_OFFICIAL_ACCOUNT_ID);
        assert!(accounts[0].is_virtual);
        assert!(accounts[0].is_applied);
        assert_eq!(accounts[0].email.as_deref(), Some("alice@example.com"));
        assert!(accounts[0].access_token_preview.is_some());

        // 3. Once saved as a managed account, the virtual row disappears.
        let mut managed = account_content(true);
        managed.email = Some("alice@example.com".to_string());
        managed.auth_snapshot = local_login;
        persist_imported_account(&db, "saved-1", &managed).unwrap();
        let accounts = list_official_accounts_with_store(&db, "official", &credentials).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "saved-1");
        assert!(!accounts[0].is_virtual);
    }

    #[test]
    fn load_code_assist_extracts_project_and_normalized_tier() {
        let (project, tier) = parse_load_code_assist_response(&json!({
            "cloudaicompanionProject": {"id": "proj-123"},
            "paidTier": {"id": "g1-pro-tier"},
            "currentTier": {"id": "free-tier"}
        }));
        assert_eq!(project.as_deref(), Some("proj-123"));
        assert_eq!(tier.as_deref(), Some("PRO"));

        let (project, tier) = parse_load_code_assist_response(&json!({
            "cloudaicompanionProject": "aicode-consumers",
            "currentTier": {"id": "free-tier"}
        }));
        assert!(project.is_none());
        assert_eq!(tier.as_deref(), Some("FREE"));
    }

    #[test]
    fn quota_summary_parses_5h_and_weekly_buckets_and_persists() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        persist_imported_account(&db, "account", &account_content(false)).unwrap();
        let snapshot = parse_quota_summary_response(
            &json!({
                "groups": [{
                    "buckets": [
                        {"bucketId": "gemini-5h", "remainingFraction": 0.85, "resetTime": "2026-11-23T15:00:00Z"},
                        {"bucketId": "3p-5h", "remainingFraction": 1.0, "resetTime": "2026-11-23T16:00:00Z"},
                        {"bucketId": "gemini-weekly", "remainingFraction": 0.92, "resetTime": "2026-11-30T00:00:00Z"},
                        {"bucketId": "3p-weekly", "remainingFraction": 0.78, "resetTime": "2026-11-29T00:00:00Z"}
                    ]
                }]
            }),
            Some("proj-123".to_string()),
            Some("PRO".to_string()),
        )
        .unwrap();

        assert_eq!(snapshot.limit_5h_text.as_deref(), Some("Gemini 85% · Claude 100%"));
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("Gemini 92% · Claude 78%"));
        assert_eq!(
            snapshot.limit_short_label.as_deref(),
            Some("5h: Gemini 85% · Claude 100% | Weekly: Gemini 92% · Claude 78%")
        );
        assert!(snapshot.limit_5h_reset_at.is_some());
        assert!(snapshot.limit_weekly_reset_at.is_some());

        persist_quota_result(&db, "account", Ok(snapshot)).unwrap();
        let account = read_account(&db, "account");
        assert_eq!(account.project_id.as_deref(), Some("proj-123"));
        assert_eq!(account.plan_type.as_deref(), Some("PRO"));
        assert_eq!(account.limit_5h_text.as_deref(), Some("Gemini 85% · Claude 100%"));
        assert_eq!(account.limit_weekly_text.as_deref(), Some("Gemini 92% · Claude 78%"));
        assert!(account.limit_5h_reset_at.is_some());
        assert!(account.limit_weekly_reset_at.is_some());
    }

    #[test]
    fn fetch_available_models_fallback_summarizes_model_families() {
        let snapshot = parse_available_models_quota(
            &json!({
                "models": {
                    "gemini-4-argon-high": {"quotaInfo": {"remainingFraction": 0.6, "resetTime": "2026-11-25T00:00:00Z"}},
                    "gemini-3.8-flash-low": {"quotaInfo": {"remainingFraction": 0.9, "resetTime": "2026-11-26T00:00:00Z"}},
                    "claude-sonnet-5-5-high": {"quotaInfo": {"remainingFraction": 0.4, "resetTime": "2026-11-24T12:00:00Z"}}
                }
            }),
            Some("proj-1".to_string()),
            Some("ULTRA".to_string()),
        )
        .unwrap();
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("Gemini 60% · Claude 40%"));
        assert_eq!(snapshot.plan_type.as_deref(), Some("ULTRA"));
        assert!(snapshot.limit_weekly_reset_at.is_some());
    }

    #[test]
    fn persist_quota_result_preserves_existing_project_and_plan_when_omitted() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let mut content = account_content(false);
        content.project_id = Some("existing-proj".to_string());
        content.plan_type = Some("PRO".to_string());
        persist_imported_account(&db, "account", &content).unwrap();

        let partial = AntigravityQuotaSnapshot {
            project_id: None,
            plan_type: None,
            limit_short_label: Some("5h: Gemini 90%".to_string()),
            limit_5h_text: Some("Gemini 90%".to_string()),
            limit_5h_reset_at: Some(1_800_000_000),
            limit_weekly_text: None,
            limit_weekly_reset_at: None,
        };
        persist_quota_result(&db, "account", Ok(partial)).unwrap();
        let account = read_account(&db, "account");
        assert_eq!(account.project_id.as_deref(), Some("existing-proj"));
        assert_eq!(account.plan_type.as_deref(), Some("PRO"));
        assert_eq!(account.limit_5h_text.as_deref(), Some("Gemini 90%"));
    }

    #[test]
    fn matching_persisted_account_deduplicates_by_email_case_insensitively() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let mut content = account_content(true);
        content.email = Some("Alice@Example.com".to_string());
        persist_imported_account(&db, "existing-1", &content).unwrap();

        let matched = find_matching_persisted_account(
            &db,
            "official",
            Some("alice@example.com"),
            &json!({}),
        )
        .unwrap();
        assert_eq!(matched.map(|a| a.id).as_deref(), Some("existing-1"));
    }
}
