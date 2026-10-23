import React from 'react';
import { Button, Collapse, Empty, Space, Tooltip, Typography } from 'antd';
import {
  CloudDownloadOutlined,
  DeleteOutlined,
  PlusOutlined,
  ApiOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import {
  arrayMove,
  SortableContext,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import ModelItem from '@/components/common/ModelItem';
import type { ModelDisplayData } from '@/components/common/ProviderCard/types';
import styles from './ModelListSection.module.less';

const { Text } = Typography;

export interface ModelListSectionProps {
  /** i18n prefix for `model.*` labels. */

  /** Models rendered as rows. Callers map their own shape to `ModelDisplayData`. */
  models: ModelDisplayData[];

  /**
   * Row identity for selection / reorder / drag. Defaults to `model.id`.
   * Providers whose rows are keyed by more than the model id (Codex keys on
   * `model + displayName`) pass a resolver so callbacks receive their own key.
   */
  rowKeyOf?: (model: ModelDisplayData) => string;

  /**
   * Keys the section's collapse. Two of them are on screen whenever several
   * providers each carry a catalog.
   *
   * The detail form also publishes it as `data-model-list-section`, which is how
   * a browser check finds the sidebar's list without also matching the folded
   * ones behind the cards' headers. The card form carries no such attribute: it
   * is reachable through its own card's `data-provider-id`.
   */
  sectionKey: string;

  /**
   * Which host drew the section.
   *
   * `'card'` (the default) is the provider list card's: the rows live behind a
   * fold, whose header is the toolbar above them. That fold is what keeps a card
   * with eight models the same height as its neighbours.
   *
   * `'detail'` is the detail sidebar's: the rows are the whole point of the
   * block there, and a fold would put a header in front of the one thing the
   * reader opened the sidebar to see (user-directed, 2026-10-12). The toolbar
   * then reads as a title for the list under it, and the rows are drawn at the
   * tighter `appearance="detail"`.
   */
  appearance?: 'card' | 'detail';

  /** Extra class on the section root, e.g. to scope a transparent-background override. */
  className?: string;

  /** Style applied to the rows wrapper, on top of the fold's own indent. */
  bodyStyle?: React.CSSProperties;

  /** Rows render with a transparent fill so a tinted parent card shows through. */
  transparentRows?: boolean;

  /** Batch-delete selection mode. */
  selectionMode?: boolean;
  selectedIds?: string[];
  onToggleSelection?: (modelId: string, selected: boolean) => void;
  onToggleBatchDeleteMode?: () => void;
  onBatchDelete?: () => void;

  onTest?: () => void;
  testDisabled?: boolean;
  testDisabledTooltip?: string;
  /**
   * Shown on the test button even when it is enabled: a caveat about *how* the
   * test runs (OMP tests each model through its own connection). The disabled
   * tooltip answers "why can't I click"; this answers "what will clicking do".
   */
  testHint?: string;
  onFetchModels?: () => void;
  fetchDisabled?: boolean;
  fetchDisabledTooltip?: string;
  onAddModel?: () => void;

  onEditModel?: (modelId: string) => void;
  onCopyModel?: (modelId: string) => void;
  onDeleteModel?: (modelId: string) => void;
  onSetPrimaryModel?: (modelId: string) => void;
  /**
   * Flips a model's enabled flag. Omit for CLIs that cannot disable a single
   * model — the row then shows no switch rather than a dead one.
   */
  onToggleModelDisabled?: (modelId: string, isDisabled: boolean) => void;
  /**
   * Per-row extra action (e.g. Codex "set as auto-review model").
   *
   * `rowKey` is the same key the row was rendered with, so callers can hand it
   * straight back to their own handlers. Do not try to re-derive it from
   * `model` — a provider whose display name falls back to the upstream id makes
   * that ambiguous, and the wrong row gets acted on.
   */
  renderModelExtraActions?: (model: ModelDisplayData, rowKey: string) => React.ReactNode;

  modelsDraggable?: boolean;
  onReorderModels?: (modelIds: string[]) => void;

  /** Rendered directly under the toolbar, above the rows (e.g. the auto-review line). */
  aboveList?: React.ReactNode;
}

/**
 * Shared model-list block for providers that own a model catalog (the Codex
 * shape): a toolbar (batch delete / test / fetch / add) over the rows, which
 * render through `ModelItem` (edit / copy / delete).
 *
 * Providers without a catalog (the Claude Code shape) simply do not render it.
 */
const ModelListSection: React.FC<ModelListSectionProps> = ({
  models,
  rowKeyOf = (model) => model.id,
  sectionKey,
  appearance = 'card',
  className,
  bodyStyle,
  transparentRows = false,
  selectionMode = false,
  selectedIds = [],
  onToggleSelection,
  onToggleBatchDeleteMode,
  onBatchDelete,
  onTest,
  testDisabled = false,
  testDisabledTooltip,
  testHint,
  onFetchModels,
  fetchDisabled = false,
  fetchDisabledTooltip,
  onAddModel,
  onEditModel,
  onCopyModel,
  onDeleteModel,
  onSetPrimaryModel,
  onToggleModelDisabled,
  renderModelExtraActions,
  modelsDraggable = false,
  onReorderModels,
  aboveList,
}) => {
  const { t } = useTranslation();

  const actionButtonStyle: React.CSSProperties = { fontSize: 12 };

  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) {
      return;
    }
    const oldIndex = models.findIndex((model) => rowKeyOf(model) === active.id);
    const newIndex = models.findIndex((model) => rowKeyOf(model) === over.id);
    if (oldIndex < 0 || newIndex < 0) {
      return;
    }
    onReorderModels?.(arrayMove(models, oldIndex, newIndex).map((model) => rowKeyOf(model)));
  };

  const canReorder = modelsDraggable && !selectionMode && Boolean(onReorderModels);

  const modelRows = models.map((model) => {
    const rowKey = rowKeyOf(model);
    return (
      <ModelItem
        key={rowKey}
        model={model}
        draggable={canReorder}
        sortableId={rowKey}
        appearance={appearance}
        transparentBackground={transparentRows}
        onEdit={onEditModel ? () => onEditModel(rowKey) : undefined}
        onCopy={onCopyModel ? () => onCopyModel(rowKey) : undefined}
        onDelete={onDeleteModel ? () => onDeleteModel(rowKey) : undefined}
        onSetPrimary={onSetPrimaryModel ? () => onSetPrimaryModel(rowKey) : undefined}
        isDisabled={model.isDisabled}
        onToggleDisabled={
          onToggleModelDisabled
            ? () => onToggleModelDisabled(rowKey, !model.isDisabled)
            : undefined
        }
        selectionMode={selectionMode}
        selected={selectedIds.includes(rowKey)}
        onSelectChange={
          onToggleSelection ? (selected) => onToggleSelection(rowKey, selected) : undefined
        }
        extraActions={renderModelExtraActions?.(model, rowKey)}
      />
    );
  });

  // The toolbar is one node, used twice: as the fold's header on a card, and as
  // the list's title line in the detail sidebar. Building it here rather than in
  // each branch is what keeps the two hosts offering the same actions.
  const toolbar = (
    <div className={styles.toolbar}>
      <Text strong className={styles.title}>
        {t(`common.model.title`)} ({models.length})
      </Text>
      <Space size={0} onClick={(event) => event.stopPropagation()}>
        {onToggleBatchDeleteMode && (
          <Button
            size="small"
            type="text"
            icon={<DeleteOutlined />}
            style={actionButtonStyle}
            onClick={onToggleBatchDeleteMode}
          >
            {selectionMode
              ? t(`common.model.cancelBatchDelete`)
              : t(`common.model.batchDelete`)}
          </Button>
        )}
        {selectionMode && onBatchDelete && (
          <Button
            size="small"
            type="text"
            danger
            style={actionButtonStyle}
            disabled={selectedIds.length === 0}
            onClick={onBatchDelete}
          >
            {t(`common.model.deleteSelected`, { count: selectedIds.length })}
          </Button>
        )}
        {onTest && (
          <Tooltip title={testDisabled ? testDisabledTooltip : (testHint ?? '')}>
            <span>
              <Button
                size="small"
                type="text"
                style={actionButtonStyle}
                onClick={onTest}
                disabled={testDisabled}
              >
                <ApiOutlined style={{ marginRight: 4 }} />
                {t('opencode.connectivity.button')}
              </Button>
            </span>
          </Tooltip>
        )}
        {onFetchModels && (
          <Tooltip title={fetchDisabled ? fetchDisabledTooltip : ''}>
            <span>
              <Button
                size="small"
                type="text"
                style={actionButtonStyle}
                onClick={onFetchModels}
                disabled={fetchDisabled}
              >
                <CloudDownloadOutlined style={{ marginRight: 4 }} />
                {t(`common.model.fetchModels`)}
              </Button>
            </span>
          </Tooltip>
        )}
        {onAddModel && (
          <Button
            size="small"
            type="text"
            style={actionButtonStyle}
            onClick={onAddModel}
          >
            <PlusOutlined style={{ marginRight: 4 }} />
            {t(`common.model.addModel`)}
          </Button>
        )}
      </Space>
    </div>
  );

  // The indent exists to clear the fold's caret: it is what puts the rows under
  // the *text* of `模型列表` rather than under its arrow. The detail appearance
  // draws no caret, so its rows run to the section's edge and line up with the
  // title directly (user-directed, 2026-10-12).
  const foldableRows = appearance === 'card';
  const rowsStyle: React.CSSProperties | undefined = foldableRows
    ? { paddingLeft: 18, ...bodyStyle }
    : bodyStyle;

  const body = (
    <div className={styles.body} style={rowsStyle}>
      {aboveList}
      {models.length === 0 ? (
        <Empty
          image={Empty.PRESENTED_IMAGE_SIMPLE}
          description={t(`common.model.emptyText`)}
          style={{ margin: '8px 0' }}
        />
      ) : canReorder ? (
        <DndContext
          sensors={sensors}
          collisionDetection={closestCenter}
          modifiers={[restrictToVerticalAxis]}
          onDragEnd={handleDragEnd}
        >
          <SortableContext
            items={models.map((model) => rowKeyOf(model))}
            strategy={verticalListSortingStrategy}
          >
            <Space orientation="vertical" style={{ width: '100%' }} size={4}>
              {modelRows}
            </Space>
          </SortableContext>
        </DndContext>
      ) : (
        <Space orientation="vertical" style={{ width: '100%' }} size={4}>
          {modelRows}
        </Space>
      )}
    </div>
  );

  if (!foldableRows) {
    // A titled block, not a fold (2026-10-12, user-directed): in the detail
    // sidebar the rows are what the reader opened it to see, and a closed one
    // hid them behind a header whose only job was to hide them.
    return (
      <div
        className={[styles.section, className, transparentRows ? styles.transparentRows : undefined]
          .filter(Boolean)
          .join(' ')}
        data-model-list-section={sectionKey}
      >
        {toolbar}
        {body}
      </div>
    );
  }

  // The list card's form, unchanged: the toolbar is the fold's header, so a card
  // carrying eight models is the same height as its neighbours.
  return (
    <Collapse
      ghost
      className={[className, transparentRows ? styles.transparentCollapse : undefined]
        .filter(Boolean)
        .join(' ')}
      defaultActiveKey={[]}
      style={{ marginTop: 12, background: 'transparent' }}
      items={[{ key: sectionKey, label: toolbar, children: body }]}
    />
  );
};

export default ModelListSection;
