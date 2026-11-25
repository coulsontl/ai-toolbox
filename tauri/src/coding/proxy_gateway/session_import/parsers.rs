use super::{source_identity, SessionUsageRecord};
use crate::coding::proxy_gateway::types::GatewayUsageTool;
use crate::coding::proxy_gateway::usage_parser::{from_response_body, TokenUsage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::OnceLock;

/// A model call cannot run longer than this; anything above it is a stray
/// timestamp pair (a resumed session, a clock change), not a real invocation.
const MAX_SESSION_DURATION_MS: u64 = 30 * 60 * 1000;

/// A transcript only brackets a call with two independent events. Anything
/// shorter than this is the same event seen twice (a snapshot rewrite, a
/// millisecond truncation) and says nothing about generation time.
const MIN_SESSION_DURATION_MS: u64 = 200;

/// Bounds a raw timestamp delta to something that describes one model call.
pub(super) fn session_duration_ms(start_ms: i64, end_ms: i64) -> Option<u64> {
    let elapsed = end_ms.checked_sub(start_ms)?;
    let elapsed = u64::try_from(elapsed).ok()?;
    bounded_session_duration_ms(elapsed)
}

/// The same sanity window for a duration a transcript states directly, where
/// there is no pair of timestamps to subtract.
pub(super) fn bounded_session_duration_ms(duration_ms: u64) -> Option<u64> {
    (MIN_SESSION_DURATION_MS..=MAX_SESSION_DURATION_MS)
        .contains(&duration_ms)
        .then_some(duration_ms)
}

/// Native per-request durations reported directly by a CLI (e.g. Antigravity
/// protobuf Duration, OMP `duration`, Grok `model_elapsed_ms`, MiniMax
/// `request_duration_ms`, Kimi native latency) do not suffer from timestamp
/// collision, so any positive value up to `MAX_SESSION_DURATION_MS` is valid.
pub(super) fn native_session_duration_ms(duration_ms: u64) -> Option<u64> {
    (duration_ms > 0 && duration_ms <= MAX_SESSION_DURATION_MS).then_some(duration_ms)
}

/// Parses a JSON millisecond number (integer or floating-point, e.g. JS
/// `performance.now()` deltas) into a rounded `u64`.
pub(super) fn json_millis(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    if let Some(int) = value.as_u64() {
        return Some(int);
    }
    let float = value.as_f64()?;
    (float.is_finite() && float >= 0.0 && float <= MAX_SESSION_DURATION_MS as f64)
        .then_some(float.round() as u64)
}

/// Bounds a native or explicit first-token latency to `[0, duration_ms]`.
pub(super) fn bounded_session_first_token_ms(
    first_token_ms: u64,
    duration_ms: u64,
) -> Option<u64> {
    (first_token_ms <= duration_ms).then_some(first_token_ms)
}

/// Derives first-token latency from the request start and first output mark.
/// Must fall strictly within `(0, duration_ms]`; otherwise one of the
/// timestamps belongs to a different event.
pub(super) fn session_first_token_ms(
    start_ms: i64,
    first_output_ms: i64,
    duration_ms: u64,
) -> Option<u64> {
    let elapsed = u64::try_from(first_output_ms.checked_sub(start_ms)?).ok()?;
    (elapsed > 0 && elapsed <= duration_ms).then_some(elapsed)
}

/// The prompt rows that time the next reply, matched on the raw line so the
/// (possibly huge) body is never decoded.
fn user_prompt_line() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        regex::Regex::new(r#""type"\s*:\s*"user"|"role"\s*:\s*"user""#)
            .expect("prompt matcher")
    })
}

/// The timestamp field of such a row, read from the same raw line.
fn line_timestamp() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        regex::Regex::new(r#""timestamp"\s*:\s*"([^"]+)""#).expect("timestamp matcher")
    })
}

