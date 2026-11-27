use super::super::providers::UpstreamProvider;
use super::super::routes::GatewayRoute;
use crate::coding::proxy_gateway::transformer::{
    convert_error_response_body as convert_protocol_error_response_body,
    convert_responses_compact_request_body_to_target,
    convert_target_response_body_to_responses_compact, AiProtocol, ConversionContext,
    ConversionRoute, ProtocolConversionError,
};
use crate::coding::proxy_gateway::types::GatewayCliKey;
use serde_json::{json, Value};

pub(in crate::coding::proxy_gateway::runtime) const CODEX_RESPONSES_COMPACT_COMPAT_HEADER: &str =
    "X-Gateway-Compat";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CodexResponsesCompactMode {
    NotCompact,
    NativeResponses,
    OpenAiChatFallback,
    AnthropicMessagesFallback,
    GeminiNativeFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::coding::proxy_gateway::runtime) struct CodexResponsesCompactCompat {
    mode: CodexResponsesCompactMode,
}

impl CodexResponsesCompactCompat {
    pub(in crate::coding::proxy_gateway::runtime) fn none() -> Self {
        Self {
            mode: CodexResponsesCompactMode::NotCompact,
        }
    }

    pub(in crate::coding::proxy_gateway::runtime) fn new(
        route: &GatewayRoute,
        provider: &UpstreamProvider,
    ) -> Self {
        if !is_codex_responses_compact_route(route) {
            return Self::none();
        }

        let mode = match provider.target_protocol {
            AiProtocol::OpenAiResponses => CodexResponsesCompactMode::NativeResponses,
            AiProtocol::OpenAiChat => CodexResponsesCompactMode::OpenAiChatFallback,
            AiProtocol::AnthropicMessages => CodexResponsesCompactMode::AnthropicMessagesFallback,
            AiProtocol::GeminiNative => CodexResponsesCompactMode::GeminiNativeFallback,
        };

        Self { mode }
    }

    #[cfg(test)]
    pub(super) fn mode(self) -> CodexResponsesCompactMode {
        self.mode
    }

    pub(in crate::coding::proxy_gateway::runtime) fn is_compact(self) -> bool {
        self.mode != CodexResponsesCompactMode::NotCompact
    }

    #[cfg(test)]
    pub(in crate::coding::proxy_gateway::runtime) fn is_fallback(self) -> bool {
        matches!(
            self.mode,
            CodexResponsesCompactMode::OpenAiChatFallback
                | CodexResponsesCompactMode::AnthropicMessagesFallback
                | CodexResponsesCompactMode::GeminiNativeFallback
        )
    }

    pub(in crate::coding::proxy_gateway::runtime) fn conversion_route(
        self,
    ) -> Option<ConversionRoute> {
        match self.mode {
            CodexResponsesCompactMode::OpenAiChatFallback => Some(ConversionRoute::new(
                AiProtocol::OpenAiResponses,
                AiProtocol::OpenAiChat,
            )),
            CodexResponsesCompactMode::AnthropicMessagesFallback => Some(ConversionRoute::new(
                AiProtocol::OpenAiResponses,
                AiProtocol::AnthropicMessages,
            )),
            CodexResponsesCompactMode::GeminiNativeFallback => Some(ConversionRoute::new(
                AiProtocol::OpenAiResponses,
                AiProtocol::GeminiNative,
            )),
            CodexResponsesCompactMode::NotCompact | CodexResponsesCompactMode::NativeResponses => {
                None
            }
        }
    }

    pub(in crate::coding::proxy_gateway::runtime) fn should_use_codex_chat_history(self) -> bool {
        self.mode == CodexResponsesCompactMode::OpenAiChatFallback
    }

