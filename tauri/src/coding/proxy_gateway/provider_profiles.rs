use crate::coding::proxy_gateway::transformer::AiProtocol;
use crate::db::SqliteDbState;
use crate::http_client;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

const CACHE_FILE_NAME: &str = "gateway_provider_profiles.json";
const DEFAULT_GATEWAY_PROVIDER_PROFILES_JSON: &str =
    include_str!("../../../resources/gateway_provider_profiles.json");
const SUPPORTED_PROFILE_TOOLS: [&str; 4] = ["claude", "codex", "grok", "gemini"];
struct CompatRuleRegistration {
    name: &'static str,
    runtime_owner: &'static str,
    test_name: &'static str,
}

impl CompatRuleRegistration {
    fn has_static_evidence(&self) -> bool {
        !self.runtime_owner.trim().is_empty() && !self.test_name.trim().is_empty()
    }
}

const SUPPORTED_COMPAT_RULES: [CompatRuleRegistration; 25] = [
    CompatRuleRegistration {
        name: "anthropic_tool_thinking_history",
        runtime_owner: "runtime::upstream::normalize_anthropic_tool_thinking_history",
        test_name:
            "provider_body_compat_anthropic_reasoning_vendor_normalizes_tool_thinking_history",
    },
    CompatRuleRegistration {
        name: "bailian_tool_call_merge",
        runtime_owner: "runtime::upstream::merge_consecutive_tool_call_messages",
        test_name: "provider_body_compat_bailian_chat_merges_consecutive_tool_call_messages",
    },
    CompatRuleRegistration {
        name: "bailian_tool_call_stream_filter",
        runtime_owner: "runtime::upstream::filter_bailian_openai_chat_sse_stream",
        test_name: "bailian_stream_filter_buffers_text_after_tool_calls_until_finish",
    },
    CompatRuleRegistration {
        name: "codex_chat_reasoning_enable_thinking",
        runtime_owner: "runtime::upstream::apply_codex_chat_reasoning_config",
        test_name: "provider_compat_siliconflow_uses_enable_thinking_without_reasoning_effort",
    },
    CompatRuleRegistration {
        name: "codex_chat_reasoning_low_high_effort",
        runtime_owner: "runtime::upstream::map_codex_chat_reasoning_effort",
        test_name: "provider_compat_stepfun_uses_per_model_effort_tiers",
    },
    CompatRuleRegistration {
        name: "codex_chat_reasoning_split",
        runtime_owner: "runtime::upstream::apply_codex_chat_reasoning_config",
        test_name: "provider_compat_minimax_uses_reasoning_split_and_reasoning_details_output",
    },
    CompatRuleRegistration {
        name: "copilot_dynamic_route",
        runtime_owner: "runtime::upstream::effective_upstream_provider_for_request",
        test_name: "copilot_effective_provider_switches_chat_and_responses_by_model",
    },
    CompatRuleRegistration {
        name: "copilot_headers",
        runtime_owner: "runtime::upstream::inject_copilot_headers",
        test_name: "copilot_headers_override_forwarded_fingerprint_and_infer_agent_turn",
    },
    CompatRuleRegistration {
        name: "copilot_token_exchange",
        runtime_owner: "runtime::upstream::resolve_copilot_token_for_provider",
        test_name: "copilot_token_exchange_sends_github_token_and_caches_response",
    },
    CompatRuleRegistration {
        name: "deepseek_disabled_strip_effort",
        runtime_owner: "runtime::upstream::apply_anthropic_provider_body_compat",
        test_name: "provider_body_compat_deepseek_anthropic_disabled_thinking_strips_effort_fields",
    },
    CompatRuleRegistration {
        name: "deepseek_json_schema",
        runtime_owner: "runtime::upstream::convert_response_format_json_schema_to_json_object",
        test_name:
            "provider_body_compat_deepseek_chat_rewrites_json_schema_thinking_and_custom_tools",
    },
    CompatRuleRegistration {
        name: "deepseek_thinking",
        runtime_owner: "runtime::upstream::apply_deepseek_openai_chat_thinking_compat",
        test_name: "codex_chat_reasoning_config_maps_deepseek_effort_and_thinking",
    },
    CompatRuleRegistration {
        name: "doubao_metadata",
        runtime_owner: "runtime::upstream::extract_metadata_to_vendor_ids",
        test_name: "provider_body_compat_doubao_chat_extracts_metadata_and_generates_request_id",
    },
    CompatRuleRegistration {
        name: "longcat_message_content_array",
        runtime_owner: "runtime::upstream::normalize_longcat_message_content_arrays",
        test_name: "provider_body_compat_longcat_chat_forces_message_content_arrays",
    },
    CompatRuleRegistration {
        name: "modelscope_remove_metadata",
        runtime_owner: "runtime::upstream::apply_provider_body_compat_before_generic",
        test_name: "provider_compat_modelscope_removes_metadata_for_chat_and_responses",
    },
    CompatRuleRegistration {
        name: "moonshot_json_schema",
        runtime_owner: "runtime::upstream::apply_openai_chat_provider_body_compat_before_generic",
        test_name: "provider_compat_moonshot_rewrites_schema_and_backfills_tool_reasoning",
    },
    CompatRuleRegistration {
        name: "ollama_api_chat_wire_adapter",
        runtime_owner: "runtime::upstream::convert_openai_chat_request_to_ollama_chat",
        test_name: "ollama_body_compat_converts_openai_chat_request_shape",
    },
    CompatRuleRegistration {
        name: "openrouter_reasoning_object",
        runtime_owner: "runtime::upstream::apply_openrouter_openai_chat_reasoning_effort",
        test_name: "provider_body_compat_openrouter_moves_reasoning_effort_to_reasoning_object",
    },
    CompatRuleRegistration {
        name: "reasoning_field_reasoning",
        runtime_owner: "runtime::upstream::apply_openai_chat_reasoning_field_policy",
        test_name: "provider_body_compat_openai_chat_applies_reasoning_field_policy",
    },
    CompatRuleRegistration {
        name: "xai_filter_empty_delta",
        runtime_owner: "runtime::upstream::filter_xai_openai_chat_sse_stream",
        test_name: "xai_stream_filter_drops_empty_delta_chunks",
    },
    CompatRuleRegistration {
        name: "xai_responses_passthrough",
        runtime_owner: "runtime::compat::xai_responses::apply_xai_responses_passthrough",
        test_name: "xai_responses_passthrough_scrubs_native_responses_body",
    },
    CompatRuleRegistration {
        name: "xai_strip_unsupported_fields",
        runtime_owner: "runtime::upstream::strip_xai_unsupported_openai_chat_fields",
        test_name: "provider_body_compat_xai_chat_strips_model_specific_unsupported_fields",
    },
    CompatRuleRegistration {
        name: "zai_metadata",
        runtime_owner: "runtime::upstream::extract_metadata_to_vendor_ids",
        test_name: "provider_body_compat_zai_chat_moves_metadata_and_forces_auto_tool_choice",
    },
    CompatRuleRegistration {
        name: "zai_thinking",
        runtime_owner: "runtime::upstream::apply_zai_openai_chat_thinking_compat",
        test_name: "provider_body_compat_zai_chat_moves_metadata_and_forces_auto_tool_choice",
    },
    CompatRuleRegistration {
        name: "zai_tool_choice",
        runtime_owner: "runtime::upstream::force_tool_choice_auto",
        test_name: "provider_body_compat_zai_chat_moves_metadata_and_forces_auto_tool_choice",
    },
];

