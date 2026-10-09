import type { ReactNode } from 'react';

/**
 * Where the section is hosted, which is the *only* styling axis the callers
 * genuinely disagreed on:
 *
 * - `embedded` — the section is a block inside a provider card (Codex, Kimi).
 *   It needs a separator above it so it does not read as part of the meta line,
 *   its rows indent under the collapsible title, and the empty state is a plain
 *   line: an illustration in the middle of a card pushes the model list below
 *   it off the fold.
 * - `standalone` — the section owns its card (ZCode, which has no official
 *   provider row to hang off), so it needs no separator and the empty state
 *   gets the standard `Empty` illustration.
 *
 * This is a real difference carried by the two original implementations, not a
 * knob: keep new values out unless a third host shape actually appears.
 */
export type OfficialAccountsSectionVariant = 'embedded' | 'standalone';

/**
 * One account row, resolved to display data.
 *
 * Every string here is already formatted and translated by the caller's mapping
 * layer: quota lines, plan names and identity labels are CLI-specific, and the
 * section must not learn any CLI's storage shape (same boundary as
 * `providerCardVariants`: the shared component owns layout, the caller owns the
 * meaning of the fields).
 */
export interface OfficialAccountRowView {
  id: string;
  /** Display name — an email, a username, or the caller's fallback wording. */
  label: string;
  /** Short kind tag, e.g. the caller's wording for "local" or "OAuth". */
  kindTag?: string;
  /** Secondary lines shown after the tag, in order (plan, quotas, resets). */
  metaLines?: string[];
  /**
   * The account the CLI is currently using. Drives the bold name and the
   * "default" tag. The caller must AND this with any state that hides runtime
   * state (Codex hides it during gateway takeover) before passing it in.
   */
  isApplied: boolean;
  /**
   * A login that is live on disk but has no stored snapshot. It cannot be
   * switched to (it *is* what is live) or deleted — it can only be saved.
   */
  isVirtual: boolean;
  /** Already-formatted refresh failure; replaces `metaLines` when present. */
  lastError?: string;
}

/** The row-level actions, named by what they do rather than where they sit. */
export type OfficialAccountAction = 'refresh' | 'details' | 'save' | 'apply' | 'delete';

export interface OfficialAccountPendingAction {
  accountId: string;
  action: OfficialAccountAction;
}

export interface OfficialAccountsSectionProps {
  variant: OfficialAccountsSectionVariant;
  title: string;
  /** The section's own explanation, rendered as the title's subtitle. */
  hint?: string;
  /**
   * Hover text for the switch action. Each CLI words it differently ("switch
   * Kimi to this account" / "switch ZCode to this account"), so it is passed in
   * rather than composed here — the section does not know the product name.
   */
  applyHint?: string;
  emptyText: string;
  accounts: OfficialAccountRowView[];
  /**
   * The sign-in entry, injected as a node so the section never learns how many
   * OAuth providers a CLI has: Codex and Kimi pass a button, ZCode passes a
   * dropdown over its two providers.
   */
  loginAction?: ReactNode;
  /**
   * Rendered before the title, inside the title line. Exists for the host that
   * owns its card: ZCode's card is a list member, so its drag handle and icon
   * have to sit on the title line. Provier-card hosts (Codex, Kimi) pass
   * nothing — their card already has a handle.
   */
  leadingAction?: ReactNode;
  /** Omit both to render the section uncollapsible (only Codex collapses). */
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
  /**
   * The one action currently in flight, if any. Carries the *kind* as well as
   * the row because two actions on two different rows can be pending at once
   * (Codex refreshes one account while saving another) and the spinner has to
   * land on the button that was actually pressed.
   */
  pending?: OfficialAccountPendingAction | null;
  /**
   * Withhold every row action. ZCode and Kimi set this while a login is in
   * flight: the login rewrites the same credential file the row actions touch,
   * so acting mid-login would race it.
   */
  actionsDisabled?: boolean;
  /**
   * Row actions, by fixed semantic rather than by placement. Each renders only
   * when its handler is supplied, so a CLI that has no such action simply does
   * not pass it.
   */
  onApply?: (account: OfficialAccountRowView) => void;
  onSaveLocal?: (account: OfficialAccountRowView) => void;
  onRefresh?: (account: OfficialAccountRowView) => void;
  onViewDetails?: (account: OfficialAccountRowView) => void;
  onDelete?: (account: OfficialAccountRowView) => void;
}