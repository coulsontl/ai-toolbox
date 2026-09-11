use super::parsers::{
    native_record, number, read_jsonl, reported_cost, string, timestamp, ParsedSession,
};
use super::{GatewayUsageTool, SessionUsageGranularity, TokenUsage};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub(super) fn parse(path: &Path, fallback: i64) -> Result<ParsedSession, String> {
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
