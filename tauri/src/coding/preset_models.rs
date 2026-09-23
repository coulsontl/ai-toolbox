use crate::db::SqliteDbState;
use crate::http_client;
use serde_json::Value;
use std::collections::HashMap;
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
// Display-name lookup
// ============================================================================

/// Model id -> display name index built once from the bundled preset models.
///
/// Built from the compile-time bundled file instead of the app-data cache so
/// callers stay deterministic and work offline; the cache is the frontend's
/// remote-refresh target, not a backend read path.
static PRESET_DISPLAY_NAMES: OnceLock<HashMap<String, String>> = OnceLock::new();

/// Display name the preset models declare for one model id.
///
/// Returns `None` for an unknown or blank id so callers keep their own fallback
/// (normally the raw id).
pub fn display_name_for_model_id(model_id: &str) -> Option<String> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return None;
    }
    PRESET_DISPLAY_NAMES
        .get_or_init(build_preset_display_name_index)
        .get(model_id)
        .cloned()
}

fn build_preset_display_name_index() -> HashMap<String, String> {
    let mut index = HashMap::new();
    let Ok(presets) = serde_json::from_str::<Value>(DEFAULT_PRESET_MODELS_JSON) else {
        return index;
    };
    let Some(groups) = presets.as_object() else {
        return index;
    };
    for models in groups.values() {
        let Some(models) = models.as_array() else {
            continue;
        };
        for model in models {
            let id = model
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty());
            let name = model
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty());
            if let (Some(id), Some(name)) = (id, name) {
                // First group wins if one id ever appears twice; the bundled file
                // has no conflicting duplicates today.
                index
                    .entry(id.to_string())
                    .or_insert_with(|| name.to_string());
            }
        }
    }
    index
}

// ============================================================================
// Input-modality lookup
// ============================================================================

/// Model id -> declared `modalities.input` index built once from the bundled
/// preset models.
///
/// Same read path as `PRESET_DISPLAY_NAMES`: compile-time bundled file only,
/// so callers stay deterministic and work offline; the app-data cache is the
/// frontend's remote-refresh target, not a backend read path.
static PRESET_INPUT_MODALITIES: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();

/// Input modalities the preset models declare for one model id.
///
/// Returns `None` for an unknown id or an entry without a usable
/// `modalities.input` array, so callers keep their own fallback (the entry for
/// `gpt-5.4-nano`, which ships no modalities at all, lands here too).
pub fn input_modalities_for_model_id(model_id: &str) -> Option<Vec<String>> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return None;
    }
    PRESET_INPUT_MODALITIES
        .get_or_init(build_preset_input_modalities_index)
        .get(model_id)
        .cloned()
}

