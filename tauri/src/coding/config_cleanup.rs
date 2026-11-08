use serde_json::Value as JsonValue;
use std::collections::HashSet;
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value as TomlValue};

pub const CLAUDE_NON_WINDOWS_TARGET_CLEANUP_PATHS: &[&str] = &[
    "$.env.CLAUDE_CODE_USE_POWERSHELL_TOOL",
    "$.env.CLAUDE_CODE_SHELL",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupFileFormat {
    Json,
    Toml,
}

pub fn cleanup_file_format_for_path(path: &str) -> Option<CleanupFileFormat> {
    let normalized = path
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .replace('\\', "/")
        .to_ascii_lowercase();

    if normalized.ends_with(".json") {
        Some(CleanupFileFormat::Json)
    } else if normalized.ends_with(".toml") {
        Some(CleanupFileFormat::Toml)
    } else {
        None
    }
}

pub fn cleanup_file_format_for_mapping_paths(
    target_path: &str,
    source_path: &str,
) -> Option<CleanupFileFormat> {
    cleanup_file_format_for_path(target_path).or_else(|| cleanup_file_format_for_path(source_path))
}

pub fn mapping_supports_cleanup_paths(
    is_directory: bool,
    is_pattern: bool,
    target_path: &str,
    source_path: &str,
) -> bool {
    !is_directory
        && !is_pattern
        && cleanup_file_format_for_mapping_paths(target_path, source_path).is_some()
}

pub fn normalize_cleanup_paths(paths: &[String]) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    let mut normalized_paths = Vec::new();

    for path in paths {
        let normalized_path = path.trim();
        if normalized_path.is_empty() {
            continue;
        }
        parse_cleanup_path(normalized_path)?;
        if seen.insert(normalized_path.to_string()) {
            normalized_paths.push(normalized_path.to_string());
        }
    }

    Ok(normalized_paths)
}

pub fn cleanup_paths_for_mapping(
    is_directory: bool,
    is_pattern: bool,
    target_path: &str,
    source_path: &str,
    cleanup_paths: &[String],
) -> Result<Vec<String>, String> {
    let normalized_paths = normalize_cleanup_paths(cleanup_paths)?;
    if normalized_paths.is_empty() {
        return Ok(normalized_paths);
    }

    if !mapping_supports_cleanup_paths(is_directory, is_pattern, target_path, source_path) {
        return Err("字段清理路径仅支持 JSON/TOML 单文件映射".to_string());
    }

    Ok(normalized_paths)
}

pub fn apply_cleanup_paths_to_content(
    content: &str,
    format: CleanupFileFormat,
    cleanup_paths: &[String],
) -> Result<Option<String>, String> {
    let cleanup_paths = normalize_cleanup_paths(cleanup_paths)?;
    if cleanup_paths.is_empty() || content.trim().is_empty() {
        return Ok(None);
    }

    match format {
        CleanupFileFormat::Json => apply_cleanup_paths_to_json_content(content, &cleanup_paths),
        CleanupFileFormat::Toml => apply_cleanup_paths_to_toml_content(content, &cleanup_paths),
    }
}

pub fn sanitize_claude_settings_for_non_windows_target(
    settings_value: &JsonValue,
) -> Result<Option<JsonValue>, String> {
    let cleanup_paths = CLAUDE_NON_WINDOWS_TARGET_CLEANUP_PATHS
        .iter()
        .map(|path| (*path).to_string())
        .collect::<Vec<_>>();
    apply_cleanup_paths_to_json_value(settings_value, &cleanup_paths)
}

pub fn sanitize_claude_settings_content_for_non_windows_target(
    raw_settings: &str,
) -> Result<Option<String>, String> {
    let cleanup_paths = CLAUDE_NON_WINDOWS_TARGET_CLEANUP_PATHS
        .iter()
        .map(|path| (*path).to_string())
        .collect::<Vec<_>>();
    apply_cleanup_paths_to_content(raw_settings, CleanupFileFormat::Json, &cleanup_paths)
}

