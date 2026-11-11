//! Classification of Codex's hidden auxiliary calls.
//!
//! Codex marks these calls with a mix of headers and the canonical
//! `client_metadata["x-codex-turn-metadata"]` body field. The classifier is
//! deliberately read-only and tolerant: an unreadable marker means "ordinary
//! turn", never a failed request.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::coding::proxy_gateway::types::CodexAuxTaskKind;

use super::super::http_io::DebugHttpRequest;

/// One auxiliary task Codex can emit. Classification is a whitelist: only
/// markers known to denote a real auxiliary/background call produce a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CodexAuxTask {
    Title,
    Description,
    Summary,
    Prediction,
    Memory,
    AutoReview,
    Review,
    Compact,
}

impl CodexAuxTask {
    pub(crate) fn kind(&self) -> CodexAuxTaskKind {
        match self {
            Self::Title => CodexAuxTaskKind::Title,
            Self::Description => CodexAuxTaskKind::Description,
            Self::Summary => CodexAuxTaskKind::Summary,
            Self::Prediction => CodexAuxTaskKind::Prediction,
            Self::Memory => CodexAuxTaskKind::Memory,
            Self::AutoReview => CodexAuxTaskKind::AutoReview,
            Self::Review | Self::Compact => CodexAuxTaskKind::Other,
        }
    }

    /// The persisted key for `settingsConfig.auxiliaryModels`. Only tasks with a
    /// user-configurable route have one; auto-review keeps using its own field.
    pub(crate) fn route_key(&self) -> Option<&'static str> {
        match self {
            Self::Title => Some("title"),
            Self::Description => Some("description"),
            Self::Summary => Some("summary"),
            Self::Prediction => Some("prediction"),
            Self::Memory => Some("memory"),
            _ => None,
        }
    }

    /// Tasks whose Responses output is wrapped back into the JSON object Codex
    /// expects when the target protocol never showed the schema to the model.
    pub(crate) fn is_shaped(&self) -> bool {
        matches!(
            self,
            Self::Title | Self::Description | Self::Summary | Self::Prediction
        )
    }

    /// Stable wire-level label for the detail dialog.
    pub(crate) fn raw_kind(&self) -> String {
        match self {
            Self::Title => "thread_title".to_string(),
            Self::Description => "thread_description".to_string(),
            Self::Summary => "thread_summary".to_string(),
            Self::Prediction => "composer_predictions".to_string(),
            Self::Memory => "memory_consolidation".to_string(),
            Self::AutoReview => "auto_review".to_string(),
            Self::Review => "review".to_string(),
            Self::Compact => "compact".to_string(),
        }
    }
}

/// Provider-level auxiliary route stored in
/// `settingsConfig.auxiliaryModels.<task>`.
///
/// `Main` deliberately stores no model name: it refers to the provider's
/// current `default_model`, so changing the main model keeps the route live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum CodexAuxiliaryModelRoute {
    Main,
    Model { model: String },
}

impl CodexAuxiliaryModelRoute {
    pub(crate) fn configured_mode(&self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Model { .. } => "model",
        }
    }
}

/// Classify an HTTP request. Returns `None` when nothing marks it as auxiliary.
pub(crate) fn classify_codex_aux_task(request: &DebugHttpRequest) -> Option<CodexAuxTask> {
    classify_codex_aux_parts(&request.headers, &request.body)
}

