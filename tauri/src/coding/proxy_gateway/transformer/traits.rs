use super::error::ProtocolConversionError;
use super::llm::{Request, Response};
use super::types::AiProtocol;
use serde_json::Value;

pub trait InboundTransformer {
    #[allow(dead_code)]
    fn protocol(&self) -> AiProtocol;
    fn request_to_llm(&self, body: Value) -> Result<Request, ProtocolConversionError>;
    fn response_from_llm(&self, response: Response) -> Result<Value, ProtocolConversionError>;
    fn error_from_llm(&self, error: Value) -> Value;
}

pub trait OutboundTransformer {
    #[allow(dead_code)]
    fn protocol(&self) -> AiProtocol;
    fn request_from_llm(&self, request: Request) -> Result<Value, ProtocolConversionError>;
    fn response_to_llm(&self, body: Value) -> Result<Response, ProtocolConversionError>;
    fn error_to_llm(&self, error: Value) -> Value;
}
