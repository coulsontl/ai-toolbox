//! Opaque envelopes for Codex remote compaction v2 summaries.
//!
//! Codex's remote compaction v2 asks the upstream for a summary and stores the
//! returned `compaction` item verbatim in its history, replaying it on every
//! later turn. Third-party upstreams cannot produce OpenAI's server-side
//! `encrypted_content`, so the Gateway generates the item itself and must be
//! able to read it back when Codex replays it.
//!
//! The summary is sealed into an AES-256-GCM capsule carried inside
//! `encrypted_content`. Two properties matter:
//!
//! - **Self-identifying.** The prefix distinguishes our capsules from real
//!   OpenAI ciphertext, so a capsule is only ever decoded where it was written.
//! - **Confidential.** The summary quotes the whole conversation; sealing keeps
//!   it unreadable in Codex's plaintext `rollout-*.jsonl` history.
//!
//! The key is a fixed constant, matching CLIProxyAPI's antigravity compaction
//! capsules. That is obfuscation rather than secrecy — the constant is in this
//! repository — but it is deliberately not a machine-derived key: a capsule
//! must stay readable after the user moves the config root, switches machines,
//! or restores a backup, because Codex's history keeps replaying it.

use super::super::routes::GatewayRoute;
use crate::coding::proxy_gateway::types::GatewayCliKey;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use super::super::providers::UpstreamProvider;
use crate::coding::proxy_gateway::transformer::AiProtocol;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// Marks a capsule written by this Gateway. The value must stay distinct from
/// CLIProxyAPI's `cpa-ag-compact-v1:` and cc-switch's `cc-switch-compaction-v1:`
/// so history carried between tools never decodes as the wrong envelope.
const COMPACTION_CAPSULE_PREFIX: &str = "ai-toolbox-compact-v1:";

/// Fixed key material. See the module docs for why this is not machine-derived.
const COMPACTION_KEY_SECRET: &str = "ai-toolbox-remote-compaction";

const NONCE_LENGTH: usize = 12;
const AUTH_TAG_LENGTH: usize = 16;

/// The payload sealed into one capsule.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct CompactionCapsule {
    summary: String,
    model: String,
    created_at: i64,
}

fn derive_key() -> LessSafeKey {
    let key_bytes = Sha256::digest(COMPACTION_KEY_SECRET.as_bytes());
    LessSafeKey::new(
        UnboundKey::new(&AES_256_GCM, &key_bytes)
            .expect("AES-256-GCM accepts a 32-byte key"),
    )
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// Nanosecond clock for ids that must stay unique across two turns in the same
/// second, which the capsule's `created_at` (seconds) and the item / response
/// ids have different needs for.
fn unix_timestamp_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as i64)
        .unwrap_or(0)
}

/// Seals a summary into an `encrypted_content` capsule.
///
/// Returns `None` only when the platform random source fails, which would leave
/// the nonce predictable; callers treat that as "no summary to install" rather
/// than falling back to a weaker envelope.
pub(super) fn seal_compaction_summary(summary: &str, model: &str) -> Option<String> {
    let capsule = CompactionCapsule {
        summary: summary.to_string(),
        model: model.to_string(),
        created_at: unix_timestamp(),
    };
    let plaintext = serde_json::to_vec(&capsule).ok()?;

    let mut nonce_bytes = [0u8; NONCE_LENGTH];
    SystemRandom::new().fill(&mut nonce_bytes).ok()?;

    let mut sealed = plaintext;
    derive_key()
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::empty(),
            &mut sealed,
        )
        .ok()?;

    let mut encoded = Vec::with_capacity(NONCE_LENGTH + sealed.len());
    encoded.extend_from_slice(&nonce_bytes);
    encoded.extend_from_slice(&sealed);
    Some(format!(
        "{COMPACTION_CAPSULE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(&encoded)
    ))
}

/// Opens a capsule produced by [`seal_compaction_summary`].
///
/// Returns `None` for anything this Gateway did not write — real OpenAI
/// ciphertext, another tool's envelope, or a corrupted value. Callers must treat
/// that as "leave the item alone", never as an error: an unreadable capsule is
/// indistinguishable from a legitimate upstream item.
pub(in crate::coding::proxy_gateway::runtime) fn open_compaction_summary(
    encrypted_content: &str,
) -> Option<String> {
    let body = encrypted_content.strip_prefix(COMPACTION_CAPSULE_PREFIX)?;
    let decoded = URL_SAFE_NO_PAD.decode(body).ok()?;
    if decoded.len() < NONCE_LENGTH + AUTH_TAG_LENGTH {
        return None;
    }
    let (nonce_bytes, sealed) = decoded.split_at(NONCE_LENGTH);
    let nonce = nonce_bytes.try_into().ok()?;
    let mut sealed = sealed.to_vec();

    let plaintext = derive_key()
        .open_in_place(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut sealed)
        .ok()?;
    let capsule: CompactionCapsule = serde_json::from_slice(plaintext).ok()?;
    let summary = capsule.summary.trim().to_string();
    (!summary.is_empty()).then_some(summary)
}

/// Whether a value carries this Gateway's compaction envelope.
pub(super) fn is_compaction_capsule(encrypted_content: &str) -> bool {
    encrypted_content.starts_with(COMPACTION_CAPSULE_PREFIX)
}

/// Log label: the upstream itself answered the compaction turn with an item.
pub(in crate::coding::proxy_gateway::runtime) const COMPACTION_MODE_NATIVE: &str = "native";
/// Log label: the trigger was forwarded, the upstream did not answer with an
/// item, and the Gateway summarized the turn instead.
pub(in crate::coding::proxy_gateway::runtime) const COMPACTION_MODE_FALLBACK: &str = "fallback";
/// Log label: the target protocol could never serve the turn natively, so the
/// trigger was never forwarded at all.
pub(in crate::coding::proxy_gateway::runtime) const COMPACTION_MODE_SIMULATED: &str = "simulated";
/// Log label: Codex asked the model for a compaction summary on the ordinary
/// `/responses` path (no `compaction_trigger`). The Gateway does nothing here —
/// the reply is an ordinary assistant summary and Codex installs it as local
/// history itself — but the request list still needs to say what the turn was.
pub(in crate::coding::proxy_gateway::runtime) const COMPACTION_MODE_LOCAL: &str = "local";

/// How the current Codex request relates to remote compaction.
///
/// Resolved once per request from the route and the inbound body, then carried
/// through both the request and response halves so they cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::coding::proxy_gateway::runtime) enum RemoteCompactionCompat {
    /// An ordinary turn.
    None,
    /// The upstream is being asked for a summary; the reply is rewritten into a
    /// `compaction` item instead of being passed through.
    Summarize,
    /// The `compaction_trigger` is forwarded unchanged so an upstream that
    /// natively implements v2 answers with its own `compaction` item. This is
    /// the mode tried first for every Responses-speaking target; a reply that
    /// is not a native item makes the caller fall back to
    /// [`Self::Summarize`] with a second request.
    Passthrough,
    /// The upstream was offered the trigger and refused it, so the turn is being
    /// summarized instead. Every wire-visible behaviour matches
    /// [`Self::Summarize`]; the distinction exists because "the Gateway
    /// summarized" and "the Gateway summarized *because* a native attempt
    /// failed" are different answers to the only question a user has about a
    /// compaction turn — whether their chain really served it.
    Fallback,
    /// An ordinary turn that replays a previous compaction item, which must be
    /// expanded back into readable context before forwarding.
    ReplayCapsules,
}

impl RemoteCompactionCompat {
    /// Test-only convenience: every production path resolves from a request.
    #[cfg(test)]
    pub(in crate::coding::proxy_gateway::runtime) fn none() -> Self {
        Self::None
    }

    /// Resolves the mode for a Codex request.
    ///
    /// Compaction only ever originates from Codex, so every other CLI short
    /// circuits before the body is inspected.
    ///
    /// `native_first` is the caller's answer to "should this turn be offered to
    /// the upstream as a native v2 turn?". It is `false` only once this exact
    /// provider configuration has already refused one (see
    /// `NativeCompactionRejectionStore`); a converted target is rejected by
    /// [`compaction_turn_mode`] regardless of what the caller passes.
    ///
    /// The legacy `/responses/compact` endpoint is deliberately left to
    /// [`CodexResponsesCompactCompat`], which already owns it; claiming it here
    /// too would rewrite the same request twice.
    pub(in crate::coding::proxy_gateway::runtime) fn resolve(
        route: &GatewayRoute,
        provider: Option<&UpstreamProvider>,
        native_first: bool,
        body: &[u8],
    ) -> Self {
        if route.cli_key != GatewayCliKey::Codex {
            return Self::None;
        }
        let Ok(value) = serde_json::from_slice::<Value>(body) else {
            return Self::None;
        };
        if is_remote_compaction_v2_request(&value) {
            return compaction_turn_mode(provider, native_first);
        }
        if carries_gateway_compaction_capsule(&value) {
            return Self::ReplayCapsules;
        }
        Self::None
    }

