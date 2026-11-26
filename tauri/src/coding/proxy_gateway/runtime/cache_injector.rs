use serde_json::{json, Value};

const MAX_CACHE_CONTROL_BREAKPOINTS: usize = 4;
/// cc-switch only adds the older user anchor once the conversation has at
/// least four messages. Shorter histories are already covered by the latest
/// anchor and would only spend a breakpoint that has to be dropped next turn.
const PRIOR_USER_ANCHOR_MIN_MESSAGES: usize = 4;

pub(super) fn inject_cache_control(body: &mut Value) -> bool {
    let original = body.clone();

    normalize_message_contents(body);
    sanitize_unsupported_cache_controls(body);
    trim_cache_controls_to_limit(body, MAX_CACHE_CONTROL_BREAKPOINTS);

    let mut remaining = remaining_cache_control_budget(body);
    if remaining > 0 {
        // Structural anchors are only added when the client has not placed one
        // on tools/system already, and never beyond the Anthropic limit.
        remaining = ensure_structural_cache_controls(body, remaining);

        // A client that planned its own message breakpoints keeps them: its
        // anchors are prefix-stable, while replanning them every turn changes
        // the serialized prefix hash and makes every request pay a full cache
        // write (AxonHub 7444f537). Codex has no message breakpoints, so this
        // path still plans the latest message anchor plus, for longer
        // histories, the previous turn's user message (cc-switch b9263a80).
        if remaining > 0 && count_message_breakpoints(body) == 0 {
            inject_message_cache_controls(body, remaining);
        }
    }

    *body != original
}

fn remaining_cache_control_budget(body: &Value) -> usize {
    MAX_CACHE_CONTROL_BREAKPOINTS.saturating_sub(count_cache_controls(body))
}

fn count_message_breakpoints(body: &Value) -> usize {
    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return 0;
    };
    messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|block| block.get("cache_control").is_some())
        .count()
}

/// Trims client breakpoints down to Anthropic limit, dropping the earliest
/// message breakpoint first and only then the earliest structural one.
fn trim_cache_controls_to_limit(body: &mut Value, limit: usize) {
    while count_cache_controls(body) > limit {
        if !remove_earliest_message_breakpoint(body) && !remove_earliest_structural_breakpoint(body)
        {
            return;
        }
    }
}

fn remove_earliest_message_breakpoint(body: &mut Value) -> bool {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return false;
    };
    for message in messages {
        let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for block in content {
            if block.get("cache_control").is_some() {
                remove_cache_control(block);
                return true;
            }
        }
    }
    false
}

fn remove_earliest_structural_breakpoint(body: &mut Value) -> bool {
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            if tool.get("cache_control").is_some() {
                remove_cache_control(tool);
                return true;
            }
        }
    }

    if let Some(system) = body.get_mut("system").and_then(Value::as_array_mut) {
        for block in system {
            if block.get("cache_control").is_some() {
                remove_cache_control(block);
                return true;
            }
        }
    }

    false
}

fn normalize_message_contents(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        let Some(content) = message.get_mut("content") else {
            continue;
        };
        let Some(text) = content.as_str().filter(|text| !text.is_empty()) else {
            continue;
        };
        *content = json!([{ "type": "text", "text": text }]);
    }
}

fn remove_cache_control(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        object.remove("cache_control");
    }
}

/// Injects the tools/system anchors the client did not provide and returns the
/// breakpoint budget left for message anchors. The budget is never exceeded:
/// Anthropic rejects more than four breakpoints per request.
fn ensure_structural_cache_controls(body: &mut Value, budget: usize) -> usize {
    let mut remaining = budget;

    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        if remaining > 0 && !tools.iter().any(|tool| tool.get("cache_control").is_some()) {
            if let Some(last_tool) = tools.last_mut() {
                if inject_block(last_tool) {
                    remaining -= 1;
                }
            }
        }
    }

    if remaining == 0 {
        return remaining;
    }

    // A string system prompt must become a block array before an anchor can be
    // attached to its last block.
    if body.get("system").and_then(Value::as_str).is_some() {
        let text = body["system"].as_str().unwrap_or_default().to_string();
        if !text.is_empty() {
            body["system"] = json!([{ "type": "text", "text": text }]);
        }
    }

    if let Some(system) = body.get_mut("system").and_then(Value::as_array_mut) {
        if !system
            .iter()
            .any(|block| block.get("cache_control").is_some())
        {
            if let Some(last_block) = system.last_mut() {
                if inject_block(last_block) {
                    remaining -= 1;
                }
            }
        }
    }

    remaining
}

