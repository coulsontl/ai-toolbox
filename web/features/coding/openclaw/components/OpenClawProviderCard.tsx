import React from 'react';
import { Button, Space, Tooltip } from 'antd';
import {
  CopyOutlined,
  DeleteOutlined,
  EditOutlined,
} from '@ant-design/icons';
import { Share2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import OpenCodeStyleCard from '@/features/coding/shared/providerCardVariants/OpenCodeStyleCard';
import ProviderModelSection from '@/features/coding/shared/providerCardVariants/ProviderModelSection';
import type {
  ProviderCardModels,
  ProviderCardVariantProps,
} from '@/features/coding/shared/providerCardVariants/types';
import {
  ProviderCompactCard,
  type ProviderDetailSection,
} from '@/features/coding/shared/providerCardView';
import type { ProviderViewMode } from '@/features/coding/shared/providerList';
import type {
  ModelDisplayData,
  ProviderConnectivityStatusItem,
} from '@/components/common/ProviderCard/types';
import type { OpenClawProviderConfig, OpenClawModel } from '@/types/openclaw';
import { openClawApiToNpm } from '../utils/apiToNpm';

interface OpenClawProviderCardProps {
  providerId: string;
  config: OpenClawProviderConfig;
  /** `'card'` renders the compact grid cell plus the detail sidebar. */
  viewMode?: ProviderViewMode;
  /** Hides the drag handle while the list order cannot be changed (non-custom sort or search). */
  dragDisabled?: boolean;
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (selected: boolean) => void;
  connectivityStatus?: ProviderConnectivityStatusItem;

  onEdit: () => void;
  onCopy: () => void;
  onShare?: () => void;
  onDelete: () => void;
  /** Disables the delete button and explains why on hover (the default provider). */
  deleteDisabledReason?: string;

  /** Model catalog actions. Omit a handler to hide its button. */
  onAddModel: () => void;
  onEditModel: (model: OpenClawModel) => void;
  onCopyModel: (model: OpenClawModel) => void;
  onDeleteModel: (modelId: string) => void;
  onReorderModels: (modelIds: string[]) => void;
  /** Model-row batch delete, driven from the catalog toolbar. */
  modelSelectionMode?: boolean;
  selectedModelIds?: string[];
  onToggleModelSelection?: (modelId: string, selected: boolean) => void;
  onToggleBatchDeleteMode?: () => void;
  onBatchDeleteModels?: () => void;

  /** Whole-catalog probes: connectivity and upstream model discovery. */
  onTestModels: () => void;
  testModelsDisabled: boolean;
  testModelsDisabledTooltip?: string;
  onFetchModels: () => void;
  fetchDisabled: boolean;
  fetchDisabledTooltip?: string;
}

/**
 * An OpenClaw provider, rendered in the OpenCode style.
 *
 * OpenClaw owns a per-provider model catalog (`models.providers.<id>.models`)
 * and has no provider-level "apply": `agents.defaults.model.primary` points at
 * one model inside one channel, and that pointer is edited in
 * `AgentsDefaultsCard`. That is the OpenCode shape, so the layout lives in the
 * shared variant and this file only maps OpenClaw's storage onto it.
 *
 * Both layouts are built from the same `props` object, so the list card and the
 * grid cell cannot offer different actions.
 */
const OpenClawProviderCard: React.FC<OpenClawProviderCardProps> = ({
  providerId,
  config,
  viewMode = 'list',
  dragDisabled = false,
  selectable = false,
  selected = false,
  onSelectChange,
  connectivityStatus,
  onEdit,
  onCopy,
  onShare,
  onDelete,
  deleteDisabledReason,
  onAddModel,
  onEditModel,
  onCopyModel,
  onDeleteModel,
  onReorderModels,
  modelSelectionMode = false,
  selectedModelIds = [],
  onToggleModelSelection,
  onToggleBatchDeleteMode,
  onBatchDeleteModels,
  onTestModels,
  testModelsDisabled,
  testModelsDisabledTooltip,
  onFetchModels,
  fetchDisabled,
  fetchDisabledTooltip,
}) => {
  const { t } = useTranslation();
  const baseUrl = config.baseUrl || '';
  const apiKey = config.apiKey || '';
  const sdkName = openClawApiToNpm(config.api, config.baseUrl);
  const catalogModels = config.models || [];

  const modelRows = React.useMemo<ModelDisplayData[]>(
    () =>
      catalogModels.map((model) => ({
        id: model.id,
        name: model.name || model.id,
        contextLimit: model.contextWindow,
        outputLimit: model.maxTokens,
      })),
    [catalogModels],
  );

  /** The row handlers take ids; the page's handlers take the stored model. */
  const findModel = (modelId: string): OpenClawModel | undefined =>
    catalogModels.find((model) => model.id === modelId);

  /**
   * The catalog, as data. Hoisted out of the props literal because the card
   * view renders it in a different place (the detail sidebar), not because it
   * states something different.
   */
  const modelSection: ProviderCardModels = {
    models: modelRows,
    modelsDraggable: !modelSelectionMode,
    onReorderModels,
    modelSelectionMode,
    selectedModelIds,
    onToggleModelSelection,
    onToggleBatchDeleteMode,
    onBatchDeleteModels,
    // Probing and discovery belong to the catalog, not to the card: they run
    // against the whole model list, which is why the bespoke card kept them in
    // its model toolbar too.
    onTestModels,
    testModelsDisabled,
    testModelsDisabledTooltip,
    onFetchModels,
    fetchDisabled,
    fetchDisabledTooltip,
    onAddModel,
    onEditModel: (modelId) => {
      const model = findModel(modelId);
      if (model) onEditModel(model);
    },
    onCopyModel: (modelId) => {
      const model = findModel(modelId);
      if (model) onCopyModel(model);
    },
    onDeleteModel,
  };

  const props: ProviderCardVariantProps = {
    provider: {
      id: providerId,
      name: providerId,
      sdkName,
      baseUrl,
    },
    providerState: {
      // Card-level drag handle: the provider order is what is being sorted.
      draggable: !selectable && !dragDisabled,
      sortableId: providerId,
      connectivityStatus,
      selectable,
      selected,
      onSelectChange,
    },
    actions: {
      onEdit,
      onCopy,
      onShare,
      onDelete,
      // No header "应用": a provider is never switched on or off here.
      deleteConfirm: false,
      deleteDisabledReason,
    },
    modelSection,
  };

  if (viewMode !== 'card') {
    return <OpenCodeStyleCard {...props} />;
  }

  const detailSections: ProviderDetailSection[] = [{
    key: 'connection',
    title: t('common.provider.connectionTitle'),
    fields: [
      {
        key: 'baseUrl',
        label: t('openclaw.providers.baseUrl'),
        value: baseUrl,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
      },
      // No `emptyText`: a provider that never declared a protocol has none to
      // show, so the row drops instead of claiming it is unconfigured.
      { key: 'api', label: t('openclaw.providers.apiProtocol'), value: config.api },
      {
        key: 'apiKey',
        label: t('openclaw.providers.apiKey'),
        value: apiKey,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
        elideMiddle: true,
      },
    ],
  }, {
    // The catalog brings its own title, toolbar and empty state.
    key: 'catalog',
    content: (
      <ProviderModelSection
        providerId={providerId}
        modelSection={modelSection}
        appearance="detail"
      />
    ),
  }];

  /**
   * The sidebar footer confirms on its own (it is not a `Popconfirm` host), and
   * the page's delete handler asks as well — the list card, the grid menu and
   * this footer all go through it, so the question is asked exactly once, where
   * every entry point meets.
   */
  const handleDeleteFromDetail = () => {
    if (deleteDisabledReason) {
      return;
    }
    onDelete();
  };

  return (
    <ProviderCompactCard
      provider={props.provider}
      // The cell states one fact: the AI SDK package this channel speaks. A
      // provider with no catalog still has one, so the second line is never
      // blank; the endpoint and the catalog live in the sidebar.
      subtitle={[{ value: sdkName }]}
      // No primary action of its own, so the action row carries only the
      // "more" menu; `'subtitle'` keeps it on the second line instead of
      // opening an empty bordered footer bar.
      actionPlacement="subtitle"
      providerState={props.providerState}
      actions={props.actions}
      extraMenuItems={[{
        key: 'model-test',
        label: t('opencode.connectivity.button'),
        disabled: testModelsDisabled,
        onClick: onTestModels,
      }]}
      detail={{
        title: <ProviderNameLink name={providerId} baseUrl={baseUrl} />,
        sections: detailSections,
        secondaryActions: (
          <Space size={4} wrap>
            <Button size="small" type="text" icon={<EditOutlined />} onClick={onEdit}>
              {t('common.edit')}
            </Button>
            <Button size="small" type="text" icon={<CopyOutlined />} onClick={onCopy}>
              {t('common.copy')}
            </Button>
            {onShare && (
              <Button size="small" type="text" icon={<Share2 size={14} />} onClick={onShare}>
                {t('common.share')}
              </Button>
            )}
            <Tooltip title={deleteDisabledReason}>
              <span>
                <Button
                  size="small"
                  type="text"
                  danger
                  icon={<DeleteOutlined />}
                  disabled={Boolean(deleteDisabledReason)}
                  onClick={handleDeleteFromDetail}
                >
                  {t('common.delete')}
                </Button>
              </span>
            </Tooltip>
          </Space>
        ),
      }}
    />
  );
};

export default OpenClawProviderCard;
