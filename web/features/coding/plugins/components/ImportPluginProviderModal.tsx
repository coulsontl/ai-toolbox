import React from 'react';
import { App, Form, Input, Modal, Spin } from 'antd';
import {
  CheckCircle2,
  CircleAlert,
  PlugZap,
  Search,
  TriangleAlert,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import {
  addPluginProviderToCli,
  listPluginProviders,
  type AddPluginProviderToCliResult,
  type PluginProviderRow,
} from '@/services';
import { formatPluginError } from '../utils/pluginFormatters';
import { getPluginCliPath } from '../utils/pluginCliRoutes';
import PluginIcon from './PluginIcon';
import styles from './PluginModals.module.less';

interface ImportPluginProviderModalProps {
  open: boolean;
  /** The CLI tab whose list triggered the import. */
  cliKey: string;
  onClose: () => void;
  /** Re-read the CLI's providers after the write. */
  onImported?: () => void | Promise<void>;
}

/**
 * The lightweight "import an already signed-in plugin channel into this CLI"
 * entry that every provider list gets through `ProviderListSection`.
 *
 * The full add-to-CLI flow (multiple CLIs, account strategy, model subset) lives
 * in the plugin center's `AddPluginToCliModal`; this one intentionally asks for
 * one provider and one CLI, because the user is already standing in that CLI's
 * list.
 */
const ImportPluginProviderModal: React.FC<ImportPluginProviderModalProps> = ({
  open,
  cliKey,
  onClose,
  onImported,
}) => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const navigate = useNavigate();

  const [providers, setProviders] = React.useState<PluginProviderRow[]>([]);
  const [loading, setLoading] = React.useState(false);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [search, setSearch] = React.useState('');
  const [selectedProviderId, setSelectedProviderId] = React.useState('');
  const [displayName, setDisplayName] = React.useState('');
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [result, setResult] = React.useState<AddPluginProviderToCliResult | null>(null);

  const loadProviders = React.useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const rows = await listPluginProviders();
      setProviders(rows.filter((provider) => provider.signedIn));
    } catch (loadFailure) {
      setLoadError(formatPluginError(loadFailure));
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    setSearch('');
    setSelectedProviderId('');
    setDisplayName('');
    setBusy(false);
    setError(null);
    setResult(null);
    void loadProviders();
  }, [cliKey, loadProviders, open]);

  const selectedProvider = providers.find((provider) => provider.id === selectedProviderId)
    ?? null;

  const filteredProviders = React.useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) {
      return providers;
    }
    return providers.filter((provider) =>
      `${provider.name} ${provider.id} ${provider.spec} ${provider.npm}`
        .toLowerCase()
        .includes(needle));
  }, [providers, search]);

  const selectProvider = (provider: PluginProviderRow) => {
    setSelectedProviderId(provider.id);
    setDisplayName(provider.name || provider.id);
    setResult(null);
    setError(null);
  };

  const handleImport = async () => {
    if (!selectedProvider) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const nextResult = await addPluginProviderToCli({
        cliKey,
        pluginSpec: selectedProvider.spec || selectedProvider.npm,
        providerId: selectedProvider.id,
        displayName: displayName.trim() || undefined,
      });
      setResult(nextResult);
      message.success(t('plugins.import.success', {
        name: nextResult.providerName || selectedProvider.name || selectedProvider.id,
      }));
      try {
        await onImported?.();
      } catch {
        // The import itself succeeded; the list will re-read on its next load.
      }
    } catch (importFailure) {
      setError(formatPluginError(importFailure));
    } finally {
      setBusy(false);
    }
  };

  const cliPath = getPluginCliPath(cliKey);

  return (
    <Modal
      open={open}
      title={t('plugins.import.title')}
      width={640}
      footer={null}
      onCancel={onClose}
      mask={{ closable: !busy }}
    >
      <div className={styles.content}>
        {/*
          The sibling provider dialogs all state `horizontal`; this picker's
          label/value rows are horizontal too. `component={false}` keeps this
          wrapper from adding a DOM node — it only records the layout so the
          shared form-dialog guard sees one uniform group.
        */}
        <Form layout="horizontal" component={false}>
        <div className={styles.sectionCard}>
          <p className={styles.sectionHint}>{t('plugins.import.hint')}</p>

          {loading ? (
            <div className={styles.pickerLoading}>
              <Spin size="small" />
              <span>{t('common.loading')}</span>
            </div>
          ) : loadError ? (
            <div className={styles.pickerError}>
              <CircleAlert size={14} aria-hidden="true" />
              <span>{t('plugins.import.loadFailed', { error: loadError })}</span>
              <button type="button" className={styles.textButton} onClick={() => void loadProviders()}>
                {t('common.retry')}
              </button>
            </div>
          ) : providers.length === 0 ? (
            <div className={styles.inlineEmpty}>
              <span>{t('plugins.import.emptyTitle')}</span>
              <span className={styles.sectionHint}>{t('plugins.import.emptyDesc')}</span>
            </div>
          ) : (
            <>
              <Input
                allowClear
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                prefix={<Search size={14} aria-hidden="true" />}
                placeholder={t('plugins.import.searchPlaceholder')}
              />
              {filteredProviders.length === 0 ? (
                <div className={styles.inlineEmpty}>
                  {t('plugins.import.filteredEmpty')}
                </div>
              ) : (
                <div className={styles.providerPicker}>
                  {filteredProviders.map((provider) => (
                    <button
                      key={provider.id}
                      type="button"
                      className={`${styles.providerOption} ${
                        selectedProviderId === provider.id ? styles.providerOptionActive : ''
                      }`}
                      onClick={() => selectProvider(provider)}
                    >
                      <PluginIcon
                        icon={provider.icon}
                        name={provider.name || provider.id}
                        size={30}
                      />
                      <span className={styles.providerOptionBody}>
                        <span className={styles.providerOptionName}>
                          {provider.name || provider.id}
                        </span>
                        <span className={styles.providerOptionMeta}>
                          {provider.spec || provider.npm}
                          {' · '}
                          {t('plugins.accounts.modelsCount', { count: provider.models.length })}
                        </span>
                      </span>
                      {selectedProviderId === provider.id ? (
                        <CheckCircle2 size={16} aria-hidden="true" />
                      ) : null}
                    </button>
                  ))}
                </div>
              )}
            </>
          )}

          {providers.length > 0 && (
            <div className={styles.formFieldRow}>
              <span className={styles.formLabel}>{t('plugins.import.displayName')}</span>
              <div className={styles.formControl}>
                <Input
                  className={styles.formControlInput}
                  value={displayName}
                  onChange={(event) => setDisplayName(event.target.value)}
                  placeholder={t('plugins.import.displayNamePlaceholder')}
                  disabled={!selectedProvider}
                />
              </div>
            </div>
          )}

          {error ? <div className={styles.formError}>{error}</div> : null}

          {result ? (
            <div className={styles.importResult}>
              <div className={styles.noticeSuccess}>
                <CheckCircle2 size={15} aria-hidden="true" />
                <span>
                  {t('plugins.import.addedToCli', { name: result.providerName })}
                </span>
              </div>
              {!result.gatewayRunning ? (
                <div className={styles.noticeWarning}>
                  <TriangleAlert size={15} aria-hidden="true" />
                  <div className={styles.noticeBody}>
                    <span className={styles.noticeTitle}>
                      {t('plugins.import.gatewayNotRunning')}
                    </span>
                    <span className={styles.sectionHint}>
                      {t('plugins.import.gatewayNotRunningDesc')}
                    </span>
                  </div>
                  <button
                    type="button"
                    className={styles.textButton}
                    onClick={() => {
                      onClose();
                      navigate('/gateway/settings');
                    }}
                  >
                    {t('plugins.import.openGateway')}
                  </button>
                </div>
              ) : null}
              {cliPath ? (
                <button
                  type="button"
                  className={styles.textButton}
                  onClick={() => {
                    onClose();
                    navigate(cliPath);
                  }}
                >
                  <PlugZap size={14} aria-hidden="true" />
                  <span>{t('plugins.import.openCli')}</span>
                </button>
              ) : null}
            </div>
          ) : null}
        </div>
        </Form>

        <div className={styles.footerActions}>
          <button
            type="button"
            className={styles.actionButton}
            onClick={onClose}
            disabled={busy}
          >
            {result ? t('plugins.import.done') : t('common.cancel')}
          </button>
          {!result && (
            <button
              type="button"
              className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
              onClick={() => void handleImport()}
              disabled={!selectedProvider || busy || loading}
            >
              <PlugZap size={15} aria-hidden="true" />
              <span>{busy ? t('plugins.import.adding') : t('plugins.import.add')}</span>
            </button>
          )}
        </div>
      </div>
    </Modal>
  );
};

export default ImportPluginProviderModal;
