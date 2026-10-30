//! The channel view both WorkBuddy desktop and CodeBuddy Code render.
//!
//! `models.json` is a flat list of models with no provider concept. The page
//! shows "providers", so one is derived by grouping entries that share an
//! endpoint **and** an API key — the pair a user thinks of as one account.
//! Because the grouping is a view rather than a record, the id is a hash of
//! that pair: stable across reloads, and never stored.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::models_json;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    pub id: String,
    pub name: String,
    pub base_url: String,
    /// Plaintext: AI Toolbox is a configuration manager, so credentials are
    /// shown and editable rather than masked.
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    pub supports_tool_call: bool,
    pub supports_images: bool,
    pub supports_reasoning: bool,
    pub only_reasoning: bool,
    pub use_custom_protocol: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelView {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub base_url: String,
    pub api_key: String,
    pub models: Vec<ModelView>,
}

/// Groups `models.json` entries into channels, preserving file order.
///
/// An entry without a `url` cannot be grouped and is skipped; it is still left
/// untouched on disk, so a page reload simply does not list it.
pub fn build_channel_views(models: &[Value]) -> Vec<ChannelView> {
    let mut channels: Vec<ChannelView> = Vec::new();
    for entry in models {
        let Some(model) = model_view(entry) else {
            continue;
        };
        let Some((base_url, api_key)) = models_json::channel_key_of(entry) else {
            continue;
        };
        let vendor = entry
            .get("vendor")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();

        match channels
            .iter_mut()
            .find(|channel| channel.base_url == base_url && channel.api_key == api_key)
        {
            Some(channel) => {
                if channel.name.is_empty() && !vendor.is_empty() {
                    channel.name = vendor.clone();
                    channel.vendor = vendor;
                }
                channel.models.push(model);
            }
            None => channels.push(ChannelView {
                id: channel_id(&base_url, &api_key),
                name: if vendor.is_empty() {
                    base_url.clone()
                } else {
                    vendor.clone()
                },
                vendor,
                base_url,
                api_key,
                models: vec![model],
            }),
        }
    }
    channels
}

/// Stable id for a `(base_url, api_key)` pair (FNV-1a, 64-bit).
pub fn channel_id(base_url: &str, api_key: &str) -> String {
    let seed = format!("{base_url}\u{1f}{api_key}");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in seed.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("channel-{hash:016x}")
}

/// One entry's page-facing fields, or `None` when it has no usable id.
fn model_view(entry: &Value) -> Option<ModelView> {
    let id = models_json::entry_id(entry)?;
    Some(ModelView {
        name: entry
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&id)
            .to_string(),
        base_url: entry
            .get("url")
            .and_then(Value::as_str)
            .map(models_json::normalize_channel_base_url)
            .unwrap_or_default(),
        api_key: entry
            .get("apiKey")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        max_input_tokens: entry.get("maxInputTokens").and_then(Value::as_u64),
        max_output_tokens: entry.get("maxOutputTokens").and_then(Value::as_u64),
        temperature: entry.get("temperature").and_then(Value::as_f64),
        supports_tool_call: bool_field(entry, "supportsToolCall", true),
        supports_images: bool_field(entry, "supportsImages", false),
        supports_reasoning: bool_field(entry, "supportsReasoning", false),
        only_reasoning: bool_field(entry, "onlyReasoning", false),
        use_custom_protocol: bool_field(entry, "useCustomProtocol", false),
        id,
    })
}

fn bool_field(entry: &Value, key: &str, default: bool) -> bool {
    entry.get(key).and_then(Value::as_bool).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn entries_sharing_an_endpoint_and_key_become_one_channel() {
        let models = vec![
            json!({"id":"a","url":"https://x/v1/chat/completions","apiKey":"k","vendor":"Acme"}),
            json!({"id":"b","url":"https://x/v1/","apiKey":"k"}),
            json!({"id":"c","url":"https://y/v1/chat/completions","apiKey":"k","vendor":"Beta"}),
        ];
        let channels = build_channel_views(&models);
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0].name, "Acme");
        assert_eq!(channels[0].base_url, "https://x/v1");
        assert_eq!(channels[0].models.len(), 2);
        assert_eq!(channels[1].name, "Beta");
    }

    #[test]
    fn a_different_key_splits_the_channel_even_on_the_same_endpoint() {
        let models = vec![
            json!({"id":"a","url":"https://x/v1","apiKey":"k1"}),
            json!({"id":"b","url":"https://x/v1","apiKey":"k2"}),
        ];
        assert_eq!(build_channel_views(&models).len(), 2);
    }

    #[test]
    fn an_entry_without_a_url_is_skipped_rather_than_grouped_under_empty() {
        assert!(build_channel_views(&[json!({"id":"a","name":"No URL"})]).is_empty());
    }

    #[test]
    fn a_channel_without_a_vendor_falls_back_to_its_endpoint() {
        let channels = build_channel_views(&[json!({"id":"a","url":"https://x/v1","apiKey":"k"})]);
        assert_eq!(channels[0].name, "https://x/v1");
        assert!(channels[0].vendor.is_empty());
    }

    #[test]
    fn the_channel_id_is_stable_and_key_sensitive() {
        assert_eq!(channel_id("https://x/v1", "k"), channel_id("https://x/v1", "k"));
        assert_ne!(channel_id("https://x/v1", "k"), channel_id("https://x/v1", "k2"));
    }

    #[test]
    fn model_rows_default_the_flags_the_products_use() {
        let channels =
            build_channel_views(&[json!({"id":"a","url":"https://x/v1","apiKey":"k"})]);
        let model = &channels[0].models[0];
        assert!(model.supports_tool_call, "the products default tool calling to on");
        assert!(!model.supports_images);
        assert!(!model.supports_reasoning);
        assert_eq!(model.name, "a", "a missing name falls back to the id");
    }
}
