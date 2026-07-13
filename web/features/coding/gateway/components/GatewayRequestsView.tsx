import React from 'react';
import { DatePicker, Empty, Input, Pagination, Select, Table } from 'antd';
import type { ColumnsType } from 'antd/es/table';
import { save } from '@tauri-apps/plugin-dialog';
import {
  AlertCircle,
  CalendarDays,
  Check,
  ChevronDown,
  ChevronUp,
  Copy,
  Database,
  Download,
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
  importProxyGatewaySessionUsage,
  listProxyGatewayRequestLogs,
  type GatewayCliKey,
  type GatewayRequestLogDetail,
  type GatewayRequestLogFilters,
  type GatewayRequestLogItem,
} from '@/services';
import {
  deriveGatewayRequestDisplay,
  formatCompactInteger,
  formatDateTime,
  formatDuration,
  formatGatewayError,
  formatInteger,
  formatUsd,
  isGatewayRequestUsageApplicable,
  joinClassNames,
  normalizeAttemptCounts,
  requestExportPrefix,
  requestLineText,
  sanitizeGatewayFileNamePart,
  shouldShowBodyComparison,
  stringifyDetailValue,
} from '../utils/gatewayFormatters';
import styles from './GatewayRequestsView.module.less';

const { RangePicker } = DatePicker;

type RequestDetailTabKey = 'record' | 'body' | 'headers' | 'response';
type GatewayCliFilter = 'all' | GatewayCliKey;

const REQUEST_DETAIL_TABS: RequestDetailTabKey[] = ['record', 'body', 'headers', 'response'];
const COLLAPSED_LINE_LIMIT = 10;
const COLLAPSED_CHARACTER_LIMIT = 8_000;
const PAGE_SIZE = 20;
const EXPORT_NOTICE_DURATION_MS = 3000;

interface GatewayRequestsViewProps {
  refreshKey?: number;
}

interface DateLike {
  toDate: () => Date;
}

interface RequestFilterDraft {
  cliKey: GatewayCliFilter;
  statusCode: string;
  providerName: string;
  model: string;
  dateRange: [DateLike | null, DateLike | null] | null;
}

const defaultDraft: RequestFilterDraft = {
  cliKey: 'all',
  statusCode: 'all',
  providerName: '',
  model: '',
  dateRange: null,
};

const lineCountOf = (content: string) => content.split(/\r\n|\r|\n/).length;

const tokenBreakdownText = (
  t: ReturnType<typeof useTranslation>['t'],
  value: Pick<
    GatewayRequestLogItem | GatewayRequestLogDetail,
    'input_tokens' | 'output_tokens' | 'cache_read_tokens' | 'cache_creation_tokens' | 'total_tokens'
  >,
) => t('gateway.page.requests.tokensValue', {
  input: formatInteger(value.input_tokens),
  output: formatInteger(value.output_tokens),
  cacheRead: formatInteger(value.cache_read_tokens),
  cacheCreation: formatInteger(value.cache_creation_tokens),
  total: formatInteger(value.total_tokens),
});

const providerDisplayName = (
  t: ReturnType<typeof useTranslation>['t'],
  providerId?: string | null,
  providerName?: string | null,
) => {
  if (providerName) {
    return providerName;
  }
  if (!providerId || providerId === 'unknown') {
    return t('gateway.page.requests.providerUnselected');
  }
  return providerId;
};

