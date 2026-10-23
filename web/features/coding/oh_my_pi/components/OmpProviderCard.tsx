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
import type { OmpRuntimeProviderView } from '@/types/ohMyPi';
import { getProviderModelRecords } from '@/utils/ompModelMetadata';
import { getOmpDiagnostics } from '../utils/ompDiagnostics';
import { ompApiToSdkName } from '../utils/ompFetchedModels';
import { getStringField } from '../utils/ompProviderConfig';

interface OmpProviderCardProps {
  provider: OmpRuntimeProviderView;
  /** The model `modelRoles.default` starts sessions with, when this provider owns it. */
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
  /** `openai-codex` is managed through its OAuth account block, not this form. */
  isSubscription?: boolean;

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
  /** Shown on the test button even when it is enabled (see `ModelListSection`). */
  testModelsHint?: string;
  onFetchModels?: () => void;
  fetchDisabled?: boolean;
  fetchDisabledTooltip?: string;
}

/**
 * An Oh My Pi provider, rendered in the OpenCode style.
 *
 * OMP keeps a per-provider catalog in `models.yml` and has no provider-level
 * "apply": `modelRoles.default` names one model inside one route, and
 * `设为默认` on a model row moves that pointer. That is the OpenCode shape.
 *
 * The connectivity and model-discovery gates come from `getOmpDiagnostics`:
 * OMP lets each model declare its own protocol, so "can this provider be
 * tested" is a per-model question and the toolbar button must carry the reason
 * it is greyed out rather than silently doing nothing.
 *
 * `openai-codex` is a subscription channel managed by `OmpSubscriptionCard`;
 * this card still renders it when it also has a `models.yml` entry, but without
 * the edit/copy/share actions that would create a second OAuth route.
 */
const OmpProviderCard: React.FC<OmpProviderCardProps> = ({
  provider,
  defaultModelId,
  viewMode = 'list',
  selectable = false,
  selected = false,
  onSelectChange,
  selectDisabledReason,
  connectivityStatus,
  dragDisabled = false,
  isSubscription = false,
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
  testModelsHint,
  onFetchModels,
  fetchDisabled = false,
  fetchDisabledTooltip,
}) => {
  const { t } = useTranslation();
  const providerConfig = provider.modelsProvider ?? {};
  const baseUrl = getStringField(providerConfig, 'baseUrl');
  const api = getStringField(providerConfig, 'api');
  const diagnostics = getOmpDiagnostics(providerConfig);
  const sourceTags = provider.sources.length > 0
    ? (
      <>
        {provider.sources.map((source) => (
          <Tag key={source} style={{ margin: 0 }}>
            {t(`ohMyPi.sourceLabels.${source}`, { defaultValue: source })}
          </Tag>
        ))}
      </>
    )
    : undefined;

  const modelRows = React.useMemo<ModelDisplayData[]>(
    () => getProviderModelRecords(provider.modelsProvider).map((entry) => ({
      id: entry.id,
      name: getStringField(entry.model, 'name') || entry.id,
      contextLimit: typeof entry.model.contextWindow === 'number' ? entry.model.contextWindow : undefined,
      outputLimit: typeof entry.model.maxTokens === 'number' ? entry.model.maxTokens : undefined,
      // Only the provider owning the global default marks a row: OMP stores the
      // pointer in `config.yml`'s `modelRoles.default`, not on the route.
      isPrimary: Boolean(defaultModelId) && defaultModelId === entry.id,
    })),
    [provider.modelsProvider, defaultModelId],
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
    testModelsHint,
    onFetchModels,
    fetchDisabled,
    fetchDisabledTooltip,
    onAddModel,
    onEditModel,
    onCopyModel,
    onDeleteModel,
  };

  const props: ProviderCardVariantProps = {
    provider: {
      id: provider.providerKey,
      // The card only renders for a subscription route that *also* has a
      // `models.yml` entry (otherwise `OmpSubscriptionCard` is the whole
      // member). That combination is the override case, and the suffix is what
      // tells the user this row is not the plain OAuth channel.
      name: isSubscription
        ? `${provider.displayName} · ${t('ohMyPi.subscription.yamlOverride')}`
        : provider.displayName,
      sdkName: ompApiToSdkName(api) || api || provider.categories.join(', ') || 'omp',
      baseUrl,
    },
    providerState: {
      // The OAuth route is a fixed list member (it always exists); its own
      // handle is rendered by `OmpSubscriptionCard`, not duplicated here.
      draggable: !isSubscription && !selectable && !dragDisabled,
      sortableId: provider.providerKey,
      connectivityStatus: diagnostics.supportsConnectivity ? connectivityStatus : undefined,
      selectable,
      selected,
      onSelectChange,
      selectDisabledReason,
    },
    actions: {
      // Editing the `models.yml` entry is safe for the OAuth route too — it is
      // copy / share that would mint a second OAuth route, so only those two
      // are withheld.
      onEdit: onEdit,
      onCopy: isSubscription ? undefined : onCopy,
      onShare: isSubscription ? undefined : onShare,
      onDelete,
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

  // The grid cell only carries "more"; the sidebar carries the rest. The
  // subscription route has no copy / share handler (see `props.actions`
  // above), so those buttons drop out here for the same reason.
  const secondaryActions = (
    <Space size={4} wrap>
      {onEdit && (
        <Button size="small" type="text" icon={<EditOutlined />} onClick={onEdit}>
          {t('common.edit')}
        </Button>
      )}
      {onCopy && !isSubscription && (
        <Button size="small" type="text" icon={<CopyOutlined />} onClick={onCopy}>
          {t('common.copy')}
        </Button>
      )}
      {onShare && !isSubscription && (
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
        label: t('ohMyPi.provider.baseUrl'),
        value: baseUrl,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
      },
      {
        key: 'apiType',
        label: t('ohMyPi.provider.apiType'),
        value: api,
        emptyText: t('common.provider.notConfigured'),
      },
      {
        key: 'sources',
        label: t('ohMyPi.provider.sourcesLabel'),
        value: provider.sources
          .map((source) => t(`ohMyPi.sourceLabels.${source}`, { defaultValue: source }))
          .join(' / '),
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
        { value: props.provider.sdkName ?? 'omp' },
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
        label: t('ohMyPi.connectivity.button'),
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

export default OmpProviderCard;
