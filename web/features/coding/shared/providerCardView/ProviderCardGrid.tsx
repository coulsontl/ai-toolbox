import React from 'react';
import { useAutoGridColumns } from '@/features/coding/shared/management';
import styles from './ProviderCardGrid.module.less';

interface ProviderCardGridProps<TItem> {
  items: TItem[];
  getKey: (item: TItem) => React.Key;
  renderItem: (item: TItem) => React.ReactNode;
  /**
   * Fixed column count. Omit for the responsive auto count, which is what the
   * user sees by default.
   */
  columns?: number;
}

/**
 * The responsive grid the compact provider cards sit in.
 *
 * ## Why this does not virtualize (unlike `SkillsList` / `McpList`)
 *
 * Provider lists are tens of items, and the list layout renders all of them
 * too. Virtualization would remove off-screen cards from the DOM, and
 * `ProviderListSection`'s 定位 action finds its target by querying
 * `[data-provider-id]` — a virtualized grid would answer "当前列表里找不到已应用的供应商"
 * for a provider that is right there, just below the fold. That failure reads
 * as a broken button, not as a rendering optimization. Skills and MCP
 * virtualize because they manage hundreds of items; this list does not.
 *
 * ## Why it does not own the `DndContext`
 *
 * Drag lives on the page, which already owns the sensors and the reorder
 * handler for the list layout. One context serving both layouts keeps the
 * sensors from being created twice (and from disagreeing about when dragging
 * is disabled). The only thing that changes with the layout is the sorting
 * strategy and the drag modifiers, which the page passes to `SortableContext`.
 *
 * Columns come from the same `useAutoGridColumns` math `VirtualGrid` uses, so a
 * grid here and a grid on the Skills page break at the same widths.
 */
export function ProviderCardGrid<TItem>({
  items,
  getKey,
  renderItem,
  columns,
}: ProviderCardGridProps<TItem>) {
  const { containerRef, columnCount: autoColumnCount } = useAutoGridColumns<HTMLDivElement>({
    minColumnWidth: 350,
    maxColumns: 3,
    gap: 10,
    enabled: columns === undefined,
  });
  const effectiveColumns = columns === undefined ? autoColumnCount : columns;

  return (
    <div
      ref={containerRef}
      data-provider-grid
      className={[
        styles.grid,
        columns === undefined ? styles.gridAuto : styles.gridFixed,
      ].filter(Boolean).join(' ')}
      style={effectiveColumns === undefined ? undefined : ({
        '--management-grid-columns': `repeat(${effectiveColumns}, minmax(0, 1fr))`,
      } as React.CSSProperties)}
    >
      {items.map((item) => (
        // A cell is one grid item, so a mapping layer that returns a fragment
        // (OMP: the subscription card plus its catalog warning) must have it
        // wrapped. Without the wrapper each child was its own cell, splitting
        // one provider across two columns and pushing its neighbours along.
        <div key={getKey(item)} data-provider-grid-cell>
          {renderItem(item)}
        </div>
      ))}
    </div>
  );
}

export default ProviderCardGrid;