/// Parses one matched timestamp string (RFC 3339 or an epoch number) the same
/// way [`timestamp_millis`] does.
fn parse_epoch_millis(value: &str) -> Option<i64> {
    if let Ok(number) = value.parse::<i64>() {
        return Some(if number > 100_000_000_000 {
            number
        } else {
            number * 1000
        });
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.timestamp_millis())
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct CodexSnapshot {
    pub signature: String,
    pub timestamp: i64,
    pub request_id: Option<String>,
}

#[derive(Default)]
pub(super) struct ParsedSession {
    pub pending: bool,
    pub retired_records: Vec<SessionUsageRecord>,
    pub records: Vec<SessionUsageRecord>,
    pub snapshots: Vec<CodexSnapshot>,
    pub parent_thread_id: Option<String>,
    pub started_at: Option<i64>,
}

pub(super) fn revision(cli_key: GatewayUsageTool) -> u32 {
    match cli_key {
        // Revisit cached files to retain envelope identity in the sync ledger
        // and repair matches that joined distinct, identifiable responses.
        // 4: time each reply from the user/tool-result row that sent it.
        // 5: drop the gateway's opening `input: 1 / output: 1` placeholder, so
        // a re-read removes those rows instead of keeping them as billed calls.
        // 6: estimate `first_token_ms` when the opening block is a thinking
        // block with `thinkingDurationMs`.
        // 7: trace request start along Claude's `parentUuid` chain (skipping
        // `attachment` rows) and capture first-block timing before placeholder
        // filtering.
        GatewayUsageTool::Claude | GatewayUsageTool::ClaudeDesktop => 7,
        // 4: resolve a paginated parent through its whole `history_base` lineage
        // and key each page's events by rollout id, so a child no longer re-bills
        // a copied prefix and a rollover page no longer overwrites its sibling.
        // A single-file rollout keeps its old key, so re-reading is a no-op there.
        // 5: parse `token_usage_record` as the authoritative per-response
        // source and deduct the legacy `token_count` mirror that repeats it.
        // 6: time each response from the prompt/tool output that sent it.
        // 7: keep the upstream `response_id` as the envelope identity, so a
        // cross-source match no longer depends on the usage-equal heuristic
        // (which only accepts `completed` proxy rows, issue #432).
        // 8: derive `first_token_ms` from `item_completed.started_at_ms`.
        // 9: track request boundaries with the full `CodexRequestTimer`
        // (matching `response_item` user messages, `turn_context`, tool-wait
        // fallback, and legacy flush slack).
        GatewayUsageTool::Codex => 9,
        // 5: bracket each call with the `turn/start` / `request/header` /
        // `llm/retry-started` event that sent it and the settled message.
        // 6: derive `first_token_ms` from the earliest chunk in `data.stream`.
        // 7: track `step/start` keyed by `(turn, step)`.
        GatewayUsageTool::Dsh => 7,
        // 3: subtract the cached part from `input_tokens` (the column includes
        // it), prefer `completed_at`, and re-read old databases to repair the
        // doubled cache totals. 4 adds the exact `started_at`→`completed_at`
        // duration of each call.
        GatewayUsageTool::Zcode => 4,
        // 4: read `logs/unified.jsonl` as the primary source; older installs
        // still import from the `updates.jsonl` mirror. Each inference also
        // carries its own `model_elapsed_ms`.
        // 5: persist native `ctx.ttft_ms` from `logs/unified.jsonl`.
        GatewayUsageTool::Grok => 5,
        // 3: preserve `usage_row_duration` for OpenCode JSON files instead of
        // overwriting it with an empty `trigger_ms`, and enable the OpenCode
        // token/duration branch for MiMoCode SQLite rows.
        GatewayUsageTool::OpenCode | GatewayUsageTool::Mimocode => 3,
        // 3: extract native `llmFirstTokenLatencyMs` / `llmStreamDurationMs`
        // and fallback step durations from Kimi Code loop events; time Python
        // Kimi CLI steps from `StepBegin`/`StepRetry` to `StatusUpdate`.
        GatewayUsageTool::Kimi | GatewayUsageTool::KimiCli => 3,
        // 3: decode zstd-compressed `transcript_events.event_zstd` rows and
        // extract Pi-compatible `duration_ms`/`first_token_ms`.
        GatewayUsageTool::OpenClaw => 3,
        // 4: token counts come from `conversations/<uuid>.db`; the transcript
        // mirror no longer emits zero-token rows. Generations also carry the
        // provider's own TTFT + streaming duration (fields 11/12).
        // 5: preserve full output tokens (including thoughts) and persist
        // `first_token_ms` from field 11 alongside `duration_ms`.
        GatewayUsageTool::Antigravity => 5,
        // Tools added after the original sixteen start on 2. They move to 3
        // when their transcript gains a duration the parser can read.
        // 4: persist native `ttft` when present on Pi-family messages.
        // 5: parse floating-point millisecond `duration` and `ttft` values.
        GatewayUsageTool::Pi
        | GatewayUsageTool::OhMyPi
        | GatewayUsageTool::OmoNative
        | GatewayUsageTool::PrimeAgent => 5,
        GatewayUsageTool::Musecode | GatewayUsageTool::Qwencode => 3,
        // 4: match top-level `"role":"user"` messages so the first turn in a
        // WorkBuddy/CodeBuddy session gets a start timestamp.
        // 5: prefer canonical `providerData.model` over display `requestModelName`
        // for pricing consistency, and preserve fractional credits.
        GatewayUsageTool::Workbuddy
        | GatewayUsageTool::WorkbuddyAi
        | GatewayUsageTool::Codebuddy => 5,
        // 3: enrich MiniMax Code rows with `request_duration_ms` /
        // `decode_duration_ms` from `local_runtime_message_rows`.
        GatewayUsageTool::Minimax => 3,
        // 3: bracket Command Code assistant replies from the preceding user turn.
        GatewayUsageTool::Cmdcode => 3,
        GatewayUsageTool::Devin => 2,
        // 3: track `trigger_ms` in chronological order during the second pass
        // instead of reusing the final user turn for every earlier reply.
        // 4: resolve opaque Qoder model keys (`qfmodel`, `dfmodel`, ...) via
        // `dynamic-texts.json` + static fallback and track `runtime-config`
        // model switches chronologically.
        GatewayUsageTool::QoderCli | GatewayUsageTool::QoderCliCn => 4,
        // 3: read the project from `chat_session.project_uri` when the session
        // table no longer has `cwd`, so an upgraded schema no longer fails the
        // whole scan at prepare time.
        // 4: resolve opaque Qoder model keys (`qfmodel`, `dfmodel`, ...) via
        // `dynamic-texts.json` + static fallback.
        GatewayUsageTool::QoderIde | GatewayUsageTool::QoderIdeCn => 4,
        _ => 2,
    }
}

#[derive(Clone, Default, PartialEq)]
struct Counters {
    input: u64,
    output: u64,
    cached: u64,
    cache_creation: u64,
}

impl Counters {
    fn parse(value: &Value) -> Option<Self> {
        let fields = value.as_object()?;
        if ![
            "input_tokens",
            "output_tokens",
            "cached_input_tokens",
            "cache_read_input_tokens",
            "cache_write_input_tokens",
            "total_tokens",
        ]
        .iter()
        .any(|field| fields.contains_key(*field))
        {
            return None;
        }
        Some(Self {
            input: number(value, &["input_tokens"]),
            output: number(value, &["output_tokens"]),
            cached: number(value, &["cached_input_tokens", "cache_read_input_tokens"]),
            cache_creation: number(value, &["cache_write_input_tokens"]),
        })
    }

    fn delta(&self, previous: &Self) -> Self {
        Self {
            input: self.input.saturating_sub(previous.input),
            output: self.output.saturating_sub(previous.output),
            cached: self.cached.saturating_sub(previous.cached),
            cache_creation: self.cache_creation.saturating_sub(previous.cache_creation),
        }
    }

    fn update_high_water(&mut self, next: &Self) {
        self.input = self.input.max(next.input);
        self.output = self.output.max(next.output);
        self.cached = self.cached.max(next.cached);
        self.cache_creation = self.cache_creation.max(next.cache_creation);
    }

    fn into_usage(self) -> TokenUsage {
        let cached = self.cached.min(self.input);
        let cache_creation = self.cache_creation.min(self.input.saturating_sub(cached));
        TokenUsage {
            input_tokens: Some(
                self.input
                    .saturating_sub(cached)
                    .saturating_sub(cache_creation),
            ),
            output_tokens: Some(self.output),
            cache_read_tokens: Some(cached),
            cache_creation_tokens: Some(cache_creation),
            ..Default::default()
        }
    }
}

pub(super) fn parse_file(
    cli_key: GatewayUsageTool,
    path: &Path,
    fallback_timestamp: i64,
) -> Result<ParsedSession, String> {
    match cli_key {
        GatewayUsageTool::Pi | GatewayUsageTool::OhMyPi | GatewayUsageTool::OmoNative => {
            return super::pi::parse(cli_key, path, fallback_timestamp)
        }
        GatewayUsageTool::Dsh => return super::dsh::parse(path, fallback_timestamp),
        GatewayUsageTool::Grok => return super::grok::parse(path, fallback_timestamp),
        GatewayUsageTool::ClaudeDesktop => return super::desktop::parse(path, fallback_timestamp),
        GatewayUsageTool::Kimi | GatewayUsageTool::KimiCli => {
            return super::kimi::parse(cli_key, path, fallback_timestamp)
        }
        GatewayUsageTool::OpenClaw => return super::open_claw::parse(path, fallback_timestamp),
        GatewayUsageTool::Zcode => return super::zcode::parse(path, fallback_timestamp),
        GatewayUsageTool::Antigravity => {
            return super::antigravity::parse(path, fallback_timestamp)
        }
        GatewayUsageTool::PrimeAgent => {
            return super::prime_agent::parse(path, fallback_timestamp)
        }
        GatewayUsageTool::Musecode => return super::musecode::parse(path, fallback_timestamp),
        GatewayUsageTool::Cmdcode => return super::cmdcode::parse(path, fallback_timestamp),
        GatewayUsageTool::Qwencode => return super::qwencode::parse(path, fallback_timestamp),
        GatewayUsageTool::Workbuddy | GatewayUsageTool::WorkbuddyAi => {
            return super::workbuddy_family::parse(cli_key, path, fallback_timestamp)
        }
        GatewayUsageTool::Codebuddy => {
            return super::codebuddy::parse(path, fallback_timestamp)
        }
        GatewayUsageTool::QoderCli | GatewayUsageTool::QoderCliCn => {
            return super::qoder_cli::parse(cli_key, path, fallback_timestamp)
        }
        _ => {}
    }
    parse_generic_file(cli_key, path, fallback_timestamp)
}

/// Minimal top-level metadata extracted from a Claude JSONL row without
/// materializing large prompt/tool-result bodies.
#[derive(Deserialize)]
struct ClaudeLineMeta {
    uuid: Option<String>,
    #[serde(rename = "parentUuid")]
    parent_uuid: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<Value>,
}

struct ClaudeChainNode {
    parent: Option<String>,
    timestamp_ms: Option<i64>,
    is_attachment: bool,
}

struct ClaudeMessageTiming {
    first_parent: Option<String>,
    starts_at_first_block: bool,
    first_output_ms: Option<i64>,
    end_ms: Option<i64>,
    saw_stop_reason_key: bool,
    has_stop_reason: bool,
}

fn parse_timestamp_value_millis(value: &Value) -> Option<i64> {
    if let Some(number) = value.as_f64().filter(|number| number.is_finite()) {
        let number = number as i64;
        return Some(if number > 100_000_000_000 {
            number
        } else {
            number * 1000
        });
    }
    value.as_str().and_then(parse_epoch_millis)
}

fn record_claude_chain_value(chain: &mut HashMap<String, ClaudeChainNode>, value: &Value) {
    let Some(uuid) = value.get("uuid").and_then(Value::as_str) else {
        return;
    };
    chain.insert(
        uuid.to_string(),
        ClaudeChainNode {
            parent: value
                .get("parentUuid")
                .and_then(Value::as_str)
                .map(str::to_string),
            timestamp_ms: value
                .get("timestamp")
                .and_then(parse_timestamp_value_millis),
            is_attachment: value.get("type").and_then(Value::as_str) == Some("attachment"),
        },
    );
}

fn record_claude_block_timing(
    cli_key: GatewayUsageTool,
    session_id: &str,
    timings: &mut HashMap<String, ClaudeMessageTiming>,
    value: &Value,
) {
    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return;
    }
    let Some(message_id) = value.pointer("/message/id").and_then(Value::as_str) else {
        return;
    };
    let request_id = if cli_key == GatewayUsageTool::Claude {
        format!("SESSION:{message_id}")
    } else {
        format!("SESSION:{}:{session_id}:{message_id}", cli_key.as_str())
    };
    let block_end_ms = timestamp_millis(value);
    let timing = timings.entry(request_id).or_insert_with(|| {
        let starts_at_first_block = value
            .get("apiBlockIndex")
            .is_none_or(|index| index.as_u64() == Some(0));
        ClaudeMessageTiming {
            first_parent: value
                .get("parentUuid")
                .and_then(Value::as_str)
                .map(str::to_string),
            starts_at_first_block,
            first_output_ms: starts_at_first_block
                .then(|| claude_thinking_first_output_ms(value, block_end_ms?))
                .flatten(),
            end_ms: None,
            saw_stop_reason_key: false,
            has_stop_reason: false,
        }
    });
    timing.end_ms = timing.end_ms.max(block_end_ms);
    if let Some(stop) = value
        .pointer("/message/stop_reason")
        .or_else(|| value.get("stop_reason"))
    {
        timing.saw_stop_reason_key = true;
        if !stop.is_null() {
            timing.has_stop_reason = true;
        }
    }
}

