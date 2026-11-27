use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::central_repo::{
    ensure_central_repo, resolve_central_repo_path_without_app, to_relative_central_path,
};
use super::commands::sync_skill_to_tool_record;
use super::skill_store;
use super::sync_engine::remove_path;
use super::tool_adapters::{resolve_runtime_skills_path_with_state_async, runtime_adapter_by_key};
use super::types::{now_ms, Skill};
use crate::SqliteDbState;

pub const BUILTIN_AI_TOOLBOX_SKILL_ID: &str = "builtin:ai-toolbox";
pub const BUILTIN_AI_TOOLBOX_SKILL_NAME: &str = "ai-toolbox";
pub const BUILTIN_SOURCE_TYPE: &str = "builtin";
pub const BUILTIN_AI_TOOLBOX_VERSION: &str = env!("CARGO_PKG_VERSION");

const DEFAULT_BUILTIN_TOOL: &str = "shared_agents";
const BUILTIN_SKILL_MD: &str =
    include_str!("../../../resources/builtin_skills/ai-toolbox/SKILL.md");
const BUILTIN_PLUGINS_REFERENCE: &str =
    include_str!("../../../resources/builtin_skills/ai-toolbox/references/plugins.md");

pub const BUILTIN_SKILL_DELETE_ERROR: &str =
    "Built-in skills cannot be deleted. Uncheck the tools that should stop syncing this skill instead.";
pub const BUILTIN_SKILL_EDIT_ERROR: &str =
    "Built-in skills follow the application version and cannot be edited.";
pub const BUILTIN_SKILL_DISABLE_ERROR: &str =
    "Built-in skills are always managed. Uncheck the tools that should stop syncing this skill instead.";
pub const BUILTIN_SKILL_OVERWRITE_ERROR: &str =
    "Built-in skills are updated by AI Toolbox and cannot be overwritten by an install.";

/// A Skill is built in when its row explicitly says so, or when it owns the
/// reserved built-in id. The id check keeps the protection working even for
/// rows written before `source_type = "builtin"` existed.
pub fn is_builtin_skill(skill: &Skill) -> bool {
    skill.source_type == BUILTIN_SOURCE_TYPE || skill.id == BUILTIN_AI_TOOLBOX_SKILL_ID
}

pub fn guard_builtin_delete(skill: &Skill) -> Result<(), String> {
    if is_builtin_skill(skill) {
        return Err(BUILTIN_SKILL_DELETE_ERROR.to_string());
    }
    Ok(())
}

pub fn guard_builtin_metadata_edit(skill: &Skill) -> Result<(), String> {
    if is_builtin_skill(skill) {
        return Err(BUILTIN_SKILL_EDIT_ERROR.to_string());
    }
    Ok(())
}

pub fn guard_builtin_disable(skill: &Skill) -> Result<(), String> {
    if is_builtin_skill(skill) {
        return Err(BUILTIN_SKILL_DISABLE_ERROR.to_string());
    }
    Ok(())
}

pub fn guard_builtin_overwrite(skill: &Skill) -> Result<(), String> {
    if is_builtin_skill(skill) {
        return Err(BUILTIN_SKILL_OVERWRITE_ERROR.to_string());
    }
    Ok(())
}

fn embedded_files() -> [(&'static str, &'static str); 2] {
    [
        ("SKILL.md", BUILTIN_SKILL_MD),
        ("references/plugins.md", BUILTIN_PLUGINS_REFERENCE),
    ]
}

/// Stable hash over the app version and every embedded file. A version bump
/// changes the hash even when the Markdown itself did not change, so the
/// stored revision always tracks the binary that wrote it. Tool copies are
/// refreshed separately, based on the actual embedded file contents.
pub fn builtin_content_hash() -> String {
    let mut hasher = Sha256::new();
    hasher.update(BUILTIN_AI_TOOLBOX_VERSION.as_bytes());
    for (relative_path, content) in embedded_files() {
        hasher.update(relative_path.as_bytes());
        hasher.update([0]);
        hasher.update(content.as_bytes());
        hasher.update([0]);
    }
    hex::encode(hasher.finalize())
}

fn builtin_skill_dir(central_dir: &Path) -> PathBuf {
    central_dir.join(BUILTIN_AI_TOOLBOX_SKILL_NAME)
}

