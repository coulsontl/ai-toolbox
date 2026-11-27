//! Backend services a plugin page reaches through the host.
//!
//! A page runs sandboxed: it cannot read the database, the clipboard or the
//! file system itself. The frontend answers the page bridge's messages with
//! these functions, and only what the package declared in
//! `magpie.permissions` is readable. No read here ever returns a key, token,
//! header or account credential.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{Datelike, Duration, Local, NaiveDate};
use serde_json::{json, Value};

use crate::db::SqliteDbState;

use super::installer;
use super::paths::PluginPaths;
use super::store;
use super::types::{
    InstalledPluginRecord, PluginPageManifest, PluginPageSession, RegisteredPluginPage,
};
use super::PluginManager;

/// Reads a page may be granted. Unknown names are stored but never served.
pub const PAGE_READS: [&str; 3] = ["usage:summary", "usage:day", "providers:list"];

const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_ICON_BYTES: u64 = 256 * 1024;

// ---- registered pages -------------------------------------------------------

/// Every page of every enabled plugin, with its icon as a data URL when it is
/// small enough and stays inside the package.
pub fn list_registered_pages(
    db: &SqliteDbState,
    paths: &PluginPaths,
) -> Result<Vec<RegisteredPluginPage>, String> {
    let mut pages = Vec::new();
    for record in store::list_installed_plugins(db)? {
        if !record.enabled || record.pages.is_empty() {
            continue;
        }
        let target = installer::plugin_target_path(paths, &record);
        for page in &record.pages {
            pages.push(RegisteredPluginPage {
                plugin_id: record.id.clone(),
                plugin_name: record.display_name.clone(),
                package_name: record.package_name.clone(),
                plugin_version: record.version.clone(),
                plugin_icon: record.icon.clone(),
                page_id: page.id.clone(),
                title: if page.title.trim().is_empty() {
                    record.display_name.clone()
                } else {
                    page.title.clone()
                },
                titles: page.titles.clone(),
                icon_data_url: page
                    .icon_file
                    .as_deref()
                    .and_then(|icon| read_icon_data_url(&target, icon)),
                keep_alive: page.keep_alive,
                service_port: page.service_port,
                service_option: page.service_option.clone(),
                permissions: record.permissions.clone(),
            });
        }
    }
    Ok(pages)
}

fn read_icon_data_url(plugin_root: &Path, icon_file: &str) -> Option<String> {
    let canonical_root = fs::canonicalize(plugin_root).ok()?;
    let canonical = fs::canonicalize(canonical_root.join(icon_file)).ok()?;
    if !canonical.starts_with(&canonical_root) {
        return None;
    }
    let metadata = fs::metadata(&canonical).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_ICON_BYTES {
        return None;
    }
    let mime = icon_mime(&canonical)?;
    let bytes = fs::read(&canonical).ok()?;
    Some(format!("data:{mime};base64,{}", BASE64.encode(bytes)))
}

fn icon_mime(path: &Path) -> Option<&'static str> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "svg" => Some("image/svg+xml"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "ico" => Some("image/x-icon"),
        _ => None,
    }
}

// ---- opening a page ---------------------------------------------------------

/// Mint a page token and return everything the host needs to frame it.
pub async fn open_page(
    manager: &PluginManager,
    plugin_id: &str,
    page_id: Option<String>,
) -> Result<PluginPageSession, String> {
    let record = load_enabled_page_plugin(manager.db(), plugin_id)?;
    let (page_id, entry_file, service_port, keep_alive) = {
        let page = select_page(&record, page_id.as_deref())?;
        // Defence in depth: a package without a host part must never hand its
        // sandboxed page a way to some other program's local port (Magpie's
        // `page.go` applies the same rule at parse time).
        let service_port = if record.has_provider_entry {
            resolve_service_port(page, &record.options)
        } else {
            None
        };
        (
            page.id.clone(),
            page.entry_file.clone(),
            service_port,
            page.keep_alive,
        )
    };

    if service_port.is_some() && record.has_provider_entry {
        manager.host().ensure_running().await?;
    }

    let target = installer::plugin_target_path(manager.paths(), &record);
    let entry = installer::resolve_page_entry(&target, &entry_file)?;
    let token = manager
        .page_server()
        .register_page(plugin_id, &page_id, &entry, service_port)?;
    let address = manager.page_server().ensure_started().await?;

    let entry_name = entry
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("'{}' has no usable file name", entry.display()))?;
    let url = format!(
        "http://{address}/_plugin/{token}/{}",
        encode_path_segment(entry_name)
    );
    let service_url = service_port.map(|port| format!("http://127.0.0.1:{port}"));

    Ok(PluginPageSession {
        plugin_id: plugin_id.to_string(),
        page_id,
        url,
        service_url,
        options: record.options,
        permissions: record.permissions,
        keep_alive,
    })
}

/// A page's service port: the declared one, unless the user moved it with the
/// option the package named.
pub fn resolve_service_port(page: &PluginPageManifest, options: &Value) -> Option<u16> {
    let declared = page.service_port?;
    let Some(option) = page.service_option.as_deref() else {
        return Some(declared);
    };
    options
        .get(option)
        .and_then(parse_port_value)
        .or(Some(declared))
}

fn parse_port_value(value: &Value) -> Option<u16> {
    let port = match value {
        Value::Number(number) => number.as_u64()?,
        Value::String(text) => text.trim().parse::<u64>().ok()?,
        _ => return None,
    };
    u16::try_from(port).ok().filter(|port| *port > 0)
}

