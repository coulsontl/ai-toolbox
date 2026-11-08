//! Host-local config fields.
//!
//! Some CLI config keys describe the machine that runs the CLI rather than the
//! user's shared profile:
//!
//! * Claude / ZCode / Gemini / CodeBuddy hooks contain executable commands.
//! * Claude `statusLine` may run a local script.
//! * Codex `notify` points at a local notification program, `hooks` holds
//!   hook definitions plus per-machine state, and `projects` is a map keyed by
//!   absolute project paths.
//!
//! A whole-file sync or backup restore must not copy these values across
//! machines. This module owns the registry and the merge rules so SSH sync,
//! WSL sync, and backup restore all behave the same way.
//!
//! The rule is deliberately simple: the **target** keeps its own host-local
//! values. When the source is the same platform class as the target, the source
//! value is used only if the target has none (`fill_missing`); otherwise the
//! source values are dropped.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlatformClass {
    Windows,
    Unix,
}

impl PlatformClass {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Json,
    Toml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostLocalFieldKind {
    /// JSON object keyed by hook event; each event is an array of matcher
    /// groups whose `hooks` array holds command entries.
    JsonHooks,
    /// JSON status-line object whose `command` may point at a local script.
    JsonStatusLine,
    /// TOML array whose first element is the notification executable.
    CodexNotify,
    /// TOML table keyed by absolute project paths.
    CodexProjects,
    /// TOML table holding Codex hook definitions and per-hook state. Keys and
    /// commands that only work on Windows are dropped on Unix targets.
    CodexHooks,
}

#[derive(Debug, Clone, Copy)]
pub struct HostLocalField {
    pub key: &'static str,
    pub kind: HostLocalFieldKind,
}

#[derive(Debug, Clone, Copy)]
pub struct HostLocalSpec {
    pub format: ConfigFormat,
    pub fields: &'static [HostLocalField],
}

#[derive(Debug, Clone)]
pub enum HostLocalValue {
    Json(Value),
    Toml(toml_edit::Item),
}

pub static CLAUDE_SETTINGS_SPEC: HostLocalSpec = HostLocalSpec {
    format: ConfigFormat::Json,
    fields: &[
        HostLocalField {
            key: "hooks",
            kind: HostLocalFieldKind::JsonHooks,
        },
        HostLocalField {
            key: "statusLine",
            kind: HostLocalFieldKind::JsonStatusLine,
        },
    ],
};

pub static CODEX_CONFIG_SPEC: HostLocalSpec = HostLocalSpec {
    format: ConfigFormat::Toml,
    fields: &[
        HostLocalField {
            key: "notify",
            kind: HostLocalFieldKind::CodexNotify,
        },
        HostLocalField {
            key: "projects",
            kind: HostLocalFieldKind::CodexProjects,
        },
        HostLocalField {
            key: "hooks",
            kind: HostLocalFieldKind::CodexHooks,
        },
    ],
};

/// Codex stores hook definitions in a separate `~/.codex/hooks.json`; only the
/// per-hook state lives in `config.toml`.
pub static CODEX_HOOKS_JSON_SPEC: HostLocalSpec = HostLocalSpec {
    format: ConfigFormat::Json,
    fields: &[HostLocalField {
        key: "hooks",
        kind: HostLocalFieldKind::JsonHooks,
    }],
};

pub static ZCODE_CLI_CONFIG_SPEC: HostLocalSpec = HostLocalSpec {
    format: ConfigFormat::Json,
    fields: &[HostLocalField {
        key: "hooks",
        kind: HostLocalFieldKind::JsonHooks,
    }],
};

pub static GEMINI_SETTINGS_SPEC: HostLocalSpec = HostLocalSpec {
    format: ConfigFormat::Json,
    fields: &[HostLocalField {
        key: "hooks",
        kind: HostLocalFieldKind::JsonHooks,
    }],
};

pub static CODEBUDDY_SETTINGS_SPEC: HostLocalSpec = HostLocalSpec {
    format: ConfigFormat::Json,
    fields: &[HostLocalField {
        key: "hooks",
        kind: HostLocalFieldKind::JsonHooks,
    }],
};

/// Resolve the host-local spec for one SSH/WSL file mapping.
///
/// Directory and pattern mappings are ignored: they do not have a single
/// config document to merge. `mapping_id` is used for ZCode because
/// `config.json` alone is too generic.
pub fn spec_for_sync_mapping(
    module: &str,
    mapping_id: &str,
    is_directory: bool,
    is_pattern: bool,
    source_path: &str,
    target_path: &str,
) -> Option<&'static HostLocalSpec> {
    if is_directory || is_pattern {
        return None;
    }

    let source_name = path_basename(source_path);
    let target_name = path_basename(target_path);
    match module {
        "claude"
            if matches!(source_name, "settings.json" | "settings.local.json")
                || matches!(target_name, "settings.json" | "settings.local.json") =>
        {
            Some(&CLAUDE_SETTINGS_SPEC)
        }
        "codex" if source_name == "hooks.json" || target_name == "hooks.json" => {
            Some(&CODEX_HOOKS_JSON_SPEC)
        }
        "codex" if source_name == "config.toml" || target_name == "config.toml" => {
            Some(&CODEX_CONFIG_SPEC)
        }
        "geminicli" if source_name == "settings.json" || target_name == "settings.json" => {
            Some(&GEMINI_SETTINGS_SPEC)
        }
        "codebuddy" if source_name == "settings.json" || target_name == "settings.json" => {
            Some(&CODEBUDDY_SETTINGS_SPEC)
        }
        "zcode"
            if mapping_id == "zcode-cli-config"
                || path_ends_with_segments(source_path, &["cli", "config.json"])
                || path_ends_with_segments(target_path, &["cli", "config.json"]) =>
        {
            Some(&ZCODE_CLI_CONFIG_SPEC)
        }
        _ => None,
    }
}