fn builtin_dir_matches(dir: &Path) -> bool {
    for (relative_path, expected) in embedded_files() {
        match fs::read(dir.join(relative_path)) {
            Ok(actual) if actual == expected.as_bytes() => {}
            _ => return false,
        }
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name != "SKILL.md" && name != "references" {
            return false;
        }
    }

    let references_dir = dir.join("references");
    if !references_dir.is_dir() {
        return false;
    }
    let Ok(entries) = fs::read_dir(&references_dir) else {
        return false;
    };
    for entry in entries.flatten() {
        if entry.file_name() != "plugins.md" {
            return false;
        }
    }

    true
}

fn write_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Built-in skill path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create {}: {}", parent.display(), error))?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("builtin");
    let temp_path = parent.join(format!(".{}.tmp-{}", file_name, std::process::id()));
    fs::write(&temp_path, content)
        .map_err(|error| format!("Failed to write {}: {}", temp_path.display(), error))?;

    // Windows `rename` refuses to replace an existing file.
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&temp_path, path)
        .map_err(|error| format!("Failed to replace {}: {}", path.display(), error))?;
    Ok(())
}

fn remove_dir_entries_except(dir: &Path, allowed: &[&str]) -> Result<(), String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Failed to read {}: {}", dir.display(), error)),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if allowed
            .iter()
            .any(|allowed_name| allowed_name.as_bytes() == name.as_bytes())
        {
            continue;
        }
        remove_path(&entry.path().to_string_lossy()).map_err(|error| {
            format!(
                "Failed to remove stale {}: {}",
                entry.path().display(),
                error
            )
        })?;
    }
    Ok(())
}

fn write_builtin_skill_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir.join("references"))
        .map_err(|error| format!("Failed to create {}: {}", dir.display(), error))?;

    for (relative_path, content) in embedded_files() {
        write_atomic(&dir.join(relative_path), content.as_bytes())?;
    }

    remove_dir_entries_except(dir, &["SKILL.md", "references"])?;
    remove_dir_entries_except(&dir.join("references"), &["plugins.md"])?;
    Ok(())
}

fn new_builtin_skill(central_path: String, content_hash: String) -> Skill {
    let now = now_ms();
    Skill {
        id: BUILTIN_AI_TOOLBOX_SKILL_ID.to_string(),
        name: BUILTIN_AI_TOOLBOX_SKILL_NAME.to_string(),
        source_type: BUILTIN_SOURCE_TYPE.to_string(),
        source_ref: Some(BUILTIN_AI_TOOLBOX_SKILL_NAME.to_string()),
        source_revision: Some(BUILTIN_AI_TOOLBOX_VERSION.to_string()),
        central_path,
        content_hash: Some(content_hash),
        created_at: now,
        updated_at: now,
        last_sync_at: None,
        status: "ok".to_string(),
        sort_index: -1,
        user_group: None,
        group_id: None,
        user_note: None,
        management_enabled: true,
        disabled_previous_tools: Vec::new(),
        tags: Vec::new(),
        enabled_tools: Vec::new(),
        sync_details: None,
    }
}

async fn resync_enabled_builtin_tools(db: &SqliteDbState, skill: &Skill, source_path: &Path) {
    if skill.enabled_tools.is_empty() {
        return;
    }
    let custom_tools = skill_store::get_custom_tools(db).await.unwrap_or_default();
    for tool in &skill.enabled_tools {
        if let Err(error) =
            sync_skill_to_tool_record(db, skill, tool, source_path, true, &custom_tools).await
        {
            log::warn!(
                "Failed to refresh built-in Skill '{}' on tool '{}': {}",
                skill.name,
                tool,
                error
            );
        }
    }
}

