import React from 'react';
import { Button, Space, Tag, Tooltip } from 'antd';
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
import type { HermesRuntimeProviderView } from '@/types/hermes';
import {
  getNumberField,
  getProviderModelRecords,
  getStringField,
  hermesApiModeToSdkName,
} from '../utils/hermesUtils';

interface HermesProviderCardProps {
  provider: HermesRuntimeProviderView;
  /** The model `model.defaultModel` starts sessions with, when this provider owns it. */
  defaultModelId?: string;
  /** `'card'` renders the compact grid cell plus the detail sidebar. */
  viewMode?: ProviderViewMode;
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (selected: boolean) => void;
  connectivityStatus?: ProviderConnectivityStatusItem;

  onEdit?: () => void;
  onCopy?: () => void;
  onShare?: () => void;
  onDelete?: () => void;
  /** Disables the delete button and explains why on hover (the default provider). */
  deleteDisabledReason?: string;

  /** Model catalog actions. Omit a handler to hide its button. */
  onAddModel?: () => void;
  onEditModel?: (modelId: string) => void;
  onCopyModel?: (modelId: string) => void;
  onDeleteModel?: (modelId: string) => void;
  onSetPrimaryModel?: (modelId: string) => void;
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
 * A Hermes provider, rendered in the OpenCode style.
 *
 * Hermes keeps a per-provider model catalog in `config.yaml` and has no
 * provider-level "apply": `model.defaultProvider` + `model.defaultModel` pick
 * one model inside one channel, and `设为默认` on a model row moves that
 * pointer. That is the OpenCode shape.
 *
 * Read-only providers (the built-in `providers:` dict) get no edit, copy,
 * delete or catalog actions — the YAML is owned by the engine. They still get
 * the read-only tag, which used to be glued into the endpoint fallback text.
 */
const HermesProviderCard: React.FC<HermesProviderCardProps> = ({
  provider,
  defaultModelId,
  viewMode = 'list',
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
  onSetPrimaryModel,
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
  const providerConfig = provider.provider ?? {};
  const baseUrl = getStringField(providerConfig, 'base_url')
    || getStringField(providerConfig, 'baseUrl')
    || '';
  const apiKey = getStringField(providerConfig, 'api_key')
    || getStringField(providerConfig, 'apiKey')
    || '';

  const modelRows = React.useMemo<ModelDisplayData[]>(
    () => getProviderModelRecords(provider.provider).map((entry) => ({
      id: entry.id,
      name: getStringField(entry.model, 'name') || entry.id,
      contextLimit: getNumberField(entry.model, 'context_length'),
      outputLimit: getNumberField(entry.model, 'max_tokens'),
      // Only the provider that owns the global default marks a row; Hermes
      // stores the pointer on `model:`, not on each provider entry.
      isPrimary: Boolean(defaultModelId) && defaultModelId === entry.id,
    })),
    [provider.provider, defaultModelId],
  );

  const modelSection: ProviderCardModels = {
    models: modelRows,
    // Hermes has no reorder command: the catalog is a YAML dict and the engine
    // never reads it back in a meaningful order, so the grip would drag
    // nothing. The bespoke card rendered one anyway.
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

  // Only the read-only tag is new furniture here, and it replaces the old
  // `(只读)` suffix glued onto the endpoint fallback. "Which provider is the
  // default" stays where it always was: the model row's `当前主模型`, not a
  // second statement on the card.
  const nameTags = provider.isReadOnly
    ? <Tag style={{ margin: 0 }}>{t('hermes.provider.readOnly')}</Tag>
    : undefined;

  const props: ProviderCardVariantProps = {
    provider: {
      id: provider.providerKey,
      name: provider.displayName || provider.providerKey,
      sdkName: hermesApiModeToSdkName(provider.apiMode),
      baseUrl,
    },
    providerState: {
      draggable: !selectable,
      sortableId: provider.providerKey,
      connectivityStatus,
      selectable,
      selected,
      onSelectChange,
    },
    actions: {
      onEdit,
      onCopy,
      onShare,
      // A read-only provider has no delete path at all; one that is the default
      // keeps the button but explains why it refuses.
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

  const defaultModelName = defaultModelId
    ? modelRows.find((row) => row.id === defaultModelId)?.name ?? defaultModelId
    : undefined;
  const sdkName = hermesApiModeToSdkName(provider.apiMode);

  const detailSections: ProviderDetailSection[] = [{
    key: 'connection',
    title: t('common.provider.connectionTitle'),
    fields: [
      {
        key: 'baseUrl',
        label: t('hermes.provider.baseUrl'),
        value: baseUrl,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
      },
      {
        key: 'apiMode',
        label: t('hermes.provider.apiMode'),
        value: provider.apiMode,
        emptyText: t('common.provider.notConfigured'),
      },
      // No `emptyText` on the key: a read-only built-in authenticates inside
      // the engine, so there is no static key to show and no row to leave out.
      ...(provider.isReadOnly ? [] : [{
        key: 'apiKey',
        label: t('hermes.provider.apiKey'),
        value: apiKey,
        emptyText: t('common.provider.notConfigured'),
        copyable: true,
        elideMiddle: true,
      }]),
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
      nameTags={nameTags}
      extraMenuItems={onTestModels ? [{
        key: 'model-test',
        label: t('opencode.connectivity.button'),
        disabled: testModelsDisabled,
        onClick: onTestModels,
      }] : undefined}
      detail={{
        title: <ProviderNameLink name={props.provider.name} baseUrl={baseUrl} />,
        statusTags: nameTags,
        sections: detailSections,
        secondaryActions: (
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
              <Button size="small" type="text" icon={<Share2 size={14} />} onClick={onShare}>
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
                    onClick={onDelete}
                  >
                    {t('common.delete')}
                  </Button>
                </span>
              </Tooltip>
            )}
          </Space>
        ),
      }}
    />
  );
};

export default HermesProviderCard;
