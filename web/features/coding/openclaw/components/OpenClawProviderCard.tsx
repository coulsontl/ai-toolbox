import React from 'react';
import { Button, Space, Tooltip } from 'antd';
import { ApiOutlined, CloudDownloadOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import ProviderCard from '@/components/common/ProviderCard';
import type { ProviderDisplayData, ModelDisplayData } from '@/components/common/ProviderCard/types';
import type { OpenClawProviderConfig, OpenClawModel } from '@/types/openclaw';

interface Props {
  providerId: string;
  config: OpenClawProviderConfig;
  onEdit: () => void;
  onDelete: () => void;
  onAddModel: () => void;
  onEditModel: (model: OpenClawModel) => void;
  onDeleteModel: (modelId: string) => void;
  onConnectivityTest: () => void;
  onFetchModels: () => void;
}

const toProviderDisplayData = (id: string, config: OpenClawProviderConfig): ProviderDisplayData => ({
  id,
  name: id,
  sdkName: config.api || '',
  baseUrl: config.baseUrl || '',
});

const toModelDisplayData = (model: OpenClawModel): ModelDisplayData => ({
  id: model.id,
  name: model.name || model.id,
  contextLimit: model.contextWindow,
  outputLimit: model.maxTokens,
});

const OpenClawProviderCard: React.FC<Props> = ({
  providerId,
  config,
  onEdit,
  onDelete,
  onAddModel,
  onEditModel,
  onDeleteModel,
  onConnectivityTest,
  onFetchModels,
}) => {
  const { t } = useTranslation();

  const isAuthReady = Boolean(config.baseUrl?.trim() && config.apiKey?.trim());
  const authTooltip = !isAuthReady ? t('openclaw.providers.completeUrlAndKey') : '';

  const provider = toProviderDisplayData(providerId, config);
  const models = (config.models || []).map(toModelDisplayData);

  // Map model ID back to OpenClawModel for edit callback
  const modelMap = React.useMemo(() => {
    const map = new Map<string, OpenClawModel>();
    for (const m of config.models || []) {
      map.set(m.id, m);
    }
    return map;
  }, [config.models]);

  return (
    <ProviderCard
      provider={provider}
      models={models}
      onEdit={onEdit}
      onDelete={onDelete}
      onAddModel={onAddModel}
      onEditModel={(modelId) => {
        const model = modelMap.get(modelId);
        if (model) onEditModel(model);
      }}
      onDeleteModel={onDeleteModel}
      extraActions={
        <Space size={0}>
          <Tooltip title={authTooltip}>
            <span>
              <Button
                size="small"
                type="text"
                style={{ fontSize: 12 }}
                onClick={onConnectivityTest}
                disabled={!isAuthReady || (config.models || []).length === 0}
              >
                <ApiOutlined style={{ marginRight: 4 }} />
                {t('openclaw.providers.connectivityTest')}
              </Button>
            </span>
          </Tooltip>
          <Tooltip title={authTooltip}>
            <span>
              <Button
                size="small"
                type="text"
                style={{ fontSize: 12 }}
                onClick={onFetchModels}
                disabled={!isAuthReady}
              >
                <CloudDownloadOutlined style={{ marginRight: 4 }} />
                {t('openclaw.providers.fetchModels')}
              </Button>
            </span>
          </Tooltip>
        </Space>
      }
      i18nPrefix="openclaw"
    />
  );
};

export default OpenClawProviderCard;
