import React from 'react';
import {
  Activity,
  AlertCircle,
  BarChart3,
  CheckCircle2,
  Clock3,
  FileText,
  Gauge,
  Network,
  Shield,
  Terminal,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  checkProxyGatewayHealth,
  getProxyGatewayCliStatuses,
  getProxyGatewaySettings,
  getProxyGatewayStatus,
  listProxyGatewayMetricRollups,
  listProxyGatewayModelHealthEntries,
  type GatewayCliTakeoverStatus,
  type GatewayModelHealthItem,
  type MetricRollupItem,
  type ProxyGatewayHealthCheckResult,
  type ProxyGatewaySettings,
  type ProxyGatewayStatus,
} from '@/services';
import {
  averageLatency,
  buildGatewayOrigin,
  deriveRequestLogLevel,
  formatDuration,
  formatGatewayError,
  formatInteger,
  joinClassNames,
  successRateText,
} from '../utils/gatewayFormatters';
import StatTile from './StatTile';
import styles from './GatewayStatisticsView.module.less';

interface GatewaySummaryState {
  settings: ProxyGatewaySettings | null;
  status: ProxyGatewayStatus | null;
  health: ProxyGatewayHealthCheckResult | null;
  cliStatuses: GatewayCliTakeoverStatus[];
  metricRollups: MetricRollupItem[];
  modelHealthItems: GatewayModelHealthItem[];
}

const emptySummaryState: GatewaySummaryState = {
  settings: null,
  status: null,
  health: null,
  cliStatuses: [],
  metricRollups: [],
  modelHealthItems: [],
};

interface GatewayStatisticsViewProps {
  refreshKey?: number;
}

