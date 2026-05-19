use crate::coding::proxy_gateway::types::GatewayCliKey;
use crate::coding::{claude_code, codex, gemini_cli};
use crate::db::helpers::db_list;
use crate::db::schema::{DbTable, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use serde_json::Value;
use toml_edit::{DocumentMut, Item};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UpstreamProvider {
    pub(super) cli_key: GatewayCliKey,
    pub(super) id: String,
    pub(super) name: String,
    pub(super) base_url: String,
    pub(super) api_key: String,
    pub(super) sort_index: Option<i32>,
    pub(super) model_mapping: UpstreamModelMapping,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct UpstreamModelMapping {
    pub(super) default_model: Option<String>,
    pub(super) haiku_model: Option<String>,
    pub(super) sonnet_model: Option<String>,
    pub(super) opus_model: Option<String>,
    pub(super) reasoning_model: Option<String>,
}

pub(super) async fn load_candidate_providers(
    db: &SqliteDbState,
    cli_key: GatewayCliKey,
) -> Result<Vec<UpstreamProvider>, String> {
    let table = match cli_key {
        GatewayCliKey::Claude => DbTable::ClaudeProvider,
        GatewayCliKey::Codex => DbTable::CodexProvider,
        GatewayCliKey::Gemini => DbTable::GeminiCliProvider,
        GatewayCliKey::OpenCode => {
            return Err(
                "OpenCode adapter is intentionally out of scope for the gateway MVP".to_string(),
            )
        }
    };
    let order = OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::json_text("updated_at", OrderDirection::Desc)?,
    ]);
    let records = db.with_conn(|conn| db_list(conn, table, Some(&order)))?;

    let mut providers = Vec::new();
    let mut parse_errors = Vec::new();
    for record in records {
        match provider_from_record(cli_key, record) {
            Ok(Some(provider)) => providers.push(provider),
            Ok(None) => {}
            Err(error) => parse_errors.push(error),
        }
    }
    sort_candidate_providers(&mut providers);

    if providers.is_empty() && !parse_errors.is_empty() {
        return Err(parse_errors.join("; "));
    }

    Ok(providers)
}

