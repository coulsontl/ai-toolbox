import React from 'react';
import { Button, Space, Tag, Tooltip } from 'antd';
import { CopyOutlined, DeleteOutlined, EditOutlined, ShareAltOutlined } from '@ant-design/icons';
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
import type { PiRuntimeProviderView } from '@/types/pi';
import { piApiToSdkName } from '../utils/piFetchedModels';
import { getProviderModelRecords, getStringField } from '../utils/providerModels';
import { maskProviderApiKey } from '../utils/providerConfig';

interface PiProviderCardProps {
  provider: PiRuntimeProviderView;
  /**
   * The global default model id, when this provider owns the default pointer.
   * Only then does a model row wear `当前主模型`.
   */
  defaultModelId?: string;
  /** `'card'` renders the compact grid cell plus the detail sidebar. */
  viewMode?: ProviderViewMode;
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (selected: boolean) => void;
  /** Why this row cannot be picked in selection mode (disabled checkbox tooltip). */
  selectDisabledReason?: string;
  connectivityStatus?: ProviderConnectivityStatusItem;
  /** Set while search or a non-`custom` sort makes the visible order not the stored one. */
  dragDisabled?: boolean;

  onEdit?: () => void;
  onCopy?: () => void;
  onShare?: () => void;
  onDelete?: () => void;
  deleteDisabledReason?: string;

  /** Model catalog actions. Omit a handler to hide its button. */
  onAddModel?: () => void;
  onEditModel?: (modelId: string) => void;
  onCopyModel?: (modelId: string) => void;
  onDeleteModel?: (modelId: string) => void;
  onSetPrimaryModel?: (modelId: string) => void;
  onReorderModels?: (modelIds: string[]) => void;
  modelSelectionMode?: boolean;
  selectedModelIds?: string[];
  onToggleModelSelection?: (modelId: string, selected: boolean) => void;
  onToggleBatchDeleteMode?: () => void;
  onBatchDeleteModels?: () => void;

  /** Whole-catalog probes: connectivity and upstream model discovery. */
  onTestModels?: () => void;
  testModelsDisabled?: boolean;
  testModelsDisabledTooltip?: string;
  onFetchModels?: () => void;
  fetchDisabled?: boolean;
  fetchDisabledTooltip?: string;
}

/**
 * A Pi provider, rendered in the OpenCode style.
 *
 * Pi keeps a per-provider model catalog in `models.json` and has no
 * provider-level "apply": `modelSettings.defaultProvider` + `modelId` pick one
 * model inside one channel, and `设为默认` on a model row moves that pointer.
 * That is the OpenCode shape (SOP §4.2.1).
 *
 * The old card expressed the same data through `ProviderDisplayData`, whose
 * `baseUrl` fell back to the credential mask and then the source labels. The
 * shared card has a dedicated second line, so the fallbacks stay here — but the
 * "which sources produced this provider" fact becomes a tag, not a fake URL.
 */