fn resolve_claude_request_start_ms(
    chain: &HashMap<String, ClaudeChainNode>,
    first_parent: Option<&str>,
) -> Option<i64> {
    let mut cursor = first_parent?;
    for _ in 0..32 {
        let node = chain.get(cursor)?;
        if !node.is_attachment {
            return node.timestamp_ms;
        }
        cursor = node.parent.as_deref()?;
    }
    None
}

pub(super) fn parse_generic_file(
    cli_key: GatewayUsageTool,
    path: &Path,
    fallback_timestamp: i64,
) -> Result<ParsedSession, String> {
    if cli_key == GatewayUsageTool::Codex {
        return parse_codex(path, fallback_timestamp);
    }
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut records = BTreeMap::new();
    let mut session_id = source_identity(cli_key, path);
    let file_project = file_project(cli_key, path);
    let is_claude_family = matches!(
        cli_key,
        GatewayUsageTool::Claude | GatewayUsageTool::ClaudeDesktop
    );
    // Claude and Gemini do not record how long a call took. They do write the
    // prompt (or tool result) that triggered it and every streamed snapshot of
    // the answer, so the pair of timestamps brackets the call end to end.
    let mut trigger_ms: Option<i64> = None;
    // A tool result can land between two snapshots of one response; the
    // response still started at the prompt that opened it, so each response
    // remembers its own start and first-output timestamp.
    let mut record_starts = HashMap::<String, i64>::new();
    let mut record_first_outputs = HashMap::<String, i64>::new();
    let mut claude_chain = HashMap::<String, ClaudeChainNode>::new();
    let mut claude_timings = HashMap::<String, ClaudeMessageTiming>::new();
    if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
        let prompt_pattern = user_prompt_line();
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| error.to_string())?;
            let has_usage_marker = line.contains("\"usage\"")
                || line.contains("\"usageMetadata\"")
                || line.contains("\"tokens\"")
                || line.contains("\"$set\"");
            if is_claude_family {
                // For non-usage rows, deserialize only the 4 tiny top-level
                // metadata keys so huge tool outputs are skipped without
                // allocating and quoted `"role":"user"` text inside assistant
                // bodies never spoofs a prompt trigger.
                if !has_usage_marker
                    && (line.contains("\"uuid\"")
                        || line.contains("\"type\":\"user\"")
                        || line.contains("\"type\": \"user\""))
                {
                    if let Ok(meta) = serde_json::from_str::<ClaudeLineMeta>(&line) {
                        let ts_ms = meta
                            .timestamp
                            .as_ref()
                            .and_then(parse_timestamp_value_millis);
                        if meta.kind.as_deref() == Some("user") {
                            if let Some(at) = ts_ms {
                                trigger_ms = Some(at);
                            }
                        }
                        if let Some(uuid) = meta.uuid {
                            claude_chain.insert(
                                uuid,
                                ClaudeChainNode {
                                    parent: meta.parent_uuid,
                                    timestamp_ms: ts_ms,
                                    is_attachment: meta.kind.as_deref() == Some("attachment"),
                                },
                            );
                        }
                    }
                }
            } else if prompt_pattern.is_match(&line) {
                // Non-Claude JSONL transcripts: keep the fast raw-line matcher.
                if let Some(at) = line_timestamp()
                    .captures(&line)
                    .and_then(|captures| captures.get(1))
                    .and_then(|value| parse_epoch_millis(value.as_str()))
                {
                    trigger_ms = Some(at);
                }
            }
            // The transcript can contain very large prompts/tool results. Only
            // decode records that can contain usage or session metadata.
            if !has_usage_marker {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                // A CLI can be in the middle of appending its last JSONL record.
                continue;
            };
            if is_claude_family {
                if let Some(id) = string(
                    &value,
                    &["/sessionId", "/sessionID", "/session_id", "/$set/sessionId"],
                ) {
                    session_id = id;
                }
                record_claude_chain_value(&mut claude_chain, &value);
                // Capture first-block timing before `is_unsettled_message_start_usage`
                // can drop an opening `1/1` placeholder block that still holds the
                // reply's `parentUuid` and `thinkingDurationMs`.
                record_claude_block_timing(cli_key, &session_id, &mut claude_timings, &value);
            }
            collect_values(
                cli_key,
                path,
                &value,
                &mut session_id,
                index,
                fallback_timestamp,
                &mut records,
                file_project.as_deref(),
                &mut trigger_ms,
                &mut record_starts,
                &mut record_first_outputs,
            );
        }
        if is_claude_family && !claude_chain.is_empty() {
            for record in records.values_mut() {
                let Some(timing) = claude_timings.get(&record.request_id) else {
                    continue;
                };
                if timing.first_parent.is_none() {
                    continue;
                }
                let complete = !timing.saw_stop_reason_key || timing.has_stop_reason;
                if !complete || !timing.starts_at_first_block {
                    record.duration_ms = None;
                    record.first_token_ms = None;
                    continue;
                }
                let Some(start_ms) =
                    resolve_claude_request_start_ms(&claude_chain, timing.first_parent.as_deref())
                else {
                    continue;
                };
                let duration_ms = timing
                    .end_ms
                    .and_then(|end_ms| session_duration_ms(start_ms, end_ms));
                let first_token_ms = match (timing.first_output_ms, duration_ms) {
                    (Some(first_output_ms), Some(duration)) => {
                        session_first_token_ms(start_ms, first_output_ms, duration)
                    }
                    _ => None,
                };
                record.duration_ms = duration_ms;
                record.first_token_ms = first_token_ms;
            }
        }
    } else {
        let value: Value =
            serde_json::from_reader(BufReader::new(file)).map_err(|error| error.to_string())?;
        collect_values(
            cli_key,
            path,
            &value,
            &mut session_id,
            0,
            fallback_timestamp,
            &mut records,
            file_project.as_deref(),
            &mut trigger_ms,
            &mut record_starts,
            &mut record_first_outputs,
        );
    }
    Ok(ParsedSession {
        records: records.into_values().collect(),
        ..Default::default()
    })
}