/// Resolve the host-local spec for one backup `external-configs/<tool>/...`
/// entry.
pub fn spec_for_backup_entry(tool: &str, relative_path: &str) -> Option<&'static HostLocalSpec> {
    let normalized = relative_path.replace('\\', "/");
    let normalized = normalized.trim_start_matches('/');
    match (tool, normalized) {
        ("claude", "settings.json" | "settings.local.json") => Some(&CLAUDE_SETTINGS_SPEC),
        ("codex", "config.toml") => Some(&CODEX_CONFIG_SPEC),
        ("codex", "hooks.json") => Some(&CODEX_HOOKS_JSON_SPEC),
        ("geminicli", "settings.json") => Some(&GEMINI_SETTINGS_SPEC),
        ("codebuddy", "settings.json") => Some(&CODEBUDDY_SETTINGS_SPEC),
        ("zcode", "cli/config.json") => Some(&ZCODE_CLI_CONFIG_SPEC),
        _ => None,
    }
}

/// Extract target-owned values. An empty file is treated as "no values"; a
/// non-empty file that cannot be parsed is an error so callers can skip the
/// operation instead of clobbering the target.
pub fn extract_target_values(
    content: &str,
    spec: &HostLocalSpec,
    target_platform: PlatformClass,
) -> Result<Vec<Option<HostLocalValue>>, String> {
    let mut values = extract_values(content, spec)?;
    sanitize_values(&mut values, spec, target_platform);
    Ok(values)
}

/// Extract values without sanitizing. Exposed for tests and callers that want
/// to inspect the raw values.
pub fn extract_values(
    content: &str,
    spec: &HostLocalSpec,
) -> Result<Vec<Option<HostLocalValue>>, String> {
    if content.trim().is_empty() {
        return Ok(vec![None; spec.fields.len()]);
    }

    match spec.format {
        ConfigFormat::Json => {
            let mut document: Value =
                serde_json::from_str(content).map_err(|error| format!("JSON 解析失败: {error}"))?;
            let object = document
                .as_object_mut()
                .ok_or_else(|| "JSON 根节点不是对象".to_string())?;
            Ok(spec
                .fields
                .iter()
                .map(|field| {
                    object
                        .remove(field.key)
                        .filter(|value| !value.is_null())
                        .map(HostLocalValue::Json)
                })
                .collect())
        }
        ConfigFormat::Toml => {
            let mut document = content
                .parse::<toml_edit::DocumentMut>()
                .map_err(|error| format!("TOML 解析失败: {error}"))?;
            Ok(spec
                .fields
                .iter()
                .map(|field| {
                    document
                        .as_table_mut()
                        .remove(field.key)
                        .filter(|item| !item.is_none())
                        .map(HostLocalValue::Toml)
                })
                .collect())
        }
    }
}

/// Remove values that cannot work on `target_platform`.
pub fn sanitize_values(
    values: &mut [Option<HostLocalValue>],
    spec: &HostLocalSpec,
    target_platform: PlatformClass,
) {
    for (index, field) in spec.fields.iter().enumerate() {
        let Some(value) = values.get_mut(index) else {
            continue;
        };
        let sanitized = match value.take() {
            Some(HostLocalValue::Json(json_value)) => {
                sanitize_json_value(field.kind, json_value, target_platform)
                    .map(HostLocalValue::Json)
            }
            Some(HostLocalValue::Toml(item)) => {
                sanitize_toml_value(field.kind, item, target_platform).map(HostLocalValue::Toml)
            }
            None => None,
        };
        *value = sanitized;
    }
}

/// Merge target-owned values into `source_content`.
///
/// * A target value always wins.
/// * With `fill_missing`, a missing target value keeps the source value (after
///   platform sanitization); otherwise the source value is removed.
///
/// Returns `Ok(None)` when the source document is already in the desired state.
pub fn apply_values(
    source_content: &str,
    spec: &HostLocalSpec,
    target_values: &[Option<HostLocalValue>],
    fill_missing: bool,
    target_platform: PlatformClass,
) -> Result<Option<String>, String> {
    match spec.format {
        ConfigFormat::Json => apply_json_values(
            source_content,
            spec,
            target_values,
            fill_missing,
            target_platform,
        ),
        ConfigFormat::Toml => apply_toml_values(
            source_content,
            spec,
            target_values,
            fill_missing,
            target_platform,
        ),
    }
}

