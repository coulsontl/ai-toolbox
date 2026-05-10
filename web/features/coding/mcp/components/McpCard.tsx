import React from 'react';
import { Button, Tooltip, Dropdown, Tag } from 'antd';
import {
  DeleteOutlined,
  EditOutlined,
  EllipsisOutlined,
  PlusOutlined,
  HolderOutlined,
  CodeOutlined,
  GlobalOutlined,
  TagsOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { McpServer, McpTool } from '../types';
import { getMcpDisplayNote } from '../utils/mcpGrouping';
import styles from './McpCard.module.less';

interface McpCardProps {
  server: McpServer;
  tools: McpTool[];
  loading: boolean;
  dragDisabled?: boolean;
  onEdit: (server: McpServer) => void;
  onEditMetadata: (server: McpServer) => void;
  onDelete: (serverId: string) => void;
  onToggleTool: (serverId: string, toolKey: string) => void;
}

interface McpCardContentProps extends Omit<McpCardProps, 'dragDisabled'> {
  dragHandle?: React.ReactNode;
  containerRef?: (node: HTMLDivElement | null) => void;
  containerStyle?: React.CSSProperties;
}

const McpCardContent: React.FC<McpCardContentProps> = ({
  server,
  tools,
  loading,
  onEdit,
  onEditMetadata,
  onDelete,
  onToggleTool,
  dragHandle,
  containerRef,
  containerStyle,
}) => {
  const { t } = useTranslation();

  const iconNode = React.useMemo(() => (
    server.server_type === 'stdio' ? (
      <CodeOutlined className={styles.icon} />
    ) : (
      <GlobalOutlined className={styles.icon} />
    )
  ), [server.server_type]);

  // Config summary only depends on the current server definition.
  // Memoizing keeps repeated card renders from recalculating the same display string.
  const configSummary = React.useMemo(() => {
    if (server.server_type === 'stdio') {
      const config = server.server_config as { command?: string };
      return config.command || 'stdio';
    }
    const config = server.server_config as { url?: string };
    return config.url || 'http';
  }, [server.server_config, server.server_type]);

  const displayNote = React.useMemo(() => getMcpDisplayNote(server), [server]);

  // These tool collections are pure derived data from the server/tool definitions.
  // Memoizing them reduces repeated filtering/sorting work across large card lists.
  const enabledToolKeys = React.useMemo(
    () => new Set(server.enabled_tools),
    [server.enabled_tools],
  );

  const enabledTools = React.useMemo(
    () => tools.filter((tool) => enabledToolKeys.has(tool.key)),
    [enabledToolKeys, tools],
  );

  const availableDropdownTools = React.useMemo(() => {
    return tools.filter((tool) => tool.installed && !enabledToolKeys.has(tool.key));
  }, [enabledToolKeys, tools]);

  // Dropdown items are presentation-only data. Memoizing keeps the menu stable unless
  // the tool list, translation output, or toggle handler actually changes.
  const dropdownItems = React.useMemo(
    () =>
      availableDropdownTools.map((tool) => ({
        key: tool.key,
        label: (
          <span>
            {tool.display_name}
          </span>
        ),
        onClick: () => onToggleTool(server.id, tool.key),
      })),
    [availableDropdownTools, onToggleTool, server.id],
  );

  const actionItems = React.useMemo(
    () => [
      {
        key: 'metadata',
        icon: <TagsOutlined />,
        label: t('mcp.metadata.edit'),
        onClick: () => onEditMetadata(server),
      },
      {
        key: 'delete',
        danger: true,
        icon: <DeleteOutlined />,
        label: t('mcp.delete'),
        onClick: () => onDelete(server.id),
      },
    ],
    [onDelete, onEditMetadata, server, t],
  );

  return (
    <div ref={containerRef} style={containerStyle}>
      <div className={styles.card}>
        {dragHandle}
        <div className={styles.iconArea}>{iconNode}</div>
        <div className={styles.main}>
          <div className={styles.headerRow}>
            <div className={styles.name}>{server.name}</div>
            <Tag className={styles.typeTag}>{server.server_type}</Tag>
            <span className={styles.configSummary}>{configSummary}</span>
          </div>
          {(server.user_group || displayNote) && (
            <div className={styles.metaRow}>
              {server.user_group && (
                <Tooltip title={server.user_group}>
                  <span className={styles.groupTag}>{server.user_group}</span>
                </Tooltip>
              )}
              {displayNote && (
                <Tooltip title={displayNote}>
                  <span className={styles.note}>{displayNote}</span>
                </Tooltip>
              )}
            </div>
          )}
          <div className={styles.toolMatrix}>
            {enabledTools.map((tool) => {
              const syncDetail = server.sync_details.find((d) => d.tool === tool.key);
              const status = syncDetail?.status || 'pending';
              return (
                <Tooltip
                  key={`${server.id}-${tool.key}`}
                  title={`${tool.display_name} - ${status}`}
                >
                  <button
                    type="button"
                    className={`${styles.toolPill} ${styles.active} ${status === 'error' ? styles.error : ''}`}
                    onClick={() => onToggleTool(server.id, tool.key)}
                  >
                    <span className={`${styles.statusBadge} ${styles[status]}`} />
                    {tool.display_name}
                  </button>
                </Tooltip>
              );
            })}
            {dropdownItems.length > 0 && (
              <Dropdown
                menu={{ items: dropdownItems }}
                trigger={['click']}
                disabled={loading}
              >
                <button type="button" className={styles.addToolBtn}>
                  <PlusOutlined />
                </button>
              </Dropdown>
            )}
          </div>
        </div>
        <div className={styles.actions}>
          <Dropdown
            menu={{ items: actionItems }}
            trigger={['click']}
            disabled={loading}
          >
            <Button
              type="text"
              icon={<EllipsisOutlined />}
              disabled={loading}
              title={t('mcp.more')}
            />
          </Dropdown>
          <Button
            type="text"
            icon={<EditOutlined />}
            onClick={() => onEdit(server)}
            disabled={loading}
            title={t('mcp.editServer')}
          />
        </div>
      </div>
    </div>
  );
};

const SortableMcpCard: React.FC<Omit<McpCardProps, 'dragDisabled'>> = (props) => {
  const {
    server,
  } = props;

  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: server.id });

  const sortableStyle: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
  };

  return (
    <McpCardContent
      {...props}
      containerRef={setNodeRef}
      containerStyle={sortableStyle}
      dragHandle={(
        <div
          className={styles.dragHandle}
          {...attributes}
          {...listeners}
        >
          <HolderOutlined />
        </div>
      )}
    />
  );
};

export const McpCard: React.FC<McpCardProps> = ({
  dragDisabled,
  ...props
}) => {
  if (dragDisabled) {
    return <McpCardContent {...props} />;
  }

  return <SortableMcpCard {...props} />;
};

export default McpCard;
