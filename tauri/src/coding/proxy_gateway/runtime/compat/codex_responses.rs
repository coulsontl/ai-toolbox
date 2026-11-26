//! Third-party native OpenAI Responses passthrough compatibility.
//!
//! Applied only when `source == target == OpenAiResponses` and the provider is
//! NOT `CodexOfficial`:
//! - Strips Codex 0.163+/0.167+ OpenAI-only request fields (`include`
//!   `output.encrypted_content`, `stream_options.reasoning_summary_delivery`,
//!   top-level `access_programs`, `input[]` `configuration_update` items,
//!   `internal_chat_message_metadata_passthrough`, non-reasoning/non-compaction
//!   `encrypted_content`, and `function_call.encrypted_function_args`).
//! - Rewrites call_id-less `function_call_output` items into user messages
//!   preserving text/image/file content.
//! - Regroups interleaved developer/system notices out of open parallel
//!   tool-call batches until the batch closes (or a user turn boundary).
//! - For strict Moonshot/DeepSeek native Responses upstreams, drops empty
//!   assistant text messages and hoists mid-batch assistant messages before the
//!   batch's first tool call.
//! - Deduplicates duplicate JSON object keys (last-value-wins) inside
//!   historical `function_call.arguments` strings.
//! - Wraps Responses SSE streams with a protocol-violation-triggered adapter
//!   that holds empty `function_call_arguments.done` / `output_item.done`
//!   events and backfills them from late deltas or `response.completed`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::pin::Pin;

use futures_util::StreamExt;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{json, Map, Value};

use super::super::http_io::DebugBodyStream;
use super::provider_kind::ProviderBodyCompat;
use crate::coding::proxy_gateway::transformer::{append_utf8_safe, strip_sse_field};
use crate::coding::proxy_gateway::usage_parser::flattened_flush_boundary;

const CODEX_PRIVATE_INCLUDE_MARKERS: &[&str] = &["output.encrypted_content"];
const ORPHAN_TOOL_OUTPUT_MARKER: &str = "[tool_result without a matching tool_call]";
const MESSAGE_CONTENT_PART_TYPES: &[&str] = &["input_text", "input_image", "input_file"];
const STRICT_CALL_TYPES: &[&str] = &["function_call", "custom_tool_call"];
const STRICT_OUTPUT_TYPES: &[&str] = &["function_call_output", "custom_tool_call_output"];

/// Apply all third-party native Responses request rewrites in a fixed,
/// deterministic order. Never called for `ProviderBodyCompat::CodexOfficial`.
pub(crate) fn apply_third_party_native_responses_request_compat(
    body: &mut Value,
    provider_kind: Option<ProviderBodyCompat>,
) -> bool {
    let mut changed = false;
    changed |= strip_codex_openai_only_request_fields(body);
    changed |= rewrite_call_id_less_tool_outputs(body);
    changed |= normalize_tool_output_grouping(body);
    if matches!(
        provider_kind,
        Some(ProviderBodyCompat::Moonshot | ProviderBodyCompat::DeepSeek)
    ) {
        changed |= repair_strict_tool_history(body);
    }
    changed |= dedup_responses_function_call_arguments(body);
    changed
}

/// Strip Codex-private `stream_options.reasoning_summary_delivery` from a
/// request object (shared by Chat and Responses outbound paths).
pub(crate) fn strip_reasoning_summary_delivery_stream_option(
    obj: &mut Map<String, Value>,
) -> bool {
    let mut changed = false;
    let mut stream_options_empty = false;
    if let Some(stream_options) = obj.get_mut("stream_options").and_then(Value::as_object_mut) {
        if stream_options.remove("reasoning_summary_delivery").is_some() {
            changed = true;
            stream_options_empty = stream_options.is_empty();
        }
    }
    if stream_options_empty {
        obj.remove("stream_options");
    }
    changed
}

