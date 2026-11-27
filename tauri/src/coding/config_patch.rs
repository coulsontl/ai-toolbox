//! Structured merge helpers for whole-file CLI config text (TOML / YAML).
//!
//! The control plane exposes the CLI "global config" as text for TOML/YAML
//! CLIs. Parsing and merging those formats is CLI config semantics, so it lives
//! here instead of in `control_api` (Hard Rule 15): every adapter that writes a
//! whole config file must reuse one structural merge and never re-implement
//! "patch vs replace" per CLI.
//!
//! JSON objects are different: their masked-patch contract belongs to the
//! control API response boundary (`merge_patch_preserving_secrets`), so it stays
//! there.

use serde_json::{Map, Value};

/// Placeholder emitted by the control API for a redacted secret. Writing it
/// back to a real file would replace a credential with three asterisks.
pub const MASK: &str = "***";

/// Accepts the shape the CLI sends for a config body and returns a JSON object.
///
/// A JSON string is the common case (`aitb config set --file config.json`).
/// YAML-backed CLIs also accept a YAML document because that is the file format
/// the user edits by hand.
pub fn coerce_object_input(
    incoming: &Value,
    allow_yaml: bool,
) -> Result<Map<String, Value>, String> {
    let incoming = match incoming {
        Value::String(text) => match serde_json::from_str::<Value>(text) {
            Ok(value) => value,
            Err(json_error) if allow_yaml => serde_yaml::from_str::<Value>(text).map_err(
                |yaml_error| {
                    format!(
                        "'config' string is neither valid JSON ({json_error}) nor valid YAML ({yaml_error})"
                    )
                },
            )?,
            Err(error) => return Err(format!("'config' string is not valid JSON: {error}")),
        },
        other => other.clone(),
    };
    match incoming {
        Value::Object(object) => Ok(object),
        _ => Err("'config' must be a JSON object".to_string()),
    }
}

/// Applies a TOML patch or whole-file replace to `existing`.
///
/// Patch mode merges tables recursively so unknown keys, key order and
/// comments survive. Arrays and scalars replace wholesale. A masked value is
/// rejected instead of being written into the file.
pub fn merge_toml_config_text(
    existing: &str,
    incoming: &Value,
    mode: &str,
) -> Result<String, String> {
    if let Value::String(text) = incoming {
        ensure_no_mask_in_text(text)?;
        if mode == "replace" {
            return Ok(text.clone());
        }
        let patch = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| format!("'config' string is not valid TOML: {error}"))?;
        let mut document = parse_existing_toml(existing)?;
        merge_toml_document(&mut document, &patch);
        return Ok(document.to_string());
    }
    if !incoming.is_object() {
        return Err("'config' must be a TOML string or an object".to_string());
    }
    if contains_mask(incoming) {
        return Err(
            "Masked secrets ('***') cannot be written back to a TOML file; send the full value or omit the key"
                .to_string(),
        );
    }
    let patch_text = toml::to_string(incoming)
        .map_err(|error| format!("Config cannot be represented as TOML: {error}"))?;
    if mode == "replace" {
        return Ok(patch_text);
    }
    let patch = patch_text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("Config cannot be represented as TOML: {error}"))?;
    let mut document = parse_existing_toml(existing)?;
    merge_toml_document(&mut document, &patch);
    Ok(document.to_string())
}

