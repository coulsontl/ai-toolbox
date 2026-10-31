import React from 'react';
import { useTranslation } from 'react-i18next';
import ProviderListSection from '@/features/coding/shared/ProviderListSection';
import {
  filterProviderItems,
  providerSelectionDisabledReason,
  sortProviderItems,
  useProviderBatchSelection,
  useProviderListSort,
  useProviderListViewPrefs,
} from '@/features/coding/shared/providerList';
import { ProviderCardGrid } from '@/features/coding/shared/providerCardView';
import type { WorkbuddyProviderView } from '@/types/workbuddy';
import ModelsJsonProviderCard from './ModelsJsonProviderCard';

export interface ModelsJsonProviderSectionProps {
  /** All channels, in file order. */
  providers: WorkbuddyProviderView[];
  /** Sort/list preferences are stored per module (`workbuddy` / `codebuddy`). */
  moduleKey: string;
  loading: boolean;
  /** Localized section hint; the shell owns its styling. */
  hint: React.ReactNode;
  /**
   * The toolbar's 「添加供应商」.
   *
   * A channel is derived from `(endpoint, api key)`, so creating one means
   * writing a first model entry that carries the connection — the page opens a
   * provider form for exactly that (SOP §4.2.1).
   */
  onAddProvider: () => void;
  /** Adds one more model to an existing channel. */
  onAddModel: (channel: WorkbuddyProviderView) => void;
  /** Opens the model modal; a `modelId` means that entry is being edited. */
  onEditChannel: (channel: WorkbuddyProviderView, modelId: string, options?: { copy?: boolean }) => void;
  /**
   * Rewrites the channel's connection across every entry (endpoint / key /
   * vendor).
   *
   * There is no channel record to edit — a channel is derived from
   * `(endpoint, key)` — so this applies one connection to the whole group,
   * which is the only way to move a channel elsewhere without editing each
   * model. The page opens the provider form for it.
   */
  onEditConnection: (channel: WorkbuddyProviderView) => void;
  onDeleteChannels: (channels: WorkbuddyProviderView[]) => Promise<boolean>;
  onDeleteModel: (channel: WorkbuddyProviderView, modelId: string) => void;
  /** Batch-removes several models of one channel after the page confirms. */
  onDeleteModels: (channel: WorkbuddyProviderView, modelIds: string[]) => Promise<void> | void;
  /** Runs the shared connectivity probe for one channel. */
  onTestModels: (channel: WorkbuddyProviderView) => void;
  /** Opens the shared "fetch models" dialog for one channel. */
  onFetchModels: (channel: WorkbuddyProviderView) => void;
}

/**
 * Sorting options offered here.
 *
 * `created` needs a creation timestamp and neither product's `models.json`
 * stores one; `recent` needs `provider_last_used`, which only an "apply"
 * action ever writes — and a channel has no apply action (that is the whole
 * point of the OpenCode-style card). Offering either would leave a menu entry
 * that provably never reorders anything, so only the two modes that work are
 * listed.
 */
const MODELS_JSON_SORT_MODES = ['custom', 'name'] as const;

/**
 * The channel list both WorkBuddy and CodeBuddy render.
 *
 * They share the whole `models.json` contract, so the list, its search/sort
 * state and its batch delete are identical; only the surrounding page (config
 * root, identity files vs `CODEBUDDY.md`) differs.
 */
