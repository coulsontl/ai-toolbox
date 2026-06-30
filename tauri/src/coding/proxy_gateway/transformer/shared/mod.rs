pub mod error;
pub mod messages;
pub mod signature;

use super::llm::ApiFormat;

pub(crate) use error::extract_error_message;
pub use messages::{
    content_text, json_string, message_parts, stop_from_value, stop_to_value, tool_arguments_value,
    tool_choice_from_anthropic, tool_choice_from_gemini, tool_choice_from_openai,
    tool_choice_to_anthropic, tool_choice_to_openai, tool_choice_to_responses,
};

pub(crate) fn should_emit_openai_request_metadata(api_format: Option<ApiFormat>) -> bool {
    matches!(
        api_format,
        Some(
            ApiFormat::OpenAiChatCompletions
                | ApiFormat::OpenAiResponses
                | ApiFormat::OpenAiResponsesCompact
        )
    )
}
