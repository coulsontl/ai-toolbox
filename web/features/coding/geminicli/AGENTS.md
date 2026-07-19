# AGENTS.md - Gemini CLI Frontend

## Source of Truth

- Gemini CLI page manages the runtime root, not a single loose file. `.env`, `settings.json`, the current global prompt file, `oauth_creds.json`, and `tmp/` are derived from the backend root path. The prompt file defaults to `GEMINI.md` but follows `settings.context.fileName` when present.
- Provider/common/prompt data follows the same database shape as Claude Code and Codex.
- Custom provider endpoint and API key are stored in `settingsConfig.env.GOOGLE_GEMINI_BASE_URL` and `settingsConfig.env.GEMINI_API_KEY`. Dedicated inputs must sync with the JSON editor and must not introduce separate database fields.
- Custom provider Gateway protocol selection is stored in provider `meta.apiFormat`, not in Gemini CLI runtime `settingsConfig`. The default is Gemini Native; selecting OpenAI Chat, OpenAI Responses, or Claude/Anthropic only changes Gateway target protocol and must preserve existing billing meta. Built-in Gateway provider endpoints are loaded from the shared `gateway_provider_profiles.json` catalog under `tools.gemini`; saving one must write only `meta.gatewayProfile={tool:"gemini",profileId,endpointId}` plus user-owned meta such as billing, while still saving the user-edited Base URL into `settingsConfig.env.GOOGLE_GEMINI_BASE_URL`. Do not persist profile-derived snapshots such as `providerType`, `apiFormat`, `apiKeyField`, `reasoningField`, `defaultMaxTokens`, or image policy fields for built-in endpoints.
- Gemini CLI must not persist or apply Codex-only `codexChatReasoning`. Some `tools.gemini` endpoints are derived from Codex endpoints for the same upstream API target, but runtime resolves Gemini effective meta from `tools.gemini` and explicitly ignores Codex-only reasoning config for Gemini providers.
- Provider model switching is stored in `settingsConfig.env.GEMINI_MODEL`. The dedicated model input must sync with the JSON editor and must not introduce a separate database field. Official mode refreshes Gemini CLI's shared supported-model catalog through the backend and falls back to bundled Gemini CLI constants; custom gateway/API-key mode fetches Google-native models through the shared `fetch_provider_models` command. `GOOGLE_GEMINI_BASE_URL` stays as the Gemini CLI runtime base URL; model-list probing may append the Gemini API version/path without changing the saved provider URL.
- Provider edit modals must initialize from the selected provider before mounting the JSON editor, and save/fetch-model actions must merge the dedicated URL/API key/model inputs back into `settingsConfig.env`. Otherwise a stale default custom template can overwrite or mask saved API-key settings.
- Official provider form state must normalize `settingsConfig.config.security.auth.selectedType` to `oauth-personal` and remove API-key/gateway managed env keys. Keep `GEMINI_MODEL` for model switching, but do not let hidden API-key fields survive in official mode.
- Custom provider form state must normalize `settingsConfig.config.security.auth.selectedType` to `gemini-api-key` while preserving `GEMINI_API_KEY`, `GOOGLE_GEMINI_BASE_URL`, and `GEMINI_MODEL`. A copied official JSON payload must not leave custom gateway providers in OAuth mode.
- Provider mode can only be selected while adding a blank new provider. The mode entry is folded into the top “channel” row: blank new providers can switch between custom and official, while copying a provider still creates a new provider record but preserves the source provider `category`; editing a saved provider must also preserve the existing `category`.
- `__local__` is a temporary local provider and has no persisted provider row. The page must not call official-account APIs for it and must not render the official-account management section on it.
- Even if backend marks `__local__` as `isApplied=true`, the UI must not show the Applied tag, selected highlight, or Apply button for it. Keep only the local-source hint; users should edit and save it into the database before it enters managed applied semantics.
- Google official account state follows the Codex official-account UI pattern: account list lives under the Google Official provider card, not as a standalone sidebar Tab/section.
- Usage/quota for Google Official accounts is account-owned display data. Do not put a separate `Usage / Quota` Tab back into the Gemini CLI page.
- The page must reuse the Claude Code / Codex layout style: `SectionSidebarLayout`, `RootDirectoryModal`, `GlobalPromptSettings`, and `SessionManagerPanel`.
- Gateway now has direct → single → failover states. The single entry is the "Gateway proxy" button on the applied provider card; both single and failover takeover must lock other provider apply buttons, while failover also shows P0/P1 priorities on cards. Switching P0 must restore direct mode first.

## Gotchas

- MCP and Skills are separate modules. Do not add Gemini CLI page logic that manages MCP servers or skills runtime paths.
- WSL sync is event-driven through the backend `wsl-sync-request-geminicli` listener. The frontend should refresh state after saves but should not implement its own WSL watcher.
- SSH remains manual/config-driven through Settings mappings; do not add SSH auto-sync behavior from this page.
- CC Switch import reuses shared `ImportFromCcSwitchModal`. The button only renders when `has_cc_switch_db` is true. Imports call `createGeminiCliProvider` with `sourceProviderId=ccs:gemini:{id}` and do not auto-apply.

## Minimal Verification

- At least verify TypeScript compiles after route, service, i18n, and page registration changes.
- At least verify the page uses `tool="geminicli"` for the shared session manager.
- Verify the Gemini page sidebar has no standalone Usage/Quota section and that official accounts render under the Google Official provider card.
- If `~/.cc-switch/cc-switch.db` exists, the Gemini page should show Import from CC Switch; imported providers keep env (including `GEMINI_MODEL`) and drop `mcpServers` from config.