/// Strip OpenAI-backend-only fields that Codex 0.163+/0.167+ emits when it
/// thinks it is talking to official OpenAI Responses (cc-switch 08ec82aa).
pub(crate) fn strip_codex_openai_only_request_fields(body: &mut Value) -> bool {
    let Some(obj) = body.as_object_mut() else {
        return false;
    };
    let mut changed = strip_codex_private_include_markers(obj);
    changed |= strip_reasoning_summary_delivery_stream_option(obj);
    changed |= obj.remove("access_programs").is_some();

    if let Some(input) = obj.get_mut("input").and_then(Value::as_array_mut) {
        let before = input.len();
        input.retain(|item| {
            item.get("type").and_then(Value::as_str) != Some("configuration_update")
        });
        changed |= input.len() != before;

        for item in input.iter_mut() {
            let Some(item_obj) = item.as_object_mut() else {
                continue;
            };
            changed |= item_obj
                .remove("internal_chat_message_metadata_passthrough")
                .is_some();
            let item_type = item_obj
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let keeps_encrypted_content = matches!(item_type, "reasoning" | "compaction");
            let is_function_call = item_type == "function_call";
            if !keeps_encrypted_content {
                changed |= item_obj.remove("encrypted_content").is_some();
            }
            if is_function_call {
                changed |= item_obj.remove("encrypted_function_args").is_some();
            }
        }
    }

    changed
}

fn strip_codex_private_include_markers(obj: &mut Map<String, Value>) -> bool {
    let mut remove_include = false;
    let changed = match obj.get_mut("include").and_then(Value::as_array_mut) {
        Some(include) => {
            let before = include.len();
            include.retain(|item| {
                !item
                    .as_str()
                    .is_some_and(|marker| CODEX_PRIVATE_INCLUDE_MARKERS.contains(&marker))
            });
            let changed = include.len() != before;
            remove_include = changed && include.is_empty();
            changed
        }
        None => false,
    };
    if remove_include {
        obj.remove("include");
    }
    changed
}