/// Downgrade Claude's bypass permission mode for root remotes, where Claude
/// Code refuses to start in `bypassPermissions` mode. Returns `None` when the
/// settings do not currently use `bypassPermissions`, so custom modes such as
/// `acceptEdits` or `plan` are left untouched.
pub fn sanitize_claude_settings_for_root_user(
    settings_value: &JsonValue,
) -> Result<Option<JsonValue>, String> {
    let uses_bypass_permissions = settings_value
        .get("permissions")
        .and_then(|permissions| permissions.as_object())
        .and_then(|permissions| permissions.get("defaultMode"))
        .and_then(|default_mode| default_mode.as_str())
        == Some("bypassPermissions");

    if !uses_bypass_permissions {
        return Ok(None);
    }

    let mut sanitized = settings_value.clone();
    let sanitized_object = sanitized
        .as_object_mut()
        .ok_or_else(|| "Claude settings must be a JSON object".to_string())?;

    if let Some(permissions) = sanitized_object
        .get_mut("permissions")
        .and_then(|permissions| permissions.as_object_mut())
    {
        permissions.insert(
            "defaultMode".to_string(),
            JsonValue::String("auto".to_string()),
        );
        permissions.remove("dangerouslySkipPermissions");
        permissions.remove("skipDangerousModePermissionPrompt");
    }

    sanitized_object.remove("dangerouslySkipPermissions");
    sanitized_object.remove("skipDangerousModePermissionPrompt");

    Ok(Some(sanitized))
}

pub fn sanitize_claude_settings_content_for_root_user(
    raw_settings: &str,
) -> Result<Option<String>, String> {
    if raw_settings.trim().is_empty() {
        return Ok(None);
    }

    let settings_value: JsonValue = serde_json::from_str(raw_settings)
        .map_err(|error| format!("Failed to parse Claude settings for root fix: {error}"))?;
    let Some(sanitized_value) = sanitize_claude_settings_for_root_user(&settings_value)? else {
        return Ok(None);
    };

    let serialized = serde_json::to_string_pretty(&sanitized_value)
        .map_err(|error| format!("Failed to serialize Claude settings after root fix: {error}"))?;
    Ok(Some(format!("{serialized}\n")))
}

pub fn apply_cleanup_paths_to_json_value(
    settings_value: &JsonValue,
    cleanup_paths: &[String],
) -> Result<Option<JsonValue>, String> {
    let cleanup_paths = normalize_cleanup_paths(cleanup_paths)?;
    if cleanup_paths.is_empty() {
        return Ok(None);
    }

    let mut cleaned_value = settings_value.clone();
    let mut changed = false;
    for cleanup_path in cleanup_paths {
        let components = parse_cleanup_path(&cleanup_path)?;
        changed |= remove_json_path(&mut cleaned_value, &components);
    }

    if changed {
        Ok(Some(cleaned_value))
    } else {
        Ok(None)
    }
}

fn apply_cleanup_paths_to_json_content(
    content: &str,
    cleanup_paths: &[String],
) -> Result<Option<String>, String> {
    let value: JsonValue = serde_json::from_str(content)
        .map_err(|error| format!("Failed to parse JSON for field cleanup: {error}"))?;
    let Some(cleaned_value) = apply_cleanup_paths_to_json_value(&value, cleanup_paths)? else {
        return Ok(None);
    };
    let serialized = serde_json::to_string_pretty(&cleaned_value)
        .map_err(|error| format!("Failed to serialize JSON after field cleanup: {error}"))?;
    Ok(Some(format!("{serialized}\n")))
}

fn apply_cleanup_paths_to_toml_content(
    content: &str,
    cleanup_paths: &[String],
) -> Result<Option<String>, String> {
    let mut document = content
        .parse::<DocumentMut>()
        .map_err(|error| format!("Failed to parse TOML for field cleanup: {error}"))?;
    let mut changed = false;

    for cleanup_path in cleanup_paths {
        let components = parse_cleanup_path(cleanup_path)?;
        changed |= remove_toml_path_from_table(document.as_table_mut(), &components);
    }

    if changed {
        Ok(Some(document.to_string()))
    } else {
        Ok(None)
    }
}

