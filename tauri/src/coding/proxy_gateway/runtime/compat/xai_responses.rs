//! xAI native OpenAI Responses passthrough: namespace flatten/restore + field sanitize.
//!
//! Codex 0.142+ sends private Responses shapes (`namespace` tools, `tool_search`,
//! OpenAI-backend-only fields) that xAI's strict `/v1/responses` rejects. Chat /
//! Anthropic conversion paths already drop most of these; **native** Responses
//! passthrough (`source == target == OpenAiResponses`) must scrub here.
//!
//! Semantics ported from cc-switch
//! (`transform_codex_responses_namespace.rs` / `transform_codex_responses_xai_sanitize.rs`)
//! and sub2api. Naming uses the same [`flatten_namespace_tool_name`] as Chat
//! conversion so flatten and restore stay consistent without threading state.

use super::super::http_io::DebugBodyStream;
use super::codex_responses::{
    is_degenerate_flattened_sse_line, rewrite_sse_block_data_preserving_framing,
    take_sse_block_preserving_delimiter,
};
use crate::coding::proxy_gateway::transformer::{
    append_utf8_safe, flatten_namespace_tool_name, strip_sse_field,
};
use crate::coding::proxy_gateway::usage_parser::flattened_flush_boundary;
use futures_util::StreamExt;
use serde_json::{json, Map, Number, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::pin::Pin;

/// Reverse map entry: flattened tool name → original namespace + bare child name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NamespacedName {
    pub namespace: String,
    pub name: String,
}

const RECURSIVE_UNSUPPORTED_FIELDS: &[&str] = &["external_web_access"];
const TOP_LEVEL_UNSUPPORTED_FIELDS: &[&str] = &["prompt_cache_retention", "safety_identifier"];
const XAI_UNSUPPORTED_INCLUDE_MARKERS: &[&str] = &["output.encrypted_content"];
const GROK_45_UNSUPPORTED_FIELDS: &[&str] = &[
    "presence_penalty",
    "presencePenalty",
    "frequency_penalty",
    "frequencyPenalty",
    "stop",
];
const XAI_SUPPORTED_TOOL_TYPES: &[&str] = &[
    "function",
    "web_search",
    "x_search",
    "image_generation",
    "collections_search",
    "file_search",
    "code_execution",
    "code_interpreter",
    "mcp",
    "shell",
];

/// Build flat-name → `{namespace, name}` from the **original** request body tools.
/// Pure: does not mutate the body; used for response restore.
pub(crate) fn namespace_restore_map(request_body: &Value) -> HashMap<String, NamespacedName> {
    let mut map = HashMap::new();
    let Some(tools) = request_body.get("tools").and_then(Value::as_array) else {
        return map;
    };
    for tool in tools {
        if tool.get("type").and_then(Value::as_str) != Some("namespace") {
            continue;
        }
        let Some(namespace) = tool.get("name").and_then(Value::as_str) else {
            continue;
        };
        let namespace = namespace.trim();
        if namespace.is_empty() {
            continue;
        }
        for child in namespace_children(tool) {
            if child.get("type").and_then(Value::as_str) != Some("function") {
                continue;
            }
            let Some(name) = child.get("name").and_then(Value::as_str) else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let flat = flatten_namespace_tool_name(namespace, name);
            map.entry(flat).or_insert_with(|| NamespacedName {
                namespace: namespace.to_string(),
                name: name.to_string(),
            });
        }
    }
    map
}

/// Flatten namespace tools, rewrite input history + tool_choice.
///
/// Returns `Ok(true)` when the body changed. Flat-name collisions fail closed.
pub(crate) fn flatten_request_namespaces(body: &mut Value) -> Result<bool, String> {
    let Some(tools) = body.get("tools").and_then(Value::as_array) else {
        return Ok(false);
    };
    if !tools
        .iter()
        .any(|tool| tool.get("type").and_then(Value::as_str) == Some("namespace"))
    {
        return Ok(false);
    }

    let mut top_level = HashSet::new();
    for tool in tools {
        let typ = tool.get("type").and_then(Value::as_str).unwrap_or("");
        if typ == "function" || typ == "custom" {
            if let Some(name) = tool.get("name").and_then(Value::as_str) {
                let name = name.trim();
                if !name.is_empty() {
                    top_level.insert(name.to_string());
                }
            }
        }
    }

    let mut owners: HashMap<String, NamespacedName> = HashMap::new();
    for tool in tools {
        if tool.get("type").and_then(Value::as_str) != Some("namespace") {
            continue;
        }
        let Some(namespace) = tool.get("name").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if namespace.is_empty() {
            continue;
        }
        for child in namespace_children(tool) {
            if child.get("type").and_then(Value::as_str) != Some("function") {
                continue;
            }
            let Some(name) = child.get("name").and_then(Value::as_str).map(str::trim) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let flat = flatten_namespace_tool_name(namespace, name);
            if top_level.contains(&flat) {
                return Err(format!(
                    "namespace tool {namespace:?}/{name:?} flattens to {flat:?} which \
                     collides with a top-level tool of the same name; rename one of them"
                ));
            }
            let entry = NamespacedName {
                namespace: namespace.to_string(),
                name: name.to_string(),
            };
            if let Some(prev) = owners.get(&flat) {
                if *prev != entry {
                    return Err(format!(
                        "namespace tools {:?}/{:?} and {namespace:?}/{name:?} both flatten to \
                         {flat:?}; rename one of them",
                        prev.namespace, prev.name
                    ));
                }
            } else {
                owners.insert(flat, entry);
            }
        }
    }

    let tools = body
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut flattened: Vec<Value> = Vec::with_capacity(tools.len());
    let mut seen_flat = HashSet::new();
    for tool in tools {
        if tool.get("type").and_then(Value::as_str) != Some("namespace") {
            flattened.push(tool);
            continue;
        }
        let Some(namespace) = tool.get("name").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        for child in namespace_children(&tool) {
            if child.get("type").and_then(Value::as_str) != Some("function") {
                continue;
            }
            let Some(name) = child.get("name").and_then(Value::as_str).map(str::trim) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let flat = flatten_namespace_tool_name(namespace, name);
            if !seen_flat.insert(flat.clone()) {
                continue;
            }
            let mut lifted = child.clone();
            if let Some(obj) = lifted.as_object_mut() {
                obj.insert("name".to_string(), json!(flat));
            }
            flattened.push(lifted);
        }
    }
    body["tools"] = json!(flattened);

    if let Some(input) = body.get_mut("input") {
        rewrite_namespace_qualified_calls(input, &owners);
    }

    if let Some(choice) = body.get_mut("tool_choice") {
        if choice.get("type").and_then(Value::as_str) == Some("namespace") {
            *choice = json!("auto");
        } else {
            rewrite_namespace_qualified_call(choice, &owners);
        }
    }

    Ok(true)
}

