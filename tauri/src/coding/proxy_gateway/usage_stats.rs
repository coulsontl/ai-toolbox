use super::types::{
    GatewayCliKey, GatewayModelStats, GatewayPaginatedRequestLogs, GatewayProviderStats,
    GatewayRequestLogDetail, GatewayRequestLogFilters, GatewayRequestLogItem,
    GatewayRequestLogSummary, GatewayUsageSummary, GatewayUsageSummaryByCli,
    GatewayUsageTrendPoint, ProxyGatewaySettings,
};
use crate::db::SqliteDbState;
use chrono::{Duration, Local, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, ToSql};
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration as StdDuration, Instant};

type ProviderNameMap = HashMap<(String, String), String>;

const MAX_PAGE_SIZE: u32 = 100;
const ROLLUP_THROTTLE_SECONDS: u64 = 300;

static LAST_ROLLUP_PRUNE_AT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

#[derive(Default)]
struct TrendAccumulator {
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
    ) {
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
    }
}

#[derive(Default)]
struct StatsAccumulator {
    request_count: u64,
    success_count: u64,
    total_tokens: u64,
    total_cost_usd: Decimal,
    latency_weighted_sum: f64,
}

#[derive(Default)]
struct SummaryAccumulator {
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
struct CostBreakdown {
    input_cost_usd: Decimal,
    output_cost_usd: Decimal,
    cache_read_cost_usd: Decimal,
    cache_creation_cost_usd: Decimal,
}

impl CostBreakdown {
    fn total(&self) -> Decimal {
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
    fn add(
        &mut self,
        request_count: u64,
        success_count: u64,
        total_tokens: u64,
        total_cost_usd: Decimal,
        latency_weighted_sum: f64,
    ) {
        self.request_count = self.request_count.saturating_add(request_count);
        self.success_count = self.success_count.saturating_add(success_count);
        self.total_tokens = self.total_tokens.saturating_add(total_tokens);
        self.total_cost_usd += total_cost_usd;
        self.latency_weighted_sum += latency_weighted_sum;
    }

    fn avg_latency_ms(&self) -> u64 {
        if self.request_count == 0 {
            0
        } else {
            (self.latency_weighted_sum / self.request_count as f64)
                .max(0.0)
                .round() as u64
        }
    }
}

pub fn record_request_summary(
    db: &SqliteDbState,
    settings: &ProxyGatewaySettings,
    detail: &GatewayRequestLogDetail,
) -> Result<(), String> {
    let summary = &detail.summary;
    let Some(cli_key) = summary.cli_key else {
        return Ok(());
    };

    db.with_conn(|conn| {
        maybe_rollup_and_prune(conn, i64::from(settings.log_retention_days))?;
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

        conn.execute(
            "INSERT OR REPLACE INTO proxy_request_logs (
                request_id, provider_id, app_type, model, request_model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                total_cost_usd, latency_ms, first_token_ms, duration_ms,
                status_code, error_message, session_id, provider_type, is_streaming,
                cost_multiplier, created_at, data_source, detail_file, detail_offset
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17,
                ?18, ?19, NULL, ?20, ?21,
                ?22, ?23, 'proxy', ?24, ?25
            )",
            rusqlite::params![
                summary.trace_id,
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
                summary.provider_type,
                i64::from(summary.is_streaming),
                cost_multiplier.to_string(),
                created_at,
                summary.detail_file,
                summary.detail_offset.map(|value| value as i64),
            ],
        )
        .map_err(|error| format!("Failed to record proxy gateway request summary: {error}"))?;
        Ok(())
    })
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
                    total_cost_usd, latency_ms, first_token_ms, duration_ms,
                    status_code, error_message, created_at, is_streaming
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
                    trace_id: row.get(0)?,
                    cli_key,
                    provider_id: provider_id.clone(),
                    provider_name: provider_names.get(&(app_type, provider_id)).cloned(),
                    upstream_model_id: row.get(3)?,
                    requested_model: row.get(4)?,
                    status_code: row.get::<_, i64>(13)?.max(0) as u16,
                    success: is_success_status(row.get::<_, i64>(13)?.max(0) as u16),
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
                        .saturating_add(cache_creation_tokens),
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

pub fn usage_summary(
    db: &SqliteDbState,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayCliKey>,
) -> Result<GatewayUsageSummary, String> {
    db.with_conn(|conn| {
        let mut params = Vec::<Box<dyn ToSql>>::new();
        let detail_where = build_stats_where(start_date, end_date, cli_key, "l", &mut params);
        let refs = to_param_refs(&params);
        let mut summary = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*),
                            COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0),
                            COALESCE(SUM(input_tokens), 0),
                            COALESCE(SUM(output_tokens), 0),
                            COALESCE(SUM(cache_read_tokens), 0),
                            COALESCE(SUM(cache_creation_tokens), 0),
                            COALESCE(SUM(CASE WHEN status_code >= 200 AND status_code < 400 THEN 1 ELSE 0 END), 0)
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
    for cli_key in GatewayCliKey::supported_mvp() {
        let summary = usage_summary(db, start_date, end_date, Some(cli_key))?;
        if summary.total_requests > 0 || summary.total_tokens > 0 {
            items.push(GatewayUsageSummaryByCli { cli_key, summary });
        }
    }
    Ok(items)
}

pub fn usage_trends(
    db: &SqliteDbState,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayCliKey>,
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
        let where_clause = build_stats_where(Some(start), Some(end), cli_key, "l", &mut params);
        let refs = to_param_refs(&params);
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {bucket_expr} AS bucket,
                        COUNT(*),
                        COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0),
                        COALESCE(SUM(input_tokens), 0),
                        COALESCE(SUM(output_tokens), 0),
                        COALESCE(SUM(cache_read_tokens), 0),
                        COALESCE(SUM(cache_creation_tokens), 0)
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
                ))
            })
            .map_err(|error| format!("Failed to query proxy gateway trends: {error}"))?;
        for row in rows {
            let (bucket, request_count, total_cost_usd, input, output, cache_read, cache_creation) =
                row.map_err(|error| format!("Failed to read trend row: {error}"))?;
            trend_map.entry(bucket).or_default().add(
                request_count,
                total_cost_usd,
                input,
                output,
                cache_read,
                cache_creation,
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
    cli_key: Option<GatewayCliKey>,
) -> Result<Vec<GatewayProviderStats>, String> {
    db.with_conn(|conn| {
        let provider_names = load_provider_names(conn)?;
        let mut stats_map = HashMap::<(String, String), StatsAccumulator>::new();
        let mut params = Vec::<Box<dyn ToSql>>::new();
        let where_clause = build_stats_where(start_date, end_date, cli_key, "l", &mut params);
        let refs = to_param_refs(&params);
        let mut stmt = conn
            .prepare(&format!(
                "SELECT app_type, provider_id,
                        COUNT(*),
                        COALESCE(SUM(input_tokens + output_tokens + cache_read_tokens + cache_creation_tokens), 0),
                        COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0),
                        COALESCE(SUM(CASE WHEN status_code >= 200 AND status_code < 400 THEN 1 ELSE 0 END), 0),
                        COALESCE(AVG(latency_ms), 0)
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
                let avg_latency_ms = row.get::<_, f64>(6)?.max(0.0);
                Ok((
                    app_type,
                    provider_id,
                    request_count,
                    row.get::<_, i64>(3)?.max(0) as u64,
                    row_decimal(row, 4)?,
                    success_count,
                    avg_latency_ms * request_count as f64,
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
            ) = row.map_err(|error| format!("Failed to read gateway stats row: {error}"))?;
            stats_map
                .entry((app_type, provider_id))
                .or_default()
                .add(
                    request_count,
                    success_count,
                    total_tokens,
                    total_cost,
                    latency_weighted_sum,
                );
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
    cli_key: Option<GatewayCliKey>,
) -> Result<Vec<GatewayModelStats>, String> {
    db.with_conn(|conn| {
        let mut stats_map = HashMap::<(String, String), StatsAccumulator>::new();
        let mut params = Vec::<Box<dyn ToSql>>::new();
        let where_clause = build_stats_where(start_date, end_date, cli_key, "l", &mut params);
        let refs = to_param_refs(&params);
        let mut stmt = conn
            .prepare(&format!(
                "SELECT app_type, model,
                        COUNT(*),
                        COALESCE(SUM(input_tokens + output_tokens + cache_read_tokens + cache_creation_tokens), 0),
                        COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0),
                        COALESCE(AVG(latency_ms), 0)
                 FROM proxy_request_logs l
                 {where_clause}
                 GROUP BY app_type, model
                 ORDER BY 3 DESC"
            ))
            .map_err(|error| format!("Failed to prepare model stats query: {error}"))?;
        let rows = stmt
            .query_map(refs.as_slice(), |row| {
                let app_type: String = row.get(0)?;
                let request_count = row.get::<_, i64>(2)?.max(0) as u64;
                let avg_latency_ms = row.get::<_, f64>(5)?.max(0.0);
                Ok((
                    app_type,
                    row.get::<_, String>(1)?,
                    request_count,
                    row.get::<_, i64>(3)?.max(0) as u64,
                    row_decimal(row, 4)?,
                    avg_latency_ms * request_count as f64,
                ))
            })
            .map_err(|error| format!("Failed to query model stats: {error}"))?;
        for row in rows {
            let (app_type, model, request_count, total_tokens, total_cost, latency_weighted_sum) =
                row.map_err(|error| format!("Failed to read gateway stats row: {error}"))?;
            stats_map
                .entry((app_type, model))
                .or_default()
                .add(request_count, 0, total_tokens, total_cost, latency_weighted_sum);
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

fn merge_rollup_trends(
    conn: &Connection,
    trend_map: &mut std::collections::BTreeMap<String, TrendAccumulator>,
    start: i64,
    end: i64,
    cli_key: Option<GatewayCliKey>,
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
                    COALESCE(SUM(r.cache_creation_tokens), 0)
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
            ))
        })
        .map_err(|error| format!("Failed to query gateway rollup trends: {error}"))?;
    for row in rows {
        let (date, request_count, total_cost, input, output, cache_read, cache_creation) =
            row.map_err(|error| format!("Failed to read gateway rollup trend row: {error}"))?;
        trend_map.entry(date).or_default().add(
            request_count,
            total_cost,
            input,
            output,
            cache_read,
            cache_creation,
        );
    }
    Ok(())
}

fn merge_rollup_provider_stats(
    conn: &Connection,
    stats_map: &mut HashMap<(String, String), StatsAccumulator>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayCliKey>,
) -> Result<(), String> {
    let mut params = Vec::<Box<dyn ToSql>>::new();
    let where_clause = build_rollup_where(start_date, end_date, cli_key, Some("r"), &mut params);
    let refs = to_param_refs(&params);
    let mut stmt = conn
        .prepare(&format!(
            "SELECT r.app_type, r.provider_id,
                    COALESCE(SUM(r.request_count), 0),
                    COALESCE(SUM(r.input_tokens + r.output_tokens + r.cache_read_tokens + r.cache_creation_tokens), 0),
                    COALESCE(SUM(CAST(r.total_cost_usd AS REAL)), 0),
                    COALESCE(SUM(r.success_count), 0),
                    COALESCE(SUM(r.avg_latency_ms * r.request_count), 0)
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
        ) = row.map_err(|error| format!("Failed to read provider rollup row: {error}"))?;
        stats_map.entry((app_type, provider_id)).or_default().add(
            request_count,
            success_count,
            total_tokens,
            total_cost,
            latency_weighted_sum,
        );
    }
    Ok(())
}

fn merge_rollup_model_stats(
    conn: &Connection,
    stats_map: &mut HashMap<(String, String), StatsAccumulator>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayCliKey>,
) -> Result<(), String> {
    let mut params = Vec::<Box<dyn ToSql>>::new();
    let where_clause = build_rollup_where(start_date, end_date, cli_key, Some("r"), &mut params);
    let refs = to_param_refs(&params);
    let mut stmt = conn
        .prepare(&format!(
            "SELECT r.app_type, r.model,
                    COALESCE(SUM(r.request_count), 0),
                    COALESCE(SUM(r.input_tokens + r.output_tokens + r.cache_read_tokens + r.cache_creation_tokens), 0),
                    COALESCE(SUM(CAST(r.total_cost_usd AS REAL)), 0),
                    COALESCE(SUM(r.avg_latency_ms * r.request_count), 0)
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
            ))
        })
        .map_err(|error| format!("Failed to query gateway model rollups: {error}"))?;
    for row in rows {
        let (app_type, model, request_count, total_tokens, total_cost, latency_weighted_sum) =
            row.map_err(|error| format!("Failed to read model rollup row: {error}"))?;
        stats_map.entry((app_type, model)).or_default().add(
            request_count,
            0,
            total_tokens,
            total_cost,
            latency_weighted_sum,
        );
    }
    Ok(())
}

fn rollup_summary(
    conn: &Connection,
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayCliKey>,
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
                    COALESCE(SUM(success_count), 0)
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
        total_requests,
        success_count,
        total_cost_usd: row_decimal(row, 1)?,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_creation_tokens: cache_creation,
    })
}