fn apply_json_values(
    source_content: &str,
    spec: &HostLocalSpec,
    target_values: &[Option<HostLocalValue>],
    fill_missing: bool,
    target_platform: PlatformClass,
) -> Result<Option<String>, String> {
    let mut document: Value = if source_content.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(source_content)
            .map_err(|error| format!("JSON 解析失败: {error}"))?
    };
    let object = document
        .as_object_mut()
        .ok_or_else(|| "JSON 根节点不是对象".to_string())?;
    let mut changed = false;

    for (index, field) in spec.fields.iter().enumerate() {
        match target_values.get(index).and_then(|value| value.as_ref()) {
            Some(HostLocalValue::Json(target_value)) => {
                if object.get(field.key) != Some(target_value) {
                    object.insert(field.key.to_string(), target_value.clone());
                    changed = true;
                }
            }
            _ if fill_missing => {
                let Some(existing) = object.get(field.key).cloned() else {
                    continue;
                };
                match sanitize_json_value(field.kind, existing.clone(), target_platform) {
                    Some(sanitized) if sanitized != existing => {
                        object.insert(field.key.to_string(), sanitized);
                        changed = true;
                    }
                    Some(_) => {}
                    None => {
                        object.remove(field.key);
                        changed = true;
                    }
                }
            }
            _ => {
                if object.remove(field.key).is_some() {
                    changed = true;
                }
            }
        }
    }

    if !changed {
        return Ok(None);
    }
    let serialized = serde_json::to_string_pretty(&document)
        .map_err(|error| format!("JSON 序列化失败: {error}"))?;
    Ok(Some(format!("{serialized}\n")))
}

fn apply_toml_values(
    source_content: &str,
    spec: &HostLocalSpec,
    target_values: &[Option<HostLocalValue>],
    fill_missing: bool,
    target_platform: PlatformClass,
) -> Result<Option<String>, String> {
    let mut document = if source_content.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        source_content
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| format!("TOML 解析失败: {error}"))?
    };
    let mut changed = false;

    for (index, field) in spec.fields.iter().enumerate() {
        match target_values.get(index).and_then(|value| value.as_ref()) {
            Some(HostLocalValue::Toml(target_value)) => {
                let same = document.get(field.key).is_some_and(|current| {
                    render_toml_field(field.key, current)
                        == render_toml_field(field.key, target_value)
                });
                if !same {
                    document
                        .as_table_mut()
                        .insert(field.key, target_value.clone());
                    changed = true;
                }
            }
            _ if fill_missing => {
                let Some(existing) = document.get(field.key).cloned() else {
                    continue;
                };
                match sanitize_toml_value(field.kind, existing.clone(), target_platform) {
                    Some(sanitized)
                        if render_toml_field(field.key, &sanitized)
                            != render_toml_field(field.key, &existing) =>
                    {
                        document.as_table_mut().insert(field.key, sanitized);
                        changed = true;
                    }
                    Some(_) => {}
                    None => {
                        document.as_table_mut().remove(field.key);
                        changed = true;
                    }
                }
            }
            _ => {
                if document.as_table_mut().remove(field.key).is_some() {
                    changed = true;
                }
            }
        }
    }

    if !changed {
        return Ok(None);
    }
    Ok(Some(document.to_string()))
}

fn render_toml_field(key: &str, item: &toml_edit::Item) -> String {
    let mut document = toml_edit::DocumentMut::new();
    document.as_table_mut().insert(key, item.clone());
    // Decor/position differs between a freshly built empty table and one
    // parsed from the merged file; leading/trailing whitespace is not a value
    // change, so ignore it when deciding whether a write is needed.
    document.to_string().trim().to_string()
}

fn sanitize_json_value(
    kind: HostLocalFieldKind,
    value: Value,
    target_platform: PlatformClass,
) -> Option<Value> {
    if target_platform != PlatformClass::Unix {
        return Some(value);
    }
    match kind {
        HostLocalFieldKind::JsonHooks => sanitize_json_hooks(value),
        HostLocalFieldKind::JsonStatusLine => sanitize_json_status_line(value),
        _ => Some(value),
    }
}

fn sanitize_toml_value(
    kind: HostLocalFieldKind,
    item: toml_edit::Item,
    target_platform: PlatformClass,
) -> Option<toml_edit::Item> {
    if target_platform != PlatformClass::Unix {
        return Some(item);
    }
    match kind {
        HostLocalFieldKind::CodexNotify => sanitize_codex_notify(item),
        HostLocalFieldKind::CodexProjects => sanitize_codex_projects(item),
        HostLocalFieldKind::CodexHooks => sanitize_codex_hooks(item),
        _ => Some(item),
    }
}

fn sanitize_json_hooks(value: Value) -> Option<Value> {
    let Value::Object(mut events) = value else {
        return Some(value);
    };

    let event_names: Vec<String> = events.keys().cloned().collect();
    for event_name in event_names {
        let Some(event_value) = events.remove(&event_name) else {
            continue;
        };
        let Value::Array(groups) = event_value else {
            // Unexpected shape: keep it untouched rather than guessing.
            events.insert(event_name, event_value);
            continue;
        };

        let mut kept_groups = Vec::with_capacity(groups.len());
        for group in groups {
            if let Some(group) = sanitize_hook_group(group) {
                kept_groups.push(group);
            }
        }
        if !kept_groups.is_empty() {
            events.insert(event_name, Value::Array(kept_groups));
        }
    }

    // Preserve an explicitly empty object: target ownership includes the
    // decision to disable hooks, so it must not be replaced by source values.
    Some(Value::Object(events))
}

