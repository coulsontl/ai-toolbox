import React from 'react';
import { Empty } from 'antd';
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
  SortableContext,
  sortableKeyboardCoordinates,
  rectSortingStrategy,
} from '@dnd-kit/sortable';
import { restrictToWindowEdges } from '@dnd-kit/modifiers';
import { useKeepAlive } from '@/components/layout/KeepAliveOutlet';
import { SkillCard } from './SkillCard';
import type { ManagedSkill, ToolOption } from '../types';
import styles from './SkillsList.module.less';

const DEFAULT_VIRTUAL_ROW_HEIGHT = 140;
const VIRTUAL_ROW_GAP = 12;
const VIRTUAL_OVERSCAN_ROWS = 3;
const SINGLE_COLUMN_MEDIA_QUERY = '(max-width: 1024px)';

interface SkillsListProps {
  skills: ManagedSkill[];
  allTools: ToolOption[];
  loading: boolean;
  updatingSkillIds: string[];
  dragDisabled?: boolean;
  getGithubInfo: (url: string | null | undefined) => { label: string; href: string } | null;
  getSkillSourceLabel: (skill: ManagedSkill) => string;
  formatRelative: (ms: number | null | undefined) => string;
  onUpdate: (skill: ManagedSkill) => void;
  onDelete: (skillId: string) => void;
  onToggleTool: (skill: ManagedSkill, toolId: string) => void;
  onEditMetadata: (skill: ManagedSkill) => void;
  onDragEnd: (event: DragEndEvent) => void;
}

