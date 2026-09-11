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
/// Upper bound on a single JSONL file the trace-id fallback
/// ([`get_request_log_detail`]) is willing to read into memory. Rows whose
/// persisted detail locator works never hit that fallback; this only guards
/// the residual case so an oversized log can no longer freeze the app.
const MAX_FALLBACK_SCAN_FILE_KB: u64 = 8 * 1024;

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
    ) || normalized_name == "token"
        || normalized_name.ends_with("-token")
        || normalized_name.ends_with("_token")
        || normalized_name.ends_with("-api-key")
}

pub fn redact_request_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let Some((base_path, query)) = trimmed.split_once('?') else {
        return trimmed.to_string();
    };
    if query.is_empty() {
        return trimmed.to_string();
    }

    let redacted_query = query
        .split('&')
        .map(|part| redact_query_part(part))
        .collect::<Vec<_>>()
        .join("&");
    format!("{base_path}?{redacted_query}")
}

fn redact_query_part(part: &str) -> String {
    let Some((key, value)) = part.split_once('=') else {
        return part.to_string();
    };
    let normalized_key = decode_query_key_for_matching(key)
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase();
    if is_sensitive_query_key(&normalized_key) {
        format!("{key}=xxx")
    } else {
        format!("{key}={value}")
    }
}

fn decode_query_key_for_matching(key: &str) -> String {
    let bytes = key.as_bytes();
    let mut decoded = String::with_capacity(key.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                match (
                    hex_digit_value(bytes[index + 1]),
                    hex_digit_value(bytes[index + 2]),
                ) {
                    (Some(high), Some(low)) => {
                        decoded.push((high << 4 | low) as char);
                        index += 3;
                    }
                    _ => {
                        decoded.push('%');
                        index += 1;
                    }
                }
            }
            byte => {
                decoded.push(byte as char);
                index += 1;
            }
        }
    }
    decoded
}

fn hex_digit_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_sensitive_query_key(normalized_key: &str) -> bool {
    let separator_normalized_key = normalize_query_key_separators(normalized_key);
    request_query_key_matches_sensitive_name(normalized_key)
        || (separator_normalized_key != normalized_key
            && request_query_key_matches_sensitive_name(&separator_normalized_key))
}

fn normalize_query_key_separators(normalized_key: &str) -> String {
    normalized_key
        .chars()
        .map(|character| match character {
            '-' | ' ' => '_',
            _ => character,
        })
        .collect()
}

fn request_query_key_matches_sensitive_name(normalized_key: &str) -> bool {
    request_query_key_is_token_like(normalized_key)
        || matches!(
            normalized_key,
            "key"
                | "api_key"
                | "apikey"
                | "x-api-key"
                | "x_api_key"
                | "access_token"
                | "refresh_token"
                | "client_secret"
                | "clientsecret"
                | "authorization"
                | "auth"
                | "password"
                | "secret"
        )
}

