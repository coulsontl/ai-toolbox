use super::request_log;
use super::types::{
    normalize_pricing_model_source, GatewayModelStats, GatewayPaginatedRequestLogs,
    GatewayProviderStats, GatewayRequestLogDetail, GatewayRequestLogFilters, GatewayRequestLogItem,
    GatewayRequestLogSummary, GatewayStreamOutcome, GatewayUsageSummary, GatewayUsageSummaryByCli,
    GatewayUsageTool, GatewayUsageTrendPoint, ProxyGatewaySettings,
};
use crate::db::SqliteDbState;
use chrono::{Duration, Local, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, ToSql};
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration as StdDuration, Instant};

type ProviderNameMap = HashMap<(String, String), String>;

const MAX_PAGE_SIZE: u32 = 100;
const ROLLUP_THROTTLE_SECONDS: u64 = 300;
const ONE_M_CONTEXT_MARKER: &str = "[1m]";
const COMPACT_ROLLUP_MODEL: &str = "__context_compact__";

static LAST_ROLLUP_PRUNE_AT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

#[derive(Default)]
struct TrendAccumulator {
    extra_tokens: u64,
    request_count: u64,
    total_cost_usd: Decimal,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
}

impl TrendAccumulator {
    fn add(
        &mut self,
        request_count: u64,
        total_cost_usd: Decimal,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_creation_tokens: u64,
        extra_tokens: u64,
    ) {
        self.extra_tokens = self.extra_tokens.saturating_add(extra_tokens);
        self.request_count = self.request_count.saturating_add(request_count);
        self.total_cost_usd += total_cost_usd;
        self.input_tokens = self.input_tokens.saturating_add(input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(output_tokens);
        self.cache_read_tokens = self.cache_read_tokens.saturating_add(cache_read_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(cache_creation_tokens);
    }

    fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_creation_tokens)
            .saturating_add(self.extra_tokens)
    }
}

#[derive(Default)]
struct StatsAccumulator {
    request_count: u64,
    success_count: u64,
    total_tokens: u64,
    total_cost_usd: Decimal,
    latency_weighted_sum: f64,
    latency_sample_count: u64,
    input_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
}

#[derive(Default)]
struct SummaryAccumulator {
    extra_tokens: u64,
    total_requests: u64,
    success_count: u64,
    total_cost_usd: Decimal,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
}

impl SummaryAccumulator {
    fn add(&mut self, other: SummaryAccumulator) {
        self.extra_tokens = self.extra_tokens.saturating_add(other.extra_tokens);
        self.total_requests = self.total_requests.saturating_add(other.total_requests);
        self.success_count = self.success_count.saturating_add(other.success_count);
        self.total_cost_usd += other.total_cost_usd;
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(other.cache_creation_tokens);
    }

    fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_creation_tokens)
            .saturating_add(self.extra_tokens)
    }

    fn into_summary(self) -> GatewayUsageSummary {
        let success_rate = percent(self.success_count, self.total_requests);
        GatewayUsageSummary {
            total_requests: self.total_requests,
            total_cost_usd: format_decimal_cost(self.total_cost_usd),
            total_input_tokens: self.input_tokens,
            total_output_tokens: self.output_tokens,
            total_cache_read_tokens: self.cache_read_tokens,
            total_cache_creation_tokens: self.cache_creation_tokens,
            success_rate,
            total_tokens: self.total_tokens(),
        }
    }
}

#[derive(Debug, Clone)]
struct ModelPricing {
    input_cost_per_million: Decimal,
    output_cost_per_million: Decimal,
    cache_read_cost_per_million: Decimal,
    cache_creation_cost_per_million: Decimal,
}

#[derive(Debug, Clone, Default)]
pub(super) struct CostBreakdown {
    pub(super) input_cost_usd: Decimal,
    pub(super) output_cost_usd: Decimal,
    pub(super) cache_read_cost_usd: Decimal,
    pub(super) cache_creation_cost_usd: Decimal,
}

impl CostBreakdown {
    pub(super) fn total(&self) -> Decimal {
        self.input_cost_usd
            + self.output_cost_usd
            + self.cache_read_cost_usd
            + self.cache_creation_cost_usd
    }

    fn apply_multiplier(mut self, multiplier: Decimal) -> Self {
        self.input_cost_usd *= multiplier;
        self.output_cost_usd *= multiplier;
        self.cache_read_cost_usd *= multiplier;
        self.cache_creation_cost_usd *= multiplier;
        self
    }
}

impl StatsAccumulator {
    fn add_input_usage(
        &mut self,
        input_tokens: u64,
        cache_read_tokens: u64,
        cache_creation_tokens: u64,
    ) {
        self.input_tokens = self.input_tokens.saturating_add(input_tokens);
        self.cache_read_tokens = self.cache_read_tokens.saturating_add(cache_read_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(cache_creation_tokens);
    }

    fn cache_hit_rate(&self) -> Option<f64> {
        // Stored input_tokens are already fresh, cache-exclusive input.
        let total_input = self
            .input_tokens
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_creation_tokens);
        (total_input > 0).then(|| self.cache_read_tokens as f64 / total_input as f64)
    }

    fn add(
        &mut self,
        request_count: u64,
        success_count: u64,
        total_tokens: u64,
        total_cost_usd: Decimal,
        latency_weighted_sum: f64,
        latency_sample_count: u64,
    ) {
        self.request_count = self.request_count.saturating_add(request_count);
        self.success_count = self.success_count.saturating_add(success_count);
        self.total_tokens = self.total_tokens.saturating_add(total_tokens);
        self.total_cost_usd += total_cost_usd;
        self.latency_weighted_sum += latency_weighted_sum;
        self.latency_sample_count = self
            .latency_sample_count
            .saturating_add(latency_sample_count);
    }

    fn avg_latency_ms(&self) -> Option<u64> {
        (self.latency_sample_count > 0).then(|| {
            (self.latency_weighted_sum / self.latency_sample_count as f64)
                .max(0.0)
                .round() as u64
        })
    }
}

/// Result of attempting to persist a compact proxy usage summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordRequestSummaryOutcome {
    /// Identical proxy semantic already exists; do not write JSONL or recount.
    Skipped,
    /// Row inserted/upgraded. `request_id` is the final SQLite primary key and must
    /// also be used as the JSONL `trace_id` so list/detail lookup stays consistent
    /// even when a collision fallback key is chosen.
    Written { request_id: String, collision: bool },
    /// Summary already exists with identical semantics but still lacks a JSONL
    /// detail locator. Observability may retry detail attachment without recounting.
    NeedsDetail { request_id: String },
}

/// Record a compact proxy request summary for request list / usage stats.
pub fn record_request_summary(
    db: &SqliteDbState,
    settings: &ProxyGatewaySettings,
    detail: &GatewayRequestLogDetail,
) -> Result<RecordRequestSummaryOutcome, String> {
    let summary = &detail.summary;
    let Some(cli_key) = summary.cli_key.and_then(GatewayUsageTool::gateway_cli) else {
        return Ok(RecordRequestSummaryOutcome::Skipped);
    };

    db.with_conn(|conn| {
        if let Err(error) = maybe_rollup_and_prune(conn, i64::from(settings.log_retention_days)) {
            // Retention is maintenance, not a prerequisite for recording usage.
            log::warn!("Gateway history maintenance failed; recording current usage: {error}");
        }
        let provider_id = summary
            .provider_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("unknown");
        let upstream_model = summary
            .upstream_model_id
            .as_deref()
            .or(summary.requested_model.as_deref())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("unknown");
        let created_at = summary.ended_at.timestamp();
        let status_code = i64::from(summary.status_code.unwrap_or(0));
        let input_tokens = summary.input_tokens.unwrap_or(0) as i64;
        let output_tokens = summary.output_tokens.unwrap_or(0) as i64;
        let cache_read_tokens = summary.cache_read_tokens.unwrap_or(0) as i64;
        let cache_creation_tokens = summary.cache_creation_tokens.unwrap_or(0) as i64;
        let first_token_ms = summary.first_token_ms.map(|value| value as i64);
        let latency_ms = first_token_ms.unwrap_or(summary.duration_ms as i64);
        let pricing = find_summary_model_pricing(conn, summary, upstream_model);
        let cost_multiplier = parse_decimal_or_default(
            summary.cost_multiplier.as_deref().unwrap_or("1.0"),
            Decimal::new(1, 0),
        );
        let pricing_model_source = normalize_pricing_model_source(
            summary
                .pricing_model_source
                .as_deref()
                .unwrap_or("upstream"),
        );
        let route_name = optional_compact_string(&summary.route_name);
        let method = optional_compact_string(&summary.method);
        let path = optional_compact_string(&request_log::redact_request_path(&summary.path));
        let costs = pricing
            .as_ref()
            .map(|pricing| {
                calculate_cost(
                    input_tokens as u64,
                    output_tokens as u64,
                    cache_read_tokens as u64,
                    cache_creation_tokens as u64,
                    pricing,
                )
                .apply_multiplier(cost_multiplier)
            })
            .unwrap_or_default();

        let semantic = UsageSemantic {
            app_type: cli_key.as_str().to_string(),
            provider_id: provider_id.to_string(),
            model: upstream_model.to_string(),
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            status_code,
            stream_outcome: summary
                .stream_outcome
                .filter(|outcome| *outcome != GatewayStreamOutcome::NotStreaming)
                .map(|outcome| outcome.as_str().to_string()),
            error_category: summary.error_category.clone(),
        };
        let plan = resolve_usage_request_id(conn, &summary.trace_id, &semantic)?;
        let (request_id, replace_session_log, collision, preserve_existing_detail) = match plan {
            UsageWritePlan::Skip => return Ok(RecordRequestSummaryOutcome::Skipped),
            UsageWritePlan::NeedsDetail { request_id } => {
                // Summary already exists with identical semantics; only detail is missing.
                return Ok(RecordRequestSummaryOutcome::NeedsDetail { request_id });
            }
            UsageWritePlan::Write {
                request_id,
                replace_session_log,
                collision,
                preserve_existing_detail,
            } => (
                request_id,
                replace_session_log,
                collision,
                preserve_existing_detail,
            ),
        };

        let existing_detail = if preserve_existing_detail {
            load_existing_detail_locator(conn, &request_id)?
        } else {
            None
        };
        let (detail_file, detail_offset) = if preserve_existing_detail {
            // Keep a previously linked JSONL locator when upgrading a session row
            // that already points at detail, unless this write supplies a new one.
            (
                summary
                    .detail_file
                    .clone()
                    .or(existing_detail.as_ref().and_then(|(file, _)| file.clone())),
                summary
                    .detail_offset
                    .or(existing_detail.and_then(|(_, offset)| offset)),
            )
        } else {
            (summary.detail_file.clone(), summary.detail_offset)
        };
        // Preserve imported session linkage when proxy upgrades a session row.
        // INSERT OR REPLACE would otherwise wipe session_id if we hardcode NULL.
        let session_id = if replace_session_log {
            load_existing_session_id(conn, &request_id)?
        } else {
            None
        };

        let insert_verb = if replace_session_log {
            // Proxy may upgrade a prior session-import row with richer cost/detail fields.
            "INSERT OR REPLACE"
        } else {
            // Process-local gw-* keys and first-time envelope keys insert only once.
            // Identical proxy semantic replays are skipped above; different semantics
            // land on a collision fallback key instead of overwriting.
            "INSERT OR IGNORE"
        };
        let stream_outcome_value = summary
            .stream_outcome
            .filter(|outcome| *outcome != GatewayStreamOutcome::NotStreaming)
            .map(|outcome| outcome.as_str());
        let sql = format!(
            "{insert_verb} INTO proxy_request_logs (
                request_id, provider_id, app_type, model, request_model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                total_cost_usd, latency_ms, first_token_ms, duration_ms,
                status_code, error_message, session_id, provider_type, is_streaming,
                cost_multiplier, pricing_model_source, created_at, data_source, detail_file,
                detail_offset, route_name, method, path, upstream_status_code,
                stream_outcome, error_category, attempt_count, total_attempt_count, reasoning_effort
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17,
                ?18, ?19, ?20, ?21, ?22,
                ?23, ?24, ?25, 'proxy', ?26, ?27, ?28, ?29, ?30, ?31,
                ?32, ?33, ?34, ?35, ?36
            )"
        );
        let affected_rows = conn
            .execute(
                &sql,
                rusqlite::params![
                    request_id,
                    provider_id,
                    cli_key.as_str(),
                    upstream_model,
                    summary.requested_model,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                    format_decimal_cost(costs.input_cost_usd),
                    format_decimal_cost(costs.output_cost_usd),
                    format_decimal_cost(costs.cache_read_cost_usd),
                    format_decimal_cost(costs.cache_creation_cost_usd),
                    format_decimal_cost(costs.total()),
                    latency_ms,
                    first_token_ms,
                    summary.duration_ms as i64,
                    status_code,
                    summary.error_message,
                    session_id,
                    summary.provider_type,
                    i64::from(summary.is_streaming),
                    cost_multiplier.to_string(),
                    pricing_model_source,
                    created_at,
                    detail_file,
                    detail_offset.map(|value| value as i64),
                    route_name,
                    method,
                    path,
                    summary.upstream_status_code.map(|value| i64::from(value)),
                    stream_outcome_value,
                    summary.error_category,
                    i64::from(summary.attempt_count.max(1)),
                    i64::from(summary.total_attempt_count.max(1)),
                    summary.reasoning_effort,
                ],
            )
            .map_err(|error| format!("Failed to record proxy gateway request summary: {error}"))?;
        if affected_rows == 0 {
            // INSERT OR IGNORE lost a race with an identical primary key write.
            return Ok(RecordRequestSummaryOutcome::Skipped);
        }
        if collision {
            log::warn!(
                "usage request_id collision: primary={}, fallback={request_id}",
                summary.trace_id
            );
        }
        Ok(RecordRequestSummaryOutcome::Written {
            request_id,
            collision,
        })
    })
}

/// Attach JSONL detail locator fields to an already-written summary row.
pub fn update_request_detail_locator(
    db: &SqliteDbState,
    request_id: &str,
    detail_file: Option<&str>,
    detail_offset: Option<u64>,
) -> Result<(), String> {
    let request_id = request_id.trim();
    if request_id.is_empty() {
        return Ok(());
    }
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE proxy_request_logs
             SET detail_file = COALESCE(?2, detail_file),
                 detail_offset = COALESCE(?3, detail_offset)
             WHERE request_id = ?1",
            rusqlite::params![
                request_id,
                detail_file,
                detail_offset.map(|value| value as i64),
            ],
        )
        .map_err(|error| format!("Failed to update usage detail locator: {error}"))?;
        Ok(())
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UsageSemantic {
    app_type: String,
    provider_id: String,
    model: String,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
    status_code: i64,
    /// `GatewayStreamOutcome::as_str()` for streaming rows; `None` for
    /// non-streaming / `NotStreaming` / session-imported rows. Included so two
    /// rows with the same envelope id and tokens but different stream outcomes
    /// (one `completed`, one `failed`) are not treated as a same-semantic replay
    /// and deduplicated away.
    stream_outcome: Option<String>,
    /// Persisted error category; included for the same reason as
    /// `stream_outcome`.
    error_category: Option<String>,
}

impl UsageSemantic {
    fn sha256_hex(&self) -> String {
        let encoded = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.app_type,
            self.provider_id,
            self.model,
            self.input_tokens,
            self.output_tokens,
            self.cache_read_tokens,
            self.cache_creation_tokens,
            self.status_code,
            self.stream_outcome.as_deref().unwrap_or(""),
            self.error_category.as_deref().unwrap_or("")
        );
        Sha256::digest(encoded.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

enum UsageWritePlan {
    Skip,
    /// Proxy semantic already recorded, but the row still has no detail locator.
    NeedsDetail {
        request_id: String,
    },
    Write {
        request_id: String,
        replace_session_log: bool,
        collision: bool,
        /// When replacing a session row, keep any existing detail locator unless
        /// the new write supplies one.
        preserve_existing_detail: bool,
    },
}

fn resolve_usage_request_id(
    conn: &Connection,
    primary_request_id: &str,
    semantic: &UsageSemantic,
) -> Result<UsageWritePlan, String> {
    let existing = load_existing_usage_semantic(conn, primary_request_id)?;
    match existing {
        None => Ok(UsageWritePlan::Write {
            request_id: primary_request_id.to_string(),
            replace_session_log: false,
            collision: false,
            preserve_existing_detail: false,
        }),
        Some((data_source, _)) if data_source.as_deref() == Some("session") => {
            // Proxy may upgrade a session-import placeholder with richer fields.
            Ok(UsageWritePlan::Write {
                request_id: primary_request_id.to_string(),
                replace_session_log: true,
                collision: false,
                preserve_existing_detail: true,
            })
        }
        Some((data_source, existing_semantic))
            if data_source.as_deref().unwrap_or("proxy") == "proxy"
                && existing_semantic == *semantic =>
        {
            // Allow a later identical replay to attach JSONL detail if the first
            // write only managed the SQLite summary (detail half-write failure).
            if !has_detail_locator(conn, primary_request_id)? {
                return Ok(UsageWritePlan::NeedsDetail {
                    request_id: primary_request_id.to_string(),
                });
            }
            Ok(UsageWritePlan::Skip)
        }
        Some(_) => {
            let fallback = format!("{}:collision:{}", primary_request_id, semantic.sha256_hex());
            if let Some((data_source, existing_semantic)) =
                load_existing_usage_semantic(conn, &fallback)?
            {
                if data_source.as_deref().unwrap_or("proxy") == "proxy"
                    && existing_semantic == *semantic
                {
                    if !has_detail_locator(conn, &fallback)? {
                        return Ok(UsageWritePlan::NeedsDetail {
                            request_id: fallback,
                        });
                    }
                    return Ok(UsageWritePlan::Skip);
                }
                return Err(format!(
                    "usage collision fallback key already occupied with different semantics: {fallback}"
                ));
            }
            Ok(UsageWritePlan::Write {
                request_id: fallback,
                replace_session_log: false,
                collision: true,
                preserve_existing_detail: false,
            })
        }
    }
}

fn load_existing_usage_semantic(
    conn: &Connection,
    request_id: &str,
) -> Result<Option<(Option<String>, UsageSemantic)>, String> {
    conn.query_row(
        "SELECT data_source, app_type, provider_id, model,
                input_tokens, output_tokens, cache_read_tokens,
                cache_creation_tokens, status_code,
                stream_outcome, error_category
         FROM proxy_request_logs WHERE request_id = ?1",
        [request_id],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                UsageSemantic {
                    app_type: row.get(1)?,
                    provider_id: row.get(2)?,
                    model: row.get(3)?,
                    input_tokens: row.get(4)?,
                    output_tokens: row.get(5)?,
                    cache_read_tokens: row.get(6)?,
                    cache_creation_tokens: row.get(7)?,
                    status_code: row.get(8)?,
                    stream_outcome: row.get(9)?,
                    error_category: row.get(10)?,
                },
            ))
        },
    )
    .optional()
    .map_err(|error| format!("Failed to query usage request_id: {error}"))
}

