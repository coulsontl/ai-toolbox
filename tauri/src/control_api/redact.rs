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
use serde_yaml::Value as YamlValue;
use toml_edit::{DocumentMut, Item as TomlItem, Table as TomlTable, Value as TomlValue};

pub const REDACTED: &str = "***";

/// Matches `key = value`, `key: value` and inline table entries for any key
/// that ends in a credential-shaped suffix (`*_api_key`, `*_token`,
/// `*_secret`, `*_password`). The key and separator are captured separately so
/// non-secret lookalikes (`hasApiKey`, `apiKeyEnv`) can be skipped.
static TOML_ENV_SECRET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?im)(["']?)([A-Za-z0-9_.-]*(?:api[_-]?key|access[_-]?token|refresh[_-]?token|auth[_-]?token|bearer[_-]?token|client[_-]?secret|password|secret|token))(["']?\s*[=:]\s*)(["']?)([^"'\r\n,}\]\s]+)(["']?)"#,
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
///
/// JSON, TOML and YAML are handled structurally first so keys stored in an
/// inline table (`http_headers = { "x-api-key" = "..." }`), a bare YAML
/// `token:` or a nested object are covered even without a regex-friendly
/// spelling. Plain text falls back to the assignment/bearer/URL regexes.
pub fn redact_text(text: &str) -> String {
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(mut value) = serde_json::from_str::<Value>(text) {
            redact_value(&mut value);
            return serde_json::to_string(&value).unwrap_or_else(|_| text.to_string());
        }
    }
    if let Some(redacted) = redact_toml_text(text) {
        return redacted;
    }
    // YAML parses almost any prose containing `key: value` as a one-line
    // mapping. Feeding an error message through it mangles the sentence and
    // can even hide a credential inside the quoted key. Only treat input as
    // YAML when every meaningful line starts with a config-shaped key.
    if looks_like_yaml_mapping(text) {
        if let Some(redacted) = redact_yaml_text(text) {
            return redacted;
        }
    }

    let text = TOML_ENV_SECRET_RE.replace_all(text, |caps: &regex::Captures<'_>| {
        let key = caps[2]
            .chars()
            .filter(|ch| !matches!(ch, '_' | '-' | '.' | ' '))
            .collect::<String>()
            .to_ascii_lowercase();
        if key == "hasapikey" || key.ends_with("apikeyenv") {
            return caps[0].to_string();
        }
        let value = &caps[5];
        if value.chars().all(|ch| ch.is_ascii_digit()) {
            return caps[0].to_string();
        }
        format!(
            "{}{}{}{}{}{}",
            &caps[1], &caps[2], &caps[3], &caps[4], REDACTED, &caps[6]
        )
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

/// Structured TOML redaction. Returns `None` when the text is not TOML or
/// contains no credential-shaped value (so untouched configs keep their exact
/// formatting and comments).
fn redact_toml_text(text: &str) -> Option<String> {
    let mut document = text.parse::<DocumentMut>().ok()?;
    if !redact_toml_table(document.as_table_mut()) {
        return None;
    }
    Some(document.to_string())
}

fn redact_toml_table(table: &mut TomlTable) -> bool {
    let keys = table.iter().map(|(key, _)| key.to_string()).collect::<Vec<_>>();
    let mut changed = false;
    for key in keys {
        let Some(item) = table.get_mut(&key) else {
            continue;
        };
        changed |= redact_toml_item(&key, item);
    }
    changed
}

fn redact_toml_item(key: &str, item: &mut TomlItem) -> bool {
    if is_secret_key(key, &Value::Null) {
        *item = TomlItem::Value(TomlValue::from(REDACTED));
        return true;
    }
    match item {
        TomlItem::Table(table) => redact_toml_table(table),
        TomlItem::ArrayOfTables(tables) => {
            let mut changed = false;
            for table in tables.iter_mut() {
                changed |= redact_toml_table(table);
            }
            changed
        }
        TomlItem::Value(value) => redact_toml_value(key, value),
        TomlItem::None => false,
    }
}

fn redact_toml_value(key: &str, value: &mut TomlValue) -> bool {
    match value {
        TomlValue::String(text) => {
            if is_secret_key(key, &Value::String(text.value().to_string())) {
                *value = TomlValue::from(REDACTED);
                true
            } else {
                false
            }
        }
        TomlValue::InlineTable(table) => {
            let keys = table.iter().map(|(key, _)| key.to_string()).collect::<Vec<_>>();
            let mut changed = false;
            for key in keys {
                if let Some(value) = table.get_mut(&key) {
                    changed |= redact_toml_value(&key, value);
                }
            }
            changed
        }
        TomlValue::Array(array) => {
            let mut changed = false;
            for value in array.iter_mut() {
                changed |= redact_toml_value("", value);
            }
            changed
        }
        _ => false,
    }
}

/// Whether the text plausibly is a YAML mapping/sequence document rather than
/// prose that happens to contain a colon.
fn looks_like_yaml_mapping(text: &str) -> bool {
    let mut saw_key = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
            continue;
        }
        let Some((key, _)) = trimmed.split_once(':') else {
            return false;
        };
        let key = key.trim().trim_matches('"').trim_matches('\'');
        if key.is_empty()
            || !key
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/'))
        {
            return false;
        }
        saw_key = true;
    }
    saw_key
}

/// Structured YAML redaction; same no-change shortcut as TOML.
fn redact_yaml_text(text: &str) -> Option<String> {
    let mut value = serde_yaml::from_str::<YamlValue>(text).ok()?;
    if !matches!(value, YamlValue::Mapping(_) | YamlValue::Sequence(_)) {
        return None;
    }
    if !redact_yaml_value(&mut value) {
        return None;
    }
    serde_yaml::to_string(&value).ok()
}

fn redact_yaml_value(value: &mut YamlValue) -> bool {
    match value {
        YamlValue::Mapping(mapping) => {
            let keys = mapping.keys().cloned().collect::<Vec<_>>();
            let mut changed = false;
            for key in keys {
                let Some(current) = mapping.get_mut(&key) else {
                    continue;
                };
                let key_name = key.as_str().unwrap_or_default();
                let secret_value = matches!(current, YamlValue::String(_))
                    && is_secret_key(key_name, &yaml_scalar_as_json(current));
                if secret_value {
                    *current = YamlValue::String(REDACTED.to_string());
                    changed = true;
                } else {
                    changed |= redact_yaml_value(current);
                }
            }
            changed
        }
        YamlValue::Sequence(items) => {
            let mut changed = false;
            for item in items.iter_mut() {
                changed |= redact_yaml_value(item);
            }
            changed
        }
        _ => false,
    }
}

fn yaml_scalar_as_json(value: &YamlValue) -> Value {
    match value {
        YamlValue::String(text) => Value::String(text.clone()),
        YamlValue::Bool(flag) => Value::Bool(*flag),
        YamlValue::Number(number) => {
            serde_json::to_value(number).unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

/// True when any string inside the tree is a redaction placeholder.
pub fn contains_redacted(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains(REDACTED),
        Value::Array(items) => items.iter().any(contains_redacted),
        Value::Object(map) => map.values().any(contains_redacted),
        _ => false,
    }
}

/// Removes redaction placeholders from a patch tree.
///
/// A masked scalar means "keep the stored value", and a masked array element
/// has no stored counterpart to point at, so both are dropped instead of being
/// written through. Objects are rebuilt without their masked members so the
/// remaining sibling fields still apply.
fn drop_redacted(value: Value) -> Option<Value> {
    match value {
        Value::String(text) if text.contains(REDACTED) => None,
        Value::Array(items) => Some(Value::Array(
            items.into_iter().filter_map(drop_redacted).collect(),
        )),
        Value::Object(map) => Some(Value::Object(
            map.into_iter()
                .filter_map(|(key, value)| drop_redacted(value).map(|value| (key, value)))
                .collect(),
        )),
        other => Some(other),
    }
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
                // `null` on a secret key means "keep the stored credential";
                // `***` anywhere means the caller only echoed a masked read.
                // Neither may reach a real config, including brand-new keys.
                let keep_stored_secret = patch_value.is_null()
                    && existing_map
                        .get(&key)
                        .map(|value| is_secret_key(&key, value))
                        .unwrap_or(false);
                if keep_stored_secret {
                    continue;
                }
                let Some(patch_value) = drop_redacted(patch_value) else {
                    continue;
                };
                if patch_value.is_object() {
                    if let Some(existing_value) = existing_map.get_mut(&key) {
                        merge_patch_preserving_secrets(existing_value, patch_value);
                        continue;
                    }
                }
                existing_map.insert(key, patch_value);
            }
        }
        (existing_slot, patch_value) => {
            if let Some(patch_value) = drop_redacted(patch_value) {
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

    #[test]
    fn toml_redaction_is_structural_for_tables_inline_tables_and_arrays() {
        let text = concat!(
            "model = \"gpt-5\"\n",
            "api_key = \"sk-top\"\n",
            "[http_headers]\n",
            "\"x-api-key\" = \"sk-header\"\n",
            "authorization = \"Bearer sk-bearer\"\n",
            "[providers.acme]\n",
            "password = \"hunter2\"\n",
            "inline = { token = \"sk-inline\", model = \"keep-me\" }\n",
            "[[accounts]]\n",
            "client_secret = \"sk-array\"\n",
            "name = \"primary\"\n",
        );
        let redacted = redact_text(text);
        for secret in [
            "sk-top",
            "sk-header",
            "sk-bearer",
            "hunter2",
            "sk-inline",
            "sk-array",
        ] {
            assert!(
                !redacted.contains(secret),
                "'{secret}' leaked in:\n{redacted}"
            );
        }
        assert!(redacted.contains("model = \"gpt-5\""));
        assert!(redacted.contains("model = \"keep-me\""));
        assert!(redacted.contains("name = \"primary\""));
    }

    #[test]
    fn yaml_redaction_handles_bare_keys_and_nested_mappings() {
        let text = concat!(
            "model: gpt-5\n",
            "token: sk-bare\n",
            "providers:\n",
            "  acme:\n",
            "    api_key: sk-nested\n",
            "    headers:\n",
            "      X-Custom-Secret: sk-header\n",
            "    name: keep\n",
        );
        let redacted = redact_text(text);
        for secret in ["sk-bare", "sk-nested", "sk-header"] {
            assert!(
                !redacted.contains(secret),
                "'{secret}' leaked in:\n{redacted}"
            );
        }
        assert!(redacted.contains("model: gpt-5"));
        assert!(redacted.contains("name: keep"));
    }

    #[test]
    fn arbitrary_secret_suffixes_are_redacted_in_plain_text() {
        let text = "OPENROUTER_API_KEY=sk-or\nmy_service_token: tok-1\nDB_PASSWORD=pw\nbackup_secret=sec-987";
        let redacted = redact_text(text);
        for secret in ["sk-or", "tok-1", "pw", "sec-987"] {
            assert!(!redacted.contains(secret), "'{secret}' leaked: {redacted}");
        }
    }

    #[test]
    fn masked_values_are_detected_recursively() {
        assert!(contains_redacted(&json!({"a": {"b": "***"}})));
        assert!(contains_redacted(&json!([{"a": "***"}])));
        assert!(!contains_redacted(&json!({"a": "value"})));
    }

    #[test]
    fn config_patch_rejects_the_same_mask_marker() {
        assert_eq!(crate::coding::config_patch::MASK, REDACTED);
    }
}
