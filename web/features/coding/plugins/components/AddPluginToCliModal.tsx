import React from 'react';
import { App, Checkbox, Input, Modal, Select, Spin, Tooltip } from 'antd';
import {
  Check,
  CheckCircle2,
  CircleAlert,
  ExternalLink,
  PlugZap,
  Search,
  TriangleAlert,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import {
  addPluginProviderToCli,
  listPluginCliTargets,
  type AddPluginProviderToCliResult,
  type PluginAccountSummary,
  type PluginCliTarget,
  type PluginProviderRow,
} from '@/services';
import { formatPluginError } from '../utils/pluginFormatters';
import { getPluginCliPath } from '../utils/pluginCliRoutes';
import styles from './PluginModals.module.less';

interface AddPluginToCliModalProps {
  open: boolean;
  provider: PluginProviderRow | null;
  /** Signed-in accounts of `provider`, already filtered by the accounts view. */
  accounts: PluginAccountSummary[];
  onClose: () => void;
}

interface AddFailure {
  cliKey: string;
  label: string;
  message: string;
}

const TARGET_GROUPS: Array<{
  mode: PluginCliTarget['mode'];
  labelKey: string;
}> = [
  { mode: 'gateway_takeover', labelKey: 'plugins.addToCli.group.gatewayTakeover' },
  { mode: 'direct_plugin_route', labelKey: 'plugins.addToCli.group.directPluginRoute' },
];

/**
 * "Add this plugin channel to one or more CLI tabs".
 *
 * Opened from the plugin center's account detail. The command is per-CLI, so a
 * batch is issued as one request per selected target and reported as a list of
 * successes/failures rather than all-or-nothing.
 */
const AddPluginToCliModal: React.FC<AddPluginToCliModalProps> = ({
  open,
  provider,
  accounts,
  onClose,
}) => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const navigate = useNavigate();

  const [targets, setTargets] = React.useState<PluginCliTarget[]>([]);
  const [targetsLoading, setTargetsLoading] = React.useState(false);
  const [targetsError, setTargetsError] = React.useState<string | null>(null);
  const [selectedCliKeys, setSelectedCliKeys] = React.useState<string[]>([]);
  const [displayName, setDisplayName] = React.useState('');
  const [accountStrategy, setAccountStrategy] = React.useState<'auto_rotate' | 'single'>(
    'auto_rotate',
  );
  const [accountKey, setAccountKey] = React.useState('');
  const [selectedModelIds, setSelectedModelIds] = React.useState<string[]>([]);
  const [modelSearch, setModelSearch] = React.useState('');
  const [busy, setBusy] = React.useState(false);
  const [submitError, setSubmitError] = React.useState<string | null>(null);
  const [results, setResults] = React.useState<AddPluginProviderToCliResult[] | null>(null);
  const [failures, setFailures] = React.useState<AddFailure[]>([]);

  const loadTargets = React.useCallback(async () => {
    setTargetsLoading(true);
    setTargetsError(null);
    try {
      setTargets(await listPluginCliTargets());
    } catch (error) {
      setTargetsError(formatPluginError(error));
    } finally {
      setTargetsLoading(false);
    }
  }, []);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    void loadTargets();
  }, [loadTargets, open]);

  React.useEffect(() => {
    if (!open || !provider) {
      return;
    }
    setSelectedCliKeys([]);
    setDisplayName(provider.name || provider.id);
    setAccountStrategy(accounts.length > 1 ? 'auto_rotate' : 'single');
    setAccountKey(accounts[0]?.account ?? '');
    setSelectedModelIds(provider.models.map((model) => model.id));
    setModelSearch('');
    setBusy(false);
    setSubmitError(null);
    setResults(null);
    setFailures([]);
  }, [accounts, open, provider]);

  const selectedTarget = (target: PluginCliTarget) => selectedCliKeys.includes(target.cliKey);

  const selectedTargets = React.useMemo(
    () => targets.filter((target) => selectedCliKeys.includes(target.cliKey)),
    [selectedCliKeys, targets],
  );
  const hasGatewayTarget = selectedTargets.some(
    (target) => target.mode === 'gateway_takeover',
  );
  const hasDirectTarget = selectedTargets.some(
    (target) => target.mode === 'direct_plugin_route',
  );
  // File-direct CLIs always rotate accounts; only gateway takeovers honour the
  // strategy selector, and only when there is more than one account to choose.
  const gatewayAccountStrategy = accounts.length > 1 ? accountStrategy : 'single';
  const showAccountStrategy = hasGatewayTarget && accounts.length > 1;

  const toggleTarget = (target: PluginCliTarget) => {
    if (!target.supported) {
      return;
    }
    setSelectedCliKeys((current) => current.includes(target.cliKey)
      ? current.filter((key) => key !== target.cliKey)
      : [...current, target.cliKey]);
  };

  const filteredModels = React.useMemo(() => {
    const models = provider?.models ?? [];
    const needle = modelSearch.trim().toLowerCase();
    if (!needle) {
      return models;
    }
    return models.filter((model) =>
      `${model.id} ${model.name}`.toLowerCase().includes(needle));
  }, [modelSearch, provider?.models]);

  const allModelsSelected = Boolean(provider?.models.length)
    && selectedModelIds.length === provider?.models.length;

  const toggleAllModels = (checked: boolean) => {
    setSelectedModelIds(checked ? (provider?.models.map((model) => model.id) ?? []) : []);
  };

  const toggleModel = (modelId: string, checked: boolean) => {
    setSelectedModelIds((current) => checked
      ? (current.includes(modelId) ? current : [...current, modelId])
      : current.filter((id) => id !== modelId));
  };

  const handleAdd = async () => {
    if (!provider) {
      return;
    }
    if (selectedCliKeys.length === 0) {
      setSubmitError(t('plugins.addToCli.needTarget'));
      return;
    }
    if (accounts.length === 0) {
      setSubmitError(t('plugins.addToCli.needAccount'));
      return;
    }
    if (provider.models.length > 0 && selectedModelIds.length === 0) {
      setSubmitError(t('plugins.addToCli.needModel'));
      return;
    }
    if (hasGatewayTarget && gatewayAccountStrategy === 'single' && !accountKey) {
      setSubmitError(t('plugins.addToCli.needAccount'));
      return;
    }

    setBusy(true);
    setSubmitError(null);
    setResults(null);
    setFailures([]);

    const settled = await Promise.allSettled(selectedTargets.map((target) => {
      const strategy = target.mode === 'direct_plugin_route'
        ? 'auto_rotate'
        : gatewayAccountStrategy;
      return addPluginProviderToCli({
        cliKey: target.cliKey,
        pluginSpec: provider.spec || provider.npm,
        providerId: provider.id,
        displayName: displayName.trim() || undefined,
        accountStrategy: strategy,
        accountKey: strategy === 'single' ? accountKey : null,
        modelIds: provider.models.length > 0 ? selectedModelIds : undefined,
      });
    }));

    const successes: AddPluginProviderToCliResult[] = [];
    const nextFailures: AddFailure[] = [];
    for (const [index, entry] of settled.entries()) {
      const cliKey = selectedTargets[index]?.cliKey ?? selectedCliKeys[index];
      const label = targets.find((target) => target.cliKey === cliKey)?.label || cliKey;
      if (entry.status === 'fulfilled') {
        successes.push(entry.value);
      } else {
        nextFailures.push({
          cliKey,
          label,
          message: formatPluginError(entry.reason),
        });
      }
    }

    setResults(successes);
    setFailures(nextFailures);
    setBusy(false);

    if (successes.length > 0) {
      message.success(t('plugins.addToCli.success', { count: successes.length }));
    }
    if (nextFailures.length > 0) {
      message.warning(t('plugins.addToCli.partialFailed', { count: nextFailures.length }));
    }
  };

  const gatewayStopped = results?.some((result) => !result.gatewayRunning) ?? false;
  const targetLabelOf = (cliKey: string) =>
    targets.find((target) => target.cliKey === cliKey)?.label || cliKey;

  return (
    <Modal
      open={open}
      title={provider
        ? t('plugins.addToCli.title', { name: provider.name || provider.id })
        : ''}
      width={760}
      footer={null}
      onCancel={busy ? undefined : onClose}
      mask={{ closable: !busy }}
    >
      <div className={styles.content}>
        <div className={styles.sectionCard}>
          <h3 className={styles.sectionTitle}>{t('plugins.addToCli.targetsTitle')}</h3>
          <p className={styles.sectionHint}>{t('plugins.addToCli.targetsHint')}</p>

          {targetsLoading ? (
            <div className={styles.pickerLoading}>
              <Spin size="small" />
              <span>{t('common.loading')}</span>
            </div>
          ) : targetsError ? (
            <div className={styles.pickerError}>
              <CircleAlert size={14} aria-hidden="true" />
              <span>{t('plugins.addToCli.loadFailed', { error: targetsError })}</span>
              <button type="button" className={styles.textButton} onClick={() => void loadTargets()}>
                {t('common.retry')}
              </button>
            </div>
          ) : (
            TARGET_GROUPS.map((group) => {
              const groupTargets = targets.filter((target) => target.mode === group.mode);
              if (groupTargets.length === 0) {
                return null;
              }
              return (
                <div key={group.mode} className={styles.targetGroup}>
                  <div className={styles.targetGroupHeader}>
                    <span className={styles.targetGroupTitle}>{t(group.labelKey)}</span>
                    <span className={styles.targetCount}>{groupTargets.length}</span>
                  </div>
                  <div className={styles.targetList}>
                    {groupTargets.map((target) => {
                      const active = selectedTarget(target);
                      const option = (
                        <button
                          type="button"
                          className={[
                            styles.targetOption,
                            active ? styles.targetOptionActive : undefined,
                            !target.supported ? styles.targetOptionDisabled : undefined,
                          ].filter(Boolean).join(' ')}
                          onClick={() => toggleTarget(target)}
                          disabled={!target.supported}
                          aria-pressed={active}
                        >
                          <span className={`${styles.targetCheck} ${active ? styles.targetCheckActive : ''}`}>
                            {active ? <Check size={11} aria-hidden="true" /> : null}
                          </span>
                          <span className={styles.targetOptionBody}>
                            <span className={styles.targetOptionName}>
                              {target.label || target.cliKey}
                            </span>
                            <span className={styles.targetOptionKey}>{target.cliKey}</span>
                          </span>
                          {!target.supported ? (
                            target.reason ? (
                              <span className={styles.targetReason}>{target.reason}</span>
                            ) : null
                          ) : (
                            <span className={styles.targetMode}>{t('plugins.addToCli.selectable')}</span>
                          )}
                        </button>
                      );
                      return target.supported ? (
                        <React.Fragment key={target.cliKey}>{option}</React.Fragment>
                      ) : (
                        <Tooltip key={target.cliKey} title={target.reason}>
                          <span className={styles.targetTooltipHost}>{option}</span>
                        </Tooltip>
                      );
                    })}
                  </div>
                </div>
              );
            })
          )}
        </div>

        <div className={styles.sectionCard}>
          <h3 className={styles.sectionTitle}>{t('plugins.addToCli.optionsTitle')}</h3>

          <div className={styles.formFieldRow}>
            <span className={styles.formLabel}>{t('plugins.addToCli.displayName')}</span>
            <div className={styles.formControl}>
              <Input
                className={styles.formControlInput}
                value={displayName}
                onChange={(event) => setDisplayName(event.target.value)}
                placeholder={provider?.name || provider?.id}
              />
            </div>
          </div>

          {showAccountStrategy ? (
            <>
              <div className={styles.formFieldRow}>
                <span className={styles.formLabel}>{t('plugins.addToCli.accountStrategy')}</span>
                <div className={styles.formControl}>
                  <div className={styles.segmented}>
                    <button
                      type="button"
                      className={`${styles.segmentButton} ${
                        accountStrategy === 'auto_rotate' ? styles.segmentButtonActive : ''
                      }`}
                      onClick={() => setAccountStrategy('auto_rotate')}
                    >
                      {t('plugins.addToCli.strategyAuto')}
                    </button>
                    <button
                      type="button"
                      className={`${styles.segmentButton} ${
                        accountStrategy === 'single' ? styles.segmentButtonActive : ''
                      }`}
                      onClick={() => setAccountStrategy('single')}
                    >
                      {t('plugins.addToCli.strategySingle')}
                    </button>
                  </div>
                  <span className={styles.sectionHint}>
                    {accountStrategy === 'auto_rotate'
                      ? t('plugins.addToCli.strategyAutoHint')
                      : t('plugins.addToCli.strategySingleHint')}
                  </span>
                </div>
              </div>
              {accountStrategy === 'single' ? (
                <div className={styles.formFieldRow}>
                  <span className={styles.formLabel}>{t('plugins.addToCli.account')}</span>
                  <div className={styles.formControl}>
                    <Select
                      className={styles.formControlInput}
                      value={accountKey || undefined}
                      onChange={setAccountKey}
                      placeholder={t('plugins.addToCli.accountPlaceholder')}
                      options={accounts.map((account) => ({
                        value: account.account,
                        label: account.displayName || account.account,
                      }))}
                    />
                  </div>
                </div>
              ) : null}
            </>
          ) : null}

          {hasDirectTarget && hasGatewayTarget ? (
            <p className={styles.sectionHint}>{t('plugins.addToCli.directStrategyNote')}</p>
          ) : null}
          {hasDirectTarget && !hasGatewayTarget ? (
            <p className={styles.sectionHint}>{t('plugins.addToCli.directAutoRotateHint')}</p>
          ) : null}

          <div className={styles.formFieldRow}>
            <span className={styles.formLabel}>{t('plugins.addToCli.models')}</span>
            <div className={styles.formControl}>
              <span className={styles.sectionHint}>
                {t('plugins.addToCli.modelsSelected', {
                  selected: selectedModelIds.length,
                  total: provider?.models.length ?? 0,
                })}
              </span>
              <Input
                allowClear
                size="small"
                className={styles.modelSearchInput}
                value={modelSearch}
                onChange={(event) => setModelSearch(event.target.value)}
                prefix={<Search size={13} aria-hidden="true" />}
                placeholder={t('plugins.addToCli.modelSearch')}
              />
            </div>
          </div>

          {(provider?.models.length ?? 0) === 0 ? (
            <div className={styles.inlineEmpty}>{t('plugins.addToCli.noModels')}</div>
          ) : (
            <div className={styles.modelPicker}>
              <div className={styles.modelPickerHeader}>
                <Checkbox
                  checked={allModelsSelected}
                  indeterminate={selectedModelIds.length > 0 && !allModelsSelected}
                  onChange={(event) => toggleAllModels(event.target.checked)}
                >
                  {t('plugins.addToCli.selectAllModels')}
                </Checkbox>
              </div>
              <div className={styles.modelPickerList}>
                {filteredModels.length === 0 ? (
                  <div className={styles.inlineEmpty}>
                    {t('plugins.addToCli.modelsFilteredEmpty')}
                  </div>
                ) : filteredModels.map((model) => (
                  <Checkbox
                    key={model.id}
                    className={styles.modelPickRow}
                    checked={selectedModelIds.includes(model.id)}
                    onChange={(event) => toggleModel(model.id, event.target.checked)}
                  >
                    <span className={styles.modelPickBody}>
                      <span className={styles.modelPickId}>{model.id}</span>
                      {model.name && model.name !== model.id ? (
                        <span className={styles.modelPickName}>{model.name}</span>
                      ) : null}
                    </span>
                  </Checkbox>
                ))}
              </div>
            </div>
          )}

          {submitError ? <div className={styles.formError}>{submitError}</div> : null}

          {results && results.length > 0 ? (
            <div className={styles.importResult}>
              <div className={styles.noticeSuccess}>
                <CheckCircle2 size={15} aria-hidden="true" />
                <span>{t('plugins.addToCli.success', { count: results.length })}</span>
              </div>
              {gatewayStopped ? (
                <div className={styles.noticeWarning}>
                  <TriangleAlert size={15} aria-hidden="true" />
                  <div className={styles.noticeBody}>
                    <span className={styles.noticeTitle}>
                      {t('plugins.addToCli.gatewayNotRunning')}
                    </span>
                    <span className={styles.sectionHint}>
                      {t('plugins.addToCli.gatewayNotRunningDesc')}
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
                    {t('plugins.addToCli.openGateway')}
                  </button>
                </div>
              ) : null}
              <div className={styles.resultList}>
                {results.map((result) => {
                  const cliPath = getPluginCliPath(result.cliKey);
                  return (
                    <div key={result.cliKey} className={styles.resultRow}>
                      <CheckCircle2 size={13} aria-hidden="true" />
                      <span className={styles.resultUser}>
                        {targetLabelOf(result.cliKey)}
                      </span>
                      <span className={styles.resultMeta}>
                        {t('plugins.addToCli.modelCount', { count: result.modelCount })}
                      </span>
                      {cliPath ? (
                        <button
                          type="button"
                          className={styles.linkButton}
                          onClick={() => {
                            onClose();
                            navigate(cliPath);
                          }}
                        >
                          <ExternalLink size={12} aria-hidden="true" />
                          <span>{t('plugins.addToCli.openCli')}</span>
                        </button>
                      ) : null}
                    </div>
                  );
                })}
              </div>
            </div>
          ) : null}

          {failures.length > 0 ? (
            <div className={styles.resultList}>
              {failures.map((failure) => (
                <div key={failure.cliKey} className={styles.resultRow}>
                  <TriangleAlert size={13} aria-hidden="true" />
                  <span className={styles.resultUser}>{failure.label}</span>
                  <span className={styles.resultError}>{failure.message}</span>
                </div>
              ))}
            </div>
          ) : null}
        </div>

        <div className={styles.footerActions}>
          <button
            type="button"
            className={styles.actionButton}
            onClick={onClose}
            disabled={busy}
          >
            {results ? t('plugins.addToCli.done') : t('common.cancel')}
          </button>
          {!results && (
            <button
              type="button"
              className={`${styles.actionButton} ${styles.actionButtonPrimary}`}
              onClick={() => void handleAdd()}
              disabled={
                busy
                || targetsLoading
                || Boolean(targetsError)
                || selectedCliKeys.length === 0
              }
            >
              <PlugZap size={15} aria-hidden="true" />
              <span>
                {busy
                  ? t('plugins.addToCli.submitting')
                  : t('plugins.addToCli.submit', { count: selectedCliKeys.length })}
              </span>
            </button>
          )}
        </div>
      </div>
    </Modal>
  );
};

export default AddPluginToCliModal;