/// Applies a YAML patch or whole-file replace to `existing`.
///
/// Patch mode requires a mapping and merges nested mappings; a non-mapping
/// patch needs `mode=replace`. Masked values are rejected, and the document is
/// always parsed as YAML (never as TOML).
pub fn merge_yaml_config_text(
    existing: &str,
    incoming: &Value,
    mode: &str,
) -> Result<String, String> {
    let incoming_yaml = match incoming {
        Value::String(text) => {
            ensure_no_mask_in_text(text)?;
            if mode == "replace" {
                return Ok(text.clone());
            }
            serde_yaml::from_str::<serde_yaml::Value>(text)
                .map_err(|error| format!("'config' string is not valid YAML: {error}"))?
        }
        Value::Object(_) => {
            if contains_mask(incoming) {
                return Err(
                    "Masked secrets ('***') cannot be written back to a YAML file; send the full value or omit the key"
                        .to_string(),
                );
            }
            serde_yaml::to_value(incoming)
                .map_err(|error| format!("Config cannot be represented as YAML: {error}"))?
        }
        _ => return Err("'config' must be a YAML string or an object".to_string()),
    };
    if mode == "replace" {
        return serde_yaml::to_string(&incoming_yaml)
            .map_err(|error| format!("Config cannot be represented as YAML: {error}"));
    }
    if !incoming_yaml.is_mapping() {
        return Err(
            "A YAML patch must be a mapping; use mode=replace for a whole document".to_string(),
        );
    }
    let mut current = if existing.trim().is_empty() {
        serde_yaml::Value::Mapping(Default::default())
    } else {
        serde_yaml::from_str::<serde_yaml::Value>(existing)
            .map_err(|error| format!("Existing config is not valid YAML: {error}"))?
    };
    if !current.is_mapping() {
        current = serde_yaml::Value::Mapping(Default::default());
    }
    merge_yaml_value(&mut current, incoming_yaml);
    serde_yaml::to_string(&current)
        .map_err(|error| format!("Config cannot be represented as YAML: {error}"))
}

fn parse_existing_toml(existing: &str) -> Result<toml_edit::DocumentMut, String> {
    if existing.trim().is_empty() {
        return Ok(toml_edit::DocumentMut::new());
    }
    existing
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("Existing config is not valid TOML: {error}"))
}

/// Recursively merges `patch` into `document`, preserving unknown fields,
/// existing key order and comments. Tables and inline tables merge by key;
/// scalars and arrays replace wholesale.
fn merge_toml_document(document: &mut toml_edit::DocumentMut, patch: &toml_edit::DocumentMut) {
    let keys = patch
        .iter()
        .map(|(key, _)| key.to_string())
        .collect::<Vec<_>>();
    for key in keys {
        let Some(patch_item) = patch.get(key.as_str()) else {
            continue;
        };
        match document.get_mut(key.as_str()) {
            Some(existing) => merge_toml_item(existing, patch_item.clone()),
            None => {
                document[key.as_str()] = patch_item.clone();
            }
        }
    }
}

fn merge_toml_item(existing: &mut toml_edit::Item, patch: toml_edit::Item) {
    use toml_edit::Item;
    match (existing, patch) {
        (Item::Table(existing_table), Item::Table(patch_table)) => {
            let keys = patch_table
                .iter()
                .map(|(key, _)| key.to_string())
                .collect::<Vec<_>>();
            for key in keys {
                let Some(patch_item) = patch_table.get(key.as_str()) else {
                    continue;
                };
                match existing_table.get_mut(key.as_str()) {
                    Some(existing_item) => merge_toml_item(existing_item, patch_item.clone()),
                    None => {
                        existing_table[key.as_str()] = patch_item.clone();
                    }
                }
            }
        }
        (Item::Value(existing_value), Item::Value(patch_value)) => {
            merge_toml_value(existing_value, patch_value);
        }
        // `toml::to_string` serializes a nested JSON object as a `[table]`,
        // while the user's file may have written the same node as an inline
        // table (`http_headers = { ... }`). Replacing it wholesale would drop
        // every sibling header the patch did not mention.
        (Item::Value(existing_value), Item::Table(patch_table))
            if matches!(existing_value, toml_edit::Value::InlineTable(_)) =>
        {
            let toml_edit::Value::InlineTable(existing_inline) = existing_value else {
                unreachable!("guarded by matches!")
            };
            merge_inline_table_with_table(existing_inline, &patch_table);
        }
        (Item::Table(existing_table), Item::Value(toml_edit::Value::InlineTable(patch_inline))) => {
            for (key, patch_value) in patch_inline.iter() {
                match existing_table.get_mut(key) {
                    Some(existing_item) => {
                        merge_toml_item(existing_item, Item::Value(patch_value.clone()));
                    }
                    None => {
                        existing_table.insert(key, Item::Value(patch_value.clone()));
                    }
                }
            }
        }
        (existing, patch) => *existing = patch,
    }
}

