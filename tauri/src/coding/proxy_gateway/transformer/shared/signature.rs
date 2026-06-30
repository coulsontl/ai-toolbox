use base64::Engine;
use serde_json::{json, Value};

pub const GEMINI_THOUGHT_SIGNATURE_METADATA_KEY: &str = "gemini_thought_signature";
pub const DEFAULT_GEMINI_THOUGHT_SIGNATURE: &str =
    "Y29udGV4dF9lbmdpbmVlcmluZ19pc190aGVfd2F5X3RvX2dv";

const ANTHROPIC_MARKER: &str = "ai-toolbox.sig.anthropic:";
const GEMINI_MARKER: &str = "ai-toolbox.sig.gemini:";
const OPENAI_RESPONSES_MARKER: &str = "ai-toolbox.sig.openai_responses:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureProvider {
    Anthropic,
    Gemini,
    OpenAiResponses,
    Unknown,
}

impl SignatureProvider {
    fn marker(self) -> Option<&'static str> {
        match self {
            Self::Anthropic => Some(ANTHROPIC_MARKER),
            Self::Gemini => Some(GEMINI_MARKER),
            Self::OpenAiResponses => Some(OPENAI_RESPONSES_MARKER),
            Self::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureValue {
    pub provider: SignatureProvider,
    pub value: String,
}

pub fn encode_signature(provider: SignatureProvider, raw: &str) -> String {
    let Some(marker) = provider.marker() else {
        return raw.to_string();
    };
    if parse_marked_signature(raw).is_some_and(|signature| signature.provider == provider) {
        raw.to_string()
    } else {
        format!("{marker}{raw}")
    }
}

pub fn decode_signature_for(provider: SignatureProvider, value: &str) -> Option<String> {
    if provider == SignatureProvider::Unknown || signature_provider(value) != provider {
        return None;
    }
    parse_marked_signature(value)
        .map(|signature| signature.value)
        .or_else(|| Some(value.to_string()))
}

pub fn signature_provider(value: &str) -> SignatureProvider {
    parse_marked_signature(value)
        .map(|signature| signature.provider)
        .unwrap_or_else(|| guess_signature_provider(value))
}

pub fn metadata_signature(raw: &str) -> Value {
    json!(raw)
}

pub fn metadata_signature_raw(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(ToString::to_string)
}

fn parse_marked_signature(value: &str) -> Option<SignatureValue> {
    if let Some(raw) = value.strip_prefix(ANTHROPIC_MARKER) {
        return Some(SignatureValue {
            provider: SignatureProvider::Anthropic,
            value: raw.to_string(),
        });
    }
    if let Some(raw) = value.strip_prefix(GEMINI_MARKER) {
        return Some(SignatureValue {
            provider: SignatureProvider::Gemini,
            value: raw.to_string(),
        });
    }
    if let Some(raw) = value.strip_prefix(OPENAI_RESPONSES_MARKER) {
        return Some(SignatureValue {
            provider: SignatureProvider::OpenAiResponses,
            value: raw.to_string(),
        });
    }
    None
}

pub fn guess_signature_provider(raw: &str) -> SignatureProvider {
    let value = raw.trim_matches('"');
    if value.starts_with("gAAAA") || value.starts_with("gAAA") {
        return SignatureProvider::OpenAiResponses;
    }
    if value.starts_with("EqQ") || value.starts_with("Eqo") || value.starts_with("Eqr") {
        return SignatureProvider::Anthropic;
    }
    if is_standard_base64(value) {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(value) {
            if looks_like_protobuf(&bytes) {
                return SignatureProvider::Gemini;
            }
        }
    }
    SignatureProvider::Unknown
}

fn is_standard_base64(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let mut padding_started = false;
    let mut padding_count = 0;
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => {
                if padding_started {
                    return false;
                }
            }
            b'=' => {
                padding_started = true;
                padding_count += 1;
                if padding_count > 2 {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

fn looks_like_protobuf(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let mut offset = 0;
    while offset < bytes.len() {
        let Some((tag, tag_len)) = read_varint(&bytes[offset..]) else {
            return offset > 0;
        };
        offset += tag_len;
        let wire_type = tag & 0x07;
        let field_number = tag >> 3;
        if field_number == 0 || wire_type == 3 || wire_type == 4 {
            return false;
        }
        match wire_type {
            0 => {
                let Some((_, len)) = read_varint(&bytes[offset..]) else {
                    return false;
                };
                offset += len;
            }
            1 => {
                if offset + 8 > bytes.len() {
                    return false;
                }
                offset += 8;
            }
            2 => {
                let Some((len, len_size)) = read_varint(&bytes[offset..]) else {
                    return false;
                };
                let len = len as usize;
                if offset + len_size + len > bytes.len() {
                    return false;
                }
                offset += len_size + len;
            }
            5 => {
                if offset + 4 > bytes.len() {
                    return false;
                }
                offset += 4;
            }
            _ => return false,
        }
    }
    true
}

fn read_varint(bytes: &[u8]) -> Option<(u64, usize)> {
    let mut result = 0_u64;
    for (index, byte) in bytes.iter().take(10).enumerate() {
        result |= u64::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Some((result, index + 1));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marked_signatures_decode_only_for_matching_provider() {
        let anthropic = encode_signature(SignatureProvider::Anthropic, "EqQabc");
        assert_eq!(
            decode_signature_for(SignatureProvider::Anthropic, &anthropic),
            Some("EqQabc".to_string())
        );
        assert_eq!(
            decode_signature_for(SignatureProvider::OpenAiResponses, &anthropic),
            None
        );

        let gemini = encode_signature(SignatureProvider::Gemini, "CgR0ZXN0");
        assert_eq!(
            decode_signature_for(SignatureProvider::Gemini, &gemini),
            Some("CgR0ZXN0".to_string())
        );
        assert_eq!(
            decode_signature_for(SignatureProvider::Anthropic, &gemini),
            None
        );
    }

    #[test]
    fn encoding_only_treats_matching_marker_as_idempotent() {
        let nested = encode_signature(
            SignatureProvider::Anthropic,
            "ai-toolbox.sig.gemini:CgR0ZXN0",
        );
        assert_eq!(
            decode_signature_for(SignatureProvider::Anthropic, &nested),
            Some("ai-toolbox.sig.gemini:CgR0ZXN0".to_string())
        );
        assert_eq!(
            decode_signature_for(SignatureProvider::Gemini, &nested),
            None
        );
    }

    #[test]
    fn guesses_provider_from_unmarked_known_shapes() {
        assert_eq!(
            guess_signature_provider("gAAAAABopenai"),
            SignatureProvider::OpenAiResponses
        );
        assert_eq!(
            guess_signature_provider("EqQBCAEDEgQIAhAEGAAgAigBMOzOAg=="),
            SignatureProvider::Anthropic
        );
        assert_eq!(
            guess_signature_provider("CgR0ZXN0"),
            SignatureProvider::Gemini
        );
        assert_eq!(
            guess_signature_provider("plain-unknown-signature"),
            SignatureProvider::Unknown
        );
    }

    #[test]
    fn unmarked_unknown_signature_is_not_decoded_for_any_provider() {
        assert_eq!(
            decode_signature_for(SignatureProvider::Anthropic, "plain-unknown-signature"),
            None
        );
        assert_eq!(
            decode_signature_for(SignatureProvider::Gemini, "plain-unknown-signature"),
            None
        );
        assert_eq!(
            decode_signature_for(
                SignatureProvider::OpenAiResponses,
                "plain-unknown-signature"
            ),
            None
        );
    }
}