fn sanitize_hook_group(group: Value) -> Option<Value> {
    let Value::Object(mut object) = group else {
        return Some(group);
    };

    if let Some(command) = object.get("command").and_then(Value::as_str) {
        if is_windows_only_command(command) {
            return None;
        }
    }

    let Some(hooks_value) = object.remove("hooks") else {
        return Some(Value::Object(object));
    };
    let Value::Array(hooks) = hooks_value else {
        object.insert("hooks".to_string(), hooks_value);
        return Some(Value::Object(object));
    };

    let mut kept_hooks = Vec::with_capacity(hooks.len());
    for hook in hooks {
        if let Some(hook) = sanitize_hook_entry(hook) {
            kept_hooks.push(hook);
        }
    }
    if kept_hooks.is_empty() {
        return None;
    }
    object.insert("hooks".to_string(), Value::Array(kept_hooks));
    Some(Value::Object(object))
}

fn sanitize_hook_entry(entry: Value) -> Option<Value> {
    let Value::Object(object) = &entry else {
        return Some(entry);
    };
    if let Some(command) = object.get("command").and_then(Value::as_str) {
        if is_windows_only_command(command) {
            return None;
        }
    }
    Some(entry)
}

fn sanitize_json_status_line(value: Value) -> Option<Value> {
    match &value {
        Value::Object(object) => {
            if let Some(command) = object.get("command").and_then(Value::as_str) {
                if is_windows_only_command(command) {
                    return None;
                }
            }
            Some(value)
        }
        Value::String(command) if is_windows_only_command(command) => None,
        _ => Some(value),
    }
}

fn sanitize_codex_hooks(item: toml_edit::Item) -> Option<toml_edit::Item> {
    let mut item = item;
    let Some(table) = item.as_table_like_mut() else {
        return Some(item);
    };
    sanitize_toml_table_paths(table);
    if table.is_empty() {
        // The original may have been implicit (created by `[hooks.state]`);
        // rebuild it explicitly so an empty target value still serializes.
        return Some(toml_edit::Item::Table(toml_edit::Table::new()));
    }
    // An empty table is still an explicit target value; keep it so
    // `fill_missing` cannot re-import the source's hooks.
    Some(item)
}

/// Recursively drop keys and `command` values that only work on Windows.
/// Empty child tables are removed so an event with no hooks does not survive
/// as a meaningless stub.
fn sanitize_toml_table_paths(table: &mut dyn toml_edit::TableLike) -> bool {
    let mut changed = false;

    let path_keys: Vec<String> = table
        .iter()
        .filter(|(key, _)| is_windows_absolute_path(key) || contains_windows_drive_path(key))
        .map(|(key, _)| key.to_string())
        .collect();
    for key in path_keys {
        table.remove(&key);
        changed = true;
    }

    let command_keys: Vec<String> = table
        .iter()
        .filter(|(key, item)| {
            if *key != "command" {
                return false;
            }
            item.as_str().is_some_and(is_windows_only_command)
                || item.as_array().is_some_and(|array| {
                    array
                        .iter()
                        .any(|value| value.as_str().is_some_and(is_windows_only_command))
                })
        })
        .map(|(key, _)| key.to_string())
        .collect();
    for key in command_keys {
        table.remove(&key);
        changed = true;
    }

    let mut empty_child_keys = Vec::new();
    for (key, item) in table.iter_mut() {
        let Some(child) = item.as_table_like_mut() else {
            continue;
        };
        if sanitize_toml_table_paths(child) {
            changed = true;
        }
        if child.is_empty() {
            empty_child_keys.push(key.to_string());
        }
    }
    for key in empty_child_keys {
        table.remove(&key);
        changed = true;
    }

    changed
}

fn sanitize_codex_notify(item: toml_edit::Item) -> Option<toml_edit::Item> {
    let Some(array) = item.as_array() else {
        return Some(item);
    };
    let windows_only = array.iter().any(|value| {
        value
            .as_str()
            .is_some_and(|text| is_windows_only_command(text))
    });
    if windows_only {
        // Keep an explicit empty array instead of dropping the key: the target
        // chose its notify value (even if unusable here), and `fill_missing`
        // must not replace it with the source's value.
        Some(toml_edit::value(toml_edit::Array::new()))
    } else {
        Some(item)
    }
}

fn sanitize_codex_projects(item: toml_edit::Item) -> Option<toml_edit::Item> {
    let mut item = item;
    let Some(table) = item.as_table_like_mut() else {
        return Some(item);
    };

    let windows_keys: Vec<String> = table
        .iter()
        .filter(|(key, _)| is_windows_absolute_path(key))
        .map(|(key, _)| key.to_string())
        .collect();
    for key in windows_keys {
        table.remove(&key);
    }

    if table.is_empty() {
        // Same as hooks: an implicit table would disappear from the output.
        return Some(toml_edit::Item::Table(toml_edit::Table::new()));
    }
    Some(item)
}

/// True when `value` is a Windows drive-letter or UNC absolute path.
pub fn is_windows_absolute_path(value: &str) -> bool {
    let value = value.trim();
    if value.len() >= 2 && value.as_bytes()[0] == b'\\' && value.as_bytes()[1] == b'\\' {
        return true;
    }
    contains_windows_drive_path(value)
}