fn load_enabled_page_plugin(
    db: &SqliteDbState,
    plugin_id: &str,
) -> Result<InstalledPluginRecord, String> {
    let record = store::get_installed_plugin(db, plugin_id)?
        .ok_or_else(|| format!("Plugin '{plugin_id}' is not installed"))?;
    if !record.enabled {
        return Err(format!("Plugin '{plugin_id}' is disabled"));
    }
    if record.pages.is_empty() {
        return Err(format!("Plugin '{plugin_id}' has no custom pages"));
    }
    Ok(record)
}

fn select_page<'a>(
    record: &'a InstalledPluginRecord,
    page_id: Option<&str>,
) -> Result<&'a PluginPageManifest, String> {
    match page_id.filter(|value| !value.trim().is_empty()) {
        Some(page_id) => record
            .pages
            .iter()
            .find(|page| page.id == page_id)
            .ok_or_else(|| format!("Plugin '{}' has no page '{page_id}'", record.id)),
        None => record
            .pages
            .first()
            .ok_or_else(|| format!("Plugin '{}' has no custom pages", record.id)),
    }
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

// ---- reads ------------------------------------------------------------------

/// Answer one `magpie.read(name)` request.
///
/// The plugin must exist, be enabled, and declare the read in
/// `magpie.permissions`; anything else is refused before touching the
/// database.
pub fn read_scope(
    db: &SqliteDbState,
    _manager: &PluginManager,
    plugin_id: &str,
    scope: &str,
    params: Option<&Value>,
) -> Result<Value, String> {
    let record = load_enabled_page_plugin(db, plugin_id)?;
    if !PAGE_READS.contains(&scope) {
        return Err(format!("Plugin page read '{scope}' is not supported"));
    }
    if !record.permissions.iter().any(|read| read == scope) {
        return Err(format!(
            "Plugin '{}' does not declare the '{scope}' permission",
            record.display_name
        ));
    }
    match scope {
        "usage:summary" => usage_summary(db, params),
        "usage:day" => usage_day(db, params),
        "providers:list" => providers_list(db),
        _ => unreachable!("the scope was checked against PAGE_READS"),
    }
}

/// Replace one page plugin's options. Only a plugin that actually has pages is
/// allowed through; enabled state and the object shape are validated by the
/// store.
pub fn set_page_options(
    db: &SqliteDbState,
    plugin_id: &str,
    options: &Value,
) -> Result<Value, String> {
    load_enabled_page_plugin(db, plugin_id)?;
    if !options.is_object() {
        return Err("Plugin options must be a JSON object".to_string());
    }
    let saved = store::set_plugin_options(db, plugin_id, options)?;
    Ok(saved.options)
}

#[derive(Clone, Debug, Default)]
struct UsageBucket {
    calls: u64,
    errors: u64,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    cost: f64,
}

impl UsageBucket {
    fn add(&mut self, other: &UsageBucket) {
        self.calls += other.calls;
        self.errors += other.errors;
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.cost += other.cost;
    }

    fn total_tokens(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

#[derive(Clone, Debug)]
struct UsageRow {
    date: String,
    app_type: String,
    provider_id: String,
    model: String,
    bucket: UsageBucket,
}

fn round_cost(cost: f64) -> f64 {
    (cost * 1_000_000.0).round() / 1_000_000.0
}

/// One day's or range's usage, merging pruned rollups with live request logs.
/// Both tables are read-only here; the gateway owns their maintenance.
fn usage_rows(
    db: &SqliteDbState,
    start_date: &str,
    end_date: &str,
) -> Result<Vec<UsageRow>, String> {
    let mut merged: BTreeMap<(String, String, String, String), UsageBucket> = BTreeMap::new();

    db.with_conn(|conn| {
        let mut detail = conn
            .prepare(
                "SELECT date(created_at, 'unixepoch', 'localtime') AS usage_date,
                        COALESCE(NULLIF(TRIM(app_type), ''), 'unknown'),
                        COALESCE(NULLIF(TRIM(provider_id), ''), 'unknown'),
                        COALESCE(NULLIF(TRIM(model), ''), 'unknown'),
                        COALESCE(SUM(usage_request_count), 0),
                        COALESCE(SUM(CASE WHEN status_code >= 400 THEN usage_request_count ELSE 0 END), 0),
                        COALESCE(SUM(input_tokens), 0),
                        COALESCE(SUM(output_tokens), 0),
                        COALESCE(SUM(cache_read_tokens), 0),
                        COALESCE(SUM(cache_creation_tokens), 0),
                        COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0)
                 FROM proxy_request_logs
                 WHERE date(created_at, 'unixepoch', 'localtime') BETWEEN ?1 AND ?2
                 GROUP BY usage_date, app_type, provider_id, model",
            )
            .map_err(|error| format!("Failed to prepare the gateway usage query: {error}"))?;
        let rows = detail
            .query_map([start_date, end_date], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?.max(0) as u64,
                    row.get::<_, i64>(5)?.max(0) as u64,
                    row.get::<_, i64>(6)?.max(0) as u64,
                    row.get::<_, i64>(7)?.max(0) as u64,
                    row.get::<_, i64>(8)?.max(0) as u64,
                    row.get::<_, i64>(9)?.max(0) as u64,
                    row.get::<_, f64>(10)?.max(0.0),
                ))
            })
            .map_err(|error| format!("Failed to query gateway usage: {error}"))?;
        for row in rows {
            let (date, app_type, provider_id, model, calls, errors, input, output, cache_read, cache_write, cost) =
                row.map_err(|error| format!("Failed to read gateway usage: {error}"))?;
            merged
                .entry((date, app_type, provider_id, model))
                .or_default()
                .add(&UsageBucket {
                    calls,
                    errors,
                    input,
                    output,
                    cache_read,
                    cache_write,
                    cost,
                });
        }

