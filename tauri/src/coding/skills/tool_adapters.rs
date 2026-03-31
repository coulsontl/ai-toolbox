//! Tool adapters for Skills module
//!
//! This module provides backward-compatible tool adapter functionality for the Skills feature.
//! It wraps the shared tools module and provides Skills-specific types and functions.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};

use crate::coding::tools::{self, BUILTIN_TOOLS};

/// Legacy CustomTool type for backward compatibility with Skills
/// This type has required fields while the new tools::CustomTool has optional fields
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CustomTool {
    pub key: String,
    pub display_name: String,
    pub relative_skills_dir: String,
    pub relative_detect_dir: String,
    pub created_at: i64,
    /// Force copy mode for skills sync (instead of symlink)
    #[serde(default)]
    pub force_copy: bool,
}

/// Convert from shared CustomTool to skills CustomTool
impl From<tools::CustomTool> for CustomTool {
    fn from(tool: tools::CustomTool) -> Self {
        CustomTool {
            key: tool.key,
            display_name: tool.display_name,
            relative_skills_dir: tool.relative_skills_dir.unwrap_or_default(),
            relative_detect_dir: tool.relative_detect_dir.unwrap_or_default(),
            created_at: tool.created_at,
            force_copy: tool.force_copy,
        }
    }
}

/// Convert from skills CustomTool to shared CustomTool
impl From<&CustomTool> for tools::CustomTool {
    fn from(tool: &CustomTool) -> Self {
        tools::CustomTool {
            key: tool.key.clone(),
            display_name: tool.display_name.clone(),
            relative_skills_dir: Some(tool.relative_skills_dir.clone()),
            relative_detect_dir: Some(tool.relative_detect_dir.clone()),
            force_copy: tool.force_copy,
            mcp_config_path: None,
            mcp_config_format: None,
            mcp_field: None,
            created_at: tool.created_at,
        }
    }
}

/// Tool adapter with path information (legacy type for compatibility)
#[derive(Clone, Debug)]
pub struct ToolAdapter {
    pub key: &'static str,
    pub display_name: &'static str,
    pub relative_skills_dir: &'static str,
    pub relative_detect_dir: &'static str,
}

/// Get all default tool adapters (built-in tools that support Skills)
pub fn default_tool_adapters() -> Vec<ToolAdapter> {
    BUILTIN_TOOLS
        .iter()
        .filter(|t| t.relative_skills_dir.is_some())
        .filter_map(|tool| {
            Some(ToolAdapter {
                key: tool.key,
                display_name: tool.display_name,
                relative_skills_dir: tool.relative_skills_dir?,
                relative_detect_dir: tool.relative_detect_dir?,
            })
        })
        .collect()
}

/// Find adapter by key
pub fn adapter_by_key(key: &str) -> Option<ToolAdapter> {
    default_tool_adapters()
        .into_iter()
        .find(|adapter| adapter.key == key)
}

/// Resolve default skills path for a tool
pub fn resolve_default_path(adapter: &ToolAdapter) -> Result<PathBuf> {
    // Use path_utils to resolve (handles ~/  and %APPDATA%/ paths)
    tools::path_utils::resolve_storage_path(adapter.relative_skills_dir)
        .context("failed to resolve skills path")
}

/// Resolve detect path for a tool
pub fn resolve_detect_path(adapter: &ToolAdapter) -> Result<PathBuf> {
    // Use path_utils to resolve (handles ~/  and %APPDATA%/ paths)
    tools::path_utils::resolve_storage_path(adapter.relative_detect_dir)
        .context("failed to resolve detect path")
}

/// Runtime tool adapter (can be built-in or custom)
#[derive(Clone, Debug)]
pub struct RuntimeToolAdapter {
    pub key: String,
    pub display_name: String,
    pub relative_skills_dir: String,
    pub relative_detect_dir: String,
    pub is_custom: bool,
    /// Force copy mode for skills sync (instead of symlink)
    pub force_copy: bool,
}

