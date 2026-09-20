/**
 * Mini browser service.
 *
 * Opens a relay's console in an embedded native webview window so the user can
 * read API balance / usage without leaving the toolbox and without losing the
 * main window's state.
 *
 * A native top-level window is used rather than an `<iframe>` because relay
 * dashboards normally send `X-Frame-Options: DENY` (or `frame-ancestors 'none'`
 * in their CSP), which would render an empty frame. It also reuses the webview
 * engine the app already ships, so there is no extra runtime cost.
 *
 * Multiple accounts: a site can be saved once and opened as any number of
 * accounts. Each account is identified by a `profileId`; the backend gives every
 * profile its own window and its own webview data directory, so two logins for
 * the same relay keep separate cookies and stay logged in independently.
 */

import { invoke } from '@tauri-apps/api/core';

export interface MiniBrowserSite {
  id: string;
  name: string;
  url: string;
}

export interface MiniBrowserAccount {
  id: string;
  siteId: string;
  label: string;
}

/** One native window currently open in the backend. */
export interface MiniBrowserWindowInfo {
  profile_id: string | null;
  label: string;
  title: string;
  url: string;
}

export const MINI_BROWSER_SITES_STORAGE_KEY = 'ai-router.mini-browser.sites';

export const MINI_BROWSER_ACCOUNTS_STORAGE_KEY = 'ai-router.mini-browser.accounts';

/**
 * Where the panel keeps what the user left behind.
 *
 * `openTabs` is the set of accounts that had a tab, `activeProfile` the one
 * that was in front, and `lastUrl` the address each account was actually
 * showing. They live in localStorage next to the sites for the same reason:
 * the browser panel is a view over the user's own shortcuts, and the backend
 * only knows about pages that are open right now.
 */
export const MINI_BROWSER_PREFS_STORAGE_KEY = 'ai-router.mini-browser.prefs';

export interface MiniBrowserPrefs {
  /** Profile ids that had a tab when the panel was last left. */
  openTabs: string[];
  /** Profile id of the tab that was in front, or `null`. */
  activeProfile: string | null;
  /** Last address seen per profile, so a tab reopens where it was left. */
  lastUrl: Record<string, string>;
}

export const EMPTY_MINI_BROWSER_PREFS: MiniBrowserPrefs = {
  openTabs: [],
  activeProfile: null,
  lastUrl: {},
};

/**
 * Keep a stored `lastUrl` mapping usable.
 *
 * Only absolute http/https addresses are kept: anything else (a stale
 * `javascript:` entry, a half-typed string) would be rejected by the backend,
 * and silently dropping it makes the tab fall back to the site address instead
 * of failing to open.
 */
const parseMiniBrowserLastUrl = (raw: unknown): Record<string, string> => {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {};
  const entries = Object.entries(raw as Record<string, unknown>);
  const kept: Array<[string, string]> = [];
  for (const [profileId, value] of entries) {
    if (typeof value !== 'string' || profileId.length === 0) continue;
    const normalised = normaliseMiniBrowserUrl(value);
    if (normalised) kept.push([profileId, normalised]);
  }
  return Object.fromEntries(kept);
};

export const parseMiniBrowserPrefs = (raw: unknown): MiniBrowserPrefs => {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    return { openTabs: [], activeProfile: null, lastUrl: {} };
  }
  const record = raw as Record<string, unknown>;
  const openTabs = Array.isArray(record.openTabs)
    ? record.openTabs.filter((id): id is string => typeof id === 'string' && id.length > 0)
    : [];
  const activeProfile =
    typeof record.activeProfile === 'string' && record.activeProfile.length > 0
      ? record.activeProfile
      : null;
  return {
    openTabs: [...new Set(openTabs)],
    // An active tab that is not in the list is a contradiction; drop it rather
    // than render a tab strip with nothing highlighted.
    activeProfile: activeProfile && openTabs.includes(activeProfile) ? activeProfile : null,
    lastUrl: parseMiniBrowserLastUrl(record.lastUrl),
  };
};

