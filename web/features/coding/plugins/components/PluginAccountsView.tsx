import React from 'react';
import { App, Input, Switch, Tooltip, Spin } from 'antd';
import {
  CalendarCheck,
  CircleAlert,
  Gauge,
  LogIn,
  LogOut,
  Loader2,
  PlugZap,
  RefreshCw,
  Search,
  Send,
  Store,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import {
  checkPluginAccount,
  checkinPluginAccount,
  getPluginAccountUsage,
  listPluginAccounts,
  listPluginProviders,
  refreshPluginProviderModels,
  setPluginAccountAutoCheckin,
  setPluginAccountEnabled,
  setPluginProviderOption,
  signOutPluginAccount,
  type InstalledPluginRecord,
  type PluginAccountSummary,
  type PluginCheckinResult,
  type PluginProviderOptionDeclaration,
  type PluginProviderRow,
  type PluginUsage,
} from '@/services';
import {
  buildPluginQuotaRows,
  formatContextWindow,
  formatDateTime,
  formatPluginError,
  getCountdownParts,
  getPluginInitials,
  getProviderHealth,
  getProtocolLabel,
  pickLocalizedText,
  type PluginQuotaRow,
} from '../utils/pluginFormatters';
import AddPluginToCliModal from './AddPluginToCliModal';
import PluginIcon from './PluginIcon';
import PluginLoginModal from './PluginLoginModal';
import styles from './PluginAccountsView.module.less';

interface PluginAccountsViewProps {
  active: boolean;
  refreshKey: number;
  installedPlugins: InstalledPluginRecord[];
  onDataChanged: () => Promise<void>;
  onBrowseMarket: () => void;
  onOpenRuntime: () => void;
}

const PluginAccountsView: React.FC<PluginAccountsViewProps> = ({
  active,
  refreshKey,
  installedPlugins,
  onDataChanged,
  onBrowseMarket,
  onOpenRuntime,
}) => {
  const { t, i18n } = useTranslation();
  const { message, modal } = App.useApp();
  const [searchParams, setSearchParams] = useSearchParams();

  const [providers, setProviders] = React.useState<PluginProviderRow[]>([]);
  const [accounts, setAccounts] = React.useState<PluginAccountSummary[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [reloadToken, setReloadToken] = React.useState(0);
  const [modelsRefreshing, setModelsRefreshing] = React.useState(false);
  const [usageByAccount, setUsageByAccount] = React.useState<Record<string, PluginUsage>>({});
  const [usageErrors, setUsageErrors] = React.useState<Record<string, string>>({});
  const [usageLoadingAccounts, setUsageLoadingAccounts] = React.useState<Record<string, boolean>>({});
  const [usageBusy, setUsageBusy] = React.useState(false);
  const [checkinResults, setCheckinResults] = React.useState<Record<string, PluginCheckinResult>>({});
  const [checkinBusy, setCheckinBusy] = React.useState(false);
  const [checkBusyAccount, setCheckBusyAccount] = React.useState<string | null>(null);
  const [accountBusyKey, setAccountBusyKey] = React.useState<string | null>(null);
  const [autoCheckinBusyKey, setAutoCheckinBusyKey] = React.useState<string | null>(null);
  const [loginProvider, setLoginProvider] = React.useState<PluginProviderRow | null>(null);
  const [addToCliOpen, setAddToCliOpen] = React.useState(false);
  const [providerOptionValues, setProviderOptionValues] = React.useState<
    Record<string, Record<string, boolean>>
  >({});
  const [modelSearch, setModelSearch] = React.useState('');
  const [providerSearch, setProviderSearch] = React.useState('');
  const autoFetchedUsageRef = React.useRef(new Set<string>());
  const lastRefreshKeyRef = React.useRef(refreshKey);

  React.useEffect(() => {
    if (lastRefreshKeyRef.current !== refreshKey) {
      lastRefreshKeyRef.current = refreshKey;
      autoFetchedUsageRef.current.clear();
    }
  }, [refreshKey]);

  React.useEffect(() => {
    let disposed = false;
    const load = async () => {
      setLoading(true);
      const [providersResult, accountsResult] = await Promise.allSettled([
        listPluginProviders(),
        listPluginAccounts(),
      ]);
      if (disposed) return;
      if (providersResult.status === 'fulfilled') {
        setProviders(providersResult.value);
        setLoadError(null);
      } else {
        setLoadError(formatPluginError(providersResult.reason));
      }
      if (accountsResult.status === 'fulfilled') {
        const nextAccounts = accountsResult.value;
        setAccounts(nextAccounts);
        setUsageByAccount((current) => {
          let changed = false;
          const next = { ...current };
          for (const account of nextAccounts) {
            if (account.quota && !next[account.account]) {
              next[account.account] = account.quota;
              changed = true;
            }
          }
          return changed ? next : current;
        });
      }
      setLoading(false);
    };
    void load();
    return () => {
      disposed = true;
    };
  }, [refreshKey, reloadToken]);

  React.useEffect(() => {
    setProviderOptionValues((current) => {
      let changed = false;
      const next = { ...current };
      for (const provider of providers) {
        if (!next[provider.id]) {
          // The backend now returns the persisted switch values on every row;
          // declarations only fill the keys this provider has never saved.
          const declaredDefaults = Object.fromEntries(
            provider.options.map((option) => [option.key, option.default]),
          );
          next[provider.id] = {
            ...declaredDefaults,
            ...(provider.optionValues ?? {}),
          };
          changed = true;
        }
      }
      return changed ? next : current;
    });
  }, [providers]);

  const requestedProviderId = searchParams.get('provider');
  const selectedProvider = providers.find((provider) => provider.id === requestedProviderId)
    ?? providers[0]
    ?? null;

  const filteredProviders = React.useMemo(() => {
    const needle = providerSearch.trim().toLowerCase();
    if (!needle) {
      return providers;
    }
    return providers.filter((provider) =>
      `${provider.name} ${provider.id} ${provider.spec} ${provider.npm}`
        .toLowerCase()
        .includes(needle),
    );
  }, [providerSearch, providers]);

  React.useEffect(() => {
    setModelSearch('');
  }, [selectedProvider?.id]);

  const providerAccounts = React.useMemo(() => {
    if (!selectedProvider) return [];
    return accounts.filter((account) => account.provider === selectedProvider.id);
  }, [accounts, selectedProvider]);

  const quotaRowsByAccount = React.useMemo(() => {
    const next: Record<string, PluginQuotaRow[]> = {};
    for (const account of providerAccounts) {
      next[account.account] = buildPluginQuotaRows(
        usageByAccount[account.account],
        i18n.language,
      );
    }
    return next;
  }, [i18n.language, providerAccounts, usageByAccount]);

  const filteredModels = React.useMemo(() => {
    const models = selectedProvider?.models ?? [];
    const needle = modelSearch.trim().toLowerCase();
    if (!needle) return models;
    return models.filter((model) =>
      `${model.id} ${model.name}`.toLowerCase().includes(needle),
    );
  }, [modelSearch, selectedProvider?.models]);

  const accountCountFor = (providerId: string) =>
    accounts.filter((account) => account.provider === providerId).length;

  const getAccountLabel = (account: PluginAccountSummary) =>
    account.displayName || account.account;

  const selectProvider = (providerId: string) => {
    const next = new URLSearchParams(searchParams);
    next.set('provider', providerId);
    setSearchParams(next, { replace: true });
  };

  const reload = () => setReloadToken((current) => current + 1);

  const handleRefreshAll = () => {
    if (selectedProvider) {
      const prefix = `${selectedProvider.id}:`;
      for (const key of Array.from(autoFetchedUsageRef.current)) {
        if (key.startsWith(prefix)) {
          autoFetchedUsageRef.current.delete(key);
        }
      }
    }
    reload();
  };

  const formatCountdown = (iso: string) => {
    const parts = getCountdownParts(iso);
    if (!parts) return '';
    if (parts.unit === 'day') {
      return t('plugins.units.day', { value: parts.value });
    }
    if (parts.unit === 'hour') {
      return t('plugins.units.hour', { value: parts.value });
    }
    return t('plugins.units.minute', { value: parts.value });
  };

  const refreshUsage = React.useCallback(async (accountKeys?: string[], silent = false) => {
    if (!selectedProvider) return;
    const providerId = selectedProvider.id;
    const keys = accountKeys
      ?? providerAccounts.filter((account) => account.enabled).map((account) => account.account);
    if (keys.length === 0) return;
    setUsageBusy(true);
    setUsageLoadingAccounts((current) => {
      const next = { ...current };
      for (const accountKey of keys) {
        next[accountKey] = true;
      }
      return next;
    });
    const results = await Promise.all(keys.map(async (accountKey) => {
      try {
        return {
          accountKey,
          usage: await getPluginAccountUsage(providerId, accountKey),
          error: '',
        };
      } catch (error) {
        return { accountKey, usage: null, error: formatPluginError(error) };
      }
    }));
    setUsageByAccount((current) => {
      const next = { ...current };
      for (const result of results) {
        if (result.usage) next[result.accountKey] = result.usage;
      }
      return next;
    });
    setUsageErrors((current) => {
      const next = { ...current };
      for (const result of results) {
        if (result.error) next[result.accountKey] = result.error;
        else delete next[result.accountKey];
      }
      return next;
    });
    setUsageLoadingAccounts((current) => {
      const next = { ...current };
      for (const accountKey of keys) {
        delete next[accountKey];
      }
      return next;
    });
    setUsageBusy(false);
    if (!silent) {
      const failed = results.filter((result) => result.error).length;
      if (failed > 0) {
        message.warning(t('plugins.accounts.usagePartialFailed', { count: failed }));
      } else {
        message.success(t('plugins.accounts.usageRefreshed'));
      }
    }
  }, [message, providerAccounts, selectedProvider, t]);

  // Automatically fetch live usage once per enabled account when visiting a
  // usage-capable channel, while cached snapshots already paint immediately.
  React.useEffect(() => {
    if (!active || loading || !selectedProvider?.usage) {
      return;
    }
    const pendingKeys = providerAccounts
      .filter((account) => {
        if (!account.enabled) return false;
        return !autoFetchedUsageRef.current.has(`${selectedProvider.id}:${account.account}`);
      })
      .map((account) => account.account);
    if (pendingKeys.length === 0) {
      return;
    }
    for (const accountKey of pendingKeys) {
      autoFetchedUsageRef.current.add(`${selectedProvider.id}:${accountKey}`);
    }
    void refreshUsage(pendingKeys, true);
  }, [active, loading, providerAccounts, refreshUsage, selectedProvider]);

  const runCheckin = async (accountKeys?: string[]) => {
    if (!selectedProvider) return;
    const keys = accountKeys
      ?? providerAccounts.filter((account) => account.enabled).map((account) => account.account);
    if (keys.length === 0) return;
    setCheckinBusy(true);
    const results = await Promise.all(keys.map(async (accountKey) => {
      try {
        return {
          accountKey,
          result: await checkinPluginAccount(selectedProvider.id, accountKey),
        };
      } catch (error) {
        return {
          accountKey,
          result: {
            outcome: 'failed',
            credit: 0,
            streak: 0,
            message: formatPluginError(error),
            until: '',
          } satisfies PluginCheckinResult,
        };
      }
    }));
    setCheckinResults((current) => {
      const next = { ...current };
      for (const result of results) {
        next[result.accountKey] = result.result;
      }
      return next;
    });
    setCheckinBusy(false);
    const claimed = results.filter((result) => result.result.outcome === 'claimed').length;
    const done = results.filter((result) => result.result.outcome === 'done').length;
    const failed = results.filter((result) =>
      result.result.outcome === 'failed' || result.result.outcome === 'captcha',
    ).length;
    message.success(t('plugins.accounts.checkinSummary', { claimed, done, failed }));
  };

  const handleCheck = async (account: PluginAccountSummary) => {
    if (!selectedProvider) return;
    setCheckBusyAccount(account.account);
    try {
      const result = await checkPluginAccount(selectedProvider.id, account.account);
      if (result.usage) {
        setUsageByAccount((current) => ({ ...current, [account.account]: result.usage as PluginUsage }));
      }
      if (result.refused) {
        message.warning(t('plugins.accounts.checkRefused', { reason: result.refused }));
      } else {
        message.success(t('plugins.accounts.checkOk'));
      }
    } catch (error) {
      message.error(
        t('plugins.accounts.checkFailed', { error: formatPluginError(error) }),
      );
    } finally {
      setCheckBusyAccount(null);
    }
  };

  const handleSignOut = (account: PluginAccountSummary) => {
    if (!selectedProvider) return;
    modal.confirm({
      title: t('plugins.accounts.signOutConfirmTitle'),
      content: t('plugins.accounts.signOutConfirmContent', {
        account: getAccountLabel(account),
      }),
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      okButtonProps: { danger: true },
      onOk: async () => {
        try {
          await signOutPluginAccount(selectedProvider.id, account.account);
          message.success(t('plugins.accounts.signOutSuccess'));
          reload();
          await onDataChanged();
        } catch (error) {
          message.error(
            t('plugins.accounts.actionFailed', { error: formatPluginError(error) }),
          );
        }
      },
    });
  };

  const handleAccountToggle = async (account: PluginAccountSummary, enabled: boolean) => {
    if (!selectedProvider) return;
    setAccountBusyKey(account.account);
    const plugin = installedPlugins.find((entry) =>
      entry.providers.includes(selectedProvider.id),
    );
    try {
      await setPluginAccountEnabled({
        account: account.account,
        pluginId: plugin?.id ?? selectedProvider.id,
        provider: selectedProvider.id,
        displayName: account.displayName || null,
        enabled,
      });
      setAccounts((current) =>
        current.map((entry) =>
          entry.account === account.account ? { ...entry, enabled } : entry,
        ),
      );
      message.success(t('plugins.accounts.toggleAccountSuccess'));
    } catch (error) {
      message.error(
        t('plugins.accounts.actionFailed', { error: formatPluginError(error) }),
      );
    } finally {
      setAccountBusyKey(null);
    }
  };

  const handleAutoCheckinToggle = async (
    account: PluginAccountSummary,
    enabled: boolean,
  ) => {
    if (!selectedProvider) return;
    setAutoCheckinBusyKey(account.account);
    const plugin = installedPlugins.find((entry) =>
      entry.providers.includes(selectedProvider.id),
    );
    try {
      await setPluginAccountAutoCheckin({
        account: account.account,
        pluginId: plugin?.id ?? selectedProvider.id,
        provider: selectedProvider.id,
        enabled,
      });
      setAccounts((current) =>
        current.map((entry) =>
          entry.account === account.account ? { ...entry, autoCheckin: enabled } : entry,
        ),
      );
      message.success(t('plugins.accounts.autoCheckinSuccess'));
    } catch (error) {
      message.error(
        t('plugins.accounts.autoCheckinFailed', { error: formatPluginError(error) }),
      );
    } finally {
      setAutoCheckinBusyKey(null);
    }
  };

  const handleRefreshModels = async () => {
    setModelsRefreshing(true);
    try {
      const nextProviders = await refreshPluginProviderModels();
      setProviders(nextProviders);
      message.success(t('plugins.accounts.modelsRefreshed'));
    } catch (error) {
      message.error(
        t('plugins.accounts.modelsRefreshFailed', { error: formatPluginError(error) }),
      );
    } finally {
      setModelsRefreshing(false);
    }
  };

  const applyProviderOption = async (
    option: PluginProviderOptionDeclaration,
    checked: boolean,
  ) => {
    if (!selectedProvider) return;
    const current = providerOptionValues[selectedProvider.id]
      ?? Object.fromEntries(selectedProvider.options.map((entry) => [entry.key, entry.default]));
    const next = { ...current, [option.key]: checked };
    setProviderOptionValues((values) => ({
      ...values,
      [selectedProvider.id]: next,
    }));
    try {
      await setPluginProviderOption(selectedProvider.id, next);
    } catch (error) {
      setProviderOptionValues((values) => ({
        ...values,
        [selectedProvider.id]: current,
      }));
      message.error(
        t('plugins.accounts.optionSaveFailed', { error: formatPluginError(error) }),
      );
    }
  };

  const handleProviderOptionToggle = (
    option: PluginProviderOptionDeclaration,
    checked: boolean,
  ) => {
    const confirmText = pickLocalizedText(option.confirms, option.confirm, i18n.language);
    const label = pickLocalizedText(option.labels, option.label, i18n.language);
    if (!confirmText) {
      void applyProviderOption(option, checked);
      return;
    }
    modal.confirm({
      title: t('plugins.accounts.optionConfirmTitle', { label }),
      content: confirmText,
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      onOk: () => applyProviderOption(option, checked),
    });
  };

  const checkinStatusText = (result: PluginCheckinResult) => {
    if (result.outcome === 'claimed') {
      return t('plugins.accounts.checkinClaimed', {
        credit: result.credit,
        streak: result.streak,
      });
    }
    if (result.outcome === 'done') {
      return t('plugins.accounts.checkinDone');
    }
    if (result.outcome === 'ineligible') {
      return t('plugins.accounts.checkinIneligible');
    }
    if (result.outcome === 'inactive') {
      return t('plugins.accounts.checkinInactive');
    }
    if (result.outcome === 'captcha') {
      return t('plugins.accounts.checkinCaptcha');
    }
    return t('plugins.accounts.checkinFailed');
  };

  const checkinStatusClass = (result: PluginCheckinResult) => {
    if (result.outcome === 'claimed' || result.outcome === 'done') {
      return styles.checkinSuccess;
    }
    if (result.outcome === 'failed' || result.outcome === 'captcha') {
      return styles.checkinError;
    }
    return styles.checkinWarning;
  };

  const checkinOutcomeLabel = (outcome: string | null | undefined) => {
    if (outcome === 'claimed') return t('plugins.accounts.checkinOutcomeClaimed');
    if (outcome === 'done') return t('plugins.accounts.checkinOutcomeDone');
    if (outcome === 'ineligible') return t('plugins.accounts.checkinOutcomeIneligible');
    if (outcome === 'inactive') return t('plugins.accounts.checkinOutcomeInactive');
    if (outcome === 'captcha') return t('plugins.accounts.checkinOutcomeCaptcha');
    if (outcome === 'failed') return t('plugins.accounts.checkinOutcomeFailed');
    return t('plugins.accounts.checkinOutcomeUnknown');
  };

  const bunUnavailable = Boolean(loadError?.includes('PLUGIN_BUN_UNAVAILABLE'));

  if (loading && providers.length === 0) {
    return (
      <div className={styles.loading}>
        <Spin size="small" />
        <span>{t('common.loading')}</span>
      </div>
    );
  }

  if (providers.length === 0) {
    return (
      <div className={styles.empty}>
        <span className={styles.emptyIcon}>
          <PlugZap size={20} aria-hidden="true" />
        </span>
        <div className={styles.emptyTitle}>
          {bunUnavailable
            ? t('plugins.accounts.bunRequiredTitle')
            : loadError
              ? t('plugins.accounts.loadFailedTitle')
              : t('plugins.accounts.emptyProviders')}
        </div>
        <p className={styles.emptyDesc}>
          {bunUnavailable
            ? t('plugins.accounts.bunRequiredDesc')
            : loadError ?? t('plugins.accounts.emptyProvidersDesc')}
        </p>
        <div className={styles.emptyActions}>
          <button
            type="button"
            className={styles.actionButton}
            disabled={loading}
            onClick={handleRefreshAll}
          >
            <RefreshCw
              size={15}
              className={loading ? styles.spin : undefined}
              aria-hidden="true"
            />
            <span>{t('common.refresh')}</span>
          </button>
          {loadError ? (
            <button type="button" className={styles.actionButton} onClick={onOpenRuntime}>
              {t('plugins.accounts.goRuntime')}
            </button>
          ) : null}
          <button
            type="button"
            className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
            onClick={onBrowseMarket}
          >
            <Store size={15} aria-hidden="true" />
            <span>{t('plugins.accounts.browseMarket')}</span>
          </button>
        </div>
      </div>
    );
  }

  const providerName = selectedProvider?.name || selectedProvider?.id || '';
  const providerSpec = selectedProvider?.spec || selectedProvider?.npm || '';

  return (
    <div className={styles.view}>
      <div className={styles.providerPicker}>
        <div className={styles.pickerHeader}>
          <span className={styles.pickerTitle}>
            {t('plugins.accounts.providersTitle')}
          </span>
          <Input
            allowClear
            value={providerSearch}
            onChange={(event) => setProviderSearch(event.target.value)}
            prefix={<Search size={13} aria-hidden="true" />}
            placeholder={t('plugins.accounts.providerSearch')}
            className={styles.providerSearch}
          />
          <button
            type="button"
            className={styles.actionButton}
            disabled={loading}
            onClick={handleRefreshAll}
          >
            <RefreshCw
              size={14}
              className={loading ? styles.spin : undefined}
              aria-hidden="true"
            />
            <span>{t('common.refresh')}</span>
          </button>
        </div>
        <div className={styles.providerChips}>
          {filteredProviders.length === 0 ? (
            <span className={styles.chipsEmpty}>
              {t('plugins.accounts.providerSearchEmpty')}
            </span>
          ) : null}
          {filteredProviders.map((provider) => {
            const health = getProviderHealth(provider);
            const accountCount = accountCountFor(provider.id);
            return (
              <button
                key={provider.id}
                type="button"
                className={`${styles.providerChip} ${selectedProvider?.id === provider.id ? styles.providerChipActive : ''}`}
                onClick={() => selectProvider(provider.id)}
              >
                <PluginIcon
                  icon={provider.icon}
                  name={provider.name || provider.id}
                  size={22}
                />
                <span className={styles.chipName}>{provider.name || provider.id}</span>
                <span
                  className={`${styles.healthDot} ${
                    health === 'healthy'
                      ? styles.healthOk
                      : health === 'warning'
                        ? styles.healthWarn
                        : styles.healthMuted
                  }`}
                  aria-hidden="true"
                />
                <span className={styles.chipCount}>{accountCount}</span>
              </button>
            );
          })}
        </div>
      </div>

      <div className={styles.detail}>
        {loadError ? (
          <div className={styles.errorNotice}>
            <CircleAlert size={15} aria-hidden="true" />
            <div className={styles.errorNoticeBody}>
              <span className={styles.errorNoticeTitle}>
                {bunUnavailable
                  ? t('plugins.accounts.bunRequiredTitle')
                  : t('plugins.accounts.loadFailedTitle')}
              </span>
              <span className={styles.errorNoticeDesc}>
                {bunUnavailable ? t('plugins.accounts.bunRequiredDesc') : loadError}
              </span>
            </div>
            <button type="button" className={styles.textButton} onClick={onOpenRuntime}>
              {t('plugins.accounts.goRuntime')}
            </button>
          </div>
        ) : null}

        {selectedProvider ? (
          <>
            <div className={styles.detailHeader}>
              <PluginIcon
                icon={selectedProvider.icon}
                name={providerName}
                size={36}
              />
              <div className={styles.detailHeading}>
                <h2 className={styles.detailTitle}>{providerName}</h2>
                <div className={styles.detailMeta}>
                  <span className={styles.pillProvider}>
                    {t('plugins.kind.provider')}
                  </span>
                  <span className={styles.detailSpec}>{providerSpec}</span>
                  <span>
                    {t('plugins.accounts.signedInCount', { count: providerAccounts.length })}
                  </span>
                </div>
              </div>
              <div className={styles.detailActions}>
                <button
                  type="button"
                  className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
                  onClick={() => setLoginProvider(selectedProvider)}
                >
                  <LogIn size={15} aria-hidden="true" />
                  <span>{t('plugins.accounts.login')}</span>
                </button>
                <button
                  type="button"
                  className={styles.actionButton}
                  disabled={providerAccounts.length === 0}
                  title={
                    providerAccounts.length === 0
                      ? t('plugins.addToCli.needAccount')
                      : undefined
                  }
                  onClick={() => setAddToCliOpen(true)}
                >
                  <Send size={15} aria-hidden="true" />
                  <span>{t('plugins.addToCli.button')}</span>
                </button>
                <button
                  type="button"
                  className={styles.actionButton}
                  disabled={modelsRefreshing}
                  onClick={() => void handleRefreshModels()}
                >
                  <RefreshCw
                    size={15}
                    className={modelsRefreshing ? styles.spin : undefined}
                    aria-hidden="true"
                  />
                  <span>{t('plugins.accounts.refreshModels')}</span>
                </button>
                {selectedProvider.usage && providerAccounts.length > 0 ? (
                  <button
                    type="button"
                    className={styles.actionButton}
                    disabled={usageBusy}
                    onClick={() => void refreshUsage()}
                  >
                    <Gauge size={15} aria-hidden="true" />
                    <span>{t('plugins.accounts.refreshUsageAll')}</span>
                  </button>
                ) : null}
                {selectedProvider.checkin && providerAccounts.length > 0 ? (
                  <button
                    type="button"
                    className={styles.actionButton}
                    disabled={checkinBusy}
                    onClick={() => void runCheckin()}
                  >
                    <CalendarCheck size={15} aria-hidden="true" />
                    <span>{t('plugins.accounts.checkinAll')}</span>
                  </button>
                ) : null}
              </div>
            </div>

            <section className={styles.section}>
              <div className={styles.sectionHeader}>
                <h3>{t('plugins.accounts.accountsTitle')}</h3>
                <span className={styles.sectionHint}>
                  {t('plugins.accounts.accountsHint')}
                </span>
              </div>

              {providerAccounts.length === 0 ? (
                <div className={styles.inlineEmpty}>
                  <span>{t('plugins.accounts.noAccountsDesc')}</span>
                  <button
                    type="button"
                    className={styles.textButton}
                    onClick={() => setLoginProvider(selectedProvider)}
                  >
                    <LogIn size={14} aria-hidden="true" />
                    <span>{t('plugins.accounts.login')}</span>
                  </button>
                </div>
              ) : (
                <div className={styles.accountList}>
                  {providerAccounts.map((account) => {
                    const label = getAccountLabel(account);
                    const usage = usageByAccount[account.account];
                    const usageError = usageErrors[account.account];
                    const quotaRows = quotaRowsByAccount[account.account] ?? [];
                    const checkinResult = checkinResults[account.account];
                    const expiry = account.expiresAt
                      ? new Date(account.expiresAt).toISOString()
                      : '';
                    const expired = Boolean(
                      account.expiresAt && account.expiresAt < Date.now(),
                    );
                    const accountBusy = accountBusyKey === account.account;
                    const lastCheckinTime = account.lastCheckinAt
                      ? formatDateTime(account.lastCheckinAt, i18n.language)
                      : '';

                    return (
                      <article key={account.account} className={styles.accountCard}>
                        <div className={styles.accountTop}>
                          <span className={styles.accountAvatar} aria-hidden="true">
                            {getPluginInitials(label)}
                          </span>
                          <div className={styles.accountIdentity}>
                            <span className={styles.accountName} title={label}>
                              {label}
                            </span>
                            <span className={styles.accountMeta}>
                              <span className={styles.accountType}>
                                {account.authType === 'oauth'
                                  ? t('plugins.accounts.accountTypeOauth')
                                  : t('plugins.accounts.accountTypeApi')}
                              </span>
                              {account.account !== label ? (
                                <span className={styles.accountKey}>{account.account}</span>
                              ) : null}
                              {expiry ? (
                                <span className={expired ? styles.expiryExpired : undefined}>
                                  {expired
                                    ? t('plugins.accounts.expired')
                                    : t('plugins.accounts.expiresAt', {
                                      time: formatDateTime(expiry, i18n.language),
                                    })}
                                </span>
                              ) : null}
                              {account.lastCheckinOutcome || account.lastCheckinAt ? (
                                <span
                                  className={styles.accountCheckinMeta}
                                  title={account.lastCheckinMessage ?? undefined}
                                >
                                  {lastCheckinTime
                                    ? t('plugins.accounts.lastCheckin', {
                                      outcome: checkinOutcomeLabel(account.lastCheckinOutcome),
                                      time: lastCheckinTime,
                                    })
                                    : t('plugins.accounts.lastCheckinNoTime', {
                                      outcome: checkinOutcomeLabel(account.lastCheckinOutcome),
                                    })}
                                </span>
                              ) : null}
                            </span>
                          </div>
                          <div className={styles.accountControls}>
                            <div className={styles.switchGroup}>
                              <Tooltip title={t('plugins.accounts.enabledHint')}>
                                <label
                                  className={`${styles.switchControl} ${
                                    accountBusy ? styles.switchControlDisabled : ''
                                  }`}
                                >
                                  <span className={styles.switchLabel}>
                                    {t('plugins.accounts.enabledLabel')}
                                  </span>
                                  <Switch
                                    size="small"
                                    checked={account.enabled}
                                    loading={accountBusy}
                                    disabled={accountBusy}
                                    aria-label={t('plugins.accounts.enabledToggle')}
                                    onChange={(checked) =>
                                      void handleAccountToggle(account, checked)}
                                  />
                                </label>
                              </Tooltip>
                              {selectedProvider.checkin ? (
                                <Tooltip
                                  title={
                                    account.enabled
                                      ? t('plugins.accounts.autoCheckinHint')
                                      : t('plugins.accounts.autoCheckinDisabled')
                                  }
                                >
                                  <label
                                    className={`${styles.switchControl} ${
                                      !account.enabled || autoCheckinBusyKey === account.account
                                        ? styles.switchControlDisabled
                                        : ''
                                    }`}
                                  >
                                    <span className={styles.switchLabel}>
                                      {t('plugins.accounts.autoCheckin')}
                                    </span>
                                    <Switch
                                      size="small"
                                      checked={Boolean(account.autoCheckin)}
                                      loading={autoCheckinBusyKey === account.account}
                                      disabled={
                                        !account.enabled
                                        || autoCheckinBusyKey === account.account
                                      }
                                      aria-label={t('plugins.accounts.autoCheckin')}
                                      onChange={(checked) =>
                                        void handleAutoCheckinToggle(account, checked)}
                                    />
                                  </label>
                                </Tooltip>
                              ) : null}
                            </div>
                            <span className={styles.controlsDivider} aria-hidden="true" />
                            <div className={styles.iconActions}>
                              <Tooltip title={t('plugins.accounts.check')}>
                                <button
                                  type="button"
                                  className={styles.iconButton}
                                  aria-label={t('plugins.accounts.check')}
                                  disabled={checkBusyAccount === account.account}
                                  onClick={() => void handleCheck(account)}
                                >
                                  {checkBusyAccount === account.account ? (
                                    <Loader2 size={14} className={styles.spin} aria-hidden="true" />
                                  ) : (
                                    <PlugZap size={14} aria-hidden="true" />
                                  )}
                                </button>
                              </Tooltip>
                              {selectedProvider.usage ? (
                                <Tooltip title={t('plugins.accounts.refreshUsage')}>
                                  <button
                                    type="button"
                                    className={styles.iconButton}
                                    aria-label={t('plugins.accounts.refreshUsage')}
                                    disabled={usageBusy}
                                    onClick={() => void refreshUsage([account.account], true)}
                                  >
                                    <RefreshCw size={14} aria-hidden="true" />
                                  </button>
                                </Tooltip>
                              ) : null}
                              {selectedProvider.checkin ? (
                                <Tooltip title={t('plugins.accounts.checkin')}>
                                  <button
                                    type="button"
                                    className={styles.iconButton}
                                    aria-label={t('plugins.accounts.checkin')}
                                    disabled={checkinBusy}
                                    onClick={() => void runCheckin([account.account])}
                                  >
                                    <CalendarCheck size={14} aria-hidden="true" />
                                  </button>
                                </Tooltip>
                              ) : null}
                              <Tooltip title={t('plugins.accounts.signOut')}>
                                <button
                                  type="button"
                                  className={`${styles.iconButton} ${styles.iconButtonDanger}`}
                                  aria-label={t('plugins.accounts.signOut')}
                                  onClick={() => handleSignOut(account)}
                                >
                                  <LogOut size={14} aria-hidden="true" />
                                </button>
                              </Tooltip>
                            </div>
                          </div>
                        </div>

                        <div className={styles.quotaArea}>
                          {usage ? (
                            <>
                              <div className={styles.usageMetaRow}>
                                {usage.plan ? (
                                  <span className={styles.planPill}>{usage.plan}</span>
                                ) : null}
                                {usage.user ? (
                                  <span className={styles.usageUser}>{usage.user}</span>
                                ) : null}
                                {usage.balance ? (
                                  <span className={styles.walletInline}>
                                    {t('plugins.accounts.walletBalance', {
                                      balance: usage.balance,
                                    })}
                                  </span>
                                ) : null}
                                {usage.until ? (
                                  <span className={styles.usageHint}>
                                    {t('plugins.accounts.usageUntil', {
                                      time: formatDateTime(usage.until, i18n.language),
                                    })}
                                  </span>
                                ) : null}
                                {usage.signIn === 'expired' ? (
                                  <span className={styles.usageWarning}>
                                    {t('plugins.accounts.signInExpired')}
                                  </span>
                                ) : null}
                              </div>
                              {usage.error ? (
                                <div className={styles.inlineError}>
                                  {t('plugins.accounts.usageError', { error: usage.error })}
                                </div>
                              ) : null}
                              {quotaRows.length > 0 ? (
                                <div className={styles.quotaList}>
                                  {quotaRows.map((row) => (
                                    <div key={row.key} className={styles.quotaRow}>
                                      <div className={styles.quotaHeader}>
                                        <span className={styles.quotaName}>
                                          {row.name || t('plugins.accounts.quotaWindow')}
                                          {row.credits ? (
                                            <span className={styles.creditsTag}>
                                              {t('plugins.accounts.creditsTag')}
                                            </span>
                                          ) : null}
                                          {row.aside ? (
                                            <span className={styles.asideTag}>
                                              {t('plugins.accounts.asideTag')}
                                            </span>
                                          ) : null}
                                        </span>
                                        <span className={styles.quotaValue}>
                                          {row.percentText
                                            || row.amountText
                                            || row.balanceText}
                                        </span>
                                      </div>
                                      {row.percent != null ? (
                                        <div className={styles.quotaTrack}>
                                          <span
                                            className={`${styles.quotaFill} ${
                                              row.level === 'error'
                                                ? styles.quotaFillError
                                                : row.level === 'warning'
                                                  ? styles.quotaFillWarning
                                                  : ''
                                            }`}
                                            style={{ width: `${Math.min(100, row.percent)}%` }}
                                          />
                                        </div>
                                      ) : null}
                                      <div className={styles.quotaFooter}>
                                        <span>
                                          {row.displayText
                                            || (row.percent != null ? row.balanceText : '')
                                            || row.model}
                                        </span>
                                        <span>
                                          {row.resetAt
                                            ? t('plugins.accounts.resetIn', {
                                              time: formatCountdown(row.resetAt),
                                            })
                                            : ''}
                                          {row.runsOutAt
                                            ? t('plugins.accounts.runsOutIn', {
                                              time: formatCountdown(row.runsOutAt),
                                            })
                                            : ''}
                                        </span>
                                      </div>
                                    </div>
                                  ))}
                                </div>
                              ) : (
                                <div className={styles.usageEmpty}>
                                  {t('plugins.accounts.usageEmpty')}
                                </div>
                              )}
                            </>
                          ) : usageLoadingAccounts[account.account] ? (
                            <div className={styles.usageEmpty}>
                              <Spin size="small" />
                              <span>{t('common.loading')}</span>
                            </div>
                          ) : (
                            <div className={styles.usageEmpty}>
                              {usageError
                                ? t('plugins.accounts.usageLoadFailed', { error: usageError })
                                : t('plugins.accounts.usageEmpty')}
                            </div>
                          )}
                          {usage && usageError ? (
                            <Tooltip title={usageError}>
                              <span className={styles.usageRefreshWarning}>
                                <CircleAlert size={12} aria-hidden="true" />
                                <span>{t('plugins.accounts.usageRefreshFailedCached')}</span>
                              </span>
                            </Tooltip>
                          ) : null}
                          {checkinResult ? (
                            <div
                              className={`${styles.checkinStatus} ${checkinStatusClass(checkinResult)}`}
                            >
                              <CalendarCheck size={12} aria-hidden="true" />
                              <span>
                                {t('plugins.accounts.checkinStatusLabel')}
                                {': '}
                                {checkinStatusText(checkinResult)}
                                {checkinResult.message ? ` · ${checkinResult.message}` : ''}
                              </span>
                            </div>
                          ) : null}
                        </div>
                      </article>
                    );
                  })}
                </div>
              )}
            </section>

            <section className={styles.section}>
              <div className={styles.sectionHeader}>
                <h3>{t('plugins.accounts.modelsTitle')}</h3>
                <span className={styles.sectionHint}>
                  {t('plugins.accounts.modelsCount', {
                    count: selectedProvider.models.length,
                  })}
                </span>
                <Input
                  allowClear
                  value={modelSearch}
                  onChange={(event) => setModelSearch(event.target.value)}
                  prefix={<Search size={14} aria-hidden="true" />}
                  placeholder={t('plugins.accounts.modelSearch')}
                  className={styles.modelSearch}
                />
              </div>
              {selectedProvider.models.length === 0 ? (
                <div className={styles.inlineEmpty}>
                  {t('plugins.accounts.modelsEmpty')}
                </div>
              ) : filteredModels.length === 0 ? (
                <div className={styles.inlineEmpty}>
                  {t('plugins.accounts.modelsFilteredEmpty')}
                </div>
              ) : (
                <div className={styles.modelTable}>
                  <div className={styles.modelTableHead}>
                    <span>{t('plugins.accounts.modelColumns.model')}</span>
                    <span>{t('plugins.accounts.modelColumns.protocol')}</span>
                    <span>{t('plugins.accounts.modelColumns.context')}</span>
                    <span>{t('plugins.accounts.modelColumns.flags')}</span>
                  </div>
                  <div className={styles.modelTableBody}>
                    {filteredModels.map((model) => (
                      <div key={model.id} className={styles.modelRow}>
                        <div className={styles.modelIdentity}>
                          <span className={styles.modelId} title={model.id}>
                            {model.id}
                          </span>
                          {model.name && model.name !== model.id ? (
                            <span className={styles.modelName} title={model.name}>
                              {model.name}
                            </span>
                          ) : null}
                        </div>
                        <span className={styles.protocolValue}>
                          {getProtocolLabel(model.npm)}
                        </span>
                        <span className={styles.contextValue}>
                          {formatContextWindow(model.context)}
                        </span>
                        <div className={styles.modelFlags}>
                          {model.reasoning ? (
                            <span className={styles.flag}>
                              {t('plugins.accounts.modelFlags.reasoning')}
                            </span>
                          ) : null}
                          {model.image ? (
                            <span className={styles.flag}>
                              {t('plugins.accounts.modelFlags.vision')}
                            </span>
                          ) : null}
                          {model.free ? (
                            <span className={`${styles.flag} ${styles.flagFree}`}>
                              {t('plugins.accounts.modelFlags.free')}
                            </span>
                          ) : null}
                          {model.rate > 0 ? (
                            <span className={`${styles.flag} ${styles.flagRate}`}>
                              {t('plugins.accounts.modelFlags.rate', { rate: model.rate })}
                            </span>
                          ) : null}
                          {model.fast ? (
                            <span className={styles.flag}>
                              {t('plugins.accounts.modelFlags.fast')}
                            </span>
                          ) : null}
                          {!model.reasoning
                            && !model.image
                            && !model.free
                            && model.rate <= 0
                            && !model.fast ? (
                              <span className={styles.modelFlagsEmpty}>—</span>
                            ) : null}
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </section>

            {selectedProvider.options.length > 0 ? (
              <section className={styles.section}>
                <div className={styles.sectionHeader}>
                  <h3>{t('plugins.accounts.optionsTitle')}</h3>
                  <span className={styles.sectionHint}>
                    {t('plugins.accounts.optionsHint')}
                  </span>
                </div>
                <div className={styles.optionList}>
                  {selectedProvider.options.map((option) => {
                    const label = pickLocalizedText(option.labels, option.label, i18n.language);
                    const description = pickLocalizedText(
                      option.descriptions,
                      option.description,
                      i18n.language,
                    );
                    const checked = providerOptionValues[selectedProvider.id]?.[option.key]
                      ?? option.default;
                    return (
                      <div key={option.key} className={styles.optionRow}>
                        <div className={styles.optionBody}>
                          <span className={styles.optionLabel}>{label}</span>
                          {description ? (
                            <span className={styles.optionDesc}>{description}</span>
                          ) : null}
                        </div>
                        <Switch
                          size="small"
                          checked={checked}
                          aria-label={label}
                          onChange={(nextChecked) =>
                            handleProviderOptionToggle(option, nextChecked)}
                        />
                      </div>
                    );
                  })}
                </div>
              </section>
            ) : null}
          </>
        ) : (
          <div className={styles.empty}>
            <span className={styles.emptyIcon}>
              <PlugZap size={20} aria-hidden="true" />
            </span>
            <div className={styles.emptyTitle}>{t('plugins.accounts.selectProvider')}</div>
          </div>
        )}
      </div>

      <PluginLoginModal
        open={Boolean(loginProvider)}
        provider={loginProvider}
        onClose={() => setLoginProvider(null)}
        onSignedIn={() => {
          setLoginProvider(null);
          reload();
          void onDataChanged();
        }}
      />
      <AddPluginToCliModal
        open={addToCliOpen}
        provider={selectedProvider}
        accounts={providerAccounts}
        onClose={() => setAddToCliOpen(false)}
      />
    </div>
  );
};

export default PluginAccountsView;
