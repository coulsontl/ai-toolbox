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

fn resolve_special_mcp_config_path(tool: &RuntimeTool) -> Option<PathBuf> {
    match tool.key.as_str() {
        "opencode" => crate::coding::mcp::opencode_path::get_opencode_mcp_config_path_sync(),
        "github_copilot_intellij" => resolve_github_copilot_intellij_mcp_path(),
        _ => None,
    }
}

/// Check if a runtime tool is installed by checking its detect directory
pub fn is_tool_installed(tool: &RuntimeTool) -> bool {
    // Custom tools are always considered installed
    if tool.is_custom {
        return true;
    }

    // Some MCP targets have OS-specific paths that cannot be represented by a
    // single static storage string.
    if matches!(tool.key.as_str(), "opencode" | "github_copilot_intellij") {
        if let Some(config_path) = resolve_mcp_config_path(tool) {
            if config_path.exists() {
                return true;
            }
            if let Some(parent) = config_path.parent() {
                if parent.exists() {
                    return true;
                }
            }
        }
        // Fall through to default detection
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
    if let Some(config_path) = resolve_special_mcp_config_path(tool) {
        return Some(config_path);
    }

    // Use path_utils to resolve (handles ~/ and %APPDATA%/ paths)
    tool.mcp_config_path
        .as_ref()
        .and_then(|path| resolve_storage_path(path))
}

pub fn resolve_mcp_config_path_with_db(
    db: &crate::db::SqliteDbState,
    tool: &RuntimeTool,
) -> Option<PathBuf> {
    match tool.key.as_str() {
        "opencode" | "claude_code" | "codex" | "grok" | "openclaw" | "pi" => {
            crate::coding::runtime_location::get_tool_mcp_config_path_sync(db, &tool.key)
                .or_else(|| resolve_mcp_config_path(tool))
        }
        _ => resolve_mcp_config_path(tool),
    }
}

pub async fn resolve_mcp_config_path_with_db_async(
    db: &crate::db::SqliteDbState,
    tool: &RuntimeTool,
) -> Option<PathBuf> {
    match tool.key.as_str() {
        "opencode" | "claude_code" | "codex" | "grok" | "openclaw" | "pi" => {
            crate::coding::runtime_location::get_tool_mcp_config_path_async(db, &tool.key)
                .await
                .or_else(|| resolve_mcp_config_path(tool))
        }
        _ => resolve_mcp_config_path(tool),
    }
}

pub fn resolve_skills_path_with_db(
    db: &crate::db::SqliteDbState,
    tool: &RuntimeTool,
) -> Option<PathBuf> {
    match tool.key.as_str() {
        "opencode" | "claude_code" | "codex" | "grok" | "openclaw" | "pi" => {
            crate::coding::runtime_location::get_tool_skills_path_sync(db, &tool.key)
                .or_else(|| resolve_skills_path(tool))
        }
        _ => resolve_skills_path(tool),
    }
}

pub async fn resolve_skills_path_with_db_async(
    db: &crate::db::SqliteDbState,
    tool: &RuntimeTool,
) -> Option<PathBuf> {
    match tool.key.as_str() {
        "opencode" | "claude_code" | "codex" | "grok" | "openclaw" | "pi" => {
            crate::coding::runtime_location::get_tool_skills_path_async(db, &tool.key)
                .await
                .or_else(|| resolve_skills_path(tool))
        }
        _ => resolve_skills_path(tool),
    }
}

pub fn is_tool_installed_with_db(db: &crate::db::SqliteDbState, tool: &RuntimeTool) -> bool {
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
    db: &crate::db::SqliteDbState,
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
    get_all_runtime_tools(custom_tools)
        .into_iter()
        .find(|t| t.key == key)
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
    db: &crate::db::SqliteDbState,
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
    db: &crate::db::SqliteDbState,
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
