import React from 'react';
import { Button, Input, Popconfirm, Typography, message } from 'antd';
import { Globe, Plus, RefreshCw, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import {
  MINI_BROWSER_ONEOFF_PROFILE_ID,
  clearMiniBrowserProfile,
  closeMiniBrowser,
  closeMiniBrowserWindow,
  createMiniBrowserProfileId,
  listMiniBrowserWindows,
  loadMiniBrowserAccounts,
  loadMiniBrowserPrefs,
  loadMiniBrowserSites,
  nextMiniBrowserAccountLabel,
  normaliseMiniBrowserUrl,
  openMiniBrowserEmbedded,
  saveMiniBrowserAccounts,
  saveMiniBrowserPrefs,
  saveMiniBrowserSites,
  setMiniBrowserBounds,
  setMiniBrowserVisible,
  stableMiniBrowserProfileId,
  type MiniBrowserAccount,
  type MiniBrowserBounds,
  type MiniBrowserPrefs,
  type MiniBrowserSite,
  type MiniBrowserWindowInfo,
} from '@/services/miniBrowserApi';
import { areMiniBrowserBoundsEqual, toMiniBrowserBounds } from '../utils/miniBrowserEmbed';
import { deriveMiniBrowserSiteName } from '../utils/miniBrowserSiteName';
import styles from './MiniBrowserPage.module.less';

/** Throttle for `set_bounds`: a drag-resize fires far faster than the OS needs. */
const BOUNDS_SYNC_THROTTLE_MS = 100;

/**
 * How often the visible page's placement is re-asserted.
 *
 * The native page is pinned to a rectangle measured from the DOM, and a missed
 * update is fatal in a very visible way: the page keeps a rectangle from a
 * narrower window and ends up painted over the control column. `resize`,
 * `ResizeObserver` and scroll events cover the common cases, but they are all
 * one-shot — if the window grows while the update is skipped (a throttle window,
 * a layout pass that has not settled, or a failed `set_bounds`, which is
 * swallowed and would otherwise never be retried) nothing puts the page back.
 * This heartbeat re-measures and re-pins, and costs one
 * `getBoundingClientRect` per tick when nothing moved.
 */
const PLACEMENT_HEARTBEAT_MS = 400;

/** How often the open-page list is re-read (addresses, legacy windows). */
const WINDOW_POLL_INTERVAL_MS = 2000;

/**
 * Where a hidden page is parked.
 *
 * A hidden webview keeps whatever rectangle it had, so a page hidden while the
 * window was wide would still cover part of the control column if it were shown
 * before its rectangle was pushed. Parking every hidden page in a 1x1 rectangle
 * at the origin makes a stray reveal harmless instead of covering the UI, and
 * the next switch always pushes the real rectangle first.
 */
const PARKED_BOUNDS: MiniBrowserBounds = { x: 0, y: 0, width: 1, height: 1 };

/**
 * Workbench for the embedded mini browser: a tab strip over one native page.
 *
 * Every open account is a tab. The pages are native child webviews owned by the
 * backend (see `tauri/src/mini_browser.rs`) and are painted *on top of* the
 * React DOM, so:
 *
 * - exactly one native page is visible at a time and it is pinned to one empty
 *   placeholder rectangle. Switching a tab is a show/hide plus a re-pin, so
 *   there is never a second rectangle that could drift out of alignment (the
 *   old side-by-side grid had one rectangle per account, and every one of them
 *   had to stay aligned to be usable);
 * - nothing may be rendered inside that rectangle, because the native page
 *   hides it anyway;
 * - because `z-index` cannot lift React above a native child view, the visible
 *   page is hidden while one of this page's popovers is open and while another
 *   route is showing (this component is kept alive behind `display: none`).
 *
 * What is remembered between sessions (`ai-router.mini-browser.prefs`): the open
 * tab list, which tab was in front, and the address each account was showing.
 * Coming back restores the tab strip **without** loading a page — no hidden
 * WebView2 process, no background requests. A tab loads the moment the user
 * clicks it, and it resumes at the address it was left on rather than the site
 * root, so a relay console does not send you back through the landing page.
 *
 * One relay can be logged into several times: every saved account keeps its own
 * backend profile, so its page, cookies and login stay independent.
 */
export const MiniBrowserPage: React.FC = () => {
  const { t } = useTranslation();
  const { isActive } = useKeepAlive();
  const [busy, setBusy] = React.useState(false);
  const [url, setUrl] = React.useState('');
  const [sites, setSites] = React.useState<MiniBrowserSite[]>(() => loadMiniBrowserSites());
  const [accounts, setAccounts] = React.useState<MiniBrowserAccount[]>(() =>
    loadMiniBrowserAccounts(),
  );
  const [prefs, setPrefs] = React.useState<MiniBrowserPrefs>(() => loadMiniBrowserPrefs());
  const [openWindows, setOpenWindows] = React.useState<MiniBrowserWindowInfo[]>([]);
  const [siteName, setSiteName] = React.useState('');
  /** Site whose "new account" input is expanded, or null. */
  const [accountFormSiteId, setAccountFormSiteId] = React.useState<string | null>(null);
  const [accountName, setAccountName] = React.useState('');
  /**
   * Tabs whose native page exists right now.
   *
   * Separate from `prefs.openTabs` on purpose: a restored tab is listed but has
   * no native view until the user clicks it, so "is this page loaded" cannot be
   * derived from storage.
   */
  const [embeddedProfiles, setEmbeddedProfiles] = React.useState<string[]>([]);
  /** Address each page was asked to open, until the backend reports its own. */
  const [embeddedUrls, setEmbeddedUrls] = React.useState<Record<string, string>>({});
  /** Tabs whose page is being created right now. */
  const [openingProfiles, setOpeningProfiles] = React.useState<string[]>([]);
  /** Ids of this page's own popovers that are currently open. */
  const [openOverlays, setOpenOverlays] = React.useState<string[]>([]);

  const embedHostRef = React.useRef<HTMLDivElement | null>(null);
  const lastBoundsRef = React.useRef(new Map<string, MiniBrowserBounds>());
  const embeddedProfilesRef = React.useRef<string[]>([]);
  const knownWindowsRef = React.useRef(new Set<string>());
  const prefsRef = React.useRef(prefs);
  const isActiveRef = React.useRef(isActive);
  const syncTimerRef = React.useRef<number | null>(null);
  const openingRef = React.useRef(new Set<string>());

  embeddedProfilesRef.current = embeddedProfiles;
  prefsRef.current = prefs;
  isActiveRef.current = isActive;

  const overlayOpen = openOverlays.length > 0;
  const activeTab = prefs.activeProfile;
  const openCount = prefs.openTabs.length;
  const trackedProfiles = React.useMemo(
    () =>
      new Set([
        ...prefs.openTabs,
        ...openWindows
          .map((windowInfo) => windowInfo.profile_id)
          .filter((profileId): profileId is string => Boolean(profileId)),
      ]),
    [openWindows, prefs.openTabs],
  );

  /** Read inside async callbacks, which must not close over a stale render. */
  const overlayOpenRef = React.useRef(overlayOpen);
  overlayOpenRef.current = overlayOpen;

  const persistPrefs = React.useCallback((next: MiniBrowserPrefs) => {
    setPrefs(next);
    saveMiniBrowserPrefs(next);
  }, []);

  const persistSites = React.useCallback((next: MiniBrowserSite[]) => {
    setSites(next);
    saveMiniBrowserSites(next);
  }, []);

  const persistAccounts = React.useCallback((next: MiniBrowserAccount[]) => {
    setAccounts(next);
    saveMiniBrowserAccounts(next);
  }, []);

  const windowsByProfile = React.useMemo(() => {
    const map = new Map<string, MiniBrowserWindowInfo>();
    for (const windowInfo of openWindows) {
      if (windowInfo.profile_id) map.set(windowInfo.profile_id, windowInfo);
    }
    return map;
  }, [openWindows]);

  knownWindowsRef.current = new Set(windowsByProfile.keys());

  const accountsByProfile = React.useMemo(() => {
    const map = new Map<string, { account: MiniBrowserAccount; site?: MiniBrowserSite }>();
    for (const account of accounts) {
      map.set(account.id, {
        account,
        site: sites.find((candidate) => candidate.id === account.siteId),
      });
    }
    return map;
  }, [accounts, sites]);

  /**
   * Address each loaded page is really showing.
   *
   * The backend reports the live URL of every open webview, so a page that
   * navigated itself (a login redirect, a click inside the console) is reflected
   * without this page having to guess. The address a tab was *asked* to open is
   * only a fallback for the seconds before the backend lists it.
   */
  const observedUrls = React.useMemo(() => {
    const map: Record<string, string> = {};
    for (const [profileId, windowInfo] of windowsByProfile) {
      if (windowInfo.url) map[profileId] = windowInfo.url;
    }
    for (const [profileId, address] of Object.entries(embeddedUrls)) {
      if (address && !map[profileId]) map[profileId] = address;
    }
    return map;
  }, [embeddedUrls, windowsByProfile]);

  /**
   * Addresses worth remembering: web pages only.
   *
   * A webview reports `about:blank` for the moment between creation and its
   * first commit. Writing that into the remembered session would make the next
   * launch "resume" at a blank page, so anything that is not http/https — and
   * anything the backend would refuse — is filtered here instead.
   */
  const rememberableUrls = React.useMemo(() => {
    const map: Record<string, string> = {};
    for (const [profileId, address] of Object.entries(observedUrls)) {
      const normalised = normaliseMiniBrowserUrl(address);
      if (normalised) map[profileId] = normalised;
    }
    return map;
  }, [observedUrls]);

  /** Address a tab is showing (or last showed), for the strip and the rows. */
  const addressFor = React.useCallback(
    (profileId: string): string => {
      const observed = observedUrls[profileId];
      if (observed) return observed;
      const remembered = prefs.lastUrl[profileId];
      if (remembered) return remembered;
      return accountsByProfile.get(profileId)?.site?.url ?? '';
    },
    [accountsByProfile, observedUrls, prefs.lastUrl],
  );

  /** Human label and address of one tab (account, or the one-off address box). */
  const describeProfile = React.useCallback(
    (profileId: string): { name: string; url: string } => {
      const entry = accountsByProfile.get(profileId);
      if (entry) {
        return {
          name: entry.site ? `${entry.site.name} · ${entry.account.label}` : entry.account.label,
          url: addressFor(profileId),
        };
      }
      return { name: t('miniBrowser.manualWindow'), url: addressFor(profileId) };
    },
    [accountsByProfile, addressFor, t],
  );

  /**
   * Address a tab should open at: where it was left, else its site address.
   *
   * Remembering the exact page is the whole point of the preference: reopening
   * at the site root lands on a marketing/landing page and makes the user click
   * through to the console again on every launch.
   */
  const targetUrlFor = React.useCallback(
    (profileId: string): string => {
      const observed = observedUrls[profileId];
      if (observed) return observed;
      const remembered = prefsRef.current.lastUrl[profileId];
      if (remembered) return remembered;
      return accountsByProfile.get(profileId)?.site?.url ?? '';
    },
    [accountsByProfile, observedUrls],
  );

  // Pages opened outside this panel (a legacy standalone window) still belong in
  // the tab strip, so the backend's window list is re-read while this page is
  // the visible route and after every action that changes the open set. The poll
  // also keeps each tab's address honest.
  const refresh = React.useCallback(async () => {
    try {
      setOpenWindows(await listMiniBrowserWindows());
    } catch {
      // Reading state is best-effort: the page still works without it.
    }
  }, []);

  React.useEffect(() => {
    if (!isActive) return undefined;
    void refresh();
    const timer = window.setInterval(() => void refresh(), WINDOW_POLL_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [isActive, refresh]);

  /**
   * Keep the restored tab strip honest against the accounts that exist.
   *
   * Two corrections, both about not lying to the user:
   * - a remembered tab whose account was deleted elsewhere (or whose site was
   *   removed) is dropped, otherwise the strip would list a tab that can never
   *   load;
   * - a page that is open in the backend but missing from the strip (a legacy
   *   standalone window, or a tab opened before this preference existed) is
   *   added, otherwise its page would be running with no way to reach it.
   *
   * The one-off address box has no account, so it is exempt from the first rule.
   */
  React.useEffect(() => {
    const known = new Set(accounts.map((account) => account.id));
    known.add(MINI_BROWSER_ONEOFF_PROFILE_ID);
    const live = knownWindowsRef.current;
    const current = prefsRef.current;
    const kept = current.openTabs.filter((profileId) => known.has(profileId) || live.has(profileId));
    const additions = [...live].filter((profileId) => !kept.includes(profileId));
    if (kept.length === current.openTabs.length && additions.length === 0) return;

    const openTabs = [...kept, ...additions];
    persistPrefs({
      openTabs,
      activeProfile:
        current.activeProfile && openTabs.includes(current.activeProfile)
          ? current.activeProfile
          : openTabs[0] ?? null,
      lastUrl: Object.fromEntries(
        Object.entries(current.lastUrl).filter(([profileId]) =>
          openTabs.includes(profileId),
        ),
      ),
    });
  }, [accounts, openWindows, persistPrefs]);

  // Remember every address actually seen, so the next launch resumes there.
  // Only real changes are written, otherwise the two-second poll would rewrite
  // localStorage forever.
  React.useEffect(() => {
    const current = prefsRef.current.lastUrl;
    let changed = false;
    const next = { ...current };
    for (const [profileId, address] of Object.entries(rememberableUrls)) {
      if (address && next[profileId] !== address) {
        next[profileId] = address;
        changed = true;
      }
    }
    if (changed) persistPrefs({ ...prefsRef.current, lastUrl: next });
  }, [persistPrefs, rememberableUrls]);

  /**
   * Push the visible page's rectangle to the backend.
   *
   * Only the visible page is moved: a hidden page keeps whatever rectangle it
   * had, which is exactly what makes the switch a cheap show/hide. A failed send
   * forgets the cached rectangle instead of keeping a lie, so the next tick
   * retries.
   */
  const syncBounds = React.useCallback(async (profileId?: string) => {
    const target = profileId ?? prefsRef.current.activeProfile;
    if (!target || !embeddedProfilesRef.current.includes(target)) return;
    const host = embedHostRef.current;
    if (!host) return;
    const bounds = toMiniBrowserBounds(host.getBoundingClientRect());
    if (!bounds) return;
    if (areMiniBrowserBoundsEqual(lastBoundsRef.current.get(target) ?? null, bounds)) return;

    lastBoundsRef.current.set(target, bounds);
    try {
      await setMiniBrowserBounds(target, bounds);
    } catch {
      lastBoundsRef.current.delete(target);
    }
  }, []);

  /** Leading-edge throttle: the first change is served 100ms later, at most. */
  const scheduleBoundsSync = React.useCallback(() => {
    if (syncTimerRef.current !== null) return;
    syncTimerRef.current = window.setTimeout(() => {
      syncTimerRef.current = null;
      // A hidden route has no usable rectangle; nothing to sync until it is back.
      if (!isActiveRef.current) return;
      void syncBounds();
    }, BOUNDS_SYNC_THROTTLE_MS);
  }, [syncBounds]);

  React.useEffect(() => {
    return () => {
      if (syncTimerRef.current !== null) {
        window.clearTimeout(syncTimerRef.current);
        syncTimerRef.current = null;
      }
    };
  }, []);

  // Layout changes reach the backend through one throttled path: the host's own
  // resize, the window resizing, and any scrolling ancestor (`main` is the
  // scroll container, so the listener is registered in the capture phase).
  React.useEffect(() => {
    if (!isActive) return undefined;

    const observer = new ResizeObserver(() => scheduleBoundsSync());
    if (embedHostRef.current) observer.observe(embedHostRef.current);

    const handleLayoutChange = () => scheduleBoundsSync();
    window.addEventListener('resize', handleLayoutChange);
    document.addEventListener('scroll', handleLayoutChange, true);
    scheduleBoundsSync();

    return () => {
      observer.disconnect();
      window.removeEventListener('resize', handleLayoutChange);
      document.removeEventListener('scroll', handleLayoutChange, true);
    };
  }, [isActive, scheduleBoundsSync]);

  // The heartbeat that re-asserts the placement (see the constant).
  React.useEffect(() => {
    if (!isActive || !activeTab || !embeddedProfiles.includes(activeTab) || overlayOpen) {
      return undefined;
    }
    const timer = window.setInterval(() => void syncBounds(), PLACEMENT_HEARTBEAT_MS);
    return () => window.clearInterval(timer);
  }, [activeTab, embeddedProfiles, isActive, overlayOpen, syncBounds]);

  const handleOverlayChange = React.useCallback((key: string, open: boolean) => {
    setOpenOverlays((previous) => {
      if (open) return previous.includes(key) ? previous : [...previous, key];
      return previous.filter((candidate) => candidate !== key);
    });
  }, []);

  /**
   * `onOpenChange` for one page popover.
   *
   * Popconfirm and dropdowns both report their open state this way; the native
   * page is hidden for as long as any of them is open, because it would
   * otherwise be painted over them.
   */
  const overlayHandlers = React.useCallback(
    (key: string) => ({
      onOpenChange: (open: boolean) => handleOverlayChange(key, open),
    }),
    [handleOverlayChange],
  );

  /**
   * Visibility follows the active tab, the route and the page's own popovers.
   *
   * Only the active tab may be visible: the others stay alive but hidden, which
   * is both what a tab strip means and the only way two native pages cannot
   * cover each other.
   */
  React.useEffect(() => {
    const visible = isActive && !overlayOpen;
    for (const profileId of embeddedProfiles) {
      // The active tab is revealed by `reassert` below, *after* its rectangle
      // has been pushed; showing it here would flash it at the old position.
      if (profileId === activeTab && visible) continue;
      // Hide first, then park it off the visible area: a hidden view keeps its
      // rectangle, and a stale rectangle is exactly what makes the next switch
      // flash at the old position before the re-pin lands.
      void setMiniBrowserVisible(profileId, false)
        .then(() => setMiniBrowserBounds(profileId, PARKED_BOUNDS))
        .catch(() => undefined);
    }
    if (!visible || !activeTab || !embeddedProfiles.includes(activeTab)) return;

    /*
     * Re-pin before revealing.
     *
     * A hidden tab keeps the rectangle it had when it was last visible, which
     * can be stale (the window was resized, the sidebar was scrolled). Showing
     * it first and moving it 100ms later is what makes a freshly switched tab
     * appear in the wrong place for a frame — or, when the move is missed, for
     * good. The rectangle is therefore pushed *first* and re-asserted after the
     * reveal so the page is guaranteed to be both visible and aligned.
     */
    const reassert = async () => {
      lastBoundsRef.current.delete(activeTab);
      await syncBounds(activeTab);
      await setMiniBrowserVisible(activeTab, true).catch(() => undefined);
    };
    void reassert();
  }, [activeTab, embeddedProfiles, isActive, overlayOpen, syncBounds]);

  // A real unmount (LRU eviction, app shutdown) hides every page: the native
  // views outlive the React tree, so leaving one visible would leave an orphan
  // rectangle over whatever renders next.
  React.useEffect(() => {
    return () => {
      for (const profileId of embeddedProfilesRef.current) {
        void setMiniBrowserVisible(profileId, false).catch(() => undefined);
      }
    };
  }, []);

  /**
   * Create the native page for one tab, pinned to the shared host rectangle.
   *
   * `reveal` says whether this page is the one the user is looking at now. It is
   * passed in rather than read from the preferences, because the preference
   * write that accompanies a tab switch is a state update and this page would
   * still be reading the previous value: a freshly opened tab would then stay
   * hidden, which looks exactly like "the page did not open".
   */
  const loadTab = React.useCallback(
    async (profileId: string, targetUrl: string, reveal: boolean) => {
      if (openingRef.current.has(profileId)) return;
      if (embeddedProfilesRef.current.includes(profileId)) return;

      const host = embedHostRef.current;
      if (!host) return;
      const bounds = toMiniBrowserBounds(host.getBoundingClientRect());
      if (!bounds) {
        // A hidden route has no layout, so the host measures as an empty
        // rectangle. The tab stays listed and loads when the route is shown.
        if (isActiveRef.current) void message.error(t('miniBrowser.embedNotReady'));
        return;
      }

      openingRef.current.add(profileId);
      setOpeningProfiles((previous) =>
        previous.includes(profileId) ? previous : [...previous, profileId],
      );
      try {
        await openMiniBrowserEmbedded(profileId, targetUrl, bounds);
        lastBoundsRef.current.set(profileId, bounds);
        setEmbeddedUrls((previous) => ({ ...previous, [profileId]: targetUrl }));
        setEmbeddedProfiles((previous) =>
          previous.includes(profileId) ? previous : [...previous, profileId],
        );
        await setMiniBrowserVisible(
          profileId,
          reveal && isActiveRef.current && !overlayOpenRef.current,
        );
        await refresh();
      } catch (error) {
        void message.error(
          t('miniBrowser.openFailed', {
            error: error instanceof Error ? error.message : String(error),
          }),
        );
      } finally {
        openingRef.current.delete(profileId);
        setOpeningProfiles((previous) => previous.filter((id) => id !== profileId));
      }
    },
    [refresh, t],
  );

  /** Put a tab in the strip and bring it to the front, loading it if needed. */
  const openEmbedded = React.useCallback(
    (profileId: string, targetUrl?: string) => {
      const resolvedUrl = normaliseMiniBrowserUrl(targetUrl ?? '') ?? targetUrlFor(profileId);
      const current = prefsRef.current;
      persistPrefs({
        openTabs: current.openTabs.includes(profileId)
          ? current.openTabs
          : [...current.openTabs, profileId],
        activeProfile: profileId,
        lastUrl: resolvedUrl
          ? { ...current.lastUrl, [profileId]: resolvedUrl }
          : current.lastUrl,
      });

      if (embeddedProfilesRef.current.includes(profileId)) {
        // Already loaded: the visibility effect shows it and re-pins its
        // rectangle, which is all "switch to this tab" has to do.
        return;
      }
      void loadTab(profileId, resolvedUrl, true);
    },
    [loadTab, persistPrefs, targetUrlFor],
  );

  /** Close one tab: drop it from the strip and destroy its native page. */
  const handleCloseEmbedded = React.useCallback(
    async (profileId: string) => {
      const current = prefsRef.current;
      const openTabs = current.openTabs.filter((candidate) => candidate !== profileId);
      persistPrefs({
        openTabs,
        activeProfile:
          current.activeProfile === profileId
            ? openTabs[openTabs.length - 1] ?? null
            : current.activeProfile,
        lastUrl: current.lastUrl,
      });
      lastBoundsRef.current.delete(profileId);
      setEmbeddedProfiles((previous) => previous.filter((id) => id !== profileId));
      setEmbeddedUrls((previous) =>
        Object.fromEntries(
          Object.entries(previous).filter(([candidate]) => candidate !== profileId),
        ),
      );
      // Hiding first makes the close reliable: the native page is gone from the
      // user's point of view immediately, even if the backend teardown is slow.
      await setMiniBrowserVisible(profileId, false).catch(() => undefined);
      await closeMiniBrowserWindow(profileId).catch(() => undefined);
      await refresh();
    },
    [persistPrefs, refresh],
  );

  /** Forget a deleted account: close it and drop it from the strip. */
  const forgetEmbeddedProfiles = React.useCallback(
    (profileIds: string[]) => {
      if (profileIds.length === 0) return;
      const dropped = new Set(profileIds);
      const current = prefsRef.current;
      const openTabs = current.openTabs.filter((profileId) => !dropped.has(profileId));
      persistPrefs({
        openTabs,
        activeProfile:
          current.activeProfile && dropped.has(current.activeProfile)
            ? openTabs[openTabs.length - 1] ?? null
            : current.activeProfile,
        lastUrl: Object.fromEntries(
          Object.entries(current.lastUrl).filter(([profileId]) => !dropped.has(profileId)),
        ),
      });
      for (const profileId of profileIds) {
        lastBoundsRef.current.delete(profileId);
        void setMiniBrowserVisible(profileId, false).catch(() => undefined);
      }
      setEmbeddedProfiles((previous) => previous.filter((id) => !dropped.has(id)));
      setEmbeddedUrls((previous) =>
        Object.fromEntries(
          Object.entries(previous).filter(([profileId]) => !dropped.has(profileId)),
        ),
      );
    },
    [persistPrefs],
  );

  /**
   * Create a site plus its first default account.
   *
   * The name field is optional: when it is left blank the host of the address
   * becomes the name (`https://relay.example.com/console` -> `relay.example.com`),
   * which is what the user already typed anyway. A hand-typed name always wins.
   */
  const handleSaveSite = (rawUrl: string, name: string) => {
    const normalised = normaliseMiniBrowserUrl(rawUrl);
    if (!normalised) {
      void message.warning(
        rawUrl.trim() ? t('miniBrowser.urlInvalid') : t('miniBrowser.urlRequired'),
      );
      return false;
    }

    const resolvedName = name.trim() || deriveMiniBrowserSiteName(normalised);
    if (!resolvedName) {
      void message.warning(t('miniBrowser.siteRequired'));
      return false;
    }

    // Saving a URL that is already listed renames it instead of replacing it:
    // replacing would orphan the accounts (and keep their logins on disk with
    // nothing pointing at them).
    const existingSite = sites.find((candidate) => candidate.url === normalised);
    if (existingSite) {
      persistSites(
        sites.map((candidate) =>
          candidate.id === existingSite.id ? { ...candidate, name: resolvedName } : candidate,
        ),
      );
      void message.success(t('miniBrowser.siteSaved'));
      return true;
    }

    const id = createMiniBrowserProfileId();
    const site: MiniBrowserSite = { id, name: resolvedName, url: normalised };
    const nextSites = [...sites, site];
    const nextAccounts = [
      ...accounts,
      { id: stableMiniBrowserProfileId(id), siteId: id, label: site.name },
    ].filter((account) => nextSites.some((candidate) => candidate.id === account.siteId));
    persistSites(nextSites);
    persistAccounts(nextAccounts);
    void message.success(t('miniBrowser.siteSaved'));
    return true;
  };

  const handleAddSiteFromInput = () => {
    if (handleSaveSite(url, siteName)) {
      setSiteName('');
      setUrl('');
    }
  };

  const handleDeleteSite = async (siteId: string) => {
    const removed = accounts.filter((account) => account.siteId === siteId);
    persistSites(sites.filter((site) => site.id !== siteId));
    persistAccounts(accounts.filter((account) => account.siteId !== siteId));
    forgetEmbeddedProfiles(removed.map((account) => account.id));
    // Deleting a site also forgets the login state of its accounts.
    for (const account of removed) {
      await clearMiniBrowserProfile(account.id).catch(() => undefined);
    }
    await refresh();
  };

  const handleAddAccount = (site: MiniBrowserSite) => {
    const label = accountName.trim();
    if (!label) {
      void message.warning(t('miniBrowser.accountNameRequired'));
      return;
    }
    persistAccounts([
      ...accounts,
      { id: createMiniBrowserProfileId(), siteId: site.id, label },
    ]);
    setAccountFormSiteId(null);
    setAccountName('');
  };

  const handleDeleteAccount = async (account: MiniBrowserAccount) => {
    persistAccounts(accounts.filter((candidate) => candidate.id !== account.id));
    forgetEmbeddedProfiles([account.id]);
    // Closing the page and dropping its data directory is what actually forgets
    // the login; the account entry itself is already gone locally.
    await clearMiniBrowserProfile(account.id).catch(() => undefined);
    await refresh();
  };

  const handleOpenAccount = (site: MiniBrowserSite | undefined, profileId: string) => {
    if (!site) return;
    openEmbedded(profileId, targetUrlFor(profileId) || site.url);
  };

  /** Put every account in the strip, loading them so the balances are live. */
  const handleOpenAllAccounts = async () => {
    if (accounts.length === 0) {
      void message.warning(t('miniBrowser.noAccounts'));
      return;
    }
    setBusy(true);
    try {
      const alreadyLoaded = new Set(embeddedProfilesRef.current);
      const current = prefsRef.current;
      const openTabs = [...current.openTabs];
      const lastUrl = { ...current.lastUrl };
      const pending: Array<{ profileId: string; targetUrl: string }> = [];
      for (const account of accounts) {
        const site = sites.find((candidate) => candidate.id === account.siteId);
        if (!site) continue;
        if (!openTabs.includes(account.id)) openTabs.push(account.id);
        const targetUrl = targetUrlFor(account.id) || site.url;
        if (targetUrl) lastUrl[account.id] = targetUrl;
        if (!alreadyLoaded.has(account.id)) pending.push({ profileId: account.id, targetUrl });
      }
      persistPrefs({
        openTabs,
        activeProfile: current.activeProfile ?? openTabs[0] ?? null,
        lastUrl,
      });
      const resolvedActive = current.activeProfile ?? openTabs[0] ?? null;
      const accountIsActive = (profileId: string) => profileId === resolvedActive;
      // Sequential on purpose: every call creates a native webview, and firing
      // them together makes the resulting order (and the memory spike)
      // unpredictable.
      for (const { profileId, targetUrl } of pending) {
        // Only the tab that ends up active may be revealed; the rest load
        // hidden and wait for a click.
        await loadTab(profileId, targetUrl, accountIsActive(profileId));
      }
      void message.success(t('miniBrowser.openAllDone'));
    } finally {
      setBusy(false);
    }
  };

  const handleCloseAll = async () => {
    // Every tab, not just the loaded ones: a restored-but-dormant tab is still a
    // tab the user asked to close, and leaving it in the strip would make the
    // button look broken.
    forgetEmbeddedProfiles([...prefsRef.current.openTabs]);
    await closeMiniBrowser().catch(() => undefined);
    await refresh();
  };

  const handleOpenOneOff = () => {
    const normalised = normaliseMiniBrowserUrl(url);
    if (!normalised) {
      void message.warning(
        url.trim() ? t('miniBrowser.urlInvalid') : t('miniBrowser.urlRequired'),
      );
      return;
    }
    openEmbedded(MINI_BROWSER_ONEOFF_PROFILE_ID, normalised);
  };

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <div className={styles.titleBlock}>
          <span className={styles.titleIcon}>
            <Globe size={18} aria-hidden="true" />
          </span>
          <div>
            <h1>{t('miniBrowser.title')}</h1>
            <p>{t('miniBrowser.subtitle')}</p>
          </div>
        </div>
        <div className={styles.headerControls}>
          <div className={styles.statusPill}>
            <span
              className={openCount > 0 ? styles.statusDotActive : styles.statusDot}
              aria-hidden="true"
            />
            <span>{t('miniBrowser.openTabs')}</span>
            <strong>{openCount}</strong>
          </div>
          <Button type="primary" loading={busy} onClick={() => void handleOpenAllAccounts()}>
            {t('miniBrowser.openAllAccounts')}
          </Button>
          {openCount > 0 ? (
            <Button onClick={() => void handleCloseAll()}>
              {t('miniBrowser.closeAllWindows')}
            </Button>
          ) : null}
        </div>
      </div>

      <div className={styles.body}>
        <aside className={styles.sidebar}>
          <section className={styles.section}>
            <Typography.Text strong className={styles.sectionTitle}>
              {t('miniBrowser.siteSection')}
            </Typography.Text>
            {sites.length > 0 ? (
              <div className={styles.siteList}>
                {sites.map((site) => {
                  const siteAccounts = accounts.filter(
                    (account) => account.siteId === site.id,
                  );
                  return (
                    <div className={styles.siteBlock} key={site.id}>
                      <div className={styles.siteHeader}>
                        <div className={styles.siteTitle}>
                          <strong>{site.name}</strong>
                          <span>{site.url}</span>
                        </div>
                        <div className={styles.siteActions}>
                          <Button
                            size="small"
                            onClick={() => {
                              setAccountFormSiteId(site.id);
                              setAccountName(nextMiniBrowserAccountLabel(site, accounts));
                            }}
                          >
                            <Plus size={12} /> {t('miniBrowser.addAccount')}
                          </Button>
                          <Button
                            size="small"
                            onClick={() =>
                              handleOpenAccount(
                                site,
                                siteAccounts[0]?.id ?? stableMiniBrowserProfileId(site.id),
                              )
                            }
                          >
                            {t('miniBrowser.openSiteDefault')}
                          </Button>
                          <Popconfirm
                            title={t('miniBrowser.removeSiteConfirm')}
                            okText={t('miniBrowser.removeSite')}
                            cancelText={t('miniBrowser.cancel')}
                            onOpenChange={overlayHandlers(`site-${site.id}`).onOpenChange}
                            onConfirm={() => void handleDeleteSite(site.id)}
                          >
                            <Button size="small" danger>
                              {t('miniBrowser.removeSite')}
                            </Button>
                          </Popconfirm>
                        </div>
                      </div>

                      <div className={styles.accountList}>
                        {siteAccounts.map((account) => {
                          const isTab = trackedProfiles.has(account.id);
                          const isActiveAccount = activeTab === account.id;
                          return (
                            <div className={styles.accountRow} key={account.id}>
                              <div className={styles.accountInfo}>
                                <Input
                                  size="small"
                                  className={styles.accountName}
                                  value={account.label}
                                  aria-label={t('miniBrowser.accountName')}
                                  onChange={(event) => {
                                    const label = event.currentTarget.value;
                                    persistAccounts(
                                      accounts.map((candidate) =>
                                        candidate.id === account.id
                                          ? { ...candidate, label }
                                          : candidate,
                                      ),
                                    );
                                  }}
                                />
                                <span
                                  className={
                                    isActiveAccount
                                      ? styles.statusOpen
                                      : isTab
                                        ? styles.statusTab
                                        : styles.statusClosed
                                  }
                                >
                                  {isActiveAccount
                                    ? t('miniBrowser.statusActive')
                                    : isTab
                                      ? t('miniBrowser.statusTab')
                                      : t('miniBrowser.statusClosed')}
                                </span>
                              </div>
                              <div className={styles.accountMeta}>
                                <span
                                  className={styles.accountUrl}
                                  title={addressFor(account.id)}
                                >
                                  {addressFor(account.id) || t('miniBrowser.noWindow')}
                                </span>
                                <div className={styles.accountActions}>
                                  <Button
                                    size="small"
                                    type={isActiveAccount ? 'default' : 'primary'}
                                    onClick={() => handleOpenAccount(site, account.id)}
                                  >
                                    {isActiveAccount
                                      ? t('miniBrowser.statusActive')
                                      : t('miniBrowser.openNewTab')}
                                  </Button>
                                  <Popconfirm
                                    title={t('miniBrowser.removeAccountConfirm')}
                                    okText={t('miniBrowser.removeAccount')}
                                    cancelText={t('miniBrowser.cancel')}
                                    onOpenChange={overlayHandlers(`account-${account.id}`).onOpenChange}
                                    onConfirm={() => void handleDeleteAccount(account)}
                                  >
                                    <Button size="small" danger>
                                      {t('miniBrowser.removeAccount')}
                                    </Button>
                                  </Popconfirm>
                                </div>
                              </div>
                            </div>
                          );
                        })}
                        {accountFormSiteId === site.id ? (
                          <div className={styles.accountForm}>
                            <Input
                              autoFocus
                              size="small"
                              value={accountName}
                              placeholder={t('miniBrowser.accountNamePlaceholder')}
                              aria-label={t('miniBrowser.accountName')}
                              onChange={(event) => setAccountName(event.currentTarget.value)}
                              onPressEnter={() => handleAddAccount(site)}
                            />
                            <Button
                              size="small"
                              type="primary"
                              onClick={() => handleAddAccount(site)}
                            >
                              {t('miniBrowser.confirmAddAccount')}
                            </Button>
                            <Button
                              size="small"
                              onClick={() => {
                                setAccountFormSiteId(null);
                                setAccountName('');
                              }}
                            >
                              {t('miniBrowser.cancel')}
                            </Button>
                          </div>
                        ) : null}
                      </div>
                    </div>
                  );
                })}
              </div>
            ) : (
              <Typography.Paragraph type="secondary" className={styles.emptyHint}>
                {t('miniBrowser.noSites')}
              </Typography.Paragraph>
            )}
          </section>

          <section className={styles.section}>
            <Typography.Text strong className={styles.sectionTitle}>
              {t('miniBrowser.manualSection')}
            </Typography.Text>
            <div className={styles.manualRow}>
              <Input
                value={url}
                placeholder={t('miniBrowser.urlPlaceholder')}
                aria-label={t('miniBrowser.urlLabel')}
                onChange={(event) => setUrl(event.currentTarget.value)}
                onPressEnter={handleOpenOneOff}
              />
              <Button type="primary" onClick={handleOpenOneOff}>
                {t('miniBrowser.open')}
              </Button>
            </div>
            <div className={styles.saveSiteRow}>
              <Input
                value={siteName}
                placeholder={t('miniBrowser.siteNamePlaceholder')}
                aria-label={t('miniBrowser.siteName')}
                onChange={(event) => setSiteName(event.currentTarget.value)}
                onPressEnter={handleAddSiteFromInput}
              />
              <Button onClick={handleAddSiteFromInput}>{t('miniBrowser.saveSite')}</Button>
            </div>
          </section>

          <Typography.Paragraph type="secondary" className={styles.hint}>
            {t('miniBrowser.hint')}
          </Typography.Paragraph>
        </aside>

        <section className={styles.browserPane} aria-label={t('miniBrowser.embedTitle')}>
          {prefs.openTabs.length === 0 ? (
            <div className={styles.embedEmpty}>
              <Typography.Text strong>{t('miniBrowser.embedTitle')}</Typography.Text>
              <Typography.Paragraph type="secondary" className={styles.embedEmptyText}>
                {t('miniBrowser.embedEmpty')}
              </Typography.Paragraph>
              <Typography.Paragraph type="secondary" className={styles.embedEmptyText}>
                {t('miniBrowser.embedNotice')}
              </Typography.Paragraph>
            </div>
          ) : (
            <>
              <div className={styles.tabStrip} role="tablist" aria-label={t('miniBrowser.openTabs')}>
                {prefs.openTabs.map((profileId) => {
                  const label = describeProfile(profileId);
                  const isActiveTab = profileId === activeTab;
                  const isLoaded = embeddedProfiles.includes(profileId);
                  const isOpening = openingProfiles.includes(profileId);
                  return (
                    <div
                      key={profileId}
                      className={isActiveTab ? styles.tabActive : styles.tab}
                      role="tab"
                      aria-selected={isActiveTab}
                    >
                      <button
                        type="button"
                        className={styles.tabButton}
                        onClick={() => openEmbedded(profileId)}
                        title={label.url}
                      >
                        <span
                          className={
                            isLoaded ? styles.tabDotLoaded : styles.tabDotDormant
                          }
                          aria-hidden="true"
                        />
                        <span className={styles.tabName}>{label.name}</span>
                        {isOpening ? (
                          <RefreshCw size={11} className={styles.tabSpinner} aria-hidden="true" />
                        ) : null}
                      </button>
                      <button
                        type="button"
                        className={styles.tabClose}
                        aria-label={t('miniBrowser.closeTab')}
                        title={t('miniBrowser.closeTab')}
                        onClick={() => void handleCloseEmbedded(profileId)}
                      >
                        <X size={12} />
                      </button>
                    </div>
                  );
                })}
              </div>

              <div className={styles.addressBar}>
                <Typography.Text className={styles.addressText} ellipsis>
                  {activeTab ? addressFor(activeTab) || t('miniBrowser.noWindow') : ''}
                </Typography.Text>
                {activeTab && !embeddedProfiles.includes(activeTab) ? (
                  <Typography.Text type="secondary" className={styles.addressHint}>
                    {t('miniBrowser.tabDormant')}
                  </Typography.Text>
                ) : null}
              </div>

              {/*
                The stage holds the one rectangle the native page is pinned to.
                The host stays empty on purpose: a native child view is painted
                above the DOM, so anything rendered inside that rectangle would
                only be covered. A dormant tab has no native view at all, so its
                placeholder can be a real DOM element — it disappears the moment
                the tab loads.
              */}
              <div className={styles.embedStage}>
                <div className={styles.embedHost} ref={embedHostRef} aria-hidden="true" />
                {activeTab && !embeddedProfiles.includes(activeTab) ? (
                  <div className={styles.dormantOverlay}>
                    <Typography.Text strong>{t('miniBrowser.tabDormantTitle')}</Typography.Text>
                    <Typography.Paragraph type="secondary" className={styles.embedEmptyText}>
                      {t('miniBrowser.tabDormantHint')}
                    </Typography.Paragraph>
                    <Button
                      type="primary"
                      loading={openingProfiles.includes(activeTab)}
                      onClick={() => openEmbedded(activeTab)}
                    >
                      {t('miniBrowser.tabLoad')}
                    </Button>
                  </div>
                ) : null}
              </div>
            </>
          )}
        </section>
      </div>
    </div>
  );
};

export default MiniBrowserPage;