const GatewayStatisticsView: React.FC<GatewayStatisticsViewProps> = ({ refreshKey = 0 }) => {
  const { t } = useTranslation();
  const [state, setState] = React.useState<GatewaySummaryState>(emptySummaryState);
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const loadSummary = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [settings, status, health, cliStatuses] = await Promise.all([
        getProxyGatewaySettings(),
        getProxyGatewayStatus(),
        checkProxyGatewayHealth(),
        getProxyGatewayCliStatuses(),
      ]);
      const [metricRollups, modelHealthItems] = await Promise.all([
        listProxyGatewayMetricRollups(),
        listProxyGatewayModelHealthEntries(),
      ]);
      setState({ settings, status, health, cliStatuses, metricRollups, modelHealthItems });
    } catch (loadError) {
      setError(t('gateway.page.statistics.loadFailed', { error: formatGatewayError(loadError) }));
    } finally {
      setLoading(false);
    }
  }, [t]);

  React.useEffect(() => {
    void loadSummary();
  }, [loadSummary, refreshKey]);

  const statusKind = state.status?.running ? 'running' : state.status?.last_error ? 'error' : 'stopped';
  const activeCliCount = state.cliStatuses.filter((cliStatus) => cliStatus.can_restore_direct).length;
  const logLevel = deriveRequestLogLevel(state.settings);
  const totalRequests = state.metricRollups.reduce((total, item) => total + item.total_requests, 0);
  const successRequests = state.metricRollups.reduce((total, item) => total + item.success_requests, 0);
  const failoverRequests = state.metricRollups.reduce((total, item) => total + item.failover_requests, 0);
  const inputTokens = state.metricRollups.reduce((total, item) => total + item.input_tokens, 0);
  const outputTokens = state.metricRollups.reduce((total, item) => total + item.output_tokens, 0);
  const visibleMetricRollups = state.metricRollups.slice(0, 8);
  const visibleHealthItems = state.modelHealthItems.slice(0, 8);
  const modelHealthProviderName = (item: GatewayModelHealthItem) => item.provider_name ?? item.provider_id;
  const modelHealthModelName = (item: GatewayModelHealthItem) => {
    if (!item.upstream_model_id) {
      return t('gateway.page.statistics.modelHealthProviderScope', {
        provider: modelHealthProviderName(item),
      });
    }
    if (item.upstream_model_id === 'unknown') {
      return t('gateway.page.statistics.modelHealthUnknownModel');
    }
    return item.upstream_model_id;
  };

  return (
    <div className={styles.viewStack} aria-busy={loading}>
      {error ? (
        <div className={styles.inlineAlert} role="alert">
          <AlertCircle size={14} aria-hidden="true" />
          <span>{error}</span>
        </div>
      ) : null}

      <div className={styles.statGrid}>
        <StatTile
          icon={statusKind === 'running' ? <CheckCircle2 size={15} /> : <Network size={15} />}
          label={t('gateway.page.statistics.state')}
          value={t(`settings.gateway.status.${statusKind}`)}
          tone={statusKind === 'running' ? 'success' : statusKind === 'error' ? 'error' : 'muted'}
          meta={buildGatewayOrigin(state.status)}
        />
        <StatTile
          icon={<Activity size={15} />}
          label={t('gateway.page.statistics.health')}
          value={
            state.health
              ? state.health.ok
                ? t('settings.gateway.status.healthOk', { statusCode: state.health.status_code ?? '-' })
                : t('settings.gateway.status.healthFailed')
              : t('settings.gateway.status.healthUnknown')
          }
          tone={state.health?.ok ? 'success' : state.health?.ok === false ? 'error' : 'muted'}
          meta={state.health?.error ?? undefined}
        />
        <StatTile
          icon={<Terminal size={15} />}
          label={t('gateway.page.statistics.takeover')}
          value={t('gateway.page.statistics.takeoverCount', { count: activeCliCount })}
          meta={t('gateway.page.statistics.tokens', {
            input: formatInteger(inputTokens),
            output: formatInteger(outputTokens),
          })}
        />
        <StatTile
          icon={<FileText size={15} />}
          label={t('gateway.page.statistics.requestLog')}
          value={totalRequests > 0 ? String(totalRequests) : t(`gateway.page.logLevels.${logLevel}`)}
          tone={logLevel === 'off' ? 'muted' : 'default'}
          meta={
            totalRequests > 0
              ? t('gateway.page.statistics.successRate', {
                  rate: successRateText(successRequests, totalRequests),
                  failover: failoverRequests,
                })
              : state.settings?.metrics_enabled ? t('settings.gateway.fields.metrics') : undefined
          }
        />
      </div>

      <div className={styles.dataPanels}>
        <section className={styles.dataPanel}>
          <div className={styles.panelHeader}>
            <span>
              <BarChart3 size={14} aria-hidden="true" />
              {t('gateway.page.statistics.modelHealth')}
            </span>
          </div>
          {visibleHealthItems.length ? (
            <div className={styles.compactList}>
              {visibleHealthItems.map((item) => (
                <div
                  key={`${item.scope}:${item.cli_key}:${item.provider_id}:${item.upstream_model_id ?? '-'}`}
                  className={styles.compactRow}
                >
                  <span className={joinClassNames(styles.healthDot, styles[`healthDot_${item.state}`])} />
                  <span className={styles.compactMain}>
                    <strong>{modelHealthModelName(item)}</strong>
                    <small>
                      {t(`settings.gateway.cli.${item.cli_key}`)} · {modelHealthProviderName(item)}
                    </small>
                  </span>
                  <span className={styles.compactMeta}>
                    {t(`gateway.page.modelHealthState.${item.state}`)}
                    {item.failure_score > 0 ? ` · ${item.failure_score}` : ''}
                  </span>
                </div>
              ))}
            </div>
          ) : (
            <div className={styles.emptyState}>
              <Shield size={18} aria-hidden="true" />
              <span>{loading ? t('common.loading') : t('gateway.page.statistics.empty')}</span>
            </div>
          )}
        </section>
        <section className={styles.dataPanel}>
          <div className={styles.panelHeader}>
            <span>
              <Clock3 size={14} aria-hidden="true" />
              {t('gateway.page.statistics.latency')}
            </span>
          </div>
          {visibleMetricRollups.length ? (
            <div className={styles.compactList}>
              {visibleMetricRollups.map((rollup) => (
                <div
                  key={`${rollup.cli_key}:${rollup.provider_id}:${rollup.requested_model}:${rollup.upstream_model_id}`}
                  className={styles.compactRow}
                >
                  <span className={styles.compactMain}>
                    <strong>{rollup.requested_model}</strong>
                    <small>
                      {t(`settings.gateway.cli.${rollup.cli_key}`)} · {rollup.provider_id}
                    </small>
                  </span>
                  <span className={styles.compactMeta}>
                    {formatDuration(averageLatency(rollup))} · {successRateText(rollup.success_requests, rollup.total_requests)}
                  </span>
                </div>
              ))}
            </div>
          ) : (
            <div className={styles.emptyState}>
              <Gauge size={18} aria-hidden="true" />
              <span>{loading ? t('common.loading') : t('gateway.page.statistics.empty')}</span>
            </div>
          )}
        </section>
      </div>
    </div>
  );
};

export default GatewayStatisticsView;