    /// Resolves the mode for a Codex `response.create` WebSocket frame.
    ///
    /// The frame is a Responses request in a WebSocket envelope, so the same
    /// signals apply. Unlike HTTP there is no route to check: the upgrade was
    /// only accepted for a Codex Responses route to begin with.
    ///
    /// `native_first` carries the same meaning as in [`Self::resolve`]. The
    /// relay buffers a pass-through turn's frames rather than dropping them, so
    /// a native item can be replayed and anything else can be discarded before
    /// the synthesizing retry.
    pub(in crate::coding::proxy_gateway::runtime) fn resolve_websocket(
        provider: Option<&UpstreamProvider>,
        native_first: bool,
        body: &Value,
    ) -> Self {
        if is_remote_compaction_v2_request(body) {
            return compaction_turn_mode(provider, native_first);
        }
        if carries_gateway_compaction_capsule(body) {
            return Self::ReplayCapsules;
        }
        Self::None
    }

    /// Whether this turn is a compaction turn of either kind, which is what the
    /// request log records so a user can tell a native pass-through from the
    /// Gateway's own simulation without reading the wire.
    pub(in crate::coding::proxy_gateway::runtime) fn is_compaction_turn(self) -> bool {
        matches!(self, Self::Summarize | Self::Fallback | Self::Passthrough)
    }

    /// Whether the upstream is asked for a summary and the Gateway therefore
    /// writes the `compaction` item itself.
    pub(in crate::coding::proxy_gateway::runtime) fn is_summarize(self) -> bool {
        matches!(self, Self::Summarize | Self::Fallback)
    }

    /// Whether the `compaction_trigger` travels to the upstream unchanged and
    /// the upstream's own `compaction` item is forwarded back as-is.
    pub(in crate::coding::proxy_gateway::runtime) fn is_passthrough(self) -> bool {
        self == Self::Passthrough
    }

    /// Whether the input can carry capsules this Gateway wrote, which no
    /// upstream can read. Every compaction-related turn expands them — including
    /// a pass-through turn, whose history may still hold capsules from a time
    /// when the provider was simulated.
    pub(in crate::coding::proxy_gateway::runtime) fn expands_capsules(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// How a `compaction_trigger` turn is served for this provider.
///
/// A Responses target is offered the trigger first, exactly as Codex's own
/// upstream would be: the upstream that implements v2 turns it into its own
/// `compaction` item, and the caller relays that item untouched. Whether the
/// upstream really understands the trigger is not knowable in advance — an
/// upstream speaking the Responses protocol at the right path may still answer
/// an ordinary assistant message, which Codex rejects with `got 0 from 1 output
/// items` — so the caller falls back to [`RemoteCompactionCompat::Summarize`]
/// with a second request instead of trusting the answer. That is why
/// `native_first` is only ever cleared by a recorded refusal, never by a guess.
///
/// A converted target cannot be served natively at all: v2 is defined in terms
/// of the Responses wire, so an upstream reached through a protocol conversion
/// has no way to implement it and the trigger is never forwarded there.
fn compaction_turn_mode(
    provider: Option<&UpstreamProvider>,
    native_first: bool,
) -> RemoteCompactionCompat {
    let forwardable = provider
        .is_some_and(|provider| provider.target_protocol == AiProtocol::OpenAiResponses);
    if forwardable && native_first {
        RemoteCompactionCompat::Passthrough
    } else {
        RemoteCompactionCompat::Summarize
    }
}

/// Whether a reply is the native v2 answer Codex accepts: exactly one
/// `compaction` output item carrying ciphertext.
///
/// A pass-through turn is only allowed to reach Codex when this holds. Anything
/// else — an ordinary assistant message, an empty reply, two compaction items —
/// is what an upstream that does not really implement v2 returns, and Codex
/// would fail the turn on it. The caller falls back to sealing its own item.
pub(in crate::coding::proxy_gateway::runtime) fn is_native_compaction_response(body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let Some(output) = value.get("output").and_then(Value::as_array) else {
        return false;
    };
    if output.len() != 1 {
        return false;
    }
    output[0].get("type").and_then(Value::as_str) == Some("compaction")
        && output[0]
            .get("encrypted_content")
            .and_then(Value::as_str)
            .is_some_and(|content| !content.trim().is_empty())
}

/// The instruction that turns a normal turn into a summarization turn.
///
/// Codex's own base instructions tell the model to keep implementing the task,
/// so they are replaced rather than supplemented. The wording mirrors
/// CLIProxyAPI's antigravity summary prompt in intent: produce a hand-off
/// summary, do not continue the work.
pub(super) const COMPACTION_SYSTEM_INSTRUCTION: &str = "Produce a concise, complete continuation summary of the conversation so far for another coding agent. Cover user goals, constraints and decisions, work already completed, files and symbols touched, tool results, validation evidence, unresolved errors, and the exact next steps. Do not continue the task and do not call tools. Reply with the summary only.";

/// Follow-up user turn that triggers the summary.
pub(super) const COMPACTION_USER_INSTRUCTION: &str =
    "Produce the continuation summary now.";

/// Heading prefixed to a restored summary so the model reads it as background
/// rather than as its own prior words.
pub(super) const COMPACTED_CONTEXT_HEADING: &str = "Context summary from previous turns:";

/// Whether a request is a Codex remote compaction v2 request.
///
/// v2 uses the ordinary `/responses` endpoint and appends a `compaction_trigger`
/// input item; the legacy `/responses/compact` endpoint is handled separately by
/// [`CodexResponsesCompactCompat`].
///
/// `x-codex-turn-metadata.request_kind == "compaction"` is deliberately *not* a
/// signal. Codex also sends that metadata for local checkpoint compaction, whose
/// response must stay an ordinary assistant summary; treating it as remote
/// compaction would replace a real summary with an opaque item.
pub(super) fn is_remote_compaction_v2_request(body: &Value) -> bool {
    body.get("input")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(is_compaction_trigger_item))
}

fn is_compaction_trigger_item(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("compaction_trigger")
}

/// Whether Codex marked this request as a **local** checkpoint compaction.
///
/// This is the one thing the metadata is good for. It must never feed
/// [`is_remote_compaction_v2_request`] or [`RemoteCompactionCompat::resolve`]:
/// the Gateway's job on such a turn is to leave the ordinary assistant summary
/// alone, and rewriting it into a `compaction` item would destroy real context.
/// Reading it only to *label* the turn keeps those two decisions apart.
///
/// Codex sends the metadata twice — a direct `x-codex-turn-metadata` header and
/// the canonical `client_metadata["x-codex-turn-metadata"]` string — so both
/// are consulted. The body copy is what makes a WebSocket turn work: its
/// `DebugHttpRequest` carries the handshake's headers, not the frame's.
pub(in crate::coding::proxy_gateway::runtime) fn is_local_compaction_request(
    body: &Value,
    turn_metadata: Option<&str>,
) -> bool {
    let from_body = body
        .get("client_metadata")
        .and_then(|metadata| metadata.get("x-codex-turn-metadata"))
        .and_then(Value::as_str);
    from_body
        .or(turn_metadata)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|metadata| {
            metadata
                .get("request_kind")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .is_some_and(|kind| kind == "compaction")
}

/// Whether any input item is a compaction item carrying this Gateway's capsule.
///
/// Only used to decide whether a follow-up turn replays our own summary; real
/// OpenAI ciphertext and other tools' envelopes intentionally do not match.
pub(super) fn carries_gateway_compaction_capsule(body: &Value) -> bool {
    body.get("input")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("type").and_then(Value::as_str) == Some("compaction")
                    && item
                        .get("encrypted_content")
                        .and_then(Value::as_str)
                        .is_some_and(is_compaction_capsule)
            })
        })
}

