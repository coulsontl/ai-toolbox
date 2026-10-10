import React from 'react';
import { Alert, Button, Modal, Space, Typography, message } from 'antd';
import { CopyOutlined, ReloadOutlined } from '@ant-design/icons';
import { open } from '@tauri-apps/plugin-dialog';
import { readTextFile, stat } from '@tauri-apps/plugin-fs';
import { useTranslation } from 'react-i18next';
import { OpenCodeStyleCard } from '@/features/coding/shared/providerCardVariants';
import { OfficialAccountsSection } from '@/features/coding/shared/officialAccounts';
import type { OfficialAccountRowView } from '@/features/coding/shared/officialAccounts';
import { copyTextToClipboard } from '@/services/clipboardApi';
import { importOmpCodexAccount, listOmpCodexAccounts, switchOmpCodexAccount } from '@/services/ohMyPiApi';
import type { OmpCodexAccount, OmpCodexAccountsResult, OmpRuntimeProviderView } from '@/types/ohMyPi';
import { getProviderModelRecords } from '@/utils/ompModelMetadata';

interface Props {
  rootPath: string;
  provider: OmpRuntimeProviderView;
  defaultModel?: string | null;
  onSetPrimaryModel: (modelId: string) => void;
  onRefresh: () => Promise<void>;
}

