import React from 'react';
import { Collapse, Empty } from 'antd';
import { useTranslation } from 'react-i18next';
import type { McpGroup, McpServer, McpTool } from '../types';
import { McpCard } from './McpCard';
import styles from './McpGroupedList.module.less';

interface McpGroupedListProps {
  groups: McpGroup[];
  tools: McpTool[];
  loading: boolean;
  activeKeys: string[];
  onActiveKeysChange: (keys: string[]) => void;
  onEdit: (server: McpServer) => void;
  onEditMetadata: (server: McpServer) => void;
  onDelete: (serverId: string) => void;
  onToggleTool: (serverId: string, toolKey: string) => void;
}

export const McpGroupedList: React.FC<McpGroupedListProps> = ({
  groups,
  tools,
  loading,
  activeKeys,
  onActiveKeysChange,
  onEdit,
  onEditMetadata,
  onDelete,
  onToggleTool,
}) => {
  const { t } = useTranslation();

  if (groups.length === 0) {
    return (
      <div className={styles.empty}>
        <Empty description={t('mcp.noServers')} />
      </div>
    );
  }

  const items = groups.map((group) => ({
    key: group.key,
    label: (
      <div className={styles.groupHeader}>
        <span className={styles.groupLabel}>
          {group.label}
          <span className={styles.groupCount}>
            ({t('mcp.serverCount', { count: group.servers.length })})
          </span>
        </span>
      </div>
    ),
    children: (
      <div className={styles.groupGrid}>
        {group.servers.map((server) => (
          <McpCard
            key={server.id}
            server={server}
            tools={tools}
            loading={loading}
            dragDisabled
            onEdit={onEdit}
            onEditMetadata={onEditMetadata}
            onDelete={onDelete}
            onToggleTool={onToggleTool}
          />
        ))}
      </div>
    ),
  }));

  return (
    <div className={styles.groupedList}>
      <Collapse
        activeKey={activeKeys}
        onChange={(keys) => onActiveKeysChange(keys as string[])}
        items={items}
      />
    </div>
  );
};

export default McpGroupedList;
