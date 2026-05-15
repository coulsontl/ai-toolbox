use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;
use tauri::Manager;

const CENTRAL_DIR_NAME: &str = "skills";

fn central_repo_path_from_settings_record(record: &Value) -> Option<PathBuf> {
    record
        .get("central_repo_path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

async fn load_authoritative_central_repo_path(
    state: &crate::DbState,
) -> std::result::Result<Option<PathBuf>, String> {
    let db = state.db();
    let mut result = db
        .query("SELECT *, type::string(id) as id FROM skill_settings:`skills` LIMIT 1")
        .await
        .map_err(|e| e.to_string())?;

    let records: Vec<serde_json::Value> = result.take(0).map_err(|e| e.to_string())?;

    if let Some(record) = records.first() {
        return Ok(central_repo_path_from_settings_record(record));
    }
    Ok(None)
}

/// Resolve the central repo path from the authoritative skill_settings record
/// or default to app_data_dir/skills.
pub async fn resolve_central_repo_path<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &crate::DbState,
) -> Result<PathBuf> {
    // Try to get from settings first
    let settings_result = load_authoritative_central_repo_path(state).await;

    if let Ok(Some(path)) = settings_result {
        return Ok(path);
    }

    // Default to app data directory / skills
    let app_data_dir = app
        .path()
        .app_data_dir()
        .context("failed to resolve app data directory")?;
    Ok(app_data_dir.join(CENTRAL_DIR_NAME))
}

/// Save the central repo path to the same authoritative store that the resolver reads.
pub async fn save_central_repo_path(state: &crate::DbState, path: &Path) -> Result<()> {
    let db = state.db();
    let now = super::types::now_ms();

    db.query("UPSERT skill_settings:`skills` MERGE { central_repo_path: $path, updated_at: $now }")
        .bind(("path", path.to_string_lossy().to_string()))
        .bind(("now", now))
        .await
        .map_err(|e| anyhow::anyhow!("failed to save central repo path: {}", e))?;

    Ok(())
}

/// Ensure the central repo directory exists
pub fn ensure_central_repo(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).with_context(|| format!("create {:?}", path))?;
    Ok(())
}

fn is_windows_reserved_name(name: &str) -> bool {
    let upper = name.trim_end_matches([' ', '.']).to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn sanitize_windows_path_segment(segment: &str) -> String {
    let mut sanitized = String::with_capacity(segment.len());

    for ch in segment.chars() {
        let is_invalid = matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            || (ch as u32) < 0x20;
        sanitized.push(if is_invalid { '_' } else { ch });
    }

    let trimmed = sanitized.trim_matches([' ', '.']).to_string();
    let mut normalized = if trimmed.is_empty() {
        "unnamed-skill".to_string()
    } else {
        trimmed
    };

    if is_windows_reserved_name(&normalized) {
        normalized.push('_');
    }

    normalized
}

pub fn skill_storage_dir_name(skill_name: &str) -> String {
    let trimmed = skill_name.trim();
    if trimmed.is_empty() {
        return "unnamed-skill".to_string();
    }

    if cfg!(windows) {
        sanitize_windows_path_segment(trimmed)
    } else {
        trimmed.to_string()
    }
}

/// Convert a central_path to a relative path for database storage.
/// If the path starts with the central repo dir, strip the prefix and store relative.
/// Also handles legacy absolute paths from other platforms.
pub fn to_relative_central_path(absolute_path: &Path, central_dir: &Path) -> String {
    // Try to strip the central repo prefix
    if let Ok(rel) = absolute_path.strip_prefix(central_dir) {
        return rel.to_string_lossy().replace('\\', "/");
    }
    // Already relative or from another platform — extract just the file name
    absolute_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| absolute_path.to_string_lossy().replace('\\', "/"))
}

/// Check if a stored path looks like an absolute path from any platform.
/// On macOS, Rust's Path::is_absolute() won't recognize Windows paths like "C:\..."
fn is_any_platform_absolute(path: &str) -> bool {
    // Unix absolute
    if path.starts_with('/') {
        return true;
    }
    // Windows absolute: e.g. "C:\..." or "C:/..."
    let bytes = path.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    false
}

/// Resolve a stored central_path (relative or legacy absolute) to an absolute path
/// using the current central repo directory.
pub fn resolve_skill_central_path(stored_path: &str, current_central_dir: &Path) -> PathBuf {
    let stored = PathBuf::from(stored_path);

    // If it's a native absolute path and exists, use it directly
    if stored.is_absolute() && stored.exists() {
        return stored;
    }

    // Detect legacy absolute paths from any platform (including cross-platform restores)
    if is_any_platform_absolute(stored_path) {
        // Extract the last path component (skill name) using both separators
        let name = stored_path
            .rsplit(|c| c == '/' || c == '\\')
            .find(|s| !s.is_empty())
            .unwrap_or(stored_path);
        let normalized_name = skill_storage_dir_name(name);
        let normalized_path = current_central_dir.join(&normalized_name);
        if normalized_path.exists() {
            return normalized_path;
        }
        return current_central_dir.join(name);
    }

    // Relative path (new format): resolve against current central dir
    let direct_path = current_central_dir.join(&stored);
    if direct_path.exists() || stored.components().count() > 1 {
        return direct_path;
    }

    let normalized_name = skill_storage_dir_name(stored_path);
    current_central_dir.join(normalized_name)
}

