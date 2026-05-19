use serde_json::Value;
use std::collections::HashSet;

use crate::coding::db_id::db_new_id;
use crate::db::helpers::{
    db_delete, db_delete_all, db_get, db_list, db_max_i64, db_put, db_query_by_field,
};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;

use super::adapter::{
    from_db_skill, from_db_skill_group, from_db_skill_preferences, from_db_skill_repo,
    get_sync_detail, parse_sync_details, remove_sync_detail, set_sync_detail,
    to_clean_skill_payload, to_skill_group_payload, to_skill_preferences_payload,
    to_skill_repo_payload,
};
use super::tool_adapters::CustomTool;
use super::types::{now_ms, Skill, SkillGroupRecord, SkillPreferences, SkillRepo, SkillTarget};

const SKILL_PREFERENCES_ID: &str = "default";

fn skill_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::single(OrderField::json_integer(
        "sort_index",
        OrderDirection::Asc,
    )?))
}

fn skill_group_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::json_text("name", OrderDirection::Asc)?,
    ]))
}

fn skill_repo_order() -> Result<OrderSpec, String> {
    Ok(OrderSpec::new(vec![
        OrderField::json_text("owner", OrderDirection::Asc)?,
        OrderField::json_text("name", OrderDirection::Asc)?,
    ]))
}

fn sqlite_get_managed_skills(sqlite_state: &SqliteDbState) -> Result<Vec<Skill>, String> {
    let order = skill_order()?;
    sqlite_state.with_conn(|conn| {
        Ok(db_list(conn, DbTable::Skill, Some(&order))?
            .into_iter()
            .map(from_db_skill)
            .collect())
    })
}

fn sqlite_get_skill_groups(sqlite_state: &SqliteDbState) -> Result<Vec<SkillGroupRecord>, String> {
    let order = skill_group_order()?;
    sqlite_state.with_conn(|conn| {
        Ok(db_list(conn, DbTable::SkillGroup, Some(&order))?
            .into_iter()
            .map(from_db_skill_group)
            .collect())
    })
}

fn sqlite_get_skill_by_id(
    sqlite_state: &SqliteDbState,
    skill_id: &str,
) -> Result<Option<Skill>, String> {
    sqlite_state.with_conn(|conn| Ok(db_get(conn, DbTable::Skill, skill_id)?.map(from_db_skill)))
}

fn sqlite_put_skill(sqlite_state: &SqliteDbState, id: &str, skill: &Skill) -> Result<(), String> {
    sqlite_state.with_conn(|conn| db_put(conn, DbTable::Skill, id, &to_clean_skill_payload(skill)))
}

fn sqlite_patch_skill(
    sqlite_state: &SqliteDbState,
    skill_id: &str,
    update: impl FnOnce(&mut Skill),
) -> Result<Option<Skill>, String> {
    let Some(mut skill) = sqlite_get_skill_by_id(sqlite_state, skill_id)? else {
        return Ok(None);
    };
    update(&mut skill);
    sqlite_put_skill(sqlite_state, skill_id, &skill)?;
    Ok(Some(skill))
}

fn sqlite_put_skill_group(
    sqlite_state: &SqliteDbState,
    id: &str,
    group: &SkillGroupRecord,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| {
        db_put(
            conn,
            DbTable::SkillGroup,
            id,
            &to_skill_group_payload(group),
        )
    })
}

fn sqlite_put_skill_repo(
    sqlite_state: &SqliteDbState,
    id: &str,
    repo: &SkillRepo,
) -> Result<(), String> {
    sqlite_state
        .with_conn(|conn| db_put(conn, DbTable::SkillRepo, id, &to_skill_repo_payload(repo)))
}

fn sqlite_put_skill_preferences(
    sqlite_state: &SqliteDbState,
    prefs: &SkillPreferences,
) -> Result<(), String> {
    sqlite_state.with_conn(|conn| {
        db_put(
            conn,
            DbTable::SkillPreferences,
            SKILL_PREFERENCES_ID,
            &to_skill_preferences_payload(prefs),
        )
    })
}