/// Rewrite top-level `function_call_output` items lacking a non-empty `call_id`
/// into ordinary user `message` items so strict third-party Responses gateways
/// do not reject the thread (cc-switch 2b0ec81b).
pub(crate) fn rewrite_call_id_less_tool_outputs(body: &mut Value) -> bool {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    for item in input.iter_mut() {
        if item.get("type").and_then(Value::as_str).map(str::trim) != Some("function_call_output") {
            continue;
        }
        let has_call_id = item
            .get("call_id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.trim().is_empty());
        if has_call_id {
            continue;
        }
        *item = json!({
            "type": "message",
            "role": "user",
            "content": orphan_tool_output_content(item.get("output")),
        });
        changed = true;
    }
    changed
}

fn orphan_tool_output_content(output: Option<&Value>) -> Vec<Value> {
    match output {
        Some(Value::String(text)) if !text.is_empty() => {
            vec![json!({
                "type": "input_text",
                "text": format!("{ORPHAN_TOOL_OUTPUT_MARKER}\n{text}")
            })]
        }
        Some(Value::Array(parts)) => {
            let mut content = vec![json!({
                "type": "input_text",
                "text": ORPHAN_TOOL_OUTPUT_MARKER
            })];
            content.extend(parts.iter().map(|part| {
                if part
                    .get("type")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .is_some_and(|ty| MESSAGE_CONTENT_PART_TYPES.contains(&ty))
                {
                    part.clone()
                } else {
                    json!({
                        "type": "input_text",
                        "text": part.to_string()
                    })
                }
            }));
            content
        }
        Some(Value::Null | Value::String(_)) | None => {
            vec![json!({
                "type": "input_text",
                "text": ORPHAN_TOOL_OUTPUT_MARKER
            })]
        }
        Some(other) => vec![json!({
            "type": "input_text",
            "text": format!("{ORPHAN_TOOL_OUTPUT_MARKER}\n{other}")
        })],
    }
}

fn call_id_of(item: &Value) -> Option<&str> {
    item.get("call_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
}

fn message_role(item: &Value) -> Option<&str> {
    if item.get("type").is_none_or(|kind| kind == "message") {
        item.get("role").and_then(Value::as_str)
    } else {
        None
    }
}

fn is_user_message(item: &Value) -> bool {
    message_role(item) == Some("user")
}

fn is_tool_output_kind(kind: &str) -> bool {
    matches!(
        kind,
        "function_call_output"
            | "custom_tool_call_output"
            | "local_shell_call_output"
            | "tool_search_output"
    )
}

fn tool_output_positions(
    turn: &[Value],
    is_output: impl Fn(&str) -> bool,
) -> HashMap<String, usize> {
    turn.iter()
        .enumerate()
        .filter(|(_, item)| item.get("type").and_then(Value::as_str).is_some_and(&is_output))
        .filter_map(|(index, item)| Some((call_id_of(item)?.to_owned(), index)))
        .collect()
}

/// Defer developer/system notices interleaved inside open parallel tool-call
/// batches until the batch closes (stopping at user turn boundaries)
/// (cc-switch 1fba4e16).
pub(crate) fn normalize_tool_output_grouping(body: &mut Value) -> bool {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut original = std::mem::take(input);
    let mut result = Vec::with_capacity(original.len());
    let mut deferred: Vec<Value> = Vec::new();
    let mut changed = false;

    for turn in original.split_inclusive_mut(is_user_message) {
        let output_positions = tool_output_positions(turn, is_tool_output_kind);
        let mut pending = HashSet::new();
        for (index, slot) in turn.iter_mut().enumerate() {
            let item = slot.take();
            let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
            match kind {
                "function_call" | "custom_tool_call" | "local_shell_call" | "tool_search_call" => {
                    if let Some(id) = call_id_of(&item) {
                        if output_positions
                            .get(id)
                            .is_some_and(|&output| output > index)
                        {
                            pending.insert(id.to_owned());
                        }
                    }
                }
                kind if is_tool_output_kind(kind) => {
                    if let Some(id) = call_id_of(&item) {
                        pending.remove(id);
                    }
                }
                _ if !pending.is_empty()
                    && matches!(message_role(&item), Some("developer" | "system")) =>
                {
                    deferred.push(item);
                    continue;
                }
                _ => {}
            }
            changed |= !deferred.is_empty();
            result.push(item);
            if pending.is_empty() {
                result.append(&mut deferred);
            }
        }
        if !deferred.is_empty() {
            result.append(&mut deferred);
        }
    }
    *input = result;
    changed
}

/// Strict Moonshot/DeepSeek native Responses history repair: drop empty-text
/// assistant messages and hoist mid-batch assistant commentary before the
/// batch's first tool call (cc-switch 1fba4e16).
pub(crate) fn repair_strict_tool_history(body: &mut Value) -> bool {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };
    if !input.iter().any(is_assistant_message) {
        return false;
    }

    let mut changed = false;
    let mut output: Vec<Value> = Vec::with_capacity(input.len());
    let mut original = std::mem::take(input);

    for turn in original.split_inclusive_mut(is_user_message) {
        let output_positions = tool_output_positions(turn, |kind| STRICT_OUTPUT_TYPES.contains(&kind));
        let mut pending = HashSet::new();
        let mut hoist_at = 0;

        for (index, slot) in turn.iter_mut().enumerate() {
            let item = slot.take();
            let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
            if is_assistant_message(&item) {
                if is_empty_text_message(&item) {
                    changed = true;
                    continue;
                }
                if !pending.is_empty() {
                    output.insert(hoist_at, item);
                    hoist_at += 1;
                    changed = true;
                    continue;
                }
            } else if STRICT_CALL_TYPES.contains(&item_type) {
                if pending.is_empty() {
                    hoist_at = output.len();
                }
                if let Some(id) = call_id_of(&item) {
                    if output_positions
                        .get(id)
                        .is_some_and(|&output_idx| output_idx > index)
                    {
                        pending.insert(id.to_owned());
                    }
                }
            } else if STRICT_OUTPUT_TYPES.contains(&item_type) {
                if let Some(id) = call_id_of(&item) {
                    pending.remove(id);
                }
            }
            output.push(item);
        }
    }

    *input = output;
    changed
}

fn is_assistant_message(item: &Value) -> bool {
    item.get("role").and_then(Value::as_str) == Some("assistant")
        && item
            .get("type")
            .and_then(Value::as_str)
            .is_none_or(|item_type| item_type == "message")
}

fn is_empty_text_message(item: &Value) -> bool {
    match item.get("content") {
        Some(Value::String(text)) => text.is_empty(),
        Some(Value::Array(parts)) => {
            !parts.is_empty()
                && parts.iter().all(|part| {
                    matches!(
                        part.get("type").and_then(Value::as_str),
                        Some("output_text" | "input_text" | "text")
                    ) && part
                        .get("text")
                        .and_then(Value::as_str)
                        .is_some_and(str::is_empty)
                })
        }
        _ => false,
    }
}

/// Deduplicate duplicate JSON object keys (last-value-wins) inside historical
/// `function_call.arguments` strings in `input[]` (cc-switch / Codex replay).
/// Only re-serializes strings that actually contained a duplicate key.
pub(crate) fn dedup_responses_function_call_arguments(body: &mut Value) -> bool {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    for item in input.iter_mut() {
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            continue;
        }
        let Some(arguments) = item.get("arguments").and_then(Value::as_str) else {
            continue;
        };
        if let Some(deduped) = dedup_json_object_keys_if_needed(arguments) {
            if let Some(obj) = item.as_object_mut() {
                obj.insert("arguments".to_string(), Value::String(deduped));
                changed = true;
            }
        }
    }
    changed
}

