use serde_json::Value;
use std::collections::HashSet;

use crate::coding::db_id::{db_new_id, db_record_id};
use crate::DbState;

use super::adapter::{
    from_db_skill, from_db_skill_group, from_db_skill_preferences, from_db_skill_repo,
    get_sync_detail, parse_sync_details, remove_sync_detail, set_sync_detail,
    to_clean_skill_payload, to_skill_group_payload, to_skill_preferences_payload,
    to_skill_repo_payload,
};
use super::tool_adapters::CustomTool;
use super::types::{now_ms, Skill, SkillGroupRecord, SkillPreferences, SkillRepo, SkillTarget};

// ==================== Skill CRUD ====================

/// Get all managed skills
pub async fn get_managed_skills(state: &DbState) -> Result<Vec<Skill>, String> {
    migrate_legacy_skill_groups(state).await?;
    clear_dangling_skill_groups(state).await?;
    let db = state.db();

    let mut result = db
        .query("SELECT *, type::string(id) as id FROM skill ORDER BY sort_index ASC")
        .await
        .map_err(|e| format!("Failed to query skills: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(from_db_skill).collect())
}

pub async fn get_skill_groups(state: &DbState) -> Result<Vec<SkillGroupRecord>, String> {
    migrate_legacy_skill_groups(state).await?;
    clear_dangling_skill_groups(state).await?;
    let db = state.db();
    let mut result = db
        .query(
            "SELECT *, type::string(id) as id FROM skill_group ORDER BY sort_index ASC, name ASC",
        )
        .await
        .map_err(|e| format!("Failed to query skill groups: {}", e))?;
    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(from_db_skill_group).collect())
}

pub async fn save_skill_group(state: &DbState, group: &SkillGroupRecord) -> Result<String, String> {
    let db = state.db();
    let id = if group.id.is_empty() {
        db_new_id()
    } else {
        group.id.clone()
    };
    let record_id = db_record_id("skill_group", &id);
    db.query(&format!("UPSERT {} CONTENT $data", record_id))
        .bind(("data", to_skill_group_payload(group)))
        .await
        .map_err(|e| format!("Failed to save skill group: {}", e))?;
    Ok(id)
}

pub async fn delete_skill_group(state: &DbState, group_id: &str) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("skill_group", group_id);
    db.query("UPDATE skill SET group_id = NONE, user_group = NONE WHERE group_id = $group_id")
        .bind(("group_id", group_id.to_string()))
        .await
        .map_err(|e| format!("Failed to clear group skills: {}", e))?;
    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|e| format!("Failed to delete skill group: {}", e))?;
    Ok(())
}

pub async fn replace_skill_groups(
    state: &DbState,
    groups: &[SkillGroupRecord],
) -> Result<Vec<SkillGroupRecord>, String> {
    let db = state.db();
    db.query("DELETE skill_group")
        .await
        .map_err(|e| format!("Failed to clear skill groups: {}", e))?;
    let mut saved = Vec::new();
    for group in groups {
        let id = save_skill_group(state, group).await?;
        let mut group = group.clone();
        group.id = id;
        saved.push(group);
    }
    Ok(saved)
}

pub async fn migrate_legacy_skill_groups(state: &DbState) -> Result<(), String> {
    let db = state.db();
    let mut result = db
        .query("SELECT *, type::string(id) as id FROM skill WHERE group_id = NONE AND user_group != NONE")
        .await
        .map_err(|e| format!("Failed to query legacy skill groups: {}", e))?;
    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    if records.is_empty() {
        return Ok(());
    }

    let existing = get_skill_groups_without_migration(state).await?;
    let mut by_name: std::collections::HashMap<String, String> = existing
        .into_iter()
        .map(|group| (group.name.trim().to_lowercase(), group.id))
        .collect();
    let mut next_index = by_name.len() as i32;

    for record in records {
        let skill = from_db_skill(record);
        let Some(group_name) = skill
            .user_group
            .as_ref()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        else {
            continue;
        };
        let key = group_name.to_lowercase();
        let group_id = if let Some(id) = by_name.get(&key) {
            id.clone()
        } else {
            let now = now_ms();
            let group = SkillGroupRecord {
                id: db_new_id(),
                name: group_name.to_string(),
                note: None,
                sort_index: next_index,
                created_at: now,
                updated_at: now,
            };
            next_index += 1;
            let id = save_skill_group(state, &group).await?;
            by_name.insert(key, id.clone());
            id
        };
        let record_id = db_record_id("skill", &skill.id);
        db.query(&format!("UPDATE {} SET group_id = $group_id", record_id))
            .bind(("group_id", group_id))
            .await
            .map_err(|e| format!("Failed to migrate skill group: {}", e))?;
    }
    Ok(())
}

