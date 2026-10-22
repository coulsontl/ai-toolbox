import React from 'react';
import { message } from 'antd';
import {
  Code2,
  Copy,
  FileJson,
  Globe2,
  GripVertical,
  Loader2,
  MoreHorizontal,
  Pencil,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Tags,
  Trash2,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import {
  ManagementCheckbox,
  ManagementMenu,
  type ManagementMenuItem,
} from '@/features/coding/shared/management';
import { ToolIcon } from '@/features/coding/shared/toolIcon/ToolIcon';
import type { McpServer, McpSyncDetail, McpTool } from '../types';
import {
  getMcpCommandPackageVersion,
  getMcpCommandPackageVersionKey,
} from '../utils/mcpCommandPackageVersion';
import {
  hashTagColorIndex,
  normalizeTagList,
} from '../utils/mcpTags';
import { formatMcpServerExportJson } from '../utils/mcpServerExportJson';
import styles from './McpCard.module.less';

const TAG_COLOR_CLASS_NAMES: readonly string[] = [
  styles.tagColor0,
  styles.tagColor1,
  styles.tagColor2,
  styles.tagColor3,
  styles.tagColor4,
  styles.tagColor5,
  styles.tagColor6,
  styles.tagColor7,
];
const tagPillColorClass = (tag: string): string =>
  TAG_COLOR_CLASS_NAMES[hashTagColorIndex(tag)] ?? styles.tagColor0;

/** Middle-ellipsis truncation for the package@version badge. */
function truncateMiddle(text: string, maxLength = 24): string {
  if (text.length <= maxLength) {
    return text;
  }
  const keepLength = Math.max(3, Math.floor((maxLength - 1) / 2));
  return `${text.slice(0, keepLength)}…${text.slice(-keepLength)}`;
}

interface McpCardProps {
  server: McpServer;
  tools: McpTool[];
  loading: boolean;
  dragDisabled?: boolean;
  selected?: boolean;
  selectable?: boolean;
  toolsReadOnly?: boolean;
  resolvedPackageVersions?: Record<string, string>;
  preferredToolKeysForAddMore?: string[];
  limitAddMoreToPreferredTools?: boolean;
  onSelectChange?: (serverId: string, checked: boolean) => void;
  /** Browse-mode card body click opens the detail drawer. */
  onOpenDetail?: (server: McpServer) => void;
  onEdit: (server: McpServer) => void;
  onEditMetadata: (server: McpServer) => void;
  onDelete: (serverId: string) => void;
  onToggleTool: (serverId: string, toolKey: string) => void;
  /** Disable or re-enable a server's management state. */
  onSetManagementEnabled?: (server: McpServer, enabled: boolean) => void;
  /** High-frequency refresh of the server list (reloads sync statuses). */
  onRefresh?: () => void;
}

interface McpCardContentProps extends Omit<McpCardProps, 'dragDisabled'> {
  dragHandle?: React.ReactNode;
  containerRef?: (node: HTMLDivElement | null) => void;
  containerStyle?: React.CSSProperties;
}

// Card-body click opens the detail panel only in browse mode; interactive
// elements keep their own click. Mirrors the SkillCard exclusion rules.
const DETAIL_CLICK_EXCLUDE_SELECTOR =
  'button, a, input, [role="menuitem"], [role="checkbox"], [data-mcp-card-no-detail]';

const McpCardContent = React.memo(function McpCardContent({
  server,
  tools,
  loading,
  selected,
  selectable,
  toolsReadOnly,
  resolvedPackageVersions,
  preferredToolKeysForAddMore,
  limitAddMoreToPreferredTools,
  onSelectChange,
  onOpenDetail,
  onEdit,
  onEditMetadata,
  onDelete,
  onToggleTool,
  onSetManagementEnabled,
  onRefresh,
  dragHandle,
  containerRef,
  containerStyle,
}: McpCardContentProps) {
  const { t } = useTranslation();

  const mcpTools = React.useMemo(
    () => tools.filter((tool) => tool.supports_mcp),
    [tools],
  );

  const enabledToolIds = React.useMemo(
    () => new Set(server.enabled_tools),
    [server.enabled_tools],
  );

  // Footer only shows tools already synced/enabled for this server; the "+"
  // menu lists the remaining installed candidates. Matches SkillCard.
  const enabledTools = React.useMemo(
    () => mcpTools.filter((tool) => enabledToolIds.has(tool.key)),
    [enabledToolIds, mcpTools],
  );

  const syncDetailByTool = React.useMemo(() => {
    const detailMap = new Map<string, McpSyncDetail>();
    for (const detail of server.sync_details) {
      detailMap.set(detail.tool, detail);
    }
    return detailMap;
  }, [server.sync_details]);

  // Transport identity for the footer source slot: stdio shows the command,
  // http/sse show the endpoint URL.
  const configSummary = React.useMemo(() => {
    if (server.server_type === 'stdio') {
      const config = server.server_config as { command?: string };
      return config.command || 'stdio';
    }
    const config = server.server_config as { url?: string };
    return config.url || 'http';
  }, [server.server_config, server.server_type]);

  // Second row: full command line. For stdio, command + args joined by spaces;
  // for http/sse, the endpoint URL.
  const commandText = React.useMemo(() => {
    if (server.server_type === 'stdio') {
      const config = server.server_config as { command?: string; args?: string[] };
      const args = Array.isArray(config.args) && config.args.length > 0
        ? ` ${config.args.join(' ')}`
        : '';
      return `${config.command || 'stdio'}${args}`;
    }
    const config = server.server_config as { url?: string };
    return config.url || 'http';
  }, [server.server_config, server.server_type]);

  const transportIconNode = React.useMemo(
    () => (server.server_type === 'stdio'
      ? <Code2 size={13} aria-hidden="true" />
      : <Globe2 size={13} aria-hidden="true" />),
    [server.server_type],
  );

  const handleCopyConfig = React.useCallback(async () => {
    if (!configSummary) {
      return;
    }
    try {
      await navigator.clipboard.writeText(configSummary);
      message.success(t('mcp.copiedConfig'));
    } catch {
      message.error(t('mcp.copyFailed'));
    }
  }, [configSummary, t]);

  const packageVersion = React.useMemo(
    () => (server.server_type === 'stdio' ? getMcpCommandPackageVersion(server.server_config) : null),
    [server.server_config, server.server_type],
  );

  const packageVersionDisplayText = React.useMemo(() => {
    if (!packageVersion) {
      return null;
    }
    if (packageVersion.versionLabel !== 'latest') {
      // Pinned version: displayText is already the bare version label.
      return packageVersion.displayText;
    }

    const resolvedVersion = resolvedPackageVersions?.[
      getMcpCommandPackageVersionKey(packageVersion.manager, packageVersion.packageName)
    ];
    if (!resolvedVersion) {
      return null;
    }
    // Show only the version number; the package name is already visible in
    // the command line above, so it is not repeated here.
    return resolvedVersion;
  }, [packageVersion, resolvedPackageVersions]);

  const descriptionText = React.useMemo(
    () => server.description?.trim() ?? '',
    [server.description],
  );

  const groupText = React.useMemo(
    () => server.user_group?.trim() ?? '',
    [server.user_group],
  );

  const noteText = React.useMemo(
    () => server.user_note?.trim() ?? '',
    [server.user_note],
  );

  const tagList = React.useMemo(
    () => normalizeTagList(server.tags ?? []),
    [server.tags],
  );

  // Candidate tools for the footer "+" menu: installed, supports MCP, and not
  // already enabled. The preferred-tools setting may narrow this list further.
  const addToolCandidates = React.useMemo(() => {
    const candidates = mcpTools.filter(
      (tool) => tool.installed && !enabledToolIds.has(tool.key),
    );
    if (limitAddMoreToPreferredTools && preferredToolKeysForAddMore) {
      const preferredKeys = new Set(preferredToolKeysForAddMore);
      return candidates.filter((tool) => preferredKeys.has(tool.key));
    }
    return candidates;
  }, [mcpTools, enabledToolIds, limitAddMoreToPreferredTools, preferredToolKeysForAddMore]);

  const addToolItems = React.useMemo<ManagementMenuItem[]>(
    () => addToolCandidates.map((tool) => ({
      key: tool.key,
      label: tool.display_name,
      icon: (
        <ToolIcon
          toolKey={tool.key}
          label={tool.display_name}
          size={14}
          iconUrl={tool.icon_url ?? undefined}
        />
      ),
      onSelect: () => onToggleTool(server.id, tool.key),
    })),
    [addToolCandidates, onToggleTool, server.id],
  );

  // Disable/re-enable entry mirrors SkillCard's menu order: metadata first,
  // then the management toggle. Omitted entirely when no handler is wired so
  // a blank placeholder row can never appear.
  const managementItems = React.useMemo<ManagementMenuItem[]>(() => {
    if (!onSetManagementEnabled) {
      return [];
    }
    if (server.management_enabled) {
      return [{
        key: 'disable',
        icon: <PowerOff size={14} />,
        label: t('mcp.disableServer'),
        onSelect: () => onSetManagementEnabled(server, false),
        disabled: loading,
      }];
    }
    // Disabled server: only re-enable is offered and must never be disabled,
    // otherwise the user could not recover the server.
    return [{
      key: 'enable',
      icon: <Power size={14} />,
      label: t('mcp.enableServer'),
      onSelect: () => onSetManagementEnabled(server, true),
      disabled: loading,
    }];
  }, [loading, onSetManagementEnabled, server, t]);

  const handleCopyJson = React.useCallback(async () => {
    try {
      await navigator.clipboard.writeText(formatMcpServerExportJson(server));
      message.success(t('mcp.exportCopied'));
    } catch {
      message.error(t('mcp.copyFailed'));
    }
  }, [server, t]);

  const actionItems = React.useMemo<ManagementMenuItem[]>(() => [
    {
      key: 'metadata',
      icon: <Tags size={14} />,
      label: t('mcp.metadata.edit'),
      onSelect: () => onEditMetadata(server),
      disabled: loading,
    },
    ...managementItems,
    {
      key: 'edit',
      icon: <Pencil size={14} />,
      label: t('mcp.edit'),
      onSelect: () => onEdit(server),
      disabled: loading,
    },
    {
      key: 'copy-json',
      icon: <FileJson size={14} />,
      label: t('mcp.copyJson'),
      onSelect: () => { void handleCopyJson(); },
      disabled: loading,
    },
    {
      key: 'delete',
      danger: true,
      icon: <Trash2 size={14} />,
      label: t('common.delete'),
      onSelect: () => onDelete(server.id),
      disabled: loading,
    },
  ], [handleCopyJson, loading, managementItems, onDelete, onEdit, onEditMetadata, server, t]);

  const handleCardClick: React.MouseEventHandler<HTMLDivElement> = React.useCallback((event) => {
    // Selection mode reserves the card click for the checkbox; interactive
    // elements handle their own click. Only browse-mode opens the drawer.
    if (selectable) {
      return;
    }
    const target = event.target as HTMLElement;
    if (target.closest(DETAIL_CLICK_EXCLUDE_SELECTOR)) {
      return;
    }
    onOpenDetail?.(server);
  }, [onOpenDetail, selectable, server]);

  const handleReadOnlyToolClick = React.useCallback(() => {
    message.info(t('mcp.groupTools.cardToolReadOnly'));
  }, [t]);

  const handleToolPillClick = React.useCallback((tool: McpTool) => {
    if (toolsReadOnly) {
      handleReadOnlyToolClick();
      return;
    }
    if (loading) {
      return;
    }
    onToggleTool(server.id, tool.key);
  }, [handleReadOnlyToolClick, loading, onToggleTool, server.id, toolsReadOnly]);

  const cardClassName = [
    styles.card,
    selected ? styles.cardSelected : '',
    !server.management_enabled ? styles.disabledCard : '',
  ].filter(Boolean).join(' ');

  return (
    <div ref={containerRef} style={containerStyle}>
      <div className={cardClassName} onClick={onOpenDetail ? handleCardClick : undefined}>
        <div className={styles.headerRow}>
          {selectable ? (
            <ManagementCheckbox
              ariaLabel={`${t('common.select')} ${server.name}`}
              checked={!!selected}
              onChange={(checked) => onSelectChange?.(server.id, checked)}
            />
          ) : dragHandle ?? (
            <span
              className={`${styles.statusDot}${server.management_enabled ? ` ${styles.statusDotEnabled}` : ''}`}
              title={server.management_enabled ? t('mcp.enableServer') : t('mcp.disableServer')}
              aria-hidden="true"
            />
          )}
          <span className={styles.name} title={server.name}>{server.name}</span>
          <span className={styles.hoverActions}>
            <button
              type="button"
              className={styles.miniBtn}
              title={t('mcp.copyConfig')}
              aria-label={t('mcp.copyConfig')}
              disabled={!configSummary}
              onClick={handleCopyConfig}
            >
              <Copy size={13} aria-hidden="true" />
            </button>
            <button
              type="button"
              className={styles.miniBtn}
              title={t('common.refresh')}
              aria-label={t('common.refresh')}
              disabled={loading || !onRefresh}
              onClick={onRefresh}
            >
              <RefreshCw size={13} aria-hidden="true" />
            </button>
            <ManagementMenu
              items={actionItems}
              title={t('mcp.settings')}
              triggerClassName={styles.miniBtn}
            >
              <MoreHorizontal size={13} aria-hidden="true" />
            </ManagementMenu>
          </span>
        </div>

        <div className={styles.body}>
          <p className={styles.commandLine} title={commandText}>{commandText}</p>
          {descriptionText && (
            <p className={styles.description} title={descriptionText}>{descriptionText}</p>
          )}
          {(tagList.length > 0 || groupText || noteText) && (
            <div className={styles.tagRow}>
              {tagList.map((tag) => (
                <span key={tag} className={`${styles.tagPill} ${tagPillColorClass(tag)}`}>
                  <span className={styles.tagPillText} title={tag}>{tag}</span>
                </span>
              ))}
              {groupText && (
                <span className={styles.groupTag} title={groupText}>{groupText}</span>
              )}
              {noteText && (
                <span className={styles.note} title={noteText}>{noteText}</span>
              )}
            </div>
          )}
        </div>

        <div className={styles.footerRow}>
          <span className={`${styles.sourceBtn} ${styles.sourceBtnStatic}`}>
            {transportIconNode}
            <span className={styles.typeText}>{server.server_type}</span>
            {packageVersionDisplayText && (
              <span className={styles.packageVersionTag} title={packageVersionDisplayText}>
                {truncateMiddle(packageVersionDisplayText)}
              </span>
            )}
          </span>
          <span className={styles.footerTools}>
            {enabledTools.map((tool) => {
              const syncDetail = syncDetailByTool.get(tool.key);
              const isError = syncDetail?.status === 'error';
              const isPending = syncDetail?.status === 'pending';
              const statusText = isPending
                ? t('mcp.toolSync.pending')
                : isError
                  ? t('mcp.toolSync.error')
                  : t('mcp.toolSync.synced');
              const pillTitle = isError && syncDetail?.error_message
                ? `${tool.display_name} — ${statusText}: ${syncDetail.error_message}`
                : `${tool.display_name} — ${statusText}`;
              const pillClassName = [
                styles.toolPill,
                styles.active,
                isError ? styles.errorPill : '',
                toolsReadOnly ? styles.readOnlyTool : '',
              ].filter(Boolean).join(' ');

              return (
                <button
                  key={tool.key}
                  type="button"
                  className={pillClassName}
                  title={pillTitle}
                  aria-label={pillTitle}
                  disabled={loading || !server.management_enabled}
                  onClick={() => handleToolPillClick(tool)}
                >
                  <ToolIcon
                    toolKey={tool.key}
                    label={tool.display_name}
                    size={14}
                    iconUrl={tool.icon_url ?? undefined}
                  />
                  {isPending && (
                    <Loader2 size={10} className={styles.toolPendingIcon} aria-hidden="true" />
                  )}
                </button>
              );
            })}
            {addToolItems.length > 0 && !toolsReadOnly && (
              <ManagementMenu
                items={addToolItems}
                disabled={loading || !server.management_enabled}
                title={t('mcp.addTool')}
                triggerClassName={styles.addToolBtn}
              >
                <Plus size={12} aria-hidden="true" />
              </ManagementMenu>
            )}
          </span>
        </div>
      </div>
    </div>
  );
});

const SortableMcpCard: React.FC<Omit<McpCardProps, 'dragDisabled'>> = (props) => {
  const { t } = useTranslation();
  const { server } = props;

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
        <span
          {...attributes}
          {...listeners}
          className={styles.dragHandle}
          data-mcp-card-no-detail
          title={t('mcp.reorderHint')}
          aria-label={t('mcp.reorderHint')}
        >
          <GripVertical size={14} aria-hidden="true" />
        </span>
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

  // In sortable/reorder mode, keep the card click focused on drag/sort and do
  // not open the detail panel accidentally.
  const { onOpenDetail: ignoredOnOpenDetail, ...sortableProps } = props;
  return <SortableMcpCard {...sortableProps} />;
});

export default McpCard;