/// Install the built-in Skill into the Universal (`shared_agents`) directory
/// the first time it is created. Creating the tool directory is deliberate:
/// Universal is the cross-tool public directory, and a first-run machine does
/// not necessarily have `~/.agents` yet. A pre-existing foreign directory is
/// never overwritten; the Skill simply stays unsynced for the user to decide.
async fn install_default_builtin_tool(
    db: &SqliteDbState,
    skill: &Skill,
    source_path: &Path,
) -> Result<(), String> {
    let adapter = runtime_adapter_by_key(DEFAULT_BUILTIN_TOOL, &[])
        .ok_or_else(|| format!("Unknown built-in tool: {}", DEFAULT_BUILTIN_TOOL))?;
    let skills_dir = resolve_runtime_skills_path_with_state_async(db, &adapter)
        .await
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&skills_dir).map_err(|error| {
        format!(
            "Failed to create Universal Skills directory {}: {}",
            skills_dir.display(),
            error
        )
    })?;

    let custom_tools = skill_store::get_custom_tools(db).await.unwrap_or_default();
    sync_skill_to_tool_record(
        db,
        skill,
        DEFAULT_BUILTIN_TOOL,
        source_path,
        false,
        &custom_tools,
    )
    .await
    .map(|_| ())
}

async fn ensure_builtin_skills_internal(
    db: &SqliteDbState,
    install_default_tool: bool,
    resync_tools: bool,
) -> Result<(), String> {
    let central_dir = resolve_central_repo_path_without_app(db)?;
    ensure_central_repo(&central_dir).map_err(|error| error.to_string())?;

    let skill_dir = builtin_skill_dir(&central_dir);
    let relative_central_path = to_relative_central_path(&skill_dir, &central_dir);
    let embedded_hash = builtin_content_hash();
    let version = BUILTIN_AI_TOOLBOX_VERSION.to_string();

    if let Some(mut skill) = skill_store::get_skill_by_id(db, BUILTIN_AI_TOOLBOX_SKILL_ID).await? {
        let files_were_stale = !builtin_dir_matches(&skill_dir);
        if files_were_stale {
            write_builtin_skill_dir(&skill_dir)?;
        }

        let mut db_changed = false;
        if skill.name != BUILTIN_AI_TOOLBOX_SKILL_NAME {
            skill.name = BUILTIN_AI_TOOLBOX_SKILL_NAME.to_string();
            db_changed = true;
        }
        if skill.source_type != BUILTIN_SOURCE_TYPE {
            skill.source_type = BUILTIN_SOURCE_TYPE.to_string();
            db_changed = true;
        }
        if skill.source_ref.as_deref() != Some(BUILTIN_AI_TOOLBOX_SKILL_NAME) {
            skill.source_ref = Some(BUILTIN_AI_TOOLBOX_SKILL_NAME.to_string());
            db_changed = true;
        }
        if skill.source_revision.as_deref() != Some(version.as_str()) {
            skill.source_revision = Some(version.clone());
            db_changed = true;
        }
        if skill.content_hash.as_deref() != Some(embedded_hash.as_str()) {
            skill.content_hash = Some(embedded_hash.clone());
            db_changed = true;
        }
        if skill.central_path != relative_central_path {
            skill.central_path = relative_central_path;
            db_changed = true;
        }
        if !skill.management_enabled {
            skill.management_enabled = true;
            db_changed = true;
        }
        if skill.status != "ok" {
            skill.status = "ok".to_string();
            db_changed = true;
        }
        if files_were_stale || db_changed {
            skill.updated_at = now_ms();
            skill_store::upsert_skill(db, &skill).await?;
        }

        if files_were_stale && resync_tools {
            resync_enabled_builtin_tools(db, &skill, &skill_dir).await;
        }
        return Ok(());
    }

    // A user-managed Skill with the same name owns the directory name. Never
    // clobber it; the built-in row is skipped until the user renames/removes it.
    if skill_store::get_skill_by_name_ci(db, BUILTIN_AI_TOOLBOX_SKILL_NAME)
        .await?
        .is_some()
    {
        log::warn!(
            "Skipped creating built-in Skill '{}' because another managed Skill already uses that name",
            BUILTIN_AI_TOOLBOX_SKILL_NAME
        );
        return Ok(());
    }

    if skill_dir.exists() && !builtin_dir_matches(&skill_dir) {
        log::warn!(
            "Skipped creating built-in Skill '{}' because {} already exists and is not the embedded version",
            BUILTIN_AI_TOOLBOX_SKILL_NAME,
            skill_dir.display()
        );
        return Ok(());
    }

    write_builtin_skill_dir(&skill_dir)?;
    let skill = new_builtin_skill(relative_central_path, embedded_hash);
    skill_store::upsert_skill(db, &skill).await?;

    if install_default_tool {
        if let Err(error) = install_default_builtin_tool(db, &skill, &skill_dir).await {
            log::warn!(
                "Built-in Skill '{}' was created but could not be synced to Universal: {}",
                skill.name,
                error
            );
        }
    }

    Ok(())
}

