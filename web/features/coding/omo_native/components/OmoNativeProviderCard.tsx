import React from 'react';
import { useTranslation } from 'react-i18next';
import OpenCodeStyleCard from '@/features/coding/shared/providerCardVariants/OpenCodeStyleCard';
import type { ProviderCardVariantProps } from '@/features/coding/shared/providerCardVariants/types';
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
  /**
   * Hides the drag handle when reordering is not available (non-`custom` sort
   * mode, or an active search) — a handle that cannot move anything is worse
   * than no handle.
   */
  dragDisabled?: boolean;
  /** Latest result of the batch connectivity test for this provider. */
  connectivityStatus?: ProviderConnectivityStatusItem;

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
 * The layout lives in the shared variant; this file only maps Native's storage
 * shape (`models.json` + `auth.json`) onto it.
 */
const OmoNativeProviderCard: React.FC<OmoNativeProviderCardProps> = ({
  provider,
  onEdit,
  onCopy,
  onDelete,
  deleteDisabledReason,
  selectable = false,
  selected = false,
  onSelectChange,
  dragDisabled = false,
  connectivityStatus,
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

  const props: ProviderCardVariantProps = {
    provider: {
      id: provider.key,
      name: getStringField(config, 'name') || provider.key,
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

  return <OpenCodeStyleCard {...props} />;
};

export default OmoNativeProviderCard;