// ==================== Skill CRUD ====================

/// Get all managed skills
pub async fn get_managed_skills(state: &SqliteDbState) -> Result<Vec<Skill>, String> {
    migrate_legacy_skill_groups(state).await?;
    clear_dangling_skill_groups(state).await?;
    sqlite_get_managed_skills(state)
}

pub async fn get_skill_groups(state: &SqliteDbState) -> Result<Vec<SkillGroupRecord>, String> {
    migrate_legacy_skill_groups(state).await?;
    clear_dangling_skill_groups(state).await?;
    sqlite_get_skill_groups(state)
}

pub async fn save_skill_group(
    state: &SqliteDbState,
    group: &SkillGroupRecord,
) -> Result<String, String> {
    let id = if group.id.is_empty() {
        db_new_id()
    } else {
        group.id.clone()
    };
    sqlite_put_skill_group(state, &id, group)?;
    Ok(id)
}

pub async fn delete_skill_group(state: &SqliteDbState, group_id: &str) -> Result<(), String> {
    let skills = sqlite_get_managed_skills(state)?;
    for mut skill in skills
        .into_iter()
        .filter(|skill| skill.group_id.as_deref() == Some(group_id))
    {
        skill.group_id = None;
        skill.user_group = None;
        let skill_id = skill.id.clone();
        sqlite_put_skill(state, &skill_id, &skill)?;
    }
    state.with_conn(|conn| db_delete(conn, DbTable::SkillGroup, group_id).map(|_| ()))?;
    Ok(())
}

pub async fn replace_skill_groups(
    state: &SqliteDbState,
    groups: &[SkillGroupRecord],
) -> Result<Vec<SkillGroupRecord>, String> {
    state.with_conn(|conn| db_delete_all(conn, DbTable::SkillGroup).map(|_| ()))?;
    let mut saved = Vec::new();
    for group in groups {
        let id = save_skill_group(state, group).await?;
        let mut group = group.clone();
        group.id = id;
        saved.push(group);
    }
    Ok(saved)
}

