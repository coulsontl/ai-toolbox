use super::types::ConversionRoute;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolConversionError {
    InvalidJson(String),
    UnsupportedRoute(ConversionRoute),
    Transform(String),
}

impl fmt::Display for ProtocolConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(message) => write!(formatter, "Invalid JSON payload: {message}"),
            Self::UnsupportedRoute(route) => write!(
                formatter,
                "Unsupported protocol conversion route: {} -> {}",
                route.source.as_str(),
                route.target.as_str()
            ),
            Self::Transform(message) => write!(formatter, "Protocol conversion failed: {message}"),
        }
    }
}

impl std::error::Error for ProtocolConversionError {}
