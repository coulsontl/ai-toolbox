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
  dropMiniBrowserProfiles,
  listMiniBrowserWindows,
  loadMiniBrowserAccounts,
  loadMiniBrowserPrefs,
  loadMiniBrowserSites,
  nextMiniBrowserAccountLabel,
  normaliseMiniBrowserUrl,
  openMiniBrowserEmbedded,
  readMiniBrowserBounds,
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
import {
  areMiniBrowserBoundsEqual,
  planPlacementFlush,
  toMiniBrowserBounds,
  type MiniBrowserAppliedPlacement,
} from '../utils/miniBrowserEmbed';
import { deriveMiniBrowserSiteName } from '../utils/miniBrowserSiteName';
import styles from './MiniBrowserPage.module.less';

/**
 * Merge window for placement changes.
 *
 * A drag-resize and a tab switch both fire far faster than the OS needs, and
 * every placement invoke travels to the main thread, so a burst is collapsed
 * into one flush per window instead of one invoke per event.
 */
const PLACEMENT_FLUSH_DELAY_MS = 100;

/**
 * How often the visible page's placement is checked against the backend.
 *
 * The native page is pinned to a rectangle measured from the DOM, and a missed
 * update is fatal in a very visible way: the page keeps a rectangle from a
 * narrower window and ends up painted over the control column. `resize`,
 * `ResizeObserver` and scroll events cover the common cases, but they are all
 * one-shot — if an update is skipped (a throttle window, a layout pass that has
 * not settled, or a failed send) nothing puts the page back.
 *
 * The check asks the backend where the page actually is and only re-sends when
 * the answer disagrees with the rectangle this page wants, so a healthy window
 * costs one read per tick and never re-sends a rectangle it already has.
 */