static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
static ACTIVE_GATEWAY_PROVIDER_PROFILES: OnceLock<RwLock<Option<Value>>> = OnceLock::new();

pub fn set_cache_dir(dir: PathBuf) {
    let _ = CACHE_DIR.set(dir);
}

fn active_gateway_provider_profiles() -> &'static RwLock<Option<Value>> {
    ACTIVE_GATEWAY_PROVIDER_PROFILES.get_or_init(|| RwLock::new(None))
}

fn set_active_gateway_provider_profiles(data: Value) {
    let mut active = active_gateway_provider_profiles()
        .write()
        .unwrap_or_else(|error| error.into_inner());
    *active = Some(data);
}

fn get_active_gateway_provider_profiles() -> Option<Value> {
    let active = active_gateway_provider_profiles()
        .read()
        .unwrap_or_else(|error| error.into_inner());
    active.clone()
}

fn get_cache_file_path() -> Option<PathBuf> {
    CACHE_DIR.get().map(|dir| dir.join(CACHE_FILE_NAME))
}

pub fn get_gateway_provider_profiles_cache_path() -> Option<PathBuf> {
    get_cache_file_path()
}

pub(crate) fn get_bundled_gateway_provider_profiles() -> Option<Value> {
    let data: Value = serde_json::from_str(DEFAULT_GATEWAY_PROVIDER_PROFILES_JSON).ok()?;
    if is_valid_gateway_provider_profiles(&data) {
        Some(data)
    } else {
        None
    }
}

