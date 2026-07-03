//! Reusable AI protocol conversion helpers for Proxy Gateway.
//!
//! The module is deliberately independent from database, Tauri commands and
//! provider storage. Runtime code supplies a source/target protocol and the
//! module only rewrites protocol payloads.

mod anthropic;
mod error;
mod gemini;
mod kernel;
mod llm;
mod openai;
mod shared;
mod sse;
mod stream;
mod traits;
mod types;

pub use error::ProtocolConversionError;
pub use kernel::{
    convert_error_response_body, convert_request_body, convert_request_body_with_context,
    convert_request_value, convert_response_body, convert_response_body_with_context,
    convert_response_value,
};
pub use kernel::{convert_sse_stream, convert_sse_stream_with_context, ConversionContext};
pub use types::{AiProtocol, ConversionRoute};