fn rollup_and_prune(conn: &Connection, retain_days: i64) -> Result<(), String> {
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

    conn.execute(
        "INSERT OR REPLACE INTO usage_daily_rollups
            (date, app_type, provider_id, model, request_count, success_count,
             input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
             total_cost_usd, avg_latency_ms)
         SELECT
            agg.d, agg.app_type, agg.provider_id, agg.model,
            COALESCE(old.request_count, 0) + agg.request_count,
            COALESCE(old.success_count, 0) + agg.success_count,
            COALESCE(old.input_tokens, 0) + agg.input_tokens,
            COALESCE(old.output_tokens, 0) + agg.output_tokens,
            COALESCE(old.cache_read_tokens, 0) + agg.cache_read_tokens,
            COALESCE(old.cache_creation_tokens, 0) + agg.cache_creation_tokens,
            CAST(COALESCE(CAST(old.total_cost_usd AS REAL), 0) + agg.total_cost AS TEXT),
            CASE WHEN COALESCE(old.request_count, 0) + agg.request_count > 0
                THEN (COALESCE(old.avg_latency_ms, 0) * COALESCE(old.request_count, 0)
                      + agg.avg_latency_ms * agg.request_count)
                     / (COALESCE(old.request_count, 0) + agg.request_count)
                ELSE 0 END
         FROM (
            SELECT date(created_at, 'unixepoch', 'localtime') AS d,
                   app_type,
                   provider_id,
                   model,
                   COUNT(*) AS request_count,
                   SUM(CASE WHEN status_code >= 200 AND status_code < 400 THEN 1 ELSE 0 END) AS success_count,
                   COALESCE(SUM(input_tokens), 0) AS input_tokens,
                   COALESCE(SUM(output_tokens), 0) AS output_tokens,
                   COALESCE(SUM(cache_read_tokens), 0) AS cache_read_tokens,
                   COALESCE(SUM(cache_creation_tokens), 0) AS cache_creation_tokens,
                   COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0) AS total_cost,
                   COALESCE(AVG(latency_ms), 0) AS avg_latency_ms
            FROM proxy_request_logs
            WHERE created_at < ?1
            GROUP BY d, app_type, provider_id, model
         ) agg
         LEFT JOIN usage_daily_rollups old
            ON old.date = agg.d
            AND old.app_type = agg.app_type
            AND old.provider_id = agg.provider_id
            AND old.model = agg.model",
        [cutoff],
    )
    .map_err(|error| format!("Failed to roll up gateway logs: {error}"))?;
    conn.execute(
        "DELETE FROM proxy_request_logs WHERE created_at < ?1",
        [cutoff],
    )
    .map_err(|error| format!("Failed to prune gateway logs: {error}"))?;
    Ok(())
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
    rollup_and_prune(conn, retain_days)?;
    *last_run = Some(Instant::now());
    Ok(())
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
    if let Some(cli_key) = filters.cli_key {
        push_condition(
            &mut conditions,
            params,
            "l.app_type",
            cli_key.as_str().to_string(),
        );
    }
    if let Some(status_code) = filters.status_code {
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

    if conditions.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("WHERE {}", conditions.join(" AND ")))
    }
}

