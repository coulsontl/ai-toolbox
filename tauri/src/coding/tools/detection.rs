//! Tool installation detection logic
//!
//! Provides functions to detect whether tools are installed on the system.

use std::path::PathBuf;

use super::builtin::BUILTIN_TOOLS;
use super::path_utils::{resolve_storage_path, to_platform_path};
use super::types::{CustomTool, RuntimeTool, RuntimeToolDto, ToolDetectionDto};

fn resolve_github_copilot_intellij_mcp_path() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        return dirs::config_dir().map(|config_dir| {
            config_dir
                .join("github-copilot")
                .join("intellij")
                .join("mcp.json")
        });
    }

    #[cfg(target_os = "windows")]
    {
        return dirs::data_local_dir().map(|local_data_dir| {
            local_data_dir
                .join("github-copilot")
                .join("intellij")
                .join("mcp.json")
        });
    }

    #[cfg(target_os = "macos")]
    {
        return dirs::config_dir().map(|config_dir| {
            config_dir
                .join("GitHub Copilot")
                .join("intellij")
                .join("mcp.json")
        });
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        None
    }
}

fn resolve_special_mcp_config_paths(tool: &RuntimeTool) -> Option<Vec<PathBuf>> {
    match tool.key.as_str() {
        // OpenCode uses dynamic path resolution
        "opencode" => crate::coding::mcp::opencode_path::get_opencode_mcp_config_path_sync()
            .map(|path| vec![path]),
        // GitHub Copilot should sync to all known plugin MCP locations on the current OS
        "github_copilot" => {
            let mut paths = Vec::new();

            if let Some(vscode_path) = tool
                .mcp_config_path
                .as_ref()
                .and_then(|path| resolve_storage_path(path))
            {
                paths.push(vscode_path);
            }

            if let Some(intellij_path) = resolve_github_copilot_intellij_mcp_path() {
                paths.push(intellij_path);
            }

            Some(paths)
        }
        _ => None,
    }
}

pub fn resolve_mcp_config_paths(tool: &RuntimeTool) -> Vec<PathBuf> {
    if let Some(paths) = resolve_special_mcp_config_paths(tool) {
        return paths;
    }

    tool.mcp_config_path
        .as_ref()
        .and_then(|path| resolve_storage_path(path))
        .into_iter()
        .collect()
}

fn select_preferred_mcp_config_path(paths: Vec<PathBuf>) -> Option<PathBuf> {
    let mut first_path = None;
    let mut parent_existing_path = None;

    for path in paths {
        if first_path.is_none() {
            first_path = Some(path.clone());
        }

        if path.exists() {
            return Some(path);
        }

        if parent_existing_path.is_none()
            && path.parent().map(|parent| parent.exists()).unwrap_or(false)
        {
            parent_existing_path = Some(path);
        }
    }

    parent_existing_path.or(first_path)
}

/// Check if a runtime tool is installed by checking its detect directory
pub fn is_tool_installed(tool: &RuntimeTool) -> bool {
    // Custom tools are always considered installed
    if tool.is_custom {
        return true;
    }

    for config_path in resolve_mcp_config_paths(tool) {
        // Check if the config file or its parent directory exists
        if config_path.exists() {
            return true;
        }
        if let Some(parent) = config_path.parent() {
            if parent.exists() {
                return true;
            }
        }
    }

    if let Some(ref detect_dir) = tool.relative_detect_dir {
        // Use path_utils to resolve the storage path (handles ~/ and %APPDATA%/)
        if let Some(resolved) = resolve_storage_path(detect_dir) {
            return resolved.exists();
        }
    }
    false
}

/// Resolve the skills path for a tool
pub fn resolve_skills_path(tool: &RuntimeTool) -> Option<PathBuf> {
    tool.relative_skills_dir.as_ref().and_then(|dir| {
        // Use path_utils to resolve (handles ~/ and %APPDATA%/ paths)
        resolve_storage_path(dir)
    })
}

