import React from 'react';
import { Spin } from 'antd';
import { RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  closePluginPanel,
  copyPluginPageImage,
  invokePluginPageFiles,
  invokePluginPageKv,
  openPluginPanel,
  readPluginPageBridge,
  savePluginPageImage,
  setPluginPageBridgeOptions,
  type InstalledPluginRecord,
  type PluginPageSession,
  type RegisteredPluginPanel,
} from '@/services';
import {
  isPluginBridgeRecord,
  readPluginBridgeImagePayload,
  requirePluginBridgeString,
  type PluginBridgeResponse,
} from '../utils/pluginBridge';
import { formatPluginError, getPluginPageTitle } from '../utils/pluginFormatters';
import type { PluginBridgeTheme } from '../utils/pluginPageTheme';
import styles from './PluginPanelSlot.module.less';

const DEFAULT_PANEL_HEIGHT = 220;
const MIN_PANEL_HEIGHT = 80;
const MAX_PANEL_HEIGHT = 720;
const HELLO_TIMEOUT_MS = 10_000;

interface PanelBridgeError {
  kind: 'timeout' | 'page' | 'navigation';
  detail: string;
}

// One host token exists per panel key; opening replaces it. A stale close from
// an earlier mount (provider switch, React StrictMode) must not revoke the
// token a newer mount just opened, so each mount claims a generation.
const panelSessionGenerations = new Map<string, number>();

interface PluginPanelSlotProps {
  slot: 'provider' | 'settings';
  panels: RegisteredPluginPanel[];
  active: boolean;
  theme: PluginBridgeTheme;
  contextExtra?: Record<string, unknown>;
  installedPlugins: InstalledPluginRecord[];
  onOptionsChanged?: (pluginId: string, options: Record<string, unknown>) => void;
  onOpenInstall?: (packageName: string) => void;
}

interface PluginPanelFrameProps {
  panel: RegisteredPluginPanel;
  pluginName: string;
  active: boolean;
  theme: PluginBridgeTheme;
  contextExtra?: Record<string, unknown>;
  onOptionsChanged?: (pluginId: string, options: Record<string, unknown>) => void;
  onOpenInstall?: (packageName: string) => void;
}

/**
 * One sandboxed inline panel. It speaks the same Magpie Bridge v1 as a page
 * frame, but its context names the host slot and it follows the height the
 * page reports instead of filling its parent.
 */