export const SkillsList: React.FC<SkillsListProps> = ({
  skills,
  allTools,
  loading,
  updatingSkillIds,
  dragDisabled,
  getGithubInfo,
  getSkillSourceLabel,
  formatRelative,
  onUpdate,
  onDelete,
  onToggleTool,
  onEditMetadata,
  onDragEnd,
}) => {
  const { t } = useTranslation();
  const { isActive } = useKeepAlive();
  const listContainerRef = React.useRef<HTMLDivElement | null>(null);
  const rowObserverMapRef = React.useRef<Map<number, ResizeObserver>>(new Map());
  const [listViewportHeight, setListViewportHeight] = React.useState(720);
  const [scrollTop, setScrollTop] = React.useState(0);
  const [columnCount, setColumnCount] = React.useState(2);
  const [listOffsetTop, setListOffsetTop] = React.useState(0);
  const [rowHeights, setRowHeights] = React.useState<Record<number, number>>({});

  // Configure drag sensors
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: {
        distance: 8, // Prevent accidental drags
      },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  React.useLayoutEffect(() => {
    if (!dragDisabled || !isActive) {
      return undefined;
    }

    const containerElement = listContainerRef.current;
    const scrollElement = containerElement?.closest('main');
    if (!(scrollElement instanceof HTMLElement)) {
      return undefined;
    }

    const updateMetrics = () => {
      const isSingleColumn = window.matchMedia(SINGLE_COLUMN_MEDIA_QUERY).matches;
      setColumnCount(isSingleColumn ? 1 : 2);
      setListViewportHeight(scrollElement.clientHeight);
      setScrollTop(scrollElement.scrollTop);
      setListOffsetTop(containerElement?.offsetTop ?? 0);
    };

    updateMetrics();
    scrollElement.addEventListener('scroll', updateMetrics, { passive: true });
    window.addEventListener('resize', updateMetrics);

    return () => {
      scrollElement.removeEventListener('scroll', updateMetrics);
      window.removeEventListener('resize', updateMetrics);
    };
  }, [dragDisabled, isActive, skills.length]);

  React.useEffect(() => {
    setRowHeights({});
    for (const observer of rowObserverMapRef.current.values()) {
      observer.disconnect();
    }
    rowObserverMapRef.current.clear();
  }, [columnCount, skills]);

  React.useEffect(() => () => {
    for (const observer of rowObserverMapRef.current.values()) {
      observer.disconnect();
    }
    rowObserverMapRef.current.clear();
  }, []);

  const updateMeasuredRowHeight = React.useCallback((rowIndex: number, rowHeight: number) => {
    setRowHeights((previousHeights) => {
      if (previousHeights[rowIndex] === rowHeight) {
        return previousHeights;
      }
      return {
        ...previousHeights,
        [rowIndex]: rowHeight,
      };
    });
  }, []);

  const bindVirtualRowRef = React.useCallback(
    (rowIndex: number) => (node: HTMLDivElement | null) => {
      const previousObserver = rowObserverMapRef.current.get(rowIndex);
      if (previousObserver) {
        previousObserver.disconnect();
        rowObserverMapRef.current.delete(rowIndex);
      }

      if (!node) {
        return;
      }

      const measureRowHeight = () => {
        updateMeasuredRowHeight(rowIndex, node.offsetHeight);
      };

      measureRowHeight();

      const resizeObserver = new ResizeObserver(() => {
        measureRowHeight();
      });
      resizeObserver.observe(node);
      rowObserverMapRef.current.set(rowIndex, resizeObserver);
    },
    [updateMeasuredRowHeight],
  );

  const virtualizedRows = React.useMemo(() => {
    if (!dragDisabled) {
      return null;
    }

    const safeColumnCount = Math.max(1, columnCount);
    const totalRows = Math.ceil(skills.length / safeColumnCount);
    const estimatedRowHeight = DEFAULT_VIRTUAL_ROW_HEIGHT + VIRTUAL_ROW_GAP;
    const rowOffsets: number[] = [];
    let totalHeight = 0;

    for (let rowIndex = 0; rowIndex < totalRows; rowIndex += 1) {
      rowOffsets[rowIndex] = totalHeight;
      const measuredRowHeight = rowHeights[rowIndex] ?? DEFAULT_VIRTUAL_ROW_HEIGHT;
      totalHeight += measuredRowHeight + VIRTUAL_ROW_GAP;
    }

    const viewportStart = Math.max(0, scrollTop - estimatedRowHeight * VIRTUAL_OVERSCAN_ROWS);
    const viewportEnd =
      scrollTop
      + listViewportHeight
      + estimatedRowHeight * VIRTUAL_OVERSCAN_ROWS;
    const localViewportStart = Math.max(0, viewportStart - listOffsetTop);
    const localViewportEnd = Math.max(0, viewportEnd - listOffsetTop);

    let startRow = 0;
    while (startRow < totalRows) {
      const rowBottom =
        rowOffsets[startRow] + (rowHeights[startRow] ?? DEFAULT_VIRTUAL_ROW_HEIGHT);
      if (rowBottom >= localViewportStart) {
        break;
      }
      startRow += 1;
    }

    let endRow = startRow;
    while (endRow < totalRows && rowOffsets[endRow] <= localViewportEnd) {
      endRow += 1;
    }

    const rows = [];
    for (let rowIndex = startRow; rowIndex < endRow; rowIndex += 1) {
      const rowStartIndex = rowIndex * safeColumnCount;
      rows.push({
        rowIndex,
        top: rowOffsets[rowIndex] ?? 0,
        skills: skills.slice(rowStartIndex, rowStartIndex + safeColumnCount),
      });
    }

    return {
      rows,
      totalHeight: Math.max(0, totalHeight - VIRTUAL_ROW_GAP),
    };
  }, [columnCount, dragDisabled, listOffsetTop, listViewportHeight, rowHeights, scrollTop, skills]);

  if (skills.length === 0) {
    return (
      <div className={styles.empty}>
        <Empty description={t('skills.skillsEmpty')} />
      </div>
    );
  }

  const cardList = (
    <div className={styles.list}>
      {skills.map((skill) => (
        <SkillCard
          key={skill.id}
          skill={skill}
          allTools={allTools}
          loading={loading}
          isUpdating={updatingSkillIds.includes(skill.id)}
          dragDisabled={dragDisabled}
          getGithubInfo={getGithubInfo}
          getSkillSourceLabel={getSkillSourceLabel}
          formatRelative={formatRelative}
          onUpdate={onUpdate}
          onDelete={onDelete}
          onToggleTool={onToggleTool}
          onEditMetadata={onEditMetadata}
        />
      ))}
    </div>
  );

  if (dragDisabled) {
    return (
      <div ref={listContainerRef} className={styles.virtualListShell}>
        {virtualizedRows && (
          <div
            className={styles.virtualListViewport}
            style={{ height: virtualizedRows.totalHeight }}
          >
            {virtualizedRows.rows.map((row) => (
              <div
                key={`row-${row.rowIndex}`}
                ref={bindVirtualRowRef(row.rowIndex)}
                className={styles.virtualRow}
                style={{ top: row.top }}
              >
                {row.skills.map((skill) => (
                  <SkillCard
                    key={skill.id}
                    skill={skill}
                    allTools={allTools}
                    loading={loading}
                    isUpdating={updatingSkillIds.includes(skill.id)}
                    dragDisabled
                    getGithubInfo={getGithubInfo}
                    getSkillSourceLabel={getSkillSourceLabel}
                    formatRelative={formatRelative}
                    onUpdate={onUpdate}
                    onDelete={onDelete}
                    onToggleTool={onToggleTool}
                    onEditMetadata={onEditMetadata}
                  />
                ))}
                {row.skills.length < columnCount
                  ? Array.from({ length: columnCount - row.skills.length }).map((_, fillerIndex) => (
                      <div
                        key={`row-${row.rowIndex}-filler-${fillerIndex}`}
                        className={styles.virtualFiller}
                      />
                    ))
                  : null}
              </div>
            ))}
          </div>
        )}
      </div>
    );
  }

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      modifiers={[restrictToWindowEdges]}
      onDragEnd={onDragEnd}
    >
      <SortableContext
        items={skills.map((s) => s.id)}
        strategy={rectSortingStrategy}
      >
        {cardList}
      </SortableContext>
    </DndContext>
  );
};
