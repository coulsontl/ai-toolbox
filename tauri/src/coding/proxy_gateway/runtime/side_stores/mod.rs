mod codex_history;
mod gemini_shadow;
mod native_compaction;
mod responses_cipher;

use std::sync::Arc;

/// Bound side-store SSE parsers so a malformed stream without blank-line
/// delimiters cannot grow the buffer without limit. Overflow drops side-store
/// recording for the rest of the stream without affecting client forwarding.
const SIDE_STORE_SSE_BUFFER_LIMIT: usize = 1024 * 1024;

fn take_sse_block(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let lf = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|index| (index, 2));
    let crlf = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, 4));
    let position = match (lf, crlf) {
        (Some(lf), Some(crlf)) if crlf.0 < lf.0 => crlf,
        (Some(lf), _) => lf,
        (None, Some(crlf)) => crlf,
        (None, None) => return None,
    };
    let block = buffer[..position.0].to_vec();
    buffer.drain(..position.0 + position.1);
    Some(block)
}

fn append_side_store_sse_bytes(buffer: &mut Vec<u8>, bytes: &[u8]) -> bool {
    if buffer.len() >= SIDE_STORE_SSE_BUFFER_LIMIT {
        return false;
    }
    let remaining = SIDE_STORE_SSE_BUFFER_LIMIT.saturating_sub(buffer.len());
    if bytes.len() > remaining {
        buffer.extend_from_slice(&bytes[..remaining]);
        return false;
    }
    buffer.extend_from_slice(bytes);
    true
}

pub(super) use codex_history::record_responses_sse_stream;
pub(super) use gemini_shadow::{record_gemini_sse_stream, GeminiShadowSessionKey};

#[derive(Clone, Default)]
pub(super) struct GatewaySideStores {
    codex_history: Arc<codex_history::CodexHistoryStore>,
    gemini_shadow: Arc<gemini_shadow::GeminiShadowStore>,
    invalid_responses_ciphers: Arc<responses_cipher::InvalidResponsesCipherStore>,
    native_compaction_rejections: Arc<native_compaction::NativeCompactionRejectionStore>,
}

impl GatewaySideStores {
    #[allow(dead_code)]
    pub(super) fn enrich_codex_request(&self, body: &mut serde_json::Value) -> usize {
        self.codex_history.enrich_request(body)
    }

    pub(super) fn enrich_codex_request_scoped(
        &self,
        session_scope: Option<&str>,
        body: &mut serde_json::Value,
    ) -> usize {
        self.codex_history.enrich_request_scoped(session_scope, body)
    }

    #[allow(dead_code)]
    pub(super) fn record_codex_response(&self, response: &serde_json::Value) -> usize {
        self.codex_history.record_response(response)
    }

    pub(super) fn record_codex_response_scoped(
        &self,
        session_scope: Option<&str>,
        response: &serde_json::Value,
    ) -> usize {
        self.codex_history
            .record_response_scoped(session_scope, response)
    }

    pub(super) fn codex_history(&self) -> Arc<codex_history::CodexHistoryStore> {
        self.codex_history.clone()
    }

    pub(super) fn enrich_gemini_request(
        &self,
        key: &GeminiShadowSessionKey,
        body: &mut serde_json::Value,
    ) -> usize {
        self.gemini_shadow.enrich_request(key, body)
    }

    pub(super) fn record_gemini_response(
        &self,
        key: GeminiShadowSessionKey,
        response: &serde_json::Value,
    ) -> usize {
        self.gemini_shadow.record_response(key, response)
    }

    pub(super) fn gemini_shadow(&self) -> Arc<gemini_shadow::GeminiShadowStore> {
        self.gemini_shadow.clone()
    }

    pub(super) fn remember_rejected_responses_ciphers(
        &self,
        provider_config_identity: [u8; 32],
        body: &[u8],
        error_message: &str,
    ) -> usize {
        self.invalid_responses_ciphers.remember_rejected_from_body(
            provider_config_identity,
            body,
            error_message,
        )
    }

    pub(super) fn strip_known_invalid_responses_ciphers(
        &self,
        provider_config_identity: [u8; 32],
        body: &mut serde_json::Value,
    ) -> usize {
        self.invalid_responses_ciphers
            .strip_known_from_body(provider_config_identity, body)
    }

    pub(super) fn has_known_invalid_responses_ciphers(
        &self,
        provider_config_identity: [u8; 32],
    ) -> bool {
        self.invalid_responses_ciphers
            .has_entries_for_provider(provider_config_identity)
    }

    /// Records that this provider configuration answered a bare
    /// `compaction_trigger` without a `compaction` item, so later turns go
    /// straight to the Gateway's own summarization.
    pub(super) fn remember_native_compaction_rejection(
        &self,
        provider_config_identity: [u8; 32],
    ) {
        self.native_compaction_rejections
            .remember(provider_config_identity);
    }

    pub(super) fn has_native_compaction_rejection(
        &self,
        provider_config_identity: [u8; 32],
    ) -> bool {
        self.native_compaction_rejections
            .is_remembered(provider_config_identity)
    }
}
