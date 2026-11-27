import React from 'react';
import { App, Input, Select, Spin, Switch, Tooltip } from 'antd';
import {
  Check,
  Combine,
  Download,
  Layers,
  Loader2,
  PanelsTopLeft,
  PlugZap,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
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
  type PluginKind,
  type PluginSettings,
} from '@/services';
import {
  formatPluginError,
  getPluginKindCategory,
  getPluginSummary,
  isChinaMirrorRegistry,
} from '../utils/pluginFormatters';
import PluginIcon from './PluginIcon';
import PluginMarketDetailModal from './PluginMarketDetailModal';
import MarketRegistryModal from './MarketRegistryModal';
import styles from './PluginMarketView.module.less';

type MarketKindFilter = 'all' | 'provider' | 'middleware' | 'page';

interface PluginMarketViewProps {
  refreshKey: number;
  pluginSettings: PluginSettings | null;
  chinaMirrorBusy: boolean;
  onToggleChinaMirror: (enabled: boolean) => void;
  onDataChanged: () => Promise<void>;
  onListingCountChange: (count: number) => void;
}

const PluginMarketView: React.FC<PluginMarketViewProps> = ({
  refreshKey,
  pluginSettings,
  chinaMirrorBusy,
  onToggleChinaMirror,
  onDataChanged,
  onListingCountChange,
}) => {
  const { t, i18n } = useTranslation();
  const { message } = App.useApp();
  const [markets, setMarkets] = React.useState<MarketSourceRecord[]>([]);
  const [listings, setListings] = React.useState<MarketPluginListing[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [refreshing, setRefreshing] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [kindFilter, setKindFilter] = React.useState<MarketKindFilter>('all');
  const [marketFilter, setMarketFilter] = React.useState('all');
  const [searchInput, setSearchInput] = React.useState('');
  const [appliedSearch, setAppliedSearch] = React.useState('');
  const [includeNpmSearch, setIncludeNpmSearch] = React.useState(false);
  const [installingPackage, setInstallingPackage] = React.useState<string | null>(null);
  const [registryOpen, setRegistryOpen] = React.useState(false);
  const [detailPackage, setDetailPackage] = React.useState<string | null>(null);
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

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      await refreshPluginMarkets();
      await Promise.all([loadMarkets(), loadListings()]);
      message.success(t('plugins.market.refreshed'));
    } catch (refreshError) {
      setError(
        t('plugins.market.loadFailed', { error: formatPluginError(refreshError) }),
      );
    } finally {
      setRefreshing(false);
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
    return name ?? (listing.community ? t('plugins.market.communitySource') : '');
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
      return { text: t('plugins.kind.middleware'), category };
    }
    if (category === 'both') {
      return { text: t('plugins.kind.both'), category };
    }
    if (category === 'page') {
      return { text: t('plugins.kind.page'), category };
    }
    if (category === 'unknown') {
      return { text: t('plugins.kind.unknown'), category };
    }
    return { text: t('plugins.kind.provider'), category };
  };

  const kindIcon = (category: PluginKind) => {
    if (category === 'middleware') {
      return <Layers size={12} aria-hidden="true" />;
    }
    if (category === 'both') {
      return <Combine size={12} aria-hidden="true" />;
    }
    if (category === 'page') {
      return <PanelsTopLeft size={12} aria-hidden="true" />;
    }
    return <PlugZap size={12} aria-hidden="true" />;
  };

  const renderInstallControl = (listing: MarketPluginListing) => {
    const installing = installingPackage === listing.package;
    if (listing.installed) {
      return (
        <span className={styles.installedLabel}>
          <Check size={13} aria-hidden="true" />
          {listing.installedVersion
            ? t('plugins.market.installedVersion', { version: listing.installedVersion })
            : t('plugins.market.installed')}
        </span>
      );
    }
    return (
      <button
        type="button"
        className={styles.installPill}
        disabled={installingPackage !== null}
        onClick={(event) => {
          event.stopPropagation();
          void handleInstall(listing);
        }}
      >
        {installing ? (
          <Loader2 size={13} className={styles.spin} aria-hidden="true" />
        ) : (
          <Download size={13} aria-hidden="true" />
        )}
        <span>
          {installing ? t('plugins.market.installing') : t('plugins.market.install')}
        </span>
      </button>
    );
  };

  const detailListing = detailPackage
    ? listings.find((listing) => listing.package === detailPackage) ?? null
    : null;

  return (
    <div className={styles.view}>
      <div className={styles.toolbar}>
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

        <label className={styles.mirrorToggle}>
          <span>{t('plugins.market.chinaMirror')}</span>
          <Switch
            size="small"
            checked={isChinaMirrorRegistry(pluginSettings?.npmRegistry)}
            loading={chinaMirrorBusy}
            disabled={chinaMirrorBusy || !pluginSettings}
            aria-label={t('plugins.market.chinaMirror')}
            onChange={onToggleChinaMirror}
          />
        </label>

        <button
          type="button"
          className={styles.actionButton}
          onClick={() => setRegistryOpen(true)}
        >
          <Settings2 size={14} aria-hidden="true" />
          <span>{t('plugins.market.manageMarkets')}</span>
        </button>

        <Tooltip title={t('plugins.market.refresh')}>
          <button
            type="button"
            className={styles.iconButton}
            aria-label={t('plugins.market.refresh')}
            disabled={refreshing}
            onClick={() => void handleRefresh()}
          >
            <RefreshCw
              size={15}
              className={refreshing ? styles.spin : undefined}
              aria-hidden="true"
            />
          </button>
        </Tooltip>

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
            const kind = kindBadge(listing.kind);
            return (
              <article
                key={listing.package}
                className={styles.card}
                role="button"
                tabIndex={0}
                aria-label={listing.name || listing.package}
                onClick={() => setDetailPackage(listing.package)}
                onKeyDown={(event) => {
                  if (event.target !== event.currentTarget) {
                    return;
                  }
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    setDetailPackage(listing.package);
                  }
                }}
              >
                <header className={styles.cardTop}>
                  <span className={styles.cardIcon}>
                    <PluginIcon
                      icon={listing.icon}
                      name={listing.name || listing.package}
                    />
                  </span>
                  <div className={styles.cardHeading}>
                    <span className={styles.name} title={listing.name || listing.package}>
                      {listing.name || listing.package}
                    </span>
                    <span className={styles.sourceRow}>
                      {listing.community ? (
                        <>
                          <ShieldCheck
                            size={12}
                            className={styles.communityIcon}
                            aria-hidden="true"
                          />
                          <span className={styles.communityText}>
                            {t('plugins.market.communitySource')}
                          </span>
                        </>
                      ) : (
                        <span className={styles.sourceText}>{marketName}</span>
                      )}
                    </span>
                  </div>
                  <div
                    className={styles.cardAction}
                    onClick={(event) => event.stopPropagation()}
                  >
                    {renderInstallControl(listing)}
                  </div>
                </header>

                <p className={styles.summary}>
                  {summary || listing.package}
                </p>

                <footer className={styles.cardFooter}>
                  <span className={styles.kindLine}>
                    {kindIcon(kind.category)}
                    <span>{kind.text}</span>
                  </span>
                  {listing.latestVersion ? (
                    <span className={styles.version}>v{listing.latestVersion}</span>
                  ) : null}
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
      <PluginMarketDetailModal
        open={Boolean(detailListing)}
        listing={detailListing}
        marketName={detailListing ? marketNameFor(detailListing) : ''}
        installing={installingPackage === detailListing?.package}
        onInstall={() => {
          if (detailListing) {
            void handleInstall(detailListing);
          }
        }}
        onClose={() => setDetailPackage(null)}
      />
    </div>
  );
};

export default PluginMarketView;
