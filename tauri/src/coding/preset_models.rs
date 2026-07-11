use crate::db::SqliteDbState;
use crate::http_client;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

const CACHE_FILE_NAME: &str = "preset_models.json";

/// Bundled preset models JSON (compile-time embedded from resources/)
const DEFAULT_PRESET_MODELS_JSON: &str = include_str!("../../resources/preset_models.json");

/// App data directory path, set once at startup by lib.rs
static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

// ============================================================================
// Cache directory management
// ============================================================================

/// Set the cache directory (called once from lib.rs at startup)
pub fn set_cache_dir(dir: PathBuf) {
    let _ = CACHE_DIR.set(dir);
}

fn get_cache_file_path() -> Option<PathBuf> {
    CACHE_DIR.get().map(|dir| dir.join(CACHE_FILE_NAME))
}

/// Public getter for the cache file path (used by backup/restore)
pub fn get_preset_models_cache_path() -> Option<PathBuf> {
    get_cache_file_path()
}

// ============================================================================
// Bundled defaults
// ============================================================================

fn get_bundled_preset_models() -> Option<Value> {
    let data: Value = serde_json::from_str(DEFAULT_PRESET_MODELS_JSON).ok()?;
    if is_valid_preset_models(&data) {
        Some(data)
    } else {
        None
    }
}

// ============================================================================
// File-based cache read / write
// ============================================================================

fn read_cache_file() -> Option<Value> {
    let path = get_cache_file_path()?;
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Atomic write: write to .tmp then rename
fn write_cache_file(data: &Value) -> Result<(), String> {
    let path =
        get_cache_file_path().ok_or_else(|| "Cache directory not initialized".to_string())?;

    let tmp_path = path.with_extension("json.tmp");

    let json = serde_json::to_string(data)
        .map_err(|e| format!("Failed to serialize preset models cache: {}", e))?;

    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create cache directory: {}", e))?;
        }
    }

    fs::write(&tmp_path, json).map_err(|e| format!("Failed to write tmp cache file: {}", e))?;
    fs::rename(&tmp_path, &path).map_err(|e| format!("Failed to rename tmp cache file: {}", e))?;

    Ok(())
}

/// Validate that the JSON looks like a preset models map
/// (non-empty object with at least one key).
fn is_valid_preset_models(data: &Value) -> bool {
    data.as_object().map(|m| !m.is_empty()).unwrap_or(false)
}

// ============================================================================
// Tauri commands
// ============================================================================

/// Load preset models: local cache first, then bundled defaults as fallback.
#[tauri::command]
pub fn load_cached_preset_models() -> Result<Option<Value>, String> {
    // Try local cache first
    if let Some(data) = read_cache_file() {
        if is_valid_preset_models(&data) {
            return Ok(Some(data));
        }
    }
    // Fallback to bundled defaults
    Ok(get_bundled_preset_models())
}

