use super::*;
use crate::coding::proxy_gateway::transformer::anthropic::anthropic_request_to_llm;
use crate::coding::proxy_gateway::transformer::gemini::{
    gemini_request_to_llm, gemini_response_to_llm, llm_request_to_gemini, llm_response_to_gemini,
};
use crate::coding::proxy_gateway::transformer::llm::{
    ApiFormat, FunctionCall, Message, MessageContent, RequestType, ToolCall, TOOL_TYPE_FUNCTION,
    TOOL_TYPE_GOOGLE_CODE_EXECUTION, TOOL_TYPE_GOOGLE_SEARCH, TOOL_TYPE_GOOGLE_URL_CONTEXT,
    TOOL_TYPE_RESPONSES_CUSTOM_TOOL,
};
use crate::coding::proxy_gateway::transformer::openai::chat::{
    chat_request_to_llm, chat_response_to_llm, llm_response_to_chat,
};
use crate::coding::proxy_gateway::transformer::openai::responses::{
    llm_response_to_responses, responses_compact_request_to_llm, responses_request_to_llm,
    responses_response_to_llm,
};
use crate::coding::proxy_gateway::transformer::shared::signature::DEFAULT_GEMINI_THOUGHT_SIGNATURE;
use futures_util::{stream, StreamExt};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

const PROTOCOLS: [AiProtocol; 4] = [
    AiProtocol::AnthropicMessages,
    AiProtocol::OpenAiChat,
    AiProtocol::OpenAiResponses,
    AiProtocol::GeminiNative,
];
const OPENAI_RESPONSES_FIXTURE_ENCRYPTED_CONTENT: &str =
    "fixture-openai-responses-encrypted-content";

fn openai_responses_heuristic_signature() -> String {
    ["g", "AAAAABfixture-openai-responses-signature"].concat()
}

/// Builds a realistic Anthropic thinking-signature fixture: standard base64
/// whose decoded opaque bytes embed a Claude model marker, so
/// `guess_signature_provider` recognizes it as Anthropic without an explicit
/// marker. The `label` keeps parallel SSE tests distinguishable.
fn anthropic_signature_fixture(label: &str) -> String {
    use base64::Engine;
    let mut bytes = vec![0x12, 0xad, 0x02, 0x0a, 0x89, 0x01];
    bytes.extend_from_slice(b"claude-sonnet-5 ");
    bytes.extend_from_slice(label.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(&bytes)
}

fn request_fixture(protocol: AiProtocol) -> Value {
    match protocol {
        AiProtocol::AnthropicMessages => json!({
            "model": "model-a",
            "system": "system",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": [{"type": "tool_use", "id": "call_1", "name": "read_file", "input": {"path": "a.txt"}}]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "call_1", "content": "ok"}]}
            ],
            "tools": [{"name": "read_file", "input_schema": {"type": "object"}}],
            "max_tokens": 64,
            "stream": true
        }),
        AiProtocol::OpenAiChat => json!({
            "model": "model-a",
            "messages": [
                {"role": "system", "content": "system"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"a.txt\"}"}}]},
                {"role": "tool", "tool_call_id": "call_1", "content": "ok"}
            ],
            "tools": [{"type": "function", "function": {"name": "read_file", "parameters": {"type": "object"}}}],
            "max_tokens": 64,
            "stream": true
        }),
        AiProtocol::OpenAiResponses => json!({
            "model": "model-a",
            "instructions": "system",
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]},
                {"type": "function_call", "call_id": "call_1", "name": "read_file", "arguments": "{\"path\":\"a.txt\"}"},
                {"type": "function_call_output", "call_id": "call_1", "output": "ok"}
            ],
            "tools": [{"type": "function", "name": "read_file", "parameters": {"type": "object"}}],
            "max_output_tokens": 64,
            "stream": true
        }),
        AiProtocol::GeminiNative => json!({
            "model": "model-a",
            "systemInstruction": {"parts": [{"text": "system"}]},
            "contents": [
                {"role": "user", "parts": [{"text": "hi"}]},
                {"role": "model", "parts": [{"functionCall": {"id": "call_1", "name": "read_file", "args": {"path": "a.txt"}}}]},
                {"role": "user", "parts": [{"functionResponse": {"id": "call_1", "name": "read_file", "response": {"content": "ok"}}}]}
            ],
            "tools": [{"functionDeclarations": [{"name": "read_file", "parameters": {"type": "object"}}]}],
            "generationConfig": {"maxOutputTokens": 64},
            "stream": true
        }),
    }
}

fn response_fixture(protocol: AiProtocol) -> Value {
    match protocol {
        AiProtocol::AnthropicMessages => json!({
            "id": "resp_1",
            "type": "message",
            "role": "assistant",
            "model": "model-a",
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "tool_use", "id": "call_1", "name": "read_file", "input": {"path": "a.txt"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 8, "cache_read_input_tokens": 2, "output_tokens": 3}
        }),
        AiProtocol::OpenAiChat => json!({
            "id": "resp_1",
            "object": "chat.completion",
            "model": "model-a",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "hello",
                    "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"a.txt\"}"}}]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13, "prompt_tokens_details": {"cached_tokens": 2}}
        }),
        AiProtocol::OpenAiResponses => json!({
            "id": "resp_1",
            "object": "response",
            "model": "model-a",
            "status": "completed",
            "output": [
                {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "hello"}]},
                {"type": "function_call", "call_id": "call_1", "name": "read_file", "arguments": "{\"path\":\"a.txt\"}"}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 3, "total_tokens": 13, "input_tokens_details": {"cached_tokens": 2}}
        }),
        AiProtocol::GeminiNative => json!({
            "responseId": "resp_1",
            "modelVersion": "model-a",
            "candidates": [{
                "content": {"role": "model", "parts": [
                    {"text": "hello"},
                    {"functionCall": {"id": "call_1", "name": "read_file", "args": {"path": "a.txt"}}}
                ]},
                "finishReason": "STOP"
            }],
            "usageMetadata": {"promptTokenCount": 10, "cachedContentTokenCount": 2, "candidatesTokenCount": 3, "totalTokenCount": 13}
        }),
    }
}

fn assert_request_shape(protocol: AiProtocol, value: &Value) {
    match protocol {
        AiProtocol::AnthropicMessages => {
            assert!(value.get("messages").and_then(Value::as_array).is_some());
        }
        AiProtocol::OpenAiChat => {
            assert!(value.get("messages").and_then(Value::as_array).is_some());
        }
        AiProtocol::OpenAiResponses => {
            assert!(value.get("input").and_then(Value::as_array).is_some());
        }
        AiProtocol::GeminiNative => {
            assert!(value.get("contents").and_then(Value::as_array).is_some());
        }
    }
}

fn assert_response_shape(protocol: AiProtocol, value: &Value) {
    match protocol {
        AiProtocol::AnthropicMessages => {
            assert_eq!(value["type"], "message");
            assert!(value.get("content").and_then(Value::as_array).is_some());
        }
        AiProtocol::OpenAiChat => {
            assert_eq!(value["object"], "chat.completion");
            assert!(value.get("choices").and_then(Value::as_array).is_some());
        }
        AiProtocol::OpenAiResponses => {
            assert_eq!(value["object"], "response");
            assert!(value.get("output").and_then(Value::as_array).is_some());
        }
        AiProtocol::GeminiNative => {
            assert!(value.get("candidates").and_then(Value::as_array).is_some());
        }
    }
}

fn stream_fixture(protocol: AiProtocol) -> String {
    match protocol {
            AiProtocol::AnthropicMessages => [
                r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","model":"model-a","usage":{"input_tokens":11,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}

"#,
                r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}

"#,
                r#"event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"thinking_delta","thinking":"think"}}

"#,
                r#"event: content_block_start
data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"call_1","name":"read_file"}}

"#,
                r#"event: content_block_delta
data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}

"#,
                r#"event: content_block_delta
data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"\"a.txt\"}"}}

"#,
                r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":3}}

"#,
                r#"event: message_stop
data: {"type":"message_stop"}

"#,
            ]
            .concat(),
            AiProtocol::OpenAiChat => [
                r#"data: {"id":"chat_1","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_1","model":"model-a","choices":[{"index":0,"delta":{"content":"hello"}}]}

"#,
                r#"data: {"id":"chat_1","model":"model-a","choices":[{"index":0,"delta":{"reasoning_content":"think"}}]}

"#,
                r#"data: {"id":"chat_1","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}

"#,
                r#"data: {"id":"chat_1","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"type":"function","function":{"arguments":"\"a.txt\"}"}}]}}]}

"#,
                r#"data: {"id":"chat_1","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":11,"completion_tokens":3,"total_tokens":14}}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
            AiProtocol::OpenAiResponses => [
                r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_1","model":"model-a"}}

"#,
                r#"event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"hello","item_id":"msg_1","output_index":0,"content_index":0}

"#,
                r#"event: response.reasoning_summary_text.delta
data: {"type":"response.reasoning_summary_text.delta","delta":"think","item_id":"reasoning_1","output_index":1,"summary_index":0}

"#,
                r#"event: response.output_item.added
data: {"type":"response.output_item.added","output_index":2,"item":{"id":"item_1","type":"function_call","call_id":"call_1","name":"read_file"}}

"#,
                r#"event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","item_id":"item_1","output_index":2,"delta":"{\"path\":"}

"#,
                r#"event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","item_id":"item_1","output_index":2,"delta":"\"a.txt\"}"}

"#,
                r#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_1","model":"model-a","status":"completed","usage":{"input_tokens":11,"output_tokens":3,"total_tokens":14}}}

"#,
            ]
            .concat(),
            AiProtocol::GeminiNative => [
                r#"data: {"responseId":"gemini_1","modelVersion":"model-a","candidates":[{"content":{"role":"model","parts":[{"text":"hel"}]}}]}

"#,
                r#"data: {"responseId":"gemini_1","modelVersion":"model-a","candidates":[{"content":{"role":"model","parts":[{"text":"hello"},{"text":"think","thought":true}]}}]}

"#,
                r#"data: {"responseId":"gemini_1","modelVersion":"model-a","candidates":[{"content":{"role":"model","parts":[{"text":"hello"},{"text":"think","thought":true},{"functionCall":{"id":"call_1","name":"read_file","args":{"path":"a.txt"}}}]}}]}

"#,
                r#"data: {"responseId":"gemini_1","modelVersion":"model-a","candidates":[{"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":11,"candidatesTokenCount":3,"totalTokenCount":14}}

"#,
            ]
            .concat(),
        }
}

fn collect_stream(route: ConversionRoute, input: String) -> String {
    let chunks = input
        .as_bytes()
        .chunks(11)
        .map(|chunk| Ok(chunk.to_vec()))
        .collect::<Vec<Result<Vec<u8>, String>>>();
    let mut output = convert_sse_stream(route, Box::pin(stream::iter(chunks)));
    let bytes = tauri::async_runtime::block_on(async move {
        let mut bytes = Vec::new();
        while let Some(chunk) = output.next().await {
            bytes.extend(chunk.expect("converted stream chunk"));
        }
        bytes
    });
    String::from_utf8(bytes).expect("converted stream should be utf8")
}

fn collect_stream_with_context(
    route: ConversionRoute,
    input: String,
    context: ConversionContext,
) -> String {
    let chunks = input
        .as_bytes()
        .chunks(11)
        .map(|chunk| Ok(chunk.to_vec()))
        .collect::<Vec<Result<Vec<u8>, String>>>();
    let mut output =
        convert_sse_stream_with_context(route, Box::pin(stream::iter(chunks)), Some(context));
    let bytes = tauri::async_runtime::block_on(async move {
        let mut bytes = Vec::new();
        while let Some(chunk) = output.next().await {
            bytes.extend(chunk.expect("converted stream chunk"));
        }
        bytes
    });
    String::from_utf8(bytes).expect("converted stream should be utf8")
}

fn collect_stream_chunks(
    route: ConversionRoute,
    chunks: Vec<Result<Vec<u8>, String>>,
) -> (String, Vec<String>) {
    let mut output = convert_sse_stream(route, Box::pin(stream::iter(chunks)));
    let (bytes, errors) = tauri::async_runtime::block_on(async move {
        let mut bytes = Vec::new();
        let mut errors = Vec::new();
        while let Some(chunk) = output.next().await {
            match chunk {
                Ok(chunk) => bytes.extend(chunk),
                Err(error) => errors.push(error),
            }
        }
        (bytes, errors)
    });
    (
        String::from_utf8(bytes).expect("converted stream should be utf8"),
        errors,
    )
}

fn push_stream_chunk(kernel: &mut StreamKernel, chunk: &str) -> String {
    let bytes = kernel
        .push_chunk(chunk.as_bytes())
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    String::from_utf8(bytes).expect("converted stream should be utf8")
}

fn finish_stream(kernel: &mut StreamKernel) -> String {
    let bytes = kernel.finish().into_iter().flatten().collect::<Vec<_>>();
    String::from_utf8(bytes).expect("converted stream should be utf8")
}

fn sse_data_values(output: &str) -> Vec<Value> {
    output
        .split("\n\n")
        .filter_map(|block| {
            let data = block.lines().find_map(|line| line.strip_prefix("data: "))?;
            if data == "[DONE]" {
                return None;
            }
            serde_json::from_str::<Value>(data).ok()
        })
        .collect()
}

