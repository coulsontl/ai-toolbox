import React from 'react';
import { Alert, Button, Modal, Space, Typography, message } from 'antd';
import { CopyOutlined, ReloadOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import ProviderCard from '@/components/common/ProviderCard';
import { copyTextToClipboard } from '@/services/clipboardApi';
import type { OmpRuntimeProviderView } from '@/types/ohMyPi';
import { getProviderModelRecords } from '@/utils/ompModelMetadata';

interface Props {
  provider: OmpRuntimeProviderView;
  defaultModel?: string | null;
  onSetPrimaryModel: (modelId: string) => void;
  onRefresh: () => Promise<void>;
}

const OmpSubscriptionCard: React.FC<Props> = ({ provider, defaultModel, onSetPrimaryModel, onRefresh }) => {
  const { t } = useTranslation();
  const [loginOpen, setLoginOpen] = React.useState(false);
  const [refreshing, setRefreshing] = React.useState(false);
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
    setRefreshing(true);
    try {
      await onRefresh();
    } finally {
      setRefreshing(false);
    }
  };
  const status = t(`ohMyPi.subscription.status.${provider.oauthStatus ?? 'unavailable'}`);

  return (
    <>
      <ProviderCard
        provider={{
          id: `${provider.providerKey}-subscription`,
          name: provider.displayName,
          sdkName: `${t('ohMyPi.categoryLabels.subscription')} · ${status}`,
          baseUrl: t('ohMyPi.subscription.credentialHint'),
        }}
        models={models}
        modelSourceTag={t('ohMyPi.subscription.nativeModels')}
        onSetPrimaryModel={onSetPrimaryModel}
        extraActions={
          <Space size={0}>
            <Button size="small" type="text" onClick={() => setLoginOpen(true)}>
              {t('ohMyPi.subscription.loginGuidance')}
            </Button>
            <Button size="small" type="text" icon={<ReloadOutlined />} loading={refreshing} onClick={() => void refresh()}>
              {t('ohMyPi.refreshConfig')}
            </Button>
          </Space>
        }
      />
      {provider.runtimeCatalogError ? (
        <Alert type="warning" showIcon title={t('ohMyPi.subscription.catalogError')} description={provider.runtimeCatalogError} style={{ marginBottom: 12 }} />
      ) : !provider.runtimeModels?.length ? (
        <Alert type="info" showIcon title={t('ohMyPi.subscription.emptyCatalog')} style={{ marginBottom: 12 }} />
      ) : null}
      <Modal
        open={loginOpen}
        title={t('ohMyPi.subscription.loginGuidance')}
        onCancel={() => setLoginOpen(false)}
        footer={[
          <Button key="close" onClick={() => setLoginOpen(false)}>{t('common.close')}</Button>,
          <Button key="refresh" type="primary" loading={refreshing} onClick={() => void refresh()}>{t('ohMyPi.subscription.refreshAfterLogin')}</Button>,
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