/// True when the command cannot run on a Unix target without translation.
pub fn is_windows_only_command(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }

    if contains_windows_drive_path(command) || contains_unc_path(command) {
        return true;
    }
    if contains_percent_env_var(command) || command.contains("$env:") {
        return true;
    }

    let lower = command.to_ascii_lowercase();
    if lower.contains("cmd /c") || lower.contains("cmd.exe /c") {
        return true;
    }

    let first_token = first_command_token(command).to_ascii_lowercase();
    matches!(
        first_token.as_str(),
        "cmd" | "cmd.exe" | "powershell" | "powershell.exe" | "pwsh.exe"
    ) || [".exe", ".bat", ".cmd"]
        .iter()
        .any(|suffix| first_token.ends_with(suffix))
}

fn contains_unc_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index + 2 < bytes.len() {
        if bytes[index] == b'\\' && bytes[index + 1] == b'\\' {
            // A real UNC path has a second separator after the server/share
            // name. This avoids treating shell escapes such as `\\n` as paths.
            if bytes[index + 2..].iter().any(|byte| *byte == b'\\') {
                return true;
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    false
}

fn contains_windows_drive_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    for index in 0..bytes.len().saturating_sub(2) {
        if !bytes[index].is_ascii_alphabetic() || bytes[index + 1] != b':' {
            continue;
        }
        if bytes[index + 2] != b'\\' && bytes[index + 2] != b'/' {
            continue;
        }
        let boundary = index == 0
            || !(bytes[index - 1].is_ascii_alphanumeric()
                || bytes[index - 1] == b'_'
                || bytes[index - 1] == b'-');
        if boundary {
            return true;
        }
    }
    false
}

fn contains_percent_env_var(value: &str) -> bool {
    let bytes = value.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'%' {
            continue;
        }
        let Some(rest) = value.get(index + 1..) else {
            continue;
        };
        let Some(end) = rest.find('%') else {
            continue;
        };
        let name = &rest[..end];
        if is_windows_env_name(name) {
            return true;
        }
    }
    false
}

/// Distinguish `%USERPROFILE%`/`%APPDATA%` from date formats such as
/// `date +%Y%m%d`. A Windows env name is at least two characters and normally
/// uppercase; known lowercase cmd.exe names are accepted explicitly.
fn is_windows_env_name(name: &str) -> bool {
    if name.len() < 2
        || !name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'(' || byte == b')'
        })
    {
        return false;
    }
    if name.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return true;
    }
    matches!(
        name.to_ascii_lowercase().as_str(),
        "cd"
            | "date"
            | "time"
            | "random"
            | "username"
            | "computername"
            | "userdomain"
            | "errorlevel"
            | "temp"
            | "tmp"
    )
}

fn first_command_token(command: &str) -> &str {
    command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches('"')
        .trim_matches('\'')
}

fn path_basename(path: &str) -> &str {
    path.trim()
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
}