fn occurrence_count(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

fn assert_stream_shape(protocol: AiProtocol, output: &str) {
    match protocol {
        AiProtocol::AnthropicMessages => {
            assert!(output.contains("event: message_start"));
            assert!(output.contains("text_delta"));
            assert!(output.contains("thinking_delta"));
            assert!(output.contains("tool_use"));
            assert!(output.contains("input_json_delta"));
            assert!(output.contains("event: message_stop"));
            // Usage must survive cross-protocol stream conversion.
            assert!(
                output.contains(r#""input_tokens":11"#) || output.contains(r#""input_tokens": 11"#),
                "Anthropic target must preserve input_tokens; output={output}"
            );
            assert!(
                output.contains(r#""output_tokens":3"#) || output.contains(r#""output_tokens": 3"#),
                "Anthropic target must preserve output_tokens; output={output}"
            );
        }
        AiProtocol::OpenAiChat => {
            assert!(output.contains("chat.completion.chunk"));
            assert!(output.contains(r#""content":"#));
            assert!(output.contains("reasoning_content"));
            assert!(output.contains("tool_calls"));
            assert!(output.contains("[DONE]"));
            assert!(
                output.contains(r#""prompt_tokens":11"#)
                    || output.contains(r#""prompt_tokens": 11"#),
                "Chat target must preserve prompt_tokens; output={output}"
            );
            assert!(
                output.contains(r#""completion_tokens":3"#)
                    || output.contains(r#""completion_tokens": 3"#),
                "Chat target must preserve completion_tokens; output={output}"
            );
        }
        AiProtocol::OpenAiResponses => {
            assert!(output.contains("event: response.created"));
            assert!(output.contains("response.output_text.delta"));
            assert!(output.contains("response.reasoning_summary_text.delta"));
            assert!(output.contains("response.output_item.added"));
            assert!(output.contains("response.function_call_arguments.done"));
            assert!(output.contains("event: response.completed"));
            assert!(
                output.contains(r#""input_tokens":11"#) || output.contains(r#""input_tokens": 11"#),
                "Responses target must preserve input_tokens; output={output}"
            );
            assert!(
                output.contains(r#""output_tokens":3"#) || output.contains(r#""output_tokens": 3"#),
                "Responses target must preserve output_tokens; output={output}"
            );
        }
        AiProtocol::GeminiNative => {
            assert!(output.contains("candidates"));
            assert!(output.contains(r#""text":"hello""#));
            assert!(output.contains("functionCall"));
            assert!(output.contains("finishReason"));
            assert!(
                output.contains(r#""promptTokenCount":11"#)
                    || output.contains(r#""promptTokenCount": 11"#),
                "Gemini target must preserve promptTokenCount; output={output}"
            );
            assert!(
                output.contains(r#""candidatesTokenCount":3"#)
                    || output.contains(r#""candidatesTokenCount": 3"#),
                "Gemini target must preserve candidatesTokenCount; output={output}"
            );
        }
    }
}

fn assert_stream_basic_shape(protocol: AiProtocol, output: &str) {
    match protocol {
        AiProtocol::AnthropicMessages => {
            let is_error = output.contains("event: error");
            assert!(is_error || output.contains("event: message_start"));
            assert!(is_error || output.contains("event: message_stop"));
        }
        AiProtocol::OpenAiChat => {
            let is_error = output.contains(r#""error""#);
            assert!(is_error || output.contains("chat.completion.chunk"));
            assert!(is_error || output.contains("[DONE]"));
        }
        AiProtocol::OpenAiResponses => {
            let is_error =
                output.contains("event: error") || output.contains("event: response.failed");
            assert!(is_error || output.contains("event: response.created"));
            assert!(is_error || output.contains("event: response.completed"));
        }
        AiProtocol::GeminiNative => {
            assert!(output.contains("candidates") || output.contains(r#""error""#));
        }
    }
}

fn fixture_path(relative_path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/coding/proxy_gateway/transformer/fixtures/reference")
        .join(relative_path)
}

fn read_fixture_json(relative_path: &str) -> Value {
    let text = fs::read_to_string(fixture_path(relative_path))
        .unwrap_or_else(|error| panic!("read fixture {relative_path}: {error}"));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parse fixture {relative_path}: {error}"))
}

fn read_reference_stream_fixture(relative_path: &str) -> String {
    let text = fs::read_to_string(fixture_path(relative_path))
        .unwrap_or_else(|error| panic!("read stream fixture {relative_path}: {error}"));
    stream_fixture_jsonl_to_sse(relative_path, &text)
}

fn live_provider_stream_fixture_path(relative_path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/coding/proxy_gateway/transformer/fixtures/live_provider")
        .join(relative_path)
}

fn read_live_provider_stream_fixture(relative_path: &str) -> String {
    let text = fs::read_to_string(live_provider_stream_fixture_path(relative_path))
        .unwrap_or_else(|error| panic!("read live stream fixture {relative_path}: {error}"));
    stream_fixture_jsonl_to_sse(relative_path, &text)
}

fn stream_fixture_jsonl_to_sse(relative_path: &str, text: &str) -> String {
    let mut sse = String::new();
    for (line_index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!("parse stream fixture {relative_path}:{line_index}: {error}")
        });
        if let Some(event_type) = value.get("Type").and_then(Value::as_str) {
            if !event_type.is_empty() {
                sse.push_str("event: ");
                sse.push_str(event_type);
                sse.push('\n');
            }
        }
        let data = value
            .get("Data")
            .and_then(Value::as_str)
            .unwrap_or_default();
        sse.push_str("data: ");
        sse.push_str(data);
        sse.push_str("\n\n");
    }
    sse
}

fn reference_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/coding/proxy_gateway/transformer/fixtures/reference")
}

fn collect_fixture_entries(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let mut entries = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read fixture dir {}: {error}", dir.display()))
        .map(|entry| entry.expect("fixture dir entry"))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_fixture_entries(&path, root, out);
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("fixture path should be under root")
                .to_string_lossy()
                .replace('\\', "/");
            out.push(relative);
        }
    }
}

fn reference_fixture_entries() -> Vec<String> {
    let root = reference_fixture_root();
    let mut entries = Vec::new();
    collect_fixture_entries(&root, &root, &mut entries);
    entries
}

fn protocol_for_fixture(path: &str) -> AiProtocol {
    if path.starts_with("anthropic/") {
        AiProtocol::AnthropicMessages
    } else if path.starts_with("openai_chat/") {
        AiProtocol::OpenAiChat
    } else if path.starts_with("openai_responses/") {
        AiProtocol::OpenAiResponses
    } else if path.starts_with("gemini/") {
        AiProtocol::GeminiNative
    } else {
        panic!("unknown reference fixture protocol for {path}");
    }
}

fn is_request_fixture(path: &str) -> bool {
    path.ends_with(".request.json")
}

fn is_response_fixture(path: &str) -> bool {
    path.ends_with(".response.json") && !path.ends_with(".stream.response.json")
}

fn is_stream_fixture(path: &str) -> bool {
    path.ends_with(".stream.jsonl") || path.ends_with(".response.stream.jsonl")
}

fn is_auxiliary_fixture(path: &str) -> bool {
    path.ends_with(".aggregator.json")
        || path.ends_with(".stream.response.json")
        || path == "gemini/gemini-thought.jsonl"
}

fn is_out_of_scope_fixture(path: &str) -> bool {
    matches!(
        path,
        "openai_responses/compact.response.json" | "openai_responses/image-generation.request.json"
    )
}

fn is_classified_reference_fixture(path: &str) -> bool {
    is_request_fixture(path)
        || is_response_fixture(path)
        || is_stream_fixture(path)
        || is_auxiliary_fixture(path)
        || is_out_of_scope_fixture(path)
}

fn live_provider_fixture_path(protocol_dir: &str, file_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/coding/proxy_gateway/transformer/fixtures/live_provider")
        .join(protocol_dir)
        .join(file_name)
}

fn read_live_provider_fixture_json(protocol_dir: &str, file_name: &str) -> Value {
    let path = live_provider_fixture_path(protocol_dir, file_name);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read live provider fixture {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parse live provider fixture {}: {error}", path.display()))
}

fn assert_live_provider_response_semantics(
    source: AiProtocol,
    target: AiProtocol,
    converted: &Value,
) {
    match (source, target) {
        (AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages) => {
            assert_eq!(
                anthropic_response_text(converted),
                Some("ai-toolbox protocol check")
            );
            assert!(anthropic_response_has_thinking(converted));
            assert_eq!(converted["stop_reason"], "end_turn");
        }
        (AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses) => {
            assert_eq!(converted["status"], "completed");
            assert_eq!(converted["output"][0]["summary"][0]["type"], "summary_text");
            assert_eq!(
                converted["output"][1]["content"][0]["text"],
                "ai-toolbox protocol check"
            );
        }
        (AiProtocol::OpenAiChat, AiProtocol::GeminiNative) => {
            assert_eq!(
                gemini_response_text(converted),
                Some("ai-toolbox protocol check")
            );
            assert_eq!(converted["candidates"][0]["finishReason"], "STOP");
        }
        (AiProtocol::OpenAiResponses, AiProtocol::AnthropicMessages) => {
            assert_eq!(converted["content"][0]["type"], "thinking");
            assert_eq!(converted["stop_reason"], "max_tokens");
        }
        (AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat) => {
            assert!(converted["choices"][0]["message"]["reasoning_content"]
                .as_str()
                .unwrap_or_default()
                .contains("ai-toolbox protocol check"));
            assert_eq!(converted["choices"][0]["finish_reason"], "length");
        }
        (AiProtocol::OpenAiResponses, AiProtocol::GeminiNative) => {
            assert!(gemini_response_any_text(converted)
                .unwrap_or_default()
                .contains("ai-toolbox protocol check"));
            assert_eq!(converted["candidates"][0]["finishReason"], "MAX_TOKENS");
        }
        (AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat) => {
            assert_eq!(
                openai_chat_response_text(converted),
                Some("ai-toolbox protocol check")
            );
            assert_eq!(converted["choices"][0]["finish_reason"], "stop");
        }
        (AiProtocol::AnthropicMessages, AiProtocol::OpenAiResponses) => {
            assert_eq!(converted["status"], "completed");
            assert_eq!(
                responses_output_text(converted),
                Some("ai-toolbox protocol check")
            );
        }
        (AiProtocol::AnthropicMessages, AiProtocol::GeminiNative) => {
            assert_eq!(
                gemini_response_text(converted),
                Some("ai-toolbox protocol check")
            );
            assert_eq!(converted["candidates"][0]["finishReason"], "STOP");
        }
        (AiProtocol::GeminiNative, AiProtocol::AnthropicMessages) => {
            assert_eq!(anthropic_response_text(converted), Some("ai-toolbox"));
            assert_eq!(converted["stop_reason"], "max_tokens");
        }
        (AiProtocol::GeminiNative, AiProtocol::OpenAiChat) => {
            assert_eq!(openai_chat_response_text(converted), Some("ai-toolbox"));
            assert_eq!(converted["choices"][0]["finish_reason"], "length");
        }
        (AiProtocol::GeminiNative, AiProtocol::OpenAiResponses) => {
            assert_eq!(converted["status"], "incomplete");
            assert_eq!(responses_output_text(converted), Some("ai-toolbox"));
        }
        _ => {}
    }
}

fn anthropic_response_text(value: &Value) -> Option<&str> {
    value
        .get("content")
        .and_then(Value::as_array)?
        .iter()
        .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|block| block.get("text").and_then(Value::as_str))
}

fn openai_chat_response_text(value: &Value) -> Option<&str> {
    let content = &value["choices"][0]["message"]["content"];
    if let Some(text) = content.as_str() {
        return Some(text);
    }
    content
        .as_array()?
        .iter()
        .find(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|part| part.get("text").and_then(Value::as_str))
}

fn responses_output_text(value: &Value) -> Option<&str> {
    value
        .get("output")
        .and_then(Value::as_array)?
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .flat_map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .find_map(|content| content.get("text").and_then(Value::as_str))
}

fn anthropic_response_has_thinking(value: &Value) -> bool {
    value
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .any(|block| block.get("type").and_then(Value::as_str) == Some("thinking"))
        })
        .unwrap_or(false)
}

fn gemini_response_text(value: &Value) -> Option<&str> {
    value["candidates"][0]["content"]["parts"]
        .as_array()?
        .iter()
        .find(|part| part.get("thought").and_then(Value::as_bool) != Some(true))
        .and_then(|part| part.get("text").and_then(Value::as_str))
}

fn gemini_response_any_text(value: &Value) -> Option<&str> {
    value["candidates"][0]["content"]["parts"]
        .as_array()?
        .iter()
        .find_map(|part| part.get("text").and_then(Value::as_str))
}

#[test]
fn request_matrix_gemini_to_responses_uses_new_kernel() {
    let converted = convert_request_value(
            ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiResponses),
            json!({
                "systemInstruction": {"parts": [{"text": "system"}]},
                "contents": [
                    {"role": "user", "parts": [{"text": "hi"}]},
                    {"role": "model", "parts": [{"functionCall": {"id": "call_1", "name": "read_file", "args": {"path": "a.txt"}}}]},
                    {"role": "user", "parts": [{"functionResponse": {"id": "call_1", "name": "read_file", "response": {"content": "ok"}}}]}
                ],
                "tools": [{"functionDeclarations": [{"name": "read_file", "parameters": {"type": "object"}}]}]
            }),
        )
        .unwrap();

    assert_eq!(converted["instructions"], "system");
    assert_eq!(converted["input"][0]["role"], "user");
    assert_eq!(converted["input"][1]["type"], "function_call");
    assert_eq!(converted["input"][2]["type"], "function_call_output");
    assert_eq!(converted["tools"][0]["name"], "read_file");
}

#[test]
fn request_conversion_covers_all_non_identity_protocol_routes() {
    for source in PROTOCOLS {
        for target in PROTOCOLS {
            if source == target {
                continue;
            }
            let converted = convert_request_value(
                ConversionRoute::new(source, target),
                request_fixture(source),
            )
            .unwrap_or_else(|error| {
                panic!(
                    "request route {} -> {} failed: {error}",
                    source.as_str(),
                    target.as_str()
                )
            });
            assert_request_shape(target, &converted);
        }
    }
}

#[test]
fn response_conversion_covers_all_non_identity_protocol_routes() {
    for source in PROTOCOLS {
        for target in PROTOCOLS {
            if source == target {
                continue;
            }
            let converted = convert_response_value(
                ConversionRoute::new(source, target),
                response_fixture(source),
            )
            .unwrap_or_else(|error| {
                panic!(
                    "response route {} -> {} failed: {error}",
                    source.as_str(),
                    target.as_str()
                )
            });
            assert_response_shape(target, &converted);
        }
    }
}

#[test]
fn sse_conversion_covers_all_non_identity_protocol_routes() {
    for source in PROTOCOLS {
        for target in PROTOCOLS {
            if source == target {
                continue;
            }
            let output =
                collect_stream(ConversionRoute::new(source, target), stream_fixture(source));
            assert_stream_shape(target, &output);
            assert_stream_usage_values(source, target, &output);
        }
    }
}

fn assert_stream_usage_values(source: AiProtocol, target: AiProtocol, output: &str) {
    // Gemini/Anthropic source fixtures now carry exact usage; lock the converted numbers.
    let expects_usage = matches!(
        source,
        AiProtocol::GeminiNative
            | AiProtocol::AnthropicMessages
            | AiProtocol::OpenAiChat
            | AiProtocol::OpenAiResponses
    );
    if !expects_usage {
        return;
    }
    match target {
        AiProtocol::OpenAiChat => {
            let values = sse_data_values(output);
            let usage = values
                .iter()
                .find_map(|value| value.get("usage"))
                .unwrap_or_else(|| {
                    panic!("Chat target from {source:?} missing usage; output={output}")
                });
            assert_eq!(
                usage.get("prompt_tokens").and_then(Value::as_u64),
                Some(11),
                "Chat usage prompt_tokens from {source:?}: {usage}"
            );
            assert_eq!(
                usage.get("completion_tokens").and_then(Value::as_u64),
                Some(3),
                "Chat usage completion_tokens from {source:?}: {usage}"
            );
        }
        AiProtocol::OpenAiResponses => {
            assert!(
                output.contains(r#""input_tokens":11"#) || output.contains(r#""input_tokens": 11"#),
                "Responses target from {source:?} missing input_tokens=11; output={output}"
            );
            assert!(
                output.contains(r#""output_tokens":3"#) || output.contains(r#""output_tokens": 3"#),
                "Responses target from {source:?} missing output_tokens=3; output={output}"
            );
        }
        AiProtocol::AnthropicMessages => {
            assert!(
                output.contains(r#""input_tokens":11"#) || output.contains(r#""input_tokens": 11"#),
                "Anthropic target from {source:?} missing input_tokens=11; output={output}"
            );
            assert!(
                output.contains(r#""output_tokens":3"#) || output.contains(r#""output_tokens": 3"#),
                "Anthropic target from {source:?} missing output_tokens=3; output={output}"
            );
        }
        AiProtocol::GeminiNative => {
            assert!(
                output.contains(r#""promptTokenCount":11"#)
                    || output.contains(r#""promptTokenCount": 11"#),
                "Gemini target from {source:?} missing promptTokenCount=11; output={output}"
            );
            assert!(
                output.contains(r#""candidatesTokenCount":3"#)
                    || output.contains(r#""candidatesTokenCount": 3"#),
                "Gemini target from {source:?} missing candidatesTokenCount=3; output={output}"
            );
        }
    }
}

#[test]
fn reference_request_fixtures_convert_to_all_targets() {
    let cases = [
        (
            AiProtocol::AnthropicMessages,
            "anthropic/anthropic-simple-inbound.request.json",
        ),
        (
            AiProtocol::AnthropicMessages,
            "anthropic/anthropic-tool.request.json",
        ),
        (
            AiProtocol::AnthropicMessages,
            "anthropic/anthropic-thinking.request.json",
        ),
        (
            AiProtocol::OpenAiChat,
            "openai_chat/openai-tool.request.json",
        ),
        (
            AiProtocol::OpenAiResponses,
            "openai_responses/tool.request.json",
        ),
        (
            AiProtocol::OpenAiResponses,
            "openai_responses/reasoning.request.json",
        ),
        (
            AiProtocol::GeminiNative,
            "gemini/gemini-simple.request.json",
        ),
        (AiProtocol::GeminiNative, "gemini/gemini-tools.request.json"),
        (
            AiProtocol::GeminiNative,
            "gemini/gemini-thinking.request.json",
        ),
    ];

    for (source, path) in cases {
        let value = read_fixture_json(path);
        for target in PROTOCOLS {
            if source == target {
                continue;
            }
            let converted =
                convert_request_value(ConversionRoute::new(source, target), value.clone())
                    .unwrap_or_else(|error| {
                        panic!(
                            "reference request fixture {path} route {} -> {} failed: {error}",
                            source.as_str(),
                            target.as_str()
                        )
                    });
            assert_request_shape(target, &converted);
        }
    }
}

#[test]
fn reference_response_fixtures_convert_to_all_targets() {
    let cases = [
        (
            AiProtocol::AnthropicMessages,
            "anthropic/anthropic-stop.response.json",
        ),
        (
            AiProtocol::AnthropicMessages,
            "anthropic/anthropic-tool.response.json",
        ),
        (
            AiProtocol::AnthropicMessages,
            "anthropic/anthropic-think.response.json",
        ),
        (
            AiProtocol::OpenAiChat,
            "openai_chat/openai-stop.response.json",
        ),
        (
            AiProtocol::OpenAiChat,
            "openai_chat/openai-tool.response.json",
        ),
        (
            AiProtocol::OpenAiChat,
            "openai_chat/deepseek-reasoning.response.json",
        ),
        (
            AiProtocol::OpenAiResponses,
            "openai_responses/simple.response.json",
        ),
        (
            AiProtocol::OpenAiResponses,
            "openai_responses/tool.response.json",
        ),
        (
            AiProtocol::OpenAiResponses,
            "openai_responses/stop.response.json",
        ),
        (
            AiProtocol::GeminiNative,
            "gemini/gemini-simple.response.json",
        ),
        (
            AiProtocol::GeminiNative,
            "gemini/gemini-tools.response.json",
        ),
        (
            AiProtocol::GeminiNative,
            "gemini/gemini-thinking.response.json",
        ),
    ];

    for (source, path) in cases {
        let value = read_fixture_json(path);
        for target in PROTOCOLS {
            if source == target {
                continue;
            }
            let converted =
                convert_response_value(ConversionRoute::new(source, target), value.clone())
                    .unwrap_or_else(|error| {
                        panic!(
                            "reference response fixture {path} route {} -> {} failed: {error}",
                            source.as_str(),
                            target.as_str()
                        )
                    });
            assert_response_shape(target, &converted);
        }
    }
}

#[test]
fn reference_stream_fixtures_convert_to_all_targets() {
    let cases = [
        (
            AiProtocol::AnthropicMessages,
            "anthropic/anthropic-stop.stream.jsonl",
        ),
        (
            AiProtocol::AnthropicMessages,
            "anthropic/anthropic-tool.stream.jsonl",
        ),
        (
            AiProtocol::AnthropicMessages,
            "anthropic/anthropic-think.stream.jsonl",
        ),
        (
            AiProtocol::OpenAiChat,
            "openai_chat/openai-stop.stream.jsonl",
        ),
        (
            AiProtocol::OpenAiChat,
            "openai_chat/openai-tool.stream.jsonl",
        ),
        (
            AiProtocol::OpenAiChat,
            "openai_chat/deepseek-reasoninig.stream.jsonl",
        ),
        (
            AiProtocol::OpenAiResponses,
            "openai_responses/stop.response.stream.jsonl",
        ),
        (
            AiProtocol::OpenAiResponses,
            "openai_responses/tool-2.stream.jsonl",
        ),
        (
            AiProtocol::OpenAiResponses,
            "openai_responses/llm-tool-2.stream.jsonl",
        ),
        (AiProtocol::GeminiNative, "gemini/gemini-stop.stream.jsonl"),
        (AiProtocol::GeminiNative, "gemini/gemini-tool.stream.jsonl"),
        (AiProtocol::GeminiNative, "gemini/gemini-think.stream.jsonl"),
    ];

    for (source, path) in cases {
        let input = read_reference_stream_fixture(path);
        for target in PROTOCOLS {
            if source == target {
                continue;
            }
            let output = collect_stream(ConversionRoute::new(source, target), input.clone());
            assert_stream_basic_shape(target, &output);
        }
    }
}

#[test]
fn reference_all_copied_fixtures_are_classified() {
    let entries = reference_fixture_entries();
    assert_eq!(entries.len(), 118, "reference fixture corpus size changed");

    let unclassified = entries
        .iter()
        .filter(|path| !is_classified_reference_fixture(path))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        unclassified.is_empty(),
        "unclassified reference fixtures: {unclassified:#?}"
    );
}

#[test]
fn reference_all_supported_request_fixtures_convert_to_all_targets() {
    let entries = reference_fixture_entries()
        .into_iter()
        .filter(|path| is_request_fixture(path))
        .filter(|path| !is_auxiliary_fixture(path) && !is_out_of_scope_fixture(path))
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 35, "supported request fixture count changed");

    for path in entries {
        let source = protocol_for_fixture(&path);
        let value = read_fixture_json(&path);
        for target in PROTOCOLS {
            if source == target {
                continue;
            }
            let converted =
                convert_request_value(ConversionRoute::new(source, target), value.clone())
                    .unwrap_or_else(|error| {
                        panic!(
                            "reference request fixture {path} route {} -> {} failed: {error}",
                            source.as_str(),
                            target.as_str()
                        )
                    });
            assert_request_shape(target, &converted);
        }
    }
}

#[test]
fn reference_all_supported_response_fixtures_convert_to_all_targets() {
    let entries = reference_fixture_entries()
        .into_iter()
        .filter(|path| is_response_fixture(path))
        .filter(|path| !is_auxiliary_fixture(path) && !is_out_of_scope_fixture(path))
        .collect::<Vec<_>>();
    assert_eq!(
        entries.len(),
        34,
        "supported response fixture count changed"
    );

    for path in entries {
        let source = protocol_for_fixture(&path);
        let value = read_fixture_json(&path);
        for target in PROTOCOLS {
            if source == target {
                continue;
            }
            let converted =
                convert_response_value(ConversionRoute::new(source, target), value.clone())
                    .unwrap_or_else(|error| {
                        panic!(
                            "reference response fixture {path} route {} -> {} failed: {error}",
                            source.as_str(),
                            target.as_str()
                        )
                    });
            assert_response_shape(target, &converted);
        }
    }
}

#[test]
fn reference_all_supported_stream_fixtures_convert_to_all_targets() {
    let entries = reference_fixture_entries()
        .into_iter()
        .filter(|path| is_stream_fixture(path))
        .filter(|path| !is_auxiliary_fixture(path) && !is_out_of_scope_fixture(path))
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 43, "supported stream fixture count changed");

    for path in entries {
        let source = protocol_for_fixture(&path);
        let input = read_reference_stream_fixture(&path);
        for target in PROTOCOLS {
            if source == target {
                continue;
            }
            let output = collect_stream(ConversionRoute::new(source, target), input.clone());
            assert_stream_basic_shape(target, &output);
        }
    }
}

#[test]
fn reference_anthropic_request_semantics_convert_exactly() {
    let source = json!({
        "model": "claude-3-sonnet-20240229",
        "max_tokens": 1024,
        "system": "You are a helpful assistant.",
        "temperature": 0.7,
        "top_p": 0.9,
        "stop_sequences": ["Human:", "Assistant:"],
        "tool_choice": {"type": "any"},
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "What's in this image?"},
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/jpeg",
                        "data": "/9j/4AAQSkZJRg..."
                    }
                }
            ]
        }],
        "tools": [{
            "name": "get_weather",
            "description": "Get weather information",
            "input_schema": {"type": "object", "properties": {"location": {"type": "string"}}}
        }]
    });

    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        source.clone(),
    )
    .unwrap();
    assert_eq!(chat["messages"][0]["role"], "system");
    assert_eq!(
        chat["messages"][0]["content"],
        "You are a helpful assistant."
    );
    assert_eq!(
        chat["messages"][1]["content"][0]["text"],
        "What's in this image?"
    );
    assert_eq!(
        chat["messages"][1]["content"][1]["image_url"]["url"],
        "data:image/jpeg;base64,/9j/4AAQSkZJRg..."
    );
    assert_eq!(chat["stop"], json!(["Human:", "Assistant:"]));
    assert_eq!(chat["tool_choice"], "required");
    assert_eq!(chat["tools"][0]["function"]["name"], "get_weather");

    let responses = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiResponses),
        source.clone(),
    )
    .unwrap();
    assert_eq!(responses["instructions"], "You are a helpful assistant.");
    assert_eq!(responses["input"][0]["content"][0]["type"], "input_text");
    assert_eq!(responses["input"][0]["content"][1]["type"], "input_image");
    assert_eq!(responses["stop"], json!(["Human:", "Assistant:"]));
    assert_eq!(responses["tool_choice"], "required");
    assert_eq!(responses["tools"][0]["name"], "get_weather");

    let gemini = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::GeminiNative),
        source,
    )
    .unwrap();
    assert_eq!(
        gemini["systemInstruction"]["parts"][0]["text"],
        "You are a helpful assistant."
    );
    assert_eq!(
        gemini["contents"][0]["parts"][1]["inlineData"]["mimeType"],
        "image/jpeg"
    );
    assert_eq!(
        gemini["generationConfig"]["stopSequences"],
        json!(["Human:", "Assistant:"])
    );
    assert_eq!(gemini["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
}

#[test]
fn anthropic_probe_max_tokens_is_clamped_to_responses_floor() {
    // Claude Desktop probes model availability with `max_tokens=1`; the
    // Responses API rejects `max_output_tokens < 16`, so the Anthropic ->
    // Responses conversion clamps sub-floor budgets instead of failing the
    // probe (cc-switch 6e4b0e6e, #7103).
    for (budget, expected) in [(1_i64, 16_i64), (15, 16), (16, 16), (2048, 2048)] {
        let responses = convert_request_value(
            ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiResponses),
            json!({
                "model": "claude-haiku-4-5",
                "max_tokens": budget,
                "messages": [{"role": "user", "content": "ping"}]
            }),
        )
        .unwrap();
        assert_eq!(
            responses["max_output_tokens"],
            json!(expected),
            "anthropic max_tokens {budget} must convert to {expected}"
        );
    }
}

#[test]
fn anthropic_url_image_source_converts_both_directions() {
    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        json!({
            "model": "claude-3-sonnet-20240229",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image",
                    "source": {
                        "type": "url",
                        "url": "https://example.com/chart.png"
                    }
                }]
            }]
        }),
    )
    .unwrap();
    assert_eq!(
        chat["messages"][0]["content"][0]["image_url"]["url"],
        "https://example.com/chart.png"
    );

    let anthropic = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        json!({
            "model": "claude-3-sonnet-20240229",
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image_url",
                    "image_url": {"url": "https://example.com/chart.png"}
                }]
            }]
        }),
    )
    .unwrap();
    assert_eq!(
        anthropic["messages"][0]["content"][0]["source"]["type"],
        "url"
    );
    assert_eq!(
        anthropic["messages"][0]["content"][0]["source"]["url"],
        "https://example.com/chart.png"
    );
}

#[test]
fn anthropic_batch_tool_is_filtered_for_non_anthropic_targets() {
    let source = json!({
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 100,
        "messages": [{"role": "user", "content": "hi"}],
        "tools": [
            {
                "type": "BatchTool",
                "name": "BatchTool",
                "input_schema": {"type": "object"}
            },
            {
                "name": "read_file",
                "description": "Read a file",
                "input_schema": {"type": "object"}
            }
        ]
    });

    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        source.clone(),
    )
    .unwrap();
    assert_eq!(chat["tools"].as_array().unwrap().len(), 1);
    assert_eq!(chat["tools"][0]["function"]["name"], "read_file");

    let responses = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiResponses),
        source.clone(),
    )
    .unwrap();
    assert_eq!(responses["tools"].as_array().unwrap().len(), 1);
    assert_eq!(responses["tools"][0]["name"], "read_file");

    let gemini = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::GeminiNative),
        source,
    )
    .unwrap();
    assert_eq!(
        gemini["tools"][0]["functionDeclarations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        gemini["tools"][0]["functionDeclarations"][0]["name"],
        "read_file"
    );
}

#[test]
fn anthropic_to_chat_uses_max_completion_tokens_for_reasoning_models() {
    for model in ["o3", "gpt-5", "gpt-5.1-codex-mini"] {
        let converted = convert_request_value(
            ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
            json!({
                "model": model,
                "max_tokens": 100,
                "messages": [{"role": "user", "content": "hi"}]
            }),
        )
        .unwrap();
        assert_eq!(converted["max_completion_tokens"], 100);
        assert!(converted.get("max_tokens").is_none());
    }

    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        json!({
            "model": "gpt-4o",
            "max_tokens": 100,
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .unwrap();
    assert_eq!(converted["max_tokens"], 100);
    assert!(converted.get("max_completion_tokens").is_none());
}

#[test]
fn anthropic_thinking_maps_to_openai_reasoning_effort() {
    let low = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        json!({
            "model": "o3",
            "max_tokens": 100,
            "thinking": {"type": "enabled", "budget_tokens": 2000},
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .unwrap();
    assert_eq!(low["reasoning_effort"], "low");
    assert_eq!(low["max_completion_tokens"], 100);

    let xhigh = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiResponses),
        json!({
            "model": "gpt-5.1-codex-mini",
            "max_tokens": 100,
            "output_config": {"effort": "max"},
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .unwrap();
    assert_eq!(xhigh["reasoning"]["effort"], "xhigh");
}

/// Converts a Codex Responses request carrying `effort` to Anthropic Messages.
fn responses_effort_to_anthropic(model: &str, effort: &str) -> Value {
    convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::AnthropicMessages),
        json!({
            "model": model,
            "max_output_tokens": 20000,
            "reasoning": {"effort": effort},
            "input": "think"
        }),
    )
    .unwrap()
}

#[test]
fn openai_reasoning_effort_maps_to_anthropic_budget_tokens_on_legacy_models() {
    let medium = responses_effort_to_anthropic("claude-sonnet-4-5-20250929", "medium");
    assert_eq!(medium["thinking"]["type"], "enabled");
    assert_eq!(medium["thinking"]["budget_tokens"], 10240);
    assert!(medium.get("output_config").is_none());

    let xhigh = responses_effort_to_anthropic("claude-sonnet-4-5-20250929", "xhigh");
    assert_eq!(xhigh["thinking"]["budget_tokens"], 20000);
}

#[test]
fn openai_reasoning_effort_maps_to_adaptive_thinking_on_claude_4_6_and_later() {
    // Issue #437: `enabled` + `budget_tokens` is a 400 on these models.
    let medium = responses_effort_to_anthropic("claude-opus-5-5", "medium");
    assert_eq!(medium["thinking"]["type"], "adaptive");
    assert_eq!(medium["thinking"]["display"], "summarized");
    assert!(medium["thinking"].get("budget_tokens").is_none());
    assert_eq!(medium["output_config"]["effort"], "medium");

    // 4.6 accepted adaptive but predates `xhigh`.
    let xhigh = responses_effort_to_anthropic("claude-sonnet-4-6", "xhigh");
    assert_eq!(xhigh["thinking"]["type"], "adaptive");
    assert_eq!(xhigh["output_config"]["effort"], "high");
}

#[test]
fn disabling_thinking_on_claude_5_omits_the_field_instead_of_sending_disabled() {
    let none = responses_effort_to_anthropic("claude-opus-5-5", "none");
    assert!(none.get("thinking").is_none());
    assert_eq!(none["output_config"]["effort"], "low");
}

#[test]
fn anthropic_system_strips_leading_billing_header_for_converted_targets() {
    let source = json!({
        "model": "claude-sonnet-4-6",
        "max_tokens": 100,
        "system": [
            {
                "type": "text",
                "text": "x-anthropic-billing-header: cc_version=2.1.119; cch=rotating;\n\nStable prompt part 1"
            },
            {
                "type": "text",
                "text": "Stable prompt part 2"
            }
        ],
        "messages": [{"role": "user", "content": "hi"}]
    });

    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        source.clone(),
    )
    .unwrap();
    assert_eq!(chat["messages"][0]["role"], "system");
    assert_eq!(
        chat["messages"][0]["content"],
        "Stable prompt part 1\n\nStable prompt part 2"
    );
    assert!(!chat.to_string().contains("x-anthropic-billing-header"));

    let responses = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiResponses),
        source.clone(),
    )
    .unwrap();
    assert_eq!(
        responses["instructions"],
        "Stable prompt part 1\n\nStable prompt part 2"
    );
    assert!(!responses.to_string().contains("x-anthropic-billing-header"));

    let gemini = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::GeminiNative),
        source,
    )
    .unwrap();
    assert_eq!(
        gemini["systemInstruction"]["parts"][0]["text"],
        "Stable prompt part 1\n\nStable prompt part 2"
    );
    assert!(!gemini.to_string().contains("x-anthropic-billing-header"));
}

/// Text of a converted Chat message, which is either a plain string or the
/// text-parts array shape Anthropic sources normalize to.
fn chat_message_text(message: &Value) -> String {
    match &message["content"] {
        Value::String(text) => text.clone(),
        other => other
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
    }
}

#[test]
fn openai_chat_target_merges_leading_instruction_run_to_head() {
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        json!({
            "model": "gpt-5.1-codex-mini",
            "instructions": "Top instruction",
            "input": [
                {
                    "type": "message",
                    "role": "developer",
                    "content": [{"type": "input_text", "text": "Leading developer instruction"}]
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "hi"}]
                }
            ]
        }),
    )
    .unwrap();

    assert_eq!(converted["messages"][0]["role"], "system");
    assert_eq!(
        converted["messages"][0]["content"],
        "Top instruction\n\nLeading developer instruction"
    );
    assert_eq!(converted["messages"][1]["role"], "user");
    assert_eq!(converted["messages"].as_array().unwrap().len(), 2);
}

#[test]
fn openai_chat_target_merges_late_codex_instructions_for_responses_source() {
    // Codex keeps its identity/instruction blocks in `developer` items, which are
    // stable across turns, so the Responses source must keep the historical
    // merge-everything-to-head behavior (same as cc-switch's codex chat transform,
    // which merges mid-stream `developer` instructions into the head as well).
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        json!({
            "model": "gpt-5.1-codex-mini",
            "instructions": "Top instruction",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "hi"}]
                },
                {
                    "type": "message",
                    "role": "developer",
                    "content": [{"type": "input_text", "text": "Collaboration Mode: Default"}]
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "more"}]
                }
            ]
        }),
    )
    .unwrap();

    let messages = converted["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(
        chat_message_text(&messages[0]),
        "Top instruction\n\nCollaboration Mode: Default"
    );
    assert!(messages
        .iter()
        .all(|message| message["role"].as_str() != Some("developer")));
}

#[test]
fn anthropic_to_openai_chat_keeps_claude_code_token_reminder_out_of_the_head() {
    // Claude Code appends a per-turn `role:"system"` reminder such as
    // `<total_tokens>N tokens left</total_tokens>` at the tail of the conversation.
    // Hoisting it into the head system message rewrote the prompt prefix every
    // turn and dropped the upstream cache hit rate to ~7% (#356).
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        json!({
            "model": "deepseek-v4.1-flash",
            "max_tokens": 1024,
            "system": "You are Claude Code.",
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there!"},
                {
                    "role": "system",
                    "content": "<total_tokens>14963538 tokens left</total_tokens>"
                },
                {"role": "user", "content": "Continue"}
            ]
        }),
    )
    .unwrap();

    let messages = converted["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "You are Claude Code.");
    // The trailing reminder keeps its index and bytes; only the role changes.
    assert_eq!(messages[3]["role"], "user");
    assert_eq!(
        chat_message_text(&messages[3]),
        "<total_tokens>14963538 tokens left</total_tokens>"
    );
    assert_eq!(messages[4]["role"], "user");
    assert_eq!(chat_message_text(&messages[4]), "Continue");
}

#[test]
fn anthropic_to_openai_chat_head_stays_byte_stable_when_a_turn_appends_a_reminder() {
    let body_for_turn = |reminder: &str| {
        let mut messages = vec![
            json!({"role": "user", "content": "Hello"}),
            json!({"role": "assistant", "content": "Hi there!"}),
        ];
        if !reminder.is_empty() {
            messages.push(json!({"role": "system", "content": reminder}));
        }
        messages.push(json!({"role": "user", "content": "Continue"}));
        json!({
            "model": "deepseek-v4.1-flash",
            "max_tokens": 1024,
            "system": "You are Claude Code.",
            "messages": messages
        })
    };

    let convert = |reminder: &str| {
        convert_request_value(
            ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
            body_for_turn(reminder),
        )
        .unwrap()
    };

    let first = convert("");
    let second = convert("<total_tokens>14900000 tokens left</total_tokens>");

    // Whatever the reminder says, the cached prefix must be unchanged: the head
    // system message, the user turn and the assistant turn are byte-identical, and
    // the reminder only adds a message after them.
    assert_eq!(first["messages"][0], second["messages"][0]);
    assert_eq!(first["messages"][1], second["messages"][1]);
    assert_eq!(first["messages"][2], second["messages"][2]);
    assert_eq!(chat_message_text(&first["messages"][2]), "Hi there!");
    assert_eq!(first["messages"].as_array().unwrap().len(), 4);
    assert_eq!(second["messages"].as_array().unwrap().len(), 5);
}

#[test]
fn openai_responses_target_keeps_late_system_message_out_of_instructions() {
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiResponses),
        json!({
            "model": "gpt-5.1-codex-mini",
            "max_tokens": 1024,
            "system": "You are Claude Code.",
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there!"},
                {
                    "role": "system",
                    "content": "<total_tokens>14963538 tokens left</total_tokens>"
                },
                {"role": "user", "content": "Continue"}
            ]
        }),
    )
    .unwrap();

    // Only the leading instruction run may reach the hoisted `instructions` field.
    assert_eq!(converted["instructions"], "You are Claude Code.");

    // The reminder stays in `input` at its own position, downgraded to `user`.
    let input = converted["input"].as_array().unwrap();
    let late = input
        .iter()
        .find(|item| {
            item.pointer("/content/0/text")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("<total_tokens>"))
        })
        .expect("late system message should keep an input item");
    assert_eq!(late["role"], "user");
    assert_eq!(
        late.pointer("/content/0/text").and_then(Value::as_str),
        Some("<total_tokens>14963538 tokens left</total_tokens>")
    );
}

#[test]
fn anthropic_target_merges_every_system_message_into_top_level_system() {
    // Anthropic Messages is never a conversion source for an Anthropic target
    // (`conversion_route()` only exists when source != target), so this writer only
    // serves Chat/Responses/Gemini sources. Their instruction content is stable
    // across turns, so the historical merge stays - same as cc-switch's
    // Codex -> Anthropic transform, which hoists `developer` items out of `input`
    // into the top-level `system` field.
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        json!({
            "model": "claude-sonnet-4-5",
            "max_tokens": 1024,
            "messages": [
                {"role": "system", "content": "You are Claude Code."},
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there!"},
                {
                    "role": "system",
                    "content": "<total_tokens>14963538 tokens left</total_tokens>"
                },
                {"role": "user", "content": "Continue"}
            ]
        }),
    )
    .unwrap();

    assert_eq!(
        converted["system"],
        "You are Claude Code.\n\n<total_tokens>14963538 tokens left</total_tokens>"
    );

    let messages = converted["messages"].as_array().unwrap();
    assert!(messages
        .iter()
        .all(|message| message["role"].as_str() != Some("system")));
}

