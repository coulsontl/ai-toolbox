//! Credential redaction for every control-plane response.
//!
//! Redaction is deliberately structural-first: provider payloads are JSON, so
//! walking the tree and masking secret-named keys catches `apiKey`,
//! `ANTHROPIC_AUTH_TOKEN`, nested `auth` objects and custom headers without a
//! per-CLI allowlist. TOML/YAML/`.env` payloads are carried as strings and get
//! a second, conservative text pass.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

pub const REDACTED: &str = "***";

static TOML_ENV_SECRET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?im)^(\s*(?:export\s+)?(?:api[_-]?key|auth[_-]?token|access[_-]?token|refresh[_-]?token|bearer[_-]?token|experimental[_-]?bearer[_-]?token|client[_-]?secret|openai[_-]?api[_-]?key|gemini[_-]?api[_-]?key|google[_-]?api[_-]?key|anthropic[_-]?(?:auth[_-]?token|api[_-]?key)|password|secret)\s*[=:]\s*)(["']?)([^"'\r\n]*)(["']?)"#,
    )
    .expect("valid TOML/env secret regex")
});

static URL_QUERY_SECRET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)([?&](?:key|api[_-]?key|access[_-]?token|refresh[_-]?token|client[_-]?secret|token)=)[^&#\s]+"#,
    )
    .expect("valid URL query secret regex")
});

static BEARER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(bearer\s+)([A-Za-z0-9._~+/=-]{8,})").expect("valid bearer regex")
});

/// Masks secret-looking values inside a JSON tree in place.
pub fn redact_value(value: &mut Value) {
    match value {
        Value::Object(object) => redact_object(object),
        Value::Array(items) => {
            for item in items {
                redact_value(item);
            }
        }
        Value::String(text) => {
            if text.len() <= 8 * 1024 * 1024 {
                *text = redact_text(text);
            }
        }
        _ => {}
    }
}

fn redact_object(object: &mut Map<String, Value>) {
    for (key, value) in object.iter_mut() {
        if is_secret_key(key, value) {
            *value = Value::String(REDACTED.to_string());
            continue;
        }
        redact_value(value);
    }
}

/// Whether a JSON key/value pair must be fully masked.
fn is_secret_key(key: &str, value: &Value) -> bool {
    let normalized = key
        .chars()
        .filter(|ch| !matches!(ch, '_' | '-' | ' ' | '.'))
        .collect::<String>()
        .to_ascii_lowercase();

    if normalized.is_empty()
        || normalized == "hasapikey"
        || normalized == "apikeyenv"
        || normalized.ends_with("apikeyenv")
    {
        return false;
    }

    // Usage counters and model metadata legitimately contain "token".
    if matches!(
        normalized.as_str(),
        "inputtokens"
            | "outputtokens"
            | "totaltokens"
            | "cachereadtokens"
            | "cachecreationtokens"
            | "cachedtokens"
            | "maxtokens"
            | "maxoutputtokens"
            | "maxinputtokens"
            | "tokenlimit"
            | "tokencount"
            | "tokengroup"
            | "service_tier"
    ) {
        return false;
    }

    if matches!(
        normalized.as_str(),
        "apikey"
            | "authtoken"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "bearertoken"
            | "experimentalbearertoken"
            | "clientsecret"
            | "password"
            | "secret"
            | "authorization"
            | "cookie"
            | "setcookie"
            | "credential"
            | "credentials"
            | "sessiontoken"
    ) {
        return true;
    }

    if normalized.ends_with("apikey") || normalized.ends_with("secret") {
        return true;
    }

    if normalized.ends_with("token") {
        return true;
    }

    // A bare `key`/`longKey` is common in plugin/provider payloads. Provider
    // ids are short human slugs; real credentials are long opaque strings.
    if normalized.ends_with("key") {
        return value
            .as_str()
            .map(|text| text.len() >= 20 && !text.chars().any(char::is_whitespace))
            .unwrap_or(false);
    }

    false
}