fn merge_inline_table_with_table(existing: &mut toml_edit::InlineTable, patch: &toml_edit::Table) {
    for (key, patch_item) in patch.iter() {
        let patch_value = match patch_item {
            toml_edit::Item::Value(value) => value.clone(),
            toml_edit::Item::Table(table) => {
                toml_edit::Value::InlineTable(table.clone().into_inline_table())
            }
            toml_edit::Item::ArrayOfTables(tables) => {
                let mut array = toml_edit::Array::new();
                for table in tables.iter() {
                    array.push(toml_edit::Value::InlineTable(
                        table.clone().into_inline_table(),
                    ));
                }
                toml_edit::Value::Array(array)
            }
            toml_edit::Item::None => continue,
        };
        match existing.get_mut(key) {
            Some(existing_value) => merge_toml_value(existing_value, patch_value),
            None => {
                existing.insert(key, patch_value);
            }
        }
    }
}

fn merge_toml_value(existing: &mut toml_edit::Value, patch: toml_edit::Value) {
    use toml_edit::Value as TomlValue;
    match (existing, patch) {
        (TomlValue::InlineTable(existing_table), TomlValue::InlineTable(patch_table)) => {
            let keys = patch_table
                .iter()
                .map(|(key, _)| key.to_string())
                .collect::<Vec<_>>();
            for key in keys {
                let Some(patch_value) = patch_table.get(key.as_str()) else {
                    continue;
                };
                match existing_table.get_mut(key.as_str()) {
                    Some(existing_value) => merge_toml_value(existing_value, patch_value.clone()),
                    None => {
                        existing_table.insert(key.as_str(), patch_value.clone());
                    }
                }
            }
        }
        // Arrays and scalars cannot be deep-merged meaningfully.
        (existing, patch) => *existing = patch,
    }
}

fn merge_yaml_value(existing: &mut serde_yaml::Value, patch: serde_yaml::Value) {
    use serde_yaml::Value as YamlValue;
    match (existing, patch) {
        (YamlValue::Mapping(existing_map), YamlValue::Mapping(patch_map)) => {
            for (key, value) in patch_map {
                match existing_map.get_mut(&key) {
                    Some(existing_value) => merge_yaml_value(existing_value, value),
                    None => {
                        existing_map.insert(key, value);
                    }
                }
            }
        }
        (existing, patch) => *existing = patch,
    }
}

fn ensure_no_mask_in_text(text: &str) -> Result<(), String> {
    if text.contains(MASK) {
        return Err(
            "Masked secrets ('***') cannot be written back; send the full value or omit the key"
                .to_string(),
        );
    }
    Ok(())
}