#[test]
fn openai_responses_instruction_parts_convert_to_system_text() {
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        json!({
            "model": "gpt-5.1-codex-mini",
            "instructions": [
                {"type": "text", "text": "Instruction A"},
                {"type": "text", "text": "Instruction B"}
            ],
            "input": "hello"
        }),
    )
    .unwrap();

    assert_eq!(converted["messages"][0]["role"], "system");
    assert_eq!(
        converted["messages"][0]["content"],
        "Instruction A\n\nInstruction B"
    );
    assert_eq!(converted["messages"][1]["role"], "user");
}

#[test]
fn anthropic_string_tool_choice_any_maps_to_openai_required() {
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        json!({
            "model": "gpt-5",
            "max_tokens": 100,
            "tool_choice": "any",
            "tools": [{
                "name": "read_file",
                "description": "Read a file",
                "input_schema": {"type": "object"}
            }],
            "messages": [{"role": "user", "content": "use tool"}]
        }),
    )
    .unwrap();

    assert_eq!(converted["tool_choice"], "required");
}

#[test]
fn allowed_tools_tool_choice_keeps_subset_across_openai_routes() {
    // The model may only call the caller's permitted subset; dropping it (or
    // emitting an empty function name) would silently lift the restriction.
    let responses = json!({
        "model": "gpt-5",
        "input": "hi",
        "tool_choice": {
            "type": "allowed_tools",
            "mode": "required",
            "tools": [
                {"type": "function", "name": "tool_a"},
                {"type": "mcp", "server_label": "local"}
            ]
        },
        "tools": [{
            "type": "function",
            "name": "tool_a",
            "parameters": {"type": "object"}
        }]
    });

    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        responses.clone(),
    )
    .unwrap();
    assert_eq!(
        chat["tool_choice"],
        json!({
            "type": "allowed_tools",
            "allowed_tools": {
                "mode": "required",
                "tools": [
                    {"type": "function", "function": {"name": "tool_a"}},
                    {"type": "mcp", "server_label": "local"}
                ]
            }
        })
    );

    let responses_again = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        chat,
    )
    .unwrap();
    assert_eq!(
        responses_again["tool_choice"],
        json!({
            "type": "allowed_tools",
            "mode": "required",
            "tools": [
                {"type": "function", "name": "tool_a"},
                {"type": "mcp", "server_label": "local"}
            ]
        })
    );

    // Anthropic/Gemini cannot express the subset, so only the mode survives.
    let anthropic = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::AnthropicMessages),
        responses.clone(),
    )
    .unwrap();
    assert_eq!(anthropic["tool_choice"], json!({"type": "any"}));

    let gemini = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::GeminiNative),
        responses,
    )
    .unwrap();
    assert_eq!(gemini["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
}

#[test]
fn anthropic_tools_without_strict_omit_strict_for_openai_targets() {
    let source = json!({
        "model": "gpt-5",
        "messages": [{"role": "user", "content": "use tool"}],
        "tools": [{
            "name": "read_file",
            "description": "Read a file",
            "input_schema": {
                "type": "object",
                "properties": {"path": {"type": "string"}}
            }
        }]
    });

    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        source.clone(),
    )
    .unwrap();
    assert!(chat["tools"][0]["function"].get("strict").is_none());

    let responses = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiResponses),
        source,
    )
    .unwrap();
    assert!(responses["tools"][0].get("strict").is_none());
}

#[test]
fn responses_error_to_anthropic_preserves_detail_message() {
    let converted = convert_error_response_body(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::AnthropicMessages),
        br#"{"detail":"strict must be a boolean"}"#,
    );
    let value = serde_json::from_slice::<Value>(&converted).unwrap();

    assert_eq!(
        value.pointer("/error/message").and_then(Value::as_str),
        Some("strict must be a boolean")
    );
}

#[test]
fn openai_plural_errors_envelope_preserves_error_fields() {
    let converted = convert_error_response_body(
            ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
            br#"{"errors":{"message":"bad request","type":"invalid_request_error","param":"input","code":"bad_input"}}"#,
        );
    let value = serde_json::from_slice::<Value>(&converted).unwrap();

    assert_eq!(value["error"]["message"], "bad request");
    assert_eq!(value["error"]["type"], "invalid_request_error");
    assert_eq!(value["error"]["param"], "input");
    assert_eq!(value["error"]["code"], "bad_input");
}

#[test]
fn openai_error_to_gemini_uses_gemini_error_shape() {
    let converted = convert_error_response_body(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
            br#"{"error":{"message":"rate limited","type":"rate_limit_error","code":"rate_limit_exceeded"}}"#,
        );
    let value = serde_json::from_slice::<Value>(&converted).unwrap();

    assert_eq!(value["error"]["message"], "rate limited");
    assert_eq!(value["error"]["code"], 429);
    assert_eq!(value["error"]["status"], "RESOURCE_EXHAUSTED");
    assert!(value["error"].get("type").is_none());
}

#[test]
fn gemini_error_to_openai_chat_uses_openai_error_shape() {
    let converted = convert_error_response_body(
        ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
        br#"{"error":{"code":403,"message":"blocked","status":"PERMISSION_DENIED"}}"#,
    );
    let value = serde_json::from_slice::<Value>(&converted).unwrap();

    assert_eq!(value["error"]["message"], "blocked");
    assert_eq!(value["error"]["type"], "PERMISSION_DENIED");
    assert_eq!(value["error"]["code"], 403);
    assert_eq!(value["error"]["param"], Value::Null);
}

#[test]
fn openai_response_format_maps_to_responses_and_gemini() {
    let source = json!({
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "return json"}],
        "frequency_penalty": 0.4,
        "presence_penalty": 0.5,
        "seed": 42,
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "answer",
                "schema": {
                    "type": "object",
                    "properties": {"ok": {"type": "boolean"}},
                    "required": ["ok"]
                },
                "strict": true
            }
        }
    });

    let responses = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        source.clone(),
    )
    .unwrap();
    assert_eq!(responses["text"]["format"]["type"], "json_schema");
    assert_eq!(responses["text"]["format"]["name"], "answer");
    assert_eq!(
        responses["text"]["format"]["schema"]["properties"]["ok"]["type"],
        "boolean"
    );
    assert_eq!(responses["text"]["format"]["strict"], true);

    let gemini = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        source,
    )
    .unwrap();
    assert_eq!(
        gemini["generationConfig"]["responseMimeType"],
        "application/json"
    );
    assert_eq!(
        gemini["generationConfig"]["responseJsonSchema"]["properties"]["ok"]["type"],
        "boolean"
    );
    assert!(gemini["generationConfig"].get("responseSchema").is_none());
    assert_eq!(gemini["generationConfig"]["frequencyPenalty"], 0.4);
    assert_eq!(gemini["generationConfig"]["presencePenalty"], 0.5);
    assert_eq!(gemini["generationConfig"]["seed"], 42);
}

#[test]
fn openai_request_pass_through_fields_convert_to_responses() {
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        json!({
            "model": "gpt-5.1-codex-mini",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read a file",
                    "parameters": {"type": "object"}
                }
            }],
            "parallel_tool_calls": false,
            "reasoning_effort": "high",
            "frequency_penalty": 0.1,
            "presence_penalty": 0.2,
            "service_tier": "default",
            "top_logprobs": 3,
            "user": "user-1",
            "prompt_cache_key": "cache-1",
            "metadata": {"session_id": "s1"},
            "verbosity": "medium"
        }),
    )
    .unwrap();

    assert_eq!(converted["parallel_tool_calls"], false);
    assert_eq!(converted["reasoning"]["effort"], "high");
    assert_eq!(converted["frequency_penalty"], 0.1);
    assert_eq!(converted["presence_penalty"], 0.2);
    assert_eq!(converted["service_tier"], "default");
    assert_eq!(converted["top_logprobs"], 3);
    assert_eq!(converted["user"], "user-1");
    assert_eq!(converted["prompt_cache_key"], "cache-1");
    assert_eq!(converted["metadata"]["session_id"], "s1");
    assert_eq!(converted["text"]["verbosity"], "medium");
}

#[test]
fn chat_to_responses_extracts_include_without_leaking_extra_body() {
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        json!({
            "model": "gpt-5.1-codex-mini",
            "messages": [{"role": "user", "content": "hi"}],
            "extra_body": {
                "include": ["reasoning.encrypted_content"],
                "max_tool_calls": 4,
                "prompt_cache_retention": "24h",
                "truncation": "auto",
                "unsupported_chat_extension": true
            }
        }),
    )
    .unwrap();

    assert_eq!(converted["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(converted["max_tool_calls"], 4);
    assert_eq!(converted["prompt_cache_retention"], "24h");
    assert_eq!(converted["truncation"], "auto");
    assert!(converted.get("extra_body").is_none());
}

#[test]
fn chat_to_responses_normalizes_function_tool_schema_for_strict_mode() {
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        json!({
            "model": "gpt-5.1-codex-mini",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "lookup",
                    "description": "Lookup data",
                    "strict": true,
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "city": {"type": "string"},
                            "unit": {"type": "string"}
                        },
                        "required": ["city"]
                    }
                }
            }, {
                "type": "function",
                "function": {
                    "name": "ping",
                    "description": "Ping",
                    "parameters": {"type": "object"}
                }
            }]
        }),
    )
    .unwrap();

    assert_eq!(
        converted["tools"][0]["parameters"]["additionalProperties"],
        false
    );
    assert_eq!(
        converted["tools"][0]["parameters"]["required"],
        json!(["city", "unit"])
    );
    assert_eq!(converted["tools"][1]["parameters"]["properties"], json!({}));
}

#[test]
fn responses_request_pass_through_fields_convert_to_chat() {
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        json!({
            "model": "gpt-5.1-codex-mini",
            "input": "hi",
            "tools": [{
                "type": "function",
                "name": "read_file",
                "description": "Read a file",
                "parameters": {"type": "object"}
            }],
            "parallel_tool_calls": true,
            "reasoning": {"effort": "medium"},
            "frequency_penalty": 0.3,
            "presence_penalty": 0.4,
            "service_tier": "auto",
            "top_logprobs": 2,
            "user": "user-2",
            "prompt_cache_key": "cache-2",
            "metadata": {"session_id": "s2"},
            "text": {
                "verbosity": "low",
                "format": {"type": "json_object"}
            }
        }),
    )
    .unwrap();

    assert_eq!(converted["parallel_tool_calls"], true);
    assert_eq!(converted["reasoning_effort"], "medium");
    assert_eq!(converted["frequency_penalty"], 0.3);
    assert_eq!(converted["presence_penalty"], 0.4);
    assert_eq!(converted["service_tier"], "auto");
    assert_eq!(converted["top_logprobs"], 2);
    assert_eq!(converted["user"], "user-2");
    assert_eq!(converted["prompt_cache_key"], "cache-2");
    assert_eq!(converted["metadata"]["session_id"], "s2");
    assert_eq!(converted["verbosity"], "low");
    assert_eq!(converted["response_format"]["type"], "json_object");
}

#[test]
fn chat_message_name_refusal_and_annotations_roundtrip_through_llm() {
    let llm = chat_response_to_llm(json!({
        "id": "chat_1",
        "model": "gpt-4o",
        "citations": [
            "https://example.com/source-a",
            "https://example.com/source-b"
        ],
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "name": "assistant_alias",
                "content": "I cannot help with that.",
                "refusal": "policy refusal",
                "annotations": [{
                    "type": "url_citation",
                    "url_citation": {"url": "https://example.com", "title": "Example"}
                }]
            },
            "finish_reason": "stop"
        }]
    }));

    let converted = llm_response_to_chat(llm);
    let message = &converted["choices"][0]["message"];
    assert_eq!(message["name"], "assistant_alias");
    assert_eq!(message["refusal"], "policy refusal");
    assert_eq!(message["annotations"][0]["type"], "url_citation");
    assert_eq!(converted["citations"][0], "https://example.com/source-a");
    assert_eq!(converted["citations"][1], "https://example.com/source-b");
}

#[test]
fn gemini_prompt_feedback_block_reason_maps_to_refusal_non_stream() {
    // Non-stream path (gemini/convert.rs): a Gemini response carrying
    // promptFeedback.blockReason and no candidates must surface as a
    // refusal message with finish_reason=refusal.
    let llm = gemini_response_to_llm(json!({
        "responseId": "gem_block_1",
        "modelVersion": "gemini-2.5",
        "promptFeedback": {
            "blockReason": "SAFETY",
            "blockReasonMessage": "blocked by safety filters"
        }
    }));
    let content = match &llm.choices[0].message.content {
        MessageContent::Text(text) => text.clone(),
        _ => String::new(),
    };
    assert!(
        content.contains("Request blocked by Gemini safety filters: SAFETY"),
        "blockReason should surface as refusal text, got: {content}"
    );
    assert_eq!(llm.choices[0].finish_reason.as_deref(), Some("refusal"));
}

#[test]
fn gemini_block_reason_stream_to_chat_emits_blocked_text_and_content_filter() {
    // Stream path (stream.rs): promptFeedback.blockReason -> TextDelta +
    // Finish(refusal). Routed to Chat, the refusal finish must map to
    // content_filter (stream.rs refusal => content_filter).
    let sse = "data: {\"promptFeedback\":{\"blockReason\":\"SAFETY\",\"blockReasonMessage\":\"blocked\"},\"candidates\":[]}\n\n";
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
        sse.to_string(),
    );
    assert!(
        output.contains("[blocked:SAFETY]"),
        "blockReason should emit a [blocked:...] text delta"
    );
    let finish = sse_data_values(&output)
        .into_iter()
        .find(|value| value["choices"][0]["finish_reason"] == "content_filter")
        .expect("refusal finish should map to content_filter for Chat target");
    assert!(finish["choices"][0]["finish_reason"] == "content_filter");
}

#[test]
fn chat_refusal_finish_stream_to_gemini_maps_to_safety() {
    // Stream LLM->Gemini: a Chat source with finish_reason=refusal produces a
    // unified refusal finish, which the Gemini outbound must map to SAFETY
    // (stream.rs refusal => SAFETY).
    let sse = "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":\"refusal\"}]}\n\ndata: [DONE]\n\n";
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        sse.to_string(),
    );
    assert!(
        output.contains("\"finishReason\":\"SAFETY\""),
        "Chat refusal finish should map to SAFETY finishReason for Gemini target"
    );
}

#[test]
fn refusal_finish_maps_to_gemini_safety_non_stream() {
    // Non-stream LLM->Gemini (gemini/convert.rs openai_finish_to_gemini_finish):
    // a refusal finish_reason must map to SAFETY.
    let llm = gemini_response_to_llm(json!({
        "responseId": "gem_block_2",
        "modelVersion": "gemini-2.5",
        "promptFeedback": {"blockReason": "SAFETY"}
    }));
    assert_eq!(llm.choices[0].finish_reason.as_deref(), Some("refusal"));
    let gemini = llm_response_to_gemini(llm);
    assert_eq!(gemini["candidates"][0]["finishReason"], "SAFETY");
}

#[test]
fn chat_outbound_normalizes_developer_role_to_system() {
    // Codex (Responses) carries developer instructions as `developer` messages.
    // Third-party OpenAI-compatible chat upstreams only accept `system`, so the
    // chat outbound must normalize `developer` -> `system`. Match is case-insensitive
    // so `Developer` / `DEVELOPER` are also normalized.
    let request = crate::coding::proxy_gateway::transformer::llm::Request {
        model: "kimi-k2".to_string(),
        messages: vec![
            Message {
                role: "developer".to_string(),
                content: MessageContent::Text("developer instructions".to_string()),
                ..Default::default()
            },
            Message {
                role: "Developer".to_string(),
                content: MessageContent::Text("uppercased developer".to_string()),
                ..Default::default()
            },
            Message {
                role: "user".to_string(),
                content: MessageContent::Text("hi".to_string()),
                ..Default::default()
            },
        ],
        request_type: Some(RequestType::Chat),
        api_format: Some(ApiFormat::OpenAiChatCompletions),
        ..Default::default()
    };
    let converted = OpenAiChatOutbound.request_from_llm(request).unwrap();
    let messages = converted["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(
        messages[0]["content"],
        "developer instructions\n\nuppercased developer"
    );
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages.len(), 2);
}

#[test]
fn reference_tool_result_name_is_preserved_for_gemini_function_response() {
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::GeminiNative),
        json!({
            "model": "claude-sonnet-4-5-20250929",
            "max_tokens": 100,
            "messages": [
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "call_weather",
                        "name": "get_weather",
                        "input": {"location": "Tokyo"}
                    }]
                },
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "call_weather",
                        "content": "sunny"
                    }]
                }
            ]
        }),
    )
    .unwrap();

    assert_eq!(
        converted["contents"][0]["parts"][0]["functionCall"]["name"],
        "get_weather"
    );
    assert_eq!(
        converted["contents"][1]["parts"][0]["functionResponse"]["id"],
        "call_weather"
    );
    assert_eq!(
        converted["contents"][1]["parts"][0]["functionResponse"]["name"],
        "get_weather"
    );
    assert_eq!(
        converted["contents"][1]["parts"][0]["functionResponse"]["response"]["result"],
        "sunny"
    );
}

#[test]
fn gemini_function_response_preserves_json_object_tool_result() {
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        json!({
            "model": "gemini-2.5-pro",
            "messages": [
                {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_weather",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\":\"Tokyo\"}"
                        }
                    }]
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_weather",
                    "content": "{\"temperature\":24,\"condition\":\"sunny\"}"
                }
            ]
        }),
    )
    .unwrap();

    assert_eq!(
        converted["contents"][1]["parts"][0]["functionResponse"]["response"],
        json!({"temperature": 24, "condition": "sunny"})
    );
}

#[test]
fn reference_openai_reasoning_fields_convert_to_reasoning_targets() {
    let source = json!({
        "model": "deepseek-reasoner",
        "messages": [{
            "role": "assistant",
            "reasoning": "I should inspect the inputs.",
            "content": "Done."
        }]
    });

    let anthropic = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        source.clone(),
    )
    .unwrap();
    assert_eq!(anthropic["messages"][0]["content"][0]["type"], "thinking");
    assert_eq!(
        anthropic["messages"][0]["content"][0]["thinking"],
        "I should inspect the inputs."
    );
    assert_eq!(anthropic["messages"][0]["content"][1]["text"], "Done.");

    let responses = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        source.clone(),
    )
    .unwrap();
    assert_eq!(responses["input"][0]["type"], "reasoning");
    assert_eq!(
        responses["input"][0]["summary"][0]["text"],
        "I should inspect the inputs."
    );
    assert_eq!(responses["input"][1]["content"][0]["text"], "Done.");

    let gemini = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        source,
    )
    .unwrap();
    assert_eq!(gemini["contents"][0]["parts"][0]["thought"], true);
    assert_eq!(
        gemini["contents"][0]["parts"][0]["text"],
        "I should inspect the inputs."
    );
}

#[test]
fn reference_responses_request_semantics_convert_exactly() {
    let source = json!({
        "model": "gpt-4o",
        "instructions": "You are a helpful assistant.",
        "input": "Hello!",
        "temperature": 0.7,
        "top_p": 0.9,
        "max_output_tokens": 1000,
        "stop": ["END"],
        "tool_choice": {
            "type": "function",
            "name": "get_weather"
        },
        "tools": [{
            "type": "function",
            "name": "get_weather",
            "description": "Get weather information",
            "parameters": {
                "type": "object",
                "properties": {"location": {"type": "string"}}
            },
            "strict": true
        }]
    });

    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        source.clone(),
    )
    .unwrap();
    assert_eq!(chat["messages"][0]["role"], "system");
    assert_eq!(chat["messages"][1]["role"], "user");
    assert_eq!(chat["max_tokens"], 1000);
    assert_eq!(chat["temperature"], 0.7);
    assert_eq!(chat["top_p"], 0.9);
    assert_eq!(chat["stop"], json!(["END"]));
    assert_eq!(chat["tool_choice"]["function"]["name"], "get_weather");
    assert_eq!(chat["tools"][0]["function"]["strict"], true);

    let anthropic = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::AnthropicMessages),
        source.clone(),
    )
    .unwrap();
    assert_eq!(anthropic["system"], "You are a helpful assistant.");
    assert_eq!(anthropic["messages"][0]["content"][0]["text"], "Hello!");
    assert_eq!(anthropic["max_tokens"], 1000);
    assert_eq!(anthropic["stop_sequences"], json!(["END"]));
    assert_eq!(anthropic["tool_choice"]["type"], "tool");
    assert_eq!(anthropic["tool_choice"]["name"], "get_weather");

    let gemini = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::GeminiNative),
        source,
    )
    .unwrap();
    assert_eq!(
        gemini["systemInstruction"]["parts"][0]["text"],
        "You are a helpful assistant."
    );
    assert_eq!(gemini["generationConfig"]["maxOutputTokens"], 1000);
    assert_eq!(gemini["generationConfig"]["stopSequences"], json!(["END"]));
    assert_eq!(
        gemini["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"][0],
        "get_weather"
    );
}

#[test]
fn responses_type_only_tool_choice_survives_to_responses_and_gemini() {
    let source = json!({
        "model": "gpt-4o",
        "input": "hi",
        "tool_choice": {"type": "image_generation"}
    });

    // Responses -> Responses: type-only round-trips cleanly via the IR
    // (no longer relies on the raw-preservation path).
    let responses = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiResponses),
        source.clone(),
    )
    .unwrap();
    assert_eq!(
        responses["tool_choice"],
        json!({"type": "image_generation"})
    );

    // Responses -> Gemini: a type-only choice must not emit an empty
    // allowedFunctionNames entry; it degrades to mode ANY.
    let gemini = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::GeminiNative),
        source,
    )
    .unwrap();
    assert_eq!(gemini["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
    assert!(gemini
        .pointer("/toolConfig/functionCallingConfig/allowedFunctionNames")
        .is_none());
}

#[test]
fn reference_gemini_thought_text_converts_to_reasoning() {
    let source = json!({
        "model": "gemini-2.5-pro",
        "contents": [{
            "role": "model",
            "parts": [
                {"text": "Plan first.", "thought": true},
                {"text": "Final answer."}
            ]
        }],
        "generationConfig": {
            "stopSequences": ["END"],
            "presencePenalty": 0.6,
            "frequencyPenalty": 0.7,
            "seed": 43
        },
        "toolConfig": {
            "functionCallingConfig": {
                "mode": "ANY",
                "allowedFunctionNames": ["get_weather"]
            }
        }
    });

    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
        source.clone(),
    )
    .unwrap();
    assert_eq!(chat["messages"][0]["reasoning_content"], "Plan first.");
    assert_eq!(chat["messages"][0]["content"][0]["text"], "Final answer.");
    assert_eq!(chat["stop"], json!(["END"]));
    assert_eq!(chat["presence_penalty"], 0.6);
    assert_eq!(chat["frequency_penalty"], 0.7);
    assert_eq!(chat["seed"], 43);
    assert_eq!(chat["tool_choice"]["function"]["name"], "get_weather");

    let anthropic = convert_request_value(
        ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::AnthropicMessages),
        source.clone(),
    )
    .unwrap();
    assert_eq!(anthropic["messages"][0]["content"][0]["type"], "thinking");
    assert_eq!(
        anthropic["messages"][0]["content"][0]["thinking"],
        "Plan first."
    );
    assert_eq!(
        anthropic["messages"][0]["content"][1]["text"],
        "Final answer."
    );

    let responses = convert_request_value(
        ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiResponses),
        source,
    )
    .unwrap();
    assert_eq!(responses["input"][0]["type"], "reasoning");
    assert_eq!(responses["input"][0]["summary"][0]["text"], "Plan first.");
    assert_eq!(responses["input"][1]["content"][0]["text"], "Final answer.");
}

#[test]
fn anthropic_thinking_signature_is_not_emitted_as_responses_encrypted_content() {
    let converted = convert_response_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiResponses),
        json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-5-20250929",
            "content": [
                {
                    "type": "thinking",
                    "thinking": "Plan first.",
                    "signature": "anthropic-private-signature"
                },
                {"type": "text", "text": "Done."}
            ],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 5, "output_tokens": 3}
        }),
    )
    .unwrap();

    assert_eq!(converted["output"][0]["type"], "reasoning");
    assert_eq!(converted["output"][0]["summary"][0]["text"], "Plan first.");
    assert!(converted.pointer("/output/0/encrypted_content").is_none());
    assert_eq!(converted["output"][1]["content"][0]["text"], "Done.");
}

#[test]
fn anthropic_json_signature_and_redacted_thinking_roundtrip_to_anthropic() {
    let source = json!({
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 1024,
        "messages": [{
            "role": "assistant",
            "content": [
                {
                    "type": "thinking",
                    "thinking": "Plan first.",
                    "signature": "EqQanthropic-request-signature"
                },
                {
                    "type": "redacted_thinking",
                    "data": "redacted-payload"
                },
                {"type": "text", "text": "Done."}
            ]
        }]
    });

    let llm = anthropic_request_to_llm(source);
    let roundtrip = AnthropicOutbound.request_from_llm(llm).unwrap();

    assert_eq!(
        roundtrip["messages"][0]["content"][0]["signature"],
        "EqQanthropic-request-signature"
    );
    assert_eq!(
        roundtrip["messages"][0]["content"][1],
        json!({"type": "redacted_thinking", "data": "redacted-payload"})
    );
    assert_eq!(roundtrip["messages"][0]["content"][2]["text"], "Done.");
}

#[test]
fn anthropic_response_signature_roundtrips_and_does_not_leak_to_gemini() {
    let source = json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5-20250929",
        "content": [
            {
                "type": "thinking",
                "thinking": "Plan first.",
                "signature": "EqQanthropic-response-signature"
            },
            {"type": "text", "text": "Done."}
        ],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 5, "output_tokens": 3}
    });

    let llm = AnthropicOutbound.response_to_llm(source.clone()).unwrap();
    let roundtrip = AnthropicInbound.response_from_llm(llm).unwrap();
    assert_eq!(
        roundtrip["content"][0]["signature"],
        "EqQanthropic-response-signature"
    );

    let gemini = convert_response_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::GeminiNative),
        source,
    )
    .unwrap();
    let thought_part = gemini["candidates"][0]["content"]["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part.get("thought").and_then(Value::as_bool) == Some(true))
        .expect("Gemini thought part");
    assert_ne!(
        thought_part["thoughtSignature"],
        "EqQanthropic-response-signature"
    );
    assert_eq!(
        thought_part["thoughtSignature"],
        DEFAULT_GEMINI_THOUGHT_SIGNATURE
    );
}

