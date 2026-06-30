use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProtocol {
    AnthropicMessages,
    OpenAiResponses,
    OpenAiChat,
    GeminiNative,
}

impl AiProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AnthropicMessages => "anthropic_messages",
            Self::OpenAiResponses => "openai_responses",
            Self::OpenAiChat => "openai_chat",
            Self::GeminiNative => "gemini_native",
        }
    }

    pub fn from_api_format(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase().replace(['/', '-'], "_");
        match normalized.as_str() {
            "anthropic" | "anthropic_messages" | "claude" | "claude_messages" => {
                Some(Self::AnthropicMessages)
            }
            "openai_responses" | "responses" | "response" => Some(Self::OpenAiResponses),
            "openai_chat" | "chat_completions" | "chat" => Some(Self::OpenAiChat),
            "gemini_native" | "gemini" => Some(Self::GeminiNative),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AiProtocol;

    #[test]
    fn api_format_aliases_accept_slash_and_dash() {
        assert_eq!(
            AiProtocol::from_api_format("anthropic/messages"),
            Some(AiProtocol::AnthropicMessages)
        );
        assert_eq!(
            AiProtocol::from_api_format("openai/responses"),
            Some(AiProtocol::OpenAiResponses)
        );
        assert_eq!(
            AiProtocol::from_api_format("openai-chat"),
            Some(AiProtocol::OpenAiChat)
        );
        assert_eq!(
            AiProtocol::from_api_format("gemini-native"),
            Some(AiProtocol::GeminiNative)
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConversionRoute {
    pub source: AiProtocol,
    pub target: AiProtocol,
}

impl ConversionRoute {
    pub fn new(source: AiProtocol, target: AiProtocol) -> Self {
        Self { source, target }
    }

    pub fn identity(self) -> bool {
        self.source == self.target
    }

    pub fn reverse(self) -> Self {
        Self {
            source: self.target,
            target: self.source,
        }
    }
}