fn contains_mask(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains(MASK),
        Value::Array(items) => items.iter().any(contains_mask),
        Value::Object(map) => map.values().any(contains_mask),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn toml_patch_merges_top_level_keys_and_rejects_masked_secrets() {
        let existing = "model = \"old\"\napproval_policy = \"never\"\n";
        let merged = merge_toml_config_text(
            existing,
            &json!({"model": "new", "sandbox_mode": "workspace-write"}),
            "patch",
        )
        .unwrap();
        assert!(merged.contains("model = \"new\""));
        assert!(merged.contains("approval_policy = \"never\""));
        assert!(merged.contains("sandbox_mode = \"workspace-write\""));

        let rejected = merge_toml_config_text(
            existing,
            &json!({"experimental_bearer_token": "***"}),
            "patch",
        );
        assert!(rejected.is_err());
    }

    #[test]
    fn raw_toml_replace_rejects_mask_placeholders() {
        assert!(
            merge_toml_config_text("model = \"old\"", &json!("api_key = \"***\""), "replace")
                .is_err()
        );
        let accepted =
            merge_toml_config_text("model = \"old\"", &json!("model = \"new\""), "replace")
                .unwrap();
        assert_eq!(accepted, "model = \"new\"");
    }

    #[test]
    fn toml_patch_preserves_unknown_nested_fields_and_inline_tables() {
        let existing = concat!(
            "# user comment stays\n",
            "model = \"old\"\n",
            "[mcp_servers.docs]\n",
            "command = \"node\"\n",
            "args = [\"server.js\"]\n",
            "http_headers = { \"X-Trace\" = \"keep\", \"X-Other\" = \"also\" }\n",
        );
        let merged = merge_toml_config_text(
            existing,
            &json!({
                "model": "new",
                "mcp_servers": {"docs": {"http_headers": {"X-Trace": "changed"}}}
            }),
            "patch",
        )
        .unwrap();
        assert!(merged.contains("# user comment stays"));
        assert!(merged.contains("model = \"new\""));
        // Unknown sibling keys and the untouched inline header survive.
        assert!(merged.contains("command = \"node\""));
        assert!(merged.contains("\"X-Other\" = \"also\""));
        assert!(merged.contains("\"X-Trace\" = \"changed\""));
    }

    #[test]
    fn toml_raw_patch_merges_instead_of_replacing_the_whole_file() {
        let existing = "model = \"old\"\napproval_policy = \"never\"\n";
        let merged =
            merge_toml_config_text(existing, &json!("sandbox_mode = \"read-only\""), "patch")
                .unwrap();
        assert!(merged.contains("model = \"old\""));
        assert!(merged.contains("approval_policy = \"never\""));
        assert!(merged.contains("sandbox_mode = \"read-only\""));
    }

    #[test]
    fn yaml_patch_preserves_unknown_fields_and_replace_is_whole_file() {
        let existing = concat!(
            "model: old\n",
            "providers:\n",
            "  acme:\n",
            "    base_url: https://acme.test\n",
            "    timeout: 30\n",
            "unknown_extra: keep\n",
        );
        let merged = merge_yaml_config_text(
            existing,
            &json!({"providers": {"acme": {"model": "new"}}}),
            "patch",
        )
        .unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&merged).unwrap();
        assert_eq!(parsed["model"], serde_yaml::Value::String("old".into()));
        assert_eq!(
            parsed["providers"]["acme"]["base_url"],
            serde_yaml::Value::String("https://acme.test".into())
        );
        assert_eq!(
            parsed["providers"]["acme"]["timeout"],
            serde_yaml::Value::Number(30.into())
        );
        assert_eq!(
            parsed["unknown_extra"],
            serde_yaml::Value::String("keep".into())
        );
        assert_eq!(
            parsed["providers"]["acme"]["model"],
            serde_yaml::Value::String("new".into())
        );

        let replaced =
            merge_yaml_config_text(existing, &json!("model: only\n"), "replace").unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&replaced).unwrap();
        assert_eq!(parsed["model"], serde_yaml::Value::String("only".into()));
        assert!(parsed.get("unknown_extra").is_none());
    }

    #[test]
    fn yaml_patch_rejects_mask_placeholders_in_object_and_text() {
        assert!(
            merge_yaml_config_text("model: old\n", &json!({"api_key": "***"}), "patch").is_err()
        );
        assert!(
            merge_yaml_config_text("model: old\n", &json!("api_key: ***\n"), "replace").is_err()
        );
    }

    #[test]
    fn yaml_backed_cli_accepts_a_yaml_string_but_json_only_cli_does_not() {
        let parsed = coerce_object_input(&json!("model: only\n"), true).unwrap();
        assert_eq!(parsed["model"], "only");

        let rejected = coerce_object_input(&json!("model: only\n"), false);
        assert!(rejected.is_err());
    }

    #[test]
    fn config_input_rejects_non_object_documents() {
        assert!(coerce_object_input(&json!("[1, 2]"), false).is_err());
        assert!(coerce_object_input(&json!(42), false).is_err());
    }
}
