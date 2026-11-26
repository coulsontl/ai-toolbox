use crate::coding::proxy_gateway::transformer::AiProtocol;
use crate::coding::proxy_gateway::types::ProviderGatewayMeta;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::coding::proxy_gateway::runtime) enum ProviderBodyCompat {
    AnthropicBedrock,
    AnthropicVertex,
    DeepSeek,
    Moonshot,
    Zai,
    Doubao,
    Xai,
    Longcat,
    ModelScope,
    Bailian,
    Mimo,
    OpenRouter,
    GeminiVertex,
    CodexOfficial,
    Copilot,
    Ollama,
    OpenCodeGo,
}

impl ProviderBodyCompat {
    pub(in crate::coding::proxy_gateway::runtime) fn from_provider_meta(
        meta: Option<&ProviderGatewayMeta>,
        target_protocol: AiProtocol,
    ) -> Option<Self> {
        let meta = meta?;
        Self::from_provider_type(meta.provider_type.as_deref(), target_protocol).or_else(|| {
            (target_protocol == AiProtocol::OpenAiChat
                && meta.api_format.as_deref().is_some_and(is_ollama_api_format))
            .then_some(Self::Ollama)
        })
    }

    pub(in crate::coding::proxy_gateway::runtime) fn from_provider_type(
        provider_type: Option<&str>,
        target_protocol: AiProtocol,
    ) -> Option<Self> {
        let normalized = provider_type?.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "bedrock" | "anthropic-bedrock" | "aws-bedrock"
                if target_protocol == AiProtocol::AnthropicMessages =>
            {
                Some(Self::AnthropicBedrock)
            }
            "vertex" | "anthropic-vertex" | "claude-vertex"
                if target_protocol == AiProtocol::AnthropicMessages =>
            {
                Some(Self::AnthropicVertex)
            }
            "deepseek" => Some(Self::DeepSeek),
            "moonshot" | "kimi" => Some(Self::Moonshot),
            "zai" | "zhipu" | "glm" | "chatglm" | "bigmodel" | "big-model" => Some(Self::Zai),
            "doubao" | "doubaoseed" | "doubao-seed" | "volces" => Some(Self::Doubao),
            "xai" | "x-ai" | "grok" => Some(Self::Xai),
            "longcat" | "long-cat" => Some(Self::Longcat),
            "modelscope" | "model-scope" => Some(Self::ModelScope),
            "bailian" | "dashscope" | "aliyun" => Some(Self::Bailian),
            "mimo" | "xiaomimimo" | "xiaomi-mimo" => Some(Self::Mimo),
            "openrouter" | "open-router" => Some(Self::OpenRouter),
            "codex" | "openai-codex" | "chatgpt-codex" | "codex-official"
                if target_protocol == AiProtocol::OpenAiResponses =>
            {
                Some(Self::CodexOfficial)
            }
            "copilot" | "github-copilot" | "githubcopilot" => Some(Self::Copilot),
            "opencode-go" | "opencodego" | "opencode-zen" => Some(Self::OpenCodeGo),
            "ollama" | "ollama-chat" | "ollamachat"
                if target_protocol == AiProtocol::OpenAiChat =>
            {
                Some(Self::Ollama)
            }
            "vertex" | "googlevertex" | "google-vertex" | "geminivertex" | "gemini-vertex"
                if target_protocol == AiProtocol::GeminiNative =>
            {
                Some(Self::GeminiVertex)
            }
            _ => None,
        }
    }
}

fn is_ollama_api_format(value: &str) -> bool {
    matches!(
        value
            .trim()
            .to_ascii_lowercase()
            .replace(['/', '-'], "_")
            .as_str(),
        "ollama" | "ollama_chat"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_vertex_provider_kind_requires_gemini_native_target() {
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("google-vertex"), AiProtocol::GeminiNative),
            Some(ProviderBodyCompat::GeminiVertex)
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("google-vertex"), AiProtocol::OpenAiChat),
            None
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(
                Some("google-vertex"),
                AiProtocol::AnthropicMessages
            ),
            None
        );
        assert_eq!(
            ProviderBodyCompat::from_provider_type(Some("vertex"), AiProtocol::AnthropicMessages),
            Some(ProviderBodyCompat::AnthropicVertex)
        );
    }

    #[test]
    fn opencode_go_provider_kind_matches_explicit_aliases() {
        for alias in ["opencode-go", "opencodego", "opencode_go", "opencode-zen"] {
            assert_eq!(
                ProviderBodyCompat::from_provider_type(Some(alias), AiProtocol::OpenAiChat),
                Some(ProviderBodyCompat::OpenCodeGo),
                "alias={alias}"
            );
        }
    }
}