fn path_ends_with_segments(path: &str, segments: &[&str]) -> bool {
    let normalized = path.trim().replace('\\', "/");
    let normalized = normalized.trim_matches('/');
    let parts: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() < segments.len() {
        return false;
    }
    parts[parts.len() - segments.len()..]
        .iter()
        .zip(segments)
        .all(|(left, right)| left == right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_specs_for_builtin_and_custom_mapping_paths() {
        assert!(spec_for_sync_mapping(
            "claude",
            "custom-claude",
            false,
            false,
            "~/.claude/settings.local.json",
            "~/claude-settings.json"
        )
        .is_some());
        assert!(spec_for_sync_mapping(
            "codex",
            "custom-codex",
            false,
            false,
            "/home/u/.codex/config.toml",
            "/home/u/.codex/remote-config.toml"
        )
        .is_some());
        assert!(spec_for_sync_mapping(
            "zcode",
            "custom-zcode",
            false,
            false,
            "~/.zcode/cli/config.json",
            "~/zcode-config.json"
        )
        .is_some());
        assert!(spec_for_sync_mapping(
            "zcode",
            "zcode-cli-config",
            false,
            false,
            "~/whatever.json",
            "~/zcode-config.json"
        )
        .is_some());
        assert!(spec_for_sync_mapping(
            "claude",
            "claude-plugins",
            true,
            false,
            "~/.claude/plugins",
            "~/.claude/plugins"
        )
        .is_none());
        assert!(spec_for_sync_mapping(
            "opencode",
            "opencode-config",
            false,
            false,
            "~/.config/opencode/opencode.json",
            "~/.config/opencode/opencode.json"
        )
        .is_none());
    }

    #[test]
    fn codex_hooks_json_is_host_local() {
        let spec = spec_for_sync_mapping(
            "codex",
            "custom-codex-hooks",
            false,
            false,
            "~/.codex/hooks.json",
            "~/.codex/hooks.json",
        )
        .expect("hooks.json should use the host-local spec");
        assert_eq!(spec.format, ConfigFormat::Json);

        let target = r#"{"hooks":{"Stop":[{"hooks":[{"command":"/target/hook.sh"}]}]}}"#;
        let target_values = extract_target_values(target, spec, PlatformClass::Unix).unwrap();
        let source = r#"{"hooks":{"Stop":[{"hooks":[{"command":"/source/hook.sh"}]}]}}"#;

        let merged = apply_values(source, spec, &target_values, false, PlatformClass::Unix)
            .unwrap()
            .expect("changed");

        assert!(merged.contains("/target/hook.sh"));
        assert!(!merged.contains("/source/hook.sh"));
    }

    #[test]
    fn codex_toml_merge_is_idempotent() {
        let target = r#"
notify = ["/target/notify"]
[projects."/target/project"]
trust_level = "trusted"
[hooks.state]
"/target/hooks.json:stop:0:0" = true
"#;
        let target_values =
            extract_target_values(target, &CODEX_CONFIG_SPEC, PlatformClass::Unix).unwrap();
        let source = r#"
model = "gpt-5"
notify = ["/source/notify"]
[projects."/source/project"]
trust_level = "trusted"
[hooks.state]
"/source/hooks.json:stop:0:0" = true
"#;

        let first = apply_values(
            source,
            &CODEX_CONFIG_SPEC,
            &target_values,
            false,
            PlatformClass::Unix,
        )
        .unwrap()
        .expect("changed");
        let second = apply_values(
            &first,
            &CODEX_CONFIG_SPEC,
            &target_values,
            false,
            PlatformClass::Unix,
        )
        .unwrap();

        assert!(second.is_none(), "second merge should be a no-op");
    }

    #[test]
    fn resolves_backup_entries() {
        assert!(spec_for_backup_entry("claude", "settings.json").is_some());
        assert!(spec_for_backup_entry("codex", "config.toml").is_some());
        assert!(spec_for_backup_entry("geminicli", "settings.json").is_some());
        assert!(spec_for_backup_entry("codebuddy", "settings.json").is_some());
        assert!(spec_for_backup_entry("zcode", "cli/config.json").is_some());
        assert!(spec_for_backup_entry("claude", ".claude.json").is_none());
        assert!(spec_for_backup_entry("codex", "AGENTS.md").is_none());
    }

    #[test]
    fn empty_content_has_no_target_values() {
        let values = extract_target_values("", &CLAUDE_SETTINGS_SPEC, PlatformClass::Unix).unwrap();
        assert_eq!(values.len(), 2);
        assert!(values.iter().all(Option::is_none));
    }

    #[test]
    fn invalid_non_empty_target_content_is_an_error() {
        assert!(extract_target_values("{", &CLAUDE_SETTINGS_SPEC, PlatformClass::Unix).is_err());
    }

    #[test]
    fn target_hooks_override_source_hooks() {
        let target = r#"{"hooks":{"Stop":[{"hooks":[{"command":"/target/hook.sh"}]}]}}"#;
        let target_values =
            extract_target_values(target, &CLAUDE_SETTINGS_SPEC, PlatformClass::Unix).unwrap();
        let source =
            r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"command":"/source/hook.sh"}]}]}}"#;

        let merged = apply_values(
            source,
            &CLAUDE_SETTINGS_SPEC,
            &target_values,
            false,
            PlatformClass::Unix,
        )
        .unwrap()
        .expect("changed");

        let value: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(value["model"], "opus");
        assert_eq!(
            value["hooks"]["Stop"][0]["hooks"][0]["command"],
            "/target/hook.sh"
        );
    }

    #[test]
    fn empty_source_keeps_target_hooks() {
        let target = r#"{"hooks":{"Stop":[{"hooks":[{"command":"/target/hook.sh"}]}]}}"#;
        let target_values =
            extract_target_values(target, &CLAUDE_SETTINGS_SPEC, PlatformClass::Unix).unwrap();

        let merged = apply_values(
            "",
            &CLAUDE_SETTINGS_SPEC,
            &target_values,
            false,
            PlatformClass::Unix,
        )
        .unwrap()
        .expect("changed");

        let value: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(value["hooks"]["Stop"][0]["hooks"][0]["command"], "/target/hook.sh");
    }

    #[test]
    fn cross_platform_source_hooks_are_removed_when_target_has_none() {
        let source =
            r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"command":"C:/tools/hook.exe"}]}]}}"#;

        let merged = apply_values(
            source,
            &CLAUDE_SETTINGS_SPEC,
            &[None],
            false,
            PlatformClass::Unix,
        )
        .unwrap()
        .expect("changed");

        let value: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(value["model"], "opus");
        assert!(value.get("hooks").is_none());
    }

    #[test]
    fn same_platform_source_hooks_are_kept_when_target_has_none() {
        let source = r#"{"hooks":{"Stop":[{"hooks":[{"command":"/source/hook.sh"}]}]}}"#;

        let merged = apply_values(
            source,
            &CLAUDE_SETTINGS_SPEC,
            &[None],
            true,
            PlatformClass::Unix,
        )
        .unwrap();

        assert!(merged.is_none(), "portable source hook is already correct");
    }

    #[test]
    fn sanitizes_windows_only_hook_entries_and_empty_events() {
        let target = r#"{
            "hooks": {
                "Stop": [
                    {"matcher": "a", "hooks": [{"command": "C:/tools/hook.exe"}]},
                    {"matcher": "b", "hooks": [{"command": "/usr/local/bin/hook"}]}
                ],
                "PreToolUse": [
                    {"hooks": [{"command": "powershell -File hook.ps1"}]}
                ]
            }
        }"#;
        let values =
            extract_target_values(target, &CLAUDE_SETTINGS_SPEC, PlatformClass::Unix).unwrap();
        let Some(HostLocalValue::Json(value)) = &values[0] else {
            panic!("expected JSON hooks");
        };

        assert_eq!(value["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(
            value["Stop"][0]["hooks"][0]["command"],
            "/usr/local/bin/hook"
        );
        assert!(value.get("PreToolUse").is_none());
    }

    #[test]
    fn target_status_line_overrides_source() {
        let target = r#"{"statusLine":{"type":"command","command":"/target/status.sh"}}"#;
        let target_values =
            extract_target_values(target, &CLAUDE_SETTINGS_SPEC, PlatformClass::Unix).unwrap();
        let source = r#"{"statusLine":{"type":"command","command":"C:/tools/status.exe"}}"#;

        let merged = apply_values(
            source,
            &CLAUDE_SETTINGS_SPEC,
            &target_values,
            false,
            PlatformClass::Unix,
        )
        .unwrap()
        .expect("changed");

        let value: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(value["statusLine"]["command"], "/target/status.sh");
    }

    #[test]
    fn cross_platform_source_status_line_is_removed_when_target_has_none() {
        let source = r#"{"statusLine":{"type":"command","command":"C:/tools/status.exe"}}"#;

        let merged = apply_values(
            source,
            &CLAUDE_SETTINGS_SPEC,
            &[None, None],
            false,
            PlatformClass::Unix,
        )
        .unwrap()
        .expect("changed");

        let value: Value = serde_json::from_str(&merged).unwrap();
        assert!(value.get("statusLine").is_none());
    }

    #[test]
    fn codex_hook_state_with_windows_keys_is_dropped_on_unix_target() {
        let target = r#"
[hooks.state]
"C:\\Users\\me\\.codex\\hooks.json:stop:0:0" = { enabled = true }
"/home/u/.codex/hooks.json:stop:0:0" = { enabled = true }
"#;
        let values =
            extract_target_values(target, &CODEX_CONFIG_SPEC, PlatformClass::Unix).unwrap();
        let Some(HostLocalValue::Toml(item)) = &values[2] else {
            panic!("expected TOML hooks");
        };
        let table = item.as_table_like().expect("hooks table");
        let state = table
            .get("state")
            .and_then(|state| state.as_table_like())
            .expect("state table");
        let keys: Vec<String> = state.iter().map(|(key, _)| key.to_string()).collect();
        assert!(!keys.iter().any(|key| key.contains("C:\\Users")));
        assert!(keys
            .iter()
            .any(|key| key == "/home/u/.codex/hooks.json:stop:0:0"));
    }

    #[test]
    fn implicit_empty_codex_hooks_stays_explicit() {
        let target = r#"
[hooks.state.'C:\Users\me\h.json:stop:0:0']
enabled = true
"#;
        let values =
            extract_target_values(target, &CODEX_CONFIG_SPEC, PlatformClass::Unix).unwrap();
        let Some(HostLocalValue::Toml(item)) = &values[2] else {
            panic!("expected TOML hooks");
        };
        assert!(item.as_table_like().expect("hooks table").is_empty());

        let merged = apply_values(
            "model = \"gpt-5\"\n",
            &CODEX_CONFIG_SPEC,
            &values,
            false,
            PlatformClass::Unix,
        )
        .unwrap()
        .expect("changed");
        assert!(merged.contains("[hooks]"));
        assert!(
            apply_values(
                &merged,
                &CODEX_CONFIG_SPEC,
                &values,
                false,
                PlatformClass::Unix
            )
            .unwrap()
            .is_none(),
            "explicit empty hooks should be idempotent"
        );
    }

    #[test]
    fn codex_target_notify_and_projects_override_source() {
        let target = r#"
notify = ["/usr/local/bin/notify"]
[projects."/home/u/project"]
trust_level = "trusted"
"#;
        let target_values =
            extract_target_values(target, &CODEX_CONFIG_SPEC, PlatformClass::Unix).unwrap();
        let source = r#"
model = "gpt-5"
notify = ["C:\\tools\\notify.exe"]
[projects."C:\\Users\\me\\project"]
trust_level = "trusted"
[projects."/home/u/other"]
trust_level = "trusted"
"#;

        let merged = apply_values(
            source,
            &CODEX_CONFIG_SPEC,
            &target_values,
            false,
            PlatformClass::Unix,
        )
        .unwrap()
        .expect("changed");

        assert!(merged.contains(r#"notify = ["/usr/local/bin/notify"]"#));
        assert!(merged.contains(r#"[projects."/home/u/project"]"#));
        assert!(!merged.contains("C:\\Users"));
        assert!(!merged.contains("/home/u/other"));
        assert!(merged.contains(r#"model = "gpt-5""#));
    }

    #[test]
    fn codex_target_hooks_override_source_hooks() {
        let target = r#"
[hooks.state]
"/target/hooks.json:stop:0:0" = true
"#;
        let target_values =
            extract_target_values(target, &CODEX_CONFIG_SPEC, PlatformClass::Unix).unwrap();
        let source = r#"
model = "gpt-5"
[hooks.state]
"/source/hooks.json:stop:0:0" = true
"#;

        let merged = apply_values(
            source,
            &CODEX_CONFIG_SPEC,
            &target_values,
            false,
            PlatformClass::Unix,
        )
        .unwrap()
        .expect("changed");

        assert!(merged.contains("/target/hooks.json:stop:0:0"));
        assert!(!merged.contains("/source/hooks.json:stop:0:0"));
        assert!(merged.contains(r#"model = "gpt-5""#));
    }

    #[test]
    fn codex_windows_project_keys_are_removed_from_target_values() {
        let target = r#"
[projects."C:\\Users\\me\\project"]
trust_level = "trusted"
[projects."/home/u/project"]
trust_level = "trusted"
"#;
        let values =
            extract_target_values(target, &CODEX_CONFIG_SPEC, PlatformClass::Unix).unwrap();
        let Some(HostLocalValue::Toml(item)) = &values[1] else {
            panic!("expected TOML projects");
        };
        let table = item.as_table_like().expect("projects table");
        let keys: Vec<String> = table.iter().map(|(key, _)| key.to_string()).collect();
        assert!(!keys.iter().any(|key| key.contains("C:\\Users")));
        assert!(keys.iter().any(|key| key == "/home/u/project"));
    }

    #[test]
    fn windows_only_notify_is_emptied_on_unix_target() {
        let target = r#"notify = ["C:\\tools\\notify.exe", "done"]"#;
        let values =
            extract_target_values(target, &CODEX_CONFIG_SPEC, PlatformClass::Unix).unwrap();
        let Some(HostLocalValue::Toml(item)) = &values[0] else {
            panic!("expected TOML notify");
        };
        assert!(item.as_array().expect("notify array").is_empty());
    }

    #[test]
    fn explicit_empty_target_values_win_over_source() {
        let json_target = r#"{"hooks":{}}"#;
        let json_values =
            extract_target_values(json_target, &CLAUDE_SETTINGS_SPEC, PlatformClass::Unix).unwrap();
        let json_source =
            r#"{"hooks":{"Stop":[{"hooks":[{"command":"/source/hook.sh"}]}]}}"#;
        let merged_json = apply_values(
            json_source,
            &CLAUDE_SETTINGS_SPEC,
            &json_values,
            true,
            PlatformClass::Unix,
        )
        .unwrap()
        .expect("changed");
        let json_value: Value = serde_json::from_str(&merged_json).unwrap();
        assert_eq!(json_value["hooks"], serde_json::json!({}));
        assert!(!merged_json.contains("/source/hook.sh"));

        let toml_target = "notify = []\n[projects]\n[hooks]\n";
        let toml_values =
            extract_target_values(toml_target, &CODEX_CONFIG_SPEC, PlatformClass::Unix).unwrap();
        let toml_source = r#"notify = ["/source/notify"]"#;
        let merged_toml = apply_values(
            toml_source,
            &CODEX_CONFIG_SPEC,
            &toml_values,
            true,
            PlatformClass::Unix,
        )
        .unwrap()
        .expect("changed");
        let toml_value: toml_edit::DocumentMut = merged_toml.parse().unwrap();
        assert!(toml_value["notify"].as_array().unwrap().is_empty());
        assert!(toml_value.get("projects").is_some());
        assert!(toml_value.get("hooks").is_some());
        assert!(!merged_toml.contains("/source/notify"));
        assert!(
            apply_values(
                &merged_toml,
                &CODEX_CONFIG_SPEC,
                &toml_values,
                true,
                PlatformClass::Unix,
            )
            .unwrap()
            .is_none(),
            "explicit empty values should be idempotent"
        );
    }

    #[test]
    fn toml_comments_are_preserved_when_merging() {
        let source =
            "# keep this comment\nmodel = \"gpt-5\"\nnotify = [\"C:\\\\tools\\\\n.exe\"]\n";
        let merged = apply_values(
            source,
            &CODEX_CONFIG_SPEC,
            &[None, None],
            false,
            PlatformClass::Unix,
        )
        .unwrap()
        .expect("changed");

        assert!(merged.contains("# keep this comment"));
        assert!(!merged.contains("notify"));
    }

    #[test]
    fn windows_command_detection_covers_common_forms() {
        assert!(is_windows_only_command("C:/Users/me/hook.exe"));
        assert!(is_windows_only_command(r"C:\Users\me\hook.exe"));
        assert!(is_windows_only_command(r"\\server\share\hook.exe"));
        assert!(is_windows_only_command("cmd /c C:\\tools\\hook.exe"));
        assert!(is_windows_only_command("powershell -File hook.ps1"));
        assert!(is_windows_only_command(r"node %USERPROFILE%\hook.js"));
        assert!(is_windows_only_command(r"node $env:USERPROFILE\hook.js"));
        assert!(!is_windows_only_command("/usr/local/bin/hook"));
        assert!(!is_windows_only_command("node /home/u/hook.js"));
        assert!(!is_windows_only_command("pwsh -File /home/u/hook.ps1"));
        assert!(!is_windows_only_command("https://example.com/hook"));
        assert!(!is_windows_only_command(r"printf '\\n'"));
        assert!(!is_windows_only_command("date +%Y%m%d"));
        assert!(is_windows_only_command("echo %date%"));
    }

    #[test]
    fn windows_path_and_command_helpers_do_not_flag_urls() {
        assert!(!is_windows_absolute_path("https://example.com"));
        assert!(is_windows_absolute_path(r"c:\users\me"));
        assert!(is_windows_absolute_path("D:/work/project"));
        assert!(is_windows_absolute_path(r"\\server\share"));
        assert!(!is_windows_absolute_path("/home/u/project"));
    }
}