/// Rewrites a Codex remote compaction request into a plain summarization turn.
///
/// The upstream must see an ordinary chat request: Codex's `compaction_trigger`
/// is a private control item no third-party provider understands, the tool
/// inventory is irrelevant to summarizing, and Codex's base instructions would
/// otherwise make the model resume the coding task instead of summarizing it.
pub(in crate::coding::proxy_gateway::runtime) fn build_compaction_summary_request(body: &Value) -> Option<Value> {
    let mut summary_request = body.clone();
    let object = summary_request.as_object_mut()?;

    for key in [
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "response_format",
        "stop",
        "previous_response_id",
    ] {
        object.remove(key);
    }

    // Codex's base instructions describe the coding agent, not the summarizer.
    object.insert(
        "instructions".to_string(),
        Value::String(COMPACTION_SYSTEM_INSTRUCTION.to_string()),
    );

    // Ask for a single non-streamed summary. Codex always streams its own
    // request, but the compaction contract needs the whole summary before any
    // event can be emitted, so the Gateway asks upstream for it in one piece
    // rather than buffering a stream it cannot forward.
    object.insert("stream".to_string(), Value::Bool(false));

    if let Some(input) = object.get_mut("input").and_then(Value::as_array_mut) {
        input.retain(|item| !is_compaction_trigger_item(item));
        input.push(serde_json::json!({
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": COMPACTION_USER_INSTRUCTION }],
        }));
    }

    Some(summary_request)
}

/// Replaces this Gateway's capsules with readable assistant context.
///
/// Applied to the upstream body of an ordinary turn that replays a previous
/// compaction item. Items this Gateway did not write are left untouched, so a
/// real OpenAI item still flows through to an upstream that understands it.
///
/// Returns the number of capsules expanded.
pub(in crate::coding::proxy_gateway::runtime) fn expand_compaction_capsules(body: &mut Value) -> usize {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return 0;
    };

    let mut expanded = 0;
    for item in input.iter_mut() {
        if item.get("type").and_then(Value::as_str) != Some("compaction") {
            continue;
        }
        let Some(summary) = item
            .get("encrypted_content")
            .and_then(Value::as_str)
            .and_then(open_compaction_summary)
        else {
            continue;
        };
        *item = serde_json::json!({
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": format!("{COMPACTED_CONTEXT_HEADING}\n{summary}"),
            }],
        });
        expanded += 1;
    }
    expanded
}

/// Builds the compaction item Codex installs in its history.
pub(in crate::coding::proxy_gateway::runtime) fn build_compaction_item(summary: &str, model: &str) -> Option<Value> {
    let encrypted_content = seal_compaction_summary(summary, model)?;
    Some(serde_json::json!({
        "type": "compaction",
        // Nanoseconds, not seconds: two sessions compacting in the same second
        // must not hand Codex two items with the same id.
        "id": format!("cmp_{}", unix_timestamp_nanos()),
        "encrypted_content": encrypted_content,
    }))
}

/// Why a summarization response could not be turned into a compaction item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::coding::proxy_gateway::runtime) enum CompactionSummaryError {
    /// The upstream reported an error for the summarization turn.
    ///
    /// Any text that arrived before the failure is a partial draft, not a
    /// summary: using it would install half a context and hide the error Codex
    /// needs to classify and retry.
    Failed,
    /// The upstream truncated the summary, so it would install a partial context.
    Truncated,
    /// The upstream returned no usable text.
    Empty,
}

/// Extracts the summary text from an upstream response in any target protocol.
///
/// The gateway forwards the summary request in whatever protocol the provider
/// speaks, so the response shape varies. Detection is by structure rather than
/// by a passed-in protocol: a provider profile may route a single request
/// through a different wire format than its declared one.
pub(in crate::coding::proxy_gateway::runtime) fn extract_summary_text(
    response: &Value,
) -> Result<String, CompactionSummaryError> {
    if is_failed(response) {
        return Err(CompactionSummaryError::Failed);
    }
    if is_truncated(response) {
        return Err(CompactionSummaryError::Truncated);
    }

    let text = responses_output_text(response)
        .or_else(|| chat_choices_text(response))
        .or_else(|| anthropic_content_text(response))
        .or_else(|| gemini_candidates_text(response))
        .unwrap_or_default();
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(CompactionSummaryError::Empty);
    }
    Ok(text)
}

/// Whether the upstream stopped before the summary was complete.
///
/// Two shapes arrive here. A raw Chat / Anthropic body still carries the wire's
/// own stop reason. A Responses body - the shape conversion produces for every
/// source protocol, and the shape Codex's own upstreams answer with natively -
/// carries `status: "incomplete"` instead: `finish_to_responses_status` writes it
/// for `length` / `max_tokens` from all four protocols, and the forced-stream
/// aggregator keeps the terminal status. Checking only the wire fields left that
/// shape unguarded, so a truncated reply that still carried partial text was
/// sealed as the *entire* conversation history (issue #416 reported the outcome
/// as an "empty compaction summary", because nothing else was checked).
pub(in crate::coding::proxy_gateway::runtime) fn is_truncated(response: &Value) -> bool {
    let finish_reason = response
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .or_else(|| response.get("stop_reason").and_then(Value::as_str));
    matches!(finish_reason, Some("length") | Some("max_tokens"))
        || response.get("status").and_then(Value::as_str) == Some("incomplete")
}

/// Whether the summarization turn itself ended in failure.
///
/// A failed or cancelled turn can still carry text, and that draft is a partial
/// context too. This is reachable on a 2xx body: the forced-stream aggregator
/// hands back an upstream `response.failed` / `response.cancelled` as JSON, with
/// the headers that already carried the 200. Non-2xx never gets this far - those
/// fail the request before the summary is read.
pub(in crate::coding::proxy_gateway::runtime) fn is_failed(response: &Value) -> bool {
    if matches!(
        response.get("status").and_then(Value::as_str),
        Some("failed") | Some("canceled") | Some("cancelled")
    ) {
        return true;
    }
    response
        .get("error")
        .and_then(Value::as_object)
        .is_some_and(|error| !error.is_empty())
}

/// Whether a pass-through reply reports a turn the upstream could not finish,
/// as opposed to answering the trigger with something else.
///
/// The caller uses this to decide whether a non-native pass-through reply
/// counts as a refusal. A failed or truncated turn is not one: the upstream
/// processed the request but did not complete it, so the next compaction must
/// probe natively again instead of being remembered as unsupported.
pub(in crate::coding::proxy_gateway::runtime) fn is_failed_or_truncated_response(
    body: &[u8],
) -> bool {
    let Ok(response) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    is_failed(&response) || is_truncated(&response)
}

/// Collects the assistant text out of an upstream Responses WebSocket stream.
///
/// The WebSocket transport has no "give me one non-streamed body" mode: frames
/// arrive one event at a time, so the summary has to be reassembled from the
/// deltas before the compaction item can be sealed and replayed to Codex.
#[derive(Debug, Default)]
pub(in crate::coding::proxy_gateway::runtime) struct CompactionStreamSummary {
    deltas: String,
    /// Final text of an `output_item.done` message, used when the upstream
    /// omitted the deltas.
    completed: Option<String>,
    /// Set when the upstream streamed its own `compaction` item.
    ///
    /// This is the only signal that separates an upstream which implements v2
    /// from one that merely ignored the trigger and answered an ordinary
    /// message: both are a successful stream of a `message` item, and only one
    /// of them carries an item Codex will accept.
    native_item: bool,
    truncated: bool,
    failed: bool,
    terminal: bool,
    usage: Value,
}

