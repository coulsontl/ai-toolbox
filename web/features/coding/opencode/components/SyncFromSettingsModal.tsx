import React from 'react';
import { Modal, Button, Checkbox, Alert, Empty, message, Space, Typography } from 'antd';
import { useTranslation } from 'react-i18next';
import { getAllProvidersWithModels } from '@/services/providerApi';
import type { ProviderWithModels } from '@/types/provider';
import type { OpenCodeConfig, OpenCodeProvider, OpenCodeModel } from '@/types/opencode';

const { Text } = Typography;

interface SyncFromSettingsModalProps {
  open: boolean;
  currentConfig: OpenCodeConfig;
  onCancel: () => void;
  onSuccess: (config: OpenCodeConfig) => void;
}

const SyncFromSettingsModal: React.FC<SyncFromSettingsModalProps> = ({
  open,
  currentConfig,
  onCancel,
  onSuccess,
}) => {
  const { t } = useTranslation();
  const [providers, setProviders] = React.useState<ProviderWithModels[]>([]);
  const [selectedProviders, setSelectedProviders] = React.useState<string[]>([]);

  React.useEffect(() => {
    if (open) {
      loadProviders();
    }
  }, [open]);

  const loadProviders = async () => {
    try {
      const data = await getAllProvidersWithModels();
      setProviders(data);
      setSelectedProviders([]);
    } catch (error) {
      message.error(t('common.error'));
    }
  };

  const handleSync = () => {
    if (selectedProviders.length === 0) {
      message.warning(t('opencode.sync.selectProviders'));
      return;
    }

    const newProviders = { ...currentConfig.provider };

    selectedProviders.forEach((providerId) => {
      const providerWithModels = providers.find((p) => p.provider.id === providerId);
      if (!providerWithModels) return;

      const { provider, models } = providerWithModels;

      // Convert to OpenCode format
      const opencodeModels: Record<string, OpenCodeModel> = {};
      models.forEach((model) => {
        // Parse options from settings model
        let modelOptions: Record<string, unknown> | undefined;
        if (model.options && model.options !== '{}') {
          try {
            modelOptions = JSON.parse(model.options);
          } catch {
            modelOptions = undefined;
          }
        }

        opencodeModels[model.id] = {
          name: model.name,
          limit: {
            context: model.context_limit,
            output: model.output_limit,
          },
          ...(modelOptions && Object.keys(modelOptions).length > 0 ? { options: modelOptions } : {}),
        };
      });

      const opencodeProvider: OpenCodeProvider = {
        npm: provider.provider_type,
        name: provider.name,
        options: {
          baseURL: provider.base_url,
          ...(provider.api_key && { apiKey: provider.api_key }),
          ...(provider.headers && {
            headers: JSON.parse(provider.headers),
          }),
        },
        models: opencodeModels,
      };

      newProviders[provider.id] = opencodeProvider;
    });

    const newConfig: OpenCodeConfig = {
      ...currentConfig,
      provider: newProviders,
    };

    onSuccess(newConfig);
    message.success(t('opencode.sync.syncSuccess'));
  };

  const handleSelectAll = () => {
    if (selectedProviders.length === providers.length) {
      setSelectedProviders([]);
    } else {
      setSelectedProviders(providers.map((p) => p.provider.id));
    }
  };

  return (
    <Modal
      title={t('opencode.sync.title')}
      open={open}
      onCancel={onCancel}
      footer={[
        <Button key="cancel" onClick={onCancel}>
          {t('common.cancel')}
        </Button>,
        <Button key="submit" type="primary" onClick={handleSync} disabled={selectedProviders.length === 0}>
          {t('common.confirm')}
        </Button>,
      ]}
      width={700}
    >
      <Space direction="vertical" style={{ width: '100%' }} size={16}>
        <Alert
          message={t('opencode.sync.description')}
          description={t('opencode.sync.mergeDescription')}
          type="info"
          showIcon
        />

        {providers.length === 0 ? (
          <Empty description={t('opencode.sync.noProviders')} />
        ) : (
          <>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <Button type="link" onClick={handleSelectAll} style={{ padding: 0 }}>
                {selectedProviders.length === providers.length ? t('common.cancel') : t('opencode.sync.selectAll')}
              </Button>
              <Text type="secondary">
                {t('opencode.sync.selected', { count: selectedProviders.length })}
              </Text>
            </div>

            <Space direction="vertical" style={{ width: '100%', maxHeight: 400, overflowY: 'auto' }}>
              {providers.map((item) => (
                <div
                  key={item.provider.id}
                  style={{
                    border: '1px solid #e8e8e8',
                    borderRadius: 4,
                    padding: 12,
                  }}
                >
                  <Checkbox
                    checked={selectedProviders.includes(item.provider.id)}
                    onChange={(e) => {
                      if (e.target.checked) {
                        setSelectedProviders([...selectedProviders, item.provider.id]);
                      } else {
                        setSelectedProviders(selectedProviders.filter((id) => id !== item.provider.id));
                      }
                    }}
                  >
                    <div>
                      <Text strong>{item.provider.name}</Text>
                      <Text type="secondary" style={{ marginLeft: 8, fontSize: 12 }}>
                        ({item.provider.id})
                      </Text>
                    </div>
                  </Checkbox>
                  <div style={{ marginLeft: 24, marginTop: 4 }}>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {item.provider.provider_type} • {item.models.length} {t('opencode.model.title')}
                    </Text>
                  </div>
                </div>
              ))}
            </Space>
          </>
        )}
      </Space>
    </Modal>
  );
};

export default SyncFromSettingsModal;
