use super::types::GatewayCliKey;
use serde_json::Value;

const MAX_SSE_USAGE_BUFFER_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
}

impl TokenUsage {
    pub fn total_tokens(&self) -> Option<u64> {
        let input = self.input_tokens.unwrap_or(0);
        let output = self.output_tokens.unwrap_or(0);
        let cache_read = self.cache_read_tokens.unwrap_or(0);
        let cache_creation = self.cache_creation_tokens.unwrap_or(0);
        let total = input
            .saturating_add(output)
            .saturating_add(cache_read)
            .saturating_add(cache_creation);
        (total > 0).then_some(total)
    }

    fn merge_max(&mut self, other: TokenUsage) {
        self.input_tokens = max_option(self.input_tokens, other.input_tokens);
        self.output_tokens = max_option(self.output_tokens, other.output_tokens);
        self.cache_read_tokens = max_option(self.cache_read_tokens, other.cache_read_tokens);
        self.cache_creation_tokens =
            max_option(self.cache_creation_tokens, other.cache_creation_tokens);
    }
}

#[derive(Debug, Default)]
pub struct SseUsageCollector {
    buffer: Vec<u8>,
    usage: TokenUsage,
    provider_type: Option<String>,
}

impl SseUsageCollector {
    pub fn with_provider_type(provider_type: Option<&str>) -> Self {
        Self {
            buffer: Vec::new(),
            usage: TokenUsage::default(),
            provider_type: provider_type
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
        }
    }

    pub fn push_chunk(&mut self, cli_key: GatewayCliKey, chunk: &[u8]) {
        if chunk.len() > MAX_SSE_USAGE_BUFFER_BYTES {
            self.buffer.clear();
            return;
        }
        if self.buffer.len().saturating_add(chunk.len()) > MAX_SSE_USAGE_BUFFER_BYTES {
            self.buffer.clear();
        }
        self.buffer.extend_from_slice(chunk);
        while let Some(block) = take_sse_block(&mut self.buffer) {
            if let Some(value) = parse_sse_data_block(&block) {
                self.merge_event(cli_key, &value);
            }
        }
    }

    pub fn finish(mut self, cli_key: GatewayCliKey) -> TokenUsage {
        if !self.buffer.is_empty() {
            if let Some(value) = parse_sse_data_block(&self.buffer) {
                self.merge_event(cli_key, &value);
            }
        }
        self.usage
    }

    fn merge_event(&mut self, cli_key: GatewayCliKey, value: &Value) {
        self.usage.merge_max(from_json_response_with_provider_type(
            cli_key,
            value,
            self.provider_type.as_deref(),
        ));
    }
}

pub fn from_response_body(cli_key: GatewayCliKey, body: &[u8]) -> TokenUsage {
    from_response_body_with_provider_type(cli_key, None, body)
}

pub fn from_response_body_with_provider_type(
    cli_key: GatewayCliKey,
    provider_type: Option<&str>,
    body: &[u8],
) -> TokenUsage {
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        return from_json_response_with_provider_type(cli_key, &value, provider_type);
    }

    let mut collector = SseUsageCollector::with_provider_type(provider_type);
    collector.push_chunk(cli_key, body);
    collector.finish(cli_key)
}

fn from_json_response_with_provider_type(
    cli_key: GatewayCliKey,
    value: &Value,
    provider_type: Option<&str>,
) -> TokenUsage {
    match cli_key {
        GatewayCliKey::Claude => claude_usage(value, provider_type),
        GatewayCliKey::Codex | GatewayCliKey::Grok | GatewayCliKey::OpenCode => openai_usage(value),
        GatewayCliKey::Gemini => gemini_usage(value),
    }
}