fn build_preset_input_modalities_index() -> HashMap<String, Vec<String>> {
    let mut index = HashMap::new();
    let Ok(presets) = serde_json::from_str::<Value>(DEFAULT_PRESET_MODELS_JSON) else {
        return index;
    };
    let Some(groups) = presets.as_object() else {
        return index;
    };
    for models in groups.values() {
        let Some(models) = models.as_array() else {
            continue;
        };
        for model in models {
            let Some(id) = model
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let Some(modalities) = model
                .pointer("/modalities/input")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::trim))
                        .filter(|item| !item.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .filter(|items| !items.is_empty())
            else {
                continue;
            };
            // First group wins if one id ever appears twice; the bundled file
            // has no conflicting duplicates today.
            index.entry(id.to_string()).or_insert(modalities);
        }
    }
    index
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
    use super::display_name_for_model_id;
    use super::input_modalities_for_model_id;
    use super::DEFAULT_PRESET_MODELS_JSON;
    use serde_json::Value;

    const ADAPTIVE_EFFORT_LEVELS: [&str; 4] = ["low", "medium", "high", "max"];
    const EXTENDED_ADAPTIVE_EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
    const LEGACY_THINKING_LEVELS: [(&str, u64); 3] =
        [("low", 5_000), ("medium", 13_000), ("high", 18_000)];

    #[test]
    fn display_name_lookup_covers_bundled_ids_and_rejects_unknown_ones() {
        assert_eq!(
            display_name_for_model_id("gpt-6-astra").as_deref(),
            Some("GPT-6 Astra")
        );
        // Ids are trimmed before lookup.
        assert_eq!(
            display_name_for_model_id(" gpt-5.6-sol ").as_deref(),
            Some("GPT-5.6 Sol")
        );
        // Unknown or blank ids stay unknown so callers keep their own fallback.
        assert_eq!(display_name_for_model_id("no-such-model"), None);
        assert_eq!(display_name_for_model_id("   "), None);

        // Every bundled group's `id`/`name` pairs stay reachable through the
        // index, so a new preset does not silently drop out of the lookup.
        let presets: Value =
            serde_json::from_str(DEFAULT_PRESET_MODELS_JSON).expect("bundled JSON should parse");
        for models in presets.as_object().expect("preset groups").values() {
            for model in models.as_array().expect("group is an array") {
                let (Some(id), Some(name)) = (
                    model.get("id").and_then(Value::as_str),
                    model.get("name").and_then(Value::as_str),
                ) else {
                    continue;
                };
                assert_eq!(display_name_for_model_id(id).as_deref(), Some(name), "{id}");
            }
        }
    }

    #[test]
    fn input_modalities_lookup_covers_bundled_declarations_and_rejects_unknown_ones() {
        // A text-only preset stays text-only (the Codex catalog generator uses
        // this as its confirmed-text-only registry).
        assert_eq!(
            input_modalities_for_model_id("deepseek-chat"),
            Some(vec!["text".to_string()])
        );
        // An image-capable preset keeps its full declared set.
        assert_eq!(
            input_modalities_for_model_id("deepseek-v4.1-flash"),
            Some(vec!["text".to_string(), "image".to_string()])
        );
        // Non-text modalities survive verbatim (Gemini presets declare audio).
        let gemini = input_modalities_for_model_id("gemini-2.5-flash")
            .expect("gemini-2.5-flash declares modalities");
        assert!(gemini.contains(&"audio".to_string()), "{gemini:?}");

        // Unknown or blank ids stay unknown so callers keep their own fallback.
        assert_eq!(input_modalities_for_model_id("no-such-model"), None);
        assert_eq!(input_modalities_for_model_id("   "), None);

        // Every bundled group's id with a `modalities.input` array stays
        // reachable through the index; entries without one (gpt-5.4-nano) must
        // stay unknown rather than resolving to an empty set.
        let presets: Value =
            serde_json::from_str(DEFAULT_PRESET_MODELS_JSON).expect("bundled JSON should parse");
        for models in presets.as_object().expect("preset groups").values() {
            for model in models.as_array().expect("group is an array") {
                let Some(id) = model.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let declared: Option<Vec<String>> = model
                    .pointer("/modalities/input")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_string))
                            .collect()
                    })
                    .filter(|items: &Vec<String>| !items.is_empty());
                assert_eq!(
                    input_modalities_for_model_id(id),
                    declared,
                    "{id} modality lookup must mirror the bundled declaration"
                );
            }
        }
    }

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

    #[test]
    fn preset_model_limits_are_always_complete_pairs() {
        let presets: Value = serde_json::from_str(DEFAULT_PRESET_MODELS_JSON)
            .expect("bundled preset models JSON should parse");

        for (provider, models) in presets
            .as_object()
            .expect("preset model root should be an object")
        {
            for preset in models
                .as_array()
                .unwrap_or_else(|| panic!("preset group {provider} should be an array"))
            {
                let model_id = preset
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>");
                assert_eq!(
                    preset.get("contextLimit").is_some(),
                    preset.get("outputLimit").is_some(),
                    "preset {provider}/{model_id} must define contextLimit and outputLimit together"
                );
            }
        }
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
        // gpt-6-astra still leads the bundled OpenAI preset list. GPT-6 Sol and
        // Luna sit immediately after it and share the same capability set.
        const GPT_5_6_FAMILY_MODEL_IDS: [&str; 6] = [
            "gpt-6-astra",
            "gpt-6-sol",
            "gpt-6-luna",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
        ];
        const LEADING_MODEL_IDS: [&str; 3] = ["gpt-6-astra", "gpt-6-sol", "gpt-6-luna"];
        const GPT_5_6_REASONING_LEVELS: [&str; 6] =
            ["none", "low", "medium", "high", "xhigh", "max"];

        let models = bundled_openai_models();
        let model_list = models
            .as_array()
            .expect("OpenAI preset group should be an array");
        let leading_model_ids: Vec<&str> = model_list
            .iter()
            .take(LEADING_MODEL_IDS.len())
            .filter_map(|preset| preset.get("id").and_then(Value::as_str))
            .collect();

        assert_eq!(leading_model_ids, LEADING_MODEL_IDS);
        assert!(
            model_list
                .iter()
                .all(|preset| preset.get("id").and_then(Value::as_str) != Some("gpt-5.6")),
            "the gpt-5.6 alias should not duplicate the canonical Sol preset"
        );

        for model_id in GPT_5_6_FAMILY_MODEL_IDS {
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
    fn xai_presets_define_canonical_grok_4_6_and_4_5_with_supported_reasoning() {
        const GROK_4_5_REASONING_LEVELS: [&str; 3] = ["low", "medium", "high"];
        const GROK_4_6_REASONING_LEVELS: [&str; 4] = ["low", "medium", "high", "xhigh"];

        let models = bundled_xai_models();
        let model_list = models
            .as_array()
            .expect("xAI preset group should be an array");
        let model_ids: Vec<&str> = model_list
            .iter()
            .filter_map(|preset| preset.get("id").and_then(Value::as_str))
            .collect();
        assert_eq!(model_ids, ["grok-4.7", "grok-4.6", "grok-4.5"]);

        let assert_shared_grok_fields = |model_id: &str, reasoning_levels: &[&str]| {
            let preset = model_list
                .iter()
                .find(|preset| preset.get("id").and_then(Value::as_str) == Some(model_id))
                .unwrap_or_else(|| panic!("{model_id} preset should exist"));
            assert_eq!(
                preset.get("contextLimit").and_then(Value::as_u64),
                Some(500_000)
            );
            assert_eq!(
                preset.get("outputLimit").and_then(Value::as_u64),
                Some(500_000)
            );
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
                .unwrap_or_else(|| panic!("{model_id} should define reasoning variants"));
            assert_eq!(variants.len(), reasoning_levels.len());
            for reasoning_level in reasoning_levels {
                let variant = variants.get(*reasoning_level).unwrap_or_else(|| {
                    panic!("{model_id} should define the {reasoning_level} variant")
                });
                assert_eq!(
                    variant.get("reasoningEffort").and_then(Value::as_str),
                    Some(*reasoning_level)
                );
            }
        };

        assert_shared_grok_fields("grok-4.7", &GROK_4_6_REASONING_LEVELS);
        assert_shared_grok_fields("grok-4.6", &GROK_4_6_REASONING_LEVELS);
        assert_shared_grok_fields("grok-4.5", &GROK_4_5_REASONING_LEVELS);

        for alias in [
            "grok-4.7-latest",
            "grok-4.6-latest",
            "grok-4.5-latest",
            "grok-build-latest",
        ] {
            assert!(
                model_list
                    .iter()
                    .all(|model| model.get("id").and_then(Value::as_str) != Some(alias)),
                "{alias} should not duplicate a canonical Grok preset"
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