const OmpSubscriptionCard: React.FC<Props> = ({ rootPath, provider, defaultModel, onSetPrimaryModel, onRefresh }) => {
  const { t } = useTranslation();
  const [loginOpen, setLoginOpen] = React.useState(false);
  const [importOpen, setImportOpen] = React.useState(false);
  const [switchTarget, setSwitchTarget] = React.useState<OmpCodexAccount | null>(null);
  const [refreshing, setRefreshing] = React.useState(false);
  const [mutating, setMutating] = React.useState(false);
  const [accountState, setAccountState] = React.useState<OmpCodexAccountsResult>({ accounts: [], canWrite: false, error: null });
  const request = React.useRef(0);
  const active = React.useRef(true);
  const busy = React.useRef(false);

  React.useEffect(() => {
    active.current = true;
    return () => { active.current = false; request.current += 1; };
  }, []);

  const loadAccounts = React.useCallback(async () => {
    const id = ++request.current;
    try {
      const next = await listOmpCodexAccounts(rootPath);
      if (active.current && id === request.current) setAccountState(next);
    } catch {
      if (active.current && id === request.current) {
        setAccountState({ accounts: [], canWrite: false, error: t('ohMyPi.subscription.accountsUnavailable') });
      }
    }
  }, [rootPath, t]);

  React.useEffect(() => { void loadAccounts(); }, [loadAccounts]);

  const configuredIds = new Set(getProviderModelRecords(provider.modelsProvider).map(({ id }) => id));
  const models = getProviderModelRecords({ models: provider.runtimeModels })
    .filter(({ id }) => !configuredIds.has(id))
    .map(({ id, model }) => ({
      id,
      name: typeof model.name === 'string' ? model.name : id,
      contextLimit: typeof model.contextWindow === 'number' ? model.contextWindow : undefined,
      outputLimit: typeof model.maxTokens === 'number' ? model.maxTokens : undefined,
      isPrimary: provider.isDefault && defaultModel === id,
    }));
  const refresh = async () => {
    if (busy.current) return;
    busy.current = true;
    setRefreshing(true);
    try {
      await Promise.all([loadAccounts(), onRefresh()]);
    } catch {
      if (active.current) message.error(t('ohMyPi.subscription.refreshFailed'));
    } finally {
      busy.current = false;
      if (active.current) setRefreshing(false);
    }
  };
  const accountLabel = (account: OmpCodexAccount) => account.email ?? account.accountId ?? t('ohMyPi.subscription.accountFallback', { id: account.id });
  const accountRows: OfficialAccountRowView[] = accountState.accounts.map((account) => ({
    id: account.id,
    label: accountLabel(account),
    kindTag: t(account.isEnabled ? 'ohMyPi.subscription.accountEnabled' : 'ohMyPi.subscription.accountInactive'),
    metaLines: [
      account.accountId ? t('ohMyPi.subscription.accountOrganization', { id: account.accountId }) : null,
      account.plan ? t('ohMyPi.subscription.accountPlan', { plan: account.plan }) : null,
      account.expiresAt !== null ? t('ohMyPi.subscription.accountExpiry', { date: new Date(account.expiresAt).toLocaleString() }) : null,
      account.disabledCause && account.disabledCause !== 'ai-toolbox:inactive' ? t('ohMyPi.subscription.accountInvalid') : null,
    ].filter((line): line is string => line !== null),
    // Saved activation does not identify the account pinned to a running OMP session.
    isApplied: false,
    isVirtual: false,
  }));
  const canWrite = accountState.canWrite && !refreshing && !mutating;

  const mutate = async (operation: () => Promise<OmpCodexAccountsResult>) => {
    if (!canWrite || busy.current) return;
    busy.current = true;
    request.current += 1;
    setMutating(true);
    try {
      const next = await operation();
      if (!active.current) return;
      setAccountState(next);
      if (next.error) return;
      setImportOpen(false);
      setSwitchTarget(null);
      await Promise.all([loadAccounts(), onRefresh()]);
    } catch {
      if (active.current) message.error(t('ohMyPi.subscription.accountOperationFailed'));
    } finally {
      busy.current = false;
      if (active.current) setMutating(false);
    }
  };

  const importAccount = () => void mutate(async () => {
    const selected = await open({
      title: t('ohMyPi.subscription.importAccount'),
      multiple: false,
      directory: false,
      filters: [{ name: 'auth.json', extensions: ['json'] }],
    });
    if (typeof selected !== 'string' || !active.current) return accountState;
    // Credentials exist only in this operation's local scope, never UI state or logs.
    let authJson = '';
    try {
      const info = await stat(selected);
      if (!info.isFile || info.size > 256 * 1024) {
        return { ...accountState, error: t('ohMyPi.subscription.invalidAuthFile') };
      }
      if (!active.current) return accountState;
      authJson = await readTextFile(selected);
      if (!active.current) return accountState;
      return await importOmpCodexAccount(rootPath, authJson);
    } finally {
      authJson = '';
    }
  });
  const selectAccount = (row: OfficialAccountRowView) => {
    const account = accountState.accounts.find(({ id }) => id === row.id);
    if (!account || !canWrite) return;
    if (account.disabledCause && account.disabledCause !== 'ai-toolbox:inactive') {
      message.warning(t('ohMyPi.subscription.accountInvalid'));
      return;
    }
    setSwitchTarget(account);
  };
  const status = t(`ohMyPi.subscription.status.${provider.oauthStatus ?? 'unavailable'}`);

  return (
    <>
      <OpenCodeStyleCard
        provider={{
          id: `${provider.providerKey}-subscription`,
          name: provider.displayName,
          sdkName: `${t('ohMyPi.categoryLabels.subscription')} · ${status}`,
          baseUrl: t('ohMyPi.subscription.credentialHint'),
        }}
        modelSection={{
          models,
          onSetPrimaryModel,
          aboveList: <Typography.Text type="secondary">{t('ohMyPi.subscription.nativeModels')}</Typography.Text>,
        }}
        footer={
          <>
            <OfficialAccountsSection
              variant="embedded"
              listTitle={t('common.officialAccount.listTitle')}
              emptyText={refreshing ? t('common.loading') : t('ohMyPi.subscription.accountsEmpty')}
              accounts={accountRows}
              actionsDisabled={!canWrite}
              applyHint={t('ohMyPi.subscription.switchAccount')}
              onApply={selectAccount}
              pending={mutating && switchTarget ? { accountId: switchTarget.id, action: 'apply' } : null}
              loginAction={
                <Space size={0} wrap>
                  <Button size="small" type="text" disabled={!canWrite} onClick={() => setImportOpen(true)}>{t('ohMyPi.subscription.importAccount')}</Button>
                  <Button size="small" type="text" onClick={() => setLoginOpen(true)}>{t('ohMyPi.subscription.loginGuidance')}</Button>
                  <Button size="small" type="text" icon={<ReloadOutlined />} loading={refreshing} disabled={mutating} onClick={() => void refresh()}>{t('ohMyPi.refreshConfig')}</Button>
                </Space>
              }
            />
            <Typography.Paragraph type="secondary" style={{ marginTop: 12, marginBottom: 0 }}>
              {t('ohMyPi.subscription.accountsHint')}
            </Typography.Paragraph>
            {(accountState.error || !accountState.canWrite) && (
              <Alert type="warning" showIcon title={accountState.error ?? t('ohMyPi.subscription.accountsReadOnly')} style={{ marginTop: 12 }} />
            )}
          </>
        }
      />
      {provider.runtimeCatalogError ? (
        <Alert type="warning" showIcon title={t('ohMyPi.subscription.catalogError')} description={provider.runtimeCatalogError} style={{ marginBottom: 12 }} />
      ) : !provider.runtimeModels?.length ? (
        <Alert type="info" showIcon title={t('ohMyPi.subscription.emptyCatalog')} style={{ marginBottom: 12 }} />
      ) : null}
      <Modal
        open={importOpen}
        title={t('ohMyPi.subscription.importAccount')}
        onCancel={() => { if (!mutating) setImportOpen(false); }}
        onOk={importAccount}
        okText={t('ohMyPi.subscription.chooseAuthFile')}
        cancelText={t('common.cancel')}
        confirmLoading={mutating}
        okButtonProps={{ disabled: !canWrite }}
        cancelButtonProps={{ disabled: mutating }}
        closable={!mutating}
        maskClosable={!mutating}
      >
        <Alert type="warning" showIcon title={t('ohMyPi.subscription.privateAuthWarning')} />
        <Typography.Paragraph style={{ marginTop: 12 }}>{t('ohMyPi.subscription.importHint')}</Typography.Paragraph>
        <Typography.Text code style={{ overflowWrap: 'anywhere' }}>{rootPath}</Typography.Text>
      </Modal>
      <Modal
        open={switchTarget !== null}
        title={t('ohMyPi.subscription.switchAccount')}
        onCancel={() => { if (!mutating) setSwitchTarget(null); }}
        onOk={() => { if (switchTarget) void mutate(() => switchOmpCodexAccount(rootPath, switchTarget.id)); }}
        okText={t('common.confirm')}
        cancelText={t('common.cancel')}
        confirmLoading={mutating}
        okButtonProps={{ disabled: !canWrite }}
        cancelButtonProps={{ disabled: mutating }}
        closable={!mutating}
        maskClosable={!mutating}
      >
        <Typography.Paragraph>{t('ohMyPi.subscription.switchHint')}</Typography.Paragraph>
        {switchTarget && <Typography.Paragraph strong style={{ overflowWrap: 'anywhere' }}>{accountLabel(switchTarget)} · {switchTarget.accountId ?? switchTarget.id}</Typography.Paragraph>}
        <Typography.Paragraph code style={{ overflowWrap: 'anywhere' }}>{rootPath}</Typography.Paragraph>
        <Alert type="warning" showIcon title={t('ohMyPi.subscription.restartHint')} />
      </Modal>
      <Modal
        open={loginOpen}
        title={t('ohMyPi.subscription.loginGuidance')}
        onCancel={() => setLoginOpen(false)}
        footer={[
          <Button key="close" onClick={() => setLoginOpen(false)}>{t('common.close')}</Button>,
          <Button key="refresh" type="primary" loading={refreshing} disabled={mutating} onClick={() => void refresh()}>{t('ohMyPi.subscription.refreshAfterLogin')}</Button>,
        ]}
      >
        <Space orientation="vertical" style={{ width: '100%' }}>
          <Typography.Text strong>{status}</Typography.Text>
          <Typography.Paragraph>{t('ohMyPi.subscription.loginHint')}</Typography.Paragraph>
          {provider.loginCommand ? (
            <>
              <Typography.Text code style={{ overflowWrap: 'anywhere', whiteSpace: 'pre-wrap' }}>{provider.loginCommand}</Typography.Text>
              <Button icon={<CopyOutlined />} onClick={() => void copyTextToClipboard(provider.loginCommand!).then(
                () => message.success(t('common.copied')),
                () => message.error(t('common.error')),
              )}>{t('common.copy')}</Button>
            </>
          ) : <Alert type="warning" showIcon title={t('ohMyPi.subscription.loginUnavailable')} />}
          <Typography.Paragraph type="secondary">{t('ohMyPi.subscription.credentialHint')}</Typography.Paragraph>
        </Space>
      </Modal>
    </>
  );
};

export default OmpSubscriptionCard;
