use super::parsers::{
    bounded_session_first_token_ms, json_millis, native_record, native_session_duration_ms,
    number, read_jsonl, reported_cost, string, timestamp, ParsedSession,
};
use super::{GatewayUsageTool, SessionUsageGranularity, TokenUsage};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// Grok ≥ 0.2 writes one log per install at `~/.grok/logs/unified.jsonl`.
///
/// Each `shell.turn.inference_done` event is one model call with its own token
/// counts; the older `sessions/**/updates.jsonl` mirror simply does not exist
/// on current installs, which is why the importer used to see no Grok usage.
pub(super) fn is_unified_log(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("unified.jsonl")
}

pub(super) fn parse(path: &Path, fallback: i64) -> Result<ParsedSession, String> {
    if is_unified_log(path) {
        return parse_unified_log(path, fallback);
    }
    let mut records = BTreeMap::new();
    let pending = read_jsonl(path, |_, value| {
        if value.get("method").and_then(Value::as_str) != Some("_x.ai/session/update") {
            return;
        }
        let Some(update) = value.pointer("/params/update") else {
            return;
        };
        if update.get("sessionUpdate").and_then(Value::as_str) != Some("turn_completed") {
            return;
        }
        let Some(totals) = update.get("usage") else {
            return;
        };
        let Some(session) = string(&value, &["/params/sessionId"]) else {
            return;
        };
        let Some(prompt) = string(update, &["/prompt_id", "/promptId"]) else {
            return;
        };
        let mut add = |model: &str, raw: &Value| {
            let input = number(raw, &["inputTokens"]);
            let cached = number(raw, &["cachedReadTokens", "cacheReadInputTokens"]).min(input);
            let created = number(raw, &["cacheCreationTokens", "cacheCreationInputTokens"])
                .min(input.saturating_sub(cached));
            let usage = TokenUsage {
                input_tokens: Some(input.saturating_sub(cached).saturating_sub(created)),
                output_tokens: Some(number(raw, &["outputTokens"])),
                cache_read_tokens: Some(cached),
                cache_creation_tokens: Some(created),
                ..Default::default()
            };
            if usage.total_tokens().is_none()
                && raw.get("modelCalls").and_then(Value::as_u64).unwrap_or(0) == 0
            {
                return;
            }
            let mut record = native_record(
                GatewayUsageTool::Grok,
                &session,
                &format!("prompt:{prompt}:{model}"),
                Some(model.into()),
                usage,
                timestamp(&value).unwrap_or(fallback),
            );
            record.metadata.granularity = SessionUsageGranularity::Turn;
            record.metadata.call_count = raw.get("modelCalls").and_then(Value::as_u64);
            record.metadata.reported_total_tokens = raw.get("totalTokens").and_then(Value::as_u64);
            record.metadata.incomplete = totals
                .get("usageIsIncomplete")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || raw
                    .get("usageIsIncomplete")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            record.metadata.native_provider = string(raw, &["/provider"]);
            if !record.metadata.incomplete
                && raw.get("costIsPartial").and_then(Value::as_bool) != Some(true)
            {
                record.reported_cost_usd = reported_cost(
                    raw.get("costUSD").or_else(|| raw.get("costUsd")),
                )
                .or_else(|| {
                    raw.get("costUsdTicks")
                        .and_then(Value::as_i64)
                        .filter(|ticks| *ticks > 0)
                        .map(|ticks| rust_decimal::Decimal::new(ticks, 10).to_string())
                });
            }
            if record.reported_cost_usd.is_some() {
                record.metadata.cost_source = Some("reported".into());
            }
            records.insert(record.request_id.clone(), record);
        };
        if let Some(models) = totals
            .get("modelUsage")
            .and_then(Value::as_object)
            .filter(|models| !models.is_empty())
        {
            for (model, usage) in models {
                add(model, usage);
            }
        } else {
            add("unknown", totals);
        }
    })?;
    Ok(ParsedSession {
        records: records.into_values().collect(),
        pending,
        ..Default::default()
    })
}

/// One record per `shell.turn.inference_done` event.
///
/// `prompt_tokens` includes the cached prompt and `completion_tokens` includes
/// reasoning, so the importer's fresh/cached split is derived here. Events
/// without any token key are skipped: old log lines only carry timing.
fn parse_unified_log(path: &Path, _fallback: i64) -> Result<ParsedSession, String> {
    let mut records = BTreeMap::new();
    // `session created` events carry the session's working directory; the
    // per-turn `inference_done` events do not. Map it by session so a log
    // holding several sessions attributes each turn to its own project — a
    // session whose `session created` is not in the log keeps no project rather
    // than borrowing another session's directory.
    let mut projects = std::collections::HashMap::<String, String>::new();
    let pending = read_jsonl(path, |_, value| {
        match value.get("msg").and_then(Value::as_str) {
            Some("session created") => {
                if let (Some(session), Some(cwd)) =
                    (string(&value, &["/sid"]), string(&value, &["/ctx/cwd"]))
                {
                    projects.insert(session, cwd);
                }
                return;
            }
            Some("shell.turn.inference_done") => {}
            _ => return,
        }
        let Some(ctx) = value.get("ctx").filter(|ctx| ctx.is_object()) else {
            return;
        };
        let turn_project =
            string(&value, &["/sid"]).and_then(|session| projects.get(&session).cloned());
        let token_keys = [
            "prompt_tokens",
            "cached_prompt_tokens",
            "completion_tokens",
            "reasoning_tokens",
        ];
        if !token_keys.iter().any(|key| ctx.get(*key).is_some()) {
            return;
        }
        let Some(session) = string(&value, &["/sid"]) else {
            return;
        };
        let Some(created_at) = timestamp(&value) else {
            return;
        };
        let prompt = number(ctx, &["prompt_tokens"]);
        let cached = number(ctx, &["cached_prompt_tokens"]).min(prompt);
        let completion = number(ctx, &["completion_tokens"]);
        let reasoning = number(ctx, &["reasoning_tokens"]).min(completion);
        let loop_index = ctx.get("loop_index").and_then(Value::as_u64).unwrap_or(0);
        let attempts = ctx.get("attempts").and_then(Value::as_u64).unwrap_or(0);
        let usage = TokenUsage {
            input_tokens: Some(prompt.saturating_sub(cached)),
            // Reasoning is part of the completion count, not an extra bucket.
            output_tokens: Some(completion),
            cache_read_tokens: Some(cached),
            ..Default::default()
        };
        if usage.total_tokens().is_none() {
            return;
        }
        // Same composite identity as tokei: one log can hold several identical
        // counters, and re-reading the file must map them onto the same rows.
        let identity = format!(
            "inference:{created_at}:{loop_index}:{attempts}:{prompt}:{cached}:{completion}:{reasoning}"
        );
        let mut record =
            native_record(GatewayUsageTool::Grok, &session, &identity, None, usage, created_at);
        // The client reports how long the model itself ran, retries excluded,
        // and native `ttft_ms` when available.
        record.duration_ms =
            json_millis(ctx.get("model_elapsed_ms")).and_then(native_session_duration_ms);
        record.first_token_ms = record.duration_ms.and_then(|duration| {
            let ttft = json_millis(ctx.get("ttft_ms")).filter(|ttft| *ttft > 0)?;
            bounded_session_first_token_ms(ttft, duration)
        });
        record.project_path = turn_project;
        record.metadata.granularity = SessionUsageGranularity::Request;
        record.metadata.call_count = Some(1);
        records.insert(record.request_id.clone(), record);
    })?;
    Ok(ParsedSession {
        records: records.into_values().collect(),
        pending,
        ..Default::default()
    })
}
