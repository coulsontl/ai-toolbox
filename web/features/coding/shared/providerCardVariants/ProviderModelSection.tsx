import React from 'react';
import ModelListSection from '@/features/coding/shared/ModelListSection';
import type { ProviderCardModels } from './types';

interface ProviderModelSectionProps {
  /** Owner of the rows; the section's collapse state is keyed by it. */
  providerId: string;
  modelSection?: ProviderCardModels;
  /**
   * Which host is drawing the section — the list card (the default) or the
   * detail sidebar. The same `modelSection` data is rendered by both, so this is
   * the only thing that differs: the sidebar shows the rows open, the card keeps
   * them behind their fold. See `ModelListSection.appearance`.
   */
  appearance?: 'card' | 'detail';
}

/**
 * A card's model catalog, rendered once.
 *
 * The mapping-layer data (`ProviderCardModels`) and `ModelListSection`'s own
 * prop names are deliberately different — `modelSelectionMode` vs
 * `selectionMode`, `onBatchDeleteModels` vs `onBatchDelete` — and the mapping
 * was spelled out twice, in `CodexStyleCard` and `OpenCodeStyleCard`, with one
 * entry of drift between them (`onToggleModelDisabled`, which only the OpenCode
 * one forwarded). A third copy was about to appear in the card view's detail
 * sidebar, so the mapping moved here instead.
 *
 * `transparentRows` is not a prop: every consumer wants the transparent
 * collapse, because the section always sits on a surface that already has a
 * background (a card, or the sidebar).
 */
const ProviderModelSection: React.FC<ProviderModelSectionProps> = ({
  providerId,
  modelSection,
  appearance,
}) => {
  if (!modelSection) {
    return null;
  }

  return (
    <ModelListSection
      models={modelSection.models}
      rowKeyOf={modelSection.rowKeyOf}
      sectionKey={`provider-models-${providerId}`}
      appearance={appearance}
      className={modelSection.className}
      bodyStyle={modelSection.bodyStyle}
      transparentRows
      modelsDraggable={modelSection.modelsDraggable}
      onReorderModels={modelSection.onReorderModels}
      selectionMode={modelSection.modelSelectionMode}
      selectedIds={modelSection.selectedModelIds}
      onToggleSelection={modelSection.onToggleModelSelection}
      onToggleBatchDeleteMode={modelSection.onToggleBatchDeleteMode}
      onBatchDelete={modelSection.onBatchDeleteModels}
      onTest={modelSection.onTestModels}
      testDisabled={modelSection.testModelsDisabled}
      testDisabledTooltip={modelSection.testModelsDisabledTooltip}
      testHint={modelSection.testModelsHint}
      onFetchModels={modelSection.onFetchModels}
      fetchDisabled={modelSection.fetchDisabled}
      fetchDisabledTooltip={modelSection.fetchDisabledTooltip}
      onAddModel={modelSection.onAddModel}
      onEditModel={modelSection.onEditModel}
      onCopyModel={modelSection.onCopyModel}
      onDeleteModel={modelSection.onDeleteModel}
      onSetPrimaryModel={modelSection.onSetPrimaryModel}
      onToggleModelDisabled={modelSection.onToggleModelDisabled}
      renderModelExtraActions={modelSection.renderModelExtraActions}
      aboveList={modelSection.aboveList}
    />
  );
};

export default ProviderModelSection;