export const loadMiniBrowserPrefs = (): MiniBrowserPrefs =>
  parseMiniBrowserPrefs(readStoredJson(MINI_BROWSER_PREFS_STORAGE_KEY));

export const saveMiniBrowserPrefs = (prefs: MiniBrowserPrefs): void => {
  localStorage.setItem(MINI_BROWSER_PREFS_STORAGE_KEY, JSON.stringify(prefs));
};

/** Keeps stored values usable even when a previous version wrote something else. */
const parseMiniBrowserSites = (raw: unknown): MiniBrowserSite[] => {
  if (!Array.isArray(raw)) return [];
  return raw.filter(
    (item): item is MiniBrowserSite =>
      Boolean(item) &&
      typeof item.id === 'string' &&
      typeof item.name === 'string' &&
      typeof item.url === 'string',
  );
};

const parseMiniBrowserAccounts = (raw: unknown): MiniBrowserAccount[] => {
  if (!Array.isArray(raw)) return [];
  return raw.filter(
    (item): item is MiniBrowserAccount =>
      Boolean(item) &&
      typeof item.id === 'string' &&
      typeof item.siteId === 'string' &&
      typeof item.label === 'string',
  );
};

const readStoredJson = (key: string): unknown => {
  try {
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
};

export const loadMiniBrowserSites = (): MiniBrowserSite[] =>
  parseMiniBrowserSites(readStoredJson(MINI_BROWSER_SITES_STORAGE_KEY));

export const saveMiniBrowserSites = (sites: MiniBrowserSite[]): void => {
  localStorage.setItem(MINI_BROWSER_SITES_STORAGE_KEY, JSON.stringify(sites));
};

/**
 * Longest profile id accepted by the backend. Kept in sync with
 * `MAX_PROFILE_LEN` in `tauri/src/mini_browser.rs`.
 */
export const MINI_BROWSER_MAX_PROFILE_LENGTH = 64;

/**
 * Coerce a candidate into a profile id the backend accepts: lowercase ASCII
 * letters, digits and `-`, at most 64 characters.
 *
 * Returns `null` when nothing usable survives, so callers never send an id the
 * backend would reject.
 */
export const toMiniBrowserProfileId = (raw: string): string | null => {
  const cleaned = raw
    .toLowerCase()
    .replace(/[^a-z0-9-]/g, '')
    .slice(0, MINI_BROWSER_MAX_PROFILE_LENGTH);
  return cleaned.length > 0 ? cleaned : null;
};

/** Stable new profile id for an account the user just created. */
export const createMiniBrowserProfileId = (): string => {
  const generated = typeof crypto !== 'undefined' && crypto.randomUUID
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  return toMiniBrowserProfileId(generated) ?? `${Date.now()}`;
};

/**
 * Profile used by the page's "open a URL once" box.
 *
 * A one-off address has no saved account, but the embedded command still needs
 * a profile id, so it gets this fixed one. That keeps the one-off page inside
 * the main window (instead of the legacy separate window) and gives it its own
 * stable cookies. It collides with a saved account only if an account was
 * literally called `manual`, which the id generator cannot produce.
 */
export const MINI_BROWSER_ONEOFF_PROFILE_ID = 'manual';

/**
 * Derive a profile id from an existing stored id, deterministically.
 *
 * A site id written by an older version is a UUID, so this returns it
 * unchanged. If the stored id contains characters the backend rejects, the
 * fallback is a hash rather than a random value: the same stored id must always
 * map to the same profile, otherwise the account would lose its login state on
 * every reload.
 */
export const stableMiniBrowserProfileId = (rawId: string): string => {
  const cleaned = toMiniBrowserProfileId(rawId);
  if (cleaned) return cleaned;
  let hash = 2166136261;
  for (let index = 0; index < rawId.length; index += 1) {
    hash ^= rawId.charCodeAt(index);
    hash = Math.imul(hash, 16777619) >>> 0;
  }
  return `s${hash.toString(16).padStart(8, '0')}`;
};

/**
 * Reconcile stored accounts with stored sites.
 *
 * Two rules, both pure so they can be tested without a browser:
 * - every site without an account gets one default account whose id is the site
 *   id, so a site saved by an older version keeps a stable profile;
 * - accounts pointing at a site that no longer exists are dropped.
 */
export const ensureMiniBrowserAccounts = (
  sites: MiniBrowserSite[],
  accounts: MiniBrowserAccount[],
): MiniBrowserAccount[] => {
  const siteIds = new Set(sites.map((site) => site.id));
  const kept = accounts.filter((account) => siteIds.has(account.siteId));
  const sitesWithAccounts = new Set(kept.map((account) => account.siteId));

  const defaults = sites
    .filter((site) => !sitesWithAccounts.has(site.id))
    .map((site) => ({
      id: stableMiniBrowserProfileId(site.id),
      siteId: site.id,
      label: site.name,
    }));

  // Defaults go last so previously saved accounts keep their order.
  return [...kept, ...defaults];
};

/**
 * Default label for the next account of a site: `站点名-1`, `站点名-2`, ...
 *
 * Picks the lowest unused number so a freshly added account is named `-1` when
 * the site has none. The label is only a display name — the backend profile is
 * a separate stable id — so reusing the number of a deleted account is safe.
 */
export const nextMiniBrowserAccountLabel = (
  site: MiniBrowserSite,
  accounts: MiniBrowserAccount[],
): string => {
  const existing = new Set(
    accounts.filter((account) => account.siteId === site.id).map((account) => account.label),
  );
  let index = 1;
  while (existing.has(`${site.name}-${index}`)) index += 1;
  return `${site.name}-${index}`;
};

/** Accounts for the saved sites, including the legacy-site migration. */
export const loadMiniBrowserAccounts = (): MiniBrowserAccount[] =>
  ensureMiniBrowserAccounts(
    loadMiniBrowserSites(),
    parseMiniBrowserAccounts(readStoredJson(MINI_BROWSER_ACCOUNTS_STORAGE_KEY)),
  );

export const saveMiniBrowserAccounts = (accounts: MiniBrowserAccount[]): void => {
  localStorage.setItem(MINI_BROWSER_ACCOUNTS_STORAGE_KEY, JSON.stringify(accounts));
};

/** Longest address accepted, mirroring the backend guard. */
export const MINI_BROWSER_MAX_URL_LENGTH = 2048;

/**
 * Normalise user input into an absolute http/https URL.
 *
 * Mirrors the backend `normalise_browser_url` so the prompt can reject bad
 * input before invoking: a bare host gets `https://` prefixed, and non-web
 * schemes (`javascript:`, `file:`, `data:`) are refused instead of rewritten —
 * those are exactly the strings that would escape the browser sandbox.
 *
 * Returns `null` when the value is not usable.
 */
export const normaliseMiniBrowserUrl = (raw: string): string | null => {
  const trimmed = raw.trim();
  if (!trimmed || trimmed.length > MINI_BROWSER_MAX_URL_LENGTH) return null;
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(trimmed)) return null;

  const hasScheme = /^[a-zA-Z][a-zA-Z\d+\-.]*:\/\//.test(trimmed);
  const candidate = hasScheme ? trimmed : `https://${trimmed}`;
  try {
    const parsed = new URL(candidate);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null;
    if (!parsed.hostname) return null;
    return parsed.toString();
  } catch {
    return null;
  }
};