fn read_cache_file() -> Option<Value> {
    let path = get_cache_file_path()?;
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn write_cache_file(data: &Value) -> Result<(), String> {
    let path =
        get_cache_file_path().ok_or_else(|| "Cache directory not initialized".to_string())?;
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string(data)
        .map_err(|error| format!("Failed to serialize provider profiles cache: {error}"))?;

    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("Failed to create provider profiles cache dir: {error}")
            })?;
        }
    }

    fs::write(&tmp_path, json)
        .map_err(|error| format!("Failed to write provider profiles cache tmp file: {error}"))?;
    fs::rename(&tmp_path, &path)
        .map_err(|error| format!("Failed to replace provider profiles cache file: {error}"))?;
    Ok(())
}

fn text_field_is_empty(object: &serde_json::Map<String, Value>, key: &str) -> bool {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .is_none_or(str::is_empty)
}

fn endpoint_has_valid_api_format(endpoint_object: &serde_json::Map<String, Value>) -> bool {
    endpoint_object
        .get("apiFormat")
        .and_then(Value::as_str)
        .and_then(AiProtocol::from_api_format)
        .is_some()
}

fn tool_has_valid_endpoints(tool_object: &serde_json::Map<String, Value>) -> bool {
    let Some(default_endpoint_id) = tool_object
        .get("defaultEndpointId")
        .and_then(Value::as_str)
        .map(str::trim)
    else {
        return false;
    };
    if default_endpoint_id.is_empty() {
        return false;
    }

    let Some(endpoints) = tool_object.get("endpoints").and_then(Value::as_array) else {
        return false;
    };
    if endpoints.is_empty() {
        return false;
    }

    let mut endpoint_ids = HashSet::new();
    for endpoint in endpoints {
        let Some(endpoint_object) = endpoint.as_object() else {
            return false;
        };
        let Some(endpoint_id) = endpoint_object
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            return false;
        };
        if endpoint_id.is_empty() || !endpoint_ids.insert(endpoint_id.to_string()) {
            return false;
        }

        if text_field_is_empty(endpoint_object, "label")
            || text_field_is_empty(endpoint_object, "baseUrl")
            || !endpoint_has_valid_api_format(endpoint_object)
        {
            return false;
        }
    }

    endpoint_ids.contains(default_endpoint_id)
}

fn profile_has_valid_tool(tools: Option<&Value>) -> bool {
    let Some(tools_object) = tools.and_then(Value::as_object) else {
        return false;
    };

    let mut has_supported_tool = false;
    for tool_key in SUPPORTED_PROFILE_TOOLS {
        let Some(tool_value) = tools_object.get(tool_key) else {
            continue;
        };
        let Some(tool_object) = tool_value.as_object() else {
            return false;
        };
        if !tool_has_valid_endpoints(tool_object) {
            return false;
        }
        has_supported_tool = true;
    }

    has_supported_tool
}

fn profile_has_valid_compat(compat: Option<&Value>) -> bool {
    let Some(compat) = compat else {
        return true;
    };
    let Some(compat_object) = compat.as_object() else {
        return false;
    };

    for rules in compat_object.values() {
        let Some(rules) = rules.as_array() else {
            return false;
        };
        if rules.is_empty() {
            return false;
        }
        for rule in rules {
            let Some(rule) = rule.as_str().map(str::trim) else {
                return false;
            };
            if rule.is_empty()
                || !SUPPORTED_COMPAT_RULES.iter().any(|registration| {
                    registration.name == rule && registration.has_static_evidence()
                })
            {
                return false;
            }
        }
    }

    true
}

pub(crate) fn is_valid_gateway_provider_profiles(data: &Value) -> bool {
    let Some(object) = data.as_object() else {
        return false;
    };
    if object
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .is_none_or(|version| version != 1)
    {
        return false;
    }
    let Some(profiles) = object.get("profiles").and_then(Value::as_array) else {
        return false;
    };
    if profiles.is_empty() {
        return false;
    }

    let mut seen_ids = HashSet::new();
    for profile in profiles {
        let Some(profile_object) = profile.as_object() else {
            return false;
        };
        let Some(id) = profile_object
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            return false;
        };
        if id.is_empty() || !seen_ids.insert(id.to_string()) {
            return false;
        }
        if text_field_is_empty(profile_object, "providerType")
            || text_field_is_empty(profile_object, "label")
            || !profile_has_valid_tool(profile_object.get("tools"))
            || !profile_has_valid_compat(profile_object.get("compat"))
        {
            return false;
        }
    }

    true
}

