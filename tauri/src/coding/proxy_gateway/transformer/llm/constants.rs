#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestType {
    Chat,
    Compact,
    Completion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormat {
    OpenAiChatCompletions,
    OpenAiResponses,
    OpenAiResponsesCompact,
    AnthropicMessages,
    GeminiContents,
}

impl ApiFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiChatCompletions => "openai/chat_completions",
            Self::OpenAiResponses => "openai/responses",
            Self::OpenAiResponsesCompact => "openai/responses_compact",
            Self::AnthropicMessages => "anthropic/messages",
            Self::GeminiContents => "gemini/contents",
        }
    }
}

pub const TOOL_TYPE_FUNCTION: &str = "function";
pub const TOOL_TYPE_GOOGLE_SEARCH: &str = "google_search";
pub const TOOL_TYPE_GOOGLE_CODE_EXECUTION: &str = "google_code_execution";
pub const TOOL_TYPE_GOOGLE_URL_CONTEXT: &str = "google_url_context";
pub const TOOL_TYPE_RESPONSES_CUSTOM_TOOL: &str = "responses_custom_tool";
