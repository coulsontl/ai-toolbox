import React from 'react';
import { Checkbox, DatePicker, Dropdown, Empty, Input, Modal, Pagination, Select, Table } from 'antd';
import type { MenuProps } from 'antd';
import type { ColumnsType } from 'antd/es/table';
import { save } from '@tauri-apps/plugin-dialog';
import {
  AlertCircle,
  CalendarDays,
  Check,
  ChevronDown,
  ChevronUp,
  Copy,
  Download,
  Ellipsis,
  FileText,
  Loader2,
  Network,
  RefreshCw,
  Search,
  Terminal,
  X,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  exportProxyGatewayRequestLogDetail,
  getProxyGatewayRequestLogDetail,
  getProxyGatewaySessionRebuildProgress,
  importProxyGatewaySessionUsage,
  startProxyGatewaySessionRebuild,
  listProxyGatewayRequestLogs,
  type GatewaySessionRebuildProgress,
  type GatewayUsageTool,
  GATEWAY_USAGE_TOOLS,
  type GatewayRequestLogDetail,
  type GatewayRequestLogFilters,
  type GatewayRequestLogItem,
} from '@/services';
import {
  calculateCacheHitRate,
  deriveGatewayRequestDisplay,
  formatCompactInteger,
  formatDateTime,
  formatDuration,
  formatCacheHitRate,
  formatGatewayError,
  gatewayTransportBadgeKey,
  gatewayWebSocketStatusKey,
  formatInteger,
  formatModelQualifiers,
  formatTps,
  compactionModeKey,
  nativeToolBadges,
  serviceTierLabel,
  formatUsd,
  GATEWAY_USAGE_RANGE_PRESETS,
  isGatewayRequestUsageApplicable,
  joinClassNames,
  normalizeAttemptCounts,
  requestExportPrefix,
  requestLineText,
  resolveGatewayRequestRange,
  sanitizeGatewayFileNamePart,
  sessionDurationText,
  shouldShowBodyComparison,
  stringifyDetailValue,
  type GatewayRequestRangeSelection,
} from '../utils/gatewayFormatters';
import styles from './GatewayRequestsView.module.less';

const { RangePicker } = DatePicker;

type RequestDetailTabKey = 'record' | 'body' | 'headers' | 'response';
type GatewayCliFilter = 'all' | GatewayUsageTool;

const REQUEST_DETAIL_TABS: RequestDetailTabKey[] = ['record', 'body', 'headers', 'response'];
const COLLAPSED_LINE_LIMIT = 10;
const COLLAPSED_CHARACTER_LIMIT = 8_000;
const PAGE_SIZE = 20;
const EXPORT_NOTICE_DURATION_MS = 3000;
/** A rebuild is minutes long; one second keeps the phase label responsive. */
const REBUILD_POLL_INTERVAL_MS = 1000;
const EXCLUDE_MODEL_LIST_STORAGE_KEY = 'gateway.requests.excludeModelList';

const readExcludeModelListPreference = (): boolean => {
  try {
    return window.localStorage.getItem(EXCLUDE_MODEL_LIST_STORAGE_KEY) === '1';
  } catch {
    return false;
  }
};

const writeExcludeModelListPreference = (checked: boolean) => {
  try {
    if (checked) {
      window.localStorage.setItem(EXCLUDE_MODEL_LIST_STORAGE_KEY, '1');
    } else {
      window.localStorage.removeItem(EXCLUDE_MODEL_LIST_STORAGE_KEY);
    }
  } catch {
    // ignore quota / private-mode failures
  }
};

const ONLY_FAILED_STORAGE_KEY = 'gateway.requests.onlyFailed';

const readOnlyFailedPreference = (): boolean => {
  try {
    return window.localStorage.getItem(ONLY_FAILED_STORAGE_KEY) === '1';
  } catch {
    return false;
  }
};

const writeOnlyFailedPreference = (checked: boolean) => {
  try {
    if (checked) {
      window.localStorage.setItem(ONLY_FAILED_STORAGE_KEY, '1');
    } else {
      window.localStorage.removeItem(ONLY_FAILED_STORAGE_KEY);
    }
  } catch {
    // ignore quota / private-mode failures
  }
};

interface GatewayRequestsViewProps {
  refreshKey?: number;
}

interface RequestFilterDraft {
  cliKey: GatewayCliFilter;
  dataSource: 'all' | 'proxy' | 'session';
  statusCode: string;
  providerName: string;
  model: string;
  range: GatewayRequestRangeSelection;
}

const defaultDraft: RequestFilterDraft = {
  cliKey: 'all',
  dataSource: 'all',
  statusCode: 'all',
  providerName: '',
  model: '',
  range: { preset: 'all' },
};

const lineCountOf = (content: string) => content.split(/\r\n|\r|\n/).length;

const tokenBreakdownText = (
  t: ReturnType<typeof useTranslation>['t'],
  value: Pick<
    GatewayRequestLogItem | GatewayRequestLogDetail,
    'input_tokens' | 'output_tokens' | 'cache_read_tokens' | 'cache_creation_tokens' | 'total_tokens'
  >,
) => {
  const breakdown = t('gateway.page.requests.tokensValue', {
    input: formatInteger(value.input_tokens),
    output: formatInteger(value.output_tokens),
    cacheRead: formatInteger(value.cache_read_tokens),
    cacheCreation: formatInteger(value.cache_creation_tokens),
    total: formatInteger(value.total_tokens),
  });
  const extra = (value.total_tokens ?? 0) - (value.input_tokens ?? 0) - (value.output_tokens ?? 0)
    - (value.cache_read_tokens ?? 0) - (value.cache_creation_tokens ?? 0);
  return extra > 0 ? `${breakdown} · ${t('gateway.page.requests.nativeUsage.extraShort', { value: formatInteger(extra) })}` : breakdown;
};

const providerDisplayName = (
  t: ReturnType<typeof useTranslation>['t'],
  providerId?: string | null,
  providerName?: string | null,
  nativeProvider?: string | null,
) => {
  if (providerName) {
    return providerName;
  }
  if (providerId === 'session') {
    const source = t('gateway.page.requests.localSession');
    return nativeProvider ? `${source} · ${nativeProvider}` : source;
  }
  if (!providerId || providerId === 'unknown') {
    return t('gateway.page.requests.providerUnselected');
  }
  return providerId;
};

const providerDisplayMeta = (
  t: ReturnType<typeof useTranslation>['t'],
  cliKey: GatewayUsageTool,
  providerId?: string | null,
) => {
  const cliLabel = t(`settings.gateway.cli.${cliKey}`);
  if (!providerId || providerId === 'unknown' || providerId === 'session') {
    return cliLabel;
  }
  return `${cliLabel} · ${providerId}`;
};

const buildRequestDetailExportFileName = (detail: GatewayRequestLogDetail) => {
  const model = requestExportPrefix(detail);
  const traceId = sanitizeGatewayFileNamePart(detail.trace_id);
  const time = sanitizeGatewayFileNamePart(detail.ended_at?.replace(/[T:]/g, '-').replace(/\.\d+Z?$/, 'Z'));
  return `${model}-${traceId}-${time}.json`;
};