impl CompactionStreamSummary {
    pub(in crate::coding::proxy_gateway::runtime) fn push(&mut self, event: &Value) {
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_text.delta") => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    self.deltas.push_str(delta);
                }
            }
            Some("response.output_item.added") | Some("response.output_item.done") => {
                if let Some(item) = event.get("item") {
                    self.note_native_item(item);
                    if let Some(text) = responses_message_item_text(item) {
                        self.completed = Some(text);
                    }
                }
            }
            Some("response.completed") => {
                self.terminal = true;
                if let Some(response) = event.get("response") {
                    // A terminal snapshot can carry the same verdicts as the
                    // dedicated events: an upstream that reports `status:
                    // "incomplete"` or a non-empty `error` inside
                    // `response.completed` must not have its draft sealed as
                    // the whole history.
                    if is_failed(response) {
                        self.failed = true;
                    } else if is_truncated(response) {
                        self.truncated = true;
                    }
                    if let Some(usage) = response.get("usage") {
                        self.usage = usage.clone();
                    }
                    if let Some(output) = response.get("output").and_then(Value::as_array) {
                        for item in output {
                            self.note_native_item(item);
                        }
                    }
                    if let Some(text) = responses_output_text(response) {
                        self.completed = Some(text);
                    }
                }
            }
            Some("response.incomplete") => {
                self.terminal = true;
                self.truncated = true;
            }
            Some("response.failed") | Some("error") => {
                self.terminal = true;
                self.failed = true;
            }
            // A cancelled turn is a failure the same way `is_failed` reads it
            // on the HTTP side (`status: canceled/cancelled`): the upstream
            // stopped early, which is transient for native support — it must
            // end the turn (otherwise the relay waits for a terminal that
            // already passed) and must not be remembered as a refusal.
            Some("response.cancelled") | Some("response.canceled") => {
                self.terminal = true;
                self.failed = true;
            }
            _ => {}
        }
    }

    pub(in crate::coding::proxy_gateway::runtime) fn is_terminal(&self) -> bool {
        self.terminal
    }

    /// Whether the upstream streamed its own `compaction` item.
    pub(in crate::coding::proxy_gateway::runtime) fn has_native_item(&self) -> bool {
        self.native_item
    }

    /// Records an upstream output item that counts as a native v2 answer.
    ///
    /// `response.output_item.added` is enough on its own: an upstream that
    /// announces the item will finish it, and one that only ever announces a
    /// `message` has already shown what it understood the turn to be.
    fn note_native_item(&mut self, item: &Value) {
        if item.get("type").and_then(Value::as_str) == Some("compaction") {
            self.native_item = true;
        }
    }

    /// The summary text, or why it cannot be used.
    ///
    /// An unusable summary must never be replaced by placeholder text: Codex
    /// installs the returned item as the *entire* conversation history, so a
    /// marker would silently discard the context instead of surfacing the
    /// failure. Callers forward the upstream's own terminal in that case.
    pub(in crate::coding::proxy_gateway::runtime) fn finish(
        &self,
    ) -> Result<String, CompactionSummaryError> {
        if self.failed {
            return Err(CompactionSummaryError::Failed);
        }
        if self.truncated {
            return Err(CompactionSummaryError::Truncated);
        }
        let text = if self.deltas.trim().is_empty() {
            self.completed.clone().unwrap_or_default()
        } else {
            self.deltas.clone()
        };
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(CompactionSummaryError::Empty);
        }
        Ok(text)
    }

    /// The upstream's own usage for the summarization turn, in Responses shape.
    pub(in crate::coding::proxy_gateway::runtime) fn usage(&self) -> &Value {
        &self.usage
    }
}

/// The text of one Responses `message` output item, read from its own `content`.
///
/// `response.output_item.done` carries the finished item, which is where a
/// WebSocket upstream that skips the delta events puts the whole message; the
/// item has no `output` array, so [`responses_output_text`] cannot read it.
fn responses_message_item_text(item: &Value) -> Option<String> {
    if item.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let parts = match item.get("content") {
        Some(Value::String(text)) => vec![text.clone()],
        Some(Value::Array(content)) => content
            .iter()
            .filter_map(|part| {
                (part.get("type").and_then(Value::as_str) == Some("output_text"))
                    .then(|| part.get("text").and_then(Value::as_str).map(str::to_string))
                    .flatten()
            })
            .collect(),
        _ => Vec::new(),
    };
    join_non_empty(parts)
}