/// Resolve the MCP config path for a tool
pub fn resolve_mcp_config_path(tool: &RuntimeTool) -> Option<PathBuf> {
    select_preferred_mcp_config_path(resolve_mcp_config_paths(tool))
}

pub fn resolve_mcp_config_paths_with_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool: &RuntimeTool,
) -> Vec<PathBuf> {
    match tool.key.as_str() {
        "opencode" | "claude_code" | "codex" => {
            crate::coding::runtime_location::get_tool_mcp_config_path_sync(db, &tool.key)
                .map(|path| vec![path])
                .unwrap_or_else(|| resolve_mcp_config_paths(tool))
        }
        _ => resolve_mcp_config_paths(tool),
    }
}

pub fn resolve_mcp_config_path_with_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool: &RuntimeTool,
) -> Option<PathBuf> {
    select_preferred_mcp_config_path(resolve_mcp_config_paths_with_db(db, tool))
}

pub async fn resolve_mcp_config_paths_with_db_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool: &RuntimeTool,
) -> Vec<PathBuf> {
    match tool.key.as_str() {
        "opencode" | "claude_code" | "codex" => {
            crate::coding::runtime_location::get_tool_mcp_config_path_async(db, &tool.key)
                .await
                .map(|path| vec![path])
                .unwrap_or_else(|| resolve_mcp_config_paths(tool))
        }
        _ => resolve_mcp_config_paths(tool),
    }
}

pub async fn resolve_mcp_config_path_with_db_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool: &RuntimeTool,
) -> Option<PathBuf> {
    select_preferred_mcp_config_path(resolve_mcp_config_paths_with_db_async(db, tool).await)
}

pub fn resolve_skills_path_with_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool: &RuntimeTool,
) -> Option<PathBuf> {
    match tool.key.as_str() {
        "opencode" | "claude_code" | "codex" | "openclaw" => {
            crate::coding::runtime_location::get_tool_skills_path_sync(db, &tool.key)
                .or_else(|| resolve_skills_path(tool))
        }
        _ => resolve_skills_path(tool),
    }
}

pub async fn resolve_skills_path_with_db_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool: &RuntimeTool,
) -> Option<PathBuf> {
    match tool.key.as_str() {
        "opencode" | "claude_code" | "codex" | "openclaw" => {
            crate::coding::runtime_location::get_tool_skills_path_async(db, &tool.key)
                .await
                .or_else(|| resolve_skills_path(tool))
        }
        _ => resolve_skills_path(tool),
    }
}

pub fn is_tool_installed_with_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool: &RuntimeTool,
) -> bool {
    if tool.is_custom {
        return true;
    }

    if let Some(path) = resolve_mcp_config_path_with_db(db, tool) {
        if path.exists() {
            return true;
        }
        if let Some(parent) = path.parent() {
            if parent.exists() {
                return true;
            }
        }
    }

    if let Some(path) = resolve_skills_path_with_db(db, tool) {
        if path.exists() {
            return true;
        }
        if let Some(parent) = path.parent() {
            if parent.exists() {
                return true;
            }
        }
    }

    is_tool_installed(tool)
}

pub async fn is_tool_installed_with_db_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool: &RuntimeTool,
) -> bool {
    if tool.is_custom {
        return true;
    }

    if let Some(path) = resolve_mcp_config_path_with_db_async(db, tool).await {
        if path.exists() {
            return true;
        }
        if let Some(parent) = path.parent() {
            if parent.exists() {
                return true;
            }
        }
    }

    if let Some(path) = resolve_skills_path_with_db_async(db, tool).await {
        if path.exists() {
            return true;
        }
        if let Some(parent) = path.parent() {
            if parent.exists() {
                return true;
            }
        }
    }

    is_tool_installed(tool)
}

