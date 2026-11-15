//! Build Paseo model entries from an applied AI Toolbox provider.
//!
//! Paseo's `models` override is a replacement list. Each entry accepts
//! `{ id, label, description?, isDefault?, thinkingOptions? }`; only the fields
//! with a real source are emitted.

use std::collections::HashSet;

use serde::Serialize;
use serde_json::{Map, Value};

use super::PaseoProvider;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaseoModelEntry {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_default: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_options: Option<Vec<PaseoThinkingOption>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaseoThinkingOption {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_default: Option<bool>,
}

/// Claude Code role bindings in the same order the provider UI lists them:
/// Haiku, Sonnet, Opus, Fable, Subagent, Custom.
///
/// The third tuple item is the legacy top-level provider field. Subagent and
/// Custom arrived after the form moved to `env`, so they have no legacy name.
const CLAUDE_ROLE_FIELDS: [(&str, &str, &str); 6] = [
    (
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
        "haikuModel",
    ),
    (
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
        "sonnetModel",
    ),
    (
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
        "opusModel",
    ),
    (
        "ANTHROPIC_DEFAULT_FABLE_MODEL",
        "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
        "fableModel",
    ),
    ("CLAUDE_CODE_SUBAGENT_MODEL", "", ""),
    ("ANTHROPIC_CUSTOM_MODEL_OPTION", "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME", ""),
];

struct PendingModel {
    entry: PaseoModelEntry,
    label_from_display_name: bool,
}

pub fn build_models_for_provider(
    provider: PaseoProvider,
    settings_config: &str,
) -> Result<Vec<PaseoModelEntry>, String> {
    match provider {
        PaseoProvider::Claude => build_claude_models(settings_config),
        PaseoProvider::Codex => build_codex_models(settings_config),
    }
}

/// Build the Claude list. The Opus binding is marked as Paseo's default model.
pub fn build_claude_models(settings_config: &str) -> Result<Vec<PaseoModelEntry>, String> {
    let settings = parse_settings_config(settings_config)?;
    let mut pending: Vec<PendingModel> = Vec::new();

    for (env_key, display_name_key, legacy_key) in CLAUDE_ROLE_FIELDS {
        let id = read_env_or_legacy(&settings, env_key, legacy_key);
        if id.is_empty() {
            continue;
        }
        let display_name = if display_name_key.is_empty() {
            String::new()
        } else {
            read_env_string(&settings, display_name_key)
        };
        let label_from_display_name = !display_name.is_empty();
        let label = if label_from_display_name {
            display_name
        } else {
            strip_one_m_marker(&id)
        };
        push_or_merge(&mut pending, id, label, label_from_display_name);
    }

    let opus_id = read_env_or_legacy(&settings, "ANTHROPIC_DEFAULT_OPUS_MODEL", "opusModel");
    if !opus_id.is_empty() {
        if let Some(existing) = pending.iter_mut().find(|item| item.entry.id == opus_id) {
            existing.entry.is_default = Some(true);
        }
    }

    // The provider-wide default only stands in for the list when no role
    // binding exists. The provider UI treats it as a separate catch-all, and
    // the example config keeps it out of the role list.
    if pending.is_empty() {
        let fallback = read_env_or_legacy(&settings, "ANTHROPIC_MODEL", "model");
        if !fallback.is_empty() {
            pending.push(PendingModel {
                entry: PaseoModelEntry {
                    id: fallback.clone(),
                    label: strip_one_m_marker(&fallback),
                    description: None,
                    is_default: Some(true),
                    thinking_options: None,
                },
                label_from_display_name: false,
            });
        }
    }

    Ok(pending.into_iter().map(|item| item.entry).collect())
}

/// Build the Codex list from the provider's model catalog. The catalog's
/// per-model reasoning levels become Paseo `thinkingOptions`; the config.toml
/// top-level `model` is marked as the default.
pub fn build_codex_models(settings_config: &str) -> Result<Vec<PaseoModelEntry>, String> {
    let settings = parse_settings_config(settings_config)?;
    let default_model = settings
        .get("config")
        .and_then(Value::as_str)
        .and_then(read_toml_default_model)
        .unwrap_or_default();

    let catalog_items = settings
        .get("modelCatalog")
        .or_else(|| settings.get("model_catalog"))
        .and_then(|catalog| catalog.get("models"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut pending: Vec<PendingModel> = Vec::new();
    for item in catalog_items {
        let Some(item) = item.as_object() else {
            continue;
        };
        let id = item
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_default()
            .to_string();
        if id.is_empty() {
            continue;
        }

        let display_name = item
            .get("displayName")
            .or_else(|| item.get("display_name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_default()
            .to_string();
        let label_from_display_name = !display_name.is_empty();
        let label = if label_from_display_name {
            display_name
        } else {
            id.clone()
        };
        let thinking_options = build_codex_thinking_options(item);
        let is_default = (!default_model.is_empty() && id == default_model).then_some(true);

        if let Some(existing) = pending.iter_mut().find(|entry| entry.entry.id == id) {
            if !existing.label_from_display_name && label_from_display_name {
                existing.entry.label = label;
                existing.label_from_display_name = true;
            }
            if existing.entry.thinking_options.is_none() && thinking_options.is_some() {
                existing.entry.thinking_options = thinking_options;
            }
            if existing.entry.is_default.is_none() && is_default.is_some() {
                existing.entry.is_default = is_default;
            }
            continue;
        }

        pending.push(PendingModel {
            entry: PaseoModelEntry {
                id,
                label,
                description: None,
                is_default,
                thinking_options,
            },
            label_from_display_name,
        });
    }

    // A hand-written provider can point config.toml at a model that was not
    // added to the catalog. Keep the actual default selectable in Paseo.
    //
    // Only do this when the provider actually declared a catalog: an official
    // subscription has no `modelCatalog` and replacing Paseo's discovered
    // official list with a single item would be worse than leaving it alone.
    if !pending.is_empty()
        && !default_model.is_empty()
        && !pending.iter().any(|entry| entry.entry.id == default_model)
    {
        pending.push(PendingModel {
            entry: PaseoModelEntry {
                id: default_model.clone(),
                label: default_model,
                description: None,
                is_default: Some(true),
                thinking_options: None,
            },
            label_from_display_name: false,
        });
    }

    Ok(pending.into_iter().map(|item| item.entry).collect())
}

/// Strip a trailing `[1M]` marker for display while keeping the stored id
/// intact. Paseo understands the `[1m]` suffix for context-window inference and
/// Claude Code accepts it as part of the model id.
pub fn strip_one_m_marker(model: &str) -> String {
    let trimmed = model.trim_end();
    // `get` instead of direct slicing: a non-ASCII model id can end on a byte
    // that is not a UTF-8 char boundary, and `trimmed[len - 4..]` would panic.
    if trimmed.len() >= 4
        && trimmed
            .get(trimmed.len() - 4..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case("[1m]"))
    {
        trimmed[..trimmed.len() - 4].trim_end().to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_settings_config(settings_config: &str) -> Result<Value, String> {
    let trimmed = settings_config.trim();
    if trimmed.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let value: Value = serde_json::from_str(trimmed)
        .map_err(|error| format!("Invalid provider settings_config JSON: {error}"))?;
    if !value.is_object() {
        return Err("Provider settings_config is not a JSON object".to_string());
    }
    Ok(value)
}

fn read_string(settings: &Value, key: &str) -> String {
    settings
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn read_env_string(settings: &Value, key: &str) -> String {
    settings
        .get("env")
        .and_then(Value::as_object)
        .and_then(|env| env.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn read_env_or_legacy(settings: &Value, env_key: &str, legacy_key: &str) -> String {
    let env_value = read_env_string(settings, env_key);
    if !env_value.is_empty() {
        return env_value;
    }
    if legacy_key.is_empty() {
        String::new()
    } else {
        read_string(settings, legacy_key)
    }
}

fn push_or_merge(
    entries: &mut Vec<PendingModel>,
    id: String,
    label: String,
    label_from_display_name: bool,
) {
    if let Some(existing) = entries.iter_mut().find(|entry| entry.entry.id == id) {
        if !existing.label_from_display_name && label_from_display_name {
            existing.entry.label = label;
            existing.label_from_display_name = true;
        }
        return;
    }
    entries.push(PendingModel {
        entry: PaseoModelEntry {
            id,
            label,
            description: None,
            is_default: None,
            thinking_options: None,
        },
        label_from_display_name,
    });
}

fn build_codex_thinking_options(item: &Map<String, Value>) -> Option<Vec<PaseoThinkingOption>> {
    let levels = item
        .get("reasoningLevels")
        .or_else(|| item.get("reasoning_levels"))
        .and_then(Value::as_array)?;
    let default_level = item
        .get("defaultReasoningLevel")
        .or_else(|| item.get("default_reasoning_level"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let mut seen = HashSet::new();
    let mut options = Vec::new();
    for level in levels {
        let Some(level) = level
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if !seen.insert(level.to_string()) {
            continue;
        }
        options.push(PaseoThinkingOption {
            id: level.to_string(),
            label: level.to_string(),
            description: None,
            is_default: (default_level == Some(level)).then_some(true),
        });
    }

    if options.is_empty() {
        None
    } else {
        Some(options)
    }
}

fn read_toml_default_model(config_toml: &str) -> Option<String> {
    let document: toml::Value = config_toml.parse().ok()?;
    document
        .get("model")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_models_follow_role_order_and_use_opus_as_default() {
        let settings = json!({
            "env": {
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "claude-haiku-4-5",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME": "Claude Haiku 4.5",
                "ANTHROPIC_DEFAULT_SONNET_MODEL": "glm-5.3-flash[1M]",
                "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "GLM-5.3-Flash",
                "ANTHROPIC_DEFAULT_OPUS_MODEL": "deepseek-v4.1-flash[1M]",
                "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME": "DeepSeek-V4.1-Flash",
                "ANTHROPIC_DEFAULT_FABLE_MODEL": "glm-5.3[1M]",
                "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME": "GLM-5.3",
                "CLAUDE_CODE_SUBAGENT_MODEL": "deepseek-v4.1-flash[1M]",
                "ANTHROPIC_CUSTOM_MODEL_OPTION": "qwen3.8-flash[1M]",
                "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME": "Qwen3.8-Flash"
            }
        })
        .to_string();

        let models = build_claude_models(&settings).expect("build Claude models");

        assert_eq!(
            models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(),
            vec![
                "claude-haiku-4-5",
                "glm-5.3-flash[1M]",
                "deepseek-v4.1-flash[1M]",
                "glm-5.3[1M]",
                "qwen3.8-flash[1M]",
            ]
        );
        assert_eq!(models[0].label, "Claude Haiku 4.5");
        assert_eq!(models[2].label, "DeepSeek-V4.1-Flash");
        assert_eq!(models[2].is_default, Some(true));
        assert!(models.iter().filter(|model| model.is_default == Some(true)).count() == 1);
    }

    #[test]
    fn strip_one_m_marker_handles_non_ascii_model_ids() {
        // "测试" is 6 bytes; `len - 4 == 2` is not a char boundary. The old
        // byte-slice check panicked here before it even looked for `[1M]`.
        assert_eq!(strip_one_m_marker("测试"), "测试");
        assert_eq!(strip_one_m_marker("测试[1M]"), "测试");
        assert_eq!(strip_one_m_marker("模型[1m]"), "模型");
    }

    #[test]
    fn claude_labels_fall_back_to_the_model_id_without_the_one_m_marker() {
        let settings = json!({
            "env": {
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "glm-5.3[1M]",
                "ANTHROPIC_DEFAULT_OPUS_MODEL": "glm-5.3[1M]"
            }
        })
        .to_string();

        let models = build_claude_models(&settings).expect("build Claude models");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "glm-5.3[1M]");
        assert_eq!(models[0].label, "glm-5.3");
        assert_eq!(models[0].is_default, Some(true));
    }

    #[test]
    fn claude_uses_the_fallback_model_only_when_no_role_is_configured() {
        let settings = json!({
            "env": { "ANTHROPIC_MODEL": "opus[1m]" }
        })
        .to_string();

        let models = build_claude_models(&settings).expect("build Claude models");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "opus[1m]");
        assert_eq!(models[0].label, "opus");
        assert_eq!(models[0].is_default, Some(true));
    }

    #[test]
    fn claude_reads_legacy_top_level_model_fields() {
        let settings = json!({
            "model": "fallback-model",
            "haikuModel": "legacy-haiku",
            "opusModel": "legacy-opus"
        })
        .to_string();

        let models = build_claude_models(&settings).expect("build Claude models");

        assert_eq!(
            models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(),
            vec!["legacy-haiku", "legacy-opus"]
        );
        assert_eq!(models[1].is_default, Some(true));
    }

    #[test]
    fn codex_models_map_catalog_and_reasoning_levels() {
        let settings = json!({
            "config": "model = \"deepseek-v4.1-flash\"\nmodel_provider = \"custom\"\n",
            "modelCatalog": {
                "models": [
                    {
                        "model": "deepseek-v4.1-flash",
                        "displayName": "DeepSeek V4.1 Flash",
                        "reasoningLevels": ["low", "high", "max"],
                        "defaultReasoningLevel": "max"
                    },
                    {
                        "model": "glm-5.3-flash",
                        "displayName": "GLM 5.3 Flash"
                    }
                ]
            }
        })
        .to_string();

        let models = build_codex_models(&settings).expect("build Codex models");

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "deepseek-v4.1-flash");
        assert_eq!(models[0].label, "DeepSeek V4.1 Flash");
        assert_eq!(models[0].is_default, Some(true));
        let thinking_options = models[0].thinking_options.as_ref().expect("thinking options");
        assert_eq!(
            thinking_options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>(),
            vec!["low", "high", "max"]
        );
        assert_eq!(thinking_options[2].is_default, Some(true));
        assert_eq!(models[1].is_default, None);
    }

    #[test]
    fn codex_adds_the_config_default_when_the_catalog_omits_it() {
        let settings = json!({
            "config": "model = \"gpt-6-astra\"\n",
            "modelCatalog": {
                "models": [{ "model": "glm-5.3-flash", "displayName": "GLM 5.3 Flash" }]
            }
        })
        .to_string();

        let models = build_codex_models(&settings).expect("build Codex models");

        assert_eq!(
            models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(),
            vec!["glm-5.3-flash", "gpt-6-astra"]
        );
        assert_eq!(models[1].is_default, Some(true));
    }

    #[test]
    fn codex_without_a_catalog_does_not_replace_paseo_models() {
        let settings = json!({
            "config": "model = \"gpt-6-astra\"\n"
        })
        .to_string();

        let models = build_codex_models(&settings).expect("build Codex models");

        assert!(models.is_empty());
    }

    #[test]
    fn codex_uses_snake_case_catalog_aliases() {
        let settings = json!({
            "config": "model = \"mapped-a\"\n",
            "model_catalog": {
                "models": [
                    {
                        "model": "mapped-a",
                        "display_name": "Mapped A",
                        "reasoning_levels": ["low", "high"],
                        "default_reasoning_level": "high"
                    }
                ]
            }
        })
        .to_string();

        let models = build_codex_models(&settings).expect("build Codex models");

        assert_eq!(models[0].label, "Mapped A");
        assert_eq!(models[0].is_default, Some(true));
        assert_eq!(
            models[0]
                .thinking_options
                .as_ref()
                .expect("thinking options")
                .len(),
            2
        );
    }

    #[test]
    fn invalid_settings_config_is_reported() {
        assert!(build_claude_models("not-json").is_err());
        assert!(build_codex_models("[]").is_err());
    }
}
