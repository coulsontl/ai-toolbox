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

/// One auxiliary task Codex can emit. `Other` keeps the raw marker for the
/// detail dialog but never participates in routing.
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
    Other(String),
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
            Self::Review | Self::Compact | Self::Other(_) => CodexAuxTaskKind::Other,
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

    /// Stable wire-level label for the detail dialog. Unknown markers are
    /// clipped so a hostile/custom header cannot bloat the JSONL detail.
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
            Self::Other(raw) => raw.chars().take(40).collect(),
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
        return Some(match normalized.as_str() {
            "guardian" | "guardian_review" | "auto_review" => CodexAuxTask::AutoReview,
            "review" => CodexAuxTask::Review,
            "compact" | "compaction" => CodexAuxTask::Compact,
            "memory_consolidation" => CodexAuxTask::Memory,
            _ => CodexAuxTask::Other(subagent.to_string()),
        });
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
        return Some(match thread_source {
            "thread_title" | "thread_title_reconsideration" | "title_generation" => {
                CodexAuxTask::Title
            }
            "thread_description" => CodexAuxTask::Description,
            "thread_summary" => CodexAuxTask::Summary,
            "composer_predictions" => CodexAuxTask::Prediction,
            "guardian_review" => CodexAuxTask::AutoReview,
            "memory_consolidation" => CodexAuxTask::Memory,
            other => CodexAuxTask::Other(other.to_string()),
        });
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
    fn subagent_markers_win_and_unknown_markers_are_kept() {
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
        assert_eq!(
            classify_codex_aux_parts(&headers(&[("x-openai-subagent", "collab_spawn")]), b""),
            Some(CodexAuxTask::Other("collab_spawn".to_string()))
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
    fn unknown_thread_source_is_other_and_broken_json_is_ignored() {
        let body = body_with_metadata(serde_json::json!({ "thread_source": "future_kind" }));
        assert_eq!(
            classify_codex_aux_parts(&[], &body),
            Some(CodexAuxTask::Other("future_kind".to_string()))
        );
        assert_eq!(classify_codex_aux_parts(&[], b"not json"), None);
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
    fn unknown_raw_kinds_are_clipped_for_logs() {
        let long = "x".repeat(100);
        let task = CodexAuxTask::Other(long);
        assert_eq!(task.raw_kind().len(), 40);
    }
}
