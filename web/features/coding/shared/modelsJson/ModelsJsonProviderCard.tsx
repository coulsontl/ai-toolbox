import React from 'react';
import { Button, Space, Tag } from 'antd';
import { DeleteOutlined, EditOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
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
import type { ModelDisplayData } from '@/components/common/ProviderCard/types';
import ProviderNameLink from '@/components/common/ProviderNameLink';
import type { WorkbuddyProviderView } from '@/types/workbuddy';

export interface ModelsJsonProviderCardProps {
  provider: WorkbuddyProviderView;
  /** Layout the user picked; the same data renders as either. */
  viewMode?: ProviderViewMode;
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (selected: boolean) => void;
  selectDisabledReason?: string;

  /**
   * Rewrites the channel's connection (endpoint / key / vendor).
   *
   * The file has no channel record, so this is not "edit the provider" — it is
   * "apply one connection to every model entry of this channel", which is the
   * only way to move a channel to a different endpoint without editing its
   * models one at a time.
   */
  onEdit?: () => void;
  onDelete?: () => void;
  onAddModel?: () => void;
  onEditModel?: (modelId: string) => void;
  onCopyModel?: (modelId: string) => void;
  onDeleteModel?: (modelId: string) => void;

  modelSelectionMode?: boolean;
  selectedModelIds?: string[];
  onToggleModelSelection?: (modelId: string, selected: boolean) => void;
  onToggleBatchDeleteMode?: () => void;
  onBatchDeleteModels?: () => void;

  onTestModels?: () => void;
  onFetchModels?: () => void;
  fetchDisabled?: boolean;
  fetchDisabledTooltip?: string;
}

/**
 * A WorkBuddy / CodeBuddy channel, in both layouts.
 *
 * `models.json` is a flat model list with no provider concept, so a card is a
 * view over entries grouped by `(endpoint, api key)`. That is the OpenCode
 * shape: several channels are usable at once and none of them is "the active
 * one", so the card carries no apply action.
 *
 * The two layouts differ in density, not in what the CLI can do:
 *
 * - list → `OpenCodeStyleCard`, the full row with its model fold
 * - card → `ProviderCompactCard`, one fact per cell, with the model manager in
 *   that cell's detail sidebar
 *
 * Rendering the list card inside the grid was the earlier mistake: its toolbar
 * carries five text buttons, which a 350px cell cannot hold, so 「模型列表」
 * broke into one character per line and 添加模型 was pushed out of the header.
 *
 * Deliberately absent, each for a structural reason rather than an omission:
 *
 * - `onSetPrimaryModel` — `models.json` has no default-model pointer. The only
 *   candidate, `availableModels`, is a *whitelist*: writing it hides every
 *   built-in model the product ships with, which is the opposite of "pick a
 *   default".
 * - `modelsDraggable` / `onReorderModels` — the product never reads a channel or
 *   model order back out of `models.json`.
 *
 * The two products are two front ends over one engine and share the whole
 * `models.json` contract, so this single mapper serves both pages.
 */
const ModelsJsonProviderCard: React.FC<ModelsJsonProviderCardProps> = ({
  provider,
  viewMode = 'list',
  selectable = false,
  selected = false,
  onSelectChange,
  selectDisabledReason,
  onEdit,
  onDelete,
  onAddModel,
  onEditModel,
  onCopyModel,
  onDeleteModel,
  modelSelectionMode,
  selectedModelIds,
  onToggleModelSelection,
  onToggleBatchDeleteMode,
  onBatchDeleteModels,
  onTestModels,
  onFetchModels,
  fetchDisabled,
  fetchDisabledTooltip,
}) => {
  const { t } = useTranslation();

  const modelRows = React.useMemo<ModelDisplayData[]>(
    () =>
      provider.models.map((model) => ({
        id: model.id,
        name: model.name || model.id,
        contextLimit: model.maxInputTokens,
        outputLimit: model.maxOutputTokens,
      })),
    [provider.models],
  );

  const modelSection: ProviderCardModels = {
    models: modelRows,
    onAddModel,
    onEditModel,
    onCopyModel,
    onDeleteModel,
    modelSelectionMode,
    selectedModelIds,
    onToggleModelSelection,
    onToggleBatchDeleteMode,
    onBatchDeleteModels,
    onTestModels,
    // Testing needs a model to call; an empty channel would open an empty
    // dialog.
    testModelsDisabled: provider.models.length === 0,
    onFetchModels,
    fetchDisabled,
    fetchDisabledTooltip,
  };

  const nameTags = provider.vendor
    ? <Tag style={{ margin: 0 }}>{provider.vendor}</Tag>
    : undefined;

  const props: ProviderCardVariantProps = {
    provider: {
      id: provider.id,
      name: provider.name || provider.baseUrl,
      baseUrl: provider.baseUrl,
    },
    providerState: {
      // A channel has no stored position: `models.json` is a flat list and the
      // product never reads a channel order back, so a grip would drag nothing.
      draggable: false,
      sortableId: provider.id,
      selectable,
      selected,
      onSelectChange,
      selectDisabledReason,
    },
    actions: {
      onEdit,
      onDelete,
      deleteConfirm: false,
    },
    nameTags,
    modelSection,
  };

  if (viewMode !== 'card') {
    return <OpenCodeStyleCard {...props} />;
  }

  // The cell states the endpoint (the channel's identity) and how many models
  // it routes; everything else, the model manager included, lives in the
  // sidebar. `ProviderModelSection` is the *same* component the list card
  // renders, so the two layouts cannot offer different model operations.
  const detailSections: ProviderDetailSection[] = [
    {
      key: 'connection',
      title: t('common.provider.connectionTitle'),
      fields: [
        {
          key: 'baseUrl',
          label: t('workbuddy.model.baseUrl'),
          value: provider.baseUrl,
          emptyText: t('common.provider.notConfigured'),
          copyable: true,
        },
        {
          // Plaintext, like the list card and the model form: this app is a
          // configuration manager, so a credential is shown and editable rather
          // than masked.
          key: 'apiKey',
          label: t('workbuddy.model.apiKey'),
          value: provider.apiKey,
          emptyText: t('common.provider.notConfigured'),
          copyable: true,
          elideMiddle: true,
        },
      ],
    },
    {
      key: 'catalog',
      content: (
        <ProviderModelSection
          providerId={provider.id}
          modelSection={modelSection}
          appearance="detail"
        />
      ),
    },
  ];

  const secondaryActions = (
    <Space size={4} wrap>
      {onEdit && (
        <Button size="small" type="text" icon={<EditOutlined />} onClick={onEdit}>
          {t('common.edit')}
        </Button>
      )}
      {onDelete && (
        <Button size="small" type="text" danger icon={<DeleteOutlined />} onClick={onDelete}>
          {t('common.delete')}
        </Button>
      )}
    </Space>
  );

  return (
    <ProviderCompactCard
      provider={props.provider}
      subtitle={[
        { value: provider.baseUrl },
        { value: t('workbuddy.provider.modelCount', { count: provider.models.length }) },
      ]}
      actionPlacement="subtitle"
      providerState={props.providerState}
      actions={props.actions}
      nameTags={nameTags}
      extraMenuItems={onTestModels
        ? [{
          key: 'model-test',
          label: t('opencode.connectivity.button'),
          disabled: modelSection.testModelsDisabled,
          onClick: onTestModels,
        }]
        : undefined}
      detail={{
        title: <ProviderNameLink name={props.provider.name} baseUrl={provider.baseUrl} />,
        statusTags: nameTags,
        sections: detailSections,
        secondaryActions,
      }}
    />
  );
};

export default ModelsJsonProviderCard;