/// Restore flattened `function_call` names in a full JSON Responses payload.
#[cfg(test)]
pub(crate) fn restore_response_namespaces(
    value: &mut Value,
    map: &HashMap<String, NamespacedName>,
) -> bool {
    if map.is_empty() {
        return false;
    }
    restore_value(value, map)
}

/// Fixed order: namespace flatten → sanitize → agent_message rewrite → forced
/// hosted web_search tool_choice rewrite. Fail-closed on name collisions.
pub(crate) fn apply_xai_responses_passthrough(body: &mut Value) -> Result<(), String> {
    flatten_request_namespaces(body)?;
    sanitize_xai_responses_request(body);
    rewrite_xai_agent_message_input_items(body);
    rewrite_xai_hosted_web_search_tool_choice(body);
    Ok(())
}

/// Strip xAI-unsupported fields/tools and normalize root union schemas.
/// Deterministic and idempotent.
pub(crate) fn sanitize_xai_responses_request(body: &mut Value) -> bool {
    if !body.is_object() {
        return false;
    }

    let mut changed = false;

    for field in TOP_LEVEL_UNSUPPORTED_FIELDS {
        changed |= remove_top_level_field(body, field);
    }

    changed |= filter_unsupported_include_markers(body);

    if request_targets_grok_45(body) {
        for field in GROK_45_UNSUPPORTED_FIELDS {
            changed |= remove_top_level_field(body, field);
        }
    }

    for field in RECURSIVE_UNSUPPORTED_FIELDS {
        changed |= remove_field_recursive(body, field);
    }

    changed |= promote_additional_tools(body);
    changed |= strip_null_reasoning_content(body);
    changed |= filter_unsupported_tools(body);
    changed |= normalize_xai_function_tool_parameter_schemas(body);

    changed
}

/// xAI Responses `ModelToolChoice` rejects `{"type":"web_search"}` with 422.
/// Rewrite forced hosted web search to `"required"` and retain only
/// `web_search` tools so `"required"` cannot land on a local function; drop
/// both if no `web_search` tool survives (cc-switch 5ba95160).
pub(crate) fn rewrite_xai_hosted_web_search_tool_choice(body: &mut Value) -> bool {
    let is_hosted_choice = body
        .get("tool_choice")
        .and_then(Value::as_object)
        .and_then(|choice| choice.get("type"))
        .and_then(Value::as_str)
        == Some("web_search");
    if !is_hosted_choice {
        return false;
    }
    let Some(obj) = body.as_object_mut() else {
        return false;
    };
    if let Some(tools) = obj.get_mut("tools").and_then(Value::as_array_mut) {
        tools.retain(|tool| tool.get("type").and_then(Value::as_str) == Some("web_search"));
    }
    let has_search = obj
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| !tools.is_empty());
    if has_search {
        obj.insert("tool_choice".to_string(), json!("required"));
    } else {
        obj.remove("tool_choice");
        obj.remove("tools");
    }
    true
}

/// Rewrite Codex multi-agent `agent_message` items into ordinary `user`
/// `message` items (flattening `encrypted_content` parts to `input_text`).
pub(crate) fn rewrite_xai_agent_message_input_items(body: &mut Value) -> bool {
    rewrite_agent_message_value(body)
}

/// Normalize whole-number JSON floats (`92116.0` -> `92116`) on completed
/// `function_call` argument payloads, skipping `*.delta` events and guarding
/// values `>= 2^64`.
pub(crate) fn normalize_xai_function_call_integer_arguments(value: &mut Value) -> bool {
    normalize_xai_function_call_integer_arguments_value(value)
}

/// Apply all xAI Responses response-side normalizations (namespace restore +
/// whole-float integer argument rewrite) to a full JSON Responses body.
pub(crate) fn restore_xai_responses_value(
    value: &mut Value,
    map: &HashMap<String, NamespacedName>,
) -> bool {
    let mut changed = false;
    if !map.is_empty() {
        changed |= restore_value(value, map);
    }
    changed |= normalize_xai_function_call_integer_arguments(value);
    changed
}

/// Wrap a Responses SSE stream, restoring flat `function_call` names and
/// normalizing whole-float integer tool arguments on completed events.
pub(crate) fn wrap_namespace_restore_sse_stream(
    stream: DebugBodyStream,
    map: HashMap<String, NamespacedName>,
) -> DebugBodyStream {
    struct State {
        inner: DebugBodyStream,
        map: HashMap<String, NamespacedName>,
        buffer: String,
        utf8_remainder: Vec<u8>,
        pending: VecDeque<Result<Vec<u8>, String>>,
        finished: bool,
    }

    Box::pin(futures_util::stream::unfold(
        State {
            inner: stream,
            map,
            buffer: String::new(),
            utf8_remainder: Vec::new(),
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
                            // Keep blank/comment keep-alive blocks byte-equivalent; only
                            // rewrite blocks that contain data JSON.
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
                            state.pending.push_back(Ok(restore_sse_block(
                                &block,
                                delimiter,
                                &state.map,
                            )));
                        }
                        if is_degenerate_flattened_sse_line(&state.buffer) {
                            let boundary = flattened_flush_boundary(state.buffer.as_bytes());
                            if boundary > 0 && state.buffer.is_char_boundary(boundary) {
                                let passthrough: String =
                                    state.buffer.drain(..boundary).collect();
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
                                state.pending.push_back(Ok(tail.into_bytes()));
                            } else {
                                state
                                    .pending
                                    .push_back(Ok(restore_sse_block(&tail, "", &state.map)));
                            }
                        }
                    }
                }
            }
        },
    )) as Pin<Box<dyn futures_util::Stream<Item = Result<Vec<u8>, String>> + Send + 'static>>
}