async fn clear_dangling_skill_groups(state: &DbState) -> Result<(), String> {
    let groups = get_skill_groups_without_migration(state).await?;
    let valid_group_ids: HashSet<String> = groups.into_iter().map(|group| group.id).collect();
    if valid_group_ids.is_empty() {
        let db = state.db();
        db.query("UPDATE skill SET group_id = NONE, user_group = NONE WHERE group_id != NONE")
            .await
            .map_err(|e| format!("Failed to clear dangling skill groups: {}", e))?;
        return Ok(());
    }

    let db = state.db();
    let mut result = db
        .query("SELECT *, type::string(id) as id FROM skill WHERE group_id != NONE")
        .await
        .map_err(|e| format!("Failed to query dangling skill groups: {}", e))?;
    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    for record in records {
        let skill = from_db_skill(record);
        let Some(group_id) = skill.group_id.as_ref() else {
            continue;
        };
        if valid_group_ids.contains(group_id) {
            continue;
        }
        let record_id = db_record_id("skill", &skill.id);
        db.query(&format!(
            "UPDATE {} SET group_id = NONE, user_group = NONE",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to clear dangling skill group: {}", e))?;
    }
    Ok(())
}

async fn get_skill_groups_without_migration(
    state: &DbState,
) -> Result<Vec<SkillGroupRecord>, String> {
    let db = state.db();
    let mut result = db
        .query(
            "SELECT *, type::string(id) as id FROM skill_group ORDER BY sort_index ASC, name ASC",
        )
        .await
        .map_err(|e| format!("Failed to query skill groups: {}", e))?;
    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(from_db_skill_group).collect())
}

/// Get a single skill by ID
pub async fn get_skill_by_id(state: &DbState, skill_id: &str) -> Result<Option<Skill>, String> {
    let db = state.db();
    let record_id = db_record_id("skill", skill_id);

    let mut result = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to query skill: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.first().map(|r| from_db_skill(r.clone())))
}

/// Create or update a skill
pub async fn upsert_skill(state: &DbState, skill: &Skill) -> Result<String, String> {
    let db = state.db();

    if skill.id.is_empty() {
        // Get max sort_index for new skill
        let mut max_result = db
            .query("SELECT sort_index FROM skill ORDER BY sort_index DESC LIMIT 1")
            .await
            .map_err(|e| format!("Failed to query max sort_index: {}", e))?;
        let max_records: Vec<Value> = max_result.take(0).map_err(|e| e.to_string())?;
        let max_index = max_records
            .first()
            .and_then(|v| v.get("sort_index"))
            .and_then(|v| v.as_i64())
            .unwrap_or(-1) as i32;

        // Create new skill with sort_index = max + 1
        let mut new_skill = skill.clone();
        new_skill.sort_index = max_index + 1;
        let payload = to_clean_skill_payload(&new_skill);

        let id = db_new_id();
        let record_id = db_record_id("skill", &id);
        db.query(&format!("CREATE {} CONTENT $data", record_id))
            .bind(("data", payload))
            .await
            .map_err(|e| format!("Failed to create skill: {}", e))?;
        Ok(id)
    } else {
        // Update existing skill
        let payload = to_clean_skill_payload(skill);
        let record_id = db_record_id("skill", &skill.id);
        db.query(&format!("UPDATE {} CONTENT $data", record_id))
            .bind(("data", payload))
            .await
            .map_err(|e| format!("Failed to update skill: {}", e))?;
        Ok(skill.id.clone())
    }
}

/// Get a skill by name
pub async fn get_skill_by_name(state: &DbState, name: &str) -> Result<Option<Skill>, String> {
    let db = state.db();
    let name_owned = name.to_string();

    let mut result = db
        .query("SELECT *, type::string(id) as id FROM skill WHERE name = $name LIMIT 1")
        .bind(("name", name_owned))
        .await
        .map_err(|e| format!("Failed to query skill by name: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.first().map(|r| from_db_skill(r.clone())))
}

/// Delete a skill
pub async fn delete_skill(state: &DbState, skill_id: &str) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("skill", skill_id);

    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|e| format!("Failed to delete skill: {}", e))?;

    Ok(())
}

/// Update user-managed metadata for a skill without touching content timestamps or sync state.
pub async fn update_skill_metadata(
    state: &DbState,
    skill_id: &str,
    group_id: Option<String>,
    user_note: Option<String>,
) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("skill", skill_id);

    db.query(&format!(
        "UPDATE {} SET group_id = $group_id, user_group = $user_group, user_note = $user_note",
        record_id
    ))
    .bind(("group_id", group_id.clone()))
    .bind(("user_group", group_name_for_id(state, group_id).await?))
    .bind(("user_note", user_note))
    .await
    .map_err(|e| format!("Failed to update skill metadata: {}", e))?;

    Ok(())
}