/// Classifier shared by HTTP and WebSocket turns. WebSocket frames have no
/// per-request headers, so the body's canonical metadata is the only source
/// there.
pub(crate) fn classify_codex_aux_parts(
    headers: &[(String, String)],
    body: &[u8],
) -> Option<CodexAuxTask> {
    let subagent = header_value(headers, "x-openai-subagent")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let memgen = header_value(headers, "x-openai-memgen-request")
        .map(str::trim)
        .filter(|value| value.eq_ignore_ascii_case("true") || *value == "1");
    let body_value = serde_json::from_slice::<Value>(body).ok();
    let metadata = turn_metadata(headers, body_value.as_ref());

    if let Some(subagent) = subagent {
        let normalized = subagent.to_ascii_lowercase();
        return match normalized.as_str() {
            "guardian" | "guardian_review" | "auto_review" => Some(CodexAuxTask::AutoReview),
            "review" => Some(CodexAuxTask::Review),
            "compact" | "compaction" => Some(CodexAuxTask::Compact),
            "memory_consolidation" => Some(CodexAuxTask::Memory),
            // `collab_spawn` is a user-visible spawned agent turn, not a hidden
            // auxiliary call. Unknown labels are not evidence either: never
            // guess, or ordinary traffic gets an "auxiliary" badge (issue #460).
            _ => None,
        };
    }

    if memgen.is_some() {
        return Some(CodexAuxTask::Memory);
    }

    let thread_source = metadata
        .as_ref()
        .and_then(|value| value.get("thread_source"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(thread_source) = thread_source {
        match thread_source {
            "thread_title" | "thread_title_reconsideration" | "title_generation" => {
                return Some(CodexAuxTask::Title);
            }
            "thread_description" => return Some(CodexAuxTask::Description),
            "thread_summary" => return Some(CodexAuxTask::Summary),
            "composer_predictions" => return Some(CodexAuxTask::Prediction),
            "guardian_review" => return Some(CodexAuxTask::AutoReview),
            "memory_consolidation" => return Some(CodexAuxTask::Memory),
            // `user`, `subagent`, client feature labels and unknown values all
            // describe ordinary turns. Fall through so the request-kind
            // fallback below still sees a memory request.
            _ => {}
        }
    }

    if metadata
        .as_ref()
        .and_then(|value| value.get("request_kind"))
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("memory"))
    {
        return Some(CodexAuxTask::Memory);
    }

    None
}

fn turn_metadata(headers: &[(String, String)], body: Option<&Value>) -> Option<Value> {
    let body_metadata = body
        .and_then(|value| value.get("client_metadata"))
        .and_then(|value| value.get("x-codex-turn-metadata"));
    match body_metadata {
        Some(Value::Object(_)) => return body_metadata.cloned(),
        Some(Value::String(raw)) => return serde_json::from_str(raw).ok(),
        _ => {}
    }
    header_value(headers, "x-codex-turn-metadata").and_then(|raw| serde_json::from_str(raw).ok())
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect()
    }

    fn body_with_metadata(metadata: Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "client_metadata": { "x-codex-turn-metadata": metadata.to_string() }
        }))
        .expect("serialize body")
    }

    #[test]
    fn subagent_markers_map_known_tasks_and_spawns_stay_ordinary() {
        assert_eq!(
            classify_codex_aux_parts(&headers(&[("x-openai-subagent", "guardian")]), b""),
            Some(CodexAuxTask::AutoReview)
        );
        assert_eq!(
            classify_codex_aux_parts(&headers(&[("x-openai-subagent", "review")]), b""),
            Some(CodexAuxTask::Review)
        );
        assert_eq!(
            classify_codex_aux_parts(&headers(&[("x-openai-subagent", "compact")]), b""),
            Some(CodexAuxTask::Compact)
        );
        // `collab_spawn` is a spawned agent, not a hidden auxiliary call; an
        // unknown label must not be guessed either (issue #460).
        for marker in ["collab_spawn", "future_subagent"] {
            assert_eq!(
                classify_codex_aux_parts(&headers(&[("x-openai-subagent", marker)]), b""),
                None,
                "marker {marker}"
            );
        }
    }

    #[test]
    fn subagent_header_wins_over_body_thread_source() {
        let body = body_with_metadata(serde_json::json!({ "thread_source": "thread_title" }));
        assert_eq!(
            classify_codex_aux_parts(&headers(&[("x-openai-subagent", "review")]), &body),
            Some(CodexAuxTask::Review)
        );
    }

    #[test]
    fn memgen_header_marks_memory_only_when_true() {
        assert_eq!(
            classify_codex_aux_parts(&headers(&[("x-openai-memgen-request", "true")]), b""),
            Some(CodexAuxTask::Memory)
        );
        assert_eq!(
            classify_codex_aux_parts(&headers(&[("x-openai-memgen-request", "false")]), b""),
            None
        );
    }

    #[test]
    fn thread_sources_map_to_tasks() {
        for (source, expected) in [
            ("thread_title", CodexAuxTask::Title),
            ("thread_title_reconsideration", CodexAuxTask::Title),
            ("title_generation", CodexAuxTask::Title),
            ("thread_description", CodexAuxTask::Description),
            ("thread_summary", CodexAuxTask::Summary),
            ("composer_predictions", CodexAuxTask::Prediction),
            ("guardian_review", CodexAuxTask::AutoReview),
            ("memory_consolidation", CodexAuxTask::Memory),
        ] {
            let body = body_with_metadata(serde_json::json!({ "thread_source": source }));
            assert_eq!(
                classify_codex_aux_parts(&[], &body),
                Some(expected),
                "source {source}"
            );
        }
    }

    #[test]
    fn request_kind_memory_is_a_fallback() {
        let body = body_with_metadata(serde_json::json!({ "request_kind": "memory" }));
        assert_eq!(
            classify_codex_aux_parts(&[], &body),
            Some(CodexAuxTask::Memory)
        );
    }

    #[test]
    fn turn_metadata_header_is_supported() {
        let title = serde_json::json!({ "thread_source": "thread_title" }).to_string();
        assert_eq!(
            classify_codex_aux_parts(&headers(&[("x-codex-turn-metadata", title.as_str())]), b""),
            Some(CodexAuxTask::Title)
        );
        let ordinary = serde_json::json!({ "thread_source": "user" }).to_string();
        assert_eq!(
            classify_codex_aux_parts(
                &headers(&[("x-codex-turn-metadata", ordinary.as_str())]),
                b""
            ),
            None
        );
    }

    #[test]
    fn body_metadata_accepts_an_object_too() {
        let body = serde_json::to_vec(&serde_json::json!({
            "client_metadata": {
                "x-codex-turn-metadata": { "thread_source": "thread_title" }
            }
        }))
        .expect("serialize body");
        assert_eq!(
            classify_codex_aux_parts(&[], &body),
            Some(CodexAuxTask::Title)
        );
    }

    #[test]
    fn ordinary_thread_sources_and_broken_json_are_ignored() {
        // Codex emits `user` for ordinary app turns and `subagent` for spawned
        // agents; client feature labels are equally ordinary. None of them may
        // produce the "auxiliary" badge (issue #460).
        for source in [
            "user",
            "subagent",
            "automation",
            "voice_chat",
            "future_kind",
        ] {
            let body = body_with_metadata(serde_json::json!({ "thread_source": source }));
            assert_eq!(
                classify_codex_aux_parts(&[], &body),
                None,
                "source {source}"
            );
        }
        assert_eq!(classify_codex_aux_parts(&[], b"not json"), None);
    }

    #[test]
    fn unknown_thread_source_still_falls_back_to_request_kind_memory() {
        let body = body_with_metadata(serde_json::json!({
            "thread_source": "future_kind",
            "request_kind": "memory"
        }));
        assert_eq!(
            classify_codex_aux_parts(&[], &body),
            Some(CodexAuxTask::Memory)
        );
    }

    #[test]
    fn routable_keys_and_shaping_are_explicit() {
        assert_eq!(CodexAuxTask::Title.route_key(), Some("title"));
        assert_eq!(CodexAuxTask::Memory.route_key(), Some("memory"));
        assert_eq!(CodexAuxTask::AutoReview.route_key(), None);
        assert!(CodexAuxTask::Prediction.is_shaped());
        assert!(!CodexAuxTask::Memory.is_shaped());
    }

    #[test]
    fn review_and_compact_keep_stable_raw_kinds() {
        assert_eq!(CodexAuxTask::Review.raw_kind(), "review");
        assert_eq!(CodexAuxTask::Compact.raw_kind(), "compact");
    }
}