const buildFilters = (draft: RequestFilterDraft): GatewayRequestLogFilters => {
  return {
    data_source: draft.dataSource === 'all' ? null : draft.dataSource,
    cli_key: draft.cliKey === 'all' ? null : draft.cliKey,
    status_code: draft.statusCode === 'all' ? null : Number(draft.statusCode),
    provider_name: draft.providerName.trim() || null,
    model: draft.model.trim() || null,
  };
};

interface CollapsiblePreProps {
  content: string | null | undefined;
  fallback: string;
}

const CollapsiblePre: React.FC<CollapsiblePreProps> = ({ content, fallback }) => {
  const { t } = useTranslation();
  const [expanded, setExpanded] = React.useState(false);
  const [copied, setCopied] = React.useState(false);
  const copyTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  React.useEffect(() => {
    setExpanded(false);
    setCopied(false);
    if (copyTimerRef.current) {
      clearTimeout(copyTimerRef.current);
      copyTimerRef.current = null;
    }
  }, [content]);

  React.useEffect(
    () => () => {
      if (copyTimerRef.current) {
        clearTimeout(copyTimerRef.current);
      }
    },
    [],
  );

  if (content == null) {
    return <pre className={styles.detailPre}>{fallback}</pre>;
  }

  const lineCount = lineCountOf(content);
  const collapsible = lineCount > COLLAPSED_LINE_LIMIT || content.length > COLLAPSED_CHARACTER_LIMIT;

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(content);
    } catch {
      return;
    }
    setCopied(true);
    if (copyTimerRef.current) {
      clearTimeout(copyTimerRef.current);
    }
    copyTimerRef.current = setTimeout(() => {
      setCopied(false);
      copyTimerRef.current = null;
    }, 1500);
  };

  return (
    <div className={styles.collapsiblePre}>
      <div className={styles.preToolbar}>
        <span className={styles.preLineCount}>
          {t('gateway.page.requests.lines', { count: lineCount })}
        </span>
        <span className={styles.preActions}>
          {collapsible ? (
            <button
              type="button"
              className={styles.preAction}
              onClick={() => setExpanded((previousExpanded) => !previousExpanded)}
            >
              {expanded ? <ChevronUp size={13} aria-hidden="true" /> : <ChevronDown size={13} aria-hidden="true" />}
              <span>{expanded ? t('gateway.page.requests.collapse') : t('gateway.page.requests.expand')}</span>
            </button>
          ) : null}
          <button
            type="button"
            className={styles.preAction}
            onClick={() => void handleCopy()}
          >
            {copied ? <Check size={13} aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}
            <span>{copied ? t('common.copied') : t('common.copy')}</span>
          </button>
        </span>
      </div>
      <pre
        className={joinClassNames(
          styles.detailPre,
          collapsible && !expanded && styles.detailPreCollapsed,
        )}
      >
        {content}
      </pre>
    </div>
  );
};