#[test]
fn responses_json_encrypted_content_roundtrips_and_preserves_include() {
    let source = json!({
        "model": "gpt-5.2",
        "include": ["file_search_call.results", "reasoning.encrypted_content"],
        "max_tool_calls": 7,
        "prompt_cache_retention": "24h",
        "truncation": "auto",
        "input": [{
            "type": "reasoning",
            "summary": [{"type": "summary_text", "text": "Need a tool."}],
            "encrypted_content": OPENAI_RESPONSES_FIXTURE_ENCRYPTED_CONTENT
        }]
    });

    let llm = responses_request_to_llm(source);
    let roundtrip = OpenAiResponsesOutbound.request_from_llm(llm).unwrap();

    assert_eq!(
        roundtrip["input"][0]["encrypted_content"],
        OPENAI_RESPONSES_FIXTURE_ENCRYPTED_CONTENT
    );
    assert_eq!(
        roundtrip["include"],
        json!(["file_search_call.results", "reasoning.encrypted_content"])
    );
    assert_eq!(roundtrip["max_tool_calls"], 7);
    assert_eq!(roundtrip["prompt_cache_retention"], "24h");
    assert_eq!(roundtrip["truncation"], "auto");
}

#[test]
fn responses_json_encrypted_only_reasoning_is_preserved() {
    let source = json!({
        "model": "gpt-5.2",
        "input": [{
            "type": "reasoning",
            "summary": [],
            "encrypted_content": OPENAI_RESPONSES_FIXTURE_ENCRYPTED_CONTENT
        }]
    });

    let llm = responses_request_to_llm(source);
    assert!(llm.messages[0].reasoning_content.is_none());
    assert!(llm.messages[0].reasoning_signature.is_some());

    let roundtrip = OpenAiResponsesOutbound.request_from_llm(llm).unwrap();
    assert_eq!(roundtrip["input"][0]["type"], "reasoning");
    assert_eq!(roundtrip["input"][0]["summary"], json!([]));
    assert_eq!(
        roundtrip["input"][0]["encrypted_content"],
        OPENAI_RESPONSES_FIXTURE_ENCRYPTED_CONTENT
    );
}

#[test]
fn responses_response_encrypted_content_roundtrips_and_does_not_leak() {
    let source = json!({
        "id": "resp_1",
        "object": "response",
        "model": "gpt-5.2",
        "status": "completed",
        "output": [{
            "type": "reasoning",
            "summary": [{"type": "summary_text", "text": "Plan."}],
            "encrypted_content": OPENAI_RESPONSES_FIXTURE_ENCRYPTED_CONTENT
        }],
        "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
    });

    let llm = OpenAiResponsesOutbound
        .response_to_llm(source.clone())
        .unwrap();
    let roundtrip = OpenAiResponsesInbound.response_from_llm(llm).unwrap();
    assert_eq!(
        roundtrip["output"][0]["encrypted_content"],
        OPENAI_RESPONSES_FIXTURE_ENCRYPTED_CONTENT
    );

    let anthropic = convert_response_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::AnthropicMessages),
        source.clone(),
    )
    .unwrap();
    assert!(anthropic.pointer("/content/0/signature").is_none());

    let gemini = convert_response_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::GeminiNative),
        source,
    )
    .unwrap();
    let thought_part = &gemini["candidates"][0]["content"]["parts"][0];
    assert_ne!(
        thought_part["thoughtSignature"],
        OPENAI_RESPONSES_FIXTURE_ENCRYPTED_CONTENT
    );
    assert_eq!(
        thought_part["thoughtSignature"],
        DEFAULT_GEMINI_THOUGHT_SIGNATURE
    );
}

#[test]
fn gemini_json_thought_signature_roundtrips_for_thought_text() {
    let source = json!({
        "contents": [{
            "role": "model",
            "parts": [
                {
                    "text": "Plan.",
                    "thought": true,
                    "thoughtSignature": "thought-signature"
                },
                {"text": "Done."}
            ]
        }]
    });

    let llm = gemini_request_to_llm(source);
    let roundtrip = llm_request_to_gemini(llm);
    let parts = roundtrip["contents"][0]["parts"].as_array().unwrap();
    assert_eq!(parts[0]["thoughtSignature"], "thought-signature");
}

#[test]
fn gemini_json_function_call_signature_roundtrips_to_function_call_part() {
    let source = json!({
        "contents": [{
            "role": "model",
            "parts": [
                {
                    "functionCall": {
                        "id": "call_1",
                        "name": "lookup",
                        "args": {"city": "Paris"}
                    },
                    "thoughtSignature": "tool-signature"
                }
            ]
        }]
    });

    let llm = gemini_request_to_llm(source);
    let roundtrip = llm_request_to_gemini(llm);
    let parts = roundtrip["contents"][0]["parts"].as_array().unwrap();
    assert_eq!(
        parts
            .iter()
            .find(|part| part.get("functionCall").is_some())
            .unwrap()["thoughtSignature"],
        "tool-signature"
    );
}

#[test]
fn gemini_json_per_tool_signature_does_not_move_to_first_tool() {
    let source = json!({
        "contents": [{
            "role": "model",
            "parts": [
                {
                    "functionCall": {
                        "id": "call_1",
                        "name": "first",
                        "args": {"n": 1}
                    }
                },
                {
                    "functionCall": {
                        "id": "call_2",
                        "name": "second",
                        "args": {"n": 2}
                    },
                    "thoughtSignature": "second-tool-signature"
                }
            ]
        }]
    });

    let llm = gemini_request_to_llm(source);
    assert!(llm.messages[0].reasoning_signature.is_none());
    let roundtrip = llm_request_to_gemini(llm);
    let parts = roundtrip["contents"][0]["parts"].as_array().unwrap();
    assert!(parts[0].get("thoughtSignature").is_none());
    assert_eq!(parts[1]["thoughtSignature"], "second-tool-signature");
}

#[test]
fn gemini_json_default_signature_only_goes_to_first_unsigned_tool() {
    let gemini = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        json!({
            "model": "model-a",
            "messages": [{
                "role": "assistant",
                "tool_calls": [
                    {
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "first", "arguments": "{\"n\":1}"}
                    },
                    {
                        "id": "call_2",
                        "type": "function",
                        "function": {"name": "second", "arguments": "{\"n\":2}"}
                    }
                ]
            }]
        }),
    )
    .unwrap();

    let parts = gemini["contents"][0]["parts"].as_array().unwrap();
    assert_eq!(
        parts[0]["thoughtSignature"],
        DEFAULT_GEMINI_THOUGHT_SIGNATURE
    );
    assert!(parts[1].get("thoughtSignature").is_none());
}

#[test]
fn gemini_json_non_gemini_signature_uses_default_not_raw_signature() {
    let openai_responses_signature = openai_responses_heuristic_signature();
    let message = Message {
        role: "assistant".to_string(),
        reasoning_signature: Some(openai_responses_signature.clone()),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            tool_type: TOOL_TYPE_FUNCTION.to_string(),
            function: FunctionCall {
                name: "lookup".to_string(),
                arguments: "{}".to_string(),
            },
            ..Default::default()
        }],
        ..Default::default()
    };

    let gemini = llm_request_to_gemini(crate::coding::proxy_gateway::transformer::llm::Request {
        model: "model-a".to_string(),
        messages: vec![message],
        ..Default::default()
    });
    assert_eq!(
        gemini["contents"][0]["parts"][0]["thoughtSignature"],
        DEFAULT_GEMINI_THOUGHT_SIGNATURE
    );
    assert_ne!(
        gemini["contents"][0]["parts"][0]["thoughtSignature"],
        openai_responses_signature.as_str()
    );
}

#[test]
fn anthropic_mixed_tool_results_and_thinking_are_preserved() {
    let converted = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        read_fixture_json("anthropic/anthropic-tool-result-mixed.request.json"),
    )
    .unwrap();
    let messages = converted["messages"].as_array().unwrap();
    let assistant = messages
        .iter()
        .find(|message| {
            message["role"] == "assistant"
                && message["tool_calls"]
                    .as_array()
                    .is_some_and(|calls| calls.len() == 2)
        })
        .expect("assistant message with two tool calls");
    assert_eq!(
        assistant["reasoning_content"],
        "I should gather weather data and the preferred unit before answering."
    );
    assert_eq!(
        assistant["content"][0]["text"],
        "Calling the weather and unit tools now."
    );

    let tool_messages = messages
        .iter()
        .filter(|message| message["role"] == "tool")
        .collect::<Vec<_>>();
    assert_eq!(tool_messages.len(), 2);
    assert_eq!(tool_messages[0]["tool_call_id"], "call_weather");
    assert_eq!(tool_messages[0]["content"], "Weather: light rain, 18C.");
    assert_eq!(tool_messages[1]["tool_call_id"], "call_unit");
    assert_eq!(tool_messages[1]["content"], "Preferred unit: Fahrenheit.");
    assert!(messages.iter().any(|message| {
        message["role"] == "user"
            && message["content"]
                .as_array()
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("text"))
                .and_then(Value::as_str)
                == Some("Please summarize the plan.")
    }));
}

#[test]
fn anthropic_tool_results_group_when_converting_back_to_anthropic() {
    let converted = convert_request_value(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            json!({
                "model": "claude-sonnet-4-5-20250929",
                "messages": [
                    {
                        "role": "assistant",
                        "tool_calls": [
                            {"id": "call_weather", "type": "function", "function": {"name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}"}},
                            {"id": "call_unit", "type": "function", "function": {"name": "get_unit", "arguments": "{\"country\":\"JP\"}"}}
                        ]
                    },
                    {"role": "tool", "tool_call_id": "call_weather", "content": "sunny"},
                    {"role": "tool", "tool_call_id": "call_unit", "content": "celsius"}
                ]
            }),
        )
        .unwrap();
    let messages = converted["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["content"][0]["type"], "tool_use");
    assert_eq!(messages[0]["content"][1]["type"], "tool_use");
    assert_eq!(messages[1]["role"], "user");
    let tool_results = messages[1]["content"].as_array().unwrap();
    assert_eq!(tool_results.len(), 2);
    assert_eq!(tool_results[0]["type"], "tool_result");
    assert_eq!(tool_results[0]["tool_use_id"], "call_weather");
    assert_eq!(tool_results[1]["type"], "tool_result");
    assert_eq!(tool_results[1]["tool_use_id"], "call_unit");
}

#[test]
fn responses_custom_tool_request_and_output_roundtrip() {
    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        read_fixture_json("openai_responses/custom_tool.request.json"),
    )
    .unwrap();
    let messages = chat["messages"].as_array().unwrap();
    let custom_call = &messages[2]["tool_calls"][0];
    assert_eq!(custom_call["type"], TOOL_TYPE_RESPONSES_CUSTOM_TOOL);
    assert_eq!(
        custom_call["response_custom_tool_call"]["name"],
        "apply_patch"
    );
    assert!(custom_call["response_custom_tool_call"]["input"]
        .as_str()
        .unwrap()
        .contains("*** Begin Patch"));
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["content"], "Patch applied successfully.");

    let responses = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        chat,
    )
    .unwrap();
    assert_eq!(responses["tools"][0]["type"], "custom");
    assert!(responses["input"].as_array().unwrap().iter().any(|item| {
        item["type"] == "custom_tool_call"
            && item["name"] == "apply_patch"
            && item["input"].as_str().unwrap().contains("*** Begin Patch")
    }));
    let custom_tool_call = responses["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "custom_tool_call")
        .unwrap();
    assert_eq!(custom_tool_call["id"], "ctc_call_patch_001");
    assert_eq!(custom_tool_call["call_id"], "call_patch_001");
    assert!(responses["input"].as_array().unwrap().iter().any(|item| {
        item["type"] == "custom_tool_call_output" && item["output"] == "Patch applied successfully."
    }));
}

#[test]
fn responses_custom_tool_stream_converts_to_chat() {
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        read_reference_stream_fixture("openai_responses/custom_tool.stream.jsonl"),
    );
    assert!(output.contains(TOOL_TYPE_RESPONSES_CUSTOM_TOOL));
    assert!(output.contains("response_custom_tool_call"));
    assert!(output.contains("apply_patch"));
    assert!(output.contains("*** Begin Patch"));
    let values = sse_data_values(&output);
    let finish = values
        .iter()
        .find(|value| value["choices"][0]["finish_reason"] == "tool_calls")
        .expect("custom tool stream should finish as tool_calls");
    assert_eq!(finish["choices"][0]["delta"], json!({}));
}

#[test]
fn responses_custom_tool_sparse_stream_surfaces_call_without_deltas() {
    // Sparse tool stream: only output_item.added + custom_tool_call_input.done,
    // no delta events in between. The added header must still surface the call
    // (id+name) and the done snapshot must flush the complete arguments, so a
    // non-streaming/dense-unaware client still receives the tool call.
    let sse = r#"event: response.output_item.added
data: {"type":"response.output_item.added","sequence_number":2,"output_index":0,"item":{"type":"custom_tool_call","status":"in_progress","call_id":"call_sparse_001","name":"apply_patch","input":""}}

event: response.custom_tool_call_input.done
data: {"type":"response.custom_tool_call_input.done","sequence_number":3,"item_id":"call_sparse_001","output_index":0,"input":"*** Begin Patch\n@@ def hello():\n-    print(\"Hello\")\n+    print(\"Hi\")\n*** End Patch\n"}

event: response.output_item.done
data: {"type":"response.output_item.done","sequence_number":4,"output_index":0,"item":{"type":"custom_tool_call","status":"completed","call_id":"call_sparse_001","name":"apply_patch","input":"*** Begin Patch\n@@ def hello():\n-    print(\"Hello\")\n+    print(\"Hi\")\n*** End Patch\n"}}

event: response.completed
data: {"type":"response.completed","sequence_number":5,"response":{"id":"resp_sparse","model":"gpt-5","status":"completed","output":[{"type":"custom_tool_call","status":"completed","call_id":"call_sparse_001","name":"apply_patch","input":"*** Begin Patch\n@@ def hello():\n-    print(\"Hello\")\n+    print(\"Hi\")\n*** End Patch\n"}],"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}

"#;
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        sse.to_string(),
    );
    assert!(output.contains(TOOL_TYPE_RESPONSES_CUSTOM_TOOL));
    assert!(output.contains("response_custom_tool_call"));
    assert!(output.contains("apply_patch"));
    assert!(output.contains("call_sparse_001"));
    // Complete arguments must be flushed from the done snapshot even without deltas.
    assert!(output.contains("*** Begin Patch"));
    assert!(output.contains("*** End Patch"));
    let finish = sse_data_values(&output)
        .into_iter()
        .find(|value| value["choices"][0]["finish_reason"] == "tool_calls")
        .expect("sparse custom tool stream should finish as tool_calls");
    assert_eq!(finish["choices"][0]["delta"], json!({}));
}

#[test]
fn chat_custom_tool_stream_converts_to_responses_custom_events() {
    let sse = r#"data: {"id":"chatcmpl_custom","object":"chat.completion.chunk","created":1,"model":"gpt-5","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}

data: {"id":"chatcmpl_custom","object":"chat.completion.chunk","created":1,"model":"gpt-5","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_custom_001","type":"responses_custom_tool","function":{"name":""},"response_custom_tool_call":{"call_id":"call_custom_001","name":"apply_patch","input":"*** Begin Patch\n*** End Patch"}}]},"finish_reason":null}]}

data: {"id":"chatcmpl_custom","object":"chat.completion.chunk","created":1,"model":"gpt-5","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        sse.to_string(),
    );
    let values = sse_data_values(&output);

    let added = values
        .iter()
        .find(|value| value["type"] == "response.output_item.added")
        .expect("custom tool item added");
    assert_eq!(added["item"]["type"], "custom_tool_call");
    assert_eq!(added["item"]["id"], "ctc_call_custom_001");
    assert_eq!(added["item"]["call_id"], "call_custom_001");
    assert_eq!(added["item"]["name"], "apply_patch");

    let delta = values
        .iter()
        .find(|value| value["type"] == "response.custom_tool_call_input.delta")
        .expect("custom tool input delta");
    assert_eq!(delta["item_id"], "ctc_call_custom_001");
    assert_eq!(delta["output_index"], added["output_index"]);
    assert!(delta["delta"].as_str().unwrap().contains("*** Begin Patch"));

    let done = values
        .iter()
        .find(|value| value["type"] == "response.custom_tool_call_input.done")
        .expect("custom tool input done");
    assert_eq!(done["item_id"], "ctc_call_custom_001");
    assert!(done["input"].as_str().unwrap().contains("*** End Patch"));

    let item_done = values
        .iter()
        .find(|value| {
            value["type"] == "response.output_item.done"
                && value["item"]["type"] == "custom_tool_call"
        })
        .expect("custom tool output item done");
    assert_eq!(item_done["item"]["id"], "ctc_call_custom_001");
    assert_eq!(item_done["item"]["input"], done["input"]);
    assert!(!values.iter().any(|value| {
        value["type"] == "response.output_item.added"
            && value["item"]["type"] == "function_call"
            && value["item"]["call_id"] == "call_custom_001"
    }));
}

#[test]
fn responses_failed_stream_to_chat_emits_error_envelope() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
            r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_failed","model":"gpt-5","status":"in_progress","output":[]}}

event: response.failed
data: {"type":"response.failed","response":{"id":"resp_failed","model":"gpt-5","status":"failed","output":[],"error":{"message":"boom","type":"server_error","code":"upstream_failed"}}}

"#
            .to_string(),
        );

    assert!(!output.contains("data: [DONE]"));
    let values = sse_data_values(&output);
    assert!(
        values
            .iter()
            .all(|value| value.pointer("/choices/0/finish_reason").is_none()
                || value["choices"][0]["finish_reason"].is_null()),
        "response.failed must not finish Chat with a success/error finish_reason"
    );
    let error = values
        .iter()
        .find(|value| value.get("error").is_some())
        .expect("response.failed should map to Chat error envelope");
    assert_eq!(error["error"]["message"], json!("boom"));
    // StreamError currently carries a single code field; source prefers error.code.
    assert_eq!(error["error"]["type"], json!("upstream_failed"));
    assert_eq!(error["error"]["code"], json!("upstream_failed"));
}

#[test]
fn responses_failed_stream_to_anthropic_emits_error_not_message_stop() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::AnthropicMessages),
            r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_failed","model":"gpt-5","status":"in_progress","output":[]}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","item_id":"msg_1","delta":"partial"}

event: response.failed
data: {"type":"response.failed","response":{"id":"resp_failed","model":"gpt-5","status":"failed","output":[],"error":{"message":"boom","type":"server_error"}}}

"#
            .to_string(),
        );

    assert!(output.contains("event: error"));
    assert!(output.contains(r#""message":"boom""#));
    assert!(!output.contains("event: message_stop"));
    assert!(!output.contains(r#""stop_reason":"end_turn""#));
    let values = sse_data_values(&output);
    let error = values
        .iter()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("error"))
        .expect("response.failed should map to Anthropic event:error");
    assert_eq!(error["error"]["message"], json!("boom"));
    assert_eq!(error["error"]["type"], json!("server_error"));
}

#[test]
fn responses_failed_stream_to_gemini_emits_error_not_stop() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::GeminiNative),
            r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_failed","model":"gpt-5","status":"in_progress","output":[]}}

event: response.failed
data: {"type":"response.failed","response":{"id":"resp_failed","model":"gpt-5","status":"failed","output":[],"error":{"message":"boom","type":"server_error","code":"upstream_failed"}}}

"#
            .to_string(),
        );

    assert!(!output.contains(r#""finishReason":"STOP""#));
    let values = sse_data_values(&output);
    let error = values
        .iter()
        .find(|value| value.get("error").is_some())
        .expect("response.failed should map to Gemini error envelope");
    assert_eq!(error["error"]["message"], json!("boom"));
}

#[test]
fn responses_failed_json_to_anthropic_emits_error_envelope() {
    let body = json!({
        "id": "resp_failed",
        "object": "response",
        "status": "failed",
        "model": "gpt-5",
        "output": [],
        "error": {
            "message": "boom",
            "type": "server_error",
            "code": "upstream_failed"
        }
    });
    let converted = convert_response_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::AnthropicMessages),
        body,
    )
    .expect("failed response should convert");
    assert_eq!(converted["type"], json!("error"));
    assert_eq!(converted["error"]["message"], json!("boom"));
    assert_eq!(converted["error"]["type"], json!("server_error"));
    assert_ne!(converted.get("stop_reason"), Some(&json!("end_turn")));
}

#[test]
fn responses_failed_json_to_chat_and_gemini_emit_error_envelopes() {
    let body = json!({
        "id": "resp_failed",
        "object": "response",
        "status": "failed",
        "model": "gpt-5",
        "output": [],
        "error": {
            "message": "boom",
            "type": "server_error",
            "code": "upstream_failed"
        }
    });

    let chat = convert_response_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        body.clone(),
    )
    .expect("failed response should convert to chat error");
    assert_eq!(chat["error"]["message"], json!("boom"));
    assert_eq!(chat["error"]["type"], json!("server_error"));
    assert_eq!(chat["error"]["code"], json!("upstream_failed"));
    assert!(chat.get("choices").is_none());

    let gemini = convert_response_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::GeminiNative),
        body,
    )
    .expect("failed response should convert to gemini error");
    assert_eq!(gemini["error"]["message"], json!("boom"));
    assert!(gemini.get("candidates").is_none());
}

#[test]
fn responses_incomplete_stream_to_gemini_finishes_with_max_tokens() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::GeminiNative),
            r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_incomplete","model":"gpt-5","status":"in_progress","output":[]}}

event: response.incomplete
data: {"type":"response.incomplete","response":{"id":"resp_incomplete","model":"gpt-5","status":"incomplete","output":[],"usage":{"input_tokens":2,"output_tokens":0,"total_tokens":2}}}

"#
            .to_string(),
        );

    let values = sse_data_values(&output);
    let finish = values
        .iter()
        .find(|value| value["candidates"][0]["finishReason"] == "MAX_TOKENS")
        .expect("response.incomplete should map to Gemini MAX_TOKENS");
    assert_eq!(finish["responseId"], "resp_incomplete");
}

#[test]
fn responses_cancelled_stream_passthrough_does_not_synthesize_completed() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiResponses),
            r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_cancelled","model":"gpt-5","status":"in_progress","output":[]}}

event: response.cancelled
data: {"type":"response.cancelled","response":{"id":"resp_cancelled","model":"gpt-5","status":"canceled","output":[]}}

"#
            .to_string(),
        );

    assert!(output.contains("event: response.cancelled"));
    assert!(!output.contains("event: response.completed"));
    let values = sse_data_values(&output);
    let cancelled = values
        .iter()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("response.cancelled"))
        .expect("response.cancelled event should be preserved");
    assert_eq!(cancelled["response"]["status"], "canceled");
}

#[test]
fn chat_stream_error_and_cancelled_finish_map_to_responses_terminal_events() {
    let failed = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            r#"data: {"id":"chat_error","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

data: {"id":"chat_error","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"error"}]}

"#
            .to_string(),
        );
    assert!(failed.contains("event: response.failed"));
    assert!(!failed.contains("event: response.completed"));
    let failed_values = sse_data_values(&failed);
    let failed_event = failed_values
        .iter()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("response.failed"))
        .expect("Chat error finish should map to response.failed");
    assert_eq!(failed_event["response"]["status"], "failed");
    assert_eq!(failed_event["response"]["error"]["code"], "response_error");

    let cancelled = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            r#"data: {"id":"chat_cancelled","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

data: {"id":"chat_cancelled","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"cancelled"}]}

"#
            .to_string(),
        );
    assert!(cancelled.contains("event: response.cancelled"));
    assert!(!cancelled.contains("event: response.completed"));
    let cancelled_values = sse_data_values(&cancelled);
    let cancelled_event = cancelled_values
        .iter()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("response.cancelled"))
        .expect("Chat cancelled finish should map to response.cancelled");
    assert_eq!(cancelled_event["response"]["status"], "canceled");
}

#[test]
fn gemini_thinking_config_maps_to_reasoning_effort_and_back() {
    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
        read_fixture_json("gemini/gemini-thinking.request.json"),
    )
    .unwrap();
    assert_eq!(chat["reasoning_effort"], "high");

    let gemini = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        json!({
            "model": "gemini-2.5-flash",
            "reasoning_effort": "medium",
            "messages": [{"role": "user", "content": "think"}]
        }),
    )
    .unwrap();
    assert_eq!(
        gemini["generationConfig"]["thinkingConfig"]["includeThoughts"],
        true
    );
    assert_eq!(
        gemini["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        10240
    );

    let capped = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        json!({
            "model": "gemini-2.5-pro",
            "reasoning_effort": "high",
            "messages": [{"role": "user", "content": "think hard"}]
        }),
    )
    .unwrap();
    assert_eq!(
        capped["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        24576
    );

    let gemini3 = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        json!({
            "model": "gemini-3-pro",
            "reasoning_effort": "xhigh",
            "messages": [{"role": "user", "content": "think hard"}]
        }),
    )
    .unwrap();
    assert_eq!(
        gemini3["generationConfig"]["thinkingConfig"]["thinkingLevel"],
        "high"
    );
    assert!(gemini3["generationConfig"]["thinkingConfig"]
        .get("thinkingBudget")
        .is_none());
}

#[test]
fn gemini_usage_includes_thought_tokens() {
    let chat = convert_response_value(
        ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
        json!({
            "responseId": "resp_gemini",
            "modelVersion": "gemini-2.5-flash",
            "candidates": [{
                "content": {"role": "model", "parts": [{"text": "done"}]},
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 50,
                "thoughtsTokenCount": 100,
                "totalTokenCount": 250
            }
        }),
    )
    .unwrap();
    assert_eq!(chat["usage"]["completion_tokens"], 150);
    assert_eq!(
        chat["usage"]["completion_tokens_details"]["reasoning_tokens"],
        100
    );
    assert_eq!(chat["usage"]["total_tokens"], 250);

    let gemini = convert_response_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        json!({
            "id": "chat_1",
            "object": "chat.completion",
            "model": "gemini-2.5-flash",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "done"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 150,
                "total_tokens": 250,
                "completion_tokens_details": {"reasoning_tokens": 100}
            }
        }),
    )
    .unwrap();
    assert_eq!(gemini["usageMetadata"]["candidatesTokenCount"], 50);
    assert_eq!(gemini["usageMetadata"]["thoughtsTokenCount"], 100);
    assert_eq!(gemini["usageMetadata"]["totalTokenCount"], 250);
}

#[test]
fn gemini_function_response_id_is_backfilled_from_previous_function_call() {
    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
        read_fixture_json("gemini/gemini-tool-result.request.json"),
    )
    .unwrap();
    let tool_message = chat["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "tool")
        .expect("tool response message");
    assert_eq!(
        tool_message["tool_call_id"],
        "call_00_IMEgeiAgajAZ47qX9hzSnjBP"
    );
}

#[test]
fn gemini_function_response_id_falls_back_to_name_without_history() {
    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
        json!({
            "contents": [{
                "role": "user",
                "parts": [{
                    "functionResponse": {
                        "name": "lookup_weather",
                        "response": {"content": "sunny"}
                    }
                }]
            }]
        }),
    )
    .unwrap();
    let tool_message = chat["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "tool")
        .expect("tool response message");
    assert_eq!(tool_message["tool_call_id"], "lookup_weather");
    assert_eq!(tool_message["content"], "sunny");
}