const ModelsJsonProviderSection: React.FC<ModelsJsonProviderSectionProps> = ({
  providers,
  moduleKey,
  loading,
  hint,
  onAddProvider,
  onAddModel,
  onEditChannel,
  onEditConnection,
  onDeleteChannels,
  onDeleteModel,
  onDeleteModels,
  onTestModels,
  onFetchModels,
}) => {
  const { t } = useTranslation();

  const [collapsed, setCollapsed] = React.useState(false);
  const [keyword, setKeyword] = React.useState('');
  const { viewMode, setViewMode } = useProviderListViewPrefs(moduleKey);
  const { sortMode, setSortMode } = useProviderListSort(moduleKey);

  /**
   * Model-level batch delete, keyed by channel.
   *
   * One channel can be in selection mode at a time: each card renders its own
   * model list, and letting two cards collect selections at once would give the
   * user no way to tell which delete button acts on what.
   */
  const [modelSelectionChannelId, setModelSelectionChannelId] = React.useState<string | null>(null);
  const [selectedModelIds, setSelectedModelIds] = React.useState<string[]>([]);

  const exitModelSelection = React.useCallback(() => {
    setModelSelectionChannelId(null);
    setSelectedModelIds([]);
  }, []);

  const filtered = React.useMemo(
    () =>
      filterProviderItems(providers, keyword, (provider) => [
        provider.name,
        provider.baseUrl,
        provider.vendor,
        ...provider.models.map((model) => `${model.id} ${model.name}`),
      ]),
    [providers, keyword],
  );

  const visibleProviders = React.useMemo(() => {
    if (sortMode === 'custom') {
      return filtered;
    }
    return sortProviderItems(filtered, sortMode, {
      name: (provider) => provider.name || provider.baseUrl,
    });
  }, [filtered, sortMode]);

  const batchSelectableIds = React.useMemo(
    () => visibleProviders.map((provider) => provider.id),
    [visibleProviders],
  );

  const providerBatch = useProviderBatchSelection<string>({
    allIds: batchSelectableIds,
    onBatchDelete: async (ids) => {
      const channels = visibleProviders.filter((provider) => ids.includes(provider.id));
      return onDeleteChannels(channels);
    },
  });

  const renderCard = (provider: WorkbuddyProviderView) => (
    <ModelsJsonProviderCard
      provider={provider}
      viewMode={viewMode}
      selectable={providerBatch.selectionMode}
      selected={providerBatch.selectedIds.has(provider.id)}
      onSelectChange={(checked) => providerBatch.toggleSelect(provider.id, checked)}
      selectDisabledReason={providerSelectionDisabledReason(t, undefined)}
      onEdit={() => onEditConnection(provider)}
      onDelete={() => {
        void onDeleteChannels([provider]);
      }}
      onAddModel={() => onAddModel(provider)}
      onEditModel={(modelId) => onEditChannel(provider, modelId)}
      onCopyModel={(modelId) => onEditChannel(provider, modelId, { copy: true })}
      onDeleteModel={(modelId) => onDeleteModel(provider, modelId)}
      modelSelectionMode={modelSelectionChannelId === provider.id}
      selectedModelIds={modelSelectionChannelId === provider.id ? selectedModelIds : []}
      onToggleModelSelection={(modelId, selected) => {
        setSelectedModelIds((current) => selected
          ? (current.includes(modelId) ? current : [...current, modelId])
          : current.filter((id) => id !== modelId));
      }}
      onToggleBatchDeleteMode={() => {
        if (modelSelectionChannelId === provider.id) {
          exitModelSelection();
          return;
        }
        setModelSelectionChannelId(provider.id);
        setSelectedModelIds([]);
      }}
      onBatchDeleteModels={() => {
        if (selectedModelIds.length === 0) {
          return;
        }
        const deleting = [...selectedModelIds];
        void Promise.resolve(onDeleteModels(provider, deleting)).then(exitModelSelection);
      }}
      onTestModels={() => onTestModels(provider)}
      onFetchModels={() => onFetchModels(provider)}
      // `models.json` writes one URL per entry, so a channel that mixes
      // `useCustomProtocol` entries cannot be probed against `{base}/models`.
      fetchDisabled={provider.models.some((model) => model.useCustomProtocol)}
      fetchDisabledTooltip={t('workbuddy.providerModal.fetchDisabledCustomProtocol')}
    />
  );

  return (
    <ProviderListSection
      // The page already marks this block with `id="<module>-providers"` for
      // the sidebar, and two nodes cannot share an id. Hermes solved the same
      // collision the same way: the inner shell gets the `-section` suffix.
      sectionId={`${moduleKey}-providers-section`}
      collapsed={collapsed}
      onCollapsedChange={setCollapsed}
      loading={loading}
      providerCount={providers.length}
      visibleCount={visibleProviders.length}
      batch={providerBatch}
      batchSelectableIds={batchSelectableIds}
      keyword={keyword}
      onKeywordChange={setKeyword}
      sortMode={sortMode}
      sortModes={MODELS_JSON_SORT_MODES}
      onSortModeChange={setSortMode}
      viewMode={viewMode}
      onViewModeChange={setViewMode}
      // No `dragDisabledBySort`: that hint says "drag is available under 默认
      // sort", which is false here — a channel has no stored position at all
      // (`models.json` is a flat list and the product never reads a channel
      // order back). The cards simply carry no grip, which is accurate.
      onAddProvider={onAddProvider}
      hint={hint}
    >
      {viewMode === 'card' ? (
        <ProviderCardGrid
          items={visibleProviders}
          getKey={(provider) => provider.id}
          renderItem={(provider) => renderCard(provider)}
        />
      ) : (
        <div>
          {visibleProviders.map((provider) => (
            <React.Fragment key={provider.id}>{renderCard(provider)}</React.Fragment>
          ))}
        </div>
      )}
    </ProviderListSection>
  );
};

export default ModelsJsonProviderSection;
