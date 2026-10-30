//! WorkBuddy desktop backend constants.

/// Config-root env override. The desktop app reads the same variable
/// (`WORKBUDDY_CONFIG_DIR`) before falling back to `~/.workbuddy`, so honouring
/// it here keeps AI Toolbox pointed at whatever root the app itself uses.
pub const WORKBUDDY_CONFIG_DIR_ENV_KEY: &str = "WORKBUDDY_CONFIG_DIR";

/// Secondary env override. The app treats `CODEBUDDY_CONFIG_DIR` as a fallback
/// for `WORKBUDDY_CONFIG_DIR` in several resolvers (`resolveWorkbuddyConfigDir`),
/// so a user who set only that one still expects it to be honoured.
pub const CODEBUDDY_CONFIG_DIR_ENV_KEY: &str = "CODEBUDDY_CONFIG_DIR";

/// The product's data folder name; the default config root is `~/.workbuddy`.
pub const WORKBUDDY_DEFAULT_CONFIG_DIR_NAME: &str = ".workbuddy";

/// Custom-model config file. Owned by [`crate::coding::workbuddy_family::models_json`].
pub const WORKBUDDY_MODELS_FILE: &str = "models.json";

/// User-scope MCP config. The desktop app writes this exact path
/// (`UserMcpProvider` → `<configDir>/mcp.json`); unlike the CLI it has no
/// `.mcp.json` counterpart.
pub const WORKBUDDY_MCP_CONFIG_FILE: &str = "mcp.json";

/// Skills directory the desktop app installs into and discovers from.
pub const WORKBUDDY_SKILLS_DIR: &str = "skills";

/// Per-project transcripts (`<config root>/projects/<encoded cwd>/<id>.jsonl`).
pub const WORKBUDDY_PROJECTS_DIR: &str = "projects";

/// The desktop app's SQLite store (sessions, workspaces, usage counters).
pub const WORKBUDDY_DATABASE_FILE: &str = "workbuddy.db";

/// Reserved row id holding the config-root override in `workbuddy_settings_config`.
pub const WORKBUDDY_SETTINGS_CONFIG_ID: &str = "common";

/// Reserved row id holding the editable common (base) config. Unused by the
/// first version — WorkBuddy has no common config file — but the table is
/// created up front so a later feature does not need a migration.
pub const WORKBUDDY_COMMON_CONFIG_ID: &str = "__common__";