        let mut rollups = conn
            .prepare(
                "SELECT date,
                        COALESCE(NULLIF(TRIM(app_type), ''), 'unknown'),
                        provider_id, model,
                        COALESCE(SUM(request_count), 0),
                        COALESCE(SUM(request_count - success_count), 0),
                        COALESCE(SUM(input_tokens), 0),
                        COALESCE(SUM(output_tokens), 0),
                        COALESCE(SUM(cache_read_tokens), 0),
                        COALESCE(SUM(cache_creation_tokens), 0),
                        COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0)
                 FROM usage_daily_rollups
                 WHERE date BETWEEN ?1 AND ?2
                 GROUP BY date, app_type, provider_id, model",
            )
            .map_err(|error| format!("Failed to prepare the usage rollup query: {error}"))?;
        let rows = rollups
            .query_map([start_date, end_date], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?.max(0) as u64,
                    row.get::<_, i64>(5)?.max(0) as u64,
                    row.get::<_, i64>(6)?.max(0) as u64,
                    row.get::<_, i64>(7)?.max(0) as u64,
                    row.get::<_, i64>(8)?.max(0) as u64,
                    row.get::<_, i64>(9)?.max(0) as u64,
                    row.get::<_, f64>(10)?.max(0.0),
                ))
            })
            .map_err(|error| format!("Failed to query usage rollups: {error}"))?;
        for row in rows {
            let (date, app_type, provider_id, model, calls, errors, input, output, cache_read, cache_write, cost) =
                row.map_err(|error| format!("Failed to read usage rollup: {error}"))?;
            merged
                .entry((date, app_type, provider_id, model))
                .or_default()
                .add(&UsageBucket {
                    calls,
                    errors,
                    input,
                    output,
                    cache_read,
                    cache_write,
                    cost,
                });
        }
        Ok::<(), String>(())
    })?;

    Ok(merged
        .into_iter()
        .map(|((date, app_type, provider_id, model), bucket)| UsageRow {
            date,
            app_type,
            provider_id,
            model,
            bucket,
        })
        .collect())
}

const USAGE_MODELS_LIMIT: usize = 20;
const DAY_AGENTS_LIMIT: usize = 12;
const DAY_MODELS_LIMIT: usize = 12;