fn dedup_json_object_keys_if_needed(raw: &str) -> Option<String> {
    let mut found_duplicate = false;
    let mut deserializer = serde_json::Deserializer::from_str(raw);
    let value = DedupSeed {
        found_duplicate: &mut found_duplicate,
    }
    .deserialize(&mut deserializer)
    .ok()?;
    deserializer.end().ok()?;
    if !found_duplicate {
        return None;
    }
    serde_json::to_string(&value).ok()
}

struct DedupSeed<'a> {
    found_duplicate: &'a mut bool,
}

impl<'de, 'a> DeserializeSeed<'de> for DedupSeed<'a> {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(DedupVisitor {
            found_duplicate: self.found_duplicate,
        })
    }
}

struct DedupVisitor<'a> {
    found_duplicate: &'a mut bool,
}

impl<'de, 'a> Visitor<'de> for DedupVisitor<'a> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any valid JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
        Ok(Value::from(v))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
        Ok(Value::from(v))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E> {
        Ok(serde_json::Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(v.to_string()))
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
        Ok(Value::String(v))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut items = Vec::new();
        while let Some(elem) = seq.next_element_seed(DedupSeed {
            found_duplicate: self.found_duplicate,
        })? {
            items.push(elem);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            let value = map.next_value_seed(DedupSeed {
                found_duplicate: self.found_duplicate,
            })?;
            if object.contains_key(&key) {
                *self.found_duplicate = true;
            }
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

// ============================================================================
// Delimiter-preserving SSE helpers (shared with xai_responses)
// ============================================================================

pub(super) fn take_sse_block_preserving_delimiter(
    buffer: &mut String,
) -> Option<(String, &'static str)> {
    const DELIMITERS: [&str; 4] = ["\r\n\r\n", "\n\n", "\r\n\n", "\n\r\n"];
    let (idx, delimiter) = DELIMITERS
        .into_iter()
        .filter_map(|delim| buffer.find(delim).map(|pos| (pos, delim)))
        .min_by_key(|(pos, _)| *pos)?;
    let block = buffer[..idx].to_string();
    buffer.replace_range(..idx + delimiter.len(), "");
    Some((block, delimiter))
}

pub(super) fn rewrite_sse_block_data_preserving_framing(
    block: &str,
    delimiter: &str,
    new_data: &str,
) -> Vec<u8> {
    let mut out = String::with_capacity(block.len() + new_data.len() + delimiter.len());
    let mut wrote_data = false;
    let mut rest = block;
    while !rest.is_empty() {
        let (line, ending, next) = match rest.find('\n') {
            Some(pos) => {
                if pos > 0 && rest.as_bytes()[pos - 1] == b'\r' {
                    (&rest[..pos - 1], "\r\n", &rest[pos + 1..])
                } else {
                    (&rest[..pos], "\n", &rest[pos + 1..])
                }
            }
            None => (rest, "", ""),
        };
        if strip_sse_field(line, "data").is_some() {
            if !wrote_data {
                out.push_str("data: ");
                out.push_str(new_data);
                out.push_str(ending);
                wrote_data = true;
            }
        } else {
            out.push_str(line);
            out.push_str(ending);
        }
        rest = next;
    }
    if !wrote_data {
        out.push_str("data: ");
        out.push_str(new_data);
    }
    out.push_str(delimiter);
    out.into_bytes()
}

/// True only when `buffer` exhibits degenerate single-line flattened SSE framing
/// (issue #318/#404: `event: ... data: {...}` or multiple `data:` tokens on one
/// line without `\n`), NOT when a normal `data: {...}` chunk is merely waiting
/// for its trailing `\n\n`.
pub(super) fn is_degenerate_flattened_sse_line(buffer: &str) -> bool {
    if buffer.is_empty() || buffer.contains('\n') {
        return false;
    }
    if buffer.contains("event:") && buffer.contains("data:") {
        return true;
    }
    if let Some(first_data) = buffer.find("data:") {
        if buffer[first_data + 5..].contains("data:") {
            return true;
        }
    }
    false
}

// ============================================================================
// Late tool-argument SSE repair (cc-switch 3459658f / f5a4bdc9)
// ============================================================================

const MAX_HELD_LATE_ARG_BLOCKS: usize = 32;
const MAX_TRACKED_LATE_ARG_BYTES: usize = 2 * 1024 * 1024;

struct LateArgsBlock {
    raw: String,
    delimiter: &'static str,
    event: Option<Value>,
}

impl LateArgsBlock {
    fn parse(raw: &str, delimiter: &'static str) -> Self {
        let mut data_parts = Vec::new();
        for line in raw.lines() {
            if let Some(data) = strip_sse_field(line, "data") {
                data_parts.push(data);
            }
        }
        let event = (!data_parts.is_empty())
            .then(|| serde_json::from_str::<Value>(&data_parts.join("\n")).ok())
            .flatten();
        Self {
            raw: raw.to_string(),
            delimiter,
            event,
        }
    }

    fn event_type(&self) -> Option<&str> {
        self.event.as_ref()?.get("type")?.as_str()
    }

    fn empty_function_call_end(&self) -> Option<(String, Option<String>)> {
        let event = self.event.as_ref()?;
        let (primary_id, call_id, arguments) = match self.event_type()? {
            "response.function_call_arguments.done" => (
                event.get("item_id").or_else(|| event.get("call_id"))?,
                event.get("call_id").and_then(Value::as_str).map(str::to_string),
                event.get("arguments"),
            ),
            "response.output_item.done" => {
                let item = event.get("item")?;
                if item.get("type")?.as_str()? != "function_call" {
                    return None;
                }
                let primary = item
                    .get("id")
                    .or_else(|| event.get("item_id"))
                    .or_else(|| item.get("call_id"))?;
                let cid = item.get("call_id").and_then(Value::as_str).map(str::to_string);
                (primary, cid, item.get("arguments"))
            }
            _ => return None,
        };
        let empty = arguments
            .and_then(Value::as_str)
            .is_none_or(|arguments| arguments.is_empty());
        empty
            .then(|| primary_id.as_str().map(|id| (id.to_string(), call_id)))
            .flatten()
    }

    fn completed_non_empty_function_call_id(&self) -> Option<String> {
        if self.event_type()? != "response.output_item.done" {
            return None;
        }
        let item = self.event.as_ref()?.get("item")?;
        if item.get("type")?.as_str()? != "function_call" {
            return None;
        }
        let has_args = item
            .get("arguments")
            .and_then(Value::as_str)
            .is_some_and(|args| !args.is_empty());
        has_args
            .then(|| {
                item.get("id")
                    .or_else(|| item.get("call_id"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .flatten()
    }

    fn arguments_delta(&self) -> Option<(String, &str)> {
        if self.event_type()? != "response.function_call_arguments.delta" {
            return None;
        }
        let event = self.event.as_ref()?;
        let id = event
            .get("item_id")
            .or_else(|| event.get("call_id"))?
            .as_str()?
            .to_string();
        Some((id, event.get("delta")?.as_str()?))
    }

    fn with_arguments(mut self, arguments: &str) -> Vec<u8> {
        let Some(event) = self.event.as_mut() else {
            return self.into_bytes();
        };
        let target =
            if event.get("type").and_then(Value::as_str) == Some("response.output_item.done") {
                event.get_mut("item")
            } else {
                Some(&mut *event)
            };
        let Some(target) = target.and_then(Value::as_object_mut) else {
            return self.into_bytes();
        };
        target.insert(
            "arguments".to_string(),
            Value::String(arguments.to_string()),
        );
        let serialized = serde_json::to_string(event).unwrap_or_default();
        rewrite_sse_block_data_preserving_framing(&self.raw, self.delimiter, &serialized)
    }

    fn into_bytes(self) -> Vec<u8> {
        format!("{}{}", self.raw, self.delimiter).into_bytes()
    }
}

struct HeldLateArgsBlock {
    item_id: String,
    call_id: Option<String>,
    block: LateArgsBlock,
}

#[derive(Default)]
struct LateArgumentsRepair {
    arguments: HashMap<String, String>,
    tracked_arg_bytes: usize,
    held: Vec<HeldLateArgsBlock>,
}

impl LateArgumentsRepair {
    fn push(&mut self, raw: &str, delimiter: &'static str) -> Vec<Vec<u8>> {
        let block = LateArgsBlock::parse(raw, delimiter);
        let mut out = Vec::new();

        if let Some((item_id, delta)) = block.arguments_delta() {
            if self.tracked_arg_bytes.saturating_add(delta.len()) <= MAX_TRACKED_LATE_ARG_BYTES {
                self.arguments
                    .entry(item_id)
                    .or_default()
                    .push_str(delta);
                self.tracked_arg_bytes = self.tracked_arg_bytes.saturating_add(delta.len());
            }
            out.push(block.into_bytes());
            return out;
        }

        if let Some((item_id, call_id)) = block.empty_function_call_end() {
            if self.held.len() >= MAX_HELD_LATE_ARG_BLOCKS {
                out.extend(self.flush(None));
            }
            self.held.push(HeldLateArgsBlock {
                item_id,
                call_id,
                block,
            });
            return out;
        }

        if let Some(done_id) = block.completed_non_empty_function_call_id() {
            let is_held = self.held.iter().any(|h| h.item_id == done_id);
            if !is_held {
                if let Some(removed) = self.arguments.remove(&done_id) {
                    self.tracked_arg_bytes = self.tracked_arg_bytes.saturating_sub(removed.len());
                }
            }
        }

        let is_terminal = matches!(
            block.event_type(),
            Some(
                "response.completed"
                    | "response.incomplete"
                    | "response.failed"
                    | "response.cancelled"
                    | "response.canceled"
            )
        );
        if is_terminal {
            let completed = block
                .event
                .as_ref()
                .filter(|_| block.event_type() == Some("response.completed"));
            out.extend(self.flush(completed));
        }
        out.push(block.into_bytes());
        out
    }

    fn flush(&mut self, completed: Option<&Value>) -> Vec<Vec<u8>> {
        let out = std::mem::take(&mut self.held)
            .into_iter()
            .map(|entry| {
                let arguments = self
                    .arguments
                    .get(&entry.item_id)
                    .filter(|arguments| !arguments.is_empty())
                    .or_else(|| {
                        entry
                            .call_id
                            .as_deref()
                            .and_then(|cid| self.arguments.get(cid))
                            .filter(|arguments| !arguments.is_empty())
                    })
                    .cloned()
                    .or_else(|| {
                        completed_arguments(completed?, &entry.item_id, entry.call_id.as_deref())
                    });
                match arguments {
                    Some(arguments) => entry.block.with_arguments(&arguments),
                    None => entry.block.into_bytes(),
                }
            })
            .collect();
        if completed.is_some() {
            self.arguments.clear();
            self.tracked_arg_bytes = 0;
        }
        out
    }
}

fn completed_arguments(
    completed: &Value,
    item_id: &str,
    call_id: Option<&str>,
) -> Option<String> {
    completed
        .pointer("/response/output")?
        .as_array()?
        .iter()
        .find(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call")
                && (item.get("id").and_then(Value::as_str) == Some(item_id)
                    || call_id.is_some_and(|cid| {
                        item.get("call_id").and_then(Value::as_str) == Some(cid)
                    }))
        })?
        .get("arguments")?
        .as_str()
        .filter(|arguments| !arguments.is_empty())
        .map(str::to_string)
}

/// Wrap a third-party native Responses SSE stream so empty
/// `function_call_arguments.done` / `output_item.done` events are held until
/// late deltas or `response.completed` supply the arguments.
pub(crate) fn wrap_late_arguments_repair_sse_stream(stream: DebugBodyStream) -> DebugBodyStream {
    struct State {
        inner: DebugBodyStream,
        buffer: String,
        utf8_remainder: Vec<u8>,
        repair: LateArgumentsRepair,
        pending: VecDeque<Result<Vec<u8>, String>>,
        finished: bool,
    }

    Box::pin(futures_util::stream::unfold(
        State {
            inner: stream,
            buffer: String::new(),
            utf8_remainder: Vec::new(),
            repair: LateArgumentsRepair::default(),
            pending: VecDeque::new(),
            finished: false,
        },
        |mut state| async move {
            loop {
                if let Some(chunk) = state.pending.pop_front() {
                    return Some((chunk, state));
                }
                if state.finished {
                    return None;
                }
                match state.inner.next().await {
                    Some(Ok(bytes)) => {
                        append_utf8_safe(&mut state.buffer, &mut state.utf8_remainder, &bytes);
                        while let Some((block, delimiter)) =
                            take_sse_block_preserving_delimiter(&mut state.buffer)
                        {
                            if block.trim().is_empty()
                                || block.lines().all(|line| {
                                    let trimmed = line.trim();
                                    trimmed.is_empty() || trimmed.starts_with(':')
                                })
                            {
                                let mut raw = block.into_bytes();
                                raw.extend_from_slice(delimiter.as_bytes());
                                state.pending.push_back(Ok(raw));
                                continue;
                            }
                            for out in state.repair.push(&block, delimiter) {
                                state.pending.push_back(Ok(out));
                            }
                        }
                        // Preserve streaming for degenerate flattened SSE relays
                        // (issue #318/#404) without intercepting normal single
                        // `data: {...}` chunks waiting for their `\n\n`.
                        if state.repair.held.is_empty()
                            && is_degenerate_flattened_sse_line(&state.buffer)
                        {
                            let boundary = flattened_flush_boundary(state.buffer.as_bytes());
                            if boundary > 0 && state.buffer.is_char_boundary(boundary) {
                                let passthrough: String = state.buffer.drain(..boundary).collect();
                                state.pending.push_back(Ok(passthrough.into_bytes()));
                            }
                        }
                    }
                    Some(Err(error)) => return Some((Err(error), state)),
                    None => {
                        state.finished = true;
                        if !state.utf8_remainder.is_empty() {
                            state
                                .buffer
                                .push_str(&String::from_utf8_lossy(&state.utf8_remainder));
                            state.utf8_remainder.clear();
                        }
                        let tail = std::mem::take(&mut state.buffer);
                        if !tail.is_empty() {
                            if tail.trim().is_empty() {
                                for out in state.repair.flush(None) {
                                    state.pending.push_back(Ok(out));
                                }
                                state.pending.push_back(Ok(tail.into_bytes()));
                            } else {
                                for out in state.repair.push(&tail, "") {
                                    state.pending.push_back(Ok(out));
                                }
                            }
                        }
                        for out in state.repair.flush(None) {
                            state.pending.push_back(Ok(out));
                        }
                    }
                }
            }
        },
    )) as Pin<Box<dyn futures_util::Stream<Item = Result<Vec<u8>, String>> + Send + 'static>>
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    #[test]
    fn strip_codex_openai_only_fields_preserves_reasoning_and_compaction_encrypted_content() {
        let mut body = json!({
            "model": "kimi-k2.5",
            "include": ["reasoning.encrypted_content", "output.encrypted_content"],
            "stream_options": {"reasoning_summary_delivery": "auto"},
            "access_programs": ["codex"],
            "input": [
                {"type": "configuration_update", "reasoning": {"effort": "high"}},
                {"type": "reasoning", "encrypted_content": "keep-reasoning", "internal_chat_message_metadata_passthrough": {"x": 1}},
                {"type": "compaction", "encrypted_content": "keep-compaction"},
                {"type": "message", "role": "assistant", "encrypted_content": "drop-me", "content": [{"type": "output_text", "text": "ok"}]},
                {"type": "function_call", "call_id": "c1", "name": "run", "arguments": "{}", "encrypted_function_args": "secret", "encrypted_content": "drop"}
            ]
        });

        assert!(strip_codex_openai_only_request_fields(&mut body));
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert!(body.get("stream_options").is_none());
        assert!(body.get("access_programs").is_none());
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 4);
        assert_eq!(input[0]["encrypted_content"], "keep-reasoning");
        assert!(input[0].get("internal_chat_message_metadata_passthrough").is_none());
        assert_eq!(input[1]["encrypted_content"], "keep-compaction");
        assert!(input[2].get("encrypted_content").is_none());
        assert!(input[3].get("encrypted_function_args").is_none());
        assert!(input[3].get("encrypted_content").is_none());
    }

    #[test]
    fn rewrite_call_id_less_tool_outputs_preserves_images_and_text() {
        let mut body = json!({
            "input": [
                {
                    "type": "function_call_output",
                    "id": "fco_1",
                    "output": [
                        {"type": "input_text", "text": "delegated receipt"},
                        {"type": "input_image", "image_url": "data:image/png;base64,AA=="}
                    ]
                },
                {
                    "type": "function_call_output",
                    "call_id": "c1",
                    "output": "normal"
                }
            ]
        });

        assert!(rewrite_call_id_less_tool_outputs(&mut body));
        assert_eq!(body["input"][0]["type"], "message");
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][0]["content"][1]["text"], "delegated receipt");
        assert_eq!(body["input"][0]["content"][2]["type"], "input_image");
        assert_eq!(body["input"][1]["type"], "function_call_output");
        assert_eq!(body["input"][1]["call_id"], "c1");
    }

    #[test]
    fn normalize_tool_output_grouping_defers_notices_and_stops_at_user_turn() {
        let mut body = json!({
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "go"}]},
                {"type": "function_call", "call_id": "c0", "name": "view_image", "arguments": "{}"},
                {"type": "function_call", "call_id": "c1", "name": "view_image", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "c0", "output": "img0"},
                {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "<image_resize_notice>"}]},
                {"type": "function_call_output", "call_id": "c1", "output": "img1"}
            ]
        });
        assert!(normalize_tool_output_grouping(&mut body));
        let types: Vec<&str> = body["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["role"].as_str().or(i["type"].as_str()).unwrap())
            .collect();
        assert_eq!(
            types,
            vec![
                "user",
                "function_call",
                "function_call",
                "function_call_output",
                "function_call_output",
                "developer"
            ]
        );
    }

    #[test]
    fn repair_strict_tool_history_drops_empty_assistant_and_hoists_commentary() {
        let mut body = json!({
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "run"}]},
                {"type": "function_call", "call_id": "c1", "name": "exec", "arguments": "{}"},
                {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": ""}]},
                {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "working..."}]},
                {"type": "function_call_output", "call_id": "c1", "output": "ok"}
            ]
        });
        assert!(repair_strict_tool_history(&mut body));
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 4);
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"][0]["text"], "working...");
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[3]["type"], "function_call_output");
    }

    #[test]
    fn dedup_responses_function_call_arguments_keeps_last_value_and_skips_clean_strings() {
        let mut body = json!({
            "input": [
                {"type": "function_call", "call_id": "c1", "name": "edit", "arguments": "{\"path\":\"a.rs\",\"path\":\"b.rs\",\"nested\":{\"k\":1,\"k\":2}}"},
                {"type": "function_call", "call_id": "c2", "name": "read", "arguments": "{\"path\": \"keep_spacing.rs\"}"}
            ]
        });
        assert!(dedup_responses_function_call_arguments(&mut body));
        let parsed: Value =
            serde_json::from_str(body["input"][0]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(parsed, json!({"path": "b.rs", "nested": {"k": 2}}));
        // Clean string without duplicate keys must stay byte-identical.
        assert_eq!(body["input"][1]["arguments"], "{\"path\": \"keep_spacing.rs\"}");
    }

    #[tokio::test]
    async fn late_arguments_repair_backfills_empty_done_events_from_late_delta() {
        let sse = concat!(
            "event: response.function_call_arguments.done\n",
            "data: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_1\",\"arguments\":\"\"}\n\n",
            "event: response.output_item.done\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"read\",\"arguments\":\"\"}}\n\n",
            "event: response.function_call_arguments.delta\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{\\\"path\\\":\\\"README.md\\\"}\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"output\":[]}}\n\n"
        );
        let input = Box::pin(stream::iter(vec![Ok(sse.as_bytes().to_vec())]));
        let mut wrapped = wrap_late_arguments_repair_sse_stream(input);
        let mut out = Vec::new();
        while let Some(chunk) = wrapped.next().await {
            out.extend_from_slice(&chunk.unwrap());
        }
        let text = String::from_utf8(out).unwrap();
        let done_pos = text.find("response.function_call_arguments.done").unwrap();
        let delta_pos = text.find("response.function_call_arguments.delta").unwrap();
        assert!(delta_pos < done_pos, "delta must precede flushed done events");
        assert!(
            text.contains(r#"\"path\":\"README.md\""#),
            "flushed done events must contain backfilled arguments: {text}"
        );
    }

    #[tokio::test]
    async fn late_arguments_repair_preserves_degenerate_flattened_sse_without_blank_lines() {
        let flattened = concat!(
            "event: response.in_progress data: {\"type\":\"response.in_progress\",\"response\":{\"id\":\"resp_flat\",\"status\":\"in_progress\"}} ",
            "event: response.output_text.delta data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"} ",
            "event: response.completed data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_flat\",\"status\":\"completed\"}}"
        );
        let input = Box::pin(stream::iter(vec![Ok(flattened.as_bytes().to_vec())]));
        let mut wrapped = wrap_late_arguments_repair_sse_stream(input);
        let mut out = Vec::new();
        while let Some(chunk) = wrapped.next().await {
            out.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(String::from_utf8(out).unwrap(), flattened);
    }
}