/// Marks the newest cacheable message and, when the budget and history allow,
/// the second-most-recent `user` message. The older anchor gives long
/// tool-result turns a read point inside Anthropic's ~20-block lookback, while
/// the newest anchor keeps the prefix stable as the conversation grows. This
/// mirrors cc-switch's planner (b9263a80); it deliberately does not use a
/// distance-from-end window, whose index moves on every turn.
fn inject_message_cache_controls(body: &mut Value, mut budget: usize) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };

    let Some(latest_index) = inject_latest_message_anchor(messages) else {
        return;
    };
    budget -= 1;

    if budget == 0 || messages.len() < PRIOR_USER_ANCHOR_MIN_MESSAGES {
        return;
    }

    let mut user_count = 0;
    for (index, message) in messages.iter_mut().enumerate().rev() {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        user_count += 1;
        if user_count == 2 {
            // The newest anchor can fall back to this message when the tail is
            // only thinking or empty text; never mark the same message twice.
            if index != latest_index {
                inject_last_cacheable_block(message);
            }
            break;
        }
    }
}

/// Returns the index of the message that received the newest anchor. `None`
/// means no message had a cacheable block, so there is nothing to anchor.
fn inject_latest_message_anchor(messages: &mut [Value]) -> Option<usize> {
    for (index, message) in messages.iter_mut().enumerate().rev() {
        if inject_last_cacheable_block(message) {
            return Some(index);
        }
    }
    None
}

/// Injects `cache_control` on the last block the cache can key on, skipping
/// `thinking` / `redacted_thinking` and empty text. Returns `false` when the
/// message has no such block or its last cacheable block is already marked.
fn inject_last_cacheable_block(message: &mut Value) -> bool {
    let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) else {
        return false;
    };
    let Some(block) = content
        .iter_mut()
        .rev()
        .find(|block| is_cacheable_message_block(block))
    else {
        return false;
    };
    if block.get("cache_control").is_some() {
        return false;
    }
    inject_block(block)
}

fn sanitize_unsupported_cache_controls(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for block in content {
            if !is_cacheable_message_block(block) {
                remove_cache_control(block);
            }
        }
    }
}

fn is_cacheable_message_block(block: &Value) -> bool {
    match block.get("type").and_then(Value::as_str) {
        Some("thinking" | "redacted_thinking") => false,
        Some("text") => block
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty()),
        _ => true,
    }
}

fn inject_block(block: &mut Value) -> bool {
    let Some(object) = block.as_object_mut() else {
        return false;
    };
    object.insert("cache_control".to_string(), json!({ "type": "ephemeral" }));
    true
}

fn count_cache_controls(value: &Value) -> usize {
    let mut count = 0;
    if let Some(tools) = value.get("tools").and_then(Value::as_array) {
        count += tools
            .iter()
            .filter(|tool| tool.get("cache_control").is_some())
            .count();
    }
    if let Some(system) = value.get("system").and_then(Value::as_array) {
        count += system
            .iter()
            .filter(|block| block.get("cache_control").is_some())
            .count();
    }
    if let Some(messages) = value.get("messages").and_then(Value::as_array) {
        for message in messages {
            let Some(content) = message.get("content").and_then(Value::as_array) else {
                continue;
            };
            count += content
                .iter()
                .filter(|block| block.get("cache_control").is_some())
                .count();
        }
    }
    count
}