    pub(in crate::coding::proxy_gateway::runtime) fn warning(self) -> Option<&'static str> {
        match self.mode {
            CodexResponsesCompactMode::OpenAiChatFallback => Some(
                "codex_responses_compact_fallback: /responses/compact was converted through OpenAI Chat fallback",
            ),
            CodexResponsesCompactMode::AnthropicMessagesFallback => Some(
                "codex_responses_compact_fallback: /responses/compact was converted through Anthropic Messages fallback; compact-specific fields may be lossy",
            ),
            CodexResponsesCompactMode::GeminiNativeFallback => Some(
                "codex_responses_compact_fallback: /responses/compact was converted through Gemini Native fallback; compact-specific fields may be lossy",
            ),
            CodexResponsesCompactMode::NotCompact | CodexResponsesCompactMode::NativeResponses => {
                None
            }
        }
    }

    pub(in crate::coding::proxy_gateway::runtime) fn header_value(self) -> Option<&'static str> {
        match self.mode {
            CodexResponsesCompactMode::OpenAiChatFallback => {
                Some("codex-responses-compact-openai-chat")
            }
            CodexResponsesCompactMode::AnthropicMessagesFallback => {
                Some("codex-responses-compact-anthropic")
            }
            CodexResponsesCompactMode::GeminiNativeFallback => {
                Some("codex-responses-compact-gemini")
            }
            CodexResponsesCompactMode::NotCompact | CodexResponsesCompactMode::NativeResponses => {
                None
            }
        }
    }

    pub(in crate::coding::proxy_gateway::runtime) fn validate_request(
        self,
        route: &GatewayRoute,
        body: &[u8],
    ) -> Result<(), String> {
        if self.is_compact() && compact_declares_streaming(route, body) {
            return Err(
                "Codex Responses compact compatibility does not support streaming requests"
                    .to_string(),
            );
        }
        Ok(())
    }

    pub(in crate::coding::proxy_gateway::runtime) fn convert_request_body(
        self,
        body: &[u8],
    ) -> Result<(Vec<u8>, ConversionContext), ProtocolConversionError> {
        let target = self
            .target_protocol()
            .unwrap_or(AiProtocol::OpenAiResponses);
        let converted = convert_responses_compact_request_body_to_target(target, body)?;
        Ok((converted.body, converted.context))
    }

    pub(in crate::coding::proxy_gateway::runtime) fn convert_response_body(
        self,
        body: &[u8],
        context: Option<&ConversionContext>,
    ) -> Result<Vec<u8>, ProtocolConversionError> {
        let source = self
            .target_protocol()
            .unwrap_or(AiProtocol::OpenAiResponses);
        convert_target_response_body_to_responses_compact(source, body, context)
    }

    pub(in crate::coding::proxy_gateway::runtime) fn convert_error_response_body(
        self,
        body: &[u8],
    ) -> Vec<u8> {
        if !self.is_compact() {
            return body.to_vec();
        }

        let responses_error_body = match self.target_protocol() {
            Some(AiProtocol::OpenAiChat)
            | Some(AiProtocol::AnthropicMessages)
            | Some(AiProtocol::GeminiNative) => convert_protocol_error_response_body(
                ConversionRoute::new(
                    self.target_protocol()
                        .unwrap_or(AiProtocol::OpenAiResponses),
                    AiProtocol::OpenAiResponses,
                ),
                body,
            ),
            Some(AiProtocol::OpenAiResponses) | None => body.to_vec(),
        };
        openai_responses_error_body(&responses_error_body, "api_error", Value::Null)
    }

    pub(in crate::coding::proxy_gateway::runtime) fn request_schema_error_value(
        self,
        message: &str,
    ) -> Option<Value> {
        self.is_compact().then(|| {
            openai_responses_error_value(
                message.to_string(),
                "invalid_request_error".to_string(),
                Value::Null,
                json!("gateway_request_schema_rejected"),
            )
        })
    }

    fn target_protocol(self) -> Option<AiProtocol> {
        match self.mode {
            CodexResponsesCompactMode::NativeResponses => Some(AiProtocol::OpenAiResponses),
            CodexResponsesCompactMode::OpenAiChatFallback => Some(AiProtocol::OpenAiChat),
            CodexResponsesCompactMode::AnthropicMessagesFallback => {
                Some(AiProtocol::AnthropicMessages)
            }
            CodexResponsesCompactMode::GeminiNativeFallback => Some(AiProtocol::GeminiNative),
            CodexResponsesCompactMode::NotCompact => None,
        }
    }
}

fn openai_responses_error_body(body: &[u8], default_type: &str, default_code: Value) -> Vec<u8> {
    let fallback_message = String::from_utf8_lossy(body).trim().to_string();
    let value = serde_json::from_slice::<Value>(body)
        .unwrap_or_else(|_| json!({ "message": fallback_message }));
    let message = extract_error_message(&value)
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| "Upstream returned an error response".to_string());
    let error_type = extract_error_string(
        &value,
        &["/error/type", "/error/status", "/type", "/status"],
    )
    .unwrap_or_else(|| default_type.to_string());
    let param = extract_error_value(&value, &["/error/param", "/param"]).unwrap_or(Value::Null);
    let code = extract_error_value(&value, &["/error/code", "/code"]).unwrap_or(default_code);
    let wrapped = openai_responses_error_value(message, error_type, param, code);
    serde_json::to_vec(&wrapped).unwrap_or_else(|_| body.to_vec())
}

