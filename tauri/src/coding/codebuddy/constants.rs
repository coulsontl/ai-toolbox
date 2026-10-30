//! CodeBuddy Code CLI backend constants.

/// Config-root env override. The CLI reads `CODEBUDDY_CONFIG_DIR` first and
/// falls back to `~/.codebuddy` (`resolveCoreConfigDir` in the shipped bundle).
pub const CODEBUDDY_CONFIG_DIR_ENV_KEY: &str = "CODEBUDDY_CONFIG_DIR";

/// Secondary override. The desktop product shares this engine and sets both
/// variables; a user who pointed `WORKBUDDY_CONFIG_DIR` at a CLI root expects
/// the CLI module to see it too.
pub const WORKBUDDY_CONFIG_DIR_ENV_KEY: &str = "WORKBUDDY_CONFIG_DIR";

/// Default config root when neither variable is set.
pub const CODEBUDDY_DEFAULT_CONFIG_DIR_NAME: &str = ".codebuddy";

/// Custom-model config file, owned by [`crate::coding::workbuddy_family::models_json`].
pub const CODEBUDDY_MODELS_FILE: &str = "models.json";

/// User-scope settings (`model`, `permissions`, `env`, …).
pub const CODEBUDDY_SETTINGS_FILE: &str = "settings.json";

/// User-scope MCP config file, as a **sibling** of the config directory.
///
/// Verified against the real CLI (v2.178.1) with `codebuddy mcp add -s <scope>`:
///   local   (default) -> `~/.codebuddy.json`  `projects[<cwd>].mcpServers`
///   project           -> `<cwd>/.mcp.json`
///   user              -> `~/.codebuddy.json`  top-level `mcpServers`
///
/// Neither `~/.codebuddy/.mcp.json` nor `~/.codebuddy/mcp.json` is read or
/// created — both stay absent after a user-scope add. Pointing the sync path at
/// either one reports success while the CLI ignores the file.
pub const CODEBUDDY_USER_MCP_FILE: &str = ".codebuddy.json";

/// The project-scope MCP file (`<cwd>/.mcp.json`), not a user-scope target.
pub const CODEBUDDY_PROJECT_MCP_FILE: &str = ".mcp.json";

/// Zip entry name for the sibling `~/.codebuddy.json` inside
/// `external-configs/codebuddy/`.
///
/// Needed because the backup/restore helpers resolve every entry **inside** the
/// tool's config directory and reject `..` segments (path-traversal guard), but
/// this file sits beside it. `restore_workbuddy_family_entry` recognises this
/// exact name and writes it to the config directory's parent instead.
pub const CODEBUDDY_SIBLING_MCP_ENTRY: &str = "sibling-mcp.json";

/// User-level memory (prompt) file.
pub const CODEBUDDY_PROMPT_FILE: &str = "CODEBUDDY.md";

pub const CODEBUDDY_SKILLS_DIR: &str = "skills";

/// Per-project transcripts (`<config root>/projects/<encoded cwd>/<id>.jsonl`).
pub const CODEBUDDY_PROJECTS_DIR: &str = "projects";

/// Reserved row id holding the config-root override in `codebuddy_settings_config`.
pub const CODEBUDDY_SETTINGS_CONFIG_ID: &str = "common";