#[cfg(test)]
fn message_block_has_cache_control(
    value: &Value,
    message_index: usize,
    block_index: usize,
) -> bool {
    value
        .pointer(&format!(
            "/messages/{message_index}/content/{block_index}/cache_control"
        ))
        .is_some()
}

#[cfg(test)]
fn tool_has_cache_control(value: &Value, tool_index: usize) -> bool {
    value
        .pointer(&format!("/tools/{tool_index}/cache_control"))
        .is_some()
}

#[cfg(test)]
fn system_has_cache_control(value: &Value, block_index: usize) -> bool {
    value
        .pointer(&format!("/system/{block_index}/cache_control"))
        .is_some()
}

#[cfg(test)]
fn text_block(index: usize) -> Value {
    json!({"type":"text","text":format!("block {index}")})
}

#[cfg(test)]
fn text_blocks(count: usize) -> Vec<Value> {
    (0..count).map(text_block).collect()
}

#[cfg(test)]
fn stale_cached_text_block(text: &str) -> Value {
    json!({"type":"text","text":text,"cache_control":{"type":"ephemeral"}})
}

#[cfg(test)]
fn stale_cached_thinking_block() -> Value {
    json!({"type":"thinking","thinking":"old","cache_control":{"type":"ephemeral"}})
}

#[cfg(test)]
fn stale_cached_empty_text_block() -> Value {
    json!({"type":"text","text":"","cache_control":{"type":"ephemeral"}})
}

#[cfg(test)]
fn stale_cached_tool() -> Value {
    json!({
        "name": "old_tool",
        "input_schema": {"type":"object"},
        "cache_control": {"type":"ephemeral"}
    })
}

#[cfg(test)]
fn tool(name: &str) -> Value {
    json!({"name":name,"input_schema":{"type":"object"}})
}

#[cfg(test)]
fn stale_cached_system_block(text: &str) -> Value {
    json!({"type":"text","text":text,"cache_control":{"type":"ephemeral"}})
}

#[cfg(test)]
fn assert_ephemeral(value: &Value) {
    assert_eq!(value.get("type").and_then(Value::as_str), Some("ephemeral"));
}

#[cfg(test)]
fn cache_control_at<'a>(value: &'a Value, pointer: &str) -> &'a Value {
    value.pointer(pointer).expect("cache_control should exist")
}

#[cfg(test)]
fn assert_no_cache_control_at(value: &Value, pointer: &str) {
    assert!(
        value.pointer(pointer).is_none(),
        "{pointer} should not have cache_control"
    );
}

#[cfg(test)]
fn assert_cache_control_count(value: &Value, expected: usize) {
    assert_eq!(count_cache_controls(value), expected);
}