fn claude_usage(value: &Value, provider_type: Option<&str>) -> TokenUsage {
    let usage = value.pointer("/usage").or_else(|| {
        value
            .pointer("/message/usage")
            .or_else(|| value.pointer("/delta/usage"))
    });
    let usage_value = usage.unwrap_or(value);
    let input_tokens = first_u64_at_paths(usage_value, &["/input_tokens", "/prompt_tokens"])
        .or_else(|| {
            first_u64_at_paths(
                value,
                &[
                    "/usage/input_tokens",
                    "/message/usage/input_tokens",
                    "/delta/usage/input_tokens",
                ],
            )
        });
    let signed_input_tokens = first_i64_at_paths(usage_value, &["/input_tokens", "/prompt_tokens"])
        .or_else(|| {
            first_i64_at_paths(
                value,
                &[
                    "/usage/input_tokens",
                    "/message/usage/input_tokens",
                    "/delta/usage/input_tokens",
                ],
            )
        });
    let cache_read_tokens = first_u64_at_paths(
        usage_value,
        &[
            "/cache_read_input_tokens",
            "/cache_read_tokens",
            "/cached_tokens",
        ],
    )
    .or_else(|| {
        first_u64_at_paths(
            value,
            &[
                "/usage/cache_read_input_tokens",
                "/usage/cached_tokens",
                "/message/usage/cache_read_input_tokens",
                "/message/usage/cached_tokens",
                "/delta/usage/cache_read_input_tokens",
                "/delta/usage/cached_tokens",
            ],
        )
    });
    let cache_creation_tokens = first_u64_at_paths(
        usage_value,
        &[
            "/cache_creation_input_tokens",
            "/cache_creation_tokens",
            "/cache_write_input_tokens",
        ],
    )
    .or_else(|| {
        first_u64_at_paths(
            value,
            &[
                "/usage/cache_creation_input_tokens",
                "/message/usage/cache_creation_input_tokens",
                "/delta/usage/cache_creation_input_tokens",
            ],
        )
    });
    let input_tokens = if is_moonshot_provider_type(provider_type) {
        moonshot_fresh_input_tokens(signed_input_tokens, input_tokens, cache_read_tokens)
    } else {
        input_tokens
    };

    TokenUsage {
        input_tokens,
        output_tokens: first_u64_at_paths(usage_value, &["/output_tokens", "/completion_tokens"])
            .or_else(|| {
                first_u64_at_paths(
                    value,
                    &[
                        "/usage/output_tokens",
                        "/message/usage/output_tokens",
                        "/delta/usage/output_tokens",
                    ],
                )
            }),
        cache_read_tokens,
        cache_creation_tokens,
    }
}

fn is_moonshot_provider_type(provider_type: Option<&str>) -> bool {
    provider_type
        .map(|value| value.trim().to_ascii_lowercase().replace('_', "-"))
        .is_some_and(|value| matches!(value.as_str(), "moonshot" | "kimi"))
}

fn moonshot_fresh_input_tokens(
    signed_input_tokens: Option<i64>,
    input_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
) -> Option<u64> {
    let cache_read = cache_read_tokens.unwrap_or(0);
    if cache_read == 0 {
        return input_tokens;
    }
    if let Some(signed_input) = signed_input_tokens {
        if signed_input < 0 {
            return Some((signed_input + cache_read as i64).max(0) as u64);
        }
    }
    let input = input_tokens?;
    if input < cache_read {
        Some(input)
    } else {
        Some(input.saturating_sub(cache_read))
    }
}

fn openai_usage(value: &Value) -> TokenUsage {
    let usage = value
        .pointer("/usage")
        .or_else(|| value.pointer("/response/usage"))
        .unwrap_or(value);
    let raw_input_tokens =
        first_u64_at_paths(usage, &["/input_tokens", "/prompt_tokens"]).or_else(|| {
            first_u64_at_paths(
                value,
                &[
                    "/usage/input_tokens",
                    "/usage/prompt_tokens",
                    "/response/usage/input_tokens",
                    "/response/usage/prompt_tokens",
                ],
            )
        });
    let cache_read_tokens = first_u64_at_paths(
        usage,
        &[
            "/input_tokens_details/cached_tokens",
            "/prompt_tokens_details/cached_tokens",
        ],
    )
    .or_else(|| {
        first_u64_at_paths(
            value,
            &[
                "/usage/input_tokens_details/cached_tokens",
                "/usage/prompt_tokens_details/cached_tokens",
                "/response/usage/input_tokens_details/cached_tokens",
                "/response/usage/prompt_tokens_details/cached_tokens",
            ],
        )
    });
    TokenUsage {
        input_tokens: subtract_cache_from_inclusive_input(
            raw_input_tokens,
            cache_read_tokens,
            None,
        ),
        output_tokens: first_u64_at_paths(usage, &["/output_tokens", "/completion_tokens"])
            .or_else(|| {
                first_u64_at_paths(
                    value,
                    &[
                        "/usage/output_tokens",
                        "/usage/completion_tokens",
                        "/response/usage/output_tokens",
                        "/response/usage/completion_tokens",
                    ],
                )
            }),
        cache_read_tokens,
        cache_creation_tokens: None,
    }
}

