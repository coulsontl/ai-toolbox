import React from 'react';
import {
  AlertCircle,
  Check,
  ChevronDown,
  ChevronUp,
  Copy,
  FileText,
  Network,
  RefreshCw,
  X,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  getProxyGatewayRequestLogDetail,
  listProxyGatewayRequestLogs,
  type GatewayRequestLogDetail,
  type GatewayRequestLogSummary,
} from '@/services';
import {
  formatDateTime,
  formatDuration,
  formatGatewayError,
  formatInteger,
  joinClassNames,
  normalizeAttemptCounts,
  stringifyDetailValue,
} from '../utils/gatewayFormatters';
import styles from './GatewayRequestsView.module.less';

type RequestDetailTabKey = 'record' | 'body' | 'headers' | 'response';

const REQUEST_DETAIL_TABS: RequestDetailTabKey[] = ['record', 'body', 'headers', 'response'];
const COLLAPSED_LINE_LIMIT = 10;
const COLLAPSED_CHARACTER_LIMIT = 8_000;

const lineCountOf = (content: string) => content.split(/\r\n|\r|\n/).length;

const formatModelRoute = (
  requestedModel: string | null,
  upstreamModelId: string | null,
  fallback: string,
) => {
  const displayModel = requestedModel?.trim() || fallback;
  if (requestedModel && upstreamModelId && upstreamModelId !== requestedModel) {
    return `${requestedModel} → ${upstreamModelId}`;
  }
  return displayModel;
};

