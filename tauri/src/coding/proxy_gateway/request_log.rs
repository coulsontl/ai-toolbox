use super::paths::ProxyGatewayPaths;
use super::types::{
    GatewayRequestLogDetail, GatewayRequestLogRecord, GatewayRequestLogSummary,
    ProxyGatewayRequestLogListInput, ProxyGatewaySettings,
};
use chrono::{Duration, NaiveDate, Utc};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::PathBuf;

const REDACTED: &str = "[REDACTED]";
const REQUEST_LOG_SCHEMA_VERSION: u32 = 1;
const DEFAULT_LOG_LIST_LIMIT: usize = 100;
const MAX_LOG_LIST_LIMIT: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestLogLocation {
    pub detail_file: String,
    pub detail_offset: u64,
}

pub fn redact_headers(headers: &[(String, String)]) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            let normalized_name = name.to_ascii_lowercase();
            let redacted_value = if is_sensitive_header(&normalized_name) {
                REDACTED.to_string()
            } else {
                value.clone()
            };
            (name.clone(), redacted_value)
        })
        .collect()
}

pub fn is_sensitive_header(normalized_name: &str) -> bool {
    matches!(
        normalized_name,
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "x-api-key"
            | "api-key"
            | "anthropic-api-key"
            | "openai-api-key"
    ) || normalized_name.contains("token")
        || normalized_name.ends_with("-api-key")
}

pub fn new_request_log_record(detail: GatewayRequestLogDetail) -> GatewayRequestLogRecord {
    GatewayRequestLogRecord {
        schema_version: REQUEST_LOG_SCHEMA_VERSION,
        detail,
    }
}