fn namespace_children(tool: &Value) -> Vec<Value> {
    tool.get("tools")
        .or_else(|| tool.get("children"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn rewrite_namespace_qualified_calls(value: &mut Value, owners: &HashMap<String, NamespacedName>) {
    match value {
        Value::Array(items) => {
            for item in items {
                rewrite_namespace_qualified_calls(item, owners);
            }
        }
        Value::Object(obj) => {
            if obj.get("type").and_then(Value::as_str) == Some("function_call") {
                rewrite_namespace_qualified_call(value, owners);
                return;
            }
            for child in obj.values_mut() {
                rewrite_namespace_qualified_calls(child, owners);
            }
        }
        _ => {}
    }
}

fn rewrite_namespace_qualified_call(
    item: &mut Value,
    owners: &HashMap<String, NamespacedName>,
) -> bool {
    let Some(obj) = item.as_object_mut() else {
        return false;
    };
    let namespace = obj
        .get("namespace")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    if namespace.is_empty() || name.is_empty() {
        return false;
    }
    let flat = flatten_namespace_tool_name(&namespace, &name);
    match owners.get(&flat) {
        Some(entry) if entry.namespace == namespace && entry.name == name => {
            obj.insert("name".to_string(), json!(flat));
            obj.remove("namespace");
            true
        }
        _ => false,
    }
}

fn restore_value(value: &mut Value, map: &HashMap<String, NamespacedName>) -> bool {
    let mut changed = false;
    match value {
        Value::Array(items) => {
            for item in items {
                changed |= restore_value(item, map);
            }
        }
        Value::Object(obj) => {
            if obj.get("type").and_then(Value::as_str) == Some("function_call") {
                if let Some(flat) = obj.get("name").and_then(Value::as_str) {
                    if let Some(entry) = map.get(flat) {
                        obj.insert("name".to_string(), json!(entry.name));
                        obj.insert("namespace".to_string(), json!(entry.namespace));
                        changed = true;
                    }
                }
            }
            for child in obj.values_mut() {
                changed |= restore_value(child, map);
            }
        }
        _ => {}
    }
    changed
}

fn restore_sse_block(
    block: &str,
    delimiter: &str,
    map: &HashMap<String, NamespacedName>,
) -> Vec<u8> {
    let mut data_parts: Vec<&str> = Vec::new();
    for line in block.lines() {
        if let Some(data) = strip_sse_field(line, "data") {
            data_parts.push(data);
        }
    }

    if data_parts.is_empty() {
        return format!("{block}{delimiter}").into_bytes();
    }

    let data = data_parts.join("\n");
    if data.trim() == "[DONE]" {
        return format!("{block}{delimiter}").into_bytes();
    }

    let mut event: Value = match serde_json::from_str(&data) {
        Ok(value) => value,
        Err(_) => return format!("{block}{delimiter}").into_bytes(),
    };

    if !restore_xai_responses_value(&mut event, map) {
        return format!("{block}{delimiter}").into_bytes();
    }

    let restored = serde_json::to_string(&event).unwrap_or(data);
    rewrite_sse_block_data_preserving_framing(block, delimiter, &restored)
}

fn request_targets_grok_45(body: &Value) -> bool {
    let Some(model) = body.get("model").and_then(Value::as_str) else {
        return false;
    };
    let mut model = model.trim();
    if let Some(idx) = model.rfind('/') {
        model = model[idx + 1..].trim();
    }
    model.eq_ignore_ascii_case("grok-4.5")
}

fn remove_top_level_field(body: &mut Value, field: &str) -> bool {
    body.as_object_mut()
        .and_then(|obj| obj.remove(field))
        .is_some()
}

fn remove_field_recursive(value: &mut Value, field: &str) -> bool {
    match value {
        Value::Object(map) => {
            let mut changed = map.remove(field).is_some();
            for child in map.values_mut() {
                changed |= remove_field_recursive(child, field);
            }
            changed
        }
        Value::Array(items) => {
            let mut changed = false;
            for child in items.iter_mut() {
                changed |= remove_field_recursive(child, field);
            }
            changed
        }
        _ => false,
    }
}

fn is_additional_tools_item(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str).map(str::trim) == Some("additional_tools")
}

fn promote_additional_tools(body: &mut Value) -> bool {
    let input_items: Vec<Value> = match body.get("input").and_then(Value::as_array) {
        Some(arr) if arr.iter().any(is_additional_tools_item) => arr.clone(),
        _ => return false,
    };

    let mut merged: Vec<Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        for tool in tools {
            seen.insert(tool_dedup_key(tool));
            merged.push(tool.clone());
        }
    }

    let mut filtered_input: Vec<Value> = Vec::with_capacity(input_items.len());
    let mut promoted = false;
    for item in input_items {
        if is_additional_tools_item(&item) {
            if let Some(carrier_tools) = item.get("tools").and_then(Value::as_array) {
                for tool in carrier_tools {
                    if seen.insert(tool_dedup_key(tool)) {
                        merged.push(tool.clone());
                        promoted = true;
                    }
                }
            }
            continue;
        }
        filtered_input.push(item);
    }

    if let Some(obj) = body.as_object_mut() {
        obj.insert("input".to_string(), Value::Array(filtered_input));
        if promoted {
            obj.insert("tools".to_string(), Value::Array(merged));
        }
    }
    true
}

fn tool_dedup_key(tool: &Value) -> String {
    let tool_type = tool
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if !tool_type.is_empty() {
        if let Some(name) = tool.get("name").and_then(Value::as_str) {
            let name = name.trim();
            if !name.is_empty() {
                return format!("type:{tool_type}\u{0}name:{name}");
            }
        }
        if tool_type == "mcp" {
            if let Some(label) = tool.get("server_label").and_then(Value::as_str) {
                let label = label.trim();
                if !label.is_empty() {
                    return format!("type:mcp\u{0}server_label:{label}");
                }
            }
        }
    }
    format!("json:{tool}")
}

fn strip_null_reasoning_content(body: &mut Value) -> bool {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    for item in input.iter_mut() {
        if item.get("type").and_then(Value::as_str).map(str::trim) != Some("reasoning") {
            continue;
        }
        if let Some(obj) = item.as_object_mut() {
            if matches!(obj.get("content"), Some(Value::Null)) {
                obj.remove("content");
                changed = true;
            }
        }
    }
    changed
}

fn filter_unsupported_tools(body: &mut Value) -> bool {
    let Some(tools) = body.get("tools").and_then(Value::as_array) else {
        return false;
    };
    let original_len = tools.len();
    let filtered: Vec<Value> = tools
        .iter()
        .filter(|tool| {
            let t = tool
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            XAI_SUPPORTED_TOOL_TYPES.contains(&t)
        })
        .cloned()
        .collect();

    let mut changed = false;
    if filtered.len() != original_len {
        if let Some(obj) = body.as_object_mut() {
            if filtered.is_empty() {
                obj.remove("tools");
            } else {
                obj.insert("tools".to_string(), Value::Array(filtered.clone()));
            }
        }
        changed = true;
    }

    if body.get("tool_choice").is_some() && should_drop_tool_choice(body, &filtered) {
        if let Some(obj) = body.as_object_mut() {
            obj.remove("tool_choice");
        }
        changed = true;
    }

    changed
}

fn should_drop_tool_choice(body: &Value, tools: &[Value]) -> bool {
    let Some(tool_choice) = body.get("tool_choice") else {
        return false;
    };
    if tools.is_empty() {
        return true;
    }
    let Some(choice) = tool_choice.as_object() else {
        return false;
    };
    let choice_type = choice
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if choice_type.is_empty() {
        return false;
    }
    if !XAI_SUPPORTED_TOOL_TYPES.contains(&choice_type) {
        return true;
    }
    if choice_type == "function" {
        let choice_name = choice
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| {
                choice
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("")
            .trim();
        if choice_name.is_empty() {
            return false;
        }
        let exists = tools.iter().any(|tool| {
            let t = tool
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| {
                    tool.get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(Value::as_str)
                })
                .unwrap_or("")
                .trim();
            t == "function" && name == choice_name
        });
        return !exists;
    }
    false
}

fn filter_unsupported_include_markers(body: &mut Value) -> bool {
    let mut remove_empty_include = false;
    let changed = match body.get_mut("include").and_then(Value::as_array_mut) {
        Some(include) => {
            let original_len = include.len();
            include.retain(|item| {
                !item
                    .as_str()
                    .is_some_and(|marker| XAI_UNSUPPORTED_INCLUDE_MARKERS.contains(&marker))
            });
            let changed = include.len() != original_len;
            remove_empty_include = changed && include.is_empty();
            changed
        }
        None => false,
    };

    if remove_empty_include {
        if let Some(body) = body.as_object_mut() {
            body.remove("include");
        }
    }

    changed
}

fn xai_function_parameters_need_simplification(params: &Value) -> bool {
    match params {
        Value::Null => true,
        Value::Object(obj) if obj.is_empty() => true,
        Value::Object(obj) => {
            match obj.get("type") {
                None | Some(Value::Null) => return true,
                Some(Value::String(type_name)) if type_name != "object" => return true,
                Some(Value::Array(_)) => return true,
                _ => {}
            }
            for union_key in ["oneOf", "anyOf"] {
                let Some(branches) = obj.get(union_key).and_then(Value::as_array) else {
                    continue;
                };
                if branches.is_empty() {
                    continue;
                }
                if branches
                    .iter()
                    .any(|branch| branch.get("type").and_then(Value::as_str) != Some("object"))
                {
                    return true;
                }
            }
            false
        }
        _ => true,
    }
}

fn flatten_union_branches_to_object(branches: &[Value]) -> Value {
    let object_branches: Vec<&Value> = branches
        .iter()
        .filter(|branch| branch.get("type").and_then(Value::as_str) == Some("object"))
        .collect();

    if object_branches.len() == 1 {
        let mut result = object_branches[0].clone();
        if let Some(obj) = result.as_object_mut() {
            obj.insert("type".to_string(), json!("object"));
            obj.entry("properties".to_string())
                .or_insert_with(|| json!({}));
        }
        return result;
    }

    if !object_branches.is_empty() {
        let mut merged_properties = Map::new();
        let mut merged_required: Option<Vec<Value>> = None;
        for branch in object_branches {
            if let Some(properties) = branch.get("properties").and_then(Value::as_object) {
                for (key, value) in properties {
                    merged_properties
                        .entry(key.clone())
                        .or_insert_with(|| value.clone());
                }
            }
            let branch_required = branch
                .get("required")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            merged_required = Some(match merged_required {
                None => branch_required,
                Some(existing) => existing
                    .into_iter()
                    .filter(|item| branch_required.contains(item))
                    .collect(),
            });
        }

        let mut result = json!({
            "type": "object",
            "properties": Value::Object(merged_properties),
        });
        let merged_required = merged_required.unwrap_or_default();
        if !merged_required.is_empty() {
            result["required"] = Value::Array(merged_required);
        }
        return result;
    }

    xai_safe_empty_object_schema()
}

fn simplify_xai_function_parameters(params: Option<&Value>) -> Value {
    match params {
        None | Some(Value::Null) => xai_safe_empty_object_schema(),
        Some(Value::Object(obj)) if obj.is_empty() => xai_safe_empty_object_schema(),
        Some(Value::Object(obj)) => {
            for union_key in ["oneOf", "anyOf"] {
                if let Some(branches) = obj.get(union_key).and_then(Value::as_array) {
                    if branches
                        .iter()
                        .any(|branch| branch.get("type").and_then(Value::as_str) != Some("object"))
                    {
                        let mut flattened = flatten_union_branches_to_object(branches);
                        if let Some(flattened_obj) = flattened.as_object_mut() {
                            for defs_key in ["$defs", "definitions"] {
                                if let Some(defs) = obj.get(defs_key) {
                                    flattened_obj
                                        .entry(defs_key.to_string())
                                        .or_insert_with(|| defs.clone());
                                }
                            }
                        }
                        return flattened;
                    }
                }
            }

            let mut result = Value::Object(obj.clone());
            if let Some(obj) = result.as_object_mut() {
                if obj.get("type").and_then(Value::as_str) != Some("object") {
                    obj.insert("type".to_string(), json!("object"));
                    obj.entry("properties".to_string())
                        .or_insert_with(|| json!({}));
                }
            }
            result
        }
        _ => xai_safe_empty_object_schema(),
    }
}

fn function_tool_name(tool: &Value) -> &str {
    tool.get("name")
        .and_then(Value::as_str)
        .or_else(|| {
            tool.get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .trim()
}

fn is_automation_update_tool(name: &str) -> bool {
    name == "codex_app__automation_update"
        || name == "mcp__codex_app__automation_update"
        || name.ends_with("__automation_update")
}

fn xai_safe_empty_object_schema() -> Value {
    json!({"type": "object", "properties": {}, "additionalProperties": true})
}

fn normalize_xai_function_tool_parameters(tool: &mut Value) -> bool {
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return false;
    }

    if is_automation_update_tool(function_tool_name(tool)) {
        let safe = xai_safe_empty_object_schema();
        let needs_rewrite = {
            let current = tool.get("parameters").or_else(|| {
                tool.get("function")
                    .and_then(|function| function.get("parameters"))
            });
            current != Some(&safe)
        };
        let mut changed = needs_rewrite;
        if let Some(obj) = tool.as_object_mut() {
            if needs_rewrite {
                if obj.contains_key("parameters") || obj.get("function").is_none() {
                    obj.insert("parameters".to_string(), safe);
                } else if let Some(function) =
                    obj.get_mut("function").and_then(Value::as_object_mut)
                {
                    function.insert("parameters".to_string(), safe);
                }
            }
            if obj.get("strict") == Some(&json!(true)) {
                obj.insert("strict".to_string(), json!(false));
                changed = true;
            }
            if let Some(function) = obj.get_mut("function").and_then(Value::as_object_mut) {
                if function.get("strict") == Some(&json!(true)) {
                    function.insert("strict".to_string(), json!(false));
                    changed = true;
                }
            }
        }
        return changed;
    }

    let params = tool
        .get("parameters")
        .or_else(|| {
            tool.get("function")
                .and_then(|function| function.get("parameters"))
        })
        .cloned();

    let needs = match params.as_ref() {
        Some(params) => xai_function_parameters_need_simplification(params),
        None => false,
    };
    if !needs {
        return false;
    }
    let simplified = simplify_xai_function_parameters(params.as_ref());
    if params.as_ref() == Some(&simplified) {
        return false;
    }
    if let Some(obj) = tool.as_object_mut() {
        let should_clear_strict = simplified.get("additionalProperties") == Some(&json!(true))
            || params
                .as_ref()
                .and_then(|orig| orig.get("oneOf").or_else(|| orig.get("anyOf")))
                .is_some();
        if obj.contains_key("parameters") || obj.get("function").is_none() {
            obj.insert("parameters".to_string(), simplified);
            if should_clear_strict && obj.get("strict") == Some(&json!(true)) {
                obj.insert("strict".to_string(), json!(false));
            }
            return true;
        }
        if let Some(function) = obj.get_mut("function").and_then(Value::as_object_mut) {
            function.insert("parameters".to_string(), simplified);
            if should_clear_strict && function.get("strict") == Some(&json!(true)) {
                function.insert("strict".to_string(), json!(false));
            }
            return true;
        }
    }
    false
}

fn normalize_xai_function_tool_parameter_schemas(body: &mut Value) -> bool {
    let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    for tool in tools.iter_mut() {
        changed |= normalize_xai_function_tool_parameters(tool);
    }
    changed
}

fn rewrite_agent_message_value(value: &mut Value) -> bool {
    if rewrite_agent_message_item(value) {
        return true;
    }
    match value {
        Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= rewrite_agent_message_value(item);
            }
            changed
        }
        Value::Object(obj) => {
            let mut changed = false;
            for child in obj.values_mut() {
                changed |= rewrite_agent_message_value(child);
            }
            changed
        }
        _ => false,
    }
}