interface GatewayRequestsViewProps {
  refreshKey?: number;
}

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
  const [logs, setLogs] = React.useState<GatewayRequestLogSummary[]>([]);
  const [selectedTraceId, setSelectedTraceId] = React.useState<string | null>(null);
  const [detail, setDetail] = React.useState<GatewayRequestLogDetail | null>(null);
  const [activeDetailTab, setActiveDetailTab] = React.useState<RequestDetailTabKey>('record');
  const [loading, setLoading] = React.useState(false);
  const [detailLoading, setDetailLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const selectedTraceIdRef = React.useRef<string | null>(null);

  const closeDetail = React.useCallback(() => {
    selectedTraceIdRef.current = null;
    setSelectedTraceId(null);
    setDetail(null);
  }, []);

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

  const loadRequests = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const nextLogs = await listProxyGatewayRequestLogs({ limit: 100 });
      setLogs(nextLogs);
      if (!nextLogs.some((log) => log.trace_id === selectedTraceIdRef.current)) {
        closeDetail();
      }
    } catch (loadError) {
      setError(t('gateway.page.requests.loadFailed', { error: formatGatewayError(loadError) }));
    } finally {
      setLoading(false);
    }
  }, [closeDetail, t]);

  React.useEffect(() => {
    void loadRequests();
  }, [loadRequests, refreshKey]);

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
      return (
        <div className={styles.detailGrid}>
          <span>{t('gateway.page.requests.fields.traceId')}</span>
          <code>{detail.trace_id}</code>
          <span>{t('gateway.page.requests.fields.time')}</span>
          <strong>{formatDateTime(detail.ended_at)}</strong>
          <span>{t('gateway.page.requests.fields.provider')}</span>
          <strong>{detail.provider_name ?? detail.provider_id ?? '-'}</strong>
          <span>{t('gateway.page.requests.fields.model')}</span>
          <strong>{formatModelRoute(detail.requested_model, detail.upstream_model_id, '-')}</strong>
          <span>{t('gateway.page.requests.fields.status')}</span>
          <strong>{detail.status_code ?? '-'}</strong>
          <span>{t('gateway.page.requests.fields.duration')}</span>
          <strong>{formatDuration(detail.duration_ms)}</strong>
          <span>{t('gateway.page.requests.fields.tokens')}</span>
          <strong>
            {t('gateway.page.requests.tokensValue', {
              input: formatInteger(detail.input_tokens),
              output: formatInteger(detail.output_tokens),
              total: formatInteger(detail.total_tokens),
            })}
          </strong>
          <span>{t('gateway.page.requests.fields.attempts')}</span>
          <strong>{attemptCounts.current} / {attemptCounts.total}</strong>
          <span>{t('gateway.page.requests.fields.upstream')}</span>
          <code>{detail.upstream_url ?? '-'}</code>
          <span>{t('gateway.page.requests.fields.error')}</span>
          <strong>{detail.error_category ?? '-'}</strong>
        </div>
      );
    }

    if (activeDetailTab === 'body') {
      const showUpstreamBody =
        detail.upstream_request_body != null && detail.upstream_request_body !== detail.request_body;
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
      return (
        <CollapsiblePre content={detail.request_body} fallback={t('gateway.page.requests.notStored')} />
      );
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

    return (
      <CollapsiblePre content={detail.response_body} fallback={t('gateway.page.requests.notStored')} />
    );
  };

  return (
    <div className={styles.viewStack} aria-busy={loading}>
      {error ? (
        <div className={styles.inlineAlert} role="alert">
          <AlertCircle size={14} aria-hidden="true" />
          <span>{error}</span>
        </div>
      ) : null}
      <div className={styles.requestGrid}>
        <section className={styles.dataPanel}>
          <div className={styles.panelHeader}>
            <span>
              <FileText size={14} aria-hidden="true" />
              {t('gateway.page.requests.records')}
            </span>
            <span className={styles.panelCount}>{loading ? t('common.loading') : logs.length}</span>
          </div>
          {logs.length ? (
            <div className={styles.requestList}>
              {logs.map((log) => {
                return (
                  <button
                    key={log.trace_id}
                    type="button"
                    className={joinClassNames(
                      styles.requestRow,
                      selectedTraceId === log.trace_id && styles.requestRowActive,
                    )}
                    onClick={() => void loadDetail(log.trace_id)}
                  >
                    <span className={styles.requestMethod}>{log.method}</span>
                    <span className={styles.requestMain}>
                      <strong>{formatModelRoute(log.requested_model, log.upstream_model_id, log.path)}</strong>
                      <small>
                        {log.cli_key ? t(`settings.gateway.cli.${log.cli_key}`) : '-'} · {log.provider_name ?? log.provider_id ?? '-'} ·{' '}
                        {formatDateTime(log.ended_at)} ·{' '}
                        {t('gateway.page.requests.tokensShort', {
                          input: formatInteger(log.input_tokens),
                          output: formatInteger(log.output_tokens),
                        })}
                      </small>
                    </span>
                    <span className={styles.requestBadges}>
                      <span className={joinClassNames(styles.statusCode, log.success ? styles.statusCodeSuccess : styles.statusCodeError)}>
                        {log.status_code ?? '-'}
                      </span>
                    </span>
                    <span className={styles.requestDuration}>{formatDuration(log.duration_ms)}</span>
                  </button>
                );
              })}
            </div>
          ) : (
            <div className={styles.emptyState}>
              <FileText size={18} aria-hidden="true" />
              <span>{loading ? t('common.loading') : t('gateway.page.requests.empty')}</span>
            </div>
          )}
        </section>
      </div>
      {selectedTraceId ? (
        <div className={styles.detailModalBackdrop} role="presentation" onClick={closeDetail}>
          <section
            className={styles.detailModal}
            role="dialog"
            aria-modal="true"
            aria-labelledby="gateway-request-detail-title"
            onClick={(event) => event.stopPropagation()}
          >
            <div className={styles.detailModalHeader}>
              <span>
                <Network size={14} aria-hidden="true" />
                <strong id="gateway-request-detail-title">{t('gateway.page.requests.detail')}</strong>
              </span>
              <button
                type="button"
                className={styles.iconButton}
                aria-label={t('common.close')}
                title={t('common.close')}
                onClick={closeDetail}
              >
                <X size={16} aria-hidden="true" />
              </button>
            </div>
            <div className={styles.detailTabList}>
              {REQUEST_DETAIL_TABS.map((tabKey) => (
                <button
                  key={tabKey}
                  type="button"
                  className={joinClassNames(styles.detailTabButton, activeDetailTab === tabKey && styles.detailTabButtonActive)}
                  onClick={() => setActiveDetailTab(tabKey)}
                >
                  {t(`gateway.page.requests.detailTabs.${tabKey}`)}
                </button>
              ))}
            </div>
            <div className={styles.detailModalBody}>{renderDetailContent()}</div>
          </section>
        </div>
      ) : null}
    </div>
  );
};

export default GatewayRequestsView;
