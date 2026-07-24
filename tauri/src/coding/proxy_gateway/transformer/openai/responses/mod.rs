mod request;
mod response;
mod shared;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use crate::coding::proxy_gateway::transformer::error::ProtocolConversionError;
use crate::coding::proxy_gateway::transformer::llm::{Request, Response};
use crate::coding::proxy_gateway::transformer::shared::{
    extract_error_code, extract_error_message, extract_error_param, extract_error_type,
};
use crate::coding::proxy_gateway::transformer::traits::{InboundTransformer, OutboundTransformer};
use crate::coding::proxy_gateway::transformer::types::AiProtocol;
use serde_json::{json, Value};

pub use request::{
    llm_request_to_responses, llm_request_to_responses_compact, responses_compact_request_to_llm,
    responses_request_to_llm,
};
pub use response::{
    llm_response_to_responses, llm_response_to_responses_compact, responses_compact_response_to_llm,
    responses_response_to_llm,
};

pub struct OpenAiResponsesInbound;
pub struct OpenAiResponsesOutbound;

impl InboundTransformer for OpenAiResponsesInbound {
    fn protocol(&self) -> AiProtocol {
        AiProtocol::OpenAiResponses
    }

    fn request_to_llm(&self, body: Value) -> Result<Request, ProtocolConversionError> {
        Ok(responses_request_to_llm(body))
    }

    fn response_from_llm(&self, response: Response) -> Result<Value, ProtocolConversionError> {
        Ok(llm_response_to_responses(response))
    }

    fn error_from_llm(&self, error: Value) -> Value {
        let message = extract_error_message(&error)
            .unwrap_or_else(|| "Protocol conversion error".to_string());
        json!({
            "error": {
                "message": message,
                "type": extract_error_type(&error).unwrap_or_else(|| "api_error".to_string()),
                "param": extract_error_param(&error).unwrap_or(Value::Null),
                "code": extract_error_code(&error).unwrap_or(Value::Null)
            }
        })
    }
}

impl OutboundTransformer for OpenAiResponsesOutbound {
    fn protocol(&self) -> AiProtocol {
        AiProtocol::OpenAiResponses
    }

    fn request_from_llm(&self, request: Request) -> Result<Value, ProtocolConversionError> {
        Ok(llm_request_to_responses(request))
    }

    fn response_to_llm(&self, body: Value) -> Result<Response, ProtocolConversionError> {
        Ok(responses_response_to_llm(body))
    }

    fn error_to_llm(&self, error: Value) -> Value {
        error
    }
}