fn rewrite_agent_message_item(item: &mut Value) -> bool {
    if item.get("type").and_then(Value::as_str).map(str::trim) != Some("agent_message") {
        return false;
    }
    let id = item.get("id").cloned();
    let content = match item.get("content") {
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                let text = if part.get("type").and_then(Value::as_str).map(str::trim)
                    == Some("encrypted_content")
                {
                    part.get("encrypted_content")
                        .or_else(|| part.get("text"))
                        .and_then(Value::as_str)
                } else {
                    part.get("text").and_then(Value::as_str)
                }?;
                (!text.is_empty()).then(|| json!({"type": "input_text", "text": text}))
            })
            .collect::<Vec<_>>(),
        Some(Value::String(text)) if !text.is_empty() => {
            vec![json!({"type": "input_text", "text": text})]
        }
        _ => Vec::new(),
    };
    let mut message = json!({
        "type": "message",
        "role": "user",
        "content": content,
    });
    if let Some(id) = id {
        message["id"] = id;
    }
    *item = message;
    true
}

fn normalize_xai_function_call_integer_arguments_value(value: &mut Value) -> bool {
    match value {
        Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= normalize_xai_function_call_integer_arguments_value(item);
            }
            changed
        }
        Value::Object(obj) => {
            let event_type = obj
                .get("type")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            if event_type.as_deref() == Some("response.function_call_arguments.delta") {
                return false;
            }
            let mut changed = false;
            if event_type.as_deref() == Some("response.function_call_arguments.done")
                || event_type.as_deref() == Some("function_call")
            {
                changed |= normalize_function_call_arguments_field(obj);
            }
            for child in obj.values_mut() {
                changed |= normalize_xai_function_call_integer_arguments_value(child);
            }
            changed
        }
        _ => false,
    }
}