/// OpenAI Responses `output[].content[].output_text`, skipping reasoning items.
fn responses_output_text(response: &Value) -> Option<String> {
    let output = response.get("output")?.as_array()?;
    let mut parts = Vec::new();
    for item in output {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        match item.get("content") {
            Some(Value::String(text)) => parts.push(text.clone()),
            Some(Value::Array(content)) => {
                for part in content {
                    if part.get("type").and_then(Value::as_str) == Some("output_text") {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            parts.push(text.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    join_non_empty(parts)
}

/// OpenAI Chat `choices[].message.content`.
fn chat_choices_text(response: &Value) -> Option<String> {
    let choices = response.get("choices")?.as_array()?;
    let mut parts = Vec::new();
    for choice in choices {
        let content = choice.pointer("/message/content");
        match content {
            Some(Value::String(text)) => parts.push(text.clone()),
            Some(Value::Array(items)) => {
                for item in items {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        parts.push(text.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    join_non_empty(parts)
}

/// Anthropic Messages `content[].text`, skipping thinking blocks.
fn anthropic_content_text(response: &Value) -> Option<String> {
    let content = response.get("content")?.as_array()?;
    let mut parts = Vec::new();
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        if let Some(text) = block.get("text").and_then(Value::as_str) {
            parts.push(text.to_string());
        }
    }
    join_non_empty(parts)
}

/// Gemini Native `candidates[].content.parts[].text`, skipping thought parts.
fn gemini_candidates_text(response: &Value) -> Option<String> {
    let candidates = response
        .get("candidates")
        .or_else(|| response.pointer("/response/candidates"))?
        .as_array()?;
    let mut parts = Vec::new();
    for candidate in candidates {
        let Some(content_parts) = candidate
            .pointer("/content/parts")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for part in content_parts {
            if part.get("thought").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                parts.push(text.to_string());
            }
        }
    }
    join_non_empty(parts)
}

fn join_non_empty(parts: Vec<String>) -> Option<String> {
    let joined = parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!joined.trim().is_empty()).then_some(joined)
}

/// Wraps a compaction item in the Responses envelope Codex expects.
pub(in crate::coding::proxy_gateway::runtime) fn build_compaction_response(
    item: &Value,
    model: &str,
    upstream: &Value,
) -> Value {
    compaction_response_envelope(item, model, compaction_usage(upstream))
}

/// The same envelope for the WebSocket transport, whose usage already arrives in
/// Responses shape on `response.completed`.
pub(in crate::coding::proxy_gateway::runtime) fn build_websocket_compaction_response(
    item: &Value,
    model: &str,
    usage: &Value,
) -> Value {
    let usage = compaction_usage(&serde_json::json!({ "usage": usage }));
    compaction_response_envelope(item, model, usage)
}

fn compaction_response_envelope(item: &Value, model: &str, usage: Value) -> Value {
    let response_id = format!("resp_compact_{}", unix_timestamp_nanos());
    serde_json::json!({
        "id": response_id,
        "object": "response.compaction",
        "created_at": unix_timestamp(),
        "status": "completed",
        "model": model,
        "output": [item],
        "usage": usage,
    })
}

/// Carries the summarization call's usage through to the client.
///
/// Codex bills the compaction turn like any other request, so dropping usage
/// would under-report it. Unknown shapes yield no usage rather than zeroes.
fn compaction_usage(upstream: &Value) -> Value {
    let input_tokens = upstream
        .pointer("/usage/input_tokens")
        .or_else(|| upstream.pointer("/usage/prompt_tokens"))
        .or_else(|| upstream.pointer("/usageMetadata/promptTokenCount"))
        .and_then(Value::as_i64);
    let output_tokens = upstream
        .pointer("/usage/output_tokens")
        .or_else(|| upstream.pointer("/usage/completion_tokens"))
        .or_else(|| upstream.pointer("/usageMetadata/candidatesTokenCount"))
        .and_then(Value::as_i64);
    let total_tokens = upstream
        .pointer("/usage/total_tokens")
        .or_else(|| upstream.pointer("/usageMetadata/totalTokenCount"))
        .and_then(Value::as_i64);

    serde_json::json!({
        "input_tokens": input_tokens.unwrap_or(0),
        "output_tokens": output_tokens.unwrap_or(0),
        "total_tokens": total_tokens.unwrap_or_else(|| {
            input_tokens.unwrap_or(0) + output_tokens.unwrap_or(0)
        }),
    })
}

/// Serializes the compaction SSE stream Codex consumes.
///
/// Codex requires exactly one `compaction` output item followed by
/// `response.completed`. The full lifecycle is emitted rather than the minimum
/// two events so a stricter Responses client sees the same shape a real
/// upstream produces.
pub(in crate::coding::proxy_gateway::runtime) fn build_compaction_sse(response: &Value) -> String {
    let mut stream = String::new();
    for (name, payload) in compaction_events(response) {
        stream.push_str("event: ");
        stream.push_str(name);
        stream.push_str("\ndata: ");
        stream.push_str(&payload.to_string());
        stream.push_str("\n\n");
    }
    stream
}

/// Serializes the same lifecycle as WebSocket frames.
///
/// The Responses WebSocket carries each event as one bare JSON text frame —
/// unlike HTTP, there is no `event:`/`data:` envelope to wrap it in.
pub(in crate::coding::proxy_gateway::runtime) fn build_compaction_frames(
    response: &Value,
) -> Vec<String> {
    compaction_events(response)
        .into_iter()
        .map(|(_, payload)| payload.to_string())
        .collect()
}

/// The event lifecycle Codex consumes, shared by both transports.
fn compaction_events(response: &Value) -> Vec<(&'static str, Value)> {
    let Some(item) = response
        .get("output")
        .and_then(Value::as_array)
        .and_then(|output| output.first())
    else {
        return Vec::new();
    };

    // The opening snapshots must not carry the item yet: Codex counts every
    // `compaction` item it sees, and the terminal `response.completed` already
    // delivers exactly one.
    let mut opened = response.clone();
    opened["status"] = serde_json::json!("in_progress");
    opened["output"] = serde_json::json!([]);

    let mut in_progress_item = item.clone();
    in_progress_item["status"] = serde_json::json!("in_progress");
    let mut completed_item = item.clone();
    completed_item["status"] = serde_json::json!("completed");

    vec![
        ("response.created", serde_json::json!({ "type": "response.created", "response": opened })),
        ("response.in_progress", serde_json::json!({ "type": "response.in_progress", "response": opened })),
        (
            "response.output_item.added",
            serde_json::json!({ "type": "response.output_item.added", "output_index": 0, "item": in_progress_item }),
        ),
        (
            "response.output_item.done",
            serde_json::json!({ "type": "response.output_item.done", "output_index": 0, "item": completed_item }),
        ),
        (
            "response.completed",
            serde_json::json!({ "type": "response.completed", "response": response }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::super::super::providers::{ProviderAuthStrategy, UpstreamProvider};
    use super::*;
    use crate::coding::proxy_gateway::types::ProviderGatewayMeta;

    #[test]
    fn a_sealed_summary_round_trips() {
        let sealed = seal_compaction_summary("Goal: fix auto compact", "deepseek-v4-pro")
            .expect("sealing must succeed");
        assert!(is_compaction_capsule(&sealed));
        assert_eq!(
            open_compaction_summary(&sealed).as_deref(),
            Some("Goal: fix auto compact")
        );
    }

    #[test]
    fn capsules_are_not_plaintext() {
        let summary = "secret project plan";
        let sealed = seal_compaction_summary(summary, "m").expect("sealing must succeed");
        assert!(!sealed.contains(summary));
        // The base64 body alone must not decode to readable text either.
        let body = sealed.strip_prefix(COMPACTION_CAPSULE_PREFIX).unwrap();
        let raw = URL_SAFE_NO_PAD.decode(body).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains(summary));
    }

    #[test]
    fn each_seal_uses_a_fresh_nonce() {
        let first = seal_compaction_summary("same", "m").unwrap();
        let second = seal_compaction_summary("same", "m").unwrap();
        assert_ne!(first, second, "identical summaries must not share ciphertext");
        assert_eq!(open_compaction_summary(&first), open_compaction_summary(&second));
    }

    /// Two items sealed back to back — two sessions compacting in the same
    /// second — must not share an id, so the ids are nanosecond-clocked.
    #[test]
    fn compaction_items_get_distinct_ids() {
        let first = build_compaction_item("summary", "m").unwrap();
        let second = build_compaction_item("summary", "m").unwrap();
        let first_id = first.get("id").and_then(Value::as_str).unwrap();
        let second_id = second.get("id").and_then(Value::as_str).unwrap();
        assert_ne!(first_id, second_id);
        assert!(first_id.starts_with("cmp_"), "{first_id}");
    }

    #[test]
    fn foreign_values_are_not_opened() {
        for value in [
            "",
            "not-a-capsule",
            // Real OpenAI ciphertext shape.
            "gAAAAABm1234567890abcdef",
            // Sibling tools' envelopes must not decode here.
            "cpa-ag-compact-v1:AAAA",
            "cc-switch-compaction-v1:AAAA",
        ] {
            assert_eq!(open_compaction_summary(value), None, "{value}");
        }
    }

    #[test]
    fn corrupted_capsules_are_rejected() {
        let sealed = seal_compaction_summary("summary", "m").unwrap();
        let body = sealed.strip_prefix(COMPACTION_CAPSULE_PREFIX).unwrap();

        // Tampered ciphertext fails authentication.
        let mut bytes = URL_SAFE_NO_PAD.decode(body).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let tampered = format!("{COMPACTION_CAPSULE_PREFIX}{}", URL_SAFE_NO_PAD.encode(&bytes));
        assert_eq!(open_compaction_summary(&tampered), None);

        // Truncated and malformed bodies are rejected without panicking.
        for value in [
            format!("{COMPACTION_CAPSULE_PREFIX}"),
            format!("{COMPACTION_CAPSULE_PREFIX}AAAA"),
            format!("{COMPACTION_CAPSULE_PREFIX}!!!"),
        ] {
            assert_eq!(open_compaction_summary(&value), None, "{value}");
        }
    }

    #[test]
    fn a_blank_summary_is_not_a_capsule() {
        let sealed = seal_compaction_summary("   ", "m").unwrap();
        assert_eq!(open_compaction_summary(&sealed), None);
    }

    fn compaction_request() -> Value {
        serde_json::json!({
            "model": "deepseek-v4-pro",
            "instructions": "You are a coding agent. Continue implementing the task.",
            "input": [
                { "type": "message", "role": "user", "content": "Fix the compact bug" },
                { "type": "compaction_trigger" },
            ],
            "tools": [{ "type": "function", "name": "shell", "parameters": { "type": "object" } }],
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "stream": true,
        })
    }

    #[test]
    fn detects_the_v2_compaction_shape() {
        assert!(is_remote_compaction_v2_request(&compaction_request()));
    }

    #[test]
    fn an_ordinary_turn_is_not_compaction() {
        assert!(!is_remote_compaction_v2_request(&serde_json::json!({
            "input": [{ "type": "message", "role": "user", "content": "hello" }]
        })));
    }

    /// Local checkpoint compaction carries the same `request_kind` metadata as
    /// remote compaction but must keep returning an ordinary assistant summary.
    #[test]
    fn local_checkpoint_metadata_is_not_remote_compaction() {
        assert!(!is_remote_compaction_v2_request(&serde_json::json!({
            "input": [],
            "client_metadata": {
                "x-codex-turn-metadata": "{\"request_kind\":\"compaction\"}"
            }
        })));
    }

    /// The same metadata is still the only signal that a turn was a local
    /// compaction. Reading it for the log label is a separate decision from the
    /// rewrite, which the test above pins.
    #[test]
    fn local_checkpoint_metadata_is_recognized_for_labelling() {
        let local = serde_json::json!({
            "input": [],
            "client_metadata": {
                "x-codex-turn-metadata": "{\"request_kind\":\"compaction\",\"turn_id\":\"t\"}"
            }
        });
        assert!(is_local_compaction_request(&local, None));
        assert!(!is_remote_compaction_v2_request(&local));

        // The header is the fallback for a request body that carries no
        // metadata of its own.
        assert!(is_local_compaction_request(
            &serde_json::json!({ "input": [] }),
            Some("{\"request_kind\":\"compaction\"}"),
        ));
        // The body wins when both are present, and a non-compaction body is not
        // rescued by a compaction header.
        assert!(!is_local_compaction_request(
            &serde_json::json!({
                "client_metadata": { "x-codex-turn-metadata": "{\"request_kind\":\"turn\"}" }
            }),
            Some("{\"request_kind\":\"compaction\"}"),
        ));

        for kind in ["turn", "prewarm", "memory"] {
            let other = serde_json::json!({
                "input": [],
                "client_metadata": {
                    "x-codex-turn-metadata": format!("{{\"request_kind\":\"{kind}\"}}")
                }
            });
            assert!(!is_local_compaction_request(&other, None), "request_kind={kind}");
        }

        // Absent, malformed, or non-object metadata is simply not a label.
        assert!(!is_local_compaction_request(&serde_json::json!({ "input": [] }), None));
        assert!(!is_local_compaction_request(
            &serde_json::json!({ "client_metadata": { "x-codex-turn-metadata": "not json" } }),
            None,
        ));
        assert!(!is_local_compaction_request(
            &serde_json::json!({ "client_metadata": { "x-codex-turn-metadata": "[]" } }),
            None,
        ));
    }

    #[test]
    fn the_summary_request_drops_tools_and_replaces_instructions() {
        let summary = build_compaction_summary_request(&compaction_request()).unwrap();

        assert!(summary.get("tools").is_none());
        assert!(summary.get("tool_choice").is_none());
        assert!(summary.get("parallel_tool_calls").is_none());
        assert_eq!(
            summary.get("instructions").and_then(Value::as_str),
            Some(COMPACTION_SYSTEM_INSTRUCTION)
        );

        let input = summary.get("input").and_then(Value::as_array).unwrap();
        assert!(
            !input.iter().any(is_compaction_trigger_item),
            "the trigger must not reach the upstream"
        );
        assert_eq!(
            input.last().and_then(|item| item.get("role")).and_then(Value::as_str),
            Some("user")
        );
        assert_eq!(
            input
                .last()
                .and_then(|item| item.pointer("/content/0/text"))
                .and_then(Value::as_str),
            Some(COMPACTION_USER_INSTRUCTION)
        );
    }

    #[test]
    fn a_previous_turn_is_not_mistaken_for_a_compaction_request() {
        // The trigger is what marks a request; a replayed compaction item alone
        // is an ordinary follow-up turn.
        let follow_up = serde_json::json!({
            "input": [
                { "type": "compaction", "encrypted_content": "gAAAAABreal" },
                { "type": "message", "role": "user", "content": "Continue" },
            ]
        });
        assert!(!is_remote_compaction_v2_request(&follow_up));
        assert!(!carries_gateway_compaction_capsule(&follow_up));
    }

    #[test]
    fn capsules_expand_into_readable_context() {
        let item = build_compaction_item("Goal: fix auto compact", "deepseek-v4-pro").unwrap();
        let mut body = serde_json::json!({
            "input": [item, { "type": "message", "role": "user", "content": "Continue" }]
        });
        assert!(carries_gateway_compaction_capsule(&body));

        assert_eq!(expand_compaction_capsules(&mut body), 1);

        let input = body.get("input").and_then(Value::as_array).unwrap();
        assert_eq!(input[0].get("role").and_then(Value::as_str), Some("assistant"));
        let text = input[0]
            .pointer("/content/0/text")
            .and_then(Value::as_str)
            .unwrap();
        assert!(text.starts_with(COMPACTED_CONTEXT_HEADING));
        assert!(text.contains("Goal: fix auto compact"));
        assert_eq!(input[1].get("role").and_then(Value::as_str), Some("user"));
    }

    /// A real OpenAI item must survive untouched for an upstream that can read
    /// it; rewriting it would destroy server-side context.
    #[test]
    fn foreign_compaction_items_are_left_alone() {
        let foreign = serde_json::json!({
            "type": "compaction",
            "encrypted_content": "gAAAAABm1234567890abcdef",
        });
        let mut body = serde_json::json!({ "input": [foreign.clone()] });
        assert!(!carries_gateway_compaction_capsule(&body));
        assert_eq!(expand_compaction_capsules(&mut body), 0);
        assert_eq!(body.get("input").unwrap()[0], foreign);
    }

    #[test]
    fn the_summary_comes_out_of_every_target_protocol() {
        let cases = [
            serde_json::json!({
                "output": [
                    { "type": "reasoning", "summary": [] },
                    { "type": "message", "content": [{ "type": "output_text", "text": "from responses" }] },
                ]
            }),
            serde_json::json!({
                "choices": [{ "finish_reason": "stop", "message": { "content": "from chat" } }]
            }),
            serde_json::json!({
                "content": [
                    { "type": "thinking", "thinking": "ignored" },
                    { "type": "text", "text": "from anthropic" },
                ]
            }),
            serde_json::json!({
                "candidates": [{ "content": { "parts": [
                    { "thought": true, "text": "ignored" },
                    { "text": "from gemini" },
                ] } }]
            }),
        ];
        let expected = ["from responses", "from chat", "from anthropic", "from gemini"];
        for (response, expected) in cases.iter().zip(expected) {
            assert_eq!(extract_summary_text(response).as_deref(), Ok(expected));
        }
    }

    #[test]
    fn truncated_and_empty_summaries_are_rejected() {
        assert_eq!(
            extract_summary_text(&serde_json::json!({
                "choices": [{ "finish_reason": "length", "message": { "content": "half a summ" } }]
            })),
            Err(CompactionSummaryError::Truncated)
        );
        // The shape conversion produces for every source protocol, and the shape
        // Codex's own upstream answers with: partial text that must not be sealed
        // as a whole summary.
        assert_eq!(
            extract_summary_text(&serde_json::json!({
                "status": "incomplete",
                "incomplete_details": { "reason": "max_output_tokens" },
                "output": [{ "type": "message", "role": "assistant",
                             "content": [{ "type": "output_text", "text": "half a summ" }] }]
            })),
            Err(CompactionSummaryError::Truncated)
        );
        for status in ["failed", "canceled", "cancelled"] {
            assert_eq!(
                extract_summary_text(&serde_json::json!({
                    "status": status,
                    "output": [{ "type": "message", "role": "assistant",
                                 "content": [{ "type": "output_text", "text": "half a summ" }] }]
                })),
                Err(CompactionSummaryError::Failed),
                "{status}"
            );
        }
        assert_eq!(
            extract_summary_text(&serde_json::json!({
                "error": { "message": "upstream exploded" },
                "output": [{ "type": "message", "role": "assistant",
                             "content": [{ "type": "output_text", "text": "half a summ" }] }]
            })),
            Err(CompactionSummaryError::Failed)
        );
        // A healthy reported reply carries `error: null`; that is not a failure.
        assert_eq!(
            extract_summary_text(&serde_json::json!({
                "status": "completed",
                "error": null,
                "output": [{ "type": "message", "role": "assistant",
                             "content": [{ "type": "output_text", "text": "a whole summar" }] }]
            })),
            Ok("a whole summar".to_string())
        );
        assert_eq!(
            extract_summary_text(&serde_json::json!({
                "content": [{ "type": "text", "text": "   " }]
            })),
            Err(CompactionSummaryError::Empty)
        );
        assert_eq!(
            extract_summary_text(&serde_json::json!({})),
            Err(CompactionSummaryError::Empty)
        );
    }

    /// Codex counts `compaction` items only on `response.output_item.done`
    /// events; the `output` array inside `response.completed` is read as part of
    /// the terminal snapshot and does not add to the count. Exactly one `done`
    /// item is therefore what satisfies the "exactly one compaction output item"
    /// check.
    #[test]
    fn the_compaction_stream_has_exactly_one_done_item() {
        let item = build_compaction_item("summary", "m").unwrap();
        let response = build_compaction_response(&item, "m", &serde_json::json!({}));
        let stream = build_compaction_sse(&response);

        assert_eq!(stream.matches("event: response.output_item.done").count(), 1);
        assert_eq!(stream.matches("event: response.completed").count(), 1);
        assert_eq!(stream.matches("event: response.created").count(), 1);
        assert_eq!(stream.matches("event: response.in_progress").count(), 1);
        assert!(
            stream.find("response.output_item.done").unwrap()
                < stream.find("event: response.completed").unwrap(),
            "the item must land before the terminal event"
        );

        // The opening snapshots must not carry the item: a client that also
        // counts item snapshots would otherwise see more than one.
        let created = stream
            .split("event: response.created")
            .nth(1)
            .and_then(|rest| rest.split("\n\n").next())
            .unwrap();
        assert!(
            !created.contains("\"type\":\"compaction\""),
            "opening snapshot leaked the compaction item: {created}"
        );
    }

    /// The WebSocket transport has no non-streamed mode, so the summary is
    /// reassembled from deltas. Both the frame shape and the accumulation are
    /// asserted here: Codex parses WebSocket frames as bare JSON, not SSE.
    #[test]
    fn the_websocket_frames_are_bare_json_with_one_compaction_item() {
        let item = build_compaction_item("summary", "m").unwrap();
        let response =
            build_websocket_compaction_response(&item, "m", &serde_json::json!({"input_tokens": 5}));
        let frames = build_compaction_frames(&response);

        assert_eq!(frames.len(), 5);
        assert_eq!(
            frames.iter().filter(|frame| frame.contains("\"response.output_item.done\"")).count(),
            1,
        );
        assert_eq!(
            frames.iter().filter(|frame| frame.contains("\"response.completed\"")).count(),
            1,
        );
        for frame in &frames {
            assert!(
                !frame.starts_with("event:") && !frame.contains("\ndata:"),
                "WebSocket frames must be bare JSON, got: {frame}"
            );
            serde_json::from_str::<Value>(frame).expect("every frame must be a JSON object");
        }

        // The opening snapshots must not carry the item: Codex counts every
        // compaction item it sees.
        let created: Value = serde_json::from_str(&frames[0]).unwrap();
        assert!(created["response"]["output"].as_array().unwrap().is_empty());
    }

    #[test]
    fn a_streamed_summary_is_reassembled_from_deltas() {
        let mut summary = CompactionStreamSummary::default();
        summary.push(&serde_json::json!({
            "type": "response.output_text.delta",
            "delta": "Goal: ship ",
        }));
        summary.push(&serde_json::json!({
            "type": "response.output_text.delta",
            "delta": "compaction v2",
        }));
        assert!(!summary.is_terminal());
        summary.push(&serde_json::json!({
            "type": "response.completed",
            "response": { "id": "resp_1", "usage": { "input_tokens": 7, "output_tokens": 3 } },
        }));

        assert!(summary.is_terminal());
        assert_eq!(summary.finish().unwrap(), "Goal: ship compaction v2");
        assert_eq!(summary.usage()["input_tokens"], 7);
    }

    /// Some upstreams send the message item once instead of deltas.
    #[test]
    fn a_summary_without_deltas_falls_back_to_the_completed_item() {
        let mut summary = CompactionStreamSummary::default();
        summary.push(&serde_json::json!({
            "type": "response.completed",
            "response": {
                "id": "resp_1",
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "Summary from the item." }],
                }],
            },
        }));

        assert_eq!(summary.finish().unwrap(), "Summary from the item.");
    }

    /// Some upstreams send neither deltas nor a filled terminal snapshot — only
    /// the finished `message` item on `response.output_item.done`. The text has
    /// to be read off the item itself, which carries no `output` array.
    #[test]
    fn a_summary_reassembles_from_the_done_item_alone() {
        let mut summary = CompactionStreamSummary::default();
        summary.push(&serde_json::json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "Summary from the item." }],
            },
        }));
        summary.push(&serde_json::json!({
            "type": "response.completed",
            "response": { "id": "resp_1" },
        }));

        assert!(summary.is_terminal());
        assert_eq!(summary.finish().unwrap(), "Summary from the item.");
    }

    /// A `response.completed` snapshot can carry the same verdicts as the
    /// dedicated events; a relay that folds `status: "incomplete"` or a
    /// non-empty `error` into the terminal snapshot must not have its draft
    /// sealed as the whole history.
    #[test]
    fn a_completed_snapshot_reporting_failure_or_truncation_is_not_sealed() {
        let mut failed = CompactionStreamSummary::default();
        failed.push(&serde_json::json!({
            "type": "response.output_text.delta",
            "delta": "half a summ",
        }));
        failed.push(&serde_json::json!({
            "type": "response.completed",
            "response": {
                "id": "resp_1",
                "status": "failed",
                "error": { "code": "server_error", "message": "transient" },
            },
        }));
        assert_eq!(failed.finish(), Err(CompactionSummaryError::Failed));

        let mut truncated = CompactionStreamSummary::default();
        truncated.push(&serde_json::json!({
            "type": "response.output_text.delta",
            "delta": "half a summ",
        }));
        truncated.push(&serde_json::json!({
            "type": "response.completed",
            "response": { "id": "resp_1", "status": "incomplete" },
        }));
        assert_eq!(
            truncated.finish(),
            Err(CompactionSummaryError::Truncated)
        );

        // A healthy snapshot stays a summary.
        let mut healthy = CompactionStreamSummary::default();
        healthy.push(&serde_json::json!({
            "type": "response.output_text.delta",
            "delta": "a whole summary",
        }));
        healthy.push(&serde_json::json!({
            "type": "response.completed",
            "response": { "id": "resp_1", "status": "completed" },
        }));
        assert_eq!(healthy.finish().unwrap(), "a whole summary");
    }

    /// The pass-through caller consults this on a reply that is not a native
    /// item: a failed or truncated turn is not a refusal and must not be
    /// remembered as one.
    #[test]
    fn failed_or_truncated_pass_through_replies_are_told_apart_from_refusals() {
        for body in [
            serde_json::to_vec(&serde_json::json!({
                "status": "failed",
                "error": { "code": "server_error", "message": "transient" },
                "output": [{ "type": "message", "content": [{ "type": "output_text", "text": "draft" }] }],
            }))
            .unwrap(),
            serde_json::to_vec(&serde_json::json!({
                "status": "incomplete",
                "output": [{ "type": "message", "content": [{ "type": "output_text", "text": "draft" }] }],
            }))
            .unwrap(),
        ] {
            assert!(
                is_failed_or_truncated_response(&body),
                "a failed or truncated turn is not a refusal"
            );
        }

        for body in [
            serde_json::to_vec(&serde_json::json!({
                "status": "completed",
                "output": [{ "type": "message", "content": [{ "type": "output_text", "text": "reply" }] }],
            }))
            .unwrap(),
            b"not json".to_vec(),
        ] {
            assert!(
                !is_failed_or_truncated_response(&body),
                "an ordinary reply is a refusal"
            );
        }
    }

    /// A pass-through turn is decided from what the upstream streamed: its own
    /// `compaction` item, or anything else.
    #[test]
    fn an_upstream_compaction_item_is_told_apart_from_an_ordinary_message() {
        let mut relay = CompactionStreamSummary::default();
        relay.push(&serde_json::json!({
            "type": "response.output_item.added",
            "item": { "type": "message", "role": "assistant" },
        }));
        relay.push(&serde_json::json!({
            "type": "response.output_text.delta",
            "delta": "Here is a summary",
        }));
        relay.push(&serde_json::json!({
            "type": "response.completed",
            "response": {
                "id": "resp_1",
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "Here is a summary" }],
                }],
            },
        }));
        assert!(relay.is_terminal());
        assert!(!relay.has_native_item());

        // The announcement alone is enough: an upstream that says it is writing
        // the item is not about to write an ordinary message instead.
        let mut announced = CompactionStreamSummary::default();
        announced.push(&serde_json::json!({
            "type": "response.output_item.added",
            "item": { "type": "compaction" },
        }));
        assert!(announced.has_native_item());

        // Some upstreams skip the per-item events and only report the finished
        // item in the terminal.
        let mut terminal_only = CompactionStreamSummary::default();
        terminal_only.push(&serde_json::json!({
            "type": "response.completed",
            "response": {
                "id": "resp_1",
                "output": [{ "type": "compaction", "encrypted_content": "gAAAAABq-cipher" }],
            },
        }));
        assert!(terminal_only.has_native_item());
    }

    /// An unusable summary must never become an item. Codex installs the item as
    /// the entire conversation history, so a placeholder would discard the
    /// context instead of surfacing the failure; the caller forwards the
    /// upstream's own terminal instead.
    #[test]
    fn a_failed_or_empty_summary_yields_no_item() {
        let mut failed = CompactionStreamSummary::default();
        failed.push(&serde_json::json!({
            "type": "response.failed",
            "response": { "status": "failed" },
        }));
        assert!(failed.is_terminal());
        assert!(failed.finish().is_err());

        let mut truncated = CompactionStreamSummary::default();
        truncated.push(&serde_json::json!({ "type": "response.incomplete" }));
        assert!(truncated.is_terminal());
        assert_eq!(truncated.finish(), Err(CompactionSummaryError::Truncated));

        let mut empty = CompactionStreamSummary::default();
        empty.push(&serde_json::json!({
            "type": "response.completed",
            "response": { "id": "resp_1" },
        }));
        assert_eq!(empty.finish(), Err(CompactionSummaryError::Empty));
    }

    fn route(path: &str) -> GatewayRoute {
        GatewayRoute {
            cli_key: GatewayCliKey::Codex,
            route_name: "test",
            forwarded_path: path.to_string(),
            query: None,
        }
    }

    /// The whole point of the feature: a compaction request is summarized
    /// upstream, the reply becomes an opaque item, and the next turn replays
    /// that item as readable context instead of a bare capsule.
    #[test]
    fn a_compaction_request_round_trips_through_a_later_turn() {
        let request_body = serde_json::to_vec(&compaction_request()).unwrap();
        assert_eq!(
            RemoteCompactionCompat::resolve(&route("/v1/responses"), None, true, &request_body),
            RemoteCompactionCompat::Summarize
        );

        // The upstream answers with an ordinary summary in the client's shape.
        let summary_reply = serde_json::json!({
            "id": "resp_1",
            "model": "deepseek-v4-pro",
            "output": [{
                "type": "message",
                "content": [{ "type": "output_text", "text": "Goal: fix auto compact" }],
            }],
            "usage": { "input_tokens": 1200, "output_tokens": 80, "total_tokens": 1280 },
        });
        let summary = extract_summary_text(&summary_reply).expect("summary must be readable");
        let item = build_compaction_item(&summary, "deepseek-v4-pro").unwrap();
        let compaction_response = build_compaction_response(&item, "deepseek-v4-pro", &summary_reply);

        let items = compaction_response["output"].as_array().unwrap();
        assert_eq!(items.len(), 1, "Codex requires exactly one output item");
        assert_eq!(items[0]["type"], "compaction");
        assert!(is_compaction_capsule(
            items[0]["encrypted_content"].as_str().unwrap()
        ));
        assert_eq!(compaction_response["usage"]["input_tokens"], 1200);
        assert_eq!(compaction_response["usage"]["total_tokens"], 1280);

        // Codex stores that item and replays it on the next turn.
        let next_turn = serde_json::json!({
            "input": [
                items[0].clone(),
                { "type": "message", "role": "user", "content": "Continue" },
            ]
        });
        let next_turn_body = serde_json::to_vec(&next_turn).unwrap();
        assert_eq!(
            RemoteCompactionCompat::resolve(&route("/v1/responses"), None, true, &next_turn_body),
            RemoteCompactionCompat::ReplayCapsules
        );

        let mut upstream_body: Value = serde_json::from_slice(&next_turn_body).unwrap();
        assert_eq!(expand_compaction_capsules(&mut upstream_body), 1);
        let input = upstream_body["input"].as_array().unwrap();
        let restored = input[0]["content"][0]["text"].as_str().unwrap();
        assert!(restored.contains("Goal: fix auto compact"));
        assert_eq!(input[1]["role"], "user");
    }

    fn responses_provider() -> UpstreamProvider {
        UpstreamProvider {
            cli_key: GatewayCliKey::Codex,
            id: "provider-1".to_string(),
            name: "Native upstream".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            target_protocol: AiProtocol::OpenAiResponses,
            supports_websockets: None,
            auth_strategy: ProviderAuthStrategy::Bearer,
            is_full_url: false,
            sort_index: None,
            meta: ProviderGatewayMeta::default(),
            model_mapping: Default::default(),
        }
    }

    /// A Responses target is offered the trigger first, exactly as Codex's own
    /// upstream would be; that is the default, with nothing to switch on.
    #[test]
    fn a_responses_target_is_offered_the_trigger_first() {
        let body = serde_json::to_vec(&compaction_request()).unwrap();
        let route = route("/v1/responses");

        let provider = responses_provider();
        assert_eq!(
            RemoteCompactionCompat::resolve(&route, Some(&provider), true, &body),
            RemoteCompactionCompat::Passthrough
        );

        // A configuration that already refused one goes straight to the
        // Gateway's own summary.
        assert_eq!(
            RemoteCompactionCompat::resolve(&route, Some(&provider), false, &body),
            RemoteCompactionCompat::Summarize
        );

        // No provider resolved at all (a route with nothing to forward to) still
        // defaults to the Gateway answering.
        assert_eq!(
            RemoteCompactionCompat::resolve(&route, None, true, &body),
            RemoteCompactionCompat::Summarize
        );
    }

    /// v2 is defined on the Responses wire, so a provider reached through a
    /// protocol conversion cannot implement it natively. Offering it the trigger
    /// would spend a request on an answer it cannot give.
    #[test]
    fn a_converted_provider_never_passes_the_trigger_through() {
        let mut provider = responses_provider();
        provider.target_protocol = AiProtocol::OpenAiChat;
        let body = serde_json::to_vec(&compaction_request()).unwrap();
        assert_eq!(
            RemoteCompactionCompat::resolve(&route("/v1/responses"), Some(&provider), true, &body),
            RemoteCompactionCompat::Summarize
        );
    }

    /// The relay buffers a pass-through turn's frames, so it can offer the
    /// trigger too and still decide afterwards.
    #[test]
    fn the_websocket_relay_is_offered_the_trigger_too() {
        let provider = responses_provider();
        assert_eq!(
            RemoteCompactionCompat::resolve_websocket(Some(&provider), true, &compaction_request()),
            RemoteCompactionCompat::Passthrough
        );
        assert_eq!(
            RemoteCompactionCompat::resolve_websocket(Some(&provider), false, &compaction_request()),
            RemoteCompactionCompat::Summarize
        );
    }

    /// Only an upstream that really implements v2 may reach Codex through the
    /// pass-through branch. Everything an unimplementing upstream returns — an
    /// ordinary assistant message, an empty or doubled reply — is rejected, and
    /// the caller falls back to sealing its own item.
    #[test]
    fn only_one_ciphertext_compaction_item_counts_as_native() {
        let native = serde_json::to_vec(&serde_json::json!({
            "id": "resp_1",
            "output": [{ "type": "compaction", "encrypted_content": "gAAAAABq-cipher" }],
        }))
        .unwrap();
        assert!(is_native_compaction_response(&native));

        let cases = [
            // A relay that ignored the trigger and answered normally.
            serde_json::json!({
                "id": "resp_1",
                "output": [{
                    "type": "message",
                    "content": [{ "type": "output_text", "text": "Here is a summary" }],
                }],
            }),
            serde_json::json!({ "id": "resp_1", "output": [] }),
            serde_json::json!({
                "id": "resp_1",
                "output": [
                    { "type": "compaction", "encrypted_content": "cipher-a" },
                    { "type": "compaction", "encrypted_content": "cipher-b" },
                ],
            }),
            // A compaction item with nothing to replay.
            serde_json::json!({
                "id": "resp_1",
                "output": [{ "type": "compaction", "encrypted_content": "   " }],
            }),
            serde_json::json!({
                "id": "resp_1",
                "output": [{ "type": "compaction" }],
            }),
            serde_json::json!({ "id": "resp_1" }),
        ];
        for case in cases {
            let body = serde_json::to_vec(&case).unwrap();
            assert!(
                !is_native_compaction_response(&body),
                "must not accept {case}"
            );
        }
        assert!(!is_native_compaction_response(b"not json"));
    }

    /// A non-Codex CLI must never enter the compaction path, even if its body
    /// happens to contain a lookalike item.
    #[test]
    fn other_clis_are_never_compaction() {
        let mut claude_route = route("/v1/responses");
        claude_route.cli_key = GatewayCliKey::Claude;
        let body = serde_json::to_vec(&compaction_request()).unwrap();
        assert_eq!(
            RemoteCompactionCompat::resolve(&claude_route, None, true, &body),
            RemoteCompactionCompat::None
        );
    }

    /// The legacy endpoint stays with `CodexResponsesCompactCompat`; claiming it
    /// here would rewrite the same request twice.
    #[test]
    fn the_legacy_compact_endpoint_is_not_claimed() {
        let body = serde_json::to_vec(&serde_json::json!({ "input": [] })).unwrap();
        assert_eq!(
            RemoteCompactionCompat::resolve(&route("/v1/responses/compact"), None, true, &body),
            RemoteCompactionCompat::None
        );
    }

    #[test]
    fn usage_survives_from_every_upstream_shape() {
        let cases = [
            (serde_json::json!({ "usage": { "input_tokens": 10, "output_tokens": 2 } }), (10, 2, 12)),
            (
                serde_json::json!({ "usage": { "prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10 } }),
                (7, 3, 10),
            ),
            (
                serde_json::json!({ "usageMetadata": { "promptTokenCount": 4, "candidatesTokenCount": 1 } }),
                (4, 1, 5),
            ),
        ];
        for (upstream, (input, output, total)) in cases {
            let usage = compaction_usage(&upstream);
            assert_eq!(usage["input_tokens"], input);
            assert_eq!(usage["output_tokens"], output);
            assert_eq!(usage["total_tokens"], total);
        }
    }
}
