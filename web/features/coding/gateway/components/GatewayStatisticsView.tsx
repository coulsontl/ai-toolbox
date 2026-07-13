import React from 'react';
import { DatePicker, Empty, Select, Table } from 'antd';
import type { ColumnsType } from 'antd/es/table';
import {
  Activity,
  AlertCircle,
  BarChart3,
  CalendarDays,
  Clock,
  Coins,
  DollarSign,
  Gauge,
  RefreshCw,
  Server,
  Terminal,
  Zap,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  Area,
  AreaChart,
  CartesianGrid,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';
import { ManagementSegmented } from '../../shared/management';
import {
  getProxyGatewayModelStats,
  getProxyGatewayProviderStats,
  getProxyGatewayUsageSummary,
  getProxyGatewayUsageTrends,
  type GatewayCliKey,
  type GatewayModelStats,
  type GatewayProviderStats,
  type GatewayUsageSummary,
  type GatewayUsageTrendPoint,
} from '@/services';
import {
  formatCompactInteger,
  formatDuration,
  formatGatewayError,
  formatInteger,
  formatUsd,
  resolveGatewayUsageRange,
  type GatewayUsageRangePreset,
  type GatewayUsageRangeSelection,
} from '../utils/gatewayFormatters';
import ModelPricingModal from './ModelPricingModal';
import StatTile from './StatTile';
import styles from './GatewayStatisticsView.module.less';

const { RangePicker } = DatePicker;

type GatewayCliFilter = 'all' | GatewayCliKey;
type StatsTabKey = 'providers' | 'models';
type TrendSeriesKey = 'input' | 'output' | 'cache' | 'cost';

interface GatewayStatisticsViewProps {
  refreshKey?: number;
}

interface StatisticsState {
  summary: GatewayUsageSummary | null;
  trends: GatewayUsageTrendPoint[];
  providerStats: GatewayProviderStats[];
  modelStats: GatewayModelStats[];
}

const emptyState: StatisticsState = {
  summary: null,
  trends: [],
  providerStats: [],
  modelStats: [],
};

const cliOptions: GatewayCliFilter[] = ['all', 'claude', 'codex', 'grok', 'gemini'];
const rangeOptions: GatewayUsageRangePreset[] = ['today', '1d', '7d', '14d', '30d', 'custom'];
const trendSeriesKeys: readonly TrendSeriesKey[] = ['input', 'output', 'cache', 'cost'];
const trendCurveType = 'monotoneX' as const;
const dateOnlyBucketPattern = /^(\d{4})-(\d{2})-(\d{2})$/;
const dateTimeBucketPattern = /^(\d{4})-(\d{2})-(\d{2})[T\s](\d{2}):(\d{2})/;

const toCliKey = (value: GatewayCliFilter): GatewayCliKey | undefined =>
  value === 'all' ? undefined : value;

const statusColor = (rate: number) => {
  if (rate >= 95) {
    return 'var(--color-status-success)';
  }
  if (rate >= 80) {
    return 'var(--color-status-warning)';
  }
  return 'var(--color-status-error)';
};

const padTwoDigits = (value: number) => String(value).padStart(2, '0');

const formatTrendDateLabel = (value: string) => {
  const dateTimeMatch = value.match(dateTimeBucketPattern);
  if (dateTimeMatch) {
    const [, , month, day, hour, minute] = dateTimeMatch;
    return `${month}/${day} ${hour}:${minute}`;
  }

  const dateOnlyMatch = value.match(dateOnlyBucketPattern);
  if (dateOnlyMatch) {
    const [, , month, day] = dateOnlyMatch;
    return `${month}/${day}`;
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }

  const monthDay = `${padTwoDigits(date.getMonth() + 1)}/${padTwoDigits(date.getDate())}`;
  const hasTime = value.includes('T') || /\d{1,2}:\d{2}/.test(value);
  if (!hasTime) {
    return monthDay;
  }
  return `${monthDay} ${padTwoDigits(date.getHours())}:${padTwoDigits(date.getMinutes())}`;
};

const chartData = (trends: GatewayUsageTrendPoint[]) =>
  trends.map((item) => {
    return {
      label: formatTrendDateLabel(item.date),
      input: item.input_tokens,
      output: item.output_tokens,
      cache: item.cache_read_tokens + item.cache_creation_tokens,
      cost: Number.parseFloat(item.total_cost_usd) || 0,
    };
  });

const providerDisplayName = (
  t: ReturnType<typeof useTranslation>['t'],
  providerId: string,
  providerName: string | null,
) => {
  if (providerName) {
    return providerName;
  }
  if (providerId === 'unknown') {
    return t('gateway.page.statistics.providerUnselected');
  }
  return providerId;
};

const providerDisplayMeta = (
  t: ReturnType<typeof useTranslation>['t'],
  cliKey: GatewayCliKey,
  providerId: string,
) => {
  const cliLabel = t(`settings.gateway.cli.${cliKey}`);
  if (providerId === 'unknown') {
    return cliLabel;
  }
  return `${cliLabel} · ${providerId}`;
};

const isTrendSeriesKey = (value: unknown): value is TrendSeriesKey =>
  typeof value === 'string' && trendSeriesKeys.includes(value as TrendSeriesKey);

const trendLegendDataKey = (payload: unknown): TrendSeriesKey | null => {
  if (!payload || typeof payload !== 'object') {
    return null;
  }
  const dataKey = (payload as { dataKey?: unknown }).dataKey;
  return isTrendSeriesKey(dataKey) ? dataKey : null;
};

const GatewayStatisticsView: React.FC<GatewayStatisticsViewProps> = ({ refreshKey = 0 }) => {
  const { t } = useTranslation();
  const [cliFilter, setCliFilter] = React.useState<GatewayCliFilter>('all');
  const [range, setRange] = React.useState<GatewayUsageRangeSelection>({ preset: 'today' });
  const [activeStatsTab, setActiveStatsTab] = React.useState<StatsTabKey>('providers');
  const [refreshIntervalMs, setRefreshIntervalMs] = React.useState(30_000);
  const [showPricingModal, setShowPricingModal] = React.useState(false);
  const [hiddenSeries, setHiddenSeries] = React.useState<Set<TrendSeriesKey>>(() => new Set());
  const [state, setState] = React.useState<StatisticsState>(emptyState);
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const effectiveCliKey = toCliKey(cliFilter);

  const loadStatistics = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const resolvedRange = resolveGatewayUsageRange(range);
      const [summary, trends, providerStats, modelStats] = await Promise.all([
        getProxyGatewayUsageSummary(
          resolvedRange.startDate,
          resolvedRange.endDate,
          effectiveCliKey,
        ),
        getProxyGatewayUsageTrends(
          resolvedRange.startDate,
          resolvedRange.endDate,
          effectiveCliKey,
        ),
        getProxyGatewayProviderStats(
          resolvedRange.startDate,
          resolvedRange.endDate,
          effectiveCliKey,
        ),
        getProxyGatewayModelStats(
          resolvedRange.startDate,
          resolvedRange.endDate,
          effectiveCliKey,
        ),
      ]);
      setState({ summary, trends, providerStats, modelStats });
    } catch (loadError) {
      setError(t('gateway.page.statistics.loadFailed', { error: formatGatewayError(loadError) }));
    } finally {
      setLoading(false);
    }
  }, [effectiveCliKey, range, t]);

  React.useEffect(() => {
    void loadStatistics();
  }, [loadStatistics, refreshKey]);

  React.useEffect(() => {
    if (refreshIntervalMs <= 0) {
      return undefined;
    }
    const timer = window.setInterval(() => {
      void loadStatistics();
    }, refreshIntervalMs);
    return () => window.clearInterval(timer);
  }, [loadStatistics, refreshIntervalMs]);

  const summary = state.summary;
  const successRate = summary?.success_rate ?? 0;
  const totalCacheTokens =
    (summary?.total_cache_read_tokens ?? 0) + (summary?.total_cache_creation_tokens ?? 0);
  const chartRows = chartData(state.trends);

  const handleTrendLegendClick = React.useCallback((payload: unknown) => {
    const dataKey = trendLegendDataKey(payload);
    if (!dataKey) {
      return;
    }
    setHiddenSeries((current) => {
      const next = new Set(current);
      if (next.has(dataKey)) {
        next.delete(dataKey);
      } else {
        next.add(dataKey);
      }
      return next;
    });
  }, []);

  const renderTrendLegendLabel = React.useCallback(
    (value: unknown, payload: unknown) => {
      const dataKey = trendLegendDataKey(payload);
      const hidden = dataKey ? hiddenSeries.has(dataKey) : false;
      const className = hidden
        ? `${styles.legendLabel} ${styles.legendLabelHidden}`
        : styles.legendLabel;
      return <span className={className}>{String(value)}</span>;
    },
    [hiddenSeries],
  );

  const providerColumns: ColumnsType<GatewayProviderStats> = [
    {
      title: t('gateway.page.statistics.columns.provider'),
      dataIndex: 'provider_name',
      render: (_, record) => (
        <div className={styles.tableMainCell}>
          <strong>{providerDisplayName(t, record.provider_id, record.provider_name)}</strong>
          <small>{providerDisplayMeta(t, record.cli_key, record.provider_id)}</small>
        </div>
      ),
    },
    {
      title: t('gateway.page.statistics.columns.requests'),
      dataIndex: 'request_count',
      width: 110,
      align: 'right',
      render: (value: number) => formatInteger(value),
    },
    {
      title: t('gateway.page.statistics.columns.tokens'),
      dataIndex: 'total_tokens',
      width: 130,
      align: 'right',
      render: (value: number) => formatCompactInteger(value),
    },
    {
      title: t('gateway.page.statistics.columns.cost'),
      dataIndex: 'total_cost_usd',
      width: 120,
      align: 'right',
      render: (value: string) => formatUsd(value, 6),
    },
    {
      title: t('gateway.page.statistics.columns.successRate'),
      dataIndex: 'success_rate',
      width: 110,
      align: 'right',
      render: (value: number) => (
        <span style={{ color: statusColor(value) }}>{value.toFixed(1)}%</span>
      ),
    },
    {
      title: t('gateway.page.statistics.columns.latency'),
      dataIndex: 'avg_latency_ms',
      width: 110,
      align: 'right',
      render: (value: number) => formatDuration(value),
    },
  ];

  const modelColumns: ColumnsType<GatewayModelStats> = [
    {
      title: t('gateway.page.statistics.columns.model'),
      dataIndex: 'model',
      render: (value: string, record) => (
        <div className={styles.tableMainCell}>
          <strong>{value}</strong>
          <small>{t(`settings.gateway.cli.${record.cli_key}`)}</small>
        </div>
      ),
    },
    {
      title: t('gateway.page.statistics.columns.requests'),
      dataIndex: 'request_count',
      width: 110,
      align: 'right',
      render: (value: number) => formatInteger(value),
    },
    {
      title: t('gateway.page.statistics.columns.tokens'),
      dataIndex: 'total_tokens',
      width: 130,
      align: 'right',
      render: (value: number) => formatCompactInteger(value),
    },
    {
      title: t('gateway.page.statistics.columns.cost'),
      dataIndex: 'total_cost_usd',
      width: 120,
      align: 'right',
      render: (value: string) => formatUsd(value, 6),
    },
    {
      title: t('gateway.page.statistics.columns.latency'),
      dataIndex: 'avg_latency_ms',
      width: 110,
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

      <div className={styles.filterBar}>
        <div className={styles.filterSection}>
          <Terminal className={styles.filterIcon} size={14} aria-hidden="true" />
          <Select
            variant="borderless"
            size="small"
            className={styles.filterSelect}
            popupMatchSelectWidth={false}
            value={cliFilter}
            options={cliOptions.map((option) => ({
              value: option,
              label: option === 'all' ? t('gateway.page.statistics.filters.all') : t(`settings.gateway.cli.${option}`),
            }))}
            onChange={(value) => setCliFilter(value as GatewayCliFilter)}
          />
        </div>

        <span className={styles.filterDivider} aria-hidden="true" />

        <div className={styles.filterSection}>
          <CalendarDays className={styles.filterIcon} size={14} aria-hidden="true" />
          <Select
            variant="borderless"
            size="small"
            className={styles.filterSelect}
            popupMatchSelectWidth={false}
            value={range.preset}
            options={rangeOptions.map((option) => ({
              value: option,
              label: t(`gateway.page.statistics.range.${option}`),
            }))}
            onChange={(value) =>
              setRange((currentRange) => ({
                preset: value as GatewayUsageRangePreset,
                customRange: value === 'custom' ? currentRange.customRange : undefined,
              }))
            }
          />
          {range.preset === 'custom' ? (
            <RangePicker
              showTime
              variant="borderless"
              size="small"
              className={styles.customRangePicker}
              value={range.customRange as never}
              onChange={(dates) => setRange({ preset: 'custom', customRange: dates as never })}
            />
          ) : null}
        </div>

        <div className={styles.filterActions}>
          <span className={styles.filterDivider} aria-hidden="true" />
          <div className={styles.filterSection}>
            <Clock className={styles.filterIcon} size={14} aria-hidden="true" />
            <Select
              variant="borderless"
              size="small"
              className={styles.filterSelect}
              popupMatchSelectWidth={false}
              value={refreshIntervalMs}
              options={[
                { value: 0, label: t('gateway.page.statistics.refresh.off') },
                { value: 5_000, label: '5s' },
                { value: 10_000, label: '10s' },
                { value: 30_000, label: '30s' },
                { value: 60_000, label: '60s' },
              ]}
              onChange={(value) => setRefreshIntervalMs(Number(value))}
            />
          </div>
          <button
            type="button"
            className={styles.refreshButton}
            onClick={() => void loadStatistics()}
            disabled={loading}
            aria-label={t('common.refresh')}
            title={t('common.refresh')}
          >
            <RefreshCw size={14} className={loading ? styles.spin : undefined} aria-hidden="true" />
          </button>
          <button
            type="button"
            className={styles.refreshButton}
            onClick={() => setShowPricingModal(true)}
            aria-label={t('gateway.page.pricing.open')}
            title={t('gateway.page.pricing.open')}
          >
            <DollarSign size={14} aria-hidden="true" />
          </button>
        </div>
      </div>

      <div className={styles.statGrid}>
        <StatTile
          icon={<Activity size={15} />}
          label={t('gateway.page.statistics.summaryRequests')}
          value={formatInteger(summary?.total_requests ?? 0)}
          meta={t('gateway.page.statistics.successRateOnly', { rate: successRate.toFixed(1) })}
          tone="traffic"
        />
        <StatTile
          icon={<Zap size={15} />}
          label={t('gateway.page.statistics.summaryTokens')}
          value={formatCompactInteger(summary?.total_tokens ?? 0)}
          meta={t('gateway.page.statistics.tokens', {
            input: formatCompactInteger(summary?.total_input_tokens ?? 0),
            output: formatCompactInteger(summary?.total_output_tokens ?? 0),
            cache: formatCompactInteger(totalCacheTokens),
          })}
          tone="info"
        />
        <StatTile
          icon={<Coins size={15} />}
          label={t('gateway.page.statistics.summaryCost')}
          value={formatUsd(summary?.total_cost_usd ?? '0', 6)}
          meta={t('gateway.page.statistics.dbSummaryOnly')}
          tone="warning"
        />
      </div>

      <section className={styles.chartPanel}>
        <div className={styles.panelHeader}>
          <span>
            <BarChart3 size={14} aria-hidden="true" />
            {t('gateway.page.statistics.trends')}
          </span>
        </div>
        <div className={styles.chartBody}>
          {chartRows.length ? (
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={chartRows} margin={{ top: 10, right: 18, left: 0, bottom: 0 }}>
                <CartesianGrid stroke="var(--color-border)" strokeDasharray="3 3" vertical={false} />
                <XAxis dataKey="label" tick={{ fill: 'var(--color-text-tertiary)', fontSize: 11 }} />
                <YAxis
                  yAxisId="tokens"
                  tick={{ fill: 'var(--color-text-tertiary)', fontSize: 11 }}
                  tickFormatter={(value) => formatCompactInteger(Number(value))}
                />
                <YAxis
                  yAxisId="cost"
                  orientation="right"
                  tick={{ fill: 'var(--color-text-tertiary)', fontSize: 11 }}
                  tickFormatter={(value) => `$${value}`}
                />
                <Tooltip
                  contentStyle={{
                    background: 'var(--color-bg-elevated)',
                    border: '1px solid var(--color-border)',
                    color: 'var(--color-text-primary)',
                  }}
                />
                <Legend
                  formatter={renderTrendLegendLabel}
                  onClick={handleTrendLegendClick}
                />
                <Area
                  yAxisId="tokens"
                  type={trendCurveType}
                  dataKey="input"
                  name={t('gateway.page.statistics.chart.input')}
                  hide={hiddenSeries.has('input')}
                  stroke="var(--color-border-secondary)"
                  fill="var(--color-border-secondary)"
                  fillOpacity={0.16}
                  strokeWidth={2}
                />
                <Area
                  yAxisId="tokens"
                  type={trendCurveType}
                  dataKey="output"
                  name={t('gateway.page.statistics.chart.output')}
                  hide={hiddenSeries.has('output')}
                  stroke="var(--color-status-success)"
                  fill="var(--color-status-success)"
                  fillOpacity={0.12}
                  strokeWidth={2}
                />
                <Area
                  yAxisId="tokens"
                  type={trendCurveType}
                  dataKey="cache"
                  name={t('gateway.page.statistics.chart.cache')}
                  hide={hiddenSeries.has('cache')}
                  stroke="var(--color-status-warning)"
                  fill="var(--color-status-warning)"
                  fillOpacity={0.1}
                  strokeWidth={2}
                />
                <Area
                  yAxisId="cost"
                  type={trendCurveType}
                  dataKey="cost"
                  name={t('gateway.page.statistics.chart.cost')}
                  hide={hiddenSeries.has('cost')}
                  stroke="var(--color-status-error)"
                  fill="var(--color-status-error)"
                  fillOpacity={0.08}
                  strokeWidth={2}
                />
              </AreaChart>
            </ResponsiveContainer>
          ) : (
            <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={loading ? t('common.loading') : t('gateway.page.statistics.empty')} />
          )}
        </div>
      </section>

      <section className={styles.dataPanel}>
        <div className={styles.panelHeader}>
          <span>
            {activeStatsTab === 'providers' ? <Server size={14} aria-hidden="true" /> : <Gauge size={14} aria-hidden="true" />}
            {t('gateway.page.statistics.breakdown')}
          </span>
          <ManagementSegmented<StatsTabKey>
            value={activeStatsTab}
            options={[
              { value: 'providers', label: t('gateway.page.statistics.providerStats') },
              { value: 'models', label: t('gateway.page.statistics.modelStats') },
            ]}
            onChange={setActiveStatsTab}
            ariaLabel={t('gateway.page.statistics.breakdown')}
          />
        </div>
        {activeStatsTab === 'providers' ? (
          <Table
            rowKey={(record) => `${record.cli_key}:${record.provider_id}`}
            size="small"
            columns={providerColumns}
            dataSource={state.providerStats}
            loading={loading}
            pagination={false}
            scroll={{ x: 760 }}
          />
        ) : (
          <Table
            rowKey={(record) => `${record.cli_key}:${record.model}`}
            size="small"
            columns={modelColumns}
            dataSource={state.modelStats}
            loading={loading}
            pagination={false}
            scroll={{ x: 680 }}
          />
        )}
      </section>

      <ModelPricingModal
        open={showPricingModal}
        onClose={() => setShowPricingModal(false)}
      />
    </div>
  );
};

export default GatewayStatisticsView;
