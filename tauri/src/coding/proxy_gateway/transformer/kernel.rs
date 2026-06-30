use super::anthropic::{AnthropicInbound, AnthropicOutbound};
use super::error::ProtocolConversionError;
use super::gemini::{GeminiInbound, GeminiOutbound};
use super::openai::chat::{OpenAiChatInbound, OpenAiChatOutbound};
use super::openai::responses::{OpenAiResponsesInbound, OpenAiResponsesOutbound};
use super::stream::StreamKernel;
use super::traits::{InboundTransformer, OutboundTransformer};
use super::types::{AiProtocol, ConversionRoute};
use futures_util::{stream, Stream, StreamExt};
use serde_json::Value;
use std::collections::VecDeque;
use std::pin::Pin;

pub type ConversionByteStream =
    Pin<Box<dyn Stream<Item = Result<Vec<u8>, String>> + Send + 'static>>;

pub fn convert_request_body(
    route: ConversionRoute,
    body: &[u8],
) -> Result<Vec<u8>, ProtocolConversionError> {
    if route.identity() {
        return Ok(body.to_vec());
    }
    let value = serde_json::from_slice::<Value>(body)
        .map_err(|error| ProtocolConversionError::InvalidJson(error.to_string()))?;
    let converted = convert_request_value(route, value)?;
    serde_json::to_vec(&converted)
        .map_err(|error| ProtocolConversionError::Transform(error.to_string()))
}

pub fn convert_response_body(
    route: ConversionRoute,
    body: &[u8],
) -> Result<Vec<u8>, ProtocolConversionError> {
    if route.identity() {
        return Ok(body.to_vec());
    }
    let value = serde_json::from_slice::<Value>(body)
        .map_err(|error| ProtocolConversionError::InvalidJson(error.to_string()))?;
    let converted = convert_response_value(route, value)?;
    serde_json::to_vec(&converted)
        .map_err(|error| ProtocolConversionError::Transform(error.to_string()))
}

pub fn convert_error_response_body(route: ConversionRoute, body: &[u8]) -> Vec<u8> {
    if route.identity() {
        return body.to_vec();
    }
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return body.to_vec();
    };
    let normalized = outbound_transformer(route.source).error_to_llm(value);
    let converted = inbound_transformer(route.target).error_from_llm(normalized);
    serde_json::to_vec(&converted).unwrap_or_else(|_| body.to_vec())
}

pub fn convert_request_value(
    route: ConversionRoute,
    value: Value,
) -> Result<Value, ProtocolConversionError> {
    if route.identity() {
        return Ok(value);
    }
    let request = inbound_transformer(route.source).request_to_llm(value)?;
    outbound_transformer(route.target).request_from_llm(request)
}

pub fn convert_response_value(
    route: ConversionRoute,
    value: Value,
) -> Result<Value, ProtocolConversionError> {
    if route.identity() {
        return Ok(value);
    }
    let response = outbound_transformer(route.source).response_to_llm(value)?;
    inbound_transformer(route.target).response_from_llm(response)
}

pub fn convert_sse_stream(
    route: ConversionRoute,
    inner: ConversionByteStream,
) -> ConversionByteStream {
    if route.identity() {
        return inner;
    }

    struct StreamState {
        inner: ConversionByteStream,
        kernel: StreamKernel,
        pending: VecDeque<Result<Vec<u8>, String>>,
        source_finished: bool,
    }

    let state = StreamState {
        inner,
        kernel: StreamKernel::new(route),
        pending: VecDeque::new(),
        source_finished: false,
    };

    Box::pin(stream::unfold(state, |mut state| async move {
        loop {
            if let Some(output) = state.pending.pop_front() {
                return Some((output, state));
            }
            if state.source_finished {
                return None;
            }
            match state.inner.next().await {
                Some(Ok(chunk)) => {
                    for output in state.kernel.push_chunk(&chunk) {
                        state.pending.push_back(Ok(output));
                    }
                }
                Some(Err(error)) => return Some((Err(error), state)),
                None => {
                    state.source_finished = true;
                    for output in state.kernel.finish() {
                        state.pending.push_back(Ok(output));
                    }
                }
            }
        }
    }))
}

