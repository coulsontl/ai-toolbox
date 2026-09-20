import React from 'react';
import { Button, Input, Popconfirm, Typography, message } from 'antd';
import { Globe, Plus, X } from 'lucide-react';
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
  loadMiniBrowserSites,
  nextMiniBrowserAccountLabel,
  normaliseMiniBrowserUrl,
  openMiniBrowserEmbedded,
  saveMiniBrowserAccounts,
  saveMiniBrowserSites,
  setMiniBrowserBounds,
  setMiniBrowserVisible,
  stableMiniBrowserProfileId,
  type MiniBrowserAccount,
  type MiniBrowserBounds,
  type MiniBrowserSite,
  type MiniBrowserWindowInfo,
} from '@/services/miniBrowserApi';
import {
  areMiniBrowserBoundsEqual,
  miniBrowserGridColumns,
  toMiniBrowserBounds,
} from '../utils/miniBrowserEmbed';
import { deriveMiniBrowserSiteName } from '../utils/miniBrowserSiteName';
import styles from './MiniBrowserPage.module.less';

/** Throttle for `set_bounds`: a drag-resize fires far faster than the OS needs. */
const BOUNDS_SYNC_THROTTLE_MS = 100;

/**
 * How often a cell whose rectangle is still unusable is re-measured.
 *
 * Normally one frame is enough: the ref callback attaches during the commit that
 * adds the cell, and the frame fires after layout. This bounded retry only
 * covers a cell that is committed but still measures empty (a font/theme swap,
 * a hidden ancestor that is disappearing), and stops instead of spinning.
 */
const PENDING_OPEN_RETRY_MS = 250;
const MAX_PENDING_OPEN_RETRIES = 6;

/**
 * Standalone workbench for the embedded mini browser.
 *
 * The pages themselves are native child webviews owned by the backend (see
 * `tauri/src/mini_browser.rs`) and are painted *on top of* the React DOM, so
 * this page is arranged as a left control column plus a right embed area:
 *
 * - every embedded account gets an empty placeholder `<div>` in a CSS grid, and
 *   its rectangle is what the native view is pinned to (`open_embedded` /
 *   `set_bounds`). Nothing may be rendered inside that rectangle, because the
 *   native view would hide it anyway;
 * - because `z-index` cannot lift React above a native child view, the page
 *   hides every embedded view (`set_visible(false)`) while one of its own
 *   popovers is open and shows them again when it closes;
 * - leaving the route (this component is kept alive, so `isActive` going false,
 *   or a real unmount) hides every embedded view without closing it.
 *
 * One relay can be logged into several times: every saved account keeps its own
 * backend profile, so its page, cookies and login stay independent.
 */