impl From<&ToolAdapter> for RuntimeToolAdapter {
    fn from(adapter: &ToolAdapter) -> Self {
        RuntimeToolAdapter {
            key: adapter.key.to_string(),
            display_name: adapter.display_name.to_string(),
            relative_skills_dir: adapter.relative_skills_dir.to_string(),
            relative_detect_dir: adapter.relative_detect_dir.to_string(),
            is_custom: false,
            force_copy: false, // Built-in tools use default (cursor handled specially in sync logic)
        }
    }
}

impl From<&CustomTool> for RuntimeToolAdapter {
    fn from(tool: &CustomTool) -> Self {
        RuntimeToolAdapter {
            key: tool.key.clone(),
            display_name: tool.display_name.clone(),
            relative_skills_dir: tool.relative_skills_dir.clone(),
            relative_detect_dir: tool.relative_detect_dir.clone(),
            is_custom: true,
            force_copy: tool.force_copy,
        }
    }
}

static RUNTIME_DB: OnceLock<surrealdb::Surreal<surrealdb::engine::local::Db>> = OnceLock::new();

pub fn set_runtime_db(db: surrealdb::Surreal<surrealdb::engine::local::Db>) {
    let _ = RUNTIME_DB.set(db);
}

fn runtime_db() -> Option<&'static surrealdb::Surreal<surrealdb::engine::local::Db>> {
    RUNTIME_DB.get()
}

/// Get all tool adapters (built-in + custom)
pub fn get_all_tool_adapters(custom_tools: &[CustomTool]) -> Vec<RuntimeToolAdapter> {
    let mut adapters: Vec<RuntimeToolAdapter> = default_tool_adapters()
        .iter()
        .map(RuntimeToolAdapter::from)
        .collect();

    for tool in custom_tools {
        adapters.push(RuntimeToolAdapter::from(tool));
    }

    adapters
}

/// Find adapter by key (supports both built-in and custom)
pub fn runtime_adapter_by_key(
    key: &str,
    custom_tools: &[CustomTool],
) -> Option<RuntimeToolAdapter> {
    // Check built-in first
    if let Some(adapter) = adapter_by_key(key) {
        return Some(RuntimeToolAdapter::from(&adapter));
    }
    // Check custom tools
    custom_tools
        .iter()
        .find(|t| t.key == key)
        .map(RuntimeToolAdapter::from)
}

/// Check if a tool is installed
/// Uses the shared detection logic from tools module
pub fn is_tool_installed(adapter: &RuntimeToolAdapter) -> Result<bool> {
    // Custom tools are always considered installed
    if adapter.is_custom {
        return Ok(true);
    }
    // Use shared detection logic for built-in tools
    if let Some(builtin) = tools::builtin_tool_by_key(&adapter.key) {
        let runtime_tool = tools::RuntimeTool::from(builtin);
        if let Some(db) = runtime_db() {
            return Ok(tools::is_tool_installed_with_db(db, &runtime_tool));
        }
        return Ok(tools::is_tool_installed(&runtime_tool));
    }
    // Fallback
    Ok(false)
}

pub async fn is_tool_installed_async(adapter: &RuntimeToolAdapter) -> Result<bool> {
    if adapter.is_custom {
        return Ok(true);
    }

    if let Some(builtin) = tools::builtin_tool_by_key(&adapter.key) {
        let runtime_tool = tools::RuntimeTool::from(builtin);
        if let Some(db) = runtime_db() {
            return Ok(tools::is_tool_installed_with_db_async(db, &runtime_tool).await);
        }
        return Ok(tools::is_tool_installed(&runtime_tool));
    }

    Ok(false)
}

/// Resolve skills path for a runtime tool
pub fn resolve_runtime_skills_path(adapter: &RuntimeToolAdapter) -> Result<PathBuf> {
    if let Some(db) = runtime_db() {
        if let Some(builtin) = tools::builtin_tool_by_key(&adapter.key) {
            let runtime_tool = tools::RuntimeTool::from(builtin);
            if let Some(path) = tools::resolve_skills_path_with_db(db, &runtime_tool) {
                return Ok(path);
            }
        }
    }
    // Use path_utils to resolve (handles ~/  and %APPDATA%/ paths for both built-in and custom tools)
    if let Some(resolved) = tools::path_utils::resolve_storage_path(&adapter.relative_skills_dir) {
        return Ok(resolved);
    }
    // Fallback: treat as absolute path
    Ok(PathBuf::from(&adapter.relative_skills_dir))
}