#[test]
fn gemini_file_data_and_image_url_convert_both_directions() {
    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
        json!({
            "contents": [{
                "role": "user",
                "parts": [{
                    "fileData": {
                        "mimeType": "image/png",
                        "fileUri": "https://example.com/chart.png"
                    }
                }]
            }]
        }),
    )
    .unwrap();
    assert_eq!(
        chat["messages"][0]["content"][0]["image_url"]["url"],
        "https://example.com/chart.png"
    );

    let gemini = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        json!({
            "model": "gemini-2.5-flash",
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image_url",
                    "image_url": {"url": "https://example.com/chart.png"}
                }]
            }]
        }),
    )
    .unwrap();
    assert_eq!(
        gemini["contents"][0]["parts"][0]["fileData"]["fileUri"],
        "https://example.com/chart.png"
    );
}

#[test]
fn inbound_requests_mark_request_type_and_api_format() {
    let chat = chat_request_to_llm(json!({
        "model": "model-a",
        "messages": [{"role": "user", "content": "hi"}]
    }));
    assert_eq!(chat.request_type, Some(RequestType::Chat));
    assert_eq!(chat.api_format, Some(ApiFormat::OpenAiChatCompletions));

    let responses = responses_request_to_llm(json!({
        "model": "model-a",
        "input": "hi"
    }));
    assert_eq!(responses.request_type, Some(RequestType::Chat));
    assert_eq!(responses.api_format, Some(ApiFormat::OpenAiResponses));

    let anthropic = anthropic_request_to_llm(json!({
        "model": "model-a",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "hi"}]
    }));
    assert_eq!(anthropic.request_type, Some(RequestType::Chat));
    assert_eq!(anthropic.api_format, Some(ApiFormat::AnthropicMessages));

    let gemini = gemini_request_to_llm(json!({
        "model": "model-a",
        "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
    }));
    assert_eq!(gemini.request_type, Some(RequestType::Chat));
    assert_eq!(gemini.api_format, Some(ApiFormat::GeminiContents));

    let compact = responses_compact_request_to_llm(json!({
        "model": "model-a",
        "input": [{"role": "user", "content": [{"type": "input_text", "text": "hi"}]}]
    }));
    assert_eq!(compact.request_type, Some(RequestType::Compact));
    assert_eq!(compact.api_format, Some(ApiFormat::OpenAiResponsesCompact));
    assert_eq!(compact.stream, Some(false));
}

#[test]
fn responses_compact_request_converts_to_supported_fallback_targets() {
    let compact_request = json!({
        "model": "model-a",
        "input": [{
            "role": "user",
            "content": [{"type": "input_text", "text": "summarize this context"}]
        }]
    });

    let (chat, chat_context) = convert_responses_compact_request_value_to_target(
        AiProtocol::OpenAiChat,
        compact_request.clone(),
    )
    .unwrap();
    assert!(chat_context.is_empty());
    assert!(chat.get("messages").and_then(Value::as_array).is_some());
    assert_eq!(chat["stream"], false);

    let (anthropic, anthropic_context) = convert_responses_compact_request_value_to_target(
        AiProtocol::AnthropicMessages,
        compact_request.clone(),
    )
    .unwrap();
    assert!(anthropic_context.is_empty());
    assert!(anthropic
        .get("messages")
        .and_then(Value::as_array)
        .is_some());
    assert_eq!(anthropic["messages"][0]["role"], "user");

    let (gemini, gemini_context) = convert_responses_compact_request_value_to_target(
        AiProtocol::GeminiNative,
        compact_request,
    )
    .unwrap();
    assert!(gemini_context.is_empty());
    assert!(gemini.get("contents").and_then(Value::as_array).is_some());
    assert_eq!(gemini["contents"][0]["role"], "user");
}

#[test]
fn target_response_conversion_to_responses_compact_sets_compaction_object() {
    let compact = convert_target_response_value_to_responses_compact(
        AiProtocol::OpenAiChat,
        json!({
            "id": "chatcmpl_1",
            "object": "chat.completion",
            "created": 1,
            "model": "model-a",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "done"},
                "finish_reason": "stop"
            }]
        }),
        None,
    )
    .unwrap();

    assert_eq!(compact["object"], "response.compaction");
    assert_eq!(compact["status"], "completed");
    assert_eq!(compact["output"][0]["type"], "message");
}

#[test]
fn responses_identity_stream_preserves_compaction_bytes_without_kernel() {
    let source = [
            r#"event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"id":"","type":"compaction","encrypted_content":"enc_identity"}}

"#,
            r#"event: response.output_item.done
data: {"type":"response.output_item.done","output_index":0,"item":{"id":"","type":"compaction","encrypted_content":"enc_identity"}}

"#,
        ]
        .concat();

    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiResponses),
        source.clone(),
    );
    assert_eq!(output, source);
}

#[test]
fn responses_stream_drops_compaction_for_chat_without_losing_text() {
    let source = [
            r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_cmp","object":"response","model":"gpt-5","status":"in_progress","output":[]}}

"#,
            r#"event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"id":"cmp_stream_1","type":"compaction","encrypted_content":"enc_stream_1"}}

"#,
            r#"event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"hello","item_id":"msg_1","output_index":1,"content_index":0}

"#,
            r#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_cmp","object":"response","model":"gpt-5","status":"completed","output":[{"id":"cmp_stream_1","type":"compaction","encrypted_content":"enc_stream_1"},{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}]}}

"#,
        ]
        .concat();

    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        source,
    );
    assert!(output.contains("hello"));
    assert!(!output.contains("enc_stream_1"));
}

#[test]
fn chat_reasoning_field_syncs_into_llm_message() {
    let llm = chat_request_to_llm(json!({
        "model": "model-a",
        "messages": [{
            "role": "assistant",
            "reasoning": "internal trace",
            "content": "answer"
        }]
    }));
    assert_eq!(
        llm.messages[0].reasoning_content.as_deref(),
        Some("internal trace")
    );
    assert_eq!(llm.messages[0].reasoning.as_deref(), Some("internal trace"));

    let anthropic = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        json!({
            "model": "model-a",
            "messages": [{
                "role": "assistant",
                "reasoning": "internal trace",
                "content": "answer"
            }]
        }),
    )
    .unwrap();
    assert_eq!(anthropic["messages"][0]["content"][0]["type"], "thinking");
    assert_eq!(
        anthropic["messages"][0]["content"][0]["thinking"],
        "internal trace"
    );
}

#[test]
fn chat_inline_think_block_converts_to_reasoning() {
    let anthropic = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        json!({
            "model": "model-a",
            "messages": [{
                "role": "assistant",
                "content": "<think>\ninspect first\n</think>\n\nvisible answer"
            }]
        }),
    )
    .unwrap();

    assert_eq!(anthropic["messages"][0]["content"][0]["type"], "thinking");
    assert_eq!(
        anthropic["messages"][0]["content"][0]["thinking"],
        "inspect first"
    );
    assert_eq!(anthropic["messages"][0]["content"][1]["type"], "text");
    assert_eq!(
        anthropic["messages"][0]["content"][1]["text"],
        "visible answer"
    );
}

#[test]
fn responses_reasoning_item_merges_following_function_call() {
    let llm = responses_request_to_llm(json!({
        "model": "model-a",
        "input": [
            {
                "type": "reasoning",
                "summary": [{"type": "summary_text", "text": "need tool"}]
            },
            {
                "type": "function_call",
                "call_id": "call_weather",
                "name": "lookup_weather",
                "arguments": "{\"city\":\"Tokyo\"}"
            }
        ]
    }));
    assert_eq!(llm.messages.len(), 1);
    assert_eq!(llm.messages[0].role, "assistant");
    assert_eq!(
        llm.messages[0].reasoning_content.as_deref(),
        Some("need tool")
    );
    assert_eq!(llm.messages[0].tool_calls.len(), 1);
    assert_eq!(
        llm.messages[0].tool_calls[0].function.name,
        "lookup_weather"
    );
}

#[test]
fn responses_standalone_input_image_item_converts_to_image_url() {
    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        json!({
            "model": "model-a",
            "input": [{
                "type": "input_image",
                "image_url": "https://example.com/input.png",
                "detail": "high"
            }]
        }),
    )
    .unwrap();
    assert_eq!(
        chat["messages"][0]["content"][0]["image_url"]["url"],
        "https://example.com/input.png"
    );
    assert_eq!(
        chat["messages"][0]["content"][0]["image_url"]["detail"],
        "high"
    );
}

#[test]
fn responses_status_and_previous_response_metadata_roundtrip() {
    let llm = responses_response_to_llm(json!({
        "id": "resp_1",
        "model": "model-a",
        "created_at": 123,
        "previous_response_id": "resp_0",
        "status": "failed",
        "output": []
    }));
    assert_eq!(llm.previous_response_id.as_deref(), Some("resp_0"));
    assert_eq!(llm.created, 123);
    assert_eq!(llm.choices[0].finish_reason.as_deref(), Some("error"));

    let responses = llm_response_to_responses(llm);
    assert_eq!(responses["previous_response_id"], "resp_0");
    assert_eq!(responses["created_at"], 123);
    assert_eq!(responses["status"], "failed");
}

#[test]
fn responses_tool_call_items_include_completed_status() {
    let responses = convert_response_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        json!({
            "id": "chat_1",
            "object": "chat.completion",
            "created": 123,
            "model": "model-a",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_weather",
                        "type": "function",
                        "function": {
                            "name": "lookup_weather",
                            "arguments": "{\"city\":\"Tokyo\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }),
    )
    .unwrap();
    assert_eq!(responses["output"][0]["status"], "completed");
}

#[test]
fn responses_read_tool_call_drops_empty_pages_when_converted_to_anthropic() {
    let converted = convert_response_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::AnthropicMessages),
        json!({
            "id": "resp_read",
            "object": "response",
            "created_at": 123,
            "model": "gpt-5.1-codex-mini",
            "status": "completed",
            "output": [{
                "type": "function_call",
                "id": "fc_read",
                "call_id": "call_read",
                "name": "Read",
                "arguments": "{\"file_path\":\"/tmp/a.md\",\"pages\":\"\"}",
                "status": "completed"
            }],
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
        }),
    )
    .unwrap();

    assert_eq!(converted["content"][0]["type"], "tool_use");
    assert_eq!(converted["content"][0]["name"], "Read");
    assert_eq!(converted["content"][0]["input"]["file_path"], "/tmp/a.md");
    assert!(converted["content"][0]["input"].get("pages").is_none());
}

#[test]
fn malformed_tool_arguments_are_repaired_or_preserved() {
    let repaired = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        json!({
            "model": "claude-sonnet-4-6",
            "messages": [{
                "role": "assistant",
                "tool_calls": [{
                    "id": "call_read",
                    "type": "function",
                    "function": {
                        "name": "Read",
                        "arguments": "{\"path\":\"a\",}"
                    }
                }]
            }]
        }),
    )
    .unwrap();
    assert_eq!(
        repaired["messages"][0]["content"][0]["input"],
        json!({"path": "a"})
    );

    let single_quoted = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        json!({
            "model": "claude-sonnet-4-6",
            "messages": [{
                "role": "assistant",
                "tool_calls": [{
                    "id": "call_read",
                    "type": "function",
                    "function": {
                        "name": "Read",
                        "arguments": "{path:'a'}"
                    }
                }]
            }]
        }),
    )
    .unwrap();
    assert_eq!(
        single_quoted["messages"][0]["content"][0]["input"],
        json!({"path": "a"})
    );

    let preserved = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        json!({
            "model": "claude-sonnet-4-6",
            "messages": [{
                "role": "assistant",
                "tool_calls": [{
                    "id": "call_read",
                    "type": "function",
                    "function": {
                        "name": "Read",
                        "arguments": "{\"path\":\"a\""
                    }
                }]
            }]
        }),
    )
    .unwrap();
    assert_eq!(
        preserved["messages"][0]["content"][0]["input"],
        "{\"path\":\"a\""
    );
}

#[test]
fn chat_response_preserves_multiple_choices() {
    let llm = chat_response_to_llm(json!({
        "id": "chat_multi",
        "object": "chat.completion",
        "created": 123,
        "model": "model-a",
        "choices": [
            {
                "index": 0,
                "message": {"role": "assistant", "content": "first"},
                "finish_reason": "stop"
            },
            {
                "index": 1,
                "message": {"role": "assistant", "content": "second"},
                "finish_reason": "length"
            }
        ]
    }));
    assert_eq!(llm.choices.len(), 2);
    assert_eq!(llm.choices[1].index, 1);

    let chat = llm_response_to_chat(llm);
    assert_eq!(chat["choices"].as_array().unwrap().len(), 2);
    assert_eq!(chat["choices"][1]["message"]["content"], "second");
    assert_eq!(chat["choices"][1]["finish_reason"], "length");
}

#[test]
fn gemini_response_preserves_multiple_candidates() {
    let llm = gemini_response_to_llm(json!({
        "responseId": "gemini_multi",
        "modelVersion": "gemini-2.5-flash",
        "candidates": [
            {
                "content": {"role": "model", "parts": [{"text": "first"}]},
                "finishReason": "STOP"
            },
            {
                "content": {"role": "model", "parts": [{"text": "second"}]},
                "finishReason": "MAX_TOKENS"
            }
        ]
    }));
    assert_eq!(llm.choices.len(), 2);
    assert_eq!(llm.choices[1].finish_reason.as_deref(), Some("length"));

    let gemini = llm_response_to_gemini(llm);
    assert_eq!(gemini["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(
        gemini["candidates"][1]["content"]["parts"][0]["text"],
        "second"
    );
    assert_eq!(gemini["candidates"][1]["finishReason"], "MAX_TOKENS");
}

#[test]
fn gemini_response_empty_finish_reason_stays_absent() {
    let llm = gemini_response_to_llm(json!({
        "responseId": "gemini_empty_finish",
        "modelVersion": "gemini-2.5-flash",
        "candidates": [{
            "content": {"role": "model", "parts": [{"text": "pending"}]},
            "finishReason": ""
        }]
    }));

    assert_eq!(llm.choices.len(), 1);
    match &llm.choices[0].message.content {
        MessageContent::Parts(parts) => {
            assert_eq!(parts.len(), 1);
            assert_eq!(parts[0].part_type, "text");
            assert_eq!(parts[0].text.as_deref(), Some("pending"));
        }
        other => panic!("expected text part content, got {other:?}"),
    }
    assert_eq!(llm.choices[0].finish_reason, None);
}

#[test]
fn gemini_system_instruction_filters_thought_parts() {
    let llm = gemini_request_to_llm(json!({
        "systemInstruction": {
            "parts": [
                {"text": "visible"},
                {"text": "hidden", "thought": true},
                {"text": "also visible"}
            ]
        },
        "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
    }));
    assert_eq!(
        llm.messages[0].content,
        MessageContent::Text("visible\nalso visible".to_string())
    );
}

#[test]
fn gemini_tool_choice_allowed_names_respects_mode() {
    let auto_chat = convert_request_value(
        ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
        json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "toolConfig": {
                "functionCallingConfig": {
                    "mode": "AUTO",
                    "allowedFunctionNames": ["lookup_weather"]
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(auto_chat["tool_choice"], "auto");

    let required_chat = convert_request_value(
        ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
        json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "toolConfig": {
                "functionCallingConfig": {
                    "mode": "ANY",
                    "allowedFunctionNames": ["lookup_weather", "lookup_time"]
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(required_chat["tool_choice"], "required");
}

#[test]
fn anthropic_image_missing_media_type_uses_octet_stream() {
    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        json!({
            "model": "model-a",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "data": "AAAA"
                    }
                }]
            }]
        }),
    )
    .unwrap();
    assert_eq!(
        chat["messages"][0]["content"][0]["image_url"]["url"],
        "data:application/octet-stream;base64,AAAA"
    );
}

#[test]
fn anthropic_metadata_and_max_tokens_roundtrip_to_anthropic_target() {
    let anthropic = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        json!({
            "model": "model-a",
            "metadata": {"user_id": "user_1"},
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .unwrap();
    assert_eq!(anthropic["metadata"]["user_id"], "user_1");
    assert_eq!(anthropic["max_tokens"], 8192);
}

#[test]
fn anthropic_metadata_does_not_leak_to_openai_targets() {
    let anthropic = json!({
        "model": "model-a",
        "max_tokens": 1024,
        "metadata": {
            "user_id": "user_1",
            "session_id": "anthropic-session",
            "request_id": "anthropic-request",
            "custom_nested": {"secret": "source-only"}
        },
        "messages": [{"role": "user", "content": "hi"}]
    });
    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        anthropic.clone(),
    )
    .unwrap();
    let responses = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiResponses),
        anthropic,
    )
    .unwrap();

    assert!(chat.get("metadata").is_none());
    assert!(responses.get("metadata").is_none());
    let chat_text = serde_json::to_string(&chat).unwrap();
    let responses_text = serde_json::to_string(&responses).unwrap();
    for leaked_value in [
        "user_1",
        "anthropic-session",
        "anthropic-request",
        "source-only",
    ] {
        assert!(!chat_text.contains(leaked_value));
        assert!(!responses_text.contains(leaked_value));
    }
}

#[test]
fn anthropic_tool_use_to_responses_uses_responses_item_id() {
    let responses = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiResponses),
        json!({
            "model": "model-a",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "call_iAamgdUMID7fjUog5w2YxfIP",
                    "name": "read_file",
                    "input": {"path": "a.txt"}
                }]
            }]
        }),
    )
    .unwrap();
    let function_call = responses["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call")
        .unwrap();

    assert_eq!(function_call["id"], "fc_call_iAamgdUMID7fjUog5w2YxfIP");
    assert_eq!(function_call["call_id"], "call_iAamgdUMID7fjUog5w2YxfIP");
}

#[test]
fn anthropic_tool_use_and_result_lift_to_responses_items() {
    let responses = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiResponses),
        json!({
            "model": "model-a",
            "max_tokens": 1024,
            "messages": [
                {
                    "role": "user",
                    "content": "read README"
                },
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "call_readme",
                        "name": "read_file",
                        "input": {"path": "README.md"}
                    }]
                },
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "call_readme",
                        "content": "file contents"
                    }]
                }
            ]
        }),
    )
    .unwrap();
    let input = responses["input"].as_array().unwrap();

    assert_eq!(input[0]["type"], "message");
    assert_eq!(input[0]["role"], "user");
    assert_eq!(input[1]["type"], "function_call");
    assert_eq!(input[1]["id"], "fc_call_readme");
    assert_eq!(input[1]["call_id"], "call_readme");
    assert_eq!(input[1]["name"], "read_file");
    assert_eq!(input[1]["status"], "completed");
    let arguments: Value = serde_json::from_str(input[1]["arguments"].as_str().unwrap())
        .expect("function_call arguments should stay JSON encoded");
    assert_eq!(arguments["path"], "README.md");
    assert_eq!(input[2]["type"], "function_call_output");
    assert_eq!(input[2]["call_id"], "call_readme");
    assert_eq!(input[2]["output"], "file contents");
}

#[test]
fn anthropic_thinking_syncs_reasoning_fields() {
    let llm = anthropic_request_to_llm(json!({
        "model": "model-a",
        "max_tokens": 1024,
        "messages": [{
            "role": "assistant",
            "content": [{
                "type": "thinking",
                "thinking": "internal trace"
            }]
        }]
    }));
    assert_eq!(
        llm.messages[0].reasoning_content.as_deref(),
        Some("internal trace")
    );
    assert_eq!(llm.messages[0].reasoning.as_deref(), Some("internal trace"));
}

#[test]
fn gemini_thinking_budget_threshold_uses_standard_effort_mapping() {
    let minimal = gemini_request_to_llm(json!({
        "generationConfig": {
            "thinkingConfig": {"includeThoughts": true, "thinkingBudget": 1024}
        },
        "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
    }));
    assert_eq!(minimal.reasoning_effort.as_deref(), Some("minimal"));

    let low = gemini_request_to_llm(json!({
        "generationConfig": {
            "thinkingConfig": {"includeThoughts": true, "thinkingBudget": 4096}
        },
        "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
    }));
    assert_eq!(low.reasoning_effort.as_deref(), Some("low"));

    let medium = gemini_request_to_llm(json!({
        "generationConfig": {
            "thinkingConfig": {"includeThoughts": true, "thinkingBudget": 10240}
        },
        "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
    }));
    assert_eq!(medium.reasoning_effort.as_deref(), Some("medium"));

    let high = gemini_request_to_llm(json!({
        "generationConfig": {
            "thinkingConfig": {"includeThoughts": true, "thinkingBudget": 32768}
        },
        "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
    }));
    assert_eq!(high.reasoning_effort.as_deref(), Some("high"));
}

#[test]
fn gemini_thought_text_syncs_reasoning_fields() {
    let llm = gemini_request_to_llm(json!({
        "contents": [{
            "role": "model",
            "parts": [
                {"text": "internal", "thought": true},
                {"text": "answer"}
            ]
        }]
    }));
    assert_eq!(
        llm.messages[0].reasoning_content.as_deref(),
        Some("internal")
    );
    assert_eq!(llm.messages[0].reasoning.as_deref(), Some("internal"));
}

#[test]
fn responses_request_preserves_assistant_text_before_tool_call() {
    let responses = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        json!({
            "model": "model-a",
            "messages": [{
                "role": "assistant",
                "content": "I will call a tool.",
                "tool_calls": [{
                    "id": "call_weather",
                    "type": "function",
                    "function": {
                        "name": "lookup_weather",
                        "arguments": "{\"city\":\"Tokyo\"}"
                    }
                }]
            }]
        }),
    )
    .unwrap();
    let input = responses["input"].as_array().unwrap();
    let message_index = input
        .iter()
        .position(|item| item["type"] == "message")
        .expect("assistant message item");
    let tool_index = input
        .iter()
        .position(|item| item["type"] == "function_call")
        .expect("function call item");
    assert!(message_index < tool_index);
    assert_eq!(
        input[message_index]["content"][0]["text"],
        "I will call a tool."
    );
    assert_eq!(input[tool_index]["name"], "lookup_weather");
}

#[test]
fn chat_legacy_function_call_response_converts_to_anthropic_tool_use() {
    let anthropic = convert_response_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        json!({
            "id": "chat_legacy",
            "object": "chat.completion",
            "model": "model-a",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "function_call": {
                        "name": "lookup_weather",
                        "arguments": "{\"city\":\"Tokyo\"}"
                    }
                },
                "finish_reason": "function_call"
            }]
        }),
    )
    .unwrap();
    assert_eq!(anthropic["content"][0]["type"], "tool_use");
    assert_eq!(anthropic["content"][0]["name"], "lookup_weather");
    assert_eq!(anthropic["content"][0]["input"]["city"], "Tokyo");
    assert_eq!(anthropic["stop_reason"], "tool_use");
}

#[test]
fn responses_tool_done_snapshot_mismatch_does_not_panic_on_utf8_boundary() {
    // Delta accumulates ASCII-only bytes so emitted_len can land mid multi-byte
    // char after the done snapshot replaces the buffer with CJK JSON.
    // Example: emitted_len=3 after "{\"x", done snapshot = {"好":1} → byte 3
    // is mid-char for "好" (E5 A5 BD). Must not panic under panic=abort.
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
            [
                r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_utf8","model":"model-a"}}

"#,
                r#"event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"id":"item_1","type":"function_call","call_id":"call_1","name":"lookup"}}

"#,
                r#"event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","item_id":"item_1","output_index":0,"delta":"{\"x"}

"#,
                r#"event: response.function_call_arguments.done
data: {"type":"response.function_call_arguments.done","item_id":"item_1","output_index":0,"arguments":"{\"好\":1}"}

"#,
                r#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_utf8","model":"model-a","status":"completed"}}

"#,
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let mut joined_arguments = String::new();
    for value in &values {
        if let Some(tool_calls) = value
            .pointer("/choices/0/delta/tool_calls")
            .and_then(Value::as_array)
        {
            for tool_call in tool_calls {
                if let Some(arguments) = tool_call
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                {
                    joined_arguments.push_str(arguments);
                }
            }
        }
    }
    // After diverged done snapshot we re-emit the full authoritative snapshot.
    // Previously emitted "{\"x" remains in the Chat stream; final content must
    // still contain the done snapshot without panicking.
    assert!(
            joined_arguments.contains(r#"{"好":1}"#),
            "done snapshot must be emitted after UTF-8 mismatch; got {joined_arguments:?}; output={output}"
        );
}

#[test]
fn responses_dense_tool_stream_does_not_duplicate_arguments_for_chat() {
    // Real OpenAI wire: added + deltas + arguments.done + output_item.done + completed.
    // Done/finish must only emit the unsent suffix, otherwise Chat concatenates 4x args.
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
            [
                r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_dense","model":"model-a"}}

"#,
                r#"event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"id":"item_1","type":"function_call","call_id":"call_1","name":"lookup"}}

"#,
                r#"event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","item_id":"item_1","output_index":0,"delta":"{\"a\":"}

"#,
                r#"event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","item_id":"item_1","output_index":0,"delta":"1}"}

"#,
                r#"event: response.function_call_arguments.done
data: {"type":"response.function_call_arguments.done","item_id":"item_1","output_index":0,"arguments":"{\"a\":1}"}

"#,
                r#"event: response.output_item.done
data: {"type":"response.output_item.done","output_index":0,"item":{"id":"item_1","type":"function_call","call_id":"call_1","name":"lookup","arguments":"{\"a\":1}"}}

"#,
                r#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_dense","model":"model-a","status":"completed","usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}}

"#,
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let mut joined_arguments = String::new();
    for value in &values {
        if let Some(tool_calls) = value
            .pointer("/choices/0/delta/tool_calls")
            .and_then(Value::as_array)
        {
            for tool_call in tool_calls {
                if let Some(arguments) = tool_call
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                {
                    joined_arguments.push_str(arguments);
                }
            }
        }
    }
    assert_eq!(
            joined_arguments, r#"{"a":1}"#,
            "dense Responses tool stream must not re-emit full arguments on done/finish; got {joined_arguments:?}; output={output}"
        );
    assert!(
        !joined_arguments.contains(r#"{"a":1}{"a":1}"#),
        "arguments must not be concatenated repeatedly"
    );
}

#[test]
fn chat_legacy_function_call_stream_converts_to_responses_tool_call() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_legacy","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_legacy","model":"model-a","choices":[{"index":0,"delta":{"function_call":{"name":"lookup_weather","arguments":"{\"city\":"}}}]}

"#,
                r#"data: {"id":"chat_legacy","model":"model-a","choices":[{"index":0,"delta":{"function_call":{"arguments":"\"Tokyo\"}"}}}]}

"#,
                r#"data: {"id":"chat_legacy","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"function_call"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let added = values
        .iter()
        .find(|value| {
            value.get("type").and_then(Value::as_str) == Some("response.output_item.added")
        })
        .expect("responses output item added");
    assert_eq!(added["item"]["type"], "function_call");
    assert_eq!(added["item"]["name"], "lookup_weather");
    let done = values
        .iter()
        .find(|value| {
            value.get("type").and_then(Value::as_str)
                == Some("response.function_call_arguments.done")
        })
        .expect("responses arguments done");
    assert_eq!(done["arguments"], "{\"city\":\"Tokyo\"}");
    assert_eq!(occurrence_count(&output, "event: response.completed"), 1);
}

