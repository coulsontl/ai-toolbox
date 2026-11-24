use ai_toolbox_lib::coding::tools::{
    builtin_tool_by_key, builtin_tool_forces_skill_copy, BUILTIN_TOOLS,
};

#[test]
fn antigravity_cli_uses_its_own_skills_and_shared_mcp_config() {
    let tool = builtin_tool_by_key("antigravity_cli").expect("antigravity_cli should exist");

    assert_eq!(tool.display_name, "Antigravity CLI");
    assert_eq!(
        tool.relative_skills_dir,
        Some("~/.gemini/antigravity-cli/skills")
    );
    assert_eq!(tool.relative_detect_dir, Some("~/.gemini/antigravity-cli"));
    assert_eq!(
        tool.mcp_config_path,
        Some("~/.gemini/config/mcp_config.json")
    );
    assert_eq!(tool.mcp_config_format, Some("json"));
    assert_eq!(tool.mcp_field, Some("mcpServers"));
}

#[test]
fn antigravity_legacy_entry_keeps_old_prefix() {
    let tool = builtin_tool_by_key("antigravity").expect("antigravity should exist");

    assert_eq!(
        tool.relative_skills_dir,
        Some("~/.gemini/antigravity/skills")
    );
    assert_eq!(
        tool.mcp_config_path,
        Some("~/.gemini/antigravity/mcp_config.json")
    );
}

#[test]
fn forced_skill_copy_covers_cursor_and_antigravity_cli_surfaces_only() {
    assert!(builtin_tool_forces_skill_copy("cursor"));
    assert!(builtin_tool_forces_skill_copy("antigravity_cli"));
    assert!(builtin_tool_forces_skill_copy("antigravity_global"));
    assert!(!builtin_tool_forces_skill_copy("antigravity"));
    assert!(!builtin_tool_forces_skill_copy("claude_code"));

    // Lock the list: no other built-in tool may silently opt into forced copy.
    let forced: Vec<&'static str> = BUILTIN_TOOLS
        .iter()
        .map(|tool| tool.key)
        .filter(|key| builtin_tool_forces_skill_copy(key))
        .collect();
    assert_eq!(forced, vec!["cursor", "antigravity_cli", "antigravity_global"]);
}

#[test]
fn qoder_work_builtin_tool_uses_standard_mcp_servers_field() {
    let tool = builtin_tool_by_key("qoder_work").expect("qoder_work should exist");

    assert_eq!(tool.relative_skills_dir, Some("~/.qoderwork/skills"));
    assert_eq!(tool.relative_detect_dir, Some("~/.qoderwork"));
    assert_eq!(tool.mcp_config_path, Some("~/.qoderwork/mcp.json"));
    assert_eq!(tool.mcp_field, Some("mcpServers"));
}

#[test]
fn qoder_builtin_tool_uses_home_mcp_path() {
    let tool = builtin_tool_by_key("qoder").expect("qoder should exist");

    assert_eq!(tool.relative_skills_dir, Some("~/.qoder/skills"));
    assert_eq!(tool.relative_detect_dir, Some("%APPDATA%/Qoder"));
    assert_eq!(tool.mcp_config_path, Some("~/.qoder/mcp.json"));
    assert_eq!(tool.mcp_field, Some("mcpServers"));
}

#[test]
fn qwen_code_builtin_tool_matches_gemini_format() {
    let tool = builtin_tool_by_key("qwen_code").expect("qwen_code should exist");

    assert_eq!(tool.relative_skills_dir, Some("~/.qwen/skills"));
    assert_eq!(tool.relative_detect_dir, Some("~/.qwen"));
    assert_eq!(tool.mcp_config_path, Some("~/.qwen/settings.json"));
    assert_eq!(tool.mcp_config_format, Some("json"));
    assert_eq!(tool.mcp_field, Some("mcpServers"));
}