const providerDisplayMeta = (
  t: ReturnType<typeof useTranslation>['t'],
  cliKey: GatewayCliKey,
  providerId?: string | null,
) => {
  const cliLabel = t(`settings.gateway.cli.${cliKey}`);
  if (!providerId || providerId === 'unknown') {
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
  const [start, end] = draft.dateRange ?? [];
  return {
    cli_key: draft.cliKey === 'all' ? null : draft.cliKey,
    status_code: draft.statusCode === 'all' ? null : Number(draft.statusCode),
    provider_name: draft.providerName.trim() || null,
    model: draft.model.trim() || null,
    start_date: start ? Math.floor(start.toDate().getTime() / 1000) : null,
    end_date: end ? Math.floor(end.toDate().getTime() / 1000) : null,
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
  const [filters, setFilters] = React.useState<GatewayRequestLogFilters>({});
  const [page, setPage] = React.useState(1);
  const [logs, setLogs] = React.useState<GatewayRequestLogItem[]>([]);
  const [total, setTotal] = React.useState(0);
  const [selectedTraceId, setSelectedTraceId] = React.useState<string | null>(null);
  const [detail, setDetail] = React.useState<GatewayRequestLogDetail | null>(null);
  const [activeDetailTab, setActiveDetailTab] = React.useState<RequestDetailTabKey>('record');
  const [loading, setLoading] = React.useState(false);
  const [detailLoading, setDetailLoading] = React.useState(false);
  const [importing, setImporting] = React.useState(false);
  const [exportingDetail, setExportingDetail] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [notice, setNotice] = React.useState<string | null>(null);
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
    selectedTraceIdRef.current = null;
    setSelectedTraceId(null);
    setDetail(null);
  }, []);

  const loadRequests = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await listProxyGatewayRequestLogs(filters, Math.max(page - 1, 0), PAGE_SIZE);
      setLogs(result.data);
      setTotal(result.total);
      if (!result.data.some((log) => log.trace_id === selectedTraceIdRef.current)) {
        closeDetail();
      }
    } catch (loadError) {
      setError(t('gateway.page.requests.loadFailed', { error: formatGatewayError(loadError) }));
    } finally {
      setLoading(false);
    }
  }, [closeDetail, filters, page, t]);

  const loadDetail = React.useCallback(
    async (traceId: string) => {
      selectedTraceIdRef.current = traceId;
      setSelectedTraceId(traceId);
      setDetail(null);
      setDetailLoading(true);
      setError(null);
      try {
        const nextDetail = await getProxyGatewayRequestLogDetail(traceId);
        setDetail(nextDetail);
        setActiveDetailTab('record');
      } catch (detailError) {
        setError(t('gateway.page.requests.loadFailed', { error: formatGatewayError(detailError) }));
      } finally {
        setDetailLoading(false);
      }
    },
    [t],
  );

  React.useEffect(() => {
    void loadRequests();
  }, [loadRequests, refreshKey]);

  const applyFilters = () => {
    setFilters(buildFilters(draft));
    setPage(1);
  };

  const resetFilters = () => {
    setDraft(defaultDraft);
    setFilters({});
    setPage(1);
  };

  const handleImportSessionUsage = async () => {
    setImporting(true);
    setError(null);
    setNotice(null);
    try {
      const result = await importProxyGatewaySessionUsage({ cli_key: 'all' });
      setNotice(t('gateway.page.requests.importDone', {
        inserted: formatInteger(result.inserted_records),
        skipped: formatInteger(result.skipped_records),
        files: formatInteger(result.scanned_files),
      }));
      setPage(1);
      await loadRequests();
    } catch (importError) {
      setError(t('gateway.page.requests.importFailed', { error: formatGatewayError(importError) }));
    } finally {
      setImporting(false);
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

    if (activeDetailTab === 'record') {
      const attemptCounts = normalizeAttemptCounts(detail);
      const providerAttempts = detail.provider_attempts ?? [];
      const requestDisplay = deriveGatewayRequestDisplay(detail);
      const requestDisplayTitle = requestDisplay.titleKey ? t(requestDisplay.titleKey) : requestDisplay.modelText;
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
          <span>{t('gateway.page.requests.fields.model')}</span>
          <strong>{requestDisplay.modelApplicable ? requestDisplay.modelText : t('gateway.page.requests.notApplicable')}</strong>
          <span>{t('gateway.page.requests.fields.status')}</span>
          <strong>{detail.status_code ?? '-'}</strong>
          <span>{t('gateway.page.requests.fields.duration')}</span>
          <strong>{formatDuration(detail.duration_ms)}</strong>
          <span>{t('gateway.page.requests.fields.firstToken')}</span>
          <strong>{detail.first_token_ms != null ? formatDuration(detail.first_token_ms) : '-'}</strong>
          <span>{t('gateway.page.requests.fields.streaming')}</span>
          <strong>{detail.is_streaming ? t('common.yes') : t('common.no')}</strong>
          <span>{t('gateway.page.requests.fields.tokens')}</span>
          <strong>{isGatewayRequestUsageApplicable(detail) ? tokenBreakdownText(t, detail) : '-'}</strong>
          <span>{t('gateway.page.requests.fields.attempts')}</span>
          <strong>{attemptCounts.current} / {attemptCounts.total}</strong>
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
        </div>
      );
    }

    if (activeDetailTab === 'body') {
      const showUpstreamBody = shouldShowBodyComparison(detail.upstream_request_body, detail.request_body);
      if (showUpstreamBody) {
        return (
          <div className={styles.detailStack}>
            <span className={styles.detailSubtitle}>{t('gateway.page.requests.receivedBody')}</span>
            <CollapsiblePre content={detail.request_body} fallback={t('gateway.page.requests.notStored')} />
            <span className={styles.detailSubtitle}>{t('gateway.page.requests.upstreamBody')}</span>
            <CollapsiblePre content={detail.upstream_request_body} fallback={t('gateway.page.requests.notStored')} />
          </div>
        );
      }
      return <CollapsiblePre content={detail.request_body} fallback={t('gateway.page.requests.notStored')} />;
    }

    if (activeDetailTab === 'headers') {
      return (
        <div className={styles.detailStack}>
          <span className={styles.detailSubtitle}>{t('gateway.page.requests.requestHeaders')}</span>
          <CollapsiblePre
            content={stringifyDetailValue(detail.request_headers) || null}
            fallback={t('gateway.page.requests.notStored')}
          />
          <span className={styles.detailSubtitle}>{t('gateway.page.requests.responseHeaders')}</span>
          <CollapsiblePre
            content={stringifyDetailValue(detail.response_headers) || null}
            fallback={t('gateway.page.requests.notStored')}
          />
        </div>
      );
    }

    const showUpstreamResponseBody = shouldShowBodyComparison(detail.upstream_response_body, detail.response_body);
    if (showUpstreamResponseBody) {
      return (
        <div className={styles.detailStack}>
          <span className={styles.detailSubtitle}>{t('gateway.page.requests.upstreamResponseBody')}</span>
          <CollapsiblePre content={detail.upstream_response_body} fallback={t('gateway.page.requests.notStored')} />
          <span className={styles.detailSubtitle}>{t('gateway.page.requests.clientResponseBody')}</span>
          <CollapsiblePre content={detail.response_body} fallback={t('gateway.page.requests.notStored')} />
        </div>
      );
    }

    return <CollapsiblePre content={detail.response_body} fallback={t('gateway.page.requests.notStored')} />;
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
      render: (_, record) => (
        <div className={styles.tableMainCell}>
          <strong>{providerDisplayName(t, record.provider_id, record.provider_name)}</strong>
          <small>{providerDisplayMeta(t, record.cli_key, record.provider_id)}</small>
        </div>
      ),
    },
    {
      title: t('gateway.page.requests.columns.request'),
      dataIndex: 'requested_model',
      render: (_, record) => (
        <div className={styles.tableMainCell}>
          {(() => {
            const requestDisplay = deriveGatewayRequestDisplay(record);
            const requestDisplayTitle = requestDisplay.titleKey ? t(requestDisplay.titleKey) : requestDisplay.modelText;
            return (
              <>
                <strong>{requestDisplayTitle}</strong>
                <small>
                  {requestDisplay.kind === 'model'
                    ? t('gateway.page.requests.tokensShort', {
                        input: formatCompactInteger(record.input_tokens),
                        output: formatCompactInteger(record.output_tokens),
                        cache: formatCompactInteger(record.cache_read_tokens + record.cache_creation_tokens),
                      })
                    : requestLineText(record, t('gateway.page.requests.requestPathUnavailable'))}
                </small>
              </>
            );
          })()}
        </div>
      ),
    },
    {
      title: t('gateway.page.requests.columns.status'),
      dataIndex: 'status_code',
      width: 90,
      align: 'right',
      render: (value: number) => (
        <span className={value >= 200 && value < 400 ? styles.statusCodeSuccess : styles.statusCodeError}>
          {value}
        </span>
      ),
    },
    {
      title: t('gateway.page.requests.columns.tokens'),
      dataIndex: 'total_tokens',
      width: 110,
      align: 'right',
      render: (value: number, record) =>
        isGatewayRequestUsageApplicable(record) ? formatCompactInteger(value) : '-',
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
      title: t('gateway.page.requests.columns.duration'),
      dataIndex: 'duration_ms',
      width: 100,
      align: 'right',
      render: (value: number) => formatDuration(value),
    },
  ];

  return (
    <div className={styles.viewStack} aria-busy={loading}>
      {error ? (
        <div className={styles.inlineAlert} role="alert">
          <AlertCircle size={14} aria-hidden="true" />
          <span>{error}</span>
        </div>
      ) : null}
      {notice ? (
        <div className={styles.inlineNotice} role="status" aria-live="polite">
          <Check size={14} aria-hidden="true" />
          <span>{notice}</span>
        </div>
      ) : null}

      <div className={styles.filterBar}>
        <div className={styles.filterSection}>
          <Terminal className={styles.filterIcon} size={14} aria-hidden="true" />
          <Select
            variant="borderless"
            size="small"
            value={draft.cliKey}
            className={styles.cliSelect}
            popupMatchSelectWidth={false}
            options={[
              { value: 'all', label: t('gateway.page.requests.filters.allCli') },
              { value: 'claude', label: t('settings.gateway.cli.claude') },
              { value: 'codex', label: t('settings.gateway.cli.codex') },
              { value: 'grok', label: t('settings.gateway.cli.grok') },
              { value: 'gemini', label: t('settings.gateway.cli.gemini') },
            ]}
            onChange={(value) => setDraft((current) => ({ ...current, cliKey: value }))}
          />
        </div>
        <div className={styles.filterDivider} />

        <div className={styles.filterSection}>
          <Select
            variant="borderless"
            size="small"
            value={draft.statusCode}
            className={styles.statusSelect}
            popupMatchSelectWidth={false}
            options={[
              { value: 'all', label: t('common.all') },
              { value: '200', label: '200' },
              { value: '400', label: '400' },
              { value: '401', label: '401' },
              { value: '429', label: '429' },
              { value: '500', label: '500' },
            ]}
            onChange={(value) => setDraft((current) => ({ ...current, statusCode: value }))}
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

        <div className={styles.filterSectionShrink}>
          <CalendarDays className={styles.filterIcon} size={14} aria-hidden="true" />
          <RangePicker
            showTime
            variant="borderless"
            size="small"
            className={styles.dateRange}
            value={draft.dateRange as never}
            onChange={(dates) => setDraft((current) => ({ ...current, dateRange: dates as never }))}
          />
        </div>
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
            className={styles.importButton}
            disabled={importing}
            onClick={() => void handleImportSessionUsage()}
          >
            {importing ? <Loader2 size={13} className={styles.spin} aria-hidden="true" /> : <Database size={13} aria-hidden="true" />}
            <span>{t('gateway.page.requests.importSessions')}</span>
          </button>
          <button
            type="button"
            className={styles.iconButton}
            disabled={loading}
            onClick={() => void loadRequests()}
            aria-label={t('common.refresh')}
            title={t('common.refresh')}
          >
            <RefreshCw size={14} className={loading ? styles.spin : undefined} aria-hidden="true" />
          </button>
        </div>
      </div>

      <section className={styles.dataPanel}>
        <div className={styles.panelHeader}>
          <span>
            <Network size={14} aria-hidden="true" />
            {t('gateway.page.requests.records')}
          </span>
          <span className={styles.panelCount}>{formatInteger(total)}</span>
        </div>
        <Table
          rowKey="trace_id"
          size="small"
          columns={columns}
          dataSource={logs}
          loading={loading}
          pagination={false}
          scroll={{ x: 900 }}
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