fn build_stats_where(
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayCliKey>,
    alias: &str,
    params: &mut Vec<Box<dyn ToSql>>,
) -> String {
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
    if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    }
}

fn build_rollup_where(
    start_date: Option<i64>,
    end_date: Option<i64>,
    cli_key: Option<GatewayCliKey>,
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
    if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    }
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
        ("codex", "codex_provider"),
        ("gemini", "gemini_cli_provider"),
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

fn cli_key_from_app_type(app_type: &str) -> Option<GatewayCliKey> {
    match app_type {
        "claude" => Some(GatewayCliKey::Claude),
        "codex" => Some(GatewayCliKey::Codex),
        "gemini" => Some(GatewayCliKey::Gemini),
        "opencode" => Some(GatewayCliKey::OpenCode),
        _ => None,
    }
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

fn format_decimal_cost(value: Decimal) -> String {
    format!("{:.6}", value.round_dp(6))
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
    for candidate in model_pricing_candidates(model_id) {
        if let Some(pricing) = query_model_pricing_exact(conn, &candidate) {
            return Some(pricing);
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
    let normalized = model_id.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    push_candidate(&mut candidates, normalized.clone());

    let after_namespace = normalized
        .rsplit_once('/')
        .map(|(_, model)| model.to_string())
        .unwrap_or_else(|| normalized.clone());
    push_candidate(&mut candidates, after_namespace.clone());

    let without_provider_suffix = after_namespace
        .split_once(':')
        .map(|(model, _)| model.to_string())
        .unwrap_or_else(|| after_namespace.clone());
    push_candidate(&mut candidates, without_provider_suffix.clone());

    let at_normalized = without_provider_suffix.replace('@', "-");
    push_candidate(&mut candidates, at_normalized.clone());

    for candidate in [without_provider_suffix, at_normalized] {
        if let Some(stripped) = strip_known_model_date_suffix(&candidate) {
            push_candidate(&mut candidates, stripped);
        }
    }

    candidates
}

fn push_candidate(candidates: &mut Vec<String>, value: String) {
    if !value.is_empty() && !candidates.iter().any(|candidate| candidate == &value) {
        candidates.push(value);
    }
}

fn strip_known_model_date_suffix(value: &str) -> Option<String> {
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
                    latency_ms, first_token_ms, duration_ms, status_code, error_message,
                    created_at, is_streaming, total_cost_usd, provider_type,
                    cost_multiplier, detail_file, detail_offset
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
                let success = is_success_status(status_code);
                let total_tokens = input_tokens
                    .saturating_add(output_tokens)
                    .saturating_add(cache_read_tokens)
                    .saturating_add(cache_creation_tokens);
                Ok(GatewayRequestLogDetail {
                    summary: GatewayRequestLogSummary {
                        trace_id: row.get(0)?,
                        started_at,
                        ended_at,
                        cli_key,
                        route_name: app_type.clone(),
                        method: "-".to_string(),
                        path: String::new(),
                        provider_id: Some(provider_id.clone()),
                        provider_name: provider_names.get(&(app_type, provider_id)).cloned(),
                        provider_type: row.get(17)?,
                        cost_multiplier: row.get(18)?,
                        pricing_model_source: None,
                        requested_model: row.get(4)?,
                        upstream_model_id: Some(row.get(3)?),
                        upstream_url: None,
                        status_code: Some(status_code),
                        success,
                        error_category: (!success).then(|| "upstream_error".to_string()),
                        error_message: row.get(13)?,
                        duration_ms,
                        attempt_count: 1,
                        total_attempt_count: 1,
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
                        detail_file: row.get(19)?,
                        detail_offset: row
                            .get::<_, Option<i64>>(20)?
                            .map(|value| value.max(0) as u64),
                    },
                    request_headers: None,
                    request_body: None,
                    upstream_request_body: None,
                    response_headers: None,
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
    use crate::coding::proxy_gateway::types::GatewayRequestLogSummary;
    use crate::db::helpers::db_put;
    use crate::db::schema::DbTable;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn test_db() -> SqliteDbState {
        SqliteDbState::in_memory_for_test().expect("sqlite")
    }

    fn insert_provider(db: &SqliteDbState, id: &str, name: &str) {
        db.with_conn(|conn| {
            db_put(
                conn,
                DbTable::ClaudeProvider,
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

    fn make_detail(
        trace_id: &str,
        provider_id: &str,
        status_code: u16,
        input_tokens: u64,
        output_tokens: u64,
    ) -> GatewayRequestLogDetail {
        let ended_at = Utc.with_ymd_and_hms(2026, 5, 20, 8, 30, 0).unwrap();
        let mut request_headers = BTreeMap::new();
        request_headers.insert("authorization".to_string(), "Bearer redacted".to_string());
        GatewayRequestLogDetail {
            summary: GatewayRequestLogSummary {
                trace_id: trace_id.to_string(),
                started_at: ended_at - Duration::milliseconds(1200),
                ended_at,
                cli_key: Some(GatewayCliKey::Claude),
                route_name: "claude_messages".to_string(),
                method: "POST".to_string(),
                path: "/v1/messages".to_string(),
                provider_id: Some(provider_id.to_string()),
                provider_name: Some("Runtime Name".to_string()),
                provider_type: None,
                cost_multiplier: None,
                pricing_model_source: None,
                requested_model: Some("claude-sonnet-4-5".to_string()),
                upstream_model_id: Some("anthropic/claude-sonnet-4-5".to_string()),
                upstream_url: Some("https://example.test/v1/messages".to_string()),
                status_code: Some(status_code),
                success: (200..400).contains(&status_code),
                error_category: None,
                error_message: None,
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
            response_body: Some("{\"id\":\"msg_1\"}".to_string()),
            provider_attempts: Vec::new(),
        }
    }

    #[test]
    fn record_request_summary_stores_only_compact_fields() {
        let db = test_db();
        insert_provider(&db, "provider-alpha", "Alpha Provider");
        let detail = make_detail("trace-summary", "provider-alpha", 200, 10, 20);

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
            "response_body",
            "attempt_count",
            "total_attempt_count",
            "route_name",
            "path",
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
                cli_key: Some(GatewayCliKey::Claude),
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
        assert!(logs.data[0].success);
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
        assert!(fallback.request_body.is_none());
        assert!(fallback.response_body.is_none());
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

        let summary = usage_summary(&db, None, None, Some(GatewayCliKey::Claude)).expect("summary");
        assert_eq!(summary.total_requests, 2);
        assert_eq!(summary.total_input_tokens, 18);
        assert_eq!(summary.total_output_tokens, 8);
        assert_eq!(summary.total_tokens, 26);
        assert_eq!(summary.success_rate, 50.0);

        let provider_rows =
            provider_stats(&db, None, None, Some(GatewayCliKey::Claude)).expect("provider stats");
        assert_eq!(provider_rows.len(), 1);
        assert_eq!(
            provider_rows[0].provider_name.as_deref(),
            Some("Alpha Provider")
        );
        assert_eq!(provider_rows[0].request_count, 2);
        assert_eq!(provider_rows[0].success_rate, 50.0);

        let model_rows =
            model_stats(&db, None, None, Some(GatewayCliKey::Claude)).expect("model stats");
        assert_eq!(model_rows.len(), 1);
        assert_eq!(model_rows[0].request_count, 2);
        assert_eq!(model_rows[0].total_tokens, 26);
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

        let summary = usage_summary(&db, Some(start), Some(end), Some(GatewayCliKey::Claude))
            .expect("summary");
        assert_eq!(summary.total_requests, 4);
        assert_eq!(summary.total_tokens, 59);
        assert_eq!(summary.success_rate, 75.0);

        let provider_rows =
            provider_stats(&db, Some(start), Some(end), Some(GatewayCliKey::Claude))
                .expect("provider stats");
        assert_eq!(provider_rows.len(), 1);
        assert_eq!(
            provider_rows[0].provider_name.as_deref(),
            Some("Alpha Provider")
        );
        assert_eq!(provider_rows[0].request_count, 4);
        assert_eq!(provider_rows[0].total_tokens, 59);
        assert_eq!(provider_rows[0].success_rate, 75.0);
        assert_eq!(provider_rows[0].avg_latency_ms, 300);

        let model_rows = model_stats(&db, Some(start), Some(end), Some(GatewayCliKey::Claude))
            .expect("model stats");
        assert_eq!(model_rows.len(), 1);
        assert_eq!(model_rows[0].request_count, 4);
        assert_eq!(model_rows[0].total_tokens, 59);
        assert_eq!(model_rows[0].avg_latency_ms, 300);

        let trend_rows =
            usage_trends(&db, Some(start), Some(end), Some(GatewayCliKey::Claude)).expect("trends");
        assert_eq!(trend_rows.len(), 1);
        assert_eq!(trend_rows[0].date, "2026-05-18");
        assert_eq!(trend_rows[0].request_count, 4);
        assert_eq!(trend_rows[0].total_tokens, 59);
    }
}
