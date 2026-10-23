pub const THINKING_BUDGET_MINIMAL: i64 = 1_024;
pub const THINKING_BUDGET_LOW: i64 = 4_096;
pub const THINKING_BUDGET_MEDIUM: i64 = 10_240;
pub const THINKING_BUDGET_HIGH: i64 = 32_768;

pub fn reasoning_effort_to_budget_tokens(effort: &str, max_tokens: Option<i64>) -> Option<i64> {
    match normalize_reasoning_effort(effort).as_deref() {
        Some("none") => Some(0),
        Some("minimal") => Some(THINKING_BUDGET_MINIMAL),
        Some("low") => Some(THINKING_BUDGET_LOW),
        Some("medium") => Some(THINKING_BUDGET_MEDIUM),
        Some("high") => Some(THINKING_BUDGET_HIGH),
        Some("xhigh" | "max") => Some(max_tokens.unwrap_or(THINKING_BUDGET_HIGH)),
        _ => None,
    }
}

pub fn budget_tokens_to_reasoning_effort(budget_tokens: i64) -> &'static str {
    if budget_tokens <= 0 {
        "none"
    } else if budget_tokens <= THINKING_BUDGET_MINIMAL {
        "minimal"
    } else if budget_tokens <= THINKING_BUDGET_LOW {
        "low"
    } else if budget_tokens <= THINKING_BUDGET_MEDIUM {
        "medium"
    } else if budget_tokens <= THINKING_BUDGET_HIGH {
        "high"
    } else {
        "xhigh"
    }
}

pub fn normalize_reasoning_effort(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "none" | "off" | "disabled" => Some("none".to_string()),
        "minimal" | "min" => Some("minimal".to_string()),
        "low" => Some("low".to_string()),
        "medium" => Some("medium".to_string()),
        "high" => Some("high".to_string()),
        "xhigh" | "extra_high" | "max" => Some(normalized),
        _ => None,
    }
}

/// How an Anthropic Messages target has to spell the thinking configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnthropicThinkingPlan {
    /// `thinking: {type: "disabled"}` — the model accepts an explicit off switch.
    Disabled,
    /// Legacy manual thinking: `thinking: {type: "enabled", budget_tokens}`.
    Budget { budget_tokens: i64 },
    /// `thinking: {type: "adaptive"}` plus `output_config.effort`.
    Adaptive { effort: &'static str },
    /// Only `output_config.effort`, leaving `thinking` out of the request.
    /// The 5.x generation rejects `disabled`, and omitting the field is the one
    /// spelling every adaptive model accepts.
    EffortOnly { effort: &'static str },
}

/// Claude models that still require the legacy `enabled` + `budget_tokens`
/// pair, i.e. the 4.5 generation and older.
///
/// This is deliberately an allowlist of the *old* spellings: every id that is
/// not listed here is treated as adaptive thinking. Claude 4.6 replaced manual
/// budgets with `thinking.type=adaptive` + `output_config.effort`, and 4.7+
/// answer 400 to the legacy pair (issue #437). Keeping the list on the legacy
/// side means a newly released Claude model works without touching this code.
const LEGACY_THINKING_BUDGET_MODEL_MARKERS: &[&str] = &[
    "claude-3",           // claude-3-7-sonnet, claude-3-5-haiku, ...
    "claude-opus-4-2025", // claude-opus-4-20250514 (the 4.0 release)
    "claude-opus-4-1",
    "claude-opus-4-5",
    "claude-sonnet-4-2025", // claude-sonnet-4-20250514
    "claude-sonnet-4-0",
    "claude-sonnet-4-5",
    "claude-haiku-4-5",
];

/// Lowercases the id and folds the `.` / `_` spellings relays like to use
/// (`claude-sonnet-4.6`, `claude_opus_4_5`) onto the canonical hyphen form.
fn canonical_model_id(model: &str) -> String {
    model.trim().to_ascii_lowercase().replace(['.', '_'], "-")
}

fn uses_legacy_thinking_budget(model: &str) -> bool {
    let normalized = canonical_model_id(model);
    LEGACY_THINKING_BUDGET_MODEL_MARKERS
        .iter()
        .any(|marker| normalized.contains(marker))
}

/// Translates an OpenAI-style reasoning effort into the thinking fields an
/// Anthropic Messages upstream expects for `model`.
///
/// `None` means the effort was not understood, so no thinking field is written
/// (the same silent skip the budget mapping always had).
pub fn anthropic_thinking_plan(
    model: &str,
    reasoning_effort: &str,
    max_tokens: i64,
) -> Option<AnthropicThinkingPlan> {
    let effort = normalize_reasoning_effort(reasoning_effort)?;
    if uses_legacy_thinking_budget(model) {
        if effort == "none" {
            return Some(AnthropicThinkingPlan::Disabled);
        }
        return reasoning_effort_to_budget_tokens(&effort, Some(max_tokens))
            .map(|budget_tokens| AnthropicThinkingPlan::Budget { budget_tokens });
    }

    // Asking for "no thinking" on an adaptive model can only lower the effort:
    // 5.x models reject `disabled`, and omitting `thinking` is accepted by all
    // of them (it means "adaptive", or "off" on 4.7/4.8).
    if effort == "none" {
        return Some(AnthropicThinkingPlan::EffortOnly { effort: "low" });
    }
    Some(AnthropicThinkingPlan::Adaptive {
        effort: adaptive_effort(&effort, supports_xhigh_effort(model)),
    })
}

/// Anthropic accepts `low`/`medium`/`high`/`xhigh`/`max`; `minimal` collapses
/// to `low`, and `xhigh` falls back to `high` on models that do not list it.
fn adaptive_effort(effort: &str, supports_xhigh: bool) -> &'static str {
    match effort {
        "medium" => "medium",
        "high" => "high",
        "xhigh" if supports_xhigh => "xhigh",
        "xhigh" => "high",
        "max" => "max",
        _ => "low",
    }
}

