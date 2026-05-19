use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::plugin_types::CodexPluginWorkspaceRoot;
use crate::db::helpers::{db_get, db_put};
use crate::db::schema::DbTable;

const MARKETPLACE_RELATIVE_PATH: &str = ".agents/plugins/marketplace.json";
const WORKSPACE_SETTINGS_ID: &str = "settings";

fn normalize_workspace_root_path(raw_path: &str) -> Result<String, String> {
    let trimmed_path = raw_path.trim();
    if trimmed_path.is_empty() {
        return Err("Workspace directory is required".to_string());
    }

    let workspace_path = PathBuf::from(trimmed_path);
    if !workspace_path.is_absolute() {
        return Err(format!(
            "Workspace directory must be an absolute path: {trimmed_path}"
        ));
    }

    let metadata = fs::metadata(&workspace_path)
        .map_err(|error| format!("Failed to read workspace directory {trimmed_path}: {error}"))?;
    if !metadata.is_dir() {
        return Err(format!("Workspace path is not a directory: {trimmed_path}"));
    }

    Ok(trimmed_path.to_string())
}

fn path_strings_equal(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn find_git_repo_root(start_path: &Path) -> Option<PathBuf> {
    let mut current_path = Some(start_path);
    while let Some(path) = current_path {
        if path.join(".git").exists() {
            return Some(path.to_path_buf());
        }
        current_path = path.parent();
    }
    None
}

fn resolve_workspace_marketplace_path(
    workspace_path: &Path,
) -> Result<(PathBuf, Option<PathBuf>, String), String> {
    let direct_marketplace_path = workspace_path.join(MARKETPLACE_RELATIVE_PATH);
    if direct_marketplace_path.is_file() {
        return Ok((direct_marketplace_path, None, "direct".to_string()));
    }

    if let Some(repo_root) = find_git_repo_root(workspace_path) {
        let repo_marketplace_path = repo_root.join(MARKETPLACE_RELATIVE_PATH);
        if repo_marketplace_path.is_file() {
            return Ok((
                repo_marketplace_path,
                Some(repo_root),
                "gitRepo".to_string(),
            ));
        }
    }

    Err(format!(
        "No marketplace.json found under {} or its Git repo root",
        workspace_path.display()
    ))
}

fn from_db_value_workspace_root_paths(value: Value) -> Vec<String> {
    value
        .get("workspace_roots")
        .or_else(|| value.get("workspaceRoots"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn to_db_value_workspace_root_paths(workspace_root_paths: &[String]) -> Value {
    json!({
        "workspace_roots": workspace_root_paths,
    })
}

async fn get_stored_workspace_root_paths(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<String>, String> {
    db.with_conn(|conn| {
        Ok(db_get(
            conn,
            DbTable::CodexPluginWorkspaceRoots,
            WORKSPACE_SETTINGS_ID,
        )?
        .map(from_db_value_workspace_root_paths)
        .unwrap_or_default())
    })
}

async fn save_workspace_root_paths(
    db: &crate::db::SqliteDbState,
    workspace_root_paths: &[String],
) -> Result<(), String> {
    let payload = to_db_value_workspace_root_paths(workspace_root_paths);
    db.with_conn(|conn| {
        db_put(
            conn,
            DbTable::CodexPluginWorkspaceRoots,
            WORKSPACE_SETTINGS_ID,
            &payload,
        )
    })
}

pub(crate) fn describe_workspace_root(path: &str) -> CodexPluginWorkspaceRoot {
    let trimmed_path = path.trim();
    let workspace_path = PathBuf::from(trimmed_path);

    if trimmed_path.is_empty() {
        return CodexPluginWorkspaceRoot {
            path: path.to_string(),
            status: "missing".to_string(),
            resolution_source: None,
            resolved_marketplace_path: None,
            resolved_repo_root: None,
            error: Some("Workspace directory is empty".to_string()),
        };
    }

    match fs::metadata(&workspace_path) {
        Ok(metadata) if metadata.is_dir() => {
            match resolve_workspace_marketplace_path(&workspace_path) {
                Ok((marketplace_path, repo_root, resolution_source)) => CodexPluginWorkspaceRoot {
                    path: trimmed_path.to_string(),
                    status: "ready".to_string(),
                    resolution_source: Some(resolution_source),
                    resolved_marketplace_path: Some(marketplace_path.to_string_lossy().to_string()),
                    resolved_repo_root: repo_root.map(|item| item.to_string_lossy().to_string()),
                    error: None,
                },
                Err(error) => CodexPluginWorkspaceRoot {
                    path: trimmed_path.to_string(),
                    status: "missing".to_string(),
                    resolution_source: None,
                    resolved_marketplace_path: None,
                    resolved_repo_root: None,
                    error: Some(error),
                },
            }
        }
        Ok(_) => CodexPluginWorkspaceRoot {
            path: trimmed_path.to_string(),
            status: "missing".to_string(),
            resolution_source: None,
            resolved_marketplace_path: None,
            resolved_repo_root: None,
            error: Some(format!("Workspace path is not a directory: {trimmed_path}")),
        },
        Err(error) => CodexPluginWorkspaceRoot {
            path: trimmed_path.to_string(),
            status: "missing".to_string(),
            resolution_source: None,
            resolved_marketplace_path: None,
            resolved_repo_root: None,
            error: Some(format!(
                "Failed to read workspace directory {trimmed_path}: {error}"
            )),
        },
    }
}

pub async fn list_codex_plugin_workspace_roots(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<CodexPluginWorkspaceRoot>, String> {
    let workspace_root_paths = get_stored_workspace_root_paths(db).await?;
    Ok(workspace_root_paths
        .into_iter()
        .map(|path| describe_workspace_root(&path))
        .collect())
}

pub async fn list_ready_codex_workspace_marketplace_paths(
    db: &crate::db::SqliteDbState,
) -> Result<Vec<PathBuf>, String> {
    let workspace_root_paths = get_stored_workspace_root_paths(db).await?;
    let mut marketplace_paths = Vec::new();

    for workspace_root_path in workspace_root_paths {
        let workspace_status = describe_workspace_root(&workspace_root_path);
        if let Some(marketplace_path) = workspace_status.resolved_marketplace_path {
            let marketplace_path = PathBuf::from(&marketplace_path);
            if !marketplace_paths.iter().any(|existing_path: &PathBuf| {
                path_strings_equal(
                    &existing_path.to_string_lossy(),
                    &marketplace_path.to_string_lossy(),
                )
            }) {
                marketplace_paths.push(marketplace_path);
            }
        }
    }

    Ok(marketplace_paths)
}

pub async fn add_codex_plugin_workspace_root(
    db: &crate::db::SqliteDbState,
    raw_path: &str,
) -> Result<(), String> {
    let normalized_path = normalize_workspace_root_path(raw_path)?;
    let workspace_path = PathBuf::from(&normalized_path);
    let _ = resolve_workspace_marketplace_path(&workspace_path)?;

    let mut workspace_root_paths = get_stored_workspace_root_paths(db).await?;
    if workspace_root_paths
        .iter()
        .any(|existing_path| path_strings_equal(existing_path, &normalized_path))
    {
        return Ok(());
    }

    workspace_root_paths.push(normalized_path);
    save_workspace_root_paths(db, &workspace_root_paths).await
}

pub async fn remove_codex_plugin_workspace_root(
    db: &crate::db::SqliteDbState,
    raw_path: &str,
) -> Result<(), String> {
    let normalized_path = raw_path.trim();
    if normalized_path.is_empty() {
        return Err("Workspace directory is required".to_string());
    }

    let mut workspace_root_paths = get_stored_workspace_root_paths(db).await?;
    workspace_root_paths
        .retain(|existing_path| !path_strings_equal(existing_path, normalized_path));
    save_workspace_root_paths(db, &workspace_root_paths).await
}

#[cfg(test)]
mod tests {
    use super::{describe_workspace_root, resolve_workspace_marketplace_path};
    use tempfile::tempdir;

    #[test]
    fn resolve_workspace_marketplace_path_prefers_direct_marketplace() {
        let temp_dir = tempdir().expect("create temp dir");
        let workspace_root = temp_dir.path().join("workspace");
        std::fs::create_dir_all(workspace_root.join(".agents/plugins"))
            .expect("create marketplace dir");
        std::fs::write(
            workspace_root.join(".agents/plugins/marketplace.json"),
            r#"{"name":"demo","plugins":[]}"#,
        )
        .expect("write marketplace");

        let (marketplace_path, repo_root, resolution_source) =
            resolve_workspace_marketplace_path(&workspace_root)
                .expect("resolve direct marketplace");

        assert_eq!(
            marketplace_path,
            workspace_root.join(".agents/plugins/marketplace.json")
        );
        assert_eq!(repo_root, None);
        assert_eq!(resolution_source, "direct");
    }

    #[test]
    fn describe_workspace_root_resolves_git_repo_marketplace() {
        let temp_dir = tempdir().expect("create temp dir");
        let repo_root = temp_dir.path().join("repo");
        let workspace_root = repo_root.join("nested/project");
        std::fs::create_dir_all(repo_root.join(".git")).expect("create git dir");
        std::fs::create_dir_all(repo_root.join(".agents/plugins")).expect("create marketplace dir");
        std::fs::create_dir_all(&workspace_root).expect("create workspace dir");
        std::fs::write(
            repo_root.join(".agents/plugins/marketplace.json"),
            r#"{"name":"demo","plugins":[]}"#,
        )
        .expect("write marketplace");

        let workspace_status = describe_workspace_root(&workspace_root.to_string_lossy());

        assert_eq!(workspace_status.status, "ready");
        assert_eq!(
            workspace_status.resolution_source.as_deref(),
            Some("gitRepo")
        );
        assert_eq!(
            workspace_status.resolved_marketplace_path.as_deref(),
            Some(
                repo_root
                    .join(".agents/plugins/marketplace.json")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert_eq!(
            workspace_status.resolved_repo_root.as_deref(),
            Some(repo_root.to_string_lossy().as_ref())
        );
    }
}
