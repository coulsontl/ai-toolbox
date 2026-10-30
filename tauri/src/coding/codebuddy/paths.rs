//! Path resolution and prompt/config file locations for CodeBuddy Code.
//!
//! The CLI's own resolver is `CODEBUDDY_CONFIG_DIR` → `~/.codebuddy`. The
//! desktop product sets both `CODEBUDDY_CONFIG_DIR` and `WORKBUDDY_CONFIG_DIR`
//! when it embeds the engine, so a user who exported either expects the CLI
//! module to land on the same root.

use std::path::{Path, PathBuf};

use super::constants::{
    CODEBUDDY_CONFIG_DIR_ENV_KEY, CODEBUDDY_DEFAULT_CONFIG_DIR_NAME, CODEBUDDY_MODELS_FILE,
    WORKBUDDY_CONFIG_DIR_ENV_KEY,
};
use crate::coding::open_code::shell_env;

fn get_home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "Failed to get home directory".to_string())
}

pub fn default_config_dir() -> Result<PathBuf, String> {
    Ok(get_home_dir()?.join(CODEBUDDY_DEFAULT_CONFIG_DIR_NAME))
}

fn read_env_override(var_name: &str) -> Option<PathBuf> {
    std::env::var(var_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Resolution without the database: env → shell → default, with the source
/// label the page shows next to the path.
pub fn resolve_path_without_db() -> (PathBuf, String) {
    for var_name in [CODEBUDDY_CONFIG_DIR_ENV_KEY, WORKBUDDY_CONFIG_DIR_ENV_KEY] {
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
        default_config_dir().unwrap_or_else(|_| PathBuf::from("~/.codebuddy")),
        "default".to_string(),
    )
}

/// Whether `path` looks like a CodeBuddy config root.
///
/// Only used to normalise a hand-typed root: the CLI keeps `models.json`,
/// `settings.json`, `.mcp.json` and `projects/` directly under the root.
pub fn looks_like_config_root(path: &Path) -> bool {
    path.join("projects").is_dir()
        || path.join("settings.json").is_file()
        || path.join(CODEBUDDY_MODELS_FILE).is_file()
        || path.join(".mcp.json").is_file()
        || path.join("mcp.json").is_file()
}

/// Given a selection that may be one level too deep, return the config root.
///
/// Same shape as the desktop normalisation: an `app` child only collapses when
/// its parent actually looks like a config root.
pub fn normalize_selected_root(path: &Path) -> PathBuf {
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
    fn a_settings_file_makes_a_directory_a_config_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join(".codebuddy");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::write(root.join("settings.json"), "{}").expect("settings");

        assert!(looks_like_config_root(&root));
        assert!(!looks_like_config_root(dir.path()));
    }

    #[test]
    fn an_app_subdirectory_collapses_only_under_a_real_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join(".codebuddy");
        std::fs::create_dir_all(root.join("app")).expect("app dir");
        std::fs::create_dir_all(root.join("projects")).expect("projects dir");

        assert_eq!(normalize_selected_root(&root.join("app")), root);

        let unrelated = dir.path().join("elsewhere").join("app");
        std::fs::create_dir_all(&unrelated).expect("unrelated app dir");
        assert_eq!(normalize_selected_root(&unrelated), unrelated);
    }

    #[test]
    fn the_default_root_name_is_the_cli_directory() {
        let default = default_config_dir().expect("default");
        assert_eq!(
            default.file_name().and_then(|name| name.to_str()),
            Some(CODEBUDDY_DEFAULT_CONFIG_DIR_NAME)
        );
    }
}