/// Find one tool endpoint in the provider profile catalog.
///
/// Shared by the gateway runtime (which resolves `meta.gatewayProfile` into
/// effective provider fields) and by ordinary Coding commands that need a
/// preset's declared capability without running the gateway.
pub(crate) fn find_gateway_profile_endpoint<'a>(
    catalog: &'a Value,
    tool: &str,
    profile_id: &str,
    endpoint_id: &str,
) -> Option<(&'a Value, &'a Value)> {
    let profile = catalog
        .get("profiles")
        .and_then(Value::as_array)?
        .iter()
        .find(|profile| {
            profile
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|id| id == profile_id)
        })?;
    let endpoint = profile
        .get("tools")
        .and_then(|tools| tools.get(tool))
        .and_then(|tool_profile| tool_profile.get("endpoints"))
        .and_then(Value::as_array)?
        .iter()
        .find(|endpoint| {
            endpoint
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|id| id == endpoint_id)
        })?;
    Some((profile, endpoint))
}

fn validate_gateway_provider_profile_compatibility(
    previous: &Value,
    next: &Value,
) -> Result<(), String> {
    let previous_profiles = previous
        .get("profiles")
        .and_then(Value::as_array)
        .ok_or_else(|| "Previous provider profiles catalog is invalid".to_string())?;
    let next_profiles = next
        .get("profiles")
        .and_then(Value::as_array)
        .ok_or_else(|| "Next provider profiles catalog is invalid".to_string())?;

    for previous_profile in previous_profiles {
        let profile_id = previous_profile
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Previous provider profile is missing id".to_string())?;
        let next_profile = next_profiles
            .iter()
            .find(|profile| profile.get("id").and_then(Value::as_str) == Some(profile_id))
            .ok_or_else(|| format!("Provider profile id '{profile_id}' cannot be removed"))?;

        let previous_tools = previous_profile
            .get("tools")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("Provider profile '{profile_id}' has invalid tools"))?;
        let next_tools = next_profile
            .get("tools")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("Provider profile '{profile_id}' has invalid tools"))?;

        for tool_key in SUPPORTED_PROFILE_TOOLS {
            let Some(previous_tool) = previous_tools.get(tool_key) else {
                continue;
            };
            let next_tool = next_tools.get(tool_key).ok_or_else(|| {
                format!("Provider profile '{profile_id}' tool '{tool_key}' cannot be removed")
            })?;
            let previous_endpoints = previous_tool
                .get("endpoints")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    format!(
                        "Provider profile '{profile_id}' tool '{tool_key}' has invalid endpoints"
                    )
                })?;
            let next_endpoints = next_tool
                .get("endpoints")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    format!(
                        "Provider profile '{profile_id}' tool '{tool_key}' has invalid endpoints"
                    )
                })?;

            for previous_endpoint in previous_endpoints {
                let endpoint_id = previous_endpoint
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        format!(
                            "Provider profile '{profile_id}' tool '{tool_key}' has an endpoint without id"
                        )
                    })?;
                if !next_endpoints
                    .iter()
                    .any(|endpoint| endpoint.get("id").and_then(Value::as_str) == Some(endpoint_id))
                {
                    return Err(format!(
                        "Provider profile '{profile_id}' tool '{tool_key}' endpoint id '{endpoint_id}' cannot be removed"
                    ));
                }
            }
        }
    }

    Ok(())
}

