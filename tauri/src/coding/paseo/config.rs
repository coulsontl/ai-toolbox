//! Pure patch helpers for Paseo's persisted `config.json`.
//!
//! Only `agents.providers.<provider>.models` is written. Everything else —
//! daemon settings, relay/auth, other providers, `additionalModels` — is
//! preserved byte-for-byte at the JSON value level.

use serde_json::{Map, Value};

use super::{PaseoModelEntry, PaseoProvider};

pub fn patch_paseo_config_text(
    existing: &str,
    provider: PaseoProvider,
    models: &[PaseoModelEntry],
) -> Result<String, String> {
    if existing.trim().is_empty() {
        return Err("Paseo config is empty".to_string());
    }

    let mut root: Value = serde_json::from_str(existing)
        .map_err(|error| format!("Invalid Paseo config JSON: {error}"))?;
    let root_object = root
        .as_object_mut()
        .ok_or_else(|| "Paseo config root is not an object".to_string())?;

    let agents = ensure_object(root_object, "agents", "agents")?;
    let providers = ensure_object(agents, "providers", "agents.providers")?;
    let provider_path = format!("agents.providers.{}", provider.key());
    let provider_object = ensure_object(providers, provider.key(), &provider_path)?;

    provider_object.insert(
        "models".to_string(),
        serde_json::to_value(models)
            .map_err(|error| format!("Failed to serialize Paseo models: {error}"))?,
    );

    let mut output = serde_json::to_string_pretty(&root)
        .map_err(|error| format!("Failed to serialize Paseo config: {error}"))?;
    output.push('\n');
    Ok(output)
}

fn ensure_object<'a>(
    parent: &'a mut Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    let value = parent
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    value
        .as_object_mut()
        .ok_or_else(|| format!("Paseo config field `{path}` is not an object"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model(id: &str, label: &str, is_default: bool) -> PaseoModelEntry {
        PaseoModelEntry {
            id: id.to_string(),
            label: label.to_string(),
            description: None,
            is_default: is_default.then_some(true),
            thinking_options: None,
        }
    }

    #[test]
    fn patch_replaces_models_and_preserves_every_other_field() {
        let existing = json!({
            "$schema": "https://paseo.sh/schemas/paseo.config.v1.json",
            "version": 1,
            "daemon": {
                "listen": "127.0.0.1:6767",
                "relay": { "enabled": true, "endpoint": "relay.example:28443" }
            },
            "agents": {
                "providers": {
                    "claude": {
                        "enabled": true,
                        "additionalModels": [{ "id": "manual", "label": "Manual" }],
                        "models": [{ "id": "old", "label": "Old" }]
                    },
                    "codex": {
                        "models": [{ "id": "codex-old", "label": "Codex Old" }]
                    }
                },
                "metadataGeneration": {
                    "providers": [{ "provider": "claude", "model": "claude-haiku-4-5" }]
                }
            }
        })
        .to_string();

        let updated = patch_paseo_config_text(
            &existing,
            PaseoProvider::Claude,
            &[model("opus[1M]", "Opus", true), model("haiku", "Haiku", false)],
        )
        .expect("patch config");
        let parsed: Value = serde_json::from_str(&updated).expect("valid JSON");

        assert_eq!(parsed["daemon"]["listen"], "127.0.0.1:6767");
        assert_eq!(parsed["agents"]["providers"]["claude"]["enabled"], true);
        assert_eq!(
            parsed["agents"]["providers"]["claude"]["additionalModels"],
            json!([{ "id": "manual", "label": "Manual" }])
        );
        assert_eq!(
            parsed["agents"]["providers"]["claude"]["models"],
            json!([
                { "id": "opus[1M]", "label": "Opus", "isDefault": true },
                { "id": "haiku", "label": "Haiku" }
            ])
        );
        // The other provider is untouched.
        assert_eq!(
            parsed["agents"]["providers"]["codex"]["models"],
            json!([{ "id": "codex-old", "label": "Codex Old" }])
        );
        assert_eq!(
            parsed["agents"]["metadataGeneration"]["providers"][0]["model"],
            "claude-haiku-4-5"
        );
    }

    #[test]
    fn patch_creates_missing_provider_sections() {
        let existing = json!({ "version": 1 }).to_string();

        let updated = patch_paseo_config_text(
            &existing,
            PaseoProvider::Codex,
            &[model("gpt-6-astra", "GPT-6 Astra", true)],
        )
        .expect("patch config");
        let parsed: Value = serde_json::from_str(&updated).expect("valid JSON");

        assert_eq!(
            parsed["agents"]["providers"]["codex"]["models"],
            json!([{ "id": "gpt-6-astra", "label": "GPT-6 Astra", "isDefault": true }])
        );
    }

    #[test]
    fn patch_rejects_non_object_sections() {
        let existing = json!({ "agents": [] }).to_string();

        let error = patch_paseo_config_text(
            &existing,
            PaseoProvider::Claude,
            &[model("haiku", "Haiku", false)],
        )
        .expect_err("non-object agents");

        assert!(error.contains("agents"), "unexpected error: {error}");
    }

    #[test]
    fn patch_rejects_empty_or_non_json_config() {
        assert!(patch_paseo_config_text("", PaseoProvider::Claude, &[]).is_err());
        assert!(patch_paseo_config_text("not-json", PaseoProvider::Claude, &[]).is_err());
    }
}
