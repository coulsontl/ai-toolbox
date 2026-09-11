import React from 'react';
import { Button, Modal, Space, Tooltip } from 'antd';
import { ApiOutlined, CloudDownloadOutlined, DeleteOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import ProviderCard from '@/components/common/ProviderCard';
import type {
  ProviderDisplayData,
  ModelDisplayData,
  ProviderConnectivityStatusItem,
} from '@/components/common/ProviderCard/types';
import type { OpenClawProviderConfig, OpenClawModel } from '@/types/openclaw';

interface Props {
  providerId: string;
  config: OpenClawProviderConfig;
  draggable?: boolean;
  sortableId?: string;
  modelsDraggable?: boolean;
  onReorderModels?: (modelIds: string[]) => void;
  onEdit: () => void;
  onShare?: () => void;
  onDelete: () => void;
  onAddModel: () => void;
  onEditModel: (model: OpenClawModel) => void;
  onDeleteModel: (modelId: string) => void;
  /** 批量删除模型:多选 + 切换 + 确认执行 */
  modelSelectionMode?: boolean;
  selectedModelIds?: string[];
  onToggleModelSelection?: (modelId: string, selected: boolean) => void;
  onToggleBatchDeleteMode?: () => void;
  onBatchDeleteModels?: () => void;
  onConnectivityTest: () => void;
  onFetchModels: () => void;
  connectivityStatus?: ProviderConnectivityStatusItem;
  /** 当该渠道承载主模型(agents.defaults.model.primary)时，删除按钮置灰并显示此提示 */
  deleteDisabledReason?: string;
  /** Provider-level batch selection mode (swaps drag handle for a checkbox). */
  selectable?: boolean;
  /** Whether this provider is currently selected (only meaningful when selectable). */
  selected?: boolean;
  /** Toggle provider selection. */
  onSelectChange?: (checked: boolean) => void;
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
  draggable,
  sortableId,
  modelsDraggable,
  onReorderModels,
  onEdit,
  onShare,
  onDelete,
  onAddModel,
  onEditModel,
  onDeleteModel,
  onConnectivityTest,
  onFetchModels,
  connectivityStatus,
  modelSelectionMode,
  selectedModelIds,
  onToggleModelSelection,
  onToggleBatchDeleteMode,
  onBatchDeleteModels,
  deleteDisabledReason,
  selectable = false,
  selected = false,
  onSelectChange,
}) => {
  const { t } = useTranslation();

  const isAuthReady = Boolean(config.baseUrl?.trim() && config.apiKey?.trim());
  const authTooltip = !isAuthReady ? t('openclaw.providers.completeUrlAndKey') : '';
  const isBatchDeleteMode = Boolean(modelSelectionMode);
  const selectedModelCount = selectedModelIds?.length ?? 0;

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
      draggable={draggable}
      sortableId={sortableId}
      modelsDraggable={modelsDraggable}
      onReorderModels={onReorderModels}
      onEdit={onEdit}
      onShare={onShare}
      onDelete={onDelete}
      deleteDisabledReason={deleteDisabledReason}
      selectable={selectable}
      selected={selected}
      onSelectChange={onSelectChange}
      onAddModel={onAddModel}
      onEditModel={(modelId) => {
        const model = modelMap.get(modelId);
        if (model) onEditModel(model);
      }}
      onDeleteModel={onDeleteModel}
      modelSelectionMode={modelSelectionMode}
      selectedModelIds={selectedModelIds}
      onToggleModelSelection={onToggleModelSelection}
      connectivityStatus={connectivityStatus}
      extraActions={
        <Space size={4}>
          {onToggleBatchDeleteMode && (
            <>
              <Button
                size="small"
                type="text"
                style={{ fontSize: 12 }}
                onClick={onToggleBatchDeleteMode}
              >
                <DeleteOutlined style={{ marginRight: 4 }} />
                {isBatchDeleteMode
                  ? t('openclaw.providers.cancelBatchDelete')
                  : t('openclaw.providers.batchDelete')}
              </Button>
              {isBatchDeleteMode && (
                <Button
                  size="small"
                  danger
                  style={{ fontSize: 12 }}
                  disabled={selectedModelCount === 0}
                  onClick={() => {
                    Modal.confirm({
                      title: t('openclaw.providers.batchDeleteConfirmTitle'),
                      content: t('openclaw.providers.batchDeleteConfirmContent', { count: selectedModelCount }),
                      okText: t('common.delete'),
                      cancelText: t('common.cancel'),
                      onOk: () => onBatchDeleteModels?.(),
                    });
                  }}
                >
                  {t('openclaw.providers.deleteSelected', { count: selectedModelCount })}
                </Button>
              )}
            </>
          )}
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
                {t('opencode.connectivity.button')}
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
