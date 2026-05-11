import React from 'react';
import { ChevronDown, Plus } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  ManagementCheckbox,
  ManagementEmpty,
  ManagementMenu,
  VirtualGrid,
  type ManagementMenuItem,
} from '@/features/coding/shared/management';
import type { McpGroup, McpServer, McpTool } from '../types';
import { getMcpGroupToolKeys, isMcpUngroupedCustomGroup } from '../utils/mcpGrouping';
import { McpCard } from './McpCard';
import styles from './McpGroupedList.module.less';

interface McpGroupedListProps {
  groups: McpGroup[];
  tools: McpTool[];
  loading: boolean;
  columns?: number;
  activeKeys: string[];
  onActiveKeysChange: (keys: string[]) => void;
  selectionMode: boolean;
  selectedIds: Set<string>;
  onSelectChange: (serverId: string, checked: boolean) => void;
  onSelectAllGroup: (group: McpGroup, checked: boolean) => void;
  onEdit: (server: McpServer) => void;
  onEditMetadata: (server: McpServer) => void;
  onDelete: (serverId: string) => void;
  onToggleTool: (serverId: string, toolKey: string) => void;
  groupToolMode?: boolean;
  onAddGroupTool?: (group: McpGroup, toolKey: string) => void;
  onRemoveGroupTool?: (group: McpGroup, toolKey: string) => void;
}

export const McpGroupedList: React.FC<McpGroupedListProps> = ({
  groups,
  tools,
  loading,
  columns,
  activeKeys,
  onActiveKeysChange,
  selectionMode,
  selectedIds,
  onSelectChange,
  onSelectAllGroup,
  onEdit,
  onEditMetadata,
  onDelete,
  onToggleTool,
  groupToolMode = false,
  onAddGroupTool,
  onRemoveGroupTool,
}) => {
  const { t } = useTranslation();
  const activeKeySet = React.useMemo(() => new Set(activeKeys), [activeKeys]);

  if (groups.length === 0) {
    return (
      <div className={styles.empty}>
        <ManagementEmpty description={t('mcp.noServers')} />
      </div>
    );
  }

  const isGroupAllSelected = (group: McpGroup) =>
    group.servers.length > 0 && group.servers.every((server) => selectedIds.has(server.id));

  const isGroupPartialSelected = (group: McpGroup) =>
    group.servers.some((server) => selectedIds.has(server.id)) && !isGroupAllSelected(group);

  const renderGroupTools = (group: McpGroup) => {
    const activeToolKeys = new Set(getMcpGroupToolKeys(group));
    const activeTools = tools.filter((tool) => activeToolKeys.has(tool.key));
    const availableTools = tools.filter((tool) => tool.installed && !activeToolKeys.has(tool.key));
    const availableToolItems: ManagementMenuItem[] = availableTools.map((tool) => ({
      key: tool.key,
      label: tool.display_name,
      onSelect: () => onAddGroupTool?.(group, tool.key),
    }));

    return (
      <div className={styles.groupTools}>
        {activeTools.map((tool) => (
          <button
            key={tool.key}
            title={t('mcp.groupTools.removeTool', { tool: tool.display_name })}
            type="button"
            className={styles.groupToolPill}
            disabled={loading}
            onClick={() => onRemoveGroupTool?.(group, tool.key)}
          >
            <span className={styles.statusBadge} />
            {tool.display_name}
          </button>
        ))}
        {availableTools.length > 0 && (
          <ManagementMenu
            items={availableToolItems}
            disabled={loading}
            title={t('common.add')}
            triggerClassName={styles.groupToolAdd}
            controlSize="compact"
          >
            <Plus size={13} aria-hidden="true" />
          </ManagementMenu>
        )}
      </div>
    );
  };

  const handleToggleGroup = (groupKey: string) => {
    const nextKeys = activeKeySet.has(groupKey)
      ? activeKeys.filter((key) => key !== groupKey)
      : [...activeKeys, groupKey];
    onActiveKeysChange(nextKeys);
  };

  return (
    <div className={styles.groupedList}>
      {groups.map((group) => {
        const groupToolsEnabled = groupToolMode && !isMcpUngroupedCustomGroup(group);
        const isOpen = activeKeySet.has(group.key);

        return (
          <section key={group.key} className={styles.groupSection}>
            <div className={styles.groupHeader}>
              <div className={styles.groupTitle}>
                {selectionMode && (
                  <ManagementCheckbox
                    checked={isGroupAllSelected(group)}
                    indeterminate={isGroupPartialSelected(group)}
                    ariaLabel={`${t('mcp.batch.selectAll')} ${group.label}`}
                    onChange={(checked) => onSelectAllGroup(group, checked)}
                  />
                )}
                <button
                  type="button"
                  className={styles.groupToggle}
                  aria-expanded={isOpen}
                  onClick={() => handleToggleGroup(group.key)}
                >
                  <ChevronDown
                    size={15}
                    className={`${styles.groupChevron}${isOpen ? ` ${styles.groupChevronOpen}` : ''}`}
                    aria-hidden="true"
                  />
                  <span className={styles.groupLabel}>{group.label}</span>
                  <span className={styles.groupCount}>
                    {t('mcp.serverCount', { count: group.servers.length })}
                  </span>
                </button>
              </div>
              {groupToolsEnabled && renderGroupTools(group)}
            </div>
            {isOpen && (
              <div className={styles.groupBody}>
                <VirtualGrid
                  items={group.servers}
                  getKey={(server) => server.id}
                  columns={columns}
                  defaultRowHeight={78}
                  virtualize={group.servers.length > 24}
                  renderItem={(server) => (
                    <McpCard
                      server={server}
                      tools={tools}
                      loading={loading}
                      dragDisabled
                      selectable={selectionMode}
                      selected={selectedIds.has(server.id)}
                      toolsReadOnly={groupToolsEnabled}
                      onSelectChange={onSelectChange}
                      onEdit={onEdit}
                      onEditMetadata={onEditMetadata}
                      onDelete={onDelete}
                      onToggleTool={onToggleTool}
                    />
                  )}
                />
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
};

export default McpGroupedList;