pub async fn migrate_legacy_skill_groups(state: &SqliteDbState) -> Result<(), String> {
    let records = sqlite_get_managed_skills(state)?
        .into_iter()
        .filter(|skill| {
            skill.group_id.is_none()
                && skill
                    .user_group
                    .as_ref()
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    if records.is_empty() {
        return Ok(());
    }

    let existing = get_skill_groups_without_migration(state).await?;
    let mut by_name: std::collections::HashMap<String, String> = existing
        .into_iter()
        .map(|group| (group.name.trim().to_lowercase(), group.id))
        .collect();
    let mut next_index = by_name.len() as i32;

    for mut skill in records {
        let Some(group_name) = skill
            .user_group
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
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
                name: group_name,
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
        skill.group_id = Some(group_id);
        let skill_id = skill.id.clone();
        sqlite_put_skill(state, &skill_id, &skill)?;
    }
    Ok(())
}

async fn clear_dangling_skill_groups(state: &SqliteDbState) -> Result<(), String> {
    let groups = get_skill_groups_without_migration(state).await?;
    let valid_group_ids: HashSet<String> = groups.into_iter().map(|group| group.id).collect();
    let skills = sqlite_get_managed_skills(state)?;
    for mut skill in skills.into_iter().filter(|skill| skill.group_id.is_some()) {
        let Some(group_id) = skill.group_id.as_ref() else {
            continue;
        };
        if valid_group_ids.contains(group_id) {
            continue;
        }
        skill.group_id = None;
        skill.user_group = None;
        let skill_id = skill.id.clone();
        sqlite_put_skill(state, &skill_id, &skill)?;
    }
    Ok(())
}

async fn get_skill_groups_without_migration(
    state: &SqliteDbState,
) -> Result<Vec<SkillGroupRecord>, String> {
    sqlite_get_skill_groups(state)
}

/// Get a single skill by ID
pub async fn get_skill_by_id(
    state: &SqliteDbState,
    skill_id: &str,
) -> Result<Option<Skill>, String> {
    sqlite_get_skill_by_id(state, skill_id)
}

/// Create or update a skill
pub async fn upsert_skill(state: &SqliteDbState, skill: &Skill) -> Result<String, String> {
    let id = if skill.id.is_empty() {
        let max_index = state.with_conn(|conn| {
            db_max_i64(conn, DbTable::Skill, &JsonFieldPath::new("sort_index")?)
        })?;
        let mut new_skill = skill.clone();
        new_skill.sort_index = max_index.unwrap_or(-1) as i32 + 1;
        let id = db_new_id();
        sqlite_put_skill(state, &id, &new_skill)?;
        id
    } else {
        sqlite_put_skill(state, &skill.id, skill)?;
        skill.id.clone()
    };
    Ok(id)
}

/// Get a skill by name
pub async fn get_skill_by_name(state: &SqliteDbState, name: &str) -> Result<Option<Skill>, String> {
    let name_value = Value::String(name.to_string());
    state.with_conn(|conn| {
        Ok(db_query_by_field(
            conn,
            DbTable::Skill,
            &JsonFieldPath::new("name")?,
            &name_value,
            Some(&skill_order()?),
            Some(1),
        )?
        .into_iter()
        .next()
        .map(from_db_skill))
    })
}

/// Delete a skill
pub async fn delete_skill(state: &SqliteDbState, skill_id: &str) -> Result<(), String> {
    state.with_conn(|conn| db_delete(conn, DbTable::Skill, skill_id).map(|_| ()))?;
    Ok(())
}

/// Update user-managed metadata for a skill without touching content timestamps or sync state.
pub async fn update_skill_metadata(
    state: &SqliteDbState,
    skill_id: &str,
    group_id: Option<String>,
    user_note: Option<String>,
) -> Result<(), String> {
    let user_group = group_name_for_id(state, group_id.clone()).await?;
    sqlite_patch_skill(state, skill_id, |skill| {
        skill.group_id = group_id.clone();
        skill.user_group = user_group.clone();
        skill.user_note = user_note.clone();
    })?;
    Ok(())
}

/// Update user-managed group for multiple skills.
pub async fn update_skills_group(
    state: &SqliteDbState,
    skill_ids: &[String],
    group_id: Option<String>,
) -> Result<(), String> {
    let user_group = group_name_for_id(state, group_id.clone()).await?;
    for skill_id in skill_ids {
        sqlite_patch_skill(state, skill_id, |skill| {
            skill.group_id = group_id.clone();
            skill.user_group = user_group.clone();
        })?;
    }
    Ok(())
}

async fn group_name_for_id(
    state: &SqliteDbState,
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
    state: &SqliteDbState,
    skill_id: &str,
    enabled: bool,
) -> Result<Vec<String>, String> {
    let Some(skill) = get_skill_by_id(state, skill_id).await? else {
        return Err(format!("Skill not found: {}", skill_id));
    };
    if enabled {
        sqlite_patch_skill(state, skill_id, |skill| {
            skill.management_enabled = true;
        })?
        .ok_or_else(|| format!("Skill not found: {}", skill_id))?;
        return Ok(skill.disabled_previous_tools);
    }

    let previous_tools = if skill.enabled_tools.is_empty() {
        skill.disabled_previous_tools.clone()
    } else {
        skill.enabled_tools.clone()
    };
    sqlite_patch_skill(state, skill_id, |skill| {
        skill.management_enabled = false;
        skill.disabled_previous_tools = previous_tools.clone();
        skill.enabled_tools = Vec::new();
        skill.sync_details = Some(Value::Object(serde_json::Map::new()));
    })?
    .ok_or_else(|| format!("Skill not found: {}", skill_id))?;
    Ok(previous_tools)
}

pub async fn record_disabled_previous_tools(
    state: &SqliteDbState,
    skill_id: &str,
    previous_tools: Vec<String>,
) -> Result<(), String> {
    sqlite_patch_skill(state, skill_id, |skill| {
        skill.disabled_previous_tools = previous_tools.clone();
    })?;
    Ok(())
}

pub async fn disable_skill_with_previous_tools(
    state: &SqliteDbState,
    skill_id: &str,
    previous_tools: Vec<String>,
) -> Result<(), String> {
    sqlite_patch_skill(state, skill_id, |skill| {
        skill.management_enabled = false;
        skill.disabled_previous_tools = previous_tools.clone();
        skill.enabled_tools = Vec::new();
        skill.sync_details = Some(Value::Object(serde_json::Map::new()));
    })?;
    Ok(())
}

// ==================== Skill sync_details operations ====================

/// Get all targets for a specific skill (parsed from sync_details)
pub async fn get_skill_targets(
    state: &SqliteDbState,
    skill_id: &str,
) -> Result<Vec<SkillTarget>, String> {
    let skill = get_skill_by_id(state, skill_id).await?;
    Ok(skill.map(|s| parse_sync_details(&s)).unwrap_or_default())
}

/// Get a skill target (from sync_details for specified tool)
pub async fn get_skill_target(
    state: &SqliteDbState,
    skill_id: &str,
    tool: &str,
) -> Result<Option<SkillTarget>, String> {
    let skill = get_skill_by_id(state, skill_id).await?;
    Ok(skill.and_then(|s| get_sync_detail(&s.sync_details, tool)))
}

/// Upsert a skill target (update sync_details tool entry)
pub async fn upsert_skill_target(
    state: &SqliteDbState,
    skill_id: &str,
    target: &SkillTarget,
) -> Result<(), String> {
    let target = target.clone();
    sqlite_patch_skill(state, skill_id, |skill| {
        skill.sync_details = Some(set_sync_detail(&skill.sync_details, &target.tool, &target));
        if !skill.enabled_tools.contains(&target.tool) {
            skill.enabled_tools.push(target.tool.clone());
        }
    })?
    .ok_or_else(|| format!("Skill not found: {}", skill_id))?;
    Ok(())
}

/// Delete a skill target (remove tool entry from sync_details)
pub async fn delete_skill_target(
    state: &SqliteDbState,
    skill_id: &str,
    tool: &str,
) -> Result<(), String> {
    let tool_owned = tool.to_string();
    sqlite_patch_skill(state, skill_id, |skill| {
        skill.sync_details = Some(remove_sync_detail(&skill.sync_details, &tool_owned));
        skill.enabled_tools = skill
            .enabled_tools
            .iter()
            .filter(|value| *value != &tool_owned)
            .cloned()
            .collect();
    })?;
    Ok(())
}

// ==================== SkillRepo CRUD ====================

/// Get all skill repos
pub async fn get_skill_repos(state: &SqliteDbState) -> Result<Vec<SkillRepo>, String> {
    let order = skill_repo_order()?;
    state.with_conn(|conn| {
        Ok(db_list(conn, DbTable::SkillRepo, Some(&order))?
            .into_iter()
            .map(from_db_skill_repo)
            .collect())
    })
}

/// Save a skill repo
pub async fn save_skill_repo(state: &SqliteDbState, repo: &SkillRepo) -> Result<(), String> {
    let id = format!("{}/{}", repo.owner, repo.name);
    sqlite_put_skill_repo(state, &id, repo)?;
    Ok(())
}

/// Delete a skill repo
pub async fn delete_skill_repo(
    state: &SqliteDbState,
    owner: &str,
    name: &str,
) -> Result<(), String> {
    let id = format!("{}/{}", owner, name);
    state.with_conn(|conn| db_delete(conn, DbTable::SkillRepo, &id).map(|_| ()))?;
    Ok(())
}

// ==================== SkillPreferences CRUD ====================

/// Get skill preferences (singleton record)
pub async fn get_skill_preferences(state: &SqliteDbState) -> Result<SkillPreferences, String> {
    state.with_conn(|conn| {
        Ok(
            db_get(conn, DbTable::SkillPreferences, SKILL_PREFERENCES_ID)?
                .map(from_db_skill_preferences)
                .unwrap_or_default(),
        )
    })
}

/// Save skill preferences (singleton record)
pub async fn save_skill_preferences(
    state: &SqliteDbState,
    prefs: &SkillPreferences,
) -> Result<(), String> {
    sqlite_put_skill_preferences(state, prefs)?;
    Ok(())
}

// ==================== Settings (compatibility layer using preferences) ====================

/// Get setting value (read from skill_preferences)
pub async fn get_setting(state: &SqliteDbState, key: &str) -> Result<Option<String>, String> {
    let prefs = get_skill_preferences(state).await?;

    let value = match key {
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
pub async fn set_setting(state: &SqliteDbState, key: &str, value: &str) -> Result<(), String> {
    let mut prefs = get_skill_preferences(state).await?;
    prefs.updated_at = now_ms();

    match key {
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
pub async fn list_all_skill_target_paths(
    state: &SqliteDbState,
) -> Result<Vec<(String, String)>, String> {
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
pub async fn reorder_skills(state: &SqliteDbState, ids: &[String]) -> Result<(), String> {
    for (index, id) in ids.iter().enumerate() {
        sqlite_patch_skill(state, id, |skill| {
            skill.sort_index = index as i32;
        })?;
    }
    Ok(())
}

pub async fn update_skill_sort_index(
    state: &SqliteDbState,
    skill_id: &str,
    sort_index: i32,
) -> Result<(), String> {
    sqlite_patch_skill(state, skill_id, |skill| {
        skill.sort_index = sort_index;
    })?;
    Ok(())
}

// ==================== CustomTool CRUD (cont.) ====================

/// Get all custom tools that support Skills
/// Delegates to the shared tools module and converts to skills CustomTool type
pub async fn get_custom_tools(state: &SqliteDbState) -> Result<Vec<CustomTool>, String> {
    let tools = crate::coding::tools::custom_store::get_skills_custom_tools(state).await?;
    Ok(tools.into_iter().map(CustomTool::from).collect())
}

/// Save a custom tool (preserving MCP fields if they exist)
pub async fn save_custom_tool(state: &SqliteDbState, tool: &CustomTool) -> Result<(), String> {
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
pub async fn delete_custom_tool(state: &SqliteDbState, key: &str) -> Result<(), String> {
    crate::coding::tools::custom_store::delete_custom_tool(state, key).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn create_test_db() -> (tempfile::TempDir, SqliteDbState) {
        let temp_dir = tempfile::tempdir().expect("create temp db dir");
        let db_path = temp_dir.path().join("ai-toolbox.db");
        let state = SqliteDbState::open(db_path).expect("open sqlite test db");
        (temp_dir, state)
    }

    fn load_raw_preferences(state: &SqliteDbState) -> Value {
        state
            .with_conn(|conn| db_get(conn, DbTable::SkillPreferences, SKILL_PREFERENCES_ID))
            .expect("query preferences")
            .expect("preferences record")
    }

    #[tokio::test]
    async fn non_path_setting_write_does_not_create_central_repo_path() {
        let (_temp, state) = create_test_db();

        set_setting(&state, "default_view_mode", "grouped")
            .await
            .expect("save default view mode");

        let record = load_raw_preferences(&state);
        assert_eq!(
            record.get("default_view_mode").and_then(Value::as_str),
            Some("grouped")
        );
        assert!(record.get("central_repo_path").is_none());
    }

    #[tokio::test]
    async fn non_path_setting_write_removes_legacy_central_repo_path() {
        let (_temp, state) = create_test_db();
        let seed = serde_json::json!({
            "central_repo_path": "/Users/ralph/.skills",
            "default_view_mode": "flat"
        });
        state
            .with_conn(|conn| db_put(conn, DbTable::SkillPreferences, SKILL_PREFERENCES_ID, &seed))
            .expect("seed legacy preferences path");

        set_setting(&state, "preferred_tools_v1", "[\"codex\"]")
            .await
            .expect("save preferred tools");

        let record = load_raw_preferences(&state);
        assert!(record.get("central_repo_path").is_none());
        assert_eq!(
            record
                .get("preferred_tools")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_str),
            Some("codex")
        );
    }
}
