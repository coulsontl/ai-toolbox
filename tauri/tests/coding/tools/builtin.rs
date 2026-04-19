use ai_toolbox_lib::coding::tools::builtin_tool_by_key;

#[test]
fn qoder_work_builtin_tool_uses_standard_mcp_servers_field() {
    let tool = builtin_tool_by_key("qoder_work").expect("qoder_work should exist");

    assert_eq!(tool.relative_skills_dir, Some("~/.qoderwork/skills"));
    assert_eq!(tool.relative_detect_dir, Some("~/.qoderwork"));
    assert_eq!(tool.mcp_config_path, Some("~/.qoderwork/mcp.json"));
    assert_eq!(tool.mcp_field, Some("mcpServers"));
}

#[test]
fn qoder_builtin_tool_uses_appdata_mcp_path() {
    let tool = builtin_tool_by_key("qoder").expect("qoder should exist");

    assert_eq!(tool.relative_skills_dir, Some("~/.qoder/skills"));
    assert_eq!(tool.relative_detect_dir, Some("%APPDATA%/Qoder"));
    assert_eq!(
        tool.mcp_config_path,
        Some("%APPDATA%/Qoder/SharedClientCache/mcp.json")
    );
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