/// `usage:summary` in Magpie's shape: a period's totals plus its busiest
/// models. `days` is accepted beside `period`, and the gateway-only
/// `requests` / `totalTokens` aliases are attached for older pages.
fn usage_summary(db: &SqliteDbState, params: Option<&Value>) -> Result<Value, String> {
    let (period, days) = requested_period(params)?;
    let end = Local::now().date_naive();
    let start = end - Duration::days((days - 1) as i64);
    let rows = usage_rows(db, &date_string(start), &date_string(end))?;

    let mut total = UsageBucket::default();
    let mut by_model: BTreeMap<String, UsageBucket> = BTreeMap::new();
    for row in &rows {
        total.add(&row.bucket);
        by_model
            .entry(row.model.clone())
            .or_default()
            .add(&row.bucket);
    }

    let mut models: Vec<(String, UsageBucket)> = by_model.into_iter().collect();
    models.sort_by(|left, right| {
        right
            .1
            .calls
            .cmp(&left.1.calls)
            .then_with(|| left.0.cmp(&right.0))
    });
    let models = models
        .into_iter()
        .take(USAGE_MODELS_LIMIT)
        .map(|(model, bucket)| {
            json!({
                "model": model,
                "calls": bucket.calls,
                "input": bucket.input,
                "output": bucket.output,
                "cost": round_cost(bucket.cost),
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "period": period,
        "since": since_rfc3339(start),
        "calls": total.calls,
        "errors": total.errors,
        "input": total.input,
        "output": total.output,
        "cacheRead": total.cache_read,
        "cacheWrite": total.cache_write,
        "cost": round_cost(total.cost),
        "models": models,
        // Older gateway pages read these two aliases.
        "requests": total.calls,
        "totalTokens": total.total_tokens(),
    }))
}

/// Resolve `days` first (the gateway extension), then `period`.
fn requested_period(params: Option<&Value>) -> Result<(String, u32), String> {
    if let Some(raw) = params.and_then(|params| params.get("days")) {
        let days = raw
            .as_u64()
            .or_else(|| raw.as_str().and_then(|value| value.trim().parse().ok()))
            .filter(|days| (1..=365).contains(days))
            .ok_or_else(|| "days must be a number between 1 and 365".to_string())?
            as u32;
        let period = match days {
            1 => "today".to_string(),
            7 => "7d".to_string(),
            30 => "30d".to_string(),
            other => format!("{other}d"),
        };
        return Ok((period, days));
    }
    let period = params
        .and_then(|params| params.get("period"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|period| !period.is_empty())
        .unwrap_or("today");
    let days = match period {
        "today" => 1,
        "7d" => 7,
        "30d" => 30,
        other => return Err(format!("No period '{other}': today, 7d or 30d")),
    };
    Ok((period.to_string(), days))
}

fn since_rfc3339(date: NaiveDate) -> String {
    use chrono::TimeZone;
    Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight is valid"))
        .single()
        .map(|at| at.to_rfc3339())
        .unwrap_or_else(|| format!("{date}T00:00:00"))
}

/// `usage:day` in Magpie's shape: one day or a span, by agent and model,
/// with the vendors' names as the Usage page shows them. The database groups
/// by this computer's local day; `tz` is accepted for compatibility.
fn usage_day(db: &SqliteDbState, params: Option<&Value>) -> Result<Value, String> {
    let today = Local::now().date_naive();
    let date = match params
        .and_then(|params| params.get("date"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(date) => NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| format!("Invalid date '{date}': expected YYYY-MM-DD"))?,
        None => today,
    };
    if date > today {
        return Err(format!("{date} is after today, {today}"));
    }

    let from = params
        .and_then(|params| params.get("from"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let start = match from {
        Some("all") => earliest_usage_date(db)?.map_or(date, |first| first.min(date)),
        Some(from) => {
            let from = NaiveDate::parse_from_str(from, "%Y-%m-%d")
                .map_err(|_| format!("Invalid date '{from}': expected YYYY-MM-DD"))?;
            if from > date {
                return Err(format!("{from} is after {date}"));
            }
            from
        }
        None => date,
    };

    let rows = usage_rows(db, &date_string(start), &date_string(date))?;
    let provider_names = load_provider_names(db)?;

    let mut total = UsageBucket::default();
    let mut by_agent: BTreeMap<String, (UsageBucket, BTreeMap<(String, String), UsageBucket>)> =
        BTreeMap::new();
    for row in &rows {
        total.add(&row.bucket);
        let agent = by_agent.entry(row.app_type.clone()).or_default();
        agent.0.add(&row.bucket);
        agent
            .1
            .entry((row.provider_id.clone(), row.model.clone()))
            .or_default()
            .add(&row.bucket);
    }

    // Magpie's `days` is the 53-week heatmap ending at `date`, not the span
    // the totals cover: a day card uses it for streaks and an average day.
    let mut by_day: BTreeMap<String, UsageBucket> = BTreeMap::new();
    for row in usage_rows(db, &date_string(heatmap_since(today)), &date_string(date))? {
        by_day.entry(row.date).or_default().add(&row.bucket);
    }

    let mut agents: Vec<(String, UsageBucket, BTreeMap<(String, String), UsageBucket>)> = by_agent
        .into_iter()
        .map(|(id, (bucket, models))| (id, bucket, models))
        .collect();
    agents.sort_by(|left, right| {
        right
            .1
            .total_tokens()
            .cmp(&left.1.total_tokens())
            .then_with(|| left.0.cmp(&right.0))
    });
    let more_agents = agents.len().saturating_sub(DAY_AGENTS_LIMIT);
    let agents = agents
        .into_iter()
        .take(DAY_AGENTS_LIMIT)
        .map(|(id, bucket, by_model)| {
            let mut models: Vec<((String, String), UsageBucket)> = by_model.into_iter().collect();
            models.sort_by(|left, right| {
                right
                    .1
                    .calls
                    .cmp(&left.1.calls)
                    .then_with(|| left.0.cmp(&right.0))
            });
            let more_models = models.len().saturating_sub(DAY_MODELS_LIMIT);
            let models = models
                .into_iter()
                .take(DAY_MODELS_LIMIT)
                .map(|((provider, model), bucket)| {
                    let provider_name = provider_names
                        .get(&provider)
                        .cloned()
                        .unwrap_or_else(|| provider.clone());
                    json!({
                        "model": model,
                        "provider": provider,
                        "providerName": provider_name,
                        "icon": "generic",
                        "calls": bucket.calls,
                        "errors": bucket.errors,
                        "input": bucket.input,
                        "output": bucket.output,
                        "cacheRead": bucket.cache_read,
                        "cacheWrite": bucket.cache_write,
                        "reasoning": 0,
                        "cost": round_cost(bucket.cost),
                        "unpriced": 0,
                        "cacheUnknown": 0,
                    })
                })
                .collect::<Vec<_>>();
            let mut agent = json!({
                "id": id,
                "name": id,
                "icon": "generic",
                "calls": bucket.calls,
                "errors": bucket.errors,
                "input": bucket.input,
                "output": bucket.output,
                "cacheRead": bucket.cache_read,
                "cacheWrite": bucket.cache_write,
                "reasoning": 0,
                "cost": round_cost(bucket.cost),
                "unpriced": 0,
                "cacheUnknown": 0,
                "models": models,
            });
            if more_models > 0 {
                agent["moreModels"] = json!(more_models);
            }
            agent
        })
        .collect::<Vec<_>>();

    let days = by_day
        .into_iter()
        .filter(|(_, bucket)| bucket.calls > 0)
        .map(|(date, bucket)| {
            json!({
                "date": date,
                "calls": bucket.calls,
                "tokens": bucket.total_tokens(),
                "cost": round_cost(bucket.cost),
            })
        })
        .collect::<Vec<_>>();

    let mut payload = json!({
        "from": date_string(start),
        "date": date_string(date),
        "today": date_string(today),
        "calls": total.calls,
        "errors": total.errors,
        "input": total.input,
        "output": total.output,
        "cacheRead": total.cache_read,
        "cacheWrite": total.cache_write,
        "reasoning": 0,
        "cost": round_cost(total.cost),
        "unpriced": 0,
        "cacheUnknown": 0,
        "agents": agents,
        "days": days,
        "icons": {},
        "currency": "USD",
        "rate": 1.0,
        // Older gateway pages read these two aliases.
        "requests": total.calls,
        "totalTokens": total.total_tokens(),
    });
    if more_agents > 0 {
        payload["moreAgents"] = json!(more_agents);
    }
    Ok(payload)
}

/// The first day any usage was recorded, for `from: "all"`.
fn earliest_usage_date(db: &SqliteDbState) -> Result<Option<NaiveDate>, String> {
    let earliest = db.with_conn(|conn| {
        conn.query_row(
            "SELECT MIN(day) FROM (
                SELECT MIN(date(created_at, 'unixepoch', 'localtime')) AS day FROM proxy_request_logs
                UNION ALL
                SELECT MIN(date) AS day FROM usage_daily_rollups
             )",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|error| format!("Failed to query the first usage day: {error}"))
    })?;
    Ok(earliest.and_then(|day| NaiveDate::parse_from_str(&day, "%Y-%m-%d").ok()))
}

fn date_string(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Magpie's heatmap span: 53 weeks, starting on the Monday 52 weeks before
/// this week's Monday.
fn heatmap_since(today: NaiveDate) -> NaiveDate {
    today - Duration::days(today.weekday().num_days_from_monday() as i64 + 7 * 52)
}

/// Configured providers as the UI knows them: id, name, CLI and whether they
/// are active. The `data` JSONB is never echoed, so no key can leak.
fn providers_list(db: &SqliteDbState) -> Result<Value, String> {
    let providers = db.with_conn(|conn| {
        let mut statement = conn
            .prepare(
                "SELECT cli_key, id,
                        COALESCE(json_extract(data, '$.name'), id),
                        COALESCE(CAST(json_extract(data, '$.is_applied') AS INTEGER), 0),
                        COALESCE(CAST(json_extract(data, '$.is_disabled') AS INTEGER), 0)
                 FROM cli_provider
                 ORDER BY cli_key, id",
            )
            .map_err(|error| format!("Failed to prepare the provider query: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|error| format!("Failed to query providers: {error}"))?;
        let mut providers = Vec::new();
        for row in rows {
            let (cli_key, id, name, applied, disabled) =
                row.map_err(|error| format!("Failed to read provider: {error}"))?;
            providers.push(json!({
                "id": id,
                "name": name
                    .map(|name| name.trim().to_string())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| id.clone()),
                "cliKey": cli_key,
                "enabled": applied != 0 && disabled == 0,
            }));
        }
        Ok(providers)
    })?;
    Ok(Value::Array(providers))
}

fn load_provider_names(db: &SqliteDbState) -> Result<HashMap<String, String>, String> {
    db.with_conn(|conn| {
        let mut statement = conn
            .prepare("SELECT id, COALESCE(json_extract(data, '$.name'), id) FROM cli_provider")
            .map_err(|error| format!("Failed to prepare the provider name query: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|error| format!("Failed to query provider names: {error}"))?;
        let mut names = HashMap::new();
        for row in rows {
            let (id, name) =
                row.map_err(|error| format!("Failed to read provider name: {error}"))?;
            if let Some(name) = name
                .map(|name| name.trim().to_string())
                .filter(|n| !n.is_empty())
            {
                names.entry(id).or_insert(name);
            }
        }
        Ok(names)
    })
}

// ---- page-side images -------------------------------------------------------

fn decode_image_base64(data_base64: &str) -> Result<Vec<u8>, String> {
    let payload = data_base64.trim();
    let payload = match payload.split_once(',') {
        Some((header, body)) if header.starts_with("data:") && header.contains(";base64") => body,
        _ => payload,
    };
    if payload.len() > MAX_IMAGE_BYTES * 4 / 3 + 1024 {
        return Err(format!(
            "The picture is larger than {} MB",
            MAX_IMAGE_BYTES >> 20
        ));
    }
    let bytes = BASE64
        .decode(payload.trim())
        .map_err(|error| format!("The picture is not valid base64: {error}"))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "The picture is larger than {} MB",
            MAX_IMAGE_BYTES >> 20
        ));
    }
    Ok(bytes)
}

/// Largest picture a page may hand the host, per side. A 20 MB PNG can decode
/// into gigabytes of RGBA; the dimensions are checked before any pixels are.
const MAX_IMAGE_DIMENSION: u32 = 8192;

fn decode_rgba(bytes: &[u8]) -> Result<image::RgbaImage, String> {
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("The picture could not be read: {error}"))?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| format!("The picture could not be decoded: {error}"))?;
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(format!(
            "The picture is {width}x{height}; at most {MAX_IMAGE_DIMENSION}x{MAX_IMAGE_DIMENSION} is allowed"
        ));
    }
    let image = image::load_from_memory(bytes)
        .map_err(|error| format!("The picture could not be decoded: {error}"))?;
    Ok(image.to_rgba8())
}

/// Put a page's picture on the system clipboard.
pub fn copy_image_base64(data_base64: &str) -> Result<(), String> {
    let bytes = decode_image_base64(data_base64)?;
    let rgba = decode_rgba(&bytes)?;
    let width = rgba.width() as usize;
    let height = rgba.height() as usize;
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("Failed to open the system clipboard: {error}"))?;
    clipboard
        .set_image(arboard::ImageData {
            width,
            height,
            bytes: std::borrow::Cow::Owned(rgba.into_raw()),
        })
        .map_err(|error| format!("Failed to copy the picture: {error}"))
}

/// Save a page's picture in Downloads (or Desktop / home) as a PNG and return
/// the absolute path. Existing files are never overwritten.
pub fn save_image_base64(data_base64: &str, filename: Option<&str>) -> Result<String, String> {
    let bytes = decode_image_base64(data_base64)?;
    let rgba = decode_rgba(&bytes)?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|error| format!("Failed to encode the picture: {error}"))?;

    let directory = dirs::download_dir()
        .or_else(dirs::desktop_dir)
        .or_else(dirs::home_dir)
        .ok_or_else(|| "No Downloads, Desktop or home folder is available".to_string())?;
    let path = save_png_to_dir(&directory, filename, &png)?;
    Ok(path.to_string_lossy().to_string())
}