fn load_existing_detail_locator(
    conn: &Connection,
    request_id: &str,
) -> Result<Option<(Option<String>, Option<u64>)>, String> {
    conn.query_row(
        "SELECT detail_file, detail_offset FROM proxy_request_logs WHERE request_id = ?1",
        [request_id],
        |row| {
            let file: Option<String> = row.get(0)?;
            let offset: Option<i64> = row.get(1)?;
            Ok((file, offset.map(|value| value.max(0) as u64)))
        },
    )
    .optional()
    .map_err(|error| format!("Failed to query usage detail locator: {error}"))
}

fn has_detail_locator(conn: &Connection, request_id: &str) -> Result<bool, String> {
    let Some((file, offset)) = load_existing_detail_locator(conn, request_id)? else {
        return Ok(false);
    };
    let has_file = file
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    Ok(has_file && offset.is_some())
}

fn load_existing_session_id(conn: &Connection, request_id: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT session_id FROM proxy_request_logs WHERE request_id = ?1",
        [request_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .map(|value| value.flatten())
    .map_err(|error| format!("Failed to query usage session_id: {error}"))
}

pub fn request_logs(
    db: &SqliteDbState,
    filters: &GatewayRequestLogFilters,
    page: u32,
    page_size: u32,
) -> Result<GatewayPaginatedRequestLogs, String> {
    db.with_conn(|conn| {
        let provider_names = load_provider_names(conn)?;
        let page_size = page_size.clamp(1, MAX_PAGE_SIZE);
        let mut params: Vec<Box<dyn ToSql>> = Vec::new();
        let where_clause = build_detail_where(filters, &provider_names, &mut params)?;

        let count_sql = format!("SELECT COUNT(*) FROM proxy_request_logs l {where_clause}");
        let count_refs = to_param_refs(&params);
        let total = conn
            .query_row(&count_sql, count_refs.as_slice(), |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|error| format!("Failed to count proxy gateway request logs: {error}"))?
            .max(0) as u32;

        let offset = i64::from(page.saturating_mul(page_size));
        params.push(Box::new(i64::from(page_size)));
        params.push(Box::new(offset));
        let rows_refs = to_param_refs(&params);
        let sql = format!(
            "SELECT request_id, provider_id, app_type, model, request_model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    total_cost_usd, latency_ms, first_token_ms, COALESCE(duration_ms, latency_ms, 0),
                    status_code, error_message, created_at, is_streaming,
                    route_name, method, path, stream_outcome, reasoning_effort,
                    COALESCE(data_source, 'proxy'), json(usage_metadata), extra_tokens
             FROM proxy_request_logs l
             {where_clause}
             ORDER BY created_at DESC
             LIMIT ? OFFSET ?"
        );
        let mut stmt = conn.prepare(&sql).map_err(|error| {
            format!("Failed to prepare proxy gateway request log query: {error}")
        })?;
        let rows = stmt
            .query_map(rows_refs.as_slice(), |row| {
                let app_type: String = row.get(2)?;
                let provider_id: String = row.get(1)?;
                let Some(cli_key) = cli_key_from_app_type(&app_type) else {
                    return Ok(None);
                };
                let input_tokens = row.get::<_, i64>(5)?.max(0) as u64;
                let output_tokens = row.get::<_, i64>(6)?.max(0) as u64;
                let cache_read_tokens = row.get::<_, i64>(7)?.max(0) as u64;
                let cache_creation_tokens = row.get::<_, i64>(8)?.max(0) as u64;
                Ok(Some(GatewayRequestLogItem {
                    usage_metadata: row.get::<_, Option<String>>(23)?.and_then(|value| serde_json::from_str(&value).ok()),
                    extra_tokens: row.get::<_, i64>(24)?.max(0) as u64,
                    data_source: row.get(22)?,
                    trace_id: row.get(0)?,
                    cli_key,
                    route_name: row.get(17)?,
                    method: row.get(18)?,
                    path: row.get(19)?,
                    provider_id: provider_id.clone(),
                    provider_name: provider_names.get(&(app_type, provider_id)).cloned(),
                    upstream_model_id: row.get(3)?,
                    reasoning_effort: row.get(21)?,
                    requested_model: row.get(4)?,
                    status_code: row.get::<_, i64>(13)?.max(0) as u16,
                    success: {
                        let stream_outcome = row
                            .get::<_, Option<String>>(20)?
                            .filter(|value| !value.trim().is_empty());
                        match stream_outcome
                            .as_deref()
                            .and_then(GatewayStreamOutcome::from_str)
                        {
                            Some(outcome) => outcome.is_success(),
                            None => is_success_status(row.get::<_, i64>(13)?.max(0) as u16),
                        }
                    },
                    error_message: row.get(14)?,
                    created_at: timestamp_to_utc(row.get(15)?),
                    duration_ms: row.get::<_, i64>(12)?.max(0) as u64,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                    total_tokens: input_tokens
                        .saturating_add(output_tokens)
                        .saturating_add(cache_read_tokens)
                        .saturating_add(cache_creation_tokens)
                        .saturating_add(row.get::<_, i64>(24)?.max(0) as u64),
                    total_cost_usd: row.get(9)?,
                    is_streaming: row.get::<_, i64>(16)? != 0,
                    first_token_ms: row
                        .get::<_, Option<i64>>(11)?
                        .map(|value| value.max(0) as u64),
                }))
            })
            .map_err(|error| format!("Failed to query proxy gateway request logs: {error}"))?;

        let mut data = Vec::new();
        for row in rows {
            if let Some(item) =
                row.map_err(|error| format!("Failed to read request log row: {error}"))?
            {
                data.push(item);
            }
        }

        Ok(GatewayPaginatedRequestLogs {
            data,
            total,
            page,
            page_size,
        })
    })
}

fn optional_compact_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub fn usage_summary(
    db: &SqliteDbState,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
) -> Result<GatewayUsageSummary, String> {
    db.with_conn(|conn| {
        let mut params = Vec::<Box<dyn ToSql>>::new();
        let detail_where =
            build_usage_stats_where(start_date, end_date, cli_key, "l", true, &mut params);
        let refs = to_param_refs(&params);
        let mut summary = conn
            .query_row(
                &format!(
                    "SELECT COALESCE(SUM(usage_request_count), 0),
                            COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0),
                            COALESCE(SUM(input_tokens), 0),
                            COALESCE(SUM(output_tokens), 0),
                            COALESCE(SUM(cache_read_tokens), 0),
                            COALESCE(SUM(cache_creation_tokens), 0),
                            COALESCE(SUM(CASE WHEN (l.stream_outcome = 'completed' OR (l.stream_outcome IS NULL AND l.status_code >= 200 AND l.status_code < 400)) THEN l.usage_request_count ELSE 0 END), 0),
                            COALESCE(SUM(extra_tokens), 0)
                     FROM proxy_request_logs l {detail_where}"
                ),
                refs.as_slice(),
                row_to_summary_accumulator,
            )
            .map_err(|error| format!("Failed to summarize proxy gateway usage: {error}"))?;
        summary.add(rollup_summary(conn, start_date, end_date, cli_key)?);
        Ok(summary.into_summary())
    })
}

pub fn usage_summary_by_cli(
    db: &SqliteDbState,
    start_date: Option<i64>,
    end_date: Option<i64>,
) -> Result<Vec<GatewayUsageSummaryByCli>, String> {
    let mut items = Vec::new();
    for cli_key in GatewayUsageTool::all() {
        let summary = usage_summary(db, start_date, end_date, Some(cli_key))?;
        if summary.total_requests > 0
            || summary.total_tokens > 0
            || parse_decimal_or_default(&summary.total_cost_usd, Decimal::ZERO) != Decimal::ZERO
        {
            items.push(GatewayUsageSummaryByCli { cli_key, summary });
        }
    }
    Ok(items)
}

pub fn usage_trends(
    db: &SqliteDbState,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
) -> Result<Vec<GatewayUsageTrendPoint>, String> {
    db.with_conn(|conn| {
        let end = end_date.unwrap_or_else(|| Utc::now().timestamp());
        let start = start_date.unwrap_or(end - 24 * 60 * 60);
        let bucket_expr = if end.saturating_sub(start) <= 24 * 60 * 60 {
            "strftime('%Y-%m-%dT%H:00:00', created_at, 'unixepoch', 'localtime')"
        } else {
            "date(created_at, 'unixepoch', 'localtime')"
        };
        let mut trend_map = std::collections::BTreeMap::<String, TrendAccumulator>::new();
        let mut params = Vec::<Box<dyn ToSql>>::new();
        let where_clause =
            build_usage_stats_where(Some(start), Some(end), cli_key, "l", true, &mut params);
        let refs = to_param_refs(&params);
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {bucket_expr} AS bucket,
                        COALESCE(SUM(usage_request_count), 0),
                        COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0),
                        COALESCE(SUM(input_tokens), 0),
                        COALESCE(SUM(output_tokens), 0),
                        COALESCE(SUM(cache_read_tokens), 0),
                        COALESCE(SUM(cache_creation_tokens), 0),
                        COALESCE(SUM(extra_tokens), 0)
                 FROM proxy_request_logs l
                 {where_clause}
                 GROUP BY bucket
                 ORDER BY bucket ASC"
            ))
            .map_err(|error| format!("Failed to prepare proxy gateway trend query: {error}"))?;
        let rows = stmt
            .query_map(refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?.max(0) as u64,
                    row_decimal(row, 2)?,
                    row.get::<_, i64>(3)?.max(0) as u64,
                    row.get::<_, i64>(4)?.max(0) as u64,
                    row.get::<_, i64>(5)?.max(0) as u64,
                    row.get::<_, i64>(6)?.max(0) as u64,
                    row.get::<_, i64>(7)?.max(0) as u64,
                ))
            })
            .map_err(|error| format!("Failed to query proxy gateway trends: {error}"))?;
        for row in rows {
            let (
                bucket,
                request_count,
                total_cost_usd,
                input,
                output,
                cache_read,
                cache_creation,
                extra_tokens,
            ) = row.map_err(|error| format!("Failed to read trend row: {error}"))?;
            trend_map.entry(bucket).or_default().add(
                request_count,
                total_cost_usd,
                input,
                output,
                cache_read,
                cache_creation,
                extra_tokens,
            );
        }
        merge_rollup_trends(conn, &mut trend_map, start, end, cli_key)?;
        Ok(trend_map
            .into_iter()
            .map(|(date, item)| GatewayUsageTrendPoint {
                date,
                request_count: item.request_count,
                total_cost_usd: format_decimal_cost(item.total_cost_usd),
                total_tokens: item.total_tokens(),
                input_tokens: item.input_tokens,
                output_tokens: item.output_tokens,
                cache_read_tokens: item.cache_read_tokens,
                cache_creation_tokens: item.cache_creation_tokens,
            })
            .collect())
    })
}

pub fn provider_stats(
    db: &SqliteDbState,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
) -> Result<Vec<GatewayProviderStats>, String> {
    db.with_conn(|conn| {
        let provider_names = load_provider_names(conn)?;
        let mut stats_map = HashMap::<(String, String), StatsAccumulator>::new();
        let mut params = Vec::<Box<dyn ToSql>>::new();
        let where_clause =
            build_usage_stats_where(start_date, end_date, cli_key, "l", true, &mut params);
        let refs = to_param_refs(&params);
        let mut stmt = conn
            .prepare(&format!(
                "SELECT app_type, provider_id,
                        COALESCE(SUM(usage_request_count), 0),
                        COALESCE(SUM(input_tokens + output_tokens + cache_read_tokens + cache_creation_tokens + extra_tokens), 0),
                        COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0),
                        COALESCE(SUM(CASE WHEN (l.stream_outcome = 'completed' OR (l.stream_outcome IS NULL AND l.status_code >= 200 AND l.status_code < 400)) THEN l.usage_request_count ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN COALESCE(l.data_source, 'proxy') = 'proxy' THEN l.latency_ms ELSE 0 END), 0),
                        COALESCE(SUM(input_tokens), 0),
                        COALESCE(SUM(cache_read_tokens), 0),
                        COALESCE(SUM(cache_creation_tokens), 0),
                        COALESCE(SUM(CASE WHEN COALESCE(l.data_source, 'proxy') = 'proxy' THEN 1 ELSE 0 END), 0)
                 FROM proxy_request_logs l
                 {where_clause}
                 GROUP BY app_type, provider_id
                 ORDER BY 3 DESC"
            ))
            .map_err(|error| format!("Failed to prepare provider stats query: {error}"))?;
        let rows = stmt
            .query_map(refs.as_slice(), |row| {
                let app_type: String = row.get(0)?;
                let provider_id: String = row.get(1)?;
                let request_count = row.get::<_, i64>(2)?.max(0) as u64;
                let success_count = row.get::<_, i64>(5)?.max(0) as u64;
                let latency_weighted_sum = row.get::<_, f64>(6)?.max(0.0);
                Ok((
                    app_type,
                    provider_id,
                    request_count,
                    row.get::<_, i64>(3)?.max(0) as u64,
                    row_decimal(row, 4)?,
                    success_count,
                    latency_weighted_sum,
                    row.get::<_, i64>(7)?.max(0) as u64,
                    row.get::<_, i64>(8)?.max(0) as u64,
                    row.get::<_, i64>(9)?.max(0) as u64,
                    row.get::<_, i64>(10)?.max(0) as u64,
                ))
            })
            .map_err(|error| format!("Failed to query provider stats: {error}"))?;
        for row in rows {
            let (
                app_type,
                provider_id,
                request_count,
                total_tokens,
                total_cost,
                success_count,
                latency_weighted_sum,
                input_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                latency_sample_count,
            ) = row.map_err(|error| format!("Failed to read gateway stats row: {error}"))?;
            let item = stats_map.entry((app_type, provider_id)).or_default();
            item.add(
                    request_count,
                    success_count,
                    total_tokens,
                    total_cost,
                    latency_weighted_sum,
                    latency_sample_count,
                );
            item.add_input_usage(input_tokens, cache_read_tokens, cache_creation_tokens);
        }
        merge_rollup_provider_stats(conn, &mut stats_map, start_date, end_date, cli_key)?;
        let mut items = stats_map
            .into_iter()
            .filter_map(|((app_type, provider_id), item)| {
                let cli_key = cli_key_from_app_type(&app_type)?;
                Some(GatewayProviderStats {
                    cli_key,
                    provider_name: provider_names
                        .get(&(app_type, provider_id.clone()))
                        .cloned(),
                    provider_id,
                    request_count: item.request_count,
                    total_tokens: item.total_tokens,
                    total_cost_usd: format_decimal_cost(item.total_cost_usd),
                    success_rate: percent(item.success_count, item.request_count),
                    avg_latency_ms: item.avg_latency_ms(),
                    cache_hit_rate: item.cache_hit_rate(),
                })
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| right.request_count.cmp(&left.request_count));
        Ok(items)
    })
}

pub fn model_stats(
    db: &SqliteDbState,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
) -> Result<Vec<GatewayModelStats>, String> {
    db.with_conn(|conn| {
        let mut stats_map = HashMap::<(String, String), StatsAccumulator>::new();
        let mut params = Vec::<Box<dyn ToSql>>::new();
        let where_clause =
            build_usage_stats_where(start_date, end_date, cli_key, "l", false, &mut params);
        let model_expr = model_stats_detail_model_expression("l");
        let refs = to_param_refs(&params);
        let mut stmt = conn
            .prepare(&format!(
                "SELECT app_type, {model_expr} AS stats_model,
                        COALESCE(SUM(usage_request_count), 0),
                        COALESCE(SUM(input_tokens + output_tokens + cache_read_tokens + cache_creation_tokens + extra_tokens), 0),
                        COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0),
                        COALESCE(SUM(CASE WHEN COALESCE(l.data_source, 'proxy') = 'proxy' THEN l.latency_ms ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN COALESCE(l.data_source, 'proxy') = 'proxy' THEN 1 ELSE 0 END), 0)
                 FROM proxy_request_logs l
                 {where_clause}
                 GROUP BY app_type, stats_model
                 ORDER BY 3 DESC"
            ))
            .map_err(|error| format!("Failed to prepare model stats query: {error}"))?;
        let rows = stmt
            .query_map(refs.as_slice(), |row| {
                let app_type: String = row.get(0)?;
                let request_count = row.get::<_, i64>(2)?.max(0) as u64;
                let latency_weighted_sum = row.get::<_, f64>(5)?.max(0.0);
                Ok((
                    app_type,
                    row.get::<_, String>(1)?,
                    request_count,
                    row.get::<_, i64>(3)?.max(0) as u64,
                    row_decimal(row, 4)?,
                    latency_weighted_sum,
                    row.get::<_, i64>(6)?.max(0) as u64,
                ))
            })
            .map_err(|error| format!("Failed to query model stats: {error}"))?;
        for row in rows {
            let (app_type, model, request_count, total_tokens, total_cost, latency_weighted_sum, latency_sample_count) =
                row.map_err(|error| format!("Failed to read gateway stats row: {error}"))?;
            stats_map
                .entry((app_type, model))
                .or_default()
                .add(request_count, 0, total_tokens, total_cost, latency_weighted_sum, latency_sample_count);
        }
        merge_rollup_model_stats(conn, &mut stats_map, start_date, end_date, cli_key)?;
        let mut items = stats_map
            .into_iter()
            .filter_map(|((app_type, model), item)| {
                let cli_key = cli_key_from_app_type(&app_type)?;
                Some(GatewayModelStats {
                    cli_key,
                    model,
                    request_count: item.request_count,
                    total_tokens: item.total_tokens,
                    total_cost_usd: format_decimal_cost(item.total_cost_usd),
                    avg_latency_ms: item.avg_latency_ms(),
                })
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| right.request_count.cmp(&left.request_count));
        Ok(items)
    })
}