/// Get all tools (built-in + custom) as RuntimeTool
pub fn get_all_runtime_tools(custom_tools: &[CustomTool]) -> Vec<RuntimeTool> {
    let mut tools: Vec<RuntimeTool> = BUILTIN_TOOLS.iter().map(RuntimeTool::from).collect();

    for custom in custom_tools {
        tools.push(RuntimeTool::from(custom));
    }

    tools
}

/// Get tools that support Skills
pub fn get_skills_runtime_tools(custom_tools: &[CustomTool]) -> Vec<RuntimeTool> {
    get_all_runtime_tools(custom_tools)
        .into_iter()
        .filter(|t| t.relative_skills_dir.is_some())
        .collect()
}

/// Get tools that support MCP
pub fn get_mcp_runtime_tools(custom_tools: &[CustomTool]) -> Vec<RuntimeTool> {
    get_all_runtime_tools(custom_tools)
        .into_iter()
        .filter(|t| t.mcp_config_path.is_some())
        .collect()
}

/// Get installed tools that support Skills
pub fn get_installed_skills_tools(custom_tools: &[CustomTool]) -> Vec<RuntimeTool> {
    get_skills_runtime_tools(custom_tools)
        .into_iter()
        .filter(|t| is_tool_installed(t))
        .collect()
}

/// Get installed tools that support MCP
pub fn get_installed_mcp_tools(custom_tools: &[CustomTool]) -> Vec<RuntimeTool> {
    get_mcp_runtime_tools(custom_tools)
        .into_iter()
        .filter(|t| is_tool_installed(t))
        .collect()
}

/// Find a runtime tool by key
pub fn runtime_tool_by_key(key: &str, custom_tools: &[CustomTool]) -> Option<RuntimeTool> {
    let normalized_key = match key {
        "github_copilot_intellij" => "github_copilot",
        _ => key,
    };

    get_all_runtime_tools(custom_tools)
        .into_iter()
        .find(|t| t.key == normalized_key)
}

/// Convert RuntimeTool to RuntimeToolDto with installation status
pub fn to_runtime_tool_dto(tool: &RuntimeTool) -> RuntimeToolDto {
    let installed = is_tool_installed(tool);
    let skills_path = resolve_skills_path(tool).map(|p| p.to_string_lossy().to_string());

    RuntimeToolDto {
        key: tool.key.clone(),
        display_name: tool.display_name.clone(),
        is_custom: tool.is_custom,
        installed,
        relative_skills_dir: tool.relative_skills_dir.clone(),
        skills_path,
        supports_skills: tool.relative_skills_dir.is_some(),
        mcp_config_path: tool.mcp_config_path.as_ref().map(|p| to_platform_path(p)),
        mcp_config_format: tool.mcp_config_format.clone(),
        mcp_field: tool.mcp_field.clone(),
        supports_mcp: tool.mcp_config_path.is_some(),
    }
}

pub fn to_runtime_tool_dto_with_db(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool: &RuntimeTool,
) -> RuntimeToolDto {
    let installed = is_tool_installed_with_db(db, tool);
    let skills_path =
        resolve_skills_path_with_db(db, tool).map(|p| p.to_string_lossy().to_string());
    let mcp_config_path = resolve_mcp_config_path_with_db(db, tool)
        .map(|p| p.to_string_lossy().to_string())
        .or_else(|| tool.mcp_config_path.as_ref().map(|p| to_platform_path(p)));

    RuntimeToolDto {
        key: tool.key.clone(),
        display_name: tool.display_name.clone(),
        is_custom: tool.is_custom,
        installed,
        relative_skills_dir: tool.relative_skills_dir.clone(),
        skills_path,
        supports_skills: tool.relative_skills_dir.is_some(),
        mcp_config_path,
        mcp_config_format: tool.mcp_config_format.clone(),
        mcp_field: tool.mcp_field.clone(),
        supports_mcp: tool.mcp_config_path.is_some(),
    }
}