fn request_query_key_is_token_like(normalized_key: &str) -> bool {
    normalized_key == "token"
        || normalized_key.ends_with("_token")
        || normalized_key.ends_with("-token")
        || normalized_key.contains("access_token")
        || normalized_key.contains("refresh_token")
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
        // The trace-id fallback only runs when the persisted detail locator is
        // missing or stale. Loading a multi-hundred-MB JSONL into a single
        // `String` here froze the app (issue #324) even with the load offloaded
        // to a blocking task. Skip oversized files: rows whose locator works use
        // the fast `get_request_log_detail_at` path, and rows without a usable
        // locator on an oversized file fall through to the DB-only summary.
        let file_size = match fs::metadata(&file_path) {
            Ok(metadata) => metadata.len(),
            Err(error) => {
                log::warn!(
                    "Skipping unreadable proxy gateway request log {}: {error}",
                    file_path.display()
                );
                continue;
            }
        };
        if file_size > u64::from(MAX_FALLBACK_SCAN_FILE_KB) * 1024 {
            log::warn!(
                "Skipping proxy gateway request log {} ({} KB > {} KB fallback cap); \
                 use the detail locator instead of the trace-id scan",
                file_path.display(),
                file_size / 1024,
                MAX_FALLBACK_SCAN_FILE_KB,
            );
            continue;
        }
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
    fn is_sensitive_header_token_suffix_is_precise() {
        // Singular credential-bearing names still count as sensitive.
        assert!(is_sensitive_header("token"));
        assert!(is_sensitive_header("csrf-token"));
        assert!(is_sensitive_header("x-csrf-token"));
        assert!(is_sensitive_header("csrf_token"));
        assert!(is_sensitive_header("session-token"));
        assert!(is_sensitive_header("access_token"));
        assert!(is_sensitive_header("refresh_token"));
        assert!(is_sensitive_header("auth_token"));

        // Plural usage-metric fields and timing fields must NOT be redacted just
        // because they contain the substring "token" (issue #318: exports hid the
        // very token counts needed to diagnose consumption on failed requests).
        assert!(!is_sensitive_header("input_tokens"));
        assert!(!is_sensitive_header("output_tokens"));
        assert!(!is_sensitive_header("cache_read_tokens"));
        assert!(!is_sensitive_header("cache_creation_tokens"));
        assert!(!is_sensitive_header("total_tokens"));
        assert!(!is_sensitive_header("first_token_ms"));
    }

    #[test]
    fn redact_request_path_redacts_sensitive_query_values() {
        let redacted = redact_request_path(
            "/v1beta/models?key=secret&client_version=0.1&api%5Fkey=encoded&api-key=hyphen&client-secret=clientSecretValue&x%2Dapi%2Dkey=xhyphen&token=t",
        );

        assert_eq!(
            redacted,
            "/v1beta/models?key=xxx&client_version=0.1&api%5Fkey=xxx&api-key=xxx&client-secret=xxx&x%2Dapi%2Dkey=xxx&token=xxx"
        );
        assert!(!redacted.contains("key=secret"));
        assert!(!redacted.contains("encoded"));
        assert!(!redacted.contains("hyphen"));
        assert!(!redacted.contains("clientSecretValue"));
        assert!(!redacted.contains("xhyphen"));
    }

    #[test]
    fn redact_request_path_preserves_non_sensitive_query_values() {
        let redacted = redact_request_path("/search?monkey=value&client_version=0.1");

        assert_eq!(redacted, "/search?monkey=value&client_version=0.1");
    }

    #[test]
    fn request_logs_round_trip_summary_and_detail() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let now = Utc::now();
        let summary = GatewayRequestLogSummary {
            usage_metadata: None,
            data_source: None,
            trace_id: "trace-1".to_string(),
            started_at: now,
            ended_at: now,
            cli_key: Some(GatewayCliKey::Claude.into()),
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
            reasoning_effort: Some("high".to_string()),
            upstream_url: Some("https://api.example.com/v1/messages".to_string()),
            status_code: Some(200),
            upstream_status_code: None,
            success: true,
            error_category: None,
            error_message: None,
            stream_outcome: None,
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
            upstream_response_body: Some(r#"{"upstream":true}"#.to_string()),
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
        assert_eq!(summaries[0].reasoning_effort.as_deref(), Some("high"));

        let detail = get_request_log_detail(&paths, "trace-1").unwrap().unwrap();
        assert_eq!(
            detail.upstream_request_body.as_deref(),
            Some(r#"{"model":"upstream"}"#)
        );
        assert_eq!(
            detail.upstream_response_body.as_deref(),
            Some(r#"{"upstream":true}"#)
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
        assert_eq!(detail.summary.reasoning_effort.as_deref(), Some("high"));

        let mut legacy = serde_json::to_value(&record).unwrap();
        legacy.as_object_mut().unwrap().remove("reasoning_effort");
        let legacy: GatewayRequestLogRecord = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.detail.summary.reasoning_effort, None);
    }

    fn write_dated_log_file(paths: &ProxyGatewayPaths, date: &str, contents: &str) -> PathBuf {
        let root = paths.request_log_root();
        fs::create_dir_all(&root).unwrap();
        let path = root.join(format!("{date}.jsonl"));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn prune_by_retention_deletes_files_older_than_retention_days() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        // Files are named by date; prune parses the %Y-%m-%d stem.
        let old_date = "2020-01-01";
        let recent_date = chrono::Utc::now().date_naive();
        let recent = recent_date.format("%Y-%m-%d").to_string();
        write_dated_log_file(&paths, old_date, "old");
        write_dated_log_file(&paths, &recent, "recent");

        // retention_days=7 keeps anything within the last 7 days, drops 2020.
        prune_by_retention(&paths, 7).unwrap();

        assert!(!paths
            .request_log_root()
            .join(format!("{old_date}.jsonl"))
            .exists());
        assert!(paths
            .request_log_root()
            .join(format!("{recent}.jsonl"))
            .exists());
    }

    #[test]
    fn prune_by_retention_zero_days_keeps_everything() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let path = write_dated_log_file(&paths, "2020-01-01", "old");

        // retention_days == 0 is the documented early-return (retention disabled).
        prune_by_retention(&paths, 0).unwrap();

        assert!(path.exists(), "zero retention must not delete anything");
    }

    #[test]
    fn prune_by_size_drops_files_until_total_under_limit() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        // Three files whose combined size (~2.1 MiB) exceeds a 1 MiB cap.
        // prune_by_size must remove files until the surviving directory fits.
        let big = "x".repeat(700_000);
        let path_a = write_dated_log_file(&paths, "2020-01-01", &big);
        let path_b = write_dated_log_file(&paths, "2020-01-02", &big);
        let path_c = write_dated_log_file(&paths, "2020-01-03", &big);

        prune_by_size(&paths, 1).unwrap();

        let all = [path_a, path_b, path_c];
        let remaining = all.iter().filter(|path| path.exists()).count();
        assert!(remaining < 3, "size pruning must remove at least one file");
        let surviving_size: u64 = all
            .iter()
            .filter_map(|path| path.metadata().ok())
            .map(|metadata| metadata.len())
            .sum();
        assert!(
            surviving_size <= 1024 * 1024,
            "surviving dir size {surviving_size} must be under the 1 MiB cap"
        );
    }

    #[test]
    fn prune_by_size_zero_cap_keeps_everything() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ProxyGatewayPaths::new(dir.path());
        let path = write_dated_log_file(&paths, "2020-01-01", "data");

        prune_by_size(&paths, 0).unwrap();

        assert!(path.exists(), "zero size cap must not delete anything");
    }
}
