import React from 'react';
import { message } from 'antd';
import {
  Code2,
  Globe2,
  MoreHorizontal,
  Pencil,
  Plus,
  Tags,
  Trash2,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import {
  ManagementCard,
  ManagementCardActions,
  ManagementCardCheckboxArea,
  ManagementCardDragHandle,
  ManagementCardHeader,
  ManagementCardIcon,
  ManagementCardMain,
  ManagementCardMetaRow,
  ManagementCardToolMatrix,
  ManagementCheckbox,
  ManagementIconButton,
  ManagementMenu,
  type ManagementMenuItem,
} from '@/features/coding/shared/management';
import type { McpServer, McpTool } from '../types';
import { getMcpDisplayNote } from '../utils/mcpGrouping';
import styles from './McpCard.module.less';

interface McpCardProps {
  server: McpServer;
  tools: McpTool[];
  loading: boolean;
  dragDisabled?: boolean;
  selected?: boolean;
  selectable?: boolean;
  toolsReadOnly?: boolean;
  onSelectChange?: (serverId: string, checked: boolean) => void;
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

const McpCardContent = React.memo(function McpCardContent({
  server,
  tools,
  loading,
  selected,
  selectable,
  toolsReadOnly,
  onSelectChange,
  onEdit,
  onEditMetadata,
  onDelete,
  onToggleTool,
  dragHandle,
  containerRef,
  containerStyle,
}: McpCardContentProps) {
  const { t } = useTranslation();

  const iconNode = React.useMemo(() => (
    server.server_type === 'stdio' ? (
      <Code2 size={18} className={styles.icon} />
    ) : (
      <Globe2 size={18} className={styles.icon} />
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

  const handleReadOnlyToolClick = React.useCallback(() => {
    message.info(t('mcp.groupTools.cardToolReadOnly'));
  }, [t]);

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
  const dropdownItems = React.useMemo<ManagementMenuItem[]>(
    () =>
      availableDropdownTools.map((tool) => ({
        key: tool.key,
        label: tool.display_name,
        onSelect: () => onToggleTool(server.id, tool.key),
      })),
    [availableDropdownTools, onToggleTool, server.id],
  );

  const actionItems = React.useMemo<ManagementMenuItem[]>(
    () => [
      {
        key: 'metadata',
        icon: <Tags size={14} />,
        label: t('mcp.metadata.edit'),
        onSelect: () => onEditMetadata(server),
      },
      {
        key: 'delete',
        danger: true,
        icon: <Trash2 size={14} />,
        label: t('mcp.delete'),
        onSelect: () => onDelete(server.id),
      },
    ],
    [onDelete, onEditMetadata, server, t],
  );

  return (
    <ManagementCard
      containerRef={containerRef}
      containerStyle={containerStyle}
      selected={selected}
      selectable={selectable}
    >
      {selectable && (
        <ManagementCardCheckboxArea>
          <ManagementCheckbox
            ariaLabel={`${t('common.select')} ${server.name}`}
            checked={!!selected}
            onChange={(checked) => onSelectChange?.(server.id, checked)}
          />
        </ManagementCardCheckboxArea>
      )}
      {dragHandle}
      <ManagementCardIcon icon={iconNode} />
      <ManagementCardMain>
        <ManagementCardHeader
          title={server.name}
          minWidth={92}
          meta={
            <>
              <span className={styles.typeTag}>{server.server_type}</span>
              <span className={styles.configSummary} title={configSummary}>{configSummary}</span>
            </>
          }
        />
        {(server.user_group || displayNote) && (
          <ManagementCardMetaRow>
            {server.user_group && (
              <span className={styles.groupTag} title={server.user_group}>{server.user_group}</span>
            )}
            {displayNote && (
              <span className={styles.note} title={displayNote}>{displayNote}</span>
            )}
          </ManagementCardMetaRow>
        )}
        <ManagementCardToolMatrix>
          {enabledTools.map((tool) => {
            const syncDetail = server.sync_details.find((d) => d.tool === tool.key);
            const status = syncDetail?.status || 'pending';
            return (
              <button
                key={`${server.id}-${tool.key}`}
                title={`${tool.display_name} - ${status}`}
                type="button"
                className={`${styles.toolPill} ${styles.active} ${status === 'error' ? styles.error : ''}${toolsReadOnly ? ` ${styles.readOnlyTool}` : ''}`}
                onClick={toolsReadOnly ? handleReadOnlyToolClick : () => onToggleTool(server.id, tool.key)}
                disabled={loading}
                aria-disabled={toolsReadOnly || loading}
              >
                <span className={`${styles.statusBadge} ${styles[status]}`} />
                {tool.display_name}
              </button>
            );
          })}
          {!toolsReadOnly && dropdownItems.length > 0 && (
            <ManagementMenu
              items={dropdownItems}
              disabled={loading}
              title={t('common.add')}
              triggerClassName={styles.addToolBtn}
            >
              <Plus size={13} aria-hidden="true" />
            </ManagementMenu>
          )}
        </ManagementCardToolMatrix>
      </ManagementCardMain>
      <ManagementCardActions>
        <ManagementMenu
          items={actionItems}
          disabled={loading}
          title={t('mcp.more')}
          controlSize="compact"
        >
          <MoreHorizontal size={16} aria-hidden="true" />
        </ManagementMenu>
        <ManagementIconButton
          icon={<Pencil size={15} aria-hidden="true" />}
          onClick={() => onEdit(server)}
          disabled={loading}
          title={t('mcp.editServer')}
          controlSize="compact"
        />
      </ManagementCardActions>
    </ManagementCard>
  );
});

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
        <ManagementCardDragHandle
          {...attributes}
          listeners={listeners}
        />
      )}
    />
  );
};

export const McpCard = React.memo(function McpCard({
  dragDisabled,
  ...props
}: McpCardProps) {
  if (dragDisabled) {
    return <McpCardContent {...props} />;
  }

  return <SortableMcpCard {...props} />;
});

export default McpCard;