pub fn data_source_breakdown(
    db: &SqliteDbState,
    input: super::types::DataSourceBreakdownInput,
) -> Result<Vec<super::types::DataSourceBreakdownItem>, String> {
    db.with_conn(|conn| {
        let mut params = Vec::<Box<dyn ToSql>>::new();
        let where_clause = build_stats_where(
            input.start_unix_secs,
            input.end_unix_secs,
            input.cli_key,
            "l",
            &mut params,
        );
        let refs = to_param_refs(&params);
        let mut stmt = conn
            .prepare(&format!(
                "SELECT COALESCE(NULLIF(TRIM(l.data_source), ''), 'proxy') AS data_source,
                        COUNT(*) AS request_count
                 FROM proxy_request_logs l
                 {where_clause}
                 GROUP BY COALESCE(NULLIF(TRIM(l.data_source), ''), 'proxy')
                 ORDER BY request_count DESC"
            ))
            .map_err(|error| format!("Failed to prepare data source breakdown query: {error}"))?;
        let rows = stmt
            .query_map(refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?.max(0) as u64,
                ))
            })
            .map_err(|error| format!("Failed to query data source breakdown: {error}"))?;
        let mut items = Vec::new();
        for row in rows {
            let (data_source, request_count) =
                row.map_err(|error| format!("Failed to read data source breakdown row: {error}"))?;
            items.push(super::types::DataSourceBreakdownItem {
                data_source,
                request_count,
            });
        }
        Ok(items)
    })
}

fn merge_rollup_trends(
    conn: &Connection,
    trend_map: &mut std::collections::BTreeMap<String, TrendAccumulator>,
    start: i64,
    end: i64,
    cli_key: Option<GatewayUsageTool>,
) -> Result<(), String> {
    let mut params = Vec::<Box<dyn ToSql>>::new();
    let where_clause = build_rollup_where(Some(start), Some(end), cli_key, Some("r"), &mut params);
    let refs = to_param_refs(&params);
    let mut stmt = conn
        .prepare(&format!(
            "SELECT r.date,
                    COALESCE(SUM(r.request_count), 0),
                    COALESCE(SUM(CAST(r.total_cost_usd AS REAL)), 0),
                    COALESCE(SUM(r.input_tokens), 0),
                    COALESCE(SUM(r.output_tokens), 0),
                    COALESCE(SUM(r.cache_read_tokens), 0),
                    COALESCE(SUM(r.cache_creation_tokens), 0),
                    COALESCE(SUM(r.extra_tokens), 0)
             FROM usage_daily_rollups r
             {where_clause}
             GROUP BY r.date
             ORDER BY r.date ASC"
        ))
        .map_err(|error| format!("Failed to prepare gateway rollup trend query: {error}"))?;
    let rows = stmt
        .query_map(refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?.max(0) as u64,
                row_decimal(row, 2)?,
                row.get::<_, i64>(3)?.max(0) as u64,
                row.get::<_, i64>(4)?.max(0) as u64,
                row.get::<_, i64>(5)?.max(0) as u64,
                row.get::<_, i64>(6)?.max(0) as u64,
                row.get::<_, i64>(7)?.max(0) as u64,
            ))
        })
        .map_err(|error| format!("Failed to query gateway rollup trends: {error}"))?;
    for row in rows {
        let (
            date,
            request_count,
            total_cost,
            input,
            output,
            cache_read,
            cache_creation,
            extra_tokens,
        ) = row.map_err(|error| format!("Failed to read gateway rollup trend row: {error}"))?;
        trend_map.entry(date).or_default().add(
            request_count,
            total_cost,
            input,
            output,
            cache_read,
            cache_creation,
            extra_tokens,
        );
    }
    Ok(())
}

fn merge_rollup_provider_stats(
    conn: &Connection,
    stats_map: &mut HashMap<(String, String), StatsAccumulator>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
) -> Result<(), String> {
    let mut params = Vec::<Box<dyn ToSql>>::new();
    let where_clause = build_rollup_where(start_date, end_date, cli_key, Some("r"), &mut params);
    let refs = to_param_refs(&params);
    let mut stmt = conn
        .prepare(&format!(
            "SELECT r.app_type, r.provider_id,
                    COALESCE(SUM(r.request_count), 0),
                    COALESCE(SUM(r.input_tokens + r.output_tokens + r.cache_read_tokens + r.cache_creation_tokens + r.extra_tokens), 0),
                    COALESCE(SUM(CAST(r.total_cost_usd AS REAL)), 0),
                    COALESCE(SUM(r.success_count), 0),
                    COALESCE(SUM(r.avg_latency_ms * COALESCE(r.latency_sample_count, r.request_count)), 0),
                    COALESCE(SUM(r.input_tokens), 0),
                    COALESCE(SUM(r.cache_read_tokens), 0),
                    COALESCE(SUM(r.cache_creation_tokens), 0),
                    COALESCE(SUM(COALESCE(r.latency_sample_count, r.request_count)), 0)
             FROM usage_daily_rollups r
             {where_clause}
             GROUP BY r.app_type, r.provider_id"
        ))
        .map_err(|error| format!("Failed to prepare gateway provider rollup query: {error}"))?;
    let rows = stmt
        .query_map(refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?.max(0) as u64,
                row.get::<_, i64>(3)?.max(0) as u64,
                row_decimal(row, 4)?,
                row.get::<_, i64>(5)?.max(0) as u64,
                row.get::<_, f64>(6)?.max(0.0),
                row.get::<_, i64>(7)?.max(0) as u64,
                row.get::<_, i64>(8)?.max(0) as u64,
                row.get::<_, i64>(9)?.max(0) as u64,
                row.get::<_, i64>(10)?.max(0) as u64,
            ))
        })
        .map_err(|error| format!("Failed to query gateway provider rollups: {error}"))?;
    for row in rows {
        let (
            app_type,
            provider_id,
            request_count,
            total_tokens,
            total_cost,
            success_count,
            latency_weighted_sum,
            input_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            latency_sample_count,
        ) = row.map_err(|error| format!("Failed to read provider rollup row: {error}"))?;
        let item = stats_map.entry((app_type, provider_id)).or_default();
        item.add(
            request_count,
            success_count,
            total_tokens,
            total_cost,
            latency_weighted_sum,
            latency_sample_count,
        );
        item.add_input_usage(input_tokens, cache_read_tokens, cache_creation_tokens);
    }
    Ok(())
}

fn merge_rollup_model_stats(
    conn: &Connection,
    stats_map: &mut HashMap<(String, String), StatsAccumulator>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
) -> Result<(), String> {
    let mut params = Vec::<Box<dyn ToSql>>::new();
    let mut where_clause =
        build_rollup_where(start_date, end_date, cli_key, Some("r"), &mut params);
    append_static_where_condition(
        &mut where_clause,
        &format!("r.model != '{COMPACT_ROLLUP_MODEL}'"),
    );
    let refs = to_param_refs(&params);
    let mut stmt = conn
        .prepare(&format!(
            "SELECT r.app_type, r.model,
                    COALESCE(SUM(r.request_count), 0),
                    COALESCE(SUM(r.input_tokens + r.output_tokens + r.cache_read_tokens + r.cache_creation_tokens + r.extra_tokens), 0),
                    COALESCE(SUM(CAST(r.total_cost_usd AS REAL)), 0),
                    COALESCE(SUM(r.avg_latency_ms * COALESCE(r.latency_sample_count, r.request_count)), 0),
                    COALESCE(SUM(COALESCE(r.latency_sample_count, r.request_count)), 0)
             FROM usage_daily_rollups r
             {where_clause}
             GROUP BY r.app_type, r.model"
        ))
        .map_err(|error| format!("Failed to prepare gateway model rollup query: {error}"))?;
    let rows = stmt
        .query_map(refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?.max(0) as u64,
                row.get::<_, i64>(3)?.max(0) as u64,
                row_decimal(row, 4)?,
                row.get::<_, f64>(5)?.max(0.0),
                row.get::<_, i64>(6)?.max(0) as u64,
            ))
        })
        .map_err(|error| format!("Failed to query gateway model rollups: {error}"))?;
    for row in rows {
        let (
            app_type,
            model,
            request_count,
            total_tokens,
            total_cost,
            latency_weighted_sum,
            latency_sample_count,
        ) = row.map_err(|error| format!("Failed to read model rollup row: {error}"))?;
        stats_map.entry((app_type, model)).or_default().add(
            request_count,
            0,
            total_tokens,
            total_cost,
            latency_weighted_sum,
            latency_sample_count,
        );
    }
    Ok(())
}

fn rollup_summary(
    conn: &Connection,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
) -> Result<SummaryAccumulator, String> {
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();
    let where_clause = build_rollup_where(start_date, end_date, cli_key, None, &mut params);
    let refs = to_param_refs(&params);
    conn.query_row(
        &format!(
            "SELECT COALESCE(SUM(request_count), 0),
                    COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_read_tokens), 0),
                    COALESCE(SUM(cache_creation_tokens), 0),
                    COALESCE(SUM(success_count), 0),
                    COALESCE(SUM(extra_tokens), 0)
             FROM usage_daily_rollups {where_clause}"
        ),
        refs.as_slice(),
        row_to_summary_accumulator,
    )
    .map_err(|error| format!("Failed to summarize gateway usage rollups: {error}"))
}

fn row_to_summary_accumulator(row: &rusqlite::Row<'_>) -> rusqlite::Result<SummaryAccumulator> {
    let total_requests = row.get::<_, i64>(0)?.max(0) as u64;
    let input = row.get::<_, i64>(2)?.max(0) as u64;
    let output = row.get::<_, i64>(3)?.max(0) as u64;
    let cache_read = row.get::<_, i64>(4)?.max(0) as u64;
    let cache_creation = row.get::<_, i64>(5)?.max(0) as u64;
    let success_count = row.get::<_, i64>(6)?.max(0) as u64;
    Ok(SummaryAccumulator {
        extra_tokens: row.get::<_, i64>(7)?.max(0) as u64,
        total_requests,
        success_count,
        total_cost_usd: row_decimal(row, 1)?,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_creation_tokens: cache_creation,
    })
}

pub(super) fn rollup_and_prune(conn: &Connection, retain_days: i64) -> Result<(), String> {
    if retain_days <= 0 {
        return Ok(());
    }
    let cutoff = local_midnight_cutoff(retain_days)?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM proxy_request_logs WHERE created_at < ?1",
            [cutoff],
            |row| row.get(0),
        )
        .map_err(|error| format!("Failed to count old gateway logs: {error}"))?;
    if count == 0 {
        return Ok(());
    }

    let usage_condition = usage_applicable_detail_condition("l", true);
    let rollup_model_expr = rollup_detail_model_expression("l");
    let transaction = conn
        .unchecked_transaction()
        .map_err(|error| format!("Failed to start usage rollup transaction: {error}"))?;
    transaction.execute(
        &format!(
            "INSERT OR REPLACE INTO usage_daily_rollups
            (date, app_type, provider_id, model, request_count, success_count,
             input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
             total_cost_usd, avg_latency_ms, latency_sample_count, extra_tokens)
         SELECT
            agg.d, agg.app_type, agg.provider_id, agg.rollup_model,
            COALESCE(old.request_count, 0) + agg.request_count,
            COALESCE(old.success_count, 0) + agg.success_count,
            COALESCE(old.input_tokens, 0) + agg.input_tokens,
            COALESCE(old.output_tokens, 0) + agg.output_tokens,
            COALESCE(old.cache_read_tokens, 0) + agg.cache_read_tokens,
            COALESCE(old.cache_creation_tokens, 0) + agg.cache_creation_tokens,
            CAST(COALESCE(CAST(old.total_cost_usd AS REAL), 0) + agg.total_cost AS TEXT),
            CASE WHEN COALESCE(old.latency_sample_count, old.request_count, 0) + agg.latency_sample_count > 0
                THEN (COALESCE(old.avg_latency_ms, 0) * COALESCE(old.latency_sample_count, old.request_count, 0)
                      + agg.latency_sum)
                     / (COALESCE(old.latency_sample_count, old.request_count, 0) + agg.latency_sample_count)
                ELSE 0 END,
            COALESCE(old.latency_sample_count, old.request_count, 0) + agg.latency_sample_count,
            COALESCE(old.extra_tokens, 0) + agg.extra_tokens
         FROM (
            SELECT date(l.created_at, 'unixepoch', 'localtime') AS d,
                   l.app_type,
                   l.provider_id,
                   {rollup_model_expr} AS rollup_model,
                   SUM(l.usage_request_count) AS request_count,
                   SUM(CASE WHEN (l.stream_outcome = 'completed' OR (l.stream_outcome IS NULL AND l.status_code >= 200 AND l.status_code < 400)) THEN l.usage_request_count ELSE 0 END) AS success_count,
                   COALESCE(SUM(l.input_tokens), 0) AS input_tokens,
                   COALESCE(SUM(l.output_tokens), 0) AS output_tokens,
                   COALESCE(SUM(l.cache_read_tokens), 0) AS cache_read_tokens,
                   COALESCE(SUM(l.cache_creation_tokens), 0) AS cache_creation_tokens,
                   COALESCE(SUM(l.extra_tokens), 0) AS extra_tokens,
                   COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0) AS total_cost,
                   COALESCE(SUM(CASE WHEN COALESCE(l.data_source, 'proxy') = 'proxy' THEN l.latency_ms ELSE 0 END), 0) * 1.0 AS latency_sum,
                   SUM(CASE WHEN COALESCE(l.data_source, 'proxy') = 'proxy' THEN 1 ELSE 0 END) AS latency_sample_count
            FROM proxy_request_logs l
            WHERE l.created_at < ?1
              AND {usage_condition}
            GROUP BY d, l.app_type, l.provider_id, rollup_model
         ) agg
         LEFT JOIN usage_daily_rollups old
            ON old.date = agg.d
            AND old.app_type = agg.app_type
            AND old.provider_id = agg.provider_id
            AND old.model = agg.rollup_model"
        ),
        [cutoff],
    )
    .map_err(|error| format!("Failed to roll up gateway logs: {error}"))?;
    super::session_import::mark_archived_contributions(&transaction, cutoff)?;
    transaction
        .execute(
            "DELETE FROM proxy_request_logs WHERE created_at < ?1",
            [cutoff],
        )
        .map_err(|error| format!("Failed to prune gateway logs: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Failed to commit usage rollup: {error}"))
}

fn maybe_rollup_and_prune(conn: &Connection, retain_days: i64) -> Result<(), String> {
    if retain_days <= 0 {
        return Ok(());
    }
    let guard = LAST_ROLLUP_PRUNE_AT.get_or_init(|| Mutex::new(None));
    let mut last_run = guard
        .lock()
        .map_err(|_| "Gateway rollup throttle lock poisoned".to_string())?;
    let should_run = last_run
        .map(|instant| instant.elapsed() >= StdDuration::from_secs(ROLLUP_THROTTLE_SECONDS))
        .unwrap_or(true);
    if !should_run {
        return Ok(());
    }
    let result = rollup_and_prune(conn, retain_days);
    // Failed attempts are throttled too, so one maintenance failure does not
    // repeat on every live request. The next interval retries it.
    *last_run = Some(Instant::now());
    result
}

fn local_midnight_cutoff(retain_days: i64) -> Result<i64, String> {
    let target_day = Local::now()
        .checked_sub_signed(Duration::days(retain_days))
        .ok_or_else(|| "Gateway log retention cutoff overflow".to_string())?
        .date_naive();
    let next_day = target_day
        .succ_opt()
        .ok_or_else(|| "Gateway log retention next day overflow".to_string())?;
    let midnight = next_day
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| "Gateway log retention midnight overflow".to_string())?;
    let local_time = Local
        .from_local_datetime(&midnight)
        .earliest()
        .ok_or_else(|| "Gateway log retention local time is invalid".to_string())?;
    Ok(local_time.timestamp())
}

fn build_detail_where(
    filters: &GatewayRequestLogFilters,
    provider_names: &ProviderNameMap,
    params: &mut Vec<Box<dyn ToSql>>,
) -> Result<String, String> {
    let mut conditions = Vec::new();
    if let Some(source) = filters.data_source.as_deref() {
        if !matches!(source, "proxy" | "session") {
            return Err("Unsupported usage data source".into());
        }
        push_condition(
            &mut conditions,
            params,
            "COALESCE(l.data_source, 'proxy')",
            source.to_string(),
        );
    }
    if let Some(cli_key) = filters.cli_key {
        push_condition(
            &mut conditions,
            params,
            "l.app_type",
            cli_key.as_str().to_string(),
        );
    }
    if let Some(status_code) = filters.status_code {
        conditions.push("COALESCE(l.data_source, 'proxy') = 'proxy'".to_string());
        push_condition(
            &mut conditions,
            params,
            "l.status_code",
            i64::from(status_code),
        );
    }
    if let Some(start) = filters.start_date {
        conditions.push(format!("l.created_at >= ?{}", params.len() + 1));
        params.push(Box::new(start));
    }
    if let Some(end) = filters.end_date {
        conditions.push(format!("l.created_at <= ?{}", params.len() + 1));
        params.push(Box::new(end));
    }
    if let Some(model) = filters
        .model
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        let pattern = format!("%{model}%");
        conditions.push(format!(
            "(l.model LIKE ?{} OR l.request_model LIKE ?{})",
            params.len() + 1,
            params.len() + 2
        ));
        params.push(Box::new(pattern.clone()));
        params.push(Box::new(pattern));
    }
    if let Some(provider_name) = filters
        .provider_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let needle = provider_name.to_ascii_lowercase();
        let mut matches = provider_names
            .iter()
            .filter(|(_, name)| name.to_ascii_lowercase().contains(&needle))
            .map(|((app_type, provider_id), _)| (app_type.clone(), provider_id.clone()))
            .collect::<Vec<_>>();
        matches.sort();
        matches.dedup();
        let mut parts = Vec::new();
        parts.push(format!("LOWER(l.provider_id) LIKE ?{}", params.len() + 1));
        parts.push(format!(
            "LOWER(COALESCE(json_extract(l.usage_metadata, '$.native_provider'), '')) LIKE ?{}",
            params.len() + 1
        ));
        params.push(Box::new(format!("%{needle}%")));
        for (app_type, provider_id) in matches {
            parts.push(format!(
                "(l.app_type = ?{} AND l.provider_id = ?{})",
                params.len() + 1,
                params.len() + 2
            ));
            params.push(Box::new(app_type));
            params.push(Box::new(provider_id));
        }
        conditions.push(format!("({})", parts.join(" OR ")));
    }
    if filters.exclude_model_list.unwrap_or(false) {
        conditions.push(format!("NOT {}", model_list_request_sql_condition("l")));
    }
    if filters.only_failed.unwrap_or(false) {
        // Surface only failed requests. A request counts as failed when its
        // HTTP status is non-2xx/3xx, OR its recorded stream outcome is one of
        // the non-success terminal verdicts (incomplete/failed/canceled). This
        // catches mid-stream failures on an already-written 200 that the old
        // status-code-only filter missed.
        let param_index = params.len() + 1;
        params.push(Box::new("incomplete".to_string()));
        params.push(Box::new("failed".to_string()));
        params.push(Box::new("canceled".to_string()));
        conditions.push(format!(
            "(l.status_code < 200 OR l.status_code >= 400 OR l.stream_outcome IN (?{param_index}, ?{}, ?{}))",
            param_index + 1,
            param_index + 2,
        ));
    }

    if conditions.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("WHERE {}", conditions.join(" AND ")))
    }
}