fn openai_responses_error_value(
    message: String,
    error_type: String,
    param: Value,
    code: Value,
) -> Value {
    json!({
        "error": {
            "message": message,
            "type": error_type,
            "param": param,
            "code": code,
        }
    })
}

fn extract_error_message(value: &Value) -> Option<String> {
    if let Some(message) = value.as_str().filter(|message| !message.trim().is_empty()) {
        return Some(message.to_string());
    }

    for pointer in [
        "/error/message",
        "/message",
        "/detail",
        "/msg",
        "/status_msg",
        "/base_resp/status_msg",
    ] {
        if let Some(message) = value
            .pointer(pointer)
            .and_then(Value::as_str)
            .filter(|message| !message.trim().is_empty())
        {
            return Some(message.to_string());
        }
    }

    value
        .get("error")
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .map(ToString::to_string)
}

fn extract_error_string(value: &Value, pointers: &[&str]) -> Option<String> {
    for pointer in pointers {
        if let Some(text) = value
            .pointer(pointer)
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
        {
            return Some(text.to_string());
        }
    }
    None
}

fn extract_error_value(value: &Value, pointers: &[&str]) -> Option<Value> {
    for pointer in pointers {
        if let Some(error_value) = value
            .pointer(pointer)
            .filter(|error_value| !error_value.is_null())
        {
            return Some(error_value.clone());
        }
    }
    None
}

pub(in crate::coding::proxy_gateway::runtime) fn is_codex_responses_compact_route(
    route: &GatewayRoute,
) -> bool {
    matches!(route.cli_key, GatewayCliKey::Codex | GatewayCliKey::Grok)
        && matches!(
            route.forwarded_path.as_str(),
            "/v1/responses/compact" | "/responses/compact"
        )
}

fn compact_declares_streaming(route: &GatewayRoute, body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("stream").and_then(Value::as_bool))
        .unwrap_or(false)
        || route.query.as_deref().is_some_and(query_declares_streaming)
}