/**
 * Tauri serialises command arguments in camelCase, so `profile_id` in Rust is
 * `profileId` here. Omit the key entirely when there is no profile: the backend
 * argument is `Option<String>`, and this keeps the legacy no-profile window.
 */
const profileArgs = (profileId?: string): { profileId?: string } =>
  profileId ? { profileId } : {};

/**
 * Logical-pixel rectangle for the embedded page.
 *
 * The origin is the top-left corner of the main window's content area, which is
 * the same coordinate space as `getBoundingClientRect()` while the window is at
 * 100% scale — callers pass `left` / `top` / `width` / `height` straight
 * through, and `x` / `y` match the backend's naming.
 */
export interface MiniBrowserBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Open a URL in the embedded browser, creating its window on first use.
 *
 * Passing `profileId` opens (or reuses) that account's own window and login
 * state; omitting it keeps the original single shared window.
 */
export const openMiniBrowser = async (url: string, profileId?: string): Promise<void> => {
  await invoke('mini_browser_open', { url, ...profileArgs(profileId) });
};

/** Navigate the embedded browser to another URL, opening it when necessary. */
export const navigateMiniBrowser = async (url: string, profileId?: string): Promise<void> => {
  await invoke('mini_browser_navigate', { url, ...profileArgs(profileId) });
};