fn gemini_usage(value: &Value) -> TokenUsage {
    let usage = value.pointer("/usageMetadata").unwrap_or(value);
    let input_tokens =
        first_u64_at_paths(usage, &["/promptTokenCount", "/prompt_tokens"]).or_else(|| {
            first_u64_at_paths(
                value,
                &[
                    "/usageMetadata/promptTokenCount",
                    "/response/usageMetadata/promptTokenCount",
                ],
            )
        });
    let output_tokens = first_u64_at_paths(
        usage,
        &[
            "/candidatesTokenCount",
            "/completion_tokens",
            "/output_tokens",
        ],
    )
    .or_else(|| {
        first_u64_at_paths(
            value,
            &[
                "/usageMetadata/candidatesTokenCount",
                "/response/usageMetadata/candidatesTokenCount",
            ],
        )
    });

    let cache_read_tokens =
        first_u64_at_paths(usage, &["/cachedContentTokenCount", "/cache_read_tokens"]).or_else(
            || {
                first_u64_at_paths(
                    value,
                    &[
                        "/usageMetadata/cachedContentTokenCount",
                        "/response/usageMetadata/cachedContentTokenCount",
                    ],
                )
            },
        );

    TokenUsage {
        input_tokens: subtract_cache_from_inclusive_input(input_tokens, cache_read_tokens, None),
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens: None,
    }
}

fn first_u64_at_paths(value: &Value, paths: &[&str]) -> Option<u64> {
    paths
        .iter()
        .find_map(|path| value.pointer(path).and_then(Value::as_u64))
}

fn first_i64_at_paths(value: &Value, paths: &[&str]) -> Option<i64> {
    paths
        .iter()
        .find_map(|path| value.pointer(path).and_then(Value::as_i64))
}

fn subtract_cache_from_inclusive_input(
    input_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_creation_tokens: Option<u64>,
) -> Option<u64> {
    input_tokens.map(|tokens| {
        tokens
            .saturating_sub(cache_read_tokens.unwrap_or(0))
            .saturating_sub(cache_creation_tokens.unwrap_or(0))
    })
}

fn max_option(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn take_sse_block(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
        let block = buffer.drain(..index + 4).collect::<Vec<_>>();
        return Some(block);
    }
    if let Some(index) = buffer.windows(2).position(|window| window == b"\n\n") {
        let block = buffer.drain(..index + 2).collect::<Vec<_>>();
        return Some(block);
    }
    None
}

