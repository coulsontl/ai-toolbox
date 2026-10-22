import React from 'react';
import { Button, Modal, Space, Tooltip } from 'antd';
import {
  CopyOutlined,
  DeleteOutlined,
  EditOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import OpenCodeStyleCard from '@/features/coding/shared/providerCardVariants/OpenCodeStyleCard';
import ProviderModelSection from '@/features/coding/shared/providerCardVariants/ProviderModelSection';
import type { ProviderCardVariantProps } from '@/features/coding/shared/providerCardVariants/types';
import {
  ProviderCompactCard,
  type ProviderDetailSection,
} from '@/features/coding/shared/providerCardView';
import type { ProviderViewMode } from '@/features/coding/shared/providerList';
import type {
  ModelDisplayData,
  ProviderConnectivityStatusItem,
} from '@/components/common/ProviderCard/types';
import type { OmoNativeProvider } from '@/types/omoNative';
import {
  getNumberField,
  getOmoNativeModelEntries,
  getStringField,
  omoNativeApiToNpm,
} from '../utils/omoNativeProviders';

interface OmoNativeProviderCardProps {
  provider: OmoNativeProvider;
  /**
   * Which layout the page is in. `'card'` renders the compact grid cell (and
   * its own detail sidebar); anything else renders the list card.
   */
  viewMode?: ProviderViewMode;
  onEdit: () => void;
  onCopy: () => void;
  onDelete: () => void;
  /**
   * Greys out the delete button and explains why on hover.
   *
   * Set while this provider is the one `settings.json` starts with: deleting it
   * would leave the engine's `defaultProvider` pointing at nothing, so the
   * button stays in place but stops working (same rule as Pi).
   */
  deleteDisabledReason?: string;
  /** Renders a checkbox instead of the drag handle while batch selection is on. */
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (selected: boolean) => void;
  /** Why this row cannot be picked in selection mode (disabled checkbox tooltip). */
  selectDisabledReason?: string;
  /**
   * Hides the drag handle when reordering is not available (non-`custom` sort
   * mode, or an active search) — a handle that cannot move anything is worse
   * than no handle.
   */
  dragDisabled?: boolean;
  /** Latest result of the batch connectivity test for this provider. */
  connectivityStatus?: ProviderConnectivityStatusItem;
  /**
   * The model `settings.json` starts with. The grid cell states it only for the
   * provider that owns it, the same way OpenCode's cell states its default model.
   */
  defaultModelId?: string;

  /** Model catalog actions. Omit a handler to hide its button. */
  onAddModel?: () => void;
  onEditModel?: (modelId: string) => void;
  onCopyModel?: (modelId: string) => void;
  onDeleteModel?: (modelId: string) => void;
  onReorderModels?: (orderedModelIds: string[]) => void;
  /** Connectivity test for the whole catalog (model-list toolbar). */
  onTestModels?: () => void;
  testModelsDisabled?: boolean;
  testModelsDisabledTooltip?: string;
  /** Opens the "fetch models from the upstream API" dialog (model-list toolbar). */
  onFetchModels?: () => void;

  /** Model-row batch delete (model-list toolbar). */
  modelSelectionMode?: boolean;
  selectedModelIds?: string[];
  onToggleModelSelection?: (modelId: string, selected: boolean) => void;
  onToggleBatchDeleteMode?: () => void;
  onBatchDeleteModels?: () => void;
}

/**
 * An OmO Native provider, rendered in the OpenCode style.
 *
 * Style choice follows §0.1.1 of the onboarding SOP: `models.json` holds a
 * per-provider model catalog (so not the Claude style), and several providers
 * are usable at once — there is no "switch to this one" action (so not the
 * Codex style either).
 *
 * The layout lives in the shared variants; this file only maps Native's storage
 * shape (`models.json` + `auth.json`) onto them. Both layouts are fed by the
 * same `props` object so the two can never offer different actions.
 */
const OmoNativeProviderCard: React.FC<OmoNativeProviderCardProps> = ({
  provider,
  viewMode = 'list',
  onEdit,
  onCopy,
  onDelete,
  deleteDisabledReason,
  selectable = false,
  selected = false,
  onSelectChange,
  selectDisabledReason,
  dragDisabled = false,
  connectivityStatus,
  defaultModelId,
  onAddModel,
  onEditModel,
  onCopyModel,
  onDeleteModel,
  onReorderModels,
  onTestModels,
  testModelsDisabled = false,
  testModelsDisabledTooltip,
  onFetchModels,
  modelSelectionMode = false,
  selectedModelIds = [],
  onToggleModelSelection,
  onToggleBatchDeleteMode,
  onBatchDeleteModels,
}) => {
  const { t } = useTranslation();
  const config = provider.config;
  const baseUrl = getStringField(config, 'baseUrl');
  const api = getStringField(config, 'api');
  const apiType = getStringField(config, 'api');
  const entries = getOmoNativeModelEntries(config);

  const modelRows = React.useMemo<ModelDisplayData[]>(
    () =>
      entries.map((entry) => ({
        id: entry.id,
        name: getStringField(entry.model, 'name') || entry.id,
        contextLimit: getNumberField(entry.model, 'contextWindow'),
        outputLimit: getNumberField(entry.model, 'maxTokens'),
      })),
    [entries],
  );

  const displayName = getStringField(config, 'name') || provider.key;

  const props: ProviderCardVariantProps = {
    provider: {
      id: provider.key,
      name: displayName,
      sdkName: omoNativeApiToNpm(api),
      baseUrl,
    },
    providerState: {
      // 卡片级拖拽把手：顺序存在 `models.json` 的键序里（见 reorder 命令）。
      draggable: !selectable && !dragDisabled,
      sortableId: provider.key,
      connectivityStatus,
      selectable,
      selected,
      onSelectChange,
      selectDisabledReason,
    },
    actions: {
      onEdit,
      onCopy,
      onDelete,
      deleteDisabledReason,
      deleteConfirm: false,
      // No primary action: several Native providers are usable at once, so a
      // header "应用" would imply the others were switched off.
    },
    modelSection: {
      models: modelRows,
      modelsDraggable: Boolean(onReorderModels),
      onReorderModels,
      onAddModel,
      onEditModel,
      onCopyModel,
      onDeleteModel,
      // 「连通性测试」「获取模型」放在模型列表工具栏（与 ZCode / Codex 一致），
      // 不放在卡片头部——它们是**针对模型目录**的操作，不是针对这张卡片的。
      onTestModels,
      testModelsDisabled,
      testModelsDisabledTooltip,
      onFetchModels,
      fetchDisabled: !baseUrl,
      fetchDisabledTooltip: t('opencode.provider.completeUrlAndKey'),
      modelSelectionMode,
      selectedModelIds,
      onToggleModelSelection,
      onToggleBatchDeleteMode,
      onBatchDeleteModels,
      // Native has no per-model enable switch and no "set as default" pointer
      // in `models.json`, so neither row action is wired.
    },
  };

  if (viewMode !== 'card') {
    return <OpenCodeStyleCard {...props} />;
  }

  const modelSection = props.modelSection!;
  const defaultModelName = defaultModelId
    ? modelRows.find((row) => row.id === defaultModelId)?.name ?? defaultModelId
    : undefined;

  /**
   * The cell's second line: the npm package (a fact every provider has, with
   * `@ai-sdk/openai-compatible` as the fallback), plus the model this provider
   * starts with when it owns the default.
   *
   * Without the package the line would be empty for every provider that is not
   * the default, which reads as missing data rather than "no default here"
   * (the same reasoning as OpenCode's cell, 2026-10-19). The package carries no
   * label because `@scope/name` names itself.
   */
  const cardSubtitle = [
    { value: omoNativeApiToNpm(api) },
    ...(defaultModelName
      ? [{ label: t('common.model.currentPrimary'), value: defaultModelName }]
      : []),
  ];

  const detailSections: ProviderDetailSection[] = [
    {
      key: 'connection',
      title: t('common.provider.connectionTitle'),
      fields: [
        {
          key: 'baseUrl',
          label: t('omoNative.providers.baseUrl'),
          value: baseUrl,
          emptyText: t('common.provider.notConfigured'),
          copyable: true,
        },
        {
          key: 'api',
          label: t('omoNative.providers.apiType'),
          value: apiType,
          emptyText: t('common.provider.notConfigured'),
          copyable: true,
        },
        {
          key: 'apiKey',
          label: t('omoNative.providers.providerApiKey'),
          value: provider.apiKey,
          emptyText: t('common.provider.notConfigured'),
          copyable: true,
          elideMiddle: true,
        },
      ],
    },
    // The catalog brings its own title, toolbar and empty state.
    {
      key: 'catalog',
      content: (
        <ProviderModelSection
          providerId={provider.key}
          modelSection={modelSection}
          appearance="detail"
        />
      ),
    },
  ];

  /**
   * The sidebar footer is not a `Popconfirm` host, so its delete button asks
   * first and then runs the same handler the list card calls. The guard also
   * lives here: without it the sidebar would be a way around the default
   * provider's `deleteDisabledReason`.
   */
  const handleDeleteFromDetail = () => {
    if (deleteDisabledReason) {
      return;
    }
    Modal.confirm({
      title: t('common.provider.deleteProvider'),
      content: t('common.provider.confirmDelete', { name: displayName }),
      okText: t('common.confirm'),
      cancelText: t('common.cancel'),
      okButtonProps: { danger: true },
      onOk: onDelete,
    });
  };

  return (
    <ProviderCompactCard
      provider={props.provider}
      subtitle={cardSubtitle}
      // No provider-level apply and no enable switch exists in Native, so the
      // action row carries only the "more" menu; `'subtitle'` keeps it on the
      // second line instead of opening an empty bordered footer bar.
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
        title: <ProviderNameLink name={displayName} baseUrl={baseUrl} />,
        sections: detailSections,
        secondaryActions: (
          <Space size={4} wrap>
            <Button size="small" type="text" icon={<EditOutlined />} onClick={onEdit}>
              {t('common.edit')}
            </Button>
            <Button size="small" type="text" icon={<CopyOutlined />} onClick={onCopy}>
              {t('common.copy')}
            </Button>
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

export default OmoNativeProviderCard;