fn inbound_transformer(protocol: AiProtocol) -> Box<dyn InboundTransformer> {
    match protocol {
        AiProtocol::AnthropicMessages => Box::new(AnthropicInbound),
        AiProtocol::OpenAiChat => Box::new(OpenAiChatInbound),
        AiProtocol::OpenAiResponses => Box::new(OpenAiResponsesInbound),
        AiProtocol::GeminiNative => Box::new(GeminiInbound),
    }
}

fn outbound_transformer(protocol: AiProtocol) -> Box<dyn OutboundTransformer> {
    match protocol {
        AiProtocol::AnthropicMessages => Box::new(AnthropicOutbound),
        AiProtocol::OpenAiChat => Box::new(OpenAiChatOutbound),
        AiProtocol::OpenAiResponses => Box::new(OpenAiResponsesOutbound),
        AiProtocol::GeminiNative => Box::new(GeminiOutbound),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::proxy_gateway::transformer::anthropic::anthropic_request_to_llm;
    use crate::coding::proxy_gateway::transformer::gemini::{
        gemini_request_to_llm, gemini_response_to_llm, llm_request_to_gemini,
        llm_response_to_gemini,
    };
    use crate::coding::proxy_gateway::transformer::llm::{
        ApiFormat, FunctionCall, Message, MessageContent, RequestType, ToolCall,
        TOOL_TYPE_FUNCTION, TOOL_TYPE_GOOGLE_CODE_EXECUTION, TOOL_TYPE_GOOGLE_SEARCH,
        TOOL_TYPE_GOOGLE_URL_CONTEXT, TOOL_TYPE_RESPONSES_CUSTOM_TOOL,
    };
    use crate::coding::proxy_gateway::transformer::openai::chat::{
        chat_request_to_llm, chat_response_to_llm, llm_response_to_chat,
    };
    use crate::coding::proxy_gateway::transformer::openai::responses::{
        llm_response_to_responses, responses_request_to_llm, responses_response_to_llm,
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
data: {"type":"message_start","message":{"id":"msg_1","model":"model-a"}}

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
                r#"data: {"id":"chat_1","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"completion_tokens":3}}

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
data: {"type":"response.completed","response":{"id":"resp_1","model":"model-a","status":"completed","usage":{"output_tokens":3}}}

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
                r#"data: {"responseId":"gemini_1","modelVersion":"model-a","candidates":[{"finishReason":"STOP"}],"usageMetadata":{"candidatesTokenCount":3}}}

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
            }
            AiProtocol::OpenAiChat => {
                assert!(output.contains("chat.completion.chunk"));
                assert!(output.contains(r#""content":"#));
                assert!(output.contains("reasoning_content"));
                assert!(output.contains("tool_calls"));
                assert!(output.contains("[DONE]"));
            }
            AiProtocol::OpenAiResponses => {
                assert!(output.contains("event: response.created"));
                assert!(output.contains("response.output_text.delta"));
                assert!(output.contains("response.reasoning_summary_text.delta"));
                assert!(output.contains("response.output_item.added"));
                assert!(output.contains("response.function_call_arguments.done"));
                assert!(output.contains("event: response.completed"));
            }
            AiProtocol::GeminiNative => {
                assert!(output.contains("candidates"));
                assert!(output.contains(r#""text":"hello""#));
                assert!(output.contains("functionCall"));
                assert!(output.contains("finishReason"));
            }
        }
    }

    fn assert_stream_basic_shape(protocol: AiProtocol, output: &str) {
        match protocol {
            AiProtocol::AnthropicMessages => {
                assert!(output.contains("event: message_start"));
                assert!(output.contains("event: message_stop"));
            }
            AiProtocol::OpenAiChat => {
                assert!(output.contains("chat.completion.chunk"));
                assert!(output.contains("[DONE]"));
            }
            AiProtocol::OpenAiResponses => {
                assert!(output.contains("event: response.created"));
                assert!(output.contains("event: response.completed"));
            }
            AiProtocol::GeminiNative => {
                assert!(output.contains("candidates"));
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
            "openai_responses/compact.response.json"
                | "openai_responses/image-generation.request.json"
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
        let text = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("read live provider fixture {}: {error}", path.display())
        });
        serde_json::from_str(&text).unwrap_or_else(|error| {
            panic!("parse live provider fixture {}: {error}", path.display())
        })
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
            gemini["generationConfig"]["responseSchema"]["properties"]["ok"]["type"],
            "boolean"
        );
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
            converted["contents"][1]["parts"][0]["functionResponse"]["response"]["content"],
            "sunny"
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
            "input": [{
                "type": "reasoning",
                "summary": [{"type": "summary_text", "text": "Need a tool."}],
                "encrypted_content": "gAAAAABopenai-request-signature"
            }]
        });

        let llm = responses_request_to_llm(source);
        let roundtrip = OpenAiResponsesOutbound.request_from_llm(llm).unwrap();

        assert_eq!(
            roundtrip["input"][0]["encrypted_content"],
            "gAAAAABopenai-request-signature"
        );
        assert_eq!(
            roundtrip["include"],
            json!(["file_search_call.results", "reasoning.encrypted_content"])
        );
    }

    #[test]
    fn responses_json_encrypted_only_reasoning_is_preserved() {
        let source = json!({
            "model": "gpt-5.2",
            "input": [{
                "type": "reasoning",
                "summary": [],
                "encrypted_content": "gAAAAABencrypted-only"
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
            "gAAAAABencrypted-only"
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
                "encrypted_content": "gAAAAABopenai-response-signature"
            }],
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
        });

        let llm = OpenAiResponsesOutbound
            .response_to_llm(source.clone())
            .unwrap();
        let roundtrip = OpenAiResponsesInbound.response_from_llm(llm).unwrap();
        assert_eq!(
            roundtrip["output"][0]["encrypted_content"],
            "gAAAAABopenai-response-signature"
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
            "gAAAAABopenai-response-signature"
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
        let message = Message {
            role: "assistant".to_string(),
            reasoning_signature: Some("gAAAAABopenai-signature".to_string()),
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

        let gemini =
            llm_request_to_gemini(crate::coding::proxy_gateway::transformer::llm::Request {
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
            "gAAAAABopenai-signature"
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
            item["type"] == "custom_tool_call_output"
                && item["output"] == "Patch applied successfully."
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
            gemini["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "medium"
        );
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
            "metadata": {"user_id": "user_1"},
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
    fn gemini_thinking_budget_threshold_matches_axonhub() {
        let low = gemini_request_to_llm(json!({
            "generationConfig": {
                "thinkingConfig": {"includeThoughts": true, "thinkingBudget": 1024}
            },
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }));
        assert_eq!(low.reasoning_effort.as_deref(), Some("low"));

        let medium = gemini_request_to_llm(json!({
            "generationConfig": {
                "thinkingConfig": {"includeThoughts": true, "thinkingBudget": 8192}
            },
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }));
        assert_eq!(medium.reasoning_effort.as_deref(), Some("medium"));

        let high = gemini_request_to_llm(json!({
            "generationConfig": {
                "thinkingConfig": {"includeThoughts": true, "thinkingBudget": 8193}
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
    fn anthropic_sse_defers_signature_until_thinking_block_close() {
        let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_sig","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_sig","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":"EqQstream-signature"}}]}

"#,
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
            Some("EqQstream-signature")
        );
        assert!(signature_index < thinking_stop_index);
        assert!(thinking_stop_index < text_start_index);
    }

    #[test]
    fn anthropic_sse_signature_only_creates_synthetic_thinking_block() {
        let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_sig_only","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_sig_only","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":"EqQsignature-only"}}]}

"#,
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
                    && value.pointer("/content_block/type").and_then(Value::as_str)
                        == Some("thinking")
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
            Some("EqQsignature-only")
        );
    }

    #[test]
    fn anthropic_sse_signature_before_text_creates_synthetic_thinking_before_text() {
        let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_sig_text","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_sig_text","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":"EqQsignature-before-text"}}]}

"#,
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
            Some("EqQsignature-before-text")
        );
        assert!(signature_delta < thinking_stop);
        assert!(thinking_stop < text_start);
    }

    #[test]
    fn anthropic_sse_signature_before_tool_creates_synthetic_thinking_before_tool() {
        let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_sig_tool","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_sig_tool","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":"EqQsignature-before-tool"}}]}

"#,
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
                    && value.pointer("/content_block/type").and_then(Value::as_str)
                        == Some("tool_use")
            })
            .expect("tool start");

        assert_eq!(
            values[signature_delta]
                .pointer("/delta/signature")
                .and_then(Value::as_str),
            Some("EqQsignature-before-tool")
        );
        assert!(signature_delta < thinking_stop);
        assert!(thinking_stop < tool_start);
    }

    #[test]
    fn anthropic_sse_signature_after_thinking_flushes_on_finish() {
        let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::AnthropicMessages),
            [
                r#"data: {"id":"chat_sig_finish","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_sig_finish","model":"model-a","choices":[{"index":0,"delta":{"reasoning_content":"Think"}}]}

"#,
                r#"data: {"id":"chat_sig_finish","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":"EqQsignature-finish"}}]}

"#,
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
        let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_enc","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_enc","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":"gAAAAABstream-encrypted"}}]}

"#,
                r#"data: {"id":"chat_enc","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

"#,
                "data: [DONE]\n\n",
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
        assert_eq!(done["item"]["encrypted_content"], "gAAAAABstream-encrypted");
    }

    #[test]
    fn responses_sse_target_preserves_marked_encrypted_only_reasoning() {
        let output = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiChat, AiProtocol::OpenAiResponses),
            [
                r#"data: {"id":"chat_marked_enc","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"}}]}

"#,
                r#"data: {"id":"chat_marked_enc","model":"model-a","choices":[{"index":0,"delta":{"reasoning_signature":"ai-toolbox.sig.openai_responses:QVhOMTAzencrypted_data_here"}}]}

"#,
                r#"data: {"id":"chat_marked_enc","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

"#,
                "data: [DONE]\n\n",
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
            "QVhOMTAzencrypted_data_here"
        );
    }

    #[test]
    fn responses_sse_encrypted_content_does_not_leak_to_other_targets() {
        let input = [
            r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_enc","model":"model-a","output":[]}}

"#,
            r#"event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[]}}

"#,
            r#"event: response.output_item.done
data: {"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[],"encrypted_content":"gAAAAABresponses-stream"}}

"#,
            r#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_enc","model":"model-a","status":"completed","output":[]}}

"#,
        ]
        .concat();

        let anthropic = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::AnthropicMessages),
            input.clone(),
        );
        assert!(!anthropic.contains("signature_delta"));
        assert!(!anthropic.contains("gAAAAABresponses-stream"));

        let gemini = collect_stream(
            ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::GeminiNative),
            input,
        );
        assert!(!gemini.contains("thoughtSignature"));
        assert!(!gemini.contains("gAAAAABresponses-stream"));
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
        let chat = convert_request_value(
            ConversionRoute::new(AiProtocol::OpenAiResponses, AiProtocol::OpenAiChat),
            read_fixture_json("openai_responses/custom_tool.request.json"),
        )
        .unwrap();

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
}
