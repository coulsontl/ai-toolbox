import React from 'react';
import { Empty, Spin } from 'antd';
import { useTranslation } from 'react-i18next';
import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
} from '@dnd-kit/core';
import {
  SortableContext,
  sortableKeyboardCoordinates,
  rectSortingStrategy,
} from '@dnd-kit/sortable';
import type { DragEndEvent } from '@dnd-kit/core';
import type { McpServer, McpTool } from '../types';
import { McpCard } from './McpCard';
import styles from './McpList.module.less';

interface McpListProps {
  servers: McpServer[];
  tools: McpTool[];
  loading: boolean;
  dragDisabled?: boolean;
  onEdit: (server: McpServer) => void;
  onEditMetadata: (server: McpServer) => void;
  onDelete: (serverId: string) => void;
  onToggleTool: (serverId: string, toolKey: string) => void;
  onDragEnd: (event: DragEndEvent) => void;
}

export const McpList: React.FC<McpListProps> = ({
  servers,
  tools,
  loading,
  dragDisabled,
  onEdit,
  onEditMetadata,
  onDelete,
  onToggleTool,
  onDragEnd,
}) => {
  const { t } = useTranslation();

  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  if (loading && servers.length === 0) {
    return (
      <div className={styles.loading}>
        <Spin />
      </div>
    );
  }

  if (servers.length === 0) {
    return (
      <div className={styles.empty}>
        <Empty description={t('mcp.noServers')} />
      </div>
    );
  }

  const cardList = (
    <div className={styles.list}>
      {servers.map((server) => (
        <McpCard
          key={server.id}
          server={server}
          tools={tools}
          loading={loading}
          dragDisabled={dragDisabled}
          onEdit={onEdit}
          onEditMetadata={onEditMetadata}
          onDelete={onDelete}
          onToggleTool={onToggleTool}
        />
      ))}
    </div>
  );

  if (dragDisabled) {
    return cardList;
  }

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      onDragEnd={onDragEnd}
    >
      <SortableContext
        items={servers.map((s) => s.id)}
        strategy={rectSortingStrategy}
      >
        {cardList}
      </SortableContext>
    </DndContext>
  );
};

export default McpList;