#[test]
fn chat_stream_tool_calls_flush_in_index_order_despite_late_identity() {
    // CS#5310-style: index 1 arrives with full identity first; index 0 identity
    // is late. Target Responses must open tool 0 before tool 1.
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_order","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_order","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"second","arguments":"{\"b\":1}"}}]}}]}

"#,
                r#"data: {"id":"chat_order","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"first","arguments":"{\"a\":1}"}}]}}]}

"#,
                r#"data: {"id":"chat_order","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let added_names: Vec<&str> = values
        .iter()
        .filter(|value| {
            value.get("type").and_then(Value::as_str) == Some("response.output_item.added")
                && value.pointer("/item/type").and_then(Value::as_str) == Some("function_call")
        })
        .filter_map(|value| value.pointer("/item/name").and_then(Value::as_str))
        .collect();
    assert_eq!(
            added_names,
            vec!["first", "second"],
            "tools must open in ascending index order even when identity for index 0 is late; got {added_names:?}"
        );
}

#[test]
fn chat_stream_tool_call_waits_for_real_id_before_first_emit() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_late_id","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_late_id","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"type":"function","function":{"name":"lookup","arguments":"{}"}}]}}]}

"#,
                r#"data: {"id":"chat_late_id","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_real","type":"function"}]}}]}

"#,
                r#"data: {"id":"chat_late_id","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let added = values
        .iter()
        .find(|value| {
            value.get("type").and_then(Value::as_str) == Some("response.output_item.added")
        })
        .expect("responses tool item added");
    assert_eq!(added["item"]["call_id"], "call_real");
    assert_ne!(added["item"]["call_id"], "call_0");
    assert!(!output.contains("\"call_id\":\"call_0\""));
}

#[test]
fn chat_stream_anthropic_tool_start_waits_for_real_id_before_first_emit() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_late_id_anthropic","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_late_id_anthropic","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"type":"function","function":{"name":"lookup"}}]}}]}

"#,
                r#"data: {"id":"chat_late_id_anthropic","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_real","type":"function"}]}}]}

"#,
                r#"data: {"id":"chat_late_id_anthropic","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let tool_start = values
        .iter()
        .find(|value| {
            value["type"] == "content_block_start"
                && value.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use")
        })
        .expect("Anthropic tool start");
    assert_eq!(tool_start["content_block"]["id"], "call_real");
    assert_ne!(tool_start["content_block"]["id"], "call_0");
}

#[test]
fn chat_stream_tool_call_uses_synthetic_id_only_at_done_without_real_id() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_no_id","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_no_id","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"","type":"function","function":{"name":"lookup","arguments":"{}"}}]}}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let added = values
        .iter()
        .find(|value| {
            value.get("type").and_then(Value::as_str) == Some("response.output_item.added")
        })
        .expect("responses tool item added");
    assert_eq!(added["item"]["call_id"], "call_0");
    assert_eq!(added["item"]["name"], "lookup");
    assert_eq!(occurrence_count(&output, "event: response.completed"), 1);
}

#[test]
fn chat_stream_tool_call_uses_synthetic_id_only_at_eof_without_real_id() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_eof_no_id","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_eof_no_id","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"type":"function","function":{"name":"lookup","arguments":"{}"}}]}}]}

"#,
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let added = values
        .iter()
        .find(|value| {
            value.get("type").and_then(Value::as_str) == Some("response.output_item.added")
        })
        .expect("responses tool item added");
    assert_eq!(added["item"]["call_id"], "call_0");
    assert_eq!(added["item"]["name"], "lookup");
    assert_eq!(occurrence_count(&output, "event: response.completed"), 1);
}

#[test]
fn chat_stream_tool_call_item_preserves_reasoning_content() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_tool_reasoning","model":"deepseek-v4-flash","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_tool_reasoning","model":"deepseek-v4-flash","choices":[{"index":0,"delta":{"reasoning_content":"Need file."}}]}

"#,
                r#"data: {"id":"chat_tool_reasoning","model":"deepseek-v4-flash","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"README.md\"}"}}]},"finish_reason":"tool_calls"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let tool_done = values
        .iter()
        .find(|value| {
            value.get("type").and_then(Value::as_str) == Some("response.output_item.done")
                && value.pointer("/item/type").and_then(Value::as_str) == Some("function_call")
        })
        .expect("function call done");

    assert_eq!(tool_done["item"]["reasoning_content"], "Need file.");
}

#[test]
fn chat_stream_tool_call_item_preserves_late_reasoning_content() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_tool_late_reasoning","model":"deepseek-v4-flash","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_tool_late_reasoning","model":"deepseek-v4-flash","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file"}}]}}]}

"#,
                r#"data: {"id":"chat_tool_late_reasoning","model":"deepseek-v4-flash","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"README.md\"}"}}]}}]}

"#,
                r#"data: {"id":"chat_tool_late_reasoning","model":"deepseek-v4-flash","choices":[{"index":0,"delta":{"reasoning_content":"Need file."}}]}

"#,
                r#"data: {"id":"chat_tool_late_reasoning","model":"deepseek-v4-flash","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let tool_done = values
        .iter()
        .find(|value| {
            value.get("type").and_then(Value::as_str) == Some("response.output_item.done")
                && value.pointer("/item/type").and_then(Value::as_str) == Some("function_call")
        })
        .expect("function call done");

    assert_eq!(tool_done["item"]["reasoning_content"], "Need file.");
    assert!(!values.iter().any(|value| {
        value.get("type").and_then(Value::as_str) == Some("response.reasoning_summary_text.delta")
            && value.get("delta").and_then(Value::as_str) == Some("Need file.")
    }));
}

#[test]
fn chat_stream_to_responses_emits_message_lifecycle_and_completed_output() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_resp_lifecycle","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_resp_lifecycle","model":"model-a","choices":[{"index":0,"delta":{"reasoning_content":"Think"}}]}

"#,
                r#"data: {"id":"chat_resp_lifecycle","model":"model-a","choices":[{"index":0,"delta":{"content":"Hel"}}]}

"#,
                r#"data: {"id":"chat_resp_lifecycle","model":"model-a","choices":[{"index":0,"delta":{"content":"lo"}}]}

"#,
                r#"data: {"id":"chat_resp_lifecycle","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_weather","type":"function","function":{"name":"lookup_weather","arguments":"{\"city\":"}}]}}]}

"#,
                r#"data: {"id":"chat_resp_lifecycle","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"type":"function","function":{"arguments":"\"Tokyo\"}"}}]}}]}

"#,
                r#"data: {"id":"chat_resp_lifecycle","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"output_tokens":5}}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let event_position = |event_type: &str, item_type: Option<&str>| {
        values
            .iter()
            .position(|value| {
                value.get("type").and_then(Value::as_str) == Some(event_type)
                    && item_type.is_none_or(|expected_item_type| {
                        value.pointer("/item/type").and_then(Value::as_str)
                            == Some(expected_item_type)
                    })
            })
            .expect("expected event")
    };

    let reasoning_added_index = event_position("response.output_item.added", Some("reasoning"));
    let message_added_index = event_position("response.output_item.added", Some("message"));
    let content_part_added_index = event_position("response.content_part.added", None);
    let text_delta_index = event_position("response.output_text.delta", None);
    let text_done_index = event_position("response.output_text.done", None);
    let content_part_done_index = event_position("response.content_part.done", None);
    let message_done_index = values
        .iter()
        .position(|value| {
            value.get("type").and_then(Value::as_str) == Some("response.output_item.done")
                && value.pointer("/item/type").and_then(Value::as_str) == Some("message")
        })
        .expect("message done event");
    let tool_added_index = event_position("response.output_item.added", Some("function_call"));

    assert!(reasoning_added_index < message_added_index);
    assert!(message_added_index < content_part_added_index);
    assert!(content_part_added_index < text_delta_index);
    assert!(text_delta_index < text_done_index);
    assert!(text_done_index < content_part_done_index);
    assert!(content_part_done_index < message_done_index);
    assert!(message_done_index < tool_added_index);

    let reasoning_added = &values[reasoning_added_index];
    let message_added = &values[message_added_index];
    let content_part_added = &values[content_part_added_index];
    let text_delta = &values[text_delta_index];
    let tool_added = &values[tool_added_index];

    assert_eq!(reasoning_added["output_index"], 0);
    assert_eq!(message_added["output_index"], 1);
    assert_eq!(tool_added["output_index"], 2);

    let message_id = message_added["item"]["id"].as_str().unwrap();
    assert_eq!(content_part_added["item_id"], message_id);
    assert_eq!(text_delta["item_id"], message_id);
    assert_eq!(text_delta["output_index"], 1);

    let text_done = &values[text_done_index];
    assert_eq!(text_done["item_id"], message_id);
    assert_eq!(text_done["text"], "Hello");

    let content_part_done = &values[content_part_done_index];
    assert_eq!(content_part_done["item_id"], message_id);
    assert_eq!(content_part_done["part"]["text"], "Hello");

    let completed = values
        .iter()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("response.completed"))
        .expect("completed event");
    let completed_output = completed["response"]["output"].as_array().unwrap();
    assert_eq!(completed_output[0]["type"], "reasoning");
    assert_eq!(completed_output[1]["type"], "message");
    assert_eq!(completed_output[1]["id"], message_id);
    assert_eq!(completed_output[1]["content"][0]["text"], "Hello");
    assert_eq!(completed_output[2]["type"], "function_call");
    assert_eq!(completed_output[2]["call_id"], "call_weather");
    assert_eq!(completed_output[2]["arguments"], "{\"city\":\"Tokyo\"}");
    assert_eq!(occurrence_count(&output, "event: response.completed"), 1);
}

#[test]
fn chat_stream_reasoning_details_convert_to_responses_reasoning() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_reasoning_details","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_reasoning_details","model":"model-a","choices":[{"index":0,"delta":{"reasoning_details":[{"text":"Plan"},{"parts":[{"content":"Check"}]}]}}]}

"#,
                r#"data: {"id":"chat_reasoning_details","model":"model-a","choices":[{"index":0,"delta":{"content":"Done"},"finish_reason":"stop"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let reasoning_delta = values
        .iter()
        .find(|value| {
            value.get("type").and_then(Value::as_str)
                == Some("response.reasoning_summary_text.delta")
        })
        .expect("reasoning summary delta");

    assert_eq!(reasoning_delta["delta"], "Plan\n\nCheck");
    assert!(values
        .iter()
        .any(|value| { value.get("type").and_then(Value::as_str) == Some("response.completed") }));
}

#[test]
fn chat_stream_reasoning_object_converts_to_anthropic_thinking() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_reasoning_object","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_reasoning_object","model":"model-a","choices":[{"index":0,"delta":{"reasoning":{"summary":"Hidden plan"}}}]}

"#,
                r#"data: {"id":"chat_reasoning_object","model":"model-a","choices":[{"index":0,"delta":{"content":"Visible"},"finish_reason":"stop"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let thinking_delta = values
        .iter()
        .find(|value| {
            value.get("type").and_then(Value::as_str) == Some("content_block_delta")
                && value.pointer("/delta/type").and_then(Value::as_str) == Some("thinking_delta")
        })
        .expect("thinking delta");

    assert_eq!(thinking_delta["delta"]["thinking"], "Hidden plan");
    assert!(output.contains(r#""text":"Visible""#));
}

#[test]
fn chat_stream_inline_think_converts_to_responses_reasoning_without_leaking_tags() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_inline_think","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_inline_think","model":"model-a","choices":[{"index":0,"delta":{"content":"<thi"}}]}

"#,
                r#"data: {"id":"chat_inline_think","model":"model-a","choices":[{"index":0,"delta":{"content":"nk>\nNeed context.</think>\n\npong"},"finish_reason":"stop"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let reasoning_delta = values
        .iter()
        .find(|value| {
            value.get("type").and_then(Value::as_str)
                == Some("response.reasoning_summary_text.delta")
        })
        .expect("reasoning summary delta");
    let text_delta = values
        .iter()
        .find(|value| {
            value.get("type").and_then(Value::as_str) == Some("response.output_text.delta")
        })
        .expect("text delta");

    assert_eq!(reasoning_delta["delta"], "Need context.");
    assert_eq!(text_delta["delta"], "pong");
    assert!(!output.contains("<think>"));
    assert!(!output.contains("</think>"));
}

#[test]
fn chat_stream_to_responses_ignores_empty_finish_reason_from_deepseek_log() {
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        read_live_provider_stream_fixture(
            "openai_chat/deepseek-context7-empty-finish-gw-47760-1783004708389088-14.stream.jsonl",
        ),
    );
    let values = sse_data_values(&output);
    let event_positions = |event_type: &str| {
        values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                (value.get("type").and_then(Value::as_str) == Some(event_type)).then_some(index)
            })
            .collect::<Vec<_>>()
    };

    let text_delta_positions = event_positions("response.output_text.delta");
    let message_done_positions = values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            (value.get("type").and_then(Value::as_str) == Some("response.output_item.done")
                && value.pointer("/item/type").and_then(Value::as_str) == Some("message"))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    assert_eq!(message_done_positions.len(), 1);
    assert!(!text_delta_positions.is_empty());
    assert!(
        text_delta_positions
            .iter()
            .all(|position| *position < message_done_positions[0]),
        "message item must not be closed while later text delta events are still arriving"
    );

    let message_done = &values[message_done_positions[0]];
    assert_eq!(
        message_done["item"]["content"][0]["text"],
        "Context7 是 HTTP"
    );

    let tool_delta_positions = event_positions("response.function_call_arguments.delta");
    let tool_done_positions = event_positions("response.function_call_arguments.done");
    assert_eq!(tool_done_positions.len(), 1);
    assert!(!tool_delta_positions.is_empty());
    assert!(
        tool_delta_positions
            .iter()
            .all(|position| *position < tool_done_positions[0]),
        "tool call must not be closed while later argument delta events are still arriving"
    );

    let expected_arguments =
            "{\"cmd\":\"curl -H \\\"CONTEXT7_API_KEY: ctx7sk-REDACTED\\\" https://mcp.context7.com/mcp\"}";
    let tool_done = &values[tool_done_positions[0]];
    assert_eq!(tool_done["arguments"], expected_arguments);

    let completed = values
        .iter()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("response.completed"))
        .expect("completed event");
    let completed_output = completed["response"]["output"].as_array().unwrap();
    assert_eq!(completed_output.len(), 3);
    assert_eq!(completed_output[0]["type"], "reasoning");
    assert_eq!(completed_output[1]["type"], "message");
    assert_eq!(
        completed_output[1]["content"][0]["text"],
        "Context7 是 HTTP"
    );
    assert_eq!(completed_output[2]["type"], "function_call");
    assert_eq!(completed_output[2]["name"], "exec_command");
    assert_eq!(completed_output[2]["arguments"], expected_arguments);
    assert_eq!(occurrence_count(&output, "event: response.completed"), 1);
}

#[test]
fn gemini_stream_to_responses_ignores_empty_finish_reason() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiResponses),
            [
                r#"data: {"responseId":"gemini_empty_finish","modelVersion":"model-a","candidates":[{"content":{"role":"model","parts":[{"text":"Hel"}]},"finishReason":""}]}

"#,
                r#"data: {"responseId":"gemini_empty_finish","modelVersion":"model-a","candidates":[{"content":{"role":"model","parts":[{"text":"Hello"}]},"finishReason":"STOP"}],"usageMetadata":{"candidatesTokenCount":5}}

"#,
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let text_delta_positions = values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            (value.get("type").and_then(Value::as_str) == Some("response.output_text.delta"))
                .then_some(index)
        })
        .collect::<Vec<_>>();
    let message_done_position = values
        .iter()
        .position(|value| {
            value.get("type").and_then(Value::as_str) == Some("response.output_item.done")
                && value.pointer("/item/type").and_then(Value::as_str) == Some("message")
        })
        .expect("message done event");

    assert_eq!(text_delta_positions.len(), 2);
    assert!(text_delta_positions
        .iter()
        .all(|position| *position < message_done_position));

    let completed = values
        .iter()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("response.completed"))
        .expect("completed event");
    assert_eq!(
        completed["response"]["output"][0]["content"][0]["text"],
        "Hello"
    );
}

#[test]
fn anthropic_usage_only_message_delta_does_not_finish_responses_item() {
    let mut kernel = StreamKernel::new(ConversionRoute::new(
        AiProtocol::AnthropicMessages,
        AiProtocol::OpenAiResponses,
    ));

    let start = push_stream_chunk(
        &mut kernel,
        r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_usage_only","model":"claude","usage":{"input_tokens":2}}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

"#,
    );
    assert!(start.contains("response.output_text.delta"));

    let usage_only = push_stream_chunk(
        &mut kernel,
        r#"event: message_delta
data: {"type":"message_delta","delta":{},"usage":{"output_tokens":5}}

"#,
    );
    assert!(!usage_only.contains("response.output_text.done"));
    assert!(!usage_only.contains("response.output_item.done"));
    assert!(!usage_only.contains("response.completed"));

    let done = push_stream_chunk(
        &mut kernel,
        r#"event: message_stop
data: {"type":"message_stop"}

"#,
    );
    assert!(done.contains("response.output_text.done"));
    assert!(done.contains("response.completed"));
    let values = sse_data_values(&done);
    let completed = values
        .iter()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("response.completed"))
        .expect("completed event");
    assert_eq!(
        completed["response"]["output"][0]["content"][0]["text"],
        "Hello"
    );
    assert_eq!(completed["response"]["usage"]["output_tokens"], 5);
}

#[test]
fn chat_stream_to_responses_waits_for_usage_only_chunk_before_completed() {
    let mut kernel = StreamKernel::new(ConversionRoute::new(
        AiProtocol::OpenAiChat,
        AiProtocol::OpenAiResponses,
    ));

    let start = push_stream_chunk(
        &mut kernel,
        r#"data: {"id":"chat_resp_usage","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
    );
    assert!(start.contains("event: response.created"));
    assert!(start.contains("event: response.in_progress"));

    let text = push_stream_chunk(
        &mut kernel,
        r#"data: {"id":"chat_resp_usage","model":"model-a","choices":[{"index":0,"delta":{"content":"hello"}}]}

"#,
    );
    assert!(text.contains("event: response.output_item.added"));
    assert!(text.contains("event: response.content_part.added"));
    assert!(text.contains("event: response.output_text.delta"));

    let finish = push_stream_chunk(
        &mut kernel,
        r#"data: {"id":"chat_resp_usage","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

"#,
    );
    assert!(finish.contains("event: response.output_text.done"));
    assert!(finish.contains("event: response.content_part.done"));
    assert!(finish.contains("event: response.output_item.done"));
    assert!(!finish.contains("event: response.completed"));

    let completed = push_stream_chunk(
        &mut kernel,
        r#"data: {"id":"chat_resp_usage","model":"model-a","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":3,"total_tokens":13,"prompt_tokens_details":{"cached_tokens":2},"completion_tokens_details":{"reasoning_tokens":1}}}

"#,
    );
    let values = sse_data_values(&completed);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["type"], "response.completed");
    assert_eq!(values[0]["response"]["output"][0]["type"], "message");
    assert_eq!(
        values[0]["response"]["output"][0]["content"][0]["text"],
        "hello"
    );
    assert_eq!(
        values[0]["response"]["usage"],
        json!({
            "input_tokens": 10,
            "output_tokens": 3,
            "total_tokens": 13,
            "input_tokens_details": {"cached_tokens": 2},
            "output_tokens_details": {"reasoning_tokens": 1}
        })
    );
    assert!(finish_stream(&mut kernel).is_empty());
}

#[test]
fn chat_stream_to_gemini_waits_for_usage_only_chunk_before_finish() {
    let mut kernel = StreamKernel::new(ConversionRoute::new(
        AiProtocol::OpenAiChat,
        AiProtocol::GeminiNative,
    ));
    let content = push_stream_chunk(
        &mut kernel,
        "data: {\"id\":\"chat_usage\",\"model\":\"model-a\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hello\"}}]}\n\n",
    );
    assert_eq!(
        sse_data_values(&content)[0]["candidates"][0]["content"]["parts"][0]["text"],
        "hello"
    );
    let pending = push_stream_chunk(
        &mut kernel,
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"length\"}]}\n\n",
    );
    assert!(!pending.contains("finishReason"));
    let completed = push_stream_chunk(
        &mut kernel,
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3,\"total_tokens\":15,\"prompt_tokens_details\":{\"cached_tokens\":2}}}\n\n",
    );
    let values = sse_data_values(&completed);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["candidates"][0]["finishReason"], "MAX_TOKENS");
    assert_eq!(values[0]["usageMetadata"]["promptTokenCount"], 12);
    assert_eq!(values[0]["usageMetadata"]["candidatesTokenCount"], 3);
    assert_eq!(values[0]["usageMetadata"]["totalTokenCount"], 15);
    assert_eq!(values[0]["usageMetadata"]["cachedContentTokenCount"], 2);
    assert!(push_stream_chunk(&mut kernel, "data: [DONE]\n\n").is_empty());
    assert!(finish_stream(&mut kernel).is_empty());
}

#[test]
fn chat_stream_to_gemini_without_usage_preserves_eof_reason_or_late_error() {
    for failed in [false, true] {
        let mut kernel = StreamKernel::new(ConversionRoute::new(
            AiProtocol::OpenAiChat,
            AiProtocol::GeminiNative,
        ));
        let pending = push_stream_chunk(
            &mut kernel,
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hello\"},\"finish_reason\":\"length\"}]}\n\n",
        );
        assert!(!pending.contains("finishReason"));
        let terminal = if failed {
            push_stream_chunk(
                &mut kernel,
                "data: {\"error\":{\"message\":\"upstream failed\",\"code\":\"500\"}}\n\n",
            )
        } else {
            finish_stream(&mut kernel)
        };
        let values = sse_data_values(&terminal);
        assert_eq!(values.len(), 1);
        if failed {
            assert_eq!(values[0]["error"]["code"], 500);
            assert!(!terminal.contains("finishReason"));
        } else {
            assert_eq!(values[0]["candidates"][0]["finishReason"], "MAX_TOKENS");
        }
        assert!(finish_stream(&mut kernel).is_empty());
    }
}

#[test]
fn chat_stream_usage_only_chunk_synthesizes_zero_reasoning_tokens() {
    let mut kernel = StreamKernel::new(ConversionRoute::new(
        AiProtocol::OpenAiChat,
        AiProtocol::OpenAiResponses,
    ));

    let _ = push_stream_chunk(
        &mut kernel,
        r#"data: {"id":"chat_resp_usage_zero","model":"model-a","choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":"stop"}]}

"#,
    );
    let completed = push_stream_chunk(
        &mut kernel,
        r#"data: {"id":"chat_resp_usage_zero","model":"model-a","choices":[],"usage":{"prompt_tokens":4,"completion_tokens":6,"total_tokens":10}}

"#,
    );
    let values = sse_data_values(&completed);

    assert_eq!(values.len(), 1);
    assert_eq!(
        values[0]["response"]["usage"]["output_tokens_details"]["reasoning_tokens"],
        0
    );
}

#[test]
fn chat_stream_to_responses_ignores_zero_usage_on_delta_chunks() {
    let mut kernel = StreamKernel::new(ConversionRoute::new(
        AiProtocol::OpenAiChat,
        AiProtocol::OpenAiResponses,
    ));
    let zero_usage = r#""usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"prompt_tokens_details":{"audio_tokens":0,"cached_tokens":0},"completion_tokens_details":{"audio_tokens":0,"reasoning_tokens":0,"accepted_prediction_tokens":0,"rejected_prediction_tokens":0}}"#;

    let _ = push_stream_chunk(
        &mut kernel,
        &format!(
            r#"data: {{"id":"chat_resp_zero_delta","model":"deepseek-v4-flash","choices":[{{"index":0,"delta":{{"role":"assistant","reasoning_content":"Now","reasoning":"Now"}},"finish_reason":null}}],{zero_usage}}}

"#,
        ),
    );
    let _ = push_stream_chunk(
        &mut kernel,
        &format!(
            r#"data: {{"id":"chat_resp_zero_delta","model":"deepseek-v4-flash","choices":[{{"index":0,"delta":{{"content":"hello","reasoning_content":""}},"finish_reason":null}}],{zero_usage}}}

"#,
        ),
    );
    let finish = push_stream_chunk(
        &mut kernel,
        &format!(
            r#"data: {{"id":"chat_resp_zero_delta","model":"deepseek-v4-flash","choices":[{{"index":0,"delta":{{}},"finish_reason":"stop"}}],{zero_usage}}}

"#,
        ),
    );
    assert!(!finish.contains("event: response.completed"));

    let completed = finish_stream(&mut kernel);
    let values = sse_data_values(&completed);
    let completed = values
        .iter()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("response.completed"))
        .expect("stream end should synthesize response.completed");

    assert!(completed["response"].get("usage").is_none());
    let completed_output = completed["response"]["output"].as_array().unwrap();
    let message_item = completed_output
        .iter()
        .find(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .expect("completed output should include the assistant message");
    assert_eq!(message_item["content"][0]["text"], "hello");
}

#[test]
fn chat_stream_to_responses_transport_error_before_start_emits_error_event() {
    let (output, errors) = collect_stream_chunks(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        vec![Err("upstream boom".to_string())],
    );

    assert!(errors.is_empty());
    assert!(output.contains("event: error"));
    assert!(!output.contains("event: response.completed"));
    let values = sse_data_values(&output);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["type"], "error");
    assert_eq!(values[0]["code"], "stream_error");
    assert_eq!(values[0]["message"], "upstream boom");
}

#[test]
fn chat_stream_to_responses_transport_error_after_start_emits_response_failed() {
    let (output, errors) = collect_stream_chunks(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            vec![
                Ok(r#"data: {"id":"chat_resp_error","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#
                .as_bytes()
                .to_vec()),
                Ok(r#"data: {"id":"chat_resp_error","model":"model-a","choices":[{"index":0,"delta":{"content":"hello"}}]}

"#
                .as_bytes()
                .to_vec()),
                Err("upstream boom".to_string()),
            ],
        );

    assert!(errors.is_empty());
    assert!(output.contains("event: response.created"));
    assert!(output.contains("event: response.output_text.delta"));
    assert!(output.contains("event: response.failed"));
    assert!(!output.contains("event: response.completed"));
    let values = sse_data_values(&output);
    let failed = values
        .iter()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("response.failed"))
        .expect("response.failed event");
    assert_eq!(failed["response"]["status"], "failed");
    assert_eq!(failed["response"]["error"]["type"], "server_error");
    assert_eq!(failed["response"]["error"]["code"], "stream_error");
    assert_eq!(failed["response"]["error"]["message"], "upstream boom");
    assert_eq!(failed["response"]["output"][0]["type"], "message");
    assert_eq!(failed["response"]["output"][0]["status"], "in_progress");
    assert_eq!(
        failed["response"]["output"][0]["content"][0]["text"],
        "hello"
    );
    assert_eq!(occurrence_count(&output, "event: response.failed"), 1);
}

#[test]
fn chat_stream_to_responses_openai_error_chunk_emits_error_event() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            r#"data: {"error":{"message":"rate limited","type":"rate_limit_error","code":"rate_limit_exceeded"}}

"#
            .to_string(),
        );

    assert!(output.contains("event: error"));
    assert!(!output.contains("event: response.completed"));
    let values = sse_data_values(&output);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["type"], "error");
    assert_eq!(values[0]["code"], "rate_limit_exceeded");
    assert_eq!(values[0]["message"], "rate limited");
}