fn parse_cleanup_path(path: &str) -> Result<Vec<String>, String> {
    let chars = path.chars().collect::<Vec<_>>();
    if chars.first() != Some(&'$') {
        return Err(format!("字段清理路径必须以 $ 开头: {path}"));
    }

    let mut components = Vec::new();
    let mut index = 1;
    while index < chars.len() {
        match chars[index] {
            '.' => {
                index += 1;
                let start = index;
                while index < chars.len() && chars[index] != '.' && chars[index] != '[' {
                    index += 1;
                }
                if start == index {
                    return Err(format!("字段清理路径包含空字段: {path}"));
                }
                components.push(chars[start..index].iter().collect::<String>());
            }
            '[' => {
                index += 1;
                if index >= chars.len() || (chars[index] != '"' && chars[index] != '\'') {
                    return Err(format!("字段清理路径 bracket 写法需要引号: {path}"));
                }
                let quote = chars[index];
                index += 1;
                let mut component = String::new();
                let mut closed = false;
                while index < chars.len() {
                    let current = chars[index];
                    if current == '\\' {
                        index += 1;
                        if index >= chars.len() {
                            return Err(format!("字段清理路径转义不完整: {path}"));
                        }
                        component.push(chars[index]);
                        index += 1;
                        continue;
                    }
                    if current == quote {
                        closed = true;
                        index += 1;
                        break;
                    }
                    component.push(current);
                    index += 1;
                }
                if !closed {
                    return Err(format!("字段清理路径 bracket 未闭合: {path}"));
                }
                if index >= chars.len() || chars[index] != ']' {
                    return Err(format!("字段清理路径 bracket 缺少 ]: {path}"));
                }
                index += 1;
                if component.is_empty() {
                    return Err(format!("字段清理路径包含空字段: {path}"));
                }
                components.push(component);
            }
            _ => return Err(format!("字段清理路径格式不支持: {path}")),
        }
    }

    if components.is_empty() {
        return Err(format!("字段清理路径必须指向具体字段: {path}"));
    }

    Ok(components)
}

fn remove_json_path(value: &mut JsonValue, components: &[String]) -> bool {
    let Some((first, rest)) = components.split_first() else {
        return false;
    };
    let Some(object) = value.as_object_mut() else {
        return false;
    };

    if rest.is_empty() {
        return object.remove(first).is_some();
    }

    let Some(child_value) = object.get_mut(first) else {
        return false;
    };
    let changed = remove_json_path(child_value, rest);
    if changed
        && child_value
            .as_object()
            .is_some_and(|child| child.is_empty())
    {
        object.remove(first);
    }
    changed
}

fn remove_toml_path_from_table(table: &mut Table, components: &[String]) -> bool {
    let Some((first, rest)) = components.split_first() else {
        return false;
    };

    if rest.is_empty() {
        return table.remove(first).is_some();
    }

    let Some(item) = table.get_mut(first) else {
        return false;
    };
    let changed = remove_toml_path_from_item(item, rest);
    if changed && toml_item_is_empty_container(item) {
        table.remove(first);
    }
    changed
}

fn remove_toml_path_from_item(item: &mut Item, components: &[String]) -> bool {
    if let Some(table) = item.as_table_mut() {
        return remove_toml_path_from_table(table, components);
    }

    let Item::Value(TomlValue::InlineTable(inline_table)) = item else {
        return false;
    };
    remove_toml_path_from_inline_table(inline_table, components)
}

fn remove_toml_path_from_inline_table(table: &mut InlineTable, components: &[String]) -> bool {
    let Some((first, rest)) = components.split_first() else {
        return false;
    };

    if rest.is_empty() {
        return table.remove(first).is_some();
    }

    let Some(value) = table.get_mut(first) else {
        return false;
    };
    let TomlValue::InlineTable(child_table) = value else {
        return false;
    };
    let changed = remove_toml_path_from_inline_table(child_table, rest);
    if changed && child_table.is_empty() {
        table.remove(first);
    }
    changed
}

