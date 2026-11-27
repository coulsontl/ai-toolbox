import React from 'react';
import { Spin } from 'antd';
import { RefreshCw, Settings2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  copyPluginPageImage,
  invokePluginPageFiles,
  invokePluginPageKv,
  readPluginPageBridge,
  savePluginPageImage,
  setPluginPageBridgeOptions,
  type PluginPageSession,
  type RegisteredPluginPage,
} from '@/services';
import { formatPluginError, getPluginPageTitle } from '../utils/pluginFormatters';
import type { PluginBridgeTheme } from '../utils/pluginPageTheme';
import PluginPageGlyph from './PluginPageGlyph';
import styles from './PluginPageFrame.module.less';

const HELLO_TIMEOUT_MS = 10_000;
const MAX_IMAGE_BYTES = 20 * 1024 * 1024;

interface BridgeError {
  kind: 'timeout' | 'page' | 'navigation';
  detail: string;
}

/**
 * Accept the documented base64/data-URL string, and a `Blob` as a defensive
 * second shape because structured cloning lets pages push binary data through
 * the bridge without an encode step.
 */
const readImagePayload = (value: unknown): Promise<string | null> => {
  if (typeof value === 'string') {
    return Promise.resolve(value || null);
  }
  if (!(value instanceof Blob)) {
    return Promise.resolve(null);
  }
  if (value.size > MAX_IMAGE_BYTES) {
    return Promise.reject(new Error('image payload exceeds 20 MiB'));
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = typeof reader.result === 'string' ? reader.result : '';
      resolve(result || null);
    };
    reader.onerror = () => reject(reader.error ?? new Error('failed to read image blob'));
    reader.readAsDataURL(value);
  });
};

type BridgeResponse =
  | { ok: true; result?: unknown }
  | { ok: false; error: string };

interface PluginPageFrameProps {
  page: RegisteredPluginPage;
  session: PluginPageSession | null;
  sessionLoading: boolean;
  sessionError: string | null;
  visible: boolean;
  theme: PluginBridgeTheme;
  options: Record<string, unknown> | null;
  onRetrySession: () => void;
  onOptionsChanged: (pluginId: string, options: Record<string, unknown>) => void;
  onOpenOptions: (pluginId: string) => void;
  onOpenInstall: (packageName: string) => void;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);

const requiredString = (value: unknown): string | null =>
  typeof value === 'string' && value.trim() ? value : null;

/**
 * One sandboxed plugin page. It owns the Magpie Bridge v1 endpoint of its
 * iframe; the page sees only this window, and every capability is forwarded to
 * a typed Tauri command. No request is answered without `hello` first.
 */
