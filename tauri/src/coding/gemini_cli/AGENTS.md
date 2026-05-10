# AGENTS.md - Gemini CLI Backend

## Source of Truth

- Gemini CLI runtime root defaults to `~/.gemini`. If `GEMINI_CLI_HOME` is present in the process env or shell config, the effective root is `<GEMINI_CLI_HOME>/.gemini`, matching upstream Gemini CLI `homedir()` semantics.
- Provider/common/prompt/official-account database records must keep the same field shape as Claude Code and Codex:
  - `gemini_cli_provider`
  - `gemini_cli_common_config:\`common\``
  - `gemini_cli_prompt_config`
  - `gemini_cli_official_account`
- `settings_config` is the only provider-owned JSON payload. Do not add separate provider columns for OAuth, quota, or account state.
- Google official account OAuth snapshots are account-owned records in `gemini_cli_official_account`, matching Codex official accounts. Do not store account snapshots in provider `settings_config`.
- Official quota is refreshed from Gemini Code Assist `retrieveUserQuota` and saved on the account record. Do not add a separate usage/quota table.
- Official model refresh is a shared Gemini CLI model catalog refresh, not an OAuth account discovery API. It may fetch the public model registry and must fall back to bundled Gemini CLI constants; account-specific quota still comes only from Code Assist quota APIs.
- Prompt file defaults to `GEMINI.md`, but must follow `settings.json` `context.fileName` when present. If upstream config provides an array, AI Toolbox manages the first valid filename, matching Gemini CLI's current memory file behavior.
- Session history source is `tmp/<project>/chats/session-*.jsonl`, with legacy `session-*.json` fallback.

## Gotchas

- MCP and Skills are separate modules. Do not add Gemini CLI to `get_tool_skills_path_*` or `get_tool_mcp_config_path_*`.
- Applying a provider rewrites only AI Toolbox managed env keys in `.env` and merges provider config into `settings.json`.
- Managed env keys must include Gemini CLI auth-selector and request-shaping variables such as `GOOGLE_GENAI_USE_GCA`, `GOOGLE_GENAI_USE_VERTEXAI`, `GEMINI_CLI_USE_COMPUTE_ADC`, and `GEMINI_CLI_CUSTOM_HEADERS`; stale values can override or contaminate the selected OAuth/API-key provider.
- Applying a Gemini official account writes the selected account snapshot back to `oauth_creds.json`, then applies the official provider config so `settings.json` keeps `security.auth.selectedType = "oauth-personal"`.
- Gemini official OAuth must work without requiring end-user env vars. The OAuth desktop/installed-app client mirrors upstream Gemini CLI, but do not store the full client id/secret as contiguous source strings because GitHub Push Protection treats them as leaked credentials. Keep env vars only as optional overrides, and do not persist client id/secret into `gemini_cli_official_account` snapshots or `oauth_creds.json`.
- Official providers must be normalized on save and on apply: `security.auth.selectedType` is forced to `oauth-personal`, while API-key/gateway/Vertex managed env keys are removed. Otherwise a stale `gemini-api-key` value can make Gemini CLI show `Enter Gemini API Key` even after applying an OAuth account.
- Custom gateway/API-key providers must be normalized on save and on apply: `security.auth.selectedType` is forced to `gemini-api-key`, while custom env keys such as `GEMINI_API_KEY`, `GOOGLE_GEMINI_BASE_URL`, and `GEMINI_MODEL` are preserved. Otherwise copied official settings can leave a custom gateway in OAuth mode.
- Gemini CLI session `.jsonl` files are conversation records: metadata line, message records, `$set` updates, and `$rewindTo` rewrites. Session list/detail must parse this record stream instead of treating it as one JSON object.
- Applying provider/common config may change `settings.context.fileName`; after writing `settings.json`, rewrite the currently applied prompt config to the newly resolved prompt file before emitting sync.
- Deleting an applied prompt config must also rewrite the current runtime prompt file, then emit `wsl-sync-request-geminicli`. Removing only the database row leaves Gemini CLI and remote sync targets running stale prompt content.
- WSL sync should emit `wsl-sync-request-geminicli`; SSH remains manual/config-driven through mappings.

## Minimal Verification

- `cd tauri && cargo check`
- Unit tests for env merge, settings merge, usage/quota extraction, official-account serialization, and session parsing when those areas change.