fn toml_item_is_empty_container(item: &Item) -> bool {
    if let Some(table) = item.as_table() {
        return table.is_empty();
    }

    matches!(
        item,
        Item::Value(TomlValue::InlineTable(inline_table)) if inline_table.is_empty()
    )
}

#[cfg(test)]
mod tests {
    use super::{
        apply_cleanup_paths_to_content, cleanup_file_format_for_path,
        sanitize_claude_settings_content_for_non_windows_target,
        sanitize_claude_settings_content_for_root_user, CleanupFileFormat,
    };
    use serde_json::json;

    fn paths(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_string()).collect()
    }

    #[test]
    fn detects_json_and_toml_files() {
        assert_eq!(
            cleanup_file_format_for_path("~/.claude/settings.json"),
            Some(CleanupFileFormat::Json)
        );
        assert_eq!(
            cleanup_file_format_for_path("~/.codex/config.toml"),
            Some(CleanupFileFormat::Toml)
        );
        assert_eq!(cleanup_file_format_for_path("~/.gemini/.env"), None);
    }

    #[test]
    fn removes_json_field_paths_and_empty_parents() {
        let input = json!({
            "env": {
                "HTTP_PROXY": "http://127.0.0.1:7890",
                "HTTPS_PROXY": "http://127.0.0.1:7890"
            },
            "keep": true
        })
        .to_string();

        let output = apply_cleanup_paths_to_content(
            &input,
            CleanupFileFormat::Json,
            &paths(&["$.env.HTTP_PROXY", "$.env.HTTPS_PROXY"]),
        )
        .expect("cleanup should succeed")
        .expect("content should change");
        let output_json: serde_json::Value =
            serde_json::from_str(&output).expect("output should be valid JSON");

        assert!(output_json.get("env").is_none());
        assert_eq!(output_json.get("keep"), Some(&json!(true)));
    }

    #[test]
    fn removes_toml_field_paths() {
        let input = r#"
[mcp_servers.demo]
command = "node"

[mcp_servers.demo.env]
HTTP_PROXY = "http://127.0.0.1:7890"
HTTPS_PROXY = "http://127.0.0.1:7890"
KEEP = "yes"
"#;

        let output = apply_cleanup_paths_to_content(
            input,
            CleanupFileFormat::Toml,
            &paths(&[
                "$.mcp_servers.demo.env.HTTP_PROXY",
                "$.mcp_servers.demo.env.HTTPS_PROXY",
            ]),
        )
        .expect("cleanup should succeed")
        .expect("content should change");

        assert!(!output.contains("HTTP_PROXY"));
        assert!(!output.contains("HTTPS_PROXY"));
        assert!(output.contains("KEEP"));
        assert!(output.contains("command = \"node\""));
    }

    #[test]
    fn supports_quoted_path_segments() {
        let input = r#"
[mcp_servers."server.with.dot".env]
HTTP_PROXY = "http://127.0.0.1:7890"
KEEP = "yes"
"#;

        let output = apply_cleanup_paths_to_content(
            input,
            CleanupFileFormat::Toml,
            &paths(&[r#"$.mcp_servers["server.with.dot"].env.HTTP_PROXY"#]),
        )
        .expect("cleanup should succeed")
        .expect("content should change");

        assert!(!output.contains("HTTP_PROXY"));
        assert!(output.contains("KEEP"));
    }

    #[test]
    fn claude_non_windows_cleanup_removes_powershell_env_only() {
        let input = json!({
            "env": {
                "CLAUDE_CODE_USE_POWERSHELL_TOOL": "1",
                "CLAUDE_CODE_SHELL": "pwsh",
                "HTTP_PROXY": "http://127.0.0.1:7890",
                "CUSTOM_ENV": "keep"
            }
        })
        .to_string();

        let output = sanitize_claude_settings_content_for_non_windows_target(&input)
            .expect("cleanup should succeed")
            .expect("content should change");
        let output_json: serde_json::Value =
            serde_json::from_str(&output).expect("output should be valid JSON");

        assert!(output_json
            .pointer("/env/CLAUDE_CODE_USE_POWERSHELL_TOOL")
            .is_none());
        assert!(output_json.pointer("/env/CLAUDE_CODE_SHELL").is_none());
        assert_eq!(
            output_json.pointer("/env/HTTP_PROXY"),
            Some(&json!("http://127.0.0.1:7890"))
        );
        assert_eq!(output_json.pointer("/env/CUSTOM_ENV"), Some(&json!("keep")));
    }

    #[test]
    fn claude_root_fix_downgrades_bypass_permissions_and_removes_dangerous_fields() {
        let input = json!({
            "permissions": {
                "defaultMode": "bypassPermissions",
                "dangerouslySkipPermissions": true,
                "skipDangerousModePermissionPrompt": true,
                "allow": ["Bash(npm run lint)"]
            },
            "skipDangerousModePermissionPrompt": true,
            "dangerouslySkipPermissions": true,
            "model": "claude-sonnet-5",
            "enabledPlugins": { "demo@marketplace": true },
            "hooks": { "PreToolUse": [] },
            "extraKnownMarketplaces": { "demo": { "source": "github" } }
        })
        .to_string();

        let output = sanitize_claude_settings_content_for_root_user(&input)
            .expect("root fix should succeed")
            .expect("content should change");
        let output_json: serde_json::Value =
            serde_json::from_str(&output).expect("output should be valid JSON");

        assert_eq!(
            output_json.pointer("/permissions/defaultMode"),
            Some(&json!("auto"))
        );
        assert!(output_json
            .pointer("/permissions/dangerouslySkipPermissions")
            .is_none());
        assert!(output_json
            .pointer("/permissions/skipDangerousModePermissionPrompt")
            .is_none());
        assert!(output_json.pointer("/dangerouslySkipPermissions").is_none());
        assert!(output_json
            .pointer("/skipDangerousModePermissionPrompt")
            .is_none());
        assert_eq!(
            output_json.pointer("/permissions/allow"),
            Some(&json!(["Bash(npm run lint)"]))
        );
        assert_eq!(
            output_json.pointer("/model"),
            Some(&json!("claude-sonnet-5"))
        );
        assert_eq!(
            output_json.pointer("/enabledPlugins/demo@marketplace"),
            Some(&json!(true))
        );
        assert_eq!(output_json.pointer("/hooks/PreToolUse"), Some(&json!([])));
        assert_eq!(
            output_json.pointer("/extraKnownMarketplaces/demo/source"),
            Some(&json!("github"))
        );
    }

    #[test]
    fn claude_root_fix_leaves_non_bypass_modes_untouched() {
        for mode in ["auto", "acceptEdits", "plan", "default"] {
            let input = json!({
                "permissions": {
                    "defaultMode": mode,
                    "dangerouslySkipPermissions": true,
                    "skipDangerousModePermissionPrompt": true
                },
                "skipDangerousModePermissionPrompt": true
            })
            .to_string();

            let output = sanitize_claude_settings_content_for_root_user(&input)
                .expect("root fix should succeed");
            assert!(output.is_none(), "mode {mode} should not be rewritten");
        }
    }

    #[test]
    fn claude_root_fix_ignores_missing_or_invalid_permissions() {
        for input in [
            json!({ "model": "claude-sonnet-5" }),
            json!({ "permissions": null }),
            json!({ "permissions": [] }),
            json!({ "permissions": { "defaultMode": 1 } }),
        ] {
            let output = sanitize_claude_settings_content_for_root_user(&input.to_string())
                .expect("root fix should succeed");
            assert!(output.is_none(), "input should not be rewritten: {input}");
        }
    }

    #[test]
    fn claude_root_fix_handles_empty_and_invalid_content() {
        assert!(sanitize_claude_settings_content_for_root_user("  \n")
            .expect("empty content should succeed")
            .is_none());
        assert!(sanitize_claude_settings_content_for_root_user(r#"{ "permissions": }"#).is_err());
    }
}