fn save_png_to_dir(
    directory: &Path,
    filename: Option<&str>,
    png: &[u8],
) -> Result<PathBuf, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("Failed to create '{}': {error}", directory.display()))?;
    let base = sanitize_image_filename(filename);
    for index in 0..1000 {
        let name = if index == 0 {
            format!("{base}.png")
        } else {
            format!("{base}-{index}.png")
        };
        let path = directory.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(png)
                    .map_err(|error| format!("Failed to write '{}': {error}", path.display()))?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Failed to create '{}': {error}", path.display())),
        }
    }
    Err("Too many files with that name in the Downloads folder".to_string())
}

/// A page's suggested file name, stripped to something a file system accepts
/// on every platform. Empty, reserved or path-like names become `magpie`.
pub fn sanitize_image_filename(asked: Option<&str>) -> String {
    let asked = asked.unwrap_or("magpie").trim();
    let lower = asked.to_ascii_lowercase();
    let stem = if lower.ends_with(".png") {
        &asked[..asked.len() - 4]
    } else {
        asked
    };

    let mut cleaned = String::new();
    for character in stem.chars() {
        if character.is_control()
            || matches!(
                character,
                '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'
            )
        {
            cleaned.push('-');
        } else {
            cleaned.push(character);
        }
    }
    let trimmed = cleaned
        .trim_matches(|character: char| character == ' ' || character == '.' || character == '-');
    let mut name: String = trimmed.chars().take(80).collect();
    name = name
        .trim_matches(|character: char| character == ' ' || character == '.' || character == '-')
        .to_string();
    if name.is_empty() {
        return "magpie".to_string();
    }
    if is_reserved_windows_name(&name) {
        return format!("_{name}");
    }
    name
}