/** Address currently shown, or `null` when the browser window is closed. */
export const getMiniBrowserCurrentUrl = async (profileId?: string): Promise<string | null> => {
  return await invoke<string | null>('mini_browser_current_url', profileArgs(profileId));
};

/** Whether the embedded browser window currently exists. */
export const isMiniBrowserOpen = async (profileId?: string): Promise<boolean> => {
  return await invoke<boolean>('mini_browser_is_open', profileArgs(profileId));
};

/**
 * Close every mini browser window. No-op when none is open.
 *
 * Legacy semantics: it closes all accounts, not just one.
 */
export const closeMiniBrowser = async (): Promise<void> => {
  await invoke('mini_browser_close');
};

/** Close the window belonging to one account. No-op when it is not open. */
export const closeMiniBrowserWindow = async (profileId: string): Promise<void> => {
  await invoke('mini_browser_close_window', { profileId });
};

/** Show and focus the window belonging to one account. */
export const focusMiniBrowserWindow = async (profileId: string): Promise<void> => {
  await invoke('mini_browser_focus_window', { profileId });
};

/** Every open mini browser window, for the panel's tab strip. */
export const listMiniBrowserWindows = async (): Promise<MiniBrowserWindowInfo[]> => {
  return await invoke<MiniBrowserWindowInfo[]>('mini_browser_list_windows');
};

/**
 * Open (or navigate) an account's page inside the main window.
 *
 * The native child webview is always painted above the React DOM, so `bounds`
 * must describe an empty rectangle the page keeps free of its own content: it
 * comes from the placeholder cell's `getBoundingClientRect()`.
 */
export const openMiniBrowserEmbedded = async (
  profileId: string,
  url: string,
  bounds: MiniBrowserBounds,
): Promise<void> => {
  await invoke('mini_browser_open_embedded', {
    profileId,
    url,
    x: bounds.x,
    y: bounds.y,
    width: bounds.width,
    height: bounds.height,
  });
};

/**
 * Move (or resize) an already embedded page.
 *
 * Called after any layout change: window resize, panel scroll, grid re-flow.
 */
export const setMiniBrowserBounds = async (
  profileId: string,
  bounds: MiniBrowserBounds,
): Promise<void> => {
  await invoke('mini_browser_set_bounds', {
    profileId,
    x: bounds.x,
    y: bounds.y,
    width: bounds.width,
    height: bounds.height,
  });
};

/**
 * Show or hide an embedded page without closing it.
 *
 * Hiding is the only way a React modal, dropdown or popover can be seen over an
 * embedded page, because a native child webview ignores `z-index` entirely.
 */
export const setMiniBrowserVisible = async (
  profileId: string,
  visible: boolean,
): Promise<void> => {
  await invoke('mini_browser_set_visible', { profileId, visible });
};

/**
 * Forget an account's login state: close its window and delete its webview data
 * directory. The saved account itself stays in localStorage.
 */
export const clearMiniBrowserProfile = async (profileId: string): Promise<void> => {
  await invoke('mini_browser_clear_profile', { profileId });
};