pub fn write_request_log(
    paths: &ProxyGatewayPaths,
    settings: &ProxyGatewaySettings,
    record: &GatewayRequestLogRecord,
) -> Result<Option<RequestLogLocation>, String> {
    if !settings.request_log_enabled {
        return Ok(None);
    }

    let root = paths.request_log_root();
    fs::create_dir_all(&root).map_err(|error| {
        format!(
            "Failed to create proxy gateway request log directory {}: {}",
            root.display(),
            error
        )
    })?;
    prune_request_logs(paths, settings);

    let file_path = request_log_file_path(paths, record.detail.summary.ended_at.date_naive());
    let json_line = serde_json::to_string(record)
        .map_err(|error| format!("Failed to serialize proxy gateway request log: {error}"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(&file_path)
        .map_err(|error| {
            format!(
                "Failed to open proxy gateway request log {}: {}",
                file_path.display(),
                error
            )
        })?;
    let offset = file
        .metadata()
        .map(|metadata| metadata.len())
        .map_err(|error| {
            format!(
                "Failed to read proxy gateway request log metadata {}: {}",
                file_path.display(),
                error
            )
        })?;
    writeln!(file, "{json_line}").map_err(|error| {
        format!(
            "Failed to append proxy gateway request log {}: {}",
            file_path.display(),
            error
        )
    })?;
    let detail_file = file_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_string();
    Ok(Some(RequestLogLocation {
        detail_file,
        detail_offset: offset,
    }))
}

pub fn list_request_logs(
    paths: &ProxyGatewayPaths,
    input: ProxyGatewayRequestLogListInput,
) -> Result<Vec<GatewayRequestLogSummary>, String> {
    let limit = input
        .limit
        .unwrap_or(DEFAULT_LOG_LIST_LIMIT)
        .clamp(1, MAX_LOG_LIST_LIMIT);
    let mut summaries = Vec::new();

    for file_path in request_log_files_newest_first(paths)? {
        let content = fs::read_to_string(&file_path).map_err(|error| {
            format!(
                "Failed to read proxy gateway request log {}: {}",
                file_path.display(),
                error
            )
        })?;
        for line in content.lines().rev() {
            if summaries.len() >= limit {
                return Ok(summaries);
            }
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<GatewayRequestLogRecord>(line) {
                Ok(record) => summaries.push(record.detail.summary),
                Err(error) => {
                    log::warn!(
                        "Skipping malformed proxy gateway request log line in {}: {}",
                        file_path.display(),
                        error
                    );
                }
            }
        }
    }

    Ok(summaries)
}

pub fn get_request_log_detail(
    paths: &ProxyGatewayPaths,
    trace_id: &str,
) -> Result<Option<GatewayRequestLogDetail>, String> {
    let trace_id = trace_id.trim();
    if trace_id.is_empty() {
        return Ok(None);
    }

    for file_path in request_log_files_newest_first(paths)? {
        let content = fs::read_to_string(&file_path).map_err(|error| {
            format!(
                "Failed to read proxy gateway request log {}: {}",
                file_path.display(),
                error
            )
        })?;
        for line in content.lines().rev() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<GatewayRequestLogRecord>(line) {
                Ok(record) if record.detail.summary.trace_id == trace_id => {
                    return Ok(Some(record.detail));
                }
                Ok(_) => {}
                Err(error) => {
                    log::warn!(
                        "Skipping malformed proxy gateway request log line in {}: {}",
                        file_path.display(),
                        error
                    );
                }
            }
        }
    }

    Ok(None)
}

pub fn get_request_log_detail_at(
    paths: &ProxyGatewayPaths,
    detail_file: &str,
    detail_offset: u64,
    trace_id: &str,
) -> Result<Option<GatewayRequestLogDetail>, String> {
    let trace_id = trace_id.trim();
    if trace_id.is_empty() || detail_file.trim().is_empty() {
        return Ok(None);
    }
    if detail_file.contains('/') || detail_file.contains('\\') || !detail_file.ends_with(".jsonl") {
        return Ok(None);
    }
    let file_path = paths.request_log_root().join(detail_file);
    let mut file = OpenOptions::new()
        .read(true)
        .open(&file_path)
        .map_err(|error| {
            format!(
                "Failed to open proxy gateway request log {}: {}",
                file_path.display(),
                error
            )
        })?;
    file.seek(SeekFrom::Start(detail_offset)).map_err(|error| {
        format!(
            "Failed to seek proxy gateway request log {} at {}: {}",
            file_path.display(),
            detail_offset,
            error
        )
    })?;
    let mut line = String::new();
    let mut reader = BufReader::new(file);
    reader.read_line(&mut line).map_err(|error| {
        format!(
            "Failed to read proxy gateway request log {} at {}: {}",
            file_path.display(),
            detail_offset,
            error
        )
    })?;
    if line.trim().is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<GatewayRequestLogRecord>(line.trim_end()) {
        Ok(record) if record.detail.summary.trace_id == trace_id => Ok(Some(record.detail)),
        Ok(_) => Ok(None),
        Err(error) => {
            log::warn!(
                "Skipping malformed proxy gateway request log line in {} at {}: {}",
                file_path.display(),
                detail_offset,
                error
            );
            Ok(None)
        }
    }
}

fn request_log_file_path(paths: &ProxyGatewayPaths, date: NaiveDate) -> PathBuf {
    paths
        .request_log_root()
        .join(format!("{}.jsonl", date.format("%Y-%m-%d")))
}

fn request_log_files_newest_first(paths: &ProxyGatewayPaths) -> Result<Vec<PathBuf>, String> {
    let root = paths.request_log_root();
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let entries = fs::read_dir(&root).map_err(|error| {
        format!(
            "Failed to read proxy gateway request log directory {}: {}",
            root.display(),
            error
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "Failed to read proxy gateway request log directory entry {}: {}",
                root.display(),
                error
            )
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    files.sort_by(|left, right| {
        let left_name = left.file_name().and_then(|value| value.to_str());
        let right_name = right.file_name().and_then(|value| value.to_str());
        right_name.cmp(&left_name)
    });
    Ok(files)
}

fn prune_request_logs(paths: &ProxyGatewayPaths, settings: &ProxyGatewaySettings) {
    if let Err(error) = prune_by_retention(paths, settings.log_retention_days) {
        log::warn!("Failed to prune proxy gateway request logs by retention: {error}");
    }
    if let Err(error) = prune_by_size(paths, settings.log_max_dir_size_mb) {
        log::warn!("Failed to prune proxy gateway request logs by size: {error}");
    }
}

fn prune_by_retention(paths: &ProxyGatewayPaths, retention_days: u32) -> Result<(), String> {
    if retention_days == 0 {
        return Ok(());
    }
    let cutoff = Utc::now().date_naive() - Duration::days(i64::from(retention_days));
    for file_path in request_log_files_newest_first(paths)? {
        let Some(stem) = file_path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let Ok(file_date) = NaiveDate::parse_from_str(stem, "%Y-%m-%d") else {
            continue;
        };
        if file_date < cutoff {
            let _ = fs::remove_file(&file_path);
        }
    }
    Ok(())
}

fn prune_by_size(paths: &ProxyGatewayPaths, max_dir_size_mb: u64) -> Result<(), String> {
    if max_dir_size_mb == 0 {
        return Ok(());
    }
    let root = paths.request_log_root();
    if !root.exists() {
        return Ok(());
    }
    let max_bytes = max_dir_size_mb.saturating_mul(1024).saturating_mul(1024);
    let mut files = Vec::new();
    let mut total_size = 0_u64;
    for file_path in request_log_files_newest_first(paths)? {
        let metadata = match fs::metadata(&file_path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let modified = metadata.modified().ok();
        let size = metadata.len();
        total_size = total_size.saturating_add(size);
        files.push((file_path, modified, size));
    }
    files.sort_by_key(|(_, modified, _)| *modified);
    for (file_path, _, size) in files {
        if total_size <= max_bytes {
            break;
        }
        if fs::remove_file(&file_path).is_ok() {
            total_size = total_size.saturating_sub(size);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::proxy_gateway::types::{GatewayCliKey, ProxyGatewaySettings};
    use chrono::Utc;

    #[test]
    fn redact_headers_redacts_authorization_and_cookie() {
        let redacted = redact_headers(&[
            ("Authorization".to_string(), "Bearer secret".to_string()),
            ("Cookie".to_string(), "session=secret".to_string()),
        ]);

        assert_eq!(redacted.get("Authorization").unwrap(), REDACTED);
        assert_eq!(redacted.get("Cookie").unwrap(), REDACTED);
    }

    #[test]
    fn redact_headers_redacts_provider_api_keys_case_insensitively() {
        let redacted = redact_headers(&[
            ("Anthropic-Api-Key".to_string(), "secret".to_string()),
            ("X-Api-Key".to_string(), "secret".to_string()),
            ("Custom-Token".to_string(), "secret".to_string()),
        ]);

        assert_eq!(redacted.get("Anthropic-Api-Key").unwrap(), REDACTED);
        assert_eq!(redacted.get("X-Api-Key").unwrap(), REDACTED);
        assert_eq!(redacted.get("Custom-Token").unwrap(), REDACTED);
    }

    #[test]
    fn redact_headers_preserves_non_sensitive_headers() {
        let redacted = redact_headers(&[
            ("Content-Type".to_string(), "application/json".to_string()),
            ("User-Agent".to_string(), "ai-toolbox".to_string()),
        ]);

        assert_eq!(redacted.get("Content-Type").unwrap(), "application/json");
        assert_eq!(redacted.get("User-Agent").unwrap(), "ai-toolbox");
    }

    #[test]
    fn request_logs_round_trip_summary_and_detail() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let now = Utc::now();
        let summary = GatewayRequestLogSummary {
            trace_id: "trace-1".to_string(),
            started_at: now,
            ended_at: now,
            cli_key: Some(GatewayCliKey::Claude),
            route_name: "anthropic".to_string(),
            method: "POST".to_string(),
            path: "/anthropic/v1/messages".to_string(),
            provider_id: Some("provider-a".to_string()),
            provider_name: Some("Provider A".to_string()),
            provider_type: None,
            cost_multiplier: None,
            pricing_model_source: None,
            requested_model: Some("claude".to_string()),
            upstream_model_id: Some("claude".to_string()),
            upstream_url: Some("https://api.example.com/v1/messages".to_string()),
            status_code: Some(200),
            success: true,
            error_category: None,
            error_message: None,
            duration_ms: 42,
            attempt_count: 1,
            total_attempt_count: 1,
            failover: false,
            input_tokens: Some(10),
            output_tokens: Some(20),
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
            total_tokens: Some(30),
            request_body_bytes: 2,
            response_body_bytes: 11,
            is_streaming: false,
            first_token_ms: None,
            detail_file: None,
            detail_offset: None,
        };
        let record = new_request_log_record(GatewayRequestLogDetail {
            summary,
            request_headers: Some(BTreeMap::from([(
                "Content-Type".to_string(),
                "application/json".to_string(),
            )])),
            request_body: Some("{}".to_string()),
            upstream_request_body: Some(r#"{"model":"upstream"}"#.to_string()),
            response_headers: None,
            response_body: Some(r#"{"ok":true}"#.to_string()),
            provider_attempts: Vec::new(),
        });

        let location = write_request_log(&paths, &ProxyGatewaySettings::default(), &record)
            .unwrap()
            .unwrap();

        let summaries =
            list_request_logs(&paths, ProxyGatewayRequestLogListInput { limit: Some(10) }).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].trace_id, "trace-1");

        let detail = get_request_log_detail(&paths, "trace-1").unwrap().unwrap();
        assert_eq!(
            detail.upstream_request_body.as_deref(),
            Some(r#"{"model":"upstream"}"#)
        );
        assert_eq!(detail.response_body.as_deref(), Some(r#"{"ok":true}"#));

        let detail = get_request_log_detail_at(
            &paths,
            &location.detail_file,
            location.detail_offset,
            "trace-1",
        )
        .unwrap()
        .unwrap();
        assert_eq!(detail.summary.trace_id, "trace-1");
    }
}