/// Claude 4.6 is adaptive but predates the `xhigh` level, and Codex defaults to
/// `xhigh`, so this one pair needs an explicit cap.
fn supports_xhigh_effort(model: &str) -> bool {
    let normalized = canonical_model_id(model);
    !(normalized.contains("claude-opus-4-6") || normalized.contains("claude-sonnet-4-6"))
}

#[cfg(test)]
mod thinking_plan_tests {
    use super::*;

    fn plan(model: &str, effort: &str) -> Option<AnthropicThinkingPlan> {
        anthropic_thinking_plan(model, effort, 64_000)
    }

    #[test]
    fn adaptive_models_never_get_a_legacy_thinking_pair() {
        for model in [
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-fable-5-1",
            "claude-haiku-5-5",
            "claude-opus-4-7",
            "a-model-released-next-year",
        ] {
            assert_eq!(
                plan(model, "medium"),
                Some(AnthropicThinkingPlan::Adaptive { effort: "medium" }),
                "{model}"
            );
        }
    }

    #[test]
    fn no_thinking_on_an_adaptive_model_only_lowers_the_effort() {
        for model in ["claude-opus-5-5", "claude-opus-4-7", "claude-haiku-5-5"] {
            assert_eq!(
                plan(model, "none"),
                Some(AnthropicThinkingPlan::EffortOnly { effort: "low" }),
                "{model}"
            );
        }
    }

    #[test]
    fn xhigh_falls_back_on_models_that_do_not_list_it() {
        assert_eq!(
            plan("claude-opus-4-6", "xhigh"),
            Some(AnthropicThinkingPlan::Adaptive { effort: "high" })
        );
        assert_eq!(
            plan("claude-opus-4-7", "xhigh"),
            Some(AnthropicThinkingPlan::Adaptive { effort: "xhigh" })
        );
    }

    #[test]
    fn legacy_models_keep_the_budget_mapping() {
        assert_eq!(
            plan("claude-sonnet-4-5-20250929", "medium"),
            Some(AnthropicThinkingPlan::Budget {
                budget_tokens: 10_240
            })
        );
        assert_eq!(
            plan("claude-opus-4-5-20251101", "none"),
            Some(AnthropicThinkingPlan::Disabled)
        );
        assert_eq!(
            plan("claude-3-7-sonnet-latest", "low"),
            Some(AnthropicThinkingPlan::Budget {
                budget_tokens: 4_096
            })
        );
    }

    #[test]
    fn vendor_prefixed_ids_still_resolve_their_generation() {
        assert_eq!(
            plan("us.anthropic.claude-opus-4-7", "high"),
            Some(AnthropicThinkingPlan::Adaptive { effort: "high" })
        );
        assert_eq!(
            plan("gemini-claude-opus-4-5-thinking", "high"),
            Some(AnthropicThinkingPlan::Budget {
                budget_tokens: 32_768
            })
        );
    }

    #[test]
    fn unknown_efforts_write_no_thinking_field() {
        assert_eq!(plan("claude-opus-5-5", "turbo"), None);
        assert_eq!(plan("claude-sonnet-4-5-20250929", "turbo"), None);
    }

    #[test]
    fn dotted_and_underscored_spellings_resolve_like_the_canonical_id() {
        assert_eq!(
            plan("claude-sonnet-4.5", "medium"),
            Some(AnthropicThinkingPlan::Budget {
                budget_tokens: 10_240
            })
        );
        assert_eq!(
            plan("claude-opus-4.6", "xhigh"),
            Some(AnthropicThinkingPlan::Adaptive { effort: "high" })
        );
        assert_eq!(
            plan("claude_opus_4_5", "none"),
            Some(AnthropicThinkingPlan::Disabled)
        );
    }
}