/// When a Claude reply opens with a `thinking` block carrying
/// `thinkingDurationMs`, its start time is `block_end_ms - thinkingDurationMs`.
fn claude_thinking_first_output_ms(value: &Value, block_end_ms: i64) -> Option<i64> {
    let starts_at_first_block = value
        .get("apiBlockIndex")
        .is_none_or(|index| index.as_u64() == Some(0));
    if !starts_at_first_block {
        return None;
    }
    let duration_ms = value.get("thinkingDurationMs")?.as_i64()?;
    let first_block_type = value
        .pointer("/message/content/0/type")
        .and_then(Value::as_str)?;
    (first_block_type == "thinking" && duration_ms > 0)
        .then(|| block_end_ms.checked_sub(duration_ms))
        .flatten()
}

#[allow(clippy::too_many_arguments)]
fn collect_values(
    cli_key: GatewayUsageTool,
    path: &Path,
    value: &Value,
    session_id: &mut String,
    index: usize,
    fallback_timestamp: i64,
    records: &mut BTreeMap<String, SessionUsageRecord>,
    file_project: Option<&str>,
    trigger_ms: &mut Option<i64>,
    record_starts: &mut HashMap<String, i64>,
    record_first_outputs: &mut HashMap<String, i64>,
) {
    if let Some(id) = string(
        value,
        &["/sessionId", "/sessionID", "/session_id", "/$set/sessionId"],
    ) {
        *session_id = id;
    }
    // JSON-array transcripts (Gemini) hand each row to the collector directly,
    // so their prompt rows are timed here rather than from a raw line.
    if value.get("type").and_then(Value::as_str) == Some("user")
        || value.pointer("/message/role").and_then(Value::as_str) == Some("user")
    {
        if let Some(at) = timestamp_millis(value) {
            *trigger_ms = Some(at);
        }
    }
    if let Some(items) = value.as_array().or_else(|| {
        [
            "/messages",
            "/turns",
            "/entries",
            "/records",
            "/$set/messages",
        ]
        .iter()
        .find_map(|path| value.pointer(path).and_then(Value::as_array))
    }) {
        for (item_index, item) in items.iter().enumerate() {
            collect_values(
                cli_key,
                path,
                item,
                session_id,
                item_index,
                fallback_timestamp,
                records,
                file_project,
                trigger_ms,
                record_starts,
                record_first_outputs,
            );
        }
        return;
    }
    if cli_key == GatewayUsageTool::ClaudeDesktop
        && value.get("type").and_then(Value::as_str) != Some("assistant")
        && value.pointer("/message/role").and_then(Value::as_str) != Some("assistant")
    {
        return;
    }
    let Some(mut record) =
        parse_value_with_project(cli_key, value, session_id, index, fallback_timestamp, file_project)
    else {
        return;
    };
    // No earlier trigger (a clipped log, a non-JSONL sibling) means this
    // transcript cannot time the call; guessing from the file mtime would read
    // as a measured duration and inflate the statistics. A row whose own
    // timestamp precedes the trigger (Claude writes attachments as a reply
    // lands) is not a call start either.
    let end_ms = timestamp_millis(value);
    let started_at = match record_starts.get(&record.request_id) {
        Some(start) => Some(*start),
        None => {
            let start = trigger_ms.filter(|start| end_ms.is_none_or(|end| end >= *start));
            if let Some(start) = start {
                record_starts.insert(record.request_id.clone(), start);
            }
            start
        }
    };
    if matches!(
        cli_key,
        GatewayUsageTool::Claude | GatewayUsageTool::ClaudeDesktop
    ) && !record_first_outputs.contains_key(&record.request_id)
    {
        if let Some(first_output) = end_ms.and_then(|end| claude_thinking_first_output_ms(value, end)) {
            record_first_outputs.insert(record.request_id.clone(), first_output);
        }
    }
    if record.duration_ms.is_none() {
        record.duration_ms = started_at
            .and_then(|start| end_ms.and_then(|end| session_duration_ms(start, end)));
    }
    let mut legacy_hasher = std::collections::hash_map::DefaultHasher::new();
    cli_key.as_str().hash(&mut legacy_hasher);
    path.to_string_lossy().hash(&mut legacy_hasher);
    index.hash(&mut legacy_hasher);
    record
        .legacy_request_ids
        .push(format!("SESSION:{:016x}", legacy_hasher.finish()));
    if let Some(previous) = records.get(&record.request_id) {
        // Claude/Gemini may write several snapshots of one response. They are
        // one invocation, and partial snapshots must not erase known counters.
        if record.project_path.is_none() {
            record.project_path = previous.project_path.clone();
        }
        // The last snapshot is the one closest to the reply finishing, so the
        // larger estimate is the end-to-end duration of the call.
        record.duration_ms = record.duration_ms.max(previous.duration_ms);
        record.first_token_ms = previous.first_token_ms.or(record.first_token_ms);
        record.usage.input_tokens = record.usage.input_tokens.max(previous.usage.input_tokens);
        record.usage.output_tokens = record.usage.output_tokens.max(previous.usage.output_tokens);
        record.usage.cache_read_tokens = record
            .usage
            .cache_read_tokens
            .max(previous.usage.cache_read_tokens);
        record.usage.cache_creation_tokens = record
            .usage
            .cache_creation_tokens
            .max(previous.usage.cache_creation_tokens);
        record
            .legacy_request_ids
            .extend(previous.legacy_request_ids.iter().cloned());
    }
    if record.first_token_ms.is_none() {
        if let (Some(start), Some(first_output), Some(duration)) = (
            started_at,
            record_first_outputs.get(&record.request_id).copied(),
            record.duration_ms,
        ) {
            record.first_token_ms = session_first_token_ms(start, first_output, duration);
        }
    }
    records.insert(record.request_id.clone(), record);
}

pub(super) fn parse_value(
    cli_key: GatewayUsageTool,
    value: &Value,
    session_id: &str,
    index: usize,
    fallback_timestamp: i64,
) -> Option<SessionUsageRecord> {
    parse_value_with_project(cli_key, value, session_id, index, fallback_timestamp, None)
}