#[test]
fn chat_stream_openai_error_numeric_code_is_stringified() {
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        r#"data: {"error":{"message":"rate limited","type":"rate_limit_error","code":429}}

"#
        .to_string(),
    );

    assert!(output.contains("event: error"));
    let values = sse_data_values(&output);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["type"], "error");
    assert_eq!(values[0]["code"], "429");
    assert_eq!(values[0]["message"], "rate limited");
}

#[test]
fn chat_stream_openai_error_event_empty_payload_emits_error() {
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        "event: error\n\n".to_string(),
    );

    assert!(output.contains("event: error"));
    let values = sse_data_values(&output);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["error"]["type"], "stream_error");
    assert_eq!(values[0]["error"]["message"], "stream error");
}

#[test]
fn chat_stream_openai_wrapped_error_event_emits_error() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            r#"event: error
data: {"event":"error","data":{"error":{"message":"bad request","type":"invalid_request_error","code":400}}}

"#
            .to_string(),
        );

    assert!(output.contains("event: error"));
    let values = sse_data_values(&output);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["type"], "error");
    assert_eq!(values[0]["code"], "400");
    assert_eq!(values[0]["message"], "bad request");
}

#[test]
fn chat_stream_openai_string_error_payload_emits_error() {
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        r#"data: {"error":"upstream closed"}

"#
        .to_string(),
    );

    assert!(output.contains("event: error"));
    let values = sse_data_values(&output);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["type"], "error");
    assert_eq!(values[0]["code"], "stream_error");
    assert_eq!(values[0]["message"], "upstream closed");
}

#[test]
fn chat_stream_to_anthropic_openai_error_chunk_emits_error_event() {
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        r#"data: {"error":{"message":"rate limited","type":"rate_limit_error"}}

"#
        .to_string(),
    );

    assert!(output.contains("event: error"));
    assert!(!output.contains("event: message_stop"));
    let values = sse_data_values(&output);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["type"], "error");
    assert_eq!(values[0]["error"]["type"], "rate_limit_error");
    assert_eq!(values[0]["error"]["message"], "rate limited");
}

#[test]
fn chat_stream_to_anthropic_transport_error_emits_error_event() {
    let (output, errors) = collect_stream_chunks(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        vec![Err("upstream boom".to_string())],
    );

    assert!(errors.is_empty());
    assert!(output.contains("event: error"));
    assert!(!output.contains("event: message_stop"));
    let values = sse_data_values(&output);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["type"], "error");
    assert_eq!(values[0]["error"]["type"], "stream_error");
    assert_eq!(values[0]["error"]["message"], "upstream boom");
}

#[test]
fn chat_stream_to_gemini_openai_error_chunk_emits_gemini_error_event() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
            r#"data: {"error":{"message":"rate limited","type":"rate_limit_error","code":"rate_limit_exceeded"}}

"#
            .to_string(),
        );

    assert!(output.contains(r#""error""#));
    assert!(!output.contains("finishReason"));
    let values = sse_data_values(&output);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["error"]["message"], "rate limited");
    assert_eq!(values[0]["error"]["code"], 429);
    assert_eq!(values[0]["error"]["status"], "RESOURCE_EXHAUSTED");
}

#[test]
fn chat_stream_to_gemini_transport_error_emits_gemini_error_event() {
    let (output, errors) = collect_stream_chunks(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        vec![Err("upstream boom".to_string())],
    );

    assert!(errors.is_empty());
    assert!(output.contains(r#""error""#));
    assert!(!output.contains("finishReason"));
    let values = sse_data_values(&output);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["error"]["message"], "upstream boom");
    assert_eq!(values[0]["error"]["code"], 500);
    assert_eq!(values[0]["error"]["status"], "INTERNAL");
}

#[test]
fn anthropic_stream_error_to_chat_emits_openai_error_chunk() {
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        r#"event: error
data: {"type":"error","error":{"type":"overloaded_error","message":"try later"}}

"#
        .to_string(),
    );

    assert!(output.contains(r#""error""#));
    assert!(!output.contains("[DONE]"));
    let values = sse_data_values(&output);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["error"]["message"], "try later");
    assert_eq!(values[0]["error"]["type"], "overloaded_error");
    assert_eq!(values[0]["error"]["code"], "overloaded_error");
}

#[test]
fn anthropic_stream_ping_and_content_block_stop_are_filtered_for_chat_target() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
            r#"event: ping
data: {"type":"ping"}

event: message_start
data: {"type":"message_start","message":{"id":"msg_filter","model":"claude","usage":{"input_tokens":2}}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: ping
data: {"type":"ping"}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}

event: message_stop
data: {"type":"message_stop"}

"#
            .to_string(),
        );

    assert!(!output.contains(r#""type":"ping""#));
    assert!(!output.contains("content_block_stop"));
    assert!(output.contains("data: [DONE]"));
    let values = sse_data_values(&output);
    // role start + text delta + usage-only chunk + finish chunk
    assert_eq!(values.len(), 4);
    assert_eq!(values[0]["choices"][0]["delta"]["role"], "assistant");
    assert_eq!(values[1]["choices"][0]["delta"]["content"], "Hello");
    assert!(values[2]["choices"]
        .as_array()
        .is_some_and(|choices| choices.is_empty()));
    assert_eq!(values[2]["usage"]["prompt_tokens"], 2);
    assert_eq!(values[2]["usage"]["completion_tokens"], 1);
    assert_eq!(values[3]["choices"][0]["finish_reason"], "stop");
}

#[test]
fn anthropic_stream_kernel_emits_provider_local_server_tool_blocks_for_anthropic_target() {
    let mut kernel = StreamKernel::new(ConversionRoute::new(
        AiProtocol::AnthropicMessages,
        AiProtocol::AnthropicMessages,
    ));
    let output = push_stream_chunk(
        &mut kernel,
        r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_server_tool","model":"claude","usage":{"input_tokens":2}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srv_1","name":"web_search","input":{"query":"rust"}}}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"web_search_tool_result","tool_use_id":"srv_1","content":[{"type":"web_search_result","title":"Rust"}]}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}

event: message_stop
data: {"type":"message_stop"}

"#,
    );
    let values = sse_data_values(&output);

    assert!(values.iter().any(|value| {
        value["type"] == "content_block_start"
            && value["content_block"]["type"] == "server_tool_use"
            && value["content_block"]["input"]["query"] == "rust"
    }));
    assert!(values.iter().any(|value| {
        value["type"] == "content_block_start"
            && value["content_block"]["type"] == "web_search_tool_result"
            && value["content_block"]["content"][0]["title"] == "Rust"
    }));
    assert_eq!(occurrence_count(&output, "event: content_block_stop"), 2);
}

#[test]
fn chat_stream_to_gemini_waits_for_complete_tool_arguments() {
    let mut kernel = StreamKernel::new(ConversionRoute::new(
        AiProtocol::OpenAiChat,
        AiProtocol::GeminiNative,
    ));

    let first = push_stream_chunk(
        &mut kernel,
        r#"data: {"id":"chat_gemini_tool","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

data: {"id":"chat_gemini_tool","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{\"query\""}}]}}]}

"#,
    );
    assert!(!first.contains("functionCall"));

    let second = push_stream_chunk(
        &mut kernel,
        r#"data: {"id":"chat_gemini_tool","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"type":"function","function":{"arguments":":\"rust\"}"}}]}}]}

"#,
    );
    assert!(second.contains("functionCall"));
    let values = sse_data_values(&second);
    assert_eq!(
        values[0]["candidates"][0]["content"]["parts"][0]["functionCall"]["args"]["query"],
        "rust"
    );
    assert!(finish_stream(&mut kernel).contains(r#""finishReason":"STOP""#));
}

#[test]
fn gemini_usage_only_stream_chunk_finishes_chat_target() {
    let mut kernel = StreamKernel::new(ConversionRoute::new(
        AiProtocol::GeminiNative,
        AiProtocol::OpenAiChat,
    ));

    let output = push_stream_chunk(
        &mut kernel,
        r#"data: {"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":0,"totalTokenCount":3}}

"#,
    );

    assert!(
        output.contains(r#""finish_reason":"stop""#),
        "output={output}"
    );
    assert!(output.contains("[DONE]"), "output={output}");
    let values = sse_data_values(&output);
    let usage_chunk = values
        .iter()
        .find(|value| value.get("usage").is_some())
        .expect("chat usage-only chunk");
    assert_eq!(usage_chunk["usage"]["prompt_tokens"], 3);
    assert_eq!(usage_chunk["usage"]["completion_tokens"], 0);
    assert_eq!(usage_chunk["usage"]["total_tokens"], 3);
}

#[test]
fn anthropic_message_start_usage_merges_into_chat_usage() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
            r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_usage","model":"claude","usage":{"input_tokens":17,"cache_creation_input_tokens":3,"cache_read_input_tokens":2}}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}

event: message_stop
data: {"type":"message_stop"}

"#
            .to_string(),
        );
    let values = sse_data_values(&output);
    let usage_chunk = values
        .iter()
        .find(|value| value.get("usage").is_some())
        .expect("chat usage-only chunk from anthropic source");
    // input(17) + cache_read(2) + cache_write(3) = 22 prompt tokens
    assert_eq!(usage_chunk["usage"]["prompt_tokens"], 22);
    assert_eq!(usage_chunk["usage"]["completion_tokens"], 5);
    assert_eq!(
        usage_chunk["usage"]["prompt_tokens_details"]["cached_tokens"],
        2
    );
    assert_eq!(
        usage_chunk["usage"]["prompt_tokens_details"]["cache_write_tokens"],
        3
    );
}

#[test]
fn gemini_function_call_finish_maps_to_tool_calls_for_chat() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
            r#"data: {"responseId":"g1","modelVersion":"m","candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"lookup","args":{"q":"1"}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":2,"totalTokenCount":11}}

"#
            .to_string(),
        );
    let values = sse_data_values(&output);
    assert!(
        values.iter().any(|value| {
            value
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
                == Some("tool_calls")
        }),
        "Gemini functionCall + STOP must finish as tool_calls; output={output}"
    );
    let usage_chunk = values
        .iter()
        .find(|value| value.get("usage").is_some())
        .expect("usage chunk");
    assert_eq!(usage_chunk["usage"]["prompt_tokens"], 9);
    assert_eq!(usage_chunk["usage"]["completion_tokens"], 2);
}

#[test]
fn gemini_stream_preserves_identical_deltas_and_accepts_growing_snapshots() {
    for (pieces, expected) in [
        (vec!["_", "_", "AITB", "_", "_"], "__AITB__"),
        (vec!["ha", "ha", "!"], "haha!"),
        (vec!["H", "He", "Hello"], "Hello"),
    ] {
        let mut wire = String::new();
        for piece in pieces {
            wire.push_str(&format!("data: {}\n\n", json!({"responseId":"gemini-repeat","candidates":[{"content":{"parts":[{"text":piece},{"text":piece,"thought":true}]}}]})));
        }
        wire.push_str("data: {\"candidates\":[{\"finishReason\":\"STOP\"}]}\n\n");
        let output = collect_stream(
            ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
            wire,
        );
        let values = sse_data_values(&output);
        for pointer in [
            "/choices/0/delta/content",
            "/choices/0/delta/reasoning_content",
        ] {
            let text = values
                .iter()
                .filter_map(|value| value.pointer(pointer).and_then(Value::as_str))
                .collect::<String>();
            assert_eq!(text, expected, "{pointer}");
        }
    }
}

#[test]
fn gemini_stream_empty_response_id_emits_invalid_error() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
            r#"data: {"responseId":"","modelVersion":"gemini-2.5-flash","candidates":[{"content":{"role":"model","parts":[{"text":"hello"}]}}]}

"#
            .to_string(),
        );
    let values = sse_data_values(&output);

    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["error"]["type"], "invalid_response");
    assert_eq!(
        values[0]["error"]["message"],
        "Gemini stream responseId is empty"
    );
    assert!(!output.contains("[DONE]"));
}

#[test]
fn chat_stream_to_gemini_flushes_empty_tool_arguments_on_tool_calls_finish() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
            [
                r#"data: {"id":"chat_gemini_empty_tool","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_gemini_empty_tool","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup"}}]}}]}

"#,
                r#"data: {"id":"chat_gemini_empty_tool","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let tool_event = values
        .iter()
        .find(|value| value.to_string().contains("functionCall"))
        .expect("functionCall event");
    assert_eq!(
        tool_event["candidates"][0]["content"]["parts"][0]["functionCall"]["args"],
        json!({})
    );
    assert_eq!(occurrence_count(&output, "finishReason"), 1);
}

#[test]
fn chat_stream_to_anthropic_tool_use_start_includes_empty_input() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_tool","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_tool","model":"model-a","choices":[{"index":0,"delta":{"content":"I'll read it."}}]}

"#,
                r#"data: {"id":"chat_tool","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.txt\"}"}}]}}]}

"#,
                r#"data: {"id":"chat_tool","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"completion_tokens":3}}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );

    let values = sse_data_values(&output);
    let message_start = values
        .iter()
        .find(|value| value["type"] == "message_start")
        .expect("message start");
    let text_start = values
        .iter()
        .position(|value| {
            value["type"] == "content_block_start"
                && value.pointer("/content_block/type").and_then(Value::as_str) == Some("text")
        })
        .expect("text start");
    let text_stop = values
        .iter()
        .position(|value| value["type"] == "content_block_stop" && value["index"] == 0)
        .expect("text stop");
    let tool_start = values
        .iter()
        .position(|value| {
            value["type"] == "content_block_start"
                && value.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use")
        })
        .expect("tool start");
    let tool_delta = values
        .iter()
        .find(|value| {
            value.pointer("/delta/type").and_then(Value::as_str) == Some("input_json_delta")
        })
        .expect("tool arguments delta");
    let message_delta = values
        .iter()
        .find(|value| value["type"] == "message_delta")
        .expect("message delta");

    assert_eq!(
        message_start["message"]["usage"],
        json!({
            "input_tokens": 1,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "output_tokens": 1
        })
    );
    assert!(text_start < text_stop);
    assert!(text_stop < tool_start);
    assert_eq!(
        values[tool_start]["content_block"],
        json!({"type": "tool_use", "id": "call_1", "name": "read_file", "input": {}})
    );
    assert_eq!(
        tool_delta
            .pointer("/delta/partial_json")
            .and_then(Value::as_str),
        Some("{\"path\":\"a.txt\"}")
    );
    assert_eq!(
        message_delta["usage"],
        json!({
            "input_tokens": 0,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "output_tokens": 3
        })
    );
    assert_eq!(occurrence_count(&output, "event: message_stop"), 1);
}

#[test]
fn chat_stream_to_anthropic_keeps_tool_finish_when_upstream_sends_stop() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_tool_stop","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_tool_stop","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.txt\"}"}}]}}]}

"#,
                r#"data: {"id":"chat_tool_stop","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let message_delta = values
        .iter()
        .find(|value| value["type"] == "message_delta")
        .expect("message delta");

    assert_eq!(message_delta["delta"]["stop_reason"], "tool_use");
    assert_eq!(occurrence_count(&output, "event: message_stop"), 1);
}

#[test]
fn chat_stream_to_anthropic_waits_for_usage_only_chunk_before_message_stop() {
    let mut kernel = StreamKernel::new(ConversionRoute::new(
        AiProtocol::OpenAiChat,
        AiProtocol::AnthropicMessages,
    ));

    let start = push_stream_chunk(
        &mut kernel,
        r#"data: {"id":"chat_usage","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
    );
    assert!(start.contains("event: message_start"));

    let text = push_stream_chunk(
        &mut kernel,
        r#"data: {"id":"chat_usage","model":"model-a","choices":[{"index":0,"delta":{"content":"hello"}}]}

"#,
    );
    assert!(text.contains("event: content_block_start"));
    assert!(text.contains("event: content_block_delta"));

    let finish = push_stream_chunk(
        &mut kernel,
        r#"data: {"id":"chat_usage","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

"#,
    );
    assert!(finish.contains("event: content_block_stop"));
    assert!(!finish.contains("event: message_delta"));
    assert!(!finish.contains("event: message_stop"));

    let usage = push_stream_chunk(
        &mut kernel,
        r#"data: {"id":"chat_usage","model":"model-a","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":3,"total_tokens":13,"prompt_tokens_details":{"cached_tokens":2}}}

"#,
    );
    let values = sse_data_values(&usage);
    assert_eq!(values.len(), 2);
    assert_eq!(values[0]["type"], "message_delta");
    assert_eq!(values[0]["delta"]["stop_reason"], "end_turn");
    assert_eq!(
        values[0]["usage"],
        json!({
            "input_tokens": 8,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 2,
            "output_tokens": 3
        })
    );
    assert_eq!(values[1]["type"], "message_stop");
    assert!(finish_stream(&mut kernel).is_empty());
}

#[test]
fn chat_stream_to_anthropic_closes_previous_tool_before_next_tool() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_parallel_tools","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_parallel_tools","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_city","type":"function","function":{"name":"get_user_city","arguments":"{\"user_id\":\"123\"}"}}]}}]}

"#,
                r#"data: {"id":"chat_parallel_tools","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_language","type":"function","function":{"name":"get_user_language","arguments":"{\"user_id\":\"123\"}"}}]}}]}

"#,
                r#"data: {"id":"chat_parallel_tools","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

"#,
                r#"data: {"id":"chat_parallel_tools","model":"model-a","choices":[],"usage":{"prompt_tokens":1,"completion_tokens":49,"total_tokens":50}}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let first_tool_start = values
        .iter()
        .position(|value| {
            value["type"] == "content_block_start"
                && value["index"] == 0
                && value.pointer("/content_block/id").and_then(Value::as_str) == Some("call_city")
        })
        .expect("first tool start");
    let first_tool_stop = values
        .iter()
        .position(|value| value["type"] == "content_block_stop" && value["index"] == 0)
        .expect("first tool stop");
    let second_tool_start = values
        .iter()
        .position(|value| {
            value["type"] == "content_block_start"
                && value["index"] == 1
                && value.pointer("/content_block/id").and_then(Value::as_str)
                    == Some("call_language")
        })
        .expect("second tool start");
    let second_tool_stop = values
        .iter()
        .position(|value| value["type"] == "content_block_stop" && value["index"] == 1)
        .expect("second tool stop");
    let message_delta = values
        .iter()
        .position(|value| value["type"] == "message_delta")
        .expect("message delta");

    assert!(first_tool_start < first_tool_stop);
    assert!(first_tool_stop < second_tool_start);
    assert!(second_tool_start < second_tool_stop);
    assert!(second_tool_stop < message_delta);
    assert_eq!(occurrence_count(&output, "event: content_block_start"), 2);
    assert_eq!(occurrence_count(&output, "event: content_block_stop"), 2);
    assert_eq!(occurrence_count(&output, "event: message_stop"), 1);
}

#[test]
fn anthropic_sse_defers_signature_until_thinking_block_close() {
    let signature = anthropic_signature_fixture("stream-signature");
    let signature_chunk = [
            r#"data: {"id":"chat_sig","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":""#,
            signature.as_str(),
            r#""}}]}

"#,
        ]
        .concat();
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_sig","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                signature_chunk.as_str(),
                r#"data: {"id":"chat_sig","model":"model-a","choices":[{"index":0,"delta":{"reasoning_content":"Think"}}]}

"#,
                r#"data: {"id":"chat_sig","model":"model-a","choices":[{"index":0,"delta":{"content":"Answer"}}]}

"#,
                r#"data: {"id":"chat_sig","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );

    let values = sse_data_values(&output);
    let signature_index = values
        .iter()
        .position(|value| {
            value.pointer("/delta/type").and_then(Value::as_str) == Some("signature_delta")
        })
        .expect("signature delta");
    let thinking_stop_index = values
        .iter()
        .position(|value| value["type"] == "content_block_stop" && value["index"] == 0)
        .expect("thinking stop");
    let text_start_index = values
        .iter()
        .position(|value| {
            value["type"] == "content_block_start"
                && value.pointer("/content_block/type").and_then(Value::as_str) == Some("text")
        })
        .expect("text start");

    assert_eq!(
        values[signature_index]
            .pointer("/delta/signature")
            .and_then(Value::as_str),
        Some(signature.as_str())
    );
    assert!(signature_index < thinking_stop_index);
    assert!(thinking_stop_index < text_start_index);
}

#[test]
fn anthropic_sse_signature_only_creates_synthetic_thinking_block() {
    let signature = anthropic_signature_fixture("signature-only");
    let signature_chunk = [
            r#"data: {"id":"chat_sig_only","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":""#,
            signature.as_str(),
            r#""}}]}

"#,
        ]
        .concat();
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_sig_only","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                signature_chunk.as_str(),
                r#"data: {"id":"chat_sig_only","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let thinking_start = values
        .iter()
        .position(|value| {
            value["type"] == "content_block_start"
                && value.pointer("/content_block/type").and_then(Value::as_str) == Some("thinking")
        })
        .expect("synthetic thinking start");
    let signature_delta = values
        .iter()
        .position(|value| {
            value.pointer("/delta/type").and_then(Value::as_str) == Some("signature_delta")
        })
        .expect("signature delta");
    let thinking_stop = values
        .iter()
        .position(|value| value["type"] == "content_block_stop")
        .expect("thinking stop");

    assert!(thinking_start < signature_delta);
    assert!(signature_delta < thinking_stop);
    assert_eq!(
        values[signature_delta]
            .pointer("/delta/signature")
            .and_then(Value::as_str),
        Some(signature.as_str())
    );
}

#[test]
fn anthropic_sse_signature_before_text_creates_synthetic_thinking_before_text() {
    let signature = anthropic_signature_fixture("signature-before-text");
    let signature_chunk = [
            r#"data: {"id":"chat_sig_text","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":""#,
            signature.as_str(),
            r#""}}]}

"#,
        ]
        .concat();
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_sig_text","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                signature_chunk.as_str(),
                r#"data: {"id":"chat_sig_text","model":"model-a","choices":[{"index":0,"delta":{"content":"Answer"}}]}

"#,
                r#"data: {"id":"chat_sig_text","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );

    let values = sse_data_values(&output);
    let signature_delta = values
        .iter()
        .position(|value| {
            value.pointer("/delta/type").and_then(Value::as_str) == Some("signature_delta")
        })
        .expect("signature delta");
    let thinking_stop = values
        .iter()
        .position(|value| value["type"] == "content_block_stop" && value["index"] == 0)
        .expect("thinking stop");
    let text_start = values
        .iter()
        .position(|value| {
            value["type"] == "content_block_start"
                && value.pointer("/content_block/type").and_then(Value::as_str) == Some("text")
        })
        .expect("text start");

    assert_eq!(
        values[signature_delta]
            .pointer("/delta/signature")
            .and_then(Value::as_str),
        Some(signature.as_str())
    );
    assert!(signature_delta < thinking_stop);
    assert!(thinking_stop < text_start);
}

#[test]
fn anthropic_sse_signature_before_tool_creates_synthetic_thinking_before_tool() {
    let signature = anthropic_signature_fixture("signature-before-tool");
    let signature_chunk = [
            r#"data: {"id":"chat_sig_tool","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":""#,
            signature.as_str(),
            r#""}}]}

"#,
        ]
        .concat();
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_sig_tool","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                signature_chunk.as_str(),
                r#"data: {"id":"chat_sig_tool","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}]}}]}

"#,
                r#"data: {"id":"chat_sig_tool","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );

    let values = sse_data_values(&output);
    let signature_delta = values
        .iter()
        .position(|value| {
            value.pointer("/delta/type").and_then(Value::as_str) == Some("signature_delta")
        })
        .expect("signature delta");
    let thinking_stop = values
        .iter()
        .position(|value| value["type"] == "content_block_stop" && value["index"] == 0)
        .expect("thinking stop");
    let tool_start = values
        .iter()
        .position(|value| {
            value["type"] == "content_block_start"
                && value.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use")
        })
        .expect("tool start");

    assert_eq!(
        values[signature_delta]
            .pointer("/delta/signature")
            .and_then(Value::as_str),
        Some(signature.as_str())
    );
    assert!(signature_delta < thinking_stop);
    assert!(thinking_stop < tool_start);
}

#[test]
fn anthropic_sse_signature_after_thinking_flushes_on_finish() {
    let signature = anthropic_signature_fixture("signature-finish");
    let signature_chunk = [
            r#"data: {"id":"chat_sig_finish","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":""#,
            signature.as_str(),
            r#""}}]}

"#,
        ]
        .concat();
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_sig_finish","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_sig_finish","model":"model-a","choices":[{"index":0,"delta":{"reasoning_content":"Think"}}]}

"#,
                signature_chunk.as_str(),
                r#"data: {"id":"chat_sig_finish","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );

    let values = sse_data_values(&output);
    let thinking_delta = values
        .iter()
        .position(|value| {
            value.pointer("/delta/type").and_then(Value::as_str) == Some("thinking_delta")
        })
        .expect("thinking delta");
    let signature_delta = values
        .iter()
        .position(|value| {
            value.pointer("/delta/type").and_then(Value::as_str) == Some("signature_delta")
        })
        .expect("signature delta");
    let thinking_stop = values
        .iter()
        .position(|value| value["type"] == "content_block_stop")
        .expect("thinking stop");

    assert!(thinking_delta < signature_delta);
    assert!(signature_delta < thinking_stop);
}

#[test]
fn chat_sse_target_skips_pure_signature_events() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
            [
                r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_sig","model":"model-a"}}

"#,
                r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQanthropic-stream"}}

"#,
                r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":0}}

"#,
                r#"event: message_stop
data: {"type":"message_stop"}

"#,
            ]
            .concat(),
        );

    assert!(!output.contains("reasoning_signature"));
    assert!(!output.contains("EqQanthropic-stream"));
    assert_eq!(occurrence_count(&output, "data: [DONE]"), 1);
}

#[test]
fn responses_sse_target_preserves_encrypted_only_reasoning() {
    let encrypted_content = openai_responses_heuristic_signature();
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_enc","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#
                .to_string(),
                format!(
                    r#"data: {{"id":"chat_enc","model":"model-a","choices":[{{"index":0,"delta":{{"reasoning_signature":"{encrypted_content}"}}}}]}}

"#,
                ),
                r#"data: {"id":"chat_enc","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

"#
                .to_string(),
                "data: [DONE]\n\n".to_string(),
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let done = values
        .iter()
        .find(|value| value["type"] == "response.output_item.done")
        .expect("reasoning done");
    assert_eq!(done["item"]["type"], "reasoning");
    assert_eq!(done["item"]["summary"], json!([]));
    assert_eq!(
        done["item"]["encrypted_content"],
        encrypted_content.as_str()
    );
}

#[test]
fn responses_sse_target_preserves_marked_encrypted_only_reasoning() {
    let marked_signature =
        format!("ai-toolbox.sig.openai_responses:{OPENAI_RESPONSES_FIXTURE_ENCRYPTED_CONTENT}");
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_marked_enc","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#
                .to_string(),
                format!(
                    r#"data: {{"id":"chat_marked_enc","model":"model-a","choices":[{{"index":0,"delta":{{"reasoning_signature":"{marked_signature}"}}}}]}}

"#,
                ),
                r#"data: {"id":"chat_marked_enc","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

"#
                .to_string(),
                "data: [DONE]\n\n".to_string(),
            ]
            .concat(),
        );
    let values = sse_data_values(&output);
    let done = values
        .iter()
        .find(|value| value["type"] == "response.output_item.done")
        .expect("reasoning done");
    assert_eq!(done["item"]["type"], "reasoning");
    assert_eq!(done["item"]["summary"], json!([]));
    assert_eq!(
        done["item"]["encrypted_content"],
        OPENAI_RESPONSES_FIXTURE_ENCRYPTED_CONTENT
    );
}