pub async fn resolve_runtime_skills_path_async(adapter: &RuntimeToolAdapter) -> Result<PathBuf> {
    if let Some(db) = runtime_db() {
        if let Some(builtin) = tools::builtin_tool_by_key(&adapter.key) {
            let runtime_tool = tools::RuntimeTool::from(builtin);
            if let Some(path) = tools::resolve_skills_path_with_db_async(db, &runtime_tool).await {
                return Ok(path);
            }
        }
    }

    if let Some(resolved) = tools::path_utils::resolve_storage_path(&adapter.relative_skills_dir) {
        return Ok(resolved);
    }

    Ok(PathBuf::from(&adapter.relative_skills_dir))
}

/// Scan a tool directory for skills
pub fn scan_tool_dir(
    adapter: &ToolAdapter,
    dir: &Path,
) -> Result<Vec<super::types::DetectedSkill>> {
    let mut results = Vec::new();
    if !dir.exists() {
        return Ok(results);
    }

    // Ignore paths containing our central repo
    let ignore_hint = "Application Support/com.ai-toolbox/skills";

    for entry in std::fs::read_dir(dir).with_context(|| format!("read dir {:?}", dir))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        let is_dir = file_type.is_dir() || (file_type.is_symlink() && path.is_dir());
        if !is_dir {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        // Skip system directories
        if adapter.key == "codex" && name == ".system" {
            continue;
        }

        let (is_link, link_target) = detect_link(&path);
        if path.to_string_lossy().contains(ignore_hint)
            || link_target
                .as_ref()
                .map(|p| p.to_string_lossy().contains(ignore_hint))
                .unwrap_or(false)
        {
            continue;
        }

        results.push(super::types::DetectedSkill {
            tool: adapter.key.to_string(),
            tool_display: adapter.display_name.to_string(),
            name,
            path,
            is_link,
            link_target,
        });
    }

    Ok(results)
}

fn detect_link(path: &Path) -> (bool, Option<PathBuf>) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let target = std::fs::read_link(path).ok();
            (true, target)
        }
        _ => {
            let target = std::fs::read_link(path).ok();
            if target.is_some() {
                (true, target)
            } else {
                (false, None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{adapter_by_key, default_tool_adapters, runtime_adapter_by_key, CustomTool};
    use crate::coding::tools::BUILTIN_TOOLS;

    #[test]
    fn default_tool_adapters_cover_all_builtin_skill_tools() {
        let actual_keys: HashSet<&'static str> = default_tool_adapters()
            .into_iter()
            .map(|adapter| adapter.key)
            .collect();
        let expected_keys: HashSet<&'static str> = BUILTIN_TOOLS
            .iter()
            .filter(|tool| tool.relative_skills_dir.is_some())
            .map(|tool| tool.key)
            .collect();

        assert_eq!(actual_keys, expected_keys);
    }

    #[test]
    fn adapter_by_key_returns_qoder_variants() {
        let qoder = adapter_by_key("qoder").expect("qoder should be available in skills adapters");
        assert_eq!(qoder.display_name, "Qoder");
        assert_eq!(qoder.relative_skills_dir, "~/.qoder/skills");

        let qoder_work = adapter_by_key("qoder_work")
            .expect("qoder_work should be available in skills adapters");
        assert_eq!(qoder_work.display_name, "QoderWork");
        assert_eq!(qoder_work.relative_skills_dir, "~/.qoderwork/skills");
    }

    #[test]
    fn runtime_adapter_by_key_prefers_builtin_tool_without_custom_entry() {
        let custom_tools: Vec<CustomTool> = Vec::new();
        let runtime_adapter = runtime_adapter_by_key("qoder_work", &custom_tools)
            .expect("qoder_work runtime adapter should resolve");

        assert_eq!(runtime_adapter.key, "qoder_work");
        assert_eq!(runtime_adapter.display_name, "QoderWork");
        assert!(!runtime_adapter.is_custom);
    }
}