/// Redacts obvious secret assignments inside a textual config blob.
pub fn redact_text(text: &str) -> String {
    let text = TOML_ENV_SECRET_RE.replace_all(text, |caps: &regex::Captures<'_>| {
        format!("{}{}{}{}", &caps[1], &caps[2], REDACTED, &caps[4])
    });
    let text = BEARER_RE.replace_all(&text, |caps: &regex::Captures<'_>| {
        format!("{}{}", &caps[1], REDACTED)
    });
    URL_QUERY_SECRET_RE
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            format!("{}{}", &caps[1], REDACTED)
        })
        .into_owned()
}

/// Parses a JSON string and redacts it, returning the tree. Non-JSON strings
/// stay strings and go through the text pass.
pub fn redact_json_string(raw: &str) -> Value {
    match serde_json::from_str::<Value>(raw) {
        Ok(mut value) => {
            redact_value(&mut value);
            value
        }
        Err(_) => Value::String(redact_text(raw)),
    }
}

/// True when the tree contains any non-empty credential-shaped value.
///
/// Call this *before* redaction; it powers `hasApiKey` in provider DTOs.
pub fn has_secret_value(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let non_empty = match value {
                Value::Null => false,
                Value::String(text) => !text.trim().is_empty(),
                _ => true,
            };
            (is_secret_key(key, value) && non_empty)
                || (!matches!(value, Value::String(_)) && has_secret_value(value))
        }),
        Value::Array(items) => items
            .iter()
            .any(|item| !matches!(item, Value::String(_)) && has_secret_value(item)),
        _ => false,
    }
}

/// Whether a value is one of our redaction placeholders.
pub fn is_redacted_value(value: &Value) -> bool {
    value
        .as_str()
        .map(|text| text == REDACTED || text.contains("***"))
        .unwrap_or(false)
}

/// Deep-merges a PATCH into an existing JSON object, preserving secrets that
/// the caller did not (or could not) supply.
///
/// Arrays replace wholesale; objects merge recursively; missing keys keep the
/// stored value. A redaction placeholder in the patch means "keep the stored
/// secret", so a `GET` -> edit -> `PATCH` round trip cannot write `***` into a
/// live config.
pub fn merge_patch_preserving_secrets(existing: &mut Value, patch: Value) {
    match (existing, patch) {
        (Value::Object(existing_map), Value::Object(patch_map)) => {
            for (key, patch_value) in patch_map {
                match existing_map.get_mut(&key) {
                    Some(existing_value) if patch_value.is_object() => {
                        merge_patch_preserving_secrets(existing_value, patch_value);
                    }
                    Some(existing_value)
                        if is_secret_key(&key, existing_value)
                            && (is_redacted_value(&patch_value) || patch_value.is_null()) =>
                    {
                        // Keep the stored credential.
                    }
                    _ => {
                        existing_map.insert(key, patch_value);
                    }
                }
            }
        }
        (existing_slot, patch_value) => {
            if !is_redacted_value(&patch_value) {
                *existing_slot = patch_value;
            }
        }
    }
}