const PluginPanelFrame: React.FC<PluginPanelFrameProps> = ({
  panel,
  pluginName,
  active,
  theme,
  contextExtra,
  onOptionsChanged,
  onOpenInstall,
}) => {
  const { t, i18n } = useTranslation();
  const iframeRef = React.useRef<HTMLIFrameElement | null>(null);
  const helloTimerRef = React.useRef<number | null>(null);
  const handshakeRef = React.useRef(false);
  // Counts loads of the same iframe URL. A second load without a frame/session
  // change means the panel navigated itself away from the trusted entry point.
  const loadCountRef = React.useRef(0);
  const optionsRef = React.useRef<Record<string, unknown> | null>(null);
  const [frameKey, setFrameKey] = React.useState(0);
  const [session, setSession] = React.useState<PluginPageSession | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [bridgeError, setBridgeError] = React.useState<PanelBridgeError | null>(null);
  const [handshakeDone, setHandshakeDone] = React.useState(false);
  const [frameHeight, setFrameHeight] = React.useState(DEFAULT_PANEL_HEIGHT);

  // The bridge listener is bound once per panel, so latest props live in a ref.
  const liveRef = React.useRef({ theme, active, onOptionsChanged, onOpenInstall });
  React.useLayoutEffect(() => {
    liveRef.current = { theme, active, onOptionsChanged, onOpenInstall };
  }, [active, onOpenInstall, onOptionsChanged, theme]);

  React.useEffect(() => {
    let disposed = false;
    const panelKey = `${panel.pluginId}:${panel.panelId}`;
    const generation = (panelSessionGenerations.get(panelKey) ?? 0) + 1;
    panelSessionGenerations.set(panelKey, generation);
    setLoading(true);
    setError(null);
    setSession(null);
    setBridgeError(null);
    setHandshakeDone(false);
    setFrameHeight(DEFAULT_PANEL_HEIGHT);
    handshakeRef.current = false;
    optionsRef.current = null;

    const opening = openPluginPanel(panel.pluginId, panel.panelId);
    opening.then((nextSession) => {
      if (disposed) {
        return;
      }
      optionsRef.current = isPluginBridgeRecord(nextSession.options)
        ? nextSession.options
        : {};
      setSession(nextSession);
      setLoading(false);
    }).catch((openError) => {
      if (disposed) {
        return;
      }
      setError(formatPluginError(openError));
      setLoading(false);
    });

    return () => {
      disposed = true;
      // Close only after a pending open settles, so the host token cannot be
      // created by a late answer after the cleanup already ran. Skip it when a
      // newer mount has already replaced this panel's token.
      void opening
        .then(() => {
          if (panelSessionGenerations.get(panelKey) !== generation) {
            return undefined;
          }
          return closePluginPanel(panel.pluginId, panel.panelId);
        })
        .catch(() => undefined);
    };
  }, [panel.panelId, panel.pluginId]);

  const sessionUrl = session?.url ?? null;

  const bridgeNonce = React.useMemo(() => {
    if (!sessionUrl) {
      return '';
    }
    if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
      return crypto.randomUUID();
    }
    return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  }, [frameKey, sessionUrl]);

  const frameSrc = React.useMemo(() => {
    if (!sessionUrl) {
      return null;
    }
    const separator = sessionUrl.includes('#') ? '&' : '#';
    return `${sessionUrl}${separator}_magpie_nonce=${encodeURIComponent(bridgeNonce)}`;
  }, [bridgeNonce, sessionUrl]);

  React.useLayoutEffect(() => {
    handshakeRef.current = false;
    loadCountRef.current = 0;
    setHandshakeDone(false);
    setBridgeError(null);
  }, [frameKey, sessionUrl]);

  const handleFrameLoad = React.useCallback(() => {
    loadCountRef.current += 1;
    if (loadCountRef.current <= 1) {
      return;
    }
    // The iframe navigated itself (link click, form submit, script redirect).
    // Its origin no longer matches the trusted plugin entry point, so stop
    // talking to it rather than expose host capabilities to whatever loaded.
    if (helloTimerRef.current !== null) {
      window.clearTimeout(helloTimerRef.current);
      helloTimerRef.current = null;
    }
    handshakeRef.current = false;
    setHandshakeDone(false);
    setBridgeError({
      kind: 'navigation',
      detail: t('plugins.pageHost.navigationBlockedDetail'),
    });
  }, [t]);

  // A panel that the reader cannot see has not had its chance to answer yet;
  // the ten-second fuse only runs while the panel is actually shown.
  React.useEffect(() => {
    if (!sessionUrl || !active || handshakeRef.current) {
      return undefined;
    }
    helloTimerRef.current = window.setTimeout(() => {
      setBridgeError({
        kind: 'timeout',
        detail: i18n.t('plugins.pageHost.timeoutDetail'),
      });
    }, HELLO_TIMEOUT_MS);
    return () => {
      if (helloTimerRef.current !== null) {
        window.clearTimeout(helloTimerRef.current);
        helloTimerRef.current = null;
      }
    };
  }, [active, frameKey, i18n, sessionUrl]);

  React.useEffect(() => {
    if (!session || !handshakeDone) {
      return;
    }
    iframeRef.current?.contentWindow?.postMessage(
      { magpie: 1, event: 'theme', data: theme },
      '*',
    );
  }, [handshakeDone, session, theme]);

  React.useEffect(() => {
    if (!session || !handshakeDone) {
      return;
    }
    iframeRef.current?.contentWindow?.postMessage(
      { magpie: 1, event: 'visibility', data: { visible: active } },
      '*',
    );
  }, [active, handshakeDone, session]);

  React.useLayoutEffect(() => {
    if (!session) {
      return undefined;
    }

    const handleMessage = (event: MessageEvent) => {
      const iframe = iframeRef.current;
      if (!iframe || event.source !== iframe.contentWindow) {
        return;
      }
      const message = event.data as {
        magpie?: number;
        nonce?: string;
        id?: number | string;
        method?: string;
        params?: Record<string, unknown>;
        error?: string;
      } | null;
      if (!message || message.magpie !== 1) {
        return;
      }

      // A panel's height report carries no nonce; it changes only this frame's
      // own box, so the source check above is its whole trust boundary.
      if (message.method === 'size') {
        const reported = Number(message.params?.height);
        if (Number.isFinite(reported)) {
          setFrameHeight(
            Math.min(MAX_PANEL_HEIGHT, Math.max(MIN_PANEL_HEIGHT, Math.round(reported))),
          );
        }
        return;
      }

      if (!bridgeNonce || message.nonce !== bridgeNonce) {
        return;
      }

      if (message.method === 'error') {
        if (helloTimerRef.current !== null) {
          window.clearTimeout(helloTimerRef.current);
          helloTimerRef.current = null;
        }
        setBridgeError({
          kind: 'page',
          detail: String(message.params?.message ?? t('plugins.pageHost.errorFallback')),
        });
        return;
      }

      const requestId = message.id;
      if (requestId === undefined || requestId === null || !message.method) {
        return;
      }

      const respond = (payload: PluginBridgeResponse) => {
        iframe.contentWindow?.postMessage(
          { magpie: 1, id: requestId, ...payload },
          '*',
        );
      };

      const handleRequest = async (): Promise<PluginBridgeResponse> => {
        const params = message.params ?? {};
        if (message.method !== 'hello' && !handshakeRef.current) {
          return { ok: false, error: 'hello required before bridge requests' };
        }
        switch (message.method) {
          case 'hello': {
            if (helloTimerRef.current !== null) {
              window.clearTimeout(helloTimerRef.current);
              helloTimerRef.current = null;
            }
            handshakeRef.current = true;
            setHandshakeDone(true);
            setBridgeError((current) => (current?.kind === 'timeout' ? null : current));
            const current = liveRef.current;
            return {
              ok: true,
              result: {
                dark: current.theme.dark,
                lang: current.theme.lang,
                tokens: current.theme.tokens,
                context: {
                  pluginId: panel.pluginId,
                  panelId: panel.panelId,
                  slot: panel.slot,
                  ...(contextExtra ?? {}),
                },
                service: session.serviceUrl ?? null,
                visible: current.active,
              },
            };
          }
          case 'options.get':
            return { ok: true, result: optionsRef.current ?? {} };
          case 'options.set': {
            const nextOptions = isPluginBridgeRecord(params.options) ? params.options : params;
            const stored = await setPluginPageBridgeOptions(panel.pluginId, nextOptions);
            optionsRef.current = stored;
            liveRef.current.onOptionsChanged?.(panel.pluginId, stored);
            return { ok: true, result: stored };
          }
          case 'read': {
            const scope = typeof params.scope === 'string'
              ? params.scope
              : typeof params.name === 'string'
                ? params.name
                : '';
            if (!scope) {
              return { ok: false, error: 'read requires a scope' };
            }
            const result = await readPluginPageBridge(
              panel.pluginId,
              scope,
              isPluginBridgeRecord(params.params) ? params.params : {},
            );
            return { ok: true, result };
          }
          case 'storage.get': {
            const key = requirePluginBridgeString(params.key);
            if (!key) {
              return { ok: false, error: 'storage.get requires a non-empty key' };
            }
            const result = await invokePluginPageKv({
              pluginId: panel.pluginId,
              op: 'get',
              key,
            });
            return { ok: true, result: result ?? null };
          }
          case 'storage.set': {
            const key = requirePluginBridgeString(params.key);
            if (!key) {
              return { ok: false, error: 'storage.set requires a non-empty key' };
            }
            if (!Object.prototype.hasOwnProperty.call(params, 'value')) {
              return { ok: false, error: 'storage.set requires a value' };
            }
            await invokePluginPageKv({
              pluginId: panel.pluginId,
              op: 'set',
              key,
              value: params.value,
            });
            return { ok: true, result: { ok: true } };
          }
          case 'storage.delete': {
            const key = requirePluginBridgeString(params.key);
            if (!key) {
              return { ok: false, error: 'storage.delete requires a non-empty key' };
            }
            const result = await invokePluginPageKv({
              pluginId: panel.pluginId,
              op: 'delete',
              key,
            });
            return { ok: true, result };
          }
          case 'storage.list': {
            if (params.prefix !== undefined && typeof params.prefix !== 'string') {
              return { ok: false, error: 'storage.list prefix must be a string' };
            }
            const result = await invokePluginPageKv({
              pluginId: panel.pluginId,
              op: 'list',
              prefix: typeof params.prefix === 'string' ? params.prefix : undefined,
            });
            return { ok: true, result };
          }
          case 'storage.clear': {
            await invokePluginPageKv({ pluginId: panel.pluginId, op: 'clear' });
            return { ok: true, result: { ok: true } };
          }
          case 'files.readText': {
            const path = requirePluginBridgeString(params.path);
            if (!path) {
              return { ok: false, error: 'files.readText requires a non-empty path' };
            }
            const result = await invokePluginPageFiles({
              pluginId: panel.pluginId,
              op: 'readText',
              path,
            });
            return { ok: true, result };
          }
          case 'files.writeText': {
            const path = requirePluginBridgeString(params.path);
            if (!path) {
              return { ok: false, error: 'files.writeText requires a non-empty path' };
            }
            if (typeof params.content !== 'string') {
              return { ok: false, error: 'files.writeText requires string content' };
            }
            const result = await invokePluginPageFiles({
              pluginId: panel.pluginId,
              op: 'writeText',
              path,
              content: params.content,
            });
            return { ok: true, result };
          }
          case 'files.readBytes': {
            const path = requirePluginBridgeString(params.path);
            if (!path) {
              return { ok: false, error: 'files.readBytes requires a non-empty path' };
            }
            const result = await invokePluginPageFiles({
              pluginId: panel.pluginId,
              op: 'readBytes',
              path,
            });
            return { ok: true, result };
          }
          case 'files.writeBytes': {
            const path = requirePluginBridgeString(params.path);
            if (!path) {
              return { ok: false, error: 'files.writeBytes requires a non-empty path' };
            }
            const content = params.content ?? params.base64;
            if (typeof content !== 'string') {
              return { ok: false, error: 'files.writeBytes requires base64 content' };
            }
            const result = await invokePluginPageFiles({
              pluginId: panel.pluginId,
              op: 'writeBytes',
              path,
              content,
            });
            return { ok: true, result };
          }
          case 'files.list': {
            const path = requirePluginBridgeString(params.path);
            if (!path) {
              return { ok: false, error: 'files.list requires a non-empty path' };
            }
            const result = await invokePluginPageFiles({
              pluginId: panel.pluginId,
              op: 'list',
              path,
            });
            return { ok: true, result };
          }
          case 'files.stat': {
            const path = requirePluginBridgeString(params.path);
            if (!path) {
              return { ok: false, error: 'files.stat requires a non-empty path' };
            }
            const result = await invokePluginPageFiles({
              pluginId: panel.pluginId,
              op: 'stat',
              path,
            });
            return { ok: true, result };
          }
          case 'files.delete': {
            const path = requirePluginBridgeString(params.path);
            if (!path) {
              return { ok: false, error: 'files.delete requires a non-empty path' };
            }
            const result = await invokePluginPageFiles({
              pluginId: panel.pluginId,
              op: 'delete',
              path,
            });
            return { ok: true, result };
          }
          case 'image.copy': {
            const data = await readPluginBridgeImagePayload(params.data ?? params.png);
            if (!data) {
              return { ok: false, error: 'image.copy requires base64 data or a Blob' };
            }
            await copyPluginPageImage(data);
            return { ok: true, result: true };
          }
          case 'image.save': {
            const data = await readPluginBridgeImagePayload(params.data ?? params.png);
            if (!data) {
              return { ok: false, error: 'image.save requires base64 data or a Blob' };
            }
            const filename = typeof params.filename === 'string'
              ? params.filename
              : typeof params.name === 'string'
                ? params.name
                : undefined;
            const path = await savePluginPageImage(data, filename);
            return { ok: true, result: { path } };
          }
          case 'plugin.open': {
            const packageName = typeof params.name === 'string'
              ? params.name
              : typeof params.package === 'string'
                ? params.package
                : '';
            if (!packageName) {
              return { ok: false, error: 'plugin.open requires a package name' };
            }
            liveRef.current.onOpenInstall?.(packageName);
            return { ok: true, result: true };
          }
          default:
            return { ok: false, error: `unknown bridge method: ${message.method}` };
        }
      };

      void handleRequest().then(respond).catch((requestError) => {
        respond({ ok: false, error: formatPluginError(requestError) });
      });
    };

    window.addEventListener('message', handleMessage);
    return () => window.removeEventListener('message', handleMessage);
  }, [bridgeNonce, contextExtra, panel.panelId, panel.pluginId, panel.slot, session, t]);

  const panelTitle = getPluginPageTitle(
    { title: panel.title || pluginName, titles: panel.titles ?? {} },
    i18n.language,
  );
  const handleReload = () => {
    handshakeRef.current = false;
    loadCountRef.current = 0;
    setBridgeError(null);
    setHandshakeDone(false);
    setFrameKey((current) => current + 1);
  };

  const errorTitle = error
    ? t('plugins.panels.loadFailed')
    : bridgeError?.kind === 'timeout'
      ? t('plugins.pageHost.unresponsiveTitle')
      : bridgeError?.kind === 'navigation'
        ? t('plugins.pageHost.navigationBlockedTitle')
        : t('plugins.pageHost.errorTitle');
  const errorDetail = error ?? bridgeError?.detail ?? '';
  // A failed panel may have reported a tiny height before failing; keep the
  // error message and its retry button readable instead of clipping them.
  const bodyHeight = error || bridgeError
    ? Math.max(frameHeight, DEFAULT_PANEL_HEIGHT)
    : frameHeight;
  const showFrame = Boolean(session) && !error && !bridgeError;
  const showLoading = loading && !error && !bridgeError;

  return (
    <div className={styles.panel}>
      <div className={styles.panelHeader}>
        <span className={styles.panelTitle} title={panelTitle}>{panelTitle}</span>
        <span className={styles.panelPlugin} title={pluginName}>{pluginName}</span>
      </div>
      <div className={styles.panelBody} style={{ height: bodyHeight }}>
        {showFrame && frameSrc ? (
          <iframe
            key={frameKey}
            ref={iframeRef}
            className={styles.iframe}
            src={frameSrc}
            title={panelTitle}
            sandbox="allow-scripts"
            referrerPolicy="no-referrer"
            onLoad={handleFrameLoad}
          />
        ) : null}
        {showLoading ? (
          <div className={styles.panelPlaceholder}>
            <Spin size="small" />
            <span>{t('plugins.pageHost.loading')}</span>
          </div>
        ) : null}
        {!loading && (error || bridgeError) ? (
          <div className={styles.panelPlaceholder} role="alert">
            <span className={styles.panelError}>{errorTitle}</span>
            <span className={styles.panelErrorDetail}>{errorDetail}</span>
            {bridgeError ? (
              <button
                type="button"
                className={styles.panelRetry}
                onClick={handleReload}
              >
                <RefreshCw size={13} aria-hidden="true" />
                <span>
                  {bridgeError.kind === 'navigation'
                    ? t('plugins.pageHost.reload')
                    : t('plugins.pageHost.retry')}
                </span>
              </button>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  );
};

/**
 * Hosts the inline panels a plugin registers for one stable slot. No panel for
 * the slot means no DOM and no session requests at all.
 */
const PluginPanelSlot: React.FC<PluginPanelSlotProps> = ({
  slot,
  panels,
  active,
  theme,
  contextExtra,
  installedPlugins,
  onOptionsChanged,
  onOpenInstall,
}) => {
  const panelsForSlot = panels.filter((panel) => panel.slot === slot);
  if (panelsForSlot.length === 0) {
    return null;
  }
  // A different provider context needs a fresh handshake, because the Magpie
  // bridge exposes `context` once at ready time.
  const contextKey = JSON.stringify(contextExtra ?? {});

  return (
    <section className={styles.slot}>
      {panelsForSlot.map((panel) => {
        const displayName = panel.pluginName
          || installedPlugins.find((plugin) => plugin.id === panel.pluginId)?.displayName
          || panel.pluginId;
        return (
          <PluginPanelFrame
            key={`${panel.pluginId}:${panel.panelId}:${contextKey}`}
            panel={panel}
            pluginName={displayName}
            active={active}
            theme={theme}
            contextExtra={contextExtra}
            onOptionsChanged={onOptionsChanged}
            onOpenInstall={onOpenInstall}
          />
        );
      })}
    </section>
  );
};

export default PluginPanelSlot;
