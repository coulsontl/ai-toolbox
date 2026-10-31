import React from 'react';
import { Tag } from 'antd';
import OpenCodeStyleCard from '@/features/coding/shared/providerCardVariants/OpenCodeStyleCard';
import type {
  ProviderCardModels,
  ProviderCardVariantProps,
} from '@/features/coding/shared/providerCardVariants/types';
import type { ModelDisplayData } from '@/components/common/ProviderCard/types';
import type { WorkbuddyProviderView } from '@/types/workbuddy';

export interface ModelsJsonProviderCardProps {
  provider: WorkbuddyProviderView;
  selectable?: boolean;
  selected?: boolean;
  onSelectChange?: (selected: boolean) => void;
  selectDisabledReason?: string;

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
 * A WorkBuddy / CodeBuddy channel, rendered in the OpenCode style.
 *
 * `models.json` is a flat model list with no provider concept, so a card is a
 * view over entries grouped by `(endpoint, api key)`. That is the OpenCode
 * shape: several channels are usable at once and none of them is "the active
 * one", so the card carries no apply action.
 *
 * Deliberately absent, and each for a structural reason rather than an omission:
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
  selectable = false,
  selected = false,
  onSelectChange,
  selectDisabledReason,
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
    // Testing needs an endpoint to call; a channel always has one (it is the
    // grouping key), but an empty model list would open an empty dialog.
    testModelsDisabled: provider.models.length === 0,
    onFetchModels,
    fetchDisabled,
    fetchDisabledTooltip,
  };

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
      onDelete,
      deleteConfirm: false,
    },
    nameTags: provider.vendor
      ? <Tag style={{ margin: 0 }}>{provider.vendor}</Tag>
      : undefined,
    modelSection,
  };

  return <OpenCodeStyleCard {...props} />;
};

export default ModelsJsonProviderCard;