fn sort_candidate_providers(providers: &mut [UpstreamProvider]) {
    providers.sort_by(|left, right| {
        left.sort_index
            .unwrap_or(0)
            .cmp(&right.sort_index.unwrap_or(0))
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn provider_from_record(
    cli_key: GatewayCliKey,
    record: Value,
) -> Result<Option<UpstreamProvider>, String> {
    match cli_key {
        GatewayCliKey::Claude => {
            let provider = claude_code::adapter::from_db_value_provider(record);
            if provider.is_disabled {
                return Ok(None);
            }
            let settings =
                parse_json_config(&provider.settings_config, "Claude provider settings_config")?;
            let env = settings.get("env").and_then(Value::as_object);
            let base_url = json_object_string(env, "ANTHROPIC_BASE_URL")
                .unwrap_or_else(|| "https://api.anthropic.com".to_string());
            let api_key = json_object_string(env, "ANTHROPIC_AUTH_TOKEN")
                .or_else(|| json_object_string(env, "ANTHROPIC_API_KEY"))
                .ok_or_else(|| format!("Claude provider '{}' has no API key", provider.name))?;
            let model_mapping = claude_model_mapping_from_settings(&settings);
            Ok(Some(UpstreamProvider {
                cli_key,
                id: provider.id,
                name: provider.name,
                base_url,
                api_key,
                sort_index: provider.sort_index,
                model_mapping,
            }))
        }
        GatewayCliKey::Codex => {
            let provider = codex::adapter::from_db_value_provider(record);
            if provider.is_disabled {
                return Ok(None);
            }
            let settings =
                parse_json_config(&provider.settings_config, "Codex provider settings_config")?;
            let auth = settings.get("auth").and_then(Value::as_object);
            let api_key = json_object_string(auth, "OPENAI_API_KEY").ok_or_else(|| {
                format!("Codex provider '{}' has no OPENAI_API_KEY", provider.name)
            })?;
            let config_toml = settings.get("config").and_then(Value::as_str).unwrap_or("");
            let base_url = codex_base_url_from_config(config_toml)
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            Ok(Some(UpstreamProvider {
                cli_key,
                id: provider.id,
                name: provider.name,
                base_url,
                api_key,
                sort_index: provider.sort_index,
                model_mapping: UpstreamModelMapping::default(),
            }))
        }
        GatewayCliKey::Gemini => {
            let provider = gemini_cli::adapter::from_db_value_provider(record);
            if provider.is_disabled {
                return Ok(None);
            }
            let settings = parse_json_config(
                &provider.settings_config,
                "Gemini CLI provider settings_config",
            )?;
            let env = settings.get("env").and_then(Value::as_object);
            let api_key = json_object_string(env, "GEMINI_API_KEY")
                .or_else(|| json_object_string(env, "GOOGLE_API_KEY"))
                .ok_or_else(|| format!("Gemini CLI provider '{}' has no API key", provider.name))?;
            let base_url = json_object_string(env, "GOOGLE_GEMINI_BASE_URL")
                .or_else(|| json_object_string(env, "GOOGLE_VERTEX_BASE_URL"))
                .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".to_string());
            Ok(Some(UpstreamProvider {
                cli_key,
                id: provider.id,
                name: provider.name,
                base_url,
                api_key,
                sort_index: provider.sort_index,
                model_mapping: UpstreamModelMapping::default(),
            }))
        }
        GatewayCliKey::OpenCode => unreachable!("OpenCode is rejected before query"),
    }
}

fn parse_json_config(raw: &str, label: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|error| format!("Failed to parse {label}: {error}"))
}

fn claude_model_mapping_from_settings(settings: &Value) -> UpstreamModelMapping {
    let env = settings.get("env").and_then(Value::as_object);
    UpstreamModelMapping {
        default_model: json_value_string(settings, "model")
            .or_else(|| json_object_string(env, "ANTHROPIC_MODEL")),
        haiku_model: json_value_string(settings, "haikuModel")
            .or_else(|| json_object_string(env, "ANTHROPIC_DEFAULT_HAIKU_MODEL")),
        sonnet_model: json_value_string(settings, "sonnetModel")
            .or_else(|| json_object_string(env, "ANTHROPIC_DEFAULT_SONNET_MODEL")),
        opus_model: json_value_string(settings, "opusModel")
            .or_else(|| json_object_string(env, "ANTHROPIC_DEFAULT_OPUS_MODEL")),
        reasoning_model: json_value_string(settings, "reasoningModel")
            .or_else(|| json_object_string(env, "ANTHROPIC_REASONING_MODEL")),
    }
}

fn json_value_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn json_object_string(
    object: Option<&serde_json::Map<String, Value>>,
    key: &str,
) -> Option<String> {
    object
        .and_then(|object| object.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn codex_base_url_from_config(config_toml: &str) -> Option<String> {
    let trimmed = config_toml.trim();
    if trimmed.is_empty() {
        return None;
    }
    let document = trimmed.parse::<DocumentMut>().ok()?;
    let root = document.as_table();
    let providers = root.get("model_providers")?.as_table()?;
    let selected_provider = root
        .get("model_provider")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(selected_provider) = selected_provider {
        if let Some(base_url) = providers
            .get(selected_provider)
            .and_then(Item::as_table)
            .and_then(|provider| provider.get("base_url"))
            .and_then(Item::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(base_url.to_string());
        }
    }

    let fallback = providers.iter().find_map(|(_, item)| {
        item.as_table()
            .and_then(|provider| provider.get("base_url"))
            .and_then(Item::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str, sort_index: Option<i32>) -> UpstreamProvider {
        UpstreamProvider {
            cli_key: GatewayCliKey::Claude,
            id: name.to_string(),
            name: name.to_string(),
            base_url: "https://api.example.com".to_string(),
            api_key: "key".to_string(),
            sort_index,
            model_mapping: UpstreamModelMapping::default(),
        }
    }

    #[test]
    fn candidate_providers_keep_topmost_sort_index_first() {
        let mut providers = vec![
            provider("third", Some(30)),
            provider("first", Some(10)),
            provider("second", Some(20)),
        ];

        sort_candidate_providers(&mut providers);

        let names: Vec<&str> = providers
            .iter()
            .map(|provider| provider.name.as_str())
            .collect();
        assert_eq!(names, vec!["first", "second", "third"]);
    }

    #[test]
    fn none_sort_index_treated_as_zero_matching_frontend() {
        let mut providers = vec![provider("second", Some(1)), provider("first", None)];

        sort_candidate_providers(&mut providers);

        let names: Vec<&str> = providers
            .iter()
            .map(|provider| provider.name.as_str())
            .collect();
        assert_eq!(names, vec!["first", "second"]);
    }

    #[test]
    fn claude_model_mapping_reads_reasoning_model() {
        let mapping = claude_model_mapping_from_settings(&serde_json::json!({
            "model": "provider-default",
            "reasoningModel": "provider-reasoning",
            "env": {
                "ANTHROPIC_REASONING_MODEL": "env-reasoning"
            }
        }));

        assert_eq!(mapping.default_model.as_deref(), Some("provider-default"));
        assert_eq!(
            mapping.reasoning_model.as_deref(),
            Some("provider-reasoning")
        );
    }
}