/// The project a transcript belongs to when the rows themselves do not say.
///
/// Gemini keys its session directories by a hash of the project root and keeps
/// the original path in `<project>/.project_root` (the CLI's own registry is
/// `projects.json` at the root); everything else records the cwd on the row or
/// session header.
fn file_project(cli_key: GatewayUsageTool, path: &Path) -> Option<String> {
    if cli_key != GatewayUsageTool::Gemini {
        return None;
    }
    // `<tmp>/<project-key>/chats/session-*.json(l)`: the project directory is
    // the one holding `chats`.
    let project_dir = path.parent()?.parent()?;
    std::fs::read_to_string(project_dir.join(".project_root"))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// `project_path` carries the cwd the caller already found on a session header
/// (Claude/Gemini rows repeat it, but the catch-all parser is also reused by the
/// catch-all transcript readers); an explicit field on the row wins over it.
pub(super) fn parse_value_with_project(
    cli_key: GatewayUsageTool,
    value: &Value,
    session_id: &str,
    index: usize,
    fallback_timestamp: i64,
    project_path: Option<&str>,
) -> Option<SessionUsageRecord> {
    let usage = if cli_key == GatewayUsageTool::Gemini && value.get("tokens").is_some() {
        if value.get("type").and_then(Value::as_str) != Some("gemini") {
            return None;
        }
        let tokens = value.get("tokens")?;
        let input = number(tokens, &["input", "input_tokens"]);
        let cached = number(tokens, &["cached", "cache_read_input_tokens"]).min(input);
        TokenUsage {
            input_tokens: Some(input.saturating_sub(cached)),
            output_tokens: Some(
                number(tokens, &["output", "output_tokens"])
                    .saturating_add(number(tokens, &["thoughts"])),
            ),
            cache_read_tokens: Some(cached),
            ..Default::default()
        }
    } else if matches!(
        cli_key,
        GatewayUsageTool::OpenCode | GatewayUsageTool::Mimocode
    ) && value.get("tokens").is_some()
    {
        if !is_opencode_usage_row(value)
            || value.pointer("/time/completed").is_none_or(Value::is_null)
        {
            return None;
        }
        let tokens = value.get("tokens")?;
        TokenUsage {
            // OpenCode's input is already fresh, unlike Codex/Gemini input.
            input_tokens: Some(number(tokens, &["input", "input_tokens"])),
            output_tokens: Some(
                number(tokens, &["output", "output_tokens"])
                    .saturating_add(number(tokens, &["reasoning"])),
            ),
            cache_read_tokens: Some(
                tokens
                    .pointer("/cache/read")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            ),
            cache_creation_tokens: Some(
                tokens
                    .pointer("/cache/write")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            ),
            ..Default::default()
        }
    } else {
        let candidate = usage_candidate(value)?;
        from_response_body(cli_key.gateway_cli()?, &serde_json::to_vec(candidate).ok()?)
    };
    let message_id = string(
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
    let model = string(
        value,
        &[
            "/model",
            "/modelID",
            "/request/model",
            "/response/model",
            "/message/model",
            "/metadata/model",
            // OpenCode 2.0 nests the native id under `model`; the object itself
            // is skipped by `string`, and its `variant` is a reasoning overlay
            // rather than part of the catalog id the V1 `modelID` carries.
            "/model/id",
        ],
    )
    .unwrap_or_else(|| "unknown".to_string());
    let has_reported_usage = usage.input_tokens.is_some()
        || usage.output_tokens.is_some()
        || usage.cache_read_tokens.is_some()
        || usage.cache_creation_tokens.is_some();
    if matches!(
        cli_key,
        GatewayUsageTool::Claude | GatewayUsageTool::ClaudeDesktop
    ) && is_unsettled_message_start_usage(value, &usage)
    {
        // A translating gateway (AxonHub and friends) fills the streaming
        // `message_start` with `input: 1 / output: 1` because the upstream's
        // counts only arrive with the final chunk. When the client records
        // that snapshot, the real numbers are in a later `message_delta` (or
        // are lost entirely if the client never persisted that event). A real
        // turn always sends far more than one input token, so this shape is a
        // placeholder, never a billed call.
        return None;
    }
    let is_identifiable_claude_response = matches!(
        cli_key,
        GatewayUsageTool::Claude | GatewayUsageTool::ClaudeDesktop
    ) && message_id.is_some()
        && !matches!(model.as_str(), "unknown" | "<synthetic>")
        && (value.get("type").and_then(Value::as_str) == Some("assistant")
            || value.pointer("/message/role").and_then(Value::as_str) == Some("assistant")
            || value.get("role").and_then(Value::as_str) == Some("assistant"));
    if usage.total_tokens().is_none() && !(has_reported_usage && is_identifiable_claude_response) {
        return None;
    }
    let message_id = message_id.unwrap_or_else(|| format!("{session_id}:{index}"));
    let request_id = if cli_key == GatewayUsageTool::Claude {
        format!("SESSION:{message_id}")
    } else {
        format!("SESSION:{}:{session_id}:{message_id}", cli_key.as_str())
    };
    let created_at = timestamp(value).unwrap_or(fallback_timestamp);
    Some(SessionUsageRecord {
        metadata: Default::default(),
        request_id,
        legacy_request_ids: Vec::new(),
        cli_key,
        model,
        usage,
        created_at,
        session_id: session_id.to_string(),
        reported_cost_usd: value
            .get("cost")
            .and_then(Value::as_f64)
            .filter(|cost| cost.is_finite() && *cost > 0.0)
            .map(|cost| cost.to_string()),
        project_path: string(value, &["/cwd", "/working_directory", "/work_dir"])
            .or_else(|| project_path.map(str::to_string))
            .filter(|path| !path.trim().is_empty()),
        credits: None,
        duration_ms: usage_row_duration(cli_key, value),
        first_token_ms: None,
    })
}

/// Whether a Claude assistant row carries the gateway's opening placeholder
/// usage instead of the turn's billed counts.
///
/// A translating gateway cannot know the real counts when it opens the stream,
/// so it writes the literal `input: 1 / output: 1` (AxonHub's
/// `message_start`) and reports the true numbers in the closing
/// `message_delta`. The client persists that opening snapshot when it appends
/// each streamed block; without this guard every such row becomes a 2-token
/// "request". Two signals keep the check safe:
///
/// - `stop_reason` is only set on a settled reply, so a row that carries it is
///   an account of a finished turn even if the counts are small.
/// - A real turn always sends the system prompt and the conversation, so an
///   exact `1 / 1 / 0 / 0` split cannot be a genuine measurement.
fn is_unsettled_message_start_usage(value: &Value, usage: &TokenUsage) -> bool {
    let is_assistant = value.get("type").and_then(Value::as_str) == Some("assistant")
        || value.pointer("/message/role").and_then(Value::as_str) == Some("assistant")
        || value.get("role").and_then(Value::as_str) == Some("assistant");
    is_assistant
        && value
            .pointer("/message/stop_reason")
            .or_else(|| value.get("stop_reason"))
            .is_none_or(Value::is_null)
        && usage.input_tokens == Some(1)
        && usage.output_tokens == Some(1)
        && usage.cache_read_tokens.unwrap_or(0) == 0
        && usage.cache_creation_tokens.unwrap_or(0) == 0
}

/// The elapsed time a row itself records, for the CLIs that bracket a call
/// with two timestamps on one record. Tools whose timing needs the previous
/// event are handled inside their own parser.
fn usage_row_duration(cli_key: GatewayUsageTool, value: &Value) -> Option<u64> {
    if !matches!(
        cli_key,
        GatewayUsageTool::OpenCode | GatewayUsageTool::Mimocode
    ) {
        return None;
    }
    // OpenCode keeps both ends in milliseconds: start of the request and the
    // completed assistant message.
    let started = value
        .pointer("/time/created")
        .and_then(Value::as_i64)?;
    let completed = value
        .pointer("/time/completed")
        .and_then(Value::as_i64)?;
    session_duration_ms(started, completed)
}

fn parse_codex(path: &Path, fallback_timestamp: i64) -> Result<ParsedSession, String> {
    let Some(reader) =
        crate::coding::session_manager::codex_rollout::open_rollout_reader(path, None)
    else {
        return Err(format!("Cannot open Codex rollout {}", path.display()));
    };
    let rollout_id = crate::coding::session_manager::codex_rollout::parse_rollout_path(path)
        .map(|name| name.rollout_id);
    parse_codex_lines(
        reader,
        source_identity(GatewayUsageTool::Codex, path),
        rollout_id.as_deref(),
        fallback_timestamp,
    )
}

/// Snapshots of a Codex rollout lineage, oldest segment first.
///
/// A paginated thread spreads one conversation over several rollout files
/// chained by `history_base`, and each ancestor holds only the records its
/// child did not copy. Reading the leaf alone therefore sees a suffix, not the
/// thread's real timeline — which is what a child's replay check compares
/// against. Each segment is read only up to the byte offset its child recorded.
pub(super) fn read_codex_lineage_snapshots(
    lineage: &crate::coding::session_manager::codex_rollout::Lineage,
    fallback_timestamp: i64,
) -> Result<Vec<CodexSnapshot>, String> {
    let mut snapshots = Vec::new();
    for segment in &lineage.segments {
        let Some(reader) = crate::coding::session_manager::codex_rollout::open_rollout_reader(
            &segment.path,
            segment.end_byte_offset,
        ) else {
            return Err(format!(
                "Cannot open Codex rollout {}",
                segment.path.display()
            ));
        };
        // Only the snapshots are used here, so the thread identity and the
        // per-event request ids are irrelevant; `parse_codex_lines` computes
        // them anyway to keep one parser for every Codex rollout read.
        snapshots
            .extend(parse_codex_lines(reader, String::new(), None, fallback_timestamp)?.snapshots);
    }
    Ok(snapshots)
}

const CODEX_LINE_HEAD_BYTES: usize = 512;
const CODEX_SAME_FLUSH_SLACK_MS: i64 = 100;

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn head_str<'a>(head: &'a [u8], needle: &[u8]) -> Option<&'a str> {
    let start = find_bytes(head, needle)? + needle.len();
    let len = head[start..].iter().position(|byte| *byte == b'"')?;
    std::str::from_utf8(&head[start..start + len]).ok()
}

