//! Config-root resolution for WorkBuddy desktop.
//!
//! Precedence mirrors the app's own resolvers (`resolveWorkbuddyConfigDir` /
//! `resolveConfigDir`): `WORKBUDDY_CONFIG_DIR`, then `CODEBUDDY_CONFIG_DIR`,
//! then `~/.workbuddy`. A user-set root in AI Toolbox wins over both, matching
//! every other module in this crate.

use std::path::PathBuf;

use super::constants::{
    CODEBUDDY_CONFIG_DIR_ENV_KEY, WORKBUDDY_CONFIG_DIR_ENV_KEY, WORKBUDDY_DEFAULT_CONFIG_DIR_NAME,
};
use crate::coding::open_code::shell_env;
use crate::coding::workbuddy_family::models_json;

fn get_home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "Failed to get home directory".to_string())
}

pub fn default_config_dir() -> Result<PathBuf, String> {
    Ok(get_home_dir()?.join(WORKBUDDY_DEFAULT_CONFIG_DIR_NAME))
}

fn read_env_override(var_name: &str) -> Option<PathBuf> {
    std::env::var(var_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Resolution without the database: env → shell → default, with the source label
/// the page shows next to the path.
pub fn resolve_path_without_db() -> (PathBuf, String) {
    for var_name in [WORKBUDDY_CONFIG_DIR_ENV_KEY, CODEBUDDY_CONFIG_DIR_ENV_KEY] {
        if let Some(path) = read_env_override(var_name) {
            return (path, "env".to_string());
        }
        if let Some(value) = shell_env::get_env_from_shell_config(var_name) {
            if !value.trim().is_empty() {
                return (PathBuf::from(value), "shell".to_string());
            }
        }
    }
    (
        default_config_dir().unwrap_or_else(|_| PathBuf::from("~/.workbuddy")),
        "default".to_string(),
    )
}

/// Whether `path` looks like a WorkBuddy desktop config root.
///
/// Used only to normalise a hand-typed root: the app keeps `models.json`,
/// `mcp.json`, `projects/` and `workbuddy.db` directly under the root, so a
/// directory that has none of them yet is still accepted (a fresh install may
/// not have created `models.json`). The check exists so a user who picked
/// `~/.workbuddy/app` (the Electron user-data folder) gets pointed back up.
pub fn looks_like_config_root(path: &std::path::Path) -> bool {
    path.join("projects").is_dir()
        || path.join("storage").is_dir()
        || path.join("workbuddy.db").is_file()
        || path.join(models_json::MODELS_FILE_NAME).is_file()
        || path.join("mcp.json").is_file()
}

/// Given a selection that may be one level too deep, return the config root.
///
/// `~/.workbuddy/app` is the Electron `userData` directory; the config root is
/// its parent. Nothing else is rewritten.
pub fn normalize_selected_root(path: &std::path::Path) -> PathBuf {
    if path.file_name().map(|name| name == "app").unwrap_or(false) {
        if let Some(parent) = path.parent() {
            if looks_like_config_root(parent) {
                return parent.to_path_buf();
            }
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_subdirectory_is_rewritten_to_the_config_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join(".workbuddy");
        std::fs::create_dir_all(root.join("app")).expect("app dir");
        std::fs::create_dir_all(root.join("projects")).expect("projects dir");

        assert_eq!(normalize_selected_root(&root.join("app")), root);
    }

    #[test]
    fn an_ordinary_directory_is_left_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("custom-root");
        std::fs::create_dir_all(&root).expect("root");

        assert_eq!(normalize_selected_root(&root), root);
    }

    #[test]
    fn the_app_name_is_only_rewritten_when_its_parent_is_a_config_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let unrelated = dir.path().join("app");
        std::fs::create_dir_all(&unrelated).expect("app dir");

        assert_eq!(
            normalize_selected_root(&unrelated),
            unrelated,
            "a bare `app` directory with no sibling layout must not be rewritten"
        );
    }
}