/// Update user-managed group for multiple skills.
pub async fn update_skills_group(
    state: &DbState,
    skill_ids: &[String],
    group_id: Option<String>,
) -> Result<(), String> {
    let db = state.db();

    let user_group = group_name_for_id(state, group_id.clone()).await?;
    for skill_id in skill_ids {
        let record_id = db_record_id("skill", skill_id);
        db.query(&format!(
            "UPDATE {} SET group_id = $group_id, user_group = $user_group",
            record_id
        ))
        .bind(("group_id", group_id.clone()))
        .bind(("user_group", user_group.clone()))
        .await
        .map_err(|e| format!("Failed to update skill group: {}", e))?;
    }

    Ok(())
}

async fn group_name_for_id(
    state: &DbState,
    group_id: Option<String>,
) -> Result<Option<String>, String> {
    let Some(group_id) = group_id else {
        return Ok(None);
    };
    let groups = get_skill_groups_without_migration(state).await?;
    Ok(groups
        .into_iter()
        .find(|group| group.id == group_id)
        .map(|group| group.name))
}

pub async fn set_skill_management_enabled(
    state: &DbState,
    skill_id: &str,
    enabled: bool,
) -> Result<Vec<String>, String> {
    let db = state.db();
    let Some(skill) = get_skill_by_id(state, skill_id).await? else {
        return Err(format!("Skill not found: {}", skill_id));
    };
    let record_id = db_record_id("skill", skill_id);
    if enabled {
        db.query(&format!(
            "UPDATE {} SET management_enabled = true",
            record_id
        ))
        .await
        .map_err(|e| format!("Failed to enable skill: {}", e))?;
        return Ok(skill.disabled_previous_tools);
    }

    let previous_tools = if skill.enabled_tools.is_empty() {
        skill.disabled_previous_tools.clone()
    } else {
        skill.enabled_tools.clone()
    };
    db.query(&format!(
        "UPDATE {} SET management_enabled = false, disabled_previous_tools = $previous_tools, enabled_tools = [], sync_details = {{}}",
        record_id
    ))
    .bind(("previous_tools", previous_tools.clone()))
    .await
    .map_err(|e| format!("Failed to disable skill: {}", e))?;
    Ok(previous_tools)
}

pub async fn record_disabled_previous_tools(
    state: &DbState,
    skill_id: &str,
    previous_tools: Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("skill", skill_id);
    db.query(&format!(
        "UPDATE {} SET disabled_previous_tools = $previous_tools",
        record_id
    ))
    .bind(("previous_tools", previous_tools))
    .await
    .map_err(|e| format!("Failed to record previous tools: {}", e))?;
    Ok(())
}

pub async fn disable_skill_with_previous_tools(
    state: &DbState,
    skill_id: &str,
    previous_tools: Vec<String>,
) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("skill", skill_id);
    db.query(&format!(
        "UPDATE {} SET management_enabled = false, disabled_previous_tools = $previous_tools, enabled_tools = [], sync_details = {{}}",
        record_id
    ))
    .bind(("previous_tools", previous_tools))
    .await
    .map_err(|e| format!("Failed to disable skill: {}", e))?;
    Ok(())
}

// ==================== Skill sync_details operations ====================

/// Get all targets for a specific skill (parsed from sync_details)
pub async fn get_skill_targets(
    state: &DbState,
    skill_id: &str,
) -> Result<Vec<SkillTarget>, String> {
    let skill = get_skill_by_id(state, skill_id).await?;
    Ok(skill.map(|s| parse_sync_details(&s)).unwrap_or_default())
}

/// Get a skill target (from sync_details for specified tool)
pub async fn get_skill_target(
    state: &DbState,
    skill_id: &str,
    tool: &str,
) -> Result<Option<SkillTarget>, String> {
    let skill = get_skill_by_id(state, skill_id).await?;
    Ok(skill.and_then(|s| get_sync_detail(&s.sync_details, tool)))
}

