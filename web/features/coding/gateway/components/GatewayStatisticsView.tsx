import React from 'react';
import { DatePicker, Empty, Select, Table } from 'antd';
import type { ColumnsType } from 'antd/es/table';
import {
  AlertCircle,
  BarChart3,
  CalendarDays,
  Clock,
  DollarSign,
  Gauge,
  RefreshCw,
  Server,
  Terminal,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  Area,
  AreaChart,
  CartesianGrid,
  Label,
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
  type GatewayUsageTool,
  GATEWAY_USAGE_TOOLS,
  type GatewayModelStats,
  type GatewayProviderStats,
  type GatewayUsageSummary,
  type GatewayUsageTrendPoint,
  type ProxyGatewayStatus,
} from '@/services';
import {
  formatCacheHitRate,
  formatCompactInteger,
  formatDuration,
  formatGatewayError,
  formatInteger,
  formatUsd,
  getGatewayRequestsPerMinute,
  resolveGatewayUsageRange,
  type GatewayUsageRangePreset,
  type GatewayUsageRangeSelection,
} from '../utils/gatewayFormatters';
import ModelPricingModal from './ModelPricingModal';
import GatewayUsageOverview from './GatewayUsageOverview';
import styles from './GatewayStatisticsView.module.less';

const { RangePicker } = DatePicker;

type GatewayCliFilter = 'all' | GatewayUsageTool;
type StatsTabKey = 'providers' | 'models';
type TrendSeriesKey = 'input' | 'output' | 'cache' | 'other' | 'cost';

interface GatewayStatisticsViewProps {
  refreshKey?: number;
  gatewayStatus?: ProxyGatewayStatus | null;
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

const cliOptions: GatewayCliFilter[] = ['all', ...GATEWAY_USAGE_TOOLS];
const rangeOptions: GatewayUsageRangePreset[] = ['today', '1d', '7d', '14d', '30d', 'custom'];
const trendSeriesKeys: readonly TrendSeriesKey[] = ['input', 'output', 'cache', 'other', 'cost'];
const trendCurveType = 'monotoneX' as const;
const dateOnlyBucketPattern = /^(\d{4})-(\d{2})-(\d{2})$/;
const dateTimeBucketPattern = /^(\d{4})-(\d{2})-(\d{2})[T\s](\d{2}):(\d{2})/;

const toCliKey = (value: GatewayCliFilter): GatewayUsageTool | undefined =>
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
      other: Math.max(0, item.total_tokens - item.input_tokens - item.output_tokens - item.cache_read_tokens - item.cache_creation_tokens),
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
  if (providerId === 'session') {
    return t('gateway.page.requests.localSession');
  }
  if (providerId === 'unknown') {
    return t('gateway.page.statistics.providerUnselected');
  }
  return providerId;
};