const GatewayRequestsView: React.FC<GatewayRequestsViewProps> = ({ refreshKey = 0 }) => {
  const { t } = useTranslation();
  const [draft, setDraft] = React.useState<RequestFilterDraft>(defaultDraft);
  const [appliedRange, setAppliedRange] = React.useState(defaultDraft.range);
  const [filters, setFilters] = React.useState<GatewayRequestLogFilters>(() => ({
    exclude_model_list: readExcludeModelListPreference() ? true : null,
    only_failed: readOnlyFailedPreference() ? true : null,
  }));
  const [page, setPage] = React.useState(1);
  const [importRefreshRevision, setImportRefreshRevision] = React.useState(0);
  const [logs, setLogs] = React.useState<GatewayRequestLogItem[]>([]);
  const [total, setTotal] = React.useState(0);
  const [selectedTraceId, setSelectedTraceId] = React.useState<string | null>(null);
  const [detail, setDetail] = React.useState<GatewayRequestLogDetail | null>(null);
  const [activeDetailTab, setActiveDetailTab] = React.useState<RequestDetailTabKey>('record');
  const [loading, setLoading] = React.useState(false);
  const [detailLoading, setDetailLoading] = React.useState(false);
  // The toolbar refresh runs a session sync too, but it must not light up the
  // overflow menu: that spinner belongs to the rebuild alone. A shared flag
  // made the ellipsis spin on every refresh.
  const [syncingForRefresh, setSyncingForRefresh] = React.useState(false);
  const [rebuilding, setRebuilding] = React.useState(false);
  const [rebuildProgress, setRebuildProgress] = React.useState<GatewaySessionRebuildProgress | null>(null);
  const [exportingDetail, setExportingDetail] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [notice, setNotice] = React.useState<string | null>(null);
  const [noticeKind, setNoticeKind] = React.useState<'success' | 'warning'>('success');
  const requestRevisionRef = React.useRef(0);
  const detailRevisionRef = React.useRef(0);
  const selectedTraceIdRef = React.useRef<string | null>(null);
  const exportNoticeTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearExportNoticeTimer = React.useCallback(() => {
    if (exportNoticeTimerRef.current) {
      clearTimeout(exportNoticeTimerRef.current);
      exportNoticeTimerRef.current = null;
    }
  }, []);

  React.useEffect(
    () => () => {
      clearExportNoticeTimer();
    },
    [clearExportNoticeTimer],
  );

  const closeDetail = React.useCallback(() => {
    detailRevisionRef.current += 1;
    selectedTraceIdRef.current = null;
    setSelectedTraceId(null);
    setDetail(null);
  }, []);

  const loadRequests = React.useCallback(async () => {
    const revision = ++requestRevisionRef.current;
    setLoading(true);
    setError(null);
    try {
      const requestFilters = { ...filters, ...resolveGatewayRequestRange(appliedRange) };
      const result = await listProxyGatewayRequestLogs(requestFilters, Math.max(page - 1, 0), PAGE_SIZE);
      if (revision !== requestRevisionRef.current) return;
      setLogs(result.data);
      setTotal(result.total);
      if (!result.data.some((log) => log.trace_id === selectedTraceIdRef.current)) {
        closeDetail();
      }
    } catch (loadError) {
      if (revision === requestRevisionRef.current) {
        setError(t('gateway.page.requests.loadFailed', { error: formatGatewayError(loadError) }));
      }
    } finally {
      if (revision === requestRevisionRef.current) setLoading(false);
    }
  }, [appliedRange, closeDetail, filters, page, t]);

  const loadDetail = React.useCallback(
    async (traceId: string) => {
      const revision = ++detailRevisionRef.current;
      selectedTraceIdRef.current = traceId;
      setSelectedTraceId(traceId);
      setDetail(null);
      setDetailLoading(true);
      setError(null);
      try {
        const nextDetail = await getProxyGatewayRequestLogDetail(traceId);
        if (revision !== detailRevisionRef.current) return;
        setDetail(nextDetail);
        setActiveDetailTab('record');
      } catch (detailError) {
        if (revision === detailRevisionRef.current) {
          setError(t('gateway.page.requests.loadFailed', { error: formatGatewayError(detailError) }));
        }
      } finally {
        if (revision === detailRevisionRef.current) setDetailLoading(false);
      }
    },
    [t],
  );

  React.useEffect(() => {
    void loadRequests();
    return () => { requestRevisionRef.current += 1; };
  }, [loadRequests, refreshKey, importRefreshRevision]);

  const applyFilters = () => {
    setAppliedRange(draft.range);
    setFilters((current) => ({
      ...buildFilters(draft),
      // Title-bar switches are independent from the search form.
      exclude_model_list: current.exclude_model_list,
      only_failed: current.only_failed,
    }));
    setPage(1);
  };

  /**
   * Commits a discrete filter choice immediately.
   *
   * Dropdowns are single deliberate actions, so deferring them behind the
   * search button only makes the list look broken — pick a CLI, see nothing
   * move. The text inputs keep the draft: those fire on every keystroke, and
   * searching per keystroke is what the split exists to prevent. The title-bar
   * switches already apply immediately, so this also makes the bar internally
   * consistent.
   */
  const applyDraft = (patch: Partial<RequestFilterDraft>) => {
    const next = { ...draft, ...patch };
    setDraft(next);
    setAppliedRange(next.range);
    setFilters((current) => ({
      ...buildFilters(next),
      exclude_model_list: current.exclude_model_list,
      only_failed: current.only_failed,
    }));
    setPage(1);
  };

  const resetFilters = () => {
    setDraft(defaultDraft);
    setAppliedRange(defaultDraft.range);
    setFilters((current) => ({
      exclude_model_list: current.exclude_model_list,
      only_failed: current.only_failed,
    }));
    setPage(1);
  };

  const handleExcludeModelListChange = (checked: boolean) => {
    writeExcludeModelListPreference(checked);
    setFilters((current) => ({
      ...current,
      exclude_model_list: checked ? true : null,
    }));
    setPage(1);
  };

  const handleOnlyFailedChange = (checked: boolean) => {
    writeOnlyFailedPreference(checked);
    setFilters((current) => ({
      ...current,
      only_failed: checked ? true : null,
    }));
    setPage(1);
  };

  /**
   * Toolbar refresh: pull the local CLI sessions in, then reload this page.
   *
   * This is the everyday "show me what just happened" action: it keeps the
   * current page and stays quiet on success — the refreshed rows are the
   * feedback. Only files that failed to sync get a notice, because nothing
   * else would surface them.
   */
  const handleRefreshWithSessionSync = async () => {
    setSyncingForRefresh(true);
    setError(null);
    setNotice(null);
    try {
      const result = await importProxyGatewaySessionUsage({ cli_key: 'all' });
      if (result.failed_files > 0) {
        setNotice(t('gateway.page.requests.refreshSyncPartialFailure', {
          failed: formatInteger(result.failed_files),
        }));
        setNoticeKind('warning');
      }
      await loadRequests();
    } catch (refreshError) {
      setError(t('gateway.page.requests.loadFailed', { error: formatGatewayError(refreshError) }));
      // A failed sync must not hide the rows already on screen; still try to
      // reload them so the button always does what its icon promises.
      await loadRequests();
    } finally {
      setSyncingForRefresh(false);
    }
  };

  /**
   * Backup, clear and re-import. Destructive enough to confirm first: the
   * backend takes a database backup and refuses an empty source set, but the
   * confirmation states what a rebuild means so the button is not a mystery.
   *
   * The rebuild itself runs in the background: a full history takes minutes,
   * and awaiting it here left the toolbar spinning with no phase or progress
   * and no way to leave the page. The poll below keeps the phase visible while
   * the task runs and refreshes the list once it finishes.
   */
  const finishRebuild = React.useCallback((progress: GatewaySessionRebuildProgress) => {
    if (progress.phase === 'failed') {
      setRebuildProgress(null);
      setRebuilding(false);
      setError(t('gateway.page.requests.rebuildFailed', {
        error: progress.error ?? '',
      }));
      return;
    }
    setRebuildProgress(null);
    setRebuilding(false);
    setNotice(t(progress.failed_files > 0
      ? 'gateway.page.requests.importPartialFailure'
      : 'gateway.page.requests.rebuildDone', {
      inserted: formatInteger(progress.inserted_records),
      updated: formatInteger(progress.updated_records),
      skipped: formatInteger(progress.skipped_records),
      files: formatInteger(progress.total_files),
      failed: formatInteger(progress.failed_files),
      count: progress.failed_files,
    }));
    setNoticeKind(progress.failed_files > 0 ? 'warning' : 'success');
    setPage(1);
    setImportRefreshRevision((revision) => revision + 1);
  }, [t]);

  // The rebuild lives in the backend, not in this component. Switching tabs
  // unmounts the whole view, so local state cannot be the source of truth:
  // every mount asks the backend whether a task is running and adopts it.
  // Without that, the progress bar disappeared with the unmounted component
  // and a rebuild that finished while the user was away was never reported.
  React.useEffect(() => {
    let cancelled = false;
    // The finished snapshot stays readable so this mount can report the
    // outcome, but the timer must not report it again every second.
    let reported = false;
    const poll = () => {
      void getProxyGatewaySessionRebuildProgress().then((progress) => {
        if (cancelled || !progress) {
          return;
        }
        if (progress.phase === 'done' || progress.phase === 'failed') {
          if (reported) {
            return;
          }
          reported = true;
          setRebuildProgress(null);
          finishRebuild(progress);
          return;
        }
        setRebuildProgress(progress);
        setRebuilding(true);
      }).catch((pollError) => {
        if (cancelled) {
          return;
        }
        setRebuilding(false);
        setRebuildProgress(null);
        setError(t('gateway.page.requests.rebuildFailed', {
          error: formatGatewayError(pollError),
        }));
      });
    };
    poll();
    const timer = window.setInterval(poll, REBUILD_POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [finishRebuild, t]);

  const handleRebuildSessionUsage = () => {
    Modal.confirm({
      title: t('gateway.page.requests.rebuildConfirmTitle'),
      content: t('gateway.page.requests.rebuildConfirmContent'),
      okText: t('gateway.page.requests.rebuildConfirmOk'),
      okButtonProps: { danger: true },
      cancelText: t('common.cancel'),
      onOk: async () => {
        setRebuilding(true);
        setRebuildProgress(null);
        setError(null);
        setNotice(null);
        try {
          await startProxyGatewaySessionRebuild({ cli_key: 'all' });
        } catch (rebuildError) {
          setRebuilding(false);
          setError(t('gateway.page.requests.rebuildFailed', { error: formatGatewayError(rebuildError) }));
        }
      },
    });
  };

  /**
   * The rebuild is the only action that needs its consequences spelled out, so
   * it lives behind an overflow menu instead of a labelled toolbar button.
   * Incremental syncing is the refresh button's job; duplicating it here made
   * two entries that looked different but did the same thing.
   */
  const sessionUsageMenuItems = React.useMemo<MenuProps['items']>(
    () => [
      {
        key: 'rebuild',
        icon: <RefreshCw size={14} aria-hidden="true" />,
        label: (
          <span className={styles.menuItemText}>
            <span>{t('gateway.page.requests.rebuildSessions')}</span>
            <small>{t('gateway.page.requests.rebuildSessionsHint')}</small>
          </span>
        ),
        danger: true,
      },
    ],
    [t],
  );

  const handleSessionUsageMenuClick: MenuProps['onClick'] = ({ key }) => {
    if (syncingForRefresh || rebuilding) {
      return;
    }
    if (key === 'rebuild') {
      void handleRebuildSessionUsage();
    }
  };

  const handleExportDetail = async () => {
    if (!detail || exportingDetail) {
      return;
    }
    const exportPath = await save({
      title: t('gateway.page.requests.exportDetail'),
      defaultPath: buildRequestDetailExportFileName(detail),
      filters: [
        {
          name: 'JSON',
          extensions: ['json'],
        },
      ],
    });
    if (!exportPath) {
      return;
    }

    setExportingDetail(true);
    setError(null);
    setNotice(null);
    clearExportNoticeTimer();
    try {
      await exportProxyGatewayRequestLogDetail(detail.trace_id, exportPath);
      const exportDoneNotice = t('gateway.page.requests.exportDone');
      setNoticeKind('success');
      setNotice(exportDoneNotice);
      exportNoticeTimerRef.current = setTimeout(() => {
        setNotice((currentNotice) => (currentNotice === exportDoneNotice ? null : currentNotice));
        exportNoticeTimerRef.current = null;
      }, EXPORT_NOTICE_DURATION_MS);
    } catch (exportError) {
      setError(t('gateway.page.requests.loadFailed', { error: formatGatewayError(exportError) }));
    } finally {
      setExportingDetail(false);
    }
  };

  const renderDetailContent = () => {
    if (detailLoading) {
      return (
        <div className={styles.emptyState}>
          <RefreshCw size={18} className={styles.spin} aria-hidden="true" />
          <span>{t('common.loading')}</span>
        </div>
      );
    }
    if (!detail) {
      return (
        <div className={styles.emptyState}>
          <FileText size={18} aria-hidden="true" />
          <span>{t('gateway.page.requests.detailEmpty')}</span>
        </div>
      );
    }

    const detailEmptyMessage = detail.data_source === 'session'
      ? t('gateway.page.requests.localSessionDetailUnavailable')
      : t('gateway.page.requests.notStored');

    if (activeDetailTab === 'record') {
      const attemptCounts = normalizeAttemptCounts(detail);
      const providerAttempts = detail.provider_attempts ?? [];
      const websocketStatusKey = gatewayWebSocketStatusKey(detail);
      // Only the native/simulated split is user-visible; the item itself looks
      // the same on the wire either way.
      const compactionModeLabel = compactionModeKey(detail.compaction_mode);
      const handshakeAttemptsText = detail.websocket?.handshake_attempts?.map((attempt) =>
        `${attempt.provider_name ?? attempt.provider_id ?? '-'}: ${attempt.status_code ?? '-'}`).join(' → ');
      const requestDisplay = deriveGatewayRequestDisplay(detail);
      const requestDisplayTitle = detail.data_source === 'session' && !requestDisplay.modelApplicable
        ? t('gateway.page.requests.localSession')
        : requestDisplay.titleKey ? t(requestDisplay.titleKey) : requestDisplay.modelText;
      return (
        <div className={styles.detailGrid}>
          <span>{t('gateway.page.requests.fields.traceId')}</span>
          <code>{detail.trace_id}</code>
          <span>{t('gateway.page.requests.fields.time')}</span>
          <strong>{formatDateTime(detail.ended_at)}</strong>
          <span>{t('gateway.page.requests.fields.requestType')}</span>
          <strong>{requestDisplayTitle}</strong>
          <span>{t('gateway.page.requests.fields.requestPath')}</span>
          <code>{requestLineText(detail, t('gateway.page.requests.requestPathUnavailable'))}</code>
          <span>{t('gateway.page.requests.fields.provider')}</span>
          <strong>{providerDisplayName(t, detail.provider_id, detail.provider_name)}</strong>
          {detail.data_source === 'session' && (
            <>
              <span>{t('gateway.page.requests.nativeUsage.provider')}</span>
              <strong>{detail.usage_metadata?.native_provider || t('gateway.page.requests.nativeUsage.unknown')}</strong>
              <span>{t('gateway.page.requests.nativeUsage.callCount')}</span>
              <strong>{formatInteger(detail.usage_metadata?.call_count ?? (detail.usage_metadata?.granularity && detail.usage_metadata.granularity !== 'request' ? null : 1))}</strong>
              <span>{t('gateway.page.requests.nativeUsage.completeness')}</span>
              <strong>{t(detail.usage_metadata?.incomplete ? 'gateway.page.requests.nativeUsage.incomplete' : 'gateway.page.requests.nativeUsage.recorded')}</strong>
              <span>{t('gateway.page.requests.nativeUsage.costSource')}</span>
              <strong>{t(`gateway.page.requests.nativeUsage.cost.${detail.usage_metadata?.cost_source ?? 'model_pricing'}`)}</strong>
              {detail.usage_metadata?.cost_source === 'unavailable' && (
                <>
                  <span />
                  <strong className={styles.detailNote}>
                    {t('gateway.page.requests.nativeUsage.costUnavailableHint')}
                  </strong>
                </>
              )}
              {detail.usage_metadata?.granularity === 'session' && (
                <>
                  <span>{t('gateway.page.requests.nativeUsage.window')}</span>
                  <strong>{[
                    detail.usage_metadata.window_start,
                    detail.usage_metadata.window_end,
                  ].map((time) => time == null ? '-' : formatDateTime(new Date(time * 1000).toISOString())).join(' → ')}</strong>
                  <span>{t('gateway.page.requests.nativeUsage.timeMeaning')}</span>
                  <strong className={styles.detailNote}>{t('gateway.page.requests.nativeUsage.cumulativeHint')}</strong>
                </>
              )}
              {detail.usage_metadata?.reported_total_tokens != null && (
                <>
                  <span>{t('gateway.page.requests.nativeUsage.reportedTotal')}</span>
                  <strong>{formatInteger(detail.usage_metadata.reported_total_tokens)}</strong>
                </>
              )}
            </>
          )}
          <span>{t('gateway.page.requests.fields.model')}</span>
          <strong>{requestDisplay.modelApplicable
            ? formatModelQualifiers(requestDisplay.modelText, [
                detail.reasoning_effort,
                serviceTierLabel(detail.service_tier),
                compactionModeLabel ? t(compactionModeLabel) : null,
              ])
            : detail.data_source === 'session'
              ? t('gateway.page.statistics.modelUnavailable')
              : t('gateway.page.requests.notApplicable')}</strong>
          <span>{t('gateway.page.requests.fields.status')}</span>
          <strong title={detail.data_source === 'session' ? t('gateway.page.requests.localSessionHint') : undefined}>
            {detail.data_source === 'session' ? '-' : websocketStatusKey
              ? t(websocketStatusKey) : detail.status_code ?? '-'}
          </strong>
          {detail.upstream_status_code != null && (
            <>
              <span>{t('gateway.page.requests.fields.upstreamStatus')}</span>
              <strong>{detail.upstream_status_code}</strong>
            </>
          )}
          {detail.transport === 'websocket' && (
            <>
              <span>{t('gateway.page.requests.websocket.transport')}</span>
              <strong>WebSocket{detail.request_kind === 'websocket_warmup' ? ` · ${t('gateway.page.requests.websocket.warmup')}` : ''}</strong>
              {detail.websocket && (
                <>
                  <span>{t('gateway.page.requests.websocket.connection')}</span>
                  <code title={detail.websocket.connection_id}>{detail.websocket.connection_id}</code>
                  <span>{t('gateway.page.requests.websocket.handshake')}</span>
                  <strong>{detail.websocket.handshake_status} / {detail.websocket.upstream_handshake_status ?? '-'}</strong>
                  {(detail.websocket.handshake_attempts?.length ?? 0) > 1 && <>
                    <span>{t('gateway.page.requests.websocket.handshakeAttempts')}</span>
                    <code title={handshakeAttemptsText}>{handshakeAttemptsText}</code>
                  </>}
                  <span>{t('gateway.page.requests.websocket.response')}</span>
                  <code title={detail.websocket.response_id ?? undefined}>{detail.websocket.response_id ?? '-'}</code>
                  <span>{t('gateway.page.requests.websocket.previous')}</span>
                  <code title={detail.websocket.previous_response_id ?? undefined}>{detail.websocket.previous_response_id ?? '-'}</code>
                  <span>{t('gateway.page.requests.websocket.stream')}</span>
                  <code title={detail.websocket.stream_id ?? undefined}>{detail.websocket.stream_id ?? '-'}</code>
                  {detail.websocket.error_status != null && <>
                    <span>{t('gateway.page.requests.websocket.errorStatus')}</span>
                    <strong>{detail.websocket.error_status}</strong>
                  </>}
                  {detail.websocket.fallback_reason && <>
                    <span>{t('gateway.page.requests.websocket.fallback')}</span>
                    <strong className={styles.detailNote}>{detail.websocket.fallback_reason}</strong>
                  </>}
                </>
              )}
            </>
          )}
          <span title={t('gateway.page.requests.durationHint')}>{t('gateway.page.requests.fields.firstToken')}</span>
          <strong>{detail.first_token_ms != null ? formatDuration(detail.first_token_ms) : '-'}</strong>
          <span>{t('gateway.page.requests.fields.duration')}</span>
          <strong title={detail.data_source === 'session' ? t('gateway.page.requests.sessionDurationHint') : undefined}>
            {detail.data_source === 'session'
              ? sessionDurationText(detail.duration_ms)
              : formatDuration(detail.duration_ms)}
          </strong>
          <span>{t('gateway.page.requests.fields.streaming')}</span>
          <strong>{detail.data_source === 'session' ? '-' : detail.is_streaming ? t('common.yes') : t('common.no')}</strong>
          <span>{t('gateway.page.requests.fields.tokens')}</span>
          <strong>{isGatewayRequestUsageApplicable(detail) ? tokenBreakdownText(t, detail) : '-'}</strong>
          <span title={t('gateway.page.requests.cacheHitRateHint')}>{t('gateway.page.statistics.columns.cacheHitRate')}</span>
          <strong>{isGatewayRequestUsageApplicable(detail)
            ? formatCacheHitRate(calculateCacheHitRate(
              detail.input_tokens ?? 0,
              detail.cache_read_tokens ?? 0,
              detail.cache_creation_tokens ?? 0,
            ))
            : '-'}</strong>
          <span title={t('gateway.page.requests.tpsHint')}>{t('gateway.page.requests.tpsLabel')}</span>
          <strong title={detail.data_source === 'session' ? t('gateway.page.requests.sessionTpsHint') : undefined}>
            {formatTps(detail) ?? '-'}
          </strong>
          <span>{t('gateway.page.requests.fields.attempts')}</span>
          <strong>{detail.data_source === 'session' ? '-' : [attemptCounts.current, attemptCounts.total].join(' / ')}</strong>
          {providerAttempts.length > 0 && (
            <>
              <span>{t('gateway.page.requests.fields.attemptTimeline')}</span>
              <div className={styles.attemptTimeline}>
                {providerAttempts.map((attempt, index) => {
                  const itemAttemptCounts = normalizeAttemptCounts(attempt);
                  return (
                    <div
                      key={`${attempt.provider_id ?? 'unknown'}-${index}`}
                      className={styles.attemptTimelineItem}
                    >
                      <strong>{providerDisplayName(t, attempt.provider_id, attempt.provider_name)}</strong>
                      <small>
                        {t('gateway.page.requests.attemptDetail', {
                          index: index + 1,
                          status: attempt.status_code ?? '-',
                          current: itemAttemptCounts.current,
                          total: itemAttemptCounts.total,
                          error: attempt.error_category ?? '-',
                        })}
                      </small>
                    </div>
                  );
                })}
              </div>
            </>
          )}
          <span>{t('gateway.page.requests.fields.upstream')}</span>
          <code>{detail.upstream_url ?? '-'}</code>
          <span>{t('gateway.page.requests.fields.error')}</span>
          <strong>{detail.error_category ?? '-'}</strong>
          {detail.privacy && (detail.privacy.matched_values > 0 || detail.privacy.restored_values > 0 || detail.privacy.failed) && (
            <>
              <span>{t('gateway.privacy.title')}</span>
              <div className={styles.detailStack}>
                <strong>{detail.privacy.failed ? t('gateway.privacy.detail.failed') : t('gateway.privacy.detail.applied', {
                  matched: detail.privacy.matched_values, restored: detail.privacy.restored_values,
                })}</strong>
                {Object.keys(detail.privacy.rules).length > 0 && <span className={styles.detailSubtitle}>{t('gateway.privacy.detail.rules', {
                  rules: Object.entries(detail.privacy.rules).map(([rule, count]) => `${rule} (${count})`).join(', '),
                })}</span>}
                {detail.privacy.log_redacted && <span className={styles.detailSubtitle}>{t('gateway.privacy.detail.logs')}</span>}
              </div>
            </>
          )}
          {(detail.native_tools?.length ?? 0) > 0 && (
            <>
              <span>{t('gateway.page.requests.observations.nativeTools')}</span>
              <div className={styles.detailStack}>
                {detail.native_tools!.map((tool) => (
                  <strong key={tool.kind}>
                    {tool.kind} · {[
                      tool.declared ? t('gateway.page.requests.observations.declared') : null,
                      tool.forwarded ? t('gateway.page.requests.observations.forwarded') : null,
                      tool.used ? t('gateway.page.requests.observations.used') : null,
                    ].filter(Boolean).join(' · ')}
                  </strong>
                ))}
              </div>
            </>
          )}
          {(detail.rewrites?.length ?? 0) > 0 && (
            <>
              <span>{t('gateway.page.requests.observations.rewrites')}</span>
              <div className={styles.detailStack}>
                {detail.rewrites!.map((rewrite, index) => (
                  <strong key={`${rewrite.kind}-${index}`} title={rewrite.detail ?? undefined}>
                    {rewrite.kind}
                    {rewrite.detail ? <span className={styles.detailSubtitle}> · {rewrite.detail}</span> : null}
                  </strong>
                ))}
              </div>
            </>
          )}
        </div>
      );
    }

    if (activeDetailTab === 'body') {
      const showUpstreamBody = shouldShowBodyComparison(detail.upstream_request_body, detail.request_body);
      if (showUpstreamBody) {
        return (
          <div className={styles.detailStack}>
            <span className={styles.detailSubtitle}>{t('gateway.page.requests.receivedBody')}</span>
            <CollapsiblePre content={detail.request_body} fallback={detailEmptyMessage} />
            <span className={styles.detailSubtitle}>{t('gateway.page.requests.upstreamBody')}</span>
            <CollapsiblePre content={detail.upstream_request_body} fallback={detailEmptyMessage} />
          </div>
        );
      }
      return <CollapsiblePre content={detail.request_body} fallback={detailEmptyMessage} />;
    }

    if (activeDetailTab === 'headers') {
      return (
        <div className={styles.detailStack}>
          <span className={styles.detailSubtitle}>{t(detail.transport === 'websocket' ? 'gateway.page.requests.websocket.requestHeaders' : 'gateway.page.requests.requestHeaders')}</span>
          <CollapsiblePre
            content={stringifyDetailValue(detail.request_headers) || null}
            fallback={detailEmptyMessage}
          />
          <span className={styles.detailSubtitle}>{t(detail.transport === 'websocket' ? 'gateway.page.requests.websocket.responseHeaders' : 'gateway.page.requests.responseHeaders')}</span>
          <CollapsiblePre
            content={stringifyDetailValue(detail.response_headers) || null}
            fallback={detailEmptyMessage}
          />
        </div>
      );
    }

    const showUpstreamResponseBody = shouldShowBodyComparison(detail.upstream_response_body, detail.response_body);
    if (showUpstreamResponseBody) {
      return (
        <div className={styles.detailStack}>
          <span className={styles.detailSubtitle}>{t('gateway.page.requests.upstreamResponseBody')}</span>
          <CollapsiblePre content={detail.upstream_response_body} fallback={detailEmptyMessage} />
          <span className={styles.detailSubtitle}>{t('gateway.page.requests.clientResponseBody')}</span>
          <CollapsiblePre content={detail.response_body} fallback={detailEmptyMessage} />
        </div>
      );
    }

    return <CollapsiblePre content={detail.response_body} fallback={detailEmptyMessage} />;
  };

  const columns: ColumnsType<GatewayRequestLogItem> = [
    {
      title: t('gateway.page.requests.columns.time'),
      dataIndex: 'created_at',
      width: 170,
      render: (value: string) => formatDateTime(value),
    },
    {
      title: t('gateway.page.requests.columns.provider'),
      dataIndex: 'provider_name',
      render: (_, record) => {
        // `WS` and `回退 HTTP` are badges in the request column now. Keeping the
        // old `WS ·` prefix here made fallback rows read "WS · … 回退 HTTP",
        // claiming a transport those requests never obtained.
        return (
          <div className={styles.tableMainCell}>
            <strong title={providerDisplayName(t, record.provider_id, record.provider_name, record.usage_metadata?.native_provider)}>{providerDisplayName(t, record.provider_id, record.provider_name, record.usage_metadata?.native_provider)}</strong>
            <small>{record.request_kind === 'websocket_warmup' ? `${t('gateway.page.requests.websocket.warmup')} · ` : ''}{providerDisplayMeta(t, record.cli_key, record.provider_id)}</small>
          </div>
        );
      },
    },
    {
      title: t('gateway.page.requests.columns.request'),
      dataIndex: 'requested_model',
      render: (_, record) => (
        <div className={styles.tableMainCell}>
          {(() => {
            const requestDisplay = deriveGatewayRequestDisplay(record);
            const requestDisplayTitle = record.data_source === 'session' && !requestDisplay.modelApplicable
              ? t('gateway.page.requests.localSession')
              : requestDisplay.titleKey ? t(requestDisplay.titleKey) : requestDisplay.modelText;
            const tierLabel = serviceTierLabel(record.service_tier);
            const compactionLabelKey = compactionModeKey(record.compaction_mode);
            const transportBadgeKey = gatewayTransportBadgeKey(record);
            const toolBadges = nativeToolBadges(record.native_tools_used);
            // The badges share one tooltip: the full list in the user's own
            // words, not the wire-level kind ids.
            const toolBadgeTitle = toolBadges.labels.length > 0
              ? t('gateway.page.requests.nativeTools.hint', {
                  tools: toolBadges.labels.map((labelKey) => t(labelKey)).join(', '),
                })
              : undefined;
            return (
              <>
                <div className={styles.modelTitle} title={formatModelQualifiers(requestDisplayTitle, [record.reasoning_effort, tierLabel])}>
                  <strong>{requestDisplayTitle}</strong>
                  {requestDisplay.modelApplicable && record.reasoning_effort ? (
                    <span className={styles.modelEffort} title={t('gateway.page.requests.effortHint')}>
                      ({record.reasoning_effort})
                    </span>
                  ) : null}
                  {requestDisplay.modelApplicable && tierLabel ? (
                    <span className={styles.modelBadge} title={t('gateway.page.requests.serviceTierHint')}>
                      {tierLabel}
                    </span>
                  ) : null}
                  {compactionLabelKey ? (
                    <span className={styles.modelBadge} title={t('gateway.page.requests.compaction.hint')}>
                      {t(compactionLabelKey)}
                    </span>
                  ) : null}
                  {transportBadgeKey ? (
                    <span className={styles.modelBadge} title={t('gateway.page.requests.transportHint')}>
                      {t(transportBadgeKey)}
                    </span>
                  ) : null}
                  {toolBadges.keys.map((badgeKey) => (
                    <span key={badgeKey} className={styles.modelBadge} title={toolBadgeTitle}>
                      {t(badgeKey)}
                    </span>
                  ))}
                  {toolBadges.hiddenCount > 0 ? (
                    <span className={styles.modelBadge} title={toolBadgeTitle}>
                      {t('gateway.page.requests.nativeTools.more', { count: toolBadges.hiddenCount })}
                    </span>
                  ) : null}
                </div>
                <small title={record.usage_metadata?.incomplete ? t('gateway.page.requests.nativeUsage.incompleteHint') : undefined}>
                  {requestDisplay.kind === 'model' || record.data_source === 'session'
                    ? t('gateway.page.requests.tokensShort', {
                        input: formatCompactInteger(record.input_tokens),
                        output: formatCompactInteger(record.output_tokens),
                        cache: formatCompactInteger(record.cache_read_tokens + record.cache_creation_tokens),
                      })
                    : requestLineText(record, t('gateway.page.requests.requestPathUnavailable'))}
                  {record.usage_metadata?.incomplete ? ` · ${t('gateway.page.requests.nativeUsage.incomplete')}` : ''}
                  {record.extra_tokens ? ` · ${t('gateway.page.requests.nativeUsage.extraShort', { value: formatCompactInteger(record.extra_tokens) })}` : ''}
                </small>
              </>
            );
          })()}
        </div>
      ),
    },
    {
      title: t('gateway.page.requests.columns.tokens'),
      dataIndex: 'total_tokens',
      width: 120,
      align: 'right',
      render: (value: number, record) => {
        if (!isGatewayRequestUsageApplicable(record)) {
          return '-';
        }
        const hitRate = calculateCacheHitRate(
          record.input_tokens ?? 0,
          record.cache_read_tokens ?? 0,
          record.cache_creation_tokens ?? 0,
        );
        return (
          <div className={styles.tokenCell}>
            <span>{t('gateway.page.requests.tokenTotalCell', { value: formatInteger(value) })}</span>
            <small>{hitRate == null
              ? '-'
              : t('gateway.page.requests.cacheHitCell', { rate: formatCacheHitRate(hitRate) })}</small>
          </div>
        );
      },
    },
    {
      title: <span title={t('gateway.page.requests.durationHint')}>{t('gateway.page.requests.columns.duration')}</span>,
      dataIndex: 'duration_ms',
      width: 140,
      align: 'right',
      render: (value: number, record) => {
        if (record.data_source === 'session') {
          // Local records have no HTTP first-token mark; the single line is
          // the duration the parser estimated from the transcript's events.
          return sessionDurationText(value);
        }
        return (
          <div className={styles.tokenCell}>
            <small>{t('gateway.page.requests.durationFirstCell', {
              value: record.first_token_ms != null ? formatDuration(record.first_token_ms) : '-',
            })}</small>
            <span>{t('gateway.page.requests.durationTotalCell', { value: formatDuration(value) })}</span>
          </div>
        );
      },
    },
    {
      title: <span title={t('gateway.page.requests.tpsHint')}>{t('gateway.page.requests.columns.tps')}</span>,
      dataIndex: 'tps',
      width: 90,
      align: 'right',
      render: (_, record) => {
        const value = formatTps(record) ?? '-';
        if (record.data_source !== 'session') {
          return value;
        }
        // A session rate divides by the transcript-estimated end-to-end
        // duration, which includes queuing and the wait for the first chunk,
        // so it is not comparable with the gateway's own measurement.
        return <span title={t('gateway.page.requests.sessionTpsHint')}>{value}</span>;
      },
    },
    {
      title: t('gateway.page.requests.columns.cost'),
      dataIndex: 'total_cost_usd',
      width: 110,
      align: 'right',
      render: (value: string, record) =>
        isGatewayRequestUsageApplicable(record) ? formatUsd(value, 6) : '-',
    },
    {
      title: t('gateway.page.requests.columns.status'),
      dataIndex: 'status_code',
      width: 90,
      align: 'right',
      ellipsis: true,
      render: (value: number, record) => {
        if (record.data_source === 'session') {
          return <span title={t('gateway.page.requests.localSessionHint')}>-</span>;
        }
        const statusKey = gatewayWebSocketStatusKey(record);
        const statusText = statusKey ? t(statusKey) : String(value);
        return (
          <span title={statusText} className={record.request_kind === 'websocket_handshake' && value === 426
            ? undefined : record.success ? styles.statusCodeSuccess : styles.statusCodeError}>
            {statusText}
          </span>
        );
      },
    },
  ];

  // The import loop is the only long phase; the closing reconcilers carry no
  // per-unit counter, so they keep the bar at its last value instead of
  // jumping back to zero.
  const rebuildPercent = rebuildProgress && rebuildProgress.total_files > 0
    ? Math.min(100, Math.round((rebuildProgress.processed_files / rebuildProgress.total_files) * 100))
    : 0;

  return (
    <div className={styles.viewStack} aria-busy={loading}>
      {error ? (
        <div className={styles.inlineAlert} role="alert">
          <AlertCircle size={14} aria-hidden="true" />
          <span>{error}</span>
        </div>
      ) : null}
      {notice ? (
        <div className={joinClassNames(styles.inlineNotice, noticeKind === 'warning' && styles.inlineWarning)} role="status" aria-live="polite">
          {noticeKind === 'warning' ? <AlertCircle size={14} aria-hidden="true" /> : <Check size={14} aria-hidden="true" />}
          <span>{notice}</span>
        </div>
      ) : null}
      {rebuildProgress ? (
        <div className={styles.rebuildProgress} role="status" aria-live="polite">
          <Loader2 className={styles.spin} size={14} aria-hidden="true" />
          <div className={styles.rebuildProgressBody}>
            <span>{t(`gateway.page.requests.rebuildPhase.${rebuildProgress.phase}`)}</span>
            {rebuildProgress.total_files > 0 ? (
              <span className={styles.rebuildProgressCounts}>
                {t('gateway.page.requests.rebuildFilesProgress', {
                  processed: formatInteger(rebuildProgress.processed_files),
                  total: formatInteger(rebuildProgress.total_files),
                })}
                {' · '}
                {t('gateway.page.requests.rebuildRecordProgress', {
                  inserted: formatInteger(rebuildProgress.inserted_records),
                  updated: formatInteger(rebuildProgress.updated_records),
                })}
              </span>
            ) : null}
            <div
              className={styles.rebuildProgressBar}
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={rebuildPercent}
            >
              <span style={{ width: `${rebuildPercent}%` }} />
            </div>
          </div>
        </div>
      ) : null}

      <div className={styles.filterBar}>
        <div className={styles.filterSection}>
          <Terminal className={styles.filterIcon} size={14} aria-hidden="true" />
          <Select
            aria-label={t('gateway.page.requests.filters.allCli')}
            variant="borderless"
            size="small"
            value={draft.cliKey}
            className={styles.cliSelect}
            popupMatchSelectWidth={false}
            options={[
              { value: 'all', label: t('gateway.page.requests.filters.allCli') },
              ...GATEWAY_USAGE_TOOLS.map((tool) => ({ value: tool, label: t(`settings.gateway.cli.${tool}`) })),
            ]}
            onChange={(value) => applyDraft({ cliKey: value })}
          />
        </div>
        <div className={styles.filterDivider} />

        <div className={styles.filterSection}>
          <CalendarDays className={styles.filterIcon} size={14} aria-hidden="true" />
          <Select<GatewayRequestRangeSelection['preset']>
            aria-label={t('gateway.page.requests.filters.dateRange')}
            variant="borderless"
            size="small"
            className={styles.rangeSelect}
            popupMatchSelectWidth={false}
            value={draft.range.preset}
            options={[
              { value: 'all', label: t('gateway.page.requests.filters.allTime') },
              ...GATEWAY_USAGE_RANGE_PRESETS.map((preset) => ({
                value: preset,
                label: t(`gateway.page.statistics.range.${preset}`),
              })),
            ]}
            onChange={(preset) => applyDraft({
              range: {
                preset,
                customRange: preset === 'custom' ? draft.range.customRange : undefined,
              },
            })}
          />
        </div>
        {draft.range.preset === 'custom' ? (
          <RangePicker
            showTime
            variant="borderless"
            size="small"
            className={styles.customRangePicker}
            value={draft.range.customRange as never}
            onChange={(dates) => applyDraft({ range: { preset: 'custom', customRange: dates as never } })}
          />
        ) : null}
        <div className={styles.filterDivider} />

        <div className={styles.filterSection}>
          <Select
            aria-label={t('gateway.page.requests.nativeUsage.source')}
            variant="borderless"
            size="small"
            value={draft.dataSource}
            className={styles.statusSelect}
            popupMatchSelectWidth={false}
            options={[
              { value: 'all', label: t('gateway.page.requests.nativeUsage.allSources') },
              { value: 'proxy', label: t('gateway.page.requests.nativeUsage.proxy') },
              { value: 'session', label: t('gateway.page.requests.localSession') },
            ]}
            onChange={(value) => applyDraft({ dataSource: value })}
          />
        </div>
        <div className={styles.filterDivider} />

        <div className={styles.filterSection}>
          <Select
            aria-label={t('gateway.page.requests.filters.allStatus')}
            variant="borderless"
            size="small"
            value={draft.statusCode}
            className={styles.statusSelect}
            popupMatchSelectWidth={false}
            options={[
              { value: 'all', label: t('gateway.page.requests.filters.allStatus') },
              { value: '200', label: '200' },
              { value: '400', label: '400' },
              { value: '401', label: '401' },
              { value: '429', label: '429' },
              { value: '500', label: '500' },
            ]}
            onChange={(value) => applyDraft({ statusCode: value })}
          />
        </div>
        <div className={styles.filterDivider} />

        <Input
          size="small"
          allowClear
          variant="borderless"
          className={styles.searchInput}
          placeholder={t('gateway.page.requests.filters.providerPlaceholder')}
          value={draft.providerName}
          onChange={(event) => setDraft((current) => ({ ...current, providerName: event.target.value }))}
          onPressEnter={applyFilters}
        />
        <div className={styles.filterDivider} />
        <Input
          size="small"
          allowClear
          variant="borderless"
          className={styles.searchInput}
          placeholder={t('gateway.page.requests.filters.modelPlaceholder')}
          value={draft.model}
          onChange={(event) => setDraft((current) => ({ ...current, model: event.target.value }))}
          onPressEnter={applyFilters}
        />
        <div className={styles.filterDivider} />

        <div className={styles.filterActions}>
          <button
            type="button"
            className={styles.iconButton}
            onClick={applyFilters}
            aria-label={t('common.search')}
            title={t('common.search')}
          >
            <Search size={14} aria-hidden="true" />
          </button>
          <button
            type="button"
            className={styles.iconButton}
            onClick={resetFilters}
            aria-label={t('common.reset')}
            title={t('common.reset')}
          >
            <X size={14} aria-hidden="true" />
          </button>
          <button
            type="button"
            className={styles.iconButton}
            disabled={loading || syncingForRefresh || rebuilding}
            onClick={() => void handleRefreshWithSessionSync()}
            aria-label={t('gateway.page.requests.refreshWithSync')}
            title={t('gateway.page.requests.refreshWithSyncHint')}
          >
            <RefreshCw
              size={14}
              className={loading || syncingForRefresh ? styles.spin : undefined}
              aria-hidden="true"
            />
          </button>
          <Dropdown
            trigger={['click']}
            placement="bottomRight"
            disabled={syncingForRefresh || rebuilding}
            menu={{
              items: sessionUsageMenuItems,
              onClick: handleSessionUsageMenuClick,
            }}
          >
            <button
              type="button"
              className={styles.iconButton}
              disabled={syncingForRefresh || rebuilding}
              aria-label={t('gateway.page.requests.sessionUsageActions')}
              title={t('gateway.page.requests.sessionUsageActions')}
            >
              {rebuilding ? (
                <Loader2 size={14} className={styles.spin} aria-hidden="true" />
              ) : (
                <Ellipsis size={14} aria-hidden="true" />
              )}
            </button>
          </Dropdown>
        </div>
      </div>

      <section className={styles.dataPanel}>
        <div className={styles.panelHeader}>
          <span>
            <Network className={styles.panelIcon} size={14} aria-hidden="true" />
            {t('gateway.page.requests.records')}
          </span>
          <div className={styles.panelHeaderActions}>
            <Checkbox
              checked={Boolean(filters.exclude_model_list)}
              onChange={(event) => handleExcludeModelListChange(event.target.checked)}
            >
              {t('gateway.page.requests.filters.excludeModelList')}
            </Checkbox>
            <span className={styles.panelHeaderDivider} aria-hidden="true" />
            <Checkbox
              checked={Boolean(filters.only_failed)}
              onChange={(event) => handleOnlyFailedChange(event.target.checked)}
            >
              {t('gateway.page.requests.filters.onlyFailed')}
            </Checkbox>
            <span className={styles.panelHeaderDivider} aria-hidden="true" />
            <span className={styles.panelCount}>
              {t('gateway.page.requests.totalCount', { total: formatInteger(total) })}
            </span>
          </div>
        </div>
        <Table
          rowKey="trace_id"
          size="small"
          tableLayout="fixed"
          columns={columns}
          dataSource={logs}
          loading={loading}
          pagination={false}
          scroll={{ x: 960 }}
          locale={{
            emptyText: <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t('gateway.page.requests.empty')} />,
          }}
          onRow={(record) => ({
            onClick: () => void loadDetail(record.trace_id),
          })}
        />
        <div className={styles.paginationBar}>
          <Pagination
            size="small"
            current={page}
            pageSize={PAGE_SIZE}
            total={total}
            showSizeChanger={false}
            onChange={setPage}
          />
        </div>
      </section>

      {selectedTraceId ? (
        <div className={styles.detailModalBackdrop} role="presentation" onMouseDown={closeDetail}>
          <div
            className={styles.detailModal}
            role="dialog"
            aria-modal="true"
            aria-label={t('gateway.page.requests.detail')}
            onMouseDown={(event) => event.stopPropagation()}
          >
            <div className={styles.detailModalHeader}>
              <span>
                <FileText size={15} aria-hidden="true" />
                {t('gateway.page.requests.detail')}
              </span>
              <div className={styles.detailHeaderActions}>
                <button
                  type="button"
                  className={styles.iconButton}
                  disabled={!detail || detailLoading || exportingDetail}
                  aria-label={t('gateway.page.requests.exportDetail')}
                  title={t('gateway.page.requests.exportDetail')}
                  onClick={() => void handleExportDetail()}
                >
                  {exportingDetail ? (
                    <Loader2 size={15} className={styles.spin} aria-hidden="true" />
                  ) : (
                    <Download size={15} aria-hidden="true" />
                  )}
                </button>
                <button
                  type="button"
                  className={styles.iconButton}
                  aria-label={t('common.close')}
                  title={t('common.close')}
                  onClick={closeDetail}
                >
                  <X size={15} aria-hidden="true" />
                </button>
              </div>
            </div>
            <div className={styles.detailTabList}>
              {REQUEST_DETAIL_TABS.map((tab) => (
                <button
                  key={tab}
                  type="button"
                  className={joinClassNames(
                    styles.detailTabButton,
                    activeDetailTab === tab && styles.detailTabButtonActive,
                  )}
                  onClick={() => setActiveDetailTab(tab)}
                >
                  {t(`gateway.page.requests.detailTabs.${tab}`)}
                </button>
              ))}
            </div>
            <div className={styles.detailModalBody}>{renderDetailContent()}</div>
          </div>
        </div>
      ) : null}
    </div>
  );
};

export default GatewayRequestsView;