#[test]
fn responses_sse_encrypted_content_does_not_leak_to_other_targets() {
    let encrypted_content = OPENAI_RESPONSES_FIXTURE_ENCRYPTED_CONTENT;
    let input = [
            r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_enc","model":"model-a","output":[]}}

"#
            .to_string(),
            r#"event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[]}}

"#
            .to_string(),
            format!(
                r#"event: response.output_item.done
data: {{"type":"response.output_item.done","output_index":0,"item":{{"id":"rs_1","type":"reasoning","summary":[],"encrypted_content":"{encrypted_content}"}}}}

"#,
            ),
            r#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_enc","model":"model-a","status":"completed","output":[]}}

"#
            .to_string(),
        ]
        .concat();

    let anthropic = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::AnthropicMessages),
        input.clone(),
    );
    assert!(!anthropic.contains("signature_delta"));
    assert!(!anthropic.contains(encrypted_content));

    let gemini = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::GeminiNative),
        input,
    );
    assert!(!gemini.contains("thoughtSignature"));
    assert!(!gemini.contains(encrypted_content));
}

#[test]
fn gemini_sse_target_restores_reasoning_and_tool_signatures_from_matching_marker() {
    let reasoning_output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
            [
                r#"data: {"id":"chat_gemini_sig","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_gemini_sig","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":"CgR0ZXN0"}}]}

"#,
                r#"data: {"id":"chat_gemini_sig","model":"model-a","choices":[{"index":0,"delta":{"reasoning_content":"Think"}}]}

"#,
                r#"data: {"id":"chat_gemini_sig","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    assert!(reasoning_output.contains(r#""thoughtSignature":"CgR0ZXN0""#));

    let tool_output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
            [
                r#"data: {"id":"chat_gemini_tool","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_gemini_tool","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":"CgR0ZXN0"}}]}

"#,
                r#"data: {"id":"chat_gemini_tool","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}]}}]}

"#,
                r#"data: {"id":"chat_gemini_tool","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );
    assert!(tool_output.contains(r#""functionCall""#));
    assert!(tool_output.contains(r#""thoughtSignature":"CgR0ZXN0""#));
}

#[test]
fn gemini_sse_target_skips_finish_only_signature_without_reasoning_or_tool() {
    let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
            [
                r#"data: {"id":"chat_gemini_finish_sig","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant","content":"OK"}}]}

"#,
                r#"data: {"id":"chat_gemini_finish_sig","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":"CgR0ZXN0"},"finish_reason":"stop"}]}

"#,
                "data: [DONE]\n\n",
            ]
            .concat(),
        );

    assert!(output.contains(r#""text":"OK""#));
    assert!(output.contains(r#""finishReason":"STOP""#));
    assert!(!output.contains("thoughtSignature"));
}

#[test]
fn gemini_native_tools_and_uppercase_schema_are_preserved() {
    let llm = gemini_request_to_llm(json!({
        "contents": [{"role": "user", "parts": [{"text": "search"}]}],
        "tools": [
            {
                "functionDeclarations": [{
                    "name": "lookup",
                    "parameters": {
                        "type": "OBJECT",
                        "properties": {
                            "query": {"type": "STRING"}
                        }
                    }
                }]
            },
            {"googleSearch": {}},
            {"codeExecution": {}},
            {"urlContext": {}}
        ]
    }));
    let function_tool = llm
        .tools
        .iter()
        .find(|tool| tool.tool_type == "function")
        .unwrap();
    assert_eq!(
        function_tool
            .function
            .as_ref()
            .unwrap()
            .parameters
            .as_ref()
            .unwrap()["type"],
        "object"
    );
    assert_eq!(
        function_tool
            .function
            .as_ref()
            .unwrap()
            .parameters
            .as_ref()
            .unwrap()["properties"]["query"]["type"],
        "string"
    );
    assert!(llm
        .tools
        .iter()
        .any(|tool| tool.tool_type == TOOL_TYPE_GOOGLE_SEARCH));
    assert!(llm
        .tools
        .iter()
        .any(|tool| tool.tool_type == TOOL_TYPE_GOOGLE_CODE_EXECUTION));
    assert!(llm
        .tools
        .iter()
        .any(|tool| tool.tool_type == TOOL_TYPE_GOOGLE_URL_CONTEXT));

    let gemini = llm_request_to_gemini(llm);
    let tools = gemini["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool.get("googleSearch").is_some()));
    assert!(tools.iter().any(|tool| tool.get("codeExecution").is_some()));
    assert!(tools.iter().any(|tool| tool.get("urlContext").is_some()));
    assert_eq!(
        tools[0]["functionDeclarations"][0]["parameters"]["properties"]["query"]["type"],
        "string"
    );
}

#[test]
fn gemini_outbound_empty_function_schema_uses_object_with_properties() {
    let gemini = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        json!({
            "model": "gemini-2.5-flash",
            "messages": [{"role": "user", "content": "call tools"}],
            "tools": [
                {"type": "function", "function": {"name": "no_schema"}},
                {"type": "function", "function": {"name": "empty_schema", "parameters": {}}},
                {
                    "type": "function",
                    "function": {
                        "name": "kept_schema",
                        "parameters": {
                            "type": "object",
                            "properties": {"query": {"type": "string"}}
                        }
                    }
                }
            ]
        }),
    )
    .unwrap();
    let declarations = gemini["tools"][0]["functionDeclarations"]
        .as_array()
        .unwrap();

    assert_eq!(
        declarations[0]["parameters"],
        json!({"type": "object", "properties": {}})
    );
    assert_eq!(
        declarations[1]["parameters"],
        json!({"type": "object", "properties": {}})
    );
    assert_eq!(
        declarations[2]["parameters"]["properties"]["query"]["type"],
        "string"
    );
}

#[test]
fn gemini_outbound_rich_function_schema_uses_parameters_json_schema() {
    let gemini = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        json!({
            "model": "gemini-2.5-flash",
            "messages": [{"role": "user", "content": "call tools"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "lookup",
                    "parameters": {
                        "$schema": "https://json-schema.org/draft/2020-12/schema",
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "mode": {"const": "fast"},
                            "query": {"type": "string"}
                        },
                        "required": ["mode", "query"]
                    }
                }
            }]
        }),
    )
    .unwrap();
    let declaration = &gemini["tools"][0]["functionDeclarations"][0];

    assert!(declaration.get("parameters").is_none());
    assert!(declaration.get("parametersJsonSchema").is_some());
    assert!(declaration["parametersJsonSchema"].get("$schema").is_none());
    assert_eq!(
        declaration["parametersJsonSchema"]["additionalProperties"],
        false
    );
    assert_eq!(
        declaration["parametersJsonSchema"]["properties"]["mode"]["const"],
        "fast"
    );
}

#[test]
fn gemini_to_openai_chat_does_not_leak_google_private_fields() {
    let chat = convert_request_value(
        ConversionRoute::new(AiProtocol::GeminiNative, AiProtocol::OpenAiChat),
        json!({
            "contents": [
                {"role": "user", "parts": [{"text": "search"}]},
                {
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "id": "call_1",
                            "name": "lookup",
                            "args": {"query": "rust"},
                            "thoughtSignature": "private-google-signature"
                        },
                        "thoughtSignature": "part-private-signature"
                    }]
                }
            ],
            "tools": [
                {
                    "functionDeclarations": [{
                        "name": "lookup",
                        "parameters": {"type": "object"}
                    }]
                },
                {"googleSearch": {}},
                {"codeExecution": {}},
                {"urlContext": {}}
            ]
        }),
    )
    .unwrap();
    let body = chat.to_string();

    assert!(chat["tools"]
        .as_array()
        .unwrap()
        .iter()
        .all(|tool| { tool["type"] == "function" && tool.get("function").is_some() }));
    assert!(!body.contains("thoughtSignature"));
    assert!(!body.contains("thought_signature"));
    assert!(!body.contains("googleSearch"));
    assert!(!body.contains("codeExecution"));
    assert!(!body.contains("urlContext"));
    assert!(!body.contains("private-google-signature"));
}

#[test]
fn openai_chat_stream_target_injects_include_usage_only_for_streaming_requests() {
    let streaming = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        json!({
            "model": "claude-sonnet-4-6",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true,
            "max_tokens": 64
        }),
    )
    .unwrap();
    assert_eq!(streaming["stream_options"]["include_usage"], true);

    let non_streaming = convert_request_value(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        json!({
            "model": "claude-sonnet-4-6",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": false,
            "max_tokens": 64
        }),
    )
    .unwrap();
    assert!(non_streaming.get("stream_options").is_none());
}

#[test]
fn responses_custom_tool_extension_is_preserved_for_chat_roundtrip() {
    let (chat, context) = convert_request_value_with_context(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        read_fixture_json("openai_responses/custom_tool.request.json"),
    )
    .unwrap();

    assert!(context.is_empty());
    let tools = chat["tools"].as_array().unwrap();
    assert!(tools
        .iter()
        .any(|tool| tool["type"] == TOOL_TYPE_RESPONSES_CUSTOM_TOOL));
    assert!(chat["messages"].as_array().unwrap().iter().any(|message| {
        message
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|tool_calls| {
                tool_calls
                    .iter()
                    .any(|tool_call| tool_call["type"] == TOOL_TYPE_RESPONSES_CUSTOM_TOOL)
            })
    }));
}

#[test]
fn responses_namespace_tool_requires_chat_context_without_tool_search() {
    let (chat, context) = convert_request_value_with_context(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        json!({
            "model": "model-a",
            "input": "Use Gmail search.",
            "tools": [{
                "type": "namespace",
                "name": "gmail",
                "tools": [{
                    "type": "function",
                    "name": "search",
                    "description": "Search Gmail",
                    "parameters": {"type": "object"}
                }]
            }]
        }),
    )
    .unwrap();

    assert!(!context.is_empty());
    assert_eq!(chat["tools"][0]["function"]["name"], "gmail__search");

    let responses = convert_response_value_with_context(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        json!({
            "id": "chatcmpl_namespace",
            "object": "chat.completion",
            "created": 1,
            "model": "model-a",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_search",
                        "type": "function",
                        "function": {
                            "name": "gmail__search",
                            "arguments": "{\"query\":\"in:inbox\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }),
        Some(&context),
    )
    .unwrap();

    assert_eq!(responses["output"][0]["namespace"], "gmail");
    assert_eq!(responses["output"][0]["name"], "search");
}

#[test]
fn codex_context_request_exposes_tool_search_and_namespace_tools_for_chat() {
    let request = json!({
        "model": "gpt-5.4",
        "tools": [{"type": "tool_search"}],
        "input": [
            {
                "type": "tool_search_call",
                "call_id": "call_tool_search_1",
                "status": "completed",
                "execution": "client",
                "arguments": {"query": "Gmail search emails", "limit": 5}
            },
            {
                "type": "tool_search_output",
                "call_id": "call_tool_search_1",
                "status": "completed",
                "execution": "client",
                "tools": [{
                    "type": "namespace",
                    "name": "mcp__codex_apps__gmail",
                    "tools": [{
                        "type": "function",
                        "name": "_search_emails",
                        "description": "Search Gmail.",
                        "parameters": {"type": "object"}
                    }]
                }]
            },
            {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "Search unread inbox mail."}]
            }
        ]
    });

    let (chat, context) = convert_request_value_with_context(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        request,
    )
    .unwrap();

    assert!(!context.is_empty());
    let tool_names = chat["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool.pointer("/function/name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(tool_names.contains(&"tool_search"));
    assert!(tool_names.contains(&"mcp__codex_apps__gmail___search_emails"));
    assert_eq!(
        chat["messages"][0]["tool_calls"][0]["function"]["name"],
        "tool_search"
    );
    assert_eq!(chat["messages"][1]["role"], "tool");
    assert_eq!(chat["messages"][1]["tool_call_id"], "call_tool_search_1");
    assert!(chat["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("mcp__codex_apps__gmail"));
}

#[test]
fn codex_context_response_restores_namespace_and_tool_search_calls() {
    let request = json!({
        "model": "gpt-5.4",
        "tools": [{"type": "tool_search"}],
        "input": [{
            "type": "tool_search_output",
            "call_id": "call_tool_search_1",
            "tools": [{
                "type": "namespace",
                "name": "mcp__codex_apps__gmail",
                "tools": [{
                    "type": "function",
                    "name": "_search_emails",
                    "description": "Search Gmail.",
                    "parameters": {"type": "object"}
                }]
            }]
        }]
    });
    let (_, context) = convert_request_value_with_context(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        request,
    )
    .unwrap();

    let namespace_response = convert_response_value_with_context(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        json!({
            "id": "chatcmpl_gmail",
            "object": "chat.completion",
            "created": 123,
            "model": "gpt-5.4",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_gmail",
                        "type": "function",
                        "function": {
                            "name": "mcp__codex_apps__gmail___search_emails",
                            "arguments": "{\"query\":\"in:inbox\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }),
        Some(&context),
    )
    .unwrap();
    assert_eq!(namespace_response["output"][0]["type"], "function_call");
    assert_eq!(
        namespace_response["output"][0]["namespace"],
        "mcp__codex_apps__gmail"
    );
    assert_eq!(namespace_response["output"][0]["name"], "_search_emails");

    let tool_search_response = convert_response_value_with_context(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        json!({
            "id": "chatcmpl_tool_search",
            "object": "chat.completion",
            "created": 123,
            "model": "gpt-5.4",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_tool_search_2",
                        "type": "function",
                        "function": {
                            "name": "tool_search",
                            "arguments": "{\"query\":\"Gmail search emails\",\"limit\":10}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }),
        Some(&context),
    )
    .unwrap();
    assert_eq!(
        tool_search_response["output"][0]["type"],
        "tool_search_call"
    );
    assert_eq!(
        tool_search_response["output"][0]["arguments"]["query"],
        "Gmail search emails"
    );
    assert_eq!(tool_search_response["output"][0]["arguments"]["limit"], 10);
}

#[test]
fn codex_context_stream_restores_namespace_and_tool_search_items() {
    let request = json!({
        "model": "gpt-5.4",
        "tools": [{"type": "tool_search"}],
        "input": [{
            "type": "tool_search_output",
            "call_id": "call_tool_search_1",
            "tools": [{
                "type": "namespace",
                "name": "mcp__codex_apps__gmail",
                "tools": [{
                    "type": "function",
                    "name": "_search_emails",
                    "description": "Search Gmail.",
                    "parameters": {"type": "object"}
                }]
            }]
        }]
    });
    let (_, context) = convert_request_value_with_context(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        request,
    )
    .unwrap();
    let namespace_stream = [
            r#"data: {"id":"chatcmpl_gmail","model":"gpt-5.4","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_gmail","type":"function","function":{"name":"mcp__codex_apps__gmail___search_emails"}}]}}]}"#,
            "\n\n",
            r#"data: {"id":"chatcmpl_gmail","model":"gpt-5.4","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"query\":\"in:inbox\"}"}}]},"finish_reason":"tool_calls"}]}"#,
            "\n\n",
            "data: [DONE]\n\n",
        ]
        .concat();
    let output = collect_stream_with_context(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        namespace_stream,
        context.clone(),
    );
    assert!(output.contains("\"type\":\"function_call\""));
    assert!(output.contains("\"namespace\":\"mcp__codex_apps__gmail\""));
    assert!(output.contains("\"name\":\"_search_emails\""));

    let tool_search_stream = [
            r#"data: {"id":"chatcmpl_tool_search","model":"gpt-5.4","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_tool_search_2","type":"function","function":{"name":"tool_search"}}]}}]}"#,
            "\n\n",
            r#"data: {"id":"chatcmpl_tool_search","model":"gpt-5.4","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"query\":\"Gmail search emails\",\"limit\":10}"}}]},"finish_reason":"tool_calls"}]}"#,
            "\n\n",
            "data: [DONE]\n\n",
        ]
        .concat();
    let output = collect_stream_with_context(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        tool_search_stream,
        context,
    );
    assert!(output.contains("\"type\":\"tool_search_call\""));
    assert!(output.contains("\"execution\":\"client\""));
    assert!(output.contains("\"query\":\"Gmail search emails\""));
}

#[test]
fn responses_namespace_tools_roundtrip_through_anthropic_and_gemini() {
    let flat_name = "gmail__search";
    for target in [AiProtocol::AnthropicMessages, AiProtocol::GeminiNative] {
        let request = json!({
            "model": "model-a",
            "input": [
                {
                    "type": "function_call",
                    "call_id": "call_old",
                    "namespace": "gmail",
                    "name": "search",
                    "arguments": "{\"query\":\"old\"}"
                },
                {"type": "function_call_output", "call_id": "call_old", "output": "ok"}
            ],
            "tools": [{
                "type": "namespace",
                "name": "gmail",
                "tools": [{
                    "type": "function",
                    "name": "search",
                    "description": "Search Gmail",
                    "parameters": {"type": "object"}
                }]
            }],
            "tool_choice": {"type": "function", "namespace": "gmail", "name": "search"}
        });
        let (converted_request, context) = convert_request_value_with_context(
            ConversionRoute::new(AiProtocol::OpenAiResponses, target),
            request,
        )
        .unwrap();
        assert!(!context.is_empty());

        match target {
            AiProtocol::AnthropicMessages => {
                assert_eq!(converted_request["tools"][0]["name"], flat_name);
                assert_eq!(converted_request["tool_choice"]["name"], flat_name);
                assert_eq!(
                    converted_request["messages"][0]["content"][0]["name"],
                    flat_name
                );
            }
            AiProtocol::GeminiNative => {
                assert_eq!(
                    converted_request["tools"][0]["functionDeclarations"][0]["name"],
                    flat_name
                );
                assert_eq!(
                    converted_request["toolConfig"]["functionCallingConfig"]
                        ["allowedFunctionNames"][0],
                    flat_name
                );
                assert_eq!(
                    converted_request["contents"][0]["parts"][0]["functionCall"]["name"],
                    flat_name
                );
            }
            _ => unreachable!(),
        }

        let upstream_response = match target {
            AiProtocol::AnthropicMessages => json!({
                "id": "msg_namespace",
                "type": "message",
                "role": "assistant",
                "model": "model-a",
                "content": [{
                    "type": "tool_use",
                    "id": "call_new",
                    "name": flat_name,
                    "input": {"query": "new"}
                }],
                "stop_reason": "tool_use",
                "usage": {"input_tokens": 1, "output_tokens": 1}
            }),
            AiProtocol::GeminiNative => json!({
                "responseId": "gemini_namespace",
                "modelVersion": "model-a",
                "candidates": [{
                    "content": {"role": "model", "parts": [{
                        "functionCall": {
                            "id": "call_new",
                            "name": flat_name,
                            "args": {"query": "new"}
                        }
                    }]},
                    "finishReason": "STOP"
                }]
            }),
            _ => unreachable!(),
        };
        let responses = convert_response_value_with_context(
            ConversionRoute::new(target, AiProtocol::OpenAiResponses),
            upstream_response,
            Some(&context),
        )
        .unwrap();
        assert_eq!(responses["output"][0]["type"], "function_call");
        assert_eq!(responses["output"][0]["namespace"], "gmail");
        assert_eq!(responses["output"][0]["name"], "search");

        let namespace_stream = stream_fixture(target).replace("read_file", flat_name);
        let stream_output = collect_stream_with_context(
            ConversionRoute::new(target, AiProtocol::OpenAiResponses),
            namespace_stream,
            context,
        );
        assert!(stream_output.contains("\"namespace\":\"gmail\""));
        assert!(stream_output.contains("\"name\":\"search\""));
    }
}

#[test]
fn responses_namespace_tool_name_collisions_fail_closed() {
    let top_level_collision = json!({
        "model": "model-a",
        "input": "hi",
        "tools": [
            {"type": "function", "name": "foo__bar", "parameters": {"type": "object"}},
            {"type": "namespace", "name": "foo", "tools": [
                {"type": "function", "name": "bar", "parameters": {"type": "object"}}
            ]}
        ]
    });
    let error = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::AnthropicMessages),
        top_level_collision,
    )
    .unwrap_err();
    assert!(error.to_string().contains("foo__bar"));
    assert!(error.to_string().contains("top-level tool"));

    let namespace_collision = json!({
        "model": "model-a",
        "input": "hi",
        "tools": [
            {"type": "namespace", "name": "foo", "tools": [
                {"type": "function", "name": "bar__baz", "parameters": {"type": "object"}}
            ]},
            {"type": "namespace", "name": "foo__bar", "tools": [
                {"type": "function", "name": "baz", "parameters": {"type": "object"}}
            ]}
        ]
    });
    let error = convert_request_value(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::GeminiNative),
        namespace_collision,
    )
    .unwrap_err();
    assert!(error.to_string().contains("foo__bar__baz"));
    assert!(error.to_string().contains("namespace tool"));
}

#[test]
fn reference_stream_finish_and_tool_arguments_are_exact() {
    let responses_output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        stream_fixture(AiProtocol::OpenAiChat),
    );
    let responses_values = sse_data_values(&responses_output);
    let arguments_done = responses_values
        .iter()
        .find(|value| {
            value.get("type").and_then(Value::as_str)
                == Some("response.function_call_arguments.done")
        })
        .expect("responses arguments.done event");
    assert_eq!(arguments_done["arguments"], "{\"path\":\"a.txt\"}");
    assert_eq!(
        occurrence_count(&responses_output, "event: response.completed"),
        1
    );

    let chat_output = collect_stream(
        ConversionRoute::new(AiProtocol::AnthropicMessages, AiProtocol::OpenAiChat),
        stream_fixture(AiProtocol::AnthropicMessages),
    );
    let chat_values = sse_data_values(&chat_output);
    let finish = chat_values
        .iter()
        .find(|value| value["choices"][0]["finish_reason"] == "tool_calls")
        .expect("chat finish event");
    assert_eq!(finish["choices"][0]["delta"], json!({}));
    assert_eq!(occurrence_count(&chat_output, "data: [DONE]"), 1);

    let anthropic_output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
        stream_fixture(AiProtocol::OpenAiChat),
    );
    assert_eq!(
        occurrence_count(&anthropic_output, "event: message_stop"),
        1
    );
    assert_eq!(
        occurrence_count(&anthropic_output, r#""type":"tool_use""#),
        1
    );

    let gemini_output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::GeminiNative),
        stream_fixture(AiProtocol::OpenAiChat),
    );
    assert!(!gemini_output.contains("[DONE]"));
    assert_eq!(occurrence_count(&gemini_output, "finishReason"), 1);
}

#[test]
fn live_provider_response_fixtures_convert_to_all_targets() {
    let cases = [
        (
            AiProtocol::OpenAiChat,
            "openai_chat",
            "deepseek-v4-flash.response.json",
        ),
        (
            AiProtocol::OpenAiResponses,
            "openai_responses",
            "deepseek-v4-flash.response.json",
        ),
        (
            AiProtocol::AnthropicMessages,
            "anthropic",
            "claude-haiku-4-5-20251001.response.json",
        ),
        (
            AiProtocol::GeminiNative,
            "gemini",
            "gemini-2.5-flash.response.json",
        ),
    ];

    for (source, protocol_dir, file_name) in cases {
        let value = read_live_provider_fixture_json(protocol_dir, file_name);
        for target in PROTOCOLS {
            if source == target {
                continue;
            }
            let converted =
                    convert_response_value(ConversionRoute::new(source, target), value.clone())
                        .unwrap_or_else(|error| {
                            panic!(
                                "live provider response fixture {protocol_dir}/{file_name} route {} -> {} failed: {error}",
                                source.as_str(),
                                target.as_str()
                            )
                        });
            assert_response_shape(target, &converted);
            assert_live_provider_response_semantics(source, target, &converted);
        }
    }
}

#[test]
fn response_matrix_responses_to_gemini_uses_new_kernel() {
    let converted = convert_response_value(
            ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::GeminiNative),
            json!({
                "id": "resp_1",
                "model": "gpt-5",
                "status": "completed",
                "output": [
                    {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "hello"}]},
                    {"type": "function_call", "call_id": "call_1", "name": "read_file", "arguments": "{\"path\":\"a\"}"}
                ],
                "usage": {"input_tokens": 10, "output_tokens": 2, "input_tokens_details": {"cached_tokens": 3}}
            }),
        )
        .unwrap();

    assert_eq!(converted["responseId"], "resp_1");
    assert_eq!(
        converted["candidates"][0]["content"]["parts"][0]["text"],
        "hello"
    );
    assert_eq!(
        converted["candidates"][0]["content"]["parts"][1]["functionCall"]["name"],
        "read_file"
    );
    assert_eq!(converted["usageMetadata"]["promptTokenCount"], 10);
}

#[test]
fn chat_stream_tier_echo_survives_conversion_to_responses() {
    // A Chat-type relay echoes `service_tier` on its final chunk. Codex speaks
    // Responses, so the gateway converts the stream — the tier must re-appear
    // in the synthesized `response.completed` snapshot or billing falls back to
    // the requested tier (issue: served tier read off the converted body).
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
        [
            r#"data: {"id":"chatcmpl_1","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{"role":"assistant","content":"hi"},"finish_reason":null}]}

"#,
            r#"data: {"id":"chatcmpl_1","object":"chat.completion.chunk","model":"gpt-5","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"service_tier":"priority"}

"#,
            "data: [DONE]\n\n",
        ]
        .concat(),
    );
    let values = sse_data_values(&output);
    let completed = values
        .iter()
        .rev()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("response.completed"))
        .expect("converted stream must end with response.completed");
    assert_eq!(
        completed
            .pointer("/response/service_tier")
            .and_then(Value::as_str),
        Some("fast"),
        "tier echo must survive Chat->Responses stream conversion; output={output}"
    );
}

#[test]
fn responses_stream_tier_echo_survives_conversion_to_chat() {
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        [
            r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_1","model":"gpt-5"}}

"#,
            r#"event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"hi","item_id":"msg_1","output_index":0,"content_index":0}

"#,
            r#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_1","model":"gpt-5","status":"completed","service_tier":"ultrafast","usage":{"input_tokens":5,"output_tokens":2,"total_tokens":7}}}

"#,
        ]
        .concat(),
    );
    let values = sse_data_values(&output);
    let tier_chunk = values
        .iter()
        .rev()
        .find(|value| value.get("service_tier").is_some())
        .unwrap_or_else(|| panic!("Chat target must re-render the tier echo; output={output}"));
    assert_eq!(
        tier_chunk.get("service_tier").and_then(Value::as_str),
        Some("ultrafast")
    );
    // A `default` echo is not a premium confirmation and must not be rendered.
    let output = collect_stream(
        ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
        [
            r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_1","model":"gpt-5"}}

"#,
            r#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_1","model":"gpt-5","status":"completed","service_tier":"default"}}

"#,
        ]
        .concat(),
    );
    assert!(
        !output.contains("service_tier"),
        "a default echo is not a premium confirmation; output={output}"
    );
}