fn is_reserved_windows_name(name: &str) -> bool {
    let upper = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim()
        .to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper[3..]
                .chars()
                .all(|character| character.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::{Cli, DbTable};
    use crate::plugins::types::PluginKind;

    fn test_manager() -> (super::super::PluginManagerState, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let db = SqliteDbState::in_memory_for_test().unwrap();
        (PluginManager::new(db, root.path().to_path_buf()), root)
    }

    fn insert_page_plugin(db: &SqliteDbState, id: &str, permissions: Vec<String>, enabled: bool) {
        store::upsert_installed_plugin(
            db,
            &InstalledPluginRecord {
                id: id.to_string(),
                package_name: id.to_string(),
                display_name: "Page Demo".to_string(),
                version: "1.2.3".to_string(),
                kind: PluginKind::Page,
                enabled,
                source_type: "local".to_string(),
                pages: vec![PluginPageManifest {
                    id: "default".to_string(),
                    title: "Daily".to_string(),
                    entry_file: "index.html".to_string(),
                    ..PluginPageManifest::default()
                }],
                permissions,
                ..InstalledPluginRecord::default()
            },
        )
        .unwrap();
    }

    fn insert_rollup(db: &SqliteDbState, date: &str) {
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model, request_count, success_count,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    total_cost_usd, avg_latency_ms
                ) VALUES (?1, 'claude', 'provider-alpha', 'model-alpha', 4, 4, 100, 50, 10, 5, '0.25', 100)",
                [date],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
        })
        .unwrap();
    }

    #[test]
    fn permission_gate_rejects_undeclared_and_unknown_reads() {
        let (manager, _root) = test_manager();
        insert_page_plugin(
            manager.db(),
            "page-demo",
            vec!["usage:summary".to_string()],
            true,
        );

        let error = read_scope(manager.db(), &manager, "page-demo", "usage:day", None).unwrap_err();
        assert!(error.contains("does not declare"), "{error}");

        let error =
            read_scope(manager.db(), &manager, "page-demo", "secrets:list", None).unwrap_err();
        assert!(error.contains("not supported"), "{error}");

        let value = read_scope(manager.db(), &manager, "page-demo", "usage:summary", None).unwrap();
        assert_eq!(value["period"], "today");
        assert_eq!(value["requests"], 0);
        assert_eq!(value["totalTokens"], 0);
        assert_eq!(value["models"].as_array().unwrap().len(), 0);
        assert_eq!(value["cost"], 0.0);
    }

    #[test]
    fn disabled_plugins_refuse_every_read() {
        let (manager, _root) = test_manager();
        insert_page_plugin(
            manager.db(),
            "page-demo",
            vec!["usage:summary".to_string()],
            false,
        );
        let error =
            read_scope(manager.db(), &manager, "page-demo", "usage:summary", None).unwrap_err();
        assert!(error.contains("disabled"), "{error}");
    }

    #[test]
    fn usage_reads_merge_rollups_and_live_logs() {
        let (manager, _root) = test_manager();
        insert_page_plugin(
            manager.db(),
            "page-demo",
            vec!["usage:summary".to_string(), "usage:day".to_string()],
            true,
        );
        let today = date_string(Local::now().date_naive());
        insert_rollup(manager.db(), &today);
        // Outside the 7d summary but inside the day card's 53-week heatmap.
        let ten_days_ago = date_string(Local::now().date_naive() - Duration::days(10));
        insert_rollup(manager.db(), &ten_days_ago);

        manager
            .db()
            .with_conn(|conn| {
                let now = Local::now().timestamp();
                conn.execute(
                    "INSERT INTO proxy_request_logs (
                        request_id, provider_id, app_type, model, input_tokens, output_tokens,
                        cache_read_tokens, cache_creation_tokens, total_cost_usd,
                        status_code, created_at, usage_request_count
                    ) VALUES (
                        'req-1', 'provider-beta', 'codex', 'model-beta', 20, 10,
                        2, 1, '0.5', 200, ?1, 1
                    )",
                    [now],
                )
                .map(|_| ())
                .map_err(|error| error.to_string())
            })
            .unwrap();

        let summary = read_scope(
            manager.db(),
            &manager,
            "page-demo",
            "usage:summary",
            Some(&json!({"period": "7d"})),
        )
        .unwrap();
        assert_eq!(summary["period"], "7d");
        assert_eq!(summary["requests"], 5);
        assert_eq!(summary["calls"], 5);
        assert_eq!(summary["input"], 120);
        assert_eq!(summary["output"], 60);
        assert_eq!(summary["cacheRead"], 12);
        assert_eq!(summary["cacheWrite"], 6);
        assert_eq!(summary["totalTokens"], 198);
        assert_eq!(summary["cost"], 0.75);
        let models = summary["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["model"], "model-alpha");
        assert_eq!(models[0]["calls"], 4);

        let day = read_scope(
            manager.db(),
            &manager,
            "page-demo",
            "usage:day",
            Some(&json!({"date": today})),
        )
        .unwrap();
        assert_eq!(day["date"], today);
        assert_eq!(day["from"], today);
        assert_eq!(day["requests"], 5);
        assert_eq!(day["calls"], 5);
        let agents = day["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0]["id"], "claude");
        assert_eq!(agents[0]["calls"], 4);
        assert_eq!(agents[0]["models"][0]["provider"], "provider-alpha");
        assert_eq!(agents[0]["models"][0]["unpriced"], 0);
        assert!(agents[0].get("moreModels").is_none());
        assert_eq!(agents[1]["id"], "codex");
        assert_eq!(agents[1]["models"][0]["provider"], "provider-beta");
        assert!(day.get("moreAgents").is_none());
        let days = day["days"].as_array().unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(days[0]["date"], ten_days_ago);
        assert_eq!(days[1]["date"], today);
        assert_eq!(days[1]["tokens"], 198);
        assert_eq!(day["currency"], "USD");

        let bad = read_scope(
            manager.db(),
            &manager,
            "page-demo",
            "usage:day",
            Some(&json!({"date": "not-a-date"})),
        )
        .unwrap_err();
        assert!(bad.contains("Invalid date"), "{bad}");
    }

    #[test]
    fn providers_list_does_not_leak_credentials() {
        let (manager, _root) = test_manager();
        insert_page_plugin(
            manager.db(),
            "page-demo",
            vec!["providers:list".to_string()],
            true,
        );
        manager
            .db()
            .with_conn(|conn| {
                crate::db::helpers::db_put(
                    conn,
                    DbTable::CliProvider(Cli::Claude),
                    "provider-alpha",
                    &json!({
                        "name": "Alpha",
                        "api_key": "sk-super-secret",
                        "auth": {"token": "tok-secret"},
                        "is_applied": true,
                    }),
                )
            })
            .unwrap();

        let value =
            read_scope(manager.db(), &manager, "page-demo", "providers:list", None).unwrap();
        let text = value.to_string();
        assert!(!text.contains("sk-super-secret"));
        assert!(!text.contains("tok-secret"));
        assert!(!text.contains("api_key"));
        assert_eq!(value[0]["id"], "provider-alpha");
        assert_eq!(value[0]["name"], "Alpha");
        assert_eq!(value[0]["cliKey"], "claude");
        assert_eq!(value[0]["enabled"], true);
    }

    #[test]
    fn image_filenames_are_cleaned_and_never_overwrite() {
        assert_eq!(sanitize_image_filename(Some("../evil?.png")), "evil");
        assert_eq!(
            sanitize_image_filename(Some("  report:2026/final.png  ")),
            "report-2026-final"
        );
        assert_eq!(sanitize_image_filename(Some("....")), "magpie");
        assert_eq!(sanitize_image_filename(None), "magpie");
        assert_eq!(sanitize_image_filename(Some("con")), "_con");
        let long = "a".repeat(120);
        assert_eq!(sanitize_image_filename(Some(&long)).chars().count(), 80);

        let root = tempfile::tempdir().unwrap();
        let first = save_png_to_dir(root.path(), Some("card.png"), b"png-one").unwrap();
        let second = save_png_to_dir(root.path(), Some("card.png"), b"png-two").unwrap();
        assert_eq!(first.file_name().unwrap(), "card.png");
        assert_eq!(second.file_name().unwrap(), "card-1.png");
        assert_eq!(fs::read(&first).unwrap(), b"png-one");
        assert_eq!(fs::read(&second).unwrap(), b"png-two");
    }

    #[test]
    fn data_url_prefixes_and_sizes_are_validated() {
        let png = BASE64.encode(b"not really a png");
        assert_eq!(
            decode_image_base64(&format!("data:image/png;base64,{png}")).unwrap(),
            b"not really a png"
        );
        assert_eq!(decode_image_base64(&png).unwrap(), b"not really a png");
        assert!(decode_image_base64("***").is_err());
    }

    #[test]
    fn service_port_follows_the_declared_option() {
        let page = PluginPageManifest {
            service_port: Some(3477),
            service_option: Some("port".to_string()),
            ..PluginPageManifest::default()
        };
        assert_eq!(resolve_service_port(&page, &json!({})), Some(3477));
        assert_eq!(
            resolve_service_port(&page, &json!({"port": 4000})),
            Some(4000)
        );
        assert_eq!(
            resolve_service_port(&page, &json!({"port": "4001"})),
            Some(4001)
        );
        assert_eq!(
            resolve_service_port(&page, &json!({"port": "nope"})),
            Some(3477)
        );

        let no_option = PluginPageManifest {
            service_port: Some(5000),
            service_option: None,
            ..PluginPageManifest::default()
        };
        assert_eq!(
            resolve_service_port(&no_option, &json!({"port": 1})),
            Some(5000)
        );
        assert_eq!(
            resolve_service_port(&PluginPageManifest::default(), &json!({})),
            None
        );
    }

    #[test]
    fn oversized_images_are_refused_before_decoding() {
        let image = image::RgbaImage::new(MAX_IMAGE_DIMENSION + 1, 1);
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let error = decode_rgba(&png).unwrap_err();
        assert!(error.contains("at most"), "{error}");
    }

    #[tokio::test]
    async fn a_page_only_plugin_never_reports_a_service_url() {
        let (manager, root) = test_manager();
        let plugin_dir = root.path().join("page-only");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("index.html"), "<html></html>").unwrap();

        store::upsert_installed_plugin(
            manager.db(),
            &InstalledPluginRecord {
                id: "page-only".to_string(),
                package_name: "page-only".to_string(),
                display_name: "Page Only".to_string(),
                kind: PluginKind::Page,
                enabled: true,
                source_type: "local".to_string(),
                local_path: plugin_dir.to_string_lossy().to_string(),
                pages: vec![PluginPageManifest {
                    id: "default".to_string(),
                    entry_file: "index.html".to_string(),
                    // A stale record may still carry a service; the bridge
                    // must not hand it to the page.
                    service_port: Some(3477),
                    service_option: Some("port".to_string()),
                    ..PluginPageManifest::default()
                }],
                options: json!({"port": 4000}),
                ..InstalledPluginRecord::default()
            },
        )
        .unwrap();

        let session = open_page(&manager, "page-only", None).await.unwrap();
        assert_eq!(session.service_url, None);
    }

    #[tokio::test]
    async fn opening_a_page_returns_a_token_scoped_session() {
        let (manager, root) = test_manager();
        let plugin_dir = root.path().join("plugin");
        fs::create_dir_all(plugin_dir.join("page")).unwrap();
        fs::write(plugin_dir.join("page/index.html"), "<html></html>").unwrap();
        fs::write(plugin_dir.join("page/icon.svg"), "<svg></svg>").unwrap();

        store::upsert_installed_plugin(
            manager.db(),
            &InstalledPluginRecord {
                id: "page-demo".to_string(),
                package_name: "page-demo".to_string(),
                display_name: "Page Demo".to_string(),
                kind: PluginKind::Page,
                enabled: true,
                source_type: "local".to_string(),
                local_path: plugin_dir.to_string_lossy().to_string(),
                pages: vec![PluginPageManifest {
                    id: "default".to_string(),
                    title: "Daily".to_string(),
                    entry_file: "page/index.html".to_string(),
                    icon_file: Some("page/icon.svg".to_string()),
                    service_port: Some(3477),
                    service_option: Some("port".to_string()),
                    keep_alive: true,
                    ..PluginPageManifest::default()
                }],
                has_provider_entry: true,
                permissions: vec!["usage:summary".to_string()],
                options: json!({"port": 4000}),
                ..InstalledPluginRecord::default()
            },
        )
        .unwrap();

        let session = open_page(&manager, "page-demo", None).await.unwrap();
        assert_eq!(session.page_id, "default");
        assert!(session.url.starts_with("http://127.0.0.1:"));
        assert!(session.url.contains("/_plugin/"));
        assert!(session.url.ends_with("/index.html"));
        assert_eq!(
            session.service_url.as_deref(),
            Some("http://127.0.0.1:4000")
        );
        assert_eq!(session.permissions, vec!["usage:summary"]);
        assert!(session.keep_alive);

        let pages = list_registered_pages(manager.db(), manager.paths()).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].title, "Daily");
        assert_eq!(
            pages[0].icon_data_url.as_deref(),
            Some("data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=")
        );

        assert!(
            open_page(&manager, "page-demo", Some("missing".to_string()))
                .await
                .is_err()
        );
    }
}