fn build_stats_where(
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
    alias: &str,
    params: &mut Vec<Box<dyn ToSql>>,
) -> String {
    format_where_clause(build_stats_conditions(
        start_date, end_date, cli_key, alias, params,
    ))
}

fn build_usage_stats_where(
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
    alias: &str,
    include_compact: bool,
    params: &mut Vec<Box<dyn ToSql>>,
) -> String {
    let mut conditions = build_stats_conditions(start_date, end_date, cli_key, alias, params);
    conditions.push(usage_applicable_detail_condition(alias, include_compact));
    format_where_clause(conditions)
}

fn build_stats_conditions(
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
    alias: &str,
    params: &mut Vec<Box<dyn ToSql>>,
) -> Vec<String> {
    let mut conditions = Vec::new();
    if let Some(start) = start_date {
        conditions.push(format!("{alias}.created_at >= ?{}", params.len() + 1));
        params.push(Box::new(start));
    }
    if let Some(end) = end_date {
        conditions.push(format!("{alias}.created_at <= ?{}", params.len() + 1));
        params.push(Box::new(end));
    }
    if let Some(cli_key) = cli_key {
        conditions.push(format!("{alias}.app_type = ?{}", params.len() + 1));
        params.push(Box::new(cli_key.as_str().to_string()));
    }
    conditions
}

fn format_where_clause(conditions: Vec<String>) -> String {
    if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    }
}

fn append_static_where_condition(where_clause: &mut String, condition: &str) {
    if where_clause.trim().is_empty() {
        *where_clause = format!("WHERE {condition}");
    } else {
        where_clause.push_str(" AND ");
        where_clause.push_str(condition);
    }
}

fn build_rollup_where(
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayUsageTool>,
    alias: Option<&str>,
    params: &mut Vec<Box<dyn ToSql>>,
) -> String {
    let prefix = alias.map(|value| format!("{value}.")).unwrap_or_default();
    let mut conditions = Vec::new();
    if let Some(start) = start_date {
        conditions.push(format!(
            "{prefix}date >= date(?{}, 'unixepoch', 'localtime')",
            params.len() + 1
        ));
        params.push(Box::new(start));
    }
    if let Some(end) = end_date {
        conditions.push(format!(
            "{prefix}date <= date(?{}, 'unixepoch', 'localtime')",
            params.len() + 1
        ));
        params.push(Box::new(end));
    }
    if let Some(cli_key) = cli_key {
        conditions.push(format!("{prefix}app_type = ?{}", params.len() + 1));
        params.push(Box::new(cli_key.as_str().to_string()));
    }
    // Native imports have a reserved provider identity even when their
    // transcript does not record a model name. Rollups retain that identity.
    conditions.push(format!(
        "({} OR {prefix}provider_id = 'session')",
        valid_model_sql_condition(alias)
    ));
    format_where_clause(conditions)
}

fn usage_applicable_detail_condition(alias: &str, include_compact: bool) -> String {
    let valid_model = format!(
        "({} OR {alias}.data_source = 'session')",
        valid_detail_model_sql_condition(alias)
    );
    if include_compact {
        format!(
            "({valid_model} OR {})",
            compact_request_sql_condition(alias)
        )
    } else {
        valid_model
    }
}

fn valid_detail_model_sql_condition(alias: &str) -> String {
    format!(
        "({} OR {})",
        valid_model_column_sql_condition(Some(alias), "model"),
        valid_model_column_sql_condition(Some(alias), "request_model")
    )
}

fn model_stats_detail_model_expression(alias: &str) -> String {
    format!(
        "CASE WHEN {} THEN {alias}.model ELSE COALESCE({alias}.request_model, 'unknown') END",
        valid_model_column_sql_condition(Some(alias), "model")
    )
}

fn rollup_detail_model_expression(alias: &str) -> String {
    format!(
        "CASE WHEN {} THEN {alias}.model \
         WHEN {} THEN {alias}.request_model \
         WHEN {alias}.data_source = 'session' THEN 'unknown' \
         ELSE '{COMPACT_ROLLUP_MODEL}' END",
        valid_model_column_sql_condition(Some(alias), "model"),
        valid_model_column_sql_condition(Some(alias), "request_model")
    )
}

fn valid_model_sql_condition(alias: Option<&str>) -> String {
    format!(
        "({} OR {})",
        valid_model_column_sql_condition(alias, "model"),
        compact_rollup_model_sql_condition(alias)
    )
}

fn valid_model_column_sql_condition(alias: Option<&str>, column: &str) -> String {
    let column_ref = alias
        .map(|alias| format!("{alias}.{column}"))
        .unwrap_or_else(|| column.to_string());
    format!("LOWER(TRIM(COALESCE({column_ref}, ''))) NOT IN ('', 'unknown', 'null', 'none')")
}

fn compact_rollup_model_sql_condition(alias: Option<&str>) -> String {
    let column_ref = alias
        .map(|alias| format!("{alias}.model"))
        .unwrap_or_else(|| "model".to_string());
    format!("{column_ref} = '{COMPACT_ROLLUP_MODEL}'")
}

fn compact_request_sql_condition(alias: &str) -> String {
    format!(
        "(UPPER(TRIM(COALESCE({alias}.method, ''))) = 'POST' \
         AND (LOWER(COALESCE({alias}.path, '')) LIKE '%/responses/compact' \
              OR LOWER(COALESCE({alias}.path, '')) LIKE '%/responses/compact?%'))"
    )
}

/// Matches gateway "model list" requests: GET/HEAD to .../models or .../models:listModels.
/// Keep this aligned with frontend `gatewayRequestDisplayKind` modelList detection.
fn model_list_request_sql_condition(alias: &str) -> String {
    // Strip query string and trailing slashes, then require the final segment to be
    // `models` or `models:listmodels` (case-insensitive).
    let path_only = format!(
        "RTRIM( \
           CASE \
             WHEN INSTR(LOWER(COALESCE({alias}.path, '')), '?') > 0 \
               THEN SUBSTR(LOWER(COALESCE({alias}.path, '')), 1, INSTR(LOWER(COALESCE({alias}.path, '')), '?') - 1) \
             ELSE LOWER(TRIM(COALESCE({alias}.path, ''))) \
           END, \
           '/' \
         )"
    );
    format!(
        "(UPPER(TRIM(COALESCE({alias}.method, ''))) IN ('GET', 'HEAD') \
         AND ( \
           {path_only} = 'models' \
           OR {path_only} LIKE '%/models' \
           OR {path_only} = 'models:listmodels' \
           OR {path_only} LIKE '%/models:listmodels' \
         ))"
    )
}

fn push_condition<T: ToSql + 'static>(
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn ToSql>>,
    field: &str,
    value: T,
) {
    conditions.push(format!("{field} = ?{}", params.len() + 1));
    params.push(Box::new(value));
}

fn load_provider_names(conn: &Connection) -> Result<ProviderNameMap, String> {
    let mut names = HashMap::new();
    for (app_type, table) in [
        ("claude", "claude_provider"),
        ("claude_desktop", "claude_desktop_provider"),
        ("codex", "codex_provider"),
        ("gemini", "gemini_cli_provider"),
        ("grok", "grok_provider"),
        ("kimi", "kimi_provider"),
    ] {
        let sql = format!("SELECT id, json_extract(data, '$.name') FROM {table}");
        let mut stmt = conn.prepare(&sql).map_err(|error| {
            format!("Failed to prepare provider name query for {table}: {error}")
        })?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|error| format!("Failed to query provider names from {table}: {error}"))?;
        for row in rows {
            let (id, name) =
                row.map_err(|error| format!("Failed to read provider name row: {error}"))?;
            if let Some(name) = name
                .map(|value| value.trim().to_string())
                .filter(|v| !v.is_empty())
            {
                names.insert((app_type.to_string(), id), name);
            }
        }
    }
    load_opencode_provider_names(conn, &mut names)?;
    Ok(names)
}

fn load_opencode_provider_names(
    conn: &Connection,
    names: &mut ProviderNameMap,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT json_extract(data, '$.provider_id'),
                    json_extract(data, '$.provider_config.name')
             FROM opencode_favorite_provider",
        )
        .map_err(|error| format!("Failed to prepare OpenCode provider name query: {error}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })
        .map_err(|error| format!("Failed to query OpenCode provider names: {error}"))?;
    for row in rows {
        let (provider_id, name) =
            row.map_err(|error| format!("Failed to read OpenCode provider name row: {error}"))?;
        let Some(provider_id) = provider_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if let Some(name) = name
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            names.insert(("opencode".to_string(), provider_id), name);
        }
    }
    Ok(())
}

fn cli_key_from_app_type(app_type: &str) -> Option<GatewayUsageTool> {
    GatewayUsageTool::all()
        .into_iter()
        .find(|tool| tool.as_str() == app_type)
}

fn timestamp_to_utc(timestamp: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap())
}

fn is_success_status(status_code: u16) -> bool {
    (200..400).contains(&status_code)
}

fn percent(numerator: u64, denominator: u64) -> f32 {
    if denominator == 0 {
        0.0
    } else {
        ((numerator as f64 / denominator as f64) * 1000.0).round() as f32 / 10.0
    }
}

fn to_param_refs(params: &[Box<dyn ToSql>]) -> Vec<&dyn ToSql> {
    params.iter().map(|param| param.as_ref()).collect()
}

fn calculate_cost(
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    pricing: &ModelPricing,
) -> CostBreakdown {
    CostBreakdown {
        input_cost_usd: token_cost(input_tokens, pricing.input_cost_per_million),
        output_cost_usd: token_cost(output_tokens, pricing.output_cost_per_million),
        cache_read_cost_usd: token_cost(cache_read_tokens, pricing.cache_read_cost_per_million),
        cache_creation_cost_usd: token_cost(
            cache_creation_tokens,
            pricing.cache_creation_cost_per_million,
        ),
    }
}

fn token_cost(tokens: u64, cost_per_million: Decimal) -> Decimal {
    Decimal::from(tokens) * cost_per_million / Decimal::from(1_000_000_u64)
}

pub(super) fn format_decimal_cost(value: Decimal) -> String {
    format!("{:.6}", value.round_dp(6))
}

pub(super) fn calculate_session_costs(
    conn: &Connection,
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
) -> CostBreakdown {
    find_model_pricing(conn, model)
        .map(|pricing| {
            calculate_cost(
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                &pricing,
            )
        })
        .unwrap_or_default()
}

pub(super) fn session_model_has_pricing(conn: &Connection, model: &str) -> bool {
    find_model_pricing(conn, model).is_some()
}

fn parse_decimal_or_default(value: &str, default: Decimal) -> Decimal {
    Decimal::from_str(value.trim()).unwrap_or(default)
}

fn decimal_from_f64(value: f64) -> Decimal {
    Decimal::from_f64(value).unwrap_or_default()
}