fn normalize_function_call_arguments_field(obj: &mut Map<String, Value>) -> bool {
    match obj.get_mut("arguments") {
        Some(Value::String(arguments)) => {
            let Ok(mut parsed) = serde_json::from_str::<Value>(arguments) else {
                return false;
            };
            if !rewrite_whole_number_floats(&mut parsed) {
                return false;
            }
            if let Ok(serialized) = serde_json::to_string(&parsed) {
                *arguments = serialized;
                return true;
            }
            false
        }
        Some(other) => rewrite_whole_number_floats(other),
        None => false,
    }
}

fn rewrite_whole_number_floats(value: &mut Value) -> bool {
    match value {
        Value::Number(number) => {
            if let Some(integer) = whole_float_to_json_int(number) {
                *number = integer;
                true
            } else {
                false
            }
        }
        Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= rewrite_whole_number_floats(item);
            }
            changed
        }
        Value::Object(map) => {
            let mut changed = false;
            for child in map.values_mut() {
                changed |= rewrite_whole_number_floats(child);
            }
            changed
        }
        _ => false,
    }
}

fn whole_float_to_json_int(number: &Number) -> Option<Number> {
    if number.is_i64() || number.is_u64() {
        return None;
    }
    let float = number.as_f64()?;
    if !float.is_finite() || float.fract() != 0.0 {
        return None;
    }
    if float >= 0.0 {
        // Guard `>= 2^64`: `u64::MAX as f64` rounds up to 2^64, so `as u64`
        // would saturate to u64::MAX and slip past round-trip comparison.
        if float >= (u64::MAX as f64) {
            return None;
        }
        let int = float as u64;
        if (int as f64) == float {
            return Some(Number::from(int));
        }
        None
    } else {
        if float < (i64::MIN as f64) {
            return None;
        }
        let int = float as i64;
        if (int as f64) == float {
            return Some(Number::from(int));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use serde_json::json;

    fn namespace_request() -> Value {
        json!({
            "model": "grok-4.5",
            "tools": [
                { "type": "function", "name": "plain_tool", "parameters": {} },
                {
                    "type": "namespace",
                    "name": "mcp__files__",
                    "tools": [
                        { "type": "function", "name": "read", "description": "read a file", "parameters": {} },
                        { "type": "function", "name": "write", "parameters": {} }
                    ]
                }
            ],
            "input": [
                {
                    "type": "function_call",
                    "name": "read",
                    "namespace": "mcp__files__",
                    "call_id": "c1",
                    "arguments": "{}"
                }
            ],
            "tool_choice": { "type": "namespace", "name": "mcp__files__" }
        })
    }

    #[test]
    fn flatten_lifts_namespace_children_to_top_level_functions() {
        let mut body = namespace_request();
        assert!(flatten_request_namespaces(&mut body).unwrap());

        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        assert!(tools.iter().all(|t| t["type"] == "function"));
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"plain_tool"));
        assert!(names.contains(&"mcp__files____read"));
        assert!(names.contains(&"mcp__files____write"));
        let read = tools
            .iter()
            .find(|t| t["name"] == "mcp__files____read")
            .unwrap();
        assert_eq!(read["description"], "read a file");
    }

    #[test]
    fn flatten_rewrites_input_history_calls_and_tool_choice() {
        let mut body = namespace_request();
        flatten_request_namespaces(&mut body).unwrap();

        let call = &body["input"][0];
        assert_eq!(call["name"], "mcp__files____read");
        assert!(call.get("namespace").is_none());
        assert_eq!(call["call_id"], "c1");
        assert_eq!(body["tool_choice"], json!("auto"));
    }

    #[test]
    fn flatten_is_noop_without_namespace_tools() {
        let mut body = json!({
            "tools": [ { "type": "function", "name": "plain", "parameters": {} } ]
        });
        assert!(!flatten_request_namespaces(&mut body).unwrap());
    }

    #[test]
    fn flatten_errors_on_flat_name_collision_with_top_level() {
        let mut body = json!({
            "tools": [
                { "type": "function", "name": "mcp__files____read", "parameters": {} },
                {
                    "type": "namespace",
                    "name": "mcp__files__",
                    "tools": [ { "type": "function", "name": "read", "parameters": {} } ]
                }
            ]
        });
        assert!(flatten_request_namespaces(&mut body).is_err());
    }

    #[test]
    fn restore_map_inverts_flatten_naming() {
        let body = namespace_request();
        let map = namespace_restore_map(&body);
        let entry = map.get("mcp__files____read").unwrap();
        assert_eq!(entry.namespace, "mcp__files__");
        assert_eq!(entry.name, "read");
        assert!(!map.contains_key("plain_tool"));
    }

    #[test]
    fn round_trip_flatten_then_restore_recovers_namespace() {
        let request = namespace_request();
        let map = namespace_restore_map(&request);

        let mut response = json!({
            "type": "response",
            "output": [
                {
                    "type": "function_call",
                    "name": "mcp__files____read",
                    "call_id": "c1",
                    "arguments": "{}"
                }
            ]
        });
        assert!(restore_response_namespaces(&mut response, &map));
        let call = &response["output"][0];
        assert_eq!(call["name"], "read");
        assert_eq!(call["namespace"], "mcp__files__");
    }

    #[test]
    fn restore_leaves_unmapped_calls_untouched() {
        let map = namespace_restore_map(&namespace_request());
        let mut response = json!({
            "output": [
                { "type": "function_call", "name": "plain_tool", "call_id": "x" }
            ]
        });
        assert!(!restore_response_namespaces(&mut response, &map));
        assert_eq!(response["output"][0]["name"], "plain_tool");
        assert!(response["output"][0].get("namespace").is_none());
    }

    #[test]
    fn long_flat_names_stay_consistent_between_flatten_and_restore() {
        let long_child = "a".repeat(80);
        let body = json!({
            "tools": [{
                "type": "namespace",
                "name": "mcp__srv__",
                "tools": [ { "type": "function", "name": long_child, "parameters": {} } ]
            }]
        });
        let mut flattened = body.clone();
        flatten_request_namespaces(&mut flattened).unwrap();
        let flat_name = flattened["tools"][0]["name"].as_str().unwrap().to_string();
        assert!(flat_name.len() <= 64);

        let map = namespace_restore_map(&body);
        let entry = map.get(&flat_name).unwrap();
        assert_eq!(entry.namespace, "mcp__srv__");
        assert_eq!(entry.name, long_child);

        // Same algorithm as Chat conversion helper.
        assert_eq!(
            flat_name,
            flatten_namespace_tool_name("mcp__srv__", &long_child)
        );
    }

    #[test]
    fn strips_external_web_access_recursively() {
        let mut body = json!({
            "model": "grok-4.5",
            "external_web_access": true,
            "tools": [
                {"type": "function", "name": "f", "external_web_access": true,
                 "parameters": {"type": "object", "q": {"external_web_access": true}}}
            ],
            "metadata": {"external_web_access": false}
        });
        assert!(sanitize_xai_responses_request(&mut body));
        let s = body.to_string();
        assert!(!s.contains("external_web_access"), "left over: {s}");
    }

    #[test]
    fn strips_top_level_unsupported_fields() {
        let mut body = json!({
            "model": "grok-4.5",
            "prompt_cache_retention": "24h",
            "safety_identifier": "abc"
        });
        assert!(sanitize_xai_responses_request(&mut body));
        assert!(body.get("prompt_cache_retention").is_none());
        assert!(body.get("safety_identifier").is_none());
    }

    #[test]
    fn strips_grok_45_only_sampling_fields() {
        let mut body = json!({
            "model": "grok-4.5",
            "presence_penalty": 0.1,
            "frequency_penalty": 0.2,
            "stop": ["x"]
        });
        assert!(sanitize_xai_responses_request(&mut body));
        assert!(body.get("presence_penalty").is_none());
        assert!(body.get("frequency_penalty").is_none());
        assert!(body.get("stop").is_none());
    }

    #[test]
    fn keeps_sampling_fields_for_non_grok_45() {
        let mut body = json!({
            "model": "grok-4-fast",
            "presence_penalty": 0.1,
            "stop": ["x"]
        });
        assert!(!sanitize_xai_responses_request(&mut body));
        assert_eq!(body.get("presence_penalty"), Some(&json!(0.1)));
        assert_eq!(body.get("stop"), Some(&json!(["x"])));
    }

    #[test]
    fn matches_grok_45_with_provider_prefix() {
        let mut body = json!({"model": "xai/grok-4.5", "stop": ["x"]});
        assert!(sanitize_xai_responses_request(&mut body));
        assert!(body.get("stop").is_none());
    }

    #[test]
    fn promotes_additional_tools_dedup() {
        let mut body = json!({
            "model": "grok-4.5",
            "tools": [{"type": "function", "name": "kept"}],
            "input": [
                {"type": "message", "role": "user", "content": "hi"},
                {"type": "additional_tools", "tools": [
                    {"type": "function", "name": "kept"},
                    {"type": "function", "name": "extra"}
                ]}
            ]
        });
        assert!(sanitize_xai_responses_request(&mut body));
        let input = body.get("input").unwrap().as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert!(input.iter().all(|i| !is_additional_tools_item(i)));
        let tools = body.get("tools").unwrap().as_array().unwrap();
        let names: Vec<&str> = tools
            .iter()
            .map(|t| t.get("name").and_then(Value::as_str).unwrap())
            .collect();
        assert_eq!(names, vec!["kept", "extra"]);
    }

    #[test]
    fn strips_null_reasoning_content() {
        let mut body = json!({
            "model": "grok-4.5",
            "input": [
                {"type": "reasoning", "content": null, "id": "r1"},
                {"type": "reasoning", "content": [{"text": "keep"}], "id": "r2"}
            ]
        });
        assert!(sanitize_xai_responses_request(&mut body));
        let input = body.get("input").unwrap().as_array().unwrap();
        assert!(input[0].get("content").is_none());
        assert!(input[1].get("content").is_some());
    }

    #[test]
    fn filters_unsupported_tool_types() {
        let mut body = json!({
            "model": "grok-4.5",
            "tools": [
                {"type": "function", "name": "f"},
                {"type": "tool_search"},
                {"type": "custom", "name": "c"},
                {"type": "mcp", "server_label": "s"}
            ]
        });
        assert!(sanitize_xai_responses_request(&mut body));
        let types: Vec<&str> = body
            .get("tools")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.get("type").and_then(Value::as_str).unwrap())
            .collect();
        assert_eq!(types, vec!["function", "mcp"]);
    }

    #[test]
    fn drops_dangling_function_tool_choice() {
        let mut body = json!({
            "model": "grok-4.5",
            "tools": [{"type": "tool_search"}],
            "tool_choice": {"type": "function", "name": "gone"}
        });
        assert!(sanitize_xai_responses_request(&mut body));
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    #[test]
    fn keeps_valid_function_tool_choice() {
        let mut body = json!({
            "model": "grok-4.5",
            "tools": [{"type": "function", "name": "run"}],
            "tool_choice": {"type": "function", "name": "run"}
        });
        assert!(!sanitize_xai_responses_request(&mut body));
        assert_eq!(
            body.get("tool_choice").unwrap(),
            &json!({"type": "function", "name": "run"})
        );
    }

    #[test]
    fn keeps_string_tool_choice() {
        let mut body = json!({
            "model": "grok-4.5",
            "tools": [{"type": "function", "name": "run"}],
            "tool_choice": "auto"
        });
        assert!(!sanitize_xai_responses_request(&mut body));
        assert_eq!(body.get("tool_choice").unwrap(), &json!("auto"));
    }

    #[test]
    fn passthrough_flattens_then_sanitizes_tool_search() {
        let mut body = namespace_request();
        body.as_object_mut().unwrap().insert(
            "tools".to_string(),
            json!([
                {
                    "type": "namespace",
                    "name": "mcp__files__",
                    "tools": [
                        { "type": "function", "name": "read", "parameters": {} }
                    ]
                },
                { "type": "tool_search" },
                { "type": "function", "name": "plain", "parameters": {} }
            ]),
        );
        body.as_object_mut()
            .unwrap()
            .insert("prompt_cache_retention".to_string(), json!("24h"));
        apply_xai_responses_passthrough(&mut body).unwrap();

        let tools = body["tools"].as_array().unwrap();
        let types: Vec<&str> = tools.iter().map(|t| t["type"].as_str().unwrap()).collect();
        assert_eq!(types, vec!["function", "function"]);
        assert!(body.get("prompt_cache_retention").is_none());
        assert!(tools.iter().any(|t| t["name"] == "mcp__files____read"));
        assert!(tools.iter().any(|t| t["name"] == "plain"));
    }

    #[test]
    fn sanitize_is_idempotent() {
        let mut body = json!({
            "model": "grok-4.5",
            "external_web_access": true,
            "prompt_cache_retention": "24h",
            "tools": [{"type": "function", "name": "f"}, {"type": "tool_search"}]
        });
        assert!(sanitize_xai_responses_request(&mut body));
        assert!(!sanitize_xai_responses_request(&mut body));
    }

    #[tokio::test]
    async fn sse_stream_restores_function_call_events_and_passes_others_through() {
        let map = namespace_restore_map(&namespace_request());

        let added = "event: response.output_item.added\n\
                     data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"name\":\"mcp__files____read\",\"call_id\":\"c1\"}}\n\n";
        let delta = "event: response.output_text.delta\n\
                     data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n";
        let done = "data: [DONE]\n\n";

        let input: DebugBodyStream = Box::pin(stream::iter(vec![
            Ok(added.as_bytes().to_vec()),
            Ok(delta.as_bytes().to_vec()),
            Ok(done.as_bytes().to_vec()),
        ]));
        let out = wrap_namespace_restore_sse_stream(input, map);
        futures_util::pin_mut!(out);

        let mut collected = String::new();
        while let Some(chunk) = out.next().await {
            collected.push_str(std::str::from_utf8(&chunk.unwrap()).unwrap());
        }

        assert!(collected.contains("\"name\":\"read\""));
        assert!(collected.contains("\"namespace\":\"mcp__files__\""));
        assert!(!collected.contains("mcp__files____read"));
        assert!(collected.contains("\"delta\":\"hi\""));
        assert!(collected.contains("[DONE]"));
    }

    #[test]
    fn xai_gap_closure_include_union_schema_agent_message_web_search_and_float_guard() {
        let mut body = json!({
            "model": "grok-4.7",
            "include": ["reasoning.encrypted_content", "output.encrypted_content"],
            "tools": [
                {
                    "type": "function",
                    "name": "union_tool",
                    "parameters": {
                        "type": "object",
                        "oneOf": [
                            {"type": "object", "properties": {"a": {"type": "string"}, "shared": {"type": "integer"}}, "required": ["a", "shared"]},
                            {"type": "object", "properties": {"b": {"type": "string"}, "shared": {"type": "integer"}}, "required": ["b", "shared"]},
                            {"type": "null"}
                        ]
                    }
                },
                {
                    "type": "function",
                    "name": "mcp__codex_app__automation_update",
                    "strict": true,
                    "parameters": {"anyOf": [{"type": "object"}, {"type": "null"}]}
                }
            ],
            "input": [
                {
                    "type": "agent_message",
                    "id": "am_1",
                    "content": [{"type": "encrypted_content", "encrypted_content": "run subagent task"}]
                }
            ]
        });

        apply_xai_responses_passthrough(&mut body).unwrap();
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(body["tools"][0]["parameters"]["type"], "object");
        assert_eq!(body["tools"][0]["parameters"]["required"], json!(["shared"]));
        assert_eq!(body["tools"][1]["strict"], false);
        assert_eq!(body["tools"][1]["parameters"]["additionalProperties"], true);
        assert_eq!(body["input"][0]["type"], "message");
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][0]["content"][0]["text"], "run subagent task");

        // Forced web_search tool_choice rewrite
        let mut search_body = json!({
            "model": "grok-4.7",
            "tools": [{"type": "function", "name": "bash"}, {"type": "web_search"}],
            "tool_choice": {"type": "web_search"}
        });
        assert!(rewrite_xai_hosted_web_search_tool_choice(&mut search_body));
        assert_eq!(search_body["tool_choice"], "required");
        assert_eq!(search_body["tools"], json!([{"type": "web_search"}]));

        // Whole-float integer argument rewrite + >= 2^64 guard + skip delta
        let mut resp = json!({
            "output": [{
                "type": "function_call",
                "name": "seek",
                "arguments": "{\"line\": 42.0, \"ratio\": 1.5, \"huge\": 18446744073709551616.0}"
            }]
        });
        assert!(normalize_xai_function_call_integer_arguments(&mut resp));
        let args: Value =
            serde_json::from_str(resp["output"][0]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["line"].as_u64(), Some(42));
        assert_eq!(args["ratio"].as_f64(), Some(1.5));
        assert!(args["huge"].as_u64().is_none(), "2^64 must not saturate to u64::MAX");

        let mut delta = json!({
            "type": "response.function_call_arguments.delta",
            "delta": "{\"line\": 42.0}"
        });
        assert!(!normalize_xai_function_call_integer_arguments(&mut delta));
    }
}
