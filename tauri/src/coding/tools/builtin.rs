//! Built-in tool configurations
//!
//! Contains static configuration for all supported AI coding tools.
//!
//! Path prefix conventions:
//! - `~/` - relative to user's home directory
//! - `%APPDATA%/` - relative to config directory (APPDATA on Windows, ~/.config on Linux/macOS)
//! - No prefix - absolute path

use super::types::BuiltinTool;

/// All built-in tool configurations
/// Each tool can support Skills, MCP, or both
pub const BUILTIN_TOOLS: &[BuiltinTool] = &[
    // Claude Code - supports both Skills and MCP
    BuiltinTool {
        key: "claude_code",
        display_name: "Claude Code",
        relative_skills_dir: Some("~/.claude/skills"),
        relative_detect_dir: Some("~/.claude"),
        mcp_config_path: Some("~/.claude.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Codex - supports both Skills and MCP
    BuiltinTool {
        key: "codex",
        display_name: "Codex",
        relative_skills_dir: Some("~/.codex/skills"),
        relative_detect_dir: Some("~/.codex"),
        mcp_config_path: Some("~/.codex/config.toml"),
        mcp_config_format: Some("toml"),
        mcp_field: Some("mcp_servers"),
    },
    // Grok CLI - supports both Skills and MCP
    BuiltinTool {
        key: "grok",
        display_name: "Grok",
        relative_skills_dir: Some("~/.grok/skills"),
        relative_detect_dir: Some("~/.grok"),
        mcp_config_path: Some("~/.grok/config.toml"),
        mcp_config_format: Some("toml"),
        mcp_field: Some("mcp_servers"),
    },
    // Kimi Code CLI - supports both Skills and MCP
    // MCP servers live in <root>/mcp.json (JSON mcpServers), not config.toml.
    BuiltinTool {
        key: "kimi",
        display_name: "Kimi",
        relative_skills_dir: Some("~/.kimi-code/skills"),
        relative_detect_dir: Some("~/.kimi-code"),
        mcp_config_path: Some("~/.kimi-code/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Gemini CLI - supports both Skills and MCP
    BuiltinTool {
        key: "gemini_cli",
        display_name: "Gemini CLI",
        relative_skills_dir: Some("~/.gemini/skills"),
        relative_detect_dir: Some("~/.gemini"),
        mcp_config_path: Some("~/.gemini/settings.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Qwen Code - supports both Skills and MCP (forked from Gemini CLI)
    BuiltinTool {
        key: "qwen_code",
        display_name: "Qwen Code",
        relative_skills_dir: Some("~/.qwen/skills"),
        relative_detect_dir: Some("~/.qwen"),
        mcp_config_path: Some("~/.qwen/settings.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Cursor - supports both Skills and MCP
    BuiltinTool {
        key: "cursor",
        display_name: "Cursor",
        relative_skills_dir: Some("~/.cursor/skills"),
        relative_detect_dir: Some("~/.cursor"),
        mcp_config_path: Some("~/.cursor/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // OpenCode - supports both Skills and MCP
    BuiltinTool {
        key: "opencode",
        display_name: "OpenCode",
        relative_skills_dir: Some("~/.config/opencode/skills"),
        relative_detect_dir: Some("~/.config/opencode"),
        mcp_config_path: Some("~/.config/opencode/opencode.jsonc"), // Dynamic resolution in detection.rs
        mcp_config_format: Some("jsonc"),
        mcp_field: Some("mcp"),
    },
    // Antigravity (legacy IDE-era layout) - supports both Skills and MCP.
    // Kept for users still on ~/.gemini/antigravity; new installs use antigravity_cli.
    BuiltinTool {
        key: "antigravity",
        display_name: "Antigravity",
        relative_skills_dir: Some("~/.gemini/antigravity/skills"),
        relative_detect_dir: Some("~/.gemini/antigravity"),
        mcp_config_path: Some("~/.gemini/antigravity/mcp_config.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Antigravity CLI keeps its own skills directory but reads the shared
    // Antigravity user MCP config. Skills must be copied, not linked.
    BuiltinTool {
        key: "antigravity_cli",
        display_name: "Antigravity CLI",
        relative_skills_dir: Some("~/.gemini/antigravity-cli/skills"),
        relative_detect_dir: Some("~/.gemini/antigravity-cli"),
        mcp_config_path: Some("~/.gemini/config/mcp_config.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Amp - supports both Skills and MCP
    // MCP path uses VSCode plugin config path (%APPDATA%/Code/User/mcp.json)
    // Skills use home_dir: ~/.config/agents/skills
    BuiltinTool {
        key: "amp",
        display_name: "Amp",
        relative_skills_dir: Some("~/.config/agents/skills"),
        relative_detect_dir: Some("%APPDATA%/Code"),
        mcp_config_path: Some("%APPDATA%/Code/User/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("servers"),
    },
    // Kilo Code - supports both Skills and MCP
    // MCP path uses VSCode plugin config path
    // Skills use home_dir: ~/.kilocode/skills
    BuiltinTool {
        key: "kilo_code",
        display_name: "Kilo Code",
        relative_skills_dir: Some("~/.kilocode/skills"),
        relative_detect_dir: Some("%APPDATA%/Code/User/globalStorage/kilocode.kilo-code"),
        mcp_config_path: Some(
            "%APPDATA%/Code/User/globalStorage/kilocode.kilo-code/settings/mcp_settings.json",
        ),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Roo Code - supports both Skills and MCP
    // MCP path uses VSCode plugin config path
    // Skills use home_dir: ~/.roo/skills
    BuiltinTool {
        key: "roo_code",
        display_name: "Roo Code",
        relative_skills_dir: Some("~/.roo/skills"),
        relative_detect_dir: Some("%APPDATA%/Code/User/globalStorage/rooveterinaryinc.roo-cline"),
        mcp_config_path: Some(
            "%APPDATA%/Code/User/globalStorage/rooveterinaryinc.roo-cline/settings/mcp_settings.json",
        ),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Goose - Skills only
    BuiltinTool {
        key: "goose",
        display_name: "Goose",
        relative_skills_dir: Some("~/.config/goose/skills"),
        relative_detect_dir: Some("~/.config/goose"),
        mcp_config_path: None,
        mcp_config_format: None,
        mcp_field: None,
    },
    // GitHub Copilot - supports both Skills and MCP
    // MCP path uses the VSCode plugin config path (same as Amp).
    // The MCP page renames this entry to "GitHub Copilot (VSCode)".
    BuiltinTool {
        key: "github_copilot",
        display_name: "GitHub Copilot",
        relative_skills_dir: Some("~/.copilot/skills"),
        relative_detect_dir: Some("%APPDATA%/Code"),
        mcp_config_path: Some("%APPDATA%/Code/User/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("servers"),
    },
    // GitHub Copilot (IntelliJ) - MCP only
    // The actual config path is resolved per-OS in detection.rs.
    BuiltinTool {
        key: "github_copilot_intellij",
        display_name: "GitHub Copilot (IntelliJ)",
        relative_skills_dir: None,
        relative_detect_dir: Some("%APPDATA%/github-copilot/intellij"),
        mcp_config_path: Some("%APPDATA%/github-copilot/intellij/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("servers"),
    },
    // OpenClaw - supports both Skills and MCP
    BuiltinTool {
        key: "openclaw",
        display_name: "OpenClaw",
        relative_skills_dir: Some("~/.openclaw/skills"),
        relative_detect_dir: Some("~/.openclaw"),
        mcp_config_path: Some("~/.openclaw/openclaw.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcp.servers"),
    },
    // QClaw - Skills only (OpenClaw-family; skills dir per skills-manager reference)
    BuiltinTool {
        key: "qclaw",
        display_name: "QClaw",
        relative_skills_dir: Some("~/.qclaw/skills"),
        relative_detect_dir: Some("~/.qclaw"),
        mcp_config_path: None,
        mcp_config_format: None,
        mcp_field: None,
    },
    // EasyClaw - Skills only (OpenClaw-family; skills dir per skills-manager reference)
    BuiltinTool {
        key: "easyclaw",
        display_name: "EasyClaw",
        relative_skills_dir: Some("~/.easyclaw/skills"),
        relative_detect_dir: Some("~/.easyclaw"),
        mcp_config_path: None,
        mcp_config_format: None,
        mcp_field: None,
    },
    // AutoClaw - Skills only (OpenClaw-family; note the `.openclaw-autoclaw` dir)
    BuiltinTool {
        key: "autoclaw",
        display_name: "AutoClaw",
        relative_skills_dir: Some("~/.openclaw-autoclaw/skills"),
        relative_detect_dir: Some("~/.openclaw-autoclaw"),
        mcp_config_path: None,
        mcp_config_format: None,
        mcp_field: None,
    },
    // Pi - Skills plus MCP config. Pi 0.99+ has built-in MCP support and
    // pi-mcp-adapter reads the same `mcp.json`, so this one target covers both.
    BuiltinTool {
        key: "pi",
        display_name: "Pi",
        relative_skills_dir: Some("~/.pi/agent/skills"),
        relative_detect_dir: Some("~/.pi/agent"),
        mcp_config_path: Some("~/.pi/agent/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Oh My Pi - runtime root ~/.omp/agent; native skills dir is <agentDir>/skills.
    BuiltinTool {
        key: "oh_my_pi",
        display_name: "Oh My Pi",
        relative_skills_dir: Some("~/.omp/agent/skills"),
        relative_detect_dir: Some("~/.omp/agent"),
        mcp_config_path: Some("~/.omp/agent/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // OmO - the `omo` binary (senpi engine), distinct from the OpenCode
    // plugin edition under the opencode tab. Runtime root ~/.omo/agent.
    // Display name is just "OmO": the UI drops the "Native" qualifier
    // everywhere (tab, settings, Skills/MCP tool lists).
    BuiltinTool {
        key: "omo_native",
        display_name: "OmO",
        relative_skills_dir: Some("~/.omo/agent/skills"),
        relative_detect_dir: Some("~/.omo/agent"),
        mcp_config_path: Some("~/.omo/agent/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // QoderWork - supports both Skills and MCP
    BuiltinTool {
        key: "qoder_work",
        display_name: "QoderWork",
        relative_skills_dir: Some("~/.qoderwork/skills"),
        relative_detect_dir: Some("~/.qoderwork"),
        mcp_config_path: Some("~/.qoderwork/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Qoder - supports both Skills and MCP
    BuiltinTool {
        key: "qoder",
        display_name: "Qoder",
        relative_skills_dir: Some("~/.qoder/skills"),
        relative_detect_dir: Some("%APPDATA%/Qoder"),
        mcp_config_path: Some("~/.qoder/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Droid - supports both Skills and MCP
    BuiltinTool {
        key: "droid",
        display_name: "Droid",
        relative_skills_dir: Some("~/.factory/skills"),
        relative_detect_dir: Some("~/.factory"),
        mcp_config_path: Some("~/.factory/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Windsurf - supports both Skills and MCP
    BuiltinTool {
        key: "windsurf",
        display_name: "Windsurf",
        relative_skills_dir: Some("~/.codeium/windsurf/skills"),
        relative_detect_dir: Some("~/.codeium/windsurf"),
        mcp_config_path: Some("~/.codeium/mcp_config.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // TRAE IDE - Skills only (skills dir per skills-manager reference)
    BuiltinTool {
        key: "trae",
        display_name: "TRAE IDE",
        relative_skills_dir: Some("~/.trae/skills"),
        relative_detect_dir: Some("~/.trae"),
        mcp_config_path: None,
        mcp_config_format: None,
        mcp_field: None,
    },
    // TRAE CN - Skills only (skills dir per skills-manager reference)
    BuiltinTool {
        key: "trae_cn",
        display_name: "TRAE CN",
        relative_skills_dir: Some("~/.trae-cn/skills"),
        relative_detect_dir: Some("~/.trae-cn"),
        mcp_config_path: None,
        mcp_config_format: None,
        mcp_field: None,
    },
    // WorkBuddy AI (international) - supports both Skills and MCP
    BuiltinTool {
        key: "workbuddy_ai",
        display_name: "WorkBuddy AI",
        relative_skills_dir: Some("~/.workbuddy-ai/skills"),
        relative_detect_dir: Some("~/.workbuddy-ai"),
        mcp_config_path: Some("~/.workbuddy-ai/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // WorkBuddy (domestic) - supports both Skills and MCP
    BuiltinTool {
        key: "workbuddy",
        display_name: "WorkBuddy",
        relative_skills_dir: Some("~/.workbuddy/skills"),
        relative_detect_dir: Some("~/.workbuddy"),
        mcp_config_path: Some("~/.workbuddy/mcp.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Claude Desktop - config-file module; MCP lives in the normal config JSON's
    // `mcpServers`. Path is platform-resolved in detection.rs.
    BuiltinTool {
        key: "claude_desktop",
        display_name: "Claude Desktop",
        relative_skills_dir: None,
        relative_detect_dir: None,
        mcp_config_path: Some("%LOCALAPPDATA%/Claude/claude_desktop_config.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcpServers"),
    },
    // Hermes - runtime config.yaml holds `mcp_servers`; path is platform-resolved
    // in detection.rs (Windows: %LOCALAPPDATA%/hermes, others ~/.hermes).
    // Skills live at <hermes_root>/skills (SKILL.md compatible with agentskills.io).
    // relative_skills_dir is a fallback; the actual path is resolved via
    // resolve_special_skills_path in detection.rs to match the platform root.
    BuiltinTool {
        key: "hermes",
        display_name: "Hermes",
        relative_skills_dir: Some("~/.hermes/skills"),
        relative_detect_dir: Some("~/.hermes"),
        mcp_config_path: Some("~/.hermes/config.yaml"),
        mcp_config_format: Some("yaml"),
        mcp_field: Some("mcp_servers"),
    },
    // dsh - DeepSeek Harness. MCP is configured via Cordis patch DSL in
    // `cordis.patch.yml` (a YAML array of insert/override/delete ops, each MCP
    // server is an insert row with name `@deepseek-ai/dsh-mcp-client`).
    // The `mcp_field` is None (the key is `serverName` inside `config`, not a
    // top-level field). Format `cordis` dispatches to `mcp::cordis_patch`.
    BuiltinTool {
        key: "dsh",
        display_name: "DeepSeek Harness",
        relative_skills_dir: None,
        relative_detect_dir: None,
        mcp_config_path: Some("~/.dsh/cordis.patch.yml"),
        mcp_config_format: Some("cordis"),
        mcp_field: None,
    },
    // ZCode - supports both Skills and MCP. MCP servers live in the CLI config
    // under the nested `mcp.servers` path, not the provider registry the page
    // edits.
    BuiltinTool {
        key: "zcode",
        display_name: "ZCode",
        relative_skills_dir: Some("~/.zcode/skills"),
        relative_detect_dir: Some("~/.zcode"),
        mcp_config_path: Some("~/.zcode/cli/config.json"),
        mcp_config_format: Some("json"),
        mcp_field: Some("mcp.servers"),
    },
    // Universal - agentskills.io public shared skills directory.
    // Cross-tool directory scanned by dsh (rank 500) and other agentskills.io-
    // compliant tools. Skills-only sync target; no MCP config.
    BuiltinTool {
        key: "shared_agents",
        display_name: "Universal",
        relative_skills_dir: Some("~/.agents/skills"),
        relative_detect_dir: Some("~/.agents"),
        mcp_config_path: None,
        mcp_config_format: None,
        mcp_field: None,
    },
];

/// Get all built-in tools
pub fn get_all_builtin_tools() -> &'static [BuiltinTool] {
    BUILTIN_TOOLS
}

/// Get built-in tools that support Skills
pub fn get_skills_builtin_tools() -> Vec<&'static BuiltinTool> {
    BUILTIN_TOOLS
        .iter()
        .filter(|t| t.relative_skills_dir.is_some())
        .collect()
}

/// Get built-in tools that support MCP
pub fn get_mcp_builtin_tools() -> Vec<&'static BuiltinTool> {
    BUILTIN_TOOLS
        .iter()
        .filter(|t| t.mcp_config_path.is_some())
        .collect()
}

/// Find a built-in tool by key
pub fn builtin_tool_by_key(key: &str) -> Option<&'static BuiltinTool> {
    BUILTIN_TOOLS.iter().find(|t| t.key == key)
}

/// Built-in tools whose skills dir does not follow symlinks/junctions, so
/// skills sync must always copy for them. All sync entry points must consult
/// this helper instead of matching individual keys inline.
pub fn builtin_tool_forces_skill_copy(key: &str) -> bool {
    key.eq_ignore_ascii_case("cursor") || key.eq_ignore_ascii_case("antigravity_cli")
}