const providerDisplayMeta = (
  t: ReturnType<typeof useTranslation>['t'],
  cliKey: GatewayUsageTool,
  providerId: string,
) => {
  const cliLabel = t(`settings.gateway.cli.${cliKey}`);
  if (providerId === 'unknown' || providerId === 'session') {
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

const GatewayStatisticsView: React.FC<GatewayStatisticsViewProps> = ({ refreshKey = 0, gatewayStatus }) => {
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
  const statisticsRequestIdRef = React.useRef(0);

  const effectiveCliKey = toCliKey(cliFilter);

  const loadStatistics = React.useCallback(async () => {
    const requestId = ++statisticsRequestIdRef.current;
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
      if (requestId === statisticsRequestIdRef.current) {
        setState({ summary, trends, providerStats, modelStats });
      }
    } catch (loadError) {
      if (requestId === statisticsRequestIdRef.current) {
        setError(t('gateway.page.statistics.loadFailed', { error: formatGatewayError(loadError) }));
      }
    } finally {
      if (requestId === statisticsRequestIdRef.current) {
        setLoading(false);
      }
    }
  }, [effectiveCliKey, range, t]);

  React.useEffect(() => {
    // A new filter must not display the previous filter's totals while loading.
    setState(emptyState);
  }, [effectiveCliKey, range]);

  React.useEffect(() => {
    void loadStatistics();
    return () => {
      statisticsRequestIdRef.current += 1;
    };
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

  const requestRate = getGatewayRequestsPerMinute(gatewayStatus, effectiveCliKey);
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
          <strong title={providerDisplayName(t, record.provider_id, record.provider_name)}>{providerDisplayName(t, record.provider_id, record.provider_name)}</strong>
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
      title: <span title={t('gateway.page.statistics.cacheHitRateHint')}>{t('gateway.page.statistics.columns.cacheHitRate')}</span>,
      dataIndex: 'cache_hit_rate',
      width: 120,
      align: 'right',
      render: (value: number | null) => formatCacheHitRate(value),
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
      render: (value: number, record) => record.provider_id === 'session' ? '-' : (
        <span style={{ color: statusColor(value) }}>{value.toFixed(1)}%</span>
      ),
    },
    {
      title: t('gateway.page.statistics.columns.latency'),
      dataIndex: 'avg_latency_ms',
      width: 110,
      align: 'right',
      render: (value: number | null) => value == null ? '-' : formatDuration(value),
    },
  ];

  const modelColumns: ColumnsType<GatewayModelStats> = [
    {
      title: t('gateway.page.statistics.columns.model'),
      dataIndex: 'model',
      render: (value: string, record) => (
        <div className={styles.tableMainCell}>
          <strong title={value === 'unknown' ? undefined : value}>
            {value === 'unknown' ? t('gateway.page.statistics.modelUnavailable') : value}
          </strong>
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
      render: (value: number | null) => value == null ? '-' : formatDuration(value),
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

      <GatewayUsageOverview summary={state.summary} requestsPerMinute={requestRate} />
      <p className={styles.usageNote}>{t('gateway.page.requests.nativeUsage.overviewHint')}</p>

      <section className={styles.chartPanel}>
        <div className={styles.panelHeader}>
          <span>
            <BarChart3 className={styles.panelIcon} size={14} aria-hidden="true" />
            {t('gateway.page.statistics.trends')}
          </span>
        </div>
        <div className={styles.chartBody}>
          {chartRows.length ? (
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={chartRows} margin={{ top: 12, right: 24, left: 8, bottom: 4 }}>
                <CartesianGrid stroke="var(--color-border)" strokeDasharray="4 4" vertical={false} />
                <XAxis
                  dataKey="label"
                  axisLine={{ stroke: 'var(--color-border)' }}
                  tickLine={false}
                  tick={{ fill: 'var(--color-text-tertiary)', fontSize: 11 }}
                />
                <YAxis
                  yAxisId="tokens"
                  axisLine={{ stroke: 'var(--color-border)' }}
                  tickLine={false}
                  tick={{ fill: 'var(--color-text-tertiary)', fontSize: 11 }}
                  tickFormatter={(value) => formatCompactInteger(Number(value))}
                >
                  <Label
                    value={t('gateway.page.statistics.columns.tokens')}
                    angle={-90}
                    position="insideLeft"
                    fill="var(--color-text-tertiary)"
                    style={{ fontSize: 10 }}
                  />
                </YAxis>
                <YAxis
                  yAxisId="cost"
                  orientation="right"
                  axisLine={{ stroke: 'var(--color-border)' }}
                  tickLine={false}
                  tick={{ fill: 'var(--color-text-tertiary)', fontSize: 11 }}
                  tickFormatter={(value) => `$${value}`}
                >
                  <Label
                    value={t('gateway.page.statistics.columns.cost')}
                    angle={90}
                    position="insideRight"
                    fill="var(--color-text-tertiary)"
                    style={{ fontSize: 10 }}
                  />
                </YAxis>
                <Tooltip
                  contentStyle={{
                    background: 'var(--color-bg-elevated)',
                    border: '1px solid var(--color-border)',
                    color: 'var(--color-text-primary)',
                  }}
                  cursor={{ stroke: 'var(--color-border-secondary)', strokeDasharray: '4 4' }}
                />
                <Legend
                  formatter={renderTrendLegendLabel}
                  onClick={handleTrendLegendClick}
                  iconType="plainline"
                  wrapperStyle={{ paddingTop: 8 }}
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
                {chartRows.some((row) => row.other > 0) && (
                  <Area
                    yAxisId="tokens"
                    type={trendCurveType}
                    dataKey="other"
                    name={t('gateway.page.statistics.chart.other')}
                    hide={hiddenSeries.has('other')}
                    stroke="var(--ant-color-primary)"
                    fill="var(--ant-color-primary)"
                    fillOpacity={0.1}
                    strokeWidth={2}
                  />
                )}
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
            {activeStatsTab === 'providers' ? (
              <Server className={styles.panelIcon} size={14} aria-hidden="true" />
            ) : (
              <Gauge className={styles.panelIcon} size={14} aria-hidden="true" />
            )}
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
            tableLayout="fixed"
            columns={providerColumns}
            dataSource={state.providerStats}
            loading={loading}
            pagination={false}
            scroll={{ x: 880 }}
          />
        ) : (
          <Table
            rowKey={(record) => `${record.cli_key}:${record.model}`}
            size="small"
            tableLayout="fixed"
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
