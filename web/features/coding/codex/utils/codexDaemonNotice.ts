/**
 * Post-apply hint for processes holding the Codex config (issue #443).
 *
 * Codex reads `config.toml` / `auth.json` once when an app-server starts and
 * keeps serving from that snapshot. Two kinds of process hold it: the CLI's
 * detached background daemon, and the ChatGPT desktop app (which bundles its
 * own app-server). Every AI Toolbox path that rewrites those files therefore
 * has to tell the user the change is not live yet, and offer the one action
 * that fixes it.
 *
 * The decisions live here as pure functions so the rules can be tested without
 * React or a backend.
 */

import type { CodexConfigHolder, CodexDaemonStatus } from '@/types/codex';

/**
 * Whether a just-applied config is being masked by a running holder.
 *
 * A failed or missing probe (`null`) is deliberately silent: the apply itself
 * succeeded, and a probe failure is not evidence that the user's next Codex
 * launch will use stale settings.
 */
export const shouldPromptCodexDaemonRestart = (
  status: CodexDaemonStatus | null | undefined,
): boolean => status?.running === true;

/**
 * Which holders to name in the prompt body.
 *
 * The wording has to be actionable, and the two holders are restarted by
 * different commands on the backend (`codex app-server daemon restart` vs
 * terminate-and-relaunch), so the prompt names whichever is actually holding
 * the config. Naming them separately is what keeps the prompt honest.
 */
export const describeCodexDaemonHolders = (
  holders: CodexConfigHolder[],
): { hasDaemon: boolean; hasDesktopApp: boolean } => ({
  hasDaemon: holders.includes('daemon'),
  hasDesktopApp: holders.includes('desktopApp'),
});