const PiProviderCard: React.FC<PiProviderCardProps> = ({
  provider,
  defaultModelId,
  viewMode = 'list',
  selectable = false,
  selected = false,
  onSelectChange,
  selectDisabledReason,
  connectivityStatus,
  dragDisabled = false,
  onEdit,
  onCopy,
  onShare,
  onDelete,
  deleteDisabledReason,
  onAddModel,
  onEditModel,
  onCopyModel,
  onDeleteModel,
  onSetPrimaryModel,
  onReorderModels,
  modelSelectionMode = false,
  selectedModelIds = [],
  onToggleModelSelection,
  onToggleBatchDeleteMode,
  onBatchDeleteModels,
  onTestModels,
  testModelsDisabled = false,
  testModelsDisabledTooltip,
  onFetchModels,
  fetchDisabled = false,
  fetchDisabledTooltip,
}) => {
  const { t } = useTranslation();
  const providerConfig = provider.modelsProvider ?? {};
  const baseUrl = getStringField(providerConfig, 'baseUrl');
  const api = getStringField(providerConfig, 'api');
  const sdkName = piApiToSdkName(api) || api || provider.categories.join(', ') || 'pi';

  const modelRows = React.useMemo<ModelDisplayData[]>(
    () => getProviderModelRecords(providerConfig).map((entry) => ({
      id: entry.id,
      name: getStringField(entry.model, 'name') || entry.id,
      // Pi persists these as `contextWindow` / `maxTokens` (see the model editor
      // in `PiPage` and `buildFetchedPiModel`); `contextLimit` / `outputLimit`
      // are only the *form field* names, so reading them here always yielded
      // undefined and every row silently lost its limits.
      contextLimit: typeof entry.model.contextWindow === 'number' ? entry.model.contextWindow : undefined,
      outputLimit: typeof entry.model.maxTokens === 'number' ? entry.model.maxTokens : undefined,
      // Pi stores the pointer outside the provider record, so the page tells us
      // which provider owns it before a row may claim `当前主模型`.
      isPrimary: Boolean(defaultModelId) && defaultModelId === entry.id,
    })),
    [providerConfig, defaultModelId],
  );

  const modelSection: ProviderCardModels = {
    models: modelRows,
    modelsDraggable: !modelSelectionMode,
    onReorderModels,
    onSetPrimaryModel,
    modelSelectionMode,
    selectedModelIds,
    onToggleModelSelection,
    onToggleBatchDeleteMode,
    onBatchDeleteModels,
    onTestModels,
    testModelsDisabled,
    testModelsDisabledTooltip,
    onFetchModels,
    fetchDisabled,
    fetchDisabledTooltip,
    onAddModel,
    onEditModel,
    onCopyModel,
    onDeleteModel,
  };

  // Which runtime files produced this provider (auth.json / models.json). The
  // old card glued this into the endpoint fallback when there was no baseUrl,
  // which read as if the provider had an endpoint named "auth.json".
  const sourceTags = provider.sources.length > 0
    ? (
      <>
        {provider.sources.map((source) => (
          <Tag key={source} style={{ margin: 0 }}>
            {t(`pi.sourceLabels.${source}`, { defaultValue: source })}
          </Tag>
        ))}
      </>
    )
    : undefined;

  const props: ProviderCardVariantProps = {
    provider: {
      id: provider.providerKey,
      name: provider.displayName,
      sdkName,
      baseUrl,
    },
    providerState: {
      draggable: !selectable && !dragDisabled,
      sortableId: provider.providerKey,
      connectivityStatus,
      selectable,
      selected,
      onSelectChange,
      selectDisabledReason,
    },
    actions: {
      onEdit,
      onCopy,
      onShare,
      onDelete,
      // The page owns the confirmation: it asks which of {config, credential,
      // both} to remove, which the card's plain confirm cannot express.
      deleteConfirm: false,
      deleteDisabledReason,
    },
    nameTags: sourceTags,
    modelSection,
  };

  if (viewMode !== 'card') {
    return <OpenCodeStyleCard {...props} />;
  }

  const defaultModelName = defaultModelId
    ? modelRows.find((row) => row.id === defaultModelId)?.name ?? defaultModelId
    : undefined;

  // The grid cell only carries "more"; the destructive and copy actions live
  // here so the sidebar can do everything the list row can. `onDelete` is
  // absent for a provider that exists in neither runtime file, so the button is
  // skipped rather than drawn dead.
  const secondaryActions = (
    <Space size={4} wrap>
      {onEdit && (
        <Button size="small" type="text" icon={<EditOutlined />} onClick={onEdit}>
          {t('common.edit')}
        </Button>
      )}
      {onCopy && (
        <Button size="small" type="text" icon={<CopyOutlined />} onClick={onCopy}>
          {t('common.copy')}
        </Button>
      )}
      {onShare && (
        <Button size="small" type="text" icon={<ShareAltOutlined />} onClick={onShare}>
          {t('common.share')}
        </Button>
      )}
      {onDelete && (
        <Tooltip title={deleteDisabledReason}>
          <span>
            <Button
              size="small"
              type="text"
              danger
              icon={<DeleteOutlined />}
              disabled={Boolean(deleteDisabledReason)}
              onClick={() => {
                if (!deleteDisabledReason) {
                  onDelete();
                }
              }}
            >
              {t('common.delete')}
            </Button>
          </span>
        </Tooltip>
      )}
    </Space>
  );

  const detailSections: ProviderDetailSection[] = [{
    key: 'connection',
    title: t('common.provider.connectionTitle'),
    fields: [
      {
        key: 'baseUrl',
        label: t('pi.provider.baseUrl'),
        value: baseUrl,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
      },
      {
        key: 'apiType',
        label: t('pi.provider.apiType'),
        value: api,
        emptyText: t('common.provider.notConfigured'),
      },
      {
        key: 'sources',
        label: t('pi.provider.sourcesLabel'),
        value: provider.sources
          .map((source) => t(`pi.sourceLabels.${source}`, { defaultValue: source }))
          .join(' / '),
        emptyText: t('common.provider.notConfigured'),
      },
      // The old card used the masked credential as a stand-in for a missing
      // Base URL, which read as an endpoint. It is a fact of its own: an
      // auth-only channel has no endpoint, but it does have a key. The value
      // follows the same resolution as the outbound requests (auth.json first,
      // models.json second), so the sidebar and the test never disagree.
      {
        key: 'credential',
        label: t('pi.provider.providerApiKey'),
        value: maskProviderApiKey(provider),
        emptyText: t('common.provider.notConfigured'),
      },
    ],
  }, {
    key: 'catalog',
    content: (
      <ProviderModelSection
        providerId={provider.providerKey}
        modelSection={modelSection}
        appearance="detail"
      />
    ),
  }];

  return (
    <ProviderCompactCard
      provider={props.provider}
      subtitle={[
        { value: sdkName },
        ...(defaultModelName
          ? [{ label: t('common.model.currentPrimary'), value: defaultModelName }]
          : []),
      ]}
      actionPlacement="subtitle"
      providerState={props.providerState}
      actions={props.actions}
      nameTags={sourceTags}
      extraMenuItems={onTestModels ? [{
        key: 'model-test',
        label: t('pi.connectivity.button'),
        disabled: testModelsDisabled,
        onClick: onTestModels,
      }] : undefined}
      detail={{
        title: <ProviderNameLink name={props.provider.name} baseUrl={baseUrl} />,
        statusTags: sourceTags,
        sections: detailSections,
        secondaryActions,
      }}
    />
  );
};

export default PiProviderCard;
