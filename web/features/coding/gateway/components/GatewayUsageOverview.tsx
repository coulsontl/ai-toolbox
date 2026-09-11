import React from 'react';
import { Activity, ArrowDownToLine, ArrowUpFromLine, CircleDollarSign, Database, Gauge, Sparkles, Zap } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { GatewayUsageSummary } from '@/services';
import { calculateCacheHitRate, formatCacheHitRate, formatCompactInteger, formatInteger, formatUsd } from '../utils/gatewayFormatters';
import styles from './GatewayUsageOverview.module.less';

interface GatewayUsageOverviewProps {
  summary: GatewayUsageSummary | null;
  requestsPerMinute: number | null;
}

const GatewayUsageOverview: React.FC<GatewayUsageOverviewProps> = ({ summary, requestsPerMinute }) => {
  const { t, i18n } = useTranslation();
  const cacheHitRate = summary ? calculateCacheHitRate(
    summary.total_input_tokens,
    summary.total_cache_read_tokens,
    summary.total_cache_creation_tokens,
  ) : null;
  const cacheHitPercent = cacheHitRate == null ? null : cacheHitRate * 100;
  const cacheHitRateLabel = t('gateway.page.statistics.columns.cacheHitRate');
  const requestRate = formatInteger(requestsPerMinute);
  const extraTokens = summary ? Math.max(0, summary.total_tokens - summary.total_input_tokens
    - summary.total_output_tokens - summary.total_cache_read_tokens - summary.total_cache_creation_tokens) : 0;
  const tokenMetrics = [
    { label: t('gateway.page.statistics.freshInput'), value: summary?.total_input_tokens, icon: ArrowDownToLine },
    { label: t('gateway.page.statistics.chart.output'), value: summary?.total_output_tokens, icon: ArrowUpFromLine },
    { label: t('gateway.page.statistics.cacheCreation'), value: summary?.total_cache_creation_tokens, icon: Database },
    { label: t('gateway.page.statistics.cacheRead'), value: summary?.total_cache_read_tokens, icon: Sparkles },
  ];

  return (
    <section className={styles.overview}>
      <div className={styles.header}>
        <div className={styles.tokenTotal}>
          <span className={styles.tokenIcon}><Zap size={20} aria-hidden="true" /></span>
          <div className={styles.tokenTotalContent}>
            <span className={styles.label}>{t('gateway.page.statistics.summaryTokens')}</span>
            <div className={styles.tokenTotalValues}>
              <strong className={styles.tokenTotalValue}>{formatInteger(summary?.total_tokens)}</strong>
              {summary && summary.total_tokens >= 1000 ? (
                <span className={styles.tokenApproximation}>
                  {t('gateway.page.statistics.approximateTokens', {
                    value: formatCompactInteger(summary.total_tokens, i18n.language),
                  })}
                </span>
              ) : null}
            </div>
            {extraTokens > 0 && <span className={styles.label}>{t('gateway.page.requests.nativeUsage.extraShort', { value: formatInteger(extraTokens) })}</span>}
          </div>
        </div>

        <div className={styles.requestAndCost}>
          <div className={styles.rateSummary} title={t('gateway.page.requestRateHint')}>
            <span className={styles.metricLabel}>
              <Gauge size={14} aria-hidden="true" />
              {t('gateway.page.rpm')}
            </span>
            <strong className={styles.summaryValue}>{requestRate}</strong>
          </div>
          <div className={styles.requestSummary} title={t('gateway.page.requests.nativeUsage.granularityHint')}>
            <span className={styles.metricLabel}>
              <Activity size={14} aria-hidden="true" />
              {t('gateway.page.statistics.summaryRequests')}
            </span>
            <strong className={styles.summaryValue}>{formatInteger(summary?.total_requests)}</strong>
          </div>
          <div className={styles.costSummary}>
            <span className={styles.metricLabel}>
              <CircleDollarSign size={14} aria-hidden="true" />
              {t('gateway.page.statistics.summaryCost')}
            </span>
            <strong className={styles.summaryValue}>{summary ? formatUsd(summary.total_cost_usd, 2) : '-'}</strong>
          </div>
        </div>
      </div>

      <div className={styles.tokenBreakdown}>
        {tokenMetrics.map(({ label, value, icon: Icon }) => (
          <div className={styles.tokenMetric} key={label}>
            <span className={styles.metricLabel}><Icon size={14} aria-hidden="true" />{label}</span>
            <strong className={styles.metricValue} title={formatInteger(value)}>
              {formatCompactInteger(value, i18n.language)}
            </strong>
          </div>
        ))}
        <div className={styles.cacheHitMetric} title={t('gateway.page.statistics.cacheHitRateHint')}>
          <div className={styles.cacheHitHeading}>
            <span className={styles.label}>{cacheHitRateLabel}</span>
            <strong>{formatCacheHitRate(cacheHitRate)}</strong>
          </div>
          <div
            className={styles.progressTrack}
            role={cacheHitPercent == null ? undefined : 'meter'}
            aria-label={cacheHitPercent == null ? undefined : cacheHitRateLabel}
            aria-valuemin={cacheHitPercent == null ? undefined : 0}
            aria-valuemax={cacheHitPercent == null ? undefined : 100}
            aria-valuenow={cacheHitPercent ?? undefined}
            aria-valuetext={cacheHitPercent == null ? undefined : formatCacheHitRate(cacheHitRate)}
            aria-hidden={cacheHitPercent == null ? true : undefined}
          >
            <span className={styles.progressFill} style={{ width: `${cacheHitPercent ?? 0}%` }} />
          </div>
        </div>
      </div>
    </section>
  );
};

export default GatewayUsageOverview;