/// Upsert a skill target (update sync_details tool entry)
pub async fn upsert_skill_target(
    state: &DbState,
    skill_id: &str,
    target: &SkillTarget,
) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("skill", skill_id);

    // Get existing skill
    let mut result = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| e.to_string())?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    let skill = records
        .first()
        .map(|r| from_db_skill(r.clone()))
        .ok_or_else(|| format!("Skill not found: {}", skill_id))?;

    // Update sync_details
    let new_sync_details = set_sync_detail(&skill.sync_details, &target.tool, target);

    // Update enabled_tools
    let mut enabled_tools = skill.enabled_tools.clone();
    if !enabled_tools.contains(&target.tool) {
        enabled_tools.push(target.tool.clone());
    }

    // Save updates (don't update updated_at to preserve sort order)
    db.query(&format!(
        "UPDATE {} SET sync_details = $sync_details, enabled_tools = $enabled_tools",
        record_id
    ))
    .bind(("sync_details", new_sync_details))
    .bind(("enabled_tools", enabled_tools))
    .await
    .map_err(|e| format!("Failed to update skill target: {}", e))?;

    Ok(())
}

/// Delete a skill target (remove tool entry from sync_details)
pub async fn delete_skill_target(
    state: &DbState,
    skill_id: &str,
    tool: &str,
) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("skill", skill_id);
    let tool_owned = tool.to_string();

    // Get existing skill
    let mut result = db
        .query(&format!(
            "SELECT *, type::string(id) as id FROM {} LIMIT 1",
            record_id
        ))
        .await
        .map_err(|e| e.to_string())?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    let Some(skill) = records.first().map(|r| from_db_skill(r.clone())) else {
        return Ok(()); // Skill not found, nothing to delete
    };

    // Update sync_details
    let new_sync_details = remove_sync_detail(&skill.sync_details, &tool_owned);

    // Update enabled_tools
    let enabled_tools: Vec<String> = skill
        .enabled_tools
        .into_iter()
        .filter(|t| t != &tool_owned)
        .collect();

    // Save updates (don't update updated_at to preserve sort order)
    db.query(&format!(
        "UPDATE {} SET sync_details = $sync_details, enabled_tools = $enabled_tools",
        record_id
    ))
    .bind(("sync_details", new_sync_details))
    .bind(("enabled_tools", enabled_tools))
    .await
    .map_err(|e| format!("Failed to delete skill target: {}", e))?;

    Ok(())
}

// ==================== SkillRepo CRUD ====================