#[cfg(test)]
fn assert_no_unsupported_cache_controls(value: &Value) {
    let Some(messages) = value.get("messages").and_then(Value::as_array) else {
        return;
    };
    for message in messages {
        let Some(content) = message.get("content").and_then(Value::as_array) else {
            continue;
        };
        for block in content {
            if !is_cacheable_message_block(block) {
                assert!(block.get("cache_control").is_none());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn injects_system_string_as_last_block() {
        let mut body = json!({
            "system": "You are helpful",
            "messages": [{"role": "user", "content": "hi"}]
        });

        assert!(inject_cache_control(&mut body));
        assert_ephemeral(cache_control_at(&body, "/system/0/cache_control"));
        assert_ephemeral(cache_control_at(
            &body,
            "/messages/0/content/0/cache_control",
        ));
    }

    #[test]
    fn injects_last_user_content_block() {
        let mut body = json!({
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "first"},
                    {"type": "text", "text": "last"}
                ]
            }]
        });

        assert!(inject_cache_control(&mut body));
        assert!(message_block_has_cache_control(&body, 0, 1));
        assert!(!message_block_has_cache_control(&body, 0, 0));
    }

    #[test]
    fn preserves_client_structural_breakpoints_and_fills_message_anchors() {
        let mut body = json!({
            "tools": [
                stale_cached_tool(),
                tool("new_tool")
            ],
            "system": [
                stale_cached_system_block("old"),
                {"type":"text","text":"current"}
            ],
            "messages": [
                {"role":"user","content": text_blocks(2)},
                {"role":"assistant","content": text_blocks(2)},
                {"role":"user","content": text_blocks(2)},
                {"role":"assistant","content": text_blocks(2)},
                {"role":"user","content": text_blocks(2)}
            ]
        });

        assert!(inject_cache_control(&mut body));

        // Client anchors stay where they were; the planner adds the newest
        // message anchor and the second-most-recent user anchor within the
        // remaining budget.
        assert_cache_control_count(&body, 4);
        assert!(tool_has_cache_control(&body, 0));
        assert!(!tool_has_cache_control(&body, 1));
        assert!(system_has_cache_control(&body, 0));
        assert!(!system_has_cache_control(&body, 1));
        assert!(!message_block_has_cache_control(&body, 0, 0));
        assert!(!message_block_has_cache_control(&body, 1, 0));
        assert!(message_block_has_cache_control(&body, 2, 1));
        assert!(!message_block_has_cache_control(&body, 3, 0));
        assert!(message_block_has_cache_control(&body, 4, 1));
    }

    fn tool_loop_body(tool_results: usize) -> Value {
        let mut messages = vec![json!({
            "role": "user",
            "content": [{"type": "text", "text": "start"}]
        })];
        for index in 0..tool_results {
            messages.push(json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": format!("call_{index}"),
                    "name": "Read",
                    "input": {}
                }]
            }));
            messages.push(json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": format!("call_{index}"),
                    "content": format!("result {index}")
                }]
            }));
        }
        json!({
            "tools": [tool("Read")],
            "system": [{"type": "text", "text": "system"}],
            "messages": messages
        })
    }

    #[test]
    fn continuous_tool_loop_anchors_latest_and_prior_user() {
        let mut first_turn = tool_loop_body(2);
        assert!(inject_cache_control(&mut first_turn));
        assert_cache_control_count(&first_turn, 4);
        assert!(message_block_has_cache_control(&first_turn, 2, 0));
        assert!(message_block_has_cache_control(&first_turn, 4, 0));
        assert!(!message_block_has_cache_control(&first_turn, 0, 0));

        let mut second_turn = tool_loop_body(3);
        assert!(inject_cache_control(&mut second_turn));
        assert_cache_control_count(&second_turn, 4);
        // The previous turn's newest anchor (message 4) is kept as this turn's
        // older anchor; only the older anchor from the first turn (message 2)
        // is dropped.
        assert!(message_block_has_cache_control(&second_turn, 4, 0));
        assert!(message_block_has_cache_control(&second_turn, 6, 0));
        assert!(!message_block_has_cache_control(&second_turn, 2, 0));
    }

    #[test]
    fn short_history_below_four_messages_only_marks_latest_message() {
        let mut body = json!({
            "tools": [tool("Read")],
            "system": [{"type": "text", "text": "system"}],
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "u0"}]},
                {"role": "assistant", "content": [{"type": "text", "text": "a0"}]},
                {"role": "user", "content": [{"type": "text", "text": "u1"}]}
            ]
        });

        assert!(inject_cache_control(&mut body));

        assert_cache_control_count(&body, 3);
        assert!(message_block_has_cache_control(&body, 2, 0));
        assert!(!message_block_has_cache_control(&body, 0, 0));
    }

    #[test]
    fn long_history_ending_with_assistant_marks_latest_assistant_and_second_last_user() {
        let mut body = json!({
            "tools": [tool("Read")],
            "system": [{"type": "text", "text": "system"}],
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "first"}]},
                {"role": "assistant", "content": [{"type": "text", "text": "answer"}]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "c1", "content": "result"}]},
                {"role": "assistant", "content": [{"type": "text", "text": "latest"}]}
            ]
        });

        assert!(inject_cache_control(&mut body));

        assert_cache_control_count(&body, 4);
        assert!(message_block_has_cache_control(&body, 0, 0));
        assert!(message_block_has_cache_control(&body, 3, 0));
        assert!(!message_block_has_cache_control(&body, 2, 0));
    }

    #[test]
    fn does_not_duplicate_when_latest_fallback_lands_on_prior_user() {
        let mut body = json!({
            "tools": [tool("Read")],
            "system": [{"type": "text", "text": "system"}],
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "u0"}]},
                {"role": "assistant", "content": [{"type": "text", "text": "a0"}]},
                {"role": "user", "content": [{"type": "text", "text": "u1"}]},
                {"role": "assistant", "content": [{"type": "thinking", "thinking": "thought"}]},
                {"role": "user", "content": [{"type": "text", "text": ""}]}
            ]
        });

        assert!(inject_cache_control(&mut body));

        assert_cache_control_count(&body, 3);
        assert!(message_block_has_cache_control(&body, 2, 0));
        assert!(!message_block_has_cache_control(&body, 0, 0));
        assert_no_unsupported_cache_controls(&body);
    }

    #[test]
    fn skips_prior_user_when_only_one_budget_slot_remains() {
        let mut body = json!({
            "tools": [
                {"name": "t1", "input_schema": {"type": "object"}, "cache_control": {"type": "ephemeral"}},
                {"name": "t2", "input_schema": {"type": "object"}, "cache_control": {"type": "ephemeral"}}
            ],
            "system": [{"type": "text", "text": "system", "cache_control": {"type": "ephemeral"}}],
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "u0"}]},
                {"role": "assistant", "content": [{"type": "text", "text": "a0"}]},
                {"role": "user", "content": [{"type": "text", "text": "u1"}]},
                {"role": "assistant", "content": [{"type": "text", "text": "a1"}]},
                {"role": "user", "content": [{"type": "text", "text": "u2"}]}
            ]
        });

        assert!(inject_cache_control(&mut body));

        assert_cache_control_count(&body, 4);
        assert!(message_block_has_cache_control(&body, 4, 0));
        assert!(!message_block_has_cache_control(&body, 2, 0));
    }

    #[test]
    fn latest_and_prior_user_skip_thinking_redacted_thinking_and_empty_text() {
        let mut body = json!({
            "tools": [tool("Read")],
            "system": [{"type": "text", "text": "system"}],
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "u0"}]},
                {"role": "assistant", "content": [{"type": "text", "text": "a0"}]},
                {"role": "user", "content": [
                    {"type": "text", "text": "prior-valid"},
                    {"type": "thinking", "thinking": "prior-thought"},
                    {"type": "redacted_thinking", "data": "prior-redacted"},
                    {"type": "text", "text": ""}
                ]},
                {"role": "assistant", "content": [{"type": "text", "text": "a1"}]},
                {"role": "user", "content": [
                    {"type": "text", "text": "latest-valid"},
                    {"type": "thinking", "thinking": "latest-thought"},
                    {"type": "redacted_thinking", "data": "latest-redacted"},
                    {"type": "text", "text": ""}
                ]}
            ]
        });

        assert!(inject_cache_control(&mut body));

        assert_cache_control_count(&body, 4);
        assert!(message_block_has_cache_control(&body, 2, 0));
        assert!(message_block_has_cache_control(&body, 4, 0));
        assert!(!message_block_has_cache_control(&body, 2, 3));
        assert!(!message_block_has_cache_control(&body, 4, 1));
        assert_no_unsupported_cache_controls(&body);
    }

    #[test]
    fn keeps_client_message_breakpoints_without_replanning() {
        let mut body = json!({
            "messages": [
                {
                    "role":"user",
                    "content": [{"type":"text","text":"first","cache_control":{"type":"ephemeral"}}]
                },
                {
                    "role":"user",
                    "content": text_blocks(25)
                }
            ]
        });

        // Nothing to do: the client already planned message breakpoints, so no
        // planner anchor is added (AxonHub 7444f537) and the body stays
        // byte-identical upstream.
        assert!(!inject_cache_control(&mut body));
        assert_cache_control_count(&body, 1);
        assert!(message_block_has_cache_control(&body, 0, 0));
        for block_index in 0..25 {
            assert!(!message_block_has_cache_control(&body, 1, block_index));
        }
    }

    #[test]
    fn keeps_structural_anchors_within_breakpoint_limit() {
        let mut body = json!({
            "tools": [tool("unmarked")],
            "system": [{"type":"text","text":"unmarked system"}],
            "messages": [{
                "role":"user",
                "content": [
                    {"type":"text","text":"client-1","cache_control":{"type":"ephemeral"}},
                    {"type":"text","text":"client-2","cache_control":{"type":"ephemeral"}},
                    {"type":"text","text":"client-3","cache_control":{"type":"ephemeral"}}
                ]
            }]
        });

        assert!(inject_cache_control(&mut body));

        // Three client breakpoints leave a single free slot: tools is filled
        // first and the unmarked system block stays unmarked instead of
        // pushing the request over Anthropic's four-breakpoint limit.
        assert_cache_control_count(&body, 4);
        assert!(tool_has_cache_control(&body, 0));
        assert!(!system_has_cache_control(&body, 0));
        assert!(message_block_has_cache_control(&body, 0, 0));
        assert!(message_block_has_cache_control(&body, 0, 1));
        assert!(message_block_has_cache_control(&body, 0, 2));
    }

    #[test]
    fn keeps_string_system_untouched_when_budget_is_spent() {
        let mut body = json!({
            "system": "You are helpful",
            "messages": [{
                "role":"user",
                "content": [
                    {"type":"text","text":"client-1","cache_control":{"type":"ephemeral"}},
                    {"type":"text","text":"client-2","cache_control":{"type":"ephemeral"}},
                    {"type":"text","text":"client-3","cache_control":{"type":"ephemeral"}},
                    {"type":"text","text":"client-4","cache_control":{"type":"ephemeral"}}
                ]
            }]
        });

        // The client already spent the whole budget, so normalizing the string
        // system prompt would only churn the upstream bytes.
        assert!(!inject_cache_control(&mut body));
        assert_eq!(body["system"], json!("You are helpful"));
        assert_cache_control_count(&body, 4);
    }
    #[test]
    fn trims_excess_client_breakpoints_deterministically() {
        let mut body = json!({
            "tools": [stale_cached_tool()],
            "system": [stale_cached_system_block("kept")],
            "messages": [{
                "role":"user",
                "content": [
                    {"type":"text","text":"dropped","cache_control":{"type":"ephemeral"}},
                    {"type":"text","text":"kept-1","cache_control":{"type":"ephemeral"}},
                    {"type":"text","text":"kept-2","cache_control":{"type":"ephemeral"}}
                ]
            }]
        });

        assert!(inject_cache_control(&mut body));

        // Anthropic allows four breakpoints: the earliest message breakpoint is
        // dropped first, structural anchors and later message anchors survive.
        assert_cache_control_count(&body, 4);
        assert!(tool_has_cache_control(&body, 0));
        assert!(system_has_cache_control(&body, 0));
        assert!(!message_block_has_cache_control(&body, 0, 0));
        assert!(message_block_has_cache_control(&body, 0, 1));
        assert!(message_block_has_cache_control(&body, 0, 2));
    }

    #[test]
    fn does_not_cache_thinking_or_empty_text_blocks() {
        let mut body = json!({
            "messages": [{
                "role":"user",
                "content": [
                    stale_cached_thinking_block(),
                    stale_cached_empty_text_block(),
                    stale_cached_text_block("usable")
                ]
            }]
        });

        assert!(inject_cache_control(&mut body));

        assert_cache_control_count(&body, 1);
        assert_no_unsupported_cache_controls(&body);
        assert_no_cache_control_at(&body, "/messages/0/content/0/cache_control");
        assert_no_cache_control_at(&body, "/messages/0/content/1/cache_control");
        assert!(message_block_has_cache_control(&body, 0, 2));
    }
}