enum CodexTimingLine {
    Boundary,
    ToolOutput,
    ModelOutput,
    TurnReset,
    OutputItemStarted(i64),
}

fn classify_codex_timing_line(line: &str) -> Option<(i64, CodexTimingLine)> {
    let bytes = line.as_bytes();
    let head = &bytes[..bytes.len().min(CODEX_LINE_HEAD_BYTES)];
    let (envelope, payload) = match find_bytes(head, b"\"payload\":{") {
        Some(at) => head.split_at(at),
        None => (head, &[][..]),
    };
    let kind = match head_str(envelope, b"\"type\":\"")? {
        "turn_context" => CodexTimingLine::Boundary,
        "response_item" => match head_str(payload, b"\"type\":\"")? {
            "message" => match head_str(payload, b"\"role\":\"")? {
                "assistant" => CodexTimingLine::ModelOutput,
                _ => CodexTimingLine::Boundary,
            },
            "reasoning" => CodexTimingLine::ModelOutput,
            item if item.ends_with("_output") => CodexTimingLine::ToolOutput,
            item if item.ends_with("_call") => CodexTimingLine::ModelOutput,
            _ => return None,
        },
        "event_msg" => match head_str(payload, b"\"type\":\"")? {
            "task_started" | "turn_aborted" => CodexTimingLine::TurnReset,
            "user_message" => CodexTimingLine::Boundary,
            "item_completed" => {
                let item_at = find_bytes(payload, b"\"item\":{")?;
                match head_str(&payload[item_at..], b"\"type\":\"")? {
                    "Reasoning" | "AgentMessage" => {}
                    _ => return None,
                }
                let value = serde_json::from_str::<Value>(line).ok()?;
                let started_at_ms = value.pointer("/payload/started_at_ms")?.as_i64()?;
                CodexTimingLine::OutputItemStarted(started_at_ms)
            }
            _ => return None,
        },
        _ => return None,
    };
    let timestamp_ms = head_str(envelope, b"\"timestamp\":\"").and_then(parse_epoch_millis)?;
    Some((timestamp_ms, kind))
}

/// Estimates per-request duration and TTFT across Codex rollout layouts.
///
/// Mirrors the reference `RequestTimer`:
/// - `request_start_ms` latches `last_boundary_ms` on the first model output
///   item (ignoring legacy 2025 output items flushed within 100ms of the
///   previous settlement).
/// - `token_usage_record` settles immediately with its own end timestamp.
/// - Legacy `token_count` falls back to `last_model_output_ms` when tool
///   outputs landed right before `token_count` (`<= 100ms`), avoiding charging
///   tool execution time to the model call.
#[derive(Default)]
struct CodexRequestTimer {
    last_boundary_ms: Option<i64>,
    last_settle_ms: Option<i64>,
    request_start_ms: Option<i64>,
    last_model_output_ms: Option<i64>,
    tool_output_after_model_output_ms: Option<i64>,
    first_output_ms: Option<i64>,
}

impl CodexRequestTimer {
    fn observe_line(&mut self, line: &str) {
        let Some((timestamp_ms, kind)) = classify_codex_timing_line(line) else {
            return;
        };
        match kind {
            CodexTimingLine::Boundary => {
                self.last_boundary_ms = self.last_boundary_ms.max(Some(timestamp_ms));
            }
            CodexTimingLine::ToolOutput => {
                self.last_boundary_ms = self.last_boundary_ms.max(Some(timestamp_ms));
                if self.last_model_output_ms.is_some() {
                    self.tool_output_after_model_output_ms = Some(timestamp_ms);
                }
            }
            CodexTimingLine::ModelOutput => {
                let trails_settlement = self.request_start_ms.is_none()
                    && self
                        .last_settle_ms
                        .is_some_and(|at| timestamp_ms - at <= CODEX_SAME_FLUSH_SLACK_MS);
                if trails_settlement {
                    return;
                }
                if self.request_start_ms.is_none() {
                    self.request_start_ms = self.last_boundary_ms;
                }
                self.last_model_output_ms = Some(timestamp_ms);
                self.tool_output_after_model_output_ms = None;
            }
            CodexTimingLine::OutputItemStarted(started_at_ms) => {
                if self.request_start_ms.is_none() {
                    self.request_start_ms = self.last_boundary_ms;
                }
                self.first_output_ms = Some(
                    self.first_output_ms
                        .map_or(started_at_ms, |prev| prev.min(started_at_ms)),
                );
            }
            CodexTimingLine::TurnReset => {
                *self = Self {
                    last_boundary_ms: self.last_boundary_ms.max(Some(timestamp_ms)),
                    last_settle_ms: self.last_settle_ms,
                    ..Self::default()
                };
            }
        }
    }

    fn finish_usage_record(&mut self, record_ms: Option<i64>) -> (Option<u64>, Option<u64>) {
        let start_ms = self.request_start_ms.or(self.last_boundary_ms);
        let first_output_ms = self.first_output_ms;
        *self = Self {
            last_boundary_ms: self.last_boundary_ms.max(record_ms),
            last_settle_ms: record_ms,
            ..Self::default()
        };
        let duration_ms = match (start_ms, record_ms) {
            (Some(start), Some(end)) => session_duration_ms(start, end),
            _ => None,
        };
        let first_token_ms = match (start_ms, first_output_ms, duration_ms) {
            (Some(start), Some(first), Some(duration)) => {
                session_first_token_ms(start, first, duration)
            }
            _ => None,
        };
        (duration_ms, first_token_ms)
    }

    fn finish_token_count(&mut self, token_count_ms: Option<i64>) -> (Option<u64>, Option<u64>) {
        let start_ms = self.request_start_ms.or(self.last_boundary_ms);
        let first_output_ms = self.first_output_ms;
        let waited_for_tools = match (self.tool_output_after_model_output_ms, token_count_ms) {
            (Some(tool_ms), Some(tc_ms)) => tc_ms - tool_ms <= CODEX_SAME_FLUSH_SLACK_MS,
            (Some(_), None) => true,
            (None, _) => false,
        };
        let end_ms = if waited_for_tools {
            self.last_model_output_ms
        } else {
            token_count_ms
        };
        *self = Self {
            last_boundary_ms: self.last_boundary_ms.max(token_count_ms),
            last_settle_ms: token_count_ms,
            ..Self::default()
        };
        let duration_ms = match (start_ms, end_ms) {
            (Some(start), Some(end)) => session_duration_ms(start, end),
            _ => None,
        };
        let first_token_ms = match (start_ms, first_output_ms, duration_ms) {
            (Some(start), Some(first), Some(duration)) => {
                session_first_token_ms(start, first, duration)
            }
            _ => None,
        };
        (duration_ms, first_token_ms)
    }

    /// A mirrored `token_count` echoes an already-settled `token_usage_record`
    /// (often right after tool outputs finish). Advance the boundary timestamp
    /// without clearing any in-flight next-request state.
    fn observe_mirrored_token_count(&mut self, token_count_ms: Option<i64>) {
        self.last_boundary_ms = self.last_boundary_ms.max(token_count_ms);
        if self.request_start_ms.is_none() {
            self.last_settle_ms = token_count_ms.or(self.last_settle_ms);
        }
    }
}

