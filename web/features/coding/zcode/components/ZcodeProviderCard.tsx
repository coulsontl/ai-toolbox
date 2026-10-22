import React from 'react';
import { Button, Space, Tag, Typography } from 'antd';
import { CopyOutlined, DeleteOutlined, EditOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import AppliedTag from '@/components/common/AppliedTag';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import OpenCodeStyleCard from '@/features/coding/shared/providerCardVariants/OpenCodeStyleCard';
import ProviderModelSection from '@/features/coding/shared/providerCardVariants/ProviderModelSection';
import { ProviderEnabledSwitch } from '@/features/coding/shared/providerCardVariants/providerCardActions';
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
import { ZCODE_API_TYPES, type ZcodeProvider } from '@/types/zcode';
import { parseZcodeProviderSettings } from '../utils/zcodeSettingsConfig';
import { zcodeSdkName } from '../utils/zcodeFavoriteProvider';

const { Text } = Typography;

interface ZcodeProviderCardProps {
  provider: ZcodeProvider;
  onEdit: () => void;
  /** 复制成一条新 provider（预填来源值、新 id）。 */
  onCopy?: () => void;
  onDelete: () => void;
  /** `'card'` renders the compact grid cell plus the detail sidebar. */
  viewMode?: ProviderViewMode;
  /** Renders a checkbox instead of the drag handle while batch selection is on. */
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (selected: boolean) => void;
  /** Why this row cannot be picked in selection mode (disabled checkbox tooltip). */
  selectDisabledReason?: string;
  /** Latest result of the batch connectivity test for this provider. */
  connectivityStatus?: ProviderConnectivityStatusItem;
  /** Flips the provider's disabled flag. */
  onToggleDisabled: () => void;

  /** Model catalog actions. Omit a handler to hide its button. */
  onAddModel?: () => void;
  onEditModel?: (modelId: string) => void;
  onCopyModel?: (modelId: string) => void;
  onDeleteModel?: (modelId: string) => void;
  /**
   * Makes this model the one ZCode starts new sessions with.
   *
   * This is the card's only "apply" path: a ZCode provider is never switched on
   * or off, so there is nothing to apply at the provider level.
   */
  onSetPrimaryModel?: (modelId: string) => void;
  /** Switches one model on or off in ZCode's catalog. */
  onToggleModelDisabled?: (modelId: string, isDisabled: boolean) => void;
  onReorderModels?: (orderedModelIds: string[]) => void;
  /** Hides the drag handle while the list order cannot be changed (non-custom sort or search). */
  dragDisabled?: boolean;
  /** Disables the delete button and explains why on hover. */
  deleteDisabledReason?: string;
  /** Connectivity test for the whole catalog; hidden while it cannot run. */
  onTestModels?: () => void;
  testModelsDisabled?: boolean;
  testModelsDisabledTooltip?: string;
  onFetchModels?: () => void;
  modelSelectionMode?: boolean;
  selectedModelIds?: string[];
  onToggleModelSelection?: (modelId: string, selected: boolean) => void;
  onToggleBatchDeleteMode?: () => void;
  onBatchDeleteModels?: () => void;
}

/**
 * A ZCode provider, rendered in the OpenCode style.
 *
 * The layout lives in the shared variant; this file only maps ZCode's storage
 * shape onto it, so the card cannot drift from the other OpenCode-style CLIs.
 *
 * Two ZCode-specific facts drive the mapping:
 *
 * - The provider's API format and endpoint live inside the `settings_config`
 *   JSON blob, not on the row, so they are parsed out here.
 * - A provider is always usable; "which one is active" is expressed per model,
 *   not per card. That is why the header carries no apply button and the
 *   `设为默认` action sits on the model row.
 */
const ZcodeProviderCard: React.FC<ZcodeProviderCardProps> = ({
  provider,
  onEdit,
  onCopy,
  onDelete,
  selectable = false,
  selected = false,
  onSelectChange,
  selectDisabledReason,
  connectivityStatus,
  onToggleDisabled,
  onAddModel,
  onEditModel,
  onCopyModel,
  onDeleteModel,
  onSetPrimaryModel,
  onToggleModelDisabled,
  onReorderModels,
  dragDisabled = false,
  deleteDisabledReason,
  onTestModels,
  testModelsDisabled = false,
  testModelsDisabledTooltip,
  onFetchModels,
  modelSelectionMode = false,
  selectedModelIds = [],
  onToggleModelSelection,
  onToggleBatchDeleteMode,
  onBatchDeleteModels,
  viewMode = 'list',
}) => {
  const { t } = useTranslation();
  const settings = parseZcodeProviderSettings(provider.settingsConfig);
  const apiType = settings?.config?.api?.type;
  const baseUrl = settings?.config?.api?.baseUrl ?? '';
  const apiKey = settings?.config?.access?.apiKey ?? '';
  const models = settings?.models ?? [];
  const defaultModelId = models.find((model) => model.isDefault)?.modelId;

  /**
   * `ModelListSection` renders `ModelDisplayData`, so each ZCode row is mapped
   * here. The row key is the `modelId` — ZCode keys models by it, and the
   * default flag travels separately in `isPrimary`.
   */
  const modelRows = React.useMemo<ModelDisplayData[]>(
    () =>
      models.map((model) => ({
        id: model.modelId,
        name: model.displayName?.trim() || model.modelId,
        contextLimit: model.properties?.contextWindow,
        outputLimit: model.optionSpecs?.maxOutputTokens?.max,
        isPrimary: model.modelId === defaultModelId,
        // An absent flag means enabled — only an explicit `false` is off.
        isDisabled: model.enabled === false,
      })),
    [models, defaultModelId],
  );

  const nameTags = (
    <>
      {provider.isApplied && (
        <AppliedTag>{t('zcode.provider.applied', { defaultValue: '默认' })}</AppliedTag>
      )}
      {provider.isDisabled && (
        <Tag style={{ margin: 0 }}>{t('zcode.provider.disabled', { defaultValue: '已禁用' })}</Tag>
      )}
    </>
  );

  /** The default model's display name, which is what both the cell and the form show. */
  const defaultModelLabel = (() => {
    const model = models.find((item) => item.modelId === defaultModelId);
    return model ? model.displayName?.trim() || model.modelId : undefined;
  })();

  const apiFormatLabel =
    ZCODE_API_TYPES.find((item) => item.value === apiType)?.label ?? apiType ?? undefined;

  /**
   * The catalog, as data.
   *
   * Hoisted out of the props literal because the card view renders it in a
   * *different place* — the detail sidebar — not because it states something
   * different. Building it once is what keeps the two layouts offering the same
   * catalog with the same actions.
   */
  const modelSection: ProviderCardModels = {
    models: modelRows,
    modelsDraggable: !modelSelectionMode && Boolean(onReorderModels),
    onReorderModels,
    modelSelectionMode,
    selectedModelIds,
    onToggleModelSelection,
    onToggleBatchDeleteMode,
    onBatchDeleteModels,
    onTestModels,
    testModelsDisabled,
    testModelsDisabledTooltip,
    onFetchModels,
    fetchDisabled: !baseUrl,
    fetchDisabledTooltip: t('opencode.provider.completeUrlAndKey'),
    onAddModel,
    onEditModel,
    onCopyModel,
    onDeleteModel,
    onSetPrimaryModel,
    onToggleModelDisabled,
  };

  /**
   * The detail sidebar's content: where the provider points, and what it can
   * serve. The default model is not repeated here — the catalog below marks its
   * row 当前主模型, and a second statement of one fact is how the two drift.
   */
  const detailSections: ProviderDetailSection[] = [{
    key: 'connection',
    title: t('common.provider.connectionTitle'),
    fields: [
      {
        key: 'baseUrl',
        label: t('zcode.provider.baseUrl'),
        value: baseUrl,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
      },
      // No `emptyText`: a provider the CLI never configured has no protocol of
      // its own, so the row drops rather than rendering a placeholder.
      { key: 'apiFormat', label: t('zcode.provider.apiFormat'), value: apiFormatLabel },
      {
        key: 'apiKey',
        label: 'API Key',
        value: apiKey,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
        elideMiddle: true,
      },
    ],
  }];

  // The catalog brings its own header, toolbar and empty state, so this section
  // carries no title of its own. A provider with no models still gets it: an
  // empty list with 添加模型 is the only way to start one.
  detailSections.push({
    key: 'catalog',
    content: <ProviderModelSection providerId={provider.id} modelSection={modelSection} appearance="detail" />,
  });

  if (provider.notes) {
    detailSections.push({
      key: 'notes',
      title: t('common.provider.notes'),
      content: <Text type="secondary" style={{ fontSize: 12 }}>{provider.notes}</Text>,
    });
  }

  const props: ProviderCardVariantProps = {
    provider: {
      id: provider.id,
      name: provider.name,
      sdkName: zcodeSdkName(apiType),
      baseUrl,
    },
    providerState: {
      isDisabled: provider.isDisabled,
      onToggleDisabled,
      // Card-level drag handle: the provider order is what is being sorted here.
      // The sensors are removed while the list cannot be reordered, and the
      // handle must disappear with them (otherwise it renders but drags nothing).
      draggable: !selectable && !dragDisabled,
      sortableId: provider.id,
      connectivityStatus,
      selectable,
      selected,
      onSelectChange,
      selectDisabledReason,
      dimmed: provider.isDisabled,
    },
    actions: {
      onEdit,
      onCopy,
      onDelete,
      deleteConfirm: false,
      deleteDisabledReason,
    },
    nameTags,
    modelSection,
  };

  if (viewMode !== 'card') {
    return <OpenCodeStyleCard {...props} />;
  }

  return (
    <ProviderCompactCard
      provider={{ id: provider.id, name: provider.name, baseUrl }}
      // The one model the cell states: the one ZCode starts new sessions
      // with. The list card states the SDK and the endpoint instead — this is
      // the fact a reader of the grid has to have, and it is the same
      // `isDefault` flag the catalog below marks with 当前主模型.
      subtitle={defaultModelLabel
        ? [{ label: t('zcode.model.defaultLabel'), value: defaultModelLabel }]
        : undefined}
      // One fact and no *primary* action of its own (a ZCode provider is never
      // applied — `设为默认` lives on a model row), so there is nothing to put
      // in a footer bar, and the row's right end is free for the one control
      // this CLI does have per provider. It is the same switch the list card
      // draws inline in its header, and it rides the row the other CLIs' cells
      // give to 应用 / 网关代理 (user-directed, 2026-10-15: 「把它在列表模式下的
      // 开关按钮放出来，放到现在应用的地方」).
      actionPlacement="subtitle"
      cellControl={
        <ProviderEnabledSwitch
          isDisabled={provider.isDisabled}
          onToggleDisabled={onToggleDisabled}
        />
      }
      providerState={props.providerState}
      actions={props.actions}
      nameTags={nameTags}
      // The cell has no second line of diagnostics; the SDK and the endpoint
      // moved into the sidebar's 连接信息, which is where the rest of the
      // provider's facts live. The drawer itself is the card's (shared) — this
      // layer only says what goes in it.
      detail={{
        title: <ProviderNameLink name={provider.name} baseUrl={baseUrl} />,
        statusTags: nameTags,
        sections: detailSections,
        secondaryActions: (
          <Space size={4} wrap>
            <Button size="small" type="text" icon={<EditOutlined />} onClick={onEdit}>
              {t('common.edit')}
            </Button>
            {onCopy && (
              <Button size="small" type="text" icon={<CopyOutlined />} onClick={onCopy}>
                {t('common.copy')}
              </Button>
            )}
            <Button size="small" type="text" danger icon={<DeleteOutlined />} onClick={onDelete}>
              {t('common.delete')}
            </Button>
          </Space>
        ),
      }}
    />
  );
};

export default ZcodeProviderCard;