fn parse_sse_data_block(block: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(block);
    let mut data_lines = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.strip_prefix(' ').unwrap_or(data);
        if data.trim() == "[DONE]" {
            return None;
        }
        data_lines.push(data);
    }
    if data_lines.is_empty() {
        return None;
    }
    let data = data_lines.join("\n");
    serde_json::from_str::<Value>(&data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_stream_usage_with_cache_tokens() {
        let body = br#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":120,"output_tokens":1,"cache_read_input_tokens":40,"cache_creation_input_tokens":8}}}

event: message_delta
data: {"type":"message_delta","usage":{"output_tokens":35}}

"#;

        let usage = from_response_body(GatewayCliKey::Claude, body);

        assert_eq!(usage.input_tokens, Some(120));
        assert_eq!(usage.output_tokens, Some(35));
        assert_eq!(usage.cache_read_tokens, Some(40));
        assert_eq!(usage.cache_creation_tokens, Some(8));
        assert_eq!(usage.total_tokens(), Some(203));
    }

    #[test]
    fn parses_moonshot_anthropic_cached_tokens_as_cache_read() {
        let usage = from_response_body_with_provider_type(
            GatewayCliKey::Claude,
            Some("moonshot"),
            br#"{"usage":{"input_tokens":100,"output_tokens":10,"cached_tokens":80}}"#,
        );

        assert_eq!(usage.input_tokens, Some(20));
        assert_eq!(usage.output_tokens, Some(10));
        assert_eq!(usage.cache_read_tokens, Some(80));
        assert_eq!(usage.total_tokens(), Some(110));
    }

    #[test]
    fn parses_moonshot_negative_input_cache_discount() {
        let usage = from_response_body_with_provider_type(
            GatewayCliKey::Claude,
            Some("kimi"),
            br#"{"usage":{"input_tokens":-40,"output_tokens":10,"cached_tokens":80}}"#,
        );

        assert_eq!(usage.input_tokens, Some(40));
        assert_eq!(usage.output_tokens, Some(10));
        assert_eq!(usage.cache_read_tokens, Some(80));
        assert_eq!(usage.total_tokens(), Some(130));
    }

    #[test]
    fn default_anthropic_usage_keeps_input_tokens_as_fresh_input() {
        let usage = from_response_body_with_provider_type(
            GatewayCliKey::Claude,
            Some("anthropic"),
            br#"{"usage":{"input_tokens":100,"output_tokens":10,"cached_tokens":80}}"#,
        );

        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(10));
        assert_eq!(usage.cache_read_tokens, Some(80));
        assert_eq!(usage.total_tokens(), Some(190));
    }

    #[test]
    fn parses_openai_stream_usage_with_cached_prompt_tokens() {
        let body = br#"data: {"type":"response.completed","response":{"usage":{"input_tokens":90,"output_tokens":12,"input_tokens_details":{"cached_tokens":30}}}}

data: [DONE]

"#;

        let usage = from_response_body(GatewayCliKey::Codex, body);

        assert_eq!(usage.input_tokens, Some(60));
        assert_eq!(usage.output_tokens, Some(12));
        assert_eq!(usage.cache_read_tokens, Some(30));
        assert_eq!(usage.cache_creation_tokens, None);
        assert_eq!(usage.total_tokens(), Some(102));
    }

    #[test]
    fn parses_gemini_json_usage_metadata() {
        let usage = from_response_body(
            GatewayCliKey::Gemini,
            br#"{"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":7,"cachedContentTokenCount":3}}"#,
        );

        assert_eq!(usage.input_tokens, Some(7));
        assert_eq!(usage.output_tokens, Some(7));
        assert_eq!(usage.cache_read_tokens, Some(3));
        assert_eq!(usage.total_tokens(), Some(17));
    }

    #[test]
    fn gemini_usage_does_not_infer_output_from_total_tokens() {
        let usage = from_response_body(
            GatewayCliKey::Gemini,
            br#"{"usageMetadata":{"promptTokenCount":10,"totalTokenCount":17}}"#,
        );

        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, None);
        assert_eq!(usage.total_tokens(), Some(10));
    }

    #[test]
    fn sse_usage_collector_keeps_buffer_bounded() {
        let mut collector = SseUsageCollector::default();
        collector.push_chunk(
            GatewayCliKey::Claude,
            &vec![b'a'; MAX_SSE_USAGE_BUFFER_BYTES + 1],
        );
        assert!(collector.buffer.is_empty());

        collector.push_chunk(
            GatewayCliKey::Claude,
            &vec![b'a'; MAX_SSE_USAGE_BUFFER_BYTES - 1],
        );
        assert_eq!(collector.buffer.len(), MAX_SSE_USAGE_BUFFER_BYTES - 1);

        collector.push_chunk(GatewayCliKey::Claude, b"aa");
        assert_eq!(collector.buffer.len(), 2);
    }
}
