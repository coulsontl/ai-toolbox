import React from 'react';
import { App, Input, Select, Spin, Switch } from 'antd';
import {
  Check,
  Download,
  Loader2,
  RefreshCw,
  Search,
  Settings2,
  Store,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  getPluginMarketListings,
  installPlugin,
  listPluginMarkets,
  refreshPluginMarkets,
  type MarketPluginListing,
  type MarketSourceRecord,
} from '@/services';
import {
  formatCompactNumber,
  formatPluginError,
  getPluginKindCategory,
  getPluginSummary,
} from '../utils/pluginFormatters';
import PluginIcon from './PluginIcon';
import MarketRegistryModal from './MarketRegistryModal';
import styles from './PluginMarketView.module.less';

type MarketKindFilter = 'all' | 'provider' | 'middleware' | 'page';

interface PluginMarketViewProps {
  refreshKey: number;
  onDataChanged: () => Promise<void>;
  onListingCountChange: (count: number) => void;
}

const PluginMarketView: React.FC<PluginMarketViewProps> = ({
  refreshKey,
  onDataChanged,
  onListingCountChange,
}) => {
  const { t, i18n } = useTranslation();
  const { message } = App.useApp();
  const [markets, setMarkets] = React.useState<MarketSourceRecord[]>([]);
  const [listings, setListings] = React.useState<MarketPluginListing[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [kindFilter, setKindFilter] = React.useState<MarketKindFilter>('all');
  const [marketFilter, setMarketFilter] = React.useState('all');
  const [searchInput, setSearchInput] = React.useState('');
  const [appliedSearch, setAppliedSearch] = React.useState('');
  const [includeNpmSearch, setIncludeNpmSearch] = React.useState(false);
  const [installingPackage, setInstallingPackage] = React.useState<string | null>(null);
  const [refreshingMarkets, setRefreshingMarkets] = React.useState(false);
  const [registryOpen, setRegistryOpen] = React.useState(false);
  // Bumped when the empty state resets every filter; state updates are batched,
  // so requesting from the click handler would still read the old filters.
  const [reloadToken, setReloadToken] = React.useState(0);
  const requestIdRef = React.useRef(0);

  const loadMarkets = React.useCallback(async () => {
    try {
      const nextMarkets = await listPluginMarkets();
      setMarkets(nextMarkets);
    } catch (marketError) {
      setError(formatPluginError(marketError));
    }
  }, []);

  React.useEffect(() => {
    void loadMarkets();
  }, [loadMarkets, refreshKey]);

  React.useEffect(() => {
    const timer = window.setTimeout(() => {
      setAppliedSearch(searchInput.trim());
    }, 400);
    return () => window.clearTimeout(timer);
  }, [searchInput]);

  React.useEffect(() => {
    if (marketFilter !== 'all' && !markets.some((market) => market.id === marketFilter)) {
      setMarketFilter('all');
    }
  }, [marketFilter, markets]);

  const loadListings = React.useCallback(async (): Promise<boolean> => {
    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;
    setLoading(true);
    try {
      const nextListings = await getPluginMarketListings({
        search: appliedSearch || null,
        kind: kindFilter === 'all' ? null : kindFilter,
        marketId: marketFilter === 'all' ? null : marketFilter,
        includeNpmSearch,
      });
      if (requestIdRef.current !== requestId) return true;
      setListings(nextListings);
      setError(null);
      onListingCountChange(nextListings.length);
      return true;
    } catch (listingError) {
      if (requestIdRef.current !== requestId) return false;
      setError(
        t('plugins.market.loadFailed', { error: formatPluginError(listingError) }),
      );
      return false;
    } finally {
      if (requestIdRef.current === requestId) {
        setLoading(false);
      }
    }
  }, [appliedSearch, includeNpmSearch, kindFilter, marketFilter, onListingCountChange, t]);

  React.useEffect(() => {
    void loadListings();
  }, [loadListings, refreshKey, reloadToken]);

  const handleMarketChanged = async () => {
    await Promise.all([loadMarkets(), loadListings()]);
  };

  const handleRefreshMarket = async () => {
    setRefreshingMarkets(true);
    try {
      const nextMarkets = await refreshPluginMarkets();
      setMarkets(nextMarkets);
      const listingsLoaded = await loadListings();
      if (listingsLoaded) {
        message.success(t('plugins.page.notice.refreshed'));
      }
    } catch (refreshError) {
      setError(
        t('plugins.market.loadFailed', { error: formatPluginError(refreshError) }),
      );
    } finally {
      setRefreshingMarkets(false);
    }
  };

  const handleInstall = async (listing: MarketPluginListing) => {
    setInstallingPackage(listing.package);
    setError(null);
    try {
      await installPlugin(listing.package, listing.marketIds[0] ?? null);
      message.success(
        t('plugins.market.installSuccess', { name: listing.name || listing.package }),
      );
      await onDataChanged();
      await loadListings();
    } catch (installError) {
      setError(
        t('plugins.market.installFailed', { error: formatPluginError(installError) }),
      );
    } finally {
      setInstallingPackage(null);
    }
  };

  const marketNameFor = (listing: MarketPluginListing) => {
    const name = listing.marketIds
      .map((marketId) => markets.find((market) => market.id === marketId)?.name)
      .find(Boolean);
    return name ?? (listing.community ? t('plugins.market.community') : '');
  };

  const kindLabels: Record<MarketKindFilter, string> = {
    all: t('plugins.market.filter.all'),
    provider: t('plugins.market.filter.provider'),
    middleware: t('plugins.market.filter.middleware'),
    page: t('plugins.market.filter.page'),
  };

  const kindBadge = (kind: string) => {
    const category = getPluginKindCategory(kind);
    if (category === 'middleware') {
      return { text: t('plugins.kind.middleware'), className: styles.kindMiddleware };
    }
    if (category === 'both') {
      return { text: t('plugins.kind.both'), className: styles.kindBoth };
    }
    if (category === 'page') {
      return { text: t('plugins.kind.page'), className: styles.kindPage };
    }
    if (category === 'unknown') {
      return { text: t('plugins.kind.unknown'), className: styles.kindUnknown };
    }
    return { text: t('plugins.kind.provider'), className: styles.kindProvider };
  };

  return (
    <div className={styles.view}>
      <div className={styles.filterBar}>
        <div className={styles.segmented}>
          {(['all', 'provider', 'middleware', 'page'] as MarketKindFilter[]).map((key) => (
            <button
              key={key}
              type="button"
              className={`${styles.segmentButton} ${kindFilter === key ? styles.segmentButtonActive : ''}`}
              onClick={() => setKindFilter(key)}
            >
              {kindLabels[key]}
            </button>
          ))}
        </div>

        <Select
          value={marketFilter}
          onChange={(value: string) => setMarketFilter(value)}
          className={styles.marketSelect}
          popupMatchSelectWidth={false}
          options={[
            { value: 'all', label: t('plugins.market.allMarkets') },
            ...markets.map((market) => ({
              value: market.id,
              label: market.enabled
                ? market.name
                : `${market.name}${t('plugins.market.marketDisabledSuffix')}`,
              disabled: !market.enabled,
            })),
          ]}
        />

        <Input
          allowClear
          value={searchInput}
          onChange={(event) => setSearchInput(event.target.value)}
          prefix={<Search size={14} aria-hidden="true" />}
          placeholder={t('plugins.market.searchPlaceholder')}
          className={styles.searchInput}
        />

        <label className={styles.npmToggle}>
          <Switch
            size="small"
            checked={includeNpmSearch}
            onChange={setIncludeNpmSearch}
          />
          <span>{t('plugins.market.includeNpmSearch')}</span>
        </label>

        <button
          type="button"
          className={styles.actionButton}
          onClick={() => setRegistryOpen(true)}
        >
          <Settings2 size={14} aria-hidden="true" />
          <span>{t('plugins.market.manageMarkets')}</span>
        </button>

        <button
          type="button"
          className={styles.actionButton}
          disabled={refreshingMarkets || loading}
          onClick={() => void handleRefreshMarket()}
        >
          <RefreshCw
            size={14}
            className={refreshingMarkets || loading ? styles.spin : undefined}
            aria-hidden="true"
          />
          <span>{t('common.refresh')}</span>
        </button>

        <span className={styles.totalLabel}>
          {t('plugins.market.count', { count: listings.length })}
        </span>
      </div>

      {error ? (
        <div className={styles.errorNotice} role="alert">
          {error}
        </div>
      ) : null}

      {loading && listings.length === 0 ? (
        <div className={styles.loading}>
          <Spin size="small" />
          <span>{t('common.loading')}</span>
        </div>
      ) : listings.length === 0 ? (
        <div className={styles.empty}>
          <span className={styles.emptyIcon}>
            <Store size={20} aria-hidden="true" />
          </span>
          <div className={styles.emptyTitle}>{t('plugins.market.emptyTitle')}</div>
          <p className={styles.emptyDesc}>{t('plugins.market.emptyDesc')}</p>
          <div className={styles.emptyActions}>
            <button
              type="button"
              className={styles.actionButton}
              onClick={() => {
                setSearchInput('');
                setAppliedSearch('');
                setKindFilter('all');
                setMarketFilter('all');
                setIncludeNpmSearch(false);
                setReloadToken((current) => current + 1);
              }}
            >
              <RefreshCw size={14} aria-hidden="true" />
              <span>{t('common.refresh')}</span>
            </button>
          </div>
        </div>
      ) : (
        <div className={styles.grid}>
          {listings.map((listing) => {
            const summary = getPluginSummary(listing.summary, i18n.language)
              || listing.description;
            const marketName = marketNameFor(listing);
            const installing = installingPackage === listing.package;
            const kind = kindBadge(listing.kind);
            return (
              <article key={listing.package} className={styles.card}>
                <header className={styles.cardHeader}>
                  <PluginIcon
                    icon={listing.icon}
                    name={listing.name || listing.package}
                  />
                  <div className={styles.cardHeading}>
                    <span className={styles.name} title={listing.name || listing.package}>
                      {listing.name || listing.package}
                    </span>
                    <div className={styles.metaRow}>
                      <span className={`${styles.kindBadge} ${kind.className}`}>
                        {kind.text}
                      </span>
                      {marketName ? (
                        <span className={styles.marketBadge}>{marketName}</span>
                      ) : null}
                      {listing.latestVersion ? (
                        <span className={styles.version}>v{listing.latestVersion}</span>
                      ) : null}
                      {listing.weeklyDownloads != null ? (
                        <span className={styles.downloads}>
                          {t('plugins.market.weeklyDownloads', {
                            value: formatCompactNumber(listing.weeklyDownloads),
                          })}
                        </span>
                      ) : null}
                    </div>
                  </div>
                </header>

                <p className={styles.summary}>
                  {summary || listing.package}
                </p>

                <div className={styles.providerTags}>
                  {listing.providers.slice(0, 3).map((provider) => (
                    <span key={provider} className={styles.providerTag}>
                      {provider}
                    </span>
                  ))}
                  {listing.providers.length > 3 ? (
                    <span className={styles.providerTagMore}>
                      {t('plugins.market.providersMore', {
                        count: listing.providers.length - 3,
                      })}
                    </span>
                  ) : null}
                </div>

                <footer className={styles.cardFooter}>
                  <span className={styles.packageName} title={listing.package}>
                    {listing.package}
                  </span>
                  {listing.installed ? (
                    <span className={styles.installedButton}>
                      <Check size={13} aria-hidden="true" />
                      {listing.installedVersion
                        ? t('plugins.market.installedVersion', {
                          version: listing.installedVersion,
                        })
                        : t('plugins.market.installed')}
                    </span>
                  ) : (
                    <button
                      type="button"
                      className={styles.installButton}
                      disabled={installingPackage !== null}
                      onClick={() => void handleInstall(listing)}
                    >
                      {installing ? (
                        <Loader2 size={13} className={styles.spin} aria-hidden="true" />
                      ) : (
                        <Download size={13} aria-hidden="true" />
                      )}
                      <span>
                        {installing
                          ? t('plugins.market.installing')
                          : t('plugins.market.install')}
                      </span>
                    </button>
                  )}
                </footer>
              </article>
            );
          })}
        </div>
      )}

      <MarketRegistryModal
        open={registryOpen}
        markets={markets}
        onClose={() => setRegistryOpen(false)}
        onChanged={handleMarketChanged}
      />
    </div>
  );
};

export default PluginMarketView;
