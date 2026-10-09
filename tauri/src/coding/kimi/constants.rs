pub const KIMI_HOME_ENV_KEY: &str = "KIMI_CODE_HOME";
pub const KIMI_LOCAL_PROVIDER_ID: &str = crate::coding::local_bridge::LOCAL_CONFIG_ID;
pub const KIMI_CONFIG_FILE: &str = "config.toml";
/// Kimi Code CLI declares MCP servers in `<root>/mcp.json` (JSON `mcpServers`),
/// not in config.toml. The `[mcp_servers]` TOML table is not read by the CLI.
pub const KIMI_MCP_CONFIG_FILE: &str = "mcp.json";
pub const KIMI_PROMPT_FILE: &str = "AGENTS.md";
pub const KIMI_SKILLS_DIR: &str = "skills";
pub const KIMI_PLUGINS_DIR: &str = "plugins";
pub const KIMI_SESSIONS_DIR: &str = "sessions";
pub const KIMI_CREDENTIALS_DIR: &str = "credentials";
pub const KIMI_OFFICIAL_API_BASE_URL: &str = "https://api.kimi.com/coding/v1";

/// Official channel default model, matching what the real Kimi CLI projects:
/// catalog key `kimi-code/kimi-for-coding` -> model id `kimi-for-coding`.
pub const KIMI_OFFICIAL_DEFAULT_MODEL_KEY: &str = "kimi-code/kimi-for-coding";
pub const KIMI_OFFICIAL_DEFAULT_MODEL_DISPLAY_NAME: &str = "K2.7 Coding";
/// Conservative official per-model context size; the CLI hard-requires a
/// positive `max_context_size` on every projected model.
pub const KIMI_DEFAULT_MODEL_MAX_CONTEXT_SIZE: i64 = 262_144;

/// The official-channel provider row, created on demand.
///
/// Kimi has no `auth.json` to import the way Codex does: the official channel is
/// a provider row whose credentials live in `credentials/kimi-code.json`. The
/// row therefore gets created either by the user (Add provider -> official
/// category, which is how a fresh install gets a card to sign in from) or by
/// [`super::official_accounts`] when a login or an existing local login needs one
/// to hang off. Keeping the template here means the login path no longer has to
/// ask the frontend to create a row before it can start.
pub const KIMI_OFFICIAL_PROVIDER_NAME: &str = "Kimi Official";
pub const KIMI_OFFICIAL_PROVIDER_CATEGORY: &str = "official";
/// Must satisfy `validate_provider_settings`: an official row is the one
/// category allowed to carry a `defaultModelKey` without a model catalog.
pub const KIMI_OFFICIAL_PROVIDER_SETTINGS_CONFIG: &str = concat!(
    "{\n  \"auth\": { \"API_KEY\": \"\" },\n",
    "  \"defaultModelKey\": \"kimi-code/kimi-for-coding\",\n",
    "  \"providerConfigs\": {}\n}"
);
