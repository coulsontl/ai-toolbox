use super::types::{
    GatewayCliKey, GatewaySessionImportCli, GatewaySessionUsageImportInput,
    GatewaySessionUsageImportResult,
};
use super::usage_parser::{from_response_body, TokenUsage};
use crate::db::SqliteDbState;
use chrono::Utc;
use rusqlite::params;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MAX_IMPORT_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SCAN_FILES_PER_CLI: usize = 20_000;

struct SessionUsageRecord {
    request_id: String,
    cli_key: GatewayCliKey,
    provider_id: String,
    model: String,
    request_model: Option<String>,
    usage: TokenUsage,
    status_code: u16,
    created_at: i64,
    session_id: Option<String>,
}

pub async fn import_session_usage(
    db: SqliteDbState,
    input: GatewaySessionUsageImportInput,
) -> Result<GatewaySessionUsageImportResult, String> {
    tauri::async_runtime::spawn_blocking(move || import_session_usage_blocking(&db, input))
        .await
        .map_err(|error| format!("Failed to import gateway session usage: {error}"))?
}

fn import_session_usage_blocking(
    db: &SqliteDbState,
    input: GatewaySessionUsageImportInput,
) -> Result<GatewaySessionUsageImportResult, String> {
    let cli_keys = match input.cli_key {
        GatewaySessionImportCli::All => vec![
            GatewayCliKey::Claude,
            GatewayCliKey::Codex,
            GatewayCliKey::Grok,
            GatewayCliKey::Gemini,
        ],
        GatewaySessionImportCli::Claude => vec![GatewayCliKey::Claude],
        GatewaySessionImportCli::Codex => vec![GatewayCliKey::Codex],
        GatewaySessionImportCli::Grok => vec![GatewayCliKey::Grok],
        GatewaySessionImportCli::Gemini => vec![GatewayCliKey::Gemini],
    };
    let mut total = GatewaySessionUsageImportResult::default();
    for cli_key in cli_keys {
        let result = import_cli_session_usage(db, cli_key)?;
        total.merge(result);
    }
    Ok(total)
}

fn import_cli_session_usage(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
) -> Result<GatewaySessionUsageImportResult, String> {
    let roots = default_session_roots(cli_key);
    let mut result = GatewaySessionUsageImportResult {
        scanned_files: 0,
        parsed_records: 0,
        inserted_records: 0,
        skipped_records: 0,
    };
    for root in roots {
        if !root.exists() {
            continue;
        }
        for file_path in session_files(&root) {
            if result.scanned_files as usize >= MAX_SCAN_FILES_PER_CLI {
                break;
            }
            result.scanned_files = result.scanned_files.saturating_add(1);
            let records = parse_session_file(cli_key, &file_path)?;
            result.parsed_records = result.parsed_records.saturating_add(records.len() as u64);
            for record in records {
                if insert_session_usage_record(db, &record)? {
                    result.inserted_records = result.inserted_records.saturating_add(1);
                } else {
                    result.skipped_records = result.skipped_records.saturating_add(1);
                }
            }
        }
    }
    Ok(result)
}

fn default_session_roots(cli_key: GatewayCliKey) -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    match cli_key {
        GatewayCliKey::Claude => vec![home.join(".claude").join("projects")],
        GatewayCliKey::Codex => vec![home.join(".codex").join("sessions")],
        GatewayCliKey::Grok => vec![home.join(".grok").join("sessions")],
        GatewayCliKey::Gemini => vec![home.join(".gemini").join("tmp")],
        GatewayCliKey::OpenCode => Vec::new(),
    }
}

fn session_files(root: &Path) -> Vec<PathBuf> {
    let mut files = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let path = entry.into_path();
            let extension = path.extension().and_then(|value| value.to_str())?;
            matches!(extension, "json" | "jsonl").then_some(path)
        })
        .filter(|path| {
            fs::metadata(path)
                .map(|metadata| metadata.len() <= MAX_IMPORT_FILE_BYTES)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn parse_session_file(
    cli_key: GatewayCliKey,
    file_path: &Path,
) -> Result<Vec<SessionUsageRecord>, String> {
    let content = fs::read_to_string(file_path).map_err(|error| {
        format!(
            "Failed to read {} session file {}: {}",
            cli_key.as_str(),
            file_path.display(),
            error
        )
    })?;
    if file_path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
        let mut records = Vec::new();
        for (index, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                if let Some(record) = session_usage_record(cli_key, file_path, index, &value) {
                    records.push(record);
                }
            }
        }
        return Ok(records);
    }
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return Ok(Vec::new());
    };
    Ok(parse_json_session_values(cli_key, file_path, &value))
}

fn parse_json_session_values(
    cli_key: GatewayCliKey,
    file_path: &Path,
    value: &Value,
) -> Vec<SessionUsageRecord> {
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| session_usage_record(cli_key, file_path, index, item))
            .collect();
    }
    for path in ["/messages", "/turns", "/entries", "/records"] {
        if let Some(items) = value.pointer(path).and_then(Value::as_array) {
            return items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| session_usage_record(cli_key, file_path, index, item))
                .collect();
        }
    }
    session_usage_record(cli_key, file_path, 0, value)
        .into_iter()
        .collect()
}