/// Fill endpoint fields the bundled catalog declares but an older cached or
/// remote catalog does not carry yet.
///
/// A remote catalog can update a preset without an app release, but new fields
/// shipped with the app (for example a preset's `codexResponsesToolProfile`)
/// must still reach users whose cache predates them. Explicit values in the
/// catalog always win, so remote can override a bundled default.
fn merge_bundled_endpoint_defaults(catalog: &mut Value, bundled: &Value) {
    let Some(bundled_profiles) = bundled["profiles"].as_array() else {
        return;
    };
    for bundled_profile in bundled_profiles {
        let profile_id = bundled_profile["id"].as_str().unwrap_or_default();
        let Some(catalog_profile) = catalog["profiles"]
            .as_array_mut()
            .and_then(|profiles| {
                profiles
                    .iter_mut()
                    .find(|profile| profile["id"].as_str() == Some(profile_id))
            })
        else {
            continue;
        };
        let Some(bundled_tools) = bundled_profile["tools"].as_object() else {
            continue;
        };
        let Some(catalog_tools) = catalog_profile["tools"].as_object_mut() else {
            continue;
        };
        for (tool_key, bundled_tool) in bundled_tools {
            let Some(catalog_tool) = catalog_tools.get_mut(tool_key) else {
                continue;
            };
            let (Some(bundled_endpoints), Some(catalog_endpoints)) = (
                bundled_tool["endpoints"].as_array(),
                catalog_tool["endpoints"].as_array_mut(),
            ) else {
                continue;
            };
            for bundled_endpoint in bundled_endpoints {
                let endpoint_id = bundled_endpoint["id"].as_str().unwrap_or_default();
                let Some(catalog_endpoint) = catalog_endpoints
                    .iter_mut()
                    .find(|endpoint| endpoint["id"].as_str() == Some(endpoint_id))
                else {
                    continue;
                };
                let (Some(catalog_object), Some(bundled_object)) =
                    (catalog_endpoint.as_object_mut(), bundled_endpoint.as_object())
                else {
                    continue;
                };
                for (key, value) in bundled_object {
                    if !catalog_object.contains_key(key) {
                        catalog_object.insert(key.clone(), value.clone());
                    }
                }
            }
        }
    }
}

#[tauri::command]
pub fn load_cached_gateway_provider_profiles() -> Result<Option<Value>, String> {
    let bundled = get_bundled_gateway_provider_profiles();
    if let Some(mut data) = read_cache_file() {
        if is_valid_gateway_provider_profiles(&data) {
            let is_compatible = bundled.as_ref().is_none_or(|previous| {
                validate_gateway_provider_profile_compatibility(previous, &data)
                    .inspect_err(|error| {
                        log::warn!(
                            "[GatewayProviderProfiles] Ignoring incompatible cache: {error}"
                        );
                    })
                    .is_ok()
            });
            if is_compatible {
                if let Some(bundled) = bundled.as_ref() {
                    merge_bundled_endpoint_defaults(&mut data, bundled);
                }
                return Ok(Some(data));
            }
        }
    }
    Ok(bundled)
}

pub(crate) fn load_gateway_provider_profiles_for_runtime() -> Option<Value> {
    get_active_gateway_provider_profiles()
        .or_else(|| load_cached_gateway_provider_profiles().ok().flatten())
}