pub async fn to_runtime_tool_dto_with_db_async(
    db: &surrealdb::Surreal<surrealdb::engine::local::Db>,
    tool: &RuntimeTool,
) -> RuntimeToolDto {
    let installed = is_tool_installed_with_db_async(db, tool).await;
    let skills_path = resolve_skills_path_with_db_async(db, tool)
        .await
        .map(|p| p.to_string_lossy().to_string());
    let mcp_config_path = resolve_mcp_config_path_with_db_async(db, tool)
        .await
        .map(|p| p.to_string_lossy().to_string())
        .or_else(|| tool.mcp_config_path.as_ref().map(|p| to_platform_path(p)));

    RuntimeToolDto {
        key: tool.key.clone(),
        display_name: tool.display_name.clone(),
        is_custom: tool.is_custom,
        installed,
        relative_skills_dir: tool.relative_skills_dir.clone(),
        skills_path,
        supports_skills: tool.relative_skills_dir.is_some(),
        mcp_config_path,
        mcp_config_format: tool.mcp_config_format.clone(),
        mcp_field: tool.mcp_field.clone(),
        supports_mcp: tool.mcp_config_path.is_some(),
    }
}

/// Get tool detection results
pub fn detect_all_tools(custom_tools: &[CustomTool]) -> Vec<ToolDetectionDto> {
    get_all_runtime_tools(custom_tools)
        .iter()
        .map(|tool| ToolDetectionDto {
            key: tool.key.clone(),
            display_name: tool.display_name.clone(),
            installed: is_tool_installed(tool),
            supports_skills: tool.relative_skills_dir.is_some(),
            supports_mcp: tool.mcp_config_path.is_some(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn github_copilot_runtime_tool() -> RuntimeTool {
        RuntimeTool {
            key: "github_copilot".to_string(),
            display_name: "GitHub Copilot".to_string(),
            is_custom: false,
            relative_skills_dir: Some("~/.copilot/skills".to_string()),
            relative_detect_dir: Some("%APPDATA%/Code".to_string()),
            force_copy: false,
            mcp_config_path: Some("%APPDATA%/Code/User/mcp.json".to_string()),
            mcp_config_format: Some("json".to_string()),
            mcp_field: Some("servers".to_string()),
        }
    }

    #[test]
    fn test_resolve_mcp_config_paths_for_github_copilot_includes_vscode_and_intellij() {
        let paths = resolve_mcp_config_paths(&github_copilot_runtime_tool());
        let path_strings: Vec<String> = paths
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect();

        assert_eq!(path_strings.len(), 2);

        #[cfg(windows)]
        {
            assert!(path_strings.iter().any(|path| path.ends_with("Code\\User\\mcp.json")));
            let local_app_data = dirs::data_local_dir().unwrap();
            let expected_intellij_path = local_app_data
                .join("github-copilot")
                .join("intellij")
                .join("mcp.json")
                .to_string_lossy()
                .to_string();
            assert!(path_strings.iter().any(|path| path == &expected_intellij_path));
        }

        #[cfg(target_os = "macos")]
        {
            assert!(path_strings
                .iter()
                .any(|path| path.ends_with("Code/User/mcp.json")));
            assert!(path_strings.iter().any(|path| {
                path.ends_with("Library/Application Support/GitHub Copilot/intellij/mcp.json")
            }));
        }

        #[cfg(target_os = "linux")]
        {
            assert!(path_strings
                .iter()
                .any(|path| path.ends_with("Code/User/mcp.json")));
            assert!(path_strings
                .iter()
                .any(|path| path.ends_with("github-copilot/intellij/mcp.json")));
        }
    }

    #[test]
    fn test_runtime_tool_by_key_aliases_legacy_github_copilot_intellij_key() {
        let tool = runtime_tool_by_key("github_copilot_intellij", &[]).unwrap();
        assert_eq!(tool.key, "github_copilot");
    }
}