fn session_usage_record(
    cli_key: GatewayCliKey,
    file_path: &Path,
    index: usize,
    value: &Value,
) -> Option<SessionUsageRecord> {
    let usage_value = usage_candidate(value)?;
    let usage = from_response_body(cli_key, &serde_json::to_vec(usage_value).ok()?);
    usage.total_tokens()?;
    let message_id = first_string_at_paths(
        value,
        &[
            "/message/id",
            "/message_id",
            "/messageId",
            "/id",
            "/uuid",
            "/request_id",
        ],
    );
    let request_id = if cli_key == GatewayCliKey::Claude {
        message_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!("SESSION:{value}"))
            .unwrap_or_else(|| stable_session_id(cli_key, file_path, index))
    } else {
        stable_session_id(cli_key, file_path, index)
    };
    Some(SessionUsageRecord {
        request_id,
        cli_key,
        provider_id: "session".to_string(),
        model: model_from_value(value).unwrap_or_else(|| "unknown".to_string()),
        request_model: model_from_value(value),
        usage,
        status_code: 200,
        created_at: timestamp_from_value(value).unwrap_or_else(|| Utc::now().timestamp()),
        session_id: session_id_from_path(file_path).or(message_id),
    })
}

fn usage_candidate(value: &Value) -> Option<&Value> {
    if value.get("usage").is_some()
        || value.get("usageMetadata").is_some()
        || value.pointer("/message/usage").is_some()
        || value.pointer("/response/usage").is_some()
    {
        return Some(value);
    }
    for path in ["/response", "/message", "/payload", "/data"] {
        if let Some(candidate) = value.pointer(path) {
            if candidate.get("usage").is_some() || candidate.get("usageMetadata").is_some() {
                return Some(candidate);
            }
        }
    }
    None
}

fn insert_session_usage_record(
    db: &SqliteDbState,
    record: &SessionUsageRecord,
) -> Result<bool, String> {
    let input_tokens = record.usage.input_tokens.unwrap_or(0) as i64;
    let output_tokens = record.usage.output_tokens.unwrap_or(0) as i64;
    let cache_read_tokens = record.usage.cache_read_tokens.unwrap_or(0) as i64;
    let cache_creation_tokens = record.usage.cache_creation_tokens.unwrap_or(0) as i64;
    db.with_conn(|conn| {
        let changed = conn
            .execute(
                "INSERT OR IGNORE INTO proxy_request_logs (
                    request_id, provider_id, app_type, model, request_model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                    total_cost_usd, latency_ms, first_token_ms, duration_ms,
                    status_code, error_message, session_id, provider_type, is_streaming,
                    cost_multiplier, created_at, data_source
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5,
                    ?6, ?7, ?8, ?9,
                    '0', '0', '0', '0',
                    '0', 0, NULL, 0,
                    ?10, NULL, ?11, 'session', 0,
                    '1.0', ?12, 'session'
                )",
                params![
                    record.request_id,
                    record.provider_id,
                    record.cli_key.as_str(),
                    record.model,
                    record.request_model,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                    i64::from(record.status_code),
                    record.session_id,
                    record.created_at,
                ],
            )
            .map_err(|error| format!("Failed to insert gateway session usage: {error}"))?;
        Ok(changed > 0)
    })
}

fn stable_session_id(cli_key: GatewayCliKey, file_path: &Path, index: usize) -> String {
    let mut hasher = DefaultHasher::new();
    cli_key.as_str().hash(&mut hasher);
    file_path.to_string_lossy().hash(&mut hasher);
    index.hash(&mut hasher);
    format!("SESSION:{:016x}", hasher.finish())
}

fn session_id_from_path(file_path: &Path) -> Option<String> {
    file_path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn model_from_value(value: &Value) -> Option<String> {
    first_string_at_paths(
        value,
        &[
            "/model",
            "/request/model",
            "/response/model",
            "/message/model",
            "/metadata/model",
        ],
    )
}

fn timestamp_from_value(value: &Value) -> Option<i64> {
    for path in [
        "/created_at",
        "/createdAt",
        "/timestamp",
        "/time",
        "/message/created_at",
    ] {
        let Some(value) = value.pointer(path) else {
            continue;
        };
        if let Some(timestamp) = value.as_i64() {
            return Some(if timestamp > 10_000_000_000 {
                timestamp / 1000
            } else {
                timestamp
            });
        }
        if let Some(text) = value.as_str() {
            if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(text) {
                return Some(timestamp.timestamp());
            }
        }
    }
    None
}

fn first_string_at_paths(value: &Value, paths: &[&str]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| value.pointer(path).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn claude_import_uses_message_id_for_request_id() {
        let root = tempfile::tempdir().unwrap();
        let file_path = root.path().join("session.jsonl");
        let mut file = fs::File::create(&file_path).unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "message": {
                    "id": "msg_123",
                    "model": "claude-sonnet-4-5",
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 20,
                        "cache_read_input_tokens": 3
                    }
                }
            })
        )
        .unwrap();

        let records = parse_session_file(GatewayCliKey::Claude, &file_path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].request_id, "SESSION:msg_123");
        assert_eq!(records[0].usage.cache_read_tokens, Some(3));
    }
}