#[tauri::command]
pub async fn fetch_remote_gateway_provider_profiles(
    state: tauri::State<'_, SqliteDbState>,
    url: String,
) -> Result<Value, String> {
    let client = http_client::client_with_timeout(&state, 30).await?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("Failed to fetch remote provider profiles: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Remote provider profiles request failed: {}",
            response.status()
        ));
    }

    let mut json: Value = response
        .json()
        .await
        .map_err(|error| format!("Failed to parse remote provider profiles JSON: {error}"))?;

    if !is_valid_gateway_provider_profiles(&json) {
        return Err("Remote provider profiles JSON is invalid".to_string());
    }

    if let Some(previous) = load_gateway_provider_profiles_for_runtime() {
        validate_gateway_provider_profile_compatibility(&previous, &json).map_err(|error| {
            format!("Remote provider profiles catalog is incompatible: {error}")
        })?;
    }

    // A remote catalog written before this app version may not carry fields
    // the bundled baseline already declares; keep remote values authoritative
    // but fill the gaps so the UI and runtime agree.
    if let Some(bundled) = get_bundled_gateway_provider_profiles() {
        merge_bundled_endpoint_defaults(&mut json, &bundled);
    }

    set_active_gateway_provider_profiles(json.clone());

    if let Err(error) = write_cache_file(&json) {
        log::warn!("[GatewayProviderProfiles] Failed to write cache: {error}");
    } else {
        log::info!("[GatewayProviderProfiles] Cache updated from remote");
    }

    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_catalog() -> Value {
        json!({
            "schemaVersion": 1,
            "profiles": [
                {
                    "id": "deepseek",
                    "providerType": "deepseek",
                    "label": "DeepSeek",
                    "tools": {
                        "claude": {
                            "defaultEndpointId": "anthropic",
                            "endpoints": [
                                {
                                    "id": "anthropic",
                                    "label": "Anthropic",
                                    "apiFormat": "anthropic",
                                    "baseUrl": "https://api.deepseek.com/anthropic"
                                }
                            ]
                        }
                    }
                }
            ]
        })
    }

    #[test]
    fn bundled_gateway_provider_profiles_are_valid() {
        let bundled = get_bundled_gateway_provider_profiles();
        assert!(bundled.is_some());
    }

    #[test]
    fn bundled_doubao_coding_plan_targets_the_coding_endpoints() {
        let bundled = get_bundled_gateway_provider_profiles().unwrap();
        let profiles = bundled["profiles"].as_array().unwrap();

        let coding_plan = profiles
            .iter()
            .find(|profile| profile["id"].as_str() == Some("doubao_coding_plan"))
            .expect("bundled DouBao Coding Plan profile");
        let codex_endpoints = coding_plan["tools"]["codex"]["endpoints"]
            .as_array()
            .unwrap();
        let responses = codex_endpoints
            .iter()
            .find(|endpoint| endpoint["id"].as_str() == Some("openai_responses"))
            .unwrap();
        assert_eq!(
            responses["baseUrl"].as_str(),
            Some("https://ark.cn-beijing.volces.com/api/coding/v3")
        );
        assert_eq!(responses["model"].as_str(), Some("deepseek-v4.1-flash"));
        assert_eq!(
            responses["codexResponsesToolProfile"].as_str(),
            Some("safe")
        );
        let model_slugs: Vec<&str> = responses["modelCatalog"]["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|model| model["model"].as_str())
            .collect();
        assert!(model_slugs.contains(&"deepseek-v4.1-flash"));
        assert!(model_slugs.contains(&"ark-code-latest"));

        // The pay-as-you-go profile must keep the standard Ark endpoint.
        let pay_as_you_go = profiles
            .iter()
            .find(|profile| profile["id"].as_str() == Some("doubao"))
            .unwrap();
        let standard = pay_as_you_go["tools"]["codex"]["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .find(|endpoint| endpoint["id"].as_str() == Some("openai_responses"))
            .unwrap();
        assert_eq!(
            standard["baseUrl"].as_str(),
            Some("https://ark.cn-beijing.volces.com/api/v3")
        );
        assert_eq!(standard["codexResponsesToolProfile"].as_str(), Some("safe"));
    }

    #[test]
    fn bundled_endpoint_defaults_fill_older_catalogs_without_overriding_explicit_values() {
        let bundled = get_bundled_gateway_provider_profiles().expect("bundled catalog");
        let mut cached = bundled.clone();

        // Simulate a cache written before the field existed.
        for profile in cached["profiles"].as_array_mut().unwrap() {
            let Some(tools) = profile.get_mut("tools").and_then(Value::as_object_mut) else {
                continue;
            };
            for tool in tools.values_mut() {
                let Some(endpoints) = tool.get_mut("endpoints").and_then(Value::as_array_mut)
                else {
                    continue;
                };
                for endpoint in endpoints {
                    if let Some(object) = endpoint.as_object_mut() {
                        object.remove("codexResponsesToolProfile");
                    }
                }
            }
        }

        merge_bundled_endpoint_defaults(&mut cached, &bundled);
        let responses = cached["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["id"] == "doubao")
            .unwrap()["tools"]["codex"]["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .find(|endpoint| endpoint["id"] == "openai_responses")
            .unwrap();
        assert_eq!(
            responses["codexResponsesToolProfile"].as_str(),
            Some("safe")
        );

        // An explicit value from the catalog always wins.
        {
            let explicit = cached["profiles"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|profile| profile["id"] == "doubao")
                .unwrap()["tools"]["codex"]["endpoints"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|endpoint| endpoint["id"] == "openai_responses")
                .unwrap();
            explicit["codexResponsesToolProfile"] = json!("full");
        }
        merge_bundled_endpoint_defaults(&mut cached, &bundled);
        let explicit = cached["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["id"] == "doubao")
            .unwrap()["tools"]["codex"]["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .find(|endpoint| endpoint["id"] == "openai_responses")
            .unwrap();
        assert_eq!(
            explicit["codexResponsesToolProfile"].as_str(),
            Some("full")
        );
    }

    #[test]
    fn empty_profiles_are_rejected() {
        assert!(!is_valid_gateway_provider_profiles(&json!({
            "schemaVersion": 1,
            "profiles": []
        })));
    }

    #[test]
    fn duplicate_profile_ids_are_rejected() {
        let mut catalog = valid_catalog();
        let duplicate = catalog["profiles"][0].clone();
        catalog["profiles"].as_array_mut().unwrap().push(duplicate);
        assert!(!is_valid_gateway_provider_profiles(&catalog));
    }

    #[test]
    fn missing_provider_type_is_rejected() {
        let mut catalog = valid_catalog();
        catalog["profiles"][0]
            .as_object_mut()
            .unwrap()
            .remove("providerType");
        assert!(!is_valid_gateway_provider_profiles(&catalog));
    }

    #[test]
    fn missing_tool_endpoints_are_rejected() {
        let mut catalog = valid_catalog();
        catalog["profiles"][0]["tools"]["claude"]
            .as_object_mut()
            .unwrap()
            .remove("endpoints");
        assert!(!is_valid_gateway_provider_profiles(&catalog));
    }

    #[test]
    fn invalid_endpoint_api_format_is_rejected() {
        let mut catalog = valid_catalog();
        catalog["profiles"][0]["tools"]["claude"]["endpoints"][0]["apiFormat"] =
            json!("unknown_format");
        assert!(!is_valid_gateway_provider_profiles(&catalog));
    }

    #[test]
    fn gemini_tool_profiles_are_valid() {
        let mut catalog = valid_catalog();
        catalog["profiles"][0]["tools"]
            .as_object_mut()
            .unwrap()
            .remove("claude");
        catalog["profiles"][0]["tools"]["gemini"] = json!({
            "defaultEndpointId": "openai_chat",
            "endpoints": [
                {
                    "id": "openai_chat",
                    "label": "OpenAI Chat",
                    "apiFormat": "openai_chat",
                    "baseUrl": "https://api.deepseek.com/v1"
                }
            ]
        });

        assert!(is_valid_gateway_provider_profiles(&catalog));
    }

    #[test]
    fn grok_tool_profiles_are_valid() {
        let mut catalog = valid_catalog();
        catalog["profiles"][0]["tools"]
            .as_object_mut()
            .unwrap()
            .remove("claude");
        catalog["profiles"][0]["tools"]["grok"] = json!({
            "defaultEndpointId": "openai_chat",
            "endpoints": [
                {
                    "id": "openai_chat",
                    "label": "OpenAI Chat",
                    "apiFormat": "openai_chat",
                    "baseUrl": "https://api.deepseek.com/v1"
                }
            ]
        });

        assert!(is_valid_gateway_provider_profiles(&catalog));
    }

    #[test]
    fn unknown_compat_rules_are_rejected() {
        let mut catalog = valid_catalog();
        catalog["profiles"][0]["compat"] = json!({
            "openaiChat": ["unknown_provider_compat"]
        });

        assert!(!is_valid_gateway_provider_profiles(&catalog));
    }

    #[test]
    fn compat_rule_registry_has_unique_names_and_static_evidence() {
        let runtime_source = format!(
            "{}\n{}",
            include_str!("runtime/upstream.rs"),
            include_str!("runtime/compat/xai_responses.rs")
        );
        let mut names = HashSet::new();
        for registration in SUPPORTED_COMPAT_RULES {
            assert!(
                names.insert(registration.name),
                "duplicate compat rule name"
            );
            assert!(registration.has_static_evidence());
            let owner_function = registration
                .runtime_owner
                .rsplit("::")
                .next()
                .expect("runtime owner function");
            assert!(
                runtime_source.contains(&format!("fn {owner_function}")),
                "compat rule '{}' references missing runtime owner '{}'",
                registration.name,
                registration.runtime_owner
            );
            assert!(
                runtime_source.contains(&format!("fn {}", registration.test_name)),
                "compat rule '{}' references missing test '{}'",
                registration.name,
                registration.test_name
            );
        }
    }

    #[test]
    fn runtime_profiles_prefer_active_remote_catalog() {
        let mut first_catalog = get_bundled_gateway_provider_profiles().expect("bundled catalog");
        first_catalog["updatedAt"] = json!("remote-first");
        set_active_gateway_provider_profiles(first_catalog);

        let first_loaded = load_gateway_provider_profiles_for_runtime().expect("runtime catalog");
        assert_eq!(first_loaded["updatedAt"], "remote-first");

        let mut second_catalog = get_bundled_gateway_provider_profiles().expect("bundled catalog");
        second_catalog["updatedAt"] = json!("remote-second");
        set_active_gateway_provider_profiles(second_catalog);

        let second_loaded = load_gateway_provider_profiles_for_runtime().expect("runtime catalog");
        assert_eq!(second_loaded["updatedAt"], "remote-second");
    }

    #[test]
    fn default_endpoint_must_exist() {
        let mut catalog = valid_catalog();
        catalog["profiles"][0]["tools"]["claude"]["defaultEndpointId"] = json!("missing");
        assert!(!is_valid_gateway_provider_profiles(&catalog));
    }

    #[test]
    fn compatible_catalog_accepts_unchanged_stable_ids_and_metadata_updates() {
        let previous = valid_catalog();
        let mut next = previous.clone();
        next["profiles"][0]["providerType"] = json!("deepseek-compatible");
        next["profiles"][0]["label"] = json!("Updated DeepSeek");
        next["profiles"][0]["tools"]["claude"]["endpoints"][0]["label"] =
            json!("Updated Anthropic");
        next["profiles"][0]["tools"]["claude"]["endpoints"][0]["baseUrl"] =
            json!("https://updated.example.com/anthropic");

        assert!(validate_gateway_provider_profile_compatibility(&previous, &next).is_ok());
    }

    #[test]
    fn compatible_catalog_accepts_added_profile_tool_and_endpoint_ids() {
        let previous = valid_catalog();
        let mut next = previous.clone();
        next["profiles"][0]["tools"]["codex"] = json!({
            "defaultEndpointId": "chat",
            "endpoints": [{
                "id": "chat",
                "label": "Chat",
                "apiFormat": "openai_chat",
                "baseUrl": "https://api.deepseek.com/v1"
            }]
        });
        next["profiles"][0]["tools"]["claude"]["endpoints"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "chat",
                "label": "Chat",
                "apiFormat": "openai_chat",
                "baseUrl": "https://api.deepseek.com/v1"
            }));
        next["profiles"].as_array_mut().unwrap().push(json!({
            "id": "new-provider",
            "providerType": "new-provider",
            "label": "New Provider",
            "tools": {
                "claude": {
                    "defaultEndpointId": "anthropic",
                    "endpoints": [{
                        "id": "anthropic",
                        "label": "Anthropic",
                        "apiFormat": "anthropic",
                        "baseUrl": "https://new.example.com/anthropic"
                    }]
                }
            }
        }));

        assert!(validate_gateway_provider_profile_compatibility(&previous, &next).is_ok());
    }

    #[test]
    fn incompatible_catalog_rejects_removed_profile_id() {
        let previous = valid_catalog();
        let mut next = previous.clone();
        next["profiles"].as_array_mut().unwrap().clear();

        let error = validate_gateway_provider_profile_compatibility(&previous, &next)
            .expect_err("removed profile must be rejected");
        assert!(error.contains("deepseek"));
    }

    #[test]
    fn incompatible_catalog_rejects_removed_tool_key() {
        let previous = valid_catalog();
        let mut next = previous.clone();
        next["profiles"][0]["tools"]
            .as_object_mut()
            .unwrap()
            .remove("claude");

        let error = validate_gateway_provider_profile_compatibility(&previous, &next)
            .expect_err("removed tool must be rejected");
        assert!(error.contains("deepseek"));
        assert!(error.contains("claude"));
    }

    #[test]
    fn incompatible_catalog_rejects_removed_endpoint_id() {
        let previous = valid_catalog();
        let mut next = previous.clone();
        next["profiles"][0]["tools"]["claude"]["endpoints"]
            .as_array_mut()
            .unwrap()
            .clear();

        let error = validate_gateway_provider_profile_compatibility(&previous, &next)
            .expect_err("removed endpoint must be rejected");
        assert!(error.contains("deepseek"));
        assert!(error.contains("claude"));
        assert!(error.contains("anthropic"));
    }

    #[test]
    fn valid_cache_is_loaded_before_bundled_defaults() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        set_cache_dir(temp_dir.path().to_path_buf());
        let mut catalog = get_bundled_gateway_provider_profiles().expect("bundled catalog");
        let deepseek = catalog["profiles"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|profile| profile["id"] == "deepseek")
            .expect("deepseek profile");
        deepseek["label"] = json!("Cached DeepSeek");
        write_cache_file(&catalog).expect("write cache");

        let loaded = load_cached_gateway_provider_profiles()
            .expect("load")
            .expect("catalog");
        let loaded_deepseek = loaded["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["id"] == "deepseek")
            .expect("deepseek profile");
        assert_eq!(loaded_deepseek["label"], "Cached DeepSeek");
    }
}