fn parse_codex_lines(
    reader: Box<dyn BufRead>,
    initial_thread_id: String,
    rollout_id: Option<&str>,
    fallback_timestamp: i64,
) -> Result<ParsedSession, String> {
    let mut parsed = ParsedSession::default();
    let mut thread_id = initial_thread_id;
    let mut model = "unknown".to_string();
    let mut high_water = Counters::default();
    let mut signatures_by_source = HashMap::<String, String>::new();
    let mut previous_signature = None;
    let mut event_index = 0u64;
    let mut metadata_seen = false;
    // Codex records the working directory once, in the leading `session_meta`.
    let mut project_path: Option<String> = None;
    // Dual-write bookkeeping; see `apply_codex_token_usage_record`.
    let mut response_ids = std::collections::HashSet::<String>::new();
    let mut pending_responses = Vec::<Counters>::new();
    let mut legacy_before_response: Option<(Counters, String)> = None;
    let mut timer = CodexRequestTimer::default();
    for line in reader.lines() {
        let line = line.map_err(|error| error.to_string())?;
        timer.observe_line(&line);
        if !line.contains("\"session_meta\"")
            && !line.contains("\"turn_context\"")
            && !line.contains("\"token_usage_record\"")
            && !(line.contains("\"event_msg\"")
                && (line.contains("\"token_count\"")
                    || line.contains("\"task_started\"")
                    || line.contains("\"turn_aborted\"")))
        {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(payload) = value.get("payload") else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("session_meta") if !metadata_seen => {
                metadata_seen = true;
                if let Some(id) = string(payload, &["/id", "/thread_id", "/threadId"]) {
                    thread_id = id;
                }
                parsed.started_at = timestamp(&value);
                project_path = string(payload, &["/cwd"]);
                parsed.parent_thread_id = string(
                    payload,
                    &[
                        "/forked_from_id",
                        "/parent_thread_id",
                        "/source/subagent/thread_spawn/parent_thread_id",
                        "/source/subagent/fork/parent_thread_id",
                    ],
                );
                if let Some(name) = string(payload, &["/model"]) {
                    model = name;
                }
            }
            Some("turn_context") => {
                if let Some(name) = string(payload, &["/model", "/info/model"]) {
                    model = name;
                }
            }
            Some("event_msg")
                if matches!(
                    payload.get("type").and_then(Value::as_str),
                    Some("task_started" | "turn_aborted")
                ) =>
            {
                // A new turn cannot inherit a twin from the previous one. Any
                // still-unmatched authoritative record is an orphan (its mirror
                // never arrived); keep it pending rather than risk pairing it
                // with the next turn's numbers.
                legacy_before_response = None;
            }
            Some("event_msg")
                if payload.get("type").and_then(Value::as_str) == Some("token_count") =>
            {
                let Some(info) = payload.get("info").filter(|value| value.is_object()) else {
                    continue;
                };
                let total = info.get("total_token_usage").and_then(Counters::parse);
                let last = info.get("last_token_usage").and_then(Counters::parse);
                if total.is_none() && last.is_none() {
                    continue;
                }
                if let Some(name) = string(info, &["/model", "/model_name"])
                    .or_else(|| string(payload, &["/model"]))
                {
                    model = name;
                }
                let signature = counter_signature(info);
                let lane = string(payload, &["/rate_limits/limit_id"]).unwrap_or_default();
                let duplicate = total.is_some()
                    && (signatures_by_source.get(&lane) == Some(&signature)
                        || previous_signature.as_ref() == Some(&signature));
                if total.is_some() {
                    signatures_by_source.insert(lane, signature.clone());
                }
                previous_signature = Some(signature.clone());
                // The legacy notification mirrors an authoritative
                // `token_usage_record` that already counted this response. The
                // record normally arrives first, so its usage block is in
                // `pending_responses`; a mirror that consumes one must not
                // produce a second row.
                let mirrored = last.as_ref().is_some_and(|last| {
                    pending_responses
                        .iter()
                        .position(|pending| pending == last)
                        .map(|index| pending_responses.remove(index))
                        .is_some()
                });
                let usage = if duplicate || mirrored {
                    TokenUsage::default()
                } else {
                    last.clone().unwrap_or_else(|| {
                        total
                            .as_ref()
                            .map(|value| value.delta(&high_water))
                            .unwrap_or_default()
                    })
                    .into_usage()
                };
                if let Some(total) = total {
                    high_water.update_high_water(&total);
                }
                let created_at = timestamp(&value).unwrap_or(fallback_timestamp);
                let (duration_ms, first_token_ms) = if usage.total_tokens().is_some() {
                    timer.finish_token_count(timestamp_millis(&value))
                } else if mirrored {
                    timer.observe_mirrored_token_count(timestamp_millis(&value));
                    (None, None)
                } else {
                    (None, None)
                };
                let request_id = usage.total_tokens().map(|_| {
                    event_index += 1;
                    // A paginated thread's pages all carry the same thread id
                    // in `session_meta`, so the thread id alone would give two
                    // pages the same key and the newer page would overwrite the
                    // older one. The rollout id keeps each page's events
                    // distinct — but an ordinary single-file rollout has
                    // rollout id == thread id, where adding it would only
                    // re-key rows that are already correct.
                    match rollout_id.filter(|rollout_id| *rollout_id != thread_id) {
                        Some(rollout_id) => {
                            format!("SESSION:codex:{thread_id}:{rollout_id}:token:{event_index}")
                        }
                        None => format!("SESSION:codex:{thread_id}:token:{event_index}"),
                    }
                });
                if let (Some(request_id), Some(last)) = (&request_id, last) {
                    // Remember that this call was counted before its
                    // authoritative record arrived; that record will adopt this
                    // row's identity instead of billing the same call again.
                    legacy_before_response = Some((last, request_id.clone()));
                }
                if let Some(request_id) = &request_id {
                    parsed.records.push(SessionUsageRecord {
                        metadata: Default::default(),
                        request_id: request_id.clone(),
                        legacy_request_ids: Vec::new(),
                        cli_key: GatewayUsageTool::Codex,
                        model: model.clone(),
                        usage,
                        created_at,
                        session_id: thread_id.clone(),
                        reported_cost_usd: None,
                        project_path: project_path.clone(),
                        credits: None,
                        duration_ms,
                        first_token_ms,
                    });
                }
                parsed.snapshots.push(CodexSnapshot {
                    signature,
                    timestamp: created_at,
                    request_id,
                });
            }
            Some("token_usage_record") => {
                let Some(usage) = payload.get("usage").and_then(Counters::parse) else {
                    continue;
                };
                let response_id = string(payload, &["/response_id"]);
                if let Some(response_id) = &response_id {
                    if !response_ids.insert(response_id.clone()) {
                        // A copied or repeated record: the same response cannot
                        // be billed twice.
                        continue;
                    }
                }
                let created_at = timestamp(&value).unwrap_or(fallback_timestamp);
                // A fork copies its parent's records wholesale; the thread id
                // names the owner, and only the owner may bill the usage.
                let owner_mismatch = string(payload, &["/thread_id", "/session_id"])
                    .is_some_and(|owner| owner != thread_id);
                if owner_mismatch {
                    // Still remember it, so a fork's legacy mirror — if one
                    // follows — is consumed instead of counted.
                    pending_responses.push(usage);
                    continue;
                }
                if let Some((mirror, counted_id)) = legacy_before_response.take() {
                    if mirror == usage {
                        // The legacy notification came first and was counted.
                        // Hijack its identity so an overlapping runtime segment
                        // (or a later scan) still sees one call.
                        if let Some(response_id) = &response_id {
                            if let Some(record) = parsed
                                .records
                                .iter_mut()
                                .find(|record| record.request_id == counted_id)
                            {
                                record
                                    .legacy_request_ids
                                    .push(record.request_id.clone());
                                record.request_id =
                                    format!("SESSION:codex:{thread_id}:{response_id}");
                                // The upgraded identity is the upstream
                                // response, so it also becomes the envelope the
                                // cross-source matcher looks for.
                                record.usage.envelope_id = Some(response_id.clone());
                            }
                        }
                        continue;
                    }
                }
                let token_usage = usage.clone().into_usage();
                if token_usage.total_tokens().is_none() {
                    continue;
                }
                pending_responses.push(usage);
                let (duration_ms, first_token_ms) =
                    timer.finish_usage_record(timestamp_millis(&value));
                // `usage` includes the cached and cache-write parts; the
                // importer's input bucket is the fresh remainder.
                let request_id = match &response_id {
                    Some(response_id) => format!("SESSION:codex:{thread_id}:{response_id}"),
                    None => {
                        event_index += 1;
                        format!("SESSION:codex:{thread_id}:token_usage:{event_index}")
                    }
                };
                // The response id is the upstream envelope the gateway stamped
                // on its own row. Recording it here gives the cross-source
                // matcher an exact identity, instead of relying on equal token
                // counts and a successful terminal (issue #432).
                let usage = TokenUsage {
                    envelope_id: response_id.clone(),
                    ..token_usage
                };
                parsed.records.push(SessionUsageRecord {
                    metadata: Default::default(),
                    request_id,
                    legacy_request_ids: Vec::new(),
                    cli_key: GatewayUsageTool::Codex,
                    model: model.clone(),
                    usage,
                    created_at,
                    session_id: thread_id.clone(),
                    reported_cost_usd: None,
                    project_path: project_path.clone(),
                    credits: None,
                    duration_ms,
                    first_token_ms,
                });
            }
            _ => {}
        }
    }
    Ok(parsed)
}