/// Expand ~ and ~/ in paths
pub fn expand_home_path(input: &str) -> Result<PathBuf> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        anyhow::bail!("storage path is empty");
    }
    if trimmed == "~" {
        let home = dirs::home_dir().context("failed to resolve home directory")?;
        return Ok(home);
    }
    if let Some(stripped) = trimmed.strip_prefix("~/") {
        let home = dirs::home_dir().context("failed to resolve home directory")?;
        return Ok(home.join(stripped));
    }
    Ok(PathBuf::from(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DbState;
    use serde_json::json;
    use surrealdb::engine::local::SurrealKv;
    use surrealdb::Surreal;

    async fn create_test_db() -> (tempfile::TempDir, DbState) {
        let temp_dir = tempfile::tempdir().expect("create temp db dir");
        let db_path = temp_dir.path().join("surreal");
        let db = Surreal::new::<SurrealKv>(db_path)
            .await
            .expect("open surreal test db");
        db.use_ns("ai_toolbox")
            .use_db("main")
            .await
            .expect("select surreal test namespace");
        (temp_dir, DbState(db))
    }

    #[test]
    fn settings_record_empty_central_repo_path_is_missing() {
        assert_eq!(central_repo_path_from_settings_record(&json!({})), None);
        assert_eq!(
            central_repo_path_from_settings_record(&json!({ "central_repo_path": "   " })),
            None
        );
    }

    #[test]
    fn settings_record_central_repo_path_is_authoritative() {
        let path = central_repo_path_from_settings_record(&json!({
            "central_repo_path": "/tmp/ai-toolbox-skills",
            "git_cache_cleanup_days": 10,
        }));

        assert_eq!(path, Some(PathBuf::from("/tmp/ai-toolbox-skills")));
    }

    #[test]
    fn settings_record_does_not_read_skill_preferences_shape() {
        let path = central_repo_path_from_settings_record(&json!({
            "preferred_tools": ["codex"],
            "default_view_mode": "grouped",
        }));

        assert_eq!(path, None);
    }

    #[tokio::test]
    async fn saved_central_repo_path_is_read_by_authoritative_resolver_store() {
        let temp = tempfile::tempdir().expect("central temp dir");
        let (_db_temp, state) = create_test_db().await;
        let central_path = temp.path().join("custom-skills");

        save_central_repo_path(&state, &central_path)
            .await
            .expect("save central repo path");

        let resolved = load_authoritative_central_repo_path(&state)
            .await
            .expect("load central repo path");
        assert_eq!(resolved, Some(central_path));
    }

    #[tokio::test]
    async fn legacy_skill_preferences_path_does_not_influence_authoritative_store() {
        let (_db_temp, state) = create_test_db().await;
        let db = state.db();
        db.query(
            "UPSERT skill_preferences:`default` CONTENT { central_repo_path: '/Users/ralph/.skills' }",
        )
        .await
        .expect("seed legacy skill preferences path");

        let resolved = load_authoritative_central_repo_path(&state)
            .await
            .expect("load central repo path");

        assert_eq!(resolved, None);
    }

    #[tokio::test]
    async fn saving_central_repo_path_preserves_other_skill_settings_fields() {
        let temp = tempfile::tempdir().expect("central temp dir");
        let (_db_temp, state) = create_test_db().await;
        let db = state.db();
        db.query(
            "UPSERT skill_settings:`skills` CONTENT { git_cache_cleanup_days: 7, git_cache_ttl_secs: 120 }",
        )
        .await
        .expect("seed existing skill settings");
        let central_path = temp.path().join("custom-skills");

        save_central_repo_path(&state, &central_path)
            .await
            .expect("save central repo path");

        let mut result = db
            .query("SELECT *, type::string(id) as id FROM skill_settings:`skills` LIMIT 1")
            .await
            .expect("query skill settings");
        let records: Vec<Value> = result.take(0).expect("take skill settings");
        let record = records.first().expect("skill settings record");

        assert_eq!(
            record.get("central_repo_path").and_then(Value::as_str),
            Some(central_path.to_string_lossy().as_ref())
        );
        assert_eq!(
            record.get("git_cache_cleanup_days").and_then(Value::as_i64),
            Some(7)
        );
        assert_eq!(
            record.get("git_cache_ttl_secs").and_then(Value::as_i64),
            Some(120)
        );
    }
}
