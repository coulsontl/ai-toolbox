use serde_json::Value;

use super::tool_adapters::CustomTool;
use super::types::{Skill, SkillGroupRecord, SkillPreferences, SkillRepo, SkillTarget};
use crate::coding::db_extract_id;

// ==================== Skill ====================

/// Convert database record to Skill struct (wide table pattern)
pub fn from_db_skill(value: Value) -> Skill {
    let enabled_tools: Vec<String> = parse_string_array(value.get("enabled_tools"));
    let disabled_previous_tools: Vec<String> =
        parse_string_array(value.get("disabled_previous_tools"));

    Skill {
        id: db_extract_id(&value),
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        source_type: value
            .get("source_type")
            .and_then(|v| v.as_str())
            .unwrap_or("local")
            .to_string(),
        source_ref: value
            .get("source_ref")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        source_revision: value
            .get("source_revision")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        central_path: value
            .get("central_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        content_hash: value
            .get("content_hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        created_at: value
            .get("created_at")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        updated_at: value
            .get("updated_at")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        last_sync_at: value.get("last_sync_at").and_then(|v| v.as_i64()),
        status: value
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("active")
            .to_string(),
        sort_index: value
            .get("sort_index")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
        user_group: value
            .get("user_group")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        group_id: value
            .get("group_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        user_note: value
            .get("user_note")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        management_enabled: value
            .get("management_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        disabled_previous_tools,
        enabled_tools,
        sync_details: value.get("sync_details").cloned().filter(|v| !v.is_null()),
    }
}

fn parse_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Convert Skill to clean database payload (without id)
pub fn to_clean_skill_payload(skill: &Skill) -> Value {
    serde_json::json!({
        "name": skill.name,
        "source_type": skill.source_type,
        "source_ref": skill.source_ref,
        "source_revision": skill.source_revision,
        "central_path": skill.central_path,
        "content_hash": skill.content_hash,
        "created_at": skill.created_at,
        "updated_at": skill.updated_at,
        "last_sync_at": skill.last_sync_at,
        "status": skill.status,
        "sort_index": skill.sort_index,
        "user_group": skill.user_group,
        "group_id": skill.group_id,
        "user_note": skill.user_note,
        "management_enabled": skill.management_enabled,
        "disabled_previous_tools": skill.disabled_previous_tools,
        "enabled_tools": skill.enabled_tools,
        "sync_details": skill.sync_details,
    })
}

pub fn from_db_skill_group(value: Value) -> SkillGroupRecord {
    SkillGroupRecord {
        id: db_extract_id(&value),
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        note: value
            .get("note")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        sort_index: value
            .get("sort_index")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
        created_at: value
            .get("created_at")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        updated_at: value
            .get("updated_at")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
    }
}

pub fn to_skill_group_payload(group: &SkillGroupRecord) -> Value {
    serde_json::json!({
        "name": group.name,
        "note": group.note,
        "sort_index": group.sort_index,
        "created_at": group.created_at,
        "updated_at": group.updated_at,
    })
}

// ==================== sync_details helpers ====================

/// Parse SkillTarget list from Skill's sync_details JSON
pub fn parse_sync_details(skill: &Skill) -> Vec<SkillTarget> {
    let Some(details) = &skill.sync_details else {
        return Vec::new();
    };
    let Some(obj) = details.as_object() else {
        return Vec::new();
    };

    obj.iter()
        .map(|(tool_key, entry)| SkillTarget {
            tool: tool_key.clone(),
            target_path: entry
                .get("target_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            mode: entry
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("symlink")
                .to_string(),
            status: entry
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("pending")
                .to_string(),
            synced_at: entry.get("synced_at").and_then(|v| v.as_i64()),
            error_message: entry
                .get("error_message")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
        .collect()
}

/// Set a SkillTarget in sync_details JSON (upsert single tool)
pub fn set_sync_detail(existing: &Option<Value>, tool: &str, target: &SkillTarget) -> Value {
    let mut obj = existing
        .as_ref()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    obj.insert(
        tool.to_string(),
        serde_json::json!({
            "target_path": target.target_path,
            "mode": target.mode,
            "status": target.status,
            "synced_at": target.synced_at,
            "error_message": target.error_message,
        }),
    );

    Value::Object(obj)
}

/// Remove a tool from sync_details JSON
pub fn remove_sync_detail(existing: &Option<Value>, tool: &str) -> Value {
    let mut obj = existing
        .as_ref()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    obj.remove(tool);
    Value::Object(obj)
}

/// Get a specific tool's SkillTarget from sync_details JSON
pub fn get_sync_detail(existing: &Option<Value>, tool: &str) -> Option<SkillTarget> {
    let obj = existing.as_ref()?.as_object()?;
    let entry = obj.get(tool)?;

    Some(SkillTarget {
        tool: tool.to_string(),
        target_path: entry
            .get("target_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        mode: entry
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("symlink")
            .to_string(),
        status: entry
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("pending")
            .to_string(),
        synced_at: entry.get("synced_at").and_then(|v| v.as_i64()),
        error_message: entry
            .get("error_message")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

// ==================== SkillRepo ====================

/// Convert database record to SkillRepo struct
pub fn from_db_skill_repo(value: Value) -> SkillRepo {
    SkillRepo {
        id: db_extract_id(&value),
        owner: value
            .get("owner")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        branch: value
            .get("branch")
            .and_then(|v| v.as_str())
            .unwrap_or("main")
            .to_string(),
        enabled: value
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        created_at: value
            .get("created_at")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
    }
}

/// Convert SkillRepo to clean database payload (without id)
pub fn to_skill_repo_payload(repo: &SkillRepo) -> Value {
    serde_json::json!({
        "owner": repo.owner,
        "name": repo.name,
        "branch": repo.branch,
        "enabled": repo.enabled,
        "created_at": repo.created_at,
    })
}

// ==================== SkillPreferences ====================

/// Convert database record to SkillPreferences struct
pub fn from_db_skill_preferences(value: Value) -> SkillPreferences {
    let default = SkillPreferences::default();

    // Parse preferred_tools: JSON array -> Option<Vec<String>>
    let preferred_tools: Option<Vec<String>> = value
        .get("preferred_tools")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect()
        });

    // Parse installed_tools: JSON array -> Option<Vec<String>>
    let installed_tools: Option<Vec<String>> = value
        .get("installed_tools")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect()
        });

    SkillPreferences {
        id: db_extract_id(&value),
        central_repo_path: value
            .get("central_repo_path")
            .and_then(|v| v.as_str())
            .unwrap_or(&default.central_repo_path)
            .to_string(),
        preferred_tools,
        default_view_mode: normalize_default_view_mode(
            value.get("default_view_mode").and_then(|v| v.as_str()),
        ),
        git_cache_cleanup_days: value
            .get("git_cache_cleanup_days")
            .and_then(|v| v.as_i64())
            .unwrap_or(30) as i32,
        git_cache_ttl_secs: value
            .get("git_cache_ttl_secs")
            .and_then(|v| v.as_i64())
            .unwrap_or(60) as i32,
        known_tool_versions: value.get("known_tool_versions").cloned(),
        installed_tools,
        show_skills_in_tray: value
            .get("show_skills_in_tray")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        updated_at: value
            .get("updated_at")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
    }
}

fn normalize_default_view_mode(value: Option<&str>) -> String {
    match value {
        Some("grouped") => "grouped".to_string(),
        _ => "flat".to_string(),
    }
}

/// Convert SkillPreferences to database payload
pub fn to_skill_preferences_payload(prefs: &SkillPreferences) -> Value {
    serde_json::json!({
        "central_repo_path": prefs.central_repo_path,
        "preferred_tools": prefs.preferred_tools,
        "default_view_mode": normalize_default_view_mode(Some(&prefs.default_view_mode)),
        "git_cache_cleanup_days": prefs.git_cache_cleanup_days,
        "git_cache_ttl_secs": prefs.git_cache_ttl_secs,
        "known_tool_versions": prefs.known_tool_versions,
        "installed_tools": prefs.installed_tools,
        "show_skills_in_tray": prefs.show_skills_in_tray,
        "updated_at": prefs.updated_at,
    })
}

// ==================== CustomTool ====================

/// Convert database record to CustomTool struct
pub fn from_db_custom_tool(value: Value) -> CustomTool {
    // key is stored as the record ID, extract and clean it
    let key = db_extract_id(&value);
    CustomTool {
        key,
        display_name: value
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        relative_skills_dir: value
            .get("relative_skills_dir")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        relative_detect_dir: value
            .get("relative_detect_dir")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        created_at: value
            .get("created_at")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        force_copy: value
            .get("force_copy")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

/// Convert CustomTool to database payload (excludes key since it's used as ID)
pub fn to_custom_tool_payload(tool: &CustomTool) -> Value {
    serde_json::json!({
        "display_name": tool.display_name,
        "relative_skills_dir": tool.relative_skills_dir,
        "relative_detect_dir": tool.relative_detect_dir,
        "created_at": tool.created_at,
        "force_copy": tool.force_copy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn skill_preferences_default_view_mode_falls_back_to_flat() {
        let missing = from_db_skill_preferences(json!({}));
        assert_eq!(missing.default_view_mode, "flat");

        let explicit_flat = from_db_skill_preferences(json!({ "default_view_mode": "flat" }));
        assert_eq!(explicit_flat.default_view_mode, "flat");

        let invalid = from_db_skill_preferences(json!({ "default_view_mode": "grid" }));
        assert_eq!(invalid.default_view_mode, "flat");
    }

    #[test]
    fn skill_preferences_default_view_mode_allows_grouped() {
        let grouped = from_db_skill_preferences(json!({ "default_view_mode": "grouped" }));
        assert_eq!(grouped.default_view_mode, "grouped");
    }

    #[test]
    fn skill_preferences_payload_normalizes_default_view_mode() {
        let mut prefs = SkillPreferences::default();
        prefs.default_view_mode = "grouped".to_string();
        let grouped_payload = to_skill_preferences_payload(&prefs);
        assert_eq!(
            grouped_payload
                .get("default_view_mode")
                .and_then(Value::as_str),
            Some("grouped")
        );

        prefs.default_view_mode = "grid".to_string();
        let fallback_payload = to_skill_preferences_payload(&prefs);
        assert_eq!(
            fallback_payload
                .get("default_view_mode")
                .and_then(Value::as_str),
            Some("flat")
        );
    }
}