fn counter_signature(info: &Value) -> String {
    let counters = |name: &str| {
        info.get(name).and_then(Value::as_object).map(|fields| {
            [
                "input_tokens",
                "cached_input_tokens",
                "cache_read_input_tokens",
                "cache_write_input_tokens",
                "output_tokens",
                "reasoning_output_tokens",
                "total_tokens",
            ]
            .into_iter()
            .filter_map(|key| {
                fields
                    .get(key)
                    .map(|value| (key.to_string(), value.clone()))
            })
            .collect::<BTreeMap<_, _>>()
        })
    };
    let bytes = serde_json::to_vec(&(counters("total_token_usage"), counters("last_token_usage")))
        .unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn exclude_codex_replay(parsed: &mut ParsedSession, parent: &[CodexSnapshot]) {
    let cutoff = parsed.started_at.unwrap_or(i64::MAX);
    let parent = parent
        .iter()
        .filter(|event| event.timestamp <= cutoff)
        .collect::<Vec<_>>();
    let replay_length = (0..parent.len())
        .map(|start| {
            parsed
                .snapshots
                .iter()
                .zip(parent[start..].iter())
                .take_while(|(child, parent)| child.signature == parent.signature)
                .count()
        })
        .max()
        .unwrap_or(0);
    let inherited_ids = parsed
        .snapshots
        .iter()
        .take(replay_length)
        .filter_map(|snapshot| snapshot.request_id.as_ref())
        .collect::<std::collections::HashSet<_>>();
    parsed
        .records
        .retain(|record| !inherited_ids.contains(&record.request_id));
}

fn usage_candidate(value: &Value) -> Option<&Value> {
    if value.get("usage").is_some()
        || value.get("usageMetadata").is_some()
        || value.pointer("/message/usage").is_some()
        || value.pointer("/response/usage").is_some()
    {
        return Some(value);
    }
    ["/response", "/message", "/payload", "/data"]
        .iter()
        .find_map(|path| {
            value.pointer(path).filter(|candidate| {
                candidate.get("usage").is_some() || candidate.get("usageMetadata").is_some()
            })
        })
}

pub(super) fn number(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_u64))
        .unwrap_or(0)
}

/// OpenCode 1.x marks assistant rows with `role`; 2.0 stores the kind in the
/// `session_message.type` column instead, and its schema gives only the
/// assistant kind a `tokens` field.
pub(super) fn is_opencode_usage_row(value: &Value) -> bool {
    value.get("role").and_then(Value::as_str) == Some("assistant")
        || value.get("type").and_then(Value::as_str) == Some("assistant")
}

pub(super) fn string(value: &Value, paths: &[&str]) -> Option<String> {
    paths
        .iter()
        .filter_map(|path| value.pointer(path).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn timestamp(value: &Value) -> Option<i64> {
    timestamp_millis(value).map(|value| value / 1000)
}

/// The same timestamps as [`timestamp`], kept at millisecond precision.
///
/// Sub-second timing matters here: two events of one request sit fractions of
/// a second apart, and truncating them to whole seconds would erase the
/// difference the duration estimate is built from.
pub(super) fn timestamp_millis(value: &Value) -> Option<i64> {
    [
        "/timestamp",
        "/created_at",
        "/createdAt",
        "/time/completed",
        "/time/created",
        "/time",
        "/ts",
        "/at",
        "/message/created_at",
    ]
    .iter()
    .find_map(|path| {
        let value = value.pointer(path)?;
        if let Some(number) = value.as_i64() {
            return Some(if number > 100_000_000_000 {
                number
            } else {
                number * 1000
            });
        }
        if let Some(number) = value.as_f64().filter(|value| value.is_finite()) {
            return Some(if number > 100_000_000_000.0 {
                number.round() as i64
            } else {
                (number * 1000.0).round() as i64
            });
        }
        chrono::DateTime::parse_from_rfc3339(value.as_str()?)
            .ok()
            .map(|value| value.timestamp_millis())
    })
}


pub(super) fn read_jsonl(path: &Path, visit: impl FnMut(usize, Value)) -> Result<bool, String> {
    read_jsonl_with_policy(path, false, visit)
}

/// Tolerant variant for append-then-compact transcripts: an unparseable row in
/// the middle of the file is skipped instead of failing the whole read.
///
/// Antigravity's `transcript_full.jsonl` is rewritten by compaction, so a crash
/// mid-flush can leave a broken row anywhere. `session_manager::antigravity`
/// reads the same file with the same skip semantics; keeping both sides aligned
/// avoids one surface importing a session the other renders fine.
pub(super) fn read_jsonl_lenient(
    path: &Path,
    visit: impl FnMut(usize, Value),
) -> Result<bool, String> {
    read_jsonl_with_policy(path, true, visit)
}

fn read_jsonl_with_policy(
    path: &Path,
    lenient: bool,
    mut visit: impl FnMut(usize, Value),
) -> Result<bool, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut reader: Box<dyn BufRead> = if path
        .extension()
        .is_some_and(|ext| ext == "zstd" || ext == "zst")
    {
        Box::new(BufReader::new(
            zstd::stream::read::Decoder::new(file).map_err(|error| error.to_string())?,
        ))
    } else {
        Box::new(BufReader::new(file))
    };
    let mut line = String::new();
    let mut index = 0;
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(false),
            Ok(_) => {
                match serde_json::from_str(&line) {
                    Ok(value) => visit(index, value),
                    Err(_) if !line.ends_with('\n') => return Ok(true),
                    Err(_) if lenient => {}
                    Err(error) if !line.trim().is_empty() => {
                        return Err(format!(
                            "Invalid session record at line {}: {error}",
                            index + 1
                        ))
                    }
                    Err(_) => {}
                }
                index += 1;
            }
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(true),
            Err(error) => return Err(error.to_string()),
        }
    }
}

pub(super) fn native_record(
    tool: GatewayUsageTool,
    session: &str,
    identity: &str,
    model: Option<String>,
    usage: TokenUsage,
    created_at: i64,
) -> SessionUsageRecord {
    SessionUsageRecord {
        metadata: Default::default(),
        request_id: format!("SESSION:{}:{session}:{identity}", tool.as_str()),
        legacy_request_ids: Vec::new(),
        cli_key: tool,
        model: model.unwrap_or_else(|| "unknown".into()),
        usage,
        created_at,
        session_id: session.into(),
        reported_cost_usd: None,
        project_path: None,
        credits: None,
        duration_ms: None,
        first_token_ms: None,
    }
}

pub(super) fn reported_cost(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let text = value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string());
    let parsed = text.parse::<rust_decimal::Decimal>().ok()?;
    (parsed > rust_decimal::Decimal::ZERO).then(|| parsed.to_string())
}
