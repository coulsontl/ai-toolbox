use serde_json::Value;

use crate::DbState;

use super::adapter::{
    from_db_skill, from_db_skill_preferences, from_db_skill_repo, get_sync_detail,
    parse_sync_details, remove_sync_detail, set_sync_detail, to_clean_skill_payload,
    to_skill_preferences_payload, to_skill_repo_payload,
};
use super::types::{now_ms, Skill, SkillPreferences, SkillRepo, SkillTarget};
use super::tool_adapters::CustomTool;

// ==================== Skill CRUD ====================

/// Get all managed skills
pub async fn get_managed_skills(state: &DbState) -> Result<Vec<Skill>, String> {
    let db = state.0.lock().await;

    let mut result = db
        .query("SELECT *, type::string(id) as id FROM skill ORDER BY sort_index ASC")
        .await
        .map_err(|e| format!("Failed to query skills: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(from_db_skill).collect())
}

/// Get a single skill by ID
pub async fn get_skill_by_id(state: &DbState, skill_id: &str) -> Result<Option<Skill>, String> {
    let db = state.0.lock().await;
    let skill_id_owned = skill_id.to_string();

    let mut result = db
        .query(
            "SELECT *, type::string(id) as id FROM skill WHERE id = type::thing('skill', $id) LIMIT 1",
        )
        .bind(("id", skill_id_owned))
        .await
        .map_err(|e| format!("Failed to query skill: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.first().map(|r| from_db_skill(r.clone())))
}

/// Create or update a skill
pub async fn upsert_skill(state: &DbState, skill: &Skill) -> Result<String, String> {
    let db = state.0.lock().await;

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

        let id = uuid::Uuid::new_v4().to_string();
        db.query("CREATE type::thing('skill', $id) CONTENT $data")
            .bind(("id", id.clone()))
            .bind(("data", payload))
            .await
            .map_err(|e| format!("Failed to create skill: {}", e))?;
        Ok(id)
    } else {
        // Update existing skill
        let payload = to_clean_skill_payload(skill);
        let skill_id = skill.id.clone();
        db.query("UPDATE type::thing('skill', $id) CONTENT $data")
            .bind(("id", skill_id.clone()))
            .bind(("data", payload))
            .await
            .map_err(|e| format!("Failed to update skill: {}", e))?;
        Ok(skill.id.clone())
    }
}

/// Get a skill by name
pub async fn get_skill_by_name(state: &DbState, name: &str) -> Result<Option<Skill>, String> {
    let db = state.0.lock().await;
    let name_owned = name.to_string();

    let mut result = db
        .query(
            "SELECT *, type::string(id) as id FROM skill WHERE name = $name LIMIT 1",
        )
        .bind(("name", name_owned))
        .await
        .map_err(|e| format!("Failed to query skill by name: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.first().map(|r| from_db_skill(r.clone())))
}

/// Delete a skill
pub async fn delete_skill(state: &DbState, skill_id: &str) -> Result<(), String> {
    let db = state.0.lock().await;
    let skill_id_owned = skill_id.to_string();

    db.query("DELETE FROM skill WHERE id = type::thing('skill', $id)")
        .bind(("id", skill_id_owned))
        .await
        .map_err(|e| format!("Failed to delete skill: {}", e))?;

    Ok(())
}

// ==================== Skill sync_details operations ====================

/// Get all targets for a specific skill (parsed from sync_details)
pub async fn get_skill_targets(state: &DbState, skill_id: &str) -> Result<Vec<SkillTarget>, String> {
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
    let db = state.0.lock().await;

    // Get existing skill
    let skill_id_owned = skill_id.to_string();
    let mut result = db
        .query(
            "SELECT *, type::string(id) as id FROM skill WHERE id = type::thing('skill', $id) LIMIT 1",
        )
        .bind(("id", skill_id_owned.clone()))
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
    db.query("UPDATE type::thing('skill', $id) SET sync_details = $sync_details, enabled_tools = $enabled_tools")
        .bind(("id", skill_id_owned))
        .bind(("sync_details", new_sync_details))
        .bind(("enabled_tools", enabled_tools))
        .await
        .map_err(|e| format!("Failed to update skill target: {}", e))?;

    Ok(())
}

/// Delete a skill target (remove tool entry from sync_details)
pub async fn delete_skill_target(state: &DbState, skill_id: &str, tool: &str) -> Result<(), String> {
    let db = state.0.lock().await;

    // Get existing skill
    let skill_id_owned = skill_id.to_string();
    let tool_owned = tool.to_string();
    let mut result = db
        .query(
            "SELECT *, type::string(id) as id FROM skill WHERE id = type::thing('skill', $id) LIMIT 1",
        )
        .bind(("id", skill_id_owned.clone()))
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
    db.query("UPDATE type::thing('skill', $id) SET sync_details = $sync_details, enabled_tools = $enabled_tools")
        .bind(("id", skill_id_owned))
        .bind(("sync_details", new_sync_details))
        .bind(("enabled_tools", enabled_tools))
        .await
        .map_err(|e| format!("Failed to delete skill target: {}", e))?;

    Ok(())
}

// ==================== SkillRepo CRUD ====================

/// Get all skill repos
pub async fn get_skill_repos(state: &DbState) -> Result<Vec<SkillRepo>, String> {
    let db = state.0.lock().await;

    let mut result = db
        .query("SELECT *, type::string(id) as id FROM skill_repo ORDER BY owner ASC, name ASC")
        .await
        .map_err(|e| format!("Failed to query skill repos: {}", e))?;

    let records: Vec<Value> = result.take(0).map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(from_db_skill_repo).collect())
}

/// Save a skill repo
pub async fn save_skill_repo(state: &DbState, repo: &SkillRepo) -> Result<(), String> {
    let db = state.0.lock().await;
    let payload = to_skill_repo_payload(repo);

    // Use owner/name as ID
    let id = format!("{}/{}", repo.owner, repo.name);

    db.query("UPSERT type::thing('skill_repo', $id) CONTENT $data")
        .bind(("id", id))
        .bind(("data", payload))
        .await
        .map_err(|e| format!("Failed to save skill repo: {}", e))?;

    Ok(())
}

/// Delete a skill repo
pub async fn delete_skill_repo(state: &DbState, owner: &str, name: &str) -> Result<(), String> {
    let db = state.0.lock().await;
    let id = format!("{}/{}", owner, name);

    db.query("DELETE FROM skill_repo WHERE id = type::thing('skill_repo', $id)")
        .bind(("id", id))
        .await
        .map_err(|e| format!("Failed to delete skill repo: {}", e))?;

    Ok(())
}

// ==================== SkillPreferences CRUD ====================

/// Get skill preferences (singleton record)
pub async fn get_skill_preferences(state: &DbState) -> Result<SkillPreferences, String> {
    let db = state.0.lock().await;

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
pub async fn save_skill_preferences(state: &DbState, prefs: &SkillPreferences) -> Result<(), String> {
    let db = state.0.lock().await;
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
    let db = state.0.lock().await;

    for (index, id) in ids.iter().enumerate() {
        db.query("UPDATE type::thing('skill', $id) SET sort_index = $index")
            .bind(("id", id.clone()))
            .bind(("index", index as i32))
            .await
            .map_err(|e| format!("Failed to reorder skills: {}", e))?;
    }

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
        tool.created_at,
    )
    .await
}

/// Delete a custom tool
pub async fn delete_custom_tool(state: &DbState, key: &str) -> Result<(), String> {
    let db = state.0.lock().await;

    db.query("DELETE FROM custom_tool WHERE id = type::thing('custom_tool', $key)")
        .bind(("key", key.to_string()))
        .await
        .map_err(|e| format!("Failed to delete custom tool: {}", e))?;

    Ok(())
}
