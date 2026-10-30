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
  onAddModel: (channel: WorkbuddyProviderView | null) => void;
  /** Opens the model modal; a `modelId` means that entry is being edited. */
  onEditChannel: (channel: WorkbuddyProviderView, modelId: string) => void;
  onDeleteChannels: (channels: WorkbuddyProviderView[]) => Promise<boolean>;
  onDeleteModel: (channel: WorkbuddyProviderView, modelId: string) => void;
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
  onAddModel,
  onEditChannel,
  onDeleteChannels,
  onDeleteModel,
}) => {
  const { t } = useTranslation();

  const [collapsed, setCollapsed] = React.useState(false);
  const [keyword, setKeyword] = React.useState('');
  const { viewMode, setViewMode } = useProviderListViewPrefs(moduleKey);
  const { sortMode, setSortMode } = useProviderListSort(moduleKey);

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
      onAddProvider={() => onAddModel(null)}
      hint={hint}
    >
      <div
        style={{
          display: 'grid',
          gridTemplateColumns:
            viewMode === 'card' ? 'repeat(auto-fill, minmax(320px, 1fr))' : '1fr',
          gap: 12,
        }}
      >
        {visibleProviders.map((provider) => (
          <ModelsJsonProviderCard
            key={provider.id}
            provider={provider}
            selectable={providerBatch.selectionMode}
            selected={providerBatch.selectedIds.has(provider.id)}
            onSelectChange={(checked) => providerBatch.toggleSelect(provider.id, checked)}
            selectDisabledReason={providerSelectionDisabledReason(t, undefined)}
            // No card-level edit action: a channel is *derived* from
            // `(url, apiKey)`, there is no channel record to edit, and an
            // "edit channel" entry could only open an empty "add model" form
            // (which then refuses to submit without an id). The channel's
            // endpoint is corrected by editing one of its models, which is what
            // the per-model pencil does.
            onDelete={() => {
              void onDeleteChannels([provider]);
            }}
            onAddModel={() => onAddModel(provider)}
            onEditModel={(modelId) => onEditChannel(provider, modelId)}
            onDeleteModel={(modelId) => onDeleteModel(provider, modelId)}
          />
        ))}
      </div>
    </ProviderListSection>
  );
};

export default ModelsJsonProviderSection;