/// Ensure the versioned built-in `ai-toolbox` Skill exists in the central repo
/// and matches the content embedded in the running binary.
///
/// Idempotent: it is safe to call at startup and from the managed-skills query.
pub async fn ensure_builtin_skills(db: &SqliteDbState) -> Result<(), String> {
    ensure_builtin_skills_internal(db, true, true).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::skills::central_repo::save_central_repo_path;

    fn create_test_db() -> (tempfile::TempDir, SqliteDbState) {
        let temp_dir = tempfile::tempdir().expect("create temp db dir");
        let db_path = temp_dir.path().join("ai-toolbox.db");
        let state = SqliteDbState::open(db_path).expect("open sqlite test db");
        (temp_dir, state)
    }

    async fn create_test_env() -> (tempfile::TempDir, SqliteDbState, PathBuf) {
        let (temp_dir, state) = create_test_db();
        let central_dir = temp_dir.path().join("skills");
        save_central_repo_path(&state, &central_dir)
            .await
            .expect("save central repo path");
        (temp_dir, state, central_dir)
    }

    fn user_skill(name: &str) -> Skill {
        Skill {
            id: format!("user:{}", name),
            name: name.to_string(),
            source_type: "local".to_string(),
            source_ref: Some(format!("/tmp/{}", name)),
            source_revision: None,
            central_path: name.to_string(),
            content_hash: None,
            created_at: 1,
            updated_at: 1,
            last_sync_at: None,
            status: "ok".to_string(),
            sort_index: 0,
            user_group: None,
            group_id: None,
            user_note: None,
            management_enabled: true,
            disabled_previous_tools: Vec::new(),
            tags: Vec::new(),
            enabled_tools: Vec::new(),
            sync_details: None,
        }
    }

    #[tokio::test]
    async fn first_install_writes_embedded_files_and_creates_record() {
        let (_temp, state, central_dir) = create_test_env().await;

        ensure_builtin_skills_internal(&state, false, false)
            .await
            .expect("ensure built-in skill");

        let skill = skill_store::get_skill_by_id(&state, BUILTIN_AI_TOOLBOX_SKILL_ID)
            .await
            .expect("query built-in skill")
            .expect("built-in skill record");
        assert_eq!(skill.name, BUILTIN_AI_TOOLBOX_SKILL_NAME);
        assert_eq!(skill.source_type, BUILTIN_SOURCE_TYPE);
        assert_eq!(
            skill.source_revision.as_deref(),
            Some(BUILTIN_AI_TOOLBOX_VERSION)
        );
        assert_eq!(
            skill.content_hash.as_deref(),
            Some(builtin_content_hash().as_str())
        );
        assert!(skill.management_enabled);
        assert_eq!(skill.sort_index, -1);
        assert_eq!(skill.central_path, BUILTIN_AI_TOOLBOX_SKILL_NAME);

        let skill_dir = central_dir.join(BUILTIN_AI_TOOLBOX_SKILL_NAME);
        assert_eq!(
            fs::read_to_string(skill_dir.join("SKILL.md")).expect("read SKILL.md"),
            BUILTIN_SKILL_MD
        );
        assert_eq!(
            fs::read_to_string(skill_dir.join("references/plugins.md"))
                .expect("read plugins reference"),
            BUILTIN_PLUGINS_REFERENCE
        );

        let managed = skill_store::get_managed_skills(&state)
            .await
            .expect("list managed skills");
        assert_eq!(managed.len(), 1, "ensure must be idempotent");

        ensure_builtin_skills_internal(&state, false, false)
            .await
            .expect("ensure again");
        assert_eq!(
            skill_store::get_managed_skills(&state)
                .await
                .expect("list managed skills")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn upgrade_restores_files_and_never_resurrects_removed_shared_agents() {
        let (_temp, state, central_dir) = create_test_env().await;
        ensure_builtin_skills_internal(&state, false, false)
            .await
            .expect("initial ensure");

        let mut skill = skill_store::get_skill_by_id(&state, BUILTIN_AI_TOOLBOX_SKILL_ID)
            .await
            .expect("query built-in skill")
            .expect("built-in skill record");
        skill.source_revision = Some("0.0.0".to_string());
        skill.content_hash = Some("stale-hash".to_string());
        skill.enabled_tools = vec!["claude_code".to_string()];
        skill_store::upsert_skill(&state, &skill)
            .await
            .expect("seed stale built-in record");
        assert_eq!(
            skill_store::get_skill_by_id(&state, BUILTIN_AI_TOOLBOX_SKILL_ID)
                .await
                .expect("query seeded skill")
                .expect("seeded skill record")
                .enabled_tools,
            vec!["claude_code".to_string()],
            "seed must persist enabled_tools"
        );

        let skill_dir = central_dir.join(BUILTIN_AI_TOOLBOX_SKILL_NAME);
        fs::write(skill_dir.join("SKILL.md"), "tampered").expect("tamper SKILL.md");
        fs::write(skill_dir.join("references/stale.md"), "stale").expect("write stale reference");

        ensure_builtin_skills_internal(&state, false, false)
            .await
            .expect("upgrade built-in skill");

        let refreshed = skill_store::get_skill_by_id(&state, BUILTIN_AI_TOOLBOX_SKILL_ID)
            .await
            .expect("query refreshed skill")
            .expect("refreshed skill record");
        assert_eq!(
            refreshed.source_revision.as_deref(),
            Some(BUILTIN_AI_TOOLBOX_VERSION)
        );
        assert_eq!(
            refreshed.content_hash.as_deref(),
            Some(builtin_content_hash().as_str())
        );
        assert_eq!(
            refreshed.enabled_tools,
            vec!["claude_code".to_string()],
            "ensure must not re-add a tool the user removed"
        );
        assert!(!refreshed
            .enabled_tools
            .contains(&"shared_agents".to_string()));
        assert_eq!(
            fs::read_to_string(skill_dir.join("SKILL.md")).expect("read restored SKILL.md"),
            BUILTIN_SKILL_MD
        );
        assert_eq!(
            fs::read_to_string(skill_dir.join("references/plugins.md"))
                .expect("read restored reference"),
            BUILTIN_PLUGINS_REFERENCE
        );
        assert!(
            !skill_dir.join("references/stale.md").exists(),
            "stale files must be cleaned up"
        );
    }

    #[tokio::test]
    async fn existing_user_skill_with_same_name_is_not_overwritten() {
        let (_temp, state, central_dir) = create_test_env().await;
        let existing = user_skill(BUILTIN_AI_TOOLBOX_SKILL_NAME);
        skill_store::upsert_skill(&state, &existing)
            .await
            .expect("seed user skill");

        ensure_builtin_skills_internal(&state, false, false)
            .await
            .expect("ensure with name conflict");

        assert!(
            skill_store::get_skill_by_id(&state, BUILTIN_AI_TOOLBOX_SKILL_ID)
                .await
                .expect("query built-in skill")
                .is_none(),
            "built-in record must not be created over a user Skill"
        );
        assert!(
            !central_dir.join(BUILTIN_AI_TOOLBOX_SKILL_NAME).exists(),
            "conflicting central directory must not be written"
        );
        let user = skill_store::get_skill_by_id(&state, &existing.id)
            .await
            .expect("query user skill")
            .expect("user skill survives");
        assert_eq!(user.source_type, "local");
    }

    #[test]
    fn builtin_guards_reject_delete_edit_disable_and_overwrite() {
        let mut builtin = user_skill(BUILTIN_AI_TOOLBOX_SKILL_NAME);
        builtin.id = BUILTIN_AI_TOOLBOX_SKILL_ID.to_string();
        builtin.source_type = BUILTIN_SOURCE_TYPE.to_string();

        assert!(is_builtin_skill(&builtin));
        assert!(guard_builtin_delete(&builtin).is_err());
        assert!(guard_builtin_metadata_edit(&builtin).is_err());
        assert!(guard_builtin_disable(&builtin).is_err());
        assert!(guard_builtin_overwrite(&builtin).is_err());

        let normal = user_skill("normal-skill");
        assert!(!is_builtin_skill(&normal));
        assert!(guard_builtin_delete(&normal).is_ok());
        assert!(guard_builtin_metadata_edit(&normal).is_ok());
        assert!(guard_builtin_disable(&normal).is_ok());
        assert!(guard_builtin_overwrite(&normal).is_ok());
    }
}