/// Redacts every secret-looking key in a provider object while recording
/// whether one existed.
pub fn redact_provider_payload(value: &mut Value) -> bool {
    let had_secret = has_secret_value(value);
    redact_value(value);
    had_secret
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn masks_common_credential_keys_and_keeps_usage_counters() {
        let mut value = json!({
            "name": "demo",
            "settingsConfig": {
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "sk-secret-value",
                    "ANTHROPIC_API_KEY": "sk-legacy",
                    "ANTHROPIC_BASE_URL": "https://example.test"
                }
            },
            "auth": {"OPENAI_API_KEY": "sk-codex", "auth_mode": "apikey"},
            "apiKey": "sk-direct",
            "apiKeyEnv": "MY_KEY_REF",
            "tokens": 42,
            "input_tokens": 10,
            "max_tokens": 2048,
            "totalTokens": 52,
            "key": "short-provider-key",
            "longKey": "abcdefghijklmnopqrstuvwxyz0123456789",
            "nested": [{"password": "hunter2"}, {"value": "visible"}]
        });
        redact_value(&mut value);
        assert_eq!(
            value["settingsConfig"]["env"]["ANTHROPIC_AUTH_TOKEN"],
            REDACTED
        );
        assert_eq!(
            value["settingsConfig"]["env"]["ANTHROPIC_API_KEY"],
            REDACTED
        );
        assert_eq!(
            value["settingsConfig"]["env"]["ANTHROPIC_BASE_URL"],
            "https://example.test"
        );
        assert_eq!(value["auth"]["OPENAI_API_KEY"], REDACTED);
        assert_eq!(value["auth"]["auth_mode"], "apikey");
        assert_eq!(value["apiKey"], REDACTED);
        assert_eq!(value["apiKeyEnv"], "MY_KEY_REF");
        assert_eq!(value["tokens"], 42);
        assert_eq!(value["input_tokens"], 10);
        assert_eq!(value["max_tokens"], 2048);
        assert_eq!(value["totalTokens"], 52);
        assert_eq!(value["key"], "short-provider-key");
        assert_eq!(value["longKey"], REDACTED);
        assert_eq!(value["nested"][0]["password"], REDACTED);
        assert_eq!(value["nested"][1]["value"], "visible");
    }

    #[test]
    fn redacts_text_configs_and_urls() {
        let text = "model = \"gpt\"\nexperimental_bearer_token = \"sk-abc\"\nAPI_KEY=sk-def\nhttps://x.test/v1?key=sk-ghi&model=a\nAuthorization: Bearer sk-jklmnop";
        let redacted = redact_text(text);
        assert!(!redacted.contains("sk-abc"));
        assert!(!redacted.contains("sk-def"));
        assert!(!redacted.contains("sk-ghi"));
        assert!(!redacted.contains("sk-jklmnop"));
        assert!(redacted.contains("model = \"gpt\""));
        assert!(redacted.contains("model=a"));
    }

    #[test]
    fn patch_merge_preserves_redacted_secrets_but_applies_plain_fields() {
        let mut existing = json!({
            "name": "old",
            "apiKey": "sk-real",
            "settingsConfig": {"env": {"ANTHROPIC_AUTH_TOKEN": "sk-real", "ANTHROPIC_MODEL": "old"}}
        });
        merge_patch_preserving_secrets(
            &mut existing,
            json!({
                "name": "new",
                "apiKey": REDACTED,
                "settingsConfig": {"env": {"ANTHROPIC_AUTH_TOKEN": REDACTED, "ANTHROPIC_MODEL": "new"}}
            }),
        );
        assert_eq!(existing["name"], "new");
        assert_eq!(existing["apiKey"], "sk-real");
        assert_eq!(
            existing["settingsConfig"]["env"]["ANTHROPIC_AUTH_TOKEN"],
            "sk-real"
        );
        assert_eq!(existing["settingsConfig"]["env"]["ANTHROPIC_MODEL"], "new");
    }

    #[test]
    fn has_secret_value_ignores_plain_metadata() {
        assert!(!has_secret_value(&json!({"name": "x", "apiKeyEnv": "REF"})));
        assert!(has_secret_value(&json!({"apiKey": "sk-x"})));
        assert!(has_secret_value(&json!({"auth": {"API_KEY": "sk-x"}})));
    }

    #[test]
    fn json_string_redaction_keeps_the_tree_shape() {
        let value = redact_json_string(r#"{"env":{"GEMINI_API_KEY":"abc","MODEL":"flash"}}"#);
        assert_eq!(value["env"]["GEMINI_API_KEY"], REDACTED);
        assert_eq!(value["env"]["MODEL"], "flash");

        let raw_toml = redact_json_string("api_key = \"secret\"\nmodel = \"m\"");
        assert_eq!(raw_toml.as_str().unwrap().contains("secret"), false);
        assert!(raw_toml.as_str().unwrap().contains("model = \"m\""));
    }
}