/// Fetch preset models JSON from a remote URL, save to local cache,
/// and return the data to the frontend.
#[tauri::command]
pub async fn fetch_remote_preset_models(
    state: tauri::State<'_, SqliteDbState>,
    url: String,
) -> Result<Value, String> {
    let client = http_client::client_with_timeout(&state, 30).await?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch remote preset models: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Remote preset models request failed: {}",
            response.status()
        ));
    }

    let json: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse remote preset models JSON: {}", e))?;

    // Only cache valid data
    if !is_valid_preset_models(&json) {
        return Err("Remote preset models JSON is empty or invalid".to_string());
    }

    // Save to local cache file
    if let Err(e) = write_cache_file(&json) {
        log::warn!("[PresetModels] Failed to write cache: {}", e);
    } else {
        log::info!("[PresetModels] Cache updated from remote");
    }

    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_PRESET_MODELS_JSON;
    use serde_json::Value;

    const ADAPTIVE_EFFORT_LEVELS: [&str; 4] = ["low", "medium", "high", "max"];
    const EXTENDED_ADAPTIVE_EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
    const LEGACY_THINKING_LEVELS: [(&str, u64); 3] =
        [("low", 5_000), ("medium", 13_000), ("high", 18_000)];

    fn bundled_anthropic_models() -> Value {
        let presets: Value = serde_json::from_str(DEFAULT_PRESET_MODELS_JSON)
            .expect("bundled preset models JSON should parse");
        presets
            .get("@ai-sdk/anthropic")
            .cloned()
            .expect("Anthropic preset group should exist")
    }

    fn bundled_openai_models() -> Value {
        let presets: Value = serde_json::from_str(DEFAULT_PRESET_MODELS_JSON)
            .expect("bundled preset models JSON should parse");
        presets
            .get("@ai-sdk/openai")
            .cloned()
            .expect("OpenAI preset group should exist")
    }

    fn bundled_xai_models() -> Value {
        let presets: Value = serde_json::from_str(DEFAULT_PRESET_MODELS_JSON)
            .expect("bundled preset models JSON should parse");
        presets
            .get("@ai-sdk/xai")
            .cloned()
            .expect("xAI preset group should exist")
    }

    fn model<'a>(models: &'a Value, model_id: &str) -> &'a Value {
        models
            .as_array()
            .expect("Anthropic preset group should be an array")
            .iter()
            .find(|model| model.get("id").and_then(Value::as_str) == Some(model_id))
            .unwrap_or_else(|| panic!("Anthropic preset model {model_id} should exist"))
    }

    fn assert_adaptive_variants(
        models: &Value,
        model_id: &str,
        effort_levels: &[&str],
        summarized: bool,
    ) {
        let variants = model(models, model_id)
            .get("variants")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("{model_id} should define variants"));

        assert_eq!(
            variants.len(),
            effort_levels.len(),
            "{model_id} should expose exactly the supported effort levels"
        );

        for effort_level in effort_levels {
            let variant = variants
                .get(*effort_level)
                .unwrap_or_else(|| panic!("{model_id} should define the {effort_level} variant"));
            assert_eq!(
                variant.get("effort").and_then(Value::as_str),
                Some(*effort_level)
            );

            let thinking = variant
                .get("thinking")
                .and_then(Value::as_object)
                .unwrap_or_else(|| {
                    panic!("{model_id}/{effort_level} should enable adaptive thinking")
                });
            assert_eq!(
                thinking.get("type").and_then(Value::as_str),
                Some("adaptive")
            );
            assert!(
                thinking.get("budgetTokens").is_none(),
                "{model_id}/{effort_level} must not retain a fixed thinking budget"
            );
            assert!(
                thinking.get("effort").is_none(),
                "{model_id}/{effort_level} effort must be a sibling of thinking"
            );

            if summarized {
                assert_eq!(
                    thinking.get("display").and_then(Value::as_str),
                    Some("summarized"),
                    "{model_id}/{effort_level} should request visible thinking summaries"
                );
            } else {
                assert!(thinking.get("display").is_none());
            }
        }
    }

    fn assert_legacy_thinking_variants(models: &Value, model_id: &str) {
        let variants = model(models, model_id)
            .get("variants")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("{model_id} should define variants"));

        assert_eq!(variants.len(), LEGACY_THINKING_LEVELS.len());
        for (variant_name, budget_tokens) in LEGACY_THINKING_LEVELS {
            let variant = variants
                .get(variant_name)
                .unwrap_or_else(|| panic!("{model_id} should define the {variant_name} variant"));
            assert!(
                variant.get("effort").is_none(),
                "{model_id}/{variant_name} should use a fixed thinking budget, not effort"
            );
            assert_eq!(
                variant.pointer("/thinking/type").and_then(Value::as_str),
                Some("enabled")
            );
            assert_eq!(
                variant
                    .pointer("/thinking/budgetTokens")
                    .and_then(Value::as_u64),
                Some(budget_tokens)
            );
        }
    }

    #[test]
    fn openai_presets_define_gpt_5_6_family_with_max_reasoning() {
        const GPT_5_6_MODEL_IDS: [&str; 3] = ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"];
        const GPT_5_6_REASONING_LEVELS: [&str; 6] =
            ["none", "low", "medium", "high", "xhigh", "max"];

        let models = bundled_openai_models();
        let model_list = models
            .as_array()
            .expect("OpenAI preset group should be an array");
        let leading_model_ids: Vec<&str> = model_list
            .iter()
            .take(GPT_5_6_MODEL_IDS.len())
            .filter_map(|preset| preset.get("id").and_then(Value::as_str))
            .collect();

        assert_eq!(leading_model_ids, GPT_5_6_MODEL_IDS);
        assert!(
            model_list
                .iter()
                .all(|preset| preset.get("id").and_then(Value::as_str) != Some("gpt-5.6")),
            "the gpt-5.6 alias should not duplicate the canonical Sol preset"
        );

        for model_id in GPT_5_6_MODEL_IDS {
            let preset = model_list
                .iter()
                .find(|preset| preset.get("id").and_then(Value::as_str) == Some(model_id))
                .unwrap_or_else(|| panic!("OpenAI preset model {model_id} should exist"));

            assert_eq!(
                preset.get("contextLimit").and_then(Value::as_u64),
                Some(1_050_000)
            );
            assert_eq!(
                preset.get("outputLimit").and_then(Value::as_u64),
                Some(128_000)
            );
            assert_eq!(preset.get("reasoning").and_then(Value::as_bool), Some(true));
            assert_eq!(preset.get("tool_call").and_then(Value::as_bool), Some(true));
            assert_eq!(
                preset.get("temperature").and_then(Value::as_bool),
                Some(false)
            );
            assert_eq!(
                preset.get("attachment").and_then(Value::as_bool),
                Some(true)
            );
            assert_eq!(
                preset.pointer("/options/store").and_then(Value::as_bool),
                Some(false)
            );
            assert_eq!(
                preset.pointer("/modalities/input"),
                Some(&serde_json::json!(["text", "image"]))
            );
            assert_eq!(
                preset.pointer("/modalities/output"),
                Some(&serde_json::json!(["text"]))
            );

            let variants = preset
                .get("variants")
                .and_then(Value::as_object)
                .unwrap_or_else(|| panic!("{model_id} should define reasoning variants"));
            assert_eq!(variants.len(), GPT_5_6_REASONING_LEVELS.len());
            for reasoning_level in GPT_5_6_REASONING_LEVELS {
                let variant = variants.get(reasoning_level).unwrap_or_else(|| {
                    panic!("{model_id} should define the {reasoning_level} variant")
                });
                assert_eq!(
                    variant.get("reasoningEffort").and_then(Value::as_str),
                    Some(reasoning_level)
                );
                assert_eq!(
                    variant.get("reasoningSummary").and_then(Value::as_str),
                    Some("auto")
                );
                assert_eq!(
                    variant.get("textVerbosity").and_then(Value::as_str),
                    Some("medium")
                );
            }
        }
    }

    #[test]
    fn xai_presets_define_only_canonical_grok_4_5_with_supported_reasoning() {
        const GROK_4_5_REASONING_LEVELS: [&str; 3] = ["low", "medium", "high"];

        let models = bundled_xai_models();
        let model_list = models
            .as_array()
            .expect("xAI preset group should be an array");
        assert_eq!(model_list.len(), 1);

        let preset = model_list.first().expect("Grok 4.5 preset should exist");
        assert_eq!(preset.get("id").and_then(Value::as_str), Some("grok-4.5"));
        assert_eq!(
            preset.get("contextLimit").and_then(Value::as_u64),
            Some(500_000)
        );
        assert!(preset.get("outputLimit").is_none());
        assert_eq!(preset.get("reasoning").and_then(Value::as_bool), Some(true));
        assert_eq!(preset.get("tool_call").and_then(Value::as_bool), Some(true));
        assert_eq!(
            preset.get("attachment").and_then(Value::as_bool),
            Some(true)
        );
        assert!(preset.get("temperature").is_none());
        assert_eq!(
            preset.pointer("/modalities/input"),
            Some(&serde_json::json!(["text", "image"]))
        );
        assert_eq!(
            preset.pointer("/modalities/output"),
            Some(&serde_json::json!(["text"]))
        );

        let variants = preset
            .get("variants")
            .and_then(Value::as_object)
            .expect("Grok 4.5 should define reasoning variants");
        assert_eq!(variants.len(), GROK_4_5_REASONING_LEVELS.len());
        for reasoning_level in GROK_4_5_REASONING_LEVELS {
            let variant = variants
                .get(reasoning_level)
                .unwrap_or_else(|| panic!("Grok 4.5 should define {reasoning_level}"));
            assert_eq!(
                variant.get("reasoningEffort").and_then(Value::as_str),
                Some(reasoning_level)
            );
        }

        for alias in ["grok-4.5-latest", "grok-build-latest"] {
            assert!(
                model_list
                    .iter()
                    .all(|model| model.get("id").and_then(Value::as_str) != Some(alias)),
                "{alias} should not duplicate the canonical Grok 4.5 preset"
            );
        }
    }

    #[test]
    fn anthropic_presets_use_adaptive_thinking_for_claude_4_6_and_later() {
        let models = bundled_anthropic_models();

        for model_id in ["claude-opus-4-6", "claude-sonnet-4-6"] {
            assert_adaptive_variants(&models, model_id, &ADAPTIVE_EFFORT_LEVELS, false);
            assert_eq!(
                model(&models, model_id)
                    .get("contextLimit")
                    .and_then(Value::as_u64),
                Some(1_000_000)
            );
            assert_eq!(
                model(&models, model_id)
                    .get("outputLimit")
                    .and_then(Value::as_u64),
                Some(128_000)
            );
        }

        for model_id in [
            "claude-fable-5",
            "claude-sonnet-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
        ] {
            assert_adaptive_variants(&models, model_id, &EXTENDED_ADAPTIVE_EFFORT_LEVELS, true);
        }
    }

    #[test]
    fn anthropic_presets_keep_legacy_thinking_budgets_on_older_models() {
        let models = bundled_anthropic_models();

        for model_id in [
            "claude-sonnet-4-5-20250929",
            "claude-haiku-4-5-20251001",
            "claude-opus-4-1",
            "claude-sonnet-4-0",
            "claude-3-7-sonnet-latest",
        ] {
            assert_legacy_thinking_variants(&models, model_id);
        }
    }

    #[test]
    fn anthropic_opus_4_5_uses_effort_without_adaptive_thinking() {
        let models = bundled_anthropic_models();
        let variants = model(&models, "claude-opus-4-5-20251101")
            .get("variants")
            .and_then(Value::as_object)
            .expect("Claude Opus 4.5 should define variants");

        assert_eq!(variants.len(), 3);
        for effort_level in ["low", "medium", "high"] {
            let variant = variants
                .get(effort_level)
                .unwrap_or_else(|| panic!("Claude Opus 4.5 should define {effort_level}"));
            assert_eq!(
                variant.get("effort").and_then(Value::as_str),
                Some(effort_level)
            );
            assert!(variant.get("thinking").is_none());
        }
    }
}
