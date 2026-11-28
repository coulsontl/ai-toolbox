# Official Account Auth Refresh (shared)

## One-line role

- Schedules **startup** and **interval** OAuth token freshness passes for official CLI accounts (Grok / Codex / Gemini CLI / Kimi / Antigravity) and for gateway plugin accounts (background OAuth renewal + optional daily check-in).
- Does **not** own OAuth HTTP, auth file schemas, or quota/limits APIs.

## Source of Truth

- Per-tool lead windows and refresh HTTP live in each tool's `official_accounts.rs`.
- This module only decides **when** to call each tool's `refresh_applied_*_accounts_if_needed`.
- On-demand paths (apply / refresh limits / force refresh) still call tool-local `ensure_fresh` directly and do not go through this loop.

## Why

- Grok access tokens last hours; Codex lasts much longer; Gemini is short-lived with a 5-minute lead. One shared loop avoids N independent `spawn`s and makes “startup first pass” a first-class event.
- Token refresh ≠ quota refresh. Do not schedule `wham/usage` or Grok billing here.

## Pass model

1. **Startup pass** (after global ~90s delay): every registered tool with `run_on_startup` runs once.
2. **Interval pass**: tools with `interval` run when due; tick granularity is 60s; per-tool `in_flight` prevents re-entry.
3. Candidates are **persisted OAuth accounts** (each tool filters; virtual `__local__` excluded). Applied and non-applied both refresh tokens into SQLite; only applied accounts rewrite live auth files.
4. Interval timing is **after the last completed pass** (startup resets the interval clock). Example: Grok startup at T+90s, next interval ≈ T+90s+15m — not every 15m from process start.

### Defaults

| Tool | Startup | Interval | Notes |
|------|---------|----------|-------|
| Grok | yes | 15m | Lead 30m inside Grok ensure_fresh |
| Gemini CLI | yes | 15m | Lead 5m inside Gemini ensure_fresh |
| Codex | yes | 12h | Lead 3d inside Codex ensure_fresh |
| Kimi | yes | 15m | Lead inside Kimi ensure_fresh |
| Antigravity | yes | 15m | Lead 5m inside Antigravity ensure_fresh; refresh takes the account-operation lock and only rewrites the live credential when the applied provider owns the applied account |
| Plugins | yes | 15m | Delegates to `plugins::background_tasks::run_plugin_background_pass`: renews plugin OAuth tokens only when one is within 15 minutes of expiry (or expired with a refresh token), runs due daily check-ins, then runs due npm plugin auto-updates. Reads SQLite / `plugin-auth.json` first; never starts Bun without a candidate; auto-updates are skipped entirely when disabled, not due, or no candidate exists |

## Gotchas

- Do not put billing/limits/model-catalog refresh into this module.
- Do not share one OAuth lock across tools; locks stay tool-local.
- Startup is still `ensure_fresh(force=false)`: not near expiry ⇒ no HTTP refresh.
- Failures are isolated per account / per tool (debug log only).
- Plugin renewal goes through the Bun host's `keepSignedIn` RPC, which itself honours the plugin's `auth.keepAlive === false`; the shared scheduler never refreshes a plugin token directly.
- Plugin check-in bookkeeping lives in `PluginAccountMetaRecord` (`auto_checkin`, `last_checkin_*`), not in this module; a `failed` outcome retries after 30 minutes, and `claimed` / `done` / `ineligible` / `inactive` / `captcha` settle the Beijing day.
- The Plugins pass must not early-return when `plugin-auth.json` has no accounts: `maybe_auto_update_plugins` runs after the account work on every tick. Auto-update uses `try_lock` on the plugin manager's `update_lock`, so a long `bun add` never queues a second pass; because the pass is inline, one slow update can delay the next tick of other tools in this scheduler, but never the current tick's completed refreshes/check-ins.

## Minimal verification

- `cargo test --lib coding::auth_refresh`
- With a persisted Grok OAuth account near expiry (applied or not): after app start + ~90s, token expiry/last_refresh update without clicking UI. Non-applied must not rewrite live auth.