fn row_decimal(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Decimal> {
    row.get::<_, f64>(index).map(decimal_from_f64)
}

fn find_model_pricing(conn: &Connection, model_id: &str) -> Option<ModelPricing> {
    let candidates = model_pricing_candidates(model_id);
    find_pricing_for_candidates(conn, &candidates)
}

pub(super) fn session_model_needs_short_date_fallback(conn: &Connection, model: &str) -> bool {
    find_pricing_for_candidates(conn, &pricing_candidates_with_short_dates(model, false)).is_none()
        && find_model_pricing(conn, model).is_some()
}

fn find_pricing_for_candidates(conn: &Connection, candidates: &[String]) -> Option<ModelPricing> {
    for candidate in candidates {
        if let Some(pricing) = query_model_pricing_exact(conn, &candidate) {
            return Some(pricing);
        }
    }

    for candidate in candidates {
        if !should_try_pricing_prefix_match(candidate) {
            continue;
        }
        if let Some(pricing) = query_model_pricing_prefix(conn, &candidate) {
            return Some(pricing);
        }
    }
    None
}

fn find_summary_model_pricing(
    conn: &Connection,
    summary: &GatewayRequestLogSummary,
    upstream_model: &str,
) -> Option<ModelPricing> {
    let pricing_source = summary
        .pricing_model_source
        .as_deref()
        .unwrap_or("upstream")
        .trim()
        .to_ascii_lowercase();
    let requested_model = summary.requested_model.as_deref();
    let candidates = if matches!(pricing_source.as_str(), "request" | "requested") {
        [requested_model, Some(upstream_model)]
    } else {
        [Some(upstream_model), requested_model]
    };
    candidates
        .into_iter()
        .flatten()
        .find_map(|model| find_model_pricing(conn, model))
}

fn query_model_pricing_exact(conn: &Connection, model_id: &str) -> Option<ModelPricing> {
    conn.query_row(
        "SELECT input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
         FROM model_pricing
         WHERE LOWER(model_id) = LOWER(?1)
         LIMIT 1",
        [model_id],
        row_to_model_pricing,
    )
    .optional()
    .ok()
    .flatten()
}

fn query_model_pricing_prefix(conn: &Connection, model_id: &str) -> Option<ModelPricing> {
    let like_pattern = format!("{}-%", model_id.to_ascii_lowercase());
    conn.query_row(
        "SELECT input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
         FROM model_pricing
         WHERE LOWER(model_id) LIKE ?1
         ORDER BY LENGTH(model_id) ASC
         LIMIT 1",
        [like_pattern],
        row_to_model_pricing,
    )
    .optional()
    .ok()
    .flatten()
}

fn row_to_model_pricing(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelPricing> {
    Ok(ModelPricing {
        input_cost_per_million: parse_decimal_or_default(&row.get::<_, String>(0)?, Decimal::ZERO),
        output_cost_per_million: parse_decimal_or_default(&row.get::<_, String>(1)?, Decimal::ZERO),
        cache_read_cost_per_million: parse_decimal_or_default(
            &row.get::<_, String>(2)?,
            Decimal::ZERO,
        ),
        cache_creation_cost_per_million: parse_decimal_or_default(
            &row.get::<_, String>(3)?,
            Decimal::ZERO,
        ),
    })
}

fn model_pricing_candidates(model_id: &str) -> Vec<String> {
    pricing_candidates_with_short_dates(model_id, true)
}

fn pricing_candidates_with_short_dates(model_id: &str, include_short_dates: bool) -> Vec<String> {
    let cleaned = clean_model_id_for_pricing(model_id);
    if is_placeholder_pricing_model(&cleaned) {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    let mut queue = vec![cleaned];

    while let Some(candidate) = queue.pop() {
        if !push_candidate(&mut candidates, candidate.clone()) {
            continue;
        }

        if let Some(stripped) = strip_known_model_namespace(&candidate) {
            queue.push(stripped);
        }
        if let Some(stripped) = strip_claude_desktop_non_anthropic_prefix(&candidate) {
            queue.push(stripped);
        }
        if let Some(stripped) = strip_bedrock_model_version_suffix(&candidate) {
            queue.push(stripped);
        }
        if let Some(stripped) = strip_known_model_date_suffix(&candidate) {
            queue.push(stripped);
        }
        if include_short_dates {
            if let Some(stripped) = strip_month_day_suffix(&candidate) {
                queue.push(stripped);
            }
        }
        if let Some(stripped) = strip_reasoning_effort_suffix(&candidate) {
            queue.push(stripped);
        }
        if candidate.starts_with("claude-") && candidate.contains('.') {
            queue.push(candidate.replace('.', "-"));
        }
    }

    candidates
}

fn clean_model_id_for_pricing(model_id: &str) -> String {
    model_id
        .rsplit_once('/')
        .map_or(model_id, |(_, right)| right)
        .split(':')
        .next()
        .unwrap_or(model_id)
        .trim()
        .replace('@', "-")
        .to_ascii_lowercase()
        .trim_end_matches(ONE_M_CONTEXT_MARKER)
        .trim()
        .to_string()
}

fn is_placeholder_pricing_model(model_id: &str) -> bool {
    model_id.trim().is_empty() || matches!(model_id.trim(), "unknown" | "null" | "none")
}

fn push_candidate(candidates: &mut Vec<String>, value: String) -> bool {
    if !value.is_empty() && !candidates.iter().any(|candidate| candidate == &value) {
        candidates.push(value);
        return true;
    }
    false
}

fn strip_known_model_namespace(model_id: &str) -> Option<String> {
    if let Some(position) = model_id.rfind("claude-") {
        if position > 0 {
            return Some(model_id[position..].to_string());
        }
    }

    for marker in [
        "openai.",
        "anthropic.",
        "google.",
        "moonshot.",
        "moonshotai.",
        "bedrock.",
        "global.",
    ] {
        if let Some(stripped) = model_id.strip_prefix(marker) {
            return Some(stripped.to_string());
        }
    }

    None
}

fn strip_claude_desktop_non_anthropic_prefix(model_id: &str) -> Option<String> {
    const NON_ANTHROPIC_MARKERS: &[&str] = &[
        "abab",
        "ark-code",
        "arctic",
        "astron",
        "codex",
        "command-r",
        "deepseek",
        "doubao",
        "ernie",
        "gemini",
        "gemma",
        "glm",
        "gpt",
        "grok",
        "hermes",
        "hy3",
        "hunyuan",
        "jamba",
        "kimi",
        "lfm",
        "llama",
        "longcat",
        "mercury",
        "mimo",
        "minimax",
        "mistral",
        "mixtral",
        "moonshot",
        "nemotron",
        "nova-",
        "openai",
        "qianfan",
        "qwen",
        "seed-",
        "solar",
        "stepfun",
    ];

    let rest = model_id.strip_prefix("claude-")?;
    NON_ANTHROPIC_MARKERS
        .iter()
        .any(|marker| rest.starts_with(marker))
        .then(|| rest.to_string())
}

fn strip_bedrock_model_version_suffix(model_id: &str) -> Option<String> {
    let (base, suffix) = model_id.rsplit_once("-v")?;
    (!base.is_empty() && !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| base.to_string())
}

fn strip_known_model_date_suffix(value: &str) -> Option<String> {
    if let Some(stripped) = strip_iso_date_suffix(value) {
        return Some(stripped);
    }
    if let Some(stripped) = strip_hyphenated_date_suffix(value) {
        return Some(stripped);
    }
    let parts = value.rsplit_once('-')?;
    let date = parts.1;
    if date.len() == 8 && date.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(parts.0.to_string());
    }
    None
}

fn strip_month_day_suffix(value: &str) -> Option<String> {
    let (model, date) = value.rsplit_once('-')?;
    if model.is_empty() || date.len() != 4 || !date.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let month = date[..2].parse().ok()?;
    let day = date[2..].parse().ok()?;
    chrono::NaiveDate::from_ymd_opt(2000, month, day).map(|_| model.to_string())
}

fn strip_iso_date_suffix(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() <= 11 {
        return None;
    }

    let start = bytes.len() - 11;
    let suffix = &bytes[start..];
    let is_iso_date = suffix[0] == b'-'
        && suffix[1..5].iter().all(|byte| byte.is_ascii_digit())
        && suffix[5] == b'-'
        && suffix[6..8].iter().all(|byte| byte.is_ascii_digit())
        && suffix[8] == b'-'
        && suffix[9..11].iter().all(|byte| byte.is_ascii_digit());
    is_iso_date.then(|| value[..start].to_string())
}

fn strip_hyphenated_date_suffix(value: &str) -> Option<String> {
    let parts = value.rsplitn(4, '-').collect::<Vec<_>>();
    if parts.len() < 4 {
        return None;
    }
    let day = parts[0];
    let month = parts[1];
    let year = parts[2];
    if year.len() == 4
        && month.len() == 2
        && day.len() == 2
        && year.chars().all(|ch| ch.is_ascii_digit())
        && month.chars().all(|ch| ch.is_ascii_digit())
        && day.chars().all(|ch| ch.is_ascii_digit())
    {
        return Some(parts[3].to_string());
    }
    None
}

fn strip_reasoning_effort_suffix(value: &str) -> Option<String> {
    for suffix in ["-minimal", "-low", "-medium", "-high", "-xhigh", "-max"] {
        if let Some(stripped) = value.strip_suffix(suffix) {
            if !stripped.is_empty() {
                return Some(stripped.to_string());
            }
        }
    }
    None
}

fn should_try_pricing_prefix_match(model_id: &str) -> bool {
    let dash_count = model_id.matches('-').count();

    if model_id.starts_with("claude-") {
        return dash_count >= 3;
    }

    if ["o1", "o3", "o4", "o5"]
        .iter()
        .any(|prefix| model_id.starts_with(prefix))
    {
        return dash_count >= 1;
    }

    const PREFIX_MATCH_FAMILIES: &[&str] = &[
        "gpt-",
        "gemini-",
        "deepseek-",
        "qwen-",
        "glm-",
        "kimi-",
        "minimax-",
    ];

    PREFIX_MATCH_FAMILIES
        .iter()
        .any(|prefix| model_id.starts_with(prefix))
        && dash_count >= 2
}

pub fn request_log_detail_from_summary(
    db: &SqliteDbState,
    trace_id: &str,
) -> Result<Option<GatewayRequestLogDetail>, String> {
    let trace_id = trace_id.trim();
    if trace_id.is_empty() {
        return Ok(None);
    }
    db.with_conn(|conn| {
        let provider_names = load_provider_names(conn)?;
        conn.query_row(
            "SELECT request_id, provider_id, app_type, model, request_model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    latency_ms, first_token_ms, COALESCE(duration_ms, latency_ms, 0), status_code, error_message,
                    created_at, is_streaming, total_cost_usd, provider_type,
                    cost_multiplier, pricing_model_source, detail_file, detail_offset,
                    route_name, method, path, upstream_status_code,
                    stream_outcome, error_category, attempt_count, total_attempt_count, reasoning_effort,
                    COALESCE(data_source, 'proxy'), json(usage_metadata), extra_tokens
             FROM proxy_request_logs
             WHERE request_id = ?1",
            [trace_id],
            |row| {
                let app_type: String = row.get(2)?;
                let provider_id: String = row.get(1)?;
                let cli_key = cli_key_from_app_type(&app_type);
                let input_tokens = row.get::<_, i64>(5)?.max(0) as u64;
                let output_tokens = row.get::<_, i64>(6)?.max(0) as u64;
                let cache_read_tokens = row.get::<_, i64>(7)?.max(0) as u64;
                let cache_creation_tokens = row.get::<_, i64>(8)?.max(0) as u64;
                let duration_ms = row.get::<_, i64>(11)?.max(0) as u64;
                let ended_at = timestamp_to_utc(row.get(14)?);
                let started_at = ended_at - Duration::milliseconds(duration_ms as i64);
                let status_code = row.get::<_, i64>(12)?.max(0) as u16;
                let is_session = row.get::<_, String>(31)? == "session";
                let stream_outcome = row
                    .get::<_, Option<String>>(26)?
                    .filter(|value| !value.trim().is_empty())
                    .and_then(|value| GatewayStreamOutcome::from_str(&value));
                let success = match stream_outcome {
                    Some(outcome) => outcome.is_success(),
                    None => is_success_status(status_code),
                };
                let total_tokens = input_tokens
                    .saturating_add(output_tokens)
                    .saturating_add(cache_read_tokens)
                    .saturating_add(cache_creation_tokens)
                    .saturating_add(row.get::<_, i64>(33)?.max(0) as u64);
                Ok(GatewayRequestLogDetail {
                    summary: GatewayRequestLogSummary {
                        usage_metadata: row.get::<_, Option<String>>(32)?.and_then(|value| serde_json::from_str(&value).ok()),
                        data_source: Some(row.get(31)?),
                        trace_id: row.get(0)?,
                        started_at,
                        ended_at,
                        cli_key,
                        route_name: row
                            .get::<_, Option<String>>(22)?
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or_else(|| app_type.clone()),
                        method: row
                            .get::<_, Option<String>>(23)?
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or_default(),
                        path: row
                            .get::<_, Option<String>>(24)?
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or_default(),
                        provider_id: Some(provider_id.clone()),
                        provider_name: provider_names.get(&(app_type, provider_id)).cloned(),
                        provider_type: row.get(17)?,
                        cost_multiplier: row.get(18)?,
                        pricing_model_source: row.get(19)?,
                        requested_model: row.get(4)?,
                        upstream_model_id: Some(row.get(3)?),
                        reasoning_effort: row.get(30)?,
                        upstream_url: None,
                        status_code: (!is_session).then_some(status_code),
                        upstream_status_code: row
                            .get::<_, Option<i64>>(25)?
                            .map(|value| value.max(0) as u16),
                        success,
                        error_category: row.get::<_, Option<String>>(27)?,
                        error_message: row.get(13)?,
                        stream_outcome,
                        duration_ms,
                        attempt_count: row
                            .get::<_, Option<i64>>(28)?
                            .map(|value| value.max(0) as u32)
                            .unwrap_or(1),
                        total_attempt_count: row
                            .get::<_, Option<i64>>(29)?
                            .map(|value| value.max(0) as u32)
                            .unwrap_or(1),
                        failover: false,
                        input_tokens: Some(input_tokens),
                        output_tokens: Some(output_tokens),
                        cache_read_tokens: Some(cache_read_tokens),
                        cache_creation_tokens: Some(cache_creation_tokens),
                        total_tokens: Some(total_tokens),
                        request_body_bytes: 0,
                        response_body_bytes: 0,
                        is_streaming: row.get::<_, i64>(15)? != 0,
                        first_token_ms: row
                            .get::<_, Option<i64>>(10)?
                            .map(|value| value.max(0) as u64),
                        detail_file: row.get(20)?,
                        detail_offset: row
                            .get::<_, Option<i64>>(21)?
                            .map(|value| value.max(0) as u64),
                    },
                    request_headers: None,
                    request_body: None,
                    upstream_request_body: None,
                    response_headers: None,
                    upstream_response_body: None,
                    response_body: None,
                    provider_attempts: Vec::new(),
                })
            },
        )
        .optional()
        .map_err(|error| format!("Failed to load gateway request summary detail: {error}"))
    })
}

pub fn request_exists(conn: &Connection, request_id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM proxy_request_logs WHERE request_id = ?1)",
        [request_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|value| value.unwrap_or(0) != 0)
    .map_err(|error| format!("Failed to check gateway request log existence: {error}"))
}

pub fn request_log_location(
    db: &SqliteDbState,
    trace_id: &str,
) -> Result<Option<(String, u64)>, String> {
    let trace_id = trace_id.trim();
    if trace_id.is_empty() {
        return Ok(None);
    }
    db.with_conn(|conn| {
        conn.query_row(
            "SELECT detail_file, detail_offset
             FROM proxy_request_logs
             WHERE request_id = ?1
             LIMIT 1",
            [trace_id],
            |row| {
                let detail_file = row.get::<_, Option<String>>(0)?;
                let detail_offset = row.get::<_, Option<i64>>(1)?;
                Ok(match (detail_file, detail_offset) {
                    (Some(detail_file), Some(detail_offset)) if detail_offset >= 0 => {
                        Some((detail_file, detail_offset as u64))
                    }
                    _ => None,
                })
            },
        )
        .optional()
        .map(|value| value.flatten())
        .map_err(|error| format!("Failed to load gateway request detail location: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::proxy_gateway::types::GatewayCliKey;
    use crate::coding::proxy_gateway::types::{DataSourceBreakdownInput, GatewayRequestLogSummary};
    use crate::db::helpers::db_put;
    use crate::db::schema::DbTable;
    use rusqlite::params;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn test_db() -> SqliteDbState {
        SqliteDbState::in_memory_for_test().expect("sqlite")
    }

    fn insert_provider(db: &SqliteDbState, id: &str, name: &str) {
        insert_provider_for_cli(db, GatewayCliKey::Claude, id, name);
    }

    fn insert_provider_for_cli(db: &SqliteDbState, cli_key: GatewayCliKey, id: &str, name: &str) {
        let table = match cli_key {
            GatewayCliKey::Claude => DbTable::ClaudeProvider,
            GatewayCliKey::ClaudeDesktop => DbTable::ClaudeDesktopProvider,
            GatewayCliKey::Codex => DbTable::CodexProvider,
            GatewayCliKey::Grok => DbTable::GrokProvider,
            GatewayCliKey::Kimi => DbTable::KimiProvider,
            GatewayCliKey::Gemini => DbTable::GeminiCliProvider,
            GatewayCliKey::OpenCode => {
                panic!("OpenCode provider insertion is not used by usage_stats unit tests")
            }
        };
        db.with_conn(|conn| {
            db_put(
                conn,
                table,
                id,
                &json!({
                    "name": name,
                    "is_applied": true,
                    "sort_index": 0,
                }),
            )
            .map(|_| ())
        })
        .expect("insert provider");
    }

    fn insert_rollup(db: &SqliteDbState) {
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model, request_count, success_count,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    total_cost_usd, avg_latency_ms
                ) VALUES (
                    '2026-05-18', 'claude', 'provider-alpha', 'anthropic/claude-sonnet-4-5',
                    4, 3, 40, 12, 5, 2, '0.250000', 300
                )",
                [],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
        })
        .expect("insert rollup");
    }

    fn set_request_data_source(db: &SqliteDbState, request_id: &str, data_source: &str) {
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE proxy_request_logs SET data_source = ?1 WHERE request_id = ?2",
                rusqlite::params![data_source, request_id],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
        })
        .expect("set data source");
    }

    fn insert_model_pricing(
        db: &SqliteDbState,
        model_id: &str,
        input_cost: &str,
        output_cost: &str,
    ) {
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO model_pricing (
                    model_id, display_name, input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million
                ) VALUES (?1, ?1, ?2, ?3, '0', '0')
                ON CONFLICT(model_id) DO UPDATE SET
                    input_cost_per_million = excluded.input_cost_per_million,
                    output_cost_per_million = excluded.output_cost_per_million,
                    cache_read_cost_per_million = excluded.cache_read_cost_per_million,
                    cache_creation_cost_per_million = excluded.cache_creation_cost_per_million",
                params![model_id, input_cost, output_cost],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
        })
        .expect("insert model pricing");
    }

    fn make_detail(
        trace_id: &str,
        provider_id: &str,
        status_code: u16,
        input_tokens: u64,
        output_tokens: u64,
    ) -> GatewayRequestLogDetail {
        make_detail_for_cli(
            GatewayCliKey::Claude,
            trace_id,
            provider_id,
            status_code,
            input_tokens,
            output_tokens,
        )
    }

    fn make_detail_for_cli(
        cli_key: GatewayCliKey,
        trace_id: &str,
        provider_id: &str,
        status_code: u16,
        input_tokens: u64,
        output_tokens: u64,
    ) -> GatewayRequestLogDetail {
        let ended_at = Utc.with_ymd_and_hms(2026, 5, 20, 8, 30, 0).unwrap();
        let mut request_headers = BTreeMap::new();
        request_headers.insert("authorization".to_string(), "Bearer redacted".to_string());
        let (route_name, path) = match cli_key {
            GatewayCliKey::Claude => ("claude_messages", "/v1/messages"),
            GatewayCliKey::ClaudeDesktop => ("claude_desktop_messages", "/v1/messages"),
            GatewayCliKey::Codex => ("codex_responses", "/v1/responses"),
            GatewayCliKey::Grok => ("grok_responses", "/v1/responses"),
            GatewayCliKey::Kimi => ("kimi_chat", "/v1/chat/completions"),
            GatewayCliKey::Gemini => ("gemini_generate", "/v1beta/models/gemini:generateContent"),
            GatewayCliKey::OpenCode => ("opencode", "/v1/chat/completions"),
        };
        GatewayRequestLogDetail {
            summary: GatewayRequestLogSummary {
                usage_metadata: None,
                data_source: None,
                trace_id: trace_id.to_string(),
                started_at: ended_at - Duration::milliseconds(1200),
                ended_at,
                cli_key: Some(cli_key.into()),
                route_name: route_name.to_string(),
                method: "POST".to_string(),
                path: path.to_string(),
                provider_id: Some(provider_id.to_string()),
                provider_name: Some("Runtime Name".to_string()),
                provider_type: None,
                cost_multiplier: None,
                pricing_model_source: None,
                requested_model: Some("claude-sonnet-4-5".to_string()),
                upstream_model_id: Some("anthropic/claude-sonnet-4-5".to_string()),
                reasoning_effort: None,
                upstream_url: Some("https://example.test/v1/messages".to_string()),
                status_code: Some(status_code),
                upstream_status_code: None,
                success: (200..400).contains(&status_code),
                error_category: None,
                error_message: None,
                stream_outcome: None,
                duration_ms: 1200,
                attempt_count: 2,
                total_attempt_count: 3,
                failover: true,
                input_tokens: Some(input_tokens),
                output_tokens: Some(output_tokens),
                cache_read_tokens: Some(0),
                cache_creation_tokens: Some(0),
                total_tokens: Some(input_tokens + output_tokens),
                request_body_bytes: 512,
                response_body_bytes: 1024,
                is_streaming: false,
                first_token_ms: None,
                detail_file: None,
                detail_offset: None,
            },
            request_headers: Some(request_headers),
            request_body: Some(
                "{\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}".to_string(),
            ),
            upstream_request_body: Some(
                "{\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}".to_string(),
            ),
            response_headers: Some(BTreeMap::from([(
                "content-type".to_string(),
                "application/json".to_string(),
            )])),
            upstream_response_body: Some("{\"id\":\"upstream_msg_1\"}".to_string()),
            response_body: Some("{\"id\":\"msg_1\"}".to_string()),
            provider_attempts: Vec::new(),
        }
    }

    fn make_no_model_detail(
        trace_id: &str,
        provider_id: &str,
        method: &str,
        path: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> GatewayRequestLogDetail {
        let mut detail = make_detail(trace_id, provider_id, 200, input_tokens, output_tokens);
        detail.summary.route_name = "openai-compatible".to_string();
        detail.summary.method = method.to_string();
        detail.summary.path = path.to_string();
        detail.summary.requested_model = None;
        detail.summary.upstream_model_id = None;
        detail.summary.total_tokens = Some(input_tokens + output_tokens);
        detail
    }

    #[test]
    fn record_request_summary_stores_only_compact_fields() {
        let db = test_db();
        insert_provider(&db, "provider-alpha", "Alpha Provider");
        let mut detail = make_detail("trace-summary", "provider-alpha", 200, 10, 20);
        detail.summary.reasoning_effort = Some("high".to_string());

        record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
            .expect("record summary");

        let column_names = db
            .with_conn(|conn| {
                let mut stmt = conn
                    .prepare("PRAGMA table_info(proxy_request_logs)")
                    .map_err(|error| error.to_string())?;
                let rows = stmt
                    .query_map([], |row| row.get::<_, String>(1))
                    .map_err(|error| error.to_string())?;
                let mut names = Vec::new();
                for row in rows {
                    names.push(row.map_err(|error| error.to_string())?);
                }
                Ok(names)
            })
            .expect("column names");
        for detail_column in [
            "request_headers",
            "request_body",
            "upstream_request_body",
            "response_headers",
            "upstream_response_body",
            "response_body",
            "request_body_bytes",
            "response_body_bytes",
        ] {
            assert!(
                !column_names.iter().any(|name| name == detail_column),
                "{detail_column} should remain file-only"
            );
        }

        let logs = request_logs(
            &db,
            &GatewayRequestLogFilters {
                cli_key: Some(GatewayCliKey::Claude.into()),
                provider_name: Some("Alpha".to_string()),
                model: Some("sonnet".to_string()),
                status_code: Some(200),
                ..GatewayRequestLogFilters::default()
            },
            0,
            10,
        )
        .expect("request logs");

        assert_eq!(logs.total, 1);
        assert_eq!(logs.data.len(), 1);
        assert_eq!(
            logs.data[0].provider_name.as_deref(),
            Some("Alpha Provider")
        );
        assert_eq!(logs.data[0].provider_id, "provider-alpha");
        assert_eq!(logs.data[0].total_tokens, 30);
        assert_eq!(logs.data[0].reasoning_effort.as_deref(), Some("high"));
        assert!(logs.data[0].success);
        assert_eq!(logs.data[0].route_name.as_deref(), Some("claude_messages"));
        assert_eq!(logs.data[0].method.as_deref(), Some("POST"));
        assert_eq!(logs.data[0].path.as_deref(), Some("/v1/messages"));
    }

    #[test]
    fn record_request_summary_redacts_request_path_query() {
        let db = test_db();
        let mut detail = make_detail("trace-redact-path", "provider-alpha", 200, 0, 0);
        detail.summary.method = "GET".to_string();
        detail.summary.path =
            "/v1beta/models?key=secret&client_version=0.1&access_token=token&api%5Fkey=encoded&api-key=hyphen&client-secret=clientSecretValue"
                .to_string();

        record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
            .expect("record summary");

        let path = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT path FROM proxy_request_logs WHERE request_id = 'trace-redact-path'",
                    [],
                    |row| row.get::<_, Option<String>>(0),
                )
                .map_err(|error| error.to_string())
            })
            .expect("path")
            .expect("path should be stored");

        assert_eq!(
            path,
            "/v1beta/models?key=xxx&client_version=0.1&access_token=xxx&api%5Fkey=xxx&api-key=xxx&client-secret=xxx"
        );
        assert!(!path.contains("hyphen"));
        assert!(!path.contains("clientSecretValue"));
    }

    #[test]
    fn provider_filter_matches_visible_provider_id_without_name_match() {
        let db = test_db();
        let detail = make_detail("trace-provider-id", "provider-alpha", 200, 10, 20);

        record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
            .expect("record summary");

        let logs = request_logs(
            &db,
            &GatewayRequestLogFilters {
                provider_name: Some("provider-alpha".to_string()),
                ..GatewayRequestLogFilters::default()
            },
            0,
            10,
        )
        .expect("request logs");

        assert_eq!(logs.total, 1);
        assert_eq!(logs.data[0].provider_id, "provider-alpha");
    }

    #[test]
    fn request_logs_can_exclude_model_list_requests() {
        let db = test_db();
        let settings = ProxyGatewaySettings::default();

        let mut model_list = make_detail("trace-model-list", "provider-alpha", 200, 0, 0);
        model_list.summary.method = "GET".to_string();
        model_list.summary.path = "/v1/models".to_string();
        model_list.summary.requested_model = None;
        model_list.summary.upstream_model_id = None;
        record_request_summary(&db, &settings, &model_list).expect("record model list");

        let mut model_list_with_query =
            make_detail("trace-model-list-query", "provider-alpha", 200, 0, 0);
        model_list_with_query.summary.method = "GET".to_string();
        model_list_with_query.summary.path = "/v1beta/models?key=secret".to_string();
        model_list_with_query.summary.requested_model = None;
        model_list_with_query.summary.upstream_model_id = None;
        record_request_summary(&db, &settings, &model_list_with_query)
            .expect("record model list with query");

        let mut list_models = make_detail("trace-list-models", "provider-alpha", 200, 0, 0);
        list_models.summary.method = "GET".to_string();
        list_models.summary.path = "/v1beta/models:listModels".to_string();
        list_models.summary.requested_model = None;
        list_models.summary.upstream_model_id = None;
        record_request_summary(&db, &settings, &list_models).expect("record listModels");

        let chat = make_detail("trace-chat", "provider-alpha", 200, 10, 20);
        record_request_summary(&db, &settings, &chat).expect("record chat");

        let all_logs = request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10)
            .expect("all request logs");
        assert_eq!(all_logs.total, 4);

        let filtered = request_logs(
            &db,
            &GatewayRequestLogFilters {
                exclude_model_list: Some(true),
                ..GatewayRequestLogFilters::default()
            },
            0,
            10,
        )
        .expect("filtered request logs");

        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.data.len(), 1);
        assert_eq!(filtered.data[0].trace_id, "trace-chat");
        assert_eq!(filtered.data[0].method.as_deref(), Some("POST"));
    }

    #[test]
    fn record_request_summary_persists_cache_tokens_and_calculates_cost() {
        let db = test_db();
        let mut detail = make_detail("trace-cache", "provider-alpha", 200, 1000, 500);
        detail.summary.cache_read_tokens = Some(200);
        detail.summary.cache_creation_tokens = Some(100);
        detail.summary.total_tokens = Some(1800);
        detail.summary.first_token_ms = Some(250);
        detail.summary.is_streaming = true;

        record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
            .expect("record summary");

        let row = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT cache_read_tokens, cache_creation_tokens, total_cost_usd,
                            latency_ms, first_token_ms, is_streaming
                     FROM proxy_request_logs
                     WHERE request_id = 'trace-cache'",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, Option<i64>>(4)?,
                            row.get::<_, i64>(5)?,
                        ))
                    },
                )
                .map_err(|error| error.to_string())
            })
            .expect("row");

        assert_eq!(row.0, 200);
        assert_eq!(row.1, 100);
        assert!(row.2.parse::<f64>().unwrap() > 0.0);
        assert_eq!(row.3, 250);
        assert_eq!(row.4, Some(250));
        assert_eq!(row.5, 1);
    }

    #[test]
    fn request_log_detail_falls_back_to_sqlite_summary() {
        let db = test_db();
        let mut detail = make_detail("trace-summary-detail", "provider-alpha", 200, 10, 20);
        detail.summary.cache_read_tokens = Some(3);
        detail.summary.cache_creation_tokens = Some(2);
        detail.summary.total_tokens = Some(35);
        detail.summary.cost_multiplier = Some("1.75".to_string());
        detail.summary.pricing_model_source = Some("requested".to_string());
        detail.summary.reasoning_effort = Some("xhigh".to_string());

        record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
            .expect("record summary");

        let fallback = request_log_detail_from_summary(&db, "trace-summary-detail")
            .expect("fallback detail")
            .expect("summary detail exists");

        assert_eq!(fallback.summary.trace_id, "trace-summary-detail");
        assert_eq!(fallback.summary.input_tokens, Some(10));
        assert_eq!(fallback.summary.output_tokens, Some(20));
        assert_eq!(fallback.summary.cache_read_tokens, Some(3));
        assert_eq!(fallback.summary.cache_creation_tokens, Some(2));
        assert_eq!(fallback.summary.total_tokens, Some(35));
        assert_eq!(fallback.summary.reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(fallback.summary.cost_multiplier.as_deref(), Some("1.75"));
        assert_eq!(
            fallback.summary.pricing_model_source.as_deref(),
            Some("requested")
        );
        assert_eq!(fallback.summary.route_name, "claude_messages");
        assert_eq!(fallback.summary.method, "POST");
        assert_eq!(fallback.summary.path, "/v1/messages");
        assert!(fallback.request_body.is_none());
        assert!(fallback.response_body.is_none());
    }

    #[test]
    fn request_log_detail_summary_fallback_preserves_missing_method() {
        let db = test_db();
        let mut detail = make_detail("trace-summary-missing-method", "provider-alpha", 200, 0, 0);
        detail.summary.method = String::new();
        detail.summary.path = "/openai/v1/models".to_string();

        record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
            .expect("record summary");

        let fallback = request_log_detail_from_summary(&db, "trace-summary-missing-method")
            .expect("fallback detail")
            .expect("summary detail exists");

        assert_eq!(fallback.summary.method, "");
        assert_eq!(fallback.summary.path, "/openai/v1/models");
    }

    #[test]
    fn record_request_summary_skips_identical_proxy_semantic_replay() {
        let db = test_db();
        let detail = make_detail("SESSION:msg_stable", "provider-alpha", 200, 11, 7);

        assert!(matches!(
            record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
                .expect("first write"),
            RecordRequestSummaryOutcome::Written { .. }
        ));
        // With an attached detail locator, identical proxy semantics must skip.
        update_request_detail_locator(
            &db,
            "SESSION:msg_stable",
            Some("stable-detail.jsonl"),
            Some(1),
        )
        .expect("attach locator");
        assert_eq!(
            record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
                .expect("identical replay"),
            RecordRequestSummaryOutcome::Skipped
        );

        let count: i64 = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM proxy_request_logs WHERE request_id = 'SESSION:msg_stable'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())
            })
            .expect("count");
        assert_eq!(count, 1);
    }

    #[test]
    fn record_request_summary_uses_collision_fallback_for_different_semantics() {
        let db = test_db();
        let first = make_detail("SESSION:codex:p1:resp_1", "provider-alpha", 200, 10, 1);
        let mut second = make_detail("SESSION:codex:p1:resp_1", "provider-alpha", 200, 99, 5);
        second.summary.upstream_model_id = Some("other-model".to_string());

        assert!(matches!(
            record_request_summary(&db, &ProxyGatewaySettings::default(), &first)
                .expect("first write"),
            RecordRequestSummaryOutcome::Written {
                request_id,
                collision: false
            } if request_id == "SESSION:codex:p1:resp_1"
        ));
        let second_outcome = record_request_summary(&db, &ProxyGatewaySettings::default(), &second)
            .expect("collision fallback write");
        match second_outcome {
            RecordRequestSummaryOutcome::Written {
                request_id,
                collision,
            } => {
                assert!(collision);
                assert!(request_id.starts_with("SESSION:codex:p1:resp_1:collision:"));
            }
            RecordRequestSummaryOutcome::Skipped
            | RecordRequestSummaryOutcome::NeedsDetail { .. } => {
                panic!("expected collision write")
            }
        }

        let ids = db
            .with_conn(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT request_id FROM proxy_request_logs
                         WHERE request_id = 'SESSION:codex:p1:resp_1'
                            OR request_id LIKE 'SESSION:codex:p1:resp_1:collision:%'
                         ORDER BY request_id",
                    )
                    .map_err(|error| error.to_string())?;
                let rows = stmt
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|error| error.to_string())?;
                let mut values = Vec::new();
                for row in rows {
                    values.push(row.map_err(|error| error.to_string())?);
                }
                Ok(values)
            })
            .expect("ids");
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], "SESSION:codex:p1:resp_1");
        assert!(ids[1].starts_with("SESSION:codex:p1:resp_1:collision:"));
    }

    #[test]
    fn record_request_summary_upgrades_session_import_row_and_keeps_existing_detail() {
        let db = test_db();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model, request_model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                    total_cost_usd, latency_ms, status_code, created_at, data_source,
                    detail_file, detail_offset, session_id
                ) VALUES (
                    'SESSION:msg_upgrade', 'session', 'claude', 'claude-sonnet', NULL,
                    1, 1, 0, 0, '0', '0', '0', '0', '0', 0, 200,
                    CAST(strftime('%s', 'now') AS INTEGER), 'session',
                    'old-detail.jsonl', 42, 'imported-session-x'
                )",
                [],
            )
            .map_err(|error| error.to_string())
        })
        .expect("seed session row");

        let detail = make_detail("SESSION:msg_upgrade", "provider-alpha", 200, 10, 20);
        assert!(matches!(
            record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
                .expect("upgrade session row"),
            RecordRequestSummaryOutcome::Written { .. }
        ));

        let (provider_id, data_source, input_tokens, detail_file, detail_offset, session_id): (
            String,
            String,
            i64,
            Option<String>,
            Option<i64>,
            Option<String>,
        ) = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT provider_id, data_source, input_tokens, detail_file, detail_offset, session_id
                     FROM proxy_request_logs WHERE request_id = 'SESSION:msg_upgrade'",
                    [],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )
                .map_err(|error| error.to_string())
            })
            .expect("upgraded row");
        assert_eq!(provider_id, "provider-alpha");
        assert_eq!(data_source, "proxy");
        assert_eq!(input_tokens, 10);
        assert_eq!(detail_file.as_deref(), Some("old-detail.jsonl"));
        assert_eq!(detail_offset, Some(42));
        assert_eq!(session_id.as_deref(), Some("imported-session-x"));
    }

    #[test]
    fn record_request_summary_allows_detail_retry_when_locator_missing() {
        let db = test_db();
        let detail = make_detail("SESSION:msg_half_write", "provider-alpha", 200, 10, 20);
        assert!(matches!(
            record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
                .expect("first summary write"),
            RecordRequestSummaryOutcome::Written {
                request_id,
                collision: false
            } if request_id == "SESSION:msg_half_write"
        ));

        // First write left summary without detail locator (detail half-write failure).
        let first_replay = record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
            .expect("detail retry plan");
        assert!(matches!(
            first_replay,
            RecordRequestSummaryOutcome::NeedsDetail {
                request_id
            } if request_id == "SESSION:msg_half_write"
        ));

        // After a locator is attached, identical semantics must skip again.
        update_request_detail_locator(
            &db,
            "SESSION:msg_half_write",
            Some("retry-detail.jsonl"),
            Some(7),
        )
        .expect("attach locator");
        assert!(matches!(
            record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
                .expect("skip after detail attached"),
            RecordRequestSummaryOutcome::Skipped
        ));

        let count: i64 = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM proxy_request_logs WHERE request_id = 'SESSION:msg_half_write'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())
            })
            .expect("count");
        assert_eq!(count, 1);
    }

    #[test]
    fn record_request_summary_retries_when_detail_locator_is_partial() {
        for (request_id, detail_file, detail_offset) in [
            ("SESSION:msg_file_only", Some("partial.jsonl"), None),
            ("SESSION:msg_offset_only", None, Some(9)),
        ] {
            let db = test_db();
            let detail = make_detail(request_id, "provider-alpha", 200, 10, 20);
            assert!(matches!(
                record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
                    .expect("first summary write"),
                RecordRequestSummaryOutcome::Written { .. }
            ));
            update_request_detail_locator(&db, request_id, detail_file, detail_offset)
                .expect("attach partial locator");

            assert!(matches!(
                record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
                    .expect("partial locator retry plan"),
                RecordRequestSummaryOutcome::NeedsDetail {
                    request_id: retry_request_id
                } if retry_request_id == request_id
            ));
        }
    }

    #[test]
    fn record_request_summary_uses_requested_pricing_model_source() {
        let db = test_db();
        insert_model_pricing(&db, "pricing-request-model", "1", "2");
        insert_model_pricing(&db, "pricing-upstream-model", "100", "200");
        let mut detail = make_detail(
            "trace-request-pricing",
            "provider-alpha",
            200,
            1_000_000,
            500_000,
        );
        detail.summary.requested_model = Some("pricing-request-model".to_string());
        detail.summary.upstream_model_id = Some("pricing-upstream-model".to_string());
        detail.summary.pricing_model_source = Some("requested".to_string());
        detail.summary.cost_multiplier = Some("2".to_string());

        record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
            .expect("record summary");

        let row = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT total_cost_usd, pricing_model_source
                     FROM proxy_request_logs
                     WHERE request_id = 'trace-request-pricing'",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .map_err(|error| error.to_string())
            })
            .expect("request pricing row");

        assert_eq!(row.0, "4.000000");
        assert_eq!(row.1, "requested");
    }

    #[test]
    fn load_provider_names_resolves_grok_provider_display_name() {
        let db = test_db();
        insert_provider_for_cli(
            &db,
            GatewayCliKey::Grok,
            "provider-grok",
            "Grok Display Name",
        );
        record_request_summary(
            &db,
            &ProxyGatewaySettings::default(),
            &make_detail_for_cli(
                GatewayCliKey::Grok,
                "trace-grok",
                "provider-grok",
                200,
                9,
                4,
            ),
        )
        .expect("record grok summary");

        let provider_rows = provider_stats(&db, None, None, Some(GatewayCliKey::Grok.into()))
            .expect("provider stats");
        assert_eq!(provider_rows.len(), 1);
        assert_eq!(provider_rows[0].provider_id, "provider-grok");
        assert_eq!(
            provider_rows[0].provider_name.as_deref(),
            Some("Grok Display Name"),
            "Grok usage rows must resolve display names from grok_provider"
        );
        assert_eq!(provider_rows[0].request_count, 1);
        assert_eq!(provider_rows[0].total_tokens, 13);
    }

    #[test]
    fn kimi_usage_rows_appear_in_request_logs_and_resolve_provider_name() {
        let db = test_db();
        insert_provider_for_cli(&db, GatewayCliKey::Kimi, "provider-kimi", "AxonHub Kimi");
        record_request_summary(
            &db,
            &ProxyGatewaySettings::default(),
            &make_detail_for_cli(
                GatewayCliKey::Kimi,
                "trace-kimi",
                "provider-kimi",
                200,
                9,
                4,
            ),
        )
        .expect("record kimi summary");

        let logs =
            request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10).expect("request logs");
        assert_eq!(logs.total, 1);
        assert_eq!(logs.data.len(), 1);
        assert_eq!(logs.data[0].cli_key, GatewayUsageTool::Kimi);
        assert_eq!(
            logs.data[0].provider_name.as_deref(),
            Some("AxonHub Kimi"),
            "Kimi request rows must not be dropped and must resolve display names from kimi_provider"
        );

        let provider_rows = provider_stats(&db, None, None, Some(GatewayCliKey::Kimi.into()))
            .expect("provider stats");
        assert_eq!(provider_rows.len(), 1);
        assert_eq!(
            provider_rows[0].provider_name.as_deref(),
            Some("AxonHub Kimi")
        );
    }

    #[test]
    fn usage_summary_and_stats_read_recorded_summaries() {
        let db = test_db();
        insert_provider(&db, "provider-alpha", "Alpha Provider");
        record_request_summary(
            &db,
            &ProxyGatewaySettings::default(),
            &make_detail("trace-success", "provider-alpha", 200, 11, 5),
        )
        .expect("record success");
        record_request_summary(
            &db,
            &ProxyGatewaySettings::default(),
            &make_detail("trace-error", "provider-alpha", 500, 7, 3),
        )
        .expect("record error");

        let summary =
            usage_summary(&db, None, None, Some(GatewayCliKey::Claude.into())).expect("summary");
        assert_eq!(summary.total_requests, 2);
        assert_eq!(summary.total_input_tokens, 18);
        assert_eq!(summary.total_output_tokens, 8);
        assert_eq!(summary.total_tokens, 26);
        assert_eq!(summary.success_rate, 50.0);

        let provider_rows = provider_stats(&db, None, None, Some(GatewayCliKey::Claude.into()))
            .expect("provider stats");
        assert_eq!(provider_rows.len(), 1);
        assert_eq!(
            provider_rows[0].provider_name.as_deref(),
            Some("Alpha Provider")
        );
        assert_eq!(provider_rows[0].request_count, 2);
        assert_eq!(provider_rows[0].success_rate, 50.0);

        let model_rows =
            model_stats(&db, None, None, Some(GatewayCliKey::Claude.into())).expect("model stats");
        assert_eq!(model_rows.len(), 1);
        assert_eq!(model_rows[0].request_count, 2);
        assert_eq!(model_rows[0].total_tokens, 26);
    }

    #[test]
    fn usage_stats_exclude_no_usage_requests_but_include_compact() {
        let db = test_db();
        insert_provider(&db, "provider-alpha", "Alpha Provider");
        record_request_summary(
            &db,
            &ProxyGatewaySettings::default(),
            &make_detail("trace-model", "provider-alpha", 200, 10, 5),
        )
        .expect("record model request");
        record_request_summary(
            &db,
            &ProxyGatewaySettings::default(),
            &make_no_model_detail(
                "trace-model-list",
                "provider-alpha",
                "GET",
                "/openai/v1/models",
                99,
                88,
            ),
        )
        .expect("record model list request");
        record_request_summary(
            &db,
            &ProxyGatewaySettings::default(),
            &make_no_model_detail(
                "trace-probe",
                "provider-alpha",
                "HEAD",
                "/gemini/v1beta",
                77,
                66,
            ),
        )
        .expect("record connection probe");
        record_request_summary(
            &db,
            &ProxyGatewaySettings::default(),
            &make_no_model_detail(
                "trace-generic",
                "provider-alpha",
                "POST",
                "/openai/v1/embeddings",
                55,
                44,
            ),
        )
        .expect("record generic no-model request");
        let mut compact_detail = make_no_model_detail(
            "trace-compact",
            "provider-alpha",
            "POST",
            "/openai/v1/responses/compact",
            3,
            2,
        );
        compact_detail.summary.requested_model = Some("gpt-5-mini".to_string());
        compact_detail.summary.upstream_model_id = Some("openai/gpt-5-mini".to_string());
        record_request_summary(&db, &ProxyGatewaySettings::default(), &compact_detail)
            .expect("record compact request");
        record_request_summary(
            &db,
            &ProxyGatewaySettings::default(),
            &make_no_model_detail(
                "trace-compact-no-model",
                "provider-alpha",
                "POST",
                "/openai/v1/responses/compact",
                4,
                1,
            ),
        )
        .expect("record no-model compact request");

        let logs =
            request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10).expect("request logs");
        assert_eq!(logs.total, 6);

        let summary =
            usage_summary(&db, None, None, Some(GatewayCliKey::Claude.into())).expect("summary");
        assert_eq!(summary.total_requests, 3);
        assert_eq!(summary.total_input_tokens, 17);
        assert_eq!(summary.total_output_tokens, 8);
        assert_eq!(summary.total_tokens, 25);
        assert_eq!(summary.success_rate, 100.0);

        let provider_rows = provider_stats(&db, None, None, Some(GatewayCliKey::Claude.into()))
            .expect("provider stats");
        assert_eq!(provider_rows.len(), 1);
        assert_eq!(provider_rows[0].provider_id, "provider-alpha");
        assert_eq!(provider_rows[0].request_count, 3);
        assert_eq!(provider_rows[0].total_tokens, 25);

        let model_rows =
            model_stats(&db, None, None, Some(GatewayCliKey::Claude.into())).expect("model stats");
        let model_map = model_rows
            .iter()
            .map(|item| (item.model.as_str(), item))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(model_map.len(), 2);
        assert_eq!(
            model_map
                .get("anthropic/claude-sonnet-4-5")
                .expect("normal model")
                .request_count,
            1
        );
        assert_eq!(
            model_map
                .get("anthropic/claude-sonnet-4-5")
                .expect("normal model")
                .total_tokens,
            15
        );
        assert_eq!(
            model_map
                .get("openai/gpt-5-mini")
                .expect("compact model")
                .request_count,
            1
        );
        assert_eq!(
            model_map
                .get("openai/gpt-5-mini")
                .expect("compact model")
                .total_tokens,
            5
        );
        assert!(!model_map.contains_key("unknown"));
        assert!(!model_map.contains_key(COMPACT_ROLLUP_MODEL));

        let start = Utc
            .with_ymd_and_hms(2026, 5, 20, 0, 0, 0)
            .unwrap()
            .timestamp();
        let end = Utc
            .with_ymd_and_hms(2026, 5, 20, 23, 59, 59)
            .unwrap()
            .timestamp();
        let trend_rows = usage_trends(
            &db,
            Some(start),
            Some(end),
            Some(GatewayCliKey::Claude.into()),
        )
        .expect("trends");
        assert_eq!(trend_rows.len(), 1);
        assert_eq!(trend_rows[0].request_count, 3);
        assert_eq!(trend_rows[0].total_tokens, 25);
    }

    #[test]
    fn usage_rollups_count_compact_sentinel_but_hide_it_from_model_stats() {
        let db = test_db();
        insert_provider(&db, "provider-alpha", "Alpha Provider");
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model, request_count, success_count,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    total_cost_usd, avg_latency_ms
                ) VALUES
                ('2026-05-18', 'claude', 'provider-alpha', 'anthropic/claude-sonnet-4-5',
                    2, 2, 20, 8, 1, 0, '0.100000', 200),
                ('2026-05-18', 'claude', 'provider-alpha', 'openai/gpt-5-mini',
                    1, 1, 3, 2, 0, 0, '0.020000', 120),
                ('2026-05-18', 'claude', 'provider-alpha', '__context_compact__',
                    1, 1, 3, 2, 0, 0, '0.010000', 100),
                ('2026-05-18', 'claude', 'provider-alpha', 'unknown',
                    9, 9, 900, 900, 0, 0, '9.000000', 50)",
                [],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
        })
        .expect("insert rollups");

        let start = Utc
            .with_ymd_and_hms(2026, 5, 18, 0, 0, 0)
            .unwrap()
            .timestamp();
        let end = Utc
            .with_ymd_and_hms(2026, 5, 18, 23, 59, 59)
            .unwrap()
            .timestamp();

        let summary = usage_summary(
            &db,
            Some(start),
            Some(end),
            Some(GatewayCliKey::Claude.into()),
        )
        .expect("summary");
        assert_eq!(summary.total_requests, 4);
        assert_eq!(summary.total_tokens, 39);
        assert_eq!(summary.success_rate, 100.0);

        let provider_rows = provider_stats(
            &db,
            Some(start),
            Some(end),
            Some(GatewayCliKey::Claude.into()),
        )
        .expect("provider stats");
        assert_eq!(provider_rows.len(), 1);
        assert_eq!(provider_rows[0].request_count, 4);
        assert_eq!(provider_rows[0].total_tokens, 39);

        let model_rows = model_stats(
            &db,
            Some(start),
            Some(end),
            Some(GatewayCliKey::Claude.into()),
        )
        .expect("model stats");
        let model_map = model_rows
            .iter()
            .map(|item| (item.model.as_str(), item))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(model_map.len(), 2);
        assert_eq!(
            model_map
                .get("anthropic/claude-sonnet-4-5")
                .expect("normal model")
                .request_count,
            2
        );
        assert_eq!(
            model_map
                .get("anthropic/claude-sonnet-4-5")
                .expect("normal model")
                .total_tokens,
            29
        );
        assert_eq!(
            model_map
                .get("openai/gpt-5-mini")
                .expect("compact model")
                .request_count,
            1
        );
        assert_eq!(
            model_map
                .get("openai/gpt-5-mini")
                .expect("compact model")
                .total_tokens,
            5
        );
        assert!(!model_map.contains_key("unknown"));
        assert!(!model_map.contains_key(COMPACT_ROLLUP_MODEL));

        let trend_rows = usage_trends(
            &db,
            Some(start),
            Some(end),
            Some(GatewayCliKey::Claude.into()),
        )
        .expect("trends");
        assert_eq!(trend_rows.len(), 1);
        assert_eq!(trend_rows[0].request_count, 4);
        assert_eq!(trend_rows[0].total_tokens, 39);
    }

    #[test]
    fn rollup_and_prune_preserves_valid_compact_model_and_hides_no_model_compact() {
        let db = test_db();
        let old_created_at = Utc::now().timestamp() - 40 * 24 * 60 * 60;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model, request_model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    total_cost_usd, latency_ms, status_code, created_at, method, path
                ) VALUES
                ('old-normal', 'provider-alpha', 'claude', 'anthropic/claude-sonnet-4-5', 'claude-sonnet-4-5',
                    10, 5, 0, 0, '0.100000', 200, 200, ?1, 'POST', '/v1/messages'),
                ('old-compact-model', 'provider-alpha', 'claude', 'openai/gpt-5-mini', 'gpt-5-mini',
                    3, 2, 0, 0, '0.020000', 120, 200, ?1, 'POST', '/openai/v1/responses/compact'),
                ('old-compact-no-model', 'provider-alpha', 'claude', 'unknown', NULL,
                    4, 1, 0, 0, '0.010000', 100, 200, ?1, 'POST', '/openai/v1/responses/compact'),
                ('old-model-list', 'provider-alpha', 'claude', 'unknown', NULL,
                    90, 80, 0, 0, '9.000000', 50, 200, ?1, 'GET', '/openai/v1/models')",
                [old_created_at],
            )
            .map_err(|error| error.to_string())?;

            rollup_and_prune(conn, 30)
        })
        .expect("roll up old compact logs");

        let rollup_models = db
            .with_conn(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT model, request_count,
                                input_tokens + output_tokens + cache_read_tokens + cache_creation_tokens AS total_tokens
                         FROM usage_daily_rollups
                         ORDER BY model",
                    )
                    .map_err(|error| error.to_string())?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)? as u64,
                            row.get::<_, i64>(2)? as u64,
                        ))
                    })
                    .map_err(|error| error.to_string())?;
                let mut items = Vec::new();
                for row in rows {
                    items.push(row.map_err(|error| error.to_string())?);
                }
                Ok(items)
            })
            .expect("rollup rows");
        assert_eq!(
            rollup_models,
            vec![
                ("__context_compact__".to_string(), 1, 5),
                ("anthropic/claude-sonnet-4-5".to_string(), 1, 15),
                ("openai/gpt-5-mini".to_string(), 1, 5),
            ]
        );

        let summary = usage_summary(&db, None, None, Some(GatewayCliKey::Claude.into()))
            .expect("summary from rollups");
        assert_eq!(summary.total_requests, 3);
        assert_eq!(summary.total_tokens, 25);

        let model_rows =
            model_stats(&db, None, None, Some(GatewayCliKey::Claude.into())).expect("model stats");
        let model_map = model_rows
            .iter()
            .map(|item| (item.model.as_str(), item))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(model_map.len(), 2);
        assert_eq!(
            model_map
                .get("anthropic/claude-sonnet-4-5")
                .expect("normal model")
                .total_tokens,
            15
        );
        assert_eq!(
            model_map
                .get("openai/gpt-5-mini")
                .expect("compact model")
                .total_tokens,
            5
        );
        assert!(!model_map.contains_key("unknown"));
        assert!(!model_map.contains_key(COMPACT_ROLLUP_MODEL));
    }

    #[test]
    fn model_pricing_matching_normalizes_common_provider_wrappers() {
        let db = test_db();

        db.with_conn(|conn| {
            assert!(find_model_pricing(conn, "anthropic/claude-opus-4.8").is_some());
            assert!(find_model_pricing(conn, "global.anthropic.claude-opus-4-8-v1:0").is_some());
            assert!(find_model_pricing(conn, "claude-opus-4-8@20260527").is_some());
            assert!(find_model_pricing(conn, "OpenAI/GPT-5.5@HIGH").is_some());
            assert!(find_model_pricing(conn, "claude-gpt-5.5").is_some());
            assert!(find_model_pricing(conn, "kimi-for-coding").is_none());
            Ok(())
        })
        .expect("pricing normalization assertions");
    }

    #[test]
    fn model_pricing_supports_month_day_releases_and_preserves_exact_prices() {
        let db = test_db();
        insert_model_pricing(&db, "deepseek-v4-flash", "0.14", "0.28");
        db.with_conn(|conn| {
            let pricing = find_model_pricing(conn, "provider/deepseek-v4-flash-0731-high").unwrap();
            assert_eq!(pricing.input_cost_per_million, Decimal::new(14, 2));
            assert!(session_model_needs_short_date_fallback(
                conn,
                "deepseek-v4-flash-0731"
            ));
            assert!(find_model_pricing(conn, "deepseek-v4-flash-1331").is_none());
            assert!(find_model_pricing(conn, "deepseek-v4-flash-0230").is_none());
            assert!(find_model_pricing(conn, "deepseek-v4-flash-8192").is_none());
            Ok(())
        })
        .unwrap();
        insert_model_pricing(&db, "deepseek-v4-flash-0731", "0", "0");
        db.with_conn(|conn| {
            assert_eq!(
                find_model_pricing(conn, "deepseek-v4-flash-0731")
                    .unwrap()
                    .input_cost_per_million,
                Decimal::ZERO
            );
            assert!(!session_model_needs_short_date_fallback(
                conn,
                "deepseek-v4-flash-0731"
            ));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn model_pricing_prefix_matching_does_not_promote_short_base_to_variant() {
        let db = test_db();

        db.with_conn(|conn| {
            conn.execute("DELETE FROM model_pricing WHERE model_id LIKE 'gpt-5%'", [])
                .map_err(|error| error.to_string())?;
            for (model_id, display_name) in
                [("gpt-5-mini", "GPT-5 Mini"), ("gpt-5-pro", "GPT-5 Pro")]
            {
                conn.execute(
                    "INSERT INTO model_pricing (
                        model_id, display_name, input_cost_per_million, output_cost_per_million,
                        cache_read_cost_per_million, cache_creation_cost_per_million
                    ) VALUES (?1, ?2, '1', '2', '0', '0')",
                    params![model_id, display_name],
                )
                .map_err(|error| error.to_string())?;
            }

            assert!(find_model_pricing(conn, "gpt-5").is_none());
            Ok(())
        })
        .expect("short base pricing should not match variants");
    }

    #[test]
    fn model_pricing_matching_preserves_max_as_part_of_canonical_model_id() {
        let db = test_db();
        insert_model_pricing(&db, "qwen3.7", "1", "2");
        insert_model_pricing(&db, "qwen3.7-max", "3", "4");

        db.with_conn(|conn| {
            let pricing = find_model_pricing(conn, "qwen3.7-max")
                .expect("canonical max model pricing should exist");
            assert_eq!(pricing.input_cost_per_million, Decimal::from(3));
            assert_eq!(pricing.output_cost_per_million, Decimal::from(4));
            Ok(())
        })
        .expect("canonical max model pricing should stay exact");
    }

    #[test]
    fn model_pricing_matching_falls_back_from_appended_max_effort() {
        let db = test_db();
        insert_model_pricing(&db, "gpt-5.6-sol", "5", "6");

        db.with_conn(|conn| {
            let pricing = find_model_pricing(conn, "gpt-5.6-sol-max")
                .expect("appended max effort should fall back to base model pricing");
            assert_eq!(pricing.input_cost_per_million, Decimal::from(5));
            assert_eq!(pricing.output_cost_per_million, Decimal::from(6));
            Ok(())
        })
        .expect("appended max effort pricing should use the base model");
    }

    #[test]
    fn data_source_breakdown_groups_proxy_and_filters_by_cli_and_time() {
        let db = test_db();
        for request_id in [
            "trace-proxy-one",
            "trace-proxy-two",
            "trace-session-one",
            "trace-session-two",
            "trace-session-three",
        ] {
            record_request_summary(
                &db,
                &ProxyGatewaySettings::default(),
                &make_detail(request_id, "provider-alpha", 200, 10, 2),
            )
            .expect("record claude summary");
        }

        let mut codex_detail = make_detail("trace-codex-one", "provider-beta", 200, 4, 1);
        codex_detail.summary.cli_key = Some(GatewayCliKey::Codex.into());
        record_request_summary(&db, &ProxyGatewaySettings::default(), &codex_detail)
            .expect("record codex summary");

        set_request_data_source(&db, "trace-proxy-one", "");
        set_request_data_source(&db, "trace-proxy-two", "   ");
        set_request_data_source(&db, "trace-session-one", "session");
        set_request_data_source(&db, "trace-session-two", "session");
        set_request_data_source(&db, "trace-session-three", "session");
        set_request_data_source(&db, "trace-codex-one", "session");

        let all_sources = data_source_breakdown(&db, DataSourceBreakdownInput::default())
            .expect("all data source breakdown");
        let all_rows: Vec<_> = all_sources
            .iter()
            .map(|item| (item.data_source.as_str(), item.request_count))
            .collect();
        assert_eq!(all_rows, vec![("session", 4), ("proxy", 2)]);

        let claude_sources = data_source_breakdown(
            &db,
            DataSourceBreakdownInput {
                cli_key: Some(GatewayCliKey::Claude.into()),
                ..DataSourceBreakdownInput::default()
            },
        )
        .expect("claude data source breakdown");
        let claude_rows: Vec<_> = claude_sources
            .iter()
            .map(|item| (item.data_source.as_str(), item.request_count))
            .collect();
        assert_eq!(claude_rows, vec![("session", 3), ("proxy", 2)]);

        let after_known_records = data_source_breakdown(
            &db,
            DataSourceBreakdownInput {
                start_unix_secs: Some(
                    Utc.with_ymd_and_hms(2026, 5, 20, 8, 30, 1)
                        .unwrap()
                        .timestamp(),
                ),
                ..DataSourceBreakdownInput::default()
            },
        )
        .expect("future data source breakdown");
        assert!(after_known_records.is_empty());
    }

    #[test]
    fn provider_cache_hit_rate_distinguishes_zero_missing_and_full_input_usage() {
        let db = test_db();
        let settings = ProxyGatewaySettings {
            log_retention_days: 0,
            ..Default::default()
        };
        for (provider_id, input, creation, read, expected) in [
            ("mixed", 100, 20, 80, Some(0.4)),
            ("no-cache", 100, 0, 0, Some(0.0)),
            ("no-input", 0, 0, 0, None),
            ("all-cache", 0, 0, 100, Some(1.0)),
        ] {
            let mut detail = make_detail(provider_id, provider_id, 200, input, 10_000);
            detail.summary.cache_creation_tokens = Some(creation);
            detail.summary.cache_read_tokens = Some(read);
            record_request_summary(&db, &settings, &detail).expect("record cache usage");
            let rows = provider_stats(&db, None, None, Some(GatewayCliKey::Claude.into())).unwrap();
            let row = rows
                .iter()
                .find(|row| row.provider_id == provider_id)
                .unwrap();
            assert_eq!(row.cache_hit_rate, expected, "{provider_id}");
        }
    }

    #[test]
    fn provider_cache_hit_rate_weights_detail_and_rollup_tokens_and_respects_filters() {
        let db = test_db();
        let settings = ProxyGatewaySettings {
            log_retention_days: 0,
            ..Default::default()
        };
        insert_rollup(&db); // 40 fresh + 2 creation + 5 read, four requests on May 18.
        for (trace_id, input, read, cli_key, provider_id) in [
            (
                "cache-mixed",
                100,
                80,
                GatewayCliKey::Claude,
                "provider-alpha",
            ),
            (
                "cache-miss",
                800,
                0,
                GatewayCliKey::Claude,
                "provider-alpha",
            ),
            ("other-cli", 0, 1000, GatewayCliKey::Codex, "provider-alpha"),
            (
                "other-provider",
                0,
                1000,
                GatewayCliKey::Claude,
                "provider-beta",
            ),
        ] {
            let mut detail =
                make_detail_for_cli(cli_key, trace_id, provider_id, 200, input, 50_000);
            detail.summary.cache_read_tokens = Some(read);
            detail.summary.cache_creation_tokens =
                Some(if trace_id == "cache-mixed" { 20 } else { 0 });
            record_request_summary(&db, &settings, &detail).unwrap();
        }
        let all_rows = provider_stats(&db, None, None, Some(GatewayCliKey::Claude.into())).unwrap();
        let alpha = all_rows
            .iter()
            .find(|row| row.provider_id == "provider-alpha")
            .unwrap();
        assert_eq!(alpha.request_count, 6);
        assert_eq!(alpha.cache_hit_rate, Some(85.0 / 1047.0));

        let start = Utc
            .with_ymd_and_hms(2026, 5, 20, 0, 0, 0)
            .unwrap()
            .timestamp();
        let end = Utc
            .with_ymd_and_hms(2026, 5, 20, 23, 59, 59)
            .unwrap()
            .timestamp();
        let filtered = provider_stats(
            &db,
            Some(start),
            Some(end),
            Some(GatewayCliKey::Claude.into()),
        )
        .unwrap();
        let alpha = filtered
            .iter()
            .find(|row| row.provider_id == "provider-alpha")
            .unwrap();
        assert_eq!(alpha.request_count, 2);
        assert_eq!(alpha.cache_hit_rate, Some(0.08));
        let codex = provider_stats(
            &db,
            Some(start),
            Some(end),
            Some(GatewayCliKey::Codex.into()),
        )
        .unwrap();
        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0].cache_hit_rate, Some(1.0));

        db.with_conn(|conn| rollup_and_prune(conn, 30))
            .expect("roll older details into history");
        let rolled = provider_stats(&db, None, None, Some(GatewayCliKey::Claude.into())).unwrap();
        let alpha = rolled
            .iter()
            .find(|row| row.provider_id == "provider-alpha")
            .unwrap();
        assert_eq!(alpha.request_count, 6);
        assert_eq!(alpha.cache_hit_rate, Some(85.0 / 1047.0));
    }

    #[test]
    fn native_usage_does_not_dilute_http_latency_before_or_after_rollup() {
        let db = test_db();
        let settings = ProxyGatewaySettings {
            log_retention_days: 0,
            ..Default::default()
        };
        for (id, provider_id, latency) in [
            ("http-one", "provider-alpha", 201),
            ("http-two", "provider-alpha", 400),
            ("native-one", "session", 0),
        ] {
            let mut detail = make_detail(id, provider_id, 200, 10, 20);
            detail.summary.duration_ms = latency;
            detail.summary.ended_at = Utc::now() - Duration::days(400);
            detail.summary.started_at =
                detail.summary.ended_at - Duration::milliseconds(latency as i64);
            record_request_summary(&db, &settings, &detail).unwrap();
        }
        set_request_data_source(&db, "native-one", "session");
        for archived in [false, true] {
            if archived {
                db.with_conn(|conn| rollup_and_prune(conn, 30)).unwrap();
            }
            let models = model_stats(&db, None, None, None).unwrap();
            assert_eq!(models.len(), 1);
            assert_eq!(models[0].request_count, 3);
            assert_eq!(models[0].avg_latency_ms, Some(301));
            let providers = provider_stats(&db, None, None, None).unwrap();
            assert_eq!(
                providers
                    .iter()
                    .find(|row| row.provider_id == "session")
                    .unwrap()
                    .avg_latency_ms,
                None
            );
        }
        let mut live = make_detail("http-live", "provider-alpha", 200, 10, 20);
        live.summary.ended_at = Utc::now();
        live.summary.duration_ms = 600;
        record_request_summary(&db, &settings, &live).unwrap();
        assert_eq!(
            model_stats(&db, None, None, None).unwrap()[0].avg_latency_ms,
            Some(400)
        );
    }

    #[test]
    fn maintenance_failure_does_not_drop_a_new_proxy_summary() {
        let db = test_db();
        let mut old_detail = make_detail("old-before-maintenance", "provider-alpha", 200, 10, 20);
        old_detail.summary.ended_at = Utc::now() - Duration::days(400);
        record_request_summary(
            &db,
            &ProxyGatewaySettings {
                log_retention_days: 0,
                ..Default::default()
            },
            &old_detail,
        )
        .unwrap();
        db.with_conn(|conn| {
            conn.execute_batch("ALTER TABLE usage_daily_rollups DROP COLUMN latency_sample_count")
                .map_err(|error| error.to_string())?;
            assert!(rollup_and_prune(conn, 30).is_err());
            Ok(())
        })
        .unwrap();
        let current_detail = make_detail(
            "current-after-maintenance-failure",
            "provider-alpha",
            200,
            30,
            40,
        );
        let result = record_request_summary(
            &db,
            &ProxyGatewaySettings {
                log_retention_days: 30,
                ..Default::default()
            },
            &current_detail,
        )
        .unwrap();
        assert!(matches!(
            result,
            RecordRequestSummaryOutcome::Written { .. }
        ));
        let logs = request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10).unwrap();
        assert_eq!(logs.total, 2);
        assert!(logs.data.iter().any(
            |row| row.trace_id == "current-after-maintenance-failure" && row.total_tokens == 70
        ));
    }

    #[test]
    fn rollup_and_prune_are_atomic_when_deleting_details_fails() {
        let db = test_db();
        let settings = ProxyGatewaySettings {
            log_retention_days: 0,
            ..Default::default()
        };
        let mut detail = make_detail("atomic-rollup", "provider-alpha", 200, 10, 20);
        detail.summary.ended_at = Utc::now() - Duration::days(400);
        record_request_summary(&db, &settings, &detail).unwrap();
        db.with_conn(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER reject_prune BEFORE DELETE ON proxy_request_logs
                BEGIN SELECT RAISE(ABORT, 'simulated prune failure'); END;",
            )
            .map_err(|error| error.to_string())?;
            assert!(rollup_and_prune(conn, 30).is_err());
            let rolled: i64 = conn
                .query_row("SELECT COUNT(*) FROM usage_daily_rollups", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rolled, 0);
            conn.execute_batch("DROP TRIGGER reject_prune")
                .map_err(|error| error.to_string())?;
            rollup_and_prune(conn, 30)
        })
        .unwrap();
        assert_eq!(
            usage_summary(&db, None, None, None).unwrap().total_requests,
            1
        );
    }

    #[test]
    fn rollups_are_included_in_usage_breakdowns_and_trends() {
        let db = test_db();
        insert_provider(&db, "provider-alpha", "Alpha Provider");
        insert_rollup(&db);

        let start = Utc
            .with_ymd_and_hms(2026, 5, 17, 0, 0, 0)
            .unwrap()
            .timestamp();
        let end = Utc
            .with_ymd_and_hms(2026, 5, 19, 23, 59, 59)
            .unwrap()
            .timestamp();

        let summary = usage_summary(
            &db,
            Some(start),
            Some(end),
            Some(GatewayCliKey::Claude.into()),
        )
        .expect("summary");
        assert_eq!(summary.total_requests, 4);
        assert_eq!(summary.total_tokens, 59);
        assert_eq!(summary.success_rate, 75.0);

        let provider_rows = provider_stats(
            &db,
            Some(start),
            Some(end),
            Some(GatewayCliKey::Claude.into()),
        )
        .expect("provider stats");
        assert_eq!(provider_rows.len(), 1);
        assert_eq!(
            provider_rows[0].provider_name.as_deref(),
            Some("Alpha Provider")
        );
        assert_eq!(provider_rows[0].request_count, 4);
        assert_eq!(provider_rows[0].total_tokens, 59);
        assert_eq!(provider_rows[0].success_rate, 75.0);
        assert_eq!(provider_rows[0].avg_latency_ms, Some(300));

        let model_rows = model_stats(
            &db,
            Some(start),
            Some(end),
            Some(GatewayCliKey::Claude.into()),
        )
        .expect("model stats");
        assert_eq!(model_rows.len(), 1);
        assert_eq!(model_rows[0].request_count, 4);
        assert_eq!(model_rows[0].total_tokens, 59);
        assert_eq!(model_rows[0].avg_latency_ms, Some(300));

        let trend_rows = usage_trends(
            &db,
            Some(start),
            Some(end),
            Some(GatewayCliKey::Claude.into()),
        )
        .expect("trends");
        assert_eq!(trend_rows.len(), 1);
        assert_eq!(trend_rows[0].date, "2026-05-18");
        assert_eq!(trend_rows[0].request_count, 4);
        assert_eq!(trend_rows[0].total_tokens, 59);
    }

    #[test]
    fn record_request_summary_persists_stream_outcome_and_category() {
        let db = test_db();
        insert_provider(&db, "provider-alpha", "Alpha Provider");
        let mut detail = make_detail("trace-stream-failed", "provider-alpha", 200, 10, 20);
        detail.summary.success = false;
        detail.summary.stream_outcome = Some(GatewayStreamOutcome::Incomplete);
        detail.summary.error_category = Some("stream_incomplete".to_string());
        detail.summary.error_message = Some("stream ended without a terminal event".to_string());
        detail.summary.attempt_count = 2;
        detail.summary.total_attempt_count = 3;

        record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
            .expect("record summary");

        let loaded = request_log_detail_from_summary(&db, "trace-stream-failed")
            .expect("load detail")
            .expect("row exists");
        let summary = loaded.summary;
        assert!(
            !summary.success,
            "a 200 with an incomplete stream is not success"
        );
        assert_eq!(
            summary.stream_outcome,
            Some(GatewayStreamOutcome::Incomplete)
        );
        assert_eq!(summary.error_category.as_deref(), Some("stream_incomplete"));
        assert_eq!(summary.attempt_count, 2);
        assert_eq!(summary.total_attempt_count, 3);
    }

    #[test]
    fn request_logs_only_failed_surfaces_mid_stream_failures_on_200() {
        let db = test_db();
        let settings = ProxyGatewaySettings::default();
        insert_provider(&db, "provider-alpha", "Alpha Provider");

        // Clean 200 success.
        let success = make_detail("trace-success", "provider-alpha", 200, 10, 20);
        record_request_summary(&db, &settings, &success).expect("record success");

        // 200 whose stream never delivered a terminal event (the bug class).
        let mut incomplete = make_detail("trace-incomplete", "provider-alpha", 200, 10, 20);
        incomplete.summary.stream_outcome = Some(GatewayStreamOutcome::Incomplete);
        incomplete.summary.success = false;
        record_request_summary(&db, &settings, &incomplete).expect("record incomplete");

        // A genuine 500 failure.
        let mut failed = make_detail("trace-failed", "provider-alpha", 500, 10, 20);
        failed.summary.success = false;
        record_request_summary(&db, &settings, &failed).expect("record failed");

        let logs = request_logs(
            &db,
            &GatewayRequestLogFilters {
                only_failed: Some(true),
                ..GatewayRequestLogFilters::default()
            },
            0,
            10,
        )
        .expect("request logs");

        let trace_ids: Vec<&str> = logs
            .data
            .iter()
            .map(|item| item.trace_id.as_str())
            .collect();
        assert!(trace_ids.contains(&"trace-incomplete"));
        assert!(trace_ids.contains(&"trace-failed"));
        assert!(!trace_ids.contains(&"trace-success"));
        assert_eq!(logs.total, 2);
        // The mid-stream failure must read as success=false despite status 200.
        let incomplete_item = logs
            .data
            .iter()
            .find(|item| item.trace_id == "trace-incomplete")
            .expect("incomplete row present");
        assert!(!incomplete_item.success);
        assert_eq!(incomplete_item.status_code, 200);
    }

    #[test]
    fn request_logs_only_failed_falls_back_to_status_for_legacy_rows() {
        // Rows written before the stream_outcome column (NULL) must still surface
        // in the failed filter when their HTTP status is a failure.
        let db = test_db();
        insert_provider(&db, "provider-alpha", "Alpha Provider");
        let mut legacy = make_detail("trace-legacy-429", "provider-alpha", 429, 10, 20);
        legacy.summary.success = false;
        legacy.summary.stream_outcome = None;
        record_request_summary(&db, &ProxyGatewaySettings::default(), &legacy)
            .expect("record legacy");

        let logs = request_logs(
            &db,
            &GatewayRequestLogFilters {
                only_failed: Some(true),
                ..GatewayRequestLogFilters::default()
            },
            0,
            10,
        )
        .expect("request logs");
        assert_eq!(logs.total, 1);
        assert_eq!(logs.data[0].trace_id, "trace-legacy-429");
    }

    #[test]
    fn record_request_summary_marks_error_terminal_event_as_failed() {
        // A 200 stream that delivered an `error` / `response.failed` envelope
        // ends properly (terminal event delivered) but is a failure, not a
        // success — the original bug class recorded these as clean successes.
        let db = test_db();
        insert_provider(&db, "provider-alpha", "Alpha Provider");
        let mut detail = make_detail("trace-error-envelope", "provider-alpha", 200, 10, 20);
        detail.summary.stream_outcome = Some(GatewayStreamOutcome::Failed);
        detail.summary.success = false;
        detail.summary.error_category = Some("upstream_stream_error".to_string());
        detail.summary.error_message =
            Some("upstream stream delivered a non-success terminal event".to_string());
        record_request_summary(&db, &ProxyGatewaySettings::default(), &detail)
            .expect("record error envelope");

        let loaded = request_log_detail_from_summary(&db, "trace-error-envelope")
            .expect("load detail")
            .expect("row exists");
        assert_eq!(
            loaded.summary.stream_outcome,
            Some(GatewayStreamOutcome::Failed)
        );
        assert!(!loaded.summary.success);
        assert_eq!(
            loaded.summary.error_category.as_deref(),
            Some("upstream_stream_error")
        );

        // And the only-failed filter surfaces it despite the 200 status.
        let logs = request_logs(
            &db,
            &GatewayRequestLogFilters {
                only_failed: Some(true),
                ..GatewayRequestLogFilters::default()
            },
            0,
            10,
        )
        .expect("request logs");
        assert_eq!(logs.total, 1);
        assert_eq!(logs.data[0].trace_id, "trace-error-envelope");
        assert_eq!(logs.data[0].status_code, 200);
        assert!(!logs.data[0].success);
    }

    #[test]
    fn usage_summary_success_rate_counts_stream_failures_on_200() {
        // The core bug: a 200 whose stream never delivered a terminal event
        // (Incomplete) must lower the stats-page success rate, not read as 100%.
        let db = test_db();
        let settings = ProxyGatewaySettings::default();
        insert_provider(&db, "provider-alpha", "Alpha Provider");

        // Clean 200 success.
        let ok = make_detail("trace-ok", "provider-alpha", 200, 10, 20);
        record_request_summary(&db, &settings, &ok).expect("record ok");

        // 200 + Incomplete stream: must NOT count as success in stats.
        let mut incomplete = make_detail("trace-incomplete", "provider-alpha", 200, 10, 20);
        incomplete.summary.stream_outcome = Some(GatewayStreamOutcome::Incomplete);
        incomplete.summary.success = false;
        record_request_summary(&db, &settings, &incomplete).expect("record incomplete");

        let summary =
            usage_summary(&db, None, None, Some(GatewayCliKey::Claude.into())).expect("summary");
        assert_eq!(summary.total_requests, 2);
        assert_eq!(summary.success_rate, 50.0);

        // Provider stats must agree.
        let provider_rows = provider_stats(&db, None, None, Some(GatewayCliKey::Claude.into()))
            .expect("provider stats");
        assert_eq!(provider_rows[0].request_count, 2);
        assert_eq!(provider_rows[0].success_rate, 50.0);
    }

    #[test]
    fn usage_summary_success_rate_counts_failed_terminal_on_200() {
        // A 200 that delivered an `error`/`response.failed` envelope (Failed)
        // must also lower the success rate.
        let db = test_db();
        let settings = ProxyGatewaySettings::default();
        insert_provider(&db, "provider-alpha", "Alpha Provider");

        let ok = make_detail("trace-ok", "provider-alpha", 200, 10, 20);
        record_request_summary(&db, &settings, &ok).expect("record ok");

        let mut failed = make_detail("trace-failed", "provider-alpha", 200, 10, 20);
        failed.summary.stream_outcome = Some(GatewayStreamOutcome::Failed);
        failed.summary.success = false;
        record_request_summary(&db, &settings, &failed).expect("record failed");

        let summary =
            usage_summary(&db, None, None, Some(GatewayCliKey::Claude.into())).expect("summary");
        assert_eq!(summary.success_rate, 50.0);
    }

    #[test]
    fn usage_semantic_distinguishes_completed_from_failed_on_same_envelope() {
        // Same envelope id + tokens + 200, but one Completed and one Failed:
        // the second must NOT be deduplicated as a same-semantic replay.
        let db = test_db();
        let settings = ProxyGatewaySettings::default();
        insert_provider(&db, "provider-alpha", "Alpha Provider");

        let mut completed = make_detail("trace-shared", "provider-alpha", 200, 10, 20);
        completed.summary.stream_outcome = Some(GatewayStreamOutcome::Completed);
        completed.summary.success = true;
        record_request_summary(&db, &settings, &completed).expect("record completed");

        let mut failed = make_detail("trace-shared", "provider-alpha", 200, 10, 20);
        failed.summary.stream_outcome = Some(GatewayStreamOutcome::Failed);
        failed.summary.success = false;
        failed.summary.error_category = Some("upstream_stream_error".to_string());
        // Different trace_id so the collision-fallback path is exercised by
        // the stream_outcome difference, not the trace id.
        let _ = failed;
        record_request_summary(&db, &settings, &failed).expect("record failed");

        let logs =
            request_logs(&db, &GatewayRequestLogFilters::default(), 0, 10).expect("request logs");
        // Both rows survive (the replay idempotency did not swallow the failed one).
        assert_eq!(logs.total, 2);
    }
}