const PLACEMENT_VERIFY_MS = 1000;

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
  /**
   * The placement each loaded page should have, and the one the backend has
   * confirmed.
   *
   * A flush is the difference between the two, so a tab switch asks the backend
   * for the two pages that actually changed instead of re-pointing every open
   * tab. Bounds are physical pixels: see `utils/miniBrowserEmbed.ts` for why.
   */
  const desiredBoundsRef = React.useRef(new Map<string, MiniBrowserBounds>());
  const desiredVisibleRef = React.useRef(new Map<string, boolean>());
  const appliedRef = React.useRef(new Map<string, MiniBrowserAppliedPlacement>());
  /**
   * Per-profile counter bumped whenever a wanted placement changes.
   *
   * A reply that arrives after the page asked for something else must not be
   * recorded as applied, or the newer request would look like it had already
   * been sent and would never be retried.
   */
  const generationRef = React.useRef(new Map<string, number>());
  /** Every placement invoke runs on this chain, in FIFO order. */
  const flushChainRef = React.useRef<Promise<void>>(Promise.resolve());
  const flushBusyRef = React.useRef(false);
  const flushTimerRef = React.useRef<number | null>(null);
  const probeInFlightRef = React.useRef(false);
  /** The device pixel ratio the wanted rectangles were measured with. */
  const dprRef = React.useRef(window.devicePixelRatio);
  const embeddedProfilesRef = React.useRef<string[]>([]);
  const knownWindowsRef = React.useRef(new Set<string>());
  /** Profiles whose native page is being torn down right now. */
  const closingRef = React.useRef(new Set<string>());
  const prefsRef = React.useRef(prefs);
  const isActiveRef = React.useRef(isActive);
  const openingRef = React.useRef(new Set<string>());
  /**
   * True while any native page is being created.
   *
   * Creating a webview blocks the main thread and pumps its message loop, so
   * every invoke sent in that window is dispatched *inside* the pump: a
   * placement or verify call then runs WebView2 code on a half-built page and
   * hangs the window. The panel therefore goes quiet (no poll, no verify, no
   * flush) until the burst is over, and flushes once when it ends.
   */
  const creatingNativePageRef = React.useRef(false);

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
    const timer = window.setInterval(() => {
      if (creatingNativePageRef.current) return;
      void refresh();
    }, WINDOW_POLL_INTERVAL_MS);
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
    const closing = closingRef.current;
    const current = prefsRef.current;
    const kept = current.openTabs.filter(
      (profileId) => (known.has(profileId) && !closing.has(profileId)) || live.has(profileId),
    );
    const additions = [...live].filter(
      (profileId) => !kept.includes(profileId) && !closing.has(profileId),
    );
    if (kept.length === current.openTabs.length && additions.length === 0) return;

    const openTabs = [...kept, ...additions];
    persistPrefs({
      openTabs,
      activeProfile:
        current.activeProfile && openTabs.includes(current.activeProfile)
          ? current.activeProfile
          : openTabs[0] ?? null,
      // The remembered address is deliberately *not* pruned with the tab: the
      // user's place on a relay console is a habit, not a property of the tab,
      // so closing a tab and reopening that account later must still resume
      // there. Deleted accounts are pruned where they are deleted.
      lastUrl: current.lastUrl,
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
   * The placeholder cell's rectangle, in the physical pixels the backend wants.
   *
   * `null` means "not laid out" (a hidden route or a frame before the grid has a
   * size), which is never a rectangle to send.
   */
  const measureHostBounds = React.useCallback((): MiniBrowserBounds | null => {
    const host = embedHostRef.current;
    if (!host) return null;
    return toMiniBrowserBounds(host.getBoundingClientRect(), window.devicePixelRatio);
  }, []);

  /** Invalidate every reply still in flight for one profile. */
  const bumpGeneration = React.useCallback((profileId: string) => {
    generationRef.current.set(profileId, (generationRef.current.get(profileId) ?? 0) + 1);
  }, []);

  /** Want `bounds` for one profile. Returns whether that changed anything. */
  const markDesiredBounds = React.useCallback(
    (profileId: string, bounds: MiniBrowserBounds): boolean => {
      if (areMiniBrowserBoundsEqual(desiredBoundsRef.current.get(profileId) ?? null, bounds)) {
        return false;
      }
      desiredBoundsRef.current.set(profileId, bounds);
      bumpGeneration(profileId);
      return true;
    },
    [bumpGeneration],
  );

  /** Want `visible` for one profile. Returns whether that changed anything. */
  const markDesiredVisible = React.useCallback(
    (profileId: string, visible: boolean): boolean => {
      if (desiredVisibleRef.current.get(profileId) === visible) return false;
      desiredVisibleRef.current.set(profileId, visible);
      bumpGeneration(profileId);
      return true;
    },
    [bumpGeneration],
  );

  /** Forget everything this page remembers about one profile's placement. */
  const forgetPlacement = React.useCallback((profileId: string) => {
    desiredBoundsRef.current.delete(profileId);
    desiredVisibleRef.current.delete(profileId);
    appliedRef.current.delete(profileId);
    generationRef.current.delete(profileId);
  }, []);

  /**
   * Append one backend task to the placement chain.
   *
   * Placement invokes are serialised on purpose: a hide, a park and a show for
   * different tabs overlapping would let them land out of order, and landing out
   * of order is what makes a page appear at the previous tab's rectangle. The
   * chain never rejects, so one failed task cannot stop the ones behind it.
   */
  const appendToFlushChain = React.useCallback((task: () => Promise<void>): Promise<void> => {
    flushBusyRef.current = true;
    const tail = flushChainRef.current.then(task).catch(() => undefined);
    flushChainRef.current = tail;
    void tail.then(() => {
      // Only the last task clears the flag: an earlier one finishing while more
      // work is queued must not make the chain look idle.
      if (flushChainRef.current === tail) flushBusyRef.current = false;
    });
    return tail;
  }, []);

  /**
   * Send every placement that differs from the one the backend confirmed.
   *
   * The plan is built when the task runs rather than when it was queued, so a
   * flush that waited behind another one can never push a rectangle that a later
   * change has already replaced.
   */
  const flushPlacements = React.useCallback(() => {
    void appendToFlushChain(async () => {
      // A native page is being created right now: the main thread is pumping
      // messages inside that creation, so an invoke sent here would run
      // WebView2 code inside the pump. The burst-end effect re-schedules this.
      if (creatingNativePageRef.current) return;
      const ops = planPlacementFlush(
        desiredBoundsRef.current,
        desiredVisibleRef.current,
        appliedRef.current,
      );
      for (const op of ops) {
        // The wanted placement can change while this op waits its turn, and a
        // reply that lands afterwards must not be recorded as the applied one.
        const generation = generationRef.current.get(op.profileId) ?? 0;
        // A tab closed while its ops were queued: there is no page to move.
        if (!appliedRef.current.has(op.profileId)) continue;
        try {
          if (op.kind === 'bounds') {
            await setMiniBrowserBounds(op.profileId, op.bounds);
          } else {
            await setMiniBrowserVisible(op.profileId, op.visible);
          }
        } catch {
          // A refused send keeps the confirmed placement behind the wanted one,
          // so the next flush or verify tick offers it again.
          continue;
        }
        if ((generationRef.current.get(op.profileId) ?? 0) !== generation) continue;
        const applied = appliedRef.current.get(op.profileId);
        if (!applied) continue;
        appliedRef.current.set(
          op.profileId,
          op.kind === 'bounds'
            ? { ...applied, bounds: op.bounds }
            : { ...applied, visible: op.visible },
        );
      }
    });
  }, [appendToFlushChain]);

  /** Merge a burst of placement changes into one flush. */
  const scheduleFlush = React.useCallback(() => {
    if (flushTimerRef.current !== null) return;
    flushTimerRef.current = window.setTimeout(() => {
      flushTimerRef.current = null;
      flushPlacements();
    }, PLACEMENT_FLUSH_DELAY_MS);
  }, [flushPlacements]);

  // Keep the panel quiet while native pages are being created.
  const creatingNativePage = busy || openingProfiles.length > 0;
  React.useEffect(() => {
    creatingNativePageRef.current = creatingNativePage;
    if (!creatingNativePage) scheduleFlush();
  }, [creatingNativePage, scheduleFlush]);

  React.useEffect(() => {
    return () => {
      if (flushTimerRef.current !== null) {
        window.clearTimeout(flushTimerRef.current);
        flushTimerRef.current = null;
      }
    };
  }, []);

  /**
   * Point the page on screen at the placeholder cell.
   *
   * Only the active tab is moved: every other loaded page is parked, which is
   * exactly what makes a switch a cheap hide/show.
   */
  const measureActiveTab = React.useCallback(() => {
    const profileId = prefsRef.current.activeProfile;
    if (!profileId || !embeddedProfilesRef.current.includes(profileId)) return;
    const bounds = measureHostBounds();
    if (!bounds) return;
    markDesiredBounds(profileId, bounds);
  }, [markDesiredBounds, measureHostBounds]);

  /**
   * Re-point every loaded page after the display scale changed.
   *
   * The CSS rectangles have not moved, but their physical size has. Only the
   * page on screen has a rectangle worth redoing: a hidden page sits at the
   * scale-independent parked rectangle, so marking it changes nothing (and a
   * mark that changes nothing sends nothing).
   */
  const remeasureAfterScaleChange = React.useCallback(() => {
    const active = prefsRef.current.activeProfile;
    const bounds = measureHostBounds();
    for (const profileId of embeddedProfilesRef.current) {
      const onScreen = profileId === active && desiredVisibleRef.current.get(profileId) === true;
      if (onScreen) {
        if (bounds) markDesiredBounds(profileId, bounds);
      } else {
        markDesiredBounds(profileId, PARKED_BOUNDS);
      }
    }
    scheduleFlush();
  }, [markDesiredBounds, measureHostBounds, scheduleFlush]);

  // Layout changes reach the backend through one merged path: the host's own
  // resize, the window resizing, and any scrolling ancestor (`main` is the
  // scroll container, so the listener is registered in the capture phase).
  React.useEffect(() => {
    if (!isActive) return undefined;

    /*
     * A scale change keeps the CSS rectangle but not its physical size, so the
     * sweep replaces the ordinary re-measure. `resize` fires when the OS resizes
     * the window crossing a scale boundary; the media query covers a display
     * switch that resizes nothing.
     */
    let scaleQuery: MediaQueryList | null = null;
    const watchScale = () => {
      scaleQuery?.removeEventListener('change', handleLayoutChange);
      scaleQuery = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      scaleQuery.addEventListener('change', handleLayoutChange);
    };
    const handleLayoutChange = () => {
      if (window.devicePixelRatio !== dprRef.current) {
        dprRef.current = window.devicePixelRatio;
        watchScale();
        remeasureAfterScaleChange();
        return;
      }
      if (!isActiveRef.current) return;
      measureActiveTab();
      scheduleFlush();
    };

    const observer = new ResizeObserver(() => handleLayoutChange());
    if (embedHostRef.current) observer.observe(embedHostRef.current);

    window.addEventListener('resize', handleLayoutChange);
    document.addEventListener('scroll', handleLayoutChange, true);
    watchScale();
    handleLayoutChange();

    return () => {
      observer.disconnect();
      scaleQuery?.removeEventListener('change', handleLayoutChange);
      window.removeEventListener('resize', handleLayoutChange);
      document.removeEventListener('scroll', handleLayoutChange, true);
    };
  }, [isActive, measureActiveTab, remeasureAfterScaleChange, scheduleFlush]);

  /**
   * Ask the backend where the visible page actually is, and repair a mismatch.
   *
   * The native page can be moved behind this page's back — a missed event, a
   * layout pass that had not settled, a send the backend refused — so once a
   * second the reported rectangle is compared with the wanted one. A matching
   * answer sends nothing at all, and the probe is skipped while the flush chain
   * is busy so it can never pile up behind layout work.
   */
  const verifyPlacement = React.useCallback(() => {
    if (creatingNativePageRef.current) return;
    if (flushBusyRef.current || flushTimerRef.current !== null || probeInFlightRef.current) return;
    const profileId = prefsRef.current.activeProfile;
    if (!profileId || !embeddedProfilesRef.current.includes(profileId)) return;

    probeInFlightRef.current = true;
    void readMiniBrowserBounds(profileId)
      .then((reported) => {
        if (!reported) return;
        const desired = desiredBoundsRef.current.get(profileId);
        if (!desired || areMiniBrowserBoundsEqual(reported, desired)) return;
        if (import.meta.env.DEV) {
          console.debug(
            `[mini-browser] placement mismatch ${profileId}` +
              ` desired=${desired.x},${desired.y} ${desired.width}x${desired.height}` +
              ` reported=${reported.x},${reported.y} ${reported.width}x${reported.height}` +
              ` scale=${reported.scaleFactor} client=${reported.clientWidth}x${reported.clientHeight}`,
          );
        }
        /*
         * What the backend reports is what is really on screen, so it becomes the
         * confirmed placement: the next flush then sees the difference from the
         * wanted one and pushes it again. The rectangle is re-measured too, in
         * case the layout moved without an event.
         */
        const applied = appliedRef.current.get(profileId);
        if (applied) {
          appliedRef.current.set(profileId, {
            ...applied,
            bounds: {
              x: reported.x,
              y: reported.y,
              width: reported.width,
              height: reported.height,
            },
          });
        }
        const measured = measureHostBounds();
        if (measured) markDesiredBounds(profileId, measured);
        scheduleFlush();
      })
      .catch(() => undefined)
      .finally(() => {
        probeInFlightRef.current = false;
      });
  }, [markDesiredBounds, measureHostBounds, scheduleFlush]);

  // The verification loop that catches a placement the backend never applied.
  React.useEffect(() => {
    if (!isActive || !activeTab || !embeddedProfiles.includes(activeTab) || overlayOpen) {
      return undefined;
    }
    const timer = window.setInterval(() => verifyPlacement(), PLACEMENT_VERIFY_MS);
    return () => window.clearInterval(timer);
  }, [activeTab, embeddedProfiles, isActive, overlayOpen, verifyPlacement]);

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
      const shouldBeVisible = visible && profileId === activeTab;
      if (desiredVisibleRef.current.get(profileId) === shouldBeVisible) continue;
      markDesiredVisible(profileId, shouldBeVisible);
      /*
       * Two orders, both about what the user sees during a switch.
       *
       * A page that is leaving is hidden first and parked second, because a
       * hidden view keeps its rectangle: parked first, it would be the stale
       * rectangle that flashes on the next switch. A page that is arriving is
       * placed first and revealed second, for the same reason — it must not be
       * shown at the previous tab's rectangle, not even for a frame.
       */
      if (shouldBeVisible) {
        const bounds = measureHostBounds();
        if (bounds) markDesiredBounds(profileId, bounds);
      } else {
        markDesiredBounds(profileId, PARKED_BOUNDS);
      }
    }
    scheduleFlush();
  }, [
    activeTab,
    embeddedProfiles,
    isActive,
    markDesiredBounds,
    markDesiredVisible,
    measureHostBounds,
    overlayOpen,
    scheduleFlush,
  ]);

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
      const bounds = toMiniBrowserBounds(host.getBoundingClientRect(), window.devicePixelRatio);
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
        /*
         * A webview is created visible, so a tab that is loading behind the
         * active one is born at the parked rectangle and hidden immediately:
         * creating it at the real rectangle would paint a page the user did not
         * ask for over the current one for the length of the open call.
         */
        const placedAt = reveal ? bounds : PARKED_BOUNDS;
        const shown = reveal && isActiveRef.current && !overlayOpenRef.current;
        await openMiniBrowserEmbedded(profileId, targetUrl, placedAt);
        /*
         * "Close all" while this page was being created: the tab is gone from
         * the preferences, and the backend tears a freshly created page down
         * again when it was closed before it existed, so there is nothing to
         * place and nothing to list.
         */
        if (!prefsRef.current.openTabs.includes(profileId)) return;
        /*
         * The open call already placed the page, and a native page is born
         * visible, so its placement is seeded as both wanted and confirmed.
         * Seeding happens *before* the tab joins `embeddedProfiles`: the
         * visibility effect can then only add the show the open call has not
         * done yet, never repeat the one it just did.
         */
        bumpGeneration(profileId);
        desiredBoundsRef.current.set(profileId, placedAt);
        desiredVisibleRef.current.set(profileId, shown);
        appliedRef.current.set(profileId, { bounds: placedAt, visible: true });
        setEmbeddedUrls((previous) => ({ ...previous, [profileId]: targetUrl }));
        setEmbeddedProfiles((previous) =>
          previous.includes(profileId) ? previous : [...previous, profileId],
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
    [bumpGeneration, refresh, t],
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
      /*
       * Closing a tab keeps the remembered address on purpose: the user's place
       * on a relay console is a habit of the account, not a property of the tab.
       * `dropMiniBrowserProfiles` is the one place that rule lives, so closing
       * one tab and closing every tab cannot drift apart again.
       */
      persistPrefs(dropMiniBrowserProfiles(current, [profileId], 'closed'));
      setEmbeddedProfiles((previous) => previous.filter((id) => id !== profileId));
      setEmbeddedUrls((previous) =>
        Object.fromEntries(
          Object.entries(previous).filter(([candidate]) => candidate !== profileId),
        ),
      );
      // Marked as closing so the poll below cannot add the tab back from a
      // window list that still contains it for a moment.
      closingRef.current.add(profileId);
      try {
        // Hiding first makes the close reliable: the native page is gone from
        // the user's point of view immediately, even if the backend teardown is
        // slow.
        await appendToFlushChain(() =>
          setMiniBrowserVisible(profileId, false).catch(() => undefined),
        );
        // Reopened while that hide was queued: the label belongs to the new page
        // now, and closing it here would destroy the tab the user just asked
        // for.
        if (prefsRef.current.openTabs.includes(profileId)) return;
        // The page is hidden and about to stop existing: forget its placement so
        // a reopened tab starts from the open call's rectangle instead of a
        // stale "already applied" one.
        forgetPlacement(profileId);
        await closeMiniBrowserWindow(profileId).catch(() => undefined);
      } finally {
        closingRef.current.delete(profileId);
      }
      await refresh();
    },
    [appendToFlushChain, forgetPlacement, persistPrefs, refresh],
  );

  /** Forget a deleted account: close it and drop it from the strip. */
  const forgetEmbeddedProfiles = React.useCallback(
    (profileIds: string[]) => {
      if (profileIds.length === 0) return;
      const dropped = new Set(profileIds);
      // A deleted account takes its remembered address with it: the profile id
      // could otherwise be handed to a new account and resume a stranger's page.
      persistPrefs(dropMiniBrowserProfiles(prefsRef.current, profileIds, 'deleted'));
      for (const profileId of profileIds) {
        /*
         * The hide is queued before the placement is forgotten, in that order,
         * so it still belongs to a page this component knows about. It carries a
         * guard: a tab that was reopened while the hide waited its turn must not
         * be hidden by the stale request.
         */
        void appendToFlushChain(() => {
          if (desiredBoundsRef.current.has(profileId)) return Promise.resolve();
          return setMiniBrowserVisible(profileId, false).catch(() => undefined);
        });
        forgetPlacement(profileId);
      }
      setEmbeddedProfiles((previous) => previous.filter((id) => !dropped.has(id)));
      setEmbeddedUrls((previous) =>
        Object.fromEntries(
          Object.entries(previous).filter(([profileId]) => !dropped.has(profileId)),
        ),
      );
    },
    [appendToFlushChain, forgetPlacement, persistPrefs],
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
    /*
     * A page that is still being created owns the main thread until it exists,
     * and the single-tab path deliberately lets it finish (`loadTab` returns
     * early for a profile that is already opening). A bulk open therefore has to
     * wait for that one first, or the two creations overlap and the second one
     * blocks the window.
     */
    while (openingRef.current.size > 0 || closingRef.current.size > 0) {
      await new Promise((resolve) => window.setTimeout(resolve, 50));
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
    const closingTabs = [...prefsRef.current.openTabs];
    /*
     * Deliberately *not* `forgetEmbeddedProfiles`: that is the delete-account
     * rule and it drops each profile's remembered address. Closing every tab is
     * still just closing tabs, so the addresses stay and reopening a tab resumes
     * where it was — the same contract as closing one tab with its X.
     */
    persistPrefs(dropMiniBrowserProfiles(prefsRef.current, closingTabs, 'closed'));
    for (const profileId of closingTabs) {
      /*
       * Hide each page first, in the same order the single-tab close uses: the
       * native page is gone from the user's point of view immediately, even if
       * the backend teardown behind it is slow. The guard is the same one — a tab
       * reopened while the hide waited its turn must not be hidden by it.
       */
      void appendToFlushChain(() => {
        if (desiredBoundsRef.current.has(profileId)) return Promise.resolve();
        return setMiniBrowserVisible(profileId, false).catch(() => undefined);
      });
      // The backend keeps reporting these pages until their teardown lands, and
      // the two-second poll would otherwise add every one of them straight back.
      closingRef.current.add(profileId);
      forgetPlacement(profileId);
    }
    setEmbeddedProfiles((previous) => previous.filter((id) => !closingTabs.includes(id)));
    setEmbeddedUrls((previous) =>
      Object.fromEntries(
        Object.entries(previous).filter(([profileId]) => !closingTabs.includes(profileId)),
      ),
    );
    /*
     * Queued behind the per-tab hides above: the global close tears down the pages
     * themselves, and running it first would race them. It also runs for the
     * legacy standalone windows, which is why it exists at all.
     */
    try {
      await appendToFlushChain(() => closeMiniBrowser().catch(() => undefined));
    } finally {
      for (const profileId of closingTabs) closingRef.current.delete(profileId);
    }
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