const PluginPageFrame: React.FC<PluginPageFrameProps> = ({
  page,
  session,
  sessionLoading,
  sessionError,
  visible,
  theme,
  options,
  onRetrySession,
  onOptionsChanged,
  onOpenOptions,
  onOpenInstall,
}) => {
  const { t, i18n } = useTranslation();
  const title = getPluginPageTitle(page, i18n.language);
  const iframeRef = React.useRef<HTMLIFrameElement | null>(null);
  const helloTimerRef = React.useRef<number | null>(null);
  const handshakeRef = React.useRef(false);
  // Counts loads of the same iframe URL. A second load without a frame/session
  // change means the page navigated itself away from the trusted entry point.
  const loadCountRef = React.useRef(0);
  const [frameKey, setFrameKey] = React.useState(0);
  const [handshakeDone, setHandshakeDone] = React.useState(false);
  const [bridgeError, setBridgeError] = React.useState<BridgeError | null>(null);

  // The bridge listener is bound once per iframe, so everything it answers
  // reads from this latest-values ref instead of a stale closure.
  const liveRef = React.useRef({ theme, options, visible, onOptionsChanged, onOpenInstall });
  React.useLayoutEffect(() => {
    liveRef.current = { theme, options, visible, onOptionsChanged, onOpenInstall };
  }, [onOpenInstall, onOptionsChanged, options, theme, visible]);

  const describeError = React.useCallback(
    (error: unknown) => formatPluginError(error),
    [],
  );

  const sessionUrl = session?.url ?? null;

  // Per-frame secret passed in the iframe URL fragment; an external page that
  // self-navigates into this sandboxed iframe does not receive the fragment and
  // cannot spoof bridge messages even before `onLoad` fires.
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
    // Its origin no longer matches the trusted plugin page, so stop talking to
    // it rather than expose host capabilities to whatever it loaded.
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

  // A page hidden from the reader has not had its chance to answer yet; the
  // ten-second fuse only runs while the page is actually shown.
  React.useEffect(() => {
    if (!sessionUrl || !visible || handshakeRef.current) {
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
  }, [frameKey, i18n, sessionUrl, visible]);

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
      { magpie: 1, event: 'visibility', data: { visible } },
      '*',
    );
  }, [handshakeDone, session, visible]);

  // Attach before the parent paints: an iframe served from the local page
  // server can start talking as soon as it is in the DOM.
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

      const respond = (payload: BridgeResponse) => {
        iframe.contentWindow?.postMessage(
          { magpie: 1, id: requestId, ...payload },
          '*',
        );
      };

      const handleRequest = async (): Promise<BridgeResponse> => {
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
                context: { pluginId: page.pluginId, pageId: page.pageId },
                service: session.serviceUrl ?? null,
                visible: current.visible,
              },
            };
          }
          case 'options.get':
            return { ok: true, result: liveRef.current.options ?? {} };
          case 'options.set': {
            const nextOptions = isRecord(params.options) ? params.options : params;
            const stored = await setPluginPageBridgeOptions(
              page.pluginId,
              nextOptions,
            );
            liveRef.current = { ...liveRef.current, options: stored };
            liveRef.current.onOptionsChanged(page.pluginId, stored);
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
              page.pluginId,
              scope,
              isRecord(params.params) ? params.params : {},
            );
            return { ok: true, result };
          }
          case 'storage.get': {
            const key = requiredString(params.key);
            if (!key) {
              return { ok: false, error: 'storage.get requires a non-empty key' };
            }
            const result = await invokePluginPageKv({
              pluginId: page.pluginId,
              op: 'get',
              key,
            });
            return { ok: true, result: result ?? null };
          }
          case 'storage.set': {
            const key = requiredString(params.key);
            if (!key) {
              return { ok: false, error: 'storage.set requires a non-empty key' };
            }
            if (!Object.prototype.hasOwnProperty.call(params, 'value')) {
              return { ok: false, error: 'storage.set requires a value' };
            }
            await invokePluginPageKv({
              pluginId: page.pluginId,
              op: 'set',
              key,
              value: params.value,
            });
            return { ok: true, result: { ok: true } };
          }
          case 'storage.delete': {
            const key = requiredString(params.key);
            if (!key) {
              return { ok: false, error: 'storage.delete requires a non-empty key' };
            }
            const result = await invokePluginPageKv({
              pluginId: page.pluginId,
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
              pluginId: page.pluginId,
              op: 'list',
              prefix: typeof params.prefix === 'string' ? params.prefix : undefined,
            });
            return { ok: true, result };
          }
          case 'storage.clear': {
            await invokePluginPageKv({ pluginId: page.pluginId, op: 'clear' });
            return { ok: true, result: { ok: true } };
          }
          case 'files.readText': {
            const path = requiredString(params.path);
            if (!path) {
              return { ok: false, error: 'files.readText requires a non-empty path' };
            }
            const result = await invokePluginPageFiles({
              pluginId: page.pluginId,
              op: 'readText',
              path,
            });
            return { ok: true, result };
          }
          case 'files.writeText': {
            const path = requiredString(params.path);
            if (!path) {
              return { ok: false, error: 'files.writeText requires a non-empty path' };
            }
            if (typeof params.content !== 'string') {
              return { ok: false, error: 'files.writeText requires string content' };
            }
            const result = await invokePluginPageFiles({
              pluginId: page.pluginId,
              op: 'writeText',
              path,
              content: params.content,
            });
            return { ok: true, result };
          }
          case 'files.readBytes': {
            const path = requiredString(params.path);
            if (!path) {
              return { ok: false, error: 'files.readBytes requires a non-empty path' };
            }
            const result = await invokePluginPageFiles({
              pluginId: page.pluginId,
              op: 'readBytes',
              path,
            });
            return { ok: true, result };
          }
          case 'files.writeBytes': {
            const path = requiredString(params.path);
            if (!path) {
              return { ok: false, error: 'files.writeBytes requires a non-empty path' };
            }
            const content = params.content ?? params.base64;
            if (typeof content !== 'string') {
              return { ok: false, error: 'files.writeBytes requires base64 content' };
            }
            const result = await invokePluginPageFiles({
              pluginId: page.pluginId,
              op: 'writeBytes',
              path,
              content,
            });
            return { ok: true, result };
          }
          case 'files.list': {
            const path = requiredString(params.path);
            if (!path) {
              return { ok: false, error: 'files.list requires a non-empty path' };
            }
            const result = await invokePluginPageFiles({
              pluginId: page.pluginId,
              op: 'list',
              path,
            });
            return { ok: true, result };
          }
          case 'files.stat': {
            const path = requiredString(params.path);
            if (!path) {
              return { ok: false, error: 'files.stat requires a non-empty path' };
            }
            const result = await invokePluginPageFiles({
              pluginId: page.pluginId,
              op: 'stat',
              path,
            });
            return { ok: true, result };
          }
          case 'files.delete': {
            const path = requiredString(params.path);
            if (!path) {
              return { ok: false, error: 'files.delete requires a non-empty path' };
            }
            const result = await invokePluginPageFiles({
              pluginId: page.pluginId,
              op: 'delete',
              path,
            });
            return { ok: true, result };
          }
          case 'image.copy': {
            const data = await readImagePayload(params.data ?? params.png);
            if (!data) {
              return { ok: false, error: 'image.copy requires base64 data or a Blob' };
            }
            await copyPluginPageImage(data);
            return { ok: true, result: true };
          }
          case 'image.save': {
            const data = await readImagePayload(params.data ?? params.png);
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
            liveRef.current.onOpenInstall(packageName);
            return { ok: true, result: true };
          }
          default:
            return { ok: false, error: `unknown bridge method: ${message.method}` };
        }
      };

      void handleRequest().then(respond).catch((error) => {
        respond({ ok: false, error: describeError(error) });
      });
    };

    window.addEventListener('message', handleMessage);
    return () => window.removeEventListener('message', handleMessage);
  }, [bridgeNonce, describeError, page.pageId, page.pluginId, session, t]);

  const handleReload = () => {
    handshakeRef.current = false;
    loadCountRef.current = 0;
    setBridgeError(null);
    setHandshakeDone(false);
    setFrameKey((current) => current + 1);
  };

  const handleRetry = () => {
    if (sessionError) {
      onRetrySession();
      return;
    }
    handleReload();
  };

  const errorTitle = sessionError
    ? t('plugins.pageHost.openFailed')
    : bridgeError?.kind === 'timeout'
      ? t('plugins.pageHost.unresponsiveTitle')
      : bridgeError?.kind === 'navigation'
        ? t('plugins.pageHost.navigationBlockedTitle')
        : t('plugins.pageHost.errorTitle');
  const errorDetail = sessionError
    ?? bridgeError?.detail
    ?? '';

  const showFrame = Boolean(session) && !sessionError && !bridgeError;
  const showLoading = sessionLoading && !sessionError && !bridgeError;

  return (
    <div className={styles.frame} style={{ display: visible ? 'flex' : 'none' }}>
      <header className={styles.topbar}>
        <PluginPageGlyph
          iconDataUrl={page.iconDataUrl}
          pluginIcon={page.pluginIcon}
          pluginName={page.pluginName || page.packageName}
          title={title}
          size={24}
        />
        <div className={styles.heading}>
          <span className={styles.title} title={title}>{title}</span>
          <span className={styles.meta}>
            <span className={styles.pluginName} title={page.pluginName || page.packageName}>
              {page.pluginName || page.packageName}
            </span>
            {page.pluginVersion ? (
              <span className={styles.version}>v{page.pluginVersion}</span>
            ) : null}
          </span>
        </div>
        <div className={styles.actions}>
          <button
            type="button"
            className={styles.toolButton}
            aria-label={t('plugins.pageHost.reload')}
            onClick={handleReload}
          >
            <RefreshCw size={14} aria-hidden="true" />
            <span className={styles.toolLabel}>{t('plugins.pageHost.reload')}</span>
          </button>
          <button
            type="button"
            className={styles.toolButton}
            aria-label={t('plugins.pageHost.options')}
            onClick={() => onOpenOptions(page.pluginId)}
          >
            <Settings2 size={14} aria-hidden="true" />
            <span className={styles.toolLabel}>{t('plugins.pageHost.options')}</span>
          </button>
        </div>
      </header>

      <div className={styles.body}>
        {showFrame && session && frameSrc ? (
          <iframe
            key={frameKey}
            ref={iframeRef}
            className={styles.iframe}
            src={frameSrc}
            title={title}
            sandbox="allow-scripts"
            referrerPolicy="no-referrer"
            onLoad={handleFrameLoad}
          />
        ) : null}
        {showLoading ? (
          <div className={styles.placeholder}>
            <Spin size="small" />
            <span>{t('plugins.pageHost.loading')}</span>
          </div>
        ) : null}
        {!sessionLoading && !showFrame ? (
          <div className={styles.placeholder} role="alert">
            <span className={styles.errorTitle}>{errorTitle}</span>
            <p className={styles.errorDetail}>{errorDetail}</p>
            <button type="button" className={styles.retryButton} onClick={handleRetry}>
              <RefreshCw size={14} aria-hidden="true" />
              <span>
                {bridgeError?.kind === 'navigation'
                  ? t('plugins.pageHost.reload')
                  : t('plugins.pageHost.retry')}
              </span>
            </button>
          </div>
        ) : null}
      </div>
    </div>
  );
};

export default PluginPageFrame;