export const MiniBrowserPage: React.FC = () => {
  const { t } = useTranslation();
  const { isActive } = useKeepAlive();
  const [url, setUrl] = React.useState('');
  const [busy, setBusy] = React.useState(false);
  const [openWindows, setOpenWindows] = React.useState<MiniBrowserWindowInfo[]>([]);
  const [sites, setSites] = React.useState<MiniBrowserSite[]>(() => loadMiniBrowserSites());
  const [accounts, setAccounts] = React.useState<MiniBrowserAccount[]>(() =>
    loadMiniBrowserAccounts(),
  );
  const [siteName, setSiteName] = React.useState('');
  /** Site whose "new account" input is expanded, or null. */
  const [accountFormSiteId, setAccountFormSiteId] = React.useState<string | null>(null);
  const [accountName, setAccountName] = React.useState('');
  /**
   * Profiles currently embedded in the right-hand area, in grid order. This is
   * page state rather than backend state: the backend cannot tell us which
   * windows it has parented into this page, and inventing that answer from
   * `list_windows` would drop cells that are open but not yet listed.
   */
  const [embeddedProfiles, setEmbeddedProfiles] = React.useState<string[]>([]);
  /**
   * Address requested for each embedded profile.
   *
   * The backend's window list only reports top-level browser windows, so an
   * embedded page has no entry to read its address from. Remembering the address
   * this page actually asked for keeps the row honest ("https://…", not "not
   * opened yet") without guessing what the page navigated itself to afterwards.
   */
  const [embeddedUrls, setEmbeddedUrls] = React.useState<Record<string, string>>({});
  /** Ids of this page's own popovers that are currently open. */
  const [openOverlays, setOpenOverlays] = React.useState<string[]>([]);

  const cellElementsRef = React.useRef(new Map<string, HTMLDivElement>());
  const cellRefCallbacksRef = React.useRef(
    new Map<string, (element: HTMLDivElement | null) => void>(),
  );
  const lastBoundsRef = React.useRef(new Map<string, MiniBrowserBounds>());
  /** profileId -> address to open once its cell has been laid out. */
  const pendingOpensRef = React.useRef(new Map<string, string>());
  const embeddedProfilesRef = React.useRef<string[]>([]);
  const isActiveRef = React.useRef(isActive);
  const syncTimerRef = React.useRef<number | null>(null);
  /** Pending first-pass work for cells whose page has not been opened yet. */
  const openFlushFrameRef = React.useRef<number | null>(null);
  const openFlushTimerRef = React.useRef<number | null>(null);
  const openFlushAttemptsRef = React.useRef(0);

  embeddedProfilesRef.current = embeddedProfiles;
  isActiveRef.current = isActive;

  const overlayOpen = openOverlays.length > 0;
  const gridColumns = miniBrowserGridColumns(embeddedProfiles.length);
  const gridRows = Math.max(1, Math.ceil(embeddedProfiles.length / gridColumns));
  /**
   * Pages the counter has to report.
   *
   * Embedded pages are child webviews, so the backend's window list does not
   * contain them; a plain `openWindows.length` would read zero while several
   * pages are visibly open. Legacy standalone windows are counted in addition.
   */
  const openCount = new Set([
    ...embeddedProfiles,
    ...openWindows.map((windowInfo) => windowInfo.profile_id ?? windowInfo.label),
  ]).size;

  /** Read inside async callbacks, which must not close over a stale render. */
  const overlayOpenRef = React.useRef(overlayOpen);
  overlayOpenRef.current = overlayOpen;

  const windowsByProfile = React.useMemo(() => {
    const map = new Map<string, MiniBrowserWindowInfo>();
    for (const windowInfo of openWindows) {
      if (windowInfo.profile_id) map.set(windowInfo.profile_id, windowInfo);
    }
    return map;
  }, [openWindows]);

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

  /** Human label for one embedded cell (account, or the one-off address box). */
  const embeddedLabel = React.useCallback(
    (profileId: string): { name: string; url: string } => {
      const entry = accountsByProfile.get(profileId);
      // The window list only covers top-level windows, so an embedded page falls
      // back to the address this page asked for.
      const fallbackUrl = embeddedUrls[profileId] ?? windowsByProfile.get(profileId)?.url ?? '';
      if (entry) {
        return {
          name: entry.site ? `${entry.site.name} · ${entry.account.label}` : entry.account.label,
          url: fallbackUrl || entry.site?.url || '',
        };
      }
      return {
        name: t('miniBrowser.manualWindow'),
        url: fallbackUrl,
      };
    },
    [accountsByProfile, embeddedUrls, t, windowsByProfile],
  );

  // The embedded views live in the backend, so the address shown next to every
  // account is re-read whenever this page becomes the visible route and after
  // every action that changes the open set.
  const refresh = React.useCallback(async () => {
    try {
      setOpenWindows(await listMiniBrowserWindows());
    } catch {
      // Reading state is best-effort: the page still works without it.
    }
  }, []);

  React.useEffect(() => {
    if (!isActive) return;
    void refresh();
  }, [isActive, refresh]);

  const persistSites = React.useCallback((next: MiniBrowserSite[]) => {
    setSites(next);
    saveMiniBrowserSites(next);
  }, []);

  const persistAccounts = React.useCallback((next: MiniBrowserAccount[]) => {
    setAccounts(next);
    saveMiniBrowserAccounts(next);
  }, []);

  /** Stable ref callback per profile: a new closure would detach the old cell. */
  const getCellRef = React.useCallback((profileId: string) => {
    const existing = cellRefCallbacksRef.current.get(profileId);
    if (existing) return existing;

    const callback = (element: HTMLDivElement | null) => {
      if (element) {
        cellElementsRef.current.set(profileId, element);
      } else {
        cellElementsRef.current.delete(profileId);
        lastBoundsRef.current.delete(profileId);
      }
    };
    cellRefCallbacksRef.current.set(profileId, callback);
    return callback;
  }, []);

  /**
   * Stop tracking the given profiles as embedded.
   *
   * Drops their cells, their cached rectangles and any open request that has not
   * raced past the frame boundary yet, so a deleted account cannot reappear in
   * the grid when its pending open finally runs.
   *
   * Hiding first is what makes this reliable: the native view is hidden
   * synchronously from the page's point of view, so the cell can never reappear
   * as a live page after the state has forgotten it, no matter how long the
   * backend takes to destroy the view (`close_window` is a best-effort cleanup
   * on top — a hidden leftover view is reused and re-shown by the next open).
   */
  const forgetEmbeddedProfiles = React.useCallback((profileIds: string[]) => {
    if (profileIds.length === 0) return;
    const dropped = new Set(profileIds);
    for (const profileId of profileIds) {
      pendingOpensRef.current.delete(profileId);
      lastBoundsRef.current.delete(profileId);
      void setMiniBrowserVisible(profileId, false).catch(() => undefined);
    }
    setEmbeddedUrls((previous) =>
      Object.fromEntries(
        Object.entries(previous).filter(([profileId]) => !dropped.has(profileId)),
      ),
    );
    setEmbeddedProfiles((previous) => previous.filter((id) => !dropped.has(id)));
  }, []);

  /**
   * Push the current rectangles to the backend.
   *
   * Only cells whose rectangle actually changed are sent, so a page scroll that
   * does not move the grid costs nothing. A failed send forgets the cached
   * rectangle instead of keeping a lie, which makes the next sync retry.
   */
  const syncBounds = React.useCallback(async () => {
    for (const profileId of embeddedProfilesRef.current) {
      // A cell whose page has not been opened yet has nothing to move, and the
      // open call already carries its rectangle.
      if (pendingOpensRef.current.has(profileId)) continue;
      const element = cellElementsRef.current.get(profileId);
      if (!element) continue;
      const bounds = toMiniBrowserBounds(element.getBoundingClientRect());
      if (!bounds) continue;
      if (areMiniBrowserBoundsEqual(lastBoundsRef.current.get(profileId) ?? null, bounds)) {
        continue;
      }

      lastBoundsRef.current.set(profileId, bounds);
      try {
        await setMiniBrowserBounds(profileId, bounds);
      } catch {
        lastBoundsRef.current.delete(profileId);
      }
    }
  }, []);

  /** Leading-edge throttle: the first change is served 100ms later, at most. */
  const scheduleBoundsSync = React.useCallback(() => {
    if (syncTimerRef.current !== null) return;
    syncTimerRef.current = window.setTimeout(() => {
      syncTimerRef.current = null;
      // A hidden page has no usable rectangle; nothing to sync until it is back.
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

  // Layout changes reach the backend through one throttled path: the grid's own
  // resize, the window resizing, and any scrolling ancestor (`main` is the
  // scroll container, so the listener is registered in the capture phase).
  React.useEffect(() => {
    if (embeddedProfiles.length === 0) return undefined;

    const observer = new ResizeObserver(() => scheduleBoundsSync());
    for (const element of cellElementsRef.current.values()) observer.observe(element);

    const handleLayoutChange = () => scheduleBoundsSync();
    window.addEventListener('resize', handleLayoutChange);
    document.addEventListener('scroll', handleLayoutChange, true);
    scheduleBoundsSync();

    return () => {
      observer.disconnect();
      window.removeEventListener('resize', handleLayoutChange);
      document.removeEventListener('scroll', handleLayoutChange, true);
    };
  }, [embeddedProfiles, scheduleBoundsSync]);

  /**
   * Open a cell whose address was just requested.
   *
   * The cell only exists after the next render, so the rectangle is measured in
   * a frame instead of inside the click handler. Openings are sequential: every
   * call creates a native view, and firing them together makes the resulting
   * order unpredictable. A request whose cell has not been committed yet stays
   * pending for the next frame; one whose cell was removed (the account was
   * closed again before it opened) is dropped rather than retried forever.
   */
  const flushPendingOpens = React.useCallback(async () => {
    for (const [profileId, targetUrl] of [...pendingOpensRef.current]) {
      if (!embeddedProfilesRef.current.includes(profileId)) {
        pendingOpensRef.current.delete(profileId);
        continue;
      }

      const element = cellElementsRef.current.get(profileId);
      if (!element) continue;

      const bounds = toMiniBrowserBounds(element.getBoundingClientRect());
      if (!bounds) {
        // A hidden route has no layout, so its cells measure as an empty
        // rectangle. That is not a failure: stay pending and wait until the
        // page is the visible route again.
        if (!isActiveRef.current) continue;
        pendingOpensRef.current.delete(profileId);
        setEmbeddedProfiles((previous) => previous.filter((id) => id !== profileId));
        void message.error(t('miniBrowser.embedNotReady'));
        continue;
      }

      pendingOpensRef.current.delete(profileId);
      try {
        await openMiniBrowserEmbedded(profileId, targetUrl, bounds);
        setEmbeddedUrls((previous) => ({ ...previous, [profileId]: targetUrl }));
        lastBoundsRef.current.set(profileId, bounds);
        if (!overlayOpenRef.current) {
          await setMiniBrowserVisible(profileId, true);
        }
        await refresh();
      } catch (error) {
        // The cell was removed before its page opened (the account was closed
        // again) or its cell is no longer in the grid: nothing to report.
        if (!embeddedProfilesRef.current.includes(profileId)) continue;
        setEmbeddedProfiles((previous) => previous.filter((id) => id !== profileId));
        void message.error(
          t('miniBrowser.openFailed', {
            error: error instanceof Error ? error.message : String(error),
          }),
        );
      }
    }

  }, [refresh, t]);

  /**
   * Retry loop for pending opens.
   *
   * One frame is normally enough. When a cell is committed but still measures
   * empty, the retry waits `PENDING_OPEN_RETRY_MS` and tries again, up to
   * `MAX_PENDING_OPEN_RETRIES`; after that the cell is dropped with a message
   * instead of spinning frames forever. Anything still pending when this page is
   * not the visible route is left alone: the effect re-runs on return.
   */
  const runOpenFlush = React.useCallback(() => {
    if (openFlushFrameRef.current !== null) return;
    openFlushFrameRef.current = window.requestAnimationFrame(() => {
      openFlushFrameRef.current = null;
      void flushPendingOpens().then(() => {
        if (pendingOpensRef.current.size === 0) {
          openFlushAttemptsRef.current = 0;
          return;
        }
        // A request that arrived while another route was showing has no usable
        // rectangle yet; the layout effect re-runs on return and restarts this.
        if (!isActiveRef.current) return;

        openFlushAttemptsRef.current += 1;
        if (openFlushAttemptsRef.current > MAX_PENDING_OPEN_RETRIES) {
          const stuck = [...pendingOpensRef.current.keys()];
          openFlushAttemptsRef.current = 0;
          pendingOpensRef.current.clear();
          setEmbeddedProfiles((previous) => previous.filter((id) => !stuck.includes(id)));
          void message.error(t('miniBrowser.embedNotReady'));
          return;
        }

        openFlushTimerRef.current = window.setTimeout(() => {
          openFlushTimerRef.current = null;
          runOpenFlush();
        }, PENDING_OPEN_RETRY_MS);
      });
    });
  }, [flushPendingOpens, t]);

  // Runs after the cells of a new profile are committed (layout effects see the
  // DOM, and ref callbacks attach before them), which is the earliest moment
  // their rectangle exists.
  React.useLayoutEffect(() => {
    if (pendingOpensRef.current.size === 0 || !isActive) return undefined;
    runOpenFlush();
  }, [embeddedProfiles, isActive, runOpenFlush]);

  React.useEffect(() => {
    return () => {
      if (openFlushFrameRef.current !== null) {
        window.cancelAnimationFrame(openFlushFrameRef.current);
        openFlushFrameRef.current = null;
      }
      if (openFlushTimerRef.current !== null) {
        window.clearTimeout(openFlushTimerRef.current);
        openFlushTimerRef.current = null;
      }
    };
  }, []);

  /** Embed an account's page, reusing its cell when it is already open. */
  const openEmbedded = React.useCallback(
    (profileId: string, targetUrl: string) => {
      if (embeddedProfilesRef.current.includes(profileId)) {
        // Already embedded: the only "focus" available is to make it visible,
        // and only when nothing of this page is covering it.
        if (!overlayOpenRef.current) {
          void setMiniBrowserVisible(profileId, true).catch(() => undefined);
        }
        return;
      }

      pendingOpensRef.current.set(profileId, targetUrl);
      setEmbeddedProfiles((previous) =>
        previous.includes(profileId) ? previous : [...previous, profileId],
      );
    },
    [],
  );

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
   * Visibility follows the route and the page's own popovers.
   *
   * `isActive` covers both "another route is showing" (this page is kept alive
   * behind `display: none`) and the first mount; `overlayOpen` covers a native
   * view that would otherwise paint over an open popover.
   */
  React.useEffect(() => {
    const visible = isActive && !overlayOpen;
    for (const profileId of embeddedProfiles) {
      // A cell whose page has not been opened yet has no native view to show;
      // `flushPendingOpens` reveals it right after the open succeeds.
      if (pendingOpensRef.current.has(profileId)) continue;
      void setMiniBrowserVisible(profileId, visible).catch(() => undefined);
    }
    if (visible) {
      // The rectangles were unusable while hidden; re-pin them on return.
      lastBoundsRef.current.clear();
      scheduleBoundsSync();
    }
  }, [embeddedProfiles, isActive, overlayOpen, scheduleBoundsSync]);

  // A real unmount (LRU eviction, app shutdown) hides every embedded view: the
  // native views outlive the React tree, so leaving them visible would leave
  // orphan rectangles over whatever renders next.
  React.useEffect(() => {
    return () => {
      for (const profileId of embeddedProfilesRef.current) {
        void setMiniBrowserVisible(profileId, false).catch(() => undefined);
      }
    };
  }, []);

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
    openEmbedded(profileId, site.url);
  };

  const handleCloseEmbedded = async (profileId: string) => {
    forgetEmbeddedProfiles([profileId]);
    await closeMiniBrowserWindow(profileId).catch(() => undefined);
    await refresh();
  };

  const handleOpenAllAccounts = async () => {
    if (accounts.length === 0) {
      void message.warning(t('miniBrowser.noAccounts'));
      return;
    }
    setBusy(true);
    try {
      for (const account of accounts) {
        const site = sites.find((candidate) => candidate.id === account.siteId);
        if (!site) continue;
        openEmbedded(account.id, site.url);
      }
      void message.success(t('miniBrowser.openAllDone'));
    } finally {
      setBusy(false);
    }
  };

  const handleCloseAll = async () => {
    forgetEmbeddedProfiles([...embeddedProfilesRef.current]);
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
              className={
                openCount > 0 ? styles.statusDotActive : styles.statusDot
              }
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
                          const embedded = embeddedProfiles.includes(account.id);
                          const windowInfo = windowsByProfile.get(account.id);
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
                                    embedded || windowInfo
                                      ? styles.statusOpen
                                      : styles.statusClosed
                                  }
                                >
                                  {embedded
                                    ? t('miniBrowser.statusEmbedded')
                                    : windowInfo
                                      ? t('miniBrowser.statusOpened')
                                      : t('miniBrowser.statusClosed')}
                                </span>
                              </div>
                              <div className={styles.accountMeta}>
                                <span
                                  className={styles.accountUrl}
                                  title={embeddedUrls[account.id] ?? windowInfo?.url}
                                >
                                  {embeddedUrls[account.id]
                                    || windowInfo?.url
                                    || (embedded
                                      ? site.url
                                      : t('miniBrowser.noWindow'))}
                                </span>
                                <div className={styles.accountActions}>
                                  <Button
                                    size="small"
                                    type="primary"
                                    onClick={() => handleOpenAccount(site, account.id)}
                                  >
                                    {embedded || windowInfo
                                      ? t('miniBrowser.focusAccount')
                                      : t('miniBrowser.openAccount')}
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

        <section className={styles.embedArea} aria-label={t('miniBrowser.embedTitle')}>
          {embeddedProfiles.length === 0 ? (
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
            /*
              The cells stay empty on purpose: a native child view is painted
              above the DOM, so anything rendered here would only be hidden.
              Only the surrounding gaps and this page's own chrome stay visible.
            */
            <div
              className={styles.embedGrid}
              style={{
                gridTemplateColumns: `repeat(${gridColumns}, minmax(0, 1fr))`,
                gridTemplateRows: `repeat(${gridRows}, minmax(0, 1fr))`,
              }}
            >
              {embeddedProfiles.map((profileId) => {
                const label = embeddedLabel(profileId);
                return (
                  <div className={styles.embedSlot} key={profileId}>
                    {/*
                      The label strip sits outside the measured rectangle: it is
                      the only part of a slot React may draw, because everything
                      inside the rectangle is covered by the native page.
                    */}
                    <div className={styles.embedSlotHeader}>
                      <span className={styles.embedSlotName} title={label.name}>
                        {label.name}
                      </span>
                      <span className={styles.embedSlotUrl} title={label.url}>
                        {label.url || t('miniBrowser.noWindow')}
                      </span>
                      <button
                        type="button"
                        className={styles.cellClose}
                        aria-label={t('miniBrowser.closeTab')}
                        title={t('miniBrowser.closeTab')}
                        onClick={() => void handleCloseEmbedded(profileId)}
                      >
                        <X size={12} />
                      </button>
                    </div>
                    <div
                      className={styles.embedCell}
                      ref={getCellRef(profileId)}
                      data-profile-id={profileId}
                      aria-hidden="true"
                    />
                  </div>
                );
              })}
            </div>
          )}
        </section>
      </div>
    </div>
  );
};

export default MiniBrowserPage;