/// Get all skill repos
pub async fn get_skill_repos(state: &DbState) -> Result<Vec<SkillRepo>, String> {
    let db = state.db();

    let mut result = db
        .query("SELECT *, type::string(id) as id FROM skill_repo ORDER BY owner ASC, name ASC")
        .await
        .map_err(|e| format!("Failed to query skill repos: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(from_db_skill_repo).collect())
}

/// Save a skill repo
pub async fn save_skill_repo(state: &DbState, repo: &SkillRepo) -> Result<(), String> {
    let db = state.db();
    let payload = to_skill_repo_payload(repo);

    // Use owner/name as ID
    let id = format!("{}/{}", repo.owner, repo.name);
    let record_id = db_record_id("skill_repo", &id);

    db.query(&format!("UPSERT {} CONTENT $data", record_id))
        .bind(("data", payload))
        .await
        .map_err(|e| format!("Failed to save skill repo: {}", e))?;

    Ok(())
}

/// Delete a skill repo
pub async fn delete_skill_repo(state: &DbState, owner: &str, name: &str) -> Result<(), String> {
    let db = state.db();
    let id = format!("{}/{}", owner, name);
    let record_id = db_record_id("skill_repo", &id);

    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|e| format!("Failed to delete skill repo: {}", e))?;

    Ok(())
}

// ==================== SkillPreferences CRUD ====================

/// Get skill preferences (singleton record)
pub async fn get_skill_preferences(state: &DbState) -> Result<SkillPreferences, String> {
    let db = state.db();

    let mut result = db
        .query("SELECT *, type::string(id) as id FROM skill_preferences:`default` LIMIT 1")
        .await
        .map_err(|e| format!("Failed to query skill preferences: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;

    if let Some(record) = records.first() {
        Ok(from_db_skill_preferences(record.clone()))
    } else {
        Ok(SkillPreferences::default())
    }
}

/// Save skill preferences (singleton record)
pub async fn save_skill_preferences(
    state: &DbState,
    prefs: &SkillPreferences,
) -> Result<(), String> {
    let db = state.db();
    let payload = to_skill_preferences_payload(prefs);

    db.query("UPSERT skill_preferences:`default` CONTENT $data")
        .bind(("data", payload))
        .await
        .map_err(|e| format!("Failed to save skill preferences: {}", e))?;

    Ok(())
}

// ==================== Settings (compatibility layer using preferences) ====================

/// Get setting value (read from skill_preferences)
pub async fn get_setting(state: &DbState, key: &str) -> Result<Option<String>, String> {
    let prefs = get_skill_preferences(state).await?;

    let value = match key {
        "central_repo_path" => Some(prefs.central_repo_path),
        "preferred_tools_v1" => prefs
            .preferred_tools
            .map(|v| serde_json::to_string(&v).unwrap_or_default()),
        "default_view_mode" => Some(prefs.default_view_mode),
        "installed_tools_v1" => prefs
            .installed_tools
            .map(|v| serde_json::to_string(&v).unwrap_or_default()),
        "git_cache_cleanup_days" => Some(prefs.git_cache_cleanup_days.to_string()),
        "git_cache_ttl_secs" => Some(prefs.git_cache_ttl_secs.to_string()),
        "show_skills_in_tray" => Some(prefs.show_skills_in_tray.to_string()),
        _ => None,
    };

    Ok(value)
}

/// Set setting value (update skill_preferences)
pub async fn set_setting(state: &DbState, key: &str, value: &str) -> Result<(), String> {
    let mut prefs = get_skill_preferences(state).await?;
    prefs.updated_at = now_ms();

    match key {
        "central_repo_path" => prefs.central_repo_path = value.to_string(),
        "preferred_tools_v1" => {
            prefs.preferred_tools = serde_json::from_str(value).ok();
        }
        "default_view_mode" => {
            prefs.default_view_mode = match value {
                "grouped" => "grouped".to_string(),
                _ => "flat".to_string(),
            };
        }
        "installed_tools_v1" => {
            prefs.installed_tools = serde_json::from_str(value).ok();
        }
        "git_cache_cleanup_days" => {
            prefs.git_cache_cleanup_days = value.parse().unwrap_or(30);
        }
        "git_cache_ttl_secs" => {
            prefs.git_cache_ttl_secs = value.parse().unwrap_or(60);
        }
        "show_skills_in_tray" => {
            prefs.show_skills_in_tray = value == "true";
        }
        _ => return Err(format!("Unknown setting key: {}", key)),
    };

    save_skill_preferences(state, &prefs).await
}

/// Get all skill target paths for filtering
pub async fn list_all_skill_target_paths(state: &DbState) -> Result<Vec<(String, String)>, String> {
    let skills = get_managed_skills(state).await?;

    let mut paths = Vec::new();
    for skill in skills {
        for target in parse_sync_details(&skill) {
            paths.push((target.tool, target.target_path));
        }
    }

    Ok(paths)
}

// ==================== CustomTool CRUD ====================

// ==================== Skill Reorder ====================

/// Reorder skills by updating sort_index for each skill
pub async fn reorder_skills(state: &DbState, ids: &[String]) -> Result<(), String> {
    let db = state.db();

    for (index, id) in ids.iter().enumerate() {
        let record_id = db_record_id("skill", id);
        db.query(&format!("UPDATE {} SET sort_index = $index", record_id))
            .bind(("index", index as i32))
            .await
            .map_err(|e| format!("Failed to reorder skills: {}", e))?;
    }

    Ok(())
}

pub async fn update_skill_sort_index(
    state: &DbState,
    skill_id: &str,
    sort_index: i32,
) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("skill", skill_id);
    db.query(&format!("UPDATE {} SET sort_index = $index", record_id))
        .bind(("index", sort_index))
        .await
        .map_err(|e| format!("Failed to update skill sort index: {}", e))?;
    Ok(())
}

// ==================== CustomTool CRUD (cont.) ====================

/// Get all custom tools that support Skills
/// Delegates to the shared tools module and converts to skills CustomTool type
pub async fn get_custom_tools(state: &DbState) -> Result<Vec<CustomTool>, String> {
    let tools = crate::coding::tools::custom_store::get_skills_custom_tools(state).await?;
    Ok(tools.into_iter().map(CustomTool::from).collect())
}

/// Save a custom tool (preserving MCP fields if they exist)
pub async fn save_custom_tool(state: &DbState, tool: &CustomTool) -> Result<(), String> {
    // Use the shared tools module function that preserves MCP fields
    crate::coding::tools::custom_store::save_custom_tool_skills_fields(
        state,
        &tool.key,
        &tool.display_name,
        Some(tool.relative_skills_dir.clone()),
        Some(tool.relative_detect_dir.clone()),
        tool.force_copy,
        tool.created_at,
    )
    .await
}

/// Delete a custom tool
pub async fn delete_custom_tool(state: &DbState, key: &str) -> Result<(), String> {
    let db = state.db();
    let record_id = db_record_id("custom_tool", key);

    db.query(&format!("DELETE {}", record_id))
        .await
        .map_err(|e| format!("Failed to delete custom tool: {}", e))?;

    Ok(())
}