fn query_declares_streaming(query: &str) -> bool {
    query.split('&').any(|pair| {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or_default();
        let value = parts.next().unwrap_or_default();
        key.eq_ignore_ascii_case("stream") && value.eq_ignore_ascii_case("true")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::proxy_gateway::runtime::providers::{
        ProviderAuthStrategy, UpstreamModelMapping,
    };
    use crate::coding::proxy_gateway::types::ProviderGatewayMeta;

    fn route(path: &str) -> GatewayRoute {
        GatewayRoute {
            cli_key: GatewayCliKey::Codex,
            route_name: "test",
            forwarded_path: path.to_string(),
            query: None,
        }
    }

    fn provider(target_protocol: AiProtocol) -> UpstreamProvider {
        UpstreamProvider {
            supports_websockets: None,
            cli_key: GatewayCliKey::Codex,
            id: "p1".to_string(),
            name: "Provider".to_string(),
            base_url: "https://api.example.com".to_string(),
            api_key: "key".to_string(),
            target_protocol,
            auth_strategy: ProviderAuthStrategy::Bearer,
            is_full_url: false,
            sort_index: Some(0),
            meta: ProviderGatewayMeta::default(),
            model_mapping: UpstreamModelMapping::default(),

            plugin_provider: None,
        }
    }

    #[test]
    fn selects_modes_by_target_protocol() {
        assert_eq!(
            CodexResponsesCompactCompat::new(
                &route("/v1/responses/compact"),
                &provider(AiProtocol::OpenAiResponses),
            )
            .mode(),
            CodexResponsesCompactMode::NativeResponses
        );
        assert_eq!(
            CodexResponsesCompactCompat::new(
                &route("/responses/compact"),
                &provider(AiProtocol::OpenAiChat),
            )
            .mode(),
            CodexResponsesCompactMode::OpenAiChatFallback
        );
        assert_eq!(
            CodexResponsesCompactCompat::new(
                &route("/responses/compact"),
                &provider(AiProtocol::AnthropicMessages),
            )
            .conversion_route(),
            Some(ConversionRoute::new(
                AiProtocol::OpenAiResponses,
                AiProtocol::AnthropicMessages
            ))
        );
        assert_eq!(
            CodexResponsesCompactCompat::new(
                &route("/responses/compact"),
                &provider(AiProtocol::GeminiNative),
            )
            .conversion_route(),
            Some(ConversionRoute::new(
                AiProtocol::OpenAiResponses,
                AiProtocol::GeminiNative
            ))
        );
    }

    #[test]
    fn rejects_explicit_streaming() {
        let compat = CodexResponsesCompactCompat::new(
            &route("/v1/responses/compact"),
            &provider(AiProtocol::OpenAiChat),
        );

        assert!(compat
            .validate_request(&route("/v1/responses/compact"), br#"{"stream":true}"#)
            .is_err());

        let mut route = route("/v1/responses/compact");
        route.query = Some("stream=true".to_string());
        assert!(compat.validate_request(&route, br#"{}"#).is_err());
    }

    #[test]
    fn converts_compact_request_to_fallback_body_shapes() {
        let body = br#"{"model":"model-a","input":[{"role":"user","content":[{"type":"input_text","text":"compact this"}]}]}"#;

        let chat_compat = CodexResponsesCompactCompat::new(
            &route("/v1/responses/compact"),
            &provider(AiProtocol::OpenAiChat),
        );
        let (chat_body, chat_context) = chat_compat.convert_request_body(body).unwrap();
        let chat_value: Value = serde_json::from_slice(&chat_body).unwrap();
        assert!(chat_context.is_empty());
        assert!(chat_value
            .get("messages")
            .and_then(Value::as_array)
            .is_some());
        assert_eq!(chat_value["stream"], false);

        let anthropic_compat = CodexResponsesCompactCompat::new(
            &route("/v1/responses/compact"),
            &provider(AiProtocol::AnthropicMessages),
        );
        let (anthropic_body, anthropic_context) =
            anthropic_compat.convert_request_body(body).unwrap();
        let anthropic_value: Value = serde_json::from_slice(&anthropic_body).unwrap();
        assert!(anthropic_context.is_empty());
        assert!(anthropic_value
            .get("messages")
            .and_then(Value::as_array)
            .is_some());
        assert!(anthropic_value.get("max_tokens").is_some());

        let gemini_compat = CodexResponsesCompactCompat::new(
            &route("/v1/responses/compact"),
            &provider(AiProtocol::GeminiNative),
        );
        let (gemini_body, gemini_context) = gemini_compat.convert_request_body(body).unwrap();
        let gemini_value: Value = serde_json::from_slice(&gemini_body).unwrap();
        assert!(gemini_context.is_empty());
        assert!(gemini_value
            .get("contents")
            .and_then(Value::as_array)
            .is_some());
    }

    #[test]
    fn converts_fallback_response_as_compaction() {
        let compat = CodexResponsesCompactCompat::new(
            &route("/v1/responses/compact"),
            &provider(AiProtocol::OpenAiChat),
        );

        let body = compat
            .convert_response_body(
                br#"{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}]}"#,
                None,
            )
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["object"], "response.compaction");
        assert_eq!(value["status"], "completed");
        assert_eq!(value["output"][0]["type"], "message");
    }

    #[test]
    fn converts_fallback_error_response_to_openai_shape() {
        let compat = CodexResponsesCompactCompat::new(
            &route("/v1/responses/compact"),
            &provider(AiProtocol::GeminiNative),
        );

        let body = compat.convert_error_response_body(
            br#"{"error":{"code":403,"message":"blocked","status":"PERMISSION_DENIED"}}"#,
        );
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["error"]["message"], "blocked");
        assert_eq!(value["error"]["type"], "PERMISSION_DENIED");
        assert_eq!(value["error"]["param"], Value::Null);
        assert_eq!(value["error"]["code"], 403);
    }

    #[test]
    fn wraps_plain_text_error_response_to_openai_shape() {
        let compat = CodexResponsesCompactCompat::new(
            &route("/v1/responses/compact"),
            &provider(AiProtocol::OpenAiChat),
        );

        let body = compat.convert_error_response_body(b"upstream exploded");
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["error"]["message"], "upstream exploded");
        assert_eq!(value["error"]["type"], "api_error");
        assert_eq!(value["error"]["param"], Value::Null);
        assert_eq!(value["error"]["code"], Value::Null);
    }

    #[test]
    fn request_schema_error_uses_openai_error_shape() {
        let compat = CodexResponsesCompactCompat::new(
            &route("/v1/responses/compact"),
            &provider(AiProtocol::OpenAiChat),
        );

        let value = compat
            .request_schema_error_value("streaming is unsupported")
            .unwrap();

        assert_eq!(value["error"]["message"], "streaming is unsupported");
        assert_eq!(value["error"]["type"], "invalid_request_error");
        assert_eq!(value["error"]["param"], Value::Null);
        assert_eq!(value["error"]["code"], "gateway_request_schema_rejected");
    }
}
